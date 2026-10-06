#!/bin/bash
# [F1-(e)] THE OUTPUT-FENCE RIG — the ordering falsifier (sol Q1+Q2, node
# lores/chain-of-thought/2026-10-05-yggterm-f1-sol-patch-review.md; plan
# ACK-0b19cf9fe2). The GUI runs under YGGTERM_TEST_SUPPRESS_MOUNT_IPC=1
# (synthesis fires deterministically on fresh spawns) PLUS the (e) hook
# YGGTERM_TEST_STALL_SNAPSHOT_FETCH_MS=<stall>: the seed snapshot is
# CAPTURED at T0 but DELIVERED <stall> ms later — the deterministic wide
# form of the production race (fetch outstanding while differentials flow
# through the daemon read-poll arm to the page).
#   BAR 1 (the falsifier): a full-frame repaint differential fired at
#          armed+50ms — printf '\033[2J\033[HF1EMARK\033[H' — ENDS WITH THE
#          CURSOR PARKED AT THE HOME CELL: buffer non-empty but baseY and
#          cursorX/Y all 0, so the painted-guard reads FALSE and (unfenced)
#          the stale T0 seed APPENDS over row 1, ERASING the marker. The
#          PAGE-side screen (read-buffer --mode screen, NOT the daemon
#          screen — the daemon always composes) must show F1EMARK exactly
#          once. Sitting-8's marker-x2 was daemon-side; this is the
#          page-side bar.
#   BAR 2 (fence evidence): seed_mode=fenced_repaint with wrote_seed>0 and
#          synth_output_fence_flushed batches>=1 — the repaint RODE the
#          fence (absent on unfenced builds; skipped when BAR 1 already
#          failed, i.e. in RED runs).
#   BAR 3 (healthy control): a second boot WITHOUT suppression mounts and
#          paints a probe normally and NEVER traces a synthesis/fence event
#          — the fence is synthesis-only, the healthy bridge row unchanged.
# FAILED VERDICTS EXIT NONZERO AFTER ALL ARMS: 2=rows/hook, 4=pre-flight,
# 5=marker lost (THE RED), 6=not exactly-once / wrong mode, 7=healthy
# control, 8=exe mismatch (wrapper).
# Usage: tools/uxspeed/fence-rig.sh [worktree] [binary] [stall_ms]
#   ⛔ BUILD LAW (measured sitting 7): FULL-WORKSPACE release build — a
#   -p yggterm build changes feature unification and shifts timing.
#   Runs on dev's Xvfb :78 — jojo untouched.
set -u
WT=${1:-$HOME/gh/yggterm}
BIN=${2:-$WT/target/release/yggterm}
STALL=${3:-500}
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN — build first"; exit 2; fi
export BIN STALL

run_boot() {  # $1=scratch $2=synthesized-envs(1|0) -> boots GUI, echoes wrap pid
  local SCRATCH=$1 SYNTH=$2
  mkdir -p "$SCRATCH"
  export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
  if [ "$SYNTH" = 1 ]; then
    dbus-run-session -- bash -c "
      export DISPLAY=:78 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
      export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
      export YGGTERM_TEST_SUPPRESS_MOUNT_IPC=1 YGGTERM_HOT_PREMOUNT_CAP=2
      export YGGTERM_TEST_STALL_SNAPSHOT_FETCH_MS=$STALL
      exec $BIN > /tmp/f1e2-gui.log 2>&1
    " >/dev/null 2>&1 </dev/null &
  else
    dbus-run-session -- bash -c "
      export DISPLAY=:78 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
      export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
      exec $BIN > /tmp/f1e2-healthy-gui.log 2>&1
    " >/dev/null 2>&1 </dev/null &
  fi
}

boot_wait_ready() {
  local READY=0 i
  for i in $(seq 1 90); do sleep 1; "$BIN" server app clients >/dev/null 2>&1 && READY=1 && break; done
  echo $READY
}

pkill -f "Xvfb :78" 2>/dev/null; sleep 1
Xvfb :78 -screen 0 1600x1000x24 -ac > /tmp/f1e2-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5

# ── PHASE 1: the synthesized race (BARS 1+2) ─────────────────────────────
SCRATCH=/tmp/f1e2-home-$(date +%s)
run_boot "$SCRATCH" 1
GUI_WRAP=$!
READY=$(boot_wait_ready); echo "daemon ready=$READY"; sleep 10
SCRATCH=$SCRATCH python3 - <<'PYEOF' > /tmp/f1e2-run.log 2>&1
import json, subprocess, time, os

bin_path = os.environ["BIN"]
STALL = int(os.environ.get("STALL", "500"))
TRACE = os.environ["SCRATCH"] + "/event-trace.jsonl"

def verb(*args, timeout=60):
    return subprocess.run([bin_path] + list(args), capture_output=True, text=True, timeout=timeout)

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

