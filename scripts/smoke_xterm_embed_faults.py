#!/usr/bin/env python3
import argparse
import base64
import colorsys
import io
import json
import os
import re
import shutil
import subprocess
import time
from pathlib import Path

from PIL import Image, ImageStat


ROOT = Path(__file__).resolve().parents[1]
BIN = Path(os.environ.get("YGGTERM_BIN") or (ROOT / "target" / "debug" / "yggterm"))
ENV = os.environ.copy()
XDO_TIMEOUT_SECONDS = 4.0
PERF_TELEMETRY_MAX_BYTES = 16 * 1024 * 1024
UI_TELEMETRY_MAX_BYTES = 8 * 1024 * 1024
TELEMETRY_BUDGET_SLACK_BYTES = 512 * 1024
IDLE_ROOT_RENDER_SAMPLE_SECONDS = 1.25
IDLE_ROOT_RENDER_MAX_DELTA = 8
IDLE_HOST_RENDER_MAX_DELTA = 30
# The DOM xterm path can legitimately emit a few extra paints beyond terminal I/O dispatches
# for cursor/viewport reconciliation without representing semantic churn.
IDLE_HOST_RENDER_IO_OVERHEAD_MAX = 9
# Two retained hosts can legitimately idle a little above the older 18-dispatch
# two-host budget without semantic churn, especially after late-session view toggles.
IDLE_TERMINAL_IO_MAX_DELTA = 44
IDLE_TERMINAL_IO_BUSY_MAX_DELTA = 56
IDLE_TERMINAL_IO_BASELINE_HOST_COUNT = 4
TERMINAL_INTERACTION_LATENCY_MAX_MS = 3000.0
SIDEBAR_MIN_WIDTH = 220.0
SIDEBAR_MAX_WIDTH = 420.0
TITLEBAR_AUTOHIDE_SENSOR_HEIGHT_MAX_PX = 8.5
TITLEBAR_VISIBLE_MIN_HEIGHT_PX = 28.0
TITLEBAR_EMPTY_LANE_MIN_WIDTH_PX = 18.0
TITLEBAR_PLUS_SESSION_GAP_MIN_PX = 4.0
TITLEBAR_PLUS_SESSION_GAP_MAX_PX = 18.0
TITLEBAR_AUTOHIDE_CONTENT_BALANCE_MAX_PX = 1.5
RIGHT_PANEL_EXIT_ANIMATION_SAMPLE_SECONDS = 0.09
RIGHT_PANEL_EXIT_ANIMATION_SETTLE_SECONDS = 0.36
WINDOW_SETTLE_MIN_WIDTH_PX = 1100
WINDOW_SETTLE_MIN_HEIGHT_PX = 760
CLIENT_MAIN_RSS_MAX_KB = 512 * 1024
CLIENT_TOTAL_RSS_MAX_KB = 896 * 1024
WEBKIT_WEB_PROCESS_RSS_MAX_KB = 384 * 1024
WEBKIT_CHILD_RSS_GROWTH_MAX_KB = 96 * 1024
WEBKIT_CHILD_RSS_SOAK_CYCLES = 3
WEBKIT_CHILD_RSS_SETTLE_SECONDS = 0.9
LAST_SEARCH_FOCUS_OVERLAY_CONTRACT: dict | None = None
LAST_TITLEBAR_NEW_MENU_SHELL_CONTRACT: dict | None = None
LAST_TITLEBAR_SESSION_SHELL_CONTRACT: dict | None = None
POINTER_DRIVER = (ENV.get("YGGTERM_POINTER_DRIVER") or "xdotool").strip().lower()
KEY_DRIVER = (ENV.get("YGGTERM_KEY_DRIVER") or POINTER_DRIVER or "xdotool").strip().lower()
AVOID_FOREGROUND = (ENV.get("YGGTERM_SMOKE_AVOID_FOREGROUND") or "").strip().lower() in (
    "1",
    "true",
    "yes",
    "on",
)
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


def event_trace_paths() -> list[Path]:
    home = current_yggterm_home()
    return [
        home / "event-trace.jsonl",
        home / "event-trace.previous.jsonl",
    ]


def run(*args: str, check: bool = True, timeout_seconds: float = 20.0) -> dict:
    # App-control subcommands use their own `--timeout-ms` settle deadline internally.
    # Give the outer subprocess a small cushion so we capture the CLI's real result
    # instead of killing it right as it is about to report success or timeout.
    timeout_with_cushion = timeout_seconds + 5.0
    if "--timeout-ms" in args:
        try:
            timeout_arg = args[args.index("--timeout-ms") + 1]
            timeout_with_cushion = max(timeout_with_cushion, float(timeout_arg) / 1000.0 + 5.0)
        except (IndexError, ValueError):
            pass
    try:
        proc = subprocess.run(
            [str(BIN), *args],
            cwd=ROOT,
            text=True,
            capture_output=True,
            env=ENV,
            timeout=timeout_with_cushion,
        )
    except subprocess.TimeoutExpired as exc:
        rendered = " ".join(str(arg) for arg in args)
        raise AssertionError(
            f"command timed out after {timeout_with_cushion:.1f}s: {rendered}"
        ) from exc
    if check and proc.returncode != 0:
        raise AssertionError(
            proc.stderr.strip() or proc.stdout.strip() or f"command failed: {args!r}"
        )
    text = proc.stdout.strip()
    return json.loads(text) if text else {}


def run_timed(*args: str, check: bool = True, timeout_seconds: float = 20.0) -> tuple[dict, float]:
    started = time.perf_counter()
    response = run(*args, check=check, timeout_seconds=timeout_seconds)
    elapsed_ms = (time.perf_counter() - started) * 1000.0
    return response, elapsed_ms


def app_pointer_command(pid: int, action: str, **kwargs) -> dict:
    args = [
        "server",
        "app",
        "pointer",
        action,
        "--pid",
        str(pid),
        "--timeout-ms",
        "8000",
    ]
    for key, value in kwargs.items():
        if value is None:
            continue
        flag = f"--{key.replace('_', '-')}"
        args.extend([flag, str(value)])
    payload = run(*args)
    return (payload.get("data") or payload) if isinstance(payload, dict) else {}


def app_key_command(pid: int, action: str, *keys: str, text: str | None = None) -> dict:
    args = [
        "server",
        "app",
        "key",
        action,
        "--pid",
        str(pid),
        "--timeout-ms",
        "8000",
    ]
    if text is not None:
        args.extend(["--text", text])
    args.extend(keys)
    payload = run(*args)
    return (payload.get("data") or payload) if isinstance(payload, dict) else {}


def app_move_window_by(pid: int, delta_x: float, delta_y: float) -> dict:
    payload = run(
        "server",
        "app",
        "move-window",
        "--pid",
        str(pid),
        "--delta-x",
        str(delta_x),
        "--delta-y",
        str(delta_y),
        "--timeout-ms",
        "8000",
    )
    return (payload.get("data") or payload) if isinstance(payload, dict) else {}


def app_pointer_button_name(button: int) -> str:
    return {
        1: "primary",
        2: "middle",
        3: "secondary",
    }.get(int(button), "primary")


def app_state(pid: int, timeout_ms: int = 20000, retries: int = 2) -> dict:
    last_error = None
    for attempt in range(retries + 1):
        try:
            return run(
                "server",
                "app",
                "state",
                "--pid",
                str(pid),
                "--timeout-ms",
                str(timeout_ms),
            )["data"]
        except AssertionError as exc:
            last_error = exc
            if "timed out waiting for app control response" not in str(exc) or attempt >= retries:
                raise
            time.sleep(0.25 * (attempt + 1))
    assert last_error is not None
    raise last_error


def viewport_state(state: dict) -> dict:
    viewport = state.get("viewport")
    if isinstance(viewport, dict) and viewport:
        return viewport
    return state


def app_focus(pid: int) -> dict:
    if AVOID_FOREGROUND:
        state = app_state(pid)
        return {
            "data": {
                "skipped": True,
                "reason": "avoid_foreground",
                "window": state.get("window") or {},
            }
        }
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


def terminal_paste_image(pid: int, session: str) -> dict:
    return run(
        "server",
        "app",
        "terminal",
        "paste-image",
        "--pid",
        str(pid),
        session,
        "--timeout-ms",
        "15000",
    )


def app_create_terminal(pid: int, *, title: str | None = None, cwd: str | None = None) -> dict:
    args = [
        "server",
        "app",
        "terminal",
        "new",
        "--pid",
        str(pid),
        "--timeout-ms",
        "15000",
    ]
    if title:
        args.extend(["--title", title])
    if cwd:
        args.extend(["--cwd", cwd])
    return unwrap_data(run(*args))


def app_remove_session(pid: int, session_path: str) -> dict:
    return unwrap_data(run(
        "server",
        "app",
        "session",
        "remove",
        session_path,
        "--pid",
        str(pid),
        "--timeout-ms",
        "15000",
    ))


def terminal_reclaim_focus(pid: int, session: str) -> dict:
    return run(
        "server",
        "app",
        "terminal",
        "focus",
        "--pid",
        str(pid),
        session,
        "--timeout-ms",
        "15000",
    )


def terminal_paste_clipboard(pid: int, session: str) -> dict:
    return run(
        "server",
        "app",
        "terminal",
        "paste",
        "--pid",
        str(pid),
        session,
        "--timeout-ms",
        "15000",
    )


def assert_terminal_runtime_writable(pid: int, session: str, *, context: str) -> dict:
    response = terminal_send(pid, session, "")
    data = response.get("data") or {}
    if bool(data.get("accepted")):
        return response
    raise AssertionError(
        f"{context}: terminal runtime is not writable for session {session!r}: {response!r}"
    )


def clear_prompt_line(pid: int, session: str, *, timeout_seconds: float = 8.0) -> dict:
    """
    Reset the active shell prompt line without depending on flaky X11 keyboard
    delivery. Mixed PTY+keyboard cleanup caused delayed control bytes to land
    during later probes, so this helper stays on the PTY path.
    """
    pty_clear = terminal_send(pid, session, "\u0003")
    try:
        state = wait_for_terminal_quiescent(pid, timeout_seconds=timeout_seconds)
    except AssertionError:
        deadline = time.time() + timeout_seconds
        state = {}
        while time.time() < deadline:
            state = app_state(pid)
            viewport = viewport_state(state)
            host = active_host_or_none(state) or {}
            if (
                viewport.get("ready") is True
                and viewport.get("interactive") is True
                and viewport.get("terminal_settled_kind") == "interactive"
                and host.get("input_enabled") is True
                and not ((viewport.get("active_terminal_surface") or {}).get("problem"))
            ):
                break
            time.sleep(0.2)
        else:
            raise
    host = active_host_or_none(state) or {}
    cursor_line = str(host.get("cursor_line_text") or host.get("cursor_row_text") or "")
    if "^C" in cursor_line:
        pty_clear = terminal_send(pid, session, "\u0003")
        state = wait_for_terminal_quiescent(pid, timeout_seconds=timeout_seconds)
    return {
        "pty_clear": pty_clear,
        "state": state,
    }


def focus_terminal_helper_textarea(
    pid: int,
    session: str,
    *,
    timeout_seconds: float = 3.0,
) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        terminal_reclaim_focus(pid, session)
        last_state = app_state(pid)
        viewport = viewport_state(last_state)
        host = host_for_session_or_none(last_state, session)
        if host is None:
            time.sleep(0.12)
            continue
        if (
            viewport.get("ready") is True
            and viewport.get("interactive") is True
            and viewport.get("terminal_settled_kind") == "interactive"
            and host.get("input_enabled") is True
            and host.get("helper_textarea_focused") is True
            and host.get("host_has_active_element") is True
            and not ((viewport.get("active_terminal_surface") or {}).get("problem"))
        ):
            return last_state
        focus_rect = (host or {}).get("host_rect") or dom_rect(last_state, "main_surface_body_rect")
        if rect_is_visible(focus_rect):
            xdotool_click_window(pid, rect_center_x(focus_rect), rect_center_y(focus_rect))
        time.sleep(0.12)
    raise AssertionError(f"failed to focus terminal helper textarea: {last_state!r}")


def prime_terminal_surface_for_keyboard(pid: int, session: str) -> dict:
    focused_state = focus_terminal_helper_textarea(pid, session)
    host = host_for_session_or_none(focused_state, session) or active_host_or_none(focused_state) or {}
    focus_rect = host.get("host_rect") or dom_rect(focused_state, "main_surface_body_rect")
    return {
        "focused_state_active_tag": ((focused_state.get("dom") or {}).get("active_element") or {}).get("tag"),
        "host_rect": focus_rect,
    }


def prime_terminal_surface_for_shortcut(pid: int, session: str) -> dict:
    state = app_state(pid)
    host = host_for_session_or_none(state, session) or active_host_or_none(state) or {}
    focus_rect = host.get("host_rect") or dom_rect(state, "main_surface_body_rect")
    if not rect_is_visible(focus_rect):
        raise AssertionError(f"terminal host rect is not visible for shortcut priming: {state!r}")
    window_id = visible_window_id_for_pid(pid)
    xdotool_activate_window_if_supported(pid, window_id)
    click = xdotool_click_window(pid, rect_center_x(focus_rect), rect_center_y(focus_rect))
    time.sleep(0.18)
    return {
        "click": click,
        "host_rect": focus_rect,
        "focused_state_active_tag": ((state.get("dom") or {}).get("active_element") or {}).get("tag"),
    }


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


def app_open_raw(
    pid: int,
    session: str,
    view: str = "terminal",
    *,
    timeout_ms: int = 20000,
) -> tuple[bool, dict, str]:
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
            str(timeout_ms),
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


def app_screenshot(pid: int, path: Path, *, crop_state: dict | None = None) -> dict:
    payload = run(
        "server",
        "app",
        "screenshot",
        "--pid",
        str(pid),
        str(path),
        "--timeout-ms",
        "8000",
    )
    screenshot_state = (payload.get("data") or {}) if isinstance(payload, dict) else {}
    effective_crop_state = crop_state
    if effective_crop_state is None:
        try:
            effective_crop_state = app_state(pid, timeout_ms=12000, retries=1)
        except AssertionError:
            effective_crop_state = screenshot_state
    if not effective_crop_state:
        effective_crop_state = screenshot_state
    if path.exists() and effective_crop_state and screenshot_crop_needs_root_recrop(path, effective_crop_state):
        repair_app_screenshot_from_root(path, effective_crop_state)
    if path.exists() and effective_crop_state:
        assert_shell_corner_rounding(
            pid,
            path,
            effective_crop_state,
            context=f"app screenshot {path.name}",
        )
    return payload


def screenshot_crop_needs_root_recrop(path: Path, state: dict) -> bool:
    if not path.exists():
        return False
    try:
        image = Image.open(path).convert("RGB")
    except OSError:
        return False
    thumbnail = image.resize((64, 64))
    stat = ImageStat.Stat(thumbnail)
    mean = stat.mean
    stddev = stat.stddev
    # App-window captures should retain sidebar/titlebar/chrome variation.
    # If the image is nearly flat and near-white, repair it from an explicit
    # root-window capture instead of trusting the transient surface snapshot.
    return max(stddev) < 1.6 and min(mean) > 220.0


def repair_app_screenshot_from_root(path: Path, state: dict) -> None:
    window = state.get("window") or {}
    outer_position = window.get("outer_position") or {}
    outer_size = window.get("outer_size") or {}
    display = str(window.get("display") or ENV.get("DISPLAY") or "").strip()
    left = int(round(float(outer_position.get("x") or 0.0)))
    top = int(round(float(outer_position.get("y") or 0.0)))
    width = int(round(float(outer_size.get("width") or 0.0)))
    height = int(round(float(outer_size.get("height") or 0.0)))
    if width <= 0 or height <= 0 or not display:
        return
    root_path = path.with_name(f"{path.stem}.root{path.suffix}")
    capture_root_screenshot(display, root_path)
    image = Image.open(root_path)
    image_width, image_height = image.size
    bounds = clamp_box((left, top, left + width, top + height), image.size)
    if bounds is None:
        return
    if bounds == (0, 0, image_width, image_height):
        image.save(path)
        return
    image.crop(bounds).save(path)


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
        "--query",
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


