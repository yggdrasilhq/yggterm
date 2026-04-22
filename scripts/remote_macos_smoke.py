#!/usr/bin/env python3
import argparse
import json
import re
import time
from pathlib import Path

from remote_linux_x11_smoke import quote, scp_from, scp_to, ssh_shell
from smoke_app_control_bootstrap import (
    assert_sidebar_rows_present,
    assert_blur_expectation,
    assert_screenshot_file_usable,
    problem_notifications,
    screenshot_backend,
    screenshot_backend_attempts,
)


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ICON = ROOT / "assets" / "brand" / "yggterm-icon-512.png"
REMOTE_MACOS_HOME_PREFIX = "yggterm-remote-macos-"


def default_artifact() -> Path:
    candidates = [
        ROOT / "dist" / "yggterm-macos-x86_64.app.zip",
        ROOT / "dist" / "Yggterm-macos-x86_64.app.zip",
        ROOT / "dist" / "yggterm-macos-x86_64",
    ]
    for candidate in candidates:
        if candidate.exists():
            return candidate
    return candidates[-1]


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Stage or attach to Yggterm on a remote macOS host and run a minimal app-control smoke."
    )
    parser.add_argument("--host", required=True)
    parser.add_argument("--artifact", default=str(default_artifact()))
    parser.add_argument("--remote-bin")
    parser.add_argument("--out-dir")
    parser.add_argument("--remote-dir")
    parser.add_argument("--timeout-ms", type=int, default=30000)
    parser.add_argument("--attach-only", action="store_true")
    parser.add_argument("--install", action="store_true")
    parser.add_argument("--keep-remote-dir", action="store_true")
    parser.add_argument(
        "--expect-live-blur",
        choices=("ignore", "required", "forbidden"),
        default="ignore",
    )
    return parser.parse_args()


def resolve_remote_dir(host: str, remote_dir: str) -> str:
    if remote_dir.startswith("/"):
        return remote_dir
    proc = ssh_shell(host, "printf '%s\\n' \"$HOME\"")
    home_dir = proc.stdout.strip()
    if not home_dir:
        raise RuntimeError(f"could not resolve remote home directory on {host}")
    return f"{home_dir}/{remote_dir}"


def remote_env_exports(env: dict[str, str]) -> str:
    chunks = []
    for key, value in env.items():
        if value:
            chunks.append(f"export {key}={quote(value)}")
    return "; ".join(chunks)


def frontmost_macos_app_name(host: str) -> str | None:
    proc = ssh_shell(
        host,
        'osascript -e \'tell application "System Events" to get name of first application process whose frontmost is true\'',
        check=False,
    )
    text = proc.stdout.strip() or proc.stderr.strip()
    return text or None


def remote_notify_macos(host: str, title: str, body: str) -> None:
    ssh_shell(
        host,
        "osascript -e "
        + quote(f'display notification "{body}" with title "{title}"'),
        check=False,
    )


def extract_remote_macos_root(command: str) -> str:
    marker = command.find(REMOTE_MACOS_HOME_PREFIX)
    if marker < 0:
        return ""
    start = marker
    while start > 0 and not command[start - 1].isspace() and command[start - 1] not in "\"'":
        start -= 1
    token = command[start:].split(None, 1)[0].strip().strip("\"'")
    marker = token.find(REMOTE_MACOS_HOME_PREFIX)
    if marker < 0:
        return ""
    prefix = token[:marker].rstrip("/")
    suffix = token[marker:]
    root_name = suffix.split("/", 1)[0]
    if prefix.startswith("/"):
        return f"{prefix}/{root_name}" if prefix else f"/{root_name}"
    return root_name


def remote_macos_process_rows(host: str) -> list[dict]:
    proc = ssh_shell(
        host,
        "ps -ww -axo pid=,ppid=,etime=,command=",
        check=False,
    )
    rows: list[dict] = []
    for line in proc.stdout.splitlines():
        match = re.match(r"\s*(\d+)\s+(\d+)\s+(\S+)\s+(.*)\Z", line)
        if not match:
            continue
        rows.append(
            {
                "pid": int(match.group(1)),
                "ppid": int(match.group(2)),
                "etime": match.group(3),
                "command": match.group(4).strip(),
            }
        )
    return rows


def remote_macos_yggterm_processes(host: str) -> dict:
    clients = [
        row
        for row in remote_macos_process_rows(host)
        if "yggterm" in str(row.get("command") or "").lower()
    ]
    return {"count": len(clients), "clients": clients}


def scope_matches_remote_dir(command: str, scope: str) -> bool:
    if not scope:
        return True
    scope_name = Path(scope).name
    return scope in command or (scope_name and scope_name in command)


def remote_owned_macos_clients(host: str, remote_dir: str | None = None) -> dict:
    scope = (remote_dir or "").strip()
    clients: list[dict] = []
    for row in remote_macos_process_rows(host):
        command = str(row.get("command") or "").strip()
        if REMOTE_MACOS_HOME_PREFIX not in command:
            continue
        if scope and not scope_matches_remote_dir(command, scope):
            continue
        clients.append(
            {
                "pid": int(row.get("pid") or 0),
                "ppid": int(row.get("ppid") or 0),
                "etime": row.get("etime"),
                "command": command,
                "root": extract_remote_macos_root(command),
            }
        )
    return {"count": len(clients), "clients": clients}


