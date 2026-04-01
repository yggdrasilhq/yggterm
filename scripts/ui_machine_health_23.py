#!/usr/bin/env python3
import argparse
import json
import shlex
import subprocess
import time
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Cross-check live machine reachability against Yggterm app-state machine health "
            "for 23-style repeated verification."
        )
    )
    parser.add_argument("--bin", default="./target/debug/yggterm")
    parser.add_argument("--hosts", nargs="+", default=["jojo", "oc"])
    parser.add_argument("--count", type=int, default=23)
    parser.add_argument("--timeout-ms", type=int, default=8000)
    parser.add_argument("--poll", type=float, default=0.5)
    parser.add_argument("--out-dir", default="/tmp/yggterm-machine-health-23")
    return parser.parse_args()


def run(argv: list[str], *, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(argv, check=check, text=True, capture_output=True)


def run_json(command: str) -> dict:
    result = run(["bash", "-lc", command], check=False)
    stdout = result.stdout.strip()
    stderr = result.stderr.strip()
    if result.returncode != 0 or not stdout:
        raise RuntimeError(
            f"command failed rc={result.returncode}: {command}\nstdout:\n{stdout or '<empty>'}\nstderr:\n{stderr or '<empty>'}"
        )
    try:
        return json.loads(stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(
            f"invalid json for command: {command}\nstdout:\n{stdout}"
        ) from error


def probe_remote_truth(host: str) -> dict:
    command = (
        "ssh -o BatchMode=yes -o ConnectTimeout=5 "
        f"{shlex.quote(host)} "
        "'~/.yggterm/bin/yggterm server remote protocol-version'"
    )
    result = run(["bash", "-lc", command], check=False)
    stdout = result.stdout.strip()
    stderr = result.stderr.strip()
    reachable = result.returncode == 0 and stdout.startswith("{")
    return {
        "host": host,
        "reachable": reachable,
        "stdout": stdout,
        "stderr": stderr,
        "returncode": result.returncode,
    }


def app_state(binary: str, timeout_ms: int) -> dict:
    command = f"{shlex.quote(str(Path(binary).resolve()))} server app state --timeout-ms {timeout_ms}"
    payload = run_json(command)
    return payload.get("data") or {}


def normalize_health(value: str | None) -> str | None:
    if value is None:
        return None
    return value.strip().lower().replace(" ", "_")


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    results: list[dict] = []

    for index in range(args.count):
        trial = {"trial": index, "machines": [], "error": None}
        try:
            state = app_state(args.bin, args.timeout_ms)
            ui_machines = {
                normalize_health(str(machine.get("machine_key"))): machine
                for machine in ((state.get("remote") or {}).get("machines") or [])
                if machine.get("machine_key")
            }
            trial["state_dump"] = str(out_dir / f"machine-health-{index:02d}.json")
            Path(trial["state_dump"]).write_text(json.dumps(state, indent=2), encoding="utf-8")
            for host in args.hosts:
                truth = probe_remote_truth(host)
                ui_machine = ui_machines.get(host)
                ui_health = normalize_health(
                    None if ui_machine is None else str(ui_machine.get("health"))
                )
                ui_color = None if ui_machine is None else ui_machine.get("machine_indicator_color")
                expected_health = "healthy" if truth["reachable"] else "offline"
                expected_color = "#16a34a" if truth["reachable"] else None
                mismatch = False
                if truth["reachable"]:
                    mismatch = ui_health != "healthy" or ui_color != expected_color
                elif ui_health == "healthy":
                    mismatch = True
                trial["machines"].append(
                    {
                        "host": host,
                        "truth": truth,
                        "ui_machine": ui_machine,
                        "ui_health": ui_health,
                        "ui_color": ui_color,
                        "expected_health": expected_health,
                        "expected_color": expected_color,
                        "mismatch": mismatch,
                    }
                )
        except Exception as error:  # noqa: BLE001
            trial["error"] = str(error)
        results.append(trial)
        time.sleep(args.poll)

    summary = {
        "count": args.count,
        "hosts": args.hosts,
        "app_state_failures": len([item for item in results if item.get("error")]),
        "machine_mismatches": sum(
            1
            for item in results
            for machine in item.get("machines", [])
            if machine.get("mismatch")
        ),
        "results": results,
    }
    summary_path = out_dir / "summary.json"
    summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(summary_path)
    print(json.dumps(summary, indent=2))
    return 0 if summary["app_state_failures"] == 0 and summary["machine_mismatches"] == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
