#!/usr/bin/env python3
import argparse
import json
import os
import sys
import time
from pathlib import Path

import remote_linux_x11_smoke as linux_smoke


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ARTIFACT = ROOT / "target" / "release" / "yggterm"

CPU_SAMPLE_SNIPPET = r"""
import json
import pathlib
import time

ROOT_PID = int(%ROOT_PID%)
YGGTERM_HOME = %YGGTERM_HOME%
DURATION = float(%DURATION%)
LABEL = %LABEL%
ENV_SUBSET_KEYS = {
    "DISPLAY",
    "GDK_BACKEND",
    "WAYLAND_DISPLAY",
    "WEBKIT_DISABLE_DMABUF_RENDERER",
    "WINIT_UNIX_BACKEND",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_TYPE",
    "YGGTERM_ENABLE_XTERM_CANVAS",
    "YGGTERM_HOME",
    "YGGTERM_XTERM_CANVAS_POLICY",
}


def read_total_jiffies():
    line = pathlib.Path("/proc/stat").read_text().splitlines()[0]
    return sum(int(part) for part in line.split()[1:] if part.isdigit())


def cpu_count():
    return max(
        1,
        sum(
            1
            for line in pathlib.Path("/proc/stat").read_text().splitlines()
            if line.startswith("cpu") and len(line) > 3 and line[3].isdigit()
        ),
    )


def read_cmdline(pid):
    try:
        return [
            part.decode("utf-8", "ignore")
            for part in (pathlib.Path("/proc") / str(pid) / "cmdline").read_bytes().split(b"\0")
            if part
        ]
    except Exception:
        return []


def read_environ(pid):
    result = {}
    try:
        for raw in (pathlib.Path("/proc") / str(pid) / "environ").read_bytes().split(b"\0"):
            if raw and b"=" in raw:
                key, value = raw.split(b"=", 1)
                result[key.decode("utf-8", "ignore")] = value.decode("utf-8", "ignore")
    except Exception:
        pass
    return result


def read_env_subset(pid):
    env = read_environ(pid)
    return {key: env[key] for key in sorted(ENV_SUBSET_KEYS) if key in env}


def read_stat(pid):
    try:
        stat = (pathlib.Path("/proc") / str(pid) / "stat").read_text()
    except Exception:
        return None
    return parse_stat_text(stat)


def parse_stat_text(stat):
    fields = stat.rsplit(")", 1)[1].split()
    try:
        return {
            "ppid": int(fields[1]),
            "jiffies": int(fields[11]) + int(fields[12]),
        }
    except Exception:
        return None


def read_comm(pid):
    try:
        return (pathlib.Path("/proc") / str(pid) / "comm").read_text().strip()
    except Exception:
        return ""


def read_task_comm(pid, tid):
    try:
        return (pathlib.Path("/proc") / str(pid) / "task" / str(tid) / "comm").read_text().strip()
    except Exception:
        return ""


def read_task_stat(pid, tid):
    try:
        stat = (pathlib.Path("/proc") / str(pid) / "task" / str(tid) / "stat").read_text()
    except Exception:
        return None
    return parse_stat_text(stat)


def all_pids():
    return [int(path.name) for path in pathlib.Path("/proc").iterdir() if path.name.isdigit()]


def descendants(root):
    mapping = {}
    for pid in all_pids():
        stat = read_stat(pid)
        if stat:
            mapping.setdefault(stat["ppid"], []).append(pid)
    result = set()
    stack = [root]
    while stack:
        parent = stack.pop()
        for child in mapping.get(parent, []):
            if child in result:
                continue
            result.add(child)
            stack.append(child)
    return result


def interesting_pids():
    pids = {ROOT_PID} | descendants(ROOT_PID)
    for pid in all_pids():
        env = read_environ(pid)
        cmdline = read_cmdline(pid)
        comm = read_comm(pid)
        if env.get("YGGTERM_HOME") == YGGTERM_HOME:
            pids.add(pid)
        elif "yggterm" in comm.lower() and any(YGGTERM_HOME in part for part in cmdline):
            pids.add(pid)
    return sorted(pid for pid in pids if (pathlib.Path("/proc") / str(pid)).exists())


def snapshot():
    total = read_total_jiffies()
    rows = {}
    thread_rows = {}
    for pid in interesting_pids():
        stat = read_stat(pid)
        if not stat:
            continue
        comm = read_comm(pid)
        cmdline = read_cmdline(pid)[:6]
        rows[str(pid)] = {
            "pid": pid,
            "ppid": stat["ppid"],
            "comm": comm,
            "cmdline": cmdline,
            "env": read_env_subset(pid),
            "jiffies": stat["jiffies"],
        }
        task_dir = pathlib.Path("/proc") / str(pid) / "task"
        try:
            task_ids = [int(path.name) for path in task_dir.iterdir() if path.name.isdigit()]
        except Exception:
            task_ids = []
        for tid in task_ids:
            task_stat = read_task_stat(pid, tid)
            if not task_stat:
                continue
            thread_rows[f"{pid}:{tid}"] = {
                "pid": pid,
                "tid": tid,
                "comm": read_task_comm(pid, tid),
                "process_comm": comm,
                "cmdline": cmdline,
                "jiffies": task_stat["jiffies"],
            }
    return total, rows, thread_rows


start_ts_ms = int(time.time() * 1000)
start_monotonic_ms = int(time.monotonic() * 1000)
start_total, start_rows, start_thread_rows = snapshot()
time.sleep(DURATION)
end_total, end_rows, end_thread_rows = snapshot()
end_ts_ms = int(time.time() * 1000)
end_monotonic_ms = int(time.monotonic() * 1000)
cores = cpu_count()
denominator = max(1, end_total - start_total)
rows = []
for pid_text, end_row in end_rows.items():
    start_row = start_rows.get(pid_text)
    start_jiffies = start_row.get("jiffies", end_row["jiffies"]) if start_row else end_row["jiffies"]
    delta = max(0, end_row["jiffies"] - start_jiffies)
    row = dict(end_row)
    row["delta_jiffies"] = delta
    row["cpu_percent"] = round((delta / denominator) * cores * 100.0, 3)
    rows.append(row)
rows.sort(key=lambda row: (-row["cpu_percent"], row["comm"], row["pid"]))
threads = []
for key, end_row in end_thread_rows.items():
    start_row = start_thread_rows.get(key)
    start_jiffies = start_row.get("jiffies", end_row["jiffies"]) if start_row else end_row["jiffies"]
    delta = max(0, end_row["jiffies"] - start_jiffies)
    row = dict(end_row)
    row["delta_jiffies"] = delta
    row["cpu_percent"] = round((delta / denominator) * cores * 100.0, 3)
    if delta > 0:
        threads.append(row)
threads.sort(key=lambda row: (-row["cpu_percent"], row["process_comm"], row["comm"], row["tid"]))
app_cpu_percent = 0.0
workload_cpu_percent = 0.0
for row in rows:
    comm = str(row.get("comm") or "").lower()
    cmdline = " ".join(str(part) for part in row.get("cmdline") or []).lower()
    if "yggterm" in comm or "webkit" in comm or "yggterm" in cmdline or "webkit" in cmdline:
        app_cpu_percent += float(row.get("cpu_percent") or 0.0)
    else:
        workload_cpu_percent += float(row.get("cpu_percent") or 0.0)
print(
    json.dumps(
        {
            "label": LABEL,
            "duration_sec": DURATION,
            "start_ts_ms": start_ts_ms,
            "end_ts_ms": end_ts_ms,
            "start_monotonic_ms": start_monotonic_ms,
            "end_monotonic_ms": end_monotonic_ms,
            "cpu_count": cores,
            "root_pid": ROOT_PID,
            "home": YGGTERM_HOME,
            "total_cpu_percent": round(sum(row["cpu_percent"] for row in rows), 3),
            "app_cpu_percent": round(app_cpu_percent, 3),
            "workload_cpu_percent": round(workload_cpu_percent, 3),
            "rows": rows,
            "thread_rows": threads[:32],
            "start_pids": sorted(int(pid) for pid in start_rows),
            "end_pids": sorted(int(pid) for pid in end_rows),
        }
    )
)
"""

