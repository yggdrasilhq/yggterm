# sol s46b — the (i-r2) residual: fix-layer decision (L2 → L1)

- **Date:** 2026-10-15, sitting 46 unit 2 (zcode, jojo sess_69744283-36c8-4503-94c0-0b18a497e631)
- **Consultant:** gpt-6.1-sol, reasoning effort HIGH, via codex exec on dev (juju).
- **Staged raw:** /tmp/s46b-sol-consult/BRIEF.md (the measured three-layer
  residual + the four incident classes + the candidate layers). Ephemeral.
- **Graph id:** yggterm@s46b-residual-fix-layer-sol

## Verdicts

- **Q1 ACCEPT-with-ranking — L2 FIRST, then L1; L3/L4 rejected.**
  - **L2 (organic surface evidence)**: recover a current, attached surface
    observation + qualified application receipt independent of the
    captured-send Paint event. Neither paintCount nor receipt arrival alone
    may earn the witness (paint-zombie exclusion retained). The L2 gap:
    record_terminal_applied_content (state.rs:30063) advances paint only if
    paint was ALREADY pending; the re-attest probe (viewport.rs:7955) arms
    only when a content claim is parked — Missing Paint + no parked claim =
    no independent observation path. Precedent for the fresh-return shape:
    the qualified seed-proof route (viewport.rs:13454) — accepted-vs-promoted
    must stay distinct.
  - **L1 (attempt readiness)**: the exact missing completion site is
    `terminal_attach_local_ready` (viewport.rs:16346) — retains + removes
    attach_in_flight but never completes the attempt. Add a content claim at
    the validated local-ready transition carrying mount_origin_attempt_id;
    absent receipts park it (the Q7 transaction absorbs the rest). ⛔ NOT via
    observe_terminal_open_attempt_from_viewport (active-session scoped,
    mutates attempt + resume latch directly — state.rs:30439/30720).
  - **L4 rejected**: a broad retained+live-host exemption can preserve a
    paint-zombie and creates no ready history; a narrow evidence-qualified
    exemption duplicates the 31110 latch.
  - **L3 rejected**: the raise veto stays; ignoring the latched failure
    still leaves was_ever_ready=false, and broadening risks raising an
    unproven surface / bypassing recovery ownership.
- **Q2 REBUT — the ranking holds without the measurement, but the narrow
  L2 repair needs it.** CORRECTION to the seat's proposed discriminator:
  `paintCount>0` proves emitPaint RAN, not that a Paint event SENT (the
  counter advances BEFORE the nextPaintKey gate). The hook must also
  publish paintSendAttemptCount + lastPaintKey (gated
  YGGTERM_TEST_PAINT_PROVENANCE=1): terminal_paint_provenance_probe_script
  beside terminal_frame_hash_sync_script (terminal_scripts.rs:503), fresh
  document::eval + top-level return JSON.stringify, read-only (registry
  presence, appliedContentTuple, published appliedContent, paintCount,
  lastVisiblePaint*, host connectivity, rows/cols; nulls for missing),
  staged in TerminalCanvas's pre-select region near viewport.rs:7955 as one
  persistent future after js_ready while the attempt is Pending and the
  shell paint witness is absent; sample immediately + 250ms through the
  demotion + the 1500ms cancel grace; capture host/epoch/originating
  attempt and compare against current state; timeout/replaced-entry =
  UNKNOWN, never zero. The hook must not call emitPaint, redraw, write,
  mark paint, park a claim, or complete readiness. Acceptance needs a
  matching current-entry sample BEFORE the cancel. Also check the
  AppliedContent page receipt (same sendTerminalEvent pipe — repairing
  only Paint delivery can leave the qualified receipt stranded).
- **Q3 REBUT — the sticky-set augmentation ALREADY EXISTS:**
  terminal_session_was_ever_ready (state.rs:32659) already returns
  reached_ready.contains || has_ready_history, and the pin already calls
  it (viewport.rs:3940). was_ever_ready=false means NEITHER source had
  readiness — no free win. Keep the activation-epoch mechanism; earn
  readiness via L2/L1 and the existing freeze-don't-zero pin does the rest.
  Preserve fault unpinning + explicit mount/watchdog epoch changes.
- **Q4 ACCEPT — BAR-R becomes a hard gate, with the viewport condition**
  (reveal_served fires after a Decision completion, BEFORE the reveal-stamp
  eval supplies effect proof — viewport.rs:6111 — so the bar needs the
  re-selected viewport to already contain the late marker, sampled
  immediately). BAR-D must include owner-supersessions (the rig header
  claimed skips forbidden while ok_d checked only drops+resets — fixed).
  BAR-T stays currentness evidence; BAR-P stays delivery-only.
  Retain the named incident locks (0829 positive, paint-zombie negative,
  witness lifetime, Q7 transaction trio, provenance set, no-receipt-only-
  paint, valid-blank pair, raise-safety trio, pin-safety pair) + add five
  NEW behavioral locks: (1) current attached entry + matching published
  receipt with captured Paint suppressed earns paint + completes the
  intended attempt; (2) husk/detached-term/withheld-receipt/foreign-tuple/
  replacement-during-probe earn nothing; (3) receipt-before-observation and
  observation-before-receipt both converge; (4) a live legitimately silent
  loop with no application evidence stays Pending across elapsed time (no
  fabricated readiness/deadline) — distinct from the existing silent-loop
  raise test (absent heartbeat); (5) subsequent attempt churn preserves
  earned sticky readiness while genuine faults still veto.

## Ranked plan for the next session

1. Measure without changing behavior: the gated provenance probe; capture
   pre-cancel page evidence + Rust receipt/paint state on the residual.
2. Land L2: repair the measured observation gap (fresh-return qualified
   surface evidence where needed); extend re-attestation eligibility beyond
   already-parked content claims; keep one-flight + cooldown.
3. Land L1: at terminal_attach_local_ready, submit the captured-origin
   Content claim through complete_terminal_open_attempt_ready; absent
   receipts park it. Birth readiness without app-state queries.
4. Gate: BAR-R hard + unchanged loop identity + pre-reveal currentness +
   post-reveal typing; the incident locks; fence negative/blank/
   provenance/reattest.
5. Leave L3, broad L4, and the pin unchanged unless the evidence-corrected
   path still fails — then name the exact failed predicate.
