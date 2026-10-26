#!/usr/bin/env python3
"""cohort_reconcile.py — the C1 instrument (sol turn: falsification report
2026-10-26). Joins the dream (dispatch) ledger with the audit ledger into
ONE row per admitted attempt, so "did reliability improve" becomes a
countable claim with full denominators.

Outcome vocabulary (no greenwashing):
  verified_pass   audit verdict PASS and the attempt reached a terminal state
  audit_fail      audit verdict FAIL (repair_ref carried if present)
  cancelled       dream status cancelled before execution
  unknown_inflight audit non-terminal / attempt still moving
  unknown_noaudit admitted attempt with NO audit row (coverage hole — counted)

Cohort law: a window with no records is UNKNOWN cohort, never zero —
/tmp retention is ~48h, so older cohorts are unreconstructable and say so.

Usage:
  cohort_reconcile.py reconcile --dream L1 --audit L2 [--from ISO] [--to ISO]
  cohort_reconcile.py report   --dream L1 --audit L2 --from A --to B --from C2 --to D2
                               (compare two cohorts; never invents a trend
                                when either side is UNKNOWN/empty)

Stdlib only; read-only; exit 0 always (a report, not a gate).
"""
import argparse
import datetime as dt
import json
import sys
from collections import Counter


def load(path):
    rows = []
    try:
        with open(path) as f:
            for line in f:
                line = line.strip()
                if line:
                    try:
                        rows.append(json.loads(line))
                    except json.JSONDecodeError:
                        rows.append({"_malformed": True})
    except FileNotFoundError:
        return None  # absent source is UNKNOWN, not empty
    return rows


def latest_by_id(rows):
    out = {}
    for r in rows or []:
        i = r.get("id")
        if i:
            out[i] = r  # later rows overwrite: newest attempt/audit wins
    return out


def in_window(ts_ms, frm, to):
    if not ts_ms:
        return True  # untimestamped rows stay in the denominator, flagged
    t = dt.datetime.fromtimestamp(ts_ms / 1000, dt.timezone.utc)
    if frm and t < frm:
        return False
    if to and t > to:
        return False
    return True


def classify(dream, audit):
    if dream.get("_malformed"):
        return "unknown_malformed"  # id-less ledger row — a spawner defect, counted
    st = (dream.get("status") or "").lower()
    if st in ("cancelled", "canceled"):
        return "cancelled"
    if audit is None:
        return "unknown_noaudit"
    v = (audit.get("verdict") or "").upper()
    if v.startswith("PASS"):
        return "verified_pass" if audit.get("terminal") else "unknown_inflight"
    if v.startswith("FAIL"):
        return "audit_fail"
    if v.startswith("UNKNOWN"):
        return "unknown_verdict"
    return "unknown_verdict"


def dedupe_attempts(dreams):
    """The dream ledger is a STATUS STREAM (one row per status update).
    An admitted attempt is counted ONCE: latest row per id wins; id-less
    rows are malformed attempts and stay in the denominator, flagged."""
    latest, order = {}, []
    for d in dreams:
        i = d.get("id")
        if not i:
            d["_malformed"] = True
            order.append(d)  # id-less: keep as its own (broken) attempt
            continue
        if i not in latest:
            order.append(d)
        latest[i] = d
    return [latest.get(d.get("id"), d) if d.get("id") else d for d in order]


def reconcile(dl, al, frm, to):
    dreams = dedupe_attempts(
        [d for d in (dl or []) if in_window(d.get("created_at"), frm, to)])
    audits = latest_by_id([a for a in (al or [])
                           if in_window(_ms(a.get("audited_at")), frm, to)])
    rows = []
    for d in dreams:
        a = audits.get(d.get("id"))
        rows.append({
            "id": d.get("id"), "route": d.get("route"),
            "title": (d.get("title") or "")[:90],
            "created_at": d.get("created_at"),
            "outcome": classify(d, a),
            "audit_verdict": (a or {}).get("verdict"),
            "repair_ref": (a or {}).get("repair_ref"),
            "dream_status": d.get("status"),
        })
    return rows


