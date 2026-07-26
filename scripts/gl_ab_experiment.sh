#!/usr/bin/env bash
# The matched-load A/B/C that settles whether hardware GL costs the UI process
# more than software GL did — or reports that it settles nothing.
#
# WHY THIS IS NOT TWO ARMS. Turning hardware GL on arms three things at once:
# the GL flip, the DMABuf renderer, and Phase-F under-glass. Two arms cannot
# separate the flip from the arming, and that separation IS the open question.
# So:
#
#   S   YGGTERM_FORCE_SOFTWARE_GL=1        llvmpipe + SHM + legacy stacking:
#                                          byte-for-byte the pre-2.12.14 path
#   H   (no GL env)                        probe => hardware + DMABuf +
#                                          under-glass armed: the 2.12.14 default
#   G   YGGTERM_WEB_SURFACE_UNDER_GLASS=0  hardware + DMABuf, glass OFF
#   S2  = S again, at the END               the DRIFT CONTROL
#
#   G - S   the GL flip alone
#   H - G   under-glass alone
#   H - S   what 2.12.14 did end to end
#   S2 - S  how much the machine moved on its own. ANY contrast smaller than
#           this is indistinguishable from drift, and the analyzer says so.
#
# WHAT IT REFUSES TO DO. Every per-sample measurement goes through
# gl_ab_measure.sh, which already refuses to report a number unless the window
# was focused, the session under test was the one on screen, the active session
# did not move mid-sample, and the arm cost more than a floor. This script adds
# the arm-level refusals that a single sample cannot see:
#
#   - the private compositor really got its OWN wayland socket (or the "lab"
#     window is on the user's desktop and the user's work is the load)
#   - the arm's GL environment RESOLVED to what the arm asked for, asserted
#     from the client's own `webkit_gl_environment` and by the DRM fd count
#   - the nine GL keys are scrubbed before every launch
#
# ⚠ THE SCRUB IS LOAD-BEARING, not hygiene. `shm_force_for_arming` returns
# `Keep` when WEBKIT_DISABLE_DMABUF_RENDERER is ALREADY set, and an agent shell
# inherits the GL keys from the GUI that spawned its terminal. Unscrubbed, arm H
# silently lands on SHM and arm G on llvmpipe while both still REPORT hardware,
# and the experiment compares software against software. verify_arm asserts the
# ABSENCE of the keys, not just the policy string. Do not weaken that.
#
# ⚠ Launched WITHOUT --supervise on purpose: run_supervisor restarts the child
# with the SUPERVISOR's environment, so a crash would silently restart into a
# different arm and the samples would be averaged together.
#
# ⚠ A fresh GUI re-resumes its sessions on fresh PTYs, so the first minutes are
# a repaint storm. WARM_S discards it. Shortening WARM_S leaks warm-up into the
# arm — visible in the live data, where one generation read 0.115 cores at t+0,
# 0.214 at t+12min and 0.123 at t+19min.
#
# ⛔ EXCLUSIVE USE OF THE HOST IS REQUIRED. If another agent is A/B-ing the same
# machine, or the user is working on it, every arm is contaminated. Check first.
#
# Usage:
#   scripts/gl_ab_experiment.sh [outdir]
# Environment:
#   ARMS="S H G S2"   which arms, in order          (default: all four)
#   N=240             samples per arm               (default 240)
#   SAMPLE_S=5        seconds per sample            (default 5)
#   WARM_S=300        warm-up discarded per arm     (default 300)
#   LAUNCH_S=15       settle time before verify_arm (default 15)
#   LINES_PER_S=20    deterministic paint rate      (default 20)
#   YG=...            yggterm binary                (default ~/.local/bin/yggterm)
#   GL_AB_MEASURE=... per-sample measurement script (default gl_ab_measure.sh;
#                     scripts/gl_ab_selftest.sh is the only caller that should
#                     ever set it — the refusals that make a sample trustworthy
#                     live in the default)
#
# Smoke it before committing 90 minutes:
#   ARMS="S H" WARM_S=120 N=60 scripts/gl_ab_experiment.sh /tmp/glab-smoke
#
# Prove the harness itself still works, on any machine, GUI or not:
#   scripts/gl_ab_selftest.sh

set -uo pipefail

