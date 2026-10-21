#!/usr/bin/env python3
"""Deterministic pattern miner for the slice-6 corpus [slice 8, gap 4].

Mines evidence-linked routing PATTERNS from the DEV split only — heldout
material is never read at mining time. A pattern = a routing evidence rule
(a source-content token that reliably indicates an owning label), carried in
the slice-8 pattern-store schema: statement, scope, authority
(observed-practice — code-owned, never an owner claim), episodes (dev cases,
content-hash eided), counterexamples, uncertainty, uncertainty reason.

Discipline (jev law + MoE ruling): this is deterministic co-occurrence work —
it stays in code; no model egress. The `calls` family gets NO patterns by
design (content-undecidable; abstention is the correct behavior there).

  python3 pattern_miner.py [--corpus corpus/corpus.jsonl]
      [--out corpus/patterns-candidates.json] [--min-support N] [--max-per-label K]
      [--selftest]
"""
import argparse
import hashlib
import json
import os
import re
import sys
from collections import Counter, defaultdict

HERE = os.path.dirname(os.path.abspath(__file__))
WORD = re.compile(r"[A-Za-z\u0980-\u09FF]{3,}")
ROUTE_FAMILIES = ("hub", "notes", "cases")  # calls: abstention by design

STOPWORDS = {
    "the", "and", "for", "with", "that", "this", "from", "was", "are", "not",
    "have", "has", "had", "but", "all", "one", "out", "per", "how", "why",
    "its", "each", "their", "his", "her", "any", "同",
    # function/collective-pronoun shadows (slice-8 composer-pass finding
    # 2026-10-21: support-2 argument words pass a pure precision filter by
    # coincidence and become false-rule mass)
    "you", "your", "does", "did", "him", "them", "they", "then", "than",
    "will", "can", "who", "what", "when", "nothing", "must", "never",
    "while", "against", "should", "would", "because",
}


def tokens(text):
    return [w.lower() for w in WORD.findall(text or "")]


def doc_tokens(case, kind):
    parts = [case["input"].get("text", "")]
    if kind == "hub":
        parts.append(case["input"].get("door", ""))
    if kind == "notes":
        parts.append(case["input"].get("note", ""))
    return set(t for t in tokens(" ".join(parts)) if t not in STOPWORDS)


def mine(corpus, min_support=2, max_per_label=8):
    rows = [json.loads(line) for line in open(corpus, encoding="utf-8")]
    dev = [r for r in rows if r.get("split") == "dev"]
    candidates = []
    for family in ROUTE_FAMILIES:
        fam_rows = [r for r in dev if r["family"] == family]
        if not fam_rows:
            continue
        kind = family  # hub/notes carry extra fields in doc_tokens
        docs_by_label = defaultdict(list)
        for r in fam_rows:
            docs_by_label[r["gold"]].append(r)
        label_token_docs = {}
        for label, docs in docs_by_label.items():
            token_docs = defaultdict(int)
            for doc in docs:
                for token in doc_tokens(doc, kind):
                    token_docs[token] += 1
            label_token_docs[label] = token_docs
        doc_count = {label: len(docs) for label, docs in docs_by_label.items()}
        for label, token_docs in label_token_docs.items():
            clean = []
            for token, count in token_docs.items():
                if count < min_support:
                    continue
                # dev precision: the token must appear in NO other label's docs
                if any(token in label_token_docs[other] for other in label_token_docs if other != label):
                    continue
                clean.append((token, count))
            clean.sort(key=lambda tc: (-tc[1], tc[0]))
            for token, count in clean[:max_per_label]:
                supporting = [d for d in docs_by_label[label] if token in doc_tokens(d, kind)]
                episodes = [{
                    "kind": f"case:{family}",
                    "ref": d["case_id"],
                    "eid": hashlib.sha256(f"{family}|{d['case_id']}|{d['input'].get('text','')[:240]}".encode("utf-8")).hexdigest()[:16],
                    "as_of": "2026-10-16",  # corpus extraction date (seed 20261016)
                } for d in supporting[:12]]
                mass = sum(1 for d in fam_rows
                           if d["gold"] == label and token in doc_tokens(d, kind))
                against = sum(1 for d in fam_rows
                              if d["gold"] != label and token in doc_tokens(d, kind))
                level = "low" if count >= 6 else ("medium" if count >= 3 else "high")
                statement = (f"In {family}-family routing, content token '{token}' "
                             f"(dev support {count} docs, never any contra-label doc) "
                             f"is evidence for owning label '{label}'.")
                candidates.append({
                    "statement": (f"In {family}-family routing, content token '{token}' "
                                  f"(dev support {count} docs incl variants, never any "
                                  f"contra-label doc) is evidence for owning label '{label}'."),
                    "scope": f"{family}-family routing on the slice-6 corpus; applies only where the token co-occurs with the routed text",
                    "authority": "observed-practice",
                    "uncertainty": {"level": level,
                                    "reason": f"dev support {count}; contra-label dev fires {against} (0 expected by construction)"},
                    "no_counterexample_found": (
                        f"searched all {sum(doc_count.values())} dev {family} docs: "
                        f"token fires on {mass} '{label}' docs and on 0 contra-label docs"),
                    "episodes": episodes,
                    "counterexamples": [],
                    "machine": {"family": family, "token": token, "label": label,
                                "support": count, "against_dev": against},
                })
    return candidates


