# sol s45 — (i-r2) keep-alive/loop-survival: the attack-round consult

- When: 2026-10-15 ~05:45-06:05 IST, BEFORE the grind (the s44 pattern).
- Consultant: gpt-6.1-sol, reasoning effort HIGH, transport: headless
  codex exec YOLO on dev, juju account. Tokens: 67,623.
- Staged raw data (data law): /tmp/s45-sol-consult/ on dev — loop head +
  branch guards, the exhaustion park+break, the watchdog + drop guard,
  state.rs heartbeat/death/budget/stale-watch, the park-claim state
  machine, the BackgroundTrickle policy, the spawn-identity latch, and
  the queue records (s15 narrowing, s27 owed construction, s34 liveness
  clause, s35 loop-dead filing, s35c residue). EPHEMERAL (the /tmp dir
  dies with the host); the verdicts below are the kept record.

## The five verdicts (accept|rebut|defer — correction)

- Q1 REBUT — the deliberate park+break is plausibly covered by the
  witnessed-death mechanism; the excerpts do NOT establish the full
  claim-resolution chain. The sharper unresolved edge named: whether an
  actual attach_complete_latch pending claim survives / is legitimately
  replaced across the watchdog remount and reaches current-tuple
  promotion WITHOUT user reselection.
- Q2 ACCEPT-with-scope — if measurement shows a scheduling defect,
  arm_remount on the named break; a second restart ladder is premature.
  If main already completes the watchdog chain, arm_remount is a latency
  improvement, NOT proof of a formerly missing restart (no manufactured
  RED for latency).
- Q3 REBUT — budget exhaustion is not "wait forever" (the window rolls
  from the LAST spend); the honest failure is "this recovery attempt
  failed to establish a usable viewport", never "the terminal died".
  The attempt-scoped state machine (PendingEvidence → Recovering →
  ContentReady | RecoveryStopped | Superseded) with RecoveryStopped
  VISIBLE on the surface ("Live view could not be restored
  automatically. Retry."), last frame preserved-and-stale-labeled, and
  the deadline must ALSO cover the alive-loop no-evidence case (the
  remount budget cannot bound it — no further deaths need occur).
- Q4 REBUT — punch+bootstrap recovery is measured, but does NOT satisfy
  the s15 creation-churn obligation: BackgroundTrickle's guarantee
  (buffers current BEFORE reveal) dies with the loop; recovery on
  reselect cannot deliver it. Survival stays owed unless the owner
  re-cuts the guarantee. Fix the lifetime boundary that cancels the
  retained row's task (stable component identity is the smaller
  design); do NOT detach the loop into a raw tokio task (it owns
  UI-bound eval, signals, mount identity).
- Q5 DEFER — the remote construction is specifiable (the rig design
  delivered: marker-matched hook armed-absent, established-row
  baseline, event-driven phase deadlines, the assertion chain,
  rc 0/1/2/3 vacuity semantics, the write-round-trip living-row bar,
  the falsifier = automatic recovery fails after fault removal). The
  CRITICAL TRAP it flagged: an established painted row may already
  hold a qualifying receipt, so the park creates NO pending claim —
  a run can pass while missing the parked-claim problem entirely.

## What the seat measured against the verdicts (the outcome)

- Q1/Q5: the i2-rig construction on a REAL oc-owned remote-agy row
  measured the FULL chain on main: ladder → park (latch-release — Q5's
  trap CONFIRMED: no pending claim exists on the natural path) → break →
  witnessed-death remount in 3.4-3.8s (one tick) → successor attaches
  ~30s later; a fault clearing mid-ladder heals the same loop in place.
  NO missing-restart defect (Q1's rebut CONFIRMED; the claim question
  resolves by design — begin retires, the successor re-earns).
- Q2: arm_remount landed on the break (the latency improvement sol
  scoped): measured arm-gaps 0.00s x3 on the lane vs 2.0-4.0s witnessed
  ticks on main; the LEG C law lock extended to the third class.
- Q3: the RecoveryStopped surface landed in the degraded arm
  (Error tone, "Terminal Restore Stopped — could not restore this
  row's live view automatically. Select the row to retry."), pixel-
  proven on the lane exhaust run. The alive-loop bound (Q3's extra
  demand) stays OPEN with (i-r2) in the queue record.
- Q4: survival stays OPEN as its own arm in the queue record with the
  design inference recorded (component-identity route first).
