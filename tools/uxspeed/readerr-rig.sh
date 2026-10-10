#!/bin/bash
# [11.229](b)] THE READ-ERROR RIG (readerr-rig) — the LOCAL regression bar
# for the bounded established-frame hold: YGGTERM_TEST_READ_ERROR_FILE
# (absent = inert) makes terminal_read_async Err deterministically once the
# rig creates the marker file; a healthy LOCAL row must take the BOUNDED
# machinery (read_error_after_attach -> recovery retries -> exhaustion ->
# input re-enabled), never a silent unbounded hold.
#   ⛔ This rig does NOT construct the REMOTE established-frame branch
#   (is_remote_resume_session / LiveSsh — the (c)/(f2) construction bar;
#   s20's rig v5b remote-row pattern pointed at a live remote target is the
#   owed construction). The remote branch's fix is shape-locked in the suite
#   (established_frame_read_error_hold_is_bounded_and_escalates) and its
#   production detector is read_error_held_frame_escalated in the wild.
#   GREEN (rc 0): the typed row traces read_error_after_attach (the hook
#     demonstrably errors reads AND the bounded ladder engages) within 20s
#     of arming.
#   RED (rc 5): armed but no read-error handling traces for the row.
#   2=boot/rows, 3=paint preflight, 4=hook not engaged (no read errors at
#   all), 8=exe mismatch.
# Usage: tools/uxspeed/readerr-rig.sh <binary>
set -u
export TERM=xterm-256color
BIN=${1:?usage: readerr-rig.sh <binary>}
SCRATCH=/tmp/readerr-home-$(date +%s)-$$
export BIN SCRATCH
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN"; exit 2; fi
mkdir -p "$SCRATCH"

run_boot() {
  export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
  dbus-run-session -- bash -c "
    export DISPLAY=:79 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
    export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
    export YGGTERM_TEST_READ_ERROR_FILE=$SCRATCH/read-errors
    exec $BIN > /tmp/readerr-gui.log 2>&1
  " >/dev/null 2>&1 </dev/null &
}

boot_wait_ready() {
  local READY=0 i
  for i in $(seq 1 150); do sleep 1; "$BIN" server app clients >/dev/null 2>&1 && READY=1 && break; done
  echo $READY
}

scratch_pids_by_name() {
  for p in $(pgrep -x "$1" 2>/dev/null); do
    [ "$p" = "$$" ] && continue
    local h; h=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | sed -n 's/^YGGTERM_HOME=//p')
    [ "$h" = "$SCRATCH" ] && echo "$p"
  done
}

for p in $(pgrep -x Xvfb); do
  c=$(tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null)
  case "$c" in *":79"*) kill "$p";; esac
done 2>/dev/null; sleep 1
Xvfb :79 -screen 0 1600x1000x24 -ac > /tmp/readerr-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5

run_boot
GUI_WRAP=$!
READY=$(boot_wait_ready); echo "gui ready=$READY"
if [ "$READY" != "1" ]; then
  echo "BOOT FAIL"; kill "$GUI_WRAP" 2>/dev/null
  for p in $(scratch_pids_by_name yggterm); do kill "$p" 2>/dev/null; done
  kill "$XVFB_PID" 2>/dev/null; exit 2
fi

SCRATCH=$SCRATCH BIN=$BIN python3 - <<'PYEOF' > /tmp/readerr-run.log 2>&1
import json, subprocess, time, os

BIN = os.environ["BIN"]
TRACE = os.environ["SCRATCH"] + "/event-trace.jsonl"

def verb(*args, timeout=60):
    return subprocess.run([BIN] + list(args), capture_output=True, text=True, timeout=timeout)

def read_events():
    out = []
    try:
        text = open(TRACE).read()
    except FileNotFoundError:
        return out
    dec = json.JSONDecoder()
    idx, n = 0, len(text)
    while idx < n:
        while idx < n and text[idx] in " \t\r\n":
            idx += 1
        if idx >= n:
            break
        try:
            event, idx = dec.raw_decode(text, idx)
        except Exception:
            idx += 1; continue
        name = (event.get("name") or event.get("event")
                or (event.get("payload") or {}).get("name")
                or (event.get("payload") or {}).get("event"))
        if name:
            out.append((int(event.get("ts_ms", 0)), name, event.get("payload") or {}))
    return out

