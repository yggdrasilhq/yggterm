#!/bin/bash
# i2-rig.sh — THE (i-r2) EXHAUSTION-PARK / LOOP-RESTART MEASUREMENT RIG (sitting 45)
#
# Constructs the REMOTE established-frame read-error exhaustion on a REAL
# remote-agy row (PTY owned by oc) and measures the whole keep-alive chain:
#   inject read errors -> post-attach recovery retries -> the remote
#   exhaustion arm (park_terminal_content_claim + break) -> the drop guard
#   punch -> the witnessed-death watchdog remount -> successor loop ->
#   recovery after fault removal, at pixels, WITHOUT user reselection.
#
# This is the s27-owed remote construction bar (the readerr rig's remote
# branch) and the sol s45 measurement: whether an attach_complete_latch
# claim survives / is replaced across the remount (begin retires claims by
# design — the trace must SHOW it), and what the surface does at budget
# exhaustion.
#
# Usage: i2-rig.sh <binary> [mode] [expect-fast-arm]
#   mode: early-recover (default) | exhaust
#   early-recover: remove the fault after the FIRST successor mount; assert
#     the row recovers at pixels (write->screen mutation round trip) within 120s.
#   exhaust: let the remount budget exhaust (3 cycles), observe the degraded
#     end state (screenshot captured), remove the fault, measure the blind
#     window to recovery.
#   expect-fast-arm: fail when a drop->remount gap exceeds 1500ms (the (i-r2)
#     by-design arm; main measures the witnessed-tick baseline ~3.5-4s).
# rc 0 = chain complete + recovery at pixels
# rc 1 = chain broken (named missing event or no recovery)
# rc 2 = vacuous construction (no remote row / no injection effect / no drop)
# rc 3 = infra (boot fail, mint fail)
set -u
export TERM=xterm-256color
BIN=${1:?usage: i2-rig.sh <binary> [mode] [expect-fast-arm]}
MODE=${2:-early-recover}
FASTARM=${3:-}
SCRATCH=/tmp/i2-rig-home-$(date +%s)-$$
NONCE="s45i2$(date +%s)"
FLAG=$SCRATCH/read-errors
export BIN SCRATCH NONCE FLAG
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN"; exit 2; fi
mkdir -p "$SCRATCH/config"

scratch_pids() {
  for p in $(pgrep -u "$USER" yggterm 2>/dev/null); do
    h=$(tr '\0' '\n' < /proc/$p/environ 2>/dev/null | sed -n 's/^YGGTERM_HOME=//p')
    [ "$h" = "$SCRATCH" ] && echo "$p"
  done
}

cleanup() {
  for p in $(scratch_pids); do kill "$p" 2>/dev/null; done
  sleep 1
  for p in $(scratch_pids); do kill -9 "$p" 2>/dev/null; done
  [ -n "${XVFB_PID:-}" ] && kill "$XVFB_PID" 2>/dev/null
}
trap cleanup EXIT

# ---- machine seeding (the s38 law: a fresh home knows NO machines) ----
cp -f "$HOME/.yggterm/config/machines.json" "$SCRATCH/config/machines.json" 2>/dev/null
python3 - "$HOME/.yggterm/server-state.json" "$SCRATCH/server-state.json" <<'PYEOF'
import json, sys
prod_path, scratch_path = sys.argv[1], sys.argv[2]
try:
    p = json.load(open(prod_path))
except Exception:
    sys.exit(0)
try:
    s = json.load(open(scratch_path))
except Exception:
    s = {"stored_sessions": [], "live_sessions": []}
for k in ("ssh_targets", "remote_machines"):
    if p.get(k) and not s.get(k):
        s[k] = p[k]
json.dump(s, open(scratch_path, "w"))
PYEOF

# ---- Xvfb :80 ----
for p in $(pgrep -x Xvfb); do
  c=$(tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null)
  case "$c" in *":80 "*) kill "$p";; esac
done 2>/dev/null; sleep 1
Xvfb :80 -screen 0 1600x1000x24 -ac > /tmp/i2-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5

# ---- boot with the hook ARMED-ABSENT (the file does not exist yet) ----
export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
dbus-run-session -- bash -c "
  export DISPLAY=:80 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
  export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
  export YGGTERM_TEST_READ_ERROR_FILE=$FLAG
  exec $BIN > /tmp/i2-gui.log 2>&1
" >/dev/null 2>&1 </dev/null &
GUI_WRAP=$!

READY=0
for i in $(seq 1 150); do sleep 1; "$BIN" server app clients >/dev/null 2>&1 && READY=1 && break; done
echo "boot: ready=$READY exe=$(md5sum "$BIN" | cut -c1-8)"
if [ "$READY" != "1" ]; then
  echo "BOOT FAIL"; tail -5 /tmp/i2-gui.log 2>/dev/null
  exit 3
