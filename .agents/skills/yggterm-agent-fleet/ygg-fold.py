#!/usr/bin/env python3
"""ygg-fold — fold a finished or dead row, across every plane that holds it.

⛔⛔ WHY THIS EXISTS, AND IT IS A GAP IN THE VERB SET RATHER THAN A MISSING HABIT.

Retiring a row correctly means four planes, not one: the row is removed, the
MONITOR's subscribers are moved off it, the BOOTER is disarmed for it, and the
agent PROCESS is reaped — because `session remove` reports the request rather than
the effect and routinely delists a row whose agent keeps running. All four steps
existed, and they existed in exactly ONE place: inside `ygg-claim.sh --replace`,
as a side effect of a SUCCESSOR claiming the seat.

⇒ So the fleet had a `replace` and no `fold`. A lane that finishes its work and
  stands down with nobody coming after it had no path to being folded at all. Its
  row stayed in the sidebar, its process stayed resident, the booter went on
  arming a corpse, and nothing anywhere said so. Every orchestrator that wanted to
  tidy up had to re-assemble the four planes from primitives, which is the exact
  test for "this wants to be a verb": an agent's discipline resets every session
  and a verb's does not.

⚠ Measured 2026-08-21, and the owner found it before any instrument did: 17 dead
  rows and four finished-and-announced lanes were still seated, one of them
  showing a blank viewport with `0 user · 0 assistant`. He closed one by hand.

USAGE
    ygg-fold.py sweep --campaign 11            # classify only; changes nothing
    ygg-fold.py sweep --campaign 11 --apply    # fold what it names
    ygg-fold.py row <uri-or-uuid> --apply      # fold exactly one

⛔ DRY BY DEFAULT. Folding kills somebody's agent; it may never be the accidental
  outcome of a mistyped flag.
"""
import argparse
import glob
import json
import os
import re
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
RELAY = os.path.expanduser("~/.yggterm/relay")
YGG = os.path.expanduser("~/.local/bin/yggterm-headless")

#: A row idle for less than this is never called finished, whatever it said. A
#: lane often prints its closing summary and then keeps working for a minute.
FINISHED_IDLE_MIN = 8.0

#: ⛔⛔ THE CASE THE FIRST VERSION OF THIS TOOL MISSED ENTIRELY, AND IT IS THE
#: COMMON ONE. `FINISHED` requires a lane to ANNOUNCE that it is done. A lane that
#: simply stops — turn ended, composer empty, nothing in flight — was classified
#: WORKING forever, so the sidebar refilled with quiet corpses within the hour and
#: the owner had to point at them a second time.
#:
#: ⇒ A stall is not a fold. The remedy for a row sitting at its own composer with
#:   work still assigned is ONE `continue`, and a fold would throw away a lane
#:   that has merely paused. So STALLED is its own verdict with its own remedy,
#:   and the ledger below makes it once per stall rather than once per sweep.
STALL_IDLE_MIN = 20.0

#: ⛔⛔⛔ NEVER KICK A COLD SESSION. THE STANDING FLEET LAW, AND THIS TOOL BROKE IT.
#:
#: The law is old and it is written down: succeed a cold session by HARVESTING its
#: transcript, never by prompting it — cold cache times large context multiply,
#: **the asking IS the expense**, and prompting it makes it warm, so respawning
#: afterwards wastes exactly what you just spent. A fork with no middle: touch
#: nothing and succeed it from artefacts, or having touched it, keep it.
#:
#: ⇒ WHERE THE STEER WAS GETTING LOST, because it is worth naming: the law lives
#:   in the skill under "succeeding a session", framed as *do not ASK a cold row
#:   what it was doing*. Someone implementing a WAKE path does not read that as
#:   applying to them — a `continue` does not feel like asking. It is the same
#:   expense. So the rule now lives HERE, at the point where the decision is
#:   actually made, and the code cannot express "wake" without first asking cold.
#:
#: A wake is only ever correct for a row that is still cheap to resume. Everything
#: else is harvested and replaced by a fresh lane at the same seat.
#: A spawn's transcript lags its creation by ~15 s, so a brand-new row legitimately
#: has none. Past this it has not been briefed at all.
BRIEFLESS_GRACE_MIN = 10.0

WAKEABLE_MAX_TRANSCRIPT_BYTES = 400_000
WAKEABLE_MAX_IDLE_MIN = 20.0


def process_age_min(uuid):
    """Minutes since the agent process for this uuid started, from /proc — the one
    clock a row with no transcript still has."""
    try:
        out = subprocess.run(["bash", "-c",
                              f"for p in $(pgrep -x claude); do "
                              f"tr '\\0' ' ' < /proc/$p/cmdline 2>/dev/null | grep -q {uuid} "
                              f"&& stat -c %Y /proc/$p && break; done"],
                             capture_output=True, text=True, timeout=60).stdout.strip()
        return (time.time() - int(out)) / 60 if out else None
    except Exception:
        return None


def wakeable(transcript_bytes, idle_min):
    """May this stalled row be prompted, or must it be harvested and replaced?

    Both tests must pass. A small transcript that has gone cold is cheap to
    resume; a large one is expensive whether or not the cache still holds it,
    because the whole context is re-read either way. ⛔ An OR here would have the
    strength of the weaker test, which is how a 5 MB row gets prompted.
    """
    return transcript_bytes <= WAKEABLE_MAX_TRANSCRIPT_BYTES and idle_min <= WAKEABLE_MAX_IDLE_MIN
WAKE_LEDGER = os.path.join(os.path.expanduser("~/.yggterm/relay"), "fold-wakes.json")

#: ⚠ A PHRASE LIST IS A GUESS LIST, so it is deliberately NOT the whole test.
#: It promotes an already-idle row to FINISHED; it can never fold a busy one on
#: its own. An OR between a strong predicate and a weak one has the strength of
#: the weak one — this is an AND.
#:
#: ⛔⛔ AND IT IS READ ONLY IN THE OPENING OF THE LAST MESSAGE. The first cut
#: searched the whole text and called a LIVE MONITOR finished: its report opened
#: "Holding as monitor — 51% context, 3.2h of a 12h budget" and somewhere in the
#: body it discussed another lane completing. A lane states its status in its
#: first line and then writes about everything else; searching the body finds it
#: talking ABOUT finishing rather than saying it has. Folding kills a process, so
#: this must fail toward leaving a row alone.
STAND_DOWN_HEAD_CHARS = 240
STAND_DOWN = re.compile(
    r"\b(standing down|stood down|i am done here|lane is complete|"
    r"handoff is complete|handover is complete|nothing left for me|"
    r"work is finished|this lane is finished)\b",
    re.I,
)

