#!/usr/bin/env python3
import argparse
import json
import subprocess
import sys
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BIN = ROOT / "target" / "debug" / "yggterm"


def run(*args: str, check: bool = True) -> dict:
    proc = subprocess.run(
        [str(BIN), *args],
        cwd=ROOT,
        text=True,
        capture_output=True,
    )
    if check and proc.returncode != 0:
        raise SystemExit(proc.stderr.strip() or proc.stdout.strip() or f"command failed: {args}")
    text = proc.stdout.strip()
    return json.loads(text) if text else {}


def app_state(pid: int) -> dict:
    return run("server", "app", "state", "--pid", str(pid), "--timeout-ms", "8000")["data"]


def terminal_send(pid: int, session: str, data: str) -> dict:
    return run(
        "server",
        "app",
        "terminal",
        "send",
        "--pid",
        str(pid),
        session,
        "--data",
        data,
        "--timeout-ms",
        "15000",
    )


def terminal_probe_type(
    pid: int,
    session: str,
    data: str,
    *,
    press_enter: bool = False,
    press_ctrl_c: bool = False,
    press_ctrl_e: bool = False,
    press_ctrl_u: bool = False,
) -> dict:
    args = [
        "server",
        "app",
        "terminal",
        "probe-type",
        "--pid",
        str(pid),
        session,
        "--mode",
        "keyboard",
        "--data",
        data,
        "--timeout-ms",
        "15000",
    ]
    if press_enter:
        args.append("--enter")
    if press_ctrl_c:
        args.append("--ctrl-c")
    if press_ctrl_e:
        args.append("--ctrl-e")
    if press_ctrl_u:
        args.append("--ctrl-u")
    response = run(*args)
    return response.get("data") or response


def terminal_hosts(state: dict) -> list[dict]:
    viewport_hosts = ((state.get("viewport") or {}).get("terminal_hosts") or [])
    if isinstance(viewport_hosts, list) and viewport_hosts:
        return viewport_hosts
    dom_hosts = ((state.get("dom") or {}).get("terminal_hosts") or [])
    if isinstance(dom_hosts, list):
        return dom_hosts
    return []


def active_host(state: dict) -> dict:
    hosts = terminal_hosts(state)
    if not hosts:
        return {}
    active_session_path = state.get("active_session_path")
    explicit_matches = [host for host in hosts if host.get("is_active_session_host") is True]
    if explicit_matches:
        return explicit_matches[-1]
    if active_session_path:
        session_matches = [
            host for host in hosts if str(host.get("session_path") or "") == str(active_session_path)
        ]
        if session_matches:
            focused_matches = [
                host for host in session_matches
                if host.get("helper_textarea_focused") is True or host.get("host_has_active_element") is True
            ]
            if focused_matches:
                return focused_matches[-1]
            return session_matches[-1]
    focused_hosts = [
        host for host in hosts
        if host.get("helper_textarea_focused") is True or host.get("host_has_active_element") is True
    ]
    if focused_hosts:
        return focused_hosts[-1]
    return hosts[-1]


def strip_terminal_border(line: str) -> str:
    return line.strip().strip("╭╮╰╯─│ ").strip()


def terminal_chunk_has_codex_prompt_output(data: str) -> bool:
    normalized_lines = [line.strip() for line in str(data or "").splitlines() if line.strip()]
    if not normalized_lines:
        return False
    if len(normalized_lines) > 2 or any(len(line) > 96 for line in normalized_lines):
        return False
    return any(strip_terminal_border(line).startswith("›") for line in normalized_lines)


def host_has_live_codex_prompt(host: dict) -> bool:
    input_ready = host.get("input_enabled") is True or host.get("helper_textarea_focused") is True
    if not input_ready:
        return False
    return any(
        terminal_chunk_has_codex_prompt_output(chunk)
        for chunk in terminal_host_text_chunks(host)
    )


def host_has_shell_status_failure(host: dict) -> bool:
    haystack = terminal_host_text(host)
    recent = haystack[-400:].lower()
    return "bash: /status" in recent or (
        "/status" in recent and "no such file or directory" in recent
    )


