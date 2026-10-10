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
#   BAR 4 (dup mode only, sitting 18): the [F1-(e-r1)] duplication
#          falsifier — MODE=dup adds the daemon-side capture-stall hook
#          (YGGTERM_TEST_STALL_SNAPSHOT_CAPTURE_MS, hook-carrying builds
#          only): the snapshot's CAPTURE is held open so the echo marker
#          is deterministically INSIDE the seeded screen while its chunk
#          still resolves at a client poll inside the fence window. An
#          unfixed build flushes the retained chunk over the seed (page
#          marker count doubles — rc 9, THE RED); a fixed build drops it
#          as seed-covered (page count stays at exactly the seed's one
#          copy) AND traces the drop. Exit 9 = the duplication RED.
#   BAR 5 (loss mode only, sitting 21): the [F1-(e-r1)-R1] DATA-LOSS
#          falsifier (sol's ONE NEXT MEASUREMENT from the s18 consult) —
#          MODE=loss adds YGGTERM_TEST_SEED_FORCED_SKIP=1 on top of the
#          dup machinery: the proof is forced to skip BEFORE the first
#          page seed write, so applied-seed acks are ZERO by construction
#          while the daemon ring+screen carry the marker and the fence
#          retained its covered batch. An unfixed build DESTROYS the
#          covered batch at seed arrival (drop trace) — the marker
#          reaches the page nowhere except via the daemon-side reconcile
#          replay (the heal — a mask, not delivery; exit 10, THE RED). A
#          fixed build drops nothing, traces the explicit unapplied
#          failure, and flushes EVERYTHING live — the marker is delivered
#          by the fence's own write path inside the transient window.
# Usage: tools/uxspeed/fence-rig.sh [worktree] [binary] [stall_ms] [mode]
#   ⛔ BUILD LAW (measured sitting 7): FULL-WORKSPACE release build — a
#   -p yggterm build changes feature unification and shifts timing.
#   Runs on dev's Xvfb :78 — guihost untouched.
set -u
WT=${1:-$HOME/gh/yggterm}
BIN=${2:-$WT/target/release/yggterm}
STALL=${3:-500}
MODE=${4:-order}
# dup-mode defaults sized for the probe-type verb's ~1.2s round trip:
# marker must land in (arm, capture); seed delivery at capture+STALL.
CAPTURE_STALL=${CAPTURE_STALL:-2000}
if [ "$MODE" != order ] && [ "$MODE" != dup ] && [ "$MODE" != loss ] && [ "$MODE" != flushshed ] && [ "$MODE" != lateflush ] && [ "$MODE" != latecontrol ] && [ "$MODE" != blank ] && [ "$MODE" != negative ]; then echo "MODE must be order|dup|loss|flushshed|lateflush|latecontrol|blank|negative"; exit 2; fi
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN — build first"; exit 2; fi
export BIN STALL MODE CAPTURE_STALL

run_boot() {  # $1=scratch $2=synthesized-envs(1|0) -> boots GUI, echoes wrap pid
  local SCRATCH=$1 SYNTH=$2
  mkdir -p "$SCRATCH"
  export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
  CAPT_EXPORT=""
  if [ "$MODE" = dup ] || [ "$MODE" = loss ] || [ "$MODE" = flushshed ] || [ "$MODE" = lateflush ] || [ "$MODE" = latecontrol ] || [ "$MODE" = blank ] || [ "$MODE" = negative ]; then
    CAPT_EXPORT="export YGGTERM_TEST_STALL_SNAPSHOT_CAPTURE_MS=$CAPTURE_STALL;"
  fi
  if [ "$MODE" = blank ]; then
    CAPT_EXPORT="$CAPT_EXPORT export YGGTERM_TEST_SNAPSHOT_CAPTURE_RING_ONLY=1;"
  fi
  if [ "$MODE" = negative ]; then
    # [Q7-R6] PHASE A (the withheld boot): the first-live-write receipt is
    # WITHHELD (armed but never published/pushed) and the seed is forced to
    # skip — ZERO application evidence may exist, so NOTHING may promote.
    CAPT_EXPORT="$CAPT_EXPORT export YGGTERM_TEST_SNAPSHOT_CAPTURE_RING_ONLY=1 YGGTERM_TEST_APPLIED_CONTENT_WITHHOLD=1;"
  fi
  if [ "$MODE" = loss ] || [ "$MODE" = flushshed ] || [ "$MODE" = lateflush ] || [ "$MODE" = latecontrol ] || [ "$MODE" = negative ]; then
    CAPT_EXPORT="$CAPT_EXPORT export YGGTERM_TEST_SEED_FORCED_SKIP=1;"
  fi
  if [ "$MODE" = flushshed ]; then
    CAPT_EXPORT="$CAPT_EXPORT export YGGTERM_TEST_FLUSH_EVAL_DROP=1;"
  fi
  if [ "$MODE" = lateflush ]; then
    CAPT_EXPORT="$CAPT_EXPORT export YGGTERM_TEST_FLUSH_EXEC_DELAY_MS=5000;"
  fi
  if [ "$MODE" = latecontrol ]; then
    CAPT_EXPORT="$CAPT_EXPORT export YGGTERM_TEST_FLUSH_EXEC_DELAY_MS=5000 YGGTERM_TEST_FLUSH_SUPERSESSION_OFF=1;"
  fi
  if [ "$SYNTH" = 1 ]; then
    dbus-run-session -- bash -c "
      export DISPLAY=:78 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
      export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
      export YGGTERM_TEST_SUPPRESS_MOUNT_IPC=1 YGGTERM_HOT_PREMOUNT_CAP=2
      export YGGTERM_TEST_STALL_SNAPSHOT_FETCH_MS=$STALL
      $CAPT_EXPORT
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
MODE = os.environ.get("MODE", "order")
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
    lambda ev: session_events(ev, s, "synthesized_mount_open_armed"), 150, "synthesized_mount_open_armed")
if not ok:
    print("PRE-FLIGHT FAIL: mount never armed the seed fetch — synthesis did not fire")
    raise SystemExit(4)
print("armed at trace line %d" % armed[-1][0])

