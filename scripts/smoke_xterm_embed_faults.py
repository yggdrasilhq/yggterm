#!/usr/bin/env python3
import argparse
import colorsys
import json
import os
import shutil
import subprocess
import time
from pathlib import Path

from PIL import Image


ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "target" / "debug" / "yggterm"
ENV = os.environ.copy()
XDO_TIMEOUT_SECONDS = 4.0
PERF_TELEMETRY_MAX_BYTES = 16 * 1024 * 1024
UI_TELEMETRY_MAX_BYTES = 8 * 1024 * 1024
TELEMETRY_BUDGET_SLACK_BYTES = 512 * 1024
IDLE_ROOT_RENDER_SAMPLE_SECONDS = 1.25
IDLE_ROOT_RENDER_MAX_DELTA = 8
SIDEBAR_MIN_WIDTH = 220.0
SIDEBAR_MAX_WIDTH = 420.0
CLIENT_MAIN_RSS_MAX_KB = 512 * 1024
CLIENT_TOTAL_RSS_MAX_KB = 896 * 1024


def run(*args: str, check: bool = True, timeout_seconds: float = 20.0) -> dict:
    try:
        proc = subprocess.run(
            [str(BIN), *args],
            cwd=ROOT,
            text=True,
            capture_output=True,
            env=ENV,
            timeout=timeout_seconds,
        )
    except subprocess.TimeoutExpired as exc:
        rendered = " ".join(str(arg) for arg in args)
        raise AssertionError(f"command timed out after {timeout_seconds:.1f}s: {rendered}") from exc
    if check and proc.returncode != 0:
        raise AssertionError(
            proc.stderr.strip() or proc.stdout.strip() or f"command failed: {args!r}"
        )
    text = proc.stdout.strip()
    return json.loads(text) if text else {}


def app_state(pid: int) -> dict:
    return run("server", "app", "state", "--pid", str(pid), "--timeout-ms", "8000")["data"]


def app_focus(pid: int) -> dict:
    return run("server", "app", "focus", "--pid", str(pid), "--timeout-ms", "8000")


def terminal_send(pid: int, session: str, data: str) -> dict:
    return run(
        "server",
        "app",
        "terminal",
        "send",
        "--pid",
        str(pid),
        session,
        "--data",
        data,
        "--timeout-ms",
        "15000",
    )


def terminal_probe_type(
    pid: int,
    session: str,
    data: str,
    *,
    mode: str = "xterm",
    press_enter: bool = False,
    press_tab: bool = False,
    press_ctrl_c: bool = False,
    press_ctrl_e: bool = False,
    press_ctrl_u: bool = False,
) -> dict:
    args = [
        "server",
        "app",
        "terminal",
        "probe-type",
        "--pid",
        str(pid),
        session,
        "--data",
        data,
        "--mode",
        mode,
        "--timeout-ms",
        "15000",
    ]
    if press_enter:
        args.append("--enter")
    if press_tab:
        args.append("--tab")
    if press_ctrl_c:
        args.append("--ctrl-c")
    if press_ctrl_e:
        args.append("--ctrl-e")
    if press_ctrl_u:
        args.append("--ctrl-u")
    return run(*args)


def app_open(pid: int, session: str, view: str = "terminal") -> dict:
    return run(
        "server",
        "app",
        "open",
        "--pid",
        str(pid),
        session,
        "--view",
        view,
        "--timeout-ms",
        "20000",
    )


def app_open_raw(pid: int, session: str, view: str = "terminal") -> tuple[bool, dict, str]:
    proc = subprocess.run(
        [
            str(BIN),
            "server",
            "app",
            "open",
            "--pid",
            str(pid),
            session,
            "--view",
            view,
            "--timeout-ms",
            "20000",
        ],
        cwd=ROOT,
        text=True,
        capture_output=True,
        env=ENV,
    )
    text = (proc.stdout or "").strip()
    payload = json.loads(text) if text else {}
    detail = proc.stderr.strip() or proc.stdout.strip()
    return proc.returncode == 0, payload, detail


def app_screenshot(pid: int, path: Path) -> dict:
    return run(
        "server",
        "app",
        "screenshot",
        "--pid",
        str(pid),
        str(path),
        "--timeout-ms",
        "8000",
    )


def app_theme(pid: int, theme: str) -> dict:
    return run(
        "server",
        "app",
        "theme",
        theme,
        "--pid",
        str(pid),
        "--timeout-ms",
        "30000",
    )


def app_set_search(pid: int, query: str, *, focused: bool | None = None) -> dict:
    args = [
        "server",
        "app",
        "search",
        "set",
        "--pid",
        str(pid),
        query,
        "--timeout-ms",
        "12000",
    ]
    if focused is not None:
        args.extend(["--focus", "on" if focused else "off"])
    return run(*args)


def app_set_right_panel_mode(pid: int, mode: str) -> dict:
    return run(
        "server",
        "app",
        "panel",
        mode,
        "--pid",
        str(pid),
        "--timeout-ms",
        "12000",
    )


def app_set_maximized(pid: int, enabled: bool) -> dict:
    return run(
        "server",
        "app",
        "maximize",
        "on" if enabled else "off",
        "--pid",
        str(pid),
        "--timeout-ms",
        "12000",
    )


def app_rows(pid: int) -> list[dict]:
    payload = run("server", "app", "rows", "--pid", str(pid), "--timeout-ms", "8000")
    return ((payload.get("data") or {}).get("rows") or [])


def app_clients() -> list[dict]:
    return run("server", "app", "clients", "--timeout-ms", "8000").get("clients") or []


def client_record(pid: int) -> dict:
    for client in app_clients():
        if int(client.get("pid") or 0) == int(pid):
            return client
    raise AssertionError(f"no live app client found for pid {pid}")


def client_display_metadata(pid: int) -> tuple[str, str]:
    state = app_state(pid)
    client_instance = state.get("client_instance") or {}
    window = state.get("window") or {}
    display = str(client_instance.get("display") or window.get("display") or "").strip()
    xauthority = str(client_instance.get("xauthority") or window.get("xauthority") or "").strip()
    return display, xauthority


def process_rss_kb(pid: int) -> int:
    for line in Path(f"/proc/{pid}/status").read_text(encoding="utf-8").splitlines():
        if line.startswith("VmRSS:"):
            return int(line.split()[1])
    raise AssertionError(f"VmRSS missing for pid {pid}")


def child_pids(pid: int) -> list[int]:
    children: list[int] = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            stat = (entry / "stat").read_text(encoding="utf-8")
        except OSError:
            continue
        try:
            ppid = int(stat.split(") ", 1)[1].split()[1])
        except (IndexError, ValueError):
            continue
        if ppid == pid:
            children.append(int(entry.name))
    return sorted(children)


def assert_client_memory_budget(pid: int) -> dict:
    deadline = time.time() + 15.0
    last_sample = None
    while time.time() < deadline:
        main_rss_kb = process_rss_kb(pid)
        children = child_pids(pid)
        child_rss_kb = {
            child: process_rss_kb(child)
            for child in children
            if Path(f"/proc/{child}/status").exists()
        }
        total_rss_kb = main_rss_kb + sum(child_rss_kb.values())
        last_sample = {
            "main_rss_kb": main_rss_kb,
            "child_rss_kb": child_rss_kb,
            "total_rss_kb": total_rss_kb,
        }
        if (
            main_rss_kb <= CLIENT_MAIN_RSS_MAX_KB
            and total_rss_kb <= CLIENT_TOTAL_RSS_MAX_KB
        ):
            return last_sample
        time.sleep(0.5)
    raise AssertionError(
        "yggterm client memory did not settle within budget: "
        f"main_rss_kb={last_sample['main_rss_kb']} total_rss_kb={last_sample['total_rss_kb']} "
        f"children={last_sample['child_rss_kb']} "
        f"budgets=main<={CLIENT_MAIN_RSS_MAX_KB} total<={CLIENT_TOTAL_RSS_MAX_KB}"
    )


def latest_app_root_render_count(pid: int) -> dict | None:
    state = app_state(pid)
    browser_metrics = ((state.get("browser") or {}).get("metrics") or {})
    root_render_count = browser_metrics.get("root_render_count")
    if root_render_count is not None:
        return {
            "count": int(root_render_count or 0),
            "ts_ms": int(time.time() * 1000),
            "source": "app_state",
        }
    trace_path = current_yggterm_home() / "event-trace.jsonl"
    if not trace_path.exists():
        return None
    latest: dict | None = None
    with trace_path.open("r", encoding="utf-8", errors="replace") as fh:
        for line in fh:
            line = line.strip()
            if not line:
                continue
            try:
                record = json.loads(line)
            except json.JSONDecodeError:
                continue
            if int(record.get("pid") or 0) != int(pid):
                continue
            if record.get("component") != "ui":
                continue
            if record.get("category") != "startup":
                continue
            if record.get("name") != "app_root_render_count":
                continue
            payload = record.get("payload") or {}
            latest = {
                "count": int(payload.get("count") or 0),
                "ts_ms": int(record.get("ts_ms") or 0),
                "trace_path": str(trace_path),
            }
    return latest


def assert_idle_root_render_budget(pid: int) -> dict:
    windows: list[dict] = []
    for _ in range(4):
        before = latest_app_root_render_count(pid)
        time.sleep(IDLE_ROOT_RENDER_SAMPLE_SECONDS)
        after = latest_app_root_render_count(pid)
        if before is None or after is None:
            raise AssertionError(
                f"idle root render budget could not be measured for pid {pid}: before={before!r} after={after!r}"
            )
        delta = int(after["count"]) - int(before["count"])
        sample = {
            "before": before,
            "after": after,
            "delta": delta,
            "sample_seconds": IDLE_ROOT_RENDER_SAMPLE_SECONDS,
            "max_delta": IDLE_ROOT_RENDER_MAX_DELTA,
        }
        windows.append(sample)
        if delta <= IDLE_ROOT_RENDER_MAX_DELTA:
            return {
                **sample,
                "settle_windows": len(windows),
                "windows": list(windows),
            }
    last = windows[-1]
    raise AssertionError(
        f"idle app root render budget exceeded: delta={last['delta']} "
        f"before={last['before']['count']} after={last['after']['count']} "
        f"sample_seconds={IDLE_ROOT_RENDER_SAMPLE_SECONDS} windows={windows!r}"
    )


def xdotool_env_for_pid(pid: int) -> dict:
    env = ENV.copy()
    display, xauthority = client_display_metadata(pid)
    if not display:
        raise AssertionError(f"client {pid} is missing display metadata")
    env["DISPLAY"] = display
    if xauthority:
        env["XAUTHORITY"] = xauthority
    return env


def visible_window_id_for_pid(pid: int) -> str:
    env = xdotool_env_for_pid(pid)
    proc = subprocess.run(
        ["xdotool", "search", "--onlyvisible", "--pid", str(pid), "--name", "Yggterm"],
        text=True,
        capture_output=True,
        env=env,
        timeout=XDO_TIMEOUT_SECONDS,
    )
    if proc.returncode != 0:
        raise AssertionError(proc.stderr.strip() or f"xdotool search failed for pid {pid}")
    raw_window_ids = [line.strip() for line in proc.stdout.splitlines() if line.strip()]
    window_ids: list[str] = []
    for window_id in raw_window_ids:
        owner = subprocess.run(
            ["xdotool", "getwindowpid", window_id],
            text=True,
            capture_output=True,
            env=env,
            timeout=XDO_TIMEOUT_SECONDS,
        )
        owner_pid = (owner.stdout or "").strip()
        if owner.returncode == 0 and owner_pid == str(pid):
            window_ids.append(window_id)
    if not window_ids:
        raise AssertionError(
            f"no visible Yggterm X11 window found for pid {pid}: raw={raw_window_ids!r}"
        )
    largest_window_id = window_ids[-1]
    largest_area = -1
    for window_id in window_ids:
        geometry = subprocess.run(
            ["xdotool", "getwindowgeometry", "--shell", window_id],
            text=True,
            capture_output=True,
            env=env,
            timeout=XDO_TIMEOUT_SECONDS,
        )
        if geometry.returncode != 0:
            continue
        width = 0
        height = 0
        for line in geometry.stdout.splitlines():
            if line.startswith("WIDTH="):
                width = int(line.split("=", 1)[1] or 0)
            elif line.startswith("HEIGHT="):
                height = int(line.split("=", 1)[1] or 0)
        area = width * height
        if area > largest_area:
            largest_area = area
            largest_window_id = window_id
    return largest_window_id


def xdotool_activate_window_if_supported(pid: int, window_id: str) -> None:
    env = xdotool_env_for_pid(pid)
    try:
        proc = subprocess.run(
            ["xdotool", "windowactivate", "--sync", window_id],
            text=True,
            capture_output=True,
            env=env,
            timeout=XDO_TIMEOUT_SECONDS,
        )
        if proc.returncode == 0:
            return
        stderr = proc.stderr.strip()
    except subprocess.TimeoutExpired:
        stderr = "windowactivate timeout"
    unsupported_markers = (
        "_NET_ACTIVE_WINDOW",
        "attempt to activate the window was aborted",
        "windowactivate timeout",
    )
    if any(marker in stderr for marker in unsupported_markers):
        try:
            focus = subprocess.run(
                ["xdotool", "windowfocus", window_id],
                text=True,
                capture_output=True,
                env=env,
                timeout=XDO_TIMEOUT_SECONDS,
            )
        except subprocess.TimeoutExpired:
            focus = None
        if focus is None or focus.returncode == 0:
            return
        focus_stderr = focus.stderr.strip()
        if focus_stderr:
            raise AssertionError(focus_stderr)
        return
    raise AssertionError(stderr or f"xdotool windowactivate failed for pid {pid}")


def screen_coordinates_for_window_point(pid: int, x: float, y: float) -> tuple[int, int]:
    state = app_state(pid)
    window = state.get("window") or {}
    outer_position = window.get("outer_position") or {}
    offset_x = float(outer_position.get("x") or 0.0)
    offset_y = float(outer_position.get("y") or 0.0)
    return (
        int(round(offset_x + x)),
        int(round(offset_y + y)),
    )


def xdotool_click_window(pid: int, x: float, y: float, button: int = 1) -> dict:
    env = xdotool_env_for_pid(pid)
    window_id = visible_window_id_for_pid(pid)
    xdotool_activate_window_if_supported(pid, window_id)
    screen_x, screen_y = screen_coordinates_for_window_point(pid, x, y)
    commands = [
        [
            "xdotool",
            "mousemove",
            "--sync",
            str(screen_x),
            str(screen_y),
        ],
        ["xdotool", "mousedown", str(button)],
        ["xdotool", "mouseup", str(button)],
    ]
    for command in commands:
        try:
            proc = subprocess.run(
                command,
                text=True,
                capture_output=True,
                env=env,
                timeout=XDO_TIMEOUT_SECONDS,
            )
        except subprocess.TimeoutExpired:
            if len(command) >= 2 and command[1] == "mousemove" and "--sync" in command:
                fallback_command = [part for part in command if part != "--sync"]
                proc = subprocess.run(
                    fallback_command,
                    text=True,
                    capture_output=True,
                    env=env,
                    timeout=XDO_TIMEOUT_SECONDS,
                )
            else:
                raise
        if proc.returncode != 0:
            raise AssertionError(proc.stderr.strip() or f"xdotool click failed for pid {pid}: {command!r}")
        if command[1] == "mousedown":
            time.sleep(0.06)
    time.sleep(0.18)
    return {
        "window_id": window_id,
        "x": screen_x,
        "y": screen_y,
        "button": int(button),
    }


def xdotool_right_click_window(pid: int, x: float, y: float) -> dict:
    env = xdotool_env_for_pid(pid)
    window_id = visible_window_id_for_pid(pid)
    xdotool_activate_window_if_supported(pid, window_id)
    screen_x, screen_y = screen_coordinates_for_window_point(pid, x, y)
    commands = [
        [
            "xdotool",
            "mousemove",
            "--sync",
            str(screen_x),
            str(screen_y),
        ],
        ["xdotool", "click", "3"],
    ]
    for command in commands:
        try:
            proc = subprocess.run(
                command,
                text=True,
                capture_output=True,
                env=env,
                timeout=XDO_TIMEOUT_SECONDS,
            )
        except subprocess.TimeoutExpired:
            if len(command) >= 2 and command[1] == "mousemove" and "--sync" in command:
                fallback_command = [part for part in command if part != "--sync"]
                proc = subprocess.run(
                    fallback_command,
                    text=True,
                    capture_output=True,
                    env=env,
                    timeout=XDO_TIMEOUT_SECONDS,
                )
            else:
                raise
        if proc.returncode != 0:
            raise AssertionError(
                proc.stderr.strip()
                or f"xdotool right click failed for pid {pid}: {command!r}"
            )
    time.sleep(0.18)
    return {
        "window_id": window_id,
        "x": screen_x,
        "y": screen_y,
        "button": 3,
    }


def xdotool_drag_window(pid: int, start_x: float, start_y: float, end_x: float, end_y: float) -> dict:
    env = xdotool_env_for_pid(pid)
    window_id = visible_window_id_for_pid(pid)
    xdotool_activate_window_if_supported(pid, window_id)
    start_screen_x, start_screen_y = screen_coordinates_for_window_point(pid, start_x, start_y)
    end_screen_x, end_screen_y = screen_coordinates_for_window_point(pid, end_x, end_y)
    commands = [
        [
            "xdotool",
            "mousemove",
            "--sync",
            str(start_screen_x),
            str(start_screen_y),
        ],
        ["xdotool", "mousedown", "1"],
        [
            "xdotool",
            "mousemove",
            "--sync",
            str(end_screen_x),
            str(end_screen_y),
        ],
        ["xdotool", "mouseup", "1"],
    ]
    for index, command in enumerate(commands):
        try:
            proc = subprocess.run(
                command,
                text=True,
                capture_output=True,
                env=env,
                timeout=XDO_TIMEOUT_SECONDS,
            )
        except subprocess.TimeoutExpired:
            if len(command) >= 2 and command[1] == "mousemove" and "--sync" in command:
                fallback_command = [part for part in command if part != "--sync"]
                proc = subprocess.run(
                    fallback_command,
                    text=True,
                    capture_output=True,
                    env=env,
                    timeout=XDO_TIMEOUT_SECONDS,
                )
            else:
                raise
        if proc.returncode != 0:
            raise AssertionError(proc.stderr.strip() or f"xdotool drag failed for pid {pid}: {command!r}")
        if index == 2:
            time.sleep(0.12)
        elif index == 3:
            time.sleep(0.16)
    time.sleep(0.22)
    return {
        "window_id": window_id,
        "start": {"x": start_screen_x, "y": start_screen_y},
        "end": {"x": end_screen_x, "y": end_screen_y},
    }


def xdotool_key_window(pid: int, *keys: str) -> dict:
    env = xdotool_env_for_pid(pid)
    window_id = visible_window_id_for_pid(pid)
    xdotool_activate_window_if_supported(pid, window_id)
    proc = subprocess.run(
        ["xdotool", "key", "--clearmodifiers", *keys],
        text=True,
        capture_output=True,
        env=env,
        timeout=XDO_TIMEOUT_SECONDS,
    )
    if proc.returncode != 0:
        raise AssertionError(proc.stderr.strip() or f"xdotool key failed for pid {pid}: {keys!r}")
    time.sleep(0.12)
    return {
        "window_id": window_id,
        "keys": list(keys),
    }


def xdotool_type_window(pid: int, text: str) -> dict:
    env = xdotool_env_for_pid(pid)
    window_id = visible_window_id_for_pid(pid)
    xdotool_activate_window_if_supported(pid, window_id)
    proc = subprocess.run(
        ["xdotool", "type", "--clearmodifiers", "--", text],
        text=True,
        capture_output=True,
        env=env,
        timeout=XDO_TIMEOUT_SECONDS,
    )
    if proc.returncode != 0:
        raise AssertionError(proc.stderr.strip() or f"xdotool type failed for pid {pid}: {text!r}")
    time.sleep(0.18)
    return {
        "window_id": window_id,
        "text": text,
    }


def right_panel_mode(state: dict) -> str:
    shell = state.get("shell") or {}
    value = shell.get("right_panel_mode")
    if value is None:
        value = state.get("right_panel_mode")
    return str(value or "hidden").strip().lower()


def open_settings_panel_via_command_lane(pid: int, timeout_seconds: float = 6.0) -> dict:
    app_set_right_panel_mode(pid, "settings")
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        last_state = app_state(pid)
        if right_panel_mode(last_state) == "settings" and rect_is_visible(
            dom_rect(last_state, "settings_interface_llm_input_rect")
        ):
            return last_state
        time.sleep(0.12)
    raise AssertionError(f"settings rail did not open via command lane: {last_state!r}")


def unwrap_data(payload: dict) -> dict:
    data = payload.get("data")
    if isinstance(data, dict):
        return data
    return payload


def server_snapshot() -> dict:
    return unwrap_data(run("server", "snapshot"))


def current_yggterm_home() -> Path:
    configured = ENV.get("YGGTERM_HOME")
    if configured:
        return Path(configured).expanduser()
    return Path.home() / ".yggterm"


def normalize_live_path(path: str) -> str:
    if "://" in path or "::" not in path:
        return path
    prefix, suffix = path.split("::", 1)
    return f"{prefix}://{suffix}"


def normalize_session_kind(kind: str | None) -> str:
    return str(kind or "").strip().lower()


def find_snapshot_session(snapshot: dict, session: str) -> dict | None:
    normalized_session = normalize_live_path(session)

    def matches(candidate: dict, key: str | None = None) -> bool:
        candidates = {
            normalize_live_path(str(key or "")),
            normalize_live_path(str(candidate.get("session_path") or "")),
            normalize_live_path(str(candidate.get("path") or "")),
        }
        return normalized_session in candidates

    sessions = snapshot.get("sessions") or {}
    if isinstance(sessions, dict):
        for key, value in sessions.items():
            if isinstance(value, dict) and matches(value, str(key)):
                return value

    active_session = snapshot.get("active_session") or {}
    if isinstance(active_session, dict) and matches(active_session, snapshot.get("active_session_path")):
        return active_session

    live_sessions = snapshot.get("live_sessions") or []
    if isinstance(live_sessions, list):
        for entry in live_sessions:
            if isinstance(entry, dict) and matches(entry):
                return entry

    return None


def assert_observability_budget() -> dict:
    home = current_yggterm_home()
    budgets = [
        ("perf-telemetry.jsonl", PERF_TELEMETRY_MAX_BYTES),
        ("perf-telemetry.previous.jsonl", PERF_TELEMETRY_MAX_BYTES),
        ("ui-telemetry.jsonl", UI_TELEMETRY_MAX_BYTES),
        ("ui-telemetry.previous.jsonl", UI_TELEMETRY_MAX_BYTES),
    ]
    sizes: dict[str, int] = {}
    for filename, budget in budgets:
        path = home / filename
        size = path.stat().st_size if path.exists() else 0
        sizes[filename] = size
        if size > budget + TELEMETRY_BUDGET_SLACK_BYTES:
            raise AssertionError(
                f"observability file exceeded budget: {path} size={size} budget={budget}"
            )
    return {
        "home": str(home),
        "sizes": sizes,
        "perf_budget_bytes": PERF_TELEMETRY_MAX_BYTES,
        "ui_budget_bytes": UI_TELEMETRY_MAX_BYTES,
        "slack_bytes": TELEMETRY_BUDGET_SLACK_BYTES,
    }


def probe_select(pid: int, session: str) -> dict:
    return unwrap_data(run(
        "server",
        "app",
        "terminal",
        "probe-select",
        "--pid",
        str(pid),
        session,
        "--timeout-ms",
        "8000",
    ))


def probe_scroll(pid: int, session: str, lines: int) -> dict:
    return unwrap_data(run(
        "server",
        "app",
        "terminal",
        "probe-scroll",
        "--pid",
        str(pid),
        session,
        "--lines",
        str(lines),
        "--timeout-ms",
        "8000",
    ))


def probe_type(
    pid: int,
    session: str,
    data: str,
    *,
    mode: str = "keyboard",
    press_enter: bool = False,
    press_tab: bool = False,
    press_ctrl_c: bool = False,
    press_ctrl_e: bool = False,
    press_ctrl_u: bool = False,
) -> dict:
    args = [
        "server",
        "app",
        "terminal",
        "probe-type",
        "--pid",
        str(pid),
        session,
        "--mode",
        mode,
        "--data",
        data,
        "--timeout-ms",
        "15000",
    ]
    if press_enter:
        args.append("--enter")
    if press_tab:
        args.append("--tab")
    if press_ctrl_c:
        args.append("--ctrl-c")
    if press_ctrl_e:
        args.append("--ctrl-e")
    if press_ctrl_u:
        args.append("--ctrl-u")
    return unwrap_data(run(*args))


def parse_css_rgb(value: str) -> tuple[float, float, float] | None:
    value = value.strip()
    if value.startswith("#") and len(value) == 7:
        return (
            int(value[1:3], 16) / 255.0,
            int(value[3:5], 16) / 255.0,
            int(value[5:7], 16) / 255.0,
        )
    if value.startswith("rgb(") and value.endswith(")"):
        parts = [part.strip() for part in value[4:-1].split(",")]
        if len(parts) == 3:
            return tuple(int(part) / 255.0 for part in parts)  # type: ignore[return-value]
    if value.startswith("rgba(") and value.endswith(")"):
        parts = [part.strip() for part in value[5:-1].split(",")]
        if len(parts) == 4:
            try:
                alpha = float(parts[3])
            except ValueError:
                return None
            if alpha <= 0.0:
                return None
            return tuple(int(part) / 255.0 for part in parts[:3])  # type: ignore[return-value]
    return None


def relative_luminance(rgb: tuple[float, float, float]) -> float:
    def channel(value: float) -> float:
        return value / 12.92 if value <= 0.03928 else ((value + 0.055) / 1.055) ** 2.4

    red, green, blue = rgb
    return (0.2126 * channel(red)) + (0.7152 * channel(green)) + (0.0722 * channel(blue))


def contrast_ratio(foreground: str, background: str) -> float | None:
    fg = parse_css_rgb(foreground)
    bg = parse_css_rgb(background)
    if fg is None or bg is None:
        return None
    fg_l = relative_luminance(fg)
    bg_l = relative_luminance(bg)
    lighter = max(fg_l, bg_l)
    darker = min(fg_l, bg_l)
    return (lighter + 0.05) / (darker + 0.05)


def rect_is_visible(rect: dict | None) -> bool:
    if not rect:
        return False
    return float(rect.get("width") or 0) > 0 and float(rect.get("height") or 0) > 0


