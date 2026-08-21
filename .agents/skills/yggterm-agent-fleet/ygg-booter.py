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
import hashlib
import importlib.util
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ygg_host import resolve_gui_host  # noqa: E402
from ygg_rowarg import bare_uuid, resolve_row  # noqa: E402

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

# ⛔⛔ A REFUSAL THAT REPEATS FOR THE SAME REASON IS A CONDITION, AND A CONDITION
# MUST BE VISIBLE IN STATE. Measured 2026-08-21 across four campaigns: four rows
# sat permanently unbootable for DAYS while every instrument read healthy. The
# refusal was correct each time and the refund is correct too — the row was never
# asked, so charging it a wake would be a lie — but a refund means `boots` never
# rises, so the row can never reach MAX_BOOTS and can therefore NEVER escalate.
# The only record of the standing condition was one log line per tick, in a log
# nobody tails.
#
# ⚖ THE SILENCE IS THE DEFECT, NOT THE REFUSAL. So a run of identical refusals is
# counted on the subscription itself (`standing_refusal`), surfaced by `list` and
# `status`, and escalated exactly ONCE per condition — while the refusal decision
# and the refund are left exactly as they were.
#
# ⚠ WHY 12 AND NOT MAX_BOOTS. A draft or a choice prompt is a real thing in front
# of a real row and waiting is the right answer; escalating at three ticks (15 min)
# would page a human about an owner who stepped away from a half-typed sentence.
# Twelve ticks is an hour at the default interval — long enough that a genuine
# owner-facing prompt lives its natural life, short enough that "days" is
# impossible.
STANDING_REFUSAL_TICKS = 12

# How many consecutive identical refusals before a human is told, per reason.
# ⛔ `None` means NEVER, and every None states its reason here rather than
# leaving the absence to be read as an oversight — which is exactly how the
# missing entry below became a crash.
STANDING_REFUSAL_ESCALATE_AFTER = {
    # OUR OWN INSTRUMENT failing, and it does not clear itself: if the verb the
    # guard reads with is missing from the running build, the screen is
    # unreadable on every tick from now until someone is told. Bounded tight.
    "refused-screen-unreadable": MAX_BOOTS,
    # Observations of the ROW. Each clears itself when the row moves on — except
    # when nothing can move it, which is the case this exists for.
    "refused-draft": STANDING_REFUSAL_TICKS,
    "refused-choice-prompt": STANDING_REFUSAL_TICKS,
    "refused-draft-race": STANDING_REFUSAL_TICKS,
    # ⛔ NEVER, and deliberately: a limit wait is self-resolving by construction
    # (the CLI's own auto-continue is armed) and a human cannot grant quota, so
    # paging one buys nothing. Same argument as the fleet-wide RATE-LIMITED
    # hold. It is still COUNTED and still shown, so the wait is never invisible.
    "refused-limit-wait": None,
    # ⛔ THIS LANE ADDED FOUR REFUSALS AND HAD TO COME BACK FOR THIS TABLE AND FOR
    # THE REFUND LIST BESIDE IT — which is the exact omission the comment on that
    # list was written about, committed by the person who had just read it.
    # ⇒ A guard that learns a new refusal is not finished until BOTH lists know
    #   the name: one decides whether the row is charged a wake it never got, the
    #   other whether anybody is ever told.
    # No composer drawn is an observation of the ROW and clears when it moves on.
    "refused-no-composer": STANDING_REFUSAL_TICKS,
    # ⛔ These two do NOT clear themselves. A write that could not be confirmed
    # stands in the composer until somebody takes it out, and refusing to type a
    # second copy is correct and permanent — so the row is alive, unwakeable, and
    # nothing about waiting changes that. Bounded tight, like a blind instrument.
    "refused-unconfirmed-write": MAX_BOOTS,
    "refused-submit-unconfirmed": MAX_BOOTS,
    # Our own state directory refusing a write. Also does not clear itself.
    "refused-no-ledger": MAX_BOOTS,
}

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
#
# ⛔⛔ EVERY LINE ABOVE IS ABOUT A TIMED WINDOW AND NONE OF IT HOLDS FOR AN
# EXHAUSTED CREDIT BALANCE — see `refusal_is_a_balance_not_a_window`. There the
# limit is not account-wide (the refusal offers `/model`, a per-row switch), a
# timer clears nothing, and a human CAN fix it. That class takes a per-row
# suspension and one escalation, and the fleet stays wakeable.
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


def _host_serves_rows(host):
    """TRI-STATE: True serves a row plane · False answered-and-does-not · None could-not-ask.

    ⛔ `ygg()` returns {} for a transport failure AND for a host that replied with
    nothing useful, so it cannot answer this question. The distinction decides
    whether a bad `--host` is refused (it should be) or a network blip leaves a
    live row unarmed (it must not)."""
    try:
        if host == this_host():
            cmd = [str(Path.home() / ".local" / "bin" / "yggterm"), "server", "app", "rows"]
            r = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
        else:
            r = subprocess.run(["ssh", "-o", "ConnectTimeout=10", host,
                                "$HOME/.yggterm/bin/yggterm server app rows"],
                               capture_output=True, text=True, timeout=60)
    except Exception:
        return None                       # could not ask
    body = r.stdout[r.stdout.find("{"):] if "{" in r.stdout else ""
    if not body:
        return None                       # no JSON at all ⇒ we never reached the verb
    try:
        d = json.loads(body)
    except Exception:
        return None
    return ((d.get("data") or {}).get("rows") is not None)


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
    """The active fleet-wide quota hold, or None. Expiry evaluated HERE, once.

    ⛔⛔ A HOLD WRITTEN BY PRE-FIX CODE IS NOT HONOURED, AND THAT IS THE POINT.

    This file lives in the repo, so **every worktree carries its own copy** and a
    watcher runs whichever copy sits in the directory it was started from.
    Measured 2026-08-14, within minutes of shipping the anti-re-arm fix: a second
    watcher was started from a checkout whose tree predated it, and **it pinned
    the whole fleet again** — its record advanced `until` on every tick from the
    same frozen tail, exactly as before, while `git log` showed the fix landed and
    a reader would have believed it was live. 13 of 14 checkouts on that host were
    still pre-fix at the time.

    ⇒ A record with no `counted` key was written by code that **cannot honour the
    invariant that ends a hold**. It is therefore not merely stale, it is
    *unbounded* — nothing in it will ever stop growing. Refusing it costs at worst
    one probe boot during a genuine outage, and the next refusal re-arms a proper
    hold through the fixed path. Honouring it has no exit at all.

    ⚠ This is a floor, not the fix. The real defect is that a watcher's code comes
    from wherever it was launched; see the queue entry on the supervision watcher's
    deploy path.
    """
    try:
        d = json.loads(RLHOLDFILE.read_text())
    except Exception:
        return None
    if "counted" not in d:
        log("⛔ discarding a quota hold written by pre-fix code — it has no "
            "evidence ledger, so nothing in it can ever stop it growing. "
            "A stale watcher wrote this; check for a second watcher.")
        RLHOLDFILE.unlink(missing_ok=True)
        return None
    # ⛔⛔ AN INDEFINITE HOLD NEVER EXPIRES, AND `until` IS ABSENT ON ONE. Without
    #    this clause `(d.get("until") or 0)` is 0, `time.time() >= 0` is always
    #    true, and the strongest hold available would delete ITSELF on the very
    #    next read — the failure would look exactly like "no hold was ever set".
    if d.get("indefinite"):
        return d
    # ⛔⛔ A HOLD WHOSE OWN EVIDENCE THIS CODE RECLASSIFIES IS RELEASED, NOT
    #    WAITED OUT. Measured 2026-08-21, minutes after the balance/window split
    #    shipped: the watcher was restarted, recognised the refusal as an
    #    exhausted BALANCE on its first tick, suspended that one row exactly as
    #    designed — and 18 other rows stayed unwakeable for another 16 minutes
    #    behind a hold the previous code had armed from the very same tail.
    #
    #    ⇒ The fix corrected the DECISION and left the ARTEFACT in force. That is
    #    the shape this file already knows from `note_rate_limit`: a frozen tail
    #    read as a live signal. Here it is worse, because the tail is one this
    #    code has just decided was never fleet-wide in the first place — so the
    #    hold is not merely stale, it is unsupported by its own record.
    #
    #    ⚖ A DECLARED hold is untouched. That one is an instruction from a human
    #    who could see the account, and a reclassification of the automatic path
    #    does not get to overrule it.
    if (not d.get("declared_until")
            and refusal_is_a_balance_not_a_window(d.get("tail"))):
        log("⭐ RELEASING the fleet quota hold — its own recorded refusal is an "
            "exhausted CREDIT BALANCE, which this code holds PER ROW rather "
            "than fleet-wide. The row that hit it is suspended; nobody else "
            "should have been waiting on it.")
        RLHOLDFILE.unlink(missing_ok=True)
        return None
    if time.time() >= (d.get("until") or 0):
        RLHOLDFILE.unlink(missing_ok=True)
        return None
    return d


def parse_until(spec, now):
    """`5d` / `36h` / `90m`, or an absolute local `YYYY-MM-DDTHH:MM`. -> epoch.

    ⭐ **RESOLVED TO AN ABSOLUTE INSTANT ONCE, HERE**, per the owner's standing
    design note: *a relative delay decays, an absolute one is idempotent.* A
    duration is a convenience at the keyboard; what gets STORED is the moment, so
    re-running the same command does not walk the deadline forward, and a hold
    survives a watcher restart without gaining time.
    """
    spec = (spec or "").strip()
    if not spec:
        raise ValueError("empty --until")
    m = re.fullmatch(r"(\d+(?:\.\d+)?)([dhm])", spec, re.I)
    if m:
        n, unit = float(m.group(1)), m.group(2).lower()
        return now + n * {"d": 86400, "h": 3600, "m": 60}[unit]
    for fmt in ("%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M:%S"):
        try:
            return time.mktime(time.strptime(spec, fmt))
        except ValueError:
            pass
    raise ValueError(f"cannot read --until {spec!r}: use 5d / 36h / 90m or 2026-08-19T09:00")


def hold_remaining(rl):
    """How long a hold has left, in words. ⛔ ONE owner for this question.

    Six call sites rendered `(rl['until'] - now)` inline, and every one of them
    would divide `None` on an indefinite hold — the same shape as the `seen_on`
    crash, which was fixed in one place and left in two. A hold that cannot be
    DISPLAYED is a hold nobody can confirm, and this file's whole job is to be
    confirmable.
    """
    if rl.get("indefinite"):
        return "INDEFINITE — it will NEVER lift by itself"
    left = (rl.get("until") or 0) - time.time()
    return f"{left / 60:.0f}m left" if left < 5400 else f"{left / 3600:.1f}h left"