def app_set_window_chrome_hover(pid: int, active: bool) -> dict:
    return run(
        "server",
        "app",
        "chrome-hover",
        "--pid",
        str(pid),
        "on" if active else "off",
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


def app_set_main_zoom(pid: int, value: float, *, view: str = "terminal") -> dict:
    return run(
        "server",
        "app",
        "zoom",
        "--pid",
        str(pid),
        "--view",
        view,
        "--value",
        str(value),
        "--timeout-ms",
        "12000",
    )


def app_rows(pid: int) -> list[dict]:
    payload = run("server", "app", "rows", "--pid", str(pid), "--timeout-ms", "8000")
    return ((payload.get("data") or {}).get("rows") or [])


def preferred_hot_plain_session(pid: int) -> str | None:
    rows = app_rows(pid)
    candidates = []
    for row in rows:
        if str(row.get("kind") or "") != "Session":
            continue
        if str(row.get("icon_kind") or "") != "plain-terminal":
            continue
        path = str(row.get("path") or row.get("full_path") or "").strip()
        if not path:
            continue
        candidates.append(row)
    if not candidates:
        return None
    candidates.sort(
        key=lambda row: (
            0 if bool(row.get("selected")) else 1,
            0 if str(row.get("path") or row.get("full_path") or "").startswith("local://") else 1,
            0 if int(row.get("depth") or 0) == 1 else 1,
            str(row.get("path") or row.get("full_path") or ""),
        )
    )
    best = candidates[0]
    return str(best.get("path") or best.get("full_path") or "").strip() or None


def ensure_stable_plain_terminal_session(pid: int, preferred_session: str | None = None) -> str:
    session = None
    if preferred_session:
        normalized = str(preferred_session).strip()
        if normalized.startswith("local://"):
            session = normalized
    if session is None:
        session = preferred_hot_plain_session(pid)
    if not session:
        raise AssertionError(f"no hot plain terminal session available for pid {pid}")
    app_open(pid, session, view="terminal")
    app_set_search(pid, "", focused=False)
    deadline = time.time() + 8.0
    last_state = {}
    while time.time() < deadline:
        last_state = app_state(pid)
        host = active_host(last_state)
        shell = last_state.get("shell") or {}
        if (
            last_state.get("active_session_path") == session
            and last_state.get("active_view_mode") == "Terminal"
            and not shell.get("search_focused")
            and rect_is_visible(host.get("host_rect") or {})
        ):
            return session
        time.sleep(0.12)
    raise AssertionError(
        f"failed to restore stable plain terminal session {session!r} for pid {pid}: state={last_state!r}"
    )


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


def client_gui_env_for_pid(pid: int) -> dict:
    env = ENV.copy()
    state = app_state(pid)
    client_instance = state.get("client_instance") or {}
    window = state.get("window") or {}
    display = str(client_instance.get("display") or window.get("display") or "").strip()
    xauthority = str(client_instance.get("xauthority") or window.get("xauthority") or "").strip()
    wayland_display = str(
        client_instance.get("wayland_display") or window.get("wayland_display") or ""
    ).strip()
    xdg_runtime_dir = str(
        client_instance.get("xdg_runtime_dir") or window.get("xdg_runtime_dir") or ""
    ).strip()
    if display:
        env["DISPLAY"] = display
    if xauthority:
        env["XAUTHORITY"] = xauthority
    if wayland_display:
        env["WAYLAND_DISPLAY"] = wayland_display
    if xdg_runtime_dir:
        env["XDG_RUNTIME_DIR"] = xdg_runtime_dir
    if not display and not wayland_display:
        raise AssertionError(f"client {pid} is missing GUI display metadata")
    return env


def process_rss_kb(pid: int) -> int:
    for line in Path(f"/proc/{pid}/status").read_text(encoding="utf-8").splitlines():
        if line.startswith("VmRSS:"):
            return int(line.split()[1])
    raise AssertionError(f"VmRSS missing for pid {pid}")


def try_process_rss_kb(pid: int) -> int | None:
    try:
        return process_rss_kb(pid)
    except (AssertionError, OSError):
        return None


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


def process_identity(pid: int) -> dict:
    record = {"pid": int(pid), "comm": "", "cmdline": ""}
    try:
        record["comm"] = Path(f"/proc/{pid}/comm").read_text(encoding="utf-8").strip()
    except OSError:
        pass
    try:
        record["cmdline"] = (
            Path(f"/proc/{pid}/cmdline")
            .read_text(encoding="utf-8", errors="replace")
            .replace("\x00", " ")
            .strip()
        )
    except OSError:
        pass
    return record


def child_process_samples(pid: int) -> list[dict]:
    samples: list[dict] = []
    for child in child_pids(pid):
        rss_kb = try_process_rss_kb(child)
        if rss_kb is None:
            continue
        identity = process_identity(child)
        identity["rss_kb"] = rss_kb
        samples.append(identity)
    return samples


def webkit_child_samples(pid: int) -> list[dict]:
    return [
        sample
        for sample in child_process_samples(pid)
        if "webkit" in str(sample.get("comm") or "").lower()
        or "webkit" in str(sample.get("cmdline") or "").lower()
    ]


def assert_client_memory_budget(pid: int) -> dict:
    deadline = time.time() + 15.0
    last_sample = None
    while time.time() < deadline:
        main_rss_kb = process_rss_kb(pid)
        children = child_pids(pid)
        child_rss_kb = {
            child: rss_kb
            for child in children
            if (rss_kb := try_process_rss_kb(child)) is not None
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


def assert_webkit_child_rss_soak(pid: int) -> dict:
    baseline_state = app_state(pid)
    baseline_theme = str(((baseline_state.get("settings") or {}).get("theme")) or "light").strip() or "light"
    alternate_theme = "dark" if baseline_theme == "light" else "light"
    baseline_samples = webkit_child_samples(pid)
    if not baseline_samples:
        raise AssertionError(f"no WebKit child processes found under pid {pid}")
    baseline_by_comm = {
        str(sample.get("comm") or sample.get("pid")): int(sample.get("rss_kb") or 0)
        for sample in baseline_samples
    }
    peak_web_process_rss_kb = max(
        (
            int(sample.get("rss_kb") or 0)
            for sample in baseline_samples
            if "webkitwebproces" in str(sample.get("comm") or "").lower()
            or "webkitwebprocess" in str(sample.get("cmdline") or "").lower()
        ),
        default=0,
    )
    max_growth_kb = 0
    cycle_samples: list[dict] = []
    for cycle in range(WEBKIT_CHILD_RSS_SOAK_CYCLES):
        app_set_right_panel_mode(pid, "settings")
        time.sleep(WEBKIT_CHILD_RSS_SETTLE_SECONDS)
        app_set_right_panel_mode(pid, "hidden")
        time.sleep(WEBKIT_CHILD_RSS_SETTLE_SECONDS)
        app_theme(pid, alternate_theme if cycle % 2 == 0 else baseline_theme)
        time.sleep(WEBKIT_CHILD_RSS_SETTLE_SECONDS)
        app_theme(pid, baseline_theme if cycle % 2 == 0 else alternate_theme)
        time.sleep(WEBKIT_CHILD_RSS_SETTLE_SECONDS)
        samples = webkit_child_samples(pid)
        if not samples:
            raise AssertionError(f"WebKit child processes disappeared during soak cycle {cycle + 1}")
        web_process_rss_kb = 0
        for sample in samples:
            comm_key = str(sample.get("comm") or sample.get("pid"))
            rss_kb = int(sample.get("rss_kb") or 0)
            baseline_rss_kb = baseline_by_comm.get(comm_key, rss_kb)
            max_growth_kb = max(max_growth_kb, rss_kb - baseline_rss_kb)
            if (
                "webkitwebproces" in str(sample.get("comm") or "").lower()
                or "webkitwebprocess" in str(sample.get("cmdline") or "").lower()
            ):
                web_process_rss_kb = max(web_process_rss_kb, rss_kb)
        peak_web_process_rss_kb = max(peak_web_process_rss_kb, web_process_rss_kb)
        cycle_samples.append(
            {
                "cycle": cycle + 1,
                "children": samples,
                "web_process_rss_kb": web_process_rss_kb,
            }
        )
    app_theme(pid, baseline_theme)
    app_set_right_panel_mode(pid, "hidden")
    if peak_web_process_rss_kb > WEBKIT_WEB_PROCESS_RSS_MAX_KB:
        raise AssertionError(
            "WebKitWebProcess RSS exceeded the accepted soak budget: "
            f"peak={peak_web_process_rss_kb} limit={WEBKIT_WEB_PROCESS_RSS_MAX_KB} cycles={cycle_samples!r}"
        )
    if max_growth_kb > WEBKIT_CHILD_RSS_GROWTH_MAX_KB:
        raise AssertionError(
            "WebKit child RSS kept climbing across repeated UI cycles: "
            f"max_growth_kb={max_growth_kb} limit={WEBKIT_CHILD_RSS_GROWTH_MAX_KB} "
            f"baseline={baseline_samples!r} cycles={cycle_samples!r}"
        )
    return {
        "baseline": baseline_samples,
        "cycles": cycle_samples,
        "peak_web_process_rss_kb": peak_web_process_rss_kb,
        "max_growth_kb": max_growth_kb,
        "theme": baseline_theme,
    }


def perf_events_named(name: str) -> list[dict]:
    path = current_yggterm_home() / "perf-telemetry.jsonl"
    if not path.exists():
        return []
    events: list[dict] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if record.get("name") == name:
            events.append(record)
    return events


def assert_managed_cli_initial_install_deferred_contract() -> dict:
    report = run(
        "server",
        "remote",
        "refresh-managed-cli",
        "background",
        timeout_seconds=10.0,
    )
    if report.get("install_attempted"):
        raise AssertionError(
            f"managed cli background refresh still attempted the first install: {report!r}"
        )
    if not report.get("install_deferred"):
        raise AssertionError(
            f"managed cli background refresh did not defer the first install: {report!r}"
        )
    statuses = report.get("statuses") or []
    if not statuses or any(str(status.get("action") or "") != "deferred_install" for status in statuses):
        raise AssertionError(
            f"managed cli initial install defer did not report deferred_install statuses: {report!r}"
        )
    refresh_events = perf_events_named("refresh_managed_codex")
    if not refresh_events:
        raise AssertionError("no refresh_managed_codex perf event found after cold background proof")
    latest_refresh = (refresh_events[-1].get("payload") or {})
    latest_meta = latest_refresh.get("meta") or {}
    if latest_meta.get("install_attempted"):
        raise AssertionError(
            f"latest refresh_managed_codex perf event still recorded install_attempted: {latest_refresh!r}"
        )
    if not latest_meta.get("install_deferred"):
        raise AssertionError(
            f"latest refresh_managed_codex perf event did not record install_deferred: {latest_refresh!r}"
        )
    install_events = perf_events_named("refresh_managed_codex_install")
    if install_events:
        raise AssertionError(
            f"unexpected refresh_managed_codex_install event before explicit tool use: {install_events[-1]!r}"
        )
    return {
        "report": report,
        "latest_refresh_event": latest_refresh,
    }


def assert_managed_cli_refresh_ttl_contract() -> dict:
    install_report = run(
        "server",
        "remote",
        "refresh-managed-cli",
        "foreground",
        timeout_seconds=120.0,
    )
    if not install_report.get("install_attempted"):
        raise AssertionError(
            f"expected foreground managed cli refresh to perform the install path, got {install_report!r}"
        )
    if install_report.get("install_deferred"):
        raise AssertionError(
            f"foreground managed cli refresh unexpectedly deferred the install: {install_report!r}"
        )
    report = run(
        "server",
        "remote",
        "refresh-managed-cli",
        "background",
        timeout_seconds=10.0,
    )
    if not report.get("skipped_recently"):
        raise AssertionError(f"expected managed cli refresh to skip within TTL, got {report!r}")
    if report.get("install_attempted"):
        raise AssertionError(
            f"managed cli refresh still attempted npm install inside TTL: {report!r}"
        )
    ttl_remaining_ms = int(report.get("ttl_remaining_ms") or 0)
    if ttl_remaining_ms <= 0:
        raise AssertionError(f"managed cli refresh TTL did not report remaining time: {report!r}")
    statuses = report.get("statuses") or []
    if not statuses or any(str(status.get("action") or "") != "skipped_recent" for status in statuses):
        raise AssertionError(
            f"managed cli refresh did not report skipped_recent statuses: {report!r}"
        )
    refresh_events = perf_events_named("refresh_managed_codex")
    if not refresh_events:
        raise AssertionError("no refresh_managed_codex perf event found after TTL proof")
    latest_refresh = (refresh_events[-1].get("payload") or {})
    latest_meta = latest_refresh.get("meta") or {}
    if not latest_meta.get("skipped_recently"):
        raise AssertionError(
            f"latest refresh_managed_codex perf event did not record skipped_recently: {latest_refresh!r}"
        )
    if latest_meta.get("install_attempted"):
        raise AssertionError(
            f"latest refresh_managed_codex perf event still recorded install_attempted: {latest_refresh!r}"
        )
    install_events = perf_events_named("refresh_managed_codex_install")
    latest_install = (install_events[-1].get("payload") or {}) if install_events else None
    if latest_install is None:
        raise AssertionError("no refresh_managed_codex_install perf event found after explicit install")
    return {
        "install_report": install_report,
        "report": report,
        "latest_refresh_event": latest_refresh,
        "latest_install_event": latest_install,
    }


def latest_app_root_render_count(pid: int) -> dict | None:
    latest: dict | None = None
    for trace_path in event_trace_paths():
        if not trace_path.exists():
            continue
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
                candidate = {
                    "count": int(payload.get("count") or 0),
                    "ts_ms": int(record.get("ts_ms") or 0),
                    "trace_path": str(trace_path),
                    "source": "trace",
                }
                if latest is None or int(candidate["ts_ms"]) >= int(latest["ts_ms"]):
                    latest = candidate
    if latest is not None:
        return latest
    state = app_state(pid)
    browser_metrics = ((state.get("browser") or {}).get("metrics") or {})
    root_render_count = browser_metrics.get("root_render_count")
    if root_render_count is not None:
        return {
            "count": int(root_render_count or 0),
            "ts_ms": int(time.time() * 1000),
            "source": "app_state",
        }
    return None


def trace_event_count(pid: int, category: str, name: str) -> int:
    count = 0
    for trace_path in event_trace_paths():
        if not trace_path.exists():
            continue
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
                if record.get("category") != category:
                    continue
                if record.get("name") != name:
                    continue
                count += 1
    return count


def assert_no_duplicate_startup_terminal_bootstrap(pid: int) -> dict:
    state = app_state(pid)
    session_path = str(state.get("active_session_path") or "")
    attempt = state.get("terminal_open_attempt") or {}
    started_at_ms = int(attempt.get("started_at_ms") or 0)
    if not session_path or started_at_ms <= 0:
        raise AssertionError(
            f"could not determine startup terminal attempt for pid {pid}: "
            f"session={session_path!r} attempt={attempt!r}"
        )
    trace_paths = [path for path in event_trace_paths() if path.exists()]
    if not trace_paths:
        raise AssertionError(
            f"missing event traces for startup bootstrap check: {event_trace_paths()!r}"
        )
    window_start_ms = max(0, started_at_ms - 1000)
    window_end_ms = started_at_ms + 8000
    begin_events: list[dict] = []
    scheduled_events: list[dict] = []
    recovery_events: list[dict] = []
    skipped_existing_lease_events: list[dict] = []
    startup_ready_events: list[dict] = []
    skipped_duplicates = 0
    matched_trace_paths: set[str] = set()
    seen_event_keys: set[tuple[int, str, str, str, str]] = set()
    terminal_mount_events: list[dict] = []
    for trace_path in trace_paths:
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
                if record.get("component") != "ui" or record.get("category") != "terminal_mount":
                    continue
                ts_ms = int(record.get("ts_ms") or 0)
                if ts_ms < window_start_ms or ts_ms > window_end_ms:
                    continue
                payload = record.get("payload") or {}
                if str(payload.get("session_path") or "") != session_path:
                    continue
                matched_trace_paths.add(str(trace_path))
                name = str(record.get("name") or "")
                event_key = (
                    ts_ms,
                    name,
                    str(payload.get("session_path") or ""),
                    str(payload.get("mount_identity") or ""),
                    str(payload.get("request_id") or ""),
                )
                if event_key in seen_event_keys:
                    continue
                seen_event_keys.add(event_key)
                event = {
                    "ts_ms": ts_ms,
                    "name": name,
                    "payload": payload,
                    "trace_path": str(trace_path),
                }
                terminal_mount_events.append(event)
    ready_cutoff_ms = None
    for event in sorted(terminal_mount_events, key=lambda item: int(item.get("ts_ms") or 0)):
        name = str(event.get("name") or "")
        if name in ("attach_ready", "first_meaningful_output", "first_output", "js_ready"):
            startup_ready_events.append(event)
            if ready_cutoff_ms is None:
                ready_cutoff_ms = int(event.get("ts_ms") or 0) + 250
        if ready_cutoff_ms is not None and int(event.get("ts_ms") or 0) > ready_cutoff_ms:
            continue
        name = str(event.get("name") or "")
        if name == "begin":
            begin_events.append(event)
        elif name == "bootstrap_spawn_scheduled":
            scheduled_events.append(event)
        elif name == "startup_terminal_restore_recover":
            recovery_events.append(event)
        elif name == "bootstrap_spawn_skipped_existing_lease":
            skipped_existing_lease_events.append(event)
        elif name == "bootstrap_spawn_skipped_duplicate_attach":
            skipped_duplicates += 1
    if not begin_events and not scheduled_events and skipped_existing_lease_events:
        raise AssertionError(
            f"startup terminal bootstrap stalled behind an existing lease for {session_path}: "
            f"existing_lease_skips={len(skipped_existing_lease_events)} "
            f"recovery_events={len(recovery_events)} traces={sorted(matched_trace_paths)!r}"
        )
    if (
        not begin_events
        and not scheduled_events
        and not recovery_events
        and not skipped_existing_lease_events
        and skipped_duplicates == 0
    ):
        return {
            "session_path": session_path,
            "started_at_ms": started_at_ms,
            "begin_count": 0,
            "scheduled_count": 0,
            "recovery_count": 0,
            "existing_lease_skips": 0,
            "skipped_duplicates": 0,
            "trace_paths": sorted(matched_trace_paths),
            "trace_window_missing": True,
            "startup_ready_count": len(startup_ready_events),
            "startup_ready_cutoff_ms": ready_cutoff_ms,
        }
    if len(begin_events) != 1:
        raise AssertionError(
            f"startup terminal bootstrap duplicated for {session_path}: "
            f"begin_count={len(begin_events)} scheduled_count={len(scheduled_events)} "
            f"skipped_duplicates={skipped_duplicates} recovery_events={len(recovery_events)} "
            f"existing_lease_skips={len(skipped_existing_lease_events)} traces={trace_paths!r}"
        )
    if len(scheduled_events) != 1:
        raise AssertionError(
            f"startup terminal bootstrap scheduled {len(scheduled_events)} times for {session_path}: "
            f"skipped_duplicates={skipped_duplicates} recovery_events={len(recovery_events)} "
            f"existing_lease_skips={len(skipped_existing_lease_events)} traces={trace_paths!r}"
        )
    return {
        "session_path": session_path,
        "started_at_ms": started_at_ms,
        "begin_count": len(begin_events),
        "scheduled_count": len(scheduled_events),
        "recovery_count": len(recovery_events),
        "existing_lease_skips": len(skipped_existing_lease_events),
        "skipped_duplicates": skipped_duplicates,
        "startup_ready_count": len(startup_ready_events),
        "startup_ready_cutoff_ms": ready_cutoff_ms,
        "trace_paths": sorted(matched_trace_paths),
    }


def assert_idle_root_render_budget(pid: int) -> dict:
    before_state = wait_for_terminal_quiescent(pid, timeout_seconds=4.0, stable_polls=2)
    before_generation = before_state.get("generation") or {}
    before_hosts = ((before_state.get("dom") or {}).get("terminal_hosts") or [])
    before_rows = {
        str(row.get("path") or row.get("full_path") or ""): row for row in app_rows(pid)
    }
    before_host_render_counts = {
        str(host.get("session_path") or ""): int(host.get("render_event_count") or 0)
        for host in before_hosts
        if host.get("session_path")
    }
    before_terminal_io_dispatch_count = trace_event_count(pid, "terminal_io", "dispatch")
    time.sleep(IDLE_ROOT_RENDER_SAMPLE_SECONDS)
    after_state = app_state(pid)
    after_generation = after_state.get("generation") or {}
    after_hosts = ((after_state.get("dom") or {}).get("terminal_hosts") or [])
    after_rows = {
        str(row.get("path") or row.get("full_path") or ""): row for row in app_rows(pid)
    }
    after_host_render_counts = {
        str(host.get("session_path") or ""): int(host.get("render_event_count") or 0)
        for host in after_hosts
        if host.get("session_path")
    }
    after_terminal_io_dispatch_count = trace_event_count(pid, "terminal_io", "dispatch")
    stable_host_paths = []
    busy_host_paths = []
    for session_path in sorted(set(before_host_render_counts) | set(after_host_render_counts)):
        before_row = before_rows.get(session_path) or {}
        after_row = after_rows.get(session_path) or {}
        before_busy = bool(before_row.get("busy"))
        after_busy = bool(after_row.get("busy"))
        if before_busy or after_busy:
            busy_host_paths.append(session_path)
            continue
        stable_host_paths.append(session_path)
    host_render_deltas = {
        session_path: int(after_host_render_counts.get(session_path, 0))
        - int(before_host_render_counts.get(session_path, 0))
        for session_path in stable_host_paths
    }
    max_host_render_delta = max(host_render_deltas.values(), default=0)
    terminal_io_dispatch_delta = (
        after_terminal_io_dispatch_count - before_terminal_io_dispatch_count
    )
    monitored_host_count = max(1, len(stable_host_paths) + len(busy_host_paths))
    per_host_terminal_io_budget = max(
        1,
        (
            IDLE_TERMINAL_IO_BUSY_MAX_DELTA
            if busy_host_paths
            else IDLE_TERMINAL_IO_MAX_DELTA
        ) // IDLE_TERMINAL_IO_BASELINE_HOST_COUNT,
    )
    terminal_io_dispatch_budget = (
        per_host_terminal_io_budget * monitored_host_count
    )
    host_render_budget = max(
        IDLE_HOST_RENDER_MAX_DELTA,
        min(
            terminal_io_dispatch_budget + IDLE_HOST_RENDER_IO_OVERHEAD_MAX,
            terminal_io_dispatch_delta + IDLE_HOST_RENDER_IO_OVERHEAD_MAX,
        ),
    )
    sample = {
        "before_generation": before_generation,
        "after_generation": after_generation,
        "before_rows": {
            session_path: {"busy": bool((before_rows.get(session_path) or {}).get("busy"))}
            for session_path in sorted(before_host_render_counts)
        },
        "after_rows": {
            session_path: {"busy": bool((after_rows.get(session_path) or {}).get("busy"))}
            for session_path in sorted(after_host_render_counts)
        },
        "stable_host_paths": stable_host_paths,
        "busy_host_paths": busy_host_paths,
        "before_host_render_counts": before_host_render_counts,
        "after_host_render_counts": after_host_render_counts,
        "host_render_deltas": host_render_deltas,
        "max_host_render_delta": max_host_render_delta,
        "max_host_render_delta_budget": host_render_budget,
        "before_terminal_io_dispatch_count": before_terminal_io_dispatch_count,
        "after_terminal_io_dispatch_count": after_terminal_io_dispatch_count,
        "terminal_io_dispatch_delta": terminal_io_dispatch_delta,
        "terminal_io_dispatch_budget": terminal_io_dispatch_budget,
        "monitored_host_count": monitored_host_count,
        "per_host_terminal_io_budget": per_host_terminal_io_budget,
        "sample_seconds": IDLE_ROOT_RENDER_SAMPLE_SECONDS,
    }
    if before_generation != after_generation:
        raise AssertionError(
            f"idle semantic generation changed during steady-state window: {sample!r}"
        )
    if max_host_render_delta > host_render_budget:
        raise AssertionError(
            f"idle terminal host render budget exceeded: max_delta={max_host_render_delta} sample={sample!r}"
        )
    if terminal_io_dispatch_delta > terminal_io_dispatch_budget:
        raise AssertionError(
            f"idle terminal io dispatch budget exceeded: delta={terminal_io_dispatch_delta} sample={sample!r}"
        )
    return sample


def xdotool_env_for_pid(pid: int) -> dict:
    env = client_gui_env_for_pid(pid)
    display = str(env.get("DISPLAY") or "").strip()
    xauthority = str(env.get("XAUTHORITY") or "").strip()
    if not display:
        raise AssertionError(f"client {pid} is missing display metadata")
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


def xdotool_focus_belongs_to_pid(pid: int) -> bool:
    env = xdotool_env_for_pid(pid)
    focused = subprocess.run(
        ["xdotool", "getwindowfocus"],
        text=True,
        capture_output=True,
        env=env,
        timeout=XDO_TIMEOUT_SECONDS,
    )
    if focused.returncode != 0:
        return False
    window_id = (focused.stdout or "").strip()
    if not window_id:
        return False
    owner = subprocess.run(
        ["xdotool", "getwindowpid", window_id],
        text=True,
        capture_output=True,
        env=env,
        timeout=XDO_TIMEOUT_SECONDS,
    )
    return owner.returncode == 0 and (owner.stdout or "").strip() == str(pid)


def visible_window_origin_for_pid(pid: int, window_id: str | None = None) -> tuple[float, float]:
    env = xdotool_env_for_pid(pid)
    resolved_window_id = window_id or visible_window_id_for_pid(pid)
    state = app_state(pid)
    window = state.get("window") or {}
    outer_position = window.get("outer_position") or {}
    state_offset_x = float(outer_position.get("x") or 0.0)
    state_offset_y = float(outer_position.get("y") or 0.0)
    maximized = bool(window.get("maximized"))
    geometry = subprocess.run(
        ["xdotool", "getwindowgeometry", "--shell", resolved_window_id],
        text=True,
        capture_output=True,
        env=env,
        timeout=XDO_TIMEOUT_SECONDS,
    )
    if geometry.returncode == 0:
        offset_x = 0.0
        offset_y = 0.0
        for line in geometry.stdout.splitlines():
            if line.startswith("X="):
                offset_x = float(line.split("=", 1)[1] or 0.0)
            elif line.startswith("Y="):
                offset_y = float(line.split("=", 1)[1] or 0.0)
        if (
            not maximized
            and (state_offset_x != 0.0 or state_offset_y != 0.0)
            and (abs(offset_x - state_offset_x) >= 4.0 or abs(offset_y - state_offset_y) >= 12.0)
        ):
            return (state_offset_x, state_offset_y)
        return (offset_x, offset_y)
    return (state_offset_x, state_offset_y)


def screen_coordinates_for_window_point(
    pid: int,
    x: float,
    y: float,
    window_id: str | None = None,
) -> tuple[int, int]:
    offset_x, offset_y = visible_window_origin_for_pid(pid, window_id)
    return (
        int(round(offset_x + x)),
        int(round(offset_y + y)),
    )


def xdotool_click_window(pid: int, x: float, y: float, button: int = 1) -> dict:
    if POINTER_DRIVER == "app":
        app_pointer_command(
            pid,
            "click",
            x=x,
            y=y,
            button=app_pointer_button_name(button),
        )
        time.sleep(0.18)
        return {
            "window_id": "app",
            "x": int(round(x)),
            "y": int(round(y)),
            "button": int(button),
            "driver": "app",
        }
    env = xdotool_env_for_pid(pid)
    window_id = visible_window_id_for_pid(pid)
    if not xdotool_focus_belongs_to_pid(pid):
        xdotool_activate_window_if_supported(pid, window_id)
    screen_x, screen_y = screen_coordinates_for_window_point(pid, x, y, window_id)
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


def xdotool_press_window(pid: int, x: float, y: float, button: int = 1) -> dict:
    if POINTER_DRIVER == "app":
        app_pointer_command(
            pid,
            "press",
            x=x,
            y=y,
            button=app_pointer_button_name(button),
        )
        time.sleep(0.08)
        return {
            "window_id": "app",
            "x": int(round(x)),
            "y": int(round(y)),
            "button": int(button),
            "driver": "app",
        }
    env = xdotool_env_for_pid(pid)
    window_id = visible_window_id_for_pid(pid)
    if not xdotool_focus_belongs_to_pid(pid):
        xdotool_activate_window_if_supported(pid, window_id)
    screen_x, screen_y = screen_coordinates_for_window_point(pid, x, y, window_id)
    commands = [
        [
            "xdotool",
            "mousemove",
            "--sync",
            str(screen_x),
            str(screen_y),
        ],
        ["xdotool", "mousedown", str(button)],
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
            raise AssertionError(proc.stderr.strip() or f"xdotool press failed for pid {pid}: {command!r}")
        if command[1] == "mousedown":
            time.sleep(0.08)
    return {
        "window_id": window_id,
        "x": screen_x,
        "y": screen_y,
        "button": int(button),
    }


def xdotool_release_button(pid: int, button: int = 1) -> dict:
    if POINTER_DRIVER == "app":
        app_pointer_command(
            pid,
            "release",
            button=app_pointer_button_name(button),
        )
        time.sleep(0.08)
        return {
            "window_id": "app",
            "button": int(button),
            "driver": "app",
        }
    env = xdotool_env_for_pid(pid)
    proc = subprocess.run(
        ["xdotool", "mouseup", str(button)],
        text=True,
        capture_output=True,
        env=env,
        timeout=XDO_TIMEOUT_SECONDS,
    )
    if proc.returncode != 0:
        raise AssertionError(proc.stderr.strip() or f"xdotool release failed for pid {pid}")
    time.sleep(0.14)
    return {"button": int(button)}


def xdotool_move_window(pid: int, x: float, y: float) -> dict:
    if POINTER_DRIVER == "app":
        app_pointer_command(pid, "move", x=x, y=y)
        time.sleep(0.08)
        return {
            "window_id": "app",
            "x": int(round(x)),
            "y": int(round(y)),
            "driver": "app",
        }
    env = xdotool_env_for_pid(pid)
    window_id = visible_window_id_for_pid(pid)
    if not xdotool_focus_belongs_to_pid(pid):
        xdotool_activate_window_if_supported(pid, window_id)
    screen_x, screen_y = screen_coordinates_for_window_point(pid, x, y, window_id)
    command = [
        "xdotool",
        "mousemove",
        "--sync",
        str(screen_x),
        str(screen_y),
    ]
    try:
        proc = subprocess.run(
            command,
            text=True,
            capture_output=True,
            env=env,
            timeout=XDO_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired:
        fallback_command = [part for part in command if part != "--sync"]
        proc = subprocess.run(
            fallback_command,
            text=True,
            capture_output=True,
            env=env,
            timeout=XDO_TIMEOUT_SECONDS,
        )
    if proc.returncode != 0:
        raise AssertionError(proc.stderr.strip() or f"xdotool move failed for pid {pid}: {command!r}")
    time.sleep(0.16)
    return {
        "window_id": window_id,
        "x": screen_x,
        "y": screen_y,
    }


def xdotool_right_click_window(pid: int, x: float, y: float) -> dict:
    if POINTER_DRIVER == "app":
        app_pointer_command(pid, "click", x=x, y=y, button="secondary")
        time.sleep(0.18)
        return {
            "window_id": "app",
            "x": int(round(x)),
            "y": int(round(y)),
            "button": 3,
            "driver": "app",
        }
    env = xdotool_env_for_pid(pid)
    window_id = visible_window_id_for_pid(pid)
    if not xdotool_focus_belongs_to_pid(pid):
        xdotool_activate_window_if_supported(pid, window_id)
    screen_x, screen_y = screen_coordinates_for_window_point(pid, x, y, window_id)
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


def xdotool_double_click_window(pid: int, x: float, y: float, button: int = 1) -> dict:
    if POINTER_DRIVER == "app":
        app_pointer_command(
            pid,
            "double-click",
            x=x,
            y=y,
            button=app_pointer_button_name(button),
            count=2,
        )
        time.sleep(0.22)
        return {
            "window_id": "app",
            "x": int(round(x)),
            "y": int(round(y)),
            "button": int(button),
            "driver": "app",
        }
    env = xdotool_env_for_pid(pid)
    window_id = visible_window_id_for_pid(pid)
    if not xdotool_focus_belongs_to_pid(pid):
        xdotool_activate_window_if_supported(pid, window_id)
    screen_x, screen_y = screen_coordinates_for_window_point(pid, x, y, window_id)
    commands = [
        [
            "xdotool",
            "mousemove",
            "--sync",
            str(screen_x),
            str(screen_y),
        ],
        ["xdotool", "click", "--repeat", "2", "--delay", "90", str(button)],
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
                proc.stderr.strip() or f"xdotool double-click failed for pid {pid}: {command!r}"
            )
    time.sleep(0.22)
    return {
        "window_id": window_id,
        "x": screen_x,
        "y": screen_y,
        "button": int(button),
    }


def xdotool_drag_window(pid: int, start_x: float, start_y: float, end_x: float, end_y: float) -> dict:
    if POINTER_DRIVER == "app":
        app_pointer_command(
            pid,
            "drag",
            start_x=start_x,
            start_y=start_y,
            end_x=end_x,
            end_y=end_y,
            button="primary",
            steps=4,
            step_delay_ms=28,
        )
        time.sleep(0.34)
        return {
            "window_id": "app",
            "start": {"x": int(round(start_x)), "y": int(round(start_y))},
            "end": {"x": int(round(end_x)), "y": int(round(end_y))},
            "driver": "app",
        }
    env = xdotool_env_for_pid(pid)
    window_id = visible_window_id_for_pid(pid)
    if not xdotool_focus_belongs_to_pid(pid):
        xdotool_activate_window_if_supported(pid, window_id)
    start_screen_x, start_screen_y = screen_coordinates_for_window_point(
        pid,
        start_x,
        start_y,
        window_id,
    )
    end_screen_x, end_screen_y = screen_coordinates_for_window_point(
        pid,
        end_x,
        end_y,
        window_id,
    )
    mid_screen_x = start_screen_x + int(round((end_screen_x - start_screen_x) * 0.3))
    mid_screen_y = start_screen_y + int(round((end_screen_y - start_screen_y) * 0.3))
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
            str(mid_screen_x),
            str(mid_screen_y),
        ],
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
        if index == 1:
            # Give the shell a frame to enter its drag/resize capture path before the pointer moves.
            time.sleep(0.12)
        elif index == 2:
            time.sleep(0.12)
        elif index == 3:
            time.sleep(0.16)
    time.sleep(0.22)
    return {
        "window_id": window_id,
        "start": {"x": start_screen_x, "y": start_screen_y},
        "end": {"x": end_screen_x, "y": end_screen_y},
    }


def titlebar_drag_window(pid: int, start_x: float, start_y: float, end_x: float, end_y: float) -> dict:
    if POINTER_DRIVER == "app":
        drag = xdotool_drag_window(pid, start_x, start_y, end_x, end_y)
        time.sleep(0.34)
        drag["driver"] = "app"
        return drag
    drag = xdotool_drag_window(pid, start_x, start_y, end_x, end_y)
    drag["driver"] = "xdotool"
    return drag


def xdotool_key_window(pid: int, *keys: str) -> dict:
    if KEY_DRIVER == "app":
        app_key_command(pid, "press", *keys)
        time.sleep(0.12)
        return {
            "window_id": "app",
            "keys": list(keys),
            "driver": "app",
        }
    env = xdotool_env_for_pid(pid)
    window_id = visible_window_id_for_pid(pid)
    if not xdotool_focus_belongs_to_pid(pid):
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


def xdotool_key_focused(pid: int, *keys: str) -> dict:
    if KEY_DRIVER == "app":
        app_key_command(pid, "press", *keys)
        time.sleep(0.12)
        return {
            "window_id": "app",
            "keys": list(keys),
            "driver": "app",
        }
    env = xdotool_env_for_pid(pid)
    window_id = visible_window_id_for_pid(pid)
    if not xdotool_focus_belongs_to_pid(pid):
        xdotool_activate_window_if_supported(pid, window_id)
    proc = subprocess.run(
        ["xdotool", "key", "--clearmodifiers", *keys],
        text=True,
        capture_output=True,
        env=env,
        timeout=XDO_TIMEOUT_SECONDS,
    )
    if proc.returncode != 0:
        raise AssertionError(proc.stderr.strip() or f"xdotool focused key failed for pid {pid}: {keys!r}")
    time.sleep(0.12)
    return {
        "window_id": window_id,
        "keys": list(keys),
    }


def xdotool_type_window(pid: int, text: str) -> dict:
    if KEY_DRIVER == "app":
        app_key_command(pid, "type", text=text)
        time.sleep(0.12)
        return {
            "window_id": "app",
            "text": text,
            "driver": "app",
        }
    env = xdotool_env_for_pid(pid)
    window_id = visible_window_id_for_pid(pid)
    if not xdotool_focus_belongs_to_pid(pid):
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


def xdotool_type_focused(pid: int, text: str) -> dict:
    if KEY_DRIVER == "app":
        app_key_command(pid, "type", text=text)
        time.sleep(0.12)
        return {
            "window_id": "app",
            "text": text,
            "driver": "app",
        }
    env = xdotool_env_for_pid(pid)
    window_id = visible_window_id_for_pid(pid)
    if not xdotool_focus_belongs_to_pid(pid):
        xdotool_activate_window_if_supported(pid, window_id)
    proc = subprocess.run(
        ["xdotool", "type", "--clearmodifiers", "--", text],
        text=True,
        capture_output=True,
        env=env,
        timeout=XDO_TIMEOUT_SECONDS,
    )
    if proc.returncode != 0:
        raise AssertionError(proc.stderr.strip() or f"xdotool focused type failed for pid {pid}: {text!r}")
    time.sleep(0.18)
    return {
        "window_id": window_id,
        "text": text,
    }


def stop_clipboard_owner(proc: subprocess.Popen | None) -> None:
    return


def set_clipboard_text_for_pid(pid: int, text: str) -> subprocess.Popen | None:
    run(
        "server",
        "app",
        "clipboard",
        "text",
        "--pid",
        str(pid),
        "--value",
        text,
        "--timeout-ms",
        "12000",
    )
    time.sleep(0.18)
    return None


def set_clipboard_png_for_pid(pid: int) -> subprocess.Popen | None:
    image = Image.new("RGBA", (12, 12), (124, 200, 255, 255))
    buffer = io.BytesIO()
    image.save(buffer, format="PNG")
    payload = base64.b64encode(buffer.getvalue()).decode("ascii")
    run(
        "server",
        "app",
        "clipboard",
        "image",
        "--pid",
        str(pid),
        "--base64",
        payload,
        "--timeout-ms",
        "12000",
    )
    time.sleep(0.18)
    return None


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


def wait_for_titlebar_autohide_state(
    pid: int,
    *,
    enabled: bool | None = None,
    revealed: bool | None = None,
    hover_active: bool | None = None,
    toggle_enabled: bool | None = None,
    timeout_seconds: float = 6.0,
) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        last_state = app_state(pid)
        dom = last_state.get("dom") or {}
        if enabled is not None and bool(dom.get("titlebar_auto_hide_enabled")) != enabled:
            time.sleep(0.12)
            continue
        if revealed is not None and bool(dom.get("titlebar_revealed")) != revealed:
            time.sleep(0.12)
            continue
        if hover_active is not None and bool(dom.get("titlebar_hover_active")) != hover_active:
            time.sleep(0.12)
            continue
        if toggle_enabled is not None and bool(dom.get("settings_titlebar_auto_hide_toggle_enabled")) != toggle_enabled:
            time.sleep(0.12)
            continue
        return last_state
    raise AssertionError(
        "titlebar auto-hide state did not settle: "
        f"enabled={enabled!r} revealed={revealed!r} hover_active={hover_active!r} "
        f"toggle_enabled={toggle_enabled!r} state={last_state!r}"
    )


def titlebar_drag_request_count(state: dict) -> int:
    shell = state.get("shell") or {}
    return int(shell.get("titlebar_drag_request_count") or 0)


def wait_for_titlebar_drag_request(
    pid: int,
    previous_count: int,
    *,
    timeout_seconds: float = 2.5,
) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        last_state = app_state(pid)
        if titlebar_drag_request_count(last_state) > previous_count:
            return last_state
        time.sleep(0.12)
    raise AssertionError(
        "titlebar drag request did not register through app-control observability: "
        f"previous_count={previous_count} state={last_state!r}"
    )


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
    normalized = str(kind or "").strip().lower()
    if normalized == "codex-litellm":
        return "codex"
    return normalized


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


def probe_scroll(pid: int, session: str, lines: int, *, timeout_ms: int = 12000, retries: int = 2) -> dict:
    last_error: AssertionError | None = None
    for attempt in range(retries):
        if attempt > 0:
            terminal_reclaim_focus(pid, session)
            time.sleep(0.18)
        try:
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
                str(timeout_ms),
            ))
        except AssertionError as exc:
            last_error = exc
    assert last_error is not None
    raise last_error


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
    retries: int = 2,
) -> dict:
    last_error: AssertionError | None = None
    for attempt in range(retries):
        if attempt > 0:
            terminal_reclaim_focus(pid, session)
            time.sleep(0.18)
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
        try:
            return unwrap_data(run(*args))
        except AssertionError as exc:
            last_error = exc
    assert last_error is not None
    raise last_error


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


def parse_css_alpha(value: str | None) -> float | None:
    text = str(value or "").strip().lower()
    if not text:
        return None
    if text == "transparent":
        return 0.0
    if text.startswith("#"):
        return 1.0
    if text.startswith("rgb(") and text.endswith(")"):
        return 1.0
    if text.startswith("rgba(") and text.endswith(")"):
        parts = [part.strip() for part in text[5:-1].split(",")]
        if len(parts) != 4:
            return None
        try:
            return float(parts[3])
        except ValueError:
            return None
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


def css_colors_close(left: str, right: str, *, tolerance: float = 0.035) -> bool:
    left_rgb = parse_css_rgb(left)
    right_rgb = parse_css_rgb(right)
    if left_rgb is None or right_rgb is None:
        return False
    return all(abs(l - r) <= tolerance for l, r in zip(left_rgb, right_rgb))


def parse_css_px(value: object) -> float | None:
    text = str(value or "").strip().lower()
    if not text:
        return None
    if text.endswith("px"):
        text = text[:-2].strip()
    try:
        return float(text)
    except ValueError:
        return None


def parse_css_radius_values(value: object) -> list[float]:
    text = str(value or "").strip().lower().replace("/", " ").replace(",", " ")
    if not text:
        return []
    values: list[float] = []
    for part in text.split():
        px = parse_css_px(part)
        if px is not None:
            values.append(px)
    return values


def css_radius_delta(left: object, right: object) -> float | None:
    left_values = parse_css_radius_values(left)
    right_values = parse_css_radius_values(right)
    if not left_values or not right_values:
        return None
    max_len = max(len(left_values), len(right_values))
    if len(left_values) == 1:
        left_values = left_values * max_len
    if len(right_values) == 1:
        right_values = right_values * max_len
    if len(left_values) != len(right_values):
        return None
    return max(abs(l - r) for l, r in zip(left_values, right_values))


def rgba_distance(left: tuple[int, int, int, int], right: tuple[int, int, int, int]) -> float:
    return sum((float(l) - float(r)) ** 2 for l, r in zip(left[:3], right[:3])) ** 0.5


def capture_root_screenshot(display: str, path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["import", "-display", display, "-window", "root", str(path)],
        cwd=ROOT,
        text=True,
        capture_output=True,
        env=ENV,
        timeout=20.0,
        check=True,
    )


def shell_corner_signature(root_screenshot_path: Path, state: dict) -> dict | None:
    window = state.get("window") or {}
    dom = state.get("dom") or {}
    outer_position = window.get("outer_position") or {}
    shell_frame = dom.get("shell_frame_rect") or {}
    shell_radius = parse_css_px(dom.get("shell_frame_border_radius")) or 0.0
    window_left = int(round(float(outer_position.get("x") or 0.0)))
    window_top = int(round(float(outer_position.get("y") or 0.0)))
    window_width = int(round(float((window.get("outer_size") or {}).get("width") or 0.0)))
    window_height = int(round(float((window.get("outer_size") or {}).get("height") or 0.0)))
    left = int(round(window_left + float(shell_frame.get("left") or 0.0)))
    top = int(round(window_top + float(shell_frame.get("top") or 0.0)))
    width = int(round(float(shell_frame.get("width") or 0.0)))
    height = int(round(float(shell_frame.get("height") or 0.0)))
    if width <= 24 or height <= 24:
        return None
    image = Image.open(root_screenshot_path).convert("RGBA")
    image_width, image_height = image.size
    right = left + width
    bottom = top + height
    window_right = window_left + window_width
    window_bottom = window_top + window_height
    patch_size = 4
    outside_gap = patch_size + 2
    inset = max(8, int(round(shell_radius)) + 2)
    perimeter_margin = max(20, inset * 2)
    if left < patch_size or top < patch_size or right + patch_size >= image_width or bottom + patch_size >= image_height:
        return None

    def pixel(x: int, y: int) -> tuple[int, int, int, int]:
        return image.getpixel((max(0, min(image_width - 1, x)), max(0, min(image_height - 1, y))))

    def average_patch(x0: int, y0: int) -> tuple[int, int, int, int]:
        pixels = [
            pixel(x, y)
            for x in range(x0, x0 + patch_size)
            for y in range(y0, y0 + patch_size)
        ]
        count = len(pixels)
        return tuple(int(round(sum(channel[i] for channel in pixels) / count)) for i in range(4))  # type: ignore[return-value]
    def average_region(x0: int, y0: int, x1: int, y1: int) -> tuple[int, int, int, int]:
        pixels = [
            pixel(x, y)
            for x in range(x0, x1)
            for y in range(y0, y1)
        ]
        count = len(pixels)
        return tuple(int(round(sum(channel[i] for channel in pixels) / count)) for i in range(4))  # type: ignore[return-value]
    probe_span = max(4, min(int(round(max(shell_radius, 8.0) * 0.45)), 8))
    def corner_probe_pixels(name: str) -> list[tuple[int, int, int, int]]:
        pixels = []
        for dx in range(probe_span):
            for dy in range(probe_span):
                if dx + dy > probe_span + 1:
                    continue
                if name == "top_left":
                    x, y = left + dx, top + dy
                elif name == "top_right":
                    x, y = right - 1 - dx, top + dy
                elif name == "bottom_left":
                    x, y = left + dx, bottom - 1 - dy
                else:
                    x, y = right - 1 - dx, bottom - 1 - dy
                pixels.append(pixel(x, y))
        return pixels

    corners = [
        {
            "name": "top_left",
            "corner_patch": average_patch(left, top),
            "outside_patch": average_patch(left - patch_size, top - patch_size),
            "far_outside_patch": average_patch(left - outside_gap, top - outside_gap),
            "inside_patch": average_patch(left + inset, top + inset),
            "probe_pixels": corner_probe_pixels("top_left"),
        },
        {
            "name": "top_right",
            "corner_patch": average_patch(right - patch_size, top),
            "outside_patch": average_patch(right, top - patch_size),
            "far_outside_patch": average_patch(right + 2, top - outside_gap),
            "inside_patch": average_patch(right - patch_size - inset, top + inset),
            "probe_pixels": corner_probe_pixels("top_right"),
        },
        {
            "name": "bottom_left",
            "corner_patch": average_patch(left, bottom - patch_size),
            "outside_patch": average_patch(left - patch_size, bottom),
            "far_outside_patch": average_patch(left - outside_gap, bottom + 2),
            "inside_patch": average_patch(left + inset, bottom - patch_size - inset),
            "probe_pixels": corner_probe_pixels("bottom_left"),
        },
        {
            "name": "bottom_right",
            "corner_patch": average_patch(right - patch_size, bottom - patch_size),
            "outside_patch": average_patch(right, bottom),
            "far_outside_patch": average_patch(right + 2, bottom + 2),
            "inside_patch": average_patch(right - patch_size - inset, bottom - patch_size - inset),
            "probe_pixels": corner_probe_pixels("bottom_right"),
        },
    ]
    moat_bands = []
    def append_moat_band(name: str, x0: int, y0: int, x1: int, y1: int, ox0: int, oy0: int, ox1: int, oy1: int):
        if x1 <= x0 or y1 <= y0 or ox1 <= ox0 or oy1 <= oy0:
            return
        moat_bands.append(
            {
                "name": name,
                "moat_patch": average_region(x0, y0, x1, y1),
                "outside_patch": average_region(ox0, oy0, ox1, oy1),
            }
        )
    if left > window_left and width > perimeter_margin * 2:
        append_moat_band(
            "top_moat",
            left + perimeter_margin,
            window_top,
            right - perimeter_margin,
            top,
            left + perimeter_margin,
            max(0, window_top - patch_size),
            right - perimeter_margin,
            window_top,
        )
    if top > window_top and height > perimeter_margin * 2:
        append_moat_band(
            "left_moat",
            window_left,
            top + perimeter_margin,
            left,
            bottom - perimeter_margin,
            max(0, window_left - patch_size),
            top + perimeter_margin,
            window_left,
            bottom - perimeter_margin,
        )
    if window_right > right and height > perimeter_margin * 2:
        append_moat_band(
            "right_moat",
            right,
            top + perimeter_margin,
            window_right,
            bottom - perimeter_margin,
            window_right,
            top + perimeter_margin,
            min(image_width, window_right + patch_size),
            bottom - perimeter_margin,
        )
    if window_bottom > bottom and width > perimeter_margin * 2:
        append_moat_band(
            "bottom_moat",
            left + perimeter_margin,
            bottom,
            right - perimeter_margin,
            window_bottom,
            left + perimeter_margin,
            window_bottom,
            right - perimeter_margin,
            min(image_height, window_bottom + patch_size),
        )
    perimeter_bands = []
    if width > perimeter_margin * 2 and height > perimeter_margin * 2:
        perimeter_bands = [
            {
                "name": "top_band",
                "inner_patch": average_region(
                    left + perimeter_margin,
                    top,
                    right - perimeter_margin,
                    top + patch_size,
                ),
                "outside_patch": average_region(
                    left + perimeter_margin,
                    top - patch_size,
                    right - perimeter_margin,
                    top,
                ),
                "inside_patch": average_region(
                    left + perimeter_margin,
                    top + inset,
                    right - perimeter_margin,
                    top + inset + patch_size,
                ),
            },
            {
                "name": "left_band",
                "inner_patch": average_region(
                    left,
                    top + perimeter_margin,
                    left + patch_size,
                    bottom - perimeter_margin,
                ),
                "outside_patch": average_region(
                    left - patch_size,
                    top + perimeter_margin,
                    left,
                    bottom - perimeter_margin,
                ),
                "inside_patch": average_region(
                    left + inset,
                    top + perimeter_margin,
                    left + inset + patch_size,
                    bottom - perimeter_margin,
                ),
            },
            {
                "name": "right_band",
                "inner_patch": average_region(
                    right - patch_size,
                    top + perimeter_margin,
                    right,
                    bottom - perimeter_margin,
                ),
                "outside_patch": average_region(
                    right,
                    top + perimeter_margin,
                    right + patch_size,
                    bottom - perimeter_margin,
                ),
                "inside_patch": average_region(
                    right - patch_size - inset,
                    top + perimeter_margin,
                    right - inset,
                    bottom - perimeter_margin,
                ),
            },
            {
                "name": "bottom_band",
                "inner_patch": average_region(
                    left + perimeter_margin,
                    bottom - patch_size,
                    right - perimeter_margin,
                    bottom,
                ),
                "outside_patch": average_region(
                    left + perimeter_margin,
                    bottom,
                    right - perimeter_margin,
                    bottom + patch_size,
                ),
                "inside_patch": average_region(
                    left + perimeter_margin,
                    bottom - patch_size - inset,
                    right - perimeter_margin,
                    bottom - inset,
                ),
            },
        ]
    return {
        "root_screenshot_path": str(root_screenshot_path),
        "shell_frame_rect": {
            "left": left,
            "top": top,
            "width": width,
            "height": height,
        },
        "window_rect": {
            "left": window_left,
            "top": window_top,
            "width": window_width,
            "height": window_height,
        },
        "shell_radius_px": shell_radius,
        "corners": corners,
        "moat_bands": moat_bands,
        "perimeter_bands": perimeter_bands,
    }