OUT="${1:-/tmp/gl-ab-$(date +%Y%m%d-%H%M%S)}"
ARMS="${ARMS:-S H G S2}"
N="${N:-240}"
SAMPLE_S="${SAMPLE_S:-5}"
WARM_S="${WARM_S:-300}"
LAUNCH_S="${LAUNCH_S:-15}"
LINES_PER_S="${LINES_PER_S:-20}"
YG="${YG:-$HOME/.local/bin/yggterm}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MEASURE="${GL_AB_MEASURE:-$HERE/gl_ab_measure.sh}"
VERIFY_ENV="$HERE/gl_ab_verify_env.py"

# The keys that can decide the GL path BEHIND OUR BACK: the shell half of
# yggterm_core::gl_probe::GL_PROBE_STRIPPED_ENV. `verify_arm` asserts a hardware
# arm published NONE of them; the Rust drift lock
# (gl_probe.rs::the_gl_ab_harness_scrubs_every_key_the_probe_strips) keeps this
# list from falling behind the binary's.
#
# ONE list, used twice. It was two: the scrub knew four keys and the absence
# assertion inlined its own three, dropping WEBKIT_DISABLE_COMPOSITING_MODE —
# the exact key an inheriting agent shell carries, and the one that makes an arm
# report hardware while presenting over SHM.
SOFTWARE_GL_KEYS=(
	LIBGL_ALWAYS_SOFTWARE
	GALLIUM_DRIVER
	WEBKIT_DISABLE_DMABUF_RENDERER
	WEBKIT_DISABLE_COMPOSITING_MODE
)

# Every GL key that can decide an arm behind our back. `env -u` all of them on
# every launch; `verify_arm` then asserts the software-forcing ones are still
# absent afterwards.
GL_KEYS=(
	"${SOFTWARE_GL_KEYS[@]}"
	YGGTERM_FORCE_SOFTWARE_GL
	YGGTERM_ENABLE_WEBKIT_COMPOSITING
	YGGTERM_WEBKIT_GL_POLICY
	YGGTERM_WEB_SURFACE_UNDER_GLASS
	MESA_LOADER_DRIVER_OVERRIDE
)

die() { echo "ABORT: $*" >&2; exit 1; }
note() { echo "[$(date +%H:%M:%S)] $*" >&2; }

[ -x "$YG" ] || die "no yggterm binary at $YG"
[ -x "$MEASURE" ] || die "missing $MEASURE — this script measures THROUGH it, on purpose"
[ -r "$VERIFY_ENV" ] || die "missing $VERIFY_ENV — an arm that cannot be verified must not run"
command -v sway >/dev/null || die "sway is the headless compositor backend; not found"
mkdir -p "$OUT" || die "cannot create $OUT"

SAMPLES="$OUT/samples.jsonl"
: >"$SAMPLES"

# ---------------------------------------------------------------------------
# arm environment: the ONE place an arm's identity is expressed
# ---------------------------------------------------------------------------
arm_env() { # arm -> KEY=VALUE lines, empty for H
	case "$1" in
	S | S2) echo "YGGTERM_FORCE_SOFTWARE_GL=1" ;;
	H) : ;;
	G) echo "YGGTERM_WEB_SURFACE_UNDER_GLASS=0" ;;
	*) die "unknown arm $1" ;;
	esac
}

arm_expects_hardware() { case "$1" in S | S2) return 1 ;; *) return 0 ;; esac; }

# ---------------------------------------------------------------------------
# isolation: private YGGTERM_HOME on a private headless compositor
# ---------------------------------------------------------------------------
start_compositor() {
	COMP_SOCK="ygglab-$$-$RANDOM"
	WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 \
		sway --unsupported-gpu >"$OUT/sway.log" 2>&1 &
	COMP_PID=$!
	for _ in $(seq 40); do
		sleep 0.25
		for candidate in "${XDG_RUNTIME_DIR:-/run/user/$(id -u)}"/wayland-*; do
			case "$candidate" in *.lock) continue ;; esac
			[ -S "$candidate" ] || continue
			if [ -n "$(lsof -t "$candidate" 2>/dev/null | grep -x "$COMP_PID")" ]; then
				LAB_WAYLAND="$(basename "$candidate")"
				return 0
			fi
		done
	done
	return 1
}

