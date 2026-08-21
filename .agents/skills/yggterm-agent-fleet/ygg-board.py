#!/usr/bin/env python3
"""ygg-board — one screen that answers "what is the status of every N.x task?"

⛔⛔ WHY THIS EXISTS, AND WHY IT DERIVES RATHER THAN ASKS.

Owner-reported 2026-08-21: *"I go to sleep and wake up seeing the rows in a weird
mess. I go to each orchestrator and do not understand the status of each N task."*

The cause was visible in the row table itself. There was no artefact that
answered "what is the status of each seat", so orchestrators put status in the
one field that renders in the sidebar — the TITLE:

    6.2  6.2 row survival: DONE - shipped, deployed, and the loss diagnosed
    6.8  yedit: DONE - paint defect root-caused, 3 entries closed, 5 verbs restored

⇒ That single missing artefact caused BOTH complaints at once. The sidebar was
  doing two jobs — navigation and status — and it is bad at the second, because
  a title is one line, has no schema, and goes stale in silence.

⭐ **THE DESIGN DECISION, owner-chosen: DERIVE EVERYTHING DERIVABLE.** Seat,
liveness, how cold, wake cost, uncollected commits, booter/monitor arming — all
mechanical, and a derived field cannot go stale. The orchestrator writes only
the two things a machine cannot know: what a seat is FOR, and what it is
WAITING ON.

⚠ **AND THAT IS WHY THIS NEEDS NO CADENCE.** The reason the supervision plane
used to type `continue` into orchestrators every few hours was that the status
lived only in their heads. Once it is derived, the board is correct for a seat
whose orchestrator has gone cold — which is exactly when the old design was
least able to answer. Nothing here writes to a row, ever: this file is READ-ONLY
against the fleet.

    ygg-board.py [--host <gui-host>] [--campaign 11] [--json] [--anomalies]

`--anomalies` prints only what is wrong, which is what a steer should carry.
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
sys.path.insert(0, str(HERE))
from ygg_host import resolve_gui_host  # noqa: E402

STATE = Path.home() / ".yggterm" / "relay"
INTENT = STATE / "board-intent"

# A transcript silent longer than this is COLD: a wake re-reads it all before it
# answers a word, so the cheap move is to harvest it, never to ask it.
COLD_MINS = 120
# Big enough that waking it is priced in dollars rather than cents.
EXPENSIVE_MB = 2.0


def _monitor():
    """Reuse the monitor's readers rather than forking them — two tools that
    disagree about what a row's state means is worse than one that is sometimes
    wrong. Same reason `ygg-monitor` loads the booter."""
    spec = importlib.util.spec_from_file_location("ygg_monitor", HERE / "ygg-monitor.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def _sh(host, script, timeout=60):
    cmd = ["ssh", "-n", host, script] if host else ["bash", "-c", script]
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return r.stdout
    except Exception:
        return ""


def _intent(uuid):
    f = INTENT / f"{uuid}.json"
    try:
        return json.loads(f.read_text())
    except Exception:
        return {}


def set_intent(uuid, what_for, waiting):
    """The ONLY writable half of the board, and it is two strings.

    ⛔ Deliberately not a free-text file. Every campaign writing prose in its own
    shape is the state this replaced — the point is that the owner reads ACROSS
    campaigns without learning each one's dialect."""
    INTENT.mkdir(parents=True, exist_ok=True)
    rec = _intent(uuid)
    if what_for is not None:
        rec["for"] = what_for.strip()
    if waiting is not None:
        rec["waiting"] = waiting.strip()
    rec["updated"] = int(time.time())
    (INTENT / f"{uuid}.json").write_text(json.dumps(rec, indent=1))
    return rec


