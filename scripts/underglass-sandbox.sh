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
#   burst    <dir> <n> [gap_ms]  grab n frames back to back (tear probing)
#   cursor   <x> <y>    move the seat pointer
#   click    <x> <y>    move + left-click
#   scroll   <dy> [dx] [reps] [gap_ms]  real wheel axis events through the seat
#   backend  what windowing backend the GUI ACTUALLY got (not what env asked for)
#   env      print the sandbox env exports for ad-hoc commands
#   stop     tear everything down (sandbox home is preserved for inspection)
#
# start options:
#   --under-glass 0|1   arm or disarm Phase F under-glass (default 1)
#   --env K=V           extra environment for the GUI, repeatable (A/B arms)
#   --xwayland          allow XWayland in the sandbox compositor (default: OFF,
#                       because the live GUI on guihost is Wayland-NATIVE and a
#                       sandbox that silently lands on XWayland answers a
#                       different question — see `backend`)
#
# Requires: sway, grim (same as shadow-client.sh).

set -euo pipefail

NAME="uglass-1"
SIZE="1920x1200"
CMD="${1:-}"
[ $# -gt 0 ] && shift || true

# Positionals are everything before the first `--flag`; parsing them by fixed
# count silently ate `--name` when an optional positional was omitted.
POS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --*) break ;;
    *) POS+=("$1"); shift ;;
  esac