def snapshot_rows():
    out = verb("server", "snapshot", timeout=30)
    try:
        return json.loads(out.stdout).get("live_sessions") or []
    except Exception:
        return []

def create_row_titled(title, polls=90):
    verb("server", "app", "terminal", "new", "--kind", "shell", "--title", title, timeout=30)
    for _ in range(polls):
        for r in snapshot_rows():
            if r.get("title") == title:
                return r.get("session_path")
        time.sleep(1)
    return None

a = create_row_titled("readerr1")
b = create_row_titled("readerr2")
print("rows: a=%s b=%s" % (a, b))
if not (a and b):
    print("FAIL: could not create rows"); raise SystemExit(2)
time.sleep(12)

def screen_count(marker, session):
    body = (verb("server", "screen", session, timeout=30).stdout or "")
    return body.count("echo " + marker)

# healthy-mount proof on the ACTIVE row before any read errors exist
verb("server", "app", "terminal", "probe-type", b, "--data", "echo READERR", "--enter", "--mode", "xterm", timeout=30)
painted = 0
deadline = time.time() + 60
while time.time() < deadline:
    painted = screen_count("READERR", b)
    if painted >= 1:
        break
    time.sleep(1)
print("marker painted on active row: %d" % painted)
if painted < 1:
    print("PREFLIGHT FAIL: marker never painted before arming"); raise SystemExit(3)

# ---- ARM: every session's reads now Err (empty marker) ----
t_arm_ms = int(time.time() * 1000)
open(os.environ["SCRATCH"] + "/read-errors", "w").write("\n")
print("armed read-error hook at %d" % t_arm_ms)

time.sleep(20)
events = read_events()
def row_events(name):
    return [(ts, p) for ts, nm, p in events
            if nm == name and p.get("session_path") == b and ts >= t_arm_ms]

after_attach = row_events("read_error_after_attach")
initial_retry = row_events("read_error_retry_before_attach")
exhausted = row_events("read_error_after_attach_retries_exhausted")
held = [(ts, p) for ts, nm, p in events
        if nm == "ghost_frame_held" and ts >= t_arm_ms]
read_errors_total = len([(ts, p) for ts, nm, p in events
                         if nm in ("read_error_after_attach", "read_error_retry_before_attach", "ghost_frame_held") and ts >= t_arm_ms])
print("post-arm: after_attach=%d initial_retry=%d exhausted=%d ghost_held(any row)=%d total_read_error_events=%d" % (
    len(after_attach), len(initial_retry), len(exhausted), len(held), read_errors_total))
for ts, p in after_attach[:3]:
    print("  after_attach ts=%d attempt=%s error=%s" % (ts, p.get("attempt"), str(p.get("error"))[:60]))

if read_errors_total == 0:
    print("VERDICT HOOK: NOT ENGAGED — no read errors traced after arming; rig invalid"); raise SystemExit(4)
if after_attach:
    print("VERDICT BAR: GREEN — the hook errors reads AND the local row takes the BOUNDED ladder (read_error_after_attach x%d; exhausted=%d) — no silent hold" % (
        len(after_attach), len(exhausted)))
    raise SystemExit(0)
print("VERDICT BAR: RED — read errors engaged (%d events) but the typed row never entered the post-attach ladder" % read_errors_total)
raise SystemExit(5)
PYEOF
RC=$?
kill "$GUI_WRAP" 2>/dev/null; sleep 1
for p in $(scratch_pids_by_name yggterm); do kill "$p" 2>/dev/null; done
pkill -f "dbus-run-session" 2>/dev/null; sleep 1; kill "$XVFB_PID" 2>/dev/null
for p in $(pgrep -f 'yggterm-headless server daemon' 2>/dev/null); do
  h=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | sed -n 's/^YGGTERM_HOME=//p'); [ "$h" = "$SCRATCH" ] && kill "$p" 2>/dev/null
done
echo "=== EXE PROOF ==="
EXE=$(grep -m1 -o '"executable_path":"[^"]*"' "$SCRATCH/event-trace.jsonl" 2>/dev/null | head -1 | cut -d'"' -f4)
echo "served executable_path=$EXE"
if [ "$EXE" = "$BIN" ]; then echo "EXE OK"; else echo "EXE MISMATCH: verdicts VOID"; RC=8; fi
echo "=== DRIVER OUTPUT ==="
tail -25 /tmp/readerr-run.log
echo "scratch=$SCRATCH rc=$RC"
exit $RC