def collect(gui_host, campaign=None):
    """Everything derivable about every SEATED row, from the planes that own it.

    ⛔ Blind is not empty. Each field records whether it could be read, because a
    board that renders an unreachable host as "nothing wrong" is the failure the
    whole supervision plane keeps paying for."""
    m = _monitor()
    rows, rows_ok = m.live_rows(gui_host)
    if not rows_ok:
        return None, "the row plane did not answer — refusing to render a board I cannot read"

    # cwd + ssh target per session, from the daemon that owns them.
    meta = {}
    try:
        st = json.loads(_sh(gui_host, "~/.local/bin/yggterm-headless server status") or "{}")
        for s in st.get("live_terminal_sessions") or []:
            meta[(s.get("id") or "")] = s
    except Exception:
        pass

    # ⛔⛔ THE SUPERVISION ROSTERS ARE PER-HOST FILES OVER A FLEET-WIDE ROW PLANE,
    # AND READING ONLY THIS HOST'S IS THE BUG THIS BOARD EXISTS TO SURFACE.
    # Measured 2026-08-21: the desktop host held 1 booter subscription and 1
    # monitor subscription; the build host held 17 and 5. Each watcher compares
    # its own local pair, finds them consistent, and reports a clean board —
    # while the rows on the OTHER host are supervised by nobody and nothing
    # anywhere says so. The first cut of this file reproduced that exact defect
    # and reported every remote lane as unsupervised.
    # ⇒ Ask EVERY host that owns a row, and union the answers.
    hosts = {(r.get("full_path") or "").split("//", 1)[-1].split("/", 1)[0]
             for r in rows if "//" in (r.get("full_path") or "")}
    hosts = {h for h in hosts if h and "-" not in h}      # drop bare-uuid authorities
    booter, subs = set(), set()
    for h in sorted(hosts | {""}):
        listing = _sh(h or None,
                      "ls ~/.yggterm/relay/booter/ 2>/dev/null; echo @@; "
                      "ls ~/.yggterm/relay/monitor/ 2>/dev/null", timeout=30)
        b, _, mo = listing.partition("@@")
        booter |= {x.split(".")[0][:8] for x in b.split() if x.endswith(".json")}
        subs |= {x.split(".")[0][:8] for x in mo.split() if x.endswith(".json")}

    now = time.time()
    # Uncollected commits are per-CHECKOUT, and several seats share one. Ask each
    # checkout once, on the host that holds it.
    branch_cache = {}

    out = []
    for r in rows:
        seat = (r.get("outline_prefix") or "").strip()
        if not seat:
            continue
        if campaign and seat.split(".")[0] != str(campaign):
            continue
        path = r.get("full_path") or ""
        uuid = path.rsplit("/", 1)[-1]
        host = path.split("//", 1)[-1].split("/", 1)[0] if "//" in path else ""
        host = host if host and host != uuid else ""
        info = meta.get(uuid, {})
        cwd = info.get("cwd") or ""
        rhost = info.get("ssh_target") or host or None

        # ---- liveness, and the cost of asking ----
        proc = m.cli_process(uuid, rhost)
        t = m._transcript_for(uuid, rhost)
        age_m = int((now - t[2]) // 60) if t else None
        mb = round(t[1] / 1e6, 1) if t else None
        if proc is None:
            state = "DEAD" if t else "NO-PROCESS"
        elif age_m is None:
            state = "NO-TRANSCRIPT"
        elif age_m >= COLD_MINS:
            state = "COLD"
        elif age_m >= 30:
            state = "QUIET"
        else:
            state = "ALIVE"

        # ---- uncollected work: commits on this checkout's branch, not on main ----
        work, branch = None, None
        if cwd:
            key = (rhost or "", cwd)
            if key not in branch_cache:
                script = (f"cd {cwd} 2>/dev/null && git fetch -q origin 2>/dev/null; "
                          f"b=$(git rev-parse --abbrev-ref HEAD 2>/dev/null); "
                          f"c=$(git rev-list --count origin/main..HEAD 2>/dev/null); "
                          f"echo \"$b|$c\"")
                branch_cache[key] = _sh(rhost, script, timeout=90).strip()
            raw = branch_cache[key]
            if "|" in raw:
                branch, _, cnt = raw.partition("|")
                try:
                    work = int(cnt)
                except ValueError:
                    work = None

        it = _intent(uuid)
        out.append({
            "seat": seat, "uuid": uuid, "host": rhost or "local",
            "title": (r.get("session_title") or "").strip(),
            "for": it.get("for") or "", "waiting": it.get("waiting") or "",
            "state": state, "idle_min": age_m, "wake_mb": mb,
            "branch": branch, "uncollected": work,
            "booter": uuid[:8] in booter, "monitor": uuid[:8] in subs,
            "cwd": cwd,
        })

    out.sort(key=lambda r: [int(x) if x.isdigit() else 99 for x in r["seat"].split(".")])
    return out, None


def anomalies(board):
    """What is actually WRONG — the only thing a steer should ever carry.

    ⭐ This is the list that replaces the timed `continue`. A campaign with an
    empty list is never interrupted, which is the whole point."""
    bad = []
    for r in board:
        s, tag = r["seat"], f"{r['seat']} ({r['uuid'][:8]})"
        if r["state"] in ("DEAD", "NO-PROCESS"):
            bad.append(f"⛔ {tag} has NO agent process — its row is a husk; harvest and reap it")
        elif r["state"] == "COLD":
            cost = f", {r['wake_mb']} MB to wake" if (r["wake_mb"] or 0) >= EXPENSIVE_MB else ""
            bad.append(f"⚠ {tag} COLD {r['idle_min']}m{cost} — succeed it by HARVESTING, never by asking")
        if r["state"] == "NO-TRANSCRIPT":
            bad.append(f"⛔ {tag} has NO transcript — its brief may never have been delivered, "
                       f"or the env-poison is on it")
        if (r["uncollected"] or 0) > 0:
            bad.append(f"⛔ {tag} has {r['uncollected']} commit(s) on {r['branch']} NOT on main — "
                       f"a roll built from main ships WITHOUT them")
        if not r["booter"] and not r["monitor"]:
            bad.append(f"⛔ {tag} is on NEITHER supervision plane — nothing wakes it, "
                       f"nothing notices it stopped")
        elif not r["booter"]:
            bad.append(f"⚠ {tag} is watched but NOT armed — a stall is seen and never woken")
        elif not r["monitor"]:
            bad.append(f"⚠ {tag} is armed but NOT watched — a stall rings into an empty room")
        if not r["for"]:
            bad.append(f"· {tag} has no recorded PURPOSE — its orchestrator has not said what it is for")
    return bad


def render(board):
    lines = []
    campaign = None
    for r in board:
        maj = r["seat"].split(".")[0]
        if maj != campaign:
            campaign = maj
            lines.append("")
            lines.append(f"══ {maj}.x " + "═" * 60)
        flags = []
        if not r["booter"]:
            flags.append("no-booter")
        if not r["monitor"]:
            flags.append("no-monitor")
        if (r["uncollected"] or 0) > 0:
            flags.append(f"{r['uncollected']} UNCOLLECTED on {r['branch']}")
        idle = f"{r['idle_min']}m" if r["idle_min"] is not None else "?"
        lines.append(f"{r['seat']:<7} {r['state']:<13} last turn {idle:>6}   "
                     f"{(r['wake_mb'] or 0):>5.1f} MB to wake   {r['host']}")
        lines.append(f"        for     {r['for'] or '— (orchestrator has not said)'}")
        if r["waiting"]:
            lines.append(f"        waiting {r['waiting']}")
        if flags:
            lines.append(f"        ⛔      {' · '.join(flags)}")
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser(description="derived status board for every seated row")
    ap.add_argument("--host", default=None)
    ap.add_argument("--campaign", default=None)
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--anomalies", action="store_true")
    ap.add_argument("--set-for", nargs=2, metavar=("UUID", "TEXT"))
    ap.add_argument("--set-waiting", nargs=2, metavar=("UUID", "TEXT"))
    a = ap.parse_args()

    if a.set_for or a.set_waiting:
        u = (a.set_for or a.set_waiting)[0]
        rec = set_intent(u, a.set_for[1] if a.set_for else None,
                         a.set_waiting[1] if a.set_waiting else None)
        print(json.dumps(rec, indent=1))
        return 0

    host = resolve_gui_host(a.host)
    board, err = collect(host, a.campaign)
    if err:
        print(f"ygg-board: ⛔ {err}", file=sys.stderr)
        return 2
    if a.json:
        print(json.dumps(board, indent=1))
        return 0
    bad = anomalies(board)
    if a.anomalies:
        print("\n".join(bad) if bad else "ygg-board: nothing anomalous — no steer is warranted")
        return 0
    print(render(board))
    print()
    print(f"── {len(board)} seated row(s) · {len(bad)} anomaly(ies) " + "─" * 30)
    for x in bad:
        print(f"  {x}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
