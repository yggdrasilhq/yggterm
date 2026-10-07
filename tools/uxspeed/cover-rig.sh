#!/bin/bash
# [11.229](c) THE REVEAL-DEADLINE RIG (cover-rig) — the falsifier
# ATTEMPT for `reveal_cover_released reason:deadline bytes:0` dropping
# the cover to a canvas the reveal never painted (queue line ~660; filed
# fix shape: paint the daemon-screen reconcile on expiry).
# ⛔ CONSTRUCTION MEASURED IMPOSSIBLE FOR LOCAL ROWS (2026-10-07, two
#   variants: plain boot-2 and COVER_STALL_MS=6000 snapshot-stalled
#   boot-2 — byte-identical outcomes): every cover-arming site in the
#   mount loop gates on is_remote_resume_agent_session = a remote agent
#   PATH * SessionSource::LiveSsh — structurally unreachable for local
#   rig rows, whose boot-2 restore takes the reuse+rehydrate path and
#   paints the frozen frame from the first sample (SLEEPMARK×2 in all 12
#   samples, zero cover events). The (f2)/(h2) construction-bar
#   precedent applies: the detector is the production trace event
#   `reveal_cover_deadline_reconcile_armed`, locked by
#   a_deadline_expired_cover_with_no_buffered_bytes_rearms_the_screen_
#   reconcile. THIS RIG'S LIVE VALUE: the boot-2 reveal-health bar
#   (GREEN = the reuse+rehydrate path repaints a frozen silent row from
#   the daemon frame within the observe window) + the ready skeleton
#   for a LiveSsh-loopback row when one becomes constructible headless.
# SHAPE (f3-rig's boot-2 skeleton): boot-1 fresh scratch GUI with REAL
#   IPC; dummy+test rows; the test row builds REAL SCROLLBACK (60k
#   lines), paints SLEEPMARK, then goes SILENT (`exec sleep 9999` — the
#   runtime never paints again but KEEPS RUNNING and the daemon's vt100
#   retains the frame). Kill ONLY the GUI; boot-2 over the live daemon.
#   GREEN (rc 0): the client buffer CARRIES the daemon's frame
#     (SLEEPMARK readable at the client) — the restore repaints.
#   5=client blank (restore regression — THIS is the live bar),
#   2=rows, 3=boot-1 gate, 4=pre-flight, 6=no deadline observed (the
#   expected local-row outcome today), 8=exe, 9=daemon died, 10=heal
#   failed (re-arm fired, client stayed blank).
# Usage: tools/uxspeed/cover-rig.sh <binary>
#   ⛔ trace law: whole-buffer raw_decode, never line-parse.
set -u
export TERM=xterm-256color
BIN=${1:?usage: cover-rig.sh <binary>}
SCRATCH=/tmp/cover-home-$(date +%s)-$$
export BIN SCRATCH
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN"; exit 2; fi

run_boot() {  # $2=stall_ms (boot-2 arm: hold the snapshot fetch past the cover deadline)
  mkdir -p "$SCRATCH"
  export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
  local STALL_EXTRA=""
  [ -n "${2:-}" ] && [ "${2:-0}" != "0" ] && STALL_EXTRA="export YGGTERM_TEST_STALL_SNAPSHOT_FETCH_MS=${2}"
  dbus-run-session -- bash -c "
    export DISPLAY=:82 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
    export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
    export YGGTERM_HOT_PREMOUNT_CAP=2
    $STALL_EXTRA
    exec $BIN > /tmp/cover-gui.log 2>&1
  " >/dev/null 2>&1 </dev/null &
}

boot_wait_ready() {
  local READY=0 i
  for i in $(seq 1 90); do sleep 1; "$BIN" server app clients >/dev/null 2>&1 && READY=1 && break; done
  echo $READY
}

scratch_pids_by_name() {
  for p in $(pgrep -x "$1" 2>/dev/null); do
    [ "$p" = "$$" ] && continue
    local h; h=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | sed -n 's/^YGGTERM_HOME=//p')
    [ "$h" = "$SCRATCH" ] && echo "$p"
  done
}

kill_gui_only() {
  local p cmd
  for p in $(scratch_pids_by_name yggterm); do
    cmd=$(tr '\0' '\n' < /proc/$p/cmdline 2>/dev/null | tr '\n' ' ')
    case "$cmd" in *"server daemon"*) continue;; esac
    kill "$p" 2>/dev/null && echo "killed GUI pid $p"
  done
}

daemon_alive() {
  local p cmd h
  for p in $(pgrep -f 'server daemon' 2>/dev/null); do
    [ "$p" = "$$" ] && continue
    cmd=$(tr '\0' '\n' < /proc/$p/cmdline 2>/dev/null | tr '\n' ' ') || continue
    case "$cmd" in *yggterm*server*daemon*) ;; *) continue;; esac
    h=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | sed -n 's/^YGGTERM_HOME=//p')
    if [ "$h" = "$SCRATCH" ]; then echo "daemon pid $p"; return 0; fi
  done
  return 1
}

