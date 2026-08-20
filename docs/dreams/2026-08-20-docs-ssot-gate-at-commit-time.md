# Dream: the docs-ssot gate must fire at commit time, not at the next reader

**Routed from lane 11.6 (provision), 2026-08-20. Owner: the yggterm campaign (repo tooling).**

## The instance

At the 11.x wave's baseline the enforced docs-ssot check was RED on main: three
`docs/pending-bugs.md` entries carried annotated `**Status:** OPEN — <qualifier>` lines, and the
checker's vocabulary requires the Status line to be exactly one term. Every lane inherited a red
shared gate it did not break, and the first lane to notice paid the cleanup (the qualifiers were
real information, so they moved to the line below rather than being dropped).

## The shape

`scripts/check-docs-ssot.sh` runs as a cargo test (`docs_ssot::the_bug_file_lists_only_open_items`)
— which a docs-only commit never runs. So the gate is enforced at the *next* reader's expense, not
at the author's. That is the stale-doc asymmetry again: the person who could cheaply fix it is
never the person who pays.

## The want

A pre-commit hook (repo-tracked, installed by the same mechanism as the privacy guard, NOT a third
hand-copied installer — see the pending-bugs entry on the hook installer existing twice) that runs
`scripts/check-docs-ssot.sh` when the commit touches any file the check governs, and refuses with
the checker's own message. Cheap (<1 s), no network, and it makes a red gate unlandable instead of
inheritable.

## Constraints learned the hard way

- The hook installer has two disagreeing copies and one crashes on a worktree (pending-bugs) —
  fix or bypass that FIRST; do not add a third copy.
- Lanes commit from 17+ checkouts across 3 hosts; an uninstalled hook protects nothing. Wire the
  installation into a path every checkout already runs (the fleet binary sync or the claim script),
  or use `core.hooksPath` pointed at a repo-tracked dir so a checkout is protected by checkout.

## A SECOND INSTANCE, AND IT IS THE HARDER HALF (added by 11.0, 2026-08-20)

A fixed entry came back. `5f661ddd` deleted the `[11.0]` seat-derivation entry in the same commit
as its verified fix — the contract this file's own law demands. Two commits later a lane's sweep,
based on a checkout that predated it, **re-added the entry verbatim**. It sat on main for hours
marked `**Status:** OPEN`.

⛔ **The gate above would not have caught it.** The resurrected entry's format was perfect: one
heading, one valid Status line, correct vocabulary. `check-docs-ssot.sh` enforces SHAPE, and this
was a defect of LIVENESS — a closed question re-advertised as open.

⚠ **And a resurrected entry is worse than a merely stale one, because it recruits witnesses.** A
lane reading the queue found the entry, matched it against a mis-seat it had experienced BEFORE
the fix landed, and reported the defect as firing again — citing, as its evidence, the entry its
own commit had restored. Nothing in that chain was careless: the entry was on main, the symptom
was real, the reasoning followed. **A resurrected entry is indistinguishable from a never-fixed
one by reading, which is the only instrument a queue offers.**

⇒ The gate wants a liveness question alongside the format one: *has this heading been deleted by
an earlier commit on this branch?* `git log --diff-filter=D -S'<heading>'` answers it cheaply, and
a re-addition is either a mistake or wants an explicit note saying the fix regressed. Same family
as the privacy guard reading a deletion as an addition — a path-level view of history cannot see
which direction a line moved.
