#!/usr/bin/env python3
import argparse
import hashlib
import json
import re
import subprocess
import tempfile
import time
import uuid
from pathlib import Path, PureWindowsPath

from remote_linux_x11_smoke import scp_from, scp_to
from remote_windows_live_app import (
    extract_json_text,
    ps_literal,
    run_remote_powershell,
    strip_windows_ssh_noise,
)
from smoke_app_control_bootstrap import (
    assert_blur_expectation,
    problem_notifications,
    screenshot_backend,
    screenshot_backend_attempts,
)


ROOT = Path(__file__).resolve().parents[1]


def default_artifact() -> Path:
    candidates = [
        ROOT / "dist" / "yggterm-windows-x86_64.zip",
        ROOT / "dist" / "yggterm-windows-x86_64.exe",
    ]
    for candidate in candidates:
        if candidate.exists():
            return candidate
    return candidates[-1]

WINDOWS_HELPERS = r"""
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

function Ensure-RemoteRoot {
  param([string]$DirName)
  $root = Join-Path $HOME $DirName
  New-Item -ItemType Directory -Force -Path $root | Out-Null
  return $root
}

function Resolve-YggtermBinary {
  param([string]$Requested, [string]$RemoteDirName)
  if ($Requested -and (Test-Path $Requested)) {
    return (Resolve-Path $Requested).Path
  }

  $root = Join-Path $HOME $RemoteDirName
  $staged = Get-ChildItem -Path $root -Filter "yggterm*.exe" -File -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -notlike "yggterm-headless*" -and $_.Name -notlike "yggterm-mock-cli*" } |
    Sort-Object Name |
    Select-Object -Last 1
  if ($staged) {
    return $staged.FullName
  }

  $installRoot = Join-Path $env:LOCALAPPDATA "Yggterm"
  $installState = Join-Path $installRoot "install-state.json"
  if (Test-Path $installState) {
    try {
      $state = Get-Content $installState -Raw | ConvertFrom-Json
      if ($state.active_executable -and (Test-Path $state.active_executable)) {
        return (Resolve-Path $state.active_executable).Path
      }
    } catch {
    }
  }

  $versions = Join-Path $installRoot "versions"
  if (Test-Path $versions) {
    $candidate = Get-ChildItem $versions -Directory |
      Sort-Object Name |
      ForEach-Object { Join-Path $_.FullName "yggterm.exe" } |
      Where-Object { Test-Path $_ } |
      Select-Object -Last 1
    if ($candidate) {
      return (Resolve-Path $candidate).Path
    }
  }

  throw "could not resolve yggterm.exe from staged or installed locations"
}
"""

WINDOWS_INTERACTIVE_LAUNCHER_TEMPLATE = r"""
$ErrorActionPreference = "Stop"
$RemoteBin = {remote_bin}
$RemoteHome = {remote_home}
$MetadataPath = {metadata_path}

New-Item -ItemType Directory -Force -Path $RemoteHome, (Split-Path -Parent $MetadataPath) | Out-Null
$env:YGGTERM_HOME = $RemoteHome
$env:YGGTERM_ALLOW_MULTI_WINDOW = "1"
$env:YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF = "1"

$proc = Start-Process -FilePath $RemoteBin -WorkingDirectory (Split-Path -Parent $RemoteBin) -PassThru
$payload = @{{
  spawn_pid = $proc.Id
  session_id = $proc.SessionId
  responding = $proc.Responding
  main_window_handle = [int64]$proc.MainWindowHandle
  path = $proc.Path
}}
$payload | ConvertTo-Json -Compress | Set-Content -Encoding utf8 -Path $MetadataPath
"""


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Stage or attach to Yggterm on a remote Windows host and run a minimal app-control smoke."
    )
    parser.add_argument("--host", required=True)
    parser.add_argument("--artifact", default=str(default_artifact()))
    parser.add_argument("--remote-bin")
    parser.add_argument("--out-dir")
    parser.add_argument("--remote-dir-name")
    parser.add_argument("--timeout-ms", type=int, default=20000)
    parser.add_argument("--attach-only", action="store_true")
    parser.add_argument("--keep-remote-dir", action="store_true")
    parser.add_argument(
        "--expect-live-blur",
        choices=("ignore", "required", "forbidden"),
        default="ignore",
    )
    return parser.parse_args()


def run_remote_powershell_json(host: str, script: str) -> dict:
    proc = run_remote_powershell(host, script)
    try:
        return json.loads(extract_json_text(proc.stdout))
    except Exception as exc:  # noqa: BLE001
        details = strip_windows_ssh_noise(proc.stderr) or proc.stdout.strip() or str(exc)
        raise RuntimeError(details) from exc


def run_remote_cmd(host: str, command: str, *, check: bool = True) -> subprocess.CompletedProcess:
    proc = subprocess.run(
        ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", host, "cmd", "/c", command],
        text=True,
        capture_output=True,
    )
    cleaned_stderr = strip_windows_ssh_noise(proc.stderr)
    if check and proc.returncode != 0:
        raise RuntimeError(cleaned_stderr or proc.stdout.strip() or f"cmd failed on {host}: {command}")
    return subprocess.CompletedProcess(
        args=proc.args,
        returncode=proc.returncode,
        stdout=proc.stdout,
        stderr=cleaned_stderr,
    )


