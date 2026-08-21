#!/usr/bin/env python3
"""ygg-monitor — the supervision plane for relays and orchestrators.

⚖ WHY THIS EXISTS AND THE BOOTER IS NOT ENOUGH
==============================================
The booter is a dumb timer, and that is its virtue: it is the safety net that
still works when everything cleverer has failed. **It stays.** Every relay should
subscribe to it, and an ORCHESTRATOR MUST — an orchestrator that dies takes the
supervision of every row under it, so the one session that must not silently stop
is the one watching the others.

But a timer can only ask "has this row been quiet too long". It cannot ask WHY,
and the why decides the action. This plane adds the judgement:

  · a row mid-turn and THINKING must be left alone
  · a row mid-turn and ABANDONED must be woken — and it looks identical
  · a row out of context cannot be woken at all and must be RELAYED
  · a row the owner has taken back must not be touched by anything

⛔ THE DEFECT THAT PRODUCED THIS FILE (measured 2026-08-13)
   Two cluster rows were re-resumed on fresh PTYs by a GUI restart. Their turns
   were abandoned mid-flight; their processes stayed alive and idle. The watchdog
   classified both STUCK and then REFUSED TO ACT — "a continue would race its own
   input" — and escalated into a log file nobody was reading. They sat 22 minutes
   until a human noticed.

   Two things were wrong and only one was obvious:
   1. The escalation had nowhere to go. With an orchestrator present it must go
      to the ORCHESTRATOR'S ROW, which can probe, read and decide.
   2. ⭐ MID-TURN IS NOT ONE STATE. A thinking agent BURNS CPU; an abandoned one
      does not. That is the discriminator the old classifier lacked, so it lumped
      both into "do not touch" — and the abandoned case is precisely the one that
      needs touching. Measured: both rows at ~0% CPU, alive, 22 min silent; a PTY
      write woke both immediately.

⛔ AND THE NUDGE MUST GO TO THE PTY, NOT THE COMPOSER. `terminal submit` drives
   the GUI's mounted terminal host and answers submitted:false for a row with
   nothing mounted — which is most rows a watcher looks at. Both rows above
   refused `submit` for 30 s each and took a PTY write instantly.

⭐ PROMOTION AND DEMOTION BELONG TO THE OWNER
   Any row may be pinned out of automation entirely (`demote`), which is what a
   design fork wants: the owner takes the row, weighs the trade-off by hand, and
   nothing nudges, boots or reaps it meanwhile. `promote` hands it back. A pinned
   row is skipped by every verb here, and saying so out loud in `list` is part of
   the contract — an owner must be able to see at a glance what is under
   automation and what is his.

⭐ ATTACHING IS GENERAL-PURPOSE
   Any session, spawned for any reason, can `attach` itself to a running
   orchestrator and declare its intent. From then on it is supervised like a
   cluster row. That is what makes it safe to start something and walk away.
"""
import argparse
import collections
import importlib.util
import json
import os
import re
import socket
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ygg_host import resolve_gui_host  # noqa: E402
from ygg_rowarg import add_row_argument, bare_uuid, resolve_row  # noqa: E402

HERE = Path(__file__).resolve().parent
STATE = Path.home() / ".yggterm" / "relay"
SUBS = STATE / "monitor"
LOGPATH = STATE / "monitor.log"

# Mid-turn and silent for longer than this, at rest, is ABANDONED not thinking.
ABANDONED_SECS = 600
# CPU% at or below this over the sample window counts as "not thinking".
IDLE_CPU_PCT = 2.0
CPU_SAMPLE_SECS = 3
# ⭐ A finished relay row idles by design. Escalating it at 4 minutes produced
# three false alarms in one minute; this is the window before an idle row is
# worth a human's or an orchestrator's attention at all.
IDLE_ESCALATE_SECS = 900
EPISODES = STATE / "monitor-episodes"


def log(m):
    line = f"{time.strftime('%H:%M:%S')} ygg-monitor {m}"
    print(line, flush=True)
    try:
        STATE.mkdir(parents=True, exist_ok=True)
        with LOGPATH.open("a") as fh:
            fh.write(line + "\n")
    except Exception:
        pass


def _babysit():
    """Reuse the classifier rather than forking it — two watchdogs that disagree
    about what STUCK means is worse than one that is sometimes wrong."""
    spec = importlib.util.spec_from_file_location("ygg_babysit", HERE / "ygg-babysit.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _booter():
    """The booter's ledger readers, for the same reason `_babysit` exists.

    ⛔ This file used to parse `never-arm.tsv` itself, a few lines below, and the
    two parsers disagreed about what an unreadable list meant — which is how one
    watchdog can refuse to name a row it cannot screen while the other types into
    it. One file, one reader."""
    spec = importlib.util.spec_from_file_location("ygg_booter", HERE / "ygg-booter.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def screen_ledgers():
    """(attended, opted-out) as 8-char prefixes. ⛔ `None` means COULD NOT READ.

    Both halves are the booter's own readers, so an unreadable or torn list
    arrives here as the refusal it is rather than as an empty set. Callers must
    branch on `None` before they act — and in this file "act" means TYPE INTO A
    ROW, which is why `wake()` is gated on it."""
    b = _booter()
    blocked, optedout = b.never_arm(), b.disarmed_rows()
    return (None if blocked is None else {u[:8] for u in blocked},
            None if optedout is None else {u[:8] for u in optedout})


#: Returned instead of a reply when the CALL ITSELF failed. ⛔ It is not an
#: empty answer, and no caller may read it as one — see ygg_host.py for the
#: afternoon this distinction cost.
UNREACHABLE = {"__unreachable__": True}


def ygg(host, *args):
    if not host:
        return dict(UNREACHABLE)
    cmd = ["ssh", "-n", host, "~/.local/bin/yggterm-headless " + " ".join(
        f"'{a}'" if " " in str(a) else str(a) for a in args)]
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=90)
        out = r.stdout
        if "{" not in out:
            return dict(UNREACHABLE)
        # ⛔ NOT `json.loads(out[out.find("{"):])`. At least one verb replies with
        # TWO concatenated JSON objects, and that idiom reads the first and
        # discards the rest WITHOUT RAISING — a truncated answer the watchdog
        # then acts on confidently. `raw_decode` stops at the first document and
        # tells us there was more.
        obj, end = json.JSONDecoder().raw_decode(out[out.find("{"):])
        rest = out[out.find("{") + end:].strip()
        if rest.startswith("{"):
            obj.setdefault("__trailing_documents__", True)
        return obj
    except Exception:
        return dict(UNREACHABLE)



def _same_uuid(a, b):
    """⛔ AN IDENTIFIER STORED AT TWO LENGTHS CANNOT BE COMPARED BY EQUALITY.

    Subscriptions may hold a short uuid (whatever a brief happened to quote);
    the row plane always answers with the full one. Equality between those is a
    false negative that reads as a dead row. Prefix-match, with a floor long
    enough that a collision is not a practical concern."""
    a, b = (a or "").lower(), (b or "").lower()
    if not a or not b:
        return False
    n = min(len(a), len(b))
    return n >= 8 and a[:n] == b[:n]

def live_rows(host):
    """(rows, ok). ⛔ `ok=False` means WE ARE BLIND, not that there are no rows.

    Every conclusion of the form *"this row no longer exists"* must be gated on
    `ok`, because the failure mode this guards is a supervisor rendering its own
    blindness as a terminal verdict about somebody else's live session."""
    reply = ygg(host, "server", "app", "rows")
    if reply.get("__unreachable__"):
        return [], False
    data = reply.get("data")
    if not isinstance(data, dict) or "rows" not in data:
        return [], False
    return (data.get("rows") or []), True


def _ep_load(uuid):
    """Per-row escalation latch, so one episode produces one escalation."""
    f = EPISODES / f"{uuid}.json"
    if f.exists():
        try:
            return json.loads(f.read_text())
        except Exception:
            pass
    return {"escalated": None}


def _ep_save(uuid, st):
    EPISODES.mkdir(parents=True, exist_ok=True)
    (EPISODES / f"{uuid}.json").write_text(json.dumps(st))


def _this_host():
    return socket.gethostname().split(".")[0]


def _agent_uuids_on_this_host():
    """Every agent-CLI session uuid with a LIVE process on THIS machine.

    ⛔ ABSENCE PROVES DEATH ONLY FOR A ROW WHOSE HOST IS THIS HOST. A remote
    row's process is invisible from here, and calling that absence death is the
    instrument error this fleet keeps paying for — the caller checks the host
    before believing a miss.

    ⚠ The uuid is read OUT of each command line, never put INTO the search
    pattern: a `pgrep -f <uuid>` matches the shell doing the searching, which
    has cost three sessions here."""
    live = set()
    r = _run(None, ["pgrep", "-af", "claude|codex|gemini|amp|opencode"])
    if not r:
        return live
    for line in r.stdout.splitlines():
        m = re.search(r"--resume\s+([0-9a-f-]{36})|--session-id\s+([0-9a-f-]{36})", line)
        if m:
            live.add(m.group(1) or m.group(2))
    return live


def sub_path(uuid):
    SUBS.mkdir(parents=True, exist_ok=True)
    return SUBS / f"{uuid}.json"


def load_subs():
    out = []
    if not SUBS.exists():
        return out
    for p in sorted(SUBS.glob("*.json")):
        try:
            out.append(json.loads(p.read_text()))
        except Exception:
            log(f"⚠ unreadable subscription {p.name} — left in place, not guessed")
    return out


# ---------------------------------------------------------------------------
# The discriminator the old watchdog lacked.
# ---------------------------------------------------------------------------
def _run(host, argv, timeout=25):
    """Run locally, or over ssh when the row lives on another machine.

    ⛔ A LOCAL PROBE CANNOT ANSWER FOR A REMOTE ROW, and its silence looks
    identical to a real negative. Getting this wrong makes every remote row read
    as "no process", which would refine straight to ABANDONED and nudge rows that
    are working perfectly. Caught in this file's own first tick."""
    cmd = argv if not host else ["ssh", host, " ".join(f"'{a}'" for a in argv)]
    try:
        return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except Exception:
        return None


def cli_process(uuid, host=None):
    """The agent CLI process for this session, on the host that owns it.

    ⛔ Identify, never count. `pgrep -c` counts the shell asking the question."""
    r = _run(host, ["pgrep", "-af", uuid])
    if r is None:
        return None
    for line in r.stdout.splitlines():
        pid, _, args = line.partition(" ")
        if "pgrep" in args or "bash -c" in args:
            continue
        if re.search(r"\b(claude|codex|gemini|amp|opencode)\b", args):
            try:
                return {"pid": int(pid), "args": args,
                        "resumed": "--resume" in args or "resume" in args.split()}
            except ValueError:
                continue
    return None


def cpu_pct(pid, host=None):
    """Sampled CPU over a real window, on the host that owns the process.

    ⛔ `ps %CPU` is a LIFETIME AVERAGE, not current load — a process that burned a
    core for an hour and has since gone idle still reads busy. Sample the jiffy
    counters across a window instead; that is the only reading that answers
    "is it working RIGHT NOW". Take BOTH samples in one remote call, or the ssh
    round-trip lands inside the window and the rate is wrong."""
    script = (f"a=$(awk '{{print $14+$15}}' /proc/{pid}/stat 2>/dev/null); "
              f"sleep {CPU_SAMPLE_SECS}; "
              f"b=$(awk '{{print $14+$15}}' /proc/{pid}/stat 2>/dev/null); "
              f"echo \"$a $b\"")
    cmd = ["ssh", host, script] if host else ["bash", "-c", script]
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=CPU_SAMPLE_SECS + 25)
        a, b = r.stdout.split()
        hz = os.sysconf("SC_CLK_TCK") or 100
        return 100.0 * (int(b) - int(a)) / hz / CPU_SAMPLE_SECS
    except Exception:
        return None


