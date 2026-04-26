#!/usr/bin/env python3
import argparse
import json
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


def read_stat(pid):
    try:
        stat = (pathlib.Path("/proc") / str(pid) / "stat").read_text()
    except Exception:
        return None
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
    for pid in interesting_pids():
        stat = read_stat(pid)
        if not stat:
            continue
        rows[str(pid)] = {
            "pid": pid,
            "ppid": stat["ppid"],
            "comm": read_comm(pid),
            "cmdline": read_cmdline(pid)[:6],
            "jiffies": stat["jiffies"],
        }
    return total, rows


start_total, start_rows = snapshot()
time.sleep(DURATION)
end_total, end_rows = snapshot()
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
print(
    json.dumps(
        {
            "label": LABEL,
            "duration_sec": DURATION,
            "cpu_count": cores,
            "root_pid": ROOT_PID,
            "home": YGGTERM_HOME,
            "total_cpu_percent": round(sum(row["cpu_percent"] for row in rows), 3),
            "rows": rows,
            "start_pids": sorted(int(pid) for pid in start_rows),
            "end_pids": sorted(int(pid) for pid in end_rows),
        }
    )
)
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
    parser.add_argument("--sample-sec", type=float, default=20.0)
    parser.add_argument("--settle-sec", type=float, default=4.0)
    parser.add_argument("--visible-max-cpu", type=float, default=2.5)
    parser.add_argument("--focused-max-cpu", type=float, default=2.5)
    parser.add_argument("--background-max-cpu", type=float, default=1.2)
    parser.add_argument("--refocused-max-cpu", type=float, default=2.5)
    parser.add_argument("--keep-remote-dir", action="store_true")
    return parser.parse_args()


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
        env["WEBKIT_DISABLE_COMPOSITING_MODE"] = "1"
    return env


def state_summary(state: dict) -> dict:
    return {
        "active_session_path": state.get("active_session_path"),
        "active_view_mode": state.get("active_view_mode"),
        "terminal_hosts": len(((state.get("dom") or {}).get("terminal_hosts") or [])),
        "window": state.get("window"),
        "metrics": (state.get("browser") or {}).get("metrics") or {},
    }


def remote_cpu_sample(host: str, label: str, pid: int, home: str, duration_sec: float) -> dict:
    snippet = (
        CPU_SAMPLE_SNIPPET.replace("%ROOT_PID%", str(pid))
        .replace("%YGGTERM_HOME%", repr(home))
        .replace("%DURATION%", repr(float(duration_sec)))
        .replace("%LABEL%", repr(label))
    )
    return linux_smoke.ssh_python_json(host, snippet)


