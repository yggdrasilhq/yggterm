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
