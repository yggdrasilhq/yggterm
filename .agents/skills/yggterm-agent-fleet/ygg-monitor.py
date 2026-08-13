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
import importlib.util
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
STATE = Path.home() / ".yggterm" / "relay"
SUBS = STATE / "monitor"
LOGPATH = STATE / "monitor.log"

# Mid-turn and silent for longer than this, at rest, is ABANDONED not thinking.
ABANDONED_SECS = 600
# CPU% at or below this over the sample window counts as "not thinking".
IDLE_CPU_PCT = 2.0
CPU_SAMPLE_SECS = 3


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


def ygg(host, *args):
    cmd = ["ssh", host, "~/.local/bin/yggterm-headless " + " ".join(
        f"'{a}'" if " " in str(a) else str(a) for a in args)]
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=90)
        out = r.stdout
        return json.loads(out[out.find("{"):]) if "{" in out else {}
    except Exception:
        return {}


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
    """PTY first, and the Enter is a SEPARATE write of \\r.

    ⛔ Not `submit` — it drives the GUI's mounted terminal host and stalls 30 s
    answering submitted:false for any row with nothing mounted.
    ⛔ Not `text + "\\r"` in one write — an agent CLI reads that as a pasted
    newline (composer content), not a submit. Text, pause, then a lone CR."""
    msg = ("ORCHESTRATOR/MONITOR — continue. Your turn was cut off mid-flight "
           "(likely a restart re-resuming your session on a fresh PTY); your process "
           "and your work are intact. Check git status/log on your tree to see what "
           "landed, then carry on from where your last message stopped.")
    if dry:
        log(f"  DRY would wake {row}: {why}")
        return True
    ygg(host, "server", "app", "terminal", "send", row, "--data", msg)
    time.sleep(0.2)
    subprocess.run(["ssh", host,
                    f"~/.local/bin/yggterm-headless server app terminal send '{row}' --data $'\\r'"],
                   capture_output=True, text=True, timeout=60)
    return True


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
    if to:
        target = f"remote-cc://{sub.get('escalate_host', sub.get('host', 'dev'))}/{to}"
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
        ygg(host, "server", "app", "notify", "relay needs a human", why,
            "--tone", "warning", "--session", row)
        log("  escalated to a human card (no orchestrator subscribed)")


# ---------------------------------------------------------------------------
# Verbs
# ---------------------------------------------------------------------------
def cmd_subscribe(a):
    uuid = a.uuid or os.environ.get("YGGTERM_SESSION_ID", "")
    if not uuid:
        log("subscribe: need --uuid (or $YGGTERM_SESSION_ID)")
        return 64
    rec = {"uuid": uuid, "host": a.machine, "role": a.role,
           "escalate_to": a.escalate_to, "escalate_host": a.escalate_host,
           "campaign": a.campaign, "seat": a.seat,
           "owner_pinned": False, "booter": True,
           "intent": a.intent, "since": int(time.time())}
    sub_path(uuid).write_text(json.dumps(rec, indent=1))
    log(f"subscribed {uuid[:8]} as {a.role}"
        + (f", escalating to {a.escalate_to[:8]}" if a.escalate_to else ", escalating to a human"))
    if a.role == "orchestrator" and not a.no_booter_reminder:
        log("⛔ AN ORCHESTRATOR MUST ALSO SUBSCRIBE TO THE BOOTER — it is the net that")
        log("   catches this plane itself. Run: ygg-booter.py subscribe")
    return 0


def cmd_unsubscribe(a):
    p = sub_path(a.uuid)
    if p.exists():
        p.unlink()
        log(f"unsubscribed {a.uuid[:8]}")
    else:
        log(f"{a.uuid[:8]} was not subscribed — nothing to do")
    return 0


