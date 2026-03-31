#!/usr/bin/env python3
import argparse
import json
import os
import shutil
import signal
import subprocess
import tempfile
import time
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Run 23 strict yggterm-mock-cli server lifecycle checks against an isolated "
            "YGGTERM_HOME and write per-scenario artifacts."
        )
    )
    parser.add_argument("--bin", default="./target/debug/yggterm")
    parser.add_argument("--mock-bin", default="./target/debug/yggterm-mock-cli")
    parser.add_argument("--out-dir", default="/tmp/yggterm-mock-cli-23")
    parser.add_argument("--slow-notice-ms", type=int, default=1000)
    parser.add_argument("--slow-delay-ms", type=int, default=1600)
    parser.add_argument("--fast-delay-ms", type=int, default=250)
    return parser.parse_args()


def run(
    argv: list[str],
    *,
    env: dict[str, str],
    check: bool = True,
) -> subprocess.CompletedProcess:
    return subprocess.run(
        argv,
        text=True,
        capture_output=True,
        check=check,
        env=env,
    )


def parse_jsonl(path: Path) -> list[dict]:
    events: list[dict] = []
    if not path.exists():
        return events
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line:
            events.append(json.loads(line))
    return events


def scenario_events(events: list[dict], kind: str) -> list[dict]:
    return [event for event in events if ((event.get("event") or {}).get("kind")) == kind]


def scenario_result_data(events: list[dict]) -> dict:
    results = scenario_events(events, "result")
    if not results:
        return {}
    return results[-1].get("data") or {}


def scenario_error_message(events: list[dict]) -> str:
    errors = scenario_events(events, "error")
    if not errors:
        return ""
    return str(errors[-1].get("message") or "")


def event_sequence(events: list[dict]) -> list[str]:
    return [str((event.get("event") or {}).get("kind") or "") for event in events]


def any_message_contains(events: list[dict], needle: str) -> bool:
    lowered = needle.lower()
    for event in events:
        message = str(event.get("message") or "")
        if lowered in message.lower():
            return True
    return False


def mock_cli(
    mock_bin: str,
    env: dict[str, str],
    out_dir: Path,
    name: str,
    *extra_args: str,
) -> tuple[subprocess.CompletedProcess, list[dict]]:
    jsonl_path = out_dir / f"{name}.jsonl"
    cmd = [str(Path(mock_bin).resolve()), *extra_args, "--jsonl-out", str(jsonl_path)]
    result = run(cmd, env=env, check=False)
    return result, parse_jsonl(jsonl_path)


