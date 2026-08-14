#!/usr/bin/env python3
"""`escalate_to` stored at two lengths, and every consumer that compares it.

    python3 tests/test_monitor_pointer_length.py [--monitor <path to ygg-monitor.py>]

⛔ THE DEFECT THIS SCREENS FOR HAD THREE CALLSITES AND WAS FIXED AT ONE. A
subscription stores `escalate_to` verbatim from whatever a brief quoted, and
briefs quote eight characters because that is the width the board prints. The row
plane always answers with thirty-six. Every consumer that compared the two by
equality therefore read a live orchestrator as dead:

  • `escalate()`  — `to not in live` ⇒ fell back to a human card, logging that a
    row sitting right there "is NOT a live row". The lane's cries never arrived.
  • `cmd_succeed` — `== old` ⇒ skipped exactly the row it exists to rescue, then
    reported a clean count, so the handover looked complete.
  • `cmd_list`    — renders `[:8]`, making a stub and a good pointer pixel-
    identical. The instrument the orchestrator is told to believe could not show
    it, which is why six rows across three campaigns carried stubs unnoticed.

`_same_uuid` was written for this class on 2026-08-13 and wired into the function
that REPORTS, not the ones that ROUTE and REPAIR.

⭐ FALSIFIED, not merely passed: against the pre-fix script (`--monitor <old>`)
**every** screen here fails, measured 2026-08-14. The first draft had one that
did not — `normalize --dry-run` "changed nothing" against a script where the verb
did not exist at all, which is a control passing for the wrong reason. It now
requires positive evidence that the sweep ran and saw the row.

Isolation is by `$HOME`, matching test_booter_screens.py: the script derives its
state directory from `Path.home()`, so a temporary home is a complete sandbox
with no test-only override in the product.
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_MONITOR = HERE.parent / "ygg-monitor.py"

# Invented. ⚠ Keep any digit run under twelve — the pre-push privacy guard reads
# a long one as an identity number, and it is right to.
BOSS_OLD = "dddddddd-4444-4444-8444-dddddddddddd"
BOSS_NEW = "eeeeeeee-5555-4555-8555-eeeeeeeeeeee"
LANE = "ffffffff-6666-4666-8666-ffffffffffff"

FAILURES = []


def check(name, ok, detail=""):
    print(f"{'ok  ' if ok else 'FAIL'}  {name}{('  — ' + detail) if detail and not ok else ''}")
    if not ok:
        FAILURES.append(name)


class Sandbox:
    def __init__(self, monitor):
        self.monitor_path = str(monitor)
        self.home = Path(tempfile.mkdtemp(prefix="monitor-pointer-"))
        self.subs = self.home / ".yggterm" / "relay" / "monitor"
        self.subs.mkdir(parents=True)

    def monitor(self, *argv, timeout=120):
        """⛔ ONLY LOCAL-STATE VERBS BELONG HERE. `subscribe`/`list`/`succeed`/
        `normalize` read and write this sandbox's own subscription directory and
        never reach the row plane — which is why none of them takes `--gui-host`
        (only `tick`/`watch` do, and a test that could reach a real row is not a
        test, it is an incident). `$YGG_GUI_HOST` is cleared so nothing probes
        for a live desktop through the environment either."""
        env = dict(os.environ, HOME=str(self.home))
        env.pop("YGGTERM_SESSION_ID", None)
        env.pop("YGG_GUI_HOST", None)
        return subprocess.run([sys.executable, self.monitor_path, *argv],
                              capture_output=True, text=True, timeout=timeout, env=env)

    def write_sub(self, uuid, escalate_to, seat="-", role="relay"):
        p = self.subs / f"{uuid}.json"
        p.write_text(json.dumps({
            "uuid": uuid, "host": "testhost", "role": role,
            "escalate_to": escalate_to, "escalate_host": "testhost",
            "campaign": "test", "seat": seat, "owner_pinned": False,
            "booter": True, "intent": "screen", "since": 0}, indent=1))
        return p

    def read_sub(self, uuid):
        return json.loads((self.subs / f"{uuid}.json").read_text())

    def cleanup(self):
        import shutil
        shutil.rmtree(self.home, ignore_errors=True)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--monitor", default=str(DEFAULT_MONITOR),
                    help="point at the pre-fix script to falsify these screens")
    a = ap.parse_args()
    sb = Sandbox(a.monitor)
    try:
        # --- the source: a short --escalate-to must not be stored short -------
        sb.write_sub(BOSS_OLD, "", seat="9.0", role="orchestrator")
        r = sb.monitor("subscribe", "--uuid", LANE, "--machine", "testhost",
                       "--role", "relay", "--seat", "1.1", "--campaign", "test",
                       "--escalate-to", BOSS_OLD[:8], "--intent", "screen")
        stored = sb.read_sub(LANE)["escalate_to"] if (sb.subs / f"{LANE}.json").exists() else ""
        check("subscribe EXPANDS a short --escalate-to against what is subscribed",
              stored == BOSS_OLD, f"stored={stored!r} rc={r.returncode} {r.stdout[-160:]}")

        # --- the board: a stub must be visible as one ------------------------
        sb.write_sub(LANE, BOSS_OLD[:8], seat="1.1")
        r = sb.monitor("list")
        check("⛔ list MARKS a short pointer instead of rendering it identically",
              "!" in r.stdout, f"rc={r.returncode} {r.stdout[-240:]}")

        # --- the repair: succession must move the row that stored it short ----
        sb.write_sub(BOSS_NEW, "", seat="9.0", role="orchestrator")
        r = sb.monitor("succeed", "--from", BOSS_OLD, "--to", BOSS_NEW)
        moved = sb.read_sub(LANE)["escalate_to"]
        check("⛔ succeed MOVES a subscriber whose pointer is stored SHORT",
              moved == BOSS_NEW,
              f"escalate_to={moved!r} rc={r.returncode} {r.stdout[-240:]}")

        # ...and it must say so, rather than reporting a clean count over the gap.
        check("succeed REPORTS the row it moved",
              "re-pointed" in r.stdout and LANE[:8] in r.stdout,
              f"{r.stdout[-240:]}")

        # --- the sweep: normalize expands what a frozen brief re-introduces ---
        sb.write_sub(LANE, BOSS_NEW[:8], seat="1.1")
        r = sb.monitor("normalize", "--dry-run")
        untouched = sb.read_sub(LANE)["escalate_to"]
        # ⛔ "NOTHING CHANGED" IS ALSO WHAT A VERB THAT DOES NOT EXIST PRODUCES.
        # This screen passed against the pre-fix script for that reason alone —
        # a control that passes for the wrong reason supports nothing. Require
        # positive evidence the sweep RAN and SAW the row before believing its
        # restraint.
        check("normalize --dry-run RUNS, REPORTS the stub, and changes NOTHING",
              untouched == BOSS_NEW[:8] and r.returncode == 0
              and "DRY would expand" in r.stdout and LANE[:8] in r.stdout,
              f"escalate_to={untouched!r} rc={r.returncode} {r.stdout[-200:]}")
        r = sb.monitor("normalize")
        check("normalize EXPANDS a stub to the full uuid it names",
              sb.read_sub(LANE)["escalate_to"] == BOSS_NEW,
              f"escalate_to={sb.read_sub(LANE)['escalate_to']!r} {r.stdout[-240:]}")

        # --- and it must decline to guess -----------------------------------
        # ⛔ A PREFIX THAT NAMES NOBODY MUST BE LEFT ALONE, not resolved to the
        # nearest thing. Silently repointing a lane at the wrong orchestrator is
        # worse than the stub it replaces: the stub fails loudly at escalate time.
        sb.write_sub(LANE, "99999999", seat="1.1")
        r = sb.monitor("normalize")
        check("⛔ normalize LEAVES ALONE a prefix that matches no subscription",
              sb.read_sub(LANE)["escalate_to"] == "99999999" and "LEFT ALONE" in r.stdout,
              f"escalate_to={sb.read_sub(LANE)['escalate_to']!r} {r.stdout[-240:]}")
    finally:
        sb.cleanup()

    print()
    if FAILURES:
        print(f"⛔ {len(FAILURES)} failed: {', '.join(FAILURES)}")
        return 1
    print("all pointer-length screens hold")
    return 0


if __name__ == "__main__":
    sys.exit(main())
