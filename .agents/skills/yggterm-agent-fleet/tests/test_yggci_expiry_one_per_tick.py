#!/usr/bin/env python3
"""The [11.97] residual: the expiry probe is LANE-shaped — ONE per tick.

    python3 tests/test_yggci_expiry_one_per_tick.py

Measured live 2026-10-03 14:17: two lanes quarantined by the same failed
union both had their TTLs pop in one tick, re-merged into the SAME union,
failed identically, and re-quarantined together — an unchanged innocent lane
could never land. This locks the pure decision the tick loop now reads:
first expired lane probes (and spends the budget), a co-expired sibling
defers WITH its quarantine intact, unexpired holds, a new tip re-arms."""
import importlib.util
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
    mod.LOGPATH = tmp / "ci-test.log"
    mod.CI_STATE = tmp
    mod._STDOUT_IS_LOG = False
    return mod


def main():
    tmp = Path(tempfile.mkdtemp(prefix="yggci-expiry-rr-test-"))
    mod = load_module(tmp)
    now = time.time()
    v = mod._quarantine_expiry_verdict

    q = {"tip": "aaa", "expires": now + 900}
    check("unexpired holds",
          v("lane/a", q, "aaa", now, 1) == ("hold", None))
    check("new tip re-arms regardless of budget",
          v("lane/a", q, "bbb", now, 0) == ("rearm", None))

    exp = {"tip": "aaa", "expires": now - 1}
    check("first expired lane probes",
          v("lane/a", exp, "aaa", now, 1) == ("probe", None))
    check("the spent budget defers the co-expired sibling by name",
          v("lane/b", exp, "aaa", now, 0) == ("defer", "quarantine_expiry_deferred"))
    check("zero budget never admits an expiry probe",
          v("lane/b", exp, "aaa", now, 0)[0] == "defer")

    # The deferred lane must keep its entry — the loop only pops on "rearm"
    # and "probe" — so the NEXT tick (budget refreshed to 1) admits it: the
    # round-robin needs no persisted cursor, just the surviving entry.
    check("defer never pops the quarantine (the entry outlives the tick)",
          v("lane/b", exp, "aaa", now + 301, 1) == ("probe", None))

    print("FAILURES:", FAILURES if FAILURES else "none")
    return 1 if FAILURES else 0


if __name__ == "__main__":
    sys.exit(main())
