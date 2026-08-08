#!/usr/bin/env python3
"""ygg-babysit — keep spawned delegate rows RUNNING, without anyone remembering to look.

⛔⛔ THE DEFECT THIS EXISTS FOR (owner-reported 2026-08-08, measured the same hour):

    A DELEGATE THAT HAS FINISHED AND A DELEGATE THAT HAS STALLED ARE
    INDISTINGUISHABLE FROM THE ROW PLANE. Both present as "turn ended, nothing
    happening". So the orchestrator cannot tell success from a halted pipeline,
    and a stall is discovered only when a human notices hours later.

Measured that day: two delegates spawned together. One LANDED its whole subset;
the other ENDED ITS TURN after acknowledging the brief and sat idle for 54
minutes. Identical from `server app rows`. His words: *"I am seeing that they
both stopped … these yggterm fleet kinks should be seen, 'dreamt' of it and then
auto-resolved whenever encountered by any agent. Otherwise they will stop
critical agentic pipeline like now."*

⇒ Same family as every other scar in this skill: **silence is the most dangerous
  value a status can take.** `rows` reports existence, not liveness; a row's
  agent-CLI sits at its prompt forever, alive and idle, and looks exactly like
  one that is thinking.

★ THE DESIGN, and the asymmetry that decides it:

    A spurious `continue` to a FINISHED row costs one cheap turn, in which it
    says "already done". A MISSED stall costs the pipeline until a human looks.
    ⇒ When idle is ambiguous, NUDGE. Bound it so a finished row is not poked
      forever: MAX_NUDGES per stall, then escalate and stop.

⛔ It never nudges a MID-TURN row. A row whose last event is a tool_use or a
   tool_result is inside a turn — typing at it races the agent's own input and is
   the "never type into a live prompt" defect. Mid-turn + stale is STUCK, which
   is a human's problem, not a `continue`.

Usage:
    ygg-babysit.py --row remote-cc://dev/<uuid> [--row ...]      # explicit
    ygg-babysit.py --spawned-by <my-uuid>                        # from the state file
    ygg-babysit.py --row ... --watch 1800                        # loop until done
    ygg-babysit.py --row ... --dry-run                           # classify only

Exit: 0 all rows working or done · 3 something was nudged · 4 escalation needed.
"""
import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

STATE = Path.home() / ".yggterm" / "relay"
PROJECTS = Path.home() / ".claude" / "projects"

# A row idle longer than this, with its turn ENDED, is a stall candidate.
IDLE_STALL_SECS = 240
# Mid-turn but untouched this long = STUCK. Never nudged; escalated.
MIDTURN_STUCK_SECS = 900
# How many nudges before we stop and tell a human. Two, because the second one
# proves the first was not simply mistimed.
MAX_NUDGES = 2


def log(m):
    print(f"{time.strftime('%H:%M:%S')} ygg-babysit {m}", flush=True)


def ygg(host, *args):
    """Run a yggterm app verb on the GUI host. App-control only resolves there."""
    binp = "$HOME/.yggterm/bin/yggterm"
    cmd = " ".join(f"'{a}'" for a in args)
    r = subprocess.run(["ssh", host, f"{binp} {cmd}"],
                       capture_output=True, text=True, timeout=120)
    try:
        return json.loads(r.stdout[r.stdout.find("{"):])
    except Exception:
        return {}


def find_transcript(uuid):
    """The row uuid names the transcript — but only in the project dir for its cwd.

    ⚠ Do NOT fall back to "newest file in the directory". Linking a pid or a row
    to a transcript by recency is how a probe reports one session's health as
    another's; the uuid is the only honest key."""
    hits = list(PROJECTS.glob(f"*/{uuid}.jsonl"))
    return hits[0] if hits else None


def turn_state(path):
    """Classify from the LAST REAL TURN. Returns (state, age_secs, tail_text).

    ⛔ system/hook rows are NOT turns. Treating the file's last line as the turn
       returns UNKNOWN for nearly every session, because hooks fire after the
       agent has stopped."""
    try:
        rows = [json.loads(l) for l in path.open() if l.strip()]
    except Exception as e:
        return ("UNREADABLE", 0, str(e))
    age = time.time() - path.stat().st_mtime
    last = next((r for r in reversed(rows) if r.get("type") in ("assistant", "user")), None)
    if last is None:
        return ("EMPTY", age, "")
    if last["type"] == "user":
        return ("MIDTURN", age, "")            # a tool_result is mid-turn
    items = [c for c in (last.get("message", {}).get("content") or [])
             if isinstance(c, dict)]
    if any(c.get("type") == "tool_use" for c in items):
        return ("MIDTURN", age, "")
    text = " ".join(" ".join(c.get("text", "") for c in items
                             if c.get("type") == "text").split())
    return ("TURN_ENDED", age, text[:300])


def classify(uuid):
    t = find_transcript(uuid)
    if t is None:
        # ⛔ NOT "still starting" past a minute. An agent-CLI that took a brief
        #    writes within seconds; absence means the brief was DROPPED.
        return {"state": "NO_TRANSCRIPT", "age": 0, "tail": "", "path": None}
    state, age, tail = turn_state(t)
    if state == "MIDTURN":
        state = "WORKING" if age < MIDTURN_STUCK_SECS else "STUCK"
    elif state == "TURN_ENDED":
        state = "IDLE" if age >= IDLE_STALL_SECS else "JUST_ENDED"
    return {"state": state, "age": age, "tail": tail, "path": str(t)}


