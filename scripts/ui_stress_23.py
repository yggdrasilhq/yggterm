#!/usr/bin/env python3
import argparse
import json
import os
import random
import shlex
import subprocess
import time
from pathlib import Path
from typing import Callable


class ReadinessPending(RuntimeError):
    def __init__(self, message: str, state: dict):
        super().__init__(message)
        self.state = state


class ReadinessTimeout(RuntimeError):
    def __init__(self, label: str, timeout_s: float, last_error: Exception | None, state: dict):
        super().__init__(f"{label} timed out after {timeout_s:.2f}s: {last_error}")
        self.state = state


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Stress a running Yggterm GUI through app-control by opening random sessions "
            "and optionally exercising drag/drop."
        )
    )
    parser.add_argument("--host", default="local", help="SSH host for a running GUI, or 'local'")
    parser.add_argument("--bin", default="./target/debug/yggterm")
    parser.add_argument("--count", type=int, default=23, help="Number of random session opens")
    parser.add_argument(
        "--drag-count",
        type=int,
        default=23,
        help="Number of random drag operations to attempt when --apply-drag is set",
    )
    parser.add_argument(
        "--ready-budget",
        type=float,
        default=2.3,
        help="Maximum seconds allowed for a session open to become usable",
    )
    parser.add_argument("--poll", type=float, default=0.1)
    parser.add_argument("--timeout-ms", type=int, default=8000)
    parser.add_argument("--seed", type=int, default=23)
    parser.add_argument("--out-dir", default="/tmp/yggterm-ui-stress-23")
    parser.add_argument(
        "--launch-local",
        action="store_true",
        help="Launch a fresh local GUI client from --bin before running the test",
    )
    parser.add_argument(
        "--apply-drag",
        action="store_true",
        help="Actually perform drag/drop operations instead of only planning them",
    )
    parser.add_argument(
        "--screenshot-on-failure",
        action="store_true",
        help="Capture an app screenshot when an open or drag test fails",
    )
    return parser.parse_args()