def wait_for_remote_owned_macos_clients_gone(
    host: str, remote_dir: str, timeout_seconds: float = 8.0
) -> dict:
    deadline = time.time() + timeout_seconds
    last_inventory = remote_owned_macos_clients(host, remote_dir)
    while time.time() < deadline:
        if int(last_inventory.get("count") or 0) == 0:
            return last_inventory
        time.sleep(0.25)
        last_inventory = remote_owned_macos_clients(host, remote_dir)
    return last_inventory


def kill_remote_owned_macos_clients(host: str, remote_dir: str) -> dict:
    inventory = remote_owned_macos_clients(host, remote_dir)
    pids = [int(client.get("pid") or 0) for client in inventory.get("clients") or []]
    pids = [pid for pid in pids if pid > 0]
    if pids:
        pid_args = " ".join(str(pid) for pid in pids)
        ssh_shell(host, f"kill -TERM {pid_args} >/dev/null 2>&1 || true", check=False)
        time.sleep(0.5)
        ssh_shell(host, f"kill -KILL {pid_args} >/dev/null 2>&1 || true", check=False)
    return {
        "before": inventory,
        "after": remote_owned_macos_clients(host, remote_dir),
    }


def cleanup_stale_remote_macos_clients(host: str) -> dict:
    remote_home_proc = ssh_shell(host, "printf '%s\\n' \"$HOME\"", check=False)
    remote_home = remote_home_proc.stdout.strip()
    inventory = remote_owned_macos_clients(host)
    stale_entries: list[dict] = []
    stale_pids: list[int] = []
    stale_roots: set[str] = set()
    cleanup_roots: set[str] = set()
    for client in inventory.get("clients") or []:
        pid = int(client.get("pid") or 0)
        command = str(client.get("command") or "").strip()
        root = str(client.get("root") or "").strip()
        stale_entries.append({"pid": pid, "command": command, "root": root})
        stale_pids.append(pid)
        if root:
            stale_roots.add(root)
            if root.startswith("/"):
                cleanup_roots.add(root)
            elif remote_home:
                cleanup_roots.add(f"{remote_home.rstrip('/')}/{root}")

    if stale_pids:
        pid_args = " ".join(str(pid) for pid in stale_pids)
        ssh_shell(host, f"kill -TERM {pid_args} >/dev/null 2>&1 || true", check=False)
        time.sleep(0.75)
        ssh_shell(host, f"kill -KILL {pid_args} >/dev/null 2>&1 || true", check=False)

    if cleanup_roots:
        root_args = " ".join(quote(root) for root in sorted(cleanup_roots))
        ssh_shell(host, f"rm -rf {root_args}", check=False)

    return {
        "count": len(stale_entries),
        "entries": stale_entries,
        "roots": sorted(stale_roots),
        "cleanup_roots": sorted(cleanup_roots),
    }


def macos_session_info(host: str) -> dict:
    proc = ssh_shell(
        host,
        r"""
uid="$(id -u)"
user="$(id -un)"
console_user="$(stat -f '%Su' /dev/console 2>/dev/null || true)"
arch="$(uname -m)"
python3_bin="$(command -v python3 || true)"
finder_count="$(pgrep -x Finder | wc -l | tr -d ' ')"
dock_count="$(pgrep -x Dock | wc -l | tr -d ' ')"
loginwindow_count="$(pgrep -x loginwindow | wc -l | tr -d ' ')"
aqua_domain_present=0
if launchctl print "gui/$uid" >/dev/null 2>&1; then
  aqua_domain_present=1
fi
printf 'uid=%s\n' "$uid"
printf 'user=%s\n' "$user"
printf 'console_user=%s\n' "$console_user"
printf 'arch=%s\n' "$arch"
printf 'python3=%s\n' "$python3_bin"
printf 'finder_count=%s\n' "$finder_count"
printf 'dock_count=%s\n' "$dock_count"
printf 'loginwindow_count=%s\n' "$loginwindow_count"
printf 'aqua_domain_present=%s\n' "$aqua_domain_present"
printf '__SW_VERS__\n'
sw_vers
""",
    )
    lines = proc.stdout.splitlines()
    props: dict[str, str] = {}
    sw_vers_lines: list[str] = []
    in_sw_vers = False
    for line in lines:
        if line.strip() == "__SW_VERS__":
            in_sw_vers = True
            continue
        if in_sw_vers:
            sw_vers_lines.append(line)
            continue
        if "=" in line:
            key, value = line.split("=", 1)
            props[key.strip()] = value.strip()
    return {
        "uid": int(props.get("uid") or 0),
        "user": props.get("user") or "",
        "console_user": props.get("console_user") or "",
        "arch": props.get("arch") or "",
        "python3": props.get("python3") or "",
        "finder_count": int(props.get("finder_count") or 0),
        "dock_count": int(props.get("dock_count") or 0),
        "loginwindow_count": int(props.get("loginwindow_count") or 0),
        "aqua_domain_present": props.get("aqua_domain_present") == "1",
        "sw_vers": "\n".join(sw_vers_lines).strip(),
    }


