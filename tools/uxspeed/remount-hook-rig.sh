#!/bin/bash
# [F1-(h)] THE SYNTHESIZED-REMOUNT HOOK RIG — the deterministic falsifier
# sol Q-C specified (node lores/chain-of-thought/
# 2026-10-05-yggterm-f1-ring-rework-sol-review.md). Running the GUI with
# YGGTERM_TEST_SUPPRESS_MOUNT_IPC=1 kills the page→Rust bridge channel for
# EVERY mount (the exact production shed condition; eval returns stay
# alive), so the synthesis path fires deterministically on remounts too —
# the wall sitting 5's replay rig could not cross.
#   BAR 1: focus away+back forces a NEW synthesized mount for the row
#          (test_hook_mount_ipc_suppressed + synthesized_mount_open at a
#          higher mount_epoch for the same session_path).
#   BAR 2: ZERO old enqueues — `echo HOOKOLD` typed BEFORE the remount
#          appears as a command line EXACTLY once after it (no replay).
#   BAR 3: exactly one new — `echo HOOKNEW` typed AFTER the remount paints
#          exactly once.
#   RACE arm (delayed-drain-across-remount, best-effort): type HOOKRACE
#          then switch immediately; readout is the
#          synthesized_input_stale_answer trace (or zero duplication) —
#          reported as measured, never laundered.
# REMOUNT TRIGGER (measured sitting 6): focus away+back does NOT remount a
# healthy mount — the retention policy retains the host
# (bootstrap_spawn_skipped_inactive_retained_host), and cap-eviction via
# YGGTERM_HOT_PREMOUNT_CAP=2 does not drop it either. The deterministic
# trigger is `server terminal restart` — the deploy/daemon-swap remount
# class. The retained-switch stays as the CONTROL arm (expects NO new
# mount). Input focus latches at synthesis time for the ACTIVE row (the
# run-3 finding), so b is created last and stays active until the control
# switch returns to it.
# Usage: tools/uxspeed/remount-hook-rig.sh [worktree] [binary]
#   binary defaults to $worktree/target/debug/yggterm (cargo build -p
#   yggterm --bin yggterm). Runs on dev's Xvfb :78 — jojo untouched.
set -u
WT=${1:-$HOME/gh/yggterm}
BIN=${2:-$WT/target/debug/yggterm}
DISP=:78
SCRATCH=/tmp/f1h-home-$(date +%s)
mkdir -p "$SCRATCH"
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN — build first"; exit 2; fi
export BIN
pgrep -f "Xvfb $DISP" >/dev/null && pkill -f "Xvfb $DISP"; sleep 1
Xvfb $DISP -screen 0 1600x1000x24 -ac > /tmp/f1h-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5
# ⛔ HERMETIC ([11.231]): XDG_DATA_HOME must point at the scratch or the
# worktree binary re-execs the INSTALLED direct build and the rig measures
# the wrong code; the exe is asserted from the trace register event below.
export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
dbus-run-session -- bash -c "
  export DISPLAY=$DISP GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
  export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share YGGTERM_TEST_SUPPRESS_MOUNT_IPC=1 YGGTERM_HOT_PREMOUNT_CAP=2
  exec $BIN > /tmp/f1h-gui.log 2>&1
" >/dev/null 2>&1 </dev/null &
GUI_WRAP=$!
READY=0; for i in $(seq 1 90); do sleep 1; "$BIN" server app clients >/dev/null 2>&1 && READY=1 && break; done
echo "daemon ready=$READY after ${i}s"; sleep 10
python3 - <<'PYEOF' > /tmp/f1h-run.log 2>&1
import json, subprocess, time, os

bin_path = os.environ["BIN"]

def verb(*args, timeout=60):
    return subprocess.run([bin_path] + list(args), capture_output=True, text=True, timeout=timeout)

TRACE = os.environ.get("YGGTERM_HOME", "/tmp/f1h-home-unknown") + "/event-trace.jsonl"

def events(name, session=None):
    out = []
    try:
        for line in open(TRACE):
            try:
                event = json.loads(line)
            except Exception:
                continue
            payload = event.get("payload") or {}
            if payload.get("event") != name and payload.get("name") != name:
                # trace rows may nest differently; check the common shapes
                if event.get("event") != name and event.get("name") != name:
                    continue
            if session and payload.get("session_path") != session and event.get("payload", {}).get("session_path") != session:
                continue
            body = payload or event
            out.append(body)
    except FileNotFoundError:
        pass
    return out

def row_session(n):
    out = verb("server", "app", "terminal", "new", "--kind", "shell", "--title", "hook%d" % n)
    try:
        return (json.loads(out.stdout).get("data") or {}).get("session_path")
    except Exception:
        print("new row raw:", out.stdout[:200], out.stderr[:200])
        return None

# b is created LAST: input focus latches for the row active at synthesis
# time, and under the hook a later focus switch cannot re-latch it (the
# ack rides the suppressed bridge — measured run 3).
a = row_session(1)
b = row_session(2)
print("rows: a=%s b=%s" % (a, b))
if not (a and b):
    print("FAIL: could not create rows"); raise SystemExit
time.sleep(20)

hook_events = events("test_hook_mount_ipc_suppressed")
synths = events("synthesized_mount_open")
print("hook_events=%d synthesized_mount_open=%d" % (len(hook_events), len(synths)))
if not hook_events:
    print("VERDICT HOOK: NOT ENGAGED — env gate never traced; rig invalid"); raise SystemExit

