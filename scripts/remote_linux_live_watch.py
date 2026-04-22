#!/usr/bin/env python3
import argparse
import json
import time
from pathlib import Path

from remote_linux_x11_smoke import (
    LINUX_SESSION_SNIPPET,
    remote_env_exports,
    resolve_launch_target,
    scp_from,
    scp_to,
    ssh_python_json,
    ssh_shell,
)


PROCESS_SAMPLE_SNIPPET = r"""
import json
import os
import pathlib
import pwd

user = pwd.getpwuid(os.getuid()).pw_name
rows = []
targets = {
    "plasmashell",
    "kwin_wayland",
    "kwin_x11",
    "Xwayland",
    "yggterm",
    "yggterm-headless",
}
ssh_generation_context_count = 0
for pid in os.listdir("/proc"):
    if not pid.isdigit():
        continue
    proc = pathlib.Path("/proc") / pid
    try:
        owner = proc.stat().st_uid
        if owner != os.getuid():
            continue
        comm = (proc / "comm").read_text(encoding="utf-8").strip()
        cmdline = [
            part.decode("utf-8", "ignore")
            for part in (proc / "cmdline").read_bytes().split(b"\0")
            if part
        ]
        cmdline_text = " ".join(cmdline)
        if comm == "ssh" and " server " in f" {cmdline_text} " and " generation-context " in f" {cmdline_text} ":
            ssh_generation_context_count += 1
        if comm not in targets:
            continue
        rss_kb = 0
        for line in (proc / "status").read_text(encoding="utf-8").splitlines():
            if line.startswith("VmRSS:"):
                rss_kb = int(line.split()[1])
                break
        try:
            fd_count = len(os.listdir(proc / "fd"))
        except Exception:
            fd_count = None
        stat_fields = (proc / "stat").read_text(encoding="utf-8").split(") ", 1)[1].split()
        rows.append(
            {
                "pid": int(pid),
                "comm": comm,
                "rss_kb": rss_kb,
                "fd_count": fd_count,
                "state": stat_fields[0],
                "start_ticks": int(stat_fields[19]),
                "cmdline": cmdline,
            }
        )
    except Exception:
        continue
rows.sort(key=lambda row: (row["comm"], row["pid"]))
print(json.dumps({
    "user": user,
    "rows": rows,
    "ssh_generation_context_count": ssh_generation_context_count,
}))
"""


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Watch a live Linux desktop Yggterm session over SSH for redraw or desktop-shell instability."
    )
    parser.add_argument("--host", required=True)
    parser.add_argument("--artifact", default="")
    parser.add_argument("--remote-bin")
    parser.add_argument("--backend", choices=("native", "x11"), default="native")
    parser.add_argument("--duration-sec", type=float, default=90.0)
    parser.add_argument("--poll-sec", type=float, default=2.0)
    parser.add_argument("--timeout-ms", type=int, default=8000)
    parser.add_argument("--out-dir")
    parser.add_argument("--remote-dir")
    parser.add_argument("--reuse-existing-home", action="store_true")
    parser.add_argument("--capture-frame", action="store_true")
    parser.add_argument("--keep-remote-dir", action="store_true")
    return parser.parse_args()


def launch_env_from_session(
    session_info: dict, backend: str, remote_home: str | None
) -> dict[str, str]:
    picked = session_info.get("picked_session") or {}
    leader_env = session_info.get("leader_env") or {}
    session_type = str(picked.get("Type") or "").strip()
    runtime_dir = str(
        leader_env.get("XDG_RUNTIME_DIR") or session_info.get("runtime_dir") or ""
    ).strip()
    dbus_bus = str(leader_env.get("DBUS_SESSION_BUS_ADDRESS") or "").strip()
    if not dbus_bus and runtime_dir:
        dbus_bus = f"unix:path={runtime_dir}/bus"
    env = {
        "DBUS_SESSION_BUS_ADDRESS": dbus_bus,
        "XDG_RUNTIME_DIR": runtime_dir,
        "YGGTERM_ALLOW_MULTI_WINDOW": "1",
        "YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF": "1",
        "NO_AT_BRIDGE": "1",
        "WEBKIT_DISABLE_COMPOSITING_MODE": "1",
    }
    if remote_home:
        env["YGGTERM_HOME"] = remote_home
    if backend == "x11":
        env["DISPLAY"] = str(
            session_info.get("xwayland_display") or leader_env.get("DISPLAY") or ""
        )
        env["XAUTHORITY"] = str(
            session_info.get("xwayland_xauthority") or leader_env.get("XAUTHORITY") or ""
        )
        env["GDK_BACKEND"] = "x11"
    elif session_type == "wayland":
        wayland_display = str(leader_env.get("WAYLAND_DISPLAY") or "").strip()
        if not wayland_display:
            for candidate in session_info.get("wayland_sockets") or []:
                rendered = str(candidate or "").strip()
                if rendered and not rendered.endswith(".lock"):
                    wayland_display = rendered
                    break
        env["WAYLAND_DISPLAY"] = wayland_display
        env["GDK_BACKEND"] = "wayland"
        display_value = str(leader_env.get("DISPLAY") or session_info.get("xwayland_display") or "").strip()
        xauthority_value = str(
            leader_env.get("XAUTHORITY") or session_info.get("xwayland_xauthority") or ""
        ).strip()
        if display_value:
            env["DISPLAY"] = display_value
        if xauthority_value:
            env["XAUTHORITY"] = xauthority_value
    else:
        env["DISPLAY"] = str(leader_env.get("DISPLAY") or "")
        env["XAUTHORITY"] = str(leader_env.get("XAUTHORITY") or "")
    return env