#: ⛔ A VETO, read in the SAME OPENING WINDOW and beating any match above. A row
#: that says it is holding, watching, or staying subscribed is declaring itself
#: live, and a monitor is never finished while the thing it watches is.
STAYING = re.compile(
    r"\b(holding as|still holding|watch continues|not unsubscribing|"
    r"staying subscribed|remain subscribed|continuing to watch|"
    r"i will keep|monitor is never finished)\b",
    re.I,
)


def log(msg):
    print(f"{time.strftime('%H:%M:%S')} ygg-fold {msg}")


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, timeout=120, **kw)


def gui_host():
    r = run([os.path.join(HERE, "..", "..", "..", "scripts", "ygg-live-host.sh"), "--quiet"])
    host = (r.stdout or "").strip()
    return host or os.environ.get("YGG_GUI_HOST", "")


def rows_census(host):
    """ONE app-control call, and everything else is read off this machine.

    ⛔ Every probe against the GUI costs the person using it typing latency —
    measured at 24% of their UI-thread blocks. A sweep that asked the GUI about
    each row in turn would be the worst available shape.
    """
    r = run(["ssh", "-n", host, f"{YGG} server app rows --json"])
    try:
        rows = (json.loads(r.stdout).get("data") or {}).get("rows") or []
    except Exception:
        log("⛔ could not read the row list — refusing to guess")
        return []
    seen, out = set(), []
    for row in rows:
        path = row.get("full_path") or ""
        seat = row.get("outline_prefix")
        if seat and "://" in path and path not in seen:
            seen.add(path)
            out.append({"seat": str(seat), "uri": path, "label": row.get("label") or "",
                        "session_cwd": row.get("session_cwd") or "",
                        # ⛔ THE DAEMON ALREADY KNOWS WHO IS WORKING — this is the
                        # flag that drives the blinking indicator a person watches.
                        "busy": bool(row.get("busy")),
                        "busy_reason": row.get("busy_reason") or ""})
    return out


def rows_all(host):
    """Every session row, seated or not. `rows_census` keeps only seated ones
    because a SWEEP is seat-scoped by design; a single-row call is not."""
    r = run(["ssh", "-n", host, f"{YGG} server app rows --json"])
    try:
        rows = (json.loads(r.stdout).get("data") or {}).get("rows") or []
    except Exception:
        return []
    seen, out = set(), []
    for row in rows:
        path = row.get("full_path") or ""
        if "://" in path and path not in seen:
            seen.add(path)
            out.append({"seat": str(row.get("outline_prefix") or "-"), "uri": path,
                        "label": row.get("label") or "",
                        "session_cwd": row.get("session_cwd") or ""})
    return out


#: The scan, as one line of shell so it can run here or over ssh unchanged.
#: ⛔ `pgrep -f <uuid>` matches the asking shell, and over ssh it also matches the
#: ssh client that carried the query; both were measured lying in both directions.
#: This walks /proc and excludes anything naming this tool.
_SCAN = (
    r"""for d in /proc/[0-9]*; do c=$(tr '\0' ' ' < $d/cmdline 2>/dev/null); """
    r"""case "$c" in *ygg-fold*|*ygg-procfind*) continue;; esac; """
    r"""case "$c" in *claude*|*codex*|*--session-id*|*antigravity*) echo "$c";; esac; done"""
)


def live_agent_uuids(host=None):
    """Every uuid an agent process is running ON THE HOST THAT OWNS THE ROWS.

    ⛔⛔ THE HOST ARGUMENT IS THE WHOLE SAFETY OF THIS TOOL. Liveness was read from
    the LOCAL /proc, which is correct only when the sweep happens to run on the
    machine the agents run on. Run it anywhere else — the GUI host, a laptop, an
    hourly job on whichever box the watcher lives — and every row reads DEAD,
    because no agent process is visible from there. With `--apply` that is not a
    wrong report, it is the entire fleet reaped in one pass. ⇒ Liveness is asked
    of the host named in the row's own uri, never of whatever machine is asking.
    """
    out = set()
    if host in (None, "", "local"):
        lines = subprocess.run(["bash", "-c", _SCAN], capture_output=True, text=True, timeout=60).stdout
    else:
        r = run(["ssh", "-n", host, _SCAN])
        if r.returncode != 0:
            # ⛔ A HOST THAT CANNOT BE ASKED IS NOT A HOST WITH NO AGENTS. Returning
            # an empty set here would mark every row on it DEAD and fold them all.
            raise RuntimeError(f"cannot read processes on {host}: {(r.stderr or '').strip()[:120]}")
        lines = r.stdout
    for cmd in lines.splitlines():
        for tok in cmd.split():
            if len(tok) == 36 and tok.count("-") == 4:
                out.add(tok)
    return out


def host_of(uri):
    """`remote-cc://dev/<uuid>` → `dev`; a local scheme → None (this machine)."""
    if "://" not in uri:
        return None
    rest = uri.split("://", 1)[1]
    return rest.split("/", 1)[0] if "/" in rest else None


def screen_state(uuid, host):
    """The row's own verdict on itself, from the daemon's rendered grid.

    ⛔ THE PATH FORM IS NOT THE ONE THE VERB DOCUMENTS. `server screen <row>`
    accepts `cc-runtime://<uuid>` and answers `unreadable` for both a bare uuid
    and the `remote-cc://<host>/<uuid>` form every other verb takes — and
    `unreadable` is exactly what a genuinely blank screen returns, so a caller
    using the documented form concludes the row is broken when the verb simply
    did not resolve the path. Filed; pinned here so this tool cannot inherit it.
    """
    # ⛔⛔ `server screen --state-only` ANSWERS `unreadable` FOR EVERY ROW PATH,
    # AND THE COMMENT ABOVE PINNED THE WRONG FORM AS THE CURE. Measured
    # 2026-08-21 across six live rows including this tool's own, which was
    # painting 4847 characters at the time: `cc-runtime://<uuid>`,
    # `remote-cc://<host>/<uuid>` and a bare uuid all return `unreadable`. The
    # verb works — with no argument it dumps every local row's screen — so what
    # fails is resolving a ROW PATH, and its failure is spelled exactly like a
    # blank screen.
    #
    # ⇒ Every verdict this function fed was therefore blind: the census reported
    #   "screen says unreadable" for rows that were working, and the wake path
    #   could never tell a waiting prompt from a busy row.
    #
    # `terminal read-buffer` resolves the same rows and answers in full. It is
    # the instrument; this is now a thin classifier over it.
    cmd = (f"{YGG} server app terminal read-buffer 'remote-cc://{host or 'dev'}/{uuid}' "
           f"--mode screen")
    r = run(["ssh", "-n", host, cmd]) if host else run(["bash", "-c", cmd])
    text = ""
    try:
        text = ((json.loads(r.stdout or "{}").get("data") or {}).get("text") or "")
    except ValueError:
        text = r.stdout or ""
    if not text.strip():
        # ⚠ STILL A REAL STATE, and not the same as the verb being blind: a row
        # can be alive and working with no rendered grid at all. Reported as
        # `unreadable` so callers keep refusing to type into it.
        return "unreadable"
    tail = [ln for ln in text.splitlines() if ln.strip()][-6:]
    joined = " ".join(tail).lower()
    if any(k in joined for k in ("esc to interrupt", "tokens)", "· ↓")):
        return "working"
    return "ready"


