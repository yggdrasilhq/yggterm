#!/usr/bin/env bash
# Screen-tearing probe for a yggterm WEB SURFACE. One A/B arm, one number.
#
# WHY THIS EXISTS. The user reports "screen tearing while scroll or animations"
# in the ychrome viewport. Tearing is a thing the EYE sees, so the temptation is
# to squint at screenshots. This instrument decodes instead:
# `tools/tear-probe/page.html` paints every content band with a colour that
# encodes its own content-space row index plus a checksum, and
# `tools/tear-probe/analyze.py` decodes a captured frame back into "which scroll
# positions is this single frame made of". One position = clean. Two = a tear,
# and the step between them is its magnitude in pixels. See both files.
#
#   run --label NAME [options]     one arm, end to end, prints the number
#   arms                           the standard A/B matrix, one arm at a time
#
# run options:
#   --mode scroll|jsscroll|anim3d|anim2d
#                                  scroll = real KEYBOARD scroll through the
#                                  compositor (wlrctl's wheel is a proven dead
#                                  end here -- see the driver below);
#                                  anim3d = translate3d (forced compositing
#                                  layer, the ACCELERATED path);
#                                  anim2d = top: (the REPAINT path, no layer)
#   --frames N                     frames per burst (default 40)
#   --under-glass 0|1              Phase F arming (default 1: what jojo runs)
#   --env K=V                      extra GUI env, repeatable
#   --port N                       loopback fixture port (default 8791)
#   --heavy                        expensive per-frame paint (a cheap page can
#                                  produce a false negative -- see page.html)
#   --keep                         leave the sandbox up for inspection
#
# ⚠ THE HONEST LIMITS OF THIS INSTRUMENT, stated up front because a null result
# here is a real result and must not be over-read:
#   1. grim copies the COMPOSITED OUTPUT BUFFER. A wl_surface commit is atomic,
#      so a torn grim frame proves the CLIENT committed a half-updated buffer
#      (content tearing). True SCANOUT tearing -- the display controller
#      switching buffers mid-scan -- is invisible to this instrument, on any
#      compositor. On KWin/Wayland with no tearing-control opt-in (and jojo's
#      kwinrc has none) scanout tearing should be impossible anyway, which is
#      why content tearing is the hypothesis under test.
#   2. The sandbox compositor is sway, not KWin. Same protocol, different
#      damage/present scheduling.
#   3. The sandbox GPU is whatever this host has. `backend` prints the probe
#      class and driver for every run -- compare it against jojo's before
#      treating any number as transferable.
#   4. The sandbox GUI is Wayland-NATIVE and its compositor has XWayland
#      DISABLED, matching the live GUI on jojo. `underglass-sandbox.sh backend`
#      asserts it every run; if it ever prints gdk_backend=x11 the whole run is
#      answering a different question and must be discarded.

set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/.." && pwd)"
SANDBOX="$HERE/underglass-sandbox.sh"
ANALYZE="$REPO/tools/tear-probe/analyze.py"
FIXTURES="$REPO/tools/tear-probe"

CMD="${1:-}"
[ $# -gt 0 ] && shift || true

LABEL=""
MODE="scroll"
FRAMES=40
UNDER_GLASS=1
PORT=8791
BAND=4
KEEP=0
HEAVY=0
EXTRA_ENV=()
# (wlrctl wheel is a dead end -- see the scroll driver below)
SCROLL_GAP_MS=0

while [ $# -gt 0 ]; do
  case "$1" in
    --label) LABEL="$2"; shift 2 ;;
    --mode) MODE="$2"; shift 2 ;;
    --frames) FRAMES="$2"; shift 2 ;;
    --under-glass) UNDER_GLASS="$2"; shift 2 ;;
    --env) EXTRA_ENV+=(--env "$2"); shift 2 ;;
    --port) PORT="$2"; shift 2 ;;
    --band) BAND="$2"; shift 2 ;;
    --heavy) HEAVY=1; shift ;;
    --keep) KEEP=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

