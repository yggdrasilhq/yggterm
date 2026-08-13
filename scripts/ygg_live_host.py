"""The python doorway to `scripts/ygg-live-host.sh`.

⛔ IT IS A DOORWAY, NOT A SECOND SOURCE. The shell script owns the whole
resolution order (operator override → this machine → cached alias verified by a
probe → parallel discovery → cached alias unverified); this module only calls it
and turns a failure into something a python caller can act on. Anything here
that started deciding for itself would be the second encoding the resolver was
written to delete.

⛔ AND IT EXISTS BECAUSE A HARDCODED HOST NAME IS INVISIBLE UNTIL IT RUNS. A
literal placeholder in a default host list does not resolve, and every tool that
carried one degraded the same way: it did its job on the hosts it could reach,
reported a per-host error nobody aggregates, and silently omitted the machine
that runs the GUI — which on a fleet audit is the host with the most to say.
Measured 2026-08-13: one such placeholder blinded a fleet deploy, three fleet
supervisors, and a daemon audit whose default invocation skipped the busiest
host on the fleet while exiting 0.

⚠ Never call this from an argparse default. Defaults are evaluated even for
`--help`, and this makes ssh probes.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

_RESOLVER = Path(__file__).resolve().parent / "ygg-live-host.sh"


def resolve(timeout: float = 90.0, quiet: bool = False) -> str | None:
    """The ssh alias of the host running the live GUI, or `None`.

    Diagnostics from the resolver are passed through to stderr, because the
    reason it could not answer is the whole content of the failure.
    """
    try:
        done = subprocess.run(
            [str(_RESOLVER)] + (["--quiet"] if quiet else []),
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )
    except (OSError, subprocess.SubprocessError) as error:
        print(f"ygg_live_host: could not run {_RESOLVER}: {error}", file=sys.stderr)
        return None
    if done.stderr:
        sys.stderr.write(done.stderr)
    if done.returncode != 0:
        return None
    return done.stdout.strip() or None


def hosts_with_live(*others: str, timeout: float = 90.0) -> list[str]:
    """A host list with the resolved GUI host in it, deduplicated in order.

    ⛔ REFUSES RATHER THAN RETURNING THE SHORT LIST. A tool that quietly drops
    the GUI host still produces a confident-looking report about the rest of the
    fleet, and that report is read as complete. Raising here is the difference
    between "two hosts audited" and "two hosts audited, one silently missing".
    """
    live = resolve(timeout=timeout)
    if not live:
        raise SystemExit(
            "ygg_live_host: could not resolve the live GUI host, and continuing "
            "would report on the rest of the fleet as though it were all of it. "
            "Set $YGG_GUI_HOST, or pass the hosts explicitly."
        )
    ordered: list[str] = []
    for host in (*others, live):
        if host and host not in ordered:
            ordered.append(host)
    return ordered
