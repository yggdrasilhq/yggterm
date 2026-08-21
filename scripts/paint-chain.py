#!/usr/bin/env python3
"""Read the mount->paint chain end to end, in one query.

⛔⛔ WHY THIS EXISTS. Until the `xterm_paint` probes landed, every instrument in
this project stopped at the Rust boundary: the native side could say a terminal
MOUNTED and the canvas could say it was busy, but nothing could say whether the
glyphs ever arrived — and a mount begins with an EMPTY surface. So a ghost frame
(the old row still on screen, the new mount not yet painted) and a broken TUI
paint (a mount that painted some rows and stopped) were both invisible: they
looked exactly like a healthy mount plus a screenshot nobody took.

This joins the two halves on `host_id`, which already encodes the mount epoch
(`<host>-m<epoch>`), so no new identity is introduced anywhere.

    native      terminal_mount/begin, bootstrap_reset, retained_rehydrate_*
    canvas      xterm_paint/mount_open -> first_frame -> settle

⭐ THE VERDICT COLUMN IS THE POINT, and it is derived from coverage rather than
from a frame count. The renderer repaints only the rows it marked dirty, so "a
frame happened" says nothing about how much of the viewport it covered. On a
MOUNT the stronger test is available precisely because the surface started
blank: every row holding text must be painted at least once, so a row with
content that no frame has covered is a row the user cannot see.

    painted    every row holding content was covered by some frame
    partial    the canvas holds text on rows no frame has painted  <- the bug
    unpainted  bytes reached the canvas and no frame landed after them
    no-bytes   the surface opened and was never handed anything to paint
    blind      the buffer could not be read; NOT the same as empty
    open       a mount that opened and whose settle has not been written yet

⛔ `painted` in the record means "a frame landed AFTER bytes reached the canvas",
not "a frame happened". The first cut of this probe latched the first frame
outright and measured 218 ms of the canvas painting itself EMPTY — the exact
event it was built to tell apart from glyphs arriving. `blank_frames_before_write`
is that count kept rather than thrown away: a blank surface repainting is what a
ghost frame is.

⚠ AND AN INVISIBLE HOST IS NOT A FAULT. The churn re-mounts rows nobody is
looking at, and their renderer is idle by design, so they report `unpainted`
truthfully. `--visible-only` filters to the mounts a person could actually have
been looking at; the unfiltered count is what the churn costs.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys
from pathlib import Path

NATIVE_CATEGORY = "terminal_mount"
CANVAS_CATEGORY = "xterm_paint"


def trace_files(home: Path) -> list[Path]:
    """Newest-first is wrong here: records are read in time order, so the
    rotated generations come first and the live file last."""
    rotated = sorted(home.glob("event-trace.g*.jsonl"))
    live = home / "event-trace.jsonl"
    return rotated + ([live] if live.exists() else [])


def parse_since(raw: str | None) -> int | None:
    if not raw:
        return None
    match = re.fullmatch(r"(\d+)([smhd]?)", raw.strip())
    if not match:
        raise SystemExit(f"unparseable --since: {raw!r} (try 10m, 2h, 90s)")
    value = int(match.group(1))
    scale = {"": 1, "s": 1000, "m": 60_000, "h": 3_600_000, "d": 86_400_000}
    return value * scale[match.group(2)]


def load(home: Path, since_ms: int | None) -> list[dict]:
    now_ms = None
    rows: list[dict] = []
    for path in trace_files(home):
        try:
            with path.open("r", errors="replace") as handle:
                for line in handle:
                    line = line.strip()
                    if not line or '"terminal_mount"' not in line and '"xterm_paint"' not in line:
                        continue
                    try:
                        record = json.loads(line)
                    except json.JSONDecodeError:
                        continue
                    if record.get("category") in (NATIVE_CATEGORY, CANVAS_CATEGORY):
                        rows.append(record)
        except OSError:
            continue
    rows.sort(key=lambda r: (r.get("ts_ms") or 0, r.get("seq") or 0))
    if since_ms is not None and rows:
        now_ms = rows[-1].get("ts_ms") or 0
        rows = [r for r in rows if (r.get("ts_ms") or 0) >= now_ms - since_ms]
    return rows


def mount_key(record: dict) -> str | None:
    payload = record.get("payload") or {}
    host_id = payload.get("host_id")
    if host_id:
        return str(host_id)
    return None


def build(rows: list[dict]) -> list[dict]:
    """Segment the stream into BUILDS of a surface, not into host ids.

    ⛔⛔ A `host_id` IS NOT ONE SURFACE. It is `<host>-m<mount_epoch>`, and the
    epoch is REUSED: measured 2026-08-21, one row produced two
    `terminal_mount/begin` events and two `xterm_paint/mount_open` records
    twelve seconds apart, both on epoch 1, with `terminal_mount/mount_epoch_reused`
    saying so in between. Keying on the host id alone therefore collapses two
    surface builds into one — the later overwriting the earlier — and a probe
    that would have shown the second build painting badly disappears into the
    first build's clean record. That is the churn's own signature being hidden
    by the instrument sent to measure it.

    ⇒ A build opens on a native `begin` (which precedes the canvas by ~150 ms)
    or, failing that, on a canvas `mount_open`. Everything after it belongs to
    that build until the next one opens.
    """
    label: dict[str, str] = {}
    builds: list[dict] = []
    current: dict[str, dict] = {}

    def open_build(key: str) -> dict:
        entry = {
            "host_id": key,
            "build": sum(1 for b in builds if b["host_id"] == key),
            "native": [],
            "open": None,
            "first_frame": None,
            "settles": [],
            "resets": 0,
        }
        builds.append(entry)
        current[key] = entry
        return entry

    for record in rows:
        key = mount_key(record)
        if not key:
            continue
        payload = record.get("payload") or {}
        session_path = payload.get("session_path")
        if session_path and key not in label:
            label[key] = str(session_path)
        name = record.get("name")
        native = record.get("category") == NATIVE_CATEGORY
        entry = current.get(key)
        if entry is None:
            entry = open_build(key)
        elif name == "begin" and native:
            # ⚠ Only a SECOND `begin` opens a new build. The first one lands in
            # the segment a preceding `bootstrap_reset` already opened, and
            # treating it as a boundary would split every row's first build in
            # two and invent an empty one.
            already_begun = entry["open"] is not None or any(
                r.get("name") == "begin" for r in entry["native"]
            )
            if already_begun:
                entry = open_build(key)
        elif name == "mount_open" and entry["open"] is not None:
            # A second surface with no native `begin` in front of it. Rare, and
            # the one shape that would otherwise silently overwrite a build.
            entry = open_build(key)
        if native:
            entry["native"].append(record)
            if name == "bootstrap_reset":
                entry["resets"] += 1
        elif name == "mount_open":
            entry["open"] = record
        elif name == "first_frame":
            entry["first_frame"] = record
        elif name == "settle":
            entry["settles"].append(record)
    for entry in builds:
        entry["session"] = label.get(entry["host_id"], "")
        entry["label"] = entry["host_id"] + (f"#b{entry['build']}" if entry["build"] else "")
    return builds


def verdict(mount: dict) -> str:
    settles = mount["settles"]
    if not settles:
        if mount["open"] is None:
            # Native rows only: the script never reached the surface, or the
            # window predates the probe.
            return "native-only"
        return "open"
    last = (settles[-1].get("payload") or {})
    if last.get("rows_with_content") == -1:
        return "blind"
    if not last.get("painted"):
        # A mount handed nothing is a different finding from a mount handed
        # bytes that never reached the screen, and collapsing them would let
        # the churn's cheapest failure hide inside its most expensive one.
        return "unpainted" if last.get("writes") else "no-bytes"
    if last.get("complete"):
        return "painted"
    return "partial"


def ms(value) -> str:
    if value is None:
        return "-"
    try:
        return f"{float(value):.0f}"
    except (TypeError, ValueError):
        return "-"


def report(mounts: list[dict], visible_only: bool) -> int:
    ordered = sorted(
        mounts,
        key=lambda m: (m["open"] or (m["native"][0] if m["native"] else {})).get("ts_ms") or 0,
    )
    counts: dict[str, int] = {}
    print(
        f"{'surface build':<38} {'verdict':<9} {'vis':<4} {'rst':>3} "
        f"{'open→wr':>8} {'wr→par':>7} {'par→fr':>7} {'wr→fr':>7} "
        f"{'blank':>5} {'covered':>9} {'late':>6}  session"
    )
    for mount in ordered:
        state = verdict(mount)
        frame = (mount["first_frame"] or {}).get("payload") or {}
        settle = (mount["settles"][-1].get("payload") if mount["settles"] else {}) or {}
        visible = settle.get("visible", frame.get("visible"))
        if visible_only and visible is not True:
            continue
        counts[state] = counts.get(state, 0) + 1
        covered = "-"
        if mount["settles"]:
            covered = f"{settle.get('rows_covered', '?')}/{settle.get('rows_with_content', '?')}"
        print(
            f"{mount['label']:<38} {state:<9} "
            f"{('yes' if visible is True else 'no' if visible is False else '?'):<4} "
            f"{mount['resets']:>3} "
            f"{ms(frame.get('open_to_write_ms')):>8} "
            f"{ms(frame.get('write_to_parsed_ms')):>7} "
            f"{ms(frame.get('parsed_to_frame_ms')):>7} "
            f"{ms(frame.get('write_to_frame_ms')):>7} "
            f"{ms(settle.get('blank_frames_before_write', frame.get('blank_frames_before_write'))):>5} "
            f"{covered:>9} "
            f"{ms(settle.get('overshoot_ms')):>6}  {mount['session']}"
        )
    print()
    total = sum(counts.values())
    summary = " · ".join(f"{name} {count}" for name, count in sorted(counts.items()))
    print(f"{total} surface builds: {summary or 'none'}")
    # ⛔ A missing probe and a quiet system look identical, so say which it is.
    if not total:
        print(
            "no mounts in window — either nothing mounted, or this build predates "
            "the xterm_paint probes (check for xterm_paint records at all)"
        )
    return 0 if not counts.get("partial") and not counts.get("unpainted") else 1



def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--home", default=os.environ.get("YGGTERM_HOME") or str(Path.home() / ".yggterm"))
    parser.add_argument("--since", default=None, help="window ending at the newest record, e.g. 10m")
    parser.add_argument("--visible-only", action="store_true", help="only mounts on a host with a non-zero rect")
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    home = Path(args.home)
    if not home.is_dir():
        raise SystemExit(f"no such trace home: {home}")
    rows = load(home, parse_since(args.since))
    mounts = build(rows)
    if args.json:
        out = []
        for mount in mounts:
            out.append(
                {
                    "host_id": mount["host_id"],
                    "build": mount["build"],
                    "session_path": mount["session"],
                    "verdict": verdict(mount),
                    "bootstrap_resets": mount["resets"],
                    "mount_open": (mount["open"] or {}).get("payload"),
                    "first_frame": (mount["first_frame"] or {}).get("payload"),
                    "first_frame_ms": (mount["first_frame"] or {}).get("duration_ms"),
                    "settles": [s.get("payload") for s in mount["settles"]],
                }
            )
        json.dump(out, sys.stdout, indent=2)
        print()
        return 0
    return report(mounts, args.visible_only)


if __name__ == "__main__":
    sys.exit(main())
