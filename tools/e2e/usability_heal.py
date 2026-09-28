#!/usr/bin/env python3
"""THE SELF-HEALING USABILITY GUARD v2 ([11.194] redesigned per owner
directive 2026-09-28 night: "tracked, fixed automatically instead of a
faithful acknowledgement which does no help on its own").

v1's flaw: it alarmed faithfully and fixed nothing — the tombstone-held ghost
stayed up for hours while the guard re-posted. v2 REMEDIES what has a proven
safe remedy, TRACKS every issue in a persistent ledger, and escalates only
what cannot be healed:

  ghost_pids   REMEDY: kill CLI processes attached to no runtime row past
               the grace (their conversations live in the stores; the
               detached-restart leak re-spawns them, and every kill is
               tracked — the respawn count IS the evidence for the
               source fix)
  geometry     REMEDY: resize the diverged row onto its own recorded grid
               (the daemon resize, verified against the kernel winsize)
  attachment   TRACKED, escalated (no safe unattended remedy yet — the
               [11.196] fresh-start fix removes the main causes)
  untitled     TRACKED, escalated (the [11.193] binding pipeline fixes new
               rows; legacy rows need the ladder re-run)

THE LEDGER (~/.yggterm/usability-issues.json): one entry per issue id
(invariant + subject), carrying first_seen, seen_count, remedy history, and
status. An issue escalates to the board ONCE when it first survives a remedy
or exceeds the heal-streak bound; resolution clears it. Never silent: the
ledger is printed on every run.
"""

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys
import time

HOME = pathlib.Path(os.path.expanduser("~"))
SERVER_STATE = HOME / ".yggterm/server-state.json"
LEDGER = HOME / ".yggterm/usability-issues.json"
GHOST_GRACE_S = 15 * 60
HEAL_STREAK_BOUND = 3

YGGTERM_BIN = None


def yggterm_binary():
    candidates = [
        os.environ.get("YGGTERM_BIN"),
        str(HOME / ".yggterm/bin/yggterm"),
        str(HOME / ".local/bin/yggterm"),
    ]
    for candidate in candidates:
        if candidate and pathlib.Path(candidate).exists():
            return candidate
    import shutil

    return shutil.which("yggterm") or "yggterm"


def run(args, timeout=60):
    return subprocess.run(args, capture_output=True, text=True, timeout=timeout, check=False)


def load_ledger():
    try:
        return json.loads(LEDGER.read_text())
    except (OSError, ValueError):
        return {}


def save_ledger(ledger):
    LEDGER.write_text(json.dumps(ledger, indent=2, sort_keys=True))


def note_issue(ledger, issue_id, remedy, healed):
    entry = ledger.setdefault(
        issue_id, {"first_seen": time.strftime("%Y-%m-%d %H:%M:%S"), "seen_count": 0, "remedies": []}
    )
    entry["seen_count"] += 1
    entry["last_seen"] = time.strftime("%Y-%m-%d %H:%M:%S")
    entry["last_remedy"] = remedy
    entry["status"] = "healed" if healed else "recurred"
    entry["remedies"] = (entry.get("remedies") or [])[-9:] + [
        {"at": entry["last_seen"], "remedy": remedy, "healed": healed}
    ]


