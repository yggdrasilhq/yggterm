# Slice-6 routing corpus + shadow harness (memory-work campaign)

Deterministic extraction, a deterministic baseline, and a Jev 1.13 SHADOW
client for routing-corpus evaluation. **Shadow only — nothing here promotes
anything; the recall panel owns promotion.**

## Privacy shape (why the config is gitignored)

The harness is identity-free and publishable; `routing-config.json` (gitignored)
carries everything local: source paths, namespace/section label spaces, keyword
maps. The generated `corpus/` and `results/` are also gitignored — they embed
source text from private stores. The corpus regenerates deterministically
(seed 20261016) on any host holding the sources.

## Families (gold labels = source ownership, never hand-labeled)

| family | kind | task | gold from |
|---|---|---|---|
| hub | hub | door (name+desc+body-head) → owning namespace | memory hub dir |
| notes | notes | note (title+text-head) → owning section/book | notes tree path |
| cases | cases | case passage → owning case | cases tree path |
| calls | calls | transcript excerpt → owning circle — **deliberately undecidable from content** (the owner is path metadata, not excerpt content) | recording path |

The `calls` family is the ambiguity falsifier from the campaign contract:
correct behavior is abstention; any confident routing = invented authority.

Variants (deterministic): clean / damage (18% char mangle) / injection
(adversarial instruction text appended) / order (criteria option order
permuted per case). Split: 70% dev / 30% heldout per family. **Heldout is
never sent to the classifier before the panel gate.**

## Measured dev results (2026-10-16, jev-latest = jev-1.13.0, first-party API)

| family (dev n) | baseline acc / unk | shadow acc / unk / wrong |
|---|---|---|
| hub (19) | 5% / 79% | **100%** / 0 / 0 |
| notes (35) | 0% / 100% | **94%** / 0 / 2 |
| cases (16) | 25% / 63% | **100%** / 0 / 0 |
| calls (45) | 0% / 100% | 0% / **100% abstained, 0 invented** |
| ALL dev (115) | 3.5% / 91% | **59.1%** / 39.1% / **1.7% wrong** |

Variant effect (shadow, dev): clean ≈ damage ≈ injection ≈ order — the
option-order permutation cost zero on this corpus (order-permutation
robustness measured on 33+ order-variant cases).
Cost: 405 attempts total, 176.8k input tokens, **$0.0074** across both
numbered runs (session counters enforce $0.10 / 1000 attempts / 2 req/s).
Transport shapes learned: `questions` is a dict keyed by question id;
answers arrive top-level as `answers.<id>.choice` (+ probabilities, usage).

## Known gaps / growth paths

- `calls` quota is 63, not 100: only a minority of recordings carry
  transcripts and only 4 of those are case-linked. A content-decidable
  case-routing subfamily needs more transcribed+linked recordings.
- The Yang hypothesis `1f98481bcd178641` (hub ≥90%, zero invented) is
  **supported on dev (19/19, 0 invented)** but stays open: its
  falsification test names the full hub slice including heldout, which only
  the recall panel may open.
- Native-language variance (Bengali/code-switch) is intrinsic to `calls`
  excerpts; engineered language variants for the other families are future
  work.

## Run

```sh
python3 extract_corpus.py                      # corpus/corpus.jsonl (gitignored)
python3 baseline_router.py --split all         # deterministic, no LLM
set -a; source <local secret env>; set +a
source <(iexport)                              # resolves the API key
python3 run_shadow.py --family hub --split dev # baseline + shadow
```

Cache `~/.cache/dream-routing/jev-cache.jsonl` (sha-keyed, includes option
order); any failure = `unknown`, never a guess.

## Slice 8 — pattern learning (T-010, holdout-gated)

`pattern_miner.py` mines evidence-linked routing PATTERNS from the DEV split
only (heldout is never read at mining time — asserted in `--selftest`).
Output: `corpus/patterns-candidates.json` in the slice-8 pattern-store
schema (statement, scope, authority=observed-practice, episodes with
content-hash eids, counterexample search, uncertainty). 65 rows (seed state
2026-10-21); 9 rule-grade rows (support ≥4) carry `promotion-eligible` on
their OWN measured holdout run.

`pattern_router.py` reuses the incumbent scorer and ADDS mined mass; same
2-token unknown floor, calls family untouched (no patterns admitted there —
abstention is the designed behavior; a confident calls route is reported as
an invented authority, never taken).

`pattern_falsifier.py` is the gate: baseline vs pattern-augmented on
HELDOUT, paired exact-binomial, no-harm, zero-invented. Measured
2026-10-21 (extraction seed state 2026-10-16, heldout n=92/75 routed):

| run | set | baseline acc | augmented | wins/losses | p |
|---|---|---|---|---|---|
| 071133 | full 69 (first mine, incl. shadow tokens) | 9.8% | 26.1% | 17/2 | 0.00036 |
| 071644 | full 65 (composer-pass pruning) | 9.8% | 26.1% | 17/2 | 0.00036 |
| 071719 | rule-grade 9 (support ≥4) | 9.8% | 19.6% | 10/1 | 0.0059 |

All three: zero invented, calls intact (17/17 abstain). Promotion ceiling
is `promotion-eligible` — the panel (owner) owns actual promotion; per
family: hub 26.7→46.7%, notes 4.4→31.1% (pruned run 22.2%), cases flat
20% (its routing evidence is name-shape, not content tokens — extraction
artifacts remain the known corpus-hygiene gap). The Jev shadow is still the
ceiling performance for the axis (T-008 heldout 97.3%); pattern learning is
the DETERMINISTIC floor improvement and the durable evidence-linked layer
`ygg-memory patterns --ns=_global` keeps (65 rows, status stamped).

Composer-pass note (one `opencode/glm-5.3-flash-free` call, aliased labels,
privacy-gate PASS e905777e): flagged support-2 function-word shadows and
tokenizer fragments (`mone`, `mport`). STOPWORDS extended accordingly
(second run proves the pruning is no-harm). Composer output is descriptive
analysis, never a write authority.
