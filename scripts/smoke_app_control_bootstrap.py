#!/usr/bin/env python3
import argparse
import json
import os
import subprocess
import time
from pathlib import Path

from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
BIN = Path(os.environ.get("YGGTERM_BIN") or (ROOT / "target" / "debug" / "yggterm"))
ENV = os.environ.copy()
PROBLEM_NOTIFICATION_MARKERS = (
    "codex tool refresh failed",
    "remote codex tool refresh failed",
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
MIN_INLINE_TITLEBAR_WIDTH = 1240


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


def app_create_terminal(
    bin_path: Path,
    pid: int,
    timeout_ms: int,
    *,
    title: str = "Bootstrap Smoke Terminal",
) -> dict:
    payload = run(
        bin_path,
        "server",
        "app",
        "terminal",
        "new",
        "--pid",
        str(pid),
        "--title",
        title,
        "--timeout-ms",
        str(timeout_ms),
        timeout_seconds=max(20.0, timeout_ms / 1000.0 + 10.0),
    )
    if payload.get("error"):
        raise RuntimeError(f"terminal creation failed: {payload['error']}")
    data = payload.get("data")
    if not isinstance(data, dict):
        raise RuntimeError(f"terminal creation response missing data payload: {payload!r}")
    session_path = str(data.get("active_session_path") or "").strip()
    if not session_path:
        raise RuntimeError(f"terminal creation did not return an active session path: {payload!r}")
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


def assert_screenshot_file_usable(path: Path) -> dict:
    if not path.exists():
        raise RuntimeError(f"screenshot file was not created: {path}")
    with Image.open(path) as image:
        rgba = image.convert("RGBA")
        extrema = rgba.getextrema()
        if len(extrema) != 4:
            raise RuntimeError(f"unexpected screenshot channel layout for {path}: {extrema!r}")
        if all(channel_max == 0 for _, channel_max in extrema):
            raise RuntimeError(
                f"screenshot {path} is fully blank, all RGBA channels are zero: extrema={extrema!r}"
            )
        if extrema[3][1] == 0:
            raise RuntimeError(
                f"screenshot {path} is fully transparent, alpha never rises above zero: extrema={extrema!r}"
            )
        return {
            "size": {"width": rgba.width, "height": rgba.height},
            "extrema": extrema,
        }


def rect_visible(rect: dict | None) -> bool:
    if not isinstance(rect, dict):
        return False
    try:
        return float(rect.get("width") or 0) > 0 and float(rect.get("height") or 0) > 0
    except (TypeError, ValueError):
        return False


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


def titlebar_right_controls_summary(state: dict) -> dict:
    dom = state.get("dom") or {}
    window = state.get("window") or {}
    inner_size = window.get("inner_size") or {}
    return {
        "window_width": int(inner_size.get("width") or 0),
        "connect_visible": rect_visible(dom.get("titlebar_connect_button_rect")),
        "notifications_visible": rect_visible(dom.get("titlebar_notifications_button_rect")),
        "settings_visible": rect_visible(dom.get("titlebar_settings_button_rect")),
        "metadata_visible": rect_visible(dom.get("titlebar_metadata_button_rect")),
        "overflow_visible": rect_visible(dom.get("titlebar_overflow_button_rect")),
        "right_rect": dom.get("titlebar_right_rect"),
        "connect_rect": dom.get("titlebar_connect_button_rect"),
        "notifications_rect": dom.get("titlebar_notifications_button_rect"),
        "settings_rect": dom.get("titlebar_settings_button_rect"),
        "metadata_rect": dom.get("titlebar_metadata_button_rect"),
        "overflow_rect": dom.get("titlebar_overflow_button_rect"),
    }


def assert_titlebar_utility_buttons_inline(
    state: dict,
    *,
    min_window_width: int = MIN_INLINE_TITLEBAR_WIDTH,
) -> dict:
    summary = titlebar_right_controls_summary(state)
    if summary["window_width"] < min_window_width:
        return summary
    missing = [
        name
        for name in ("notifications_visible", "settings_visible", "metadata_visible")
        if not bool(summary.get(name))
    ]
    if missing or summary.get("overflow_visible"):
        raise RuntimeError(
            "titlebar utility buttons collapsed into overflow despite wide window: "
            f"{summary!r}"
        )
    return summary


def terminal_hosts(state: dict) -> list[dict]:
    dom = state.get("dom") or {}
    hosts = dom.get("terminal_hosts")
    if isinstance(hosts, list):
        return hosts
    viewport = state.get("viewport") or {}
    hosts = viewport.get("terminal_hosts")
    if isinstance(hosts, list):
        return hosts
    return []


def active_terminal_host(state: dict) -> dict | None:
    hosts = terminal_hosts(state)
    if not hosts:
        return None
    active_session_path = str(state.get("active_session_path") or "").strip()
    if active_session_path:
        session_matches = [
            host
            for host in hosts
            if str(host.get("session_path") or "").strip() == active_session_path
        ]
        if session_matches:
            focused_matches = [
                host
                for host in session_matches
                if host.get("helper_textarea_focused") is True
                or host.get("host_has_active_element") is True
            ]
            if focused_matches:
                return focused_matches[-1]
            return session_matches[-1]
    focused_hosts = [
        host
        for host in hosts
        if host.get("helper_textarea_focused") is True or host.get("host_has_active_element") is True
    ]
    if focused_hosts:
        return focused_hosts[-1]
    return hosts[-1]


def assert_active_terminal_host_ready(state: dict, session_path: str | None = None) -> dict:
    active_session_path = str(state.get("active_session_path") or "").strip()
    expected_session_path = str(session_path or active_session_path).strip()
    if not active_session_path:
        raise RuntimeError(f"terminal state is missing an active session path: {state!r}")
    if expected_session_path and active_session_path != expected_session_path:
        raise RuntimeError(
            "terminal state active session drifted away from the created session: "
            f"expected={expected_session_path!r} actual={active_session_path!r}"
        )
    if str(state.get("active_view_mode") or "").strip() != "Terminal":
        raise RuntimeError(f"terminal view did not become active: {state!r}")
    host = active_terminal_host(state)
    if host is None:
        raise RuntimeError(f"app state did not expose any terminal hosts: {state!r}")
    if str(host.get("session_path") or "").strip() != active_session_path:
        raise RuntimeError(
            "active terminal host did not match the active session: "
            f"host={host!r} state={state!r}"
        )
    if not (
        rect_visible(host.get("host_rect"))
        and rect_visible(host.get("screen_rect"))
        and rect_visible(host.get("viewport_rect"))
    ):
        raise RuntimeError(f"terminal host/screen/viewport is not visibly mounted: {host!r}")
    if host.get("xterm_present") is not True or host.get("viewport_present") is not True:
        raise RuntimeError(f"terminal host did not mount xterm/viewport: {host!r}")
    if host.get("input_enabled") is not True:
        raise RuntimeError(f"terminal host input remained disabled: {host!r}")
    if not (
        host.get("helper_textarea_focused") is True or host.get("host_has_active_element") is True
    ):
        raise RuntimeError(f"terminal host focus contract did not land on the active host: {host!r}")
    return {
        "active_session_path": active_session_path,
        "host_id": host.get("id"),
        "rows": host.get("rows"),
        "cols": host.get("cols"),
        "input_enabled": host.get("input_enabled"),
        "helper_textarea_focused": host.get("helper_textarea_focused"),
        "host_has_active_element": host.get("host_has_active_element"),
        "xterm_present": host.get("xterm_present"),
        "viewport_present": host.get("viewport_present"),
    }


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


def assert_sidebar_rows_present(rows: dict) -> dict:
    row_count = int(rows.get("row_count") or len(rows.get("rows") or []))
    if row_count <= 0:
        raise RuntimeError(f"sidebar rows were empty on first boot: {rows!r}")
    first_row = ((rows.get("rows") or [{}])[0] or {})
    return {
        "row_count": row_count,
        "first_row_label": first_row.get("label"),
        "first_row_path": first_row.get("full_path"),
    }


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


def wait_for_terminal_ready_state(
    bin_path: Path,
    pid: int,
    timeout_ms: int,
    wait_seconds: float,
    session_path: str,
) -> tuple[dict, dict]:
    deadline = time.time() + wait_seconds
    last_state = {}
    last_error = ""
    while time.time() < deadline:
        try:
            last_state = app_state(bin_path, pid, timeout_ms)
            bad_notifications = problem_notifications(last_state)
            if bad_notifications:
                raise RuntimeError(f"bad daemon/socket notifications observed: {bad_notifications!r}")
            terminal = assert_active_terminal_host_ready(last_state, session_path)
            return last_state, terminal
        except Exception as exc:  # noqa: BLE001
            last_error = str(exc)
            time.sleep(0.25)
    raise RuntimeError(
        "terminal state did not become ready "
        f"for pid {pid} within {wait_seconds:.1f}s: {last_error} state={last_state!r}"
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
    titlebar = assert_titlebar_utility_buttons_inline(state)
    created_terminal_response = app_create_terminal(bin_path, pid, args.timeout_ms)
    created_terminal = created_terminal_response.get("data") or {}
    created_session_path = str(created_terminal.get("active_session_path") or "").strip()
    state, terminal = wait_for_terminal_ready_state(
        bin_path,
        pid,
        args.timeout_ms,
        args.wait_seconds,
        created_session_path,
    )
    blur = assert_blur_expectation(state, args.expect_live_blur)
    rows = app_rows(bin_path, pid, args.timeout_ms)
    sidebar = assert_sidebar_rows_present(rows)

    state_path = out_dir / "state.json"
    rows_path = out_dir / "rows.json"
    screenshot_path = out_dir / "window.png"
    summary_path = out_dir / "summary.json"
    state_path.write_text(json.dumps(state, indent=2), encoding="utf-8")
    rows_path.write_text(json.dumps(rows, indent=2), encoding="utf-8")

    screenshot = None
    screenshot_error = None
    screenshot_quality = None
    try:
        screenshot = app_screenshot(bin_path, pid, screenshot_path, args.timeout_ms)
        screenshot_quality = assert_screenshot_file_usable(screenshot_path)
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
        "sidebar": sidebar,
        "titlebar_right_controls": titlebar,
        "created_terminal": created_terminal,
        "terminal": terminal,
        "state_path": str(state_path),
        "rows_path": str(rows_path),
        "screenshot_path": str(screenshot_path) if screenshot_path.exists() else None,
        "screenshot_response": screenshot,
        "screenshot_backend": screenshot_backend(screenshot),
        "screenshot_backend_attempts": screenshot_backend_attempts(screenshot),
        "screenshot_quality": screenshot_quality,
        "screenshot_error": screenshot_error,
    }
    summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(summary_path)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