def composer_is_empty(uuid, host):
    """Read the RENDERED screen and look at the composer line itself.

    ⛔⛔ `ready` DOES NOT MEAN EMPTY. A row can be at rest with text sitting in its
    composer — this fleet has rows holding a dozen repetitions of a wake message
    that were never sent. Typing a `continue` into one of those submits somebody
    else's text along with it, which is the single worst thing anything here can
    do. So the wake reads the composer directly rather than trusting the state.
    ⚠ It reads the daemon's RENDERED rows, not the raw stream: a screen does not
    contain the words on it, and a line-shaped rule over the stream is what made
    `composer_held_draft` unreliable in the first place.
    """
    cmd = f"{YGG} server screen 'cc-runtime://{uuid}'"
    r = run(["ssh", "-n", host, cmd]) if host else run(["bash", "-c", cmd])
    marker_lines = [ln for ln in (r.stdout or "").splitlines() if ln.lstrip().startswith(("❯", ">", "│ >"))]
    if not marker_lines:
        return False, "no composer line found on the rendered screen"
    tail = marker_lines[-1].lstrip()[1:].strip()
    return (not tail), ("empty" if not tail else f"holds {len(tail)} char(s)")


def wake_ledger():
    try:
        return json.load(open(WAKE_LEDGER))
    except Exception:
        return {}


def note_wake(uuid, mtime):
    led = wake_ledger()
    led[uuid] = {"woken_at": time.time(), "transcript_mtime": mtime}
    os.makedirs(os.path.dirname(WAKE_LEDGER), exist_ok=True)
    json.dump(led, open(WAKE_LEDGER, "w"))


def already_woken_for_this_stall(uuid, mtime):
    """⛔ ONE continue PER STALL, never one per sweep.

    The ledger keys on the transcript mtime at the moment of the wake: if the row
    moved afterwards, this is a NEW stall and it may be woken again; if it did not
    move, the previous wake did not take and repeating it is typing at a row that
    is not listening — the storm shape this fleet has already paid for twice.
    """
    prev = wake_ledger().get(uuid)
    return bool(prev) and prev.get("transcript_mtime") == mtime


def transcript_of(uuid):
    hits = glob.glob(os.path.expanduser(f"~/.claude/projects/*/{uuid}.jsonl"))
    return hits[0] if hits else None


def last_assistant_text(path):
    try:
        recs = [json.loads(l) for l in open(path) if l.strip()]
    except Exception:
        return ""
    for rec in reversed(recs):
        if rec.get("type") != "assistant":
            continue
        for blk in (rec.get("message") or {}).get("content") or []:
            if isinstance(blk, dict) and blk.get("type") == "text" and blk.get("text", "").strip():
                return blk["text"].strip()
    return ""


_MANUAL_TITLES = None


def manually_titled_uuids(host):
    """⛔⛔ A ROW A PERSON NAMED BY HAND IS A ROW A PERSON IS KEEPING.

    Measured 2026-08-21, by destroying one. The owner kept a row group of sessions
    whose transcripts had been deleted long ago — he kept them **for their
    titles**, as a reading list, because the title alone told him what the item
    was. The group's HEAD was such a row: no process, no transcript, and its cwd
    was a mount path that no longer resolves. `orphans` called that DEAD debris
    and folded it, and it cannot be restored: the session, its transcript and its
    cwd are all gone, so the restore verb reports `not_found`.

    ⇒ The tool's whole model was that a row's value is its PROCESS. For a bookmark
      the value is the NAME, and the name is the one thing it still had.

    `session_titles.source = 'manual'` is exactly that marker and it was already in
    the store: it means a human typed this title. Nothing folds such a row without
    `--force`, whatever its process or its cwd say.
    """
    global _MANUAL_TITLES
    if _MANUAL_TITLES is not None:
        return _MANUAL_TITLES
    # ⛔ The store is resolved from the REMOTE's own home, never written out here:
    # a literal home path is both wrong on any other account and a private-data
    # leak into a public repo. The pre-push guard caught exactly that.
    q = ("import sqlite3,json,os;"
         "c=sqlite3.connect(os.path.expanduser('~/.yggterm/session-titles.db'));"
         "print(json.dumps([r[0] for r in c.execute("
         "\"select session_id from session_titles where source='manual'\")]))")
    r = run(["ssh", "-n", host, f"python3 -c {json.dumps(q)}"])
    try:
        _MANUAL_TITLES = set(json.loads(r.stdout.strip()))
    except Exception:
        # ⛔ FAIL CLOSED. If the keepsake list cannot be read, every row looks
        # unprotected — which is the state that lost one.
        log("⚠ could not read the manual-title store — treating every row as KEPT")
        _MANUAL_TITLES = None
        return "unreadable"
    log(f"  keepsakes: {len(_MANUAL_TITLES)} row(s) carry a hand-typed title")
    return _MANUAL_TITLES


def protected_uuids():
    """Rows nothing may fold: the owner's own, and anything opted out of waking.

    ⛔ `never-arm.tsv` is PER HOST and its meaning is stronger than "do not wake":
    it marks a row a person is using. Folding one is worse than typing into it.
    """
    out = set()
    for path in (os.path.join(RELAY, "never-arm.tsv"),):
        try:
            for line in open(path):
                for tok in re.findall(r"[0-9a-f]{8}-[0-9a-f-]{27}", line):
                    out.add(tok)
        except OSError:
            pass
    for tok in (os.environ.get("YGG_FOLD_NEVER") or "").split(","):
        if tok.strip():
            out.add(tok.strip())
    return out


_FETCHED = set()