def run_process(argv: list[str], *, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(argv, check=check, text=True, capture_output=True)


def run_control(host: str, command: str, *, check: bool = True) -> subprocess.CompletedProcess:
    if host == "local":
        return run_process(["bash", "-lc", command], check=check)
    return run_process(["ssh", host, command], check=check)


def run_json(host: str, command: str) -> dict:
    result = run_control(host, command)
    return json.loads(result.stdout)


def app_state(host: str, binary: str, timeout_ms: int) -> dict:
    payload = run_json(host, f"{binary} server app state --timeout-ms {timeout_ms}")
    return payload.get("data") or {}


def app_rows(host: str, binary: str, timeout_ms: int) -> dict:
    payload = run_json(host, f"{binary} server app rows --timeout-ms {timeout_ms}")
    return payload.get("data") or {}


def canonical_session_path(session_path: str | None) -> str | None:
    if session_path is None:
        return None
    if session_path.startswith("local::"):
        return f"local://{session_path[len('local::'):]}"
    return session_path


def write_state_dump(out_dir: Path, name: str, state: dict) -> str:
    path = out_dir / f"{name}.state.json"
    path.write_text(json.dumps(state, indent=2), encoding="utf-8")
    return str(path)


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


def ready_event_seen(host: str, start_ms: int, session_path: str, view: str) -> bool:
    events = trace_events_since(host, start_ms)
    for event in events:
        payload = event.get("payload") or {}
        if view == "preview":
            if (
                event.get("category") == "main_surface"
                and event.get("name") == "viewport_ready"
                and payload.get("active_session_path") == session_path
                and payload.get("active_view_mode") == "Rendered"
            ):
                return True
            continue
        if (
            event.get("category") == "terminal_mount"
            and event.get("name") in {"attach_ready", "first_meaningful_output"}
            and payload.get("session_path") == session_path
        ):
            return True
    return False


def latest_window_spawn_event(host: str, tail_lines: int = 4000) -> dict | None:
    events = trace_events_since(host, 0, tail_lines=tail_lines)
    for event in reversed(events):
        if event.get("category") == "startup" and event.get("name") == "window_spawned":
            return event
    return None


def latest_window_spawn_event_for_pid(
    host: str,
    pid: int,
    start_ms: int,
    tail_lines: int = 4000,
) -> dict | None:
    events = trace_events_since(host, start_ms, tail_lines=tail_lines)
    for event in reversed(events):
        if (
            event.get("pid") == pid
            and event.get("category") == "startup"
            and event.get("name") == "window_spawned"
        ):
            return event
    return None


def local_client_instance_dir() -> Path:
    return Path.home() / ".yggterm" / "client-instances"


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
                except ProcessLookupError:
                    pass
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
                except ProcessLookupError:
                    pass
                except Exception:
                    pass
    subprocess.run(
        ["bash", "-lc", "pkill -f 'yggterm server daemon' || true"],
        check=False,
        capture_output=True,
        text=True,
    )


def launch_local_client(binary: str, timeout_s: float = 3.0) -> tuple[subprocess.Popen, dict]:
    binary_path = str(Path(binary).resolve())
    kill_local_clients()
    subprocess.run(
        ["bash", "-lc", f"pkill -f {shlex.quote(binary_path)} || true"],
        check=False,
        capture_output=True,
        text=True,
    )
    env = os.environ.copy()
    env.setdefault("DISPLAY", ":10.0")
    env["YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF"] = "1"
    stdout_path = "/tmp/yggterm-ui-stress-launch.out"
    stderr_path = "/tmp/yggterm-ui-stress-launch.err"
    stdout = open(stdout_path, "w", encoding="utf-8")
    stderr = open(stderr_path, "w", encoding="utf-8")
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


def app_open(host: str, binary: str, session_path: str, view: str, timeout_ms: int) -> dict:
    return run_json(
        host,
        f"{binary} server app open {json.dumps(session_path)} --view {view} --timeout-ms {timeout_ms}",
    )


def app_expand(host: str, binary: str, row_path: str, expanded: bool, timeout_ms: int) -> dict:
    action = "expand" if expanded else "collapse"
    return run_json(
        host,
        f"{binary} server app {action} {json.dumps(row_path)} --timeout-ms {timeout_ms}",
    )


def app_drag(
    host: str,
    binary: str,
    action: str,
    timeout_ms: int,
    row_path: str | None = None,
    placement: str | None = None,
) -> dict:
    parts = [f"{binary} server app drag {action}"]
    if row_path is not None:
        parts.append(json.dumps(row_path))
    if placement is not None:
        parts.append(f"--placement {placement}")
    parts.append(f"--timeout-ms {timeout_ms}")
    return run_json(host, " ".join(parts))


def capture_failure_artifact(
    host: str,
    binary: str,
    timeout_ms: int,
    out_dir: Path,
    name: str,
) -> str | None:
    remote_path = f"/tmp/{name}.png"
    local_path = out_dir / f"{name}.png"
    try:
        run_control(
            host,
            f"{binary} server app screenshot {json.dumps(remote_path)} --timeout-ms {timeout_ms}",
        )
        if host == "local":
            run_process(["cp", remote_path, str(local_path)])
        else:
            run_process(["scp", f"{host}:{remote_path}", str(local_path)])
        return str(local_path)
    except Exception:
        return None


def preview_ready(state: dict, session_path: str) -> bool:
    viewport = state.get("viewport") or {}
    if viewport.get("active_view_mode") != "Rendered":
        return False
    if viewport.get("active_session_path") != session_path:
        return False
    return bool(viewport.get("ready"))


def terminal_ready(state: dict, session_path: str) -> bool:
    viewport = state.get("viewport") or {}
    if viewport.get("active_view_mode") != "Terminal":
        return False
    if viewport.get("active_session_path") != session_path:
        return False
    return bool(viewport.get("ready"))


def viewport_failure_reason(state: dict) -> str:
    viewport = state.get("viewport") or {}
    reason = viewport.get("reason")
    if isinstance(reason, str) and reason:
        return reason
    return "main viewport not ready"


def viewport_matches_target(state: dict, session_path: str, view: str) -> bool:
    viewport = state.get("viewport") or {}
    expected_mode = "Rendered" if view == "preview" else "Terminal"
    return (
        viewport.get("active_session_path") == session_path
        and viewport.get("active_view_mode") == expected_mode
    )


def wait_until(
    label: str,
    timeout_s: float,
    poll_s: float,
    predicate: Callable[[], dict],
) -> tuple[float, dict]:
    start = time.monotonic()
    last_error = None
    last_state: dict = {}
    while time.monotonic() - start <= timeout_s:
        try:
            state = predicate()
            return time.monotonic() - start, state
        except Exception as error:  # noqa: BLE001
            last_error = error
            if isinstance(error, ReadinessPending):
                last_state = error.state
            time.sleep(poll_s)
    raise ReadinessTimeout(label, timeout_s, last_error, last_state)


def wait_for_app_state(
    host: str,
    binary: str,
    timeout_ms: int,
    timeout_s: float = 8.0,
    poll_s: float = 0.25,
) -> dict:
    def _probe() -> dict:
        state = app_state(host, binary, timeout_ms)
        window = state.get("window") or {}
        if not window.get("visible"):
            raise RuntimeError("window not visible yet")
        return state

    _, state = wait_until("app state", timeout_s, poll_s, _probe)
    return state


def choose_session_targets(
    inventory: dict,
    rows_payload: dict,
    rng: random.Random,
    count: int,
) -> list[dict]:
    del inventory
    rows = rows_payload.get("rows") or []
    candidates: list[dict] = []
    seen_paths: set[str] = set()

    for row in rows:
        row_kind = row.get("kind")
        session_path = canonical_session_path(row.get("full_path"))
        if row_kind not in {"Document", "Session"} or not session_path or session_path in seen_paths:
            continue
        if row_kind == "Document":
            view = "preview"
        else:
            view = "terminal"
        candidates.append(
            {
                "full_path": session_path,
                "label": row.get("label") or session_path,
                "kind": row_kind,
                "view": view,
            }
        )
        seen_paths.add(session_path)

    if len(candidates) <= count:
        return candidates
    return rng.sample(candidates, count)


def choose_drag_rows(rows_payload: dict, rng: random.Random, count: int) -> list[tuple[dict, dict, str]]:
    rows = rows_payload.get("rows") or []
    def is_workspace_path(path: str | None) -> bool:
        return bool(path) and path.startswith("/")

    draggable = [
        row for row in rows
        if row.get("draggable") and is_workspace_path(row.get("full_path"))
    ]
    drop_targets = [
        row for row in rows
        if row.get("drop_target_row") and is_workspace_path(row.get("full_path"))
    ]
    operations: list[tuple[dict, dict, str]] = []
    if not draggable or not drop_targets:
        return operations
    placements = ["before", "after", "into"]
    attempts = 0
    while len(operations) < count and attempts < count * 20:
        attempts += 1
        source = rng.choice(draggable)
        target = rng.choice(drop_targets)
        if source["full_path"] == target["full_path"]:
            continue
        placement = rng.choice(placements)
        if placement == "into" and target.get("kind") not in {"Group"}:
            placement = rng.choice(["before", "after"])
        operations.append((source, target, placement))
    return operations


def expand_all_groups(host: str, binary: str, timeout_ms: int, max_passes: int = 8) -> dict:
    last = app_rows(host, binary, timeout_ms)
    for _ in range(max_passes):
        rows = last.get("rows") or []
        collapsed = [
            row for row in rows
            if row.get("kind") == "Group" and not row.get("expanded") and row.get("full_path")
        ]
        if not collapsed:
            return last
        for row in collapsed:
            app_expand(host, binary, row["full_path"], True, timeout_ms)
        time.sleep(0.15)
        last = app_rows(host, binary, timeout_ms)
    return last


def wait_for_openable_rows(
    args: argparse.Namespace,
    inventory: dict,
) -> dict:
    target_count = min(
        args.count,
        (len(inventory.get("stored_sessions") or []) + len(inventory.get("live_sessions") or [])),
    )

    def _probe() -> dict:
        rows_payload = expand_all_groups(args.host, args.bin, args.timeout_ms)
        openable = choose_session_targets(inventory, rows_payload, random.Random(args.seed), args.count)
        if len(openable) < target_count:
            raise ReadinessPending(
                f"only {len(openable)} openable rows visible",
                {"rows_payload": rows_payload, "openable_count": len(openable)},
            )
        return rows_payload

    _, rows_payload = wait_until("openable rows", 12.0, 0.25, _probe)
    return rows_payload


def open_trials(
    args: argparse.Namespace,
    inventory: dict,
    rows_payload: dict,
    out_dir: Path,
    baseline_notifications_count: int,
) -> list[dict]:
    rng = random.Random(args.seed)
    chosen_rows = choose_session_targets(inventory, rows_payload, rng, args.count)
    results: list[dict] = []
    for index, row in enumerate(chosen_rows):
        kind = row.get("kind")
        view = row.get("view") or ("preview" if kind == "Document" else "terminal")
        session_path = row["full_path"]
        ready_pred = preview_ready if view == "preview" else terminal_ready
        started = time.monotonic()
        started_ms = int(time.time() * 1000)
        app_open(args.host, args.bin, session_path, view, args.timeout_ms)
        try:
            elapsed, state = wait_until(
                f"open {session_path}",
                args.ready_budget,
                args.poll,
                lambda: _require_ready_event_or_state(
                    args.host,
                    started_ms,
                    session_path,
                    view,
                    app_state(args.host, args.bin, args.timeout_ms),
                ),
            )
            state_dump = write_state_dump(out_dir, f"open-{index:02d}", state)
            anomaly = detect_anomaly(state, baseline_notifications_count)
            results.append(
                {
                    "path": session_path,
                    "label": row.get("label"),
                    "kind": kind,
                    "view": view,
                    "elapsed_s": round(elapsed, 3),
                    "within_budget": elapsed <= args.ready_budget,
                    "anomaly": anomaly,
                    "notifications_count": ((state.get("shell") or {}).get("notifications_count")),
                    "active_surface_requests": len(state.get("active_surface_requests") or []),
                    "machine_refresh_requests": ((state.get("remote") or {}).get("machine_refresh_requests")),
                    "viewport_ready": ready_pred(state, session_path),
                    "viewport_reported_ready": ((state.get("viewport") or {}).get("ready")),
                    "viewport_target_matched": viewport_matches_target(state, session_path, view),
                    "viewport_reason": ((state.get("viewport") or {}).get("reason")),
                    "state_dump": state_dump,
                }
            )
        except Exception as error:  # noqa: BLE001
            failure_artifact = None
            state = getattr(error, "state", {}) or {}
            if args.screenshot_on_failure:
                failure_artifact = capture_failure_artifact(
                    args.host,
                    args.bin,
                    args.timeout_ms,
                    out_dir,
                    f"open-failure-{index:02d}",
                )
            if not state:
                try:
                    state = app_state(args.host, args.bin, args.timeout_ms)
                except Exception:
                    state = {}
            state_dump = write_state_dump(out_dir, f"open-{index:02d}-failure", state)
            results.append(
                {
                    "path": session_path,
                    "label": row.get("label"),
                    "kind": kind,
                    "view": view,
                    "elapsed_s": round(time.monotonic() - started, 3),
                    "within_budget": False,
                    "error": str(error),
                    "notifications_count": ((state.get("shell") or {}).get("notifications_count")),
                    "active_surface_requests": len(state.get("active_surface_requests") or []),
                    "machine_refresh_requests": ((state.get("remote") or {}).get("machine_refresh_requests")),
                    "viewport_ready": ready_pred(state, session_path),
                    "viewport_reported_ready": ((state.get("viewport") or {}).get("ready")),
                    "viewport_target_matched": viewport_matches_target(state, session_path, view),
                    "viewport_reason": ((state.get("viewport") or {}).get("reason")),
                    "state_dump": state_dump,
                    "failure_artifact": failure_artifact,
                }
            )
    return results


def drag_trials(
    args: argparse.Namespace,
    rows_payload: dict,
    out_dir: Path,
    baseline_notifications_count: int,
) -> list[dict]:
    rng = random.Random(args.seed + 2300)
    results: list[dict] = []
    if not args.apply_drag:
        operations = choose_drag_rows(rows_payload, rng, args.drag_count)
        for source, target, placement in operations:
            results.append(
                {
                    "source": source["full_path"],
                    "target": target["full_path"],
                    "placement": placement,
                    "planned_only": True,
                }
            )
        return results
    for index in range(args.drag_count):
        current_rows_payload = expand_all_groups(args.host, args.bin, args.timeout_ms)
        operations = choose_drag_rows(current_rows_payload, rng, 1)
        if not operations:
            break
        source, target, placement = operations[0]
        failure_artifact = None
        try:
            begin_response = app_drag(
                args.host,
                args.bin,
                "begin",
                args.timeout_ms,
                row_path=source["full_path"],
            )
            hover_response = app_drag(
                args.host,
                args.bin,
                "hover",
                args.timeout_ms,
                row_path=target["full_path"],
                placement=placement,
            )
            drop_response = app_drag(args.host, args.bin, "drop", args.timeout_ms)
            state = app_state(args.host, args.bin, args.timeout_ms)
            state_dump = write_state_dump(out_dir, f"drag-{index:02d}", state)
            drag_state = state.get("shell") or {}
            begin_data = begin_response.get("data") or {}
            hover_data = hover_response.get("data") or {}
            drop_data = drop_response.get("data") or {}
            anomaly = None
            if not begin_data.get("accepted"):
                anomaly = "drag begin not accepted"
            elif not hover_data.get("accepted"):
                anomaly = "drag hover target missing during drag cycle"
            elif not drop_data.get("accepted"):
                anomaly = "drag drop not accepted"
            if drag_state.get("drag_paths") or drag_state.get("drag_hover_target"):
                anomaly = "drag state not cleared after drop"
            new_notifications = max(
                0,
                (drag_state.get("notifications_count") or 0) - baseline_notifications_count,
            )
            if anomaly is None and new_notifications > 0:
                tones = [
                    notification.get("tone")
                    for notification in (drag_state.get("notifications") or [])[-new_notifications:]
                ]
                if any(tone != "Success" for tone in tones):
                    anomaly = f"drag emitted non-success notifications ({new_notifications})"
            results.append(
                {
                    "source": source["full_path"],
                    "target": target["full_path"],
                    "placement": placement,
                    "planned_only": False,
                    "anomaly": anomaly,
                    "notifications_count": drag_state.get("notifications_count"),
                    "begin_response": begin_response,
                    "hover_response": hover_response,
                    "drop_response": drop_response,
                    "state_dump": state_dump,
                }
            )
        except Exception as error:  # noqa: BLE001
            try:
                app_drag(args.host, args.bin, "clear", args.timeout_ms)
            except Exception:
                pass
            if args.screenshot_on_failure:
                failure_artifact = capture_failure_artifact(
                    args.host,
                    args.bin,
                    args.timeout_ms,
                    out_dir,
                    f"drag-failure-{index:02d}",
                )
            results.append(
                {
                    "source": source["full_path"],
                    "target": target["full_path"],
                    "placement": placement,
                    "planned_only": False,
                    "error": str(error),
                    "failure_artifact": failure_artifact,
                }
            )
    return results


def detect_anomaly(state: dict, baseline_notifications_count: int) -> str | None:
    shell = state.get("shell") or {}
    remote = state.get("remote") or {}
    notifications = shell.get("notifications_count") or 0
    machine_refresh = remote.get("machine_refresh_requests") or 0
    requests = len(state.get("active_surface_requests") or [])
    if notifications > baseline_notifications_count:
        return f"notification cascade (+{notifications - baseline_notifications_count})"
    if machine_refresh > 0:
        return f"background machine refresh active ({machine_refresh})"
    if requests > 1:
        return f"surface request pileup ({requests})"
    return None


def _require_ready(state: dict, ready_pred: Callable[[dict, str], bool], session_path: str) -> dict:
    if not ready_pred(state, session_path):
        raise ReadinessPending("not ready yet", state)
    return state


def _require_ready_event_or_state(
    host: str,
    started_ms: int,
    session_path: str,
    view: str,
    state: dict,
) -> dict:
    ready_pred = preview_ready if view == "preview" else terminal_ready
    if not ready_pred(state, session_path):
        raise ReadinessPending(viewport_failure_reason(state), state)
    if ready_event_seen(host, started_ms, session_path, view):
        return state
    return state


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    launch = None
    launch_event = None
    if args.host == "local" and args.launch_local:
        launch, launch_event = launch_local_client(args.bin)

    baseline_state = wait_for_app_state(args.host, args.bin, args.timeout_ms)
    baseline_dump = write_state_dump(out_dir, "baseline", baseline_state)
    baseline_notifications_count = ((baseline_state.get("shell") or {}).get("notifications_count")) or 0
    inventory = server_inventory(args.host)
    rows_payload = wait_for_openable_rows(args, inventory)
    opens = open_trials(args, inventory, rows_payload, out_dir, baseline_notifications_count)
    drag_rows_payload = expand_all_groups(args.host, args.bin, args.timeout_ms)
    drags = drag_trials(args, drag_rows_payload, out_dir, baseline_notifications_count)
    final_state = wait_for_app_state(args.host, args.bin, args.timeout_ms)
    final_dump = write_state_dump(out_dir, "final", final_state)
    window_spawn_event = launch_event or latest_window_spawn_event(args.host)

    open_failures = [item for item in opens if not item.get("within_budget")]
    open_anomalies = [item for item in opens if item.get("anomaly")]
    drag_failures = [item for item in drags if item.get("error")]
    drag_anomalies = [item for item in drags if item.get("anomaly")]
    drag_shortfall = max(0, args.drag_count - len(drags))

    summary = {
        "host": args.host,
        "seed": args.seed,
        "count": args.count,
        "drag_count": args.drag_count,
        "ready_budget_s": args.ready_budget,
        "apply_drag": args.apply_drag,
        "baseline_active_session_path": baseline_state.get("active_session_path"),
        "baseline_notifications_count": ((baseline_state.get("shell") or {}).get("notifications_count")),
        "baseline_dump": baseline_dump,
        "window_spawn_elapsed_ms": ((window_spawn_event or {}).get("payload") or {}).get("elapsed_ms"),
        "window_spawn_within_900ms": (
            (((window_spawn_event or {}).get("payload") or {}).get("elapsed_ms") or 10_000) <= 900
        ),
        "inventory_stored_sessions": len(inventory.get("stored_sessions") or []),
        "inventory_live_sessions": len(inventory.get("live_sessions") or []),
        "rows_seen": rows_payload.get("row_count"),
        "open_failures": len(open_failures),
        "open_anomalies": len(open_anomalies),
        "drag_operations_executed": len(drags),
        "drag_shortfall": drag_shortfall,
        "drag_failures": len(drag_failures),
        "drag_anomalies": len(drag_anomalies),
        "final_active_session_path": final_state.get("active_session_path"),
        "final_notifications_count": ((final_state.get("shell") or {}).get("notifications_count")),
        "final_dump": final_dump,
        "opens": opens,
        "drags": drags,
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
    return 0 if not open_failures and not drag_failures and drag_shortfall == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
