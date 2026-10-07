#!/bin/bash
# [F1 r-j1/r-j2] THE VERIFY-OR-REARM RIG (rj2-rig) — the falsifier for the
# post-kill successor strand: after a GENUINE dead-loop recovery, the async
# watch's latch-equality never matched the combined key (`identity:request`)
# so the latch stayed set (no successor bootstrap), and should_recover's
# stable-host gate (retained host + non-empty surface, attach torn down by
# the recovery) refused forever — a USER RE-SELECT was the only re-trigger.
#   SHAPE: boot-1 fresh scratch GUI with REAL IPC; ONE shell row; painted
#   gate; kill ONLY the GUI (environ-matched, daemon spared); boot-2 over
#   the live daemon WITH YGGTERM_TEST_SILENT_LOOP_DEATHS_FILE armed
#   ("\n<count>:<delay_ms>") — the restored row's mount loops die
#   deterministically (silent bare-break death, the creation-churn shape);
#   the fix must re-arm (latch prefix clear + the late verify-or-rearm
#   watch) so a fresh healthy loop mounts WITHOUT any user re-select.
#   GREEN (rc 0): typed RJ2END delivered on the daemon screen after the
#     deaths (a live loop drains input — screen is ground truth, never
#     probe-type's accepted field).
#   RED (rc 5): deaths engaged (>=1 drop + >=1 recovery) and the typed
#     marker never lands — the row is stranded corpse-shaped.
#   2=rows/boot, 3=boot-1 painted gate, 4=hook not engaged (no drops),
#   8=exe mismatch, 9=daemon died with the GUI.
# Usage: tools/uxspeed/rj2-rig.sh <binary> [deaths-count] [death-delay-ms]
#   (absolute binary path; full-workspace release build per rig law L1.)
#   ⛔ trace law: whole-buffer raw_decode, never line-parse. Never trust
#   `terminal new`'s rc — verify via `server snapshot` live_sessions.
set -u
export TERM=xterm-256color
BIN=${1:?usage: rj2-rig.sh <binary> [count] [delay_ms]}
DEATHS=${2:-2}
DEATH_MS=${3:-3000}
SCRATCH=/tmp/rj2-home-$(date +%s)-$$
export BIN SCRATCH
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN"; exit 2; fi

run_boot() {  # $1=hook(0|1) -> boots GUI, echoes wrap pid
  mkdir -p "$SCRATCH"
  export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
  local HOOK_EXTRA=""
  [ "${1:-0}" = "1" ] && HOOK_EXTRA="export YGGTERM_TEST_SILENT_LOOP_DEATHS_FILE=$SCRATCH/silent-deaths"
  dbus-run-session -- bash -c "
    export DISPLAY=:77 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
    export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
    $HOOK_EXTRA
    exec $BIN > /tmp/rj2-gui.log 2>&1
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
  case "$c" in *":77"*) kill "$p";; esac
done 2>/dev/null; sleep 1
Xvfb :77 -screen 0 1600x1000x24 -ac > /tmp/rj2-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5

# ---- boot 1 (healthy, no hook) ----
run_boot 0
GUI_WRAP=$!
READY=$(boot_wait_ready); echo "boot1 daemon ready=$READY"

SCRATCH=$SCRATCH BIN=$BIN python3 - <<'PYEOF' > /tmp/rj2-run.log 2>&1
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

TEST = create_row_titled("rj2test1")
print("row: test=%s" % (TEST,))
if not TEST:
    print("FAIL: could not create row"); raise SystemExit(2)
open("/tmp/rj2-test-path", "w").write(TEST)

# mounted + painted gate before the kill (screen readback, ground truth)
painted = 0
for attempt in range(3):
    verb("server", "app", "terminal", "probe-type", TEST, "--data", "echo RJ2OK", "--enter", "--mode", "xterm")
    deadline = time.time() + 25
    while time.time() < deadline:
        body = verb("server", "screen", TEST, timeout=30).stdout or ""
        if body.count("RJ2OK") >= 1:
            painted = body.count("RJ2OK"); break
        time.sleep(0.5)
    if painted:
        break
    time.sleep(5)
print("boot1: RJ2OK painted=%d" % painted)
if not painted:
    print("BOOT-1 GATE FAIL: fresh row never painted — pre-flight broken"); raise SystemExit(3)
raise SystemExit(0)
PYEOF
RC=$?
if [ "$RC" -ne 0 ]; then
  echo "boot1 phase rc=$RC"; kill "$GUI_WRAP" 2>/dev/null
  for p in $(scratch_pids_by_name yggterm); do kill "$p" 2>/dev/null; done
  kill "$XVFB_PID" 2>/dev/null; exit "$RC"
fi
TEST=$(cat /tmp/rj2-test-path)
echo "boot1 ok, test=$TEST"

# ---- kill ONLY the GUI; the daemon must survive ----
sleep 3
kill_gui_only
sleep 2
if daemon_alive >/dev/null; then
  daemon_alive