def wait_for(predicate, deadline_s, what):
    deadline = time.time() + deadline_s
    while time.time() < deadline:
        value = predicate(read_events())
        if value:
            return True, value
        time.sleep(0.05)
    return False, predicate(read_events())

def session_events(events, session, name):
    return [(ln, p) for ln, event_name, p in events
            if event_name == name and p.get("session_path") == session]

def row_session(n):
    out = verb("server", "app", "terminal", "new", "--kind", "shell", "--title", "fence%d" % n)
    try:
        return (json.loads(out.stdout).get("data") or {}).get("session_path")
    except Exception:
        print("new row raw:", out.stdout[:200], out.stderr[:200])
        return None

def page_screen_count(session, marker):
    """The PAGE-side assertion instrument: read-buffer reads the row's
    CLIENT buffer (the xterm host's applied state), not the daemon grid —
    the daemon always composes correctly, so only this read can see the
    erase."""
    out = verb("server", "app", "terminal", "read-buffer", session, "--mode", "screen")
    return (out.stdout or "").count(marker), (out.stdout or "")[-200:]

def daemon_screen_count(session, marker):
    return (verb("server", "screen", session).stdout or "").count(marker)

# ⛔ ROW SHAPE (measured this sitting): a SINGLE row under CAP=8 never
# armed — the remount rig's proven shape is a dummy first row + the test
# row second under CAP=2; startup-restore churn outlives the 60s patience
# either way, so no watch barrier is needed.
_a = row_session(0)
s = row_session(1)
if not s:
    print("FAIL: could not create row"); raise SystemExit(2)
print("rows: a=%s s=%s stall=%dms" % (_a, s, STALL))

ok, armed = wait_for(
    lambda ev: session_events(ev, s, "synthesized_mount_open_armed"), 60, "synthesized_mount_open_armed")
if not ok:
    print("PRE-FLIGHT FAIL: mount never armed the seed fetch — synthesis did not fire")
    raise SystemExit(4)
print("armed at trace line %d" % armed[-1][0])

# BAR 1: the repaint differential at armed+50ms — ends with cursor HOME.
time.sleep(0.05)
repaint_cmd = "printf '\\033[2J\\033[HF1EMARK\\033[H'"
out = verb("server", "app", "terminal", "probe-type", s, "--data", repaint_cmd, "--enter", "--mode", "xterm")
print("probe-type -> %s" % (out.stdout or out.stderr)[:120].replace("\n", " "))

ok, proof = wait_for(
    lambda ev: session_events(ev, s, "synthesized_mount_open"), 30, "synthesized_mount_open proof")
ok_stall = bool(wait_for(
    lambda ev: session_events(ev, s, "test_hook_snapshot_fetch_stalled"), 10, "stall hook trace")[0])
print("stall_hook_traced=%s proof_seen=%s" % (ok_stall, ok))
if not ok_stall:
    print("PRE-FLIGHT FAIL: the stall hook never traced — rig invalid (hook missing from build?)")
    raise SystemExit(2)

time.sleep(4)  # settle: the proof arm completes; the fence flush (if any) rides it

page_count, page_tail = page_screen_count(s, "F1EMARK")
daemon_count = daemon_screen_count(s, "F1EMARK")
proof_payload = (session_events(read_events(), s, "synthesized_mount_open") or [(0, {})])[-1][1]
flush_events = session_events(read_events(), s, "synth_output_fence_flushed")
print("VERDICT FENCE-ORDERING: page F1EMARK=%d (daemon=%d) seed_mode=%s wrote_seed=%s" % (
    page_count, daemon_count, proof_payload.get("seed_mode"), proof_payload.get("wrote_seed")))
print("   page tail: %r" % page_tail[-160:])
if flush_events:
    print("   fence flushed: %s" % json.dumps({k: flush_events[-1][1].get(k) for k in ("batches", "bytes")}))
fail_rc = 0
if page_count < 1:
    print("VERDICT RED CONFIRMED: the stale T0 seed ERASED the repaint differential page-side (marker lost)")
    fail_rc = fail_rc or 5
elif page_count != 1:
    print("VERDICT EXACTLY-ONCE FAIL: page F1EMARK=%d (want exactly 1)" % page_count)
    fail_rc = fail_rc or 6

# BAR 2: fence evidence (only meaningful once ordering holds).
if fail_rc == 0:
    seed_mode = proof_payload.get("seed_mode")
    if seed_mode != "fenced_repaint" or not flush_events:
        print("VERDICT FENCE-EVIDENCE FAIL: seed_mode=%s flush_events=%d (want fenced_repaint + >=1)" % (
            seed_mode, len(flush_events)))
        fail_rc = fail_rc or 6
    elif int(proof_payload.get("wrote_seed") or 0) <= 0:
        print("VERDICT FENCE-EVIDENCE FAIL: fenced seed wrote nothing (wrote_seed=%s)" % proof_payload.get("wrote_seed"))
        fail_rc = fail_rc or 6
    else:
        print("VERDICT FENCE-EVIDENCE: OK (fenced_repaint + flush rode the proof)")

