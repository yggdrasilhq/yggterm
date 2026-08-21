#!/usr/bin/env python3
"""ygg-spawn — birth a lane complete: seated, grouped, briefed, proven, armed.

⛔⛔ WHY THIS EXISTS. Spawning a lane correctly is seven steps across four
surfaces, and every orchestrator has re-assembled them from primitives. The
failures are not in the create — they are in the parts around it, and each one
has cost this campaign a lane:

  · `terminal new --outline` is HONOURED for a shell row and silently DROPPED for
    an agent-CLI row, replying `error: null` either way. The seat has to be set
    again afterwards and read back.
  · nothing nests the new row into its campaign's group, so lanes accumulate at
    the top level beside the orchestrator instead of underneath it.
  · a brief passed at create has silently vanished; the four-step submit is the
    only path that can be proven.
  · `submitted: true` describes the WRITE. The transcript proves the delivery,
    and it lags ~14.5 s behind the submit, so checking too early is a false
    negative in exactly the window a spawner is deciding in.
  · a brief that opens "claim your row first" spends the lane's opening turn on
    bookkeeping and the turn ends there — measured, repeatedly. An orchestrator
    already knows the seat and title and sets them itself.
  · nothing armed the booter, so a lane that stalled was watched by nobody.

⇒ One verb, or the next orchestrator learns the same six things again.

USAGE
    ygg-spawn.py --seat 11.21 --into <head-row-uri-or-uuid> \\
                 --title "topic: what it is for" --purpose "one line" \\
                 --cwd /path/to/worktree --brief brief.txt [--ack ACK-TOKEN]

Prints the new row's uri on stdout. Diagnostics go to stderr.

⭐ **IT CAN BE AIMED AT A SANDBOX.** `YGGTERM_HOME=$SB
YGG_HEADLESS_BIN=$SB/bin/yggterm-headless ygg-spawn.py …` births the row on an
isolated plane on this machine — no ssh, nobody's desktop, and a seven-step verb
that was previously only ever rehearsed by spending a real seat. `ygg_appctl`
owns that question for every fleet verb.
"""
import argparse
import glob
import json
import os
import subprocess
import sys
import time

import sys as _sys, os as _os
_sys.path.insert(0, _os.path.dirname(_os.path.abspath(__file__)))
import ygg_appctl  # noqa: E402
import ygg_transcript  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))

#: The transcript lags the submit. Measured 2026-08-21: brief accepted at
#: 13:33:58 with submitted:true, transcript created 13:34:12.500 — 14.5 s, with
#: no project directory at all before that. Checking earlier reports "not
#: delivered" about a brief that is on its way.
TRANSCRIPT_LAG_GRACE_S = 90

#: How long to wait for a freshly created agent row to start CONSUMING INPUT.
#: ⚠ Generous on purpose: a machine already running twenty agents starts the
#: twenty-first slowly, and that is precisely when an unattended respawn runs.
READY_WAIT_S = 420


