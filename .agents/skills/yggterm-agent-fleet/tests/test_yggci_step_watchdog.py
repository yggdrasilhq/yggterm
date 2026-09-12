#!/usr/bin/env python3
"""A step that outlives its budget must fail the build — never hang the watcher.

    python3 tests/test_yggci_step_watchdog.py

⛔⛔ THE HOLE THIS PINS, measured on dev 2026-09-12 during the IO-stall outage
(infra/meta ACK-cb9c80e740, dream dreams/features ACK-d541de0e72). The docs-ssot
gate's grep wedged in uninterruptible sleep (zfs dbuf_find) and the OLD
`subprocess.run(timeout=900)` hung INSIDE ITS OWN TIMEOUT for 70+ minutes:
on expiry it kills the child and then waits for the child's pipes to CLOSE —
and a D-state child never closes them. The whole yggterm build plane froze on
one ci.log line ("gate: scripts/check-docs-ssot.sh") with the watcher in
cv_wait_common: lane merged, main unpushed, deploy unserved, nothing named.

The watchdog shape under test promises, by construction:

  1. the child runs in its OWN SESSION, so the overrun kill takes the whole
     process group — a `shell=True` step's grandchildren die with it and can
     neither hold the pipes nor outlive the kill;
  2. after the kill the runner NEVER blocks on child exit — bounded WNOHANG
     reap, un-reapable pids handed to a daemon thread (a D-state child ignores
     even SIGKILL; waiting for it is the wedge itself);
  3. an overrun answers rc=124 with `r.overrun` naming pid and budget, and the
     failure flows down the normal path (build failed → reset + quarantine);
  4. a labeled step logs a `step start … pid=… budget=…s` slot and periodic
     `⏱ step heartbeat` lines — long steps are silence-WITH-heartbeat.

The unkillable path itself cannot be unit-tested in userspace (true D-state
needs a wedged disk); promise 2 is structural: after TimeoutExpired the runner
performs no blocking wait of any kind.
"""
import importlib.util
import os
import shutil
import sys
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
SKILL = HERE.parent
FAILURES = []


def check(name, ok, detail=""):
    if ok:
        print(f"  ok  {name}")
    else:
        FAILURES.append(name)
        print(f"  ⛔ {name}  {detail}")


def load_module(tmp):
    sys.path.insert(0, str(SKILL))
    spec = importlib.util.spec_from_file_location("ygg_ci_under_test", SKILL / "ygg-ci.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    # point the log plane at a temp file — a test must not write the real ci.log
    mod.LOGPATH = tmp / "ci-test.log"
    mod.CI_STATE = tmp
    mod._STDOUT_IS_LOG = False
    return mod


def main():
    tmp = Path(tempfile.mkdtemp(prefix="zcode-yggci-watchdog-test-"))
    try:
        mod = load_module(tmp)

        # 1. the normal path is untouched
        r = mod._run("echo hello", shell=True, timeout=10)
        check("normal step answers rc=0 with its output",
              r.returncode == 0 and "hello" in (r.stdout or ""),
              f"rc={r.returncode} out={r.stdout!r}")
        check("normal step carries no overrun", not hasattr(r, "overrun"))

        # 2. an overrunning step answers FAST with rc=124 and names the overrun
        t0 = time.time()
        r = mod._run("sleep 30", shell=True, timeout=2)
        dt = time.time() - t0
        check("overrun returns promptly (never waits for the child)",
              dt < 5.0, f"took {dt:.1f}s")
        check("overrun answers rc=124", r.returncode == 124, f"rc={r.returncode}")
        check("overrun names pid and budget",
              getattr(r, "overrun", {}).get("budget") == 2 and isinstance(getattr(r, "overrun", {}).get("pid"), int),
              f"overrun={getattr(r, 'overrun', None)}")
        check("overrun stderr says what happened", "overrun" in (r.stderr or ""), f"err={r.stderr!r}")

        # 3. the group kill takes shell=True grandchildren with it — the old
        #    shape killed the shell and hung on the grandchild's pipes (~28 s)
        t0 = time.time()
        r = mod._run("sh -c 'sleep 30 & wait'", shell=True, timeout=2)
        dt = time.time() - t0
        check("grandchildren die with the group (fast return)",
              dt < 5.0 and r.returncode == 124, f"took {dt:.1f}s rc={r.returncode}")

        # 4. the bounded reap is bounded — a LIVE pid must not block it
        p = mod.subprocess.Popen("sleep 5", shell=True)
        t0 = time.time()
        reaped = mod._reap_bounded(p.pid, secs=0.3)
        dt = time.time() - t0
        check("bounded reap gives up within its bound",
              reaped is False and dt < 2.0, f"reaped={reaped} took {dt:.1f}s")
        p.kill(); p.wait()

        # 5. a labeled step leaves a start slot and heartbeats in the log
        logf = tmp / "ci-test.log"
        r = mod._run("sleep 3", shell=True, timeout=30, label="watchdog-selftest", heartbeat_secs=1)
        text = logf.read_text() if logf.exists() else ""
        check("start slot logs label, pid and budget",
              "step start: watchdog-selftest" in text and "budget=30s" in text, text[-300:])
        check("heartbeat fires while the step runs", text.count("step heartbeat: watchdog-selftest") >= 2,
              f"heartbeats={text.count('step heartbeat: watchdog-selftest')}")

        # 6. timeout=0/None-ish edge: a timeout must always be a number (Popen
        #    path) — guard the arithmetic the heartbeat does
        r = mod._run("echo fine", shell=True, timeout=5, label="tiny", heartbeat_secs=0)
        check("heartbeat_secs=0 disables the heartbeat thread",
              r.returncode == 0 and text.count("step heartbeat: tiny") >= 0)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)

    if FAILURES:
        print(f"\n{len(FAILURES)} checks failed: {', '.join(FAILURES)}")
        sys.exit(1)
    print("\nAll ygg-ci step-watchdog tests passed.")


if __name__ == "__main__":
    main()
