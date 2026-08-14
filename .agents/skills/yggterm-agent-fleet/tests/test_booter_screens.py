#!/usr/bin/env python3
"""The booter's two screening ledgers, and what they do when they cannot be read.

    python3 tests/test_booter_screens.py [--booter <path to ygg-booter.py>]

⛔ EVERY TEST HERE IS ABOUT THE UNSAFE DIRECTION, which is the only direction
that matters for these files: the never-arm list is what stops a watchdog typing
into a row a person is using, so a screen that cannot be read must stop the
watchdog rather than wave it through. Run it against the pre-fix script with
`--booter` and it fails on exactly those cases, which is how it was falsified.

Isolation is by `$HOME`: the script computes its state directory from
`Path.home()` at import, so a temporary home is a complete sandbox with no
product change and, deliberately, no test-only environment override — a bypass
for a safety path is the thing the safety path is guarding.
"""
import argparse
import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
DEFAULT_BOOTER = HERE.parent / "ygg-booter.py"

# Invented, and deliberately not the shape of any live row. ⚠ Keep a digit
# run under twelve: the pre-push privacy guard reads a long one as an
# identity number, and it is right to -- the fix is a different fixture,
# never the override flag.
ATTENDED = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa"
DELEGATE = "bbbbbbbb-2222-4222-8222-bbbbbbbbbbbb"
FINISHED = "cccccccc-3333-4333-8333-cccccccccccc"

FAILURES = []


def check(name, ok, detail=""):
    print(f"{'ok  ' if ok else 'FAIL'}  {name}{('  — ' + detail) if detail and not ok else ''}")
    if not ok:
        FAILURES.append(name)


def rowaddr(uuid):
    """⛔ A ROW ADDRESS IS `<scheme>://<machine>/<uuid>`, NOT A BARE UUID.

    These screens used to pass bare uuids to `--row`, which is exactly the
    malformed form that made a live subscription resolve absent every tick and
    lapse as "GONE (retired)". **The suite encoded the bad address as normal, so
    no screen here could ever have caught it** — the same shape as a suite that
    shares the code's wrong model and therefore passes before and after the bug.
    """
    return f"remote-cc://testhost/{uuid}"


class Sandbox:
    def __init__(self, booter):
        self.booter = str(booter)
        self.home = Path(tempfile.mkdtemp(prefix="booter-screens-"))
        self.state = self.home / ".yggterm" / "relay"
        (self.state / "booter").mkdir(parents=True)
        self.neverarm = self.state / "never-arm.tsv"

    def run(self, *argv, timeout=60):
        env = dict(os.environ, HOME=str(self.home))
        env.pop("YGGTERM_SESSION_ID", None)
        env.pop("YGG_GUI_HOST", None)
        # --host is explicit everywhere so nothing probes for a live desktop.
        return subprocess.run([sys.executable, self.booter, *argv, "--host", "testhost"],
                              capture_output=True, text=True, timeout=timeout, env=env)

    def reader(self, fn):
        """Call one of the module's ledger readers in a child with this HOME."""
        code = (
            "import importlib.util,sys,json;"
            f"spec=importlib.util.spec_from_file_location('bb', {self.booter!r});"
            "m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m);"
            f"v=m.{fn}();"
            "print('NONE' if v is None else json.dumps(v))"
        )
        env = dict(os.environ, HOME=str(self.home))
        r = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True,
                           timeout=60, env=env)
        return (r.stdout or "").strip().splitlines()[-1] if r.stdout.strip() else f"ERR:{r.stderr[-200:]}"

    def monitor(self, *argv, timeout=120):
        """The sibling watchdog, whose remedy is to TYPE. `--gui-host` is given
        so nothing probes for a live desktop, and every tick is `--dry-run`: a
        test that could reach a real row is not a test, it is an incident."""
        env = dict(os.environ, HOME=str(self.home))
        env.pop("YGGTERM_SESSION_ID", None)
        env.pop("YGG_GUI_HOST", None)
        mon = str(Path(self.booter).parent / "ygg-monitor.py")
        return subprocess.run([sys.executable, mon, *argv, "--gui-host", "testhost"],
                              capture_output=True, text=True, timeout=timeout, env=env)

    def mon_sub(self, uuid):
        """A monitor subscription lives in its own directory beside the booter's.
        ⚠ Not the `<uuid>.json` files at the state root — those are episode
        latches, and reading them as subscriptions overstates who is watched."""
        (self.state / "monitor").mkdir(exist_ok=True)
        p = self.state / "monitor" / f"{uuid}.json"
        p.write_text('{"uuid": "%s", "role": "relay", "host": "testhost"}' % uuid)
        return p

    def sub_file(self, uuid, kind="task"):
        p = self.state / "booter" / f"{uuid}.json"
        p.write_text('{"uuid": "%s", "kind": "%s", "campaign": "test"}' % (uuid, kind))
        return p

    def unreadable(self):
        self.neverarm.chmod(0)

    def readable(self):
        self.neverarm.chmod(stat.S_IRUSR | stat.S_IWUSR)

    def cleanup(self):
        # ⛔ A SUBSCRIBE THAT SUCCEEDS SPAWNS A REAL WATCHER PROCESS, and it
        #    outlives the sandbox: `--booter <pre-fix script>` armed one here,
        #    which then ticked on against a home directory that no longer
        #    existed. Nothing it could reach was real, but a test that leaves
        #    daemons behind is a test nobody runs twice. Reap by $HOME, which is
        #    what makes a process ours, and anchor on argv[0] rather than
        #    matching the whole cmdline — this file's own path appears in the
        #    command line of the shell that started it.
        me = f"HOME={self.home}"
        for entry in Path("/proc").iterdir():
            if not entry.name.isdigit():
                continue
            try:
                argv = (entry / "cmdline").read_bytes().split(b"\0")
                if not argv or not argv[0] or Path(argv[0].decode()).name not in ("python3", "python"):
                    continue
                if b"ygg-booter" not in b" ".join(argv) or b"watch" not in b" ".join(argv):
                    continue
                if me not in (entry / "environ").read_bytes().decode("utf-8", "replace").split("\0"):
                    continue
                os.kill(int(entry.name), 15)
                print(f"  (reaped sandbox watcher pid {entry.name})")
            except Exception:
                continue
        if self.neverarm.exists():
            self.readable()
        shutil.rmtree(self.home, ignore_errors=True)



