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
    configure_remote_transport,
    extract_json_text,
    ps_literal,
    run_remote_powershell,
    run_remote_powershell_best_effort,
    ssh_base,
    strip_windows_ssh_noise,
)
from smoke_app_control_bootstrap import (
    active_terminal_host,
    assert_active_terminal_host_ready,
    assert_sidebar_rows_present,
    assert_blur_expectation,
    assert_screenshot_file_usable,
    assert_titlebar_utility_buttons_inline,
    problem_notifications,
    screenshot_backend,
    screenshot_backend_attempts,
)


ROOT = Path(__file__).resolve().parents[1]


def workspace_version() -> str | None:
    cargo_toml = ROOT / "Cargo.toml"
    try:
        text = cargo_toml.read_text(encoding="utf-8")
    except OSError:
        return None
    match = re.search(
        r'^\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"',
        text,
        re.MULTILINE,
    )
    if not match:
        return None
    return str(match.group(1)).strip() or None


def artifact_under_dist(path: Path) -> bool:
    try:
        path.resolve().relative_to((ROOT / "dist").resolve())
        return True
    except ValueError:
        return False


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
    parser.add_argument("--proxy-jump")
    parser.add_argument("--ssh-port", type=int)
    parser.add_argument("--artifact", default=str(default_artifact()))
    parser.add_argument("--remote-bin")
    parser.add_argument("--out-dir")
    parser.add_argument("--remote-dir-name")
    parser.add_argument("--timeout-ms", type=int, default=20000)
    parser.add_argument("--attach-only", action="store_true")
    parser.add_argument("--install", action="store_true")
    parser.add_argument("--keep-remote-dir", action="store_true")
    parser.add_argument(
        "--expect-live-blur",
        choices=("ignore", "required", "forbidden"),
        default="ignore",
    )
    return parser.parse_args()


def run_remote_powershell_json(host: str, script: str) -> dict:
    proc = run_remote_powershell_best_effort(host, script)
    try:
        return json.loads(extract_json_text(proc.stdout))
    except Exception as exc:  # noqa: BLE001
        details = strip_windows_ssh_noise(proc.stderr) or proc.stdout.strip() or str(exc)
        raise RuntimeError(details) from exc