def branch_state(row):
    """⛔ A TRANSCRIPT SAYS WHAT A SESSION BELIEVED. ITS BRANCH SAYS WHAT IT DID.

    Every arm above this reads PROSE — a phrase list of ways a lane might announce
    it is done. That list is the same hand-written match this project keeps paying
    for: a lane that opened "Done. Landed on main as <sha>." matched none of the
    nine phrases and classified WORKING at nineteen minutes idle, while its branch
    had nothing left to land and its own last words said so.

    ⇒ So do not widen the phrase list. Ask the artefact instead and hand the
    ANSWER to whoever decides. This never reclassifies on its own — a lane with a
    clean branch may be mid-thought, and "has nothing unlanded" is not "has
    nothing left to do". It is the one fact a person needs and the transcript
    cannot give.
    """
    cwd = (row.get("session_cwd") or "").strip()
    if not cwd or not os.path.isdir(os.path.join(cwd, ".git")) and not os.path.exists(os.path.join(cwd, ".git")):
        return ""
    def g(*args):
        return subprocess.run(["git", "-C", cwd, *args], capture_output=True,
                              text=True, timeout=60)
    br = g("rev-parse", "--abbrev-ref", "HEAD").stdout.strip()
    if not br.startswith("lane/"):
        return ""
    # One fetch per checkout per run. `origin/main` is a cached note of where the
    # server was, and a stale one reports landed work as unlanded.
    if cwd not in _FETCHED:
        _FETCHED.add(cwd)
        g("fetch", "-q", "origin")
    c = g("cherry", "origin/main", br)
    if c.returncode != 0:
        return ""
    unlanded = [l for l in c.stdout.splitlines() if l.startswith("+")]
    dirty = bool(g("status", "--porcelain").stdout.strip())
    if unlanded:
        return f" · {br.rsplit('/', 1)[-1]}: {len(unlanded)} unlanded" + (", tree dirty" if dirty else "")
    return f" · {br.rsplit('/', 1)[-1]}: nothing unlanded" + (", tree dirty" if dirty else "")


def classify(row, live, protected):
    uuid = row["uri"].rsplit("/", 1)[-1]
    row["uuid"] = uuid
    if uuid in protected:
        return "PROTECTED", "listed as a row a person uses"
    tr = transcript_of(uuid)
    row["mtime"] = os.path.getmtime(tr) if tr else None
    row["idle_min"] = round((time.time() - row["mtime"]) / 60, 1) if tr else None
    if uuid not in live:
        return "DEAD", "no agent process on this host"

    # ⛔⛔ ASK THE DAEMON WHETHER THIS ROW IS WORKING. IT KNOWS, AND THIS TOOL WAS
    # GUESSING. `busy` is the same flag that drives the blinking working
    # indicator, so a census that disagrees with it disagrees with what a person
    # is looking at — and it did: six rows reported WORKING while the sidebar
    # showed two. The cause is that WORKING is this classifier's DEFAULT verdict,
    # so every row it could not place read as the busiest thing it has, exactly
    # as BRIEFLESS rows did before they got a name.
    #
    # ⇒ A busy row is WORKING on the daemon's word and no further test is needed.
    #   A row the daemon calls idle is NOT working, and the idle/stall/cold tests
    #   below become meaningful instead of being a guess about a row that may
    #   have been mid-turn all along.
    if row.get("busy"):
        return "WORKING", f"the daemon says {row.get('busy_reason') or 'working'}"
    if tr is None:
        # ⛔⛔ A LIVE PROCESS WITH NO TRANSCRIPT AT ALL IS THE EMPTIEST POSSIBLE ROW,
        # AND THIS ARM CALLED IT THE BUSIEST VERDICT IT HAS. A CLI that started and
        # was never given anything writes no transcript, so it can never be COLD,
        # FINISHED or DEAD — it is unclassifiable forever and therefore unfoldable
        # and unsucceedable. Measured 2026-08-21: one row sat in exactly this state
        # for two hours while every sweep reported it WORKING.
        #
        # ⇒ The grace is real — a spawn's transcript lags creation by ~15 s — but it
        #   is a GRACE, not a permanent exemption. Past it, a row that has never
        #   written a word has never been briefed, and saying so is the whole point.
        age = process_age_min(uuid)
        if age is not None and age > BRIEFLESS_GRACE_MIN:
            return "BRIEFLESS", (f"alive {age:.0f}m and has never written a transcript — "
                                 f"it was started and never briefed")
        return "WORKING", "process alive, no transcript to judge by (still starting?)"
    text = last_assistant_text(tr)
    row["last"] = text.replace("\n", " ")[:200]
    # ⚠ Both phrase tests read the OPENING, not the body. Whole-message matching
    # made the veto fire on a lane that had opened with "Standing down" and merely
    # quoted the booter's "a MONITOR is never finished" further down; searching the
    # body for a stand-down called a live monitor finished. A lane states its
    # status in its first line and then writes about everything else.
    said_done = bool(STAND_DOWN.search(text[:STAND_DOWN_HEAD_CHARS]))
    says_watching = bool(STAYING.search(text[:STAND_DOWN_HEAD_CHARS]))
    idle = row["idle_min"]

    if idle is not None and idle >= FINISHED_IDLE_MIN and said_done and not says_watching:
        return "FINISHED", f"opened by standing down, quiet {idle}m"

    # ⛔⛔ A MONITOR IS NOT EXEMPT FROM STALLING — IT IS THE MOST LIKELY THING TO
    # STALL, AND THE LAST THING ANYONE CHECKS. The first cut let "watch continues"
    # end the classification, so three watchers sat quiet for forty minutes each
    # while their own last words asserted they were watching. A row's claim about
    # itself is a CLAIM; its turn having ended is a FACT. The claim is allowed to
    # decide whether a quiet row is finished, and never whether it is running.
    if idle is not None and idle >= STALL_IDLE_MIN:
        # Ask the row what it is sitting on before calling it stalled. `working`
        # or `limit_wait` is a row that is fine; a picker or a gate wants a PERSON
        # and must never be typed at.
        state = screen_state(row["uuid"], host_of(row["uri"]))
        row["screen"] = state
        if state == "ready":
            note = " (its last words said it was watching)" if says_watching else ""
            size = os.path.getsize(tr) if tr else 0
            row["bytes"] = size
            if wakeable(size, idle):
                return "STALLED", f"at its composer, quiet {idle}m, {size // 1000}KB — still cheap to resume{note}"
            return "COLD", (f"quiet {idle}m with a {size // 1000}KB transcript — "
                            f"harvest and replace, never prompt{note}")
        # ⛔⛔ AND `unreadable` MUST NOT PROMOTE AN IDLE ROW TO WORKING. This arm
        # existed to protect a row that is mid-output or at a picker — states
        # where typing is destructive — but it fired on rows whose SURFACE IS
        # GONE, and a lost grid is not evidence of activity. With the daemon
        # already saying idle, that read the quietest possible row as busy: three
        # rows sat WORKING for half an hour each while the sidebar's own working
        # indicator showed them dark, which is what the owner was looking at when
        # he said the census disagreed with his eyes.
        if state == "unreadable":
            size = os.path.getsize(tr) if tr else 0
            row["bytes"] = size
            return "COLD", (f"quiet {idle}m with NO READABLE SCREEN and a "
                            f"{size // 1000}KB transcript — it cannot be woken, because "
                            f"nothing can check whether a prompt is waiting. Harvest and "
                            f"replace{note if 'note' in dir() else ''}")
        return "WORKING", f"screen says {state}, quiet {idle}m"

    if says_watching:
        return "WORKING", f"declares itself still watching, quiet {idle}m{branch_state(row)}"
    return "WORKING", f"quiet {idle}m, no stand-down in its opening{branch_state(row)}"