def assert_shell_corner_rounding(pid: int, screenshot_path: Path, state: dict | None = None, *, context: str) -> dict | None:
    state = state or app_state(pid)
    window = state.get("window") or {}
    dom = state.get("dom") or {}
    display = str(window.get("display") or ENV.get("DISPLAY") or "").strip()
    expected_unmaximized_radius_px = 10.0
    if not display:
        return None
    def capture_signature(path: Path) -> tuple[dict | None, list[dict]]:
        capture_root_screenshot(display, path)
        signature = shell_corner_signature(path, state)
        if signature is None:
            return None, []
        samples = []
        for corner in signature["corners"]:
            corner_vs_outside = rgba_distance(corner["corner_patch"], corner["outside_patch"])
            corner_vs_far_outside = rgba_distance(corner["corner_patch"], corner["far_outside_patch"])
            corner_vs_inside = rgba_distance(corner["corner_patch"], corner["inside_patch"])
            probe_pixels = corner.get("probe_pixels") or []
            outside_like = 0
            inside_like = 0
            for probe_pixel in probe_pixels:
                probe_vs_outside = rgba_distance(probe_pixel, corner["far_outside_patch"])
                probe_vs_inside = rgba_distance(probe_pixel, corner["inside_patch"])
                if probe_vs_outside + 2.0 < probe_vs_inside:
                    outside_like += 1
                elif probe_vs_inside + 2.0 < probe_vs_outside:
                    inside_like += 1
            samples.append(
                {
                    "name": corner["name"],
                    "kind": "corner",
                    "corner_vs_outside": corner_vs_outside,
                    "corner_vs_far_outside": corner_vs_far_outside,
                    "corner_vs_inside": corner_vs_inside,
                    "corner_outside_like_ratio": (outside_like / len(probe_pixels)) if probe_pixels else 0.0,
                    "corner_inside_like_ratio": (inside_like / len(probe_pixels)) if probe_pixels else 0.0,
                }
            )
        for moat in signature.get("moat_bands", []):
            samples.append(
                {
                    "name": moat["name"],
                    "kind": "moat",
                    "corner_vs_outside": rgba_distance(moat["moat_patch"], moat["outside_patch"]),
                    "corner_vs_inside": 0.0,
                }
            )
        for band in signature.get("perimeter_bands", []):
            inner_vs_outside = rgba_distance(band["inner_patch"], band["outside_patch"])
            inner_vs_inside = rgba_distance(band["inner_patch"], band["inside_patch"])
            samples.append(
                {
                    "name": band["name"],
                    "kind": "perimeter",
                    "corner_vs_outside": inner_vs_outside,
                    "corner_vs_inside": inner_vs_inside,
                }
            )
        return signature, samples

    def bad_samples(samples: list[dict]) -> list[dict]:
        return [
            sample
            for sample in samples
            if (
                sample["kind"] == "corner"
                and (
                    (
                        sample["corner_vs_outside"] > 22.0
                        and sample["corner_vs_outside"] >= sample["corner_vs_inside"] * 0.7
                    )
                    or (
                        sample["corner_vs_far_outside"] >= sample["corner_vs_inside"] - 4.0
                    )
                    or (
                        sample["corner_outside_like_ratio"] < 0.55
                        and sample["corner_inside_like_ratio"] > 0.45
                    )
                )
            )
            or (
                sample["kind"] == "moat"
                and sample["corner_vs_outside"] > 18.0
            )
            or (
                sample["kind"] == "perimeter"
                and sample["corner_vs_outside"] > 20.0
                and sample["corner_vs_outside"] >= sample["corner_vs_inside"] * 1.4
            )
        ]

    def bad_flush_corner_samples(samples: list[dict]) -> list[dict]:
        return [
            sample
            for sample in samples
            if (
                sample["kind"] == "corner"
                and sample["corner_outside_like_ratio"] < 0.25
                and sample["corner_inside_like_ratio"] > 0.55
            )
        ]

    root_path = screenshot_path.with_name(f"{screenshot_path.stem}.root.png")
    try:
        signature, samples = capture_signature(root_path)
    except Exception:
        return None
    if signature is None:
        return None
    shell_frame_rect = signature["shell_frame_rect"]
    window_rect = signature["window_rect"]
    window_maximized = bool(window.get("maximized"))
    shell_radius_px = float(signature.get("shell_radius_px") or 0.0)
    shell_frame_shadow = str(dom.get("shell_frame_box_shadow") or "").strip().lower()
    if window_maximized:
        if shell_radius_px > 0.5:
            raise AssertionError(
                f"{context}: maximized shell should flatten to 0px radius, got {shell_radius_px:.1f}px"
            )
    elif abs(shell_radius_px - expected_unmaximized_radius_px) > 0.5:
        raise AssertionError(
            f"{context}: unmaximized shell radius regressed, expected {expected_unmaximized_radius_px:.0f}px and got {shell_radius_px:.1f}px"
        )
    flush_opaque_profile = (
        abs(int(shell_frame_rect["left"]) - int(window_rect["left"])) <= 1
        and abs(int(shell_frame_rect["top"]) - int(window_rect["top"])) <= 1
        and abs(int(shell_frame_rect["width"]) - int(window_rect["width"])) <= 1
        and abs(int(shell_frame_rect["height"]) - int(window_rect["height"])) <= 1
        and shell_frame_shadow in ("", "none")
    )
    if flush_opaque_profile:
        first_bad = bad_flush_corner_samples(samples)
        if not window_maximized and first_bad:
            raise AssertionError(
                f"{context}: unmaximized window fell back to a flush opaque frame with square outer edges; rounded shell corners regressed near {root_path}"
            )
        return {
            "root_screenshot_path": str(root_path),
            "root_retry_path": None,
            "window_rect": window_rect,
            "samples": samples,
            "transient_bad_samples": first_bad,
            "retry_samples": None,
            "mode": "flush_opaque_profile",
        }
    samples = [sample for sample in samples if sample["kind"] != "perimeter"]
    first_bad = bad_samples(samples)
    retry_path = None
    retry_samples: list[dict] | None = None
    if first_bad:
        retry_path = screenshot_path.with_name(f"{screenshot_path.stem}.root-retry.png")
        try:
            time.sleep(0.18)
            retry_signature, retry_samples = capture_signature(retry_path)
            if retry_signature is not None:
                signature = retry_signature
        except Exception:
            retry_samples = None
        if retry_samples is None:
            retry_samples = []
        retry_samples = [sample for sample in retry_samples if sample["kind"] != "perimeter"]
        second_bad = bad_samples(retry_samples)
        if second_bad:
            raise AssertionError(
                f"{context}: root-window perimeter/corner pixels no longer match the desktop outside the window and may show a white halo or square-border artifact near {retry_path or root_path}: first={first_bad!r} retry={second_bad!r}"
            )
    return {
        "root_screenshot_path": str(root_path),
        "root_retry_path": str(retry_path) if retry_path else None,
        "window_rect": signature["window_rect"],
        "samples": samples,
        "transient_bad_samples": first_bad,
        "retry_samples": retry_samples,
    }


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
    cursor_rect = host.get("cursor_sample_rect") or {}
    expected_rect = host.get("cursor_expected_rect") or {}
    probe_anchor_rect = expected_rect if rect_is_visible(expected_rect) else cursor_rect
    if not rect_is_visible(host_rect) or not rect_is_visible(probe_anchor_rect):
        raise AssertionError(f"{context}: missing host/cursor geometry for pixel probe")
    cursor_box = rect_bounds(probe_anchor_rect)
    host_box = rect_bounds(host_rect)
    image = Image.open(screenshot_path)
    visible_rows_by_index: dict[int, dict] = {}
    for sample in list(host.get("visible_row_samples_head") or []) + list(host.get("visible_row_samples_tail") or []):
        if not isinstance(sample, dict):
            continue
        try:
            row_index = int(sample.get("index"))
        except (TypeError, ValueError):
            continue
        visible_rows_by_index[row_index] = sample
    previous_row_overlaps_cursor_column = False
    try:
        cursor_visible_row_index = int(host.get("cursor_visible_row_index"))
        cursor_x = int(host.get("cursor_x"))
    except (TypeError, ValueError):
        cursor_visible_row_index = -1
        cursor_x = -1
    previous_row = visible_rows_by_index.get(cursor_visible_row_index - 1)
    if previous_row is not None and cursor_x >= 0:
        previous_row_text = str(previous_row.get("text") or "")
        if cursor_x < len(previous_row_text):
            previous_row_overlaps_cursor_column = not previous_row_text[cursor_x].isspace()
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
    probe_pixel_limits = {
        "above": 4,
        "below": 16,
        "right": 36,
    }
    probes: dict[str, dict] = {}
    for name, probe_box in probe_specs.items():
        if name == "above" and previous_row_overlaps_cursor_column:
            probes[name] = {
                "skipped": "previous_row_overlaps_cursor_column",
                "cursor_visible_row_index": cursor_visible_row_index,
                "cursor_x": cursor_x,
            }
            continue
        if probe_box[3] <= probe_box[1]:
            continue
        if probe_box[1] < host_box[1]:
            probe_box = (probe_box[0], host_box[1], probe_box[2], probe_box[3])
        clamped = clamp_box(probe_box, image.size)
        if clamped is None:
            continue
        crop = image.crop(clamped)
        non_background_pixels, background = count_non_background_pixels(crop)
        max_non_background_pixels = probe_pixel_limits.get(name, 4)
        if non_background_pixels > max_non_background_pixels:
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
            "max_non_background_pixels": max_non_background_pixels,
        }
    if not probes:
        return {"skipped": "probe_boxes_outside_image"}
    return probes


def assert_cursor_cell_glyph_pixels_visible(
    screenshot_path: Path,
    state: dict,
    *,
    context: str,
) -> dict:
    host = active_host(state)
    cursor_text = str(
        host.get("cursor_buffer_cell_text")
        or host.get("cursor_sample_text")
        or ""
    )
    if not cursor_text or cursor_text.isspace():
        return {"skipped": "cursor_text_blank"}
    cursor_rect = host.get("cursor_sample_rect") or host.get("cursor_expected_rect") or {}
    if not rect_is_visible(cursor_rect):
        raise AssertionError(f"{context}: missing cursor rect for glyph pixel probe")
    image = Image.open(screenshot_path)
    cursor_box = rect_bounds(cursor_rect)
    glyph_probe = (
        cursor_box[0] + 4,
        cursor_box[1] + 1,
        cursor_box[2] - 1,
        cursor_box[3] - 1,
    )
    clamped = clamp_box(glyph_probe, image.size)
    if clamped is None or clamped[2] <= clamped[0] or clamped[3] <= clamped[1]:
        return {"skipped": "cursor_glyph_probe_outside_image"}
    crop = image.crop(clamped)
    non_background_pixels, background = count_non_background_pixels(crop)
    if non_background_pixels <= 6:
        raise AssertionError(
            f"{context}: cursor cell only shows the bar and not the glyph pixels: "
            f"cursor_text={cursor_text!r} non_background_pixels={non_background_pixels}"
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
        "cursor_text": cursor_text,
    }


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
    used_clamped = clamped
    fallback_row_shift_px = 0
    fallback_search_region = None
    if non_background_pixels <= 12:
        row_height = max(1, row_box[3] - row_box[1])
        for shift_rows in (1, 2, 3):
            fallback = clamp_box(
                (
                    row_box[0],
                    row_box[1] - (row_height * shift_rows),
                    row_box[0] + prefix_width,
                    row_box[3] - (row_height * shift_rows),
                ),
                image.size,
            )
            if fallback is None:
                continue
            fallback_crop = image.crop(fallback)
            fallback_non_background_pixels, fallback_background = count_non_background_pixels(fallback_crop)
            if fallback_non_background_pixels > non_background_pixels:
                non_background_pixels = fallback_non_background_pixels
                background = fallback_background
                used_clamped = fallback
                fallback_row_shift_px = row_height * shift_rows
    if non_background_pixels <= 12:
        host_rect = host.get("host_rect") or {}
        if rect_is_visible(host_rect):
            host_box = rect_bounds(host_rect)
            search_region = clamp_box(
                (
                    host_box[0],
                    host_box[1],
                    min(host_box[2], host_box[0] + prefix_width),
                    min(host_box[3], host_box[1] + max(row_box[3] - row_box[1], 18) * 10),
                ),
                image.size,
            )
            if search_region is not None:
                search_crop = image.crop(search_region)
                search_non_background_pixels, search_background = count_non_background_pixels(search_crop)
                if search_non_background_pixels > non_background_pixels:
                    non_background_pixels = search_non_background_pixels
                    background = search_background
                    used_clamped = search_region
                    fallback_search_region = {
                        "left": search_region[0],
                        "top": search_region[1],
                        "width": search_region[2] - search_region[0],
                        "height": search_region[3] - search_region[1],
                    }
    if non_background_pixels <= 12:
        raise AssertionError(
            f"{context}: prompt-prefix pixels are missing from the screenshot despite visible cursor row text: "
            f"cursor_line={cursor_line_text!r} non_background_pixels={non_background_pixels}"
        )
    return {
        "probe_box": {
            "left": used_clamped[0],
            "top": used_clamped[1],
            "width": used_clamped[2] - used_clamped[0],
            "height": used_clamped[3] - used_clamped[1],
        },
        "background_rgb": background,
        "non_background_pixels": non_background_pixels,
        "cursor_line_text": cursor_line_text,
        "fallback_row_shift_px": fallback_row_shift_px,
        "fallback_search_region": fallback_search_region,
    }


def wait_for_window_geometry_settle(
    pid: int,
    *,
    min_width: int = WINDOW_SETTLE_MIN_WIDTH_PX,
    min_height: int = WINDOW_SETTLE_MIN_HEIGHT_PX,
    stable_polls: int = 3,
    timeout_seconds: float = 6.0,
) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    last_size = None
    stable = 0
    while time.time() < deadline:
        last_state = app_state(pid)
        window = last_state.get("window") or {}
        outer_size = window.get("outer_size") or {}
        width = int(round(float(outer_size.get("width") or 0.0)))
        height = int(round(float(outer_size.get("height") or 0.0)))
        size = (width, height)
        if width >= min_width and height >= min_height:
            if size == last_size:
                stable += 1
            else:
                stable = 1
                last_size = size
            if stable >= stable_polls:
                return last_state
        else:
            stable = 0
            last_size = size
        time.sleep(0.15)
    raise AssertionError(
        "window geometry did not settle before screenshot capture: "
        f"min=({min_width},{min_height}) last_state={last_state!r}"
    )


def capture_prompt_visible_screenshot(
    pid: int,
    session: str,
    screenshot_path: Path,
    *,
    context: str,
    timeout_seconds: float = 8.0,
) -> tuple[dict, dict]:
    deadline = time.time() + timeout_seconds
    last_state = {}
    last_error = None
    while time.time() < deadline:
        try:
            last_state = wait_for_visible_cursor_session(
                pid,
                session,
                timeout_seconds=min(3.0, max(1.5, deadline - time.time())),
            )
        except AssertionError as exc:
            last_error = exc
            time.sleep(0.12)
            continue
        try:
            last_state = wait_for_window_geometry_settle(
                pid,
                timeout_seconds=min(3.0, max(1.0, deadline - time.time())),
            )
        except AssertionError as exc:
            last_error = exc
            time.sleep(0.12)
            continue
        app_screenshot(pid, screenshot_path, crop_state=last_state)
        try:
            return last_state, assert_prompt_prefix_pixels_visible(
                screenshot_path,
                last_state,
                context=context,
            )
        except AssertionError as exc:
            last_error = exc
            time.sleep(0.18)
    raise AssertionError(
        f"{context}: prompt pixels never became visible in screenshots after retries: {last_error}"
    )


def strip_terminal_border(line: str) -> str:
    return line.strip().strip("╭╮╰╯─│ ").strip()


def terminal_chunk_has_codex_prompt_output(data: str) -> bool:
    normalized_lines = [line.strip() for line in str(data or "").splitlines() if line.strip()]
    if not normalized_lines:
        return False
    trailing_lines = normalized_lines[-4:]
    if len(trailing_lines) > 4 or any(len(line) > 160 for line in trailing_lines):
        return False
    return any(strip_terminal_border(line).startswith("›") for line in trailing_lines)


