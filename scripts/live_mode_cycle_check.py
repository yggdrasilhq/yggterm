#!/usr/bin/env python3
import argparse
import json
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path


def _default_live_host() -> str | None:
    # Resolve the live host from the gitignored config so the private SSH alias is
    # never baked into this public repo. Override with --host. (.agents/config/ is
    # in .gitignore; the file holds one line, e.g. the host alias.)
    cfg = Path(__file__).resolve().parents[1] / ".agents" / "config" / "live-host"
    try:
        value = cfg.read_text(encoding="utf-8").strip()
        return value or None
    except OSError:
        return None


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Drive a running Yggterm app over SSH app-control and time preview/terminal readiness."
    )
    parser.add_argument(
        "--host",
        default=_default_live_host(),
        help="Live host SSH alias; defaults to .agents/config/live-host (gitignored).",
    )
    parser.add_argument("--bin", default="~/.local/bin/yggterm")
    parser.add_argument("--out-dir", default="~/.tmp/yggterm/live-mode-cycle")
    parser.add_argument(
        "--remote-tmp-dir",
        default=None,
        help="Remote directory for screenshots; defaults to $HOME/.tmp/yggterm on the target.",
    )
    parser.add_argument("--timeout", type=float, default=45.0)
    parser.add_argument("--poll", type=float, default=0.75)
    parser.add_argument(
        "--all-live",
        action="store_true",
        help="Cycle every top-level Live Sessions row instead of only the active session.",
    )
    parser.add_argument("--limit", type=int, default=0, help="Optional cap for --all-live.")
    parser.add_argument(
        "--no-screenshots",
        action="store_true",
        help="Skip screenshot capture; useful for fast incident triage.",
    )
    return parser.parse_args()


@dataclass
class CommandFailure(Exception):
    command: str
    returncode: int | None
    stdout: str
    stderr: str
    timed_out: bool = False

    def __str__(self) -> str:
        status = "timed out" if self.timed_out else f"exit {self.returncode}"
        stdout = self.stdout.strip()
        stderr = self.stderr.strip()
        pieces = [f"{self.command} failed ({status})"]
        if stdout:
            pieces.append(f"stdout: {stdout[-1200:]}")
        if stderr:
            pieces.append(f"stderr: {stderr[-1200:]}")
        return " | ".join(pieces)


def run_ssh(
    host: str,
    command: str,
    check: bool = True,
    timeout_s: float | None = None,
) -> subprocess.CompletedProcess:
    full_command = ["ssh", host, command]
    try:
        result = subprocess.run(
            full_command,
            check=False,
            text=True,
            capture_output=True,
            timeout=timeout_s,
        )
    except subprocess.TimeoutExpired as error:
        if not check:
            raise
        raise CommandFailure(
            " ".join(full_command),
            None,
            error.stdout or "",
            error.stderr or "",
            timed_out=True,
        ) from error
    if check and result.returncode != 0:
        raise CommandFailure(
            " ".join(full_command),
            result.returncode,
            result.stdout,
            result.stderr,
        )
    return result


def app_control_timeout_s(timeout_ms: int) -> float:
    return max(8.0, (timeout_ms / 1000.0) + 5.0)


def run_json(host: str, command: str, timeout_s: float | None = None) -> dict:
    result = run_ssh(host, command, timeout_s=timeout_s)
    return json.loads(result.stdout)


def app_state(host: str, binary: str, timeout_ms: int = 15000) -> dict:
    response = run_json(
        host,
        f"{binary} server app state --timeout-ms {timeout_ms}",
        timeout_s=app_control_timeout_s(timeout_ms),
    )
    return response.get("data") or {}


def app_rows(host: str, binary: str, timeout_ms: int = 15000) -> dict:
    response = run_json(
        host,
        f"{binary} server app rows --timeout-ms {timeout_ms}",
        timeout_s=app_control_timeout_s(timeout_ms),
    )
    return response.get("data") or {}


def open_view(host: str, binary: str, session_path: str, view: str, timeout_ms: int = 15000) -> dict:
    return run_json(
        host,
        f"{binary} server app open {json.dumps(session_path)} --view {view} --timeout-ms {timeout_ms}",
        timeout_s=app_control_timeout_s(timeout_ms),
    )


def capture(host: str, binary: str, remote_path: str, local_path: Path, timeout_ms: int = 15000) -> None:
    run_ssh(
        host,
        f"mkdir -p {json.dumps(str(Path(remote_path).parent))} && "
        f"{binary} server app screenshot {json.dumps(remote_path)} --timeout-ms {timeout_ms}",
        timeout_s=app_control_timeout_s(timeout_ms),
    )
    subprocess.run(["scp", f"{host}:{remote_path}", str(local_path)], check=True)


