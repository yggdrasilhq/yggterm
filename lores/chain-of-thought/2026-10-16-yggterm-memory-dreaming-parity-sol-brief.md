# CONSULT — yggterm memory "dreaming": bring it to parity with OpenAI's ChatGPT dreaming (2026-06)

You are sol HIGH, the reasoning/instruct seat (owner ruling 2026-10-06: pattern
compaction uses sol/GLM, NEVER gemini; gemini is only an optional divergent pass).

## The owner's ask (verbatim intent)
"I need the auto dreaming. I want agents to have an idea about time and contexts
and about me. We implemented the NOW system and dreaming not long ago. Give a
holistic report and fix all memory issues. Bring the memory auto-dreaming
architecture and dreaming on par (can be implementationally different) with
OpenAI's ChatGPT 'Dreaming' memory (openai.com/index/chatgpt-memory-dreaming/)."

## What EXISTS today (measured, 2026-10-16)

### The dream pipeline (ygg-memory.py, landed 2026-10-06, slices 1/1.1/1.2/1.3+2)
- `ygg-memory dream --ns=<ns> --prepare` : collects journal deltas since the
  per-ns dream watermark (new sitting archives, door §STATE/§HANDOVERS, ACK'd
  board posts, ci.log tail; NEVER NOW.md or dreamer-origin artifacts) → stages
  pending/BRIEF.md + MANIFEST.json under a byte budget (DREAM_INPUT_BUDGET_BYTES).
- `--run` : composer pass (codex exec, model gpt-6.1-sol, effort high) on the
  brief → RESPONSE.md; then a CRITIC pass; then apply.
- Composer must emit EXACTLY: NOW.MD (REPLACE, ≤2600B, 4 sections
  §CURRENT_STATUS/§NEXT_UNITS/§ACTIVE_TRAPS/§POINTERS, every claim stamped
  [as-of: DATE; proof: deployed|suite|measured-live|claimed]); OPS (≤3 narrow
  ops: append-section, propose-tombstone — NEVER rewrite bodies or indexes);
  CLAIMS (per-claim id/text/as_of/proof/source_refs); OWNER-NOW (≤1100B
  owner fast-rotting context: mood/energy, location/travel, commitments,
  working hours, comms shape — each line [as-of: DATE], or null).
- 4 GATES: (1) delta gate; (2) cooldown (DREAM_AUTO_COOLDOWN_S=2h);
  (3) QUIESCENCE (DREAM_QUIESCENCE_SECONDS=15min: defer if any non-dream
  journal write in the window — the dreamer never competes with a writer);
  (4) READ-VERSION FENCE (if the journal advanced past the prepare watermark
  during compose, the apply is REFUSED — re-prepare).
- `--auto` : auto-dreamer, picks ONE hungriest namespace (delta-sorted),
  config-gated in the tick (armed only where ~/.yggterm/config/dream/enabled
  exists — dev), 1h backoff on composer failure.
- Ownership partition: seats own their campaign door §STATE; the dreamer owns
  ns-level NOW.md ONLY. Seats write raw memories, never consolidate mid-task.
- CUT in round-4 (do NOT rebuild without a new owner ruling): H_vocab phase
  telemetry, Hebbian decay, 3-strike abstraction engine.
- Steer law: session start reads NOW.md FIRST; boards are conversation
  (dreamer input); post ≠ law.

### Observed health (this sweep, 2026-10-16)
- The auto-dreamer was DEAD 2026-10-06 → 2026-10-16: `--ns <ns>` (separate
  argv) made argparse eat the dash-leading namespace value ("expected one
  argument"); every tick backoff-stamped. FIXED today (= form). 14,169 journal
  deltas had accrued unconsolidated.
- A second defect churned the journal: zcode's ARCHIVE-INDEX.md was ingested as
  a native door by an older build, then two zcode project stores rewrote each
  other's identical-named door 11,534 times (each rewrite advanced the
  read-version fence → dream applies refused). Door retired; builds aligned.