HOST_BASELINE_SNIPPET = r"""
import json
import os
import pathlib
import time

DURATION = float(%DURATION%)
LABEL = %LABEL%
TOKENS = tuple(token.strip().lower() for token in %TOKENS% if token.strip())
TOP_N = int(%TOP_N%)
PAGE_SIZE = os.sysconf("SC_PAGE_SIZE")


def read_total_jiffies():
    try:
        line = pathlib.Path("/proc/stat").read_text().splitlines()[0]
        return sum(int(part) for part in line.split()[1:] if part.isdigit())
    except Exception:
        return 0


def cpu_count():
    try:
        return max(
            1,
            sum(
                1
                for line in pathlib.Path("/proc/stat").read_text().splitlines()
                if line.startswith("cpu") and len(line) > 3 and line[3].isdigit()
            ),
        )
    except Exception:
        return 1


def read_cmdline(pid):
    try:
        return (
            pathlib.Path("/proc")
            .joinpath(str(pid), "cmdline")
            .read_bytes()
            .replace(b"\0", b" ")
            .decode("utf-8", "replace")
            .strip()
        )
    except Exception:
        return ""


def read_comm(pid):
    try:
        return pathlib.Path("/proc").joinpath(str(pid), "comm").read_text().strip()
    except Exception:
        return ""


def read_env_subset(pid):
    wanted = {
        "YGGTERM_HOME",
        "YGGTERM_DESKTOP_APP_ID_SUFFIX",
        "YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF",
        "DISPLAY",
        "XAUTHORITY",
    }
    result = {}
    try:
        for raw in pathlib.Path("/proc").joinpath(str(pid), "environ").read_bytes().split(b"\0"):
            if raw and b"=" in raw:
                key, value = raw.split(b"=", 1)
                key_text = key.decode("utf-8", "replace")
                if key_text in wanted:
                    result[key_text] = value.decode("utf-8", "replace")
    except Exception:
        pass
    return result


def read_stat(pid):
    try:
        fields = pathlib.Path("/proc").joinpath(str(pid), "stat").read_text().rsplit(")", 1)[1].split()
        return {
            "ppid": int(fields[1]),
            "jiffies": int(fields[11]) + int(fields[12]),
            "rss_kb": int(fields[21]) * PAGE_SIZE // 1024,
        }
    except Exception:
        return None


def all_pids():
    return [int(path.name) for path in pathlib.Path("/proc").iterdir() if path.name.isdigit()]


def snapshot():
    total = read_total_jiffies()
    rows = {}
    for pid in all_pids():
        stat = read_stat(pid)
        if not stat:
            continue
        comm = read_comm(pid)
        cmdline = read_cmdline(pid)
        hay = f"{comm} {cmdline}".lower()
        rows[str(pid)] = {
            "pid": pid,
            "ppid": stat["ppid"],
            "comm": comm,
            "cmd": cmdline[:260],
            "jiffies": stat["jiffies"],
            "rss_kb": stat["rss_kb"],
            "matches_tokens": any(token in hay for token in TOKENS),
        }
    return total, rows


start_total, start_rows = snapshot()
start_ts_ms = int(time.time() * 1000)
time.sleep(DURATION)
end_total, end_rows = snapshot()
end_ts_ms = int(time.time() * 1000)
cores = cpu_count()
denominator = max(1, end_total - start_total)
rows = []
for pid_text, row in end_rows.items():
    start = start_rows.get(pid_text)
    start_jiffies = start.get("jiffies", row["jiffies"]) if start else row["jiffies"]
    delta = max(0, row["jiffies"] - start_jiffies)
    out = dict(row)
    out["delta_jiffies"] = delta
    out["cpu_percent"] = round((delta / denominator) * cores * 100.0, 3)
    if out["matches_tokens"] or out["cpu_percent"] > 0:
        out["env"] = read_env_subset(int(pid_text)) if out["matches_tokens"] else {}
        rows.append(out)
rows.sort(key=lambda row: (-row["cpu_percent"], row["comm"], row["pid"]))
matched = [row for row in rows if row.get("matches_tokens")]
system_top = rows[:TOP_N]
print(json.dumps({
    "label": LABEL,
    "duration_sec": DURATION,
    "start_ts_ms": start_ts_ms,
    "end_ts_ms": end_ts_ms,
    "cpu_count": cores,
    "matched_cpu_percent": round(sum(float(row.get("cpu_percent") or 0) for row in matched), 3),
    "system_top_cpu_percent": round(sum(float(row.get("cpu_percent") or 0) for row in system_top), 3),
    "matched": matched[:TOP_N],
    "system_top": system_top,
    "tokens": TOKENS,
}))
"""


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Stage Yggterm on a remote Linux desktop and fail if idle CPU exceeds budget."
    )
    parser.add_argument("--host", required=True)
    parser.add_argument("--proxy-jump")
    parser.add_argument("--ssh-port", type=int)
    parser.add_argument("--artifact", default=str(DEFAULT_ARTIFACT))
    parser.add_argument("--remote-bin")
    parser.add_argument("--backend", choices=("x11", "wayland"), default="x11")
    parser.add_argument("--out-dir")
    parser.add_argument("--remote-dir")
    parser.add_argument("--timeout-ms", type=int, default=20000)
    parser.add_argument("--baseline-sec", type=float, default=5.0)
    parser.add_argument("--no-baseline", action="store_true")
    parser.add_argument("--sample-sec", type=float, default=20.0)
    parser.add_argument("--settle-sec", type=float, default=10.0)
    parser.add_argument(
        "--post-state-cooldown-sec",
        type=float,
        default=2.0,
        help="Wait this long after app-control state probes before starting a CPU sample.",
    )
    parser.add_argument("--visible-max-cpu", type=float, default=2.5)
    parser.add_argument("--focused-max-cpu", type=float, default=2.5)
    parser.add_argument("--tui-max-cpu", type=float, default=18.0)
    parser.add_argument("--background-max-cpu", type=float, default=1.2)
    parser.add_argument("--background-tui-max-cpu", type=float, default=6.0)
    parser.add_argument("--refocused-max-cpu", type=float, default=2.5)
    parser.add_argument(
        "--long-quiet-soak-sec",
        type=float,
        default=0.0,
        help="After backgrounding, keep the GUI idle for this many seconds and sample CPU again.",
    )
    parser.add_argument("--long-quiet-max-cpu", type=float, default=1.2)
    parser.add_argument("--skip-tui", action="store_true")
    parser.add_argument(
        "--scrollback-lines",
        type=int,
        default=0,
        help="After creating the terminal, print this many lines before idle CPU sampling.",
    )
    parser.add_argument("--keep-remote-dir", action="store_true")
    return parser.parse_args()


def record_phase(summary: dict[str, object], phase: str, **fields: object) -> None:
    events = summary.setdefault("phase_events", [])
    if isinstance(events, list):
        events.append(
            {
                "phase": phase,
                "ts_ms": int(time.time() * 1000),
                "monotonic_ms": int(time.monotonic() * 1000),
                **fields,
            }
        )


def launch_env_from_session(session_info: dict, backend: str, remote_home: str) -> dict[str, str]:
    picked = session_info.get("picked_session") or {}
    leader_env = session_info.get("leader_env") or {}
    desktop_env = session_info.get("desktop_env") or leader_env
    runtime_dir = str(
        leader_env.get("XDG_RUNTIME_DIR")
        or desktop_env.get("XDG_RUNTIME_DIR")
        or session_info.get("runtime_dir")
        or ""
    ).strip()
    if not runtime_dir:
        raise RuntimeError(f"could not resolve XDG_RUNTIME_DIR: {session_info!r}")
    dbus_bus = str(leader_env.get("DBUS_SESSION_BUS_ADDRESS") or "").strip()
    if not dbus_bus:
        dbus_bus = f"unix:path={runtime_dir}/bus"
    env = {
        "DBUS_SESSION_BUS_ADDRESS": dbus_bus,
        "XDG_RUNTIME_DIR": runtime_dir,
        "YGGTERM_HOME": remote_home,
        "YGGTERM_ALLOW_MULTI_WINDOW": "1",
        "YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF": "1",
        "YGGTERM_POINTER_DRIVER": "app",
        "YGGTERM_KEY_DRIVER": "app",
        "YGGTERM_APP_CONTROL_SKIP_X11_SYNTHETIC_INPUT": "1",
        "YGGTERM_REMOTE_SMOKE_TAG": "1",
        "NO_AT_BRIDGE": "1",
    }
    for passthrough_key in (
        "YGGTERM_ENABLE_WEBKIT_COMPOSITING",
        "YGGTERM_ENABLE_XTERM_CANVAS",
        "YGGTERM_FORCE_X11_BACKEND",
        "YGGTERM_TERMINAL_WRITE_FRAME_MS",
        "YGGTERM_TERMINAL_ACTIVE_WRITE_FRAME_MS",
        "YGGTERM_TRACE_TERMINAL_READS",
        "YGGTERM_TRACE_RENDER",
    ):
        passthrough_value = os.environ.get(passthrough_key)
        if passthrough_value:
            env[passthrough_key] = passthrough_value
    picked_session_id = str(picked.get("session_id") or picked.get("Name") or "").strip()
    if picked_session_id:
        env["XDG_SESSION_ID"] = picked_session_id
    if picked.get("Type"):
        env["XDG_SESSION_TYPE"] = str(picked.get("Type"))
    if picked.get("Class"):
        env["XDG_SESSION_CLASS"] = str(picked.get("Class"))
    for key in ("DESKTOP_SESSION", "XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP", "KDE_FULL_SESSION"):
        value = str(desktop_env.get(key) or "").strip()
        if value:
            env[key] = value
    screenshot_wayland = str(
        desktop_env.get("WAYLAND_DISPLAY") or leader_env.get("WAYLAND_DISPLAY") or ""
    ).strip()
    if screenshot_wayland:
        env["YGGTERM_SCREENSHOT_WAYLAND_DISPLAY"] = screenshot_wayland
    if backend == "wayland":
        wayland_display = screenshot_wayland
        if not wayland_display:
            for candidate in session_info.get("wayland_sockets") or []:
                rendered = str(candidate or "").strip()
                if rendered:
                    wayland_display = rendered
                    break
        if not wayland_display:
            raise RuntimeError(f"could not resolve WAYLAND_DISPLAY: {session_info!r}")
        env["WAYLAND_DISPLAY"] = wayland_display
        env["GDK_BACKEND"] = "wayland"
        display = str(desktop_env.get("DISPLAY") or leader_env.get("DISPLAY") or "").strip()
        xauthority = str(desktop_env.get("XAUTHORITY") or leader_env.get("XAUTHORITY") or "").strip()
        if display:
            env["DISPLAY"] = display
        if xauthority:
            env["XAUTHORITY"] = xauthority
    else:
        display = str(
            session_info.get("xwayland_display")
            or desktop_env.get("DISPLAY")
            or leader_env.get("DISPLAY")
            or ""
        ).strip()
        xauthority = str(
            session_info.get("xwayland_xauthority")
            or desktop_env.get("XAUTHORITY")
            or leader_env.get("XAUTHORITY")
            or ""
        ).strip()
        if not display or not xauthority:
            raise RuntimeError(f"could not resolve X11 env: {session_info!r}")
        env["DISPLAY"] = display
        env["XAUTHORITY"] = xauthority
        env["GDK_BACKEND"] = "x11"
        if not env.get("YGGTERM_ENABLE_WEBKIT_COMPOSITING"):
            env["WEBKIT_DISABLE_COMPOSITING_MODE"] = "1"
    return env


