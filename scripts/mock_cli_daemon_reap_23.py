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
        description="Spawn 23 orphan daemons and verify a fresh daemon reaps them."
    )
    parser.add_argument("--bin", default="./target/debug/yggterm")
    parser.add_argument("--out-dir", default="/tmp/yggterm-mock-cli-daemon-reap-23")
    parser.add_argument("--orphan-count", type=int, default=23)
    parser.add_argument("--reap-after-ms", type=int, default=1000)
    parser.add_argument("--age-wait-ms", type=int, default=1600)
    parser.add_argument("--shutdown-ms", type=int, default=600000)
    return parser.parse_args()


def run(argv: list[str], *, env: dict[str, str], check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(argv, text=True, capture_output=True, check=check, env=env)


def start_daemon(binary: str, env: dict[str, str]) -> subprocess.Popen:
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


def process_alive(pid: int) -> bool:
    stat = Path(f"/proc/{pid}/stat")
    if not stat.exists():
        return False
    try:
        state = stat.read_text(encoding="utf-8").split(") ", 1)[1].split()[0]
    except Exception:
        return False
    return state != "Z"


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir)
    if out_dir.exists():
        shutil.rmtree(out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    sandbox = Path(tempfile.mkdtemp(prefix="yggterm-daemon-reap-23-"))
    orphan_procs: list[subprocess.Popen] = []
    primary_proc: subprocess.Popen | None = None
    checks: list[dict] = []

    def record(name: str, passed: bool, details: dict) -> None:
        checks.append({"name": name, "passed": passed, "details": details})

    try:
        orphan_envs: list[dict[str, str]] = []
        orphan_homes: list[str] = []
        orphan_pids: list[int] = []
        for idx in range(args.orphan_count):
            home = sandbox / f"orphan-{idx:02d}"
            home.mkdir(parents=True, exist_ok=True)
            env = os.environ.copy()
            env["YGGTERM_HOME"] = str(home)
            env["YGGTERM_DAEMON_IDLE_SHUTDOWN_MS"] = str(args.shutdown_ms)
            proc = start_daemon(args.bin, env)
            orphan_procs.append(proc)
            orphan_envs.append(env)
            orphan_homes.append(str(home))
            orphan_pids.append(proc.pid)
            record(
                f"orphan_{idx+1:02d}_started",
                proc.poll() is None,
                {"pid": proc.pid, "home": str(home)},
            )

        time.sleep(args.age_wait_ms / 1000.0)

        primary_home = sandbox / "primary"
        primary_home.mkdir(parents=True, exist_ok=True)
        primary_env = os.environ.copy()
        primary_env["YGGTERM_HOME"] = str(primary_home)
        primary_env["YGGTERM_DAEMON_IDLE_SHUTDOWN_MS"] = str(args.shutdown_ms)
        primary_env["YGGTERM_DAEMON_ORPHAN_REAP_AFTER_MS"] = str(args.reap_after_ms)
        primary_proc = start_daemon(args.bin, primary_env)
        record(
            "primary_started",
            primary_proc.poll() is None,
            {"pid": primary_proc.pid, "home": str(primary_home)},
        )

        time.sleep(2.0)
        reaped = []
        survivors = []
        for home, pid in zip(orphan_homes, orphan_pids):
            alive = process_alive(pid)
            if alive:
                survivors.append({"pid": pid, "home": home})
            else:
                reaped.append({"pid": pid, "home": home})

        record(
            "all_orphans_reaped",
            len(reaped) == args.orphan_count,
            {
                "expected": args.orphan_count,
                "reaped": len(reaped),
                "survivors": survivors,
            },
        )
        record(
            "primary_survived_reap",
            process_alive(primary_proc.pid),
            {"pid": primary_proc.pid, "home": str(primary_home)},
        )

        run([str(Path(args.bin).resolve()), "server", "shutdown"], env=primary_env, check=False)
        for _ in range(20):
            if not process_alive(primary_proc.pid):
                break
            time.sleep(0.1)
        record(
            "primary_shutdown_clean",
            not process_alive(primary_proc.pid),
            {"pid": primary_proc.pid},
        )
    finally:
        stop_process(primary_proc)
        for proc in orphan_procs:
            stop_process(proc)

    summary = {
        "sandbox": str(sandbox),
        "check_count": len(checks),
        "passed": sum(1 for check in checks if check["passed"]),
        "failed": sum(1 for check in checks if not check["passed"]),
        "checks": checks,
    }
    (out_dir / "summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(json.dumps(summary, indent=2))
    return 0 if summary["failed"] == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
