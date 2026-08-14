#!/usr/bin/env python3
"""`retire` must clean the plane the OWNER can see, not only our bookkeeping.

    python3 tests/test_retire_drops_the_sidebar_row.py [--booter <path>]

⛔ THE BUG THIS ENCODES. `retire` dropped the booter subscription and the monitor
subscription and stopped. Both are OUR bookkeeping. The sidebar row — the only
plane the owner looks at — was left standing, because `live_keep_alive` is true
on every agent row, which is right for an ordinary resumable session and wrong
for a seat declared dead with evidence. Owner-reported 2026-08-14: *why are the
6.x predecessors not despawned?* Fifteen were listed, several already reaped as
processes.

⚠ An earlier fix taught this verb about the MONITOR plane and its comment argued
the general case — *a death is a fact about the row, not about one watcher's
bookkeeping* — and then stopped one plane short of the screen. So these checks
are written against the direction that keeps recurring: a plane nobody tested.

No live host is touched. `ygg` and `resolve_gui_host` are replaced in the loaded
module, so the test asserts on the CALLS the verb makes.
"""
import argparse
import importlib.util
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_BOOTER = HERE.parent / "ygg-booter.py"

# Invented. ⚠ Digit runs stay under twelve: the pre-push privacy guard reads a
# long one as an identity number, and the fix is a different fixture rather than
# the override flag.
DEAD = "dddddddd-4444-4444-8444-dddddddddddd"
FAILURES = []


def check(name, ok, detail=""):
    print(f"{'ok  ' if ok else 'FAIL'}  {name}{('  — ' + detail) if detail and not ok else ''}")
    if not ok:
        FAILURES.append(name)


def load(booter):
    spec = importlib.util.spec_from_file_location("bb", str(booter))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class FakeGui:
    """A GUI host that lists rows and forgets the ones it is told to remove."""

    def __init__(self, paths, stubborn=False):
        # `stubborn` models the failure the read-back exists for: the verb
        # answers happily and the row is still there afterwards.
        self.paths = list(paths)
        self.stubborn = stubborn
        self.removed = []

    def __call__(self, host, *args):
        if args[:3] == ("server", "app", "rows"):
            return {"rows": [{"session_id": DEAD, "full_path": p} for p in self.paths]}
        if args[:4] == ("server", "app", "session", "remove"):
            path = args[4]
            self.removed.append(path)
            if not self.stubborn and path in self.paths:
                self.paths.remove(path)
            return {"row_still_listed": False, "verified": True}
        return {}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--booter", default=str(DEFAULT_BOOTER))
    a = ap.parse_args()
    mod = load(a.booter)
    mod.resolve_gui_host = lambda _h: "testhost"

    # 1. The ordinary case: one row, and it goes.
    gui = FakeGui(["remote-cc://testhost/" + DEAD])
    mod.ygg = gui
    mod._drop_row(DEAD)
    check("a retired row is removed from the sidebar",
          gui.removed == ["remote-cc://testhost/" + DEAD] and not gui.paths,
          f"removed={gui.removed} left={gui.paths}")

    # 2. ⛔ One session can render in SEVERAL views (the live rail and the cwd
    #    tree both list it, by design). Removing the first and reporting success
    #    is how a row appears to survive its own retirement.
    both = ["local://" + DEAD, "remote-cc://testhost/" + DEAD]
    gui = FakeGui(both)
    mod.ygg = gui
    mod._drop_row(DEAD)
    check("EVERY path naming the session is removed, not just the first",
          sorted(gui.removed) == sorted(both) and not gui.paths,
          f"removed={gui.removed} left={gui.paths}")

    # 3. ⛔ The verb reports the REQUEST, not the EFFECT. If the row survives,
    #    the retirement must NOT read as clean.
    gui = FakeGui(["remote-cc://testhost/" + DEAD], stubborn=True)
    mod.ygg = gui
    out = []
    mod.log = lambda m: out.append(str(m))
    mod._drop_row(DEAD)
    check("a row that SURVIVES removal is reported, not swallowed",
          any("ROW SURVIVED REMOVAL" in m for m in out),
          f"said={out}")

    # 4. An unreachable GUI must not eat the retirement, and must say so.
    mod.resolve_gui_host = lambda _h: None
    out = []
    mod.log = lambda m: out.append(str(m))
    mod._drop_row(DEAD)
    check("no GUI host leaves a LOUD note rather than a silent pass",
          any("ROW LEFT IN THE SIDEBAR" in m for m in out), f"said={out}")

    print()
    if FAILURES:
        print(f"⛔ {len(FAILURES)} failed: {', '.join(FAILURES)}")
        return 1
    print("the third plane holds")
    return 0


if __name__ == "__main__":
    sys.exit(main())
