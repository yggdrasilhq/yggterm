#!/usr/bin/env python3
"""ygg-booter — a session SUBSCRIBES, and something outside it kicks it when it stalls.

⛔⛔ THE DEFECT THIS EXISTS FOR (recorded 2026-08-09):

    *"I have seen you stall sometimes, so arm a booter in a fleet. A booter is a
    tool that monitors any session that has subscribed to it, to kick it and say
    'continue, the booter booted'. Sometimes you may feel that the work is done
    so you need to unsubscribe from the booter."*

He was manually booting a stalled relay session when it was stated it. **That is the
whole specification: the human should never be the watchdog.**

★ WHY THIS IS NOT `ygg-babysit.py`, and why both exist.

    babysit  — an ORCHESTRATOR watches rows IT SPAWNED, for the length of one run.
    booter   — a session watches ITSELF, by subscribing; the watcher OUTLIVES it.

The difference is load-bearing and it is not a style choice: **a stalled session
cannot boot itself.** Anything that runs inside the session — a wakeup it
schedules, a loop it drives, a check at the end of its own turn — is dead in
exactly the case that matters, because the stall IS the turn ending early. So the
booter's watcher is a DETACHED process that survives its subscribers, and
subscribing is a thing you do TO it, not a thing you run inside your own loop.

⛔ THE CLASSIFIER IS NOT DUPLICATED HERE. It is imported from `ygg-babysit.py`,
   which owns "is this row working, idle, stuck or gone" — one question, one
   owner. A second copy would drift, and the two would disagree about a live row
   on the day it mattered. Everything babysit learned the hard way therefore
   applies for free: ask the ROW LIST before the transcript (a retired row's
   transcript is frozen mid-turn and reads as a live wedge); never type into a
   MID-TURN row (it races the agent's own input); "I could not look" is not "it
   is not there".

★ THE END CONDITION IS A DECISION, NOT A TIMEOUT. A subscription ends when the
  work does, and only the subscriber knows that — hence `unsubscribe`. Three
  cheaper endings are automatic, because each is a fact rather than a judgement:
  the row is GONE (retired by a relay — a corpse must not be booted forever), the
  subscription passed its `--max-hours`, or the booter escalated to a human and
  the human owns it now.

Usage:
    ygg-booter.py subscribe [--row <path>] [--campaign yggterm] [--max-hours 12]
                            [--kind task|monitor]   # monitor: unsubscribe needs --force
    ygg-booter.py unsubscribe [--row <path>]        # no --row = this session
    ygg-booter.py list [--json]
    ygg-booter.py tick [--dry-run]                  # one pass over all subscribers
    ygg-booter.py watch [--interval 300]            # the loop — let `subscribe` spawn it
    ygg-booter.py status [--json]                   # alive AND audible? (MUTE is a fault)
    ygg-booter.py disarm [--hours 4|--forever] [--note why]   # the OFF SWITCH
    ygg-booter.py arm                               # back on

★ THE OFF SWITCH, and why it is not `unsubscribe` (added 2026-08-13).

  Reported: subscribers kept being booted during a rate-limited window, and there
  was no way to check whether the booter was the thing causing the pain, let
  alone stop it, without a shell on every host. Two gaps, both real:

  **1. No disarm.** Stopping the booter meant deleting subscriptions or killing
  the watcher — both destructive, both losing WHO was being watched, so re-arming
  meant rebuilding the list from memory. `disarm` keeps every subscription and
  simply refuses to act; `arm` puts it back exactly as it was. It expires on its
  own (default 4h) for the same reason a deferral does: a safety net switched off
  and forgotten is worse than one that was never installed, because everybody
  still believes it is there.

  **2. No quota awareness.** A rate limit is an ACCOUNT state, not a session
  state — the session is fine, the account cannot spend — and it is invisible to
  anything measuring activity, because a refused turn ends like any other. So a
  quota outage read as a stall and got booted every few minutes, and each boot
  was refused before the agent ran. The classifier now names it (`RATE_LIMITED`,
  keyed on the CLI's own `apiErrorStatus: 429` record, not on prose) and one
  sighting holds the WHOLE fleet, because the limit is account-wide while
  detection can only ever be per-row.

  Both are readable and drivable from outside via `--json`, which is what the
  `yggtopo` surface renders — the point being that a human should be able to
  answer "is the booter what is hurting me" without opening a shell at all.

`subscribe` ARMS the watcher if none is running, so arming the booter and joining
it are one call — a two-step arm is a step somebody skips.

Exit: 0 nothing to do · 3 something was booted · 4 a human is needed.
"""
import argparse
import importlib.util
import json
import os
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ygg_host import resolve_gui_host  # noqa: E402

HERE = Path(__file__).resolve().parent
STATE = Path.home() / ".yggterm" / "relay"
SUBS = STATE / "booter"
PIDFILE = STATE / "booter.pid"
HEARTBEAT = STATE / "booter.heartbeat"
LOGPATH = STATE / "booter.log"
# ⭐ THE OFF SWITCH. A human standing the booter down, without dismantling it.
DISARMFILE = STATE / "booter.disarmed"
# ⭐ THE ACCOUNT HAS NO QUOTA. Fleet-wide, because a rate limit is.
RLHOLDFILE = STATE / "booter.rate-limit-hold"

# The words he used. A boot must be recognisable AS a boot in the transcript —
# both to a human reading back and to the session itself, which should be able to
# tell "a machine woke me" from "a person asked for something".
#
# ⭐⭐ AND IT NOW CARRIES A STEER, recorded 2026-08-10: *"if the booter is armed then I
#   want a relay. relay and booter go hand in hand and are part of a system."*
#
#   His reasoning is exact, and it is the same argument as putting a refusal at the point of
#   the mistake rather than in a document: THE SUBSCRIPTION IS THE SIGNAL. Only long-running
#   relay work subscribes, so this text can assert relay context with no false positives, and
#   it lands at the one moment that is definitionally "a turn just ended".
#
# ⛔ THE PREFIX IS UNCHANGED ON PURPOSE. Detectors in the wild match the exact phrase
#   "continue, the booter booted" (row-health probes, delivery checks, a human's eye). The
#   steer is APPENDED so every existing matcher keeps working — changing the marker to improve
#   the message would have broken the instruments that prove the message arrived.
#
# ⚠ AND THE HONEST LIMIT, measured the day it was written: this reaches a session that stalled
#   SILENTLY. It does NOT reach one that stalled BY ASKING — a question gets answered by him,
#   the row classifies as WORKING, and no boot ever fires. That hole is closed elsewhere (the
#   context-gauge hook's NOTICE tier), not here. Two different stalls, two different cures.
BOOT_TEXT = (
    "continue, the booter booted"
    " — you are a RELAY session: the booter and the relay are one system, so if you are "
    "subscribed you are long-running work. DECIDE and ACT; do not end this turn on a "
    "question. If the next plane belongs to a fresh session, execute the handover yourself "
    "(brief, spawn, verify armed, stand down). If the work is genuinely finished, "
    "`ygg-booter.py unsubscribe` — but ⛔ a MONITOR is never finished while the thing it "
    "watches is live."
)

# How long a subscribed row may sit with its turn ENDED before it is booted.
# Deliberately longer than babysit's 240s: a subscriber is a long-running campaign
# session that may legitimately pause between phases, and a boot costs it a turn.
#
# ⛔ THIS STAYS THE DEFAULT FOR EVERY SUBSCRIBER. A session that KNOWS it is about
# to wait — a release build, a 40-minute test target, an ssh fan-out — asks for a
# longer window for that wait only (`defer`, below). Raising the default instead
# would make every genuinely stalled session in the fleet sit undetected for the
# length of the slowest thing anyone ever does.
BOOT_AFTER_SECS = 420

# ⛔⛔ THE CEILING IS A BILLING LIMIT, NOT A TUNING KNOB — recorded
# 2026-08-09. The plan's prompt cache stays hot for ~1 hour; a session that does
# NOTHING for an hour comes back to a COLD cache, and re-reading a large campaign
# context at full price is the expensive failure. So no session may ever ask to
# be left alone for an hour. 55 min leaves five minutes of margin for the tick
# interval and the boot's own round trip.
# ⇒ requests above this are CLAMPED, never refused: a refusal would leave the
# caller on the 420s default, which is the opposite of what it asked for.
#
# ⚠ WHY 3000 AND NOT 3300 — the number that must stay under the hour is the
# WORST-CASE DELIVERY, not the setting. The watcher only looks every
# DEFAULT_INTERVAL (300s), so a row crossing its window right after a tick waits
# almost a full interval more: 3000 + 300 = 3300s (55 min) worst case, which is
# the real five minutes of margin. Setting this to 3300 would put worst-case
# delivery at exactly 3600s — the cache expiry itself, margin zero. Any change
# here must keep `MAX_BOOT_AFTER_SECS + DEFAULT_INTERVAL` comfortably below 3600.
MAX_BOOT_AFTER_SECS = 3000
MIN_BOOT_AFTER_SECS = 60

# ⭐ THE ONE EXCEPTION THE OWNER NAMED: a session running sub-agents or workflows
# INSIDE itself is mid-turn, and `classify` already reports that as STUCK, which
# escalates instead of booting ("a boot here races the agent's own input"). Those
# sessions keep their own cache warm by working, so the ceiling does not apply and
# no `defer` is needed. ⚠ In relay mode sub-agents are discouraged anyway, so this
# arm should almost never fire — if it fires often, something is spawning agents
# that should not be.
# Consecutive boots that produced no transcript growth before a human is told.
MAX_BOOTS = 3
DEFAULT_INTERVAL = 300

# ⭐⭐ HOW LONG THE FLEET HOLDS AFTER SEEING A 429 (reported 2026-08-13: "during a
# rate-limited window, subscribers kept being booted").
#
# ⛔ THE HOLD IS FLEET-WIDE BECAUSE THE LIMIT IS ACCOUNT-WIDE. Detection is
# necessarily per-row — only a session that TRIED gets refused — but the fact it
# discovers is about the account, so one subscriber's 429 stands the whole
# watcher down. Holding only the row that happened to be refused would boot every
# other subscriber into the same wall, one at a time, which is the reported
# symptom exactly.
#
# ⚠ WHY A TIMER AND NOT A RESET TIME: the refusal carries no reset timestamp
# ("try again later"), the CLI keeps no quota state file, and there is no API to
# ask. Inventing a reset time we cannot observe would be a confident lie. So the
# hold expires on a timer and the NEXT tick sends one boot as a PROBE; if the
# account is still dry that row classifies RATE_LIMITED again and the hold
# re-arms. Cost of being wrong is one wasted boot per half hour, against one
# every seven minutes today.
#
# ⛔ AND IT MUST NOT ESCALATE. A quota window is not something a human can fix,
# and the owner is the person whose quota it is — he already knows. A watchdog
# that pages about the weather teaches people to ignore it, which is the failure
# the CONTEXT_DEAD arm was written to undo.
RATE_LIMIT_HOLD_SECS = 1800

# ⭐ HOW LONG A MANUAL DISARM LASTS BY DEFAULT.
#
# ⛔ IT EXPIRES ON ITS OWN, for the same reason `defer` does: a safety net
# switched off and forgotten is worse than one that never existed, because
# everybody still believes it is there. Long enough to cover the reason a human
# reaches for it (a noisy window, a hands-on debugging session), short enough
# that forgetting is not fatal. `--forever` exists for the case that genuinely
# needs it and makes the choice explicit rather than accidental.
DISARM_HOURS = 4.0


