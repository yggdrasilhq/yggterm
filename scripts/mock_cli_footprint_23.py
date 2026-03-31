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
        description="Run 23 strict daemon footprint checks against an isolated YGGTERM_HOME."
    )
    parser.add_argument("--bin", default="./target/debug/yggterm")
    parser.add_argument("--mock-bin", default="./target/debug/yggterm-mock-cli")
    parser.add_argument("--out-dir", default="/tmp/yggterm-mock-cli-footprint-23")
    parser.add_argument("--session-count", type=int, default=23)
    parser.add_argument("--burst-bytes", type=int, default=524288)
    parser.add_argument("--idle-trim-ms", type=int, default=1500)
    parser.add_argument("--idle-wait-ms", type=int, default=14000)
    parser.add_argument("--idle-shutdown-ms", type=int, default=120000)
    parser.add_argument("--rss-ceiling-mib", type=int, default=384)
    return parser.parse_args()


def run(argv: list[str], *, env: dict[str, str], check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(argv, text=True, capture_output=True, check=check, env=env)


def parse_jsonl(path: Path) -> list[dict]:
    events: list[dict] = []
    if not path.exists():
        return events
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line:
            events.append(json.loads(line))
    return events


def scenario_result_data(events: list[dict]) -> dict:
    for event in reversed(events):
        payload = event.get("event") or {}
        if payload.get("kind") == "result":
            return event.get("data") or {}
    return {}


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
    started_at = time.monotonic()
    proc = subprocess.Popen(
        [str(Path(binary).resolve()), "server", "daemon"],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        env=env,
    )
    deadline = time.monotonic() + 10.0
    while time.monotonic() < deadline:
        if proc.poll() is not None:
            raise RuntimeError(f"daemon exited early with code {proc.returncode}")
        if run([str(Path(binary).resolve()), "server", "ping"], env=env, check=False).returncode == 0:
            proc.startup_ms = int((time.monotonic() - started_at) * 1000)
            return proc
        time.sleep(0.1)
    raise RuntimeError("daemon did not become reachable within 10s")


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
        try:
            cmdline = (proc_dir / "cmdline").read_bytes().replace(b"\x00", b" ").decode("utf-8", "ignore")
            environ = (proc_dir / "environ").read_bytes().decode("utf-8", "ignore")
        except Exception:
            continue
        if "yggterm server daemon" not in cmdline:
            continue
        if f"YGGTERM_HOME={home}" not in environ:
            continue
        pids.append(int(proc_dir.name))
    return sorted(pids)


def rss_bytes_for_pid(pid: int) -> int:
    status = Path(f"/proc/{pid}/status")
    if not status.exists():
        return 0
    for line in status.read_text(encoding="utf-8").splitlines():
        if line.startswith("VmRSS:"):
            parts = line.split()
            if len(parts) >= 2:
                return int(parts[1]) * 1024
    return 0


def cpu_time_ms_for_pid(pid: int) -> int:
    stat = Path(f"/proc/{pid}/stat")
    if not stat.exists():
        return 0
    fields = stat.read_text(encoding="utf-8").split()
    if len(fields) < 17:
        return 0
    utime = int(fields[13])
    stime = int(fields[14])
    clk_tck = os.sysconf(os.sysconf_names["SC_CLK_TCK"])
    return int(((utime + stime) * 1000) / max(clk_tck, 1))


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir)
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    home = Path(tempfile.mkdtemp(prefix="yggterm-mock-cli-footprint-home-"))
    env = os.environ.copy()
    env["YGGTERM_HOME"] = str(home)
    env["YGGTERM_DAEMON_TERMINAL_IDLE_TRIM_MS"] = str(args.idle_trim_ms)
    env["YGGTERM_DAEMON_IDLE_SHUTDOWN_MS"] = str(args.idle_shutdown_ms)

    checks: list[dict] = []
    daemon_proc: subprocess.Popen | None = None

    def record(name: str, passed: bool, details: dict) -> None:
        checks.append({"name": name, "passed": passed, "details": details})

    try:
        daemon_proc = start_daemon(args.bin, env)
        pids = live_daemon_pids_for_home(home)
        rss_before = rss_bytes_for_pid(pids[0]) if pids else 0
        startup_ms = int(getattr(daemon_proc, "startup_ms", 0))
        record(
            "daemon_started",
            daemon_proc.poll() is None,
            {"pid": daemon_proc.pid, "pids": pids, "startup_ms": startup_ms},
        )
        record(
            "startup_under_1500ms",
            0 < startup_ms <= 1500,
            {"startup_ms": startup_ms, "startup_budget_ms": 1500},
        )

        startup_result, startup_events = mock_cli(args.mock_bin, env, out_dir, "01-startup", "--scenario", "startup")
        startup = scenario_result_data(startup_events)
        record("startup_ok", startup_result.returncode == 0 and bool(startup.get("server_version")), startup)

        status_result, status_events = mock_cli(args.mock_bin, env, out_dir, "02-status-initial", "--scenario", "status")
        status0 = scenario_result_data(status_events)
        record("initial_status_ok", status_result.returncode == 0, status0)
        record(
            "initial_terminal_bytes_small",
            int(status0.get("terminal_retained_bytes") or 0) <= 65536,
            status0,
        )

        footprint_result, footprint_events = mock_cli(
            args.mock_bin,
            env,
            out_dir,
            "03-terminal-footprint",
            "--scenario",
            "terminal-footprint",
            "--session-count",
            str(args.session_count),
            "--burst-bytes",
            str(args.burst_bytes),
        )
        footprint = scenario_result_data(footprint_events)
        session_paths = footprint.get("session_paths") or []
        retained_bytes = int(footprint.get("terminal_retained_bytes") or 0)
        session_count = int(footprint.get("terminal_session_count") or 0)
        session_cap = int(footprint.get("terminal_session_buffer_limit_bytes") or 0)
        idle_cap = int(footprint.get("terminal_idle_buffer_limit_bytes") or 0)
        pids = live_daemon_pids_for_home(home)
        rss_peak = max((rss_bytes_for_pid(pid) for pid in pids), default=0)
        record("footprint_scenario_ok", footprint_result.returncode == 0, footprint)
        record("footprint_created_23_sessions", len(session_paths) == args.session_count, {"session_paths": session_paths})
        record("footprint_session_count_at_least_23", session_count >= args.session_count, footprint)
        record("footprint_retained_bytes_nonzero", retained_bytes > 0, footprint)
        record(
            "footprint_retained_bytes_bounded_per_session",
            session_cap > 0 and retained_bytes <= session_count * session_cap,
            footprint,
        )
        record(
            "footprint_retained_bytes_under_64mib",
            retained_bytes <= 64 * 1024 * 1024,
            {"retained_bytes": retained_bytes},
        )
        record(
            "daemon_rss_under_ceiling",
            rss_peak <= args.rss_ceiling_mib * 1024 * 1024,
            {"rss_bytes": rss_peak, "rss_ceiling_mib": args.rss_ceiling_mib},
        )

        status_after_result, status_after_events = mock_cli(args.mock_bin, env, out_dir, "04-status-after-load", "--scenario", "status")
        status1 = scenario_result_data(status_after_events)
        record("status_after_load_ok", status_after_result.returncode == 0, status1)
        record(
            "status_matches_loaded_terminal_bytes",
            int(status1.get("terminal_retained_bytes") or -1) == retained_bytes,
            {"loaded": retained_bytes, "status": status1.get("terminal_retained_bytes")},
        )
        record(
            "status_matches_loaded_session_count",
            int(status1.get("terminal_session_count") or -1) == session_count,
            {"loaded": session_count, "status": status1.get("terminal_session_count")},
        )

        ping_after = run([str(Path(args.bin).resolve()), "server", "ping"], env=env, check=False)
        record("ping_after_load_ok", ping_after.returncode == 0, {"returncode": ping_after.returncode})

        snapshot_result, snapshot_events = mock_cli(args.mock_bin, env, out_dir, "05-snapshot-after-load", "--scenario", "snapshot")
        snapshot_data = scenario_result_data(snapshot_events)
        record("snapshot_after_load_ok", snapshot_result.returncode == 0, snapshot_data)

        time.sleep(args.idle_wait_ms / 1000.0)
        status_trim_result, status_trim_events = mock_cli(args.mock_bin, env, out_dir, "06-status-after-trim", "--scenario", "status")
        status2 = scenario_result_data(status_trim_events)
        trimmed_bytes = int(status2.get("terminal_retained_bytes") or 0)
        pids = live_daemon_pids_for_home(home)
        rss_after_trim = max((rss_bytes_for_pid(pid) for pid in pids), default=0)
        record("post_trim_status_ok", status_trim_result.returncode == 0, status2)
        record(
            "idle_trim_reduced_or_equal_bytes",
            trimmed_bytes <= retained_bytes,
            {"before": retained_bytes, "after": trimmed_bytes},
        )
        record(
            "idle_trim_converges_near_idle_budget",
            idle_cap > 0 and trimmed_bytes <= max(args.session_count * idle_cap, 2 * 1024 * 1024),
            {"trimmed_bytes": trimmed_bytes, "idle_cap": idle_cap, "session_count": args.session_count},
        )
        record(
            "daemon_alive_after_trim",
            daemon_proc.poll() is None and bool(live_daemon_pids_for_home(home)),
            {"poll": daemon_proc.poll(), "pids": live_daemon_pids_for_home(home)},
        )
        record(
            "rss_not_worse_after_trim",
            rss_after_trim <= rss_peak + (16 * 1024 * 1024),
            {
                "rss_peak": rss_peak,
                "rss_after_trim": rss_after_trim,
                "tolerance_bytes": 16 * 1024 * 1024,
            },
        )

        pids = live_daemon_pids_for_home(home)
        idle_pid = pids[0] if pids else 0
        cpu_before_idle = cpu_time_ms_for_pid(idle_pid)
        time.sleep(5.0)
        cpu_after_idle = cpu_time_ms_for_pid(idle_pid)
        idle_cpu_ms = max(0, cpu_after_idle - cpu_before_idle)
        record(
            "idle_cpu_under_250ms_over_5s",
            idle_cpu_ms <= 250,
            {
                "pid": idle_pid,
                "idle_cpu_ms": idle_cpu_ms,
                "window_ms": 5000,
                "budget_ms": 250,
            },
        )

        footprint2_result, footprint2_events = mock_cli(
            args.mock_bin,
            env,
            out_dir,
            "07-terminal-footprint-second-pass",
            "--scenario",
            "terminal-footprint",
            "--session-count",
            str(args.session_count),
            "--burst-bytes",
            str(args.burst_bytes),
        )
        footprint2 = scenario_result_data(footprint2_events)
        retained_bytes2 = int(footprint2.get("terminal_retained_bytes") or 0)
        session_count2 = int(footprint2.get("terminal_session_count") or 0)
        record("second_footprint_scenario_ok", footprint2_result.returncode == 0, footprint2)
        record(
            "second_footprint_still_bounded",
            session_cap > 0 and retained_bytes2 <= session_count2 * session_cap,
            footprint2,
        )

        shutdown_result, shutdown_events = mock_cli(args.mock_bin, env, out_dir, "08-graceful-shutdown", "--scenario", "graceful-shutdown")
        shutdown_data = scenario_result_data(shutdown_events)
        record("graceful_shutdown_ok", shutdown_result.returncode == 0, shutdown_data)
        time.sleep(0.5)
        record(
            "daemon_gone_after_shutdown",
            not live_daemon_pids_for_home(home),
            {"pids": live_daemon_pids_for_home(home)},
        )
    finally:
        stop_process(daemon_proc)

    passed = sum(1 for check in checks if check["passed"])
    summary = {
        "check_count": len(checks),
        "passed": passed,
        "failed": len(checks) - passed,
        "checks": checks,
    }
    (out_dir / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(json.dumps(summary, indent=2))
    return 0 if passed == len(checks) else 1


if __name__ == "__main__":
    raise SystemExit(main())