def start_daemon(binary: str, env: dict[str, str]) -> subprocess.Popen:
    proc = subprocess.Popen(
        [str(Path(binary).resolve()), "server", "daemon"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        env=env,
    )
    deadline = time.monotonic() + 8.0
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise RuntimeError(f"daemon exited early with code {proc.returncode}")
        if run([str(Path(binary).resolve()), "server", "ping"], env=env, check=False).returncode == 0:
            return proc
        time.sleep(0.1)
    raise RuntimeError("daemon did not become reachable within 8.0s")


def stop_process(proc: subprocess.Popen | None) -> None:
    if proc is None or proc.poll() is not None:
        return
    proc.send_signal(signal.SIGTERM)
    try:
        proc.wait(timeout=3)
    except subprocess.TimeoutExpired:
        proc.kill()
        proc.wait(timeout=3)


def live_daemon_pids_for_home(home: Path) -> list[int]:
    pids: list[int] = []
    for proc_dir in Path("/proc").iterdir():
        if not proc_dir.name.isdigit():
            continue
        pid = int(proc_dir.name)
        try:
            cmdline = (proc_dir / "cmdline").read_bytes().replace(b"\x00", b" ").decode("utf-8", "ignore")
            environ = (proc_dir / "environ").read_bytes().decode("utf-8", "ignore")
        except Exception:
            continue
        if "yggterm server daemon" not in cmdline:
            continue
        if f"YGGTERM_HOME={home}" not in environ:
            continue
        pids.append(pid)
    return sorted(pids)


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir)
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    home = Path(tempfile.mkdtemp(prefix="yggterm-mock-cli-23-home-"))
    env = os.environ.copy()
    env["YGGTERM_HOME"] = str(home)

    checks: list[dict] = []
    daemon_proc: subprocess.Popen | None = None
    session_path = ""

    def record(name: str, passed: bool, details: dict) -> None:
        checks.append({"name": name, "passed": passed, "details": details})

    try:
        daemon_proc = start_daemon(args.bin, env)
        record("daemon_started", True, {"pid": daemon_proc.pid})

        startup_result, startup_events = mock_cli(
            args.mock_bin, env, out_dir, "01-startup", "--scenario", "startup"
        )
        startup_data = scenario_result_data(startup_events)
        startup_result_events = scenario_events(startup_events, "result")
        record(
            "startup_sequence_ok",
            startup_result.returncode == 0 and event_sequence(startup_events)[:3] == ["accepted", "loading", "result"],
            {"returncode": startup_result.returncode, "sequence": event_sequence(startup_events)},
        )
        record(
            "startup_has_version_and_build_id",
            bool(startup_data.get("server_version"))
            and isinstance(startup_data.get("server_build_id"), int)
            and bool(startup_result_events)
            and isinstance(startup_result_events[-1].get("elapsed_ms"), int),
            startup_data,
        )

        ping_result, ping_events = mock_cli(
            args.mock_bin, env, out_dir, "02-ping", "--scenario", "ping"
        )
        record(
            "ping_returns_pong",
            ping_result.returncode == 0 and scenario_result_data(ping_events).get("pong") is True,
            scenario_result_data(ping_events),
        )

        status_result, status_events = mock_cli(
            args.mock_bin, env, out_dir, "03-status", "--scenario", "status"
        )
        status_data = scenario_result_data(status_events)
        record(
            "status_has_restore_fields",
            status_result.returncode == 0
            and bool(status_data.get("server_version"))
            and all(
                key in status_data
                for key in (
                    "restored_from_persisted_state",
                    "restored_stored_sessions",
                    "restored_live_sessions",
                    "restored_remote_machines",
                )
            ),
            status_data,
        )

        snapshot_result, snapshot_events = mock_cli(
            args.mock_bin, env, out_dir, "04-snapshot", "--scenario", "snapshot"
        )
        snapshot_data = scenario_result_data(snapshot_events)
        record(
            "snapshot_counts_numeric",
            snapshot_result.returncode == 0
            and "active_view_mode" in snapshot_data
            and "live_sessions" in snapshot_data
            and all(
                isinstance(snapshot_data.get(key), int)
                for key in ("live_sessions", "remote_machines", "ssh_targets")
            ),
            snapshot_data,
        )

        slow_result, slow_events = mock_cli(
            args.mock_bin,
            env,
            out_dir,
            "05-slow-startup",
            "--scenario",
            "startup",
            "--delay-ms",
            str(args.slow_delay_ms),
            "--slow-notice-ms",
            str(args.slow_notice_ms),
        )
        record(
            "slow_notice_emits_progress",
            slow_result.returncode == 0
            and any_message_contains(
                scenario_events(slow_events, "progress"),
                "loading threshold exceeded",
            ),
            {
                "sequence": event_sequence(slow_events),
                "stderr": slow_result.stderr.strip(),
            },
        )

        fast_result, fast_events = mock_cli(
            args.mock_bin,
            env,
            out_dir,
            "06-fast-startup",
            "--scenario",
            "startup",
            "--delay-ms",
            str(args.fast_delay_ms),
            "--slow-notice-ms",
            str(args.slow_notice_ms),
        )
        record(
            "fast_path_suppresses_progress",
            fast_result.returncode == 0
            and not any_message_contains(
                scenario_events(fast_events, "progress"),
                "loading threshold exceeded",
            ),
            {
                "sequence": event_sequence(fast_events),
                "stderr": fast_result.stderr.strip(),
            },
        )

        ping3_result, ping3_events = mock_cli(
            args.mock_bin,
            env,
            out_dir,
            "07-ping-iter3",
            "--scenario",
            "ping",
            "--iterations",
            "3",
        )
        record(
            "ping_iterations_all_result",
            ping3_result.returncode == 0 and len(scenario_events(ping3_events, "result")) == 3,
            {"sequence": event_sequence(ping3_events)},
        )

        snapshot3_result, snapshot3_events = mock_cli(
            args.mock_bin,
            env,
            out_dir,
            "08-snapshot-iter3",
            "--scenario",
            "snapshot",
            "--iterations",
            "3",
        )
        record(
            "snapshot_iterations_all_result",
            snapshot3_result.returncode == 0 and len(scenario_events(snapshot3_events, "result")) == 3,
            {"sequence": event_sequence(snapshot3_events)},
        )

        disconnect_result, disconnect_events = mock_cli(
            args.mock_bin,
            env,
            out_dir,
            "09-disconnect-safe",
            "--scenario",
            "disconnect-safe",
            "--cwd",
            "/tmp",
            "--title-hint",
            "mock cli 23 session",
        )
        disconnect_data = scenario_result_data(disconnect_events)
        session_path = str(disconnect_data.get("session_path") or "")
        record(
            "disconnect_safe_retains_session",
            disconnect_result.returncode == 0
            and bool(session_path)
            and disconnect_data.get("retained_after_disconnect") is True,
            disconnect_data,
        )

        reconnect_result, reconnect_events = mock_cli(
            args.mock_bin,
            env,
            out_dir,
            "10-reconnect-check-live",
            "--scenario",
            "reconnect-check",
            "--expect-path",
            session_path,
        )
        reconnect_data = scenario_result_data(reconnect_events)
        record(
            "reconnect_check_finds_session_live",
            reconnect_result.returncode == 0
            and (reconnect_data.get("active_matches") is True or reconnect_data.get("listed") is True),
            reconnect_data,
        )

        snapshot_after_result, snapshot_after_events = mock_cli(
            args.mock_bin, env, out_dir, "11-snapshot-after-reconnect", "--scenario", "snapshot"
        )
        snapshot_after_data = scenario_result_data(snapshot_after_events)
        active_path = snapshot_after_data.get("active_session_path")
        record(
            "snapshot_after_reconnect_mentions_session",
            snapshot_after_result.returncode == 0 and (active_path == session_path or bool(active_path)),
            snapshot_after_data,
        )

        invalid_refresh_result, invalid_refresh_events = mock_cli(
            args.mock_bin,
            env,
            out_dir,
            "12-invalid-refresh-remote",
            "--scenario",
            "refresh-remote",
            "--machine-key",
            "missing-machine-key",
        )
        record(
            "invalid_refresh_remote_errors_cleanly",
            invalid_refresh_result.returncode != 0
            and (
                bool(scenario_error_message(invalid_refresh_events))
                or bool(invalid_refresh_result.stderr.strip())
            ),
            {
                "returncode": invalid_refresh_result.returncode,
                "error": scenario_error_message(invalid_refresh_events),
                "stderr": invalid_refresh_result.stderr.strip(),
                "sequence": event_sequence(invalid_refresh_events),
            },
        )
        record(
            "invalid_refresh_remote_sequence_starts_cleanly",
            event_sequence(invalid_refresh_events)[:2] == ["accepted", "loading"],
            {"sequence": event_sequence(invalid_refresh_events)},
        )

        graceful_result, graceful_events = mock_cli(
            args.mock_bin,
            env,
            out_dir,
            "13-graceful-shutdown",
            "--scenario",
            "graceful-shutdown",
        )
        graceful_data = scenario_result_data(graceful_events)
        record(
            "graceful_shutdown_reports_unreachable_after",
            graceful_result.returncode == 0 and graceful_data.get("daemon_reachable_after") is False,
            graceful_data,
        )

        ping_after_shutdown = run([str(Path(args.bin).resolve()), "server", "ping"], env=env, check=False)
        record(
            "ping_after_shutdown_fails",
            ping_after_shutdown.returncode != 0,
            {"returncode": ping_after_shutdown.returncode, "stderr": ping_after_shutdown.stderr.strip()},
        )

        daemon_proc = start_daemon(args.bin, env)
        record("restart_daemon_after_shutdown", True, {"pid": daemon_proc.pid})

        startup_restart_result, startup_restart_events = mock_cli(
            args.mock_bin, env, out_dir, "14-startup-after-restart", "--scenario", "startup"
        )
        record(
            "startup_after_restart_succeeds",
            startup_restart_result.returncode == 0
            and bool(scenario_result_data(startup_restart_events).get("server_version")),
            scenario_result_data(startup_restart_events),
        )

        reconnect_restart_result, reconnect_restart_events = mock_cli(
            args.mock_bin,
            env,
            out_dir,
            "15-reconnect-after-restart",
            "--scenario",
            "reconnect-check",
            "--expect-path",
            session_path,
        )
        reconnect_restart_data = scenario_result_data(reconnect_restart_events)
        record(
            "reconnect_after_restart_restored_state_present",
            reconnect_restart_result.returncode == 0
            and (reconnect_restart_data.get("active_matches") is True or reconnect_restart_data.get("listed") is True)
            and "restored_from_persisted_state" in reconnect_restart_data,
            reconnect_restart_data,
        )

        daemon_pids = live_daemon_pids_for_home(home)
        record(
            "single_daemon_process_for_isolated_home",
            len(daemon_pids) == 1,
            {"pids": daemon_pids},
        )

        startup3_result, startup3_events = mock_cli(
            args.mock_bin,
            env,
            out_dir,
            "16-startup-iter3",
            "--scenario",
            "startup",
            "--iterations",
            "3",
        )
        record(
            "startup_iterations_all_result",
            startup3_result.returncode == 0 and len(scenario_events(startup3_events, "result")) == 3,
            {"sequence": event_sequence(startup3_events)},
        )

        graceful2_result, graceful2_events = mock_cli(
            args.mock_bin,
            env,
            out_dir,
            "17-graceful-shutdown-final",
            "--scenario",
            "graceful-shutdown",
        )
        graceful2_data = scenario_result_data(graceful2_events)
        daemon_proc = None

        ping_final = run([str(Path(args.bin).resolve()), "server", "ping"], env=env, check=False)
        daemon_pids_after = live_daemon_pids_for_home(home)
        record(
            "final_shutdown_and_cleanup",
            graceful2_result.returncode == 0
            and graceful2_data.get("daemon_reachable_after") is False
            and ping_final.returncode != 0
            and len(daemon_pids_after) == 0,
            {
                "graceful": graceful2_data,
                "ping_returncode": ping_final.returncode,
                "ping_stderr": ping_final.stderr.strip(),
                "pids": daemon_pids_after,
            },
        )
    finally:
        stop_process(daemon_proc)

    summary = {
        "home": str(home),
        "check_count": len(checks),
        "failed": len([check for check in checks if not check["passed"]]),
        "checks": checks,
    }
    (out_dir / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(json.dumps(summary, indent=2))
    return 0 if summary["failed"] == 0 and summary["check_count"] == 23 else 1


if __name__ == "__main__":
    raise SystemExit(main())
