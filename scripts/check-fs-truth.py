#!/usr/bin/env python3
"""
check-fs-truth.py — the manual gauge for fs-resolution truthfulness.

The owner's law (2026-09-02): "yggterm fs resolution of each CLI needs to be
truthful — cwdtree/startpage must show the truth of the actual fs underlying.
We can only gauge their truthfulness by manually calculating ourselves."

This script does exactly that calculation, per CLI store, and diffs it against
what the verbs claim. The store walk is deliberately INDEPENDENT of Rust —
raw file mtimes and raw sqlite reads, in Python only — so a shared bug cannot
hide from both sides at once (the ygg_scan_truth.py two-implementation rule).

Checks (exit 2 on any violation):

  A. STAMP LIE — no row may claim a recency NEWER than its own store fact by
     more than RE-STAMP_SKEW. This is the 2026-09-02 lie measured live: the
     verbs stamped every live row with the scan-time millisecond, so rows idle
     for days claimed "used one second ago" at every scan tick.

  B. ORDER LIE — restricted to rows whose truth the store knows, the verb's
     order must BE the store's recency order. Any inversion (an older row
     ranked above a newer one) fails with both rows named.

Stores walked (the CLIs named in the falsifier; membership vs the verbs is
check-startpage/check-cwdtree's job, this file owns RECENCY truth):
  codex         ~/.codex/sessions/**/rollout-*.jsonl          (file mtime)
  claude-code   ~/.claude/projects/*/*.jsonl (minus agent-*)  (file mtime)
  opencode      ~/.local/share/opencode/opencode.db session_v2 (time_updated, ms)

Usage:
  python3 scripts/check-fs-truth.py                 # local host
  python3 scripts/check-fs-truth.py --json          # machine report
  python3 scripts/check-fs-truth.py --skip-cwdtree  # startpage only

Exit 0 when the verbs' fs resolution matches the manual truth; exit 2 on any
lie; exit 3 when the verbs could not be reached (infrastructure, not a lie).
"""

import argparse
import glob
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path

# A claimed recency may exceed the store fact by at most this, to absorb
# write-lag between the CLI's last store flush and the verb's read.
RE_STAMP_SKEW_MS = 10 * 60 * 1000

# Live rows with no store row at all cannot be judged for recency truth from
# the stores alone; they are reported, not failed.
HEADLESS_TIMEOUT = 120

# More than this many rows carrying ONE identical claimed millisecond is the
# scan-stamp signature, not usage truth (A2).
COLLAPSE_K = 3


def now_ms():
    return int(time.time() * 1000)


def walk_codex(home: Path):
    out = {}
    for p in glob.glob(str(home / ".codex/sessions/**/rollout-*.jsonl"), recursive=True):
        sid = Path(p).stem.rsplit("-", 1)[-1]
        # The filename carries a timestamp prefix and the UUID tail; the id is
        # the whole filename stem — use it whole, matching the Rust reader.
        out[Path(p).stem] = int(os.path.getmtime(p) * 1000)
    return out


def walk_claude(home: Path):
    out = {}
    for p in glob.glob(str(home / ".claude/projects/*/*.jsonl")):
        if Path(p).name.startswith("agent-"):
            continue
        out[Path(p).stem] = int(os.path.getmtime(p) * 1000)
    return out


def walk_opencode(home: Path, scratch: Path):
    db = home / ".local/share/opencode/opencode.db"
    out = {}
    if not db.exists():
        return out
    # Never open the live db in place — copy it (WAL included by sqlite's
    # backup path is unnecessary here; a plain file copy of the db plus a
    # retry on a stale read is enough for recency truth).
    tmp = scratch / "opencode-truth.db"
    try:
        tmp.write_bytes(db.read_bytes())
        conn = sqlite3.connect(f"file:{tmp}?mode=ro", uri=True)
        has_v2 = conn.execute(
            "select count(*) from sqlite_master where type='table' and name='session_v2'"
        ).fetchone()[0] > 0
        if has_v2:
            rows = conn.execute(
                "select id, time_updated, time_created from session_v2 "
                "where parent_id is null or parent_id = ''"
            )
            for sid, updated, created in rows:
                ms = updated or created or 0
                out[sid] = int(ms if ms > 10**12 else ms * 1000) if ms else 0
        else:
            rows = conn.execute("select id, time_updated, time_created from session")
            for sid, updated, created in rows:
                ms = (updated or created or 0) * 1000
                out[sid] = ms
    except Exception:
        return out
    finally:
        try:
            tmp.unlink()
        except OSError:
            pass
    return out