def resolve_remote_dir_name(args: argparse.Namespace) -> str:
    return args.remote_dir_name or f"yggterm-remote-windows-{int(time.time())}"


def windows_scp_path(path: str) -> str:
    return path.replace("\\", "/")


def ensure_remote_dir(host: str, remote_dir_name: str) -> str:
    script = (
        f"$RemoteDirName = {ps_literal(remote_dir_name)}\n"
        + WINDOWS_HELPERS
        + "\n"
        + "$remoteRoot = Ensure-RemoteRoot -DirName $RemoteDirName\n"
        + '@{ remote_root = $remoteRoot } | ConvertTo-Json -Compress' + "\n"
    )
    payload = run_remote_powershell_json(host, script)
    remote_root = str(payload.get("remote_root") or "").strip()
    if not remote_root:
        raise RuntimeError(f"could not resolve remote root on {host}")
    return remote_root


def stage_artifact(host: str, artifact: Path, remote_root: str) -> None:
    remote_artifact = str(PureWindowsPath(remote_root) / artifact.name)
    lower_name = artifact.name.lower()
    scp_to(host, artifact, windows_scp_path(remote_artifact))
    if lower_name.endswith(".tar.gz"):
        script = (
            f"$RemoteRoot = {ps_literal(remote_root)}\n"
            + WINDOWS_HELPERS
            + "\n"
            + f"tar -xzf (Join-Path $RemoteRoot {ps_literal(artifact.name)}) -C $RemoteRoot\n"
        )
        run_remote_powershell(host, script)
        return
    if lower_name.endswith(".zip"):
        script = (
            f"$RemoteRoot = {ps_literal(remote_root)}\n"
            + WINDOWS_HELPERS
            + "\n"
            + f"Expand-Archive -Path (Join-Path $RemoteRoot {ps_literal(artifact.name)}) "
            + "-DestinationPath $RemoteRoot -Force\n"
        )
        run_remote_powershell(host, script)


def discover_windows_support_files(artifact: Path) -> list[Path]:
    support_paths: list[Path] = []
    seen: set[Path] = set()

    def add_candidate(path: Path) -> None:
        resolved = path.expanduser().resolve()
        if resolved.exists() and resolved.is_file() and resolved not in seen:
            seen.add(resolved)
            support_paths.append(resolved)

    adjacent = artifact.with_name("WebView2Loader.dll")
    if adjacent.exists():
        add_candidate(adjacent)

    for path in sorted(
        ROOT.glob("target/**/build/webview2-com-sys*/out/x64/WebView2Loader.dll"),
        key=lambda candidate: candidate.stat().st_mtime,
        reverse=True,
    ):
        add_candidate(path)
        break

    cargo_registry = Path.home() / ".cargo" / "registry" / "src"
    if cargo_registry.exists():
        matches = sorted(
            cargo_registry.glob("*/webview2-com-sys-*/x64/WebView2Loader.dll"),
            key=lambda candidate: candidate.stat().st_mtime,
            reverse=True,
        )
        for path in matches[:1]:
            add_candidate(path)

    return support_paths


def stage_support_files(host: str, support_files: list[Path], remote_root: str) -> list[str]:
    staged: list[str] = []
    seen_remote_paths: set[str] = set()
    for path in support_files:
        remote_path = str(PureWindowsPath(remote_root) / path.name)
        if remote_path in seen_remote_paths:
            continue
        seen_remote_paths.add(remote_path)
        scp_to(host, path, windows_scp_path(remote_path))
        staged.append(remote_path)
    return staged


def discover_windows_companion_binaries(artifact: Path) -> list[Path]:
    companions: list[Path] = []
    seen: set[Path] = set()

    def add_candidate(path: Path) -> None:
        resolved = path.expanduser().resolve()
        if resolved.exists() and resolved.is_file() and resolved not in seen:
            seen.add(resolved)
            companions.append(resolved)

    name = artifact.name
    if name.startswith("yggterm-windows-") and name.endswith(".exe"):
        suffix = name[len("yggterm-windows-") :]
        add_candidate(artifact.with_name(f"yggterm-headless-windows-{suffix}"))
        add_candidate(artifact.with_name(f"yggterm-mock-cli-windows-{suffix}"))
    elif name == "yggterm.exe":
        add_candidate(artifact.with_name("yggterm-headless.exe"))
        add_candidate(artifact.with_name("yggterm-mock-cli.exe"))

    return companions


def stage_text_file(host: str, contents: str, remote_path: str) -> None:
    temp_path = None
    try:
        with tempfile.NamedTemporaryFile("w", delete=False, encoding="utf-8", newline="\n") as handle:
            handle.write(contents)
            temp_path = Path(handle.name)
        scp_to(host, temp_path, windows_scp_path(remote_path))
    finally:
        if temp_path is not None:
            temp_path.unlink(missing_ok=True)