def _busy_descendant(pid, host=None, younger_than=None):
    """The command line of a live descendant that STARTED INSIDE THIS TURN, or None.

    ⛔⛔ CPU IS NOT THE TEST FOR "IS THIS TURN STILL ALIVE", AND THE OWNER PAID
    FOR THAT 2026-08-21. An agent CLI mid-tool-call — waiting on ssh, a build, a
    remote probe, a sleep — burns essentially NO cpu in the parent while being
    entirely mid-turn. `cpu_pct` reads that as "at rest", `ABANDONED_SECS`
    elapses, and the supervisor types `continue` into a session that was in the
    middle of a sentence. That is precisely what happened to an orchestrator
    driving a fleet deploy: every one of its long steps is a subprocess it is
    blocked on, so the busier it was, the more abandoned it looked.

    ⇒ ASK WHAT THE TURN IS WAITING ON, NOT WHAT THE PARENT IS BURNING. A live
      descendant that is not just the CLI's idle shell is positive evidence of
      an unfinished tool call, and positive evidence beats an absence.

    ⛔⛔ AGE IS THE WHOLE DISCRIMINATOR, AND THE FIRST CUT OF THIS FUNCTION GOT
    IT WRONG — caught on its own author's row before it shipped. "Has a live
    child" is TRUE for every CLI that runs an MCP server: those are launched
    with the session and outlive every turn, so the test returned WORKING
    unconditionally and would have disabled waking altogether. A supervisor that
    can never act is worse than the false wake it was fixing, and it would have
    failed SILENTLY, looking like a quiet fleet.

    ⇒ A sidecar is as old as the session; a tool call is YOUNGER THAN THE STALL.
      `younger_than` is the row's idle age: a descendant that started after the
      row went quiet is the thing the row is waiting on. One older than the
      stall was already there when it stalled and proves nothing.

    ⚠ One-sided on purpose: finding such a descendant proves WORKING; finding
    none proves nothing on its own, so the caller falls through to the cpu
    reading rather than upgrading "no child" into a verdict."""
    # etimes = seconds since that process started, from the host that owns it.
    script = (f"for c in $(pgrep -P {pid} 2>/dev/null); do "
              f"  e=$(ps -o etimes= -p $c 2>/dev/null | tr -d ' '); "
              f"  printf '%s ' \"${{e:-999999}}\"; "
              f"  tr '\\0' ' ' < /proc/$c/cmdline 2>/dev/null; echo; done")
    r = _run(host, ["bash", "-c", script])
    if r is None:
        return None
    # A stall of N seconds is explained only by work started within it. The 30 s
    # floor covers a row that has only just gone quiet.
    horizon = max(int(younger_than or 0), 30)
    for line in r.stdout.splitlines():
        age_s, _, cmd = line.strip().partition(" ")
        cmd = cmd.strip()
        if not cmd:
            continue
        try:
            age = int(age_s)
        except ValueError:
            continue
        if age > horizon:
            continue                      # a sidecar, not this turn's work
        # A bare login shell parked at a prompt is what an IDLE agent keeps
        # around; anything else is work in flight.
        head = cmd.split()[0].rsplit("/", 1)[-1]
        if head in ("bash", "sh", "zsh", "dash") and len(cmd.split()) <= 1:
            continue
        return f"{cmd[:110]} ({age}s old)"
    return None


def refine(state, uuid, host=None):
    """Split the old catch-all STUCK into what it was always hiding.

    Returns (state, why). ABANDONED is the new one and the whole point: a row
    whose turn was cut off mid-flight, alive and at rest, waiting forever for a
    turn nobody will finish."""
    if state["state"] != "STUCK":
        return state["state"], ""
    proc = cli_process(uuid, host)
    if proc is None:
        return "STUCK", "mid-turn, no CLI process on this host — cannot judge from here"
    # ⛔ BEFORE the cpu sample, because a blocked turn is the case cpu gets wrong.
    child = _busy_descendant(proc["pid"], host, state.get("age"))
    if child:
        return "WORKING", f"mid-turn, blocked on a live tool call ({child[:60]}) — leave it alone"
    pct = cpu_pct(proc["pid"], host)
    if pct is None:
        return "STUCK", f"mid-turn, pid {proc['pid']} vanished while sampling"
    if pct > IDLE_CPU_PCT:
        return "WORKING", f"mid-turn and BUSY ({pct:.1f}% cpu) — thinking, leave it alone"
    if state["age"] < ABANDONED_SECS:
        return "STUCK", f"mid-turn, at rest ({pct:.1f}%) but only {state['age']//60}m — too early to call"
    return "ABANDONED", (f"mid-turn, at rest ({pct:.1f}% cpu) for {state['age']//60}m"
                         + (", process was re-resumed" if proc["resumed"] else "")
                         + " — its turn was cut off and nothing will finish it")


# ---------------------------------------------------------------------------
# Actions
# ---------------------------------------------------------------------------
def wake(host, row, why, dry):
    """Wake a row through the BOOTER'S GUARDED WRITER — never a raw send.

    ⛔⛔ THIS FUNCTION USED TO TYPE WITH NO GUARDS AT ALL, AND IT IS THE ONE
    AIMED AT ORCHESTRATORS. It sent `terminal send <msg>` followed by a lone
    `\\r`, with no screen read, no choice-prompt refusal, no `--refuse-if-draft`
    and no verify-before-Enter — every one of which the booter had, for the same
    act, on the same rows. Two watchdogs, two write paths, and the careless one
    pointed at the sessions that coordinate everything else.

    ⇒ Owner-reported 2026-08-21: a wake from here landed IN THE MIDDLE OF his
      orchestrator's turn. Two things were wrong at once and only one is fixed
      here — the write is now guarded (this change), and the CLASSIFIER must
      stop calling a row "at rest" while it waits on a slow subprocess (its own
      entry). A guarded write cannot rescue a wrong verdict; it can only stop
      that verdict from landing in somebody's half-typed sentence.

    ⚠ The guarded path REFUSES rather than types when it cannot prove the
    composer is clear, and returns a reason string. A refusal is a real outcome,
    not a failure to retry: `False` here means the row was deliberately left
    alone, and the caller must not fall back to an unguarded send."""
    msg = ("ORCHESTRATOR/MONITOR — continue. Your turn was cut off mid-flight "
           "(likely a restart re-resuming your session on a fresh PTY); your process "
           "and your work are intact. Check git status/log on your tree to see what "
           "landed, then carry on from where your last message stopped.")
    if dry:
        log(f"  DRY would wake {row}: {why}")
        return True
    outcome = _booter()._pty_type_and_enter(host, row, msg)
    if outcome in ("pty-write",):
        return True
    log(f"  ⛔ wake REFUSED for {row.rsplit('/', 1)[-1][:8]}: {outcome or 'write not accepted'} "
        f"— typing nothing. The row is left exactly as it was.")
    return False


def escalate(host, sub, row, why, dry):
    """Route UP, never into a log nobody reads.

    ⭐ With an orchestrator present the escalation goes to ITS ROW: it can probe,
    read the tail and decide, which a timer cannot. Only when there is no
    orchestrator does a human get a card — and the card points at the ROW THAT IS
    STUCK, never at whoever noticed."""
    to = sub.get("escalate_to") or ""
    if dry:
        log(f"  DRY would escalate {row} -> {to or 'human'}: {why}")
        return
    # ⛔ AN ESCALATION TARGET THAT NO LONGER EXISTS SWALLOWS THE ESCALATION AND
    # REPORTS SUCCESS. `terminal send` answers about the REQUEST, not the
    # delivery, so a retired orchestrator absorbs every cluster's cry for help
    # and this function used to log "escalated to orchestrator" over the top of
    # it. `succeed` keeps the pointers fresh at a handover; this is the backstop
    # for every other way a target goes stale, and it fails to the human rather
    # than into a void — the whole point of the plane.
    orphaned = ""
    if to:
        live = {(r.get("full_path") or "").rsplit("/", 1)[-1]
                for r in (ygg(host, "server", "app", "rows").get("data") or {}).get("rows", [])}
        # ⚠ An EMPTY row list is an instrument failure, not a dead target. Falling
        # back on it would route every escalation to the human the moment ssh
        # blips — so require positive evidence that the row plane answered.
        #
        # ⛔⛔ AND MATCH BY PREFIX, NEVER BY SET MEMBERSHIP. `escalate_to` is stored
        # verbatim, so it may hold 8 chars while the row plane always answers with
        # 36 — and `to not in live` is then TRUE for a perfectly live orchestrator.
        # Every escalation from such a row fell back to a human card while logging
        # that a row sitting right there "is NOT a live row". The identical defect
        # was diagnosed and commented in `audit` on 2026-08-13 and `_same_uuid` was
        # written for it; the fix went into the function that REPORTS and not into
        # this one, which ROUTES. Measured 2026-08-14: seat 6.7 carried a short
        # pointer from 07:00 and could not have reached its orchestrator all day.
        if live and not any(_same_uuid(to, r) for r in live):
            log(f"  ⚠ escalation target {to[:8]} is NOT a live row — falling back to a human card")
            orphaned, to = to, ""
        else:
            # Address the row plane in the length it speaks, never the stored stub.
            to = next((r for r in live if _same_uuid(to, r)), to)
    if to:
        target = f"remote-cc://{_host_of(sub, 'escalate_host', 'host')}/{to}"
        note = (f"MONITOR — row {sub.get('seat') or row} needs a decision: {why}. "
                f"Its path is {row}. Probe it, read its tail, and act; do not "
                f"assume it is finished.")
        ygg(host, "server", "app", "terminal", "send", target, "--data", note)
        time.sleep(0.2)
        subprocess.run(["ssh", host,
                        f"~/.local/bin/yggterm-headless server app terminal send '{target}' --data $'\\r'"],
                       capture_output=True, text=True, timeout=60)
        log(f"  escalated to orchestrator {to[:8]}")
    else:
        extra = (f". Its orchestrator {orphaned[:8]} is GONE — this row is unsupervised "
                 f"until someone claims the seat.") if orphaned else ""
        ygg(host, "server", "app", "notify", "relay needs a human", why + extra,
            "--tone", "warning", "--session", row)
        log("  escalated to a human card "
            + (f"(orchestrator {orphaned[:8]} is gone)" if orphaned else "(no orchestrator subscribed)"))


# ---------------------------------------------------------------------------
# Verbs
# ---------------------------------------------------------------------------
def _host_of(sub, *keys):
    """⛔ `.get(k, default)` DEFAULTS ON A MISSING KEY, NOT ON AN EMPTY VALUE.

    A subscription written without `--machine` stores `host: ""` — the key is
    PRESENT and empty. So `sub.get("host", "dev")` returns `""`, and the row path
    composes as `remote-cc:///<uuid>`: an empty authority, unroutable, and it
    reaches nobody while every field involved looks populated.

    Measured 2026-08-13: an escalation for a live row was addressed that way, and
    the malformed path was visible in the escalation text itself before anyone
    noticed the subscription had an empty host. ⇒ Fall back on FALSINESS, not on
    absence, wherever a stored value composes into an address."""
    for k in keys:
        v = (sub.get(k) or "").strip()
        if v and v != "local":
            return v
    return "dev"


def _bare_uuid(v):
    """⛔ $YGGTERM_SESSION_ID IS NOT A BARE UUID — it is `cc-runtime://<uuid>`.

    Used as a filename the `//` becomes a path separator, so subscribe died with
    FileNotFoundError on `.../monitor/cc-runtime:/<uuid>.json`. ygg-claim.sh has
    always stripped it (`${SESSION##*/}`); this did not, so every session that
    subscribed the documented way crashed and only `--uuid` worked. Reported by a
    cluster 2026-08-13. Take the last path segment, exactly as the claim does.

    ⭐ The rule now lives in `ygg_rowarg`, which both watchdogs share, so the
    booter and the monitor cannot drift about what a row name means."""
    return bare_uuid(v)


