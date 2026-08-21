"""Shared plumbing for the yggterm profiling notebooks.

Standard library only, deliberately. These notebooks run on whichever fleet host
happens to be free, over ssh, against a GUI machine whose resource priority is
memory first — installing a scientific stack to compute a median would be the
wrong trade, and a notebook that needs one cannot be executed from a shell
script on a headless box. Percentiles come from ``statistics``, timelines are
unicode sparklines, and the output is text an agent or an LLM can read as-is.

Two rules this module exists to enforce, both from ``docs/observability.md``:

* **Never hand-resolve a ytrace home.** ``ytrace``'s own resolution prefers the
  yggterm home when it exists, so the XDG path is usually a stale orphan holding
  well-formed, parseable, day-old records. Everything here goes through the CLI.
* **Never compare two clocks.** ``duration_ms`` is wall for every category but
  ``render``, where it is CPU-ms consumed over ``interval_ms``. Summing the two
  into one ranking produces a number with no unit.
"""

from __future__ import annotations

import json
import os
import shutil
import statistics
import subprocess
from collections import Counter, defaultdict
from datetime import datetime, timezone

# ── hosts ────────────────────────────────────────────────────────────────────

LOCAL = "local"

# Hosts are configuration, never a literal in a tracked file. Set
# YGG_NOTEBOOK_HOSTS to a comma-separated list of ssh aliases; "local" means
# this machine. The GUI host is named separately because the render, input and
# attach probes only exist where a GUI runs.
HOSTS = [h.strip() for h in os.environ.get("YGG_NOTEBOOK_HOSTS", LOCAL).split(",") if h.strip()]
GUI_HOST = os.environ.get("YGG_NOTEBOOK_GUI_HOST", HOSTS[0] if HOSTS else LOCAL)

# ssh runs a non-login shell, which does not read the profile that puts
# ~/.local/bin on PATH. Without this prefix every remote verb returns exit 127
# in about a millisecond, which reads exactly like a fast, empty, healthy
# answer. Blind is not broken: an instrument that cannot run is a verdict about
# the instrument, not about the host.
_REMOTE_PATH_PREFIX = 'export PATH="$HOME/.local/bin:$PATH"; '


class ProbeError(RuntimeError):
    """A verb could not be run at all — distinct from running and finding nothing."""


def run_on(host: str, argv: list[str], timeout: int = 120) -> str:
    """Run a command on ``host`` and return stdout, raising if it could not run."""
    if host == LOCAL:
        if shutil.which(argv[0]) is None:
            raise ProbeError(f"{argv[0]} is not on PATH on the local host")
        proc = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
    else:
        remote = _REMOTE_PATH_PREFIX + " ".join(_quote(a) for a in argv)
        proc = subprocess.run(
            ["ssh", "-o", "BatchMode=yes", host, remote],
            capture_output=True, text=True, timeout=timeout,
        )
    if proc.returncode != 0:
        raise ProbeError(
            f"{host}: {' '.join(argv)} exited {proc.returncode}: {proc.stderr.strip()[:400]}"
        )
    return proc.stdout


def _quote(arg: str) -> str:
    if arg and all(c.isalnum() or c in "-_=./:" for c in arg):
        return arg
    return "'" + arg.replace("'", "'\\''") + "'"


# ── ytrace verbs ─────────────────────────────────────────────────────────────

def ytrace(host: str, verb: str, **flags) -> list[dict]:
    """Call one ``ytrace`` verb and return parsed JSON rows.

    Returns ``[]`` when the verb ran and found nothing. Raises ``ProbeError``
    when it could not run — the caller must be able to tell those apart.
    """
    argv = ["ytrace", verb]
    for key, value in flags.items():
        if value is None or value is False:
            continue
        argv.append("--" + key.replace("_", "-"))
        if value is not True:
            argv.append(str(value))
    argv.append("--json")
    out = run_on(host, argv).strip()
    if not out:
        return []
    try:
        parsed = json.loads(out)
    except json.JSONDecodeError as exc:
        raise ProbeError(f"{host}: {verb} returned unparsable output: {exc}") from exc
    return parsed if isinstance(parsed, list) else [parsed]


