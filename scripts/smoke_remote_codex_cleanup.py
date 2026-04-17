#!/usr/bin/env python3
import argparse
import json
import shlex
import subprocess
import time
from pathlib import Path


REMOTE_DISCOVER_STATE_SCRIPT = r"""
import json
import sqlite3
import os
from pathlib import Path

sid = {session_id!r}
home = Path.home()
state_path = home / ".yggterm" / "server-state.json"
db_path = home / ".yggterm" / "remote-runtime.db"
runtime_dir = home / ".yggterm" / "sessions" / sid
data = {{
    "live_matches": [],
    "runtime_rows": [],
    "runtime_dir_exists": runtime_dir.exists(),
    "pgrep_matches": [],
}}

if state_path.exists():
    state = json.loads(state_path.read_text())
    for item in state.get("live_sessions") or []:
        key = item.get("key") or ""
        item_id = item.get("id") or ""
        if sid == item_id or sid in key:
            data["live_matches"].append(
                {{
                    "key": key,
                    "id": item_id,
                    "kind": item.get("kind"),
                    "ssh_target": item.get("ssh_target"),
                    "cwd": item.get("cwd"),
                }}
            )

if db_path.exists():
    conn = sqlite3.connect(str(db_path))
    rows = conn.execute(
        "select session_id,machine_key,runtime_kind,state,health,title,cwd,updated_at "
        "from runtime_sessions where session_id = ?",
        (sid,),
    ).fetchall()
    data["runtime_rows"] = [list(row) for row in rows]
    conn.close()

current_pid = os.getpid()
parent_pid = os.getppid()
for proc in Path("/proc").iterdir():
    if not proc.name.isdigit():
        continue
    pid = int(proc.name)
    if pid in (current_pid, parent_pid):
        continue
    try:
        cmdline = (proc / "cmdline").read_bytes().replace(b"\x00", b" ").decode("utf-8", "replace").strip()
    except Exception:
        continue
    if not cmdline or sid not in cmdline:
        continue
    data["pgrep_matches"].append(f"{{pid}} {{cmdline}}")

print(json.dumps(data))
"""

REMOTE_DISCOVER_SAVED_SCRIPT = r"""
import json
from pathlib import Path

def read_session_id(path: Path):
    try:
        with path.open("r", encoding="utf-8", errors="replace") as fh:
            for raw in fh:
                raw = raw.strip()
                if not raw:
                    continue
                try:
                    value = json.loads(raw)
                except Exception:
                    continue
                if value.get("type") != "session_meta":
                    continue
                payload = value.get("payload") or {}
                session_id = payload.get("id")
                cwd = payload.get("cwd")
                if session_id:
                    return session_id, cwd
    except Exception:
        return None
    return None

root = Path.home() / ".codex" / "sessions"
candidates = []
if root.exists():
    for path in sorted(root.rglob("*.jsonl"), key=lambda candidate: candidate.stat().st_mtime, reverse=True):
        identity = read_session_id(path)
        if identity is None:
            continue
        session_id, cwd = identity
        candidates.append(
            {
                "session_id": session_id,
                "cwd": cwd,
                "path": str(path),
                "mtime": path.stat().st_mtime,
            }
        )
        if len(candidates) >= 25:
            break
print(json.dumps(candidates))
"""


def shell_quote(value: str) -> str:
    return shlex.quote(value)


def remote_shell(exec_prefix: str | None, inner: str) -> str:
    if exec_prefix and exec_prefix.strip():
        return f"{exec_prefix.strip()} sh -lc {shell_quote(inner)}"
    return inner


def run_ssh(
    ssh_target: str,
    command: str,
    *,
    exec_prefix: str | None = None,
    timeout_seconds: float = 15.0,
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        [
            "ssh",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=8",
            ssh_target,
            remote_shell(exec_prefix, command),
        ],
        text=True,
        capture_output=True,
        timeout=timeout_seconds,
        check=False,
    )
    if check and completed.returncode != 0:
        raise RuntimeError(
            f"remote command failed on {ssh_target}: rc={completed.returncode} stderr={completed.stderr.strip()!r}"
        )
    return completed


def discover_saved_session(
    ssh_target: str,
    exec_prefix: str | None,
    excluded_ids: set[str],
) -> dict:
    completed = run_ssh(
        ssh_target,
        f"python3 - <<'PY'\n{REMOTE_DISCOVER_SAVED_SCRIPT}\nPY",
        exec_prefix=exec_prefix,
        timeout_seconds=25.0,
        check=True,
    )
    candidates = json.loads(completed.stdout.strip() or "[]")
    for item in candidates:
        if item.get("session_id") not in excluded_ids:
            return item
    if candidates:
        return candidates[0]
    raise AssertionError(
        f"no saved Codex session found on {ssh_target}; cannot prove require-existing runtime cleanup"
    )


