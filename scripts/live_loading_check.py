#!/usr/bin/env python3
import os
import shlex
import signal
import subprocess
import time
from pathlib import Path


ROOT = Path("/home/pi/gh/yggterm")
OUT_DIR = Path("/tmp")
DISPLAY = os.environ.get("YGGTERM_TEST_DISPLAY", ":102")
RUNTIME_ROOT = OUT_DIR / f"yggterm-x11-runtime-{DISPLAY.replace(':', '')}"


def run(cmd, **kwargs):
    return subprocess.run(cmd, check=True, **kwargs)


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
        ["bash", "-lc", f"Xvfb {DISPLAY} -screen 0 1600x1000x24 >/tmp/xvfb-live-loading.log 2>&1 &"],
        check=True,
    )
    for _ in range(20):
        time.sleep(0.2)
        if display_ready(DISPLAY):
            return
    raise RuntimeError(
        f"failed to start Xvfb on {DISPLAY}: "
        + Path("/tmp/xvfb-live-loading.log").read_text(encoding="utf-8", errors="replace")
    )


def child_pids(pid: int) -> list[int]:
    try:
        output = subprocess.check_output(["bash", "-lc", f"pgrep -P {pid} || true"], text=True)
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
        if "target/debug/yggterm" in cmdline and "mock-yggclient" not in cmdline:
            return candidate
    return launcher_pid


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


def find_yggterm_window(env: dict[str, str], pid: int | None = None) -> tuple[str, str, str]:
    if pid is not None:
        try:
            pid_output = subprocess.check_output(
                ["bash", "-lc", f"DISPLAY={DISPLAY} xdotool search --onlyvisible --pid {pid} || true"],
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
        if "yggterm" in name.lower():
            score += 3
        if "yggterm" in klass.lower():
            score += 4
        if score:
            scored.append((score, window_id, name, klass))
    scored.sort(reverse=True)
    if not scored:
        raise RuntimeError("could not find yggterm window on test display")
    _, window_id, name, klass = scored[0]
    return window_id, name, klass


def capture(name: str, window_id: str):
    path = OUT_DIR / name
    run(["import", "-display", DISPLAY, "-window", window_id, str(path)])
    return path


def main() -> int:
    ensure_xvfb()
    RUNTIME_ROOT.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env["DISPLAY"] = DISPLAY
    env["XDG_RUNTIME_DIR"] = str(RUNTIME_ROOT)
    env["GDK_BACKEND"] = "x11"
    env["YGGTERM_DEBUG_DISABLE_CACHED_SERVER_SNAPSHOT"] = "1"
    env["YGGTERM_DEBUG_REQUEST_DELAY_MS"] = env.get("YGGTERM_DEBUG_REQUEST_DELAY_MS", "4200")
    os.environ["DISPLAY"] = DISPLAY

    subprocess.run(
        f"pkill -f 'DISPLAY={DISPLAY} ./target/debug/yggterm' || true",
        shell=True,
        check=False,
    )
    app = subprocess.Popen(
        "dbus-run-session -- bash -lc 'export DISPLAY="
        + DISPLAY
        + " XDG_RUNTIME_DIR="
        + shlex.quote(str(RUNTIME_ROOT))
        + " GDK_BACKEND=x11"
        + " YGGTERM_DEBUG_DISABLE_CACHED_SERVER_SNAPSHOT=1"
        + " YGGTERM_DEBUG_REQUEST_DELAY_MS="
        + shlex.quote(env["YGGTERM_DEBUG_REQUEST_DELAY_MS"])
        + "; RUST_BACKTRACE=full ./target/debug/yggterm >/tmp/yggterm-live-loading.log 2>&1'",
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
        time.sleep(1.2)
        early = capture("yggterm-live-loading-early.png", win)
        time.sleep(3.8)
        late = capture("yggterm-live-loading-late.png", win)
        print(f"launcher_pid={app.pid} target_pid={target_pid} window={win} name={name}")
        print(klass)
        print(early)
        print(late)
        return 0
    finally:
        try:
            os.killpg(os.getpgid(app.pid), signal.SIGTERM)
        except ProcessLookupError:
            pass


if __name__ == "__main__":
    raise SystemExit(main())