def query(host: str, since: str = "30m", **flags) -> list[dict]:
    """Ranked probe summary. Rows carry ``clock``; see :func:`split_by_clock`."""
    return ytrace(host, "query", since=since, **flags)


# `ytrace tail` applies `--lines` with a DEFAULT OF 20 even when `--since` is
# given, so `--since 1h` silently returns the last twenty records rather than an
# hour of them. The flag you set is overridden by one you did not, no warning is
# printed, and the truncated result is perfectly well-formed — it just describes
# the last few seconds while claiming to describe an hour. Every rate, timeline
# and percentile computed from it would be wrong in the same invisible way.
TAIL_LINES_DEFAULT = 100_000


def tail(host: str, since: str = "30m", lines: int | None = None, **flags) -> list[dict]:
    """Raw records, newest last. The only verb that exposes per-record payloads.

    ``lines`` defaults high on purpose: see ``TAIL_LINES_DEFAULT`` above. Pass an
    explicit small value only when you genuinely want the last N records rather
    than a window.
    """
    return ytrace(host, "tail", since=since,
                  lines=TAIL_LINES_DEFAULT if lines is None else lines, **flags)


def incidents(host: str, since: str = "24h") -> list[dict]:
    return ytrace(host, "incidents", since=since)


def health(host: str, since: str = "1h") -> list[dict]:
    return ytrace(host, "health", since=since)


def probe_key(record: dict) -> str:
    return f"{record.get('category')}/{record.get('name')}"


def split_by_clock(rows: list[dict]) -> dict[str, list[dict]]:
    """Partition summary rows by clock, because they must never be ranked together.

    A ``render`` row is CPU-ms consumed over an interval; every other row is a
    wall-clock latency. Sorting both by ``total_ms`` orders two different
    quantities against each other and calls the largest one the hottest.
    """
    out: dict[str, list[dict]] = defaultdict(list)
    for row in rows:
        out[row.get("clock", "wall")].append(row)
    return dict(out)


def split_by_version(rows: list[dict]) -> dict[str, list[dict]]:
    """Partition records by the EMITTER's ``app_version``, because a fleet
    mid-roll writes several builds into one stream.

    ⛔ A window chosen by wall clock reports whatever mix of versions it happens
    to span and attributes it to the build running now. That is how a FIXED
    class reads as a live one, and how a stale p95 reads as current. Measured
    2026-08-21, all three in one session: one class read 0.81/min across a
    straddling window and 0.00/min once split by emitter; a snapshot handler's
    p95 read 10,110 ms across a roll and 13.2 ms on the build that fixed it; a
    deploy's own remote install read as a steady-state stall.

    ⚠ The retiring daemon keeps emitting for SECONDS after its successor binds —
    that is the version-coexistence the daemon/GUI split is built on — so a
    record timestamped after a handover may still carry the old version. Trust
    this field, not the clock.
    """
    out: dict[str, list[dict]] = defaultdict(list)
    for row in rows:
        out[row.get("app_version") or "unknown"].append(row)
    return dict(out)


def version_spans(records: list[dict]) -> dict[str, tuple[int, int, float]]:
    """Per version: ``(first_ts_ms, last_ts_ms, minutes)``.

    ⛔ A rate must be divided by the bucket's OWN span, never by the window you
    asked for. A version present for two minutes of a six-hour window otherwise
    reports a rate three orders of magnitude too low — or, if it is the newest
    build and barely represented, a reassuring near-zero that means only that it
    has not been running long.

    Returns nothing for a version with fewer than two timestamped records: a
    span needs two points, and inventing one is how a single record becomes an
    infinite rate.
    """
    out: dict[str, tuple[int, int, float]] = {}
    for version, rows in split_by_version(records).items():
        stamps = sorted(r["ts_ms"] for r in rows if isinstance(r.get("ts_ms"), (int, float)))
        if len(stamps) < 2:
            continue
        out[version] = (stamps[0], stamps[-1], (stamps[-1] - stamps[0]) / 60_000.0)
    return out


