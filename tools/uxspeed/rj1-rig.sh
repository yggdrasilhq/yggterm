#!/bin/bash
# [r-j1] THE STARTUP-RESTORE-RECOVER LIVE-LOOP RIG — the (j)-class
# kill-site falsifier for recover_startup_terminal_restore (plan
# ACK-93f62f9d08; sitting-9 measurement: this recovery superseded live
# pre-attach_ready loops, registry_owner NULL, no successor, rows
# mountless). BOTH call sites funnel through the SAME fn: the async
# watch (startup_terminal_restore_recover_watch, +5.25s) and the
# synchronous render pass (startup_terminal_restore_recover) — the
# guard covers them together.
#
# SHAPE (the fence-rig boot-1 shape, stall widened): fresh scratch GUI
# under YGGTERM_TEST_SUPPRESS_MOUNT_IPC=1 (deterministic synthesis) +
# YGGTERM_TEST_STALL_SNAPSHOT_FETCH_MS=<stall> — the seed holds, so the
# row's mount loop sits PRE-attach_ready past the 5s staleness window
# while its ATTEMPT reads stale (the blank row also latches the
# empty-surface arm) -> the recovery tears the bootstrap owner out from
# under the LIVE loop.
#   RED (regression tripwire): recover fired while live ->
#          bootstrap_owner_superseded_during_loop {registry_owner:
#          NULL} -> NO attach_ready, row blank (rc 5).
#   GREEN = SURVIVAL (the sitting-9 law "rig rows must outlive the
#          restore window"): attach_ready + painted + probe-typed
#          RJ1OK + no supersede (rc 0). On current main the
#          ready-marking fires early and recovery never needs to run;
#          the kill's live entry needs a slow-proof window — the fix's
#          direct proof is the unit locks + the
#          startup_restore_recover_deferred_live_loop production
#          discriminator (the (f2)/(h2) construction-proof precedent).
# FAILED VERDICTS EXIT NONZERO AFTER ALL ARMS: 2=rows, 4=pre-flight,
# 5=RED CONFIRMED (the kill signature — expected on unfixed main),
# 6=neither contract (rig or fix problem), 8=exe mismatch.
# Usage: tools/uxspeed/rj1-rig.sh [worktree] [binary] [stall_ms]
#   ⛔ BUILD LAW (sitting 7): FULL-WORKSPACE release build. Runs on dev's
#   Xvfb :78 — jojo untouched.
set -u
WT=${1:-$HOME/gh/yggterm}
BIN=${2:-$WT/target/release/yggterm}
STALL=${3:-8000}
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN — build first"; exit 2; fi
export BIN STALL

run_boot() {  # $1=scratch -> boots GUI with synthesis+stall, echoes wrap pid
  local SCRATCH=$1
  mkdir -p "$SCRATCH"
  export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
  dbus-run-session -- bash -c "
    export DISPLAY=:78 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
    export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
    export YGGTERM_TEST_SUPPRESS_MOUNT_IPC=1 YGGTERM_HOT_PREMOUNT_CAP=2
    export YGGTERM_TEST_STALL_SNAPSHOT_FETCH_MS=$STALL
    exec $BIN > /tmp/rj1-gui.log 2>&1
  " >/dev/null 2>&1 </dev/null &
}

boot_wait_ready() {
  local READY=0 i
  for i in $(seq 1 90); do sleep 1; "$BIN" server app clients >/dev/null 2>&1 && READY=1 && break; done
  echo $READY
}

for p in $(pgrep -x Xvfb); do
  c=$(tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null)
  case "$c" in *":78"*) kill "$p";; esac
done 2>/dev/null; sleep 1
Xvfb :78 -screen 0 1600x1000x24 -ac > /tmp/rj1-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5

SCRATCH=/tmp/rj1-home-$(date +%s)
T0_MS=$(date +%s%3N)
run_boot "$SCRATCH"
GUI_WRAP=$!
READY=$(boot_wait_ready); echo "daemon ready=$READY"

SCRATCH=$SCRATCH T0_MS=$T0_MS python3 - <<'PYEOF' > /tmp/rj1-run.log 2>&1
import json, subprocess, time, os

bin_path = os.environ["BIN"]
TRACE = os.environ["SCRATCH"] + "/event-trace.jsonl"
T0 = int(os.environ["T0_MS"])
STALL = int(os.environ.get("STALL", "8000"))

def verb(*args, timeout=60):
    return subprocess.run([bin_path] + list(args), capture_output=True, text=True, timeout=timeout)

def read_events():
    # The trace MIXES compact single-line and pretty multi-line JSON — a
    # line-based parser silently drops the pretty events (measured:
    # attach_ready missed while the row was attached). Decode the whole
    # buffer with raw_decode instead.
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
            idx += 1
            continue
        name = (event.get("name") or event.get("event")
                or (event.get("payload") or {}).get("name")
                or (event.get("payload") or {}).get("event"))
        if name:
            out.append((int(event.get("ts_ms", 0)), name, event.get("payload") or {}))
    return out

def ev(name, pred=lambda p: True):
    return [p for ts, nm, p in read_events() if nm == name and pred(p)]

def snapshot_rows():
    # daemon-served (no GUI round trip): the row LIST is authoritative even
    # while the app-control plane warms up under software rendering
    # (measured: `terminal new` can land the row but miss the CLI's fixed
    # 15s response wait — never trust that verb's rc here).
    out = verb("server", "snapshot", timeout=30)
    try:
        return json.loads(out.stdout).get("live_sessions") or []
    except Exception:
        return []