def wait_until(label: str, timeout_s: float, poll_s: float, predicate):
    start = time.monotonic()
    last_state = None
    last_error = None
    while time.monotonic() - start <= timeout_s:
        try:
            state = predicate()
            last_state = state
            return time.monotonic() - start, state
        except Exception as error:  # noqa: BLE001
            last_error = error
            time.sleep(poll_s)
    if last_error is not None:
        raise RuntimeError(f"{label} timed out after {timeout_s:.1f}s: {last_error}") from last_error
    raise RuntimeError(f"{label} timed out after {timeout_s:.1f}s with no usable state")


def live_rows(rows_payload: dict) -> list[dict]:
    rows = rows_payload.get("rows") or []
    result: list[dict] = []
    seen: set[str] = set()
    for row in rows:
        if row.get("depth") != 1:
            continue
        if row.get("kind") != "Session" or row.get("live_member") is not True:
            continue
        path = row.get("full_path")
        if not isinstance(path, str) or not path or path in seen:
            continue
        seen.add(path)
        result.append(
            {
                "path": path,
                "label": row.get("title") or row.get("display_title") or path.rsplit("/", 1)[-1],
                "keep_alive": row.get("live_keep_alive") is True,
            }
        )
    return result


def shell_settled(state: dict) -> bool:
    shell = state.get("shell") or {}
    attach = shell.get("terminal_attach_in_flight")
    if shell.get("needs_initial_server_sync"):
        return False
    if attach is True or (isinstance(attach, list) and attach):
        return False
    if state.get("active_surface_requests") or []:
        return False
    if state.get("session_view_contract_violations") or []:
        return False
    return True


def runtime_truth_clean_for_live(state: dict) -> bool:
    truth = state.get("runtime_truth") or {}
    if truth.get("live_row_count", 0) > 0 and not truth.get("daemon_runtime_keys"):
        return False
    if truth.get("active_runtime_present") is False:
        return False
    return True


def active_terminal_problem(state: dict) -> str:
    surface = state.get("active_terminal_surface") or {}
    problem = surface.get("problem") or surface.get("geometry_problem") or ""
    if surface.get("content_source") == "empty":
        return "empty terminal content"
    return str(problem)


def preview_ready(state: dict, session_path: str) -> bool:
    if state.get("active_session_path") != session_path:
        return False
    if state.get("active_view_mode") != "Rendered":
        return False
    if not shell_settled(state) or not runtime_truth_clean_for_live(state):
        return False
    dom = state.get("dom") or {}
    text = "\n".join(
        str(value or "")
        for value in (
            dom.get("preview_text_sample"),
            state.get("active_summary"),
            state.get("active_precis"),
            state.get("active_title"),
        )
    )
    return (
        dom.get("preview_scroll_count", 0) > 0
        or len(text.strip()) > 0
    )


def terminal_ready(state: dict, session_path: str) -> bool:
    if state.get("active_session_path") != session_path:
        return False
    if state.get("active_view_mode") != "Terminal":
        return False
    if not shell_settled(state) or not runtime_truth_clean_for_live(state):
        return False
    surface_problem = active_terminal_problem(state)
    if surface_problem:
        return False
    dom = state.get("dom") or {}
    truth = state.get("runtime_truth") or {}
    hosts = dom.get("terminal_hosts") or []
    if truth.get("active_host_ready") is False or truth.get("active_host_input_enabled") is False:
        return False
    if dom.get("terminal_host_count", 0) <= 0 or not hosts:
        return False
    sample = (hosts[0].get("text_sample") or "").strip()
    return bool(sample) or hosts[0].get("canvas_count", 0) > 0


def summarize_state(state: dict) -> dict:
    truth = state.get("runtime_truth") or {}
    shell = state.get("shell") or {}
    surface = state.get("active_terminal_surface") or {}
    return {
        "active_session_path": state.get("active_session_path"),
        "active_view_mode": state.get("active_view_mode"),
        "ready": state.get("ready"),
        "needs_initial_server_sync": shell.get("needs_initial_server_sync"),
        "terminal_attach_in_flight": shell.get("terminal_attach_in_flight"),
        "active_surface_requests": len(state.get("active_surface_requests") or []),
        "contract_violations": state.get("session_view_contract_violations") or [],
        "live_row_count": truth.get("live_row_count"),
        "daemon_runtime_keys": truth.get("daemon_runtime_keys"),
        "active_runtime_present": truth.get("active_runtime_present"),
        "active_host_ready": truth.get("active_host_ready"),
        "active_host_input_enabled": truth.get("active_host_input_enabled"),
        "active_host_raw_input_enabled": truth.get("active_host_raw_input_enabled"),
        "active_host_effective_input_focus": truth.get("active_host_effective_input_focus"),
        "terminal_problem": surface.get("problem"),
        "terminal_input_enabled": surface.get("input_enabled"),
        "terminal_raw_input_enabled": surface.get("raw_input_enabled"),
        "terminal_effective_input_focus": surface.get("effective_input_focus"),
        "terminal_content_source": surface.get("content_source"),
        "terminal_settled_kind": state.get("terminal_settled_kind"),
        "terminal_open_attempt": {
            key: (state.get("terminal_open_attempt") or {}).get(key)
            for key in (
                "state",
                "request_to_ready_ms",
                "last_observed_reason",
                "last_surface_problem",
                "observations",
            )
        },
    }


