#!/bin/bash
# [F1-(i)/(i-r1)] THE EXIT-ARM NAMING RIG (exitarm-rig) — the falsifier for
# "exit_hint bottoms out at pre_select": a mount-loop select arm without a
# TerminalLoopBranchGuard leaves the drop witness's exit_hint stale, so a
# death mid-arm-body masquerades as a death at the await.
#   MODE=aimed (THE BAR, default): fresh scratch GUI (hermetic XDG +
#   exe-proof, the [11.231] law) on the HEALTHY bridge path; two rows; the
#   active row is typed + painted (healthy-mount proof); the silent-death
#   token carries an optional "@<branch>" aim — the loop-top death fires
#   only after the aimed arm's guard stamped. Default aim read_poll_start
#   (fires naturally on every read deadline of a live row).
#   RED (rc 5, DEGENERATION BAR): the deaths file's write-back lost the
#   suffix — the build's hook cannot AIM a death (pre-fix parsers eat the
#   suffix and default the delay; an uninstrumented arm never stamps a
#   name to aim at). GREEN (rc 0): suffix survived + typed marker painted +
#   a drop's exit_hint == the armed branch exactly.
#   MODE=close (the measurement arm, NOT a RED/GREEN bar): switch ACTIVE to
#   row a, remove it, read the post-remove drop's exit_hint. MEASURED
#   2026-10-10 on BOTH the unfixed and the fixed build: the drop reads
#   pre_select — session-remove CANCELS the mount task at the select await;
#   it does NOT flow through the eval arm. On the fixed build that reading
#   is CONCLUSIVE (every arm stamps; the pre-select body has zero awaits).
#   rc 0 = a post-remove drop captured (the hint is printed as the
#   measurement); rc 5 = the drop names a NON-pre_select arm (a genuinely
#   new finding — the bridge arm ran); rc 4 = no drop.
#   ⛔ (measured 2026-10-10): the write arm fires only for remote-resume
#   rows (track_completion) or post-failure recovery — plain local typing
#   never emits TerminalWriteEvent::Completed; do not aim at write_* on
#   local rigs. Aim at naturally-firing arms (read_poll_start et al.).
#   4=hook not engaged (no drops), 2=row/boot failure, 3=marker never
#   painted (preflight), 8=exe mismatch.
# Usage: tools/uxspeed/exitarm-rig.sh <binary> [armed-branch] [close|aimed]
#   (absolute binary path; full-workspace release build per rig law L1.)
#   ⛔ trace law: whole-buffer raw_decode, never line-parse.
set -u
export TERM=xterm-256color
BIN=${1:?usage: exitarm-rig.sh <binary> [armed-branch] [close|aimed]}
AIM=${2:-read_poll_start}
MODE=${3:-aimed}
SCRATCH=/tmp/exitarm-home-$(date +%s)-$$
export BIN SCRATCH
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN"; exit 2; fi

mkdir -p "$SCRATCH"

run_boot() {  # $1=hook(0|1) -> boots GUI, echoes wrap pid
  export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
  local HOOK_EXTRA=""
  [ "${1:-0}" = "1" ] && HOOK_EXTRA="export YGGTERM_TEST_SILENT_LOOP_DEATHS_FILE=$SCRATCH/silent-deaths"
  dbus-run-session -- bash -c "
    export DISPLAY=:79 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
    export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
    $HOOK_EXTRA
    exec $BIN > /tmp/exitarm-gui.log 2>&1
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

# ---- display ----
for p in $(pgrep -x Xvfb); do
  c=$(tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null)
  case "$c" in *":79"*) kill "$p";; esac
done 2>/dev/null; sleep 1
Xvfb :79 -screen 0 1600x1000x24 -ac > /tmp/exitarm-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5

# ---- boot (aimed mode arms the deaths file; count high enough that every
# task start — boot dummies, rows, remounts — holds an aimed token; a loop
# whose session never runs the aimed arm simply never dies from it) ----
if [ "$MODE" = "aimed" ]; then
  printf '\n8:250@%s\n' "$AIM" > "$SCRATCH/silent-deaths"
  echo "deaths file: $(tr '\n' ' ' < "$SCRATCH/silent-deaths") (aim=$AIM)"
  run_boot 1
else
  run_boot 0
fi
GUI_WRAP=$!
READY=$(boot_wait_ready); echo "gui ready=$READY"
if [ "$READY" != "1" ]; then
  echo "BOOT FAIL: daemon never answered clients"; kill "$GUI_WRAP" 2>/dev/null
  for p in $(scratch_pids_by_name yggterm); do kill "$p" 2>/dev/null; done
  kill "$XVFB_PID" 2>/dev/null; exit 2
fi

SCRATCH=$SCRATCH BIN=$BIN AIM=$AIM MODE=$MODE python3 - <<'PYEOF' > /tmp/exitarm-run.log 2>&1
import json, subprocess, time, os

BIN = os.environ["BIN"]; AIM = os.environ["AIM"]; MODE = os.environ["MODE"]
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

a = create_row_titled("exitarm1")
b = create_row_titled("exitarm2")
print("rows: a=%s b=%s" % (a, b))
if not (a and b):
    print("FAIL: could not create rows"); raise SystemExit(2)
time.sleep(12)

def screen_count(marker, session):
    body = (verb("server", "screen", session, timeout=30).stdout or "")
    return body.count("echo " + marker)

def drops_now():
    return [(ts, p) for ts, nm, p in read_events() if nm == "terminal_mount_task_dropped"]

