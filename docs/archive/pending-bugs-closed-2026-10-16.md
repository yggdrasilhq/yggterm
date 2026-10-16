# pending-bugs closed 2026-10-16

Closed narratives moved out of the queue in the same commit (docs-ssot law:
a fixed entry is deleted; git remembers it). Nothing here is lost.

## ⛔ [11.230] DELETING A MERGED LANE'S REMOTE BRANCH WEDGES ITS CI SUBSCRIPTION FOREVER — THE WATCHER COUNTS A SUB WHOSE BRANCH IS GONE AS A CONFLICT AND EVERY TICK REPORTS merged=0 (measured live 2026-10-05 11:20-11:45 IST, five stale subs, conflicts=5 × six ticks)

**Status:** CLOSED 2026-10-16 (sitting 57, zcode sess_43c12a73; fix
lane lane/ci/11230-sub-expiry 9b80a605 → main e2d03908, CI-deployed
22:32:52 IST, ls-remote read-back)

Filed 2026-10-05 ~12:5x IST by the zcode seat (board ACK-9b24b841bf).
The mechanism: the worktree law deletes a lane's remote branch after its
merge; the watcher's merge step then cannot resolve the branch (git
ls-remote answers nothing), the failure is booked as a hunk conflict, and
the sub file is never consumed — every subsequent tick reports
merged=0 conflicts=N while REAL lanes (the [11.227] guard among them) sit
stuck behind the wedge. The auto-unsubscribe's "already in main" check
also needs the branch to exist to compare, so it never fires. Repaired by
hand this sitting (five sub files removed after verifying each
tip_at_enlist's content was in main). FIX SHAPE: when the sub's branch is
absent from origin, fall back to checking tip_at_enlist ancestry against
origin/main — ancestor ⇒ auto-unsubscribe ("merged under a rebase"),
non-ancestor ⇒ drop the sub with an EXPIRED notice (a deleted unmerged
branch is an abandoned lane, not a retryable merge). FALSIFIER: merge a
lane, delete its remote branch, watch the next tick consume the sub
instead of reporting conflicts.

CLOSED 2026-10-16 (sitting 57, zcode sess_43c12a73; board
ACK-db560c0146 plan → outcome): the fix code landed 2026-10-05
(merge-loop arm c042909e + pre-dirty sweep) and closed here after a
two-round falsification that also killed the two holes triage found:
(1) the sweep's arms both required a non-empty tip_at_enlist, so a
branch-deleted sub with NO tip survived as a "no-remote-branch"
conflict every tick — the original wedge in a new costume — now the
sweep's third arm consumes it EXPIRED; (2) the merge-loop arm's
drop_sub=True conflict flag was dead code (nothing consumed it, so an
abandoned lane would be re-reported every tick) —
_consume_expired_conflicts now unlinks the sub post-loop, the conflict
stays in the record for accounting. The EXPIRED notice the entry
declared is a sub_expired_abandoned event on the CI events plane plus
the ci.log line. PROOF: three live production consumptions on
2026-10-16 (ci.log 07:24 lane/docs/s53-f7-frame-full, 09:51
lane/docs/s54-profile, 10:43 lane/docs/s55-cache-lore); controlled
falsifier round 1 on pre-patch main (synthetic subs through the live
tick 22:21:21 IST — ancestor-true tip consumed, non-ancestor tip
consumed as abandoned, both sub files gone, tick clean, no build);
round 2 on the patched watcher (22:40:11 IST — the seat's own
just-merged lane consumed by arm A and a no-tip synthetic consumed by
the new third arm, sub_expired_abandoned event in events.jsonl). The
sweep is extracted into _sweep_branch_deleted_subs (dispositions ride
the tick record's swept field) and locked by
tests/test_yggci_branch_deleted_sweep.py (12 checks); sibling ygg-ci
suites green. Watcher restarted between ticks (pid 260221) to load the
patched bytes.