def macos_desktop_ready(session_info: dict) -> bool:
    return (
        session_info.get("console_user") == session_info.get("user")
        and bool(session_info.get("aqua_domain_present"))
        and int(session_info.get("finder_count") or 0) > 0
        and int(session_info.get("dock_count") or 0) > 0
    )


def stage_artifact(host: str, artifact: Path, remote_dir: str) -> tuple[str, str | None]:
    ssh_shell(host, f"mkdir -p {quote(remote_dir)}")
    remote_rel = f"{remote_dir}/{artifact.name}"
    lower_name = artifact.name.lower()
    scp_to(host, artifact, remote_rel)
    if lower_name.endswith(".tar.gz"):
        ssh_shell(
            host,
            f"tar -xzf {quote(remote_rel)} -C {quote(remote_dir)} && "
            f"find {quote(remote_dir)} -maxdepth 1 -type f -name 'yggterm*' "
            f"! -name '*headless*' ! -name '*mock-cli*' ! -name '*.sha256' -exec chmod +x {{}} +",
        )
        remote_bin = ssh_shell(
            host,
            f"find {quote(remote_dir)} -maxdepth 1 -type f -name 'yggterm*' "
            f"! -name '*headless*' ! -name '*mock-cli*' ! -name '*.sha256' | head -n1",
        ).stdout.strip()
        if not remote_bin:
            raise RuntimeError(f"could not resolve staged macOS yggterm binary from archive {artifact}")
        return remote_bin, None
    if lower_name.endswith(".zip"):
        ssh_shell(
            host,
            f"rm -rf {quote(remote_dir)}/Yggterm.app && "
            f"(command -v ditto >/dev/null 2>&1 && ditto -x -k {quote(remote_rel)} {quote(remote_dir)} "
            f"|| unzip -oq {quote(remote_rel)} -d {quote(remote_dir)})",
        )
        remote_app = ssh_shell(
            host,
            f"find {quote(remote_dir)} -maxdepth 2 -type d -name '*.app' | head -n1",
        ).stdout.strip()
        if remote_app:
            remote_bin = ssh_shell(
                host,
                f"find {quote(remote_app)}/Contents/MacOS -maxdepth 1 -type f | head -n1",
            ).stdout.strip()
            if not remote_bin:
                raise RuntimeError(
                    f"staged macOS app bundle {remote_app} did not expose an executable under Contents/MacOS"
                )
            ssh_shell(host, f"chmod +x {quote(remote_bin)}")
            return remote_bin, remote_app
        remote_bin = ssh_shell(
            host,
            f"find {quote(remote_dir)} -maxdepth 1 -type f -name 'yggterm*' "
            f"! -name '*headless*' ! -name '*mock-cli*' | head -n1",
        ).stdout.strip()
        if not remote_bin:
            raise RuntimeError(f"could not resolve staged macOS payload from zip {artifact}")
        ssh_shell(host, f"chmod +x {quote(remote_bin)}")
        return remote_bin, None
    ssh_shell(host, f"chmod +x {quote(remote_rel)}")
    return remote_rel, None


def stage_macos_app_bundle(host: str, remote_dir: str, remote_bin: str) -> str:
    remote_app = f"{remote_dir}/Yggterm.app"
    remote_icon = f"{remote_dir}/yggterm-icon-512.png"
    if DEFAULT_ICON.exists():
        scp_to(host, DEFAULT_ICON, remote_icon)
    ssh_shell(
        host,
        f"""
set -e
app={quote(remote_app)}
bin={quote(remote_bin)}
resources="$app/Contents/Resources"
macos="$app/Contents/MacOS"
mkdir -p "$resources" "$macos"
cp "$bin" "$macos/Yggterm"
chmod +x "$macos/Yggterm"
icon_file="yggterm.png"
if [ -f {quote(remote_icon)} ]; then
  cp {quote(remote_icon)} "$resources/yggterm.png"
  if command -v sips >/dev/null 2>&1 && command -v iconutil >/dev/null 2>&1; then
    iconset={quote(remote_dir + '/yggterm.iconset')}
    rm -rf "$iconset"
    mkdir -p "$iconset"
    for spec in \
      "16:icon_16x16.png" \
      "32:icon_16x16@2x.png" \
      "32:icon_32x32.png" \
      "64:icon_32x32@2x.png" \
      "128:icon_128x128.png" \
      "256:icon_128x128@2x.png" \
      "256:icon_256x256.png" \
      "512:icon_256x256@2x.png" \
      "512:icon_512x512.png" \
      "1024:icon_512x512@2x.png"
    do
      size="${{spec%%:*}}"
      name="${{spec#*:}}"
      sips -z "$size" "$size" {quote(remote_icon)} --out "$iconset/$name" >/dev/null
    done
    if iconutil -c icns "$iconset" -o "$resources/yggterm.icns" >/dev/null 2>&1; then
      icon_file="yggterm.icns"
    fi
    rm -rf "$iconset"
  fi
fi
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>Yggterm</string>
  <key>CFBundleDisplayName</key>
  <string>Yggterm</string>
  <key>CFBundleIdentifier</key>
  <string>dev.yggterm.Yggterm.Smoke</string>
  <key>CFBundleExecutable</key>
  <string>Yggterm</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleIconFile</key>
  <string>$icon_file</string>
  <key>LSBackgroundOnly</key>
  <false/>
</dict>
</plist>
PLIST
""",
    )
    return remote_app


