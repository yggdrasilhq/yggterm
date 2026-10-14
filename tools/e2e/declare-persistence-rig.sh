#!/usr/bin/env bash
# declare-persistence-rig.sh — THE [11.80] LEDGER DURABILITY NATURAL-FIRE (sitting 43).
#
# Drives the dev-local GUI harness (tools/e2e/gui-harness.sh) on a GIVEN
# binary pair and proves, at the effect level:
#   BAR 1  a real OSC 7717 sidebar declare typed into a local row is served
#          by the single ask (declare_count >= 1);
#   BAR 2  the record reaches server-state.json within the deferred-flush
#          window (< ~8s) — the s43 cadence fix (defer_persist at record
#          sites + the flusher mute re-arm);
#   BAR 3  a daemon SIGKILL + fresh respawn serves the SAME record —
#          persistence across a hard kill, not just a graceful restart;
#   BAR 4  the client restore batch does NOT declare the row absent — no
#          daemon_declare_absent event for the row in the post-respawn
#          harness trace (the s43 batch-inclusion + silence law).
# Usage: declare-persistence-rig.sh <gui-binary> <headless-binary>
# rc 0 GREEN; 10-40 name the failing bar. Ephemeral scratch under /tmp.
set -u
GUI=${1:?gui binary}
HEADLESS=${2:?headless binary}
HARNESS=$(dirname "$(readlink -f "$0")")/gui-harness.sh
SCRATCH=${YGG_GUIHARNESS_HOME:-/tmp/ygg-guiharness-home}
STATE=$SCRATCH/server-state.json

die() { echo "RED bar-$1: $2"; exit "$1"; }
export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
v() { timeout 30 "$HEADLESS" "$@" 2>&1; }

echo "== staging lane binaries + booting the harness on them =="
STAGE=/tmp/s43-nf-bin; rm -rf "$STAGE"; mkdir -p "$STAGE"
cp "$GUI" "$STAGE/yggterm"; cp "$HEADLESS" "$STAGE/yggterm-headless"
GUI_MD5=$(md5sum "$STAGE/yggterm" | cut -d" " -f1)
HL_MD5=$(md5sum "$STAGE/yggterm-headless" | cut -d" " -f1)
YGG_GUIHARNESS_BIN="$STAGE/yggterm" bash "$HARNESS" stop >/dev/null 2>&1
YGG_GUIHARNESS_BIN="$STAGE/yggterm" bash "$HARNESS" start | tail -2
# Wait for app-control registration (a fresh lane build's webview init on
# Xvfb takes longer than any fixed sleep — measured 2026-10-15).
READY=0
for i in $(seq 1 30); do
  sleep 2
  if v server app clients >/dev/null 2>&1; then READY=1; break; fi
done
[ "$READY" = 1 ] || die 3 "GUI never registered for app control (60s)"
sleep 2
DPID=$(pgrep -u "$USER" -f "server daemon" | while read -r p; do
  tr '\0' '\n' < "/proc/$p/environ" 2>/dev/null | grep -qx "YGGTERM_HOME=$SCRATCH" && echo "$p" && break
done)
[ -n "${DPID:-}" ] || die 3 "no harness daemon found after boot"
DAEMON_EXE=$(readlink -f "/proc/$DPID/exe")
[ "$(md5sum "$DAEMON_EXE" | cut -d" " -f1)" = "$HL_MD5" ] || die 3 "daemon exe $DAEMON_EXE is not the lane build ([11.231] class)"

echo "== BAR 1: declare + single ask =="
NEW=$(v server app terminal new --kind shell --title s43nf)
ROW=$(echo "$NEW" | grep -o '"session_path": *"[^"]*"' | head -1 | sed 's/.*"session_path": *"//; s/"$//')
[ -n "$ROW" ] || die 1 "no row created (raw: $(echo "$NEW" | head -c 200))"
v server app open "$ROW" --view terminal >/dev/null; sleep 2
B64=$(python3 -c "import base64,json; print(base64.b64encode(json.dumps({'panes':[{'label':'s43nf','kind':'note'}]}).encode()).decode())")
v server app terminal probe-type "$ROW" --data "printf '\\033]7717;sidebar;declare;${B64}\\007'" --enter --mode xterm >/dev/null
sleep 2
ANS=$(v server terminal app-declares "$ROW")
echo "$ANS" | grep -q '"declare_count": 1' || die 1 "single ask did not serve the live declare: $ANS"
echo "bar1 ok: $ROW"

echo "== BAR 2: the record reaches the state doc within the flush window =="
OK=0
for i in $(seq 1 10); do
  sleep 1
  if python3 -c "
import json,sys
d=json.load(open('$STATE'))
recs=d.get('last_known_app_declares',{}).get('$ROW',[])
sys.exit(0 if recs else 1)" 2>/dev/null; then OK=1; break; fi
done
[ "$OK" = 1 ] || die 2 "record never reached server-state.json (cadence fix dead)"
echo "bar2 ok after ${i}s"

echo "== BAR 3: SIGKILL the daemon; the respawn serves the record =="
TRACE_LINES_BEFORE_KILL=$(wc -l < "$SCRATCH/event-trace.jsonl" 2>/dev/null || echo 0)
kill -9 "$DPID"
sleep 3
DPID2=""
for i in $(seq 1 20); do
  sleep 2
  DPID2=$(pgrep -u "$USER" -f "server daemon" | while read -r p; do
    tr '\0' '\n' < "/proc/$p/environ" 2>/dev/null | grep -qx "YGGTERM_HOME=$SCRATCH" && echo "$p" && break
  done)
  [ -n "$DPID2" ] && break
done
[ -n "$DPID2" ] || die 3 "daemon did not respawn"
sleep 4
ANS2=$(v server terminal app-declares "$ROW")
echo "$ANS2" | grep -q '"declare_count": 1' || die 3 "fresh incarnation lost the record: $ANS2"
echo "bar3 ok (respawn pid $DPID2)"

echo "== BAR 4: the client batch must not declare the row absent =="
sleep 12   # let the GUI restore poll tick at least once
ABSENT=$(tail -n +"$((TRACE_LINES_BEFORE_KILL + 1))" "$SCRATCH/event-trace.jsonl" 2>/dev/null | grep "daemon_declare_absent" | grep -c "$ROW" || true)
[ "$ABSENT" = 0 ] || die 4 "the batch declared the row absent $ABSENT times AFTER the kill (pre-kill line offset $TRACE_LINES_BEFORE_KILL)"
echo "bar4 ok: zero daemon_declare_absent for the row"

echo "GREEN — all four bars (gui $GUI_MD5, headless $HL_MD5)"
