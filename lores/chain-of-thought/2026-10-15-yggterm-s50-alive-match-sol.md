# 2026-10-15 — yggterm s50 — the warm-gate alive-match false convictions (gpt-6.1-sol HIGH, juju)

Brief: /tmp/s50-sol-brief.md (raw s47 trace rows + both dispatch drivers +
the poll script + the gate branch + the constants + the hypothesis and five
questions). Run from dev, juju codex, 31,620 tokens. Plan ACK-8175a8718c.

## Verdicts

- **Q1 accept** — the payload (A: attempt=1/dispatched=2/stage=posted; B:
  attempt=2/dispatched=2) proves a mismatch between identity scopes; the
  comparator necessarily rejected A despite its posted record. CAVEAT: the
  trace does not uniquely attribute WHO wrote dispatched=2 (A's own newer
  dispatch with a stale posted record is the alternative reading). Narrow
  the claim to the identity-scope mismatch; cross-host attribution is
  strongly supported, not proven per-row.
- **Q2 accept** — per-host dispatch bookkeeping beside BOTH dispatch ops;
  poll compares within the same host scope. Keep globally unique attempt
  numbers + the legacy globals; ONE attempt value feeds map + invocation
  with no intervening await. Missing map entry = unverified liveness
  (record-absent recovery), NEVER a page-wide fallback. A present entry
  disagreeing with the record still rejects an older same-host record. No
  fn-version bump PROVIDED the writes ride the driver scripts (outside the
  cached function body) — an already-cached fn receives the bookkeeping on
  its next invocation. The lock test MUST demand both map writes + the
  scoped poll lookup; keeping only the old two-global-write assertion
  would protect the defect.
- **Q3 defer** — the entry supersede guard stays page-wide (same-host redo
  supersession is its job). Describe the patch as fixing FALSE LIVENESS
  CONVICTIONS, not all concurrent-mount supersession. If the differential
  exposes an entry abort preventing completion, that becomes a completion
  blocker (function-body repair + version bump).
- **Q4 accept with conditions** — the fence-order differential is the right
  bar IF: overlap is made explicit (A posted → B dispatches → A's poll);
  eligible overlapping mounts AND completed polls are counted alongside
  convictions (zero convictions with fewer mounts/missing polls is
  inconclusive); "false" is defined by dispatch ownership + attempt
  history, not by banning the decision name (legitimate same-host
  mismatches still justify pipeline_alive_eval_lost); recovery controls
  preserved (genuinely missing execution and same-host supersession must
  still recover; stall_cap/alive_at_cap allowed on their own evidence).
  Fix claim additionally needs the affected rows' rendered terminal output
  + continued delivery after warm completion.
- **Q5 accept** — an accidental redo could be load-bearing; a posted record
  proves page-side execution, NOT that Rust received readiness or that
  delivery ran. Post-fix, A enters the resend/synthesis path — verify it
  targets A's CURRENT attempt (`__yggtermMountResend` is page-wide; taking
  hostId does not alone prove its implementation dispatches correctly —
  measured here: it is a per-instance closure guarding `h === hostId`, and
  the Rust synthesis consumes the poll verdict's readyRecord, not the
  resend). Bar: current-attempt readiness acceptance, retained output
  rendered, fresh output delivered, no switch away/back; include the
  lost-first-ready control (the [11.217] synthesis path).

## Applied

Lane lane/f1/alive-match-keying: per-host `__yggtermMountDispatchedByHost`
map written by both dispatch shapes (warm invoke + cold tail), poll matches
within the host scope (no page-wide fallback), locks updated (source-form
counts for format!-doubled braces), sol node in-repo. Differential per Q4.
