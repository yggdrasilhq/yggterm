#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP_BIN="${ROOT_DIR}/target/debug/yggterm"
PERF_FILE="${HOME}/.yggterm/perf-telemetry.jsonl"
TRACE_FILE="${HOME}/.yggterm/event-trace.jsonl"
SUMMARY_FILE="/tmp/yggterm-perf-summary.txt"
STARTUP_FILE="/tmp/yggterm-startup-measure.txt"
STDOUT_FILE="/tmp/yggterm-gui.out"
STDERR_FILE="/tmp/yggterm-gui.err"
SVG_FILE="/tmp/yggterm-perf.svg"
WAIT_SECS="${WAIT_SECS:-5}"

rm -f "${PERF_FILE}" "${SUMMARY_FILE}" "${STARTUP_FILE}" "${STDOUT_FILE}" "${STDERR_FILE}" "${SVG_FILE}"
pkill -f "${APP_BIN}" || true

start_ms="$(date +%s%3N)"
DISPLAY="${DISPLAY:-:10.0}" YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF=1 "${APP_BIN}" >"${STDOUT_FILE}" 2>"${STDERR_FILE}" &
app_pid="$!"

for _ in $(seq 1 400); do
  if ! kill -0 "${app_pid}" 2>/dev/null; then
    break
  fi
  spawn_json="$(python3 - <<'PY' "${TRACE_FILE}" "${app_pid}" "${start_ms}"
import json, sys
from pathlib import Path
trace_path = Path(sys.argv[1])
pid = int(sys.argv[2])
start_ms = int(sys.argv[3])
if not trace_path.exists():
    sys.exit(1)
for line in reversed(trace_path.read_text(encoding="utf-8").splitlines()):
    try:
        event = json.loads(line)
    except json.JSONDecodeError:
        continue
    if (event.get("ts_ms") or 0) < start_ms:
        break
    payload = event.get("payload") or {}
    if (
        event.get("pid") == pid
        and event.get("category") == "startup"
        and event.get("name") == "window_spawned"
        and (event.get("ts_ms") or 0) >= start_ms
    ):
        print(json.dumps(payload))
        sys.exit(0)
sys.exit(1)
PY
  )" || spawn_json=""
  if [[ -n "${spawn_json}" ]]; then
    python3 - <<'PY' "${STARTUP_FILE}" "${spawn_json}"
import json, sys
payload = json.loads(sys.argv[2])
window = payload.get("window") or {}
with open(sys.argv[1], "w", encoding="utf-8") as fh:
    fh.write(f"window_ms={payload.get('elapsed_ms', '')}\n")
    fh.write(f"window_id={window.get('window_id', '')}\n")
    fh.write(f"window_pid={window.get('pid', '')}\n")
    fh.write("window_source=trace_window_spawned\n")
PY
    break
  fi
  sleep 0.1
done

if [[ ! -f "${STARTUP_FILE}" ]]; then
  {
    echo "window_ms="
    echo "window_id="
    echo "window_pid=${app_pid}"
    echo "window_source=missing_trace_window_spawned"
  } >"${STARTUP_FILE}"
fi

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
