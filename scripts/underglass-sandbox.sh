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
#            ⛔ `eval "$(… env)"` EXPORTS `HOME` TOO, so every later `~/…` path
#            resolves INTO the sandbox home: `~/.yggterm/bin/yggterm-headless`
#            becomes a file that does not exist, and the first probe fails with
#            "No such file or directory" — which reads as "the harness is
#            broken" rather than "your path moved". Use ABSOLUTE paths for the
#            real binaries after eval-ing this. Reported 2026-08-14 by the
#            second session to use the harness, having cost it a false start.
#   stop     tear everything down (sandbox home is preserved for inspection)
#   reap     delete DEAD sandbox homes, skipping any whose compositor is alive.
#            The homes sit in a tmpfs -- RAM -- and nothing else ever frees
#            them; 48 leaked ones once filled it and made `start` fail silently.
#
# start options:
#   --under-glass 0|1   arm or disarm Phase F under-glass (default 1)
#   --env K=V           extra environment for the GUI, repeatable (A/B arms)
#   --xwayland          allow XWayland in the sandbox compositor (default: OFF,
#                       because the live GUI on guihost is Wayland-NATIVE and a
#                       sandbox that silently lands on XWayland answers a
#                       different question — see `backend`)
#
# ⛔ WHICH BINARY IT ACTUALLY RUNS — set YGGTERM_GUI_BIN, every time.
#   The resolver tries the INSTALLED build first and your repo build second, so a
#   sandbox started to prove a change you have not installed comes up green,
#   behaves perfectly, and proves nothing. It fails in the reassuring direction:
#   there is no error, only a GUI that does not have your code in it.
#     YGGTERM_GUI_BIN=<repo>/target/release/yggterm scripts/underglass-sandbox.sh start
#   Then confirm, rather than assume — `strings -a <bin> | grep -F '<a string
#   only your build has>'`. ⚠ MTIME CANNOT ANSWER THIS: an installed binary
#   stamped minutes AFTER a commit has been observed not to contain it.
#
# ⛔ SETTINGS ARE READ AT START. Anything you want the GUI to come up with —
#   vertical tabs is the one that keeps catching people, because without it you
#   silently get the horizontal strip and go looking for a rail that was never on
#   screen — must be written into <sandbox home>/.yggterm/settings.json BEFORE
#   `start`. Written afterwards it is not re-read.
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

