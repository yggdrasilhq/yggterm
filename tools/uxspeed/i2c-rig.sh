#!/bin/bash
# [i-r2] THE CREATION-CHURN SURVIVAL RIG (i2c-rig) — the falsifier for the s15
# BackgroundTrickle guarantee dying with the loop (sol s45 Q4; root-caused s46:
# the for-loop's UNKEYED ITERATION WRAPPER made dioxus diff the retained list
# POSITIONALLY, so a sibling's creation prepend destroyed + recreated the
# demoted row's TerminalCanvas and its render-spawned mount loop).
#   SETUP: hermetic scratch home + Xvfb + dbus (d-rig scaffolding), NO IPC
#          suppression — production shape. HOT_PREMOUNT_CAP=2 as d-rig.
#   SEQUENCE: row a healthy+active; arm late output via a SPLIT marker
#          (printf '%s%s' CHURN LATE_<nonce> — the typed line can never satisfy
#          the contiguous-marker bar, sol s46 Q4); create row b — b's CREATION
#          prepends b to the live order and demotes a inside b's churn window.
#   BAR-D (survival, HARD, FULL slice — sol s47 Q4): a's component
#          instance, loop AND owner survive b's creation THROUGH the
#          re-select and the effect check — zero terminal_mount_task_dropped,
#          zero bootstrap_reset, zero bootstrap_owner_superseded_during_loop
#          (an owner switch = a replacement loop start; samples partition by
#          incarnation at each supersession). spawn_skipped stays
#          INFORMATIONAL (the residual cascade's fingerprint). Row b's
#          legitimate initial bootstrap and a's pre-churn cold-mount
#          refusal are outside this slice.
#   BAR-T (trickle): the SPLIT marker must reach a's client buffer while a is
#          demoted, absent before demotion. NON-DISCRIMINATING as instrumented
#          (a remount re-seeds from the daemon screen too) — BAR-D/BAR-R carry
#          the verdict; kept as the symptom bar.
#   BAR-R (fast raise, HARD — sol s47 Q4): the re-select takes the fast
#          raise branch — the birth attempt EARNED readiness at attach (the
#          in-slice 'ready' attempt event, the L1 fix), reveal_served fires
#          with ZERO reveal_raise_refused. BAR-P below is the fresh-effect
#          proof (a typed marker read back — no stale retained frame can
#          satisfy it); no latency budget is asserted (none is declared).
#   BAR-P (client typing): the SPLIT-typed echo lands in a's client buffer.
#          Delivery-only (no latency claim).
# RC: 0 all green · 2 setup · 4 pre-flight · 5 BAR-T · 6 BAR-D ·
#     7 BAR-R · 8 BAR-P · 9 exe-mismatch voids all verdicts.
# Usage: tools/uxspeed/i2c-rig.sh <binary>  (absolute path; full-workspace
# release build per rig law L1).
set -u
BIN=${1:?usage: i2c-rig.sh <binary>}
SCRATCH=/tmp/i2ch-home-$(date +%s)
mkdir -p "$SCRATCH"
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN — build first"; exit 2; fi
export BIN
pkill -f "Xvfb :79" 2>/dev/null; sleep 1
Xvfb :79 -screen 0 1600x1000x24 -ac > /tmp/i2ch-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5
# ⛔ HERMETIC ([11.231]): XDG_DATA_HOME must point at the scratch or the
# binary re-execs the INSTALLED direct build; asserted via the EXE PROOF below.
export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
dbus-run-session -- bash -c "
  export DISPLAY=:79 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
  export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share YGGTERM_HOT_PREMOUNT_CAP=2
  exec $BIN > /tmp/i2ch-gui.log 2>&1
" >/dev/null 2>&1 </dev/null &
GUI_WRAP=$!
READY=0; for i in $(seq 1 90); do sleep 1; "$BIN" server app clients >/dev/null 2>&1 && READY=1 && break; done
echo "daemon ready=$READY after ${i}s"; sleep 8
python3 - <<'PYEOF' > /tmp/i2ch-run.log 2>&1
import json, subprocess, time, os

