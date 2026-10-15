#!/bin/bash
# [s49] THE PAINT SEND-LEDGER MEASUREMENT RIG (i2send-rig) — F1(c) site
# discrimination (board plan ACK-a505c8d0ff; sol s49 outcome map). NOT a
# pass/fail falsifier: it NAMES the loss site (or proves arrivals) with
# per-record identity evidence.
#   ARMING: YGGTERM_TEST_PAINT_PROVENANCE=1 (the 250ms page probe, extended
#   with the send-ledger summary) + YGGTERM_TEST_PAINT_SEND_LEDGER=1 (the
#   staged send ledger). Production never sets either.
#   SEQUENCE: row a born via app-control (organic cold mount) -> PHASE 1
#   plain settle (10s) -> PHASE 2 the s47 churn (row b demotes a, cancel
#   grace, re-select a) -> read the whole trace + the ledger samples.
#   VERDICT DATA (sol s49 Q3 outcome map):
#     ledger paint stages per outcome + Rust terminal_paint_receive count
#     + paint_seen_at_entry + js_event_ignored + applied_content_promoted.
#     SITE NAMED: no-send-api => A; build-threw => B; threw => C;
#     returned-sync+receive=0 => returned-not-received (D2);
#     returned-sync+ignored-with-paint => D1 decode;
#     receive>0 => paints ARRIVE (all-paint_seen = the guard masked them).
#   RC: 0 measurement captured · 2 setup · 4 pre-flight · 9 exe-mismatch.
# Usage: tools/uxspeed/i2send-rig.sh <binary>  (absolute path; full-workspace
# release build per rig law L1).
set -u
BIN=${1:?usage: i2send-rig.sh <binary>}
SCRATCH=/tmp/i2send-home-$(date +%s)
mkdir -p "$SCRATCH"
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN — build first"; exit 2; fi
export BIN
pkill -f "Xvfb :79" 2>/dev/null; sleep 1
Xvfb :79 -screen 0 1600x1000x24 -ac > /tmp/i2send-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5
# ⛔ HERMETIC ([11.231]): XDG_DATA_HOME must point at the scratch or the
# binary re-execs the INSTALLED direct build; asserted via the EXE PROOF below.
export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
dbus-run-session -- bash -c "
  export DISPLAY=:79 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
  export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share YGGTERM_HOT_PREMOUNT_CAP=2
  export YGGTERM_TEST_PAINT_PROVENANCE=1 YGGTERM_TEST_PAINT_SEND_LEDGER=1
  exec $BIN > /tmp/i2send-gui.log 2>&1
" >/dev/null 2>&1 </dev/null &
GUI_WRAP=$!
READY=0; for i in $(seq 1 90); do sleep 1; "$BIN" server app clients >/dev/null 2>&1 && READY=1 && break; done
echo "daemon ready=$READY after ${i}s"; sleep 8
python3 - <<'PYEOF' > /tmp/i2send-run.log 2>&1
import json, subprocess, time, os

bin_path = os.environ["BIN"]
NONCE = str(int(time.time()))[-6:]

def verb(*args, timeout=90):
    return subprocess.run([bin_path] + list(args), capture_output=True, text=True, timeout=timeout)

TRACE = os.environ.get("YGGTERM_HOME", "/tmp/i2send-home-unknown") + "/event-trace.jsonl"

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

# ---- row a: organic cold mount -------------------------------------------
a = row_session(1)
if not a:
    print("FAIL: no row a"); raise SystemExit(2)
ok, _ = wait_for(lambda ev: session_events(ev, a, "attach_ready"), 40, "a ready")
if not ok:
    print("PRE-FLIGHT FAIL: a never ready (healthy mount)"); raise SystemExit(4)
print("row a born:", a)

# ---- PHASE 1: plain organic settle (no churn) ----------------------------
time.sleep(10)
phase1_end = len(read_events())
print("PHASE 1 complete at trace line", phase1_end)

# ---- PHASE 2: the s47 churn ----------------------------------------------
# Arm a late marker (SPLIT) so the demotion + re-select is observable.
ML = "SENDLATE_" + NONCE
typed(a, "sleep 12; printf '%s%s\\n' SEND LATE_" + NONCE)
time.sleep(0.5)
if ML in client_buffer(a):
    print("PRE-FLIGHT FAIL: late marker already present before the churn"); raise SystemExit(4)
b = row_session(2)
if not b:
    print("FAIL: no row b"); raise SystemExit(2)
print("row b born (churn):", b)
ok, _ = wait_for(lambda ev: session_events(ev, b, "attach_ready"), 45, "b ready")
if not ok:
    print("PRE-FLIGHT FAIL: b never ready"); raise SystemExit(4)
time.sleep(12)  # through the cancel grace
verb("server", "app", "open", a, "--view", "terminal")
time.sleep(6)

