#!/bin/bash
# [RT] THE REPLACED-RUNTIME RIG — the [11.229](b) LIVE-GUI arm reproducer
# (plan ACK-c588ce4b8b; sitting-13 closed the GUI-RELAUNCH arm — this is the
# OTHER arm: a runtime replaced while the client stays MOUNTED).
#
# MECHANISM UNDER TEST: `server terminal restart` swaps the
# PtySessionRuntime under the SAME key (fresh chunk ring, seq resets,
# NEW runtime_spawn_id) while the mounted client keeps polling
# TerminalRead by path. The TerminalStream answer carries NO spawn
# identity; the client's only same-daemon recovery signal is a cursor
# rewind (next_cursor < cursor). The (b) measurement says the client
# freezes anyway (dead runtime's last frame forever).
#
# SHAPE: boot-1 fresh scratch GUI with REAL IPC on dev Xvfb :78 (no
# suppression); dummy+test rows (CAP=2, dummy FIRST — the dummy absorbs
# the cold path); painted gate: RTOLD typed + verified on BOTH the
# daemon screen AND the client read-buffer; then restart the test row's
# runtime DAEMON-SIDE (GUI stays up, mounted); type RTNEW on the new
# runtime; verify RTNEW lands on the daemon screen; observe the client
# read-buffer 80 s; verdict from the post-restart trace window
# (epoch ts_ms, whole-buffer raw_decode — the file mixes compact+pretty
# JSON) + exe-proof from the register event.
#   GREEN (rc 0): the client buffer carries RTNEW — the new runtime's
#     output reached the mounted host (direct paint or remount heal).
#   RED-A (rc 5): daemon has RTNEW, client NEVER does, and NO recovery
#     signal fired in the window (no terminal_stream_cursor_rewound, no
#     new mount/attach cycle) — the DETECTION hole: the stream needs
#     runtime_spawn_id + client invalidation (the filed fix shape).
#   RED-B (rc 4): a recovery signal fired (rewound or a new attach
#     cycle) but the client is still frozen — the recovery path itself
#     is broken/gated; instrument it, do NOT add another signal.
#   2=rows, 3=boot-1 painted gate, 6=neither contract, 8=exe mismatch,
#   9=daemon died at restart (shape impossible).
# Usage: tools/uxspeed/rt-rig.sh [binary] (dev Xvfb :78)
#   ⛔ Never trust `terminal new`'s rc — verify rows via `server snapshot`
#   live_sessions. probe-type bypasses the focus ladder; input delivery
#   is verified on the daemon screen only.
set -u
export TERM=xterm-256color
BIN=${BIN:-$HOME/.local/bin/yggterm}
SCRATCH=/tmp/rt-home-$(date +%s)-$$
export BIN SCRATCH
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN"; exit 2; fi

run_boot() {  # boots GUI, echoes wrap pid (real IPC — no suppression)
  mkdir -p "$SCRATCH"
  export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
  local FAKE_ENV=""
  if [ "${RT_FAKE_ID:-0}" = "1" ]; then
    # id-isolation arm: the daemon answers a FILE-driven fake spawn id
    # (YGGTERM_TEST_STREAM_SPAWN_ID_FILE, lane-only hook). Seed id 111111
    # at boot; the rig flips it later with NO restart — isolating the
    # client's replacement signal from the cursor-rewind signal.
    echo 111111 > "/tmp/rt-fake-spawn-id-$(basename "$SCRATCH")"
    FAKE_ENV="export YGGTERM_TEST_STREAM_SPAWN_ID_FILE=/tmp/rt-fake-spawn-id-$(basename "$SCRATCH")"
  fi
  dbus-run-session -- bash -c "
    export DISPLAY=:78 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
    export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
    export YGGTERM_HOT_PREMOUNT_CAP=2
    $FAKE_ENV
    exec $BIN > /tmp/rt-gui-$(basename $SCRATCH).log 2>&1
  " >/dev/null 2>&1 </dev/null &
}

boot_wait_ready() {
  local READY=0 i
  for i in $(seq 1 90); do sleep 1; "$BIN" server app clients >/dev/null 2>&1 && READY=1 && break; done
  echo $READY
}

