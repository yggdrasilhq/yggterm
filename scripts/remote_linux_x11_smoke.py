#!/usr/bin/env python3
import argparse
import json
import shlex
import shutil
import subprocess
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ARTIFACT = ROOT / "dist" / "yggterm-linux-x86_64"
SMOKE_SCRIPT = ROOT / "scripts" / "smoke_xterm_embed_faults.py"
SSH_BASE = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8"]
SCP_BASE = ["scp", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8"]

LINUX_SESSION_SNIPPET = r"""
import json
import os
import pathlib
import pwd
import shutil
import subprocess


def proc_cmdline(pid: str) -> list[str]:
    try:
        return [
            part.decode("utf-8", "ignore")
            for part in (pathlib.Path("/proc") / pid / "cmdline").read_bytes().split(b"\0")
            if part
        ]
    except Exception:
        return []


def proc_env(pid: str) -> dict[str, str]:
    data: dict[str, str] = {}
    try:
        for raw in (pathlib.Path("/proc") / pid / "environ").read_bytes().split(b"\0"):
            if not raw or b"=" not in raw:
                continue
            key, value = raw.split(b"=", 1)
            data[key.decode("utf-8", "ignore")] = value.decode("utf-8", "ignore")
    except Exception:
        pass
    return data


def session_props(session_id: str) -> dict[str, str]:
    proc = subprocess.run(
        [
            "loginctl",
            "show-session",
            session_id,
            "-p",
            "Name",
            "-p",
            "Type",
            "-p",
            "Class",
            "-p",
            "State",
            "-p",
            "Remote",
            "-p",
            "Leader",
            "-p",
            "Active",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    props: dict[str, str] = {}
    for line in proc.stdout.splitlines():
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        props[key] = value
    return props


uid = os.getuid()
user = pwd.getpwuid(uid).pw_name
runtime_dir = pathlib.Path(f"/run/user/{uid}")
sessions: list[dict[str, str]] = []
listing = subprocess.run(
    ["loginctl", "list-sessions", "--no-legend"],
    capture_output=True,
    text=True,
    check=False,
)
for line in listing.stdout.splitlines():
    parts = line.split()
    if not parts:
        continue
    session_id = parts[0]
    props = session_props(session_id)
    if props.get("Name") != user or props.get("Class") != "user" or props.get("Remote") != "no":
        continue
    props["session_id"] = session_id
    sessions.append(props)

def _priority(item: dict[str, str]) -> tuple[int, int, int]:
    active_rank = 0 if item.get("Active") == "yes" else 1
    type_value = item.get("Type") or ""
    type_rank = 0 if type_value == "x11" else 1 if type_value == "wayland" else 2
    leader_rank = -int(item.get("Leader") or 0)
    return (active_rank, type_rank, leader_rank)


sessions.sort(key=_priority)
picked = sessions[0] if sessions else None
leader_env = proc_env(str(picked.get("Leader") or "")) if picked else {}
xwayland_display = ""
xwayland_xauthority = ""
for pid in os.listdir("/proc"):
    if not pid.isdigit():
        continue
    cmdline = proc_cmdline(pid)
    if not cmdline:
        continue
    argv0 = pathlib.Path(cmdline[0]).name
    if argv0 == "kwin_wayland":
        for index, value in enumerate(cmdline):
            if value == "--xwayland-display" and index + 1 < len(cmdline):
                xwayland_display = cmdline[index + 1]
            if value == "--xwayland-xauthority" and index + 1 < len(cmdline):
                xwayland_xauthority = cmdline[index + 1]
    elif argv0 == "Xwayland":
        if not xwayland_display:
            for value in cmdline[1:]:
                if value.startswith(":"):
                    xwayland_display = value
                    break
        if not xwayland_xauthority:
            for index, value in enumerate(cmdline):
                if value == "-auth" and index + 1 < len(cmdline):
                    xwayland_xauthority = cmdline[index + 1]
                    break

print(
    json.dumps(
        {
            "uid": uid,
            "user": user,
            "runtime_dir": str(runtime_dir),
            "wayland_sockets": [
                path.name
                for path in sorted(runtime_dir.glob("wayland-*"))
                if path.exists()
            ],
            "sessions": sessions,
            "picked_session": picked,
            "leader_env": leader_env,
            "python3": shutil.which("python3"),
            "xdotool": shutil.which("xdotool"),
            "imagemagick_import": shutil.which("import"),
            "xwayland_display": xwayland_display,
            "xwayland_xauthority": xwayland_xauthority,
        }
    )
)
"""

OWNED_CLIENTS_SNIPPET = r"""
import json
import os
import pathlib


def proc_cmdline(pid: str) -> list[str]:
    try:
        return [
            part.decode("utf-8", "ignore")
            for part in (pathlib.Path("/proc") / pid / "cmdline").read_bytes().split(b"\0")
            if part
        ]
    except Exception:
        return []


def proc_env(pid: str) -> dict[str, str]:
    data: dict[str, str] = {}
    try:
        for raw in (pathlib.Path("/proc") / pid / "environ").read_bytes().split(b"\0"):
            if not raw or b"=" not in raw:
                continue
            key, value = raw.split(b"=", 1)
            data[key.decode("utf-8", "ignore")] = value.decode("utf-8", "ignore")
    except Exception:
        pass
    return data


clients = []
for pid in sorted(os.listdir("/proc")):
    if not pid.isdigit():
        continue
    cmdline = proc_cmdline(pid)
    if not cmdline:
        continue
    env = proc_env(pid)
    home = str(env.get("YGGTERM_HOME") or "")
    if not home.startswith("/tmp/yggterm-remote-smoke-"):
        continue
    exe = str(cmdline[0] or "")
    if "yggterm" not in pathlib.Path(exe).name.lower() and not any("yggterm" in part.lower() for part in cmdline):
        continue
    clients.append(
        {
            "pid": int(pid),
            "home": home,
            "exe": exe,
            "cmdline": cmdline,
            "display": env.get("DISPLAY"),
            "wayland_display": env.get("WAYLAND_DISPLAY"),
        }
    )

print(json.dumps({"count": len(clients), "clients": clients}))
"""

PROBLEM_NOTIFICATION_MARKERS = (
    "connection refused",
    "no such file or directory",
    "server unavailable",
    "daemon did not become reachable",
    "local yggterm daemon",
    "reading daemon response",
    "parsing daemon response",
    "timed out waiting for app control response",
    "failed sock",
    "sock connection",
    ".sock",
)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Stage a Linux Yggterm binary on a remote host and run the smoke suite there."
    )
    parser.add_argument("--host", required=True)
    parser.add_argument("--artifact", default=str(DEFAULT_ARTIFACT))
    parser.add_argument("--remote-bin")
    parser.add_argument("--session", default="local://remote-smoke")
    parser.add_argument("--session-kind", choices=("plain", "codex"), default="plain")
    parser.add_argument("--backend", choices=("x11", "wayland", "auto"), default="x11")
    parser.add_argument("--out-dir")
    parser.add_argument("--remote-dir")
    parser.add_argument("--timeout-ms", type=int, default=20000)
    parser.add_argument("--smoke-timeout-sec", type=int, default=420)
    parser.add_argument("--keep-remote-dir", action="store_true")
    return parser.parse_args()