def state_file(uuid):
    STATE.mkdir(parents=True, exist_ok=True)
    return STATE / f"{uuid}.json"


def load_state(uuid):
    p = state_file(uuid)
    if p.exists():
        try:
            return json.loads(p.read_text())
        except Exception:
            pass
    return {"nudges": 0, "last_size": 0, "escalated": False}


def save_state(uuid, st):
    state_file(uuid).write_text(json.dumps(st))


def nudge(host, row, dry):
    """Send exactly one `continue`. Never more, never to a mid-turn row."""
    if dry:
        log(f"DRY-RUN would nudge {row}")
        return True
    tmp = "/tmp/.ygg-babysit-continue"
    subprocess.run(["ssh", host, f"printf 'continue' > {tmp}"], timeout=60)
    r = ygg(host, "server", "app", "terminal", "submit", row, "--stdin")
    # ⚠ `submitted:true` describes the WRITE, not the delivery — the caller must
    #   confirm by transcript GROWTH on the next pass, which is what last_size is.
    ok = subprocess.run(
        ["ssh", host, f"$HOME/.yggterm/bin/yggterm server app terminal submit '{row}' --stdin < {tmp}"],
        capture_output=True, text=True, timeout=120)
    return "submitted" in (ok.stdout or "") or bool(r)


def escalate(host, row, why, notify_session):
    log(f"ESCALATE {row}: {why}")
    if notify_session:
        ygg(host, "server", "app", "notify", "delegate needs a human", why,
            "--tone", "warning", "--session", notify_session)


def main():
    ap = argparse.ArgumentParser(description="keep delegate rows running")
    ap.add_argument("--row", action="append", default=[])
    ap.add_argument("--spawned-by", default="")
    ap.add_argument("--host", default=os.environ.get("YGG_GUI_HOST", "jojo"))
    ap.add_argument("--notify-session", default=os.environ.get("YGGTERM_SESSION_ID", ""))
    ap.add_argument("--watch", type=int, default=0,
                    help="seconds to keep watching; 0 = one pass")
    ap.add_argument("--interval", type=int, default=180)
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()

    rows = list(args.row)
    if args.spawned_by:
        f = STATE / f"spawned-by-{args.spawned_by}.txt"
        if f.exists():
            rows += [l.strip() for l in f.read_text().splitlines() if l.strip()]
    rows = sorted(set(rows))
    if not rows:
        log("no rows given (--row or --spawned-by)")
        return 2

    deadline = time.time() + args.watch
    rc = 0
    while True:
        report = []
        for row in rows:
            uuid = row.rstrip("/").split("/")[-1]
            c = classify(uuid)
            st = load_state(uuid)
            size = os.path.getsize(c["path"]) if c["path"] else 0
            grew = size > st["last_size"]
            st["last_size"] = size
            action = "-"

            if c["state"] in ("WORKING", "JUST_ENDED"):
                st["nudges"] = 0            # progress clears the stall counter
            elif c["state"] == "NO_TRANSCRIPT":
                action = "RESUBMIT-BRIEF"   # the brief was dropped; caller owns the text
                rc = max(rc, 4)
                escalate(args.host, row, "no transcript: the brief was dropped, re-submit it",
                         args.notify_session)
            elif c["state"] == "STUCK":
                action = "ESCALATE"
                rc = max(rc, 4)
                if not st["escalated"]:
                    escalate(args.host, row,
                             f"mid-turn and untouched for {c['age']/60:.0f} min — a `continue` "
                             f"would race its own input", args.notify_session)
                    st["escalated"] = True
            elif c["state"] == "IDLE":
                if grew:
                    st["nudges"] = 0        # it did work since last pass; just finished a turn
                if st["nudges"] >= MAX_NUDGES:
                    action = "ESCALATE"
                    rc = max(rc, 4)
                    if not st["escalated"]:
                        escalate(args.host, row,
                                 f"idle {c['age']/60:.0f} min and did not wake after "
                                 f"{MAX_NUDGES} nudges", args.notify_session)
                        st["escalated"] = True
                else:
                    st["nudges"] += 1
                    action = f"NUDGE#{st['nudges']}"
                    nudge(args.host, row, args.dry_run)
                    rc = max(rc, 3)
            # ⛔ A DRY RUN MUST NOT MUTATE STATE. The first version incremented the
            #    nudge counter under --dry-run, so classifying a row twice burned
            #    its whole nudge budget without ever sending anything — and the
            #    NEXT real pass would have escalated instead of nudging. Same
            #    family as every other scar here: an instrument whose observation
            #    changes what it observes.
            if not args.dry_run:
                save_state(uuid, st)
            report.append({"row": row, "state": c["state"], "age_min": round(c["age"] / 60, 1),
                           "action": action, "tail": c["tail"][:120]})

        if args.json:
            print(json.dumps(report, indent=2))
        else:
            for r in report:
                log(f"{r['state']:<14} {r['age_min']:>6.1f}m  {r['action']:<12} {r['row']}")
                if r["tail"]:
                    log(f"                              tail: {r['tail'][:100]}")
        if time.time() >= deadline:
            break
        time.sleep(args.interval)
    return rc


if __name__ == "__main__":
    sys.exit(main())
