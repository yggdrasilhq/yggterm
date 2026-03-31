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
            "Run 23 strict mock-cli checks for restored remote-machine health state "
            "against an isolated YGGTERM_HOME."
        )
    )
    parser.add_argument("--bin", default="./target/debug/yggterm")
    parser.add_argument("--mock-bin", default="./target/debug/yggterm-mock-cli")
    parser.add_argument("--out-dir", default="/tmp/yggterm-mock-cli-remote-machine-23")
    return parser.parse_args()


def run(argv: list[str], *, env: dict[str, str], check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(argv, text=True, capture_output=True, env=env, check=check)


def mock_cli(
    mock_bin: str,
    env: dict[str, str],
    out_dir: Path,
    name: str,
    *extra_args: str,
) -> tuple[subprocess.CompletedProcess, dict]:
    jsonl_path = out_dir / f"{name}.jsonl"
    result = run(
        [str(Path(mock_bin).resolve()), *extra_args, "--jsonl-out", str(jsonl_path)],
        env=env,
        check=False,
    )
    events = []
    if jsonl_path.exists():
        for line in jsonl_path.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line:
                events.append(json.loads(line))
    return result, {"events": events}


def scenario_events(payload: dict, kind: str) -> list[dict]:
    return [event for event in payload.get("events", []) if ((event.get("event") or {}).get("kind")) == kind]


def scenario_result_data(payload: dict) -> dict:
    events = scenario_events(payload, "result")
    if not events:
        return {}
    return events[-1].get("data") or {}


def scenario_error_message(payload: dict) -> str:
    events = scenario_events(payload, "error")
    if not events:
        return ""
    return str(events[-1].get("message") or "")


def event_sequence(payload: dict) -> list[str]:
    return [str((event.get("event") or {}).get("kind") or "") for event in payload.get("events", [])]


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


def write_seed_state(home: Path) -> None:
    payload = {
        "active_session_path": None,
        "active_view_mode": "Rendered",
        "ssh_targets": [
            {"label": "healthy-dev", "kind": "ssh_shell", "ssh_target": "healthy-dev", "prefix": None, "cwd": "/srv/app"},
            {"label": "cached-dev", "kind": "ssh_shell", "ssh_target": "cached-dev", "prefix": None, "cwd": "/srv/cache"},
            {"label": "offline-dev", "kind": "ssh_shell", "ssh_target": "offline-dev", "prefix": None, "cwd": "/srv/offline"},
        ],
        "remote_machines": [
            {
                "machine_key": "healthy-dev",
                "label": "healthy-dev",
                "ssh_target": "healthy-dev",
                "prefix": None,
                "remote_binary_expr": "~/.yggterm/bin/yggterm",
                "remote_deploy_state": "Ready",
                "health": "healthy",
                "sessions": [
                    {
                        "session_path": "remote-session://healthy-dev/001",
                        "session_id": "001",
                        "cwd": "/srv/app",
                        "started_at": "2026-03-31T00:00:00Z",
                        "modified_epoch": 10,
                        "event_count": 4,
                        "user_message_count": 1,
                        "assistant_message_count": 1,
                        "title_hint": "healthy session",
                        "recent_context": "USER: hi\nASSISTANT: ok",
                        "cached_precis": "healthy precis",
                        "cached_summary": "healthy summary",
                        "storage_path": "/tmp/healthy-session.jsonl",
                    }
                ],
            },
            {
                "machine_key": "cached-dev",
                "label": "cached-dev",
                "ssh_target": "cached-dev",
                "prefix": None,
                "remote_binary_expr": None,
                "remote_deploy_state": "Planned",
                "health": "cached",
                "sessions": [
                    {
                        "session_path": "remote-session://cached-dev/002",
                        "session_id": "002",
                        "cwd": "/srv/cache",
                        "started_at": "2026-03-31T00:10:00Z",
                        "modified_epoch": 9,
                        "event_count": 3,
                        "user_message_count": 1,
                        "assistant_message_count": 1,
                        "title_hint": "cached session",
                        "recent_context": "USER: cache\nASSISTANT: snapshot",
                        "cached_precis": "cached precis",
                        "cached_summary": "cached summary",
                        "storage_path": "/tmp/cached-session.jsonl",
                    }
                ],
            },
            {
                "machine_key": "offline-dev",
                "label": "offline-dev",
                "ssh_target": "offline-dev",
                "prefix": None,
                "remote_binary_expr": None,
                "remote_deploy_state": "Planned",
                "health": "offline",
                "sessions": [],
            },
        ],
        "stored_sessions": [],
        "live_sessions": [],
    }
    (home / "server-state.json").write_text(json.dumps(payload, indent=2), encoding="utf-8")


def machine_by_key(details: list[dict], machine_key: str) -> dict:
    for item in details:
        if item.get("machine_key") == machine_key:
            return item
    return {}


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir)
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    home = Path(tempfile.mkdtemp(prefix="yggterm-mock-cli-remote-machine-home-"))
    env = os.environ.copy()
    env["YGGTERM_HOME"] = str(home)
    write_seed_state(home)

    checks: list[dict] = []
    daemon_proc: subprocess.Popen | None = None

    def record(name: str, passed: bool, details: dict) -> None:
        checks.append({"name": name, "passed": passed, "details": details})

    try:
        daemon_proc = start_daemon(args.bin, env)
        record("daemon_started", True, {"pid": daemon_proc.pid})

        startup_result, startup_payload = mock_cli(args.mock_bin, env, out_dir, "01-startup", "--scenario", "startup")
        startup = scenario_result_data(startup_payload)
        startup_details = startup.get("remote_machine_details") or []
        startup_counts = startup.get("remote_machine_health_counts") or {}
        record("startup_restores_three_remote_machines", startup_result.returncode == 0 and startup.get("remote_machines") == 3, startup)
        record("startup_restored_remote_machine_count", startup.get("restored_remote_machines") == 3, startup)
        record("startup_health_counts_total", startup_counts == {"healthy": 0, "cached": 2, "offline": 1}, startup_counts)
        record(
            "startup_demotes_restored_ready_machine_to_cached",
            machine_by_key(startup_details, "healthy-dev").get("health") == "cached"
            and machine_by_key(startup_details, "healthy-dev").get("remote_deploy_state") == "Ready",
            {"details": startup_details},
        )
        record("startup_has_cached_machine_detail", machine_by_key(startup_details, "cached-dev").get("health") == "cached", {"details": startup_details})
        record("startup_has_offline_machine_detail", machine_by_key(startup_details, "offline-dev").get("health") == "offline", {"details": startup_details})

        ping_result, ping_payload = mock_cli(args.mock_bin, env, out_dir, "02-ping", "--scenario", "ping")
        record("ping_returns_pong", ping_result.returncode == 0 and scenario_result_data(ping_payload).get("pong") is True, scenario_result_data(ping_payload))

        status_result, status_payload = mock_cli(args.mock_bin, env, out_dir, "03-status", "--scenario", "status")
        status = scenario_result_data(status_payload)
        record("status_restored_from_persisted_state", status_result.returncode == 0 and status.get("restored_from_persisted_state") is True, status)
        record("status_restored_remote_machines", status.get("restored_remote_machines") == 3, status)

        snapshot_result, snapshot_payload = mock_cli(args.mock_bin, env, out_dir, "04-snapshot", "--scenario", "snapshot")
        snapshot_data = scenario_result_data(snapshot_payload)
        snapshot_details = snapshot_data.get("remote_machine_details") or []
        snapshot_counts = snapshot_data.get("remote_machine_health_counts") or {}
        record("snapshot_reports_three_remote_machines", snapshot_result.returncode == 0 and snapshot_data.get("remote_machines") == 3, snapshot_data)
        record("snapshot_health_counts_match", snapshot_counts == {"healthy": 0, "cached": 2, "offline": 1}, snapshot_counts)
        record(
            "snapshot_restored_ready_machine_is_cached_until_refresh",
            machine_by_key(snapshot_details, "healthy-dev").get("health") == "cached"
            and machine_by_key(snapshot_details, "healthy-dev").get("remote_deploy_state") == "Ready"
            and machine_by_key(snapshot_details, "healthy-dev").get("session_count") == 1,
            {"details": snapshot_details},
        )
        record(
            "snapshot_cached_machine_planned",
            machine_by_key(snapshot_details, "cached-dev").get("remote_deploy_state") == "Planned"
            and machine_by_key(snapshot_details, "cached-dev").get("session_count") == 1,
            {"details": snapshot_details},
        )
        record(
            "snapshot_offline_machine_planned",
            machine_by_key(snapshot_details, "offline-dev").get("remote_deploy_state") == "Planned"
            and machine_by_key(snapshot_details, "offline-dev").get("session_count") == 0,
            {"details": snapshot_details},
        )

        snapshot3_result, snapshot3_payload = mock_cli(
            args.mock_bin, env, out_dir, "05-snapshot-iter3", "--scenario", "snapshot", "--iterations", "3"
        )
        record(
            "snapshot_iterations_all_result",
            snapshot3_result.returncode == 0 and len(scenario_events(snapshot3_payload, "result")) == 3,
            {"sequence": event_sequence(snapshot3_payload)},
        )

        invalid_refresh_result, invalid_refresh_payload = mock_cli(
            args.mock_bin,
            env,
            out_dir,
            "06-invalid-refresh",
            "--scenario",
            "refresh-remote",
            "--machine-key",
            "missing-machine-key",
        )
        record(
            "invalid_refresh_remote_errors_cleanly",
            invalid_refresh_result.returncode != 0
            and (bool(scenario_error_message(invalid_refresh_payload)) or bool(invalid_refresh_result.stderr.strip())),
            {
                "returncode": invalid_refresh_result.returncode,
                "error": scenario_error_message(invalid_refresh_payload),
                "stderr": invalid_refresh_result.stderr.strip(),
            },
        )
        record(
            "invalid_refresh_remote_sequence_starts_cleanly",
            event_sequence(invalid_refresh_payload)[:2] == ["accepted", "loading"],
            {"sequence": event_sequence(invalid_refresh_payload)},
        )

        shutdown_result, shutdown_payload = mock_cli(
            args.mock_bin, env, out_dir, "07-graceful-shutdown", "--scenario", "graceful-shutdown"
        )
        shutdown_data = scenario_result_data(shutdown_payload)
        daemon_proc = None
        record(
            "graceful_shutdown_reports_unreachable_after",
            shutdown_result.returncode == 0 and shutdown_data.get("daemon_reachable_after") is False,
            shutdown_data,
        )

        ping_after = run([str(Path(args.bin).resolve()), "server", "ping"], env=env, check=False)
        record("ping_after_shutdown_fails", ping_after.returncode != 0, {"returncode": ping_after.returncode, "stderr": ping_after.stderr.strip()})
        record("no_daemon_after_shutdown", len(live_daemon_pids_for_home(home)) == 0, {"pids": live_daemon_pids_for_home(home)})

        daemon_proc = start_daemon(args.bin, env)
        restart_startup_result, restart_startup_payload = mock_cli(
            args.mock_bin, env, out_dir, "08-startup-after-restart", "--scenario", "startup"
        )
        restart_startup = scenario_result_data(restart_startup_payload)
        record(
            "startup_after_restart_restores_remote_machines",
            restart_startup_result.returncode == 0 and restart_startup.get("restored_remote_machines") == 3,
            restart_startup,
        )

        restart_snapshot_result, restart_snapshot_payload = mock_cli(
            args.mock_bin, env, out_dir, "09-snapshot-after-restart", "--scenario", "snapshot"
        )
        restart_snapshot = scenario_result_data(restart_snapshot_payload)
        record(
            "snapshot_after_restart_preserves_health_counts",
            restart_snapshot_result.returncode == 0
            and restart_snapshot.get("remote_machine_health_counts") == {"healthy": 0, "cached": 2, "offline": 1},
            restart_snapshot,
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