def cmd_demote(a):
    """The owner takes a row back. Nothing automated touches it again."""
    p = sub_path(a.uuid)
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
    p = sub_path(a.uuid)
    if not p.exists():
        log(f"{a.uuid[:8]} is not subscribed")
        return 1
    s = json.loads(p.read_text())
    s["owner_pinned"] = False
    s.pop("pinned_reason", None)
    p.write_text(json.dumps(s, indent=1))
    log(f"{a.uuid[:8]} promoted back under supervision")
    return 0


def cmd_list(a):
    subs = load_subs()
    if not subs:
        log("no subscribers")
        return 0
    for s in subs:
        pin = "  ⭐ OWNER-PINNED" if s.get("owner_pinned") else ""
        log(f"{s['uuid'][:8]}  {s.get('role','relay'):<13} seat={str(s.get('seat') or '-'):<5} "
            f"→{(s.get('escalate_to') or 'human')[:8]}  {(s.get('intent') or '')[:44]}{pin}")
    return 0


def tick(a):
    bs = _babysit()
    for s in load_subs():
        uuid = s["uuid"]
        if s.get("owner_pinned"):
            log(f"{uuid[:8]} SKIP — owner-pinned ({s.get('pinned_reason','')})")
            continue
        rhost = None if s.get("host") in ("", None, "local") else s.get("host")
        raw = bs.classify(uuid, rhost)
        state, why = refine(raw, uuid, rhost)
        row = bs.resolve_row_path(a.gui_host, uuid) or f"remote-cc://{s.get('host','dev')}/{uuid}"
        log(f"{uuid[:8]} {state:<12} {raw['age']//60:>3}m  {why or raw.get('tail','')[:60]}")

        if state == "ABANDONED":
            wake(a.gui_host, row, why, a.dry_run)
            log(f"  ⇒ woke {uuid[:8]} on the PTY")
        elif state == "CONTEXT_DEAD":
            escalate(a.gui_host, s, row, "context exhausted — booting cannot help, it must be RELAYED", a.dry_run)
        elif state in ("IDLE", "STUCK"):
            escalate(a.gui_host, s, row, why or f"{state} for {raw['age']//60}m", a.dry_run)
        elif state == "NO_TRANSCRIPT":
            escalate(a.gui_host, s, row, "no transcript — its brief was DROPPED, re-submit it", a.dry_run)
    return 0


def cmd_watch(a):
    deadline = time.time() + a.watch
    while time.time() < deadline:
        tick(a)
        time.sleep(a.interval)
    return 0


def main():
    ap = argparse.ArgumentParser(description="supervision plane for relays and orchestrators")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("subscribe")
    p.add_argument("--uuid", default="")
    p.add_argument("--machine", default="")
    p.add_argument("--role", choices=["orchestrator", "relay", "standalone"], default="relay")
    p.add_argument("--escalate-to", default="", help="orchestrator UUID; empty = escalate to a human")
    p.add_argument("--escalate-host", default="dev")
    p.add_argument("--campaign", default="")
    p.add_argument("--seat", default="")
    p.add_argument("--intent", default="", help="what this row is for, in one line")
    p.add_argument("--no-booter-reminder", action="store_true")
    p.set_defaults(fn=cmd_subscribe)

    for name, fn in (("unsubscribe", cmd_unsubscribe), ("demote", cmd_demote), ("promote", cmd_promote)):
        p = sub.add_parser(name)
        p.add_argument("uuid")
        if name == "demote":
            p.add_argument("--reason", default="")
        p.set_defaults(fn=fn)

    p = sub.add_parser("list"); p.set_defaults(fn=cmd_list)

    for name, fn in (("tick", tick), ("watch", cmd_watch)):
        p = sub.add_parser(name)
        p.add_argument("--gui-host", default=os.environ.get("YGG_GUI_HOST", "guihost"))
        p.add_argument("--dry-run", action="store_true")
        p.add_argument("--interval", type=int, default=180)
        if name == "watch":
            p.add_argument("--watch", type=int, default=7200)
        p.set_defaults(fn=fn)

    a = ap.parse_args()
    return a.fn(a) or 0


if __name__ == "__main__":
    sys.exit(main())