def install_staged_macos_app_bundle(host: str, remote_app: str) -> dict:
    home_dir = ssh_shell(host, "printf '%s\\n' \"$HOME\"").stdout.strip()
    if not home_dir:
        raise RuntimeError(f"could not resolve remote home directory on {host}")
    install_root = f"{home_dir}/Applications"
    installed_app = f"{install_root}/Yggterm.app"
    ssh_shell(
        host,
        f"mkdir -p {quote(install_root)} && "
        f"rm -rf {quote(installed_app)} && "
        f"(command -v ditto >/dev/null 2>&1 && ditto {quote(remote_app)} {quote(installed_app)} "
        f"|| cp -R {quote(remote_app)} {quote(installed_app)})",
    )
    installed_bin = ssh_shell(
        host,
        f"find {quote(installed_app)}/Contents/MacOS -maxdepth 1 -type f | head -n1",
    ).stdout.strip()
    if not installed_bin:
        raise RuntimeError(f"installed macOS app bundle did not expose a launcher under {installed_app}")
    return {
        "install_root": install_root,
        "installed_app": installed_app,
        "installed_bin": installed_bin,
    }


def normalize_staged_macos_payload(
    host: str, remote_dir: str, remote_bin: str, remote_app: str | None
) -> tuple[str, str | None]:
    lower_remote = remote_bin.lower()
    if lower_remote.endswith(".tar.gz"):
        ssh_shell(
            host,
            f"tar -xzf {quote(remote_bin)} -C {quote(remote_dir)} && "
            f"find {quote(remote_dir)} -maxdepth 1 -type f -name 'yggterm*' "
            f"! -name '*headless*' ! -name '*mock-cli*' ! -name '*.sha256' -exec chmod +x {{}} +",
        )
        resolved = ssh_shell(
            host,
            f"find {quote(remote_dir)} -maxdepth 1 -type f -name 'yggterm*' "
            f"! -name '*headless*' ! -name '*mock-cli*' ! -name '*.tar.gz' ! -name '*.sha256' | head -n1",
        ).stdout.strip()
        if not resolved:
            raise RuntimeError(f"could not resolve staged macOS binary from remote archive {remote_bin}")
        return resolved, remote_app
    if lower_remote.endswith(".zip"):
        ssh_shell(
            host,
            f"rm -rf {quote(remote_dir)}/Yggterm.app && "
            f"(command -v ditto >/dev/null 2>&1 && ditto -x -k {quote(remote_bin)} {quote(remote_dir)} "
            f"|| unzip -oq {quote(remote_bin)} -d {quote(remote_dir)})",
        )
        app = ssh_shell(
            host,
            f"find {quote(remote_dir)} -maxdepth 2 -type d -name '*.app' | head -n1",
        ).stdout.strip()
        if app:
            resolved = ssh_shell(
                host,
                f"find {quote(app)}/Contents/MacOS -maxdepth 1 -type f | head -n1",
            ).stdout.strip()
            if not resolved:
                raise RuntimeError(f"staged macOS app bundle {app} did not expose an executable")
            ssh_shell(host, f"chmod +x {quote(resolved)}")
            return resolved, app
    return remote_bin, remote_app


def resolve_remote_launch_target(host: str, remote_dir: str, args: argparse.Namespace) -> tuple[str, str | None]:
    if args.remote_bin:
        return args.remote_bin, None
    artifact_value = str(args.artifact or "").strip()
    if artifact_value:
        artifact = Path(artifact_value).expanduser()
        if artifact.exists():
            return stage_artifact(host, artifact, remote_dir)
    proc = ssh_shell(
        host,
        "if [ -x \"$HOME/.local/bin/yggterm\" ]; then printf '%s\\n' \"$HOME/.local/bin/yggterm\"; "
        "elif [ -x \"$HOME/.yggterm/bin/yggterm\" ]; then printf '%s\\n' \"$HOME/.yggterm/bin/yggterm\"; "
        "else exit 1; fi",
        check=False,
    )
    path = proc.stdout.strip()
    if proc.returncode != 0 or not path:
        raise RuntimeError(f"could not resolve a remote macOS yggterm binary on {host}")
    return path, None


def remote_binary_version(host: str, remote_bin: str) -> str | None:
    proc = ssh_shell(host, f"{quote(remote_bin)} --version", check=False)
    text = proc.stdout.strip() or proc.stderr.strip()
    return text or None


