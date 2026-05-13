#!/usr/bin/env python3
"""Capture first-paint/readiness timelines for fresh Codex terminals.

This smoke intentionally launches an isolated Yggterm profile so it can create
and remove throwaway local/remote Codex terminals without touching the user's
live sidebar. It captures the same two truths for each offset: what the user
would see (screenshot) and what app-control reports (state/rows/surface).
"""

from __future__ import annotations

import argparse
import json
import os
import shlex
import subprocess
import sys
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any

try:
    from PIL import Image, ImageStat
except ModuleNotFoundError:
    Image = ImageStat = None


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_OFFSETS_SEC = (0.5, 1.0, 3.0, 5.0, 10.0, 30.0, 60.0)
RESOURCE_ORIGIN_MONOTONIC_MS: int | None = None

RESOURCE_LOGGER_SCRIPT = r"""#!/usr/bin/env python3
import json
import os
import pathlib
import sys
import time

out_path = pathlib.Path(os.environ["YGGTERM_RESOURCE_LOG"])
interval = max(0.25, float(os.environ.get("YGGTERM_RESOURCE_INTERVAL_SEC", "1.0")))
tokens = tuple(
    token.strip().lower()
    for token in os.environ.get(
        "YGGTERM_RESOURCE_TOKENS",
        "yggterm,codex,webkit,xterm,ssh",
    ).split(",")
    if token.strip()
)
top_n = max(1, int(os.environ.get("YGGTERM_RESOURCE_TOP_N", "16")))
clock_ticks = os.sysconf(os.sysconf_names.get("SC_CLK_TCK", "SC_CLK_TCK"))
page_size = os.sysconf("SC_PAGE_SIZE")
self_pid = os.getpid()
proc_cache = {}


def now_ms():
    return int(time.time() * 1000)


def read_total_ticks():
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


def read_cmdline(pid):
    try:
        return (
            pathlib.Path("/proc")
            .joinpath(str(pid), "cmdline")
            .read_bytes()
            .replace(b"\0", b" ")
            .decode("utf-8", "replace")
            .strip()
        )
    except Exception:
        return ""


def read_comm(pid):
    try:
        return pathlib.Path("/proc").joinpath(str(pid), "comm").read_text().strip()
    except Exception:
        return ""


def read_env_subset(pid):
    wanted = {
        "YGGTERM_HOME",
        "YGGTERM_DESKTOP_APP_ID_SUFFIX",
        "YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF",
        "DISPLAY",
        "XAUTHORITY",
    }
    result = {}
    try:
        for raw in pathlib.Path("/proc").joinpath(str(pid), "environ").read_bytes().split(b"\0"):
            if not raw or b"=" not in raw:
                continue
            key, value = raw.split(b"=", 1)
            key_text = key.decode("utf-8", "replace")
            if key_text in wanted:
                result[key_text] = value.decode("utf-8", "replace")
    except Exception:
        pass
    return result


def read_stat(pid):
    try:
        fields = pathlib.Path("/proc").joinpath(str(pid), "stat").read_text().rsplit(")", 1)[1].split()
        return {
            "ppid": int(fields[1]),
            "ticks": int(fields[11]) + int(fields[12]),
            "rss_kb": int(fields[21]) * page_size // 1024,
        }
    except Exception:
        return None


def all_pids():
    try:
        return [int(path.name) for path in pathlib.Path("/proc").iterdir() if path.name.isdigit()]
    except Exception:
        return []


def snapshot():
    rows = {}
    for pid in all_pids():
        stat = read_stat(pid)
        if not stat:
            continue
        cached = proc_cache.get(pid)
        if not cached:
            comm = read_comm(pid)
            cmd = read_cmdline(pid)
            hay = f"{comm} {cmd}".lower()
            matches_tokens = any(token in hay for token in tokens)
            is_logger = pid == self_pid or "YGGTERM_RESOURCE_LOG" in cmd or "resource_logger_script" in cmd
            cached = {
                "comm": comm,
                "cmd": cmd[:260],
                "matches_tokens": matches_tokens,
                "env": read_env_subset(pid) if matches_tokens and not is_logger else {},
                "resource_logger": is_logger,
            }
            proc_cache[pid] = cached
        rows[pid] = {
            "pid": pid,
            "ppid": stat["ppid"],
            "comm": cached["comm"],
            "cmd": cached["cmd"],
            "ticks": stat["ticks"],
            "rss_kb": stat["rss_kb"],
            "matches_tokens": cached["matches_tokens"],
            "resource_logger": cached["resource_logger"],
            "env": cached["env"],
        }
    live_pids = set(rows)
    for cached_pid in list(proc_cache):
        if cached_pid not in live_pids:
            proc_cache.pop(cached_pid, None)
    return read_total_ticks(), rows


def emit(prev_total, prev_rows, next_total, next_rows, seq, started_ms):
    cores = cpu_count()
    denominator = max(1, next_total - prev_total)
    rows = []
    for pid, row in next_rows.items():
        previous = prev_rows.get(pid)
        start_ticks = previous["ticks"] if previous else row["ticks"]
        delta = max(0, row["ticks"] - start_ticks)
        out = dict(row)
        out["delta_ticks"] = delta
        out["cpu_percent"] = round((delta / denominator) * cores * 100.0, 3)
        if out["matches_tokens"] or out["cpu_percent"] > 0:
            rows.append(out)
    rows.sort(key=lambda item: (-float(item.get("cpu_percent") or 0), item["comm"], item["pid"]))
    logger_rows = [row for row in rows if row.get("resource_logger")]
    matched = [row for row in rows if row.get("matches_tokens") and not row.get("resource_logger")]
    system_top = rows[:top_n]
    payload = {
        "seq": seq,
        "ts_ms": now_ms(),
        "elapsed_ms": now_ms() - started_ms,
        "interval_sec": interval,
        "cpu_count": cores,
        "matched_cpu_percent": round(sum(float(row.get("cpu_percent") or 0) for row in matched), 3),
        "resource_logger_cpu_percent": round(sum(float(row.get("cpu_percent") or 0) for row in logger_rows), 3),
        "system_top_cpu_percent": round(sum(float(row.get("cpu_percent") or 0) for row in system_top), 3),
        "matched": matched[:top_n],
        "resource_logger": logger_rows[:top_n],
        "system_top": system_top,
    }
    with out_path.open("a") as handle:
        handle.write(json.dumps(payload, sort_keys=True) + "\n")


out_path.parent.mkdir(parents=True, exist_ok=True)
started_ms = now_ms()
prev_total, prev_rows = snapshot()
seq = 0
with out_path.open("a") as handle:
    handle.write(json.dumps({
        "seq": seq,
        "ts_ms": started_ms,
        "elapsed_ms": 0,
        "event": "resource_logger_started",
        "tokens": tokens,
        "interval_sec": interval,
    }, sort_keys=True) + "\n")
while True:
    time.sleep(interval)
    seq += 1
    next_total, next_rows = snapshot()
    emit(prev_total, prev_rows, next_total, next_rows, seq, started_ms)
    prev_total, prev_rows = next_total, next_rows
"""


@dataclass
class CommandResult:
    argv: list[str]
    elapsed_ms: float
    stdout: str
    stderr: str
    returncode: int


