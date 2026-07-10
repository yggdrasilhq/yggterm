#!/usr/bin/env python3
"""Summarize client-detected render FAIL PATTERNS from the yggterm event trace.

The GUI's xterm host emits a `render_fail_pattern` trace event whenever it
detects an anomalous rendering pattern (e.g. a `redraw_burst` — repeated
repaints with no session change/reveal behind them, i.e. the "N quick blinks"
symptom). Each event carries the anomaly JSON plus context (render-health
status/reason, recovery count, rows, cursor, visible non-blank rows).

This lets the recurring rendering nuances surface in the trace instead of the
user having to report each one. Run it, then read the grouped patterns +
examples to decide which reconcile/recovery logic needs an edge-case patch.

Usage:
    scripts/render_fail_patterns.py [~/.yggterm/event-trace.jsonl ...]
    ssh <host> 'cat ~/.yggterm/event-trace.previous.jsonl ~/.yggterm/event-trace.jsonl' | scripts/render_fail_patterns.py -
"""
import json
import sys
import collections
from pathlib import Path


def iter_lines(paths):
    if paths == ["-"]:
        for line in sys.stdin:
            yield line
        return
    if not paths:
        home = Path.home() / ".yggterm"
        paths = sorted(str(p) for p in home.glob("event-trace.g*.jsonl"))
        paths += [str(home / "event-trace.previous.jsonl"), str(home / "event-trace.jsonl")]
    for p in paths:
        try:
            with open(p, "r", errors="replace") as fh:
                yield from fh
        except OSError:
            continue


def main():
    paths = sys.argv[1:]
    events = []
    for line in iter_lines(paths):
        try:
            d = json.loads(line)
        except Exception:
            continue
        if d.get("name") == "detected" and d.get("category") == "render_fail_pattern":
            events.append(d)

    if not events:
        print("no render_fail_pattern events found")
        return

    by_pattern = collections.Counter()
    reason_counts = collections.Counter()
    per_session = collections.Counter()
    for d in events:
        p = d.get("payload", {})
        anomaly = p.get("anomaly", {})
        if isinstance(anomaly, str):
            try:
                anomaly = json.loads(anomaly)
            except Exception:
                anomaly = {"pattern": anomaly}
        pat = anomaly.get("pattern", "?")
        by_pattern[pat] += 1
        per_session[str(p.get("session_path", "?"))[-14:]] += 1
        for r in anomaly.get("reasons", []) or []:
            reason_counts[r] += 1

    print(f"== render_fail_pattern events: {len(events)} ==\n")
    print("by pattern:")
    for k, v in by_pattern.most_common():
        print(f"  {v:5}  {k}")
    print("\ntop redraw reasons in bursts:")
    for k, v in reason_counts.most_common(12):
        print(f"  {v:5}  {k}")
    print("\nby session (tail):")
    for k, v in per_session.most_common(10):
        print(f"  {v:5}  {k}")

    print("\n== recent examples (last 8) ==")
    for d in events[-8:]:
        p = d.get("payload", {})
        anomaly = p.get("anomaly", {})
        if isinstance(anomaly, str):
            try:
                anomaly = json.loads(anomaly)
            except Exception:
                pass
        extra = ""
        if isinstance(anomaly, dict) and anomaly.get("pattern") == "stale_atlas_paint":
            extra = (
                f" raf_gap_ms={anomaly.get('raf_gap_ms')} atlas_age_ms={anomaly.get('atlas_age_ms')}"
                f" heals={anomaly.get('heal_count')} focused={anomaly.get('window_focused')}"
                f" vis={anomaly.get('visibility')}"
            )
        print(
            f"  ts={d.get('ts_ms')} pat={anomaly.get('pattern','?') if isinstance(anomaly,dict) else anomaly} "
            f"count={anomaly.get('count') if isinstance(anomaly,dict) else '?'} "
            f"health={p.get('render_health_status','')}/{p.get('render_health_reason','')} "
            f"recov={p.get('recovery_count')} nonblank_rows={p.get('visible_nonblank_rows')} "
            f"reasons={anomaly.get('reasons') if isinstance(anomaly,dict) else ''}" + extra
        )


if __name__ == "__main__":
    main()