bin_path = os.environ["BIN"]
NONCE = str(int(time.time()))[-6:]
def verb(*args, timeout=90):
    return subprocess.run([bin_path] + list(args), capture_output=True, text=True, timeout=timeout)

TRACE = os.environ.get("YGGTERM_HOME", "/tmp/i2ch-home-unknown") + "/event-trace.jsonl"

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

def session_events(events, session, name):
    return [(ln, p) for ln, event_name, p in events
            if event_name == name and p.get("session_path") == session]

def wait_for(predicate, deadline_s, what, poll=0.4):
    deadline = time.time() + deadline_s
    while time.time() < deadline:
        value = predicate(read_events())
        if value:
            return True, value
        time.sleep(poll)
    return False, predicate(read_events())

def client_buffer(session):
    out = verb("server", "app", "terminal", "read-buffer", session)
    raw = out.stdout or ""
    text = None
    try:
        body = json.loads(raw)
        data = body.get("data") if isinstance(body, dict) else None
        if isinstance(data, dict):
            text = data.get("text") or data.get("buffer") or ""
        elif isinstance(body, dict):
            text = body.get("text") or ""
    except Exception:
        pass
    if text is None:
        print("read-buffer %r raw head: %s | %s" % (session, raw[:220], out.stderr[:160]))
        text = ""
    return text

def typed(session, data):
    out = verb("server", "app", "terminal", "probe-type", session, "--data", data, "--enter", "--mode", "xterm")
    try:
        verdict = json.loads(out.stdout).get("data") or {}
    except Exception:
        print("probe-type %r raw: %s %s" % (data, out.stdout[:200], out.stderr[:200]))
        return {}
    print("probe-type -> accepted=%s reason=%s core_trigger=%s term_input=%s" % (
        verdict.get("accepted"), verdict.get("reason"),
        verdict.get("used_core_trigger"), verdict.get("used_term_input")))
    return verdict

def row_session(n):
    out = verb("server", "app", "terminal", "new", "--kind", "shell", "--title", "demo%d" % n)
    try:
        return (json.loads(out.stdout).get("data") or {}).get("session_path")
    except Exception:
        print("new row raw:", out.stdout[:200], out.stderr[:200])
        return None

# SPLIT markers: the typed command text never contains the contiguous marker;
# only the shell's OUTPUT does (printf concatenates at runtime).
M0 = "CHURNZERO_" + NONCE
ML = "CHURNLATE_" + NONCE
M2 = "CHURNRET_" + NONCE
typed_cmd_0 = "printf '%s%s\\n' CHURN ZERO_" + NONCE
typed_cmd_l = "sleep 12; printf '%s%s\\n' CHURN LATE_" + NONCE
typed_cmd_2 = "printf '%s%s\\n' CHURN RET_" + NONCE

fail = 0

# PRE-FLIGHT: row a healthy and active; marker lands in the client buffer.
a = row_session(1)
if not a:
    print("FAIL: no row a"); raise SystemExit(2)
ok, _ = wait_for(lambda ev: session_events(ev, a, "attach_ready"), 40, "a ready")
if not ok:
    print("PRE-FLIGHT FAIL: a never ready (healthy mount)"); raise SystemExit(4)
typed(a, typed_cmd_0)
ok = False
for _ in range(40):
    if M0 in client_buffer(a):
        ok = True; break
    time.sleep(0.5)
print("VERDICT PRE-FLIGHT (a healthy active, split marker %s): %s" % (M0, "GREEN" if ok else "RED"))
if not ok:
    raise SystemExit(4)

# ARM late output on a (SPLIT marker — echo of the typed line cannot pass),
# confirm it is ABSENT, then CREATE b — the churn demotes a.
typed(a, typed_cmd_l)
if ML in client_buffer(a):
    print("PRE-FLIGHT FAIL: late marker already present before the churn"); raise SystemExit(4)
time.sleep(0.5)
slice_start = len(read_events())
b = row_session(2)
if not b:
    print("FAIL: no row b"); raise SystemExit(2)
print("rows: a=%s b=%s (slice starts at event line %d)" % (a, b, slice_start))
ok, _ = wait_for(lambda ev: session_events(ev, b, "attach_ready"), 45, "b ready")
if not ok:
    print("PRE-FLIGHT FAIL: b never ready"); raise SystemExit(4)

