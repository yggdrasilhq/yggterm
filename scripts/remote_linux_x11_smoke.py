#!/usr/bin/env python3
import argparse
import json
import os
import shlex
import shutil
import subprocess
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ARTIFACT = ROOT / "dist" / "yggterm-linux-x86_64"
SMOKE_SCRIPT = ROOT / "scripts" / "smoke_xterm_embed_faults.py"
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
plasmashell_pid = ""
plasmashell_env: dict[str, str] = {}
numeric_pids = sorted((pid for pid in os.listdir("/proc") if pid.isdigit()), key=int, reverse=True)
for pid in numeric_pids:
    cmdline = proc_cmdline(pid)
    if not cmdline:
        continue
    argv0 = pathlib.Path(cmdline[0]).name
    if argv0 == "plasmashell":
        plasmashell_pid = pid
        plasmashell_env = proc_env(pid)
        break
desktop_env = dict(leader_env)
desktop_env.update(plasmashell_env)
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
            "home_dir": pwd.getpwuid(uid).pw_dir,
            "runtime_dir": str(runtime_dir),
            "wayland_sockets": [
                path.name
                for path in sorted(runtime_dir.glob("wayland-*"))
                if path.exists()
            ],
            "sessions": sessions,
            "picked_session": picked,
            "leader_env": leader_env,
            "plasmashell_pid": plasmashell_pid,
            "plasmashell_env": plasmashell_env,
            "desktop_env": desktop_env,
            "python3": shutil.which("python3"),
            "xdotool": shutil.which("xdotool"),
            "imagemagick_import": shutil.which("import"),
            "xwayland_display": xwayland_display,
            "xwayland_xauthority": xwayland_xauthority,
        }
    )
)
"""


def write_json(path: Path, payload) -> None:
    path.write_text(json.dumps(payload, indent=2, default=str), encoding="utf-8")

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
    smoke_tag = str(env.get("YGGTERM_REMOTE_SMOKE_TAG") or "")
    if smoke_tag != "1" and not home.startswith("/tmp/yggterm-remote-smoke-"):
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
    parser.add_argument("--proxy-jump")
    parser.add_argument("--ssh-port", type=int)
    parser.add_argument("--artifact", default=str(DEFAULT_ARTIFACT))
    parser.add_argument("--remote-bin")
    parser.add_argument("--session", default="local://remote-smoke")
    parser.add_argument("--session-kind", choices=("plain", "codex"), default="plain")
    parser.add_argument("--backend", choices=("x11", "wayland", "auto"), default="x11")
    parser.add_argument("--out-dir")
    parser.add_argument("--remote-dir")
    parser.add_argument("--timeout-ms", type=int, default=20000)
    parser.add_argument("--smoke-timeout-sec", type=int)
    parser.add_argument("--only-check", action="append", default=[])
    parser.add_argument("--keep-remote-dir", action="store_true")
    args = parser.parse_args()
    if args.smoke_timeout_sec is None:
        if args.only_check:
            args.smoke_timeout_sec = max(420, 120 + 40 * len(args.only_check))
        else:
            args.smoke_timeout_sec = 900
    return args


def quote(value: str) -> str:
    return shlex.quote(value)


def remote_file_text_if_exists(host: str, path: str, max_bytes: int = 16384) -> str:
    proc = ssh_shell(
        host,
        f"if test -f {quote(path)}; then tail -c {max_bytes} {quote(path)}; fi",
        check=False,
    )
    return (proc.stdout or "").strip()


def remote_panic_log(host: str, home: str) -> dict | None:
    path = f"{home.rstrip('/')}/panic.log"
    text = remote_file_text_if_exists(host, path)
    if not text:
        return None
    return {
        "path": path,
        "tail": text,
    }


def configure_remote_transport(proxy_jump: str | None, ssh_port: int | None) -> None:
    if proxy_jump:
        os.environ["YGGTERM_REMOTE_PROXY_JUMP"] = proxy_jump
    else:
        os.environ.pop("YGGTERM_REMOTE_PROXY_JUMP", None)
    if ssh_port:
        os.environ["YGGTERM_REMOTE_PORT"] = str(ssh_port)
    else:
        os.environ.pop("YGGTERM_REMOTE_PORT", None)


def _ssh_transport_args() -> list[str]:
    args: list[str] = []
    proxy_jump = str(os.environ.get("YGGTERM_REMOTE_PROXY_JUMP") or "").strip()
    if proxy_jump:
        args.extend(["-J", proxy_jump])
    ssh_port = str(os.environ.get("YGGTERM_REMOTE_PORT") or "").strip()
    if ssh_port:
        args.extend(["-p", ssh_port])
    return args


def _scp_transport_args() -> list[str]:
    args: list[str] = []
    proxy_jump = str(os.environ.get("YGGTERM_REMOTE_PROXY_JUMP") or "").strip()
    if proxy_jump:
        args.extend(["-J", proxy_jump])
    ssh_port = str(os.environ.get("YGGTERM_REMOTE_PORT") or "").strip()
    if ssh_port:
        args.extend(["-P", ssh_port])
    return args


def ssh_base() -> list[str]:
    connect_timeout = str(os.environ.get("YGGTERM_REMOTE_CONNECT_TIMEOUT") or "").strip() or "40"
    return [
        "ssh",
        "-o",
        "BatchMode=yes",
        "-o",
        f"ConnectTimeout={connect_timeout}",
        "-o",
        "StrictHostKeyChecking=accept-new",
        *_ssh_transport_args(),
    ]


def scp_base() -> list[str]:
    connect_timeout = str(os.environ.get("YGGTERM_REMOTE_CONNECT_TIMEOUT") or "").strip() or "40"
    return [
        "scp",
        "-o",
        "BatchMode=yes",
        "-o",
        f"ConnectTimeout={connect_timeout}",
        "-o",
        "StrictHostKeyChecking=accept-new",
        *_scp_transport_args(),
    ]


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
        [*ssh_base(), host, f"bash -lc {quote(command)}"],
        check=check,
        timeout_seconds=timeout_seconds,
    )


def ssh_python_json(host: str, snippet: str) -> dict:
    proc = local_run([*ssh_base(), host, "python3", "-"], input_text=snippet, check=True)
    text = proc.stdout.strip()
    if not text:
        raise RuntimeError(f"remote python returned empty output on {host}")
    return json.loads(text)


def scp_to(host: str, local_path: Path, remote_path: str, *, timeout_seconds: float = 180.0) -> None:
    local_run(
        [*scp_base(), str(local_path), f"{host}:{remote_path}"],
        check=True,
        timeout_seconds=timeout_seconds,
    )


def scp_from(
    host: str,
    remote_path: str,
    local_path: Path,
    *,
    timeout_seconds: float = 180.0,
) -> None:
    local_path.parent.mkdir(parents=True, exist_ok=True)
    local_run(
        [*scp_base(), "-r", f"{host}:{remote_path}", str(local_path)],
        check=True,
        timeout_seconds=timeout_seconds,
    )


def maybe_stage_linux_companion_binaries(host: str, artifact: Path, remote_dir: str) -> dict[str, str]:
    staged: dict[str, str] = {}
    artifact_name = artifact.name
    companion_specs = [
        (
            "yggterm-headless",
            artifact_name.replace("yggterm-", "yggterm-headless-", 1)
            if artifact_name.startswith("yggterm-")
            else "yggterm-headless",
        ),
        (
            "yggterm-mock-cli",
            artifact_name.replace("yggterm-", "yggterm-mock-cli-", 1)
            if artifact_name.startswith("yggterm-")
            else "yggterm-mock-cli",
        ),
    ]
    for remote_name, sibling_name in companion_specs:
        sibling = artifact.with_name(sibling_name)
        if not sibling.exists():
            continue
        remote_path = f"{remote_dir}/{remote_name}"
        scp_to(host, sibling, remote_path)
        ssh_shell(host, f"chmod +x {quote(remote_path)}")
        staged[remote_name] = remote_path
    return staged


def remote_user_service_status(host: str, unit: str) -> dict:
    proc = ssh_shell(
        host,
        f"systemctl --user show {quote(unit)} -p LoadState -p ActiveState -p SubState -p MainPID",
        check=False,
    )
    props: dict[str, str] = {}
    for line in (proc.stdout or "").splitlines():
        if "=" not in line:
            continue
        key, value = line.split("=", 1)
        props[key.strip()] = value.strip()
    load_state = str(props.get("LoadState") or "").strip()
    active_state = str(props.get("ActiveState") or "").strip()
    sub_state = str(props.get("SubState") or "").strip()
    main_pid_text = str(props.get("MainPID") or "").strip()
    try:
        main_pid = int(main_pid_text or "0")
    except ValueError:
        main_pid = 0
    available = bool(load_state) and load_state not in {"not-found", "masked"}
    return {
        "unit": unit,
        "available": available,
        "load_state": load_state,
        "active": active_state == "active",
        "active_state": active_state,
        "sub_state": sub_state,
        "pid": main_pid,
        "returncode": proc.returncode,
        "stderr": (proc.stderr or "").strip(),
    }


def remote_restart_user_service(host: str, unit: str) -> dict:
    restart_proc = ssh_shell(
        host,
        f"systemctl --user restart {quote(unit)}",
        check=False,
        timeout_seconds=20.0,
    )
    time.sleep(2.0)
    status = remote_user_service_status(host, unit)
    status["restart_returncode"] = restart_proc.returncode
    status["restart_stderr"] = (restart_proc.stderr or "").strip()
    status["restart_stdout"] = (restart_proc.stdout or "").strip()
    return status


def detect_user_service_regression(before: dict, after: dict) -> str | None:
    if not before.get("available"):
        return None
    before_active = bool(before.get("active"))
    after_active = bool(after.get("active"))
    before_pid = int(before.get("pid") or 0)
    after_pid = int(after.get("pid") or 0)
    unit = str(before.get("unit") or after.get("unit") or "service")
    if before_active and not after_active:
        return f"{unit} became inactive during smoke"
    if before_active and before_pid > 0 and after_active and after_pid > 0 and before_pid != after_pid:
        return f"{unit} pid changed during smoke ({before_pid} -> {after_pid})"
    return None


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
        maybe_stage_linux_companion_binaries(host, artifact, remote_dir)
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
    tmp_dir = f"{remote_dir}/tmp"
    ssh_shell(
        host,
        f"mkdir -p {quote(tmp_dir)} && "
        f"TMPDIR={quote(tmp_dir)} python3 -m venv {quote(venv_dir)} && "
        f"TMPDIR={quote(tmp_dir)} {quote(venv_dir)}/bin/python -m pip install --quiet --upgrade pip Pillow",
    )
    return f"{venv_dir}/bin/python"


def remote_owned_clients(host: str) -> dict:
    return ssh_python_json(host, OWNED_CLIENTS_SNIPPET)


def remote_notify(host: str, env: dict[str, str], title: str, body: str) -> None:
    exports = remote_env_exports(env)
    ssh_shell(
        host,
        f"{exports}; if command -v notify-send >/dev/null 2>&1; then "
        f"-h {quote('string:x-canonical-private-synchronous:yggterm-remote-smoke')} "
        f"{quote(title)} {quote(body)}; fi",
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


def remote_ensure_local_daemon_ready(
    host: str,
    remote_bin: str,
    env: dict[str, str],
    *,
    timeout_seconds: float = 20.0,
) -> dict:
    deadline = time.time() + timeout_seconds
    exports = remote_env_exports(env)
    last_error = ""
    while time.time() < deadline:
        proc = ssh_shell(
            host,
            f"{exports}; {quote(remote_bin)} server snapshot >/dev/null",
            check=False,
            timeout_seconds=timeout_seconds,
        )
        if proc.returncode == 0:
            return {"ok": True, "stderr": "", "stdout": (proc.stdout or "").strip()}
        last_error = (proc.stderr or proc.stdout or "").strip()
        time.sleep(0.35)
    return {"ok": False, "stderr": last_error}


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


def wait_for_remote_owned_clients_gone(host: str, timeout_seconds: float = 8.0) -> dict:
    deadline = time.time() + timeout_seconds
    last_inventory = remote_owned_clients(host)
    while time.time() < deadline:
        if int(last_inventory.get("count") or 0) == 0:
            return last_inventory
        time.sleep(0.25)
        last_inventory = remote_owned_clients(host)
    return last_inventory


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
            client = data.get("client_instance") or {}
            client_pid = int(client.get("pid") or 0)
            dom_ready = (
                dom.get("shell_root_count") == 1
                or dom.get("degraded_reason") == "dom_debug_snapshot_timeout"
                or dom.get("error") == "dom_debug_snapshot_timeout"
            )
            if visible_ok and (dom_ready or client_pid == pid):
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


def remote_launch_visible_window(
    host: str,
    remote_bin: str,
    env: dict[str, str],
    *,
    timeout_ms: int,
    remote_log: str,
) -> dict:
    exports = remote_env_exports(env)
    launch = ssh_shell(
        host,
        f"{exports}; {quote(remote_bin)} server app launch "
        f"--allow-multi-window --skip-active-exec-handoff --timeout-ms {timeout_ms} "
        f"--log {quote(remote_log)}",
    )
    payload = json.loads(launch.stdout.strip() or "{}")
    pid = int(payload.get("pid") or 0)
    if pid <= 0:
        raise RuntimeError(
            f"background app launch did not return a pid on {host}: {launch.stdout!r}"
    )
    return payload


def remote_launch_direct_window(
    host: str,
    remote_bin: str,
    env: dict[str, str],
    *,
    timeout_ms: int,
    remote_log: str,
) -> dict:
    exports = remote_env_exports(env)
    launch = ssh_shell(
        host,
        f"{exports}; nohup {quote(remote_bin)} > {quote(remote_log)} 2>&1 < /dev/null & echo $!",
    )
    pid = int((launch.stdout or "").strip() or 0)
    if pid <= 0:
        raise RuntimeError(
            f"direct background app launch did not return a pid on {host}: {launch.stdout!r}"
        )
    registered = False
    deadline = time.time() + max(timeout_ms / 1000.0, 1.0)
    while time.time() < deadline:
        inventory = remote_owned_clients(host)
        if any(int(client.get("pid") or 0) == pid for client in inventory.get("clients") or []):
            registered = True
            break
        time.sleep(0.25)
    return {
        "pid": pid,
        "log_path": remote_log,
        "registered": registered,
        "launch_mode": "direct_shell",
    }


def remote_kill_pid(host: str, pid: int) -> None:
    if pid <= 0:
        return
    ssh_shell(host, f"kill -TERM {pid} >/dev/null 2>&1 || true", check=False)
    time.sleep(0.5)
    alive = ssh_shell(host, f"kill -0 {pid} >/dev/null 2>&1", check=False).returncode == 0
    if alive:
        ssh_shell(host, f"kill -KILL {pid} >/dev/null 2>&1 || true", check=False)
        time.sleep(0.2)


def remote_kill_yggterm_processes_for_home(host: str, yggterm_home: str) -> dict:
    if not yggterm_home:
        return {"home": yggterm_home, "pids": [], "killed": []}
    snippet = f"""
