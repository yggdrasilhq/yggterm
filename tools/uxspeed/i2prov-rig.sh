#!/bin/bash
# [s47] THE PAINT-PROVENANCE MEASUREMENT RIG (i2prov-rig) — the measurement
# arm of the (i-r2) residual ranked plan (queue record; board plan
# ACK-e081e1775a). NOT a pass/fail falsifier: it ANSWERS the owed question —
# does an organic cold mount of an app-control row ever ATTEMPT a paint send
# (paintSendAttemptCount past the nextPaintKey gate), or does the witness
# starve because the send never fires? paintCount alone proves emitPaint RAN,
# never that the event was attempted (sol's correction).
#   SEQUENCE: row a born via app-control (terminal new — the birth attempt
#   that sits Pending forever, queue defect (a)); settle; arm a SPLIT late
#   marker; create row b — the churn demotes a and the bootstrap-skip cancel
#   converts a's Pending attempt to LATCHED FAILURE (defect (b)); re-select
#   a — the raise veto refuses (defect (c)). The gated probe (YGGTERM_TEST_
#   PAINT_PROVENANCE=1) samples a's page entry every ~250ms THROUGH the
#   whole sequence (read-only; the s46 fix keeps a's loop alive across the
#   churn, so sampling continues).
#   READ (informational, printed in full): the sample series (paint_count,
#   send_attempts, last_paint_key, entry_live, applied_receipt) + the
#   incident events (session_cancelled, reveal_raise_refused,
#   bootstrap_owner_superseded_during_loop).
#   RC: 0 measurement captured (>=3 samples for a — verdicts below are DATA,
#       not a green/red claim) · 2 setup · 4 pre-flight · 9 exe-mismatch.
# Usage: tools/uxspeed/i2prov-rig.sh <binary>  (absolute path; full-workspace
# release build per rig law L1).
set -u
BIN=${1:?usage: i2prov-rig.sh <binary>}
SCRATCH=/tmp/i2prov-home-$(date +%s)
mkdir -p "$SCRATCH"
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN — build first"; exit 2; fi
export BIN
pkill -f "Xvfb :79" 2>/dev/null; sleep 1
Xvfb :79 -screen 0 1600x1000x24 -ac > /tmp/i2prov-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5
# ⛔ HERMETIC ([11.231]): XDG_DATA_HOME must point at the scratch or the
# binary re-execs the INSTALLED direct build; asserted via the EXE PROOF below.
export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
dbus-run-session -- bash -c "
  export DISPLAY=:79 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
  export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share YGGTERM_HOT_PREMOUNT_CAP=2
  export YGGTERM_TEST_PAINT_PROVENANCE=1
  exec $BIN > /tmp/i2prov-gui.log 2>&1
" >/dev/null 2>&1 </dev/null &
GUI_WRAP=$!
READY=0; for i in $(seq 1 90); do sleep 1; "$BIN" server app clients >/dev/null 2>&1 && READY=1 && break; done
echo "daemon ready=$READY after ${i}s"; sleep 8
python3 - <<'PYEOF' > /tmp/i2prov-run.log 2>&1
import json, subprocess, time, os

bin_path = os.environ["BIN"]
NONCE = str(int(time.time()))[-6:]

def verb(*args, timeout=90):
    return subprocess.run([bin_path] + list(args), capture_output=True, text=True, timeout=timeout)

TRACE = os.environ.get("YGGTERM_HOME", "/tmp/i2prov-home-unknown") + "/event-trace.jsonl"

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
    return text or ""

def typed(session, data):
    verb("server", "app", "terminal", "probe-type", session, "--data", data, "--enter", "--mode", "xterm")

def row_session(n):
    out = verb("server", "app", "terminal", "new", "--kind", "shell", "--title", "demo%d" % n)
    try:
        return (json.loads(out.stdout).get("data") or {}).get("session_path")
    except Exception:
        print("new row raw:", out.stdout[:200], out.stderr[:200])
        return None