def run_cycle_for_session(
    host: str,
    binary: str,
    out_dir: Path,
    remote_tmp_dir: str,
    session_path: str,
    label: str,
    timeout_s: float,
    poll_s: float,
    screenshots: bool,
) -> tuple[bool, dict]:
    started = time.monotonic()
    steps: list[dict] = []
    ok = True

    for index, view in enumerate(("terminal", "preview", "terminal")):
        step: dict = {"view": view, "index": index}
        open_started = time.monotonic()
        try:
            open_view(host, binary, session_path, view)
            step["open_elapsed_s"] = round(time.monotonic() - open_started, 3)
            ready_pred = preview_ready if view == "preview" else terminal_ready
            elapsed, state = wait_until(
                f"{label} {view} readiness",
                timeout_s,
                poll_s,
                lambda: _require_ready(app_state(host, binary), lambda current: ready_pred(current, session_path)),
            )
            step["ready_elapsed_s"] = round(elapsed, 3)
            step["state"] = summarize_state(state)
            if screenshots:
                remote = f"{remote_tmp_dir}/yggterm-mode-cycle-{int(time.time())}-{index}.png"
                local = out_dir / f"{safe_name(label)}-{index}-{view}.png"
                capture(host, binary, remote, local)
                step["screenshot"] = str(local)
        except Exception as error:  # noqa: BLE001
            ok = False
            step["error"] = str(error)
            try:
                step["state"] = summarize_state(app_state(host, binary, 6000))
            except Exception as state_error:  # noqa: BLE001
                step["state_error"] = str(state_error)
        steps.append(step)

    summary = {
        "label": label,
        "session_path": session_path,
        "ok": ok,
        "elapsed_total_s": round(time.monotonic() - started, 3),
        "steps": steps,
    }
    return ok, summary


def safe_name(value: str) -> str:
    cleaned = "".join(ch if ch.isalnum() else "-" for ch in value.lower()).strip("-")
    return cleaned[:80] or "session"


def main() -> int:
    args = parse_args()
    if not args.host:
        print(
            "error: no live host — pass --host, or write the SSH alias to "
            ".agents/config/live-host",
            file=sys.stderr,
        )
        return 2
    out_dir = Path(args.out_dir).expanduser()
    out_dir.mkdir(parents=True, exist_ok=True)
    binary = args.bin
    host = args.host
    remote_tmp_dir = args.remote_tmp_dir
    if not remote_tmp_dir:
        remote_tmp_dir = run_ssh(
            host,
            'printf "%s" "$HOME/.tmp/yggterm"',
            timeout_s=8,
        ).stdout.strip()
    run_ssh(host, f"mkdir -p {json.dumps(remote_tmp_dir)}", timeout_s=8)

    baseline = app_state(host, binary)
    if args.all_live:
        rows = live_rows(app_rows(host, binary))
        if args.limit > 0:
            rows = rows[: args.limit]
        if not rows:
            raise RuntimeError("no top-level live sessions in running app rows")
    else:
        active_path = baseline.get("active_session_path")
        if not active_path:
            raise RuntimeError("no active session path in running app state")
        rows = [{"path": active_path, "label": baseline.get("active_title") or "active"}]

    if not args.no_screenshots:
        remote_initial = f"{remote_tmp_dir}/yggterm-mode-cycle-initial-{int(time.time())}.png"
        capture(host, binary, remote_initial, out_dir / "initial.png")

    summaries: list[dict] = []
    passed = True
    started = time.monotonic()
    for row in rows:
        ok, row_summary = run_cycle_for_session(
            host,
            binary,
            out_dir,
            remote_tmp_dir,
            row["path"],
            row["label"],
            args.timeout,
            args.poll,
            not args.no_screenshots,
        )
        summaries.append(row_summary)
        passed = passed and ok

    summary = {
        "host": host,
        "started_at_epoch_s": time.time(),
        "elapsed_total_s": round(time.monotonic() - started, 3),
        "all_live": args.all_live,
        "session_count": len(rows),
        "passed": passed,
        "baseline": summarize_state(baseline),
        "sessions": summaries,
    }
    summary_path = out_dir / "summary.json"
    summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(summary_path)
    print(json.dumps(summary, indent=2))
    return 0 if passed else 1


def _require_ready(state: dict, ready_pred):
    if not ready_pred(state):
        raise RuntimeError("not ready yet")
    return state


if __name__ == "__main__":
    raise SystemExit(main())
