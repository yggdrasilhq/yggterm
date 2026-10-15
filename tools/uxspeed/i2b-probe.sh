#!/bin/bash
# i2b-probe.sh — THE ALIVE-LOOP EVIDENCE-LESS PARK measurement (sitting 45 unit 2)
#
# Constructs sol s45 Q3's uncovered case: a content claim parked on a
# LIVING loop with NO evidence source (the s35 PARK_CLAIM hook parks +
# drops the Rust receipt; the rig then clears the PAGE entry the re-attest
# probe reads, so the probe abstains — receipt:null — and the claim waits).
# Measures the SURFACE STORY while it waits (pixels, input, notifications,
# attempt state), then the natural resolution (a real write mints a fresh
# receipt and the promotion completes).
#
# rc 0 = shape measured; the verdict lines record the surface story.
# rc 2 = vacuous (the park never fired / the row never painted).
# rc 3 = infra.
set -u
export TERM=xterm-256color
BIN=${1:?usage: i2b-probe.sh <binary>}
SCRATCH=/tmp/i2b-home-$(date +%s)-$$
NONCE="i2b$(date +%s)"
PARKFILE=$SCRATCH/park-claim
export BIN SCRATCH NONCE PARKFILE
if ! [ -x "$BIN" ]; then echo "NO BINARY at $BIN"; exit 2; fi
mkdir -p "$SCRATCH"

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

for p in $(pgrep -x Xvfb); do
  c=$(tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null)
  case "$c" in *":81 "*) kill "$p";; esac
done 2>/dev/null; sleep 1
Xvfb :81 -screen 0 1600x1000x24 -ac > /tmp/i2b-xvfb.log 2>&1 &
XVFB_PID=$!; sleep 1.5

export PATH="$(dirname "$BIN"):$PATH" YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
dbus-run-session -- bash -c "
  export DISPLAY=:81 GDK_BACKEND=x11 YGGTERM_FORCE_X11_BACKEND=1 RUST_BACKTRACE=1
  export YGGTERM_HOME=$SCRATCH XDG_DATA_HOME=$SCRATCH/xdg-share
  export YGGTERM_TEST_PARK_CLAIM_FILE=$PARKFILE
  exec $BIN > /tmp/i2b-gui.log 2>&1
" >/dev/null 2>&1 </dev/null &

READY=0
for i in $(seq 1 150); do sleep 1; "$BIN" server app clients >/dev/null 2>&1 && READY=1 && break; done
echo "boot: ready=$READY exe=$(md5sum "$BIN" | cut -c1-8)"
[ "$READY" = "1" ] || { echo "BOOT FAIL"; tail -5 /tmp/i2b-gui.log; exit 3; }