def host_has_live_codex_prompt(host: dict) -> bool:
    input_ready = host.get("input_enabled") is True or host.get("helper_textarea_focused") is True
    if not input_ready:
        return False
    text_sample = str(host.get("text_sample") or "")
    cursor_line_text = str(host.get("cursor_line_text") or host.get("cursor_row_text") or "")
    host_text = terminal_host_text(host)
    transcript_only_markers = (
        "To continue this session, run codex resume",
        "codex resume ",
    )
    if any(marker in host_text for marker in transcript_only_markers):
        return False
    return terminal_chunk_has_codex_prompt_output(cursor_line_text) or terminal_chunk_has_codex_prompt_output(
        text_sample
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


def terminal_host_text(host: dict) -> str:
    samples: list[str] = []
    for key in ("text_sample", "cursor_line_text", "cursor_row_text"):
        value = str(host.get(key) or "").strip()
        if value:
            samples.append(value)
    for row in list(host.get("visible_row_samples_head") or []) + list(host.get("visible_row_samples_tail") or []):
        value = str(row.get("text") or "").strip()
        if value:
            samples.append(value)
    return "\n".join(samples)


def detect_codex_startup_failure(host: dict) -> dict | None:
    haystack = terminal_host_text(host)
    recent = haystack[-4000:]
    lowered = recent.lower()
    if not (
        "mcp startup incomplete" in lowered
        or "failed to start: mcp startup failed" in lowered
        or "handshaking with mcp server failed" in lowered
        or ("mcp client for " in lowered and "failed to start" in lowered)
    ):
        return None
    return {
        "kind": "codex_apps_startup_failed" if "codex_apps" in lowered else "codex_mcp_startup_failed",
        "text_tail": recent[-1200:],
    }


def read_text_tail(path: Path, max_bytes: int = 262144) -> str:
    if not path.exists():
        return ""
    with path.open("rb") as fh:
        fh.seek(0, os.SEEK_END)
        size = fh.tell()
        fh.seek(max(0, size - max_bytes))
        return fh.read().decode("utf-8", errors="replace")


def codex_connector_log_hits(limit: int = 8) -> list[str]:
    log_tail = read_text_tail(Path.home() / ".codex" / "log" / "codex-tui.log", max_bytes=8 * 1024 * 1024)
    if not log_tail:
        return []
    keywords = (
        "failed to load full apps list",
        "403 forbidden",
        "failed to force-refresh tools for mcp server 'codex_apps'",
        "unexpected content type",
        "/connectors/directory/list",
        "enable javascript and cookies to continue",
    )
    matches = [
        line.strip()
        for line in log_tail.splitlines()
        if re.match(r"^20\d{2}-\d{2}-\d{2}T", line)
        and any(keyword in line.lower() for keyword in keywords)
    ]
    return matches[-limit:]


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
    viewport = viewport_state(state)
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
    if active_session_path:
        session_matches = [
            host
            for host in hosts
            if normalize_live_path(str(host.get("session_path") or ""))
            == normalize_live_path(str(active_session_path))
        ]
        if session_matches:
            focused_matches = [
                host for host in session_matches
                if host.get("helper_textarea_focused") is True or host.get("host_has_active_element") is True
            ]
            if focused_matches:
                return focused_matches[-1]
            return session_matches[-1]
    explicit_matches = [host for host in hosts if host.get("is_active_session_host") is True]
    if explicit_matches:
        return explicit_matches[-1]
    focused_hosts = [
        host for host in hosts
        if host.get("helper_textarea_focused") is True or host.get("host_has_active_element") is True
    ]
    if focused_hosts:
        return focused_hosts[-1]
    return hosts[-1]


def assert_only_active_host_accepts_input(state: dict) -> dict:
    active = active_host_or_none(state)
    if active is None:
        raise AssertionError("no terminal hosts present")
    active_session = str(active.get("session_path") or "")
    invalid = [
        {
            "session_path": str(host.get("session_path") or ""),
            "input_enabled": host.get("input_enabled"),
            "helper_textarea_focused": host.get("helper_textarea_focused"),
            "host_has_active_element": host.get("host_has_active_element"),
        }
        for host in terminal_hosts(state)
        if str(host.get("session_path") or "") != active_session and host.get("input_enabled") is True
    ]
    if invalid:
        raise AssertionError(f"inactive terminal hosts still accept input: {invalid!r}")
    return {
        "active_session_path": active_session,
        "inactive_input_enabled_count": 0,
    }


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


def dom_value(state: dict, key: str):
    return (state.get("dom") or {}).get(key)


def shell_theme_spec(state: dict, key: str) -> dict:
    spec = ((state.get("shell") or {}).get(key) or {})
    return spec if isinstance(spec, dict) else {}


def theme_grain_value(spec: dict) -> float | None:
    try:
        return float(spec.get("grain"))
    except (TypeError, ValueError, AttributeError):
        return None


def shell_context_menu_row_path(state: dict) -> str:
    shell = state.get("shell") or {}
    row = shell.get("context_menu_row") or {}
    return str(row.get("full_path") or "").strip()


def visible_sidebar_rows(state: dict) -> list[dict]:
    rows = ((state.get("dom") or {}).get("sidebar_visible_rows") or [])
    return rows if isinstance(rows, list) else []


def sidebar_row_rect(row: dict) -> dict:
    if isinstance(row.get("rect"), dict):
        return row.get("rect") or {}
    left = float(row.get("left") or 0.0)
    top = float(row.get("top") or 0.0)
    width = float(row.get("width") or 0.0)
    height = float(row.get("height") or 0.0)
    return {
        "left": left,
        "top": top,
        "width": width,
        "height": height,
        "right": left + width,
        "bottom": top + height,
    }


def find_visible_sidebar_row_by_path(state: dict, row_path: str, *, kind: str | None = None) -> dict | None:
    target_path = normalize_live_path(str(row_path or "").strip())
    if not target_path:
        return None
    matches = []
    for row in visible_sidebar_rows(state):
        row_path_value = normalize_live_path(str(row.get("path") or row.get("full_path") or "").strip())
        if row_path_value != target_path:
            continue
        if kind is not None and str(row.get("kind") or "").strip() != kind:
            continue
        matches.append(row)
    if matches:
        return matches[-1]
    return None


def wait_for_visible_sidebar_row(
    pid: int,
    row_path: str,
    *,
    kind: str | None = None,
    timeout_seconds: float = 8.0,
) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        last_state = app_state(pid)
        row = find_visible_sidebar_row_by_path(last_state, row_path, kind=kind)
        if row is not None and rect_is_visible(sidebar_row_rect(row)):
            return row
        time.sleep(0.1)
    raise AssertionError(
        f"sidebar row did not become visible: path={row_path!r} kind={kind!r} state={last_state!r}"
    )


def wait_for_sidebar_path_absent(pid: int, row_path: str, *, timeout_seconds: float = 10.0) -> dict:
    target_path = normalize_live_path(row_path)
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        last_state = app_state(pid)
        active_session = normalize_live_path(str(last_state.get("active_session_path") or "").strip())
        browser = last_state.get("browser") or {}
        browser_selected = normalize_live_path(str(browser.get("selected_path") or "").strip())
        selected_row = browser.get("selected_row") or {}
        selected_row_path = normalize_live_path(str(selected_row.get("full_path") or "").strip())
        visible = find_visible_sidebar_row_by_path(last_state, target_path) is not None
        snapshot_entry = find_snapshot_session(server_snapshot(), target_path)
        if (
            not visible
            and active_session != target_path
            and browser_selected != target_path
            and selected_row_path != target_path
            and snapshot_entry is None
        ):
            return last_state
        time.sleep(0.12)
    raise AssertionError(f"sidebar path still present after delete: path={row_path!r} state={last_state!r}")


def wait_for_app_idle(pid: int, timeout_seconds: float = 15.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        state = app_state(pid)
        last_state = state
        shell = state.get("shell") or {}
        requests = state.get("active_surface_requests") or []
        blocking_requests = [
            request
            for request in requests
            if str(request.get("operation") or "").strip() == "server_action"
            or str(request.get("surface") or "").strip() in {"App", "Terminal", "Preview"}
        ]
        if shell.get("server_busy") is True or blocking_requests:
            time.sleep(0.12)
            continue
        viewport = viewport_state(state)
        active_session = str(state.get("active_session_path") or "").strip()
        if not active_session:
            return state
        if viewport.get("ready") is True and viewport.get("interactive") is True:
            return state
        time.sleep(0.12)
    raise AssertionError(f"app did not become idle: {last_state!r}")


def invoke_sidebar_context_menu_action(
    pid: int,
    row_path: str,
    action: str,
    *,
    row_kind: str | None = None,
    timeout_seconds: float = 8.0,
) -> dict:
    xdotool_key_window(pid, "Escape")
    target_path = normalize_live_path(row_path)
    deadline = time.time() + timeout_seconds
    last_state = {}
    row = None
    while time.time() < deadline:
        last_state = wait_for_window_focus(pid, timeout_seconds=4.0)
        row = find_visible_sidebar_row_by_path(last_state, target_path, kind=row_kind)
        if row is not None and rect_is_visible(sidebar_row_rect(row)):
            break
        time.sleep(0.1)
    else:
        raise AssertionError(
            f"could not resolve sidebar row for context menu action: path={row_path!r} action={action!r} state={last_state!r}"
        )

    click_x = sidebar_row_click_x(last_state)
    select_click = xdotool_click_window(pid, click_x, rect_center_y(sidebar_row_rect(row)))
    time.sleep(0.18)

    opened = {}
    action_rect = None
    open_click = None
    for _attempt in range(4):
        current = wait_for_window_focus(pid, timeout_seconds=3.0)
        row = find_visible_sidebar_row_by_path(current, target_path, kind=row_kind) or row
        if row is None:
            raise AssertionError(f"sidebar row disappeared before context menu action: path={row_path!r}")
        click_x = sidebar_row_click_x(current)
        open_click = xdotool_right_click_window(pid, click_x, rect_center_y(sidebar_row_rect(row)))
        inner_deadline = time.time() + 2.5
        while time.time() < inner_deadline:
            opened = app_state(pid)
            actions = opened.get("dom", {}).get("context_menu_action_rects") or []
            action_rect = next(
                (entry.get("rect") for entry in actions if str(entry.get("action") or "").strip() == action),
                None,
            )
            if shell_context_menu_row_path(opened) == target_path and rect_is_visible(action_rect):
                break
            time.sleep(0.08)
        if shell_context_menu_row_path(opened) == target_path and rect_is_visible(action_rect):
            break
        xdotool_key_window(pid, "Escape")
        time.sleep(0.15)
    else:
        raise AssertionError(
            f"context menu action did not become clickable: path={row_path!r} action={action!r} state={opened!r}"
        )

    action_click = xdotool_click_window(pid, rect_center_x(action_rect), rect_center_y(action_rect))
    return {
        "target_path": target_path,
        "action": action,
        "select_click": select_click,
        "open_click": open_click,
        "action_click": action_click,
        "action_rect": action_rect,
    }


def create_terminal_via_sidebar_context_menu(pid: int, *, parent_path: str = "local") -> dict:
    baseline = wait_for_app_idle(pid, timeout_seconds=20.0)
    baseline_active_session_path = normalize_live_path(
        str(baseline.get("active_session_path") or "").strip()
    )
    baseline_paths = {
        normalize_live_path(str(row.get("path") or row.get("full_path") or "").strip())
        for row in visible_sidebar_rows(baseline)
        if str(row.get("path") or row.get("full_path") or "").strip()
    }
    action = invoke_sidebar_context_menu_action(pid, parent_path, "new-terminal")
    deadline = time.time() + 20.0
    last_state = {}
    created_path = ""
    while time.time() < deadline:
        last_state = app_state(pid)
        active_path = normalize_live_path(str(last_state.get("active_session_path") or "").strip())
        if active_path.startswith("local://") and active_path not in baseline_paths:
            created_path = active_path
            break
        new_visible_paths = [
            normalize_live_path(str(row.get("path") or row.get("full_path") or "").strip())
            for row in visible_sidebar_rows(last_state)
            if normalize_live_path(str(row.get("path") or row.get("full_path") or "").strip()).startswith("local://")
            and normalize_live_path(str(row.get("path") or row.get("full_path") or "").strip()) not in baseline_paths
        ]
        if new_visible_paths:
            created_path = new_visible_paths[-1]
            break
        blocking_requests = [
            request
            for request in (last_state.get("active_surface_requests") or [])
            if str(request.get("operation") or "").strip() == "server_action"
            or str(request.get("surface") or "").strip() in {"App", "Terminal", "Preview"}
        ]
        if blocking_requests:
            time.sleep(0.18)
            continue
        time.sleep(0.12)
    if not created_path:
        raise AssertionError(f"new terminal did not create a fresh local session: {last_state!r}")
    app_open(pid, created_path, view="terminal")
    wait_for_session_focus(pid, created_path, timeout_seconds=12.0)
    wait_for_visible_sidebar_row(pid, created_path, kind="Session", timeout_seconds=8.0)
    return {
        **action,
        "session_path": created_path,
        "active_title": last_state.get("active_title"),
        "baseline_active_session_path": baseline_active_session_path,
    }


def delete_session_via_context_menu(
    pid: int,
    session_path: str,
    *,
    fallback_session_path: str | None = None,
) -> dict:
    action = invoke_sidebar_context_menu_action(pid, session_path, "delete-session", row_kind="Session")
    deadline = time.time() + 3.0
    deleted = {}
    while time.time() < deadline:
        deleted = app_state(pid)
        pending = ((deleted.get("shell") or {}).get("pending_delete") or {})
        pending_paths = {normalize_live_path(str(path or "").strip()) for path in (pending.get("session_paths") or [])}
        dialog_visible = rect_is_visible(dom_rect(deleted, "delete_confirm_dialog_rect"))
        title_text = str(((deleted.get("dom") or {}).get("delete_confirm_title_text") or "")).strip()
        copy_text = str(((deleted.get("dom") or {}).get("delete_confirm_copy_text") or "")).strip().lower()
        if dialog_visible and (
            normalize_live_path(session_path) in pending_paths
            or (title_text == "Delete Selected Items?" and "live sessions" in copy_text)
        ):
            break
        time.sleep(0.05)
    else:
        raise AssertionError(f"context menu delete action did not open confirm dialog for session: {deleted!r}")

    confirm_rect = dom_rect(deleted, "delete_confirm_action_rect")
    if not rect_is_visible(confirm_rect):
        raise AssertionError(f"delete confirm action rect missing: {deleted!r}")
    confirm_click = xdotool_click_window(pid, rect_center_x(confirm_rect), rect_center_y(confirm_rect))
    removed_state = wait_for_sidebar_path_absent(pid, session_path, timeout_seconds=10.0)
    settled_state = wait_for_app_idle(pid, timeout_seconds=12.0)
    active_path = normalize_live_path(str(settled_state.get("active_session_path") or "").strip())
    fallback_path = normalize_live_path(str(fallback_session_path or "").strip())
    if fallback_path and fallback_path != normalize_live_path(session_path):
        if not active_path or active_path != fallback_path:
            settled_state = wait_for_session_focus(pid, fallback_path, timeout_seconds=12.0)
            active_path = fallback_path
        else:
            try:
                settled_state = wait_for_session_focus(pid, fallback_path, timeout_seconds=12.0)
                active_path = fallback_path
            except AssertionError:
                pass
    if active_path:
        try:
            settled_state = wait_for_visible_cursor_session(pid, active_path, timeout_seconds=8.0)
        except AssertionError:
            settled_state = wait_for_window_focus(pid, timeout_seconds=4.0)
        active_element = ((settled_state.get("dom") or {}).get("active_element") or {})
        host = active_host_or_none(settled_state)
        if (
            host is not None
            and rect_is_visible(host.get("host_rect"))
            and active_element.get("class_name") != "xterm-helper-textarea"
        ):
            xdotool_click_window(
                pid,
                rect_center_x(host["host_rect"]),
                rect_center_y(host["host_rect"]),
            )
            settled_state = wait_for_window_focus(pid, timeout_seconds=4.0)
    return {
        **action,
        "delete_confirm_dialog_rect": dom_rect(deleted, "delete_confirm_dialog_rect"),
        "delete_confirm_action_rect": confirm_rect,
        "delete_confirm_click": confirm_click,
        "post_delete_active_session_path": settled_state.get("active_session_path"),
    }


def selected_visible_session_row(state: dict) -> dict:
    browser = state.get("browser") or {}
    selected = browser.get("selected_row") or {}
    target_path = str(selected.get("full_path") or "").strip()
    target_kind = str(selected.get("kind") or "").strip()
    rows = visible_sidebar_rows(state)
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


def visible_shell_content_rects(state: dict) -> list[tuple[str, dict]]:
    rects: list[tuple[str, dict]] = []
    for key in ("sidebar_rect", "main_surface_rect", "right_side_rail_rect"):
        rect = dom_rect(state, key)
        if rect_is_visible(rect):
            rects.append((key, rect))
    return rects


def shell_content_vertical_balance(state: dict) -> dict:
    dom = state.get("dom") or {}
    window_height = float(dom.get("window_inner_height") or 0.0)
    if window_height <= 0:
        raise AssertionError(f"window inner height missing from app state: {state!r}")
    rects = visible_shell_content_rects(state)
    if not rects:
        raise AssertionError(f"shell content rects missing from app state: {state!r}")
    top_edge = min(float(rect.get("top") or 0.0) for _, rect in rects)
    bottom_edge = max(rect_bottom(rect) for _, rect in rects)
    top_gap = top_edge
    bottom_gap = window_height - bottom_edge
    return {
        "window_inner_height": window_height,
        "top_gap": top_gap,
        "bottom_gap": bottom_gap,
        "gap_delta": abs(top_gap - bottom_gap),
        "rects": {name: rect for name, rect in rects},
    }


def window_outer_geometry(state: dict) -> tuple[int, int, int, int]:
    window = state.get("window") or {}
    outer_position = window.get("outer_position") or {}
    outer_size = window.get("outer_size") or {}
    return (
        int(round(float(outer_position.get("x") or 0.0))),
        int(round(float(outer_position.get("y") or 0.0))),
        int(round(float(outer_size.get("width") or 0.0))),
        int(round(float(outer_size.get("height") or 0.0))),
    )


def titlebar_empty_lane_point(state: dict) -> tuple[float, float, str]:
    titlebar_rect = dom_rect(state, "titlebar_rect")
    left_rect = dom_rect(state, "titlebar_left_rect")
    search_rect = dom_rect(state, "titlebar_search_shell_rect")
    right_rect = dom_rect(state, "titlebar_right_rect")
    if not rect_is_visible(titlebar_rect):
        raise AssertionError(f"titlebar rect missing before empty-lane probe: {titlebar_rect!r}")
    y = rect_center_y(titlebar_rect)
    candidates: list[tuple[float, float, str]] = []
    if rect_is_visible(left_rect) and rect_is_visible(search_rect):
        gap_left = rect_right(left_rect)
        gap_right = float(search_rect.get("left") or 0.0)
        if gap_right - gap_left >= TITLEBAR_EMPTY_LANE_MIN_WIDTH_PX:
            candidates.append(((gap_left + gap_right) / 2.0, y, "left-search-gap"))
    if rect_is_visible(search_rect) and rect_is_visible(right_rect):
        gap_left = rect_right(search_rect)
        gap_right = float(right_rect.get("left") or 0.0)
        if gap_right - gap_left >= TITLEBAR_EMPTY_LANE_MIN_WIDTH_PX:
            candidates.append(((gap_left + gap_right) / 2.0, y, "search-right-gap"))
    if not candidates:
        raise AssertionError(
            "titlebar did not expose an empty draggable lane between content groups: "
            f"left={left_rect!r} search={search_rect!r} right={right_rect!r}"
        )
    return candidates[0]


def titlebar_transient_open(state: dict) -> bool:
    shell = state.get("shell") or {}
    active_element = ((state.get("dom") or {}).get("active_element") or {})
    active_id = str(active_element.get("id") or "")
    search_dropdown_visible = rect_is_visible(dom_rect(state, "titlebar_search_dropdown_rect"))
    search_focus_effective = bool(
        shell.get("search_focused")
        and (
            search_dropdown_visible
            or active_id == "yggterm-search-input"
            or str(shell.get("search_query") or "").strip() != ""
        )
    )
    return bool(
        search_focus_effective
        or shell.get("command_mode_active")
        or search_dropdown_visible
        or rect_is_visible(dom_rect(state, "titlebar_new_menu_rect"))
        or rect_is_visible(dom_rect(state, "titlebar_overflow_menu_rect"))
        or rect_is_visible(dom_rect(state, "titlebar_summary_menu_rect"))
    )


def ensure_unmaximized_window(pid: int, state: dict, *, timeout_seconds: float = 6.0) -> dict:
    if not bool((state.get("window") or {}).get("maximized")):
        return state
    app_set_maximized(pid, False)
    time.sleep(0.45)
    restored = wait_for_window_maximized(pid, False, timeout_seconds=timeout_seconds)
    if restored is not None:
        state = restored
    return wait_for_window_geometry_settle(pid, timeout_seconds=timeout_seconds)


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
                for _ in range(2):
                    xdotool_click_window(
                        pid,
                        rect_center_x(summary_button_rect),
                        rect_center_y(summary_button_rect),
                    )
                    settle_deadline = time.time() + 0.65
                    while time.time() < settle_deadline:
                        time.sleep(0.08)
                        last_state = app_state(pid)
                        if not titlebar_transient_open(last_state):
                            return last_state
                continue
            except Exception:
                pass
        new_menu_rect = dom_rect(last_state, "titlebar_new_menu_rect")
        new_button_rect = dom_rect(last_state, "titlebar_new_button_rect")
        if rect_is_visible(new_menu_rect) and rect_is_visible(new_button_rect):
            try:
                for _ in range(2):
                    xdotool_click_window(
                        pid,
                        rect_center_x(new_button_rect),
                        rect_center_y(new_button_rect),
                    )
                    settle_deadline = time.time() + 0.7
                    while time.time() < settle_deadline:
                        time.sleep(0.08)
                        last_state = app_state(pid)
                        if not titlebar_transient_open(last_state):
                            return last_state
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
        settle_deadline = time.time() + 0.45
        while time.time() < settle_deadline:
            time.sleep(0.08)
            last_state = app_state(pid)
            if not titlebar_transient_open(last_state):
                return last_state
    if not titlebar_transient_open(last_state):
        return last_state
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


def assert_no_problem_notifications(state: dict, *, context: str) -> dict:
    bad = problem_notifications(state)
    if bad:
        raise AssertionError(f"{context}: bad daemon/socket notifications observed: {bad!r}")
    return {"context": context, "bad_notification_count": 0}


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
        viewport = viewport_state(last_state)
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


def wait_for_interactive_session(pid: int, session: str, timeout_seconds: float = 20.0) -> dict:
    normalized_session = normalize_live_path(session)
    deadline = time.time() + timeout_seconds
    last_state = {}
    last_focus_attempt = 0.0
    last_open_attempt = 0.0
    last_dismiss_attempt = 0.0
    last_panel_close_attempt = 0.0
    while time.time() < deadline:
        last_state = app_state(pid)
        if normalize_live_path(str(last_state.get("active_session_path") or "")) != normalized_session:
            if time.time() - last_open_attempt >= 0.5:
                app_open(pid, session, view="terminal")
                last_open_attempt = time.time()
                time.sleep(0.18)
                last_state = app_state(pid)
        if (
            right_panel_mode(last_state)
            not in ("", "hidden", "none", "null")
            and time.time() - last_panel_close_attempt >= 0.5
        ):
            last_state = close_right_panel(pid, last_state, timeout_seconds=1.5)
            last_panel_close_attempt = time.time()
        if titlebar_transient_open(last_state) and time.time() - last_dismiss_attempt >= 0.5:
            try:
                last_state = dismiss_titlebar_transients(pid, last_state, timeout_seconds=1.5)
            except AssertionError:
                # Some title-chip/search shells can report visible for a stale frame while the
                # terminal itself is already healthy. Push focus back to the main surface and
                # let the settle loop re-evaluate instead of failing the whole smoke early.
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
                try:
                    xdotool_key_window(pid, "Escape")
                except Exception:
                    pass
                time.sleep(0.12)
                last_state = app_state(pid)
            last_dismiss_attempt = time.time()
        if shell_context_menu_row_path(last_state) and time.time() - last_dismiss_attempt >= 0.5:
            try:
                xdotool_key_window(pid, "Escape")
            except Exception:
                pass
            last_dismiss_attempt = time.time()
            time.sleep(0.1)
            last_state = app_state(pid)
        viewport = viewport_state(last_state)
        host = host_for_session_or_none(last_state, session)
        if host is None:
            time.sleep(0.25)
            continue
        if (
            normalize_live_path(str(last_state.get("active_session_path") or "")) == normalized_session
            and last_state.get("active_view_mode") == "Terminal"
            and viewport.get("ready") is True
            and viewport.get("interactive") is True
            and viewport.get("terminal_settled_kind") == "interactive"
            and host.get("input_enabled") is True
            and not visible_notifications(last_state)
            and not ((viewport.get("active_terminal_surface") or {}).get("problem"))
            and right_panel_mode(last_state) in ("", "hidden", "none", "null")
            and not titlebar_transient_open(last_state)
        ):
            return last_state
        if time.time() - last_focus_attempt >= 0.75:
            focus_rect = host.get("host_rect") or dom_rect(last_state, "main_surface_body_rect")
            if rect_is_visible(focus_rect):
                try:
                    xdotool_click_window(pid, rect_center_x(focus_rect), rect_center_y(focus_rect))
                    last_focus_attempt = time.time()
                    time.sleep(0.12)
                    last_state = app_state(pid)
                except Exception:
                    pass
        time.sleep(0.25)
    raise AssertionError(f"terminal session did not settle interactive for {session!r}: {last_state!r}")


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
    if AVOID_FOREGROUND:
        return app_state(pid)
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
    last_focus_attempt = 0.0
    last_open_attempt = 0.0
    last_open_error = ""
    while time.time() < deadline:
        last_state = app_state(pid)
        viewport = viewport_state(last_state)
        active_session_matches = (
            normalize_live_path(str(last_state.get("active_session_path") or ""))
            == normalized_session
        )
        active_view_mode = str(
            viewport.get("active_view_mode") or last_state.get("active_view_mode") or ""
        ).strip()
        if not active_session_matches or active_view_mode != "Terminal":
            if time.time() - last_open_attempt >= 0.5:
                ok, _payload, detail = app_open_raw(
                    pid,
                    session,
                    view="terminal",
                    timeout_ms=4000,
                )
                last_open_error = "" if ok else detail
                last_open_attempt = time.time()
                time.sleep(0.18)
                last_state = app_state(pid)
        active_element = (last_state.get("dom") or {}).get("active_element") or {}
        shell = last_state.get("shell") or {}
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
        if shell.get("search_focused") and not helper_active and time.time() - last_dismiss_attempt >= 0.5:
            try:
                app_set_search(pid, "", focused=False)
            except Exception:
                try:
                    xdotool_key_window(pid, "Escape")
                except Exception:
                    pass
            last_dismiss_attempt = time.time()
            time.sleep(0.12)
            last_state = app_state(pid)
        viewport = viewport_state(last_state)
        host = host_for_session_or_none(last_state, session)
        active = active_host_or_none(last_state)
        if host is None or active is None:
            time.sleep(0.25)
            continue
        active_session = normalize_live_path(str(active.get("session_path") or ""))
        helper_focused = host.get("helper_textarea_focused") is True
        host_has_active_element = host.get("host_has_active_element") is True
        window_focused = ((last_state.get("window") or {}).get("focused")) is True
        cursor_line_text = str(host.get("cursor_line_text") or host.get("cursor_row_text") or "")
        prompt_visible = bool(cursor_line_text.strip() or str(host.get("text_sample") or "").strip())
        cursor_visible = cursor_sample_is_visibly_active(host)
        focus_contract_ok = (helper_focused and host_has_active_element) or (
            window_focused and prompt_visible and cursor_visible
        )
        if (
            viewport.get("ready") is True
            and viewport.get("interactive") is True
            and viewport.get("terminal_settled_kind") == "interactive"
            and host.get("input_enabled") is True
            and focus_contract_ok
            and active_session == normalized_session
            and not visible_notifications(last_state)
            and not ((viewport.get("active_terminal_surface") or {}).get("problem"))
        ):
            return last_state
        if (
            active_session == normalized_session
            and viewport.get("ready") is True
            and viewport.get("interactive") is True
            and viewport.get("terminal_settled_kind") == "interactive"
            and host.get("input_enabled") is True
            and not visible_notifications(last_state)
            and not ((viewport.get("active_terminal_surface") or {}).get("problem"))
            and time.time() - last_focus_attempt >= 0.75
        ):
            focus_rect = host.get("host_rect") or dom_rect(last_state, "main_surface_body_rect")
            if rect_is_visible(focus_rect):
                try:
                    xdotool_click_window(pid, rect_center_x(focus_rect), rect_center_y(focus_rect))
                    last_focus_attempt = time.time()
                    time.sleep(0.12)
                    last_state = app_state(pid)
                except Exception:
                    pass
        time.sleep(0.25)
    error_suffix = f" last_open_error={last_open_error!r}" if last_open_error else ""
    raise AssertionError(f"terminal did not focus requested session: {last_state!r}{error_suffix}")


def wait_for_visible_cursor_session(pid: int, session: str, timeout_seconds: float = 12.0) -> dict:
    normalized_session = normalize_live_path(session)
    deadline = time.time() + timeout_seconds
    last_state = {}
    last_focus_attempt = 0.0
    last_keyboard_attempt = 0.0
    last_open_attempt = 0.0
    while time.time() < deadline:
        last_state = app_state(pid)
        if normalize_live_path(str(last_state.get("active_session_path") or "")) != normalized_session:
            if time.time() - last_open_attempt >= 0.5:
                app_open(pid, session, view="terminal")
                last_open_attempt = time.time()
                time.sleep(0.15)
                last_state = app_state(pid)
        viewport = viewport_state(last_state)
        host = host_for_session_or_none(last_state, session)
        if host is None:
            time.sleep(0.12)
            continue
        cursor_class_name = str(host.get("cursor_sample_class_name") or "")
        cursor_visible = cursor_sample_is_visibly_active(host)
        cursor_rect = host.get("cursor_sample_rect") or host.get("cursor_expected_rect") or {}
        visible_cursor_fallback = (
            host.get("helper_textarea_focused") is True
            and host.get("host_has_active_element") is True
            and rect_is_visible(cursor_rect)
            and host.get("xterm_cursor_hidden") is not True
            and (
                str(host.get("cursor_row_text") or host.get("cursor_line_text") or "").strip() != ""
                or int(host.get("cursor_visible_row_index") or -1) >= 0
            )
        )
        visible_block_cursor = (
            "xterm-cursor-block" in cursor_class_name
            and cursor_visible
            and rect_is_visible(cursor_rect)
            and host.get("xterm_cursor_hidden") is not True
        ) or visible_cursor_fallback
        if (
            normalize_live_path(str(last_state.get("active_session_path") or "")) == normalized_session
            and last_state.get("active_view_mode") == "Terminal"
            and viewport.get("ready") is True
            and viewport.get("interactive") is True
            and viewport.get("terminal_settled_kind") == "interactive"
            and host.get("input_enabled") is True
            and not visible_notifications(last_state)
            and not ((viewport.get("active_terminal_surface") or {}).get("problem"))
            and visible_block_cursor
        ):
            return last_state
        if time.time() - last_focus_attempt >= 0.5:
            focus_rect = host.get("host_rect") or dom_rect(last_state, "main_surface_body_rect")
            if rect_is_visible(focus_rect):
                try:
                    xdotool_click_window(pid, rect_center_x(focus_rect), rect_center_y(focus_rect))
                except Exception:
                    pass
            last_focus_attempt = time.time()
        if time.time() - last_keyboard_attempt >= 0.75:
            try:
                probe_type(pid, session, "", mode="keyboard")
            except Exception:
                pass
            last_keyboard_attempt = time.time()
        time.sleep(0.12)
    raise AssertionError(f"terminal did not reach visible block-cursor state for {session!r}: {last_state!r}")


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
        viewport = viewport_state(state)
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
        viewport = viewport_state(state)
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
        {"data": "q"},
        {"data": "\u0003"},
    )
    last_error: AssertionError | None = None
    for attempt in recovery_attempts:
        terminal_send(pid, session, str(attempt.get("data") or ""))
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


def terminal_visible_text_candidates(host: dict | None) -> list[str]:
    if not isinstance(host, dict):
        return []
    candidates: list[str] = []
    for value in (
        host.get("text_sample"),
        host.get("cursor_line_text"),
        host.get("cursor_row_text"),
        host.get("last_write_sample"),
        host.get("last_write_applied_tail"),
    ):
        text = str(value or "")
        if text:
            candidates.append(text)
    return candidates


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
    surface = (viewport_state(state).get("active_terminal_surface") or {})
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
    host_width = float(host_rect["width"])
    viewport_width = float(viewport_rect["width"])
    screen_width = float(screen_rect["width"])
    helpers_width = float(helpers_rect.get("width") or 0.0)
    compensated_screen_gap = (
        host_width >= 240.0
        and screen_width >= 200.0
        and helpers_width >= 200.0
        and viewport_width >= 200.0
        and abs(host_width - screen_width) > 12.0
        and abs(host_width - screen_width) <= 28.0
        and abs(screen_width - helpers_width) <= 4.0
        and abs(host_width - viewport_width) <= 4.0
    )
    if abs(screen_width - viewport_width) > 18.0 and not compensated_screen_gap:
        raise AssertionError(
            f"screen width drifted from viewport width: screen={screen_rect['width']!r} viewport={viewport_rect['width']!r}"
        )
    if abs(float(screen_rect["height"]) - float(viewport_rect["height"])) > 2.0:
        raise AssertionError(
            f"screen height drifted from viewport height: screen={screen_rect['height']!r} viewport={viewport_rect['height']!r}"
        )
    if rect_is_visible(helpers_rect):
        if abs(float(helpers_rect["width"]) - float(screen_rect["width"])) > 18.0 and not compensated_screen_gap:
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
    search_field_rect = search_input_rect or dom_rect(state, "titlebar_search_field_shell_rect")
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


def wait_for_titlebar_centering(pid: int, timeout_seconds: float = 4.0) -> tuple[dict, dict]:
    deadline = time.time() + timeout_seconds
    last_state = {}
    last_error = None
    reset_attempted = False
    app_set_search(pid, "", focused=False)
    while time.time() < deadline:
        last_state = wait_for_interactive(pid, timeout_seconds=min(2.0, max(0.5, deadline - time.time())))
        try:
            return last_state, assert_titlebar_centering(last_state)
        except AssertionError as exc:
            last_error = exc
            if not reset_attempted:
                app_set_search(pid, "", focused=True)
                time.sleep(0.12)
                app_set_search(pid, "", focused=False)
                reset_attempted = True
            time.sleep(0.12)
    if last_error is not None:
        raise last_error
    raise AssertionError(f"titlebar centering did not settle for pid {pid}")


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
    global LAST_SEARCH_FOCUS_OVERLAY_CONTRACT
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
    baseline_dom = baseline.get("dom") or {}
    native_wayland = bool(
        str(
            ((baseline.get("client_instance") or {}).get("wayland_display"))
            or ((baseline.get("window") or {}).get("wayland_display"))
            or ""
        ).strip()
    )
    search_input_rect = dom_rect(baseline, "titlebar_search_input_rect")
    baseline_search_field_shell_rect = dom_rect(baseline, "titlebar_search_field_shell_rect")
    baseline_search_field_shell_background = str(
        baseline_dom.get("titlebar_search_field_shell_background") or ""
    )
    baseline_search_field_shell_box_shadow = str(
        baseline_dom.get("titlebar_search_field_shell_box_shadow") or ""
    ).lower()
    baseline_search_field_shell_border_radius = str(
        baseline_dom.get("titlebar_search_field_shell_border_radius") or ""
    )
    if not rect_is_visible(baseline_titlebar):
        raise AssertionError(f"baseline titlebar rect missing: {baseline_titlebar!r}")
    if not rect_is_visible(baseline_host):
        raise AssertionError(f"baseline host rect missing: {baseline_host!r}")
    if not rect_is_visible(search_input_rect):
        raise AssertionError(f"baseline titlebar search input rect missing: {search_input_rect!r}")
    if not rect_is_visible(baseline_search_field_shell_rect):
        raise AssertionError(
            f"baseline titlebar search field shell rect missing: {baseline_search_field_shell_rect!r}"
        )
    if is_transparent_css_color(baseline_search_field_shell_background):
        raise AssertionError(
            "baseline titlebar search field shell lost its visible chrome before focus: "
            f"background={baseline_search_field_shell_background!r}"
        )
    if baseline_search_field_shell_box_shadow in ("", "none"):
        raise AssertionError(
            "baseline titlebar search field shell lost its supporting chrome shadow/border before focus: "
            f"box_shadow={baseline_search_field_shell_box_shadow!r}"
        )

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
    opened_via = "titlebar_click"
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
        app_set_search(pid, "", focused=True)
        time.sleep(0.18)
        fallback_deadline = time.time() + 2.0
        while time.time() < fallback_deadline:
            focused = app_state(pid)
            if (focused.get("shell") or {}).get("search_focused"):
                active_element = (focused.get("dom") or {}).get("active_element") or {}
                if str(active_element.get("id") or "") == "yggterm-search-input":
                    opened_via = "app_control_focus_fallback"
                    break
            time.sleep(0.08)
    if not ((focused.get("shell") or {}).get("search_focused")):
        raise AssertionError(
            f"titlebar search did not open focused search: click={click!r} focused={focused!r}"
        )
    active_element = (focused.get("dom") or {}).get("active_element") or {}
    if str(active_element.get("id") or "") != "yggterm-search-input":
        raise AssertionError(
            f"clicking the titlebar search field did not leave the search input active: "
            f"click={click!r} active_element={active_element!r} focused={focused!r}"
        )

    overlay_retry_limit = 3 if native_wayland else 2
    overlay_retry_count = 0
    while True:
        focused_settle_deadline = time.time() + (3.5 if native_wayland else 2.5)
        while time.time() < focused_settle_deadline:
            titlebar_rect = dom_rect(focused, "titlebar_rect")
            search_modal_rect = dom_rect(focused, "titlebar_search_outer_shell_rect")
            search_shell_rect = dom_rect(focused, "titlebar_search_field_shell_rect")
            search_dropdown_rect = dom_rect(focused, "titlebar_search_dropdown_rect")
            search_dropdown_header_rect = dom_rect(focused, "titlebar_search_dropdown_header_rect")
            search_dropdown_entry_rects = list((focused.get("dom") or {}).get("titlebar_search_dropdown_entry_rects") or [])
            if (
                rect_is_visible(titlebar_rect)
                and rect_is_visible(search_modal_rect)
                and rect_is_visible(search_shell_rect)
                and rect_is_visible(search_dropdown_rect)
                and rect_is_visible(search_dropdown_header_rect)
                and search_dropdown_entry_rects
                and rect_bottom(search_modal_rect) >= rect_bottom(search_dropdown_rect) + 2.0
                and float(search_dropdown_rect["top"]) >= rect_bottom(search_shell_rect) + 2.0
            ):
                break
            time.sleep(0.08)
            focused = app_state(pid)
            active_element = (focused.get("dom") or {}).get("active_element") or {}
            if str(active_element.get("id") or "") != "yggterm-search-input":
                raise AssertionError(
                    f"focused search lost the active input while waiting for overlay settle: "
                    f"active_element={active_element!r} state={focused!r}"
                )
        else:
            if overlay_retry_count + 1 >= overlay_retry_limit:
                break
            active_element = (focused.get("dom") or {}).get("active_element") or {}
            if native_wayland:
                click = xdotool_click_window(
                    pid,
                    rect_center_x(search_input_rect),
                    rect_center_y(search_input_rect),
                )
                time.sleep(0.14)
            app_set_search(pid, str(active_element.get("value") or ""), focused=True)
            focused = app_state(pid)
            opened_via = f"{opened_via}_overlay_fallback"
            overlay_retry_count += 1
            continue
        break

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
    if abs(float(search_shell_rect["height"]) - float(baseline_search_field_shell_rect["height"])) > 1.0:
        raise AssertionError(
            f"focused search field shell changed height instead of preserving the idle field shape: "
            f"baseline={baseline_search_field_shell_rect!r} focused={search_shell_rect!r}"
        )
    if abs(float(search_shell_rect["width"]) - float(baseline_search_field_shell_rect["width"])) > 2.0:
        raise AssertionError(
            f"focused search field shell changed width instead of staying anchored to the same titlebar slot: "
            f"baseline={baseline_search_field_shell_rect!r} focused={search_shell_rect!r}"
        )
    if float(search_dropdown_header_rect["top"]) < rect_bottom(search_shell_rect) + 2.0:
        raise AssertionError(
            f"search dropdown overlaps upward into the search field/titlebar geometry: "
            f"search_shell={search_shell_rect!r} search_dropdown_header={search_dropdown_header_rect!r}"
        )
    if float(search_modal_rect["width"]) < 280.0:
        raise AssertionError(
            f"focused search modal collapsed below the usable width floor: modal={search_modal_rect!r}"
        )
    if (
        float(search_dropdown_header_rect["left"]) < float(search_shell_rect["left"]) - 2.0
        or float(search_dropdown_header_rect["right"]) > float(search_shell_rect["right"]) + 2.0
    ):
        raise AssertionError(
            f"search dropdown header drifted outside the search shell width: "
            f"header={search_dropdown_header_rect!r} shell={search_shell_rect!r}"
        )
    for ix, rect in enumerate(search_dropdown_entry_rects):
        if (
            float(rect["left"]) < float(search_shell_rect["left"]) - 2.0
            or float(rect["right"]) > float(search_shell_rect["right"]) + 2.0
            or float(rect["top"]) < rect_bottom(search_dropdown_header_rect) - 1.0
        ):
            raise AssertionError(
                f"search dropdown entry {ix} drifted outside the visible dropdown body: "
                f"rect={rect!r} header={search_dropdown_header_rect!r} shell={search_shell_rect!r}"
            )
    dom = focused.get("dom") or {}
    search_field_shell_background = str(dom.get("titlebar_search_field_shell_background") or "")
    search_field_shell_box_shadow = str(dom.get("titlebar_search_field_shell_box_shadow") or "").lower()
    search_field_shell_border_radius = str(dom.get("titlebar_search_field_shell_border_radius") or "")
    search_input_background = str(dom.get("titlebar_search_input_background") or "")
    search_input_box_shadow = str(dom.get("titlebar_search_input_box_shadow") or "").lower()
    search_input_border_radius = str(dom.get("titlebar_search_input_border_radius") or "")
    search_shell_background = str(dom.get("titlebar_search_outer_shell_background") or "")
    search_shell_box_shadow = str(dom.get("titlebar_search_outer_shell_box_shadow") or "").lower()
    search_shell_border_radius = str(dom.get("titlebar_search_outer_shell_border_radius") or "")
    search_dropdown_background = str(dom.get("titlebar_search_dropdown_background") or "")
    search_dropdown_box_shadow = str(dom.get("titlebar_search_dropdown_box_shadow") or "").lower()
    search_field_shell_radius_delta = css_radius_delta(
        baseline_search_field_shell_border_radius,
        search_field_shell_border_radius,
    )
    search_field_shell_radius_values = parse_css_radius_values(search_field_shell_border_radius)
    search_shell_radius_values = parse_css_radius_values(search_shell_border_radius)
    if is_transparent_css_color(search_field_shell_background):
        raise AssertionError(
            "focused search field shell lost its visible chrome and fell back to the old floating-capsule behavior: "
            f"background={search_field_shell_background!r}"
        )
    if not css_colors_close(
        baseline_search_field_shell_background,
        search_field_shell_background,
        tolerance=0.04,
    ):
        raise AssertionError(
            "focused search field shell changed its fill instead of preserving the idle field treatment: "
            f"baseline={baseline_search_field_shell_background!r} focused={search_field_shell_background!r}"
        )
    if search_field_shell_box_shadow in ("", "none"):
        raise AssertionError(
            "focused search field shell lost its chrome instead of preserving the field inside the search modal: "
            f"box_shadow={search_field_shell_box_shadow!r}"
        )
    if css_colors_close(
        search_shell_background,
        search_field_shell_background,
        tolerance=0.012,
    ):
        raise AssertionError(
            "focused search modal shell is visually collapsing into the field instead of reading as a VS Code-like attached surface: "
            f"panel={search_shell_background!r} field={search_field_shell_background!r}"
        )
    if search_field_shell_radius_delta is None or search_field_shell_radius_delta > 1.0:
        raise AssertionError(
            "focused search field changed its corner geometry instead of keeping the same compact field shape: "
            f"baseline={baseline_search_field_shell_border_radius!r} focused={search_field_shell_border_radius!r}"
        )
    if search_field_shell_radius_values and max(search_field_shell_radius_values) > 12.5:
        raise AssertionError(
            f"focused search field became pill-like instead of staying softly rounded: {search_field_shell_border_radius!r}"
        )
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
    if search_shell_radius_values and max(search_shell_radius_values) > 15.0:
        raise AssertionError(
            "focused search outer shell drifted into a large pill instead of a VS Code-like modal surface: "
            f"border_radius={search_shell_border_radius!r}"
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
    result = {
        "opened_via": opened_via,
        "baseline_titlebar_rect": baseline_titlebar,
        "focused_titlebar_rect": titlebar_rect,
        "baseline_search_field_shell_rect": baseline_search_field_shell_rect,
        "search_modal_rect": search_modal_rect,
        "search_shell_rect": search_shell_rect,
        "search_dropdown_rect": search_dropdown_rect,
        "search_dropdown_header_rect": search_dropdown_header_rect,
        "search_dropdown_entry_rects": search_dropdown_entry_rects,
        "baseline_host_rect": baseline_host,
        "focused_host_rect": host_rect,
        "baseline_search_field_shell_background": baseline_search_field_shell_background,
        "baseline_search_field_shell_box_shadow": baseline_search_field_shell_box_shadow,
        "baseline_search_field_shell_border_radius": baseline_search_field_shell_border_radius,
        "search_field_shell_background": search_field_shell_background,
        "search_field_shell_box_shadow": search_field_shell_box_shadow,
        "search_field_shell_border_radius": search_field_shell_border_radius,
        "search_field_shell_radius_delta": search_field_shell_radius_delta,
        "search_input_background": search_input_background,
        "search_input_box_shadow": search_input_box_shadow,
        "search_input_border_radius": search_input_border_radius,
        "search_shell_background": search_shell_background,
        "search_shell_box_shadow": search_shell_box_shadow,
        "search_shell_border_radius": search_shell_border_radius,
        "search_dropdown_background": search_dropdown_background,
        "search_dropdown_box_shadow": search_dropdown_box_shadow,
        "focus_release_click": click,
        "retry_focus_release_click": retried_focus_click,
        "restored_search_focused": ((restored.get("shell") or {}).get("search_focused")),
        "restored_active_element": active_element,
    }
    LAST_SEARCH_FOCUS_OVERLAY_CONTRACT = result
    return result


def assert_titlebar_new_menu_shell_contract(pid: int) -> dict:
    global LAST_TITLEBAR_NEW_MENU_SHELL_CONTRACT
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

    click = None
    opened = {}
    for attempt in range(2):
        button_rect = dom_rect(baseline, "titlebar_new_button_rect")
        if not rect_is_visible(button_rect):
            raise AssertionError(f"titlebar new button rect missing before probe attempt {attempt + 1}: {baseline!r}")
        click = xdotool_click_window(pid, rect_center_x(button_rect), rect_center_y(button_rect))
        deadline = time.time() + 1.5
        while time.time() < deadline:
            opened = app_state(pid)
            if rect_is_visible(dom_rect(opened, "titlebar_new_menu_rect")):
                break
            time.sleep(0.12)
        if rect_is_visible(dom_rect(opened, "titlebar_new_menu_rect")):
            break
        baseline = dismiss_titlebar_transients(pid, opened, timeout_seconds=1.5)
        time.sleep(0.1)
    else:
        opened = app_state(pid)
    menu_rect = dom_rect(opened, "titlebar_new_menu_rect")
    opened_button_rect = dom_rect(opened, "titlebar_new_button_rect")
    session_button_rect = dom_rect(opened, "titlebar_session_button_rect")
    opened_button_count = int((opened.get("dom") or {}).get("titlebar_new_button_count") or 0)
    opened_button_visible_count = int((opened.get("dom") or {}).get("titlebar_new_button_visible_count") or 0)
    opened_button_rects = list((opened.get("dom") or {}).get("titlebar_new_button_rects") or [])
    opened_button_background = str((opened.get("dom") or {}).get("titlebar_new_button_background") or "").strip()
    opened_button_box_shadow = str((opened.get("dom") or {}).get("titlebar_new_button_box_shadow") or "").strip()
    opened_button_border_radius = str((opened.get("dom") or {}).get("titlebar_new_button_border_radius") or "").strip()
    session_button_box_shadow = str((opened.get("dom") or {}).get("titlebar_session_button_box_shadow") or "").strip()
    session_button_border_radius = str(
        (opened.get("dom") or {}).get("titlebar_session_button_border_radius") or ""
    ).strip()
    menu_background = str((opened.get("dom") or {}).get("titlebar_new_menu_background") or "").strip()
    menu_box_shadow = str((opened.get("dom") or {}).get("titlebar_new_menu_box_shadow") or "").strip()
    menu_action_rects = list((opened.get("dom") or {}).get("titlebar_new_menu_action_rects") or [])
    menu_action_backgrounds = list((opened.get("dom") or {}).get("titlebar_new_menu_action_backgrounds") or [])
    menu_action_box_shadows = list((opened.get("dom") or {}).get("titlebar_new_menu_action_box_shadows") or [])
    if not rect_is_visible(menu_rect):
        raise AssertionError(f"titlebar new menu rect missing after click: click={click!r} state={opened!r}")
    if opened_button_count != 1 or opened_button_visible_count != 1:
        raise AssertionError(
            f"titlebar new menu reopened with duplicate tab buttons instead of one attached tab: "
            f"count={opened_button_count} visible={opened_button_visible_count} rects={opened_button_rects!r}"
        )
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
    if rect_is_visible(session_button_rect):
        button_session_gap = float(session_button_rect["left"]) - float(opened_button_rect["right"])
        if button_session_gap < TITLEBAR_PLUS_SESSION_GAP_MIN_PX:
            raise AssertionError(
                f"titlebar new tab is still crowding the active session chip instead of leaving a deliberate gap: "
                f"gap={button_session_gap!r} button={opened_button_rect!r} session={session_button_rect!r}"
            )
        if button_session_gap > TITLEBAR_PLUS_SESSION_GAP_MAX_PX:
            raise AssertionError(
                f"titlebar new tab drifted too far from the active session chip: "
                f"gap={button_session_gap!r} button={opened_button_rect!r} session={session_button_rect!r}"
            )
        if session_button_box_shadow.lower() in ("", "none"):
            raise AssertionError(
                f"titlebar session chip lost its own chrome while the + menu was open: "
                f"box_shadow={session_button_box_shadow!r}"
            )
        normalized_session_radius = session_button_border_radius.replace(" ", "")
        if normalized_session_radius.startswith("0") or normalized_session_radius.startswith("0px"):
            raise AssertionError(
                f"titlebar session chip kept a flattened leading edge while the + menu was open: "
                f"border_radius={session_button_border_radius!r}"
            )
    if is_transparent_css_color(menu_background):
        raise AssertionError(
            f"titlebar new panel lost its shared modal background: background={menu_background!r}"
        )
    if str(opened_button_box_shadow or "").strip().lower() in ("", "none"):
        raise AssertionError(
            "titlebar new tab lost its attached-surface chrome, which leaves the top tab visually detached from the body: "
            f"box_shadow={opened_button_box_shadow!r}"
        )
    if menu_box_shadow in ("", "none"):
        raise AssertionError(
            f"titlebar new panel lost its shared modal shadow: box_shadow={menu_box_shadow!r}"
        )
    normalized_menu_box_shadow = str(menu_box_shadow or "").replace(" ", "").lower()
    if "0px0px0px1px" in normalized_menu_box_shadow:
        raise AssertionError(
            f"titlebar new panel still uses a full outline shadow, which reintroduces the tab/body seam: "
            f"box_shadow={menu_box_shadow!r}"
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
    for ix, background in enumerate(menu_action_backgrounds):
        if not is_transparent_css_color(background):
            raise AssertionError(
                f"titlebar new-menu action {ix} still renders as a boxed control instead of a menu item: "
                f"background={background!r}"
            )
    for ix, box_shadow in enumerate(menu_action_box_shadows):
        if str(box_shadow or "").strip().lower() not in ("", "none"):
            raise AssertionError(
                f"titlebar new-menu action {ix} still renders button chrome instead of menu-row chrome: "
                f"box_shadow={box_shadow!r}"
            )
    if not menu_action_rects:
        raise AssertionError("titlebar new-menu did not expose any action rects to verify")
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
    hover_move = xdotool_move_window(
        pid,
        rect_center_x(menu_action_rects[0]),
        rect_center_y(menu_action_rects[0]),
    )
    time.sleep(0.16)
    hovered = app_state(pid)
    hovered_menu_rect = dom_rect(hovered, "titlebar_new_menu_rect")
    hovered_backgrounds = list((hovered.get("dom") or {}).get("titlebar_new_menu_action_backgrounds") or [])
    if not rect_is_visible(hovered_menu_rect):
        raise AssertionError(
            f"titlebar new-menu closed or lost its shell during hover verification: move={hover_move!r} state={hovered!r}"
        )
    if not hovered_backgrounds:
        raise AssertionError(
            f"titlebar new-menu lost action backgrounds during hover verification: state={hovered!r}"
        )
    first_hover_background = str(hovered_backgrounds[0] or "").strip()
    if (
        len(hovered_backgrounds) > 1
        and not is_transparent_css_color(first_hover_background)
        and not is_transparent_css_color(str(hovered_backgrounds[1] or "").strip())
    ):
        raise AssertionError(
            f"titlebar new-menu hover leaked into non-hovered rows instead of isolating the active row: "
            f"backgrounds={hovered_backgrounds!r}"
        )
    hovered_shot = Path(f"/tmp/yggterm-titlebar-new-menu-hover-{pid}.png")
    app_screenshot(pid, hovered_shot)
    hovered_image = Image.open(hovered_shot)
    hovered_seam_crop = hovered_image.crop(seam_band)
    hovered_mismatch_pixels = count_background_mismatch_pixels(
        hovered_seam_crop,
        background=seam_background,
        tolerance=12,
    )
    if hovered_mismatch_pixels > 12:
        raise AssertionError(
            f"titlebar new-menu hover reintroduced the tab/body seam: "
            f"mismatch_pixels={hovered_mismatch_pixels} seam_band={seam_band}"
        )
    xdotool_click_window(pid, rect_center_x(button_rect), rect_center_y(button_rect))
    time.sleep(0.15)
    result = {
        "click": click,
        "hover_move": hover_move,
        "titlebar_rect": titlebar_rect,
        "button_rect": opened_button_rect,
        "button_count": opened_button_count,
        "button_visible_count": opened_button_visible_count,
        "button_rects": opened_button_rects,
        "button_background": opened_button_background,
        "button_box_shadow": opened_button_box_shadow,
        "button_border_radius": opened_button_border_radius,
        "session_button_rect": session_button_rect,
        "session_button_box_shadow": session_button_box_shadow,
        "session_button_border_radius": session_button_border_radius,
        "button_session_gap": (
            round(float(session_button_rect["left"]) - float(opened_button_rect["right"]), 2)
            if rect_is_visible(session_button_rect)
            else None
        ),
        "menu_rect": menu_rect,
        "menu_background": menu_background,
        "menu_box_shadow": menu_box_shadow,
        "menu_action_rects": menu_action_rects,
        "menu_action_backgrounds": menu_action_backgrounds,
        "menu_action_box_shadows": menu_action_box_shadows,
        "hovered_action_backgrounds": hovered_backgrounds,
        "seam_mismatch_pixels": mismatch_pixels,
        "hovered_seam_mismatch_pixels": hovered_mismatch_pixels,
    }
    LAST_TITLEBAR_NEW_MENU_SHELL_CONTRACT = result
    return result


def assert_background_blur_contract(pid: int) -> dict:
    state = wait_for_interactive(pid, timeout_seconds=10.0)
    dom = state.get("dom") or {}
    shell = state.get("shell") or {}
    live_blur_supported = bool(shell.get("live_blur_supported"))
    transparent_window = bool(shell.get("transparent_window"))
    profile_reason = str(shell.get("transparent_window_profile_reason") or "")
    shell_frame_backdrop = str(dom.get("shell_frame_backdrop_filter") or "").strip().lower()
    shell_root_backdrop = str(dom.get("shell_root_backdrop_filter") or "").strip().lower()
    shell_frame_background = str(dom.get("shell_frame_background") or "").strip()
    shell_root_background = str(dom.get("shell_root_background") or "").strip()
    shell_fill_alpha = parse_css_alpha(shell_frame_background)
    if shell_fill_alpha is None:
        shell_fill_alpha = parse_css_alpha(shell_root_background)
    blur_expected = live_blur_supported and transparent_window
    active_filter = shell_frame_backdrop or shell_root_backdrop
    result = {
        "live_blur_supported": live_blur_supported,
        "transparent_window": transparent_window,
        "profile_reason": profile_reason,
        "shell_frame_backdrop_filter": shell_frame_backdrop,
        "shell_root_backdrop_filter": shell_root_backdrop,
        "shell_frame_background": shell_frame_background,
        "shell_root_background": shell_root_background,
        "shell_fill_alpha": shell_fill_alpha,
    }
    if blur_expected:
        if active_filter in ("", "none"):
            raise AssertionError(f"expected live shell blur, but computed backdrop filter is absent: {result!r}")
        if shell_fill_alpha is not None and shell_fill_alpha >= 0.97:
            raise AssertionError(f"expected translucent shell fill on blur-capable backend: {result!r}")
    else:
        if active_filter not in ("", "none"):
            raise AssertionError(f"unexpected live shell blur on opaque/safe backend: {result!r}")
    return result


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
    baseline_handle_center_x = rect_center_x(handle_rect)
    baseline_handle_center_y = rect_center_y(handle_rect)

    delta = 56.0
    expected_widened_width = min(SIDEBAR_MAX_WIDTH, baseline_width + delta)
    drag = xdotool_drag_window(
        pid,
        baseline_handle_center_x,
        baseline_handle_center_y,
        baseline_handle_center_x + delta,
        baseline_handle_center_y,
    )
    deadline = time.time() + 4.0
    widened = {}
    widened_handle_rect = None
    while time.time() < deadline:
        widened = app_state(pid)
        widened_width = float(((widened.get("shell") or {}).get("sidebar_width")) or 0.0)
        widened_handle_rect = dom_rect(widened, "sidebar_resize_handle_rect")
        widened_handle_center_x = (
            rect_center_x(widened_handle_rect) if rect_is_visible(widened_handle_rect) else 0.0
        )
        if (
            widened_width >= expected_widened_width - 2.0
            and rect_is_visible(widened_handle_rect)
            and widened_handle_center_x >= baseline_handle_center_x + delta - 6.0
        ):
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
    if not rect_is_visible(widened_handle_rect):
        raise AssertionError(
            f"sidebar resize handle rect missing after widen probe: {widened_handle_rect!r}"
        )
    restore_drag = xdotool_drag_window(
        pid,
        rect_center_x(widened_handle_rect),
        rect_center_y(widened_handle_rect),
        rect_center_x(widened_handle_rect) - delta,
        rect_center_y(widened_handle_rect),
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
    global LAST_TITLEBAR_SESSION_SHELL_CONTRACT
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
    native_wayland = bool(
        str(
            ((baseline.get("client_instance") or {}).get("wayland_display"))
            or ((baseline.get("window") or {}).get("wayland_display"))
            or ""
        ).strip()
    )
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
        search_retry_limit = 3 if native_wayland else 2
        search_retry_count = 0
        search_opened = {}
        while True:
            search_click = xdotool_click_window(
                pid,
                rect_center_x(search_input_rect),
                rect_center_y(search_input_rect),
            )
            deadline = time.time() + (7.0 if native_wayland else 6.0)
            while time.time() < deadline:
                search_opened = app_state(pid)
                if rect_is_visible(dom_rect(search_opened, "titlebar_search_dropdown_rect")):
                    active_element = (search_opened.get("dom") or {}).get("active_element") or {}
                    if str(active_element.get("id") or "") == "yggterm-search-input":
                        break
                time.sleep(0.12)
            if rect_is_visible(dom_rect(search_opened, "titlebar_search_dropdown_rect")):
                break
            if search_retry_count + 1 >= search_retry_limit:
                raise AssertionError(
                    f"search shell did not open before the session-shell cross-state probe: "
                    f"search_click={search_click!r} state={search_opened!r}"
                )
            app_set_search(pid, "", focused=True)
            time.sleep(0.16 if native_wayland else 0.12)
            search_retry_count += 1
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
    opened_button_background = str(dom_value(opened, "titlebar_session_button_background") or "").strip()
    opened_menu_background = str(dom_value(opened, "titlebar_summary_menu_background") or "").strip()
    opened_button_box_shadow = str(dom_value(opened, "titlebar_session_button_box_shadow") or "").strip()
    summary_menu_box_shadow = str(dom_value(opened, "titlebar_summary_menu_box_shadow") or "").strip()
    regenerate_summary_background = str(
        dom_value(opened, "titlebar_summary_regenerate_summary_background") or ""
    ).strip()
    regenerate_summary_box_shadow = str(
        dom_value(opened, "titlebar_summary_regenerate_summary_box_shadow") or ""
    ).strip()
    summary_menu_border_top_width = str(
        dom_value(opened, "titlebar_summary_menu_border_top_width") or ""
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
    deadline = time.time() + 2.0
    focused = {}
    focused_button_rect = {}
    focused_summary_shell_rect = {}
    focused_summary_menu_rect = {}
    focused_regenerate_summary_rect = {}
    focused_button_background = ""
    focused_menu_background = ""
    focused_regenerate_summary_background = ""
    focused_regenerate_summary_box_shadow = ""
    focused_summary_menu_box_shadow = ""
    while time.time() < deadline:
        time.sleep(0.12)
        focused = app_state(pid)
        focused_button_rect = dom_rect(focused, "titlebar_session_button_rect")
        focused_summary_shell_rect = dom_rect(focused, "titlebar_summary_shell_rect")
        focused_summary_menu_rect = dom_rect(focused, "titlebar_summary_menu_rect")
        focused_regenerate_summary_rect = dom_rect(focused, "titlebar_summary_regenerate_summary_rect")
        focused_button_background = str(
            dom_value(focused, "titlebar_session_button_background") or ""
        ).strip()
        focused_menu_background = str(
            dom_value(focused, "titlebar_summary_menu_background") or ""
        ).strip()
        focused_regenerate_summary_background = str(
            dom_value(focused, "titlebar_summary_regenerate_summary_background") or ""
        ).strip()
        focused_regenerate_summary_box_shadow = str(
            dom_value(focused, "titlebar_summary_regenerate_summary_box_shadow") or ""
        ).strip()
        focused_summary_menu_box_shadow = str(
            dom_value(focused, "titlebar_summary_menu_box_shadow") or ""
        ).strip()
        if not rect_is_visible(focused_summary_menu_rect):
            break
        if focused_summary_menu_box_shadow not in ("", "none"):
            break
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
    top_right_probe_right = int(round(float(focused_summary_menu_rect["right"]) - 10))
    if rect_is_visible(focused_regenerate_summary_rect):
        top_right_probe_right = min(
            top_right_probe_right,
            int(round(float(focused_regenerate_summary_rect["left"]) - 8)),
        )
    top_right_probe_left = max(
        int(round(float(focused_summary_menu_rect["left"]) + 10)),
        top_right_probe_right - int(round(min(84.0, float(focused_summary_menu_rect["width"]) * 0.28))),
    )
    top_right_probe = clamp_box(
        (
            top_right_probe_left,
            int(round(float(focused_summary_menu_rect["top"]) + 8)),
            top_right_probe_right,
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
    if top_right_delta > 0.07:
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
    focus_settled = focused
    deadline = time.time() + 3.0
    while time.time() < deadline:
        focus_settled = app_state(pid)
        surface_problem = ((focus_settled.get("active_terminal_surface") or {}).get("problem")) or ""
        if (
            rect_is_visible(dom_rect(focus_settled, "titlebar_summary_menu_rect"))
            and rect_is_visible(dom_rect(focus_settled, "titlebar_search_input_rect"))
            and not surface_problem
            and bool(focus_settled.get("ready"))
        ):
            break
        time.sleep(0.12)
    focused = focus_settled
    search_input_rect = dom_rect(focused, "titlebar_search_input_rect")
    if rect_is_visible(search_input_rect):
        handoff_retry_limit = 3 if native_wayland else 2
        handoff_retry_count = 0
        switched = {}
        while True:
            search_click = xdotool_click_window(
                pid,
                rect_center_x(search_input_rect),
                rect_center_y(search_input_rect),
            )
            deadline = time.time() + (7.0 if native_wayland else 6.0)
            while time.time() < deadline:
                switched = app_state(pid)
                if (
                    rect_is_visible(dom_rect(switched, "titlebar_search_dropdown_rect"))
                    and not rect_is_visible(dom_rect(switched, "titlebar_summary_menu_rect"))
                ):
                    break
                time.sleep(0.12)
            if rect_is_visible(dom_rect(switched, "titlebar_search_dropdown_rect")):
                break
            if handoff_retry_count + 1 >= handoff_retry_limit:
                raise AssertionError(
                    f"clicking search did not open the search shell while the session shell was open: "
                    f"search_click={search_click!r} state={switched!r}"
                )
            app_set_search(pid, "", focused=True)
            time.sleep(0.16 if native_wayland else 0.12)
            handoff_retry_count += 1
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
    result = {
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
    LAST_TITLEBAR_SESSION_SHELL_CONTRACT = result
    return result


def capture_titlebar_session_shell_style(pid: int, session: str) -> dict:
    app_set_search(pid, "", focused=False)
    time.sleep(0.1)
    baseline = wait_for_window_focus(pid, timeout_seconds=3.0)
    if titlebar_transient_open(baseline):
        baseline = dismiss_titlebar_transients(pid, baseline, timeout_seconds=2.0)
    if right_panel_mode(baseline) not in ("", "hidden", "none", "null", "settings"):
        baseline = close_right_panel(pid, baseline, timeout_seconds=1.5)
    wait_for_session_focus(pid, session, timeout_seconds=8.0)
    time.sleep(0.18)
    baseline = app_state(pid)
    button_rect = dom_rect(baseline, "titlebar_session_button_rect")
    if not rect_is_visible(button_rect):
        raise AssertionError(
            f"titlebar session button missing during style capture: {baseline!r}"
        )
    xdotool_click_window(
        pid,
        rect_center_x(button_rect),
        rect_center_y(button_rect),
    )
    deadline = time.time() + 4.0
    opened = {}
    while time.time() < deadline:
        time.sleep(0.12)
        opened = app_state(pid)
        menu_rect = dom_rect(opened, "titlebar_summary_menu_rect")
        menu_background = normalize_css_value(
            str(dom_value(opened, "titlebar_summary_menu_background") or "")
        )
        if rect_is_visible(menu_rect) and menu_background not in ("", "none"):
            break
    else:
        raise AssertionError(
            f"titlebar session shell style capture did not open the summary menu: {opened!r}"
        )

    opened_button_rect = dom_rect(opened, "titlebar_session_button_rect")
    close_button_rect = opened_button_rect if rect_is_visible(opened_button_rect) else button_rect
    xdotool_click_window(
        pid,
        rect_center_x(close_button_rect),
        rect_center_y(close_button_rect),
    )
    close_deadline = time.time() + 2.5
    restored = {}
    while time.time() < close_deadline:
        time.sleep(0.12)
        restored = app_state(pid)
        if not rect_is_visible(dom_rect(restored, "titlebar_summary_menu_rect")):
            break
    else:
        restored = dismiss_titlebar_transients(pid, timeout_seconds=2.0)
    wait_for_session_focus(pid, session, timeout_seconds=8.0)
    return {
        "menu_background": normalize_css_value(
            str(dom_value(opened, "titlebar_summary_menu_background") or "")
        ),
        "menu_box_shadow": normalize_css_value(
            str(dom_value(opened, "titlebar_summary_menu_box_shadow") or "")
        ),
        "menu_rect": dom_rect(opened, "titlebar_summary_menu_rect"),
        "button_rect": close_button_rect,
        "seam_mismatch_pixels": None,
        "restored_state": restored,
    }


def assert_titlebar_modal_visual_parity(pid: int) -> dict:
    global LAST_SEARCH_FOCUS_OVERLAY_CONTRACT, LAST_TITLEBAR_NEW_MENU_SHELL_CONTRACT, LAST_TITLEBAR_SESSION_SHELL_CONTRACT
    session = str((app_state(pid).get("active_session_path")) or "")
    if not session:
        raise AssertionError("missing active session path for titlebar modal parity probe")
    cached_search = LAST_SEARCH_FOCUS_OVERLAY_CONTRACT or {}
    if (
        normalize_css_value(cached_search.get("search_shell_background") or "") not in ("", "none")
        and normalize_css_value(cached_search.get("search_shell_box_shadow") or "") not in ("", "none")
    ):
        search = cached_search
    else:
        search = assert_search_focus_overlay_contract(pid, session)
        dismiss_titlebar_transients(pid, timeout_seconds=2.0)
        wait_for_session_focus(pid, session, timeout_seconds=8.0)
        time.sleep(0.18)
    cached_new_menu = LAST_TITLEBAR_NEW_MENU_SHELL_CONTRACT or {}
    if (
        normalize_css_value(cached_new_menu.get("menu_background") or "") not in ("", "none")
        and normalize_css_value(cached_new_menu.get("menu_box_shadow") or "") not in ("", "none")
    ):
        new_menu = cached_new_menu
    else:
        new_menu = assert_titlebar_new_menu_shell_contract(pid)
        dismiss_titlebar_transients(pid, timeout_seconds=2.0)
        wait_for_session_focus(pid, session, timeout_seconds=8.0)
        time.sleep(0.18)
    cached_session_shell = LAST_TITLEBAR_SESSION_SHELL_CONTRACT or {}
    if (
        normalize_css_value(cached_session_shell.get("menu_background") or "") not in ("", "none")
        and (
            normalize_css_value(cached_session_shell.get("menu_box_shadow") or "") not in ("", "none")
            or cached_session_shell.get("seam_mismatch_pixels") is not None
        )
    ):
        session_shell = cached_session_shell
    else:
        session_shell = capture_titlebar_session_shell_style(pid, session)

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
    if search_box_shadow in ("", "none"):
        raise AssertionError(
            f"titlebar search modal lost its floating panel shadow: search={search_box_shadow!r}"
        )
    if new_box_shadow in ("", "none"):
        raise AssertionError(
            f"titlebar attached tab modals lost their shared shell shadow: new={new_box_shadow!r} session={session_box_shadow!r}"
        )
    session_seam_mismatch_pixels_raw = session_shell.get("seam_mismatch_pixels")
    session_seam_mismatch_pixels = (
        None
        if session_seam_mismatch_pixels_raw is None
        else int(session_seam_mismatch_pixels_raw)
    )
    if session_box_shadow in ("", "none"):
        if session_seam_mismatch_pixels is None:
            raise AssertionError(
                f"titlebar attached session shell is missing shadow and seam evidence: "
                f"session={session_box_shadow!r} new={new_box_shadow!r}"
            )
        if session_seam_mismatch_pixels != 0:
            raise AssertionError(
                f"titlebar attached session shell lost its observable shadow contract and still shows a seam: "
                f"session={session_box_shadow!r} seam_pixels={session_seam_mismatch_pixels} new={new_box_shadow!r}"
            )
    elif new_box_shadow != session_box_shadow:
        raise AssertionError(
            f"titlebar attached tab modals are still using different panel shadows: "
            f"new={new_box_shadow!r} session={session_box_shadow!r} search={search_box_shadow!r}"
        )
    return {
        "background": search_background,
        "search_box_shadow": search_box_shadow,
        "attached_box_shadow": new_box_shadow,
        "session_box_shadow": session_box_shadow,
        "session_seam_mismatch_pixels": session_seam_mismatch_pixels,
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
            if right_panel_mode(opened) not in ("", "hidden", "none", "null"):
                raise AssertionError(
                    "titlebar settings button opened the wrong right panel: "
                    f"click={click!r} mode={right_panel_mode(opened)!r} state={opened!r}"
                )
            time.sleep(0.12)
    if right_panel_mode(opened) != "settings":
        opened = open_settings_panel_via_command_lane(pid, timeout_seconds=6.0)
        open_strategy = "command_lane"
    input_rect = dom_rect(opened, "settings_interface_llm_input_rect")
    if not rect_is_visible(input_rect):
        raise AssertionError(f"interface llm input rect missing after opening settings: {opened!r}")
    host = active_host_or_none(opened)
    active_element = ((opened.get("dom") or {}).get("active_element") or {})
    if host is not None and host.get("input_enabled") is True:
        raise AssertionError(f"terminal input stayed enabled just by opening settings: host={host!r}")
    if host is not None and host.get("helper_textarea_focused") is True:
        raise AssertionError(
            f"terminal helper textarea kept focus just by opening settings: host={host!r} active_element={active_element!r}"
        )
    if host is not None and (
        host.get("xterm_present") is not True or host.get("viewport_present") is not True
    ):
        raise AssertionError(
            f"terminal surface stopped presenting just by opening settings: host={host!r}"
        )
    if active_element.get("data_settings_field_key") == "interface-llm":
        raise AssertionError(
            f"settings field auto-focused on open instead of waiting for explicit click: active_element={active_element!r}"
        )
    initial_value = str(((opened.get("settings") or {}).get("interface_llm_model")) or "")
    if not initial_value:
        initial_value = str((active_element.get("value")) or "")
    time.sleep(0.18)
    input_click = xdotool_click_window(pid, rect_center_x(input_rect), rect_center_y(input_rect))
    input_retry_click = None
    focus_state = {}
    deadline = time.time() + 6.0
    while time.time() < deadline:
        focus_state = app_state(pid)
        current_active = ((focus_state.get("dom") or {}).get("active_element") or {})
        if current_active.get("data_settings_field_key") == "interface-llm":
            break
        if input_retry_click is None and time.time() + 5.2 < deadline:
            input_retry_click = xdotool_click_window(pid, rect_center_x(input_rect), rect_center_y(input_rect))
        time.sleep(0.12)
    current_active = ((focus_state.get("dom") or {}).get("active_element") or {})
    if current_active.get("data_settings_field_key") != "interface-llm":
        raise AssertionError(
            f"interface llm input did not take focus after click: input_click={input_click!r} "
            f"retry={input_retry_click!r} active_element={current_active!r}"
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
    time.sleep(0.18)
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
            and reclaim_host.get("helper_textarea_focused") is True
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
        elif reclaim_retry_click is None and time.time() + 5.2 < deadline:
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
    if reclaim_host.get("helper_textarea_focused") is not True:
        raise AssertionError(
            f"terminal helper textarea did not reclaim focus after viewport click with settings open: "
            f"click={reclaim_click!r} retry={reclaim_retry_click!r} host={reclaim_host!r} reclaimed={reclaimed!r}"
        )
    if reclaim_active.get("data_settings_field_key") in {"interface-llm", "litellm-endpoint", "litellm-api-key"}:
        raise AssertionError(
            f"settings input kept focus after clicking the terminal viewport: active_element={reclaim_active!r}"
        )
    reclaim_text_before_candidates = terminal_visible_text_candidates(reclaim_host)
    reclaim_text_before = "\n".join(reclaim_text_before_candidates)
    reclaim_data_events_before = int(reclaim_host.get("data_event_count") or 0)
    reclaim_write_commands_before = int(reclaim_host.get("write_command_count") or 0)
    reclaim_last_write_before = int(reclaim_host.get("last_write_queued_at_ms") or 0)
    reclaim_probe_char = "/"
    reclaim_session = str(reclaimed.get("active_session_path") or "")
    if not reclaim_session:
        raise AssertionError(f"active session path missing before viewport reclaim typing probe: reclaimed={reclaimed!r}")
    time.sleep(0.22)
    try:
        reclaim_type = probe_type(pid, reclaim_session, reclaim_probe_char, mode="keyboard")
    except AssertionError:
        reclaim_type = terminal_send(pid, reclaim_session, reclaim_probe_char)
    reclaim_typed = {}
    deadline = time.time() + 4.0
    while time.time() < deadline:
        reclaim_typed = app_state(pid)
        typed_host = active_host_or_none(reclaim_typed)
        typed_candidates = terminal_visible_text_candidates(typed_host)
        typed_sample = "\n".join(typed_candidates)
        typed_data_events = int((typed_host or {}).get("data_event_count") or 0)
        typed_write_commands = int((typed_host or {}).get("write_command_count") or 0)
        typed_last_write = int((typed_host or {}).get("last_write_queued_at_ms") or 0)
        if (
            (
                any(reclaim_probe_char in candidate for candidate in typed_candidates)
                and typed_sample != reclaim_text_before
            )
            or typed_data_events > reclaim_data_events_before
            or typed_write_commands > reclaim_write_commands_before
            or typed_last_write > reclaim_last_write_before
        ):
            break
        time.sleep(0.12)
    typed_host = active_host_or_none(reclaim_typed)
    typed_candidates = terminal_visible_text_candidates(typed_host)
    typed_sample = "\n".join(typed_candidates)
    typed_data_events = int((typed_host or {}).get("data_event_count") or 0)
    typed_write_commands = int((typed_host or {}).get("write_command_count") or 0)
    typed_last_write = int((typed_host or {}).get("last_write_queued_at_ms") or 0)
    if (
        not (
            (
                any(reclaim_probe_char in candidate for candidate in typed_candidates)
                and typed_sample != reclaim_text_before
            )
            or typed_data_events > reclaim_data_events_before
            or typed_write_commands > reclaim_write_commands_before
            or typed_last_write > reclaim_last_write_before
        )
    ):
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
            and settled_host.get("helper_textarea_focused") is True
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
    selection = assert_selection(pid, reclaim_session)
    settled = wait_for_session_focus(pid, reclaim_session, timeout_seconds=4.0)
    settled_host = active_host_or_none(settled)
    settled_active = ((settled.get("dom") or {}).get("active_element") or {})
    if right_panel_mode(settled) != "settings":
        raise AssertionError(
            f"selection probe broke the settings-open reclaim contract by closing settings: settled={settled!r}"
        )
    if settled_host is None or settled_host.get("input_enabled") is not True:
        raise AssertionError(
            f"terminal lost input after selection probe with settings open: host={settled_host!r} settled={settled!r}"
        )
    if settled_active.get("data_settings_field_key") in {"interface-llm", "litellm-endpoint", "litellm-api-key"}:
        raise AssertionError(
            f"settings input stole focus back after selection probe: active_element={settled_active!r}"
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
        "selection": selection,
        "settled_active_element": settled_active,
        "reclaim_cleanup": reclaim_cleanup,
        "persisted_restore": persisted_restore,
        "restored_right_panel_mode": right_panel_mode(restored),
    }


def assert_settings_terminal_reclaim_contract(pid: int) -> dict:
    baseline = app_state(pid)
    if titlebar_transient_open(baseline):
        baseline = dismiss_titlebar_transients(pid, baseline, timeout_seconds=1.5)
    if right_panel_mode(baseline) not in ("", "hidden", "none", "null", "settings"):
        baseline = close_right_panel(pid, baseline, timeout_seconds=1.5)
    baseline = app_state(pid)
    button_rect = dom_rect(baseline, "titlebar_settings_button_rect")
    if not rect_is_visible(button_rect):
        raise AssertionError(f"settings button rect missing before reclaim probe: {baseline!r}")
    if right_panel_mode(baseline) != "settings":
        xdotool_click_window(pid, rect_center_x(button_rect), rect_center_y(button_rect))
        deadline = time.time() + 6.0
        while time.time() < deadline:
            baseline = app_state(pid)
            if right_panel_mode(baseline) == "settings":
                break
            if right_panel_mode(baseline) not in ("", "hidden", "none", "null"):
                raise AssertionError(
                    "titlebar settings button opened the wrong right panel before reclaim probe: "
                    f"mode={right_panel_mode(baseline)!r} state={baseline!r}"
                )
            time.sleep(0.12)
    if right_panel_mode(baseline) != "settings":
        baseline = open_settings_panel_via_command_lane(pid, timeout_seconds=6.0)
    input_rect = dom_rect(baseline, "settings_interface_llm_input_rect")
    if not rect_is_visible(input_rect):
        raise AssertionError(f"interface llm input rect missing for settings reclaim probe: {baseline!r}")
    input_click = xdotool_click_window(pid, rect_center_x(input_rect), rect_center_y(input_rect))
    focused = {}
    deadline = time.time() + 6.0
    while time.time() < deadline:
        focused = app_state(pid)
        active = ((focused.get("dom") or {}).get("active_element") or {})
        if active.get("data_settings_field_key") == "interface-llm":
            break
        time.sleep(0.12)
    focused_active = ((focused.get("dom") or {}).get("active_element") or {})
    if focused_active.get("data_settings_field_key") != "interface-llm":
        raise AssertionError(
            f"interface llm input did not take focus before reclaim probe: input_click={input_click!r} active={focused_active!r}"
        )
    focused_host = active_host_or_none(focused)
    focused_host_rect = focused_host.get("host_rect") if focused_host else None
    if not rect_is_visible(focused_host_rect):
        raise AssertionError(
            f"active terminal host rect missing before settings reclaim probe: host={focused_host!r} focused={focused!r}"
        )
    session = str(focused.get("active_session_path") or "")
    if not session:
        raise AssertionError(f"active session path missing before settings reclaim probe: focused={focused!r}")
    reclaim_click = xdotool_click_window(
        pid,
        float(focused_host_rect["left"]) + min(28.0, max(12.0, float(focused_host_rect["width"]) * 0.08)),
        float(focused_host_rect["top"]) + min(24.0, max(14.0, float(focused_host_rect["height"]) * 0.06)),
    )
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
            and reclaim_host.get("helper_textarea_focused") is True
            and reclaim_active.get("data_settings_field_key") != "interface-llm"
        ):
            break
        time.sleep(0.12)
    reclaim_active = ((reclaimed.get("dom") or {}).get("active_element") or {})
    reclaim_host = active_host_or_none(reclaimed)
    if right_panel_mode(reclaimed) != "settings":
        raise AssertionError(
            f"settings rail closed during terminal reclaim probe instead of staying open: reclaimed={reclaimed!r}"
        )
    if reclaim_host is None or reclaim_host.get("input_enabled") is not True:
        raise AssertionError(
            f"terminal input did not reclaim from settings focus: host={reclaim_host!r} reclaimed={reclaimed!r}"
        )
    if reclaim_host.get("helper_textarea_focused") is not True:
        raise AssertionError(
            f"terminal helper textarea did not reclaim focus from settings: host={reclaim_host!r} reclaimed={reclaimed!r}"
        )
    if reclaim_active.get("data_settings_field_key") == "interface-llm":
        raise AssertionError(
            f"settings input kept focus after terminal reclaim click: active={reclaim_active!r}"
        )
    selection = assert_selection(pid, session)
    settled = {}
    deadline = time.time() + 4.0
    while time.time() < deadline:
        settled = app_state(pid)
        settled_active = ((settled.get("dom") or {}).get("active_element") or {})
        settled_host = active_host_or_none(settled)
        if (
            right_panel_mode(settled) == "settings"
            and settled_host is not None
            and settled_host.get("input_enabled") is True
            and settled_host.get("helper_textarea_focused") is True
            and settled_active.get("data_settings_field_key") != "interface-llm"
        ):
            break
        time.sleep(0.12)
    settled_active = ((settled.get("dom") or {}).get("active_element") or {})
    settled_host = active_host_or_none(settled)
    if right_panel_mode(settled) != "settings":
        raise AssertionError(
            f"selection probe closed settings instead of preserving the reclaim contract: settled={settled!r}"
        )
    if settled_host is None or settled_host.get("input_enabled") is not True:
        raise AssertionError(
            f"terminal lost input after settings reclaim selection probe: host={settled_host!r} settled={settled!r}"
        )
    if settled_active.get("data_settings_field_key") == "interface-llm":
        raise AssertionError(
            f"settings input stole focus back after selection probe: active={settled_active!r}"
        )
    type_result = probe_type(pid, session, "/", mode="keyboard")
    close_result = close_right_panel(pid, settled, timeout_seconds=2.5)
    return {
        "input_click": input_click,
        "reclaim_click": reclaim_click,
        "selection": selection,
        "type_result": type_result,
        "reclaimed_active_element": reclaim_active,
        "settled_active_element": settled_active,
        "close_result_mode": right_panel_mode(close_result),
    }


def assert_right_panel_animation_contract(pid: int) -> dict:
    baseline = app_state(pid)
    if titlebar_transient_open(baseline):
        baseline = dismiss_titlebar_transients(pid, baseline, timeout_seconds=1.5)
    if right_panel_mode(baseline) not in ("", "hidden", "none", "null", "settings"):
        baseline = close_right_panel(pid, baseline, timeout_seconds=1.5)
    if right_panel_mode(baseline) == "settings":
        baseline = close_right_panel(pid, baseline, timeout_seconds=2.5)
    hidden = app_state(pid)
    hidden_transition = str(dom_value(hidden, "right_side_rail_transition") or "")
    if not all(token in hidden_transition for token in ("width", "opacity", "transform")):
        raise AssertionError(
            f"right rail hidden-state transition drifted away from width/opacity/transform animation: "
            f"transition={hidden_transition!r} state={hidden!r}"
        )
    opened = open_settings_panel_via_command_lane(pid, timeout_seconds=6.0)
    opened_transition = str(dom_value(opened, "right_side_rail_transition") or "")
    if not all(token in opened_transition for token in ("width", "opacity", "transform")):
        raise AssertionError(
            f"right rail open-state transition drifted away from width/opacity/transform animation: "
            f"transition={opened_transition!r} state={opened!r}"
        )
    opened_rect = dom_rect(opened, "right_side_rail_rect")
    if not rect_is_visible(opened_rect):
        raise AssertionError(f"right rail rect missing after opening settings: {opened!r}")
    opened_header = str(dom_value(opened, "right_side_rail_header_text") or "").strip()
    if "Settings" not in opened_header:
        raise AssertionError(
            f"right rail header drifted after opening settings: header={opened_header!r} state={opened!r}"
        )
    opened_scroll_children = int(dom_value(opened, "right_side_rail_scroll_child_count") or 0)
    if opened_scroll_children <= 0:
        raise AssertionError(
            f"right rail opened without mounted body content: children={opened_scroll_children} state={opened!r}"
        )
    app_set_right_panel_mode(pid, "hidden")
    time.sleep(RIGHT_PANEL_EXIT_ANIMATION_SAMPLE_SECONDS)
    exiting = app_state(pid)
    if right_panel_mode(exiting) not in ("", "hidden", "none", "null"):
        raise AssertionError(f"right rail close request did not switch shell mode to hidden: {exiting!r}")
    exiting_transition = str(dom_value(exiting, "right_side_rail_transition") or "")
    if not all(token in exiting_transition for token in ("width", "opacity", "transform")):
        raise AssertionError(
            f"right rail exit transition drifted away from width/opacity/transform animation: "
            f"transition={exiting_transition!r} state={exiting!r}"
        )
    exiting_header = str(dom_value(exiting, "right_side_rail_header_text") or "").strip()
    exiting_scroll_children = int(dom_value(exiting, "right_side_rail_scroll_child_count") or 0)
    exiting_scroll_rect = dom_rect(exiting, "right_side_rail_scroll_rect")
    exiting_settings_rect = dom_rect(exiting, "settings_interface_llm_input_rect")
    if (
        exiting_scroll_children <= 0
        and not rect_is_visible(exiting_scroll_rect)
        and not rect_is_visible(exiting_settings_rect)
    ):
        raise AssertionError(
            "right rail body disappeared immediately instead of animating out: "
            f"header={exiting_header!r} children={exiting_scroll_children} scroll_rect={exiting_scroll_rect!r} "
            f"settings_rect={exiting_settings_rect!r} state={exiting!r}"
        )
    time.sleep(RIGHT_PANEL_EXIT_ANIMATION_SETTLE_SECONDS)
    settled = app_state(pid)
    settled_header = str(dom_value(settled, "right_side_rail_header_text") or "").strip()
    settled_scroll_children = int(dom_value(settled, "right_side_rail_scroll_child_count") or 0)
    settled_settings_rect = dom_rect(settled, "settings_interface_llm_input_rect")
    settled_visible_attr = str(dom_value(settled, "right_side_rail_visible_attr") or "").strip()
    settled_pointer_events = str(dom_value(settled, "right_side_rail_pointer_events") or "").strip()
    settled_opacity = str(dom_value(settled, "right_side_rail_opacity") or "").strip()
    if right_panel_mode(settled) not in ("", "hidden", "none", "null"):
        raise AssertionError(f"right rail did not stay hidden after exit settle window: {settled!r}")
    if (
        rect_is_visible(settled_settings_rect)
        or settled_visible_attr != "0"
        or settled_pointer_events != "none"
        or settled_opacity not in {"0", "0.0", "0.00"}
    ):
        raise AssertionError(
            "right rail did not finish the hidden non-interactive exit contract: "
            f"header={settled_header!r} children={settled_scroll_children} settings_rect={settled_settings_rect!r} "
            f"visible_attr={settled_visible_attr!r} pointer_events={settled_pointer_events!r} "
            f"opacity={settled_opacity!r} state={settled!r}"
        )
    return {
        "hidden_transition": hidden_transition,
        "opened_transition": opened_transition,
        "opened_header": opened_header,
        "opened_rect": opened_rect,
        "opened_scroll_children": opened_scroll_children,
        "exiting_transition": exiting_transition,
        "exiting_header": exiting_header,
        "exiting_scroll_children": exiting_scroll_children,
        "exiting_scroll_rect": exiting_scroll_rect,
        "settled_header": settled_header,
        "settled_scroll_children": settled_scroll_children,
        "settled_visible_attr": settled_visible_attr,
        "settled_pointer_events": settled_pointer_events,
        "settled_opacity": settled_opacity,
    }


def assert_titlebar_autohide_hover_contract(pid: int, out_dir: Path) -> dict:
    baseline = app_state(pid)
    if titlebar_transient_open(baseline):
        baseline = dismiss_titlebar_transients(pid, baseline, timeout_seconds=1.5)
    baseline = ensure_unmaximized_window(pid, baseline, timeout_seconds=6.0)
    baseline_panel_mode = right_panel_mode(baseline)
    if baseline_panel_mode not in ("", "hidden", "none", "null", "settings"):
        baseline = close_right_panel(pid, baseline, timeout_seconds=1.5)
        baseline_panel_mode = right_panel_mode(baseline)
    if baseline_panel_mode != "settings":
        baseline = open_settings_panel_via_command_lane(pid, timeout_seconds=6.0)
    toggle_rect = dom_rect(baseline, "settings_titlebar_auto_hide_toggle_rect")
    if not rect_is_visible(toggle_rect):
        raise AssertionError(f"titlebar auto-hide toggle rect missing in settings rail: {baseline!r}")
    toggle_hit_target = dom_value(baseline, "settings_titlebar_auto_hide_toggle_hit_target") or {}
    toggle_text = str(dom_value(baseline, "settings_titlebar_auto_hide_toggle_text") or "").strip()
    if toggle_text not in {"On", "Off"}:
        raise AssertionError(
            f"titlebar auto-hide toggle text is missing or drifted from the expected On/Off state: "
            f"text={toggle_text!r} state={baseline!r}"
        )
    baseline_enabled = bool(dom_value(baseline, "settings_titlebar_auto_hide_toggle_enabled"))
    enable_click = None
    if not baseline_enabled:
        enable_click = xdotool_click_window(
            pid,
            rect_center_x(toggle_rect),
            rect_center_y(toggle_rect),
        )
    enabled_state = wait_for_titlebar_autohide_state(
        pid,
        enabled=True,
        toggle_enabled=True,
        timeout_seconds=6.0,
    )
    settle_rect = dom_rect(enabled_state, "main_surface_body_rect") or dom_rect(enabled_state, "sidebar_rect")
    if not rect_is_visible(settle_rect):
        raise AssertionError(f"main surface rect missing before titlebar auto-hide collapse probe: {enabled_state!r}")
    if POINTER_DRIVER == "app":
        collapse_move = app_set_window_chrome_hover(pid, False)
    else:
        collapse_move = xdotool_move_window(
            pid,
            rect_center_x(settle_rect),
            rect_center_y(settle_rect),
        )
    collapsed = wait_for_titlebar_autohide_state(
        pid,
        enabled=True,
        revealed=False,
        hover_active=False,
        toggle_enabled=True,
        timeout_seconds=6.0,
    )
    collapsed_rect = dom_rect(collapsed, "titlebar_rect")
    if not rect_is_visible(collapsed_rect):
        raise AssertionError(f"titlebar rect disappeared instead of collapsing to a hover strip: {collapsed!r}")
    if float(collapsed_rect["height"]) > TITLEBAR_AUTOHIDE_SENSOR_HEIGHT_MAX_PX:
        raise AssertionError(
            f"titlebar auto-hide collapse left too much height instead of the hover strip: rect={collapsed_rect!r}"
        )
    collapsed_balance = shell_content_vertical_balance(collapsed)
    if collapsed_balance["gap_delta"] > TITLEBAR_AUTOHIDE_CONTENT_BALANCE_MAX_PX:
        raise AssertionError(
            "titlebar auto-hide collapse still biases the shell content vertically instead of overlaying it: "
            f"balance={collapsed_balance!r}"
        )
    app_screenshot(pid, out_dir / "titlebar-autohide-collapsed.png")
    hover_y = float(collapsed_rect["top"]) + min(
        max(1.0, float(collapsed_rect["height"]) / 2.0),
        max(1.0, float(collapsed_rect["height"]) - 1.0),
    )
    if POINTER_DRIVER == "app":
        hover_move = app_set_window_chrome_hover(pid, True)
    else:
        hover_move = xdotool_move_window(pid, rect_center_x(collapsed_rect), hover_y)
    revealed = wait_for_titlebar_autohide_state(
        pid,
        enabled=True,
        revealed=True,
        hover_active=True,
        toggle_enabled=True,
        timeout_seconds=6.0,
    )
    revealed_rect = dom_rect(revealed, "titlebar_rect")
    if float(revealed_rect.get("height") or 0.0) < TITLEBAR_VISIBLE_MIN_HEIGHT_PX:
        raise AssertionError(
            f"titlebar hover reveal did not restore the full chrome lane: rect={revealed_rect!r}"
        )
    search_rect = dom_rect(revealed, "titlebar_search_input_rect")
    if not rect_is_visible(search_rect):
        raise AssertionError(
            f"titlebar hover reveal brought the lane back but search input stayed hidden: state={revealed!r}"
        )
    revealed_balance = shell_content_vertical_balance(revealed)
    if revealed_balance["gap_delta"] > TITLEBAR_AUTOHIDE_CONTENT_BALANCE_MAX_PX:
        raise AssertionError(
            "titlebar hover reveal biased the shell content vertically instead of preserving the overlay contract: "
            f"balance={revealed_balance!r}"
        )
    revealed_titlebar_centering = assert_titlebar_centering(revealed)
    app_screenshot(pid, out_dir / "titlebar-autohide-revealed.png")
    (
        revealed_drag_x,
        revealed_drag_y,
        revealed_drag_reason,
    ) = titlebar_empty_lane_point(revealed)
    reveal_double_click = xdotool_double_click_window(
        pid,
        revealed_drag_x,
        revealed_drag_y,
    )
    maximized = wait_for_window_maximized(pid, True)
    if maximized is None:
        raise AssertionError(
            "titlebar hover reveal restored the chrome visually but double-click still did not maximize the window: "
            f"point={revealed_drag_reason} click={reveal_double_click!r}"
        )
    maximized = wait_for_window_focus(pid, timeout_seconds=4.0)
    maximized_x, maximized_y, maximized_reason = titlebar_empty_lane_point(maximized)
    reveal_restore_double_click = xdotool_double_click_window(
        pid,
        maximized_x,
        maximized_y,
    )
    restored_after_maximize = wait_for_window_maximized(pid, False)
    if restored_after_maximize is None:
        raise AssertionError(
            "titlebar hover reveal double-click maximized the window but did not restore it on the second double-click"
        )
    settle_rect = dom_rect(restored_after_maximize, "main_surface_body_rect") or dom_rect(
        restored_after_maximize, "sidebar_rect"
    )
    if not rect_is_visible(settle_rect):
        raise AssertionError(
            f"main surface rect missing before restored auto-hide collapse probe: {restored_after_maximize!r}"
        )
    if POINTER_DRIVER == "app":
        app_set_window_chrome_hover(pid, False)
    else:
        xdotool_move_window(
            pid,
            rect_center_x(settle_rect),
            rect_center_y(settle_rect),
        )
    collapsed_after_restore = wait_for_titlebar_autohide_state(
        pid,
        enabled=True,
        revealed=False,
        hover_active=False,
        toggle_enabled=True,
        timeout_seconds=6.0,
    )
    collapsed_after_restore_rect = dom_rect(collapsed_after_restore, "titlebar_rect")
    if not rect_is_visible(collapsed_after_restore_rect):
        raise AssertionError(
            f"titlebar rect disappeared after restore instead of collapsing back to the hover strip: {collapsed_after_restore!r}"
        )
    hover_y = float(collapsed_after_restore_rect["top"]) + min(
        max(1.0, float(collapsed_after_restore_rect["height"]) / 2.0),
        max(1.0, float(collapsed_after_restore_rect["height"]) - 1.0),
    )
    if POINTER_DRIVER == "app":
        app_set_window_chrome_hover(pid, True)
    else:
        xdotool_move_window(pid, rect_center_x(collapsed_after_restore_rect), hover_y)
    revealed = wait_for_titlebar_autohide_state(
        pid,
        enabled=True,
        revealed=True,
        hover_active=True,
        toggle_enabled=True,
        timeout_seconds=6.0,
    )
    (
        restored_drag_x,
        restored_drag_y,
        restored_drag_reason,
    ) = titlebar_empty_lane_point(revealed)
    drag_before = window_outer_geometry(revealed)
    reveal_drag = titlebar_drag_window(
        pid,
        restored_drag_x,
        restored_drag_y,
        restored_drag_x + 96.0,
        restored_drag_y + 48.0,
    )
    drag_delta = None
    if POINTER_DRIVER == "app":
        dragged = wait_for_titlebar_drag_request(
            pid,
            titlebar_drag_request_count(revealed),
            timeout_seconds=2.5,
        )
        drag_after = window_outer_geometry(dragged)
        drag_delta = (drag_after[0] - drag_before[0], drag_after[1] - drag_before[1])
    else:
        dragged = wait_for_window_geometry_settle(pid, timeout_seconds=6.0)
        drag_after = window_outer_geometry(dragged)
        drag_delta = (drag_after[0] - drag_before[0], drag_after[1] - drag_before[1])
        if abs(drag_delta[0]) < 40 and abs(drag_delta[1]) < 20:
            raise AssertionError(
                f"titlebar hover reveal restored the chrome visually but the empty lane still did not drag the window: "
                f"point={restored_drag_reason} before={drag_before!r} after={drag_after!r} drag={reveal_drag!r}"
            )
    if POINTER_DRIVER == "app":
        reveal_drag_restore = None
        revealed = dragged
    else:
        dragged_point_x, dragged_point_y, _ = titlebar_empty_lane_point(dragged)
        reveal_drag_restore = titlebar_drag_window(
            pid,
            dragged_point_x,
            dragged_point_y,
            dragged_point_x - 96.0,
            dragged_point_y - 48.0,
        )
        revealed = wait_for_window_geometry_settle(pid, timeout_seconds=6.0)
    drag_restored = window_outer_geometry(revealed)
    if POINTER_DRIVER != "app" and (
        abs(drag_restored[0] - drag_before[0]) > 28
        or abs(drag_restored[1] - drag_before[1]) > 28
    ):
        raise AssertionError(
            f"titlebar hover reveal drag moved the window but did not restore cleanly for the rest of the probe: "
            f"before={drag_before!r} restored={drag_restored!r} restore_drag={reveal_drag_restore!r}"
        )
    search_pin = app_set_search(pid, "titlebar autohide smoke", focused=True)
    if POINTER_DRIVER == "app":
        pin_move = app_set_window_chrome_hover(pid, False)
    else:
        pin_move = xdotool_move_window(
            pid,
            rect_center_x(settle_rect),
            rect_center_y(settle_rect),
        )
    pinned = wait_for_titlebar_autohide_state(
        pid,
        enabled=True,
        revealed=True,
        hover_active=False,
        toggle_enabled=True,
        timeout_seconds=6.0,
    )
    pinned_rect = dom_rect(pinned, "titlebar_rect")
    if float(pinned_rect.get("height") or 0.0) < TITLEBAR_VISIBLE_MIN_HEIGHT_PX:
        raise AssertionError(
            f"titlebar did not stay pinned open while search owned it: rect={pinned_rect!r} state={pinned!r}"
        )
    clear_search = app_set_search(pid, "", focused=False)
    if POINTER_DRIVER == "app":
        clear_move = app_set_window_chrome_hover(pid, False)
    else:
        clear_move = xdotool_move_window(
            pid,
            rect_center_x(settle_rect),
            rect_center_y(settle_rect),
        )
    repacked = wait_for_titlebar_autohide_state(
        pid,
        enabled=True,
        revealed=False,
        hover_active=False,
        toggle_enabled=True,
        timeout_seconds=6.0,
    )
    disable_click = None
    restored_setting_state = repacked
    if not baseline_enabled:
        current_toggle_rect = dom_rect(repacked, "settings_titlebar_auto_hide_toggle_rect")
        if not rect_is_visible(current_toggle_rect):
            raise AssertionError(
                f"titlebar auto-hide toggle rect disappeared before restore: state={repacked!r}"
            )
        disable_click = xdotool_click_window(
            pid,
            rect_center_x(current_toggle_rect),
            rect_center_y(current_toggle_rect),
        )
        restored_setting_state = wait_for_titlebar_autohide_state(
            pid,
            enabled=False,
            revealed=True,
            toggle_enabled=False,
            timeout_seconds=6.0,
        )
    restored_panel_mode = right_panel_mode(restored_setting_state)
    if baseline_panel_mode != "settings":
        restored_setting_state = close_right_panel(pid, restored_setting_state, timeout_seconds=2.5)
        restored_panel_mode = right_panel_mode(restored_setting_state)
    return {
        "baseline_panel_mode": baseline_panel_mode,
        "baseline_enabled": baseline_enabled,
        "toggle_rect": toggle_rect,
        "toggle_hit_target": toggle_hit_target,
        "toggle_text": toggle_text,
        "enable_click": enable_click,
        "collapse_move": collapse_move,
        "collapsed_rect": collapsed_rect,
        "collapsed_balance": collapsed_balance,
        "hover_move": hover_move,
        "revealed_rect": revealed_rect,
        "revealed_balance": revealed_balance,
        "revealed_titlebar_centering": revealed_titlebar_centering,
        "revealed_drag_point": {
            "x": round(revealed_drag_x, 2),
            "y": round(revealed_drag_y, 2),
            "reason": revealed_drag_reason,
        },
        "reveal_double_click": reveal_double_click,
        "reveal_maximized_point": {
            "x": round(maximized_x, 2),
            "y": round(maximized_y, 2),
            "reason": maximized_reason,
        },
        "reveal_restore_double_click": reveal_restore_double_click,
        "reveal_drag": reveal_drag,
        "reveal_drag_restore": reveal_drag_restore,
        "reveal_drag_delta": drag_delta,
        "search_pin": search_pin,
        "pin_move": pin_move,
        "pinned_rect": pinned_rect,
        "clear_search": clear_search,
        "clear_move": clear_move,
        "disable_click": disable_click,
        "restored_panel_mode": restored_panel_mode,
    }


def assert_titlebar_empty_lane_window_controls_contract(pid: int, out_dir: Path) -> dict:
    app_set_search(pid, "", focused=False)
    time.sleep(0.1)
    baseline = wait_for_window_focus(pid, timeout_seconds=4.0)
    if titlebar_transient_open(baseline):
        baseline = dismiss_titlebar_transients(pid, baseline, timeout_seconds=1.5)
    baseline = ensure_unmaximized_window(pid, baseline, timeout_seconds=6.0)
    if right_panel_mode(baseline) not in ("", "hidden", "none", "null", "settings"):
        baseline = close_right_panel(pid, baseline, timeout_seconds=1.5)
    if bool((baseline.get("shell") or {}).get("titlebar_auto_hide_enabled")):
        if right_panel_mode(baseline) != "settings":
            baseline = open_settings_panel_via_command_lane(pid, timeout_seconds=6.0)
        toggle_rect = dom_rect(baseline, "settings_titlebar_auto_hide_toggle_rect")
        if not rect_is_visible(toggle_rect):
            raise AssertionError(
                f"titlebar auto-hide toggle rect missing while restoring the visible titlebar lane: {baseline!r}"
            )
        xdotool_click_window(pid, rect_center_x(toggle_rect), rect_center_y(toggle_rect))
        baseline = wait_for_titlebar_autohide_state(
            pid,
            enabled=False,
            revealed=True,
            toggle_enabled=False,
            timeout_seconds=6.0,
        )
    if right_panel_mode(baseline) == "settings":
        baseline = close_right_panel(pid, baseline, timeout_seconds=2.5)
    baseline = ensure_unmaximized_window(pid, baseline, timeout_seconds=6.0)
    baseline = wait_for_window_geometry_settle(pid, timeout_seconds=6.0)
    empty_x, empty_y, empty_reason = titlebar_empty_lane_point(baseline)
    before_geometry = window_outer_geometry(baseline)
    before_shot = out_dir / "titlebar-empty-lane-before.png"
    app_screenshot(pid, before_shot)

    double_click = xdotool_double_click_window(pid, empty_x, empty_y)
    maximized = wait_for_window_maximized(pid, True)
    if maximized is None:
        raise AssertionError(
            f"titlebar empty lane still does not maximize the window on double-click: "
            f"reason={empty_reason} click={double_click!r}"
        )
    maximized = wait_for_window_focus(pid, timeout_seconds=4.0)
    maximized_x, maximized_y, maximized_reason = titlebar_empty_lane_point(maximized)
    restore_double_click = xdotool_double_click_window(pid, maximized_x, maximized_y)
    restored = wait_for_window_maximized(pid, False)
    if restored is None:
        raise AssertionError("titlebar double-click maximized the window but did not restore it")
    baseline = wait_for_window_geometry_settle(pid, timeout_seconds=6.0)

    drag_x, drag_y, drag_reason = titlebar_empty_lane_point(baseline)
    drag = titlebar_drag_window(pid, drag_x, drag_y, drag_x + 96.0, drag_y + 48.0)
    drag_delta = None
    if POINTER_DRIVER == "app":
        moved = wait_for_titlebar_drag_request(
            pid,
            titlebar_drag_request_count(baseline),
            timeout_seconds=2.5,
        )
        moved_geometry = window_outer_geometry(moved)
        drag_delta = (moved_geometry[0] - before_geometry[0], moved_geometry[1] - before_geometry[1])
        restore_drag = None
        restored = moved
        restored_geometry = moved_geometry
    else:
        moved = wait_for_window_geometry_settle(pid, timeout_seconds=6.0)
        moved_geometry = window_outer_geometry(moved)
        drag_delta = (moved_geometry[0] - before_geometry[0], moved_geometry[1] - before_geometry[1])
        if abs(drag_delta[0]) < 40 and abs(drag_delta[1]) < 20:
            raise AssertionError(
                f"titlebar empty lane still does not drag the window in the visible chrome state: "
                f"reason={drag_reason} before={before_geometry!r} after={moved_geometry!r} drag={drag!r}"
            )
        moved_x, moved_y, _ = titlebar_empty_lane_point(moved)
        restore_drag = titlebar_drag_window(pid, moved_x, moved_y, moved_x - 96.0, moved_y - 48.0)
        restored = wait_for_window_geometry_settle(pid, timeout_seconds=6.0)
        restored_geometry = window_outer_geometry(restored)
        if abs(restored_geometry[0] - before_geometry[0]) > 28 or abs(restored_geometry[1] - before_geometry[1]) > 28:
            raise AssertionError(
                f"titlebar drag moved the window but did not restore within tolerance: "
                f"before={before_geometry!r} restored={restored_geometry!r} restore_drag={restore_drag!r}"
            )
    after_shot = out_dir / "titlebar-empty-lane-after.png"
    app_screenshot(pid, after_shot)
    return {
        "empty_lane_point": {
            "x": round(empty_x, 2),
            "y": round(empty_y, 2),
            "reason": empty_reason,
        },
        "before_geometry": before_geometry,
        "double_click": double_click,
        "maximized_point": {
            "x": round(maximized_x, 2),
            "y": round(maximized_y, 2),
            "reason": maximized_reason,
        },
        "restore_double_click": restore_double_click,
        "drag_point": {
            "x": round(drag_x, 2),
            "y": round(drag_y, 2),
            "reason": drag_reason,
        },
        "drag": drag,
        "drag_delta": drag_delta,
        "restore_drag": restore_drag,
        "restored_geometry": restored_geometry,
        "before_screenshot": str(before_shot),
        "after_screenshot": str(after_shot),
    }


def assert_theme_editor_contract(pid: int, out_dir: Path) -> dict:
    default_grain = 0.12
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
            if right_panel_mode(opened) not in ("", "hidden", "none", "null"):
                raise AssertionError(
                    "titlebar settings button opened the wrong right panel before theme-editor probe: "
                    f"mode={right_panel_mode(opened)!r} state={opened!r}"
                )
            time.sleep(0.12)
        if right_panel_mode(baseline) != "settings":
            baseline = open_settings_panel_via_command_lane(pid, timeout_seconds=6.0)
    update_cta = (((baseline.get("shell") or {}).get("update_call_to_action")) or {})
    update_button_rect = dom_rect(baseline, "install_update_button_rect")
    update_button_text = str(dom_value(baseline, "install_update_button_text") or "").strip()
    update_detail_text = str(dom_value(baseline, "install_update_detail_text") or "").strip()
    update_mode = str(dom_value(baseline, "install_update_button_mode") or "").strip()
    if not rect_is_visible(update_button_rect):
        raise AssertionError(f"install update button rect missing in settings rail: {baseline!r}")
    if not update_button_text:
        raise AssertionError(f"install update button label missing: {baseline!r}")
    if update_mode != str(update_cta.get("mode") or "").strip():
        raise AssertionError(
            f"install update button mode drifted from shell state: dom={update_mode!r} shell={update_cta!r}"
        )
    if update_cta.get("label") and update_button_text != str(update_cta["label"]).strip():
        raise AssertionError(
            f"install update button label drifted from shell state: dom={update_button_text!r} shell={update_cta!r}"
        )
    if update_cta.get("detail") and update_detail_text != str(update_cta["detail"]).strip():
        raise AssertionError(
            f"install update detail drifted from shell state: dom={update_detail_text!r} shell={update_cta!r}"
        )
    edit_button_rect = dom_rect(baseline, "theme_editor_open_button_rect")
    if not rect_is_visible(edit_button_rect):
        raise AssertionError(f"theme editor open button rect missing: {baseline!r}")
    open_result = run(
        "server",
        "app",
        "theme-editor",
        "open",
        "--pid",
        str(pid),
        "--timeout-ms",
        "8000",
    )
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
    grain_rect = dom_rect(opened, "theme_editor_grain_input_rect")
    if not rect_is_visible(theme_shell_rect):
        raise AssertionError(
            f"theme editor did not open after deterministic app-control open: open_result={open_result!r} state={opened!r}"
        )
    reset_result = run(
        "server",
        "app",
        "theme-editor",
        "reset",
        "--pid",
        str(pid),
        "--timeout-ms",
        "8000",
    )
    deadline = time.time() + 6.0
    restored = {}
    while time.time() < deadline:
        restored = app_state(pid)
        restored_grain = theme_grain_value(shell_theme_spec(restored, "saved_yggui_theme"))
        if rect_is_visible(dom_rect(restored, "theme_editor_shell_rect")) and restored_grain is not None and abs(restored_grain - default_grain) <= 0.01:
            opened = restored
            break
        time.sleep(0.12)
    restored_grain = theme_grain_value(shell_theme_spec(opened, "saved_yggui_theme"))
    if restored_grain is None or abs(restored_grain - default_grain) > 0.01:
        raise AssertionError(
            f"theme editor reset did not restore default grain before edit probe: reset={reset_result!r} state={opened!r}"
        )
    shell_frame_before = {
        "background": dom_value(opened, "shell_frame_background"),
        "background_image": dom_value(opened, "shell_frame_background_image"),
        "box_shadow": dom_value(opened, "shell_frame_box_shadow"),
        "border_radius": dom_value(opened, "shell_frame_border_radius"),
    }
    saved_theme_before = shell_theme_spec(opened, "saved_yggui_theme")
    theme_shell_rect = dom_rect(opened, "theme_editor_shell_rect")
    apply_rect = dom_rect(opened, "theme_editor_apply_button_rect")
    reset_rect = dom_rect(opened, "theme_editor_reset_button_rect")
    seed_rect = dom_rect(opened, "theme_editor_seed_button_rect")
    grain_rect = dom_rect(opened, "theme_editor_grain_input_rect")
    if rect_is_visible(apply_rect):
        raise AssertionError(f"theme editor still exposes an Apply button: {apply_rect!r}")
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
    reset_button_background = str(dom_value(opened, "theme_editor_reset_button_background") or "")
    reset_button_shadow = str(dom_value(opened, "theme_editor_reset_button_box_shadow") or "")
    reset_button_radius = str(dom_value(opened, "theme_editor_reset_button_border_radius") or "")
    if not reset_button_background or not reset_button_shadow or not reset_button_radius:
        raise AssertionError(
            "theme editor reset button lost its styled button contract: "
            f"background={reset_button_background!r} shadow={reset_button_shadow!r} radius={reset_button_radius!r}"
        )
    if not rect_is_visible(grain_rect):
        raise AssertionError(f"theme editor grain input rect missing: {opened!r}")
    before_grain = theme_grain_value(saved_theme_before)
    grain_target_ratio = 0.96 if before_grain is None or before_grain < 0.72 else 0.08
    grain_click = xdotool_click_window(
        pid,
        float(grain_rect.get("left") or 0.0) + float(grain_rect.get("width") or 0.0) * grain_target_ratio,
        float(grain_rect.get("top") or 0.0) + float(grain_rect.get("height") or 0.0) * 0.24,
    )
    deadline = time.time() + 6.0
    changed = {}
    while time.time() < deadline:
        changed = app_state(pid)
        saved_theme_after_change = shell_theme_spec(changed, "saved_yggui_theme")
        effective_theme_after_change = shell_theme_spec(changed, "effective_yggui_theme")
        changed_grain = theme_grain_value(saved_theme_after_change)
        changed_shell_frame_background_image = dom_value(changed, "shell_frame_background_image")
        if (
            changed_grain is not None
            and before_grain is not None
            and abs(changed_grain - before_grain) >= 0.05
            and abs(changed_grain - (theme_grain_value(effective_theme_after_change) or changed_grain)) <= 0.005
            and changed_shell_frame_background_image != shell_frame_before["background_image"]
        ):
            break
        time.sleep(0.12)
    saved_theme_after_change = shell_theme_spec(changed, "saved_yggui_theme")
    effective_theme_after_change = shell_theme_spec(changed, "effective_yggui_theme")
    changed_grain = theme_grain_value(saved_theme_after_change)
    if (
        changed_grain is None
        or before_grain is None
        or abs(changed_grain - before_grain) < 0.05
    ):
        raise AssertionError(
            f"theme editor grain move did not auto-apply to saved theme: before={saved_theme_before!r} after={saved_theme_after_change!r}"
        )
    shell_frame_after_change = {
        "background": dom_value(changed, "shell_frame_background"),
        "background_image": dom_value(changed, "shell_frame_background_image"),
        "box_shadow": dom_value(changed, "shell_frame_box_shadow"),
        "border_radius": dom_value(changed, "shell_frame_border_radius"),
    }
    if shell_frame_after_change["background_image"] == shell_frame_before["background_image"]:
        raise AssertionError(
            "theme editor grain move did not change the live shell frame background image: "
            f"before={shell_frame_before!r} after={shell_frame_after_change!r}"
        )
    if shell_frame_after_change["box_shadow"] != shell_frame_before["box_shadow"]:
        raise AssertionError(
            f"shell frame shadow drifted after live theme edit: before={shell_frame_before!r} after={shell_frame_after_change!r}"
        )
    if shell_frame_after_change["border_radius"] != shell_frame_before["border_radius"]:
        raise AssertionError(
            f"shell frame border radius drifted after live theme edit: before={shell_frame_before!r} after={shell_frame_after_change!r}"
        )
    reset_result = run(
        "server",
        "app",
        "theme-editor",
        "reset",
        "--pid",
        str(pid),
        "--timeout-ms",
        "8000",
    )
    deadline = time.time() + 6.0
    restored = {}
    while time.time() < deadline:
        restored = app_state(pid)
        restored_grain = theme_grain_value(shell_theme_spec(restored, "saved_yggui_theme"))
        if restored_grain is not None and abs(restored_grain - default_grain) <= 0.01:
            break
        time.sleep(0.12)
    restored_grain = theme_grain_value(shell_theme_spec(restored, "saved_yggui_theme"))
    if restored_grain is None or abs(restored_grain - default_grain) > 0.01:
        raise AssertionError(
            f"theme editor reset did not restore default grain: restored={shell_theme_spec(restored, 'saved_yggui_theme')!r}"
        )
    dismiss_click = xdotool_click_window(
        pid,
        max(8.0, float(theme_shell_rect.get("left") or 0.0) - 18.0),
        max(8.0, float(theme_shell_rect.get("top") or 0.0) + 12.0),
    )
    deadline = time.time() + 6.0
    closed = {}
    while time.time() < deadline:
        closed = app_state(pid)
        if not rect_is_visible(dom_rect(closed, "theme_editor_shell_rect")):
            break
        time.sleep(0.12)
    if rect_is_visible(dom_rect(closed, "theme_editor_shell_rect")):
        raise AssertionError(f"theme editor did not close after outside click: click={dismiss_click!r} state={closed!r}")
    shot_path = out_dir / "theme-editor-live.png"
    app_screenshot(pid, shot_path)
    return {
        "open_result": open_result,
        "grain_click": grain_click,
        "reset_result": reset_result,
        "dismiss_click": dismiss_click,
        "shell_rect": theme_shell_rect,
        "shell_background": shell_background,
        "shell_box_shadow": shell_shadow,
        "reset_button_background": reset_button_background,
        "reset_button_box_shadow": reset_button_shadow,
        "reset_button_border_radius": reset_button_radius,
        "saved_theme_before": saved_theme_before,
        "saved_theme_after_change": saved_theme_after_change,
        "effective_theme_after_change": effective_theme_after_change,
        "shell_frame_before": shell_frame_before,
        "shell_frame_after_change": shell_frame_after_change,
        "screenshot": str(shot_path),
    }


def assert_clipboard_image_contract(pid: int, session: str, out_dir: Path) -> dict:
    def refocus_terminal_surface() -> dict:
        state = focus_terminal_helper_textarea(pid, session, timeout_seconds=8.0)
        host = host_for_session_or_none(state, session) or active_host_or_none(state) or {}
        active_element = ((state.get("dom") or {}).get("active_element") or {})
        if str(active_element.get("tag") or "") != "textarea":
            raise AssertionError(
                f"terminal helper focus contract did not land on the xterm textarea: active_element={active_element!r} state={state!r}"
            )
        focus_rect = host.get("host_rect") or dom_rect(state, "main_surface_body_rect")
        return {
            "focused_state_active_tag": active_element.get("tag"),
            "host_rect": focus_rect,
        }

    wait_for_session_focus(pid, session, timeout_seconds=12.0)
    clear_terminal_selection(pid, session, timeout_seconds=4.0)
    wait_for_terminal_quiescent(pid, timeout_seconds=8.0)
    clear_probe = clear_prompt_line(pid, session, timeout_seconds=8.0)
    initial = clear_probe["state"]
    failure_owner = None
    success_owner = None
    try:
        refocus_terminal_surface()
        failure_owner = set_clipboard_text_for_pid(pid, "not-a-png")
        time.sleep(0.25)
        text_focus = refocus_terminal_surface()
        time.sleep(0.2)
        failure_key = terminal_paste_clipboard(pid, session)
        time.sleep(0.8)
        deadline = time.time() + 6.0
        text_state = {}
        while time.time() < deadline:
            text_state = app_state(pid)
            text_notifications = [
                notification
                for notification in visible_notifications(text_state)
                if str(notification.get("title") or "") == "Image Paste Failed"
            ]
            text_host = host_for_session_or_none(text_state, session) or active_host_or_none(text_state) or {}
            text_line = str(text_host.get("cursor_line_text") or text_host.get("cursor_row_text") or "")
            if "not-a-png" in text_line:
                break
            time.sleep(0.15)
        text_notifications = [
            notification
            for notification in visible_notifications(text_state)
            if str(notification.get("title") or "") == "Image Paste Failed"
        ]
        failure_host = host_for_session_or_none(text_state, session) or active_host(text_state)
        polluted_samples = [
            str(failure_host.get("text_sample") or ""),
            str(failure_host.get("cursor_line_text") or ""),
            str(failure_host.get("cursor_row_text") or ""),
        ]
        if not any("not-a-png" in sample for sample in polluted_samples):
            raise AssertionError(
                f"clipboard text paste never reached the prompt row: samples={polluted_samples!r} state={text_state!r}"
            )
        if any("Failed to paste image:" in sample for sample in polluted_samples):
            raise AssertionError(
                "clipboard image failure leaked into the PTY text surface: "
                f"samples={polluted_samples!r} notifications={text_notifications!r}"
            )
        if text_notifications:
            raise AssertionError(
                f"ordinary text paste incorrectly surfaced an image-paste error: {text_notifications!r}"
            )
        stale_remote_failures = [
            notification
            for notification in visible_notifications(text_state)
            if str(notification.get("title") or "") == "Remote Terminal Failed"
        ]
        if stale_remote_failures:
            raise AssertionError(
                f"clipboard text paste left a stale remote-terminal error visible: {stale_remote_failures!r}"
            )
        stop_clipboard_owner(failure_owner)
        failure_owner = None
        time.sleep(0.2)
        clear_prompt_line(pid, session, timeout_seconds=6.0)

        success_focus = refocus_terminal_surface()
        success_owner = set_clipboard_png_for_pid(pid)
        time.sleep(0.25)
        success_key = terminal_paste_clipboard(pid, session)
        deadline = time.time() + 6.0
        success_state = {}
        success_notification = None
        success_line = ""
        while time.time() < deadline:
            success_state = app_state(pid)
            current_success_notifications = [
                notification
                for notification in visible_notifications(success_state)
                if str(notification.get("title") or "") == "Image Staged"
            ]
            if current_success_notifications:
                success_notification = current_success_notifications[-1]
            success_host = (
                host_for_session_or_none(success_state, session)
                or active_host_or_none(success_state)
                or {}
            )
            current_success_line = str(
                success_host.get("cursor_line_text") or success_host.get("cursor_row_text") or ""
            )
            if ".png" in current_success_line:
                success_line = current_success_line
            if success_notification and success_line:
                break
            time.sleep(0.15)
        success_host = host_for_session_or_none(success_state, session) or active_host(success_state)
        if not success_line:
            success_line = str(success_host.get("cursor_line_text") or success_host.get("cursor_row_text") or "")
        if success_notification is None:
            raise AssertionError(f"clipboard image success notification never surfaced: {success_state!r}")
        if ".png" not in success_line:
            raise AssertionError(
                f"clipboard image success did not paste a png path into the prompt row: line={success_line!r} state={success_state!r}"
            )
        if "Failed to paste image:" in str(success_host.get("text_sample") or ""):
            raise AssertionError(
                f"clipboard image success left stale failure text in the terminal buffer: {success_host!r}"
            )
        shot_path = out_dir / "clipboard-image.png"
        app_screenshot(pid, shot_path)
        cleanup_probe = clear_prompt_line(pid, session, timeout_seconds=6.0)
        return {
            "clear_probe": clear_probe,
            "initial_state": initial.get("active_session_path"),
            "text_focus": text_focus,
            "text_key": failure_key,
            "text_notification": text_notifications[-1] if text_notifications else None,
            "success_focus": success_focus,
            "success_key": success_key,
            "success_notification": success_notification,
            "success_cursor_line": success_line,
            "cleanup_probe": cleanup_probe,
            "screenshot": str(shot_path),
        }
    finally:
        stop_clipboard_owner(failure_owner)
        stop_clipboard_owner(success_owner)


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


def assert_maximize_roundtrip_layout(pid: int, out_dir: Path) -> dict:
    app_set_search(pid, "", focused=False)
    time.sleep(0.1)
    baseline = wait_for_window_focus(pid, timeout_seconds=4.0)
    if shell_context_menu_row_path(baseline):
        try:
            xdotool_key_window(pid, "Escape")
        except Exception:
            pass
        time.sleep(0.02)
        baseline = app_state(pid)
    if titlebar_transient_open(baseline):
        baseline = dismiss_titlebar_transients(pid, baseline, timeout_seconds=1.5)
    if right_panel_mode(baseline) not in ("", "hidden", "none", "null"):
        baseline = close_right_panel(pid, baseline, timeout_seconds=1.5)
    wait_for_notifications_clear(pid, timeout_seconds=12.0)
    before, before_titlebar = wait_for_titlebar_centering(pid, timeout_seconds=4.0)
    before_flush = assert_terminal_viewport_inset(before)
    before_shot = out_dir / "maximize-roundtrip-before.png"
    app_screenshot(pid, before_shot)

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
    _, maximized_titlebar = wait_for_titlebar_centering(pid, timeout_seconds=4.0)
    maximized_shot = out_dir / "maximize-roundtrip-maximized.png"
    app_screenshot(pid, maximized_shot)

    app_set_maximized(pid, False)
    time.sleep(0.45)
    restored = wait_for_window_maximized(pid, False)
    restored, restored_titlebar = wait_for_titlebar_centering(pid, timeout_seconds=4.0)
    restored_flush = assert_terminal_viewport_inset(restored)
    restored_shot = out_dir / "maximize-roundtrip-restored.png"
    app_screenshot(pid, restored_shot)

    return {
        "before": {
            "flush": before_flush,
            "titlebar": before_titlebar,
            "screenshot": str(before_shot),
        },
        "maximized": {
            "flush": maximized_flush,
            "titlebar": maximized_titlebar,
            "screenshot": str(maximized_shot),
        },
        "restored": {
            "flush": restored_flush,
            "titlebar": restored_titlebar,
            "screenshot": str(restored_shot),
        },
    }


def wait_for_terminal_zoom_contract(
    pid: int,
    session: str,
    *,
    expected_font_size: float,
    expected_zoom_percent: float,
    expected_mount_epoch: int,
    timeout_seconds: float = 8.0,
) -> dict:
    normalized_session = normalize_live_path(session)
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        last_state = app_state(pid)
        if normalize_live_path(str(last_state.get("active_session_path") or "")) != normalized_session:
            app_open(pid, session, view="terminal")
            time.sleep(0.18)
            continue
        host = host_for_session_or_none(last_state, session)
        if host is None:
            time.sleep(0.15)
            continue
        settings = last_state.get("settings") or {}
        row_rect = ((host.get("visible_row_samples_head") or [{}])[0].get("rect") or {})
        row_height = float(row_rect.get("height") or 0.0)
        cursor_line = str(host.get("cursor_line_text") or host.get("cursor_row_text") or "")
        text_sample = str(host.get("text_sample") or "")
        interactive = (
            last_state.get("active_view_mode") == "Terminal"
            and (viewport_state(last_state).get("ready") is True)
            and (viewport_state(last_state).get("interactive") is True)
            and viewport_state(last_state).get("terminal_settled_kind") == "interactive"
            and host.get("input_enabled") is True
            and not visible_notifications(last_state)
            and not ((viewport_state(last_state).get("active_terminal_surface") or {}).get("problem"))
        )
        if (
            interactive
            and abs(float(settings.get("terminal_font_size") or 0.0) - expected_font_size) <= 0.3
            and abs(float(settings.get("terminal_zoom_percent") or 0.0) - expected_zoom_percent) <= 1.2
            and int(host.get("mount_epoch") or 0) == expected_mount_epoch
            and row_height >= 8.0
            and (cursor_line.strip() or text_sample.strip())
        ):
            return last_state
        time.sleep(0.2)
    raise AssertionError(
        "terminal zoom did not settle to the requested live contract: "
        f"expected_font_size={expected_font_size} expected_zoom_percent={expected_zoom_percent} "
        f"expected_mount_epoch={expected_mount_epoch} state={last_state!r}"
    )


def assert_terminal_zoom_live_apply(pid: int, session: str, out_dir: Path) -> dict:
    app_set_search(pid, "", focused=False)
    state = wait_for_session_focus(pid, session, timeout_seconds=10.0)
    if titlebar_transient_open(state):
        state = dismiss_titlebar_transients(pid, state, timeout_seconds=1.5)
    if right_panel_mode(state) not in ("", "hidden", "none", "null"):
        state = close_right_panel(pid, state, timeout_seconds=1.5)
    wait_for_notifications_clear(pid, timeout_seconds=6.0)

    baseline_request = app_set_main_zoom(pid, 100.0, view="terminal")
    baseline = wait_for_terminal_zoom_contract(
        pid,
        session,
        expected_font_size=14.0,
        expected_zoom_percent=100.0,
        expected_mount_epoch=int(host_for_session(state, session).get("mount_epoch") or 0),
        timeout_seconds=8.0,
    )
    baseline_host = host_for_session(baseline, session)
    baseline_row_rect = ((baseline_host.get("visible_row_samples_head") or [{}])[0].get("rect") or {})
    baseline_row_height = float(baseline_row_rect.get("height") or 0.0)
    baseline_mount_epoch = int(baseline_host.get("mount_epoch") or 0)
    baseline_host_id = str(baseline_host.get("id") or "")
    baseline_shot = out_dir / "terminal-zoom-baseline.png"
    app_screenshot(pid, baseline_shot)

    zoom_request = app_set_main_zoom(pid, 140.0, view="terminal")
    zoomed = wait_for_terminal_zoom_contract(
        pid,
        session,
        expected_font_size=19.6,
        expected_zoom_percent=140.0,
        expected_mount_epoch=baseline_mount_epoch,
        timeout_seconds=8.0,
    )
    zoomed_host = host_for_session(zoomed, session)
    zoomed_row_rect = ((zoomed_host.get("visible_row_samples_head") or [{}])[0].get("rect") or {})
    zoomed_row_height = float(zoomed_row_rect.get("height") or 0.0)
    if str(zoomed_host.get("id") or "") != baseline_host_id:
        raise AssertionError(
            "terminal zoom remounted the retained xterm host instead of updating it live: "
            f"before={baseline_host_id!r} after={zoomed_host.get('id')!r}"
        )
    if int(zoomed_host.get("mount_epoch") or 0) != baseline_mount_epoch:
        raise AssertionError(
            "terminal zoom bumped mount_epoch and likely reloaded the session instead of applying live: "
            f"before={baseline_mount_epoch} after={zoomed_host.get('mount_epoch')!r}"
        )
    if zoomed_row_height < baseline_row_height + 4.0:
        raise AssertionError(
            "terminal zoom did not materially enlarge the visible terminal rows: "
            f"baseline_row_height={baseline_row_height} zoomed_row_height={zoomed_row_height}"
        )
    zoomed_shot = out_dir / "terminal-zoom-140.png"
    app_screenshot(pid, zoomed_shot)

    restore_request = app_set_main_zoom(pid, 100.0, view="terminal")
    restored = wait_for_terminal_zoom_contract(
        pid,
        session,
        expected_font_size=14.0,
        expected_zoom_percent=100.0,
        expected_mount_epoch=baseline_mount_epoch,
        timeout_seconds=8.0,
    )
    restored_host = host_for_session(restored, session)
    restored_row_rect = ((restored_host.get("visible_row_samples_head") or [{}])[0].get("rect") or {})
    restored_row_height = float(restored_row_rect.get("height") or 0.0)
    if abs(restored_row_height - baseline_row_height) > 1.5:
        raise AssertionError(
            "terminal zoom did not restore the visible row height after returning to 100%: "
            f"baseline_row_height={baseline_row_height} restored_row_height={restored_row_height}"
        )
    restored_shot = out_dir / "terminal-zoom-restored.png"
    app_screenshot(pid, restored_shot)

    return {
        "baseline_request": baseline_request,
        "zoom_request": zoom_request,
        "restore_request": restore_request,
        "baseline": {
            "terminal_font_size": (baseline.get("settings") or {}).get("terminal_font_size"),
            "terminal_zoom_percent": (baseline.get("settings") or {}).get("terminal_zoom_percent"),
            "mount_epoch": baseline_mount_epoch,
            "host_id": baseline_host_id,
            "row_height": baseline_row_height,
            "screenshot": str(baseline_shot),
        },
        "zoomed": {
            "terminal_font_size": (zoomed.get("settings") or {}).get("terminal_font_size"),
            "terminal_zoom_percent": (zoomed.get("settings") or {}).get("terminal_zoom_percent"),
            "mount_epoch": zoomed_host.get("mount_epoch"),
            "host_id": zoomed_host.get("id"),
            "row_height": zoomed_row_height,
            "screenshot": str(zoomed_shot),
        },
        "restored": {
            "terminal_font_size": (restored.get("settings") or {}).get("terminal_font_size"),
            "terminal_zoom_percent": (restored.get("settings") or {}).get("terminal_zoom_percent"),
            "mount_epoch": restored_host.get("mount_epoch"),
            "host_id": restored_host.get("id"),
            "row_height": restored_row_height,
            "screenshot": str(restored_shot),
        },
    }


def assert_context_menu_rename_session(pid: int) -> dict:
    xdotool_key_window(pid, "Escape")
    baseline_deadline = time.time() + 2.5
    while time.time() < baseline_deadline:
        baseline_state = app_state(pid)
        if (
            not str(((baseline_state.get("shell") or {}).get("tree_rename_path") or "")).strip()
            and not rect_is_visible(dom_rect(baseline_state, "context_menu_rect"))
        ):
            break
        time.sleep(0.02)
    baseline = wait_for_window_focus(pid, timeout_seconds=4.0)
    session_row = selected_visible_session_row(baseline)
    target_path = str(session_row.get("path") or "").strip()
    if not target_path:
        raise AssertionError(f"selected session row missing path: {session_row!r}")
    click_x = sidebar_row_click_x(baseline)
    selected_deadline = time.time() + 2.5
    while time.time() < selected_deadline:
        xdotool_click_window(pid, click_x, rect_center_y(session_row))
        time.sleep(0.12)
        baseline = wait_for_window_focus(pid, timeout_seconds=4.0)
        session_row = selected_visible_session_row(baseline)
        if str(session_row.get("path") or "").strip() == target_path:
            break
    else:
        raise AssertionError(
            f"failed to re-anchor selected sidebar row before rename probe: "
            f"target_path={target_path!r} selected_row={session_row!r}"
        )
    active_session = str(viewport_state(baseline).get("active_session_path") or "").strip()

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
    dismiss_rect = dom_rect(opened, "main_surface_body_rect") or dom_rect(opened, "main_surface_rect")
    if not rect_is_visible(dismiss_rect):
        dismiss_rect = baseline.get("dom", {}).get("sidebar_rect") or {}
    xdotool_click_window(
        pid,
        rect_center_x(dismiss_rect),
        rect_center_y(dismiss_rect),
    )
    dismiss_deadline = time.time() + 2.0
    while time.time() < dismiss_deadline:
        current = app_state(pid)
        if not rect_is_visible(dom_rect(current, "context_menu_rect")):
            break
        time.sleep(0.02)
    else:
        raise AssertionError(f"context menu did not dismiss cleanly before rename gesture: {current!r}")
    rename_click = xdotool_double_click_window(
        pid,
        click_x,
        rect_center_y(session_row),
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
        time.sleep(0.02)
    else:
        raise AssertionError(f"context menu rename action did not enter rename mode: {renamed!r}")
    focus_deadline = time.time() + 2.5
    focus_trace: list[dict] = []
    while time.time() < focus_deadline:
        renamed = app_state(pid)
        dom = renamed.get("dom") or {}
        active_element = dom.get("active_element") or {}
        focus_trace.append(
            {
                "tree_rename_path": str(((renamed.get("shell") or {}).get("tree_rename_path") or "")).strip(),
                "tree_rename_input_focused_once": bool(
                    ((renamed.get("shell") or {}).get("tree_rename_input_focused_once"))
                ),
                "tree_rename_input_focused": bool(dom.get("tree_rename_input_focused")),
                "tree_rename_input_rect": dom_rect(renamed, "tree_rename_input_rect"),
                "active_tag": str(active_element.get("tag_name") or ""),
                "active_class": str(active_element.get("class_name") or ""),
                "active_tree_rename_input": str(active_element.get("data_tree_rename_input") or ""),
                "helper_focused": bool((active_host_or_none(renamed) or {}).get("helper_textarea_focused")),
                "input_enabled": bool((active_host_or_none(renamed) or {}).get("input_enabled")),
            }
        )
        if bool(dom.get("tree_rename_input_focused")):
            break
        time.sleep(0.02)
    else:
        raise AssertionError(
            "rename mode opened without focusing the rename input: "
            f"trace={focus_trace!r} final={renamed!r}"
        )

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
        try:
            wait_for_session_focus(pid, active_session, timeout_seconds=10.0)
        except AssertionError:
            pass
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


def assert_context_menu_delete_session(pid: int) -> dict:
    created = create_terminal_via_sidebar_context_menu(pid)
    target_path = created["session_path"]
    try:
        deleted = delete_session_via_context_menu(
            pid,
            target_path,
            fallback_session_path=created.get("baseline_active_session_path"),
        )
    except Exception:
        try:
            app_remove_session(pid, target_path)
        except Exception:
            pass
        raise
    return {
        "created": created,
        "deleted": deleted,
    }


def assert_focus_and_visibility(pid: int, state: dict) -> dict:
    def current_focus_snapshot(snapshot: dict) -> tuple[dict, dict, dict, list[dict]]:
        return (
            viewport_state(snapshot),
            active_host(snapshot),
            ((snapshot.get("dom") or {}).get("active_element") or {}),
            visible_notifications(snapshot),
        )

    def focus_contract_satisfied(host: dict, active_element: dict) -> bool:
        return (
            host.get("helper_textarea_focused") is True
            and host.get("host_has_active_element") is True
            and active_element.get("class_name") == "xterm-helper-textarea"
        )

    viewport, host, active_element, notifications = current_focus_snapshot(state)
    if (
        viewport.get("ready") is True
        and viewport.get("interactive") is True
        and host.get("input_enabled") is True
        and not focus_contract_satisfied(host, active_element)
    ):
        focus_rect = host.get("host_rect") or dom_rect(state, "main_surface_body_rect")
        if rect_is_visible(focus_rect):
            for attempt in range(4):
                xdotool_click_window(pid, rect_center_x(focus_rect), rect_center_y(focus_rect))
                deadline = time.time() + 1.2
                while time.time() < deadline:
                    time.sleep(0.12)
                    state = app_state(pid)
                    viewport, host, active_element, notifications = current_focus_snapshot(state)
                    if focus_contract_satisfied(host, active_element):
                        break
                if focus_contract_satisfied(host, active_element):
                    break
                if viewport.get("ready") is not True or viewport.get("interactive") is not True:
                    break

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
    input_policy = assert_only_active_host_accepts_input(state)
    return {
        "ready": viewport.get("ready"),
        "interactive": viewport.get("interactive"),
        "terminal_settled_kind": viewport.get("terminal_settled_kind"),
        "active_element": active_element,
        "input_enabled": host.get("input_enabled"),
        "inactive_input_enabled_count": input_policy["inactive_input_enabled_count"],
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


def assert_live_session_metadata_consistency(pid: int, session: str) -> dict:
    state = app_state(pid)
    rows = [
        row
        for row in app_rows(pid)
        if normalize_live_path(str(row.get("full_path") or row.get("path") or "")) == normalize_live_path(session)
        and str(row.get("kind") or "") == "Session"
    ]
    if not rows:
        raise AssertionError(
            f"session is missing from app rows for metadata consistency check: {session!r}"
        )
    active_title = str(state.get("active_title") or "").strip()
    active_summary = str(state.get("active_summary") or "").strip()
    if not active_title:
        raise AssertionError(
            f"active title is empty for metadata consistency check: session={session!r} state={state!r}"
        )
    mismatched_titles = [
        {
            "label": row.get("label"),
            "session_title": row.get("session_title"),
            "depth": row.get("depth"),
        }
        for row in rows
        if str(row.get("label") or "").strip() != active_title
        or str(row.get("session_title") or "").strip() not in ("", active_title)
    ]
    if mismatched_titles:
        raise AssertionError(
            "same live session path is rendering with different titles across surfaces: "
            f"active_title={active_title!r} rows={mismatched_titles!r}"
        )
    mismatched_summaries = [
        {
            "detail_label": row.get("detail_label"),
            "depth": row.get("depth"),
        }
        for row in rows
        if active_summary and str(row.get("detail_label") or "").strip() != active_summary
    ]
    if mismatched_summaries:
        raise AssertionError(
            "same live session path is rendering with different summaries across surfaces: "
            f"active_summary={active_summary!r} rows={mismatched_summaries!r}"
        )
    return {
        "row_count": len(rows),
        "active_title": active_title,
        "active_summary": active_summary,
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
    partner_text_sample = str(
        partner_host.get("text_sample") or partner_host.get("cursor_line_text") or ""
    ).strip()
    if not partner_text_sample:
        raise AssertionError(
            f"hot-switched partner session is interactive but still blank: {partner_host!r}"
        )
    partner_text_lower = partner_text_sample.lower()
    if (
        "queue live " in partner_text_lower
        or "daemon pty: request main viewport terminal stream" in partner_text_lower
        or "waiting for terminal host" in partner_text_lower
        or "runtime degraded" in partner_text_lower
    ):
        raise AssertionError(
            "hot-switched partner reopened launcher boilerplate instead of a retained hot terminal: "
            f"path={partner_path!r} text_sample={partner_text_sample[-400:]!r}"
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
    final_text_lower = final_text.lower()
    if (
        "queue live " in final_text_lower
        or "daemon pty: request main viewport terminal stream" in final_text_lower
        or "waiting for terminal host" in final_text_lower
        or "runtime degraded" in final_text_lower
    ):
        raise AssertionError(
            "hot switch back reopened launcher boilerplate instead of the retained terminal surface: "
            f"path={session!r} text_sample={final_text[-400:]!r}"
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
    cursor_text = str(
        host.get("cursor_buffer_cell_text")
        or host.get("cursor_sample_text")
        or ""
    )
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
    block_cursor = "xterm-cursor-block" in cursor_class_name
    contrast_background = (
        cursor_background if block_cursor and not is_transparent_css_color(cursor_background) else row_background
    )
    glyph_contrast = contrast_ratio(cursor_color, contrast_background) if cursor_text.strip() else None
    background_rgb = parse_css_rgb(contrast_background)
    minimum_visible_contrast = (
        6.5
        if background_rgb is not None and relative_luminance(background_rgb) > 0.72
        else 4.5
    )
    if cursor_text.strip():
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
                f"cursor glyph contrast too low: text={cursor_text!r} color={cursor_color!r} background={contrast_background!r} contrast={glyph_contrast!r}"
            )
        if block_cursor and is_transparent_css_color(cursor_background):
            raise AssertionError(
                f"block cursor lost its fill paint for visible text {cursor_text!r}: {cursor_background!r}"
            )
    return {
        "cursor_sample_text": cursor_text,
        "cursor_buffer_cell_text": str(host.get("cursor_buffer_cell_text") or ""),
        "cursor_buffer_prev_cell_text": str(host.get("cursor_buffer_prev_cell_text") or ""),
        "cursor_sample_color": cursor_color,
        "cursor_sample_background": cursor_background,
        "contrast_background": contrast_background,
        "cursor_sample_border_left": cursor_border_left,
        "cursor_sample_class_name": cursor_class_name,
        "cursor_glyph_visibility": visibility,
        "cursor_glyph_opacity": opacity,
        "cursor_glyph_contrast": round(glyph_contrast, 2) if glyph_contrast is not None else None,
    }


def assert_cursor_focus_blur_contract(pid: int, session: str) -> dict:
    app_set_search(pid, "", focused=False)
    app_open(pid, session, view="terminal")
    deadline = time.time() + 8.0
    pre_click_state = {}
    while time.time() < deadline:
        pre_click_state = app_state(pid)
        shell = pre_click_state.get("shell") or {}
        focused_host = active_host(pre_click_state)
        if (
            pre_click_state.get("active_session_path") == session
            and pre_click_state.get("active_view_mode") == "Terminal"
            and not shell.get("search_focused")
            and rect_is_visible(focused_host.get("host_rect") or {})
        ):
            break
        time.sleep(0.12)
    else:
        raise AssertionError(f"failed to restore terminal host before click focus probe: {pre_click_state!r}")
    click_host = active_host(pre_click_state)
    click_rect = click_host.get("host_rect") or {}
    focus_click = xdotool_click_window(pid, rect_center_x(click_rect), rect_center_y(click_rect))
    deadline = time.time() + 8.0
    focused_state = {}
    accepted_unfocused_outline = False
    while time.time() < deadline:
        focused_state = app_state(pid)
        shell = focused_state.get("shell") or {}
        focused_host = active_host(focused_state)
        focused_glyph_text = str(
            focused_host.get("cursor_sample_text") or focused_host.get("cursor_buffer_cell_text") or ""
        )
        focused_block_cursor = "xterm-cursor-block" in str(focused_host.get("cursor_sample_class_name") or "")
        focused_outline_cursor = "xterm-cursor-outline" in str(
            focused_host.get("cursor_sample_class_name") or ""
        )
        window_focused = bool((focused_state.get("window") or {}).get("focused"))
        focused_terminal_surface = bool(focused_host.get("helper_textarea_focused")) or (
            focused_block_cursor
            and rect_is_visible(
                focused_host.get("cursor_sample_rect") or focused_host.get("cursor_expected_rect") or {}
            )
            and (not focused_glyph_text or not is_transparent_css_color(str(focused_host.get("cursor_sample_background") or "")))
        )
        allow_unfocused_outline = (
            AVOID_FOREGROUND
            and not window_focused
            and bool(focused_host.get("helper_textarea_focused"))
            and focused_outline_cursor
        )
        if (
            focused_state.get("active_session_path") == session
            and focused_state.get("active_view_mode") == "Terminal"
            and not shell.get("search_focused")
            and focused_terminal_surface
            and (focused_block_cursor or allow_unfocused_outline)
        ):
            accepted_unfocused_outline = allow_unfocused_outline and not focused_block_cursor
            break
        time.sleep(0.12)
    else:
        raise AssertionError(
            f"terminal did not retain focused block cursor after click release: click={focus_click!r} state={focused_state!r}"
        )
    focused_host = active_host(focused_state)
    focused_glyph = assert_cursor_glyph_visibility(focused_state)
    if not accepted_unfocused_outline and "xterm-cursor-block" not in str(
        focused_host.get("cursor_sample_class_name") or ""
    ):
        raise AssertionError(f"focused cursor is not a block cursor: {focused_host!r}")
    if not accepted_unfocused_outline and is_transparent_css_color(
        str(focused_host.get("cursor_sample_background") or "")
    ):
        raise AssertionError(f"focused block cursor lost its fill: {focused_host!r}")

    app_set_search(pid, "", focused=True)
    deadline = time.time() + 6.0
    blurred_state = {}
    blurred_glyph = {}
    while time.time() < deadline:
        blurred_state = app_state(pid)
        shell = blurred_state.get("shell") or {}
        blurred_host = active_host(blurred_state)
        if not (shell.get("search_focused") and not blurred_host.get("helper_textarea_focused")):
            time.sleep(0.12)
            continue
        try:
            blurred_glyph = assert_cursor_glyph_visibility(blurred_state)
        except AssertionError:
            time.sleep(0.12)
            continue
        break
        time.sleep(0.12)
    else:
        raise AssertionError(f"failed to blur terminal cursor into outline state: {blurred_state!r}")

    blurred_host = active_host(blurred_state)
    blurred_class_name = str(blurred_host.get("cursor_sample_class_name") or "")
    blurred_background = str(blurred_host.get("cursor_sample_background") or "")
    row_background = str(blurred_host.get("cursor_row_background") or "")
    blurred_box_shadow = str(blurred_host.get("cursor_sample_box_shadow") or "").strip().lower()
    blurred_looks_like_outline = "xterm-cursor-outline" in blurred_class_name or (
        (
            is_transparent_css_color(blurred_background)
            or css_colors_close(blurred_background, row_background)
        )
        and blurred_box_shadow not in {"", "none"}
    )
    if not blurred_looks_like_outline:
        raise AssertionError(f"blurred cursor is not rendering as an outline cursor: {blurred_host!r}")
    if (
        not is_transparent_css_color(blurred_background)
        and not css_colors_close(blurred_background, row_background)
    ):
        raise AssertionError(f"blurred outline cursor still has fill paint: {blurred_host!r}")
    normal_foreground_candidates = [
        str(blurred_host.get("host_css_foreground") or ""),
        str(blurred_host.get("rows_color") or ""),
        str(blurred_host.get("cursor_row_color") or ""),
    ]
    cursor_sample_color = str(blurred_host.get("cursor_sample_color") or "")
    if normal_foreground_candidates and not any(
        candidate and css_colors_close(cursor_sample_color, candidate)
        for candidate in normal_foreground_candidates
    ):
        raise AssertionError(
            "blurred outline cursor inherited the focused inverted glyph color instead of the normal row foreground: "
            f"cursor_color={blurred_host.get('cursor_sample_color')!r} "
            f"foreground_candidates={normal_foreground_candidates!r}"
        )

    app_set_search(pid, "", focused=False)
    deadline = time.time() + 8.0
    restored_state = {}
    while time.time() < deadline:
        restored_state = app_state(pid)
        shell = restored_state.get("shell") or {}
        restored_host = active_host(restored_state)
        if (
            restored_state.get("active_session_path") == session
            and restored_state.get("active_view_mode") == "Terminal"
            and not shell.get("search_focused")
            and restored_host.get("helper_textarea_focused")
        ):
            break
        time.sleep(0.12)
    else:
        raise AssertionError(f"failed to restore focused terminal after blur probe: {restored_state!r}")
    return {
        "focus_click": focus_click,
        "focused": focused_glyph,
        "focused_cursor_mode": "unfocused-outline" if accepted_unfocused_outline else "focused-block",
        "blurred": blurred_glyph,
        "blurred_row_color": str(blurred_host.get("cursor_row_color") or ""),
    }


def assert_cursor_mouse_hold_contract(pid: int, session: str) -> dict:
    normalized_session = str(session or "").strip()
    if normalized_session.startswith("local://"):
        probe_session = ensure_stable_plain_terminal_session(pid, normalized_session)
    else:
        probe_session = normalized_session
        app_open(pid, probe_session, view="terminal")
        app_set_search(pid, "", focused=False)
    deadline = time.time() + 8.0
    pre_click_state = {}
    while time.time() < deadline:
        pre_click_state = app_state(pid)
        shell = pre_click_state.get("shell") or {}
        host = active_host(pre_click_state)
        if (
            pre_click_state.get("active_session_path") == probe_session
            and pre_click_state.get("active_view_mode") == "Terminal"
            and not shell.get("search_focused")
            and rect_is_visible(host.get("host_rect") or {})
        ):
            break
        time.sleep(0.12)
    else:
        raise AssertionError(f"failed to restore terminal host before mouse-hold probe: {pre_click_state!r}")
    click_host = active_host(pre_click_state)
    click_rect = click_host.get("host_rect") or {}
    focus_click = None
    focused_before_type = {}
    accepted_unfocused_outline = False
    for _ in range(2):
        focus_click = xdotool_click_window(pid, rect_center_x(click_rect), rect_center_y(click_rect))
        focus_deadline = time.time() + 2.0
        while time.time() < focus_deadline:
            focused_before_type = app_state(pid)
            shell = focused_before_type.get("shell") or {}
            host = active_host(focused_before_type)
            glyph_text = str(host.get("cursor_sample_text") or host.get("cursor_buffer_cell_text") or "")
            block_cursor_visible = "xterm-cursor-block" in str(host.get("cursor_sample_class_name") or "")
            outline_cursor_visible = "xterm-cursor-outline" in str(host.get("cursor_sample_class_name") or "")
            window_focused = bool((focused_before_type.get("window") or {}).get("focused"))
            focused_terminal_surface = bool(host.get("helper_textarea_focused")) or (
                block_cursor_visible
                and rect_is_visible(host.get("cursor_sample_rect") or host.get("cursor_expected_rect") or {})
                and (not glyph_text or not is_transparent_css_color(str(host.get("cursor_sample_background") or "")))
            )
            allow_unfocused_outline = (
                AVOID_FOREGROUND
                and not window_focused
                and bool(host.get("helper_textarea_focused"))
                and outline_cursor_visible
            )
            if (
                focused_before_type.get("active_session_path") == probe_session
                and focused_before_type.get("active_view_mode") == "Terminal"
                and not shell.get("search_focused")
                and focused_terminal_surface
                and (block_cursor_visible or allow_unfocused_outline)
            ):
                accepted_unfocused_outline = allow_unfocused_outline and not block_cursor_visible
                break
            time.sleep(0.12)
        else:
            continue
        break
    deadline = time.time() + 8.0
    focused_state = {}
    while time.time() < deadline:
        focused_state = app_state(pid)
        shell = focused_state.get("shell") or {}
        host = active_host(focused_state)
        glyph_text = str(host.get("cursor_sample_text") or host.get("cursor_buffer_cell_text") or "")
        block_cursor_visible = "xterm-cursor-block" in str(host.get("cursor_sample_class_name") or "")
        outline_cursor_visible = "xterm-cursor-outline" in str(host.get("cursor_sample_class_name") or "")
        window_focused = bool((focused_state.get("window") or {}).get("focused"))
        focused_terminal_surface = bool(host.get("helper_textarea_focused")) or (
            block_cursor_visible
            and rect_is_visible(host.get("cursor_sample_rect") or host.get("cursor_expected_rect") or {})
            and (not glyph_text or not is_transparent_css_color(str(host.get("cursor_sample_background") or "")))
        )
        allow_unfocused_outline = (
            AVOID_FOREGROUND
            and not window_focused
            and bool(host.get("helper_textarea_focused"))
            and outline_cursor_visible
        )
        if (
            focused_state.get("active_session_path") == probe_session
            and focused_state.get("active_view_mode") == "Terminal"
            and not shell.get("search_focused")
            and focused_terminal_surface
            and (block_cursor_visible or allow_unfocused_outline)
        ):
            accepted_unfocused_outline = allow_unfocused_outline and not block_cursor_visible
            break
        time.sleep(0.12)
    else:
        raise AssertionError(
            f"failed to reach focused block-cursor state before mouse-hold probe: click={focus_click!r} state={focused_state!r}"
        )
    baseline_glyph = assert_cursor_glyph_visibility(focused_state)
    focus_host = active_host(focused_state)
    cursor_rect = focus_host.get("cursor_sample_rect") or focus_host.get("cursor_expected_rect") or {}
    press = xdotool_press_window(pid, rect_center_x(cursor_rect), rect_center_y(cursor_rect))
    held_state = {}
    held_state_path = Path(f"/tmp/yggterm-cursor-held-{pid}-state.json")
    held_shot = Path(f"/tmp/yggterm-cursor-held-{pid}.png")
    try:
        held_host = {}
        held_glyph = {}
        deadline = time.time() + 3.0
        while True:
            time.sleep(0.12)
            held_state = app_state(pid)
            held_host = active_host(held_state)
            held_glyph = assert_cursor_glyph_visibility(held_state)
            held_class_name = str(held_host.get("cursor_sample_class_name") or "")
            held_accept_unfocused_outline = (
                accepted_unfocused_outline
                and not bool((held_state.get("window") or {}).get("focused"))
                and bool(held_host.get("helper_textarea_focused"))
                and "xterm-cursor-outline" in held_class_name
            )
            if "xterm-cursor-block" in held_class_name or held_accept_unfocused_outline:
                break
            if time.time() >= deadline:
                break
        if held_state.get("active_session_path") != probe_session or held_state.get("active_view_mode") != "Terminal":
            raise AssertionError(
                f"cursor mouse-hold moved focus away from the active terminal: state={held_state!r}"
            )
        if not held_accept_unfocused_outline and "xterm-cursor-block" not in str(
            held_host.get("cursor_sample_class_name") or ""
        ):
            raise AssertionError(f"cursor lost block state during mouse hold: {held_host!r}")
        if str(held_glyph.get("cursor_sample_text") or "").strip():
            held_contrast = held_glyph.get("cursor_glyph_contrast")
            baseline_contrast = baseline_glyph.get("cursor_glyph_contrast")
            if held_contrast is None or held_contrast < 4.5:
                raise AssertionError(
                    f"cursor glyph lost readable contrast during mouse hold: glyph={held_glyph!r} host={held_host!r}"
                )
            if baseline_contrast is not None and held_contrast + 0.1 < baseline_contrast:
                raise AssertionError(
                    f"cursor glyph contrast regressed during mouse hold: baseline={baseline_glyph!r} held={held_glyph!r}"
                )
        held_state_path.write_text(json.dumps(held_state, indent=2), encoding="utf-8")
        app_screenshot(pid, held_shot)
    finally:
        release = xdotool_release_button(pid)
    return {
        "focus_click": focus_click,
        "press": press,
        "release": release,
        "baseline_glyph": baseline_glyph,
        "held_glyph": held_glyph,
        "baseline_cursor_mode": "unfocused-outline" if accepted_unfocused_outline else "focused-block",
        "probe_session": probe_session,
        "held_state_path": str(held_state_path),
        "held_screenshot_path": str(held_shot),
    }


def assert_cursor_alignment(state: dict) -> dict:
    host = active_host(state)
    expected_rect = host.get("cursor_expected_rect") or {}
    cursor_rect = host.get("cursor_sample_rect") or {}
    focused_expected_rect_fallback = (
        host.get("helper_textarea_focused") is True
        and host.get("host_has_active_element") is True
        and host.get("xterm_cursor_hidden") is not True
        and rect_is_visible(expected_rect)
        and (
            str(host.get("cursor_row_text") or host.get("cursor_line_text") or "").strip() != ""
            or int(host.get("cursor_visible_row_index") or -1) >= 0
        )
    )
    if not rect_is_visible(expected_rect):
        raise AssertionError(
            f"expected cursor cell rect is missing/empty, cannot prove alignment: {expected_rect!r}"
        )
    if not cursor_sample_is_visibly_active(host):
        if not focused_expected_rect_fallback:
            raise AssertionError(
                f"no visible native cursor rect: raw={cursor_rect!r} hidden={host.get('xterm_cursor_hidden')!r}"
            )
        return {
            "cursor_expected_rect": expected_rect,
            "cursor_sample_rect": cursor_rect,
            "active_cursor_rect": expected_rect,
            "using_overlay": False,
            "cursor_dx": 0.0,
            "cursor_dy": 0.0,
            "fallback_expected_rect": True,
        }
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


def assert_terminal_interaction_latency(pid: int, session: str) -> dict:
    focus_response, focus_elapsed_ms = run_timed(
        "server",
        "app",
        "terminal",
        "focus",
        "--pid",
        str(pid),
        session,
        "--timeout-ms",
        "15000",
        timeout_seconds=20.0,
    )
    focus_data = unwrap_data(focus_response)
    if not bool(focus_data.get("accepted")):
        raise AssertionError(f"terminal focus command was not accepted: {focus_response!r}")

    select_response, select_elapsed_ms = run_timed(
        "server",
        "app",
        "terminal",
        "probe-select",
        "--pid",
        str(pid),
        session,
        "--timeout-ms",
        "8000",
        timeout_seconds=20.0,
    )
    select_data = unwrap_data(select_response)
    if int(select_data.get("selected_text_length") or 0) <= 0:
        raise AssertionError(f"selection probe did not capture visible text during latency check: {select_response!r}")

    type_response, type_elapsed_ms = run_timed(
        "server",
        "app",
        "terminal",
        "probe-type",
        "--pid",
        str(pid),
        session,
        "--mode",
        "keyboard",
        "--data",
        "/",
        "--timeout-ms",
        "15000",
        timeout_seconds=20.0,
    )
    type_data = unwrap_data(type_response)
    active_state = wait_for_session_focus(pid, session, timeout_seconds=10.0)
    active = host_for_session(active_state, session)
    cursor_line_text = str(active.get("cursor_line_text") or active.get("cursor_row_text") or "")
    text_tail = str(active.get("text_sample") or "")
    cleanup_response = terminal_send(pid, session, "\u0015")
    if not cleanup_response or not bool((cleanup_response.get("data") or {}).get("accepted")):
        raise AssertionError(
            f"terminal cleanup control-U could not be delivered after latency probe: "
            f"response={cleanup_response!r}"
        )
    time.sleep(0.12)
    if type_elapsed_ms > TERMINAL_INTERACTION_LATENCY_MAX_MS:
        raise AssertionError(
            f"terminal type probe exceeded latency budget: elapsed_ms={type_elapsed_ms:.1f} "
            f"budget_ms={TERMINAL_INTERACTION_LATENCY_MAX_MS:.1f} type_data={type_data!r} "
            f"cursor_line={cursor_line_text!r}"
        )
    if not bool(type_data.get("accepted")):
        raise AssertionError(f"terminal type probe was not accepted: {type_response!r}")

    scroll_response, scroll_elapsed_ms = run_timed(
        "server",
        "app",
        "terminal",
        "probe-scroll",
        "--pid",
        str(pid),
        session,
        "--lines",
        "5",
        "--timeout-ms",
        "12000",
        timeout_seconds=20.0,
    )
    scroll_data = unwrap_data(scroll_response)
    if scroll_elapsed_ms > TERMINAL_INTERACTION_LATENCY_MAX_MS:
        raise AssertionError(
            f"terminal scroll probe exceeded latency budget: elapsed_ms={scroll_elapsed_ms:.1f} "
            f"budget_ms={TERMINAL_INTERACTION_LATENCY_MAX_MS:.1f} scroll_data={scroll_data!r}"
        )
    if not bool(scroll_data.get("accepted")):
        raise AssertionError(f"terminal scroll probe was not accepted: {scroll_response!r}")

    return {
        "focus_elapsed_ms": round(focus_elapsed_ms, 1),
        "select_elapsed_ms": round(select_elapsed_ms, 1),
        "type_elapsed_ms": round(type_elapsed_ms, 1),
        "scroll_elapsed_ms": round(scroll_elapsed_ms, 1),
        "latency_budget_ms": TERMINAL_INTERACTION_LATENCY_MAX_MS,
        "type_cursor_line_text": cursor_line_text,
        "type_text_tail": text_tail[-240:],
        "type_keyboard_backend": type_data.get("keyboard_backend"),
        "type_used_core_trigger": type_data.get("used_core_trigger"),
        "type_used_term_input": type_data.get("used_term_input"),
        "scroll_before": scroll_data.get("before"),
        "scroll_after": scroll_data.get("after"),
        "selected_text_length": int(select_data.get("selected_text_length") or 0),
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
    input_backend: str = "keyboard",
) -> dict:
    steps: list[dict] = []
    typed_so_far = ""
    for index in range(0, len(text), chunk_size):
        chunk = text[index : index + chunk_size]
        typed_so_far += chunk
        focus_state = None
        probe = None
        state = None
        host = None
        cursor_line_text = ""
        text_sample = ""
        if input_backend == "send":
            focus_state = prime_terminal_surface_for_keyboard(pid, session)
            probe = terminal_send(pid, session, chunk)
            deadline = time.time() + 4.0
            while True:
                time.sleep(0.18)
                candidate_state = wait_for_terminal_quiescent(pid, timeout_seconds=8.0)
                candidate_host = active_host(candidate_state)
                cursor_line_text = str(candidate_host.get("cursor_line_text") or candidate_host.get("cursor_row_text") or "")
                text_sample = str(candidate_host.get("text_sample") or "")
                state = candidate_state
                host = candidate_host
                if typed_so_far in cursor_line_text or typed_so_far in text_sample:
                    break
                if time.time() >= deadline:
                    break
        else:
            for attempt in range(3):
                focus_state = prime_terminal_surface_for_keyboard(pid, session)
                if attempt == 0:
                    probe = probe_type(pid, session, chunk, mode="keyboard")
                elif attempt == 1:
                    probe = xdotool_type_focused(pid, chunk)
                else:
                    probe = terminal_send(pid, session, chunk)
                deadline = time.time() + 4.0
                while True:
                    time.sleep(0.18)
                    candidate_state = wait_for_terminal_quiescent(pid, timeout_seconds=8.0)
                    candidate_host = active_host(candidate_state)
                    cursor_line_text = str(candidate_host.get("cursor_line_text") or candidate_host.get("cursor_row_text") or "")
                    text_sample = str(candidate_host.get("text_sample") or "")
                    state = candidate_state
                    host = candidate_host
                    if typed_so_far in cursor_line_text or typed_so_far in text_sample:
                        break
                    if time.time() >= deadline:
                        break
                if typed_so_far in cursor_line_text or typed_so_far in text_sample:
                    break
                if attempt < 2:
                    terminal_reclaim_focus(pid, session)
                    time.sleep(0.25)
        assert state is not None
        assert host is not None
        assert focus_state is not None
        assert probe is not None
        screenshot_path = out_dir / f"{prefix}-step-{(index // chunk_size) + 1:02d}.png"
        state_path = out_dir / f"{prefix}-step-{(index // chunk_size) + 1:02d}-state.json"
        app_screenshot(pid, screenshot_path)
        with state_path.open("w") as fh:
            json.dump(state, fh, indent=2)
        if typed_so_far not in cursor_line_text and typed_so_far not in text_sample:
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
        cursor_cell_pixels = assert_cursor_cell_glyph_pixels_visible(
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
                "focus_state_active_tag": ((focus_state.get("dom") or {}).get("active_element") or {}).get("tag"),
                "probe": probe,
                "screenshot": str(screenshot_path),
                "state": str(state_path),
                "prompt_anchor": prompt_anchor,
                "pixel_probe": pixel_probe,
                "cursor_cell_pixels": cursor_cell_pixels,
                "prompt_pixels": prompt_pixels,
            }
        )
    return {
        "chunk_size": chunk_size,
        "steps": steps,
        "final_state": state,
    }


def assert_partial_input_flow(pid: int, session: str, out_dir: Path) -> dict:
    baseline = app_state(pid)
    if titlebar_transient_open(baseline):
        baseline = dismiss_titlebar_transients(pid, baseline, timeout_seconds=1.5)
    if right_panel_mode(baseline) not in ("", "hidden", "none", "null"):
        baseline = close_right_panel(pid, baseline, timeout_seconds=1.5)
    if baseline.get("active_session_path") != session or baseline.get("active_view_mode") != "Terminal":
        app_open(pid, session, view="terminal")
    wait_for_session_focus(pid, session, timeout_seconds=12.0)
    prime_terminal_surface_for_keyboard(pid, session)
    clear_terminal_selection(pid, session, timeout_seconds=4.0)
    wait_for_terminal_quiescent(pid, timeout_seconds=12.0)
    prime_terminal_surface_for_keyboard(pid, session)
    clear = terminal_send(pid, session, "\u0003")
    time.sleep(0.3)
    cleared_state = wait_for_terminal_quiescent(pid, timeout_seconds=8.0)
    cleared_host = active_host(cleared_state)
    cleared_cursor_line = str(
        cleared_host.get("cursor_line_text") or cleared_host.get("cursor_row_text") or ""
    )
    if "^C" in cleared_cursor_line:
        clear = terminal_send(pid, session, "\u0003")
        time.sleep(0.3)
        wait_for_terminal_quiescent(pid, timeout_seconds=8.0)
    chunked_typing = type_with_cursor_artifact_checks(
        pid,
        session,
        "/sta",
        out_dir,
        prefix="partial-type",
        chunk_size=1,
        input_backend="send",
    )
    typed_state = chunked_typing.get("final_state") or app_state(pid)
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
    low_contrast_cursor_spans = []
    for sample in (typed_host.get("cursor_row_span_samples") or []):
        if not isinstance(sample, dict) or not sample.get("text") or sample.get("contrast") is None:
            continue
        background_rgb = parse_css_rgb(str(sample.get("background") or ""))
        minimum_visible_contrast = (
            6.5
            if background_rgb is not None and relative_luminance(background_rgb) > 0.72
            else 4.5
        )
        if float(sample["contrast"]) < minimum_visible_contrast:
            low_contrast_cursor_spans.append(sample)
    if low_contrast_cursor_spans:
        raise AssertionError(
            f"cursor row still has low-contrast spans: {low_contrast_cursor_spans!r}"
        )
    typed_anchor = assert_cursor_prompt_visibility(typed_state, context="after partial typing")
    assert_cursor_alignment(typed_state)

    scroll_probe = probe_scroll(pid, session, -5)
    before_scroll = scroll_probe.get("before") or {}
    after_scroll = scroll_probe.get("after") or {}
    if (
        after_scroll.get("input_enabled") is not True
        or after_scroll.get("helper_textarea_focused") is not True
        or after_scroll.get("host_has_active_element") is not True
    ):
        raise AssertionError(f"partial scroll lost interactive terminal focus: {scroll_probe!r}")
    deadline = time.time() + 3.0
    scroll_state = {}
    scroll_host = {}
    while True:
        time.sleep(0.12)
        scroll_state = app_state(pid)
        scroll_host = active_host(scroll_state)
        if scroll_host.get("input_enabled") is True and scroll_host.get("helper_textarea_focused") is True:
            break
        if time.time() >= deadline:
            break
    scroll_shot = out_dir / "after-partial-scroll.png"
    app_screenshot(pid, scroll_shot)
    with (out_dir / "after-partial-scroll-state.json").open("w") as fh:
        json.dump(scroll_state, fh, indent=2)
    viewport_moved = (
        before_scroll.get("viewport_y") != after_scroll.get("viewport_y")
        or before_scroll.get("text_tail") != after_scroll.get("text_tail")
    )
    scroll_anchor = None
    if not viewport_moved:
        visibility_deadline = time.time() + 3.0
        last_visibility_error: AssertionError | None = None
        while True:
            try:
                scroll_anchor = assert_cursor_prompt_visibility(scroll_state, context="after partial scroll")
                assert_cursor_alignment(scroll_state)
                break
            except AssertionError as exc:
                last_visibility_error = exc
                if time.time() >= visibility_deadline:
                    raise last_visibility_error
                time.sleep(0.12)
                scroll_state = app_state(pid)
                scroll_host = active_host(scroll_state)

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


def wait_for_live_codex_prompt(pid: int, session: str, timeout_seconds: float = 20.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        last_state = wait_for_interactive_session(pid, session, timeout_seconds=8.0)
        host = host_for_session(last_state, session)
        if host_has_live_codex_prompt(host):
            return last_state
        time.sleep(0.25)
    raise AssertionError(f"live Codex prompt did not become visible in time: {last_state!r}")


def ensure_live_codex_runtime(pid: int, session: str) -> dict:
    current = wait_for_interactive_session(pid, session, timeout_seconds=20.0)
    host = host_for_session(current, session)
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
    state = wait_for_live_codex_prompt(pid, session=session, timeout_seconds=30.0)
    return {
        "action": "launch_codex",
        "prepare_probe": prepare,
        "launch_probe": launch,
        "state": state,
    }


def assert_codex_startup_health(pid: int, session: str, out_dir: Path) -> dict:
    state = wait_for_interactive_session(pid, session, timeout_seconds=12.0)
    host = host_for_session(state, session)
    failure = detect_codex_startup_failure(host)
    text_tail = terminal_host_text(host)[-600:]
    log_hits = codex_connector_log_hits()
    if failure is None:
        return {
            "text_tail": text_tail,
            "recent_log_hits": log_hits,
        }

    screenshot_path = out_dir / "codex-startup-health.png"
    state_path = out_dir / "codex-startup-health-state.json"
    app_screenshot(pid, screenshot_path)
    with state_path.open("w") as fh:
        json.dump(state, fh, indent=2)
    challenge_like = any(
        "403 forbidden" in line.lower()
        or "/connectors/directory/list" in line.lower()
        or "enable javascript and cookies to continue" in line.lower()
        for line in log_hits
    )
    if challenge_like:
        return {
            "skipped": "external_connectors_backend_failure",
            "reason": (
                "Codex runtime surfaced an MCP startup failure because the ChatGPT connectors backend "
                "returned a 403/Cloudflare challenge instead of MCP JSON."
            ),
            "text_tail": failure["text_tail"],
            "recent_log_hits": log_hits,
            "screenshot": str(screenshot_path),
            "state": str(state_path),
        }
    raise AssertionError(
        "Codex runtime surfaced an MCP startup failure in the live terminal buffer: "
        f"text_tail={failure['text_tail']!r} recent_log_hits={log_hits!r} "
        f"screenshot={screenshot_path} state={state_path}"
    )


def ensure_plain_shell_runtime(pid: int, session: str) -> dict:
    def plain_shell_ready(state: dict) -> bool:
        host = host_for_session_or_none(state, session)
        if host is None:
            return False
        cursor_line_text = str(host.get("cursor_line_text") or host.get("cursor_row_text") or "")
        return (
            normalize_live_path(str(state.get("active_session_path") or "")) == normalize_live_path(session)
            and state.get("active_view_mode") == "Terminal"
            and host.get("input_enabled") is True
            and host.get("xterm_buffer_kind") == "normal"
            and host.get("xterm_cursor_hidden") is False
            and "$" in cursor_line_text
        )

    def wait_for_plain_shell(deadline: float) -> dict | None:
        last_state = {}
        while time.time() < deadline:
            last_state = app_state(pid)
            if plain_shell_ready(last_state):
                return last_state
            time.sleep(0.12)
        return None

    current = wait_for_interactive_session(pid, session, timeout_seconds=20.0)
    if plain_shell_ready(current):
        runtime_probe = assert_terminal_runtime_writable(
            pid,
            session,
            context="plain shell noop runtime probe",
        )
        return {
            "action": "noop",
            "runtime_probe": runtime_probe,
            "state": app_state(pid),
        }

    recovery_steps: list[dict] = []
    deadline = time.time() + 20.0
    for label, kwargs in [
        ("ctrl_c", {"data": "", "mode": "keyboard", "press_ctrl_c": True}),
        ("q", {"data": "q", "mode": "keyboard"}),
        ("escape", {"data": "", "mode": "keyboard"}),
        ("ctrl_c_again", {"data": "", "mode": "keyboard", "press_ctrl_c": True}),
        ("q_enter", {"data": "q", "mode": "keyboard", "press_enter": True}),
    ]:
        recovery_steps.append({
            "label": label,
            "probe": probe_type(pid, session, **kwargs),
        })
        time.sleep(0.3)
        restored = wait_for_plain_shell(min(deadline, time.time() + 3.0))
        if restored is not None:
            runtime_probe = assert_terminal_runtime_writable(
                pid,
                session,
                context=f"plain shell runtime probe after {label}",
            )
            return {
                "action": "restore_plain_shell",
                "recovery_steps": recovery_steps,
                "runtime_probe": runtime_probe,
                "state": app_state(pid),
            }

    final_state = wait_for_plain_shell(deadline)
    if final_state is not None:
        runtime_probe = assert_terminal_runtime_writable(
            pid,
            session,
            context="plain shell runtime probe after recovery deadline",
        )
        return {
            "action": "restore_plain_shell",
            "recovery_steps": recovery_steps,
            "runtime_probe": runtime_probe,
            "state": app_state(pid),
        }
    raise AssertionError(
        f"failed to restore plain shell runtime for session {session!r}: {app_state(pid)!r}"
    )


def assert_codex_session_tui_vitality(pid: int, session: str, out_dir: Path) -> dict:
    command = resolve_codex_session_tui_command()
    if command is None:
        return {"skipped": "codex-session-tui binary not found"}

    clear_probe = clear_prompt_line(pid, session, timeout_seconds=6.0)
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
    vitality_mode = "legacy_browser"
    while time.time() < deadline:
        state = app_state(pid)
        host = host_for_session_or_none(state, session)
        if host is None:
            time.sleep(0.25)
            continue
        text_sample = str(host.get("text_sample") or "")
        cursor_line_text = str(host.get("cursor_line_text") or host.get("cursor_row_text") or "")
        row_text_samples = "\n".join(
            str(row.get("text") or "")
            for row in (
                list(host.get("visible_row_samples_head") or [])
                + list(host.get("visible_row_samples_tail") or [])
            )
            if str(row.get("text") or "").strip()
        )
        tui_text = "\n".join(
            sample for sample in [text_sample, cursor_line_text, row_text_samples] if sample
        )
        if host.get("xterm_buffer_kind") == "normal":
            tui_text_lower = tui_text.lower()
            if (
                "failed to inspect litellm configuration" in tui_text_lower
                or "wire_api = \"chat\"" in tui_text_lower
                or "wire_api = \"responses\"" in tui_text_lower
            ):
                restored = ensure_plain_shell_runtime(pid, session)
                return {
                    "skipped": "codex-session-tui launch failed due to external LiteLLM config",
                    "restore": restored,
                    "text_tail": tui_text[-400:],
                }
        browser_visible = "Browser [0 selected" in tui_text or "Browser [0 sele" in tui_text
        preview_visible = (
            "Preview (Chat) No session selected" in tui_text
            or "Preview (Chat) No session sele" in tui_text
        )
        status_visible = "Status" in tui_text or "┌Status" in tui_text
        machine_visible = (
            "local [ok]" in tui_text
            or "local [o" in tui_text
            or "pi@jojo" in tui_text
            or "pi@openc" in tui_text
        )
        structured_tui_visible = (
            "j/k" in tui_text
            or "toggle" in tui_text
            or "panes" in tui_text
            or "project jump" in tui_text
            or "subtree" in tui_text
            or "folder" in tui_text
        )
        box_frame_visible = (
            "┌" in tui_text
            or "└" in tui_text
            or "│" in tui_text
            or "╭" in tui_text
            or "╰" in tui_text
        )
        if (
            host.get("xterm_buffer_kind") == "alternate"
            and host.get("xterm_cursor_hidden") is True
            and (
                (browser_visible and (preview_visible or status_visible or machine_visible))
                or (status_visible and structured_tui_visible and box_frame_visible)
            )
        ):
            vitality_mode = (
                "legacy_browser"
                if browser_visible and (preview_visible or status_visible or machine_visible)
                else "structured_tui"
            )
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
            int(round(float(host_rect.get("left") or 0)))
            + (240 if vitality_mode == "structured_tui" else 170),
            int(round(float(host_rect.get("top") or 0)))
            + (120 if vitality_mode == "structured_tui" else 92),
        ),
        image.size,
    )
    if browser_crop_box is None:
        raise AssertionError("codex-session-tui browser vitality crop fell outside the screenshot")
    browser_crop = image.crop(browser_crop_box)
    dark_pixels = count_dark_foreground_pixels(browser_crop)
    minimum_dark_pixels = 150 if vitality_mode == "structured_tui" else 500
    if dark_pixels < minimum_dark_pixels:
        raise AssertionError(
            "codex-session-tui browser rows painted too faintly in the screenshot: "
            f"mode={vitality_mode} dark_pixels={dark_pixels} min_dark_pixels={minimum_dark_pixels} crop={browser_crop_box}"
        )
    colorful_pixels, colorful_hue_buckets = colorful_foreground_stats(browser_crop)
    minimum_colorful_pixels = 16 if vitality_mode == "structured_tui" else 32
    minimum_colorful_hue_buckets = 1 if vitality_mode == "structured_tui" else 2
    if (
        colorful_pixels < minimum_colorful_pixels
        or colorful_hue_buckets < minimum_colorful_hue_buckets
    ):
        raise AssertionError(
            "codex-session-tui browser crop still looks visually flat in the screenshot: "
            f"mode={vitality_mode} colorful_pixels={colorful_pixels} min_colorful_pixels={minimum_colorful_pixels} "
            f"colorful_hue_buckets={colorful_hue_buckets} min_colorful_hue_buckets={minimum_colorful_hue_buckets} "
            f"crop={browser_crop_box}"
        )
    host_left = int(round(float(host_rect.get("left") or 0)))
    host_top = int(round(float(host_rect.get("top") or 0)))
    host_right = int(round(float(host_rect.get("right") or 0)))
    host_bottom = int(round(float(host_rect.get("bottom") or 0)))
    host_width = max(1, host_right - host_left)
    host_height = max(1, host_bottom - host_top)
    preview_crop_box = clamp_box(
        (
            host_left + max(120, min(182, int(round(host_width * 0.22)))),
            host_top + 4,
            host_right - 12,
            host_top + min(104, max(64, int(round(host_height * 0.16)))),
        ),
        image.size,
    )
    if preview_crop_box is None:
        preview_crop_box = clamp_box(
            (
                max(0, image.size[0] // 2),
                max(0, host_top + 4),
                max(0, image.size[0] - 12),
                min(image.size[1], max(host_top + 72, image.size[1] // 5)),
            ),
            image.size,
        )
    if preview_crop_box is None:
        raise AssertionError("codex-session-tui preview vitality crop fell outside the screenshot")
    preview_crop = image.crop(preview_crop_box)
    preview_dark_pixels = count_dark_foreground_pixels(preview_crop)
    minimum_preview_dark_pixels = 220 if vitality_mode == "structured_tui" else 300
    if preview_dark_pixels < minimum_preview_dark_pixels:
        raise AssertionError(
            "codex-session-tui preview pane painted too faintly in the screenshot: "
            f"mode={vitality_mode} dark_pixels={preview_dark_pixels} min_dark_pixels={minimum_preview_dark_pixels} crop={preview_crop_box}"
        )
    status_crop_box = clamp_box(
        (
            int(round(float(host_rect.get("left") or 0))) + 6,
            int(round(float(host_rect.get("bottom") or 0))) - 34,
            int(round(float(host_rect.get("right") or 0))) - 6,
            int(round(float(host_rect.get("bottom") or 0))) - 6,
        ),
        image.size,
    )
    if status_crop_box is None:
        status_crop_box = clamp_box(
            (
                max(0, host_left + 6),
                max(0, image.size[1] - max(40, min(72, image.size[1] // 10))),
                max(0, image.size[0] - 6),
                max(0, image.size[1] - 6),
            ),
            image.size,
        )
    if status_crop_box is None:
        raise AssertionError("codex-session-tui status vitality crop fell outside the screenshot")
    status_crop = image.crop(status_crop_box)
    status_non_background_pixels, status_background = count_non_background_pixels(
        status_crop,
        tolerance=12,
    )
    if status_non_background_pixels < 1600:
        raise AssertionError(
            "codex-session-tui status strip painted too faintly in the screenshot: "
            f"non_background_pixels={status_non_background_pixels} min_non_background_pixels=1600 "
            f"background={status_background} crop={status_crop_box}"
        )

    restored = restore_prompt_after_codex_session_tui(
        pid,
        session,
        timeout_seconds=12.0,
    )
    restored_host = host_for_session(restored, session)
    return {
        "clear_probe": clear_probe,
        "command": command,
        "screenshot": str(screenshot_path),
        "browser_crop_box": {
            "left": browser_crop_box[0],
            "top": browser_crop_box[1],
            "width": browser_crop_box[2] - browser_crop_box[0],
            "height": browser_crop_box[3] - browser_crop_box[1],
        },
        "preview_crop_box": {
            "left": preview_crop_box[0],
            "top": preview_crop_box[1],
            "width": preview_crop_box[2] - preview_crop_box[0],
            "height": preview_crop_box[3] - preview_crop_box[1],
        },
        "status_crop_box": {
            "left": status_crop_box[0],
            "top": status_crop_box[1],
            "width": status_crop_box[2] - status_crop_box[0],
            "height": status_crop_box[3] - status_crop_box[1],
        },
        "vitality_mode": vitality_mode,
        "browser_dark_pixels": dark_pixels,
        "browser_colorful_pixels": colorful_pixels,
        "browser_colorful_hue_buckets": colorful_hue_buckets,
        "preview_dark_pixels": preview_dark_pixels,
        "status_non_background_pixels": status_non_background_pixels,
        "status_background": {
            "r": status_background[0],
            "g": status_background[1],
            "b": status_background[2],
        },
        "renderer_mode": host.get("xterm_renderer_mode"),
        "xterm_minimum_contrast_ratio": host.get("xterm_minimum_contrast_ratio"),
        "xterm_font_weight": host.get("xterm_font_weight"),
        "xterm_line_height": host.get("xterm_line_height"),
        "restored_renderer_mode": restored_host.get("xterm_renderer_mode"),
    }


def assert_hidden_cursor_tui(pid: int, session: str, out_dir: Path) -> dict:
    clear = terminal_send(pid, session, "\u0003")
    wait_for_terminal_quiescent(pid, timeout_seconds=6.0)
    command = "printf '\\033[?1049h\\033[?25lhc'; sleep 1; printf '\\033[?25h\\033[?1049l'"
    probe = terminal_send(pid, session, f"{command}\n")
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
    }


def assert_status_command(pid: int, session: str, out_dir: Path) -> dict:
    ensure = ensure_live_codex_runtime(pid, session)
    wait_for_session_focus(pid, session, timeout_seconds=12.0)
    prime_terminal_surface_for_keyboard(pid, session)
    clear_terminal_selection(pid, session, timeout_seconds=4.0)
    wait_for_terminal_quiescent(pid, timeout_seconds=12.0)
    prime_terminal_surface_for_keyboard(pid, session)
    clear = clear_prompt_line(pid, session, timeout_seconds=8.0)
    cleared_state = clear.get("state") or app_state(pid)
    cleared_host = host_for_session_or_none(cleared_state, session) or active_host_or_none(cleared_state) or {}
    cleared_cursor_line = str(
        cleared_host.get("cursor_line_text") or cleared_host.get("cursor_row_text") or ""
    )
    cleared_text_sample = str(cleared_host.get("text_sample") or "")
    if "/status" in cleared_cursor_line or "/status" in cleared_text_sample:
        raise AssertionError(
            "status command prompt was not clean before typing: "
            f"cursor_line={cleared_cursor_line!r} text_tail={cleared_text_sample[-200:]!r}"
        )
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
    cursor_cell_pixels = assert_cursor_cell_glyph_pixels_visible(
        shot_path,
        state,
        context="after /status",
    )
    return {
        "ensure_live_codex": ensure,
        "clear_probe": clear,
        "typed_probe": typed_probe,
        "probe": probe,
        "settled": settled,
        "screenshot": str(shot_path),
        "cursor_line_text": cursor_line_text,
        "cursor_glyph": cursor_glyph,
        "cursor_cell_pixels": cursor_cell_pixels,
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
        active_session = str(state.get("active_session_path") or "")
        expected_bg = "#1e1e1e" if theme == "dark" else "#fbfbfd"
        if (state.get("settings") or {}).get("theme") != theme:
            raise AssertionError(f"UI theme did not switch to {theme!r}: {state.get('settings')!r}")
        if host.get("xterm_theme_background") != expected_bg:
            raise AssertionError(
                f"xterm background did not track {theme} mode: {host.get('xterm_theme_background')!r}"
            )
        assert_text_readability(state)
        if active_session:
            focus_session = active_session
            if focus_session.startswith("local://"):
                try:
                    ensure_plain_shell_runtime(pid, focus_session)
                except AssertionError:
                    created = app_create_terminal(pid, title="Smoke Theme Terminal")
                    focus_session = str(created.get("active_session_path") or "").strip() or focus_session
                    if focus_session.startswith("local://"):
                        ensure_plain_shell_runtime(pid, focus_session)
            terminal_reclaim_focus(pid, focus_session)
            state = wait_for_visible_cursor_session(pid, focus_session, timeout_seconds=10.0)
            host = active_host(state)
        assert_cursor_alignment(state)
        shot_path = out_dir / f"theme-{theme}.png"
        app_screenshot(pid, shot_path)
        results[theme] = {
            "background": host.get("xterm_theme_background"),
            "rows_sample_color": host.get("rows_sample_color"),
            "dim_sample_color": host.get("dim_sample_color"),
            "session": str(state.get("active_session_path") or ""),
            "screenshot": str(shot_path),
        }
    return results


def main() -> int:
    global BIN
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin")
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--session", required=True)
    parser.add_argument("--session-kind", choices=("codex", "plain"), default="codex")
    parser.add_argument("--out", default="/tmp/xterm-embed-faults")
    parser.add_argument("--reopen", action="store_true")
    parser.add_argument("--home")
    args = parser.parse_args()

    if args.home:
        ENV["YGGTERM_HOME"] = str(Path(args.home).expanduser())
    if args.bin:
        BIN = Path(args.bin).expanduser()
        ENV["YGGTERM_BIN"] = str(BIN)

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)
    requested_session = args.session
    disposable_codex_session: str | None = None
    disposable_codex_create: dict | None = None

    initial_state = app_state(args.pid)
    if args.session_kind != "codex" and (args.reopen or (
        initial_state.get("active_session_path") != args.session
        or initial_state.get("active_view_mode") != "Terminal"
    )):
        app_open(args.pid, args.session, view="terminal")
    initial_settle = "interactive_only"
    if args.session_kind == "plain":
        try:
            state = ensure_plain_shell_runtime(args.pid, args.session)["state"]
            initial_settle = "quiescent"
        except AssertionError:
            created = app_create_terminal(args.pid, title="Smoke Plain Terminal")
            fresh_session = str(created.get("active_session_path") or "").strip()
            if not fresh_session:
                raise
            args.session = fresh_session
            state = ensure_plain_shell_runtime(args.pid, args.session)["state"]
            initial_settle = "quiescent_fresh_terminal"
    elif args.session_kind == "codex":
        disposable_codex_create = create_terminal_via_sidebar_context_menu(args.pid)
        disposable_codex_session = disposable_codex_create["session_path"]
        args.session = disposable_codex_session
        state = ensure_live_codex_runtime(args.pid, args.session)["state"]
        initial_settle = "interactive_disposable_codex_session"
    else:
        state = wait_for_interactive_session(args.pid, args.session, timeout_seconds=25.0)
    state, initial_prompt_pixels = capture_prompt_visible_screenshot(
        args.pid,
        args.session,
        out_dir / "initial.png",
        context="initial screenshot",
        timeout_seconds=8.0,
    )
    with (out_dir / "initial-state.json").open("w") as fh:
        json.dump(state, fh, indent=2)

    summary = {
        "pid": args.pid,
        "session": args.session,
        "requested_session": requested_session,
        "session_kind": args.session_kind,
        "initial_settle": initial_settle,
        "pointer_driver": POINTER_DRIVER,
        "key_driver": KEY_DRIVER,
        "checks": {},
    }
    if disposable_codex_create is not None:
        summary["checks"]["disposable_codex_session_created"] = disposable_codex_create

    def run_check(name: str, fn):
        print(f"RUN {name}", flush=True)
        result = fn()
        summary["checks"][name] = result
        print(f"PASS {name}", flush=True)
        return result

    disposable_codex_deleted = False
    try:
        run_check("initial_prompt_pixels", lambda: initial_prompt_pixels)
        if args.session_kind == "codex":
            run_check("codex_startup_health", lambda: assert_codex_startup_health(args.pid, args.session, out_dir))
        run_check("startup_bootstrap_dedupe", lambda: assert_no_duplicate_startup_terminal_bootstrap(args.pid))
        run_check("focus", lambda: assert_focus_and_visibility(args.pid, state))
        run_check("geometry", lambda: assert_geometry(state))
        run_check("notification_health_initial", lambda: assert_no_problem_notifications(state, context="initial"))
        titlebar_state, titlebar_centering = wait_for_titlebar_centering(args.pid)
        state = titlebar_state
        run_check("titlebar_centering", lambda: titlebar_centering)
        run_check("terminal_viewport_inset", lambda: assert_terminal_viewport_inset(state))
        run_check("background_blur", lambda: assert_background_blur_contract(args.pid))
        run_check("sidebar_resize", lambda: assert_sidebar_resize_persists_to_settings(args.pid))
        run_check("search_focus_overlay", lambda: assert_search_focus_overlay_contract(args.pid, args.session))
        run_check("titlebar_new_menu_shell", lambda: assert_titlebar_new_menu_shell_contract(args.pid))
        run_check("titlebar_session_shell", lambda: assert_titlebar_session_shell_contract(args.pid))
        run_check("titlebar_modal_visual_parity", lambda: assert_titlebar_modal_visual_parity(args.pid))
        run_check(
            "live_session_metadata_consistency",
            lambda: assert_live_session_metadata_consistency(args.pid, args.session),
        )
        run_check("titlebar_overflow_menu", lambda: assert_titlebar_overflow_menu_contract(args.pid))
        run_check("settings_terminal_reclaim", lambda: assert_settings_terminal_reclaim_contract(args.pid))
        run_check("right_panel_animation", lambda: assert_right_panel_animation_contract(args.pid))
        run_check("context_menu_rename_session", lambda: assert_context_menu_rename_session(args.pid))
        run_check("context_menu_delete_session", lambda: assert_context_menu_delete_session(args.pid))
        run_check(
            "titlebar_empty_lane_window_controls",
            lambda: assert_titlebar_empty_lane_window_controls_contract(args.pid, out_dir),
        )
        run_check("titlebar_autohide_hover", lambda: assert_titlebar_autohide_hover_contract(args.pid, out_dir))
        run_check("theme_editor_contract", lambda: assert_theme_editor_contract(args.pid, out_dir))
        run_check("terminal_zoom_live_apply", lambda: assert_terminal_zoom_live_apply(
            args.pid, args.session, out_dir
        ))
        run_check("maximize_roundtrip_layout", lambda: assert_maximize_roundtrip_layout(args.pid, out_dir))
        run_check("webkit_child_rss_soak", lambda: assert_webkit_child_rss_soak(args.pid))
        if args.session_kind != "codex":
            run_check(
                "managed_cli_initial_install_deferred",
                lambda: assert_managed_cli_initial_install_deferred_contract(),
            )
        run_check("managed_cli_refresh_ttl", lambda: assert_managed_cli_refresh_ttl_contract())
        run_check("client_memory_budget", lambda: assert_client_memory_budget(args.pid))
        late_phase_session = args.session
        late_phase_session_reset: dict | None = None
        if args.session:
            if args.session_kind == "plain":
                try:
                    late_phase_session_reset = ensure_plain_shell_runtime(args.pid, args.session)
                    state = late_phase_session_reset["state"]
                except AssertionError:
                    created = app_create_terminal(args.pid, title="Smoke Plain Terminal")
                    late_phase_session = str(created.get("active_session_path") or "").strip()
                    if not late_phase_session:
                        raise
                    late_phase_session_reset = {
                        "action": "spawn_fresh_late_phase_terminal",
                        "create": created,
                    }
                    state = ensure_plain_shell_runtime(args.pid, late_phase_session)["state"]
            terminal_reclaim_focus(args.pid, late_phase_session)
            state = wait_for_visible_cursor_session(args.pid, late_phase_session, timeout_seconds=10.0)
        summary["late_phase_session"] = late_phase_session
        if late_phase_session_reset is not None:
            summary["checks"]["late_phase_session_reset"] = late_phase_session_reset
        run_check("renderer", lambda: assert_renderer_contract(state))
        run_check("readability", lambda: assert_text_readability(state))
        run_check("cursor", lambda: assert_cursor_alignment(state))
        run_check("cursor_glyph", lambda: assert_cursor_glyph_visibility(state))
        run_check("cursor_focus_blur", lambda: assert_cursor_focus_blur_contract(args.pid, late_phase_session))
        run_check("cursor_mouse_hold", lambda: assert_cursor_mouse_hold_contract(args.pid, late_phase_session))
        run_check("live_sessions_restore", lambda: assert_live_sessions_restore_visibility(args.pid))
        run_check("selection", lambda: assert_selection(args.pid, late_phase_session))
        run_check(
            "terminal_interaction_latency",
            lambda: assert_terminal_interaction_latency(args.pid, late_phase_session),
        )
        if args.session_kind == "plain":
            run_check("clipboard_image", lambda: assert_clipboard_image_contract(
                args.pid, late_phase_session, out_dir
            ))
            run_check("partial_input", lambda: assert_partial_input_flow(args.pid, late_phase_session, out_dir))
            run_check("scroll", lambda: assert_scroll(args.pid, late_phase_session, state))
            run_check("hidden_cursor_tui", lambda: assert_hidden_cursor_tui(args.pid, late_phase_session, out_dir))
            if late_phase_session.startswith("local://"):
                run_check("codex_session_tui_vitality", lambda: assert_codex_session_tui_vitality(
                    args.pid, late_phase_session, out_dir
                ))
        if args.session_kind == "codex":
            run_check("status_command", lambda: assert_status_command(args.pid, late_phase_session, out_dir))
        if late_phase_session.startswith("local://"):
            run_check("local_tree", lambda: assert_local_tree_placement(args.pid, late_phase_session))
            run_check("sidebar_contract", lambda: assert_sidebar_contract(args.pid, late_phase_session))
            run_check(
                "live_session_metadata",
                lambda: assert_live_session_metadata_consistency(args.pid, late_phase_session),
            )
            run_check("local_runtime", lambda: assert_local_session_runtime_ready(late_phase_session))
            run_check("hot_session_switch", lambda: assert_hot_session_switch(
                args.pid, args.session, args.session_kind, out_dir
            ))
        run_check("observability_budget", lambda: assert_observability_budget())
        run_check("themes", lambda: assert_theme_contract(args.pid, out_dir))
        run_check("idle_root_render_budget", lambda: assert_idle_root_render_budget(args.pid))
        run_check(
            "notification_health_final",
            lambda: assert_no_problem_notifications(app_state(args.pid), context="final"),
        )
        final_state = app_state(args.pid)
        with (out_dir / "final-state.json").open("w") as fh:
            json.dump(final_state, fh, indent=2)
        app_screenshot(args.pid, out_dir / "final.png")
        if disposable_codex_session:
            summary["checks"]["disposable_codex_session_deleted"] = delete_session_via_context_menu(
                args.pid, disposable_codex_session
            )
            disposable_codex_deleted = True
        print(json.dumps(summary, indent=2))
        return 0
    finally:
        if disposable_codex_session and not disposable_codex_deleted:
            try:
                summary["checks"]["disposable_codex_session_cleanup_fallback"] = app_remove_session(
                    args.pid, disposable_codex_session
                )
            except Exception:
                pass
        try:
            with (out_dir / "summary.json").open("w") as fh:
                json.dump(summary, fh, indent=2)
        except Exception:
            pass


if __name__ == "__main__":
    raise SystemExit(main())