def state_summary(state: dict) -> dict:
    dom = state.get("dom") or {}
    terminal_hosts = dom.get("terminal_hosts") or []
    active_hosts = dom.get("active_terminal_hosts") or []
    shell = state.get("shell") or {}

    def compact_host(host: dict) -> dict:
        return {
            "id": host.get("id"),
            "session_path": host.get("session_path"),
            "active": host.get("active"),
            "input_enabled": host.get("input_enabled"),
            "programmatic_focus_enabled": host.get("programmatic_focus_enabled"),
            "terminal_write_frame_ms": host.get("terminal_write_frame_ms"),
            "terminal_active_write_frame_ms": host.get("terminal_active_write_frame_ms"),
            "terminal_active_animation_write_frame_ms": host.get(
                "terminal_active_animation_write_frame_ms"
            ),
            "effective_terminal_write_frame_ms": host.get(
                "effective_terminal_write_frame_ms"
            ),
            "active_write_frame_budget": host.get("active_write_frame_budget"),
            "recent_frame_like_write_hot": host.get("recent_frame_like_write_hot"),
            "recent_inline_status_animation_hot": host.get(
                "recent_inline_status_animation_hot"
            ),
            "document_focused": host.get("document_focused"),
            "host_input_focused": host.get("host_input_focused"),
            "text_sample": str(host.get("text_sample") or "")[-240:],
            "text_tail": str(host.get("text_tail") or "")[-240:],
            "buffer_text_sample": str(host.get("buffer_text_sample") or "")[-240:],
            "cursor_line_text": str(host.get("cursor_line_text") or "")[-240:],
            "render_event_count": host.get("render_event_count"),
            "write_command_count": host.get("write_command_count"),
            "write_bridge_flush_count": host.get("write_bridge_flush_count"),
            "write_parsed_count": host.get("write_parsed_count"),
            "skipped_perf_event_count": host.get("skipped_perf_event_count"),
            "last_skipped_perf_event_name": host.get("last_skipped_perf_event_name"),
            "hot_host_health_suppressed_count": host.get("hot_host_health_suppressed_count"),
            "write_bridge_in_flight": host.get("write_bridge_in_flight"),
            "write_bridge_pending_chars": host.get("write_bridge_pending_chars"),
            "last_raw_payload_length": host.get("last_raw_payload_length"),
            "last_raw_payload_line_count": host.get("last_raw_payload_line_count"),
            "read_nudge_count": host.get("read_nudge_count"),
            "last_read_nudge_reason": host.get("last_read_nudge_reason"),
            "last_read_nudge_at_ms": host.get("last_read_nudge_at_ms"),
            "retained_replay_expected": host.get("retained_replay_expected"),
            "retained_replay_source": host.get("retained_replay_source"),
            "retained_replay_prompt_follow_ready": host.get(
                "retained_replay_prompt_follow_ready"
            ),
            "last_retained_replay_follow_debug": host.get(
                "last_retained_replay_follow_debug"
            ),
            "scrollback_expected": host.get("scrollback_expected"),
            "scrollback_locked": host.get("scrollback_locked"),
            "scrollback_intent": host.get("scrollback_intent"),
            "base_y": host.get("base_y"),
            "viewport_y": host.get("viewport_y"),
            "last_viewport_force_debug": host.get("last_viewport_force_debug"),
            "last_write_sample": str(host.get("last_write_sample") or "")[-240:],
            "last_write_applied_tail": str(host.get("last_write_applied_tail") or "")[-240:],
            "xterm_canvas_renderer_requested": host.get("xterm_canvas_renderer_requested"),
            "xterm_renderer_mode": host.get("xterm_renderer_mode"),
            "canvas_count": host.get("canvas_count"),
            "visible_canvas_layer_count": host.get("visible_canvas_layer_count"),
            "hidden_canvas_layer_count": host.get("hidden_canvas_layer_count"),
            "software_canvas_layer_optimization_active": host.get(
                "software_canvas_layer_optimization_active"
            ),
            "software_canvas_hidden_layer_count": host.get(
                "software_canvas_hidden_layer_count"
            ),
            "software_canvas_visible_layer_count": host.get(
                "software_canvas_visible_layer_count"
            ),
            "software_canvas_input_line_overlay_present": host.get(
                "software_canvas_input_line_overlay_present"
            ),
            "software_canvas_input_line_overlay_visible": host.get(
                "software_canvas_input_line_overlay_visible"
            ),
            "software_canvas_cursor_overlay_present": host.get(
                "software_canvas_cursor_overlay_present"
            ),
            "software_canvas_cursor_overlay_visible": host.get(
                "software_canvas_cursor_overlay_visible"
            ),
            "xterm_input_line_decoration_present": host.get(
                "xterm_input_line_decoration_present"
            ),
            "xterm_input_line_decoration_visible": host.get(
                "xterm_input_line_decoration_visible"
            ),
            "xterm_input_line_decoration_line": host.get(
                "xterm_input_line_decoration_line"
            ),
            "xterm_input_line_decoration_width": host.get(
                "xterm_input_line_decoration_width"
            ),
            "xterm_input_line_decoration_background": host.get(
                "xterm_input_line_decoration_background"
            ),
            "xterm_input_line_decoration_error": host.get(
                "xterm_input_line_decoration_error"
            ),
            "xterm_input_line_decoration_disposed": host.get(
                "xterm_input_line_decoration_disposed"
            ),
            "xterm_input_line_decoration_marker_line": host.get(
                "xterm_input_line_decoration_marker_line"
            ),
            "xterm_input_line_decoration_element_visible": host.get(
                "xterm_input_line_decoration_element_visible"
            ),
            "xterm_input_line_decoration_element_background": host.get(
                "xterm_input_line_decoration_element_background"
            ),
            "xterm_input_line_decoration_render_count": host.get(
                "xterm_input_line_decoration_render_count"
            ),
            "software_canvas_layer_optimization_reason": host.get(
                "software_canvas_layer_optimization_reason"
            ),
            "low_power_tui_overlay_present": host.get("low_power_tui_overlay_present"),
            "low_power_tui_overlay_active": host.get("low_power_tui_overlay_active"),
            "low_power_tui_frame_count": host.get("low_power_tui_frame_count"),
            "low_power_tui_text_sample": str(host.get("low_power_tui_text_sample") or "")[-240:],
            "inactive_tui_frame_drop_count": host.get("inactive_tui_frame_drop_count"),
            "inactive_tui_last_tail": str(host.get("inactive_tui_last_tail") or "")[-240:],
            "unfocused_tui_frame_drop_count": host.get("unfocused_tui_frame_drop_count"),
            "unfocused_tui_last_tail": str(host.get("unfocused_tui_last_tail") or "")[-240:],
            "rect": host.get("rect"),
            "host_rect": host.get("host_rect"),
            "screen_rect": host.get("screen_rect"),
            "viewport_rect": host.get("viewport_rect"),
        }

    active_session_path = state.get("active_session_path")
    fallback_active_hosts: list[dict] = []
    if not active_hosts and active_session_path:
        fallback_active_hosts = [
            host
            for host in terminal_hosts
            if isinstance(host, dict)
            and host.get("session_path") == active_session_path
            and (
                host.get("active") is True
                or host.get("active_write_frame_budget") is True
                or host.get("input_enabled") is True
                or host.get("effective_input_focus") is True
            )
        ]
        active_hosts = fallback_active_hosts

    return {
        "active_session_path": active_session_path,
        "active_view_mode": state.get("active_view_mode"),
        "active_session_terminal_process_id": state.get(
            "active_session_terminal_process_id"
        ),
        "active_session_terminal_foreground_active": state.get(
            "active_session_terminal_foreground_active"
        ),
        "dom_snapshot": {
            "snapshot_mode": dom.get("snapshot_mode"),
            "degraded_reason": dom.get("degraded_reason"),
            "css_animation_count": dom.get("css_animation_count"),
            "css_running_animation_count": dom.get("css_running_animation_count"),
            "css_animation_samples": (dom.get("css_animation_samples") or [])[:8],
            "terminal_host_count": dom.get("terminal_host_count"),
            "active_terminal_host_count": dom.get("active_terminal_host_count"),
            "active_terminal_host_fallback_count": len(fallback_active_hosts),
            "sidebar_visible_row_count": dom.get("sidebar_visible_row_count"),
        },
        "terminal_hosts": len(terminal_hosts),
        "active_terminal_hosts": len(active_hosts),
        "terminal_host_details": [compact_host(host) for host in terminal_hosts[:6]],
        "active_terminal_host_details": [compact_host(host) for host in active_hosts[:6]],
        "active_terminal_input_enabled": any(
            bool(host.get("input_enabled")) for host in active_hosts
        ),
        "shell_window_focused": shell.get("window_focused"),
        "shell_app_control_backgrounded": shell.get("app_control_backgrounded"),
        "shell_app_control_backgrounded_at_ms": shell.get(
            "app_control_backgrounded_at_ms"
        ),
        "window": state.get("window"),
        "metrics": (state.get("browser") or {}).get("metrics") or {},
    }


def _counter_delta(before: dict, after: dict, key: str) -> int | None:
    before_value = before.get(key)
    after_value = after.get(key)
    if before_value is None or after_value is None:
        return None
    try:
        return max(0, int(after_value) - int(before_value))
    except (TypeError, ValueError):
        return None


def render_counter_delta(before: dict, after: dict, elapsed_sec: float) -> dict:
    duration = max(0.001, float(elapsed_sec or 0.0))
    before_metrics = before.get("metrics") or {}
    after_metrics = after.get("metrics") or {}
    before_hosts = before.get("terminal_host_details") or []
    after_hosts = after.get("terminal_host_details") or []
    before_host = before_hosts[0] if before_hosts else {}
    after_host = after_hosts[0] if after_hosts else {}
    result = {"duration_sec": duration}
    for output_key, source_before, source_after, counter_key in (
        ("root_render_delta", before_metrics, after_metrics, "root_render_count"),
        ("browser_rebuild_delta", before_metrics, after_metrics, "rebuild_count"),
        ("xterm_render_event_delta", before_host, after_host, "render_event_count"),
        ("xterm_write_command_delta", before_host, after_host, "write_command_count"),
        ("xterm_write_flush_delta", before_host, after_host, "write_bridge_flush_count"),
        ("xterm_write_parsed_delta", before_host, after_host, "write_parsed_count"),
        ("skipped_perf_event_delta", before_host, after_host, "skipped_perf_event_count"),
        (
            "hot_host_health_suppressed_delta",
            before_host,
            after_host,
            "hot_host_health_suppressed_count",
        ),
    ):
        delta = _counter_delta(source_before, source_after, counter_key)
        result[output_key] = delta
        result[f"{output_key}_per_sec"] = None if delta is None else delta / duration
    return result