else
  echo "DAEMON DIED WITH THE GUI — relaunch shape impossible (rc 9)"
  kill "$XVFB_PID" 2>/dev/null; exit 9
fi

# ---- arm the deaths file, boot 2 over the live daemon WITH the hook ----
printf '\n%s:%s\n' "$DEATHS" "$DEATH_MS" > "$SCRATCH/silent-deaths"
echo "deaths file: $(cat "$SCRATCH/silent-deaths" | tr '\n' ' ') (left=$DEATHS delay=$DEATH_MS)"
T_BOOT2_MS=$(date +%s%3N)
run_boot 1
GUI_WRAP=$!
READY=$(boot_wait_ready); echo "boot2 gui ready=$READY"

# ---- observe + verdict ----
SCRATCH=$SCRATCH BIN=$BIN TEST=$TEST T_BOOT2_MS=$T_BOOT2_MS python3 - <<'PYEOF' >> /tmp/rj2-run.log 2>&1
import json, subprocess, time, os

BIN = os.environ["BIN"]; TEST = os.environ["TEST"]
TRACE = os.environ["SCRATCH"] + "/event-trace.jsonl"
T_BOOT2 = int(os.environ["T_BOOT2_MS"])

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

def daemon_screen(path):
    r = verb("server", "screen", path, timeout=30)
    return r.stdout or ""

def sess(events, name):
    return [(ts, p) for ts, nm, p in events
            if nm == name and (p.get("session_path") == TEST or TEST in str(p.get("session_path", "")))]

# settle: deaths at ~+3s and ~+8s of their loop starts; recoveries from
# +5.25s; the rearm watch at +7.5s past its recovery; healthy loop by ~+16s.
time.sleep(50)

events = read_events()
drops = sess(events, "terminal_mount_task_dropped")
recovers = (sess(events, "startup_terminal_restore_recover")
            + sess(events, "startup_terminal_restore_recover_watch"))
rearms = sess(events, "startup_restore_rearm_no_successor")
deferred = [(ts, p) for ts, nm, p in events
            if nm == "startup_restore_recover_deferred_live_loop"
            and TEST in str(p.get("session_path", ""))]
refused = [(ts, p) for ts, nm, p in events
           if nm == "reveal_raise_refused" and TEST in str(p.get("session_path", ""))]
attached = (sess(events, "attach_ready") + sess(events, "first_meaningful_output"))
left = open(os.environ["SCRATCH"] + "/silent-deaths").read().strip() if __import__("os").path.exists(os.environ["SCRATCH"] + "/silent-deaths") else "(gone)"
print("boot2 trace: drops=%d recoveries=%d deferred_live=%d refused_raise=%d attach/first_output=%d rearms=%d deaths-file-now=%r" % (
    len(drops), len(recovers), len(deferred), len(refused), len(attached), len(rearms), left))
for ts, p in drops[:6]:
    print("  drop ts=%d remount_armed=%s exit_hint=%s" % (ts, p.get("remount_armed"), p.get("exit_hint")))
for ts, p in recovers[:6]:
    print("  recovery ts=%d" % ts)
for ts, p in rearms[:6]:
    print("  REARM ts=%d" % ts)

# THE BAR: typed input on the restored row — screen ground truth
verb("server", "app", "terminal", "probe-type", TEST, "--data", "echo RJ2END", "--enter", "--mode", "xterm", timeout=30)
delivered = 0
deadline = time.time() + 30
while time.time() < deadline:
    ds = daemon_screen(TEST)
    if ds.count("RJ2END") >= 1:
        delivered = ds.count("RJ2END"); break
    time.sleep(0.5)
print("boot2: typed RJ2END delivered=%d" % delivered)

if len(drops) == 0:
    print("VERDICT HOOK: NOT ENGAGED — no drops traced; rig invalid"); raise SystemExit(4)
if delivered:
    print("VERDICT BAR: GREEN — the row re-armed itself after the deaths and delivers input (rearms=%d)" % len(rearms))
    raise SystemExit(0)
if len(drops) >= 1 and len(attached) >= 1:
    print("VERDICT BAR: RED — the row MOUNTED (attach/first_output=%d) then its loop/loops died (%d drops) and typing NEVER delivered: the post-death strand (recoveries=%d)" % (len(attached), len(drops), len(recovers)))
    raise SystemExit(5)
print("VERDICT AMBIGUOUS: deaths=%d attached=%d delivered=%d — inspect trace" % (len(drops), len(attached), delivered)); raise SystemExit(6)
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
if [ "$EXE" = "$BIN" ]; then echo "EXE OK: requested binary served this run"; else echo "EXE MISMATCH: verdicts above are VOID"; RC=8; fi
echo "=== DRIVER OUTPUT ==="
tail -45 /tmp/rj2-run.log
echo "scratch=$SCRATCH rc=$RC"
exit $RC