scratch_pids_by_name() {  # $1=comm name -> pids whose YGGTERM_HOME=$SCRATCH
  for p in $(pgrep -x "$1" 2>/dev/null); do
    [ "$p" = "$$" ] && continue
    local h; h=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | sed -n 's/^YGGTERM_HOME=//p')
    [ "$h" = "$SCRATCH" ] && echo "$p"
  done
}

gui_alive() { [ -n "$(scratch_pids_by_name yggterm)" ] && echo up || return 1; }

cleanup() {
  [ -n "${GUI_WRAP:-}" ] && kill "$GUI_WRAP" 2>/dev/null
  sleep 1
  for p in $(scratch_pids_by_name yggterm); do
    cmd=$(tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null)
    case "$cmd" in *"server daemon"*) ;; *) kill "$p" 2>/dev/null;; esac
  done
  pkill -f "dbus-run-session" 2>/dev/null; sleep 1
  for p in $(pgrep -f 'yggterm-headless server daemon' 2>/dev/null); do
    h=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | sed -n 's/^YGGTERM_HOME=//p')
    [ "$h" = "$SCRATCH" ] && kill "$p" 2>/dev/null
  done
  kill "${XVFB_PID:-0}" 2>/dev/null
}
trap cleanup EXIT

# ---- display ----
for p in $(pgrep -x Xvfb); do
  c=$(tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null)
  case "$c" in *":78"*) kill "$p";; esac
done 2>/dev/null; sleep 1
Xvfb :78 -screen 0 1600x1000x24 -ac > /tmp/rt-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5

# ---- boot ----
run_boot
GUI_WRAP=$!
READY=$(boot_wait_ready); echo "boot daemon ready=$READY"

# ---- phase 1: rows + painted gate (RTOLD on daemon AND client) ----
SCRATCH=$SCRATCH BIN=$BIN python3 - <<'PYEOF' > /tmp/rt-run-$(basename $SCRATCH).log 2>&1
import json, subprocess, time, os

BIN = os.environ["BIN"]

def verb(*args, timeout=60):
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

def daemon_screen(path):
    r = verb("server", "screen", path, timeout=30)
    return r.stdout or ""

def client_buffer(path):
    r = verb("server", "app", "terminal", "read-buffer", path, "--mode", "screen", timeout=30)
    return (r.stdout or "") + (("\n[stderr] " + r.stderr.strip()) if r.returncode != 0 else "")

_a = create_row_titled("rtseed0")
TEST = create_row_titled("rtseed1")
print("rows: a=%s test=%s" % (_a, TEST))
if not TEST:
    print("FAIL: could not create rows"); raise SystemExit(2)
open("/tmp/rt-test-path", "w").write(TEST)

# wait for the MOUNT before typing (send on an unmounted row is handled
# but NOT delivered — probe first, then verify by screen)
mounted_prompt = False
deadline = time.time() + 90
body = ""
while time.time() < deadline:
    body = client_buffer(TEST)
    if len(body.strip()) > 40:
        mounted_prompt = True; break
    time.sleep(1)
print("boot: prompt mounted=%s (len=%d)" % (mounted_prompt, len(body.strip())))
if not mounted_prompt:
    print("BOOT GATE FAIL: fresh row never mounted"); raise SystemExit(3)

painted_daemon = painted_client = 0
for attempt in range(3):
    verb("server", "app", "terminal", "probe-type", TEST, "--data", "echo RTOLD", "--enter", "--mode", "xterm")
    deadline = time.time() + 20
    while time.time() < deadline:
        if daemon_screen(TEST).count("RTOLD") >= 1 and client_buffer(TEST).count("RTOLD") >= 1:
            painted_daemon = painted_client = 1; break
        time.sleep(0.5)
    if painted_daemon:
        break
    time.sleep(5)
print("boot gate: RTOLD daemon=%s client=%s" % (bool(painted_daemon), bool(painted_client)))
if not (painted_daemon and painted_client):
    print("BOOT GATE FAIL: RTOLD never painted on both planes — pre-flight broken"); raise SystemExit(3)
