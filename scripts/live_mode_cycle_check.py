#!/usr/bin/env python3
import argparse
import json
import subprocess
import sys
import time
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Drive a running Yggterm app over SSH app-control and time preview/terminal readiness."
    )
    parser.add_argument("--host", default="jojo")
    parser.add_argument("--bin", default="~/.local/bin/yggterm")
    parser.add_argument("--out-dir", default="/tmp/yggterm-live-mode-cycle")
    parser.add_argument("--timeout", type=float, default=45.0)
    parser.add_argument("--poll", type=float, default=0.75)
    return parser.parse_args()


def run_ssh(host: str, command: str, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["ssh", host, command],
        check=check,
        text=True,
        capture_output=True,
    )


def run_json(host: str, command: str) -> dict:
    result = run_ssh(host, command)
    return json.loads(result.stdout)


def app_state(host: str, binary: str, timeout_ms: int = 15000) -> dict:
    response = run_json(host, f"{binary} server app state --timeout-ms {timeout_ms}")
    return response.get("data") or {}


def open_view(host: str, binary: str, session_path: str, view: str, timeout_ms: int = 15000) -> dict:
    return run_json(
        host,
        f"{binary} server app open {json.dumps(session_path)} --view {view} --timeout-ms {timeout_ms}",
    )


def capture(host: str, binary: str, remote_path: str, local_path: Path, timeout_ms: int = 15000) -> None:
    run_ssh(host, f"{binary} server app screenshot {json.dumps(remote_path)} --timeout-ms {timeout_ms}")
    subprocess.run(["scp", f"{host}:{remote_path}", str(local_path)], check=True)


def wait_until(label: str, timeout_s: float, poll_s: float, predicate):
    start = time.monotonic()
    last_state = None
    last_error = None
    while time.monotonic() - start <= timeout_s:
        try:
            state = predicate()
            last_state = state
            return time.monotonic() - start, state
        except Exception as error:  # noqa: BLE001
            last_error = error
            time.sleep(poll_s)
    if last_error is not None:
        raise RuntimeError(f"{label} timed out after {timeout_s:.1f}s: {last_error}") from last_error
    raise RuntimeError(f"{label} timed out after {timeout_s:.1f}s with no usable state")


def preview_ready(state: dict) -> bool:
    if state.get("active_view_mode") != "Rendered":
        return False
    dom = state.get("dom") or {}
    shell = state.get("shell") or {}
    return (
        dom.get("preview_scroll_count", 0) > 0
        and len((dom.get("preview_text_sample") or "").strip()) > 0
        and not shell.get("terminal_attach_in_flight")
    )


def terminal_ready(state: dict) -> bool:
    if state.get("active_view_mode") != "Terminal":
        return False
    dom = state.get("dom") or {}
    shell = state.get("shell") or {}
    hosts = dom.get("terminal_hosts") or []
    if shell.get("terminal_attach_in_flight"):
        return False
    if dom.get("terminal_host_count", 0) <= 0 or not hosts:
        return False
    sample = (hosts[0].get("text_sample") or "").strip()
    return bool(sample) or hosts[0].get("canvas_count", 0) > 0


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    binary = args.bin
    host = args.host

    baseline = app_state(host, binary)
    active_path = baseline.get("active_session_path")
    if not active_path:
        raise RuntimeError("no active session path in running app state")

    remote_initial = f"/tmp/yggterm-mode-cycle-initial-{int(time.time())}.png"
    remote_preview = f"/tmp/yggterm-mode-cycle-preview-{int(time.time())}.png"
    remote_terminal = f"/tmp/yggterm-mode-cycle-terminal-{int(time.time())}.png"
    capture(host, binary, remote_initial, out_dir / "initial.png")

    started = time.monotonic()
    open_view(host, binary, active_path, "preview")
    preview_elapsed, preview_state = wait_until(
        "preview readiness",
        args.timeout,
        args.poll,
        lambda: _require_ready(app_state(host, binary), preview_ready),
    )
    capture(host, binary, remote_preview, out_dir / "preview-ready.png")

    open_view(host, binary, active_path, "terminal")
    terminal_elapsed, terminal_state = wait_until(
        "terminal readiness",
        args.timeout,
        args.poll,
        lambda: _require_ready(app_state(host, binary), terminal_ready),
    )
    capture(host, binary, remote_terminal, out_dir / "terminal-ready.png")

    summary = {
        "host": host,
        "active_session_path": active_path,
        "started_at_epoch_s": time.time(),
        "elapsed_total_s": round(time.monotonic() - started, 3),
        "preview_ready_s": round(preview_elapsed, 3),
        "terminal_ready_s": round(terminal_elapsed, 3),
        "preview_notifications_count": (preview_state.get("shell") or {}).get("notifications_count"),
        "terminal_notifications_count": (terminal_state.get("shell") or {}).get("notifications_count"),
        "preview_active_surface_requests": len(preview_state.get("active_surface_requests") or []),
        "terminal_active_surface_requests": len(terminal_state.get("active_surface_requests") or []),
        "preview_machine_refresh_requests": ((preview_state.get("remote") or {}).get("machine_refresh_requests")),
        "terminal_machine_refresh_requests": ((terminal_state.get("remote") or {}).get("machine_refresh_requests")),
        "preview_terminal_attach_in_flight": ((preview_state.get("shell") or {}).get("terminal_attach_in_flight")),
        "terminal_terminal_attach_in_flight": ((terminal_state.get("shell") or {}).get("terminal_attach_in_flight")),
        "artifacts": {
            "initial": str(out_dir / "initial.png"),
            "preview": str(out_dir / "preview-ready.png"),
            "terminal": str(out_dir / "terminal-ready.png"),
        },
    }
    summary_path = out_dir / "summary.json"
    summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(summary_path)
    print(json.dumps(summary, indent=2))
    return 0


def _require_ready(state: dict, ready_pred):
    if not ready_pred(state):
        raise RuntimeError("not ready yet")
    return state


if __name__ == "__main__":
    raise SystemExit(main())
