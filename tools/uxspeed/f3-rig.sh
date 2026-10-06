#!/bin/bash
# [F3] THE GUI-RELAUNCH-CHURN RIG — the [11.229](b) replaced-runtime
# stage-post-death reproducer (plan ACK-18d44cc4a0; sitting-12 rj1-rig
# iteration measured the signature: GUI relaunch over a LIVE daemon ->
# the retained host never posts its page stage, reveal_raise_refused
# daemon_owns_runtime:True, no synthesis arming, while the render-site
# recovery re-drives every 5s and burns the futile streak — rows churn
# mountless: runtime alive on the daemon, client never re-binds).
#
# SHAPE: boot-1 fresh scratch GUI with REAL IPC (no suppression, no
# stall); dummy+test rows (CAP=2 shape, dummy FIRST); probe-type F3OK +
# painted gate; kill ONLY the GUI (environ-matched, daemon spared);
# boot-2 over the same daemon; verdict from the post-kill trace window
# (epoch ts_ms — trace.rs amortized_unix_ms) + client buffer vs daemon
# screen.
#   RED CONFIRMED (rc 5): post-kill window shows the churn signature
#     (startup_terminal_restore_recover >=2 OR reveal_raise_refused with
#     daemon_owns_runtime:true) AND zero stage="posted" readings AND zero
#     attach signals AND daemon screen carries F3OK while the client
#     buffer does not.
#   GREEN (rc 0): the retained row re-binds (stage posted / attach
#     signal) and F3OK reads back at the CLIENT buffer.
#   2=rows, 3=boot-1 painted gate, 4=pre-flight, 6=neither contract,
#   8=exe mismatch, 9=daemon died with the GUI (shape impossible).
# Usage: tools/uxspeed/f3-rig.sh [binary] (F3_BOOT2_SUPPRESS=1 for the shed variant; dev Xvfb :78)
#   ⛔ trace law: the file mixes compact+pretty JSON — whole-buffer
#   raw_decode, never line-parse. Never trust `terminal new`'s rc —
#   verify via `server snapshot` live_sessions.
set -u
export TERM=xterm-256color
BIN=${BIN:-$HOME/.local/bin/yggterm}
SCRATCH=/tmp/f3-home-$(date +%s)-$$
export BIN SCRATCH
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN"; exit 2; fi

run_boot() {  # $1=scratch $2=suppress(0|1) -> boots GUI, echoes wrap pid
  mkdir -p "$SCRATCH"
  export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
  local SUP_EXTRA=""
  [ "${2:-0}" = "1" ] && SUP_EXTRA="export YGGTERM_TEST_SUPPRESS_MOUNT_IPC=1"
  dbus-run-session -- bash -c "
    export DISPLAY=:78 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
    export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
    export YGGTERM_HOT_PREMOUNT_CAP=2
    $SUP_EXTRA
    exec $BIN > /tmp/f3-gui.log 2>&1
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

kill_gui_only() {
  local p cmd
  for p in $(scratch_pids_by_name yggterm); do
    cmd=$(tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null)
    case "$cmd" in *"server daemon"*) continue;; esac
    kill "$p" 2>/dev/null && echo "killed GUI pid $p"
  done
}

daemon_alive() {
  local p cmd h
  for p in $(pgrep -f 'server daemon' 2>/dev/null); do
    [ "$p" = "$$" ] && continue
    cmd=$(tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null) || continue
    case "$cmd" in *yggterm*server*daemon*) ;; *) continue;; esac
    h=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | sed -n 's/^YGGTERM_HOME=//p')
    if [ "$h" = "$SCRATCH" ]; then echo "daemon pid $p"; return 0; fi
  done
  return 1
}

# ---- display ----
for p in $(pgrep -x Xvfb); do
  c=$(tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null)
  case "$c" in *":78"*) kill "$p";; esac
done 2>/dev/null; sleep 1
Xvfb :78 -screen 0 1600x1000x24 -ac > /tmp/f3-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5

# ---- boot 1 ----
run_boot "$SCRATCH"
GUI_WRAP=$!
READY=$(boot_wait_ready); echo "boot1 daemon ready=$READY"

SCRATCH=$SCRATCH BIN=$BIN python3 - <<'PYEOF' > /tmp/f3-run.log 2>&1
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

_a = create_row_titled("f3seed0")
TEST = create_row_titled("f3seed1")
print("rows: a=%s test=%s" % (_a, TEST))
if not TEST:
    print("FAIL: could not create rows"); raise SystemExit(2)
open("/tmp/f3-test-path", "w").write(TEST)

# wait for the row's MOUNT before typing (gotcha: send on an unmounted
# row is handled but NOT delivered — probe first, then verify by screen)
mounted_prompt = False
deadline = time.time() + 90
while time.time() < deadline:
    body = verb("server", "app", "terminal", "read-buffer", TEST, "--mode", "screen").stdout or ""
    if len(body.strip()) > 40:
        mounted_prompt = True; break
    time.sleep(1)
print("boot1: prompt mounted=%s (len=%d)" % (mounted_prompt, len(body.strip())))
painted = 0
for attempt in range(3):
    verb("server", "app", "terminal", "probe-type", TEST, "--data", "echo F3OK", "--enter", "--mode", "xterm")
    deadline = time.time() + 20
    while time.time() < deadline:
        body = verb("server", "app", "terminal", "read-buffer", TEST, "--mode", "screen").stdout or ""
        if body.count("F3OK") >= 1:
            painted = body.count("F3OK"); break
        time.sleep(0.5)
    if painted:
        break
    time.sleep(5)
