#!/bin/bash
# [11.229](a) THE REUSE-WITHOUT-REPAIR RIG (reuse-rig) — the falsifier for
# the reused-mount grid hole (queue line ~640; filed fix shape: launch-side
# reuse repair). HOOK-FREE — the simplest rig in the tree: a healthy boot,
# real switches, and a verb-poisoned PTY.
#   SETUP: hermetic GUI boot (exe-proof law L1/L2). Row a is created,
#          painted, and its CLIENT grid recorded via `echo STTY$(stty size)`
#          (healthy PTY == client grid). Row b is created (a demotes
#          quiescently, keeps its live host). a's PTY is then POISONED via
#          `server terminal resize --cols 100 --rows 30` — the [11.228] heal
#          verb in reverse: the PTY changes, the client xterm never learns
#          (no viewport resize, last-sent stale-equal), exactly the rotation
#          re-resume divergence.
#   ARM: switch back to a (`server app open a --view terminal`) — a REUSE
#          (mount_epoch_reused reused_live_host:true), the filed defect's
#          exact path.
#   BAR TRACE: terminal_startup_resize_repair with source=mount_epoch_reused
#          for a — absent on unfixed main, present on the lane.
#   BAR EFFECT (owner-visible): probe-type `echo STTY$(stty size)` on a
#          after the reuse — RED prints STTY30 100 (the poison survived the
#          reuse; the row renders at the wrong grid), GREEN prints the
#          client grid (the reuse healed the PTY).
#   BAR CONTROL (lane builds): a healthy never-poisoned reuse on b fires the
#          repair and leaves the screen nonblank.
# FAILED VERDICTS EXIT NONZERO AFTER ALL ARMS:
# 2=rows, 3=reuse never fired, 4=pre-flight, 5=control regression, 6=RED
# CONFIRMED (expected on unfixed main), 7=exe, 8=unexpected heal path,
# 9=repair fired but heal failed. rc=0 = GREEN.
# Usage: tools/uxspeed/reuse-rig.sh <binary>   (absolute path; full-workspace
# release build per rig law L1).
set -u
BIN=${1:?usage: reuse-rig.sh <binary>}
SCRATCH=/tmp/11229a-home-$(date +%s)
mkdir -p "$SCRATCH"
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN — build first"; exit 2; fi
export BIN
pkill -f "Xvfb :81" 2>/dev/null; sleep 1
Xvfb :81 -screen 0 1600x1000x24 -ac > /tmp/11229a-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5
# ⛔ HERMETIC ([11.231]): XDG_DATA_HOME must point at the scratch or the
# binary re-execs the INSTALLED direct build; asserted via the trace's
# register event below (EXE PROOF).
export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
dbus-run-session -- bash -c "
  export DISPLAY=:81 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
  export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
  exec $BIN > /tmp/11229a-gui.log 2>&1
" >/dev/null 2>&1 </dev/null &
GUI_WRAP=$!
READY=0; for i in $(seq 1 90); do sleep 1; "$BIN" server app clients >/dev/null 2>&1 && READY=1 && break; done
echo "daemon ready=$READY after ${i}s"; sleep 8
python3 - <<'PYEOF' > /tmp/11229a-run.log 2>&1
import json, re, subprocess, time, os

bin_path = os.environ["BIN"]
def verb(*args, timeout=90):
    return subprocess.run([bin_path] + list(args), capture_output=True, text=True, timeout=timeout)

TRACE = os.environ.get("YGGTERM_HOME", "/tmp/11229a-home-unknown") + "/event-trace.jsonl"

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

def screen_text(session):
    return verb("server", "screen", session).stdout or ""

def stty_read(session, marker):
    """Types `echo STTY$(stty size)` and parses the marker line off the screen."""
    out = verb("server", "app", "terminal", "probe-type", session,
               "--data", "echo STTY$(stty size)", "--enter", "--mode", "xterm")
    try:
        verdict = json.loads(out.stdout).get("data") or {}
    except Exception:
        print("probe-type raw: %s %s" % (out.stdout[:200], out.stderr[:200]))
        return None
    for _ in range(40):
        time.sleep(0.5)
        m = re.findall(r"STTY(\d+) (\d+)", screen_text(session))
        if m:
            return tuple(int(x) for x in m[-1])
    print("probe-type accepted=%s reason=%s" % (verdict.get("accepted"), verdict.get("reason")))
    print("screen tail: %r" % screen_text(session)[-240:])
    return None

def row_session(n):
    out = verb("server", "app", "terminal", "new", "--kind", "shell", "--title", "reuse%d" % n)
    try:
        return (json.loads(out.stdout).get("data") or {}).get("session_path")
    except Exception:
        print("new row raw:", out.stdout[:200], out.stderr[:200])
        return None

# --- pre-flight: row a painted + client grid recorded ----------------------
a = row_session(1)
if not a:
    print("FAIL: could not create row a"); raise SystemExit(2)
ok, _ = wait_for(lambda ev: screen_text(a).strip() != "", 45, "a painted")
if not ok:
    print("PRE-FLIGHT FAIL: row a never painted"); raise SystemExit(4)
