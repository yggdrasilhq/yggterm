#!/usr/bin/env bash
# Report whether yggterm's GUI and its WebKit web processes are rasterizing on the
# GPU or on the CPU, and what that costs. Run it before and after any change to the
# compositing decision (see docs/optimization-pass.md section 1a).
#
# The verdict comes from two independent facts, because either one alone can lie:
#
#   1. DRM engine time. llvmpipe needs no DRM node at all, so a process that has
#      loaded Mesa's gallium but holds ZERO /dev/dri fds is rasterizing on CPU by
#      construction. A nonzero drm-engine-gfx in fdinfo is the positive proof that
#      the GPU really is doing the work.
#   2. CPU-seconds, read from /proc/<pid>/stat as a DELTA over a measured interval.
#      Never `ps %CPU` — that is a lifetime average, and reading it as current load
#      is the mistake this whole workstream was founded on.
#
# /proc/<pid>/environ is NOT a witness here: configure_linux_webkit_compositing()
# sets the GL variables with set_var() after exec, so they never appear in it.
# Absence there says nothing.

set -uo pipefail

INTERVAL="${1:-10}"
HZ=$(getconf CLK_TCK 2>/dev/null || echo 100)

cpu_ticks() { awk '{print $14 + $15}' "/proc/$1/stat" 2>/dev/null || echo 0; }

dri_fd_count() {
	local n
	n=$(find "/proc/$1/fd" -maxdepth 1 -type l 2>/dev/null \
		-exec readlink {} + 2>/dev/null | grep -c '/dev/dri/')
	echo "${n:-0}"
}

# Sum engine time ONCE PER DRM CLIENT, not once per fd. Duplicated fds share one
# struct file and every one of them reports the SAME cumulative counter, so a
# naive per-fd sum multiplies by the fd count — measured 5.00x on Xorg and 4.00x
# on a compositor. `drm-client-id` is the kernel's own dedup key and sits right
# there in the same file.
drm_gfx_ns() {
	local total=0
	total=$(awk '
		/^drm-client-id:/ { client = $2 }
		/^drm-engine-gfx:/ { if (!(client in seen)) { seen[client] = 1; sum += $2 } }
		END { print sum + 0 }
	' "/proc/$1"/fdinfo/* 2>/dev/null)
	echo "${total:-0}"
}

gui_pid=$(pgrep -x yggterm | while read -r p; do
	# The supervisor shim forks the real GUI as a child; the child owns the window.
	[ -n "$(pgrep -P "$p" -x yggterm)" ] || echo "$p"
done | head -1)

if [ -z "${gui_pid:-}" ]; then
	echo "no yggterm GUI process found" >&2
	exit 1
fi

pids=("$gui_pid")
while read -r p; do [ -n "$p" ] && pids+=("$p"); done < <(pgrep -P "$gui_pid")

declare -A t0 g0
for p in "${pids[@]}"; do
	t0[$p]=$(cpu_ticks "$p")
	g0[$p]=$(drm_gfx_ns "$p")
done

sleep "$INTERVAL"

printf '%-8s %-22s %8s %8s %10s %s\n' PID COMM CORES DRI_FDS GPU_MS VERDICT
gpu_total=0
total_dri_fds=0
for p in "${pids[@]}"; do
	[ -d "/proc/$p" ] || continue
	comm=$(cat "/proc/$p/comm" 2>/dev/null || echo '?')
	dticks=$(( $(cpu_ticks "$p") - ${t0[$p]} ))
	cores=$(awk -v d="$dticks" -v hz="$HZ" -v s="$INTERVAL" 'BEGIN{printf "%.3f", d/hz/s}')
	fds=$(dri_fd_count "$p")
	gpu_ms=$(( ($(drm_gfx_ns "$p") - ${g0[$p]}) / 1000000 ))
	gpu_total=$((gpu_total + gpu_ms))
	total_dri_fds=$((total_dri_fds + fds))

	if [ "$gpu_ms" -gt 0 ]; then
		verdict='GPU rasterizing'
	elif [ "$fds" -eq 0 ]; then
		verdict='CPU (no DRM node open)'
	else
		verdict='hardware GL, idle this window'
	fi
	printf '%-8s %-22s %8s %8s %10s %s\n' "$p" "$comm" "$cores" "$fds" "$gpu_ms" "$verdict"
done

echo
# Read the two facts in the right order. Holding a DRM render node is the
# structural answer — llvmpipe never opens one — while GPU engine time is a
# WORKLOAD answer, and an idle window legitimately reports zero on a perfectly
# hardware-accelerated process. Judging "software" from engine time alone was
# wrong in exactly the direction that matters: it reported software on the first
# window after the GPU was switched back on, because nothing happened to be
# painting.
if [ "$total_dri_fds" -eq 0 ]; then
	echo "VERDICT: no yggterm process holds a DRM render node — software rasterization."
	echo "llvmpipe needs no DRM node, so this is structural, not a workload artefact."
	echo "Cross-check that the host itself can: other desktop apps should show nonzero"
	echo "drm-engine-gfx. If they do and we do not, the premise is ours, not the host's."
elif [ "$gpu_total" -eq 0 ]; then
	echo "VERDICT: hardware GL is in use (${total_dri_fds} DRM fds held), but nothing"
	echo "painted in this ${INTERVAL}s window. Zero engine time here means IDLE, not"
	echo "software — re-run while the terminal is actually rendering to see it rise."
else
	echo "VERDICT: hardware GL, and the GPU did ${gpu_total} ms of work in ${INTERVAL}s."
fi