def render_windows_interactive_launcher(
    remote_bin: str,
    remote_home: str,
    remote_metadata: str,
) -> str:
    return WINDOWS_INTERACTIVE_LAUNCHER_TEMPLATE.format(
        remote_bin=ps_literal(remote_bin),
        remote_home=ps_literal(remote_home),
        metadata_path=ps_literal(remote_metadata),
    )


def resolve_remote_bin(host: str, remote_dir_name: str, requested_bin: str | None) -> str:
    script = (
        f"$RemoteDirName = {ps_literal(remote_dir_name)}\n"
        + f"$RequestedBin = {ps_literal(requested_bin or '')}\n"
        + WINDOWS_HELPERS
        + "\n"
        + "$bin = Resolve-YggtermBinary -Requested $RequestedBin -RemoteDirName $RemoteDirName\n"
        + '@{ bin = $bin } | ConvertTo-Json -Compress' + "\n"
    )
    payload = run_remote_powershell_json(host, script)
    remote_bin = str(payload.get("bin") or "").strip()
    if not remote_bin:
        raise RuntimeError(f"could not resolve a remote Windows yggterm binary on {host}")
    return remote_bin


def remote_app_json_command(
    host: str,
    remote_bin: str,
    remote_home: str,
    args: list[str],
    *,
    expect_data: bool = False,
) -> dict:
    rendered_args = ", ".join(ps_literal(arg) for arg in args)
    script = (
        f"$RemoteBin = {ps_literal(remote_bin)}\n"
        + f"$RemoteHome = {ps_literal(remote_home)}\n"
        + "$ErrorActionPreference = 'Stop'\n"
        + "$env:YGGTERM_HOME = $RemoteHome\n"
        + "$env:YGGTERM_ALLOW_MULTI_WINDOW = '1'\n"
        + "$env:YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF = '1'\n"
        + f"$CommandArgs = @({rendered_args})\n"
        + "& $RemoteBin @CommandArgs\n"
    )
    payload = run_remote_powershell_json(host, script)
    if expect_data:
        data = payload.get("data")
        if not isinstance(data, dict):
            raise RuntimeError(f"expected data payload from {args!r} on {host}: {payload!r}")
        return data
    return payload


def windows_session_info(host: str) -> dict:
    script = r"""
$ErrorActionPreference = "Stop"; $queryOutput = (& query user 2>$null | Out-String); $users = @(); foreach ($line in ($queryOutput -split "`r?`n")) { $trimmed = $line.Trim(); if (-not $trimmed -or $trimmed.StartsWith("USERNAME")) { continue }; if ($trimmed -match '^\>?\s*(\S+)\s+(\S*)\s+(\d+)\s+(\S+)') { $users += @{ username = $Matches[1]; session_name = $Matches[2]; session_id = [int]$Matches[3]; state = $Matches[4] } } }; $explorers = @(Get-Process explorer -ErrorAction SilentlyContinue | Select-Object Id, SessionId, MainWindowHandle, Path); Write-Output (@{ users = $users; explorers = $explorers } | ConvertTo-Json -Depth 8 -Compress)
"""
    return run_remote_powershell_json(host, script)


def dismiss_windows_error_dialogs(host: str) -> dict:
    titles = [
        "yggterm-windows-x86_64.exe - System Error",
        "yggterm.exe - System Error",
        "System Error",
    ]
    dismissed: list[str] = []
    attempts: list[dict] = []
    for title in titles:
        proc = run_remote_cmd(host, f'taskkill /FI "WINDOWTITLE eq {title}" /F', check=False)
        attempts.append(
            {
                "title": title,
                "returncode": proc.returncode,
                "stdout": proc.stdout.strip() or None,
                "stderr": proc.stderr.strip() or None,
            }
        )
        if proc.returncode == 0 and "SUCCESS:" in (proc.stdout or ""):
            dismissed.append(title)
    return {
        "activated_titles": dismissed,
        "dismissed": len(dismissed),
        "attempts": attempts,
    }


def collect_recent_windows_error_events(host: str, *, minutes: int = 20) -> list[dict]:
    script = rf"""
$cutoff = (Get-Date).AddMinutes(-{minutes})
$events = Get-WinEvent -LogName Application -MaxEvents 80 -ErrorAction SilentlyContinue |
  Where-Object {{
    $_.TimeCreated -ge $cutoff -and $_.Message -match 'yggterm|WebView2|WebView2Loader|entry point|System Error|dll'
  }} |
  Select-Object -First 16 TimeCreated, Id, ProviderName, Message
Write-Output ($events | ConvertTo-Json -Depth 6 -Compress)
"""
    payload = run_remote_powershell_json(host, script)
    if isinstance(payload, list):
        return payload
    if isinstance(payload, dict):
        return [payload]
    return []


def parse_wmic_list_payload(text: str) -> list[dict[str, str]]:
    records: list[dict[str, str]] = []
    current: dict[str, str] = {}
    for raw_line in text.splitlines():
        line = raw_line.strip()
        if not line or "=" not in line:
            continue
        key, value = line.split("=", 1)
        normalized_key = key.strip()
        current[normalized_key] = value.strip().strip('"')
        if normalized_key.lower() == "processid":
            records.append(current)
            current = {}
    if current:
        records.append(current)
    return records


