#!/usr/bin/env python3
import argparse
import json
import time
from pathlib import Path, PureWindowsPath

from remote_linux_x11_smoke import scp_from, scp_to
from remote_windows_live_app import extract_json_text, ps_literal, run_remote_powershell
from smoke_app_control_bootstrap import assert_blur_expectation, problem_notifications


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_ARTIFACT = ROOT / "dist" / "yggterm-windows-x86_64.exe"

WINDOWS_HELPERS = r"""
$ErrorActionPreference = "Stop"

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


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Stage or attach to Yggterm on a remote Windows host and run a minimal app-control smoke."
    )
    parser.add_argument("--host", required=True)
    parser.add_argument("--artifact", default=str(DEFAULT_ARTIFACT))
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
        details = proc.stderr.strip() or proc.stdout.strip() or str(exc)
        raise RuntimeError(details) from exc


def resolve_remote_dir_name(args: argparse.Namespace) -> str:
    return args.remote_dir_name or f"yggterm-remote-windows-{int(time.time())}"


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


def stage_artifact(host: str, artifact: Path, remote_dir_name: str) -> None:
    ensure_remote_dir(host, remote_dir_name)
    remote_rel = f"{remote_dir_name}/{artifact.name}"
    scp_to(host, artifact, remote_rel)
    if artifact.suffixes[-2:] == [".tar", ".gz"]:
        script = (
            f"$RemoteDirName = {ps_literal(remote_dir_name)}\n"
            + WINDOWS_HELPERS
            + "\n"
            + "$remoteRoot = Ensure-RemoteRoot -DirName $RemoteDirName\n"
            + f"tar -xzf (Join-Path $remoteRoot {ps_literal(artifact.name)}) -C $remoteRoot\n"
        )
        run_remote_powershell(host, script)


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
    scp_from(host, remote_scp_path, local_path)
    return payload


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
    remote_dir_name = resolve_remote_dir_name(args)
    remote_root = ensure_remote_dir(args.host, remote_dir_name)
    local_summary_path = out_dir / "summary.json"
    remote_bin = ""
    remote_home = ""
    remote_log = ""
    remote_screenshot_path = ""
    remote_screenshot_scp = f"{remote_dir_name}/window.png"
    app_pid = 0
    owned_launch = False
    launch_payload = None
    direct_spawn_pid = 0

    try:
        artifact_value = str(args.artifact or "").strip()
        if artifact_value:
            artifact = Path(artifact_value).expanduser()
            if artifact.exists():
                stage_artifact(args.host, artifact, remote_dir_name)

        remote_bin = resolve_remote_bin(args.host, remote_dir_name, args.remote_bin)
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
            try:
                launch_payload = remote_app_json_command(
                    args.host,
                    remote_bin,
                    remote_home,
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
                app_pid = int(launch_payload.get("pid") or 0)
                if app_pid <= 0:
                    raise RuntimeError(
                        f"background app launch did not return a pid on {args.host}: {launch_payload!r}"
                    )
                owned_launch = True
            except Exception as exc:  # noqa: BLE001
                if not should_fallback_direct_launch(exc):
                    raise
                launch_payload = {
                    "mode": "direct_process_fallback",
                    "error": str(exc),
                    "spawn": spawn_direct_windows_app(
                        args.host,
                        remote_bin,
                        remote_home,
                        remote_log,
                    ),
                }
                direct_spawn_pid = int(
                    ((launch_payload.get("spawn") or {}).get("spawn_pid")) or 0
                )
                app_pid = wait_for_client_pid(
                    args.host,
                    remote_bin,
                    remote_home,
                    args.timeout_ms,
                )
                owned_launch = True

        state = wait_for_ready_state(
            args.host,
            remote_bin,
            remote_home,
            app_pid,
            args.timeout_ms,
        )
        blur = assert_blur_expectation(state, args.expect_live_blur)
        rows_payload = remote_app_json_command(
            args.host,
            remote_bin,
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
            screenshot_response = capture_remote_screenshot(
                args.host,
                remote_bin,
                remote_home,
                app_pid,
                args.timeout_ms,
                remote_screenshot_path,
                remote_screenshot_scp,
                screenshot_path,
            )
        except Exception as exc:  # noqa: BLE001
            screenshot_error = str(exc)

        proof_summary = {
            "bin": remote_bin,
            "pid": app_pid,
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
            "screenshot_error": screenshot_error,
        }
        summary_path.write_text(json.dumps(proof_summary, indent=2), encoding="utf-8")

        summary = {
            "host": args.host,
            "remote_dir_name": remote_dir_name,
            "remote_root": remote_root,
            "remote_home": remote_home,
            "remote_bin": remote_bin,
            "pid": app_pid,
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
            "remote_dir_name": remote_dir_name,
            "remote_root": remote_root,
            "remote_home": remote_home or None,
            "remote_bin": remote_bin or None,
            "pid": app_pid or None,
            "owned_launch": owned_launch,
            "launch": launch_payload,
            "error": str(exc),
            "local_proof_dir": str(proof_dir),
        }
        local_summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
        print(local_summary_path)
        return_code = 1
    finally:
        if app_pid > 0 and owned_launch:
            try:
                remote_app_json_command(
                    args.host,
                    remote_bin,
                    remote_home,
                    ["server", "app", "close", "--pid", str(app_pid), "--timeout-ms", str(args.timeout_ms)],
                )
            except Exception:
                if direct_spawn_pid > 0:
                    stop_direct_windows_process(args.host, direct_spawn_pid)
            try:
                remote_app_json_command(
                    args.host,
                    remote_bin,
                    remote_home,
                    ["server", "shutdown"],
                )
            except Exception:
                pass
        if not args.keep_remote_dir:
            cleanup_remote_dir(args.host, remote_root)
    return return_code


if __name__ == "__main__":
    raise SystemExit(main())
