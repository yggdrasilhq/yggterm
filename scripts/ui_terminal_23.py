#!/usr/bin/env python3
import argparse
import json
import os
import random
import re
import shlex
import subprocess
import time
from pathlib import Path


MARKER_BEGIN = "__YGGTERM23_BEGIN__"
MARKER_END = "__YGGTERM23_END__"
SHORT_HASH_RE = re.compile(r"^[0-9a-f]{7,8}$")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Create 23 real terminals through app-control, run probe commands, "
            "verify viewport/title/summary state, and remove the created live sessions."
        )
    )
    parser.add_argument("--host", default="local")
    parser.add_argument("--bin", default="./target/debug/yggterm")
    parser.add_argument("--count", type=int, default=23)
    parser.add_argument("--seed", type=int, default=23)
    parser.add_argument("--timeout-ms", type=int, default=8000)
    parser.add_argument("--poll", type=float, default=0.15)
    parser.add_argument("--ready-budget", type=float, default=2.3)
    parser.add_argument("--spawn-budget", type=float, default=0.9)
    parser.add_argument("--summary-budget", type=float, default=12.0)
    parser.add_argument("--launch-local", action="store_true")
    parser.add_argument("--out-dir", default="/tmp/yggterm-terminal-23")
    return parser.parse_args()