def quote(value: str) -> str:
    return shlex.quote(value)


def local_run(
    argv: list[str],
    *,
    input_text: str | None = None,
    check: bool = True,
    timeout_seconds: float | None = None,
) -> subprocess.CompletedProcess:
    proc = subprocess.run(
        argv,
        text=True,
        capture_output=True,
        input=input_text,
        timeout=timeout_seconds,
    )
    if check and proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip() or proc.stdout.strip() or f"command failed: {argv!r}")
    return proc


def ssh_shell(
    host: str,
    command: str,
    *,
    check: bool = True,
    timeout_seconds: float | None = None,
) -> subprocess.CompletedProcess:
    return local_run(
        [*SSH_BASE, host, f"bash -lc {quote(command)}"],
        check=check,
        timeout_seconds=timeout_seconds,
    )


def ssh_python_json(host: str, snippet: str) -> dict:
    proc = local_run([*SSH_BASE, host, "python3", "-"], input_text=snippet, check=True)
    text = proc.stdout.strip()
    if not text:
        raise RuntimeError(f"remote python returned empty output on {host}")
    return json.loads(text)


def scp_to(host: str, local_path: Path, remote_path: str) -> None:
    local_run([*SCP_BASE, str(local_path), f"{host}:{remote_path}"], check=True)