import json
import os
import pathlib
import signal
import time

home = {json.dumps(yggterm_home)}
needle = ("YGGTERM_HOME=" + home).encode()
pids = []
for name in os.listdir("/proc"):
    if not name.isdigit():
        continue
    pid = int(name)
    if pid == os.getpid():
        continue
    proc_dir = pathlib.Path("/proc") / name
    try:
        env = proc_dir.joinpath("environ").read_bytes().split(b"\\0")
        if needle not in env:
            continue
        cmdline = [
            part.decode("utf-8", "ignore")
            for part in proc_dir.joinpath("cmdline").read_bytes().split(b"\\0")
            if part
        ]
    except Exception:
        continue
    exe = pathlib.Path(cmdline[0]).name if cmdline else ""
    if exe.startswith("yggterm"):
        pids.append(pid)

for sig in (signal.SIGTERM, signal.SIGKILL):
    for pid in pids:
        try:
            os.kill(pid, sig)
        except ProcessLookupError:
            pass
        except PermissionError:
            pass
    time.sleep(0.35 if sig == signal.SIGTERM else 0.1)

alive = []
for pid in pids:
    try:
        os.kill(pid, 0)
        alive.append(pid)
    except ProcessLookupError:
        pass
    except PermissionError:
        alive.append(pid)