def cmd_subscribe(a):
    uuid = _bare_uuid(a.uuid or os.environ.get("YGGTERM_SESSION_ID", ""))
    if not uuid:
        log("subscribe: need --uuid (or $YGGTERM_SESSION_ID)")
        return 64
    # ⛔⛔ A ROW THAT RELEASED ITSELF IS NOT A ROW NOBODY SUBSCRIBED. Restoring
    #    one resurrects a campaign that declared itself finished, and the board
    #    then demands a decision about it forever — with both offered answers
    #    wrong, because it is neither a delegate to arm nor a place a human
    #    types. The booter has refused this since `optout` shipped; the monitor
    #    did not, so a row could protect itself on one plane and be restored on
    #    the other. Measured 2026-08-14 on a closed campaign.
    released = released_rows()
    if released is None:
        log("⛔ the release ledger is UNREADABLE — refusing to subscribe rather than")
        log("   risk resurrecting a row that stood down. Fix or move "
            f"{RELEASED_LEDGER}, then retry.")
        return 65
    if uuid in released and not getattr(a, "rearm", ""):
        log(f"⛔ {uuid[:8]} RELEASED its own monitor subscription — it is not "
            f"unsubscribed by accident.")
        log(f"   reason recorded: {released[uuid]}")
        log("   Restoring it resurrects a row that declared itself finished, and it")
        log("   will then be reported as an unarmed gap on every listing.")
        log("   If you genuinely mean to watch it again: --rearm '<why>'.")
        return 66
    if getattr(a, "rearm", "") and uuid in released:
        _record_release(uuid, RESUB_MARK + a.rearm)
        log(f"⚠ re-subscribing {uuid[:8]} over its own release: {a.rearm}")
    # ⛔⛔ AN IDENTIFIER STORED AT TWO LENGTHS BECOMES TWO SUBSCRIBERS. Subscriptions
    # are keyed by FILE NAME, so subscribing `bb5b4358` when `bb5b4358-b83a-…` is
    # already subscribed does not update it — it creates a SECOND watcher for one
    # row, which then double-escalates. Caught 2026-08-14 by the orchestrator doing
    # it to three rows at once, an hour after re-reading its own note about this
    # exact trap: the guard belongs in the tool, because discipline resets every
    # session and a check does not.
    # ⇒ Resolve a short uuid against what is already subscribed rather than
    #   refusing it, so the convenient form keeps working and stops forking state.
    if len(uuid) < 36:
        matches = [p.stem for p in SUBS.glob("*.json") if p.stem.startswith(uuid)]
        if len(matches) == 1:
            log(f"⚠ '{uuid}' is a PREFIX — resolved to the subscribed row {matches[0]}")
            uuid = matches[0]
        elif len(matches) > 1:
            log(f"⛔ '{uuid}' is ambiguous across {len(matches)} subscriptions — pass the full uuid:")
            for m in matches:
                log(f"     {m}")
            return 64
        else:
            log(f"⛔ REFUSING a short uuid '{uuid}' that matches no existing subscription.")
            log("   Subscriptions are keyed by uuid, so a truncated one silently creates a")
            log("   SECOND subscriber for the same row and both escalate. Pass the full uuid.")
            return 64
    # ⛔⛔ THE SAME FIELD-CLASS, THE OTHER FIELD. The block above hardened `--uuid`
    # against the two-lengths trap and left `--escalate-to` — the other uuid in the
    # same record — taking whatever a brief happened to quote. A short one stored
    # here is invisible on the board (it renders `[:8]`, so both forms look
    # identical) and breaks BOTH consumers: `escalate` fell back to a human card
    # claiming the live orchestrator was dead, and `succeed` skipped the row at
    # handover. Normalise at the source; the consumers now tolerate it too, but a
    # value that is correct when written cannot rot in a frozen brief.
    escalate_to = _bare_uuid(a.escalate_to or "")
    if escalate_to and len(escalate_to) < 36:
        hits = [p.stem for p in SUBS.glob("*.json") if p.stem.startswith(escalate_to)]
        if len(hits) == 1:
            log(f"⚠ --escalate-to '{escalate_to}' is a PREFIX — resolved to {hits[0]}")
            escalate_to = hits[0]
        else:
            # Not refused: the target may legitimately not be subscribed yet. But
            # say it out loud, because the board cannot show this and will not.
            log(f"⛔ --escalate-to '{escalate_to}' is SHORT and matches "
                f"{len(hits)} subscriptions — storing it verbatim, but pass the FULL "
                f"uuid: a stub cannot be told from a good pointer on the board.")
    # ⛔⛔ THE SAME ROT, THE OTHER CLASS: a WELL-FORMED pointer at a seat that has
    # since been relieved. The block above catches a uuid that is the wrong
    # SHAPE; this catches one that is the right shape and the wrong ERA. A brief
    # is frozen at the moment it is written, orchestrators turn over several
    # times a day, and every successor inherits the predecessor's uuid in prose.
    # `succeed` re-points rows that were subscribed AT the handover — it cannot
    # reach a row that subscribes afterwards quoting the old brief, which is
    # exactly when this fires.
    # ⇒ Measured 2026-08-14: THREE briefs in one afternoon named a retired
    #   orchestrator, and one of them armed a fresh lane whose every escalation
    #   would have rung into an empty room. Nothing on the board shows it: a
    #   dead pointer and a live one render identically.
    # ⚖ Resolve only when it is UNAMBIGUOUS — the same rule the prefix branch
    #   above follows. One live orchestrator for the campaign is a fact; two is
    #   a question, and guessing between them would deliver alarms to the wrong
    #   seat, which is worse than saying so.
    if escalate_to and len(escalate_to) == 36 and not sub_path(escalate_to).exists():
        live = []
        for p in SUBS.glob("*.json"):
            try:
                s = json.loads(p.read_text())
            except Exception:
                continue
            if s.get("role") == "orchestrator" and (
                    not a.campaign or s.get("campaign") == a.campaign):
                live.append((p.stem, s.get("seat") or "-"))
        if len(live) == 1:
            log(f"⚠ --escalate-to {escalate_to[:8]} names a seat that is NO LONGER "
                f"SUBSCRIBED — it has been relieved since your brief was written. "
                f"Resolved to the one live orchestrator for this campaign: "
                f"{live[0][0]} (seat {live[0][1]}).")
            escalate_to = live[0][0]
        else:
            log(f"⛔⛔ --escalate-to {escalate_to[:8]} names a seat that is NO LONGER "
                f"SUBSCRIBED, and {len(live)} live orchestrators could be meant, so "
                f"this is NOT being guessed. Storing it verbatim — but every "
                f"escalation from this row will ring into an empty room until it is "
                f"corrected. Live orchestrators: "
                f"{', '.join(f'{u[:8]}(seat {st})' for u, st in live) or 'none'}. "
                f"Fix with: ygg-monitor.py succeed --from {escalate_to} --to <uuid>")
    rec = {"uuid": uuid, "host": a.machine, "role": a.role,
           "escalate_to": escalate_to or a.escalate_to, "escalate_host": a.escalate_host,
           "campaign": a.campaign, "seat": a.seat,
           "owner_pinned": False, "booter": True,
           "intent": a.intent, "since": int(time.time())}
    sub_path(uuid).write_text(json.dumps(rec, indent=1))
    # ⛔ REPORT THE EFFECT, NOT THE REQUEST. This read `a.escalate_to` — the value
    # the CALLER passed — so a pointer that had just been resolved (from a prefix,
    # or from a retired seat) was still announced as the old one. The record on
    # disk and the line on screen disagreed, and the line is the one a human
    # reads back to confirm the subscription. Same field-class as the row verbs.
    stored = rec["escalate_to"]
    log(f"subscribed {uuid[:8]} as {a.role}"
        + (f", escalating to {stored[:8]}" if stored else ", escalating to a human"))
    if a.role == "orchestrator" and not a.no_booter_reminder:
        log("⛔ AN ORCHESTRATOR MUST ALSO SUBSCRIBE TO THE BOOTER — it is the net that")
        log("   catches this plane itself. Run: ygg-booter.py subscribe")
    return 0


SEAT_MEMORY = STATE / "seat-membership.json"


def _deferred_until(uuid):
    """Epoch a row is deferred until, or 0. ⛔ THE DEFERRAL LIVES IN THE BOOTER'S
    STORE, NOT THIS ONE — the two planes keep separate files, which is exactly why
    the idle verdict could not see it and pronounced a correctly-waiting lane
    finished. Reading across is the same move `screen_ledgers` already makes."""
    try:
        rec = json.loads((STATE / "booter" / f"{uuid}.json").read_text())
    except Exception:
        return 0
    until = rec.get("boot_after_until") or 0
    return until if until > time.time() else 0