raise SystemExit(0)
PYEOF
RC=$?
if [ "$RC" -ne 0 ]; then echo "phase1 rc=$RC"; exit "$RC"; fi
TEST=$(cat /tmp/rt-test-path)
echo "phase1 ok, test=$TEST"

sleep 3   # settle past the fresh-mount window
if [ "${RT_FAKE_ID:-0}" = "1" ]; then
  # ID-ISOLATION ARM: flip the daemon's reported runtime id with NO restart
  # (no rewind can exist — cursor continuity is untouched). On the lane the
  # client must fire terminal_stream_runtime_replaced + recover; on unfixed
  # main the hook does not exist and the flip is invisible by construction.
  echo 222222 > "/tmp/rt-fake-spawn-id-$(basename "$SCRATCH")"
  echo "fake-id flipped 111111 -> 222222 (no restart)"
elif [ "${RT_DIVERGE:-0}" = "1" ]; then
  # THE (b) WILD SHAPE: daemon-side grid DIVERGENCE (client host stays at its
  # mounted grid; the CLI resize re-pins the PTY wide AND poisons the grid
  # record — the [11.228] vector) BEFORE the replacement.
  "$BIN" server terminal resize "$TEST" --cols 240 --rows 50 2>&1 | head -2
  sleep 1
fi
T_RESTART_MS=$(date +%s%3N)
echo "T_RESTART_MS=$T_RESTART_MS"

# ---- phase 2: restart the runtime DAEMON-SIDE; GUI stays up + mounted ----
if [ "${RT_FAKE_ID:-0}" != "1" ]; then
  "$BIN" server terminal restart "$TEST" 2>&1 | head -3
fi
sleep 2
if ! gui_alive >/dev/null; then echo "GUI DIED at restart — shape impossible"; exit 9; fi

# ---- phase 3: observe + verdict ----
SCRATCH=$SCRATCH BIN=$BIN TEST=$TEST T_RESTART_MS=$T_RESTART_MS RT_FAKE_ID=${RT_FAKE_ID:-0} python3 - <<'PYEOF' >> /tmp/rt-run-$(basename $SCRATCH).log 2>&1
import json, subprocess, time, os
from collections import Counter

BIN = os.environ["BIN"]; TEST = os.environ["TEST"]
TRACE = os.environ["SCRATCH"] + "/event-trace.jsonl"
T_RESTART = int(os.environ["T_RESTART_MS"])
RT_FAKE_ID = os.environ.get("RT_FAKE_ID", "0") == "1"

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

def daemon_screen(path):
    r = verb("server", "screen", path, timeout=30)
    return r.stdout or ""

def client_buffer(path):
    r = verb("server", "app", "terminal", "read-buffer", path, "--mode", "screen", timeout=30)
    return (r.stdout or "") + (("\n[stderr] " + r.stderr.strip()) if r.returncode != 0 else "")

# the new runtime must CONSUME input: type RTNEW, verify on daemon screen
verb("server", "app", "terminal", "probe-type", TEST, "--data", "echo RTNEW", "--enter", "--mode", "xterm", timeout=30)
daemon_new = 0
deadline = time.time() + 25
while time.time() < deadline:
    daemon_new = daemon_screen(TEST).count("RTNEW")
    if daemon_new >= 1:
        break
    time.sleep(0.5)
print("post-restart: typed RTNEW on daemon screen=%d" % daemon_new)
if daemon_new == 0:
    # retry once — a just-restarted shell can drop the first probe
    verb("server", "app", "terminal", "probe-type", TEST, "--data", "echo RTNEW", "--enter", "--mode", "xterm", timeout=30)
    deadline = time.time() + 25
    while time.time() < deadline:
        daemon_new = daemon_screen(TEST).count("RTNEW")
        if daemon_new >= 1:
            break
        time.sleep(0.5)
    print("post-restart retry: RTNEW on daemon=%d" % daemon_new)
if daemon_new == 0:
    print("RIG INCONCLUSIVE rc=6 — the new runtime never consumed input; restart itself failed")
    raise SystemExit(6)