def rect_contains_rect(outer: dict | None, inner: dict | None, *, tolerance: float = 1.5) -> bool:
    if not rect_is_visible(outer) or not rect_is_visible(inner):
        return False
    return (
        float(inner.get("left") or 0.0) >= float(outer.get("left") or 0.0) - tolerance
        and float(inner.get("top") or 0.0) >= float(outer.get("top") or 0.0) - tolerance
        and rect_right(inner) <= rect_right(outer) + tolerance
        and rect_bottom(inner) <= rect_bottom(outer) + tolerance
    )


def rect_bounds(rect: dict, *, pad_x: int = 0, pad_y: int = 0) -> tuple[int, int, int, int]:
    left = int(round(float(rect.get("left") or 0))) - pad_x
    top = int(round(float(rect.get("top") or 0))) - pad_y
    right = int(round(float(rect.get("left") or 0) + float(rect.get("width") or 0))) + pad_x
    bottom = int(round(float(rect.get("top") or 0) + float(rect.get("height") or 0))) + pad_y
    return left, top, right, bottom


def clamp_box(
    box: tuple[int, int, int, int], image_size: tuple[int, int]
) -> tuple[int, int, int, int] | None:
    left, top, right, bottom = box
    width, height = image_size
    left = max(0, min(left, width))
    right = max(0, min(right, width))
    top = max(0, min(top, height))
    bottom = max(0, min(bottom, height))
    if right <= left or bottom <= top:
        return None
    return left, top, right, bottom


def pixel_delta(a: tuple[int, int, int], b: tuple[int, int, int]) -> int:
    return max(abs(a[0] - b[0]), abs(a[1] - b[1]), abs(a[2] - b[2]))