time.sleep(2)
client_grid = stty_read(a, "STTY0")
if not client_grid or client_grid[0] < 10:
    print("PRE-FLIGHT FAIL: client grid unreadable (%r)" % (client_grid,)); raise SystemExit(4)
print("client grid (rows, cols) = %s" % (client_grid,))

# --- row b demotes a quiescently -------------------------------------------
b = row_session(2)
if not b:
    print("FAIL: could not create row b"); raise SystemExit(2)
time.sleep(4)

# --- POISON a's PTY while demoted (the [11.228] vector, verb-reversed) ------
poison = verb("server", "terminal", "resize", a, "--cols", "100", "--rows", "30")
print("poison resize: %s" % (poison.stdout or poison.stderr).strip()[:120])
time.sleep(2)

# --- THE ARM: switch back to a — the REUSE ---------------------------------
switch_line = len(open(TRACE).readlines())
verb("server", "app", "open", a, "--view", "terminal")
ok, _ = wait_for(lambda ev: session_events(ev, a, "mount_epoch_reused"), 15, "reuse fired")
if not ok:
    print("VERDICT WINDOW: mount_epoch_reused never fired for a — rig invalid")
    raise SystemExit(3)
print("mount_epoch_reused fired for a")
time.sleep(5)  # repair window: 180ms delay + verb + SIGWINCH + redraw

# --- BAR TRACE: the repair under the reuse source --------------------------
events = read_events()
repairs = [(ln, p) for ln, n, p in events
           if n == "terminal_startup_resize_repair"
           and p.get("session_path") == a and p.get("source") == "mount_epoch_reused"]
skips = [(ln, p) for ln, n, p in events
         if n == "reuse_repair_skipped" and p.get("session_path") == a]
repair_errors = [(ln, p) for ln, n, p in events
                 if n == "terminal_startup_resize_repair_error" and p.get("session_path") == a]
print("reuse repairs for a: %d  skips: %d  errors: %d" % (len(repairs), len(skips), len(repair_errors)))
for ln, p in skips[-3:]:
    print("   skip line %d: reason=%s" % (ln, p.get("reason")))
for ln, p in repairs[-3:]:
    print("   repair line %d: cols=%s rows=%s" % (ln, p.get("cols"), p.get("rows")))
repair_fired = bool(repairs)

# --- BAR EFFECT: stty after the reuse --------------------------------------
post_grid = stty_read(a, "STTY1")
print("post-reuse stty (rows, cols) = %r (client %r, poison 30x100)" % (post_grid, client_grid))
if post_grid is None:
    print("VERDICT EFFECT: stty unreadable — row dead after reuse?"); raise SystemExit(8)

poisoned = post_grid == (30, 100)
healed = post_grid == client_grid

# --- BAR CONTROL (healthy reuse on b; meaningful on lane builds) -----------
control_ok = True
if repair_fired:
    verb("server", "app", "open", a, "--view", "terminal"); time.sleep(2)
    verb("server", "app", "open", b, "--view", "terminal"); time.sleep(1)
    verb("server", "app", "open", a, "--view", "terminal"); time.sleep(2)
    verb("server", "app", "open", b, "--view", "terminal"); time.sleep(5)
    ev2 = read_events()
    b_repairs = [(ln, p) for ln, n, p in ev2
                 if n == "terminal_startup_resize_repair"
                 and p.get("session_path") == b and p.get("source") == "mount_epoch_reused"]
    b_reuses = session_events(ev2, b, "mount_epoch_reused")
    b_text = screen_text(b)
    control_ok = bool(b_repairs) and b_text.strip() != ""
    print("CONTROL (healthy reuse on b): reuses=%d repairs=%d screen_nonblank=%s -> %s" % (
        len(b_reuses), len(b_repairs), b_text.strip() != "",
        "GREEN" if control_ok else "RED"))
    if not b_text.strip():
        print("  b screen tail: %r" % b_text[-200:])
else:
    print("CONTROL: skipped (no repair machinery on this build — unfixed main)")

# --- VERDICT ----------------------------------------------------------------
if repair_fired and healed:
    if not control_ok:
        print("VERDICT: GREEN heal but CONTROL FAILED — healthy reuse regressed"); raise SystemExit(5)
    print("RIG: GREEN — reuse repair fired, PTY converged to the client grid, control clean")
    raise SystemExit(0)
if repair_fired and poisoned:
    print("RIG: rc9 — repair fired but the PTY stayed poisoned (heal failed)"); raise SystemExit(9)
if not repair_fired and poisoned:
    print("RIG: RED CONFIRMED — no reuse repair, poison survived the reuse (the (a) defect)")
    raise SystemExit(6)
print("RIG: rc8 — unexpected state: no repair trace but stty=%r (healed by something else — "
      "scan resize events below)" % (post_grid,))
for ln, n, p in events:
    if ln >= switch_line and "resize" in n and p.get("session_path") == a:
        print("   line %d %s %s" % (ln, n, json.dumps(p)[:160]))
raise SystemExit(8)
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
tail -60 /tmp/11229a-run.log
echo "scratch=$SCRATCH rc=$RC"
exit $RC