# ---- display ----
for p in $(pgrep -x Xvfb); do
  c=$(tr '\0' '\n' < /proc/$p/cmdline 2>/dev/null | tr '\n' ' ')
  case "$c" in *":82"*) kill "$p";; esac
done 2>/dev/null; sleep 1
Xvfb :82 -screen 0 1600x1000x24 -ac > /tmp/cover-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5

# ---- boot 1 ----
run_boot "$SCRATCH"
GUI_WRAP=$!
READY=$(boot_wait_ready); echo "boot1 daemon ready=$READY"

SCRATCH=$SCRATCH BIN=$BIN python3 - <<'PYEOF' > /tmp/cover-run.log 2>&1
import json, subprocess, time, os

BIN = os.environ["BIN"]

def verb(*args, timeout=90):
    return subprocess.run([BIN] + list(args), capture_output=True, text=True, timeout=timeout)

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

def client_screen(path):
    r = verb("server", "app", "terminal", "read-buffer", path, "--mode", "screen", timeout=30)
    return (r.stdout or "")

def daemon_screen(path):
    return verb("server", "screen", path, timeout=30).stdout or ""

def type_into(path, data):
    verb("server", "app", "terminal", "probe-type", path, "--data", data, "--enter", "--mode", "xterm", timeout=30)

_a = create_row_titled("cov0")
TEST = create_row_titled("cov1")
print("rows: a=%s test=%s" % (_a, TEST))
if not TEST:
    print("FAIL: could not create rows"); raise SystemExit(2)
open("/tmp/cover-test-path", "w").write(TEST)

# wait for the mount, then build scrollback + the marker, then silence
mounted = False
deadline = time.time() + 90
while time.time() < deadline:
    if len(client_screen(TEST).strip()) > 40:
        mounted = True; break
    time.sleep(1)
print("boot1: prompt mounted=%s" % mounted)
if not mounted:
    print("BOOT-1 GATE FAIL: rows never mounted"); raise SystemExit(3)

# scrollback: the filed signature's cold-transcript ingredient
type_into(TEST, "seq 1 60000 > /tmp/cov-seq && tail -n +59990 /tmp/cov-seq | head -1 && echo SCROLLDONE")
saw = 0
deadline = time.time() + 60
while time.time() < deadline:
    body = daemon_screen(TEST)
    if body.count("SCROLLDONE") >= 1:
        saw = body.count("SCROLLDONE"); break
    time.sleep(1)
print("boot1: scrollback built=%d (SCROLLDONE on daemon)" % saw)

# the frozen frame marker, then silence the runtime FOR GOOD
type_into(TEST, "echo SLEEPMARK-7f31 && exec sleep 9999")
marked = 0
deadline = time.time() + 30
while time.time() < deadline:
    body = daemon_screen(TEST)
    if body.count("SLEEPMARK-7f31") >= 1:
        marked = body.count("SLEEPMARK-7f31"); break
    time.sleep(0.5)
print("boot1: SLEEPMARK painted=%d" % marked)
time.sleep(3)
# silence proof: another echo typed now must NEVER execute. The kernel's
# line-discipline echo still paints the TYPED line once (the tty echoes
# what is typed regardless of who reads stdin) — exactly ONE occurrence
# is the silent state; a second occurrence is bash's output = NOT silent.
type_into(TEST, "echo AFTER-SILENCE")
time.sleep(4)
leaked = daemon_screen(TEST).count("AFTER-SILENCE")
print("boot1: silence verified (AFTER-SILENCE occurrences=%d, want 1 = kernel echo only)" % leaked)
if not (saw and marked and leaked == 1):
    print("PRE-FLIGHT FAIL: scrollback/marker/silence incomplete"); raise SystemExit(4)
raise SystemExit(0)
PYEOF
RC=$?
if [ "$RC" -ne 0 ]; then
  echo "boot1 phase rc=$RC"; kill "$GUI_WRAP" 2>/dev/null
  for p in $(scratch_pids_by_name yggterm); do kill "$p" 2>/dev/null; done
  for p in $(pgrep -f 'yggterm-headless server daemon' 2>/dev/null); do
    h=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | sed -n 's/^YGGTERM_HOME=//p'); [ "$h" = "$SCRATCH" ] && kill "$p" 2>/dev/null
  done
  kill "$XVFB_PID" 2>/dev/null; exit "$RC"
fi
TEST=$(cat /tmp/cover-test-path)
echo "boot1 ok, test=$TEST"

# ---- kill ONLY the GUI; the daemon must survive ----
sleep 3
T_KILL_MS=$(date +%s%3N)
echo "T_KILL_MS=$T_KILL_MS"
kill_gui_only
sleep 2
if daemon_alive >/dev/null; then
  daemon_alive
else
  echo "DAEMON DIED WITH THE GUI — relaunch shape impossible (rc 9)"
  kill "$XVFB_PID" 2>/dev/null; exit 9
fi