def screen_count(marker, session):
    body = (verb("server", "screen", session).stdout or "")
    return body.count("echo " + marker), body[-200:]

def typed(session, data):
    out = verb("server", "app", "terminal", "probe-type", session, "--data", data, "--enter", "--mode", "xterm")
    try:
        verdict = json.loads(out.stdout).get("data") or {}
        print("probe-type %r -> %s" % (data, json.dumps({k: verdict.get(k) for k in ("accepted", "reason")})))
    except Exception:
        print("probe-type %r raw: %s %s" % (data, out.stdout[:160], out.stderr[:160]))

# --- seed OLD input on b; it MUST paint before the rig may proceed ---
typed(b, "echo HOOKOLD")
time.sleep(5)
pre_lines, _ = screen_count("HOOKOLD", b)
print("HOOKOLD pre-remount command_lines=%d" % pre_lines)
if pre_lines < 1:
    print("PRE-FLIGHT FAIL: seed did not paint — b never synthesized; dump:")
    for e in events("warm_eval_mount_alive_events_shed", b)[-3:]:
        print("  shed:", json.dumps(e)[:200])
    raise SystemExit

# --- control arm: retained switch (away+back) — expects NO new mount ---
verb("server", "app", "terminal", "focus", a); time.sleep(3)
verb("server", "app", "terminal", "focus", b); time.sleep(6)
epochs_control = sorted({e.get("mount_epoch") for e in events("test_hook_mount_ipc_suppressed", b)})
ctrl_lines, _ = screen_count("HOOKOLD", b)
print("CONTROL retained-switch epochs=%s marker_lines=%d (expect unchanged+1)" % (epochs_control, ctrl_lines))

# --- THE REMOUNT: `server terminal restart` — the deterministic trigger
# (focus-eviction does NOT remount: the retention policy keeps the host —
# bootstrap_spawn_skipped_inactive_retained_host, measured run 4). The
# restart re-runs the mount; the hook forces it to synthesize.
verb("server", "terminal", "restart", b)
time.sleep(15)  # fresh bootstrap + synthesis + paint

epochs_after = sorted({e.get("mount_epoch") for e in events("test_hook_mount_ipc_suppressed", b)})
print("epochs for row b: control=%s after_restart=%s" % (epochs_control, epochs_after))
forced = bool(epochs_after) and max(epochs_after) > max(epochs_control)
print("VERDICT REMOUNT-SYNTHESIS: %s" % ("FORCED (new synthesized mount at higher epoch)" if forced else "NOT FORCED — report honestly"))

post_lines, post_tail = screen_count("HOOKOLD", b)
print("HOOKOLD post-restart command_lines=%d (control was %d; growth=replay, same/fewer=reseed-or-fresh)" % (post_lines, ctrl_lines))
print("VERDICT REPLAY: %s" % ("NONE (zero old enqueues)" if post_lines <= ctrl_lines else "REPRODUCED (command re-executed: lines %d -> %d)" % (ctrl_lines, post_lines)))

# --- exactly one NEW input after the remount ---
typed(b, "echo HOOKNEW")
time.sleep(5)
new_lines, _ = screen_count("HOOKNEW", b)
print("VERDICT NEW-INPUT: %s (command_lines=%d)" % ("EXACTLY-ONCE" if new_lines == 1 else "WRONG", new_lines))

# --- delayed-drain race arm: type then restart IMMEDIATELY (a drain eval
# straddling the remount must be discarded by the incarnation echo) ---
verb("server", "app", "terminal", "probe-type", b, "--data", "echo HOOKRACE", "--enter", "--mode", "xterm")
verb("server", "terminal", "restart", b)
time.sleep(10)
race_lines, _ = screen_count("HOOKRACE", b)
stale = events("synthesized_input_stale_answer")
print("VERDICT RACE: HOOKRACE command_lines=%d stale_answers=%d (delayed-drain discard %s)" % (
    race_lines, len(stale), "OBSERVED" if stale else "not-observed (window may not have straddled)"))

drains = events("synthesized_input_drained", b)
print("drain events for row b: %d; payloads:" % len(drains))
for d in drains[:8]:
    print("  ", json.dumps({k: d.get(k) for k in ("applied", "skipped", "inc", "pruned", "watermark", "mount_epoch") if k in d}))
PYEOF
RC=$?
kill "$GUI_WRAP" 2>/dev/null; sleep 1
# ⛔ NOT pkill -f "target/debug/yggterm": the rig's own cmdline carries the
# binary path and -f self-matches (the gotcha-7 law) — it killed this very
# rig once. -x matches the comm name only.
pkill -x yggterm 2>/dev/null
pkill -f "dbus-run-session" 2>/dev/null; sleep 1; kill "$XVFB_PID" 2>/dev/null
echo "=== EXE PROOF ==="
EXE=$(grep -m1 -o '"executable_path":"[^"]*"' "$SCRATCH/event-trace.jsonl" 2>/dev/null | head -1 | cut -d'"' -f4)
echo "served executable_path=$EXE"
if [ "$EXE" = "$BIN" ]; then echo "EXE OK: worktree binary served this run"; else echo "EXE MISMATCH: verdicts above are VOID"; RC=3; fi
echo "=== DRIVER OUTPUT ==="
tail -30 /tmp/f1h-run.log
echo "scratch=$SCRATCH rc=$RC"
exit $RC
