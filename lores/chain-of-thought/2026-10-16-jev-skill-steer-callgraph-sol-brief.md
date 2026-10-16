# CONSULT — round 5: the Jev skill + steer door + callgraph/kastengraph classification (sol HIGH)

You are sol HIGH. Round 4 landed the whole-network dream architecture (typed
observation adapters, cost gate, work-class registry, Yin/Yang, focus docs) and
ruled "Luna first, shadow-only." The OWNER HAS OVERRULED on provider economics:
**Jev 1.13 is chosen** ($5 prepaid topup via OpenRouter Decisions endpoint; no
API account setup; $0.042/M input, output free). He now wants:

**(R1)** A **Jev skill in the ynpm skill registry** so agents use it without
hand-wiring it. Registry shape (measured): `~/.claude/skills/<name>/SKILL.md`
with frontmatter `name/description/triggers/allowed-tools`, synced fleet-wide by
fleet-skill-sync; SSOT homes live in `~/git/yggsteer/skills/` on dev
(data-fabric, delta-zero, dossiergraph, fingraph, infra, ygg-auth).

**(R2)** A **small tokenomics + Jev steer door** in the yggsteer root files
(`sections/jev-door.md`, injected into EVERY harness at stamp time) so agents
"use it wholeheartedly where it is fitting and where it is overfitted they
remove it and do the needful as the context needs it."

**(R3)** This work goes into the memory-work campaign (`_global/
campaign-memory-work.md`, seq #58608) — fold into the existing slice 6 (Jev
shadow→production) or re-slice.

## JEV DUE DILIGENCE (verified 2026-10-16)
- Question types: `noul` (P(yes) 0..1), `choice` (pick from runtime options,
  full distribution), `score` (probability-weighted position on an ordered
  rubric). Multiple questions per request, evaluated in parallel.
- HARD LIMITS (published): 64k tokens per request; **32k for state + longest
  question**; throughput 250k tok/s; **1200 requests/min**.
- Known weaknesses (TypeSafe's own docs): arithmetic, date comparison,
  indirection, adversarial/irrelevant content, option-order effects; the
  "confidence" field is NOT a probability of correctness.
- Access: OpenRouter `typesafe/jev-1.13` (Decisions-shaped endpoint; chat SDKs
  do NOT work); state = string/object/array; code owns the workflow.
- Cost: $0.042/M input, output free. vs sol HIGH consult ~60-85k quota tokens
  each (quota-capped subscription — quota is the scarce resource), vs
  GLM-5.3-Flash generative ~same price class but needs schema parsing + is slow
  for classification at volume.

## THE OWNER'S FLAGSHIP CASE (verbatim intent)
"This idea came to me on callgraph work that is mostly investigating of which
are important calls and further exploratory pathways for anything in
dossiergraph or to be in dossiergraph, kastengraph etc. Currently Zcode does it
in batches and it is slow. Can Jev speed this up drastically?"

MEASURED CALLGRAPH FACTS (2026-10-16): TC decode fleet (Truecaller), corpus
35,343 recordings / 2,061h; unknowns queue ~425 unknown numbers; peer_identity
1,271 named / 36 nohit; TC chewer is CREDIT-limited ($2.33 left across 5 keys,
refill cadence external) and WINDOW-limited (bon/dada/maa scheduled windows,
two-seat collisions have corrupted streams); importance investigation =
cross-referencing named peers, transcript content (RTI/police/business), case
links. The SLOW part is the TC-credit+window economy, not model latency — but
the CLASSIFICATION of "which unknown is worth spending TC credits on" and
"which chewed transcript matters and which pathways it opens" is model work
currently done by zcode seats in batches.

ALSO: kastengraph = 3,575 md personal notes (tagging/classification);
dossiergraph = 938 md evidence-first case archive (testimony vs interpretation
vs verified classification); hub = 1,177 doors.

## QUESTIONS

**Q1 — CAN JEV DRASTICALLY SPEED CALLGRAPH?** Analyze honestly against the
measured bottleneck: the TC credit/window economy gates LOOKUPS; classification
of the unknowns queue and chewed transcripts is model work. What is the actual
speedup shape (pre-lookup triage to spend TC credits only on high-value
unknowns? corpus-scale importance sweep of chewed transcripts? pathway
nomination for dossiergraph promotion?)? Quantify: at 1,200 req/min and 32k
state, how long does a full unknowns-queue triage (425 items) and a corpus
importance sweep (35k recordings' metadata; assume 2k token states) take and
cost? What Jev CANNOT do here (fetch TC data, resolve identity, be the sole
authority)?

**Q2 — THE SKILL.** Specify `skills/jev/SKILL.md` contents: access recipe
(OpenRouter endpoint + $5 credit + key handling — NEVER commit keys; env var
name), the three question shapes with callgraph/kastengraph/dossiergraph
examples, the cache rule (content-hash key), limits table, failure modes
(timeout/uncertain ⇒ `unknown` + deterministic fallback), the tokenomics
judgment rule ("use wholeheartedly where fitting; remove where overfit" — give
the agent a DECISION PROCEDURE not a vibe), and what Jev must never decide
(authority, dates, arithmetic, owner decisions). Keep it ONE skill file, tight.

**Q3 — THE STEER DOOR.** Specify `sections/jev-door.md` (injected into every
harness root, so it is per-turn budget — keep it under ~150 words): the law in
3-5 bullets (classify/route/triage ⇒ Jev first; generate/reason/decide ⇒ not
Jev; uncertain ⇒ unknown then escalate; costs are booked to the dream/campaign
ledger; overfit ⇒ remove). 

**Q4 — REGISTRY + CAMPAIGN FOLD.** How the skill + door + the existing slice 6
(Jev shadow→production) fold into the memory-work campaign: re-slice or amend?
Where does the 200-case routing corpus (your round-2 falsifier) live, and what
is the promotion gate from shadow to production for callgraph specifically
(given TC credits make false negatives EXPENSIVE — a mis-triaged important
unknown never gets looked up)?

**Q5 — THE RISK.** Name the top failure mode of adopting Jev wholeheartedly in
callgraph/kastengraph and the cheapest guard.