def log(msg):
    print(f"{time.strftime('%H:%M:%S')} ygg-spawn {msg}", file=sys.stderr)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--seat", required=True, help="dotted seat, e.g. 11.21")
    ap.add_argument("--title", required=True)
    ap.add_argument("--purpose", required=True, help="one line a human can act on")
    ap.add_argument("--cwd", required=True)
    ap.add_argument("--brief", required=True, help="file holding the brief to submit")
    ap.add_argument("--no-group", action="store_true",
                    help="leave the row at the top level. Needed for a SUCCESSOR at a head "
                         "seat: deriving the group from the seat would nest 11.0's successor "
                         "under 11.0, i.e. under the row it is replacing.")
    ap.add_argument("--into", help="row this one is nested under (uri or uuid); "
                                   "default: the seat's own head, e.g. 11.0 for 11.21")
    ap.add_argument("--ack", help="token that must appear in the transcript; "
                                  "default: the first line of the brief")
    ap.add_argument("--kind", default="claude-code")
    ap.add_argument("--machine-key", default="dev")
    ap.add_argument("--model", default="claude-opus-5")
    ap.add_argument("--host", help="the machine whose GUI answers; `local` for this one "
                                   "(a sandbox). $YGGTERM_HOME and $YGG_HEADLESS_BIN aim "
                                   "the home and the binary — see ygg_appctl.")
    ap.add_argument("--dry-run", action="store_true")
    a = ap.parse_args()

    # ⛔ WHICH PLANE THIS ROW IS BORN ON — machine, home and binary, stated once.
    #    A sandbox home makes this verb rehearsable; it used to be provable only
    #    by spending a seat on somebody's live desktop.
    plane = ygg_appctl.resolve(a.host)
    if plane is None:
        log("⛔ no row plane — app control did not answer anywhere.")
        return 2
    app = plane.app_json
    if not os.path.exists(a.brief):
        log(f"⛔ no such brief file: {a.brief}")
        return 2
    brief = open(a.brief).read()
    ack = a.ack or brief.strip().split()[0]

    # ⛔ THE BRIEF MUST NOT OPEN WITH THE CLAIM. Refused rather than warned,
    # because the failure is silent: the lane reports having claimed and its turn
    # ends, which looks like a working spawn for as long as nobody counts turns.
    head = brief.strip()[:400].lower()
    if "ygg-claim" in head or "claim your row first" in head:
        log("⛔ this brief opens by telling the lane to claim its row. That spends its")
        log("   first turn on bookkeeping and the turn ends there — measured, twice.")
        log("   This verb seats and titles the row for you; take the claim out.")
        return 3

    if a.dry_run:
        log(f"would spawn seat {a.seat} in {a.cwd}, brief {len(brief)}B, ack {ack}")
        return 0

    # 1. CREATE, never activated: a spawn must not take the owner's screen.
    reply = app(f"terminal new --kind {a.kind} --machine-key {a.machine_key} "
                      f"--cwd {a.cwd} --title {json.dumps(a.title)} "
                      f"--purpose {json.dumps(a.purpose)} --no-activate "
                      f"--model {a.model} --permission-mode bypass --outline {a.seat}")
    data = reply.get("data") or {}
    row = data.get("session_path")
    if not row:
        log(f"⛔ create failed: {reply.get('error')}")
        return 4
    uuid = row.rsplit("/", 1)[-1]
    log(f"created {row}")

    # 2. THE SEAT, RE-ASSERTED AND READ BACK. `--outline` at create is honoured
    #    for a shell row and dropped for an agent row, with `error: null` both
    #    times — so the reply cannot be trusted and the state must be read.
    if (data.get("seat") or {}).get("honoured") or data.get("outline_prefix") == a.seat:
        log(f"seat {a.seat} honoured at create")
    else:
        app(f"session outline '{row}' {a.seat}")
        rows = (app("rows --json").get("data") or {}).get("rows") or []
        got = next((r.get("outline_prefix") for r in rows if r.get("full_path") == row), None)
        if str(got) != a.seat:
            log(f"⛔ seat did not take: wanted {a.seat}, row reads {got}")
        else:
            log(f"seat {a.seat} set after the fact (create dropped it)")

    # 3. INTO ITS GROUP. A lane that sits beside its orchestrator instead of
    #    under it is the hygiene complaint this verb exists to end.
    into = None if a.no_group else a.into
    if a.no_group:
        log("left at the top level (--no-group)")
    elif not into:
        rows = (app("rows --json").get("data") or {}).get("rows") or []
        head_seat = a.seat.split(".")[0] + ".0"
        into = next((r.get("full_path") for r in rows
                     if str(r.get("outline_prefix")) == head_seat), None)
    if into and not a.no_group:
        if "://" not in into:
            rows = (app("rows --json").get("data") or {}).get("rows") or []
            into = next((r["full_path"] for r in rows if into in (r.get("full_path") or "")), into)
        res = app(f"row-set '{row}' --into '{into}'")
        log(f"nested under {str(into)[-42:]}: accepted={(res.get('data') or {}).get('accepted')}")
    elif not a.no_group:
        log("⚠ no head row found for this campaign — left at the top level")

    # 4. WAIT FOR IT TO BE READING INPUT. The composer is drawn well before the
    #    input loop is live, so this is a real wait and not a formality.
    #
    # ⛔⛔ THIS WAIT WAS 30 SECONDS AND A COLD AGENT CLI TAKES LONGER, SO EVERY
    #    UNATTENDED SPAWN FAILED HERE. Measured 2026-08-21: two automated respawns
    #    in a row timed out at 30 s, refused their submit, and exited — and because
    #    a hand-run spawn is watched by someone who simply re-runs it, the ceiling
    #    only ever bit the automated path. That is why no cold row was ever
    #    replaced by the hourly loop even after `--respawn` was wired: the verb it
    #    calls could not finish.
    #
    # ⇒ Wait on a DEADLINE, not on a fixed number of tries, and make the deadline
    #   generous — a machine running twenty agents starts the twenty-first slowly,
    #   which is exactly when a respawn is needed most.
    ready, deadline = False, time.time() + READY_WAIT_S
    while time.time() < deadline:
        v = (app(f"terminal input-check '{row}' --check-timeout-ms 20000").get("data") or {})
        if v.get("consuming_input"):
            ready = True
            break
        time.sleep(10)
    log(f"input-check consuming_input={ready} (waited {READY_WAIT_S}s at most)")

    # 5. SUBMIT, and read the answer. ⛔ `submitted:false` means MID-OUTPUT, never
    #    unreachable, and is NEVER retried — that is the bug that types over people.
    staged_brief = plane.put(a.brief, f"/tmp/ygg-spawn-{uuid[:8]}.brief")
    sub = (app(f"terminal submit '{row}' --stdin", stdin_path=staged_brief).get("data") or {})
    if not sub.get("submitted"):
        # ⛔⛔ NEVER RETRY A `submitted:false` — that is the bug that types over
        # people. But printing advice and exiting leaves a SEATED, BRIEFLESS row
        # behind, and nothing ever comes back for it: measured 2026-08-21, three
        # such rows, one alive for over two hours, each one also holding a seat
        # that its own predecessor still held. A failed spawn that leaves debris
        # is worse than one that cleans up after itself.
        #
        # ⇒ Hand the brief to `ygg-deliver`, which waits for the row to be able to
        #   take it and then submits ONCE. The row stays; the caller is told the
        #   delivery is armed rather than done, and a respawn still refuses to fold
        #   its predecessor because the brief is not yet PROVEN.
        deliver = os.path.join(HERE, "ygg-deliver.py")
        if os.path.exists(deliver):
            subprocess.Popen([deliver, uuid, "--message", a.brief, "--ack", ack,
                              "--wait-min", "30"],
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            log("⛔ submit refused (mid-output). NOT retried — delivery ARMED via ygg-deliver,")
            log("   which waits for the row to be consuming input and then submits once.")
        else:
            log("⛔ submit refused (mid-output). NOT retried. The row exists and is seated;")
            log(f"   deliver by file: {a.brief}")
        print(row)
        return 5
    log(f"submitted {sub.get('bytes')}B")

    # 6. PROVE DELIVERY IN THE TRANSCRIPT. A transcript FILE existing proves a
    #    process started; the ACK token proves what reached it.
    deadline = time.time() + TRANSCRIPT_LAG_GRACE_S
    seen = False
    while time.time() < deadline:
        # ⛔ The brief-arrived proof. Hardcoding one store made this unanswerable
        #    for every other CLI — so a non-reference lane could never be born
        #    PROVEN, which is the one guarantee this verb exists to give.
        if ygg_transcript.carries(uuid, ack):
            seen = True
        if seen:
            break
        time.sleep(3)
    log(f"transcript carries {ack}: {seen}")

    # 7. ARM **BOTH** SUPERVISION PLANES, AND THEY ARE SEPARATE STORES.
    #
    # ⛔⛔ THE BOOTER WAKES; THE MONITOR ROUTES. Arming only the booter produces a
    # row that something will nudge and whose escalations reach NOBODY — the
    # monitor lists exactly that state as "armed on the booter but escalating to
    # nobody: a stall would ring into an empty room". The first two lanes this
    # verb spawned landed in it, because this step armed one plane and called the
    # job done.
    #
    # ⚠ And the monitor subscription must name the ORCHESTRATOR, not a human. A
    # subscription whose `escalate_to` points at a retired row swallows every cry
    # for help and reports success: measured today with FIVE lanes all escalating
    # to an orchestrator folded hours earlier, which is why the monitor appeared
    # to fire only when everything had gone quiet. It was never firing at all.
    boot = os.path.join(HERE, "ygg-booter.py")
    if os.path.exists(boot):
        subprocess.run([sys.executable, boot, "subscribe", "--row", row,
                        "--campaign", a.seat.split(".")[0]],
                       capture_output=True, text=True, timeout=120)
        log("armed on the booter")
    mon = os.path.join(HERE, "ygg-monitor.py")
    me = (os.environ.get("YGGTERM_SESSION_ID") or "").rsplit("/", 1)[-1]
    if os.path.exists(mon) and me:
        subprocess.run([sys.executable, mon, "subscribe", uuid,
                        "--machine", a.machine_key, "--role", "relay",
                        "--escalate-to", me, "--escalate-host", a.machine_key,
                        "--campaign", a.seat.split(".")[0], "--seat", a.seat],
                       capture_output=True, text=True, timeout=120)
        log(f"subscribed on the monitor, escalating to {me[:8]}")
    elif not me:
        log("⚠ no YGGTERM_SESSION_ID — the monitor subscription would escalate to nobody,")
        log("  so it was SKIPPED rather than written pointing at a human card.")

    print(row)
    return 0 if seen else 6


if __name__ == "__main__":
    sys.exit(main())