def _load_babysit():
    """Import the sibling classifier. One owner for row liveness."""
    spec = importlib.util.spec_from_file_location("ygg_babysit", HERE / "ygg-babysit.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


BB = _load_babysit()


def _watcher_silence_secs(beat=None):
    """How long has the RUNNING WATCHER been silent? `None` = cannot tell.

    Reads `last_log_write_ts` out of the heartbeat: both fields are written by
    the watcher about itself, so their gap is that watcher's own silence.
    ⛔ Do NOT substitute `booter.log`'s mtime — every `subscribe`/`list`/`status`
    touches that file too, so on a busy fleet it stays fresh and MASKS a mute
    watcher. That is the same wrong-subject error the bare heartbeat made.

    Falls back to the file mtime only for a PRE-FIX watcher whose heartbeat has
    no such field — the one case where the file is the only evidence there is.
    """
    try:
        if beat is None:
            beat = json.loads(HEARTBEAT.read_text())
        if "last_log_write_ts" in beat:
            last = float(beat["last_log_write_ts"] or 0)
            if last <= 0:
                # Never logged at all. The silence is its whole uptime — and a
                # watcher with no `started_ts` either predates this field or is
                # lying, so treat that as maximally silent rather than healthy.
                started = float(beat.get("started_ts", 0) or 0)
                if started <= 0:
                    return float("inf")
                return float(beat.get("ts", time.time())) - started
            return float(beat.get("ts", time.time())) - last
        return time.time() - LOGPATH.stat().st_mtime
    except Exception:
        return None


def _stdout_is_the_log():
    """Is our stdout ALREADY the log file? Then writing by path too would double
    every line. Compared by (device, inode), not by name."""
    try:
        s = os.fstat(sys.stdout.fileno())
        t = LOGPATH.stat()
        return (s.st_dev, s.st_ino) == (t.st_dev, t.st_ino)
    except Exception:
        return False


_STDOUT_IS_LOG = None
# When THIS process last wrote a decision line successfully. The subject of the
# mute test has to be the WATCHER's own speech: `booter.log`'s mtime is touched by
# every `subscribe`/`list`/`status` too, so a busy fleet keeps the file fresh and
# masks a watcher that has gone silent. Same trap the heartbeat fell into — a fact
# about the wrong subject.
_LAST_LOG_WRITE_TS = 0.0
# When this watcher started. A watcher that has NEVER logged is mute, and the only
# honest measure of that silence is its own uptime.
_WATCH_STARTED_TS = 0.0


def log(m):
    """Write a decision line to BOTH stdout and the log file BY PATH.

    ⛔⛔ THE LOG USED TO BE `print()` ALONE — i.e. it went wherever stdout
    happened to point, which is a property of whoever SPAWNED us. The heartbeat,
    two functions down, writes `booter.heartbeat` by PATH. Those are different
    mechanisms for the same subject, and one of them can die without the other
    noticing. It did.

    **Measured 2026-08-11 (reported by a sibling campaign whose relay row was
    woken ~27 min late and could not be diagnosed):** `booter.heartbeat` was
    current to the second while `booter.log` had not been touched for **21
    hours**. The watcher was alive, ticking, and re-arming subscribers correctly
    the whole time — `/proc/741787/fd/1 -> /dev/null`. Every `log()` call
    succeeded and wrote to nothing. `ensure_watcher` had spawned an earlier
    watcher with `stdout=logf`, but THIS one was started detached by something
    else, and this module's own usage line invited exactly that: *"watch — the
    loop (usually detached)"*.

    ⇒ **A SCHEDULER THAT HEARTBEATS WITHOUT LOGGING ITS DECISIONS CAN BE NEITHER
    EXONERATED NOR CONVICTED.** The default outcome is worse than silence: the
    reporting row initially wrote a mea culpa accepting blame for its own arming,
    then measured and retracted it. A blackout does not merely hide the defect,
    it MISATTRIBUTES it — and it makes every future "the booter woke me late"
    report from any campaign unfalsifiable.

    Same family as a health check that ANDs facts about two different subjects
    and passes on a corpse: liveness is not speech.
    """
    global _STDOUT_IS_LOG, _LAST_LOG_WRITE_TS
    line = f"{time.strftime('%H:%M:%S')} ygg-booter {m}"
    print(line, flush=True)
    if _STDOUT_IS_LOG is None:
        _STDOUT_IS_LOG = _stdout_is_the_log()
    if _STDOUT_IS_LOG:
        _LAST_LOG_WRITE_TS = time.time()
        return
    try:
        LOGPATH.parent.mkdir(parents=True, exist_ok=True)
        with open(LOGPATH, "a") as f:
            f.write(line + "\n")
        _LAST_LOG_WRITE_TS = time.time()
    except Exception:
        # Never let a logging failure take down a scheduler tick — this is the
        # ONE place that may swallow. It is safe to swallow only because
        # `_LAST_LOG_WRITE_TS` is NOT updated on failure, so the heartbeat still
        # carries the silence outward.
        pass


def this_host():
    return os.uname().nodename


def ygg(host, *args):
    """A yggterm app verb, run WHERE THE GUI IS.

    ⚠ App control resolves only on the GUI host, so a booter running on any other
    machine must route there. When the booter already runs on that host, skip ssh
    — an ssh to yourself is a second authentication and a second failure mode for
    no gain."""
    if host == this_host():
        cmd = [str(Path.home() / ".local" / "bin" / "yggterm"), *args]
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    else:
        binp = "$HOME/.yggterm/bin/yggterm"
        joined = " ".join(f"'{a}'" for a in args)
        r = subprocess.run(["ssh", host, f"{binp} {joined}"],
                           capture_output=True, text=True, timeout=120)
    try:
        return json.loads(r.stdout[r.stdout.find("{"):])
    except Exception:
        return {}


def sub_path(uuid):
    SUBS.mkdir(parents=True, exist_ok=True)
    return SUBS / f"{uuid}.json"


def load_subs():
    if not SUBS.exists():
        return []
    out = []
    for p in sorted(SUBS.glob("*.json")):
        try:
            out.append(json.loads(p.read_text()))
        except Exception:
            continue
    return out


def own_uuid():
    return (os.environ.get("YGGTERM_SESSION_ID") or "").rstrip("/").split("/")[-1]


# ─── the off switch ────────────────────────────────────────────────────────────

def disarm_state():
    """The active disarm, or None. Expiry is evaluated HERE, once.

    ⛔ One reader, so `tick`, `status` and the app surface can never disagree
    about whether the booter is on — the question a human asks the off switch is
    "is it off RIGHT NOW", and two answers to that is the same defect as two
    files claiming to list the open bugs."""
    try:
        d = json.loads(DISARMFILE.read_text())
    except Exception:
        return None
    until = d.get("until") or 0
    if until and time.time() >= until:
        # Self-clearing, and it says so out loud: a watchdog that silently comes
        # back on is as surprising as one that silently stays off.
        DISARMFILE.unlink(missing_ok=True)
        log(f"⭐ disarm EXPIRED ({d.get('note') or 'no reason given'}) — the booter is ARMED again")
        return None
    return d


def cmd_disarm(args):
    """Stand the booter down WITHOUT dismantling it.

    ⛔ THIS IS NOT `unsubscribe`, and the difference is the whole point. Today the
    only way to stop the booter is to delete subscriptions or kill the watcher —
    both of which lose who was being watched, so re-arming means reconstructing
    it from memory, and nobody does. A disarm keeps every subscription intact and
    simply refuses to act, so `arm` puts the fleet back exactly as it was."""
    hours = None if args.forever else float(args.hours or DISARM_HOURS)
    rec = {
        "since": time.time(),
        "until": 0 if hours is None else time.time() + hours * 3600,
        "hours": hours,
        "note": args.note,
        "by": own_uuid() or "shell",
        "host": this_host(),
    }
    DISARMFILE.parent.mkdir(parents=True, exist_ok=True)
    DISARMFILE.write_text(json.dumps(rec, indent=1))
    back = disarm_state()
    span = "until re-armed by hand" if hours is None else f"for {hours:g}h"
    log(f"⛔ booter DISARMED {span} on {this_host()} — {len(load_subs())} subscriptions kept, "
        f"nobody will be booted. Reason: {args.note or 'none given'}")
    log(f"   re-arm with: ygg-booter.py arm")
    log(f"read-back: {'present' if back else '⛔ ABSENT — the disarm did not land'}")
    return 0 if back else 1


def cmd_arm(args):
    d = disarm_state()
    if not d:
        log(f"booter is already armed on {this_host()}")
    else:
        DISARMFILE.unlink(missing_ok=True)
        down_m = (time.time() - d.get("since", time.time())) / 60
        log(f"⭐ booter ARMED on {this_host()} — it was down {down_m:.0f} min "
            f"({d.get('note') or 'no reason given'})")
    if disarm_state():
        log("⛔ read-back: still disarmed — the file did not clear")
        return 1
    # Arming is also the moment to notice that nothing is watching.
    if load_subs() and not watcher_alive():
        log(f"⚠ {len(load_subs())} subscriptions but NO watcher process — "
            f"re-arm the watcher too: ygg-booter.py subscribe --row <row>")
    return 0


# ─── the account ran out of quota ──────────────────────────────────────────────

def rate_limit_hold():
    """The active fleet-wide quota hold, or None. Expiry evaluated HERE, once."""
    try:
        d = json.loads(RLHOLDFILE.read_text())
    except Exception:
        return None
    if time.time() >= (d.get("until") or 0):
        RLHOLDFILE.unlink(missing_ok=True)
        return None
    return d


def row_process_absent(uuid):
    """True only when NOTHING is running as this row. ⛔ Biased toward "alive".

    ⭐ ARGV[0]-ANCHORED, because every weaker form failed in one night: a bare
    `pgrep -f <uuid>` matches the shell that asked; *"exclude grep"* does not
    exclude `bash -c`; and *"the cmdline contains `claude`"* matches every probe
    this fleet runs, since their command lines carry a `…/.claude/…` path. Read
    the FIRST NUL-separated field and judge its basename.

    ⚠ An EMPTY cmdline is a zombie or a kernel thread — `/proc/<pid>` exists for
    a reaped-but-unwaited child, so directory existence answers *has this been
    reaped*, not *is this running*. Empty ⇒ not this row.

    ⛔ The bias is deliberate and it is the safe direction. A false "absent"
    would skip a fleet hold during a REAL outage; a false "present" merely keeps
    today's behaviour. So anything that is not clearly a shell counts as alive,
    and only a total absence counts as dead."""
    shells = {"bash", "sh", "zsh", "dash", "ssh", "grep", "awk", "sed", "xargs"}
    try:
        pids = [d for d in os.listdir("/proc") if d.isdigit()]
    except Exception:
        return False                      # cannot tell ⇒ treat as alive
    needle = uuid.encode()
    for d in pids:
        try:
            cl = Path(f"/proc/{d}/cmdline").read_bytes()
        except Exception:
            continue
        if not cl or needle not in cl:
            continue
        a0 = os.path.basename(cl.split(b"\0")[0].decode("utf8", "ignore"))
        if a0 not in shells:
            return False
    return True


def note_rate_limit(uuid, tail):
    """A subscriber was refused on quota ⇒ hold the whole fleet.

    Refreshing rather than accumulating: each fresh sighting pushes the window
    out, so a long outage holds continuously without anyone tracking rounds."""
    prev = rate_limit_hold()
    rec = {
        "since": (prev or {}).get("since", time.time()),
        "last_seen": time.time(),
        "until": time.time() + RATE_LIMIT_HOLD_SECS,
        "seen_on": uuid,
        "tail": (tail or "")[:200],
    }
    RLHOLDFILE.parent.mkdir(parents=True, exist_ok=True)
    RLHOLDFILE.write_text(json.dumps(rec, indent=1))
    return rec


def resolve(host, ident):
    """Any identifier -> a real ROW PATH, or None.

    ⛔ `$YGGTERM_SESSION_ID` is `cc-runtime://<uuid>`; the row is
    `remote-cc://<host>/<uuid>`. Same uuid, different string, and the wrong one
    addresses nothing while every verb still answers OK. Resolve by UUID against
    the live row list so the mistake cannot be made.

    ⛔⛔ ITS `None` IS AMBIGUOUS AND MUST NOT DECIDE ANYTHING DESTRUCTIVE — see
    `row_presence` below. Use it to ADDRESS a row, never to judge whether one
    exists."""
    BB._ROWS_CACHE.clear()
    return BB.resolve_row_path(host, ident)


def row_presence(host, uuid):
    """TRI-STATE: True listed · False answered-and-absent · None could-not-ask.

    ⛔⛔ THE OUTAGE THIS EXISTS TO PREVENT, measured 2026-08-13 21:04:54-59:
    **nine live subscriptions were deleted in six seconds, every one of them
    logged as `GONE (retired)`, while every one of those sessions was alive and
    working.** The whole fleet lost its watchdog silently, and the only trace was
    nine confident lines in a log.

    The mechanism is one collapsed distinction. `resolve_row_path` answers `None`
    both for "the row is not in the listing" and for "the listing never arrived",
    and it builds its cache ONCE from a single call — so a single failed
    row-list makes every lookup in that pass answer `None`, and a tick that
    reads `None` as "retired" unsubscribes everybody. **A watchdog that deletes
    its own subscriptions because one probe failed cannot be relied on, and
    nothing about the failure looks like one.**

    ⇒ `row_exists` was already tri-state, and its docstring already said the
    third value is the important one: *"None = the plane did not answer at all,
    which is a fact about US and must never be rendered as a verdict about the
    row."* The verdict path simply was not using it.

    ⚠ AND EVEN `False` IS NOT PROOF ON ITS OWN — row listings have omitted live
    rows before. So `False` is counted, not acted on; see `GONE_SIGHTINGS`."""
    BB._ROWS_CACHE.clear()
    return BB.row_exists(host, uuid)


# How many consecutive answered-and-absent readings before a subscription is
# deleted. Deliberately more than one: a listing that omits a live row has been
# seen, and the cost of waiting an extra tick is nothing next to the cost of
# unsubscribing a session that is still working.
GONE_SIGHTINGS = 3


NEVERARM = STATE / "never-arm.tsv"


def never_arm():
    """⛔⛔ ROWS A HUMAN ATTENDS. THE ANSWER IS ALWAYS NO.

    This watchdog's remedy is to **type into** a row. Typing into a row a person
    is actively using splices into whatever they have half-written and submits
    the fusion as their own turn. For an unattended agent row that is a rescue;
    for an attended one it is putting words in someone's mouth.

    ⛔ **The protection here was previously an ACCIDENT and that is why this file
    exists.** Such a row was safe only because nobody had ever subscribed it —
    absent from both rosters, so nothing flagged it and nothing armed it. That is
    load-bearing and invisible, and it inverts under tidying: subscribe it to the
    monitor "because it looks unwatched", and the coverage crossing then reports
    it as a gap, whose obvious remedy is to arm it. **A well-meant cleanup would
    walk straight into typing over a person's unsent draft.** Safety that rests
    on an omission is one tidy-up from being removed, so it is written down.

    ⇒ The list lives OUTSIDE the repository, because who attends which row is not
    engineering content. This code only needs to know that the class exists.

    ⚠ A refusal cannot be tested by writing to the row — the readiness probe
    types first, which is the same hazard wearing a lab coat. **The admissible
    test is that the filter excludes it from METADATA ALONE**, which needs no
    write and is therefore both safe and cheap. If an arming path ever classifies
    one of these as armable, that path is wrong and nothing downstream ships.

    ⛔⛔ **UNREADABLE IS NOT EMPTY, AND THIS LIST IS THE ONE WHERE THAT COSTS
    MOST.** Until 2026-08-14 every read error here returned `{}` — so a list that
    could not be read (permissions, a truncated write, fd exhaustion, ENOMEM)
    reported *nobody is attended*, and the very next line of every caller is an
    arming decision. The weaker ledger beside it already failed closed, so the
    two halves of one decision disagreed about what unreadable meant and the
    careless half owned the stronger list.

    ⇒ **Absent stays a real answer** (`{}` — there is no list, and that is a fact
    a caller may act on), **unreadable travels as `None`**, and so does a line
    this parser cannot make sense of: a corrupted entry is one person's row
    silently unprotected, which is indistinguishable from the entry never having
    been written. Every caller must treat `None` as *refuse to act* — see
    `tick()`, which is the only place that types.
    """
    out = {}
    try:
        raw = NEVERARM.read_text()
    except FileNotFoundError:
        return {}
    except Exception:
        return None
    for line in raw.splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split("\t", 1)
        uuid = parts[0].strip()
        # A first field that is not uuid-shaped means this file is not the file
        # we think it is, or a write landed torn. Either way the screen cannot be
        # trusted, and a half-trusted screen is the failure being fixed here.
        if len(uuid) < 8 or any(c not in "0123456789abcdefABCDEF-" for c in uuid):
            return None
        out[uuid] = (parts[1].strip() if len(parts) > 1 else "")
    return out


DISARMED_LEDGER = STATE / "booter-disarmed.tsv"
REARM_MARK = "__rearmed__:"


def disarmed_rows():
    """⛔⛔ THE LEDGER THAT NOTHING READ. Rows that opted OUT, with a reason.

    `ygg-claim.sh --no-booter <reason>` has always APPENDED here, and until now
    **nothing read it back**. A row that deliberately opted out was therefore
    indistinguishable from one nobody had ever armed — the same
    absent-vs-refused ambiguity that let 42 of 47 rows run unwatched without
    anyone noticing, except pointing the other way.

    ⇒ That made it the load-bearing prerequisite for ANY automatic arming:
    enumerate-and-arm shipped over a write-only ledger would re-arm every
    deliberately disarmed row one tick later, while looking like it honoured
    them.

    **Append-only, latest record per uuid wins.** A re-arm is a new line whose
    reason begins with `__rearmed__:`, so the decision history is kept rather
    than rewritten — the file is evidence, not state to be edited.

    ⚠ Distinct from `never_arm()` and weaker on purpose: never-arm is *a person
    attends this row, the answer is always no*; this is *this row asked not to be
    watched, and can ask again*.
    """
    out = {}
    try:
        for line in DISARMED_LEDGER.read_text().splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) < 2:
                continue
            uuid = parts[1].strip()
            reason = parts[3].strip() if len(parts) > 3 else ""
            if not uuid:
                continue
            if reason.startswith(REARM_MARK):
                out.pop(uuid, None)
            else:
                out[uuid] = reason or "no reason recorded"
    except FileNotFoundError:
        return {}
    except Exception:
        # ⛔ UNREADABLE IS NOT EMPTY. Treating a damaged ledger as "nobody opted
        # out" is the failure that re-arms everyone; refuse to answer instead.
        return None
    return out


