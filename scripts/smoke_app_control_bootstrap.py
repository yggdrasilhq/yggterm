#!/usr/bin/env python3
import argparse
import json
import os
import subprocess
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BIN = Path(os.environ.get("YGGTERM_BIN") or (ROOT / "target" / "debug" / "yggterm"))
ENV = os.environ.copy()
PROBLEM_NOTIFICATION_MARKERS = (
    "connection refused",
    "no such file or directory",
    "server unavailable",
    "daemon did not become reachable",
    "local yggterm daemon",
    "reading daemon response",
    "parsing daemon response",
    "timed out waiting for app control response",
    "failed sock",
    "sock connection",
    ".sock",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Minimal cross-platform app-control smoke for a live Yggterm GUI client."
    )
    parser.add_argument("--bin", default=str(BIN))
    parser.add_argument("--pid", type=int)
    parser.add_argument("--out", required=True)
    parser.add_argument("--timeout-ms", type=int, default=15000)
    parser.add_argument("--wait-seconds", type=float, default=20.0)
    parser.add_argument("--require-visible", action="store_true", default=True)
    parser.add_argument("--allow-hidden", action="store_true")
    parser.add_argument(
        "--expect-live-blur",
        choices=("ignore", "required", "forbidden"),
        default="ignore",
    )
    return parser.parse_args()


def run(bin_path: Path, *args: str, timeout_seconds: float = 20.0) -> dict:
    timeout_with_cushion = timeout_seconds + 5.0
    if "--timeout-ms" in args:
        try:
            timeout_arg = args[args.index("--timeout-ms") + 1]
            timeout_with_cushion = max(timeout_with_cushion, float(timeout_arg) / 1000.0 + 5.0)
        except (IndexError, ValueError):
            pass
    proc = subprocess.run(
        [str(bin_path), *args],
        cwd=ROOT,
        text=True,
        capture_output=True,
        env=ENV,
        timeout=timeout_with_cushion,
    )
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip() or proc.stdout.strip() or f"command failed: {args!r}")
    text = proc.stdout.strip()
    return json.loads(text) if text else {}


def choose_pid(bin_path: Path, timeout_ms: int) -> int:
    payload = run(
        bin_path,
        "server",
        "app",
        "clients",
        "--timeout-ms",
        str(timeout_ms),
    )
    clients = list(payload.get("clients") or [])
    if not clients:
        raise RuntimeError("no live Yggterm GUI clients are registered for app control")
    chosen = sorted(clients, key=lambda item: int(item.get("started_at_ms") or 0))[-1]
    pid = int(chosen.get("pid") or 0)
    if pid <= 0:
        raise RuntimeError(f"chosen client did not expose a pid: {chosen!r}")
    return pid


def app_state(bin_path: Path, pid: int, timeout_ms: int) -> dict:
    payload = run(
        bin_path,
        "server",
        "app",
        "state",
        "--pid",
        str(pid),
        "--timeout-ms",
        str(timeout_ms),
    )
    data = payload.get("data")
    if not isinstance(data, dict):
        raise RuntimeError(f"app state response missing data payload: {payload!r}")
    return data


def app_rows(bin_path: Path, pid: int, timeout_ms: int) -> dict:
    payload = run(
        bin_path,
        "server",
        "app",
        "rows",
        "--pid",
        str(pid),
        "--timeout-ms",
        str(timeout_ms),
    )
    data = payload.get("data")
    if isinstance(data, dict):
        return data
    return payload


def app_screenshot(bin_path: Path, pid: int, output_path: Path, timeout_ms: int) -> dict:
    output_path.parent.mkdir(parents=True, exist_ok=True)
    payload = run(
        bin_path,
        "server",
        "app",
        "screenshot",
        "--pid",
        str(pid),
        str(output_path),
        "--timeout-ms",
        str(timeout_ms),
        timeout_seconds=max(20.0, timeout_ms / 1000.0 + 10.0),
    )
    return payload


def screenshot_backend(payload: dict | None) -> str | None:
    data = (payload or {}).get("data")
    if not isinstance(data, dict):
        return None
    backend = data.get("capture_backend")
    return str(backend) if backend else None


def screenshot_backend_attempts(payload: dict | None) -> list[str]:
    data = (payload or {}).get("data")
    if not isinstance(data, dict):
        return []
    attempts = data.get("capture_backend_attempts")
    if not isinstance(attempts, list):
        return []
    return [str(item) for item in attempts if str(item).strip()]


def problem_notifications(state: dict) -> list[dict]:
    shell = state.get("shell") or {}
    notifications = []
    history = shell.get("notifications")
    if isinstance(history, list):
        notifications.extend(history)
    visible = shell.get("visible_notifications")
    if isinstance(visible, list):
        notifications.extend(visible)
    bad = []
    seen = set()
    for notification in notifications:
        if not isinstance(notification, dict):
            continue
        identifier = notification.get("id")
        if identifier in seen:
            continue
        seen.add(identifier)
        haystack = " ".join(
            str(notification.get(key) or "")
            for key in ("title", "message", "tone")
        ).lower()
        if any(marker in haystack for marker in PROBLEM_NOTIFICATION_MARKERS):
            bad.append(notification)
    return bad


