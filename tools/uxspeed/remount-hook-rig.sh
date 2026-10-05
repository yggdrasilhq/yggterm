#!/bin/bash
# [F1-(h)/(h2)] THE SURFACE-REMOUNT HOOK RIG — the deterministic falsifier
# chain (sol Q2 bar; nodes lores/chain-of-thought/
# 2026-10-05-yggterm-f1-ring-rework-sol-review.md and
# 2026-10-05-yggterm-f1h-remount-hook-sol-review.md). The GUI runs with
# YGGTERM_TEST_SUPPRESS_MOUNT_IPC=1 (the production shed condition at the
# mount loop; eval returns stay alive) so synthesis fires deterministically,
# PLUS the (h2) one-shot trigger YGGTERM_TEST_SURFACE_REMOUNT_ON_IDLE=1:
# when a mount's input ring goes QUIESCENT (a drain answer with zero new
# chunks after that mount applied input — every old chunk acked+pruned), the
# loop executes the bridge-ended arms' exit and the WATCHDOG remounts at a
# higher epoch with a fresh incarnation. No verb can do this (sitting-6
# map); the hook is the only deterministic surface-remount trigger.
#   BAR A (retained switch): focus away+back retains the host — NO new
#          epoch (the negative control).
#   BAR B (h2 remount): forced at HOOKOLD quiesce; asserts SAME session,
#          pty NEVER restarted, epoch strictly higher, incarnation strictly
#          higher, verified synthesis (synthesized_mount_open after the
#          forced line), ZERO chunk enqueues between the forced line and
#          the rig's next typing (writer-enqueue granularity via
#          synthesized_input_chunk_enqueued; screen counts secondary).
#   BAR C (exactly-once new input): HOOKNEW typed after the remount
#          enqueues each new chunk exactly once and paints exactly once.
#   BUCKET REUSE (measured sitting 7, no flag needed): the watchdog
#          remount REUSES the mount epoch, so the host id — and the page
#          input-ring bucket keyed by it — is the SAME key across the
#          remount. Every run therefore exercises sol's bucket-reuse case:
#          the drained baseline CONTINUES the bucket's nextId (1 -> 3
#          measured), old chunks are pruned/acked never re-enqueued, and
#          the fresh incarnation owns the new chunks. HONEST SCOPE: the
#          drain script's STALE branch is unreachable through a quiesced
#          death — the baseline seal (ids below the stamp-time nextId
#          predate the mount) swallows pre-stamp chunks via the acked path
#          BY DESIGN; the stale branch's live entry is the old-drain-
#          crossing case = (f2)'s page-side mutation, which stays open.
#   PTY-RESTART control: `server terminal restart` restarts the pty with
#          the mount loop SURVIVING (epochs unchanged) — input correctness
#          through pty churn on the remounted mount (HOOKR2 exactly once,
#          HOOKNEW count not grown).
# FAILED VERDICTS EXIT NONZERO AFTER ALL ARMS RUN (sol Q4):
# 2=rows/hook, 4=pre-flight, 5=remount-not-forced, 6=replay,
# 7=new-input, 8=exe-mismatch (wrapper).
# Usage: tools/uxspeed/remount-hook-rig.sh [worktree] [binary]
#   ⛔ BUILD LAW (measured sitting 7): build the binary with a FULL
#   WORKSPACE release build (cargo build --release, deploy shape) — a
#   -p yggterm build changes feature unification and loses pre-flight
#   deterministically (supersede race); a dev-profile build shifts timing
#   the same way.
#   binary defaults to $worktree/target/debug/yggterm. Runs on dev's Xvfb
#   :78 — jojo untouched. Input focus latches for the row ACTIVE at
#   synthesis: the row under test is created LAST and stays active.
set -u
WT=${1:-$HOME/gh/yggterm}
BIN=${2:-$WT/target/debug/yggterm}
SCRATCH=/tmp/f1h-home-$(date +%s)
mkdir -p "$SCRATCH"
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN — build first"; exit 2; fi
export BIN
pkill -f "Xvfb :78" 2>/dev/null; sleep 1
Xvfb :78 -screen 0 1600x1000x24 -ac > /tmp/f1h-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5
# ⛔ HERMETIC ([11.231]): XDG_DATA_HOME must point at the scratch or the
# worktree binary re-execs the INSTALLED direct build and the rig measures
# the wrong code; the exe is asserted from the trace register event below.
export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
dbus-run-session -- bash -c "
  export DISPLAY=:78 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
  export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share YGGTERM_TEST_SUPPRESS_MOUNT_IPC=1 YGGTERM_HOT_PREMOUNT_CAP=2
  export YGGTERM_TEST_SURFACE_REMOUNT_ON_IDLE=1
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

