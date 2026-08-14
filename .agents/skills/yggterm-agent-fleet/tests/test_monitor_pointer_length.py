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


def seat_repair_screens(monitor_path, sb):
    """Drive `_seat_handover_repair` directly, in a child with the sandbox HOME.

    ⛔ The two instruments it reads are STUBBED rather than mocked away: the point
    of three of these screens is what it does when an instrument REFUSES, and a
    refusal is not an empty answer. `live_rows` returning ok=False is blindness;
    `screen_ledgers` returning None is an unreadable attended list. Acting on
    either would be a repair founded on no evidence."""
    code = r'''
import importlib.util, json, sys
spec = importlib.util.spec_from_file_location("mon", sys.argv[1])
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
HEIR = "cafecafe-8888-4888-8888-cafecafecafe"
OLD  = "beefbeef-9999-4999-8999-beefbeefbeef"
BOSS = "d0d0d0d0-1010-4010-8010-d0d0d0d0d0d0"
def rows(pairs, ok=True):
    # pairs are (seat, uuid) or (seat, uuid, label) — the label is what a retiring
    # row rewrites about ITSELF, and it is the discriminator the repair needs.
    return (lambda host: ([{"outline_prefix": p[0], "full_path": f"remote-cc://dev/{p[1]}",
                            "label": (p[2] if len(p) > 2 else f"{p[0]} a working lane")}
                           for p in pairs], ok))
def sub(u, seat, esc):
    m.sub_path(u).write_text(json.dumps({"uuid": u, "host": "testhost", "role": "relay",
        "escalate_to": esc, "escalate_host": "testhost", "campaign": "test",
        "seat": seat, "owner_pinned": False, "booter": True, "intent": "x", "since": 0}))
out = {}

# 1. healthy tick REMEMBERS the seat, and changes nothing
sub(OLD, "9.2", BOSS)
m.live_rows = rows([("9.2", OLD)]); m.screen_ledgers = lambda: (set(), set())
m._seat_handover_repair("h", False)
out["remembered"] = json.loads(m.SEAT_MEMORY.read_text()).get("9.2", {}).get("escalate_to") == BOSS

# 2. the successor holds the seat, unsubscribed (a stale `succeed` deleted the record)
m.sub_path(OLD).unlink(missing_ok=True)
m.live_rows = rows([("9.2", HEIR)])
r = m._seat_handover_repair("h", False)
rec = json.loads(m.sub_path(HEIR).read_text()) if m.sub_path(HEIR).exists() else {}
out["restored"] = bool(r) and rec.get("escalate_to") == BOSS and rec.get("seat") == "9.2"

# 3. a seat NEVER subscribed is never invented — this is what keeps deliberate
#    stand-downs and the owner's copilot row off the plane
m.live_rows = rows([("7.7", "0bad0bad-2020-4020-8020-0bad0bad0bad")])
m._seat_handover_repair("h", False)
out["never_invents"] = not m.sub_path("0bad0bad-2020-4020-8020-0bad0bad0bad").exists()

# 3b. ⛔⛔ A RETIRED ROW IS STILL A LISTED ROW AND STILL HOLDS ITS SEAT. A seat that
#     has relayed five times has four corpses under it; the first version restored
#     ALL of them and resurrected four rows onto the plane in one tick.
DEAD1 = "deadbeef-ab3a-4a3a-8a3a-deadbeefaaaa"
DEAD2 = "deadbeef-cd3a-4a3a-8a3a-deadbeefbbbb"
m.sub_path(HEIR).unlink(missing_ok=True)
m.live_rows = rows([("9.2", DEAD1, "9.2 RETIRED, succeeded by cafecafe"),
                    ("9.2", DEAD2, "9.2 RETIRED, succeeded by dead0001"),
                    ("9.2", HEIR,  "9.2 vault legacy: the live one")])
m._seat_handover_repair("h", False)
out["skips_retired"] = (not m.sub_path(DEAD1).exists() and not m.sub_path(DEAD2).exists()
                        and m.sub_path(HEIR).exists())

# 3c. ⛔ TWO LIVE CLAIMANTS ON ONE SEAT IS A HUMAN'S CALL, NOT A BROADCAST.
TWIN = "twinbeef-ef4a-4a4a-8a4a-twinbeefcccc"
m.sub_path(HEIR).unlink(missing_ok=True)
m.live_rows = rows([("9.2", HEIR, "9.2 one claimant"), ("9.2", TWIN, "9.2 another claimant")])
m._seat_handover_repair("h", False)
out["one_holder_only"] = not m.sub_path(HEIR).exists() and not m.sub_path(TWIN).exists()

# 3d. ⛔⛔ A SEAT SOMEBODY ALREADY HOLDS NEEDS NO REPAIR. Restoring a SECOND row
#     onto a covered seat is the duplicate-claimant state, reached by counting only
#     the unsubscribed side.
m.sub_path(HEIR).unlink(missing_ok=True)
sub(HEIR, "9.2", BOSS)                       # a live, SUBSCRIBED holder
OTHER = "0the0000-5050-4a50-8a50-0the0000aaaa"
m.live_rows = rows([("9.2", HEIR, "9.2 the live holder"),
                    ("9.2", OTHER, "9.2 an unsubscribed second row")])
m._seat_handover_repair("h", False)
out["skips_covered_seat"] = not m.sub_path(OTHER).exists()

# 3e. ⛔⛔ A STAND-DOWN IS NOT A HANDOVER. The same uuid leaving the plane on
#     purpose must not be put back from a stale memory of it.
m.live_rows = rows([("9.2", HEIR, "9.2 the live holder")])
m._seat_handover_repair("h", False)          # record membership FOR HEIR
m.sub_path(HEIR).unlink()                    # it stands down and unsubscribes
m._seat_handover_repair("h", False)          # same uuid still holds the seat
out["standdown_is_not_handover"] = not m.sub_path(HEIR).exists()

# 4. an ATTENDED row is refused even with a remembered seat
m.sub_path(HEIR).unlink(missing_ok=True)
m.live_rows = rows([("9.2", HEIR)]); m.screen_ledgers = lambda: ({HEIR[:8]}, set())
m._seat_handover_repair("h", False)
out["skips_attended"] = not m.sub_path(HEIR).exists()

# 5. BLIND row plane repairs nothing
m.screen_ledgers = lambda: (set(), set()); m.live_rows = rows([("9.2", HEIR)], ok=False)
m._seat_handover_repair("h", False)
out["blind_is_not_empty"] = not m.sub_path(HEIR).exists()

# 6. UNREADABLE attended ledger repairs nothing
m.live_rows = rows([("9.2", HEIR)]); m.screen_ledgers = lambda: (None, None)
m._seat_handover_repair("h", False)
out["unreadable_is_not_empty"] = not m.sub_path(HEIR).exists()
print(json.dumps(out))
'''
    env = dict(os.environ, HOME=str(sb.home))
    env.pop("YGGTERM_SESSION_ID", None)
    r = subprocess.run([sys.executable, "-c", code, str(monitor_path)],
                       capture_output=True, text=True, timeout=90, env=env)
    try:
        got = json.loads((r.stdout or "").strip().splitlines()[-1])
    except Exception:
        got = {}
    for name, label in (
        ("remembered", "tick REMEMBERS a healthy seat's membership"),
        ("restored", "⛔ tick RESTORES membership to the seat's next holder"),
        ("never_invents", "⛔ tick NEVER invents membership for a seat that had none"),
        ("skips_retired", "⛔⛔ tick NEVER restores to a row whose title says RETIRED"),
        ("skips_covered_seat", "⛔⛔ tick SKIPS a seat a subscribed row already holds"),
        ("standdown_is_not_handover", "⛔⛔ tick treats the SAME uuid as a STAND-DOWN, not a handover"),
        ("one_holder_only", "⛔ tick restores to NO ONE when a seat has two claimants"),
        ("skips_attended", "⛔ tick REFUSES an attended row even with a remembered seat"),
        ("blind_is_not_empty", "⛔ a BLIND row plane repairs nothing"),
        ("unreadable_is_not_empty", "⛔ an UNREADABLE attended list repairs nothing"),
    ):
        check(label, got.get(name) is True,
              f"got={got.get(name)!r} rc={r.returncode} {(r.stderr or r.stdout)[-200:]}")


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

        # --- succession must hand the PLANE over, not just the seat ----------
        # ⛔ Deleting the predecessor's own subscription without adding the
        # successor's mints a fresh orphan on EVERY relay: a live seat, armed on
        # the booter, escalating to nobody. Two campaigns reported it the same
        # hour; one relays hourly and regenerated it every time.
        sb.write_sub(BOSS_OLD, "", seat="9.2", role="relay")
        p = sb.subs / f"{BOSS_OLD}.json"
        import json as _j
        rec = _j.loads(p.read_text()); rec["escalate_to"] = BOSS_NEW; p.write_text(_j.dumps(rec))
        HEIR = "aaaabbbb-7777-4777-8777-aaaabbbbcccc"
        r = sb.monitor("succeed", "--from", BOSS_OLD, "--to", HEIR)
        got = sb.read_sub(HEIR) if (sb.subs / f"{HEIR}.json").exists() else {}
        check("⛔ succeed HANDS THE PLANE to the successor, not only the seat",
              got.get("role") == "relay" and got.get("seat") == "9.2"
              and got.get("escalate_to") == BOSS_NEW
              and not (sb.subs / f"{BOSS_OLD}.json").exists(),
              f"heir={got!r} rc={r.returncode} {r.stdout[-240:]}")

        # ...and it must never clobber a successor that already knows its own job.
        sb.write_sub(BOSS_OLD, BOSS_NEW, seat="9.2", role="relay")
        sb.write_sub(HEIR, BOSS_NEW, seat="9.9", role="orchestrator")
        r = sb.monitor("succeed", "--from", BOSS_OLD, "--to", HEIR)
        check("⛔ succeed does NOT clobber a successor that already subscribed",
              sb.read_sub(HEIR)["seat"] == "9.9" and "left alone" in r.stdout,
              f"heir={sb.read_sub(HEIR)!r} {r.stdout[-240:]}")

        # --- and it must decline to guess -----------------------------------
        # ⛔ A PREFIX THAT NAMES NOBODY MUST BE LEFT ALONE, not resolved to the
        # nearest thing. Silently repointing a lane at the wrong orchestrator is
        # worse than the stub it replaces: the stub fails loudly at escalate time.
        sb.write_sub(LANE, "99999999", seat="1.1")
        r = sb.monitor("normalize")
        check("⛔ normalize LEAVES ALONE a prefix that matches no subscription",
              sb.read_sub(LANE)["escalate_to"] == "99999999" and "LEFT ALONE" in r.stdout,
              f"escalate_to={sb.read_sub(LANE)['escalate_to']!r} {r.stdout[-240:]}")
        # --- the tick's seat-membership repair -------------------------------
        # ⛔ A SILENT SWEEP ON A HEALTHY BOARD PROVES NOTHING. Driven in-process
        # with the row plane and the attended ledger stubbed, because the repair's
        # whole job is to act on evidence those two supply — and a screen that
        # can only observe the quiet path is the control that always passes.
        seat_repair_screens(a.monitor, sb)
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