# observe the client 16 x 5s = 80s
samples_new = []; samples_old = []
for i in range(16):
    body = client_buffer(TEST)
    samples_new.append(body.count("RTNEW"))
    samples_old.append(body.count("RTOLD"))
    time.sleep(5)
client_new = max(samples_new) if samples_new else 0
client_old_final = samples_old[-1] if samples_old else 0
print("observe: client RTNEW samples=%s max=%d | RTOLD final=%d"
      % (samples_new, client_new, client_old_final))

# grid evidence (the (b) wild signature: daemon frame wide, client host narrow)
def max_line_len(text):
    return max((len(l) for l in text.splitlines()), default=0)
ds_final = daemon_screen(TEST)
print("grid: daemon max_line_len=%d client max_line_len=%d"
      % (max_line_len(ds_final), max_line_len(client_buffer(TEST))))

# trace verdict over the post-restart window
events = [(ts, comp, nm, p) for ts, comp, nm, p in read_events() if ts >= T_RESTART - 250]

# exe-proof ([11.231] law): the register event fires at GUI BOOT — scan the
# WHOLE trace for the LAST register, never the windowed slice.
all_events = read_events()
reg = [p for ts, comp, nm, p in all_events if nm == "register"]
exe = (reg[-1].get("executable_path") if reg else "") or ""
want = os.path.realpath(BIN)
got = os.path.realpath(exe) if exe else ""
print("exe-proof: served=%s want=%s" % (exe, want))
if got != want:
    print("EXE MISMATCH — the rig measured a different build than it launched")
    raise SystemExit(8)

test_events = [(ts, nm, p) for ts, comp, nm, p in events if p.get("session_path") == TEST]
hist = Counter(nm for ts, nm, p in test_events)
print("post-restart event histogram for test row (%d events):" % len(test_events))
for nm, c in hist.most_common(24):
    print("  %4d  %s" % (c, nm))

rewound = [(ts, p) for ts, nm, p in test_events if nm == "terminal_stream_cursor_rewound"]
replaced = [(ts, p) for ts, nm, p in test_events if nm == "terminal_stream_runtime_replaced"]
mount_cycle = [(ts, nm) for ts, nm, p in test_events
               if nm in ("synthesized_mount_open", "attach_ready", "first_meaningful_output",
                         "mount_epoch_bumped", "mount_epoch_reused")]
restart_events = [(ts, nm) for ts, comp, nm, p in events
                  if nm in ("restart", "restart_before_read") and p.get("path") == TEST]
recovery = bool(rewound or mount_cycle)
print("VERDICT inputs: daemon_new=%d client_new=%d rewound=%d replaced=%d mount_cycle=%d daemon_restart_events=%d"
      % (daemon_new, client_new, len(rewound), len(replaced), len(mount_cycle), len(restart_events)))

if client_new >= 1:
    if RT_FAKE_ID and not replaced:
        print("RIG INCONCLUSIVE rc=6 — fake-id arm painted but NO terminal_stream_runtime_replaced"
              " event: the hook did not reach the daemon (env/file wrong) — not a verdict")
        raise SystemExit(6)
    print("RIG GREEN: the mounted client bound the replaced runtime (RTNEW on client buffer)")
    raise SystemExit(0)
if RT_FAKE_ID and replaced:
    # the signal FIRED but the client never painted — the recovery arm broke
    print("VERDICT RED-B CONFIRMED (rc 4): runtime_replaced fired but the client never painted"
          " the post-flip output — the recovery path is broken/gated")
    raise SystemExit(4)
if not recovery:
    print("VERDICT RED-A CONFIRMED (rc 5): frozen with NO recovery signal — the stream lacks"
          " runtime identity; add runtime_spawn_id to TerminalStream + client invalidation")
    raise SystemExit(5)
print("VERDICT RED-B CONFIRMED (rc 4): recovery fired (%s) but the client stayed frozen —"
      " the recovery path is broken/gated; instrument it" % ([nm for _, nm in mount_cycle[:3]] or ["cursor_rewound"]))
raise SystemExit(4)
PYEOF
RC=$?
echo "rt-rig done rc=$RC"
exit "$RC"
