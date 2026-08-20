#!/usr/bin/env bash
# corner-contract.sh — the ROUNDED-CORNER CONTRACT, asserted in PIXELS.
#
# WHY THIS EXISTS
#   yggterm draws its own chrome, so its window corners are its own problem, and
#   they have cycled fixed→broken for the product's life. Fixed-then-broken means
#   UNTESTED: every previous fix was verified by looking at a screenshot once, and
#   nothing afterwards could notice when a launch path, a compositing flag or a
#   desktop-detection rule squared them off again.
#
#   ⛔ THE INSTRUMENTS THAT ALREADY EXISTED CANNOT SEE THIS FAULT, and that is the
#   whole reason the cycle survived:
#     · `app state → dom.shell_root_border_radius` reads the CSS the page ASKED
#       for. It said `10px` on a window whose corners were square, because the
#       radius is applied in the DOM and DEFEATED further down (an opaque surface,
#       a compositing mode, an X11-only shape call on a Wayland session). A DOM
#       probe cannot fail on a pixel that never got drawn.
#     · `server app screenshot` returns an RGB PNG — the WebKit snapshot FLATTENS
#       alpha. A corner that was rounded away comes back as opaque background, so
#       the default capture reports square and rounded identically. `capture_faithful:true`
#       is a claim about the xterm canvas, never about the window's edge.
#   ⇒ The only honest eye is a COMPOSITOR grab of the real surface against a
#     known background, which is what this script takes.
#
# WHAT IT ASSERTS
#   Against a headless sway whose background is a known solid colour, on a
#   Wayland-NATIVE session (XWayland off — the live desktop is Wayland-native and
#   an XWayland arm measures a different machine):
#     floating arm    the four corners are the COMPOSITOR's background (rounded
#                     away), the window's own paint is absent from them, and the
#                     cut follows a quarter-circle rather than a notch or a bevel;
#     maximized arm   the four corners are the WINDOW's paint (square by design —
#                     a maximized window squares off, that is the contract, not a
#                     regression).
#   The floating arm — the one that has actually regressed, repeatedly — must
#   hold, and a failure there is fatal. An arm the RACK cannot drive (this
#   compositor declines to set a window's maximized state) is announced in the
#   open, every run, and names the unit test that covers the same rule; it is
#   never silently absent, because a skipped corner test reads exactly like a
#   passing one in a log. A missing TOOL is fatal for the same reason.
#
# USAGE
#   scripts/corner-contract.sh check [--radius N] [--keep] [--name NAME]
#   scripts/corner-contract.sh explain          # what the arms mean, no GUI needed
#
# Requires: sway, grim, python3 with Pillow. Same rack as underglass-sandbox.sh.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SANDBOX="$REPO_ROOT/scripts/underglass-sandbox.sh"
NAME="corner-contract"
RADIUS=10
KEEP=0
CMD="${1:-check}"
[ $# -gt 0 ] && shift || true

while [ $# -gt 0 ]; do
  case "$1" in
    --radius) RADIUS="${2:-10}"; shift 2 ;;
    --name)   NAME="${2:-$NAME}"; shift 2 ;;
    --keep)   KEEP=1; shift ;;
    *) echo "corner-contract: unknown argument '$1'" >&2; exit 2 ;;
  esac
done

# The sandbox paints its output this colour; it is the oracle a rounded corner
# must reveal. Kept in step with underglass-sandbox.sh's `output * bg`.
SWAY_BG="16,20,24"
WORK="${TMPDIR:-/tmp}/yggterm-corner-contract"
mkdir -p "$WORK"

if [ "$CMD" = "explain" ]; then
  sed -n '3,45p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
  exit 0
fi
[ "$CMD" = "check" ] || { echo "corner-contract: unknown command '$CMD'" >&2; exit 2; }

fail() { echo "corner-contract: ⛔ $*" >&2; FAILED=1; }
need() { command -v "$1" >/dev/null 2>&1 || { echo "corner-contract: UNTESTED — '$1' is not installed. A corner arm that cannot run is not a pass." >&2; exit 3; }; }
need sway; need grim
python3 -c 'import PIL' 2>/dev/null || { echo "corner-contract: UNTESTED — python3 Pillow missing. A corner arm that cannot run is not a pass." >&2; exit 3; }

FAILED=0
RUN_DIR="/run/user/$(id -u)/yggterm-uglass/$NAME"

cleanup() { [ "$KEEP" = 1 ] || bash "$SANDBOX" stop --name "$NAME" >/dev/null 2>&1; }
trap cleanup EXIT

