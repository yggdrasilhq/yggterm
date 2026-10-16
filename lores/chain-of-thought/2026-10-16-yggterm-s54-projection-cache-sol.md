# 2026-10-16 — yggterm s54 → s55: the browser projection cache (sol HIGH)

Consult: gpt-6.1-sol HIGH via codex exec (juju), 84,648 tokens, session
01a142f2-a5c0-7c62-8137-8e1e8f261993. Staged raw: /tmp/s54-sol-consult/
(BRIEF + browser.rs selections + snapshot callers + s54 measurements;
dev, ephemeral). Raw output: /tmp/s54-sol-out.md.

Context: sol s53's own Q4/Q5 rebut demanded the app.snapshot split BEFORE
any fix prescription. s54 delivered the split + a 7-phase profile; the
growing slice measured out as the browser whole-tree walk (all_rows()
re-run per merge; BrowserMetrics last_rebuild_ms=53.6 at row_count=7 —
NOTE: that metric covers rebuild_rows, not the all_rows walk; sol's
correction kept the record honest). This consult decided the cache shape.

## VERDICTS (all accept-with-corrections)

Q1 CACHE SHAPE: (a) lazy per-instance cache — `OnceLock<Arc<Vec<BrowserRow>>>`
on SessionBrowserState, first all_rows() initializes synchronously, return
Arc::clone. Short-ids/group-paths stay initializer temporaries (no proven
consumer). Every new construction starts empty; clones may share. NEVER
TTL expiry, async population, previous-instance fallback, or second-paint
repair. NEVER call clone-from-cache equivalent (it still copies rows).

Q2 INVALIDATION SAFETY: the source-scan invariant gate, strengthened —
constructor-only root init; reject subtree mutation, mutable borrows, and
mem::replace (assignment spelling alone insufficient); the guard MUST
demonstrably fail on representative shapes; any new in-place root
mutation changes the cache contract in the same change.

Q3 CALLERS: borrow through the Arc; "zero per-merge cost" means no
traversal/row-clone (refcount ops remain). The two fixtures using
.into_iter() move to .iter().find().cloned(); NEVER .to_vec() per caller;
mutable-need callers clone explicitly.

Q4 FALSIFIER (the protocol, predeclared): ≥3 matched control/candidate
trials on the dev harness; each ≥55 windows, ≥100 warm builds, ≥3 browser
replacements; control median all_rows ≥50ms or the trial is INCONCLUSIVE;
EXACTLY ONE initialization per demanded browser instance across all
callers; every warm search-inactive snapshot.p.browser ≤2.0ms; median
warm access ≥95% below control; no outlier removal; real build activity
retained (no 0.0-rounding substitutes); ≥100 first-switch paints with
ZERO mismatches across warm-A→replace-B→switch, add/remove, same-count
changes, short-ID collisions, collapse/expand, search; search-active
reported separately (its search_rows() residual is honest, never silently
excluded).

Q5 SCOPE: identity-based merge keys and TTL policy DEFERRED to separate
units. Follow-up ranking: (1) per-input key costs + projection content
fingerprint, (2) TTL audit (busy-hint expiry depends on it — NEVER remove
without a proven replacement freshness mechanism), (3) reprofile the
residual merge. NEVER a bare pointer as durable identity.

## What the seat applied this sitting
Q1-Q3 + the Q2 gate + the exact-one counter landed as
lane/f7/browser-projection-cache (dae5e210). Q4's full protocol is the
post-landing falsifier; the harness run this sitting covers the warm bar,
the init count, and a first-paint spot check — the 100-paint matrix is
OWED and recorded as such.