def background_tui_throttled_without_render_progress(
    summary: dict,
    *,
    session_path: str,
    cpu_sample: dict,
    max_cpu: float,
    render_delta: dict,
) -> tuple[bool, dict]:
    host = _select_terminal_host(summary, session_path)
    if not host:
        return False, {"reason": "no_terminal_host"}
    app_cpu = float(
        cpu_sample.get("app_cpu_percent")
        or cpu_sample.get("total_cpu_percent")
        or 0.0
    )
    workload_cpu = float(cpu_sample.get("workload_cpu_percent") or 0.0)
    xterm_delta_keys = (
        "xterm_render_event_delta",
        "xterm_write_command_delta",
        "xterm_write_flush_delta",
        "xterm_write_parsed_delta",
    )
    xterm_deltas = {key: render_delta.get(key) for key in xterm_delta_keys}
    xterm_counters_flat = all(value == 0 for value in xterm_deltas.values())
    app_control_backgrounded = summary.get("shell_app_control_backgrounded") is True
    backgrounded_low_power_contract = (
        app_control_backgrounded
        and summary.get("active_terminal_input_enabled") is not True
        and host.get("input_enabled") is not True
        and host.get("active_write_frame_budget") is not True
        and int(host.get("effective_terminal_write_frame_ms") or 0) >= 1000
    )
    workload_still_alive = (
        summary.get("active_session_terminal_foreground_active") is True
        or workload_cpu > 0.05
    )
    accepted = (
        backgrounded_low_power_contract
        and xterm_counters_flat
        and app_cpu <= max_cpu
        and workload_still_alive
    )
    return accepted, {
        "reason": (
            "backgrounded_low_power_no_frame_progress"
            if accepted
            else "backgrounded_low_power_contract_not_met"
        ),
        "shell_app_control_backgrounded": summary.get("shell_app_control_backgrounded"),
        "active_terminal_input_enabled": summary.get("active_terminal_input_enabled"),
        "host_input_enabled": host.get("input_enabled"),
        "active_write_frame_budget": host.get("active_write_frame_budget"),
        "effective_terminal_write_frame_ms": host.get(
            "effective_terminal_write_frame_ms"
        ),
        "active_session_terminal_foreground_active": summary.get(
            "active_session_terminal_foreground_active"
        ),
        "workload_cpu_percent": workload_cpu,
        "app_cpu_percent": app_cpu,
        "max_cpu": max_cpu,
        "xterm_deltas": xterm_deltas,
    }


def sample_cpu_with_render_probe(
    summary: dict,
    args: argparse.Namespace,
    *,
    host: str,
    remote_bin: str,
    launch_env: dict[str, str],
    pid: int,
    remote_home: str,
    cpu_key: str,
    label: str,
    before_summary: dict,
    state_key: str,
    delta_key: str,
    require_visible: bool,
) -> dict:
    summary[cpu_key] = remote_cpu_sample(host, label, pid, remote_home, args.sample_sec)
    after_state = linux_smoke.wait_for_remote_state(
        host,
        remote_bin,
        launch_env,
        pid,
        args.timeout_ms,
        timeout_seconds=20.0,
        require_visible=require_visible,
    )
    after_summary = state_summary(after_state)
    summary[state_key] = after_summary
    summary[delta_key] = render_counter_delta(before_summary, after_summary, args.sample_sec)
    return after_summary


def _optional_int(value: object) -> int | None:
    if value is None:
        return None
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def _select_terminal_host(summary: dict, session_path: str) -> dict:
    hosts = list(summary.get("active_terminal_host_details") or [])
    hosts.extend(summary.get("terminal_host_details") or [])
    for host in hosts:
        if host.get("session_path") == session_path:
            return host
    return hosts[0] if hosts else {}


def terminal_mount_contract_failures(
    summary: dict,
    *,
    state_key: str,
    session_path: str,
) -> list[dict]:
    failures: list[dict] = []
    active_session_path = summary.get("active_session_path")
    active_view_mode = summary.get("active_view_mode")
    active_host_count = _optional_int(summary.get("active_terminal_hosts")) or 0
    all_host_count = _optional_int(summary.get("terminal_hosts")) or 0
    active_hosts = summary.get("active_terminal_host_details") or []
    backgrounded_by_app_control = summary.get("shell_app_control_backgrounded") is True
    host = _select_terminal_host(summary, session_path)
    if active_session_path != session_path:
        failures.append(
            {
                "sample": state_key,
                "error": "active_session_mismatch",
                "expected_session_path": session_path,
                "active_session_path": active_session_path,
            }
        )
    if active_view_mode != "Terminal":
        failures.append(
            {
                "sample": state_key,
                "error": "active_view_not_terminal",
                "active_view_mode": active_view_mode,
            }
        )
    if not backgrounded_by_app_control and (active_host_count < 1 or not active_hosts):
        failures.append(
            {
                "sample": state_key,
                "error": "active_terminal_host_missing",
                "active_terminal_hosts": active_host_count,
                "terminal_hosts": all_host_count,
            }
        )
    if not host:
        failures.append(
            {
                "sample": state_key,
                "error": "terminal_host_missing",
                "terminal_hosts": all_host_count,
            }
        )
        return failures
    if host.get("session_path") != session_path:
        failures.append(
            {
                "sample": state_key,
                "error": "terminal_host_session_mismatch",
                "expected_session_path": session_path,
                "host_session_path": host.get("session_path"),
            }
        )
    if backgrounded_by_app_control:
        if summary.get("active_terminal_input_enabled") is True or host.get("input_enabled") is True:
            failures.append(
                {
                    "sample": state_key,
                    "error": "backgrounded_terminal_input_enabled",
                    "active_terminal_input_enabled": summary.get(
                        "active_terminal_input_enabled"
                    ),
                    "host_input_enabled": host.get("input_enabled"),
                }
            )
        if host.get("active_write_frame_budget") is True:
            failures.append(
                {
                    "sample": state_key,
                    "error": "backgrounded_terminal_uses_active_write_budget",
                    "effective_terminal_write_frame_ms": host.get(
                        "effective_terminal_write_frame_ms"
                    ),
                    "terminal_write_frame_ms": host.get("terminal_write_frame_ms"),
                    "active_write_frame_budget": host.get("active_write_frame_budget"),
                }
            )
    requested = host.get("xterm_canvas_renderer_requested")
    mode = host.get("xterm_renderer_mode")
    if requested is True:
        if mode != "canvas":
            failures.append(
                {
                    "sample": state_key,
                    "error": "xterm_canvas_requested_but_not_mounted",
                    "xterm_renderer_mode": mode,
                }
            )
            return failures
        visible_layers = _optional_int(host.get("visible_canvas_layer_count"))
        if host.get("software_canvas_layer_optimization_active") is not True:
            failures.append(
                {
                    "sample": state_key,
                    "error": "software_canvas_layer_optimization_inactive",
                    "visible_canvas_layer_count": visible_layers,
                    "hidden_canvas_layer_count": host.get("hidden_canvas_layer_count"),
                    "reason": host.get("software_canvas_layer_optimization_reason"),
                }
            )
        if visible_layers is not None and visible_layers > 2:
            failures.append(
                {
                    "sample": state_key,
                    "error": "too_many_visible_canvas_layers",
                    "visible_canvas_layer_count": visible_layers,
                    "hidden_canvas_layer_count": host.get("hidden_canvas_layer_count"),
                }
            )
        overlay_flags = {
            "input_line_present": host.get("software_canvas_input_line_overlay_present"),
            "input_line_visible": host.get("software_canvas_input_line_overlay_visible"),
            "cursor_present": host.get("software_canvas_cursor_overlay_present"),
            "cursor_visible": host.get("software_canvas_cursor_overlay_visible"),
        }
        if any(value is True for value in overlay_flags.values()):
            failures.append(
                {
                    "sample": state_key,
                    "error": "software_canvas_prompt_or_cursor_overlay_present",
                    "overlay_flags": overlay_flags,
                }
            )
    base_y = _optional_int(host.get("base_y"))
    viewport_y = _optional_int(host.get("viewport_y"))
    retained_prompt_follow = (
        (host.get("retained_replay_expected") is True or host.get("scrollback_expected") is True)
        and str(host.get("scrollback_intent") or "PromptFollow") != "UserScrollback"
    )
    if (
        retained_prompt_follow
        and base_y is not None
        and viewport_y is not None
        and base_y > 0
        and viewport_y < base_y
    ):
        failures.append(
            {
                "sample": state_key,
                "error": "retained_replay_prompt_follow_stuck_in_scrollback",
                "base_y": base_y,
                "viewport_y": viewport_y,
                "follow": host.get("last_retained_replay_follow_debug"),
                "force": host.get("last_viewport_force_debug"),
            }
        )
    if (
        retained_prompt_follow
        and host.get("retained_replay_source")
        and host.get("retained_replay_prompt_follow_ready") is False
    ):
        failures.append(
            {
                "sample": state_key,
                "error": "retained_replay_prompt_follow_not_ready",
                "source": host.get("retained_replay_source"),
                "follow": host.get("last_retained_replay_follow_debug"),
            }
        )
    return failures


def terminal_quiescent_key(summary: dict) -> tuple:
    hosts = summary.get("terminal_host_details") or []
    if not hosts:
        return tuple()
    host = hosts[0]
    return (
        host.get("render_event_count"),
        host.get("write_command_count"),
        host.get("write_bridge_flush_count"),
        host.get("write_bridge_in_flight"),
        host.get("write_bridge_pending_chars"),
        host.get("recent_frame_like_write_hot"),
        host.get("low_power_tui_overlay_active"),
        host.get("low_power_tui_frame_count"),
        host.get("inactive_tui_frame_drop_count"),
        host.get("unfocused_tui_frame_drop_count"),
        host.get("cursor_line_text"),
        host.get("text_tail"),
    )


