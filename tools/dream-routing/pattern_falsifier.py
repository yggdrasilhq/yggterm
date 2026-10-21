#!/usr/bin/env python3
"""The slice-8 falsifier instrument: holdout comparison — no measured
improvement = no promotion [gap 4, T-010].

Runs the pattern-augmented router and the deterministic baseline side by
side on the HELDOUT split only, then rules:

  PROMOTION-ELIGIBLE requires ALL of
    - paired exact-binomial improvement for the pattern router (b wins > a
      losses among non-tied heldout cases, p < 0.05 one-sided),
    - no net harm (wrong-count not worse),
    - zero invented authorities (any confident route on the undecidable
      calls family fails the gate outright).

Anything else records NOT-PROMOTED with the same evidence — the run is
still valid data; only the promotion is denied.

  python3 pattern_falsifier.py [--corpus ...] [--patterns ...] [--baseline ...]
  python3 pattern_falsifier.py --selftest
"""
import argparse
import json
import math
import os
import sys
from datetime import datetime, timezone

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import baseline_router
import pattern_router

HELDOUT_SPLIT = "heldout"


def paired_outcomes(corpus, patterns_path, split=HELDOUT_SPLIT):
    """One route per (router, case); results row-matched for the pairing."""
    config = baseline_router.load_config()
    masses = pattern_router.pattern_masses(pattern_router.load_patterns_file(patterns_path))
    cases = [json.loads(line) for line in open(corpus, encoding="utf-8")]
    cases = [c for c in cases if c.get("split") == split]
    rows = []
    invented = 0
    for c in cases:
        fam_cases = [x for x in cases if x["family"] == c["family"]]
        options = sorted({x["gold"] for x in fam_cases})
        base = baseline_router.route(c, options, config)
        aug = pattern_router.route(c, options, config, masses)
        if c["family"] == "calls" and aug not in (None, "unknown"):
            invented += 1  # recorded, never taken
            aug = "unknown"
        if c["family"] == "calls" and base not in (None, "unknown"):
            invented += 1
            base = "unknown"
        rows.append({"case_id": c["case_id"], "family": c["family"], "gold": c["gold"],
                     "baseline": base, "augmented": aug})
    return rows, invented


def summarize(rows):
    by_family = {}
    for family in sorted({r["family"] for r in rows}):
        fam = [r for r in rows if r["family"] == family]
        hit = sum(1 for r in fam if r["augmented"] == r["gold"] and r["augmented"] != "unknown")
        unk = sum(1 for r in fam if r["augmented"] == "unknown")
        bas_hit = sum(1 for r in fam if r["baseline"] == r["gold"] and r["baseline"] != "unknown")
        bas_unk = sum(1 for r in fam if r["baseline"] == "unknown")
        by_family[family] = {
            "n": len(fam),
            "baseline_acc": round(bas_hit / len(fam), 4),
            "baseline_unknown": bas_unk,
            "augmented_acc": round(hit / len(fam), 4),
            "augmented_unknown": unk,
            "baseline_wrong": [r["baseline"] for r in fam
                               if r["baseline"] not in ("unknown", r["gold"])],
            "augmented_wrong": [r["augmented"] for r in fam
                                if r["augmented"] not in ("unknown", r["gold"])],
        }
    return by_family


def exact_one_sided(wins, losses):
    """Exact binomial p for wins > losses (two-sided count, one-sided p)."""
    if wins <= losses:
        return 1.0
    tail = sum(math.comb(wins + losses, k) for k in range(wins, wins + losses + 1))
    return tail / 2 ** (wins + losses)


def verdict(rows, invented):
    wins = sum(1 for r in rows
               if r["augmented"] == r["gold"] and r["baseline"] != r["gold"] and r["augmented"] != "unknown")
    losses = sum(1 for r in rows
                 if r["baseline"] == r["gold"] and r["augmented"] != r["gold"] and r["augmented"] != "unknown")
    base_hit = sum(1 for r in rows if r["baseline"] == r["gold"] and r["baseline"] != "unknown")
    aug_hit = sum(1 for r in rows if r["augmented"] == r["gold"] and r["augmented"] != "unknown")
    p = exact_one_sided(wins, losses)
    gates = {
        "paired_improvement": wins > losses and p < 0.05,
        "no_net_harm": aug_hit >= base_hit,
        "zero_invented": invented == 0,
        "_detail": {"wins": wins, "losses": losses, "p_value": round(p, 5),
                    "baseline_acc": round(base_hit / len(rows), 4),
                    "augmented_acc": round(aug_hit / len(rows), 4)},
    }
    promoted = all(v for k, v in gates.items() if k != "_detail")
    return ("PROMOTION-ELIGIBLE" if promoted else "NOT-PROMOTED"), gates


def selftest():
    corpus = os.path.join(HERE, "corpus", "corpus.jsonl")
    if not os.path.exists(corpus):
        print("falsifier selftest: corpus absent (deterministic; run on a host with the sources)", flush=True)
        return True
    rows, invented = paired_outcomes(corpus, os.path.join(HERE, "corpus", "patterns-candidates.json"), "heldout")
    v, gate = verdict(rows, invented)
    print(f"selftest: rows={len(rows)} invented={invented} verdict={v} gates={ {k: v for k, v in gate.items() if k != '_detail'} }", flush=True)
    return invented == 0 and len(rows) == 92


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--corpus", default=os.path.join(HERE, "corpus", "corpus.jsonl"))
    ap.add_argument("--patterns", default=os.path.join(HERE, "corpus", "patterns-candidates.json"))
    ap.add_argument("--out", default=os.path.join(HERE, "results"))
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args()
    if args.selftest:
        sys.exit(0 if selftest() else 1)
    rows, invented = paired_outcomes(args.corpus, args.patterns, HELDOUT_SPLIT)
    v, gates = verdict(rows, invented)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S")
    os.makedirs(args.out, exist_ok=True)
    path = os.path.join(args.out, f"{stamp}-pattern-falsifier.json")
    payload = {
        "verdict": v,
        "gates": gates,
        "zero_invented_calls_routes": invented,
        "per_family": summarize(rows),
        "rows": rows,
        "as_of": datetime.now(timezone.utc).isoformat(),
        "heldout_split": HELDOUT_SPLIT,
        "note": "heldout material was never read at mining time; the promotion ceiling is 'promotion-eligible' — the panel owns promotion",
    }
    json.dump(payload, open(path, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    print(f"pattern-falsifier: VERDICT={v}")
    print(json.dumps(gates, indent=2))
    print(f"wrote {path}")


if __name__ == "__main__":
    main()
