#!/usr/bin/env python3
import argparse
import json
import subprocess
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "target" / "debug" / "yggterm"


def run(*args: str, check: bool = True) -> dict:
    proc = subprocess.run(
        [str(BIN), *args],
        cwd=ROOT,
        text=True,
        capture_output=True,
    )
    if check and proc.returncode != 0:
        raise AssertionError(
            proc.stderr.strip() or proc.stdout.strip() or f"command failed: {args!r}"
        )
    text = proc.stdout.strip()
    return json.loads(text) if text else {}


def app_state(pid: int) -> dict:
    return run("server", "app", "state", "--pid", str(pid), "--timeout-ms", "8000")["data"]


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


def app_rows(pid: int) -> list[dict]:
    payload = run("server", "app", "rows", "--pid", str(pid), "--timeout-ms", "8000")
    return ((payload.get("data") or {}).get("rows") or [])


def unwrap_data(payload: dict) -> dict:
    data = payload.get("data")
    if isinstance(data, dict):
        return data
    return payload


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


def active_host(state: dict) -> dict:
    hosts = terminal_hosts(state)
    if not hosts:
        raise AssertionError("no terminal host found in app state")
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


def wait_for_interactive(pid: int, timeout_seconds: float = 20.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        last_state = app_state(pid)
        viewport = last_state.get("viewport") or {}
        host = active_host(last_state)
        if (
            viewport.get("ready") is True
            and viewport.get("interactive") is True
            and viewport.get("terminal_settled_kind") == "interactive"
            and host.get("input_enabled") is True
            and not ((last_state.get("shell") or {}).get("notifications") or [])
            and not ((viewport.get("active_terminal_surface") or {}).get("problem"))
        ):
            return last_state
        time.sleep(0.25)
    raise AssertionError(f"terminal did not settle interactive: {last_state!r}")


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
    if rect_is_visible(helper_textarea_rect):
        if abs(float(helper_textarea_rect["left"]) - float(host_rect["left"])) > 4.0:
            raise AssertionError(
                f"helper textarea drifted away from host left edge: helper={helper_textarea_rect!r} host={host_rect!r}"
            )
        if abs(float(helper_textarea_rect["top"]) - float(host_rect["top"])) > 4.0:
            raise AssertionError(
                f"helper textarea drifted away from host top edge: helper={helper_textarea_rect!r} host={host_rect!r}"
            )
    return {
        "host_rect": host_rect,
        "screen_rect": screen_rect,
        "viewport_rect": viewport_rect,
        "helpers_rect": helpers_rect,
        "helper_textarea_rect": helper_textarea_rect,
    }


def assert_focus_and_visibility(state: dict) -> dict:
    viewport = state.get("viewport") or {}
    host = active_host(state)
    active_element = ((state.get("dom") or {}).get("active_element") or {})
    notifications = ((state.get("shell") or {}).get("notifications") or [])
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
    if str(host.get("rows_sample_font_weight") or host.get("rows_font_weight")) != "500":
        raise AssertionError(
            f"rows font weight drifted: {host.get('rows_sample_font_weight') or host.get('rows_font_weight')!r}"
        )
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
    }