def blur_summary(state: dict) -> dict:
    shell = state.get("shell") or {}
    dom = state.get("dom") or {}
    return {
        "live_blur_supported": shell.get("live_blur_supported"),
        "transparent_window": shell.get("transparent_window"),
        "profile_reason": shell.get("transparent_window_profile_reason"),
        "shell_frame_backdrop_filter": dom.get("shell_frame_backdrop_filter"),
        "shell_root_backdrop_filter": dom.get("shell_root_backdrop_filter"),
        "shell_frame_background": dom.get("shell_frame_background"),
        "shell_root_background": dom.get("shell_root_background"),
    }


def assert_blur_expectation(state: dict, expectation: str) -> dict:
    summary = blur_summary(state)
    if (
        expectation != "ignore"
        and summary.get("live_blur_supported") is None
        and summary.get("transparent_window") is None
        and summary.get("profile_reason") is None
    ):
        raise RuntimeError(
            "app state did not expose blur observability fields; the staged binary is likely stale "
            f"or missing the current app-control contract: {summary!r}"
        )
    live = bool(summary.get("live_blur_supported"))
    if expectation == "required" and not live:
        raise RuntimeError(f"expected live blur support but state reported otherwise: {summary!r}")
    if expectation == "required":
        transparent = summary.get("transparent_window")
        backdrop = str(summary.get("shell_frame_backdrop_filter") or "").strip().lower()
        if transparent is False:
            raise RuntimeError(f"expected a transparent live-blur window but state reported otherwise: {summary!r}")
        if backdrop in ("", "none"):
            raise RuntimeError(f"expected a live backdrop blur but state reported otherwise: {summary!r}")
    if expectation == "forbidden" and live:
        raise RuntimeError(f"expected no live blur support but state reported otherwise: {summary!r}")
    return summary


def wait_for_ready_state(
    bin_path: Path,
    pid: int,
    timeout_ms: int,
    wait_seconds: float,
    require_visible: bool,
) -> dict:
    deadline = time.time() + wait_seconds
    last_state = {}
    last_error = ""
    while time.time() < deadline:
        try:
            last_state = app_state(bin_path, pid, timeout_ms)
            window = last_state.get("window") or {}
            shell = last_state.get("shell") or {}
            dom = last_state.get("dom") or {}
            visible = bool(window.get("visible"))
            if require_visible and not visible:
                raise RuntimeError("window not visible yet")
            if shell.get("needs_initial_server_sync"):
                raise RuntimeError("initial server sync still in progress")
            if shell.get("server_busy"):
                raise RuntimeError("server still busy")
            if dom.get("shell_root_count") != 1:
                raise RuntimeError(f"unexpected shell root count: {dom.get('shell_root_count')!r}")
            bad_notifications = problem_notifications(last_state)
            if bad_notifications:
                raise RuntimeError(f"bad daemon/socket notifications observed: {bad_notifications!r}")
            return last_state
        except Exception as exc:  # noqa: BLE001
            last_error = str(exc)
            time.sleep(0.25)
    raise RuntimeError(
        f"app state did not become ready for pid {pid} within {wait_seconds:.1f}s: {last_error} state={last_state!r}"
    )


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    bin_path = Path(args.bin).expanduser().resolve()
    require_visible = bool(args.require_visible and not args.allow_hidden)
    clients_payload = run(
        bin_path,
        "server",
        "app",
        "clients",
        "--timeout-ms",
        str(args.timeout_ms),
    )
    clients = list(clients_payload.get("clients") or [])
    pid = args.pid or choose_pid(bin_path, args.timeout_ms)
    state = wait_for_ready_state(
        bin_path,
        pid,
        args.timeout_ms,
        args.wait_seconds,
        require_visible,
    )
    blur = assert_blur_expectation(state, args.expect_live_blur)
    rows = app_rows(bin_path, pid, args.timeout_ms)

    state_path = out_dir / "state.json"
    rows_path = out_dir / "rows.json"
    screenshot_path = out_dir / "window.png"
    summary_path = out_dir / "summary.json"
    state_path.write_text(json.dumps(state, indent=2), encoding="utf-8")
    rows_path.write_text(json.dumps(rows, indent=2), encoding="utf-8")

    screenshot = None
    screenshot_error = None
    try:
        screenshot = app_screenshot(bin_path, pid, screenshot_path, args.timeout_ms)
    except Exception as exc:  # noqa: BLE001
        screenshot_error = str(exc)

    summary = {
        "bin": str(bin_path),
        "pid": pid,
        "clients_count": len(clients),
        "window": state.get("window") or {},
        "client_instance": state.get("client_instance") or {},
        "active_session_path": state.get("active_session_path"),
        "active_view_mode": state.get("active_view_mode"),
        "notifications_count": int(((state.get("shell") or {}).get("notifications_count")) or 0),
        "visible_notifications_count": int(
            ((state.get("shell") or {}).get("visible_notifications_count")) or 0
        ),
        "problem_notifications": problem_notifications(state),
        "blur": blur,
        "state_path": str(state_path),
        "rows_path": str(rows_path),
        "screenshot_path": str(screenshot_path) if screenshot_path.exists() else None,
        "screenshot_response": screenshot,
        "screenshot_backend": screenshot_backend(screenshot),
        "screenshot_backend_attempts": screenshot_backend_attempts(screenshot),
        "screenshot_error": screenshot_error,
    }
    summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(summary_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