# row a: the app-control birth. Its birth open-attempt sits Pending forever
# (queue defect (a)) — the measurement watches its page counters the whole way.
a = row_session(1)
if not a:
    print("FAIL: no row a"); raise SystemExit(2)
ok, _ = wait_for(lambda ev: session_events(ev, a, "attach_ready"), 40, "a ready")
if not ok:
    print("PRE-FLIGHT FAIL: a never ready (healthy mount)"); raise SystemExit(4)
print("row a born:", a)
time.sleep(6)  # settle: organic paint flow on the initially-active row

# Arm the late marker (SPLIT — echo can never satisfy it), confirm absent.
ML = "PROVLATE_" + NONCE
typed(a, "sleep 12; printf '%s%s\\n' PROV LATE_" + NONCE)
time.sleep(0.5)
if ML in client_buffer(a):
    print("PRE-FLIGHT FAIL: late marker already present before the churn"); raise SystemExit(4)

# THE CHURN: b's creation prepends and demotes a; the skip-cancel grace runs.
slice_start = len(read_events())
b = row_session(2)
if not b:
    print("FAIL: no row b"); raise SystemExit(2)
print("row b born (churn):", b, "— slice starts at line", slice_start)
ok, _ = wait_for(lambda ev: session_events(ev, b, "attach_ready"), 45, "b ready")
if not ok:
    print("PRE-FLIGHT FAIL: b never ready"); raise SystemExit(4)
time.sleep(12)  # through the cancel grace

# Re-select a: the raise veto arm (defect (c)) fires here.
verb("server", "app", "open", a, "--view", "terminal")
time.sleep(6)

# THE MEASUREMENT READ — full series + incidents, all DATA.
events = read_events()
samples = [(ln, p) for ln, p in session_events(events, a, "terminal_paint_provenance")]
print("=== SAMPLE SERIES for a (%d samples) ===" % len(samples))
for ln, p in samples:
    print("sample[%s] ln=%d paint_count=%s send_attempts=%s key=%s live=%s husk=%s receipt=%s" % (
        p.get("sample"), ln, p.get("paint_count"), p.get("send_attempts"),
        p.get("last_paint_key"), p.get("entry_live"), p.get("was_husk"),
        json.dumps(p.get("applied_receipt")) if p.get("applied_receipt") else "null"))

incidents = {}
for name in ("session_cancelled", "reveal_raise_refused", "bootstrap_owner_superseded_during_loop",
             "terminal_first_paint_witness", "paint_ready", "applied_content_reattested"):
    hits = session_events(events, a, name)
    incidents[name] = hits
    print("=== %s on a: %d ===" % (name, len(hits)))
    for ln, p in hits[-6:]:
        print("   ln=%d %s" % (ln, json.dumps({k: v for k, v in p.items()
              if k in ("reason", "loop_live", "was_ever_ready", "paint_qualified",
                       "paint_pending", "promoted", "accepted", "child_count",
                       "xterm_present", "screen_present")})))

# MEASUREMENT SUMMARY (the answer to the owed question).
send_vals = [p.get("send_attempts") for _, p in samples if isinstance(p.get("send_attempts"), (int, float))]
paint_vals = [p.get("paint_count") for _, p in samples if isinstance(p.get("paint_count"), (int, float))]
receipt_vals = [p for _, p in samples if p.get("applied_receipt")]
print("=== MEASUREMENT SUMMARY ===")
print("samples=%d max_paint_count=%s max_send_attempts=%s send_ever_attempted=%s receipt_ever_seen=%s" % (
    len(samples),
    max(paint_vals) if paint_vals else None,
    max(send_vals) if send_vals else None,
    any(v > 0 for v in send_vals) if send_vals else None,
    bool(receipt_vals)))
if len(samples) < 3:
    print("MEASUREMENT VOID: <3 samples captured")
    raise SystemExit(4)
print("MEASUREMENT CAPTURED (data, not a verdict)")
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
tail -80 /tmp/i2prov-run.log
echo "scratch=$SCRATCH rc=$RC"
exit "$RC"
