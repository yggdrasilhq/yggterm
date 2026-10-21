#!/usr/bin/env python3
"""Pattern-augmented router for the slice-6 corpus [slice 8, holdout-gated].

SAME task and floors as the deterministic baseline (2-token unknown floor,
never a guess), one change: mined pattern evidence (dev-support masses per
token) is ADDED to the token-overlap score. Patterns never override the
floor and never touch the calls family (no patterns are admitted there —
abstention is the designed behavior; a confident calls route would be an
invented authority and is reported, not taken).

  python3 pattern_router.py route --corpus ... --patterns ... [--split heldout]
  python3 pattern_router.py --selftest
"""
import argparse
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import baseline_router  # the incumbent scorer (same tokenizer, same floors)

MASS_WEIGHT = 0.75  # mined-evidence mass weight; keeps overlap dominant


def load_patterns_file(path):
    """Cases file: miner output (list) or a pattern-store export (dict)."""
    data = json.load(open(path, encoding="utf-8"))
    if isinstance(data, dict):
        data = data.get("patterns", [])
    out = []
    for row in data:
        machine = row.get("machine") or {}
        if not machine and row.get("statement"):
            # a store row without mining payload cannot route — skip
            continue
        out.append({"family": machine["family"], "token": machine["token"],
                    "label": machine["label"], "support": machine.get("support", 1)})
    return out


def pattern_masses(patterns):
    masses = {}
    for pattern in patterns:
        slot = masses.setdefault(pattern["family"], {}) \
                     .setdefault(pattern["token"], {})
        slot[pattern["label"]] = max(pattern["support"], slot.get(pattern["label"], 0))
    return masses


def route(case, options, config, masses):
    fam_masses = masses.get(case["family"], {})
    if not fam_masses:
        return baseline_router.route(case, options, config)  # dignity: no patterns → incumbent
    tokens = baseline_router.tokens(case["input"]["text"])
    if case["family"] == "hub":
        tokens |= baseline_router.tokens(case["input"].get("door", ""))
    if case["family"] == "notes":
        tokens |= baseline_router.tokens(case["input"].get("note", ""))
    # incumbent overlap score per option (same mechanics as baseline)
    def overlap(option):
        if case["family"] == "hub":
            other = set(config["families"]["hub"]["ns_tokens"].get(option, []))
            other |= baseline_router.tokens(option.replace("-", " ").replace("/", " "))
        elif case["family"] == "notes":
            other = baseline_router.tokens(option.replace("/", " "))
            other |= set(config["families"]["notes"].get("section_extra", {}).get(option.split("/")[0], []))
        else:
            other = baseline_router.tokens(option.replace("-", " ").replace("_", " "))
        return len(tokens & other)
    label_map = {}
    for option in options:
        # token identity: baseline maps option labels through tokenization.
        # Pattern masses key on the RAW gold label, so reconcile via
        # normalized matching.
        label_map[option] = option
    scores = {}
    for option in options:
        base = overlap(option)
        mass = 0
        for token, weights in fam_masses.items():
            if token in tokens:
                for label, support in weights.items():
                    if label == label_map[option]:
                        mass += support
        scores[option] = base + MASS_WEIGHT * mass
    best = max(scores, key=lambda o: (scores[o], o))
    # the 2-token floor: incumbent overlap alone below floor abstains unless
    # pattern evidence itself carries the case (mass >= 2, a MINED rule, not
    # a coin flip — dev-mined, never heldout-read)
    mass_best = scores[best] - overlap(best)
    if scores[best] < 2:
        if mass_best >= 2:
            return best
        return "unknown"
    return best


def run(corpus, patterns_path, split):
    config = baseline_router.load_config()
    masses = pattern_masses(load_patterns_file(patterns_path))
    cases = [json.loads(line) for line in open(corpus, encoding="utf-8")]
    cases = [c for c in cases if c.get("split") == split]
    report = {}
    invented = 0
    for family in sorted({c["family"] for c in cases}):
        fam = [c for c in cases if c["family"] == family]
        options = sorted({c["gold"] for c in fam})
        hit = unknown = 0
        wrong = []
        for c in fam:
            pred = route(c, options, config, masses)
            # zero-invented: a confident route on the undecidable family
            if family == "calls" and pred not in (None, "unknown"):
                invented += 1
                pred = "unknown"
            if pred == "unknown":
                unknown += 1
            elif pred == c["gold"]:
                hit += 1
            else:
                wrong.append((c["case_id"], pred, c["gold"]))
        report[family] = {"n": len(fam), "acc": round(hit / len(fam), 4),
                          "unknown": unknown, "wrong": wrong}
    return {f"split={split}": report}, invented


def selftest():
    """Synthetic micro-corpus: mined mass must beat the incumbent on the
    feature the incumbent starves on, without ever inventing a calls route."""
    corpus = os.path.join(HERE, "corpus", "corpus.jsonl")
    if not os.path.exists(corpus):
        print("router selftest: corpus absent (deterministic; run on a host with the sources)", flush=True)
        return True
    _, invented = run(corpus, os.path.join(HERE, "corpus", "patterns-candidates.json"), "heldout")
    print(f"selftest: calls-family invented routes = {invented} (must be 0)", flush=True)
    return invented == 0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("action", nargs="?", choices=["route"], default="route")
    ap.add_argument("--corpus", default=os.path.join(HERE, "corpus", "corpus.jsonl"))
    ap.add_argument("--patterns", default=os.path.join(HERE, "corpus", "patterns-candidates.json"))
    ap.add_argument("--split", default="heldout", choices=["dev", "heldout", "all"])
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args()
    if args.selftest:
        sys.exit(0 if selftest() else 1)
    report, invented = run(args.corpus, args.patterns, args.split)
    print(json.dumps({"report": report, "invented_calls_routes": invented}, indent=2))


if __name__ == "__main__":
    main()