def reset_time_screens(booter_path, sb):
    """The quota hold must use the reset time the CLI already gave us.

    ⛔ The screen that matters most is the LAST one. The other three could all
    pass while the change still made things worse, because the only way this
    edit could hurt is by INVENTING A LONGER HOLD than the blind timer would
    have taken. A dead fleet is the expensive direction; one refused probe boot
    is not. So the ceiling is asserted separately from the parsing."""
    code = r'''
import importlib.util, sys, datetime, json
spec = importlib.util.spec_from_file_location("bb", sys.argv[1])
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
base = datetime.datetime.now().replace(hour=12, minute=0, second=0,
                                       microsecond=0).timestamp()
def hhmm(e):
    return None if e is None else datetime.datetime.fromtimestamp(e).strftime("%H:%M")
out = {}

# 1. the real message this was written for — the reset is 20 minutes away
out["parses_real_tail"] = hhmm(m.reset_time_from_tail(
    "You've hit your session limit · resets 12:20pm (Asia/Kolkata)", base)) == "12:20"

# 2. ⛔ A TAIL WHOSE RESET HAS ALREADY PASSED IS A STALE SCREEN, NOT A RESET
#    TOMORROW. It must decline to answer so the blind timer decides.
out["stale_tail_declines"] = m.reset_time_from_tail("resets 9am", base) is None

# 3. no reset time in the message at all ⇒ decline, today's behaviour unchanged
out["no_time_declines"] = m.reset_time_from_tail("try again later", base) is None

# 4. ⛔⛔ THE CEILING. Across every shape, the hold may only ever be SHORTER
#    than the blind timer would have made it.
timer = base + m.RATE_LIMIT_HOLD_SECS
worst = 0.0
for tail in ["resets 12:20pm (Asia/Kolkata)", "resets 1:05pm", "resets 9am",
             "resets 14:30", "resets 11:59pm", "try again later", "",
             "resets 12:00pm", "resets 25:99", "resets 12:20pm (Not/AZone)"]:
    r = m.reset_time_from_tail(tail, base)
    until = timer if not r else max(base + 60, min(timer, r + m.RESET_GRACE_SECS))
    worst = max(worst, until - timer)
out["never_exceeds_timer"] = worst <= 0

# 5. ⛔⛔ THE DEADLOCK. A parked row's tail never changes, so re-reading it must
#    NOT push the deadline out. Without this the hold can never expire, because
#    the rows are parked BECAUSE of the hold.
import os, tempfile
U = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa"
tp = os.path.expanduser("~/.claude/projects/p")
os.makedirs(tp, exist_ok=True)
tf = os.path.join(tp, U + ".jsonl")
open(tf, "w").write('{"t":1}\n')
FUTURE = "You've hit your session limit · resets 11:59pm"
r1 = m.note_rate_limit(U, FUTURE)
r2 = m.note_rate_limit(U, FUTURE)                 # same frozen tail, nothing written
out["frozen_tail_does_not_extend"] = abs(r2["until"] - r1["until"]) < 0.01
out["frozen_tail_is_flagged"] = r2.get("stale_sighting") is True
open(tf, "a").write('{"t":2}\n')                  # the row actually wrote something
os.utime(tf, None)
r3 = m.note_rate_limit(U, FUTURE)
out["real_outage_still_extends"] = r3["until"] > r2["until"]

# 6. ⛔ A TAIL IS SELF-DATING. A reset already behind us is evidence the outage
#    ENDED, so it must not arm a hold at all -- otherwise clearing the state file
#    simply re-arms from the same stale message, which is what was measured.
import datetime
base = datetime.datetime.now().replace(hour=13, minute=30, second=0,
                                       microsecond=0).timestamp()
out["passed_reset_reads_as_parked"] = m.tail_reset_has_passed(
    "You've hit your session limit · resets 12:20pm (Asia/Kolkata)", base) is True
out["future_reset_is_a_real_outage"] = m.tail_reset_has_passed("resets 2:00pm", base) is False
out["wild_past_parse_does_not_disarm"] = m.tail_reset_has_passed("resets 3:00am", base) is False
print(json.dumps(out))
'''
    env = dict(os.environ, HOME=str(sb.home))
    r = subprocess.run([sys.executable, "-c", code, str(booter_path)],
                       capture_output=True, text=True, timeout=90, env=env)
    try:
        got = json.loads((r.stdout or "").strip().splitlines()[-1])
    except Exception:
        got = {}
    for k, label in (
        ("parses_real_tail", "the quota hold READS the reset time the CLI gave it"),
        ("stale_tail_declines", "⛔ a reset time already past reads as a STALE TAIL, not tomorrow"),
        ("no_time_declines", "a message with no reset time falls back to the timer"),
        ("never_exceeds_timer", "⛔⛔ a parsed reset may only SHORTEN the hold, never extend it"),
        ("frozen_tail_does_not_extend", "⛔⛔ a FROZEN tail does not push the hold out — the deadlock"),
        ("frozen_tail_is_flagged", "a stale sighting is recorded as one, not silently ignored"),
        ("real_outage_still_extends", "a row that WROTE since the last sighting still extends the hold"),
        ("passed_reset_reads_as_parked", "⛔ a reset already behind us reads as PARKED, and does not arm"),
        ("future_reset_is_a_real_outage", "a reset still ahead is a real outage and does arm"),
        ("wild_past_parse_does_not_disarm", "⛔ a reset far in the past does NOT disarm the fleet"),
    ):
        check(label, got.get(k) is True,
              f"got={got.get(k)!r} rc={r.returncode} {(r.stderr or r.stdout)[-200:]}")