def wait_for_terminal_quiescent_state(
    host: str,
    remote_bin: str,
    launch_env: dict[str, str],
    pid: int,
    timeout_ms: int,
    *,
    timeout_seconds: float = 20.0,
    require_visible: bool = False,
    require_cool: bool = False,
) -> tuple[dict, dict]:
    deadline = time.time() + max(1.0, timeout_seconds)
    last_key: tuple | None = None
    stable_polls = 0
    last_summary: dict = {}
    while time.time() < deadline:
        state = linux_smoke.wait_for_remote_state(
            host,
            remote_bin,
            launch_env,
            pid,
            timeout_ms,
            timeout_seconds=5.0,
            require_visible=require_visible,
        )
        summary = state_summary(state)
        key = terminal_quiescent_key(summary)
        hosts = summary.get("terminal_host_details") or []
        host_summary = hosts[0] if hosts else {}
        idle_bridge = (
            host_summary.get("write_bridge_in_flight") is not True
            and int(host_summary.get("write_bridge_pending_chars") or 0) == 0
            and host_summary.get("low_power_tui_overlay_active") is not True
            and (
                not require_cool
                or host_summary.get("recent_frame_like_write_hot") is not True
            )
        )
        if key and key == last_key and idle_bridge:
            stable_polls += 1
            if stable_polls >= 2:
                return state, {
                    "quiescent": True,
                    "stable_polls": stable_polls,
                    "key": key,
                }
        else:
            stable_polls = 0
            last_key = key
        last_summary = summary
        time.sleep(1.0)
    return state, {
        "quiescent": False,
        "stable_polls": stable_polls,
        "last_key": last_key,
        "last_summary": last_summary,
    }


def synthetic_tui_drop_count(summary: dict) -> int:
    hosts = summary.get("terminal_host_details") or []
    if not hosts:
        return 0
    host = hosts[0]
    return sum(
        int(host.get(key) or 0)
        for key in (
            "inactive_tui_frame_drop_count",
            "unfocused_tui_frame_drop_count",
            "low_power_tui_frame_count",
        )
    )


def synthetic_tui_signal(
    summary: dict,
    *,
    baseline_drop_count: int = 0,
    allow_foreground_active: bool = False,
) -> tuple[bool, dict]:
    hosts = summary.get("terminal_host_details") or []
    if not hosts:
        return False, {"reason": "no_terminal_host"}
    host = hosts[0]
    text = "\n".join(
        str(host.get(key) or "")
        for key in ("text_sample", "text_tail", "buffer_text_sample", "cursor_line_text")
    )
    tui_tail = "\n".join(
        str(host.get(key) or "")
        for key in (
            "inactive_tui_last_tail",
            "unfocused_tui_last_tail",
            "low_power_tui_text_sample",
        )
    )
    visible_has_frame = (
        "Yggterm synthetic TUI CPU smoke frame" in text
        and any(f"{row:02d} [" in text for row in range(1, 42))
    )
    dropped_has_frame = any(f"{row:02d} [" in tui_tail for row in range(1, 42))
    drop_count = synthetic_tui_drop_count(summary)
    drop_delta = max(0, drop_count - max(0, int(baseline_drop_count or 0)))
    if visible_has_frame:
        return True, {
            "reason": "visible_frame_text",
            "drop_count": drop_count,
            "baseline_drop_count": baseline_drop_count,
            "drop_delta": drop_delta,
        }
    if drop_delta > 0 and dropped_has_frame:
        return True, {
            "reason": "frame_drop_counter",
            "drop_count": drop_count,
            "baseline_drop_count": baseline_drop_count,
            "drop_delta": drop_delta,
        }
    if drop_delta > 0 and (
        host.get("low_power_tui_overlay_active") is True
        or host.get("low_power_tui_overlay_present") is True
    ):
        return True, {
            "reason": "low_power_frame_counter_advanced",
            "drop_count": drop_count,
            "baseline_drop_count": baseline_drop_count,
            "drop_delta": drop_delta,
            "low_power_tui_overlay_active": host.get("low_power_tui_overlay_active"),
            "low_power_tui_overlay_present": host.get("low_power_tui_overlay_present"),
        }
    if (
        allow_foreground_active
        and summary.get("active_session_terminal_foreground_active") is True
    ):
        return True, {
            "reason": "foreground_process_active",
            "drop_count": drop_count,
            "baseline_drop_count": baseline_drop_count,
            "drop_delta": drop_delta,
            "cursor_line_text": host.get("cursor_line_text"),
            "text_tail": host.get("text_tail"),
        }
    return False, {
        "reason": "waiting_for_frame",
        "drop_count": drop_count,
        "baseline_drop_count": baseline_drop_count,
        "drop_delta": drop_delta,
        "cursor_line_text": host.get("cursor_line_text"),
        "text_tail": host.get("text_tail"),
    }


def wait_for_synthetic_tui_started(
    host: str,
    remote_bin: str,
    launch_env: dict[str, str],
    pid: int,
    timeout_ms: int,
    *,
    baseline_drop_count: int = 0,
    timeout_seconds: float = 14.0,
    require_visible: bool = False,
    allow_foreground_active: bool = False,
) -> tuple[dict, dict]:
    deadline = time.time() + max(1.0, timeout_seconds)
    last_summary: dict = {}
    last_signal: dict = {}
    while time.time() < deadline:
        state = linux_smoke.wait_for_remote_state(
            host,
            remote_bin,
            launch_env,
            pid,
            timeout_ms,
            timeout_seconds=5.0,
            require_visible=require_visible,
        )
        summary = state_summary(state)
        started, signal = synthetic_tui_signal(
            summary,
            baseline_drop_count=baseline_drop_count,
            allow_foreground_active=allow_foreground_active,
        )
        if started:
            return state, {"started": True, **signal}
        last_summary = summary
        last_signal = signal
        time.sleep(0.5)
    return state, {
        "started": False,
        "last_signal": last_signal,
        "last_summary": last_summary,
    }


def remote_cpu_sample(host: str, label: str, pid: int, home: str, duration_sec: float) -> dict:
    snippet = (
        CPU_SAMPLE_SNIPPET.replace("%ROOT_PID%", str(pid))
        .replace("%YGGTERM_HOME%", repr(home))
        .replace("%DURATION%", repr(float(duration_sec)))
        .replace("%LABEL%", repr(label))
    )
    return linux_smoke.ssh_python_json(host, snippet)


def remote_host_baseline_sample(host: str, label: str, duration_sec: float) -> dict:
    snippet = (
        HOST_BASELINE_SNIPPET.replace("%DURATION%", repr(float(duration_sec)))
        .replace("%LABEL%", repr(label))
        .replace("%TOKENS%", repr(["yggterm", "codex", "webkit", "xterm", "ssh"]))
        .replace("%TOP_N%", repr(20))
    )
    return linux_smoke.ssh_python_json(host, snippet)


def cooldown_before_cpu_sample(args: argparse.Namespace) -> None:
    cooldown = max(0.0, float(args.post_state_cooldown_sec or 0.0))
    if cooldown > 0:
        time.sleep(cooldown)


def synthetic_tui_command(duration_sec: float) -> str:
    duration = max(6.0, float(duration_sec))
    return f"""python3 - <<'PY'
import shutil
import sys
import time

duration = {duration!r}
size = shutil.get_terminal_size((100, 32))
row_count = max(8, min(size.lines - 2, 42))
bar_width = max(24, min(size.columns - 14, 96))
start = time.time()
frame = 0
sys.stdout.write("\\x1b[?1049h\\x1b[?25l\\x1b[2J")
try:
    while time.time() - start < duration:
        frame += 1
        sys.stdout.write("\\x1b[H")
        sys.stdout.write(f"Yggterm synthetic TUI CPU smoke frame {{frame}}\\x1b[K\\r\\n")
        for row in range(1, row_count):
            filled = (frame + row * 3) % bar_width
            bar = ("#" * filled).ljust(bar_width, ".")
            pct = round((filled / max(1, bar_width)) * 100)
            sys.stdout.write(f"{{row:02d}} [{{bar}}] {{pct:02d}}%\\x1b[K\\r\\n")
        sys.stdout.flush()
        time.sleep(0.08)
finally:
    sys.stdout.write("\\x1b[?25h\\x1b[?1049l\\r\\n")
    sys.stdout.flush()
PY
"""


def scrollback_fill_command(line_count: int) -> str:
    count = max(0, int(line_count))
    return (
        "python3 -c 'import sys; "
        f"count={count!r}; "
        "[sys.stdout.write(\"YGGTERM_SCROLLBACK_FILL %05d %s\\n\" % (i, \".\" * 96)) "
        "for i in range(count)]; "
        "sys.stdout.write(\"YGGTERM_SCROLLBACK_FILL_DONE %d\\n\" % count); "
        "sys.stdout.flush()'"
    )


def remote_terminal_send(
    host: str,
    control_bin: str,
    env: dict[str, str],
    pid: int,
    session_path: str,
    data: str,
    timeout_ms: int,
) -> dict:
    proc = linux_smoke.ssh_shell(
        host,
        f"{linux_smoke.remote_env_exports(env)}; "
        f"{linux_smoke.quote(control_bin)} server app terminal send --pid {pid} "
        f"{linux_smoke.quote(session_path)} --stdin "
        f"--timeout-ms {timeout_ms}",
        check=False,
        input_text=data,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            proc.stderr.strip()
            or proc.stdout.strip()
            or f"terminal send failed for {session_path}"
        )
    return json.loads(proc.stdout.strip() or "{}")


