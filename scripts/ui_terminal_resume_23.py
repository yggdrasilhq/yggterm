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


PLACEHOLDER_FRAGMENTS = (
    "resuming live codex session",
    "waiting for the remote terminal to paint",
    "the terminal will appear here",
    "still connecting to remote terminal",
)

ERROR_FRAGMENTS = (
    "mux_client_request_session",
    "session open refused by peer",
    "controlsocket",
    "permission denied",
    "connection refused",
    "could not resolve hostname",
    "no route to host",
    "connection timed out",
    "broken pipe",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Open 23 existing remote sessions in terminal view and verify the terminal paints "
            "real, non-placeholder content within a strict budget."
        )
    )
    parser.add_argument("--host", default="local")
    parser.add_argument("--bin", default="./target/debug/yggterm")
    parser.add_argument("--count", type=int, default=23)
    parser.add_argument("--seed", type=int, default=23)
    parser.add_argument("--timeout-ms", type=int, default=8000)
    parser.add_argument("--poll", type=float, default=0.08)
    parser.add_argument("--paint-budget", type=float, default=1.0)
    parser.add_argument("--launch-local", action="store_true")
    parser.add_argument("--out-dir", default="/tmp/yggterm-terminal-resume-23")
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


def quote(value: str) -> str:
    return shlex.quote(value)


def write_json(path: Path, payload: dict) -> str:
    path.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    return str(path)


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


def kill_local_clients() -> None:
    instances_root = local_yggterm_path("client-instances")
    using_default_home = local_yggterm_home() == Path.home() / ".yggterm"
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
    if using_default_home:
        run_process(["bash", "-lc", "pkill -f 'yggterm server daemon' || true"], check=False)


def launch_local_client(binary: str, timeout_s: float = 4.0) -> tuple[subprocess.Popen, dict]:
    binary_path = str(Path(binary).resolve())
    kill_local_clients()
    env = os.environ.copy()
    env.setdefault("DISPLAY", ":10.0")
    env.pop("XRDP_SESSION", None)
    env.pop("XRDP_SOCKET_PATH", None)
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


def app_state(host: str, binary: str, timeout_ms: int) -> dict:
    payload = run_json(host, f"{quote(binary)} server app state --timeout-ms {timeout_ms}")
    return payload.get("data") or {}


def app_rows(host: str, binary: str, timeout_ms: int) -> dict:
    payload = run_json(host, f"{quote(binary)} server app rows --timeout-ms {timeout_ms}")
    return payload.get("data") or {}


def app_open(host: str, binary: str, session_path: str, timeout_ms: int) -> dict:
    command = (
        f"{quote(binary)} server app open {json.dumps(session_path)} "
        f"--view terminal --timeout-ms {timeout_ms}"
    )
    return run_json(host, command)


def app_screenshot(host: str, binary: str, dest: Path, timeout_ms: int) -> str | None:
    command = (
        f"{quote(binary)} server app screenshot {quote(str(dest))} "
        f"--timeout-ms {timeout_ms}"
    )
    result = run_control(host, command, check=False)
    return str(dest) if result.returncode == 0 and dest.exists() else None


def wait_for_window(host: str, binary: str, timeout_ms: int) -> dict:
    def _probe() -> dict:
        state = app_state(host, binary, timeout_ms)
        window = state.get("window") or {}
        if not window.get("visible"):
            raise RuntimeError("window not visible")
        return state

    return wait_until("window visible", 8.0, 0.25, _probe)[1]


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


def active_terminal_text(state: dict) -> str:
    viewport = state.get("viewport") or {}
    hosts = viewport.get("active_terminal_hosts") or []
    if not hosts:
        return ""
    return "\n".join((host.get("text_sample") or "") for host in hosts).strip()


