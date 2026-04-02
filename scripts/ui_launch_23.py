#!/usr/bin/env python3
import argparse
import json
import os
import signal
import shutil
import subprocess
import time
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Launch a fresh local Yggterm GUI 23 times, require a real window_spawned event, "
            "verify app-control state and DOM counts, then cleanly shut it down."
        )
    )
    parser.add_argument("--bin", default="./target/debug/yggterm")
    parser.add_argument("--count", type=int, default=23)
    parser.add_argument("--spawn-budget-ms", type=int, default=900)
    parser.add_argument("--timeout-ms", type=int, default=8000)
    parser.add_argument("--poll", type=float, default=0.1)
    parser.add_argument("--seed", type=int, default=23)
    parser.add_argument("--out-dir", default="/tmp/yggterm-launch-23")
    return parser.parse_args()


def run_process(argv: list[str], *, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(argv, check=check, text=True, capture_output=True)


def run_json(command: str) -> dict:
    result = run_process(["bash", "-lc", command], check=False)
    stdout = result.stdout.strip()
    stderr = result.stderr.strip()
    if result.returncode != 0 or not stdout:
        raise RuntimeError(
            f"command failed rc={result.returncode}: {command}\nstdout:\n{stdout or '<empty>'}\nstderr:\n{stderr or '<empty>'}"
        )
    try:
        return json.loads(stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(
            f"invalid json: {command}\nstdout:\n{stdout or '<empty>'}\nstderr:\n{stderr or '<empty>'}"
        ) from error


def local_yggterm_home() -> Path:
    override = os.environ.get("YGGTERM_HOME", "").strip()
    if override:
        return Path(override).expanduser()
    return Path.home() / ".yggterm"


def local_yggterm_path(*parts: str) -> Path:
    return local_yggterm_home().joinpath(*parts)


def local_x11_window_count(title: str = "Yggterm") -> int:
    display = os.environ.get("DISPLAY", "").strip()
    if not display or shutil.which("xwininfo") is None:
        return 0
    result = subprocess.run(
        ["xwininfo", "-root", "-tree"],
        check=False,
        text=True,
        capture_output=True,
        env={**os.environ, "DISPLAY": display},
    )
    if result.returncode != 0:
        return 0
    marker = f'"{title}"'
    return sum(1 for line in result.stdout.splitlines() if marker in line)


def app_state(binary: str, timeout_ms: int) -> dict:
    payload = run_json(f"{Path(binary).resolve()} server app state --timeout-ms {timeout_ms}")
    return payload.get("data") or {}


def kill_local_clients(binary: str) -> None:
    binary_path = str(Path(binary).resolve())
    instances_root = local_yggterm_path("client-instances")
    if instances_root.is_dir():
        for path in instances_root.glob("*/*.json"):
            try:
                record = json.loads(path.read_text(encoding="utf-8"))
            except Exception:
                continue
            pid = int(record.get("pid") or 0)
            if pid > 0:
                try:
                    os.kill(pid, signal.SIGTERM)
                except Exception:
                    pass
        time.sleep(0.3)
        for path in instances_root.glob("*/*.json"):
            try:
                record = json.loads(path.read_text(encoding="utf-8"))
            except Exception:
                continue
            pid = int(record.get("pid") or 0)
            if pid > 0:
                try:
                    os.kill(pid, signal.SIGKILL)
                except Exception:
                    pass
    self_pid = os.getpid()
    for proc_dir in Path("/proc").iterdir():
        if not proc_dir.name.isdigit():
            continue
        pid = int(proc_dir.name)
        if pid == self_pid:
            continue
        try:
            raw = (proc_dir / "cmdline").read_bytes()
        except Exception:
            continue
        if not raw:
            continue
        argv = [part.decode("utf-8", errors="ignore") for part in raw.split(b"\0") if part]
        if not argv:
            continue
        is_local_client = argv[0] == binary_path
        is_local_daemon = len(argv) >= 4 and argv[0] == binary_path and argv[1:4] == [
            "server",
            "daemon",
            "--stdio",
        ]
        is_legacy_daemon = len(argv) >= 3 and "yggterm" in argv[0] and argv[1:3] == [
            "server",
            "daemon",
        ]
        if not (is_local_client or is_local_daemon or is_legacy_daemon):
            continue
        try:
            os.kill(pid, signal.SIGKILL)
        except Exception:
            pass


def latest_window_spawn_event_for_pid(pid: int, start_ms: int) -> dict | None:
    path = local_yggterm_path("event-trace.jsonl")
    if not path.exists():
        return None
    for line in reversed(path.read_text(encoding="utf-8").splitlines()):
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (
            event.get("pid") == pid
            and event.get("category") == "startup"
            and event.get("name") == "window_spawned"
        ):
            return event
        ts_ms = event.get("ts_ms")
        if isinstance(ts_ms, int) and ts_ms < start_ms:
            break
    return None


def launch_local_client(binary: str, timeout_s: float = 4.0) -> tuple[subprocess.Popen, dict]:
    binary_path = str(Path(binary).resolve())
    kill_local_clients(binary_path)
    env = os.environ.copy()
    env.setdefault("DISPLAY", ":10.0")
    env.setdefault("XAUTHORITY", str(Path.home() / ".Xauthority"))
    env["YGGTERM_ALLOW_MULTI_WINDOW"] = "1"
    env["YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF"] = "1"
    baseline_window_count = local_x11_window_count()
    proc = subprocess.Popen(
        [binary_path],
        cwd=str(Path(binary).resolve().parent.parent.parent),
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    start_ms = int(time.time() * 1000)
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        if local_x11_window_count() > baseline_window_count:
            return proc, {
                "category": "startup",
                "name": "window_spawned",
                "payload": {
                    "elapsed_ms": int(time.time() * 1000) - start_ms,
                    "source": "x11_root_tree",
                },
            }
        event = latest_window_spawn_event_for_pid(proc.pid, start_ms)
        if event is not None:
            return proc, event
        if proc.poll() is not None:
            break
        time.sleep(0.05)
    raise RuntimeError(
        f"local launch did not emit window_spawned within {timeout_s:.2f}s for pid {proc.pid}"
    )


def wait_for_visible_state(binary: str, timeout_ms: int, timeout_s: float = 8.0, poll_s: float = 0.25) -> dict:
    started = time.monotonic()
    last_error = None
    while time.monotonic() - started <= timeout_s:
        try:
            state = app_state(binary, timeout_ms)
            window = state.get("window") or {}
            dom = state.get("dom") or {}
            if not window.get("visible"):
                raise RuntimeError("window not visible")
            if dom.get("shell_root_count") != 1:
                raise RuntimeError(f"shell_root_count={dom.get('shell_root_count')}")
            if dom.get("sidebar_count") != 1:
                raise RuntimeError(f"sidebar_count={dom.get('sidebar_count')}")
            if dom.get("titlebar_count") != 1:
                raise RuntimeError(f"titlebar_count={dom.get('titlebar_count')}")
            if dom.get("main_surface_count") != 1:
                raise RuntimeError(f"main_surface_count={dom.get('main_surface_count')}")
            return state
        except Exception as error:  # noqa: BLE001
            last_error = error
            time.sleep(poll_s)
    raise RuntimeError(f"visible app state timed out after {timeout_s:.2f}s: {last_error}")


def shutdown_launch(proc: subprocess.Popen) -> None:
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            proc.kill()
    run_process(["bash", "-lc", "pkill -f 'yggterm server daemon' || true"], check=False)


def write_json(path: Path, payload: dict) -> str:
    path.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    return str(path)


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    results: list[dict] = []

    for index in range(args.count):
        proc = None
        event = None
        try:
            proc, event = launch_local_client(args.bin)
            state = wait_for_visible_state(args.bin, args.timeout_ms, poll_s=args.poll)
            payload = (event or {}).get("payload") or {}
            elapsed_ms = payload.get("elapsed_ms")
            notifications = ((state.get("shell") or {}).get("notifications_count")) or 0
            result = {
                "trial": index,
                "pid": proc.pid,
                "elapsed_ms": elapsed_ms,
                "within_budget": (elapsed_ms or 10_000) <= args.spawn_budget_ms,
                "notifications_count": notifications,
                "dom_counts": {
                    "shell_root_count": ((state.get("dom") or {}).get("shell_root_count")),
                    "sidebar_count": ((state.get("dom") or {}).get("sidebar_count")),
                    "titlebar_count": ((state.get("dom") or {}).get("titlebar_count")),
                    "main_surface_count": ((state.get("dom") or {}).get("main_surface_count")),
                },
                "state_dump": write_json(out_dir / f"launch-{index:02d}.json", state),
            }
            results.append(result)
        except Exception as error:  # noqa: BLE001
            state = {}
            try:
                state = app_state(args.bin, args.timeout_ms)
            except Exception:
                state = {}
            results.append(
                {
                    "trial": index,
                    "pid": proc.pid if proc is not None else None,
                    "elapsed_ms": (((event or {}).get("payload") or {}).get("elapsed_ms")),
                    "within_budget": False,
                    "error": str(error),
                    "state_dump": write_json(out_dir / f"launch-{index:02d}-failure.json", state),
                }
            )
        finally:
            if proc is not None:
                shutdown_launch(proc)

    summary = {
        "count": args.count,
        "spawn_budget_ms": args.spawn_budget_ms,
        "launch_failures": len([item for item in results if item.get("error")]),
        "spawn_budget_failures": len([item for item in results if not item.get("within_budget")]),
        "notification_anomalies": len([item for item in results if (item.get("notifications_count") or 0) > 0]),
        "results": results,
    }
    summary_path = out_dir / "summary.json"
    summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(summary_path)
    print(json.dumps(summary, indent=2))
    return 0 if (
        summary["launch_failures"] == 0
        and summary["spawn_budget_failures"] == 0
        and summary["notification_anomalies"] == 0
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