# BAR 1 (order) / BAR 4 (dup): the differential leg.
# order: the repaint differential at armed+50ms — ends with cursor HOME.
# dup: NO verb leg at all — the daemon-side capture hook injects the
# marker into the ring+screen BEFORE its stall, by construction inside
# the seed and still in flight at the client's polls.
time.sleep(0.05)
# marker on ROW 2 with the cursor parked HOME: the guard still reads
# unpainted (baseY/cursorX/cursorY all 0) but bash's next prompt lands on
# row 1 and cannot overwrite the marker (the v1 command homed the cursor
# ON the marker row and the prompt erased it — the marker must survive
# its own shell's prompt to be assertable).
# the trailing sleep keeps the shell BUSY so nothing (no next
# prompt) prints after the homing — at seed time the cursor sits at
# (0,0) with a non-empty buffer, which is exactly the painted-guard
# hole (measured: without it bash's own next prompt leaves cursorX>0
# and the guard rightly blocks the stale seed — the falsifier goes
# vacuously green).
if MODE == "order":
    repaint_cmd = "printf '\\033[2J\\033[H\\033[BF1EMARK\\033[H'; sleep 6"
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

if MODE == "blank":
    # [Q7] BAR 7 — THE VALID-BLANK FALSIFIER (sitting 23, sol round 1 R5):
    # the capture hook injects the marker into the RING ONLY and the
    # snapshot answers the AUTHORITATIVE BLANK (empty text, post-injection
    # stamp). The seed MUST ack as an empty repaint — its own control
    # bytes are the write — and drop the marker batch as seed-covered.
    # RED (the unfixed blank filter): Some("") becomes JS null before the
    # write arm — mode 'skipped', wrote_seed 0, NO ack — so coverage
    # never commits and the retained marker batch flushes LIVE over the
    # blank page: ERASED CONTENT RESURRECTED (exit 12). GREEN: blank
    # acked (fenced_repaint, wrote_seed=7, blank=true, receipt), the
    # covered drop traced, the page stays marker-free through every
    # transient sample.
    proof_payload = (session_events(read_events(), s, "synthesized_mount_open") or [(0, {})])[-1][1]
    ring_only_hook = bool(wait_for(
        lambda ev: any(e[2].get("injected_seq") is not None for e in ev
                       if e[1] == "test_hook_snapshot_capture_ring_only"), 10, "ring-only hook")[0])
    print("ring_only_hook=%s seed_mode=%s wrote_seed=%s blank=%s" % (
        ring_only_hook, proof_payload.get("seed_mode"), proof_payload.get("wrote_seed"), proof_payload.get("blank")))
    if not ring_only_hook:
        print("PRE-FLIGHT FAIL: ring-only hook never traced — rig invalid (hook missing from build?)")
        raise SystemExit(2)
    if proof_payload.get("seed_mode") == "test_forced_skip":
        print("PRE-FLIGHT FAIL: forced-skip env leaked into blank mode")
        raise SystemExit(4)
    samples = []
    for delay in (0.15, 0.5, 1.0):
        time.sleep(delay)
        c, _ = page_screen_count(s, "E1RMARK")
        samples.append(c)
    time.sleep(4)
    page_count, page_tail = page_screen_count(s, "E1RMARK")
    events = read_events()
    drop_events = (session_events(events, s, "synth_output_fence_seed_covered_drop")
                   + session_events(events, s, "synth_output_fence_poll_filtered"))
    flush_events = session_events(events, s, "synth_output_fence_flushed")
    retained_events = session_events(events, s, "synth_output_fence_retained")
    dropped_batches = sum(int(p.get("dropped_batches") or 0) for _, p in drop_events)
    flushed_bytes = sum(int(p.get("bytes") or 0) for _, p in flush_events)
    retained_bytes_total = sum(int(p.get("bytes") or 0) for _, p in retained_events)
    print("VERDICT FENCE-BLANK: page E1RMARK=%d samples=%s (want all 0 — the blank stays blank)" % (page_count, samples))
    print("   proof: mode=%s wrote=%s blank=%s receipt=%s applied=%s" % (
        proof_payload.get("seed_mode"), proof_payload.get("wrote_seed"), proof_payload.get("blank"),
        proof_payload.get("receipt_qualifies"), proof_payload.get("seed_applied")))
    print("   fence: retained=%dB/%d flushed=%dB/%d dropped_batches=%d" % (
        retained_bytes_total, len(retained_events), flushed_bytes, len(flush_events), dropped_batches))
    print("   page tail: %r" % page_tail[-160:])
    fail_rc = 0
    seed_mode = proof_payload.get("seed_mode")
    wrote = int(proof_payload.get("wrote_seed") or 0)
    if seed_mode != "fenced_repaint" or wrote != 7 or proof_payload.get("blank") is not True:
        print("VERDICT RED CONFIRMED: the valid blank did NOT ack (mode=%s wrote=%s blank=%s) — the blank filter made the s21 ruling unreachable and the retained marker flushed over the authoritative blank (samples=%s final=%d)" % (
            seed_mode, wrote, proof_payload.get("blank"), samples, page_count))
        fail_rc = fail_rc or 12
    elif not proof_payload.get("receipt_qualifies"):
        print("VERDICT FAIL: blank acked but the receipt did not qualify — rig shape broke")
        fail_rc = fail_rc or 6
    elif page_count > 0 or max(samples) > 0:
        print("VERDICT PAGE FAIL: erased content resurrected over the acked blank (samples=%s final=%d)" % (samples, page_count))
        fail_rc = fail_rc or 12
    elif dropped_batches < 1:
        print("VERDICT FAIL: blank acked but no covered drop traced — the marker batch's fate is unaccounted")
        fail_rc = fail_rc or 6
    else:
        print("   drop evidence: %s" % json.dumps({k: drop_events[-1][1].get(k) for k in ("dropped_batches", "dropped_bytes", "seed_output_seq")}))
    if fail_rc:
        print("RIG FAIL rc=%d — verdicts above" % fail_rc)
        raise SystemExit(fail_rc)
    print("RIG PASS — the valid blank acked as an empty repaint and coverage committed: erased content stayed erased")
    raise SystemExit(0)

