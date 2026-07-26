#!/usr/bin/env bash
# Prove scripts/gl_ab_experiment.sh can actually COMPLETE AN ARM — on any
# machine, with no GUI, no user desktop, and nothing of the live host touched.
#
# WHY THIS EXISTS. The shipped harness could not finish a single arm: verify_arm
# fed python a heredoc program and a herestring payload, both on fd 0, so python
# read the desktop-identity JSON as its program and the function died ~15 s into
# arm S of every run. It was reported as "runnable" and `docs/optimization-pass.md`
# told the next reader not to hand-roll it. The only test was `bash -n`, which
# cannot see a redirection-precedence bug. So: an end-to-end run, against
# throwaway stand-ins, that exercises the real script.
#
# WHAT IS REAL AND WHAT IS STUBBED. The script under test is the SHIPPED
# scripts/gl_ab_experiment.sh, unmodified, with its real arm table, its real
# scrub, its real verify_arm, its real generational copy and its real analyzer.
# Stubbed: the yggterm binary (a shell script that publishes a GL environment
# and answers the four verbs the harness uses), the compositor (a process that
# really does own a wayland socket, so the "not on the user's desktop" guard is
# SATISFIED rather than bypassed), and the per-sample measurement (whose own
# refusals have their own tests).
#
# The stub GUI opens /dev/dri/renderD128 exactly when the arm does not force
# software, so the DRM-fd assertions in verify_arm are exercised for real in
# both directions.
#
# Usage: scripts/gl_ab_selftest.sh [workdir]

set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORK="${1:-$(mktemp -d /tmp/gl-ab-selftest-XXXXXX)}"
mkdir -p "$WORK/bin" "$WORK/run"
FAILURES=0

fail() {
	echo "SELF-TEST FAILED: $*" >&2
	FAILURES=$((FAILURES + 1))
}
ok() { echo "  ok: $*"; }

# ---------------------------------------------------------------------------
# stand-ins
# ---------------------------------------------------------------------------
cat >"$WORK/bin/sway" <<'SWAY'
#!/usr/bin/env bash
# A compositor stand-in that really owns a wayland socket, so
# start_compositor's ownership check passes for the true reason.
exec python3 -c '
import os, socket, sys, time
path = os.path.join(os.environ["XDG_RUNTIME_DIR"], "wayland-lab")
# yggterm-gl-ab-standin
try:
    os.unlink(path)
except FileNotFoundError:
    pass
s = socket.socket(socket.AF_UNIX)
s.bind(path)
s.listen(1)
sys.stderr.write("lab compositor up\n")
sys.stderr.flush()
time.sleep(86400)
'
SWAY

cat >"$WORK/bin/yggterm" <<'YG'
#!/usr/bin/env bash
# A yggterm stand-in: publishes a GL environment the way a real client does
# (the CLIENT decides and publishes; the CLI only reads it back), and answers
# the verbs the harness calls.
set -uo pipefail
home="${YGGTERM_HOME:-$HOME/.yggterm}"

publish() {
	mkdir -p "$home"
	local policy="hardware_gl_probed"
	local extra=""
	if [ "${YGGTERM_FORCE_SOFTWARE_GL:-}" = "1" ]; then
		policy="software_gl_forced"
		extra=',"LIBGL_ALWAYS_SOFTWARE":"1","GALLIUM_DRIVER":"llvmpipe"'
	fi
	# An inherited key the scrub was supposed to remove. Present only when the
	# self-test asks for it, so the absence assertion can be shown to FIRE.
	if [ -n "${SELFTEST_LEAK_KEY:-}" ]; then
		extra="$extra,\"${SELFTEST_LEAK_KEY}\":\"1\""
	fi
	printf '{"desktop_file":{"name":"yggterm"},"clients":[{"pid":%s,"webkit_gl_environment":{"YGGTERM_WEBKIT_GL_POLICY":"%s"%s}}]}\n' \
		"$$" "$policy" "$extra" >"$home/gl-identity.json"
}

case "${1:-}" in
"")
	# The GUI itself. Hold a DRM render node open iff this arm is not forcing
	# software — that is exactly what verify_arm's fd count asserts on.
	publish
	# STANDIN_MARK rides in argv so the orphan check below can find these
	# processes. `pgrep -f` on a sleep call is not a check: the marker must be
	# something only this file puts there.
	if [ "${YGGTERM_FORCE_SOFTWARE_GL:-}" = "1" ]; then
		exec python3 -c 'import time; time.sleep(86400)' yggterm-gl-ab-standin
	fi
	exec python3 -c '