[ -n "$LABEL" ] || LABEL="$MODE"
NAME="tear-$LABEL"
OUTROOT="${YGGTERM_TEAR_OUT:-/tmp/yggterm-tear}"
OUTDIR="$OUTROOT/$LABEL"
SANDBOX_HOME="$XDG_RUNTIME_DIR/yggterm-uglass/$NAME/home"

serve() {
  if ! curl -sS -o /dev/null "http://127.0.0.1:$PORT/page.html" 2>/dev/null; then
    (cd "$FIXTURES" && setsid python3 -m http.server "$PORT" --bind 127.0.0.1 \
      >"$OUTROOT/http-$PORT.log" 2>&1 &)
    for _ in $(seq 1 40); do
      sleep 0.25
      curl -sS -o /dev/null "http://127.0.0.1:$PORT/page.html" 2>/dev/null && return 0
    done
    echo "fixture server did not come up on $PORT" >&2
    exit 4
  fi
}

# The headless CLI is wherever this host keeps it: a repo build on a dev box,
# the installed sibling on the GUI host. Named, never guessed.
HEADLESS_BIN="${YGGTERM_HEADLESS_BIN:-$REPO/target/release/yggterm-headless}"
# `server app web ...` lives on the GUI binary's CLI, not the headless one.
GUI_CLI="${YGGTERM_GUI_CLI:-$REPO/target/release/yggterm}"
ygg() { YGGTERM_HOME="$SANDBOX_HOME/.yggterm" HOME="$SANDBOX_HOME" "$HEADLESS_BIN" "$@"; }
yggui() { YGGTERM_HOME="$SANDBOX_HOME/.yggterm" HOME="$SANDBOX_HOME" "$GUI_CLI" "$@"; }

# numpy+PIL are how a frame becomes a number. A GUI host may not have them --
# jojo does not -- and that must NOT stop the arm from RUNNING there, because
# the GPU under suspicion is on that host. Without them the driver falls back to
# a page-side readiness probe and leaves the frames for central analysis.
if python3 -c "import numpy, PIL" >/dev/null 2>&1; then
  CAN_ANALYZE=1
else
  CAN_ANALYZE=0
fi

case "$CMD" in
  run) ;;
  arms)
    # The standard matrix. ONE variable per arm, each a separate GUI process,
    # each printing its own number. Sequential on purpose: two armed GUIs on one
    # host share a GPU and would confound each other.
    for spec in \
      "anim2d|--mode anim2d" \
      "scroll|--mode scroll" \
      "noglass-anim3d|--mode anim3d --under-glass 0" \
      "noglass-anim2d|--mode anim2d --under-glass 0" \
      "noglass-scroll|--mode scroll --under-glass 0" \
      "shm-anim3d|--mode anim3d --env WEBKIT_DISABLE_DMABUF_RENDERER=1" \
      "shm-anim2d|--mode anim2d --env WEBKIT_DISABLE_DMABUF_RENDERER=1" \
      "softgl-anim3d|--mode anim3d --env YGGTERM_FORCE_SOFTWARE_GL=1" \
      "softgl-anim2d|--mode anim2d --env YGGTERM_FORCE_SOFTWARE_GL=1" \
      "nocomposite-anim3d|--mode anim3d --env WEBKIT_DISABLE_COMPOSITING_MODE=1" \
      "nocomposite-scroll|--mode scroll --env WEBKIT_DISABLE_COMPOSITING_MODE=1" \
      ; do
      lbl="${spec%%|*}"; rest="${spec#*|}"
      # shellcheck disable=SC2086
      "$0" run --label "$lbl" $rest --frames "$FRAMES" || echo "[$lbl] ARM FAILED"
    done
    exit 0
    ;;
  *)
    sed -n '2,50p' "$0" >&2
    exit 2
    ;;
esac