def run_remote_cmd(host: str, command: str, *, check: bool = True) -> subprocess.CompletedProcess:
    proc = subprocess.run(
        [*ssh_base(), host, "cmd", "/c", command],
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


def infer_windows_asset_label(*values: str) -> str:
    for value in values:
        lower = str(value or "").lower()
        if "windows-aarch64" in lower:
            return "windows-aarch64"
        if "windows-x86_64" in lower or "windows-amd64" in lower or "windows-x64" in lower:
            return "windows-x86_64"
    return "windows-x86_64"


def install_staged_windows_artifact(
    host: str,
    remote_root: str,
    remote_bin: str,
    artifact_hint: str,
) -> dict:
    asset_label = infer_windows_asset_label(artifact_hint, remote_bin)
    statements = [
        f"$RemoteRoot = {ps_literal(remote_root)}",
        f"$StagedBin = {ps_literal(remote_bin)}",
        f"$AssetLabel = {ps_literal(asset_label)}",
        "$ErrorActionPreference = 'Stop'",
        "$versionText = (& $StagedBin --version 2>$null | Out-String).Trim()",
        "if (-not $versionText) { throw 'failed to read staged yggterm version' }",
        "$version = $versionText",
        "if ($versionText -match '([0-9]+\\.[0-9]+\\.[0-9]+(?:[-+][^\\s]+)?)') { $version = $Matches[1] }",
        "$installRoot = Join-Path $env:LOCALAPPDATA 'Yggterm'",
        "$versionDir = Join-Path (Join-Path $installRoot 'versions') $version",
        "New-Item -ItemType Directory -Force -Path $installRoot, (Split-Path -Parent $versionDir), $versionDir | Out-Null",
        "$installedExe = Join-Path $versionDir 'yggterm.exe'",
        "$installedHeadless = Join-Path $versionDir 'yggterm-headless.exe'",
        "$installedMock = Join-Path $versionDir 'yggterm-mock-cli.exe'",
        "$installedLoader = Join-Path $versionDir 'WebView2Loader.dll'",
        "$launcher = Join-Path $installRoot 'Yggterm.vbs'",
        "$stagedHeadless = Get-ChildItem -Path $RemoteRoot -Filter 'yggterm-headless*.exe' -File -ErrorAction SilentlyContinue | Sort-Object Name | Select-Object -Last 1",
        "$stagedMock = Get-ChildItem -Path $RemoteRoot -Filter 'yggterm-mock-cli*.exe' -File -ErrorAction SilentlyContinue | Sort-Object Name | Select-Object -Last 1",
        "$stagedLoader = Get-ChildItem -Path $RemoteRoot -Filter 'WebView2Loader*.dll' -File -ErrorAction SilentlyContinue | Sort-Object Name | Select-Object -Last 1",
        "if (-not $stagedHeadless) { throw 'staged artifact is missing yggterm-headless.exe' }",
        "if (-not $stagedLoader) { throw 'staged artifact is missing WebView2Loader.dll' }",
        "Copy-Item $StagedBin $installedExe -Force",
        "Copy-Item $stagedHeadless.FullName $installedHeadless -Force",
        "if ($stagedMock) { Copy-Item $stagedMock.FullName $installedMock -Force }",
        "Copy-Item $stagedLoader.FullName $installedLoader -Force",
        "$statePayload = @{ channel = 'direct'; repo = 'yggdrasilhq/yggterm'; asset_label = $AssetLabel; active_version = $version; active_executable = $installedExe; icon_revision = $version }",
        "$state = $statePayload | ConvertTo-Json -Depth 6",
        "$utf8NoBom = New-Object System.Text.UTF8Encoding($false)",
        "[System.IO.File]::WriteAllText((Join-Path $installRoot 'install-state.json'), $state, $utf8NoBom)",
        "$integrateOutput = $null",
        "$launcherScript = "
        + "('Set shell = CreateObject(\"\"WScript.Shell\"\")' + \"`r`n\" "
        + "+ 'shell.CurrentDirectory = \"\"' + $versionDir + '\"\"' + \"`r`n\" "
        + "+ 'shell.Run \"\"\"\"' + $installedExe + '\"\"\"\", 0, False' + \"`r`n\")",
        "[System.IO.File]::WriteAllText($launcher, $launcherScript, $utf8NoBom)",
        "$startMenuShortcut = Join-Path $env:APPDATA 'Microsoft\\Windows\\Start Menu\\Programs\\Yggterm.lnk'",
        "$legacyShortcut = Join-Path $env:APPDATA 'Microsoft\\Windows\\Start Menu\\Programs\\Yggterm\\Yggterm.lnk'",
        "$legacyShortcutDir = Join-Path $env:APPDATA 'Microsoft\\Windows\\Start Menu\\Programs\\Yggterm'",
        "$appPathsKey = 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\yggterm.exe'",
        "$uninstallKey = 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Yggterm'",
        "if (Test-Path -LiteralPath $legacyShortcut) { Remove-Item -LiteralPath $legacyShortcut -Force -ErrorAction SilentlyContinue }",
        "if (Test-Path -LiteralPath $legacyShortcutDir) { Remove-Item -LiteralPath $legacyShortcutDir -Recurse -Force -ErrorAction SilentlyContinue }",
        "$ws = New-Object -ComObject WScript.Shell",
        "$sc = $ws.CreateShortcut($startMenuShortcut)",
        "$sc.TargetPath = $installedExe",
        "$sc.WorkingDirectory = $versionDir",
        "$sc.IconLocation = \"$installedExe,0\"",
        "$sc.Description = 'Remote-first terminal workspace'",
        "$sc.Save()",
        "& reg.exe add 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\yggterm.exe' /ve /d $installedExe /f | Out-Null",
        "& reg.exe add 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\yggterm.exe' /v Path /d $versionDir /f | Out-Null",
        "& reg.exe add 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Yggterm' /v DisplayName /d 'Yggterm' /f | Out-Null",
        "& reg.exe add 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Yggterm' /v DisplayVersion /d $version /f | Out-Null",
        "& reg.exe add 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Yggterm' /v DisplayIcon /d $installedExe /f | Out-Null",
        "& reg.exe add 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Yggterm' /v InstallLocation /d $installRoot /f | Out-Null",
        "& reg.exe add 'HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Yggterm' /v Publisher /d 'YggdrasilHQ' /f | Out-Null",
        "$installedMockPath = $null",
        "if (Test-Path $installedMock) { $installedMockPath = $installedMock }",
        "$result = @{ asset_label = $AssetLabel; version_text = $versionText; version = $version; install_root = $installRoot; installed_exe = $installedExe; installed_headless = $installedHeadless; installed_mock = $installedMockPath; installed_loader = $installedLoader; launcher = $launcher; launcher_exists = (Test-Path $launcher); integrate_output = $integrateOutput; start_menu_shortcut = $startMenuShortcut; start_menu_shortcut_exists = (Test-Path $startMenuShortcut); legacy_start_menu_shortcut_exists = (Test-Path $legacyShortcut); app_paths_exists = (Test-Path $appPathsKey); uninstall_key_exists = (Test-Path $uninstallKey) }",
        "$result | ConvertTo-Json -Depth 8 -Compress",
    ]
    script = ";\n".join(statements) + "\n"
    return run_remote_powershell_json(host, script)


def query_windows_install_integration(host: str) -> dict:
    statements = [
        "$ErrorActionPreference = 'Stop'",
        "$installRoot = Join-Path $env:LOCALAPPDATA 'Yggterm'",
        "$installState = Join-Path $installRoot 'install-state.json'",
        "$launcher = Join-Path $installRoot 'Yggterm.vbs'",
        "$startMenuShortcut = Join-Path $env:APPDATA 'Microsoft\\Windows\\Start Menu\\Programs\\Yggterm.lnk'",
        "$legacyShortcut = Join-Path $env:APPDATA 'Microsoft\\Windows\\Start Menu\\Programs\\Yggterm\\Yggterm.lnk'",
        "$appPathsKey = 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\App Paths\\yggterm.exe'",
        "$uninstallKey = 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Yggterm'",
        "$installStateJson = $null",
        "$appPathsDefault = $null",
        "$installedExeSubsystem = $null",
        "if (Test-Path $installState) { try { $installStateJson = Get-Content $installState -Raw | ConvertFrom-Json } catch {} }",
        "if (Test-Path $appPathsKey) { try { $appPathsDefault = (Get-ItemProperty -Path $appPathsKey).'(default)' } catch {} }",
        "$installedExePath = $null",
        "if ($installStateJson -and $installStateJson.active_executable) { $installedExePath = [string]$installStateJson.active_executable }",
        "if ($installedExePath -and (Test-Path $installedExePath)) { $fs = [System.IO.File]::Open($installedExePath, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite); try { $br = New-Object System.IO.BinaryReader($fs); $fs.Seek(0x3C, [System.IO.SeekOrigin]::Begin) | Out-Null; $peOffset = $br.ReadInt32(); $fs.Seek($peOffset + 0x5C, [System.IO.SeekOrigin]::Begin) | Out-Null; $installedExeSubsystem = [int]$br.ReadUInt16() } finally { $fs.Dispose() } }",
        "$result = @{ install_root = $installRoot; install_root_exists = (Test-Path $installRoot); install_state_path = $installState; install_state_exists = (Test-Path $installState); install_state = $installStateJson; launcher = $launcher; launcher_exists = (Test-Path $launcher); start_menu_shortcut = $startMenuShortcut; start_menu_shortcut_exists = (Test-Path $startMenuShortcut); legacy_start_menu_shortcut_exists = (Test-Path $legacyShortcut); app_paths_exists = (Test-Path $appPathsKey); app_paths_default = $appPathsDefault; uninstall_key_exists = (Test-Path $uninstallKey); installed_exe_subsystem = $installedExeSubsystem }",
        "$result | ConvertTo-Json -Depth 8 -Compress",
    ]
    script = ";\n".join(statements) + "\n"
    return run_remote_powershell_json(host, script)


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


def remote_create_plain_terminal(
    host: str,
    remote_bin: str,
    remote_home: str,
    pid: int,
    timeout_ms: int,
    *,
    title: str = "Remote Smoke Plain",
) -> dict:
    payload = remote_app_json_command(
        host,
        remote_bin,
        remote_home,
        [
            "server",
            "app",
            "terminal",
            "new",
            "--pid",
            str(pid),
            "--title",
            title,
            "--timeout-ms",
            str(timeout_ms),
        ],
    )
    if payload.get("error"):
        raise RuntimeError(f"plain terminal creation failed on {host}: {payload['error']}")
    data = payload.get("data")
    if not isinstance(data, dict):
        raise RuntimeError(f"plain terminal creation returned no data payload on {host}: {payload!r}")
    session_path = str(data.get("active_session_path") or "").strip()
    if not session_path:
        raise RuntimeError(f"plain terminal creation did not return an active_session_path on {host}")
    return data


def wait_for_terminal_ready_state(
    host: str,
    remote_bin: str,
    remote_home: str,
    app_pid: int,
    timeout_ms: int,
    session_path: str,
    *,
    wait_seconds: float = 45.0,
) -> tuple[dict, dict]:
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
            bad_notifications = problem_notifications(last_state)
            if bad_notifications:
                raise RuntimeError(f"bad daemon/socket notifications observed: {bad_notifications!r}")
            terminal = assert_active_terminal_host_ready(last_state, session_path)
            return last_state, terminal
        except Exception as exc:  # noqa: BLE001
            last_error = str(exc)
            time.sleep(0.25)
    raise RuntimeError(
        f"remote Windows terminal never became ready for pid {app_pid} within {wait_seconds:.1f}s: "
        f"{last_error} state={last_state!r}"
    )


WINDOWS_LOCAL_SHELL_FAILURE_MARKERS = (
    "not recognized as an internal or external command",
    "operable program or batch file",
)


def assert_windows_local_shell_text_healthy(state: dict, session_path: str) -> dict:
    host = active_terminal_host(state)
    if not isinstance(host, dict):
        raise RuntimeError(f"Windows terminal proof state did not expose an active host: {state!r}")
    if str(host.get("session_path") or "").strip() != session_path:
        raise RuntimeError(
            "Windows terminal proof state active host drifted away from the created session: "
            f"expected={session_path!r} host={host!r}"
        )
    haystack = "\n".join(
        str(host.get(key) or "")
        for key in (
            "text_sample",
            "cursor_line_text",
            "cursor_row_text",
            "resume_overlay_text",
            "resume_overlay_excerpt",
        )
    )
    if not haystack.strip():
        raise RuntimeError(f"Windows terminal proof did not expose readable terminal text: {host!r}")
    lowered = haystack.lower()
    bad_markers = [marker for marker in WINDOWS_LOCAL_SHELL_FAILURE_MARKERS if marker in lowered]
    if bad_markers:
        raise RuntimeError(
            "Windows local shell launched into an error screen instead of an interactive prompt: "
            f"markers={bad_markers!r} text={haystack[-500:]!r}"
        )
    return {
        "text_tail": haystack[-500:],
        "cursor_row_text": host.get("cursor_row_text"),
        "cursor_line_text": host.get("cursor_line_text"),
    }


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


def list_visible_console_windows(host: str) -> list[dict]:
    script = r"""
$names = @('cmd','powershell','pwsh','conhost','wt','WindowsTerminal','OpenConsole');
$rows = @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 -and $names -contains $_.ProcessName } | Select-Object Id, ProcessName, SessionId, MainWindowHandle, MainWindowTitle, Path);
@{ rows = $rows } | ConvertTo-Json -Depth 6 -Compress
"""
    payload = run_remote_powershell_json(host, script)
    rows = payload.get("rows") if isinstance(payload, dict) else payload
    if isinstance(rows, list):
        return rows
    if isinstance(rows, dict):
        return [rows]
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


def list_windows_installed_processes(host: str) -> list[dict]:
    installed = []
    for process in list_windows_yggterm_processes(host):
        executable_path = str(process.get("executable_path") or "").strip().lower().replace("/", "\\")
        if "\\appdata\\local\\yggterm\\versions\\" in executable_path:
            installed.append(process)
    return installed


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
    task_query = None
    run_remote_cmd(host, f"schtasks /Delete /TN {task_name} /F", check=False)
    if not metadata_text:
        task_query = run_remote_cmd(host, f"schtasks /Query /TN {task_name} /FO LIST /V", check=False)
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
        "task_query": task_query.stdout.strip() if task_query else None,
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


def assert_windows_screenshot_quality(payload: dict | None, screenshot_path: Path) -> dict:
    backend = screenshot_backend(payload)
    attempts = screenshot_backend_attempts(payload)
    if not backend:
        raise RuntimeError(f"remote screenshot response did not expose a capture backend: {payload!r}")
    if backend == "windows_screen_copy":
        raise RuntimeError(
            "Windows screenshot fell back to desktop screen copy, which is not release-grade "
            f"for proof capture because other windows/dialogs can pollute the image: attempts={attempts!r}"
        )
    quality = assert_screenshot_file_usable(screenshot_path)
    return {
        "backend": backend,
        "attempts": attempts,
        "image": quality,
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
    configure_remote_transport(args.proxy_jump, args.ssh_port)
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
    expected_workspace_version = None
    artifact_hint = ""
    remote_version = None
    staged_support_files: list[str] = []
    staged_companion_binaries: list[str] = []
    dismissed_error_dialogs_prelaunch = None
    dismissed_error_dialogs_prescreenshot = None
    visible_console_windows_before_launch: list[dict] = []
    visible_console_windows_after_terminal_create: list[dict] = []
    new_visible_console_windows: list[dict] = []
    owned_processes_before_launch: list[dict] = []
    owned_processes_after_prelaunch_cleanup: list[dict] = []
    killed_processes_prelaunch: list[dict] = []
    installed_processes_before_install: list[dict] = []
    killed_installed_processes_preinstall: list[dict] = []
    owned_processes_before_close: list[dict] = []
    owned_processes_after_close: list[dict] = []
    killed_processes_postclose: list[dict] = []
    postclose_error: str | None = None
    windows_error_events_before_launch: list[dict] = []
    windows_error_events_after_launch: list[dict] = []
    new_windows_error_events: list[dict] = []
    install_summary: dict | None = None
    install_integration: dict | None = None

    try:
        try:
            windows_error_events_before_launch = collect_recent_windows_error_events(args.host)
        except Exception:
            windows_error_events_before_launch = []
        try:
            visible_console_windows_before_launch = list_visible_console_windows(args.host)
        except Exception:
            visible_console_windows_before_launch = []
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
                artifact_hint = artifact.name
                local_artifact_sha256 = sha256_file(artifact)
                if artifact_under_dist(artifact):
                    expected_workspace_version = workspace_version()
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
        if args.install:
            installed_processes_before_install = list_windows_installed_processes(args.host)
            if installed_processes_before_install:
                killed_installed_processes_preinstall = kill_windows_processes(
                    args.host,
                    installed_processes_before_install,
                )
                time.sleep(0.75)
            install_summary = install_staged_windows_artifact(
                args.host,
                remote_root,
                remote_bin,
                artifact_hint or remote_bin,
            )
            remote_bin = str(install_summary.get("installed_exe") or "").strip() or remote_bin
        remote_control_bin = next(
            (
                path
                for path in staged_companion_binaries
                if path.lower().endswith("yggterm-headless-windows-x86_64.exe")
                or path.lower().endswith("yggterm-headless.exe")
            ),
            remote_bin,
        )
        if args.install:
            installed_headless = str((install_summary or {}).get("installed_headless") or "").strip()
            if installed_headless:
                remote_control_bin = installed_headless
        try:
            remote_version = remote_binary_version(args.host, remote_bin)
        except Exception:
            remote_version = None
        if install_summary is not None and not remote_version:
            remote_version = str(install_summary.get("version") or "").strip() or None
        if expected_workspace_version and remote_version and remote_version != expected_workspace_version:
            raise RuntimeError(
                "remote Windows smoke staged a stale artifact from dist/: "
                f"workspace_version={expected_workspace_version!r} artifact_version={remote_version!r}"
            )
        if args.install:
            install_integration = query_windows_install_integration(args.host)
            if not (
                bool(install_integration.get("install_state_exists"))
                and bool(install_integration.get("start_menu_shortcut_exists"))
                and bool(install_integration.get("app_paths_exists"))
            ):
                raise RuntimeError(
                    "Windows staged install did not produce the expected shell integration: "
                    f"{install_integration!r}"
                )
            installed_exe = str((install_summary or {}).get("installed_exe") or "").strip()
            shortcut_target = str(install_integration.get("start_menu_shortcut_target") or "").strip()
            app_paths_default = str(install_integration.get("app_paths_default") or "").strip()
            if installed_exe and shortcut_target and shortcut_target.lower() != installed_exe.lower():
                raise RuntimeError(
                    "Windows Start Menu shortcut is not targeting the real GUI executable: "
                    f"shortcut_target={shortcut_target!r} installed_exe={installed_exe!r}"
                )
            if installed_exe and app_paths_default.lower() != installed_exe.lower():
                raise RuntimeError(
                    "Windows App Paths entry is not targeting the installed GUI executable: "
                    f"app_paths_default={app_paths_default!r} installed_exe={installed_exe!r}"
                )
            if int(install_integration.get("installed_exe_subsystem") or 0) != 2:
                raise RuntimeError(
                    "Windows installed yggterm.exe is not using the GUI subsystem: "
                    f"{install_integration!r}"
                )
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
        titlebar = assert_titlebar_utility_buttons_inline(state)
        created_terminal = remote_create_plain_terminal(
            args.host,
            remote_control_bin,
            remote_home,
            app_pid,
            args.timeout_ms,
        )
        state, terminal = wait_for_terminal_ready_state(
            args.host,
            remote_control_bin,
            remote_home,
            app_pid,
            args.timeout_ms,
            str(created_terminal.get("active_session_path") or "").strip(),
        )
        try:
            visible_console_windows_after_terminal_create = list_visible_console_windows(args.host)
        except Exception:
            visible_console_windows_after_terminal_create = []
        baseline_console_window_ids = {
            int(window.get("Id") or 0) for window in visible_console_windows_before_launch
        }
        new_visible_console_windows = [
            window
            for window in visible_console_windows_after_terminal_create
            if int(window.get("Id") or 0) not in baseline_console_window_ids
        ]
        if new_visible_console_windows:
            raise RuntimeError(
                "Windows launch/workflow surfaced new visible console windows instead of staying first-class GUI-only: "
                f"{new_visible_console_windows!r}"
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
        sidebar = assert_sidebar_rows_present(rows)

        state_path = proof_dir / "state.json"
        rows_path = proof_dir / "rows.json"
        screenshot_path = proof_dir / "window.png"
        summary_path = proof_dir / "summary.json"
        state_path.write_text(json.dumps(state, indent=2), encoding="utf-8")
        rows_path.write_text(json.dumps(rows, indent=2), encoding="utf-8")

        screenshot_response = None
        screenshot_error = None
        terminal_text = None
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
            screenshot_quality = assert_windows_screenshot_quality(
                screenshot_response,
                screenshot_path,
            )
        except Exception as exc:  # noqa: BLE001
            screenshot_error = str(exc)
            screenshot_quality = None
        if screenshot_error:
            raise RuntimeError(f"Windows proof screenshot failed: {screenshot_error}")
        terminal_text = assert_windows_local_shell_text_healthy(
            (screenshot_response or {}).get("data") or {},
            str(created_terminal.get("active_session_path") or "").strip(),
        )

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
            "sidebar": sidebar,
            "titlebar_right_controls": titlebar,
            "created_terminal": created_terminal,
            "terminal": terminal,
            "terminal_text": terminal_text,
            "state_path": str(state_path),
            "rows_path": str(rows_path),
            "screenshot_path": str(screenshot_path) if screenshot_path.exists() else None,
            "screenshot_response": screenshot_response,
            "screenshot_backend": (screenshot_quality or {}).get("backend"),
            "screenshot_backend_attempts": (screenshot_quality or {}).get("attempts") or [],
            "screenshot_quality": (screenshot_quality or {}).get("image"),
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
            "workspace_version": expected_workspace_version,
            "staged_support_files": staged_support_files,
            "staged_companion_binaries": staged_companion_binaries,
            "install_summary": install_summary,
            "install_integration": install_integration,
            "owned_processes_before_launch": owned_processes_before_launch,
            "killed_processes_prelaunch": killed_processes_prelaunch,
            "installed_processes_before_install": installed_processes_before_install,
            "killed_installed_processes_preinstall": killed_installed_processes_preinstall,
            "owned_processes_after_prelaunch_cleanup": owned_processes_after_prelaunch_cleanup,
            "dismissed_error_dialogs_prelaunch": dismissed_error_dialogs_prelaunch,
            "dismissed_error_dialogs_prescreenshot": dismissed_error_dialogs_prescreenshot,
            "visible_console_windows_before_launch": visible_console_windows_before_launch,
            "visible_console_windows_after_terminal_create": visible_console_windows_after_terminal_create,
            "new_visible_console_windows": new_visible_console_windows,
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
            "workspace_version": expected_workspace_version,
            "staged_support_files": staged_support_files,
            "staged_companion_binaries": staged_companion_binaries,
            "install_summary": install_summary,
            "install_integration": install_integration,
            "owned_processes_before_launch": owned_processes_before_launch,
            "killed_processes_prelaunch": killed_processes_prelaunch,
            "installed_processes_before_install": installed_processes_before_install,
            "killed_installed_processes_preinstall": killed_installed_processes_preinstall,
            "owned_processes_after_prelaunch_cleanup": owned_processes_after_prelaunch_cleanup,
            "dismissed_error_dialogs_prelaunch": dismissed_error_dialogs_prelaunch,
            "dismissed_error_dialogs_prescreenshot": dismissed_error_dialogs_prescreenshot,
            "visible_console_windows_before_launch": visible_console_windows_before_launch,
            "visible_console_windows_after_terminal_create": visible_console_windows_after_terminal_create,
            "new_visible_console_windows": new_visible_console_windows,
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