class Runner:
    def __init__(self, args: argparse.Namespace, yggterm_home: str, app_id_suffix: str) -> None:
        self.args = args
        self.yggterm_home = yggterm_home
        self.app_id_suffix = app_id_suffix
        self.control_bin = args.bin

    def _env_prefix(self) -> str:
        env = {
            "YGGTERM_HOME": self.yggterm_home,
            "YGGTERM_REMOTE_SMOKE_TAG": "1",
            "YGGTERM_DESKTOP_APP_ID_SUFFIX": self.app_id_suffix,
            "YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF": "1",
        }
        if self.args.display:
            env["DISPLAY"] = self.args.display
        if self.args.xauthority:
            env["XAUTHORITY"] = self.args.xauthority
        if self.args.force_x11:
            env["GDK_BACKEND"] = "x11"
            env["WINIT_UNIX_BACKEND"] = "x11"
            env["YGGTERM_FORCE_X11_BACKEND"] = "1"
        return " ".join(f"{key}={shlex.quote(value)}" for key, value in env.items())

    def _command(
        self,
        argv: list[str],
        *,
        shell: bool = False,
        bin_path: str | None = None,
    ) -> list[str]:
        if shell:
            command = " ".join(argv)
        else:
            binary = bin_path or self.control_bin
            command = " ".join([shlex.quote(binary), *(shlex.quote(part) for part in argv)])
        command = f"{self._env_prefix()} {command}"
        if self.args.host:
            return ["ssh", self.args.host, command]
        return ["bash", "-lc", command]

    def run(
        self,
        argv: list[str],
        *,
        timeout: float = 30.0,
        check: bool = True,
        bin_path: str | None = None,
    ) -> CommandResult:
        started = time.perf_counter()
        proc = subprocess.run(
            self._command(argv, bin_path=bin_path),
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
        )
        elapsed_ms = (time.perf_counter() - started) * 1000.0
        result = CommandResult(argv=argv, elapsed_ms=elapsed_ms, stdout=proc.stdout, stderr=proc.stderr, returncode=proc.returncode)
        if check and proc.returncode != 0:
            raise RuntimeError(
                f"command failed ({proc.returncode}): {' '.join(argv)}\n"
                f"{proc.stderr.strip() or proc.stdout.strip()}"
            )
        return result

    def run_json(
        self,
        argv: list[str],
        *,
        timeout: float = 30.0,
        check: bool = True,
        bin_path: str | None = None,
    ) -> dict[str, Any]:
        result = self.run(argv, timeout=timeout, check=check, bin_path=bin_path)
        if result.returncode != 0 and not check:
            return {
                "returncode": result.returncode,
                "stdout": result.stdout,
                "stderr": result.stderr,
                "elapsed_ms": result.elapsed_ms,
            }
        text = result.stdout.strip()
        if not text:
            return {}
        try:
            payload = json.loads(text)
        except json.JSONDecodeError as error:
            raise RuntimeError(f"command did not return JSON: {' '.join(argv)}\n{text[:1000]}") from error
        if isinstance(payload, dict):
            payload.setdefault("_elapsed_ms", result.elapsed_ms)
        return payload

    def run_gui_json(
        self,
        argv: list[str],
        *,
        timeout: float = 30.0,
        check: bool = True,
    ) -> dict[str, Any]:
        return self.run_json(argv, timeout=timeout, check=check, bin_path=self.args.bin)

    def shell(self, script: str, *, timeout: float = 30.0, check: bool = True) -> CommandResult:
        command = f"{self._env_prefix()} bash -lc {shlex.quote(script)}"
        argv = ["ssh", self.args.host, command] if self.args.host else ["bash", "-lc", command]
        started = time.perf_counter()
        proc = subprocess.run(
            argv,
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
        )
        elapsed_ms = (time.perf_counter() - started) * 1000.0
        result = CommandResult(
            argv=argv,
            elapsed_ms=elapsed_ms,
            stdout=proc.stdout,
            stderr=proc.stderr,
            returncode=proc.returncode,
        )
        if check and proc.returncode != 0:
            raise RuntimeError(
                f"shell command failed ({proc.returncode}): {script}\n"
                f"{proc.stderr.strip() or proc.stdout.strip()}"
            )
        return result

    def write_json(self, path: str, payload: dict[str, Any]) -> None:
        text = json.dumps(payload, indent=2)
        if self.args.host:
            subprocess.run(
                ["ssh", self.args.host, f"mkdir -p {shlex.quote(str(Path(path).parent))} && cat > {shlex.quote(path)}"],
                input=text,
                text=True,
                check=True,
                timeout=20,
            )
        else:
            target = Path(path)
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(text)

    def pull_file(self, remote_path: str, local_path: Path) -> None:
        local_path.parent.mkdir(parents=True, exist_ok=True)
        if self.args.host:
            subprocess.run(
                ["scp", f"{self.args.host}:{remote_path}", str(local_path)],
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                timeout=30,
                check=True,
            )
        else:
            source = Path(remote_path)
            if source != local_path:
                local_path.write_bytes(source.read_bytes())