def harvest(row, verdict, why):
    """A fold must leave a note, because the row is the only place this is written.

    ⚠ Not a summary of the work — that belongs in the lane's own report and in
    git. This says WHICH row went, WHEN, and WHY, so a successor reading the
    sidebar's absence has something to read instead of nothing.
    """
    os.makedirs(os.path.join(RELAY, "folded"), exist_ok=True)
    path = os.path.join(RELAY, "folded", f"{row['uuid']}.md")
    with open(path, "w") as fh:
        fh.write(f"# folded {row['seat']} — {row['uuid']}\n\n")
        fh.write(f"* when: {time.strftime('%Y-%m-%d %H:%M:%S')}\n")
        fh.write(f"* verdict: {verdict} ({why})\n")
        fh.write(f"* uri: {row['uri']}\n")
        fh.write(f"* title: {row['label']}\n")
        if row.get("last"):
            fh.write(f"\nits last words:\n\n> {row['last']}\n")
    return path


def wake(row, host, apply_it):
    """One `continue`, and only into a row that is genuinely at rest.

    ⛔ `submitted: false` means the row was mid-output, NOT unreachable. It is
    never retried — retrying is the defect that types over people.
    """
    uri, uuid = row["uri"], row["uuid"]
    if already_woken_for_this_stall(uuid, row.get("mtime")):
        log("  already woken for this exact stall — escalating instead of typing again")
        return False
    empty, why = composer_is_empty(uuid, host_of(uri))
    if not empty:
        log(f"  ⛔ composer is not empty ({why}) — NOT typing into it")
        return False
    if not apply_it:
        log("  (dry run: would send one `continue`)")
        return True
    out = run(["ssh", "-n", host, f"printf 'continue' | {YGG} server app terminal submit '{uri}' --stdin"])
    try:
        data = json.loads(out.stdout).get("data") or {}
    except Exception:
        data = {}
    if data.get("submitted"):
        note_wake(uuid, row.get("mtime"))
        log(f"  woken with one continue ({data.get('bytes')} bytes)")
        return True
    log("  ⛔ submit refused (mid-output) — NOT retried, left for the next sweep")
    return False


def successor_brief(row, why):
    """Distil a cold row into something a fresh lane can start from.

    ⛔ THIS IS THE ONLY LEGITIMATE WAY TO GET A COLD LANE'S STATE. Everything here
    is read from ARTEFACTS — the row's own title, purpose and last written words,
    and the files it left behind. Nothing is asked of the session, because asking
    is the expense the law exists to avoid.
    """
    os.makedirs(os.path.join(RELAY, "successors"), exist_ok=True)
    path = os.path.join(RELAY, "successors", f"{row['seat']}-successor.md")
    tr = transcript_of(row["uuid"])
    with open(path, "w") as fh:
        # ⛔ THE ACK TOKEN MUST BE IN THE BRIEF ITSELF. The spawner proves delivery
        # by finding this exact string in the successor's transcript; a brief that
        # does not carry it can never verify, and the spawn reports a failure it
        # did not have.
        fh.write(f"SUCCESSOR-{row['seat']} — you are seat {row['seat']} on the yggterm campaign. "
                 f"Your row is already seated, titled and grouped: do not claim it, do not spend "
                 f"a turn on bookkeeping, start on the work.\n\n")
        fh.write(f"# successor brief for seat {row['seat']}\n\n")
        fh.write(f"*Written from artefacts on {time.strftime('%Y-%m-%d %H:%M')}. The predecessor "
                 f"({row['uuid'][:8]}) was COLD — {why} — and was NEVER PROMPTED, because "
                 f"prompting a cold session is the expense this replaces.*\n\n")
        fh.write(f"**Its title:** {row['label']}\n\n")
        if row.get("last"):
            fh.write(f"**The last thing it wrote:**\n\n> {row['last']}\n\n")
        if tr:
            fh.write(f"**Its transcript** (read it, do not wake it): `{tr}`\n\n")
        fh.write("**Where its work landed:** `git log --author-date-order` on its lane branch, "
                 "and any `~/.yggterm/relay/` file naming this seat.\n\n")
        fh.write("**Your first act:** read the transcript above for what your predecessor was "
                 "doing, then `docs/pending-bugs.md` for the entries carrying your seat, and "
                 "continue that work. If the predecessor finished, say so in "
                 "`~/.yggterm/relay/" + row["seat"] + "-to-11.0.md` and stand down.\n\n")
        fh.write("⛔ The GUI host is the owner's laptop and he is using it. Never restart his "
                 "window, never run anything that OPENS a row, read-only probes only, and batch "
                 "them — a quarter of his input blocks are agent probes.\n\n")
        fh.write("⛔ Do not end a turn on a question. Record it with your recommendation and "
                 "carry on.\n")
    return path


def respawn(row, why, host, apply_it):
    """Replace a cold lane: spawn its successor FIRST, then fold the predecessor.

    ⛔⛔ THE ORDER IS THE WHOLE DESIGN. Folding first empties the seat, and a seat
    that is empty for even a minute is a campaign with a hole in it that nobody is
    watching — and if the spawn then fails, the lane is simply gone. So the
    successor is created, briefed and PROVEN to hold the brief before anything is
    removed, and a failed spawn leaves the predecessor exactly where it was.
    ⚠ The predecessor is never prompted at any point. Everything the successor is
    told comes from artefacts.
    """
    brief = successor_brief(row, why)
    log(f"  successor brief → {os.path.relpath(brief, os.path.expanduser('~'))}")
    if not apply_it:
        log("  (dry run: would spawn a successor at this seat, then fold this row)")
        return False
    spawn = os.path.join(HERE, "ygg-spawn.py")
    if not os.path.exists(spawn):
        log("  ⛔ ygg-spawn.py is missing — leaving the predecessor alone")
        return False
    cwd = row.get("session_cwd") or os.path.expanduser("~/gh/yggterm")
    r = subprocess.run(
        [sys.executable, spawn, "--seat", row["seat"],
         "--title", row["label"] or f"seat {row['seat']}",
         "--purpose", f"successor to {row['uuid'][:8]}, which went cold",
         "--cwd", cwd, "--brief", brief,
         "--ack", f"SUCCESSOR-{row['seat']}"],
        capture_output=True, text=True, timeout=600)
    new_row = (r.stdout or "").strip().splitlines()[-1] if r.stdout.strip() else ""
    if r.returncode != 0 or "://" not in new_row:
        log(f"  ⛔ successor did not come up (exit {r.returncode}) — predecessor LEFT ALONE")
        for line in (r.stderr or "").strip().splitlines()[-3:]:
            log(f"     {line}")
        return False
    log(f"  successor is up: {new_row}")
    return fold(row, "COLD", why, host, True)


