#!/usr/bin/env python3
import argparse
import json
import os
import random
import re
import shlex
import signal
import shutil
import statistics
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
    "error: reading /tmp/yggterm-screen",
    "mux_client_request_session",
    "session open refused by peer",
    "controlsocket",
    "exec: export: not found",
    "permission denied",
    "connection refused",
    "connection timed out",
    "broken pipe",
    "shared connection to ",
    "terminal session not found",
)

TRANSCRIPT_BROWSER_HINTS = (
    "q to quit",
    "to scroll",
    "pgup/pgdn",
    "home/end to jump",
    "home end to jump",
    "esc to edit prev",
    "edit prev",
)

SAVED_TRANSCRIPT_HINTS = (
    "saved transcript ·",
    "typing takes over the live terminal",
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
    parser.add_argument("--paint-budget", type=float, default=0.45)
    parser.add_argument("--launch-local", action="store_true")
    parser.add_argument("--include-unreachable", action="store_true")
    parser.add_argument(
        "--no-roundtrip-preview",
        action="store_true",
        help="Skip the preview -> terminal round-trip and only measure cold terminal open.",
    )
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


def launch_local_client(binary: str, timeout_s: float = 4.0) -> tuple[subprocess.Popen, dict]:
    binary_path = str(Path(binary).resolve())
    kill_local_clients(binary_path)
    env = os.environ.copy()
    env.setdefault("DISPLAY", ":10.0")
    env.setdefault("XAUTHORITY", str(Path.home() / ".Xauthority"))
    env.pop("XRDP_SESSION", None)
    env.pop("XRDP_SOCKET_PATH", None)
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


def app_open(host: str, binary: str, session_path: str, timeout_ms: int, view: str) -> dict:
    command = (
        f"{quote(binary)} server app open {json.dumps(session_path)} "
        f"--view {quote(view)} --timeout-ms {timeout_ms}"
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
    hosts = viewport.get("active_terminal_hosts") or []
    host_kind = ""
    host_visible = False
    host_excerpt = ""
    if hosts:
        first = hosts[0] or {}
        host_kind = str(first.get("resume_overlay_kind") or "")
        host_visible = bool(first.get("resume_overlay_visible"))
        host_excerpt = str(first.get("resume_overlay_excerpt") or "")
    if not isinstance(overlay, dict):
        return {"visible": host_visible, "text_sample": "", "excerpt": host_excerpt, "kind": host_kind}
    return {
        "visible": bool(overlay.get("visible")) or host_visible,
        "text_sample": str(overlay.get("text_sample") or ""),
        "excerpt": str(overlay.get("excerpt") or host_excerpt or ""),
        "kind": str(overlay.get("kind") or host_kind or ""),
    }


def active_terminal_surface(state: dict) -> dict:
    viewport = state.get("viewport") or {}
    surface = viewport.get("active_terminal_surface") or {}
    if not isinstance(surface, dict):
        return {"rendered": False, "problem": None}
    return {
        "rendered": bool(surface.get("rendered")),
        "problem": surface.get("problem"),
    }


def active_terminal_open_attempt(state: dict) -> dict:
    viewport = state.get("viewport") or {}
    attempt = viewport.get("terminal_open_attempt")
    if not isinstance(attempt, dict):
        attempt = state.get("terminal_open_attempt")
    if not isinstance(attempt, dict):
        return {}
    return attempt


def looks_like_low_signal_summary(text: str) -> bool:
    normalized = " ".join(text.strip().lower().split())
    if not normalized:
        return True
    return normalized.startswith("local codex terminal rooted at ") or normalized.startswith(
        "ssh terminal on "
    )


def terminal_text_looks_like_placeholder(text: str) -> bool:
    normalized = " ".join(text.strip().lower().split())
    if not normalized:
        return True
    return any(fragment in normalized for fragment in PLACEHOLDER_FRAGMENTS)


def resume_overlay_counts_as_painted(viewport: dict, overlay: dict, terminal_text: str) -> bool:
    if not overlay.get("visible"):
        return False
    overlay_text = str(overlay.get("text_sample") or "").strip()
    overlay_excerpt = str(overlay.get("excerpt") or "").strip()
    source_text = terminal_text.strip() or overlay_excerpt or overlay_text
    if not source_text:
        return False
    overlay_has_saved_transcript = bool(overlay_excerpt) or "saved transcript" in overlay_text.lower()
    active_summary = str(viewport.get("active_summary") or "").strip()
    if source_text == overlay_text and not overlay_has_saved_transcript:
        return False
    if overlay_has_saved_transcript and looks_like_low_signal_summary(active_summary) and not overlay_excerpt:
        return False
    if terminal_text_looks_like_placeholder(source_text) and not overlay_has_saved_transcript:
        return False
    if terminal_text_looks_like_error(source_text):
        return False
    if terminal_text_looks_like_shell_prompt(source_text):
        return False
    if terminal_text_looks_like_transcript_browser(source_text):
        return False
    return True


def terminal_text_looks_like_error(text: str) -> bool:
    lines = [line.strip().lower() for line in text.splitlines() if line.strip()]
    if not lines:
        return False
    head = " ".join(lines[:4])
    if any(fragment in head for fragment in ERROR_FRAGMENTS):
        return True
    return any(
        line.startswith("connection to ")
        and (" closed" in line or "refused" in line or "timed out" in line)
        for line in lines[:4]
    ) or any("no route to host" in line for line in lines[:2])


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
        semantic = line.strip("╭╮╰╯─│ ")
        lower = semantic.lower()
        border_only = not semantic
        if "openai codex" in lower:
            continue
        if lower.startswith("tip:"):
            continue
        if "model:" in lower:
            continue
        if "directory:" in lower:
            continue
        if lower.startswith("›"):
            continue
        if "% left" in lower:
            continue
        if border_only:
            continue
        transcript_like_lines += 1
    return transcript_like_lines <= 2


def strip_ansi(text: str) -> str:
    return re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", text)


def terminal_text_looks_like_generic_idle_footer(text: str) -> bool:
    normalized = " ".join(text.strip().lower().split())
    if not normalized:
        return False
    mentions_generic_prompt = (
        "implement {feature}" in normalized
        or "explain this codebase" in normalized
        or "find and fix a bug" in normalized
        or "resume a previous session" in normalized
    )
    mentions_model_footer = (
        ("gpt-5" in normalized or "gpt-4" in normalized or "claude" in normalized)
        and "% left" in normalized
    )
    return mentions_generic_prompt and mentions_model_footer


def terminal_text_has_meaningful_resume_context(text: str) -> bool:
    stripped = strip_ansi(text)
    lines = [line.strip() for line in stripped.splitlines() if line.strip()]
    if not lines:
        return False
    kept = []
    for line in lines:
        lower = line.lower()
        semantic = line.strip("╭╮╰╯─│ ")
        if not semantic:
            continue
        if "openai codex" in lower:
            continue
        if lower.startswith("tip:"):
            continue
        if "model:" in lower:
            continue
        if "directory:" in lower:
            continue
        if lower.startswith("›"):
            continue
        if "% left" in lower:
            continue
        if terminal_text_looks_like_generic_idle_footer(line):
            continue
        kept.append(semantic)
    if not kept:
        return False
    printable = sum(1 for ch in " ".join(kept) if ch.isprintable() and not ch.isspace())
    return printable >= 80 or len(kept) >= 4


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


def terminal_text_looks_like_transcript_browser(text: str) -> bool:
    normalized = " ".join(text.strip().lower().split())
    if not normalized:
        return False
    if (
        "resume a previous session" in normalized
        and (
            "sort updated at" in normalized
            or "sort: updated at" in normalized
            or "conversation" in normalized
        )
    ):
        return True
    if "transcript" not in normalized and "t r a n s c r i p t" not in normalized:
        return False
    return any(fragment in normalized for fragment in TRANSCRIPT_BROWSER_HINTS)


def terminal_text_looks_like_saved_transcript_prefill(text: str) -> bool:
    normalized = " ".join(text.strip().lower().split())
    if not normalized:
        return False
    return normalized.startswith("saved transcript") and all(
        fragment in normalized for fragment in SAVED_TRANSCRIPT_HINTS
    )


def terminal_text_looks_like_launcher_boilerplate(text: str) -> bool:
    normalized = " ".join(text.strip().lower().split())
    if not normalized:
        return False
    return (
        "open live terminal " in normalized
        or "launch command prepared:" in normalized
        or "daemon pty:" in normalized
        or "queue remote yggterm resume " in normalized
        or "__yggterm_requested=" in text
        or "__yggterm_cwd_ok=" in text
    )


def overlay_resolved(overlay: dict) -> bool:
    if not bool(overlay.get("visible")):
        return True
    text = " ".join(str(overlay.get("text_sample") or "").strip().lower().split())
    if not text:
        return False
    return (
        "remote host is unavailable" in text
        or "remote terminal needs attention" in text
        or "still not interactive" in text
        or "has not become interactive" in text
    )


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
                "remote_host": path.split("//", 1)[-1].split("/", 1)[0],
                "label": row.get("label") or path,
                "summary": str(row.get("summary") or "").strip(),
            }
        )
        seen_paths.add(path)
    return candidates