# ⛔ ONE owner for "which binary can be a GUI" — see scripts/lib/gui-binary.sh.
# The `*-headless` case statement that used to live here was half right: it
# caught the plain headless path but not `…/yggterm-headless (deleted)`, which
# is what a hot-restarted daemon actually exports, nor a GUI-named binary that
# is a headless build. The override is YGGTERM_GUI_BIN; YGGTERM_BIN belongs to
# the daemon.
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/gui-binary.sh
. "$REPO_ROOT/scripts/lib/gui-binary.sh"

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
  # ⛔⛔ A MISSING DISPLAY IS AN ERROR, NOT A STATE. This used to emit
  # `export WAYLAND_DISPLAY=''` when the file was absent -- `cat` complained on
  # stderr and the export went out empty anyway, so `eval "$(... env)"` handed
  # the caller a sandbox with no display and returned 0. Every probe downstream
  # then measured nothing and said so in the language of success: a reproduction
  # loop of eight trials printed a clean verdict for every one of them and no
  # GUI had started in any. A harness that renders no pixels must never be able
  # to report absence of a fault. See docs/pending-bugs.md [11.26] harness entry.
  if [ ! -s "$DISPLAY_FILE" ]; then
    echo "sandbox '$NAME' has no wayland display ($DISPLAY_FILE missing or empty);" >&2
    echo "  it is not running, or \`start\` failed. Nothing measured here means anything." >&2
    exit 6
  fi
  if ! is_running "$SWAY_PID_FILE"; then
    echo "sandbox '$NAME' has a display file but its compositor is dead (pid file $SWAY_PID_FILE)." >&2
    exit 6
  fi
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
    # Resolved here, not at load time, so `stop`/`env` keep working on a host
    # with no GUI build at all.
    YGGTERM_GUI_BINARY="$(yggterm_resolve_gui_binary "$REPO_ROOT")" || exit 3
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
    # The write above is the one that fails first when the runtime tmpfs is
    # full, and a start that wrote no display file must not go on to report a
    # GUI. Assert it landed rather than trusting the redirect's exit status.
    [ -s "$DISPLAY_FILE" ] || {
      echo "could not record the sandbox display in $DISPLAY_FILE" >&2
      df -h "$XDG_RUNTIME_DIR" >&2 || true
      exit 4
    }

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
      setsid "$YGGTERM_GUI_BINARY" > "$CLIENT_LOG" 2>&1 &
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
    # ⛔ AND THE COMPOSITOR, WHICH THE PID FILE DOES NOT RELIABLY NAME. `setpid`
    # is written by `setsid sway …`, and util-linux `setsid` only forks when it
    # is already a process-group leader — so `$!` is sometimes sway and
    # sometimes a parent that has already exited, in which case `is_running`
    # answers false and the compositor is never killed. Observed leaking one
    # sway PER STOP on one host while cleaning up correctly on another, which
    # is exactly how an intermittent leak survives review. Sweep by the config
    # path, which names THIS sandbox and nothing else — the same shape as the
    # YGGTERM_HOME-scoped sweep above, and for the same reason.
    for pid in $(pgrep -f "sway -c $CONF" 2>/dev/null || true); do
      kill "$pid" 2>/dev/null || true
    done
    echo "sandbox '$NAME' stopped (home preserved at $SANDBOX_HOME; \`reap\` frees it)"
    ;;
  reap)
    # ⛔ THE HOMES LIVE IN A tmpfs -- RAM, NOT DISK -- AND NOTHING EVER REAPED
    # THEM. 48 dead ones once held a 51 GB runtime dir at 100% full, after which
    # `start` failed and the whole harness reported clean results it never
    # rendered. `stop` preserves by design (the inspection case), so the reaping
    # has to be its own verb.
    # ⛔ AND THE HOST IS SHARED: another session's sandbox may be LIVE in this
    # same runtime dir, so every directory is liveness-checked on its OWN
    # compositor pid and skipped if it answers. A blanket rm here takes down
    # work that is not yours.
    reaped=0; kept=0
    for d in "$XDG_RUNTIME_DIR"/yggterm-uglass/*/; do
      [ -d "$d" ] || continue
      name="$(basename "$d")"
      pidf="$d/sway.pid"
      alive=0
      if [ -f "$pidf" ]; then
        pid="$(cat "$pidf" 2>/dev/null || true)"
        if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then alive=1; fi
      fi
      # A pid file can go missing while the compositor lives, so ask the config
      # path too -- the same discriminator `stop` uses to find a sway the pid
      # file failed to name.
      if [ "$alive" -eq 0 ] && pgrep -f "sway -c $d/sway.conf" >/dev/null 2>&1; then alive=1; fi
      if [ "$alive" -eq 1 ]; then
        echo "  keep  $name (compositor alive)"
        kept=$((kept + 1))
        continue
      fi
      size="$(du -sh "$d" 2>/dev/null | cut -f1)"
      rm -rf "$d"
      echo "  reap  $name ($size)"
      reaped=$((reaped + 1))
    done
    echo "reaped $reaped dead sandbox home(s), kept $kept live"
    df -h "$XDG_RUNTIME_DIR" | tail -1
    ;;
  *)
    echo "usage: $0 <start|capture|burst|cursor|click|scroll|backend|env|stop|reap> [args] [--name id] [--size WxH] [--under-glass 0|1] [--env K=V] [--xwayland]" >&2
    exit 2
    ;;
esac
