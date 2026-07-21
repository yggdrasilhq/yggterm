#!/usr/bin/env bash
# Shadow view client under a headless compositor (agent control plane, slice 4.3).
#
# Runs a SECOND yggterm view client that attaches to the same daemon as the
# user's GUI but declares `--client-role shadow`, so the daemon's slice-4.0 role
# gate refuses it every ownership-claiming request. It gets its own compositor,
# its own viewport, and its own active session — so an agent can look at a
# session WITHOUT yanking the user's view (the pt10 "whooping" annoyance).
#
# It is a VIEW, never a second source of truth: both clients read the one daemon
# (Phase-2 doctrine — daemon = truth, view client = disposable).
#
#   start   spawn the compositor + shadow client
#   capture write a PNG of the shadow viewport (grim)
#   status  report whether the compositor/client are up
#   stop    tear both down
#
# Usage:
#   scripts/shadow-client.sh start [--name <id>] [--size WxH]
#   scripts/shadow-client.sh capture <out.png> [--name <id>]
#   scripts/shadow-client.sh status [--name <id>]
#   scripts/shadow-client.sh stop [--name <id>]
#
# Requires: sway (wlroots headless backend) and grim. Both must be on PATH.
#
# ⚠ Read-only geometry (eng-review D8): a shadow NEVER drives PTY winsize,
# terminal focus, or scroll — a differently-sized shadow view issuing SIGWINCH
# would reflow the CLI and scramble the USER's live frame without ever claiming
# ownership, which the takeover guard alone does not cover. The daemon enforces
# this (TerminalResize / FocusLive / TerminalEnsure are all Deny for a Shadow);
# this script additionally pins a fixed canonical size so the shadow never even
# asks.

set -euo pipefail

NAME="shadow-1"
SIZE="1920x1080"
CMD="${1:-}"
[ $# -gt 0 ] && shift || true

OUT=""
case "$CMD" in
  capture)
    if [ $# -gt 0 ] && [[ "${1:-}" != --* ]]; then
      OUT="$1"
      shift
    fi
    ;;
esac

while [ $# -gt 0 ]; do
  case "$1" in
    --name) NAME="$2"; shift 2 ;;
    --size) SIZE="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

: "${XDG_RUNTIME_DIR:=/run/user/$(id -u)}"
export XDG_RUNTIME_DIR

RUN_DIR="$XDG_RUNTIME_DIR/yggterm-shadow/$NAME"
CONF="$RUN_DIR/sway.conf"
LOG="$RUN_DIR/sway.log"
CLIENT_LOG="$RUN_DIR/client.log"
DISPLAY_FILE="$RUN_DIR/wayland-display"
SWAY_PID_FILE="$RUN_DIR/sway.pid"
CLIENT_PID_FILE="$RUN_DIR/client.pid"

# Prefer the deployed binary, fall back to a local release build.
YGGTERM_BIN="${YGGTERM_BIN:-$HOME/.local/bin/yggterm}"
if [ ! -x "$YGGTERM_BIN" ]; then
  YGGTERM_BIN="$(cd "$(dirname "$0")/.." && pwd)/target/release/yggterm"
fi

need() { command -v "$1" >/dev/null 2>&1 || { echo "missing required tool: $1" >&2; exit 3; }; }

# Alive AND not a zombie. `kill -0` alone succeeds against an exited-but-unreaped
# process, which made a client that had already failed its startup role gate
# report as "up" — a false success is worse than a loud failure.
is_running() {
  [ -f "$1" ] || return 1
  local pid state
  pid="$(cat "$1")"
  [ -n "$pid" ] || return 1
  kill -0 "$pid" 2>/dev/null || return 1
  state="$(awk '{print $3}' "/proc/$pid/stat" 2>/dev/null || echo '')"
  [ "$state" != "Z" ]
}

# Wayland sockets currently in XDG_RUNTIME_DIR, newest first. A glob (not
# `ls | grep`) so odd filenames cannot confuse the match.
wayland_sockets() {
  local s
  for s in "$XDG_RUNTIME_DIR"/wayland-[0-9]*; do
    [ -S "$s" ] || continue
    case "${s##*/}" in
      wayland-*[!0-9-]*) continue ;;
    esac
    printf '%s\n' "${s##*/}"
  done
}

