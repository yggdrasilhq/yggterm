#!/usr/bin/env python3
"""Measure Yggterm app-control, UI, and terminal input/drawing latency.

This smoke is intentionally user-visible when pointed at a live profile: terminal
typing probes insert short marker text into the active prompt. Use --clear-after
when it is acceptable to send Ctrl+U after the samples. The first terminal
sample after opening/clearing the viewport is reported as warmup and gets a
separate budget; steady-state samples keep the stricter visible-echo budget.
Use --read-only-drawing for live active sessions where typing is not acceptable.
"""

from __future__ import annotations

import argparse
import json
import shlex
import statistics
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any


DEFAULT_BIN = "~/.local/bin/yggterm"


@dataclass
class CommandResult:
    name: str
    elapsed_ms: float
    payload: dict[str, Any]


def percentile(values: list[float], pct: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    if len(ordered) == 1:
        return ordered[0]
    rank = (len(ordered) - 1) * pct
    lower = int(rank)
    upper = min(lower + 1, len(ordered) - 1)
    weight = rank - lower
    return ordered[lower] * (1.0 - weight) + ordered[upper] * weight


def command_for(args: argparse.Namespace, argv: list[str]) -> list[str]:
    if args.host:
        bin_part = args.bin if args.bin.startswith(("~/", "$HOME/")) else shlex.quote(args.bin)
        remote = " ".join([bin_part, *(shlex.quote(part) for part in argv)])
        return ["ssh", args.host, remote]
    return [args.bin, *argv]


def run_json(args: argparse.Namespace, name: str, argv: list[str]) -> CommandResult:
    started = time.perf_counter()
    proc = subprocess.run(
        command_for(args, argv),
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=args.command_timeout_sec,
    )
    elapsed_ms = (time.perf_counter() - started) * 1000.0
    if proc.returncode != 0:
        raise RuntimeError(
            f"{name} failed with exit {proc.returncode}: {proc.stderr.strip() or proc.stdout.strip()}"
        )
    try:
        payload = json.loads(proc.stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(f"{name} did not return JSON: {error}: {proc.stdout[:500]}") from error
    return CommandResult(name=name, elapsed_ms=elapsed_ms, payload=payload)


def app_args(args: argparse.Namespace, *extra: str) -> list[str]:
    argv = ["server", "app", *extra, "--timeout-ms", str(args.timeout_ms)]
    if args.pid is not None and extra and extra[0] in {"state", "rows", "screenshot"}:
        if extra[0] == "screenshot":
            argv = ["server", "app", "screenshot", *extra[1:], "--pid", str(args.pid), "--timeout-ms", str(args.timeout_ms)]
        else:
            argv = ["server", "app", extra[0], "--pid", str(args.pid), "--timeout-ms", str(args.timeout_ms)]
    return argv


def terminal_args(args: argparse.Namespace, session_path: str, *extra: str) -> list[str]:
    argv = ["server", "app", "terminal", *extra, "--timeout-ms", str(args.timeout_ms)]
    if args.pid is not None:
        argv[4:4] = ["--pid", str(args.pid)]
    return argv


def data_from(result: CommandResult) -> dict[str, Any]:
    data = result.payload.get("data")
    if not isinstance(data, dict):
        raise RuntimeError(f"{result.name} response missing data object")
    return data


def active_session_from_state(state: dict[str, Any]) -> str:
    active = state.get("active_session_path")
    if isinstance(active, str) and active:
        return active
    raise RuntimeError("app state has no active_session_path; pass --session-path")


def active_terminal_viewport(state: dict[str, Any]) -> dict[str, Any]:
    viewport = state.get("viewport")
    merged = dict(viewport) if isinstance(viewport, dict) else dict(state)
    dom_hosts = (state.get("dom") or {}).get("terminal_hosts")
    if (
        not isinstance(merged.get("terminal_hosts"), list)
        and isinstance(dom_hosts, list)
    ):
        merged["terminal_hosts"] = dom_hosts
    if (
        not isinstance(merged.get("active_terminal_hosts"), list)
        and isinstance(dom_hosts, list)
    ):
        active = state.get("active_session_path") or merged.get("active_session_path")
        merged["active_terminal_hosts"] = [
            host
            for host in dom_hosts
            if isinstance(host, dict) and host.get("session_path") == active
        ]
    return merged


def drawing_probe_summary(state: dict[str, Any], session_path: str) -> dict[str, Any]:
    probe = terminal_drawing_probe(state, session_path)
    surface = probe.get("surface") if isinstance(probe.get("surface"), dict) else {}
    host = probe.get("host") if isinstance(probe.get("host"), dict) else {}
    runtime_truth = probe.get("runtime_truth") if isinstance(probe.get("runtime_truth"), dict) else {}
    return {
        "active_host_ready": runtime_truth.get("active_host_ready"),
        "input_enabled": host.get("input_enabled"),
        "surface_problem": surface.get("problem") or surface.get("live_problem"),
        "content_source": surface.get("content_source") or host.get("terminal_content_source"),
        "retained_replay_source": surface.get("retained_replay_source") or host.get("retained_replay_source"),
        "base_y": (surface.get("prompt_band") or {}).get("base_y") if isinstance(surface.get("prompt_band"), dict) else host.get("base_y"),
        "viewport_y": (surface.get("prompt_band") or {}).get("viewport_y") if isinstance(surface.get("prompt_band"), dict) else host.get("viewport_y"),
        "cursor_line_text": (surface.get("prompt_band") or {}).get("cursor_line_text") if isinstance(surface.get("prompt_band"), dict) else host.get("cursor_line_text"),
        "render_event_count": (surface.get("timing") or {}).get("render_event_count") if isinstance(surface.get("timing"), dict) else host.get("render_event_count"),
        "write_bridge_flush_count": (surface.get("timing") or {}).get("write_bridge_flush_count") if isinstance(surface.get("timing"), dict) else host.get("write_bridge_flush_count"),
    }


def wait_for_read_only_terminal_settle(
    args: argparse.Namespace,
    report: dict[str, Any],
    state_result: CommandResult,
    state_data: dict[str, Any],
    session_path: str,
    readiness_failures: list[str],
) -> tuple[CommandResult, dict[str, Any], str, list[str]]:
    if not args.read_only_drawing or not readiness_failures or args.read_only_settle_ms <= 0:
        return state_result, state_data, session_path, readiness_failures

    started = time.perf_counter()
    attempts: list[dict[str, Any]] = [
        {
            "elapsed_ms": 0.0,
            "state_ms": state_result.elapsed_ms,
            "active_session_path": session_path,
            "failures": readiness_failures,
            "drawing": drawing_probe_summary(state_data, session_path),
        }
    ]
    deadline = started + (args.read_only_settle_ms / 1000.0)
    final_failures = readiness_failures
    final_result = state_result
    final_state = state_data
    final_session_path = session_path
    interval_sec = max(0.05, args.read_only_settle_interval_ms / 1000.0)
    while time.perf_counter() < deadline and final_failures:
        time.sleep(interval_sec)
        next_result = run_json(args, "state_settle", app_args(args, "state"))
        next_state = data_from(next_result)
        next_session_path = args.session_path or active_session_from_state(next_state)
        next_failures = collect_terminal_readiness_failures(next_state, next_session_path)
        attempts.append(
            {
                "elapsed_ms": (time.perf_counter() - started) * 1000.0,
                "state_ms": next_result.elapsed_ms,
                "active_session_path": next_session_path,
                "failures": next_failures,
                "drawing": drawing_probe_summary(next_state, next_session_path),
            }
        )
        final_result = next_result
        final_state = next_state
        final_session_path = next_session_path
        final_failures = next_failures

    retained_before_ready = [
        attempt
        for attempt in attempts
        if attempt.get("failures")
        and str((attempt.get("drawing") or {}).get("retained_replay_source") or "")
        == "daemon_retained_snapshot"
    ]
    if retained_before_ready:
        final_failures = [
            *final_failures,
            "daemon retained snapshot replay was visible before terminal readiness",
        ]

    settle_elapsed_ms = (time.perf_counter() - started) * 1000.0
    report["read_only_settle"] = {
        "enabled": True,
        "settled": not final_failures,
        "elapsed_ms": settle_elapsed_ms,
        "attempt_count": len(attempts),
        "initial_failures": readiness_failures,
        "final_failures": final_failures,
        "attempts": attempts,
    }
    report["measurements"]["terminal_readiness_settle_ms"] = settle_elapsed_ms
    return final_result, final_state, final_session_path, final_failures


def active_terminal_host(viewport: dict[str, Any], session_path: str) -> dict[str, Any]:
    hosts = viewport.get("active_terminal_hosts")
    if not isinstance(hosts, list):
        hosts = viewport.get("terminal_hosts")
    if not isinstance(hosts, list):
        return {}
    matches = [
        host
        for host in hosts
        if isinstance(host, dict) and host.get("session_path") == session_path
    ]
    if matches:
        focused = [
            host
            for host in matches
            if host.get("helper_textarea_focused") is True
            or host.get("host_has_active_element") is True
        ]
        if focused:
            return focused[-1]
        input_enabled = [host for host in matches if host.get("input_enabled") is True]
        if input_enabled:
            return input_enabled[-1]
        return matches[-1]
    return {}


def number_field(value: Any) -> float | None:
    try:
        if value is None:
            return None
        return float(value)
    except (TypeError, ValueError):
        return None


def terminal_drawing_probe(state: dict[str, Any], session_path: str) -> dict[str, Any]:
    viewport = active_terminal_viewport(state)
    surface = viewport.get("active_terminal_surface")
    host = active_terminal_host(viewport, session_path)
    runtime_truth = state.get("runtime_truth") if isinstance(state.get("runtime_truth"), dict) else {}
    probe = {
        "active_session_path": session_path,
        "runtime_truth": {
            "active_runtime_present": runtime_truth.get("active_runtime_present"),
            "active_host_ready": runtime_truth.get("active_host_ready"),
            "active_host_input_enabled": runtime_truth.get("active_host_input_enabled"),
            "active_host_render_event_count": runtime_truth.get("active_host_render_event_count"),
            "active_host_write_bridge_flush_count": runtime_truth.get("active_host_write_bridge_flush_count"),
            "active_host_manual_redraw_count": runtime_truth.get("active_host_manual_redraw_count"),
        },
        "surface": surface if isinstance(surface, dict) else None,
        "host": {},
    }
    if host:
        probe["host"] = {
            "host_id": host.get("host_id"),
            "session_path": host.get("session_path"),
            "xterm_present": host.get("xterm_present"),
            "viewport_present": host.get("viewport_present"),
            "screen_present": host.get("screen_present"),
            "rows_present": host.get("rows_present"),
            "xterm_renderer_mode": host.get("xterm_renderer_mode"),
            "input_enabled": host.get("input_enabled"),
            "helper_textarea_focused": host.get("helper_textarea_focused"),
            "terminal_content_source": host.get("terminal_content_source"),
            "retained_replay_source": host.get("retained_replay_source"),
            "terminal_source_mismatch_reason": host.get("terminal_source_mismatch_reason"),
            "render_health_status": host.get("render_health_status"),
            "render_health_reason": host.get("render_health_reason"),
            "render_health_ink_sample": host.get("render_health_ink_sample"),
            "cursor_line_text": host.get("cursor_line_text") or host.get("cursor_row_text"),
            "cursor_expected_rect": host.get("cursor_expected_rect"),
            "cursor_sample_rect": host.get("cursor_sample_rect"),
            "cursor_row_rect": host.get("cursor_row_rect"),
            "cursor_bottom_overflow_px": host.get("cursor_bottom_overflow_px"),
            "fit_overflow_px": host.get("fit_overflow_px"),
            "fit_required_height_px": host.get("fit_required_height_px"),
            "fit_available_height_px": host.get("fit_available_height_px"),
            "blank_rows_below_cursor": host.get("blank_rows_below_cursor"),
            "base_y": host.get("base_y"),
            "viewport_y": host.get("viewport_y"),
            "last_data_event_at_ms": host.get("last_data_event_at_ms"),
            "last_write_queued_at_ms": host.get("last_write_queued_at_ms"),
            "last_write_flush_started_at_ms": host.get("last_write_flush_started_at_ms"),
            "last_write_callback_at_ms": host.get("last_write_callback_at_ms"),
            "last_render_event_at_ms": host.get("last_render_event_at_ms"),
            "terminal_write_frame_ms": host.get("terminal_write_frame_ms"),
            "terminal_active_write_frame_ms": host.get("terminal_active_write_frame_ms"),
            "effective_terminal_write_frame_ms": host.get("effective_terminal_write_frame_ms"),
            "active_write_frame_budget": host.get("active_write_frame_budget"),
            "recent_frame_like_write_hot": host.get("recent_frame_like_write_hot"),
            "last_raw_payload_length": host.get("last_raw_payload_length"),
            "last_raw_payload_line_count": host.get("last_raw_payload_line_count"),
            "write_command_count": host.get("write_command_count"),
            "write_bridge_flush_count": host.get("write_bridge_flush_count"),
            "render_event_count": host.get("render_event_count"),
            "forced_refresh_count": host.get("forced_refresh_count"),
            "forced_refresh_skipped_count": host.get("forced_refresh_skipped_count"),
            "manual_redraw_count": host.get("manual_redraw_count"),
            "last_manual_redraw_started_at_ms": host.get("last_manual_redraw_started_at_ms"),
            "last_manual_redraw_settled_at_ms": host.get("last_manual_redraw_settled_at_ms"),
            "last_manual_redraw_duration_ms": host.get("last_manual_redraw_duration_ms"),
            "last_manual_redraw_effect": host.get("last_manual_redraw_effect"),
            "low_power_tui_overlay_active": host.get("low_power_tui_overlay_active"),
            "low_power_tui_overlay_present": host.get("low_power_tui_overlay_present"),
            "last_skipped_fit": host.get("last_skipped_fit"),
        }
    return probe


def collect_terminal_readiness_failures(state: dict[str, Any], session_path: str) -> list[str]:
    failures: list[str] = []
    viewport = active_terminal_viewport(state)
    shell = state.get("shell") if isinstance(state.get("shell"), dict) else {}
    active_session = state.get("active_session_path") or viewport.get("active_session_path")
    if isinstance(active_session, str) and active_session and active_session != session_path:
        failures.append(
            f"active session changed during proof: {active_session!r} != {session_path!r}"
        )
    in_flight = shell.get("terminal_attach_in_flight")
    if isinstance(in_flight, list) and session_path in in_flight:
        failures.append(f"terminal attach still in flight for {session_path}")
    if viewport.get("active_view_mode") != "Terminal":
        failures.append(f"active view is {viewport.get('active_view_mode')!r}, not Terminal")
    if viewport.get("ready") is not True:
        failures.append(f"terminal viewport not ready: {viewport.get('reason') or 'unknown reason'}")
    if viewport.get("interactive") is not True:
        failures.append("terminal viewport not interactive")
    surface = viewport.get("active_terminal_surface")
    if isinstance(surface, dict):
        surface_session = str(surface.get("session_path") or "")
        if surface_session and surface_session != session_path:
            failures.append(
                "active terminal surface belongs to a different session: "
                f"{surface_session!r} != {session_path!r}"
            )
        if surface.get("rendered") is not True:
            failures.append("active terminal surface is not rendered")
        problem = surface.get("problem") or surface.get("geometry_problem") or surface.get("live_problem")
        if problem:
            failures.append(f"active terminal surface problem: {problem}")
    host = active_terminal_host(viewport, session_path)
    if not host:
        failures.append("no active terminal host found")
    else:
        host_session = str(host.get("session_path") or "")
        if host_session and host_session != session_path:
            failures.append(
                "active terminal host belongs to a different session: "
                f"{host_session!r} != {session_path!r}"
            )
        for key in ("xterm_present", "viewport_present", "input_enabled"):
            if host.get(key) is not True:
                failures.append(f"active terminal host {key}={host.get(key)!r}")
        if host.get("low_power_tui_overlay_active") is True or host.get("low_power_tui_overlay_present") is True:
            failures.append("active terminal host is showing the low-power TUI overlay")
        content_source = str(host.get("terminal_content_source") or "")
        retained_source = str(host.get("retained_replay_source") or "")
        source_mismatch = str(host.get("terminal_source_mismatch_reason") or "")
        if "server_prompt" in content_source or "server_prompt" in retained_source or source_mismatch:
            failures.append(
                "active terminal host has a non-PTY or mismatched content source: "
                f"content={content_source!r} retained={retained_source!r} mismatch={source_mismatch!r}"
            )
        fit_overflow = number_field(host.get("fit_overflow_px"))
        if fit_overflow is not None and fit_overflow > 1.5:
            failures.append(f"active terminal fit overflows visible host by {fit_overflow:.1f}px")
        cursor_overflow = number_field(host.get("cursor_bottom_overflow_px"))
        if cursor_overflow is not None and cursor_overflow > 1.0:
            failures.append(f"active terminal cursor overflows prompt band by {cursor_overflow:.1f}px")
        if host.get("scrollback_locked") is True:
            failures.append("active terminal host is scrollback-locked away from the live cursor")
        active_visible_terminal = (
            host.get("input_enabled") is True
            and host.get("viewport_present") is True
            and host.get("screen_present") is True
        )
        if active_visible_terminal:
            effective_frame_ms = number_field(host.get("effective_terminal_write_frame_ms"))
            active_frame_ms = number_field(host.get("terminal_active_write_frame_ms"))
            if host.get("active_write_frame_budget") is not True:
                failures.append("active visible terminal is not using the active write frame budget")
            if effective_frame_ms is None:
                failures.append("active visible terminal does not expose effective write frame budget")
            elif effective_frame_ms > 750.0:
                failures.append(
                    f"active visible terminal write frame budget is too slow: {effective_frame_ms:.0f}ms"
                )
            if (
                active_frame_ms is not None
                and effective_frame_ms is not None
                and effective_frame_ms > active_frame_ms + 1.0
            ):
                failures.append(
                    "active visible terminal is using background write budget "
                    f"({effective_frame_ms:.0f}ms > active {active_frame_ms:.0f}ms)"
                )
        cursor_rect = host.get("cursor_expected_rect")
        viewport_rect = host.get("viewport_rect")
        if isinstance(cursor_rect, dict) and isinstance(viewport_rect, dict):
            try:
                cursor_top = float(cursor_rect.get("top"))
                cursor_bottom = float(cursor_rect.get("bottom", cursor_top + float(cursor_rect.get("height", 0.0))))
                viewport_top = float(viewport_rect.get("top"))
                viewport_bottom = float(viewport_rect.get("bottom", viewport_top + float(viewport_rect.get("height", 0.0))))
                if cursor_bottom < viewport_top - 1.0 or cursor_top > viewport_bottom + 1.0:
                    failures.append("active terminal cursor is outside the visible viewport")
            except (TypeError, ValueError):
                pass
    return failures


def terminal_probe(args: argparse.Namespace, session_path: str, token: str) -> CommandResult:
    argv = terminal_args(
        args,
        session_path,
        "probe-type",
        session_path,
        "--mode",
        args.terminal_mode,
        "--per-char",
        "--data",
        token,
    )
    return run_json(args, f"terminal_type_{token}", argv)


def sample_process_cpu(args: argparse.Namespace) -> dict[str, Any] | None:
    if args.pid is None:
        return None
    if args.host:
        cmd = [
            "ssh",
            args.host,
            f"ps -p {int(args.pid)} -o pid=,pcpu=,rss=,etime=,cmd=",
        ]
    else:
        cmd = ["ps", "-p", str(int(args.pid)), "-o", "pid=,pcpu=,rss=,etime=,cmd="]
    proc = subprocess.run(
        cmd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=args.command_timeout_sec,
    )
    if proc.returncode != 0:
        return {"error": proc.stderr.strip() or proc.stdout.strip()}
    line = proc.stdout.strip()
    parts = line.split(None, 4)
    if len(parts) < 4:
        return {"raw": line}
    result: dict[str, Any] = {
        "pid": parts[0],
        "pcpu": None,
        "rss_kb": None,
        "etime": parts[3],
        "cmd": parts[4] if len(parts) > 4 else "",
    }
    try:
        result["pcpu"] = float(parts[1])
    except ValueError:
        pass
    try:
        result["rss_kb"] = int(parts[2])
    except ValueError:
        pass
    return result


def sample_webkit_child_cpu(args: argparse.Namespace) -> list[dict[str, Any]]:
    if args.pid is None:
        return []
    ps_cmd = f"ps --ppid {int(args.pid)} -o pid=,pcpu=,rss=,comm=,cmd="
    cmd = ["ssh", args.host, ps_cmd] if args.host else ["bash", "-lc", ps_cmd]
    proc = subprocess.run(
        cmd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=args.command_timeout_sec,
    )
    if proc.returncode != 0:
        return [{"error": proc.stderr.strip() or proc.stdout.strip()}]
    rows: list[dict[str, Any]] = []
    for line in proc.stdout.splitlines():
        parts = line.split(None, 4)
        if len(parts) < 4:
            continue
        comm = parts[3]
        cmdline = parts[4] if len(parts) > 4 else ""
        if "WebKit" not in comm and "WebKit" not in cmdline:
            continue
        row: dict[str, Any] = {
            "pid": None,
            "pcpu": None,
            "rss_kb": None,
            "comm": comm,
            "cmd": cmdline,
        }
        try:
            row["pid"] = int(parts[0])
        except ValueError:
            row["pid"] = parts[0]
        try:
            row["pcpu"] = float(parts[1])
        except ValueError:
            pass
        try:
            row["rss_kb"] = int(parts[2])
        except ValueError:
            pass
        rows.append(row)
    return rows


def sample_proc_cpu_ticks(args: argparse.Namespace, pids: list[int]) -> dict[str, Any] | None:
    if not pids:
        return None
    pid_list = ",".join(str(int(pid)) for pid in pids)
    script = (
        "python3 - <<'PY'\n"
        "import json, os\n"
        f"pids=[int(part) for part in {pid_list!r}.split(',') if part]\n"
        "def read_total():\n"
        "    return sum(int(part) for part in open('/proc/stat').readline().split()[1:])\n"
        "rows={}\n"
        "for pid in pids:\n"
        "    try:\n"
        "        stat=open(f'/proc/{pid}/stat').read().rsplit(')',1)[1].split()\n"
        "        rows[str(pid)]={'ticks': int(stat[11]) + int(stat[12]), 'comm': open(f'/proc/{pid}/comm').read().strip()}\n"
        "    except Exception as error:\n"
        "        rows[str(pid)]={'error': str(error)}\n"
        "print(json.dumps({'total_ticks': read_total(), 'cpu_count': os.cpu_count() or 1, 'processes': rows}))\n"
        "PY"
    )
    cmd = ["ssh", args.host, script] if args.host else ["bash", "-lc", script]
    proc = subprocess.run(
        cmd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=args.command_timeout_sec,
    )
    if proc.returncode != 0:
        return {"error": proc.stderr.strip() or proc.stdout.strip()}
    try:
        payload = json.loads(proc.stdout)
    except json.JSONDecodeError as error:
        return {"error": f"proc CPU sample JSON error: {error}"}
    return payload if isinstance(payload, dict) else None


def host_counter(host: dict[str, Any], key: str) -> int:
    try:
        return int(host.get(key) or 0)
    except (TypeError, ValueError):
        return 0


def probe_counter_delta(before: dict[str, Any], after: dict[str, Any], key: str) -> int | None:
    try:
        return int(after.get(key) or 0) - int(before.get(key) or 0)
    except (TypeError, ValueError):
        return None


def process_cpu_percent(sample: dict[str, Any] | None) -> float:
    if not sample:
        return 0.0
    value = sample.get("pcpu")
    return float(value) if isinstance(value, (int, float)) else 0.0


def webkit_cpu_percent(children: list[dict[str, Any]]) -> float:
    total = 0.0
    for child in children:
        value = child.get("pcpu")
        if isinstance(value, (int, float)):
            total += float(value)
    return total


def pid_int(value: Any) -> int | None:
    try:
        if value is None:
            return None
        return int(value)
    except (TypeError, ValueError):
        return None


def read_only_counter_sample(
    result: CommandResult,
    state: dict[str, Any],
    session_path: str,
    app_cpu: dict[str, Any] | None,
    webkit_children: list[dict[str, Any]],
    proc_cpu: dict[str, Any] | None,
    elapsed_ms: float,
) -> dict[str, Any]:
    host = active_terminal_host(active_terminal_viewport(state), session_path)
    app_cpu_percent = process_cpu_percent(app_cpu)
    webkit_cpu = webkit_cpu_percent(webkit_children)
    return {
        "elapsed_ms": elapsed_ms,
        "state_ms": result.elapsed_ms,
        "active_session_path": state.get("active_session_path"),
        "render_event_count": host_counter(host, "render_event_count"),
        "write_bridge_flush_count": host_counter(host, "write_bridge_flush_count"),
        "write_command_count": host_counter(host, "write_command_count"),
        "manual_redraw_count": host_counter(host, "manual_redraw_count"),
        "forced_refresh_count": host_counter(host, "forced_refresh_count"),
        "forced_refresh_skipped_count": host_counter(host, "forced_refresh_skipped_count"),
        "effective_terminal_write_frame_ms": number_field(host.get("effective_terminal_write_frame_ms")),
        "active_write_frame_budget": host.get("active_write_frame_budget"),
        "app_cpu": app_cpu,
        "webkit_children": webkit_children,
        "proc_cpu": proc_cpu,
        "app_cpu_percent": app_cpu_percent,
        "webkit_cpu_percent": webkit_cpu,
        "combined_cpu_percent": app_cpu_percent + webkit_cpu,
    }


def proc_cpu_interval_percent(
    before: dict[str, Any],
    after: dict[str, Any],
    pids: list[int],
) -> float | None:
    before_proc = before.get("proc_cpu") if isinstance(before.get("proc_cpu"), dict) else None
    after_proc = after.get("proc_cpu") if isinstance(after.get("proc_cpu"), dict) else None
    if not before_proc or not after_proc:
        return None
    try:
        total_delta = int(after_proc["total_ticks"]) - int(before_proc["total_ticks"])
        cpu_count = int(after_proc.get("cpu_count") or before_proc.get("cpu_count") or 1)
    except (KeyError, TypeError, ValueError):
        return None
    if total_delta <= 0:
        return None
    before_processes = before_proc.get("processes")
    after_processes = after_proc.get("processes")
    if not isinstance(before_processes, dict) or not isinstance(after_processes, dict):
        return None
    tick_delta = 0
    for pid in pids:
        before_row = before_processes.get(str(pid))
        after_row = after_processes.get(str(pid))
        if not isinstance(before_row, dict) or not isinstance(after_row, dict):
            continue
        try:
            tick_delta += int(after_row["ticks"]) - int(before_row["ticks"])
        except (KeyError, TypeError, ValueError):
            continue
    return 100.0 * float(tick_delta) * float(cpu_count) / float(total_delta)


def collect_read_only_activity(
    args: argparse.Namespace,
    session_path: str,
) -> dict[str, Any]:
    samples: list[dict[str, Any]] = []
    started = time.perf_counter()
    sample_count = max(2, args.read_only_samples)
    interval_sec = max(0.05, args.read_only_sample_interval_ms / 1000.0)
    for ix in range(sample_count):
        result = run_json(args, f"state_read_only_activity_{ix}", app_args(args, "state"))
        state = data_from(result)
        app_cpu = sample_process_cpu(args)
        webkit_children = sample_webkit_child_cpu(args)
        sample_pids: list[int] = []
        app_pid = pid_int((app_cpu or {}).get("pid") if isinstance(app_cpu, dict) else None)
        if app_pid is not None:
            sample_pids.append(app_pid)
        for child in webkit_children:
            child_pid = pid_int(child.get("pid")) if isinstance(child, dict) else None
            if child_pid is not None:
                sample_pids.append(child_pid)
        proc_cpu = sample_proc_cpu_ticks(args, sample_pids)
        elapsed_ms = (time.perf_counter() - started) * 1000.0
        samples.append(
            read_only_counter_sample(
                result,
                state,
                session_path,
                app_cpu,
                webkit_children,
                proc_cpu,
                elapsed_ms,
            )
        )
        if ix != sample_count - 1:
            time.sleep(interval_sec)

    first = samples[0]
    last = samples[-1]
    elapsed_sec = max(0.001, (last["elapsed_ms"] - first["elapsed_ms"]) / 1000.0)
    render_delta = max(0, int(last["render_event_count"]) - int(first["render_event_count"]))
    flush_delta = max(
        0,
        int(last["write_bridge_flush_count"]) - int(first["write_bridge_flush_count"]),
    )
    write_delta = max(0, int(last["write_command_count"]) - int(first["write_command_count"]))
    manual_redraw_delta = max(
        0,
        int(last["manual_redraw_count"]) - int(first["manual_redraw_count"]),
    )
    forced_refresh_delta = max(
        0,
        int(last["forced_refresh_count"]) - int(first["forced_refresh_count"]),
    )
    combined_cpu_values = [
        float(sample["combined_cpu_percent"])
        for sample in samples
        if isinstance(sample.get("combined_cpu_percent"), (int, float))
    ]
    app_cpu_values = [
        float(sample["app_cpu_percent"])
        for sample in samples
        if isinstance(sample.get("app_cpu_percent"), (int, float))
    ]
    webkit_cpu_values = [
        float(sample["webkit_cpu_percent"])
        for sample in samples
        if isinstance(sample.get("webkit_cpu_percent"), (int, float))
    ]
    app_current_cpu_values: list[float] = []
    webkit_current_cpu_values: list[float] = []
    combined_current_cpu_values: list[float] = []
    for before, after in zip(samples, samples[1:]):
        app_pid = pid_int((after.get("app_cpu") or {}).get("pid")) if isinstance(after.get("app_cpu"), dict) else None
        child_pids = [
            pid
            for pid in (
                pid_int(child.get("pid")) if isinstance(child, dict) else None
                for child in after.get("webkit_children", [])
            )
            if pid is not None
        ]
        if app_pid is not None:
            value = proc_cpu_interval_percent(before, after, [app_pid])
            if value is not None:
                app_current_cpu_values.append(value)
        if child_pids:
            value = proc_cpu_interval_percent(before, after, child_pids)
            if value is not None:
                webkit_current_cpu_values.append(value)
        combined_pids = ([app_pid] if app_pid is not None else []) + child_pids
        if combined_pids:
            value = proc_cpu_interval_percent(before, after, combined_pids)
            if value is not None:
                combined_current_cpu_values.append(value)
    app_current_cpu_max = max(app_current_cpu_values) if app_current_cpu_values else None
    webkit_current_cpu_max = max(webkit_current_cpu_values) if webkit_current_cpu_values else None
    combined_current_cpu_max = (
        max(combined_current_cpu_values) if combined_current_cpu_values else None
    )
    return {
        "sample_count": sample_count,
        "elapsed_sec": elapsed_sec,
        "samples": samples,
        "render_event_delta": render_delta,
        "write_bridge_flush_delta": flush_delta,
        "write_command_delta": write_delta,
        "manual_redraw_delta": manual_redraw_delta,
        "forced_refresh_delta": forced_refresh_delta,
        "render_events_per_sec": render_delta / elapsed_sec,
        "write_flushes_per_sec": flush_delta / elapsed_sec,
        "write_commands_per_sec": write_delta / elapsed_sec,
        "manual_redraws_per_sec": manual_redraw_delta / elapsed_sec,
        "forced_refreshes_per_sec": forced_refresh_delta / elapsed_sec,
        "combined_cpu_max_percent": max(combined_cpu_values) if combined_cpu_values else None,
        "app_cpu_max_percent": max(app_cpu_values) if app_cpu_values else None,
        "webkit_cpu_max_percent": max(webkit_cpu_values) if webkit_cpu_values else None,
        "combined_cpu_current_max_percent": combined_current_cpu_max,
        "app_cpu_current_max_percent": app_current_cpu_max,
        "webkit_cpu_current_max_percent": webkit_current_cpu_max,
    }


def budget_check(report: dict[str, Any], name: str, value: float | None, budget: float) -> None:
    if value is None:
        report["failures"].append(f"{name}: missing measurement")
    elif value > budget:
        report["failures"].append(f"{name}: {value:.1f}ms > {budget:.1f}ms")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", help="Run commands over ssh on this host.")
    parser.add_argument("--bin", default=DEFAULT_BIN, help="Yggterm binary path on the target.")
    parser.add_argument("--pid", type=int, help="Target app-control client PID.")
    parser.add_argument("--session-path", help="Terminal session path to probe. Defaults to active session.")
    parser.add_argument("--samples", type=int, default=5)
    parser.add_argument("--timeout-ms", type=int, default=15000)
    parser.add_argument("--command-timeout-sec", type=float, default=25.0)
    parser.add_argument("--terminal-mode", choices=["keyboard", "xterm", "auto"], default="keyboard")
    parser.add_argument(
        "--read-only-drawing",
        action="store_true",
        help="Only collect state/rows/drawing diagnostics; do not type, scroll, search, or change panels.",
    )
    parser.add_argument(
        "--screenshot-out",
        type=Path,
        help="Optional target-side screenshot path for --read-only-drawing proof.",
    )
    parser.add_argument(
        "--read-only-settle-ms",
        type=float,
        default=8000.0,
        help="How long --read-only-drawing waits for terminal retained replay/render readiness.",
    )
    parser.add_argument(
        "--read-only-settle-interval-ms",
        type=float,
        default=250.0,
        help="Polling interval while --read-only-drawing waits for terminal readiness.",
    )
    parser.add_argument(
        "--read-only-samples",
        type=int,
        default=4,
        help="Number of read-only state/CPU samples to collect after readiness settles.",
    )
    parser.add_argument(
        "--read-only-sample-interval-ms",
        type=float,
        default=1000.0,
        help="Interval between read-only churn samples.",
    )
    parser.add_argument("--clear-after", action="store_true", help="Send Ctrl+U after terminal samples.")
    parser.add_argument(
        "--clear-every-samples",
        type=int,
        default=16,
        help="When --clear-after is set, also clear the prompt every N samples to prevent line-wrap false negatives.",
    )
    parser.add_argument("--json-out", type=Path)
    parser.add_argument("--max-state-ms", type=float, default=1200.0)
    parser.add_argument("--max-rows-ms", type=float, default=1200.0)
    parser.add_argument("--max-search-ms", type=float, default=1200.0)
    parser.add_argument("--max-panel-ms", type=float, default=1200.0)
    parser.add_argument("--max-terminal-warmup-visible-ms", type=float, default=700.0)
    parser.add_argument("--max-terminal-visible-ms", type=float, default=500.0)
    parser.add_argument("--max-terminal-p95-ms", type=float, default=450.0)
    parser.add_argument("--max-terminal-drift-ms", type=float, default=175.0)
    parser.add_argument("--max-terminal-scroll-ms", type=float, default=800.0)
    parser.add_argument(
        "--max-terminal-scroll-noop-ms",
        type=float,
        default=1500.0,
        help="Budget for accepted scroll probes when the terminal has no scrollback to move.",
    )
    parser.add_argument("--max-render-events-per-sample", type=float, default=18.0)
    parser.add_argument("--max-write-flushes-per-sample", type=float, default=4.0)
    parser.add_argument("--max-read-only-render-events-per-sec", type=float, default=3.0)
    parser.add_argument("--max-read-only-write-flushes-per-sec", type=float, default=1.5)
    parser.add_argument("--max-read-only-combined-cpu-percent", type=float, default=25.0)
    parser.add_argument("--max-app-cpu-percent", type=float, default=65.0)
    parser.add_argument("--skip-scroll-check", action="store_true")
    parser.add_argument("--skip-readiness-gate", action="store_true")
    args = parser.parse_args()

    report: dict[str, Any] = {
        "ok": False,
        "target": {"host": args.host, "bin": args.bin, "pid": args.pid},
        "budgets_ms": {
            "state": args.max_state_ms,
            "rows": args.max_rows_ms,
            "search": args.max_search_ms,
            "panel": args.max_panel_ms,
            "terminal_warmup_visible": args.max_terminal_warmup_visible_ms,
            "terminal_visible": args.max_terminal_visible_ms,
            "terminal_p95": args.max_terminal_p95_ms,
            "terminal_drift": args.max_terminal_drift_ms,
            "terminal_scroll": args.max_terminal_scroll_ms,
            "terminal_scroll_noop": args.max_terminal_scroll_noop_ms,
            "render_events_per_sample": args.max_render_events_per_sample,
            "write_flushes_per_sample": args.max_write_flushes_per_sample,
            "read_only_render_events_per_sec": args.max_read_only_render_events_per_sec,
            "read_only_write_flushes_per_sec": args.max_read_only_write_flushes_per_sec,
            "read_only_combined_cpu_percent": args.max_read_only_combined_cpu_percent,
            "app_cpu_percent": args.max_app_cpu_percent,
        },
        "measurements": {},
        "terminal_samples": [],
        "terminal_scroll": None,
        "process_samples": [],
        "failures": [],
    }

    state_result = run_json(args, "state", app_args(args, "state"))
    state_data = data_from(state_result)
    session_path = args.session_path or active_session_from_state(state_data)
    report["active_session_path"] = session_path
    report["measurements"]["initial_state_ms"] = state_result.elapsed_ms
    readiness_failures = collect_terminal_readiness_failures(state_data, session_path)
    report["terminal_readiness_initial"] = {
        "ok": not readiness_failures,
        "failures": readiness_failures,
    }
    state_result, state_data, session_path, readiness_failures = wait_for_read_only_terminal_settle(
        args,
        report,
        state_result,
        state_data,
        session_path,
        readiness_failures,
    )
    report["active_session_path"] = session_path
    report["measurements"]["state_ms"] = state_result.elapsed_ms
    report["terminal_readiness"] = {
        "ok": not readiness_failures,
        "failures": readiness_failures,
    }
    report["terminal_drawing"] = terminal_drawing_probe(state_data, session_path)
    if readiness_failures:
        report["failures"].extend(readiness_failures)

    rows_result = run_json(args, "rows", app_args(args, "rows"))
    report["measurements"]["rows_ms"] = rows_result.elapsed_ms

    if args.read_only_drawing:
        if args.screenshot_out:
            screenshot_result = run_json(
                args,
                "screenshot",
                app_args(args, "screenshot", str(args.screenshot_out)),
            )
            report["measurements"]["screenshot_ms"] = screenshot_result.elapsed_ms
            screenshot_data = data_from(screenshot_result)
            report["screenshot"] = screenshot_data
            report["screenshot_terminal_drawing"] = terminal_drawing_probe(
                screenshot_data,
                session_path,
            )
        read_only_activity = collect_read_only_activity(args, session_path)
        report["read_only_activity"] = read_only_activity
        report["measurements"]["read_only_render_events_per_sec"] = read_only_activity[
            "render_events_per_sec"
        ]
        report["measurements"]["read_only_write_flushes_per_sec"] = read_only_activity[
            "write_flushes_per_sec"
        ]
        report["measurements"]["read_only_write_commands_per_sec"] = read_only_activity[
            "write_commands_per_sec"
        ]
        report["measurements"]["read_only_combined_cpu_max_percent"] = read_only_activity[
            "combined_cpu_max_percent"
        ]
        report["measurements"]["read_only_combined_cpu_current_max_percent"] = read_only_activity[
            "combined_cpu_current_max_percent"
        ]
        report["measurements"]["read_only_app_cpu_max_percent"] = read_only_activity[
            "app_cpu_max_percent"
        ]
        report["measurements"]["read_only_app_cpu_current_max_percent"] = read_only_activity[
            "app_cpu_current_max_percent"
        ]
        report["measurements"]["read_only_webkit_cpu_max_percent"] = read_only_activity[
            "webkit_cpu_max_percent"
        ]
        report["measurements"]["read_only_webkit_cpu_current_max_percent"] = read_only_activity[
            "webkit_cpu_current_max_percent"
        ]
        if (
            read_only_activity["render_events_per_sec"]
            > args.max_read_only_render_events_per_sec
        ):
            report["failures"].append(
                "read-only render churn: "
                f"{read_only_activity['render_events_per_sec']:.2f} render events/sec "
                f"> {args.max_read_only_render_events_per_sec:.2f}"
            )
        if (
            read_only_activity["write_flushes_per_sec"]
            > args.max_read_only_write_flushes_per_sec
        ):
            report["failures"].append(
                "read-only write flush churn: "
                f"{read_only_activity['write_flushes_per_sec']:.2f} flushes/sec "
                f"> {args.max_read_only_write_flushes_per_sec:.2f}"
            )
        combined_cpu = read_only_activity.get("combined_cpu_current_max_percent")
        cpu_sample_kind = "current"
        if not isinstance(combined_cpu, (int, float)):
            combined_cpu = read_only_activity.get("combined_cpu_max_percent")
            cpu_sample_kind = "lifetime"
        if (
            isinstance(combined_cpu, (int, float))
            and combined_cpu > args.max_read_only_combined_cpu_percent
        ):
            report["failures"].append(
                f"read-only combined GUI/WebKit CPU ({cpu_sample_kind}): "
                f"{combined_cpu:.1f}% > {args.max_read_only_combined_cpu_percent:.1f}%"
            )
        budget_check(report, "state", state_result.elapsed_ms, args.max_state_ms)
        budget_check(report, "rows", rows_result.elapsed_ms, args.max_rows_ms)
        report["ok"] = not report["failures"]
        output = json.dumps(report, indent=2, sort_keys=True)
        if args.json_out:
            args.json_out.write_text(output + "\n", encoding="utf-8")
        print(output)
        return 0 if report["ok"] else 1

    if readiness_failures and not args.skip_readiness_gate:
        budget_check(report, "state", state_result.elapsed_ms, args.max_state_ms)
        budget_check(report, "rows", rows_result.elapsed_ms, args.max_rows_ms)
        report["ok"] = False
        output = json.dumps(report, indent=2, sort_keys=True)
        if args.json_out:
            args.json_out.write_text(output + "\n", encoding="utf-8")
        print(output)
        return 1

    if args.clear_after:
        try:
            run_json(
                args,
                "terminal_initial_clear",
                terminal_args(
                    args,
                    session_path,
                    "probe-type",
                    session_path,
                    "--mode",
                    args.terminal_mode,
                    "--data",
                    "",
                    "--ctrl-u",
                ),
            )
            time.sleep(0.2)
        except Exception as error:  # noqa: BLE001
            report["failures"].append(f"terminal initial clear failed: {error}")

    pre_type_host = active_terminal_host(active_terminal_viewport(state_data), session_path)
    pre_type_cpu = sample_process_cpu(args)
    if pre_type_cpu:
        report["process_samples"].append({"phase": "before_typing", **pre_type_cpu})
    webkit_children_before = sample_webkit_child_cpu(args)
    if webkit_children_before:
        report["process_samples"].append(
            {"phase": "before_typing_webkit_children", "children": webkit_children_before}
        )

    token_prefix = f"l{int(time.time()) % 100:02d}"
    terminal_visible_ms: list[float] = []
    for ix in range(max(1, args.samples)):
        if args.clear_after and args.clear_every_samples > 0 and ix > 0 and ix % args.clear_every_samples == 0:
            try:
                run_json(
                    args,
                    f"terminal_periodic_clear_{ix}",
                    terminal_args(
                        args,
                        session_path,
                        "probe-type",
                        session_path,
                        "--mode",
                        args.terminal_mode,
                        "--data",
                        "",
                        "--ctrl-u",
                    ),
                )
                time.sleep(0.08)
            except Exception as error:  # noqa: BLE001
                report["failures"].append(f"terminal periodic clear {ix} failed: {error}")
        token = f"{token_prefix}{ix:x}"
        probe = terminal_probe(args, session_path, token)
        probe_data = data_from(probe)
        timings = probe_data.get("timings") if isinstance(probe_data.get("timings"), dict) else {}
        visible_ms = timings.get("visible_echo_ms")
        probe_before = probe_data.get("before") if isinstance(probe_data.get("before"), dict) else {}
        probe_after = probe_data.get("after") if isinstance(probe_data.get("after"), dict) else {}
        sample = {
            "token": token,
            "phase": "warmup" if ix == 0 and args.samples > 1 else "steady",
            "command_ms": probe.elapsed_ms,
            "accepted": probe_data.get("accepted"),
            "visible_echo_observed": probe_data.get("visible_echo_observed"),
            "visible_echo_ms": visible_ms,
            "counter_change_ms": timings.get("counter_change_ms"),
            "dispatch_ms": timings.get("dispatch_ms"),
            "total_ms": timings.get("total_ms"),
            "keyboard_backend": probe_data.get("keyboard_backend"),
            "write_command_delta": probe_counter_delta(
                probe_before, probe_after, "write_command_count"
            ),
            "write_flush_delta": probe_counter_delta(
                probe_before, probe_after, "write_bridge_flush_count"
            ),
            "render_event_delta": probe_counter_delta(
                probe_before, probe_after, "render_event_count"
            ),
            "forced_refresh_skipped_delta": probe_counter_delta(
                probe_before, probe_after, "forced_refresh_skipped_count"
            ),
            "terminal_input_hot_after": probe_after.get("terminal_input_hot"),
            "after_cursor_line_text": probe_after.get("cursor_line_text"),
        }
        report["terminal_samples"].append(sample)
        if isinstance(visible_ms, (int, float)):
            terminal_visible_ms.append(float(visible_ms))
        if not probe_data.get("visible_echo_observed"):
            report["failures"].append(f"terminal sample {ix}: visible echo not observed")

    post_type_state = data_from(run_json(args, "state_after_typing", app_args(args, "state")))
    post_type_host = active_terminal_host(active_terminal_viewport(post_type_state), session_path)
    post_type_cpu = sample_process_cpu(args)
    if post_type_cpu:
        report["process_samples"].append({"phase": "after_typing", **post_type_cpu})
    webkit_children_after = sample_webkit_child_cpu(args)
    if webkit_children_after:
        report["process_samples"].append(
            {"phase": "after_typing_webkit_children", "children": webkit_children_after}
        )
    sample_count = max(1, args.samples)
    render_delta = host_counter(post_type_host, "render_event_count") - host_counter(
        pre_type_host, "render_event_count"
    )
    write_flush_delta = host_counter(post_type_host, "write_bridge_flush_count") - host_counter(
        pre_type_host, "write_bridge_flush_count"
    )
    write_command_delta = host_counter(post_type_host, "write_command_count") - host_counter(
        pre_type_host, "write_command_count"
    )
    report["measurements"]["terminal_render_events_per_sample"] = render_delta / sample_count
    report["measurements"]["terminal_write_flushes_per_sample"] = write_flush_delta / sample_count
    report["measurements"]["terminal_write_commands_per_sample"] = write_command_delta / sample_count
    report["measurements"]["terminal_skipped_perf_events"] = host_counter(
        post_type_host, "skippedPerfEventCount"
    )
    for process_sample in report["process_samples"]:
        pcpu = process_sample.get("pcpu")
        if isinstance(pcpu, (int, float)) and pcpu > args.max_app_cpu_percent:
            report["failures"].append(
                f"app CPU {process_sample['phase']}: {pcpu:.1f}% > {args.max_app_cpu_percent:.1f}%"
            )
    if report["measurements"]["terminal_render_events_per_sample"] > args.max_render_events_per_sample:
        report["failures"].append(
            "terminal render churn while typing: "
            f"{report['measurements']['terminal_render_events_per_sample']:.2f} render events/sample "
            f"> {args.max_render_events_per_sample:.2f}"
        )
    if report["measurements"]["terminal_write_flushes_per_sample"] > args.max_write_flushes_per_sample:
        report["failures"].append(
            "terminal write flush churn while typing: "
            f"{report['measurements']['terminal_write_flushes_per_sample']:.2f} flushes/sample "
            f"> {args.max_write_flushes_per_sample:.2f}"
        )

    if args.clear_after:
        try:
            run_json(
                args,
                "terminal_clear",
                terminal_args(
                    args,
                    session_path,
                    "probe-type",
                    session_path,
                    "--mode",
                    args.terminal_mode,
                    "--data",
                    "",
                    "--ctrl-u",
                ),
            )
        except Exception as error:  # noqa: BLE001
            report["failures"].append(f"terminal final clear failed: {error}")

    search_token = f"latency-{token_prefix}"
    search_result = run_json(
        args,
        "search_set",
        ["server", "app", "search", "set", "--query", search_token, "--focus", "on", "--timeout-ms", str(args.timeout_ms)],
    )
    report["measurements"]["search_set_ms"] = search_result.elapsed_ms
    clear_result = run_json(
        args,
        "search_clear",
        ["server", "app", "search", "clear", "--timeout-ms", str(args.timeout_ms)],
    )
    report["measurements"]["search_clear_ms"] = clear_result.elapsed_ms

    panel_result = run_json(
        args,
        "panel_settings",
        ["server", "app", "panel", "settings", "--timeout-ms", str(args.timeout_ms)],
    )
    report["measurements"]["panel_settings_ms"] = panel_result.elapsed_ms

    terminal_warmup_visible_ms = terminal_visible_ms[0] if len(terminal_visible_ms) > 1 else None
    terminal_steady_visible_ms = terminal_visible_ms[1:] if len(terminal_visible_ms) > 1 else terminal_visible_ms
    terminal_p95 = percentile(terminal_steady_visible_ms, 0.95)
    terminal_drift_ms = None
    if len(terminal_steady_visible_ms) >= 4:
        midpoint = len(terminal_steady_visible_ms) // 2
        early = terminal_steady_visible_ms[:midpoint]
        late = terminal_steady_visible_ms[midpoint:]
        terminal_drift_ms = statistics.median(late) - statistics.median(early)
    report["measurements"]["terminal_warmup_visible_ms"] = terminal_warmup_visible_ms
    report["measurements"]["terminal_visible_ms"] = {
        "min": min(terminal_steady_visible_ms) if terminal_steady_visible_ms else None,
        "median": statistics.median(terminal_steady_visible_ms) if terminal_steady_visible_ms else None,
        "p95": terminal_p95,
        "max": max(terminal_steady_visible_ms) if terminal_steady_visible_ms else None,
        "drift_ms": terminal_drift_ms,
    }

    if not args.skip_scroll_check:
        try:
            scroll_started = time.perf_counter()
            scroll_result = run_json(
                args,
                "terminal_scroll",
                terminal_args(args, session_path, "probe-scroll", session_path, "--lines", "-5"),
            )
            scroll_elapsed_ms = (time.perf_counter() - scroll_started) * 1000.0
            scroll_data = data_from(scroll_result)
            report["terminal_scroll"] = {
                "elapsed_ms": scroll_elapsed_ms,
                "accepted": scroll_data.get("accepted"),
                "movement_expected": scroll_data.get("movement_expected"),
                "scroll_probe_moved": scroll_data.get("scroll_probe_moved"),
                "after": scroll_data.get("after"),
            }
            if scroll_data.get("accepted") is not True:
                report["failures"].append(f"terminal scroll not accepted: {scroll_data!r}")
            movement_expected = scroll_data.get("movement_expected") is True
            scroll_budget_ms = (
                args.max_terminal_scroll_ms if movement_expected else args.max_terminal_scroll_noop_ms
            )
            scroll_budget_name = "terminal scroll" if movement_expected else "terminal scroll noop"
            if scroll_elapsed_ms > scroll_budget_ms:
                report["failures"].append(
                    f"{scroll_budget_name}: {scroll_elapsed_ms:.1f}ms > {scroll_budget_ms:.1f}ms"
                )
            if movement_expected and not scroll_data.get("scroll_probe_moved"):
                report["failures"].append(f"terminal scroll expected movement but did not move: {scroll_data!r}")
            after_scroll = scroll_data.get("after") if isinstance(scroll_data.get("after"), dict) else {}
            if movement_expected and scroll_data.get("scroll_probe_moved"):
                if after_scroll.get("scrollback_intent") not in (None, "UserScrollback"):
                    report["failures"].append(
                        "terminal scroll moved into scrollback without UserScrollback intent: "
                        f"{after_scroll!r}"
                    )
                time.sleep(0.65)
                settled_state = data_from(run_json(args, "state_after_scroll", app_args(args, "state")))
                settled_host = active_terminal_host(active_terminal_viewport(settled_state), session_path)
                try:
                    settled_base = int(settled_host.get("base_y") or 0)
                    settled_viewport = int(settled_host.get("viewport_y") or 0)
                    if settled_base > 0 and settled_base <= settled_viewport:
                        report["failures"].append(
                            "terminal scrollback snapped back to bottom after wheel release: "
                            f"{settled_host!r}"
                        )
                except (TypeError, ValueError):
                    pass
                if settled_host.get("scrollback_intent") not in (None, "UserScrollback"):
                    report["failures"].append(
                        "terminal scrollback lost UserScrollback intent after wheel release: "
                        f"{settled_host!r}"
                    )
            try:
                run_json(
                    args,
                    "terminal_scroll_restore",
                    terminal_args(args, session_path, "probe-scroll", session_path, "--lines", "9999"),
                )
            except Exception as error:  # noqa: BLE001
                report["failures"].append(f"terminal scroll restore failed: {error}")
        except Exception as error:  # noqa: BLE001
            report["failures"].append(f"terminal scroll check failed: {error}")

    budget_check(report, "state", state_result.elapsed_ms, args.max_state_ms)
    budget_check(report, "rows", rows_result.elapsed_ms, args.max_rows_ms)
    budget_check(report, "search_set", search_result.elapsed_ms, args.max_search_ms)
    budget_check(report, "panel_settings", panel_result.elapsed_ms, args.max_panel_ms)
    for sample in report["terminal_samples"]:
        sample_budget = (
            args.max_terminal_warmup_visible_ms
            if sample.get("phase") == "warmup"
            else args.max_terminal_visible_ms
        )
        budget_check(
            report,
            f"terminal {sample['token']}",
            sample.get("visible_echo_ms"),
            sample_budget,
        )
    budget_check(report, "terminal p95", terminal_p95, args.max_terminal_p95_ms)
    if terminal_drift_ms is not None and terminal_drift_ms > args.max_terminal_drift_ms:
        report["failures"].append(
            f"terminal visible echo drift: {terminal_drift_ms:.1f}ms > {args.max_terminal_drift_ms:.1f}ms"
        )

    report["ok"] = not report["failures"]
    output = json.dumps(report, indent=2, sort_keys=True)
    if args.json_out:
        args.json_out.write_text(output + "\n", encoding="utf-8")
    print(output)
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
