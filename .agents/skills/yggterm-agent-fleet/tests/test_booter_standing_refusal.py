#!/usr/bin/env python3
"""A refusal that repeats is a CONDITION, and a condition must be visible.

    python3 tests/test_booter_standing_refusal.py [--booter <path to ygg-booter.py>]

Two defects, one arm of the tick loop, both falsified here — run with `--booter`
pointing at the pre-2026-08-21 script and both fail.

⛔ **The crash.** The label for a refusal was an exhaustive `dict[via]` over a
vocabulary defined two thousand lines away in `boot()`. A fifth refusal was added
to the membership test without being added to the labels, so the first row to be
refused for that reason would raise `KeyError` out of the per-row loop, out of
`tick()`, past the `finally` that removes the pidfile — and kill the watcher for
the whole host. It had never fired, and it would have fired on the ordinary event
of a session running out of quota.

⛔ **The silence.** A refusal is REFUNDED, and correctly: the row was never asked.
But then `boots` never rises, so the row can never reach `MAX_BOOTS`, so it can
never escalate — a row refused every tick forever is indistinguishable in state
from one that never needed a boot. Four rows across four campaigns sat that way
for days.

⚖ Neither fix relaxes a refusal. The guards still refuse, the refund still stands;
what changes is that the standing condition is counted, shown, and told once.

Isolation is by `$HOME`, exactly as the sibling screen suite: no test-only
environment override into a safety path.
"""
import argparse
import importlib.util
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_BOOTER = HERE.parent / "ygg-booter.py"

# Invented. ⚠ Digit runs kept under twelve — the pre-push privacy guard reads a
# long one as an identity number, and the fix for that is a different fixture.
WEDGED = "dddddddd-4444-4444-8444-dddddddddddd"
ROW = f"remote-cc://testhost/{WEDGED}"

FAILURES = []


def check(name, ok, detail=""):
    print(f"{'ok  ' if ok else 'FAIL'}  {name}{('  — ' + detail) if detail and not ok else ''}")
    if not ok:
        FAILURES.append(name)


DRIVER = r'''
import importlib.util, json, sys, time, types
spec = importlib.util.spec_from_file_location("bb", sys.argv[1])
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)

VIA, TICKS, UUID, ROW = sys.argv[2], int(sys.argv[3]), sys.argv[4], sys.argv[5]

# ⛔ NOTHING BELOW MAY REACH A REAL ROW. Every door out of the loop is stubbed,
#    and `boot` is replaced outright — a test that could type into a row is not
#    a test, it is an incident.
escalations = []
m.boot = lambda host, row, dry=False: VIA
m.escalate = lambda host, row, why: escalations.append(why)
m.resolve = lambda host, row: None
m.row_presence = lambda host, uuid: True
m.disarm_state = lambda: None
m.rate_limit_hold = lambda: None
m.note_rate_limit = lambda uuid, tail: None
m.BB.row_host = lambda row, host: None
m.BB.progress_marks = lambda path: 0
# IDLE, and older than any boot window, so the loop reaches the boot arm.
m.BB.classify = lambda uuid, rhost=None: {
    "state": "IDLE", "age": 99999, "path": None, "tail": "", "seat": None,
}

args = types.SimpleNamespace(dry_run=False, host="testhost", interval=300)

rec = {
    "uuid": UUID, "row": ROW, "host": "testhost", "campaign": "test",
    "subscribed_at": int(time.time()), "max_hours": 999, "kind": "task", "boots": 0,
    "last_size": 0, "escalated": False, "standing_refusal": None,
}
m.sub_path(UUID).parent.mkdir(parents=True, exist_ok=True)
m.sub_path(UUID).write_text(json.dumps(rec))

crash = None
for _ in range(TICKS):
    try:
        m.tick(args)
    except BaseException as exc:            # the KeyError is what we are hunting
        crash = f"{type(exc).__name__}: {exc}"
        break

after = json.loads(m.sub_path(UUID).read_text()) if m.sub_path(UUID).exists() else {}
print("RESULT" + json.dumps({
    "crash": crash,
    "standing": after.get("standing_refusal"),
    "boots": after.get("boots"),
    "escalations": escalations,
}))
'''


def drive(booter, home, via, ticks):
    env = dict(os.environ, HOME=str(home))
    env.pop("YGGTERM_SESSION_ID", None)
    env.pop("YGG_GUI_HOST", None)
    r = subprocess.run(
        [sys.executable, "-c", DRIVER, str(booter), via, str(ticks), WEDGED, ROW],
        capture_output=True, text=True, timeout=180, env=env)
    for line in (r.stdout or "").splitlines():
        if line.startswith("RESULT"):
            return json.loads(line[len("RESULT"):])
    return {"crash": f"driver produced nothing: {(r.stderr or '')[-400:]}",
            "standing": None, "boots": None, "escalations": []}