def record_rearm(uuid, why):
    """Append the re-arm so the ledger keeps the whole decision history."""
    try:
        STATE.mkdir(parents=True, exist_ok=True)
        with DISARMED_LEDGER.open("a") as fh:
            fh.write("%s\t%s\t%s\t%s%s\n" % (
                time.strftime("%Y-%m-%dT%H:%M:%S%z"), uuid, "", REARM_MARK, why))
        return True
    except Exception as exc:
        log(f"⛔ could not record the re-arm: {exc}")
        return False


def cmd_subscribe(args):
    uuid = (args.row or "").rstrip("/").split("/")[-1] or own_uuid()
    if not uuid:
        log("no row given and $YGGTERM_SESSION_ID is unset — nothing to subscribe")
        return 2
    opted_out = disarmed_rows()
    if opted_out is None:
        log(f"⛔ REFUSING to arm {uuid[:8]} — {DISARMED_LEDGER} is unreadable.")
        log("   An unreadable opt-out ledger is not an empty one. Fix the file,")
        log("   or pass --rearm '<why>' if you know this row never opted out.")
        if not args.rearm:
            return 4
    elif uuid in opted_out and not args.rearm:
        log(f"⛔ REFUSING to arm {uuid[:8]} — it opted OUT of the booter:")
        log(f"     {opted_out[uuid]}")
        log("   ⭐ This is the ledger `ygg-claim.sh --no-booter` writes, and it is")
        log("     now read. Re-arming is deliberate: pass --rearm '<why>', which")
        log("     appends the decision rather than editing the record away.")
        log("   ⚠ If you meant 'busy for a while', use `ygg-booter.py defer`.")
        return 5
    if args.rearm and opted_out and uuid in opted_out:
        log(f"⭐ re-arming {uuid[:8]}, which had opted out ({opted_out[uuid]})")
        record_rearm(uuid, args.rearm)
    dead = retired_rows() or {}
    if uuid in dead:
        log(f"⚠ {uuid[:8]} was recorded DEAD ({dead[uuid]}) — arming it anyway.")
        log("   Not refused: a boot at a dead pid is useless, not dangerous. But")
        log("   if it really is a corpse this buys wasted wakes; check first.")
    blocked = never_arm()
    if blocked is None:
        log(f"⛔ REFUSING to arm {uuid[:8]} — {NEVERARM} is UNREADABLE.")
        log("   That list is the only thing standing between this watchdog and")
        log("   typing into a row a person is using, and an unreadable list is")
        log("   not an empty one. Fix the file; there is deliberately no flag.")
        return 4
    if uuid in blocked:
        log(f"⛔ REFUSING to arm {uuid[:8]} — {blocked[uuid] or 'human-attended row'}")
        log("   This watchdog TYPES INTO what it wakes. Arming a row a person is")
        log("   using would splice into their unsent text and submit it as theirs.")
        log(f"   If this is genuinely wrong, remove the line from {NEVERARM}")
        log("   deliberately — do not pass a flag to route around it.")
        return 3
    row = resolve(args.host, uuid)
    if row is None:
        log(f"⚠ {uuid} does not resolve to a live row on {args.host} — "
            f"subscribing anyway; the first tick will retire it if it stays gone")
        row = args.row or uuid
    # ⛔ A RE-SUBSCRIBE MUST NOT ERASE WHAT IT DOES NOT MENTION. This record was
    #    rebuilt from scratch every time, so `subscribe --campaign X` on an
    #    existing subscriber wrote `note: None` and silently blanked a note the
    #    caller never referred to. Measured on a sibling campaign: four
    #    subscriptions lost their notes while someone was fixing something else.
    #    ⚠ It bites hardest because the note is normally set FOR you —
    #    `ygg-claim.sh` rejects a `--note` flag while passing one internally — so
    #    the only way most callers ever touch it is by destroying it.
    #    ⇒ Carry forward any field the caller left unspecified. `boots` and
    #    `subscribed_at` still reset by design: re-subscribing is a fresh watch.
    prior = {}
    try:
        prior = json.loads(sub_path(uuid).read_text())
    except Exception:
        prior = {}
    rec = {
        "uuid": uuid,
        "row": row,
        "campaign": args.campaign or prior.get("campaign"),
        "note": args.note or prior.get("note"),
        "host": args.host,
        "subscribed_at": time.time(),
        "max_hours": args.max_hours,
        # ⭐ WHAT KIND OF WATCH THIS IS. "task" has a terminal state and may
        #    unsubscribe itself when the work is done; "monitor" does not --
        #    see cmd_unsubscribe.
        "kind": getattr(args, "kind", None) or "task",
        "boots": 0,
        "last_size": 0,
        "escalated": False,
    }
    sub_path(uuid).write_text(json.dumps(rec, indent=1))
    log(f"subscribed {row} (campaign={args.campaign or '-'}, max_hours={args.max_hours})")
    armed = ensure_watcher(args)
    log(f"watcher: {armed}")
    # Read the subscription back. A write that reports success and leaves nothing
    # is the failure this whole skill is about.
    back = [s for s in load_subs() if s["uuid"] == uuid]
    log(f"read-back: {'present' if back else '⛔ ABSENT — subscription did not land'}")
    return 0 if back else 1


