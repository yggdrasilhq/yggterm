"""Resolve the GUI host, and say so out loud when it cannot be resolved.

⛔⛔ WHY THIS FILE EXISTS: A PLACEHOLDER DEFAULT TURNED THREE SUPERVISORS BLIND,
AND BLINDNESS WAS RENDERED AS DEATH.

Measured 2026-08-13, from two campaigns independently within the same hour. All
three fleet tools defaulted their GUI host to a name that does not resolve.
App control only answers on the host where the GUI runs, so every call failed at
the transport — and every caller treated the resulting empty answer as a fact
about the rows:

    ygg-babysit  →  live, subscribed, working rows reported `GONE / RETIRED`
    ygg-monitor  →  every subscriber reported `SUBSCRIBED BUT NO ROW`,
                    including the orchestrator's own row, which was live

⇒ **`GONE` is not "I could not see". It is a positive claim that the row is
dead, and it is TERMINAL — a supervisor that reaches it stops supervising.** A
watchdog that cannot tell *I am blind* from *it is gone*, and resolves that
ambiguity toward standing down, is worse than no watchdog: it reports success
while abandoning the fleet. One campaign was a single step from treating a whole
wave as finished on the strength of it; two of those rows were idle with open
obligations.

⚠ **It was intermittent**, which is worse than consistent — the same command from
the same host answered correctly three times over the preceding hour and then
flipped. An intermittent blindness gets attributed to the subject rather than to
the instrument.

⭐ **THE TWO RULES THIS ENCODES, and they are separable:**

1. **Never let an unresolvable default stand in for a real one.** A default that
   is a placeholder fails at the far end of the call, where the failure looks
   like data. Resolve it, or refuse to run — never guess.
2. **A blind instrument must say it is blind.** Callers here get an explicit
   `ok=False`, and every conclusion of the form *"this row does not exist"* is
   required to hold positive evidence that the row plane actually answered.

⇒ The same shape has now been sighted five times in one afternoon across three
campaigns: an instrument answers about the wrong subject and says nothing about
having done so. **The fix shape is always the same — make the instrument state
its subject in its own output.**
"""

import json
import os
import subprocess
from pathlib import Path

_CACHE = {}


def _probe(host, timeout=25):
    """Does app control actually ANSWER on this host? Not "is it pingable"."""
    try:
        r = subprocess.run(
            ["ssh", "-n", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", host,
             "~/.yggterm/bin/yggterm server app rows"],
            capture_output=True, text=True, timeout=timeout)
    except Exception:
        return False
    out = r.stdout
    if "{" not in out:
        return False
    try:
        json.loads(out[out.find("{"):])
    except Exception:
        # A reply we cannot parse is still a reply from a live app-control
        # endpoint; the parse defect is a separate, filed problem.
        return r.returncode == 0
    return True


def _candidates_from_local_daemon():
    """The local daemon names the hosts it knows; ask it rather than guessing.

    This is the same source `ygg-claim.sh` uses, and it carries no hostname in
    this file — which is the point, since this repo is public."""
    try:
        r = subprocess.run(["yggterm-headless", "server", "app", "rows"],
                           capture_output=True, text=True, timeout=30)
    except Exception:
        return []
    blob = (r.stdout or "") + (r.stderr or "")
    key = "candidates this daemon knows:"
    if key not in blob:
        return []
    tail = blob.split(key, 1)[1].splitlines()[0]
    return [c.strip() for c in tail.replace(",", " ").split() if c.strip()]


def resolve_gui_host(explicit=None, verbose=True):
    """The GUI host, or None. ⛔ None means REFUSE TO CONCLUDE, never "empty".

    Order: an explicit flag · $YGG_GUI_HOST · the repo's own resolver
    (`scripts/ygg-live-host.sh`, the single owner of this question) · the local
    daemon's candidate list, each probed until one answers.

    ⚠ Every source but the first is verified by an actual app-control call. A
    name that resolves in DNS but has no GUI behind it is exactly the failure
    this function exists to stop, so "it is configured" is not accepted as
    "it answers"."""
    if explicit:
        return explicit
    if "resolved" in _CACHE:
        return _CACHE["resolved"]

    env = os.environ.get("YGG_GUI_HOST")
    tried = []
    if env:
        if _probe(env):
            _CACHE["resolved"] = env
            return env
        tried.append(f"$YGG_GUI_HOST={env} (did not answer)")

    root = Path(__file__).resolve().parents[3]
    script = root / "scripts" / "ygg-live-host.sh"
    if script.exists():
        try:
            r = subprocess.run([str(script)], capture_output=True, text=True, timeout=45)
            name = (r.stdout or "").strip().splitlines()[-1].strip() if r.stdout.strip() else ""
        except Exception:
            name = ""
        if name and _probe(name):
            _CACHE["resolved"] = name
            return name
        if name:
            tried.append(f"ygg-live-host.sh -> {name} (did not answer)")

    for c in _candidates_from_local_daemon():
        if _probe(c):
            _CACHE["resolved"] = c
            return c
        tried.append(f"daemon candidate {c} (did not answer)")

    if verbose:
        print("ygg-host: ⛔ COULD NOT RESOLVE A GUI HOST — app control is UNREACHABLE "
              "from here. Every row-existence check is BLIND until this is fixed; "
              "nothing below may be read as evidence that a row is gone. "
              "Pass --host/--gui-host or set $YGG_GUI_HOST. "
              f"Tried: {'; '.join(tried) or 'nothing (no sources available)'}",
              flush=True)
    _CACHE["resolved"] = None
    return None
