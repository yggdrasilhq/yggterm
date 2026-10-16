# s53 unit B sol consult — the app render's snapshot cost (2026-10-16)

Seat: zcode jojo sess_a4ee4199 (sitting 53). Transport: sol gpt-6.1-sol
HIGH via codex exec YOLO on dev, staged raw tree /tmp/s53-sol-brief
(BRIEF.md + 5 .rs stages), session 01a14294-59ec-7273-afaa-b5055ed518da.
Trigger: the campaign's Q3-rank-1 fix unit (snapshot sharing) is
architecture-sensitive; consult before grind per the charter law.

CONTEXT: unit B's section spans measured the app root render at ~60ms/
pass on the dev harness with app.snapshot (ShellState::snapshot_shared)
at ~47ms mean = 78-80%, app.rsx 0.2ms, children sub-9ms. Brief asked
how to cut the per-click snapshot cost and what the honest falsifier is.

VERDICTS:
- Q1 ACCEPT (provisional rank c→a→b→d): (c) split cache-lookup /
  construction / cache-replacement and PROFILE construction's internals
  (live-session conversion, overlays, search-context install,
  retained-session processing) FIRST; then (a) cache expensive
  projections by complete dependency revisions; (b) structural sharing
  supports that but wrapping NEWLY-REBUILT vectors in Arc saves no
  reconstruction work (KILLED as a standalone fix); (d) write
  coalescing helps totals but six write sites do not establish six
  preventable renders and leaves the cold snapshot expensive. Caveat:
  staged snapshot_fn.rs ended mid-loop, struct truncated — definitive
  hotspot ranking unavailable from the brief.
- Q2 ACCEPT: invariant = dependency-complete freshness + cross-field
  consistency on the FIRST rendered frame after each committed change;
  selection/activation must recompute dependent projections
  immediately; published sections immutable; changed output must reach
  child props. TEST: `first_switch_paint_matches_uncached_snapshot` —
  warm cache + controlled clock, change selection alongside row
  add/remove + runtime status + expansion + search; first shared result
  must equal fresh construction; no TTL wait or corrective second paint
  may rescue it.
- Q3 ACCEPT: time-dependent invalidation (the 500ms TTL / busy-hint
  inputs) stays independent of data revisions; 500ms is checked on
  ACCESS, not an auto-refresh deadline; the modal-bit exception is
  deliberate — preserve unless changing deliberately; refreshing an
  unrelated section must not reset the age of unrecomputed busy data.
- Q4 REBUT: section mean alone is an insufficient falsifier. Require:
  materially lower TOTAL snapshot+root-render time per matched switch,
  click-to-reveal latency including the tail, hit/miss/build/root-pass
  counts, snapshot work in EFFECTS as well as the root, predeclared
  numerical thresholds; lightpass bars as the no-regression control.
- Q5 REBUT (adversarial — the load-bearing one): the 47ms is a
  CALL-BOUNDARY cost, not yet "merge construction". The span includes
  state.read() bookkeeping, cache handling, allocation, and possibly
  the DESTRUCTION of the replaced Arc (the old snapshot's drop is
  charged to the span when the replacement takes the last reference).
  Attribution accounting (nested spans, window boundaries, denominators)
  unverified; effects call snapshot_shared() separately (misses outside
  the section). NEXT SITTING'S FIRST MOVE: split the span
  (lookup/construction/replacement+drop) + hit/miss reason counters +
  verify attribution accounting BEFORE prescribing a cloning fix.

WHAT WAS APPLIED: nothing code-side this sitting beyond the section
spans already landed (82e35573); the verdicts reshape the next unit's
attack order (Q5's split-span instrument first, then Q1(c) profiling,
then the projection cache; the falsifier per Q4; the correctness test
per Q2).