def update_sub(uuid, rec):
    """⛔ WRITE A SUBSCRIPTION BACK ONLY IF IT STILL EXISTS — never recreate it.

    `tick()` loads the subscription set once, then writes each record back as it
    advances counters (gone_sightings, boots, rate-limit holds). A row that
    unsubscribed DURING that tick had its file deleted after the load, so a plain
    `write_text` CREATED IT AGAIN — carrying the original `subscribed_at`, which
    is what makes this hard to see: the row reappears with a continuous age, so
    it reads as "never unsubscribed" rather than "resurrected".

    Measured 2026-08-14 and reported by a sibling campaign: unsubscribe at
    00:56:48, verified ABSENT at 00:56:50, tracked again at 00:57:10 with its age
    continuous from the original 00:13 subscription.

    ⛔ THE DOCUMENTED DEFENCE DOES NOT CATCH THIS, AND THAT IS THE POINT. This
    fleet's standing law is "a verb reports the REQUEST, not the EFFECT — assert
    on the read-back". That session DID assert on the read-back and the read-back
    was correct; the state reverted twenty seconds later. A value that is right
    when you look and wrong afterwards is a strictly harder failure than a lying
    verb, because the defence passes on its way to being wrong.

    ⇒ Consequence this removes: `AUTO ARM AND DISARM WITH REASON` (owner ruling,
    2026-08-13) had only half of it working. If a delete cannot persist, the
    subscriber cannot perform the disarm the ruling assigns it, and every row
    that correctly stands down pays a wake on the next idle window.

    `O_WRONLY` WITHOUT `O_CREAT` is the whole fix: an update that is incapable of
    creating. It also closes the window rather than narrowing it — an
    `exists()` check followed by a write is the same race, just shorter.
    """
    try:
        fd = os.open(sub_path(uuid), os.O_WRONLY | os.O_TRUNC)
    except FileNotFoundError:
        log(f"{uuid[:8]} unsubscribed during this tick — not resurrecting it")
        return False
    with os.fdopen(fd, "w") as fh:
        fh.write(json.dumps(rec, indent=1))
    return True


def boot_after_for(s):
    """This subscriber's boot window RIGHT NOW, and why.

    ⛔ The override must EXPIRE on its own. A session that asked for 50 minutes,
    then died mid-wait, must not keep that window forever — the next session to
    inherit the row would be watched far too loosely. So the deferral carries a
    wall-clock deadline and the default resumes the moment it passes, with no
    action required from a session that may no longer exist to take one."""
    secs = s.get("boot_after_secs")
    until = s.get("boot_after_until", 0)
    if not secs:
        return BOOT_AFTER_SECS, ""
    if time.time() >= until:
        return BOOT_AFTER_SECS, "deferral-expired"
    return int(secs), s.get("boot_after_note") or "deferred"


def cmd_defer(args):
    """Ask for a longer boot window while waiting on something long.

    The caller is the only one who knows it is about to block for 40 minutes on a
    test suite; the watcher cannot see that (a waiting session and a stalled one
    are identical from outside). So the session declares it, for that wait only."""
    uuid = (args.row or "").rstrip("/").split("/")[-1] or own_uuid()
    p = sub_path(uuid)
    if not p.exists():
        log(f"{uuid} is not subscribed — nothing to defer")
        return 2
    s = json.loads(p.read_text())

    if args.clear:
        for k in ("boot_after_secs", "boot_after_until", "boot_after_note"):
            s.pop(k, None)
        p.write_text(json.dumps(s, indent=1))
        log(f"deferral cleared for {uuid[:8]} — back to the {BOOT_AFTER_SECS}s default")
        return 0

    asked = int(args.secs)
    secs = max(MIN_BOOT_AFTER_SECS, min(asked, MAX_BOOT_AFTER_SECS))
    if secs != asked:
        log(f"⚠ clamped {asked}s to {secs}s — the {MAX_BOOT_AFTER_SECS}s ceiling is the "
            f"prompt-cache/billing limit, not a preference")
    # The window outlives the wait by one boot interval, so a job that overruns
    # slightly is not booted the instant it goes long.
    s["boot_after_secs"] = secs
    s["boot_after_until"] = time.time() + secs + DEFAULT_INTERVAL
    s["boot_after_note"] = args.note or "long wait"
    p.write_text(json.dumps(s, indent=1))

    back = json.loads(p.read_text())
    ok = back.get("boot_after_secs") == secs
    log(f"defer {uuid[:8]}: boot after {secs}s ({secs/60:.0f} min) — {s['boot_after_note']}")
    log(f"read-back: {'present' if ok else '⛔ ABSENT — deferral did not land'}")
    return 0 if ok else 1


def cmd_unsubscribe(args):
    """⛔ "UNSUBSCRIBE WHEN THE WORK IS DONE" IS WRONG FOR A MONITOR.

    That instruction is right for work with a TERMINAL STATE (a build, a review,
    a migration) and wrong for a watch, where "done" is never true while the
    thing being watched is still live. So "am I done?" is the wrong question to
    hand such a session, and ANY agent asked it eventually answers yes — at the
    moment its task list happens to look empty.

    Measured 2026-08-10 (reported by a sibling campaign): a relay row armed the
    booter, finished its task, and unsubscribed ITSELF at 00:40:43 following the
    contract verbatim. At 02:33 the thing it was watching died. At 09:15 the
    market opened. 7h43m of blindness, ended only when the owner hand-booted it.

    ⚠ The fix is deliberately NOT a better instruction — a rule saying "do not
    unsubscribe" is the same class of object that just failed. The refusal lives
    HERE, at the point where the mistake is actually made.
    """
    uuid = (args.row or "").rstrip("/").split("/")[-1] or own_uuid()
    p = sub_path(uuid)
    if p.exists():
        try:
            rec = json.loads(p.read_text())
        except Exception:
            rec = {}
        if rec.get("kind") == "monitor" and not getattr(args, "force", False):
            log(f"⛔ {uuid[:8]} is a MONITOR subscription — refusing to unsubscribe.")
            log("   A monitor has no 'done': the thing it watches is still live, and")
            log("   'my task list looks empty' is not the same question. If you really")
            log("   mean to stop watching, say so explicitly:")
            log(f"   ygg-booter.py unsubscribe --row {uuid} --force")
            return 3
        p.unlink()
        log(f"unsubscribed {uuid}")
    else:
        log(f"{uuid} was not subscribed")
    if not load_subs():
        log("no subscribers left — the watcher will retire itself on its next tick")
    return 0


def fleet_state(args):
    """Everything a surface needs to render THIS host's booter, in one read.

    ⭐ This is the shape `yggtopo` renders and drives. It exists because "is the
    booter what is hurting me right now" was unanswerable without a shell on each
    host — and an answer that requires a shell is not an answer a human has."""
    d = disarm_state()
    rl = rate_limit_hold()
    alive = watcher_alive()
    now = time.time()
    subs = []
    for s in load_subs():
        win, why = boot_after_for(s)
        # ⭐ "WHEN IS THIS ROW DUE" IS THE EXPENSIVE HALF, SO IT IS OPT-IN.
        #    Answering it means classifying the row — a live row-list call plus
        #    a transcript read, the same work a tick does — which is fine when a
        #    human is looking at the pane and ruinous on a refresh loop. Without
        #    `--due` this stays a cheap read of local files.
        # ⛔ AND A FAILED LOOK IS REPORTED AS A FAILED LOOK. `due_in_s: null`
        #    with a state of UNREACHABLE says "I could not see"; rendering that
        #    as "not due" would be a verdict we never earned.
        due = {}
        if getattr(args, "due", False):
            try:
                rhost = BB.row_host(s.get("row") or "", s.get("host") or args.host)
                if rhost and rhost == this_host():
                    rhost = None
                c = BB.classify(s["uuid"], rhost)
                # ⛔ "DUE" IS A COUNTDOWN, NOT A VERDICT ALREADY REACHED. Only
                #    answering once a row is already IDLE makes the number
                #    useless for the one thing a human wants it for — deciding
                #    whether to defer a row BEFORE it is kicked. JUST_ENDED and
                #    IDLE are the same fact at two ages (the turn has ended),
                #    and the countdown is meaningful across both.
                # ⛔ A MID-TURN ROW HAS NO DUE TIME AT ALL, and must not be
                #    given one: the window is measured from a turn ending, and
                #    this row's has not.
                due = {
                    "state": c["state"],
                    "idle_s": round(c["age"]),
                    "due_in_s": (max(0, round(win - c["age"]))
                                 if c["state"] in ("IDLE", "JUST_ENDED") else None),
                }
            except Exception as e:
                due = {"state": "UNREACHABLE", "idle_s": None, "due_in_s": None,
                       "why": str(e)[:120]}
        subs.append({
            **due,
            "uuid": s["uuid"],
            "row": s.get("row"),
            "campaign": s.get("campaign") or "",
            "kind": s.get("kind") or "task",
            "note": s.get("note") or "",
            "age_h": round((now - s["subscribed_at"]) / 3600, 2),
            "max_hours": s.get("max_hours"),
            "boots": s.get("boots", 0),
            "escalated": bool(s.get("escalated")),
            # WHEN this row is next at risk of a boot, which is the thing a human
            # actually wants ("who is due") — not the raw window it was set to.
            "boot_window_secs": win,
            "boot_window_why": why,
        })
    beat = None
    if HEARTBEAT.exists():
        try:
            beat = json.loads(HEARTBEAT.read_text())
        except Exception:
            beat = None
    silent_s = _watcher_silence_secs(beat)
    return {
        "host": this_host(),
        "now": now,
        "armed": d is None,
        "disarm": None if not d else {
            "since": d.get("since"), "until": d.get("until"),
            "forever": not d.get("until"), "note": d.get("note") or "",
            "by": d.get("by") or "",
        },
        "watcher": {
            "pid": alive,
            "alive": bool(alive),
            "heartbeat_age_s": None if not beat else round(now - beat.get("ts", 0)),
            "log_silent_s": silent_s,
            # ⛔ ALIVE IS NOT AUDIBLE — a surface that draws a green dot for a
            #    MUTE watcher republishes the exact lie `status` was fixed to stop.
            "mute": bool(alive) and (silent_s is None
                                     or silent_s > max(3 * args.interval, 900)),
        },
        "rate_limit_hold": None if not rl else {
            "since": rl.get("since"), "until": rl.get("until"),
            "seen_on": rl.get("seen_on"), "tail": rl.get("tail"),
            "secs_left": round(rl["until"] - now),
        },
        "subscribers": subs,
    }


RETIRED_LEDGER = STATE / "booter-retired.tsv"


def retired_rows():
    """⛔ ROWS DECIDED TO BE DEAD — a THIRD state, distinct from both others.

    `coverage` first shipped with three buckets and one of them was doing two
    jobs: **UNKNOWN meant both "nobody has decided" and "decided: this row is a
    corpse."** Every coverage run therefore reported settled rows as undecided,
    and the next reader re-derived their liveness from transcripts by hand — the
    exact absent-vs-refused ambiguity `disarmed_rows()` exists to kill,
    reappearing one level up. Reported by the orchestrator 2026-08-14 with
    process-level evidence for seven rows.

    ⛔ **It cannot live in the opt-out ledger, and the reason is that ledger's own
    docstring: *"this row asked not to be watched."* A CORPSE NEVER ASKED.**
    Folding a death into a record of consent misdescribes both, so this is its
    own file and "asked" stays pure.

    ⚠ Arming one of these is not dangerous, it is merely useless — a boot types
    into a process that no longer exists. So `subscribe` WARNS rather than
    refuses: the danger direction is typing into a live person, not a dead pid,
    and a refusal here would be a floor with nothing under it.

    Same append-only shape as the other ledgers, latest record wins, because the
    decision and who made it are the durable part.
    """
    out = {}
    try:
        for line in RETIRED_LEDGER.read_text().splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) < 2:
                continue
            uuid = parts[1].strip()
            if uuid:
                who = parts[2].strip() if len(parts) > 2 else "unrecorded"
                why = parts[3].strip() if len(parts) > 3 else "no evidence recorded"
                out[uuid] = f"{why} (decided by {who})"
    except FileNotFoundError:
        return {}
    except Exception:
        return None
    return out