def main() -> int:
    args = parse_args()
    timestamp = int(time.time())
    out_dir = Path(args.out_dir or f"/tmp/yggterm-live-watch-{args.host}-{timestamp}")
    out_dir.mkdir(parents=True, exist_ok=True)
    remote_dir = args.remote_dir or f"/tmp/yggterm-live-watch-{timestamp}"
    ssh_shell(args.host, f"rm -rf '{remote_dir}' && mkdir -p '{remote_dir}'")

    metadata: dict[str, object] = {"host": args.host, "backend": args.backend, "remote_dir": remote_dir}
    try:
        session_info = ssh_python_json(args.host, LINUX_SESSION_SNIPPET)
        metadata["session_info"] = session_info
        remote_bin = resolve_launch_target(args.host, args, remote_dir)
        metadata["remote_bin"] = remote_bin

        remote_home = None if args.reuse_existing_home else f"{remote_dir}/home"
        remote_shot = f"{remote_dir}/watch-final.png"
        remote_frame_shot = f"{remote_dir}/watch-frame.png"
        if remote_home:
            ssh_shell(args.host, f"mkdir -p '{remote_home}'")
        env = launch_env_from_session(session_info, args.backend, remote_home)
        metadata["launch_env"] = env
        exports = remote_env_exports(env)

        launch = ssh_shell(
            args.host,
            f"{exports}; nohup '{remote_bin}' > '{remote_dir}/client.log' 2>&1 < /dev/null & echo $!",
        )
        pid = int(launch.stdout.strip())
        metadata["pid"] = pid

        samples: list[dict] = []
        started = time.time()
        while time.time() - started < args.duration_sec:
            sample: dict[str, object] = {"elapsed_sec": round(time.time() - started, 2)}
            state_proc = ssh_shell(
                args.host,
                f"{exports}; '{remote_bin}' server app state --pid {pid} --timeout-ms {args.timeout_ms}",
                check=False,
            )
            sample["state_returncode"] = state_proc.returncode
            sample["state_stdout"] = state_proc.stdout.strip()
            sample["state_stderr"] = state_proc.stderr.strip()
            if state_proc.returncode == 0 and state_proc.stdout.strip():
                try:
                    payload = json.loads(state_proc.stdout.strip())
                    sample["state"] = payload.get("data") or {}
                except json.JSONDecodeError as exc:
                    sample["state_json_error"] = str(exc)
            sample["processes"] = ssh_python_json(args.host, PROCESS_SAMPLE_SNIPPET)
            process_rows = (sample["processes"] or {}).get("rows") or []
            sample["owned_pid_present"] = any(int(row.get("pid") or 0) == pid for row in process_rows)
            samples.append(sample)
            time.sleep(args.poll_sec)

        metadata["samples"] = samples
        yggterm_fd_counts: list[int] = []
        ssh_generation_context_counts: list[int] = []
        missing_owned_pid_samples = 0
        for sample in samples:
            processes = sample.get("processes") or {}
            ssh_generation_context_counts.append(
                int(processes.get("ssh_generation_context_count") or 0)
            )
            process_rows = processes.get("rows") or []
            owned_row = next(
                (row for row in process_rows if int(row.get("pid") or 0) == pid),
                None,
            )
            if owned_row is None:
                missing_owned_pid_samples += 1
                continue
            fd_count = owned_row.get("fd_count")
            if isinstance(fd_count, int):
                yggterm_fd_counts.append(fd_count)
        metadata["watch_summary"] = {
            "owned_pid": pid,
            "owned_pid_missing_samples": missing_owned_pid_samples,
            "initial_fd_count": yggterm_fd_counts[0] if yggterm_fd_counts else None,
            "max_fd_count": max(yggterm_fd_counts) if yggterm_fd_counts else None,
            "final_fd_count": yggterm_fd_counts[-1] if yggterm_fd_counts else None,
            "fd_growth": (
                yggterm_fd_counts[-1] - yggterm_fd_counts[0]
                if len(yggterm_fd_counts) >= 2
                else None
            ),
            "max_ssh_generation_context_count": (
                max(ssh_generation_context_counts) if ssh_generation_context_counts else 0
            ),
        }
        journal_proc = ssh_shell(
            args.host,
            (
                "journalctl --user -b "
                f"--since '@{int(started)}' --no-pager | "
                "rg -i 'yggterm|plasmashell|kwin|too many open files|libpng|segfault|abort|crash' -n || true"
            ),
            check=False,
        )
        metadata["journal_matches"] = journal_proc.stdout.strip().splitlines()
        if samples:
            last_state = samples[-1].get("state")
            if isinstance(last_state, dict):
                ssh_shell(
                    args.host,
                    f"{exports}; '{remote_bin}' server app screenshot --pid {pid} '{remote_shot}' --timeout-ms {args.timeout_ms}",
                    check=False,
                )
                shot_exists = ssh_shell(args.host, f"test -f '{remote_shot}'", check=False)
                if shot_exists.returncode == 0:
                    scp_from(args.host, remote_shot, out_dir / "watch-final.png")
                if args.capture_frame:
                    frame_proc = ssh_shell(
                        args.host,
                        (
                            f"{exports}; export QT_QPA_PLATFORM=wayland; "
                            f"if command -v spectacle >/dev/null 2>&1; then "
                            f"spectacle --background --nonotify --activewindow --output '{remote_frame_shot}' "
                            "--delay 450 >/dev/null 2>&1; "
                            "fi"
                        ),
                        check=False,
                        timeout_seconds=20.0,
                    )
                    metadata["frame_capture_returncode"] = frame_proc.returncode
                    frame_exists = ssh_shell(args.host, f"test -f '{remote_frame_shot}'", check=False)
                    if frame_exists.returncode == 0:
                        scp_from(args.host, remote_frame_shot, out_dir / "watch-frame.png")

        summary_path = out_dir / "summary.json"
        summary_path.write_text(json.dumps(metadata, indent=2), encoding="utf-8")
        ssh_shell(
            args.host,
            f"{exports}; '{remote_bin}' server app close --pid {pid} --timeout-ms {args.timeout_ms} >/dev/null 2>&1 || true",
            check=False,
        )
        if not args.reuse_existing_home:
            ssh_shell(
                args.host,
                f"{exports}; '{remote_bin}' server shutdown >/dev/null 2>&1 || true",
                check=False,
            )
        if not args.keep_remote_dir:
            ssh_shell(args.host, f"rm -rf '{remote_dir}'", check=False)
        print(summary_path)
        app_control_failures = 0
        ready_seen = False
        for sample in samples:
            if int(sample.get("state_returncode") or 0) == 0:
                ready_seen = True
                continue
            if ready_seen:
                app_control_failures += 1
        metadata["watch_summary"]["app_control_failures_after_ready"] = app_control_failures
        summary_path.write_text(json.dumps(metadata, indent=2), encoding="utf-8")
        watch_summary = metadata.get("watch_summary") or {}
        fd_growth = watch_summary.get("fd_growth")
        max_fd_count = watch_summary.get("max_fd_count")
        runaway_journal = any(
            "too many open files" in line.lower() or "main process exited" in line.lower()
            for line in metadata.get("journal_matches") or []
        )
        runaway_fd = (
            isinstance(fd_growth, int)
            and isinstance(max_fd_count, int)
            and fd_growth >= 96
            and max_fd_count >= 192
        )
        owned_pid_missing = int(watch_summary.get("owned_pid_missing_samples") or 0) > 0
        return 0 if not (app_control_failures or runaway_journal or runaway_fd or owned_pid_missing) else 1
    finally:
        if args.keep_remote_dir:
            (out_dir / "remote-dir.txt").write_text(f"{remote_dir}\n", encoding="utf-8")


if __name__ == "__main__":
    raise SystemExit(main())