def run_process(argv: list[str], *, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(argv, check=check, text=True, capture_output=True)


def run_control(host: str, command: str, *, check: bool = True) -> subprocess.CompletedProcess:
    if host == "local":
        return run_process(["bash", "-lc", command], check=check)
    return run_process(["ssh", host, command], check=check)


def run_json(host: str, command: str) -> dict:
    result = run_control(host, command, check=False)
    stdout = result.stdout.strip()
    stderr = result.stderr.strip()
    if result.returncode != 0 and not stdout:
        raise RuntimeError(
            f"command failed rc={result.returncode}: {command}\nstderr:\n{stderr or '<empty>'}"
        )
    try:
        return json.loads(stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(
            f"invalid json rc={result.returncode}: {command}\nstdout:\n{stdout or '<empty>'}\nstderr:\n{stderr or '<empty>'}"
        ) from error


def run_capture(argv: list[str]) -> tuple[int, str, str]:
    result = subprocess.run(argv, check=False, text=True, capture_output=True)
    return result.returncode, result.stdout, result.stderr


def quote(value: str) -> str:
    return shlex.quote(value)


def app_state(host: str, binary: str, timeout_ms: int) -> dict:
    payload = run_json(host, f"{quote(binary)} server app state --timeout-ms {timeout_ms}")
    return payload.get("data") or {}


def app_create_terminal(
    host: str,
    binary: str,
    timeout_ms: int,
    machine_key: str | None,
    cwd: str | None,
) -> dict:
    parts = [quote(binary), "server", "app", "terminal", "new"]
    if machine_key:
        parts.extend(["--machine-key", quote(machine_key)])
    if cwd:
        parts.extend(["--cwd", quote(cwd)])
    parts.extend(["--timeout-ms", str(timeout_ms)])
    return run_json(host, " ".join(parts))


def app_send_terminal_input(
    host: str,
    binary: str,
    timeout_ms: int,
    session_path: str,
    data: str,
) -> dict:
    command = (
        f"{quote(binary)} server app terminal send {quote(session_path)} "
        f"--data {quote(data)} --timeout-ms {timeout_ms}"
    )
    return run_json(host, command)


def app_remove_session(host: str, binary: str, timeout_ms: int, session_path: str) -> dict:
    command = (
        f"{quote(binary)} server app session remove {quote(session_path)} "
        f"--timeout-ms {timeout_ms}"
    )
    return run_json(host, command)


def server_inventory(host: str) -> dict:
    if host == "local":
        path = Path.home() / ".yggterm" / "server-state.json"
        return json.loads(path.read_text(encoding="utf-8"))
    result = run_control(host, "cat ~/.yggterm/server-state.json")
    return json.loads(result.stdout)


def trace_events_since(host: str, start_ms: int, tail_lines: int = 4000) -> list[dict]:
    if host == "local":
        path = Path.home() / ".yggterm" / "event-trace.jsonl"
        lines = path.read_text(encoding="utf-8").splitlines()[-tail_lines:]
    else:
        result = run_control(host, f"tail -n {tail_lines} ~/.yggterm/event-trace.jsonl")
        lines = result.stdout.splitlines()
    events = []
    for line in lines:
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (event.get("ts_ms") or 0) >= start_ms:
            events.append(event)
    return events


def latest_window_spawn_event_for_pid(host: str, pid: int, start_ms: int) -> dict | None:
    if host == "local":
        path = Path.home() / ".yggterm" / "event-trace.jsonl"
        if not path.exists():
            return None
        for line in reversed(path.read_text(encoding="utf-8").splitlines()):
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if (event.get("ts_ms") or 0) < start_ms:
                break
            if (
                event.get("pid") == pid
                and event.get("category") == "startup"
                and event.get("name") == "window_spawned"
            ):
                return event
        return None
    for event in reversed(trace_events_since(host, start_ms)):
        if (
            event.get("pid") == pid
            and event.get("category") == "startup"
            and event.get("name") == "window_spawned"
        ):
            return event
    return None


def local_client_instance_dir() -> Path:
    return Path.home() / ".yggterm" / "client-instances"


def clear_local_app_control_files() -> None:
    home = Path.home() / ".yggterm"
    for rel in ("app-control-requests", "app-control-responses"):
        root = home / rel
        if not root.is_dir():
            continue
        for path in root.glob("*.json"):
            try:
                path.unlink()
            except FileNotFoundError:
                pass


def kill_local_clients() -> None:
    instances_root = local_client_instance_dir()
    if instances_root.is_dir():
        for path in instances_root.glob("*/*.json"):
            try:
                record = json.loads(path.read_text(encoding="utf-8"))
            except Exception:
                continue
            pid = int(record.get("pid") or 0)
            if pid > 0:
                try:
                    os.kill(pid, 15)
                except Exception:
                    pass
        time.sleep(0.4)
        for path in instances_root.glob("*/*.json"):
            try:
                record = json.loads(path.read_text(encoding="utf-8"))
            except Exception:
                continue
            pid = int(record.get("pid") or 0)
            if pid > 0:
                try:
                    os.kill(pid, 9)
                except Exception:
                    pass
    subprocess.run(
        ["bash", "-lc", "pkill -f 'yggterm server daemon' || true"],
        check=False,
        capture_output=True,
        text=True,
    )
    clear_local_app_control_files()


def launch_local_client(binary: str, timeout_s: float = 3.0) -> tuple[subprocess.Popen, dict]:
    binary_path = str(Path(binary).resolve())
    kill_local_clients()
    env = os.environ.copy()
    env.setdefault("DISPLAY", ":10.0")
    env["YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF"] = "1"
    stdout = open("/tmp/yggterm-terminal-23-launch.out", "w", encoding="utf-8")
    stderr = open("/tmp/yggterm-terminal-23-launch.err", "w", encoding="utf-8")
    start_ms = int(time.time() * 1000)
    proc = subprocess.Popen(
        [binary_path],
        stdout=stdout,
        stderr=stderr,
        env=env,
        cwd=str(Path(binary).resolve().parent.parent.parent),
    )
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        event = latest_window_spawn_event_for_pid("local", proc.pid, start_ms)
        if event is not None:
            return proc, event
        if proc.poll() is not None:
            break
        time.sleep(0.05)
    raise RuntimeError(
        f"local launch did not emit window_spawned within {timeout_s:.2f}s for pid {proc.pid}"
    )


def wait_until(label: str, timeout_s: float, poll_s: float, predicate):
    start = time.monotonic()
    last_error = None
    while time.monotonic() - start <= timeout_s:
        try:
            return time.monotonic() - start, predicate()
        except Exception as error:  # noqa: BLE001
            last_error = error
            remaining = timeout_s - (time.monotonic() - start)
            if remaining <= 0:
                break
            time.sleep(min(poll_s, max(0.0, remaining)))
    try:
        return min(time.monotonic() - start, timeout_s), predicate()
    except Exception as error:  # noqa: BLE001
        last_error = error
    raise RuntimeError(f"{label} timed out after {timeout_s:.2f}s: {last_error}")


def wait_for_window(host: str, binary: str, timeout_ms: int) -> dict:
    def _probe() -> dict:
        state = app_state(host, binary, timeout_ms)
        window = state.get("window") or {}
        if not window.get("visible"):
            raise RuntimeError("window not visible")
        return state

    return wait_until("window visible", 8.0, 0.25, _probe)[1]


def viewport_terminal_ready(state: dict, session_path: str) -> bool:
    viewport = state.get("viewport") or {}
    return (
        viewport.get("active_view_mode") == "Terminal"
        and viewport.get("active_session_path") == session_path
        and bool(viewport.get("ready"))
    )


def terminal_attach_ready_seen(
    host: str,
    session_path: str,
    start_ms: int,
    deadline_ms: int,
) -> bool:
    for event in reversed(trace_events_since(host, start_ms)):
        ts_ms = event.get("ts_ms") or 0
        if ts_ms > deadline_ms:
            continue
        if (
            event.get("category") == "terminal_mount"
            and event.get("name") == "attach_ready"
            and ((event.get("payload") or {}).get("session_path") == session_path)
        ):
            return True
    return False


def active_terminal_text(state: dict) -> str:
    viewport = state.get("viewport") or {}
    hosts = viewport.get("active_terminal_hosts") or []
    if not hosts:
        return ""
    return "\n".join((host.get("text_sample") or "") for host in hosts)


def titlebar_matches_viewport(state: dict) -> bool:
    viewport = state.get("viewport") or {}
    titlebar = viewport.get("titlebar") or {}
    active_title = (viewport.get("active_title") or "").strip()
    active_summary = (viewport.get("active_summary") or "").strip()
    title_text = (titlebar.get("title_text") or "").strip()
    summary_text = (titlebar.get("summary_text") or "").strip()
    button_tooltip = (titlebar.get("button_tooltip") or "").strip()
    if active_title and title_text != active_title:
        return False
    if active_summary and titlebar.get("menu_open") and summary_text != active_summary:
        return False
    if active_summary and not (
        active_summary == button_tooltip
        or active_summary.startswith(button_tooltip)
        or button_tooltip.startswith(active_summary)
    ):
        return False
    return True


def output_matches_cwd(text: str, expected_cwd: str | None) -> bool:
    expected = (expected_cwd or "").strip()
    if not expected:
        return True
    lines = [line.strip() for line in text.splitlines()]
    if expected in lines:
        return True
    squashed_expected = "".join(expected.split())
    squashed_text = "".join(text.split())
    return squashed_expected in squashed_text


def title_is_good(value: str | None) -> bool:
    title = (value or "").strip()
    if not title:
        return False
    lowered = title.lower()
    if lowered in {"resuming terminal...", "resuming terminal…", "connecting..."}:
        return False
    if SHORT_HASH_RE.fullmatch(title):
        return False
    return True


def local_dir_exists(path: str) -> bool:
    target = Path(path)
    return target.is_dir() and os.access(target, os.X_OK)


def remote_dir_exists(ssh_target: str, path: str) -> bool:
    rc, _, _ = run_capture(
        [
            "ssh",
            ssh_target,
            f"cd {quote(path)} >/dev/null 2>&1",
        ]
    )
    return rc == 0


def choose_terminal_targets(inventory: dict, rng: random.Random, count: int) -> list[dict]:
    candidates: list[dict] = []
    seen: set[tuple[str | None, str]] = set()
    machine_targets = {
        (machine.get("machine_key") or "").strip(): (machine.get("ssh_target") or "").strip()
        for machine in (inventory.get("remote_machines") or [])
    }

    for session in inventory.get("stored_sessions") or []:
        cwd = (session.get("cwd") or "").strip()
        path = (session.get("path") or "").strip()
        if not cwd or path.startswith("remote-session://"):
            continue
        key = (None, cwd)
        if key in seen:
            continue
        seen.add(key)
        candidates.append({"machine_key": None, "cwd": cwd, "label": f"local:{cwd}"})

    for machine in inventory.get("remote_machines") or []:
        machine_key = (machine.get("machine_key") or "").strip()
        if not machine_key:
            continue
        ssh_target = machine_targets.get(machine_key) or ""
        if not ssh_target:
            continue
        for session in machine.get("sessions") or []:
            cwd = (session.get("cwd") or "").strip()
            if not cwd:
                continue
            key = (machine_key, cwd)
            if key in seen:
                continue
            seen.add(key)
            candidates.append(
                {
                    "machine_key": machine_key,
                    "cwd": cwd,
                    "label": f"{machine_key}:{cwd}",
                }
            )

    if not candidates:
        raise RuntimeError("no machine/cwd targets available for terminal-23 test")
    rng.shuffle(candidates)
    dir_exists_cache: dict[tuple[str | None, str], bool] = {}
    chosen: list[dict] = []
    for candidate in candidates:
        machine_key = candidate.get("machine_key")
        cwd = candidate.get("cwd") or ""
        key = (machine_key, cwd)
        if machine_key:
            ssh_target = machine_targets.get(machine_key) or ""
            exists = dir_exists_cache.setdefault(key, bool(ssh_target) and remote_dir_exists(ssh_target, cwd))
        else:
            exists = dir_exists_cache.setdefault(key, local_dir_exists(cwd))
        if not exists:
            continue
        chosen.append(candidate)
        if len(chosen) == count:
            return chosen
    if not chosen:
        raise RuntimeError("no existing local/remote cwd targets available for terminal-23 test")
    while len(chosen) < count:
        chosen.append(rng.choice(chosen))
    return chosen


def write_json(path: Path, payload: dict) -> str:
    path.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    return str(path)


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    launch = None
    launch_event = None
    if args.host == "local" and args.launch_local:
        launch, launch_event = launch_local_client(args.bin)

    baseline_state = wait_for_window(args.host, args.bin, args.timeout_ms)
    baseline_notifications = ((baseline_state.get("shell") or {}).get("notifications_count")) or 0
    inventory = server_inventory(args.host)
    targets = choose_terminal_targets(inventory, random.Random(args.seed), args.count)
    results: list[dict] = []

    for index, target in enumerate(targets):
        started_ms = int(time.time() * 1000)
        try:
            create = app_create_terminal(
                args.host,
                args.bin,
                args.timeout_ms,
                target.get("machine_key"),
                target.get("cwd"),
            )
        except Exception as error:  # noqa: BLE001
            entry = {
                "target": target,
                "error": str(error),
            }
            results.append(entry)
            continue
        created_path = ((create.get("data") or {}).get("active_session_path")) or None
        entry = {
            "target": target,
            "create": create,
            "created_session_path": created_path,
        }
        if not created_path:
            entry["error"] = "create terminal did not return active_session_path"
            results.append(entry)
            continue

        try:
            ready_started_ms = int(time.time() * 1000)
            ready_deadline_ms = ready_started_ms + int(args.ready_budget * 1000)
            ready_elapsed, ready_state = wait_until(
                f"terminal ready {created_path}",
                args.ready_budget,
                args.poll,
                lambda: _require_terminal_ready(
                    app_state(args.host, args.bin, args.timeout_ms),
                    created_path,
                    args.host,
                    ready_started_ms,
                    ready_deadline_ms,
                ),
            )
            entry["ready_elapsed_s"] = round(ready_elapsed, 3)
            entry["ready_within_budget"] = ready_elapsed <= args.ready_budget
            entry["attach_ready_before_deadline"] = terminal_attach_ready_seen(
                args.host,
                created_path,
                ready_started_ms,
                ready_deadline_ms,
            )
        except Exception as error:  # noqa: BLE001
            state = app_state(args.host, args.bin, args.timeout_ms)
            entry["error"] = str(error)
            entry["attach_ready_before_deadline"] = terminal_attach_ready_seen(
                args.host,
                created_path,
                ready_started_ms,
                ready_deadline_ms,
            )
            entry["state_dump"] = write_json(out_dir / f"terminal-{index:02d}-ready-failure.json", state)
            results.append(entry)
            try:
                entry["remove"] = app_remove_session(args.host, args.bin, args.timeout_ms, created_path)
            except Exception as remove_error:  # noqa: BLE001
                entry["remove_error"] = str(remove_error)
            continue

        probe_command = (
            f"printf '{MARKER_BEGIN}\\n'; "
            "uname -a || true; "
            "pwd; "
            "(free -h || vm_stat || sysctl vm.swapusage || true); "
            "(ls -1A | head -12) || true; "
            f"printf '{MARKER_END}\\n'"
        )
        send = app_send_terminal_input(
            args.host,
            args.bin,
            args.timeout_ms,
            created_path,
            probe_command + "\n",
        )
        entry["send"] = send

        try:
            output_elapsed, output_state = wait_until(
                f"terminal output {created_path}",
                args.summary_budget,
                args.poll,
                lambda: _require_terminal_markers(
                    app_state(args.host, args.bin, args.timeout_ms),
                    created_path,
                ),
            )
            entry["output_elapsed_s"] = round(output_elapsed, 3)
            entry["output_contains_markers"] = True
        except Exception as error:  # noqa: BLE001
            output_state = app_state(args.host, args.bin, args.timeout_ms)
            entry["error"] = str(error)
            entry["state_dump"] = write_json(out_dir / f"terminal-{index:02d}-output-failure.json", output_state)
            results.append(entry)
            try:
                entry["remove"] = app_remove_session(args.host, args.bin, args.timeout_ms, created_path)
            except Exception as remove_error:  # noqa: BLE001
                entry["remove_error"] = str(remove_error)
            continue

        viewport = output_state.get("viewport") or {}
        titlebar = viewport.get("titlebar") or {}
        notifications = (output_state.get("shell") or {}).get("notifications_count") or 0
        entry["active_title"] = viewport.get("active_title")
        entry["active_summary"] = viewport.get("active_summary")
        entry["titlebar_title_text"] = titlebar.get("title_text")
        entry["titlebar_summary_text"] = titlebar.get("summary_text")
        entry["titlebar_button_tooltip"] = titlebar.get("button_tooltip")
        entry["titlebar_menu_open"] = titlebar.get("menu_open")
        entry["notification_count"] = notifications
        entry["notification_delta"] = max(0, notifications - baseline_notifications)
        entry["terminal_text_sample"] = active_terminal_text(output_state)
        entry["state_dump"] = write_json(out_dir / f"terminal-{index:02d}.json", output_state)
        entry["title_present"] = title_is_good(viewport.get("active_title"))
        entry["summary_present"] = bool((viewport.get("active_summary") or "").strip())
        entry["titlebar_matches_viewport"] = titlebar_matches_viewport(output_state)
        entry["cwd_matches"] = output_matches_cwd(
            entry["terminal_text_sample"],
            target.get("cwd"),
        )
        entry["notification_noise"] = notifications > baseline_notifications

        try:
            remove = app_remove_session(args.host, args.bin, args.timeout_ms, created_path)
            entry["remove"] = remove
        except Exception as error:  # noqa: BLE001
            entry["remove_error"] = str(error)
        results.append(entry)

    final_state = app_state(args.host, args.bin, args.timeout_ms)
    summary = {
        "host": args.host,
        "count": args.count,
        "seed": args.seed,
        "spawn_budget_s": args.spawn_budget,
        "ready_budget_s": args.ready_budget,
        "summary_budget_s": args.summary_budget,
        "window_spawn_elapsed_ms": ((launch_event or {}).get("payload") or {}).get("elapsed_ms"),
        "window_spawn_within_900ms": (
            (((launch_event or {}).get("payload") or {}).get("elapsed_ms") or 10_000) <= 900
        ),
        "baseline_notifications_count": baseline_notifications,
        "final_notifications_count": ((final_state.get("shell") or {}).get("notifications_count")) or 0,
        "terminals_created": len(results),
        "creation_failures": len([item for item in results if item.get("created_session_path") is None]),
        "ready_failures": len([item for item in results if item.get("created_session_path") and not item.get("ready_within_budget")]),
        "command_failures": len([item for item in results if item.get("created_session_path") and not item.get("output_contains_markers")]),
        "summary_failures": len([item for item in results if item.get("created_session_path") and not item.get("summary_present")]),
        "title_failures": len([item for item in results if item.get("created_session_path") and not item.get("title_present")]),
        "titlebar_failures": len([item for item in results if item.get("created_session_path") and not item.get("titlebar_matches_viewport")]),
        "cwd_failures": len([item for item in results if item.get("created_session_path") and not item.get("cwd_matches", False)]),
        "notification_anomalies": len([item for item in results if item.get("notification_noise")]),
        "remove_failures": len([item for item in results if item.get("remove_error")]),
        "results": results,
    }
    summary_path = out_dir / "summary.json"
    summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(summary_path)
    print(json.dumps(summary, indent=2))

    if launch is not None and launch.poll() is None:
        launch.terminate()
        try:
            launch.wait(timeout=2)
        except subprocess.TimeoutExpired:
            launch.kill()

    return 0 if all(
        item.get("ready_within_budget")
        and item.get("output_contains_markers")
        and item.get("cwd_matches")
        and item.get("title_present")
        and item.get("summary_present")
        and item.get("titlebar_matches_viewport")
        and not item.get("notification_noise")
        and not item.get("remove_error")
        for item in results
    ) else 1


def _require_terminal_ready(
    state: dict,
    session_path: str,
    host: str,
    started_ms: int,
    deadline_ms: int,
) -> dict:
    if not viewport_terminal_ready(state, session_path):
        viewport = state.get("viewport") or {}
        if (
            (viewport.get("active_view_mode") == "Terminal")
            and (viewport.get("active_session_path") == session_path)
            and titlebar_matches_viewport(state)
            and terminal_attach_ready_seen(host, session_path, started_ms, deadline_ms)
        ):
            return state
        viewport = state.get("viewport") or {}
        raise RuntimeError(viewport.get("reason") or "terminal viewport not ready")
    if not titlebar_matches_viewport(state):
        raise RuntimeError("titlebar not in sync with viewport")
    return state


def _require_terminal_markers(state: dict, session_path: str) -> dict:
    if not viewport_terminal_ready(state, session_path):
        viewport = state.get("viewport") or {}
        raise RuntimeError(viewport.get("reason") or "terminal viewport not ready")
    if not titlebar_matches_viewport(state):
        raise RuntimeError("titlebar not in sync with viewport")
    text = active_terminal_text(state)
    if MARKER_BEGIN not in text or MARKER_END not in text:
        raise RuntimeError("terminal output markers not visible yet")
    return state


if __name__ == "__main__":
    raise SystemExit(main())
