#!/usr/bin/env python3
import argparse
import base64
import json
import os
import subprocess
from pathlib import Path


POWERSHELL_SNIPPET = r"""
$ErrorActionPreference = "Stop"

function Resolve-YggtermBinary {
  param([string]$Requested)
  if ($Requested -and (Test-Path $Requested)) {
    return (Resolve-Path $Requested).Path
  }

  $root = Join-Path $env:LOCALAPPDATA "Yggterm"
  $installState = Join-Path $root "install-state.json"
  if (Test-Path $installState) {
    try {
      $state = Get-Content $installState -Raw | ConvertFrom-Json
      if ($state.active_executable -and (Test-Path $state.active_executable)) {
        return (Resolve-Path $state.active_executable).Path
      }
    } catch {
    }
  }

  $versions = Join-Path $root "versions"
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

  throw "could not resolve yggterm.exe under $root"
}

$bin = Resolve-YggtermBinary -Requested $RequestedBin
$clientsPayload = & $bin server app clients --timeout-ms $TimeoutMs | ConvertFrom-Json
$rawClients = @()
if ($clientsPayload.clients) {
  $rawClients = @($clientsPayload.clients)
} elseif ($clientsPayload.data) {
  $rawClients = @($clientsPayload.data)
}
$clients = @()
foreach ($client in $rawClients) {
  $clientPid = [int]($client.pid)
  $procInfo = Get-Process -Id $clientPid -ErrorAction SilentlyContinue |
    Select-Object Id, Responding, MainWindowTitle, MainWindowHandle, Path
  $clients += [pscustomobject]@{
    pid = $clientPid
    started_at_ms = $client.started_at_ms
    display = $client.display
    wayland_display = $client.wayland_display
    xauthority = $client.xauthority
    xdg_runtime_dir = $client.xdg_runtime_dir
    xdg_session_id = $client.xdg_session_id
    process_path = if ($procInfo) { $procInfo.Path } else { $null }
    responding = if ($procInfo) { $procInfo.Responding } else { $null }
    main_window_title = if ($procInfo) { $procInfo.MainWindowTitle } else { $null }
    main_window_handle = if ($procInfo) { $procInfo.MainWindowHandle } else { $null }
  }
}
$chosen = $null
if ($clients.Count -gt 0) {
  $matching = @($clients | Where-Object { $_.process_path -eq $bin })
  if ($matching.Count -gt 0) {
    $chosen = $matching | Sort-Object {[int]($_.pid)} | Select-Object -Last 1
  }
}

$state = $null
$screenshotBase64 = $null
if ($chosen) {
  $clientPid = [int]$chosen.pid
  $statePayload = & $bin server app state --pid $clientPid --timeout-ms $TimeoutMs | ConvertFrom-Json
  $state = $statePayload.data
  if ($ScreenshotPath) {
    & $bin server app screenshot --pid $clientPid $ScreenshotPath --timeout-ms $TimeoutMs | Out-Null
    if (Test-Path $ScreenshotPath) {
      $bytes = [System.IO.File]::ReadAllBytes($ScreenshotPath)
      $screenshotBase64 = [Convert]::ToBase64String($bytes)
    }
  }
}

@{
  bin = $bin
  clients = $clients
  chosen_client = $chosen
  state = $state
  screenshot_base64 = $screenshotBase64
} | ConvertTo-Json -Depth 16 -Compress
"""


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Inspect a live Yggterm client on a remote Windows host over SSH."
    )
    parser.add_argument("--host", required=True)
    parser.add_argument("--proxy-jump")
    parser.add_argument("--ssh-port", type=int)
    parser.add_argument("--bin")
    parser.add_argument("--timeout-ms", type=int, default=8000)
    parser.add_argument("--out-dir")
    parser.add_argument("--skip-screenshot", action="store_true")
    return parser.parse_args()


def configure_remote_transport(proxy_jump: str | None, ssh_port: int | None) -> None:
    if proxy_jump:
        os.environ["YGGTERM_REMOTE_PROXY_JUMP"] = proxy_jump
    else:
        os.environ.pop("YGGTERM_REMOTE_PROXY_JUMP", None)
    if ssh_port:
        os.environ["YGGTERM_REMOTE_PORT"] = str(ssh_port)
    else:
        os.environ.pop("YGGTERM_REMOTE_PORT", None)


def ssh_base() -> list[str]:
    connect_timeout = str(os.environ.get("YGGTERM_REMOTE_CONNECT_TIMEOUT") or "").strip() or "40"
    args = [
        "ssh",
        "-o",
        "BatchMode=yes",
        "-o",
        f"ConnectTimeout={connect_timeout}",
        "-o",
        "StrictHostKeyChecking=accept-new",
    ]
    proxy_jump = str(os.environ.get("YGGTERM_REMOTE_PROXY_JUMP") or "").strip()
    if proxy_jump:
        args.extend(["-J", proxy_jump])
    ssh_port = str(os.environ.get("YGGTERM_REMOTE_PORT") or "").strip()
    if ssh_port:
        args.extend(["-p", ssh_port])
    return args


