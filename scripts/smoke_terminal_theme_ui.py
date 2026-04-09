#!/usr/bin/env python3
import argparse
import json
import subprocess
import sys
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
        raise SystemExit(proc.stderr.strip() or proc.stdout.strip() or f"command failed: {args}")
    text = proc.stdout.strip()
    return json.loads(text) if text else {}


def app_state(pid: int) -> dict:
    return run("server", "app", "state", "--pid", str(pid), "--timeout-ms", "8000")["data"]


def wait_for_terminal_settled(pid: int, timeout_seconds: float = 15.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        last_state = app_state(pid)
        viewport = last_state.get("viewport") or {}
        hosts = viewport.get("terminal_hosts") or []
        if (
            viewport.get("ready") is True
            and viewport.get("interactive") is True
            and hosts
            and hosts[0].get("input_enabled") is True
        ):
            return last_state
        time.sleep(0.2)
    raise AssertionError(f"terminal did not settle in time: {last_state!r}")


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


def assert_terminal_font_contract(state: dict, expect_bg: str) -> None:
    hosts = state["viewport"]["terminal_hosts"]
    if not hosts:
        raise AssertionError("no terminal host found")
    host = hosts[0]
    host_width = ((host.get("host_rect") or {}).get("width")) or ((host.get("viewport_rect") or {}).get("width"))
    screen_width = (host.get("screen_rect") or {}).get("width")
    xterm_family = host.get("xterm_font_family") or ""
    rows_family = host.get("rows_sample_font_family") or host.get("rows_font_family") or ""
    rows_weight = host.get("rows_sample_font_weight") or host.get("rows_font_weight") or ""
    rows_features = (
        host.get("rows_sample_font_feature_settings")
        or host.get("rows_font_feature_settings")
        or ""
    )
    rows_spacing = host.get("rows_sample_letter_spacing") or host.get("rows_letter_spacing") or ""
    rows_line_height = host.get("rows_sample_line_height") or host.get("rows_line_height") or ""
    rows_color = host.get("rows_sample_color") or host.get("rows_color") or ""
    dim_color = host.get("dim_sample_color") or ""
    dim_opacity = host.get("dim_sample_opacity") or ""
    cursor_class = host.get("cursor_sample_class_name") or ""
    cursor_background = host.get("cursor_sample_background") or ""
    cursor_border_left = host.get("cursor_sample_border_left") or ""
    cursor_border_bottom = host.get("cursor_sample_border_bottom") or ""
    cursor_outline = host.get("cursor_sample_outline") or ""
    if not xterm_family:
        raise AssertionError("xterm font family missing; terminal runtime likely did not fully mount")
    if "JetBrains Mono" not in xterm_family:
        raise AssertionError(f"xterm font family drifted: {xterm_family!r}")
    if "JetBrains Mono" not in rows_family:
        raise AssertionError(f"rows font family drifted: {rows_family!r}")
    if rows_family.startswith('"') and rows_family.endswith('"'):
        raise AssertionError(f"rows font family is still quoted as one literal: {rows_family!r}")
    if str(host.get("xterm_font_weight")) != "500":
        raise AssertionError(f"xterm font weight drifted: {host.get('xterm_font_weight')!r}")
    if str(host.get("xterm_font_weight_bold")) != "700":
        raise AssertionError(
            f"xterm bold font weight drifted: {host.get('xterm_font_weight_bold')!r}"
        )
    try:
        xterm_line_height = float(str(host.get("xterm_line_height")))
    except (TypeError, ValueError):
        raise AssertionError(f"xterm line height drifted: {host.get('xterm_line_height')!r}")
    if abs(xterm_line_height - 1.12) > 0.01:
        raise AssertionError(f"xterm line height drifted: {host.get('xterm_line_height')!r}")
    if rows_weight != "500":
        raise AssertionError(f"rows font weight drifted: {rows_weight!r}")
    if '"liga" 0' not in rows_features and '"calt" 0' not in rows_features:
        raise AssertionError(f"rows font features drifted: {rows_features!r}")
    if rows_spacing not in ("0px", "0", "normal"):
        raise AssertionError(f"rows letter spacing drifted: {rows_spacing!r}")
    if rows_line_height in ("", "normal"):
        raise AssertionError(f"rows line height drifted: {rows_line_height!r}")
    if not rows_color:
        raise AssertionError("rows color missing from app-control state")
    sample_contrast = contrast_ratio(rows_color, expect_bg)
    if sample_contrast is None or sample_contrast < 4.5:
        raise AssertionError(
            f"rows sample contrast drifted: color={rows_color!r} background={expect_bg!r} contrast={sample_contrast!r}"
        )
    if dim_color:
        dim_contrast = contrast_ratio(dim_color, expect_bg)
        if dim_contrast is None or dim_contrast < 3.0:
            raise AssertionError(
                f"dim text contrast drifted: color={dim_color!r} background={expect_bg!r} contrast={dim_contrast!r}"
            )
    if dim_opacity not in ("", "1", "1.0"):
        raise AssertionError(f"dim text opacity drifted: {dim_opacity!r}")
    if cursor_class and "xterm-cursor" not in cursor_class:
        raise AssertionError(f"cursor class drifted: {cursor_class!r}")
    if cursor_class and not any(
        value and value not in ("rgba(0, 0, 0, 0)", "transparent")
        for value in (cursor_background, cursor_border_left, cursor_border_bottom, cursor_outline)
    ):
        raise AssertionError(
            "cursor styling drifted: "
            f"background={cursor_background!r} border_left={cursor_border_left!r} "
            f"border_bottom={cursor_border_bottom!r} outline={cursor_outline!r}"
        )
    if host_width and screen_width:
        if abs(float(host_width) - float(screen_width)) > 18.0:
            raise AssertionError(
                f"xterm screen width drifted: host={host_width!r} screen={screen_width!r}"
            )
    if host.get("xterm_theme_background") != expect_bg:
        raise AssertionError(
            f"xterm background drifted: expected {expect_bg}, got {host.get('xterm_theme_background')}"
        )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--session", required=True)
    parser.add_argument("--settle-seconds", type=float, default=2.0)
    args = parser.parse_args()

    run(
        "server",
        "app",
        "open",
        "--pid",
        str(args.pid),
        args.session,
        "--view",
        "terminal",
        "--timeout-ms",
        "20000",
        check=False,
    )
    wait_for_terminal_settled(args.pid)

    run("server", "app", "theme", "dark", "--pid", str(args.pid), "--timeout-ms", "30000")
    time.sleep(args.settle_seconds)
    dark_state = app_state(args.pid)
    assert_terminal_font_contract(dark_state, "#1e1e1e")

    run("server", "app", "theme", "light", "--pid", str(args.pid), "--timeout-ms", "30000")
    time.sleep(args.settle_seconds)
    light_state = app_state(args.pid)
    assert_terminal_font_contract(light_state, "#fbfbfd")

    print(
        json.dumps(
            {
                "ok": True,
                "pid": args.pid,
                "session": args.session,
                "dark_theme": dark_state["settings"]["effective_terminal_theme_name"],
                "light_theme": light_state["settings"]["effective_terminal_theme_name"],
                "rows_font_family": light_state["viewport"]["terminal_hosts"][0]["rows_font_family"],
                "rows_font_weight": light_state["viewport"]["terminal_hosts"][0]["rows_font_weight"],
            },
            indent=2,
        )
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
