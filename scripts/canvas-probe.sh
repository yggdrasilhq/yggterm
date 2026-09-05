#!/usr/bin/env bash
# Draw a terminal canvas and photograph it, WITHOUT going near the owner's view.
#
# ⛔ THE REASON THIS EXISTS (the hourly eye check it was built for was retired
# 2026-09-05, but the lesson — and the render faults — are not): a screenshot
# of whatever is on screen is NOT a canvas test. With no session open the GUI
# shows the start page and `capture_faithful` still reports `true` — it means
# "an honest picture of the screen", never "the terminal was captured". Only a
# deliberate draw-and-photograph of a row we own exercises the xterm surface
# the render faults live in. See the [6.7] history in docs/pending-bugs.md.
#
# The two things that make this safe are the whole design, and both were learned
# by getting them wrong first:
#
#   1. ⛔ IT MUST DRAW A ROW WE OWN. A shadow client attaches to whatever session
#      is current, so capturing "whatever is open" photographs other agents'
#      live work — private material included — and writes it to disk. This
#      creates its own ephemeral probe row and points the shadow at THAT.
#   2. ⛔ IT MUST NOT TAKE THE FOREGROUND. The probe row is created
#      `--no-activate`, and the shadow runs on its own headless compositor at
#      `--client-role shadow`, which the daemon role-gates against every
#      ownership-claiming request. The owner's client is never addressed.
#
# ⚠ AND THE LIMIT, SO NOBODY OVER-READS A GREEN CARD: the shadow renders under
# headless sway, not the owner's desktop compositor and GL path. A clean card is
# INCONCLUSIVE, NOT EXONERATING — the same rule that governs the sandbox. This
# answers "a canvas was drawn and here is a faithful frame of it". It never
# answers "the owner's terminal is rendering correctly".
#
# ⚠ Deliberately NOT wired into the hourly check. A second GUI plus a compositor
# every hour is real memory on the machine this campaign exists to protect, and
# the result would not cover the owner's canvas anyway. Run it when investigating
# a render fault, not on a timer.
#
#   scripts/canvas-probe.sh [out.png]
#
# Everything it creates is torn down on EXIT, including on failure and on Ctrl-C.
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-canvas-probe.png}"

HOST="$("$REPO_ROOT/scripts/ygg-live-host.sh" 2>/dev/null)"
if [ -z "${HOST:-}" ]; then
  echo "cannot resolve the live desktop host - scripts/ygg-live-host.sh is its one owner" >&2
  exit 2
fi
SSH="ssh -o BatchMode=yes -o ConnectTimeout=10 $HOST"

# ⛔ NOT /tmp on that host: it is a tmpfs, so a screenshot written there is RAM.
REMOTE_DIR='~/.yggterm/scratchpad'
SHADOW_NAME="canvas-probe-$$"
PROBE=""
SHADOW_UP=0

cleanup() {
  # ⛔ Unconditional. A probe row that outlives its run is a row the owner sees
  # and did not ask for, and a shadow client that outlives it is a second GUI
  # burning memory on the machine being watched.
  [ "$SHADOW_UP" -eq 1 ] && $SSH "cd ~/gh/yggterm && ./scripts/shadow-client.sh stop --name $SHADOW_NAME" >/dev/null 2>&1
  if [ -n "$PROBE" ]; then
    # `server app session remove`, NOT `session remove` — and read BOTH fields
    # back, because this verb reports the request rather than the effect.
    local removed
    removed="$($SSH "~/.yggterm/bin/yggterm-headless server app session remove '$PROBE'" 2>/dev/null)"
    case "$removed" in
      *'"verified": true'*) : ;;
      *) echo "⚠ probe row $PROBE may still exist - check 'server app rows'" >&2 ;;
    esac
  fi
}
trap cleanup EXIT INT TERM

say() { echo "canvas-probe: $*"; }

# ---------------------------------------------------------------- the probe row
PROBE="$($SSH "~/.yggterm/bin/yggterm-headless server app terminal new \
  --kind shell --title '6.7 canvas probe' \
  --purpose 'render-fault canvas capture' \
  --no-activate --ephemeral --ephemeral-idle-ttl-secs 300" 2>/dev/null \
  | grep -oE '"session_path": *"[^"]*"' | head -1 | sed 's/.*"\([^"]*\)"$/\1/')"