def run_remote_powershell(host: str, script: str) -> subprocess.CompletedProcess:
    proc = subprocess.run(
        [*ssh_base(), host, "powershell", "-NoProfile", "-NonInteractive", "-File", "-"],
        text=True,
        capture_output=True,
        input=script,
    )
    stderr = strip_windows_ssh_noise(proc.stderr)
    stdout = proc.stdout.strip()
    if proc.returncode != 0:
        raise RuntimeError(stderr or stdout or f"powershell failed on {host}")
    return proc


def run_remote_powershell_encoded(host: str, script: str) -> subprocess.CompletedProcess:
    encoded = base64.b64encode(script.encode("utf-16le")).decode("ascii")
    proc = subprocess.run(
        [
            *ssh_base(),
            host,
            "powershell",
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
            encoded,
        ],
        text=True,
        capture_output=True,
    )
    stderr = strip_windows_ssh_noise(proc.stderr)
    stdout = proc.stdout.strip()
    if proc.returncode != 0:
        raise RuntimeError(stderr or stdout or f"powershell failed on {host}")
    return proc


def run_remote_powershell_best_effort(host: str, script: str) -> subprocess.CompletedProcess:
    encoded_script = script.strip() + "\n"
    encoded = base64.b64encode(encoded_script.encode("utf-16le")).decode("ascii")
    if len(encoded) > 7900:
        return run_remote_powershell(host, script)
    try:
        return run_remote_powershell_encoded(host, encoded_script)
    except RuntimeError as exc:
        if "The command line is too long." in str(exc):
            return run_remote_powershell(host, script)
        raise


def ps_literal(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def strip_windows_ssh_noise(text: str) -> str:
    lines = []
    for raw_line in text.splitlines():
        line = raw_line.strip()
        if not line:
            continue
        if line.startswith("** WARNING: connection is not using a post-quantum key exchange algorithm."):
            continue
        if line.startswith('** This session may be vulnerable to "store now, decrypt later" attacks.'):
            continue
        if line.startswith("** The server may need to be upgraded. See https://openssh.com/pq.html"):
            continue
        lines.append(raw_line)
    return "\n".join(lines).strip()


def extract_json_text(stdout: str) -> str:
    lines = stdout.splitlines()
    for start in range(len(lines)):
        if not lines[start].lstrip().startswith(("{", "[")):
            continue
        for end in range(len(lines), start, -1):
            candidate = "\n".join(lines[start:end]).strip()
            if not candidate.endswith(("}", "]")):
                continue
            try:
                json.loads(candidate)
                return candidate
            except Exception:
                continue
    raise RuntimeError(f"could not find JSON payload in PowerShell output: {stdout!r}")


def main() -> int:
    args = parse_args()
    configure_remote_transport(args.proxy_jump, args.ssh_port)
    out_dir = Path(args.out_dir or f"/tmp/yggterm-remote-windows-{args.host}")
    out_dir.mkdir(parents=True, exist_ok=True)
    screenshot_remote = "" if args.skip_screenshot else r"$env:TEMP\yggterm-remote-live.png"
    launcher = (
        f"$RequestedBin = {ps_literal(args.bin or '')}\n"
        + f"$TimeoutMs = {args.timeout_ms}\n"
        + f"$ScreenshotPath = {ps_literal(screenshot_remote)}\n"
        + POWERSHELL_SNIPPET
    )
    proc = run_remote_powershell_best_effort(args.host, launcher)
    try:
        payload = json.loads(extract_json_text(proc.stdout))
    except Exception as exc:  # noqa: BLE001
        detail = proc.stderr.strip() or proc.stdout.strip() or str(exc)
        raise RuntimeError(detail) from exc
    if payload.get("chosen_client") and not isinstance(payload.get("state"), dict):
        raise RuntimeError(
            "remote Windows live-app inspection found a client but did not receive an app-control state payload"
        )

    screenshot_base64 = payload.pop("screenshot_base64", None)
    if screenshot_base64:
        screenshot_path = out_dir / "live-client.png"
        screenshot_path.write_bytes(base64.b64decode(screenshot_base64))
        payload["screenshot"] = str(screenshot_path)

    summary_path = out_dir / "summary.json"
    summary_path.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    print(summary_path)
    print(json.dumps(payload, indent=2))
    return 0 if payload.get("chosen_client") else 1


if __name__ == "__main__":
    raise SystemExit(main())
