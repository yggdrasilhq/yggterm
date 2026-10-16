# CONSULT — round 6 (focused): corrected callgraph facts, free-session economics, System-1 in-chain steer (sol HIGH)

You are sol HIGH. Round 5 specified the Jev skill + steer door (shipped: yggsteer
39e4e62, all harness roots stamped) and estimated a "full 35,343-recording sweep
= 29min / $2.97". The OWNER was alarmed by that number and supplied CORRECTED
FACTS. Re-analyze with them, then rule on two new questions.

## CORRECTED FACTS (measured just now on jojo's callgraph.db, sqlite read-only)
- Total recordings: 35,400. **TRANSCRIBED (transcript text present): 145** (avg
  ~742 tokens each; whole transcript corpus = 430,917 bytes ≈ 108k tokens TOTAL).
- Junk-flagged: 378. Named-peer rows: 24,707. Unknown-peer rows: 10,693.
- Truecaller lookups are gated by FREE-USAGE QUOTAS on 5 accounts (not cash).
- Transcription (mai-transcribe) is the cash-gated step: **$2.33 left of a pooled
  $25 across 5 accounts**.
- ZCode offers the owner THREE FREE SESSIONS per 24h on GLM-5.3-Flash — a $0
  generative lane he was already planning to use for callgraph/graph grinding
  batches before Jev existed.

So: the "$2.97 sweep" was a HYPOTHETICAL ceiling (if all 35k were transcribed).
The REAL classify-now workload is 145 transcripts (~108k tok ≈ **$0.005** at Jev,
or ~$0.012 with rich 2k-token states). The true bottleneck chain is:
transcription budget → THEN classification. TC lookups: free-quota, scheduled.

## OWNER QUESTION 1 — "Will the free ZCode sessions be better suited here?"
Compare honestly for the CURRENT corpus (145 transcripts) and the NEAR-FUTURE
corpus (the remaining ~$2.33 buys roughly 500-600 more transcripts at current
burn): free ZCode batch sessions (3/24h, GLM-5.3-Flash, $0 cash, but session
overhead + wall-clock + occupies the owner's ZCode session slots) VS Jev
($0.005-0.012 per full sweep, ~seconds, any time, no session slot). Are they
competitors or complements? Give the division of labor.

## OWNER QUESTION 2 — SYSTEM-1 IN-CHAIN TOOL USE (the main ruling request)
"If I do end up buying Jev, I want models to use it on the fly when making a
long chain of decisions — System-1 thinking. Make a System-1 thinking line in
the yggsteer for all harnesses to not shy away from using Jev using small bash
or python scripts just like regular tool calls like cat, sed, awk."
So: during ANY session, when a model faces a small classification/routing/triage
judgment mid-chain, it should fire a 5-line Jev request (the skill's transport)
the way it would run `grep` — not switch sessions, not escalate to a consultant,
not agonize about the $0.0001. Design the STEER LINE: exact wording (it joins
sections/jev-door.md, per-turn budget — the whole door must stay under ~150
words, so the line must be tight), the guardrails that keep it honest (cache,
unknown-on-failure, never for generation/authority, book-to-ledger aggregated
not per-call), and the anti-patterns to name (using Jev for deterministic
lookups; letting a low noul silently drop an item; agonizing/asking permission
for a $0.0001 call). Also: any per-session rate/cost sanity cap so a runaway
chain cannot burn the $5 in one session?

## OWNER QUESTION 3 — revised tokenomics table
Produce the corrected money+time chart: current corpus (145 transcripts; 425→
now 10,693 unknown-peer rows metadata triage), near-future (+~550 transcripts),
and the hypothetical-full-corpus ceiling — each with Jev cost, Jev wall-time,
and the equivalent free-ZCode-session cost (sessions consumed) + wall-time.
Note metadata-only states for the 10,693 unknown rows are much cheaper than
transcript states (peer_number/duration/direction/features ≈ 200-400 tok).

Keep answers tight; this is a focused re-analysis, not a full re-architecture.