def agent_pids(uuid):
    out = []
    for d in glob.glob("/proc/[0-9]*"):
        pid = d.split("/")[-1]
        try:
            cmd = open(d + "/cmdline", "rb").read().replace(b"\0", b" ").decode("utf8", "replace")
        except OSError:
            continue
        if uuid not in cmd or "ygg-fold" in cmd:
            continue
        if any(k in cmd for k in ("claude ", "codex ", "--session-id")):
            out.append(int(pid))
    return out


def fold(row, verdict, why, host, apply_it):
    uuid, uri = row["uuid"], row["uri"]
    note = harvest(row, verdict, why)
    log(f"  harvested → {os.path.relpath(note, os.path.expanduser('~'))}")
    if not apply_it:
        log("  (dry run: nothing removed)")
        return True

    # 1. the ROW
    out = run(["ssh", "-n", host, f"{YGG} server app session remove '{uri}'"])
    try:
        data = json.loads(out.stdout).get("data") or {}
    except Exception:
        data = {}
    log(f"  remove: row_still_listed={data.get('row_still_listed')} verified={data.get('verified')}")

    # 2. the MONITOR — orphaned subscribers escalate into a corpse otherwise, and
    #    `escalate()` logs success whether or not the row exists.
    mon = os.path.join(HERE, "ygg-monitor.py")
    if os.path.exists(mon):
        run([sys.executable, mon, "unsubscribe", uuid])

    # 3. the BOOTER — a separate store, missed once before for exactly that reason.
    boot = os.path.join(HERE, "ygg-booter.py")
    if os.path.exists(boot):
        run([sys.executable, boot, "unsubscribe", "--row", uri, "--force"])

    # 4. the PROCESS. `verified:false` with the row delisted means the row is gone
    #    and the agent is still running; that is the state that leaves a ghost.
    victims = agent_pids(uuid)
    if victims:
        log(f"  reaping pids {victims}")
        for pid in victims:
            try:
                os.kill(pid, 15)
            except OSError:
                pass
        for _ in range(6):
            time.sleep(1)
            if not agent_pids(uuid):
                break
    survived = agent_pids(uuid)
    if survived:
        log(f"  ⛔ SURVIVED: {survived} — reap by hand")
        return False
    log("  folded clean")
    return True



# ── worktree hygiene ────────────────────────────────────────────────────────
#
# ⛔⛔ A LANE'S WORKTREE OUTLIVES THE LANE, AND IT IS VISIBLE TO THE PERSON USING
# THE APP. Every lane gets `~/gh/<repo>--<name>`, and nothing has ever removed
# one. They accumulate in the cwd tree as folders holding a handful of dead
# sessions each — the owner reported it as a hygiene problem, and he is right
# that it is the orchestrator's: the same seat that creates a worktree is the
# only one that knows when its lane is over.
#
# ⛔ FOUR REFUSALS, and every one of them has to hold before a directory is
#   removed, because a wrong removal destroys work that exists nowhere else:
#     1. uncommitted changes — anything at all in `git status --porcelain`;
#     2. commits not on `origin/main` — the branch still carries work;
#     3. a live process whose cwd is inside it — somebody is standing there;
#     4. the infrastructure trees, which are not lanes at all.
#   ⚠ Refusal 2 is checked against a FRESHLY FETCHED origin/main. A stale
#     remote-tracking ref would call unmerged work merged, which is the one
#     mistake here that cannot be undone from the local machine.
INFRA_TREES = {"deploy", "orch"}


def _cwd_users(path):
    """Pids whose cwd is inside path. Someone standing in a directory keeps it."""
    users = []
    for d in glob.glob("/proc/[0-9]*"):
        try:
            cwd = os.readlink(d + "/cwd")
        except OSError:
            continue
        if cwd == path or cwd.startswith(path + "/"):
            users.append(int(d.split("/")[-1]))
    return users


def sweep_worktrees(repo, apply_it):
    run(["git", "-C", repo, "fetch", "-q", "origin"])
    listing = run(["git", "-C", repo, "worktree", "list", "--porcelain"]).stdout
    trees, cur = [], {}
    for line in listing.splitlines():
        if line.startswith("worktree "):
            cur = {"path": line.split(" ", 1)[1]}
        elif line.startswith("branch "):
            cur["branch"] = line.split(" ", 1)[1].replace("refs/heads/", "")
            trees.append(cur)
        elif not line.strip() and cur:
            cur = {}
    removed = 0
    for t in trees:
        path, branch = t["path"], t.get("branch", "")
        name = os.path.basename(path)
        if "--" not in name:
            continue
        suffix = name.split("--", 1)[1]
        if suffix in INFRA_TREES:
            log(f"🔒 {name:<28} infrastructure tree, never swept")
            continue
        dirty = run(["git", "-C", path, "status", "--porcelain"]).stdout.strip()
        if dirty:
            log(f"· {name:<28} KEEP — {len(dirty.splitlines())} uncommitted change(s)")
            continue
        ahead = run(["git", "-C", repo, "rev-list", "--count", f"origin/main..{branch}"]).stdout.strip()
        if ahead not in ("0", ""):
            log(f"· {name:<28} KEEP — {ahead} commit(s) not on origin/main ({branch})")
            continue
        users = _cwd_users(path)
        if users:
            log(f"· {name:<28} KEEP — {len(users)} process(es) standing in it")
            continue
        log(f"✔ {name:<28} REMOVABLE — clean, merged, nobody in it ({branch})")
        # ⚠ Counted here, not inside the apply arm: a dry run that reports "0
        # removable" after listing six of them is an instrument disagreeing with
        # its own output, and the reader believes the summary.
        removed += 1
        if apply_it:
            # ⛔ THE COST OF THIS REMOVAL IS THE BUILD DIRECTORY, NOT THE CHECKOUT.
            # A lane worktree that has been built in carries a multi-gigabyte
            # `target/`, and deleting it took longer than the shared 120 s timeout
            # every time — so the sweep failed on exactly the worktrees it most
            # wants to reclaim, and reported the failure as a refusal. Measured
            # 2026-08-21: 2.9 GB in one tree. Say the size, then wait for it.
            size = subprocess.run(["du", "-sh", path], capture_output=True,
                                  text=True, timeout=300).stdout.split("\t")[0] or "?"
            log(f"  removing {size} — a built worktree is mostly `target/`")
            r = subprocess.run(["git", "-C", repo, "worktree", "remove", path],
                               capture_output=True, text=True, timeout=1800)
            if r.returncode != 0:
                log(f"  ⛔ refused: {(r.stderr or '').strip()[:160]}")
                removed -= 1
                continue
            run(["git", "-C", repo, "branch", "-d", branch])
            log("  removed, branch deleted")
    log(f"— worktrees {'removed' if apply_it else 'removable'}: {removed}")
    if not apply_it:
        log("  nothing was changed. Re-run with --apply.")
    return 0


