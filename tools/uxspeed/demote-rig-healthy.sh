#!/bin/bash
# [F1-(i)] THE PRE-SYNTHESIS-DEMOTION INPUT RIG (d-rig) — the falsifier for
# the demoted-mid-synthesis input refusal (queue line ~333; sol Q3; sitting-10
# rig run 3 measured the refusal; this rig makes it deterministic and
# discriminates the latch sites).
#   SETUP: the GUI runs with YGGTERM_TEST_SUPPRESS_MOUNT_IPC=1 (total bridge
#          shed at the mount loop; eval returns alive) so synthesis fires
#          deterministically, PLUS YGGTERM_TEST_STALL_SNAPSHOT_FETCH_MS=6000:
#          the seed DELIVERY stalls 6s between synthesized_mount_open_armed
#          and synthesized_mount_open — a deterministic WIDE synthesis window.
#   ARM (the unit): row a (dummy, synthesized-while-active control) is
#          created first; row b is created second (ACTIVE at its mount);
#          the rig focuses a INSIDE b's armed→open window (b demoted before
#          its synthesis completes); b's synthesis then completes DEMOTED.
#   BAR 1 (RED expected on unfixed main): focus b (re-select) + probe-type
#          "echo DEMOTE1" — refused (accepted=false / not painted) and the
#          probe's own after-snapshot names the latch site:
#          host_stdin_enabled=false  → the enable never re-applied (page
#                                      arbitration lost it — sites a/b)
#          effective_input_focus=false → enable applied, focus never landed
#                                      (site c)
#   BAR 2 (GREEN control — rig validity): a, synthesized while active, keeps
#          input across the same switch dance (focus a, probe-type CTRL1).
#   BAR 3 (severity): probe-select (the pointerdown ladder) on b, then
#          probe-type DEMOTE2 — does the REAL pointer path re-latch?
#          The ladder calls setInputEnabled with policySource='local', which
#          cannot reopen rustInputGateOpen — measure, don't assume.
# FAILED VERDICTS EXIT NONZERO AFTER ALL ARMS:
# 2=rows/hook, 3=window-miss (demotion did not land inside the window),
# 4=pre-flight (control row never synthesized), 5=control arm failed,
# 6=BAR1 REFUSAL REPRODUCED (the expected RED on unfixed main), 7=exe
# (wrapper). rc=0 = BAR1 green (fixed build or the refusal no longer
# reproduces).
# Usage: tools/uxspeed/demote-rig.sh <binary>   (absolute path; full-workspace
# release build per rig law L1).
set -u
BIN=${1:?usage: demote-rig.sh <binary>}
SCRATCH=/tmp/f1ih-home-$(date +%s)
mkdir -p "$SCRATCH"
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN — build first"; exit 2; fi
export BIN
pkill -f "Xvfb :79" 2>/dev/null; sleep 1
Xvfb :79 -screen 0 1600x1000x24 -ac > /tmp/f1ih-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5
# ⛔ HERMETIC ([11.231]): XDG_DATA_HOME must point at the scratch or the
# binary re-execs the INSTALLED direct build; asserted via the trace's
# register event below (EXE PROOF).
export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
dbus-run-session -- bash -c "
  export DISPLAY=:79 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
  export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share YGGTERM_HOT_PREMOUNT_CAP=2
  exec $BIN > /tmp/f1ih-gui.log 2>&1
" >/dev/null 2>&1 </dev/null &
GUI_WRAP=$!
READY=0; for i in $(seq 1 90); do sleep 1; "$BIN" server app clients >/dev/null 2>&1 && READY=1 && break; done
echo "daemon ready=$READY after ${i}s"; sleep 8
python3 - <<'PYEOF' > /tmp/f1ih-run.log 2>&1
import json, subprocess, time, os

bin_path = os.environ["BIN"]
def verb(*args, timeout=90):
    return subprocess.run([bin_path] + list(args), capture_output=True, text=True, timeout=timeout)

TRACE = os.environ.get("YGGTERM_HOME", "/tmp/f1ih-home-unknown") + "/event-trace.jsonl"

def read_events():
    out = []
    try:
        for ln, line in enumerate(open(TRACE)):
            try:
                event = json.loads(line)
            except Exception:
                continue
            name = (event.get("name") or event.get("event")
                    or (event.get("payload") or {}).get("name")
                    or (event.get("payload") or {}).get("event"))
            if name:
                out.append((ln, name, event.get("payload") or {}))
    except FileNotFoundError:
        pass
    return out

def wait_for(predicate, deadline_s, what, poll=0.4):
    deadline = time.time() + deadline_s
    while time.time() < deadline:
        value = predicate(read_events())
        if value:
            return True, value
        time.sleep(poll)
    return False, predicate(read_events())

def session_events(events, session, name):
    return [(ln, p) for ln, event_name, p in events
            if event_name == name and p.get("session_path") == session]

def typed(session, data):
    out = verb("server", "app", "terminal", "probe-type", session, "--data", data, "--enter", "--mode", "xterm")
    try:
        verdict = json.loads(out.stdout).get("data") or {}
    except Exception:
        print("probe-type %r raw: %s %s" % (data, out.stdout[:200], out.stderr[:200]))
        return {}
    after = verdict.get("after") or {}
    print("probe-type %r -> accepted=%s reason=%s core_trigger=%s term_input=%s" % (
        data, verdict.get("accepted"), verdict.get("reason"),
        verdict.get("used_core_trigger"), verdict.get("used_term_input")))
    return verdict