"$BIN" server app terminal new --kind shell --cwd /tmp --title "$NONCE" > "$SCRATCH/new.out" 2>&1
ROW=""
for i in $(seq 1 45); do
  sleep 2
  ROW=$("$BIN" server snapshot 2>/dev/null | python3 -c "
import json,sys
try: d=json.load(sys.stdin)
except Exception: sys.exit(0)
for r in d.get('live_sessions') or []:
    if '$NONCE' in str(r.get('title') or ''):
        print(r.get('session_path') or ''); break
")
  [ -n "$ROW" ] && break
done
echo "row: $ROW"
[ -n "$ROW" ] || { echo "ROW FAIL"; exit 3; }

SCRATCH=$SCRATCH BIN=$BIN ROW=$ROW NONCE=$NONCE PARKFILE=$PARKFILE python3 - <<'PYEOF'
import json, subprocess, time, sys, os

BIN = os.environ["BIN"]; SCRATCH = os.environ["SCRATCH"]; ROW = os.environ["ROW"]
PARKFILE = os.environ["PARKFILE"]; NONCE = os.environ["NONCE"]
TRACE = SCRATCH + "/ytrace.jsonl"

def verb(*args, timeout=60):
    try:
        return subprocess.run([BIN] + list(args), capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        class R: stdout = ""; returncode = -1
        return R()

def screen_text():
    out = verb("server", "terminal", "screen", ROW, timeout=30)
    b = (out.stdout or "").strip()
    if b.startswith("{"):
        try:
            env = json.loads(b)
            if env.get("accepted") is False: return ""
            return str(env.get("text") or "")
        except Exception:
            return ""
    return b

def page_eval(script):
    out = verb("server", "app", "web", "eval", script, "--session", ROW, timeout=30)
    return (out.stdout or "").strip()

def finish(rc):
    print("RC%d" % rc)
    sys.exit(rc)

# --- baseline: painted ---
BASE = ""
for i in range(90):
    BASE = screen_text()
    b = BASE.strip()
    if len(b) > 40 and not b.startswith("{"):
        break
    time.sleep(2)
print("baseline: bytes=%d" % len(BASE))
if len(BASE.strip()) <= 40:
    print("VACUOUS: row never painted")
    finish(2)

# --- the mount's host_id from the trace ---
host_id = ""
try:
    for line in open(TRACE):
        if "mount_open" in line and ROW in line:
            payload = json.loads(line).get("payload") or {}
            host_id = str(payload.get("host_id") or "")
            break
except FileNotFoundError:
    pass
print("host_id: %s" % (host_id or "MISSING"))
if not host_id:
    print("VACUOUS: no mount_open host_id")
    finish(2)

# --- clear the PAGE evidence FIRST (the probe reads the page; a park
# armed before the clear races the probe's 2s cadence and reattests from
# the pre-clear evidence) ---
cleared = page_eval(
    'const __h = window.__yggtermXtermHosts || {};'
    'const __e = __h["%s"];'
    'if (__e) { delete __e.appliedContent; }'
    'JSON.stringify({cleared: Boolean(__e)});' % host_id
)
print("page evidence cleared (pre-park): %s" % cleared[:200])

# --- arm the park hook (marker matches the row) ---
with open(PARKFILE, "w") as f:
    # ⛔ THIS hook treats an EMPTY marker as INERT (unlike READ_ERROR_FILE):
    # the marker must be a NON-EMPTY substring of the SESSION PATH.
    f.write(ROW.split("//")[-1] + "\n")
print("park hook armed")

# --- wait for the park trace ---
parked = None
t0 = time.time()
while time.time() - t0 < 30:
    try:
        for line in open(TRACE):
            if "applied_content_test_park" in line and ROW in line:
                payload = json.loads(line).get("payload") or {}
                if payload.get("parked"):
                    parked = payload
                    break
    except FileNotFoundError:
        pass
    if parked:
        break
    time.sleep(1)
print("park: %s" % json.dumps(parked or {}))
if not parked:
    print("VACUOUS: the park hook never fired")
    finish(2)

# --- observe the WAIT: 45s of surface story ---
time.sleep(2)
proof = page_eval(
    'const __h = window.__yggtermXtermHosts || {};'
    'const __e = __h["%s"] || null;'
    'const __r = __e && __e.appliedContent ? __e.appliedContent : null;'
    'JSON.stringify({present: Boolean(__r), entry_live: Boolean(__e)});' % host_id
)
print("probe view (must be present:false): %s" % proof[:140])

# reattest probe must ABSTAIN: no applied_content_reattested in the trace
reatt = 0
try:
    for line in open(TRACE):
        if "applied_content_reattested" in line and ROW in line:
            reatt += 1
except FileNotFoundError:
    pass
print("reattest events while evidence-less: %d (0 = the probe abstains)" % reatt)

# the surface while waiting: screen still shows the painted prompt, input works
wait_screen = screen_text()
surf = {"screen_bytes": len(wait_screen.strip()), "screen_head": wait_screen.strip()[:60].replace(chr(27), "^[")}
print("surface while waiting: %s" % json.dumps(surf))

# --- the natural resolution: a real write mints a fresh receipt ---
t1 = time.time()
resolved = False
last = wait_screen
while time.time() - t1 < 45:
    subprocess.run([BIN, "server", "terminal", "write", ROW, "--stdin"],
                   input="x", capture_output=True, text=True, timeout=30)
    time.sleep(4)
    now = screen_text()
    if now != last:
        resolved = True
        print("write resolved the wait: screen mutated at +%.0fs (%d -> %d bytes)"
              % (time.time() - t1, len(last), len(now)))
        break
    last = now

try:
    for line in open(TRACE):
        if "applied_content_reattested" in line and ROW in line:
            payload = json.loads(line).get("payload") or {}
            print("post-write reattest: %s" % json.dumps(payload)[:160])
            break
except FileNotFoundError:
    pass

print("VERDICT: alive-loop evidence-less park — waited with the surface honest (%s), resolved by a real write: %s"
      % ("screen painted, probe abstained" if reatt == 0 else "UNEXPECTED reattest", resolved))
finish(0)
PYEOF
exit $?
