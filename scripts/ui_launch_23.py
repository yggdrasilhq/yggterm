#!/usr/bin/env python3
import argparse
import json
import os
import signal
import shutil
import statistics
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
    parser.add_argument("--avg-spawn-budget-ms", type=int, default=520)
    parser.add_argument("--p95-spawn-budget-ms", type=int, default=600)
    parser.add_argument("--timeout-ms", type=int, default=8000)
    parser.add_argument("--poll", type=float, default=0.1)
    parser.add_argument("--seed", type=int, default=23)
    parser.add_argument("--out-dir", default="/tmp/yggterm-launch-23")
    parser.add_argument("--display", default=os.environ.get("DISPLAY", ":10.0"))
    parser.add_argument("--xvfb", action="store_true")
    parser.add_argument("--app-rss-ceiling-mib", type=int, default=384)
    parser.add_argument("--stack-rss-ceiling-mib", type=int, default=768)
    parser.add_argument("--shutdown-timeout-ms", type=int, default=5000)
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


def display_ready(display: str) -> bool:
    if not display or shutil.which("xdpyinfo") is None:
        return False
    result = subprocess.run(
        ["xdpyinfo"],
        check=False,
        text=True,
        capture_output=True,
        env={**os.environ, "DISPLAY": display},
    )
    return result.returncode == 0


def app_state(binary: str, timeout_ms: int) -> dict:
    payload = run_json(f"{Path(binary).resolve()} server app state --timeout-ms {timeout_ms}")
    return payload.get("data") or {}


def process_rss_kb(pid: int) -> int:
    status = Path(f"/proc/{pid}/status")
    if not status.exists():
        return 0
    for line in status.read_text(encoding="utf-8").splitlines():
        if line.startswith("VmRSS:"):
            parts = line.split()
            if len(parts) >= 2:
                return int(parts[1])
    return 0


def child_pids(pid: int) -> list[int]:
    children: list[int] = []
    for entry in Path("/proc").iterdir():
        if not entry.name.isdigit():
            continue
        try:
            stat = (entry / "stat").read_text(encoding="utf-8")
        except OSError:
            continue
        try:
            ppid = int(stat.split(") ", 1)[1].split()[1])
        except (IndexError, ValueError):
            continue
        if ppid == pid:
            children.append(int(entry.name))
    return sorted(children)


def process_alive(pid: int) -> bool:
    stat = Path(f"/proc/{pid}/stat")
    if not stat.exists():
        return False
    try:
        state = stat.read_text(encoding="utf-8").split(") ", 1)[1].split()[0]
    except Exception:
        return False
    return state != "Z"


def stack_rss_sample(pid: int) -> dict:
    child_rss_kb = {
        child: process_rss_kb(child)
        for child in child_pids(pid)
        if process_alive(child)
    }
    main_rss_kb = process_rss_kb(pid)
    return {
        "main_rss_kb": main_rss_kb,
        "child_rss_kb": child_rss_kb,
        "total_rss_kb": main_rss_kb + sum(child_rss_kb.values()),
    }


def wait_for_stack_exit(pids: list[int], timeout_s: float) -> list[int]:
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        alive = [pid for pid in pids if process_alive(pid)]
        if not alive:
            return []
        time.sleep(0.1)
    return [pid for pid in pids if process_alive(pid)]


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
    env["YGGTERM_ALLOW_MULTI_WINDOW"] = "1"
    env["YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF"] = "1"
    if env.get("DISPLAY") != os.environ.get("DISPLAY"):
        env.pop("XAUTHORITY", None)
        env.pop("WAYLAND_DISPLAY", None)
        env.pop("XRDP_SESSION", None)
        env.pop("XRDP_SOCKET_PATH", None)
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


def shutdown_launch(binary: str, proc: subprocess.Popen) -> None:
    run_process([str(Path(binary).resolve()), "server", "shutdown"], check=False)
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=2)
        except subprocess.TimeoutExpired:
            proc.kill()