def assert_renderer_contract(state: dict) -> dict:
    host = active_host(state)
    canvas_count = int(host.get("canvas_count") or 0)
    renderer_mode = str(host.get("xterm_renderer_mode") or "")
    if canvas_count <= 0:
        raise AssertionError(
            f"xterm did not mount the canvas renderer: canvas_count={canvas_count} renderer_mode={renderer_mode!r}"
        )
    return {
        "canvas_count": canvas_count,
        "renderer_mode": renderer_mode or "canvas",
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
        if glyph_contrast is None or glyph_contrast < 7.0:
            raise AssertionError(
                f"cursor glyph contrast too low: text={cursor_text!r} color={cursor_color!r} background={row_background!r} contrast={glyph_contrast!r}"
            )
    return {
        "cursor_sample_text": cursor_text,
        "cursor_sample_color": cursor_color,
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


def assert_scroll(pid: int, session: str, before_state: dict) -> dict:
    host = active_host(before_state)
    before_viewport_y = int(host.get("viewport_y") or 0)
    base_y = int(host.get("base_y") or 0)
    if base_y <= before_viewport_y <= 0:
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


def assert_partial_input_flow(pid: int, session: str, out_dir: Path) -> dict:
    wait_for_terminal_quiescent(pid, timeout_seconds=12.0)
    clear = probe_type(pid, session, "", mode="keyboard", press_ctrl_c=True, press_ctrl_e=True, press_ctrl_u=True)
    time.sleep(0.3)
    wait_for_terminal_quiescent(pid, timeout_seconds=8.0)
    typed = probe_type(pid, session, "/sta", mode="keyboard")
    time.sleep(0.4)
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
        "typed_probe": typed,
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


def assert_hidden_cursor_tui(pid: int, session: str, out_dir: Path) -> dict:
    clear = probe_type(pid, session, "", mode="keyboard", press_ctrl_c=True, press_ctrl_e=True, press_ctrl_u=True)
    time.sleep(0.2)
    command = "sh -lc 'tput smcup;tput civis;printf hc;sleep 3;tput cnorm;tput rmcup'"
    probe = probe_type(pid, session, command, mode="keyboard", press_enter=True)
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
    clear = probe_type(pid, session, "", mode="keyboard", press_ctrl_e=True, press_ctrl_u=True)
    time.sleep(0.3)
    probe = probe_type(pid, session, "/status", mode="keyboard", press_enter=True)
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
    if "/status" not in text_sample and "/status" not in cursor_line_text:
        raise AssertionError("typed /status is not visible in terminal text sample")
    if "OpenAI Codex" not in text_sample and "Session:" not in text_sample:
        raise AssertionError("Codex status panel is not visible after /status<Enter>")
    assert_cursor_alignment(state)
    cursor_glyph = assert_cursor_glyph_visibility(state)
    prompt_anchor = assert_cursor_prompt_visibility(state, context="after /status")
    return {
        "ensure_live_codex": ensure,
        "clear_probe": clear,
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
    args = parser.parse_args()

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
        state = wait_for_terminal_quiescent(args.pid, timeout_seconds=25.0)
        initial_settle = "quiescent"
    elif args.session_kind == "codex":
        state = ensure_live_codex_runtime(args.pid, args.session)["state"]
    app_screenshot(args.pid, out_dir / "initial.png")
    with (out_dir / "initial-state.json").open("w") as fh:
        json.dump(state, fh, indent=2)

    summary = {
        "pid": args.pid,
        "session": args.session,
        "session_kind": args.session_kind,
        "initial_settle": initial_settle,
        "checks": {},
    }

    summary["checks"]["focus"] = assert_focus_and_visibility(state)
    summary["checks"]["geometry"] = assert_geometry(state)
    summary["checks"]["renderer"] = assert_renderer_contract(state)
    summary["checks"]["readability"] = assert_text_readability(state)
    summary["checks"]["cursor"] = assert_cursor_alignment(state)
    summary["checks"]["cursor_glyph"] = assert_cursor_glyph_visibility(state)
    summary["checks"]["selection"] = assert_selection(args.pid, args.session)
    if args.session_kind == "plain":
        summary["checks"]["partial_input"] = assert_partial_input_flow(args.pid, args.session, out_dir)
        summary["checks"]["scroll"] = assert_scroll(args.pid, args.session, state)
        summary["checks"]["hidden_cursor_tui"] = assert_hidden_cursor_tui(args.pid, args.session, out_dir)
    if args.session_kind == "codex":
        summary["checks"]["status_command"] = assert_status_command(args.pid, args.session, out_dir)
    if args.session.startswith("local://"):
        summary["checks"]["local_tree"] = assert_local_tree_placement(args.pid, args.session)
    summary["checks"]["themes"] = assert_theme_contract(args.pid, out_dir)
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