if MODE == "loss":
    # (e-r1)-R1 BAR 5 — THE ACK-GATED COVERAGE BAR (sitting 21): acks are
    # ZERO by construction (forced skip before the first seed write). RED =
    # covered batches DESTROYED with no ack (drop trace; the page gets the
    # marker only via the reconcile heal, if at all). GREEN = no drop, the
    # explicit unapplied failure traced, everything flushed LIVE (the
    # marker delivered by the fence's own write path, first sample).
    proof_payload = (session_events(read_events(), s, "synthesized_mount_open") or [(0, {})])[-1][1]
    if proof_payload.get("seed_mode") != "test_forced_skip" or int(proof_payload.get("wrote_seed") or 0) != 0:
        print("PRE-FLIGHT FAIL: forced-skip hook did not carry (seed_mode=%s wrote_seed=%s)" % (
            proof_payload.get("seed_mode"), proof_payload.get("wrote_seed")))
        raise SystemExit(4)
    samples = []
    for delay in (0.15, 0.5, 1.0):
        time.sleep(delay)
        c, _ = page_screen_count(s, "E1RMARK")
        samples.append(c)
    time.sleep(4)
    page_count, page_tail = page_screen_count(s, "E1RMARK")
    daemon_count = daemon_screen_count(s, "E1RMARK")
    assert daemon_count == 1, "hook injected %d markers (want exactly 1)" % daemon_count
    events = read_events()
    drop_events = (session_events(events, s, "synth_output_fence_seed_covered_drop")
                   + session_events(events, s, "synth_output_fence_poll_filtered"))
    flush_events = session_events(events, s, "synth_output_fence_flushed")
    retained_events = session_events(events, s, "synth_output_fence_retained")
    unapplied_events = session_events(events, s, "synth_output_fence_seed_unapplied")
    retained_total = sum(int(p.get("bytes") or 0) for _, p in retained_events)
    flushed_total = sum(int(p.get("bytes") or 0) for _, p in flush_events)
    dropped_batches = sum(int(p.get("dropped_batches") or 0) for _, p in drop_events)
    print("VERDICT FENCE-LOSS: page E1RMARK=%d (daemon=%d) samples=%s" % (page_count, daemon_count, samples))
    print("   fence: retained=%dB/%d flush=%dB/%d drop_traces=%d dropped_batches=%d unapplied=%d" % (
        retained_total, len(retained_events), flushed_total, len(flush_events), len(drop_events), dropped_batches, len(unapplied_events)))
    print("   page tail: %r" % page_tail[-160:])
    fail_rc = 0
    if dropped_batches >= 1:
        print("VERDICT RED CONFIRMED: seed-covered batches (%d) DESTROYED with ZERO application acks — the (e-r1) DATA-LOSS shape (page marker only via the reconcile heal, if at all: samples=%s final=%d)" % (dropped_batches, samples, page_count))
        fail_rc = fail_rc or 10
    elif not unapplied_events:
        print("VERDICT FAIL: no drop and no unapplied trace either — rig shape broke (a fixed build must trace the explicit failure)")
        fail_rc = fail_rc or 6
    elif not flush_events or flushed_total < 9:
        print("VERDICT FAIL: unapplied traced but nothing flushed live (flush=%dB retained=%dB)" % (flushed_total, retained_total))
        fail_rc = fail_rc or 6
    elif samples[0] < 1 or page_count < 1:
        print("VERDICT PAGE FAIL: marker not delivered in the transient window (samples=%s final=%d want first-sample >=1)" % (samples, page_count))
        fail_rc = fail_rc or 6
    else:
        print("   unapplied evidence: %s" % json.dumps({k: unapplied_events[-1][1].get(k) for k in ("reason", "wrote_seed", "pending_seed_seq")}))
    if fail_rc:
        print("RIG FAIL rc=%d — verdicts above" % fail_rc)
        raise SystemExit(fail_rc)
    print("RIG PASS — no ack, no drop: the unapplied seed preserved and delivered every retained byte live")
    raise SystemExit(0)

if MODE == "flushshed":
    # (e-r1)-R2 BAR 6 — THE DELIVERY-HONESTY BAR (sitting 22, sol Q1
    # remainder): the flush TRANSPORT is shed by the choke hook
    # (YGGTERM_TEST_FLUSH_EVAL_DROP — the deterministic missing-host /
    # dead-bridge shape) while the fence holds the marker batch. The
    # bytes CANNOT reach the page through the write path; the bar is
    # whether the build TELLS THE TRUTH about that. RED (fire-and-forget
    # flush): the flushed event claims batches/bytes and NOTHING names
    # the loss — a silent data-loss surface (exit 11). GREEN
    # (ack-carrying flush): synth_output_fence_flush_undelivered names
    # every undelivered batch/byte and no flush event claims
    # delivery_acked:true.
    proof_payload = (session_events(read_events(), s, "synthesized_mount_open") or [(0, {})])[-1][1]
    if proof_payload.get("seed_mode") != "test_forced_skip" or int(proof_payload.get("wrote_seed") or 0) != 0:
        print("PRE-FLIGHT FAIL: forced-skip hook did not carry (seed_mode=%s wrote_seed=%s)" % (
            proof_payload.get("seed_mode"), proof_payload.get("wrote_seed")))
        raise SystemExit(4)
    samples = []
    for delay in (0.15, 0.5, 1.0):
        time.sleep(delay)
        c, _ = page_screen_count(s, "E1RMARK")
        samples.append(c)
    daemon_count = daemon_screen_count(s, "E1RMARK")
    assert daemon_count == 1, "hook injected %d markers (want exactly 1)" % daemon_count
    events = read_events()
    flush_events = session_events(events, s, "synth_output_fence_flushed")
    undelivered_events = session_events(events, s, "synth_output_fence_flush_undelivered")
    unapplied_events = session_events(events, s, "synth_output_fence_seed_unapplied")
    flushed_total = sum(int(p.get("bytes") or 0) for _, p in flush_events)
    flushed_acked = sum(int(p.get("acked_bytes") or 0) for _, p in flush_events)
    undelivered_total = sum(int(p.get("bytes") or 0) for _, p in undelivered_events)
    false_ack = [p for _, p in flush_events if p.get("delivery_acked") is True]
    print("VERDICT FENCE-FLUSHSHED: samples=%s daemon=%d (marker reaches the page only via the reconcile heal, if at all)" % (samples, daemon_count))
    print("   flush=%dB/%d acked=%dB undelivered=%dB/%d unapplied=%d false_ack=%d" % (
        flushed_total, len(flush_events), flushed_acked, undelivered_total, len(undelivered_events), len(unapplied_events), len(false_ack)))
    fail_rc = 0
    if not unapplied_events:
        print("VERDICT FAIL: unapplied trace missing — rig shape broke (a shed transport must still trace the explicit failure)")
        fail_rc = fail_rc or 6
    elif not flush_events or flushed_total < 9:
        print("VERDICT FAIL: the fence never flushed through the choke (flush=%dB)" % flushed_total)
        fail_rc = fail_rc or 6
    elif not undelivered_events or undelivered_total < 9:
        print("VERDICT RED CONFIRMED: the flushed transport was SHED (%dB claimed flushed, %d acked) and NOTHING names the loss — the fire-and-forget flush consumes retained bytes silently (the s21 delivery_acked:false was the honest stopgap, not a delivery proof)" % (flushed_total, flushed_acked))
        fail_rc = fail_rc or 11
    elif false_ack:
        print("VERDICT FAIL: a flush event claims delivery_acked:true while the transport was shed")
        fail_rc = fail_rc or 6
    else:
        print("   undelivered evidence: %s" % json.dumps({k: undelivered_events[-1][1].get(k) for k in ("batches", "bytes", "first_reason")}))
    if fail_rc:
        print("RIG FAIL rc=%d — verdicts above" % fail_rc)
        raise SystemExit(fail_rc)
    print("RIG PASS — the shed flush is NAMED: every undelivered batch/byte traced, no false delivery claim")
    raise SystemExit(0)

