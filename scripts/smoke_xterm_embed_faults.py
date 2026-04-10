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


def is_transparent_css_color(value: str | None) -> bool:
    text = str(value or "").strip().lower()
    return text in ("", "transparent", "rgba(0, 0, 0, 0)", "rgba(0,0,0,0)")


def active_host(state: dict) -> dict:
    viewport = state.get("viewport") or {}
    hosts = viewport.get("terminal_hosts") or []
    if not hosts:
        raise AssertionError("no terminal host found in app state")
    return hosts[0]


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
    row_contrast = contrast_ratio(rows_color, bg)
    dim_contrast = contrast_ratio(dim_color, bg) if dim_color else None
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
    if row_contrast is None or row_contrast < 7.0:
        raise AssertionError(
            f"main row contrast too low: color={rows_color!r} background={bg!r} contrast={row_contrast!r}"
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
        "dim_sample_color": dim_color,
        "dim_contrast": round(dim_contrast, 2) if dim_contrast is not None else None,
        "low_contrast_span_count": low_contrast_count,
    }


def assert_cursor_alignment(state: dict) -> dict:
    host = active_host(state)
    overlay_rect = host.get("cursor_overlay_rect") or {}
    expected_rect = host.get("cursor_expected_rect") or {}
    cursor_rect = host.get("cursor_sample_rect") or {}
    if host.get("cursor_overlay_present") is not True:
        raise AssertionError("cursor overlay is missing")
    if host.get("cursor_overlay_display") in ("", "none"):
        raise AssertionError(f"cursor overlay is hidden: {host.get('cursor_overlay_display')!r}")
    if not rect_is_visible(overlay_rect):
        raise AssertionError(f"cursor overlay rect is not visible: {overlay_rect!r}")
    if not rect_is_visible(expected_rect):
        raise AssertionError(
            f"expected cursor cell rect is missing/empty, cannot prove alignment: {expected_rect!r}"
        )
    dx = abs(float(overlay_rect["left"]) - float(expected_rect["left"]))
    dy = abs(float(overlay_rect["top"]) - float(expected_rect["top"]))
    dw = abs(float(overlay_rect["width"]) - float(expected_rect["width"]))
    dh = abs(float(overlay_rect["height"]) - float(expected_rect["height"]))
    if dx > 4.0 or dy > 4.0 or dw > 8.0 or dh > 8.0:
        raise AssertionError(
            "cursor overlay drifted from expected cursor cell: "
            f"overlay={overlay_rect!r} expected={expected_rect!r} dx={dx:.2f} dy={dy:.2f} dw={dw:.2f} dh={dh:.2f}"
        )
    if rect_is_visible(cursor_rect) and float(cursor_rect["width"]) > float(overlay_rect["width"]) * 4.0:
        background = host.get("cursor_sample_background")
        border_left = host.get("cursor_sample_border_left")
        border_bottom = host.get("cursor_sample_border_bottom")
        outline_style = str(host.get("cursor_sample_outline_style") or "").strip().lower()
        box_shadow = str(host.get("cursor_sample_box_shadow") or "").strip().lower()
        if (
            not is_transparent_css_color(background)
            or not is_transparent_css_color(border_left)
            or not is_transparent_css_color(border_bottom)
            or (outline_style not in ("", "none"))
            or box_shadow not in ("", "none")
        ):
            raise AssertionError(
                "rendered xterm cursor span is wide and still visually active: "
                f"rect={cursor_rect!r} background={background!r} border_left={border_left!r} "
                f"border_bottom={border_bottom!r} outline_style={outline_style!r} box_shadow={box_shadow!r}"
            )
    return {
        "cursor_overlay_rect": overlay_rect,
        "cursor_expected_rect": expected_rect,
        "cursor_sample_rect": cursor_rect,
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
            },
            "after": {
                "base_y": base_y,
                "viewport_y": before_viewport_y,
            },
            "reason": "no_scrollback_available",
        }
    lines = -5 if before_viewport_y > 0 else 5 if base_y > before_viewport_y else -5
    first = probe_scroll(pid, session, lines)
    before = first.get("before") or {}
    after = first.get("after") or {}
    if before.get("viewport_y") == after.get("viewport_y"):
        second = probe_scroll(pid, session, -lines)
        before = second.get("before") or {}
        after = second.get("after") or {}
        if before.get("viewport_y") == after.get("viewport_y"):
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


def assert_status_command(pid: int, session: str, out_dir: Path) -> dict:
    probe = probe_type(pid, session, "/status", mode="keyboard", press_enter=True)
    time.sleep(1.0)
    state = wait_for_interactive(pid, timeout_seconds=20.0)
    shot_path = out_dir / "after-status.png"
    app_screenshot(pid, shot_path)
    host = active_host(state)
    text_sample = str(host.get("text_sample") or "")
    cursor_line_text = str(host.get("cursor_line_text") or "")
    if "/status" not in text_sample and "/status" not in cursor_line_text:
        raise AssertionError("typed /status is not visible in terminal text sample")
    if "OpenAI Codex" not in text_sample and "Session:" not in text_sample:
        raise AssertionError("Codex status panel is not visible after /status<Enter>")
    assert_cursor_alignment(state)
    return {
        "probe": probe,
        "screenshot": str(shot_path),
        "cursor_line_text": cursor_line_text,
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
    args = parser.parse_args()

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    app_open(args.pid, args.session, view="terminal")
    state = wait_for_interactive(args.pid, timeout_seconds=25.0)
    app_screenshot(args.pid, out_dir / "initial.png")
    with (out_dir / "initial-state.json").open("w") as fh:
        json.dump(state, fh, indent=2)

    summary = {
        "pid": args.pid,
        "session": args.session,
        "session_kind": args.session_kind,
        "checks": {},
    }

    summary["checks"]["focus"] = assert_focus_and_visibility(state)
    summary["checks"]["geometry"] = assert_geometry(state)
    summary["checks"]["readability"] = assert_text_readability(state)
    summary["checks"]["cursor"] = assert_cursor_alignment(state)
    summary["checks"]["selection"] = assert_selection(args.pid, args.session)
    summary["checks"]["scroll"] = assert_scroll(args.pid, args.session, state)
    if args.session_kind == "codex":
        summary["checks"]["status_command"] = assert_status_command(args.pid, args.session, out_dir)
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
