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
SCRATCH=/tmp/f1i-home-$(date +%s)
mkdir -p "$SCRATCH"
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN — build first"; exit 2; fi
export BIN
pkill -f "Xvfb :79" 2>/dev/null; sleep 1
Xvfb :79 -screen 0 1600x1000x24 -ac > /tmp/f1i-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5
# ⛔ HERMETIC ([11.231]): XDG_DATA_HOME must point at the scratch or the
# binary re-execs the INSTALLED direct build; asserted via the trace's
# register event below (EXE PROOF).
export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
dbus-run-session -- bash -c "
  export DISPLAY=:79 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
  export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share YGGTERM_TEST_SUPPRESS_MOUNT_IPC=1 YGGTERM_HOT_PREMOUNT_CAP=2
  export YGGTERM_TEST_STALL_SNAPSHOT_FETCH_MS=6000
  exec $BIN > /tmp/f1i-gui.log 2>&1
" >/dev/null 2>&1 </dev/null &
GUI_WRAP=$!
READY=0; for i in $(seq 1 90); do sleep 1; "$BIN" server app clients >/dev/null 2>&1 && READY=1 && break; done
echo "daemon ready=$READY after ${i}s"; sleep 8
python3 - <<'PYEOF' > /tmp/f1i-run.log 2>&1
import json, subprocess, time, os

bin_path = os.environ["BIN"]
def verb(*args, timeout=90):
    return subprocess.run([bin_path] + list(args), capture_output=True, text=True, timeout=timeout)

TRACE = os.environ.get("YGGTERM_HOME", "/tmp/f1i-home-unknown") + "/event-trace.jsonl"

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
    print("probe-type %r -> accepted=%s reason=%s host_stdin_enabled=%s effective_input_focus=%s" % (
        data, verdict.get("accepted"), verdict.get("reason"),
        after.get("host_stdin_enabled"), after.get("effective_input_focus")))
    keep = ("session_path","host_id","accepted","reason","used_core_trigger","used_term_input",
            "keyboard_backend","mode","press_enter","per_char")
    print("  verdict: %s" % json.dumps({k: verdict.get(k) for k in keep if k in verdict}))
    at = after.get("visible_text") or ""
    print("  after.visible_text tail: %r" % at[-220:])
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

# --- setup: a FIRST and ALONE (control; must synthesize while ACTIVE ----
# before b exists — measured: creating b immediately demotes a mid-mount and
# its task drops with remount_armed:false, so a never synthesizes).
a = row_session(1)
if not a:
    print("FAIL: could not create row a"); raise SystemExit(2)
ok, _ = wait_for(lambda ev: session_events(ev, a, "synthesized_mount_open"), 40, "a synthesized")
if not ok:
    print("PRE-FLIGHT FAIL: control row a never synthesized"); raise SystemExit(4)
# baseline probe BEFORE any switch exists: a is active, freshly synthesized.
typed(a, "echo CTRL0")
ok, _ = wait_for(lambda ev: screen_count("CTRL0", a)[0] >= 1, 45, "CTRL0 painted")
c0_lines, _ = screen_count("CTRL0", a)
print("VERDICT BASELINE (never-switched active row real-input): %s (CTRL0 lines=%d)" % (
    "GREEN" if ok and c0_lines >= 1 else "RED", c0_lines))
if not (ok and c0_lines >= 1):
    raise SystemExit(4)
b = row_session(2)
print("rows: a=%s b=%s" % (a, b))
if not b:
    print("FAIL: could not create row b"); raise SystemExit(2)
ok, _ = wait_for(lambda ev: session_events(ev, b, "test_hook_mount_ipc_suppressed"), 30, "b suppression engaged")
if not ok:
    print("VERDICT HOOK: NOT ENGAGED — env gate never traced; rig invalid"); raise SystemExit(2)

# --- THE ARM: demote b INSIDE its armed->open window ----------------------
ok, armed_line = wait_for(lambda ev: (session_events(ev, b, "synthesized_mount_open_armed") or [(0,)])[0][0] + 1
                          if session_events(ev, b, "synthesized_mount_open_armed") else 0, 30, "b armed")
if not ok:
    print("VERDICT WINDOW: b never armed — stall hook or suppression missing"); raise SystemExit(3)
