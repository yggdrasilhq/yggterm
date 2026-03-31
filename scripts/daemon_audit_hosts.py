#!/usr/bin/env python3
import argparse
import json
import subprocess
from pathlib import Path


AUDIT_SNIPPET = r"""
import json, os, pathlib
rows = []
for pid in os.listdir('/proc'):
    if not pid.isdigit():
        continue
    proc = pathlib.Path('/proc') / pid
    try:
        parts = [
            part.decode('utf-8', 'ignore')
            for part in (proc / 'cmdline').read_bytes().split(b'\0')
            if part
        ]
    except Exception:
        continue
    if not parts:
        continue
    argv0 = pathlib.Path(parts[0]).name
    if 'yggterm' not in argv0:
        continue
    if 'server' not in parts or 'daemon' not in parts:
        continue
    try:
        env_items = (proc / 'environ').read_bytes().decode('utf-8', 'ignore').split('\0')
        home = next((item.split('=', 1)[1] for item in env_items if item.startswith('YGGTERM_HOME=')), '')
        stat_fields = (proc / 'stat').read_text().split(') ', 1)[1].split()
        state = stat_fields[0]
        rss_kb = 0
        for line in (proc / 'status').read_text().splitlines():
            if line.startswith('VmRSS:'):
                rss_kb = int(line.split()[1])
                break
        rows.append({
            'pid': int(pid),
            'state': state,
            'rss_kb': rss_kb,
            'home': home,
            'argv': parts,
        })
    except Exception:
        continue
print(json.dumps(rows))
"""


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Audit yggterm daemon processes across hosts.")
    parser.add_argument("hosts", nargs="*", default=["local", "oc", "jojo"])
    return parser.parse_args()


def run_host(host: str) -> dict:
    if host == "local":
        cmd = ["python3", "-c", AUDIT_SNIPPET]
        proc = subprocess.run(cmd, text=True, capture_output=True)
    else:
        # Stream the snippet over stdin so the remote shell never re-parses
        # embedded quotes/newlines from the Python body.
        cmd = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", host, "python3", "-"]
        proc = subprocess.run(cmd, text=True, capture_output=True, input=AUDIT_SNIPPET)
    if proc.returncode != 0:
        return {"host": host, "ok": False, "error": proc.stderr.strip() or proc.stdout.strip()}
    try:
        daemons = json.loads(proc.stdout or "[]")
    except json.JSONDecodeError as error:
        return {"host": host, "ok": False, "error": f"invalid json: {error}"}
    return {
        "host": host,
        "ok": True,
        "daemon_count": len(daemons),
        "rss_kb_total": sum(int(row.get("rss_kb") or 0) for row in daemons),
        "daemons": daemons,
    }


def main() -> int:
    args = parse_args()
    report = [run_host(host) for host in args.hosts]
    print(json.dumps(report, indent=2))
    return 0 if all(item.get("ok") for item in report) else 1


if __name__ == "__main__":
    raise SystemExit(main())