def cmd_retire(args):
    """Record that a row is DEAD, with who decided and on what evidence."""
    uuid = (args.row or "").rstrip("/").split("/")[-1]
    if not uuid:
        log("retire needs --row <uuid>")
        return 2
    if not args.evidence:
        log("⛔ retire needs --evidence: what makes you say this row is dead?")
        log("   e.g. 'cut mid tool_use 16:55:04Z, no process carries its uuid'")
        log("   ⚠ A decision with no evidence is the thing this ledger replaces.")
        return 2
    blocked = never_arm()
    if blocked is None:
        log(f"⛔ REFUSING — {NEVERARM} is UNREADABLE, so I cannot tell whether a")
        log("   person attends this row, and declaring someone's live row dead is")
        log("   how it stops being watched by anything at all.")
        return 4
    if uuid in blocked:
        log(f"⛔ REFUSING — {uuid[:8]} is on never-arm ({blocked[uuid]}).")
        log("   A row a person attends is not yours to declare dead.")
        return 3
    try:
        STATE.mkdir(parents=True, exist_ok=True)
        with RETIRED_LEDGER.open("a") as fh:
            fh.write("%s\t%s\t%s\t%s\n" % (
                time.strftime("%Y-%m-%dT%H:%M:%S%z"), uuid,
                args.decided_by or "unrecorded", args.evidence))
    except Exception as exc:
        log(f"⛔ could not record: {exc}")
        return 1
    back = retired_rows()
    log(f"recorded {uuid[:8]} as decided-dead")
    log(f"read-back: {'present' if back and uuid in back else '⛔ ABSENT — it did not land'}")
    # A dead row has no business holding a live subscription.
    if sub_path(uuid).exists():
        sub_path(uuid).unlink(missing_ok=True)
        log(f"  and dropped its subscription — a boot would have typed at a corpse")
    return 0


def _drop_sub(uuid, why):
    """Remove a booter subscription that a just-recorded decision contradicts."""
    if not sub_path(uuid).exists():
        return
    sub_path(uuid).unlink(missing_ok=True)
    log(f"  and dropped its booter subscription — {why}")


def cmd_never_arm(args):
    """⛔ RECORD THAT A PERSON ATTENDS THIS ROW. The answer is always no.

    ⭐ **This verb exists because `coverage` names three decisions and the CLI
    only offered instruments for two of them.** A triage session could
    `subscribe` a delegate and `retire` a corpse, but the third — *a person types
    here* — meant hand-appending to a TSV. So the decision that matters most was
    the one the tool made hardest, and hand-assembling it two dozen times is how
    a wrong uuid or a lost tab reaches the list that stops this watchdog typing
    into someone's composer.

    ⚠ Recording is deliberately one-way here: there is no `--remove`. Taking a
    row OFF this list is a decision about a person's keyboard and it should cost
    an explicit, deliberate edit of the file, not a flag someone reaches for
    while tidying."""
    uuid = (args.row or "").rstrip("/").split("/")[-1]
    if not uuid:
        log("never-arm needs --row <uuid>")
        return 2
    if not args.note:
        log("⛔ never-arm needs --note: who attends this row, and how you know.")
        log("   e.g. --note 'owner types here; first turn is a person, not a brief'")
        return 2
    blocked = never_arm()
    if blocked is None:
        log(f"⛔ {NEVERARM} is UNREADABLE — refusing to append to it.")
        log("   Appending to a file this parser cannot make sense of would bury")
        log("   the damage under a record that looks like protection.")
        return 4
    if uuid in blocked:
        log(f"{uuid[:8]} is already on never-arm ({blocked[uuid] or 'no reason recorded'})")
        _drop_sub(uuid, "an attended row must never carry one")
        return 0
    dead = retired_rows()
    if dead and uuid in dead:
        log(f"⚠ {uuid[:8]} was previously recorded DEAD ({dead[uuid]}).")
        log("   Listing it as attended anyway — a corpse and a person are")
        log("   different claims, and the attended one is the safe way to be wrong.")
    note = args.note.strip().replace("\t", " ")
    if args.decided_by:
        note = f"{note} (recorded by {args.decided_by})"
    try:
        STATE.mkdir(parents=True, exist_ok=True)
        with NEVERARM.open("a") as fh:
            fh.write(f"{uuid}\t{note}\n")
    except Exception as exc:
        log(f"⛔ could not record: {exc}")
        return 1
    back = never_arm()
    if back is None:
        log(f"⛔ READ-BACK FAILED — {NEVERARM} no longer parses after that write.")
        return 1
    log(f"recorded {uuid[:8]} as human-attended")
    log(f"read-back: {'present' if uuid in back else '⛔ ABSENT — it did not land'}")
    _drop_sub(uuid, "this watchdog types into what it wakes")
    return 0 if uuid in back else 1


def cmd_optout(args):
    """Record that a row is not to be watched, WITHOUT claiming a person attends it.

    The weaker of the two refusals, and the distinction is the point: never-arm
    says *a person types here*; this says *this row has nothing to continue*.
    A delegate that finished its work and is waiting on a decision belongs here —
    arming it would resurrect a row that stopped on purpose, which is the
    write-back-recreates-a-deletion shape this fleet has already paid for once.

    ⚠ `ygg-claim.sh --no-booter` writes the same ledger from inside a row that is
    standing itself down. This is the same record made from outside, which is the
    only way it can be made ABOUT a row whose session will never run again."""
    uuid = (args.row or "").rstrip("/").split("/")[-1]
    if not uuid:
        log("optout needs --row <uuid>")
        return 2
    if not args.note:
        log("⛔ optout needs --note: why this row is not to be watched.")
        log("   e.g. --note 'work complete, waiting on a decision; nothing to continue'")
        return 2
    if args.note.strip().startswith(REARM_MARK):
        log(f"⛔ a reason may not begin with {REARM_MARK} — that prefix is how the")
        log("   ledger records a RE-ARM, so such a line would read as the opposite")
        log("   of what you meant.")
        return 2
    blocked = never_arm()
    if blocked is None:
        log(f"⛔ {NEVERARM} is UNREADABLE — refusing to record an opt-out while the")
        log("   stronger list cannot be screened; the two are read together.")
        return 4
    if uuid in blocked:
        log(f"{uuid[:8]} is already NEVER-ARM ({blocked[uuid]}), which is stronger.")
        log("   Nothing recorded: an opt-out beneath a never-arm reads as though")
        log("   the row could ask to be watched again, and it cannot.")
        return 0
    existing = disarmed_rows()
    if existing is None:
        log(f"⛔ {DISARMED_LEDGER} is UNREADABLE — refusing to append to it.")
        return 4
    note = args.note.strip().replace("\t", " ")
    who = args.decided_by or own_uuid() or "shell"
    try:
        STATE.mkdir(parents=True, exist_ok=True)
        with DISARMED_LEDGER.open("a") as fh:
            fh.write("%s\t%s\t%s\t%s\n" % (
                time.strftime("%Y-%m-%dT%H:%M:%S%z"), uuid, who, note))
    except Exception as exc:
        log(f"⛔ could not record: {exc}")
        return 1
    back = disarmed_rows()
    if back is None:
        log(f"⛔ READ-BACK FAILED — {DISARMED_LEDGER} no longer parses after that write.")
        return 1
    log(f"recorded {uuid[:8]} as opted out of the booter")
    log(f"read-back: {'present' if uuid in back else '⛔ ABSENT — it did not land'}")
    _drop_sub(uuid, "a row that opted out must not carry one")
    return 0 if uuid in back else 1


def cmd_coverage(args):
    """⭐ WHICH LIVE ROWS ARE WATCHED, AND — THE POINT — WHICH NOBODY HAS DECIDED.

    ⛔ THIS DELIBERATELY DOES NOT ARM ANYTHING, and that is the design, not a
    missing feature. The obvious next step, *enumerate live rows and arm the
    unwatched ones*, was measured and is not implementable safely: across all 31
    fields of the row listing, a human-attended row and an unattended delegate
    are identical — same `kind`, same `icon_kind`, same tenancy, same presence.
    The only separators are free text, identity, a folding flag, transient
    `busy`, and a seat number an agent types about itself. **No field says a
    person types here.**

    ⇒ So `never-arm.tsv` is not a backstop beneath a filter, it IS the filter,
    and auto-arming would be fail-open: any attended row nobody has hand-listed
    is armable by construction, and the remedy for being armed is being typed
    over. The whole watchdog's function is to TYPE INTO what it wakes.

    ⇒ What IS decidable is the BOOKKEEPING, and that is what the original
    complaint was really about — 42 of 47 rows ran unwatched and nothing said
    so. This turns an undecidable arming question into a decidable reporting
    one: every live row lands in exactly one bucket, and the UNKNOWN bucket is
    the one a person acts on.

    ⚠ It is also why auto-arm stays unbuilt rather than merely unfinished: an
    attestation-driven re-arm would resurrect rows that deliberately
    unsubscribed when their work finished, which is the write-back-recreates-a
    -deletion shape this fleet has already paid for once.
    """
    subs = {s["uuid"]: s for s in load_subs()}
    blocked = never_arm()
    if blocked is None:
        # ⛔ Refusing to report, for the same reason the empty row listing below
        #    refuses: this report's whole output is a to-arm list, and with the
        #    attended list unreadable every attended row would appear in the
        #    UNKNOWN bucket, whose printed remedy is `subscribe`. A report that
        #    cannot screen is not a partial answer, it is a trap.
        log(f"⛔ {NEVERARM} is UNREADABLE — refusing to report.")
        log("   Attended rows would land in UNKNOWN, and UNKNOWN's remedy is to")
        log("   arm. Fix the file, then ask again.")
        return 2
    opted_out = disarmed_rows()
    ledger_readable = opted_out is not None
    opted_out = opted_out or {}
    dead = retired_rows()
    dead_readable = dead is not None
    dead = dead or {}

    d = BB.ygg(args.host, "server", "app", "rows")
    rows = (d.get("data", {}) or {}).get("rows", []) or []
    if not rows:
        log("⛔ the row listing came back EMPTY — that is 'I could not ask', not "
            "'there are no rows'. Nothing below would mean anything; refusing to "
            "report.")
        return 2

    buckets = {"watched": [], "never_arm": [], "opted_out": [], "retired": [], "unknown": []}
    # ⛔ COUNT SESSIONS, NOT ROWS. The listing can render ONE session as SEVERAL
    #    rows — measured 2026-08-14: one key appearing twice, at depth 2 and
    #    depth 4, both `live_rail` and `live_member`. Counting rows made a
    #    seven-entry ledger report as eight, which is the kind of off-by-one that
    #    gets explained away rather than chased. The duplication is a sidebar
    #    defect and belongs to whoever owns row identity; here it is reported and
    #    then collapsed, never silently summed.
    seen = set()
    duplicated = []
    for r in rows:
        if r.get("kind") != "Session":
            continue
        path = r.get("path") or ""
        uuid = path.rstrip("/").split("/")[-1]
        if not uuid or "://" not in path:
            continue
        if uuid in seen:
            duplicated.append(f"{uuid[:8]} (depth {r.get('depth')})")
            continue
        seen.add(uuid)
        label = (r.get("outline_prefix") or "").strip()
        who = f"{uuid[:8]}{(' [' + label + ']') if label else ''}"
        if uuid in blocked:
            buckets["never_arm"].append(f"{who} — {blocked[uuid]}")
        elif uuid in subs:
            buckets["watched"].append(f"{who} — campaign={subs[uuid].get('campaign') or '-'}")
        elif uuid in opted_out:
            buckets["opted_out"].append(f"{who} — {opted_out[uuid]}")
        elif uuid in dead:
            buckets["retired"].append(f"{who} — {dead[uuid]}")
        else:
            buckets["unknown"].append(who)

    if duplicated:
        log(f"⚠ {len(duplicated)} row(s) render a session that is ALREADY LISTED: "
            f"{', '.join(duplicated)}")
        log("   Counted once here. A session appearing twice at different depths "
            "is a row-identity defect, not a booter one — report it to whoever "
            "owns the sidebar rather than tolerating the double count.")
    if not dead_readable:
        log("⚠ the retired ledger is UNREADABLE — dead rows will appear as UNKNOWN.")
    if not ledger_readable:
        log("⚠ the opt-out ledger is UNREADABLE — rows that opted out will appear "
            "as UNKNOWN below. Treat this report as incomplete.")
    log(f"watched   {len(buckets['watched']):>3}")
    log(f"never-arm {len(buckets['never_arm']):>3}  (a person attends these; the answer is always no)")
    log(f"opted-out {len(buckets['opted_out']):>3}  (asked not to be watched, with a reason)")
    log(f"retired   {len(buckets['retired']):>3}  (decided dead, with evidence — a corpse never asked)")
    log(f"UNKNOWN   {len(buckets['unknown']):>3}  ⭐ nobody has decided about these")
    for name in ("unknown", "retired", "opted_out", "never_arm", "watched"):
        if not buckets[name] or (name != "unknown" and not args.verbose):
            continue
        log(f"  -- {name} --")
        for line in buckets[name]:
            log(f"     {line}")
    if buckets["unknown"]:
        log("⇒ For each UNKNOWN row, decide and RECORD it — there is no probe that "
            "can decide for you:")
        log("     an unattended delegate  → ygg-booter.py subscribe --row <uuid>")
        log("     a row a person types in → add it to never-arm.tsv")
        log("     a row that is DEAD      → ygg-booter.py retire --row <uuid> --evidence '…'")
        log("   ⛔ Do not bulk-arm this list. That is the fail-open move this "
            "report exists to replace.")
    return 0