def append_jsonl(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a") as handle:
        handle.write(json.dumps(payload, sort_keys=True) + "\n")


def record_phase(path: Path, phase: str, **fields: Any) -> None:
    monotonic_ms = int(time.monotonic() * 1000)
    payload = {
        "phase": phase,
        "ts_ms": int(time.time() * 1000),
        "monotonic_ms": monotonic_ms,
        **fields,
    }
    if RESOURCE_ORIGIN_MONOTONIC_MS is not None:
        payload["resource_elapsed_ms"] = monotonic_ms - RESOURCE_ORIGIN_MONOTONIC_MS
    append_jsonl(
        path,
        payload,
    )


def start_resource_logger(
    args: argparse.Namespace,
    runner: Runner,
    *,
    remote_resource_log: str,
    remote_resource_script: str,
    interval_sec: float,
) -> int:
    runner.write_json(remote_resource_script, {"script": RESOURCE_LOGGER_SCRIPT})
    bootstrap = f"""
set -eu
script_json={shlex.quote(remote_resource_script)}
script_py={shlex.quote(remote_resource_script + '.py')}
python3 - "$script_json" "$script_py" <<'PY'
import json, pathlib, sys
payload = json.loads(pathlib.Path(sys.argv[1]).read_text())
pathlib.Path(sys.argv[2]).write_text(payload["script"])
PY
chmod +x "$script_py"
YGGTERM_RESOURCE_LOG={shlex.quote(remote_resource_log)} \
YGGTERM_RESOURCE_INTERVAL_SEC={shlex.quote(str(interval_sec))} \
nohup python3 "$script_py" >/tmp/yggterm-resource-logger.out 2>/tmp/yggterm-resource-logger.err &
printf '%s\\n' "$!"
"""
    result = runner.shell(bootstrap, timeout=15)
    try:
        return int(result.stdout.strip().splitlines()[-1])
    except Exception as error:
        raise RuntimeError(f"resource logger did not return pid: {result.stdout!r}") from error


def storage_preflight(runner: Runner, paths: list[str]) -> dict[str, Any]:
    unique_paths = []
    for path in [*paths, "/tmp"]:
        if path and path not in unique_paths:
            unique_paths.append(path)
    quoted = " ".join(shlex.quote(path) for path in unique_paths)
    result = runner.shell(f"df -Pk {quoted} 2>/dev/null || true", timeout=10, check=False)
    return {
        "paths": unique_paths,
        "stdout": result.stdout,
        "stderr": result.stderr,
        "returncode": result.returncode,
        "elapsed_ms": result.elapsed_ms,
    }


def stop_resource_logger(runner: Runner, pid: int | None) -> None:
    if not pid:
        return
    runner.shell(f"kill {int(pid)} >/dev/null 2>&1 || true", timeout=5, check=False)


def seed_remote_machine_state(args: argparse.Namespace, runner: Runner) -> None:
    if not args.remote_machine_key:
        return
    state = {
        "active_session_path": None,
        "active_view_mode": "Rendered",
        "ssh_targets": [
            {
                "label": args.remote_machine_key,
                "kind": "ssh_shell",
                "ssh_target": args.remote_ssh_target or args.remote_machine_key,
                "prefix": args.remote_prefix,
                "cwd": args.remote_cwd,
            }
        ],
        "remote_machines": [
            {
                "machine_key": args.remote_machine_key,
                "label": args.remote_machine_key,
                "ssh_target": args.remote_ssh_target or args.remote_machine_key,
                "prefix": args.remote_prefix,
                "remote_binary_expr": args.remote_binary_expr,
                "remote_deploy_state": "Ready",
                "health": "healthy",
                "sessions": [],
            }
        ],
        "live_sessions": [],
        "stored_sessions": [],
    }
    runner.write_json(str(Path(runner.yggterm_home) / "server-state.json"), state)


def preflight_binary_versions(args: argparse.Namespace, runner: Runner) -> dict[str, str]:
    script = f"""
set -eu
bin={shlex.quote(args.bin)}
gui_version="$("$bin" --version 2>/dev/null || true)"
bin_dir="$(dirname "$bin")"
bin_base="$(basename "$bin")"
headless_base="$bin_base"
case "$bin_base" in
  yggterm*) headless_base="yggterm-headless${{bin_base#yggterm}}" ;;
esac
headless=""
headless_version=""
for candidate in "$bin_dir/yggterm-headless" "$bin_dir/$headless_base"; do
  if [ -x "$candidate" ]; then
    headless="$candidate"
    headless_version="$("$headless" --version 2>/dev/null || true)"
    break
  fi
done
printf '%s\\n%s\\n%s\\n' "$gui_version" "$headless" "$headless_version"
"""
    result = runner.shell(script, timeout=15)
    lines = result.stdout.splitlines()
    gui_version = lines[0].strip() if len(lines) >= 1 else ""
    headless_path = lines[1].strip() if len(lines) >= 2 else ""
    headless_version = lines[2].strip() if len(lines) >= 3 else ""
    if not headless_path or not headless_version:
        raise RuntimeError(
            "timeline smoke requires a matched yggterm-headless sibling for app-control probes; "
            f"missing executable or version at {headless_path or '<none>'}. "
            "Using the GUI binary for probe commands can perturb focus and startup measurements."
        )
    if headless_version and gui_version and headless_version != gui_version:
        raise RuntimeError(
            "yggterm GUI/headless version mismatch before smoke launch: "
            f"{args.bin}={gui_version}, {headless_path}={headless_version}. "
            "Build both binaries before running the timeline smoke."
        )
    runner.control_bin = headless_path
    return {
        "gui_version": gui_version,
        "headless_path": headless_path,
        "headless_version": headless_version,
        "control_bin": runner.control_bin,
    }


def focus_owned_window(
    runner: Runner,
    *,
    pid: int,
    timeout_ms: int,
    command_timeout_sec: float,
) -> dict[str, Any]:
    payload = runner.run_json(
        ["server", "app", "focus", "--pid", str(pid), "--timeout-ms", str(timeout_ms)],
        timeout=command_timeout_sec,
        check=False,
    )
    data = response_data(payload)
    window = data.get("window") if isinstance(data.get("window"), dict) else {}
    focused = bool(data.get("focused") or window.get("focused"))
    payload["focus_confirmed"] = focused
    if not focused:
        payload.setdefault(
            "focus_problem",
            data.get("error")
            or payload.get("error")
            or "app-control focus request did not produce native window focus",
        )
    return payload


def response_data(payload: dict[str, Any]) -> dict[str, Any]:
    data = payload.get("data")
    return data if isinstance(data, dict) else {}


def active_viewport(state: dict[str, Any]) -> dict[str, Any]:
    viewport = state.get("viewport")
    return viewport if isinstance(viewport, dict) else state


def active_surface(state: dict[str, Any]) -> dict[str, Any]:
    viewport = active_viewport(state)
    surface = viewport.get("active_terminal_surface")
    if isinstance(surface, dict):
        return surface
    surface = state.get("active_terminal_surface")
    return surface if isinstance(surface, dict) else {}


def runtime_truth(state: dict[str, Any]) -> dict[str, Any]:
    value = state.get("runtime_truth")
    return value if isinstance(value, dict) else {}


def active_host(state: dict[str, Any], session_path: str) -> dict[str, Any]:
    viewport = active_viewport(state)
    hosts = viewport.get("active_terminal_hosts")
    if not isinstance(hosts, list):
        hosts = viewport.get("terminal_hosts")
    if not isinstance(hosts, list):
        hosts = (state.get("dom") or {}).get("terminal_hosts")
    if not isinstance(hosts, list):
        return {}
    matches = [host for host in hosts if isinstance(host, dict) and host.get("session_path") == session_path]
    if not matches:
        return {}
    matches.sort(
        key=lambda host: (
            0
            if host.get("effective_input_focus")
            or host.get("helper_textarea_focused")
            or host.get("host_has_active_element")
            else 1,
            0 if host.get("input_enabled") else 1,
            -int(host.get("last_render_event_at_ms") or 0),
        )
    )
    return matches[0]


def screenshot_ink(path: Path) -> dict[str, Any] | None:
    if Image is None or ImageStat is None or not path.exists():
        return None
    try:
        image = Image.open(path).convert("RGB")
    except Exception as error:
        return {"error": str(error)}
    stat = ImageStat.Stat(image)
    extrema = stat.extrema
    spread = sum(float(high) - float(low) for low, high in extrema)
    sample = image.resize((64, 64))
    if hasattr(sample, "get_flattened_data"):
        pixels = list(sample.get_flattened_data())
    else:
        pixels = list(sample.getdata())
    background = pixels[0] if pixels else (255, 255, 255)
    changed = sum(
        1
        for pixel in pixels
        if sum(abs(int(pixel[index]) - int(background[index])) for index in range(3)) > 30
    )
    return {
        "width": image.width,
        "height": image.height,
        "channel_spread": spread,
        "changed_sample_pixels": changed,
    }


def proc_cpu_sample(runner: Runner) -> dict[str, Any]:
    script = r"""python3 - <<'PY'
import json, subprocess
proc = subprocess.run(['ps', '-eo', 'pid=,ppid=,pcpu=,pmem=,comm=,args='], text=True, stdout=subprocess.PIPE)
rows = []
for line in proc.stdout.splitlines():
    parts = line.strip().split(None, 5)
    if len(parts) < 6:
        continue
    pid, ppid, pcpu, pmem, comm, cmd = parts
    hay = (comm + ' ' + cmd).lower()
    if any(token in hay for token in ('yggterm', 'codex', 'webkit', 'xterm')):
        try:
            rows.append({'pid': int(pid), 'ppid': int(ppid), 'pcpu': float(pcpu), 'pmem': float(pmem), 'comm': comm, 'cmd': cmd[:240]})
        except ValueError:
            pass
rows.sort(key=lambda row: row['pcpu'], reverse=True)
print(json.dumps(rows[:20]))
PY"""
    try:
        result = runner.shell(script, timeout=10)
        rows = json.loads(result.stdout)
        return {"rows": rows, "elapsed_ms": result.elapsed_ms}
    except Exception as error:
        return {"error": str(error)}


def classify_sample(
    label: str,
    session_path: str,
    offset_sec: float,
    state: dict[str, Any],
    shot_ink: dict[str, Any] | None,
) -> dict[str, Any]:
    surface = active_surface(state)
    truth = runtime_truth(state)
    host = active_host(state, session_path)
    dom = state.get("dom") if isinstance(state.get("dom"), dict) else {}
    shell = state.get("shell") if isinstance(state.get("shell"), dict) else {}
    dom_error = dom.get("error") if isinstance(dom, dict) else None
    problem = surface.get("problem") or surface.get("live_problem") or surface.get("geometry_problem")
    contract_violations = state.get("session_view_contract_violations")
    if not problem and isinstance(contract_violations, list) and contract_violations:
        problem = str(contract_violations[0])
    if not problem and isinstance(dom_error, str) and dom_error.strip():
        problem = f"app-control DOM snapshot failed: {dom_error.strip()}"
    content_source = surface.get("content_source") or host.get("terminal_content_source")
    text_tail = str(host.get("text_tail") or host.get("buffer_text_sample") or "")[-500:]
    has_codex_prompt = "›" in text_tail or ">_" in text_tail
    has_codex_welcome = "OpenAI Codex" in text_tail
    window_focused = shell.get("window_focused")
    terminal_input_override_active = shell.get("terminal_input_override_active")
    document_focused = host.get("document_focused")
    host_effective_input_focus = host.get("effective_input_focus") is True or (
        host.get("input_enabled") is True
        and host.get("helper_textarea_focused") is True
        and host.get("host_has_active_element") is True
    )
    surface_effective_input_focus = surface.get("effective_input_focus") is True
    runtime_effective_input_focus = truth.get("active_host_effective_input_focus") is True
    effective_terminal_input_focus = (
        host_effective_input_focus
        or surface_effective_input_focus
        or runtime_effective_input_focus
    )
    focus_ready = (
        window_focused is True
        and document_focused is True
    ) or effective_terminal_input_focus
    input_ready = (
        truth.get("active_host_input_enabled") is True
        or surface.get("input_enabled") is True
        or host.get("input_enabled") is True
    )
    render_ready = (
        truth.get("active_host_ready") is True
        and not problem
        and str(content_source or "") not in ("", "empty")
        and (has_codex_prompt or has_codex_welcome)
    )
    focus_problem = None
    if render_ready and not focus_ready:
        missing = []
        if window_focused is not True:
            missing.append("window")
        if document_focused is not True:
            missing.append("document")
        if not effective_terminal_input_focus:
            missing.append("xterm-helper")
        focus_problem = "rendered terminal surface is focus-gated"
        if missing:
            focus_problem = f"{focus_problem}: {'/'.join(missing)} focus is not owned"
    if not problem and render_ready and not input_ready:
        problem = focus_problem or "ready terminal surface is not accepting input"
    if not problem and render_ready and input_ready and focus_problem:
        problem = "terminal input is enabled while app/document focus is not owned"
    ready = render_ready and input_ready and focus_ready and not problem
    if ready:
        readiness_class = "ready"
    elif render_ready and focus_problem and not input_ready:
        readiness_class = "render_ready_focus_gated"
    elif render_ready and not input_ready:
        readiness_class = "render_ready_input_disabled"
    elif render_ready and focus_problem:
        readiness_class = "render_ready_focus_inconsistent"
    elif problem:
        readiness_class = "surface_problem"
    else:
        readiness_class = "not_render_ready"
    screenshot_blank = None
    if shot_ink:
        screenshot_blank = (
            float(shot_ink.get("channel_spread") or 0.0) < 30.0
            and int(shot_ink.get("changed_sample_pixels") or 0) < 24
        )
    return {
        "label": label,
        "session_path": session_path,
        "offset_sec": offset_sec,
        "ready": ready,
        "render_ready": render_ready,
        "input_ready": input_ready,
        "focus_ready": focus_ready,
        "problem": problem,
        "focus_problem": focus_problem,
        "input_blocked_by_focus": render_ready and not input_ready and bool(focus_problem),
        "input_enabled_without_focus": render_ready and input_ready and bool(focus_problem),
        "readiness_class": readiness_class,
        "dom_error": dom_error,
        "dom_snapshot_mode": dom.get("snapshot_mode") if isinstance(dom, dict) else None,
        "dom_degraded_reason": dom.get("degraded_reason") if isinstance(dom, dict) else None,
        "window_focused": window_focused,
        "terminal_input_override_active": terminal_input_override_active,
        "document_focused": document_focused,
        "host_effective_input_focus": host_effective_input_focus,
        "surface_effective_input_focus": surface_effective_input_focus,
        "runtime_active_host_effective_input_focus": runtime_effective_input_focus,
        "effective_terminal_input_focus": effective_terminal_input_focus,
        "host_helper_textarea_focused": host.get("helper_textarea_focused"),
        "host_has_active_element": host.get("host_has_active_element"),
        "session_view_contract_violations": contract_violations if isinstance(contract_violations, list) else [],
        "content_source": content_source,
        "surface_input_enabled": surface.get("input_enabled"),
        "surface_raw_input_enabled": surface.get("raw_input_enabled"),
        "runtime_active_host_ready": truth.get("active_host_ready"),
        "runtime_active_host_input_enabled": truth.get("active_host_input_enabled"),
        "runtime_active_host_raw_input_enabled": truth.get("active_host_raw_input_enabled"),
        "host_input_enabled": host.get("input_enabled"),
        "write_bridge_flush_count": host.get("write_bridge_flush_count"),
        "write_command_count": host.get("write_command_count"),
        "data_event_count": host.get("data_event_count"),
        "last_raw_payload_length": host.get("last_raw_payload_length"),
        "last_raw_payload_line_count": host.get("last_raw_payload_line_count"),
        "render_event_count": host.get("render_event_count"),
        "blank_rows_below_cursor": host.get("blank_rows_below_cursor"),
        "rows": host.get("rows"),
        "screenshot_blank": screenshot_blank,
        "screenshot_ink": shot_ink,
        "text_tail": text_tail,
    }


def launch_terminal(
    runner: Runner,
    *,
    remote_machine_key: str | None,
    cwd: str,
    title: str,
    timeout_ms: int,
) -> dict[str, Any]:
    argv = ["server", "app", "terminal", "new", "--kind", "codex", "--cwd", cwd, "--title", title, "--timeout-ms", str(timeout_ms)]
    if remote_machine_key:
        argv[4:4] = ["--machine-key", remote_machine_key]
    payload = runner.run_json(argv, timeout=max(30.0, timeout_ms / 1000.0 + 5.0))
    data = response_data(payload)
    session_path = data.get("session_path") or data.get("active_session_path")
    if not isinstance(session_path, str) or not session_path:
        raise RuntimeError(f"terminal new did not return session_path: {json.dumps(payload)[:1000]}")
    return {
        "session_path": session_path,
        "payload": payload,
        "elapsed_ms": payload.get("_elapsed_ms"),
    }


def capture_one(
    args: argparse.Namespace,
    runner: Runner,
    *,
    label: str,
    session_path: str,
    offset_sec: float,
    started: float,
    local_capture_dir: Path,
    remote_capture_dir: str,
    pid: int,
) -> dict[str, Any]:
    deadline = started + offset_sec
    remaining = deadline - time.perf_counter()
    if remaining > 0:
        time.sleep(remaining)
    focus_payload: dict[str, Any] | None = None
    terminal_focus_payload: dict[str, Any] | None = None
    if not args.no_focus_before_capture:
        focus_payload = focus_owned_window(
            runner,
            pid=pid,
            timeout_ms=args.timeout_ms,
            command_timeout_sec=args.command_timeout_sec,
        )
        terminal_focus_payload = runner.run_json(
            [
                "server",
                "app",
                "terminal",
                "focus",
                session_path,
                "--timeout-ms",
                str(args.timeout_ms),
            ],
            timeout=args.command_timeout_sec,
            check=False,
        )
    state_payload = runner.run_json(
        ["server", "app", "state", "--pid", str(pid), "--timeout-ms", str(args.timeout_ms)],
        timeout=args.command_timeout_sec,
    )
    rows_payload = runner.run_json(
        ["server", "app", "rows", "--pid", str(pid), "--timeout-ms", str(args.timeout_ms)],
        timeout=args.command_timeout_sec,
        check=False,
    )
    state = response_data(state_payload)
    remote_shot = f"{remote_capture_dir}/{label}/shot_{str(offset_sec).replace('.', '_')}.png"
    local_shot = local_capture_dir / label / f"shot_{str(offset_sec).replace('.', '_')}.png"
    screenshot_payload = runner.run_json(
        ["server", "app", "screenshot", remote_shot, "--pid", str(pid), "--timeout-ms", str(args.timeout_ms)],
        timeout=args.command_timeout_sec,
    )
    runner.pull_file(remote_shot, local_shot)
    post_screenshot_state_payload = runner.run_json(
        ["server", "app", "state", "--pid", str(pid), "--timeout-ms", str(args.timeout_ms)],
        timeout=args.command_timeout_sec,
    )
    shot_ink = screenshot_ink(local_shot)
    cpu = proc_cpu_sample(runner)
    screenshot_state = response_data(post_screenshot_state_payload)
    summary = classify_sample(label, session_path, offset_sec, state, shot_ink)
    screenshot_summary = classify_sample(label, session_path, offset_sec, screenshot_state, shot_ink)
    if screenshot_summary.get("ready") and not summary.get("ready"):
        summary.update(
            {
                "ready": True,
                "problem": screenshot_summary.get("problem"),
                "focus_problem": screenshot_summary.get("focus_problem"),
                "input_blocked_by_focus": screenshot_summary.get("input_blocked_by_focus"),
                "input_enabled_without_focus": screenshot_summary.get("input_enabled_without_focus"),
                "readiness_class": screenshot_summary.get("readiness_class"),
                "content_source": screenshot_summary.get("content_source"),
                "surface_input_enabled": screenshot_summary.get("surface_input_enabled"),
                "surface_raw_input_enabled": screenshot_summary.get("surface_raw_input_enabled"),
                "runtime_active_host_ready": screenshot_summary.get("runtime_active_host_ready"),
                "runtime_active_host_input_enabled": screenshot_summary.get("runtime_active_host_input_enabled"),
                "runtime_active_host_raw_input_enabled": screenshot_summary.get("runtime_active_host_raw_input_enabled"),
                "host_input_enabled": screenshot_summary.get("host_input_enabled"),
                "window_focused": screenshot_summary.get("window_focused"),
                "terminal_input_override_active": screenshot_summary.get(
                    "terminal_input_override_active"
                ),
                "document_focused": screenshot_summary.get("document_focused"),
                "host_effective_input_focus": screenshot_summary.get(
                    "host_effective_input_focus"
                ),
                "surface_effective_input_focus": screenshot_summary.get(
                    "surface_effective_input_focus"
                ),
                "runtime_active_host_effective_input_focus": screenshot_summary.get(
                    "runtime_active_host_effective_input_focus"
                ),
                "effective_terminal_input_focus": screenshot_summary.get(
                    "effective_terminal_input_focus"
                ),
                "host_helper_textarea_focused": screenshot_summary.get(
                    "host_helper_textarea_focused"
                ),
                "host_has_active_element": screenshot_summary.get("host_has_active_element"),
                "focus_ready": screenshot_summary.get("focus_ready"),
                "render_ready": screenshot_summary.get("render_ready"),
                "input_ready": screenshot_summary.get("input_ready"),
                "write_bridge_flush_count": screenshot_summary.get("write_bridge_flush_count"),
                "write_command_count": screenshot_summary.get("write_command_count"),
                "data_event_count": screenshot_summary.get("data_event_count"),
                "last_raw_payload_length": screenshot_summary.get("last_raw_payload_length"),
                "last_raw_payload_line_count": screenshot_summary.get("last_raw_payload_line_count"),
                "render_event_count": screenshot_summary.get("render_event_count"),
                "blank_rows_below_cursor": screenshot_summary.get("blank_rows_below_cursor"),
                "rows": screenshot_summary.get("rows"),
                "text_tail": screenshot_summary.get("text_tail"),
                "reconciled_from_screenshot_state": True,
                "initial_ready": False,
                "initial_problem": classify_sample(label, session_path, offset_sec, state, shot_ink).get("problem"),
            }
        )
    else:
        summary["reconciled_from_screenshot_state"] = False
        if screenshot_summary.get("focus_ready") and not summary.get("focus_ready"):
            summary.update(
                {
                    "focus_ready": True,
                    "focus_problem": screenshot_summary.get("focus_problem"),
                    "input_blocked_by_focus": screenshot_summary.get("input_blocked_by_focus"),
                    "input_enabled_without_focus": screenshot_summary.get(
                        "input_enabled_without_focus"
                    ),
                    "window_focused": screenshot_summary.get("window_focused"),
                    "terminal_input_override_active": screenshot_summary.get(
                        "terminal_input_override_active"
                    ),
                    "document_focused": screenshot_summary.get("document_focused"),
                    "host_effective_input_focus": screenshot_summary.get(
                        "host_effective_input_focus"
                    ),
                    "surface_effective_input_focus": screenshot_summary.get(
                        "surface_effective_input_focus"
                    ),
                    "runtime_active_host_effective_input_focus": screenshot_summary.get(
                        "runtime_active_host_effective_input_focus"
                    ),
                    "effective_terminal_input_focus": screenshot_summary.get(
                        "effective_terminal_input_focus"
                    ),
                    "host_helper_textarea_focused": screenshot_summary.get(
                        "host_helper_textarea_focused"
                    ),
                    "host_has_active_element": screenshot_summary.get("host_has_active_element"),
                    "reconciled_focus_from_screenshot_state": True,
                    "initial_focus_ready": False,
                }
            )
        else:
            summary["reconciled_focus_from_screenshot_state"] = False
    if isinstance(focus_payload, dict):
        focus_confirmed = bool(focus_payload.get("focus_confirmed"))
        summary["sample_focus_confirmed"] = focus_confirmed
        summary["focus_command_state_disagrees"] = focus_confirmed != bool(summary.get("focus_ready"))
    else:
        summary["sample_focus_confirmed"] = None
        summary["focus_command_state_disagrees"] = False
    if isinstance(terminal_focus_payload, dict):
        terminal_focus_data = response_data(terminal_focus_payload)
        terminal_focus_accepted = terminal_focus_data.get("accepted") is True
        summary["sample_terminal_focus_accepted"] = terminal_focus_accepted
        summary["terminal_focus_command_state_disagrees"] = terminal_focus_accepted and not bool(
            summary.get("focus_ready")
        )
    else:
        summary["sample_terminal_focus_accepted"] = None
        summary["terminal_focus_command_state_disagrees"] = False
    state_path = local_capture_dir / label / f"state_{str(offset_sec).replace('.', '_')}.json"
    rows_path = local_capture_dir / label / f"rows_{str(offset_sec).replace('.', '_')}.json"
    screenshot_state_path = local_capture_dir / label / f"screenshot_state_{str(offset_sec).replace('.', '_')}.json"
    screenshot_response_path = local_capture_dir / label / f"screenshot_response_{str(offset_sec).replace('.', '_')}.json"
    state_path.parent.mkdir(parents=True, exist_ok=True)
    state_path.write_text(json.dumps(state_payload, indent=2))
    rows_path.write_text(json.dumps(rows_payload, indent=2))
    screenshot_state_path.write_text(json.dumps(post_screenshot_state_payload, indent=2))
    screenshot_response_path.write_text(json.dumps(screenshot_payload, indent=2))
    summary.update(
        {
            "screenshot": str(local_shot),
            "state_json": str(state_path),
            "rows_json": str(rows_path),
            "screenshot_state_json": str(screenshot_state_path),
            "screenshot_response_json": str(screenshot_response_path),
            "focus": focus_payload,
            "terminal_focus": terminal_focus_payload,
            "command_elapsed_ms": {
                "focus": focus_payload.get("_elapsed_ms") if isinstance(focus_payload, dict) else None,
                "terminal_focus": terminal_focus_payload.get("_elapsed_ms")
                if isinstance(terminal_focus_payload, dict)
                else None,
                "state": state_payload.get("_elapsed_ms"),
                "rows": rows_payload.get("_elapsed_ms"),
                "screenshot": screenshot_payload.get("_elapsed_ms"),
                "post_screenshot_state": post_screenshot_state_payload.get("_elapsed_ms"),
                "cpu": cpu.get("elapsed_ms") if isinstance(cpu, dict) else None,
            },
            "cpu": cpu,
        }
    )
    return summary


def process_match_script(session_id: str) -> str:
    return f"""YGGTERM_CHECK_SESSION_ID={shlex.quote(session_id)} python3 - <<'PY'
import json, os
needle = os.environ.get('YGGTERM_CHECK_SESSION_ID', '')
ancestors = set()
pid = os.getpid()
while pid > 1 and pid not in ancestors:
    ancestors.add(pid)
    try:
        stat = open(f'/proc/{{pid}}/stat').read()
        pid = int(stat.rsplit(')', 1)[1].split()[1])
    except Exception:
        break
rows = []
for name in os.listdir('/proc'):
    if not name.isdigit():
        continue
    proc_pid = int(name)
    if proc_pid in ancestors:
        continue
    try:
        stat = open(f'/proc/{{proc_pid}}/stat').read().rsplit(')', 1)[1].split()
        raw = open(f'/proc/{{proc_pid}}/cmdline', 'rb').read().replace(b'\\0', b' ').decode('utf-8', 'replace').strip()
        comm = open(f'/proc/{{proc_pid}}/comm').read().strip()
    except Exception:
        continue
    if 'YGGTERM_CHECK_SESSION_ID=' in raw or 'smoke_codex_launch_timeline.py' in raw:
        continue
    if needle and needle in raw:
        rows.append({{'pid': proc_pid, 'ppid': int(stat[1]), 'state': stat[0], 'comm': comm, 'cmd': raw[:500]}})
print(json.dumps(rows))
PY"""


def collect_process_matches(
    runner: Runner,
    session_id: str,
    *,
    host_label: str,
    ssh_target: str | None = None,
) -> tuple[list[dict[str, Any]], dict[str, Any] | None]:
    script = process_match_script(session_id)
    if ssh_target:
        script = f"ssh {shlex.quote(ssh_target)} {shlex.quote(script)}"
    result = runner.shell(script, timeout=15, check=False)
    if result.returncode != 0:
        return [], {
            "host": host_label,
            "returncode": result.returncode,
            "stdout": result.stdout[-1000:],
            "stderr": result.stderr[-1000:],
        }
    try:
        rows = json.loads(result.stdout or "[]")
    except Exception as error:
        return [], {"host": host_label, "error": str(error), "stdout": result.stdout[-1000:]}
    if not isinstance(rows, list):
        return [], {"host": host_label, "error": "process probe returned non-list JSON"}
    normalized = []
    for row in rows:
        if isinstance(row, dict):
            row = dict(row)
            row["host"] = host_label
            normalized.append(row)
    return normalized, None


def cleanup_session(
    runner: Runner,
    session_path: str,
    timeout_ms: int,
    *,
    remote_process_host: str | None = None,
    process_retries: int = 3,
    retry_delay_sec: float = 1.0,
) -> dict[str, Any]:
    result: dict[str, Any] = {"session_path": session_path}
    try:
        result["remove"] = runner.run_json(
            ["server", "app", "session", "remove", session_path, "--timeout-ms", str(timeout_ms)],
            timeout=max(30.0, timeout_ms / 1000.0 + 5.0),
            check=False,
        )
    except Exception as error:
        result["remove_error"] = str(error)
    session_id = session_path.rstrip("/").split("/")[-1]
    if session_id:
        attempts = []
        process_matches: list[dict[str, Any]] = []
        errors = []
        retries = max(1, process_retries)
        for attempt in range(1, retries + 1):
            attempt_matches = []
            local_rows, local_error = collect_process_matches(
                runner,
                session_id,
                host_label="app_host",
            )
            attempt_matches.extend(local_rows)
            if local_error:
                errors.append(local_error)
            if remote_process_host:
                remote_rows, remote_error = collect_process_matches(
                    runner,
                    session_id,
                    host_label=remote_process_host,
                    ssh_target=remote_process_host,
                )
                attempt_matches.extend(remote_rows)
                if remote_error:
                    errors.append(remote_error)
            attempts.append({"attempt": attempt, "matches": attempt_matches})
            process_matches = attempt_matches
            if not attempt_matches:
                break
            if attempt < retries:
                try:
                    result.setdefault("remove_retries", []).append(
                        runner.run_json(
                            [
                                "server",
                                "app",
                                "session",
                                "remove",
                                session_path,
                                "--timeout-ms",
                                str(timeout_ms),
                            ],
                            timeout=max(30.0, timeout_ms / 1000.0 + 5.0),
                            check=False,
                        )
                    )
                except Exception as error:
                    result.setdefault("remove_retry_errors", []).append(str(error))
                time.sleep(max(0.0, retry_delay_sec))
        result["process_match_attempts"] = attempts
        result["process_matches"] = process_matches
        if errors:
            result["process_match_errors"] = errors
    return result


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    if not path.exists():
        return rows
    for line in path.read_text(errors="replace").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            payload = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(payload, dict):
            rows.append(payload)
    return rows


def phase_match_key(phase: dict[str, Any]) -> tuple[Any, ...]:
    return (
        phase.get("phase"),
        phase.get("label"),
        phase.get("offset_sec"),
    )


def resource_phase_name(start_phase: str) -> str:
    return start_phase.removesuffix("_start").removesuffix("_end")


def resource_process_category(row: dict[str, Any], *, yggterm_home: str | None) -> str:
    cmd = str(row.get("cmd") or "")
    comm = str(row.get("comm") or "")
    env = row.get("env") if isinstance(row.get("env"), dict) else {}
    proc_home = str(env.get("YGGTERM_HOME") or "")
    if row.get("resource_logger"):
        return "resource_logger"
    if yggterm_home and (proc_home == yggterm_home or yggterm_home in cmd):
        return "test_yggterm_home"
    if proc_home == "/home/pi/.yggterm" or "/.local/share/yggterm/direct/versions/" in cmd:
        return "live_yggterm_home"
    hay = f"{comm} {cmd}".lower()
    if "codex" in hay:
        return "codex_or_prompt"
    if "webkit" in hay:
        return "webkit_unattributed"
    if "ssh" in hay:
        return "ssh_other"
    return "other_matched"


def summarize_resource_window(
    samples: list[dict[str, Any]],
    *,
    start_ms: int,
    end_ms: int,
    yggterm_home: str | None,
) -> dict[str, Any]:
    window = [
        sample
        for sample in samples
        if isinstance(sample.get("elapsed_ms"), int) and start_ms <= int(sample["elapsed_ms"]) <= end_ms
    ]
    if not window:
        return {"sample_count": 0}
    matched_cpu = [float(sample.get("matched_cpu_percent") or 0.0) for sample in window]
    logger_cpu = [float(sample.get("resource_logger_cpu_percent") or 0.0) for sample in window]
    top_processes: list[dict[str, Any]] = []
    per_sample_categories: list[dict[str, float]] = []
    for sample in window:
        category_totals: dict[str, float] = {}
        for row in sample.get("matched") or []:
            if not isinstance(row, dict):
                continue
            category = resource_process_category(row, yggterm_home=yggterm_home)
            category_totals[category] = category_totals.get(category, 0.0) + float(
                row.get("cpu_percent") or 0.0
            )
            top_processes.append(
                {
                    "pid": row.get("pid"),
                    "comm": row.get("comm"),
                    "cpu_percent": row.get("cpu_percent"),
                    "rss_kb": row.get("rss_kb"),
                    "cmd": row.get("cmd"),
                }
            )
        per_sample_categories.append(category_totals)
    top_processes.sort(key=lambda row: -float(row.get("cpu_percent") or 0.0))
    category_names = sorted({name for sample in per_sample_categories for name in sample})
    category_cpu_percent = {
        name: {
            "avg": round(
                sum(sample.get(name, 0.0) for sample in per_sample_categories)
                / max(1, len(per_sample_categories)),
                3,
            ),
            "max": round(
                max((sample.get(name, 0.0) for sample in per_sample_categories), default=0.0),
                3,
            ),
        }
        for name in category_names
    }
    top_categories = sorted(
        (
            {"category": category, **summary}
            for category, summary in category_cpu_percent.items()
        ),
        key=lambda row: -float(row.get("avg") or 0.0),
    )
    return {
        "sample_count": len(window),
        "elapsed_start_ms": start_ms,
        "elapsed_end_ms": end_ms,
        "duration_ms": max(0, end_ms - start_ms),
        "matched_cpu_percent_avg": round(sum(matched_cpu) / max(1, len(matched_cpu)), 3),
        "matched_cpu_percent_max": round(max(matched_cpu), 3),
        "resource_logger_cpu_percent_max": round(max(logger_cpu), 3),
        "category_cpu_percent": category_cpu_percent,
        "top_categories": top_categories[:8],
        "top_matched_processes": top_processes[:6],
    }


def summarize_resource_trace(
    resource_log: Path,
    phase_file: Path,
    *,
    yggterm_home: str | None,
) -> dict[str, Any]:
    resource_rows = [row for row in read_jsonl(resource_log) if "elapsed_ms" in row and "event" not in row]
    phase_rows = read_jsonl(phase_file)
    pending: dict[tuple[Any, ...], dict[str, Any]] = {}
    windows: list[dict[str, Any]] = []
    for phase in phase_rows:
        name = str(phase.get("phase") or "")
        elapsed = phase.get("resource_elapsed_ms")
        if not isinstance(elapsed, int):
            continue
        if name.endswith("_start"):
            key = phase_match_key({**phase, "phase": resource_phase_name(name)})
            pending[key] = phase
            continue
        if not name.endswith("_end"):
            continue
        base_name = resource_phase_name(name)
        key = phase_match_key({**phase, "phase": base_name})
        start = pending.pop(key, None)
        if not start:
            continue
        start_elapsed = start.get("resource_elapsed_ms")
        if not isinstance(start_elapsed, int):
            continue
        summary = summarize_resource_window(
            resource_rows,
            start_ms=start_elapsed,
            end_ms=elapsed,
            yggterm_home=yggterm_home,
        )
        summary.update(
            {
                "phase": base_name,
                "label": phase.get("label"),
                "session_path": phase.get("session_path"),
                "offset_sec": phase.get("offset_sec"),
                "ready": phase.get("ready"),
                "problem": phase.get("problem"),
            }
        )
        windows.append(summary)
    baseline = next((window for window in windows if window.get("phase") == "pretest_baseline"), None)
    return {
        "resource_sample_count": len(resource_rows),
        "category_hints": {
            "test_yggterm_home": yggterm_home,
            "live_yggterm_home": "/home/pi/.yggterm",
        },
        "baseline": baseline,
        "phase_windows": windows,
    }


def run_launch_group(
    args: argparse.Namespace,
    runner: Runner,
    *,
    label_prefix: str,
    count: int,
    remote_machine_key: str | None,
    cwd: str,
    pid: int,
    local_capture_dir: Path,
    remote_capture_dir: str,
    summary_file: Path,
    phase_file: Path,
) -> list[dict[str, Any]]:
    launches: list[dict[str, Any]] = []
    for index in range(1, count + 1):
        label = f"{label_prefix}_{index}"
        title = f"timeline {label} {uuid.uuid4().hex[:8]}"
        record_phase(phase_file, "terminal_new_start", label=label, cwd=cwd, remote=bool(remote_machine_key))
        terminal_new = launch_terminal(
            runner,
            remote_machine_key=remote_machine_key,
            cwd=cwd,
            title=title,
            timeout_ms=args.timeout_ms,
        )
        session_path = str(terminal_new["session_path"])
        focus_payload: dict[str, Any] | None = None
        if not args.no_focus_before_capture:
            record_phase(phase_file, "focus_start", label=label, session_path=session_path)
            focus_payload = focus_owned_window(
                runner,
                pid=pid,
                timeout_ms=args.timeout_ms,
                command_timeout_sec=args.command_timeout_sec,
            )
            record_phase(
                phase_file,
                "focus_end",
                label=label,
                session_path=session_path,
                elapsed_ms=focus_payload.get("_elapsed_ms") if isinstance(focus_payload, dict) else None,
                returncode=focus_payload.get("returncode") if isinstance(focus_payload, dict) else None,
            )
        record_phase(
            phase_file,
            "terminal_new_end",
            label=label,
            session_path=session_path,
            elapsed_ms=terminal_new.get("elapsed_ms"),
        )
        started = time.perf_counter()
        samples: list[dict[str, Any]] = []
        for offset_sec in args.offsets:
            record_phase(phase_file, "capture_start", label=label, session_path=session_path, offset_sec=offset_sec)
            sample = capture_one(
                args,
                runner,
                label=label,
                session_path=session_path,
                offset_sec=offset_sec,
                started=started,
                local_capture_dir=local_capture_dir,
                remote_capture_dir=remote_capture_dir,
                pid=pid,
            )
            record_phase(
                phase_file,
                "capture_end",
                label=label,
                session_path=session_path,
                offset_sec=offset_sec,
                ready=sample.get("ready"),
                problem=sample.get("problem"),
            )
            samples.append(sample)
            with summary_file.open("a") as handle:
                handle.write(json.dumps(sample, sort_keys=True) + "\n")
        record_phase(phase_file, "cleanup_start", label=label, session_path=session_path)
        cleanup = cleanup_session(
            runner,
            session_path,
            args.timeout_ms,
            remote_process_host=(args.remote_ssh_target or remote_machine_key)
            if remote_machine_key
            else None,
            process_retries=args.cleanup_process_retries,
            retry_delay_sec=args.cleanup_retry_delay_sec,
        )
        record_phase(
            phase_file,
            "cleanup_end",
            label=label,
            session_path=session_path,
            process_match_count=len(cleanup.get("process_matches") or []),
        )
        launch = {
            "label": label,
            "session_path": session_path,
            "terminal_new": terminal_new,
            "focus": focus_payload,
            "samples": samples,
            "cleanup": cleanup,
        }
        launches.append(launch)
    return launches


def summarize_failures(args: argparse.Namespace, launches: list[dict[str, Any]]) -> list[str]:
    failures: list[str] = []
    for launch in launches:
        label = str(launch.get("label"))
        samples = launch.get("samples") if isinstance(launch.get("samples"), list) else []
        ready_samples = [sample for sample in samples if isinstance(sample, dict) and sample.get("ready")]
        if not ready_samples:
            failures.append(f"{label}: no ready Codex surface by {max(args.offsets):.1f}s")
        elif isinstance(samples[-1], dict) and not samples[-1].get("ready"):
            failures.append(
                f"{label}: final sample at {samples[-1].get('offset_sec')}s was not ready "
                f"({samples[-1].get('readiness_class')}): {samples[-1].get('problem')}"
            )
        late_ready = [
            sample
            for sample in ready_samples
            if float(sample.get("offset_sec") or 0.0) <= args.ready_deadline_sec
        ]
        if not late_ready:
            failures.append(f"{label}: first ready Codex surface missed {args.ready_deadline_sec:.1f}s deadline")
        if ready_samples:
            first_ready_offset = float(ready_samples[0].get("offset_sec") or 0.0)
            regressions = [
                sample
                for sample in samples
                if isinstance(sample, dict)
                and float(sample.get("offset_sec") or 0.0) > first_ready_offset
                and not sample.get("ready")
            ]
            if regressions:
                first_regression = regressions[0]
                failures.append(
                    f"{label}: readiness regressed after {first_ready_offset:.1f}s; "
                    f"{first_regression.get('offset_sec')}s "
                    f"class={first_regression.get('readiness_class')} "
                    f"problem={first_regression.get('problem')}"
                )
        for sample in samples:
            if not isinstance(sample, dict):
                continue
            if sample.get("dom_error"):
                failures.append(
                    f"{label}: app-control DOM snapshot failed at {sample.get('offset_sec')}s: {sample.get('dom_error')}"
                )
            if sample.get("problem") and sample.get("runtime_active_host_input_enabled") is True:
                failures.append(
                    f"{label}: runtime input enabled while problem was reported at {sample.get('offset_sec')}s: {sample.get('problem')}"
                )
            if sample.get("surface_input_enabled") is True and sample.get("problem"):
                failures.append(
                    f"{label}: surface input enabled while problem was reported at {sample.get('offset_sec')}s: {sample.get('problem')}"
                )
            if sample.get("focus_command_state_disagrees") is True:
                failures.append(
                    f"{label}: app-control focus command/state disagreed at {sample.get('offset_sec')}s "
                    f"(focus_confirmed={sample.get('sample_focus_confirmed')}, "
                    f"focus_ready={sample.get('focus_ready')})"
                )
            if sample.get("terminal_focus_command_state_disagrees") is True:
                failures.append(
                    f"{label}: terminal focus command/state disagreed at {sample.get('offset_sec')}s "
                    f"(terminal_focus_accepted={sample.get('sample_terminal_focus_accepted')}, "
                    f"focus_ready={sample.get('focus_ready')})"
                )
            if sample.get("render_ready") is True and sample.get("input_ready") is not True:
                if sample.get("input_blocked_by_focus") is True:
                    failures.append(
                        f"{label}: prompt rendered but app/document focus was not retained at {sample.get('offset_sec')}s "
                        f"(window_focused={sample.get('window_focused')}, "
                        f"document_focused={sample.get('document_focused')})"
                    )
                    continue
                failures.append(
                    f"{label}: prompt rendered but terminal input was not ready at {sample.get('offset_sec')}s "
                    f"(window_focused={sample.get('window_focused')}, document_focused={sample.get('document_focused')})"
                )
            if sample.get("ready") and sample.get("screenshot_blank") is True:
                failures.append(f"{label}: app-control ready but screenshot was blank at {sample.get('offset_sec')}s")
        cleanup = launch.get("cleanup") if isinstance(launch.get("cleanup"), dict) else {}
        process_matches = cleanup.get("process_matches")
        if isinstance(process_matches, list) and process_matches:
            failures.append(
                f"{label}: cleanup left matching process/session id: {json.dumps(process_matches[:3])[:360]}"
            )
    return failures


def parse_offsets(raw: str) -> list[float]:
    values = [float(part.strip()) for part in raw.split(",") if part.strip()]
    if not values:
        raise argparse.ArgumentTypeError("at least one offset is required")
    return sorted(values)


def main() -> int:
    global RESOURCE_ORIGIN_MONOTONIC_MS
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", help="SSH host for running the isolated GUI")
    parser.add_argument("--bin", default=str(ROOT / "target" / "debug" / "yggterm"))
    parser.add_argument("--output-dir", type=Path)
    parser.add_argument("--remote-output-dir")
    parser.add_argument("--remote-base-dir", default="/home/pi/.cache")
    parser.add_argument("--yggterm-home")
    parser.add_argument("--display")
    parser.add_argument("--xauthority")
    parser.add_argument("--force-x11", action="store_true")
    parser.add_argument("--local-count", type=int, default=1)
    parser.add_argument("--remote-count", type=int, default=0)
    parser.add_argument("--local-cwd", default="/home/pi")
    parser.add_argument("--remote-cwd", default="/home/pi")
    parser.add_argument("--remote-machine-key")
    parser.add_argument("--remote-ssh-target")
    parser.add_argument("--remote-prefix")
    parser.add_argument("--remote-binary-expr", default="~/.yggterm/bin/yggterm")
    parser.add_argument("--offsets", type=parse_offsets, default=list(DEFAULT_OFFSETS_SEC))
    parser.add_argument("--ready-deadline-sec", type=float, default=10.0)
    parser.add_argument("--baseline-sec", type=float, default=5.0)
    parser.add_argument("--resource-interval-sec", type=float, default=1.0)
    parser.add_argument("--no-resource-log", action="store_true")
    parser.add_argument("--no-focus-before-capture", action="store_true")
    parser.add_argument("--cleanup-process-retries", type=int, default=3)
    parser.add_argument("--cleanup-retry-delay-sec", type=float, default=1.0)
    parser.add_argument("--timeout-ms", type=int, default=12_000)
    parser.add_argument("--command-timeout-sec", type=float, default=25.0)
    parser.add_argument("--keep-open", action="store_true")
    parser.add_argument("--allow-failures", action="store_true")
    args = parser.parse_args()

    stamp = time.strftime("%Y%m%d-%H%M%S")
    local_output_dir = args.output_dir or Path(f"/tmp/yggterm-codex-launch-timeline-{stamp}")
    local_output_dir.mkdir(parents=True, exist_ok=True)
    remote_base_dir = args.remote_base_dir.rstrip("/") or "/home/pi/.cache"
    remote_output_dir = args.remote_output_dir or (
        f"{remote_base_dir}/{local_output_dir.name}" if args.host else f"/tmp/{local_output_dir.name}"
    )
    yggterm_home = args.yggterm_home or (
        f"{remote_base_dir}/{local_output_dir.name}-home"
        if args.host
        else f"/tmp/{local_output_dir.name}-home"
    )
    suffix = f"timeline-{uuid.uuid4().hex[:8]}"
    runner = Runner(args, yggterm_home, suffix)
    summary_file = local_output_dir / "summary.jsonl"
    phase_file = local_output_dir / "phase_trace.jsonl"
    local_resource_log = local_output_dir / "resource_timeline.jsonl"
    report_file = local_output_dir / "report.json"
    captures_dir = local_output_dir / "captures"
    remote_resource_log = f"{remote_output_dir}/resource_timeline.jsonl"
    remote_resource_script = f"{remote_output_dir}/resource_logger_script.json"
    resource_logger_pid: int | None = None

    record_phase(phase_file, "setup_start", host=args.host, yggterm_home=yggterm_home)
    runner.shell(f"mkdir -p {shlex.quote(yggterm_home)} {shlex.quote(remote_output_dir)}", timeout=10)
    storage = storage_preflight(runner, [yggterm_home, remote_output_dir])
    seed_remote_machine_state(args, runner)
    record_phase(phase_file, "preflight_versions_start")
    binary_versions = preflight_binary_versions(args, runner)
    record_phase(phase_file, "preflight_versions_end", **binary_versions)

    if not args.no_resource_log:
        record_phase(
            phase_file,
            "resource_logger_start",
            remote_resource_log=remote_resource_log,
            interval_sec=args.resource_interval_sec,
        )
        resource_logger_pid = start_resource_logger(
            args,
            runner,
            remote_resource_log=remote_resource_log,
            remote_resource_script=remote_resource_script,
            interval_sec=args.resource_interval_sec,
        )
        RESOURCE_ORIGIN_MONOTONIC_MS = int(time.monotonic() * 1000)
        record_phase(phase_file, "resource_logger_started", pid=resource_logger_pid)
        if args.baseline_sec > 0:
            record_phase(phase_file, "pretest_baseline_start", duration_sec=args.baseline_sec)
            time.sleep(args.baseline_sec)
            record_phase(phase_file, "pretest_baseline_end", duration_sec=args.baseline_sec)

    pid = 0
    launch_payload: dict[str, Any] = {}
    launches: list[dict[str, Any]] = []
    try:
        record_phase(phase_file, "app_launch_start")
        launch_payload = runner.run_gui_json(
            [
                "server",
                "app",
                "launch",
                "--wait-settled",
                "--allow-multi-window",
                "--skip-active-exec-handoff",
                "--timeout-ms",
                str(max(args.timeout_ms, 15_000)),
            ],
            timeout=max(30.0, args.timeout_ms / 1000.0 + 20.0),
        )
        record_phase(phase_file, "app_launch_end", elapsed_ms=launch_payload.get("_elapsed_ms"))
        launch_data = response_data(launch_payload)
        pid = int(launch_data.get("pid") or launch_payload.get("pid") or 0)
        if pid <= 0:
            raise RuntimeError(f"app launch did not return pid: {json.dumps(launch_payload)[:1000]}")
        if not args.no_focus_before_capture:
            record_phase(phase_file, "app_focus_start", pid=pid)
            app_focus_payload = focus_owned_window(
                runner,
                pid=pid,
                timeout_ms=args.timeout_ms,
                command_timeout_sec=args.command_timeout_sec,
            )
            record_phase(
                phase_file,
                "app_focus_end",
                pid=pid,
                elapsed_ms=app_focus_payload.get("_elapsed_ms") if isinstance(app_focus_payload, dict) else None,
                returncode=app_focus_payload.get("returncode") if isinstance(app_focus_payload, dict) else None,
            )

        if args.local_count > 0:
            launches.extend(
                run_launch_group(
                    args,
                    runner,
                    label_prefix="local_codex",
                    count=args.local_count,
                    remote_machine_key=None,
                    cwd=args.local_cwd,
                    pid=pid,
                    local_capture_dir=captures_dir,
                    remote_capture_dir=remote_output_dir,
                    summary_file=summary_file,
                    phase_file=phase_file,
                )
            )
        if args.remote_count > 0:
            if not args.remote_machine_key:
                raise RuntimeError("--remote-count requires --remote-machine-key")
            launches.extend(
                run_launch_group(
                    args,
                    runner,
                    label_prefix=f"{args.remote_machine_key}_codex",
                    count=args.remote_count,
                    remote_machine_key=args.remote_machine_key,
                    cwd=args.remote_cwd,
                    pid=pid,
                    local_capture_dir=captures_dir,
                    remote_capture_dir=remote_output_dir,
                    summary_file=summary_file,
                    phase_file=phase_file,
                )
            )
    finally:
        if pid > 0 and not args.keep_open:
            record_phase(phase_file, "app_close_start", pid=pid)
            runner.run(["server", "app", "close", "--pid", str(pid), "--timeout-ms", str(args.timeout_ms)], timeout=args.command_timeout_sec, check=False)
            runner.run(["server", "shutdown"], timeout=args.command_timeout_sec, check=False)
            record_phase(phase_file, "app_close_end", pid=pid)
        if resource_logger_pid:
            record_phase(phase_file, "resource_logger_stop", pid=resource_logger_pid)
            stop_resource_logger(runner, resource_logger_pid)
            try:
                runner.pull_file(remote_resource_log, local_resource_log)
                record_phase(
                    phase_file,
                    "resource_log_pulled",
                    resource_log_path=str(local_resource_log),
                    bytes=local_resource_log.stat().st_size,
                )
            except Exception as error:
                record_phase(phase_file, "resource_log_pull_failed", error=str(error))

    failures = summarize_failures(args, launches)
    resource_summary = (
        summarize_resource_trace(local_resource_log, phase_file, yggterm_home=yggterm_home)
        if local_resource_log.exists()
        else None
    )
    report = {
        "ok": not failures,
        "failures": failures,
        "output_dir": str(local_output_dir),
        "summary_jsonl": str(summary_file),
        "phase_trace_jsonl": str(phase_file),
        "resource_timeline_jsonl": str(local_resource_log) if local_resource_log.exists() else None,
        "baseline_sec": args.baseline_sec if not args.no_resource_log else 0.0,
        "resource_interval_sec": args.resource_interval_sec if not args.no_resource_log else None,
        "resource_summary": resource_summary,
        "storage_preflight": storage,
        "yggterm_home": yggterm_home,
        "remote_output_dir": remote_output_dir,
        "remote_base_dir": remote_base_dir if args.host else None,
        "host": args.host,
        "pid": pid,
        "binary_versions": binary_versions,
        "launch": launch_payload,
        "launches": [
            {
                "label": launch["label"],
                "session_path": launch["session_path"],
                "terminal_new_elapsed_ms": (launch.get("terminal_new") or {}).get("elapsed_ms"),
                "focus": launch.get("focus"),
                "first_ready_offset_sec": next(
                    (
                        sample.get("offset_sec")
                        for sample in launch.get("samples", [])
                        if isinstance(sample, dict) and sample.get("ready")
                    ),
                    None,
                ),
                "first_render_ready_offset_sec": next(
                    (
                        sample.get("offset_sec")
                        for sample in launch.get("samples", [])
                        if isinstance(sample, dict) and sample.get("render_ready")
                    ),
                    None,
                ),
                "sample_statuses": [
                    {
                        "offset_sec": sample.get("offset_sec"),
                        "ready": sample.get("ready"),
                        "render_ready": sample.get("render_ready"),
                        "input_ready": sample.get("input_ready"),
                        "focus_ready": sample.get("focus_ready"),
                        "problem": sample.get("problem"),
                        "focus_problem": sample.get("focus_problem"),
                        "readiness_class": sample.get("readiness_class"),
                        "input_blocked_by_focus": sample.get("input_blocked_by_focus"),
                        "input_enabled_without_focus": sample.get("input_enabled_without_focus"),
                        "window_focused": sample.get("window_focused"),
                        "terminal_input_override_active": sample.get(
                            "terminal_input_override_active"
                        ),
                        "document_focused": sample.get("document_focused"),
                        "effective_terminal_input_focus": sample.get(
                            "effective_terminal_input_focus"
                        ),
                        "host_effective_input_focus": sample.get("host_effective_input_focus"),
                        "host_helper_textarea_focused": sample.get(
                            "host_helper_textarea_focused"
                        ),
                        "host_has_active_element": sample.get("host_has_active_element"),
                        "runtime_active_host_ready": sample.get("runtime_active_host_ready"),
                        "runtime_active_host_input_enabled": sample.get(
                            "runtime_active_host_input_enabled"
                        ),
                        "runtime_active_host_effective_input_focus": sample.get(
                            "runtime_active_host_effective_input_focus"
                        ),
                        "surface_input_enabled": sample.get("surface_input_enabled"),
                        "surface_effective_input_focus": sample.get(
                            "surface_effective_input_focus"
                        ),
                        "sample_focus_confirmed": sample.get("sample_focus_confirmed"),
                        "focus_command_state_disagrees": sample.get("focus_command_state_disagrees"),
                        "sample_terminal_focus_accepted": sample.get("sample_terminal_focus_accepted"),
                        "terminal_focus_command_state_disagrees": sample.get(
                            "terminal_focus_command_state_disagrees"
                        ),
                        "screenshot": sample.get("screenshot"),
                        "state_json": sample.get("state_json"),
                        "rows_json": sample.get("rows_json"),
                        "screenshot_state_json": sample.get("screenshot_state_json"),
                    }
                    for sample in launch.get("samples", [])
                    if isinstance(sample, dict)
                ],
                "cleanup": launch.get("cleanup"),
            }
            for launch in launches
        ],
    }
    report_file.write_text(json.dumps(report, indent=2))
    print(json.dumps(report, indent=2))
    return 0 if (report["ok"] or args.allow_failures) else 1


if __name__ == "__main__":
    sys.exit(main())
