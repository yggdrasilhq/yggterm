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
BOOT_TEXT = "continue, the booter booted"

# How long a subscribed row may sit with its turn ENDED before it is booted.
# Deliberately longer than babysit's 240s: a subscriber is a long-running campaign
# session that may legitimately pause between phases, and a boot costs it a turn.
#
# ⛔ RAISED 420 -> 1800 ON OWNER DIRECTIVE, 2026-08-09: *"in case of long waits,
# the booter should also wait ~30min before booting to avoid unnecessarily
# booting."* Measured cause: a campaign session waiting on `cargo test
# --workspace` was booted FIVE times in 45 min (12:26 · 12:36 · 12:51 · 13:01 ·
# 13:11) while one test target alone ran 2386s. It was working the whole time.
#
# ⚠ THE BLIND SPOT THIS ACCEPTS, stated so nobody re-derives it: a session
# waiting on a long child process and a session that has genuinely stalled look
# IDENTICAL from here — turn ended, transcript not growing. 420s could not tell
# them apart either; it just guessed sooner and was wrong most of the time. The
# real discriminator is whether the row still has a live child doing work
# (a build, a test binary, an ssh), which this watcher does not read today. Until
# it does, 1800s is the honest trade: a genuine stall is caught inside half an
# hour, and legitimate long work is left alone.
BOOT_AFTER_SECS = 1800
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


def cmd_unsubscribe(args):
    uuid = (args.row or "").rstrip("/").split("/")[-1] or own_uuid()
    p = sub_path(uuid)
    if p.exists():
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

    ⇒ Try the composer first (it is the right door for an agent CLI that is
      reading input), and fall back to the PTY when it says it did not land.
      ⚠ The newline is the Enter; without it the text sits in the line editor.
      One line only — a multi-line send is one Enter per line and the rest queue."""
    if dry:
        log(f"DRY-RUN would boot {row}")
        return "dry-run"
    r = _run(host, ["server", "app", "terminal", "submit", row, "--stdin"], BOOT_TEXT)
    if _field(r.stdout or "", "submitted") is True:
        return "submit"
    w = _run(host, ["server", "terminal", "write", row, "--stdin"], BOOT_TEXT + "\n")
    if _field(w.stdout or "", "accepted") is True:
        return "pty-write"
    return ""


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
        grew = size > s.get("last_size", 0)
        s["last_size"] = size
        action = "-"
        state = c["state"]

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
        elif state == "IDLE" and c["age"] >= BOOT_AFTER_SECS:
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
                # Say WHICH door delivered it. A watchdog that reports "booted"
                # without saying how cannot be debugged when it silently stops.
                action = f"BOOT#{s['boots']}:{via or 'NOT-DELIVERED'}"
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
        log(f"{state:<14} {c['age']/60:>6.1f}m  {action:<12} {uuid[:8]}")
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
                    choices=["subscribe", "unsubscribe", "list", "tick", "watch", "status"])
    ap.add_argument("--row", default="")
    ap.add_argument("--campaign", default="")
    ap.add_argument("--note", default="")
    ap.add_argument("--host", default=os.environ.get("YGG_GUI_HOST", "guihost"),
                    help="the GUI host — app control resolves only there")
    ap.add_argument("--max-hours", type=float, default=12.0)
    ap.add_argument("--interval", type=int, default=DEFAULT_INTERVAL)
    ap.add_argument("--dry-run", action="store_true")
    args = ap.parse_args()
    return {
        "subscribe": cmd_subscribe,
        "unsubscribe": cmd_unsubscribe,
        "list": cmd_list,
        "tick": tick,
        "watch": cmd_watch,
        "status": cmd_status,
    }[args.action](args)


if __name__ == "__main__":
    sys.exit(main())