# ---------------------------------------------------------------------------
# arm verification: did this arm actually TAKE?
# ---------------------------------------------------------------------------
verify_arm() { # arm gui_pid -> 0 or die
	local arm="$1" pid="$2"
	local identity
	identity=$("$YG" server app desktop-identity 2>/dev/null) || die "[$arm] no desktop-identity"
	echo "$identity" >"$OUT/$arm.desktop-identity.json"

	# Read the CLIENT's own view. NEVER /proc/<pid>/environ beside it: that is
	# the exec-time environment, and every GL key is written after exec, so it
	# shows nothing on a fresh launch and the PREDECESSOR's decision after a
	# hot restart.
	#
	# The reader is a FILE taking the report as an argument, not a heredoc
	# taking it on stdin. It was the latter, with the JSON ALSO on stdin as a
	# herestring: two redirections, one fd, the herestring last — so python read
	# the JSON as its program and this function died on every arm of every run.
	# One thing on fd 0, and the reader is self-testable on a machine with no
	# GUI at all.
	local policy absent
	read -r policy absent < <(python3 - "$arm" <<-'PY' <<<"$identity"
		import json, sys
		keys = ("LIBGL_ALWAYS_SOFTWARE", "GALLIUM_DRIVER", "WEBKIT_DISABLE_DMABUF_RENDERER")
		doc = json.load(sys.stdin)
		env = {}
		def walk(node):
		    if isinstance(node, dict):
		        if "webkit_gl_environment" in node and isinstance(node["webkit_gl_environment"], dict):
		            env.update(node["webkit_gl_environment"])
		        for value in node.values():
		            walk(value)
		    elif isinstance(node, list):
		        for value in node:
		            walk(value)
		walk(doc)
		policy = env.get("YGGTERM_WEBKIT_GL_POLICY", "MISSING")
		absent = all(key not in env for key in keys)
		print(policy, "yes" if absent else "no")
	PY
	) || die "[$arm] could not read webkit_gl_environment"

	echo "$arm policy=$policy software_keys_absent=$absent" >>"$OUT/arm-verification.txt"

	local fds
	fds=$(ls -l /proc/"$pid"/fd 2>/dev/null | grep -c "/dev/dri/renderD" || true)
	echo "$arm drm_render_fds=$fds" >>"$OUT/arm-verification.txt"

	if arm_expects_hardware "$arm"; then
		[ "$policy" = "hardware_gl_probed" ] || [ "$policy" = "hardware_gl" ] ||
			die "[$arm] wanted hardware, resolved policy=$policy"
		# The absence assertion is the one that catches the inherited-env trap:
		# with WEBKIT_DISABLE_DMABUF_RENDERER already set, shm_force_for_arming
		# returns Keep and the arm presents over SHM while still saying hardware.
		[ "$absent" = "yes" ] ||
			die "[$arm] a software GL key survived the scrub — this arm would report hardware while presenting over SHM"
		[ "${fds:-0}" -gt 0 ] ||
			die "[$arm] hardware arm holds 0 DRM render-node fds; llvmpipe never opens one, so this arm is software"
	else
		[ "$policy" = "software_gl_forced" ] ||
			die "[$arm] wanted software, resolved policy=$policy"
		[ "${fds:-0}" -eq 0 ] ||
			die "[$arm] software arm holds $fds DRM render-node fds — it is not on llvmpipe"
	fi
}