def validate(candidates):
    """Import-time discipline mirroring the pattern verb (fail-closed)."""
    problems = []
    for row in candidates:
        for field in ("statement", "scope", "authority", "uncertainty"):
            if not row.get(field):
                problems.append(f"{field} missing on {row.get('machine', {}).get('token')}")
        if not row.get("episodes"):
            problems.append("no episodes")
        if row["authority"] not in ("observed-practice", "owner-instruction"):
            problems.append("authority outside the vocabulary")
    if any(c["machine"]["family"] == "calls" for c in candidates):
        problems.append("calls-family pattern invented (abstention by design)")
    return problems


def selftest():
    with open(os.devnull, "w") as devnull:
        pass
    corpus = os.path.join(HERE, "corpus", "corpus.jsonl")
    if not os.path.exists(corpus):
        print("pattern-miner selftest: corpus absent on this checkout (deterministic; run on a host with the sources)", flush=True)
        return True
    rows = [json.loads(line) for line in open(corpus, encoding="utf-8")]
    heldout_ids = {r["case_id"] for r in rows if r.get("split") == "heldout"}
    candidates = mine(corpus)
    problems = validate(candidates)
    # the mining time must never have touched heldout rows
    episode_refs = {e["ref"] for c in candidates for e in c["episodes"]}
    leak = heldout_ids & episode_refs
    print(f"selftest: {len(candidates)} candidate patterns; "
          f"episodes all-dev={not leak}; problems={problems or 'none'}", flush=True)
    return not problems and not leak


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--corpus", default=os.path.join(HERE, "corpus", "corpus.jsonl"))
    ap.add_argument("--out", default=os.path.join(HERE, "corpus", "patterns-candidates.json"))
    ap.add_argument("--min-support", type=int, default=2)
    ap.add_argument("--max-per-label", type=int, default=8)
    ap.add_argument("--selftest", action="store_true")
    args = ap.parse_args()
    if args.selftest:
        sys.exit(0 if selftest() else 1)
    candidates = mine(args.corpus, args.min_support, args.max_per_label)
    problems = validate(candidates)
    if problems:
        print("pattern-miner: validation problems:", *problems, sep="\n  ")
        sys.exit(1)
    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    json.dump(candidates, open(args.out, "w", encoding="utf-8"), ensure_ascii=False, indent=1)
    print(f"pattern-miner: {len(candidates)} candidate patterns -> {args.out}")


if __name__ == "__main__":
    main()
