#!/usr/bin/env python3
"""Measure Yggterm app-control, UI, and terminal input latency.

This smoke is intentionally user-visible when pointed at a live profile: terminal
typing probes insert short marker text into the active prompt. Use --clear-after
when it is acceptable to send Ctrl+U after the samples. The first terminal
sample after opening/clearing the viewport is reported as warmup and gets a
separate budget; steady-state samples keep the stricter visible-echo budget.
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
    if isinstance(viewport, dict):
        return viewport
    return state


def active_terminal_host(viewport: dict[str, Any], session_path: str) -> dict[str, Any]:
    hosts = viewport.get("active_terminal_hosts")
    if not isinstance(hosts, list):
        hosts = viewport.get("terminal_hosts")
    if not isinstance(hosts, list):
        return {}
    for host in hosts:
        if isinstance(host, dict) and host.get("session_path") == session_path:
            return host
    return hosts[0] if hosts and isinstance(hosts[0], dict) else {}


def collect_terminal_readiness_failures(state: dict[str, Any], session_path: str) -> list[str]:
    failures: list[str] = []
    viewport = active_terminal_viewport(state)
    shell = state.get("shell") if isinstance(state.get("shell"), dict) else {}
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
        if surface.get("rendered") is not True:
            failures.append("active terminal surface is not rendered")
        problem = surface.get("problem") or surface.get("geometry_problem") or surface.get("live_problem")
        if problem:
            failures.append(f"active terminal surface problem: {problem}")
    host = active_terminal_host(viewport, session_path)
    if not host:
        failures.append("no active terminal host found")
    else:
        for key in ("xterm_present", "viewport_present", "input_enabled"):
            if host.get(key) is not True:
                failures.append(f"active terminal host {key}={host.get(key)!r}")
        if host.get("scrollback_locked") is True:
            failures.append("active terminal host is scrollback-locked away from the live cursor")
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


def host_counter(host: dict[str, Any], key: str) -> int:
    try:
        return int(host.get(key) or 0)
    except (TypeError, ValueError):
        return 0


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
    parser.add_argument("--max-render-events-per-sample", type=float, default=18.0)
    parser.add_argument("--max-write-flushes-per-sample", type=float, default=4.0)
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
            "render_events_per_sample": args.max_render_events_per_sample,
            "write_flushes_per_sample": args.max_write_flushes_per_sample,
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
    report["measurements"]["state_ms"] = state_result.elapsed_ms
    readiness_failures = collect_terminal_readiness_failures(state_data, session_path)
    report["terminal_readiness"] = {
        "ok": not readiness_failures,
        "failures": readiness_failures,
    }
    if readiness_failures:
        report["failures"].extend(readiness_failures)

    rows_result = run_json(args, "rows", app_args(args, "rows"))
    report["measurements"]["rows_ms"] = rows_result.elapsed_ms

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
            "after_cursor_line_text": (probe_data.get("after") or {}).get("cursor_line_text"),
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
            if scroll_elapsed_ms > args.max_terminal_scroll_ms:
                report["failures"].append(
                    f"terminal scroll: {scroll_elapsed_ms:.1f}ms > {args.max_terminal_scroll_ms:.1f}ms"
                )
            if scroll_data.get("movement_expected") and not scroll_data.get("scroll_probe_moved"):
                report["failures"].append(f"terminal scroll expected movement but did not move: {scroll_data!r}")
            after_scroll = scroll_data.get("after") if isinstance(scroll_data.get("after"), dict) else {}
            if scroll_data.get("movement_expected") and scroll_data.get("scroll_probe_moved"):
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
