#!/usr/bin/env python3
"""ygg-babysit — keep spawned delegate rows RUNNING, without anyone remembering to look.

⛔⛔ THE DEFECT THIS EXISTS FOR (reported 2026-08-08, measured the same hour):

    A DELEGATE THAT HAS FINISHED AND A DELEGATE THAT HAS STALLED ARE
    INDISTINGUISHABLE FROM THE ROW PLANE. Both present as "turn ended, nothing
    happening". So the orchestrator cannot tell success from a halted pipeline,
    and a stall is discovered only when a human notices hours later.

Measured that day: two delegates spawned together. One LANDED its whole subset;
the other ENDED ITS TURN after acknowledging the brief and sat idle for 54
minutes. Identical from `server app rows`. The requirement: *"I am seeing that they
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

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ygg_host import resolve_gui_host  # noqa: E402

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


def ledger_row_path(entry):
    """The ROW PATH inside a spawned-by ledger line.

    ⛔⛔ THE LEDGER LINE IS NOT A ROW PATH, AND EVERY CONSUMER HERE WANTED THE
    PATH. `--spawned-by` reads `<seat>|<lane>|<cwd>|<row-path>` lines and fed
    them, whole, to code that splits on `://` to find a scheme. So for
    `7.1|widgets|<checkout>|local://<uuid>` the "scheme" came out as
    `7.1|widgets|<checkout>|local`, which is neither `local`
    nor a `remote*` prefix — `row_host` fell through to None, the transcript was
    hunted on the WRONG MACHINE, and the tool announced **"the brief was
    dropped, re-submit it"** about a healthy row whose transcript held 8 hits of
    that very brief.

    ⚠ **And the remedy it prescribes is destructive**, which is what makes this
    worse than a wrong label: re-submitting types a full brief into a row that is
    mid-task. Same shape as the monitor's stand-down defect — *a warning whose
    remedy another verb forbids is a defect in the warning* — except this one
    would have been acted on, because a dropped brief is a real and common fault.

    ⚠ **It hid behind an accident.** A `remote-cc://<host>/<uuid>` line
    mis-parses identically, but its transcript happens to live on the machine the
    fallback searches, so those rows read correctly and the tool looked fine on
    every row except the one kind that exposes it. `--row` passes a bare path and
    never had the fault at all, so the two doors into this tool disagreed.
    """
    entry = (entry or "").strip()
    return entry.rsplit("|", 1)[-1].strip() if "|" in entry else entry


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


# ⭐ THE ACCOUNT RAN OUT OF QUOTA — a state a boot cannot improve, like
# CONTEXT_DEAD, but for the opposite reason and with the opposite cure.
#
# CONTEXT_DEAD is about THIS session and is permanent: relay it. A rate limit is
# about the ACCOUNT and is temporary: wait for the window. Both look identical to
# a classifier that only measures activity — the turn ended, the file stopped
# growing, the row goes IDLE — so both get booted, and a boot into an exhausted
# quota is refused before the agent ever runs. It spends the wake and leaves the
# row exactly where it was.
#
# ⛔ KEY ON THE STRUCTURED FIELDS, NEVER THE PROSE. The record's own text names
# the MODEL whose limit was hit ("You've reached your <model> limit…"), so a
# substring match on it goes stale the day a model is renamed and reads as
# healthy — the failure-open direction. The fields below are what the CLI writes
# to say "the API refused this on quota":
#
#   {"type":"assistant", "isApiErrorMessage":true, "apiErrorStatus":429,
#    "error":"rate_limit", "errorDetails":"429 {…\"type\":\"rate_limit_error\"…}",
#    "message":{"usage":{"output_tokens":0,…}}}
#
# Both discriminators are ORed on purpose: the measured record carries both, and
# keying on one alone would fail silently if that one were ever dropped.
#
# ⚠ `usage.output_tokens` is 0, so `progress_marks` already declines to count
# this as work — the anti-flap counter is not fooled. It is the BOOT that was
# wrong, not the accounting.
def api_rate_limited(rec):
    """Is this transcript record the CLI reporting an account-level rate limit?"""
    if not isinstance(rec, dict) or not rec.get("isApiErrorMessage"):
        return False
    return rec.get("apiErrorStatus") == 429 or rec.get("error") == "rate_limit"


REMOTE_PROBE = r'''
import json,os,sys,time
p=sys.argv[1]
try: rows=[json.loads(l) for l in open(p) if l.strip()]
except Exception as e: print(json.dumps(["UNREADABLE",0,str(e)])); sys.exit()
age=time.time()-os.path.getmtime(p)
last=next((r for r in reversed(rows) if r.get("type") in ("assistant","user")),None)
if last is None: print(json.dumps(["EMPTY",age,""])); sys.exit()
if last["type"]=="user": print(json.dumps(["MIDTURN",age,""])); sys.exit()
# ⛔ Same discriminator as api_rate_limited() above, and it must stay the same:
#    a local row and a remote row differing about whether the account has quota
#    is a fleet that boots half of itself into a wall.
if last.get("isApiErrorMessage") and (last.get("apiErrorStatus")==429 or last.get("error")=="rate_limit"):
    t=" ".join(" ".join(c.get("text","") for c in (last.get("message",{}).get("content") or []) if isinstance(c,dict) and c.get("type")=="text").split())
    print(json.dumps(["RATE_LIMITED",age,t[:300]])); sys.exit()
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
    if api_rate_limited(last):
        # ⛔ BEFORE the tool_use test and before TURN_ENDED. This record has no
        #    tool_use and does have text, so it would otherwise read as an
        #    ordinary finished turn — which is precisely how a quota outage was
        #    indistinguishable from a stall.
        text = " ".join(" ".join(c.get("text", "") for c in items
                                 if c.get("type") == "text").split())
        return ("RATE_LIMITED", age, text[:300])
    if any(c.get("type") == "tool_use" for c in items):
        return ("MIDTURN", age, "")
    text = " ".join(" ".join(c.get("text", "") for c in items
                             if c.get("type") == "text").split())
    return ("TURN_ENDED", age, text[:300])



def progress_marks(path):
    """How many turns in this transcript did REAL WORK.

    ⛔ "Did the file grow" is NOT "did the agent work", and the difference is a
    ten-hour outage. A refused turn ("Prompt is too long") writes three rows in
    5-66 ms, so `size > last_size` is TRUE for a session that is dead — which
    reset the booter's anti-flap counter on every tick. Fingerprint in
    booter.log: every boot is BOOT#1, never #2, so MAX_BOOTS could never fire
    for the real reason. Verified 2026-08-10: 8 boots in the incident window,
    all #1 (whole log: 43x#1, 6x#2, 4x#3 — the counter only ever accumulates on
    sessions whose file is NOT growing).

    A mark is a turn that used a tool or actually spent output tokens. An error
    reply does neither, so a corpse's marks stay flat while its bytes climb.
    """
    try:
        rows = [json.loads(l) for l in Path(path).open() if l.strip()]
    except Exception:
        return 0
    n = 0
    for r in rows:
        if r.get("type") != "assistant":
            continue
        msg = r.get("message") or {}
        items = [c for c in (msg.get("content") or []) if isinstance(c, dict)]
        if any(c.get("type") == "tool_use" for c in items):
            n += 1
            continue
        if (msg.get("usage") or {}).get("output_tokens", 0):
            n += 1
    return n


def row_exists(gui_host, ident):
    """Is this row still IN THE LIVE ORDER?

    ⛔⛔ WITHOUT THIS, A RETIRED ROW READS AS A WEDGED ONE — measured 2026-08-08,
    on my own first use. The yggterm row had been retired by its campaign's baton
    relay; its transcript is frozen MID-TURN forever, so the classifier called it
    `STUCK` for 54 minutes and I reported that to the owner as a live wedge. It
    was a corpse.
    ⇒ **A transcript cannot distinguish KILLED from WEDGED.** Only the row list
      can, and it must be consulted FIRST. (It also explains a `submitted:false`
      that looked like a busy row refusing input: there was no row.)

    ⛔ TRI-STATE, AND THE THIRD VALUE IS THE IMPORTANT ONE. True = listed.
    False = the plane answered and this row was not in it. **None = the plane did
    not answer at all**, which is a fact about US and must never be rendered as a
    verdict about the row."""
    if not gui_host:
        return None
    reply = ygg(gui_host, "server", "app", "rows")
    if not isinstance(reply, dict) or "data" not in reply:
        return None
    return resolve_row_path(gui_host, ident) is not None



# ⭐ THE CONTEXT GAUGE — the one state a boot can never fix.
#
# ⛔ AN ERROR RETURNED FASTER THAN A SUCCESS LOOKS LIKE HEALTH to anything that
# measures ACTIVITY rather than OUTCOME. A context-exhausted session answers
# "Prompt is too long" in 5-66 ms, writing three rows, which resets the
# transcript mtime -- so `turn_state`'s age goes to ~0 and this classifier
# called it `WORKING 0.1m` about a session that had been dead for two hours.
# Measured 2026-08-10: the booter kicked that corpse every ~10 min for TEN
# HOURS and the owner found it by looking at a screen.
#
# An external watchdog cannot see a token count -- it exists only inside the
# CLI -- which is why inferring from mtimes was the only option. It is not any
# more: a UserPromptSubmit hook writes the real number on every prompt, so this
# becomes a lookup costing one open().
#   ~/.claude/context-gauge/<session_id>.json
#   {"pct":98,"used":976493,"window":1000000,"verdict":"CRITICAL","dead":true}
# `dead` means the transcript tail already carries "Prompt is too long".
#
# ⚠ STALENESS IS OURS TO HANDLE: the file is only as fresh as that row's LAST
# PROMPT. So a missing or stale gauge must never be read as "healthy" -- it is
# simply no information, and we fall through to the transcript classifier.
CONTEXT_GAUGE = Path.home() / ".claude" / "context-gauge"


def context_gauge(uuid):
    """The row's own report of its context budget, or None if it never said."""
    try:
        return json.loads((CONTEXT_GAUGE / f"{uuid}.json").read_text())
    except Exception:
        return None