fi

# ---- create the remote-agy row (the ensure mints agy on oc over the hop) ----
"$BIN" server app terminal new --machine-key oc --kind antigravity --cwd /home/pi --title "$NONCE" > "$SCRATCH/new.out" 2>&1
echo "terminal new: rc=$? out=$(head -c 200 "$SCRATCH/new.out" | tr '\n' ' ')"
ROW=""
for i in $(seq 1 45); do
  sleep 2
  ROW=$("$BIN" server snapshot 2>/dev/null | python3 -c "
import json,sys
try:
    d=json.load(sys.stdin)
except Exception:
    sys.exit(0)
for r in d.get('live_sessions') or []:
    if '$NONCE' in str(r.get('title') or ''):
        print(r.get('session_path') or '')
        break
")
  [ -n "$ROW" ] && break
done
echo "row: $ROW"
if [ -z "$ROW" ]; then
  echo "ROW FAIL"; tail -3 /tmp/i2-gui.log 2>/dev/null
  exit 3
fi

# ---- python driver ----
SCRATCH=$SCRATCH BIN=$BIN ROW=$ROW MODE=$MODE NONCE=$NONCE FLAG=$FLAG FASTARM=$FASTARM python3 - <<'PYEOF'
import json, subprocess, time, os, sys

BIN = os.environ["BIN"]
SCRATCH = os.environ["SCRATCH"]
ROW = os.environ["ROW"]
MODE = os.environ["MODE"]
FLAG = os.environ["FLAG"]
FASTARM = os.environ.get("FASTARM", "")
TRACE = SCRATCH + "/event-trace.jsonl"

def verb(*args, timeout=60):
    try:
        return subprocess.run([BIN] + list(args), capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        class R: stdout = ""; returncode = -1
        return R()

def screen_bytes():
    # The screen verbs answer a JSON envelope (source: daemon_screen carries
    # the frame in "text"); accepted:false envelopes are unpainted rows.
    out = verb("server", "terminal", "screen", ROW, timeout=30)
    body = (out.stdout or "").strip()
    if body.startswith("{"):
        try:
            env = json.loads(body)
        except Exception:
            return ""
        if env.get("accepted") is False:
            return ""
        return str(env.get("text") or "")
    return body

def finish(rc):
    print("RC%d" % rc)
    sys.exit(rc)

WATCH = [
    "transport_error_visible_in_host_retry",
    "transport_error_recovery_exhausted",
    "read_error_after_attach",
    "terminal_mount_task_dropped",
    "terminal_mount_loop_stale_detected",
    "remount_armed",
    "terminal_mount_loop_stale_degraded",
    "attach_ready",
    "attach_complete",
    "stale_claim",
    "voided",
    "park",
    "reattest",
]

def events_since(offset):
    out = []
    try:
        with open(TRACE) as f:
            f.seek(offset)
            text = f.read()
    except FileNotFoundError:
        return out, offset
    dec = json.JSONDecoder()
    idx, n = 0, len(text)
    consumed = 0
    while idx < n:
        while idx < n and text[idx] in " \t\r\n":
            idx += 1
        if idx >= n:
            break
        try:
            event, idx2 = dec.raw_decode(text, idx)
        except Exception:
            idx += 1
            continue
        idx = idx2
        consumed = idx2
        if not isinstance(event, dict):
            continue
        payload = event.get("payload")
        if not isinstance(payload, dict):
            payload = {}
        name = (event.get("name") or event.get("event") or payload.get("name") or payload.get("event"))
        if name and any(want in name for want in WATCH):
            out.append((name, payload))
    return out, offset + consumed

def row_of(payload):
    return ROW in str(payload.get("session_path") or "")

def brief(payload):
    keys = ("session_path", "attempt", "exit_hint", "remount_armed", "reason",
            "stale_threshold_ms", "remount_epoch", "source")
    return json.dumps({k: payload.get(k) for k in keys if k in payload})[:200]

# --- baseline: painted before injection (verb JSON errors start with '{' —
# an unmounted row answers a JSON envelope, NEVER count that as paint) ---
BASE = ""
for i in range(150):
    BASE = screen_bytes()
    body = BASE.strip()
    if len(body) > 40 and not body.startswith("{"):
        break
    time.sleep(2)
print("baseline: bytes=%d head=%r" % (len(BASE), BASE[:70].replace(chr(27), "^[")))
if len(BASE.strip()) <= 40 or BASE.strip().startswith("{"):
    print("VACUOUS: row never painted (trust gate stuck or ensure dead)")
    finish(2)

# --- INJECT ---
with open(FLAG, "w") as f:
    f.write("\n")
print("inject: flag created")
t0 = time.time()
seen = {}
off = 0
drops = []
remounts = []
post_drop_ready = 0
drop_at = None
exhaust_at = None
arm_gaps = []
INJECT_WINDOW = 420 if MODE == "exhaust" else 240
while time.time() - t0 < INJECT_WINDOW:
    evs, off = events_since(off)
    for name, payload in evs:
        if payload and not row_of(payload):
            continue
        seen[name] = seen.get(name, 0) + 1
        print("  ev %s %s" % (name, brief(payload)))
        if name == "terminal_mount_task_dropped":
            drops.append(time.time())
            if drop_at is None:
                drop_at = time.time()
        if "remount_armed" in name:
            remounts.append(time.time())
            if drops:
                arm_gaps.append(time.time() - drops[-1])
        if name == "terminal_mount_loop_stale_degraded" and exhaust_at is None:
            exhaust_at = time.time()
            try:
                cap = verb("server", "app", "screenshot", timeout=60)
                print("exhaust screenshot: %s" % (cap.stdout or "").strip().split("output_path")[-1][:160])
            except Exception as e:
                print("exhaust screenshot failed: %s" % e)
        if drop_at is not None and "attach_ready" in name:
            post_drop_ready += 1
    if MODE == "early-recover" and drops and remounts:
        # remove the fault at the FIRST witnessed arm — the instant remount
        # then attaches with CLEAN reads and the recovery bar is honest
        # (waiting for a successor attach trace can overrun the budget
        # window and turn the scenario into an exhaust run)
        print("first drop armed — removing fault now")
        break
    time.sleep(2)

print("phase-inject: %.0fs seen=%s" % (time.time() - t0, json.dumps(seen)))
print("arm-gaps(s): %s" % ["%.2f" % g for g in arm_gaps])
if FASTARM == "expect-fast-arm" and any(g > 1.5 for g in arm_gaps):
    print("FAST-ARM RED: a drop waited >1.5s for its remount (witnessed-tick baseline)")
    finish(1)
if not drops:
    print("VACUOUS: no task drop observed (injection never reached the exhaustion arm)")
    finish(2)
if not remounts:
    print("CHAIN BROKEN: drop observed but NO remount armed (watchdog silent)")
    finish(1)

# --- fault removal + recovery ---
if os.path.exists(FLAG):
    os.remove(FLAG)
print("fault removed at t=+%0.f (mode=%s)" % (time.time() - t0, MODE))
rec_t0 = time.time()
RECOVERY_WAIT = 120 if MODE == "early-recover" else 720
recovered = False
last_write = 0.0
while time.time() - rec_t0 < RECOVERY_WAIT:
    evs, off = events_since(off)
    for name, payload in evs:
        if payload and not row_of(payload):
            continue
        seen[name] = seen.get(name, 0) + 1
        if "remount_armed" in name:
            remounts.append(time.time())
            print("  ev %s (post-fault) %s" % (name, brief(payload)))
    body = screen_bytes()
    if body.strip() and time.time() - last_write > 12:
        # LIVING-ROW BAR: write-drive unconditionally (resumed mounts never
        # re-trace attach_ready, so a trace-gated bar never fires). A typed
        # char rendered into the composer = input drained through a LIVING
        # loop and echoed back — the round trip sol Q5 demanded. A dead loop
        # leaves the keystroke in the ring and the screen unchanged.
        last_write = time.time()
        subprocess.run([BIN, "server", "terminal", "write", ROW, "--stdin"],
                       input="x", capture_output=True, text=True, timeout=30)
        time.sleep(4)
        after = screen_bytes()
        if after != body:
            recovered = True
            print("living-row proof: write mutated the screen (%d -> %d bytes) at +%.0fs"
                  % (len(body), len(after), time.time() - rec_t0))
            break
    time.sleep(3)

print("phase-recover: recovered=%s wait=%.0fs" % (recovered, time.time() - rec_t0))
print("final-seen: %s" % json.dumps(seen))
if MODE == "exhaust":
    if exhaust_at:
        print("exhaust: degraded observed at +%.0fs; post-window recovery: %s"
              % (exhaust_at - t0, recovered))
        finish(0 if recovered else 1)
    print("exhaust: budget NEVER exhausted inside the window (cycles slower than modeled)")
    finish(1 if recovered else 2)

if recovered and drops and remounts:
    print("CHAIN COMPLETE: inject -> retries -> drop -> witnessed remount -> successor -> pixels (no user reselect)")
    finish(0)
if recovered:
    print("PARTIAL: recovery happened but chain events incomplete")
    finish(1)
print("CHAIN BROKEN: no recovery at pixels within %ds after fault removal" % RECOVERY_WAIT)
finish(1)
PYEOF
exit $?