import time
fd = open("/dev/dri/renderD128", "rb")
time.sleep(86400)
' yggterm-gl-ab-standin
	;;
server)
	shift
	case "$*" in
	"app desktop-identity")
		[ -f "$home/gl-identity.json" ] || exit 7
		cat "$home/gl-identity.json"
		;;
	"terminal new --json") echo '{"session_path":"local://lab-session"}' ;;
	"shutdown") : ;;
	*) : ;;
	esac
	;;
*) : ;;
esac
YG

cat >"$WORK/bin/measure" <<'MEASURE'
#!/usr/bin/env bash
# A measurement stand-in. The real one's refusals are tested where they live;
# this exists so the harness's arm loop can be driven to completion.
set -uo pipefail
arm="$1"
session="$2"
now=$(date +%s%3N)
cores=0.30
gpu=7
case "$arm" in
S | S2) gpu=0 ;;
esac
# The self-test can force the "hardware arm that was simply idle" signature, so
# the run's ability to say "this settles nothing" is exercised too.
[ "${SELFTEST_IDLE_ARM:-}" = "$arm" ] && gpu=0
printf '{"arm":"%s","tag":"%s","session":"%s","dur_s":1,"cores":%s,"gpu_ms":%s,"t0_ms":%s,"t1_ms":%s}\n' \
	"$arm" "${SAMPLE_TAG:-}" "$session" "$cores" "$gpu" "$((now - 500))" "$now" >>"$SAMPLE_JSON"
MEASURE

chmod +x "$WORK/bin/sway" "$WORK/bin/yggterm" "$WORK/bin/measure"

export XDG_RUNTIME_DIR="$WORK/run"
export PATH="$WORK/bin:$PATH"

# The lab GUI's diagnostic streams, seeded into each arm's home the moment it is
# created. The ONE window_focus transition lives in a ROTATED generation, which
# is the shape that made the analyzer void arms for a non-GL reason.
seed_streams() { # home
	local home="$1" now
	now=$(date +%s%3N)
	mkdir -p "$home"
	printf '{"ts_ms":%s,"category":"window_focus","name":"transition","payload":{"focused":true}}\n' \
		"$((now - 60000))" >"$home/event-trace.g$((now - 60000)).jsonl"
	printf '{"ts_ms":%s,"category":"render","name":"frame"}\n' "$now" >"$home/event-trace.jsonl"
	: >"$home/perf-telemetry.jsonl"
	local i
	for i in $(seq 40); do
		printf '{"ts_ms":%s,"name":"xterm_write_flush"}\n' "$((now - 30000 + i * 10))" \
			>>"$home/perf-telemetry.g$((now - 30000)).jsonl"
	done
	for i in $(seq 40); do
		printf '{"ts_ms":%s,"name":"xterm_write_flush"}\n' "$((now + i * 10))" \
			>>"$home/perf-telemetry.jsonl"
	done
}

run_experiment() { # outdir arms [extra env assignments...]
	local outdir="$1" arms="$2"
	shift 2
	# The harness wipes and recreates each arm's home, so the streams are seeded
	# by a watcher rather than up front.
	(
		local waited=0
		while [ "$waited" -lt 600 ]; do
			for home in "$outdir"/home-*; do
				[ -d "$home" ] || continue
				[ -f "$home/event-trace.jsonl" ] || seed_streams "$home"
			done
			sleep 0.1
			waited=$((waited + 1))
		done
	) &
	local seeder=$!
	env "$@" \
		YG="$WORK/bin/yggterm" \
		GL_AB_MEASURE="$WORK/bin/measure" \
		ARMS="$arms" N=3 SAMPLE_S=1 WARM_S=0 LAUNCH_S=1 \
		bash "$HERE/gl_ab_experiment.sh" "$outdir" >"$outdir.log" 2>&1
	local status=$?
	kill "$seeder" 2>/dev/null
	wait "$seeder" 2>/dev/null
	return $status
}

# ---------------------------------------------------------------------------
echo "gl_ab self-test in $WORK"

echo "[1/7] the orphan detector is not blind"
YGGTERM_HOME="$WORK/probe-home" YGGTERM_FORCE_SOFTWARE_GL=1 "$WORK/bin/yggterm" >/dev/null 2>&1 &
PROBE_PID=$!
sleep 1
if [ "$(pgrep -fc yggterm-gl-ab-standin 2>/dev/null || true)" -ge 1 ]; then
	ok "a live stand-in is visible to the orphan check in [7]"
else
	fail "the orphan check in [7] matches nothing even with a stand-in RUNNING — it could only pass"