def _seat_handover_repair(host, dry):
    """⛔⛔ THE FIX FOR `succeed` LIVES IN A FILE THAT SEVENTEEN CHECKOUTS OWN.

    `succeed` now migrates a retiring row's subscription to its successor — but
    `succeed` is invoked BY THE RELAYING LANE, from that lane's own copy of this
    file. A lane six commits behind runs the old body, deletes the record, and
    mints the orphan exactly as before. Reported by a 9.x seat that relays hourly
    and had watched an orchestrator hand-backfill the same gap all night:
    *"your fix is correct; it is just not where the call happens."*

    ⇒ Same shape as the arming guard, which was moved out of `cmd_subscribe`
      (eleven copies, lane-invoked) into the tick (one process, the point of
      harm). There is ONE watcher and it runs from a current checkout.

    ⚠ BUT THE TICK CANNOT RECOVER WHAT IS ALREADY DELETED. By the time a stale
    `succeed` has run, the predecessor's record is gone and there is nothing left
    to migrate — so a purely reactive sweep is too late by construction. This
    therefore REMEMBERS each seat's membership while it is healthy, and restores
    it to whoever holds the seat next.

    ⛔ IT NEVER INVENTS MEMBERSHIP. A seat that was never subscribed is never
    subscribed by this, which is what keeps it away from the rows that stood down
    DELIBERATELY (three of them on 9.x) and from the owner's own copilot row —
    none of those has a remembered record, because none was ever on the plane.
    The attended screen is consulted as a second floor regardless."""
    rows, ok = live_rows(host)
    if not ok:
        return []          # blind is not empty; a repair on no evidence is a guess
    attended, _ = screen_ledgers()
    if attended is None:
        return []          # the same refusal the wake path takes, for the same reason
    try:
        mem = json.loads(SEAT_MEMORY.read_text()) if SEAT_MEMORY.exists() else {}
    except Exception:
        mem = {}
    subs = {s["uuid"]: s for s in load_subs()}
    repaired = []
    candidates = collections.defaultdict(list)
    covered = set()          # seats already held by a SUBSCRIBED live row
    for r in rows:
        seat = (r.get("outline_prefix") or "").strip()
        uuid = _bare_uuid(r.get("full_path") or "")
        if not seat or not uuid:
            continue
        if uuid in subs:                       # healthy: remember this seat's shape
            s = subs[uuid]
            mem[seat] = {"role": s.get("role"), "campaign": s.get("campaign"),
                         "escalate_to": s.get("escalate_to"),
                         "escalate_host": s.get("escalate_host"), "host": s.get("host"),
                         # ⛔⛔ WHOSE membership this was. THIS FIELD IS THE WHOLE FIX.
                         # A HANDOVER IS A CHANGE OF HOLDER; without the previous
                         # holder's identity this cannot tell a handover from a
                         # STAND-DOWN, and it re-nets rows that deliberately left.
                         "for_uuid": uuid}
            covered.add(seat)                  # somebody live already holds it
            continue
        # ⛔⛔ A RETIRED ROW IS STILL A LISTED ROW, AND IT STILL HOLDS ITS SEAT.
        # The first version of this restored membership to EVERY unsubscribed
        # holder of a remembered seat — and a seat that has relayed five times has
        # four corpses still listed under it. It resurrected all four onto the
        # supervision plane within one tick, where each would escalate as a
        # stalled lane, and a boot would have set a corpse racing its live
        # successor over the same files. **The resurrection hazard, arriving
        # through the repair built to prevent it.** Caught within 5 minutes by the
        # seat whose lane relays hourly, monitor-only, before anything was armed.
        #
        # ⇒ The row says so ITSELF. A retiring row rewrites its own title to
        #   "RETIRED, succeeded by <uuid>" — that is the row speaking about
        #   itself, not a ledger claiming something about it, and it is the most
        #   trustworthy signal available here.
        label = (r.get("label") or "")
        if re.search(r"\bRETIRED\b|succeeded by", label, re.I):
            continue
        if uuid[:8] in attended:
            continue
        if mem.get(seat):
            candidates[seat].append(uuid)
    for seat, uuids in candidates.items():
        prev = mem.get(seat)
        # ⛔⛔ A SEAT SOMEBODY ALREADY HOLDS NEEDS NO REPAIR. Restoring a SECOND row
        # onto a covered seat is the duplicate-claimant state this function has a
        # screen against — and I walked straight past that screen by counting only
        # the UNSUBSCRIBED candidates. Measured 2026-08-14: a lane stood down and
        # unsubscribed itself, its successor was seated two minutes later, and the
        # tick then restored the RETIRED predecessor alongside the live successor,
        # which promptly escalated the corpse as an idle lane needing a decision.
        if seat in covered:
            continue
        # ⛔⛔ AND A STAND-DOWN IS NOT A HANDOVER. If the row still holding the seat
        # is the SAME uuid the membership was recorded for, nothing was handed over
        # — that row simply left the plane on purpose, and putting it back overrides
        # a deliberate decision with a stale memory of it. Third misfire of this
        # repair in one session, and the through-line is that I kept writing screens
        # about STATES when the thing being repaired is a TRANSITION.
        same = [u for u in uuids if prev and _same_uuid(u, prev.get("for_uuid") or "")]
        if same:
            log(f"  ⏭ seat {seat}: {same[0][:8]} is the SAME row the membership was "
                f"recorded for — a stand-down, not a handover; leaving it off the plane")
            uuids = [u for u in uuids if u not in same]
            if not uuids:
                continue
        # ⛔ AT MOST ONE HOLDER PER SEAT. Two live claimants on one seat is a state
        # that needs a person, not a broadcast — restoring to both would point two
        # rows at one escalation target and make the duplicate look sanctioned.
        if len(uuids) != 1:
            log(f"  ⚠ seat {seat} has {len(uuids)} unsubscribed live holders "
                f"({', '.join(u[:8] for u in uuids)}) — restoring to NONE; a seat with "
                f"two claimants is a human's call")
            continue
        uuid = uuids[0]
        if not dry:
            rec = dict(prev)
            rec.update({"uuid": uuid, "seat": seat, "owner_pinned": False, "booter": True,
                        "since": int(time.time()),
                        "intent": f"seat {seat} membership restored at handover by the tick"})
            sub_path(uuid).write_text(json.dumps(rec, indent=1))
        repaired.append(f"seat {seat}: {uuid[:8]} inherited "
                        f"role={prev.get('role')} →{(prev.get('escalate_to') or 'human')[:8]}")
    for x in repaired:
        log(f"  {'DRY would restore' if dry else '⭐ RESTORED'} {x}")
    if not dry:
        try:
            SEAT_MEMORY.write_text(json.dumps(mem, indent=1))
        except Exception as e:
            log(f"  ⚠ could not persist seat membership: {e}")
    return repaired


def cmd_normalize(a):
    """⛔ A SHORT POINTER IS NOT A COSMETIC PROBLEM — IT IS A SEVERED ESCALATION.

    `escalate_to` is stored verbatim from whatever a brief quoted, and briefs
    quote 8 chars because that is what the board prints. Stored short it breaks
    every consumer that compares it against the row plane, which always answers
    with 36 — so the lane's cries fell back to a human card while the log claimed
    its live orchestrator was dead.

    The consumers now prefix-match, so nothing is BROKEN by a short value any
    more. This exists because the data should still be right: a frozen brief
    re-introduces one on every spawn, and a stub cannot be told from a good
    pointer by eye. Run it after a wave of spawns.

    ⇒ Found 2026-08-14 the moment the board was made to MARK the stubs: SIX rows
    across three other campaigns (2.x, 3.x, 9.x) were carrying them, every one
    backfilled by an orchestrator that had quoted its own board. The display was
    hiding a fleet-wide defect, not a one-row slip."""
    fixed, unresolved, scanned = _normalize_pointers(a.dry_run, quiet=False)
    log(f"normalize: {len(fixed)} expanded, {len(unresolved)} unresolved, "
        f"{scanned} subscription(s) scanned")
    return 0


def _normalize_pointers(dry, quiet=True):
    """Expand every short `escalate_to`. Shared by the verb and the tick.

    ⛔ ONE IMPLEMENTATION, because two would be the second encoding this whole
    fix exists to remove. `quiet` only suppresses the no-op chatter of a
    background tick — a repair is always logged, or the plane heals silently and
    nobody learns their brief is teaching lanes to write stubs."""
    known = [p.stem for p in SUBS.glob("*.json")]
    fixed, unresolved = [], []
    for s in load_subs():
        to = _bare_uuid(s.get("escalate_to") or "")
        if not to or len(to) >= 36:
            continue
        hits = [k for k in known if k.startswith(to)]
        if len(hits) != 1:
            unresolved.append(f"{s['uuid'][:8]}(seat {s.get('seat') or '-'}) -> "
                              f"{to} matches {len(hits)} subscriptions")
            continue
        if not dry:
            s["escalate_to"] = hits[0]
            sub_path(s["uuid"]).write_text(json.dumps(s, indent=1))
        fixed.append(f"{s['uuid'][:8]}(seat {s.get('seat') or '-'}) {to} -> {hits[0][:8]}…")
    for x in fixed:
        log(f"  {'DRY would expand' if dry else 'expanded'} {x}")
    for x in unresolved:
        log(f"  ⚠ LEFT ALONE — {x}")
    if not quiet and not fixed and not unresolved:
        pass  # the summary line the caller prints says it
    return fixed, unresolved, len(known)


RELEASED_LEDGER = STATE / "monitor-released.tsv"
RESUB_MARK = "__resubscribed__:"


def released_rows():
    """Rows that RELEASED their own monitor subscription, with a reason.

    ⛔⛔ THE ASYMMETRY THIS CLOSES. The booter has release memory — `optout`
    writes a ledger and a later `subscribe` is refused without `--rearm '<why>'`
    — and the monitor had NONE. So a row could protect itself on one plane and
    be silently restored on the other, and a deliberate stand-down was
    indistinguishable from a subscription nobody had ever made.

    ⇒ Measured 2026-08-14: a campaign closed itself down and released this
    subscription as its final act. A well-meant repair restored it — the board
    entry literally read *"subscription restored by …"* — which resurrected a
    finished row into the unarmed-row population, where the board then demanded
    a decision about it on every listing. Both offered answers were wrong,
    because the row was neither an unattended delegate nor a place a human
    types: **it was DONE.**

    ⭐ The third state is not a new classification to compute. It is what a
    release already meant, thrown away because nothing recorded it.

    **Append-only, latest record per uuid wins.** A re-subscribe is a new line
    marked `__resubscribed__:`, so the decision history is kept rather than
    rewritten — the file is evidence, not state to edit.

    ⚠ Unreadable is NOT empty: a damaged ledger returns None, and the caller
    refuses to resurrect anything rather than treating silence as consent."""
    out = {}
    try:
        for line in RELEASED_LEDGER.read_text().splitlines():
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) < 2:
                continue
            uuid = parts[1].strip()
            reason = parts[2].strip() if len(parts) > 2 else ""
            if not uuid:
                continue
            if reason.startswith(RESUB_MARK):
                out.pop(uuid, None)
            else:
                out[uuid] = reason or "no reason recorded"
    except FileNotFoundError:
        return {}
    except Exception:
        return None
    return out


def _record_release(uuid, reason):
    try:
        STATE.mkdir(parents=True, exist_ok=True)
        with RELEASED_LEDGER.open("a") as fh:
            fh.write("%s\t%s\t%s\n" % (
                time.strftime("%Y-%m-%dT%H:%M:%S%z"), uuid, reason or ""))
        return True
    except Exception as exc:
        log(f"⛔ could not record the release: {exc}")
        return False


def cmd_unsubscribe(a):
    uuid = _bare_uuid(a.uuid)
    p = sub_path(uuid)
    if p.exists():
        p.unlink()
        # ⛔ Record it BEFORE reporting success. A release that leaves no trace
        #    is exactly what let a finished row be restored by someone tidying
        #    up, and "unsubscribed" printed either way.
        _record_release(uuid, getattr(a, "note", "") or "")
        log(f"unsubscribed {a.uuid[:8]} (release recorded)")
    else:
        log(f"{a.uuid[:8]} was not subscribed — nothing to do")
    return 0


def cmd_succeed(a):
    """⛔ REAPING DOES NOT UNSUBSCRIBE, AND THE ORPHANS ESCALATE INTO A CORPSE.

    Retiring an orchestrator removes its ROW. It does not touch this plane, so
    every cluster that named it in `escalate_to` goes on naming it — and
    `escalate()` addresses `remote-cc://<host>/<dead-uuid>` unconditionally and
    logs "escalated to orchestrator". The send lands nowhere and reports success,
    which is the worst available shape: the supervision plane looks healthy while
    no escalation from any cluster can arrive.

    Measured 2026-08-13 at a seat-6.0 handover: FIVE cluster rows were left
    escalating to a UUID reaped ninety seconds earlier. Nothing reported it —
    a cluster cannot see this at all, because from inside a cluster the plane
    it escalates into is background weather.

    ⇒ Succession must move the subscribers with the seat. One call, and
    ygg-claim.sh --replace runs it for you."""
    old, new = _bare_uuid(a.from_uuid), _bare_uuid(a.to_uuid)
    if not old or not new:
        log("succeed: need --from <old-orchestrator> and --to <new>")
        return 64
    moved = []
    for s in load_subs():
        # ⛔ PREFIX-MATCH, NEVER EQUALITY — the row this function exists to rescue
        # is exactly the one that stored a SHORT pointer, because that is also the
        # row whose escalations were already misrouting. `==` skipped it silently
        # and the succession still reported a clean "3 row(s) re-pointed", so the
        # board read healthy with one lane escalating into a corpse. Caught
        # 2026-08-14 by the incoming 6.0 on its own claim, one commit after the
        # same one-function-fixed-its-sibling-was-not shape in the booter.
        if _same_uuid(_bare_uuid(s.get("escalate_to") or ""), old):
            s["escalate_to"] = new
            if a.escalate_host:
                s["escalate_host"] = a.escalate_host
            sub_path(s["uuid"]).write_text(json.dumps(s, indent=1))
            moved.append(f"{s['uuid'][:8]}(seat {s.get('seat') or '-'})")
    for who in moved:
        log(f"  re-pointed {who} -> {new[:8]}")
    log(f"succeeded {old[:8]} -> {new[:8]}: {len(moved)} row(s) re-pointed")
    # ⛔⛔ A SUCCESSOR INHERITS THE SEAT AND NOT THE PLANE, SO EVERY RELAY MINTS A
    # FRESH ORPHAN. This used to just DELETE the predecessor's own subscription:
    # the retiring row left the plane, the incoming row was never added, and the
    # board then showed a live seat armed on the booter and escalating to nobody.
    #
    # Reported independently by TWO campaigns within an hour, 2026-08-14. One
    # orchestrator ran `succeed` correctly, saw its subscribers re-point, and did
    # not notice it had never subscribed ITSELF. The other relays roughly hourly
    # and said it plainly: *each relay leaves the predecessor subscribed and the
    # successor unsubscribed, so the gap is generated fresh every time.* An
    # orchestrator backfilling those by hand is treating a symptom that the tool
    # re-creates on the next handover.
    #
    # ⇒ MIGRATE the record instead of deleting it. Role, campaign and escalate_to
    #   are properties of the SEAT, not of the session sitting in it.
    # ⚠ Never clobber: a successor that already subscribed itself knows more about
    #   its own intent than the corpse does.
    p = sub_path(old)
    if p.exists() and old != new:
        try:
            rec = json.loads(p.read_text())
        except Exception:
            rec = None
        if rec and not sub_path(new).exists():
            rec["uuid"] = new
            rec["since"] = int(time.time())
            rec["intent"] = (rec.get("intent") or "") + " (inherited at succession)"
            sub_path(new).write_text(json.dumps(rec, indent=1))
            log(f"  ⭐ successor {new[:8]} INHERITED the seat's subscription "
                f"(role={rec.get('role')}, seat={rec.get('seat') or '-'})")
        elif rec:
            log(f"  successor {new[:8]} was already subscribed — left alone")
        p.unlink()
        log(f"  unsubscribed the retired orchestrator {old[:8]}")
    return 0