# BAR-T: the trickle keeps a's client buffer current while a is demoted.
ok_t = False
for _ in range(60):
    if ML in client_buffer(a):
        ok_t = True; break
    time.sleep(0.5)
print("VERDICT BAR-T (trickle to demoted row, split marker %s): %s" % (ML, "GREEN" if ok_t else "RED"))
if not ok_t:
    fail = 5

# BAR-R (HARD, sol s47 Q4): the fast raise raises. Precondition in-slice:
# the birth attempt EARNED readiness at attach (the L1 'ready' attempt
# event); the re-select takes the raise branch — reveal_served with ZERO
# reveal_raise_refused. BAR-P below is the fresh-effect proof.
post_pre = read_events()[slice_start:]
ready_a = session_events(post_pre, a, "ready")
reselect_start = len(read_events())
verb("server", "app", "open", a, "--view", "terminal"); time.sleep(4)
post2 = read_events()[reselect_start:]
served_a = session_events(post2, a, "reveal_served")
refused_a = session_events(post2, a, "reveal_raise_refused")
print("a birth-attempt 'ready' events in slice: %d" % len(ready_a))
print("a after re-select (slice %d..): served=%d refused=%d" % (reselect_start, len(served_a), len(refused_a)))
for _, p in refused_a[-2:]:
    print("   refused: loop_live=%s was_ever_ready=%s" % (p.get("loop_live"), p.get("was_ever_ready")))
ok_r = len(ready_a) >= 1 and len(served_a) >= 1 and len(refused_a) == 0
print("VERDICT BAR-R (fast raise raises on re-select; HARD): %s" % ("GREEN" if ok_r else "RED"))
if not ok_r:
    fail = fail or 7

# BAR-P (the fresh-effect proof): typing lands in the client buffer after
# the raise — a stale retained frame cannot satisfy a typed-marker read.
typed(a, typed_cmd_2)
ok_p = False
for _ in range(24):
    if M2 in client_buffer(a):
        ok_p = True; break
    time.sleep(0.5)
print("VERDICT BAR-P (typing green after re-select, %s): %s" % (M2, "GREEN" if ok_p else "RED"))
if not ok_p:
    fail = fail or 8

# BAR-D (HARD, FULL slice — sol s47 Q4): zero drops, zero resets, zero
# owner-supersessions for the retained target through re-select + effect +
# settle. spawn_skipped stays INFORMATIONAL.
post = read_events()[slice_start:]
drops_a = [(ln, p) for ln, p in session_events(post, a, "terminal_mount_task_dropped")]
resets_a = session_events(post, a, "bootstrap_reset")
superseded_a = [(ln, p) for ln, p in session_events(post, a, "bootstrap_owner_superseded_during_loop")]
skips_a = session_events(post, a, "bootstrap_spawn_skipped_inactive_retained_host")
print("a full slice (churn->re-select->effect->settle): drops=%d bootstrap_reset=%d superseded=%d spawn_skipped=%d" % (
    len(drops_a), len(resets_a), len(superseded_a), len(skips_a)))
for ln, p in drops_a:
    print("   drop ln=%d: remount_armed=%s exit_hint=%s" % (ln, p.get("remount_armed"), p.get("exit_hint")))
for ln, _ in superseded_a:
    print("   superseded ln=%d (owner switch — a replacement loop started; samples partition by incarnation)" % ln)
ok_d = not drops_a and not resets_a and not superseded_a
print("VERDICT BAR-D (instance+owner survive the churn through the re-select; HARD): %s%s" % (
    "GREEN" if ok_d else "RED",
    "" if ok_d else " (spawn_skipped=%d informational)" % len(skips_a)))
if not ok_d:
    fail = fail or 6

if fail:
    raise SystemExit(fail)
print("RIG: churn-survival all green")
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
if [ "$EXE" = "$BIN" ]; then echo "EXE OK: requested binary served this run"; else echo "EXE MISMATCH: verdicts above are VOID"; RC=9; fi
echo "=== DRIVER OUTPUT ==="
tail -50 /tmp/i2ch-run.log
echo "scratch=$SCRATCH rc=$RC"
exit "$RC"