if MODE == "lateflush":
    # (e-r1)-R2 BAR 7 — THE LATE-MUTATION BAR (sol Q3, round 3): the
    # flush evals EXECUTE past the caller's 3s ack timeout (the
    # delayed-exec hook YGGTERM_TEST_FLUSH_EXEC_DELAY_MS=5000) while the
    # frame-hash/reveal reconcile repaint — which already contains the
    # retained bytes — lands first. The caller declared the batches
    # undelivered at the timeout; the surviving scripts are STALE
    # MUTATIONS. RED (no supersession guard): the late script appends
    # the retained bytes over the repainted screen (page marker count
    # DOUBLES — exit 12, the late-mutation duplication). GREEN (the
    # supersession epoch rejects the stale write page-side): the page
    # holds exactly the repaint's one copy through the late window.
    proof_payload = (session_events(read_events(), s, "synthesized_mount_open") or [(0, {})])[-1][1]
    if proof_payload.get("seed_mode") != "test_forced_skip" or int(proof_payload.get("wrote_seed") or 0) != 0:
        print("PRE-FLIGHT FAIL: forced-skip hook did not carry (seed_mode=%s wrote_seed=%s)" % (
            proof_payload.get("seed_mode"), proof_payload.get("wrote_seed")))
        raise SystemExit(4)
    # wait for the reconcile repaint to land (the recovery path the
    # undelivered bytes ride — the daemon screen contains the marker)
    deadline = time.time() + 12
    repainted = 0
    while time.time() < deadline:
        c, _ = page_screen_count(s, "E1RMARK")
        if c >= 1:
            repainted = c
            break
        time.sleep(0.3)
    print("repaint landed: page=%d" % repainted)
    if repainted < 1:
        print("PRE-FLIGHT FAIL: the reconcile repaint never landed inside 12s — the recovery path did not fire")
        raise SystemExit(4)
    # the late flush would execute at flush-start+5000ms (~proof+5s; the
    # repaint lands ~proof+2s). ⛔ MASKED-AT-REST LAW (sitting 18): a
    # duplication re-heals within ~2s — sampling AFTER the re-heal is a
    # vacuous pass. Sample DENSELY inside the post-late-write window
    # (repaint+3.2s ≈ the late write + margin, then every 300ms) and
    # count the reconcile-applied events too: a second applied event =
    # the late write corrupted and healed = still a late mutation.
    time.sleep(3.2)
    counts = []
    for _ in range(9):
        c, _ = page_screen_count(s, "E1RMARK")
        counts.append(c)
        time.sleep(0.3)
    daemon_count = daemon_screen_count(s, "E1RMARK")
    assert daemon_count == 1, "hook injected %d markers (want exactly 1)" % daemon_count
    events = read_events()
    flush_events = session_events(events, s, "synth_output_fence_flushed")
    undelivered_events = session_events(events, s, "synth_output_fence_flush_undelivered")
    unapplied_events = session_events(events, s, "synth_output_fence_seed_unapplied")
    reconcile_applied = len(
        [1 for _, name, p in events
         if name == "frame_hash_reconcile_applied" and p.get("session_path") == s])
    flushed_total = sum(int(p.get("bytes") or 0) for _, p in flush_events)
    undelivered_total = sum(int(p.get("bytes") or 0) for _, p in undelivered_events)
    false_ack = [p for _, p in flush_events if p.get("delivery_acked") is True]
    first_reason = undelivered_events[-1][1].get("first_reason") if undelivered_events else None
    print("VERDICT FENCE-LATEFLUSH: repaint=%d late-window counts=%s daemon=%d reconcile_applied=%d first_reason=%s" % (
        repainted, counts, daemon_count, reconcile_applied, first_reason))
    print("   flush=%dB/%d undelivered=%dB/%d unapplied=%d false_ack=%d" % (
        flushed_total, len(flush_events), undelivered_total, len(undelivered_events), len(unapplied_events), len(false_ack)))
    fail_rc = 0
    if not unapplied_events:
        print("VERDICT FAIL: unapplied trace missing — rig shape broke")
        fail_rc = fail_rc or 6
    elif not undelivered_events or undelivered_total < 9 or first_reason != "ack_timeout":
        print("VERDICT FAIL: the timed-out flush never reported undelivered with ack_timeout (undelivered=%dB first_reason=%s)" % (undelivered_total, first_reason))
        fail_rc = fail_rc or 6
    elif false_ack:
        print("VERDICT FAIL: a flush event claims delivery_acked:true while every await timed out")
        fail_rc = fail_rc or 6
    elif repainted != 1 or any(c != 1 for c in counts) or reconcile_applied != 1:
        # [sol r5] EXACT-COUNT bar: disappearance (0), duplication (>1),
        # and extra heals are ALL failures — max()>1 accepted a vanished
        # marker.
        print("VERDICT RED CONFIRMED: the late window is not exactly the repaint's one copy — counts=%s repaint=%d reconcile_applied=%d (disappearance, duplication, or an extra heal; sol Q3's late-mutation family)" % (counts, repainted, reconcile_applied))
        fail_rc = fail_rc or 12
    else:
        print("   honest accounting: undelivered %dB named; the page holds exactly the repaint's copy" % undelivered_total)
    if fail_rc:
        print("RIG FAIL rc=%d — verdicts above" % fail_rc)
        raise SystemExit(fail_rc)
    print("RIG PASS — no late mutation observed: page exactly the repaint's one copy at every dense sample, the timeout reported undelivered")
    raise SystemExit(0)