def cmd_park(a):
    """⭐ A ROW BLOCKED ON PURPOSE IS NEITHER WORKING NOR FINISHED, AND THE
    CLASSIFIER HAD NO STATE FOR IT.

    An idle row is reported as *"most likely FINISHED its scope — give it more
    work, relay it, or reap it"*. That is right for a row that ran out of work
    and wrong for one the ORCHESTRATOR deliberately blocked, which is a different
    decision with a different answer. Measured 2026-08-13 within five minutes:
    seat 6.2 was idle because a deploy freeze forbade the desktop screenshot its
    remaining scope needed, and seat 6.3 was idle because its next step needs a
    field in a file another seat was mid-edit in. Both escalated as probably
    finished; reaping either would have destroyed live context.

    ⛔ The cost is not the noise — an episode latch already stops the repeat. It
    is that the REASON lives only in the orchestrator's head, so the next
    orchestrator re-derives it from scratch, and the obvious reading of an idle
    row is the destructive one.

    ⚠ EVERY PARK EXPIRES. A suppression without an expiry is how a row goes
    unsupervised forever, which is the failure this whole plane exists to
    prevent — so `--hours` is bounded and the tick resumes normal classification
    the moment it lapses, saying so out loud."""
    p = sub_path(_bare_uuid(a.uuid))
    if not p.exists():
        log(f"{a.uuid[:8]} is not subscribed")
        return 1
    if not a.reason:
        log("park: --reason is required — a park nobody can read is a silence")
        return 64
    hours = max(0.1, min(float(a.hours), 24.0))
    s = json.loads(p.read_text())
    s["parked"] = True
    s["parked_reason"] = a.reason
    s["parked_until"] = int(time.time() + hours * 3600)
    s["parked_by"] = _bare_uuid(a.by or os.environ.get("YGGTERM_SESSION_ID", ""))
    p.write_text(json.dumps(s, indent=1))
    log(f"⭐ {a.uuid[:8]} PARKED for {hours:g}h — not escalated as finished while it waits.")
    log(f"   Reason: {a.reason}")
    log(f"   ⚠ Expires {time.strftime('%H:%M', time.localtime(s['parked_until']))}; "
        "after that it classifies normally again.")
    return 0


def cmd_unpark(a):
    p = sub_path(_bare_uuid(a.uuid))
    if not p.exists():
        log(f"{a.uuid[:8]} is not subscribed")
        return 1
    s = json.loads(p.read_text())
    was = s.pop("parked_reason", "")
    for k in ("parked", "parked_until", "parked_by"):
        s.pop(k, None)
    p.write_text(json.dumps(s, indent=1))
    log(f"{a.uuid[:8]} unparked — back under normal classification"
        + (f" (was: {was})" if was else ""))
    return 0


def cmd_demote(a):
    """The owner takes a row back. Nothing automated touches it again."""
    p = sub_path(_bare_uuid(a.uuid))
    if not p.exists():
        log(f"{a.uuid[:8]} is not subscribed")
        return 1
    s = json.loads(p.read_text())
    s["owner_pinned"] = True
    s["pinned_reason"] = a.reason or "owner took this row back"
    p.write_text(json.dumps(s, indent=1))
    log(f"⭐ {a.uuid[:8]} DEMOTED to a normal session — no nudges, no escalation, no reaping.")
    log(f"   Reason: {s['pinned_reason']}")
    log("   ⚠ Its booter subscription is separate: `ygg-booter.py unsubscribe` to silence that too.")
    return 0


def cmd_promote(a):
    p = sub_path(_bare_uuid(a.uuid))
    if not p.exists():
        log(f"{a.uuid[:8]} is not subscribed")
        return 1
    s = json.loads(p.read_text())
    s["owner_pinned"] = False
    s.pop("pinned_reason", None)
    p.write_text(json.dumps(s, indent=1))
    log(f"{a.uuid[:8]} promoted back under supervision")
    return 0


def report_watcher_health():
    """⛔ A SUBSCRIPTION LIST IS NOT A SUPERVISION GUARANTEE, and until now `list`
    could not tell the two apart: it printed the same contented roster whether a
    watcher was running or had exited hours ago. That is this project's own
    pathology — an instrument answering truthfully about a question nobody asked
    it. So `list` now states its subject: who is subscribed AND whether anything
    is actually looking."""
    procs = watcher_procs()
    if not procs:
        log("⛔ NO WATCHER IS RUNNING — the subscriptions below are being read by NOBODY.")
        log("   Start one:  ygg-monitor.py watch --watch 86400 --interval 240")
        return
    # ⛔ AGE IS NOT LIFE. A watcher's age only means something to someone who
    # already knows its deadline, and the deadline is the thing that kills it.
    # `age=5h48m` printed on a 6h window read as healthy and was twelve minutes
    # from ending the campaign's only supervision. So report what is LEFT, and
    # let the age be the supporting detail rather than the headline.
    for pid, age, window in procs:
        fmt = lambda s: f"{s // 3600}h{(s % 3600) // 60:02d}m"
        if window is None:
            log(f"⚠ watcher pid={pid} age={fmt(age)} — window UNKNOWN, so time-to-death "
                f"cannot be stated. Treat it as expiring at any moment.")
            continue
        left = window - age
        if left <= 0:
            log(f"⛔ watcher pid={pid} is PAST its {fmt(window)} deadline and is exiting.")
        elif left <= 3600:
            log(f"⛔ watcher pid={pid} DIES IN {fmt(left)} (age {fmt(age)} of {fmt(window)}) — "
                f"restart it now, or {len(load_subs())} subscriber(s) lose their reader.")
        else:
            log(f"✅ watcher pid={pid} {fmt(left)} left (age {fmt(age)} of {fmt(window)})")
    if len(procs) > 1:
        log(f"⚠ {len(procs)} WATCHERS RUNNING — they will double-escalate. Kill all but one.")