def cmd_hold(args):
    """Declare a fleet-wide boot hold BEFORE anything gets banged.

    ⛔⛔ WHY THIS EXISTS AND WHY THE REACTIVE PATH IS NOT ENOUGH. `note_rate_limit`
    arms a hold by reading a row's transcript tail — which means **a row has to be
    booted and refused first**. That is fine for a session limit nobody saw
    coming; it is exactly the wrong shape for a limit the OWNER KNOWS IS ABOUT TO
    BE HIT. Waiting for the detector costs one wasted wake per subscriber, on an
    account with nothing left to spend, and each of those wakes is the "banging"
    this verb removes.

    ⭐ It writes the SAME file the reactive path uses, so the suppression is the
    one already proven in `tick` — one hold mechanism, not two. What it adds is
    `declared_until`, a FLOOR that a later tail sighting may raise and never
    lower (see `note_rate_limit`), because a session-limit tail would otherwise
    shorten a week-long hold to thirty minutes.

    ⚠ `counted` is written empty and that is deliberate: `rate_limit_hold()`
    discards any record lacking the key as pre-fix code, and a declared hold has
    no per-row evidence to carry.

    ⭐ IT ENDS BY ITSELF. `rate_limit_hold()` unlinks the file the moment `until`
    passes, so the next tick boots normally — and because every subscriber has
    been idle throughout, they are all past their boot window and go on the FIRST
    tick after expiry. That is the intended behaviour, not a side effect: sit out
    the outage, then wake everyone promptly.
    """
    now = time.time()
    cur = rate_limit_hold()
    if args.clear:
        if not cur:
            log("no hold in force — nothing to clear")
            return 0
        RLHOLDFILE.unlink(missing_ok=True)
        log(f"⭐ hold CLEARED (was {hold_remaining(cur)}). "
            f"Boots resume on the next tick.")
        return 0
    if args.forever:
        # ⛔⛔ A HOLD WITH NO EXIT IS THE SHAPE THIS PROJECT KEEPS PAYING FOR, so
        #    it is allowed only because it is LOUD. It never lifts by itself, it
        #    says so on every listing and every status line, and the only way out
        #    is a person typing `hold --clear`. That is the point: after a weekly
        #    reset EVERY session is cold, so an automatic wake would spend a full
        #    cold context re-read per subscriber before any work happened — the
        #    fleet would burn a large share of the fresh window on re-reading
        #    itself. Recovery is meant to start with ONE master orchestrator,
        #    chosen by a human, which then decides who comes back.
        rec = {
            "since": (cur or {}).get("since", now),
            "last_seen": now,
            # ⛔⛔ BELT AND BRACES, AND THE BELT IS FOR READERS THAT PREDATE THE
            #    BRACES. An `until: null` record is not merely misread by older
            #    code — it is DESTROYED by it: `time.time() >= (d.get("until")
            #    or 0)` is `>= 0`, always true, so the old reader UNLINKS the
            #    strongest hold available and the fleet resumes booting. Measured
            #    the hard way: an indefinite hold armed while a pre-indefinite
            #    watcher was still alive vanished within twenty seconds, and the
            #    aftermath is indistinguishable from "no hold was ever set".
            #    ⚠ This host carries a dozen checkouts of this script and any of
            #    them running `list` would have done the same.
            # ⇒ A far-future deadline makes an OLD reader honour it; the flag
            #    makes a NEW reader describe it honestly. Neither alone is enough,
            #    and the ordering lesson stands on its own: DEPLOY THE READER
            #    EVERYWHERE BEFORE WRITING A RECORD ONLY THE NEW ONE UNDERSTANDS.
            "until": now + 10 * 365 * 86400,
            "indefinite": True,
            "counted": dict((cur or {}).get("counted") or {}),
            "declared_until": None,
            "declared_reason": args.reason or args.note or "declared by hand, indefinite",
            "declared_by": args.decided_by or this_host(),
            "stale_sighting": False,
            "reset_at": None,
            "released_by": "a human running `ygg-booter.py hold --clear`",
            "seen_on": None,
            "tail": None,
        }
        RLHOLDFILE.parent.mkdir(parents=True, exist_ok=True)
        RLHOLDFILE.write_text(json.dumps(rec, indent=1))
        back = rate_limit_hold()
        if not back or not back.get("indefinite"):
            log("⛔ indefinite hold did NOT read back — the fleet is still free to boot.")
            return 1
        subs = load_subs()
        log(f"⏸ FLEET HOLD ARMED — ⛔ INDEFINITE. {len(subs)} subscriber(s) will not be "
            f"booted, and nothing will lift this on its own.")
        log(f"   reason: {rec['declared_reason']}")
        log("   ⛔ IT WILL NOT EXPIRE. Release is deliberate: `ygg-booter.py hold --clear`.")
        log("   ⭐ RECOVERY IS THE OWNER'S TO START — he starts one master orchestrator")
        log("      himself. Nothing here should wake anybody: after a limit reset every")
        log("      session is COLD, so an automatic wake pays a full context re-read per")
        log("      row before any work happens, and 7 of those land at once.")
        return 0
    if not args.until:
        if not cur:
            log("no hold in force — the booter is free to wake stalled rows")
            return 0
        kind = ("DECLARED INDEFINITE" if cur.get("indefinite")
                else "DECLARED" if cur.get("declared_until") else "detected from a tail")
        when = ("" if cur.get("indefinite") else
                f", until {time.strftime('%Y-%m-%d %H:%M', time.localtime(cur['until']))}")
        log(f"⏸ hold in force: {hold_remaining(cur)}{when} ({kind})")
        if cur.get("declared_reason"):
            log(f"   reason: {cur['declared_reason']}")
        return 0
    until = parse_until(args.until, now)
    if until <= now:
        log(f"⛔ refusing: {args.until} is in the past. A hold that has already "
            f"expired is not a hold, and writing one would read as protection.")
        return 2
    # ⛔ Never SHORTEN an existing hold by accident — say so and keep the longer.
    if cur and cur.get("indefinite"):
        log("⛔ an INDEFINITE hold is already in force — refusing to replace it with a "
            "dated one, because that would silently give the fleet an expiry it was "
            "deliberately denied. Run `hold --clear` first if you mean to.")
        return 2
    if cur and (cur.get("until") or 0) > until:
        log(f"⚠ an existing hold runs longer "
            f"({(cur['until'] - now) / 3600:.1f}h vs {(until - now) / 3600:.1f}h) — keeping it. "
            f"Use `hold --clear` first if you really mean to shorten it.")
        until = cur["until"]
    rec = {
        "since": (cur or {}).get("since", now),
        "last_seen": now,
        "until": until,
        "counted": dict((cur or {}).get("counted") or {}),
        "declared_until": until,
        "declared_reason": args.reason or args.note or "declared by hand",
        "declared_by": args.decided_by or this_host(),
        "stale_sighting": False,
        "reset_at": None,
        "released_by": "declared-deadline",
        "seen_on": None,
        "tail": None,
    }
    RLHOLDFILE.parent.mkdir(parents=True, exist_ok=True)
    RLHOLDFILE.write_text(json.dumps(rec, indent=1))
    # ⛔ READ IT BACK. Every verb in this fleet reports the REQUEST, not the
    #    effect, and a hold nobody can prove is in force is worth nothing.
    back = rate_limit_hold()
    if not back or abs((back.get("until") or 0) - until) > 1:
        log("⛔ hold did NOT read back — the fleet is still free to boot.")
        return 1
    subs = load_subs()
    log(f"⏸ FLEET HOLD ARMED until "
        f"{time.strftime('%Y-%m-%d %H:%M', time.localtime(until))} "
        f"({(until - now) / 3600:.1f}h) — {len(subs)} subscriber(s) will not be booted.")
    log(f"   reason: {rec['declared_reason']}")
    log("   ⭐ it releases itself at that instant; the next tick then wakes every")
    log("      row that is past its window, which is all of them by definition.")
    return 0


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
    # ⛔ A DENYLIST OF SHELLS WAS THE WRONG SHAPE AND I SHIPPED IT. `python3` was
    #    not on it, so a probe of the form `python3 -c "…<uuid>…"` — which is how
    #    every sweep in this fleet is written — matched ITSELF and reported the
    #    row alive. That is the FIFTH costume of one trap in a single night:
    #    `pgrep -f <uuid>` · "exclude grep" · "cmdline contains claude" ·
    #    "argv[0] is not a shell" · and now the querying interpreter.
    #    ⇒ Stop enumerating what ISN'T the row. Exclude THIS process explicitly,
    #    and treat an inline interpreter invocation (`-c`) as what it always is:
    #    somebody asking the question, never the row being asked about.
    #    ⚠ The alive-bias is deliberately KEPT — anything else still counts as
    #    running — because a false "absent" would skip a fleet hold during a real
    #    outage, while a false "present" merely preserves today's behaviour.
    shells = {"bash", "sh", "zsh", "dash", "ssh", "grep", "awk", "sed", "xargs"}
    try:
        pids = [d for d in os.listdir("/proc") if d.isdigit()]
    except Exception:
        return False                      # cannot tell ⇒ treat as alive
    needle, me = uuid.encode(), str(os.getpid())
    for d in pids:
        if d == me:
            continue                      # the querying process is not the row
        try:
            cl = Path(f"/proc/{d}/cmdline").read_bytes()
        except Exception:
            continue
        if not cl or needle not in cl:
            continue
        parts = cl.split(b"\0")
        a0 = os.path.basename(parts[0].decode("utf8", "ignore"))
        if a0 in shells:
            continue
        if a0.startswith(("python", "perl", "ruby")) and b"-c" in parts[1:2]:
            continue                      # an inline script is a probe, not a row
        return False
    return True


# ⭐⭐ THE RESET TIME WAS IN THE MESSAGE ALL ALONG — measured 2026-08-14.
# The comment above RATE_LIMIT_HOLD_SECS asserted "the refusal carries no reset
# timestamp ('try again later')" and held the fleet on a blind timer because of
# it. That premise was false, and this tool's own evidence file disproved it: a
# stored hold record read
#     "tail": "You've hit your session limit · resets 12:20pm (Asia/Kolkata)"
# The reset moment was captured, persisted, and printed in `status` — and nothing
# ever parsed it. Cost, measured live: last refusal 12:35:54 held the fleet to
# 13:05:54 against a reset that had already happened at 12:20, leaving 17
# subscribers unwakeable while the account was healthy.
# ⭐ THE CLASS: an answer the system already holds, discarded because a comment
# said it did not exist. Nobody re-read the artefact the comment was about.
#
# ⚠ IT MAY ONLY SHORTEN THE HOLD, NEVER EXTEND IT. The two failure directions
# are not symmetric: holding too SHORT costs one refused probe boot and
# self-corrects on the next tick, while holding too LONG is a dead fleet. So the
# timer stays as a CEILING and a parsed reset can only release earlier. A
# misparse therefore cannot invent a long hold, which is the only way this change
# could have made things worse.
RESET_GRACE_SECS = 90          # clocks disagree; come back just after, not on the tick
RESET_MAX_AHEAD_SECS = 6 * 3600  # further out than this is a misparse, not a reset
RESET_PAST_WINDOW_SECS = 6 * 3600  # how far back a "already reset" claim stays credible


def reset_time_from_tail(tail, now=None):
    """The reset moment the CLI already told us, as an epoch — or None.

    ⛔ None means *nothing trustworthy here*, and every caller must fall back to
    the blind timer. That is the safe direction: an unparsed tail leaves today's
    behaviour exactly as it was.

    ⚠ A time-of-day with no date is ambiguous across midnight, so a reset that
    has already passed rolls to tomorrow — and a "reset" more than a few hours
    out is far more likely a STALE TAIL being re-read than a real quota window.
    Both collapse to None, which is why a stale screen cannot pin the fleet.
    """
    if not tail:
        return None
    m = re.search(r"resets?\s+(\d{1,2})(?::(\d{2}))?\s*([ap]\.?m\.?)?"
                  r"(?:\s*\(([^)]+)\))?", tail, re.I)
    if not m:
        return None
    hour, minute, ampm, tzname = m.group(1), m.group(2), m.group(3), m.group(4)
    hour, minute = int(hour), int(minute or 0)
    if ampm:
        ampm = ampm.replace(".", "").lower()
        if ampm == "pm" and hour != 12:
            hour += 12
        elif ampm == "am" and hour == 12:
            hour = 0
    if not (0 <= hour <= 23 and 0 <= minute <= 59):
        return None
    import datetime
    tz = None
    if tzname:
        try:
            from zoneinfo import ZoneInfo
            tz = ZoneInfo(tzname.strip())
        except Exception:
            tz = None            # unknown zone ⇒ local, not a failure
    now_dt = datetime.datetime.fromtimestamp(now or time.time(), tz)
    cand = now_dt.replace(hour=hour, minute=minute, second=0, microsecond=0)
    if cand <= now_dt:
        cand += datetime.timedelta(days=1)
    epoch = cand.timestamp()
    if epoch - (now or time.time()) > RESET_MAX_AHEAD_SECS:
        return None              # stale tail or misparse ⇒ let the timer decide
    return epoch


def tail_reset_has_passed(tail, now=None):
    """True when the tail names a reset time that is already BEHIND us.

    ⛔ Deliberately narrow. Only answers True for a time within the last
    `RESET_PAST_WINDOW_SECS`, so a tail whose reset is far in the past — a row
    parked overnight, a clock skewed by a day — falls through to the ordinary
    path instead of silently disarming the fleet on a wild parse.
    """
    if not tail:
        return False
    now = now or time.time()
    m = re.search(r"resets?\s+(\d{1,2})(?::(\d{2}))?\s*([ap]\.?m\.?)?"
                  r"(?:\s*\(([^)]+)\))?", tail, re.I)
    if not m:
        return False
    hour, minute, ampm, tzname = m.group(1), m.group(2), m.group(3), m.group(4)
    hour, minute = int(hour), int(minute or 0)
    if ampm:
        ampm = ampm.replace(".", "").lower()
        if ampm == "pm" and hour != 12:
            hour += 12
        elif ampm == "am" and hour == 12:
            hour = 0
    if not (0 <= hour <= 23 and 0 <= minute <= 59):
        return False
    import datetime
    tz = None
    if tzname:
        try:
            from zoneinfo import ZoneInfo
            tz = ZoneInfo(tzname.strip())
        except Exception:
            tz = None
    now_dt = datetime.datetime.fromtimestamp(now, tz)
    cand = now_dt.replace(hour=hour, minute=minute, second=0, microsecond=0)
    delta = now - cand.timestamp()
    return 0 < delta <= RESET_PAST_WINDOW_SECS


def _evidence_marker(uuid):
    """`(size, mtime)` of this row's transcript — *has it written anything since?*

    ⛔ Returns None when it cannot be read, and **None must never be treated as
    fresh evidence.** See `note_rate_limit`: an unreadable marker releases the
    hold early, which costs one refused probe boot, whereas treating it as fresh
    re-arms the fleet forever. Same asymmetry as everywhere else in this file.
    """
    import glob
    for p in glob.glob(os.path.expanduser(f"~/.claude/projects/*/{uuid}.jsonl")):
        try:
            st = os.stat(p)
            return [st.st_size, int(st.st_mtime)]
        except OSError:
            continue
    return None


# ⛔⛔ A BALANCE IS NOT A WINDOW, AND ONLY ONE OF THEM IS ACCOUNT-WIDE.
#
# The fleet-wide hold is right for a SESSION LIMIT: the account is out of quota
# for a while, a timer clears it, and booting other rows meanwhile just walks
# them into the same wall one at a time.
#
# It is WRONG for an exhausted CREDIT BALANCE, and the refusal says so itself —
# it offers `/model` as a remedy, which is a PER-ROW switch, and a balance is
# restored by a purchase rather than by waiting. So on that class the fleet hold
# has no upside and a fleet-scale downside: the row is deterministically
# unbootable, the timer clears nothing, and every probe re-arms a blackout over
# every other campaign's rows. Reported live 2026-08-21 with 23 subscribers
# unwakeable behind one row's billing state; the same shape ran 7.4 continuous
# hours on 2026-08-14.
#
# ⚖ AND THE ESCALATION RULE INVERTS WITH IT. The note on RATE_LIMIT_HOLD_SECS
# says not to page a human about a quota window, because a human cannot grant
# quota. A human CAN add credits and CAN switch model — so this is the carve-out
# that note describes, not an exception to it.
#
# ⚠ Matched on the refusal's own wording, which is the only evidence there is:
# the API reports both classes with the same status.
BALANCE_REFUSAL_MARKERS = (
    "out of usage credits", "usage-credits", "purchase more credits",
    "credit balance", "insufficient credits", "add credits",
)


def refusal_is_a_balance_not_a_window(tail):
    """True when the refusal is an exhausted balance rather than a timed window.

    ⛔ CONSERVATIVE BY CONSTRUCTION: anything it does not recognise is treated as
    a WINDOW, so every refusal whose wording has not been measured keeps today's
    fleet-wide behaviour. A wrong True stops holding the fleet during a real
    account outage; a wrong False costs what it costs today, which is at least
    visible in the log."""
    low = (tail or "").lower()
    return any(marker in low for marker in BALANCE_REFUSAL_MARKERS)


