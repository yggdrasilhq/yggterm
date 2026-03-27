#!/usr/bin/env python3
import os
import shlex
import signal
import subprocess
import sys
import time
from pathlib import Path


ROOT = Path("/home/pi/gh/yggterm")
OUT_DIR = Path("/tmp")
DISPLAY = os.environ.get("YGGTERM_TEST_DISPLAY", ":101")
RUNTIME_ROOT = OUT_DIR / f"yggterm-x11-runtime-{DISPLAY.replace(':', '')}"
TELEMETRY_PATH = Path.home() / ".yggterm" / "ui-telemetry.jsonl"
SERVER_STATE_PATH = Path.home() / ".yggterm" / "server-state.json"


def run(cmd, **kwargs):
    return subprocess.run(cmd, check=True, **kwargs)


def shell(cmd: str, **kwargs):
    return subprocess.run(cmd, shell=True, check=True, **kwargs)


def display_ready(display: str) -> bool:
    return (
        subprocess.run(
            ["bash", "-lc", f"DISPLAY={display} xdpyinfo >/dev/null 2>&1"],
            check=False,
        ).returncode
        == 0
    )


def ensure_xvfb():
    if display_ready(DISPLAY):
        return
    subprocess.run(
        ["bash", "-lc", f"Xvfb {DISPLAY} -screen 0 1600x1000x24 >/tmp/xvfb-live-search.log 2>&1 &"],
        check=True,
    )
    for _ in range(20):
        time.sleep(0.2)
        if display_ready(DISPLAY):
            return
    raise RuntimeError(
        f"failed to start Xvfb on {DISPLAY}: "
        + Path("/tmp/xvfb-live-search.log").read_text(encoding="utf-8", errors="replace")
    )


def capture(name: str, window_id: str):
    path = OUT_DIR / name
    run(["import", "-display", DISPLAY, "-window", window_id, str(path)])
    return path


def recent_search_telemetry(after_size: int) -> str:
    if not TELEMETRY_PATH.exists():
        return ""
    with TELEMETRY_PATH.open("r", encoding="utf-8", errors="replace") as handle:
        handle.seek(after_size)
        return handle.read()


def derive_search_query() -> str:
    if not SERVER_STATE_PATH.exists():
        return 'excel "shortcut design"'
    try:
        import json

        obj = json.loads(SERVER_STATE_PATH.read_text(encoding="utf-8", errors="replace"))
        active = obj.get("active_session_path")
        if not active:
            return 'excel "shortcut design"'
        for item in obj.get("stored_sessions", []):
            if item.get("path") == active and item.get("title_hint"):
                title = str(item["title_hint"]).strip()
                if title:
                    return title
        for item in obj.get("live_sessions", []):
            if item.get("key") == active and item.get("title"):
                title = str(item["title"]).strip()
                if title:
                    return title
    except Exception:
        pass
    return 'excel "shortcut design"'


