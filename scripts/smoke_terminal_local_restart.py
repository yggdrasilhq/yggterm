#!/usr/bin/env python3
import argparse
import json
import os
import signal
import shutil
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(Path(__file__).resolve().parent))

import smoke_terminal_new_local as new_local
import smoke_xterm_embed_faults as faults


def daemon_pids_for_home(home: Path) -> list[int]:
    normalized_home = str(home.expanduser().resolve())
    pids: list[int] = []
    for proc_dir in Path("/proc").iterdir():
        if not proc_dir.name.isdigit():
            continue
        try:
            cmdline = (proc_dir / "cmdline").read_text(errors="ignore").replace("\x00", " ")
            if "yggterm" not in cmdline or "server daemon" not in cmdline:
                continue
            environ = (proc_dir / "environ").read_bytes().decode("utf-8", errors="ignore")
            if f"YGGTERM_HOME={normalized_home}" not in environ:
                continue
            pids.append(int(proc_dir.name))
        except OSError:
            continue
    return sorted(pids)


def kill_daemons_for_home(home: Path) -> list[int]:
    pids = daemon_pids_for_home(home)
    for pid in pids:
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            continue
    deadline = time.time() + 10.0
    while time.time() < deadline:
        remaining = daemon_pids_for_home(home)
        if not remaining:
            return pids
        time.sleep(0.1)
    raise AssertionError(f"home-scoped daemons did not exit for {home}: {daemon_pids_for_home(home)!r}")


def wait_app_state(pid: int, timeout_seconds: float = 20.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_error = None
    while time.time() < deadline:
        try:
            return faults.app_state(pid)
        except Exception as error:  # noqa: BLE001
            last_error = str(error)
            time.sleep(0.25)
    raise AssertionError(f"app state never became ready for pid {pid}: {last_error}")


def wait_for_terminal_surface(pid: int, session: str, timeout_seconds: float = 20.0) -> dict:
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
    raise AssertionError(
        f"local terminal surface did not become visible after restart: {last_state!r}"
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--display", default=":235")
    parser.add_argument("--cwd", default=str(ROOT))
    parser.add_argument("--title", default="Smoke Local Restart")
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--home", type=Path, required=True)
    parser.add_argument("--fresh-home", action="store_true")
    args = parser.parse_args()

    if args.fresh_home and args.home.exists():
        shutil.rmtree(args.home)
    args.home.mkdir(parents=True, exist_ok=True)
    args.out_dir.mkdir(parents=True, exist_ok=True)

    faults.ENV["YGGTERM_HOME"] = str(args.home)
    env = os.environ.copy()
    env["DISPLAY"] = args.display
    env["YGGTERM_HOME"] = str(args.home)

    xvfb_log = (args.out_dir / "xvfb.log").open("w")
    xvfb = subprocess.Popen(
        ["Xvfb", args.display, "-screen", "0", "1460x920x24", "-ac"],
        stdout=xvfb_log,
        stderr=subprocess.STDOUT,
    )
    app = None
    app2 = None
    try:
        time.sleep(1.0)
        app_log = (args.out_dir / "app.log").open("w")
        app = subprocess.Popen(
            [str(ROOT / "target" / "debug" / "yggterm")],
            cwd=ROOT,
            env=env,
            stdout=app_log,
            stderr=subprocess.STDOUT,
        )
        wait_app_state(app.pid)
        created = new_local.run_terminal_new(app.pid, args.cwd, args.title)
        session = str(created.get("active_session_path") or "")
        if not session:
            raise AssertionError(f"terminal new did not return a session path: {created!r}")
        before_state = new_local.wait_for_local_terminal(app.pid, session, timeout_seconds=15.0)
        faults.app_screenshot(app.pid, args.out_dir / "before-restart.png")
        with (args.out_dir / "before-restart-state.json").open("w") as fh:
            json.dump(before_state, fh, indent=2)
        daemon_pids_before_restart = daemon_pids_for_home(args.home)
        if not daemon_pids_before_restart:
            raise AssertionError(f"expected a daemon for {args.home}, found none")

        app.terminate()
        app.wait(timeout=20)
        app = None
        killed_daemons = kill_daemons_for_home(args.home)

        app_restart_log = (args.out_dir / "app-restart.log").open("w")
        app2 = subprocess.Popen(
            [str(ROOT / "target" / "debug" / "yggterm")],
            cwd=ROOT,
            env=env,
            stdout=app_restart_log,
            stderr=subprocess.STDOUT,
        )
        wait_app_state(app2.pid)
        open_result = faults.app_open(app2.pid, session, view="terminal")
        after_state = wait_for_terminal_surface(app2.pid, session)
        daemon_pids_after_restart = daemon_pids_for_home(args.home)
        if not daemon_pids_after_restart:
            raise AssertionError(f"daemon did not respawn for {args.home}")
        after_screenshot = faults.app_screenshot(app2.pid, args.out_dir / "after-restart.png")
        with (args.out_dir / "after-restart-state.json").open("w") as fh:
            json.dump(after_state, fh, indent=2)
        with (args.out_dir / "after-restart-screenshot-state.json").open("w") as fh:
            json.dump(after_screenshot, fh, indent=2)
        screenshot_failed_notifications = [
            notification
            for notification in ((after_screenshot.get("shell") or {}).get("notifications") or [])
            if str(notification.get("title") or "") == "Remote Terminal Failed"
        ]
        if screenshot_failed_notifications:
            raise AssertionError(
                "stale remote terminal failure notification is still visible after restart: "
                f"{screenshot_failed_notifications!r}"
            )

        summary = {
            "display": args.display,
            "home": str(args.home),
            "pid_before": created.get("handled_by_pid"),
            "pid_after": app2.pid,
            "session": session,
            "daemon_pids_before_restart": daemon_pids_before_restart,
            "killed_daemons": killed_daemons,
            "daemon_pids_after_restart": daemon_pids_after_restart,
            "created": created,
            "open_result": open_result,
            "checks": {
                "renderer": faults.assert_renderer_contract(after_state),
                "geometry": faults.assert_geometry(after_state),
                "focus_visibility": faults.assert_focus_and_visibility(after_state),
                "local_tree": faults.assert_local_tree_placement(app2.pid, session),
                "sidebar_contract": faults.assert_sidebar_contract(app2.pid, session),
                "runtime_ready": faults.assert_local_session_runtime_ready(session),
                "observability_budget": faults.assert_observability_budget(),
                "screenshot_failed_notification_count": len(screenshot_failed_notifications),
            },
        }
        with (args.out_dir / "summary.json").open("w") as fh:
            json.dump(summary, fh, indent=2)
        print(json.dumps(summary, indent=2))
        return 0
    finally:
        if app is not None and app.poll() is None:
            app.terminate()
            app.wait(timeout=20)
        if app2 is not None and app2.poll() is None:
            app2.terminate()
            app2.wait(timeout=20)
        kill_daemons_for_home(args.home)
        xvfb.terminate()
        xvfb.wait(timeout=20)
        xvfb_log.close()


if __name__ == "__main__":
    raise SystemExit(main())
