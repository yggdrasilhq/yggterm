#!/bin/bash
# [s56-merge-key] THE MERGE-KEY / Q4-MATRIX RIG — the s54/s55 measurement
# machinery rebuilt as a durable tool (the sitting rigs were /tmp-ephemeral).
#
# Runs the GUI hermetically on its own Xvfb display, drives the six Q4
# paint-scenario classes + the warm-merge leg, and captures the
# component_window spans, the sidebar merge_rows_breakdown lines (key_ms)
# and the projection init counter. The CONTROL arm (pre-projection-cache
# binary) runs the same rig unchanged — the Q4 bars compare the two.
#
# Usage: tools/uxspeed/mergekey-rig.sh <binary> [trials] [out-dir]
#   binary   the yggterm GUI binary to measure (hermetic: PATH-first +
#            YGGTERM_HOME/XDG_DATA_HOME scratch; the served executable_path
#            is exe-proofed per trial — [11.231] law).
#   trials   GUI launches for the main arm (each is one browser
#            replacement; default 3 = the Q4 minimum), plus one
#            search-active launch per trial (YGGTERM_SEARCH_QUERY).
#
# Output: <out-dir>/trial-<n>.json, trial-<n>-search.json, exe-proof lines
# on stdout. Analysis (Q4 bars) is mergekey_rig.py --analyze <out-dir>.
set -u
BIN=${1:?usage: mergekey-rig.sh <binary> [trials] [out-dir]}
TRIALS=${2:-3}
OUT=${3:-/tmp/s56-rig}
SESS=${S56_SESSIONS:-600}
DISP=${S56_DISPLAY:-:84}
WT=$(cd "$(dirname "$0")/../.." && pwd)
DRIVER="${S56_DRIVER:-$WT/tools/uxspeed/mergekey_rig.py}"

if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN"; exit 2; fi
mkdir -p "$OUT"
STAMP=$(date +%s)
SCRATCH=/tmp/s56-home-$STAMP
mkdir -p "$SCRATCH"

# ---- stage the durable-session world (real agent store; additive + removed)
STAGE_TAG="rig-s56-$STAMP"
STAGE_BASE="$HOME/.codex/sessions/2026/01/02/$STAGE_TAG"
mkdir -p "$STAGE_BASE"
python3 - "$STAGE_BASE" "$SESS" <<'PYEOF'
import json, os, sys, uuid
base, count = sys.argv[1], int(sys.argv[2])
cwds = [f"/home/pi/proj-{tag}" for tag in "abcdefgh"]
for ix in range(count):
    sid = str(uuid.uuid4())
    cwd = cwds[ix % len(cwds)]
    body = (
        json.dumps({"type": "session_meta", "payload": {"id": sid, "cwd": cwd}}) + "\n"
        + json.dumps({"type": "event_msg", "payload": {
            "type": "user_message", "message": f"s56 staged session {ix} on {cwd}"}}) + "\n"
    )
    with open(os.path.join(base, f"rollout-{sid}.jsonl"), "w") as fh:
        fh.write(body)
print(f"staged {count} sessions across {len(cwds)} cwds")
PYEOF

cleanup() {
  pkill -f "s56-home-$STAMP" 2>/dev/null
  pkill -f "Xvfb $DISP" 2>/dev/null
  rm -rf "$STAGE_BASE" 2>/dev/null
}
trap cleanup EXIT

# [s56-paintmatrix] S56_SMALL_WORLD=1 stages the session store INSIDE the
# scratch home and launches the GUI with HOME redirected there — the tree
# scan walks dirs::home_dir, so this is the only way to measure a SMALL
# hermetic world (the real store's 20k sessions otherwise dominate every
# trial; the Q4 paint matrix needs a world where DOM reads are verifiable).
SMALL_WORLD=${S56_SMALL_WORLD:-0}

for t in $(seq 1 "$TRIALS"); do
  HOME_DIR="$SCRATCH/t$t"
  mkdir -p "$HOME_DIR"
  if [ "$SMALL_WORLD" = "1" ]; then
    STAGE_BASE="$HOME_DIR/.codex/sessions/2026/01/02/$STAGE_TAG"
    mkdir -p "$STAGE_BASE"
    python3 - "$STAGE_BASE" "$SESS" <<'PYSTAGE'