def classify(uuid, host=None):
    # ⛔ THE GAUGE BEFORE THE TRANSCRIPT, for the same reason the row list comes
    #    before both: a corpse's transcript lies about liveness, and this is the
    #    one state where BOOTING IS GUARANTEED TO FAIL FOREVER rather than merely
    #    being useless. It gets its own terminal state so the caller can stop.
    g = context_gauge(uuid)
    if g and g.get("dead"):
        return {"state": "CONTEXT_DEAD", "age": 0, "path": None,
                "tail": f"context exhausted: {g.get('used')}/{g.get('window')} "
                        f"({g.get('pct')}%) — unrecoverable, relay it"}
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
    # ⭐ RATE_LIMITED passes through untouched, deliberately. It is not a point on
    #    the idle/working axis — the row is neither stalled nor working, the
    #    ACCOUNT is out of quota — so ageing it into IDLE would put it right back
    #    on the path that boots it.
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


def _field(out, name):
    try:
        d = json.loads(out[out.find("{"):])
    except Exception:
        return None
    return (d.get("data") or d).get(name)


def nudge(host, row, dry):
    """Send exactly one `continue`. Never more, never to a mid-turn row.

    ⛔⛔ FIXED 2026-08-09, and it had been lying since this file was written:
    `"submitted" in stdout` IS TRUE FOR `"submitted": false`. Every nudge this
    function ever reported as sent was reported the same way whether it landed or
    not — a watchdog whose success field is a substring match on the word for
    failure. **Read the FIELD'S VALUE.**

    ⚠ And `terminal submit` drives the GUI's MOUNTED terminal host, so a row with
    nothing mounted waits out its 30s deadline and honestly answers
    `submitted:false` — which is most rows a babysitter looks at. Fall back to
    `server terminal write`, which addresses the PTY itself. Measured on a live
    row: submit `submitted:false`, PTY write delivered, same minute.

    ⛔⛔ ALSO FIXED 2026-08-09, reported while watching it fail: **the Enter
    is a SEPARATE write of `\r`, and this sent one concatenated `\n`.** Two
    mistakes in one line. `\n` is not Enter — an agent CLI runs its tty in RAW
    mode, so Enter is CR and a bare LF is inserted as a literal newline; his
    words: *"I just saw `continue, the booter booted` and an empty line. The enter
    key did not send the prompt."* And `text + "\r"` in ONE write does not submit
    either: yggterm's own `server app terminal submit` (`shell.rs:76405`) writes
    the text, sleeps 80 ms, then writes `"\r"` discretely, because *"codex treats
    a `\r` concatenated with the text in one write as a pasted newline (composer
    content), not a submit"*. ⇒ The product already knew how to press Enter; both
    watchdogs had invented their own encoding of it.

    ⇒ PTY FIRST. In `ygg-booter.py`'s log every boot took the fallback —
    `pty-write`, 5 for 5 — so a composer attempt on an unmounted row is not a
    cheap first try, it is a 30-second stall in front of the thing that works.

    Even so: `submitted:true` describes the WRITE, never the delivery. The only
    proof is transcript GROWTH on the next pass — that is what `last_size` is."""
    if dry:
        log(f"DRY-RUN would nudge {row}")
        return "dry-run"
    binp = "$HOME/.yggterm/bin/yggterm"

    def write(payload):
        out = subprocess.run(
            ["ssh", host, f"{binp} server terminal write '{row}' --stdin"],
            input=payload, capture_output=True, text=True, timeout=180)
        return _field(out.stdout or "", "accepted") is True

    if write("continue"):
        time.sleep(0.08)
        if write("\r"):
            return "pty-write"
    r = subprocess.run(
        ["ssh", host, f"{binp} server app terminal submit '{row}' --stdin"],
        input="continue", capture_output=True, text=True, timeout=180)
    return "submit" if _field(r.stdout or "", "submitted") is True else ""