def scp_from(host: str, remote_path: str, local_path: Path) -> None:
    local_path.parent.mkdir(parents=True, exist_ok=True)
    local_run([*SCP_BASE, "-r", f"{host}:{remote_path}", str(local_path)], check=True)


def remote_env_exports(env: dict[str, str]) -> str:
    chunks = []
    for key, value in env.items():
        if value:
            chunks.append(f"export {key}={quote(value)}")
    return "; ".join(chunks)


def autodetect_remote_bin(host: str) -> str:
    command = r"""
set -e
if [ -x "$HOME/.local/bin/yggterm" ]; then
  printf '%s\n' "$HOME/.local/bin/yggterm"
  exit 0
fi
latest_direct="$(find "$HOME/.local/share/yggterm/direct/versions" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' 2>/dev/null | sort -V | tail -n1)"
if [ -n "$latest_direct" ] && [ -x "$HOME/.local/share/yggterm/direct/versions/$latest_direct/yggterm" ]; then
  printf '%s\n' "$HOME/.local/share/yggterm/direct/versions/$latest_direct/yggterm"
  exit 0
fi
if [ -x "$HOME/.yggterm/bin/yggterm" ]; then
  printf '%s\n' "$HOME/.yggterm/bin/yggterm"
  exit 0
fi
exit 1
"""
    proc = ssh_shell(host, command, check=False)
    path = proc.stdout.strip()
    if proc.returncode != 0 or not path:
        raise RuntimeError(f"could not resolve a remote yggterm binary on {host}")
    return path


def resolve_launch_target(host: str, args: argparse.Namespace, remote_dir: str) -> str:
    if args.remote_bin:
        return args.remote_bin
    artifact_value = str(getattr(args, "artifact", "") or "").strip()
    if artifact_value:
        artifact = Path(artifact_value).expanduser()
    else:
        artifact = None
    if artifact is not None and artifact.exists():
        if artifact.suffixes[-2:] == [".tar", ".gz"]:
            remote_archive = f"{remote_dir}/{artifact.name}"
            scp_to(host, artifact, remote_archive)
            remote_bin = f"{remote_dir}/yggterm-linux-x86_64"
            ssh_shell(
                host,
                f"tar -xzf {quote(remote_archive)} -C {quote(remote_dir)} && chmod +x {quote(remote_bin)}",
            )
            return remote_bin
        remote_bin = f"{remote_dir}/yggterm"
        scp_to(host, artifact, remote_bin)
        ssh_shell(host, f"chmod +x {quote(remote_bin)}")
        return remote_bin
    return autodetect_remote_bin(host)


def smoke_python(host: str, remote_dir: str) -> str:
    check = ssh_shell(
        host,
        r"""python3 - <<'PY'
import importlib.util
print("ok" if importlib.util.find_spec("PIL") else "missing")
PY""",
    )
    if check.stdout.strip() == "ok":
        return "python3"
    venv_dir = f"{remote_dir}/venv"
    ssh_shell(
        host,
        f"python3 -m venv {quote(venv_dir)} && {quote(venv_dir)}/bin/python -m pip install --quiet --upgrade pip Pillow",
    )
    return f"{venv_dir}/bin/python"


def remote_owned_clients(host: str) -> dict:
    return ssh_python_json(host, OWNED_CLIENTS_SNIPPET)


def remote_notify(host: str, env: dict[str, str], title: str, body: str) -> None:
    exports = remote_env_exports(env)
    ssh_shell(
        host,
        f"{exports}; if command -v notify-send >/dev/null 2>&1; then "
        f"notify-send {quote(title)} {quote(body)}; fi",
        check=False,
    )