def cmd_list(args):
    if args.json:
        print(json.dumps(fleet_state(args), indent=1))
        return 0
    subs = load_subs()
    d = disarm_state()
    if d:
        left = "until re-armed" if not d.get("until") else \
            f"{(d['until'] - time.time()) / 60:.0f}m left"
        log(f"⛔ DISARMED ({left}) — {d.get('note') or 'no reason given'}")
    rl = rate_limit_hold()
    if rl:
        log(f"⏸ QUOTA HOLD {(rl['until'] - time.time()) / 60:.0f}m left "
            f"(429 seen on {rl['seen_on'][:8]})")
    # ⭐ Name the directory this reads. A sibling campaign lost minutes concluding
    #   "no subscription file exists" while looking in the relay root — which also
    #   holds per-uuid .json files of a DIFFERENT kind, so the wrong directory
    #   looks like the right one with the answer missing.
    if not subs:
        log(f"no subscribers (reading {SUBS})")
        return 0
    for s in subs:
        age_h = (time.time() - s["subscribed_at"]) / 3600
        log(f"{s['uuid'][:8]}  {s.get('campaign') or '-':<12} "
            f"age={age_h:4.1f}h boots={s['boots']} {s['row']}")
    log(f"{len(subs)} subscription(s) in {SUBS}")
    return 0


def _run(host, argv, stdin_text):
    """Run a yggterm CLI verb with text on stdin, wherever the row lives."""
    if host == this_host():
        cmd = [str(Path.home() / ".local" / "bin" / "yggterm-headless"), *argv]
        return subprocess.run(cmd, input=stdin_text, capture_output=True,
                              text=True, timeout=180)
    joined = " ".join(f"'{a}'" for a in argv)
    return subprocess.run(["ssh", host, f"$HOME/.yggterm/bin/yggterm {joined}"],
                          input=stdin_text, capture_output=True, text=True, timeout=180)


def _field(out, name):
    try:
        d = json.loads(out[out.find("{"):])
    except Exception:
        return None
    return (d.get("data") or d).get(name)


def boot(host, row, dry):
    """Send exactly one boot. Never to a mid-turn row — the classifier decides that.

    ⛔⛔ TWO WAYS TO GET THIS WRONG, both measured here on 2026-08-09 rather than
    reasoned about, and the first was in the first draft of this very function:

    1. **`"submitted" in stdout` IS TRUE FOR `"submitted": false`.** The verb was
       reporting an honest failure and the substring test read it as success — so
       the booter logged a delivered boot for a boot that never arrived. Read the
       FIELD'S VALUE, never the field's presence. (`ygg-babysit.py` had the same
       bug; fixed there too.)
    2. **`terminal submit` drives the GUI's mounted terminal host, so a row with
       no host waits out its 30s deadline and answers `submitted:false`.** That
       is correct of the composer and useless to a watchdog, whose whole job is
       rows nobody is looking at. `server terminal write` addresses the PTY —
       the layer that exists whether or not anything is mounted — and delivered
       on the same row in the same minute.

    3. ⛔⛔ **THE ENTER IS A SEPARATE WRITE OF `\\r`, AND THIS FUNCTION SENT ONE
       CONCATENATED `\\n`.** reported 2026-08-09 ~19:10, watching it happen:
       *"I just saw `continue, the booter booted` and an empty line. The enter key
       did not send the prompt."* Two independent mistakes in one line:

       - **`\\n` is not Enter.** An agent CLI runs its tty in RAW mode and reads
         bytes itself, so Enter is **CR (`\\r`, 0x0D)**; a bare LF is inserted as a
         literal newline — the empty line he saw. A plain shell forgives this
         (canonical mode's `ICRNL` translates for you), which is exactly why it
         survived: it works everywhere except the row type this tool exists for.
       - **And `text + "\\r"` in ONE write does not submit either.** yggterm's own
         `server app terminal submit` (`shell.rs:76405`) writes the TEXT, sleeps
         80 ms, then writes `"\\r"` as a DISCRETE second write, and says why:
         *"codex treats a `\\r` concatenated with the text in one write as a pasted
         newline (composer content), not a submit (verified live 2026-06-04)."*
         ⇒ my first fix here was `BOOT_TEXT + "\\r"` in one write, which would
         have reproduced the bug in a new costume. **The product already knew;
         the watchdog had its own private encoding of "press Enter".**

       Measured cost of the old `\\n`: `booter.log` shows `BOOT#1..#3:pty-write`
       then `ESCALATE ... did not wake after 3 boots (21 min idle)`, twice over,
       while the session sat idle for 40 minutes with every boot logged delivered.

    ⇒ **PTY FIRST, composer as the fallback** — the reverse of the original order,
      and the measurement is the reason. Every boot in the log took the fallback:
      `pty-write`, 5 for 5. A composer attempt on an unmounted row is not a cheap
      first try, it is a 30-SECOND STALL in front of the thing that works, and an
      "exception" branch with a 100% hit rate means the code above it is
      decoration. [[finding-a-deadline-shorter-than-its-release-condition]]
      One line only — a multi-line send is one Enter per line and the rest queue.

    ⚠ **KNOWN HAZARD, unchanged by this fix and filed rather than fixed here:**
      the write APPENDS to whatever is in the composer, so a half-typed draft is
      submitted with the boot text glued onto it. The daemon knows
      (`session_has_pending_input_draft`); the booter does not ask."""
    if dry:
        log(f"DRY-RUN would boot {row}")
        return "dry-run"
    outcome = _pty_type_and_enter(host, row)
    if outcome:
        return outcome
    r = _run(host, ["server", "app", "terminal", "submit", row, "--stdin"], BOOT_TEXT)
    if _field(r.stdout or "", "submitted") is True:
        return "submit"
    return ""


def _pty_type_and_enter(host, row):
    """Type the boot text, pause, then press Enter — as TWO writes.

    The 80 ms mirrors `shell.rs`'s own submit path exactly; see `boot`'s §3 for
    why a concatenated Enter reads as pasted content rather than a submit.

    ⛔ `--refuse-if-draft` is not optional politeness. A PTY write APPENDS, so if
    the owner half-typed a sentence into this row and walked away — which is
    precisely the shape of a row a watchdog calls idle — an unguarded boot glues
    `continue, the booter booted` onto the end of HIS sentence and submits the
    pair. The guard is evaluated by the daemon that OWNS the PTY (the only one
    that can see a draft) and is therefore TOCTOU-free.
    ⚠ A pre-3.0.83 owner ignores the flag, so acceptance is not proof the guard
    ran. That is the honest limit and it is why the return distinguishes a
    refusal rather than folding it into failure."""
    typed = _run(host, ["server", "terminal", "write", row, "--stdin",
                        "--refuse-if-draft"], BOOT_TEXT)
    if _field(typed.stdout or "", "refused_for_draft") is True:
        return "refused-draft"
    if _field(typed.stdout or "", "accepted") is not True:
        return ""
    time.sleep(0.08)
    enter = _run(host, ["server", "terminal", "write", row, "--stdin"], "\r")
    return "pty-write" if _field(enter.stdout or "", "accepted") is True else ""


def escalate(host, row, why):
    """Tell a human, and address the card AT THE ROW THAT NEEDS THEM.

    ⛔ Not at whoever noticed. A card pointing at the observer works, and takes
    them to the wrong place — worse than inert, because it looks like it
    functioned."""
    log(f"ESCALATE {row}: {why}")
    args = ["server", "app", "notify", "booter: a session needs you", why,
            "--tone", "warning"]
    target = resolve(host, row)
    if target:
        args += ["--session", target]
    else:
        log("  ⚠ no row path resolves — sending an UNTARGETED card rather than an inert one")
    ygg(host, *args)