# ---- THE MEASUREMENT READ -------------------------------------------------
events = read_events()
samples = [(ln, p) for ln, p in session_events(events, a, "terminal_paint_provenance")]
print("=== SAMPLE SERIES for a (%d samples) ===" % len(samples))
for ln, p in samples:
    sl = p.get("send_ledger")
    sl_sum = None
    if isinstance(sl, dict):
        sl_sum = "seq=%s dropped=%s retained=%s stages=%s" % (
            sl.get("seq"), sl.get("dropped"), sl.get("retained"),
            json.dumps(sl.get("paint_by_stage")))
    mf = p.get("mount_fn_has_ledger")
    print("   [diag] mount_fn_has_ledger=%s branch=%s mount_fn_v=%s" % (mf, p.get("mount_fn_ledger_branch"), p.get("mount_fn_v")))
    print("sample[%s] ln=%d paint_count=%s send_attempts=%s key=%s receipt=%s ledger=%s" % (
        p.get("sample"), ln, p.get("paint_count"), p.get("send_attempts"),
        p.get("last_paint_key"),
        json.dumps(p.get("applied_receipt")) if p.get("applied_receipt") else "null",
        sl_sum))

receives = session_events(events, a, "terminal_paint_receive")
print("=== terminal_paint_receive on a: %d ===" % len(receives))
for ln, p in receives[-8:]:
    print("   ln=%d child=%s cols=%s/%s paint_seen_at_entry=%s" % (
        ln, p.get("child_count"), p.get("cols"), p.get("rows"),
        p.get("paint_seen_at_entry")))
ready = session_events(events, a, "paint_ready")
ignored = [(ln, p) for ln, p in session_events(events, a, "js_event_ignored")
           if "paint" in json.dumps(p.get("value") or "")[:200]]
promoted = session_events(events, a, "applied_content_promoted")
print("=== paint_ready=%d js_event_ignored(paint-ish)=%d applied_content_promoted=%d ===" % (
    len(ready), len(ignored), len(promoted)))
for ln, p in ignored[-4:]:
    print("   ignored ln=%d reason=%s" % (ln, p.get("reason")))
for ln, p in promoted[-4:]:
    print("   promoted ln=%d accepted=%s promoted=%s wrote=%s" % (
        ln, p.get("accepted"), p.get("promoted"), p.get("wrote")))

# ---- SITE NAMING (sol s49 Q3 outcome map) ---------------------------------
stage_counts = {}
last_ledger = None
for _, p in samples:
    sl = p.get("send_ledger")
    if isinstance(sl, dict):
        last_ledger = sl
if last_ledger:
    for k, v in (last_ledger.get("paint_by_stage") or {}).items():
        stage_counts[k] = v
print("=== SITE VERDICT (data) ===")
print("ledger_paint_stages=%s" % json.dumps(stage_counts, sort_keys=True))
returned = sum(v for k, v in stage_counts.items() if k == "returned-sync")
no_api = stage_counts.get("no-send-api", 0)
build_threw = sum(v for k, v in stage_counts.items() if k.startswith("build-threw"))
threw = sum(v for k, v in stage_counts.items() if k.startswith("threw") and not k.startswith("build"))
attempted = sum(v for k, v in stage_counts.items() if k == "attempt-recorded")
if attempted == 0 and len(receives) == 0:
    print("SITE: no paint sends ATTEMPTED and none received — the s47 premise")
    print("  (3 attempts) did not reproduce this run; rerun before concluding.")
elif no_api:
    print("SITE A NAMED: no-send-api — terminalDioxusSend was null at send time")
elif build_threw:
    print("SITE B NAMED: build-threw — payload construction raised (%s)" % build_threw)
elif threw:
    print("SITE C NAMED: send threw synchronously (%s)" % threw)
elif returned and len(receives) == 0 and len(ignored) == 0:
    print("SITE D2 FAMILY: returned-sync=%d but ZERO Rust receives and no" % returned)
    print("  Ignored — returned is not enqueued; the transport boundary is")
    print("  between the page send and the eval queue drain.")
elif returned and len(ignored) > 0:
    print("SITE D1 NAMED: decode rejection — js_event_ignored carries paint payloads")
elif len(receives) > 0:
    seen_true = sum(1 for _, p in receives if p.get("paint_seen_at_entry") is True)
    print("PAINTS ARRIVE: %d receives (paint_seen_at_entry=true on %d — the" % (
        len(receives), seen_true))
    print("  witness guard masked arrivals exactly as the s49 map predicted)")
else:
    print("SITE: UNRESOLVED — inspect the full sample series above")
print("=== END VERDICT ===")

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
tail -90 /tmp/i2send-run.log
echo "scratch=$SCRATCH rc=$RC"
exit "$RC"