# guard: b must NOT have completed synthesis yet (the stall holds it open)
if session_events(read_events(), b, "synthesized_mount_open"):
    print("VERDICT WINDOW: b already completed before demotion — window missed"); raise SystemExit(3)
verb("server", "app", "open", a, "--view", "terminal"); time.sleep(1)
def demote_mark(ev):
    hits = [(ln, p) for ln, n, p in ev if n == "open_path_resolve" and p.get("session_path") == a]
    return hits[-1][0] + 1 if hits else 0
ok_dem, demote_line = wait_for(demote_mark, 10, "open-a resolve traced")
print("demote verb (open a) resolved at trace line %s (armed at %d)" % (demote_line, armed_line))

ok, open_line = wait_for(lambda ev: (session_events(ev, b, "synthesized_mount_open") or [(0,)])[0][0] + 1
                         if session_events(ev, b, "synthesized_mount_open") else 0, 25, "b synthesized while demoted")
if not ok:
    print("VERDICT WINDOW: b never completed synthesis after demotion"); raise SystemExit(3)
print("b synthesis completed at line %d while DEMOTED" % open_line)
time.sleep(3)

# --- BAR 2 first (control): a keeps input across the switch dance ---------
verb("server", "app", "open", a, "--view", "terminal"); time.sleep(4)
typed(a, "echo CTRL1")
ok, _ = wait_for(lambda ev: screen_count("CTRL1", a)[0] >= 1, 45, "CTRL1 painted")
ctrl_lines, _ = screen_count("CTRL1", a)
print("VERDICT CONTROL (synthesized-while-active keeps input): %s (CTRL1 lines=%d)" % (
    "GREEN" if ok and ctrl_lines >= 1 else "RED — RIG INVALID", ctrl_lines))
if not (ok and ctrl_lines >= 1):
    raise SystemExit(5)

# --- BAR 1: re-select b and type — THE RED --------------------------------
verb("server", "app", "open", b, "--view", "terminal"); time.sleep(5)
reselect_line = len(open(TRACE).readlines())
v1 = typed(b, "echo DEMOTE1")
ok, _ = wait_for(lambda ev: screen_count("DEMOTE1", b)[0] >= 1, 45, "DEMOTE1 painted")
d1_lines, tail = screen_count("DEMOTE1", b)
bar1_green = ok and d1_lines >= 1
print("VERDICT BAR1 (demoted-mid-synthesis row re-latches input): %s (DEMOTE1 lines=%d)" % (
    "GREEN" if bar1_green else "RED — REFUSAL REPRODUCED", d1_lines))

# policy trace for b across the reselect: did Rust re-send an enable?
pol = [(ln, p) for ln, n, p in read_events()
       if n == "applied" and "allow_input" in p and p.get("session_path") == b]
pre = [(ln, p) for ln, p in pol if ln < reselect_line]
post = [(ln, p) for ln, p in pol if ln >= reselect_line]
print("input_policy.applied for b: %d pre-reselect, %d post-reselect" % (len(pre), len(post)))
for ln, p in post[-6:]:
    print("   post line %d: allow=%s focus=%s window_focused=%s" % (
        ln, p.get("allow_input"), p.get("focus_input"), p.get("window_focused")))
enq = [(ln, p) for ln, p in session_events(read_events(), b, "synthesized_input_chunk_enqueued")
       if ln >= reselect_line]
print("post-reselect chunk enqueues for b: %d" % len(enq))

# --- BAR 3 (severity): does the pointerdown ladder re-latch? -------------
verb("server", "app", "terminal", "probe-select", b)
time.sleep(2)
v2 = typed(b, "echo DEMOTE2")
ok2, _ = wait_for(lambda ev: screen_count("DEMOTE2", b)[0] >= 1, 45, "DEMOTE2 painted")
d2_lines, _ = screen_count("DEMOTE2", b)
print("VERDICT BAR3 (pointerdown ladder heals): %s (DEMOTE2 lines=%d)" % (
    "YES" if ok2 and d2_lines >= 1 else "NO — real clicks stay dead too", d2_lines))

if bar1_green:
    print("RIG: BAR1 GREEN — the refusal does not reproduce on this build")
    raise SystemExit(0)
print("RIG: RED state confirmed (BAR1 refusal live; control GREEN; see latch evidence above)")
raise SystemExit(6)
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
tail -50 /tmp/f1i-run.log
echo "scratch=$SCRATCH rc=$RC"
exit $RC
