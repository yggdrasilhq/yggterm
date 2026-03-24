#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_BIN="${ROOT_DIR}/target/debug/yggterm"
PERF_FILE="${HOME}/.yggterm/perf-telemetry.jsonl"
SUMMARY_FILE="/tmp/yggterm-perf-summary.txt"
STARTUP_FILE="/tmp/yggterm-startup-measure.txt"
STDOUT_FILE="/tmp/yggterm-gui.out"
STDERR_FILE="/tmp/yggterm-gui.err"
SVG_FILE="/tmp/yggterm-perf.svg"
WAIT_SECS="${WAIT_SECS:-5}"

rm -f "${PERF_FILE}" "${SUMMARY_FILE}" "${STARTUP_FILE}" "${STDOUT_FILE}" "${STDERR_FILE}" "${SVG_FILE}"
pkill -f "${APP_BIN}" || true

start_ms="$(date +%s%3N)"
DISPLAY="${DISPLAY:-:10.0}" "${APP_BIN}" >"${STDOUT_FILE}" 2>"${STDERR_FILE}" &
app_pid="$!"

for _ in $(seq 1 400); do
  win="$(DISPLAY="${DISPLAY:-:10.0}" xdotool search --name 'yggterm' 2>/dev/null | head -n 1 || true)"
  if [[ -n "${win}" ]]; then
    now_ms="$(date +%s%3N)"
    {
      echo "window_ms=$((now_ms - start_ms))"
      echo "window_id=${win}"
    } >"${STARTUP_FILE}"
    break
  fi
  sleep 0.1
done

sleep "${WAIT_SECS}"
kill "${app_pid}" || true
wait "${app_pid}" || true
pkill -f "${APP_BIN} server daemon" || true

if [[ -f "${PERF_FILE}" ]]; then
  "${ROOT_DIR}/scripts/plot-perf-telemetry.py" "${PERF_FILE}" "${SVG_FILE}" >"${SUMMARY_FILE}"
fi

echo "--- startup ---"
cat "${STARTUP_FILE}" 2>/dev/null || true
echo "--- perf summary ---"
cat "${SUMMARY_FILE}" 2>/dev/null || true
echo "--- telemetry tail ---"
tail -n 30 "${PERF_FILE}" 2>/dev/null || true
echo "--- stderr tail ---"
tail -n 80 "${STDERR_FILE}" 2>/dev/null || true
