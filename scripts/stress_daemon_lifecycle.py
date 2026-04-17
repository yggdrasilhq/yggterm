#!/usr/bin/env python3
import argparse
import json
import os
import shutil
import signal
import subprocess
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_BIN = ROOT / "target" / "debug" / "yggterm"


def run(cmd: list[str], *, env: dict[str, str], check: bool = True) -> subprocess.CompletedProcess:
    proc = subprocess.run(cmd, text=True, capture_output=True, env=env)
    if check and proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip() or proc.stdout.strip() or f"command failed: {cmd!r}")
    return proc


def daemon_pids_for_home(home: Path) -> list[int]:
    home_text = str(home)
    results: list[int] = []
    for proc_dir in Path("/proc").iterdir():
        if not proc_dir.name.isdigit():
            continue
        pid = int(proc_dir.name)
        try:
            cmdline = (proc_dir / "cmdline").read_bytes().split(b"\0")
            if not cmdline or not any(part == b"server" for part in cmdline) or not any(
                part == b"daemon" for part in cmdline
            ):
                continue
            if b"yggterm" not in cmdline[0]:
                continue
            environ = (proc_dir / "environ").read_bytes().split(b"\0")
            env_map = {}
            for item in environ:
                if b"=" in item:
                    key, value = item.split(b"=", 1)
                    env_map[key.decode("utf-8", "ignore")] = value.decode("utf-8", "ignore")
            if env_map.get("YGGTERM_HOME") != home_text:
                continue
            results.append(pid)
        except Exception:
            continue
    return sorted(results)


def proc_exe_target(pid: int) -> str:
    try:
        return os.readlink(f"/proc/{pid}/exe")
    except OSError:
        return ""


def wait_for(predicate, *, timeout: float, step: float = 0.1):
    deadline = time.time() + timeout
    last = None
    while time.time() < deadline:
        last = predicate()
        if last:
            return last
        time.sleep(step)
    return last


def ensure_single_current_daemon(home: Path, current_bin: Path) -> dict:
    pids = daemon_pids_for_home(home)
    if len(pids) != 1:
        raise AssertionError(f"expected exactly one daemon for {home}, found {pids}")
    pid = pids[0]
    exe = proc_exe_target(pid)
    if " (deleted)" in exe:
        raise AssertionError(f"daemon {pid} still runs a deleted binary: {exe}")
    if exe != str(current_bin):
        raise AssertionError(f"daemon {pid} runs unexpected binary: exe={exe} expected={current_bin}")
    return {"pid": pid, "exe": exe}


def launch_deleted_old_daemon(home: Path, current_bin: Path, work_dir: Path) -> dict:
    old_bin = work_dir / "old-yggterm"
    old_stderr = work_dir / "old-daemon.stderr.log"
    shutil.copy2(current_bin, old_bin)
    env = os.environ.copy()
    env["YGGTERM_HOME"] = str(home)
    stderr_handle = old_stderr.open("w")
    proc = subprocess.Popen(
        [str(old_bin), "server", "daemon"],
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=stderr_handle,
        start_new_session=True,
    )
    time.sleep(0.5)
    old_pids = wait_for(lambda: daemon_pids_for_home(home), timeout=8.0)
    stderr_handle.close()
    if not old_pids:
        raise AssertionError(
            f"old daemon never appeared; exit={proc.poll()} stderr={old_stderr.read_text().strip()!r}"
        )
    os.unlink(old_bin)
    stale_pid = None
    deadline = time.time() + 4.0
    while time.time() < deadline:
        for pid in daemon_pids_for_home(home):
            if " (deleted)" in proc_exe_target(pid):
                stale_pid = pid
                break
        if stale_pid is not None:
            break
        time.sleep(0.1)
    if stale_pid is None:
        raise AssertionError("failed to get a deleted-binary daemon for lifecycle stress")
    return {"stale_pid": stale_pid}


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--home", required=True)
    parser.add_argument("--iterations", type=int, default=3)
    parser.add_argument("--binary", default=str(DEFAULT_BIN))
    parser.add_argument("--keep-home", action="store_true")
    args = parser.parse_args()

    home = Path(args.home).expanduser().resolve()
    current_bin = Path(args.binary).expanduser().resolve()
    work_dir = home / "stress-work"
    if home.exists() and not args.keep_home:
        shutil.rmtree(home)
    home.mkdir(parents=True, exist_ok=True)
    work_dir.mkdir(parents=True, exist_ok=True)

    env = os.environ.copy()
    env["YGGTERM_HOME"] = str(home)
    results = []

    for ix in range(args.iterations):
        run([str(current_bin), "server", "shutdown"], env=env, check=False)
        time.sleep(0.2)
        for pid in daemon_pids_for_home(home):
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        wait_for(lambda: not daemon_pids_for_home(home), timeout=5.0)

        stale = launch_deleted_old_daemon(home, current_bin, work_dir)
        status = json.loads(run([str(current_bin), "server", "status"], env=env).stdout)
        ping = run([str(current_bin), "server", "ping"], env=env).stdout.strip()
        snapshot = json.loads(run([str(current_bin), "server", "snapshot"], env=env).stdout)
        single = ensure_single_current_daemon(home, current_bin)
        results.append(
            {
                "iteration": ix,
                "stale_pid": stale["stale_pid"],
                "status_version": status.get("server_version"),
                "ping": ping,
                "snapshot_counts": {
                    "stored_sessions": len(snapshot.get("stored_sessions") or []),
                    "live_sessions": len(snapshot.get("live_sessions") or []),
                    "remote_machines": len(snapshot.get("remote_machines") or []),
                },
                "current_daemon": single,
            }
        )

    print(json.dumps({"home": str(home), "iterations": args.iterations, "results": results}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
