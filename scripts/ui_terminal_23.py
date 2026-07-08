#!/usr/bin/env python3
import argparse
import json
import os
import random
import re
import shutil
import shlex
import subprocess
import time
from pathlib import Path


MARKER_BEGIN = "__YGGTERM23_BEGIN__"
MARKER_END = "__YGGTERM23_END__"
SHORT_HASH_RE = re.compile(r"^[0-9a-f]{7,8}$")
CPU_SAMPLE_SNIPPET = r"""
import json
import os
import pathlib
import time

DURATION = float(%DURATION%)
LABEL = %LABEL%
TOKENS = tuple(token.lower() for token in %TOKENS%)
PAGE_SIZE = os.sysconf("SC_PAGE_SIZE")

def read_total_jiffies():
    try:
        line = pathlib.Path("/proc/stat").read_text().splitlines()[0]
        return sum(int(part) for part in line.split()[1:] if part.isdigit())
    except Exception:
        return 0

def cpu_count():
    try:
        return max(
            1,
            sum(
                1
                for line in pathlib.Path("/proc/stat").read_text().splitlines()
                if line.startswith("cpu") and len(line) > 3 and line[3].isdigit()
            ),
        )
    except Exception:
        return 1

def read_comm(pid):
    try:
        return pathlib.Path("/proc", str(pid), "comm").read_text().strip()
    except Exception:
        return ""

def read_cmdline(pid):
    try:
        return (
            pathlib.Path("/proc", str(pid), "cmdline")
            .read_bytes()
            .replace(b"\0", b" ")
            .decode("utf-8", "replace")
            .strip()
        )
    except Exception:
        return ""

def read_environ(pid):
    result = {}
    wanted = {"YGGTERM_HOME", "DISPLAY", "WAYLAND_DISPLAY", "GDK_BACKEND"}
    try:
        for raw in pathlib.Path("/proc", str(pid), "environ").read_bytes().split(b"\0"):
            if raw and b"=" in raw:
                key, value = raw.split(b"=", 1)
                key = key.decode("utf-8", "replace")
                if key in wanted:
                    result[key] = value.decode("utf-8", "replace")
    except Exception:
        pass
    return result

def read_stat(pid):
    try:
        fields = pathlib.Path("/proc", str(pid), "stat").read_text().rsplit(")", 1)[1].split()
        return {
            "ppid": int(fields[1]),
            "jiffies": int(fields[11]) + int(fields[12]),
            "rss_kb": int(fields[21]) * PAGE_SIZE // 1024,
        }
    except Exception:
        return None

def process_matches(comm, cmd):
    comm_lower = comm.lower()
    cmd_lower = cmd.lower()
    if comm_lower.startswith("webkit"):
        return True
    if comm_lower in {
        "yggterm",
        "yggterm-headles",
        "yggterm-headless",
        "htop",
        "top",
        "ssh",
        "codex",
        "node",
    }:
        return True
    return any(
        needle in cmd_lower
        for needle in (
            "/yggterm",
            "yggterm-headless server daemon",
            "codex-session-tui",
            " npx --yes codex-session-tui",
        )
    )

def pids():
    return [int(path.name) for path in pathlib.Path("/proc").iterdir() if path.name.isdigit()]

def snapshot():
    total = read_total_jiffies()
    rows = {}
    for pid in pids():
        stat = read_stat(pid)
        if not stat:
            continue
        comm = read_comm(pid)
        cmd = read_cmdline(pid)
        rows[str(pid)] = {
            "pid": pid,
            "ppid": stat["ppid"],
            "comm": comm,
            "cmd": cmd[:300],
            "jiffies": stat["jiffies"],
            "rss_kb": stat["rss_kb"],
            "matched": process_matches(comm, cmd),
        }
    return total, rows

start_total, start_rows = snapshot()
start_ts_ms = int(time.time() * 1000)
time.sleep(DURATION)
end_total, end_rows = snapshot()
end_ts_ms = int(time.time() * 1000)
cores = cpu_count()
denominator = max(1, end_total - start_total)
rows = []
for pid_text, end_row in end_rows.items():
    start_row = start_rows.get(pid_text)
    start_jiffies = start_row.get("jiffies", end_row["jiffies"]) if start_row else end_row["jiffies"]
    delta = max(0, end_row["jiffies"] - start_jiffies)
    row = dict(end_row)
    row["delta_jiffies"] = delta
    row["cpu_percent"] = round((delta / denominator) * cores * 100.0, 3)
    if row["matched"] or row["cpu_percent"] > 0.0:
        row["env"] = read_environ(row["pid"]) if row["matched"] else {}
        rows.append(row)
rows.sort(key=lambda row: (-row["cpu_percent"], row["comm"], row["pid"]))
matched = [row for row in rows if row["matched"]]
system_top = rows[:20]
print(json.dumps({
    "label": LABEL,
    "duration_sec": DURATION,
    "start_ts_ms": start_ts_ms,
    "end_ts_ms": end_ts_ms,
    "cpu_count": cores,
    "matched_cpu_percent": round(sum(float(row.get("cpu_percent") or 0.0) for row in matched), 3),
    "matched_rss_kb": sum(int(row.get("rss_kb") or 0) for row in matched),
    "matched": matched[:40],
    "system_top": system_top,
    "tokens": TOKENS,
}))
"""


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Create 23 real terminals through app-control, run probe commands, "
            "verify viewport/title/summary state, and remove the created live sessions."
        )
    )
    parser.add_argument("--host", default="local")
    parser.add_argument("--bin", default="./target/debug/yggterm")
    parser.add_argument("--count", type=int, default=23)
    parser.add_argument("--seed", type=int, default=23)
    parser.add_argument("--timeout-ms", type=int, default=8000)
    parser.add_argument("--poll", type=float, default=0.15)
    parser.add_argument("--ready-budget", type=float, default=2.3)
    parser.add_argument("--spawn-budget", type=float, default=0.9)
    parser.add_argument("--summary-budget", type=float, default=12.0)
    parser.add_argument("--launch-local", action="store_true")
    parser.add_argument("--out-dir", default="/tmp/yggterm-terminal-23")
    parser.add_argument("--heavy-count", type=int, default=7)
    parser.add_argument("--restore-pass", action="store_true")
    parser.add_argument("--restore-wait-sec", type=float, default=300.0)
    parser.add_argument("--resource-sample-sec", type=float, default=8.0)
    parser.add_argument("--baseline-max-cpu", type=float, default=8.0)
    parser.add_argument("--active-max-cpu", type=float, default=30.0)
    parser.add_argument("--cooldown-max-cpu", type=float, default=8.0)
    parser.add_argument("--respawn-max-cpu", type=float, default=18.0)
<<<<<<< HEAD
    parser.add_argument("--respawn-burst-max-cpu", type=float, default=35.0)
    parser.add_argument("--respawn-settle-sec", type=float, default=20.0)
=======
>>>>>>> c162185 (Snapshot alpha blur experiment)
    parser.add_argument("--gui-bin", help="GUI-capable yggterm binary for restore relaunch")
    parser.add_argument("--screenshots", action="store_true")
    parser.add_argument("--skip-quirks", action="store_true")
    parser.add_argument("--skip-launcher-preflight", action="store_true")
    return parser.parse_args()