def fresh_home():
    home = Path(tempfile.mkdtemp(prefix="booter-standing-"))
    (home / ".yggterm" / "relay" / "booter").mkdir(parents=True)
    return home


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--booter", default=str(DEFAULT_BOOTER))
    args = ap.parse_args()
    booter = Path(args.booter).resolve()
    source = booter.read_text()

    # ── 1. THE CRASH ─────────────────────────────────────────────────────────
    # The label lookup must survive every refusal `boot()` can actually return.
    # Harvested from the product source rather than listed here, because a list
    # written in a test is exactly the second encoding that drifted.
    vocabulary = sorted(set(re.findall(r'return "(refused-[a-z-]+)"', source)))
    check("the refusal vocabulary is non-empty (the harvest still works)",
          len(vocabulary) >= 4, f"found {vocabulary}")
    for via in vocabulary:
        home = fresh_home()
        try:
            out = drive(booter, home, via, 1)
            check(f"⛔ one tick refused {via} does not kill the watcher",
                  out["crash"] is None, out["crash"] or "")
        finally:
            shutil.rmtree(home, ignore_errors=True)

    # ── 2. THE SILENCE ───────────────────────────────────────────────────────
    # A run of identical refusals must be countable in the subscription itself.
    home = fresh_home()
    try:
        out = drive(booter, home, "refused-draft-race", 3)
        standing = out["standing"] or {}
        check("⛔ a repeated refusal is COUNTED on the subscription",
              standing.get("reason") == "refused-draft-race" and standing.get("ticks") == 3,
              f"standing={standing}")
        check("⛔ the refund still stands — a refused row was never asked",
              out["boots"] == 0, f"boots={out['boots']}")
        check("a condition below the threshold does NOT page anyone",
              not out["escalations"], f"{out['escalations']}")
    finally:
        shutil.rmtree(home, ignore_errors=True)

    # ── 3. EXACTLY ONE ESCALATION, AND THEN QUIET ────────────────────────────
    home = fresh_home()
    try:
        mod = importlib.util.module_from_spec(
            importlib.util.spec_from_file_location("bb_ro", booter))
        mod.__spec__.loader.exec_module(mod)
        threshold = getattr(mod, "STANDING_REFUSAL_TICKS", None)
        check("the row-observation threshold is declared as a constant",
              isinstance(threshold, int) and threshold > 0, f"{threshold!r}")
        ticks = (threshold or 12) + 4
        out = drive(booter, home, "refused-draft-race", ticks)
        check("⛔⛔ a standing refusal reaches a human — EXACTLY ONCE",
              len(out["escalations"]) == 1,
              f"{len(out['escalations'])} escalations: {out['escalations'][:2]}")
        if out["escalations"]:
            why = out["escalations"][0]
            check("the escalation names the reason and says the row is not being woken",
                  "refused-draft-race" in why and "NOT being woken" in why, why[:200])
        check("and the subscription still records the standing condition",
              (out["standing"] or {}).get("escalated") is True, f"{out['standing']}")
    finally:
        shutil.rmtree(home, ignore_errors=True)

    # ── 4. THE EXEMPTION IS DELIBERATE, NOT AN OVERSIGHT ─────────────────────
    # A limit wait is self-resolving and a human cannot grant quota, so it is
    # counted and shown but never paged. That must be a stated `None`, not a
    # missing key — a missing key is how the crash above happened.
    home = fresh_home()
    try:
        out = drive(booter, home, "refused-limit-wait", (threshold or 12) + 4)
        check("⛔ a limit wait is COUNTED, so the wait is never invisible",
              (out["standing"] or {}).get("ticks", 0) >= (threshold or 12),
              f"{out['standing']}")
        check("⛔ but it never pages a human — nobody can grant quota",
              not out["escalations"], f"{out['escalations']}")
    finally:
        shutil.rmtree(home, ignore_errors=True)

    # ── 5. EVERY REFUSAL HAS A STATED POLICY ─────────────────────────────────
    table = getattr(mod, "STANDING_REFUSAL_ESCALATE_AFTER", {})
    missing = [via for via in vocabulary if via not in table]
    check("⛔ every refusal `boot()` can return has an explicit escalation policy",
          not missing, f"unstated: {missing}")

    print()
    if FAILURES:
        print(f"⛔ {len(FAILURES)} failed: " + ", ".join(FAILURES))
        return 1
    print(f"✅ all {len(vocabulary) + 10} checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