- NOW.md for -home-pi-gh-yggterm is from 2026-10-06 (10 days stale).

## OpenAI's ChatGPT "Dreaming" (June 2026) — the parity target
Sources: openai.com/index/chatgpt-memory-dreaming/.
- Three generations: 2024 Saved memories (explicit "remember X", strong cues,
  goes stale); 2025 Saved memories + Dreaming V0 (background process curates
  memory from many conversations, supplements saved memories, always freshest);
  2026 Dreaming V3 (a significantly more capable, compute-efficient memory
  architecture BUILT ON dreaming — dreaming is now the primary system).
- Three evaluation objectives: (1) CARRY FORWARD useful context; (2) FOLLOW
  preferences/constraints; (3) STAY CURRENT OVER TIME (temporal decay: "planning
  birthday for next Saturday" → Sunday arrives; the memory must age).
- The synthesized memory is REVIEWABLE: a memory summary page; user can add/
  update, correct, or dismiss specific details, and give instructions on what
  topics to surface and when. Drill-down by chatting with the model.
- Background synthesis across many conversations; freshness/correctness/
  scalability at hundreds of millions of users / multi-year horizons.

## QUESTIONS (answer each with accept / accept-with-corrections / reject + reasons)

Q1 PARITY GAP ANALYSIS. Against the three objectives (carry-forward,
preferences/constraints, stay-current) and against OpenAI's "reviewable
summary + user corrections + surfacing instructions", what does yggterm's dream
lack? Rank the gaps by owner-visible impact.

Q2 TIME & CONTEXT AWARENESS. The owner explicitly wants "agents to have an idea
about time and contexts and about me." Today's mechanisms: per-claim
[as-of: DATE; proof:] stamps; §STATE/§HANDOVERS (hours strata); NOW.md (days
strata); lore (weeks-months). Gaps: (a) there is no "today" anchor — a seat
reads NOW.md but not how OLD each claim is relative to NOW; (b) the freshness
§FRESHNESS design (retrieval LAG vs semantic ROT; contradiction-density
invalidation) is partly unbuilt. What is the MINIMUM addition that makes a
fresh session genuinely time-aware (know the date, know claim ages, know what
rotted) without token bloat? Consider a computed age prefix per NOW.md claim
and a "stale-by" section for the composer.

Q3 OWNER MODEL ("about me"). owner-now.md exists (≤1KB, fast-rotting). What
must be added so the owner model is (a) always current, (b) correctable by the
owner cheaply, (c) rich enough to stop agents re-deriving him? OpenAI exposes a
reviewable summary; yggterm has only the file. Is an owner-facing review
surface (a rendered owner-now + recent NOW deltas the owner can correct via a
board post or a single file edit) the right shape, and what is the minimal
version?

Q4 AUTO-DREAM RELIABILITY. This sweep found the auto-dreamer silently dead 10
days. What DETERMINISTIC liveness law should exist so "the dreamer stopped" is
always visible within one tick (the canary covers index size / archive pointer /
NOW size but did NOT catch a dead dreamer)? Also: now that 14k deltas are
backlogged, is a bounded catch-up pass correct, or must the tail stay small by
design (the freshness-law says keep the unabsorbed tail SMALL)?

Q5 SCHEMA & SCALE. The composer brief feeds RAW deltas (byte-budgeted). At
14k-delta backlogs and multi-year horizons, what changes (delta summarization
before compose? hierarchical NOW? per-campaign sub-NOWs?) to stay both cheap
and fresh — implementationally different from OpenAI is fine, but the
objectives must match.

Q6 WHAT TO BUILD NEXT. Give a ranked, minimal, falsifiable slice list (max 4)
that takes yggterm's dream to parity on the three objectives + owner
time-awareness, reusing existing machinery where possible. For each: what it
is, what it changes, how we'd know it works (effect-level proof), and the
cheapest falsifier that would kill it.