def tick(args):
    subs = load_subs()
    if not subs:
        return 0
    # ⛔ THE OFF SWITCH IS CHECKED FIRST AND SUSPENDS EVERYTHING — not just boots.
    #    "Off" has to mean off, including the housekeeping that unsubscribes a
    #    row and the escalations that page a human. A switch with quiet
    #    exceptions is a switch nobody trusts, and an untrusted safety net gets
    #    dismantled instead of disarmed.
    d = disarm_state()
    if d:
        left = ("until re-armed" if not d.get("until")
                else f"{(d['until'] - time.time()) / 60:.0f}m left")
        log(f"DISARMED ({left}) — {len(subs)} subscriptions held, no boots. "
            f"{d.get('note') or ''}".rstrip())
        return 0
    # The quota hold from a previous tick. Re-read per tick, never cached: a hold
    # that outlived its window must not keep a healthy fleet asleep.
    rl = rate_limit_hold()
    # ⛔⛔ ENFORCE never-arm AT THE TICK, NOT ONLY AT `subscribe`.
    #
    #    The refusal in `cmd_subscribe` lives in THIS SCRIPT, and this script is
    #    copied into every checkout on the machine — a lane claims with its own
    #    copy, which is routinely many commits behind. Measured: the guard was
    #    present in 1 checkout of 11, and the exposed end was the one the hazard
    #    actually runs through, because `subscribe` is called by a LANE from its
    #    own tree while the tick runs from exactly one.
    #
    #    ⇒ A guard in code that exists in eleven versions is eleven guards. The
    #    STATE DIR is shared by every checkout and the tick is a single process,
    #    so enforcement belongs here — and this is also the point of HARM: the
    #    tick is the only thing that types. A stale script can still create the
    #    subscription; it can no longer cause anyone to be typed over, and the
    #    bad record is purged on sight rather than left to be re-found.
    blocked = never_arm()
    if blocked is None:
        # ⛔⛔ THE TICK IS THE ONLY THING THAT TYPES, SO IT IS WHERE A BLIND
        #    SCREEN MUST STOP. Booting with the attended list unreadable is the
        #    fail-open move in the place it does the damage: this tick's remedy
        #    is to write into somebody's composer. A watchdog that cannot see
        #    who it must never wake does not get to wake anyone.
        #    ⚠ Yes, an unreadable list therefore disables the booter until it is
        #    fixed. That is the safe direction, and it is not silent: the watch
        #    loop keeps heartbeating and this refusal is logged on every tick,
        #    so the outage is visible in the same place a boot would have been.
        log(f"⛔ {NEVERARM} is UNREADABLE — BOOTING NOTHING this tick.")
        log("   An unreadable attended-row list is not an empty one, and the")
        log("   remedy this tick would apply is typing into a live person.")
        return 0
    if blocked:
        for s in list(subs):
            if s["uuid"] in blocked:
                log(f"⛔ {s['uuid'][:8]} is NEVER-ARM ({blocked[s['uuid']]}) yet was "
                    f"SUBSCRIBED — purging it now, not booting it.")
                log("   Something armed a human-attended row; it was almost "
                    "certainly a pre-guard copy of this script in another checkout.")
                # Dropped from THIS tick's working set either way — a dry run must
                # not mutate, but it must not pretend it would boot this row.
                if not args.dry_run:
                    sub_path(s["uuid"]).unlink(missing_ok=True)
                subs.remove(s)
    # ⛔ AND ENFORCE THE OPT-OUT LEDGER AT THE TICK TOO, for the same reason the
    #    never-arm list is enforced here: `subscribe` runs from a LANE's own
    #    checkout, which is routinely many commits behind, while the tick runs
    #    from exactly one. A guard that lives only in `subscribe` is as many
    #    guards as there are copies of this script.
    #    ⚠ Weaker remedy than never-arm, deliberately: a row that opted out is
    #    unsubscribed, not treated as a person's keyboard. The re-arm path
    #    (`subscribe --rearm`) is what puts it back, and it leaves a record.
    opted_out = disarmed_rows()
    if opted_out is None:
        log(f"⚠ {DISARMED_LEDGER} is UNREADABLE — opt-outs NOT verified this tick. "
            f"Treating every subscription as armable is the unsafe direction, so "
            f"nothing is purged, but do not read a quiet tick as consent.")
    elif opted_out:
        for s in list(subs):
            if s["uuid"] in opted_out:
                log(f"⛔ {s['uuid'][:8]} OPTED OUT of the booter "
                    f"({opted_out[s['uuid']]}) yet was SUBSCRIBED — unsubscribing.")
                log("   Most likely a re-claim: relabelling a row through "
                    "ygg-claim.sh re-subscribes it, so standing a row down and "
                    "then renaming it silently armed it again.")
                if not args.dry_run:
                    sub_path(s["uuid"]).unlink(missing_ok=True)
                subs.remove(s)
    rc = 0
    for s in subs:
        uuid, row, host = s["uuid"], s["row"], s.get("host", args.host)
        age_h = (time.time() - s["subscribed_at"]) / 3600
        if s.get("max_hours") and age_h > s["max_hours"]:
            log(f"{uuid[:8]} EXPIRED after {age_h:.1f}h — unsubscribing")
            sub_path(uuid).unlink(missing_ok=True)
            continue
        # ⛔ ROW LIST FIRST. A retired row's transcript is frozen mid-turn forever
        #    and is indistinguishable from a live wedge; booting a corpse is a
        #    watchdog barking at a grave.
        # ⛔⛔ BUT TRI-STATE, AND CONFIRMED. `None` is "I could not ask" and must
        #    change nothing; `False` is counted rather than acted on. Reading
        #    either as "retired" deleted nine live subscriptions in six seconds
        #    on 2026-08-13 — see `row_presence`.
        presence = row_presence(host, uuid)
        if presence is False:
            s["gone_sightings"] = s.get("gone_sightings", 0) + 1
            if s["gone_sightings"] >= GONE_SIGHTINGS:
                log(f"{uuid[:8]} GONE — absent from {GONE_SIGHTINGS} consecutive "
                    f"row listings, unsubscribing")
                sub_path(uuid).unlink(missing_ok=True)
                continue
            log(f"{uuid[:8]} absent from the row list "
                f"({s['gone_sightings']}/{GONE_SIGHTINGS}) — waiting for confirmation")
            if not args.dry_run:
                update_sub(uuid, s)
            continue
        if presence is None:
            # ⛔ Say "I could not look", never "it is not there" — and change
            #    NOTHING, including the counter: no information is not evidence
            #    in either direction.
            log(f"{'CANNOT-SEE':<14} {'':>6}  {'-':<12} {uuid[:8]}  "
                f"the row list did not answer; nothing decided")
            continue
        s["gone_sightings"] = 0
        rhost = BB.row_host(row, host)
        if rhost and rhost == this_host():
            rhost = None
        c = BB.classify(uuid, rhost)
        try:
            size = os.path.getsize(c["path"]) if (c["path"] and not rhost) else 0
        except OSError:
            size = 0
        # ⛔ PROGRESS, NOT BYTES. A refused turn grows the file, so `size >
        #    last_size` reset the stall counter on a dead session forever. See
        #    BB.progress_marks.
        marks = BB.progress_marks(c["path"]) if (c["path"] and not rhost) else 0
        grew = marks > s.get("last_marks", 0)
        s["last_marks"] = marks
        s["last_size"] = size
        action = "-"
        state = c["state"]

        if state == "CONTEXT_DEAD":
            # ⛔ THE ONE STATE A BOOT CANNOT FIX. Booting a context-exhausted
            #    session is not merely useless -- it is guaranteed to fail
            #    forever, because every prompt is refused before the agent ever
            #    runs. Say the TRUE thing ("unrecoverable, relay it") instead of
            #    "did not wake after 3 boots", escalate ONCE, and stop watching:
            #    a watchdog that keeps barking at a grave taught the owner to
            #    ignore it. Measured 2026-08-10: ten hours of it.
            action = "CONTEXT-DEAD"
            rc = max(rc, 4)
            if not s["escalated"]:
                escalate(host, row, f"context exhausted and UNRECOVERABLE — {c['tail']}. "
                                    f"No boot can clear this; relay the campaign to a "
                                    f"fresh session.")
                s["escalated"] = True
            log(f"{uuid[:8]} CONTEXT-DEAD — unsubscribing (a boot cannot fix this)")
            sub_path(uuid).unlink(missing_ok=True)
            continue
        if state == "RATE_LIMITED":
            # ⛔⛔ THE PREMISE BELOW WAS FALSE FOR 7.5 HOURS AND TOOK THE WHOLE
            #    WAKE PLANE DOWN WITH IT. "The account is out of quota, not this
            #    row" is inferred from the row's TAIL — and a DEAD row's tail
            #    never changes. Eight rows had died on quota hours earlier; the
            #    account had long since reset and other sessions were running
            #    normally, but each tick re-read those frozen last words,
            #    re-classified RATE_LIMITED, and re-armed a 30-minute FLEET-WIDE
            #    hold. The mechanism that ends the hold was the one renewing it,
            #    and every instrument still reported "✅ armed".
            #    ⭐ THE CLASS: a STALE ARTEFACT READ AS A LIVE SIGNAL — the same
            #    shape as a transcript's mtime being when a row DIED rather than
            #    when it last worked. A quota tail is evidence about a MOMENT;
            #    treating it as evidence about NOW needs the row to still exist.
            #    ⇒ Ask whether anything is actually running as this row before
            #    holding the entire fleet on its behalf.
            if row_process_absent(uuid):
                log(f"{uuid[:8]} QUOTA-DEAD — no process; its quota tail is history, "
                    f"not a live refusal. Unsubscribing instead of holding the fleet.")
                try:
                    STATE.mkdir(parents=True, exist_ok=True)
                    with RETIRED_LEDGER.open("a") as fh:
                        fh.write("%s\t%s\t%s\t%s\n" % (
                            time.strftime("%Y-%m-%dT%H:%M:%S%z"), uuid,
                            "booter tick (auto)",
                            "classified RATE_LIMITED with no process running as this row: "
                            "a corpse whose last words were a quota message. Retrying it "
                            "re-armed the fleet-wide hold on every tick."))
                except Exception as exc:
                    log(f"   ⚠ could not record the retirement: {exc}")
                sub_path(uuid).unlink(missing_ok=True)
                continue
            # A LIVE row refused on quota is the real thing: hold the FLEET, do
            # not escalate (a human cannot grant quota), and do not unsubscribe
            # — unlike CONTEXT_DEAD this ends by itself, and the row is meant
            # to still be watched when it does.
            rl = note_rate_limit(uuid, c["tail"])
            # ⭐ GIVE THE LAST ATTEMPT BACK. That boot was refused by the API
            #    before the agent ran, so counting it toward MAX_BOOTS would
            #    escalate "did not wake after 3 boots" about a session that was
            #    never actually asked anything. Same argument as `refused-draft`.
            if s.get("boots"):
                s["boots"] -= 1
            action = "RATE-LIMITED"
        elif state in ("WORKING", "JUST_ENDED"):
            s["boots"] = 0                     # progress clears the stall counter
            s["escalated"] = False
        elif state == "UNREACHABLE":
            action = "CANNOT-SEE"              # never a verdict about the row
        elif state == "NO_TRANSCRIPT":
            action = "NO-TRANSCRIPT"
            rc = max(rc, 4)
            if not s["escalated"]:
                escalate(host, row, "subscribed to the booter but never wrote a transcript")
                s["escalated"] = True
        elif state == "STUCK":
            # ⛔ Mid-turn. A boot here races the agent's own input.
            action = "ESCALATE"
            rc = max(rc, 4)
            if not s["escalated"]:
                escalate(host, row, f"mid-turn and untouched for {c['age']/60:.0f} min — "
                                    f"a boot would race its own input")
                s["escalated"] = True
        elif state == "IDLE" and c["age"] >= (boot_after := boot_after_for(s)[0]):
            if grew:
                s["boots"] = 0                 # it worked since last tick
            if rl:
                # ⛔ A BOOT INTO AN EXHAUSTED QUOTA IS REFUSED BEFORE THE AGENT
                #    RUNS. It spends the wake, changes nothing, and — because the
                #    refusal grows the transcript — looks enough like activity to
                #    keep the loop going. So: skip, do NOT count it as a boot
                #    attempt, and say WHY in the log, because "nothing happened
                #    for an hour" with no line explaining it is how a watchdog
                #    becomes unfalsifiable.
                action = "HOLD:rate-limit"
                held_m = (rl["until"] - time.time()) / 60
                log(f"{'RATE-HOLD':<14} {c['age'] / 60:>6.1f}m  {action:<12} {uuid[:8]}  "
                    f"quota hold {held_m:.0f}m left (seen on {rl['seen_on'][:8]})")
                if not args.dry_run:
                    update_sub(uuid, s)
                continue
            if s["boots"] >= MAX_BOOTS:
                action = "ESCALATE"
                rc = max(rc, 4)
                if not s["escalated"]:
                    escalate(host, row, f"did not wake after {MAX_BOOTS} boots "
                                        f"({c['age']/60:.0f} min idle)")
                    s["escalated"] = True
            else:
                s["boots"] += 1
                via = boot(host, row, args.dry_run)
                if via == "refused-draft":
                    # ⛔ A refusal is NOT a failed boot and must not count as one.
                    # The row is idle because its owner is mid-sentence, which is
                    # the one state where booting is worse than waiting — so give
                    # the attempt back, keep the deferral, and try again next
                    # tick. ⚠ Do NOT `continue` here: the state write and the
                    # window log at the bottom of the loop are what make a
                    # skipped row visible instead of merely absent.
                    s["boots"] -= 1
                    action = "SKIP:drafting"
                else:
                    # Say WHICH door delivered it. A watchdog that reports
                    # "booted" without saying how cannot be debugged when it
                    # silently stops.
                    action = f"BOOT#{s['boots']}:{via or 'NOT-DELIVERED'}"
                    # ⭐ A deferral covers ONE wait. Once the boot it was
                    # protecting has fired, the reason for it is over — leaving
                    # it set would silently widen the window for everything that
                    # follows.
                    for k in ("boot_after_secs", "boot_after_until", "boot_after_note"):
                        s.pop(k, None)
                    if not via:
                        rc = max(rc, 4)
                        if not s["escalated"]:
                            escalate(host, row, "a boot could not be delivered by either "
                                                "the composer or the PTY")
                            s["escalated"] = True
                    rc = max(rc, 3)
        # ⛔ A dry run must not mutate state — an instrument whose observation
        #    changes what it observes is not an instrument.
        if not args.dry_run:
            update_sub(uuid, s)
        # Print the WINDOW this row is being judged against, not just its age.
        # Without it a deferred row and a default one look the same in the log,
        # and "why was it not booted at 8 minutes" costs a code read to answer.
        win, why = boot_after_for(s)
        window = f"{win//60}m" + (f"/{why}" if why else "")
        log(f"{state:<14} {c['age']/60:>6.1f}m  {action:<12} {uuid[:8]}  win={window}")
    return rc