print("boot1: F3OK painted=%d" % painted)
if not painted:
    print("BOOT-1 GATE FAIL: fresh rows never painted — pre-flight broken"); raise SystemExit(3)
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
TEST=$(cat /tmp/f3-test-path)
echo "boot1 ok, test=$TEST"

# ---- kill ONLY the GUI; the daemon must survive ----
sleep 3   # let boot-1 settle fully past the restore window
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

# ---- boot 2 over the live daemon (SUPPRESS=${F3_BOOT2_SUPPRESS:-0}: 1 =
# relaunch inside the F1 shed-window proxy, the sitting-12 iteration env) ----
run_boot "$SCRATCH" "${F3_BOOT2_SUPPRESS:-0}"
GUI_WRAP=$!
READY=$(boot_wait_ready); echo "boot2 gui ready=$READY"

# ---- observe + verdict ----
SCRATCH=$SCRATCH BIN=$BIN TEST=$TEST T_KILL_MS=$T_KILL_MS python3 - <<'PYEOF' >> /tmp/f3-run.log 2>&1
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

def read_buffer(path):
    r = verb("server", "app", "terminal", "read-buffer", path, "--mode", "screen", timeout=30)
    return (r.stdout or "") + (("\n[stderr] " + r.stderr.strip()) if r.returncode != 0 else "")

def daemon_screen(path):
    r = verb("server", "screen", path, timeout=30)
    return r.stdout or ""

# observe: poll the client buffer 16 x 5s = 80s
samples = []
for i in range(16):
    body = read_buffer(TEST)
    samples.append(body.count("F3OK"))
    time.sleep(5)
client_final = samples[-1] if samples else 0
client_max = max(samples) if samples else 0
unmounted = all(s == 0 for s in samples)
print("boot2 observe: client F3OK samples=%s max=%d final=%d" % (samples, client_max, client_final))
ds = daemon_screen(TEST)
daemon_has = ds.count("F3OK")
print("boot2: daemon screen F3OK=%d (client buffer %s)" % (daemon_has, "BLANK" if client_final == 0 else "carries"))

# input-delivery proof: type a fresh marker on the restored row
verb("server", "app", "terminal", "probe-type", TEST, "--data", "echo F3B2", "--enter", "--mode", "xterm", timeout=30)
typed = 0
deadline = time.time() + 25
while time.time() < deadline:
    ds2 = daemon_screen(TEST)
    if ds2.count("F3B2") >= 1:
        typed = ds2.count("F3B2"); break
    time.sleep(0.5)
print("boot2: typed F3B2 delivered=%d" % typed)

# trace verdict over the post-kill window
events = [(ts, comp, nm, p) for ts, comp, nm, p in read_events() if ts >= T_KILL - 250]

# exe proof ([11.231] law): the register event must name THIS binary.
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
for nm, c in hist.most_common():
    print("  %4d  %s" % (c, nm))

recover = [(ts, p) for ts, nm, p in test_events if nm == "startup_terminal_restore_recover"]
refused = [p for ts, nm, p in test_events if nm == "reveal_raise_refused"]
refused_daemon = [p for p in refused if p.get("daemon_owns_runtime") is True]
stage_posted = [p for ts, nm, p in test_events if p.get("stage") == "posted"]
attach = [nm for ts, nm, p in test_events
          if nm in ("attach_ready", "first_meaningful_output", "synthesized_mount_open",
                    "synthesized_mount_open_armed")]
if len(recover) >= 2:
    deltas = [recover[i+1][0] - recover[i][0] for i in range(len(recover)-1)]
    print("recover cadence deltas_ms=%s" % deltas)
for p in refused[:3]:
    print("refused payload: has_host=%s was_ever_ready=%s daemon_owns=%s remote_read_age_ms=%s degraded=%s"
          % (p.get("has_host_epoch"), p.get("was_ever_ready"), p.get("daemon_owns_runtime"),
             p.get("remote_read_age_ms"), p.get("transport_degraded")))
print("VERDICT F3: recover=%d refused=%d(daemon_owns=%d) stage_posted=%d attach_signals=%d daemon_has=%d client_has=%d"
      % (len(recover), len(refused), len(refused_daemon), len(stage_posted), len(attach), daemon_has, client_final))

churn = len(recover) >= 2 or len(refused_daemon) >= 1
if churn and not stage_posted and not attach and daemon_has and client_final == 0:
    print("VERDICT RED CONFIRMED: relaunch churn — retained row mountless (stage never posted, no attach, daemon alive, client blank)")
    raise SystemExit(5)
if attach and typed:
    print("RIG GREEN: retained row re-bound after relaunch (attach signal %s + typed input delivered)" % attach[:2])
    raise SystemExit(0)
print("RIG INCONCLUSIVE rc=6 — neither contract; see histogram above")
raise SystemExit(6)
PYEOF
RC=$?

kill "$GUI_WRAP" 2>/dev/null; sleep 1
for p in $(scratch_pids_by_name yggterm); do
  cmd=$(tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null)
  case "$cmd" in *"server daemon"*) ;; *) kill "$p" 2>/dev/null;; esac
done
pkill -f "dbus-run-session" 2>/dev/null; sleep 1
for p in $(pgrep -f 'yggterm-headless server daemon' 2>/dev/null); do
  h=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | sed -n 's/^YGGTERM_HOME=//p')
  [ "$h" = "$SCRATCH" ] && kill "$p" 2>/dev/null
done
kill "$XVFB_PID" 2>/dev/null
echo "f3-rig done rc=$RC"
exit "$RC"