def probe_remote_clients(host: str, remote_bin: str) -> dict | None:
    proc = ssh_shell(
        host,
        f"{quote(remote_bin)} server app clients --timeout-ms 5000",
        check=False,
    )
    text = proc.stdout.strip()
    if proc.returncode != 0 or not text:
        return None
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        return None


def remote_json_command(
    host: str,
    remote_bin: str,
    env: dict[str, str],
    args: list[str],
    *,
    expect_data: bool = False,
    check: bool = True,
) -> dict:
    exports = remote_env_exports(env)
    command_prefix = f"{exports}; " if exports else ""
    command = command_prefix + f"{quote(remote_bin)} " + " ".join(quote(arg) for arg in args)
    proc = ssh_shell(host, command, check=check)
    text = proc.stdout.strip()
    if not text:
        if check:
            raise RuntimeError(f"remote command produced no JSON on {host}: {args!r}")
        return {}
    payload = json.loads(text)
    if expect_data:
        data = payload.get("data")
        if not isinstance(data, dict):
            raise RuntimeError(f"expected data payload from {args!r} on {host}: {payload!r}")
        return data
    return payload


def choose_client_pid(payload: dict) -> int:
    clients = list(payload.get("clients") or [])
    if not clients:
        raise RuntimeError("no live Yggterm GUI clients are registered for app control")
    chosen = sorted(clients, key=lambda item: int(item.get("started_at_ms") or 0))[-1]
    pid = int(chosen.get("pid") or 0)
    if pid <= 0:
        raise RuntimeError(f"chosen client did not expose a pid: {chosen!r}")
    return pid


def wait_for_client_pid(
    host: str,
    remote_bin: str,
    env: dict[str, str],
    timeout_ms: int,
    *,
    wait_seconds: float = 45.0,
) -> int:
    deadline = time.time() + wait_seconds
    last_error = ""
    while time.time() < deadline:
        try:
            clients_payload = remote_json_command(
                host,
                remote_bin,
                env,
                ["server", "app", "clients", "--timeout-ms", str(timeout_ms)],
            )
            return choose_client_pid(clients_payload)
        except Exception as exc:  # noqa: BLE001
            last_error = str(exc)
            time.sleep(0.25)
    raise RuntimeError(
        f"remote macOS app never registered a controllable GUI client within {wait_seconds:.1f}s: {last_error}"
    )


def should_fallback_direct_launch(error: Exception) -> bool:
    text = str(error).lower()
    return "unsupported app control command: launch" in text


def spawn_direct_macos_app(
    host: str,
    remote_bin: str,
    env: dict[str, str],
    remote_log: str,
    remote_app: str | None = None,
) -> dict:
    exports = remote_env_exports(env)
    if remote_app:
        ssh_shell(
            host,
            f"mkdir -p {quote(Path(remote_log).parent.as_posix())}; "
            f"{exports}; open -na {quote(remote_app)} >/dev/null 2>&1",
        )
        return {
            "mode": "app_bundle_open",
            "app_bundle": remote_app,
            "stdout_log": remote_log,
        }
    proc = ssh_shell(
        host,
        f"mkdir -p {quote(Path(remote_log).parent.as_posix())}; "
        f"{exports}; nohup {quote(remote_bin)} > {quote(remote_log)} 2>&1 < /dev/null & echo $!",
    )
    spawn_pid = int((proc.stdout.strip().splitlines() or ["0"])[-1])
    return {
        "spawn_pid": spawn_pid,
        "stdout_log": remote_log,
    }


def wait_for_ready_state(
    host: str,
    remote_bin: str,
    env: dict[str, str],
    pid: int,
    timeout_ms: int,
    *,
    wait_seconds: float = 45.0,
    require_visible: bool = True,
) -> dict:
    deadline = time.time() + wait_seconds
    last_state = {}
    last_error = ""
    while time.time() < deadline:
        try:
            last_state = remote_json_command(
                host,
                remote_bin,
                env,
                ["server", "app", "state", "--pid", str(pid), "--timeout-ms", str(timeout_ms)],
                expect_data=True,
            )
            window = last_state.get("window") or {}
            shell = last_state.get("shell") or {}
            dom = last_state.get("dom") or {}
            visible = bool(window.get("visible"))
            if require_visible and not visible:
                raise RuntimeError("window not visible yet")
            if shell.get("needs_initial_server_sync"):
                raise RuntimeError("initial server sync still in progress")
            if shell.get("server_busy"):
                raise RuntimeError("server still busy")
            if dom.get("shell_root_count") != 1:
                raise RuntimeError(f"unexpected shell root count: {dom.get('shell_root_count')!r}")
            bad_notifications = problem_notifications(last_state)
            if bad_notifications:
                raise RuntimeError(f"bad daemon/socket notifications observed: {bad_notifications!r}")
            return last_state
        except Exception as exc:  # noqa: BLE001
            last_error = str(exc)
            time.sleep(0.25)
    raise RuntimeError(
        f"remote app state did not become ready for pid {pid} within {wait_seconds:.1f}s: "
        f"{last_error} state={last_state!r}"
    )