def xprop_wm_class(window_id: str, env: dict[str, str]) -> str:
    try:
        return subprocess.check_output(
            ["bash", "-lc", f"xprop -id {shlex.quote(window_id)} WM_CLASS"],
            env=env,
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
    except subprocess.CalledProcessError:
        return ""


def visible_window_candidates(env: dict[str, str]) -> list[str]:
    try:
        output = subprocess.check_output(
            ["bash", "-lc", f"DISPLAY={DISPLAY} xdotool search --onlyvisible --name . || true"],
            env=env,
            text=True,
        )
    except subprocess.CalledProcessError:
        return []
    return [line.strip() for line in output.splitlines() if line.strip()]


def child_pids(pid: int) -> list[int]:
    try:
        output = subprocess.check_output(
            ["bash", "-lc", f"pgrep -P {pid} || true"],
            text=True,
        )
    except subprocess.CalledProcessError:
        return []
    return [int(line.strip()) for line in output.splitlines() if line.strip()]


def descendant_pids(pid: int) -> list[int]:
    pending = [pid]
    seen: set[int] = set()
    result: list[int] = []
    while pending:
        current = pending.pop()
        for child in child_pids(current):
            if child in seen:
                continue
            seen.add(child)
            result.append(child)
            pending.append(child)
    return result


def resolve_yggterm_pid(launcher_pid: int) -> int:
    for candidate in reversed(descendant_pids(launcher_pid)):
        try:
            cmdline = Path(f"/proc/{candidate}/cmdline").read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        if "target/debug/yggterm" in cmdline and "yggterm-mock-cli" not in cmdline:
            return candidate
    return launcher_pid


def find_yggterm_window(env: dict[str, str], pid: int | None = None) -> tuple[str, str, str]:
    if pid is not None:
        try:
            pid_output = subprocess.check_output(
                [
                    "bash",
                    "-lc",
                    f"DISPLAY={DISPLAY} xdotool search --onlyvisible --pid {pid} || true",
                ],
                env=env,
                text=True,
            )
            for window_id in [line.strip() for line in pid_output.splitlines() if line.strip()]:
                try:
                    name = subprocess.check_output(
                        ["xdotool", "getwindowname", window_id],
                        env=env,
                        text=True,
                        stderr=subprocess.DEVNULL,
                    ).strip()
                except subprocess.CalledProcessError:
                    continue
                return window_id, name, xprop_wm_class(window_id, env)
        except subprocess.CalledProcessError:
            pass
    candidates = visible_window_candidates(env)
    scored = []
    for window_id in candidates:
        try:
            name = subprocess.check_output(
                ["xdotool", "getwindowname", window_id],
                env=env,
                text=True,
                stderr=subprocess.DEVNULL,
            ).strip()
        except subprocess.CalledProcessError:
            continue
        klass = xprop_wm_class(window_id, env)
        score = 0
        lower_name = name.lower()
        lower_class = klass.lower()
        if "yggterm" in lower_name:
            score += 3
        if "yggterm" in lower_class:
            score += 4
        if "dioxus" in lower_class:
            score += 1
        if score:
            scored.append((score, window_id, name, klass))
    scored.sort(reverse=True)
    if not scored:
        raise RuntimeError(
            "could not find yggterm window on test display; candidates="
            + ", ".join(
                f"{window_id}:{subprocess.check_output(['xdotool','getwindowname',window_id], env=env, text=True, stderr=subprocess.DEVNULL).strip()}"
                for window_id in candidates[:12]
            )
        )
    _, window_id, name, klass = scored[0]
    return window_id, name, klass


def main() -> int:
    ensure_xvfb()
    RUNTIME_ROOT.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env["DISPLAY"] = DISPLAY
    env["XDG_RUNTIME_DIR"] = str(RUNTIME_ROOT)
    env["GDK_BACKEND"] = "x11"
    env["YGGTERM_SEARCH_QUERY"] = derive_search_query()
    os.environ["DISPLAY"] = DISPLAY
    subprocess.run(
        f"pkill -f 'DISPLAY={DISPLAY} ./target/debug/yggterm' || true",
        shell=True,
        check=False,
    )
    telemetry_size = TELEMETRY_PATH.stat().st_size if TELEMETRY_PATH.exists() else 0
    app = subprocess.Popen(
        "dbus-run-session -- bash -lc 'export DISPLAY="
        + DISPLAY
        + " XDG_RUNTIME_DIR="
        + shlex.quote(str(RUNTIME_ROOT))
        + " GDK_BACKEND=x11; "
        + "RUST_BACKTRACE=full ./target/debug/yggterm >/tmp/yggterm-live-search.log 2>&1'",
        cwd=ROOT,
        env=env,
        shell=True,
        preexec_fn=os.setsid,
    )
    try:
        target_pid = resolve_yggterm_pid(app.pid)
        for _ in range(20):
            time.sleep(0.5)
            try:
                target_pid = resolve_yggterm_pid(app.pid)
                win, name, klass = find_yggterm_window(env, target_pid)
                break
            except RuntimeError:
                continue
        else:
            target_pid = resolve_yggterm_pid(app.pid)
            win, name, klass = find_yggterm_window(env, target_pid)
        run(["xdotool", "windowfocus", win], env=env)
        time.sleep(1)
        time.sleep(1.4)
        filtered = capture("yggterm-live-search-filtered.png", win)
        run(["xdotool", "key", "--window", win, "bracketright"], env=env)
        time.sleep(0.8)
        stepped = capture("yggterm-live-search-next.png", win)
        run(["xdotool", "key", "--window", win, "Escape"], env=env)
        time.sleep(0.8)
        cleared = capture("yggterm-live-search-cleared.png", win)
        telemetry = recent_search_telemetry(telemetry_size)
        print(f"search_query={env['YGGTERM_SEARCH_QUERY']}")
        print(f"launcher_pid={app.pid} target_pid={target_pid} window={win} name={name}")
        print(klass)
        print(filtered)
        print(stepped)
        print(cleared)
        if telemetry:
            print("--- search telemetry ---")
            print(telemetry.strip())
        return 0
    finally:
        try:
            os.killpg(os.getpgid(app.pid), signal.SIGTERM)
        except ProcessLookupError:
            pass


if __name__ == "__main__":
    raise SystemExit(main())