def write_json(path: Path, payload: dict) -> str:
    path.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    return str(path)


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    results: list[dict] = []
    previous_display = os.environ.get("DISPLAY")
    if args.display:
        os.environ["DISPLAY"] = args.display
    xvfb = None
    xvfb_log = None
    if args.xvfb:
        xvfb_log = (out_dir / "xvfb.log").open("w")
        xvfb = subprocess.Popen(
            ["Xvfb", args.display, "-screen", "0", "1600x1000x24", "-ac"],
            stdout=xvfb_log,
            stderr=subprocess.STDOUT,
        )
        deadline = time.monotonic() + 10.0
        while time.monotonic() < deadline:
            if display_ready(args.display):
                break
            time.sleep(0.1)
        else:
            raise RuntimeError(f"Xvfb display never became ready on {args.display}")

    try:
        for index in range(args.count):
            proc = None
            event = None
            stack_sample = None
            observed_stack_pids: list[int] = []
            result: dict | None = None
            try:
                proc, event = launch_local_client(args.bin)
                state = wait_for_visible_state(args.bin, args.timeout_ms, poll_s=args.poll)
                payload = (event or {}).get("payload") or {}
                elapsed_ms = payload.get("elapsed_ms")
                notifications = ((state.get("shell") or {}).get("notifications_count")) or 0
                stack_sample = stack_rss_sample(proc.pid)
                observed_stack_pids = [proc.pid, *stack_sample["child_rss_kb"].keys()]
                result = {
                    "trial": index,
                    "pid": proc.pid,
                    "elapsed_ms": elapsed_ms,
                    "within_budget": (elapsed_ms or 10_000) <= args.spawn_budget_ms,
                    "within_rss_budget": (
                        stack_sample["main_rss_kb"] <= args.app_rss_ceiling_mib * 1024
                        and stack_sample["total_rss_kb"] <= args.stack_rss_ceiling_mib * 1024
                    ),
                    "notifications_count": notifications,
                    "memory": stack_sample,
                    "dom_counts": {
                        "shell_root_count": ((state.get("dom") or {}).get("shell_root_count")),
                        "sidebar_count": ((state.get("dom") or {}).get("sidebar_count")),
                        "titlebar_count": ((state.get("dom") or {}).get("titlebar_count")),
                        "main_surface_count": ((state.get("dom") or {}).get("main_surface_count")),
                    },
                    "state_dump": write_json(out_dir / f"launch-{index:02d}.json", state),
                }
            except Exception as error:  # noqa: BLE001
                state = {}
                try:
                    state = app_state(args.bin, args.timeout_ms)
                except Exception:
                    state = {}
                result = {
                    "trial": index,
                    "pid": proc.pid if proc is not None else None,
                    "elapsed_ms": (((event or {}).get("payload") or {}).get("elapsed_ms")),
                    "within_budget": False,
                    "within_rss_budget": False,
                    "error": str(error),
                    "memory": stack_sample or {},
                    "state_dump": write_json(out_dir / f"launch-{index:02d}-failure.json", state),
                }
            finally:
                if proc is not None:
                    shutdown_launch(args.bin, proc)
                    leaked_pids = wait_for_stack_exit(
                        observed_stack_pids,
                        timeout_s=max(args.shutdown_timeout_ms, 1000) / 1000.0,
                    )
                else:
                    leaked_pids = []
                if result is None:
                    result = {
                        "trial": index,
                        "pid": proc.pid if proc is not None else None,
                        "elapsed_ms": (((event or {}).get("payload") or {}).get("elapsed_ms")),
                        "within_budget": False,
                        "within_rss_budget": False,
                        "error": "launch result missing",
                    }
                result["leaked_pids_after_shutdown"] = leaked_pids
                result["clean_shutdown"] = not leaked_pids
                results.append(result)
    finally:
        if previous_display is None:
            os.environ.pop("DISPLAY", None)
        else:
            os.environ["DISPLAY"] = previous_display
        if xvfb is not None:
            xvfb.terminate()
            try:
                xvfb.wait(timeout=5)
            except subprocess.TimeoutExpired:
                xvfb.kill()
        if xvfb_log is not None:
            xvfb_log.close()

    summary = {
        "count": args.count,
        "spawn_budget_ms": args.spawn_budget_ms,
        "avg_spawn_budget_ms": args.avg_spawn_budget_ms,
        "p95_spawn_budget_ms": args.p95_spawn_budget_ms,
        "app_rss_ceiling_mib": args.app_rss_ceiling_mib,
        "stack_rss_ceiling_mib": args.stack_rss_ceiling_mib,
        "launch_failures": len([item for item in results if item.get("error")]),
        "spawn_budget_failures": len([item for item in results if not item.get("within_budget")]),
        "rss_budget_failures": len([item for item in results if not item.get("within_rss_budget")]),
        "notification_anomalies": len([item for item in results if (item.get("notifications_count") or 0) > 0]),
        "shutdown_leaks": len([item for item in results if not item.get("clean_shutdown")]),
        "results": results,
    }
    successful_elapsed_ms = sorted(
        int(item["elapsed_ms"])
        for item in results
        if item.get("error") is None and item.get("elapsed_ms") is not None
    )
    if successful_elapsed_ms:
        p95_index = max(
            0, min(len(successful_elapsed_ms) - 1, int((len(successful_elapsed_ms) - 1) * 0.95))
        )
        summary["spawn_latency_ms"] = {
            "min": successful_elapsed_ms[0],
            "max": successful_elapsed_ms[-1],
            "avg": round(statistics.mean(successful_elapsed_ms), 1),
            "p95": successful_elapsed_ms[p95_index],
        }
        summary["avg_spawn_budget_failures"] = int(
            summary["spawn_latency_ms"]["avg"] > args.avg_spawn_budget_ms
        )
        summary["p95_spawn_budget_failures"] = int(
            summary["spawn_latency_ms"]["p95"] > args.p95_spawn_budget_ms
        )
    else:
        summary["spawn_latency_ms"] = None
        summary["avg_spawn_budget_failures"] = 1
        summary["p95_spawn_budget_failures"] = 1
    summary_path = out_dir / "summary.json"
    summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(summary_path)
    print(json.dumps(summary, indent=2))
    return 0 if (
        summary["launch_failures"] == 0
        and summary["spawn_budget_failures"] == 0
        and summary["rss_budget_failures"] == 0
        and summary["avg_spawn_budget_failures"] == 0
        and summary["p95_spawn_budget_failures"] == 0
        and summary["notification_anomalies"] == 0
        and summary["shutdown_leaks"] == 0
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