import json, os, sys, uuid
base, count = sys.argv[1], int(sys.argv[2])
cwds = [f"/home/pi/proj-{tag}" for tag in "abcdefgh"]
for ix in range(count):
    sid = str(uuid.uuid4())
    cwd = cwds[ix % len(cwds)]
    body = (
        json.dumps({"type": "session_meta", "payload": {"id": sid, "cwd": cwd}}) + "\n"
        + json.dumps({"type": "event_msg", "payload": {
            "type": "user_message", "message": f"s56 staged session {ix} on {cwd}"}}) + "\n"
    )
    with open(os.path.join(base, f"rollout-{sid}.jsonl"), "w") as fh:
        fh.write(body)
print(f"staged {count} sessions (small-world) across {len(cwds)} cwds")
PYSTAGE
  fi
  pkill -f "Xvfb $DISP" 2>/dev/null; sleep 1
  Xvfb $DISP -screen 0 1600x1000x24 -ac > "$OUT/xvfb-t$t.log" 2>&1 &
  XPID=$!; sleep 1.5
  export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME="$HOME_DIR" \
         XDG_DATA_HOME="$HOME_DIR/xdg-share"
  HOME_REDIRECT=""
  if [ "$SMALL_WORLD" = "1" ]; then HOME_REDIRECT="HOME=$HOME_DIR"; fi
  dbus-run-session -- bash -c "
    export DISPLAY=$DISP GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 \
           RUST_BACKTRACE=1 YGGTERM_HOME=$HOME_DIR
    export $HOME_REDIRECT
    $BIN > $OUT/gui-t$t.log 2>&1
  " > /dev/null 2>&1 </dev/null &
  GUIWRAP=$!
  READY=0
  for i in $(seq 1 90); do
    sleep 1
    "$BIN" server app clients >/dev/null 2>&1 && READY=1 && break
  done
  echo "trial $t daemon ready=$READY after ${i}s"
  if [ "$READY" != 1 ]; then
    echo "{\"trial\": $t, \"fatal\": \"daemon never ready\"}" > "$OUT/trial-$t.json"
    kill "$GUIWRAP" 2>/dev/null; kill "$XPID" 2>/dev/null; continue
  fi
  sleep 6
  python3 "$DRIVER" --phase main --home "$HOME_DIR" --out "$OUT/trial-$t.json" \
    || echo "DRIVER FAILED trial $t"
  # ---- search-active arm: fresh instance, search armed from boot
  kill "$GUIWRAP" 2>/dev/null; sleep 2
  pkill -f "s56-home-$STAMP/t$t" 2>/dev/null; sleep 1
  dbus-run-session -- bash -c "
    export DISPLAY=$DISP GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 \
           YGGTERM_HOME=$HOME_DIR YGGTERM_SEARCH_QUERY=proj-a
    $BIN > $OUT/gui-t$t-search.log 2>&1
  " > /dev/null 2>&1 </dev/null &
  GUIWRAP=$!
  READY=0
  for i in $(seq 1 60); do
    sleep 1
    "$BIN" server app clients >/dev/null 2>&1 && READY=1 && break
  done
  echo "trial $t search arm ready=$READY after ${i}s"
  sleep 6
  if [ "$READY" = 1 ]; then
    python3 "$DRIVER" --phase search --home "$HOME_DIR" \
      --out "$OUT/trial-$t-search.json" || echo "SEARCH DRIVER FAILED trial $t"
  else
    echo "{\"trial\": $t, \"fatal\": \"search arm never ready\"}" > "$OUT/trial-$t-search.json"
  fi
  kill "$GUIWRAP" 2>/dev/null; sleep 1
  pkill -f "s56-home-$STAMP/t$t" 2>/dev/null
  kill "$XPID" 2>/dev/null
  sleep 1
  EXE=$(grep -m1 -o '"executable_path":"[^"]*"' "$HOME_DIR/event-trace.jsonl" 2>/dev/null | head -1 | cut -d'"' -f4)
  if [ "$EXE" = "$BIN" ]; then
    echo "trial $t EXE OK"
  else
    echo "trial $t EXE MISMATCH: served=$EXE expected=$BIN — THIS RUN MEASURED THE WRONG BINARY ([11.231])"
  fi
done

echo "rig complete; reports in $OUT; analyze with: python3 $DRIVER --analyze $OUT"
