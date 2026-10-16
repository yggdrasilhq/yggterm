#!/usr/bin/env python3
"""Deterministic baseline router for the slice-6 corpus (no LLM).

Token-overlap routing against the label space; the namespace/section/person
keyword maps come from the GITIGNORED routing-config.json (identity stays
local, method stays publishable). Deliberately simple — the incumbent the
shadow classifier is compared against. 2-token floor: below it, unknown
(never a coin flip).
"""
import argparse
import json
import os
import re

WORD = re.compile(r"[A-Za-z\u0980-\u09FF]{3,}")


def tokens(text):
    return set(w.lower() for w in WORD.findall(text or ""))


def _best_overlap(text_tokens, option_tokens):
    best, best_score = "unknown", 0
    for option, toks in option_tokens.items():
        score = len(text_tokens & toks)
        if score > best_score:
            best, best_score = option, score
    return best if best_score >= 2 else "unknown"


def load_config(path=None):
    here = os.path.dirname(os.path.abspath(__file__))
    if path is None:
        path = os.path.join(here, "routing-config.json")
    return json.load(open(path, encoding="utf-8"))


def route(case, options, config):
    fam_cfg = config["families"][case["family"]]
    kind = fam_cfg["kind"]
    text_tokens = tokens(case["input"]["text"])
    if kind == "hub":
        ns_tokens = fam_cfg["ns_tokens"]
        m = {ns: set(ns_tokens.get(ns, []))
             | tokens(ns.replace("-", " ").replace("/", " ")) for ns in options}
        text_tokens |= tokens(case["input"].get("door", ""))
        return _best_overlap(text_tokens, m)
    if kind == "notes":
        extra = fam_cfg.get("section_extra", {})
        m = {}
        for label in options:
            toks = tokens(label.replace("/", " "))
            toks |= set(extra.get(label.split("/")[0], []))
            m[label] = toks
        text_tokens |= tokens(case["input"].get("note", ""))
        return _best_overlap(text_tokens, m)
    if kind == "cases":
        m = {c: tokens(c.replace("-", " ").replace("_", " ")) for c in options}
        return _best_overlap(text_tokens, m)
    if kind == "calls":
        m = {p: set(ws) for p, ws in fam_cfg.get("person_words", {}).items()
             if p in options}
        return _best_overlap(text_tokens, m)
    return "unknown"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--corpus", default=os.path.join(
        os.path.dirname(os.path.abspath(__file__)), "corpus", "corpus.jsonl"))
    ap.add_argument("--split", default="all", choices=["all", "dev", "heldout"])
    a = ap.parse_args()
    config = load_config()
    cases = [json.loads(l) for l in open(a.corpus, encoding="utf-8")]
    cases = [c for c in cases if a.split == "all" or c["split"] == a.split]
    by_fam = {}
    for c in cases:
        by_fam.setdefault(c["family"], []).append(c)
    report = {}
    for fam, fam_cases in sorted(by_fam.items()):
        options = sorted({c["gold"] for c in fam_cases})
        hit = unknown = 0
        for c in fam_cases:
            pred = route(c, options, config)
            if pred == "unknown":
                unknown += 1
            elif pred == c["gold"]:
                hit += 1
        n = len(fam_cases)
        report[fam] = {"n": n, "correct": hit, "abstained_unknown": unknown,
                       "wrong": n - hit - unknown,
                       "accuracy": round(hit / n, 4) if n else None}
    print(json.dumps(report, indent=1))


if __name__ == "__main__":
    main()