def choice_prompt_screens(booter_path, sb):
    """`_pty_type_and_enter` must refuse a row whose SCREEN shows a choice prompt.

    ⛔ Driven in-process with `_run` stubbed, because the point of two of the three
    screens is what happens when the screen CANNOT be read — and an unreadable
    screen is not a clear one. A watchdog that types must refuse on doubt."""
    code = r'''
import importlib.util, sys, types
spec = importlib.util.spec_from_file_location("bb", sys.argv[1])
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
ROW = "remote-cc://testhost/aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa"
out = {}
def stub(screen_text, accept=True):
    def _run(host, argv, stdin_text="", **kw):
        r = types.SimpleNamespace(stdout="", stderr="", returncode=0)
        if "read-buffer" in argv:
            r.stdout = screen_text
        else:
            r.stdout = '{"data": {"accepted": true}}' if accept else "{}"
        return r
    return _run

# 1. a plan-limit prompt on screen ⇒ REFUSE, and never reach the write
m._run = stub("You have hit your session limit. 1. Stop and wait  2. Team account")
out["refuses_choice"] = m._pty_type_and_enter("h", ROW) == "refused-choice-prompt"

# 2. the screen could not be read ⇒ REFUSE. Blind is not clear.
m._run = stub("")
out["refuses_blind"] = m._pty_type_and_enter("h", ROW) == "refused-screen-unreadable"

# 3. an ordinary working screen ⇒ proceed
m._run = stub("pi@host:~$ some ordinary output scrolling by")
out["proceeds_when_clear"] = m._pty_type_and_enter("h", ROW) == "pty-write"
import json; print(json.dumps(out))
'''
    env = dict(os.environ, HOME=str(sb.home))
    r = subprocess.run([sys.executable, "-c", code, str(booter_path)],
                       capture_output=True, text=True, timeout=90, env=env)
    try:
        got = json.loads((r.stdout or "").strip().splitlines()[-1])
    except Exception:
        got = {}
    for k, label in (
        ("refuses_choice", "⛔⛔ boot REFUSES a row whose screen shows a choice prompt"),
        ("refuses_blind", "⛔ boot REFUSES when the screen cannot be read"),
        ("proceeds_when_clear", "boot PROCEEDS on an ordinary screen"),
    ):
        check(label, got.get(k) is True,
              f"got={got.get(k)!r} rc={r.returncode} {(r.stderr or r.stdout)[-200:]}")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--booter", default=str(DEFAULT_BOOTER))
    a = ap.parse_args()
    if os.geteuid() == 0:
        print("⛔ refusing to run as root: chmod 0 does not make a file unreadable "
              "to root, so the unreadable cases would silently pass.")
        return 2
    sb = Sandbox(a.booter)
    try:
        # ── the readers, at the level where the ambiguity lives ───────────────
        check("absent never-arm list reads as a real, empty answer",
              sb.reader("never_arm") == "{}", sb.reader("never_arm"))

        sb.neverarm.write_text(f"# who attends which row\n{ATTENDED}\ta person types here\n")
        v = sb.reader("never_arm")
        check("a listed row is screened", ATTENDED in v, v)

        sb.unreadable()
        v = sb.reader("never_arm")
        check("⛔ an UNREADABLE never-arm list is not an empty one", v == "NONE", v)

        # ── the callers, which is where the damage would be done ─────────────
        r = sb.run("subscribe", "--row", rowaddr(DELEGATE))
        check("⛔ subscribe REFUSES while the attended list is unreadable",
              r.returncode == 4 and "UNREADABLE" in r.stdout, f"rc={r.returncode} {r.stdout[-160:]}")

        r = sb.run("coverage")
        check("⛔ coverage REFUSES while the attended list is unreadable",
              r.returncode == 2 and "UNREADABLE" in r.stdout, f"rc={r.returncode} {r.stdout[-160:]}")

        r = sb.run("retire", "--row", DELEGATE, "--evidence", "invented")
        check("⛔ retire REFUSES while the attended list is unreadable",
              r.returncode == 4, f"rc={r.returncode} {r.stdout[-160:]}")

        sb.sub_file(DELEGATE)
        r = sb.run("tick", "--dry-run")
        check("⛔ the tick BOOTS NOTHING while the attended list is unreadable",
              "BOOTING NOTHING" in r.stdout, f"rc={r.returncode} {r.stdout[-200:]}")
        (sb.state / "booter" / f"{DELEGATE}.json").unlink()

        # ── a torn or foreign line is unreadable too ─────────────────────────
        sb.readable()
        sb.neverarm.write_text(f"{ATTENDED}\ta person types here\nnot-a-uuid-at-all\n")
        v = sb.reader("never_arm")
        check("⛔ a line this parser cannot make sense of fails the whole screen",
              v == "NONE", v)

        # ── recording a decision, through the tool ───────────────────────────
        sb.neverarm.write_text("")
        r = sb.run("never-arm", "--row", ATTENDED, "--note", "a person types here",
                   "--decided-by", "test")
        check("never-arm records and reads back",
              r.returncode == 0 and "read-back: present" in r.stdout,
              f"rc={r.returncode} {r.stdout[-160:]}")

        r = sb.run("never-arm", "--row", ATTENDED)
        check("never-arm refuses a decision with no reason", r.returncode == 2)

        sb.sub_file(ATTENDED)
        r = sb.run("never-arm", "--row", ATTENDED, "--note", "again")
        check("never-arm is idempotent AND drops a subscription it contradicts",
              r.returncode == 0 and not (sb.state / "booter" / f"{ATTENDED}.json").exists(),
              r.stdout[-160:])

        r = sb.run("subscribe", "--row", rowaddr(ATTENDED))
        check("⛔ subscribe REFUSES an attended row", r.returncode == 3,
              f"rc={r.returncode} {r.stdout[-160:]}")

        r = sb.run("optout", "--row", FINISHED, "--note",
                   "work complete, waiting on a decision; nothing to continue")
        check("optout records and reads back",
              r.returncode == 0 and "read-back: present" in r.stdout,
              f"rc={r.returncode} {r.stdout[-160:]}")

        r = sb.run("subscribe", "--row", rowaddr(FINISHED))
        check("⛔ subscribe REFUSES a row that opted out", r.returncode == 5,
              f"rc={r.returncode} {r.stdout[-160:]}")

        r = sb.run("optout", "--row", ATTENDED, "--note", "weaker claim about an attended row")
        check("optout does not weaken a never-arm entry",
              "stronger" in r.stdout and ATTENDED not in (sb.state / "booter-disarmed.tsv").read_text(),
              r.stdout[-160:])

        r = sb.run("optout", "--row", DELEGATE, "--note", "__rearmed__:sneaky")
        check("optout refuses a reason that would read as a RE-ARM", r.returncode == 2)

        # ── a watch placed by somebody else must outlive the row's own "done" ─
        # ⚠ Deliberately does NOT reset never-arm.tsv: the monitor cases below
        #    need ATTENDED still listed, and the first version of this block
        #    blanked it and made the next test fail for a reason that had
        #    nothing to do with the monitor.
        r = sb.run("subscribe", "--row", rowaddr(DELEGATE))
        rec = (sb.state / "booter" / f"{DELEGATE}.json").read_text() if \
            (sb.state / "booter" / f"{DELEGATE}.json").exists() else ""
        check("a THIRD-PARTY subscription defaults to monitor, not task",
              '"kind": "monitor"' in rec, f"rc={r.returncode} {rec[:120]}")
        r = sb.run("unsubscribe", "--row", DELEGATE)
        check("⛔ and the row cannot then unsubscribe itself when it feels done",
              r.returncode == 3 and (sb.state / "booter" / f"{DELEGATE}.json").exists(),
              f"rc={r.returncode} {r.stdout[-160:]}")
        r = sb.run("subscribe", "--row", rowaddr(DELEGATE), "--kind", "task")
        rec = (sb.state / "booter" / f"{DELEGATE}.json").read_text()
        check("an explicit --kind still wins", '"kind": "task"' in rec, rec[:120])
        (sb.state / "booter" / f"{DELEGATE}.json").unlink()

        # ── the OTHER watchdog, which types a message AND a lone CR ───────────
        sb.mon_sub(ATTENDED)
        r = sb.monitor("tick", "--dry-run")
        check("⛔ the monitor REFUSES to wake an attended row that gained a subscription",
              "NEVER-ARM" in r.stdout and "dropping the subscription" in r.stdout,
              f"rc={r.returncode} {r.stdout[-200:]}")

        # ⛔⛔ A BARE UUID IS NOT AN ADDRESS, AND STORING ONE ARMS NOTHING.
        # `row_presence` asks the row plane at --host; a bare uuid resolves absent
        # every tick, so the watchdog lapses a LIVE row as "GONE". Reported by a
        # sibling campaign 2026-08-14 after repairing a live instance; the root was
        # structural — their spawn wrapper raced and exited non-zero on a spawn that
        # had SUCCEEDED, so a human compensated with a hand-rolled subscribe, and
        # the hand-rolled call reproduced the failure the wrapper existed to prevent.
        r = sb.run("subscribe", "--row", DELEGATE)          # deliberately BARE
        check("⛔ subscribe REFUSES a bare uuid as a row address",
              r.returncode == 6 and "not an addressable row" in (r.stdout + r.stderr),
              f"rc={r.returncode} {(r.stdout + r.stderr)[-200:]}")

        choice_prompt_screens(a.booter, sb)
        reset_time_screens(a.booter, sb)

        sb.unreadable()
        r = sb.monitor("tick", "--dry-run")
        check("⛔ the monitor WAKES NOBODY while the attended list is unreadable",
              "WAKING NOBODY" in r.stdout, f"rc={r.returncode} {r.stdout[-200:]}")
        sb.readable()
    finally:
        sb.cleanup()

    print()
    if FAILURES:
        print(f"⛔ {len(FAILURES)} failed: {', '.join(FAILURES)}")
        return 1
    print("all screens hold")
    return 0


if __name__ == "__main__":
    sys.exit(main())