def terminal_resume_overlay(state: dict) -> dict:
    viewport = state.get("viewport") or {}
    overlay = viewport.get("terminal_resume_overlay") or {}
    if not isinstance(overlay, dict):
        return {"visible": False, "text_sample": ""}
    return {
        "visible": bool(overlay.get("visible")),
        "text_sample": str(overlay.get("text_sample") or ""),
    }


def terminal_text_looks_like_placeholder(text: str) -> bool:
    normalized = " ".join(text.strip().lower().split())
    if not normalized:
        return True
    return any(fragment in normalized for fragment in PLACEHOLDER_FRAGMENTS)


def resume_overlay_counts_as_painted(viewport: dict, overlay: dict, terminal_text: str) -> bool:
    return False


def terminal_text_looks_like_error(text: str) -> bool:
    normalized = " ".join(text.strip().lower().split())
    if not normalized:
        return False
    return any(fragment in normalized for fragment in ERROR_FRAGMENTS)


def terminal_text_looks_like_generic_idle(text: str) -> bool:
    stripped = text.strip()
    if not stripped:
        return False
    lowered = stripped.lower()
    has_codex_header = "openai codex" in lowered and "/model to change" in lowered
    if not has_codex_header:
        return False
    printable = sum(1 for ch in stripped if not ch.isspace() and ch.isprintable())
    lines = [line.strip() for line in stripped.splitlines() if line.strip()]
    if printable > 420 or len(lines) > 12:
        return False
    transcript_like_lines = 0
    for line in lines:
        lower = line.lower()
        if line.startswith(">_ OpenAI Codex"):
            continue
        if lower.startswith("tip:"):
            continue
        if lower.startswith("model:"):
            continue
        if lower.startswith("directory:"):
            continue
        transcript_like_lines += 1
    return transcript_like_lines <= 2


def terminal_text_looks_like_shell_prompt(text: str) -> bool:
    stripped = text.strip()
    if not stripped:
        return False
    lines = [line.strip() for line in stripped.splitlines() if line.strip()]
    if not lines or len(lines) > 4:
        return False
    for line in lines:
        if line.endswith(("$", "#", "%", "$ ", "# ", "% ")):
            continue
        if "@" in line and ":" in line and (line.endswith("$") or line.endswith("#")):
            continue
        return False
    return True


def is_remote_machine_group(row: dict) -> bool:
    path = (row.get("full_path") or "").strip()
    return path.startswith("__remote_machine__/")


def remote_session_row_count(rows_payload: dict) -> int:
    return len(collect_remote_terminal_targets(rows_payload))


def expand_groups_until_target(
    host: str,
    binary: str,
    timeout_ms: int,
    target_count: int,
    max_passes: int = 8,
) -> dict:
    last = app_rows(host, binary, timeout_ms)
    if remote_session_row_count(last) >= target_count:
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
        for row in collapsed:
            command = (
                f"{quote(binary)} server app expand {json.dumps(row['full_path'])} "
                f"--timeout-ms {min(timeout_ms, 1500)}"
            )
            try:
                run_json(host, command)
            except Exception:
                continue
        time.sleep(0.15)
        last = app_rows(host, binary, timeout_ms)
        if remote_session_row_count(last) >= target_count:
            return last
    return last


def collect_remote_terminal_targets(rows_payload: dict) -> list[dict]:
    candidates: list[dict] = []
    seen_paths: set[str] = set()
    for row in rows_payload.get("rows") or []:
        path = (row.get("full_path") or "").strip()
        if row.get("kind") != "Session" or not path.startswith("remote-session://"):
            continue
        if path in seen_paths:
            continue
        candidates.append(
            {
                "full_path": path,
                "label": row.get("label") or path,
                "summary": row.get("summary") or "",
            }
        )
        seen_paths.add(path)
    return candidates