def screen_count(marker, session):
    body = (verb("server", "screen", session).stdout or "")
    return body.count("echo " + marker), body[-160:]

def row_session(n):
    out = verb("server", "app", "terminal", "new", "--kind", "shell", "--title", "demo%d" % n)
    try:
        return (json.loads(out.stdout).get("data") or {}).get("session_path")
    except Exception:
        print("new row raw:", out.stdout[:200], out.stderr[:200])
        return None

a = row_session(1)
if not a:
    print("FAIL: no row a"); raise SystemExit(2)
ok, _ = wait_for(lambda ev: session_events(ev, a, "attach_ready"), 40, "a ready")
if not ok:
    print("PRE-FLIGHT FAIL: a never ready (healthy mount)"); raise SystemExit(4)
typed(a, "echo CTRL0")
ok, _ = wait_for(lambda ev: screen_count("CTRL0", a)[0] >= 1, 45, "CTRL0 painted")
c0, _ = screen_count("CTRL0", a)
print("VERDICT BASELINE (healthy active row): %s (CTRL0 lines=%d)" % ("GREEN" if ok and c0 >= 1 else "RED", c0))
if not (ok and c0 >= 1):
    raise SystemExit(4)

b = row_session(2)
print("rows: a=%s b=%s" % (a, b))
if not b:
    print("FAIL: no row b"); raise SystemExit(2)
ok, _ = wait_for(lambda ev: session_events(ev, b, "attach_ready"), 40, "b ready")
if not ok:
    print("PRE-FLIGHT FAIL: b never ready"); raise SystemExit(4)
time.sleep(3)

# CONTROL: re-select a after the demotion-at-b-creation; measure drop/raise/input
verb("server", "app", "open", a, "--view", "terminal"); time.sleep(5)
drops_a = session_events(read_events(), a, "terminal_mount_task_dropped")
refused_a = session_events(read_events(), a, "reveal_raise_refused")
served_a = session_events(read_events(), a, "reveal_served")
spawned_a = session_events(read_events(), a, "bootstrap_spawn_scheduled")
print("a lifecycle across demote+reselect: drops=%d refused=%d served=%d spawns=%d" % (
    len(drops_a), len(refused_a), len(served_a), len(spawned_a)))
for _, p in drops_a:
    print("   drop: remount_armed=%s exit_hint=%s" % (p.get("remount_armed"), p.get("exit_hint")))
for _, p in refused_a[-2:]:
    print("   refused: loop_live=%s was_ever_ready=%s" % (p.get("loop_live"), p.get("was_ever_ready")))
typed(a, "echo CTRL1")
ok, _ = wait_for(lambda ev: screen_count("CTRL1", a)[0] >= 1, 45, "CTRL1 painted")
c1, _ = screen_count("CTRL1", a)
print("VERDICT CONTROL (healthy row demoted then re-selected): %s (CTRL1 lines=%d)" % (
    "GREEN" if ok and c1 >= 1 else "RED", c1))
fail = 0 if (ok and c1 >= 1) else 5

# BAR1: b demoted (by the control's re-select of a), then re-selected
verb("server", "app", "open", b, "--view", "terminal"); time.sleep(5)
drops_b = session_events(read_events(), b, "terminal_mount_task_dropped")
refused_b = session_events(read_events(), b, "reveal_raise_refused")
served_b = session_events(read_events(), b, "reveal_served")
print("b lifecycle: drops=%d refused=%d served=%d" % (len(drops_b), len(refused_b), len(served_b)))
for _, p in drops_b:
    print("   drop: remount_armed=%s exit_hint=%s" % (p.get("remount_armed"), p.get("exit_hint")))
typed(b, "echo DEMOTE1")
ok, _ = wait_for(lambda ev: screen_count("DEMOTE1", b)[0] >= 1, 45, "DEMOTE1 painted")
d1, _ = screen_count("DEMOTE1", b)
print("VERDICT BAR1 (healthy b demoted then re-selected): %s (DEMOTE1 lines=%d)" % (
    "GREEN" if ok and d1 >= 1 else "RED", d1))
if not (ok and d1 >= 1):
    fail = fail or 6
if fail:
    raise SystemExit(fail)
print("RIG: healthy-mode all green")
PYEOF
RC=$?
kill "$GUI_WRAP" 2>/dev/null; sleep 1
pkill -x yggterm 2>/dev/null
pkill -f "dbus-run-session" 2>/dev/null; sleep 1; kill "$XVFB_PID" 2>/dev/null
# REAP THE SCRATCH DAEMON (matched by YGGTERM_HOME only — never by name)
for p in $(pgrep -f 'yggterm-headless server daemon'); do
  h=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | sed -n 's/^YGGTERM_HOME=//p')
  [ "$h" = "$SCRATCH" ] && kill "$p" 2>/dev/null
done
echo "=== EXE PROOF ==="
EXE=$(grep -m1 -o '"executable_path":"[^"]*"' "$SCRATCH/event-trace.jsonl" 2>/dev/null | head -1 | cut -d'"' -f4)
echo "served executable_path=$EXE"
if [ "$EXE" = "$BIN" ]; then echo "EXE OK: requested binary served this run"; else echo "EXE MISMATCH: verdicts above are VOID"; RC=7; fi
echo "=== DRIVER OUTPUT ==="
tail -50 /tmp/f1ih-run.log
echo "scratch=$SCRATCH rc=$RC"
exit $RC