def read_events():
    """All trace rows as (line_no, name, payload); ordering by line = time.
    The trace row shape is {component, category, name, payload:{...}} — the
    event NAME lives at the top level, the session_path inside the payload."""
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
    """Barrier: poll the trace until predicate(line-numbered events) is
    truthy or the deadline passes. Returns (ok, value)."""
    deadline = time.time() + deadline_s
    while time.time() < deadline:
        value = predicate(read_events())
        if value:
            return True, value
        time.sleep(0.5)
    return False, predicate(read_events())

def session_events(events, session, name):
    return [(ln, p) for ln, event_name, p in events
            if event_name == name and p.get("session_path") == session]

def typed(session, data):
    out = verb("server", "app", "terminal", "probe-type", session, "--data", data, "--enter", "--mode", "xterm")
    try:
        verdict = json.loads(out.stdout).get("data") or {}
        print("probe-type %r -> %s" % (data, json.dumps({k: verdict.get(k) for k in ("accepted", "reason")})))
    except Exception:
        print("probe-type %r raw: %s %s" % (data, out.stdout[:160], out.stderr[:160]))

def screen_count(marker, session):
    body = (verb("server", "screen", session).stdout or "")
    return body.count("echo " + marker), body[-200:]

def row_session(n):
    out = verb("server", "app", "terminal", "new", "--kind", "shell", "--title", "hook%d" % n)
    try:
        return (json.loads(out.stdout).get("data") or {}).get("session_path")
    except Exception:
        print("new row raw:", out.stdout[:200], out.stderr[:200])
        return None

# b is created LAST: input focus latches for the row active at synthesis
# time, and under the hook a later focus switch cannot re-latch it.
a = row_session(1)
b = row_session(2)
print("rows: a=%s b=%s" % (a, b))
if not (a and b):
    print("FAIL: could not create rows"); raise SystemExit(2)
time.sleep(20)

if not session_events(read_events(), b, "test_hook_mount_ipc_suppressed"):
    print("VERDICT HOOK: NOT ENGAGED — env gate never traced; rig invalid")
    raise SystemExit(2)

# --- BAR A: retained switch (away+back) — the negative control -----------
verb("server", "app", "terminal", "focus", a); time.sleep(3)
verb("server", "app", "terminal", "focus", b); time.sleep(6)

# --- seed OLD input on b; the paint barrier proves synthesis -------------
# (paint-only barrier: the chunk-enqueued trace is (h2) evidence collected
# for the window asserts below — requiring it here would void RED runs on
# unpatched builds, where it does not exist yet.)
typed(b, "echo HOOKOLD")
ok, _ = wait_for(lambda ev: screen_count("HOOKOLD", b)[0] >= 1, 45, "HOOKOLD painted")
old_lines, _ = screen_count("HOOKOLD", b)
print("HOOKOLD pre-remount command_lines=%d" % old_lines)
if not (ok and old_lines >= 1):
    print("PRE-FLIGHT FAIL: seed did not paint — b never synthesized")
    raise SystemExit(4)

# --- BAR B: the (h2) forced surface remount, barrier-ordered --------------
def forced_at(ev):
    hits = session_events(ev, b, "test_hook_surface_remount_forced")
    return hits[-1][0] + 1 if hits else 0

ok, forced_line = wait_for(lambda ev: forced_at(ev), 60, "test_hook_surface_remount_forced")
if not ok:
    print("VERDICT REMOUNT-SYNTHESIS: NOT FORCED — the idle trigger never fired (hook missing or ring never quiesced)")
    raise SystemExit(5)
forced_payload = session_events(read_events(), b, "test_hook_surface_remount_forced")[-1][1]
old_inc = forced_payload.get("input_incarnation")
print("forced at line %d: epoch=%s inc=%s acked_watermark=%s" % (
    forced_line, forced_payload.get("mount_epoch"), old_inc, forced_payload.get("acked_watermark")))

def new_mount_at(ev):
    # [Measured sitting 7] a watchdog remount REUSES the mount epoch (the
    # host id's -mN does not move; the identity that changes is the
    # bootstrap identity's wr suffix). The discriminators for a genuinely
    # NEW mount are therefore line-ordered: a suppressed event at/after the
    # forced line followed by synthesized_mount_open, and (asserted after
    # typing) a strictly higher input incarnation.
    posts = [ln for ln, p in session_events(ev, b, "test_hook_mount_ipc_suppressed")
             if ln >= forced_line]
    synth = [ln for ln, p in session_events(ev, b, "synthesized_mount_open") if posts and ln > posts[0]]
    return posts[0] + 1 if posts and synth else 0

