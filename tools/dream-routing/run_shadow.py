#!/usr/bin/env python3
"""Slice-6 SHADOW runner: deterministic baseline vs Jev on the routing corpus.

SHADOW ONLY — records agreement numbers; promotes nothing (the recall panel
owns any promotion decision per the round-5 contract).

  python3 run_shadow.py --family hub --split dev            # baseline + jev
  python3 run_shadow.py --family all --split dev --no-jev   # baseline only
"""
import argparse
import json
import os
import time

import baseline_router
import jev_shadow

CONFIG = baseline_router.load_config()


def evaluate(cases, router_name, route_fn):
    per_case, hit, unknown = [], 0, 0
    for c in cases:
        pred = route_fn(c)
        ok = pred == c["gold"]
        if pred == "unknown":
            unknown += 1
        elif ok:
            hit += 1
        per_case.append({"case_id": c["case_id"], "family": c["family"],
                         "variant": c["variant"], "gold": c["gold"],
                         "pred": pred, "ok": ok})
    n = len(cases)
    return {
        "router": router_name, "n": n, "correct": hit,
        "abstained_unknown": unknown, "wrong": n - hit - unknown,
        "accuracy": round(hit / n, 4) if n else None,
        "unknown_rate": round(unknown / n, 4) if n else None,
    }, per_case


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--corpus", default=os.path.join(os.path.dirname(__file__), "corpus", "corpus.jsonl"))
    ap.add_argument("--family", default="all")
    ap.add_argument("--split", default="dev", choices=["all", "dev", "heldout"])
    ap.add_argument("--no-jev", action="store_true")
    ap.add_argument("--out-dir", default=os.path.join(os.path.dirname(__file__), "results"))
    a = ap.parse_args()

    cases = [json.loads(l) for l in open(a.corpus, encoding="utf-8")]
    cases = [c for c in cases
             if (a.family == "all" or c["family"] == a.family)
             and (a.split == "all" or c["split"] == a.split)]
    if not cases:
        print(json.dumps({"error": "no cases matched"}))
        return
    options_all = {}
    for fam in {c["family"] for c in cases}:
        options_all[fam] = sorted({x["gold"] for x in cases if x["family"] == fam})

    report = {"ran_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
              "family": a.family, "split": a.split, "routers": {}}
    base_rep, base_cases = evaluate(
        cases, "deterministic-baseline",
        lambda c: baseline_router.route(c, options_all[c["family"]], CONFIG))
    report["routers"]["deterministic-baseline"] = base_rep
    if not a.no_jev:
        jev_res = []
        for c in cases:
            r = jev_shadow.classify(c, options_all[c["family"]], CONFIG)
            r.update({"case_id": c["case_id"], "gold": c["gold"],
                      "variant": c["variant"], "family": c["family"]})
            jev_res.append(r)
        hit = unknown = 0
        for r in jev_res:
            if r["prediction"] == "unknown":
                unknown += 1
            elif r["prediction"] == r["gold"]:
                hit += 1
        n = len(jev_res)
        report["routers"]["jev-1.13-shadow"] = {
            "n": n, "correct": hit, "abstained_unknown": unknown,
            "wrong": n - hit - unknown,
            "accuracy": round(hit / n, 4) if n else None,
            "unknown_rate": round(unknown / n, 4) if n else None,
            "sources": {s: sum(1 for r in jev_res if r["source"] == s)
                        for s in {r["source"] for r in jev_res}},
        }
        report["jev_per_case"] = jev_res
    report["baseline_per_case"] = base_cases

    os.makedirs(a.out_dir, exist_ok=True)
    out = os.path.join(a.out_dir, time.strftime("%Y%m%dT%H%M%S-shadow.json"))
    json.dump(report, open(out, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    summary = {k: v for k, v in report.items()
               if k in ("family", "split")}
    summary["routers"] = {k: {kk: vv for kk, vv in v.items() if kk != "sources"}
                          for k, v in report["routers"].items()}
    print(json.dumps(summary, indent=1))
    print(f"report: {out}")


if __name__ == "__main__":
    main()