if MODE == "aimed":
    # ---- the DEGENERATION BAR: the deaths file's own write-back ----------
    # The (i-r1) parser preserves the "@<branch>" suffix on write-back; a
    # pre-fix parser ate the suffix and defaulted the delay.
    deaths_now = open(os.environ["SCRATCH"] + "/silent-deaths").read().strip() if os.path.exists(os.environ["SCRATCH"] + "/silent-deaths") else "(gone)"
    print("deaths file now: %r" % deaths_now)
    if "@" not in deaths_now:
        hints = sorted({str(p.get("exit_hint")) for _, p in drops_now()})
        print("VERDICT BAR: RED — the aimed token DEGENERATED (%r): the build's death hook cannot aim at a branch, so no exit_hint can ever name an uninstrumented arm (drops so far: %d, hints: %s)" % (
            deaths_now, len(drops_now()), hints))
        raise SystemExit(5)
    # an aimed arm that fires early (read_poll_start: the first read
    # deadline of a live mount) kills the loop before any typed input could
    # flow — so the DROP is the engagement proof, corroborated by a
    # prompt-bearing screen (the mount lived long enough to read its prompt)
    deadline = time.time() + 60
    drops = []
    while time.time() < deadline:
        drops = drops_now()
        if any(p.get("exit_hint") == AIM for _, p in drops):
            break
        time.sleep(1)
    screen_nonempty = bool((verb("server", "screen", a, timeout=30).stdout or "").strip())
    print("drops=%d aim=%s screen_nonempty=%s" % (len(drops), AIM, screen_nonempty))
    for ts, p in drops[:8]:
        print("  drop ts=%d session=%s remount_armed=%s exit_hint=%s" % (
            ts, str(p.get("session_path"))[-40:], p.get("remount_armed"), p.get("exit_hint")))
    if not drops:
        print("VERDICT HOOK: NOT ENGAGED — no drops traced; rig invalid"); raise SystemExit(4)
    named = [(ts, p) for ts, p in drops if p.get("exit_hint") == AIM]
    if named and screen_nonempty:
        print("VERDICT BAR: GREEN — the armed arm is named exactly (exit_hint=%s on %d drop(s)) and the mount demonstrably read its prompt (screen non-empty)" % (
            AIM, len(named)))
        raise SystemExit(0)
    hints = sorted({str(p.get("exit_hint")) for _, p in drops})
    print("VERDICT BAR: RED — %d drop(s) traced but exit_hint never == %s (hints seen: %s) — the arm did not stamp its name" % (
        len(drops), AIM, hints))
    raise SystemExit(5)

# ---- MODE=close: the measurement arm -------------------------------------
# healthy-mount proof on the ACTIVE row (b, created last — input focus
# latches to the active row, the remount-rig law; a is the remove target)
verb("server", "app", "terminal", "probe-type", b, "--data", "echo EXITARM", "--enter", "--mode", "xterm", timeout=30)
painted = 0
deadline = time.time() + 60
while time.time() < deadline:
    painted = screen_count("EXITARM", b)
    if painted >= 1:
        break
    time.sleep(1)
print("marker painted on active row: %d" % painted)
if painted < 1:
    print("PREFLIGHT FAIL: marker never painted — the write drive is broken"); raise SystemExit(3)
# Switch ACTIVE to a first (the REAL switch verb — the focus verb never
# switches, gotcha 19; a demoted row's loop may churn-die before the close,
# and a dead loop cannot run the exit arm), then remove a: its mount eval
# bridge RESOLVES, the eval_result arm runs (lease release + remount arm +
# break), the drop witness fires. Only POST-REMOVE drops count — a's
# creation-churn death (measured: pre_select, 13s before the remove in the
# first RED run) is evidence of the same defect but not this bar.
verb("server", "app", "open", a, "--view", "terminal", timeout=30)
time.sleep(3)
t_remove_ms = int(time.time() * 1000)
out = verb("server", "app", "session", "remove", a, timeout=30)
print("session remove rc=%s out=%s%s" % (out.returncode, (out.stdout or "")[:120], (out.stderr or "")[:120]))
deadline = time.time() + 30
a_drops = []
while time.time() < deadline:
    a_drops = [(ts, p) for ts, p in drops_now()
               if p.get("session_path") == a and ts >= t_remove_ms]
    if a_drops:
        break
    time.sleep(0.5)
print("a-drops(post-remove)=%d" % len(a_drops))
for ts, p in a_drops[:5]:
    print("  drop ts=%d remount_armed=%s exit_hint=%s" % (
        ts, p.get("remount_armed"), p.get("exit_hint")))
if not a_drops:
    print("VERDICT HOOK: NOT ENGAGED — the removed row never dropped; rig invalid"); raise SystemExit(4)
hints = sorted({str(p.get("exit_hint")) for _, p in a_drops})
if hints == ["pre_select"]:
    print("VERDICT MEASUREMENT: the remove-death reads pre_select — session-remove CANCELS the mount task at the select await (does NOT flow through the eval arm); on an all-arms-stamped build this is CONCLUSIVE (drop remount_armed=%s, healthy mount marker=%d)" % (
        a_drops[0][1].get("remount_armed"), painted))
    raise SystemExit(0)
print("VERDICT MEASUREMENT: the post-remove drop names a NON-pre_select arm (%s) — the exit arm RAN; inspect the trace" % (hints,))
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
if [ "$EXE" = "$BIN" ]; then echo "EXE OK: requested binary served this run"; else echo "EXE MISMATCH: verdicts above are VOID"; RC=8; fi
echo "=== DRIVER OUTPUT ==="
tail -30 /tmp/exitarm-run.log
echo "scratch=$SCRATCH rc=$RC"
exit $RC