done
ARG1="${POS[0]:-}"; ARG2="${POS[1]:-}"; ARG3="${POS[2]:-}"; ARG4="${POS[3]:-}"
UNDER_GLASS=1
EXTRA_ENV=()
XWAYLAND=disable
while [ $# -gt 0 ]; do
  case "$1" in
    --name) NAME="$2"; shift 2 ;;
    --size) SIZE="$2"; shift 2 ;;
    --under-glass) UNDER_GLASS="$2"; shift 2 ;;
    --env) EXTRA_ENV+=("$2"); shift 2 ;;
    --xwayland) XWAYLAND=enable; shift ;;
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
xwayland $XWAYLAND
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

    # GDK_BACKEND is set EXPLICITLY and never left to inference. Two separate
    # pieces of code force x11 when it is unset -- yggterm's own
    # `linux_desktop_backend_policy_from_input` on a KDE wayland+X11 pair, and
    # vendored dioxus's `linux_dma_buf_workaround_should_force_x11`
    # (vendor/dioxus-desktop/src/app.rs:658) which fires whenever GDK_BACKEND is
    # absent. The live GUI on guihost is Wayland-NATIVE; a sandbox that quietly
    # lands on XWayland is measuring a different machine. `backend` asserts it.
    env -i \
      PATH="$PATH" \
      XDG_RUNTIME_DIR="$XDG_RUNTIME_DIR" \
      DBUS_SESSION_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" \
      WAYLAND_DISPLAY="$display" \
      GDK_BACKEND=wayland \
      XDG_SESSION_TYPE=wayland \
      HOME="$SANDBOX_HOME" YGGTERM_HOME="$SANDBOX_HOME/.yggterm" \
      YGGTERM_WEB_SURFACE_UNDER_GLASS="$UNDER_GLASS" \
      YGGTERM_DESKTOP_APP_ID_SUFFIX="uglass-$NAME" \
      "${EXTRA_ENV[@]}" \
      setsid "$YGGTERM_BIN" > "$CLIENT_LOG" 2>&1 &
    echo $! > "$CLIENT_PID_FILE"
    printf '%s\n' "${EXTRA_ENV[@]}" > "$RUN_DIR/arm-env"
    echo "under_glass=$UNDER_GLASS" >> "$RUN_DIR/arm-env"
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
  burst)
    # Frames back to back, no sleep between: grim is ~100 ms, which is the
    # sampling rate. A tear lives INSIDE one committed buffer, so the sampling
    # rate bounds how many tears we SEE, never whether a seen frame is torn.
    [ -n "$ARG1" ] && [ -n "$ARG2" ] || { echo "burst needs <dir> <n> [gap_ms]" >&2; exit 2; }
    mkdir -p "$ARG1"
    export WAYLAND_DISPLAY="$(cat "$DISPLAY_FILE")"
    i=0
    while [ "$i" -lt "$ARG2" ]; do
      grim "$ARG1/$(printf 'f%04d' "$i").png"
      i=$((i + 1))
      if [ -n "${ARG3:-}" ] && [ "${ARG3}" != "0" ]; then
        sleep "$(awk -v m="$ARG3" 'BEGIN{print m/1000}')"
      fi
    done
    echo "burst: $ARG2 frames in $ARG1"
    ;;
  scroll)
    # REAL wl_pointer axis events through the compositor seat -- not a DOM
    # scrollTop write, not `web do scroll`. Only this path exercises WebKit's
    # own (smooth, possibly threaded) scrolling machinery, which is the thing
    # under suspicion.
    [ -n "$ARG1" ] || { echo "scroll needs <dy> [dx] [reps] [gap_ms]" >&2; exit 2; }
    need wlrctl
    export WAYLAND_DISPLAY="$(cat "$DISPLAY_FILE")"
    i=0
    while [ "$i" -lt "${ARG3:-1}" ]; do
      wlrctl pointer scroll "$ARG1" "${ARG2:-0}"
      i=$((i + 1))
      if [ -n "${ARG4:-}" ] && [ "${ARG4}" != "0" ]; then
        sleep "$(awk -v m="$ARG4" 'BEGIN{print m/1000}')"
      fi
    done
    ;;
  backend)
    # The GUI's OWN view, never /proc/<pid>/environ (which holds only the
    # exec-time environment -- field guide §7.4). The startup trace records
    # `gdk_backend` as the process read it back after its policy ran; the
    # compositor's toplevel list is the independent second opinion, because a
    # sandbox with `xwayland disable` cannot host an X client at all.
    trace="$SANDBOX_HOME/.yggterm/event-trace.jsonl"
    if [ -f "$trace" ]; then
      grep -m1 'linux_desktop_backend_policy' "$trace" \
        | python3 -c 'import json,sys; d=json.loads(sys.stdin.read())["payload"]; print(" ".join(f"{k}={d.get(k)}" for k in ("gdk_backend","policy","wayland_display_present","display_present","webkit_gl_policy","gl_probe_class","gl_probe_driver","web_surface_under_glass","webkit_disable_dmabuf_renderer","xterm_canvas_renderer")))' \
        2>/dev/null || echo "gdk_backend=<trace unreadable>"
    else
      echo "gdk_backend=<no trace at $trace>"
    fi
    if [ "$XWAYLAND" = disable ]; then
      echo "compositor_xwayland=disabled (an X client could not have started)"
    fi
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
    # The GUI is not the only process the sandbox owns: it spawns a daemon and
    # every session it started keeps running. Killing only the GUI leaked one
    # daemon plus one shell PER ARM across an A/B matrix. Scope the sweep by
    # YGGTERM_HOME read from each process's OWN environment, so it can only ever
    # match this sandbox and never another agent's real daemon.
    for pid in $(pgrep -f yggterm 2>/dev/null || true); do
      home="$(tr '\0' '\n' < "/proc/$pid/environ" 2>/dev/null | grep -m1 '^YGGTERM_HOME=' || true)"
      case "$home" in
        "YGGTERM_HOME=$SANDBOX_HOME/.yggterm") kill "$pid" 2>/dev/null || true ;;
      esac
    done
    echo "sandbox '$NAME' stopped (home preserved at $SANDBOX_HOME)"
    ;;
  *)
    echo "usage: $0 <start|capture|burst|cursor|click|scroll|backend|env|stop> [args] [--name id] [--size WxH] [--under-glass 0|1] [--env K=V] [--xwayland]" >&2
    exit 2
    ;;
esac