def discover_state(
    ssh_target: str, exec_prefix: str | None, session_id: str
) -> dict:
    command = f"python3 - <<'PY'\n{REMOTE_DISCOVER_STATE_SCRIPT.format(session_id=session_id)}\nPY"
    completed = run_ssh(
        ssh_target,
        command,
        exec_prefix=exec_prefix,
        timeout_seconds=20.0,
        check=True,
    )
    return json.loads(completed.stdout.strip() or "{}")


def wait_for_predicate(
    ssh_target: str,
    exec_prefix: str | None,
    session_id: str,
    *,
    predicate,
    timeout_seconds: float,
    label: str,
) -> dict:
    deadline = time.time() + timeout_seconds
    last_state = {}
    while time.time() < deadline:
        last_state = discover_state(ssh_target, exec_prefix, session_id)
        if predicate(last_state):
            return last_state
        time.sleep(0.35)
    raise AssertionError(f"{label} did not settle for {session_id}: {last_state!r}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--ssh-target", required=True)
    parser.add_argument("--exec-prefix")
    parser.add_argument("--cwd", default=None)
    parser.add_argument("--remote-binary", default="$HOME/.yggterm/bin/yggterm")
    parser.add_argument("--out-dir", type=Path, required=True)
    parser.add_argument("--launch-timeout-seconds", type=float, default=4.0)
    args = parser.parse_args()

    args.out_dir.mkdir(parents=True, exist_ok=True)

    state_probe = run_ssh(
        args.ssh_target,
        "python3 - <<'PY'\nimport json, sqlite3\nfrom pathlib import Path\nhome = Path.home()\nstate_path = home/'.yggterm'/'server-state.json'\nlive_ids = []\nif state_path.exists():\n    state = json.loads(state_path.read_text())\n    live_ids = [item.get('id') for item in (state.get('live_sessions') or []) if item.get('id')]\ndb_path = home/'.yggterm'/'remote-runtime.db'\nruntime_ids = []\nif db_path.exists():\n    conn = sqlite3.connect(str(db_path))\n    runtime_ids = [row[0] for row in conn.execute('select session_id from runtime_sessions')]\n    conn.close()\nprint(json.dumps({'live_ids': live_ids, 'runtime_ids': runtime_ids}))\nPY",
        exec_prefix=args.exec_prefix,
        timeout_seconds=15.0,
        check=True,
    )
    state_ids = json.loads(state_probe.stdout.strip() or "{}")
    excluded_ids = set(state_ids.get("live_ids") or []) | set(state_ids.get("runtime_ids") or [])
    saved = discover_saved_session(args.ssh_target, args.exec_prefix, excluded_ids)
    session_id = saved["session_id"]
    cwd = args.cwd or saved.get("cwd") or "$HOME"

    before = discover_state(args.ssh_target, args.exec_prefix, session_id)
    if before.get("live_matches") or before.get("runtime_rows") or before.get("runtime_dir_exists"):
        raise AssertionError(
            f"target session {session_id!r} is already active before launch: {before!r}"
        )

    launch_command = (
        f"exec {args.remote_binary} server remote resume-codex "
        f"{shell_quote(session_id)} {shell_quote(cwd)} --require-existing"
    )
    launch = subprocess.run(
        [
            "timeout",
            str(args.launch_timeout_seconds),
            "ssh",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=8",
            args.ssh_target,
            remote_shell(args.exec_prefix, launch_command),
        ],
        text=True,
        capture_output=True,
        check=False,
    )

    launched = wait_for_predicate(
        args.ssh_target,
        args.exec_prefix,
        session_id,
        predicate=lambda state: bool(state.get("live_matches")) and bool(state.get("runtime_rows")),
        timeout_seconds=15.0,
        label="remote runtime launch",
    )
    if not launched.get("pgrep_matches"):
        raise AssertionError(
            f"session {session_id!r} launched without a discoverable process match: {launched!r}"
        )

    terminate = run_ssh(
        args.ssh_target,
        f"exec {args.remote_binary} server remote terminate-codex {shell_quote(session_id)}",
        exec_prefix=args.exec_prefix,
        timeout_seconds=20.0,
        check=True,
    )
    cleaned = wait_for_predicate(
        args.ssh_target,
        args.exec_prefix,
        session_id,
        predicate=lambda state: (
            not state.get("live_matches")
            and not state.get("runtime_rows")
            and not state.get("runtime_dir_exists")
            and not state.get("pgrep_matches")
        ),
        timeout_seconds=15.0,
        label="remote runtime cleanup",
    )

    summary = {
        "ssh_target": args.ssh_target,
        "exec_prefix": args.exec_prefix,
        "remote_binary": args.remote_binary,
        "saved_session": saved,
        "cwd": cwd,
        "before": before,
        "launch": {
            "returncode": launch.returncode,
            "stdout_tail": launch.stdout[-400:],
            "stderr_tail": launch.stderr[-400:],
        },
        "launched": launched,
        "terminate": {
            "returncode": terminate.returncode,
            "stdout_tail": terminate.stdout[-400:],
            "stderr_tail": terminate.stderr[-400:],
        },
        "cleaned": cleaned,
    }
    (args.out_dir / "summary.json").write_text(json.dumps(summary, indent=2))
    print(json.dumps(summary, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