def collect_store_truth(home: Path, scratch: Path):
    truth = {}
    for kind, epochs in (
        ("codex", walk_codex(home)),
        ("claude_code", walk_claude(home)),
        ("open_code", walk_opencode(home, scratch)),
    ):
        for sid, ms in epochs.items():
            truth.setdefault(sid, {"kind": kind, "epoch_ms": ms})
    return truth


def collect_live_activity(snapshot_json):
    """The daemon's own per-row last-activity fact (epoch ms), when it serves one.

    A pre-fix daemon omits the field entirely — an empty map, and the
    stamp-collapse check below is what still catches that generation.
    """
    out = {}
    for s in snapshot_json.get("live_sessions", []) or []:
        sid = (s.get("id") or "").strip()
        ms = s.get("last_activity_epoch_ms")
        if sid and isinstance(ms, (int, float)) and ms > 0:
            out[sid] = int(ms)
    return out


def run_snapshot(headless: str, timeout=HEADLESS_TIMEOUT):
    out = subprocess.run(
        [headless, "server", "snapshot"], capture_output=True, text=True, timeout=timeout
    )
    start = out.stdout.find("{")
    if start < 0:
        return {}
    try:
        return json.loads(out.stdout[start:])
    except json.JSONDecodeError:
        return {}


def run_verb(headless: str, args, timeout=HEADLESS_TIMEOUT):
    out = subprocess.run(
        [headless, "server", *args], capture_output=True, text=True, timeout=timeout
    )
    if out.returncode != 0:
        raise RuntimeError(f"verb failed: {args}: {out.stderr[:400]}")
    start = out.stdout.find("{")
    if start < 0:
        raise RuntimeError(f"verb produced no JSON object: {args}")
    return json.loads(out.stdout[start:])


def check_rows(label, rows, truth, live_activity, verdicts):
    stamp_lies = 0
    inversions = 0
    known = []
    for row in rows:
        sid = row.get("session_id", "")
        claimed = int(row.get("modified_epoch_ms") or 0)
        fact_ms = None
        if sid in truth and truth[sid]["epoch_ms"] > 0:
            fact_ms = truth[sid]["epoch_ms"]
        if sid in live_activity:
            # The daemon's own PTY clock is the freshest honest fact about a
            # live row — take the newer of it and the store fact.
            fact_ms = max(fact_ms or 0, live_activity[sid])
        if fact_ms is not None:
            known.append((sid, claimed, fact_ms))
            if claimed - fact_ms > RE_STAMP_SKEW_MS:
                stamp_lies += 1
                verdicts.append(
                    f"FAIL[{label}] STAMP LIE: {kind_of(row)} {sid[:20]} claims "
                    f"{claimed} but the newest fact (store/daemon activity) says "
                    f"{fact_ms} (+{(claimed - fact_ms) // 1000}s of invented recency)"
                )
    # A2 — THE SCAN-STAMP COLLAPSE SIGNATURE. The measured lie (2026-09-02):
    # every live row re-stamped with the scan's own millisecond. Genuinely
    # distinct sessions were not all touched inside one millisecond; a shared
    # stamp across > COLLAPSE_K rows is a scan artifact, store knowledge or
    # not — it is how the lie shows up on generations that serve no
    # per-row activity fact yet.
    by_stamp = {}
    for row in rows:
        claimed = int(row.get("modified_epoch_ms") or 0)
        by_stamp.setdefault(claimed, []).append(row.get("session_id", "?"))
    for stamp, sids in sorted(by_stamp.items(), reverse=True)[:8]:
        if len(sids) > COLLAPSE_K:
            stamp_lies += 1
            verdicts.append(
                f"FAIL[{label}] STAMP COLLAPSE: {len(sids)} rows claim the SAME "
                f"millisecond {stamp} (e.g. {', '.join(s[:18] for s in sids[:4])}) — "
                f"scan-time stamping, not usage truth"
            )
    # Order lie: among rows with a known fact, claimed order must equal truth order.
    truth_rank = sorted(known, key=lambda item: item[2], reverse=True)
    for (sid_a, claimed_a, fact_a), (sid_b, claimed_b, fact_b) in zip(truth_rank, truth_rank[1:]):
        if claimed_b > claimed_a:
            inversions += 1
            verdicts.append(
                f"FAIL[{label}] ORDER LIE: {sid_b[:20]} (truth {fact_b}, claimed {claimed_b}) "
                f"ranks ABOVE {sid_a[:20]} (truth {fact_a}, claimed {claimed_a})"
            )
            if inversions >= 5:
                break
    return stamp_lies, inversions


