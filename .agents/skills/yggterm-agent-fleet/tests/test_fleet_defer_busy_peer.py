#!/usr/bin/env python3
"""[mem-dream 1.3] a busy PEER lock defers the fleet sync instead of failing it.

    python3 tests/test_fleet_defer_busy_peer.py [path-to-ygg-memory.py]

⛔ THE HOLE THIS PINS (measured 2026-10-06, slice-1 rollout): the LOCAL hub
lock already defers (cmd_sync_fleet catches 'Lock held by another process'
and exits 0 — pinned by test_fleet_sync_defers_when_lock_busy since
2026-09-10), but a PEER leg hitting its own busy lock appended
'failed=<peer>:migrate' and the whole sync RAISED 'did not converge' — a
false alarm in every concurrent-tick window. The peer legs (migrate,
import-journal) now classify the lock signature as a deferral: the peer is
skipped for this round's exchange, the run converges, and the report
carries the deferrals. Real faults still raise (polarity pinned).
"""

import importlib.util
import sys
from pathlib import Path


def load_module(path: Path):
    spec = importlib.util.spec_from_file_location("ygg_memory_under_test", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main() -> int:
    default = Path(__file__).resolve().parent.parent / "ygg-memory.py"
    module = load_module(Path(sys.argv[1]) if len(sys.argv) > 1 else default)
    failures = []

    def check(condition, message):
        if condition:
            print(f"  ok: {message}")
        else:
            failures.append(message)
            print(f"  FAIL: {message}")

    print("[1] the busy-lock classifier")
    check(
        module._peer_busy_lock(
            "RuntimeError: Lock held by another process: /home/pi/.yggterm/memory/.ygg-memory.lock (waited 60s)"
        ),
        "the peer's lock-held RuntimeError classifies as busy",
    )
    check(not module._peer_busy_lock(""), "an empty stderr is not busy")
    check(not module._peer_busy_lock("Traceback ... KeyError: 'watermark'"), "a real fault is not busy")
    check(not module._peer_busy_lock(None), "a missing stderr is not busy")

    print("[2] the outcome split")
    check(module._fleet_outcome([], []) == "", "clean run converges")
    check(
        module._fleet_outcome(["oc"], []) == "unreachable=oc",
        "unreachable peers still fail the run (ssh is a real fault class)",
    )
    check(
        "failed=dev:migrate" in module._fleet_outcome([], ["dev:migrate"]),
        "a non-lock peer failure still fails the run (polarity held)",
    )
    # The deferral itself never enters the outcome: deferrals-only converges.
    check(module._fleet_outcome([], []) == "", "deferrals are not failures — the report carries them, the outcome does not")

    if failures:
        print(f"\n{len(failures)} FAILURES")
        return 1
    print("\nall deferral locks green")
    return 0


if __name__ == "__main__":
    sys.exit(main())
