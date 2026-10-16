#!/usr/bin/env python3
"""[11.241] the spawn guard: N concurrent ensure_watcher callers spawn
EXACTLY ONE watcher — the losers re-check inside the lock and adopt.

    python3 tests/test_yggci_spawn_guard.py
"""
import importlib.util
import sys
import tempfile
import threading
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
    mod.LOGPATH = tmp / "ci-test.log"
    mod.CI_STATE = tmp
    mod._STDOUT_IS_LOG = False
    return mod


def main():
    tmp = Path(tempfile.mkdtemp(prefix="yggci-11241-spawn-guard-"))
    mod = load_module(tmp)

    # no watcher alive anywhere: every caller wants to spawn
    mod.watcher_alive = lambda: False
    mod.any_watcher_process_live = lambda: None
    spawned = []

    def fake_popen(cmd, **kw):
        spawned.append(cmd)
        # the spawned child writes its pidfile a moment later
        mod.watcher_alive = lambda: True
        return type("P", (), {"pid": 1234})()

    mod.subprocess = type("S", (), {})()
    mod.subprocess.STDOUT = -2
    mod.subprocess.DEVNULL = -3
    mod.subprocess.Popen = fake_popen

    results = []
    lock = threading.Lock()

    def caller():
        r = mod.ensure_watcher(interval=300)
        with lock:
            results.append(r)

    threads = [threading.Thread(target=caller) for _ in range(8)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()

    check("eight concurrent callers produced exactly ONE spawn",
          len(spawned) == 1, f"spawns={len(spawned)}")
    check("every caller reports success (spawned or adopted)",
          all(r is True for r in results), f"results={results}")
    check("the spawn lock lives in CI_STATE", (tmp / "spawn.lock").exists())

    # ── the adopt path still works under the lock: stale pidfile + live rogue ──
    mod.watcher_alive = lambda: False
    mod.any_watcher_process_live = lambda: 4242
    adopted = mod.ensure_watcher(interval=300)
    check("a rogue is adopted inside the lock, not spawned again",
          adopted is True and len(spawned) == 1)
    check("adoption rewrites the pidfile with the rogue's pid",
          mod.PIDFILE.read_text().strip() == "4242")

    # ── an alive watcher short-circuits without touching the spawn path ──
    mod.watcher_alive = lambda: True
    before = len(spawned)
    check("an alive watcher returns True with no spawn",
          mod.ensure_watcher(interval=300) is True and len(spawned) == before)

    print()
    if FAILURES:
        print(f"{len(FAILURES)} FAILED")
        return 1
    print("All [11.241] spawn-guard tests passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
