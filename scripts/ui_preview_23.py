#!/usr/bin/env python3
import argparse
import json
import os
import random
import shlex
import struct
import subprocess
import time
from pathlib import Path


FORBIDDEN_PREVIEW_MARKERS = (
    "<INSTRUCTIONS>",
    "<permissions instructions>",
    "collaboration mode:",
    "request_user_input",
    "environment_context",
    "filesystem sandboxing",
    "<instructions>",
    "<cwd>",
    "<shell>",
    "<approval_policy>",
    "<sandbox_mode>",
    "<network_access>",
    "</collaboration_mode>",
    "you are now in default mode.",
    "agents.md instructions for",
    "<turn_aborted>",
    "approvals are your mechanism to get user consent",
    "approval_policy is",
    "danger-full-access",
    "non-interactive mode where you may never ask the user for approval",
)

FORBIDDEN_PREVIEW_ENTRY_MARKERS = (
    "PRIMARY USER GOALS:",
    "RECENT SUBSTANTIVE TURNS:",
    "RECENT CONTEXT:",
    "SERVER NOTES:",
    "<instructions>",
    "<cwd>",
    "<shell>",
    "<approval_policy>",
    "<sandbox_mode>",
    "<network_access>",
    "</collaboration_mode>",
    "you are now in default mode.",
    "agents.md instructions for",
    "any previous instructions for other modes",
    "default mode you should strongly prefer",
    "<turn_aborted>",
    "approvals are your mechanism to get user consent",
    "approval_policy is",
    "danger-full-access",
    "non-interactive mode where you may never ask the user for approval",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Open 23 previewable rows, verify viewport/titlebar parity, and capture "
            "viewport-only preview screenshots."
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
    parser.add_argument("--out-dir", default="/tmp/yggterm-preview-23")
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


def app_state(host: str, binary: str, timeout_ms: int) -> dict:
    payload = run_json(host, f"{quote(binary)} server app state --timeout-ms {timeout_ms}")
    return payload.get("data") or {}


def app_rows(host: str, binary: str, timeout_ms: int) -> dict:
    payload = run_json(host, f"{quote(binary)} server app rows --timeout-ms {timeout_ms}")
    return payload.get("data") or {}


def app_open(host: str, binary: str, session_path: str, timeout_ms: int) -> dict:
    command = (
        f"{quote(binary)} server app open {json.dumps(session_path)} "
        f"--view preview --timeout-ms {timeout_ms}"
    )
    return run_json(host, command)


def app_expand(host: str, binary: str, row_path: str, expanded: bool, timeout_ms: int) -> dict:
    action = "expand" if expanded else "collapse"
    command = f"{quote(binary)} server app {action} {json.dumps(row_path)} --timeout-ms {timeout_ms}"
    return run_json(host, command)


def app_screenshot_preview(host: str, binary: str, output_path: Path, timeout_ms: int) -> dict:
    command = (
        f"{quote(binary)} server app screenshot --target preview_viewport {quote(str(output_path))} "
        f"--timeout-ms {timeout_ms}"
    )
    return run_json(host, command)


def app_scroll_preview(
    host: str,
    binary: str,
    *,
    timeout_ms: int,
    top_px: float | None = None,
    ratio: float | None = None,
) -> dict:
    command = [quote(binary), "server", "app", "preview", "scroll", "--timeout-ms", str(timeout_ms)]
    if top_px is not None:
        command.extend(["--top", str(top_px)])
    if ratio is not None:
        command.extend(["--ratio", str(ratio)])
    return run_json(host, " ".join(command))


def canonical_session_path(session_path: str | None) -> str | None:
    if session_path is None:
        return None
    if session_path.startswith("local::"):
        return f"local://{session_path[len('local::'):]}"
    return session_path


def latest_window_spawn_event_for_pid(host: str, pid: int, start_ms: int) -> dict | None:
    if host != "local":
        return None
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


def kill_local_clients(binary: str) -> None:
    binary_path = str(Path(binary).resolve())
    instances_root = Path.home() / ".yggterm" / "client-instances"
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
    run_process(["bash", "-lc", f"pkill -f {json.dumps(binary_path)} || true"], check=False)
    run_process(["bash", "-lc", "pkill -f 'yggterm server daemon' || true"], check=False)


def launch_local_client(binary: str, timeout_s: float = 4.0) -> tuple[subprocess.Popen, dict]:
    binary_path = str(Path(binary).resolve())
    kill_local_clients(binary_path)
    env = os.environ.copy()
    env.setdefault("DISPLAY", ":10.0")
    env["YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF"] = "1"
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
        dom = state.get("dom") or {}
        if not window.get("visible"):
            raise RuntimeError("window not visible")
        if shell.get("needs_initial_server_sync"):
            raise RuntimeError("initial server sync still in progress")
        if dom.get("shell_root_count") != 1:
            raise RuntimeError(f"shell_root_count={dom.get('shell_root_count')}")
        if dom.get("sidebar_count") != 1:
            raise RuntimeError(f"sidebar_count={dom.get('sidebar_count')}")
        if dom.get("titlebar_count") != 1:
            raise RuntimeError(f"titlebar_count={dom.get('titlebar_count')}")
        if dom.get("main_surface_count") != 1:
            raise RuntimeError(f"main_surface_count={dom.get('main_surface_count')}")
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


def preview_ready(state: dict, session_path: str) -> bool:
    viewport = state.get("viewport") or {}
    preview = viewport.get("preview") or {}
    return (
        viewport.get("active_view_mode") == "Rendered"
        and viewport.get("active_session_path") == session_path
        and bool(viewport.get("ready"))
        and (preview.get("visible_block_count") or 0) > 0
    )


def normalize_preview_entry_text(value: str) -> str:
    normalized = (value or "")
    for marker in ("`", "**", "__", "*"):
        normalized = normalized.replace(marker, "")
    return " ".join(normalized.split()).strip().lower()


def preview_entry_body_text(value: str) -> str:
    lines = [(line or "").strip() for line in (value or "").splitlines()]
    lines = [line for line in lines if line]
    while lines:
        first = lines[0]
        upper = first.upper()
        if upper == "COLLAPSE" or upper == "EXPAND":
            lines.pop(0)
            continue
        if any(ch.isdigit() for ch in first) and (
            "UTC" in upper or "AM" in upper or "PM" in upper
        ):
            lines.pop(0)
            continue
        break
    while lines and lines[0].upper() in {"COLLAPSE", "EXPAND"}:
        lines.pop(0)
    return "\n".join(lines)


def normalize_preview_entry_body_text(value: str) -> str:
    return normalize_preview_entry_text(preview_entry_body_text(value))


def preview_semantic_issues(state: dict) -> list[str]:
    preview = ((state.get("viewport") or {}).get("preview") or {})
    entries = preview.get("visible_entries") or []
    rendered_sections = preview.get("rendered_sections") or []
    return preview_semantic_issues_for_entries(entries, rendered_sections)


def preview_semantic_issues_for_entries(entries: list[dict], rendered_sections: list[dict] | None = None) -> list[str]:
    issues = []
    normalized_entries = []
    for index, entry in enumerate(entries):
        text = (entry.get("text") or "").strip()
        tone = (entry.get("tone") or "").strip()
        normalized = normalize_preview_entry_body_text(text)
        if not normalized:
            issues.append(f"entry[{index}] is empty")
            continue
        if "<image name=[" in normalized or "</image>" in normalized:
            issues.append(f"entry[{index}] still shows raw image markup")
        for marker in FORBIDDEN_PREVIEW_ENTRY_MARKERS:
            if marker.lower() in normalized:
                issues.append(f"entry[{index}] still shows scaffold marker {marker}")
        normalized_entries.append((index, tone, normalized))

    for prev, curr in zip(normalized_entries, normalized_entries[1:]):
        prev_ix, prev_tone, prev_text = prev
        curr_ix, curr_tone, curr_text = curr
        if len(prev_text) < 20 or len(curr_text) < 20:
            continue
        if prev_text == curr_text:
            issues.append(
                f"adjacent duplicate preview text at entries {prev_ix} ({prev_tone}) and {curr_ix} ({curr_tone})"
            )

    entry_texts = {text for _, _, text in normalized_entries if text}
    for section_index, section in enumerate(rendered_sections or []):
        title = (section.get("title") or "").strip()
        for line_index, line in enumerate(section.get("lines") or []):
            normalized = normalize_preview_entry_text(line)
            if not normalized:
                continue
            if "<image name=[" in normalized or "</image>" in normalized:
                issues.append(
                    f"rendered section {section_index}:{line_index} still shows raw image markup"
                )
            if normalized in entry_texts:
                issues.append(
                    f"rendered section {section_index}:{line_index} duplicates visible transcript text"
                )
            if "goal" in title.lower() and normalized in entry_texts:
                issues.append(
                    f"goal section {section_index}:{line_index} duplicates a visible transcript turn"
                )

    return issues


def load_server_state() -> dict:
    path = Path.home() / ".yggterm" / "server-state.json"
    if not path.exists():
        return {}
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except Exception:
        return {}


def session_source_record(server_state: dict, session_path: str) -> dict | None:
    for machine in server_state.get("remote_machines") or []:
        for session in machine.get("sessions") or []:
            if session.get("session_path") == session_path:
                return {
                    "storage_path": session.get("storage_path") or "",
                    "ssh_target": machine.get("ssh_target") or machine.get("machine_key") or "",
                    "prefix": machine.get("prefix"),
                    "machine_key": machine.get("machine_key"),
                }
    for collection in ("local_sessions",):
        for session in server_state.get(collection) or []:
            if session.get("session_path") == session_path:
                return {
                    "storage_path": session.get("storage_path") or "",
                    "ssh_target": "",
                    "prefix": None,
                    "machine_key": "local",
                }
    return None


def expected_preview_turns_for_session(server_state: dict, session_path: str, binary: str) -> list[dict]:
    record = session_source_record(server_state, session_path)
    if not record:
        return []
    storage_path = (record.get("storage_path") or "").strip()
    if not storage_path:
        return []

    if record.get("ssh_target"):
        remote_bin = "~/.yggterm/bin/yggterm"
        payload = run_json(
            "local",
            " ".join(
                [
                    "ssh",
                    "-o",
                    "BatchMode=yes",
                    "-o",
                    "ConnectTimeout=8",
                    quote(record["ssh_target"]),
                    quote(f"{remote_bin} server remote preview {quote(storage_path)}"),
                ]
            ),
        )
    else:
        payload = run_json(
            "local",
            f"{quote(binary)} server remote preview {quote(storage_path)}",
        )

    preview = payload.get("preview") or {}
    turns = []
    for block in preview.get("blocks") or []:
        tone = (block.get("tone") or "").strip().lower()
        text = " ".join((block.get("lines") or [])).strip()
        if tone and text:
            turns.append({"tone": tone, "text": text})
    return turns


def preview_entry_matches_expected(entry: dict, expected_turns: list[dict]) -> bool:
    entry_tone = (entry.get("tone") or "").strip().lower()
    entry_text = normalize_preview_entry_body_text(entry.get("text") or "")
    if not entry_tone or not entry_text:
        return False
    for turn in expected_turns:
        if (turn.get("tone") or "").strip().lower() != entry_tone:
            continue
        turn_text = normalize_preview_entry_text(turn.get("text") or "")
        if not turn_text:
            continue
        if (
            turn_text == entry_text
            or turn_text.startswith(entry_text)
            or entry_text.startswith(turn_text)
        ):
            return True
    return False


def preview_expected_turn_issues_for_entries(
    entries: list[dict], expected_turns: list[dict]
) -> list[str]:
    if not expected_turns:
        return []
    issues = []
    for index, entry in enumerate(entries):
        if not preview_entry_matches_expected(entry, expected_turns):
            issues.append(
                f"entry[{index}] does not match expected preview payload turn for tone={entry.get('tone')}"
            )
    return issues


def preview_expected_turn_issues(state: dict, expected_turns: list[dict]) -> list[str]:
    entries = (((state.get("viewport") or {}).get("preview") or {}).get("visible_entries") or [])
    return preview_expected_turn_issues_for_entries(entries, expected_turns)


def read_png_size(path: Path) -> tuple[int, int]:
    with path.open("rb") as handle:
        header = handle.read(24)
    if len(header) < 24 or header[:8] != b"\x89PNG\r\n\x1a\n":
        raise RuntimeError(f"{path} is not a PNG")
    return struct.unpack(">II", header[16:24])


def collect_preview_targets(rows_payload: dict, rng: random.Random, count: int) -> list[dict]:
    candidates = []
    seen = set()
    for row in rows_payload.get("rows") or []:
        kind = row.get("kind")
        session_path = canonical_session_path(row.get("full_path"))
        if kind != "Session" or not session_path or session_path in seen:
            continue
        candidates.append(
            {
                "full_path": session_path,
                "label": row.get("label") or session_path,
                "kind": kind,
            }
        )
        seen.add(session_path)
    if len(candidates) > count:
        return rng.sample(candidates, count)
    return candidates


def expand_groups_until_target(host: str, binary: str, timeout_ms: int, target_count: int) -> dict:
    last = app_rows(host, binary, timeout_ms)
    for _ in range(8):
        if len(collect_preview_targets(last, random.Random(23), target_count)) >= target_count:
            return last
        collapsed = [
            row for row in (last.get("rows") or [])
            if row.get("kind") == "Group" and not row.get("expanded") and row.get("full_path")
        ]
        if not collapsed:
            return last
        for row in collapsed:
            try:
                app_expand(host, binary, row["full_path"], True, min(timeout_ms, 1500))
            except Exception:
                continue
        time.sleep(0.15)
        last = app_rows(host, binary, timeout_ms)
    return last


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

    wait_for_window(args.host, args.bin, args.timeout_ms)
    server_state = load_server_state() if args.host == "local" else {}
    rows_payload = expand_groups_until_target(args.host, args.bin, args.timeout_ms, args.count)
    targets = collect_preview_targets(rows_payload, random.Random(args.seed), args.count)
    if not targets:
        raise RuntimeError("no preview targets available")

    results = []
    for index, target in enumerate(targets):
        session_path = target["full_path"]
        entry = {
            "path": session_path,
            "label": target["label"],
            "kind": target["kind"],
        }
        expected_turns = expected_preview_turns_for_session(server_state, session_path, args.bin)
        try:
            entry["open"] = app_open(args.host, args.bin, session_path, args.timeout_ms)
            elapsed, state = wait_until(
                f"preview {session_path}",
                args.ready_budget,
                args.poll,
                lambda: _require_preview_ready(
                    app_state(args.host, args.bin, args.timeout_ms),
                    session_path,
                    expected_turns,
                ),
            )
            screenshot_path = out_dir / f"preview-{index:02d}.png"
            entry["screenshot"] = app_screenshot_preview(args.host, args.bin, screenshot_path, args.timeout_ms)
            screenshot_dom = (((entry["screenshot"].get("data") or {}).get("dom")) or {})
            preview = ((state.get("viewport") or {}).get("preview") or {})
            screenshot_entries = screenshot_dom.get("preview_visible_entries") or []
            png_width, png_height = read_png_size(screenshot_path)
            viewport_rect = (screenshot_dom.get("preview_viewport_rect") or preview.get("viewport_rect") or {})
            window = (screenshot_dom.get("preview_window") or preview.get("window") or {})
            font_family = ((screenshot_dom.get("preview_font_family") or preview.get("font_family") or "")).strip()
            text_sample = ((screenshot_dom.get("preview_text_sample") or preview.get("text_sample") or "")).strip()
            timestamp_labels = screenshot_dom.get("preview_timestamp_labels") or preview.get("timestamp_labels") or []
            raw_timestamps = [
                value
                for value in (screenshot_dom.get("preview_raw_timestamps") or [])
                if any(ch.isdigit() for ch in value)
            ]
            titlebar_ok = titlebar_matches_viewport(state)
            viewport_size_ok = (
                abs(png_width - int(round(viewport_rect.get("width") or 0))) <= 4
                and abs(png_height - int(round(viewport_rect.get("height") or 0))) <= 4
            )
            forbidden_hits = [
                marker for marker in FORBIDDEN_PREVIEW_MARKERS if marker.lower() in text_sample.lower()
            ]
            semantic_issues = preview_semantic_issues(state) + preview_expected_turn_issues(
                state, expected_turns
            )
            screenshot_rendered_sections = screenshot_dom.get("preview_rendered_sections") or preview.get("rendered_sections") or []
            screenshot_semantic_issues = preview_semantic_issues_for_entries(
                screenshot_entries,
                screenshot_rendered_sections,
            )
            screenshot_expected_issues = preview_expected_turn_issues_for_entries(
                screenshot_entries,
                expected_turns,
            )
            screenshot_forbidden_hits = [
                marker
                for marker in FORBIDDEN_PREVIEW_MARKERS
                if marker.lower()
                in " ".join((entry.get("text") or "") for entry in screenshot_entries).lower()
            ]
            entry.update(
                {
                    "elapsed_s": round(elapsed, 3),
                    "within_budget": elapsed <= args.ready_budget,
                    "state_dump": write_json(out_dir / f"preview-{index:02d}.json", state),
                    "preview_png": str(screenshot_path),
                    "png_width": png_width,
                    "png_height": png_height,
                    "viewport_rect": viewport_rect,
                    "viewport_size_matches": viewport_size_ok,
                    "visible_block_count": preview.get("visible_block_count"),
                    "visible_block_ids": preview.get("visible_block_ids"),
                    "font_family": font_family,
                    "font_family_ok": any(token in font_family.lower() for token in ("serif", "georgia", "iowan")),
                    "timestamp_labels": timestamp_labels,
                    "raw_timestamps": raw_timestamps,
                    "timestamp_labels_ok": (len(raw_timestamps) == 0) or (len(timestamp_labels) > 0),
                    "titlebar_matches_viewport": titlebar_ok,
                    "semantic_issues": list(
                        dict.fromkeys(
                            semantic_issues
                            + screenshot_semantic_issues
                            + screenshot_expected_issues
                        )
                    ),
                    "forbidden_hits": list(dict.fromkeys(forbidden_hits + screenshot_forbidden_hits)),
                    "preview_window": window,
                    "preview_window_ok": (
                        isinstance(window.get("start_index"), int)
                        and isinstance(window.get("end_index"), int)
                        and window.get("end_index", 0) >= window.get("start_index", 0)
                        and window.get("total_count", 0) >= window.get("end_index", 0)
                    ),
                }
            )
        except Exception as error:  # noqa: BLE001
            state = {}
            try:
                state = app_state(args.host, args.bin, args.timeout_ms)
            except Exception:
                pass
            entry["error"] = str(error)
            entry["state_dump"] = write_json(out_dir / f"preview-{index:02d}-failure.json", state)
            entry["within_budget"] = False
        results.append(entry)

    summary = {
        "host": args.host,
        "count": args.count,
        "executed_count": len(results),
        "ready_budget_s": args.ready_budget,
        "window_spawn_elapsed_ms": ((launch_event or {}).get("payload") or {}).get("elapsed_ms"),
        "open_failures": len([item for item in results if item.get("error")]),
        "ready_failures": len([item for item in results if not item.get("within_budget")]),
        "titlebar_failures": len([item for item in results if not item.get("titlebar_matches_viewport", False)]),
        "viewport_size_failures": len([item for item in results if not item.get("viewport_size_matches", False)]),
        "font_failures": len([item for item in results if not item.get("font_family_ok", False)]),
        "timestamp_failures": len([item for item in results if not item.get("timestamp_labels_ok", False)]),
        "semantic_failures": len([item for item in results if item.get("semantic_issues")]),
        "forbidden_content_failures": len([item for item in results if item.get("forbidden_hits")]),
        "virtual_window_failures": len([item for item in results if not item.get("preview_window_ok", False)]),
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
        summary["open_failures"] == 0
        and summary["ready_failures"] == 0
        and summary["titlebar_failures"] == 0
        and summary["viewport_size_failures"] == 0
        and summary["font_failures"] == 0
        and summary["timestamp_failures"] == 0
        and summary["semantic_failures"] == 0
        and summary["forbidden_content_failures"] == 0
        and summary["virtual_window_failures"] == 0
    ) else 1


def _require_preview_ready(
    state: dict, session_path: str, expected_turns: list[dict] | None = None
) -> dict:
    dom = state.get("dom") or {}
    for key in ("shell_root_count", "sidebar_count", "titlebar_count", "main_surface_count"):
        if dom.get(key) != 1:
            raise RuntimeError(f"{key}={dom.get(key)}")
    if not preview_ready(state, session_path):
        viewport = state.get("viewport") or {}
        raise RuntimeError(viewport.get("reason") or "preview viewport not ready")
    if not titlebar_matches_viewport(state):
        raise RuntimeError("titlebar not in sync with preview viewport")
    expected_issues = preview_expected_turn_issues(state, expected_turns or [])
    if expected_issues:
        raise RuntimeError(expected_issues[0])
    return state


if __name__ == "__main__":
    raise SystemExit(main())
