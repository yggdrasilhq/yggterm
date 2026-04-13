#!/usr/bin/env python3
import argparse
import json
import os
import time
from pathlib import Path

import smoke_xterm_embed_faults as faults


def run_terminal_new(pid: int, cwd: str, title: str) -> dict:
    payload = faults.run(
        "server",
        "app",
        "terminal",
        "new",
        "--pid",
        str(pid),
        "--cwd",
        cwd,
        "--title",
        title,
    )
    return payload.get("data") or payload


def wait_for_local_terminal(pid: int, session: str, timeout_seconds: float = 12.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        last_state = faults.app_state(pid)
        host = faults.active_host_or_none(last_state)
        if (
            last_state.get("active_session_path") == session
            and host is not None
            and str(host.get("session_path") or "") == session
            and str(host.get("text_sample") or "").strip()
        ):
            return last_state
        time.sleep(0.25)
    raise AssertionError(f"fresh local terminal did not become visible in time: {last_state!r}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--cwd", default=str(Path.cwd()))
    parser.add_argument("--title", default="Smoke Local Shell")
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--home", type=Path)
    args = parser.parse_args()

    if args.home is not None:
        faults.ENV["YGGTERM_HOME"] = str(args.home)

    args.out_dir.mkdir(parents=True, exist_ok=True)
    created = run_terminal_new(args.pid, args.cwd, args.title)
    session = str(created.get("active_session_path") or "")
    if not session:
        raise AssertionError(f"terminal new did not return an active session path: {created!r}")

    state = wait_for_local_terminal(args.pid, session)
    screenshot = faults.app_screenshot(args.pid, args.out_dir / "fresh-local-terminal.png")
    summary = {
        "pid": args.pid,
        "session": session,
        "created": created,
        "checks": {
            "renderer": faults.assert_renderer_contract(state),
            "local_tree": faults.assert_local_tree_placement(args.pid, session),
            "sidebar_contract": faults.assert_sidebar_contract(args.pid, session),
            "busy_lifecycle": faults.assert_busy_icon_lifecycle(args.pid, session),
        },
        "host": faults.active_host(state),
        "screenshot": screenshot.get("output_path"),
    }
    with (args.out_dir / "summary.json").open("w") as fh:
        json.dump(summary, fh, indent=2)
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
