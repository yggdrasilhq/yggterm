#!/usr/bin/env python3
import argparse
import base64
import json
import subprocess
from pathlib import Path


SSH_BASE = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8"]

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
$clients = @()
if ($clientsPayload.clients) {
  $clients = @($clientsPayload.clients)
} elseif ($clientsPayload.data) {
  $clients = @($clientsPayload.data)
}
$chosen = $null
if ($clients.Count -gt 0) {
  $chosen = $clients | Sort-Object {[int]($_.pid)} | Select-Object -Last 1
}

$state = $null
$screenshotBase64 = $null
if ($chosen) {
  $pid = [int]$chosen.pid
  $statePayload = & $bin server app state --pid $pid --timeout-ms $TimeoutMs | ConvertFrom-Json
  $state = $statePayload.data
  if ($ScreenshotPath) {
    & $bin server app screenshot --pid $pid $ScreenshotPath --timeout-ms $TimeoutMs | Out-Null
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
    parser.add_argument("--bin")
    parser.add_argument("--timeout-ms", type=int, default=8000)
    parser.add_argument("--out-dir")
    parser.add_argument("--skip-screenshot", action="store_true")
    return parser.parse_args()


def run_remote_powershell(host: str, script: str) -> subprocess.CompletedProcess:
    proc = subprocess.run(
        [*SSH_BASE, host, "powershell", "-NoProfile", "-NonInteractive", "-File", "-"],
        text=True,
        capture_output=True,
        input=script,
    )
    if proc.returncode != 0:
        raise RuntimeError(proc.stderr.strip() or proc.stdout.strip() or f"powershell failed on {host}")
    return proc


def ps_literal(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def extract_json_text(stdout: str) -> str:
    for line in reversed(stdout.splitlines()):
        candidate = line.strip()
        if candidate.startswith("{") or candidate.startswith("["):
            return candidate
    raise RuntimeError(f"could not find JSON payload in PowerShell output: {stdout!r}")


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir or f"/tmp/yggterm-remote-windows-{args.host}")
    out_dir.mkdir(parents=True, exist_ok=True)
    screenshot_remote = "" if args.skip_screenshot else r"$env:TEMP\yggterm-remote-live.png"
    launcher = (
        f"$RequestedBin = {ps_literal(args.bin or '')}\n"
        + f"$TimeoutMs = {args.timeout_ms}\n"
        + f"$ScreenshotPath = {ps_literal(screenshot_remote)}\n"
        + POWERSHELL_SNIPPET
    )
    proc = run_remote_powershell(args.host, launcher)
    try:
        payload = json.loads(extract_json_text(proc.stdout))
    except Exception as exc:  # noqa: BLE001
        detail = proc.stderr.strip() or proc.stdout.strip() or str(exc)
        raise RuntimeError(detail) from exc

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