# ---- boot 2 over the live daemon (COVER_STALL_MS: hold the snapshot
# fetch past the 1s cover deadline — the re-render race's deterministic
# stand-in for the field's slow cold transcript) ----
run_boot "$SCRATCH" "${COVER_STALL_MS:-0}"
GUI_WRAP=$!
READY=$(boot_wait_ready); echo "boot2 gui ready=$READY"

# ---- observe + verdict ----
SCRATCH=$SCRATCH BIN=$BIN TEST=$TEST T_KILL_MS=$T_KILL_MS python3 - <<'PYEOF' >> /tmp/cover-run.log 2>&1
import json, subprocess, time, os
from collections import Counter

BIN = os.environ["BIN"]; TEST = os.environ["TEST"]
TRACE = os.environ["SCRATCH"] + "/event-trace.jsonl"
T_KILL = int(os.environ["T_KILL_MS"])

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
            out.append((int(event.get("ts_ms", 0)), event.get("component",""), name, event.get("payload") or {}))
    return out

def client_screen(path):
    r = verb("server", "app", "terminal", "read-buffer", path, "--mode", "screen", timeout=30)
    return (r.stdout or "")

def daemon_screen(path):
    return verb("server", "screen", path, timeout=30).stdout or ""

# observe the client buffer across the heal windows (cover deadline 1s;
# reconcile settle + unwritable retries; frame-hash 2s)
samples = []
for i in range(12):
    body = client_screen(TEST)
    samples.append(body.count("SLEEPMARK-7f31"))
    time.sleep(5)
client_final = samples[-1] if samples else 0
client_max = max(samples) if samples else 0
print("boot2 observe: client SLEEPMARK samples=%s max=%d final=%d" % (samples, client_max, client_final))
ds = daemon_screen(TEST)
daemon_has = ds.count("SLEEPMARK-7f31")
print("boot2: daemon screen SLEEPMARK=%d (client %s)" % (daemon_has, "BLANK" if client_final == 0 else "carries"))

events = [(ts, comp, nm, p) for ts, comp, nm, p in read_events() if ts >= T_KILL - 250]
reg = [p for ts, comp, nm, p in events if nm == "register"]
exe = (reg[-1].get("executable_path") if reg else "") or ""
want = os.path.realpath(BIN)
got = os.path.realpath(exe) if exe else ""
print("exe-proof: served=%s want=%s" % (exe, want))
if got != want:
    print("EXE MISMATCH — the rig measured a different build than it launched")
    raise SystemExit(8)

test_events = [(ts, nm, p) for ts, comp, nm, p in events if p.get("session_path") == TEST]
hist = Counter(nm for ts, nm, p in test_events)
print("boot2 event histogram for test row (%d events):" % len(test_events))
for nm, c in hist.most_common(24):
    print("  %4d  %s" % (c, nm))

deadline0 = [(ts, p) for ts, nm, p in test_events if nm == "reveal_cover_released" and p.get("reason") == "deadline"]
zero_byte = [(ts, p) for ts, p in deadline0 if int(p.get("bytes") or 0) == 0]
rearm = [(ts, p) for ts, nm, p in test_events if nm == "reveal_cover_deadline_reconcile_armed"]
reconcile_writes = [(ts, p) for ts, nm, p in test_events
                    if nm in ("screen_reconcile_written", "screen_reconcile_should_write")
                    or "reconcile" in nm and "writ" in nm]
print("cover deadlines=%d (bytes:0=%d) rearm=%d reconcile-ish=%d" % (
    len(deadline0), len(zero_byte), len(rearm), len(reconcile_writes)))
for ts, p in deadline0[:4]:
    print("   deadline at +%dms bytes=%s gen=%s" % (ts - T_KILL, p.get("bytes"), p.get("generation")))

if not zero_byte:
    print("VERDICT: rc6 — construction failed: no bytes:0 cover deadline observed for the test row")
    print("   (the reveal never armed a cover on this shape — see histogram)")
    raise SystemExit(6)
if client_final >= 1:
    if rearm:
        print("RIG: GREEN — deadline re-armed the reconcile and the client carries the daemon frame")
        raise SystemExit(0)
    print("RIG: rc6-ambiguous — client painted but WITHOUT the re-arm (another path healed it; see histogram)")
    raise SystemExit(6)
if rearm:
    print("RIG: rc10 — heal failed: re-arm fired but the client stayed blank (reconcile write failed?)")
    raise SystemExit(10)
print("RIG: RED CONFIRMED — bytes:0 deadline left the client blank with the daemon frame available")
raise SystemExit(5)
PYEOF
RC=$?
kill "$GUI_WRAP" 2>/dev/null; sleep 1
for p in $(scratch_pids_by_name yggterm); do kill "$p" 2>/dev/null; done
for p in $(pgrep -f 'yggterm-headless server daemon' 2>/dev/null); do
  h=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | sed -n 's/^YGGTERM_HOME=//p'); [ "$h" = "$SCRATCH" ] && kill "$p" 2>/dev/null
done
sleep 1; kill "$XVFB_PID" 2>/dev/null
echo "=== EXE/DRIVER TAIL ==="
tail -45 /tmp/cover-run.log
echo "scratch=$SCRATCH rc=$RC"
exit $RC
