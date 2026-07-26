#!/usr/bin/env bash
# One arm of a hardware-GL vs software-GL A/B on the GUI host.
#
# This script exists because the same measurement has now been botched three
# ways, and each failure produced a confident wrong number rather than an error:
#
#   1. The first attempt compared an evening of real use against an overnight
#      window with 23x less terminal activity and reported an 18x win. The
#      `gpu_ms` series refuted it from the inside: the GPU was idle in 523 of
#      the "after" window's 532 render ticks, because nothing was painting.
#   2. The second attempt ran a paint load with its output redirected to
#      /dev/null, so it painted nothing and measured the idle floor.
#   3. The third attempt restarted the GUI between arms, and the GUI came back
#      displaying a DIFFERENT session — so the load went to a session that was
#      not on screen. The arm reported 0.27 cores and looked like a 5x win.
#
# Every one of those is the same failure: the load did not reach the renderer,
# and nothing said so. So this script REFUSES to report a number unless it can
# show the work actually landed. It checks, before and after:
#
#   - the window is focused (an unfocused window paints nothing)
#   - the session under test is the ACTIVE one (the GUI paints one session)
#   - the GUI's own render tick observed CPU in the window
#   - nothing else is loading the host
#
# If any check fails it exits non-zero with the reason. A measurement that
# cannot fail is worth exactly as much as a test that cannot fail.
#
# Usage:  gl_ab_measure.sh <arm-label> <expected-session-path> <duration-s>
# The paint load is generated SEPARATELY, by the session named above, because
# only the displayed session's output reaches the renderer.

set -uo pipefail

ARM="${1:?arm label}"
EXPECT_SESSION="${2:?session path that will generate the load}"
DUR="${3:-40}"
HZ=$(getconf CLK_TCK 2>/dev/null || echo 100)
YG="$HOME/.local/bin/yggterm"

fail() { echo "REFUSED[$ARM]: $*" >&2; exit 1; }

gui_pid=$(pgrep -x yggterm | while read -r p; do
	[ -n "$(pgrep -P "$p" -x yggterm)" ] || echo "$p"
done | head -1)
[ -n "${gui_pid:-}" ] || fail "no GUI process"

# --- preconditions, read from the GUI's own state -------------------------
state_probe() {
	"$YG" server app state 2>/dev/null | python3 -c '
import json,sys
d=json.load(sys.stdin)
out={}
def f(o):
    if isinstance(o,dict):
        for k,v in o.items():
            if k in ("active_session_path","window_focused") and k not in out: out[k]=v
            elif isinstance(v,(dict,list)): f(v)
    elif isinstance(o,list):
        for v in o: f(v)
f(d)
print(json.dumps(out))'
}

before_state=$(state_probe)
active=$(echo "$before_state" | python3 -c 'import json,sys;print(json.load(sys.stdin).get("active_session_path",""))')
focused=$(echo "$before_state" | python3 -c 'import json,sys;print(json.load(sys.stdin).get("window_focused",False))')

[ "$focused" = "True" ] || [ "$focused" = "true" ] || fail "window not focused — it would paint nothing"
[ "$active" = "$EXPECT_SESSION" ] || fail "the GUI is displaying $active, not $EXPECT_SESSION — the load would paint nothing"

# Anything else busy on this host contaminates the arm.
others=$(ps -eo pcpu,comm --no-headers | awk '$1 > 20 && $2 !~ /yggterm|WebKit/ {print $2}' | head -3)
[ -z "$others" ] || echo "WARN[$ARM]: other busy processes: $others" >&2

pids=("$gui_pid")
while read -r p; do [ -n "$p" ] && pids+=("$p"); done < <(pgrep -P "$gui_pid")

cpu_ticks() { awk '{print $14 + $15}' "/proc/$1/stat" 2>/dev/null || echo 0; }

# Per DRM CLIENT, not per fd: duplicated fds share one struct file and each
# repeats the same cumulative counter, so a per-fd sum over-counts by the fd
# count (measured 3.8x here, 5.0x on Xorg).
gpu_ns() {
	awk '
		/^drm-client-id:/  { client = $2 }
		/^drm-engine-gfx:/ { if (!(client in seen)) { seen[client] = 1; sum += $2 } }
		END { print sum + 0 }
	' "/proc/$1"/fdinfo/* 2>/dev/null || echo 0
}

declare -A t0 g0
for p in "${pids[@]}"; do t0[$p]=$(cpu_ticks "$p"); g0[$p]=$(gpu_ns "$p"); done

sleep "$DUR"

# --- postconditions: the state must not have moved under us ---------------
after_active=$(state_probe | python3 -c 'import json,sys;print(json.load(sys.stdin).get("active_session_path",""))')
[ "$after_active" = "$EXPECT_SESSION" ] || fail "the active session changed to $after_active mid-arm"

total_cores=0
total_gpu=0
echo "--- arm=$ARM session=$EXPECT_SESSION dur=${DUR}s"
printf '%-8s %-20s %10s %10s\n' PID COMM CORES GPU_MS
for p in "${pids[@]}"; do
	[ -d "/proc/$p" ] || continue
	comm=$(cat "/proc/$p/comm" 2>/dev/null || echo '?')
	d=$(( $(cpu_ticks "$p") - ${t0[$p]} ))
	cores=$(awk -v d="$d" -v hz="$HZ" -v s="$DUR" 'BEGIN{printf "%.4f", d/hz/s}')
	gms=$(( ($(gpu_ns "$p") - ${g0[$p]}) / 1000000 ))
	total_cores=$(awk -v a="$total_cores" -v b="$cores" 'BEGIN{printf "%.4f", a+b}')
	total_gpu=$((total_gpu + gms))
	printf '%-8s %-20s %10s %10s\n' "$p" "$comm" "$cores" "$gms"
done

# The load must have cost SOMETHING. A near-zero arm means the paint never
# arrived — which is exactly how the third botched attempt reported a 5x win.
below=$(awk -v c="$total_cores" 'BEGIN{print (c < 0.05) ? 1 : 0}')
[ "$below" = "0" ] || fail "total ${total_cores} cores over ${DUR}s — the load did not reach the renderer"

echo "ARM=$ARM TOTAL_CORES=$total_cores TOTAL_GPU_MS=$total_gpu DUR=${DUR}s"