def extract_windows_harness_root(text: str | None) -> str | None:
    if not text:
        return None
    match = re.search(
        r"([A-Za-z]:\\(?:[^\\/\r\n\"]+\\)*yggterm-remote-windows-[^\\/\s\"]+)",
        text,
    )
    if not match:
        return None
    root = match.group(1).rstrip("\\/")
    return root or None


def list_windows_yggterm_processes(host: str) -> list[dict]:
    proc = run_remote_cmd(
        host,
        'wmic process where "name like \'yggterm%\'" get ProcessId,CommandLine,ExecutablePath /format:list',
        check=False,
    )
    processes: list[dict] = []
    for record in parse_wmic_list_payload(proc.stdout):
        pid = int(record.get("ProcessId") or 0)
        if pid <= 0:
            continue
        command_line = str(record.get("CommandLine") or "").strip()
        executable_path = str(record.get("ExecutablePath") or "").strip()
        harness_root = extract_windows_harness_root(executable_path) or extract_windows_harness_root(
            command_line
        )
        processes.append(
            {
                "pid": pid,
                "command_line": command_line or None,
                "executable_path": executable_path or None,
                "harness_root": harness_root,
            }
        )
    return processes


def list_windows_harness_processes(host: str, remote_root: str | None = None) -> list[dict]:
    target_root = remote_root.lower() if remote_root else None
    owned = []
    for process in list_windows_yggterm_processes(host):
        harness_root = str(process.get("harness_root") or "")
        if not harness_root:
            continue
        if target_root and harness_root.lower() != target_root:
            continue
        owned.append(process)
    return owned


def kill_windows_processes(host: str, processes: list[dict]) -> list[dict]:
    killed: list[dict] = []
    for process in processes:
        pid = int(process.get("pid") or 0)
        if pid <= 0:
            continue
        proc = run_remote_cmd(host, f"taskkill /PID {pid} /F /T", check=False)
        killed.append(
            {
                **process,
                "taskkill_returncode": proc.returncode,
                "taskkill_stdout": proc.stdout.strip() or None,
                "taskkill_stderr": proc.stderr.strip() or None,
            }
        )
    return killed


def choose_active_windows_session(session_info: dict) -> dict | None:
    users = list(session_info.get("users") or [])
    active_users = [
        item
        for item in users
        if str(item.get("state") or "").lower() == "active" and int(item.get("session_id") or 0) > 0
    ]
    if active_users:
        return sorted(active_users, key=lambda item: int(item.get("session_id") or 0))[-1]
    return None


def try_activate_windows_console_session(host: str, session_info: dict) -> tuple[dict, dict | None, dict | None]:
    users = list(session_info.get("users") or [])
    explorers = {
        int(item.get("SessionId") or 0)
        for item in (session_info.get("explorers") or [])
        if int(item.get("SessionId") or 0) > 0
    }
    disconnected = [
        item
        for item in users
        if str(item.get("state") or "").lower() in {"disc", "disconnected"}
        and int(item.get("session_id") or 0) > 0
        and int(item.get("session_id") or 0) in explorers
    ]
    if len(disconnected) != 1:
        return session_info, choose_active_windows_session(session_info), None
    candidate = disconnected[0]
    session_id = int(candidate.get("session_id") or 0)
    if session_id <= 0:
        return session_info, choose_active_windows_session(session_info), None
    proc = run_remote_cmd(host, f"tscon {session_id} /dest:console", check=False)
    time.sleep(1.0)
    refreshed = windows_session_info(host)
    active = choose_active_windows_session(refreshed)
    if active and int(active.get("session_id") or 0) == session_id:
        return (
            refreshed,
            active,
            {
                "mode": "tscon_console_attach",
                "requested_session_id": session_id,
                "requested_user": candidate.get("username"),
                "stdout": proc.stdout.strip() or None,
                "stderr": proc.stderr.strip() or None,
            },
        )
    return session_info, choose_active_windows_session(session_info), None


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def remote_binary_version(host: str, remote_bin: str) -> str | None:
    script = (
        f"$RemoteBin = {ps_literal(remote_bin)}\n"
        + "$ErrorActionPreference = 'Stop'\n"
        + "$item = Get-Item -LiteralPath $RemoteBin -ErrorAction Stop\n"
        + "$version = ''\n"
        + "if ($item.VersionInfo -and $item.VersionInfo.FileVersion) {\n"
        + "  $version = $item.VersionInfo.FileVersion\n"
        + "}\n"
        + '@{ version = $version.Trim() } | ConvertTo-Json -Compress' + "\n"
    )
    payload = run_remote_powershell_json(host, script)
    version = str(payload.get("version") or "").strip()
    return version or None


def describe_windows_process(host: str, pid: int) -> dict | None:
    script = (
        f"$TargetPid = {pid}\n"
        + "$ErrorActionPreference = 'Stop'; "
        + "$proc = Get-Process -Id $TargetPid -ErrorAction SilentlyContinue | "
        + "Select-Object Id, SessionId, Responding, MainWindowHandle, MainWindowTitle, Path; "
        + "Write-Output (@{ process = $proc } | ConvertTo-Json -Depth 8 -Compress)\n"
    )
    payload = run_remote_powershell_json(host, script)
    process = payload.get("process")
    return process if isinstance(process, dict) else None