def choose_remote_terminal_targets(rows_payload: dict, seed: int, count: int) -> list[dict]:
    candidates = collect_remote_terminal_targets(rows_payload)
    if not candidates:
        raise RuntimeError("no remote session rows available for terminal-resume test")

    chosen: list[dict] = []
    seen_paths: set[str] = set()

    def add(candidate: dict) -> None:
        path = candidate["full_path"]
        if path in seen_paths or len(chosen) >= count:
            return
        chosen.append(candidate)
        seen_paths.add(path)

    # Always cover the first recent visible session per host before sampling broadly.
    seen_hosts: set[str] = set()
    for candidate in candidates:
        host = candidate["full_path"].split("//", 1)[-1].split("/", 1)[0]
        if host in seen_hosts:
            continue
        seen_hosts.add(host)
        add(candidate)

    # Then prioritize the top of the displayed tree, where user-facing regressions are most obvious.
    for candidate in candidates:
        add(candidate)
        if len(chosen) >= min(count, max(8, count // 2)):
            break

    remaining = [candidate for candidate in candidates if candidate["full_path"] not in seen_paths]
    rng = random.Random(seed)
    rng.shuffle(remaining)
    for candidate in remaining:
        add(candidate)
        if len(chosen) >= count:
            break

    return chosen


def require_terminal_painted(state: dict, session_path: str) -> dict:
    viewport = state.get("viewport") or {}
    if viewport.get("active_view_mode") != "Terminal":
        raise RuntimeError("viewport not in terminal mode")
    if viewport.get("active_session_path") != session_path:
        raise RuntimeError("viewport not on requested session")
    if not titlebar_matches_viewport(state):
        raise RuntimeError("titlebar not in sync with viewport")
    if (viewport.get("active_terminal_host_count") or 0) <= 0:
        raise RuntimeError(viewport.get("reason") or "active terminal host missing")
    text = active_terminal_text(state)
    overlay = terminal_resume_overlay(state)
    overlay_visible = bool(overlay.get("visible"))
    overlay_text = (overlay.get("text_sample") or "").strip().lower()
    active_summary = (viewport.get("active_summary") or "").strip()
    if terminal_text_looks_like_error(text):
        raise RuntimeError("terminal painted transport/error output instead of the session")
    if terminal_text_looks_like_generic_idle(text):
        raise RuntimeError("terminal exposed generic Codex idle prompt instead of restored session UX")
    if terminal_text_looks_like_shell_prompt(text):
        raise RuntimeError("terminal exposed plain shell prompt instead of restored session UX")
    if overlay_visible and "live terminal is ready" in overlay_text:
        raise RuntimeError("resume overlay still uses the old misleading terminal-ready copy")
    if overlay_visible and "still connecting to remote terminal" in overlay_text:
        raise RuntimeError("resume overlay still uses indefinite connecting copy")
    if overlay_visible:
        raise RuntimeError("resume overlay still covers terminal after paint budget")
    if terminal_text_looks_like_placeholder(text):
        raise RuntimeError("terminal still showing placeholder content")
    if not text:
        raise RuntimeError("terminal is blank")
    return state


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    launch = None
    launch_event = None
    if args.host == "local" and args.launch_local:
        launch, launch_event = launch_local_client(args.bin)

    wait_for_window(args.host, args.bin, args.timeout_ms)
    rows_payload = expand_groups_until_target(args.host, args.bin, args.timeout_ms, args.count)
    targets = choose_remote_terminal_targets(rows_payload, args.seed, args.count)
    results: list[dict] = []

    for index, target in enumerate(targets):
        session_path = target["full_path"]
        started_ms = int(time.time() * 1000)
        entry = {
            "path": session_path,
            "label": target.get("label"),
            "summary": target.get("summary"),
        }
        try:
            entry["open"] = app_open(args.host, args.bin, session_path, args.timeout_ms)
            elapsed, state = wait_until(
                f"terminal paint {session_path}",
                args.paint_budget,
                args.poll,
                lambda: require_terminal_painted(
                    app_state(args.host, args.bin, args.timeout_ms),
                    session_path,
                ),
            )
            viewport = state.get("viewport") or {}
            entry["elapsed_s"] = round(elapsed, 3)
            entry["within_budget"] = elapsed <= args.paint_budget
            entry["ready"] = bool(viewport.get("ready"))
            entry["reason"] = viewport.get("reason")
            entry["active_title"] = viewport.get("active_title")
            entry["active_summary"] = viewport.get("active_summary")
            entry["terminal_resume_overlay"] = terminal_resume_overlay(state)
            entry["terminal_text_sample"] = active_terminal_text(state)
            entry["state_dump"] = write_json(out_dir / f"terminal-resume-{index:02d}.json", state)
        except Exception as error:  # noqa: BLE001
            state = {}
            try:
                state = app_state(args.host, args.bin, args.timeout_ms)
            except Exception:
                state = {}
            viewport = state.get("viewport") or {}
            entry["error"] = str(error)
            entry["within_budget"] = False
            entry["ready"] = bool(viewport.get("ready"))
            entry["reason"] = viewport.get("reason")
            entry["active_title"] = viewport.get("active_title")
            entry["active_summary"] = viewport.get("active_summary")
            entry["terminal_resume_overlay"] = terminal_resume_overlay(state)
            entry["terminal_text_sample"] = active_terminal_text(state)
            entry["state_dump"] = write_json(
                out_dir / f"terminal-resume-{index:02d}-failure.json",
                state,
            )
            if args.host == "local":
                shot = app_screenshot(
                    args.host,
                    args.bin,
                    out_dir / f"terminal-resume-{index:02d}-failure.png",
                    args.timeout_ms,
                )
                if shot:
                    entry["screenshot"] = shot
        results.append(entry)

    summary = {
        "host": args.host,
        "count": args.count,
        "executed_count": len(results),
        "unique_target_count": len({item["path"] for item in results}),
        "seed": args.seed,
        "paint_budget_s": args.paint_budget,
        "window_spawn_elapsed_ms": ((launch_event or {}).get("payload") or {}).get("elapsed_ms"),
        "window_spawn_within_900ms": (
            (((launch_event or {}).get("payload") or {}).get("elapsed_ms") or 10_000) <= 900
        ),
        "open_failures": len([item for item in results if item.get("open") is None]),
        "paint_budget_failures": len([item for item in results if not item.get("within_budget")]),
        "placeholder_failures": len(
            [
                item for item in results
                if terminal_text_looks_like_placeholder(item.get("terminal_text_sample") or "")
                and not resume_overlay_counts_as_painted(
                    {
                        "active_summary": item.get("active_summary") or "",
                    },
                    item.get("terminal_resume_overlay") or {},
                    item.get("terminal_text_sample") or "",
                )
            ]
        ),
        "generic_idle_failures": len(
            [
                item for item in results
                if terminal_text_looks_like_generic_idle(item.get("terminal_text_sample") or "")
            ]
        ),
        "shell_prompt_failures": len(
            [
                item for item in results
                if terminal_text_looks_like_shell_prompt(item.get("terminal_text_sample") or "")
            ]
        ),
        "overlay_copy_failures": len(
            [
                item for item in results
                if any(
                    bad in ((item.get("terminal_resume_overlay") or {}).get("text_sample") or "").lower()
                    for bad in ("live terminal is ready", "still connecting to remote terminal")
                )
            ]
        ),
        "terminal_error_failures": len(
            [item for item in results if terminal_text_looks_like_error(item.get("terminal_text_sample") or "")]
        ),
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
        summary["executed_count"] == args.count
        and summary["open_failures"] == 0
        and summary["paint_budget_failures"] == 0
        and summary["placeholder_failures"] == 0
        and summary["generic_idle_failures"] == 0
        and summary["shell_prompt_failures"] == 0
        and summary["terminal_error_failures"] == 0
        and summary["overlay_copy_failures"] == 0
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