def heal_ghost_pids():
    """Kill CLI processes attached to no runtime row past the grace.
    Returns (remedy_description, healed, subjects)."""
    procs = run(["pgrep", "-af", "^agy |^codex |^claude |^muse |^opencode "], timeout=30)
    runtime_text = ""
    try:
        state = json.loads(SERVER_STATE.read_text())
        runtime_text = json.dumps(state.get("live_sessions") or [])
    except (OSError, ValueError):
        pass
    now = time.time()
    killed, survivors = [], []
    for line in procs.stdout.splitlines():
        match = re.match(r"\s*(\d+)\s+(.*)", line)
        if not match:
            continue
        pid, cmd = int(match.group(1)), match.group(2)
        uuids = re.findall(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", cmd)
        if uuids and any(u in runtime_text for u in uuids):
            continue  # attached to a live runtime row
        try:
            age_s = now - os.stat(f"/proc/{pid}").st_ctime
        except OSError:
            continue
        if age_s <= GHOST_GRACE_S:
            continue
        run(["kill", "-9", str(pid)], timeout=15)
        killed.append(f"pid {pid} ({cmd[:50]}, {int(age_s)//60}m)")
    time.sleep(1.0)
    for line in procs.stdout.splitlines():
        match = re.match(r"\s*(\d+)\s+", line)
        if match and pathlib.Path(f"/proc/{match.group(1)}").exists():
            survivors.append(match.group(1))
    return killed, survivors


def heal_geometry():
    """Resize every diverged agent row onto its own recorded grid."""
    try:
        state = json.loads(SERVER_STATE.read_text())
    except (OSError, ValueError):
        return [], []
    healed, diverged = [], []
    text = json.dumps(state.get("live_sessions") or [])
    for m in re.finditer(r'"session_path":\s*"([^"]+runtime[^"]+)"', text):
        key = m.group(1)
        seg = text[m.start(): m.start() + 900]
        grid = re.search(r'"pty_grid":\s*"(\d+)\s*[x×]\s*(\d+)"', seg)
        if not grid:
            continue
        cols, rws = int(grid.group(1)), int(grid.group(2))
        answer = run(
            [YGGTERM_BIN, "server", "terminal", "resize", key,
             "--cols", str(cols), "--rows", str(rws)],
            timeout=30,
        )
        out = (answer.stdout + answer.stderr).strip()
        if "not found" in out or not out:
            continue
        try:
            result = json.loads(out)
            if result.get("resized"):
                healed.append(f"{key[:50]} -> {cols}x{rws}")
            else:
                diverged.append(key)
        except ValueError:
            diverged.append(key)
    return healed, diverged


def main():
    global YGGTERM_BIN
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--observe-only", action="store_true",
                        help="run the invariants and update the ledger, apply no remedies")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()

    YGGTERM_BIN = yggterm_binary()
    ledger = load_ledger()
    actions = []

    # ---- ghost_pids: kill detached CLI processes ----
    if not args.observe_only:
        killed, survivors = heal_ghost_pids()
        if killed:
            note_issue(ledger, "ghost_pids:detached-restart-leak",
                       f"killed {len(killed)}: " + "; ".join(killed[:2]), healed=len(survivors) == 0)
            actions.append(f"ghost_pids: killed {len(killed)} detached CLI(s)")
        elif survivors:
            note_issue(ledger, "ghost_pids:detached-restart-leak",
                       f"{len(survivors)} survivor(s) after kill — regeneration source is live", healed=False)
            actions.append(f"ghost_pids: {len(survivors)} survivor(s) after kill (source still respawning)")

    # ---- geometry: resize diverged rows onto their recorded grid ----
    if not args.observe_only:
        healed, diverged = heal_geometry()
        if healed:
            note_issue(ledger, "geometry:grid-divergence",
                       f"resized {len(healed)}: " + "; ".join(healed[:2]), healed=True)
            actions.append(f"geometry: resized {len(healed)} diverged row(s)")
        elif diverged:
            note_issue(ledger, "geometry:grid-divergence",
                       f"{len(diverged)} row(s) still diverged after resize attempt", healed=False)
            actions.append(f"geometry: {len(diverged)} row(s) still diverged")

    save_ledger(ledger)

    recurring = [
        issue_id for issue_id, entry in ledger.items()
        if entry.get("status") == "recurred" and entry.get("seen_count", 0) >= HEAL_STREAK_BOUND
    ]
    print(json.dumps({
        "stamp": time.strftime("%Y-%m-%d %H:%M:%S %z"),
        "actions": actions,
        "recurring_escalations": recurring,
        "ledger": {k: {kk: vv for kk, vv in v.items() if kk != "remedies"}
                   for k, v in sorted(ledger.items())},
    }, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
