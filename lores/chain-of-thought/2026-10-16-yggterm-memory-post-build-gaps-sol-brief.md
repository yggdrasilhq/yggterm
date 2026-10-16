# CONSULT — yggterm dreaming: post-build gap review + three owner design questions (sol HIGH)

You are sol HIGH (reasoning/instruct; owner ruling: pattern compaction uses sol/GLM,
never gemini).

## Part A — what was BUILT since your last consult (2026-10-16, same day)

You previously gave 4 ranked slices. They are now IMPLEMENTED and tested
(yggterm main 30e47e59, live on jojo/dev/oc):

**Slice 1 — observable, bounded progress.**
- `_dream_receipt()`: every auto-dream invocation appends a bounded receipt
  (last 64) to `dream/receipts.jsonl` with outcome ∈ idle|deferred|running|
  failed|applied + ns + reason.
- Standing backoff is now a FAILURE STATE: `canary` reports it as a violation
  ("standing FAILURE backoff (Ns left: reason)") and never treats it as healthy.
- Canary gained two INDEPENDENT checks (the dreamer cannot report its own
  absence): (a) execution liveness — no receipt for >6h while a namespace has
  pending work ⇒ violation; (b) synthesis progress — pending deltas older than
  6h ⇒ "progress deadline BLOWN (degraded freshness)".
- Fairness: candidate score = delta × (1 + min(oldest_pending_hours,24)/6), so a
  starving small namespace can outrank a churny big one (was pure-delta sort).
- Falsifier run: kill controller ⇒ canary red; injected backoff ⇒ red; aged
  pending ⇒ red. All three fired. PASS.

**Slice 2 — time-aware delivery.**
- Persistent claim state `dream/<ns>/claims.json`: per-claim `text, as_of,
  proof, last_seen, source_refs` + optional `valid_from, valid_until,
  review_after, supersedes, superseded_by`.
- `merge_claims()`: evidence date is STICKY (a restatement never re-dates
  evidence); supersession edges preserved; optional temporal fields validated
  (regex YYYY-MM-DD) and dropped if malformed — never invented.
- `temporal_envelope()` computes AT READ: `Now: <date>`, `Synthesis: Nd old
  (wm seq)`, claim counts, and per-claim EXPIRED / REVIEW DUE / SUPERSEDED
  lines. Nothing stored — zero composer tokens maintain ages.
- New verb `ygg-memory now --ns=<ns>`: the session-start orientation read =
  NOW.md + the envelope.
- Trigger: `work_due = new_source_delta OR temporal_deadline_due` — a
  namespace with expired/due claims and ZERO new deltas still becomes a
  candidate (delta floor of 1).
- Falsifier run: advanced a fake clock with ZERO journal writes; the expired
  claim was flagged EXPIRED and `temporal_work_due` returned True. PASS.

**Slice 3 — correctable owner context.**
- `_global/durable-owner.md` (durable layer: preferences, constraints,
  practices, surfacing instructions — NOT fast-rotting) as distinct from
  `owner-now.md` (short validity).
- `owner/corrections.json` — authoritative owner record, append-only:
  `owner --correct/--dismiss/--surface` (with --target and --text).
- `apply_owner_filters()` runs at READ time: dismissed content is stripped
  line-by-line from NOW.md and durable-owner on every read, and surfacing
  instructions appended. Effects land WITHOUT quiescence, cooldown, or
  composition.
- Falsifier run: dismissal applied with composition DISABLED; then a replayed
  old archive (same NOW.md content) was ALSO filtered ⇒ resurrection blocked.
  PASS.

**Slice 4 — evidence-preserving compaction.**
- Coverage manifest in MANIFEST.json: per-event disposition
  (represented|deferred + why). Every delta accounted.
- Retained CLAIMS injected into the composer BRIEF (evidence-linked, with
  supersession edges) so the composer reuses state instead of relearning from
  the newest batch. Previous NOW prose stays context, never evidence.