# ---------------------------------------------------------------------------
# one arm
# ---------------------------------------------------------------------------
run_arm() {
	local arm="$1"
	local home="$OUT/home-$arm"
	rm -rf "$home"
	mkdir -p "$home"
	note "arm $arm: launching into $home on wayland=$LAB_WAYLAND"

	local -a scrub=()
	local key
	for key in "${GL_KEYS[@]}"; do scrub+=(-u "$key"); done

	local -a armenv=()
	while read -r line; do [ -n "$line" ] && armenv+=("$line"); done < <(arm_env "$arm")

	# No --supervise: a supervisor restart would come back with the
	# SUPERVISOR's environment, i.e. a different arm, silently.
	env "${scrub[@]}" \
		YGGTERM_HOME="$home" \
		WAYLAND_DISPLAY="$LAB_WAYLAND" \
		"${armenv[@]}" \
		"$YG" >"$OUT/$arm.gui.log" 2>&1 &
	local gui=$!
	# Published BEFORE anything can refuse, so the EXIT trap can reap it. Every
	# `die` below used to skip the kill at the end of this function, so the first
	# refusal orphaned a GUI and its daemon still holding YGGTERM_HOME=$home —
	# and the next run's `rm -rf "$home"` deleted that home under a live daemon.
	ARM_GUI_PID="$gui"
	ARM_GUI_HOME="$home"
	sleep "$LAUNCH_S"
	[ -d "/proc/$gui" ] || die "[$arm] GUI died on launch; see $OUT/$arm.gui.log"

	export YGGTERM_HOME="$home"
	verify_arm "$arm" "$gui"

	# ONE session, created by the harness and left ACTIVE. Only the displayed
	# session's output reaches the renderer — the third botched attempt measured
	# a session that was not on screen and called it a 5x win.
	local session
	session=$("$YG" server terminal new --json 2>/dev/null |
		python3 -c 'import json,sys; print(json.load(sys.stdin).get("session_path",""))') ||
		die "[$arm] could not create the load session"
	[ -n "$session" ] || die "[$arm] terminal new returned no session path"
	note "arm $arm: session $session; warming ${WARM_S}s"

	# Deterministic emitter: a fixed rate of fixed-width lines, so paint
	# exposure is a constant of the experiment and not of the machine's mood.
	"$YG" server terminal send --session "$session" \
		"while :; do for i in \$(seq $LINES_PER_S); do printf '%s\n' \"\$(head -c 100 /dev/zero | tr '\\0' 'x')\"; done; sleep 1; done" \
		>/dev/null 2>&1 || die "[$arm] could not start the emitter"

	sleep "$WARM_S"

	local ok=0 refused=0 i
	for i in $(seq "$N"); do
		if SAMPLE_JSON="$SAMPLES" SAMPLE_TAG="$arm/$i" GUI_PID="$gui" YG="$YG" \
			"$MEASURE" "$arm" "$session" "$SAMPLE_S" >>"$OUT/$arm.samples.log" 2>&1; then
			ok=$((ok + 1))
		else
			refused=$((refused + 1))
		fi
	done
	note "arm $arm: $ok samples accepted, $refused refused by the measurement"

	# EVERY GENERATION, not just the live file. These streams rotate by SIZE
	# (event-trace at 8 MiB, perf-telemetry at 16 MiB) into
	# `<stem>.g<ts_ms>.jsonl`, and an arm runs ~25 minutes under a forced
	# 20-lines/s emitter. A lab GUI that focuses at launch and stays focused
	# emits exactly ONE ui/window_focus/transition — the OLDEST line in the
	# file. Copying only the live file drops it the moment the stream rotates,
	# and the analyzer then voids the arm for "no window_focus trace": a refusal
	# that has nothing to do with GL. Exposure counts degrade the same way, and
	# asymmetrically per arm.
	copy_stream() { # stem
		local stem="$1" generation base
		for generation in "$home/$stem".jsonl "$home/$stem".g*.jsonl "$home/$stem".previous.jsonl; do
			[ -f "$generation" ] || continue
			base="$(basename "$generation")"
			cp "$generation" "$OUT/$arm.$base" 2>/dev/null || true
		done
	}
	copy_stream perf-telemetry
	copy_stream event-trace

	kill "$gui" 2>/dev/null
	wait "$gui" 2>/dev/null
	# The GUI's daemon does NOT die with it: it holds this arm's home, and the
	# next arm's `rm -rf` would delete the home out from under it. YGGTERM_HOME
	# is spelled inline rather than inherited so this can never reach the user's
	# own daemon.
	YGGTERM_HOME="$home" "$YG" server shutdown >/dev/null 2>&1 || true
	ARM_GUI_PID=""
	ARM_GUI_HOME=""
	unset YGGTERM_HOME
	[ "$ok" -gt 0 ] || die "[$arm] every sample was refused — nothing to analyze"
}

# ---------------------------------------------------------------------------
# Reap EVERYTHING this run started, on every exit path including `die`. The
# trap used to kill only the compositor.
ARM_GUI_PID=""
ARM_GUI_HOME=""
cleanup() {
	local status=$?
	if [ -n "$ARM_GUI_PID" ]; then
		kill "$ARM_GUI_PID" 2>/dev/null || true
		wait "$ARM_GUI_PID" 2>/dev/null || true
	fi
	if [ -n "$ARM_GUI_HOME" ]; then
		YGGTERM_HOME="$ARM_GUI_HOME" "$YG" server shutdown >/dev/null 2>&1 || true
	fi
	if [ -n "${COMP_PID:-}" ]; then
		kill "$COMP_PID" 2>/dev/null || true
	fi
	exit "$status"
}
trap cleanup EXIT

start_compositor || die "the private compositor did not get its own wayland socket — refusing to run the lab window on the user's desktop"
note "private compositor pid=$COMP_PID socket=$LAB_WAYLAND"

for arm in $ARMS; do run_arm "$arm"; done

note "done. samples: $SAMPLES"
echo
python3 "$HERE/gl_ab_analyze.py" "$OUT"