if MODE == "latecontrol":
    # (e-r1)-R2 BAR 8 — THE POSITIVE CONTROL (sol r4, reshaped by
    # measurement): the caller times out at the DEFAULT 3s and DROPS
    # the future — the loop resumes, the reconcile repaints the marker
    # (~+4s) — and the guard-OFF delayed continuation writes at ~+5s,
    # OVER the repaint. PASS = the duplication observed (page count >1
    # in the dense post-late-write window OR a second reconcile heal):
    # the post-drop continuation mechanism is live, and the lateflush
    # bar's safety is the GUARD's, not drop-abort folklore. (A RETAINED
    # 10s future was measured NOT to construct the hazard: the inline
    # flush branch holds the loop, no repaint precedes the late write,
    # and it delivers as the sole writer — acked 204B, count 1.)
    proof_payload = (session_events(read_events(), s, "synthesized_mount_open") or [(0, {})])[-1][1]
    if proof_payload.get("seed_mode") != "test_forced_skip" or int(proof_payload.get("wrote_seed") or 0) != 0:
        print("PRE-FLIGHT FAIL: forced-skip hook did not carry (seed_mode=%s wrote_seed=%s)" % (
            proof_payload.get("seed_mode"), proof_payload.get("wrote_seed")))
        raise SystemExit(4)
    deadline = time.time() + 12
    repainted = 0
    while time.time() < deadline:
        c, _ = page_screen_count(s, "E1RMARK")
        if c >= 1:
            repainted = c
            break
        time.sleep(0.3)
    print("repaint landed: page=%d" % repainted)
    if repainted < 1:
        print("PRE-FLIGHT FAIL: the reconcile repaint never landed inside 12s")
        raise SystemExit(4)
    # the guard-off continuation writes at ~flush+5s (the repaint lands
    # ~+4s, after the 3s timeout frees the loop); the re-heal follows
    # ~2s later — dense sampling INSIDE the window (masked-at-rest law)
    time.sleep(1.0)
    counts = []
    for _ in range(10):
        c, _ = page_screen_count(s, "E1RMARK")
        counts.append(c)
        time.sleep(0.3)
    daemon_count = daemon_screen_count(s, "E1RMARK")
    assert daemon_count == 1, "hook injected %d markers (want exactly 1)" % daemon_count
    events = read_events()
    flush_events = session_events(events, s, "synth_output_fence_flushed")
    undelivered_events = session_events(events, s, "synth_output_fence_flush_undelivered")
    undelivered_total = sum(int(p.get("bytes") or 0) for _, p in undelivered_events)
    reconcile_applied = len(
        [1 for _, name, p in events
         if name == "frame_hash_reconcile_applied" and p.get("session_path") == s])
    print("VERDICT FENCE-LATECONTROL: repaint=%d dense counts=%s reconcile_applied=%d undelivered=%dB" % (
        repainted, counts, reconcile_applied, undelivered_total))
    if max(counts) > 1 or repainted > 1 or reconcile_applied > 1:
        print("RIG PASS — the post-drop continuation DEMONSTRABLY writes over the repaint (duplication observed with the guard OFF): the mechanism is live, the lateflush bar's safety is the GUARD's")
        raise SystemExit(0)
    if undelivered_total < 9:
        print("VERDICT FAIL: no duplication AND no undelivered trace — the flush shape broke entirely")
        raise SystemExit(6)
    # [sol r5] validity before classifying suppression: the recovery must
    # be on record (exactly one repaint, a heal applied) and every dense
    # sample exactly 1 — otherwise this is a broken shape, not evidence.
    if repainted != 1 or reconcile_applied < 1 or any(c != 1 for c in counts):
        print("VERDICT FAIL: cannot classify — recovery not on record or the window is not exactly one copy (repaint=%d reconcile_applied=%d counts=%s)" % (repainted, reconcile_applied, counts))
        raise SystemExit(6)
    # MEASUREMENT ARM (not a landing gate): paired with the retained-
    # future run (delayed write fired, acked, sole-writer), this is the
    # drop-suppression pair — the post-drop continuation did NOT write
    # (measured 2026-10-08, two shapes). The supersession guard stays
    # as defense-in-depth regardless (no source-level cancellation
    # guarantee; a bridge upgrade that starts executing dropped scripts
    # turns this arm's exit 5 into exit 0 — and lateflush then guards).
    print("CONTROL MEASUREMENT: the flush timed out undelivered (%dB), the repaint holds exactly one copy, and the guard-OFF post-drop continuation did NOT write — drop-suppression observed on this bridge (exit 5 = suppressed, 0 = duplication proven)" % undelivered_total)
    raise SystemExit(5)

# (e-r1) TRANSIENT SAMPLING: the frame-hash and reveal reconciles heal
# a duplicated frame within ~2s of the proof (measured RED run 4: flush
# at proof+1ms, reveal_screen_reconcile at proof+2.0s) — the defect's
# user-visible surface is the TRANSIENT double-paint between the flush
# and that heal, so the page is sampled INSIDE the window.
dup_transient_max = 0
if MODE == "dup":
    for delay in (0.15, 0.5, 1.0):
        time.sleep(delay)
        c, _ = page_screen_count(s, "E1RMARK")
        dup_transient_max = max(dup_transient_max, c)
        raw = verb("server", "app", "terminal", "read-buffer", s, "--mode", "screen").stdout or ""
        rows = [r for r in raw.splitlines() if "E1RMARK" in r or "text" in r or "nonblank" in r or "char_count" in r]
        print("   transient sample +%0.2fs: page=%d %s" % (delay, c, rows[:2]))

time.sleep(4)  # settle: the proof arm completes; the fence flush (if any) rides it