def remote_launch_visible_state_with_fallback(
    host: str,
    remote_bin: str,
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
            remote_bin,
            env,
            timeout_ms=timeout_ms,
            remote_log=remote_log,
        )
        pid = int(launch_payload.get("pid") or 0)
        state = linux_smoke.wait_for_remote_state(
            host, remote_bin, env, pid, timeout_ms, timeout_seconds=30.0
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
        remote_bin,
        env,
        timeout_ms=timeout_ms,
        remote_log=fallback_log,
    )
    fallback_pid = int(fallback_payload.get("pid") or 0)
    state = linux_smoke.wait_for_remote_state(
        host, remote_bin, env, fallback_pid, timeout_ms, timeout_seconds=30.0
    )
    launch_info["mode"] = "direct_shell_fallback"
    launch_info["fallback_response"] = fallback_payload
    return fallback_payload, fallback_pid, state, launch_info


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
            "background_max_cpu": args.background_max_cpu,
            "refocused_max_cpu": args.refocused_max_cpu,
        },
    }
    pid = 0
    remote_bin = ""
    launch_env: dict[str, str] = {}
    try:
        linux_smoke.ssh_shell(args.host, f"mkdir -p {linux_smoke.quote(remote_dir)}")
        summary["cleanup_before"] = linux_smoke.remote_cleanup_owned_clients(args.host, args.timeout_ms)
        launch_env = launch_env_from_session(session_info, args.backend, remote_home)
        summary["launch_env"] = launch_env
        remote_bin = linux_smoke.resolve_launch_target(args.host, args, remote_dir)
        summary["remote_bin"] = remote_bin
        linux_smoke.ssh_shell(args.host, f"mkdir -p {linux_smoke.quote(remote_home)}")
        summary["daemon_prewarm"] = linux_smoke.remote_ensure_local_daemon_ready(
            args.host, remote_bin, launch_env, timeout_seconds=20.0
        )
        launch, pid, state, launch_info = remote_launch_visible_state_with_fallback(
            args.host,
            remote_bin,
            launch_env,
            timeout_ms=args.timeout_ms,
            remote_log=f"{remote_dir}/client.log",
        )
        summary["launch"] = launch
        summary["launch_info"] = launch_info
        summary["pid"] = pid
        time.sleep(args.settle_sec)
        summary["state_after_launch"] = state_summary(state)
        summary["cpu_after_launch_idle_visible"] = remote_cpu_sample(
            args.host, "after_launch_idle_visible", pid, remote_home, args.sample_sec
        )
        session_path = linux_smoke.remote_create_plain_terminal(
            args.host, remote_bin, launch_env, pid, title="CPU Smoke Plain"
        )
        summary["created_session"] = session_path
        time.sleep(args.settle_sec)
        state = linux_smoke.wait_for_remote_state(
            args.host, remote_bin, launch_env, pid, args.timeout_ms, timeout_seconds=30.0
        )
        summary["state_after_terminal_create"] = state_summary(state)
        summary["cpu_terminal_focused_idle"] = remote_cpu_sample(
            args.host, "terminal_focused_idle", pid, remote_home, args.sample_sec
        )
        summary["background_response"] = linux_smoke.remote_background_window(
            args.host, remote_bin, launch_env, pid, args.timeout_ms
        )
        time.sleep(args.settle_sec)
        state = linux_smoke.wait_for_remote_state(
            args.host,
            remote_bin,
            launch_env,
            pid,
            args.timeout_ms,
            timeout_seconds=20.0,
            require_visible=False,
        )
        summary["state_after_background"] = state_summary(state)
        background_window = summary["state_after_background"].get("window") or {}
        background_effective = (
            background_window.get("focused") is False or background_window.get("minimized") is True
        )
        summary["background_effective"] = background_effective
        summary["cpu_terminal_background_idle"] = remote_cpu_sample(
            args.host, "terminal_background_idle", pid, remote_home, args.sample_sec
        )
        focus_proc = linux_smoke.ssh_shell(
            args.host,
            f"{linux_smoke.remote_env_exports(launch_env)}; "
            f"{linux_smoke.quote(remote_bin)} server app focus --pid {pid} --timeout-ms {args.timeout_ms}",
            check=False,
        )
        summary["focus_returncode"] = focus_proc.returncode
        summary["focus_stderr"] = (focus_proc.stderr or "").strip()
        time.sleep(args.settle_sec)
        state = linux_smoke.wait_for_remote_state(
            args.host, remote_bin, launch_env, pid, args.timeout_ms, timeout_seconds=30.0
        )
        summary["state_after_refocus"] = state_summary(state)
        summary["cpu_terminal_refocused_idle"] = remote_cpu_sample(
            args.host, "terminal_refocused_idle", pid, remote_home, args.sample_sec
        )
        budgets = {
            "cpu_after_launch_idle_visible": args.visible_max_cpu,
            "cpu_terminal_focused_idle": args.focused_max_cpu,
            "cpu_terminal_background_idle": (
                args.background_max_cpu if background_effective else args.focused_max_cpu
            ),
            "cpu_terminal_refocused_idle": args.refocused_max_cpu,
        }
        failures = []
        for key, max_cpu in budgets.items():
            sample = summary.get(key) or {}
            total = float(sample.get("total_cpu_percent") or 0.0)
            if total > max_cpu:
                failures.append({"sample": key, "total_cpu_percent": total, "max_cpu": max_cpu})
        summary["failures"] = failures
        summary["ok"] = not failures
    except Exception as exc:
        summary["error"] = str(exc)
        summary["failures"] = [{"error": str(exc)}]
        summary["ok"] = False
    finally:
        if pid and remote_bin:
            try:
                summary["close_response"] = linux_smoke.remote_close_window(
                    args.host, remote_bin, launch_env, pid, args.timeout_ms
                )
                linux_smoke.wait_for_remote_pid_gone(args.host, pid, timeout_seconds=8.0)
            except Exception as exc:
                summary["close_error"] = str(exc)
                linux_smoke.remote_kill_pid(args.host, pid)
        if remote_bin and launch_env.get("YGGTERM_HOME"):
            linux_smoke.ssh_shell(
                args.host,
                f"export YGGTERM_HOME={linux_smoke.quote(launch_env['YGGTERM_HOME'])}; "
                f"{linux_smoke.quote(remote_bin)} server shutdown >/dev/null 2>&1 || true",
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