def cmd_list(a):
    subs = load_subs()
    report_watcher_health()
    rl = _booter().rate_limit_hold()
    if rl:
        if rl.get("indefinite"):
            log(f"⏸ QUOTA HOLD INDEFINITE — it will NEVER lift by itself ({rl.get('reason','')}) — ⛔ NO WAKE OR ESCALATION DELIVERED WHILE THIS IS UP")
        else:
            left_m = int(((rl.get("until") or 0) - time.time()) // 60)
            log(f"⏸ QUOTA HOLD {left_m}m left — ({rl.get('reason','')}) — ⛔ NO WAKE OR ESCALATION DELIVERED WHILE THIS IS UP")
    if not subs:
        log("no subscribers")
        return 0
    for s in subs:
        pin = "  ⭐ OWNER-PINNED" if s.get("owner_pinned") else ""
        if s.get("parked"):
            left = int(((s.get("parked_until") or 0) - time.time()) // 60)
            pin += (f"  ⏸ PARKED {left}m left: {s.get('parked_reason','')[:44]}"
                    if left > 0 else f"  ⏸ PARK LAPSED: {s.get('parked_reason','')[:44]}")
        # ⛔ THE COLUMN THAT HID THE BUG. Rendering `[:8]` makes a stored 8-char stub
        # and a good 36-char pointer PIXEL-IDENTICAL, so the board — the instrument
        # this seat is told to believe over every table — could not show that a lane
        # was escalating into nothing. Mark the stub rather than widen the column.
        _to = _bare_uuid(s.get("escalate_to") or "")
        stub = "!" if _to and len(_to) < 36 else " "
        log(f"{s['uuid'][:8]}  {s.get('role','relay'):<13} seat={str(s.get('seat') or '-'):<5} "
            f"→{(_to or 'human')[:8]}{stub} {(s.get('intent') or '')[:44]}{pin}")
    report_escalation_gap(subs)
    return 0


def report_escalation_gap(subs):
    """⛔⛔ CROSS THE TWO PLANES — EACH ONE ALONE LOOKS COMPLETE.

    A row armed on the BOOTER but absent from the MONITOR is watched by a timer
    that can wake it and by **nobody who would notice it had stopped**: if it
    stalls, the escalation rings into an empty room. Nothing anywhere reported
    that asymmetry, because each roster is internally consistent — the booter
    listed the row, the monitor listed a complete-looking set without it.

    Measured 2026-08-14: **nine** live relays in that state at one crossing, and
    **four more two hours later** — one of them the successor of a row that had
    relayed *during* the first sweep. ⇒ **A manual backfill is a SNAPSHOT, and a
    relay invalidates it silently**, which is precisely why this belongs in the
    tool and not in an orchestrator's discipline. (Reported by a sibling
    orchestrator that hit the same trap from the other side.)

    ⚠ This REPORTS; it does not arm and does not subscribe. That restraint is
    deliberate: auto-arming is a separate, blocked item — the booter's function is
    to TYPE INTO a stalled session, so arming the wrong row types into a human's
    terminal, and `booter-disarmed.tsv` has no reader yet, so an eager arm would
    silently re-arm every deliberately disarmed row. **Reporting a gap is
    read-only and safe; closing it automatically is not yet.**
    """
    # Read the booter's STATE DIRECTORY, not its `list` output. The state dir is
    # shared by every checkout; the script is not (a guard living only in one
    # copy of a script is the failure this fleet already paid for). It also
    # carries `gone_sightings`, which the printed roster does not.
    booter_subs = STATE / "booter"
    armed, dying = set(), set()
    try:
        files = sorted(booter_subs.glob("*.json"))
    except Exception:
        files = []
    if not files:
        # ⛔ An empty result is a tool that never ran, not a clean bill of health.
        log(f"⚠ no booter roster at {booter_subs} — coverage NOT verified")
        return
    for p in files:
        try:
            rec = json.loads(p.read_text())
        except Exception:
            continue
        u = (rec.get("uuid") or p.stem)[:8]
        # ⛔ A CORPSE IS NOT AN UNCOVERED ROW, AND CONFLATING THEM KILLS THE
        #    WARNING. A retired row stays on the booter until GONE_SIGHTINGS
        #    confirms it — deliberate slowness that exists because a fast
        #    unsubscribe once deleted nine LIVE subscriptions in six seconds. So
        #    every reaped row would be reported here as a gap, every time, until
        #    it aged out.
        #    ⚠ The cost is asymmetric and that is the trap: a false alarm merely
        #    looks like diligence, so it survives, and the warning is trained
        #    into background noise right up until the one that mattered. The
        #    booter has already begun counting these down; they need no human.
        if rec.get("gone_sightings", 0) > 0:
            dying.add(u)
        else:
            armed.add(u)
    watched = {s["uuid"][:8] for s in subs}
    # ⛔⛔ A PARK *IS* THE RECORDED DECISION, AND THE ALARM BELOW WAS DEMANDING IT
    #    WHILE PRINTING IT. `park` takes a MANDATORY `--reason` and an expiry, so
    #    a parked row has already answered "arm it or never-arm it" — with a third
    #    answer, the one `released_rows()` exists to name: it is neither an
    #    unattended delegate nor a place a human types, it was DONE. One `list`
    #    rendered both, four lines apart, and the next orchestrator re-decided it
    #    every listing.
    #    ⚠ Why the existing screens could not catch it: the row is ALIVE, so the
    #    liveness check added for the reaped-row case passes it through — alive
    #    and FINISHED is its own state. And both offered remedies are the two
    #    this file forbids: arming it types into a row whose remaining work is
    #    owner-gated, and never-arming it writes a permanent assertion that a
    #    human types at a delegate's address into the one file whose whole job is
    #    to stop a machine typing at a person.
    #    ⇒ Reported by the 2.x orchestrator 2026-08-14, read from the source
    #    rather than inferred, on its own publication lane.
    # ⚠ A LAPSED park is deliberately NOT screened: the expiry is the point, and a
    #   park that ran out is a row nobody came back for — exactly what the alarm is
    #   for. Only a park still inside its window silences it.
    now = time.time()
    parked = {s["uuid"][:8] for s in subs
              if s.get("parked") and (s.get("parked_until") or 0) > now}
    # ⛔⛔ AND THE FORWARD CHECK HAD THE SAME BLIND SPOT THE REVERSE ONE WAS FIXED
    #    FOR: it screened neither a park nor a RELEASE, so a row that stood itself
    #    down deliberately was reported as "escalating into an empty room" — an
    #    alarm whose only offered remedy is to re-subscribe it. `subscribe` then
    #    REFUSES that exact repair, demanding `--rearm '<why>'`, so the two verbs
    #    disagreed about the same row: one manufactured the work, the other
    #    forbade it. Measured 2026-08-20 by seat 11.0, which was one flag away
    #    from resurrecting FOUR rows holding a "phase 1 pre-reset clean hold".
    #    ⇒ A release is a third answer, exactly as `released_rows()` says. The
    #    listing must name it, not re-open it.
    # ⚠ Unreadable is NOT empty, and the screen is the dangerous half: a damaged
    #   ledger must not silently downgrade real gaps, so an unreadable ledger
    #   reports the gap UNSCREENED and says the screen could not run.
    rel = released_rows()
    rel_short = {u[:8] for u in rel} if rel is not None else set()
    gap = sorted(armed - watched - rel_short)
    stood_down = sorted((armed - watched) & rel_short)
    if gap:
        log(f"⛔ {len(gap)} ROW(S) ARMED ON THE BOOTER BUT ESCALATING TO NOBODY — "
            f"a stall would ring into an empty room:")
        for u in gap:
            log(f"   {u}  ⇒ subscribe it with --escalate-to <its campaign's orchestrator>")
        log("   ⚠ cwd is a PRIOR, not the answer: a row can work in a checkout that has")
        log("     nothing to do with its subject. Confirm against its last prose turn.")
        if rel is None:
            log("   ⛔ the release ledger is UNREADABLE — this list is UNSCREENED and may")
            log("     name rows that stood themselves down. Fix the ledger before acting.")
    if stood_down:
        log(f"({len(stood_down)} row(s) armed on the booter RELEASED their own supervision "
            f"deliberately — not a gap, no action:)")
        for u in stood_down:
            why = next((v for k, v in (rel or {}).items() if k[:8] == u), "") or "(no reason recorded)"
            log(f"   {u}  stood down: {why[:88]}")
    # ⛔⛔ A CROSSING THAT CHECKS ONE DIRECTION IS NOT A CROSSING. This reported
    # only `armed - watched` (on the booter, escalating to nobody) and was blind
    # to the reverse — subscribed here, unarmed there — while calling itself the
    # coverage crossing and printing "0 warnings". Measured cost: seat 6.6 ran a
    # full hour with a monitor subscription and no booter arm, and this said the
    # board was clean the whole time. A peer's separate report found it.
    #
    # ⛔⛔ AND THE REVERSE CHECK IS THE DANGEROUS HALF — `never_arm()`'s own
    # docstring predicts exactly this failure: the booter's remedy is to TYPE
    # INTO a row. An attended row that ever gains a monitor subscription would
    # surface here as "unarmed", whose obvious remedy is to arm it, walking a
    # well-meant tidy-up straight into typing over someone's unsent draft. So
    # attended and opted-out rows are excluded, and if those lists cannot be READ
    # this refuses to report at all rather than name a row it could not screen.
    # ⚠ Missing is not unreadable: an absent list legitimately means "nobody yet".
    attended, optedout = screen_ledgers()
    screens_ok = attended is not None and optedout is not None
    attended, optedout = attended or set(), optedout or set()

    if not screens_ok:
        log("⚠ never-arm / opt-out ledger UNREADABLE — the unarmed-row check did NOT run.")
        log("   Refusing to name rows I cannot screen: an attended row listed here would")
        log("   invite arming it, and the booter's remedy is to TYPE INTO the row.")
    else:
        unarmed = sorted(watched - armed - dying - attended - optedout - parked)
        # Keep the state VISIBLE without demanding action on it — the point is to
        # stop asking, not to stop showing.
        silenced = sorted((watched - armed - dying - attended - optedout) & parked)
        if silenced:
            log(f"⏸ {len(silenced)} unarmed row(s) are PARKED with a reason — decision already "
                f"recorded, not asked about: {' '.join(silenced)}")
        if unarmed:
            # ⛔⛔ A DEAD ROW IS A THIRD CASE, AND OFFERING ONLY TWO PUT A LIE IN
            #    THE GUARD FILE. This asked "arm it, or add it to never-arm.tsv"
            #    about a row that had been REAPED hours earlier. Both options
            #    were wrong, and the asymmetry is what matters: arming a corpse
            #    is merely useless, while never-arming one writes a PERMANENT
            #    ASSERTION THAT A HUMAN TYPES AT THAT ADDRESS into the one file
            #    whose entire job is to stop a machine typing at a person — and
            #    no later audit questions it, because that is what the file
            #    means. Reported by a peer campaign 2026-08-14, which correctly
            #    unsubscribed instead and told me the prompt was the defect.
            # ⇒ The liveness check costs one /proc scan and turns "record a
            #   decision" into "unsubscribe this, it is dead", which needs no
            #   judgement at all.
            live_here = _agent_uuids_on_this_host()
            by_stub = {s["uuid"][:8]: s for s in subs}
            log(f"⛔ {len(unarmed)} ROW(S) SUBSCRIBED HERE BUT NOT ARMED ON THE BOOTER — "
                f"an escalation target exists, but nothing will WAKE a stall:")
            for u in unarmed:
                rec_s = by_stub.get(u, {})
                full = rec_s.get("uuid") or ""
                # An empty host means this machine; see the note at `sub.get("host", "dev")`.
                host = (rec_s.get("host") or "").strip() or _this_host()
                if host != _this_host():
                    # ⚠ BLIND IS NOT DEAD. A remote row's process cannot be seen
                    #    from here, and reporting that absence as death is the
                    #    exact instrument error this campaign keeps paying for.
                    log(f"   {u}  ⇒ RECORD A DECISION — arm it if it is an unattended delegate;")
                    log(f"        add it to never-arm.tsv if a person types in it.")
                    log(f"        ⚠ lives on {host}: liveness NOT checked from here, so "
                        f"'no process' is not among the things I know.")
                elif full and full not in live_here:
                    log(f"   {u}  ⇒ DEAD — no agent process on {host} holds it. `unsubscribe` it.")
                    log(f"        ⛔ Do NOT put it in never-arm.tsv. That file asserts a HUMAN "
                        f"types at this address; a reaped uuid there is a lie the guard can "
                        f"never question.")
                else:
                    log(f"   {u}  ⇒ RECORD A DECISION — arm it if it is an unattended delegate;")
                    log(f"        add it to never-arm.tsv if a person types in it.")
            log("   ⛔ Do NOT bulk-arm: no probe separates those two cases, and guessing")
            log("     wrong types into a human. Decide per row.")
    if dying:
        log(f"   ({len(dying)} retired row(s) on the booter are being counted down "
            f"by GONE_SIGHTINGS — not a gap, no action)")


def fishy_audit(subs, dry):
    """⛔⛔ AN ORCHESTRATOR'S OWN HOLDS BLIND IT TO ITS ROWS GOING COLD.

    Owner-directed 2026-08-13, after a relay sat at **6.1 MB and 37 minutes
    cold** while its orchestrator believed the fleet was healthy. Two causes, and
    the first is the orchestrator's own doing:

    1. **`park` suppresses the IDLE verdict** — correctly, because a row blocked
       on purpose is not finished. But IDLE was the ONLY signal that ever
       mentioned that row, so parking it silenced the health report along with
       the wrong verdict. **A hold must silence a VERDICT, never an AUDIT.**
    2. **Nothing here ever measured what a wake would COST.** Every verb asked
       "is it working"; none asked "what would it cost me to find out". A cold
       multi-megabyte row is priced at dollars per wake, charged before it
       answers a word — so the cheapest question is the one nobody was asking.

    ⇒ This runs over EVERY subscriber, parked and pinned included, and reports
    only anomalies. It never nudges, wakes or reaps — it exists so the
    orchestrator sees the fishy row BEFORE it costs something."""
    WAKE_EXPENSIVE_KB = 2048      # a wake re-reads this much context, cold
    COLD_MINS = 25                # long enough that the cache is likely gone
    findings = []
    now = time.time()
    live_row_set = None
    for s in subs:
        uuid = s["uuid"]
        tag = f"{uuid[:8]} (seat {s.get('seat') or '-'})"
        f = _transcript_for(uuid, s.get("host"))
        if f:
            kb, age_m = f[1] // 1024, int((now - f[2]) // 60)
            if kb >= WAKE_EXPENSIVE_KB and age_m >= COLD_MINS:
                findings.append(
                    f"⚠ {tag} is {kb//1024}.{(kb%1024)*10//1024} MB and {age_m}m COLD — "
                    f"a wake re-reads it all before answering. SUCCEED IT BY HARVESTING, never by asking.")
            elif age_m >= 180:
                findings.append(f"⚠ {tag} silent {age_m}m — confirm it is meant to be idle")
        else:
            findings.append(f"⚠ {tag} has NO TRANSCRIPT — its brief may never have been delivered")
        # A stale escalate_to re-enters through frozen briefs, so re-check it here.
        to = _bare_uuid(s.get("escalate_to") or "")
        if to:
            if live_row_set is None:
                _rows, _ok = live_rows(resolve_gui_host())
                # ⛔ Blind means blind. An unanswered row plane must not be
                # allowed to say a live escalation target is dead — that would
                # send the orchestrator to repoint a subscription that is fine.
                live_row_set = ({(r.get("full_path") or "").rsplit("/", 1)[-1]
                                 for r in _rows} if _ok else None)
            # ⛔ TWO LENGTHS OF THE SAME IDENTIFIER, COMPARED BY EQUALITY.
            # `escalate_to` is stored verbatim, so a subscription made with a
            # short uuid holds 8 chars while the row plane speaks full ones —
            # and this check then reported a live orchestrator row as dead.
            # Measured on the row of the seat running the check, 2026-08-13.
            if live_row_set is not None and not any(_same_uuid(to, r) for r in live_row_set):
                findings.append(f"⛔ {tag} escalates to {to[:8]}, which is NOT a live row — its cries go nowhere")
    if findings:
        log("FISHY — the orchestrator should look at these:")
        for x in findings:
            log(f"  {x}")
    return findings


def _transcript_for(uuid, host=None):
    """(path, bytes, mtime) for a row's transcript, local or over ssh.

    ⛔⛔ THE HOST THAT COUNTS AS "LOCAL" IS THIS MACHINE, AND IT USED TO BE THE
    LITERAL STRING "dev". That is correct on exactly one host and wrong on every
    other: run anywhere else, every row belonging to that peer was looked up in
    the LOCAL filesystem, found nothing, and returned None — which `fishy_audit`
    renders as "⚠ has NO TRANSCRIPT — its brief may never have been delivered".
    So the audit produced a false alarm for every remote row of one host, on
    every tick, and an alarm that is always wrong is one nobody reads.

    ⇒ Measured 2026-08-21 from the desktop host: all seven live lanes reported
      NO-TRANSCRIPT while every one of them had a healthy, growing transcript.
    ⚠ A hardcoded peer name inside a locality test is the shape to watch for —
      it makes a tool correct on its author's machine and silently wrong on the
      rest of the fleet."""
    if host and host not in ("", "local", _this_host()):
        # ⛔ NO SINGLE QUOTES IN THIS COMMAND. `_run` wraps every argv element in
        # single quotes, and shell single-quotes cannot nest — so a `stat -c '%s
        # %Y'` here arrives malformed and returns nothing. The audit then read
        # that silence as "NO TRANSCRIPT — the brief may never have been
        # delivered", a false alarm about a healthy row on its very first run.
        # ⇒ A space-free format needs no quoting and cannot be broken this way.
        r = _run(host, ["sh", "-c",
                        f"ls -t ~/.claude/projects/*/{uuid}.jsonl 2>/dev/null | head -1 | "
                        f"xargs -r stat -c %s..%Y"], timeout=20)
        if r and r.stdout.strip():
            try:
                sz, mt = r.stdout.strip().split("..")[:2]
                return ("remote", int(sz), int(mt))
            except Exception:
                return None
        return None
    import glob
    hits = glob.glob(os.path.expanduser(f"~/.claude/projects/*/{uuid}.jsonl"))
    if not hits:
        return None
    p = max(hits, key=os.path.getmtime)
    st = os.stat(p)
    return (p, st.st_size, int(st.st_mtime))


def seat_audit(gui_host, subs, dry):
    """⛔ IDENTITY FAULTS ARE INVISIBLE TO A LIVENESS WATCHER, AND THEY ARE WORSE.

    Every classifier in this file asks "is this row still working". None of them
    can see a row that is working PERFECTLY and should not exist. Measured
    2026-08-13, all three in one sidebar while every subscribed row read WORKING:

      · TWO ROWS SHARING A SEAT — a relay took a number already held, so the
        sidebar had two 6.1s and the pair could only be told apart by grepping
        their transcripts.
      · A REAPED ROW BACK FROM THE DEAD, holding a LIVE agent process, sharing a
        worktree with its own successor. Two agents, one tree — the exact clobber
        that separate worktrees exist to prevent. The reap had answered
        verified:true with live_processes:[] and was wrong.
      · A LIVE AGENT PROCESS SUBSCRIBED TO NOTHING, so no plane was watching it.

    ⇒ A supervision plane that only measures liveness will report a green fleet
    while work is being silently overwritten. These checks are cheap, they run on
    every tick, and they are the half that was missing."""
    findings = []
    rows, plane_ok = live_rows(gui_host)
    if not plane_ok:
        log("  AUDIT ⛔ THE ROW PLANE DID NOT ANSWER — every check below that would "
            "conclude a row is GONE is SKIPPED. This is blindness, not absence.")
    seats = {}
    for r in rows:
        p = str(r.get("outline_prefix") or "")
        if p:
            seats.setdefault(p, []).append(r)
    for seat, rs in sorted(seats.items()):
        if len(rs) > 1:
            findings.append(f"⛔ SEAT {seat} IS HELD BY {len(rs)} ROWS: "
                            + " | ".join((x.get('label') or '?')[:34] for x in rs))

    # ⛔ SCOPE EVERY CHECK TO WHAT THIS ORCHESTRATOR OWNS, AND COUNT THE REST.
    # The first version flagged every agent on the machine, so seven other
    # campaigns' healthy rows arrived as warnings addressed to me. A watcher that
    # reports other people's business is one whose output stops being read —
    # which is how the escalation that started all this ended up in a log nobody
    # opened. Mine = a seat under my own top-level number, or an explicit
    # subscription. Everything else is a COUNT, so it stays visible without
    # becoming noise.
    mine_tops = {str(s.get("seat") or "").split(".")[0] for s in subs if s.get("seat")}
    mine_tops.discard("")

    def is_mine(seat, uuid):
        return uuid in {s["uuid"] for s in subs} or str(seat).split(".")[0] in mine_tops

    foreign_numbered_titles = 0
    for r in rows:
        t = r.get("session_title") or ""
        if re.match(r"^\d+(\.\d+)*\.?\s", t):
            if is_mine(r.get("outline_prefix") or "", (r.get("full_path") or "").rsplit("/", 1)[-1]):
                findings.append(f"⚠ TITLE CARRIES ITS OWN NUMBER (renders twice): {t[:52]}")
            else:
                foreign_numbered_titles += 1

    # Every subscribed row must still exist; a subscription to a vanished row is a
    # watcher watching nothing and reporting healthy.
    live_paths = {r.get("full_path") for r in rows}
    # ⛔ ONLY WHEN THE PLANE ACTUALLY ANSWERED. An unreachable GUI host once made
    # this report every subscriber as vanished — including the orchestrator's own
    # live row — which is a supervisor describing its own blindness as six deaths.
    for s in (subs if plane_ok else []):
        if not any(s["uuid"] in (p or "") for p in live_paths):
            findings.append(f"⚠ SUBSCRIBED BUT NO ROW: {s['uuid'][:8]} (seat {s.get('seat') or '-'}) "
                            "— unsubscribe it or restore the row")

    # And the reverse: an agent process nothing is supervising. ⭐ THIS IS THE ONE
    # THAT CAUGHT A REAPED ROW BACK FROM THE DEAD holding a live CLI in a worktree
    # its own successor was editing — so it is worth keeping even though most hits
    # are other campaigns minding their own business.
    seat_of = {(r.get("full_path") or "").rsplit("/", 1)[-1]: (r.get("outline_prefix") or "")
               for r in rows}
    known = {s["uuid"] for s in subs}
    unsupervised_foreign = 0
    r = _run(None, ["pgrep", "-af", "claude|codex|gemini|amp|opencode"])
    if r:
        for line in r.stdout.splitlines():
            m = re.search(r"--resume\s+([0-9a-f-]{36})|--session-id\s+([0-9a-f-]{36})", line)
            u = (m.group(1) or m.group(2)) if m else None
            if not u or u in known or u not in seat_of:
                continue
            if is_mine(seat_of[u], u):
                findings.append(f"⛔ LIVE AGENT IN MY NUMBER SPACE, UNSUPERVISED: {u[:8]} "
                                f"(seat {seat_of[u] or 'NONE'}) — subscribe it or reap it")
            else:
                unsupervised_foreign += 1

    if foreign_numbered_titles or unsupervised_foreign:
        log(f"  AUDIT (other campaigns, FYI: {foreign_numbered_titles} numbered titles, "
            f"{unsupervised_foreign} unsupervised agents — not mine to fix)")

    for f in findings:
        log(f"  AUDIT {f}")
    if not findings:
        log("  AUDIT seats unique · titles clean · subscriptions match rows")
    return findings


def prune_dead(gui_host, subs, dry):
    """⛔ A REAP DOES NOT UNSUBSCRIBE, SO THE WATCHER ESCALATES A CORPSE.

    Measured 2026-08-13: two rows were retired hours earlier — row gone, processes
    gone, verified — and this plane went on classifying them and escalating them by
    seat, because retiring a row and unsubscribing it are two different acts and
    only one of them happened. Meanwhile their successors were never subscribed, so
    the set being watched and the set doing the work had drifted apart entirely.

    ⭐ That is the SAME failure the babysit spawn-set had, reappearing in its
    replacement: a watcher pinned to a list captured at one moment reports
    faithfully about rows that no longer matter. ⇒ **Re-derive from live state on
    every tick**, never from the file you wrote at launch.

    Prunes only when BOTH are true — no row AND no process. A row that is merely
    absent from the listing is not proof of death; listings have omitted live rows."""
    rows, plane_ok = live_rows(gui_host)
    if not plane_ok:
        log("  PRUNE ⛔ SKIPPED ENTIRELY — the row plane did not answer. An empty "
            "listing is an instrument failure, never evidence that a row died.")
        return
    live = {r.get("full_path") or "" for r in rows}
    for s in subs:
        u = s["uuid"]
        if any(u in p for p in live):
            continue
        host = None if s.get("host") in ("", None, "local") else s.get("host")
        if cli_process(u, host):
            log(f"  PRUNE skip {u[:8]} — no row, but a process is alive (orphan; investigate)")
            continue
        if dry:
            log(f"  DRY would unsubscribe {u[:8]} (seat {s.get('seat') or '-'}) — row and process both gone")
            continue
        p = sub_path(u)
        if p.exists():
            p.unlink()
        log(f"  PRUNED {u[:8]} (seat {s.get('seat') or '-'}) — reaped elsewhere, no longer watched")


def tick(a):
    bs = _babysit()
    # ⛔ Resolve ONCE, out loud. An unresolved host is not a quiet default —
    # it is a blind tick, and every verb below must know that before it runs.
    a.gui_host = resolve_gui_host(a.gui_host)
    # ⛔⛔ REPAIR THE POINTERS HERE, BECAUSE THE TOOL FIX CANNOT REACH WHERE THEY
    # ARE MADE. A short `escalate_to` is produced in PROSE — an orchestrator
    # quotes eight characters into a brief because eight is what the board prints
    # — and written by a LANE, from that lane's own checkout of this file. So the
    # normalisation added to `subscribe` only helps lanes that have rebased since
    # it landed, and the ones that have not are exactly the ones still copying old
    # briefs.
    #
    # Measured 2026-08-14, one hour after fixing the comparison: seat 6.0 sent 6.7
    # a message reading "(459e6b63)", 6.7 re-subscribed with it three minutes
    # later from a worktree two hours behind the fix, and the stub was back. The
    # orchestrator reproduced the defect class it had just closed, by its original
    # mechanism, while holding the fix.
    #
    # ⇒ There is ONE watcher and there are N lane checkouts. Repair from the one.
    #   Idempotent, expands only unambiguous prefixes, silent when there is
    #   nothing to do.
    _normalize_pointers(a.dry_run)
    # ⛔ BEFORE prune_dead, which is what removes the evidence. The repair reads a
    # seat's remembered membership; pruning a retired row first is fine, but
    # recording the HEALTHY state has to happen while the healthy row is still
    # listed, and both live in this one call.
    _seat_handover_repair(a.gui_host, a.dry_run)
    prune_dead(a.gui_host, load_subs(), a.dry_run)
    seat_audit(a.gui_host, load_subs(), a.dry_run)
    # ⛔ Runs over PARKED and PINNED rows too — a hold silences a verdict, not an
    # audit. This is the check that would have surfaced a 6.1 MB row going cold
    # while its orchestrator believed the fleet was healthy.
    fishy_audit(load_subs(), a.dry_run)
    # ⛔ Check fleet-wide quota hold before proceeding to any wake/escalate actions
    rl = _booter().rate_limit_hold()
    if rl:
        reason = rl.get("reason", "")
        if rl.get("indefinite"):
            log(f"⏸ QUOTA HOLD INDEFINITE ({reason}) — all wakeups and escalations SUPPRESSED this tick")
        else:
            left_m = int(((rl.get("until") or 0) - time.time()) // 60)
            log(f"⏸ QUOTA HOLD ({left_m}m left, {reason}) — all wakeups and escalations SUPPRESSED this tick")
        return 0
    # ⛔⛔ SCREEN BEFORE THE LOOP, BECAUSE THIS LOOP TYPES. `wake()` sends a
    #    message and then a lone CR, and a CR into a row somebody is using
    #    SUBMITS whatever they had half-written. The audit above has warned about
    #    exactly this hazard since it was written — for the booter — while this
    #    loop, the one that actually types, screened nothing. The only reason it
    #    had never happened is that no attended row had ever gained a
    #    subscription here, which is safety by omission and one tidy-up from
    #    being removed.
    #    ⚠ Unreadable is not empty, and the safe direction is to wake nobody: the
    #    audits above still run, because a hold silences a verdict, not an audit.
    attended, _optedout = screen_ledgers()
    if attended is None:
        log("⛔ the attended-row list is UNREADABLE — WAKING NOBODY this tick.")
        log("   This loop types into rows; a screen it cannot read is not an")
        log("   empty one, and the remedy here lands in a person's composer.")
        return 0
    for s in load_subs():
        uuid = s["uuid"]
        if uuid[:8] in attended:
            # Purged, not merely skipped: a subscription on an attended row is a
            # standing invitation for the next tick, or the next reader, to act.
            log(f"⛔ {uuid[:8]} is NEVER-ARM (a person types there) yet is SUBSCRIBED "
                f"HERE — dropping the subscription, not waking it.")
            if not a.dry_run:
                sub_path(uuid).unlink(missing_ok=True)
            continue
        if s.get("owner_pinned"):
            log(f"{uuid[:8]} SKIP — owner-pinned ({s.get('pinned_reason','')})")
            continue
        if s.get("parked"):
            left = (s.get("parked_until") or 0) - time.time()
            if left > 0:
                log(f"{uuid[:8]} PARKED       {int(left//60):>3}m left  {s.get('parked_reason','')[:60]}")
                continue
            # ⚠ Lapsed parks announce themselves. A park that expired quietly
            # would be indistinguishable from one still in force, and the row
            # would look supervised while nothing was deciding about it.
            log(f"{uuid[:8]} PARK EXPIRED — was: {s.get('parked_reason','')[:60]}")
            for k in ("parked", "parked_until", "parked_reason", "parked_by"):
                s.pop(k, None)
            sub_path(uuid).write_text(json.dumps(s, indent=1))
        rhost = None if s.get("host") in ("", None, "local") else s.get("host")
        raw = bs.classify(uuid, rhost)
        state, why = refine(raw, uuid, rhost)
        row = bs.resolve_row_path(a.gui_host, uuid) or f"remote-cc://{_host_of(s, 'host')}/{uuid}"
        log(f"{uuid[:8]} {state:<12} {raw['age']//60:>3}m  {why or raw.get('tail','')[:60]}")

        # ⛔ ESCALATE ONCE PER EPISODE, NOT ONCE PER TICK.
        # Three rows that had just delivered landing reports were escalated inside
        # one minute, and each would have been escalated again every 4 minutes
        # forever. A finished relay row is SUPPOSED to be idle — that is what
        # finishing looks like — so re-reporting it is pure noise, and a watcher
        # whose output stops being read is the failure this whole plane exists to
        # correct. The latch clears the moment the row moves again.
        st = _ep_load(uuid)
        moved = raw.get("age", 0) < st.get("last_age", 10 ** 9)
        if moved:
            st = {"escalated": None}

        def once(kind, why_text):
            if st.get("escalated") == kind:
                log(f"  (already escalated {kind}; silent until it moves)")
                return
            escalate(a.gui_host, s, row, why_text, a.dry_run)
            st["escalated"] = kind

        if state == "ABANDONED":
            # ⛔ The guarded writer REFUSES on a draft, a choice prompt or an
            # unreadable screen. This used to log "woke" either way, so a refusal
            # and a delivery were indistinguishable in the one record anybody
            # reads afterwards — and the refusals are the interesting half.
            if wake(a.gui_host, row, why, a.dry_run):
                log(f"  ⇒ woke {uuid[:8]} on the PTY")
                st["escalated"] = None
            else:
                log(f"  ⇒ {uuid[:8]} NOT woken — the guarded writer refused; "
                    f"escalating instead of typing")
                once("wake-refused", f"{why} — and a guarded wake was refused, "
                                     f"so this row needs a human or its orchestrator")
        elif state == "CONTEXT_DEAD":
            once("dead", "context exhausted — booting cannot help, it must be RELAYED")
        elif state == "IDLE":
            # ⭐ An IDLE row is most often FINISHED, not stuck. Say so, and ask for
            # the decision that actually applies: more work, or a reap.
            #
            # ⛔⛔ UNLESS IT HAS ARMED A DEFERRAL, WHICH IS A DECLARATION OF INTENT
            # TO CONTINUE. Idleness cannot tell "finished" from "correctly
            # waiting"; a future `boot_after_until` can, and the lane wrote it
            # itself. Reported 2026-08-14 by an orchestrator whose market-session
            # lane — holding a socket through a trading day, deferred 25 minutes
            # by its own hand — was escalated as *"most likely FINISHED … give it
            # more work, relay it, or reap it"*, quoting the lane's own words
            # "staying subscribed" back as evidence that it had stopped.
            #
            # ⚠ That campaign had already paid for this once, from the other
            # side: a lane stood ITSELF down on "am I done?" and left an account
            # unwatched for 7h43m through a market open. They replaced the
            # question with "what is the market doing?" — and this verdict asked
            # the discarded question again from the supervisor's seat, where a
            # supervisor acting on it in good faith would undo their fix.
            # ⇒ A row with no terminal state cannot answer "am I done". Do not
            #   ask it, and do not answer for it.
            until = _deferred_until(uuid)
            if until:
                log(f"  ⏳ idle {raw['age']//60}m but ARMED until "
                    f"{time.strftime('%H:%M', time.localtime(until))} — a deferral is a "
                    f"declaration of intent to continue; NO verdict offered")
            elif raw["age"] >= IDLE_ESCALATE_SECS:
                once("idle", f"idle {raw['age']//60}m — it has most likely FINISHED its scope. "
                             "Read its last prose turn: give it more work, relay it, or reap it")
        elif state == "STUCK":
            once("stuck", why or f"STUCK for {raw['age']//60}m")
        elif state == "NO_TRANSCRIPT":
            once("nobrief", "no transcript — its brief was DROPPED, re-submit it")
        st["last_age"] = raw.get("age", 0)
        _ep_save(uuid, st)
    return 0


def watcher_procs():
    """Every live `watch` process, as (pid, age_seconds, window_seconds_or_None).
    Identify, never count — a bare `pgrep -f` matches the shell that asked the
    question.

    ⭐ The window is read from the process's OWN `--watch` argument rather than
    assumed, because the default has changed and a watcher started by an older
    checkout carries the older window. Asking the process is the only thing that
    survives that skew."""
    out = []
    try:
        ps = subprocess.run(["ps", "-eo", "pid=,etimes=,args="],
                            capture_output=True, text=True, timeout=10).stdout
    except Exception:
        return out
    for line in ps.splitlines():
        parts = line.split(None, 2)
        if len(parts) < 3:
            continue
        pid, etimes, args = parts
        if "ygg-monitor.py" in args and " watch" in args and "bash -c" not in args:
            window = None
            toks = args.split()
            if "--watch" in toks:
                try:
                    window = int(toks[toks.index("--watch") + 1])
                except (ValueError, IndexError):
                    window = None
            try:
                out.append((int(pid), int(etimes), window))
            except ValueError:
                pass
    return out


def cmd_watch(a):
    deadline = time.time() + a.watch
    while time.time() < deadline:
        tick(a)
        time.sleep(a.interval)
    # ⛔⛔ AN EXPIRING SUPERVISOR IS INDISTINGUISHABLE FROM A HEALTHY QUIET ONE.
    # This loop used to just fall off its deadline and exit silently. Measured
    # 2026-08-14: the watcher was started with `--watch 21600` and found at age
    # 5.8h — twelve minutes from ending the only supervision the campaign had,
    # with nothing to restart it and nothing to announce it. Four orchestrators
    # had already stalled unnoticed. Same family as "a reader that finds nothing
    # looks exactly like a thing that has nothing": say it out loud.
    log(f"⛔ WATCHER EXPIRED after {a.watch}s — NOTHING IS WATCHING {len(load_subs())} "
        f"SUBSCRIBER(S) ANY MORE. This is not a clean shutdown, it is a deadline.")
    log("   Restart it, or the next stall escalates to nobody:")
    log(f"     ygg-monitor.py watch --watch 86400 --interval {a.interval}")
    return 0


def main():
    ap = argparse.ArgumentParser(description="supervision plane for relays and orchestrators")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("subscribe")
    # ⛔ `subscribe --uuid X` beside `unsubscribe X` — one tool, two spellings
    #    for one argument. Both verbs now take both. See ygg_rowarg.
    add_row_argument(p)
    p.add_argument("--machine", default="")
    p.add_argument("--role", choices=["orchestrator", "relay", "standalone"], default="relay")
    p.add_argument("--escalate-to", default="", help="orchestrator UUID; empty = escalate to a human")
    p.add_argument("--escalate-host", default="dev")
    p.add_argument("--campaign", default="")
    p.add_argument("--seat", default="")
    p.add_argument("--intent", default="", help="what this row is for, in one line")
    p.add_argument("--no-booter-reminder", action="store_true")
    p.add_argument("--rearm", default="",
                   help="why a row that RELEASED its own subscription is being watched again")
    p.set_defaults(fn=cmd_subscribe)

    pu = sub.add_parser("unsubscribe")
    add_row_argument(pu, required=True)
    pu.add_argument("--note", default="",
                    help="why this row is standing down; kept in the release ledger")
    pu.set_defaults(fn=cmd_unsubscribe)

    for name, fn in (("demote", cmd_demote),
                     ("promote", cmd_promote), ("park", cmd_park), ("unpark", cmd_unpark)):
        p = sub.add_parser(name)
        add_row_argument(p, required=True)
        if name == "demote":
            p.add_argument("--reason", default="")
        if name == "park":
            p.add_argument("--reason", default="", help="why it waits, and on what — required")
            p.add_argument("--hours", type=float, default=2.0, help="expiry, clamped to 24h")
            p.add_argument("--by", default="", help="the orchestrator parking it")
        p.set_defaults(fn=fn)

    p = sub.add_parser("succeed", help="move every subscriber from a retired orchestrator to its successor")
    p.add_argument("--from", dest="from_uuid", required=True)
    p.add_argument("--to", dest="to_uuid", required=True)
    p.add_argument("--escalate-host", default="")
    p.set_defaults(fn=cmd_succeed)

    p = sub.add_parser("normalize", help="expand every SHORT escalate_to to the full uuid it names")
    p.add_argument("--dry-run", action="store_true")
    p.set_defaults(fn=cmd_normalize)

    p = sub.add_parser("list"); p.set_defaults(fn=cmd_list)

    for name, fn in (("tick", tick), ("watch", cmd_watch)):
        p = sub.add_parser(name)
        # ⛔ NO PLACEHOLDER DEFAULT. A name that does not resolve fails at the far
        # end of the call, where the failure arrives looking like data.
        p.add_argument("--gui-host", default=None)
        p.add_argument("--dry-run", action="store_true")
        p.add_argument("--interval", type=int, default=180)
        if name == "watch":
            p.add_argument("--watch", type=int, default=7200)
        p.set_defaults(fn=fn)

    a = ap.parse_args()
    # ⭐ Fold whichever spelling the caller used into the one attribute the
    #    commands read. Verbs that take no row simply have no `_row_dest`.
    if getattr(a, "_row_dest", None):
        try:
            row = resolve_row(a, env_fallback=os.environ.get("YGGTERM_SESSION_ID", "")
                              if a.cmd == "subscribe" else "")
        except ValueError as exc:
            log(f"⛔ {exc}")
            return 64
        if not row and getattr(a, "_row_required", False):
            log(f"{a.cmd}: name a row — `{a.cmd} <uuid>` or `{a.cmd} --row <uuid>`")
            return 64
    return a.fn(a) or 0


if __name__ == "__main__":
    sys.exit(main())