if fail_rc:
    print("RIG FAIL rc=%d — verdicts above" % fail_rc)
    raise SystemExit(fail_rc)
print("RIG PASS — marker survived the stall exactly-once through the fence")
PYEOF
RC=$?
kill "$GUI_WRAP" 2>/dev/null; sleep 1
pkill -x yggterm 2>/dev/null
pkill -f "dbus-run-session" 2>/dev/null; sleep 1
# ⛔ REAP THE SCRATCH DAEMON (sitting-7 law): matched by YGGTERM_HOME ONLY —
# never by name (that would kill the fleet's live daemon on the host).
for p in $(pgrep -f 'yggterm-headless server daemon'); do
  h=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | sed -n 's/^YGGTERM_HOME=//p')
  [ "$h" = "$SCRATCH" ] && kill "$p" 2>/dev/null
done

# ── PHASE 2: the healthy control (BAR 3) — only if phase 1 passed ────────
if [ "$RC" = 0 ]; then
  HSCRATCH=/tmp/f1e2-healthy-$(date +%s)
  run_boot "$HSCRATCH" 0
  HGUI_WRAP=$!
  HREADY=$(boot_wait_ready); echo "healthy daemon ready=$HREADY"; sleep 10
  HSCRATCH=$HSCRATCH python3 - <<'PYEOF' > /tmp/f1e2-healthy-run.log 2>&1
import json, subprocess, time, os

bin_path = os.environ["BIN"]
TRACE = os.environ["HSCRATCH"] + "/event-trace.jsonl"

def verb(*args, timeout=60):
    return subprocess.run([bin_path] + list(args), capture_output=True, text=True, timeout=timeout)

def read_events():
    out = []
    try:
        for line in open(TRACE):
            try:
                event = json.loads(line)
            except Exception:
                continue
            name = (event.get("name") or event.get("event")
                    or (event.get("payload") or {}).get("name")
                    or (event.get("payload") or {}).get("event"))
            if name:
                out.append((event.get("ts_ms", 0), name, event.get("payload") or {}))
    except FileNotFoundError:
        pass
    return out

out = verb("server", "app", "terminal", "new", "--kind", "shell", "--title", "healthy")
s = (json.loads(out.stdout).get("data") or {}).get("session_path")
if not s:
    print("HEALTHY FAIL: no row"); raise SystemExit(7)
time.sleep(8)
verb("server", "app", "terminal", "probe-type", s, "--data", "echo HCGOOD", "--enter", "--mode", "xterm")
deadline = time.time() + 30
painted = 0
while time.time() < deadline:
    body = verb("server", "app", "terminal", "read-buffer", s, "--mode", "screen").stdout or ""
    if body.count("HCGOOD") >= 1:
        painted = body.count("HCGOOD")
        break
    time.sleep(0.5)
synth_names = [nm for _, nm, p in read_events()
               if ("synth" in nm or "fence" in nm or nm.startswith("test_hook"))
               and p.get("session_path") == s]
print("VERDICT HEALTHY-CONTROL: HCGOOD painted=%d synth/fence/hook events=%d %s" % (
    painted, len(synth_names), synth_names[:4] if synth_names ""))
if not (painted >= 1 and not synth_names):
    print("HEALTHY CONTROL FAIL — the fence leaked into a healthy mount")
    raise SystemExit(7)
print("HEALTHY CONTROL PASS")
PYEOF
  RC=$?
  kill "$HGUI_WRAP" 2>/dev/null; sleep 1
  pkill -x yggterm 2>/dev/null
  pkill -f "dbus-run-session" 2>/dev/null; sleep 1
  for p in $(pgrep -f 'yggterm-headless server daemon'); do
    h=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | sed -n 's/^YGGTERM_HOME=//p')
    [ "$h" = "$HSCRATCH" ] && kill "$p" 2>/dev/null
  done
  echo "=== HEALTHY DRIVER OUTPUT ==="; tail -6 /tmp/f1e2-healthy-run.log
fi

kill "$XVFB_PID" 2>/dev/null
echo "=== EXE PROOF (phase 1 scratch) ==="
EXE=$(grep -m1 -o '"executable_path":"[^"]*"' "$SCRATCH/event-trace.jsonl" 2>/dev/null | head -1 | cut -d'"' -f4)
echo "served executable_path=$EXE"
if [ "$EXE" = "$BIN" ]; then echo "EXE OK: worktree binary served this run"; else echo "EXE MISMATCH: verdicts above are VOID"; RC=8; fi
echo "=== DRIVER OUTPUT ==="
tail -30 /tmp/f1e2-run.log
echo "scratch=$SCRATCH rc=$RC"
exit $RC