print(json.dumps({{"home": home, "pids": pids, "alive_after_kill": alive}}))
"""
    proc = local_run([*ssh_base(), host, "python3", "-"], input_text=snippet, check=False)
    if proc.returncode != 0:
        return {
            "home": yggterm_home,
            "pids": [],
            "alive_after_kill": [],
            "error": proc.stderr.strip() or proc.stdout.strip(),
        }
    text = proc.stdout.strip()
    return json.loads(text) if text else {"home": yggterm_home, "pids": [], "alive_after_kill": []}


def wait_for_remote_pid_gone(host: str, pid: int, timeout_seconds: float = 8.0) -> dict:
    deadline = time.time() + timeout_seconds
    checks = 0
    while time.time() < deadline:
        checks += 1
        alive = ssh_shell(host, f"kill -0 {pid} >/dev/null 2>&1", check=False).returncode == 0
        if not alive:
            return {
                "pid": pid,
                "exited": True,
                "checks": checks,
            }
        time.sleep(0.25)
    return {
        "pid": pid,
        "exited": False,
        "checks": checks,
    }


def remote_kde_terminal_close_probe(
    host: str,
    remote_bin: str,
    env: dict[str, str],
    *,
    timeout_ms: int,
    remote_log: str,
) -> dict:
    launch_mode = "app_cli"
    launch_visibility_error = None
    app_cli_cleanup = None
    direct_fallback_cleanup = None

    def launch_direct_fallback(launch_error: Exception) -> tuple[dict, int]:
        nonlocal launch_mode, launch_visibility_error, app_cli_cleanup, direct_fallback_cleanup
        launch_visibility_error = str(launch_error)
        app_cli_cleanup = remote_kill_yggterm_processes_for_home(
            host,
            env.get("YGGTERM_HOME", ""),
        )
        fallback_log = f"{remote_log}.direct"
        fallback_payload = remote_launch_direct_window(
            host,
            remote_bin,
            env,
            timeout_ms=timeout_ms,
            remote_log=fallback_log,
        )
        launch_mode = "direct_shell_fallback"
        fallback_pid = int(fallback_payload.get("pid") or 0)
        try:
            wait_for_remote_state(host, remote_bin, env, fallback_pid, timeout_ms)
        except RuntimeError:
            direct_fallback_cleanup = remote_kill_yggterm_processes_for_home(
                host,
                env.get("YGGTERM_HOME", ""),
            )
            raise
        return fallback_payload, fallback_pid

    try:
        launch_payload = remote_launch_visible_window(
            host,
            remote_bin,
            env,
            timeout_ms=timeout_ms,
            remote_log=remote_log,
        )
        pid = int(launch_payload.get("pid") or 0)
        try:
            wait_for_remote_state(host, remote_bin, env, pid, timeout_ms)
        except RuntimeError as launch_error:
            remote_kill_pid(host, pid)
            launch_payload, pid = launch_direct_fallback(launch_error)
    except RuntimeError as launch_error:
        launch_payload, pid = launch_direct_fallback(launch_error)
    session_path = remote_create_plain_terminal(
        host,
        remote_bin,
        env,
        pid,
        title="KDE Close Probe",
    )
    time.sleep(2.0)
    close_response = remote_close_window(host, remote_bin, env, pid, timeout_ms)
    close_exit = wait_for_remote_pid_gone(host, pid)
    if not close_exit.get("exited"):
        remote_kill_pid(host, pid)
        after_kill = wait_for_remote_pid_gone(host, pid, timeout_seconds=3.0)
        close_exit["killed_after_timeout"] = True
        close_exit["after_kill"] = after_kill
    exports = remote_env_exports(env)
    shutdown_proc = ssh_shell(
        host,
        f"{exports}; {quote(remote_bin)} server shutdown >/dev/null 2>&1 || true",
        check=False,
    )
    panic_log = remote_panic_log(host, env.get("YGGTERM_HOME", ""))
    return {
        "launch": launch_payload,
        "launch_mode": launch_mode,
        "launch_visibility_error": launch_visibility_error,
        "app_cli_cleanup_after_launch_failure": app_cli_cleanup,
        "direct_fallback_cleanup_after_launch_failure": direct_fallback_cleanup,
        "session_path": session_path,
        "close": close_response,
        "close_exit": close_exit,
        "shutdown_returncode": shutdown_proc.returncode,
        "panic_log": panic_log,
        "plasmashell_after": remote_user_service_status(host, "plasma-plasmashell.service"),
    }


def main() -> int:
    args = parse_args()
    configure_remote_transport(args.proxy_jump, args.ssh_port)
    timestamp = int(time.time())
    out_dir = Path(args.out_dir or f"/tmp/yggterm-remote-smoke-{args.host}-{timestamp}")
    out_dir.mkdir(parents=True, exist_ok=True)

    metadata: dict[str, object] = {
        "host": args.host,
        "session": args.session,
        "session_kind": args.session_kind,
        "backend": args.backend,
    }
    metadata_path = out_dir / "summary.json"
    smoke_proc: subprocess.CompletedProcess | None = None
    remote_dir: str | None = None
    try:
        cleanup_summary = remote_cleanup_owned_clients(args.host, args.timeout_ms)
        metadata["owned_clients_cleanup"] = cleanup_summary
        session_info = ssh_python_json(args.host, LINUX_SESSION_SNIPPET)
        metadata["session_info"] = session_info
        plasmashell_before = remote_user_service_status(args.host, "plasma-plasmashell.service")
        if plasmashell_before.get("available") and not plasmashell_before.get("active"):
            plasmashell_before = remote_restart_user_service(args.host, "plasma-plasmashell.service")
            metadata["plasmashell_restarted_before"] = plasmashell_before
        metadata["plasmashell_before"] = plasmashell_before
        desktop_notifications_enabled = not bool(plasmashell_before.get("available"))
        metadata["desktop_notifications_enabled"] = desktop_notifications_enabled
        if not desktop_notifications_enabled:
            metadata["desktop_notifications_suppressed_reason"] = (
                "suppressed notify-send because Plasma notifications are crashing on this host"
            )
        if not session_info.get("python3"):
            raise RuntimeError(f"python3 is missing on {args.host}")
        remote_dir = args.remote_dir or (
            f"{str(session_info.get('home_dir') or '').rstrip('/')}/.cache/yggterm-remote-smoke-{timestamp}"
        )
        if not str(remote_dir).startswith("/"):
            raise RuntimeError(f"could not resolve a writable remote home dir on {args.host}: {session_info!r}")
        ssh_shell(args.host, f"rm -rf {quote(remote_dir)} && mkdir -p {quote(remote_dir)}")
        metadata["remote_dir"] = remote_dir

        picked = session_info.get("picked_session") or {}
        leader_env = session_info.get("leader_env") or {}
        desktop_env = session_info.get("desktop_env") or leader_env
        picked_type = str(picked.get("Type") or "").strip().lower()
        runtime_dir = str(
            desktop_env.get("XDG_RUNTIME_DIR")
            or leader_env.get("XDG_RUNTIME_DIR")
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
            "YGGTERM_REMOTE_SMOKE_TAG": "1",
            "NO_AT_BRIDGE": "1",
        }
        picked_session_id = str(picked.get("session_id") or picked.get("Name") or "").strip()
        if picked_session_id:
            launch_env["XDG_SESSION_ID"] = picked_session_id
        if picked_type:
            launch_env["XDG_SESSION_TYPE"] = picked_type
        picked_class = str(picked.get("Class") or "").strip()
        if picked_class:
            launch_env["XDG_SESSION_CLASS"] = picked_class
        for desktop_key in (
            "DESKTOP_SESSION",
            "XDG_CURRENT_DESKTOP",
            "XDG_SESSION_DESKTOP",
            "KDE_FULL_SESSION",
        ):
            desktop_value = str(desktop_env.get(desktop_key) or "").strip()
            if desktop_value:
                launch_env[desktop_key] = desktop_value
        screenshot_wayland_display = str(
            desktop_env.get("WAYLAND_DISPLAY") or leader_env.get("WAYLAND_DISPLAY") or ""
        ).strip()
        if screenshot_wayland_display:
            launch_env["YGGTERM_SCREENSHOT_WAYLAND_DISPLAY"] = screenshot_wayland_display
            launch_env["YGGTERM_ROOT_SCREENSHOT_FOCUS"] = "1"
        effective_backend = args.backend
        if effective_backend == "auto":
            if picked_type == "wayland" and plasmashell_before.get("available"):
                effective_backend = "x11"
                metadata["effective_backend_reason"] = (
                    "auto-fell back to X11 because native Wayland smoke is destabilizing Plasma on this host"
                )
            else:
                effective_backend = "wayland" if picked_type == "wayland" else "x11"
        if effective_backend == "wayland":
            wayland_display = str(
                desktop_env.get("WAYLAND_DISPLAY") or leader_env.get("WAYLAND_DISPLAY") or ""
            ).strip()
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
            display_value = str(
                desktop_env.get("DISPLAY") or leader_env.get("DISPLAY") or ""
            ).strip()
            xauthority_value = str(
                desktop_env.get("XAUTHORITY") or leader_env.get("XAUTHORITY") or ""
            ).strip()
            if display_value:
                launch_env["DISPLAY"] = display_value
            if xauthority_value:
                launch_env["XAUTHORITY"] = xauthority_value
        else:
            x11_display = str(
                session_info.get("xwayland_display")
                or desktop_env.get("DISPLAY")
                or leader_env.get("DISPLAY")
                or ""
            ).strip()
            x11_xauthority = str(
                session_info.get("xwayland_xauthority")
                or desktop_env.get("XAUTHORITY")
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
        desktop_markers = " ".join(
            str(desktop_env.get(key) or "")
            for key in (
                "DESKTOP_SESSION",
                "XDG_CURRENT_DESKTOP",
                "XDG_SESSION_DESKTOP",
                "KDE_FULL_SESSION",
            )
        ).strip()
        kde_desktop = bool(plasmashell_before.get("available")) or any(
            marker in desktop_markers.lower() for marker in ("kde", "plasma")
        )
        metadata["desktop_markers"] = desktop_markers
        metadata["kde_desktop"] = kde_desktop

        ssh_shell(args.host, f"mkdir -p {quote(remote_home)} {quote(remote_out)}")
        exports = remote_env_exports(launch_env)
        metadata["daemon_prewarm"] = remote_ensure_local_daemon_ready(
            args.host,
            remote_bin,
            launch_env,
        )
        if not (metadata.get("daemon_prewarm") or {}).get("ok"):
            raise RuntimeError(
                "failed to prewarm the temp-home daemon before GUI launch: "
                f"{(metadata.get('daemon_prewarm') or {}).get('stderr')!r}"
            )
        if kde_desktop and effective_backend == "x11" and plasmashell_before.get("available"):
            probe_home = f"{remote_dir}/kde-close-probe-home"
            probe_log = f"{remote_dir}/kde-close-probe.log"
            probe_env = dict(launch_env)
            probe_env["YGGTERM_HOME"] = probe_home
            ssh_shell(args.host, f"mkdir -p {quote(probe_home)}")
            metadata["kde_terminal_close_probe"] = remote_kde_terminal_close_probe(
                args.host,
                remote_bin,
                probe_env,
                timeout_ms=args.timeout_ms,
                remote_log=probe_log,
            )
            probe_panic_log = (metadata.get("kde_terminal_close_probe") or {}).get("panic_log")
            if probe_panic_log:
                raise RuntimeError(
                    "kde terminal close probe panicked before the main launcher smoke: "
                    f"{probe_panic_log.get('path')}"
                )
            probe_close_exit = (
                (metadata.get("kde_terminal_close_probe") or {}).get("close_exit") or {}
            )
            if not probe_close_exit.get("exited"):
                raise RuntimeError(
                    "kde terminal close probe did not exit after app-control close"
                )
            probe_plasmashell_after = (
                (metadata.get("kde_terminal_close_probe") or {}).get("plasmashell_after") or {}
            )
            if not probe_plasmashell_after.get("active"):
                metadata["plasmashell_restarted_after_kde_terminal_close_probe"] = (
                    remote_restart_user_service(args.host, "plasma-plasmashell.service")
                )
                raise RuntimeError(
                    "kde terminal close probe crashed plasma-plasmashell.service"
                )
        if desktop_notifications_enabled:
            remote_notify(
                args.host,
                launch_env,
                "Yggterm automated testing",
                "Automated testing is starting. The Yggterm window will move to the background when possible.",
        )
        launch_payload = remote_launch_visible_window(
            args.host,
            remote_bin,
            launch_env,
            timeout_ms=args.timeout_ms,
            remote_log=remote_log,
        )
        pid = int(launch_payload.get("pid") or 0)
        metadata["launch_response"] = launch_payload
        metadata["pid"] = pid
        metadata["launch_mode"] = "app_cli"
        try:
            initial_state = wait_for_remote_state(
                args.host, remote_bin, launch_env, pid, args.timeout_ms
            )
        except RuntimeError as launch_error:
            metadata["launch_visibility_error"] = str(launch_error)
            remote_kill_pid(args.host, pid)
            fallback_log = f"{remote_dir}/client-direct.log"
            fallback_launch = remote_launch_direct_window(
                args.host,
                remote_bin,
                launch_env,
                timeout_ms=args.timeout_ms,
                remote_log=fallback_log,
            )
            pid = int(fallback_launch.get("pid") or 0)
            metadata["launch_fallback_response"] = fallback_launch
            metadata["launch_mode"] = "direct_shell_fallback"
            metadata["pid"] = pid
            initial_state = wait_for_remote_state(
                args.host, remote_bin, launch_env, pid, args.timeout_ms
            )
        initial_bad_notifications = remote_problem_notifications(initial_state)
        if initial_bad_notifications:
            raise RuntimeError(
                f"bad daemon/socket notifications were already visible right after launch: {initial_bad_notifications!r}"
            )
        write_json(out_dir / "initial-state.json", initial_state)
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

        try:
            only_check_args = "".join(
                f" --only-check {quote(check_name)}" for check_name in args.only_check
            )
            smoke_proc = ssh_shell(
                args.host,
                f"{exports}; YGGTERM_BIN={quote(remote_bin)} YGGTERM_SMOKE_AVOID_FOREGROUND=1 "
                f"{quote(python_cmd)} {quote(remote_smoke_script)} "
                f"--bin {quote(remote_bin)} --pid {pid} --session {quote(smoke_session)} "
                f"--session-kind {quote(args.session_kind)} --out {quote(remote_out)} --home {quote(remote_home)}"
                f"{only_check_args}",
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
        try:
            metadata["background_response_after_smoke"] = remote_background_window(
                args.host,
                remote_bin,
                launch_env,
                pid,
                args.timeout_ms,
            )
        except Exception as exc:
            metadata["background_error_after_smoke"] = str(exc)

        pulled_dir = out_dir / "proof"
        pulled_dir.mkdir(parents=True, exist_ok=True)
        metadata["local_proof_dir"] = str(pulled_dir)
        write_json(metadata_path, metadata)
        proof_pull_timeout = max(60.0, min(float(args.smoke_timeout_sec), 180.0))
        try:
            scp_from(
                args.host,
                f"{remote_out}/.",
                pulled_dir,
                timeout_seconds=proof_pull_timeout,
            )
        except Exception as exc:
            metadata["proof_pull_error"] = str(exc)
        perf_remote = f"{remote_home}/perf-telemetry.jsonl"
        perf_proc = ssh_shell(args.host, f"test -f {quote(perf_remote)}", check=False)
        if perf_proc.returncode == 0:
            try:
                scp_from(
                    args.host,
                    perf_remote,
                    out_dir / "perf-telemetry.jsonl",
                    timeout_seconds=60.0,
                )
            except Exception as exc:
                metadata["perf_pull_error"] = str(exc)

        write_json(metadata_path, metadata)

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
        metadata["owned_clients_after_close"] = wait_for_remote_owned_clients_gone(args.host)
        final_owned_clients = metadata["owned_clients_after_close"]
        if int((final_owned_clients or {}).get("count") or 0) != 0:
            metadata["owned_clients_cleanup_after_close"] = remote_cleanup_owned_clients(
                args.host,
                args.timeout_ms,
            )
            final_owned_clients = (metadata["owned_clients_cleanup_after_close"] or {}).get(
                "after"
            )
        if int((final_owned_clients or {}).get("count") or 0) != 0:
            metadata["owned_clients_cleanup_after_close_retry"] = remote_cleanup_owned_clients(
                args.host,
                args.timeout_ms,
            )
            final_owned_clients = (
                metadata["owned_clients_cleanup_after_close_retry"] or {}
            ).get("after")
        metadata["owned_clients_final"] = final_owned_clients
        plasmashell_after = remote_user_service_status(args.host, "plasma-plasmashell.service")
        metadata["plasmashell_after"] = plasmashell_after
        plasmashell_problem = detect_user_service_regression(
            metadata.get("plasmashell_before") or {},
            plasmashell_after,
        )
        if plasmashell_problem:
            metadata["plasmashell_problem"] = plasmashell_problem
            if plasmashell_after.get("available") and not plasmashell_after.get("active"):
                metadata["plasmashell_restarted_after"] = remote_restart_user_service(
                    args.host,
                    "plasma-plasmashell.service",
                )
        owned_clients_survived = int((final_owned_clients or {}).get("count") or 0) != 0
        metadata["ok"] = (
            (smoke_proc is not None and smoke_proc.returncode == 0)
            and not bool(metadata.get("final_problem_notifications"))
            and not bool(metadata.get("plasmashell_problem"))
            and not owned_clients_survived
        )
        if metadata.get("plasmashell_problem") and not metadata.get("error"):
            metadata["error"] = metadata["plasmashell_problem"]
        if owned_clients_survived and not metadata.get("error"):
            metadata["error"] = "owned Yggterm clients survived remote smoke cleanup"
        write_json(metadata_path, metadata)
        if desktop_notifications_enabled:
            remote_notify(
                args.host,
                launch_env,
                "Yggterm automated testing",
                "Automated testing finished. The test window should be in the background or closed.",
            )
        if not args.keep_remote_dir:
            ssh_shell(args.host, f"rm -rf {quote(remote_dir)}", check=False)
        print(metadata_path)
        if metadata.get("ok"):
            return 0
        if smoke_proc is not None and smoke_proc.returncode not in (None, 0):
            return smoke_proc.returncode
        return 1
    except Exception as exc:
        metadata["ok"] = False
        metadata["error"] = str(exc)
        try:
            write_json(metadata_path, metadata)
        except Exception:
            pass
        print(metadata_path)
        return 1
    finally:
        if args.keep_remote_dir and remote_dir:
            (out_dir / "remote-dir.txt").write_text(f"{remote_dir}\n", encoding="utf-8")


if __name__ == "__main__":
    raise SystemExit(main())