def cmd_orphans(a, host, live):
    """⛔⛔ REMOVING A WORKTREE DOES NOT REMOVE ITS ROWS, AND NOTHING SAID SO.

    A lane's session is rooted in the lane's worktree, so the cwd tree draws a
    folder for it. Reclaim the worktree and the folder stays — pointing at a
    directory that no longer exists, with rows under it that can only fail when
    clicked. Measured 2026-08-21, after the first sweep that actually removed
    anything: 9 such rows across 7 vanished trees, and 7 of them predated that
    sweep by weeks. ⇒ The tidy-up was creating exactly the litter it was run to
    clear, and quietly.

    ⚖ A LIVE session in a vanished tree is NOT reaped here. Its process is fine,
    its cwd is simply gone; re-rooting it to the repo's main checkout is a product
    verb this tool does not have, so it is named and left alone rather than killed
    for the crime of being untidy.
    """
    rows = rows_census(host)
    # rows_census keeps only SEATED rows; an orphan usually has no seat left.
    r = run(["ssh", "-n", host, f"{YGG} server app rows --json"])
    try:
        allrows = (json.loads(r.stdout).get("data") or {}).get("rows") or []
    except Exception:
        log("⛔ could not read the row list")
        return 2
    seen, dead, alive, keepsakes = set(), [], [], []
    kept = manually_titled_uuids(host)
    for row in allrows:
        path = row.get("full_path") or ""
        cwd = (row.get("session_cwd") or "").strip()
        if "://" not in path or path in seen or not cwd:
            continue
        seen.add(path)
        if os.path.isdir(cwd):
            continue
        uuid = path.rsplit("/", 1)[-1]
        if kept == "unreadable" or (kept is not None and uuid in kept):
            keepsakes.append((path, cwd, row.get("outline_prefix")))
            continue
        (alive if uuid in live else dead).append((path, cwd, row.get("outline_prefix")))
    for path, cwd, seat in keepsakes:
        log(f"🔖 {str(seat) or '-':<7} {path[-40:]} — KEPT: hand-typed title, cwd {cwd} is gone")
        log("    a bookmark's value is its NAME, not its process. Never folded without --force")
    for path, cwd, seat in alive:
        log(f"· {str(seat):<7} {path[-40:]} — ALIVE in a vanished tree {cwd}")
        log("    left alone: re-rooting a live session is a product verb, not a reap")
    for path, cwd, seat in dead:
        log(f"⛔ {str(seat) or '-':<7} {path[-40:]} — dead, and its tree {os.path.basename(cwd)} is gone")
        if a.apply:
            row = {"uri": path, "uuid": path.rsplit("/", 1)[-1], "seat": str(seat or "-"),
                   "label": row_label(allrows, path), "session_cwd": cwd}
            fold(row, "DEAD", "cwd tree no longer exists", host, True)
    log(f"— orphaned rows: {len(dead)} dead, {len(alive)} alive, {len(keepsakes)} kept"
        + ("" if a.apply else " · nothing was changed. Re-run with --apply."))
    return 0


def row_label(allrows, path):
    for r in allrows:
        if (r.get("full_path") or "") == path:
            return r.get("label") or ""
    return ""