def remote_terminal_write(
    host: str,
    control_bin: str,
    env: dict[str, str],
    session_path: str,
    data: str,
) -> dict:
    proc = linux_smoke.ssh_shell(
        host,
        f"{linux_smoke.remote_env_exports(env)}; "
        f"{linux_smoke.quote(control_bin)} server terminal write "
        f"{linux_smoke.quote(session_path)} --stdin",
        check=False,
        input_text=data,
    )
    if proc.returncode != 0:
        raise RuntimeError(
            proc.stderr.strip()
            or proc.stdout.strip()
            or f"terminal write failed for {session_path}"
        )
    return json.loads(proc.stdout.strip() or "{}")


def clear_terminal_prompt(
    host: str,
    control_bin: str,
    env: dict[str, str],
    pid: int,
    session_path: str,
    timeout_ms: int,
) -> dict:
    return remote_terminal_send(host, control_bin, env, pid, session_path, "\x15", timeout_ms)


def remote_launch_visible_state_with_fallback(
    host: str,
    launch_bin: str,
    control_bin: str,
    env: dict[str, str],
    *,
    timeout_ms: int,
    remote_log: str,
) -> tuple[dict, int, dict, dict]:
    launch_info: dict[str, object] = {
        "mode": "app_cli",
        "visibility_error": None,
        "app_cli_cleanup": None,
        "fallback_response": None,
    }
    pid = 0
    try:
        launch_payload = linux_smoke.remote_launch_visible_window(
            host,
            launch_bin,
            env,
            timeout_ms=timeout_ms,
            remote_log=remote_log,
        )
        pid = int(launch_payload.get("pid") or 0)
        state = linux_smoke.wait_for_remote_state(
            host, control_bin, env, pid, timeout_ms, timeout_seconds=30.0
        )
        launch_info["response"] = launch_payload
        return launch_payload, pid, state, launch_info
    except RuntimeError as exc:
        launch_info["visibility_error"] = str(exc)
        if pid:
            linux_smoke.remote_kill_pid(host, pid)
        launch_info["app_cli_cleanup"] = linux_smoke.remote_kill_yggterm_processes_for_home(
            host,
            env.get("YGGTERM_HOME", ""),
        )

    fallback_log = f"{remote_log}.direct"
    fallback_payload = linux_smoke.remote_launch_direct_window(
        host,
        launch_bin,
        env,
        timeout_ms=timeout_ms,
        remote_log=fallback_log,
    )
    fallback_pid = int(fallback_payload.get("pid") or 0)
    state = linux_smoke.wait_for_remote_state(
        host, control_bin, env, fallback_pid, timeout_ms, timeout_seconds=30.0
    )
    launch_info["mode"] = "direct_shell_fallback"
    launch_info["fallback_response"] = fallback_payload
    return fallback_payload, fallback_pid, state, launch_info


def remote_control_bin_for_launch_target(host: str, launch_bin: str) -> str:
    script = f"""
bin={linux_smoke.quote(launch_bin)}
dir="${{bin%/*}}"
base="${{bin##*/}}"
candidates="$dir/yggterm-headless"
case "$base" in
  yggterm-*) candidates="$candidates $dir/yggterm-headless-${{base#yggterm-}}" ;;
esac
for candidate in $candidates; do
  if [ -x "$candidate" ]; then
    printf '%s\\n' "$candidate"
    exit 0
  fi
done
printf 'missing matched yggterm-headless sibling for %s\\n' "$bin" >&2
exit 1
"""
    proc = linux_smoke.ssh_shell(host, script, check=False)
    if proc.returncode != 0:
        raise RuntimeError((proc.stderr or proc.stdout).strip())
    control_bin = (proc.stdout or "").strip().splitlines()[-1].strip()
    if not control_bin:
        raise RuntimeError(f"empty control binary path for {launch_bin}")
    return control_bin