def probe_remote_host(host: str) -> tuple[bool, str]:
    result = run_process(
        ["bash", "-lc", f"timeout 5s ssh {quote(host)} 'true'"],
        check=False,
    )
    stderr = (result.stderr or "").strip()
    stdout = (result.stdout or "").strip()
    message = stderr or stdout
    return result.returncode == 0, message


def choose_remote_terminal_targets(
    rows_payload: dict,
    seed: int,
    count: int,
    *,
    include_unreachable: bool,
) -> tuple[list[dict], dict[str, dict]]:
    candidates = collect_remote_terminal_targets(rows_payload)
    if not candidates:
        raise RuntimeError("no remote session rows available for terminal-resume test")

    host_probe: dict[str, dict] = {}
    for candidate in candidates:
        remote_host = candidate["remote_host"]
        if remote_host in host_probe:
            continue
        reachable, detail = probe_remote_host(remote_host)
        host_probe[remote_host] = {"reachable": reachable, "detail": detail}

    if not include_unreachable:
        candidates = [
            candidate
            for candidate in candidates
            if host_probe.get(candidate["remote_host"], {}).get("reachable")
        ]
        if not candidates:
            raise RuntimeError("no SSH-reachable remote sessions available for terminal-resume test")

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

    return chosen, host_probe