def remote_background_window(
    host: str,
    remote_bin: str,
    env: dict[str, str],
    pid: int,
    timeout_ms: int,
) -> dict:
    exports = remote_env_exports(env)
    proc = ssh_shell(
        host,
        f"{exports}; {quote(remote_bin)} server app background --pid {pid} --timeout-ms {timeout_ms}",
        check=False,
    )
    text = proc.stdout.strip()
    if proc.returncode != 0 or not text:
        raise RuntimeError(
            proc.stderr.strip()
            or proc.stdout.strip()
            or f"failed to background pid {pid} on {host}"
        )
    return json.loads(text)


def remote_close_window(
    host: str,
    remote_bin: str,
    env: dict[str, str],
    pid: int,
    timeout_ms: int,
) -> dict:
    exports = remote_env_exports(env)
    proc = ssh_shell(
        host,
        f"{exports}; {quote(remote_bin)} server app close --pid {pid} --timeout-ms {timeout_ms}",
        check=False,
    )
    text = proc.stdout.strip()
    if proc.returncode != 0 or not text:
        raise RuntimeError(
            proc.stderr.strip()
            or proc.stdout.strip()
            or f"failed to close pid {pid} on {host}"
        )
    return json.loads(text)


def remote_problem_notifications(state: dict) -> list[dict]:
    shell = state.get("shell") or {}
    notifications = []
    history = shell.get("notifications")
    if isinstance(history, list):
        notifications.extend(history)
    visible = shell.get("visible_notifications")
    if isinstance(visible, list):
        notifications.extend(visible)
    bad = []
    seen = set()
    for notification in notifications:
        if not isinstance(notification, dict):
            continue
        identifier = notification.get("id")
        if identifier in seen:
            continue
        seen.add(identifier)
        haystack = " ".join(
            str(notification.get(key) or "")
            for key in ("title", "message", "tone")
        ).lower()
        if any(marker in haystack for marker in PROBLEM_NOTIFICATION_MARKERS):
            bad.append(notification)
    return bad


def remote_cleanup_owned_clients(host: str, timeout_ms: int) -> dict:
    inventory = remote_owned_clients(host)
    results: list[dict] = []
    for client in list(inventory.get("clients") or []):
        pid = int(client.get("pid") or 0)
        home = str(client.get("home") or "").strip()
        exe = str(client.get("exe") or "").strip()
        result: dict[str, object] = {
            "pid": pid,
            "home": home,
            "exe": exe,
        }
        if pid <= 0:
            results.append(result)
            continue
        if home and exe:
            close_proc = ssh_shell(
                host,
                f"export YGGTERM_HOME={quote(home)}; {quote(exe)} server app close --pid {pid} "
                f"--timeout-ms {timeout_ms}",
                check=False,
            )
            result["close_returncode"] = close_proc.returncode
            if close_proc.stdout.strip():
                result["close_stdout"] = close_proc.stdout.strip()
            if close_proc.stderr.strip():
                result["close_stderr"] = close_proc.stderr.strip()
            shutdown_proc = ssh_shell(
                host,
                f"export YGGTERM_HOME={quote(home)}; {quote(exe)} server shutdown >/dev/null 2>&1 || true",
                check=False,
            )
            result["shutdown_returncode"] = shutdown_proc.returncode
        alive = ssh_shell(host, f"kill -0 {pid} >/dev/null 2>&1", check=False).returncode == 0
        result["alive_after_close"] = alive
        if alive:
            ssh_shell(host, f"kill -TERM {pid} >/dev/null 2>&1 || true", check=False)
            time.sleep(0.6)
            alive = ssh_shell(host, f"kill -0 {pid} >/dev/null 2>&1", check=False).returncode == 0
            result["alive_after_term"] = alive
        if alive:
            ssh_shell(host, f"kill -KILL {pid} >/dev/null 2>&1 || true", check=False)
            time.sleep(0.2)
            alive = ssh_shell(host, f"kill -0 {pid} >/dev/null 2>&1", check=False).returncode == 0
            result["alive_after_kill"] = alive
        results.append(result)
    return {
        "before": inventory,
        "results": results,
        "after": remote_owned_clients(host),
    }