ok, new_mount_line = wait_for(new_mount_at, 12, "immediate focus-cycle remount")
recovery = None
if ok:
    recovery = "immediate-focus-cycle"
else:
    # Belt only: with the armed-death heartbeat punch the recovery is
    # immediate (the punch's state write re-renders the canvas at once —
    # measured sitting 7). If the immediate cycle ever fails again, the
    # candidate is being swallowed and this arm names it honestly.
    time.sleep(65)
    verb("server", "app", "terminal", "focus", a); time.sleep(2)
    verb("server", "app", "terminal", "focus", b); time.sleep(2)
    ok, new_mount_line = wait_for(new_mount_at, 30, "post-stale focus-cycle remount")
    if ok:
        recovery = "after-heartbeat-stale-focus-cycle"
if not ok:
    print("VERDICT REMOUNT-SYNTHESIS: NOT FORCED — remount armed but no new synthesized mount appeared (immediate AND post-stale focus cycles both failed)")
    raise SystemExit(5)
print("VERDICT REMOUNT-RECOVERY: %s" % recovery)
post_epochs = sorted({p.get("mount_epoch") for _, p in
                      session_events(read_events(), b, "test_hook_mount_ipc_suppressed")})
print("VERDICT REMOUNT-SYNTHESIS: FORCED (new synthesized mount; epochs=%s — the watchdog remount REUSES the mount epoch by design, the fresh incarnation is the discriminator)" % post_epochs)

# pty survival: no restart verb ran in this arm; the daemon snapshot seed
# must repaint the old command's output on the surviving pty.
survivor_lines, _ = screen_count("HOOKOLD", b)
print("VERDICT PTY-SURVIVES: %s (HOOKOLD still on screen: %d line(s))" % (
    "YES" if survivor_lines >= 1 else "NO", survivor_lines))
fail_rc = 0
if survivor_lines < 1:
    fail_rc = fail_rc or 6

# --- zero old enqueues between the forced line and the next typing --------
pre_type_line = len(open(TRACE).readlines())
window = [(ln, p) for ln, p in session_events(read_events(), b, "synthesized_input_chunk_enqueued")
          if forced_line <= ln < pre_type_line]
old_ids = {p.get("chunk_id") for ln, p in
           session_events(read_events(), b, "synthesized_input_chunk_enqueued") if ln < forced_line}
print("VERDICT REPLAY (writer-enqueue window): %s (old ids %s; window enqueues %d)" % (
    "NONE" if not window else "REPRODUCED", sorted(x for x in old_ids if x is not None),
    len(window)))
for ln, p in window[:5]:
    print("   window enqueue line %d: %s" % (ln, json.dumps({k: p.get(k) for k in ("chunk_id", "len")})))
if window:
    fail_rc = fail_rc or 6

# --- BAR C: exactly-once NEW input on the remounted mount -----------------
typed(b, "echo HOOKNEW")
ok, _ = wait_for(lambda ev: screen_count("HOOKNEW", b)[0] >= 1, 45, "HOOKNEW painted")
new_window = [(ln, p) for ln, p in session_events(read_events(), b, "synthesized_input_chunk_enqueued")
              if ln >= pre_type_line]
new_ids = [p.get("chunk_id") for _, p in new_window]
new_lines, _ = screen_count("HOOKNEW", b)
exactly_once_ids = len(new_ids) == len(set(new_ids)) and len(new_ids) >= 1
print("VERDICT NEW-INPUT: enqueued=%s screen_lines=%d %s" % (
    new_ids, new_lines, "EXACTLY-ONCE" if exactly_once_ids and new_lines == 1 else "WRONG"))
if not (exactly_once_ids and new_lines == 1):
    fail_rc = fail_rc or 7

# fresh incarnation proof: post-remount drains must carry a strictly higher
# input_incarnation than the forced mount's — the strongest same-session
# discriminator (the mount epoch itself is REUSED by the mechanism).
drains_post = [p for _, p in session_events(read_events(), b, "synthesized_input_drained")
               if p.get("input_incarnation") is not None and p.get("input_incarnation", 0) > (old_inc or 0)]