def run_process(argv: list[str], *, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(argv, check=check, text=True, capture_output=True)


def run_control(host: str, command: str, *, check: bool = True) -> subprocess.CompletedProcess:
    if host == "local":
        return run_process(["bash", "-lc", command], check=check)
    return run_process(["ssh", host, command], check=check)


def run_json(host: str, command: str) -> dict:
    result = run_control(host, command, check=False)
    stdout = result.stdout.strip()
    stderr = result.stderr.strip()
    if result.returncode != 0 and not stdout:
        raise RuntimeError(
            f"command failed rc={result.returncode}: {command}\nstderr:\n{stderr or '<empty>'}"
        )
    try:
        return json.loads(stdout)
    except json.JSONDecodeError as error:
        raise RuntimeError(
            f"invalid json rc={result.returncode}: {command}\nstdout:\n{stdout or '<empty>'}\nstderr:\n{stderr or '<empty>'}"
        ) from error


def run_capture(argv: list[str]) -> tuple[int, str, str]:
    result = subprocess.run(argv, check=False, text=True, capture_output=True)
    return result.returncode, result.stdout, result.stderr


def quote(value: str) -> str:
    return shlex.quote(value)


def direct_install_launcher_preflight(host: str, binary: str, out_dir: Path) -> dict:
    snippet = r"""
import json
import os
import pathlib
import sys

MARKER = "yggterm-direct-launcher-v3"
binary_arg = sys.argv[1]
home = pathlib.Path.home()
root = pathlib.Path(os.environ.get("YGGTERM_DIRECT_INSTALL_ROOT") or home / ".local/share/yggterm/direct")
state_path = root / "install-state.json"

def info_for(path_text, expected):
    path = pathlib.Path(path_text).expanduser()
    exists = path.exists() or path.is_symlink()
    resolved = ""
    marker = False
    text_sample = ""
    if exists:
        try:
            resolved = str(path.resolve(strict=False))
        except Exception:
            resolved = ""
        try:
            text_sample = path.read_text(encoding="utf-8", errors="replace")[:4096]
            marker = MARKER in text_sample
        except Exception:
            text_sample = ""
    direct_versions = f"{root}/versions/"
    expected_text = str(expected) if expected else ""
    stale_direct = bool(resolved and direct_versions in resolved and expected_text and resolved != expected_text)
    acceptable = (not exists) or marker or (expected_text and resolved == expected_text)
    return {
        "path": str(path),
        "exists": exists,
        "is_symlink": path.is_symlink(),
        "resolved": resolved,
        "expected": expected_text,
        "has_launcher_marker": marker,
        "stale_direct_version": stale_direct,
        "acceptable": acceptable and not stale_direct,
    }

payload = {
    "checked": False,
    "ok": True,
    "binary_arg": binary_arg,
    "install_root": str(root),
    "state_path": str(state_path),
    "failures": [],
    "entries": [],
}
if state_path.is_file():
    state = json.loads(state_path.read_text(encoding="utf-8"))
    active = pathlib.Path(str(state.get("active_executable") or "")).expanduser()
    headless = active.parent / "yggterm-headless" if active else pathlib.Path("")
    payload["checked"] = True
    payload["active_version"] = state.get("active_version")
    payload["active_executable"] = str(active)
    checks = [
        (binary_arg, active),
        (home / ".local/bin/yggterm", active),
        (home / ".local/bin/yggterm-headless", headless),
    ]
    seen = set()
    for path, expected in checks:
        path_text = str(path)
        if path_text in seen:
            continue
        seen.add(path_text)
        entry = info_for(path_text, expected)
        payload["entries"].append(entry)
        if entry["exists"] and not entry["acceptable"]:
            payload["failures"].append(entry)
    payload["ok"] = not payload["failures"]
print(json.dumps(payload))
"""
    command = f"python3 -c {quote(snippet)} {quote(binary)}"
    result = run_control(host, command, check=False)
    stdout = result.stdout.strip()
    try:
        payload = json.loads(stdout) if stdout else {"ok": False, "failures": ["empty output"]}
    except json.JSONDecodeError:
        payload = {
            "ok": False,
            "failures": ["invalid json"],
            "stdout": stdout,
            "stderr": result.stderr,
            "returncode": result.returncode,
        }
    payload["returncode"] = result.returncode
    payload["stderr"] = result.stderr.strip()
    write_json(out_dir / "launcher-preflight.json", payload)
    if result.returncode != 0 or not payload.get("ok", False):
        raise RuntimeError(f"direct install launcher preflight failed: {payload}")
    return payload


def local_yggterm_home() -> Path:
    override = os.environ.get("YGGTERM_HOME", "").strip()
    if override:
        return Path(override).expanduser()
    return Path.home() / ".yggterm"


def local_yggterm_path(*parts: str) -> Path:
    return local_yggterm_home().joinpath(*parts)


def local_x11_window_count(title: str = "Yggterm") -> int:
    display = os.environ.get("DISPLAY", "").strip()
    if not display or shutil.which("xwininfo") is None:
        return 0
    result = subprocess.run(
        ["xwininfo", "-root", "-tree"],
        check=False,
        text=True,
        capture_output=True,
        env={**os.environ, "DISPLAY": display},
    )
    if result.returncode != 0:
        return 0
    marker = f'"{title}"'
    return sum(1 for line in result.stdout.splitlines() if marker in line)


def app_state(host: str, binary: str, timeout_ms: int) -> dict:
    payload = run_json(host, f"{quote(binary)} server app state --timeout-ms {timeout_ms}")
    return payload.get("data") or {}


def app_create_terminal(
    host: str,
    binary: str,
    timeout_ms: int,
    machine_key: str | None,
    cwd: str | None,
) -> dict:
    parts = [quote(binary), "server", "app", "terminal", "new"]
    if machine_key:
        parts.extend(["--machine-key", quote(machine_key)])
    if cwd:
        parts.extend(["--cwd", quote(cwd)])
    parts.extend(["--timeout-ms", str(timeout_ms)])
    return run_json(host, " ".join(parts))


def app_send_terminal_input(
    host: str,
    binary: str,
    timeout_ms: int,
    session_path: str,
    data: str,
) -> dict:
    command = (
        f"{quote(binary)} server app terminal send {quote(session_path)} "
        f"--data {quote(data)} --timeout-ms {timeout_ms}"
    )
    return run_json(host, command)


def app_remove_session(host: str, binary: str, timeout_ms: int, session_path: str) -> dict:
    command = (
        f"{quote(binary)} server app session remove {quote(session_path)} "
        f"--timeout-ms {timeout_ms}"
    )
    return run_json(host, command)


def app_keep_session(
    host: str,
    binary: str,
    timeout_ms: int,
    session_path: str,
    keep_alive: bool,
) -> dict:
    action = "keep" if keep_alive else "unkeep"
    command = (
        f"{quote(binary)} server app terminal {action} {quote(session_path)} "
        f"--timeout-ms {timeout_ms}"
    )
    return run_json(host, command)


def app_open_session(
    host: str,
    binary: str,
    timeout_ms: int,
    session_path: str,
) -> dict:
    command = (
        f"{quote(binary)} server app open {quote(session_path)} --view terminal "
        f"--timeout-ms {timeout_ms}"
    )
    return run_json(host, command)


def app_probe_terminal(
    host: str,
    binary: str,
    timeout_ms: int,
    session_path: str,
    probe: str,
    *args: str,
) -> dict:
    command = (
        f"{quote(binary)} server app terminal {probe} {quote(session_path)} "
        + " ".join(quote(arg) for arg in args)
        + f" --timeout-ms {timeout_ms}"
    )
    return run_json(host, command)


def app_chrome_hover(host: str, binary: str, timeout_ms: int, active: bool) -> dict:
    state = "on" if active else "off"
    return run_json(
        host,
        f"{quote(binary)} server app chrome-hover {state} --timeout-ms {timeout_ms}",
    )


def app_screenshot(host: str, binary: str, timeout_ms: int, path: str) -> dict:
    return run_json(
        host,
        f"{quote(binary)} server app screenshot {quote(path)} --timeout-ms {timeout_ms}",
    )


def app_close_preserve(host: str, binary: str, timeout_ms: int) -> dict:
    return run_json(
        host,
        f"{quote(binary)} server app close --preserve-live-sessions --timeout-ms {timeout_ms}",
    )


def app_launch(
    host: str,
    binary: str,
    timeout_ms: int,
) -> dict:
    return run_json(
        host,
        f"{quote(binary)} server app launch --wait-settled --allow-multi-window "
        f"--skip-active-exec-handoff "
        f"--timeout-ms {timeout_ms}",
    )


def headless_binary_for(binary: str) -> str:
    path = Path(binary)
    name = path.name
    if name == "yggterm-headless":
        return binary
    if name == "yggterm":
        return str(path.with_name("yggterm-headless"))
    return binary.replace("yggterm", "yggterm-headless")


def daemon_server_list(host: str, binary: str) -> dict:
    headless = headless_binary_for(binary)
    result = run_control(
        host,
        f"{quote(headless)} server monitor --scenario server-list",
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(
            "server-list failed "
            f"rc={result.returncode}: {result.stderr.strip() or result.stdout.strip()}"
        )
    for line in reversed(result.stdout.splitlines()):
        line = line.strip()
        if not line:
            continue
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("event", {}).get("kind") == "result":
            return event.get("data") or {}
    raise RuntimeError(f"server-list did not return a result event: {result.stdout[-2000:]}")


def daemon_owner_failures(report: dict) -> list[str]:
    owners_by_key: dict[str, list[str]] = {}
    for server in report.get("servers") or []:
        endpoint = ((server.get("endpoint") or {}).get("path")) or str(server.get("endpoint"))
        label = f"{server.get('server_version')} pid {server.get('server_pid')} {endpoint}"
        for key in server.get("owned_terminal_session_keys") or []:
            owners_by_key.setdefault(str(key), []).append(label)
    failures = []
    for key, owners in sorted(owners_by_key.items()):
        unique_owners = sorted(set(owners))
        if len(unique_owners) > 1:
            failures.append(
                f"runtime key {key} is directly owned by multiple daemons: "
                + " | ".join(unique_owners)
            )
    return failures


def server_inventory(host: str) -> dict:
    if host == "local":
        path = local_yggterm_path("server-state.json")
        return json.loads(path.read_text(encoding="utf-8"))
    result = run_control(host, "cat ~/.yggterm/server-state.json")
    return json.loads(result.stdout)


def trace_events_since(host: str, start_ms: int, tail_lines: int = 4000) -> list[dict]:
    if host == "local":
        path = local_yggterm_path("event-trace.jsonl")
        lines = path.read_text(encoding="utf-8").splitlines()[-tail_lines:]
    else:
        result = run_control(host, f"tail -n {tail_lines} ~/.yggterm/event-trace.jsonl")
        lines = result.stdout.splitlines()
    events = []
    for line in lines:
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (event.get("ts_ms") or 0) >= start_ms:
            events.append(event)
    return events


def latest_window_spawn_event_for_pid(host: str, pid: int, start_ms: int) -> dict | None:
    if host == "local":
        path = local_yggterm_path("event-trace.jsonl")
        if not path.exists():
            return None
        for line in reversed(path.read_text(encoding="utf-8").splitlines()):
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if (event.get("ts_ms") or 0) < start_ms:
                break
            if (
                event.get("pid") == pid
                and event.get("category") == "startup"
                and event.get("name") == "window_spawned"
            ):
                return event
        return None
    for event in reversed(trace_events_since(host, start_ms)):
        if (
            event.get("pid") == pid
            and event.get("category") == "startup"
            and event.get("name") == "window_spawned"
        ):
            return event
    return None


def local_client_instance_dir() -> Path:
    return local_yggterm_path("client-instances")


def clear_local_app_control_files() -> None:
    home = local_yggterm_home()
    for rel in ("app-control-requests", "app-control-responses"):
        root = home / rel
        if not root.is_dir():
            continue
        for path in root.glob("*.json"):
            try:
                path.unlink()
            except FileNotFoundError:
                pass


def kill_local_clients() -> None:
    instances_root = local_client_instance_dir()
    if instances_root.is_dir():
        for path in instances_root.glob("*/*.json"):
            try:
                record = json.loads(path.read_text(encoding="utf-8"))
            except Exception:
                continue
            pid = int(record.get("pid") or 0)
            if pid > 0:
                try:
                    os.kill(pid, 15)
                except Exception:
                    pass
        time.sleep(0.4)
        for path in instances_root.glob("*/*.json"):
            try:
                record = json.loads(path.read_text(encoding="utf-8"))
            except Exception:
                continue
            pid = int(record.get("pid") or 0)
            if pid > 0:
                try:
                    os.kill(pid, 9)
                except Exception:
                    pass
    subprocess.run(
        ["bash", "-lc", "pkill -f 'yggterm server daemon' || true"],
        check=False,
        capture_output=True,
        text=True,
    )
    clear_local_app_control_files()


def launch_local_client(binary: str, timeout_s: float = 3.0) -> tuple[subprocess.Popen, dict]:
    binary_path = str(Path(binary).resolve())
    kill_local_clients()
    env = os.environ.copy()
    env.setdefault("DISPLAY", ":10.0")
    env["YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF"] = "1"
    baseline_window_count = local_x11_window_count()
    stdout = open("/tmp/yggterm-terminal-23-launch.out", "w", encoding="utf-8")
    stderr = open("/tmp/yggterm-terminal-23-launch.err", "w", encoding="utf-8")
    start_ms = int(time.time() * 1000)
    proc = subprocess.Popen(
        [binary_path],
        stdout=stdout,
        stderr=stderr,
        env=env,
        cwd=str(Path(binary).resolve().parent.parent.parent),
    )
    deadline = time.monotonic() + timeout_s
    while time.monotonic() < deadline:
        if local_x11_window_count() > baseline_window_count:
            return proc, {
                "category": "startup",
                "name": "window_spawned",
                "payload": {
                    "elapsed_ms": int(time.time() * 1000) - start_ms,
                    "source": "x11_root_tree",
                },
            }
        event = latest_window_spawn_event_for_pid("local", proc.pid, start_ms)
        if event is not None:
            return proc, event
        if proc.poll() is not None:
            break
        time.sleep(0.05)
    raise RuntimeError(
        f"local launch did not emit window_spawned within {timeout_s:.2f}s for pid {proc.pid}"
    )


def wait_until(label: str, timeout_s: float, poll_s: float, predicate):
    start = time.monotonic()
    last_error = None
    while time.monotonic() - start <= timeout_s:
        try:
            return time.monotonic() - start, predicate()
        except Exception as error:  # noqa: BLE001
            last_error = error
            remaining = timeout_s - (time.monotonic() - start)
            if remaining <= 0:
                break
            time.sleep(min(poll_s, max(0.0, remaining)))
    try:
        return min(time.monotonic() - start, timeout_s), predicate()
    except Exception as error:  # noqa: BLE001
        last_error = error
    raise RuntimeError(f"{label} timed out after {timeout_s:.2f}s: {last_error}")


def wait_for_window(host: str, binary: str, timeout_ms: int) -> dict:
    def _probe() -> dict:
        state = app_state(host, binary, timeout_ms)
        window = state.get("window") or {}
        if not window.get("visible"):
            raise RuntimeError("window not visible")
        return state

    return wait_until("window visible", 8.0, 0.25, _probe)[1]


def viewport_terminal_ready(state: dict, session_path: str) -> bool:
    viewport = state.get("viewport") or {}
    return (
        viewport.get("active_view_mode") == "Terminal"
        and viewport.get("active_session_path") == session_path
        and bool(viewport.get("ready"))
    )


def terminal_attach_ready_seen(
    host: str,
    session_path: str,
    start_ms: int,
    deadline_ms: int,
) -> bool:
    for event in reversed(trace_events_since(host, start_ms)):
        ts_ms = event.get("ts_ms") or 0
        if ts_ms > deadline_ms:
            continue
        if (
            event.get("category") == "terminal_mount"
            and event.get("name") == "attach_ready"
            and ((event.get("payload") or {}).get("session_path") == session_path)
        ):
            return True
    return False


def active_terminal_text(state: dict) -> str:
    viewport = state.get("viewport") or {}
    hosts = viewport.get("active_terminal_hosts") or []
    if not hosts:
        return ""
    chunks: list[str] = []
    for host in hosts:
        for key in ("text_sample", "text_tail", "buffer_text_sample", "cursor_line_text"):
            value = str(host.get(key) or "")
            if value and value not in chunks:
                chunks.append(value)
    return "\n".join(chunks)


def titlebar_matches_viewport(state: dict) -> bool:
    dom = state.get("dom") or {}
    viewport = state.get("viewport") or {}
    titlebar = viewport.get("titlebar") or {}
    active_title = (viewport.get("active_title") or "").strip()
    active_summary = (viewport.get("active_summary") or "").strip()
    title_text = (titlebar.get("title_text") or "").strip()
    summary_text = (titlebar.get("summary_text") or "").strip()
    if (
        dom.get("snapshot_mode") == "terminal-fallback"
        and not title_text
        and not summary_text
    ):
        return True
    if active_title and title_text != active_title:
        return False
    if active_summary and titlebar.get("menu_open") and summary_text != active_summary:
        return False
    if active_summary and summary_text and summary_text != active_summary:
        return False
    return True


def output_matches_cwd(text: str, expected_cwd: str | None) -> bool:
    expected = (expected_cwd or "").strip()
    if not expected:
        return True
    lines = [line.strip() for line in text.splitlines()]
    if expected in lines:
        return True
    squashed_expected = "".join(expected.split())
    squashed_text = "".join(text.split())
    return squashed_expected in squashed_text


def title_is_good(value: str | None) -> bool:
    title = (value or "").strip()
    if not title:
        return False
    lowered = title.lower()
    if lowered in {"resuming terminal...", "resuming terminal…", "connecting..."}:
        return False
    if SHORT_HASH_RE.fullmatch(title):
        return False
    return True


def local_dir_exists(path: str) -> bool:
    target = Path(path)
    return target.is_dir() and os.access(target, os.X_OK)


def remote_dir_exists(ssh_target: str, path: str) -> bool:
    rc, _, _ = run_capture(
        [
            "ssh",
            ssh_target,
            f"cd {quote(path)} >/dev/null 2>&1",
        ]
    )
    return rc == 0


def remote_resolve_cwd(ssh_target: str, path: str) -> str | None:
    command = (
        f"p={quote(path)}; "
        'while [ -n "$p" ]; do '
        'if [ -d "$p" ] && cd "$p" >/dev/null 2>&1; then pwd -P; exit 0; fi; '
        'if [ "$p" = "/" ]; then break; fi; '
        'next=$(dirname -- "$p"); '
        'if [ "$next" = "$p" ]; then break; fi; '
        'p="$next"; '
        "done; "
        'if [ -n "$HOME" ] && [ -d "$HOME" ] && cd "$HOME" >/dev/null 2>&1; then pwd -P; fi'
    )
    rc, stdout, _ = run_capture(["ssh", ssh_target, command])
    if rc != 0:
        return None
    resolved = stdout.strip()
    return resolved or None


def choose_terminal_targets(inventory: dict, rng: random.Random, count: int) -> list[dict]:
    candidates: list[dict] = []
    seen: set[tuple[str | None, str]] = set()
    machine_targets = {
        (machine.get("machine_key") or "").strip(): (machine.get("ssh_target") or "").strip()
        for machine in (inventory.get("remote_machines") or [])
    }

    for session in inventory.get("stored_sessions") or []:
        cwd = (session.get("cwd") or "").strip()
        path = (session.get("path") or "").strip()
        if not cwd or path.startswith("remote-session://"):
            continue
        key = (None, cwd)
        if key in seen:
            continue
        seen.add(key)
        candidates.append(
            {
                "machine_key": None,
                "cwd": cwd,
                "label": f"local:{cwd}",
                "cwd_verified": False,
            }
        )

    for machine in inventory.get("remote_machines") or []:
        machine_key = (machine.get("machine_key") or "").strip()
        if not machine_key:
            continue
        ssh_target = machine_targets.get(machine_key) or ""
        if not ssh_target:
            continue
        for session in machine.get("sessions") or []:
            cwd = (session.get("cwd") or "").strip()
            if not cwd:
                continue
            key = (machine_key, cwd)
            if key in seen:
                continue
            seen.add(key)
            candidates.append(
                {
                    "machine_key": machine_key,
                    "cwd": cwd,
                    "label": f"{machine_key}:{cwd}",
                    "cwd_verified": False,
                }
            )

    if not candidates:
        raise RuntimeError("no machine/cwd targets available for terminal-23 test")
    rng.shuffle(candidates)
    dir_exists_cache: dict[tuple[str | None, str], bool] = {}
    resolved_remote_cwds: dict[tuple[str, str], str | None] = {}
    chosen: list[dict] = []
    chosen_keys: set[tuple[str | None, str]] = set()
    for candidate in candidates:
        machine_key = candidate.get("machine_key")
        cwd = candidate.get("cwd") or ""
        key = (machine_key, cwd)
        if machine_key:
            ssh_target = machine_targets.get(machine_key) or ""
            resolved = resolved_remote_cwds.setdefault(
                (machine_key, cwd),
                remote_resolve_cwd(ssh_target, cwd) if ssh_target else None,
            )
            if not resolved:
                continue
            candidate = dict(candidate)
            candidate["cwd"] = resolved
            candidate["cwd_verified"] = resolved == cwd
            candidate["label"] = f"{machine_key}:{resolved}"
            if resolved != cwd:
                candidate["requested_cwd"] = cwd
            key = (machine_key, resolved)
        else:
            exists = dir_exists_cache.setdefault(key, local_dir_exists(cwd))
            if not exists:
                continue
        if key in chosen_keys:
            continue
        chosen_keys.add(key)
        chosen.append(candidate)
        if len(chosen) == count:
            return chosen
    if not chosen:
        raise RuntimeError("no existing local/remote cwd targets available for terminal-23 test")
    while len(chosen) < count:
        chosen.append(rng.choice(chosen))
    return chosen


def write_json(path: Path, payload: dict) -> str:
    path.write_text(json.dumps(payload, indent=2), encoding="utf-8")
    return str(path)


def sample_resources(host: str, label: str, duration_s: float, out_dir: Path) -> dict:
    if duration_s <= 0:
        return {"label": label, "skipped": True, "reason": "duration <= 0"}
    snippet = (
        CPU_SAMPLE_SNIPPET.replace("%DURATION%", repr(float(duration_s)))
        .replace("%LABEL%", repr(label))
        .replace(
            "%TOKENS%",
            repr(
                [
                    "yggterm",
                    "webkit",
                    "htop",
                    "codex-session-tui",
                    "codex",
                    "node",
                    "ssh",
                ]
            ),
        )
    )
    if host == "local":
        result = run_process(["python3", "-c", snippet], check=True)
    else:
        result = subprocess.run(
            ["ssh", host, "python3", "-"],
            check=True,
            text=True,
            capture_output=True,
            input=snippet,
        )
    payload = json.loads(result.stdout)
    write_json(out_dir / f"resources-{label}.json", payload)
    return payload


def gui_binary_for_restore(args: argparse.Namespace) -> str:
    if args.gui_bin:
        return args.gui_bin
    if "yggterm-headless" in args.bin:
        return args.bin.replace("yggterm-headless", "yggterm")
    return args.bin


def heavy_workload_command() -> str:
    return (
        "if command -v python3 >/dev/null 2>&1 && python3 - <<'PY'\n"
        "import curses\n"
        "PY\n"
        "then\n"
        "python3 - <<'PY'\n"
        "import curses\n"
        "import time\n"
        "\n"
        "def main(stdscr):\n"
        "    try:\n"
        "        curses.curs_set(0)\n"
        "    except Exception:\n"
        "        pass\n"
        "    stdscr.nodelay(True)\n"
        "    for frame in range(4500):\n"
        "        rows, cols = stdscr.getmaxyx()\n"
        "        width = max(20, cols - 1)\n"
        "        bar_width = max(8, min(42, width - 14))\n"
        "        filled = 1 + (frame % bar_width)\n"
        "        mem_bar = '|' * filled + ' ' * (bar_width - filled)\n"
        "        stdscr.erase()\n"
        "        stdscr.addnstr(0, 0, f'YGGTERM TUI SMOKE frame {frame}', width)\n"
        "        if rows > 1:\n"
        "            stdscr.addnstr(1, 0, 'Tasks: smoke heavy terminal', width)\n"
        "        if rows > 2:\n"
        "            stdscr.addnstr(2, 0, f'Mem[{mem_bar}] {int((filled / bar_width) * 100):02d}%', width)\n"
        "        if rows > 3:\n"
        "            stdscr.addnstr(3, 0, 'F1Help F2Setup F10Quit', width)\n"
        "        stdscr.refresh()\n"
        "        ch = stdscr.getch()\n"
        "        if ch in (ord('q'), ord('Q'), 3):\n"
        "            break\n"
        "        time.sleep(0.2)\n"
        "\n"
        "curses.wrapper(main)\n"
        "PY\n"
        "elif command -v htop >/dev/null 2>&1; then "
        "htop; "
        "elif command -v top >/dev/null 2>&1; then "
        "top; "
        "elif command -v npx >/dev/null 2>&1; then "
        "npx --yes --prefer-offline codex-session-tui; "
        "else printf 'no tui workload available\\n'; sleep 30; fi\n"
    )


def ordinary_probe_command() -> str:
    return (
        f"printf '{MARKER_BEGIN}\\n'; "
        "uname -a || true; "
        "pwd; "
        "(free -h || vm_stat || sysctl vm.swapusage || true); "
        "(ls -1A | head -12) || true; "
        f"printf '{MARKER_END}\\n'\n"
    )


def terminal_text_looks_like_tui(text: str) -> bool:
    lowered = text.lower()
    if any(
        marker in lowered
        for marker in (
            "load average",
            "yggterm tui smoke",
            "tasks:",
            "%cpu",
            "mem[",
            "swp[",
            "uptime:",
            "f1help",
            "mib mem",
            "kib mem",
            "gib mem",
            "browser [",
            "preview (chat)",
            "no session selected",
            "session selected",
        )
    ):
        return True
    htop_bar_rows = re.findall(r"(?:^|\s)\d+\[[|#:/\\*.\-=\s]{2,}\d+(?:\.\d+)?%", text)
    return len(htop_bar_rows) >= 2


def stop_heavy_workload(host: str, binary: str, timeout_ms: int, session_path: str) -> dict:
    stopped: dict[str, object] = {}
    try:
        stopped["quit"] = app_send_terminal_input(host, binary, timeout_ms, session_path, "q")
        time.sleep(0.25)
    except Exception as error:  # noqa: BLE001
        stopped["quit_error"] = str(error)
    try:
        stopped["ctrl_c"] = app_send_terminal_input(host, binary, timeout_ms, session_path, "\x03")
    except Exception as error:  # noqa: BLE001
        stopped["ctrl_c_error"] = str(error)
    return stopped


def _resource_failure(sample: dict, max_cpu: float) -> str | None:
    cpu = float(sample.get("matched_cpu_percent") or 0.0)
    if cpu > max_cpu:
        return f"{sample.get('label')} matched CPU {cpu:.3f}% exceeded budget {max_cpu:.3f}%"
    return None


def main() -> int:
    args = parse_args()
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)

    launch = None
    launch_event = None
    if args.host == "local" and args.launch_local:
        launch, launch_event = launch_local_client(args.bin)

    resources: dict[str, dict] = {}
    resource_failures: list[str] = []
<<<<<<< HEAD
    daemon_reports: dict[str, dict] = {}
    daemon_failures: list[str] = []
=======
>>>>>>> c162185 (Snapshot alpha blur experiment)
    launcher_preflight = None
    if not args.skip_launcher_preflight:
        launcher_preflight = direct_install_launcher_preflight(args.host, args.bin, out_dir)
    resources["baseline"] = sample_resources(
        args.host,
        "baseline",
        args.resource_sample_sec,
        out_dir,
    )
    baseline_failure = _resource_failure(resources["baseline"], args.baseline_max_cpu)
    if baseline_failure:
        resource_failures.append(baseline_failure)
    baseline_state = wait_for_window(args.host, args.bin, args.timeout_ms)
    try:
        daemon_reports["baseline"] = daemon_server_list(args.host, args.bin)
        daemon_failures.extend(
            f"baseline: {failure}"
            for failure in daemon_owner_failures(daemon_reports["baseline"])
        )
    except Exception as error:  # noqa: BLE001
        daemon_failures.append(f"baseline server-list failed: {error}")
    baseline_notifications = ((baseline_state.get("shell") or {}).get("notifications_count")) or 0
    inventory = server_inventory(args.host)
    rng = random.Random(args.seed)
    targets = choose_terminal_targets(inventory, rng, args.count)
    heavy_count = min(max(args.heavy_count, 0), len(targets))
    heavy_indices = set(rng.sample(range(len(targets)), heavy_count)) if heavy_count else set()
    results: list[dict] = []
    keep_alive_results: list[dict] = []
    restore_results: list[dict] = []
    quirk_results: dict[str, object] = {}

    for index, target in enumerate(targets):
        started_ms = int(time.time() * 1000)
        workload_kind = "heavy" if index in heavy_indices else "ordinary"
        try:
            create = app_create_terminal(
                args.host,
                args.bin,
                args.timeout_ms,
                target.get("machine_key"),
                target.get("cwd"),
            )
        except Exception as error:  # noqa: BLE001
            entry = {
                "target": target,
                "error": str(error),
            }
            results.append(entry)
            continue
        created_path = ((create.get("data") or {}).get("active_session_path")) or None
        entry = {
            "target": target,
            "create": create,
            "created_session_path": created_path,
            "workload_kind": workload_kind,
        }
        if not created_path:
            entry["error"] = "create terminal did not return active_session_path"
            results.append(entry)
            continue

        try:
            ready_started_ms = int(time.time() * 1000)
            ready_deadline_ms = ready_started_ms + int(args.ready_budget * 1000)
            ready_elapsed, ready_state = wait_until(
                f"terminal ready {created_path}",
                args.ready_budget,
                args.poll,
                lambda: _require_terminal_ready(
                    app_state(args.host, args.bin, args.timeout_ms),
                    created_path,
                    args.host,
                    ready_started_ms,
                    ready_deadline_ms,
                ),
            )
            entry["ready_elapsed_s"] = round(ready_elapsed, 3)
            entry["ready_within_budget"] = ready_elapsed <= args.ready_budget
            entry["attach_ready_before_deadline"] = terminal_attach_ready_seen(
                args.host,
                created_path,
                ready_started_ms,
                ready_deadline_ms,
            )
        except Exception as error:  # noqa: BLE001
            state = app_state(args.host, args.bin, args.timeout_ms)
            entry["error"] = str(error)
            entry["attach_ready_before_deadline"] = terminal_attach_ready_seen(
                args.host,
                created_path,
                ready_started_ms,
                ready_deadline_ms,
            )
            entry["state_dump"] = write_json(out_dir / f"terminal-{index:02d}-ready-failure.json", state)
            results.append(entry)
            try:
                entry["remove"] = app_remove_session(args.host, args.bin, args.timeout_ms, created_path)
            except Exception as remove_error:  # noqa: BLE001
                entry["remove_error"] = str(remove_error)
            continue

        send = app_send_terminal_input(
            args.host,
            args.bin,
            args.timeout_ms,
            created_path,
            heavy_workload_command() if workload_kind == "heavy" else ordinary_probe_command(),
        )
        entry["send"] = send

        try:
            if workload_kind == "heavy":
                output_elapsed, output_state = wait_until(
                    f"heavy terminal TUI output {created_path}",
                    args.summary_budget,
                    args.poll,
                    lambda: _require_terminal_tui(
                        app_state(args.host, args.bin, args.timeout_ms),
                        created_path,
                    ),
                )
                entry["output_contains_markers"] = True
                entry["heavy_tui_visible"] = True
            else:
                output_elapsed, output_state = wait_until(
                    f"terminal output {created_path}",
                    args.summary_budget,
                    args.poll,
                    lambda: _require_terminal_markers(
                        app_state(args.host, args.bin, args.timeout_ms),
                        created_path,
                    ),
                )
                entry["output_contains_markers"] = True
                entry["heavy_tui_visible"] = None
            entry["output_elapsed_s"] = round(output_elapsed, 3)
        except Exception as error:  # noqa: BLE001
            output_state = app_state(args.host, args.bin, args.timeout_ms)
            entry["error"] = str(error)
            entry["state_dump"] = write_json(out_dir / f"terminal-{index:02d}-output-failure.json", output_state)
            results.append(entry)
            try:
                if workload_kind == "heavy":
                    entry["stop_heavy_after_failure"] = stop_heavy_workload(
                        args.host,
                        args.bin,
                        args.timeout_ms,
                        created_path,
                    )
                entry["remove"] = app_remove_session(args.host, args.bin, args.timeout_ms, created_path)
            except Exception as remove_error:  # noqa: BLE001
                entry["remove_error"] = str(remove_error)
            continue

        viewport = output_state.get("viewport") or {}
        titlebar = viewport.get("titlebar") or {}
        notifications = (output_state.get("shell") or {}).get("notifications_count") or 0
        entry["active_title"] = viewport.get("active_title")
        entry["active_summary"] = viewport.get("active_summary")
        entry["titlebar_title_text"] = titlebar.get("title_text")
        entry["titlebar_summary_text"] = titlebar.get("summary_text")
        entry["titlebar_button_tooltip"] = titlebar.get("button_tooltip")
        entry["titlebar_menu_open"] = titlebar.get("menu_open")
        entry["notification_count"] = notifications
        entry["notification_delta"] = max(0, notifications - baseline_notifications)
        entry["terminal_text_sample"] = active_terminal_text(output_state)
        entry["state_dump"] = write_json(out_dir / f"terminal-{index:02d}.json", output_state)
        entry["title_present"] = title_is_good(viewport.get("active_title"))
        entry["summary_present"] = bool((viewport.get("active_summary") or "").strip())
        entry["titlebar_matches_viewport"] = titlebar_matches_viewport(output_state)
        entry["cwd_matches"] = output_matches_cwd(
            entry["terminal_text_sample"],
            target.get("cwd") if workload_kind == "ordinary" else None,
        )
        entry["notification_noise"] = notifications > baseline_notifications

        results.append(entry)

    resources["active_workload"] = sample_resources(
        args.host,
        "active-workload",
        args.resource_sample_sec,
        out_dir,
    )
    active_failure = _resource_failure(resources["active_workload"], args.active_max_cpu)
    if active_failure:
        resource_failures.append(active_failure)

    created_results = [
        item
        for item in results
        if item.get("created_session_path") and not item.get("remove")
    ]
    if not args.skip_quirks and created_results:
        quirk_path = str(created_results[0]["created_session_path"])
        try:
            quirk_results["chrome_hover_on"] = app_chrome_hover(
                args.host,
                args.bin,
                args.timeout_ms,
                True,
            )
            if args.screenshots:
                quirk_results["chrome_hover_screenshot"] = app_screenshot(
                    args.host,
                    args.bin,
                    args.timeout_ms,
                    f"{out_dir}/chrome-hover.png",
                )
            quirk_results["chrome_hover_state"] = app_state(args.host, args.bin, args.timeout_ms)
            hover_shell = (quirk_results["chrome_hover_state"] or {}).get("shell") or {}
            hover_dom = (quirk_results["chrome_hover_state"] or {}).get("dom") or {}
            hover_failures = []
<<<<<<< HEAD
            live_blur_supported = bool(hover_shell.get("live_blur_supported"))
            compositor_blur_active = bool(hover_shell.get("compositor_blur_active"))
            css_blur_enabled = bool(hover_shell.get("css_backdrop_filter_enabled"))
            try:
                material_blur_px = float(hover_shell.get("material_blur_px") or 0.0)
            except Exception:
                material_blur_px = 0.0
            if (
                live_blur_supported
                and hover_shell.get("transparent_window")
                and not compositor_blur_active
            ):
                hover_failures.append("native compositor blur inactive while transparent hover chrome is visible")
            shell_filter = str(hover_dom.get("shell_frame_backdrop_filter") or "")
            if compositor_blur_active and shell_filter not in {"", "none"}:
                hover_failures.append(f"native compositor path mixed CSS backdrop filter {shell_filter!r}")
            if not live_blur_supported:
                if compositor_blur_active:
                    hover_failures.append("native compositor blur active in stable no-blur profile")
                if css_blur_enabled:
                    hover_failures.append("CSS backdrop blur active in stable no-blur profile")
                if material_blur_px != 0.0:
                    hover_failures.append(
                        f"stable no-blur profile reported material blur {material_blur_px:.1f}px"
                    )
                if shell_filter not in {"", "none"}:
                    hover_failures.append(
                        f"stable no-blur profile reported shell backdrop filter {shell_filter!r}"
                    )
=======
            if hover_shell.get("transparent_window") and not hover_shell.get("compositor_blur_active"):
                hover_failures.append("native compositor blur inactive while transparent hover chrome is visible")
            shell_filter = str(hover_dom.get("shell_frame_backdrop_filter") or "")
            if hover_shell.get("compositor_blur_active") and shell_filter not in {"", "none"}:
                hover_failures.append(f"native compositor path mixed CSS backdrop filter {shell_filter!r}")
>>>>>>> c162185 (Snapshot alpha blur experiment)
            titlebar_rect = hover_dom.get("titlebar_rect") or {}
            titlebar_visible = float(titlebar_rect.get("height") or 0) > 8
            titlebar_background = str(hover_dom.get("titlebar_background") or "")
            titlebar_filter = str(hover_dom.get("titlebar_backdrop_filter") or "")
            if titlebar_visible and not titlebar_background:
                hover_failures.append("hovered titlebar material background was not observable")
            if (
<<<<<<< HEAD
                compositor_blur_active
=======
                hover_shell.get("compositor_blur_active")
>>>>>>> c162185 (Snapshot alpha blur experiment)
                and titlebar_visible
                and titlebar_filter not in {"", "none"}
            ):
                hover_failures.append(
                    f"native compositor titlebar mixed CSS backdrop filter {titlebar_filter!r}"
                )
<<<<<<< HEAD
            if not live_blur_supported and titlebar_filter not in {"", "none"}:
                hover_failures.append(
                    f"stable no-blur profile reported titlebar backdrop filter {titlebar_filter!r}"
                )
            shell_background = str(hover_dom.get("shell_frame_background") or "")
            if live_blur_supported and "rgba(" in shell_background:
=======
            shell_background = str(hover_dom.get("shell_frame_background") or "")
            if "rgba(" in shell_background:
>>>>>>> c162185 (Snapshot alpha blur experiment)
                try:
                    alpha_text = shell_background.rsplit(",", 1)[1].strip().rstrip(")")
                    if float(alpha_text) >= 0.74:
                        hover_failures.append(f"shell material alpha {alpha_text} is too opaque for live blur")
                except Exception:
                    hover_failures.append(f"could not parse shell material alpha from {shell_background!r}")
            if hover_failures:
                quirk_results["chrome_hover_material_error"] = "; ".join(hover_failures)
        except Exception as error:  # noqa: BLE001
            quirk_results["chrome_hover_error"] = str(error)
        finally:
            try:
                quirk_results["chrome_hover_off"] = app_chrome_hover(
                    args.host,
                    args.bin,
                    args.timeout_ms,
                    False,
                )
            except Exception as error:  # noqa: BLE001
                quirk_results["chrome_hover_off_error"] = str(error)

        try:
            quirk_results["scroll_probe_up"] = app_probe_terminal(
                args.host,
                args.bin,
                args.timeout_ms,
                quirk_path,
                "probe-scroll",
                "--lines",
                "-5",
            )
            quirk_results["scroll_probe_bottom"] = app_probe_terminal(
                args.host,
                args.bin,
                args.timeout_ms,
                quirk_path,
                "probe-scroll",
                "--lines",
                "9999",
            )
        except Exception as error:  # noqa: BLE001
            quirk_results["scroll_probe_error"] = str(error)

        try:
            quirk_results["select_probe"] = app_probe_terminal(
                args.host,
                args.bin,
                args.timeout_ms,
                quirk_path,
                "probe-select",
            )
        except Exception as error:  # noqa: BLE001
            quirk_results["select_probe_error"] = str(error)

        try:
            quirk_results["context_menu_probe"] = app_probe_terminal(
                args.host,
                args.bin,
                args.timeout_ms,
                quirk_path,
                "probe-context-menu",
            )
        except Exception as error:  # noqa: BLE001
            quirk_results["context_menu_probe_error"] = str(error)

    for entry in created_results:
        session_path = str(entry["created_session_path"])
        try:
            keep_alive_results.append(
                {
                    "session_path": session_path,
                    "keep": app_keep_session(
                        args.host,
                        args.bin,
                        args.timeout_ms,
                        session_path,
                        True,
                    ),
                }
            )
        except Exception as error:  # noqa: BLE001
            keep_alive_results.append({"session_path": session_path, "error": str(error)})

    restore_summary: dict[str, object] = {"enabled": bool(args.restore_pass)}
    if args.restore_pass and created_results:
        gui_bin = gui_binary_for_restore(args)
        try:
            restore_summary["close_preserve"] = app_close_preserve(
                args.host,
                args.bin,
                args.timeout_ms,
            )
            if args.restore_wait_sec > 0:
                time.sleep(args.restore_wait_sec)
            resources["cooldown"] = sample_resources(
                args.host,
                "cooldown",
                args.resource_sample_sec,
                out_dir,
            )
            cooldown_failure = _resource_failure(resources["cooldown"], args.cooldown_max_cpu)
            if cooldown_failure:
                resource_failures.append(cooldown_failure)
            restore_summary["launch"] = app_launch(args.host, gui_bin, args.timeout_ms)
            wait_for_window(args.host, args.bin, args.timeout_ms)
            resources["respawn_burst"] = sample_resources(
                args.host,
                "respawn-burst",
                args.resource_sample_sec,
                out_dir,
            )
<<<<<<< HEAD
            respawn_burst_failure = _resource_failure(
                resources["respawn_burst"],
                args.respawn_burst_max_cpu,
            )
            if respawn_burst_failure:
                resource_failures.append(respawn_burst_failure)
=======
            respawn_failure = _resource_failure(resources["respawn"], args.respawn_max_cpu)
            if respawn_failure:
                resource_failures.append(respawn_failure)
>>>>>>> c162185 (Snapshot alpha blur experiment)
            for index, original in enumerate(created_results):
                session_path = str(original["created_session_path"])
                restored: dict[str, object] = {
                    "session_path": session_path,
                    "workload_kind": original.get("workload_kind"),
                }
                try:
                    restored["open"] = app_open_session(
                        args.host,
                        args.bin,
                        args.timeout_ms,
                        session_path,
                    )
                    _, restored_state = wait_until(
                        f"restored terminal ready {session_path}",
                        args.ready_budget,
                        args.poll,
                        lambda: _require_terminal_ready(
                            app_state(args.host, args.bin, args.timeout_ms),
                            session_path,
                            args.host,
                            int(time.time() * 1000),
                            int(time.time() * 1000) + int(args.ready_budget * 1000),
                        ),
                    )
                    restored["titlebar_matches_viewport"] = titlebar_matches_viewport(
                        restored_state
                    )
                    restored["terminal_text_sample"] = active_terminal_text(restored_state)
                    if original.get("workload_kind") == "heavy":
                        restored["heavy_tui_visible"] = terminal_text_looks_like_tui(
                            str(restored["terminal_text_sample"])
                        )
                    else:
                        restored["markers_visible"] = (
                            MARKER_BEGIN in str(restored["terminal_text_sample"])
                            and MARKER_END in str(restored["terminal_text_sample"])
                        )
                    restored["state_dump"] = write_json(
                        out_dir / f"terminal-{index:02d}-restore.json",
                        restored_state,
                    )
                except Exception as error:  # noqa: BLE001
                    restored["error"] = str(error)
                    try:
                        restored["state_dump"] = write_json(
                            out_dir / f"terminal-{index:02d}-restore-failure.json",
                            app_state(args.host, args.bin, args.timeout_ms),
                        )
                    except Exception as state_error:  # noqa: BLE001
                        restored["state_error"] = str(state_error)
                restore_results.append(restored)
            if args.respawn_settle_sec > 0:
                time.sleep(args.respawn_settle_sec)
            resources["respawn_settled"] = sample_resources(
                args.host,
                "respawn-settled",
                args.resource_sample_sec,
                out_dir,
            )
            try:
                daemon_reports["after_restore"] = daemon_server_list(args.host, args.bin)
                daemon_failures.extend(
                    f"after_restore: {failure}"
                    for failure in daemon_owner_failures(daemon_reports["after_restore"])
                )
            except Exception as error:  # noqa: BLE001
                daemon_failures.append(f"after_restore server-list failed: {error}")
            respawn_failure = _resource_failure(resources["respawn_settled"], args.respawn_max_cpu)
            if respawn_failure:
                resource_failures.append(respawn_failure)
        except Exception as error:  # noqa: BLE001
            restore_summary["error"] = str(error)

    cleanup_results: list[dict] = []
    for entry in reversed(created_results):
        session_path = str(entry["created_session_path"])
        cleanup: dict[str, object] = {"session_path": session_path}
        if entry.get("workload_kind") == "heavy":
            cleanup["stop_heavy"] = stop_heavy_workload(
                args.host,
                args.bin,
                args.timeout_ms,
                session_path,
            )
        try:
            cleanup["unkeep"] = app_keep_session(
                args.host,
                args.bin,
                args.timeout_ms,
                session_path,
                False,
            )
        except Exception as error:  # noqa: BLE001
            cleanup["unkeep_error"] = str(error)
        try:
            cleanup["remove"] = app_remove_session(
                args.host,
                args.bin,
                args.timeout_ms,
                session_path,
            )
        except Exception as error:  # noqa: BLE001
            cleanup["remove_error"] = str(error)
        cleanup_results.append(cleanup)

    final_state = app_state(args.host, args.bin, args.timeout_ms)
    try:
        daemon_reports["final"] = daemon_server_list(args.host, args.bin)
        daemon_failures.extend(
            f"final: {failure}"
            for failure in daemon_owner_failures(daemon_reports["final"])
        )
    except Exception as error:  # noqa: BLE001
        daemon_failures.append(f"final server-list failed: {error}")
    quirk_failures = [
        key
        for key in quirk_results
        if key.endswith("_error") and quirk_results.get(key)
    ]
    restore_failures = [
        item
        for item in restore_results
        if item.get("error")
        or (
            item.get("workload_kind") == "heavy"
            and not item.get("heavy_tui_visible")
        )
        or (
            item.get("workload_kind") != "heavy"
            and not item.get("markers_visible")
        )
        or not item.get("titlebar_matches_viewport")
    ]
    summary = {
        "host": args.host,
        "count": args.count,
        "seed": args.seed,
        "heavy_count": heavy_count,
        "heavy_indices": sorted(heavy_indices),
        "spawn_budget_s": args.spawn_budget,
        "ready_budget_s": args.ready_budget,
        "summary_budget_s": args.summary_budget,
        "restore_pass": bool(args.restore_pass),
        "restore_wait_sec": args.restore_wait_sec,
        "resource_budgets": {
            "baseline_max_cpu": args.baseline_max_cpu,
            "active_max_cpu": args.active_max_cpu,
            "cooldown_max_cpu": args.cooldown_max_cpu,
<<<<<<< HEAD
            "respawn_burst_max_cpu": args.respawn_burst_max_cpu,
            "respawn_settle_sec": args.respawn_settle_sec,
=======
>>>>>>> c162185 (Snapshot alpha blur experiment)
            "respawn_max_cpu": args.respawn_max_cpu,
        },
        "launcher_preflight": launcher_preflight,
        "window_spawn_elapsed_ms": ((launch_event or {}).get("payload") or {}).get("elapsed_ms"),
        "window_spawn_within_900ms": (
            (((launch_event or {}).get("payload") or {}).get("elapsed_ms") or 10_000) <= 900
        ),
        "baseline_notifications_count": baseline_notifications,
        "final_notifications_count": ((final_state.get("shell") or {}).get("notifications_count")) or 0,
        "terminals_created": len(results),
        "creation_failures": len([item for item in results if item.get("created_session_path") is None]),
        "ready_failures": len([item for item in results if item.get("created_session_path") and not item.get("ready_within_budget")]),
        "command_failures": len([item for item in results if item.get("created_session_path") and not item.get("output_contains_markers")]),
        "heavy_tui_failures": len([item for item in results if item.get("workload_kind") == "heavy" and not item.get("heavy_tui_visible")]),
        "summary_failures": len([item for item in results if item.get("created_session_path") and not item.get("summary_present")]),
        "title_failures": len([item for item in results if item.get("created_session_path") and not item.get("title_present")]),
        "titlebar_failures": len([item for item in results if item.get("created_session_path") and not item.get("titlebar_matches_viewport")]),
        "cwd_failures": len([item for item in results if item.get("created_session_path") and not item.get("cwd_matches", False)]),
        "notification_anomalies": len([item for item in results if item.get("notification_noise")]),
        "keep_alive_failures": len([item for item in keep_alive_results if item.get("error")]),
        "restore_failures": len(restore_failures),
        "quirk_failures": quirk_failures,
        "resource_failures": resource_failures,
        "daemon_failures": daemon_failures,
        "daemon_reports": daemon_reports,
        "remove_failures": len([item for item in cleanup_results if item.get("remove_error")]),
        "resources": resources,
        "quirks": quirk_results,
        "keep_alive_results": keep_alive_results,
        "restore_summary": restore_summary,
        "restore_results": restore_results,
        "cleanup_results": cleanup_results,
        "results": results,
    }
    summary_path = out_dir / "summary.json"
    summary_path.write_text(json.dumps(summary, indent=2), encoding="utf-8")
    print(summary_path)
    print(json.dumps(summary, indent=2))

    if launch is not None and launch.poll() is None:
        launch.terminate()
        try:
            launch.wait(timeout=2)
        except subprocess.TimeoutExpired:
            launch.kill()

    return 0 if all(
        item.get("ready_within_budget")
        and item.get("output_contains_markers")
        and item.get("cwd_matches")
        and item.get("title_present")
        and item.get("summary_present")
        and item.get("titlebar_matches_viewport")
        and (item.get("workload_kind") != "heavy" or item.get("heavy_tui_visible"))
        and not item.get("notification_noise")
        for item in results
    ) and not quirk_failures and not resource_failures and not daemon_failures and not restore_failures and all(
        not item.get("error") for item in keep_alive_results
    ) and all(
        not item.get("remove_error") for item in cleanup_results
    ) else 1


def _require_terminal_ready(
    state: dict,
    session_path: str,
    host: str,
    started_ms: int,
    deadline_ms: int,
) -> dict:
    if not viewport_terminal_ready(state, session_path):
        viewport = state.get("viewport") or {}
        reason = str(viewport.get("reason") or "")
        if (
            (viewport.get("active_view_mode") == "Terminal")
            and (viewport.get("active_session_path") == session_path)
            and titlebar_matches_viewport(state)
            and (
                terminal_attach_ready_seen(host, session_path, started_ms, deadline_ms)
                or "plain shell prompt" in reason
            )
        ):
            return state
        viewport = state.get("viewport") or {}
        raise RuntimeError(viewport.get("reason") or "terminal viewport not ready")
    if not titlebar_matches_viewport(state):
        raise RuntimeError("titlebar not in sync with viewport")
    return state


def _require_terminal_markers(state: dict, session_path: str) -> dict:
    viewport = state.get("viewport") or {}
    if not viewport_terminal_ready(state, session_path) and not (
        viewport.get("active_view_mode") == "Terminal"
        and viewport.get("active_session_path") == session_path
    ):
        raise RuntimeError(viewport.get("reason") or "terminal viewport not ready")
    if not titlebar_matches_viewport(state):
        raise RuntimeError("titlebar not in sync with viewport")
    text = active_terminal_text(state)
    if MARKER_BEGIN not in text or MARKER_END not in text:
        raise RuntimeError("terminal output markers not visible yet")
    return state


def _require_terminal_tui(state: dict, session_path: str) -> dict:
    viewport = state.get("viewport") or {}
    if not viewport_terminal_ready(state, session_path) and not (
        viewport.get("active_view_mode") == "Terminal"
        and viewport.get("active_session_path") == session_path
    ):
        raise RuntimeError(viewport.get("reason") or "terminal viewport not ready")
    if not titlebar_matches_viewport(state):
        raise RuntimeError("titlebar not in sync with viewport")
    text = active_terminal_text(state)
    if not terminal_text_looks_like_tui(text):
        raise RuntimeError("heavy terminal TUI text not visible yet")
    return state


if __name__ == "__main__":
    raise SystemExit(main())