def wait_for_remote_state(
    host: str,
    remote_bin: str,
    env: dict[str, str],
    pid: int,
    timeout_ms: int,
    *,
    timeout_seconds: float = 30.0,
    require_visible: bool = True,
) -> dict:
    deadline = time.time() + timeout_seconds
    exports = remote_env_exports(env)
    while time.time() < deadline:
        proc = ssh_shell(
            host,
            f"{exports}; {quote(remote_bin)} server app state --pid {pid} --timeout-ms {timeout_ms}",
            check=False,
        )
        text = proc.stdout.strip()
        if proc.returncode == 0 and text:
            payload = json.loads(text)
            data = payload.get("data") or {}
            dom = data.get("dom") or {}
            window = data.get("window") or {}
            visible_ok = window.get("visible") is True or not require_visible
            if dom.get("shell_root_count") == 1 and visible_ok:
                return data
        time.sleep(0.5)
    state_goal = "visible" if require_visible else "available"
    raise RuntimeError(f"remote app state never became {state_goal} on {host} for pid {pid}")


def remote_create_plain_terminal(
    host: str,
    remote_bin: str,
    env: dict[str, str],
    pid: int,
    *,
    title: str = "Remote Smoke Plain",
) -> str:
    exports = remote_env_exports(env)
    proc = ssh_shell(
        host,
        f"{exports}; {quote(remote_bin)} server app terminal new --pid {pid} --title {quote(title)} --timeout-ms 15000",
        check=False,
    )
    text = proc.stdout.strip()
    if proc.returncode != 0 or not text:
        raise RuntimeError(
            proc.stderr.strip()
            or proc.stdout.strip()
            or f"failed to create a plain terminal on {host}"
        )
    payload = json.loads(text)
    data = payload.get("data") or payload
    session_path = str(data.get("active_session_path") or "").strip()
    if not session_path:
        raise RuntimeError(f"plain terminal creation did not return an active_session_path on {host}")
    return session_path