def percentile(values: list[float], pct: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    if len(ordered) == 1:
        return ordered[0]
    index = max(0.0, min(len(ordered) - 1, (len(ordered) - 1) * pct))
    lower = int(index)
    upper = min(lower + 1, len(ordered) - 1)
    if lower == upper:
        return ordered[lower]
    fraction = index - lower
    return ordered[lower] * (1 - fraction) + ordered[upper] * fraction


def require_terminal_painted(state: dict, session_path: str, expected_attempt_id: str | None = None) -> dict:
    viewport = state.get("viewport") or {}
    if viewport.get("active_view_mode") != "Terminal":
        raise RuntimeError("viewport not in terminal mode")
    if viewport.get("active_session_path") != session_path:
        raise RuntimeError("viewport not on requested session")
    if not titlebar_matches_viewport(state):
        raise RuntimeError("titlebar not in sync with viewport")
    if (viewport.get("active_terminal_host_count") or 0) <= 0:
        raise RuntimeError(viewport.get("reason") or "active terminal host missing")
    first = (viewport.get("active_terminal_hosts") or [{}])[0]
    child_count = int(first.get("child_count") or 0)
    xterm_present = bool(first.get("xterm_present"))
    screen_present = bool(first.get("screen_present"))
    viewport_present = bool(first.get("viewport_present"))
    rows_present = bool(first.get("rows_present"))
    cols = int(first.get("cols") or 0)
    rows = int(first.get("rows") or 0)
    if not (child_count > 0 or xterm_present or screen_present or viewport_present or rows_present):
        raise RuntimeError("terminal host exists but no visible xterm paint landed")
    if cols < 20 or rows < 4:
        raise RuntimeError(
            f"terminal geometry is still degenerate cols={cols} rows={rows}"
        )
    text = active_terminal_text(state)
    overlay = terminal_resume_overlay(state)
    surface = active_terminal_surface(state)
    attempt = active_terminal_open_attempt(state)
    overlay_visible = bool(overlay.get("visible"))
    overlay_text = (overlay.get("text_sample") or "").strip().lower()
    overlay_kind = (overlay.get("kind") or "").strip().lower()
    surface_problem = (surface.get("problem") or "").strip()
    attempt_id = (attempt.get("attempt_id") or "").strip()
    latched_failure_reason = (attempt.get("latched_failure_reason") or "").strip()
    if expected_attempt_id:
        if not attempt_id:
            raise RuntimeError("terminal open attempt is missing from app state")
        if attempt_id != expected_attempt_id:
            raise RuntimeError(
                f"terminal open attempt drifted expected={expected_attempt_id} actual={attempt_id}"
            )
    if latched_failure_reason:
        raise RuntimeError(f"terminal open attempt latched failure: {latched_failure_reason}")
    if surface_problem:
        raise RuntimeError(surface_problem)
    if terminal_text_looks_like_error(text):
        raise RuntimeError("terminal painted transport/error output instead of the session")
    if terminal_text_looks_like_generic_idle(text):
        raise RuntimeError("terminal resumed into generic Codex idle surface")
    if terminal_text_looks_like_generic_idle_footer(
        text
    ) and not terminal_text_has_meaningful_resume_context(text):
        raise RuntimeError("terminal resumed into generic Codex idle footer instead of the session")
    if terminal_text_looks_like_shell_prompt(text):
        raise RuntimeError("terminal exposed plain shell prompt instead of restored session UX")
    if terminal_text_looks_like_transcript_browser(text):
        raise RuntimeError("terminal exposed Codex transcript browser instead of live session surface")
    if terminal_text_looks_like_saved_transcript_prefill(text):
        raise RuntimeError("terminal still showing saved transcript prefill instead of the live host")
    if terminal_text_looks_like_launcher_boilerplate(text):
        raise RuntimeError("terminal is still showing launcher boilerplate instead of the live host")
    if overlay_visible:
        if overlay_kind != "chip":
            raise RuntimeError("resume overlay still covers terminal after paint budget")
        raise RuntimeError("resume chip still visible after paint budget")
    if "live terminal is ready" in overlay_text:
        raise RuntimeError("resume overlay still uses the old misleading terminal-ready copy")
    if "still connecting to remote terminal" in overlay_text:
        raise RuntimeError("resume overlay still uses indefinite connecting copy")
    if terminal_text_looks_like_placeholder(text):
        raise RuntimeError("terminal still showing placeholder content")
    if not text:
        raise RuntimeError("terminal is blank")
    if len(text.strip()) <= 24:
        raise RuntimeError("terminal did not paint meaningful saved/live context")
    return state


def overlay_reports_failure(overlay: dict) -> bool:
    text = " ".join(str(overlay.get("text_sample") or "").strip().lower().split())
    if not text:
        return False
    return (
        "remote host is unavailable" in text
        or "remote terminal needs attention" in text
        or "still not interactive" in text
        or "has not become interactive" in text
    )


def require_overlay_resolved(state: dict, session_path: str) -> dict:
    viewport = state.get("viewport") or {}
    if viewport.get("active_view_mode") != "Terminal":
        raise RuntimeError("viewport not in terminal mode")
    if viewport.get("active_session_path") != session_path:
        raise RuntimeError("viewport not on requested session")
    overlay = terminal_resume_overlay(state)
    if overlay_reports_failure(overlay):
        raise RuntimeError("resume settled into failure card")
    if bool(overlay.get("visible")):
        raise RuntimeError("resume chip still visible")
    return state


def require_view_selected(state: dict, session_path: str, view_mode: str) -> dict:
    viewport = state.get("viewport") or {}
    if viewport.get("active_session_path") != session_path:
        raise RuntimeError("viewport not on requested session")
    if viewport.get("active_view_mode") != view_mode:
        raise RuntimeError(f"viewport not in {view_mode.lower()} mode")
    return state


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    roundtrip_preview = not args.no_roundtrip_preview

    launch = None
    launch_event = None
    if args.host == "local" and args.launch_local:
        launch, launch_event = launch_local_client(args.bin)

    wait_for_window(args.host, args.bin, args.timeout_ms)
    rows_payload = expand_groups_until_target(args.host, args.bin, args.timeout_ms, args.count)
    targets, host_probe = choose_remote_terminal_targets(
        rows_payload,
        args.seed,
        args.count,
        include_unreachable=args.include_unreachable,
    )
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
            open_started = time.monotonic()
            entry["open"] = app_open(
                args.host,
                args.bin,
                session_path,
                args.timeout_ms,
                "terminal",
            )
            attempt = (
                (entry["open"].get("data") or {}).get("terminal_open_attempt")
                if isinstance(entry["open"], dict)
                else None
            ) or {}
            attempt_id = (attempt.get("attempt_id") or "").strip() or None
            entry["terminal_open_attempt"] = attempt
            cold_elapsed, cold_state = wait_until(
                f"terminal paint {session_path}",
                args.paint_budget,
                args.poll,
                lambda: require_terminal_painted(
                    app_state(args.host, args.bin, args.timeout_ms),
                    session_path,
                    attempt_id,
                ),
            )
            cold_elapsed = time.monotonic() - open_started
            entry["cold_elapsed_s"] = round(cold_elapsed, 3)
            state = cold_state
            elapsed = cold_elapsed
            if roundtrip_preview:
                entry["preview_open"] = app_open(
                    args.host,
                    args.bin,
                    session_path,
                    args.timeout_ms,
                    "preview",
                )
                preview_elapsed, preview_state = wait_until(
                    f"preview switch {session_path}",
                    1.5,
                    args.poll,
                    lambda: require_view_selected(
                        app_state(args.host, args.bin, args.timeout_ms),
                        session_path,
                        "Rendered",
                    ),
                )
                entry["preview_switch_s"] = round(preview_elapsed, 3)
                roundtrip_started = time.monotonic()
                entry["roundtrip_open"] = app_open(
                    args.host,
                    args.bin,
                    session_path,
                    args.timeout_ms,
                    "terminal",
                )
                elapsed, state = wait_until(
                    f"terminal roundtrip paint {session_path}",
                    args.paint_budget,
                    args.poll,
                    lambda: require_terminal_painted(
                        app_state(args.host, args.bin, args.timeout_ms),
                        session_path,
                        attempt_id,
                    ),
                )
                elapsed = time.monotonic() - roundtrip_started
                entry["roundtrip_elapsed_s"] = round(elapsed, 3)
                entry["preview_state_dump"] = write_json(
                    out_dir / f"terminal-resume-{index:02d}-preview.json",
                    preview_state,
                )
            viewport = state.get("viewport") or {}
            entry["elapsed_s"] = round(cold_elapsed, 3)
            entry["within_budget"] = elapsed <= args.paint_budget
            entry["ready"] = bool(viewport.get("ready"))
            entry["reason"] = viewport.get("reason")
            entry["active_title"] = viewport.get("active_title")
            entry["active_summary"] = viewport.get("active_summary")
            entry["terminal_resume_overlay"] = terminal_resume_overlay(state)
            entry["terminal_surface"] = active_terminal_surface(state)
            entry["terminal_open_attempt"] = active_terminal_open_attempt(state)
            entry["terminal_text_sample"] = active_terminal_text(state)
            overlay_elapsed, overlay_state = wait_until(
                f"overlay settle {session_path}",
                args.paint_budget,
                args.poll,
                lambda: require_overlay_resolved(
                    app_state(args.host, args.bin, args.timeout_ms),
                    session_path,
                ),
            )
            entry["overlay_resolved_s"] = round(overlay_elapsed, 3)
            entry["overlay_resolved_within_budget"] = overlay_elapsed <= args.paint_budget
            entry["terminal_resume_overlay_after_settle"] = terminal_resume_overlay(overlay_state)
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
            entry["terminal_surface"] = active_terminal_surface(state)
            entry["terminal_open_attempt"] = active_terminal_open_attempt(state)
            entry["terminal_text_sample"] = active_terminal_text(state)
            entry["overlay_resolved_within_budget"] = False
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
        "reachable_hosts": sorted(
            host for host, payload in host_probe.items() if payload.get("reachable")
        ),
        "unreachable_hosts": {
            host: payload.get("detail")
            for host, payload in host_probe.items()
            if not payload.get("reachable")
        },
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
            ]
        ),
        "generic_idle_surfaces": len(
            [
                item for item in results
                if terminal_text_looks_like_generic_idle(item.get("terminal_text_sample") or "")
            ]
        ),
        "generic_idle_footer_surfaces": len(
            [
                item for item in results
                if terminal_text_looks_like_generic_idle_footer(item.get("terminal_text_sample") or "")
                and not terminal_text_has_meaningful_resume_context(
                    item.get("terminal_text_sample") or ""
                )
            ]
        ),
        "shell_prompt_failures": len(
            [
                item for item in results
                if terminal_text_looks_like_shell_prompt(item.get("terminal_text_sample") or "")
            ]
        ),
        "transcript_browser_failures": len(
            [
                item for item in results
                if terminal_text_looks_like_transcript_browser(item.get("terminal_text_sample") or "")
            ]
        ),
        "launcher_boilerplate_failures": len(
            [
                item for item in results
                if terminal_text_looks_like_launcher_boilerplate(
                    item.get("terminal_text_sample") or ""
                )
            ]
        ),
        "overlay_copy_failures": len(
            [
                item for item in results
                if any(
                    bad in ((item.get("terminal_resume_overlay") or {}).get("text_sample") or "").lower()
                    for bad in ("live terminal is ready", "still connecting to remote terminal")
                )
                or (
                    "saved transcript is visible"
                    in ((item.get("terminal_resume_overlay") or {}).get("text_sample") or "").lower()
                    and not ((item.get("terminal_resume_overlay") or {}).get("excerpt") or "").strip()
                    and looks_like_low_signal_summary(item.get("active_summary") or "")
                )
            ]
        ),
        "overlay_visible_failures": len(
            [
                item for item in results
                if bool(
                    (
                        item.get("terminal_resume_overlay_after_settle")
                        or item.get("terminal_resume_overlay")
                        or {}
                    ).get("visible")
                )
                and (
                    (
                        item.get("terminal_resume_overlay_after_settle")
                        or item.get("terminal_resume_overlay")
                        or {}
                    ).get("kind")
                    or ""
                ).lower()
                != "chip"
            ]
        ),
        "overlay_resolve_failures": len(
            [item for item in results if not item.get("overlay_resolved_within_budget")]
        ),
        "failure_card_failures": len(
            [
                item for item in results
                if overlay_reports_failure(
                    item.get("terminal_resume_overlay_after_settle")
                    or item.get("terminal_resume_overlay")
                    or {}
                )
            ]
        ),
        "terminal_error_failures": len(
            [item for item in results if terminal_text_looks_like_error(item.get("terminal_text_sample") or "")]
        ),
        "terminal_surface_problem_failures": len(
            [item for item in results if (item.get("terminal_surface") or {}).get("problem")]
        ),
        "terminal_open_attempt_failure_latches": len(
            [
                item
                for item in results
                if ((item.get("terminal_open_attempt") or {}).get("latched_failure_reason") or "").strip()
            ]
        ),
        "results": results,
    }
    elapsed_values = [float(item["elapsed_s"]) for item in results if item.get("elapsed_s") is not None]
    summary["latency_stats_s"] = {
        "min": min(elapsed_values) if elapsed_values else None,
        "mean": statistics.fmean(elapsed_values) if elapsed_values else None,
        "median": statistics.median(elapsed_values) if elapsed_values else None,
        "p95": percentile(elapsed_values, 0.95),
        "max": max(elapsed_values) if elapsed_values else None,
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
        and summary["shell_prompt_failures"] == 0
        and summary["transcript_browser_failures"] == 0
        and summary["launcher_boilerplate_failures"] == 0
        and summary["terminal_error_failures"] == 0
        and summary["terminal_surface_problem_failures"] == 0
        and summary["terminal_open_attempt_failure_latches"] == 0
        and summary["overlay_copy_failures"] == 0
        and summary["overlay_visible_failures"] == 0
        and summary["overlay_resolve_failures"] == 0
        and summary["failure_card_failures"] == 0
    ) else 1


if __name__ == "__main__":
    raise SystemExit(main())
