# s47 sol consult — the paint-provenance measurement freezes the L2/L1 shapes

- **When / who:** 2026-10-15 ~12:1x IST, zcode jojo sess_77e0f9df; consultant
  gpt-6.1-sol HIGH via codex exec on dev (juju), 64,950 tokens. Read all four
  staged files from disk (BRIEF, measurement.md, trace-slice.jsonl,
  code-excerpts.txt — RAW per the data law). Staged at /tmp/s47-sol-consult/
  on dev (ephemeral).
- **Context:** the (i-r2) residual's owed measurement (s46b ranked plan).
  Lane lane/i2/paint-provenance 95339388 (probe + i2prov-rig) on main
  1d018bf8f875.

## THE MEASUREMENT (what sol judged)

Row a (app-control birth, organic cold mount): paintSendAttemptCount hit 3
(the kind:"paint" send IS attempted page-side through the nextPaintKey gate,
key [3,…]↔[4,…]); a qualified receipt (wrote=349, blank=false, gen=1) sat in
the page entry by ~mount+2s; yet ZERO paint_ready / first-paint-witness
events — the TerminalJsEvent::Paint arm never ran; the re-attest probe never
armed (no parked claim); the churn's skip-cancel latched the Pending birth
attempt FAILURE (host_has_painted false); re-select refused
(was_ever_ready false) and full-bootstrapped (bootstrap_owner_superseded_
during_loop, ln 550), growing a second loop with fresh closures. The shed
is page→Rust DELIVERY (the F1 dead-IPC-leg class), not the page-side gate.

## VERDICTS (all applied in the same sitting)

- **Q1 ACCEPT-correction (L2):** drop the parked-claim prerequisite — probe
  while evidence/witness missing OR a claim parked, loop-whole-life, NO
  mount-age or attempt-state boundary; keep js_ready + one-in-flight + 2s
  cooldown. state.rs earns the witness on the VALIDATED receipt without
  paint_pending pre-arm, idempotent, pending marker cleared as cleanup.
  ⭐ RECEIPT PRESENCE ALONE INSUFFICIENT: at ln 646/648 entry_live:true +
  was_husk:true + a gen-3 receipt COEXIST (teardown transient) — the probe
  ingress must require was_husk === false (absent field skips the sample;
  strictness costs one 2s round, never liveness). ⭐ Do NOT require
  lastVisiblePaint (background recovery must work without a current visible
  paint).
- **Q2 ACCEPT (five locks):** (a) paint-suppressed organic earn — witness
  earns with zero Paint delivery; (b) husk/detached/withheld/foreign/
  replacement earn nothing — ingress ownership checks + validation +
  async-commit revalidation, explicitly covering receipt-plus-husk;
  (c) receipt-first and claim-first converge on the same completion,
  preserving the parked origin; (d) a legitimately silent live loop stays
  Pending — silence is not a fault; (e) attempt churn preserves earned
  witness + sticky readiness while genuine faults still veto.
- **Q3 ACCEPT-correction (L1):** L1 is the PRIMARY attempt-completion fix;
  L2 repairs witness/evidence liveness; BOTH required. terminal_attach_
  local_ready submits the Content claim (captured origin BEFORE the async
  work, never the latest at callback; NOT via observe_terminal_open_
  attempt_from_viewport); attach-time submission is safe for the ACTIVE
  row because absent receipts PARK. Preserve the cancel-latch fallback and
  its paint requirement; preserve the (e-r8) origin check during parked
  replay.
- **Q4 ACCEPT-correction (bars):** BAR-R hard = pre-re-select earned
  readiness + live loop; fast-raise branch with reveal_served and ZERO
  reveal_raise_refused; AFTER reveal_served a fresh interaction/output
  effect check (no stale retained frame); same loop owner through effect +
  observation. BAR-D = zero target-row supersessions/drops/resets/
  replacement-loop starts from pre-churn baseline through settle; scope to
  the retained target (b's legit bootstrap and a's ln-119 cold-mount
  refusal excluded). Partition samples by loop/probe INCARNATION (the
  sample-counter restart is the boundary); second-loop samples are
  supersession evidence, never corruption, never a green. On the residual
  build ln 550 is a REAL BAR-D failure — report it as the causal failure.
- **Q5 REBUT:** the shed Paint events stay a measured subcase under F1(c)
  (no separate member): sol's queue wording preserved in the queue record.
  Do not call it cosmetic without auditing the Paint arm's remaining
  consumers.
- **Data correction (applied):** the rig's driver read raced 2 final
  samples — the raw slice holds 36 a-samples (24+12), not 34; no verdict
  changes.

## APPLIED AS

lane/i2/paint-provenance commits: 95339388 (probe+rig) → L2 (organic earn +
claim-independent arming + husk gate + six locks) → L1 (attach-time Content
claim) → i2c-rig BAR-R/BAR-D hard revisions → queue record. Bars + suites
on the lane build; fences order/dup/swap.
