#!/usr/bin/env python3
import os
import subprocess
import sys
import time


def main() -> int:
    out1 = sys.argv[1] if len(sys.argv) > 1 else "/tmp/yggterm-live-check-1.png"
    out2 = sys.argv[2] if len(sys.argv) > 2 else "/tmp/yggterm-live-check-2.png"
    subprocess.run(["pkill", "-f", "Xvfb :99"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    xvfb_log = open("/tmp/xvfb99.log", "w")
    app_log = open("/tmp/yggterm99.log", "w")
    xvfb = subprocess.Popen(
        ["Xvfb", ":99", "-screen", "0", "1600x900x24"],
        stdout=xvfb_log,
        stderr=subprocess.STDOUT,
    )
    app = None
    try:
        time.sleep(1)
        env = os.environ.copy()
        env["DISPLAY"] = ":99"
        env["RUST_BACKTRACE"] = "full"
        app = subprocess.Popen(
            ["./target/debug/yggterm"],
            cwd="/home/pi/gh/yggterm",
            env=env,
            stdout=app_log,
            stderr=subprocess.STDOUT,
        )
        time.sleep(8)
        subprocess.run(["import", "-window", "root", out1], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        subprocess.run("xdotool search --name yggterm windowactivate --sync %@",
                       shell=True, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(2)
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