_ROWS_CACHE = {}


def resolve_row_path(host, ident):
    """Turn ANY session identifier into a real ROW PATH, or None.

    ⛔⛔ THE TRAP THIS CLOSES, reported 2026-08-08: *"Clicking these delegate
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
    ap.add_argument("--host", default=None)   # ⛔ resolved, never a placeholder
    ap.add_argument("--notify-session", default=os.environ.get("YGGTERM_SESSION_ID", ""))
    ap.add_argument("--watch", type=int, default=0,
                    help="seconds to keep watching; 0 = one pass")
    ap.add_argument("--interval", type=int, default=180)
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args()
    # ⛔ Resolve the GUI host OUT LOUD before any row is judged. App control
    # only answers where the GUI runs; an unresolved host makes every
    # `row_exists` call fail, and this tool renders that as GONE/RETIRED —
    # a TERMINAL verdict about live rows, which stops the supervision.
    args.host = resolve_gui_host(args.host)

    rows = list(args.row)
    if args.spawned_by:
        f = STATE / f"spawned-by-{args.spawned_by}.txt"
        if f.exists():
            rows += [l.strip() for l in f.read_text().splitlines() if l.strip()]
    # ⭐ Normalise to ROW PATHS before anything judges a row, keeping the ledger
    #    line only as a display label. Every consumer below — row_host,
    #    row_exists, escalate, nudge — addresses a row, and a ledger line is not
    #    an address. See ledger_row_path.
    labels = {}
    paths = []
    for entry in rows:
        path = ledger_row_path(entry)
        if not path:
            continue
        if path not in labels:
            paths.append(path)
        # A seat-bearing ledger line is the more useful label; keep it if we have one.
        if path not in labels or entry != path:
            labels[path] = entry
    rows = sorted(set(paths))
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
            exists = row_exists(args.host, row)
            if exists is None:
                # ⛔⛔ BLIND, NOT GONE — and the difference is the whole point.
                # `GONE`/`RETIRED` is terminal: a supervisor that reaches it stops
                # supervising. Measured 2026-08-13 from two campaigns in one hour:
                # an unresolvable GUI host made this report live, subscribed,
                # working rows as retired, and a wave was one step from being
                # treated as finished. A watchdog that cannot tell "I could not
                # look" from "it is dead", and resolves that toward standing down,
                # is worse than no watchdog. BLIND is non-terminal: keep watching.
                report.append({"row": labels.get(row, row), "state": "BLIND", "age_min": 0,
                               "action": "STILL-WATCHING", "tail":
                               "row plane unreachable — this says nothing about the row"})
                continue
            if not exists:
                # ⛔ Ask the ROW LIST before the transcript. A retired row's
                #    transcript is frozen mid-turn and is indistinguishable from
                #    a live wedge.
                report.append({"row": labels.get(row, row), "state": "GONE", "age_min": 0,
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
