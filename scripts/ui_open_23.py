#!/usr/bin/env python3
import argparse
import json
import os
import random
import shlex
import shutil
import subprocess
import time
from pathlib import Path


SHORT_HASH_RE = __import__("re").compile(r"^[0-9a-f]{7,8}$")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Open 23 existing sessions/documents through app-control, mix preview and terminal "
            "views, and verify real viewport/title/summary readiness."
        )
    )
    parser.add_argument("--host", default="local")
    parser.add_argument("--bin", default="./target/debug/yggterm")
    parser.add_argument("--count", type=int, default=23)
    parser.add_argument("--seed", type=int, default=23)
    parser.add_argument("--timeout-ms", type=int, default=8000)
    parser.add_argument("--poll", type=float, default=0.15)
    parser.add_argument("--ready-budget", type=float, default=2.3)
    parser.add_argument("--launch-local", action="store_true")
    parser.add_argument("--out-dir", default="/tmp/yggterm-open-23")
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
    if result.returncode != 0 or not stdout:
        raise RuntimeError(
            f"command failed rc={result.returncode}: {command}\nstdout:\n{stdout or '<empty>'}\nstderr:\n{stderr or '<empty>'}"
        )
    try:
        return json.loads(stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(
            f"invalid json rc={result.returncode}: {command}\nstdout:\n{stdout or '<empty>'}\nstderr:\n{stderr or '<empty>'}"
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


def app_state(host: str, binary: str, timeout_ms: int) -> dict:
    payload = run_json(host, f"{shlex.quote(binary)} server app state --timeout-ms {timeout_ms}")
    return payload.get("data") or {}


def app_rows(host: str, binary: str, timeout_ms: int) -> dict:
    payload = run_json(host, f"{shlex.quote(binary)} server app rows --timeout-ms {timeout_ms}")
    return payload.get("data") or {}


def app_open(host: str, binary: str, session_path: str, view: str, timeout_ms: int) -> dict:
    command = (
        f"{shlex.quote(binary)} server app open {json.dumps(session_path)} "
        f"--view {view} --timeout-ms {timeout_ms}"
    )
    return run_json(host, command)


def app_expand(host: str, binary: str, row_path: str, expanded: bool, timeout_ms: int) -> dict:
    action = "expand" if expanded else "collapse"
    command = f"{shlex.quote(binary)} server app {action} {json.dumps(row_path)} --timeout-ms {timeout_ms}"
    return run_json(host, command)


def canonical_session_path(session_path: str | None) -> str | None:
    if session_path is None:
        return None
    if session_path.startswith("local::"):
        return f"local://{session_path[len('local::'):]}"
    return session_path


def latest_window_spawn_event_for_pid(host: str, pid: int, start_ms: int) -> dict | None:
    if host != "local":
        return None
    path = local_yggterm_path("event-trace.jsonl")
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


def kill_local_clients(binary: str) -> None:
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
                    os.kill(pid, 15)
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
                    os.kill(pid, 9)
                except Exception:
                    pass
    run_process(["bash", "-lc", "pkill -f 'yggterm server daemon' || true"], check=False)


def launch_local_client(binary: str, timeout_s: float = 4.0) -> tuple[subprocess.Popen, dict]:
    binary_path = str(Path(binary).resolve())
    kill_local_clients(binary_path)
    env = os.environ.copy()
    env.setdefault("DISPLAY", ":10.0")
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
        shell = state.get("shell") or {}
        if not window.get("visible"):
            raise RuntimeError("window not visible")
        if shell.get("needs_initial_server_sync"):
            raise RuntimeError("initial server sync still in progress")
        if shell.get("server_busy"):
            raise RuntimeError("server still busy")
        return state

    return wait_until("window visible", 8.0, 0.25, _probe)[1]


def viewport_matches_target(state: dict, session_path: str, view: str) -> bool:
    viewport = state.get("viewport") or {}
    expected_mode = "Rendered" if view == "preview" else "Terminal"
    return (
        viewport.get("active_session_path") == session_path
        and viewport.get("active_view_mode") == expected_mode
    )


def viewport_ready(state: dict, session_path: str, view: str) -> bool:
    viewport = state.get("viewport") or {}
    return bool(viewport.get("ready")) and viewport_matches_target(state, session_path, view)


def trace_events_since(host: str, start_ms: int, tail_lines: int = 6000) -> list[dict]:
    if host == "local":
        path = local_yggterm_path("event-trace.jsonl")
        if not path.exists():
            return []
        lines = path.read_text(encoding="utf-8").splitlines()[-tail_lines:]
    else:
        result = run_control(host, f"tail -n {tail_lines} ~/.yggterm/event-trace.jsonl")
        lines = result.stdout.splitlines()
    events: list[dict] = []
    for line in lines:
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (event.get("ts_ms") or 0) >= start_ms:
            events.append(event)
    return events


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
    if active_summary and not _summary_matches_probe_text(active_summary, button_tooltip):
        return False
    return True


def _summary_matches_probe_text(active_summary: str, probe_text: str) -> bool:
    active = active_summary.strip()
    probe = probe_text.strip()
    if not active:
        return True
    if not probe:
        return False
    return active == probe or active.startswith(probe) or probe.startswith(active)


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


def write_json(path: Path, payload: dict) -> str:
    path.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    return str(path)


def is_remote_machine_group(row: dict) -> bool:
    path = (row.get("full_path") or "").strip()
    return path.startswith("__remote_machine__/")


def openable_row_count(rows_payload: dict) -> int:
    return len(collect_open_targets(rows_payload, random.Random(23)))


def expand_groups_until_target(
    host: str,
    binary: str,
    timeout_ms: int,
    target_count: int,
    max_passes: int = 8,
) -> dict:
    last = app_rows(host, binary, timeout_ms)
    if openable_row_count(last) >= target_count:
        return last
    for _ in range(max_passes):
        rows = last.get("rows") or []
        collapsed_non_remote = [
            row
            for row in rows
            if row.get("kind") == "Group" and not row.get("expanded") and row.get("full_path")
            and not is_remote_machine_group(row)
        ]
        collapsed_remote = [
            row
            for row in rows
            if row.get("kind") == "Group" and not row.get("expanded") and row.get("full_path")
            and is_remote_machine_group(row)
        ]
        collapsed = collapsed_non_remote or collapsed_remote
        if not collapsed:
            return last
        short_timeout_ms = min(timeout_ms, 1500)
        for row in collapsed:
            try:
                app_expand(host, binary, row["full_path"], True, short_timeout_ms)
            except Exception:
                continue
        time.sleep(0.15)
        last = app_rows(host, binary, timeout_ms)
        if openable_row_count(last) >= target_count:
            return last
    return last


def collect_open_targets(rows_payload: dict, rng: random.Random) -> list[dict]:
    candidates: list[dict] = []
    seen_paths: set[str] = set()
    for row in rows_payload.get("rows") or []:
        kind = row.get("kind")
        session_path = canonical_session_path(row.get("full_path"))
        if kind not in {"Session", "Document"} or not session_path or session_path in seen_paths:
            continue
        if kind == "Document":
            document_kind = (row.get("document_kind") or "").strip().lower()
            requested_view = rng.choice(["preview", "terminal"])
            if document_kind == "terminal_recipe":
                expected_view = requested_view
            else:
                expected_view = "preview"
        else:
            requested_view = rng.choice(["preview", "terminal"])
            expected_view = requested_view
        candidates.append(
            {
                "full_path": session_path,
                "label": row.get("label") or session_path,
                "kind": kind,
                "document_kind": row.get("document_kind"),
                "requested_view": requested_view,
                "expected_view": expected_view,
            }
        )
        seen_paths.add(session_path)
    return candidates


def choose_open_targets(rows_payload: dict, rng: random.Random, count: int) -> list[dict]:
    candidates = collect_open_targets(rows_payload, rng)
    if not candidates:
        raise RuntimeError("no openable rows available")
    if len(candidates) > count:
        return rng.sample(candidates, count)
    return candidates


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
    rows_payload = expand_groups_until_target(args.host, args.bin, args.timeout_ms, args.count)
    targets = choose_open_targets(rows_payload, random.Random(args.seed), args.count)
    results: list[dict] = []

    for index, target in enumerate(targets):
        session_path = target["full_path"]
        requested_view = target["requested_view"]
        expected_view = target["expected_view"]
        started_ms = int(time.time() * 1000)
        deadline_ms = started_ms + int(args.ready_budget * 1000)
        entry = {
            "path": session_path,
            "label": target.get("label"),
            "kind": target.get("kind"),
            "document_kind": target.get("document_kind"),
            "requested_view": requested_view,
            "expected_view": expected_view,
        }
        try:
            entry["open"] = app_open(args.host, args.bin, session_path, requested_view, args.timeout_ms)
            elapsed, state = wait_until(
                f"open {session_path}",
                args.ready_budget,
                args.poll,
                lambda: _require_viewport_ready(
                    app_state(args.host, args.bin, args.timeout_ms),
                    session_path,
                    expected_view,
                    args.host,
                    started_ms,
                    deadline_ms,
                ),
            )
            viewport = state.get("viewport") or {}
            titlebar = viewport.get("titlebar") or {}
            notifications = ((state.get("shell") or {}).get("notifications_count")) or 0
            entry["elapsed_s"] = round(elapsed, 3)
            entry["within_budget"] = elapsed <= args.ready_budget
            entry["active_title"] = viewport.get("active_title")
            entry["active_summary"] = viewport.get("active_summary")
            entry["titlebar_title_text"] = titlebar.get("title_text")
            entry["titlebar_summary_text"] = titlebar.get("summary_text")
            entry["titlebar_button_tooltip"] = titlebar.get("button_tooltip")
            entry["titlebar_menu_open"] = titlebar.get("menu_open")
            entry["title_present"] = title_is_good(viewport.get("active_title"))
            entry["summary_present"] = bool((viewport.get("active_summary") or "").strip())
            entry["titlebar_matches_viewport"] = titlebar_matches_viewport(state)
            entry["notification_count"] = notifications
            entry["notification_delta"] = max(0, notifications - baseline_notifications)
            entry["state_dump"] = write_json(out_dir / f"open-{index:02d}.json", state)
            entry["attach_ready_before_deadline"] = (
                terminal_attach_ready_seen(args.host, session_path, started_ms, deadline_ms)
                if expected_view == "terminal"
                else None
            )
        except Exception as error:  # noqa: BLE001
            state = {}
            try:
                state = app_state(args.host, args.bin, args.timeout_ms)
            except Exception:
                state = {}
            viewport = state.get("viewport") or {}
            titlebar = viewport.get("titlebar") or {}
            notifications = ((state.get("shell") or {}).get("notifications_count")) or 0
            entry["error"] = str(error)
            entry["within_budget"] = False
            entry["active_title"] = viewport.get("active_title")
            entry["active_summary"] = viewport.get("active_summary")
            entry["titlebar_title_text"] = titlebar.get("title_text")
            entry["titlebar_summary_text"] = titlebar.get("summary_text")
            entry["titlebar_button_tooltip"] = titlebar.get("button_tooltip")
            entry["titlebar_menu_open"] = titlebar.get("menu_open")
            entry["title_present"] = title_is_good(viewport.get("active_title"))
            entry["summary_present"] = bool((viewport.get("active_summary") or "").strip())
            entry["titlebar_matches_viewport"] = titlebar_matches_viewport(state)
            entry["notification_count"] = notifications
            entry["notification_delta"] = max(0, notifications - baseline_notifications)
            entry["state_dump"] = write_json(out_dir / f"open-{index:02d}-failure.json", state)
            entry["attach_ready_before_deadline"] = (
                terminal_attach_ready_seen(args.host, session_path, started_ms, deadline_ms)
                if expected_view == "terminal"
                else None
            )
        results.append(entry)

    final_state = app_state(args.host, args.bin, args.timeout_ms)
    summary = {
        "host": args.host,
        "count": args.count,
        "executed_count": len(results),
        "unique_target_count": len({item["path"] for item in results}),
        "diversity_shortfall": max(0, args.count - len(results)),
        "seed": args.seed,
        "ready_budget_s": args.ready_budget,
        "window_spawn_elapsed_ms": ((launch_event or {}).get("payload") or {}).get("elapsed_ms"),
        "window_spawn_within_900ms": (
            (((launch_event or {}).get("payload") or {}).get("elapsed_ms") or 10_000) <= 900
        ),
        "baseline_notifications_count": baseline_notifications,
        "final_notifications_count": ((final_state.get("shell") or {}).get("notifications_count")) or 0,
        "open_failures": len([item for item in results if item.get("error")]),
        "ready_failures": len([item for item in results if not item.get("within_budget")]),
        "title_failures": len([item for item in results if not item.get("title_present")]),
        "summary_failures": len([item for item in results if not item.get("summary_present")]),
        "titlebar_failures": len([item for item in results if not item.get("titlebar_matches_viewport")]),
        "notification_anomalies": len([item for item in results if (item.get("notification_delta") or 0) > 0]),
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
        run_process(["bash", "-lc", "pkill -f 'yggterm server daemon' || true"], check=False)

    return 0 if (
        summary["diversity_shortfall"] == 0
        and
        summary["open_failures"] == 0
        and summary["ready_failures"] == 0
        and summary["title_failures"] == 0
        and summary["summary_failures"] == 0
        and summary["titlebar_failures"] == 0
        and summary["notification_anomalies"] == 0
    ) else 1


def _require_viewport_ready(
    state: dict,
    session_path: str,
    view: str,
    host: str,
    started_ms: int,
    deadline_ms: int,
) -> dict:
    if not viewport_ready(state, session_path, view):
        if (
            view == "terminal"
            and viewport_matches_target(state, session_path, view)
            and titlebar_matches_viewport(state)
            and terminal_attach_ready_seen(host, session_path, started_ms, deadline_ms)
        ):
            return state
        viewport = state.get("viewport") or {}
        raise RuntimeError(viewport.get("reason") or "viewport not ready")
    if not titlebar_matches_viewport(state):
        raise RuntimeError("titlebar not in sync with viewport")
    return state


if __name__ == "__main__":
    raise SystemExit(main())