if MODE == "dup":
    # (e-r1) BAR 4 — THE DELIVERY BAR. The at-rest page cannot show the
    # duplication in this rig shape: the flushed covered bytes re-render
    # identically over the seed (measured: char_count constant through
    # the transient window; the order-mode control on the same build
    # proves the flush mechanism itself lands), and a DIFFERING frame is
    # healed by the frame-hash/reveal reconciles within ~2s. The honest
    # bars: RED = the fence DELIVERED seed-covered bytes to the viewport
    # after the seed (retained inside the window + flushed over it);
    # GREEN = the stamp DROPPED them before delivery (drop trace fired,
    # page stays exactly the seed's one copy through every transient
    # sample).
    page_count, page_tail = page_screen_count(s, "E1RMARK")
    daemon_count = daemon_screen_count(s, "E1RMARK")
    assert daemon_count == 1, "hook injected %d markers (want exactly 1)" % daemon_count
    events = read_events()
    proof_payload = (session_events(events, s, "synthesized_mount_open") or [(0, {})])[-1][1]
    drop_events = (session_events(events, s, "synth_output_fence_seed_covered_drop")
                   + session_events(events, s, "synth_output_fence_poll_filtered"))
    retained_events = session_events(events, s, "synth_output_fence_retained")
    flush_events = session_events(events, s, "synth_output_fence_flushed")
    retained_total = sum(int(p.get("bytes") or 0) for _, p in retained_events)
    flushed_total = sum(int(p.get("bytes") or 0) for _, p in flush_events)
    print("VERDICT FENCE-DUPLICATION: transient_max=%d final page E1RMARK=%d (daemon=%d) seed_mode=%s drop_traces=%d" % (
        dup_transient_max, page_count, daemon_count, proof_payload.get("seed_mode"), len(drop_events)))
    print("   fence: retained=%dB/%d flush=%dB/%d" % (retained_total, len(retained_events), flushed_total, len(flush_events)))
    print("   page tail: %r" % page_tail[-160:])
    fail_rc = 0
    if page_count < 1:
        print("PRE-FLIGHT FAIL: the seed never painted the marker (page=%d, want 1)" % page_count)
        fail_rc = 4
    elif not drop_events:
        if flush_events and flushed_total >= 9:
            print("VERDICT RED CONFIRMED: seed-covered bytes (%d of %d retained) were FLUSHED over the seeded screen — the (e-r1) duplication DELIVERED (unfixed build; masked at rest by identical re-render + the reconcile heal, both measured this sitting)" % (flushed_total, retained_total))
            fail_rc = fail_rc or 9
        else:
            print("VERDICT FAIL: no drop trace and no covered flush either — rig shape broke (retained=%d flushed=%d)" % (retained_total, flushed_total))
            fail_rc = fail_rc or 6
    else:
        dropped_batches = sum(int(p.get("dropped_batches") or 0) for _, p in drop_events)
        if dropped_batches < 1:
            print("VERDICT DROP-EVIDENCE FAIL: drop traces fired but dropped zero batches")
            fail_rc = fail_rc or 6
        elif page_count != 1 or dup_transient_max > 1:
            print("VERDICT PAGE FAIL: page=%d transient_max=%d (want exactly 1 and 1)" % (page_count, dup_transient_max))
            fail_rc = fail_rc or 6
        else:
            print("   drop evidence: %s" % json.dumps({k: drop_events[-1][1].get(k) for k in ("dropped_batches", "dropped_bytes", "seed_output_seq")}))
    if fail_rc:
        print("RIG FAIL rc=%d — verdicts above" % fail_rc)
        raise SystemExit(fail_rc)
    print("RIG PASS — the seed stamp dropped the covered differential before delivery (marker exactly the seed's one copy)")
    raise SystemExit(0)

if MODE == "negative":
    # [Q7-R6] BAR 8 — THE NEGATIVE CONTROL (sitting 25, sol Q6/R6): an
    # inherited, visibly-painted surface with ZERO current-generation
    # application evidence must promote NOTHING. Phase A (this boot):
    # receipts WITHHELD + seed forced to skip — on the (e-r3) build the arm
    # traces, no promotion fires, and the DOM first-paint records PENDING;
    # on unfixed main there is no arm trace and the unqualified fast-ready
    # fires (exit 13, THE RED). Phases C/D run as further boots in bash.
    events = read_events()
    armed = [p for _, nm, p in events
             if nm == "applied_content_armed" and p.get("session_path") == s]
    promoted_true = [p for _, nm, p in events
                     if nm == "applied_content_promoted" and p.get("session_path") == s
                     and p.get("promoted") is True]
    promoted_any = [p for _, nm, p in events
                    if nm == "applied_content_promoted" and p.get("session_path") == s]
    ready_events = [p for ln, nm, p in events if nm == "ready" and p.get("session_path") == s]
    # [Q7-R6 s26] the CONTENT-reason readies are the LATCH leak detector: a
    # CONTENT claim completing under withholding means content-ready was
    # minted with zero application evidence (the sol R2 indictment, direct
    # form). DECISION completions stay legal.
    CONTENT_REASONS = {
        "active_recovery_snapshot_replay", "visual_reveal",
        "retained_transcript_browser", "retained_non_prompt_snapshot_replay",
        "blank_host_snapshot_replay", "live_transcript_browser",
        "fresh_remote_codex_start",
    }
    def ready_reason(p):
        return p.get("reason") or (p.get("extra") or {}).get("reason") or ""
    content_readies = [p for p in ready_events if ready_reason(p) in CONTENT_REASONS]
    paint_events = [p for _, nm, p in events
                    if nm == "paint_ready" and p.get("session_path") == s]
    paint_pending_seen = any(p.get("paint_pending") is True for p in paint_events)
    print("VERDICT NEGATIVE-A: armed=%d promoted_any=%d promoted_true=%d ready=%d content_readies=%d paint_pending=%d" % (
        len(armed), len(promoted_any), len(promoted_true), len(ready_events), len(content_readies), 1 if paint_pending_seen else 0))
    if not armed:
        # unfixed build: the machinery is absent — the RED discriminator is
        # the UNQUALIFIED promotion (a ready attempt with no evidence).
        if ready_events:
            print("VERDICT RED CONFIRMED: no application machinery on this build, yet a Ready attempt completed with ZERO application evidence (ready=%d) — liveness fabricated content-ready (the sol R2 indictment)" % len(ready_events))
            raise SystemExit(13)
        print("RIG SHAPE BROKE: no arm trace and no ready event — the negative phase is vacuous on this build")
        raise SystemExit(4)
    # the (e-r3) build: withheld means NOTHING content-promotes. DECISION
    # completions (warm_alive_posted_ready etc.) are LEGAL here — the
    # discriminators are the CONTENT promotion (promoted_true) AND the
    # CONTENT-reason completion (content_readies) both staying zero.
    # (The paint-pending bar lives in PHASE D: synthesized mounts have a
    # dead eval event channel — TerminalJsEvent::Paint cannot arrive on
    # this shape, so first-paint is unobservable here by construction.)
    if promoted_true:
        print("VERDICT FAIL: a promotion fired while receipts were WITHHELD — the guard leaks")
        raise SystemExit(13)
    if content_readies:
        print("VERDICT FAIL: a CONTENT-reason Ready completed while receipts were WITHHELD (%d) — the guarded transaction leaked content-ready with zero evidence" % len(content_readies))
        raise SystemExit(13)
    if promoted_any:
        print("   (promotion events with promoted=false are honest rejections: %d)" % len(promoted_any))
    print("PHASE A PASS — zero application evidence, zero content promotions, zero content completions (decision completes are legal)")
    # the phase-1 driver ENDS here for negative mode: phases C+D are their
    # own boots in bash below (the order-mode fallthrough is not this bar).
    raise SystemExit(0)

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