def main() -> int:
    args = parse_args()
    timestamp = int(time.time())
    out_dir = Path(args.out_dir or f"/tmp/yggterm-remote-smoke-{args.host}-{timestamp}")
    out_dir.mkdir(parents=True, exist_ok=True)
    remote_dir = args.remote_dir or f"/tmp/yggterm-remote-smoke-{timestamp}"
    ssh_shell(args.host, f"rm -rf {quote(remote_dir)} && mkdir -p {quote(remote_dir)}")

    metadata: dict[str, object] = {
        "host": args.host,
        "remote_dir": remote_dir,
        "session": args.session,
        "session_kind": args.session_kind,
        "backend": args.backend,
    }
    metadata_path = out_dir / "summary.json"
    smoke_proc: subprocess.CompletedProcess | None = None
    try:
        cleanup_summary = remote_cleanup_owned_clients(args.host, args.timeout_ms)
        metadata["owned_clients_cleanup"] = cleanup_summary
        session_info = ssh_python_json(args.host, LINUX_SESSION_SNIPPET)
        metadata["session_info"] = session_info
        if not session_info.get("python3"):
            raise RuntimeError(f"python3 is missing on {args.host}")

        picked = session_info.get("picked_session") or {}
        leader_env = session_info.get("leader_env") or {}
        picked_type = str(picked.get("Type") or "").strip().lower()
        runtime_dir = str(
            leader_env.get("XDG_RUNTIME_DIR")
            or session_info.get("runtime_dir")
            or ""
        ).strip()
        if not runtime_dir:
            raise RuntimeError(f"could not resolve XDG_RUNTIME_DIR on {args.host}: {session_info!r}")
        dbus_bus = str(leader_env.get("DBUS_SESSION_BUS_ADDRESS") or "").strip()
        if not dbus_bus:
            dbus_bus = f"unix:path={runtime_dir}/bus"

        remote_bin = resolve_launch_target(args.host, args, remote_dir)
        metadata["remote_bin"] = remote_bin

        remote_smoke_script = f"{remote_dir}/smoke_xterm_embed_faults.py"
        scp_to(args.host, SMOKE_SCRIPT, remote_smoke_script)
        python_cmd = smoke_python(args.host, remote_dir)
        metadata["remote_python"] = python_cmd

        remote_home = f"{remote_dir}/home"
        remote_out = f"{remote_dir}/proof"
        remote_log = f"{remote_dir}/client.log"
        launch_env = {
            "DBUS_SESSION_BUS_ADDRESS": dbus_bus,
            "XDG_RUNTIME_DIR": runtime_dir,
            "YGGTERM_HOME": remote_home,
            "YGGTERM_ALLOW_MULTI_WINDOW": "1",
            "YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF": "1",
            "YGGTERM_POINTER_DRIVER": "app",
            "YGGTERM_KEY_DRIVER": "app",
            "YGGTERM_APP_CONTROL_SKIP_X11_SYNTHETIC_INPUT": "1",
            "NO_AT_BRIDGE": "1",
        }
        effective_backend = args.backend
        if effective_backend == "auto":
            effective_backend = "wayland" if picked_type == "wayland" else "x11"
        if effective_backend == "wayland":
            wayland_display = str(leader_env.get("WAYLAND_DISPLAY") or "").strip()
            if not wayland_display:
                candidates = session_info.get("wayland_sockets") or []
                if isinstance(candidates, list):
                    for candidate in candidates:
                        rendered = str(candidate or "").strip()
                        if rendered:
                            wayland_display = rendered
                            break
            if not wayland_display:
                raise RuntimeError(f"could not resolve WAYLAND_DISPLAY on {args.host}: {session_info!r}")
            launch_env["WAYLAND_DISPLAY"] = wayland_display
            launch_env["GDK_BACKEND"] = "wayland"
            display_value = str(leader_env.get("DISPLAY") or "").strip()
            xauthority_value = str(leader_env.get("XAUTHORITY") or "").strip()
            if display_value:
                launch_env["DISPLAY"] = display_value
            if xauthority_value:
                launch_env["XAUTHORITY"] = xauthority_value
        else:
            x11_display = str(
                session_info.get("xwayland_display")
                or leader_env.get("DISPLAY")
                or ""
            ).strip()
            x11_xauthority = str(
                session_info.get("xwayland_xauthority")
                or leader_env.get("XAUTHORITY")
                or ""
            ).strip()
            if not x11_display:
                raise RuntimeError(f"could not resolve an X11 display on {args.host}: {session_info!r}")
            if not x11_xauthority:
                raise RuntimeError(f"could not resolve XAUTHORITY on {args.host}: {session_info!r}")
            launch_env["DISPLAY"] = x11_display
            launch_env["XAUTHORITY"] = x11_xauthority
            launch_env["GDK_BACKEND"] = "x11"
            launch_env["WEBKIT_DISABLE_COMPOSITING_MODE"] = "1"
        metadata["effective_backend"] = effective_backend
        metadata["launch_env"] = launch_env

        ssh_shell(args.host, f"mkdir -p {quote(remote_home)} {quote(remote_out)}")
        exports = remote_env_exports(launch_env)
        remote_notify(
            args.host,
            launch_env,
            "Yggterm automated testing",
            "Automated testing is starting. The Yggterm window will move to the background when possible.",
        )
        launch = ssh_shell(
            args.host,
            f"{exports}; {quote(remote_bin)} server app launch --wait-visible "
            f"--allow-multi-window --skip-active-exec-handoff --timeout-ms {args.timeout_ms} "
            f"--log {quote(remote_log)}",
        )
        launch_payload = json.loads(launch.stdout.strip() or "{}")
        pid = int(launch_payload.get("pid") or 0)
        if pid <= 0:
            raise RuntimeError(f"background app launch did not return a pid on {args.host}: {launch.stdout!r}")
        metadata["launch_response"] = launch_payload
        metadata["pid"] = pid

        initial_state = wait_for_remote_state(args.host, remote_bin, launch_env, pid, args.timeout_ms)
        initial_bad_notifications = remote_problem_notifications(initial_state)
        if initial_bad_notifications:
            raise RuntimeError(
                f"bad daemon/socket notifications were already visible right after launch: {initial_bad_notifications!r}"
            )
        (out_dir / "initial-state.json").write_text(json.dumps(initial_state, indent=2), encoding="utf-8")
        smoke_session = args.session
        if args.session_kind == "plain" and smoke_session == "local://remote-smoke":
            smoke_session = remote_create_plain_terminal(
                args.host,
                remote_bin,
                launch_env,
                pid,
            )
        metadata["requested_session"] = args.session
        metadata["resolved_session"] = smoke_session
        metadata["owned_clients_after_launch"] = remote_owned_clients(args.host)
        metadata["background_response"] = remote_background_window(
            args.host,
            remote_bin,
            launch_env,
            pid,
            args.timeout_ms,
        )

        try:
            smoke_proc = ssh_shell(
                args.host,
                f"{exports}; YGGTERM_BIN={quote(remote_bin)} YGGTERM_SMOKE_AVOID_FOREGROUND=1 "
                f"{quote(python_cmd)} {quote(remote_smoke_script)} "
                f"--bin {quote(remote_bin)} --pid {pid} --session {quote(smoke_session)} "
                f"--session-kind {quote(args.session_kind)} --out {quote(remote_out)} --home {quote(remote_home)}",
                check=False,
                timeout_seconds=max(30, args.smoke_timeout_sec),
            )
            metadata["smoke_returncode"] = smoke_proc.returncode
            metadata["smoke_stdout"] = smoke_proc.stdout
            metadata["smoke_stderr"] = smoke_proc.stderr
        except subprocess.TimeoutExpired as exc:
            metadata["smoke_timeout_sec"] = args.smoke_timeout_sec
            metadata["smoke_stdout"] = (exc.stdout or "").strip()
            metadata["smoke_stderr"] = (exc.stderr or "").strip()
            raise RuntimeError(f"remote smoke timed out after {args.smoke_timeout_sec}s on {args.host}")
        final_state = wait_for_remote_state(
            args.host,
            remote_bin,
            launch_env,
            pid,
            args.timeout_ms,
            timeout_seconds=10.0,
            require_visible=False,
        )
        metadata["final_problem_notifications"] = remote_problem_notifications(final_state)
        if metadata["final_problem_notifications"]:
            raise RuntimeError(
                f"bad daemon/socket notifications were observed during smoke: {metadata['final_problem_notifications']!r}"
            )

        pulled_dir = out_dir / "proof"
        pulled_dir.mkdir(parents=True, exist_ok=True)
        scp_from(args.host, f"{remote_out}/.", pulled_dir)
        perf_remote = f"{remote_home}/perf-telemetry.jsonl"
        perf_proc = ssh_shell(args.host, f"test -f {quote(perf_remote)}", check=False)
        if perf_proc.returncode == 0:
            scp_from(args.host, perf_remote, out_dir / "perf-telemetry.jsonl")

        metadata["local_proof_dir"] = str(pulled_dir)
        metadata_path.write_text(json.dumps(metadata, indent=2), encoding="utf-8")

        try:
            metadata["close_response"] = remote_close_window(
                args.host,
                remote_bin,
                launch_env,
                pid,
                args.timeout_ms,
            )
        except Exception as exc:
            metadata["close_error"] = str(exc)
        ssh_shell(
            args.host,
            f"{exports}; {quote(remote_bin)} server shutdown >/dev/null 2>&1 || true",
            check=False,
        )
        metadata["owned_clients_after_close"] = remote_owned_clients(args.host)
        metadata["ok"] = (smoke_proc is not None and smoke_proc.returncode == 0) and not bool(
            metadata.get("final_problem_notifications")
        )
        metadata_path.write_text(json.dumps(metadata, indent=2), encoding="utf-8")
        remote_notify(
            args.host,
            launch_env,
            "Yggterm automated testing",
            "Automated testing finished. The test window should be in the background or closed.",
        )
        if not args.keep_remote_dir:
            ssh_shell(args.host, f"rm -rf {quote(remote_dir)}", check=False)
        print(metadata_path)
        return 0 if metadata.get("ok") else (smoke_proc.returncode if smoke_proc is not None else 1)
    except Exception as exc:
        metadata["ok"] = False
        metadata["error"] = str(exc)
        try:
            metadata_path.write_text(json.dumps(metadata, indent=2), encoding="utf-8")
        except Exception:
            pass
        print(metadata_path)
        return 1
    finally:
        if args.keep_remote_dir:
            (out_dir / "remote-dir.txt").write_text(f"{remote_dir}\n", encoding="utf-8")


if __name__ == "__main__":
    raise SystemExit(main())
