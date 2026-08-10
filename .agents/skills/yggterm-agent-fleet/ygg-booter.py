#!/usr/bin/env python3
"""ygg-booter — a session SUBSCRIBES, and something outside it kicks it when it stalls.

⛔⛔ THE DEFECT THIS EXISTS FOR (owner-directed 2026-08-09):

    *"I have seen you stall sometimes, so arm a booter in a fleet. A booter is a
    tool that monitors any session that has subscribed to it, to kick it and say
    'continue, the booter booted'. Sometimes you may feel that the work is done
    so you need to unsubscribe from the booter."*

He was manually booting a stalled relay session when he said it. **That is the
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
    ygg-booter.py list
    ygg-booter.py tick [--dry-run]                  # one pass over all subscribers
    ygg-booter.py watch [--interval 300]            # the loop (usually detached)
    ygg-booter.py status                            # is a watcher alive?

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

HERE = Path(__file__).resolve().parent
STATE = Path.home() / ".yggterm" / "relay"
SUBS = STATE / "booter"
PIDFILE = STATE / "booter.pid"
HEARTBEAT = STATE / "booter.heartbeat"

# The words he used. A boot must be recognisable AS a boot in the transcript —
# both to a human reading back and to the session itself, which should be able to
# tell "a machine woke me" from "a person asked for something".
#
# ⭐⭐ AND IT NOW CARRIES A STEER, owner-directed 2026-08-10: *"if the booter is armed then I
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

# ⛔⛔ THE CEILING IS A BILLING LIMIT, NOT A TUNING KNOB — owner-directed
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


def _load_babysit():
    """Import the sibling classifier. One owner for row liveness."""
    spec = importlib.util.spec_from_file_location("ygg_babysit", HERE / "ygg-babysit.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


BB = _load_babysit()


def log(m):
    print(f"{time.strftime('%H:%M:%S')} ygg-booter {m}", flush=True)


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


def resolve(host, ident):
    """Any identifier -> a real ROW PATH, or None.

    ⛔ `$YGGTERM_SESSION_ID` is `cc-runtime://<uuid>`; the row is
    `remote-cc://<host>/<uuid>`. Same uuid, different string, and the wrong one
    addresses nothing while every verb still answers OK. Resolve by UUID against
    the live row list so the mistake cannot be made."""
    BB._ROWS_CACHE.clear()
    return BB.resolve_row_path(host, ident)


def cmd_subscribe(args):
    uuid = (args.row or "").rstrip("/").split("/")[-1] or own_uuid()
    if not uuid:
        log("no row given and $YGGTERM_SESSION_ID is unset — nothing to subscribe")
        return 2
    row = resolve(args.host, uuid)
    if row is None:
        log(f"⚠ {uuid} does not resolve to a live row on {args.host} — "
            f"subscribing anyway; the first tick will retire it if it stays gone")
        row = args.row or uuid
    rec = {
        "uuid": uuid,
        "row": row,
        "campaign": args.campaign,
        "note": args.note,
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


def cmd_list(args):
    subs = load_subs()
    if not subs:
        log("no subscribers")
        return 0
    for s in subs:
        age_h = (time.time() - s["subscribed_at"]) / 3600
        log(f"{s['uuid'][:8]}  {s.get('campaign') or '-':<12} "
            f"age={age_h:4.1f}h boots={s['boots']} {s['row']}")
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
       CONCATENATED `\\n`.** Owner-reported 2026-08-09 ~19:10, watching it happen:
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
        if resolve(host, uuid) is None:
            log(f"{uuid[:8]} GONE (retired) — unsubscribing")
            sub_path(uuid).unlink(missing_ok=True)
            continue
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
        if state in ("WORKING", "JUST_ENDED"):
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
            sub_path(uuid).write_text(json.dumps(s, indent=1))
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
        return f"already running (pid {alive})"
    STATE.mkdir(parents=True, exist_ok=True)
    logf = open(STATE / "booter.log", "a")
    p = subprocess.Popen(
        [sys.executable, str(HERE / "ygg-booter.py"), "watch",
         "--host", args.host, "--interval", str(args.interval)],
        stdout=logf, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
        start_new_session=True)
    time.sleep(1.5)
    return f"armed (pid {p.pid})" if watcher_alive() else "⛔ FAILED TO ARM"


def cmd_watch(args):
    PIDFILE.parent.mkdir(parents=True, exist_ok=True)
    PIDFILE.write_text(str(os.getpid()))
    log(f"watcher up (pid {os.getpid()}, interval {args.interval}s, gui host {args.host})")
    try:
        while True:
            HEARTBEAT.write_text(json.dumps({"ts": time.time(), "pid": os.getpid()}))
            if not load_subs():
                log("no subscribers left — retiring")
                break
            tick(args)
            time.sleep(args.interval)
    finally:
        PIDFILE.unlink(missing_ok=True)
    return 0


def cmd_status(args):
    alive = watcher_alive()
    hb = "never"
    if HEARTBEAT.exists():
        try:
            hb = f"{time.time() - json.loads(HEARTBEAT.read_text())['ts']:.0f}s ago"
        except Exception:
            pass
    log(f"watcher: {'alive pid ' + str(alive) if alive else 'NOT RUNNING'} · "
        f"heartbeat {hb} · subscribers {len(load_subs())}")
    return 0 if alive else 1


def main():
    ap = argparse.ArgumentParser(description="boot a stalled session that subscribed")
    ap.add_argument("action",
                    choices=["subscribe", "unsubscribe", "defer", "list", "tick",
                             "watch", "status"])
    ap.add_argument("--secs", type=int, default=0,
                    help=f"defer: boot window for one long wait, clamped to "
                         f"{MIN_BOOT_AFTER_SECS}-{MAX_BOOT_AFTER_SECS}s "
                         f"(the ceiling is the prompt-cache limit)")
    ap.add_argument("--clear", action="store_true",
                    help="defer: drop the deferral and return to the default window")
    ap.add_argument("--row", default="")
    ap.add_argument("--campaign", default="")
    ap.add_argument("--note", default="")
    ap.add_argument("--host", default=os.environ.get("YGG_GUI_HOST", "guihost"),
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
    args = ap.parse_args()
    return {
        "subscribe": cmd_subscribe,
        "unsubscribe": cmd_unsubscribe,
        "defer": cmd_defer,
        "list": cmd_list,
        "tick": tick,
        "watch": cmd_watch,
        "status": cmd_status,
    }[args.action](args)


if __name__ == "__main__":
    sys.exit(main())