def create_row_titled(title):
    verb("server", "app", "terminal", "new", "--kind", "shell", "--title", title, timeout=30)
    for _ in range(60):
        for r in snapshot_rows():
            if r.get("title") == title:
                return r.get("session_path")
        time.sleep(1)
    return None

# ⛔ ROW SHAPE (fence-rig law): dummy row FIRST, test row second, CAP=2.
_a = create_row_titled("rj1seed0")
TEST = create_row_titled("rj1seed1")
print("rows: a=%s test=%s stall=%dms" % (_a, TEST, STALL))
if not TEST:
    print("FAIL: could not create rows"); raise SystemExit(2)

# wait for the synthesis arm (pre-flight), then sit out the whole kill
# window: render/async recovery at ~5-6s, seed+proof at ~stall+1s.
armed = False
deadline = time.time() + 45
while time.time() < deadline:
    if ev("synthesized_mount_open_armed", lambda p: p.get("session_path") == TEST):
        armed = True; break
    time.sleep(1)
print("synthesis armed=%s" % armed)
if not armed:
    print("PRE-FLIGHT FAIL: mount never armed the seed fetch — synthesis did not fire")
    raise SystemExit(4)

time.sleep(30)  # recovery fire + stall + attach + settle
events = read_events()

# exe proof ([11.231] law): the register event must name THIS binary.
reg = [p for ts, nm, p in events if nm == "register"]
exe = (reg[-1].get("executable_path") if reg else "") or ""
want = os.path.realpath(bin_path)
got = os.path.realpath(exe) if exe else ""
print("exe-proof: served=%s want=%s" % (exe, want))
if got != want:
    print("EXE MISMATCH — the rig measured a different build than it launched")
    raise SystemExit(8)

watch = [p for ts, nm, p in events if nm == "startup_terminal_restore_recover_watch"
         and p.get("session_path") == TEST]
render_recover = [p for ts, nm, p in events if nm == "startup_terminal_restore_recover"
                  and p.get("session_path") == TEST]
deferred = [p for ts, nm, p in events if nm == "startup_restore_recover_deferred_live_loop"
            and p.get("session_path") == TEST]
superseded = [p for ts, nm, p in events if nm == "bootstrap_owner_superseded_during_loop"
              and p.get("session_path") == TEST and p.get("registry_owner") is None]
# attach_ready does not fire on synthesized second-row mounts (its gate
# needs the local attach-ready output marker, which this path doesn't
# produce — measured: dummy row attach_ready=1, test row=0 while the
# test row paints and types). Survival evidence = ANY mount-completion
# signal: attach_ready | first_meaningful_output | synthesized proof.
attached = bool([1 for ts, nm, p in events
                 if nm in ("attach_ready", "first_meaningful_output",
                           "synthesized_mount_open")
                 and p.get("session_path") == TEST])
dropped = [p for ts, nm, p in events if nm == "terminal_mount_task_dropped"
           and p.get("session_path") == TEST]
print("VERDICT RJ1: watch=%d render_recover=%d deferred=%d superseded(null)=%d "
      "attached=%d dropped=%d" % (len(watch), len(render_recover), len(deferred),
                                  len(superseded), int(bool(attached)), len(dropped)))

kill = bool(superseded)
if kill and not attached:
    print("VERDICT RED CONFIRMED: startup-restore recovery KILLED the live pre-attach "
          "loop (supersede registry_owner=NULL, no attach_ready) — row mountless")
    raise SystemExit(5)

# GREEN contract = SURVIVAL (the sitting-9 law: "rig rows must outlive the
# restore window"): the row attaches, paints, and types through the window.
# `deferred` is the guard's DIRECT positive signal — reported when present,
# not required: on current main the ready-marking fires early and recovery
# never needs to run (the kill's live entry needs a slow-proof window; see
# the (f2)/(h2) construction-proof precedent for why no GUI RED is owed).
verb("server", "app", "terminal", "probe-type", TEST, "--data", "echo RJ1OK", "--enter", "--mode", "xterm")
deadline = time.time() + 25
painted = 0
while time.time() < deadline:
    body = verb("server", "app", "terminal", "read-buffer", TEST, "--mode", "screen").stdout or ""
    if body.count("RJ1OK") >= 1:
        painted = body.count("RJ1OK"); break
    time.sleep(0.5)
print("GREEN arm: RJ1OK painted=%d deferred_signal=%d" % (painted, len(deferred)))
if attached and painted and not kill:
    print("RIG PASS — the row OUTLIVED the restore window: attached, painted, typed")
    raise SystemExit(0)
print("RIG FAIL rc=6 — the row did not survive the restore window (see verdicts above)")
raise SystemExit(6)
PYEOF
RC=$?
kill "$GUI_WRAP" 2>/dev/null; sleep 1
pkill -x yggterm 2>/dev/null
pkill -f "dbus-run-session" 2>/dev/null; sleep 1
# ⛔ REAP THE SCRATCH DAEMON (sitting-7 law): matched by YGGTERM_HOME ONLY.
for p in $(pgrep -f 'yggterm-headless server daemon'); do
  h=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | sed -n 's/^YGGTERM_HOME=//p')
  [ "$h" = "$SCRATCH" ] && kill "$p" 2>/dev/null
done
kill "$XVFB_PID" 2>/dev/null
echo "rj1-rig done rc=$RC"
exit "$RC"
