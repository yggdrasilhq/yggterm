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


def host_problem(state: dict) -> str | None:
    viewport = state.get("viewport") or {}
    surface = viewport.get("active_terminal_surface") or {}
    problem = surface.get("problem")
    if isinstance(problem, str) and problem.strip():
        return problem
    host = ((state.get("dom") or {}).get("terminal_hosts") or [{}])[0]
    cursor_line_text = str(host.get("cursor_line_text") or "").lower()
    if "shared connection to " in cursor_line_text and " closed" in cursor_line_text:
        return "active terminal host is showing transport/error output"
    return None


def wait_for_terminal_interactive(pid: int, timeout_seconds: float = 15.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        last_state = app_state(pid)
        viewport = last_state.get("viewport") or {}
        hosts = (last_state.get("dom") or {}).get("terminal_hosts") or []
        if (
            viewport.get("ready") is True
            and viewport.get("interactive") is True
            and hosts
            and hosts[0].get("input_enabled") is True
            and not host_problem(last_state)
        ):
            return last_state
        time.sleep(0.2)
    raise AssertionError(f"terminal did not settle in time: {last_state!r}")


def run_status_probe(pid: int, session: str) -> dict:
    return run(
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
        "/status",
        "--enter",
        "--timeout-ms",
        "15000",
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--session", required=True)
    parser.add_argument("--out", default="/tmp/terminal-status-smoke")
    args = parser.parse_args()

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    before = wait_for_terminal_interactive(args.pid)
    with (out_dir / "before.json").open("w") as fh:
        json.dump(before, fh, indent=2)
    run(
        "server",
        "app",
        "screenshot",
        "--pid",
        str(args.pid),
        str(out_dir / "before.png"),
        "--timeout-ms",
        "8000",
    )

    probe = run_status_probe(args.pid, args.session)
    with (out_dir / "probe.json").open("w") as fh:
        json.dump(probe, fh, indent=2)

    time.sleep(1.0)
    after = app_state(args.pid)
    after_host = ((after.get("dom") or {}).get("terminal_hosts") or [{}])[0]
    after_text_sample = str(after_host.get("text_sample") or "")
    after_cursor_line_text = str(after_host.get("cursor_line_text") or "")
    if (
        "/status" not in after_text_sample
        and "OpenAI Codex" not in after_text_sample
        and "Session:" not in after_text_sample
        and "/status" not in after_cursor_line_text
        and "OpenAI Codex" not in after_cursor_line_text
        and "Session:" not in after_cursor_line_text
    ):
        after = wait_for_terminal_interactive(args.pid, timeout_seconds=20.0)
        retry_probe = run_status_probe(args.pid, args.session)
        with (out_dir / "retry-probe.json").open("w") as fh:
            json.dump(retry_probe, fh, indent=2)
        time.sleep(1.0)
        after = app_state(args.pid)
    with (out_dir / "after.json").open("w") as fh:
        json.dump(after, fh, indent=2)
    run(
        "server",
        "app",
        "screenshot",
        "--pid",
        str(args.pid),
        str(out_dir / "after.png"),
        "--timeout-ms",
        "8000",
    )

    select = run(
        "server",
        "app",
        "terminal",
        "probe-select",
        "--pid",
        str(args.pid),
        args.session,
        "--timeout-ms",
        "8000",
    )
    with (out_dir / "select.json").open("w") as fh:
        json.dump(select, fh, indent=2)

    viewport = after.get("viewport") or {}
    host = ((after.get("dom") or {}).get("terminal_hosts") or [{}])[0]
    notifications = (after.get("shell") or {}).get("notifications") or []
    text_sample = host.get("text_sample") or ""
    cursor_line_text = host.get("cursor_line_text") or ""
    selected_text_length = (((select.get("data") or {}).get("selected_text_length")) or 0)

    if viewport.get("ready") is not True or viewport.get("interactive") is not True:
        raise AssertionError(f"terminal not interactive after /status: {viewport!r}")
    if notifications:
        raise AssertionError(f"notifications still visible after /status: {notifications!r}")
    if host.get("low_contrast_span_count") not in (0, None):
        raise AssertionError(
            f"low contrast spans remain visible: {host.get('low_contrast_span_count')!r}"
        )
    if host.get("cursor_overlay_present") is not True:
        raise AssertionError("cursor overlay missing after /status")
    if host.get("cursor_overlay_display") in ("", "none"):
        raise AssertionError(f"cursor overlay hidden after /status: {host.get('cursor_overlay_display')!r}")
    if "/status" not in text_sample and "/status" not in cursor_line_text:
        raise AssertionError("typed /status did not appear in terminal text sample")
    if (
        "OpenAI Codex" not in text_sample
        and "Session:" not in text_sample
        and "OpenAI Codex" not in cursor_line_text
        and "Session:" not in cursor_line_text
    ):
        raise AssertionError("Codex status panel did not appear after /status")
    if selected_text_length <= 0:
        raise AssertionError(f"selection probe did not capture visible text: {select!r}")

    summary = {
        "pid": args.pid,
        "session": args.session,
        "ready": viewport.get("ready"),
        "interactive": viewport.get("interactive"),
        "cursor_overlay_present": host.get("cursor_overlay_present"),
        "cursor_overlay_display": host.get("cursor_overlay_display"),
        "selected_text_length": selected_text_length,
        "selected_contrast": (select.get("data") or {}).get("selected_contrast"),
        "rows_sample_color": host.get("rows_sample_color"),
        "dim_sample_color": host.get("dim_sample_color"),
        "low_contrast_span_count": host.get("low_contrast_span_count"),
    }
    with (out_dir / "summary.json").open("w") as fh:
        json.dump(summary, fh, indent=2)
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
