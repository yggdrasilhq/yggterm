#!/usr/bin/env bash
# Isolated under-glass acceptance sandbox (Phase F / immersion close-out).
#
# Runs a FULL yggterm GUI (not a shadow) under a private headless sway, with a
# private YGGTERM_HOME + HOME so its daemon, sessions, profiles and web
# surfaces touch nothing real. Under-glass is ARMED
# (YGGTERM_WEB_SURFACE_UNDER_GLASS=1). sway is the instrument rack:
#   - `swaymsg seat * cursor set X Y` = REAL pointer motion through the
#     compositor → GTK motion events → the edge-motion reveal path (the thing
#     no DOM-synthesized event can drive);
#   - `grim` = fast native-webview-faithful frames (~100 ms), the capture
#     spectacle is too slow for.
#
#   start    spawn compositor + armed GUI on a fresh sandbox home
#   capture  <out.png>  grab a frame
#   cursor   <x> <y>    move the seat pointer
#   click    <x> <y>    move + left-click
#   env      print the sandbox env exports for ad-hoc commands
#   stop     tear everything down (sandbox home is preserved for inspection)
#
# Requires: sway, grim (same as shadow-client.sh).

set -euo pipefail

NAME="uglass-1"
SIZE="1920x1200"
CMD="${1:-}"
[ $# -gt 0 ] && shift || true

ARG1=""; ARG2=""
case "$CMD" in
  capture) ARG1="${1:-}"; shift || true ;;
  cursor|click) ARG1="${1:-}"; ARG2="${2:-}"; shift 2 || true ;;
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

# Same D-Bus refusal as shadow-client.sh: an absent address autolaunches a
# private bus nobody reaps; an invalid one fails loudly and leaks nothing.
if [ -z "${DBUS_SESSION_BUS_ADDRESS:-}" ]; then
  if [ -S "$XDG_RUNTIME_DIR/bus" ]; then
    DBUS_SESSION_BUS_ADDRESS="unix:path=$XDG_RUNTIME_DIR/bus"
  else
    DBUS_SESSION_BUS_ADDRESS="unix:path=/nonexistent/yggterm-refuses-dbus-autolaunch"
  fi
  export DBUS_SESSION_BUS_ADDRESS
fi

RUN_DIR="$XDG_RUNTIME_DIR/yggterm-uglass/$NAME"
SANDBOX_HOME="$RUN_DIR/home"
CONF="$RUN_DIR/sway.conf"
LOG="$RUN_DIR/sway.log"
CLIENT_LOG="$RUN_DIR/client.log"
DISPLAY_FILE="$RUN_DIR/wayland-display"
SWAY_PID_FILE="$RUN_DIR/sway.pid"
CLIENT_PID_FILE="$RUN_DIR/client.pid"

# ⚠ Daemon-owned rows export YGGTERM_BIN=<yggterm-headless> (pending-bugs:
# "shadow-client.sh is broken for every in-session agent"). A headless binary
# cannot be a GUI; refuse it rather than launch usage-text into the log.
YGGTERM_BIN="${YGGTERM_BIN:-}"
case "$YGGTERM_BIN" in
  ""|*-headless) YGGTERM_BIN="$(cd "$(dirname "$0")/.." && pwd)/target/release/yggterm" ;;
esac

need() { command -v "$1" >/dev/null 2>&1 || { echo "missing required tool: $1" >&2; exit 3; }; }

is_running() {
  [ -f "$1" ] || return 1
  local pid state
  pid="$(cat "$1")"
  [ -n "$pid" ] || return 1
  kill -0 "$pid" 2>/dev/null || return 1
  state="$(awk '{print $3}' "/proc/$pid/stat" 2>/dev/null || echo '')"
  [ "$state" != "Z" ]
}

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

sandbox_env() {
  echo "export XDG_RUNTIME_DIR='$XDG_RUNTIME_DIR'"
  echo "export WAYLAND_DISPLAY='$(cat "$DISPLAY_FILE")'"
  echo "export GDK_BACKEND=wayland"
  echo "export HOME='$SANDBOX_HOME'"
  echo "export YGGTERM_HOME='$SANDBOX_HOME/.yggterm'"
  echo "export DBUS_SESSION_BUS_ADDRESS='$DBUS_SESSION_BUS_ADDRESS'"
  echo "export SWAYSOCK='$(ls "$XDG_RUNTIME_DIR"/sway-ipc.*.$(cat "$SWAY_PID_FILE").sock 2>/dev/null | head -1)'"
}