def capture_remote_screenshot(
    host: str,
    remote_bin: str,
    env: dict[str, str],
    pid: int,
    timeout_ms: int,
    remote_path: str,
    local_path: Path,
) -> dict:
    payload = remote_json_command(
        host,
        remote_bin,
        env,
        [
            "server",
            "app",
            "screenshot",
            "--pid",
            str(pid),
            remote_path,
            "--timeout-ms",
            str(timeout_ms),
        ],
    )
    if payload.get("error"):
        raise RuntimeError(str(payload.get("error")))
    resolved_output = str(payload.get("output_path") or remote_path).strip()
    if not resolved_output:
        raise RuntimeError("remote screenshot command returned no output path")
    scp_from(host, resolved_output, local_path)
    return payload


def remote_background_window(
    host: str,
    remote_bin: str,
    env: dict[str, str],
    pid: int,
    timeout_ms: int,
) -> dict:
    payload = remote_json_command(
        host,
        remote_bin,
        env,
        ["server", "app", "background", "--pid", str(pid), "--timeout-ms", str(timeout_ms)],
        check=False,
    )
    if not payload:
        raise RuntimeError(f"remote macOS background command returned no payload for pid {pid}")
    if payload.get("error"):
        raise RuntimeError(str(payload.get("error")))
    return payload


