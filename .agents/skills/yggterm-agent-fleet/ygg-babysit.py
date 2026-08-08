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


def row_host(row, gui_host):
    """Which MACHINE holds this row's transcript.

    ⛔⛔ THE BUG THIS FIXES, caught by dogfooding within a minute of writing the
    tool. It searched `~/.claude/projects` on the LOCAL host for every row, so a
    `local://<uuid>` row — which runs on the GUI host — came back NO_TRANSCRIPT,
    and the tool confidently announced "the brief was dropped, re-submit it"
    about a perfectly healthy session on another machine.
    ⇒ That is the exact defect this whole fleet keeps re-finding: A CAUSE NOT
      DERIVED FROM A MEASUREMENT, stated with confidence. "I looked in the wrong
      place" and "it is not there" are different facts.

    Row path forms: `remote-cc://<host>/<uuid>` · `remote-session://<host>/<uuid>`
    · `local://<uuid>` (the GUI host) · a bare uuid (assume local)."""
    if "://" not in row:
        return None                                   # bare uuid: this host
    scheme, rest = row.split("://", 1)
    if scheme.startswith("remote") and "/" in rest:
        return rest.split("/", 1)[0]
    if scheme == "local":
        return gui_host
    return None


def find_transcript(uuid, host=None):
    """The row uuid names the transcript — but only in the project dir for its cwd.

    ⚠ Do NOT fall back to "newest file in the directory". Linking a pid or a row
    to a transcript by recency is how a probe reports one session's health as
    another's; the uuid is the only honest key."""
    if host:
        r = subprocess.run(
            ["ssh", host, f"ls -1 ~/.claude/projects/*/{uuid}.jsonl 2>/dev/null | head -1"],
            capture_output=True, text=True, timeout=60)
        out = (r.stdout or "").strip()
        return out or None
    hits = list(PROJECTS.glob(f"*/{uuid}.jsonl"))
    return str(hits[0]) if hits else None


REMOTE_PROBE = r'''
import json,os,sys,time
p=sys.argv[1]
try: rows=[json.loads(l) for l in open(p) if l.strip()]
except Exception as e: print(json.dumps(["UNREADABLE",0,str(e)])); sys.exit()
age=time.time()-os.path.getmtime(p)
last=next((r for r in reversed(rows) if r.get("type") in ("assistant","user")),None)
if last is None: print(json.dumps(["EMPTY",age,""])); sys.exit()
if last["type"]=="user": print(json.dumps(["MIDTURN",age,""])); sys.exit()
items=[c for c in (last.get("message",{}).get("content") or []) if isinstance(c,dict)]
if any(c.get("type")=="tool_use" for c in items): print(json.dumps(["MIDTURN",age,""])); sys.exit()
t=" ".join(" ".join(c.get("text","") for c in items if c.get("type")=="text").split())
print(json.dumps(["TURN_ENDED",age,t[:300]]))
'''


def turn_state_remote(host, path):
    """Same classification, executed WHERE THE TRANSCRIPT LIVES."""
    # ⛔ NOT `ssh host python3 -c <script> <path>`. subprocess passes argv
    #    unquoted, and ssh JOINS argv into ONE remote shell command string — so a
    #    multi-line script with quotes is re-parsed by the remote shell and
    #    arrives mangled. It fails as UNREACHABLE, which reads like a dead host.
    #    Feed the script on STDIN, where no shell can touch it.
    r = subprocess.run(["ssh", host, f"python3 - '{path}'"],
                       input=REMOTE_PROBE, capture_output=True, text=True, timeout=90)
    try:
        st, age, tail = json.loads((r.stdout or "").strip().splitlines()[-1])
        return st, age, tail
    except Exception:
        return "UNREACHABLE", 0, ""


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