def launch_interactive_windows_app(
    host: str,
    remote_bin: str,
    remote_home: str,
    remote_root: str,
    active_session: dict,
) -> dict:
    active_user = str(active_session.get("username") or "").strip()
    active_session_id = int(active_session.get("session_id") or 0)
    if not active_user or active_session_id <= 0:
        raise RuntimeError(f"no active interactive Windows session is available: {active_session!r}")
    task_name = f"YggtermSmoke-{uuid.uuid4().hex[:10]}"
    remote_root_path = PureWindowsPath(remote_root)
    remote_launcher = str(remote_root_path / f"{task_name}.ps1")
    remote_metadata = str(remote_root_path / f"{task_name}.json")
    stage_text_file(
        host,
        render_windows_interactive_launcher(remote_bin, remote_home, remote_metadata),
        remote_launcher,
    )
    run_remote_cmd(host, f'del /f /q "{remote_metadata}"', check=False)
    create_command = (
        f'schtasks /Create /TN {task_name} /SC ONCE /ST 23:59 '
        f'/TR "powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File {remote_launcher}" '
        f"/F /RL HIGHEST /RU {active_user} /IT"
    )
    run_remote_cmd(host, create_command)
    run_remote_cmd(host, f"schtasks /Run /TN {task_name}")
    metadata_text = ""
    for _ in range(80):
        proc = run_remote_cmd(host, f'type "{remote_metadata}"', check=False)
        if proc.returncode == 0 and proc.stdout.strip():
            metadata_text = proc.stdout.lstrip("\ufeff").strip()
            break
        time.sleep(0.25)
    task_query = run_remote_cmd(host, f"schtasks /Query /TN {task_name} /FO LIST /V", check=False)
    run_remote_cmd(host, f"schtasks /Delete /TN {task_name} /F", check=False)
    if not metadata_text:
        raise RuntimeError(
            "interactive Windows launch did not produce metadata: "
            f"task_query={task_query.stdout.strip() or task_query.stderr.strip()!r}"
        )
    metadata = json.loads(metadata_text)
    if int(metadata.get("session_id") or 0) != active_session_id:
        raise RuntimeError(
            "interactive Windows launch landed in the wrong session: "
            f"expected {active_session_id}, got {metadata.get('session_id')!r}"
        )
    return {
        "task_name": task_name,
        "launcher_path": remote_launcher,
        "metadata_path": remote_metadata,
        "metadata": metadata,
        "task_query": task_query.stdout.strip() or None,
    }


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
    remote_home: str,
    timeout_ms: int,
    *,
    wait_seconds: float = 45.0,
) -> int:
    deadline = time.time() + wait_seconds
    last_error = ""
    while time.time() < deadline:
        try:
            clients_payload = remote_app_json_command(
                host,
                remote_bin,
                remote_home,
                ["server", "app", "clients", "--timeout-ms", str(timeout_ms)],
            )
            return choose_client_pid(clients_payload)
        except Exception as exc:  # noqa: BLE001
            last_error = str(exc)
            time.sleep(0.25)
    raise RuntimeError(
        f"remote Windows app never registered a controllable GUI client within {wait_seconds:.1f}s: {last_error}"
    )


def should_fallback_direct_launch(error: Exception) -> bool:
    text = str(error).lower()
    return "unsupported app control command: launch" in text


def spawn_direct_windows_app(
    host: str,
    remote_bin: str,
    remote_home: str,
    remote_log: str,
) -> dict:
    script = (
        f"$RemoteBin = {ps_literal(remote_bin)}\n"
        + f"$RemoteHome = {ps_literal(remote_home)}\n"
        + f"$RemoteLog = {ps_literal(remote_log)}\n"
        + "$ErrorActionPreference = 'Stop'\n"
        + "New-Item -ItemType Directory -Force -Path $RemoteHome, (Split-Path -Parent $RemoteLog) | Out-Null\n"
        + "$env:YGGTERM_HOME = $RemoteHome\n"
        + "$env:YGGTERM_ALLOW_MULTI_WINDOW = '1'\n"
        + "$env:YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF = '1'\n"
        + "$stderrLog = \"$RemoteLog.stderr\"\n"
        + "$proc = Start-Process -FilePath $RemoteBin -WorkingDirectory (Split-Path -Parent $RemoteBin) -PassThru\n"
        + '@{ spawn_pid = $proc.Id; stdout_log = $RemoteLog; stderr_log = $stderrLog } | ConvertTo-Json -Compress'
        + "\n"
    )
    return run_remote_powershell_json(host, script)


def stop_direct_windows_process(host: str, pid: int) -> None:
    script = (
        f"$TargetPid = {pid}\n"
        + "if ($TargetPid -gt 0) { Stop-Process -Id $TargetPid -Force -ErrorAction SilentlyContinue }\n"
    )
    try:
        run_remote_powershell(host, script)
    except RuntimeError:
        pass