case "$CMD" in
  start)
    need sway; need grim
    [ -x "$YGGTERM_BIN" ] || { echo "yggterm binary not found: $YGGTERM_BIN" >&2; exit 3; }
    if is_running "$SWAY_PID_FILE"; then
      echo "shadow '$NAME' already running (sway pid $(cat "$SWAY_PID_FILE"))"
      exit 0
    fi
    mkdir -p "$RUN_DIR"
    # Note the compositor sockets BEFORE starting, so we can identify the one
    # sway creates (it names its own `wayland-N`; WAYLAND_DISPLAY on the way IN
    # is what a NESTED client would connect to, not what sway will create).
    before="$(wayland_sockets | wc -l)"
    cat > "$CONF" <<EOF
# Headless compositor for the yggterm shadow view client. No bars, no
# keybindings, no seat devices: nothing here should ever reach the user's seat.
# Xwayland off — it is not needed and only adds a failure mode (and a warning
# on hosts that ship sway without it).
xwayland disable
output HEADLESS-1 resolution $SIZE position 0 0
output * bg #101418 solid_color
EOF

    WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 \
      setsid sway -c "$CONF" > "$LOG" 2>&1 &
    echo $! > "$SWAY_PID_FILE"

    # Wait for the new socket to appear.
    display=""
    for _ in $(seq 1 50); do
      sleep 0.2
      after="$(wayland_sockets | wc -l)"
      if [ "$after" -gt "$before" ]; then
        # Newest socket = the one this sway just created.
        display="$(for s in $(wayland_sockets); do
          printf '%s %s\n' "$(stat -c %Y "$XDG_RUNTIME_DIR/$s")" "$s"
        done | sort -rn | head -1 | cut -d' ' -f2)"
        break
      fi
    done
    [ -n "$display" ] || { echo "headless compositor did not come up; see $LOG" >&2; exit 4; }
    echo "$display" > "$DISPLAY_FILE"

    # The client declares its role; the daemon enforces it. If the daemon is too
    # old to enforce roles, the client REFUSES to attach (fail closed, D7).
    WAYLAND_DISPLAY="$display" GDK_BACKEND=wayland \
      setsid "$YGGTERM_BIN" --client-role shadow --client-id "$NAME" \
      > "$CLIENT_LOG" 2>&1 &
    echo $! > "$CLIENT_PID_FILE"
    # Watch long enough to catch a startup failure. The fail-closed role gate
    # fires only after the client has opened its store and resolved a daemon, so
    # a 2s glance reported a doomed client as healthy — poll instead, and treat
    # "survived the window" as the success signal.
    for _ in $(seq 1 40); do
      sleep 0.25
      is_running "$CLIENT_PID_FILE" || break
    done
    if ! is_running "$CLIENT_PID_FILE"; then
      echo "shadow client exited during startup:" >&2
      tail -5 "$CLIENT_LOG" >&2
      exit 5
    fi
    echo "shadow '$NAME' up: WAYLAND_DISPLAY=$display sway=$(cat "$SWAY_PID_FILE") client=$(cat "$CLIENT_PID_FILE")"
    ;;

  capture)
    need grim
    [ -n "$OUT" ] || { echo "usage: $0 capture <out.png> [--name <id>]" >&2; exit 2; }
    [ -f "$DISPLAY_FILE" ] || { echo "shadow '$NAME' is not running" >&2; exit 4; }
    WAYLAND_DISPLAY="$(cat "$DISPLAY_FILE")" grim "$OUT"
    echo "captured shadow '$NAME' -> $OUT"
    ;;

  status)
    if is_running "$SWAY_PID_FILE"; then
      echo "sway:   up (pid $(cat "$SWAY_PID_FILE"), display $(cat "$DISPLAY_FILE" 2>/dev/null || echo '?'))"
    else
      echo "sway:   down"
    fi
    if is_running "$CLIENT_PID_FILE"; then
      echo "client: up (pid $(cat "$CLIENT_PID_FILE"))"
    else
      echo "client: down"
      [ -f "$CLIENT_LOG" ] && tail -3 "$CLIENT_LOG"
    fi
    ;;

  stop)
    for f in "$CLIENT_PID_FILE" "$SWAY_PID_FILE"; do
      if is_running "$f"; then
        kill -TERM "$(cat "$f")" 2>/dev/null || true
      fi
    done
    sleep 1
    for f in "$CLIENT_PID_FILE" "$SWAY_PID_FILE"; do
      if is_running "$f"; then
        kill -KILL "$(cat "$f")" 2>/dev/null || true
      fi
      rm -f "$f"
    done
    rm -f "$DISPLAY_FILE"
    echo "shadow '$NAME' stopped"
    ;;

  *)
    sed -n '2,30p' "$0"
    exit 2
    ;;
esac