def watcher_alive():
    if not PIDFILE.exists():
        return None
    try:
        pid = int(PIDFILE.read_text().strip())
    except Exception:
        return None
    try:
        cmd = Path(f"/proc/{pid}/cmdline").read_bytes().decode(errors="ignore")
    except OSError:
        return None
    # ⛔ Identify, never count. A pid file whose pid has been REUSED by an
    #    unrelated process is how a watchdog reports itself alive while nothing
    #    is watching — the exact silence it exists to prevent.
    return pid if "ygg-booter" in cmd else None


def ensure_watcher(args):
    alive = watcher_alive()
    if alive:
        # ⛔ "already running" IS NOT "already working" — §7 of this skill, applied
        # to this skill's own tool. For 21 hours this line reported a healthy
        # watcher that was ticking into /dev/null, and every subscribe that
        # followed accepted it. A live pid is the request; a written log is the
        # effect. Say which one is missing.
        silent_s = _watcher_silence_secs()
        if silent_s is None or silent_s > max(3 * args.interval, 900):
            return (f"⛔ pid {alive} is alive but MUTE — the log has not been "
                    f"written for "
                    f"{'ever' if silent_s is None else f'{silent_s / 60:.0f}m'}. "
                    f"Its decisions are unrecordable, so nothing it does can be "
                    f"diagnosed. Restart it: kill {alive} && "
                    f"ygg-booter.py subscribe …")
        return f"already running (pid {alive})"
    STATE.mkdir(parents=True, exist_ok=True)
    logf = open(LOGPATH, "a")
    p = subprocess.Popen(
        [sys.executable, str(HERE / "ygg-booter.py"), "watch",
         "--host", args.host, "--interval", str(args.interval)],
        stdout=logf, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
        start_new_session=True)
    time.sleep(1.5)
    return f"armed (pid {p.pid})" if watcher_alive() else "⛔ FAILED TO ARM"


def cmd_watch(args):
    global _WATCH_STARTED_TS
    _WATCH_STARTED_TS = time.time()
    PIDFILE.parent.mkdir(parents=True, exist_ok=True)
    PIDFILE.write_text(str(os.getpid()))
    log(f"watcher up (pid {os.getpid()}, interval {args.interval}s, gui host {args.host})")
    try:
        while True:
            # ⛔ THE HEARTBEAT ASSERTS THE LOG IS BREATHING, NOT ONLY THAT WE ARE.
            # A pid-and-timestamp heartbeat is a claim about the wrong subject:
            # it proved the process lived through 21 hours in which it said
            # nothing (see `log`). Carrying the log's own mtime/size makes the
            # blackout visible from OUTSIDE the process — which is the only place
            # it can be noticed, since the mute process has no way to tell.
            HEARTBEAT.write_text(json.dumps({
                "ts": time.time(),
                "pid": os.getpid(),
                # Both fields written by the SAME process about ITSELF, so the
                # gap between them is this watcher's own silence and nobody
                # else's. `0.0` means it has never managed to log at all.
                "last_log_write_ts": _LAST_LOG_WRITE_TS,
                "started_ts": _WATCH_STARTED_TS,
                "log_path": str(LOGPATH),
            }))
            if not load_subs():
                log("no subscribers left — retiring")
                break
            tick(args)
            time.sleep(args.interval)
    finally:
        PIDFILE.unlink(missing_ok=True)
    return 0


def cmd_status(args):
    if args.json:
        st = fleet_state(args)
        print(json.dumps(st, indent=1))
        return 0 if (st["watcher"]["alive"] and not st["watcher"]["mute"]) else 1
    alive = watcher_alive()
    hb = "never"
    mute = ""
    if HEARTBEAT.exists():
        try:
            beat = json.loads(HEARTBEAT.read_text())
            hb = f"{time.time() - beat['ts']:.0f}s ago"
            # ⛔ ALIVE IS NOT AUDIBLE. A watcher whose stdout was wired to
            # /dev/null ticked for 21 hours with a perfect heartbeat and an
            # untouched log; a status that reports only liveness called that
            # healthy. Report the log's own silence as a FAULT, loudly, because
            # every campaign's "the booter woke me late" is unfalsifiable while
            # it lasts — and because the mute process cannot notice it itself.
            silent_s = _watcher_silence_secs(beat)
            if silent_s is None:
                mute = " · ⛔ MUTE: no log file at all"
            elif alive and silent_s > max(3 * args.interval, 900):
                mute = (f" · ⛔ MUTE: heartbeat is current but the log has not "
                        f"been written for {silent_s / 60:.0f}m — this watcher is "
                        f"ticking silently, restart it "
                        f"(kill {alive}; ygg-booter.py subscribe …)")
        except Exception:
            pass
    # ⛔ THE OFF SWITCH IS PART OF THE STATUS LINE, not a footnote. A disarmed
    #    booter and a broken one are the same silence from the outside; the only
    #    thing that tells them apart is this line, and a human debugging "why was
    #    I not booted" reads it first.
    d = disarm_state()
    armed = "⛔ DISARMED" if d else "armed"
    if d:
        armed += (" until re-armed" if not d.get("until")
                  else f" {(d['until'] - time.time()) / 60:.0f}m left")
        if d.get("note"):
            armed += f" ({d['note']})"
    # ⛔⛔ "armed" AND "QUOTA HOLD" USED TO APPEAR IN THE SAME LINE, and "armed"
    #    came first — so wrapping tools grepped it and reported ✅ ARMED through
    #    a 7.5-hour fleet-wide blackout in which no boot could be delivered to
    #    anybody. The hold was printed the whole time and nobody read past the
    #    first word. ⇒ The instruments knew SUBSCRIBED (wakeable) and ADDRESSABLE
    #    (namable); none of them knew DELIVERABLE, which is the only one a caller
    #    asking "am I watched" actually cares about.
    #    ⇒ A held booter must not describe itself as armed AT ALL. One state, one
    #    word, and the word says what a boot would actually do right now.
    rl = rate_limit_hold()
    hold = ""
    if rl:
        armed = (f"⏸ HELD — NO BOOT CAN BE DELIVERED TO ANY ROW "
                 f"({(rl['until'] - time.time()) / 60:.0f}m left)")
        hold = (f" · quota refusal last seen on {rl['seen_on'][:8]}; "
                f"{len(load_subs())} subscriber(s) are unwakeable meanwhile")
    log(f"watcher: {'alive pid ' + str(alive) if alive else 'NOT RUNNING'} · "
        f"{armed} · heartbeat {hb} · subscribers {len(load_subs())}{hold}{mute}")
    return 0 if (alive and not mute) else 1


def main():
    ap = argparse.ArgumentParser(description="boot a stalled session that subscribed")
    ap.add_argument("action",
                    choices=["subscribe", "unsubscribe", "defer", "list", "tick",
                             "watch", "status", "disarm", "arm", "coverage", "retire",
                             "never-arm", "optout"])
    ap.add_argument("--secs", type=int, default=0,
                    help=f"defer: boot window for one long wait, clamped to "
                         f"{MIN_BOOT_AFTER_SECS}-{MAX_BOOT_AFTER_SECS}s "
                         f"(the ceiling is the prompt-cache limit)")
    ap.add_argument("--clear", action="store_true",
                    help="defer: drop the deferral and return to the default window")
    ap.add_argument("--row", default="")
    ap.add_argument("--campaign", default="")
    ap.add_argument("--note", default="")
    ap.add_argument("--evidence", default="",
                    help="retire: what makes you say this row is dead")
    ap.add_argument("--decided-by", default="",
                    help="retire/never-arm/optout: who decided (a seat number or name)")
    ap.add_argument("--verbose", action="store_true",
                    help="coverage: list every bucket, not only UNKNOWN")
    ap.add_argument("--rearm", default="",
                    metavar="WHY",
                    help="subscribe: arm a row that previously opted out via "
                         "`ygg-claim.sh --no-booter`. Requires a reason, which is "
                         "APPENDED to the ledger so the decision history survives. "
                         "⛔ Not a way past never-arm.tsv, which refuses regardless.")
    ap.add_argument("--host", default=None,   # ⛔ resolved, never a placeholder
                    help="the GUI host — app control resolves only there")
    ap.add_argument("--max-hours", type=float, default=12.0)
    ap.add_argument("--kind", choices=("task", "monitor"), default="task",
                    help="task: has a terminal state, may unsubscribe itself when done. "
                         "monitor: watches something still live, so 'done' is never true — "
                         "unsubscribing one needs --force")
    ap.add_argument("--force", action="store_true",
                    help="unsubscribe: stop watching even a monitor subscription")
    ap.add_argument("--interval", type=int, default=DEFAULT_INTERVAL)
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--hours", type=float, default=None,
                    help=f"disarm: how long to stay off before re-arming itself "
                         f"(default {DISARM_HOURS:g}h)")
    ap.add_argument("--forever", action="store_true",
                    help="disarm: stay off until `arm` is run by hand. ⛔ A safety "
                         "net switched off and forgotten is worse than none, so "
                         "this is deliberately not the default")
    ap.add_argument("--json", action="store_true",
                    help="list/status: the machine-readable state, for a surface "
                         "to render and drive")
    ap.add_argument("--due", action="store_true",
                    help="list/status --json: also classify each subscriber to say "
                         "WHEN it is next at risk of a boot. Costs a row-list call "
                         "and a transcript read per subscriber, so it is opt-in")
    args = ap.parse_args()
    # ⛔ Never carry a placeholder host into a boot decision.
    # ⭐ But a LEDGER WRITE IS NOT A BOOT DECISION, and making it wait on the GUI
    #    host is how recording a decision fails on a host that cannot reach the
    #    desktop — which is precisely the moment someone gives up and edits the
    #    file by hand. These verbs touch only local state, so they never ask.
    if args.action in ("never-arm", "optout", "retire", "arm", "disarm"):
        args.host = args.host or this_host()
    else:
        args.host = resolve_gui_host(args.host)
    return {
        "subscribe": cmd_subscribe,
        "unsubscribe": cmd_unsubscribe,
        "defer": cmd_defer,
        "list": cmd_list,
        "tick": tick,
        "watch": cmd_watch,
        "status": cmd_status,
        "coverage": cmd_coverage,
        "retire": cmd_retire,
        "never-arm": cmd_never_arm,
        "optout": cmd_optout,
        "disarm": cmd_disarm,
        "arm": cmd_arm,
    }[args.action](args)


if __name__ == "__main__":
    sys.exit(main())