def terminal_host_text(host: dict) -> str:
    return "\n".join(terminal_host_text_chunks(host))


def terminal_host_text_chunks(host: dict) -> list[str]:
    samples: list[str] = []
    for key in ("text_sample", "text_tail", "cursor_line_text", "cursor_row_text"):
        value = str(host.get(key) or "").strip()
        if value:
            samples.append(value)
    for row in list(host.get("visible_row_samples_head") or []) + list(host.get("visible_row_samples_tail") or []):
        value = str(row.get("text") or "").strip()
        if value:
            samples.append(value)
    return samples


def host_problem(state: dict) -> str | None:
    viewport = state.get("viewport") or {}
    surface = viewport.get("active_terminal_surface") or {}
    problem = surface.get("problem")
    if isinstance(problem, str) and problem.strip():
        return problem
    host = active_host(state)
    cursor_line_text = str(host.get("cursor_line_text") or "").lower()
    if "shared connection to " in cursor_line_text and " closed" in cursor_line_text:
        return "active terminal host is showing transport/error output"
    return None


def wait_for_terminal_interactive(pid: int, timeout_seconds: float = 15.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        last_state = app_state(pid)
        viewport = last_state.get("viewport") or {}
        host = active_host(last_state)
        if (
            viewport.get("ready") is True
            and viewport.get("interactive") is True
            and host
            and host.get("input_enabled") is True
            and not host_problem(last_state)
        ):
            return last_state
        time.sleep(0.2)
    raise AssertionError(f"terminal did not settle in time: {last_state!r}")


def wait_for_live_codex_prompt(pid: int, timeout_seconds: float = 20.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        last_state = wait_for_terminal_interactive(pid, timeout_seconds=8.0)
        if host_has_live_codex_prompt(active_host(last_state)):
            return last_state
        time.sleep(0.25)
    raise AssertionError(f"live Codex prompt did not become visible in time: {last_state!r}")


def ensure_live_codex_runtime(pid: int, session: str) -> dict:
    current = wait_for_terminal_interactive(pid, timeout_seconds=25.0)
    if host_has_live_codex_prompt(active_host(current)):
        return {
            "action": "noop",
            "state": current,
        }
    prepare = terminal_probe_type(
        pid,
        session,
        "",
        press_ctrl_c=True,
        press_ctrl_e=True,
        press_ctrl_u=True,
    )
    time.sleep(0.4)
    launch = terminal_probe_type(pid, session, "codex", press_enter=True)
    state = wait_for_live_codex_prompt(pid, timeout_seconds=30.0)
    return {
        "action": "launch_codex",
        "prepare_probe": prepare,
        "launch_probe": launch,
        "state": state,
    }


def clear_prompt(pid: int, session: str) -> dict:
    return terminal_probe_type(pid, session, "", press_ctrl_e=True, press_ctrl_u=True)


def run_status_probe(pid: int, session: str) -> dict:
    return terminal_probe_type(pid, session, "/status", press_enter=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--pid", type=int, required=True)
    parser.add_argument("--session", required=True)
    parser.add_argument("--out", default="/tmp/terminal-status-smoke")
    parser.add_argument("--reopen", action="store_true")
    args = parser.parse_args()

    out_dir = Path(args.out)
    out_dir.mkdir(parents=True, exist_ok=True)

    if args.reopen:
        run(
            "server",
            "app",
            "open",
            "--pid",
            str(args.pid),
            args.session,
            "--view",
            "terminal",
            "--timeout-ms",
            "20000",
        )
    ensure = ensure_live_codex_runtime(args.pid, args.session)
    with (out_dir / "ensure-live-codex.json").open("w") as fh:
        json.dump(ensure, fh, indent=2)
    before = ensure["state"]
    with (out_dir / "before.json").open("w") as fh:
        json.dump(before, fh, indent=2)
    run(
        "server",
        "app",
        "screenshot",
        "--pid",
        str(args.pid),
        str(out_dir / "before.png"),
        "--timeout-ms",
        "8000",
    )

    clear = clear_prompt(args.pid, args.session)
    with (out_dir / "clear.json").open("w") as fh:
        json.dump(clear, fh, indent=2)
    time.sleep(0.4)
    probe = run_status_probe(args.pid, args.session)
    with (out_dir / "probe.json").open("w") as fh:
        json.dump(probe, fh, indent=2)

    time.sleep(1.0)
    after = app_state(args.pid)
    after_host = active_host(after)
    after_terminal_text = terminal_host_text(after_host)
    if (
        "/status" not in after_terminal_text
        and "OpenAI Codex" not in after_terminal_text
        and "Session:" not in after_terminal_text
    ):
        after = wait_for_terminal_interactive(args.pid, timeout_seconds=20.0)
        retry_probe = run_status_probe(args.pid, args.session)
        with (out_dir / "retry-probe.json").open("w") as fh:
            json.dump(retry_probe, fh, indent=2)
        time.sleep(1.0)
        after = app_state(args.pid)
    with (out_dir / "after.json").open("w") as fh:
        json.dump(after, fh, indent=2)
    run(
        "server",
        "app",
        "screenshot",
        "--pid",
        str(args.pid),
        str(out_dir / "after.png"),
        "--timeout-ms",
        "8000",
    )

    select = run(
        "server",
        "app",
        "terminal",
        "probe-select",
        "--pid",
        str(args.pid),
        args.session,
        "--timeout-ms",
        "8000",
    )
    with (out_dir / "select.json").open("w") as fh:
        json.dump(select, fh, indent=2)

    viewport = after.get("viewport") or {}
    host = active_host(after)
    notifications = (after.get("shell") or {}).get("notifications") or []
    text_sample = host.get("text_sample") or ""
    text_tail = host.get("text_tail") or ""
    cursor_line_text = host.get("cursor_line_text") or ""
    terminal_text = terminal_host_text(host)
    selected_text_length = (((select.get("data") or {}).get("selected_text_length")) or 0)

    if viewport.get("ready") is not True or viewport.get("interactive") is not True:
        raise AssertionError(f"terminal not interactive after /status: {viewport!r}")
    if host_has_shell_status_failure(host):
        raise AssertionError("Codex status probe typed /status into the shell instead of the live Codex runtime")
    if notifications:
        raise AssertionError(f"notifications still visible after /status: {notifications!r}")
    if host.get("low_contrast_span_count") not in (0, None):
        raise AssertionError(
            f"low contrast spans remain visible: {host.get('low_contrast_span_count')!r}"
        )
    raw_cursor_visible = (host.get("cursor_sample_rect") or {}).get("width", 0) not in (0, None)
    if not raw_cursor_visible:
        raise AssertionError(
            "no visible native cursor after /status: "
            f"raw={host.get('cursor_sample_rect')!r} hidden={host.get('xterm_cursor_hidden')!r}"
        )
    if "/status" not in terminal_text:
        raise AssertionError("typed /status did not appear in terminal text sample")
    if "OpenAI Codex" not in terminal_text and "Session:" not in terminal_text:
        raise AssertionError("Codex status panel did not appear after /status")
    if selected_text_length <= 0:
        raise AssertionError(f"selection probe did not capture visible text: {select!r}")

    summary = {
        "pid": args.pid,
        "session": args.session,
        "ensure_live_codex_action": ensure.get("action"),
        "ready": viewport.get("ready"),
        "interactive": viewport.get("interactive"),
        "cursor_sample_rect": host.get("cursor_sample_rect"),
        "xterm_cursor_hidden": host.get("xterm_cursor_hidden"),
        "selected_text_length": selected_text_length,
        "selected_contrast": (select.get("data") or {}).get("selected_contrast"),
        "rows_sample_color": host.get("rows_sample_color"),
        "dim_sample_color": host.get("dim_sample_color"),
        "low_contrast_span_count": host.get("low_contrast_span_count"),
        "text_tail": str(text_tail)[-400:],
    }
    with (out_dir / "summary.json").open("w") as fh:
        json.dump(summary, fh, indent=2)
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