def wait_for_ready_state(
    host: str,
    remote_bin: str,
    remote_home: str,
    app_pid: int,
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
            last_state = remote_app_json_command(
                host,
                remote_bin,
                remote_home,
                ["server", "app", "state", "--pid", str(app_pid), "--timeout-ms", str(timeout_ms)],
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
        f"remote app state did not become ready for pid {app_pid} within {wait_seconds:.1f}s: "
        f"{last_error} state={last_state!r}"
    )


def capture_remote_screenshot(
    host: str,
    remote_bin: str,
    remote_home: str,
    app_pid: int,
    timeout_ms: int,
    remote_path: str,
    remote_scp_path: str,
    local_path: Path,
) -> dict:
    payload = remote_app_json_command(
        host,
        remote_bin,
        remote_home,
        [
            "server",
            "app",
            "screenshot",
            "--pid",
            str(app_pid),
            remote_path,
            "--timeout-ms",
            str(timeout_ms),
        ],
    )
    if payload.get("error"):
        raise RuntimeError(f"remote screenshot command failed: {payload['error']}")
    scp_from(host, remote_scp_path, local_path)
    return payload


def assert_windows_screenshot_quality(payload: dict | None) -> dict:
    backend = screenshot_backend(payload)
    attempts = screenshot_backend_attempts(payload)
    if not backend:
        raise RuntimeError(f"remote screenshot response did not expose a capture backend: {payload!r}")
    if backend == "windows_screen_copy":
        raise RuntimeError(
            "Windows screenshot fell back to desktop screen copy, which is not release-grade "
            f"for proof capture because other windows/dialogs can pollute the image: attempts={attempts!r}"
        )
    return {
        "backend": backend,
        "attempts": attempts,
    }


def cleanup_remote_dir(host: str, remote_root: str) -> None:
    script = (
        f"$RemoteRoot = {ps_literal(remote_root)}\n"
        + "if (Test-Path $RemoteRoot) { Remove-Item -Recurse -Force $RemoteRoot }\n"
    )
    try:
        run_remote_powershell(host, script)
    except RuntimeError:
        pass


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir or f"/tmp/yggterm-remote-windows-smoke-{args.host}")
    out_dir.mkdir(parents=True, exist_ok=True)
    proof_dir = out_dir / "proof"
    proof_dir.mkdir(parents=True, exist_ok=True)
    session_info = windows_session_info(args.host)
    active_session = choose_active_windows_session(session_info)
    session_recovery = None
    if active_session is None:
        session_info, active_session, session_recovery = try_activate_windows_console_session(
            args.host,
            session_info,
        )
    remote_dir_name = resolve_remote_dir_name(args)
    remote_root = ensure_remote_dir(args.host, remote_dir_name)
    local_summary_path = out_dir / "summary.json"
    remote_bin = ""
    remote_control_bin = ""
    remote_home = ""
    remote_log = ""
    remote_screenshot_path = ""
    remote_screenshot_scp = windows_scp_path(str(PureWindowsPath(remote_root) / "window.png"))
    app_pid = 0
    owned_launch = False
    launch_payload = None
    direct_spawn_pid = 0
    local_artifact_sha256 = None
    remote_version = None
    staged_support_files: list[str] = []
    staged_companion_binaries: list[str] = []
    dismissed_error_dialogs_prelaunch = None
    dismissed_error_dialogs_prescreenshot = None
    owned_processes_before_launch: list[dict] = []
    owned_processes_after_prelaunch_cleanup: list[dict] = []
    killed_processes_prelaunch: list[dict] = []
    owned_processes_before_close: list[dict] = []
    owned_processes_after_close: list[dict] = []
    killed_processes_postclose: list[dict] = []
    postclose_error: str | None = None
    windows_error_events_before_launch: list[dict] = []
    windows_error_events_after_launch: list[dict] = []
    new_windows_error_events: list[dict] = []

    try:
        try:
            windows_error_events_before_launch = collect_recent_windows_error_events(args.host)
        except Exception:
            windows_error_events_before_launch = []
        owned_processes_before_launch = list_windows_harness_processes(args.host)
        if owned_processes_before_launch:
            killed_processes_prelaunch = kill_windows_processes(
                args.host,
                owned_processes_before_launch,
            )
            time.sleep(0.75)
        owned_processes_after_prelaunch_cleanup = list_windows_harness_processes(args.host)
        if owned_processes_after_prelaunch_cleanup:
            raise RuntimeError(
                "stale harness-owned Windows Yggterm processes remained after cleanup: "
                f"{owned_processes_after_prelaunch_cleanup!r}"
            )
        try:
            dismissed_error_dialogs_prelaunch = dismiss_windows_error_dialogs(args.host)
        except Exception as exc:  # noqa: BLE001
            dismissed_error_dialogs_prelaunch = {"error": str(exc)}
        artifact_value = str(args.artifact or "").strip()
        if artifact_value:
            artifact = Path(artifact_value).expanduser()
            if artifact.exists():
                local_artifact_sha256 = sha256_file(artifact)
                stage_artifact(args.host, artifact, remote_root)
                staged_support_files = stage_support_files(
                    args.host,
                    discover_windows_support_files(artifact),
                    remote_root,
                )
                staged_companion_binaries = stage_support_files(
                    args.host,
                    discover_windows_companion_binaries(artifact),
                    remote_root,
                )

        remote_bin = resolve_remote_bin(args.host, remote_dir_name, args.remote_bin)
        remote_control_bin = next(
            (
                path
                for path in staged_companion_binaries
                if path.lower().endswith("yggterm-headless-windows-x86_64.exe")
                or path.lower().endswith("yggterm-headless.exe")
            ),
            remote_bin,
        )
        try:
            remote_version = remote_binary_version(args.host, remote_bin)
        except Exception:
            remote_version = None
        remote_root_path = PureWindowsPath(remote_root)
        remote_home = str(remote_root_path / "home")
        remote_log = str(remote_root_path / "client.log")
        remote_screenshot_path = str(remote_root_path / "window.png")

        if args.attach_only:
            clients_payload = remote_app_json_command(
                args.host,
                remote_bin,
                remote_home,
                ["server", "app", "clients", "--timeout-ms", str(args.timeout_ms)],
            )
            app_pid = choose_client_pid(clients_payload)
        else:
            if active_session is None:
                raise RuntimeError(
                    f"no active interactive Windows desktop session found on {args.host}: {session_info!r}"
                )
            interactive_launch = launch_interactive_windows_app(
                args.host,
                remote_bin,
                remote_home,
                remote_root,
                active_session,
            )
            launch_payload = {
                "mode": "scheduled_task_interactive",
                "session": active_session,
                "spawn": interactive_launch,
            }
            direct_spawn_pid = int(
                (((launch_payload.get("spawn") or {}).get("metadata") or {}).get("spawn_pid")) or 0
            )
            app_pid = direct_spawn_pid
            if app_pid <= 0:
                app_pid = wait_for_client_pid(
                    args.host,
                    remote_bin,
                    remote_home,
                    args.timeout_ms,
                )
            owned_launch = True

        state = wait_for_ready_state(
            args.host,
            remote_control_bin,
            remote_home,
            app_pid,
            args.timeout_ms,
        )
        try:
            windows_error_events_after_launch = collect_recent_windows_error_events(args.host)
        except Exception:
            windows_error_events_after_launch = []
        baseline_event_keys = {
            (
                str(event.get("TimeCreated") or ""),
                int(event.get("Id") or 0),
                str(event.get("ProviderName") or ""),
                str(event.get("Message") or ""),
            )
            for event in windows_error_events_before_launch
        }
        new_windows_error_events = [
            event
            for event in windows_error_events_after_launch
            if (
                str(event.get("TimeCreated") or ""),
                int(event.get("Id") or 0),
                str(event.get("ProviderName") or ""),
                str(event.get("Message") or ""),
            )
            not in baseline_event_keys
        ]
        app_process = describe_windows_process(args.host, app_pid)
        blur = assert_blur_expectation(state, args.expect_live_blur)
        rows_payload = remote_app_json_command(
            args.host,
            remote_control_bin,
            remote_home,
            ["server", "app", "rows", "--pid", str(app_pid), "--timeout-ms", str(args.timeout_ms)],
        )
        rows = rows_payload.get("data") if isinstance(rows_payload.get("data"), dict) else rows_payload

        state_path = proof_dir / "state.json"
        rows_path = proof_dir / "rows.json"
        screenshot_path = proof_dir / "window.png"
        summary_path = proof_dir / "summary.json"
        state_path.write_text(json.dumps(state, indent=2), encoding="utf-8")
        rows_path.write_text(json.dumps(rows, indent=2), encoding="utf-8")

        screenshot_response = None
        screenshot_error = None
        try:
            try:
                dismissed_error_dialogs_prescreenshot = dismiss_windows_error_dialogs(args.host)
            except Exception as exc:  # noqa: BLE001
                dismissed_error_dialogs_prescreenshot = {"error": str(exc)}
            if (dismissed_error_dialogs_prescreenshot or {}).get("dismissed", 0) > 0:
                raise RuntimeError(
                    "Windows run surfaced a system-error dialog before proof capture: "
                    f"{dismissed_error_dialogs_prescreenshot!r}"
                )
            if new_windows_error_events:
                raise RuntimeError(
                    "Windows run emitted new application error events: "
                    f"{new_windows_error_events!r}"
                )
            screenshot_response = capture_remote_screenshot(
                args.host,
                remote_control_bin,
                remote_home,
                app_pid,
                args.timeout_ms,
                remote_screenshot_path,
                remote_screenshot_scp,
                screenshot_path,
            )
            screenshot_quality = assert_windows_screenshot_quality(screenshot_response)
        except Exception as exc:  # noqa: BLE001
            screenshot_error = str(exc)
            screenshot_quality = None

        proof_summary = {
            "bin": remote_bin,
            "pid": app_pid,
            "app_process": app_process,
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
            "state_path": str(state_path),
            "rows_path": str(rows_path),
            "screenshot_path": str(screenshot_path) if screenshot_path.exists() else None,
            "screenshot_response": screenshot_response,
            "screenshot_backend": (screenshot_quality or {}).get("backend"),
            "screenshot_backend_attempts": (screenshot_quality or {}).get("attempts") or [],
            "screenshot_error": screenshot_error,
        }
        summary_path.write_text(json.dumps(proof_summary, indent=2), encoding="utf-8")

        summary = {
            "host": args.host,
            "session_info": session_info,
            "active_session": active_session,
            "session_recovery": session_recovery,
            "remote_dir_name": remote_dir_name,
            "remote_root": remote_root,
            "remote_home": remote_home,
            "remote_bin": remote_bin,
            "remote_control_bin": remote_control_bin,
            "remote_version": remote_version,
            "local_artifact_sha256": local_artifact_sha256,
            "staged_support_files": staged_support_files,
            "staged_companion_binaries": staged_companion_binaries,
            "owned_processes_before_launch": owned_processes_before_launch,
            "killed_processes_prelaunch": killed_processes_prelaunch,
            "owned_processes_after_prelaunch_cleanup": owned_processes_after_prelaunch_cleanup,
            "dismissed_error_dialogs_prelaunch": dismissed_error_dialogs_prelaunch,
            "dismissed_error_dialogs_prescreenshot": dismissed_error_dialogs_prescreenshot,
            "windows_error_events_before_launch": windows_error_events_before_launch,
            "windows_error_events_after_launch": windows_error_events_after_launch,
            "new_windows_error_events": new_windows_error_events,
            "pid": app_pid,
            "owned_launch": owned_launch,
            "launch": launch_payload,
            "owned_processes_before_close": owned_processes_before_close,
            "killed_processes_postclose": killed_processes_postclose,
            "owned_processes_after_close": owned_processes_after_close,
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
            "active_session": active_session,
            "session_recovery": session_recovery,
            "remote_dir_name": remote_dir_name,
            "remote_root": remote_root,
            "remote_home": remote_home or None,
            "remote_bin": remote_bin or None,
            "remote_control_bin": remote_control_bin or None,
            "remote_version": remote_version,
            "local_artifact_sha256": local_artifact_sha256,
            "staged_support_files": staged_support_files,
            "staged_companion_binaries": staged_companion_binaries,
            "owned_processes_before_launch": owned_processes_before_launch,
            "killed_processes_prelaunch": killed_processes_prelaunch,
            "owned_processes_after_prelaunch_cleanup": owned_processes_after_prelaunch_cleanup,
            "dismissed_error_dialogs_prelaunch": dismissed_error_dialogs_prelaunch,
            "dismissed_error_dialogs_prescreenshot": dismissed_error_dialogs_prescreenshot,
            "windows_error_events_before_launch": windows_error_events_before_launch,
            "windows_error_events_after_launch": windows_error_events_after_launch,
            "new_windows_error_events": new_windows_error_events,
            "pid": app_pid or None,
            "app_process": describe_windows_process(args.host, app_pid) if app_pid > 0 else None,
            "owned_launch": owned_launch,
            "launch": launch_payload,
            "owned_processes_before_close": owned_processes_before_close,
            "killed_processes_postclose": killed_processes_postclose,
            "owned_processes_after_close": owned_processes_after_close,
            "error": str(exc),
            "local_proof_dir": str(proof_dir),
        }
        local_summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
        print(local_summary_path)
        return_code = 1
    finally:
        if app_pid > 0 and owned_launch:
            owned_processes_before_close = list_windows_harness_processes(args.host)
            try:
                remote_app_json_command(
                    args.host,
                    remote_control_bin or remote_bin,
                    remote_home,
                    ["server", "app", "close", "--pid", str(app_pid), "--timeout-ms", str(args.timeout_ms)],
                )
            except Exception:
                if direct_spawn_pid > 0:
                    stop_direct_windows_process(args.host, direct_spawn_pid)
            try:
                remote_app_json_command(
                    args.host,
                    remote_control_bin or remote_bin,
                    remote_home,
                    ["server", "shutdown"],
                )
            except Exception:
                pass
            time.sleep(0.75)
            owned_processes_after_close = list_windows_harness_processes(args.host)
            current_root_processes = list_windows_harness_processes(args.host, remote_root)
            if current_root_processes:
                killed_processes_postclose = kill_windows_processes(args.host, current_root_processes)
                time.sleep(0.5)
                owned_processes_after_close = list_windows_harness_processes(args.host)
            if owned_processes_after_close:
                postclose_error = (
                    "harness-owned Windows Yggterm processes remained after close: "
                    f"{owned_processes_after_close!r}"
                )
                if return_code == 0:
                    return_code = 1
            if local_summary_path.exists():
                try:
                    summary = json.loads(local_summary_path.read_text(encoding="utf-8"))
                    summary["owned_processes_before_close"] = owned_processes_before_close
                    summary["killed_processes_postclose"] = killed_processes_postclose
                    summary["owned_processes_after_close"] = owned_processes_after_close
                    if postclose_error:
                        summary["postclose_error"] = postclose_error
                    local_summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
                except Exception:
                    pass
        if not args.keep_remote_dir:
            cleanup_remote_dir(args.host, remote_root)
    return return_code


if __name__ == "__main__":
    raise SystemExit(main())