def core_fraction(record: dict) -> float | None:
    """Cores burned, for a cpu-clock span. ``None`` when the interval is missing.

    Records written before the mirror carried the whole payload have no
    ``interval_ms`` and are genuinely uninterpretable — returning ``None`` keeps
    them out of an average instead of silently scoring them zero.
    """
    payload = record.get("payload") or {}
    if not isinstance(payload, dict):
        return None
    if isinstance(payload.get("core_fraction"), (int, float)):
        return float(payload["core_fraction"])
    interval = payload.get("interval_ms")
    duration = record.get("duration_ms")
    if isinstance(interval, (int, float)) and interval > 0 and isinstance(duration, (int, float)):
        return duration / interval
    return None


# ── statistics ───────────────────────────────────────────────────────────────

def percentiles(values: list[float]) -> dict:
    """p50/p95/p99 plus n, max and mean. Empty in, empty out — never a zero."""
    clean = sorted(v for v in values if isinstance(v, (int, float)))
    if not clean:
        return {"n": 0}
    def at(p: float) -> float:
        idx = min(len(clean) - 1, int(round(p * (len(clean) - 1))))
        return clean[idx]
    return {
        "n": len(clean),
        "p50": at(0.50),
        "p95": at(0.95),
        "p99": at(0.99),
        "max": clean[-1],
        "mean": statistics.fmean(clean),
    }


