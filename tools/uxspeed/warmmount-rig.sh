#!/bin/bash
# [11.178→F1] THE WARM-MOUNT WEDGE RIG — the campaign's deterministic
# paint-zombie factory (5/6 fresh spawns wedge on current main, measured
# 2026-10-05). Runs the GUI on dev's own Xvfb :77 — jojo untouched.
# Usage: tools/uxspeed/warmmount-rig.sh [worktree-path] [iters]
#   worktree defaults to ~/gh/yggterm (needs target/debug/yggterm built:
#   cargo build -p yggterm --bin yggterm).
# Falsifier for the F1 fix (synthesize-full-contract): 0/6 wedges AND 6/6
# painted WITH content (spawn_to_paint with nonblank>0 per iteration).
set -u
WT=${1:-$HOME/gh/yggterm}
ITERS=${2:-6}
BIN=$WT/target/debug/yggterm
DISP=:77
SCRATCH=/tmp/warmmount-home-$(date +%s)
mkdir -p "$SCRATCH"
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN — cargo build -p yggterm --bin yggterm first"; exit 2; fi
pgrep -f "Xvfb $DISP" >/dev/null && pkill -f "Xvfb $DISP"; sleep 1
Xvfb $DISP -screen 0 1600x1000x24 -ac > /tmp/warmmount-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5
# ⛔ HERMETIC: without this the worktree binary hands off to the INSTALLED
# direct build (~/.local/share/yggterm/direct/builds/<sha>) and the rig
# measures the wrong binary — it reported two falsifier verdicts about
# unpatched main before this was found (2026-10-05).
export PATH=$WT/target/debug:$PATH YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
dbus-run-session -- bash -c "
  export DISPLAY=$DISP GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1 YGGTERM_HOME=$SCRATCH
  $BIN > /tmp/warmmount-gui.log 2>&1
" >/tmp/warmmount-wrap.log 2>&1 </dev/null &
GUI_WRAP=$!
READY=0; for i in $(seq 1 90); do sleep 1; $BIN server app clients >/dev/null 2>&1 && READY=1 && break; done
echo "daemon ready=$READY after ${i}s"; sleep 10
python3 $WT/tools/uxspeed/uxprobe.py --actions spawn --iters "$ITERS" --out /tmp/warmmount-report.json > /tmp/warmmount-probe.log 2>&1
echo "probe rc=$?"
kill "$GUI_WRAP" 2>/dev/null; sleep 1
pkill -f "target/debug/yggterm" 2>/dev/null; sleep 1; kill "$XVFB_PID" 2>/dev/null
echo "=== EXE PROOF ==="
EXE=$(grep -m1 -o '"executable_path":"[^"]*"' "$SCRATCH/event-trace.jsonl" | head -1 | cut -d'"' -f4)
echo "served executable_path=$EXE"
if [ "$EXE" = "$BIN" ]; then echo "EXE OK: worktree binary served this run"; else echo "EXE MISMATCH: this run did NOT test $BIN"; fi
echo "=== F1 READOUTS (scratch=$SCRATCH) ==="
for f in $(find "$SCRATCH" -name ytrace.jsonl); do
  jsr=$(grep -o '"js_ready"' "$f" | wc -l); mo=$(grep -o '"mount_open"' "$f" | wc -l)
  echo "js_ready=$jsr mount_open=$mo wedges=$((jsr - mo))"
done
tail -6 /tmp/warmmount-probe.log
echo done