if [ -z "$PROBE" ]; then
  echo "could not create a probe row - the app cannot open a new terminal" >&2
  exit 3
fi
say "probe row $PROBE (ephemeral, --no-activate)"

# ---------------------------------------------------------------- the shadow
if ! $SSH "cd ~/gh/yggterm && ./scripts/shadow-client.sh start --name $SHADOW_NAME" >/dev/null 2>&1; then
  echo "could not start a shadow client (needs sway + grim on the desktop host)" >&2
  exit 4
fi
SHADOW_UP=1
SHADOW_PID="$($SSH "~/.yggterm/bin/yggterm-headless server app clients" 2>/dev/null \
  | python3 -c "
import json,sys
try:
    for c in json.load(sys.stdin).get('clients', []):
        if c.get('client_role') == 'shadow':
            print(c.get('pid') or '')
            break
except Exception:
    pass
")"
if [ -z "${SHADOW_PID:-}" ]; then
  echo "the shadow client did not register - nothing to capture" >&2
  exit 5
fi
say "shadow client pid=$SHADOW_PID"

# ⭐ `--pid` is what keeps this off the owner's screen: it addresses ONE client.
$SSH "~/.yggterm/bin/yggterm-headless server app open '$PROBE' --view terminal --pid $SHADOW_PID" >/dev/null 2>&1
sleep 2

# ---------------------------------------------------------------- draw the card
$SSH "mkdir -p $REMOTE_DIR" >/dev/null 2>&1
scp -q "$REPO_ROOT/scripts/lib/canvas-card.sh" "$HOST:.yggterm/scratchpad/canvas-card.sh" 2>/dev/null
# ⛔ The Enter key is a SEPARATE write of a lone newline, after a beat. A command
#    and its terminator sent as one write is not what a keyboard does.
$SSH "~/.yggterm/bin/yggterm-headless server app terminal send '$PROBE' --data 'bash ~/.yggterm/scratchpad/canvas-card.sh'" >/dev/null 2>&1
sleep 1
$SSH "~/.yggterm/bin/yggterm-headless server app terminal send '$PROBE' --data '
'" >/dev/null 2>&1
sleep 3

# ---------------------------------------------------------------- capture
SHOT="$($SSH "~/.yggterm/bin/yggterm-headless server app screenshot $REMOTE_DIR/canvas-probe.png --pid $SHADOW_PID" 2>/dev/null)"
BACKEND="$(grep -o '"capture_backend": *"[^"]*"' <<<"$SHOT" | head -1 | sed 's/.*"\([^"]*\)"$/\1/')"
FAITHFUL="$(grep -o '"capture_faithful": *[a-z]*' <<<"$SHOT" | head -1 | grep -o '[a-z]*$')"
DREW="$(grep -o '"active_session_path": *"[^"]*"' <<<"$SHOT" | head -1 | sed 's/.*"\([^"]*\)"$/\1/')"

say "capture_backend=${BACKEND:-unknown} capture_faithful=${FAITHFUL:-unknown}"
# ⛔ Prove the frame is OF OUR ROW. If the shadow drifted to another session the
#    capture holds someone else's work, and must not be kept.
if [ "${DREW:-}" != "$PROBE" ]; then
  echo "⛔ the shadow drew '${DREW:-nothing}', not our probe row - discarding the capture" >&2
  $SSH "rm -f $REMOTE_DIR/canvas-probe.png" >/dev/null 2>&1
  exit 6
fi
case "${BACKEND:-unknown}" in
  xterm_canvas_composite*) : ;;
  *)
    echo "⛔ backend '${BACKEND:-unknown}' is not the canvas compositor - the frame is canvas-blind" >&2
    exit 7
    ;;
esac

scp -q "$HOST:.yggterm/scratchpad/canvas-probe.png" "$OUT" 2>/dev/null
$SSH "rm -f $REMOTE_DIR/canvas-probe.png" >/dev/null 2>&1
if [ ! -s "$OUT" ]; then
  echo "could not retrieve the capture" >&2
  exit 8
fi

say "wrote $OUT"
say "⇒ NOW LOOK AT IT. Expect a full glyph sweep, three rows of six coloured runs,"
say "  and three identical control lines. Missing characters mid-word are the"
say "  BLANKING fault; wrong characters throughout are the SUBSTITUTION fault."
say "⚠ A clean card is INCONCLUSIVE, not exonerating: this renders under headless"
say "  sway, not the owner's compositor and GL path."