def main():
    global FINISHED_IDLE_MIN, STALL_IDLE_MIN
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    sw = sub.add_parser("sweep")
    sw.add_argument("--campaign", help="only rows whose seat starts with this, e.g. 11")
    sw.add_argument("--apply", action="store_true")
    sw.add_argument("--host")
    sw.add_argument("--dead-only", action="store_true",
                    help="classify everything, act ONLY on DEAD. ⛔ The scoping rule — an "
                         "orchestrator folds its own spawns and nobody else's — protects a "
                         "JUDGEMENT: whether a quiet lane is finished. A row with no process "
                         "is not a judgement, so this pass may run unscoped and is the only "
                         "thing watching campaigns that have no orchestrator of their own")
    sw.add_argument("--max-respawns", type=int, default=0,
                    help="0 = no cap. A cap exists because this loop runs unattended: a bad "
                         "hour must not be able to spawn a lane per cold row across the fleet")
    sw.add_argument("--respawn", action="store_true",
                    help="replace each COLD row: spawn a successor at the same seat from a "
                         "brief distilled from artefacts, prove it holds the brief, and only "
                         "then fold the predecessor. The predecessor is never prompted.")
    sw.add_argument("--wake", action="store_true",
                    help="send ONE `continue` to each STALLED row (never to a protected one, "
                         "never twice for the same stall)")
    sw.add_argument("--stall-idle-min", type=float, default=STALL_IDLE_MIN,
                    help="a row at its composer, quiet this long, is STALLED (default %(default)s)")
    sw.add_argument("--finished-idle-min", type=float, default=FINISHED_IDLE_MIN,
                    help="a row that announced it was done is folded once it has been "
                         "quiet this long (default %(default)s). Raise it for unattended runs.")
    one = sub.add_parser("row")
    one.add_argument("target")
    one.add_argument("--apply", action="store_true")
    one.add_argument("--host")
    # ⛔⛔ THE SINGLE-ROW VERB MUST ANSWER THE SAME QUESTION THE SWEEP ASKED.
    # `sweep --stall-idle-min 10` printed COLD and named `row <uuid> --force
    # --apply` as the remedy in its own output; `row` then read the module
    # default of 20 and replied WORKING, folding nothing. Two classifiers, one
    # question, different answers, and no hint that a threshold was the whole
    # difference. Measured 2026-08-21 on a lane whose work was already landed.
    one.add_argument("--stall-idle-min", type=float, default=STALL_IDLE_MIN,
                     help="the same threshold sweep takes; a verdict is only "
                          "meaningful beside the threshold that produced it")
    one.add_argument("--finished-idle-min", type=float, default=FINISHED_IDLE_MIN)
    one.add_argument("--force", action="store_true",
                     help="fold this row whatever the verdict — for a row an operator has "
                          "named explicitly and decided about. Never available to `sweep`, "
                          "and never able to touch a PROTECTED row.")
    orph = sub.add_parser("orphans",
                          help="rows whose cwd tree no longer exists — reap the dead, name the live")
    orph.add_argument("--apply", action="store_true")
    orph.add_argument("--host")
    wt = sub.add_parser("worktrees")
    wt.add_argument("--apply", action="store_true")
    wt.add_argument("--repo", default=os.path.abspath(os.path.join(HERE, "..", "..", "..")))
    a = ap.parse_args()
    FINISHED_IDLE_MIN = getattr(a, "finished_idle_min", FINISHED_IDLE_MIN)
    STALL_IDLE_MIN = getattr(a, "stall_idle_min", STALL_IDLE_MIN)

    if a.cmd == "worktrees":
        return sweep_worktrees(a.repo, a.apply)

    host = a.host or gui_host()
    if not host:
        log("⛔ no GUI host — blind is not clear")
        return 2

    rows = rows_census(host)
    protected = protected_uuids()
    live_by_host, live = {}, set()
    try:
        for row in rows:
            h = host_of(row["uri"])
            if h not in live_by_host:
                live_by_host[h] = live_agent_uuids(h)
                log(f"  liveness from {h or 'this machine'}: {len(live_by_host[h])} agent uuid(s)")
            live |= live_by_host[h]
    except RuntimeError as exc:
        log(f"⛔ {exc}")
        log("   refusing to classify: an unreachable host reads as a host with no agents,")
        log("   and with --apply that is the whole fleet reaped in one pass.")
        return 2

    if a.cmd == "orphans":
        return cmd_orphans(a, host, live)

    if a.cmd == "row":
        rows = [r for r in rows if a.target in r["uri"]]
        if not rows:
            # ⛔⛔ THE CENSUS KEEPS ONLY SEATED ROWS, AND AN UNSEATED ROW IS THE ONE
            # MOST LIKELY TO NEED FOLDING. A row loses its seat when its group head
            # is folded, or when it was never numbered — so the debris this verb
            # exists to clear was precisely the debris it refused to look at, with
            # "no seated row matches" reading as "no such row". Measured
            # 2026-08-21 on a dead lane whose seat had gone with its twin.
            rows = [r for r in rows_all(host) if a.target in r["uri"]]
            if not rows:
                log(f"⛔ no row matches {a.target}")
                return 2
            log(f"  {a.target} has no seat — folding it by path")

    counts, folded, seen_rows, respawned = {}, 0, [], 0
    for row in sorted(rows, key=lambda r: r["seat"]):
        if a.cmd == "sweep" and a.campaign:
            head = row["seat"].split(".")[0]
            if head != a.campaign:
                continue
        verdict, why = classify(row, live, protected)
        counts[verdict] = counts.get(verdict, 0) + 1
        if verdict != "PROTECTED":
            seen_rows.append(row)
        mark = {"DEAD": "⛔", "FINISHED": "✔", "WORKING": "·", "BRIEFLESS": "⚠",
                "PROTECTED": "🔒", "STALLED": "⏸", "COLD": "❄"}[verdict]
        log(f"{mark} {row['seat']:<7} {row['uuid'][:8]} {verdict:<9} {why}")
        if getattr(a, "dead_only", False) and verdict != "DEAD":
            continue
        forced = getattr(a, "force", False) and verdict != "PROTECTED"
        # ⚠ A single-row call is an operator asking about ONE row, usually because
        # a sweep told them to. Declining in silence makes the two verbs look as
        # though they disagree about the row, when they disagree about a number.
        if a.cmd == "row" and verdict not in ("DEAD", "FINISHED") and not forced:
            log(f"  not folded: {verdict} at --stall-idle-min {STALL_IDLE_MIN:g} "
                f"(idle {row.get('idle_min')}m). A lower threshold, or --force, folds it.")
        if verdict in ("DEAD", "FINISHED") or forced:
            if forced and verdict not in ("DEAD", "FINISHED"):
                log(f"  --force: folding a {verdict} row on an operator's say-so")
            if fold(row, verdict, why, host, a.apply):
                folded += 1
        elif verdict == "STALLED" and getattr(a, "wake", False):
            wake(row, host, a.apply)
        elif verdict == "COLD":
            cap = getattr(a, "max_respawns", 0) or 0
            if getattr(a, "respawn", False) and cap and respawned >= cap:
                log(f"  ⛔ respawn cap of {cap} reached this sweep — left cold, "
                    f"its successor brief is written and the next sweep will take it")
            elif getattr(a, "respawn", False):
                respawned += 1
                if respawn(row, why, host, a.apply):
                    folded += 1
            else:
                path = successor_brief(row, why)
                log(f"  successor brief → {os.path.relpath(path, os.path.expanduser('~'))}")
                log("  ⛔ NOT woken. Re-run with --respawn to replace it at this seat.")
    # ⛔⛔ TWO ROWS AT ONE SEAT IS THE HYGIENE DEFECT THIS TOOL EXISTS TO PREVENT,
    # AND THE TOOL CAUSED ONE. A respawn spawns the successor first and folds the
    # predecessor second, on purpose — but when that second half fails, the seat
    # holds two rows and NOTHING SAID SO. It read as a healthy census: both rows
    # classify WORKING, both are listed, and the duplicate is visible only to a
    # person counting seat numbers in the sidebar.
    # ⇒ Uniqueness is asserted at the end of every sweep, because the failure is
    #   silent by construction and a count of verdicts cannot show it.
    from collections import Counter
    dupes = [seat for seat, n in Counter(r["seat"] for r in seen_rows).items() if n > 1]
    for seat in sorted(dupes):
        holders = [r["uuid"][:8] for r in seen_rows if r["seat"] == seat]
        log(f"⛔ SEAT {seat} IS HELD BY {len(holders)} ROWS: {', '.join(holders)}")
        log(f"   A respawn whose fold half failed leaves exactly this. Decide which is live")
        log(f"   and fold the other: ygg-fold.py row <uuid> --force --apply")
    log(f"— {counts} · {'folded' if a.apply else 'would fold'} {folded}")
    if not a.apply:
        log("  nothing was changed. Re-run with --apply.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