def note_rate_limit(uuid, tail):
    """A subscriber was refused on quota ⇒ hold the whole fleet.

    ⛔⛔ THE HOLD USED TO RE-ARM FROM THE ARTEFACT IT HAD ITSELF FROZEN, AND SO
    COULD NEVER END. Reported and reproduced by a sibling campaign 2026-08-14.

    A quota limit is detected by reading the row's transcript TAIL. A row parked
    on the limit **stops writing**, so its tail keeps saying the same thing — and
    every tick re-read that same frozen message, refreshed `last_seen`, and
    pushed `until` out another 30 minutes. **The hold could not expire while rows
    were parked, and the rows were parked because of the hold.** Measured live:
    the feeding transcript had not changed size or mtime for 34 minutes while
    `until` walked 70 minutes past a reset that had already happened.

    ⚠ Deleting the state file does NOT fix it — the next tick re-arms from the
    same stale tail. That was verified by the reporting campaign before filing.

    ⇒ **A sighting may only extend the hold if the row has WRITTEN SOMETHING
    since the sighting we already counted.** The tail is evidence about a moment;
    re-reading it is not a new moment. This is the load-bearing half — parsing the
    reset time out of the tail (above) helps only while the reset is still in the
    future, and a stale tail parses to nothing and falls straight through to the
    timer, which *is* the deadlock.

    ⭐ Same class as the corpse-tail guard below it: **a stale artefact read as a
    live signal.**
    """
    prev = rate_limit_hold()
    now = time.time()
    # ⛔⛔ AN INDEFINITE HOLD IS NOT NEGOTIABLE BY THIS PATH. It is the strongest
    #    statement available — "wake nobody until a human says so" — and every
    #    line below computes a dated window at most RATE_LIMIT_HOLD_SECS wide.
    #    Rewriting the record here silently DROPPED the flag, so one tail
    #    sighting downgraded an indefinite hold to a 30-minute one and the fleet
    #    would resume banging into a dead account. Caught by the test that
    #    asserts the flag survives a sighting, not by reading the code.
    #    ⇒ Same class as `declared_until` immediately below: a reactive rewrite
    #    must carry every field that expresses an INSTRUCTION, not just the one
    #    that was noticed first.
    if prev and prev.get("indefinite"):
        return prev
    # ⛔⛔ A TAIL IS SELF-DATING, AND A RESET THAT HAS PASSED IS EVIDENCE THE
    #    OUTAGE ENDED — not evidence to keep holding. If the message says it
    #    resets at a time that is now behind us, and the row has written nothing
    #    since, then the account is presumptively back and this row is merely
    #    PARKED on an old message. Arming from it is how the fleet stayed held
    #    70 minutes past a reset that had already happened.
    # ⚠ Cheap to be wrong in this direction: we probe once, and a genuine refusal
    #    writes a NEW message with a new marker, which arms a proper hold. The
    #    opposite error has no exit at all.
    if tail_reset_has_passed(tail, now):
        log(f"{uuid[:8]} quota tail names a reset that has already passed — "
            f"treating it as a PARKED row, not a live outage. Not arming.")
        return prev
    marker = _evidence_marker(uuid)
    counted = dict((prev or {}).get("counted") or {})
    # ⛔ `marker is not None` is required: an unreadable transcript is NOT new
    #    evidence. Without that clause a row whose transcript cannot be found
    #    would extend the hold on every single tick, which is the original bug
    #    wearing a different hat.
    is_new_evidence = marker is not None and counted.get(uuid) != marker
    if prev and not is_new_evidence:
        # Nothing has moved. Keep the existing deadline exactly as it was — do
        # not extend, and do not shorten either; another row's live sighting may
        # legitimately own this window.
        until = prev.get("until", now + RATE_LIMIT_HOLD_SECS)
        last_seen = prev.get("last_seen", now)
    else:
        timer_until = now + RATE_LIMIT_HOLD_SECS
        reset_at = reset_time_from_tail(tail, now)
        until = timer_until
        if reset_at:
            # ⛔ min(), never max() — see the note above. The timer is the ceiling.
            until = max(now + 60, min(timer_until, reset_at + RESET_GRACE_SECS))
        last_seen = now
        if marker is not None:
            counted[uuid] = marker
    reset_at = reset_time_from_tail(tail, now)
    # ⛔⛔ A DECLARED HOLD IS A FLOOR THE REACTIVE PATH MAY RAISE AND NEVER LOWER.
    #    Everything above computes a window for a SESSION limit — at most
    #    RATE_LIMIT_HOLD_SECS, i.e. half an hour. A weekly limit is declared by
    #    hand and runs for days, and without this clause the first tail sighting
    #    during that window would overwrite `until` with `now + 1800` and the
    #    fleet would start banging again inside the outage it was told to sit
    #    out. The declaration is the owner's instruction; a tail is a guess about
    #    a moment, and a guess does not get to shorten an instruction.
    declared_until = (prev or {}).get("declared_until") or 0
    if declared_until > until:
        until = declared_until
    rec = {
        "since": (prev or {}).get("since", now),
        "last_seen": last_seen,
        "until": until,
        "counted": counted,
        # Carried forward so the floor survives every reactive rewrite.
        "declared_until": declared_until or None,
        "declared_reason": (prev or {}).get("declared_reason"),
        "declared_by": (prev or {}).get("declared_by"),
        "stale_sighting": bool(prev and not is_new_evidence),
        # ⭐ Recorded so `status` can SAY why it will lift when it does. The
        # owner's complaint about the last outage was never the hold, it was
        # "I do not know how all the sessions recovered."
        "reset_at": reset_at,
        "released_by": "reset-time" if reset_at else "timer",
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
    # ⛔⛔ AN ADDRESS THIS WATCHDOG CANNOT RESOLVE IS A SUBSCRIPTION THAT DELETES
    # ITSELF, AND IT READS AS ARMED THE WHOLE TIME. `row_presence` asks the row
    # plane at `--host`; a bare uuid or a host with no GUI client makes the plane
    # answer, truthfully, that nothing lives at that address. That is `False`, not
    # `None` — so the tri-state below behaves perfectly, counts three consecutive
    # absences, and unsubscribes a live row with the line "GONE — absent from 3
    # consecutive row listings". Indistinguishable from a genuine retirement.
    #
    # Reported 2026-08-14 by an orchestrator who repaired a live instance: a
    # delegate stored `row=<bare-uuid>, host=dev` while the GUI runs elsewhere.
    # ⚠ AND THE ROOT IS STRUCTURAL, NOT CARELESSNESS. Their spawn wrapper raced
    # (the transcript takes ~75 s to appear, the check waited 60), exited non-zero
    # on a spawn that had SUCCEEDED, and never reached its arming step — so a human
    # compensated with a hand-rolled subscribe, and the hand-rolled call reproduced
    # the exact failure the wrapper existed to prevent. **Whenever a wrapper fails,
    # the fallback is a hand-rolled call.** That is why this check belongs here, at
    # the moment someone can still fix it, rather than in the wrapper.
    # ⛔⛔ A LOCAL ROW HAS NO MACHINE SEGMENT, AND REFUSING IT LEFT EVERY LOCAL
    #    ROW UNSUPERVISED. The check demanded `<scheme>://<machine>/<uuid>` and
    #    rejected `local://<uuid>` — which is not a malformed address, it is the
    #    canonical path the create verb RETURNS for a row on the GUI host. The
    #    monitor accepted the same string, so a row could be watched on one plane
    #    and unarmable on the other, and the refusal read as caller error.
    #    Measured 2026-08-20 arming a delegate on the GUI host.
    #    ⇒ Both shapes are addressable; a BARE uuid is still refused, which is
    #    the failure this check was actually written for.
    if args.row:
        addressable = re.match(
            r"^[a-z][a-z0-9+.-]*://([^/]+/)?[0-9a-fA-F-]{36}$", args.row.rstrip("/"))
        if not addressable:
            log(f"⛔ REFUSING to arm — --row {args.row!r} is not an addressable row.")
            log("   It must be <scheme>://<machine>/<full-uuid> for a remote row, e.g.")
            log("     remote-cc://dev/xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx")
            log("   or <scheme>://<full-uuid> for a row on the GUI host, e.g.")
            log("     local://xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx")
            log("   A bare uuid resolves ABSENT on every tick, and this watchdog")
            log("   then unsubscribes it as retired — silently, and looking healthy")
            log("   in `list` until it vanishes. Fix the address, not this check.")
            return 6
    host = args.host or resolve_gui_host(None)
    # ⛔⛔ TRI-STATE, AND I GOT THIS WRONG ON THE FIRST WRITE. The first version
    # refused whenever the probe came back empty — collapsing "this host answered
    # and has no row plane" (a bad address, refuse) into "I could not reach this
    # host at all" (a blip, which must NOT leave a live row unarmed). That is the
    # exact collapse this file's own `row_presence` exists to prevent, committed
    # one screen away from the comment warning about it.
    # ⇒ `ygg()` returns {} for BOTH, so ask the transport directly.
    presence = _host_serves_rows(host)
    if presence is False:
        log(f"⛔ REFUSING to arm {uuid[:8]} — host {host!r} answered, and serves no row plane.")
        log("   App control resolves ONLY where the GUI runs, so a subscription")
        log("   pointed at a host with no GUI client can never see its own row.")
        log("   It would read absent on every tick and lapse as 'GONE'.")
        log("   Pass --host <the GUI host>.")
        return 6
    if presence is None:
        # ⚠ Store, and say so. An unwatched row is the failure this tool exists to
        #   prevent; a possibly-misaddressed one now LAPSES VISIBLY rather than
        #   vanishing, so storing is the safer side of this particular blindness.
        log(f"⚠ COULD NOT VERIFY host {host!r} (transport failed, not a refusal).")
        log("   Arming anyway — an unwatched row is the worse failure. If the")
        log("   address is wrong you will see it LAPSE in `list` rather than")
        log("   silently disappear.")
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
    # ⭐ SAY THAT A LAPSE IS BEING CLEARED. `rec` is built fresh, so `lapsed` drops
    #   out on its own — but a re-arm that silently un-lapses is the same
    #   invisibility running backwards, and it hides how long the row went
    #   unwatched. That interval is the whole cost of the incident this records.
    if prior.get("lapsed"):
        mins = int((time.time() - prior.get("lapsed_at", time.time())) // 60)
        log(f"⭐ CLEARING A LAPSE on {uuid[:8]} — it was unwatched for ~{mins}m "
            f"({prior.get('lapsed_reason', 'no reason recorded')})")
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
        # ⛔ A WATCH PLACED BY SOMEBODY ELSE DEFAULTS TO `monitor`, AND THAT IS
        #    NOT A STYLE CHOICE. Measured 2026-08-14: a triage session armed
        #    another campaign's stalled delegate, the booter woke it 20 minutes
        #    later (`boots=1`, and the `continue` is in that row's transcript),
        #    and the woken row unsubscribed ITSELF four seconds afterwards --
        #    the exact "am I done?" answer `cmd_unsubscribe` exists to refuse,
        #    which it only refuses for monitors. So the wake worked and the
        #    watch died in the same minute, and the NEXT stall had nobody.
        #    ⇒ "Is this finished?" is not the subscriber's question to answer
        #    when the subscriber is not the row. Pass --kind explicitly to
        #    override in either direction.
        "kind": getattr(args, "kind", None)
        or ("task" if uuid == own_uuid() else "monitor"),
        # ⛔⛔ NOT A LIFETIME BOOT COUNT, DESPITE THE NAME. This is the run of
        # CONSECUTIVE boots the row has not answered: any progress resets it to
        # zero ("progress clears the stall counter" in the tick loop), a refused
        # or rate-limited attempt is refunded, and it escalates at MAX_BOOTS.
        # ⇒ **`boots: 0` IS THE HEALTHY STATE**, and a rising value is the
        # alarm.
        #
        # ⛔ THEREFORE COUNTING `boots > 0` ACROSS SUBSCRIBERS MEASURES ROWS
        # THAT ARE FAILING TO ANSWER, NOT ROWS THAT HAVE EVER BEEN WOKEN — the
        # two readings run in opposite directions. Measured 2026-08-21: a census
        # found 2 of 23 live records above zero and concluded the fleet wake path
        # was dead, while this file's own log held 341 successful boots across 98
        # distinct rows, and the seat doing the counting had been woken by that
        # path minutes earlier. ⇒ **The log is the history; this field is a
        # gauge. For "does waking work", read the log.**
        "boots": 0,
        # The run of consecutive refusals for ONE reason, or absent when nothing
        # is standing in the way. Kept apart from `boots` because a refusal is
        # not a wake: the row was never asked. See `note_standing_refusal` and
        # the escalation in the tick loop.
        "standing_refusal": None,
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


def clear_standing_refusal(s):
    """The row moved (or was actually booted), so nothing is standing in the way.

    ⛔ ONE OWNER. This also drops the pre-2026-08-21 `blind_skips` /
    `blind_escalated` pair, which counted exactly one of the five refusals and is
    now expressed through `standing_refusal` like every other. Leaving both would
    be two encodings of "how long has this row been refused", and the older one
    would keep answering for the single reason it knew about."""
    s.pop("standing_refusal", None)
    s.pop("blind_skips", None)
    s.pop("blind_escalated", None)
    # ⭐ AND A BALANCE SUSPENSION ENDS THE SAME WAY AND ONLY THAT WAY. It cannot
    # expire on a timer, for the reason it was not armed on one — nothing about
    # waiting restores a balance. A row that has written again has been paid for
    # by somebody, and that is the only evidence available.
    for key in ("balance_suspended", "balance_marker", "balance_since",
                "balance_escalated"):
        s.pop(key, None)


def note_standing_refusal(s, via):
    """Advance — or start — the run of consecutive refusals for THIS reason.

    A DIFFERENT reason restarts the run rather than extending it: "refused twelve
    times" is only a condition if it is twelve times for the same thing. A row
    that alternates between a draft and a choice prompt is a row someone is
    using, and it should not accumulate toward an escalation."""
    rec = s.get("standing_refusal") or {}
    if rec.get("reason") != via:
        rec = {"reason": via, "since": int(time.time()), "ticks": 0, "escalated": False}
    rec["ticks"] = rec.get("ticks", 0) + 1
    rec["last"] = int(time.time())
    s["standing_refusal"] = rec
    return rec


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

    # ⛔ `--hours` belongs to `disarm`, but nothing stopped it being passed HERE,
    # and `--secs` defaults to 0 — so `defer --hours 3` read zero seconds,
    # clamped up to the 60s floor, wrote a clean read-back and reported success.
    # A request to go quiet for three hours became a boot in one minute, and the
    # reply said it worked. Reported 2026-08-21 by a long-running row that had
    # asked for hours and was woken in 60s.
    #
    # Honour the flag rather than rejecting it: an agent reaching for `--hours`
    # on a "how long to wait" verb has guessed the right thing, and a tool that
    # punishes the natural guess is the defect. `--secs` still wins if both are
    # given, being the more specific.
    if args.secs:
        asked = int(args.secs)
    elif args.hours:
        asked = int(float(args.hours) * 3600)
    else:
        # ⛔ Never silently mean "60 seconds". A bare `defer` is someone asking
        # for a long wait without saying how long — the one reading under which
        # the old behaviour was catastrophic rather than merely wrong.
        log("⛔ defer needs a duration: pass --secs <n> or --hours <n> "
            f"(clamped to {MIN_BOOT_AFTER_SECS}-{MAX_BOOT_AFTER_SECS}s). "
            "Refusing rather than deferring for the 60s floor, which is what a "
            "bare defer used to do while reporting success.")
        return 2
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
            # ⛔ Consecutive UNANSWERED boots, not a lifetime count — zero is
            # healthy. See the field's definition in the subscribe record.
            "boots": s.get("boots", 0),
            "escalated": bool(s.get("escalated")),
            # ⛔ A ROW REFUSED EVERY TICK IS INDISTINGUISHABLE FROM A HEALTHY ONE
            # WITHOUT THIS. `boots` cannot rise (the refusal is refunded, and
            # rightly), so every other field here reads exactly as it would for a
            # row that has simply never needed a boot.
            "standing_refusal": s.get("standing_refusal"),
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
            "indefinite": bool(rl.get("indefinite")),
            "secs_left": None if rl.get("indefinite") else round((rl.get("until") or 0) - now),
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
    # ⛔ AND THE OTHER PLANE. `retire` used to clean the booter and leave the
    #    MONITOR subscription standing, so a row recorded decided-dead kept
    #    escalating a corpse to a live orchestrator forever. Found because the
    #    monitor's coverage crossing started reporting the retired set as
    #    "subscribed here but not armed" — the gap was invisible until an
    #    instrument looked in both directions at once.
    #    ⇒ A death is a fact about the ROW, not about one watcher's bookkeeping;
    #    every plane that watches it has to hear the same answer.
    mon = STATE / "monitor" / f"{uuid}.json"
    if mon.exists():
        mon.unlink(missing_ok=True)
        log(f"  and dropped its monitor subscription — a corpse escalates to nobody")
    # ⛔⛔ AND THE THIRD PLANE — THE ONLY ONE HE CAN SEE.
    #
    #    The comment above argues the case and then stops one plane short:
    #    *a death is a fact about the ROW, not about one watcher's bookkeeping.*
    #    Booter and monitor are OUR bookkeeping. The sidebar is his. A seat that
    #    retired with both subscriptions dropped still sat in his sidebar
    #    forever, because `live_keep_alive` is true on every agent row — which
    #    is correct for an ordinary resumable session and wrong for a seat that
    #    has been declared dead with evidence.
    #
    #    ⇒ Owner-reported 2026-08-14: *"Why are 6.x predecessors not despawned
    #    including yours?"* Fifteen 6.x rows were listed, several of them seats
    #    whose scope was complete and whose processes were already reaped. The
    #    earlier fix taught this verb about the monitor; nobody taught it about
    #    the screen.
    _drop_row(uuid)
    return 0


def _row_is_live(row):
    """Is this row entry a LIVE session, as opposed to a durable on-disk one?

    The one owner of that question for this tool. `presence` is the field the
    GUI added so a consumer could tell dual presence from duplication:
    `live_rail` (the Live Sessions rail) and `cwd_tree` (the same live session
    in its cwd folder) are live; `row` is a durable session listed because its
    transcript is on disk and can be resumed.

    ⛔ Unknown/absent presence falls back to `live_member` and then to TRUE.
    A row we cannot classify must be treated as live: the cost of a false
    "live" is one harmless removal attempt, and the cost of a false "durable"
    is a genuine husk left in the sidebar reported as clean.
    """
    presence = row.get("presence")
    if presence in ("live_rail", "cwd_tree"):
        return True
    if presence == "row":
        return False
    live_member = row.get("live_member")
    return True if live_member is None else bool(live_member)


def _drop_row(uuid):
    """Remove a decided-dead row from the sidebar, on the host that renders it.

    ⚠ Best-effort by design: the ledger write above is the durable record and
    must not be undone by an unreachable GUI. A failure here is reported loudly
    and left for a human, never swallowed.

    ⛔ Two traps, both already paid for by this campaign:
    - **App control only resolves where the GUI runs**, and `retire` deliberately
      runs on the LOCAL host to write its ledger. So this resolves the GUI host
      itself rather than inheriting `args.host`.
    - **The verb reports the REQUEST, not the EFFECT** (`row_still_listed` is
      not `verified`), so the removal is confirmed by re-reading the rows and
      looking for the uuid, not by believing the reply.
    """
    try:
        host = resolve_gui_host(None)
    except Exception as exc:
        log(f"  ⚠ could not resolve the GUI host ({exc}) — ROW LEFT IN THE SIDEBAR")
        return
    if not host:
        log("  ⚠ no GUI host — ROW LEFT IN THE SIDEBAR, remove it by hand")
        return

    def row_paths():
        """The LIVE rows this uuid renders as — the only ones retirement removes.

        ⛔⛔ **A DURABLE ON-DISK SESSION IS NOT A ROW THAT SURVIVED ITS OWN
        RETIREMENT.** This asked only "does any listed row carry this session
        id", and `server app rows` answers with the whole tree: 66 live rows
        beside 448 durable listings on one measured host. A durable listing is
        the session's transcript being resumable in its cwd folder — the
        product's core value proposition, not a husk. `session remove` does not
        take it away, so the re-read found it again and the verb reported
        `⛔ ROW SURVIVED REMOVAL` on a reap that was clean, naming the one plane
        the owner actually looks at and telling the caller not to trust it.

        ⚠ **The alarm could never be cleared, and its only implied remedy —
        remove it by hand — was itself wrong**, because removing that entry
        deletes a resumable session from the tree. Same shape as the monitor's
        stand-down defect fixed in 5dcefb17: *a warning whose remedy another
        verb forbids is a defect in the warning.*

        ⚠ **And it was non-deterministic**: whether the durable listing had been
        rescanned into the tree yet decided whether the same clean reap printed
        "already gone" or the alarm. Two identical retirements, two verdicts,
        minutes apart (measured 2026-08-20).

        `presence` exists precisely so a consumer need not infer this — it is
        `live_rail`/`cwd_tree` for a live session and `row` for a durable one,
        and it agreed with `live_member` on every one of 514 measured rows. Both
        are read, and a row counts as live if EITHER says so: a missing field
        must not silently demote a live row to "nothing to remove here".
        """
        # A uuid is not an address: `session remove` takes the row's PATH.
        data = ygg(host, "server", "app", "rows") or {}
        rows = (data.get("data") or data).get("rows") or []
        return [r.get("full_path") or r.get("path") for r in rows
                if str(r.get("session_id") or "") == uuid
                and (r.get("full_path") or r.get("path"))
                and _row_is_live(r)]

    paths = row_paths()
    if not paths:
        log("  and its sidebar row was already gone")
        return
    # One session can legitimately render in several views, so remove every
    # path that names it rather than the first — removing one and reporting
    # success is how a row appears to survive its own retirement.
    for path in dict.fromkeys(paths):
        ygg(host, "server", "app", "session", "remove", path)
    still = row_paths()
    if still:
        log(f"  ⛔ ROW SURVIVED REMOVAL — {len(still)} path(s) still listed on {host}")
        log("     This is the plane he can see; do not report the retirement as clean.")
    else:
        log("  and removed its sidebar row — the plane he actually looks at")


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
        # ⛔⛔ NAME THE CONSEQUENCE, NOT JUST THE CONDITION. "QUOTA HOLD, seen on
        #    <row>" reads as a fact about THAT row, and a campaign checking its
        #    own seat concluded it was fine. The hold is FLEET-WIDE: while it is
        #    up, nothing can be delivered to anybody.
        # ⚠ A DECLARED hold has no row to name — `seen_on` is None — and this
        #   line indexed it unconditionally, so arming one CRASHED `list`, the
        #   fleet's main status verb, at the exact moment it had most to report.
        #   Same defect as the tick's RATE-HOLD line; fixed there first and not
        #   grepped for, which is how the second and third copies survived.
        why = (f"429 seen on {rl['seen_on'][:8]}" if rl.get("seen_on")
               else f"DECLARED: {rl.get('declared_reason') or 'no reason given'}")
        log(f"⏸ QUOTA HOLD {hold_remaining(rl)} "
            f"({why}) — ⛔ NO BOOT CAN BE DELIVERED TO "
            f"ANY ROW, INCLUDING YOURS, WHILE THIS IS UP")
    # ⭐ Name the directory this reads. A sibling campaign lost minutes concluding
    #   "no subscription file exists" while looking in the relay root — which also
    #   holds per-uuid .json files of a DIFFERENT kind, so the wrong directory
    #   looks like the right one with the answer missing.
    if not subs:
        log(f"no subscribers (reading {SUBS})")
        return 0
    lapsed = 0
    for s in subs:
        age_h = (time.time() - s["subscribed_at"]) / 3600
        # ⛔ A LAPSE MUST READ AS A LAPSE. Deleting the record made "it expired"
        #   and "it was never armed" the same observation.
        mark = ""
        if s.get("lapsed"):
            lapsed += 1
            mins = int((time.time() - s.get("lapsed_at", 0)) // 60)
            mark = (f"  ⛔ LAPSED {mins}m ago — {s.get('lapsed_reason', 'no reason recorded')}"
                    f" · NOT WATCHED; re-subscribe to clear")
        # ⛔⛔ THE SUPPRESSION MUST BE ON THE ROW'S OWN LINE, reported 2026-08-14
        #    by a campaign that had been reading its seat as healthy for hours.
        #    Everybody runs `list | grep <their-uuid>`, which throws away the
        #    header — so a row reads `boots=0`, its subscription JSON is perfect,
        #    its own tick prints ✅, and nothing can reach it. That is what makes
        #    a watchdog unfalsifiable, and it is how one outage ran 7.5 hours
        #    with every instrument green.
        held = "  ⏸ SUPPRESSED — fleet quota hold, no boot can be delivered" if rl else ""
        # ⛔⛔ AND THE STANDING REFUSAL GOES ON THIS LINE TOO, for the same reason
        #    the suppression does. It is recorded on the subscription — but a row
        #    refused on every tick still PRINTS as `boots=0` beside a healthy one,
        #    because a refused boot is refunded. The datum existing is not the
        #    same as anybody seeing it, and the header nobody reads is where it
        #    would otherwise live. A sibling lane lost ~7.8 hours reading exactly
        #    this line and concluding it was armed.
        rec = s.get("standing_refusal") or {}
        refused = ""
        if s.get("balance_suspended"):
            refused = ("  ⛔ SUSPENDED — refused for an exhausted CREDIT BALANCE; neither "
                       "waiting nor a boot can clear it (add credits, or switch model)")
        elif rec.get("ticks"):
            mins = int((time.time() - rec.get("since", time.time())) // 60)
            refused = (f"  ⚠ REFUSED {rec['ticks']}x for {mins}m ({rec.get('reason')})"
                       f" — armed, and NOT being woken")
        # ⭐ AND "IT WOKE" IS A THIRD FACT. `boots` is a stall counter that gets
        #    refunded; a row can be escalated for never waking while it reads 0.
        woke = "  ⚠ ESCALATED — boots delivered, no turn started" if s.get("escalated") else ""
        log(f"{s['uuid'][:8]}  {s.get('campaign') or '-':<12} "
            f"age={age_h:4.1f}h boots={s['boots']} {s['row']}{held}{refused}{woke}{mark}")
    log(f"{len(subs)} subscription(s) in {SUBS}"
        + (f" — ⛔ {lapsed} LAPSED and no longer watched" if lapsed else ""))
    return 0


def _run(host, argv, stdin_text, remote_binary=None):
    """Run a yggterm CLI verb with text on stdin, wherever the row lives.

    ⚠ The two binaries do NOT expose the same verbs. Locally this drives the
    HEADLESS binary; remotely it drives the GUI one, because that is the one
    that can reach the display plane. A verb that only the headless binary
    serves must say so with `remote_binary`, or it answers "unsupported"
    over ssh while working perfectly on this host — which is indistinguishable
    from the verb being missing everywhere."""
    if host == this_host():
        cmd = [str(Path.home() / ".local" / "bin" / "yggterm-headless"), *argv]
        return subprocess.run(cmd, input=stdin_text, capture_output=True,
                              text=True, timeout=180)
    joined = " ".join(f"'{a}'" for a in argv)
    binary = remote_binary or "$HOME/.yggterm/bin/yggterm"
    return subprocess.run(["ssh", host, f"{binary} {joined}"],
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

    ⚠ **THE APPEND HAZARD PAID OUT 2026-08-20 AND THE WINDOW IS NOW CLOSED** —
      the owner's half-typed sentence was submitted with the boot text spliced
      in. The text write asks the daemon (`--refuse-if-draft`), and the Enter is
      no longer a second unguarded write: `--submit-iff-line-equals` presses it
      only if the input line still reads exactly what we wrote, compared and
      enqueued under one lock in the daemon that owns the PTY.
      ⛔ THAT VERB HAS EXISTED SINCE 3.1.x. This docstring said it "is requested
      in pending-bugs" for as long as the gap it describes stayed open, and a
      stale claim about a missing tool is indistinguishable from the tool being
      missing. Re-test an inherited "not available" before building around it."""
    if dry:
        log(f"DRY-RUN would boot {row}")
        return "dry-run"
    # ⛔ A LIMIT-WAITING ROW IS NOT STALLED — its CLI's auto-continue is armed
    # and the turn resumes when the window opens, so a boot here types into a
    # composer that is about to be consumed by the continuation. The daemon
    # detects the wait footer per-CLI (3.1.13+) and this consumes ITS verdict
    # instead of re-deriving one from screen text; on older daemons the field
    # is absent and the guard is inert. Per-row and self-resolving: no fleet
    # hold, no escalation, no boot counted — just wait the window out.
    # (Ships while booters are HELD fleet-wide; the un-hold is its activation.)
    if _daemon_limit_wait(host, row):
        log(f"   ⏸ refusing the boot: the daemon reports this row is waiting "
            f"out a usage limit (auto-continue armed); it resumes by itself")
        return "refused-limit-wait"
    outcome = _pty_type_and_enter(host, row)
    if outcome:
        return outcome
    r = _run(host, ["server", "app", "terminal", "submit", row, "--stdin"], BOOT_TEXT)
    if _field(r.stdout or "", "submitted") is True:
        return "submit"
    return ""


CHOICE_PROMPT_MARKERS = (
    "hit your session limit", "session limit", "usage limit", "upgrade to max",
    "switch to a team account", "team account", "api billing", "pay per token",
    "select an option", "choose an option", "1. stop and wait", "stop and wait",
)


_CSI = re.compile(r"\x1b\[([0-9;]*)([A-Za-z])")
# The same grammar arriving as LITERAL TEXT — six characters "\u001b[…" rather
# than one escape byte — which is what a JSON envelope read raw (not parsed)
# hands over. See _plain_screen's second paragraph.
_CSI_LITERAL = re.compile(r"\\u001b\[([0-9;]*)([A-Za-z])")


def _plain_screen(text):
    """Screen bytes -> matchable text.

    ⛔ A RAW SCREEN DOES NOT CONTAIN THE WORDS YOU ARE LOOKING FOR. A terminal
    writes runs of spaces as CURSOR-FORWARD (`ESC[<n>C`) and sprays colour SGR
    mid-word, so `stop and wait` arrives as `stop\\x1b[Cand\\x1b[C\\x1b[1mwait`
    and a plain substring test finds nothing. A guard that silently matches
    nothing is worse than no guard: it reports "no prompt on screen" for a
    screen that is entirely a prompt.
    ⇒ Cursor-forward becomes a space (it IS whitespace on a rendered screen),
      every other CSI is dropped, and runs of blanks collapse.

    ⛔⛔ AND THE SAME GRAMMAR CAN ARRIVE AS LITERAL TEXT — measured 2026-08-21 on
    four wedged rows. The read-buffer arm's stdout is a JSON ENVELOPE; consumed
    raw, every escape byte is the six literal characters `\\u001b[…`, the real-
    byte regex above never fires, tokens split mid-word ("booter\\u001b[Cbooted"
    does not contain "booter boot"), the residue cleaner's length cap blows on
    the inflation, and the choice-prompt guard can silently miss a real billing
    prompt — the exact failure its own docstring warns about, alive through the
    other spelling. The primary fix is parsing the envelope (_screen_text);
    normalizing the literal form here as well means no future caller can
    reintroduce the hole by handing this function raw JSON.
    Literal `\\n`/`\\r`/`\\t` collapse with the blanks for the same reason."""
    def sub(m):
        return " " if m.group(2) == "C" else ""
    text = _CSI.sub(sub, text)
    text = _CSI_LITERAL.sub(sub, text)
    text = text.replace("\\n", " ").replace("\\r", " ").replace("\\t", " ")
    return re.sub(r"\s+", " ", text)


def _daemon_screen_text(host, row):
    """The DAEMON's own vt100 screen for one row, or None if it cannot look.

    Independent of the GUI binary's verb surface — the daemon owns the PTY, so
    this keeps working when the app-control arm the other reader uses is
    missing. `screen_available: false` is a real "could not look" and is
    reported as such rather than as an empty screen, because the caller
    distinguishes them and refuses on doubt."""
    uuid = row.rsplit("/", 1)[-1]
    # ⛔⛔ ASK THE ROW'S OWN MACHINE, NOT THE GUI HOST. `boot` is handed the GUI
    #    host because that is where a WRITE is proxied from -- but the daemon
    #    that owns this PTY, and therefore the only one holding its screen, is
    #    on the machine the row lives on. Asking the GUI host answered
    #    "unsupported" for every remote row, which reads exactly like the verb
    #    being missing everywhere and kept the whole fleet blind after the
    #    fallback was added. Caught by watching the log after the fix and
    #    finding it changed nothing.
    rhost = BB.row_host(row, host) or host
    r = _run(rhost, ["server", "gate-screen", f"cc-runtime://{uuid}",
                     "--tail", "60", "--json"], "",
             remote_binary="$HOME/.local/bin/yggterm-headless")
    try:
        entries = json.loads((r.stdout or "").strip() or "[]")
    except Exception:
        return None
    if not isinstance(entries, list):
        return None
    for e in entries:
        if not isinstance(e, dict):
            continue
        if (e.get("session_key") or "").rsplit("/", 1)[-1] != uuid:
            continue
        if not e.get("screen_available"):
            return None                  # it said plainly that it could not look
        # ⭐ PREFER THE RENDERED GRID. `screen_plain_rows` (3.1.21+) is the
        # daemon's own vt100 viewport, one entry per VISIBLE row; `screen_tail`
        # is the escape stream that paints it. A TUI draws with absolute cursor
        # moves and emits single spaces as cursor-forward, so on the stream a
        # modal's nine rows arrive as two lines and its heading is not present
        # as a substring at all — `_plain_screen` was written to paper over
        # exactly that, and it cannot recover the row boundaries the stream
        # never carried. Falls back on an older daemon, where the normalizer is
        # still the best available.
        rows = e.get("screen_plain_rows")
        if isinstance(rows, list) and rows:
            return "\n".join(rows)
        return "\n".join(e.get("screen_tail") or [])
    return None                          # the daemon has no screen for this row


def _daemon_limit_wait(host, row):
    """True ONLY when the row's own daemon says `shows_limit_wait: true`.

    The daemon (3.1.13+) detects the CLI's usage-limit wait footer per-CLI and
    publishes it on the gate-screen reading — the SSOT this replaces was our own
    python re-derivation of the same screen, which is the second-encoding shape
    the queue's limit-wait entry exists to end. A limit-waiting row has
    auto-continue armed and resumes by itself; typing into it is at best noise
    and at worst lands in the composer of a turn that is about to continue.

    ⛔ ANYTHING SHORT OF AN EXPLICIT true IS false — deliberately NOT the
    choice-guard's refuse-on-doubt. An older daemon's JSON simply lacks the
    field, and "the daemon is too old to say" must leave the boot behaviour
    exactly as it was, or shipping this guard would silently stop the whole
    fleet's wake plane on mixed versions. The choice guard refuses on doubt
    because its Enter can change billing; this one only defers a wake."""
    uuid = row.rsplit("/", 1)[-1]
    rhost = BB.row_host(row, host) or host
    r = _run(rhost, ["server", "gate-screen", f"cc-runtime://{uuid}",
                     "--tail", "1", "--json"], "",
             remote_binary="$HOME/.local/bin/yggterm-headless")
    try:
        entries = json.loads((r.stdout or "").strip() or "[]")
    except Exception:
        return False
    if not isinstance(entries, list):
        return False
    for e in entries:
        if not isinstance(e, dict):
            continue
        if (e.get("session_key") or "").rsplit("/", 1)[-1] != uuid:
            continue
        return e.get("shows_limit_wait") is True
    return False


def _daemon_row_state(host, row):
    """The row's STATE as the daemon names it, or None if it cannot say.

    ⭐ ONE OWNER FOR "WHAT IS THIS SCREEN". The daemon (3.1.21+) classifies from
    the rendered grid, where the words on the screen actually are, and publishes
    a single state slug with a documented precedence — a question picker outranks
    `working` because a picker IS mid-turn, a billing dialog outranks the
    limit-wait footer because their wording overlaps and only one of them is safe
    to leave alone. Re-deriving any of that here would be a second encoding of
    the one question this watcher must not get wrong.

    ⛔ None means "this daemon is too old to say", NEVER "nothing is holding the
    row". Every caller must keep its own refuse-on-doubt behaviour for that case.
    """
    uuid = row.rsplit("/", 1)[-1]
    rhost = BB.row_host(row, host) or host
    r = _run(rhost, ["server", "gate-screen", f"cc-runtime://{uuid}",
                     "--tail", "1", "--json"], "",
             remote_binary="$HOME/.local/bin/yggterm-headless")
    try:
        entries = json.loads((r.stdout or "").strip() or "[]")
    except Exception:
        return None
    if not isinstance(entries, list):
        return None
    for e in entries:
        if not isinstance(e, dict):
            continue
        if (e.get("session_key") or "").rsplit("/", 1)[-1] != uuid:
            continue
        state = e.get("state")
        return state if isinstance(state, str) and state else None
    return None


# ⛔ STATES A WRITER MUST NOT TYPE INTO, and WHY each one is here. The remedy and
# the prohibition ship together in `yggterm_core::screen_state`; this is the
# watcher's half of that contract — the subset where its remedy (type a message,
# then Enter) is the wrong act.
#   startup_gate      a modal reading single keys: a typed message is swallowed,
#                     and its Enter answers a question nobody read
#   plan_limit_choice a bare Enter SELECTS, and the options spend money
#   question_picker   the owner is being asked; typed text vanishes
#   limit_wait        the CLI resumes by itself; bytes land in the next turn
STATES_A_WATCHER_MUST_NOT_TYPE_INTO = frozenset({
    "startup_gate", "plan_limit_choice", "question_picker", "limit_wait",
})


def _screen_shows_a_choice(host, row):
    """TRI-STATE: True a prompt is on screen · False none · None could not look.

    ⛔⛔ THE DEFECT THIS EXISTS FOR, RAISED BY THE OWNER 2026-08-14. When the plan's
    limit is hit the CLI parks on a three-option prompt -- stop and wait, switch to
    a team account, or use API billing -- and the first is dismissed with Enter.
    `_pty_type_and_enter` finishes with a **lone `\\r`**, which does not "dismiss"
    anything: **it selects whatever option is highlighted.** If the highlight is not
    on the harmless one, a timer silently changes billing on the owner's account.
    Nobody decided that; a watchdog did.

    ⚠ `--refuse-if-draft` does NOT cover this. It guards a half-typed DRAFT, and a
    modal is not a draft — so the existing guard passes and the Enter still lands.

    ⚠ Nor does the `RATE_LIMITED` classifier: that keys on the CLI's own
    `apiErrorStatus: 429` record in the TRANSCRIPT, and a row parked on a modal has
    not necessarily written one. **The dialog is only visible on the SCREEN**, which
    is why this reads the screen and not the transcript.

    ⛔ REFUSE ON DOUBT. `None` (could not look) is treated as a refusal by the
    caller, for the same reason the never-arm ledger is: this thing types."""
    # ⭐ ASK THE DAEMON FIRST. It classifies from the rendered grid and pairs the
    # money wording with the STRUCTURAL test this function never had — a
    # selection marker sitting on a numbered option — which is what separates a
    # dialog awaiting a keypress from a footer merely reporting a limit. Only
    # when it cannot say do we fall back to matching phrases ourselves.
    state = _daemon_row_state(host, row)
    if state is not None:
        return state in STATES_A_WATCHER_MUST_NOT_TYPE_INTO
    body = _screen_text(host, row)
    if body is None:
        return None                      # could not look, either way
    low = _plain_screen(body).lower()
    return any(m in low for m in CHOICE_PROMPT_MARKERS)


def _screen_text(host, row):
    """One row's rendered screen text, or None if neither instrument can look.

    Two instruments, one source of truth (the PTY's screen):
    1. `server app terminal read-buffer` — the GUI arm. ⛔⛔ IT CAN VANISH, and
       when it did the whole fleet stopped waking: a refactor collapsed two
       per-binary `match` blocks into one and dropped the verb from the CLI
       dispatcher; the handler and its tests stayed, so the verb looked present
       in the source and was absent from every built binary. Measured
       2026-08-14 — no row on the fleet was booted for an hour.
    2. `_daemon_screen_text` — the daemon owns the PTY, so its vt100 screen is
       the more direct source anyway and does not depend on the GUI binary's
       verb surface. The fallback runs only when the first could not answer;
       refusing on doubt is the caller's job and is unchanged."""
    r = _run(host, ["server", "app", "terminal", "read-buffer", row,
                    "--mode", "screen"], "")
    body = (r.stdout or "")
    # ⛔ The stdout is a JSON ENVELOPE, and the screen lives in its `text`
    # field. Consuming the envelope raw hands every escape byte over as six
    # literal characters (`[…`), which defeated the residue cleaner and
    # blinded the choice-prompt guard on four rows at once (2026-08-21). Parse
    # it; fall back to the raw body only when it is not JSON at all, where the
    # literal-form normalization in _plain_screen still covers the match.
    if body.strip():
        try:
            envelope = json.loads(body)
            if isinstance(envelope, dict):
                body = envelope.get("text") or ""
        except ValueError:
            pass
    if not body.strip():
        body = _daemon_screen_text(host, row) or ""
    return body if body.strip() else None


# ⛔⛔ THE COMPOSER IS A ROW, AND THE BOOT TEXT LIVES IN THE TRANSCRIPT TOO.
# Measured 2026-08-21 across 19 rows and 434 consecutive refusals. The residue
# check flattened the WHOLE SCREEN to one line and asked whether the boot text
# stood after a `❯`. The agent CLI prefixes every DELIVERED message with the
# same glyph, so a boot that WORKED read back as composer residue for as long
# as it stayed on screen — and nothing clears a transcript, so the row refused
# every later boot forever. One row was made unbootable by each boot it had
# already accepted.
# ⇒ Read the composer ROW off the daemon's RENDERED GRID. A `❯` with prose
#   under it is a transcript entry; the composer is the bottom-most one, with
#   only the CLI's own border and footer below it.
COMPOSER_MARKERS = ("❯", "›")

# How far above the last chrome row the composer's marker may be. A composer
# wraps over a few rows when the line is long; anything deeper is transcript.
COMPOSER_WRAP_ROWS = 14


def _is_composer_chrome(row_text):
    """Rows that may sit BELOW the composer without making it a transcript entry."""
    t = row_text.strip()
    if not t:
        return True
    if not t.strip("─━│╭╮╰╯╱-=_ "):
        return True                      # a pure border run, drawn or ASCII
    low = t.lower()
    return any(h in low for h in (
        "bypass permissions", "shift+tab", "for shortcuts", "⏵⏵",
        "esc to interrupt", "auto-accept", "plan mode", "accept edits",
        "context left", "/clear to save", "new task?",
    ))


def _composer_row_content(host, row):
    """What the COMPOSER ROW holds. FOUR states, and they license opposite acts:

        (False, None)  could not look          -> refuse; blind is not clear
        (True,  None)  no composer on screen   -> refuse; mid-output or a modal
        (True,  "")    composer present, empty -> the only state that may be typed into
        (True,  "...") composer holds text     -> refuse; capture it, type nothing

    ⛔ THE GRID, NEVER THE STREAM. `screen_plain_rows` is the daemon's vt100
    viewport, one entry per VISIBLE row. The escape stream that paints it is not
    row-shaped: measured on this fleet, a 65-row screen arrived as FOUR
    newline-delimited lines, so "the last line beginning with the marker" is not
    a window over anything a person can see."""
    uuid = row.rsplit("/", 1)[-1]
    rhost = BB.row_host(row, host) or host
    r = _run(rhost, ["server", "screen", f"cc-runtime://{uuid}", "--json"], "",
             remote_binary="$HOME/.local/bin/yggterm-headless")
    try:
        entries = json.loads((r.stdout or "").strip() or "[]")
    except Exception:
        return (False, None)
    if not isinstance(entries, list):
        return (False, None)
    for e in entries:
        if not isinstance(e, dict):
            continue
        if (e.get("session_key") or "").rsplit("/", 1)[-1] != uuid:
            continue
        rows = e.get("screen_plain_rows")
        if not isinstance(rows, list) or not rows:
            return (False, None)         # it said plainly that it could not look
        return (True, _composer_from_grid(rows))
    return (False, None)


def _composer_from_grid(rows):
    """The composer row's content on a rendered grid, or None if none is drawn.

    Walks up from the bottom past the CLI's own chrome, then up through the
    composer's wrapped continuation rows to the marker. Stops at the first
    marker: a `❯` with prose still below it is a delivered message, not a
    composer, and returning None there refuses the boot rather than typing
    into a screen nobody can vouch for."""
    end = len(rows) - 1
    while end >= 0 and _is_composer_chrome(rows[end]):
        end -= 1
    if end < 0:
        return None                      # nothing but chrome: no composer drawn
    collected = []
    for idx in range(end, max(-1, end - COMPOSER_WRAP_ROWS), -1):
        text = rows[idx].strip().lstrip("│ ")
        for marker in COMPOSER_MARKERS:
            if text.startswith(marker):
                collected.append(text[len(marker):])
                collected.reverse()
                return re.sub(r"\s+", " ", " ".join(collected)).strip()
        collected.append(text)
    return None                          # no marker within reach: transcript


# ⭐ THE WRITE LEDGER — one file per row, written BEFORE the bytes go out.
#
# ⛔⛔ THE STORM THIS ENDS, measured 2026-08-21: the booter typed its wake text,
# could not confirm it on screen, refused to press Enter ("residue self-heals
# next tick"), and then TYPED ANOTHER COPY next tick — because both decisions
# read the same failing detector, and "I cannot see it" licensed *do not
# submit* and *type again* at once. Two rows were found holding a dozen
# unsent copies filling the viewport, cleared by hand.
# ⇒ A WRITER THAT CANNOT CONFIRM ITS OWN SUBMIT MUST NOT WRITE AGAIN. The
#   ledger survives the tick, so the next pass COMPLETES the pending write
#   with an atomic submit or refuses; it never re-types.
def _pending_write_path(row):
    d = os.path.join(STATE, "booter", "pending-write")
    os.makedirs(d, exist_ok=True)
    return os.path.join(d, f"{row.rsplit('/', 1)[-1]}.json")


def _pending_write(row):
    try:
        with open(_pending_write_path(row)) as f:
            rec = json.load(f)
        return rec if isinstance(rec, dict) and rec.get("text") else None
    except Exception:
        return None


def _record_pending_write(row, text):
    try:
        with open(_pending_write_path(row), "w") as f:
            json.dump({"text": text, "at": time.time(),
                       "row": row, "attempts": 1}, f)
    except Exception as e:
        # ⛔ A ledger that cannot be written must STOP the write, not accompany
        # it. Typing without the record is exactly the storm shape again.
        log(f"  ⛔ could not record the pending write for {row}: {e}")
        return False
    return True


def _clear_pending_write(row):
    try:
        os.unlink(_pending_write_path(row))
    except FileNotFoundError:
        pass
    except Exception as e:
        log(f"  ⚠ could not clear the pending write for {row}: {e}")


def _atomic_submit(host, row, text):
    """Press Enter IFF the composer's line is exactly `text`. TRI-STATE.

    ⭐ `--submit-iff-line-equals` is a DAEMON verb and has shipped since 3.1.x.
    This file carried a comment saying the atomic form "needs a daemon-side
    verb — requested via pending-bugs" long after it existed, and that stale
    claim is why the two-write submit kept its unguarded gap. The daemon holds
    the input line under the same lock it enqueues the Enter with, so a
    keystroke can never land between the comparison and the submit.

    ⛔ `accepted:true` IS NOT PROOF HERE. A daemon that does not own the row
    never evaluates the condition, and the envelope's `accepted` is then true
    for a submit that never happened. The daemon's own message is the answer."""
    uuid = row.rsplit("/", 1)[-1]
    rhost = BB.row_host(row, host) or host
    r = _run(rhost, ["server", "terminal", "write", f"cc-runtime://{uuid}",
                     "--submit-iff-line-equals", text], "",
             remote_binary="$HOME/.local/bin/yggterm-headless")
    out = r.stdout or ""
    if _field(out, "submitted") is True:
        return True
    if _field(out, "refused_for_line") is True:
        return False
    message = _field(out, "message") or ""
    if isinstance(message, str) and message.startswith("submitted:"):
        return True
    if isinstance(message, str) and "line" in message and "expected" in message:
        return False
    return None                          # the daemon could not say


def _pty_type_and_enter(host, row, text=None):
    """Deliver `text` to a row's composer and submit it — AT MOST ONCE.

    ⭐ `text` IS A PARAMETER BECAUSE THIS IS THE FLEET'S ONLY GUARDED WRITER, AND
    THE OTHER WATCHDOG HAD NONE. The monitor's `wake()` typed with a bare
    `terminal send` plus a lone `\\r`: no screen read, no choice-prompt refusal,
    no draft guard, no verify-before-Enter. So the two watchdogs typed into the
    SAME rows with opposite levels of care, and the careless one was aimed at
    orchestrators. One guarded path, parameterised, rather than a second copy of
    five guards that would drift.

    THE SEQUENCE, and every step refuses rather than guesses:

    1. the row's STATE, from the daemon — a modal reads single keys and a bare
       Enter SELECTS, so a watcher must not type into one;
    2. the COMPOSER ROW, from the rendered grid — the only state that may be
       typed into is a composer that is present and EMPTY;
    3. a PENDING WRITE from an earlier tick is COMPLETED, never repeated;
    4. the write, recorded before it is sent;
    5. the ATOMIC submit, which presses Enter only if the line is still exactly
       what we wrote — so a sentence that raced us cannot be submitted as ours.

    ⛔ There is no path here that types twice. The bytes we could not confirm
    are the bytes we must not send again."""
    text = BOOT_TEXT if text is None else text
    short = row.rsplit("/", 1)[-1][:8]
    # ⛔⛔ STATE FIRST. The Enter below SELECTS a highlighted option, so a row
    # parked on a choice prompt must never be typed into. Refuse on doubt.
    choice = _screen_shows_a_choice(host, row)
    if choice is True:
        log(f"⛔ NOT BOOTING {short} — its SCREEN is showing a prompt awaiting a "
            f"choice (plan limit / billing). A bare Enter here SELECTS an option; "
            f"that decision is the owner's, not a timer's.")
        return "refused-choice-prompt"
    if choice is None:
        log(f"⛔ NOT BOOTING {short} — could not read its screen, so it cannot be "
            f"ruled out that a prompt is waiting. Blind is not clear.")
        return "refused-screen-unreadable"
    readable, composer = _composer_row_content(host, row)
    if not readable:
        log(f"⛔ NOT BOOTING {short} — could not read its composer row. "
            f"Blind is not clear.")
        return "refused-screen-unreadable"
    if composer is None:
        log(f"⛔ NOT BOOTING {short} — no composer is drawn on its screen "
            f"(mid-output, or a modal). Nothing here may be typed into.")
        return "refused-no-composer"
    pending = _pending_write(row)
    if pending:
        # ⛔ COMPLETE IT OR REFUSE IT. Never a second copy.
        done = _atomic_submit(host, row, pending["text"])
        if done is True:
            _clear_pending_write(row)
            log(f"⭐ COMPLETED the pending write to {short} with an atomic submit "
                f"— the line still held exactly what we wrote.")
            return "pty-write"
        if not composer:
            # The composer is empty, so our bytes are not standing in it: the
            # write evaporated (a restart, a CLI clear). The ledger is stale and
            # a fresh attempt is honest — but it starts from zero copies.
            _clear_pending_write(row)
            log(f"⚠ the pending write to {short} is no longer in its composer; "
                f"the ledger is cleared and this tick starts a fresh single write.")
        else:
            _capture_draft(row, composer, text[:27], text)
            log(f"⛔ NOT BOOTING {short} — a write from an earlier tick could not "
                f"be confirmed and the composer no longer holds exactly it. "
                f"Captured to the draft store; typing nothing. "
                f"[[a writer that cannot confirm its own submit must not write again]]")
            return "refused-unconfirmed-write"
        pending = None
    if composer:
        # Somebody else's words, or our own unconfirmed ones. Either way this
        # writer does not get to add to them.
        _capture_draft(row, composer, text[:27], text)
        log(f"⛔ NOT BOOTING {short} — its composer already holds text "
            f"({len(composer)} chars). Captured to the draft store; typing nothing.")
        return "refused-draft"
    if not _record_pending_write(row, text):
        return "refused-no-ledger"
    typed = _run(host, ["server", "terminal", "write", row, "--stdin",
                        "--refuse-if-draft"], text)
    if _field(typed.stdout or "", "refused_for_draft") is True:
        _clear_pending_write(row)
        _capture_draft(row, composer, text[:27], text)
        return "refused-draft"
    if _field(typed.stdout or "", "accepted") is not True:
        _clear_pending_write(row)
        return ""
    done = _atomic_submit(host, row, text)
    if done is True:
        _clear_pending_write(row)
        return "pty-write"
    # ⛔ THE LEDGER STAYS. Whatever happened, the next tick COMPLETES this write
    # or refuses it; it does not type a second copy.
    why = ("the line no longer reads exactly what we wrote"
           if done is False else "the daemon could not answer")
    log(f"⛔ ENTER NOT SENT to {short} — {why}. The write is recorded as pending; "
        f"the next tick completes it or refuses. Nothing will be typed twice.")
    return "refused-submit-unconfirmed"


def _capture_draft(row, plain_screen, boot_head, full_text=None):
    """Owner design 2026-08-20: a draft the booter meets is NEVER lost — store it
    durably so the boot (or a handover) can re-deliver it. 'If I type something
    the booter should store it … and in a handover my prompt must be handed to
    the next spawnee too. So there is no data loss.'

    Best-effort extraction: the text after the LAST prompt marker, minus any
    boot-text copies. Empty extractions are not written. The consumer is the
    relay protocol (fleet SKILL §8): a successor's spawner checks
    ~/.yggterm/relay/drafts/<uuid>.txt and re-types it into the fresh composer."""
    try:
        uuid = row.rsplit("/", 1)[-1]
        seg = plain_screen
        m = max(seg.rfind("❯"), seg.rfind("> "))
        if m >= 0:
            seg = seg[m + 1:]
        for chunk in (full_text or BOOT_TEXT, BOOT_TEXT, boot_head):
            seg = seg.replace(_plain_screen(chunk), " ").replace(chunk, " ")
        seg = re.sub(r"\s+", " ", seg).strip()
        if not seg:
            return
        d = os.path.join(STATE, "drafts")
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, f"{uuid}.txt"), "a") as f:
            f.write(f"--- {time.strftime('%Y-%m-%dT%H:%M:%S')} captured by the booter\n{seg}\n")
        log(f"  ⭐ draft captured for {uuid[:8]} → relay/drafts/{uuid}.txt ({len(seg)} chars)")
    except Exception as e:
        log(f"  ⚠ draft capture failed for {row}: {e} — refusing the boot is unchanged")


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
        if s.get("lapsed"):
            # Already reported once; stay quiet but stay VISIBLE in `list`.
            continue
        if s.get("max_hours") and age_h > s["max_hours"]:
            # ⛔⛔ THE SECOND SILENT-UNLINK PATH, and the likelier cause of the
            # nine-hour outage: a campaign found all three of its subscriptions
            # simply absent, with max_hours=12.0 and ~10.6h elapsed, and could not
            # tell expiry from loss because BOTH leave the same nothing. An expiry
            # is a legitimate decision; deleting the evidence of it is not.
            s["lapsed"] = True
            s["lapsed_at"] = int(time.time())
            s["lapsed_reason"] = f"max_hours {s['max_hours']} exceeded at {age_h:.1f}h"
            log(f"{uuid[:8]} LAPSED — EXPIRED after {age_h:.1f}h (max_hours "
                f"{s['max_hours']}); keeping the record so this is visible. "
                f"Re-subscribe to clear it.")
            if not args.dry_run:
                update_sub(uuid, s)
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
                # ⛔⛔ LAPSE LOUDLY. This used to DELETE the record, and an arm that
                # lapses then becomes indistinguishable from an arm that was never
                # set — the same failure signature as the thing this tool exists to
                # eliminate. Reported 2026-08-14 by a campaign that lost all three
                # of its subscriptions overnight: a seat hit its plan's session
                # limit at 01:44, the limit reset at 02:20, nothing woke it, and it
                # was found dead at 10:49 with a release unshipped. Nine hours. The
                # subscriptions had not been refused, they had simply gone, and
                # `list` showed an absence rather than a story.
                # ⇒ Keep the record, mark it, and let `list` show it. A subscriber
                #   that can vanish without a trace is not a safety net.
                s["lapsed"] = True
                s["lapsed_at"] = int(time.time())
                s["lapsed_reason"] = (f"absent from {GONE_SIGHTINGS} consecutive row "
                                      f"listings on host {host!r}")
                log(f"{uuid[:8]} LAPSED — absent from {GONE_SIGHTINGS} consecutive "
                    f"row listings; keeping the record so this is visible. "
                    f"Re-subscribe to clear it.")
                if not args.dry_run:
                    update_sub(uuid, s)
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
            # ⛔⛔ THE THIRD SILENT-UNLINK PATH, AND IT SURVIVED THE FIX TO THE
            # OTHER TWO. GONE and EXPIRED were both taught to lapse loudly on
            # 2026-08-14 because a subscription that vanishes without a trace is
            # indistinguishable from one that was never set. This one does the
            # same thing for the same reason and was left deleting the record —
            # the shape where one case is handled and its sibling is not, which
            # this fleet has now paid for four times in a week.
            # ⇒ Context exhaustion is the MOST common death here, so the path
            #   most likely to erase the evidence was the one still doing it.
            # ⚠ Lapse rather than retire: the SESSION is unrecoverable, but the
            #   ROW is routinely re-claimed by a successor, and re-subscribing
            #   clears the lapse and reports how long it went uncovered.
            s["lapsed"] = True
            s["lapsed_at"] = int(time.time())
            s["lapsed_reason"] = "context exhausted and unrecoverable; escalated"
            log(f"{uuid[:8]} LAPSED — CONTEXT-DEAD (a boot cannot fix this); "
                f"keeping the record so this is visible. Re-subscribe to clear it.")
            if not args.dry_run:
                update_sub(uuid, s)
            continue
        # ⛔⛔ A QUOTA MESSAGE WHOSE OWN RESET TIME HAS PASSED IS HISTORY, NOT A
        #    LIVE LIMIT — demote it to IDLE *before* dispatch, or the row stays
        #    parked forever. `RATE_LIMITED` is deliberately never booted (correct:
        #    do not boot into a live wall), but the classification is read off a
        #    frozen tail, and a parked row never writes anything to change it. So
        #    a row that hit a limit ONCE is skipped on every tick thereafter.
        #    ⇒ Measured by a sibling campaign: a subscription present the whole
        #    time, a reset 74 minutes past, `boots=0`, and a live runtime that a
        #    single carriage return then woke on the first try. The row was
        #    perfectly wakeable; nothing ever asked it.
        # ⚠ Typing here is still guarded and that is what makes this safe: the
        #    boot reads the SCREEN first and refuses on a choice prompt or an
        #    unreadable screen, so a row sitting on the plan-limit DIALOG is
        #    still never typed into, and `--refuse-if-draft` still protects a
        #    half-typed sentence.
        if state == "RATE_LIMITED" and tail_reset_has_passed(c.get("tail")):
            log(f"{uuid[:8]} quota message is HISTORY — its own reset time has "
                f"passed and nothing has been written since. Treating as IDLE so "
                f"it can be woken rather than skipped forever.")
            state = "IDLE"

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
                # ⛔ A DRY RUN MUST NOT MUTATE, and this path did both things a
                # dry run is promised not to do: it appended to the retired
                # ledger and deleted the subscription. Every other mutation in
                # this tick is guarded; this one was missed, so `tick --dry-run`
                # — the command someone reaches for precisely BECAUSE they are
                # unsure — was the one that quietly retired rows.
                if not args.dry_run:
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
            # ⛔⛔ AN EXHAUSTED BALANCE IS THIS ROW'S PROBLEM, NOT THE FLEET'S.
            #    Suspend ONE row, tell a human ONCE, and leave every other
            #    campaign wakeable — see `refusal_is_a_balance_not_a_window`.
            #    ⛔ It cannot expire on a timer, for exactly the reason it was not
            #    armed on one: nothing about waiting restores a balance. It ends
            #    when the row WRITES SOMETHING NEW, which is the same
            #    anti-stale-artefact discipline `note_rate_limit` uses — a frozen
            #    tail is evidence about a moment, never about now.
            if refusal_is_a_balance_not_a_window(c.get("tail")):
                marker = _evidence_marker(uuid)
                if marker is not None and s.get("balance_marker") != marker:
                    s["balance_marker"] = marker
                    s.pop("balance_escalated", None)   # a NEW refusal is news
                s["balance_suspended"] = True
                s["balance_since"] = s.get("balance_since") or int(time.time())
                if not s.get("balance_escalated"):
                    rc = max(rc, 4)
                    escalate(host, row,
                             "refused for an exhausted CREDIT BALANCE, not a timed quota "
                             "window. No timer clears this and no boot can: it needs "
                             "credits added or a different model. This row is suspended "
                             "until it writes again; the rest of the fleet is NOT held.")
                    s["balance_escalated"] = True
                note_standing_refusal(s, "balance-exhausted")
                if s.get("boots"):
                    s["boots"] -= 1
                action = "SUSPENDED:balance"
                log(f"{'BALANCE':<14} {c['age'] / 60:>6.1f}m  {action:<12} {uuid[:8]}  "
                    f"refused for exhausted credits, which no wait can fix — holding THIS "
                    f"ROW only; the fleet stays wakeable")
                if not args.dry_run:
                    update_sub(uuid, s)
                continue
            # A LIVE row refused on a timed WINDOW is the real account-wide
            # thing: hold the FLEET, do not escalate (a human cannot grant
            # quota), and do not unsubscribe — unlike CONTEXT_DEAD this ends by
            # itself, and the row is meant to still be watched when it does.
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
            # The row moved, so whatever was standing in the way no longer is.
            clear_standing_refusal(s)
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
                clear_standing_refusal(s)
            if rl:
                # ⛔ A BOOT INTO AN EXHAUSTED QUOTA IS REFUSED BEFORE THE AGENT
                #    RUNS. It spends the wake, changes nothing, and — because the
                #    refusal grows the transcript — looks enough like activity to
                #    keep the loop going. So: skip, do NOT count it as a boot
                #    attempt, and say WHY in the log, because "nothing happened
                #    for an hour" with no line explaining it is how a watchdog
                #    becomes unfalsifiable.
                action = "HOLD:rate-limit"
                held = hold_remaining(rl)
                # ⚠ `seen_on` exists only on a hold armed by a TAIL SIGHTING. A
                #   declared hold has no row to name, and indexing it blindly
                #   turned the suppression path into a crash on the one code
                #   path whose whole job is to do nothing quietly.
                why = (f"seen on {rl['seen_on'][:8]}" if rl.get("seen_on")
                       else f"declared: {rl.get('declared_reason') or 'no reason given'}")
                log(f"{'RATE-HOLD':<14} {c['age'] / 60:>6.1f}m  {action:<12} {uuid[:8]}  "
                    f"quota hold {held} ({why})")
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
                if via in ("refused-draft", "refused-choice-prompt",
                           "refused-screen-unreadable", "refused-draft-race",
                           "refused-limit-wait", "refused-no-composer",
                           "refused-unconfirmed-write", "refused-submit-unconfirmed",
                           "refused-no-ledger"):
                    # ⛔ A refusal is NOT a failed boot and must not count as one.
                    # The row is idle because its owner is mid-sentence, which is
                    # the one state where booting is worse than waiting — so give
                    # the attempt back, keep the deferral, and try again next
                    # tick. ⚠ Do NOT `continue` here: the state write and the
                    # window log at the bottom of the loop are what make a
                    # skipped row visible instead of merely absent.
                    # ⛔⛔ THE OTHER TWO REFUSALS WERE MISSING FROM THIS LIST AND
                    #    THAT COST A LANE ITS WHOLE BUDGET. A row parked on the
                    #    plan-limit dialog is refused by OUR OWN screen guard on
                    #    every tick — and each refusal was counted as a boot, so
                    #    after MAX_BOOTS the watchdog escalated "did not wake
                    #    after N boots" about a session **nobody ever asked
                    #    anything**. Measured 2026-08-14 on the lane holding the
                    #    owner's top item.
                    # ⭐ The rule the original comment states is the right one; it
                    #    was simply applied to one refusal out of three. **If the
                    #    guard stopped the write, the row was never asked** —
                    #    which is true of a draft, a choice prompt, and an
                    #    unreadable screen alike.
                    s["boots"] -= 1
                    # ⛔⛔ `.get`, NOT `[via]`, AND THE DEFAULT IS DERIVED — this
                    #    lookup used to be an exhaustive dict over a set defined
                    #    two thousand lines away in `boot()`, and it DRIFTED: a
                    #    fifth refusal (`refused-limit-wait`) was added to the
                    #    membership test above without being added here, so the
                    #    first limit-waiting row to come due would raise KeyError
                    #    out of the per-row loop, out of `tick()`, past the
                    #    `finally` that removes the pidfile, and kill the watcher
                    #    for the whole host. Latent, never fired, and it would
                    #    have fired on the ordinary event of a session running out
                    #    of quota. ⇒ A label is cosmetic; never let one be able to
                    #    stop the loop that types.
                    action = {"refused-draft": "SKIP:drafting",
                              "refused-choice-prompt": "SKIP:choice-prompt",
                              "refused-screen-unreadable": "SKIP:blind",
                              "refused-draft-race": "SKIP:draft-race",
                              "refused-limit-wait": "SKIP:limit-wait",
                              }.get(via, f"SKIP:{via.removeprefix('refused-')}")
                    # ⛔⛔ THE MIRROR IMAGE OF THE DOUBLE-CHARGE ABOVE, AND IT IS
                    #    WORSE: a refund means `boots` never rises, so the row
                    #    can never reach MAX_BOOTS, so it can NEVER ESCALATE. A
                    #    row refused forever is a row silent forever, and the
                    #    condition was invisible to every instrument except this
                    #    log line. Measured 2026-08-14: a lane slept through a
                    #    hard external deadline while the watchdog refused it
                    #    every tick and told nobody. Measured again 2026-08-21,
                    #    the other reason and the expensive one: FOUR rows across
                    #    four campaigns stood refused for DAYS on boot-text
                    #    residue — a condition that cannot clear itself, because
                    #    the only thing that could clear it is the boot being
                    #    refused.
                    #
                    # ⚖ THE REFUND STAYS CORRECT — the row was never asked, so
                    #   charging it a wake would be a lie. What was missing is
                    #   that the SILENCE is the defect, not the refusal.
                    #
                    # ⇒ So the run is counted on the subscription, where `list`
                    #   and `status` can show it, and told to a human exactly
                    #   once per condition. The refusal itself is untouched: this
                    #   arm still does NOT type. "Blind is not clear" remains the
                    #   right rule for WRITING into a row.
                    #
                    # ⭐ THE THRESHOLD DIFFERS BY REASON, and the reasons are not
                    #   symmetric. An unreadable screen is an observation of OUR
                    #   OWN INSTRUMENT and does not clear itself, so it stays
                    #   bounded tight at MAX_BOOTS. A draft or a choice prompt is
                    #   an observation of the ROW — something is genuinely in
                    #   front of it and waiting is usually right — so it gets an
                    #   hour before anyone is paged. A limit wait is exempt
                    #   entirely and says so in the table.
                    rec = note_standing_refusal(s, via)
                    after = STANDING_REFUSAL_ESCALATE_AFTER.get(
                        via, STANDING_REFUSAL_TICKS)
                    if after is not None and rec["ticks"] >= after and not rec["escalated"]:
                        rc = max(rc, 4)
                        held = (time.time() - rec["since"]) / 60
                        if via == "refused-screen-unreadable":
                            why = (f"screen unreadable for {rec['ticks']} ticks "
                                   f"({c['age']/60:.0f} min idle) — the guard cannot rule "
                                   f"out a waiting prompt, so it will not boot this row. "
                                   f"This is our instrument failing, not the row: check "
                                   f"that the running build still exposes the screen-read "
                                   f"verb.")
                        else:
                            why = (f"refused {rec['ticks']} consecutive ticks for the same "
                                   f"reason ({via}) over {held:.0f} min — the refusal is "
                                   f"correct and is NOT being relaxed; the standing "
                                   f"condition is the defect. Nothing clears this by "
                                   f"itself: the row is alive, is not being woken, and no "
                                   f"counter rises to say so. If the composer holds "
                                   f"exactly our own boot text, `ygg-booter.py unjam "
                                   f"<row>` delivers or clears it; it refuses "
                                   f"anything that is not character-for-character "
                                   f"ours.")
                        escalate(host, row, why + " The row is NOT being woken by anything.")
                        rec["escalated"] = True
                        action = f"{action}→ESCALATED"
                else:
                    # Say WHICH door delivered it. A watchdog that reports
                    # "booted" without saying how cannot be debugged when it
                    # silently stops.
                    action = f"BOOT#{s['boots']}:{via or 'NOT-DELIVERED'}"
                    # The write went through, so nothing is standing in the way
                    # any more — a run of refusals ended by a delivered boot must
                    # not keep counting toward an escalation about a condition
                    # that is over.
                    if via:
                        clear_standing_refusal(s)
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


def _source_is_current():
    """(ok, detail): is THIS copy of the booter the one on `origin/main`?

    ⛔⛔ THE DEFECT THIS EXISTS FOR, measured 2026-08-14. `ensure_watcher`
    spawns the watcher from `HERE` -- the copy of whichever checkout happened to
    run a `subscribe`. There are fourteen checkouts of this repo on one host and
    NINE of them carried a superseded booter at the moment this was written, so
    any of them arming the fleet installs old supervision code, silently, and
    the fleet then runs whichever copy was launched last rather than the newest.
    It is not theoretical: minutes after the screen-read outage was fixed, a
    watcher respawned from a stale checkout and the fleet stayed blind.

    ⚠ AND THE STALE PATH IS THE HABITUAL ONE. The copy an agent invokes by hand
    is the one in its own cwd and its own shell history; the copy that is
    actually supervising is discoverable only from /proc. Three sessions were
    caught by this in one afternoon, INCLUDING two that had the identify-which-
    copy-is-executing law in memory at the time. Knowing the law does not
    protect you, so the tool has to say it.

    ⚖ Compares against the last-fetched `origin/main` and does NOT fetch: this
    runs at watcher startup, and a network call there would make arming depend
    on the network. A stale answer here is still worth having -- the failure is
    a copy that is hours behind, not seconds."""
    src = Path(__file__).resolve()
    try:
        top = subprocess.run(["git", "-C", str(src.parent), "rev-parse",
                              "--show-toplevel"], capture_output=True,
                             text=True, timeout=20).stdout.strip()
        if not top:
            return (None, "not inside a git checkout")
        rel = str(src.relative_to(Path(top)))
        r = subprocess.run(["git", "-C", top, "show", f"origin/main:{rel}"],
                           capture_output=True, timeout=20)
        if r.returncode != 0:
            return (None, f"origin/main carries no {rel} to compare against")
        mine = hashlib.sha256(src.read_bytes()).hexdigest()[:12]
        theirs = hashlib.sha256(r.stdout).hexdigest()[:12]
        return (mine == theirs,
                f"this copy {mine} ({top}) vs origin/main {theirs}")
    except Exception as exc:                     # never block arming on this
        return (None, f"could not compare: {exc}")


def _source_digest():
    """sha256 of the file this process is EXECUTING, or None if it cannot be read.

    ⛔ `None` is "cannot tell", never "unchanged" — a branch switch can take the
    file out from under a running watcher, and a missing file must not read as
    a match."""
    try:
        return hashlib.sha256(Path(__file__).resolve().read_bytes()).hexdigest()
    except OSError:
        return None


def _reexec_if_source_changed(baseline):
    """Re-exec into the current source when this file changes under us.

    ⛔⛔ THE WATCHER IS A LOOP NOBODY RESTARTS, AND `_warn_if_stale_source` ONLY
    EVER FIRES AT STARTUP — i.e. at the one moment it is least likely to be true.
    A watcher that starts current and is superseded twenty minutes later reports
    a clean source in its log forever, and every fix shipped to this file is
    inert until a human happens to notice.

    ⇒ Measured 2026-08-21. The watcher came up at 17:52 from a checkout that was
    current. The balance/window split landed at 18:12. At 19:45 the fleet was
    still fully blacked out behind one row's exhausted credit balance, 23
    subscribers unwakeable, because the process was running code from twenty
    minutes before the fix — with `source:` printed in its own log and the
    startup staleness check reporting nothing wrong. Restarting it by hand fixed
    the fleet in one tick, which is the whole argument: the fix existed, on disk,
    in the right checkout, and nothing carried it into the running process.

    ⭐ `os.execv` KEEPS THE PID, so the pidfile, the heartbeat's `pid`, and every
    `watcher_alive()` reader stay correct across the swap. There is deliberately
    no handover dance: all of this loop's state is on disk (subscriptions, write
    ledger, holds), which is what makes replacing the code between ticks safe.

    ⛔ It compiles the new source before exec-ing into it. Fourteen checkouts of
    this repo share one host and a `git pull` is not the only way this file
    changes; exec-ing into a half-written file would take the fleet's watchdog
    down with a SyntaxError nobody is watching for. A file that does not compile
    is left alone and retried next tick.
    """
    current = _source_digest()
    if current is None or current == baseline:
        return
    src = Path(__file__).resolve()
    try:
        compile(src.read_bytes(), str(src), "exec")
    except (SyntaxError, ValueError, OSError) as exc:
        log(f"⚠ this booter's source changed ({baseline[:12]} -> {current[:12]}) "
            f"but the new copy does not compile ({exc}) — staying on the old "
            f"code and retrying next tick. Somebody may be mid-edit.")
        return
    log(f"⭐ SOURCE CHANGED UNDER THIS WATCHER ({baseline[:12]} -> "
        f"{current[:12]}) — re-exec-ing into it now, same pid {os.getpid()}. "
        f"A fix shipped to {src} is live from the next tick; nobody has to "
        f"remember to restart the loop.")
    sys.stdout.flush()
    sys.stderr.flush()
    os.execv(sys.executable,
             [sys.executable, str(src), "watch",
              "--host", _REEXEC_ARGS["host"],
              "--interval", str(_REEXEC_ARGS["interval"])])


# The argv the loop must come back up with after a re-exec. Written once by
# `cmd_watch` so the swap cannot silently drop a flag it was started with.
_REEXEC_ARGS = {"host": None, "interval": None}


def _warn_if_stale_source(where):
    ok, detail = _source_is_current()
    if ok is False:
        log(f"⛔⛔ STALE BOOTER — {where} is running a copy that does NOT match "
            f"origin/main. {detail}. It will supervise the fleet with whatever "
            f"this checkout last pulled; a fix that has shipped is not "
            f"necessarily in it. Pull this checkout, or arm from one that is "
            f"current, and keep exactly ONE watcher.")
    elif ok is None:
        log(f"⚠ could not tell whether {where}'s booter is current: {detail}")
    return ok


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
    # ⛔ Say it BEFORE spawning, while the agent arming the fleet is still here
    #    to read it. After the spawn this is a line in a log nobody opens.
    _warn_if_stale_source("the checkout arming this watcher")
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
    # ⛔ WHICH COPY IS SUPERVISING? Recorded at startup so the log answers it
    #    without anyone having to read /proc — the question that cost three
    #    sessions an afternoon.
    _warn_if_stale_source(f"this watcher (pid {os.getpid()})")
    log(f"  source: {Path(__file__).resolve()}")
    # ⛔ The startup check above answers "was this copy current when I started".
    #    This one answers "is the code I am RUNNING still the code on disk",
    #    which is the question that actually costs the fleet — see
    #    `_reexec_if_source_changed`.
    _REEXEC_ARGS["host"] = args.host
    _REEXEC_ARGS["interval"] = args.interval
    source_baseline = _source_digest()
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
            # ⛔ BETWEEN TICKS, never inside one: a tick holds the write ledger's
            #    invariant across a boot, and swapping the code mid-boot is the
            #    one moment where "all state is on disk" stops being true.
            if source_baseline is not None:
                _reexec_if_source_changed(source_baseline)
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
                 f"({hold_remaining(rl)})")
        why = (f"quota refusal last seen on {rl['seen_on'][:8]}" if rl.get("seen_on")
               else f"DECLARED: {rl.get('declared_reason') or 'no reason given'}")
        hold = (f" · {why}; "
                f"{len(load_subs())} subscriber(s) are unwakeable meanwhile")
    log(f"watcher: {'alive pid ' + str(alive) if alive else 'NOT RUNNING'} · "
        f"{armed} · heartbeat {hb} · subscribers {len(load_subs())}{hold}{mute}")
    return 0 if (alive and not mute) else 1


# The one control byte this file may send into a composer, named once.
COMPOSER_CLEAR_LINE = "\u0015"          # Ctrl+U — clears the composer line


def cmd_unjam(args):
    """Recover a row jammed by OUR OWN unsent boot text. Refuses anything else.

    ⛔⛔ THE HOLE THIS FILLS HAD TWO SIGNPOSTS AND NO FLOOR. The escalation for a
    standing `refused-draft` said *"for boot-text residue try `ygg-unwedge.sh`"*;
    that script's own header says it is NOT for an unbootable row and points back
    here, at `_composer_is_boot_residue_only` — a function this file no longer
    has, because the residue cleaner was deleted (correctly: it was firing a
    Ctrl+C into a live campaign session every five and a half minutes for three
    hours). So each signpost pointed at the other, the thing they both named was
    gone, and an operator following either arrived nowhere. A row jammed this way
    sat refused and escalated with a remedy that could not have worked.

    ⛔⛔ AND IT CANNOT CLEAR ITSELF, FOR A REASON WORTH KNOWING. The ledger's
    "COMPLETE it next tick" path is `--submit-iff-line-equals`, which compares
    against the input line the DAEMON built from bytes it forwarded. That line is
    reconstructed from zero when a newer daemon adopts the session — so across a
    handover the comparison is 0 bytes against 458 and the submit can never fire.
    Measured 2026-08-21 on a live row: the rendered screen held all 458 bytes and
    the daemon answered `holds 0 bytes, expected 458`. The atomic submit and the
    draft flag share one blind spot, so the guard that made the writer safe is
    also what makes these rows permanently unrecoverable.

    ⭐ WHAT MAKES TYPING HERE PERMISSIBLE AT ALL IS AN EXACT MATCH, and nothing
    weaker. The composer reconstructed from the rendered grid must equal
    `BOOT_TEXT` CHARACTER FOR CHARACTER — not contain it, not start with it.
    Verified byte-exact against a live jammed row before this was written. A
    prefix or a substring test would fire on a row where somebody had typed after
    our text, and that is somebody's sentence.

    ⚖ It is a VERB, not a tick action. Every guard in this file exists to stop a
    timer typing into a row; the answer to one of them being unrecoverable is a
    person asking for it, not the loop deciding on its own.
    """
    row = resolve(args.host, args.row) or args.row
    if not row:
        log("⛔ unjam: name a row")
        return 64
    short = row.rsplit("/", 1)[-1][:8]
    # The same state guards a boot takes, in the same order and for the same
    # reasons — a modal reads single keys, and blind is not clear.
    choice = _screen_shows_a_choice(args.host, row)
    if choice is not False:
        log(f"⛔ NOT UNJAMMING {short} — " + ("its screen shows a prompt awaiting a "
            "choice" if choice else "its screen could not be read, so a waiting "
            "prompt cannot be ruled out") + ". Blind is not clear.")
        return 1
    readable, composer = _composer_row_content(args.host, row)
    if not readable or composer is None:
        log(f"⛔ NOT UNJAMMING {short} — its composer row could not be read.")
        return 1
    if composer == "":
        log(f"⭐ {short} has an EMPTY composer — nothing is jamming it. "
            f"If it is still not being woken, the reason is elsewhere.")
        return 0
    pending = _pending_write(row) or {}
    ours = {BOOT_TEXT, pending.get("text") or BOOT_TEXT}
    if composer not in ours:
        log(f"⛔ REFUSING to touch {short} — its composer holds {len(composer)} "
            f"character(s) that are NOT exactly our own boot text. This may be a "
            f"person's unsent sentence, and nothing here gets to decide it is not. "
            f"Read it with `server screen` and clear it by hand if it is yours.")
        return 1
    if args.dry_run:
        log(f"⭐ {short} holds EXACTLY our own boot text ({len(composer)} chars) "
            f"and nothing else. --dry-run: not touching it.")
        return 0
    # 1. THE BOOT WAS INTENDED, SO TRY TO DELIVER IT FIRST. If the daemon's line
    #    survived, this presses Enter on our own text under its lock and the row
    #    gets the wake it was owed — strictly better than throwing it away.
    if _atomic_submit(args.host, row, composer) is True:
        _clear_pending_write(row)
        log(f"⭐ UNJAMMED {short} by DELIVERING it — the daemon's line still held "
            f"exactly our text, so the Enter it never got has now been pressed.")
        return 0
    # 2. The daemon cannot vouch for the line (a handover zeroed it), so the
    #    atomic path can never fire on this row again. Clear it and let the next
    #    tick write once, from a composer that is provably empty.
    log(f"⚠ {short}: the daemon cannot confirm the line (a handover reconstructs "
        f"it from zero), so the atomic submit can never fire here. Clearing our "
        f"own text instead — the next tick writes once into an empty composer.")
    _run(BB.row_host(row, args.host) or args.host,
         ["server", "terminal", "write", f"cc-runtime://{row.rsplit('/', 1)[-1]}",
          "--stdin"], COMPOSER_CLEAR_LINE,
         remote_binary="$HOME/.local/bin/yggterm-headless")
    time.sleep(1.0)
    # ⛔ READ IT BACK. Every verb in this file reports the REQUEST unless it is
    #    made to report the EFFECT, and "I sent a Ctrl+U" is not "the line is
    #    gone" — a composer that did not clear must not be reported as recovered.
    readable, after = _composer_row_content(args.host, row)
    if readable and after == "":
        _clear_pending_write(row)
        log(f"⭐ UNJAMMED {short} — read back EMPTY. It boots normally from here.")
        return 0
    log(f"⛔ {short} did NOT clear: it still holds "
        f"{'an unreadable composer' if not readable else str(len(after or '')) + ' character(s)'}. "
        f"Nothing further is attempted — the next step is an interrupt, and that "
        f"ends a turn, which is a person's call and not this verb's.")
    return 1


def main():
    ap = argparse.ArgumentParser(description="boot a stalled session that subscribed")
    ap.add_argument("action",
                    choices=["subscribe", "unsubscribe", "defer", "list", "tick",
                             "watch", "status", "disarm", "arm", "coverage", "retire",
                             "never-arm", "optout", "hold", "unjam"])
    ap.add_argument("--secs", type=int, default=0,
                    help=f"defer: boot window for one long wait, clamped to "
                         f"{MIN_BOOT_AFTER_SECS}-{MAX_BOOT_AFTER_SECS}s "
                         f"(the ceiling is the prompt-cache limit)")
    ap.add_argument("--clear", action="store_true",
                    help="defer: drop the deferral and return to the default window. "
                         "hold: lift the fleet-wide boot hold now")
    ap.add_argument("--until", default="",
                    help="hold: when the hold expires — `5d`/`36h`/`90m`, or an "
                         "absolute local `2026-08-19T09:00`. Stored as an ABSOLUTE "
                         "instant either way, so re-running does not walk it forward. "
                         "With no --until, `hold` reports the current hold")
    ap.add_argument("--reason", default="",
                    help="hold: why the fleet is being held (recorded and logged)")
    # ⛔ THE ROW ARGUMENT, IN EVERY SPELLING. `--row` here versus a bare
    #    positional on the monitor meant naming one row took a different
    #    incantation per tool, and each refusal reads like "not subscribed".
    #    ygg_rowarg owns the vocabulary now; both watchdogs accept both.
    ap.add_argument("row", nargs="?", default="",
                    help="the row: a bare uuid, or an addressable "
                         "`scheme://host/<uuid>` path")
    ap.add_argument("--row", "--uuid", dest="_row_flag", default="", metavar="ROW",
                    help="the same row, named by flag instead of position — "
                         "both spellings work on every verb and on both watchdogs")
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
    ap.add_argument("--kind", choices=("task", "monitor"), default=None,
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
                         "this is deliberately not the default. "
                         "hold: hold the fleet with NO expiry — nothing lifts it but "
                         "`hold --clear`. For a limit reset, after which every session "
                         "is cold and recovery should start one master orchestrator "
                         "rather than waking everyone")
    ap.add_argument("--json", action="store_true",
                    help="list/status: the machine-readable state, for a surface "
                         "to render and drive")
    ap.add_argument("--due", action="store_true",
                    help="list/status --json: also classify each subscriber to say "
                         "WHEN it is next at risk of a boot. Costs a row-list call "
                         "and a transcript read per subscriber, so it is opt-in")
    args = ap.parse_args()
    # ⭐ One row name, whichever way the caller spelled it. ⛔ `arm` validates
    #    args.row as a full addressable path, so the RAW value is restored
    #    after resolution and only the disagreement check is applied here —
    #    folding a path down to a bare uuid would break that refusal.
    args._row_dest = "row"
    raw_row = args.row or args._row_flag
    try:
        resolve_row(args)
    except ValueError as exc:
        log(f"⛔ {exc}")
        return 64
    args.row = raw_row.strip().rstrip("/") if raw_row else ""
    # ⛔ Never carry a placeholder host into a boot decision.
    # ⭐ But a LEDGER WRITE IS NOT A BOOT DECISION, and making it wait on the GUI
    #    host is how recording a decision fails on a host that cannot reach the
    #    desktop — which is precisely the moment someone gives up and edits the
    #    file by hand. These verbs touch only local state, so they never ask.
    if args.action in ("never-arm", "optout", "retire", "arm", "disarm", "hold"):
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
        "hold": cmd_hold,
        "unjam": cmd_unjam,
    }[args.action](args)


if __name__ == "__main__":
    sys.exit(main())