# ⛔ THE ARM RUNS WITH NO TRANSPARENCY OPT-IN, DELIBERATELY.
# `YGGTERM_ENABLE_TRANSPARENT_WINDOW=1` short-circuits the window profile to
# `explicit_opt_in`, so an arm carrying it rounds its corners no matter what the
# profile would have decided on its own — it tests the RENDERING and blinds
# itself to the DECISION, which is the half that actually broke. A bare sway
# session is a non-KDE Wayland desktop, exactly the case that used to fall to
# `wayland_opaque_default` and square off; leaving the opt-in out is what makes
# this test able to fail.
#
# The sandbox resolver prefers ~/.local/bin/yggterm — the INSTALLED build — so a
# working tree with the fix in it would be tested through a binary without it.
# Point it at this checkout's build unless the caller named one.
if [ -z "${YGGTERM_GUI_BIN:-}" ] && [ -x "$REPO_ROOT/target/release/yggterm" ]; then
  export YGGTERM_GUI_BIN="$REPO_ROOT/target/release/yggterm"
fi
echo "corner-contract: gui=${YGGTERM_GUI_BIN:-<resolver default>}"
echo "corner-contract: starting Wayland-native arm (radius ${RADIUS}px, bg #101418)"
bash "$SANDBOX" stop --name "$NAME" >/dev/null 2>&1
bash "$SANDBOX" start --name "$NAME" --under-glass 0 >/dev/null 2>&1 \
  || { echo "corner-contract: UNTESTED — sandbox failed to start" >&2; exit 3; }

# The sandbox compositor tiles with a titlebar and a border by default. Those
# belong to SWAY, not to yggterm, and sampling them instead of the window's own
# edge is how this measurement lies: a scan from (0,0) reads sway's decoration,
# finds paint, and reports square corners on a perfectly rounded window. Strip
# the decoration so the surface is edge-to-edge and screen coordinates ARE window
# coordinates.
SWAYSOCK="/run/user/$(id -u)/sway-ipc.$(id -u).$(cat "$RUN_DIR/sway.pid").sock"
export SWAYSOCK
swaymsg 'default_border none' >/dev/null 2>&1
swaymsg '[title="Yggterm"] border none' >/dev/null 2>&1
sleep 1

assert_arm() {  # <arm> <expect: rounded|square> <shot>
  local arm="$1" expect="$2" shot="$3"
  bash "$SANDBOX" capture "$shot" --name "$NAME" >/dev/null 2>&1 \
    || { fail "$arm: capture failed"; return; }
  python3 - "$shot" "$expect" "$RADIUS" "$SWAY_BG" "$arm" <<'PY'
import sys
from PIL import Image
shot, expect, radius, bg, arm = sys.argv[1], sys.argv[2], int(sys.argv[3]), tuple(int(v) for v in sys.argv[4].split(",")), sys.argv[5]
im = Image.open(shot).convert("RGB")
w, h = im.size
px = im.load()
# Each corner as (name, x-direction, y-direction, origin).
corners = [("TL", 1, 1, (0, 0)), ("TR", -1, 1, (w - 1, 0)),
           ("BL", 1, -1, (0, h - 1)), ("BR", -1, -1, (w - 1, h - 1))]
problems = []
# The window must actually be on screen and painting: a blank output would pass
# a "corner is background" test trivially, which is the classic false green.
cx, cy = w // 2, h // 2
if px[cx, cy] == bg:
    problems.append(f"the window is not painting at all (centre pixel is the compositor background) — this arm proves nothing")
for name, dx, dy, (ox, oy) in corners:
    tip = px[ox, oy]
    if expect == "rounded" and tip != bg:
        problems.append(f"{name} tip is {tip}, expected the compositor background {bg} — the corner is SQUARE")
    if expect == "square":
        # ⛔ "not the compositor background" is NOT enough. A rounded corner over
        # a surface with nothing behind it composites onto BLACK, which is also
        # not the background — so the weak form of this check passes on the very
        # frame it exists to catch. The corner must be the window's OWN paint.
        if tip != px[cx, cy]:
            problems.append(
                f"{name} tip is {tip} but the window paints {px[cx, cy]} — a squared-off "
                f"corner must be the window's own colour, not background and not black"
            )
if expect == "rounded" and not problems:
    # A rounded corner is a quarter-circle: the number of background pixels per
    # row must DECREASE monotonically as you walk down the arc, and must reach 0
    # by the radius. A notch, a bevel or a chopped triangle all satisfy "the tip
    # is background" and are caught only here.
    for name, dx, dy, (ox, oy) in corners:
        runs = []
        for step in range(radius + 2):
            y = oy + dy * step
            n = 0
            while n < radius + 2 and px[ox + dx * n, y] == bg:
                n += 1
            runs.append(n)
        if runs[0] < 1:
            problems.append(f"{name} has no arc at all (first row shows {runs[0]} background px)")
        if any(b > a for a, b in zip(runs, runs[1:])):
            problems.append(f"{name} arc is not monotonic: {runs} — that is a notch or a bevel, not a radius")
        if runs[-1] != 0:
            problems.append(f"{name} arc never closes within {radius}px: {runs}")
print(f"  {arm}: corners={[px[c[3]] for c in corners]} centre={px[cx, cy]}")
if problems:
    for p in problems:
        print(f"  ⛔ {p}")
    sys.exit(1)
print(f"  ✅ {arm}: corners are {expect} as the contract requires")
PY
  [ $? -eq 0 ] || fail "$arm arm failed its pixel assertion (frame kept at $shot)"
}