fi
kill "$PROBE_PID" 2>/dev/null
wait "$PROBE_PID" 2>/dev/null
pkill -f yggterm-gl-ab-standin 2>/dev/null

echo "[2/7] the readers' own self-tests"
python3 "$HERE/gl_ab_verify_env.py" --self-test || fail "gl_ab_verify_env self-test"
python3 "$HERE/gl_ab_analyze.py" --self-test >/dev/null || fail "gl_ab_analyze self-test"
ok "verifier and analyzer self-tests pass"

echo "[3/7] a full software + hardware run completes and produces a verdict"
OUT1="$WORK/run-clean"
run_experiment "$OUT1" "S H S2"
STATUS=$?
if [ "$STATUS" -ne 0 ]; then
	fail "a clean 3-arm run exited $STATUS; see $OUT1.log"
	tail -20 "$OUT1.log" >&2
fi
for arm in S H S2; do
	grep -q "^$arm policy=" "$OUT1/arm-verification.txt" 2>/dev/null ||
		fail "arm $arm was never verified — verify_arm did not complete"
	grep -q "\"arm\":\"$arm\"" "$OUT1/samples.jsonl" 2>/dev/null ||
		fail "arm $arm produced no samples"
done
grep -q "^H policy=hardware_gl_probed software_keys_absent=yes" "$OUT1/arm-verification.txt" 2>/dev/null ||
	fail "the hardware arm's verdict was not read back correctly"
grep -q "^S policy=software_gl_forced" "$OUT1/arm-verification.txt" 2>/dev/null ||
	fail "the software arm's verdict was not read back correctly"
[ "$FAILURES" -eq 0 ] && ok "3 arms verified and sampled, analyzer ran"

echo "[4/7] rotated generations reach the analyzer"
ls "$OUT1"/S.event-trace.g*.jsonl >/dev/null 2>&1 ||
	fail "the rotated event-trace generation was not copied out of the arm's home"
ls "$OUT1"/S.perf-telemetry.g*.jsonl >/dev/null 2>&1 ||
	fail "the rotated perf-telemetry generation was not copied out of the arm's home"
grep -q "samples inside a focused interval" "$OUT1.log" ||
	fail "focus could not be established — the transition lives in a rotated generation"
ok "generational streams copied and read"

echo "[5/7] the run can still say THIS SETTLES NOTHING"
OUT2="$WORK/run-idle"
run_experiment "$OUT2" "S H" SELFTEST_IDLE_ARM=H
grep -q "THIS RUN SETTLES NOTHING" "$OUT2.log" ||
	fail "an idle hardware arm did not void the run; see $OUT2.log"
grep -q "IDLE window" "$OUT2.log" ||
	fail "the void did not name the 523-of-532 idle signature"
ok "an idle hardware arm voids the run, with the reason"

echo "[6/7] an inherited software GL key is caught on a hardware arm"
OUT3="$WORK/run-leak"
run_experiment "$OUT3" "H" SELFTEST_LEAK_KEY=WEBKIT_DISABLE_COMPOSITING_MODE
if grep -q "a software GL key survived the scrub" "$OUT3.log"; then
	ok "WEBKIT_DISABLE_COMPOSITING_MODE is caught (the key the old check omitted)"
else
	fail "an inherited WEBKIT_DISABLE_COMPOSITING_MODE was NOT caught; see $OUT3.log"
fi

# Run [6] ABORTED mid-arm: that is the path that used to orphan a GUI (every
# `die` in verify_arm skipped the kill at the end of run_arm) and its daemon,
# still holding YGGTERM_HOME — which the next run's `rm -rf` then deleted under
# it. The marker is planted by this file's stand-ins, so the check cannot pass
# by matching nothing; step [0] proved the pattern matches a live one.
echo "[7/7] a refusal leaves no orphan GUI or compositor"
sleep 1
LEAKED=$(pgrep -fc "yggterm-gl-ab-standin" 2>/dev/null || true)
if [ "${LEAKED:-0}" -ne 0 ]; then
	fail "$LEAKED stand-in process(es) survived the aborted run — the EXIT trap does not reap the arm GUI"
	pkill -f "yggterm-gl-ab-standin" 2>/dev/null
else
	ok "no orphaned arm GUI or compositor"
fi

echo
if [ "$FAILURES" -eq 0 ]; then
	echo "gl_ab self-test PASSED (workdir $WORK)"
	exit 0
fi
echo "gl_ab self-test FAILED: $FAILURES check(s); workdir kept at $WORK"
exit 1