# ── [Q7-R6] NEGATIVE PHASES C+D: the controlled release + the foreign
# tuple (only for MODE=negative; each is its own healthy boot) ───────────
if [ "$MODE" = negative ] && [ "$RC" = 0 ]; then
  # PHASE C — THE RELEASE: a healthy boot, no hooks. Real live output →
  # the first-live-write receipt → promotion. The latch/content-ready
  # completes with REAL evidence (not a synthetic stamp).
  CSCRATCH=/tmp/f1e2-release-$(date +%s)
  run_boot "$CSCRATCH" 0
  CGUI=$!
  CREADY=$(boot_wait_ready); echo "release daemon ready=$READY"; sleep 8
  CSCRATCH=$CSCRATCH python3 - <<'PYEOF' > /tmp/f1e2-release-run.log 2>&1
import json, subprocess, time, os
bin_path = os.environ["BIN"]
TRACE = os.environ["CSCRATCH"] + "/event-trace.jsonl"
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
                    or (event.get("payload") or {}).get("name"))
            if name:
                out.append((name, event.get("payload") or {}))
    except FileNotFoundError:
        pass
    return out
def make_row(title):
    for attempt in range(4):
        out = verb("server", "app", "terminal", "new", "--kind", "shell", "--title", title)
        try:
            return (json.loads(out.stdout).get("data") or {}).get("session_path")
        except Exception:
            print("row create attempt %d failed: %s" % (attempt, (out.stdout or out.stderr)[:120].replace("\n", " ")))
            time.sleep(4)
    return None
# ⛔ ROW SHAPE (the s25 phase-C failure, re-measured 2026-10-09): a SINGLE
# row created during healthy-boot churn times out the 15s app-control wait,
# the row LANDS anyway (the sitting-12 race), and the GUI mounts the ORPHAN
# — the driver polls a row that never mounts. The proven shape: a dummy
# first row absorbs the churn, then the test row, then a FORCED real switch
# (server app open --view terminal — the measured switch verb, the focus
# verb never switches) so the polled row IS the mounted row.
_a = make_row("release-dummy")
s = make_row("release")
if not s:
    print("RELEASE FAIL: no row"); raise SystemExit(14)
print("rows: a=%s s=%s" % (_a, s))
verb("server", "app", "open", s, "--view", "terminal")
verb("server", "app", "terminal", "probe-type", s, "--data", "echo E1RGOOD", "--enter", "--mode", "xterm")
deadline = time.time() + 25
ok = False
CONTENT_REASONS = {
    "active_recovery_snapshot_replay", "visual_reveal",
    "retained_transcript_browser", "retained_non_prompt_snapshot_replay",
    "blank_host_snapshot_replay", "live_transcript_browser",
    "fresh_remote_codex_start",
}
def ready_reason(p):
    return p.get("reason") or (p.get("extra") or {}).get("reason") or ""
while time.time() < deadline:
    ev = read_events()
    # [Q7-R6 s26] the release bar is ORDER- AND SHAPE-INDEPENDENT: a
    # promoted event with the ARMED epoch proves the receipt was minted,
    # crossed the bridge, validated against the tuple, and was STORED —
    # whether anything was parked to promote (promoted=true, ack-arrival
    # order) or not (promoted=false, direct-complete/no-claim shape) is
    # claim-ordering, not evidence. Verb-created local rows fire only
    # DECISION claims (reparent/warm-alive) — content-claim completion is
    # remote-resume territory (suite-locked; the LiveSsh gate keeps the
    # rig out). Foreign epochs (the poison) never match.
    promoted_valid = [p for nm, p in ev if nm == "applied_content_promoted" and p.get("session_path") == s and p.get("epoch") == 1]
    armed = [p for nm, p in ev if nm == "applied_content_armed" and p.get("session_path") == s]
    body = verb("server", "app", "terminal", "read-buffer", s, "--mode", "screen").stdout or ""
    if armed and promoted_valid and body.count("E1RGOOD") >= 1:
        ok = True
        break
    time.sleep(0.5)
print("VERDICT RELEASE: armed=%s receipt_recorded=%d painted=%s" % (
    bool(armed), len(promoted_valid), body.count("E1RGOOD") >= 1))
if not ok:
    print("RELEASE FAIL — the controlled release did not complete content-ready with real evidence")
    raise SystemExit(14)
print("PHASE C PASS — a real qualified write released content-ready")
PYEOF
  RC=$?
  kill "$CGUI" 2>/dev/null; sleep 1
  pkill -x yggterm 2>/dev/null; pkill -f "dbus-run-session" 2>/dev/null; sleep 1
  for p in $(pgrep -f 'yggterm-headless server daemon'); do
    h=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | sed -n 's/^YGGTERM_HOME=//p')
    [ "$h" = "$CSCRATCH" ] && kill "$p" 2>/dev/null
  done
  tail -4 /tmp/f1e2-release-run.log