mkdir -p "$OUTROOT" "$OUTDIR"
rm -f "$OUTDIR"/*.png

"$SANDBOX" stop --name "$NAME" >/dev/null 2>&1 || true
"$SANDBOX" start --name "$NAME" --under-glass "$UNDER_GLASS" "${EXTRA_ENV[@]}" >/dev/null
serve

# The backend assertion is a GATE, not a footnote: a run that silently landed on
# XWayland is answering a different question than the user's Wayland-native GUI.
backend="$("$SANDBOX" backend --name "$NAME")"
echo "[$LABEL] $backend"
case "$backend" in
  *gdk_backend=x11*) echo "[$LABEL] REFUSED: sandbox is XWayland, live GUI is Wayland-native" >&2; exit 5 ;;
esac

url="http://127.0.0.1:$PORT/page.html?mode=$MODE&band=$BAND&n=6000&speed=1400&span=3000&heavy=$HEAVY"
sp="$(ygg server app terminal new --kind shell --cwd /tmp --title "tear-$LABEL" \
      --purpose "web tearing probe" \
      | python3 -c 'import json,sys; print(json.load(sys.stdin)["data"]["session_path"])')"

cat > "$OUTROOT/declare.sh" <<'EOS'
url="$1"
emit() {
  printf '{"session":"%s","url":"%s","title":"tear-probe"}' "${YGGTERM_SESSION_ID:-none}" "$url" \
    | base64 -w0 | { read -r b64; printf '\033]7717;web-surface;%s;%s\a' "$2" "$b64"; }
}
emit "$url" open
while true; do emit "$url" heartbeat; sleep 4; done
EOS
ygg server app terminal send "$sp" --data "bash $OUTROOT/declare.sh '$url'
" >/dev/null

# Wait for the PAGE, judged by the page itself: the probe decodes, or it does
# not. No sleep-and-hope, and no telemetry field standing in for pixels.
cx=0; cy=0
if [ "$CAN_ANALYZE" = 1 ]; then
  rect=""
  for _ in $(seq 1 60); do
    sleep 0.5
    "$SANDBOX" capture "$OUTDIR/ready.png" --name "$NAME"
    rect="$(python3 "$ANALYZE" "$OUTDIR/ready.png" --band "$BAND" --json --per-frame \
      | python3 -c 'import json,sys; f=json.load(sys.stdin)["frames"][0]; print(f["page_rows"], f["page_x0"], f["page_x1"], f["page_y0"], f["page_y1"])')"
    [ "$(echo "$rect" | cut -d' ' -f1)" -gt 300 ] 2>/dev/null && break
    rect=""
  done
  if [ -z "$rect" ]; then
    echo "[$LABEL] REFUSED: the probe page never appeared in the surface" >&2
    [ "$KEEP" = 1 ] || "$SANDBOX" stop --name "$NAME" >/dev/null 2>&1 || true
    exit 6
  fi
  set -- $rect
  px0="$2"; px1="$3"; py0="$4"; py1="$5"
  cx=$(( (px0 + px1) / 2 )); cy=$(( (py0 + py1) / 2 ))
  echo "[$LABEL] page rect x=$px0..$px1 y=$py0..$py1 pointer=($cx,$cy)"
  rm -f "$OUTDIR/ready.png"
else
  # No decoder on this host: ask the PAGE whether it is alive and animating.
  # Advancing rAF frames are the load gauge; a stalled page sits at its first
  # value and the arm refuses rather than capturing 50 identical frames.
  ready=0
  for _ in $(seq 1 60); do
    sleep 0.5
    v="$(yggui server app web eval 'JSON.stringify([window.__tearReady===true, window.__tearFrames|0])' \
        --session "$sp" 2>/dev/null | python3 -c 'import json,sys
try:
    print(json.load(sys.stdin)["data"]["value"])
except Exception:
    print("")' || true)"
    case "$v" in
      *true*)
        seen="$(echo "$v" | tr -dc '0-9,' | cut -d, -f2)"
        if [ "${seen:-0}" -gt 30 ]; then ready=1; break; fi
        ;;
    esac
  done
  if [ "$ready" != 1 ]; then
    echo "[$LABEL] REFUSED: the probe page never reported ready (web eval)" >&2
    [ "$KEEP" = 1 ] || "$SANDBOX" stop --name "$NAME" >/dev/null 2>&1 || true
    exit 6
  fi
  echo "[$LABEL] page ready (no local decoder; frames kept for central analysis)"
  if [ "$MODE" = scroll ]; then
    echo "[$LABEL] REFUSED: --mode scroll needs the local decoder for the page rect; use --mode jsscroll on this host" >&2
    [ "$KEEP" = 1 ] || "$SANDBOX" stop --name "$NAME" >/dev/null 2>&1 || true
    exit 7
  fi
fi

if [ "$cx" -gt 0 ]; then "$SANDBOX" cursor "$cx" "$cy" --name "$NAME"; fi

if [ "$MODE" = scroll ]; then
  # ⚠ `wlrctl pointer scroll` IS A DEAD END on wlrctl 0.2.2 + this wlroots:
  # the axis event is accepted and delivered NOWHERE. Proven, not assumed --
  # a `wheel` listener installed in the page counted 0 events across
  # `scroll 100 0`, `scroll 0 100` and `scroll -15 0`, while a `mousemove`
  # listener counted the pointer moves from the SAME tool in the same run.
  # Keyboard scrolling is real input through the same compositor->GTK->WebKit
  # path and it works (~21 px per Down, verified against window.scrollY), so
  # that is the driver. Key auto-repeat does NOT happen for a virtual keyboard
  # (a 3 s held Down moved scrollY by 0), hence one -k per step.
  need_wtype=$(command -v wtype || true)
  [ -n "$need_wtype" ] || { echo "[$LABEL] REFUSED: wtype missing (scroll driver)" >&2; exit 7; }
  "$SANDBOX" click "$cx" "$cy" --name "$NAME"
  keys=""
  for _ in $(seq 1 600); do keys="$keys -k Down"; done
  # shellcheck disable=SC2086
  ( WAYLAND_DISPLAY="$(cat "$XDG_RUNTIME_DIR/yggterm-uglass/$NAME/wayland-display")" \
      wtype -d 12 $keys >/dev/null 2>&1 || true ) &
  driver=$!
  "$SANDBOX" burst "$OUTDIR" "$FRAMES" "$SCROLL_GAP_MS" --name "$NAME" >/dev/null
  kill "$driver" 2>/dev/null || true
  wait "$driver" 2>/dev/null || true
else
  "$SANDBOX" burst "$OUTDIR" "$FRAMES" 0 --name "$NAME" >/dev/null
fi

if [ "$CAN_ANALYZE" != 1 ]; then
  echo "[$LABEL] captured $FRAMES frames in $OUTDIR (no numpy/PIL here -- copy them to a host with the decoder and run tools/tear-probe/analyze.py)"
  [ "$KEEP" = 1 ] || "$SANDBOX" stop --name "$NAME" >/dev/null 2>&1 || true
  exit 0
fi

python3 "$ANALYZE" "$OUTDIR" --band "$BAND" --label "$LABEL" --json --per-frame \
  > "$OUTDIR/result.json"
python3 "$ANALYZE" "$OUTDIR" --band "$BAND" --label "$LABEL"

# A measurement must be able to REFUSE (field guide §7.3). An arm whose frames
# all show the SAME content position never loaded the renderer, and its zero
# tears mean nothing. `distinct_positions` is the load gauge and it is checked,
# not eyeballed.
python3 - "$OUTDIR/result.json" "$LABEL" <<'PY'
import json, sys
s = json.load(open(sys.argv[1]))["summary"]
if s["usable_frames"] < 8 or s["distinct_positions"] < max(4, s["usable_frames"] // 4):
    print(
        f"[{sys.argv[2]}] ⚠ ARM DID NOT LOAD THE RENDERER: "
        f"usable={s['usable_frames']} distinct_positions={s['distinct_positions']}. "
        "Its tear count is not evidence.",
        file=sys.stderr,
    )
    raise SystemExit(8)
PY

[ "$KEEP" = 1 ] || "$SANDBOX" stop --name "$NAME" >/dev/null 2>&1 || true