def cleanup_remote_dir(host: str, remote_dir: str) -> None:
    ssh_shell(host, f"rm -rf {quote(remote_dir)}", check=False)


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir or f"/tmp/yggterm-remote-macos-smoke-{args.host}")
    out_dir.mkdir(parents=True, exist_ok=True)
    proof_dir = out_dir / "proof"
    proof_dir.mkdir(parents=True, exist_ok=True)
    remote_dir = resolve_remote_dir(
        args.host,
        args.remote_dir or f"yggterm-remote-macos-{int(time.time())}",
    )
    remote_home = f"{remote_dir}/home"
    remote_log = f"{remote_dir}/client.log"
    local_summary_path = out_dir / "summary.json"
    remote_bin = ""
    remote_app = None
    remote_version = None
    remote_bin_error = None
    remote_clients_probe = None
    frontmost_app_name = None
    background_response = None
    background_error = None
    install_summary = None
    prelaunch_cleanup = cleanup_stale_remote_macos_clients(args.host)
    yggterm_processes_before_launch = remote_macos_yggterm_processes(args.host)

    session_info = macos_session_info(args.host)
    ssh_shell(args.host, f"mkdir -p {quote(remote_dir)} {quote(remote_home)}")
    try:
        remote_bin, remote_app = resolve_remote_launch_target(args.host, remote_dir, args)
        remote_bin, remote_app = normalize_staged_macos_payload(
            args.host, remote_dir, remote_bin, remote_app
        )
        if not remote_app:
            remote_app = stage_macos_app_bundle(args.host, remote_dir, remote_bin)
        if args.install:
            install_summary = install_staged_macos_app_bundle(args.host, remote_app)
            remote_app = str(install_summary.get("installed_app") or "").strip() or remote_app
            remote_bin = str(install_summary.get("installed_bin") or "").strip() or remote_bin
        try:
            remote_version = remote_binary_version(args.host, remote_bin)
        except Exception:
            remote_version = None
        try:
            remote_clients_probe = probe_remote_clients(args.host, remote_bin)
        except Exception:
            remote_clients_probe = None
    except Exception as exc:  # noqa: BLE001
        remote_bin_error = str(exc)
    if not macos_desktop_ready(session_info):
        summary = {
            "host": args.host,
            "session_info": session_info,
            "remote_dir": remote_dir,
            "remote_home": remote_home,
            "remote_bin": remote_bin or None,
            "remote_app": remote_app,
            "remote_version": remote_version,
            "remote_bin_error": remote_bin_error,
            "remote_clients_probe": remote_clients_probe,
            "frontmost_app_name": frontmost_app_name,
            "prelaunch_cleanup": prelaunch_cleanup,
            "install_summary": install_summary,
            "pid": None,
            "owned_launch": False,
            "launch": None,
            "prereq_failure": "no_active_aqua_session",
            "error": (
                "remote macOS host does not currently expose an interactive Aqua desktop session "
                f"for user {session_info.get('user')!r}: {session_info!r}"
            ),
            "local_proof_dir": str(proof_dir),
        }
        local_summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
        print(local_summary_path)
        return 1
    launch_env = {
        "YGGTERM_HOME": remote_home,
        "YGGTERM_ALLOW_MULTI_WINDOW": "1",
        "YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF": "1",
    }
    control_env = {} if args.attach_only else dict(launch_env)
    pid = 0
    owned_launch = False
    launch_payload = None
    direct_spawn_pid = 0
    try:
        cleanup_count = int(prelaunch_cleanup.get("count") or 0)
        remote_notify_macos(
            args.host,
            "Yggterm automated testing",
            (
                f"Automated testing is starting. Cleaned up {cleanup_count} stale automation window(s). "
                "Yggterm may briefly come to the front, then it will be moved behind other apps and closed."
            ),
        )
        if not remote_bin:
            raise RuntimeError(remote_bin_error or f"could not resolve remote macOS binary on {args.host}")
        if args.attach_only:
            clients_payload = remote_json_command(
                args.host,
                remote_bin,
                control_env,
                ["server", "app", "clients", "--timeout-ms", str(args.timeout_ms)],
            )
            pid = choose_client_pid(clients_payload)
        else:
            if remote_app:
                launch_payload = {
                    "mode": "app_bundle_open",
                    "spawn": spawn_direct_macos_app(
                        args.host,
                        remote_bin,
                        launch_env,
                        remote_log,
                        remote_app,
                    ),
                }
                owned_launch = True
                for _ in range(40):
                    frontmost_app_name = frontmost_macos_app_name(args.host)
                    if frontmost_app_name:
                        break
                    time.sleep(0.25)
                pid = wait_for_client_pid(
                    args.host,
                    remote_bin,
                    launch_env,
                    args.timeout_ms,
                )
            else:
                try:
                    launch_payload = remote_json_command(
                        args.host,
                        remote_bin,
                        launch_env,
                        [
                            "server",
                            "app",
                            "launch",
                            "--wait-visible",
                            "--allow-multi-window",
                            "--skip-active-exec-handoff",
                            "--timeout-ms",
                            str(args.timeout_ms),
                            "--log",
                            remote_log,
                        ],
                    )
                    pid = int(launch_payload.get("pid") or 0)
                    if pid <= 0:
                        raise RuntimeError(
                            f"background app launch did not return a pid on {args.host}: {launch_payload!r}"
                        )
                    owned_launch = True
                    frontmost_app_name = frontmost_macos_app_name(args.host)
                except Exception as exc:  # noqa: BLE001
                    if not should_fallback_direct_launch(exc):
                        raise
                    launch_payload = {
                        "mode": "direct_process_fallback",
                        "error": str(exc),
                        "spawn": spawn_direct_macos_app(
                            args.host,
                            remote_bin,
                            launch_env,
                            remote_log,
                            remote_app,
                        ),
                    }
                    owned_launch = True
                    direct_spawn_pid = int(
                        ((launch_payload.get("spawn") or {}).get("spawn_pid")) or 0
                    )
                    for _ in range(40):
                        frontmost_app_name = frontmost_macos_app_name(args.host)
                        if frontmost_app_name:
                            break
                        time.sleep(0.25)
                    pid = wait_for_client_pid(
                        args.host,
                        remote_bin,
                        launch_env,
                        args.timeout_ms,
                    )
        owned_clients_after_launch = remote_owned_macos_clients(args.host, remote_dir)

        state = wait_for_ready_state(
            args.host,
            remote_bin,
            control_env,
            pid,
            args.timeout_ms,
        )
        blur = assert_blur_expectation(state, args.expect_live_blur)
        rows_payload = remote_json_command(
            args.host,
            remote_bin,
            control_env,
            ["server", "app", "rows", "--pid", str(pid), "--timeout-ms", str(args.timeout_ms)],
        )
        rows = rows_payload.get("data") if isinstance(rows_payload.get("data"), dict) else rows_payload
        sidebar = assert_sidebar_rows_present(rows)

        state_path = proof_dir / "state.json"
        rows_path = proof_dir / "rows.json"
        screenshot_path = proof_dir / "window.png"
        summary_path = proof_dir / "summary.json"
        state_path.write_text(json.dumps(state, indent=2), encoding="utf-8")
        rows_path.write_text(json.dumps(rows, indent=2), encoding="utf-8")

        screenshot_response = None
        screenshot_error = None
        screenshot_quality = None
        try:
            screenshot_response = capture_remote_screenshot(
                args.host,
                remote_bin,
                control_env,
                pid,
                args.timeout_ms,
                f"{remote_dir}/window.png",
                screenshot_path,
            )
            screenshot_quality = assert_screenshot_file_usable(screenshot_path)
        except Exception as exc:  # noqa: BLE001
            screenshot_error = str(exc)
        if owned_launch and pid > 0:
            try:
                background_response = remote_background_window(
                    args.host,
                    remote_bin,
                    control_env,
                    pid,
                    args.timeout_ms,
                )
            except Exception as exc:  # noqa: BLE001
                background_error = str(exc)

        proof_summary = {
            "bin": remote_bin,
            "pid": pid,
            "window": state.get("window") or {},
            "client_instance": state.get("client_instance") or {},
            "active_session_path": state.get("active_session_path"),
            "active_view_mode": state.get("active_view_mode"),
            "notifications_count": int(((state.get("shell") or {}).get("notifications_count")) or 0),
            "visible_notifications_count": int(
                ((state.get("shell") or {}).get("visible_notifications_count")) or 0
            ),
            "problem_notifications": problem_notifications(state),
            "blur": blur,
            "sidebar": sidebar,
            "state_path": str(state_path),
            "rows_path": str(rows_path),
            "screenshot_path": str(screenshot_path) if screenshot_path.exists() else None,
            "screenshot_response": screenshot_response,
            "screenshot_backend": screenshot_backend(screenshot_response),
            "screenshot_backend_attempts": screenshot_backend_attempts(screenshot_response),
            "screenshot_quality": screenshot_quality,
            "screenshot_error": screenshot_error,
            "background_response": background_response,
            "background_error": background_error,
        }
        summary_path.write_text(json.dumps(proof_summary, indent=2), encoding="utf-8")
        if remote_app and frontmost_app_name != "Yggterm":
            raise RuntimeError(
                f"staged macOS app bundle still exposed the wrong frontmost app name: {frontmost_app_name!r}"
            )
        if screenshot_error:
            raise RuntimeError(f"macOS screenshot capture failed: {screenshot_error}")

        summary = {
            "host": args.host,
            "session_info": session_info,
            "remote_dir": remote_dir,
            "remote_home": remote_home,
            "remote_bin": remote_bin,
            "remote_app": remote_app,
            "remote_version": remote_version,
            "remote_clients_probe": remote_clients_probe,
            "frontmost_app_name": frontmost_app_name,
            "prelaunch_cleanup": prelaunch_cleanup,
            "install_summary": install_summary,
            "yggterm_processes_before_launch": yggterm_processes_before_launch,
            "owned_clients_after_launch": owned_clients_after_launch,
            "pid": pid,
            "owned_launch": owned_launch,
            "launch": launch_payload,
            "proof_summary": proof_summary,
            "local_proof_dir": str(proof_dir),
        }
        local_summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
        print(local_summary_path)
        return_code = 0
    except Exception as exc:  # noqa: BLE001
        summary = {
            "host": args.host,
            "session_info": session_info,
            "remote_dir": remote_dir,
            "remote_home": remote_home,
            "remote_bin": remote_bin or None,
            "remote_app": remote_app,
            "remote_version": remote_version,
            "remote_bin_error": remote_bin_error,
            "remote_clients_probe": remote_clients_probe,
            "frontmost_app_name": frontmost_app_name,
            "prelaunch_cleanup": prelaunch_cleanup,
            "install_summary": install_summary,
            "yggterm_processes_before_launch": yggterm_processes_before_launch,
            "owned_clients_after_launch": remote_owned_macos_clients(args.host, remote_dir),
            "pid": pid or None,
            "owned_launch": owned_launch,
            "launch": launch_payload,
            "error": str(exc),
            "local_proof_dir": str(proof_dir),
        }
        local_summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
        print(local_summary_path)
        return_code = 1
    finally:
        if owned_launch and direct_spawn_pid > 0:
            ssh_shell(
                args.host,
                f"kill -TERM {direct_spawn_pid} >/dev/null 2>&1 || true",
                check=False,
            )
            time.sleep(0.5)
            ssh_shell(
                args.host,
                f"kill -0 {direct_spawn_pid} >/dev/null 2>&1 && kill -KILL {direct_spawn_pid} >/dev/null 2>&1 || true",
                check=False,
            )
        if pid > 0 and owned_launch:
            try:
                remote_json_command(
                    args.host,
                    remote_bin,
                    control_env,
                    ["server", "app", "close", "--pid", str(pid), "--timeout-ms", str(args.timeout_ms)],
                    check=False,
                )
            except Exception:
                pass
            ssh_shell(
                args.host,
                f"kill -TERM {pid} >/dev/null 2>&1 || true",
                check=False,
            )
            time.sleep(0.5)
            ssh_shell(
                args.host,
                f"kill -0 {pid} >/dev/null 2>&1 && kill -KILL {pid} >/dev/null 2>&1 || true",
                check=False,
            )
            try:
                remote_json_command(
                    args.host,
                    remote_bin,
                    control_env,
                    ["server", "shutdown"],
                    check=False,
                )
            except Exception:
                pass
        owned_clients_after_close = wait_for_remote_owned_macos_clients_gone(args.host, remote_dir)
        forced_cleanup_after_close = None
        cleanup_error = None
        if int(owned_clients_after_close.get("count") or 0) > 0:
            forced_cleanup_after_close = kill_remote_owned_macos_clients(args.host, remote_dir)
            owned_clients_after_close = wait_for_remote_owned_macos_clients_gone(args.host, remote_dir)
        if int(owned_clients_after_close.get("count") or 0) > 0:
            cleanup_error = (
                "harness-owned macOS Yggterm instances survived post-run cleanup: "
                f"{owned_clients_after_close!r}"
            )
        remote_notify_macos(
            args.host,
            "Yggterm automated testing",
            "Automated testing finished. The test window should now be closed.",
        )
        try:
            summary = json.loads(local_summary_path.read_text(encoding="utf-8"))
            summary["owned_clients_after_close"] = owned_clients_after_close
            summary["yggterm_processes_after_close"] = remote_macos_yggterm_processes(args.host)
            if forced_cleanup_after_close is not None:
                summary["forced_cleanup_after_close"] = forced_cleanup_after_close
            if cleanup_error is not None:
                summary["cleanup_error"] = cleanup_error
            local_summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
        except Exception:
            pass
        if cleanup_error is not None:
            return_code = 1
        if not args.keep_remote_dir:
            cleanup_remote_dir(args.host, remote_dir)
    return return_code


if __name__ == "__main__":
    raise SystemExit(main())
