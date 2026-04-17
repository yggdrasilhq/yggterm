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
    parser.add_argument("--iterations", type=int, default=3)
    parser.add_argument("--growth-tolerance-mib", type=int, default=32)
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


def wait_for_trim_status(
    mock_bin: str,
    env: dict[str, str],
    out_dir: Path,
    name: str,
    *,
    idle_cap: int,
    minimum_wait_ms: int,
    poll_ms: int = 1000,
) -> tuple[subprocess.CompletedProcess, dict]:
    deadline = time.monotonic() + (max(minimum_wait_ms, 15000) / 1000.0)
    last_result: subprocess.CompletedProcess | None = None
    last_status: dict = {}
    while time.monotonic() < deadline:
        last_result, status_events = mock_cli(mock_bin, env, out_dir, name, "--scenario", "status")
        last_status = scenario_result_data(status_events)
        session_count = int(last_status.get("terminal_session_count") or 0)
        retained_bytes = int(last_status.get("terminal_retained_bytes") or 0)
        budget_bytes = max(session_count * idle_cap, 2 * 1024 * 1024) if idle_cap > 0 else 0
        if last_result.returncode == 0 and budget_bytes > 0 and retained_bytes <= budget_bytes:
            return last_result, last_status
        time.sleep(poll_ms / 1000.0)
    return last_result or subprocess.CompletedProcess([], 1), last_status


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
    iteration_samples: list[dict] = []

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

        growth_tolerance_bytes = args.growth_tolerance_mib * 1024 * 1024
        session_cap = 0
        idle_cap = 0
        retained_bytes = 0
        trimmed_bytes = 0
        rss_peak = 0
        rss_after_trim = 0
        session_count = 0
        for iteration in range(args.iterations):
            name_prefix = f"{iteration + 1:02d}"
            footprint_result, footprint_events = mock_cli(
                args.mock_bin,
                env,
                out_dir,
                f"{name_prefix}-terminal-footprint",
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
            record(f"footprint_scenario_ok_{name_prefix}", footprint_result.returncode == 0, footprint)
            record(
                f"footprint_created_{args.session_count}_sessions_{name_prefix}",
                len(session_paths) == args.session_count,
                {"session_paths": session_paths},
            )
            record(
                f"footprint_session_count_at_least_{args.session_count}_{name_prefix}",
                session_count >= args.session_count,
                footprint,
            )
            record(f"footprint_retained_bytes_nonzero_{name_prefix}", retained_bytes > 0, footprint)
            record(
                f"footprint_retained_bytes_bounded_per_session_{name_prefix}",
                session_cap > 0 and retained_bytes <= session_count * session_cap,
                footprint,
            )
            record(
                f"footprint_retained_bytes_under_64mib_{name_prefix}",
                retained_bytes <= 64 * 1024 * 1024,
                {"retained_bytes": retained_bytes},
            )
            record(
                f"daemon_rss_under_ceiling_{name_prefix}",
                rss_peak <= args.rss_ceiling_mib * 1024 * 1024,
                {"rss_bytes": rss_peak, "rss_ceiling_mib": args.rss_ceiling_mib},
            )
            record(
                f"single_daemon_after_load_{name_prefix}",
                len(pids) == 1,
                {"pids": pids},
            )

            status_after_result, status_after_events = mock_cli(
                args.mock_bin,
                env,
                out_dir,
                f"{name_prefix}-status-after-load",
                "--scenario",
                "status",
            )
            status1 = scenario_result_data(status_after_events)
            record(f"status_after_load_ok_{name_prefix}", status_after_result.returncode == 0, status1)
            record(
                f"status_matches_loaded_terminal_bytes_{name_prefix}",
                int(status1.get("terminal_retained_bytes") or -1) == retained_bytes,
                {"loaded": retained_bytes, "status": status1.get("terminal_retained_bytes")},
            )
            record(
                f"status_matches_loaded_session_count_{name_prefix}",
                int(status1.get("terminal_session_count") or -1) == session_count,
                {"loaded": session_count, "status": status1.get("terminal_session_count")},
            )

            ping_after = run([str(Path(args.bin).resolve()), "server", "ping"], env=env, check=False)
            record(f"ping_after_load_ok_{name_prefix}", ping_after.returncode == 0, {"returncode": ping_after.returncode})

            snapshot_result, snapshot_events = mock_cli(
                args.mock_bin,
                env,
                out_dir,
                f"{name_prefix}-snapshot-after-load",
                "--scenario",
                "snapshot",
            )
            snapshot_data = scenario_result_data(snapshot_events)
            record(f"snapshot_after_load_ok_{name_prefix}", snapshot_result.returncode == 0, snapshot_data)

            status_trim_result, status2 = wait_for_trim_status(
                args.mock_bin,
                env,
                out_dir,
                f"{name_prefix}-status-after-trim",
                idle_cap=idle_cap,
                minimum_wait_ms=args.idle_wait_ms,
            )
            trimmed_bytes = int(status2.get("terminal_retained_bytes") or 0)
            trimmed_session_count = int(status2.get("terminal_session_count") or 0)
            pids = live_daemon_pids_for_home(home)
            rss_after_trim = max((rss_bytes_for_pid(pid) for pid in pids), default=0)
            record(f"post_trim_status_ok_{name_prefix}", status_trim_result.returncode == 0, status2)
            record(
                f"idle_trim_reduced_or_equal_bytes_{name_prefix}",
                trimmed_bytes <= retained_bytes,
                {"before": retained_bytes, "after": trimmed_bytes},
            )
            record(
                f"idle_trim_converges_near_idle_budget_{name_prefix}",
                idle_cap > 0 and trimmed_bytes <= max(trimmed_session_count * idle_cap, 2 * 1024 * 1024),
                {
                    "trimmed_bytes": trimmed_bytes,
                    "idle_cap": idle_cap,
                    "session_count": trimmed_session_count,
                    "minimum_wait_ms": args.idle_wait_ms,
                },
            )
            record(
                f"daemon_alive_after_trim_{name_prefix}",
                daemon_proc.poll() is None and bool(pids),
                {"poll": daemon_proc.poll(), "pids": pids},
            )
            record(
                f"rss_not_worse_after_trim_{name_prefix}",
                rss_after_trim <= rss_peak + (16 * 1024 * 1024),
                {
                    "rss_peak": rss_peak,
                    "rss_after_trim": rss_after_trim,
                    "tolerance_bytes": 16 * 1024 * 1024,
                },
            )
            record(
                f"single_daemon_after_trim_{name_prefix}",
                len(pids) == 1,
                {"pids": pids},
            )
            iteration_samples.append(
                {
                    "iteration": iteration + 1,
                    "loaded_retained_bytes": retained_bytes,
                    "trimmed_retained_bytes": trimmed_bytes,
                    "rss_peak": rss_peak,
                    "rss_after_trim": rss_after_trim,
                    "session_count": session_count,
                }
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

        loaded_retained_values = [sample["loaded_retained_bytes"] for sample in iteration_samples]
        trimmed_retained_values = [sample["trimmed_retained_bytes"] for sample in iteration_samples]
        rss_peak_values = [sample["rss_peak"] for sample in iteration_samples]
        record(
            "loaded_retained_growth_bounded",
            bool(loaded_retained_values)
            and max(loaded_retained_values) - min(loaded_retained_values) <= growth_tolerance_bytes,
            {"samples": iteration_samples, "growth_tolerance_bytes": growth_tolerance_bytes},
        )
        record(
            "trimmed_retained_growth_bounded",
            bool(trimmed_retained_values)
            and max(trimmed_retained_values) - min(trimmed_retained_values) <= growth_tolerance_bytes,
            {"samples": iteration_samples, "growth_tolerance_bytes": growth_tolerance_bytes},
        )
        record(
            "daemon_rss_growth_bounded",
            bool(rss_peak_values)
            and max(rss_peak_values) - min(rss_peak_values) <= growth_tolerance_bytes,
            {"samples": iteration_samples, "growth_tolerance_bytes": growth_tolerance_bytes},
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
