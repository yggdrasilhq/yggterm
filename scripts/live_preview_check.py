#!/usr/bin/env python3
import argparse
import os
import subprocess
import sys
import time


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--display", default=":99")
    parser.add_argument("--delay-first", type=float, default=8.0)
    parser.add_argument("--delay-second", type=float, default=10.0)
    parser.add_argument(
        "--first-out",
        default="/tmp/yggterm-live-check-1.png",
    )
    parser.add_argument(
        "--second-out",
        default="/tmp/yggterm-live-check-2.png",
    )
    parser.add_argument(
        "--log",
        default="/tmp/yggterm-live-preview.log",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    out1 = args.first_out
    out2 = args.second_out
    display = args.display
    xvfb_log_path = f"/tmp/xvfb{display.lstrip(':')}.log"
    subprocess.run(["pkill", "-f", f"Xvfb {display}"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    xvfb_log = open(xvfb_log_path, "w")
    app_log = open(args.log, "w")
    xvfb = subprocess.Popen(
        ["Xvfb", display, "-screen", "0", "1600x900x24"],
        stdout=xvfb_log,
        stderr=subprocess.STDOUT,
    )
    app = None
    try:
        time.sleep(1)
        env = os.environ.copy()
        env["DISPLAY"] = display
        env["RUST_BACKTRACE"] = "full"
        app = subprocess.Popen(
            ["./target/debug/yggterm"],
            cwd="/home/pi/gh/yggterm",
            env=env,
            stdout=app_log,
            stderr=subprocess.STDOUT,
        )
        time.sleep(args.delay_first)
        subprocess.run(["import", "-window", "root", out1], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run("xdotool search --name yggterm windowactivate --sync %@",
                       shell=True, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(max(0.0, args.delay_second - args.delay_first))
        subprocess.run(["import", "-window", "root", out2], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        print(out1)
        print(out2)
        return 0
    finally:
        if app is not None:
            app.terminate()
            try:
                app.wait(timeout=2)
            except subprocess.TimeoutExpired:
                app.kill()
        xvfb.terminate()
        try:
            xvfb.wait(timeout=2)
        except subprocess.TimeoutExpired:
            xvfb.kill()
        xvfb_log.close()
        app_log.close()


if __name__ == "__main__":
    raise SystemExit(main())