- Replay test: rare fact placed in an EARLY batch, decisive correction placed
  ACROSS the batch boundary — both survived into the brief, supersession edge
  visible, coverage complete. PASS.

**Also fixed this day (pre-existing defects):** the auto-dreamer had been dead
10 days on a `--ns <ns>` argv bug (argparse ate the dash-leading ns); a zcode
ARCHIVE-INDEX door flapped 11,534 journal records advancing the fence. Both
root-caused and fixed. The 10-day backlog (~14.2k deltas) is now being
consolidated; the RUN currently trips the read-version fence because fleet syncs
land during the multi-minute compose — a known policy tension you flagged.

## Part B — the owner's three design questions (answer each decisively)

**Q-A. "Instant forwarding": should ygg-memory auto-forward NOW contexts and
diffs INSTANTANEOUSLY to all root context files every time it detects a memory
change? Is that useful or vanity?**
Consider: root context files are the per-harness steer files (~/.zcode/AGENTS.md,
~/.pi/agent/AGENTS.md, etc.) stamped by yggsteer. Today `sync-harness` already
delivers `pinned/yggterm/` doors and the managed memory block. The proposal adds
push-on-change of NOW.md + recent diffs into every harness root file. Concerns I
see: (1) root files are injected into EVERY session — volatile content there is
per-turn token inflation fleet-wide; (2) yggsteer owns those files (foreign
guards) — a memory push could fight the stamping tool; (3) NOW.md is the
dreamer's REPLACE-semantics rollup — pushing it instantly means every session
reads a half-consolidated state; (4) the owner's own steer law says "no volatile
state in steer (fleet-wide per-turn token inflation)". Is there a NON-vanity
version (e.g. a single small pointer line + mtime, or push only the envelope
SUMMARY), or is pull-at-session-start (the `now` verb) strictly better? Rule.

**Q-B. "Checkpoint consolidation": at certain checkpoints, should ygg-memory call
sol MEDIUM to consolidate/organize memory based on context + lobe-manager +
boards + graphs?**
Today the dream composer is sol HIGH full-context. The proposal: periodic
cheaper passes (sol medium) that consume the broader graph state (lobe-manager
agenda, msgGraph boards, the lobes' graphs) and reorganize memory structure
(not just NOW). Assess: is a second, cheaper, structurally-focused pass worth it
vs the existing composer+critic? What should be its INPUT (boards/graphs are not
journal deltas — ingestion isolation?), its OUTPUT (what may it write without
violating the ownership partition?), its cadence, and its guardrails against
reorganizing away still-true facts? If yes, give the minimal design.

**Q-C. "Decision models": should the ygg harness use a System-1 decision model
(OpenAI Decisions API on gpt-6-luna: predicate/choice/score, $0.10 per 1M input
tokens, no output charge, ~10x faster; OR TypeSafe Jev 1.13: noul/choice/score,
$0.042/M in, 70-500ms, RLCD-trained) to fast-organize and classify lobe-manager
graphs/boards/memories? Do System-1 decision models have a place in our harness
at all?**
Concretely: our hot paths that are really DECISIONS, not generation —
(a) dreamer candidate selection (which namespace is hungriest), (b) claim
triage (is this claim superseded/expired/contested), (c) board/graph
classification and routing (which lobe/board/campaign owns this), (d) the
critic's verdicts (accept/contest per claim), (e) gauge/relay decisions. For
each: is a decision model a better fit than sol-HIGH, sol-MEDIUM, or
deterministic code? Note the ownership law (must classify, not write), the
"verify before you relay" law (a calibrated probability is evidence, not
authority), and cost/latency. Give a decision per use-case + a recommendation on
whether to adopt one of these (and which) or neither, and the cheapest way to
FALSIFY the choice.

**Q-D. Remaining gaps** — after slices 1-4, what is STILL missing for full
parity with OpenAI's dreaming (their three objectives + reviewable/correctable
memory at scale)? Rank what remains (max 4) and say which are truly needed vs
nice-to-have. Be specific about what would break at multi-year scale.