def _ms(v):
    if v is None:
        return None
    if isinstance(v, (int, float)):
        return v if v > 1e11 else v * 1000  # s → ms heuristics
    try:
        return dt.datetime.fromisoformat(str(v)).timestamp() * 1000
    except ValueError:
        return None


def summarize(rows, label):
    c = Counter(r["outcome"] for r in rows)
    total = len(rows)
    denom = total if total else "UNKNOWN (no records in window)"
    routes = Counter(r["route"] for r in rows if r["outcome"] == "audit_fail")
    lines = [f"== cohort {label}: {total} admitted attempts =="
             f"  (denominator: {denom})"]
    for k in ("verified_pass", "audit_fail", "cancelled", "unknown_inflight",
              "unknown_noaudit", "unknown_verdict", "unknown_malformed"):
        if c.get(k):
            pct = f" ({100 * c[k] / total:.0f}%)" if total else ""
            lines.append(f"  {k:18} {c[k]}{pct}")
    if routes:
        lines.append(f"  audit_fail by route: {dict(routes)}")
    fixes = [r["repair_ref"] for r in rows if r.get("repair_ref")]
    if fixes:
        lines.append(f"  repairs referenced: {fixes}")
    return "\n".join(lines)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dream", required=True)
    ap.add_argument("--audit", required=True)
    ap.add_argument("--from", dest="frm")
    ap.add_argument("--to", dest="to")
    ap.add_argument("--json-out")
    ap.add_argument("--compare", help="second --from/--to window: ISO,ISO")
    a = ap.parse_args()

    def parse(x):
        return dt.datetime.fromisoformat(x).astimezone(dt.timezone.utc) if x else None

    dl, al = load(a.dream), load(a.audit)
    cov = ("dream ledger: " + ("ABSENT" if dl is None else f"{len(dl)} rows") +
           "; audit ledger: " + ("ABSENT" if al is None else f"{len(al)} rows"))
    frm, to = parse(a.frm), parse(a.to)
    rows = reconcile(dl, al, frm, to)
    print(f"# cohort reconciliation — {dt.datetime.now(dt.timezone.utc).isoformat(timespec='seconds')}")
    print(f"# coverage: {cov}")
    if dl:
        ts = [d.get("created_at") for d in dl if d.get("created_at")]
        if ts:
            print(f"# ledger span: {dt.datetime.fromtimestamp(min(ts)/1000, dt.timezone.utc).isoformat()} .. {dt.datetime.fromtimestamp(max(ts)/1000, dt.timezone.utc).isoformat()}")
    print(summarize(rows, f"{a.frm or 'all'}..{a.to or 'now'}"))
    if a.compare:
        f2s, t2s = a.compare.split(",")
        rows2 = reconcile(dl, al, parse(f2s), parse(t2s))
        print()
        print(summarize(rows2, f"{f2s}..{t2s}"))
        c1 = Counter(r["outcome"] for r in rows)
        c2 = Counter(r["outcome"] for r in rows2)
        if rows and rows2:
            def share(c):
                n = sum(c.values())
                return (c.get("verified_pass", 0) / n,
                        c.get("audit_fail", 0) / n) if n else (None, None)
            (p1, f1), (p2, f2) = share(c1), share(c2)
            if None not in (p1, p2):
                d = "UP" if p2 > p1 else "DOWN" if p2 < p1 else "FLAT"
                print(f"\n# verified_pass share: {p1:.0%} -> {p2:.0%} ({d}); "
                      f"audit_fail share: {f1:.0%} -> {f2:.0%}")
                print("# claim 'reliability improved' is TESTABLE only with "
                      "identical admission criteria + follow-up windows on "
                      "both sides; short cohorts are indicative, not proof.")
        else:
            print("\n# one cohort has NO records — comparison is UNKNOWN, "
                  "never a trend (tmp retention ~48h; older cohorts are "
                  "unreconstructable).")
    if a.json_out:
        with open(a.json_out, "w") as f:
            for r in rows:
                f.write(json.dumps(r) + "\n")
    return 0


if __name__ == "__main__":
    sys.exit(main())