assert_arm "floating" rounded "$WORK/floating.png"

# The other half of the contract. A maximized window squares off by design, and
# a test that only ever checks the rounded case cannot tell "rounding works" from
# "rounding is stuck ON".
#
# ⛔ MAXIMIZE THROUGH THE APP, NEVER THROUGH THE COMPOSITOR. `swaymsg fullscreen
# enable` looks like it does the same thing and does not: it sets the surface's
# FULLSCREEN state, which the window's `maximized` flag never sees, so the app
# goes on drawing its 10px radius over a fullscreen surface with nothing behind
# it. The corners then composite onto BLACK — and a naive assertion of "the
# corner is not the compositor background" PASSES on that, reporting a squared
# window that is still visibly rounded. This arm was written that way first and
# passed for exactly that wrong reason.
echo 'corner-contract: maximizing (through the app, so the maximized flag is really set)'
# ⚠ A TILED WINDOW CANNOT BE MAXIMIZED. sway tiles by default and ignores
# `set_maximized` on a tiled surface, so `app maximize on` returns success and
# the window goes on reporting `maximized: false` — the arm then measures the
# floating case twice and calls that coverage. Float the window first; only a
# floating window has a maximized state to set.
swaymsg '[title="Yggterm"] floating enable' >/dev/null 2>&1
swaymsg '[title="Yggterm"] border none' >/dev/null 2>&1
sleep 1
HOME="$RUN_DIR/home" YGGTERM_HOME="$RUN_DIR/home/.yggterm"   "${YGGTERM_GUI_BIN:-yggterm}" server app maximize on >/dev/null 2>&1
sleep 2
maximized_state="$(HOME="$RUN_DIR/home" YGGTERM_HOME="$RUN_DIR/home/.yggterm"   "${YGGTERM_GUI_BIN:-yggterm}" server app state 2>/dev/null   | python3 -c 'import sys,json; print(json.load(sys.stdin)["data"]["window"]["maximized"])' 2>/dev/null)"
if [ "$maximized_state" != "True" ]; then
  # ⚠ NAMED, NOT SWALLOWED. `app maximize on` answers `enabled:true` and the
  # window goes on reporting `maximized:false` — the verb reports the REQUEST,
  # not the EFFECT, and this compositor declines to set the state. That is a
  # limitation of the HARNESS, not a failure of the contract, so it must not be
  # dressed up as either a pass or a regression.
  #
  # The squaring RULE is not left unguarded by this: it is asserted in-suite by
  # `opaque_linux_shell_drops_radius_when_native_shape_is_unavailable`
  # (crates/yggterm-shell — `shell_effective_radius_for_platform(r, true, …) == 0`).
  # What stays unproven here is only the PIXEL half of the maximized arm, and it
  # is stated in the open every run rather than being quietly absent from a log.
  echo "  ⚠ maximized arm SKIPPED — this compositor would not set the maximized"
  echo "    state (app maximize answered enabled:true, window still reports"
  echo "    maximized:${maximized_state:-<no answer>}). The squaring rule is covered by the"
  echo "    unit test; its pixel proof is not available on this rack."
  SKIPPED_ARMS=1
else
  assert_arm "maximized" square "$WORK/maximized.png"
fi
HOME="$RUN_DIR/home" YGGTERM_HOME="$RUN_DIR/home/.yggterm"   "${YGGTERM_GUI_BIN:-yggterm}" server app maximize off >/dev/null 2>&1

if [ "$FAILED" = 0 ]; then
  if [ "${SKIPPED_ARMS:-0}" = 1 ]; then
    echo "corner-contract: ✅ the rounded arm holds (a maximized arm was skipped, named above)"
  else
    echo "corner-contract: ✅ both arms hold"
  fi
  exit 0
fi
echo "corner-contract: ⛔ contract violated — frames in $WORK" >&2
exit 1