case "$CMD" in
  start)
    need sway; need grim
    [ -x "$YGGTERM_BIN" ] || { echo "yggterm binary not found: $YGGTERM_BIN" >&2; exit 3; }
    if is_running "$SWAY_PID_FILE"; then
      echo "sandbox '$NAME' already running (sway pid $(cat "$SWAY_PID_FILE"))"
      exit 0
    fi
    mkdir -p "$RUN_DIR" "$SANDBOX_HOME"
    before="$(wayland_sockets | wc -l)"
    cat > "$CONF" <<EOF
xwayland disable
output HEADLESS-1 resolution $SIZE position 0 0
output * bg #101418 solid_color
# One seat, no devices: the ONLY input this compositor ever sees is what
# swaymsg synthesizes — deterministic by construction.
EOF
    WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 \
      setsid sway -c "$CONF" > "$LOG" 2>&1 &
    echo $! > "$SWAY_PID_FILE"
    display=""
    for _ in $(seq 1 50); do
      sleep 0.2
      after="$(wayland_sockets | wc -l)"
      if [ "$after" -gt "$before" ]; then
        display="$(for s in $(wayland_sockets); do
          printf '%s %s\n' "$(stat -c %Y "$XDG_RUNTIME_DIR/$s")" "$s"
        done | sort -rn | head -1 | cut -d' ' -f2)"
        break
      fi
    done
    [ -n "$display" ] || { echo "headless compositor did not come up; see $LOG" >&2; exit 4; }
    echo "$display" > "$DISPLAY_FILE"

    WAYLAND_DISPLAY="$display" GDK_BACKEND=wayland \
      HOME="$SANDBOX_HOME" YGGTERM_HOME="$SANDBOX_HOME/.yggterm" \
      YGGTERM_WEB_SURFACE_UNDER_GLASS=1 \
      YGGTERM_DESKTOP_APP_ID_SUFFIX="uglass-$NAME" \
      setsid "$YGGTERM_BIN" > "$CLIENT_LOG" 2>&1 &
    echo $! > "$CLIENT_PID_FILE"
    for _ in $(seq 1 60); do
      sleep 0.25
      is_running "$CLIENT_PID_FILE" || break
    done
    if ! is_running "$CLIENT_PID_FILE"; then
      echo "sandbox GUI exited during startup:" >&2
      tail -8 "$CLIENT_LOG" >&2
      exit 5
    fi
    echo "sandbox '$NAME' up: display=$display home=$SANDBOX_HOME gui=$(cat "$CLIENT_PID_FILE")"
    ;;
  capture)
    [ -n "$ARG1" ] || { echo "capture needs an output path" >&2; exit 2; }
    WAYLAND_DISPLAY="$(cat "$DISPLAY_FILE")" grim "$ARG1"
    ;;
  cursor|click)
    [ -n "$ARG1" ] && [ -n "$ARG2" ] || { echo "$CMD needs x y" >&2; exit 2; }
    # ⚠ `swaymsg seat cursor press` silently no-ops on a seat with ZERO input
    # devices (WLR_LIBINPUT_NO_DEVICES=1 ⇒ no wl_pointer capability at all).
    # wlrctl attaches a VIRTUAL POINTER (zwlr_virtual_pointer_v1), which gives
    # the seat a real pointer capability and delivers genuine events. Its moves
    # are RELATIVE: park at the top-left corner first, then offset — absolute
    # by construction, deterministic for any prior position.
    need wlrctl
    export WAYLAND_DISPLAY="$(cat "$DISPLAY_FILE")"
    wlrctl pointer move -20000 -20000
    wlrctl pointer move "$ARG1" "$ARG2"
    if [ "$CMD" = click ]; then
      wlrctl pointer click left
    fi
    ;;
  env)
    sandbox_env
    ;;
  stop)
    for f in "$CLIENT_PID_FILE" "$SWAY_PID_FILE"; do
      if is_running "$f"; then kill "$(cat "$f")" 2>/dev/null || true; fi
      rm -f "$f"
    done
    echo "sandbox '$NAME' stopped (home preserved at $SANDBOX_HOME)"
    ;;
  *)
    echo "usage: $0 <start|capture|cursor|click|env|stop> [args] [--name id] [--size WxH]" >&2
    exit 2
    ;;
esac