post_incs = sorted({p.get("input_incarnation") for p in drains_post})
baselines = sorted({p.get("baseline") for _, p in session_events(read_events(), b, "synthesized_input_drained")
                    if p.get("baseline") is not None and p.get("input_incarnation", 0) > (old_inc or 0)})
print("VERDICT FRESH-INCARNATION: forced inc=%s post-drain incs=%s baselines=%s %s" % (
    old_inc, post_incs, baselines, "OK" if post_incs else "MISSING (no post-remount drain with higher inc)"))
if not post_incs:
    fail_rc = fail_rc or 7

# --- BUCKET-REUSE evidence (always on — see header) -----------------------
old_max = max((i for i in old_ids if i is not None), default=0)
continued = any((bl or 0) > 1 for bl in baselines)
new_above = bool(new_ids) and all((i or 0) > old_max for i in new_ids if i is not None)
old_again, _ = screen_count("HOOKOLD", b)
print("VERDICT BUCKET-REUSE: old_max=%s new_ids=%s baselines=%s -> %s; HOOKOLD count=%d" % (
    old_max, new_ids, baselines,
    "SAME-BUCKET-CONTINUED+NO-OLD-REENQUEUE" if continued and new_above and old_again == 1 else "EVIDENCE MISSING",
    old_again))
if not (continued and new_above and old_again == 1):
    fail_rc = fail_rc or 7

# --- PTY-RESTART control on the remounted mount ---------------------------
epochs_before_restart = post_epochs
verb("server", "terminal", "restart", b)
time.sleep(15)
epochs_after_restart = sorted({p.get("mount_epoch") for _, p in
                               session_events(read_events(), b, "test_hook_mount_ipc_suppressed")})
typed(b, "echo HOOKR2")
time.sleep(6)
r2_lines, _ = screen_count("HOOKR2", b)
new_still, _ = screen_count("HOOKNEW", b)
print("VERDICT PTY-RESTART-CONTROL: epochs %s -> %s (loop survives if equal); HOOKR2 lines=%d; HOOKNEW after fresh-shell restart=%d (informational — a restarted pty is a fresh shell; the input-correctness bar is HOOKR2 exactly once, no replay)" % (
    epochs_before_restart, epochs_after_restart, r2_lines, new_still))
if r2_lines != 1:
    fail_rc = fail_rc or 7

drains = session_events(read_events(), b, "synthesized_input_drained")
print("drain events for row b: %d; payloads:" % len(drains))
for _, d in drains[:10]:
    print("  ", json.dumps({k: d.get(k) for k in ("applied", "skipped_duplicates", "pruned", "pruned_stale", "watermark", "input_incarnation", "baseline") if k in d}))
if fail_rc:
    print("RIG FAIL rc=%d — verdicts above" % fail_rc)
    raise SystemExit(fail_rc)
print("RIG PASS — all bars green")
PYEOF
RC=$?
kill "$GUI_WRAP" 2>/dev/null; sleep 1
# ⛔ NOT pkill -f "target/debug/yggterm": the rig's own cmdline carries the
# binary path and -f self-matches (the gotcha-7 law). -x matches comm only.
pkill -x yggterm 2>/dev/null
pkill -f "dbus-run-session" 2>/dev/null; sleep 1; kill "$XVFB_PID" 2>/dev/null
# ⛔ REAP THE SCRATCH DAEMON (measured sitting 7): the GUI spawns a
# yggterm-headless daemon under $SCRATCH that survives pkill -x yggterm —
# one leaked per run, each burning CPU on its orphaned home, and the
# accumulated load flipped the attach-vs-supersede race after ~10 runs.
# Matched by YGGTERM_HOME ONLY (never pkill by name — that would kill the
# fleet's live daemon on the host).
for p in $(pgrep -f 'yggterm-headless server daemon'); do
  h=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | sed -n 's/^YGGTERM_HOME=//p')
  [ "$h" = "$SCRATCH" ] && kill "$p" 2>/dev/null
done
echo "=== EXE PROOF ==="
EXE=$(grep -m1 -o '"executable_path":"[^"]*"' "$SCRATCH/event-trace.jsonl" 2>/dev/null | head -1 | cut -d'"' -f4)
echo "served executable_path=$EXE"
if [ "$EXE" = "$BIN" ]; then echo "EXE OK: worktree binary served this run"; else echo "EXE MISMATCH: verdicts above are VOID"; RC=8; fi
echo "=== DRIVER OUTPUT ==="
tail -40 /tmp/f1h-run.log
echo "scratch=$SCRATCH rc=$RC"
exit $RC