def row_exists(gui_host, ident):
    """Is this row still IN THE LIVE ORDER?

    ⛔⛔ WITHOUT THIS, A RETIRED ROW READS AS A WEDGED ONE — measured 2026-08-08,
    on my own first use. The yggterm row had been retired by its campaign's baton
    relay; its transcript is frozen MID-TURN forever, so the classifier called it
    `STUCK` for 54 minutes and I reported that to the owner as a live wedge. It
    was a corpse.
    ⇒ **A transcript cannot distinguish KILLED from WEDGED.** Only the row list
      can, and it must be consulted FIRST. (It also explains a `submitted:false`
      that looked like a busy row refusing input: there was no row.)"""
    return resolve_row_path(gui_host, ident) is not None


def classify(uuid, host=None):
    t = find_transcript(uuid, host)
    if t is None:
        # ⛔ NOT "still starting" past a minute. An agent-CLI that took a brief
        #    writes within seconds; absence means the brief was DROPPED.
        return {"state": "NO_TRANSCRIPT", "age": 0, "tail": "", "path": None}
    state, age, tail = (turn_state_remote(host, t) if host
                        else turn_state(Path(t)))
    if state == "UNREACHABLE":
        # ⛔ Say "I could not look", never "it is not there".
        return {"state": "UNREACHABLE", "age": 0, "tail": "", "path": t}
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


_ROWS_CACHE = {}


def resolve_row_path(host, ident):
    """Turn ANY session identifier into a real ROW PATH, or None.

    ⛔⛔ THE TRAP THIS CLOSES, owner-reported 2026-08-08: *"Clicking these delegate
    notification does not transfer me to the required attention session."*

    `notify --session` makes the card clickable through to that row — but ONLY if
    it is given a genuine row path. `$YGGTERM_SESSION_ID` is `cc-runtime://<uuid>`
    while the row is `remote-cc://<host>/<uuid>`: same uuid, different string. Pass
    the former and the card renders, looks correct, and is **INERT**. The verb's
    own help warns about it — which is precisely the shape this skill exists to
    kill, because a warning in prose is something an agent has to REMEMBER.
    ⇒ Resolve by UUID against `server app rows` so passing the wrong one is
      impossible rather than merely discouraged."""
    if not ident:
        return None
    uuid = ident.rstrip("/").split("/")[-1]
    if not _ROWS_CACHE:
        d = ygg(host, "server", "app", "rows")
        for r in (d.get("data", {}) or {}).get("rows", []) or []:
            path = r.get("path") or ""
            if path:
                _ROWS_CACHE[path.rstrip("/").split("/")[-1]] = path
    return _ROWS_CACHE.get(uuid)


def escalate(host, row, why, notify_session):
    """Tell a human — and make the card land WHERE THE ATTENTION IS NEEDED.

    ⛔ The card must point at the ROW THAT IS STUCK, not at the orchestrator that
       noticed. Pointing it at myself was the second half of the same bug: the
       notification worked and took him to the wrong place, which is worse than
       inert because it looks like it functioned."""
    log(f"ESCALATE {row}: {why}")
    target = resolve_row_path(host, row) or resolve_row_path(host, notify_session)
    if target is None:
        log(f"  ⚠ no row path resolves for {row} — sending an UNTARGETED card "
            f"rather than an inert one")
    args = ["server", "app", "notify", "delegate needs a human", why, "--tone", "warning"]
    if target:
        args += ["--session", target]
    ygg(host, *args)


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
            rhost = row_host(row, args.host)
            if rhost and rhost == os.uname().nodename:
                rhost = None                      # it is this machine after all
            if not row_exists(args.host, row):
                # ⛔ Ask the ROW LIST before the transcript. A retired row's
                #    transcript is frozen mid-turn and is indistinguishable from
                #    a live wedge.
                report.append({"row": row, "state": "GONE", "age_min": 0,
                               "action": "RETIRED", "tail": ""})
                continue
            c = classify(uuid, rhost)
            st = load_state(uuid)
            try:
                size = os.path.getsize(c["path"]) if (c["path"] and not rhost) else 0
            except OSError:
                size = 0
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
            elif c["state"] == "UNREACHABLE":
                action = "CANNOT-SEE"             # not a verdict about the row
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