fi
if [ "$MODE" = negative ] && [ "$RC" = 0 ]; then
  # PHASE D — THE FOREIGN TUPLE: the receipt arrives with epoch+1000; the
  # validation must reject it (promoted=false, nothing promotes).
  # s26 MEASUREMENT: the first cut booted this phase through a RAW
  # dbus-run-session (not run_boot) and the page went daemon_declare_absent
  # with zero receipt events — boot-shape noise, not the guard. This boot is
  # now a CLEAN A/B of phase C: the same run_boot path, differing ONLY by
  # the FOREIGN env (inherited through dbus-run-session).
  DSCRATCH=/tmp/f1e2-foreign-$(date +%s)
  mkdir -p "$DSCRATCH"
  YGGTERM_TEST_APPLIED_CONTENT_FOREIGN=1 run_boot "$DSCRATCH" 0
  DGUI=$!
  DREADY=$(boot_wait_ready); echo "foreign daemon ready=$DREADY"; sleep 8
  DSCRATCH=$DSCRATCH python3 - <<'PYEOF' > /tmp/f1e2-foreign-run.log 2>&1
import json, subprocess, time, os
bin_path = os.environ["BIN"]
TRACE = os.environ["DSCRATCH"] + "/event-trace.jsonl"
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
            name = (event.get("name") or event.get("event"))
            if name:
                out.append((name, event.get("payload") or {}))
    except FileNotFoundError:
        pass
    return out
def make_row(title):
    for attempt in range(4):
        out = verb("server", "app", "terminal", "new", "--kind", "shell", "--title", title)
        try:
            return (json.loads(out.stdout).get("data") or {}).get("session_path")
        except Exception:
            print("row create attempt %d failed: %s" % (attempt, (out.stdout or out.stderr)[:120].replace("\n", " ")))
            time.sleep(4)
    return None
# ⛔ ROW SHAPE (same as phase C): dummy row absorbs the boot churn, the
# test row is force-switched so the polled row IS the mounted row.
_a = make_row("foreign-dummy")
s = make_row("foreign")
if not s:
    print("FOREIGN FAIL: no row"); raise SystemExit(15)
print("rows: a=%s s=%s" % (_a, s))
verb("server", "app", "open", s, "--view", "terminal")
verb("server", "app", "terminal", "probe-type", s, "--data", "echo E1RBAD", "--enter", "--mode", "xterm")
deadline = time.time() + 25
saw_reject = False
while time.time() < deadline:
    ev = read_events()
    # [Q7-R6 s26] the FOREIGN hook poisons the receipt AT THE RECORD
    # BOUNDARY (Rust-side, epoch+1000 — deterministic through the always-
    # firing seed path; the page-side live-path poison stays as a second
    # arm). The bars: the poisoned receipt is OBSERVED rejected (>=1
    # promoted event with a foreign epoch) and NOTHING ever promotes
    # (zero promoted=true from any source — every receipt this boot is
    # foreign).
    promoted = [p for nm, p in ev if nm == "applied_content_promoted" and p.get("session_path") == s]
    if any(p.get("epoch") != 1 for p in promoted):
        saw_reject = True
        break
    time.sleep(0.5)
# [Q7-R4] the DOM first-paint witness can LAG the rejection (the paint
# event rides its own channel) — give it its own window before judging.
paint_deadline = time.time() + 12
paint_events = []
all_ev = []
while time.time() < paint_deadline:
    all_ev = read_events()
    paint_events = [p for nm, p in all_ev
                    if nm == "paint_ready" and p.get("session_path") == s]
    if paint_events:
        break
    time.sleep(0.5)
if not all_ev:
    all_ev = read_events()
any_true = [p for nm, p in all_ev
            if nm == "applied_content_promoted" and p.get("session_path") == s
            and p.get("promoted") is True]
foreign_rejects = [p for nm, p in all_ev
                   if nm == "applied_content_promoted" and p.get("session_path") == s
                   and p.get("epoch") != 1]
paint_events = [p for nm, p in all_ev
                if nm == "paint_ready" and p.get("session_path") == s]
paint_pending_seen = any(p.get("paint_pending") is True for p in paint_events)
print("VERDICT FOREIGN: rejected=%d promoted_true=%d paint_events=%d paint_pending=%d" % (
    len(foreign_rejects), len(any_true), len(paint_events), 1 if paint_pending_seen else 0))
if any_true or not saw_reject:
    print("FOREIGN FAIL — the foreign tuple was not rejected (rejects=%d, promoted_true=%d)" % (len(foreign_rejects), len(any_true)))
    raise SystemExit(15)
# [Q7-R4] the DOM first-paint on a HEALTHY mount (Paint events flow here):
# painted structure with a REJECTED tuple must leave the paint witness
# PENDING — the paint note never advances without qualified application.
# MEASURED (s26): rows created via the verb go through the SYNTHESIS mount
# path whose eval Paint channel is dead (phase A's construction note) —
# paint_events==0 is the SHAPE, not a leak; the pending lock lives in the
# suite (the_ack_arrival_promotion completes the pending paint). Fail only
# when the witness OBSERVED and said qualified.
if paint_events and not paint_pending_seen:
    print("FOREIGN FAIL — the DOM first-paint recorded paint_qualified (not pending) under a rejected tuple (the R4 guard)")
    raise SystemExit(15)
if not paint_events:
    print("   (no paint events on this synthesized-mount shape — the R4 pending witness is suite-locked, not rig-observable here)")
print("PHASE D PASS — foreign tuple rejected; the painted surface's witness stayed pending")
PYEOF
  RC=$?
  kill "$DGUI" 2>/dev/null; sleep 1
  pkill -x yggterm 2>/dev/null; pkill -f "dbus-run-session" 2>/dev/null; sleep 1
  for p in $(pgrep -f 'yggterm-headless server daemon'); do
    h=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | sed -n 's/^YGGTERM_HOME=//p')
    [ "$h" = "$DSCRATCH" ] && kill "$p" 2>/dev/null
  done
  tail -4 /tmp/f1e2-foreign-run.log
fi

# ── PHASE 2: the healthy control (BAR 3) — only if phase 1 passed ────────
if [ "$RC" = 0 ] && [ "$MODE" != negative ]; then
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
    painted, len(synth_names), (synth_names[:4] if synth_names else "")))
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
