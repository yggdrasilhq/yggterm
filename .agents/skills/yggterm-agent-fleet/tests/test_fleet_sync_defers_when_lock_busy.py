#!/usr/bin/env python3
"""sync-fleet defers (exit 0) when the hub lock is busy; real faults still raise.

    python3 tests/test_fleet_sync_defers_when_lock_busy.py [path-to-ygg-memory.py]

⛔ THE HOLE THIS PINS, measured 2026-09-10 ~23:15: three seats, one hub flock.
Another seat's long `sync-harness --all` held the lock past sync-fleet's 60s
patience, and `cmd_sync_fleet` let the RuntimeError escape — exit 1, traceback,
"Lock held by another process" — in the middle of every seat's session-end
ritual. sync-fleet is the RECOVERY CONVERGENCE leg: the catch-up tick re-runs
it, so a busy lock is a deferral, not a failure. The verb now says exactly that
and exits 0; the failure mode that used to read as breakage is gone.

The polarity case is pinned too: a RuntimeError that is NOT the lock (a real
fault) must still propagate — deferral must never swallow genuine breakage.
"""

import contextlib
import importlib.util
import io
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_SCRIPT = HERE.parent / "ygg-memory.py"

FAILURES = []


def check(name, ok, detail=""):
    print(f"{'ok  ' if ok else 'FAIL'}  {name}{('  — ' + detail) if detail and not ok else ''}")
    if not ok:
        FAILURES.append(name)


def load_module(script_path):
    spec = importlib.util.spec_from_file_location("ygg_memory_defer", str(script_path))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class Args:
    def __init__(self, root, **kw):
        self.root = str(root)
        self.mesh = []
        self.quick = True
        self.json = False
        self.quiet = False
        for key, value in kw.items():
            setattr(self, key, value)


def run():
    script = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_SCRIPT
    mod = load_module(script)
    tmp_root = Path(tempfile.mkdtemp(prefix="ygg-mem-defer-test-"))

    # Case 1 — lock genuinely held on another file descriptor: cmd_sync_fleet
    # must RETURN (no raise), print the deferral, and leave exit code 0 to its
    # caller. Real flock contention, not a mock.
    holder = mod._flock_open(tmp_root / ".ygg-memory.lock", timeout_seconds=1)
    try:
        captured = io.StringIO()
        with contextlib.redirect_stdout(captured):
            mod.cmd_sync_fleet(Args(tmp_root))
        out = captured.getvalue()
        check(
            "busy lock defers instead of raising",
            "deferred" in out and "catch-up tick" in out,
            out.strip()[:120],
        )
    except RuntimeError as error:
        check("busy lock defers instead of raising", False, f"raised: {error}")
    finally:
        mod._flock_close(holder)

    # Case 1b — the same run is a JSON deferral under --json (the tick and any
    # machine consumer must be able to tell deferral from success-from-zero).
    holder = mod._flock_open(tmp_root / ".ygg-memory.lock", timeout_seconds=1)
    try:
        captured = io.StringIO()
        with contextlib.redirect_stdout(captured):
            mod.cmd_sync_fleet(Args(tmp_root, json=True))
        out = captured.getvalue().strip()
        check(
            "json deferral names its status",
            '"status": "deferred"' in out,
            out[:120],
        )
    except RuntimeError as error:
        check("json deferral names its status", False, f"raised: {error}")
    finally:
        mod._flock_close(holder)

    # Case 2 — a real fault (not the lock) must NOT be deferred into silence.
    original = mod._run_fleet_sync

    def broken_sync(root, mesh, quick=False):
        raise RuntimeError("ssh mesh exploded for real")

    mod._run_fleet_sync = broken_sync
    try:
        captured = io.StringIO()
        with contextlib.redirect_stdout(captured):
            mod.cmd_sync_fleet(Args(tmp_root))
        check(
            "non-lock RuntimeError still raises",
            False,
            "cmd_sync_fleet swallowed a real fault",
        )
    except RuntimeError as error:
        check(
            "non-lock RuntimeError still raises",
            "ssh mesh exploded" in str(error),
            str(error)[:120],
        )
    finally:
        mod._run_fleet_sync = original

    # Case 3 — a quiet hub is a normal sync, never a deferral message.
    captured = io.StringIO()
    with contextlib.redirect_stdout(captured):
        mod.cmd_sync_fleet(Args(tmp_root))
    out = captured.getvalue()
    check(
        "quiet hub syncs normally (no deferral)",
        "deferred" not in out and "Fleet memory sync:" in out,
        out.strip()[:120],
    )

    # Case 4 — the lock error itself is actionable: names the wait and the
    # remedy, while keeping the matchable prefix. The lock must actually be
    # CONTENDED here: holder on one fd, attempt on another.
    holder = mod._flock_open(tmp_root / ".ygg-memory.lock", timeout_seconds=1)
    try:
        mod._flock_open(tmp_root / ".ygg-memory.lock", timeout_seconds=1)
        check("lock error is raised at all", False, "no error on contended lock")
    except RuntimeError as error:
        text = str(error)
        check(
            "lock error names wait + remedy, keeps prefix",
            text.startswith("Lock held by another process")
            and "waited 1s" in text
            and "catch-up tick" in text,
            text[:160],
        )
    finally:
        mod._flock_close(holder)

    print()
    if FAILURES:
        print(f"{len(FAILURES)} failure(s)")
        return 1
    print("all ok")
    return 0


if __name__ == "__main__":
    sys.exit(run())
