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


def assert_terminal_font_contract(state: dict, expect_bg: str) -> None:
    hosts = state["viewport"]["terminal_hosts"]
    if not hosts:
        raise AssertionError("no terminal host found")
    host = hosts[0]
    xterm_family = host.get("xterm_font_family") or ""
    rows_family = host.get("rows_font_family") or ""
    rows_weight = host.get("rows_font_weight") or ""
    rows_features = host.get("rows_font_feature_settings") or ""
    rows_spacing = host.get("rows_letter_spacing") or ""
    if "JetBrains Mono" not in xterm_family:
        raise AssertionError(f"xterm font family drifted: {xterm_family!r}")
    if "JetBrains Mono" not in rows_family:
        raise AssertionError(f"rows font family drifted: {rows_family!r}")
    if str(host.get("xterm_font_weight")) != "500":
        raise AssertionError(f"xterm font weight drifted: {host.get('xterm_font_weight')!r}")
    if str(host.get("xterm_font_weight_bold")) != "700":
        raise AssertionError(
            f"xterm bold font weight drifted: {host.get('xterm_font_weight_bold')!r}"
        )
    if rows_weight != "500":
        raise AssertionError(f"rows font weight drifted: {rows_weight!r}")
    if '"liga" 0' not in rows_features and '"calt" 0' not in rows_features:
        raise AssertionError(f"rows font features drifted: {rows_features!r}")
    if rows_spacing not in ("0px", "0", "normal"):
        raise AssertionError(f"rows letter spacing drifted: {rows_spacing!r}")
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