def main() -> int:
    args = parse_args()
    linux_smoke.configure_remote_transport(args.proxy_jump, args.ssh_port)
    timestamp = time.strftime("%Y%m%d-%H%M%S")
    out_dir = Path(args.out_dir or f"/tmp/yggterm-remote-{args.host}-idle-cpu-{timestamp}")
    out_dir.mkdir(parents=True, exist_ok=True)
    session_info = linux_smoke.ssh_python_json(args.host, linux_smoke.LINUX_SESSION_SNIPPET)
    remote_base = str(session_info.get("home_dir") or "").rstrip("/")
    if not remote_base:
        raise RuntimeError(f"could not resolve remote home dir: {session_info!r}")
    remote_dir = args.remote_dir or f"{remote_base}/.cache/yggterm-remote-idle-cpu-{timestamp}"
    remote_home = f"{remote_dir}/home"
    summary: dict[str, object] = {
        "host": args.host,
        "timestamp": timestamp,
        "backend": args.backend,
        "out_dir": str(out_dir),
        "remote_dir": remote_dir,
        "thresholds": {
            "visible_max_cpu": args.visible_max_cpu,
            "focused_max_cpu": args.focused_max_cpu,
            "tui_max_cpu": args.tui_max_cpu,
            "background_max_cpu": args.background_max_cpu,
            "background_tui_max_cpu": args.background_tui_max_cpu,
            "refocused_max_cpu": args.refocused_max_cpu,
            "post_state_cooldown_sec": args.post_state_cooldown_sec,
            "baseline_sec": 0.0 if args.no_baseline else args.baseline_sec,
        },
    }
    pid = 0
    launch_bin = ""
    control_bin = ""
    launch_env: dict[str, str] = {}
    try:
        record_phase(summary, "pretest_baseline_start", duration_sec=0.0 if args.no_baseline else args.baseline_sec)
        if not args.no_baseline and args.baseline_sec > 0:
            summary["pretest_host_resource_baseline"] = remote_host_baseline_sample(
                args.host,
                "pretest_host_resource_baseline",
                args.baseline_sec,
            )
        record_phase(summary, "pretest_baseline_end")
        linux_smoke.ssh_shell(args.host, f"mkdir -p {linux_smoke.quote(remote_dir)}")
        record_phase(summary, "cleanup_before_start")
        summary["cleanup_before"] = linux_smoke.remote_cleanup_owned_clients(args.host, args.timeout_ms)
        record_phase(summary, "cleanup_before_end")
        launch_env = launch_env_from_session(session_info, args.backend, remote_home)
        summary["launch_env"] = launch_env
        launch_bin = linux_smoke.resolve_launch_target(args.host, args, remote_dir)
        control_bin = remote_control_bin_for_launch_target(args.host, launch_bin)
        summary["remote_bin"] = launch_bin
        summary["control_bin"] = control_bin
        linux_smoke.ssh_shell(args.host, f"mkdir -p {linux_smoke.quote(remote_home)}")
        record_phase(summary, "daemon_prewarm_start")
        summary["daemon_prewarm"] = linux_smoke.remote_ensure_local_daemon_ready(
            args.host, control_bin, launch_env, timeout_seconds=20.0
        )
        record_phase(summary, "daemon_prewarm_end")
        record_phase(summary, "app_launch_start")
        launch, pid, state, launch_info = remote_launch_visible_state_with_fallback(
            args.host,
            launch_bin,
            control_bin,
            launch_env,
            timeout_ms=args.timeout_ms,
            remote_log=f"{remote_dir}/client.log",
        )
        summary["launch"] = launch
        summary["launch_info"] = launch_info
        summary["pid"] = pid
        record_phase(summary, "app_launch_end", pid=pid)
        time.sleep(max(args.settle_sec, 8.0))
        summary["state_after_launch"] = state_summary(state)
        cooldown_before_cpu_sample(args)
        record_phase(summary, "cpu_after_launch_idle_visible_start")
        sample_cpu_with_render_probe(
            summary,
            args,
            host=args.host,
            remote_bin=control_bin,
            launch_env=launch_env,
            pid=pid,
            remote_home=remote_home,
            cpu_key="cpu_after_launch_idle_visible",
            label="after_launch_idle_visible",
            before_summary=summary["state_after_launch"],
            state_key="state_after_launch_idle_visible_sample",
            delta_key="after_launch_idle_visible_render_counter_delta",
            require_visible=True,
        )
        record_phase(summary, "cpu_after_launch_idle_visible_end")
        record_phase(summary, "terminal_create_start")
        session_path = linux_smoke.remote_create_plain_terminal(
            args.host, control_bin, launch_env, pid, title="CPU Smoke Plain"
        )
        summary["created_session"] = session_path
        record_phase(summary, "terminal_create_end", session_path=session_path)
        focus_proc = linux_smoke.ssh_shell(
            args.host,
            f"{linux_smoke.remote_env_exports(launch_env)}; "
            f"{linux_smoke.quote(control_bin)} server app focus --pid {pid} --timeout-ms {args.timeout_ms}",
            check=False,
        )
        summary["focus_after_terminal_create_returncode"] = focus_proc.returncode
        summary["focus_after_terminal_create_stderr"] = (focus_proc.stderr or "").strip()
        time.sleep(args.settle_sec)
        state = linux_smoke.wait_for_remote_state(
            args.host, control_bin, launch_env, pid, args.timeout_ms, timeout_seconds=30.0
        )
        summary["state_after_terminal_create"] = state_summary(state)
        summary["clear_prompt_after_terminal_create"] = clear_terminal_prompt(
            args.host,
            control_bin,
            launch_env,
            pid,
            session_path,
            args.timeout_ms,
        )
        if args.scrollback_lines > 0:
            record_phase(summary, "scrollback_fill_start", line_count=args.scrollback_lines)
            summary["scrollback_fill_write"] = remote_terminal_write(
                args.host,
                control_bin,
                launch_env,
                session_path,
                scrollback_fill_command(args.scrollback_lines) + "\r",
            )
            scrollback_state, scrollback_quiescence = wait_for_terminal_quiescent_state(
                args.host,
                control_bin,
                launch_env,
                pid,
                args.timeout_ms,
                timeout_seconds=max(12.0, args.settle_sec + 20.0),
                require_visible=True,
                require_cool=True,
            )
            summary["scrollback_fill_quiescence"] = scrollback_quiescence
            summary["state_after_scrollback_fill"] = state_summary(scrollback_state)
            record_phase(summary, "scrollback_fill_end")
        cooldown_before_cpu_sample(args)
        record_phase(summary, "cpu_terminal_focused_idle_start")
        sample_cpu_with_render_probe(
            summary,
            args,
            host=args.host,
            remote_bin=control_bin,
            launch_env=launch_env,
            pid=pid,
            remote_home=remote_home,
            cpu_key="cpu_terminal_focused_idle",
            label="terminal_focused_idle",
            before_summary=summary.get("state_after_scrollback_fill")
            or summary["state_after_terminal_create"],
            state_key="state_after_terminal_focused_idle_sample",
            delta_key="terminal_focused_idle_render_counter_delta",
            require_visible=True,
        )
        record_phase(summary, "cpu_terminal_focused_idle_end")
        if not args.skip_tui:
            tui_duration = args.sample_sec + 12.0
            record_phase(summary, "active_tui_start", duration_sec=tui_duration)
            summary["clear_prompt_before_synthetic_tui"] = clear_terminal_prompt(
                args.host,
                control_bin,
                launch_env,
                pid,
                session_path,
                args.timeout_ms,
            )
            summary["synthetic_tui_send"] = remote_terminal_send(
                args.host,
                control_bin,
                launch_env,
                pid,
                session_path,
                synthetic_tui_command(tui_duration) + "\r",
                args.timeout_ms,
            )
            active_tui_baseline_drop_count = synthetic_tui_drop_count(
                summary.get("state_after_terminal_focused_idle_sample")
                or summary.get("state_after_terminal_create")
                or {}
            )
            tui_state, tui_start = wait_for_synthetic_tui_started(
                args.host,
                control_bin,
                launch_env,
                pid,
                args.timeout_ms,
                baseline_drop_count=active_tui_baseline_drop_count,
                timeout_seconds=max(8.0, args.settle_sec + 12.0),
            )
            summary["synthetic_tui_start"] = tui_start
            summary["state_during_synthetic_tui"] = state_summary(tui_state)
            if not tui_start.get("started"):
                raise RuntimeError(
                    f"synthetic TUI did not start before active sample: {tui_start!r}"
                )
            cooldown_before_cpu_sample(args)
            record_phase(summary, "cpu_terminal_active_tui_start")
            summary["cpu_terminal_active_tui"] = remote_cpu_sample(
                args.host, "terminal_active_tui", pid, remote_home, args.sample_sec
            )
            record_phase(summary, "cpu_terminal_active_tui_end")
            active_tui_after_sample_state = linux_smoke.wait_for_remote_state(
                args.host,
                control_bin,
                launch_env,
                pid,
                args.timeout_ms,
                timeout_seconds=20.0,
                require_visible=True,
            )
            summary["state_after_synthetic_tui_sample"] = state_summary(
                active_tui_after_sample_state
            )
            summary["active_tui_render_counter_delta"] = render_counter_delta(
                summary["state_during_synthetic_tui"],
                summary["state_after_synthetic_tui_sample"],
                args.sample_sec,
            )
            remote_terminal_send(
                args.host,
                control_bin,
                launch_env,
                pid,
                session_path,
                "\u0003",
                args.timeout_ms,
            )
            record_phase(summary, "active_tui_end")
        record_phase(summary, "background_start")
        summary["background_response"] = linux_smoke.remote_background_window(
            args.host, control_bin, launch_env, pid, args.timeout_ms
        )
        time.sleep(args.settle_sec)
        state = linux_smoke.wait_for_remote_state(
            args.host,
            control_bin,
            launch_env,
            pid,
            args.timeout_ms,
            timeout_seconds=20.0,
            require_visible=False,
        )
        summary["state_after_background"] = state_summary(state)
        background_window = summary["state_after_background"].get("window") or {}
        background_shell_focused = summary["state_after_background"].get("shell_window_focused")
        background_app_control = summary["state_after_background"].get(
            "shell_app_control_backgrounded"
        )
        background_effective = (
            background_window.get("focused") is False or background_window.get("minimized") is True
            or background_shell_focused is False
            or background_app_control is True
        )
        summary["background_effective"] = background_effective
        record_phase(summary, "background_end", effective=background_effective)
        quiescent_state, quiescence = wait_for_terminal_quiescent_state(
            args.host,
            control_bin,
            launch_env,
            pid,
            args.timeout_ms,
            timeout_seconds=max(6.0, args.settle_sec + 10.0),
            require_visible=False,
            require_cool=True,
        )
        summary["background_quiescence"] = quiescence
        summary["state_after_background_quiescent"] = state_summary(quiescent_state)
        post_background_tui_state = quiescent_state
        if not args.skip_tui:
            background_tui_duration = args.sample_sec + args.settle_sec + 28.0
            record_phase(summary, "background_tui_start", duration_sec=background_tui_duration)
            record_phase(summary, "background_tui_focus_start")
            focus_proc = linux_smoke.ssh_shell(
                args.host,
                f"{linux_smoke.remote_env_exports(launch_env)}; "
                f"{linux_smoke.quote(control_bin)} server app focus --pid {pid} --timeout-ms {args.timeout_ms}",
                check=False,
            )
            summary["focus_before_background_tui_returncode"] = focus_proc.returncode
            summary["focus_before_background_tui_stderr"] = (focus_proc.stderr or "").strip()
            time.sleep(args.settle_sec)
            background_tui_launch_state = linux_smoke.wait_for_remote_state(
                args.host,
                control_bin,
                launch_env,
                pid,
                args.timeout_ms,
                timeout_seconds=20.0,
                require_visible=True,
            )
            summary["state_before_background_tui_launch"] = state_summary(
                background_tui_launch_state
            )
            record_phase(summary, "background_tui_focus_end")
            summary["clear_prompt_before_background_tui"] = clear_terminal_prompt(
                args.host,
                control_bin,
                launch_env,
                pid,
                session_path,
                args.timeout_ms,
            )
            summary["synthetic_tui_background_send"] = remote_terminal_send(
                args.host,
                control_bin,
                launch_env,
                pid,
                session_path,
                synthetic_tui_command(background_tui_duration) + "\r",
                args.timeout_ms,
            )
            background_tui_baseline_drop_count = synthetic_tui_drop_count(
                summary.get("state_before_background_tui_launch") or {}
            )
            background_tui_active_state, background_tui_active_start = (
                wait_for_synthetic_tui_started(
                    args.host,
                    control_bin,
                    launch_env,
                    pid,
                    args.timeout_ms,
                    baseline_drop_count=background_tui_baseline_drop_count,
                    timeout_seconds=max(8.0, args.settle_sec + 12.0),
                    require_visible=True,
                )
            )
            summary["synthetic_tui_background_active_start"] = background_tui_active_start
            summary["state_before_background_tui_background"] = state_summary(
                background_tui_active_state
            )
            if not background_tui_active_start.get("started"):
                raise RuntimeError(
                    "synthetic TUI did not start before backgrounding for background sample: "
                    f"{background_tui_active_start!r}"
                )
            summary["background_response_during_tui"] = (
                linux_smoke.remote_background_window(
                    args.host, control_bin, launch_env, pid, args.timeout_ms
                )
            )
            time.sleep(args.settle_sec)
            background_tui_baseline_drop_count = synthetic_tui_drop_count(
                summary.get("state_before_background_tui_background") or {}
            )
            background_tui_state, background_tui_start = wait_for_synthetic_tui_started(
                args.host,
                control_bin,
                launch_env,
                pid,
                args.timeout_ms,
                baseline_drop_count=background_tui_baseline_drop_count,
                timeout_seconds=max(14.0, args.settle_sec + 18.0),
                require_visible=False,
                allow_foreground_active=True,
            )
            summary["synthetic_tui_background_start"] = background_tui_start
            background_tui_before_sample_summary = state_summary(background_tui_state)
            summary["state_before_background_tui_sample"] = background_tui_before_sample_summary
            if not background_tui_start.get("started"):
                raise RuntimeError(
                    "synthetic TUI did not start before background sample: "
                    f"{background_tui_start!r}"
                )
            cooldown_before_cpu_sample(args)
            record_phase(summary, "cpu_terminal_background_tui_start")
            summary["cpu_terminal_background_tui"] = remote_cpu_sample(
                args.host, "terminal_background_tui", pid, remote_home, args.sample_sec
            )
            record_phase(summary, "cpu_terminal_background_tui_end")
            background_tui_after_sample_state = linux_smoke.wait_for_remote_state(
                args.host,
                control_bin,
                launch_env,
                pid,
                args.timeout_ms,
                timeout_seconds=20.0,
                require_visible=False,
            )
            background_tui_summary = state_summary(background_tui_after_sample_state)
            background_tui_still_running, background_tui_after_signal = synthetic_tui_signal(
                background_tui_summary,
                baseline_drop_count=synthetic_tui_drop_count(
                    background_tui_before_sample_summary
                ),
                allow_foreground_active=True,
            )
            summary["synthetic_tui_background_after_sample"] = {
                "started": background_tui_still_running,
                **background_tui_after_signal,
            }
            summary["state_during_background_tui"] = background_tui_summary
            summary["background_tui_render_counter_delta"] = render_counter_delta(
                background_tui_before_sample_summary,
                summary["state_during_background_tui"],
                args.sample_sec,
            )
            if not background_tui_still_running:
                throttled_ok, throttled_signal = (
                    background_tui_throttled_without_render_progress(
                        background_tui_summary,
                        session_path=session_path,
                        cpu_sample=summary["cpu_terminal_background_tui"],
                        max_cpu=args.background_tui_max_cpu,
                        render_delta=summary["background_tui_render_counter_delta"],
                    )
                )
                summary["synthetic_tui_background_after_sample"][
                    "background_throttle_check"
                ] = throttled_signal
                if throttled_ok:
                    summary["synthetic_tui_background_after_sample"]["started"] = True
                    summary["synthetic_tui_background_after_sample"]["reason"] = (
                        throttled_signal["reason"]
                    )
                    background_tui_still_running = True
            if not background_tui_still_running:
                raise RuntimeError(
                    "synthetic TUI stopped before background sample finished: "
                    f"{summary['synthetic_tui_background_after_sample']!r}"
                )
            summary["synthetic_tui_background_interrupt"] = remote_terminal_send(
                args.host,
                control_bin,
                launch_env,
                pid,
                session_path,
                "\u0003",
                args.timeout_ms,
            )
            time.sleep(1.0)
            post_background_tui_state, post_background_tui_quiescence = wait_for_terminal_quiescent_state(
                args.host,
                control_bin,
                launch_env,
                pid,
                args.timeout_ms,
                timeout_seconds=max(14.0, args.settle_sec + 18.0),
                require_visible=False,
                require_cool=True,
            )
            summary["post_background_tui_quiescence"] = post_background_tui_quiescence
            background_tui_drain_wait_sec = max(18.0, args.post_state_cooldown_sec)
            summary["background_tui_drain_wait_sec"] = background_tui_drain_wait_sec
            time.sleep(background_tui_drain_wait_sec)
            post_background_tui_state, post_background_tui_drain_quiescence = wait_for_terminal_quiescent_state(
                args.host,
                control_bin,
                launch_env,
                pid,
                args.timeout_ms,
                timeout_seconds=max(8.0, args.settle_sec + 8.0),
                require_visible=False,
                require_cool=True,
            )
            summary["post_background_tui_drain_quiescence"] = (
                post_background_tui_drain_quiescence
            )
            record_phase(summary, "background_tui_end")
        summary["state_after_background_tui_quiescent"] = state_summary(
            post_background_tui_state
        )
        cooldown_before_cpu_sample(args)
        record_phase(summary, "cpu_terminal_background_idle_start")
        sample_cpu_with_render_probe(
            summary,
            args,
            host=args.host,
            remote_bin=control_bin,
            launch_env=launch_env,
            pid=pid,
            remote_home=remote_home,
            cpu_key="cpu_terminal_background_idle",
            label="terminal_background_idle",
            before_summary=summary["state_after_background_tui_quiescent"],
            state_key="state_after_terminal_background_idle_sample",
            delta_key="terminal_background_idle_render_counter_delta",
            require_visible=False,
        )
        record_phase(summary, "cpu_terminal_background_idle_end")
        if args.long_quiet_soak_sec > 0:
            record_phase(summary, "terminal_long_quiet_soak_start", duration_sec=args.long_quiet_soak_sec)
            time.sleep(args.long_quiet_soak_sec)
            summary["state_after_long_quiet_soak"] = state_summary(
                linux_smoke.wait_for_remote_state(
                    args.host,
                    control_bin,
                    launch_env,
                    pid,
                    args.timeout_ms,
                    timeout_seconds=20.0,
                    require_visible=False,
                )
            )
            record_phase(summary, "terminal_long_quiet_soak_end")
            cooldown_before_cpu_sample(args)
            record_phase(summary, "cpu_terminal_long_quiet_soak_start")
            sample_cpu_with_render_probe(
                summary,
                args,
                host=args.host,
                remote_bin=control_bin,
                launch_env=launch_env,
                pid=pid,
                remote_home=remote_home,
                cpu_key="cpu_terminal_long_quiet_soak",
                label="terminal_long_quiet_soak",
                before_summary=summary["state_after_long_quiet_soak"],
                state_key="state_after_terminal_long_quiet_soak_sample",
                delta_key="terminal_long_quiet_soak_render_counter_delta",
                require_visible=False,
            )
            record_phase(summary, "cpu_terminal_long_quiet_soak_end")
        record_phase(summary, "refocus_start")
        focus_proc = linux_smoke.ssh_shell(
            args.host,
            f"{linux_smoke.remote_env_exports(launch_env)}; "
            f"{linux_smoke.quote(control_bin)} server app focus --pid {pid} --timeout-ms {args.timeout_ms}",
            check=False,
        )
        summary["focus_returncode"] = focus_proc.returncode
        summary["focus_stderr"] = (focus_proc.stderr or "").strip()
        time.sleep(args.settle_sec)
        state = linux_smoke.wait_for_remote_state(
            args.host, control_bin, launch_env, pid, args.timeout_ms, timeout_seconds=30.0
        )
        summary["state_after_refocus"] = state_summary(state)
        refocus_quiescent_state, refocus_quiescence = wait_for_terminal_quiescent_state(
            args.host,
            control_bin,
            launch_env,
            pid,
            args.timeout_ms,
            timeout_seconds=max(6.0, args.settle_sec + 10.0),
            require_visible=False,
            require_cool=True,
        )
        summary["refocus_quiescence"] = refocus_quiescence
        summary["state_after_refocus_quiescent"] = state_summary(refocus_quiescent_state)
        record_phase(summary, "refocus_end")
        cooldown_before_cpu_sample(args)
        record_phase(summary, "cpu_terminal_refocused_idle_start")
        sample_cpu_with_render_probe(
            summary,
            args,
            host=args.host,
            remote_bin=control_bin,
            launch_env=launch_env,
            pid=pid,
            remote_home=remote_home,
            cpu_key="cpu_terminal_refocused_idle",
            label="terminal_refocused_idle",
            before_summary=summary["state_after_refocus_quiescent"],
            state_key="state_after_terminal_refocused_idle_sample",
            delta_key="terminal_refocused_idle_render_counter_delta",
            require_visible=True,
        )
        record_phase(summary, "cpu_terminal_refocused_idle_end")
        budgets = {
            "cpu_after_launch_idle_visible": args.visible_max_cpu,
            "cpu_terminal_focused_idle": args.focused_max_cpu,
            "cpu_terminal_active_tui": args.tui_max_cpu,
            "cpu_terminal_background_tui": args.background_tui_max_cpu,
            "cpu_terminal_background_idle": args.background_max_cpu,
            "cpu_terminal_refocused_idle": args.refocused_max_cpu,
        }
        if args.long_quiet_soak_sec > 0:
            budgets["cpu_terminal_long_quiet_soak"] = args.long_quiet_max_cpu
        failures = []
        if not background_effective:
            failures.append(
                {
                    "sample": "state_after_background",
                    "error": "background_window_not_effective",
                    "window": background_window,
                }
            )
        terminal_state_keys = [
            "state_after_terminal_create",
            "state_after_scrollback_fill",
            "state_after_terminal_focused_idle_sample",
            "state_after_background",
            "state_after_background_quiescent",
            "state_after_background_tui_quiescent",
            "state_after_terminal_background_idle_sample",
            "state_after_refocus",
            "state_after_refocus_quiescent",
            "state_after_terminal_refocused_idle_sample",
        ]
        if not args.skip_tui:
            terminal_state_keys.extend(
                [
                    "state_during_synthetic_tui",
                    "state_after_synthetic_tui_sample",
                    "state_before_background_tui_sample",
                    "state_during_background_tui",
                ]
            )
        if args.long_quiet_soak_sec > 0:
            terminal_state_keys.extend(
                [
                    "state_after_long_quiet_soak",
                    "state_after_terminal_long_quiet_soak_sample",
                ]
            )
        for state_key in terminal_state_keys:
            sample = summary.get(state_key)
            if isinstance(sample, dict):
                failures.extend(
                    terminal_mount_contract_failures(
                        sample,
                        state_key=state_key,
                        session_path=session_path,
                    )
                )
        for key, max_cpu in budgets.items():
            sample = summary.get(key) or {}
            total = float(sample.get("total_cpu_percent") or 0.0)
            app_total = float(sample.get("app_cpu_percent") or total)
            if app_total > max_cpu:
                failures.append(
                    {
                        "sample": key,
                        "app_cpu_percent": app_total,
                        "total_cpu_percent": total,
                        "workload_cpu_percent": float(sample.get("workload_cpu_percent") or 0.0),
                        "max_cpu": max_cpu,
                    }
                )
            for row in sample.get("rows") or []:
                comm = str(row.get("comm") or "").lower()
                env = row.get("env") or {}
                gdk_backend = str(env.get("GDK_BACKEND") or "").strip()
                canvas_policy = str(env.get("YGGTERM_XTERM_CANVAS_POLICY") or "").strip()
                if (
                    "webkit" in comm
                    and gdk_backend == "x11"
                    and canvas_policy == "xterm_canvas_enabled_for_wayland"
                ):
                    failures.append(
                        {
                            "sample": key,
                            "error": "x11_webkit_canvas_backend_mismatch",
                            "pid": row.get("pid"),
                            "comm": row.get("comm"),
                            "gdk_backend": gdk_backend,
                            "xterm_canvas_policy": canvas_policy,
                            "wayland_display": env.get("WAYLAND_DISPLAY"),
                        }
                    )
        summary["failures"] = failures
        summary["ok"] = not failures
    except Exception as exc:
        summary["error"] = str(exc)
        summary["failures"] = [{"error": str(exc)}]
        summary["ok"] = False
    finally:
        if pid and control_bin:
            try:
                summary["close_response"] = linux_smoke.remote_close_window(
                    args.host, control_bin, launch_env, pid, args.timeout_ms
                )
                linux_smoke.wait_for_remote_pid_gone(args.host, pid, timeout_seconds=8.0)
            except Exception as exc:
                summary["close_error"] = str(exc)
                linux_smoke.remote_kill_pid(args.host, pid)
        if control_bin and launch_env.get("YGGTERM_HOME"):
            linux_smoke.ssh_shell(
                args.host,
                f"export YGGTERM_HOME={linux_smoke.quote(launch_env['YGGTERM_HOME'])}; "
                f"{linux_smoke.quote(control_bin)} server shutdown >/dev/null 2>&1 || true",
                check=False,
            )
        summary["cleanup_after"] = linux_smoke.remote_cleanup_owned_clients(args.host, args.timeout_ms)
        if not args.keep_remote_dir:
            linux_smoke.ssh_shell(args.host, f"rm -rf {linux_smoke.quote(remote_dir)}", check=False)
    summary_path = out_dir / "summary.json"
    linux_smoke.write_json(summary_path, summary)
    print(json.dumps({"summary_path": str(summary_path), "ok": summary.get("ok"), "failures": summary.get("failures")}, indent=2))
    return 0 if summary.get("ok") else 1


if __name__ == "__main__":
    raise SystemExit(main())