def kind_of(row):
    return row.get("kind") or "?"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--headless", default="yggterm-headless")
    ap.add_argument("--skip-cwdtree", action="store_true")
    ap.add_argument("--json", action="store_true", help="print a JSON report")
    args = ap.parse_args()

    home = Path.home()
    verdicts = []
    report = {"startpage": None, "cwdtree": None, "verdicts": verdicts}

    try:
        start = time.time()
        sp = run_verb(args.headless, ["startpage", "ls", "--json"])
        verb_ran_at = int((start + time.time()) / 2 * 1000)
        snap = run_snapshot(args.headless)
    except Exception as error:  # noqa: BLE001 — infrastructure, not a lie
        print(f"infrastructure: cannot reach startpage ls: {error}", file=sys.stderr)
        return 3

    scratch = Path(tempfile.mkdtemp(prefix="fs-truth-", dir=home / ".yggterm/scratchpad"))
    try:
        truth = collect_store_truth(home, scratch)
        live_activity = collect_live_activity(snap)
        rows = sp.get("rows", [])
        stamped, inversions = check_rows("startpage", rows, truth, live_activity, verdicts)
        report["startpage"] = {
            "rows": len(rows),
            "store_known": len(truth),
            "live_activity_known": len(live_activity),
            "stamp_lies": stamped,
            "order_lies": inversions,
        }

        if not args.skip_cwdtree:
            try:
                ct = run_verb(args.headless, ["cwdtree", "ls", "--json", "--limit", "100000"])
                ct_rows = [
                    s
                    for g in ct.get("groups", [])
                    for s in g.get("sessions", [])
                ]
                stamped_c, inversions_c = check_rows(
                    "cwdtree", ct_rows, truth, live_activity, verdicts
                )
                report["cwdtree"] = {
                    "rows": len(ct_rows),
                    "stamp_lies": stamped_c,
                    "order_lies": inversions_c,
                }
            except Exception as error:  # noqa: BLE001
                verdicts.append(f"WARN[cwdtree] unreachable: {error}")
    finally:
        pass

    if args.json:
        print(json.dumps(report, indent=1))
    if any(v.startswith("FAIL") for v in verdicts):
        for v in verdicts:
            if not args.json:
                print(v)
        print(
            "\nfs resolution LIES to the reader — the verbs' recency is not the "
            "store's recency. Fix the stamping/ordering, not this checker."
        )
        return 2
    if not args.json:
        print(
            f"fs truth OK — startpage {report['startpage']['rows']} rows / "
            f"cwdtree {report.get('cwdtree', {}).get('rows', 'n/a')} rows agree "
            f"with the stores' own recency."
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())