def bucket_by_time(records: list[dict], bucket_ms: int) -> dict[int, list[dict]]:
    """Group records into fixed time buckets keyed by bucket start (epoch ms)."""
    out: dict[int, list[dict]] = defaultdict(list)
    for record in records:
        ts = record.get("ts_ms")
        if isinstance(ts, (int, float)):
            out[int(ts) // bucket_ms * bucket_ms].append(record)
    return dict(out)


def rate_series(records: list[dict], bucket_ms: int) -> list[tuple[int, float]]:
    """Per-second rate in each bucket, as ``(bucket_start_ms, per_second)``.

    Buckets with no records are filled in as zero: a storm is visible only
    against the calm around it, and dropping empty buckets hides the onset.
    """
    buckets = bucket_by_time(records, bucket_ms)
    if not buckets:
        return []
    lo, hi = min(buckets), max(buckets)
    return [
        (b, len(buckets.get(b, [])) / (bucket_ms / 1000.0))
        for b in range(lo, hi + bucket_ms, bucket_ms)
    ]


# ── rendering ────────────────────────────────────────────────────────────────

_BLOCKS = " ▁▂▃▄▅▆▇█"


def sparkline(values: list[float]) -> str:
    """A unicode sparkline. Scaled to the series' own max, which is stated by the caller."""
    nums = [v for v in values if isinstance(v, (int, float))]
    if not nums:
        return "(no data)"
    lo, hi = min(nums), max(nums)
    if hi <= lo:
        return _BLOCKS[1] * len(nums)
    span = hi - lo
    return "".join(_BLOCKS[min(8, int((v - lo) / span * 8) + 1)] for v in nums)


def ts(ms: int | float) -> str:
    return datetime.fromtimestamp(ms / 1000, timezone.utc).strftime("%H:%M:%S")


def table(rows: list[dict], columns: list[str], limit: int = 20) -> str:
    """Fixed-width table. Text, so the output survives being read by an agent."""
    if not rows:
        return "(no rows)"
    widths = {c: max(len(c), *(len(_cell(r.get(c))) for r in rows[:limit])) for c in columns}
    head = "  ".join(c.ljust(widths[c]) for c in columns)
    rule = "  ".join("-" * widths[c] for c in columns)
    body = [
        "  ".join(_cell(r.get(c)).ljust(widths[c]) for c in columns)
        for r in rows[:limit]
    ]
    return "\n".join([head, rule, *body])


def _cell(value) -> str:
    if value is None:
        return "-"
    if isinstance(value, float):
        return f"{value:.2f}"
    return str(value)


# ── verdicts ─────────────────────────────────────────────────────────────────

PASS, WARN, FAIL, UNKNOWN = "PASS", "WARN", "FAIL", "INSUFFICIENT DATA"

_MARK = {PASS: "🟢", WARN: "🟡", FAIL: "🔴", UNKNOWN: "⚪"}


class Verdict:
    """One notebook's conclusion: findings with an explicit threshold each.

    Every notebook ends with one of these. The point is that the reader — the
    owner, or the interface LLM reading the notebook's output — gets a decision
    and the number that drove it, rather than a chart to interpret.

    ``UNKNOWN`` is a first-class outcome and never collapses into ``PASS``. A
    probe that never fired and a probe that fired healthily produce the same
    empty result set, and calling that green is how an instrument gap becomes an
    all-clear.
    """

    def __init__(self, title: str):
        self.title = title
        self.findings: list[tuple[str, str, str]] = []

    def check(self, name: str, observed, threshold, *, fail_over=None, warn_over=None,
              detail: str = "", n: int | None = None, min_n: int = 20) -> str:
        """Grade one observation against its thresholds.

        ``n``/``min_n`` exist because a tail percentile computed from a handful
        of samples is not a tail percentile — it is the maximum wearing one's
        name. Below ``min_n`` a breach is reported one level softer and says so,
        so that a single slow sample cannot turn a quiet window red. That is not
        leniency: an over-confident red is the failure that makes the next
        reader stop trusting the whole verdict.
        """
        low_confidence = n is not None and n < min_n
        if observed is None:
            state = UNKNOWN
        elif fail_over is not None and observed > fail_over:
            state = FAIL
        elif warn_over is not None and observed > warn_over:
            state = WARN
        else:
            state = PASS
        if low_confidence and state in (FAIL, WARN):
            state = WARN if state == FAIL else PASS
            detail = (detail + " · " if detail else "") + \
                     f"LOW CONFIDENCE: n={n} < {min_n}, so this tail is really the maximum"
        obs = "-" if observed is None else (f"{observed:.2f}" if isinstance(observed, float) else str(observed))
        self.findings.append((state, name, f"observed {obs} · threshold {threshold}"
                                           + (f" · {detail}" if detail else "")))
        return state

    def note(self, state: str, name: str, detail: str) -> None:
        self.findings.append((state, name, detail))

    @property
    def worst(self) -> str:
        for state in (FAIL, WARN, UNKNOWN, PASS):
            if any(f[0] == state for f in self.findings):
                return state
        return UNKNOWN

    def render(self) -> str:
        lines = [f"VERDICT · {self.title}", "=" * (10 + len(self.title))]
        for state, name, detail in self.findings:
            lines.append(f"{_MARK[state]} {state:18} {name}")
            lines.append(f"{'':21}{detail}")
        lines.append("")
        lines.append(f"{_MARK[self.worst]} OVERALL: {self.worst}")
        return "\n".join(lines)

    def show(self) -> None:
        print(self.render())


def describe_source(host: str, window: str) -> None:
    """Print what was read, so a verdict is never separated from its provenance."""
    print(f"host={host}  window={window}  read_at={datetime.now(timezone.utc).isoformat(timespec='seconds')}")

def window_minutes(window: str) -> float | None:
    """Length of a ytrace `--since` window in minutes.

    A rate must be divided by the window it was measured over, never by the
    span between the first and last event: a burst of six blocks four seconds
    apart spans 0.06 minutes, and dividing by that reports 93 per minute for a
    window that contained six.
    """
    text = window.strip().lower()
    for suffix, minutes in (("ms", 1 / 60000), ("s", 1 / 60), ("m", 1.0), ("h", 60.0), ("d", 1440.0)):
        if text.endswith(suffix):
            try:
                return float(text[: -len(suffix)]) * minutes
            except ValueError:
                return None
    try:
        return float(text) / 60000  # bare number is raw ms, as ytrace reads it
    except ValueError:
        return None