def dominant_border_color(image: Image.Image) -> tuple[int, int, int]:
    width, height = image.size
    samples: list[tuple[int, int, int]] = []
    for x in range(width):
        samples.append(image.getpixel((x, 0))[:3])
        samples.append(image.getpixel((x, height - 1))[:3])
    for y in range(height):
        samples.append(image.getpixel((0, y))[:3])
        samples.append(image.getpixel((width - 1, y))[:3])
    samples.sort()
    return samples[len(samples) // 2]


def count_non_background_pixels(
    image: Image.Image,
    *,
    tolerance: int = 18,
) -> tuple[int, tuple[int, int, int]]:
    rgb = image.convert("RGBA")
    background = dominant_border_color(rgb)
    count = 0
    pixels = rgb.load()
    for y in range(rgb.height):
        for x in range(rgb.width):
            pixel = pixels[x, y]
            alpha = pixel[3]
            if alpha <= 16:
                continue
            if pixel_delta(pixel[:3], background) > tolerance:
                count += 1
    return count, background


def count_background_mismatch_pixels(
    image: Image.Image,
    *,
    background: tuple[int, int, int],
    tolerance: int = 12,
) -> int:
    rgba = image.convert("RGBA")
    pixels = rgba.load()
    count = 0
    for y in range(rgba.height):
        for x in range(rgba.width):
            pixel = pixels[x, y]
            if pixel[3] <= 16:
                continue
            if pixel_delta(pixel[:3], background) > tolerance:
                count += 1
    return count


def sampled_fill_color(
    image: Image.Image,
    box: tuple[int, int, int, int],
) -> tuple[int, int, int] | None:
    clamped = clamp_box(box, image.size)
    if clamped is None:
        return None
    crop = image.crop(clamped).convert("RGBA")
    samples: list[tuple[int, int, int]] = []
    pixels = crop.load()
    for y in range(crop.height):
        for x in range(crop.width):
            pixel = pixels[x, y]
            if pixel[3] <= 16:
                continue
            samples.append(pixel[:3])
    if not samples:
        return None
    samples.sort()
    return samples[len(samples) // 2]


def css_rgb_tuple(value: str) -> tuple[int, int, int] | None:
    rgb = parse_css_rgb(value)
    if rgb is None:
        return None
    return tuple(int(round(channel * 255.0)) for channel in rgb)


def normalize_css_value(value: str) -> str:
    return " ".join(str(value or "").strip().lower().split())


def assert_cursor_neighbor_pixels_clean(
    screenshot_path: Path,
    state: dict,
    *,
    context: str,
) -> dict:
    host = active_host(state)
    host_rect = host.get("host_rect") or {}
    cursor_rect = host.get("cursor_sample_rect") or host.get("cursor_expected_rect") or {}
    if not rect_is_visible(host_rect) or not rect_is_visible(cursor_rect):
        raise AssertionError(f"{context}: missing host/cursor geometry for pixel probe")
    cursor_box = rect_bounds(cursor_rect)
    host_box = rect_bounds(host_rect)
    image = Image.open(screenshot_path)
    probe_specs = {
        "above": (
            cursor_box[0] - 3,
            cursor_box[1] - 4,
            cursor_box[2] + 3,
            cursor_box[1] - 1,
        ),
        "below": (
            cursor_box[0] - 3,
            cursor_box[3] + 1,
            cursor_box[2] + 3,
            cursor_box[3] + 4,
        ),
        "right": (
            cursor_box[2] + 1,
            cursor_box[1],
            cursor_box[2] + 5,
            cursor_box[3],
        ),
    }
    probes: dict[str, dict] = {}
    for name, probe_box in probe_specs.items():
        if probe_box[3] <= probe_box[1]:
            continue
        if probe_box[1] < host_box[1]:
            probe_box = (probe_box[0], host_box[1], probe_box[2], probe_box[3])
        clamped = clamp_box(probe_box, image.size)
        if clamped is None:
            continue
        crop = image.crop(clamped)
        non_background_pixels, background = count_non_background_pixels(crop)
        if non_background_pixels > 4:
            raise AssertionError(
                f"{context}: detected {non_background_pixels} unexpected non-background pixels in the {name} cursor-adjacent probe"
            )
        probes[name] = {
            "probe_box": {
                "left": clamped[0],
                "top": clamped[1],
                "width": clamped[2] - clamped[0],
                "height": clamped[3] - clamped[1],
            },
            "background_rgb": background,
            "non_background_pixels": non_background_pixels,
        }
    if not probes:
        return {"skipped": "probe_boxes_outside_image"}
    return probes


def assert_prompt_prefix_pixels_visible(
    screenshot_path: Path,
    state: dict,
    *,
    context: str,
) -> dict:
    host = active_host(state)
    row_rect = host.get("cursor_row_rect") or {}
    cursor_line_text = str(host.get("cursor_line_text") or host.get("cursor_row_text") or "").strip()
    if not rect_is_visible(row_rect) or not cursor_line_text:
        return {"skipped": "missing_cursor_row"}
    row_box = rect_bounds(row_rect)
    prefix_width = min(220, row_box[2] - row_box[0])
    if prefix_width <= 0:
        return {"skipped": "empty_prefix_width"}
    image = Image.open(screenshot_path)
    clamped = clamp_box((row_box[0], row_box[1], row_box[0] + prefix_width, row_box[3]), image.size)
    if clamped is None:
        return {"skipped": "prefix_box_outside_image"}
    crop = image.crop(clamped)
    non_background_pixels, background = count_non_background_pixels(crop)
    if non_background_pixels <= 12:
        raise AssertionError(
            f"{context}: prompt-prefix pixels are missing from the screenshot despite visible cursor row text: "
            f"cursor_line={cursor_line_text!r} non_background_pixels={non_background_pixels}"
        )
    return {
        "probe_box": {
            "left": clamped[0],
            "top": clamped[1],
            "width": clamped[2] - clamped[0],
            "height": clamped[3] - clamped[1],
        },
        "background_rgb": background,
        "non_background_pixels": non_background_pixels,
        "cursor_line_text": cursor_line_text,
    }


def strip_terminal_border(line: str) -> str:
    return line.strip().strip("╭╮╰╯─│ ").strip()


def terminal_chunk_has_codex_prompt_output(data: str) -> bool:
    normalized_lines = [line.strip() for line in str(data or "").splitlines() if line.strip()]
    if not normalized_lines:
        return False
    if len(normalized_lines) > 2 or any(len(line) > 96 for line in normalized_lines):
        return False
    return any(strip_terminal_border(line).startswith("›") for line in normalized_lines)


def host_has_live_codex_prompt(host: dict) -> bool:
    input_ready = host.get("input_enabled") is True or host.get("helper_textarea_focused") is True
    if not input_ready:
        return False
    text_sample = str(host.get("text_sample") or "")
    cursor_line_text = str(host.get("cursor_line_text") or host.get("cursor_row_text") or "")
    return terminal_chunk_has_codex_prompt_output(text_sample) or terminal_chunk_has_codex_prompt_output(
        cursor_line_text
    )


def host_has_shell_status_failure(host: dict) -> bool:
    haystack = "\n".join(
        [
            str(host.get("text_sample") or ""),
            str(host.get("cursor_line_text") or ""),
            str(host.get("cursor_row_text") or ""),
        ]
    )
    recent = haystack[-400:].lower()
    return "bash: /status" in recent or (
        "/status" in recent and "no such file or directory" in recent
    )


def max_blank_rows_below_live_cursor(rows: int | float | None) -> int:
    rows = int(rows or 0)
    if rows >= 36:
        return 3
    if rows >= 20:
        return 2
    return 1


def is_transparent_css_color(value: str | None) -> bool:
    text = str(value or "").strip().lower()
    return text in ("", "transparent", "rgba(0, 0, 0, 0)", "rgba(0,0,0,0)")


def is_effectively_hidden_css_opacity(value: str | None) -> bool:
    text = str(value or "").strip().lower()
    if text in ("", "none", "normal"):
        return False
    try:
        return float(text) < 0.05
    except ValueError:
        return False


def terminal_hosts(state: dict) -> list[dict]:
    viewport = state.get("viewport") or {}
    hosts = viewport.get("terminal_hosts")
    if isinstance(hosts, list):
        return hosts
    dom_hosts = (state.get("dom") or {}).get("terminal_hosts")
    if isinstance(dom_hosts, list):
        return dom_hosts
    return []


def active_host_or_none(state: dict) -> dict | None:
    hosts = terminal_hosts(state)
    if not hosts:
        return None
    active_session_path = state.get("active_session_path")
    explicit_matches = [host for host in hosts if host.get("is_active_session_host") is True]
    if explicit_matches:
        return explicit_matches[-1]
    if active_session_path:
        session_matches = [
            host for host in hosts if str(host.get("session_path") or "") == str(active_session_path)
        ]
        if session_matches:
            focused_matches = [
                host for host in session_matches
                if host.get("helper_textarea_focused") is True or host.get("host_has_active_element") is True
            ]
            if focused_matches:
                return focused_matches[-1]
            return session_matches[-1]
    focused_hosts = [
        host for host in hosts
        if host.get("helper_textarea_focused") is True or host.get("host_has_active_element") is True
    ]
    if focused_hosts:
        return focused_hosts[-1]
    return hosts[-1]


def active_host(state: dict) -> dict:
    host = active_host_or_none(state)
    if host is None:
        raise AssertionError("no terminal host found in app state")
    return host


def dom_rect(state: dict, key: str) -> dict:
    rect = ((state.get("dom") or {}).get(key) or {})
    if not isinstance(rect, dict):
        return {}
    return rect


def shell_context_menu_row_path(state: dict) -> str:
    shell = state.get("shell") or {}
    row = shell.get("context_menu_row") or {}
    return str(row.get("full_path") or "").strip()


def selected_visible_session_row(state: dict) -> dict:
    browser = state.get("browser") or {}
    selected = browser.get("selected_row") or {}
    target_path = str(selected.get("full_path") or "").strip()
    target_kind = str(selected.get("kind") or "").strip()
    rows = ((state.get("dom") or {}).get("sidebar_visible_rows") or [])
    if target_path and target_kind == "Session":
        matches = []
        for row in rows:
            if str(row.get("path") or "").strip() == target_path:
                matches.append(row)
        if matches:
            return matches[-1]
    selected_matches = []
    for row in rows:
        if bool(row.get("selected")) and str(row.get("kind") or "").strip() == "Session":
            selected_matches.append(row)
    if selected_matches:
        return selected_matches[-1]
    raise AssertionError(f"no selected visible session row: {state!r}")


def sidebar_row_click_x(state: dict) -> float:
    sidebar_rect = dom_rect(state, "sidebar_rect")
    if not rect_is_visible(sidebar_rect):
        raise AssertionError(f"sidebar rect missing for row click: {state!r}")
    left = float(sidebar_rect.get("left") or 0.0)
    width = float(sidebar_rect.get("width") or 0.0)
    return min(left + max(56.0, width * 0.42), rect_right(sidebar_rect) - 24.0)


def rect_center_y(rect: dict) -> float:
    return float(rect.get("top") or 0.0) + (float(rect.get("height") or 0.0) / 2.0)


def rect_center_x(rect: dict) -> float:
    return float(rect.get("left") or 0.0) + (float(rect.get("width") or 0.0) / 2.0)


def rect_bottom(rect: dict) -> float:
    if "bottom" in rect:
        return float(rect["bottom"])
    return float(rect.get("top") or 0.0) + float(rect.get("height") or 0.0)


def rect_right(rect: dict) -> float:
    if "right" in rect:
        return float(rect["right"])
    return float(rect.get("left") or 0.0) + float(rect.get("width") or 0.0)


def titlebar_transient_open(state: dict) -> bool:
    shell = state.get("shell") or {}
    return bool(
        shell.get("search_focused")
        or shell.get("command_mode_active")
        or rect_is_visible(dom_rect(state, "titlebar_search_dropdown_rect"))
        or rect_is_visible(dom_rect(state, "titlebar_new_menu_rect"))
        or rect_is_visible(dom_rect(state, "titlebar_overflow_menu_rect"))
        or rect_is_visible(dom_rect(state, "titlebar_summary_menu_rect"))
    )


def close_right_panel(pid: int, state: dict | None = None, timeout_seconds: float = 4.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = state or {}
    button_key_for_mode = {
        "connect": "titlebar_connect_button_rect",
        "notifications": "titlebar_notifications_button_rect",
        "settings": "titlebar_settings_button_rect",
        "metadata": "titlebar_metadata_button_rect",
    }
    while time.time() < deadline:
        if not last_state:
            last_state = app_state(pid)
        mode = right_panel_mode(last_state)
        if mode not in ("", "hidden", "none", "null"):
            try:
                app_set_right_panel_mode(pid, "hidden")
                time.sleep(0.18)
                last_state = app_state(pid)
                if right_panel_mode(last_state) in ("", "hidden", "none", "null"):
                    return last_state
            except Exception:
                pass
        settings_rect = dom_rect(last_state, "settings_interface_llm_input_rect")
        endpoint_rect = dom_rect(last_state, "settings_endpoint_input_rect")
        active_element = (last_state.get("dom") or {}).get("active_element") or {}
        helper_active = active_element.get("class_name") == "xterm-helper-textarea"
        settings_dom_active = active_element.get("data_settings_field_key") in {
            "interface-llm",
            "litellm-endpoint",
            "litellm-api-key",
        }
        if mode in ("", "hidden", "none", "null") and helper_active and not settings_dom_active:
            return last_state
        if (
            not rect_is_visible(settings_rect)
            and not rect_is_visible(endpoint_rect)
            and not settings_dom_active
            and helper_active
        ):
            return last_state
        if mode in ("", "hidden", "none", "null") and not rect_is_visible(settings_rect) and not rect_is_visible(endpoint_rect) and not settings_dom_active:
            return last_state
        button_key = button_key_for_mode.get(mode)
        if not button_key and settings_dom_active:
            button_key = "titlebar_settings_button_rect"
        if not button_key:
            raise AssertionError(f"unknown right panel mode while normalizing: mode={mode!r} state={last_state!r}")
        button_rect = dom_rect(last_state, button_key)
        if not rect_is_visible(button_rect):
            raise AssertionError(
                f"right panel toggle button missing while normalizing: mode={mode!r} button_key={button_key!r} state={last_state!r}"
            )
        try:
            app_focus(pid)
            time.sleep(0.08)
        except Exception:
            pass
        xdotool_click_window(pid, rect_center_x(button_rect), rect_center_y(button_rect))
        time.sleep(0.22)
        last_state = app_state(pid)
        mode = right_panel_mode(last_state)
        settings_rect = dom_rect(last_state, "settings_interface_llm_input_rect")
        endpoint_rect = dom_rect(last_state, "settings_endpoint_input_rect")
        active_element = (last_state.get("dom") or {}).get("active_element") or {}
        if mode in ("", "hidden", "none", "null"):
            return last_state
        if (
            not rect_is_visible(settings_rect)
            and not rect_is_visible(endpoint_rect)
            and active_element.get("class_name") == "xterm-helper-textarea"
        ):
            return last_state
        if mode not in ("", "hidden", "none", "null"):
            try:
                if not ((last_state.get("window") or {}).get("focused")):
                    last_state = wait_for_window_focus(pid, timeout_seconds=2.0)
                button_rect = dom_rect(last_state, button_key)
                if rect_is_visible(button_rect):
                    xdotool_click_window(pid, rect_center_x(button_rect), rect_center_y(button_rect))
                    time.sleep(0.2)
                    last_state = app_state(pid)
                    mode = right_panel_mode(last_state)
                    settings_rect = dom_rect(last_state, "settings_interface_llm_input_rect")
                    endpoint_rect = dom_rect(last_state, "settings_endpoint_input_rect")
                    active_element = (last_state.get("dom") or {}).get("active_element") or {}
                    if mode in ("", "hidden", "none", "null"):
                        return last_state
                    if (
                        not rect_is_visible(settings_rect)
                        and not rect_is_visible(endpoint_rect)
                        and active_element.get("class_name") == "xterm-helper-textarea"
                    ):
                        return last_state
                    if (
                        not rect_is_visible(settings_rect)
                        and not rect_is_visible(endpoint_rect)
                        and active_element.get("tag") == "body"
                    ):
                        return last_state
            except Exception:
                pass
        try:
            xdotool_key_window(pid, "Escape")
            time.sleep(0.14)
            last_state = app_state(pid)
            mode = right_panel_mode(last_state)
            settings_rect = dom_rect(last_state, "settings_interface_llm_input_rect")
            endpoint_rect = dom_rect(last_state, "settings_endpoint_input_rect")
            active_element = (last_state.get("dom") or {}).get("active_element") or {}
            if mode in ("", "hidden", "none", "null"):
                return last_state
            if (
                not rect_is_visible(settings_rect)
                and not rect_is_visible(endpoint_rect)
                and active_element.get("class_name") == "xterm-helper-textarea"
            ):
                return last_state
            if (
                not rect_is_visible(settings_rect)
                and not rect_is_visible(endpoint_rect)
                and active_element.get("tag") == "body"
            ):
                return last_state
        except Exception:
            pass
    raise AssertionError(f"right panel did not close during normalization: {last_state!r}")


def dismiss_titlebar_transients(pid: int, state: dict | None = None, timeout_seconds: float = 3.5) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = state or {}
    while time.time() < deadline:
        if not last_state:
            last_state = app_state(pid)
        if not titlebar_transient_open(last_state):
            return last_state
        shell = last_state.get("shell") or {}
        if shell.get("search_focused") or shell.get("command_mode_active"):
            try:
                app_set_search(pid, "", focused=False)
            except Exception:
                pass
        summary_rect = dom_rect(last_state, "titlebar_summary_menu_rect")
        summary_button_rect = dom_rect(last_state, "titlebar_session_button_rect")
        if rect_is_visible(summary_rect) and rect_is_visible(summary_button_rect):
            try:
                xdotool_click_window(
                    pid,
                    rect_center_x(summary_button_rect),
                    rect_center_y(summary_button_rect),
                )
                time.sleep(0.12)
                last_state = app_state(pid)
                continue
            except Exception:
                pass
        new_menu_rect = dom_rect(last_state, "titlebar_new_menu_rect")
        new_button_rect = dom_rect(last_state, "titlebar_new_button_rect")
        if rect_is_visible(new_menu_rect) and rect_is_visible(new_button_rect):
            try:
                xdotool_click_window(
                    pid,
                    rect_center_x(new_button_rect),
                    rect_center_y(new_button_rect),
                )
                time.sleep(0.12)
                last_state = app_state(pid)
                continue
            except Exception:
                pass
        click_rect = dom_rect(last_state, "main_surface_body_rect")
        if not rect_is_visible(click_rect):
            click_rect = dom_rect(last_state, "main_surface_rect")
        if rect_is_visible(click_rect):
            try:
                xdotool_click_window(
                    pid,
                    float(click_rect["left"]) + max(18.0, float(click_rect["width"]) * 0.84),
                    float(click_rect["top"]) + max(18.0, float(click_rect["height"]) * 0.82),
                )
            except Exception:
                pass
        time.sleep(0.12)
        last_state = app_state(pid)
    raise AssertionError(f"titlebar/search transient did not dismiss: {last_state!r}")


def host_for_session_or_none(state: dict, session: str) -> dict | None:
    normalized_session = normalize_live_path(session)
    matches = [
        host
        for host in terminal_hosts(state)
        if normalize_live_path(str(host.get("session_path") or "")) == normalized_session
    ]
    if not matches:
        return None
    focused = [
        host
        for host in matches
        if host.get("helper_textarea_focused") is True or host.get("host_has_active_element") is True
    ]
    return (focused or matches)[-1]


def host_for_session(state: dict, session: str) -> dict:
    host = host_for_session_or_none(state, session)
    if host is None:
        raise AssertionError(f"no terminal host found for session {session!r}")
    return host


def visible_notifications(state: dict) -> list[dict]:
    shell = state.get("shell") or {}
    visible = shell.get("visible_notifications")
    if isinstance(visible, list):
        return visible
    notifications = shell.get("notifications")
    if isinstance(notifications, list):
        return notifications
    return []


def wait_for_interactive(pid: int, timeout_seconds: float = 20.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    last_focus_attempt = 0.0
    last_dismiss_attempt = 0.0
    last_panel_close_attempt = 0.0
    while time.time() < deadline:
        last_state = app_state(pid)
        if (
            right_panel_mode(last_state)
            not in ("", "hidden", "none", "null")
            and time.time() - last_panel_close_attempt >= 0.5
        ):
            last_state = close_right_panel(pid, last_state, timeout_seconds=1.5)
            last_panel_close_attempt = time.time()
        if titlebar_transient_open(last_state) and time.time() - last_dismiss_attempt >= 0.5:
            last_state = dismiss_titlebar_transients(pid, last_state, timeout_seconds=1.5)
            last_dismiss_attempt = time.time()
        if shell_context_menu_row_path(last_state) and time.time() - last_dismiss_attempt >= 0.5:
            try:
                xdotool_key_window(pid, "Escape")
            except Exception:
                pass
            last_dismiss_attempt = time.time()
            time.sleep(0.1)
            last_state = app_state(pid)
        viewport = last_state.get("viewport") or {}
        host = active_host_or_none(last_state)
        if host is None:
            time.sleep(0.25)
            continue
        if (
            viewport.get("ready") is True
            and viewport.get("interactive") is True
            and viewport.get("terminal_settled_kind") == "interactive"
            and host.get("input_enabled") is True
            and not visible_notifications(last_state)
            and not ((viewport.get("active_terminal_surface") or {}).get("problem"))
        ):
            return last_state
        if (
            viewport.get("ready") is True
            and viewport.get("active_view_mode") == "Terminal"
            and host.get("input_enabled") is not True
            and not visible_notifications(last_state)
            and not ((viewport.get("active_terminal_surface") or {}).get("problem"))
            and time.time() - last_focus_attempt >= 0.75
        ):
            session = str(viewport.get("active_session_path") or "")
            if session:
                try:
                    probe_type(pid, session, "", mode="keyboard")
                except Exception:
                    pass
                last_focus_attempt = time.time()
        time.sleep(0.25)
    raise AssertionError(f"terminal did not settle interactive: {last_state!r}")


def wait_for_notifications_clear(pid: int, timeout_seconds: float = 6.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        last_state = app_state(pid)
        if not visible_notifications(last_state):
            return last_state
        time.sleep(0.2)
    raise AssertionError(f"notifications did not clear before next smoke phase: {last_state!r}")


def wait_for_window_focus(pid: int, timeout_seconds: float = 3.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    last_focus_attempt = 0.0
    while time.time() < deadline:
        if time.time() - last_focus_attempt >= 0.35:
            try:
                app_focus(pid)
            except Exception:
                pass
            last_focus_attempt = time.time()
        last_state = app_state(pid)
        if (last_state.get("window") or {}).get("focused") is True:
            return last_state
        time.sleep(0.12)
    raise AssertionError(f"window did not focus for pid {pid}: {last_state!r}")


def wait_for_session_focus(pid: int, session: str, timeout_seconds: float = 12.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    normalized_session = normalize_live_path(session)
    last_panel_close_attempt = 0.0
    last_dismiss_attempt = 0.0
    while time.time() < deadline:
        last_state = app_state(pid)
        active_element = (last_state.get("dom") or {}).get("active_element") or {}
        helper_active = active_element.get("class_name") == "xterm-helper-textarea"
        settings_dom_active = active_element.get("data_settings_field_key") in {
            "interface-llm",
            "litellm-endpoint",
            "litellm-api-key",
        }
        settings_rect = dom_rect(last_state, "settings_interface_llm_input_rect")
        endpoint_rect = dom_rect(last_state, "settings_endpoint_input_rect")
        if (
            (
                right_panel_mode(last_state) not in ("", "hidden", "none", "null")
                or (rect_is_visible(settings_rect) and not helper_active)
                or rect_is_visible(endpoint_rect)
                or settings_dom_active
            )
            and time.time() - last_panel_close_attempt >= 0.5
        ):
            last_state = close_right_panel(pid, last_state, timeout_seconds=1.5)
            last_panel_close_attempt = time.time()
        if titlebar_transient_open(last_state) and time.time() - last_dismiss_attempt >= 0.5:
            last_state = dismiss_titlebar_transients(pid, last_state, timeout_seconds=1.5)
            last_dismiss_attempt = time.time()
        viewport = last_state.get("viewport") or {}
        host = host_for_session_or_none(last_state, session)
        active = active_host_or_none(last_state)
        if host is None or active is None:
            time.sleep(0.25)
            continue
        active_session = normalize_live_path(str(active.get("session_path") or ""))
        if (
            viewport.get("ready") is True
            and viewport.get("interactive") is True
            and viewport.get("terminal_settled_kind") == "interactive"
            and host.get("input_enabled") is True
            and host.get("helper_textarea_focused") is True
            and host.get("host_has_active_element") is True
            and active_session == normalized_session
            and not visible_notifications(last_state)
            and not ((viewport.get("active_terminal_surface") or {}).get("problem"))
        ):
            return last_state
        time.sleep(0.25)
    raise AssertionError(f"terminal did not focus requested session: {last_state!r}")


def terminal_activity_signature(state: dict) -> tuple:
    host = active_host(state)
    text_sample = str(host.get("text_sample") or "")
    return (
        int(host.get("data_event_count") or 0),
        int(host.get("viewport_y") or 0),
        int(host.get("base_y") or 0),
        str(host.get("cursor_row_text") or host.get("cursor_line_text") or ""),
        text_sample[-320:],
    )


def wait_for_terminal_quiescent(pid: int, timeout_seconds: float = 12.0, stable_polls: int = 3) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = wait_for_interactive(pid, timeout_seconds=min(4.0, timeout_seconds))
    last_sig = terminal_activity_signature(last_state)
    stable = 0
    while time.time() < deadline:
        time.sleep(0.25)
        state = app_state(pid)
        viewport = state.get("viewport") or {}
        host = active_host(state)
        if not (
            viewport.get("ready") is True
            and viewport.get("interactive") is True
            and viewport.get("terminal_settled_kind") == "interactive"
            and host.get("input_enabled") is True
        ):
            last_state = state
            last_sig = terminal_activity_signature(state)
            stable = 0
            continue
        sig = terminal_activity_signature(state)
        if sig == last_sig:
            stable += 1
            last_state = state
            if stable >= stable_polls:
                return state
        else:
            stable = 0
            last_state = state
            last_sig = sig
    raise AssertionError(f"terminal did not become quiescent: {last_state!r}")


def wait_for_terminal_restore(pid: int, timeout_seconds: float = 12.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        time.sleep(0.25)
        state = app_state(pid)
        viewport = state.get("viewport") or {}
        host = active_host(state)
        if (
            viewport.get("ready") is True
            and viewport.get("interactive") is True
            and viewport.get("terminal_settled_kind") == "interactive"
            and host.get("input_enabled") is True
            and host.get("xterm_buffer_kind") == "normal"
            and host.get("xterm_cursor_hidden") is False
        ):
            return state
        last_state = state
    raise AssertionError(f"terminal did not restore to the normal buffer: {last_state!r}")


def restore_prompt_after_codex_session_tui(
    pid: int,
    session: str,
    *,
    timeout_seconds: float = 12.0,
) -> dict:
    recovery_attempts = (
        {"data": "q", "mode": "keyboard"},
        {"data": "", "mode": "keyboard", "press_ctrl_c": True},
        {"data": "q", "mode": "xterm"},
        {"data": "", "mode": "xterm", "press_ctrl_c": True},
    )
    last_error: AssertionError | None = None
    for attempt in recovery_attempts:
        terminal_probe_type(
            pid,
            session,
            attempt.get("data", ""),
            mode=str(attempt.get("mode") or "xterm"),
            press_ctrl_c=bool(attempt.get("press_ctrl_c")),
        )
        try:
            return wait_for_terminal_restore(pid, timeout_seconds=timeout_seconds)
        except AssertionError as error:
            last_error = error
    if last_error is not None:
        raise last_error
    raise AssertionError("codex-session-tui restore attempts did not run")


def resolve_codex_session_tui_command() -> str | None:
    repo_binary = Path("/home/pi/gh/codex-session-tui/target/debug/codex-session-tui")
    if repo_binary.exists():
        return str(repo_binary)
    path_binary = shutil.which("codex-session-tui")
    if path_binary:
        return path_binary
    return None


def count_dark_foreground_pixels(
    image: Image.Image,
    *,
    background_tolerance: int = 18,
    dark_channel_threshold: int = 180,
) -> int:
    rgb = image.convert("RGBA")
    background = dominant_border_color(rgb)
    count = 0
    pixels = rgb.load()
    for y in range(rgb.height):
        for x in range(rgb.width):
            pixel = pixels[x, y]
            if pixel[3] <= 16:
                continue
            if pixel_delta(pixel[:3], background) <= background_tolerance:
                continue
            if (pixel[0] + pixel[1] + pixel[2]) / 3.0 < dark_channel_threshold:
                count += 1
    return count


def colorful_foreground_stats(
    image: Image.Image,
    *,
    background_tolerance: int = 18,
    min_channel_spread: int = 28,
    min_saturation: float = 0.24,
) -> tuple[int, int]:
    rgb = image.convert("RGBA")
    background = dominant_border_color(rgb)
    colorful_pixels = 0
    hue_buckets: set[int] = set()
    pixels = rgb.load()
    for y in range(rgb.height):
        for x in range(rgb.width):
            pixel = pixels[x, y]
            if pixel[3] <= 16:
                continue
            rgb_pixel = pixel[:3]
            if pixel_delta(rgb_pixel, background) <= background_tolerance:
                continue
            channel_spread = max(rgb_pixel) - min(rgb_pixel)
            if channel_spread < min_channel_spread:
                continue
            hue, saturation, _value = colorsys.rgb_to_hsv(
                rgb_pixel[0] / 255.0,
                rgb_pixel[1] / 255.0,
                rgb_pixel[2] / 255.0,
            )
            if saturation < min_saturation:
                continue
            colorful_pixels += 1
            hue_buckets.add(int((hue * 360.0) // 24.0))
    return colorful_pixels, len(hue_buckets)


def assert_geometry(state: dict) -> dict:
    host = active_host(state)
    surface = ((state.get("viewport") or {}).get("active_terminal_surface") or {})
    if surface.get("geometry_problem"):
        raise AssertionError(f"geometry problem reported: {surface.get('geometry_problem')!r}")
    host_rect = host.get("host_rect") or {}
    screen_rect = host.get("screen_rect") or {}
    viewport_rect = host.get("viewport_rect") or {}
    helpers_rect = host.get("helpers_rect") or {}
    helper_textarea_rect = host.get("helper_textarea_rect") or {}
    if not rect_is_visible(host_rect) or not rect_is_visible(screen_rect) or not rect_is_visible(viewport_rect):
        raise AssertionError(
            f"xterm host/screen/viewport not visibly mounted: host={host_rect!r} screen={screen_rect!r} viewport={viewport_rect!r}"
        )
    if abs(float(host_rect["width"]) - float(viewport_rect["width"])) > 2.0:
        raise AssertionError(
            f"viewport width drifted from host width: host={host_rect['width']!r} viewport={viewport_rect['width']!r}"
        )
    if abs(float(host_rect["height"]) - float(viewport_rect["height"])) > 2.0:
        raise AssertionError(
            f"viewport height drifted from host height: host={host_rect['height']!r} viewport={viewport_rect['height']!r}"
        )
    if abs(float(screen_rect["width"]) - float(viewport_rect["width"])) > 18.0:
        raise AssertionError(
            f"screen width drifted from viewport width: screen={screen_rect['width']!r} viewport={viewport_rect['width']!r}"
        )
    if abs(float(screen_rect["height"]) - float(viewport_rect["height"])) > 2.0:
        raise AssertionError(
            f"screen height drifted from viewport height: screen={screen_rect['height']!r} viewport={viewport_rect['height']!r}"
        )
    if rect_is_visible(helpers_rect):
        if abs(float(helpers_rect["width"]) - float(screen_rect["width"])) > 18.0:
            raise AssertionError(
                f"helpers width drifted from screen width: helpers={helpers_rect['width']!r} screen={screen_rect['width']!r}"
            )
        if abs(float(helpers_rect["height"]) - float(screen_rect["height"])) > 2.0:
            raise AssertionError(
                f"helpers height drifted from screen height: helpers={helpers_rect['height']!r} screen={screen_rect['height']!r}"
            )
    if host.get("input_enabled") is True and host.get("helper_textarea_present") is True:
        helper_opacity = str(host.get("helper_textarea_opacity") or "")
        helper_background = str(host.get("helper_textarea_background") or "").lower()
        helper_outline_style = str(host.get("helper_textarea_outline_style") or "").lower()
        helper_outline_color = str(host.get("helper_textarea_outline_color") or "").lower()
        helper_box_shadow = str(host.get("helper_textarea_box_shadow") or "").lower()
        helper_clip_path = str(host.get("helper_textarea_clip_path") or "").lower()
        helper_clip = str(host.get("helper_textarea_clip") or "").lower()
        helper_pointer_events = str(host.get("helper_textarea_pointer_events") or "").lower()
        if not rect_is_visible(helper_textarea_rect):
            raise AssertionError(
                f"helper textarea has no measurable rect: helper={helper_textarea_rect!r}"
            )
        tiny_helper = (
            1.0 <= float(helper_textarea_rect["width"]) <= 2.0
            and 1.0 <= float(helper_textarea_rect["height"]) <= 2.0
        )
        hidden_offscreen = (
            float(helper_textarea_rect["left"]) <= float(host_rect["left"]) - 1000.0
            and abs(float(helper_textarea_rect["top"]) - float(host_rect["top"])) <= 32.0
        )
        if not tiny_helper or not hidden_offscreen:
            raise AssertionError(
                f"helper textarea is outside the expected hidden geometry contract: helper={helper_textarea_rect!r} host={host_rect!r}"
            )
        if helper_opacity != "0":
            raise AssertionError(f"helper textarea opacity drifted: {helper_opacity!r}")
        if helper_background not in ("transparent", "rgba(0, 0, 0, 0)"):
            raise AssertionError(f"helper textarea background is visible: {helper_background!r}")
        if helper_outline_style != "none":
            raise AssertionError(f"helper textarea outline style is visible: {helper_outline_style!r}")
        if helper_outline_color not in ("", "transparent", "rgba(0, 0, 0, 0)"):
            raise AssertionError(f"helper textarea outline color is visible: {helper_outline_color!r}")
        if helper_box_shadow != "none":
            raise AssertionError(f"helper textarea box shadow is visible: {helper_box_shadow!r}")
        if helper_pointer_events != "none":
            raise AssertionError(f"helper textarea pointer-events drifted: {helper_pointer_events!r}")
        if "inset(50%)" not in helper_clip_path and "rect(0" not in helper_clip:
            raise AssertionError(
                f"helper textarea is missing hidden clipping contract: clip_path={helper_clip_path!r} clip={helper_clip!r}"
            )
    return {
        "host_rect": host_rect,
        "screen_rect": screen_rect,
        "viewport_rect": viewport_rect,
        "helpers_rect": helpers_rect,
        "helper_textarea_rect": helper_textarea_rect,
        "helper_textarea_opacity": host.get("helper_textarea_opacity"),
        "helper_textarea_clip_path": host.get("helper_textarea_clip_path"),
    }


def assert_titlebar_centering(state: dict) -> dict:
    titlebar_rect = dom_rect(state, "titlebar_rect")
    search_input_rect = dom_rect(state, "titlebar_search_input_rect")
    search_field_rect = dom_rect(state, "titlebar_search_field_shell_rect") or search_input_rect
    left_rect = dom_rect(state, "titlebar_left_rect")
    right_rect = dom_rect(state, "titlebar_right_rect")
    connect_rect = dom_rect(state, "titlebar_connect_button_rect")
    if not rect_is_visible(titlebar_rect):
        raise AssertionError(f"titlebar rect missing from app state: {titlebar_rect!r}")
    if not rect_is_visible(search_input_rect):
        raise AssertionError(f"search input rect missing from app state: {search_input_rect!r}")
    if not rect_is_visible(search_field_rect):
        raise AssertionError(f"search field shell rect missing from app state: {search_field_rect!r}")
    top_gap = float(search_field_rect["top"]) - float(titlebar_rect["top"])
    bottom_gap = rect_bottom(titlebar_rect) - rect_bottom(search_field_rect)
    if abs(top_gap - bottom_gap) > 1.5:
        raise AssertionError(
            f"search input is not vertically centered in titlebar: top_gap={top_gap:.2f} bottom_gap={bottom_gap:.2f} "
            f"titlebar={titlebar_rect!r} search={search_field_rect!r}"
        )
    if top_gap < 2.0 or bottom_gap < 2.0:
        raise AssertionError(
            f"titlebar search field is crowding the titlebar edge: top_gap={top_gap:.2f} bottom_gap={bottom_gap:.2f} "
            f"titlebar={titlebar_rect!r} search={search_field_rect!r}"
        )
    search_center = rect_center_y(search_input_rect)
    horizontal_center_drift = abs(rect_center_x(search_field_rect) - rect_center_x(titlebar_rect))
    if horizontal_center_drift > 3.0:
        raise AssertionError(
            f"titlebar search field drifted away from the titlebar center: drift={horizontal_center_drift:.2f} "
            f"titlebar={titlebar_rect!r} search={search_field_rect!r}"
        )
    center_drift = {}
    for name, rect in (
        ("left", left_rect),
        ("right", right_rect),
        ("connect", connect_rect),
    ):
        if not rect_is_visible(rect):
            continue
        drift = abs(rect_center_y(rect) - search_center)
        center_drift[name] = round(drift, 2)
        if drift > 2.0:
            raise AssertionError(
                f"titlebar {name} group drifted off the search centerline: drift={drift:.2f} "
                f"search={search_input_rect!r} rect={rect!r}"
            )
    return {
        "titlebar_rect": titlebar_rect,
        "search_field_rect": search_field_rect,
        "search_input_rect": search_input_rect,
        "top_gap": round(top_gap, 2),
        "bottom_gap": round(bottom_gap, 2),
        "horizontal_center_drift": round(horizontal_center_drift, 2),
        "center_drift": center_drift,
    }


def assert_terminal_viewport_inset(state: dict) -> dict:
    main_surface_rect = dom_rect(state, "main_surface_rect")
    main_surface_body_rect = dom_rect(state, "main_surface_body_rect")
    host = active_host(state)
    host_rect = host.get("host_rect") or {}
    if not rect_is_visible(main_surface_rect):
        raise AssertionError(f"main surface rect missing from app state: {main_surface_rect!r}")
    if not rect_is_visible(main_surface_body_rect):
        raise AssertionError(f"main surface body rect missing from app state: {main_surface_body_rect!r}")
    if not rect_is_visible(host_rect):
        raise AssertionError(f"terminal host rect missing for viewport inset check: {host_rect!r}")
    fullscreen = bool(((state.get("shell") or {}).get("fullscreen")) or False)
    left_gap = float(host_rect["left"]) - float(main_surface_body_rect["left"])
    top_gap = float(host_rect["top"]) - float(main_surface_body_rect["top"])
    right_gap = rect_right(main_surface_body_rect) - rect_right(host_rect)
    bottom_gap = rect_bottom(main_surface_body_rect) - rect_bottom(host_rect)
    if fullscreen:
        if max(left_gap, top_gap, right_gap, bottom_gap) > 1.5:
            raise AssertionError(
                f"fullscreen terminal should be edge-aligned: gaps="
                f"{(left_gap, top_gap, right_gap, bottom_gap)!r} body={main_surface_body_rect!r} host={host_rect!r}"
            )
    else:
        for name, gap in (
            ("left", left_gap),
            ("top", top_gap),
            ("right", right_gap),
            ("bottom", bottom_gap),
        ):
            if gap < 2.0 or gap > 6.5:
                raise AssertionError(
                    f"terminal viewport inset drifted out of the accepted band on {name}: gap={gap:.2f} "
                    f"body={main_surface_body_rect!r} host={host_rect!r}"
                )
    return {
        "main_surface_rect": main_surface_rect,
        "main_surface_body_rect": main_surface_body_rect,
        "host_rect": host_rect,
        "left_gap": round(left_gap, 2),
        "top_gap": round(top_gap, 2),
        "bottom_gap": round(bottom_gap, 2),
        "right_gap": round(right_gap, 2),
        "fullscreen": fullscreen,
    }


def assert_search_focus_overlay_contract(pid: int, session: str) -> dict:
    initial = app_state(pid)
    if initial.get("active_view_mode") != "Terminal":
        active_session_path = initial.get("active_session_path")
        if active_session_path:
            ok, _payload, detail = app_open_raw(pid, active_session_path, "terminal")
            if not ok:
                raise AssertionError(
                    f"failed to switch into terminal view before titlebar search probe: detail={detail!r}"
                )
            time.sleep(0.35)
    baseline = wait_for_window_focus(pid, timeout_seconds=3.0)
    if titlebar_transient_open(baseline):
        baseline = dismiss_titlebar_transients(pid, baseline, timeout_seconds=1.5)
    if right_panel_mode(baseline) not in ("", "hidden", "none", "null", "settings"):
        baseline = close_right_panel(pid, baseline, timeout_seconds=1.5)
    existing_summary_shell = dom_rect(baseline, "titlebar_summary_shell_rect")
    existing_session_button = dom_rect(baseline, "titlebar_session_button_rect")
    if rect_is_visible(existing_summary_shell) and rect_is_visible(existing_session_button):
        xdotool_click_window(
            pid,
            rect_center_x(existing_session_button),
            rect_center_y(existing_session_button),
        )
        time.sleep(0.25)
        baseline = app_state(pid)
    existing_new_menu = dom_rect(baseline, "titlebar_new_menu_rect")
    existing_new_button = dom_rect(baseline, "titlebar_new_button_rect")
    if rect_is_visible(existing_new_menu) and rect_is_visible(existing_new_button):
        xdotool_click_window(
            pid,
            rect_center_x(existing_new_button),
            rect_center_y(existing_new_button),
        )
        time.sleep(0.25)
        baseline = app_state(pid)
    baseline_titlebar = dom_rect(baseline, "titlebar_rect")
    baseline_host = active_host(baseline).get("host_rect") or {}
    search_input_rect = dom_rect(baseline, "titlebar_search_input_rect")
    if not rect_is_visible(baseline_titlebar):
        raise AssertionError(f"baseline titlebar rect missing: {baseline_titlebar!r}")
    if not rect_is_visible(baseline_host):
        raise AssertionError(f"baseline host rect missing: {baseline_host!r}")
    if not rect_is_visible(search_input_rect):
        raise AssertionError(f"baseline titlebar search input rect missing: {search_input_rect!r}")

    app_set_search(pid, "", focused=False)
    time.sleep(0.12)
    click = xdotool_click_window(
        pid,
        rect_center_x(search_input_rect),
        rect_center_y(search_input_rect),
    )
    deadline = time.time() + 6.0
    focused = {}
    retried_focus = False
    while time.time() < deadline:
        focused = app_state(pid)
        if (focused.get("shell") or {}).get("search_focused"):
            active_element = (focused.get("dom") or {}).get("active_element") or {}
            if str(active_element.get("id") or "") == "yggterm-search-input":
                break
        if not retried_focus and not ((focused.get("window") or {}).get("focused")):
            focused = wait_for_window_focus(pid, timeout_seconds=2.0)
            click = xdotool_click_window(
                pid,
                rect_center_x(search_input_rect),
                rect_center_y(search_input_rect),
            )
            retried_focus = True
        time.sleep(0.12)
    if not ((focused.get("shell") or {}).get("search_focused")):
        raise AssertionError(
            f"clicking the titlebar search field did not open focused search: click={click!r} focused={focused!r}"
        )
    active_element = (focused.get("dom") or {}).get("active_element") or {}
    if str(active_element.get("id") or "") != "yggterm-search-input":
        raise AssertionError(
            f"clicking the titlebar search field did not leave the search input active: "
            f"click={click!r} active_element={active_element!r} focused={focused!r}"
        )

    titlebar_rect = dom_rect(focused, "titlebar_rect")
    search_modal_rect = dom_rect(focused, "titlebar_search_outer_shell_rect")
    search_shell_rect = dom_rect(focused, "titlebar_search_field_shell_rect")
    search_dropdown_rect = dom_rect(focused, "titlebar_search_dropdown_rect")
    search_dropdown_header_rect = dom_rect(focused, "titlebar_search_dropdown_header_rect")
    search_dropdown_entry_rects = list((focused.get("dom") or {}).get("titlebar_search_dropdown_entry_rects") or [])
    summary_shell_rect = dom_rect(focused, "titlebar_summary_shell_rect")
    host_rect = active_host(focused).get("host_rect") or {}
    if not rect_is_visible(titlebar_rect):
        raise AssertionError(f"focused titlebar rect missing: {titlebar_rect!r}")
    if not rect_is_visible(search_modal_rect):
        raise AssertionError(f"focused search modal rect missing: {search_modal_rect!r}")
    if not rect_is_visible(search_shell_rect):
        raise AssertionError(f"focused search shell rect missing: {search_shell_rect!r}")
    if not rect_is_visible(search_dropdown_rect):
        raise AssertionError(f"focused search dropdown rect missing: {search_dropdown_rect!r}")
    if not rect_is_visible(search_dropdown_header_rect):
        raise AssertionError(
            f"focused search dropdown header rect missing: header={search_dropdown_header_rect!r} state={focused!r}"
        )
    if not search_dropdown_entry_rects:
        raise AssertionError(f"focused search dropdown entries missing: state={focused!r}")
    if rect_is_visible(summary_shell_rect):
        raise AssertionError(
            f"titlebar search focus left the session shell open instead of owning the floating chrome: "
            f"search={search_modal_rect!r} summary={summary_shell_rect!r} state={focused!r}"
        )
    if abs(float(titlebar_rect["height"]) - float(baseline_titlebar["height"])) > 1.0:
        raise AssertionError(
            f"search focus changed the titlebar height: baseline={baseline_titlebar!r} focused={titlebar_rect!r}"
        )
    if abs(float(host_rect.get("top") or 0.0) - float(baseline_host.get("top") or 0.0)) > 1.0:
        raise AssertionError(
            f"search focus shifted the main terminal viewport vertically: baseline={baseline_host!r} focused={host_rect!r}"
        )
    if float(search_modal_rect["top"]) < float(titlebar_rect["top"]) + 2.0:
        raise AssertionError(
            f"focused search modal is crowding the titlebar top edge: "
            f"titlebar={titlebar_rect!r} modal={search_modal_rect!r}"
        )
    if float(search_shell_rect["top"]) < float(search_modal_rect["top"]) + 4.0:
        raise AssertionError(
            f"focused search field is still glued to the floating shell top edge: "
            f"modal={search_modal_rect!r} field={search_shell_rect!r}"
        )
    if float(search_shell_rect["height"]) > float(titlebar_rect["height"]) + 1.0:
        raise AssertionError(
            f"search shell grew taller than the titlebar instead of remaining an anchored field: "
            f"titlebar={titlebar_rect!r} search_shell={search_shell_rect!r}"
        )
    if float(search_dropdown_rect["top"]) < rect_bottom(search_shell_rect) + 2.0:
        raise AssertionError(
            f"search dropdown overlaps upward into the search field/titlebar geometry: "
            f"search_shell={search_shell_rect!r} search_dropdown={search_dropdown_rect!r}"
        )
    if rect_bottom(search_modal_rect) < rect_bottom(search_dropdown_rect) + 2.0:
        raise AssertionError(
            f"focused search modal does not enclose the dropdown body: "
            f"modal={search_modal_rect!r} dropdown={search_dropdown_rect!r}"
        )
    if float(search_modal_rect["width"]) < 280.0:
        raise AssertionError(
            f"focused search modal collapsed below the usable width floor: modal={search_modal_rect!r}"
        )
    if (
        float(search_dropdown_header_rect["left"]) < float(search_modal_rect["left"]) - 1.0
        or float(search_dropdown_header_rect["right"]) > float(search_modal_rect["right"]) + 1.0
        or float(search_dropdown_header_rect["top"]) < float(search_modal_rect["top"]) - 1.0
        or float(search_dropdown_header_rect["bottom"]) > float(search_modal_rect["bottom"]) + 1.0
    ):
        raise AssertionError(
            f"search dropdown header overflowed outside the modal bounds: "
            f"header={search_dropdown_header_rect!r} modal={search_modal_rect!r}"
        )
    for ix, rect in enumerate(search_dropdown_entry_rects):
        if (
            float(rect["left"]) < float(search_modal_rect["left"]) - 1.0
            or float(rect["right"]) > float(search_modal_rect["right"]) + 1.0
            or float(rect["top"]) < float(search_modal_rect["top"]) - 1.0
            or float(rect["bottom"]) > float(search_modal_rect["bottom"]) + 1.0
        ):
            raise AssertionError(
                f"search dropdown entry {ix} overflowed outside the modal bounds: rect={rect!r} modal={search_modal_rect!r}"
            )
    dom = focused.get("dom") or {}
    search_input_background = str(dom.get("titlebar_search_input_background") or "")
    search_input_box_shadow = str(dom.get("titlebar_search_input_box_shadow") or "").lower()
    search_shell_background = str(dom.get("titlebar_search_outer_shell_background") or "")
    search_shell_box_shadow = str(dom.get("titlebar_search_outer_shell_box_shadow") or "").lower()
    search_dropdown_background = str(dom.get("titlebar_search_dropdown_background") or "")
    search_dropdown_box_shadow = str(dom.get("titlebar_search_dropdown_box_shadow") or "").lower()
    if not is_transparent_css_color(search_input_background):
        raise AssertionError(
            f"focused search input still draws its own chrome instead of blending into the floating shell: "
            f"background={search_input_background!r} dom={dom!r}"
        )
    if search_input_box_shadow not in ("", "none"):
        raise AssertionError(
            f"focused search input still draws an inner chip shadow: box_shadow={search_input_box_shadow!r}"
        )
    if is_transparent_css_color(search_shell_background):
        raise AssertionError(
            f"focused search shell lost its floating panel background: {search_shell_background!r}"
        )
    if search_shell_box_shadow in ("", "none"):
        raise AssertionError(
            f"focused search shell lost its floating panel shadow: {search_shell_box_shadow!r}"
        )
    if search_dropdown_background not in ("", "transparent", "rgba(0, 0, 0, 0)"):
        raise AssertionError(
            f"focused search dropdown regained its own background instead of blending into the shared panel: "
            f"background={search_dropdown_background!r}"
        )
    if search_dropdown_box_shadow not in ("", "none"):
        raise AssertionError(
            f"focused search dropdown regained its own shadow instead of relying on the shared panel shell: "
            f"box_shadow={search_dropdown_box_shadow!r}"
        )
    click_y = float(host_rect["top"]) + min(32.0, max(16.0, float(host_rect["height"]) * 0.08))
    if rect_is_visible(search_modal_rect):
        click_y = max(click_y, rect_bottom(search_modal_rect) + 14.0)
    click_y = min(click_y, float(host_rect["top"]) + float(host_rect["height"]) - 18.0)
    click = xdotool_click_window(
        pid,
        float(host_rect["left"]) + min(28.0, max(12.0, float(host_rect["width"]) * 0.08)),
        click_y,
    )
    deadline = time.time() + 6.0
    restored = {}
    retried_focus_click = None
    while time.time() < deadline:
        restored = app_state(pid)
        if not ((restored.get("shell") or {}).get("search_focused")):
            restored_host = host_for_session(restored, session)
            if (
                restored_host.get("input_enabled") is True
                and (
                    restored_host.get("helper_textarea_focused") is True
                    or restored_host.get("host_has_active_element") is True
                )
            ):
                break
            if retried_focus_click is None:
                retried_focus_click = xdotool_click_window(
                    pid,
                    float(host_rect["left"]) + min(28.0, max(12.0, float(host_rect["width"]) * 0.08)),
                    click_y,
                )
        time.sleep(0.12)
    if (restored.get("shell") or {}).get("search_focused"):
        raise AssertionError(
            f"search focus did not release after clicking the terminal viewport: click={click!r} restored={restored!r}"
        )
    restored_host = host_for_session(restored, session)
    active_element = (restored.get("dom") or {}).get("active_element") or {}
    if str(active_element.get("id") or "") == "yggterm-search-input":
        raise AssertionError(
            f"search input stayed active after clicking the terminal viewport: click={click!r} active_element={active_element!r}"
        )
    if restored_host.get("input_enabled") is not True:
        raise AssertionError(
            f"terminal host stayed input-disabled after clicking the viewport from focused search: "
            f"click={click!r} host={restored_host!r}"
        )
    if not (
        restored_host.get("helper_textarea_focused") is True
        or restored_host.get("host_has_active_element") is True
    ):
        raise AssertionError(
            f"terminal did not reclaim input focus after clicking the viewport: click={click!r} host={restored_host!r}"
        )
    app_set_search(pid, "", focused=False)
    time.sleep(0.1)
    restored = app_state(pid)
    return {
        "baseline_titlebar_rect": baseline_titlebar,
        "focused_titlebar_rect": titlebar_rect,
        "search_modal_rect": search_modal_rect,
        "search_shell_rect": search_shell_rect,
        "search_dropdown_rect": search_dropdown_rect,
        "search_dropdown_header_rect": search_dropdown_header_rect,
        "search_dropdown_entry_rects": search_dropdown_entry_rects,
        "baseline_host_rect": baseline_host,
        "focused_host_rect": host_rect,
        "search_input_background": search_input_background,
        "search_input_box_shadow": search_input_box_shadow,
        "search_shell_background": search_shell_background,
        "search_shell_box_shadow": search_shell_box_shadow,
        "search_dropdown_background": search_dropdown_background,
        "search_dropdown_box_shadow": search_dropdown_box_shadow,
        "focus_release_click": click,
        "retry_focus_release_click": retried_focus_click,
        "restored_search_focused": ((restored.get("shell") or {}).get("search_focused")),
        "restored_active_element": active_element,
    }


def assert_titlebar_new_menu_shell_contract(pid: int) -> dict:
    app_set_search(pid, "", focused=False)
    time.sleep(0.1)
    baseline = app_state(pid)
    if titlebar_transient_open(baseline):
        baseline = dismiss_titlebar_transients(pid, baseline, timeout_seconds=1.5)
    if right_panel_mode(baseline) not in ("", "hidden", "none", "null", "settings"):
        baseline = close_right_panel(pid, baseline, timeout_seconds=1.5)
    titlebar_rect = dom_rect(baseline, "titlebar_rect")
    button_rect = dom_rect(baseline, "titlebar_new_button_rect")
    if not rect_is_visible(titlebar_rect):
        raise AssertionError(f"titlebar rect missing before new-menu probe: {titlebar_rect!r}")
    if not rect_is_visible(button_rect):
        raise AssertionError(f"titlebar new button rect missing before probe: {button_rect!r}")

    click = xdotool_click_window(pid, rect_center_x(button_rect), rect_center_y(button_rect))
    deadline = time.time() + 6.0
    opened = {}
    while time.time() < deadline:
        opened = app_state(pid)
        if rect_is_visible(dom_rect(opened, "titlebar_new_menu_rect")):
            break
        time.sleep(0.12)
    menu_rect = dom_rect(opened, "titlebar_new_menu_rect")
    opened_button_rect = dom_rect(opened, "titlebar_new_button_rect")
    opened_button_background = str((opened.get("dom") or {}).get("titlebar_new_button_background") or "").strip()
    menu_background = str((opened.get("dom") or {}).get("titlebar_new_menu_background") or "").strip()
    menu_box_shadow = str((opened.get("dom") or {}).get("titlebar_new_menu_box_shadow") or "").strip()
    menu_action_rects = list((opened.get("dom") or {}).get("titlebar_new_menu_action_rects") or [])
    if not rect_is_visible(menu_rect):
        raise AssertionError(f"titlebar new menu rect missing after click: click={click!r} state={opened!r}")
    if float(opened_button_rect["height"]) < 27.0:
        raise AssertionError(
            f"titlebar new tab height is still too small for the focused chrome lane: button={opened_button_rect!r}"
        )
    if float(opened_button_rect["top"]) < float(titlebar_rect["top"]) + 2.0:
        raise AssertionError(
            f"titlebar new tab is crowding the titlebar top edge instead of hanging from it: "
            f"titlebar={titlebar_rect!r} button={opened_button_rect!r}"
        )
    if abs(float(menu_rect["top"]) - rect_bottom(opened_button_rect)) > 2.5:
        raise AssertionError(
            f"titlebar new panel body is detached from its tab instead of forming one attached shell: "
            f"button={opened_button_rect!r} menu={menu_rect!r}"
        )
    if float(menu_rect["left"]) > float(opened_button_rect["left"]) + 2.0:
        raise AssertionError(
            f"titlebar new panel body shifted right of its tab instead of hanging from the same anchor: "
            f"button={opened_button_rect!r} menu={menu_rect!r}"
        )
    if rect_bottom(menu_rect) <= rect_bottom(titlebar_rect) + 8.0:
        raise AssertionError(
            f"titlebar new panel body did not extend below the titlebar: titlebar={titlebar_rect!r} menu={menu_rect!r}"
        )
    if float(menu_rect["width"]) < 240.0:
        raise AssertionError(
            f"titlebar new panel collapsed below its usable width floor: menu={menu_rect!r}"
        )
    if is_transparent_css_color(menu_background):
        raise AssertionError(
            f"titlebar new panel lost its shared modal background: background={menu_background!r}"
        )
    if menu_box_shadow in ("", "none"):
        raise AssertionError(
            f"titlebar new panel lost its shared modal shadow: box_shadow={menu_box_shadow!r}"
        )
    for ix, rect in enumerate(menu_action_rects):
        if (
            float(rect["left"]) < float(menu_rect["left"]) - 1.0
            or float(rect["right"]) > float(menu_rect["right"]) + 1.0
            or float(rect["top"]) < float(menu_rect["top"]) - 1.0
            or float(rect["bottom"]) > float(menu_rect["bottom"]) + 1.0
        ):
            raise AssertionError(
                f"titlebar new-menu action {ix} overflowed outside the panel bounds: rect={rect!r} menu={menu_rect!r}"
            )
    opened_shot = Path(f"/tmp/yggterm-titlebar-new-menu-open-{pid}.png")
    app_screenshot(pid, opened_shot)
    opened_image = Image.open(opened_shot)
    button_background = parse_css_rgb(opened_button_background)
    menu_background_rgb = parse_css_rgb(menu_background)
    if button_background is None:
        sampled = sampled_fill_color(
            opened_image,
            (
                int(round(float(opened_button_rect["left"]) + 8)),
                int(round(float(opened_button_rect["top"]) + 3)),
                int(round(float(opened_button_rect["right"]) - 8)),
                int(round(float(opened_button_rect["top"]) + 10)),
            ),
        )
        if sampled is not None:
            button_background = tuple(channel / 255.0 for channel in sampled)
    if menu_background_rgb is None:
        sampled = sampled_fill_color(
            opened_image,
            (
                int(round(float(menu_rect["left"]) + 10)),
                int(round(float(menu_rect["top"]) + 8)),
                int(round(float(menu_rect["left"]) + 80)),
                int(round(float(menu_rect["top"]) + 22)),
            ),
        )
        if sampled is not None:
            menu_background_rgb = tuple(channel / 255.0 for channel in sampled)
    if button_background is None or menu_background_rgb is None:
        raise AssertionError(
            "titlebar new-menu contract could not determine button/body fills from CSS or screenshot sampling: "
            f"button={opened_button_background!r} menu={menu_background!r}"
        )
    background_delta = max(
        abs(button_background[0] - menu_background_rgb[0]),
        abs(button_background[1] - menu_background_rgb[1]),
        abs(button_background[2] - menu_background_rgb[2]),
    )
    if background_delta > 0.02:
        raise AssertionError(
            f"titlebar new-menu tab and body use different fills instead of reading as one surface: "
            f"button={opened_button_background!r} menu={menu_background!r} delta={background_delta!r}"
        )
    seam_band = clamp_box(
        (
            int(round(float(opened_button_rect["left"]) + 8)),
            int(round(rect_bottom(opened_button_rect) - 1)),
            int(round(float(opened_button_rect["right"]) - 8)),
            int(round(rect_bottom(opened_button_rect) + 2)),
        ),
        opened_image.size,
    )
    if seam_band is None:
        raise AssertionError("titlebar new-menu seam probe fell outside the screenshot bounds")
    seam_crop = opened_image.crop(seam_band)
    seam_background = tuple(int(channel * 255) for channel in menu_background_rgb)
    mismatch_pixels = count_background_mismatch_pixels(
        seam_crop,
        background=seam_background,
        tolerance=12,
    )
    if mismatch_pixels > 12:
        raise AssertionError(
            f"titlebar new-menu seam is still visually present in the screenshot band: "
            f"mismatch_pixels={mismatch_pixels} seam_band={seam_band}"
        )
    xdotool_click_window(pid, rect_center_x(button_rect), rect_center_y(button_rect))
    time.sleep(0.15)
    return {
        "click": click,
        "titlebar_rect": titlebar_rect,
        "button_rect": opened_button_rect,
        "button_background": opened_button_background,
        "menu_rect": menu_rect,
        "menu_background": menu_background,
        "menu_box_shadow": menu_box_shadow,
        "menu_action_rects": menu_action_rects,
        "seam_mismatch_pixels": mismatch_pixels,
    }


def assert_sidebar_resize_persists_to_settings(pid: int) -> dict:
    app_set_search(pid, "", focused=False)
    time.sleep(0.1)
    baseline = wait_for_interactive(pid, timeout_seconds=10.0)
    sidebar_rect = dom_rect(baseline, "sidebar_rect")
    handle_rect = dom_rect(baseline, "sidebar_resize_handle_rect")
    baseline_width = float(((baseline.get("shell") or {}).get("sidebar_width")) or 0.0)
    baseline_setting = float(((baseline.get("settings") or {}).get("tree_width")) or 0.0)
    if not rect_is_visible(sidebar_rect) or not rect_is_visible(handle_rect):
        sidebar_button_rect = dom_rect(baseline, "titlebar_sidebar_button_rect")
        if not rect_is_visible(sidebar_button_rect):
            raise AssertionError(
                f"sidebar rect missing before resize probe and sidebar toggle button is unavailable: "
                f"sidebar={sidebar_rect!r} handle={handle_rect!r} button={sidebar_button_rect!r}"
            )
        xdotool_click_window(pid, rect_center_x(sidebar_button_rect), rect_center_y(sidebar_button_rect))
        deadline = time.time() + 4.0
        while time.time() < deadline:
            baseline = app_state(pid)
            sidebar_rect = dom_rect(baseline, "sidebar_rect")
            handle_rect = dom_rect(baseline, "sidebar_resize_handle_rect")
            if rect_is_visible(sidebar_rect) and rect_is_visible(handle_rect):
                break
            time.sleep(0.12)
    if not rect_is_visible(sidebar_rect):
        raise AssertionError(f"sidebar rect missing before resize probe: {sidebar_rect!r}")
    if not rect_is_visible(handle_rect):
        raise AssertionError(f"sidebar resize handle rect missing before resize probe: {handle_rect!r}")
    if baseline_width <= 0 or baseline_setting <= 0:
        raise AssertionError(
            f"sidebar width missing from app state before resize probe: shell={baseline.get('shell')} settings={baseline.get('settings')}"
        )

    delta = 56.0
    expected_widened_width = min(SIDEBAR_MAX_WIDTH, baseline_width + delta)
    drag = xdotool_drag_window(
        pid,
        rect_center_x(handle_rect),
        rect_center_y(handle_rect),
        rect_center_x(handle_rect) + delta,
        rect_center_y(handle_rect),
    )
    deadline = time.time() + 4.0
    widened = {}
    while time.time() < deadline:
        widened = app_state(pid)
        widened_width = float(((widened.get("shell") or {}).get("sidebar_width")) or 0.0)
        if widened_width >= expected_widened_width - 2.0:
            break
        time.sleep(0.12)
    widened_width = float(((widened.get("shell") or {}).get("sidebar_width")) or 0.0)
    widened_setting = float(((widened.get("settings") or {}).get("tree_width")) or 0.0)
    if widened_width < expected_widened_width - 2.0:
        raise AssertionError(
            f"sidebar width did not respond to drag: drag={drag!r} baseline_width={baseline_width} widened={widened!r}"
        )
    if abs(widened_width - widened_setting) > 1.5:
        raise AssertionError(
            f"sidebar width drifted from persisted settings after drag: width={widened_width} setting={widened_setting}"
        )

    restore_drag = xdotool_drag_window(
        pid,
        rect_center_x(handle_rect) + delta,
        rect_center_y(handle_rect),
        rect_center_x(handle_rect),
        rect_center_y(handle_rect),
    )
    deadline = time.time() + 4.0
    restored = {}
    while time.time() < deadline:
        restored = app_state(pid)
        restored_width = float(((restored.get("shell") or {}).get("sidebar_width")) or 0.0)
        if abs(restored_width - baseline_width) <= 12.0:
            break
        time.sleep(0.12)
    restored_width = float(((restored.get("shell") or {}).get("sidebar_width")) or 0.0)
    if abs(restored_width - baseline_width) > 12.0:
        raise AssertionError(
            f"sidebar width did not restore after inverse drag: baseline_width={baseline_width} restored_width={restored_width} drag={restore_drag!r}"
        )
    return {
        "baseline_width": baseline_width,
        "baseline_setting": baseline_setting,
        "widened_width": widened_width,
        "widened_setting": widened_setting,
        "restored_width": restored_width,
        "drag": drag,
        "restore_drag": restore_drag,
    }


def assert_titlebar_session_shell_contract(pid: int) -> dict:
    app_set_search(pid, "", focused=False)
    time.sleep(0.1)
    baseline = wait_for_window_focus(pid, timeout_seconds=3.0)
    if titlebar_transient_open(baseline):
        baseline = dismiss_titlebar_transients(pid, baseline, timeout_seconds=1.5)
    if right_panel_mode(baseline) not in ("", "hidden", "none", "null"):
        baseline = close_right_panel(pid, baseline, timeout_seconds=1.5)
    existing_new_menu = dom_rect(baseline, "titlebar_new_menu_rect")
    existing_new_button = dom_rect(baseline, "titlebar_new_button_rect")
    if rect_is_visible(existing_new_menu) and rect_is_visible(existing_new_button):
        xdotool_click_window(
            pid,
            rect_center_x(existing_new_button),
            rect_center_y(existing_new_button),
        )
        time.sleep(0.25)
        baseline = app_state(pid)
    existing_summary_shell = dom_rect(baseline, "titlebar_summary_shell_rect")
    existing_button_rect = dom_rect(baseline, "titlebar_session_button_rect")
    if rect_is_visible(existing_summary_shell) and rect_is_visible(existing_button_rect):
        xdotool_click_window(
            pid,
            rect_center_x(existing_button_rect),
            rect_center_y(existing_button_rect),
        )
        time.sleep(0.25)
        baseline = app_state(pid)
    titlebar_rect = dom_rect(baseline, "titlebar_rect")
    button_rect = dom_rect(baseline, "titlebar_session_button_rect")
    baseline_button_rect = button_rect
    if not rect_is_visible(titlebar_rect):
        raise AssertionError(f"titlebar rect missing before session-menu probe: {titlebar_rect!r}")
    if not rect_is_visible(button_rect):
        return {"skipped": True, "reason": "no active titlebar session button"}

    search_input_rect = dom_rect(baseline, "titlebar_search_input_rect")
    search_click = None
    if rect_is_visible(search_input_rect):
        search_click = xdotool_click_window(
            pid,
            rect_center_x(search_input_rect),
            rect_center_y(search_input_rect),
        )
        deadline = time.time() + 6.0
        search_opened = {}
        while time.time() < deadline:
            search_opened = app_state(pid)
            if rect_is_visible(dom_rect(search_opened, "titlebar_search_dropdown_rect")):
                active_element = (search_opened.get("dom") or {}).get("active_element") or {}
                if str(active_element.get("id") or "") == "yggterm-search-input":
                    break
            time.sleep(0.12)
        if not rect_is_visible(dom_rect(search_opened, "titlebar_search_dropdown_rect")):
            raise AssertionError(
                f"search shell did not open before the session-shell cross-state probe: "
                f"search_click={search_click!r} state={search_opened!r}"
            )
        baseline = search_opened
        titlebar_rect = dom_rect(baseline, "titlebar_rect")
        button_rect = dom_rect(baseline, "titlebar_session_button_rect")
        baseline_button_rect = button_rect

    new_button_rect = dom_rect(baseline, "titlebar_new_button_rect")
    plus_open_click = None
    plus_opened = baseline
    plus_cross_hit_skipped = False
    if rect_is_visible(new_button_rect):
        plus_open_click = xdotool_click_window(
            pid,
            rect_center_x(new_button_rect),
            rect_center_y(new_button_rect),
        )
        deadline = time.time() + 4.0
        while time.time() < deadline:
            plus_opened = app_state(pid)
            if rect_is_visible(dom_rect(plus_opened, "titlebar_new_menu_rect")):
                break
            time.sleep(0.12)
        if not rect_is_visible(dom_rect(plus_opened, "titlebar_new_menu_rect")):
            plus_cross_hit_skipped = True
        else:
            titlebar_rect = dom_rect(plus_opened, "titlebar_rect")
            button_rect = dom_rect(plus_opened, "titlebar_session_button_rect")

    click = None
    opened = {}
    for attempt in range(2):
        click = xdotool_click_window(pid, rect_center_x(button_rect), rect_center_y(button_rect))
        deadline = time.time() + 6.0
        retried_focus = False
        while time.time() < deadline:
            opened = app_state(pid)
            if (
                (
                    rect_is_visible(dom_rect(opened, "titlebar_summary_shell_rect"))
                    or rect_is_visible(dom_rect(opened, "titlebar_summary_menu_rect"))
                )
                and not rect_is_visible(dom_rect(opened, "titlebar_new_menu_rect"))
                and not rect_is_visible(dom_rect(opened, "titlebar_search_dropdown_rect"))
                and rect_is_visible(dom_rect(opened, "titlebar_summary_regenerate_summary_rect"))
            ):
                break
            if not retried_focus and not ((opened.get("window") or {}).get("focused")):
                opened = wait_for_window_focus(pid, timeout_seconds=2.0)
                click = xdotool_click_window(
                    pid,
                    rect_center_x(button_rect),
                    rect_center_y(button_rect),
                )
                retried_focus = True
            time.sleep(0.12)
        if rect_is_visible(dom_rect(opened, "titlebar_summary_menu_rect")):
            break
        if attempt == 0:
            baseline = wait_for_window_focus(pid, timeout_seconds=2.0)
            if right_panel_mode(baseline) not in ("", "hidden", "none", "null"):
                baseline = close_right_panel(pid, baseline, timeout_seconds=1.5)
            button_rect = dom_rect(baseline, "titlebar_session_button_rect")
            if not rect_is_visible(button_rect):
                raise AssertionError(
                    f"titlebar session button disappeared while retrying the session-shell contract: {baseline!r}"
                )
    summary_shell_rect = dom_rect(opened, "titlebar_summary_shell_rect")
    opened_button_rect = dom_rect(opened, "titlebar_session_button_rect")
    summary_menu_rect = dom_rect(opened, "titlebar_summary_menu_rect")
    summary_body_rect = dom_rect(opened, "titlebar_summary_body_rect")
    regenerate_summary_rect = dom_rect(opened, "titlebar_summary_regenerate_summary_rect")
    opened_button_background = str(opened.get("titlebar_session_button_background") or "").strip()
    opened_menu_background = str(opened.get("titlebar_summary_menu_background") or "").strip()
    opened_button_box_shadow = str(opened.get("titlebar_session_button_box_shadow") or "").strip()
    summary_menu_box_shadow = str(opened.get("titlebar_summary_menu_box_shadow") or "").strip()
    regenerate_summary_background = str(
        opened.get("titlebar_summary_regenerate_summary_background") or ""
    ).strip()
    regenerate_summary_box_shadow = str(
        opened.get("titlebar_summary_regenerate_summary_box_shadow") or ""
    ).strip()
    summary_menu_border_top_width = str(
        opened.get("titlebar_summary_menu_border_top_width") or ""
    ).strip()
    new_menu_rect_after_session_click = dom_rect(opened, "titlebar_new_menu_rect")
    if not (rect_is_visible(summary_shell_rect) or rect_is_visible(summary_menu_rect)):
        raise AssertionError(f"titlebar session shell did not open after click: click={click!r} state={opened!r}")
    if not rect_is_visible(summary_menu_rect):
        raise AssertionError(f"titlebar summary menu rect missing after click: shell={summary_shell_rect!r} state={opened!r}")
    if rect_is_visible(new_menu_rect_after_session_click):
        raise AssertionError(
            f"titlebar new menu stayed open and blocked the session shell contract: "
            f"session_click={click!r} new_menu={new_menu_rect_after_session_click!r} state={opened!r}"
        )
    if rect_is_visible(dom_rect(opened, "titlebar_search_dropdown_rect")):
        raise AssertionError(
            f"titlebar session shell opened while the search shell was still visible: state={opened!r}"
        )
    if abs(float(opened_button_rect["top"]) - float(baseline_button_rect["top"])) > 1.5:
        raise AssertionError(
            f"titlebar summary tab drifted vertically when opened instead of staying on the titlebar lane: "
            f"baseline={baseline_button_rect!r} button={opened_button_rect!r} titlebar={titlebar_rect!r}"
        )
    if float(opened_button_rect["top"]) < float(titlebar_rect["top"]) + 2.0:
        raise AssertionError(
            f"titlebar summary tab is crowding the top edge instead of hanging from the chip: "
            f"titlebar={titlebar_rect!r} button={opened_button_rect!r}"
        )
    if abs(float(opened_button_rect["height"]) - float(baseline_button_rect["height"])) > 1.2:
        raise AssertionError(
            f"titlebar summary tab changed height instead of matching the closed chip: "
            f"baseline={baseline_button_rect!r} button={opened_button_rect!r}"
        )
    if float(opened_button_rect["height"]) < 27.0:
        raise AssertionError(
            f"titlebar summary tab height is still too small for the focused modal chrome lane: button={opened_button_rect!r}"
        )
    if abs(float(opened_button_rect["width"]) - float(baseline_button_rect["width"])) > 1.5:
        raise AssertionError(
            f"titlebar summary tab changed width instead of preserving the closed chip width: "
            f"baseline={baseline_button_rect!r} button={opened_button_rect!r}"
        )
    if abs(float(summary_menu_rect["top"]) - rect_bottom(opened_button_rect)) > 2.5:
        raise AssertionError(
            f"titlebar summary body detached from its tab instead of expanding as one attached polygon: "
            f"button={opened_button_rect!r} menu={summary_menu_rect!r}"
        )
    if float(summary_menu_rect["left"]) > float(opened_button_rect["left"]) + 2.0:
        raise AssertionError(
            f"titlebar summary body shifted right of the title tab instead of sharing the same anchor: "
            f"button={opened_button_rect!r} menu={summary_menu_rect!r}"
        )
    if rect_bottom(summary_menu_rect) <= rect_bottom(titlebar_rect) + 8.0:
        raise AssertionError(
            f"titlebar summary body did not expand below the titlebar as a hanging panel: "
            f"titlebar={titlebar_rect!r} menu={summary_menu_rect!r}"
        )
    if float(summary_menu_rect["width"]) < 352.0:
        raise AssertionError(
            f"titlebar summary panel collapsed below the usable width floor: menu={summary_menu_rect!r}"
        )
    if "inset" in opened_button_box_shadow.lower():
        raise AssertionError(
            f"titlebar summary tab is still painting an inner border seam instead of blending into the body: "
            f"box_shadow={opened_button_box_shadow!r} state={opened!r}"
        )
    if "inset" in summary_menu_box_shadow.lower():
        raise AssertionError(
            f"titlebar summary body is still painting an inset edge that reads as a seam: "
            f"box_shadow={summary_menu_box_shadow!r} state={opened!r}"
        )
    if summary_menu_border_top_width not in ("", "0", "0px"):
        raise AssertionError(
            f"titlebar summary body still paints a top border seam under the tab: "
            f"border_top_width={summary_menu_border_top_width!r} state={opened!r}"
        )
    if not rect_is_visible(summary_body_rect):
        raise AssertionError(
            f"titlebar summary body rect missing after open: menu={summary_menu_rect!r} state={opened!r}"
        )
    if not rect_is_visible(regenerate_summary_rect):
        raise AssertionError(
            f"titlebar summary regenerate icon is missing or hidden: "
            f"summary={regenerate_summary_rect!r} state={opened!r}"
        )
    for name, rect in (
        ("summary body", summary_body_rect),
        ("regenerate summary", regenerate_summary_rect),
    ):
        if (
            float(rect["left"]) < float(summary_menu_rect["left"]) - 1.0
            or float(rect["right"]) > float(summary_menu_rect["right"]) + 1.0
            or float(rect["top"]) < float(summary_menu_rect["top"]) - 1.0
            or float(rect["bottom"]) > float(summary_menu_rect["bottom"]) + 1.0
        ):
            raise AssertionError(
                f"titlebar {name} overflowed outside the summary menu bounds: "
                f"rect={rect!r} menu={summary_menu_rect!r}"
            )
    body_click = xdotool_click_window(
        pid,
        float(summary_menu_rect["left"]) + min(48.0, max(20.0, float(summary_menu_rect["width"]) * 0.18)),
        float(summary_menu_rect["top"]) + min(40.0, max(18.0, float(summary_menu_rect["height"]) * 0.2)),
    )
    time.sleep(0.2)
    focused = app_state(pid)
    focused_button_rect = dom_rect(focused, "titlebar_session_button_rect")
    focused_summary_shell_rect = dom_rect(focused, "titlebar_summary_shell_rect")
    focused_summary_menu_rect = dom_rect(focused, "titlebar_summary_menu_rect")
    focused_regenerate_summary_rect = dom_rect(focused, "titlebar_summary_regenerate_summary_rect")
    focused_button_background = str(
        focused.get("titlebar_session_button_background") or ""
    ).strip()
    focused_menu_background = str(
        focused.get("titlebar_summary_menu_background") or ""
    ).strip()
    focused_regenerate_summary_background = str(
        focused.get("titlebar_summary_regenerate_summary_background") or ""
    ).strip()
    focused_regenerate_summary_box_shadow = str(
        focused.get("titlebar_summary_regenerate_summary_box_shadow") or ""
    ).strip()
    focused_summary_menu_box_shadow = str(
        focused.get("titlebar_summary_menu_box_shadow") or ""
    ).strip()
    if not rect_is_visible(focused_summary_menu_rect):
        raise AssertionError(
            f"titlebar summary shell did not stay open after focusing the body: body_click={body_click!r} state={focused!r}"
        )
    if abs(float(focused_button_rect["top"]) - float(baseline_button_rect["top"])) > 1.5:
        raise AssertionError(
            f"titlebar summary tab drifted vertically after focus instead of staying on the titlebar lane: "
            f"baseline={baseline_button_rect!r} focused={focused_button_rect!r}"
        )
    if abs(float(focused_button_rect["width"]) - float(baseline_button_rect["width"])) > 1.5:
        raise AssertionError(
            f"titlebar summary tab changed width after focus instead of preserving the closed chip width: "
            f"baseline={baseline_button_rect!r} focused={focused_button_rect!r}"
        )
    if abs(float(focused_summary_menu_rect["top"]) - rect_bottom(focused_button_rect)) > 2.5:
        raise AssertionError(
            f"titlebar summary body detached from its tab after focus: "
            f"button={focused_button_rect!r} menu={focused_summary_menu_rect!r}"
        )
    if rect_is_visible(dom_rect(focused, "titlebar_search_dropdown_rect")):
        raise AssertionError(
            f"titlebar summary shell kept the search shell visible after focusing the body: state={focused!r}"
        )
    if "inset" in focused_summary_menu_box_shadow.lower():
        raise AssertionError(
            f"titlebar summary body regained an inset seam after focus: "
            f"box_shadow={focused_summary_menu_box_shadow!r} state={focused!r}"
        )
    if not rect_is_visible(focused_regenerate_summary_rect):
        raise AssertionError(
            f"titlebar regenerate summary icon disappeared after focusing the summary body: "
            f"summary={focused_regenerate_summary_rect!r}"
        )
    focused_shot = Path(f"/tmp/yggterm-titlebar-shell-focused-{pid}.png")
    app_screenshot(pid, focused_shot)
    focused_image = Image.open(focused_shot)
    button_background = parse_css_rgb(focused_button_background)
    menu_background = parse_css_rgb(focused_menu_background)
    if button_background is None:
        sampled = sampled_fill_color(
            focused_image,
            (
                int(round(float(focused_button_rect["left"]) + 8)),
                int(round(float(focused_button_rect["top"]) + 3)),
                int(round(float(focused_button_rect["right"]) - 8)),
                int(round(float(focused_button_rect["top"]) + 9)),
            ),
        )
        if sampled is not None:
            button_background = tuple(channel / 255.0 for channel in sampled)
    if menu_background is None:
        sampled = sampled_fill_color(
            focused_image,
            (
                int(round(float(focused_summary_menu_rect["left"]) + 10)),
                int(round(float(focused_summary_menu_rect["top"]) + 8)),
                int(round(float(focused_summary_menu_rect["left"]) + 80)),
                int(round(float(focused_summary_menu_rect["top"]) + 20)),
            ),
        )
        if sampled is not None:
            menu_background = tuple(channel / 255.0 for channel in sampled)
    if button_background is None or menu_background is None:
        raise AssertionError(
            "titlebar summary contract could not determine focused fills from CSS or screenshot sampling: "
            f"button={focused_button_background!r} menu={focused_menu_background!r}"
        )
    resolved_button_background = focused_button_background or "rgb({},{},{})".format(
        int(round(button_background[0] * 255.0)),
        int(round(button_background[1] * 255.0)),
        int(round(button_background[2] * 255.0)),
    )
    resolved_menu_background = focused_menu_background or "rgb({},{},{})".format(
        int(round(menu_background[0] * 255.0)),
        int(round(menu_background[1] * 255.0)),
        int(round(menu_background[2] * 255.0)),
    )
    background_delta = max(
        abs(button_background[0] - menu_background[0]),
        abs(button_background[1] - menu_background[1]),
        abs(button_background[2] - menu_background[2]),
    )
    if background_delta > 0.02:
        raise AssertionError(
            f"titlebar summary tab and body use different fills instead of reading as one surface: "
            f"button={focused_button_background!r} menu={focused_menu_background!r} delta={background_delta!r}"
        )
    def resolve_action_fill(background: str, rect: dict) -> tuple[float, float, float] | None:
        parsed = parse_css_rgb(background)
        if parsed is not None:
            return parsed
        probe = clamp_box(
            (
                int(round(float(rect["left"]) + 10)),
                int(round(float(rect["top"]) + 7)),
                int(round(float(rect["right"]) - 10)),
                int(round(float(rect["bottom"]) - 7)),
            ),
            focused_image.size,
        )
        if probe is None:
            return None
        sampled = sampled_fill_color(focused_image, probe)
        if sampled is None:
            return None
        return tuple(channel / 255.0 for channel in sampled)

    for name, background, rect in (("regenerate summary", focused_regenerate_summary_background, focused_regenerate_summary_rect),):
        action_fill = resolve_action_fill(background, rect)
        if action_fill is None:
            raise AssertionError(
                f"titlebar {name} button background is transparent or unreadable instead of a modal button surface: "
                f"background={background!r} rect={rect!r}"
            )
        if action_fill[0] < 0.88 or action_fill[1] < 0.88 or action_fill[2] < 0.88:
            raise AssertionError(
                f"titlebar {name} button fill is darker than the intended light button surface: "
                f"background={background!r} rect={rect!r} fill={action_fill!r}"
            )
        action_delta = max(
            abs(action_fill[0] - menu_background[0]),
            abs(action_fill[1] - menu_background[1]),
            abs(action_fill[2] - menu_background[2]),
        )
        if action_delta < 0.015:
            raise AssertionError(
                f"titlebar {name} button fill is too close to the modal body and does not read like a button: "
                f"background={background!r} rect={rect!r} delta={action_delta!r}"
            )
    top_right_probe = clamp_box(
        (
            int(round(float(focused_summary_menu_rect["right"]) - min(84.0, float(focused_summary_menu_rect["width"]) * 0.28))),
            int(round(float(focused_summary_menu_rect["top"]) + 8)),
            int(round(float(focused_summary_menu_rect["right"]) - 10)),
            int(round(float(focused_summary_menu_rect["top"]) + 28)),
        ),
        focused_image.size,
    )
    if top_right_probe is None:
        raise AssertionError("titlebar summary top-right probe fell outside the screenshot bounds")
    top_right_sample = sampled_fill_color(focused_image, top_right_probe)
    if top_right_sample is None:
        raise AssertionError(
            f"titlebar summary top-right fill could not be sampled: probe={top_right_probe!r}"
        )
    top_right_delta = max(
        abs((top_right_sample[0] / 255.0) - menu_background[0]),
        abs((top_right_sample[1] / 255.0) - menu_background[1]),
        abs((top_right_sample[2] / 255.0) - menu_background[2]),
    )
    if top_right_delta > 0.04:
        raise AssertionError(
            f"titlebar summary top-right fill drifted from the modal background, which usually means chrome overlap or tint bleed: "
            f"probe={top_right_probe!r} sample={top_right_sample!r} menu={focused_menu_background!r} delta={top_right_delta!r}"
        )
    seam_band = clamp_box(
        (
            int(round(float(focused_button_rect["left"]) + 8)),
            int(round(rect_bottom(focused_button_rect) - 1)),
            int(round(float(focused_button_rect["right"]) - 8)),
            int(round(rect_bottom(focused_button_rect) + 2)),
        ),
        focused_image.size,
    )
    if seam_band is None:
        raise AssertionError("titlebar seam probe fell outside the screenshot bounds")
    seam_crop = focused_image.crop(seam_band)
    if menu_background is None:
        _, seam_background = count_non_background_pixels(seam_crop)
    else:
        seam_background = tuple(int(channel * 255) for channel in menu_background)
    mismatch_pixels = count_background_mismatch_pixels(
        seam_crop,
        background=seam_background,
        tolerance=12,
    )
    if mismatch_pixels > 12:
        raise AssertionError(
            f"titlebar summary seam is still visually present in the focused screenshot band: "
            f"mismatch_pixels={mismatch_pixels} seam_band={seam_band}"
        )
    search_input_rect = dom_rect(focused, "titlebar_search_input_rect")
    if rect_is_visible(search_input_rect):
        search_click = xdotool_click_window(
            pid,
            rect_center_x(search_input_rect),
            rect_center_y(search_input_rect),
        )
        deadline = time.time() + 6.0
        switched = {}
        while time.time() < deadline:
            switched = app_state(pid)
            if (
                rect_is_visible(dom_rect(switched, "titlebar_search_dropdown_rect"))
                and not rect_is_visible(dom_rect(switched, "titlebar_summary_menu_rect"))
            ):
                break
            time.sleep(0.12)
        if not rect_is_visible(dom_rect(switched, "titlebar_search_dropdown_rect")):
            raise AssertionError(
                f"clicking search did not open the search shell while the session shell was open: "
                f"search_click={search_click!r} state={switched!r}"
            )
        if rect_is_visible(dom_rect(switched, "titlebar_summary_menu_rect")):
            raise AssertionError(
                f"clicking search left the session shell open instead of transferring chrome ownership: "
                f"search_click={search_click!r} state={switched!r}"
            )
        app_set_search(pid, "", focused=False)
        time.sleep(0.2)
        reopen_click = xdotool_click_window(
            pid,
            rect_center_x(dom_rect(switched, "titlebar_session_button_rect")),
            rect_center_y(dom_rect(switched, "titlebar_session_button_rect")),
        )
        deadline = time.time() + 6.0
        reopened = {}
        while time.time() < deadline:
            reopened = app_state(pid)
            if rect_is_visible(dom_rect(reopened, "titlebar_summary_menu_rect")):
                break
            time.sleep(0.12)
        if not rect_is_visible(dom_rect(reopened, "titlebar_summary_menu_rect")):
            raise AssertionError(
                f"reopening the session shell after the search handoff failed: "
                f"reopen_click={reopen_click!r} state={reopened!r}"
            )
        focused = reopened
        focused_button_rect = dom_rect(focused, "titlebar_session_button_rect")
    close_click = xdotool_click_window(
        pid,
        rect_center_x(focused_button_rect),
        rect_center_y(focused_button_rect),
    )
    deadline = time.time() + 6.0
    restored = {}
    while time.time() < deadline:
        restored = app_state(pid)
        if not rect_is_visible(dom_rect(restored, "titlebar_summary_menu_rect")):
            break
        time.sleep(0.12)
    if rect_is_visible(dom_rect(restored, "titlebar_summary_menu_rect")):
        raise AssertionError(
            f"titlebar session shell did not close after clicking the active chip: "
            f"open_click={click!r} close_click={close_click!r} restored={restored!r}"
        )
    return {
        "plus_open_click": plus_open_click,
        "plus_cross_hit_skipped": plus_cross_hit_skipped,
        "click": click,
        "body_click": body_click,
        "close_click": close_click,
        "titlebar_rect": titlebar_rect,
        "button_rect": focused_button_rect,
        "summary_shell_rect": focused_summary_shell_rect,
        "summary_menu_rect": focused_summary_menu_rect,
        "baseline_button_rect": baseline_button_rect,
        "opened_button_rect": opened_button_rect,
        "button_background": resolved_button_background,
        "menu_background": resolved_menu_background,
        "menu_box_shadow": focused_summary_menu_box_shadow or summary_menu_box_shadow,
        "background_delta": background_delta,
        "seam_mismatch_pixels": mismatch_pixels,
        "menu_open_after_close": rect_is_visible(dom_rect(restored, "titlebar_summary_menu_rect")),
    }


def assert_titlebar_modal_visual_parity(pid: int) -> dict:
    session = str((app_state(pid).get("active_session_path")) or "")
    if not session:
        raise AssertionError("missing active session path for titlebar modal parity probe")
    search = assert_search_focus_overlay_contract(pid, session)
    new_menu = assert_titlebar_new_menu_shell_contract(pid)
    session_shell = assert_titlebar_session_shell_contract(pid)

    search_background = normalize_css_value(search.get("search_shell_background") or "")
    search_box_shadow = normalize_css_value(search.get("search_shell_box_shadow") or "")
    new_background = normalize_css_value(new_menu.get("menu_background") or "")
    new_box_shadow = normalize_css_value(new_menu.get("menu_box_shadow") or "")
    session_background = normalize_css_value(session_shell.get("menu_background") or "")
    session_box_shadow = normalize_css_value(session_shell.get("menu_box_shadow") or "")
    search_background_rgb = css_rgb_tuple(search_background)
    new_background_rgb = css_rgb_tuple(new_background)
    session_background_rgb = css_rgb_tuple(session_background)

    if search_background_rgb and new_background_rgb and session_background_rgb:
        backgrounds_match = (
            search_background_rgb == new_background_rgb == session_background_rgb
        )
    else:
        backgrounds_match = (
            search_background == new_background == session_background
        )
    if not backgrounds_match:
        raise AssertionError(
            f"titlebar modals are still using different panel backgrounds: "
            f"search={search_background!r} new={new_background!r} session={session_background!r}"
        )
    if search_box_shadow != new_box_shadow:
        raise AssertionError(
            f"titlebar modals are still using different panel shadows: "
            f"search={search_box_shadow!r} new={new_box_shadow!r} session={session_box_shadow!r}"
        )
    if session_box_shadow not in ("", search_box_shadow):
        raise AssertionError(
            f"titlebar session panel shadow drifted from the shared modal shadow: "
            f"search={search_box_shadow!r} session={session_box_shadow!r}"
        )
    return {
        "background": search_background,
        "box_shadow": search_box_shadow,
        "search": search,
        "new_menu": new_menu,
        "session": session_shell,
    }


def assert_titlebar_overflow_menu_contract(pid: int) -> dict:
    baseline = app_state(pid)
    titlebar_rect = dom_rect(baseline, "titlebar_rect")
    button_rect = dom_rect(baseline, "titlebar_overflow_button_rect")
    if not rect_is_visible(button_rect):
        return {"skipped": True, "reason": "overflow trigger hidden at this window width"}

    click = xdotool_click_window(pid, rect_center_x(button_rect), rect_center_y(button_rect))
    deadline = time.time() + 6.0
    opened = {}
    while time.time() < deadline:
        opened = app_state(pid)
        if rect_is_visible(dom_rect(opened, "titlebar_overflow_menu_rect")):
            break
        time.sleep(0.12)
    menu_rect = dom_rect(opened, "titlebar_overflow_menu_rect")
    if not rect_is_visible(menu_rect):
        raise AssertionError(
            f"titlebar overflow menu did not open after clicking the ellipsis: click={click!r} state={opened!r}"
        )
    if float(menu_rect["top"]) < float(titlebar_rect["top"]) + 2.0:
        raise AssertionError(
            f"titlebar overflow menu crowded the titlebar top edge: titlebar={titlebar_rect!r} menu={menu_rect!r}"
        )
    if rect_bottom(menu_rect) <= rect_bottom(titlebar_rect) + 8.0:
        raise AssertionError(
            f"titlebar overflow menu did not hang below the titlebar: titlebar={titlebar_rect!r} menu={menu_rect!r}"
        )
    close_click = xdotool_click_window(pid, rect_center_x(button_rect), rect_center_y(button_rect))
    time.sleep(0.15)
    restored = app_state(pid)
    if rect_is_visible(dom_rect(restored, "titlebar_overflow_menu_rect")):
        raise AssertionError(
            f"titlebar overflow menu stayed open after clicking the ellipsis again: open_click={click!r} close_click={close_click!r}"
        )
    return {
        "click": click,
        "close_click": close_click,
        "button_rect": button_rect,
        "menu_rect": menu_rect,
    }


def assert_settings_field_accepts_text_in_terminal_mode(pid: int) -> dict:
    settings_path = current_yggterm_home() / "settings.json"
    persisted_initial_value = None
    if settings_path.exists():
        try:
            persisted_settings = json.loads(settings_path.read_text(encoding="utf-8"))
        except Exception:
            persisted_settings = None
        if isinstance(persisted_settings, dict):
            persisted_initial_value = str(persisted_settings.get("interface_llm_model") or "")
    baseline = app_state(pid)
    if titlebar_transient_open(baseline):
        baseline = dismiss_titlebar_transients(pid, baseline, timeout_seconds=1.5)
    if right_panel_mode(baseline) not in ("", "hidden", "none", "null", "settings"):
        baseline = close_right_panel(pid, baseline, timeout_seconds=1.5)
    time.sleep(0.24)
    baseline = app_state(pid)
    existing_summary_shell = dom_rect(baseline, "titlebar_summary_shell_rect")
    existing_button_rect = dom_rect(baseline, "titlebar_session_button_rect")
    if rect_is_visible(existing_summary_shell) and rect_is_visible(existing_button_rect):
        xdotool_click_window(
            pid,
            rect_center_x(existing_button_rect),
            rect_center_y(existing_button_rect),
        )
        time.sleep(0.25)
        baseline = app_state(pid)
    button_rect = dom_rect(baseline, "titlebar_settings_button_rect")
    button_hit_target = ((baseline.get("dom") or {}).get("titlebar_settings_button_hit_target")) or {}
    if not rect_is_visible(button_rect):
        raise AssertionError(f"settings button rect missing before focus probe: {button_rect!r}")
    if not button_hit_target:
        raise AssertionError(
            f"settings button hit target missing before focus probe: rect={button_rect!r} baseline={baseline!r}"
        )
    click = None
    open_strategy = "titlebar_button"
    if right_panel_mode(baseline) == "settings":
        opened = baseline
    else:
        click = xdotool_click_window(pid, rect_center_x(button_rect), rect_center_y(button_rect))
        deadline = time.time() + 6.0
        opened = {}
        while time.time() < deadline:
            opened = app_state(pid)
            if right_panel_mode(opened) == "settings":
                break
            time.sleep(0.12)
        if right_panel_mode(opened) != "settings":
            fallback_state = opened or app_state(pid)
            host = active_host_or_none(fallback_state)
            host_rect = host.get("host_rect") if host else None
            if rect_is_visible(host_rect):
                xdotool_click_window(
                    pid,
                    float(host_rect["left"]) + 24.0,
                    float(host_rect["top"]) + 42.0,
                )
                time.sleep(0.24)
            reopened = app_state(pid)
            button_rect = dom_rect(reopened, "titlebar_settings_button_rect")
            time.sleep(0.18)
            click = xdotool_click_window(pid, rect_center_x(button_rect), rect_center_y(button_rect))
            deadline = time.time() + 3.0
            while time.time() < deadline:
                opened = app_state(pid)
                if right_panel_mode(opened) == "settings":
                    break
                time.sleep(0.12)
    if right_panel_mode(opened) != "settings":
        opened = open_settings_panel_via_command_lane(pid, timeout_seconds=6.0)
        open_strategy = "command_lane"
    input_rect = dom_rect(opened, "settings_interface_llm_input_rect")
    if not rect_is_visible(input_rect):
        raise AssertionError(f"interface llm input rect missing after opening settings: {opened!r}")
    host = active_host_or_none(opened)
    active_element = ((opened.get("dom") or {}).get("active_element") or {})
    if host is not None and host.get("input_enabled") is not True:
        raise AssertionError(f"terminal input was disabled just by opening settings: host={host!r}")
    if host is not None and host.get("helper_textarea_focused") is not True:
        raise AssertionError(
            f"terminal helper textarea lost focus just by opening settings: host={host!r}"
        )
    if active_element.get("data_settings_field_key") == "interface-llm":
        raise AssertionError(
            f"settings field auto-focused on open instead of waiting for explicit click: active_element={active_element!r}"
        )
    initial_value = str(((opened.get("settings") or {}).get("interface_llm_model")) or "")
    if not initial_value:
        initial_value = str((active_element.get("value")) or "")
    input_click = xdotool_click_window(pid, rect_center_x(input_rect), rect_center_y(input_rect))
    focus_state = {}
    deadline = time.time() + 6.0
    while time.time() < deadline:
        focus_state = app_state(pid)
        current_active = ((focus_state.get("dom") or {}).get("active_element") or {})
        if current_active.get("data_settings_field_key") == "interface-llm":
            break
        time.sleep(0.12)
    current_active = ((focus_state.get("dom") or {}).get("active_element") or {})
    if current_active.get("data_settings_field_key") != "interface-llm":
        raise AssertionError(
            f"interface llm input did not take focus after click: input_click={input_click!r} active_element={current_active!r}"
        )
    caret_to_end = xdotool_key_window(pid, "End")
    typed_text = f"yggprobe{int(time.time() * 1000) % 1_000_000:06d}"
    type_result = xdotool_type_window(pid, typed_text)
    deadline = time.time() + 6.0
    typed = {}
    while time.time() < deadline:
        typed = app_state(pid)
        current_value = str(((typed.get("settings") or {}).get("interface_llm_model")) or "")
        current_active = ((typed.get("dom") or {}).get("active_element") or {})
        current_input_value = str(current_active.get("value") or "")
        candidate = current_value or current_input_value
        if (
            current_active.get("data_settings_field_key") == "interface-llm"
            and candidate == f"{initial_value}{typed_text}"
        ):
            break
        time.sleep(0.12)
    current_value = str(((typed.get("settings") or {}).get("interface_llm_model")) or "")
    current_active = ((typed.get("dom") or {}).get("active_element") or {})
    current_input_value = str(current_active.get("value") or "")
    if current_value == initial_value and current_input_value != initial_value:
        current_value = current_input_value
    if current_value == initial_value:
        raise AssertionError(
            f"interface llm input did not accept direct typing while terminal mode was active: "
            f"initial_value={initial_value!r} current_value={current_value!r} "
            f"input_value={current_input_value!r} active_element={current_active!r}"
        )
    if current_active.get("tag") != "input" or current_active.get("data_settings_field_key") != "interface-llm":
        raise AssertionError(
            f"interface llm input lost focus while typing: active_element={current_active!r}"
        )
    if current_active.get("class_name") == "xterm-helper-textarea":
        raise AssertionError(
            f"terminal helper textarea stole focus from the settings input: active_element={current_active!r}"
        )
    restore_select = None
    restore_delete = [xdotool_key_window(pid, "BackSpace") for _ in typed_text]
    restore_type_result = None
    restored_value_state = {}
    deadline = time.time() + 6.0
    while time.time() < deadline:
        restored_value_state = app_state(pid)
        restored_active = ((restored_value_state.get("dom") or {}).get("active_element") or {})
        restored_value = str(
            ((restored_value_state.get("settings") or {}).get("interface_llm_model"))
            or restored_active.get("value")
            or ""
        )
        if restored_value == initial_value and restored_active.get("data_settings_field_key") == "interface-llm":
            break
        time.sleep(0.12)
    restored_active = ((restored_value_state.get("dom") or {}).get("active_element") or {})
    restored_value = str(
        ((restored_value_state.get("settings") or {}).get("interface_llm_model"))
        or restored_active.get("value")
        or ""
    )
    persisted_restore = None
    target_persisted_value = persisted_initial_value if persisted_initial_value is not None else initial_value
    if settings_path.exists():
        try:
            persisted_settings = json.loads(settings_path.read_text(encoding="utf-8"))
            if isinstance(persisted_settings, dict):
                persisted_settings["interface_llm_model"] = target_persisted_value
                settings_path.write_text(
                    json.dumps(persisted_settings, indent=2) + "\n",
                    encoding="utf-8",
                )
                persisted_restore = {
                    "path": str(settings_path),
                    "restored_value": target_persisted_value,
                }
        except Exception as error:
            persisted_restore = {
                "path": str(settings_path),
                "error": str(error),
            }
    reclaim_host = active_host_or_none(typed)
    reclaim_host_rect = reclaim_host.get("host_rect") if reclaim_host else None
    if not rect_is_visible(reclaim_host_rect):
        raise AssertionError(
            f"terminal host rect missing before viewport-reclaim probe: host={reclaim_host!r} typed={typed!r}"
        )
    reclaim_click_y = float(reclaim_host_rect["top"]) + min(
        36.0, max(18.0, float(reclaim_host_rect["height"]) * 0.08)
    )
    reclaim_click = xdotool_click_window(
        pid,
        float(reclaim_host_rect["left"]) + min(28.0, max(12.0, float(reclaim_host_rect["width"]) * 0.08)),
        reclaim_click_y,
    )
    reclaim_retry_click = None
    reclaimed = {}
    deadline = time.time() + 6.0
    while time.time() < deadline:
        reclaimed = app_state(pid)
        reclaim_active = ((reclaimed.get("dom") or {}).get("active_element") or {})
        reclaim_host = active_host_or_none(reclaimed)
        if (
            right_panel_mode(reclaimed) == "settings"
            and reclaim_host is not None
            and reclaim_host.get("input_enabled") is True
            and (
                reclaim_host.get("helper_textarea_focused") is True
                or reclaim_host.get("host_has_active_element") is True
            )
            and reclaim_active.get("data_settings_field_key")
            not in {"interface-llm", "litellm-endpoint", "litellm-api-key"}
        ):
            break
        if reclaim_retry_click is None and not ((reclaimed.get("window") or {}).get("focused")):
            reclaimed = wait_for_window_focus(pid, timeout_seconds=2.0)
            reclaim_retry_click = xdotool_click_window(
                pid,
                float(reclaim_host_rect["left"]) + min(28.0, max(12.0, float(reclaim_host_rect["width"]) * 0.08)),
                reclaim_click_y,
            )
        time.sleep(0.12)
    reclaim_active = ((reclaimed.get("dom") or {}).get("active_element") or {})
    reclaim_host = active_host_or_none(reclaimed)
    if right_panel_mode(reclaimed) != "settings":
        raise AssertionError(
            f"viewport reclaim unexpectedly closed the settings rail instead of just restoring terminal input: reclaimed={reclaimed!r}"
        )
    if reclaim_host is None or reclaim_host.get("input_enabled") is not True:
        raise AssertionError(
            f"terminal did not reclaim input after clicking the viewport with settings open: "
            f"click={reclaim_click!r} retry={reclaim_retry_click!r} host={reclaim_host!r} reclaimed={reclaimed!r}"
        )
    if not (
        reclaim_host.get("helper_textarea_focused") is True
        or reclaim_host.get("host_has_active_element") is True
    ):
        raise AssertionError(
            f"terminal host did not reclaim active focus after viewport click with settings open: "
            f"click={reclaim_click!r} retry={reclaim_retry_click!r} host={reclaim_host!r} reclaimed={reclaimed!r}"
        )
    if reclaim_active.get("data_settings_field_key") in {"interface-llm", "litellm-endpoint", "litellm-api-key"}:
        raise AssertionError(
            f"settings input kept focus after clicking the terminal viewport: active_element={reclaim_active!r}"
        )
    reclaim_text_before = str(reclaim_host.get("text_sample") or "")
    reclaim_probe_char = "z"
    reclaim_type = xdotool_type_window(pid, reclaim_probe_char)
    reclaim_typed = {}
    deadline = time.time() + 4.0
    while time.time() < deadline:
        reclaim_typed = app_state(pid)
        typed_host = active_host_or_none(reclaim_typed)
        typed_sample = str((typed_host or {}).get("text_sample") or "")
        if reclaim_probe_char in typed_sample and typed_sample != reclaim_text_before:
            break
        time.sleep(0.12)
    typed_host = active_host_or_none(reclaim_typed)
    typed_sample = str((typed_host or {}).get("text_sample") or "")
    if reclaim_probe_char not in typed_sample or typed_sample == reclaim_text_before:
        raise AssertionError(
            f"terminal did not keep accepting typed input after viewport reclaim: "
            f"type={reclaim_type!r} before={reclaim_text_before!r} after={typed_sample!r} state={reclaim_typed!r}"
        )
    settled = {}
    deadline = time.time() + 2.0
    while time.time() < deadline:
        settled = app_state(pid)
        settled_host = active_host_or_none(settled)
        settled_active = ((settled.get("dom") or {}).get("active_element") or {})
        if (
            right_panel_mode(settled) == "settings"
            and settled_host is not None
            and settled_host.get("input_enabled") is True
            and (
                settled_host.get("helper_textarea_focused") is True
                or settled_host.get("host_has_active_element") is True
            )
            and settled_active.get("data_settings_field_key")
            not in {"interface-llm", "litellm-endpoint", "litellm-api-key"}
        ):
            break
        time.sleep(0.12)
    settled_host = active_host_or_none(settled)
    settled_active = ((settled.get("dom") or {}).get("active_element") or {})
    if right_panel_mode(settled) != "settings":
        raise AssertionError(
            f"terminal reclaim did not survive settle window because settings closed: settled={settled!r}"
        )
    if settled_host is None or settled_host.get("input_enabled") is not True:
        raise AssertionError(
            f"terminal reclaim did not survive settle window: host={settled_host!r} settled={settled!r}"
        )
    if settled_active.get("data_settings_field_key") in {"interface-llm", "litellm-endpoint", "litellm-api-key"}:
        raise AssertionError(
            f"settings input stole focus back after viewport reclaim settle: active_element={settled_active!r}"
        )
    reclaim_cleanup = xdotool_key_window(pid, "BackSpace")
    restored = close_right_panel(pid, reclaimed, timeout_seconds=2.5)
    return {
        "open_strategy": open_strategy,
        "open_click": click,
        "button_hit_target": button_hit_target,
        "input_click": input_click,
        "caret_to_end": caret_to_end,
        "type_result": type_result,
        "initial_value": initial_value,
        "typed_text": typed_text,
        "current_value": current_value,
        "active_element": current_active,
        "restore_select": restore_select,
        "restore_delete": restore_delete,
        "restore_type_result": restore_type_result,
        "restore_succeeded": restored_value == initial_value,
        "restored_value": restored_value,
        "restored_active_element": restored_active,
        "reclaim_click": reclaim_click,
        "reclaim_retry_click": reclaim_retry_click,
        "reclaimed_active_element": reclaim_active,
        "reclaim_type": reclaim_type,
        "settled_active_element": settled_active,
        "reclaim_cleanup": reclaim_cleanup,
        "persisted_restore": persisted_restore,
        "restored_right_panel_mode": right_panel_mode(restored),
    }


def assert_theme_editor_contract(pid: int, out_dir: Path) -> dict:
    baseline = app_state(pid)
    if titlebar_transient_open(baseline):
        baseline = dismiss_titlebar_transients(pid, baseline, timeout_seconds=1.5)
    if right_panel_mode(baseline) not in ("", "hidden", "none", "null", "settings"):
        baseline = close_right_panel(pid, baseline, timeout_seconds=1.5)
    if right_panel_mode(baseline) != "settings":
        button_rect = dom_rect(baseline, "titlebar_settings_button_rect")
        if not rect_is_visible(button_rect):
            raise AssertionError(f"settings button rect missing before theme-editor probe: {baseline!r}")
        xdotool_click_window(pid, rect_center_x(button_rect), rect_center_y(button_rect))
        deadline = time.time() + 6.0
        opened = {}
        while time.time() < deadline:
            opened = app_state(pid)
            if right_panel_mode(opened) == "settings":
                baseline = opened
                break
            time.sleep(0.12)
        if right_panel_mode(baseline) != "settings":
            baseline = open_settings_panel_via_command_lane(pid, timeout_seconds=6.0)
    shell_root_before = {
        "background": (baseline.get("dom") or {}).get("shell_root_background"),
        "box_shadow": (baseline.get("dom") or {}).get("shell_root_box_shadow"),
        "border_radius": (baseline.get("dom") or {}).get("shell_root_border_radius"),
    }
    edit_button_rect = dom_rect(baseline, "theme_editor_open_button_rect")
    if not rect_is_visible(edit_button_rect):
        raise AssertionError(f"theme editor open button rect missing: {baseline!r}")
    edit_click = xdotool_click_window(pid, rect_center_x(edit_button_rect), rect_center_y(edit_button_rect))
    deadline = time.time() + 6.0
    opened = {}
    while time.time() < deadline:
        opened = app_state(pid)
        if rect_is_visible(dom_rect(opened, "theme_editor_shell_rect")):
            break
        time.sleep(0.12)
    theme_shell_rect = dom_rect(opened, "theme_editor_shell_rect")
    apply_rect = dom_rect(opened, "theme_editor_apply_button_rect")
    reset_rect = dom_rect(opened, "theme_editor_reset_button_rect")
    seed_rect = dom_rect(opened, "theme_editor_seed_button_rect")
    if not rect_is_visible(theme_shell_rect):
        raise AssertionError(f"theme editor did not open after click: click={edit_click!r} state={opened!r}")
    if not rect_contains_rect(theme_shell_rect, apply_rect):
        raise AssertionError(
            f"theme editor apply button overflows shell bounds: shell={theme_shell_rect!r} apply={apply_rect!r}"
        )
    if not rect_contains_rect(theme_shell_rect, reset_rect):
        raise AssertionError(
            f"theme editor reset button overflows shell bounds: shell={theme_shell_rect!r} reset={reset_rect!r}"
        )
    if rect_is_visible(seed_rect) and not rect_contains_rect(theme_shell_rect, seed_rect):
        raise AssertionError(
            f"theme editor seed button overflows shell bounds: shell={theme_shell_rect!r} seed={seed_rect!r}"
        )
    shell_background = str(((opened.get("dom") or {}).get("theme_editor_shell_background")) or "")
    shell_shadow = str(((opened.get("dom") or {}).get("theme_editor_shell_box_shadow")) or "")
    if not shell_background:
        raise AssertionError(f"theme editor shell background missing: {opened!r}")
    if not shell_shadow:
        raise AssertionError(f"theme editor shell shadow missing: {opened!r}")
    apply_click = xdotool_click_window(pid, rect_center_x(apply_rect), rect_center_y(apply_rect))
    deadline = time.time() + 6.0
    restored = {}
    while time.time() < deadline:
        restored = app_state(pid)
        if not rect_is_visible(dom_rect(restored, "theme_editor_shell_rect")):
            break
        time.sleep(0.12)
    if rect_is_visible(dom_rect(restored, "theme_editor_shell_rect")):
        raise AssertionError(f"theme editor did not close after apply: click={apply_click!r} state={restored!r}")
    shell_root_after = {
        "background": ((restored.get("dom") or {}).get("shell_root_background")),
        "box_shadow": ((restored.get("dom") or {}).get("shell_root_box_shadow")),
        "border_radius": ((restored.get("dom") or {}).get("shell_root_border_radius")),
    }
    if shell_root_after["background"] != shell_root_before["background"]:
        raise AssertionError(
            f"shell root background drifted after theme editor apply without theme-mode change: before={shell_root_before!r} after={shell_root_after!r}"
        )
    if shell_root_after["box_shadow"] != shell_root_before["box_shadow"]:
        raise AssertionError(
            f"shell root frame shadow drifted after theme editor apply: before={shell_root_before!r} after={shell_root_after!r}"
        )
    if shell_root_after["border_radius"] != shell_root_before["border_radius"]:
        raise AssertionError(
            f"shell root border radius drifted after theme editor apply: before={shell_root_before!r} after={shell_root_after!r}"
        )
    shot_path = out_dir / "theme-editor-applied.png"
    app_screenshot(pid, shot_path)
    return {
        "edit_click": edit_click,
        "apply_click": apply_click,
        "shell_rect": theme_shell_rect,
        "shell_background": shell_background,
        "shell_box_shadow": shell_shadow,
        "shell_root_before": shell_root_before,
        "shell_root_after": shell_root_after,
        "screenshot": str(shot_path),
    }


def wait_for_window_maximized(pid: int, enabled: bool, timeout_seconds: float = 10.0) -> dict | None:
    deadline = time.time() + timeout_seconds
    last_state = {}
    baseline_state = app_state(pid)
    baseline_window = baseline_state.get("window") or {}
    baseline_signature = (
        baseline_window.get("maximized"),
        json.dumps(baseline_window.get("outer_position") or {}, sort_keys=True),
        json.dumps(baseline_window.get("outer_size") or {}, sort_keys=True),
    )
    while time.time() < deadline:
        last_state = app_state(pid)
        window = last_state.get("window") or {}
        if window.get("maximized") is enabled:
            return last_state
        time.sleep(0.2)
    last_window = last_state.get("window") or {}
    last_signature = (
        last_window.get("maximized"),
        json.dumps(last_window.get("outer_position") or {}, sort_keys=True),
        json.dumps(last_window.get("outer_size") or {}, sort_keys=True),
    )
    if baseline_signature == last_signature:
        return None
    raise AssertionError(f"window maximize state did not settle to {enabled!r}: {last_state!r}")


def assert_maximize_roundtrip_layout(pid: int) -> dict:
    app_set_search(pid, "", focused=False)
    time.sleep(0.1)
    baseline = wait_for_window_focus(pid, timeout_seconds=4.0)
    if shell_context_menu_row_path(baseline):
        try:
            xdotool_key_window(pid, "Escape")
        except Exception:
            pass
        time.sleep(0.1)
        baseline = app_state(pid)
    if titlebar_transient_open(baseline):
        baseline = dismiss_titlebar_transients(pid, baseline, timeout_seconds=1.5)
    if right_panel_mode(baseline) not in ("", "hidden", "none", "null"):
        baseline = close_right_panel(pid, baseline, timeout_seconds=1.5)
    wait_for_notifications_clear(pid, timeout_seconds=12.0)
    before = wait_for_interactive(pid, timeout_seconds=10.0)
    before_flush = assert_terminal_viewport_inset(before)
    before_titlebar = assert_titlebar_centering(before)

    app_set_maximized(pid, True)
    time.sleep(0.45)
    maximized = wait_for_window_maximized(pid, True)
    if maximized is None:
        app_set_maximized(pid, False)
        return {
            "skipped": True,
            "reason": "maximize unsupported on this X11 session/window manager",
            "before": {
                "flush": before_flush,
                "titlebar": before_titlebar,
            },
        }
    maximized_flush = assert_terminal_viewport_inset(maximized)
    maximized_titlebar = assert_titlebar_centering(maximized)

    app_set_maximized(pid, False)
    time.sleep(0.45)
    restored = wait_for_window_maximized(pid, False)
    restored = wait_for_interactive(pid, timeout_seconds=10.0)
    restored_flush = assert_terminal_viewport_inset(restored)
    restored_titlebar = assert_titlebar_centering(restored)

    return {
        "before": {
            "flush": before_flush,
            "titlebar": before_titlebar,
        },
        "maximized": {
            "flush": maximized_flush,
            "titlebar": maximized_titlebar,
        },
        "restored": {
            "flush": restored_flush,
            "titlebar": restored_titlebar,
        },
    }


def assert_context_menu_rename_session(pid: int) -> dict:
    baseline = wait_for_window_focus(pid, timeout_seconds=4.0)
    session_row = selected_visible_session_row(baseline)
    target_path = str(session_row.get("path") or "").strip()
    if not target_path:
        raise AssertionError(f"selected session row missing path: {session_row!r}")
    active_session = str((baseline.get("viewport") or {}).get("active_session_path") or "").strip()

    opened = {}
    open_click = None
    for _attempt in range(3):
        current = wait_for_window_focus(pid, timeout_seconds=3.0)
        session_row = selected_visible_session_row(current)
        click_x = sidebar_row_click_x(current)
        open_click = xdotool_right_click_window(
            pid,
            click_x,
            rect_center_y(session_row),
        )
        deadline = time.time() + 2.5
        while time.time() < deadline:
            opened = app_state(pid)
            if (
                shell_context_menu_row_path(opened) == target_path
                and rect_is_visible(dom_rect(opened, "context_menu_rename_session_rect"))
            ):
                break
            time.sleep(0.1)
        if (
            shell_context_menu_row_path(opened) == target_path
            and rect_is_visible(dom_rect(opened, "context_menu_rename_session_rect"))
        ):
            break
        xdotool_click_window(
            pid,
            click_x,
            rect_center_y(session_row),
        )
        time.sleep(0.15)
    else:
        raise AssertionError(f"context menu did not open on selected session row: {opened!r}")

    rename_rect = dom_rect(opened, "context_menu_rename_session_rect")
    rename_click = xdotool_click_window(
        pid,
        rect_center_x(rename_rect),
        rect_center_y(rename_rect),
    )
    deadline = time.time() + 2.5
    renamed = {}
    while time.time() < deadline:
        renamed = app_state(pid)
        if (
            str(((renamed.get("shell") or {}).get("tree_rename_path") or "")).strip() == target_path
            and rect_is_visible(dom_rect(renamed, "tree_rename_input_rect"))
        ):
            break
        time.sleep(0.1)
    else:
        raise AssertionError(f"context menu rename action did not enter rename mode: {renamed!r}")
    if not bool((renamed.get("dom") or {}).get("tree_rename_input_focused")):
        raise AssertionError(f"rename mode opened without focusing the rename input: {renamed!r}")

    xdotool_key_window(pid, "Escape")
    deadline = time.time() + 2.5
    cancelled = {}
    while time.time() < deadline:
        cancelled = app_state(pid)
        if not str(((cancelled.get("shell") or {}).get("tree_rename_path") or "")).strip():
            break
        time.sleep(0.1)
    else:
        raise AssertionError(f"rename mode did not cancel cleanly: {cancelled!r}")

    if active_session:
        wait_for_session_focus(pid, active_session, timeout_seconds=10.0)
    return {
        "target_path": target_path,
        "active_session": active_session,
        "open_click": open_click,
        "menu_rect": dom_rect(opened, "context_menu_rect"),
        "rename_rect": rename_rect,
        "rename_click": rename_click,
        "tree_rename_path": (renamed.get("shell") or {}).get("tree_rename_path"),
        "tree_rename_input_rect": dom_rect(renamed, "tree_rename_input_rect"),
    }


def assert_focus_and_visibility(state: dict) -> dict:
    viewport = state.get("viewport") or {}
    host = active_host(state)
    active_element = ((state.get("dom") or {}).get("active_element") or {})
    notifications = visible_notifications(state)
    if viewport.get("ready") is not True or viewport.get("interactive") is not True:
        raise AssertionError(f"terminal not interactive: {viewport!r}")
    if notifications:
        raise AssertionError(f"notifications still visible in interactive state: {notifications!r}")
    if host.get("input_enabled") is not True:
        raise AssertionError(f"terminal input still disabled: {host.get('input_enabled')!r}")
    if host.get("helper_textarea_focused") is not True:
        raise AssertionError("helper textarea is not focused")
    if host.get("host_has_active_element") is not True:
        raise AssertionError("active element is not inside terminal host")
    if active_element.get("class_name") != "xterm-helper-textarea":
        raise AssertionError(f"unexpected active element for terminal input: {active_element!r}")
    return {
        "ready": viewport.get("ready"),
        "interactive": viewport.get("interactive"),
        "terminal_settled_kind": viewport.get("terminal_settled_kind"),
        "active_element": active_element,
        "input_enabled": host.get("input_enabled"),
    }


def assert_text_readability(state: dict) -> dict:
    host = active_host(state)
    bg = str(host.get("xterm_theme_background") or host.get("viewport_background_color") or "")
    rows_color = str(host.get("rows_sample_color") or host.get("rows_color") or "")
    dim_color = str(host.get("dim_sample_color") or "")
    low_contrast_count = int(host.get("low_contrast_span_count") or 0)
    low_contrast_row_count = int(host.get("low_contrast_row_count") or 0)
    row_contrast = contrast_ratio(rows_color, bg)
    dim_contrast = contrast_ratio(dim_color, bg) if dim_color else None
    min_row_contrast = float(host.get("xterm_minimum_contrast_ratio") or 0.0)
    if min_row_contrast <= 0.0:
        min_row_contrast = 8.5 if bg == "#fbfbfd" else 6.5
    if "JetBrains Mono" not in str(host.get("rows_sample_font_family") or host.get("rows_font_family") or ""):
        raise AssertionError(
            f"rows font family drifted from JetBrains Mono stack: {host.get('rows_sample_font_family')!r}"
        )
    if str(host.get("rows_sample_font_weight") or host.get("rows_font_weight")) != "400":
        raise AssertionError(
            f"rows font weight drifted: {host.get('rows_sample_font_weight') or host.get('rows_font_weight')!r}"
        )
    line_height = float(host.get("xterm_line_height") or 0.0)
    if abs(line_height - 1.0) > 0.01:
        raise AssertionError(f"terminal line height drifted from VS Code parity: {line_height!r}")
    if low_contrast_count != 0:
        raise AssertionError(
            f"visible low-contrast spans remain: count={low_contrast_count} samples={host.get('low_contrast_span_samples')!r}"
        )
    if low_contrast_row_count != 0:
        raise AssertionError(
            f"visible low-contrast rows remain: count={low_contrast_row_count} samples={host.get('low_contrast_row_samples')!r}"
        )
    if row_contrast is None or row_contrast < min_row_contrast:
        raise AssertionError(
            f"main row contrast too low: color={rows_color!r} background={bg!r} contrast={row_contrast!r} required={min_row_contrast!r}"
        )
    if dim_color:
        min_dim = 10.0 if bg == "#fbfbfd" else 3.5
        if dim_contrast is None or dim_contrast < min_dim:
            raise AssertionError(
                f"dim row contrast too low: color={dim_color!r} background={bg!r} contrast={dim_contrast!r}"
            )
    if not str(host.get("text_sample") or "").strip():
        raise AssertionError("terminal text sample is empty despite interactive terminal")
    return {
        "background": bg,
        "rows_sample_color": rows_color,
        "rows_contrast": round(row_contrast, 2) if row_contrast is not None else None,
        "min_row_contrast": round(min_row_contrast, 2),
        "dim_sample_color": dim_color,
        "dim_contrast": round(dim_contrast, 2) if dim_contrast is not None else None,
        "low_contrast_span_count": low_contrast_count,
        "low_contrast_row_count": low_contrast_row_count,
        "xterm_line_height": line_height,
    }


def assert_renderer_contract(state: dict) -> dict:
    host = active_host(state)
    canvas_count = int(host.get("canvas_count") or 0)
    renderer_mode = str(host.get("xterm_renderer_mode") or "").strip().lower() or "unknown"
    screen_rect = host.get("screen_rect") or host.get("viewport_rect") or host.get("host_rect") or {}
    xterm_root_user_select = str(host.get("xterm_root_user_select") or "").strip().lower()
    rows_user_select = str(host.get("rows_user_select") or "").strip().lower()
    selection_range_count = int(host.get("selection_range_count") or 0)
    if renderer_mode not in ("canvas", "dom"):
        raise AssertionError(
            f"xterm mounted the wrong renderer: renderer_mode={renderer_mode!r} canvas_count={canvas_count}"
        )
    if renderer_mode == "canvas" and canvas_count <= 0:
        raise AssertionError(
            f"xterm reported canvas renderer without canvas nodes: canvas_count={canvas_count}"
        )
    if renderer_mode == "dom" and canvas_count != 0:
        raise AssertionError(
            f"xterm reported dom renderer but still exposed canvas nodes: canvas_count={canvas_count}"
        )
    if not str(host.get("text_sample") or "").strip():
        raise AssertionError(
            f"xterm renderer mounted without visible terminal text: renderer_mode={renderer_mode!r}"
        )
    if xterm_root_user_select != "none":
        raise AssertionError(
            f"xterm root user-select drifted from native terminal behavior: {xterm_root_user_select!r}"
        )
    if rows_user_select != "none":
        raise AssertionError(
            f"xterm rows user-select drifted from native terminal behavior: {rows_user_select!r}"
        )
    if selection_range_count != 0:
        raise AssertionError(
            f"browser DOM selection leaked into the terminal host: selection_range_count={selection_range_count} text={host.get('selection_text')!r}"
        )
    if int(screen_rect.get("width") or 0) < 280 or int(screen_rect.get("height") or 0) < 140:
        raise AssertionError(
            f"xterm renderer surface is too small: renderer_mode={renderer_mode!r} screen_rect={screen_rect!r}"
        )
    return {
        "canvas_count": canvas_count,
        "renderer_mode": renderer_mode,
        "screen_rect": screen_rect,
        "xterm_root_user_select": xterm_root_user_select,
        "rows_user_select": rows_user_select,
        "selection_range_count": selection_range_count,
        "text_sample": str(host.get("text_sample") or "")[:160],
    }


def assert_local_tree_placement(pid: int, session: str) -> dict:
    rows = app_rows(pid)
    matches = [
        (index, row)
        for index, row in enumerate(rows)
        if str(row.get("full_path") or "") == session
    ]
    if not matches:
        raise AssertionError(f"session is missing from app rows: {session}")
    for index, row in matches:
        if int(row.get("depth") or 0) <= 0:
            continue
        cursor = index - 1
        while cursor >= 0:
            parent = rows[cursor]
            if int(parent.get("depth") or 0) < int(row.get("depth") or 0):
                if str(parent.get("full_path") or "") != "__live_sessions__":
                    return {
                        "row_index": index,
                        "depth": row.get("depth"),
                        "parent_path": parent.get("full_path"),
                        "parent_label": parent.get("label"),
                    }
                break
            cursor -= 1
    raise AssertionError(
        f"session only appears under Live Sessions instead of the local tree: {matches!r}"
    )



def sidebar_dom_row(state: dict, session: str) -> dict | None:
    dom_rows = ((state.get("dom") or {}).get("sidebar_visible_rows") or [])
    return next(
        (
            row
            for row in dom_rows
            if str(row.get("path") or "") == session
        ),
        None,
    )


def assert_busy_icon_lifecycle(pid: int, session: str, *, sleep_seconds: int = 3) -> dict:
    launch = unwrap_data(terminal_send(pid, session, f"sleep {sleep_seconds}\r"))
    if not bool(launch.get("accepted")):
        raise AssertionError(f"terminal send rejected busy lifecycle probe: {launch}")

    busy_row = None
    busy_deadline = time.time() + 4.0
    while time.time() < busy_deadline:
        state = app_state(pid)
        row = sidebar_dom_row(state, session)
        if row and str(row.get("icon_kind") or "") == "busy":
            busy_row = row
            break
        time.sleep(0.2)
    if busy_row is None:
        raise AssertionError("local terminal never entered the busy icon state during a foreground command")

    idle_row = None
    idle_deadline = time.time() + max(10.0, sleep_seconds + 8.0)
    while time.time() < idle_deadline:
        state = app_state(pid)
        row = sidebar_dom_row(state, session)
        if row and not bool(row.get("busy")) and str(row.get("icon_kind") or "") == "plain-terminal":
            idle_row = row
            break
        time.sleep(0.25)
    if idle_row is None:
        raise AssertionError("local terminal did not recover from busy spinner back to the plain terminal icon")

    return {
        "busy_icon_kind": str(busy_row.get("icon_kind") or ""),
        "busy_icon_text": str(busy_row.get("icon_text") or ""),
        "idle_icon_kind": str(idle_row.get("icon_kind") or ""),
        "idle_icon_text": str(idle_row.get("icon_text") or ""),
    }


def assert_sidebar_contract(pid: int, session: str) -> dict:
    state = app_state(pid)
    host = active_host(state)
    snapshot = server_snapshot()
    dom_rows = ((state.get("dom") or {}).get("sidebar_visible_rows") or [])
    if not dom_rows:
        raise AssertionError("sidebar dom rows are missing from app state")

    live_group_index = next(
        (
            index
            for index, row in enumerate(dom_rows)
            if str(row.get("path") or "") == "__live_sessions__"
        ),
        None,
    )
    live_group_children = []
    if live_group_index is not None:
        for row in dom_rows[live_group_index + 1 :]:
            path = str(row.get("path") or "")
            if path.startswith("__remote_machine__/") or path == "local":
                break
            live_group_children.append(row)
    if live_group_index is not None and not live_group_children:
        raise AssertionError("Live Sessions is visible but empty")
    live_group_documents = [
        row
        for row in live_group_children
        if str(row.get("kind") or "") == "Document"
    ]
    if live_group_documents:
        raise AssertionError(
            f"Live Sessions contains document rows instead of only live terminals: {live_group_documents!r}"
        )
    cached_live_rows = [
        row
        for row in live_group_children
        if str(row.get("machine_health") or "").strip().lower() == "cached"
    ]
    if cached_live_rows:
        raise AssertionError(
            f"Live Sessions still contains cached rows instead of only actual live sessions: {cached_live_rows!r}"
        )
    snapshot_live_sessions = {
        normalize_live_path(str(entry.get("session_path") or "")): entry
        for entry in (snapshot.get("live_sessions") or [])
        if str(entry.get("session_path") or "").strip()
    }
    invalid_busy_live_rows = []
    for row in live_group_children:
        row_path = normalize_live_path(str(row.get("path") or ""))
        session_entry = snapshot_live_sessions.get(row_path)
        if session_entry is None:
            continue
        row_busy = bool(row.get("busy")) or str(row.get("icon_kind") or "") == "busy"
        terminal_foreground_active = session_entry.get("terminal_foreground_active")
        if row_busy and terminal_foreground_active is not True:
            invalid_busy_live_rows.append(
                {
                    "row": row,
                    "snapshot": {
                        "launch_phase": session_entry.get("launch_phase"),
                        "source": session_entry.get("source"),
                        "status_line": session_entry.get("status_line"),
                        "terminal_foreground_active": terminal_foreground_active,
                    },
                }
            )
    if invalid_busy_live_rows:
        raise AssertionError(
            "restored live-session rows are still showing busy without a real foreground process: "
            f"{invalid_busy_live_rows!r}"
        )
    invalid_cached_ready_machine_rows = [
        row
        for row in dom_rows
        if str(row.get("path") or "").startswith("__remote_machine__/")
        and str(row.get("machine_health") or "").strip().lower() == "cached"
        and str(row.get("remote_deploy_state") or "").strip() == "Ready"
        and int(row.get("child_count") or 0) > 0
    ]
    if invalid_cached_ready_machine_rows:
        raise AssertionError(
            "remote machine rows are still painted as cached/warning even though a ready remote runtime and sessions exist: "
            f"{invalid_cached_ready_machine_rows!r}"
        )

    selected_row = next(
        (
            row
            for row in dom_rows
            if str(row.get("path") or "") == session
        ),
        None,
    )
    if selected_row is None:
        raise AssertionError(f"selected session is missing from sidebar dom rows: {session}")
    if not bool(selected_row.get("selected")):
        raise AssertionError(
            f"active session row exists but is not the selected sidebar row: {selected_row!r}"
        )
    active_session_path = str(state.get("active_session_path") or "")
    if active_session_path != session:
        raise AssertionError(
            f"active session path drifted away from the proved sidebar session: expected={session!r} actual={active_session_path!r}"
        )
    browser_selected_row = str(
        (((state.get("browser") or {}).get("selected_row") or {}).get("full_path")) or ""
    )
    if browser_selected_row != session:
        raise AssertionError(
            f"browser selected row drifted away from the active session: expected={session!r} actual={browser_selected_row!r}"
        )
    selected_icon_kind = str(selected_row.get("icon_kind") or "")
    selected_icon_text = str(selected_row.get("icon_text") or "")
    selected_busy = bool(selected_row.get("busy")) or selected_icon_kind == "busy"
    active_foreground = state.get("active_session_terminal_foreground_active")
    cursor_line_text = str(host.get("cursor_line_text") or host.get("cursor_row_text") or "")
    shell_prompt_ready = cursor_line_text.rstrip().endswith("$") or "$ " in cursor_line_text
    if selected_busy and (active_foreground is False or shell_prompt_ready):
        raise AssertionError(
            "selected local terminal row is still busy after the shell returned to an idle prompt: "
            f"row={selected_row!r} active_foreground={active_foreground!r} cursor_line={cursor_line_text!r}"
        )
    if selected_busy:
        if selected_icon_kind != "busy" or selected_icon_text:
            raise AssertionError(
                f"selected busy local terminal row lost the busy spinner contract: {selected_row!r}"
            )
    elif selected_icon_kind != "plain-terminal" or selected_icon_text != "⌘":
        raise AssertionError(
            f"selected local terminal row lost the plain terminal icon contract: {selected_row!r}"
        )

    inconsistent_generic_session_icons = [
        row
        for row in dom_rows
        if str(row.get("kind") or "") == "Session"
        and str(row.get("icon_kind") or "") == "session"
    ]
    if inconsistent_generic_session_icons:
        raise AssertionError(
            f"terminal sessions still use the inconsistent generic session icon: {inconsistent_generic_session_icons!r}"
        )
    invalid_codex_icons = [
        row
        for row in dom_rows
        if "/.codex/sessions/" in str(row.get("path") or "")
        and (
            str(row.get("icon_kind") or "") != "codex"
            or str(row.get("icon_text") or "") != "✦"
        )
    ]
    if invalid_codex_icons:
        raise AssertionError(
            f"Codex session rows lost their dedicated icon contract: {invalid_codex_icons!r}"
        )

    failed_notifications = [
        notification
        for notification in visible_notifications(state)
        if str(notification.get("title") or "") == "Remote Terminal Failed"
    ]
    if failed_notifications:
        raise AssertionError(
            f"stale remote terminal failure notification is still visible: {failed_notifications!r}"
        )

    return {
        "live_group_present": live_group_index is not None,
        "live_group_child_count": len(live_group_children),
        "live_group_document_count": len(live_group_documents),
        "browser_selected_row": browser_selected_row,
        "selected_busy": selected_busy,
        "selected_icon_kind": selected_icon_kind,
        "selected_icon_text": selected_icon_text,
        "invalid_busy_live_row_count": len(invalid_busy_live_rows),
        "invalid_cached_ready_machine_row_count": len(invalid_cached_ready_machine_rows),
        "invalid_codex_icon_count": len(invalid_codex_icons),
        "generic_session_icon_count": len(inconsistent_generic_session_icons),
        "failed_notification_count": len(failed_notifications),
    }


def find_hot_switch_partner(snapshot: dict, session: str, session_kind: str) -> dict | None:
    live_sessions = snapshot.get("live_sessions") or []
    normalized_session = normalize_live_path(session)
    preferred_kind = "codex" if session_kind == "plain" else "shell"
    candidates = []
    for entry in live_sessions:
        if not isinstance(entry, dict):
            continue
        entry_path = normalize_live_path(str(entry.get("session_path") or ""))
        if not entry_path or entry_path == normalized_session:
            continue
        if entry_path.startswith("remote-session://"):
            continue
        if str(entry.get("source") or "") != "LiveLocal":
            continue
        kind = normalize_session_kind(entry.get("kind"))
        if kind not in ("shell", "codex"):
            continue
        candidates.append(entry)
    if not candidates:
        return None
    exact = [entry for entry in candidates if normalize_session_kind(entry.get("kind")) == preferred_kind]
    return (exact or candidates)[0]


def warm_hot_switch_partner(pid: int, session: str, session_kind: str) -> dict | None:
    preferred_icon = "codex" if session_kind == "plain" else "plain-terminal"
    normalized_session = normalize_live_path(session)
    snapshot = server_snapshot()
    live_paths = {
        normalize_live_path(str(entry.get("session_path") or ""))
        for entry in (snapshot.get("live_sessions") or [])
        if isinstance(entry, dict) and str(entry.get("session_path") or "").strip()
    }
    rows = app_rows(pid)
    for row in rows:
        if str(row.get("kind") or "") != "Session":
            continue
        full_path = normalize_live_path(str(row.get("full_path") or ""))
        if not full_path or full_path == normalized_session:
            continue
        if full_path not in live_paths:
            continue
        if full_path.startswith("remote-session://") or full_path.startswith("local://"):
            continue
        if str(row.get("icon_kind") or "") != preferred_icon:
            continue
        ok, _, detail = app_open_raw(pid, full_path, view="terminal")
        if not ok:
            raise AssertionError(
                f"failed to warm hot-switch partner: path={full_path!r} detail={detail!r}"
            )
        wait_for_interactive(pid, timeout_seconds=12.0)
        snapshot = server_snapshot()
        partner = find_hot_switch_partner(snapshot, session, session_kind)
        if partner is not None:
            return partner
    return None


def assert_hot_session_switch(pid: int, session: str, session_kind: str, out_dir: Path) -> dict:
    snapshot = server_snapshot()
    partner = find_hot_switch_partner(snapshot, session, session_kind)
    partner_warmed = False
    if partner is None:
        partner = warm_hot_switch_partner(pid, session, session_kind)
        partner_warmed = partner is not None
    if partner is None:
        return {"skipped": True, "reason": "no alternate hot local live session"}

    partner_path = str(partner.get("session_path") or "")
    if partner_warmed:
        ok, _, detail = app_open_raw(pid, session, view="terminal")
        if not ok:
            state = app_state(pid)
            shot_path = out_dir / "hot-switch-return-after-warm-failed.png"
            app_screenshot(pid, shot_path)
            raise AssertionError(
                f"hot switch warmup could not return to the original session: session={session!r} detail={detail!r} state={state!r} screenshot={shot_path}"
            )
        if session_kind == "plain":
            wait_for_terminal_quiescent(pid, timeout_seconds=10.0)
        else:
            wait_for_interactive(pid, timeout_seconds=10.0)

    ok, _, detail = app_open_raw(pid, partner_path, view="terminal")
    if not ok:
        state = app_state(pid)
        shot_path = out_dir / "hot-switch-partner-failed.png"
        app_screenshot(pid, shot_path)
        raise AssertionError(
            f"hot switch to partner timed out or failed: partner={partner_path!r} detail={detail!r} state={state!r} screenshot={shot_path}"
        )
    partner_state = wait_for_interactive(pid, timeout_seconds=10.0)
    if str(partner_state.get("active_session_path") or "") != partner_path:
        raise AssertionError(
            f"hot switch landed on the wrong session: expected={partner_path!r} actual={partner_state.get('active_session_path')!r}"
        )
    partner_selected = str(((partner_state.get("browser") or {}).get("selected_path")) or "")
    if partner_selected != partner_path:
        raise AssertionError(
            f"hot switch left sidebar selection behind: expected={partner_path!r} selected={partner_selected!r}"
        )
    partner_requests = partner_state.get("active_surface_requests") or []
    if partner_requests:
        raise AssertionError(
            f"hot switch kept surface requests in flight after settle: {partner_requests!r}"
        )
    partner_host = active_host(partner_state)
    if not str(partner_host.get("text_sample") or "").strip():
        raise AssertionError(
            f"hot-switched partner session is interactive but still blank: {partner_host!r}"
        )
    partner_shot = out_dir / "hot-switch-partner.png"
    app_screenshot(pid, partner_shot)

    ok, _, detail = app_open_raw(pid, session, view="terminal")
    if not ok:
        state = app_state(pid)
        shot_path = out_dir / "hot-switch-return-failed.png"
        app_screenshot(pid, shot_path)
        raise AssertionError(
            f"hot switch back to original session timed out or failed: session={session!r} detail={detail!r} state={state!r} screenshot={shot_path}"
        )
    if session_kind == "plain":
        final_state = wait_for_terminal_quiescent(pid, timeout_seconds=10.0)
    else:
        final_state = wait_for_interactive(pid, timeout_seconds=10.0)
    if str(final_state.get("active_session_path") or "") != session:
        raise AssertionError(
            f"hot switch back landed on the wrong session: expected={session!r} actual={final_state.get('active_session_path')!r}"
        )
    final_selected = str(((final_state.get("browser") or {}).get("selected_path")) or "")
    if final_selected != session:
        raise AssertionError(
            f"hot switch back left sidebar selection behind: expected={session!r} selected={final_selected!r}"
        )
    final_requests = final_state.get("active_surface_requests") or []
    if final_requests:
        raise AssertionError(
            f"hot switch back kept surface requests in flight after settle: {final_requests!r}"
        )
    final_host = active_host(final_state)
    final_text = str(final_host.get("text_sample") or final_host.get("cursor_line_text") or "").strip()
    if not final_text:
        raise AssertionError(
            f"hot switch back returned a blank terminal surface: {final_host!r}"
        )
    final_notifications = visible_notifications(final_state)
    if final_notifications:
        raise AssertionError(
            f"hot switch back left stale notifications visible: {final_notifications!r}"
        )
    final_shot = out_dir / "hot-switch-return.png"
    app_screenshot(pid, final_shot)
    return {
        "partner_path": partner_path,
        "partner_kind": partner.get("kind"),
        "partner_screenshot": str(partner_shot),
        "return_screenshot": str(final_shot),
        "final_text_sample": final_text[-240:],
    }


def assert_live_sessions_restore_visibility(pid: int) -> dict:
    snapshot = server_snapshot()
    expected_live_paths = [
        normalize_live_path(str(entry.get("session_path") or ""))
        for entry in (snapshot.get("live_sessions") or [])
        if str(entry.get("session_path") or "").strip()
    ]
    rows = app_rows(pid)
    row_paths = {str(row.get("full_path") or "") for row in rows}
    live_group_present = "__live_sessions__" in row_paths

    if expected_live_paths and not live_group_present:
        raise AssertionError(
            f"server snapshot restored live sessions but the sidebar omitted Live Sessions: {expected_live_paths!r}"
        )

    missing_paths = [path for path in expected_live_paths if path not in row_paths]
    if missing_paths:
        raise AssertionError(
            f"restored live sessions are missing from app rows: missing={missing_paths!r} rows={sorted(row_paths)!r}"
        )

    return {
        "expected_live_session_count": len(expected_live_paths),
        "live_group_present": live_group_present,
        "visible_live_session_count": len(
            [path for path in expected_live_paths if path in row_paths]
        ),
    }


def assert_local_session_runtime_ready(session: str) -> dict:
    snapshot = server_snapshot()
    session_entry = find_snapshot_session(snapshot, session)
    if session_entry is None:
        raise AssertionError(f"session is missing from server snapshot: {session}")
    launch_phase = str(session_entry.get("launch_phase") or "")
    status_line = str(session_entry.get("status_line") or "")
    lowered_status = status_line.lower()
    if launch_phase != "Running":
        raise AssertionError(
            f"local session is not running yet: session={session!r} launch_phase={launch_phase!r}"
        )
    if "planned" in lowered_status or "waiting for terminal host" in lowered_status:
        raise AssertionError(
            f"local session runtime is not ready: session={session!r} status_line={status_line!r}"
        )
    terminal_process_id = session_entry.get("terminal_process_id")
    terminal_env = {}
    if terminal_process_id:
        environ_path = Path(f"/proc/{int(terminal_process_id)}/environ")
        if environ_path.exists():
            for item in environ_path.read_bytes().split(b"\0"):
                if not item or b"=" not in item:
                    continue
                key, value = item.split(b"=", 1)
                terminal_env[key.decode("utf-8", "replace")] = value.decode("utf-8", "replace")
    if terminal_env.get("NO_COLOR") is not None:
        raise AssertionError(
            "local PTY child leaked NO_COLOR despite Yggterm advertising a color terminal: "
            f"pid={terminal_process_id} NO_COLOR={terminal_env.get('NO_COLOR')!r}"
        )
    return {
        "launch_phase": launch_phase,
        "status_line": status_line,
        "source": session_entry.get("source"),
        "session_id": session_entry.get("id"),
        "terminal_process_id": terminal_process_id,
        "term": terminal_env.get("TERM"),
        "colorterm": terminal_env.get("COLORTERM"),
        "term_program": terminal_env.get("TERM_PROGRAM"),
        "term_program_version": terminal_env.get("TERM_PROGRAM_VERSION"),
        "colorfgbg": terminal_env.get("COLORFGBG"),
        "no_color": terminal_env.get("NO_COLOR"),
    }


def cursor_sample_is_visibly_active(host: dict) -> bool:
    rect = host.get("cursor_sample_rect") or {}
    if not rect_is_visible(rect):
        return False
    opacity = host.get("cursor_sample_opacity")
    if is_effectively_hidden_css_opacity(opacity):
        return False
    color = host.get("cursor_sample_color")
    background = host.get("cursor_sample_background")
    border_left = host.get("cursor_sample_border_left")
    border_bottom = host.get("cursor_sample_border_bottom")
    outline_style = str(host.get("cursor_sample_outline_style") or "").strip().lower()
    box_shadow = str(host.get("cursor_sample_box_shadow") or "").strip().lower()
    if (
        is_transparent_css_color(color)
        and is_transparent_css_color(background)
        and is_transparent_css_color(border_left)
        and is_transparent_css_color(border_bottom)
        and outline_style in ("", "none")
        and box_shadow in ("", "none")
    ):
        return False
    return True


def assert_cursor_glyph_visibility(state: dict) -> dict:
    host = active_host(state)
    cursor_text = str(host.get("cursor_sample_text") or "")
    cursor_color = str(host.get("cursor_sample_color") or "")
    cursor_background = str(host.get("cursor_sample_background") or "")
    cursor_border_left = str(host.get("cursor_sample_border_left") or "")
    cursor_class_name = str(host.get("cursor_sample_class_name") or "")
    row_background = str(
        host.get("cursor_row_background")
        or host.get("viewport_background_color")
        or host.get("xterm_theme_background")
        or ""
    )
    node_rects = host.get("cursor_node_rects") or []
    active_node = node_rects[0] if isinstance(node_rects, list) and node_rects else {}
    visibility = str(active_node.get("visibility") or "").strip().lower()
    opacity = active_node.get("opacity")
    glyph_contrast = contrast_ratio(cursor_color, row_background) if cursor_text.strip() else None
    background_rgb = parse_css_rgb(row_background)
    minimum_visible_contrast = (
        6.5
        if background_rgb is not None and relative_luminance(background_rgb) > 0.72
        else 4.5
    )
    hidden_raw_cursor = "yggterm-hidden-raw-cursor" in cursor_class_name
    bar_cursor = "xterm-cursor-bar" in cursor_class_name
    if cursor_text.strip() and (bar_cursor or hidden_raw_cursor):
        if visibility == "hidden" or is_effectively_hidden_css_opacity(opacity):
            raise AssertionError(
                f"bar cursor glyph node is geometrically hidden instead of paint-hidden: {active_node!r}"
            )
        if not is_transparent_css_color(cursor_color):
            raise AssertionError(
                f"bar cursor leaked visible glyph color {cursor_color!r} for text {cursor_text!r}"
            )
        if not is_transparent_css_color(cursor_background):
            raise AssertionError(
                f"bar cursor leaked glyph background {cursor_background!r} for text {cursor_text!r}"
            )
        if not is_transparent_css_color(cursor_border_left):
            raise AssertionError(
                f"bar cursor leaked border paint {cursor_border_left!r} for text {cursor_text!r}"
            )
    elif cursor_text.strip():
        if visibility == "hidden":
            raise AssertionError(
                f"cursor glyph node is hidden while showing text {cursor_text!r}: {active_node!r}"
            )
        if is_effectively_hidden_css_opacity(opacity):
            raise AssertionError(
                f"cursor glyph node opacity hides visible text {cursor_text!r}: {active_node!r}"
            )
        if is_transparent_css_color(cursor_color):
            raise AssertionError(
                f"cursor glyph color is transparent for visible text {cursor_text!r}: {cursor_color!r}"
            )
        if glyph_contrast is None or glyph_contrast < minimum_visible_contrast:
            raise AssertionError(
                f"cursor glyph contrast too low: text={cursor_text!r} color={cursor_color!r} background={row_background!r} contrast={glyph_contrast!r}"
            )
    return {
        "cursor_sample_text": cursor_text,
        "cursor_sample_color": cursor_color,
        "cursor_sample_background": cursor_background,
        "cursor_sample_border_left": cursor_border_left,
        "cursor_sample_class_name": cursor_class_name,
        "cursor_glyph_visibility": visibility,
        "cursor_glyph_opacity": opacity,
        "cursor_glyph_contrast": round(glyph_contrast, 2) if glyph_contrast is not None else None,
    }


def assert_cursor_alignment(state: dict) -> dict:
    host = active_host(state)
    expected_rect = host.get("cursor_expected_rect") or {}
    cursor_rect = host.get("cursor_sample_rect") or {}
    if not rect_is_visible(expected_rect):
        raise AssertionError(
            f"expected cursor cell rect is missing/empty, cannot prove alignment: {expected_rect!r}"
        )
    if not cursor_sample_is_visibly_active(host):
        raise AssertionError(
            f"no visible native cursor rect: raw={cursor_rect!r} hidden={host.get('xterm_cursor_hidden')!r}"
        )
    dx = abs(float(cursor_rect["left"]) - float(expected_rect["left"]))
    dy = abs(float(cursor_rect["top"]) - float(expected_rect["top"]))
    dw = abs(float(cursor_rect["width"]) - float(expected_rect["width"]))
    dh = abs(float(cursor_rect["height"]) - float(expected_rect["height"]))
    if dx > 4.0 or dy > 4.0 or dw > 8.0 or dh > 8.0:
        raise AssertionError(
            "native cursor drifted from expected cursor cell: "
            f"cursor={cursor_rect!r} expected={expected_rect!r} dx={dx:.2f} dy={dy:.2f} dw={dw:.2f} dh={dh:.2f}"
        )
    return {
        "cursor_expected_rect": expected_rect,
        "cursor_sample_rect": cursor_rect,
        "active_cursor_rect": cursor_rect,
        "using_overlay": False,
        "cursor_dx": round(dx, 2),
        "cursor_dy": round(dy, 2),
    }


def assert_selection(pid: int, session: str) -> dict:
    select = probe_select(pid, session)
    selected_len = int(select.get("selected_text_length") or 0)
    selected_contrast = select.get("selected_contrast")
    if selected_len <= 0:
        raise AssertionError(f"selection probe did not capture visible text: {select!r}")
    if selected_contrast is not None and float(selected_contrast) < 4.5:
        raise AssertionError(f"selected text contrast too low: {selected_contrast!r}")
    return {
        "selected_text_length": selected_len,
        "selected_contrast": selected_contrast,
        "selected_excerpt": select.get("selected_excerpt"),
    }


def clear_terminal_selection(pid: int, session: str, timeout_seconds: float = 4.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        last_state = wait_for_session_focus(pid, session, timeout_seconds=4.0)
        host = host_for_session(last_state, session)
        if int(host.get("selection_range_count") or 0) == 0 and not str(host.get("selection_text") or ""):
            return last_state
        host_rect = host.get("host_rect") or {}
        if rect_is_visible(host_rect):
            xdotool_click_window(
                pid,
                rect_center_x(host_rect),
                rect_center_y(host_rect),
            )
            time.sleep(0.15)
        else:
            try:
                xdotool_key_window(pid, "Escape")
            except Exception:
                pass
            time.sleep(0.15)
    raise AssertionError(f"terminal selection did not clear: {last_state!r}")


def assert_scroll(pid: int, session: str, before_state: dict) -> dict:
    host = active_host(before_state)
    before_viewport_y = int(host.get("viewport_y") or 0)
    base_y = int(host.get("base_y") or 0)
    buffer_kind = str(host.get("xterm_buffer_kind") or "")
    mouse_tracking_mode = str(host.get("xterm_mouse_tracking_mode") or "")
    alternate_mouse_owned = buffer_kind == "alternate" and mouse_tracking_mode not in ("", "none", "None")
    if base_y <= before_viewport_y <= 0:
        if not alternate_mouse_owned:
            return {
                "lines": 0,
                "before": {
                    "base_y": base_y,
                    "viewport_y": before_viewport_y,
                    "viewport_scroll_top": host.get("viewport_scroll_top"),
                },
                "after": {
                    "base_y": base_y,
                    "viewport_y": before_viewport_y,
                    "viewport_scroll_top": host.get("viewport_scroll_top"),
                },
                "reason": "no_scrollback_available",
            }

    def moved(before: dict, after: dict) -> bool:
        return (
            before.get("viewport_y") != after.get("viewport_y")
            or before.get("viewport_scroll_top") != after.get("viewport_scroll_top")
            or before.get("text_head") != after.get("text_head")
            or before.get("text_tail") != after.get("text_tail")
        )

    def focused(snapshot: dict) -> bool:
        return (
            snapshot.get("input_enabled") is True
            and snapshot.get("helper_textarea_focused") is True
            and snapshot.get("host_has_active_element") is True
        )

    if not focused(host):
        probe_type(pid, session, "", mode="keyboard")

    lines = -5 if before_viewport_y > 0 else 5 if base_y > before_viewport_y else -5
    first = probe_scroll(pid, session, lines)
    before = first.get("before") or {}
    after = first.get("after") or {}
    if after.get("input_enabled") is not True or after.get("helper_textarea_focused") is not True:
        raise AssertionError(
            f"scroll probe lost terminal input/focus: first={first!r}"
        )
    if (not focused(before)) and focused(after) and not moved(before, after):
        first = probe_scroll(pid, session, lines)
        before = first.get("before") or {}
        after = first.get("after") or {}
    if alternate_mouse_owned:
        wheel_delta = int(after.get("wheel_event_count") or 0) - int(before.get("wheel_event_count") or 0)
        data_delta = int(after.get("data_event_count") or 0) - int(before.get("data_event_count") or 0)
        scroll_delta = int(after.get("scroll_event_count") or 0) - int(before.get("scroll_event_count") or 0)
        visual_change = (
            before.get("text_head") != after.get("text_head")
            or before.get("text_tail") != after.get("text_tail")
        )
        if wheel_delta <= 0 and data_delta <= 0 and scroll_delta <= 0 and not visual_change:
            second = probe_scroll(pid, session, lines)
            before = second.get("before") or {}
            after = second.get("after") or {}
            wheel_delta = int(after.get("wheel_event_count") or 0) - int(before.get("wheel_event_count") or 0)
            data_delta = int(after.get("data_event_count") or 0) - int(before.get("data_event_count") or 0)
            scroll_delta = int(after.get("scroll_event_count") or 0) - int(before.get("scroll_event_count") or 0)
            visual_change = (
                before.get("text_head") != after.get("text_head")
                or before.get("text_tail") != after.get("text_tail")
            )
            if wheel_delta <= 0 and data_delta <= 0 and scroll_delta <= 0 and not visual_change:
                raise AssertionError(
                    f"alternate-buffer wheel probe did not reach the app surface: first={first!r} second={second!r}"
                )
            first = second
            before = second.get("before") or {}
            after = second.get("after") or {}
        return {
            "lines": lines,
            "before": before,
            "after": after,
            "alternate_mouse_owned": True,
            "wheel_delta": int(after.get("wheel_event_count") or 0) - int(before.get("wheel_event_count") or 0),
            "data_delta": int(after.get("data_event_count") or 0) - int(before.get("data_event_count") or 0),
            "scroll_delta": int(after.get("scroll_event_count") or 0) - int(before.get("scroll_event_count") or 0),
            "visual_change": (
                before.get("text_head") != after.get("text_head")
                or before.get("text_tail") != after.get("text_tail")
            ),
        }
    if not moved(before, after):
        second = probe_scroll(pid, session, -lines)
        before = second.get("before") or {}
        after = second.get("after") or {}
        if after.get("input_enabled") is not True or after.get("helper_textarea_focused") is not True:
            raise AssertionError(
                f"scroll probe lost terminal input/focus on reverse attempt: second={second!r}"
            )
        if not moved(before, after):
            raise AssertionError(
                f"scroll probe did not move viewport in either direction: first={first!r} second={second!r}"
            )
        return {
            "lines": -lines,
            "before": before,
            "after": after,
        }
    return {
        "lines": lines,
        "before": before,
        "after": after,
    }


def assert_cursor_prompt_visibility(state: dict, *, context: str) -> dict:
    host = active_host(state)
    cursor_rect = host.get("cursor_sample_rect") or {}
    cursor_row_rect = host.get("cursor_row_rect") or {}
    host_rect = host.get("host_rect") or {}
    cursor_line_text = str(host.get("cursor_line_text") or host.get("cursor_row_text") or "")
    active_cursor_rect = cursor_rect
    if not cursor_sample_is_visibly_active(host):
        raise AssertionError(
            f"{context}: native cursor rect missing: raw={cursor_rect!r} hidden={host.get('xterm_cursor_hidden')!r}"
        )
    if not cursor_line_text.strip():
        raise AssertionError(f"{context}: cursor line text is empty")
    row_color = str(host.get("cursor_row_color") or host.get("rows_color") or "")
    row_background = str(
        host.get("cursor_row_background")
        or host.get("viewport_background_color")
        or host.get("xterm_theme_background")
        or ""
    )
    row_contrast = contrast_ratio(row_color, row_background)
    if row_contrast is None or row_contrast < 7.0:
        raise AssertionError(
            f"{context}: cursor row contrast too low: color={row_color!r} background={row_background!r} contrast={row_contrast!r}"
        )
    if rect_is_visible(host_rect) and rect_is_visible(cursor_row_rect):
        host_left = float(host_rect.get("left") or 0)
        host_top = float(host_rect.get("top") or 0)
        host_right = host_left + float(host_rect.get("width") or 0)
        host_bottom = host_top + float(host_rect.get("height") or 0)
        row_left = float(cursor_row_rect.get("left") or 0)
        row_top = float(cursor_row_rect.get("top") or 0)
        row_right = row_left + float(cursor_row_rect.get("width") or 0)
        row_bottom = row_top + float(cursor_row_rect.get("height") or 0)
        if row_left < host_left - 4.0 or row_right > host_right + 4.0 or row_top < host_top - 4.0 or row_bottom > host_bottom + 4.0:
            raise AssertionError(
                f"{context}: cursor row rect drifted outside the host viewport: row={cursor_row_rect!r} host={host_rect!r}"
            )
    if rect_is_visible(cursor_row_rect) and rect_is_visible(active_cursor_rect):
        row_left = float(cursor_row_rect.get("left") or 0)
        row_top = float(cursor_row_rect.get("top") or 0)
        row_right = row_left + float(cursor_row_rect.get("width") or 0)
        row_bottom = row_top + float(cursor_row_rect.get("height") or 0)
        cursor_left = float(active_cursor_rect.get("left") or 0)
        cursor_top = float(active_cursor_rect.get("top") or 0)
        cursor_right = cursor_left + float(active_cursor_rect.get("width") or 0)
        cursor_bottom = cursor_top + float(active_cursor_rect.get("height") or 0)
        if cursor_left < row_left - 4.0 or cursor_right > row_right + 4.0 or cursor_top < row_top - 4.0 or cursor_bottom > row_bottom + 4.0:
            raise AssertionError(
                f"{context}: active cursor rect drifted outside the cursor row rect: cursor={active_cursor_rect!r} row={cursor_row_rect!r}"
            )
    return {
        "cursor_line_text": cursor_line_text,
        "active_cursor_rect": active_cursor_rect,
        "cursor_row_rect": cursor_row_rect,
        "cursor_row_contrast": round(row_contrast, 2),
        "cursor_visible_row_index": host.get("cursor_visible_row_index"),
        "blank_rows_below_cursor": host.get("blank_rows_below_cursor"),
    }


def type_with_cursor_artifact_checks(
    pid: int,
    session: str,
    text: str,
    out_dir: Path,
    *,
    prefix: str,
    chunk_size: int = 2,
) -> dict:
    steps: list[dict] = []
    typed_so_far = ""
    for index in range(0, len(text), chunk_size):
        chunk = text[index : index + chunk_size]
        typed_so_far += chunk
        probe = probe_type(pid, session, chunk, mode="keyboard")
        time.sleep(0.25)
        state = wait_for_terminal_quiescent(pid, timeout_seconds=8.0)
        host = active_host(state)
        screenshot_path = out_dir / f"{prefix}-step-{(index // chunk_size) + 1:02d}.png"
        state_path = out_dir / f"{prefix}-step-{(index // chunk_size) + 1:02d}-state.json"
        app_screenshot(pid, screenshot_path)
        with state_path.open("w") as fh:
            json.dump(state, fh, indent=2)
        cursor_line_text = str(host.get("cursor_line_text") or host.get("cursor_row_text") or "")
        if typed_so_far not in cursor_line_text and typed_so_far not in str(host.get("text_sample") or ""):
            raise AssertionError(
                f"{prefix}: typed text is not visible after chunk {chunk!r}: expected={typed_so_far!r} cursor_line={cursor_line_text!r}"
            )
        prompt_anchor = assert_cursor_prompt_visibility(
            state, context=f"{prefix} after typing {typed_so_far!r}"
        )
        assert_cursor_alignment(state)
        pixel_probe = assert_cursor_neighbor_pixels_clean(
            screenshot_path,
            state,
            context=f"{prefix} after typing {typed_so_far!r}",
        )
        prompt_pixels = assert_prompt_prefix_pixels_visible(
            screenshot_path,
            state,
            context=f"{prefix} after typing {typed_so_far!r}",
        )
        steps.append(
            {
                "typed": typed_so_far,
                "chunk": chunk,
                "probe": probe,
                "screenshot": str(screenshot_path),
                "state": str(state_path),
                "prompt_anchor": prompt_anchor,
                "pixel_probe": pixel_probe,
                "prompt_pixels": prompt_pixels,
            }
        )
    return {
        "chunk_size": chunk_size,
        "steps": steps,
    }


def assert_partial_input_flow(pid: int, session: str, out_dir: Path) -> dict:
    wait_for_session_focus(pid, session, timeout_seconds=12.0)
    clear_terminal_selection(pid, session, timeout_seconds=4.0)
    wait_for_terminal_quiescent(pid, timeout_seconds=12.0)
    clear = probe_type(
        pid,
        session,
        "",
        mode="keyboard",
        press_ctrl_c=True,
        press_ctrl_e=True,
        press_ctrl_u=True,
    )
    time.sleep(0.3)
    wait_for_terminal_quiescent(pid, timeout_seconds=8.0)
    chunked_typing = type_with_cursor_artifact_checks(
        pid,
        session,
        "/sta",
        out_dir,
        prefix="partial-type",
    )
    typed_state = wait_for_terminal_quiescent(pid, timeout_seconds=8.0)
    typed_host = active_host(typed_state)
    typed_shot = out_dir / "after-partial-type.png"
    app_screenshot(pid, typed_shot)
    with (out_dir / "after-partial-type-state.json").open("w") as fh:
        json.dump(typed_state, fh, indent=2)
    cursor_line_text = str(typed_host.get("cursor_line_text") or typed_host.get("cursor_row_text") or "")
    if "/sta" not in cursor_line_text and "/sta" not in str(typed_host.get("text_sample") or ""):
        raise AssertionError(
            f"partial typed text is not visible on the prompt line: cursor_line={cursor_line_text!r}"
        )
    low_contrast_cursor_spans = [
        sample
        for sample in (typed_host.get("cursor_row_span_samples") or [])
        if isinstance(sample, dict)
        and sample.get("text")
        and sample.get("contrast") is not None
        and float(sample["contrast"]) < 6.5
    ]
    if low_contrast_cursor_spans:
        raise AssertionError(
            f"cursor row still has low-contrast spans: {low_contrast_cursor_spans!r}"
        )
    typed_anchor = assert_cursor_prompt_visibility(typed_state, context="after partial typing")
    assert_cursor_alignment(typed_state)

    scroll_probe = probe_scroll(pid, session, -5)
    time.sleep(0.4)
    scroll_state = app_state(pid)
    scroll_host = active_host(scroll_state)
    scroll_shot = out_dir / "after-partial-scroll.png"
    app_screenshot(pid, scroll_shot)
    with (out_dir / "after-partial-scroll-state.json").open("w") as fh:
        json.dump(scroll_state, fh, indent=2)
    if scroll_host.get("input_enabled") is not True or scroll_host.get("helper_textarea_focused") is not True:
        raise AssertionError("after partial scroll the terminal lost focused input")
    before_scroll = scroll_probe.get("before") or {}
    after_scroll = scroll_probe.get("after") or {}
    viewport_moved = (
        before_scroll.get("viewport_y") != after_scroll.get("viewport_y")
        or before_scroll.get("text_tail") != after_scroll.get("text_tail")
    )
    scroll_anchor = None
    if not viewport_moved:
        scroll_anchor = assert_cursor_prompt_visibility(scroll_state, context="after partial scroll")
        assert_cursor_alignment(scroll_state)

    return {
        "clear_probe": clear,
        "typed_probe": chunked_typing,
        "typed_screenshot": str(typed_shot),
        "typed_cursor_line_text": cursor_line_text,
        "typed_anchor": typed_anchor,
        "scroll_probe": scroll_probe,
        "scroll_screenshot": str(scroll_shot),
        "scroll_anchor": scroll_anchor,
        "scroll_viewport_moved": viewport_moved,
    }


def wait_for_status_panel(pid: int, timeout_seconds: float = 12.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    markers = (
        "OpenAI Codex",
        "Session:",
        "Collaboration mode:",
        "Weekly limit:",
    )
    transcript_only_markers = (
        "To continue this session, run codex resume",
        "codex resume ",
    )
    while time.time() < deadline:
        last_state = app_state(pid)
        host = active_host(last_state)
        if host_has_shell_status_failure(host):
            raise AssertionError(
                "Codex status probe fell back to the shell and ran /status there instead of inside a live Codex runtime"
            )
        text_sample = str(host.get("text_sample") or "")
        cursor_line_text = str(host.get("cursor_line_text") or "")
        haystack = text_sample + "\n" + cursor_line_text
        if any(marker in haystack for marker in markers):
            return last_state
        if any(marker in haystack for marker in transcript_only_markers):
            raise AssertionError(
                "Codex status panel check requires a live Codex runtime, but the active host is a stored transcript with a codex resume footer"
            )
        time.sleep(0.25)
    raise AssertionError(f"Codex status panel did not become visible in time: {last_state!r}")


def wait_for_live_codex_prompt(pid: int, timeout_seconds: float = 20.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        last_state = wait_for_interactive(pid, timeout_seconds=8.0)
        host = active_host(last_state)
        if host_has_live_codex_prompt(host):
            return last_state
        time.sleep(0.25)
    raise AssertionError(f"live Codex prompt did not become visible in time: {last_state!r}")


def ensure_live_codex_runtime(pid: int, session: str) -> dict:
    current = wait_for_interactive(pid, timeout_seconds=20.0)
    host = active_host(current)
    if host_has_live_codex_prompt(host):
        return {
            "action": "noop",
            "state": current,
        }
    prepare = probe_type(
        pid,
        session,
        "",
        mode="keyboard",
        press_ctrl_c=True,
        press_ctrl_e=True,
        press_ctrl_u=True,
    )
    time.sleep(0.4)
    launch = probe_type(pid, session, "codex", mode="keyboard", press_enter=True)
    state = wait_for_live_codex_prompt(pid, timeout_seconds=30.0)
    return {
        "action": "launch_codex",
        "prepare_probe": prepare,
        "launch_probe": launch,
        "state": state,
    }


def ensure_plain_shell_runtime(pid: int, session: str) -> dict:
    current = wait_for_interactive(pid, timeout_seconds=20.0)
    host = active_host(current)
    cursor_line_text = str(host.get("cursor_line_text") or host.get("cursor_row_text") or "")
    if (
        host.get("xterm_buffer_kind") == "normal"
        and host.get("xterm_cursor_hidden") is False
        and "$" in cursor_line_text
    ):
        return {
            "action": "noop",
            "state": current,
        }
    prepare = probe_type(
        pid,
        session,
        "q",
        mode="keyboard",
        press_ctrl_c=True,
    )
    time.sleep(0.4)
    state = wait_for_terminal_quiescent(pid, timeout_seconds=20.0)
    return {
        "action": "restore_plain_shell",
        "prepare_probe": prepare,
        "state": state,
    }


def assert_codex_session_tui_vitality(pid: int, session: str, out_dir: Path) -> dict:
    command = resolve_codex_session_tui_command()
    if command is None:
        return {"skipped": "codex-session-tui binary not found"}

    launch = unwrap_data(
        run(
            "server",
            "app",
            "terminal",
            "send",
            "--pid",
            str(pid),
            session,
            "--data",
            f"{command}\r",
            "--timeout-ms",
            "12000",
        )
    )
    if not bool(launch.get("accepted")):
        raise AssertionError(f"codex-session-tui launch rejected: {launch!r}")

    deadline = time.time() + 20.0
    last_state = {}
    while time.time() < deadline:
        state = app_state(pid)
        host = host_for_session_or_none(state, session)
        if host is None:
            time.sleep(0.25)
            continue
        text_sample = str(host.get("text_sample") or "")
        if (
            host.get("xterm_buffer_kind") == "alternate"
            and host.get("xterm_cursor_hidden") is True
            and "Browser [0 selected" in text_sample
            and (
                "Preview (Chat) No session selected" in text_sample
                or "Status" in text_sample
                or "local [ok]" in text_sample
            )
        ):
            last_state = state
            break
        last_state = state
        time.sleep(0.25)
    else:
        raise AssertionError(
            f"codex-session-tui did not reach the alternate-buffer browser layout: {last_state!r}"
        )

    screenshot_path = out_dir / "codex-session-tui-vitality.png"
    app_screenshot(pid, screenshot_path)
    host = host_for_session(last_state, session)
    host_rect = host.get("host_rect") or {}
    if not rect_is_visible(host_rect):
        raise AssertionError(f"codex-session-tui host rect missing for vitality probe: {host_rect!r}")
    image = Image.open(screenshot_path)
    browser_crop_box = clamp_box(
        (
            int(round(float(host_rect.get("left") or 0))),
            int(round(float(host_rect.get("top") or 0))),
            int(round(float(host_rect.get("left") or 0))) + 170,
            int(round(float(host_rect.get("top") or 0))) + 92,
        ),
        image.size,
    )
    if browser_crop_box is None:
        raise AssertionError("codex-session-tui browser vitality crop fell outside the screenshot")
    browser_crop = image.crop(browser_crop_box)
    dark_pixels = count_dark_foreground_pixels(browser_crop)
    if dark_pixels < 500:
        raise AssertionError(
            f"codex-session-tui browser rows painted too faintly in the screenshot: dark_pixels={dark_pixels} crop={browser_crop_box}"
        )
    colorful_pixels, colorful_hue_buckets = colorful_foreground_stats(browser_crop)
    if colorful_pixels < 32 or colorful_hue_buckets < 2:
        raise AssertionError(
            "codex-session-tui browser crop still looks visually flat in the screenshot: "
            f"colorful_pixels={colorful_pixels} colorful_hue_buckets={colorful_hue_buckets} crop={browser_crop_box}"
        )

    restored = restore_prompt_after_codex_session_tui(
        pid,
        session,
        timeout_seconds=12.0,
    )
    restored_host = host_for_session(restored, session)
    return {
        "command": command,
        "screenshot": str(screenshot_path),
        "browser_crop_box": {
            "left": browser_crop_box[0],
            "top": browser_crop_box[1],
            "width": browser_crop_box[2] - browser_crop_box[0],
            "height": browser_crop_box[3] - browser_crop_box[1],
        },
        "browser_dark_pixels": dark_pixels,
        "browser_colorful_pixels": colorful_pixels,
        "browser_colorful_hue_buckets": colorful_hue_buckets,
        "renderer_mode": host.get("xterm_renderer_mode"),
        "xterm_minimum_contrast_ratio": host.get("xterm_minimum_contrast_ratio"),
        "xterm_font_weight": host.get("xterm_font_weight"),
        "xterm_line_height": host.get("xterm_line_height"),
        "restored_renderer_mode": restored_host.get("xterm_renderer_mode"),
    }


def assert_hidden_cursor_tui(pid: int, session: str, out_dir: Path) -> dict:
    clear = terminal_probe_type(
        pid,
        session,
        "",
        mode="xterm",
        press_ctrl_c=True,
        press_ctrl_e=True,
        press_ctrl_u=True,
    )
    wait_for_terminal_quiescent(pid, timeout_seconds=6.0)
    command = "printf '\\033[?1049h\\033[?25lhc'; sleep 1; printf '\\033[?25h\\033[?1049l'"
    probe = terminal_probe_type(pid, session, command, mode="xterm", press_enter=True)
    deadline = time.time() + 6.0
    state = {}
    host = {}
    while time.time() < deadline:
        state = app_state(pid)
        host = active_host(state)
        text_sample = str(host.get("text_sample") or "")
        cursor_line_text = str(host.get("cursor_line_text") or host.get("cursor_row_text") or "")
        if (
            host.get("xterm_buffer_kind") == "alternate"
            and host.get("xterm_cursor_hidden") is True
            and ("hc" in text_sample or "hc" in cursor_line_text)
        ):
            break
        time.sleep(0.15)
    shot_path = out_dir / "hidden-cursor-tui.png"
    app_screenshot(pid, shot_path)
    with (out_dir / "hidden-cursor-tui-state.json").open("w") as fh:
        json.dump(state, fh, indent=2)
    observed_live = (
        host.get("xterm_buffer_kind") == "alternate"
        and host.get("xterm_cursor_hidden") is True
    )
    if observed_live:
        if cursor_sample_is_visibly_active(host):
            raise AssertionError(
                f"raw cursor node stayed visibly active while xterm reported cursor hidden: {host.get('cursor_node_rects')!r}"
            )
        text_sample = str(host.get("text_sample") or "")
        cursor_line_text = str(host.get("cursor_line_text") or host.get("cursor_row_text") or "")
        if "hc" not in text_sample and "hc" not in cursor_line_text:
            raise AssertionError(
                f"hidden-cursor fixture text is missing from the terminal buffer: text={text_sample!r} cursor={cursor_line_text!r}"
            )
    restored_state = wait_for_terminal_restore(pid, timeout_seconds=8.0)
    restored_host = active_host(restored_state)
    if restored_host.get("xterm_buffer_kind") != "normal":
        raise AssertionError(f"hidden-cursor fixture did not restore the normal buffer: {restored_host!r}")
    if int(restored_host.get("xterm_buffer_transition_count") or 0) < 2:
        raise AssertionError(f"expected alternate-buffer transitions, saw {restored_host!r}")
    if int(restored_host.get("xterm_cursor_hidden_toggle_count") or 0) < 2:
        raise AssertionError(f"expected hidden-cursor toggles, saw {restored_host!r}")
    assert_cursor_alignment(restored_state)
    return {
        "clear_probe": clear,
        "probe": probe,
        "screenshot": str(shot_path),
        "observed_live_alternate_buffer": observed_live,
        "buffer_kind": host.get("xterm_buffer_kind"),
        "cursor_hidden": host.get("xterm_cursor_hidden"),
        "renderer_mode": host.get("xterm_renderer_mode"),
        "restored_buffer_kind": restored_host.get("xterm_buffer_kind"),
        "buffer_transition_count": restored_host.get("xterm_buffer_transition_count"),
        "cursor_hidden_toggle_count": restored_host.get("xterm_cursor_hidden_toggle_count"),
        "raw_cursor_hidden_count": restored_host.get("raw_cursor_hidden_count"),
    }


def assert_status_command(pid: int, session: str, out_dir: Path) -> dict:
    ensure = ensure_live_codex_runtime(pid, session)
    wait_for_session_focus(pid, session, timeout_seconds=12.0)
    clear_terminal_selection(pid, session, timeout_seconds=4.0)
    clear = probe_type(
        pid,
        session,
        "",
        mode="keyboard",
        press_ctrl_e=True,
        press_ctrl_u=True,
    )
    time.sleep(0.3)
    typed_probe = type_with_cursor_artifact_checks(
        pid,
        session,
        "/status",
        out_dir,
        prefix="status-type",
    )
    probe = probe_type(pid, session, "", mode="keyboard", press_enter=True)
    state = wait_for_status_panel(pid, timeout_seconds=12.0)
    try:
        state = wait_for_terminal_quiescent(pid, timeout_seconds=6.0)
        settled = "quiescent"
    except AssertionError:
        state = wait_for_interactive(pid, timeout_seconds=6.0)
        settled = "interactive_only"
    shot_path = out_dir / "after-status.png"
    app_screenshot(pid, shot_path)
    host = active_host(state)
    text_sample = str(host.get("text_sample") or "")
    cursor_line_text = str(host.get("cursor_line_text") or "")
    if host_has_shell_status_failure(host):
        raise AssertionError("Codex status probe typed /status into the shell instead of the live Codex runtime")
    if "OpenAI Codex" not in text_sample and "Session:" not in text_sample:
        raise AssertionError("Codex status panel is not visible after /status<Enter>")
    assert_cursor_alignment(state)
    cursor_glyph = assert_cursor_glyph_visibility(state)
    prompt_anchor = assert_cursor_prompt_visibility(state, context="after /status")
    return {
        "ensure_live_codex": ensure,
        "clear_probe": clear,
        "typed_probe": typed_probe,
        "probe": probe,
        "settled": settled,
        "screenshot": str(shot_path),
        "cursor_line_text": cursor_line_text,
        "cursor_glyph": cursor_glyph,
        "prompt_anchor": prompt_anchor,
        "text_tail": text_sample[-400:],
    }


def assert_theme_contract(pid: int, out_dir: Path) -> dict:
    results: dict[str, dict] = {}
    for theme in ("dark", "light"):
        app_theme(pid, theme)
        time.sleep(1.25)
        state = wait_for_interactive(pid, timeout_seconds=20.0)
        host = active_host(state)
        expected_bg = "#1e1e1e" if theme == "dark" else "#fbfbfd"
        if (state.get("settings") or {}).get("theme") != theme:
            raise AssertionError(f"UI theme did not switch to {theme!r}: {state.get('settings')!r}")
        if host.get("xterm_theme_background") != expected_bg:
            raise AssertionError(
                f"xterm background did not track {theme} mode: {host.get('xterm_theme_background')!r}"
            )
        assert_text_readability(state)
        assert_cursor_alignment(state)
        shot_path = out_dir / f"theme-{theme}.png"
        app_screenshot(pid, shot_path)
        results[theme] = {
            "background": host.get("xterm_theme_background"),
            "rows_sample_color": host.get("rows_sample_color"),
            "dim_sample_color": host.get("dim_sample_color"),
            "screenshot": str(shot_path),
        }
    return results


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--session", required=True)
    parser.add_argument("--session-kind", choices=("codex", "plain"), default="codex")
    parser.add_argument("--out", default="/tmp/xterm-embed-faults")
    parser.add_argument("--reopen", action="store_true")
    parser.add_argument("--home")
    args = parser.parse_args()

    if args.home:
        ENV["YGGTERM_HOME"] = str(Path(args.home).expanduser())

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    initial_state = app_state(args.pid)
    if args.reopen or (
        initial_state.get("active_session_path") != args.session
        or initial_state.get("active_view_mode") != "Terminal"
    ):
        app_open(args.pid, args.session, view="terminal")
    state = wait_for_interactive(args.pid, timeout_seconds=25.0)
    initial_settle = "interactive_only"
    if args.session_kind == "plain":
        state = ensure_plain_shell_runtime(args.pid, args.session)["state"]
        initial_settle = "quiescent"
    elif args.session_kind == "codex":
        state = ensure_live_codex_runtime(args.pid, args.session)["state"]
    state = wait_for_session_focus(args.pid, args.session, timeout_seconds=12.0)
    app_screenshot(args.pid, out_dir / "initial.png")
    initial_prompt_pixels = assert_prompt_prefix_pixels_visible(
        out_dir / "initial.png",
        state,
        context="initial screenshot",
    )
    with (out_dir / "initial-state.json").open("w") as fh:
        json.dump(state, fh, indent=2)

    summary = {
        "pid": args.pid,
        "session": args.session,
        "session_kind": args.session_kind,
        "initial_settle": initial_settle,
        "checks": {},
    }

    summary["checks"]["initial_prompt_pixels"] = initial_prompt_pixels
    summary["checks"]["focus"] = assert_focus_and_visibility(state)
    summary["checks"]["geometry"] = assert_geometry(state)
    summary["checks"]["titlebar_centering"] = assert_titlebar_centering(state)
    summary["checks"]["terminal_viewport_inset"] = assert_terminal_viewport_inset(state)
    summary["checks"]["sidebar_resize"] = assert_sidebar_resize_persists_to_settings(args.pid)
    summary["checks"]["search_focus_overlay"] = assert_search_focus_overlay_contract(args.pid, args.session)
    summary["checks"]["titlebar_new_menu_shell"] = assert_titlebar_new_menu_shell_contract(args.pid)
    summary["checks"]["titlebar_session_shell"] = assert_titlebar_session_shell_contract(args.pid)
    summary["checks"]["titlebar_modal_visual_parity"] = assert_titlebar_modal_visual_parity(args.pid)
    summary["checks"]["context_menu_rename_session"] = assert_context_menu_rename_session(args.pid)
    summary["checks"]["titlebar_overflow_menu"] = assert_titlebar_overflow_menu_contract(args.pid)
    summary["checks"]["settings_field_focus"] = assert_settings_field_accepts_text_in_terminal_mode(args.pid)
    summary["checks"]["theme_editor_contract"] = assert_theme_editor_contract(args.pid, out_dir)
    summary["checks"]["maximize_roundtrip_layout"] = assert_maximize_roundtrip_layout(args.pid)
    summary["checks"]["client_memory_budget"] = assert_client_memory_budget(args.pid)
    summary["checks"]["renderer"] = assert_renderer_contract(state)
    summary["checks"]["readability"] = assert_text_readability(state)
    summary["checks"]["cursor"] = assert_cursor_alignment(state)
    summary["checks"]["cursor_glyph"] = assert_cursor_glyph_visibility(state)
    summary["checks"]["live_sessions_restore"] = assert_live_sessions_restore_visibility(args.pid)
    summary["checks"]["selection"] = assert_selection(args.pid, args.session)
    if args.session_kind == "plain":
        summary["checks"]["partial_input"] = assert_partial_input_flow(args.pid, args.session, out_dir)
        summary["checks"]["scroll"] = assert_scroll(args.pid, args.session, state)
        summary["checks"]["hidden_cursor_tui"] = assert_hidden_cursor_tui(args.pid, args.session, out_dir)
        if args.session.startswith("local://"):
            summary["checks"]["codex_session_tui_vitality"] = assert_codex_session_tui_vitality(
                args.pid, args.session, out_dir
            )
    if args.session_kind == "codex":
        summary["checks"]["status_command"] = assert_status_command(args.pid, args.session, out_dir)
    if args.session.startswith("local://"):
        summary["checks"]["local_tree"] = assert_local_tree_placement(args.pid, args.session)
        summary["checks"]["sidebar_contract"] = assert_sidebar_contract(args.pid, args.session)
        summary["checks"]["local_runtime"] = assert_local_session_runtime_ready(args.session)
        summary["checks"]["hot_session_switch"] = assert_hot_session_switch(
            args.pid, args.session, args.session_kind, out_dir
        )
    summary["checks"]["observability_budget"] = assert_observability_budget()
    summary["checks"]["themes"] = assert_theme_contract(args.pid, out_dir)
    summary["checks"]["idle_root_render_budget"] = assert_idle_root_render_budget(args.pid)
    final_state = app_state(args.pid)
    with (out_dir / "final-state.json").open("w") as fh:
        json.dump(final_state, fh, indent=2)
    app_screenshot(args.pid, out_dir / "final.png")

    with (out_dir / "summary.json").open("w") as fh:
        json.dump(summary, fh, indent=2)
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
