#!/usr/bin/env python3
"""A fix on disk is not a fix in the process, and a hold outlives its own reason.

    python3 tests/test_booter_source_and_hold.py [--booter <path to ygg-booter.py>]

Two defects measured 2026-08-21 within one hour of each other, both of the shape
this file keeps paying for: **the decision was corrected and the artefact carrying
the old decision stayed in force.**

⛔ **The loop nobody restarts.** The watcher came up at 17:52 from a checkout that
was current, and the balance/window split landed at 18:12. At 19:45 the fleet was
still fully blacked out — 23 subscribers unwakeable behind one row's exhausted
credit balance — because the running process held code from before the fix, with
`source:` printed in its own log and the startup staleness check reporting nothing
wrong. That check answers *"was this copy current when I started"*, which is the
one moment it is least likely to be false. Restarting by hand fixed the fleet on
the first tick, which is the whole argument.

⛔ **The hold that outlived its evidence.** On that restart the watcher recognised
the refusal as an exhausted BALANCE, suspended that one row exactly as designed —
and eighteen other rows stayed held for another sixteen minutes behind a record
the previous code had armed from the very same tail. The hold was not merely
stale: it was unsupported by its own recorded evidence, which this code no longer
reads as fleet-wide at all.

⚖ Neither fix relaxes a hold that is doing its job. A TIMED window still holds the
fleet, and a hold a human DECLARED is untouched by any reclassification of the
automatic path.

Isolation is by `$HOME` and by a scratch copy of the script, exactly as the
sibling suites: no test-only environment override into a safety path.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_BOOTER = HERE.parent / "ygg-booter.py"

# Invented. ⚠ Digit runs kept under twelve — the pre-push privacy guard reads a
# long one as an identity number.
SPENT = "cccccccc-4444-4444-8444-cccccccccccc"

# ⛔ The two tails are the VENDOR'S OWN WORDINGS, not paraphrases, because the
#    classifier matches on wording and nothing else. A fixture that invents its
#    own phrasing tests the fixture.
BALANCE_TAIL = ("You're out of usage credits. Run /usage-credits to keep using "
                "the model, or /model to switch models.")
WINDOW_TAIL = "5-hour limit reached. Your limit resets at 9pm."

FAILURES = []


def check(name, ok, detail=""):
    print(f"{'ok  ' if ok else 'FAIL'}  {name}{('  — ' + detail) if detail and not ok else ''}")
    if not ok:
        FAILURES.append(name)


# ── driver 1: what does `rate_limit_hold()` do with a record already on disk ──
HOLD_DRIVER = r'''
import importlib.util, json, sys, time
spec = importlib.util.spec_from_file_location("bb", sys.argv[1])
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)

rec = json.loads(sys.argv[2])
# `until` arrives as SECONDS FROM NOW so the fixture cannot go stale on the shelf.
if rec.get("until") is not None:
    rec["until"] = time.time() + rec.pop("until")
if rec.get("declared_until") is not None:
    rec["declared_until"] = time.time() + rec["declared_until"]
m.RLHOLDFILE.parent.mkdir(parents=True, exist_ok=True)
m.RLHOLDFILE.write_text(json.dumps(rec))

held = m.rate_limit_hold()
print("RESULT" + json.dumps({
    "held": held is not None,
    "file_remains": m.RLHOLDFILE.exists(),
}))
'''


# ── driver 2: does the loop carry a change to its own source into the process ──
EXEC_DRIVER = r'''
import importlib.util, json, sys, types
spec = importlib.util.spec_from_file_location("bb", sys.argv[1])
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)

MUTATION, EXPECT_COMPILES = sys.argv[2], sys.argv[3] == "yes"
src = m.Path(sys.argv[1])

baseline = m._source_digest()
unchanged_execs = []
# ⛔ A NAMESPACE, NOT A CLASS. Given a class, `m.os.execv(...)` binds and the
#    recorded argv arrives shifted by one `self` — which reads as the fix
#    passing the wrong arguments rather than as the fixture being wrong.
m.os = types.SimpleNamespace(execv=lambda *a: unchanged_execs.append(a),
                             getpid=m.os.getpid)
# ⭐ An unchanged file must NOT re-exec. Without this half the fix would be a
#    watcher that restarts itself every five minutes forever.
m._REEXEC_ARGS["host"] = "testhost"
m._REEXEC_ARGS["interval"] = 300
m._reexec_if_source_changed(baseline)
execs_when_unchanged = len(unchanged_execs)

# Now move the file under the running process, exactly as a `git pull` does.
src.write_text(src.read_text() + MUTATION)
m._reexec_if_source_changed(baseline)

print("RESULT" + json.dumps({
    "execs_when_unchanged": execs_when_unchanged,
    "execs_after_change": len(unchanged_execs) - execs_when_unchanged,
    "argv": [str(x) for x in (unchanged_execs[-1][1] if unchanged_execs else [])],
    "digest_moved": m._source_digest() != baseline,
}))
'''


def run(driver, booter, home, *argv):
    env = dict(os.environ, HOME=str(home))
    env.pop("YGGTERM_SESSION_ID", None)
    env.pop("YGG_GUI_HOST", None)
    r = subprocess.run(
        [sys.executable, "-c", driver, str(booter), *[str(a) for a in argv]],
        capture_output=True, text=True, timeout=120, env=env)
    for line in (r.stdout or "").splitlines():
        if line.startswith("RESULT"):
            return json.loads(line[len("RESULT"):])
    return {"error": (r.stderr or r.stdout or "")[-500:]}


def fresh_home():
    home = Path(tempfile.mkdtemp(prefix="booter-source-hold-"))
    (home / ".yggterm" / "relay" / "booter").mkdir(parents=True)
    return home


def scratch_checkout(booter, home):
    """A throwaway copy of the whole skill directory, and the script inside it.

    ⛔ The driver APPENDS to the file to simulate a pull landing under a running
    loop, so it must never be pointed at this repo's own script. It copies the
    directory rather than the one file because the booter imports its siblings
    off its own parent path — a lone copy fails to import and every check below
    it reads as a broken fix rather than a broken fixture.
    """
    dst = home / "skill"
    shutil.copytree(booter.parent, dst,
                    ignore=shutil.ignore_patterns("__pycache__", "tests"))
    return dst / booter.name


def hold_record(**over):
    """The live record's own shape. `counted` is present because a record
    lacking it is discarded by an older clause for a different reason, and a
    fixture that trips that clause would pass this suite while proving nothing.
    """
    rec = {
        "since": 0, "last_seen": 0, "until": 1800,
        "counted": {SPENT: [1, 2]},
        "declared_until": None, "declared_reason": None, "declared_by": None,
        "stale_sighting": True, "reset_at": None, "released_by": "timer",
        "seen_on": SPENT, "tail": BALANCE_TAIL,
    }
    rec.update(over)
    return rec


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--booter", default=str(DEFAULT_BOOTER))
    args = ap.parse_args()
    booter = Path(args.booter).resolve()

    # ── 1. A FLEET HOLD WHOSE OWN TAIL IS A BALANCE IS RELEASED ──────────────
    home = fresh_home()
    got = run(HOLD_DRIVER, booter, home, json.dumps(hold_record()))
    check("a fleet hold armed on an exhausted BALANCE is released, not waited out",
          got.get("held") is False and got.get("file_remains") is False,
          json.dumps(got))
    shutil.rmtree(home, ignore_errors=True)

    # ── 2. A TIMED WINDOW STILL HOLDS THE FLEET ──────────────────────────────
    # ⛔ The load-bearing negative. A release that fires on every quota refusal
    #    would walk 20 rows into the same wall one at a time, which is the
    #    behaviour the fleet hold exists to prevent.
    home = fresh_home()
    got = run(HOLD_DRIVER, booter, home, json.dumps(hold_record(tail=WINDOW_TAIL)))
    check("a hold armed on a TIMED WINDOW is still honoured",
          got.get("held") is True and got.get("file_remains") is True,
          json.dumps(got))
    shutil.rmtree(home, ignore_errors=True)

    # ── 3. A DECLARED HOLD OUTRANKS THE RECLASSIFICATION ─────────────────────
    home = fresh_home()
    got = run(HOLD_DRIVER, booter, home,
              json.dumps(hold_record(declared_until=7200,
                                     declared_reason="owner said sit it out")))
    check("a hold a HUMAN declared is not released by a balance tail",
          got.get("held") is True and got.get("file_remains") is True,
          json.dumps(got))
    shutil.rmtree(home, ignore_errors=True)

    # ── 4. AN INDEFINITE HOLD IS NEVER RELEASED ──────────────────────────────
    home = fresh_home()
    got = run(HOLD_DRIVER, booter, home,
              json.dumps(hold_record(indefinite=True, until=None)))
    check("an INDEFINITE hold survives a balance tail",
          got.get("held") is True and got.get("file_remains") is True,
          json.dumps(got))
    shutil.rmtree(home, ignore_errors=True)

    # ── 5. THE LOOP CARRIES A CHANGE TO ITS OWN SOURCE INTO THE PROCESS ──────
    # Driven against a SCRATCH COPY: the driver appends to the file to simulate
    # the pull, and it must never be this repo's script that gets appended to.
    home = fresh_home()
    scratch = scratch_checkout(booter, home)
    got = run(EXEC_DRIVER, scratch, home, "\n# a fix lands under the loop\n", "yes")
    check("an UNCHANGED source does not re-exec (or the watcher restarts forever)",
          got.get("execs_when_unchanged") == 0, json.dumps(got))
    check("a source that CHANGED under the running loop is re-exec-ed into",
          got.get("execs_after_change") == 1, json.dumps(got))
    check("the re-exec comes back up as a watcher with the same host and interval",
          got.get("argv") and got["argv"][2:] == ["watch", "--host", "testhost",
                                                  "--interval", "300"],
          json.dumps(got.get("argv")))
    shutil.rmtree(home, ignore_errors=True)

    # ── 6. A HALF-WRITTEN SOURCE IS NOT EXEC-ED INTO ─────────────────────────
    # ⛔ Fourteen checkouts of this repo share one host and a pull is not the only
    #    way this file changes. Exec-ing into a half-written copy would take the
    #    fleet's watchdog down with a SyntaxError nobody is watching for.
    home = fresh_home()
    scratch = scratch_checkout(booter, home)
    got = run(EXEC_DRIVER, scratch, home, "\ndef (: this is half a file\n", "no")
    check("a source that does not COMPILE is left alone, not exec-ed into",
          got.get("execs_after_change") == 0 and got.get("digest_moved") is True,
          json.dumps(got))
    shutil.rmtree(home, ignore_errors=True)

    print()
    if FAILURES:
        print(f"⛔ {len(FAILURES)} failed: {', '.join(FAILURES)}")
        return 1
    print("all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
