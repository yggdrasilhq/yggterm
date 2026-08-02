# The docs SSOT law — who owns which question

**Why this file exists.** On 2026-08-02 an agent reported five open ychrome bugs
that the user had already fixed, argued from a bug file that was a day stale,
and burned a chunk of a session on it. The bug file was stale because the
commits that closed those bugs had been orphaned by a history rewrite — but the
*reason nobody noticed* is that three different files all claimed to answer
"what is open", and no two agreed. Duplication is what let one of them rot
unseen.

**The law, in one sentence:** every question has exactly ONE owner, and a
document that reports on a question it does not own must POINT at the owner,
never copy the answer.

## The owner table

| The question | The one owner | Everyone else |
|---|---|---|
| What is OPEN right now? | `docs/pending-bugs.md` (this repo; ychrome has its own) | link to it |
| What shipped, and when? | git history + `CHANGELOG.md` | link to it |
| Which instruments lie? What traps cost time? | `docs/agent-field-guide.md` | link to it |
| How is the product SUPPOSED to behave? | the spec docs + `DESIGN.md` | link to it |
| Why was a call decided this way? | the campaign memory | link to it |
| What happened in the past? | `docs/archive/` + `~/.claude/memory-archive/` | **do not load it; search it** |

## What `pending-bugs.md` may contain

**Open items only.** An entry is removed in the same commit as its verified fix
— that rule already existed and was not kept, which is how the file reached
3,001 lines with 1,442 of them describing bugs that were already dead.

Every entry declares exactly one status on its own line:

- `Status: OPEN` — nobody has fixed it.
- `Status: FIXED IN CODE — LIVE PROOF OWED` — it is written and merged, and the
  thing that would falsify it has not been observed yet. Name the observation.
- `Status: AWAITING A DECISION` — a design call, not a defect. Name who decides.

There is no `Status: FIXED`. A fixed entry is deleted; git remembers it.

## The archive, and how to call it back

Nothing is deleted in the sense of lost. Closed narratives move to
`docs/archive/` in the same commit that removes them, and stale memory moves to
`~/.claude/memory-archive/-home-user-gh-yggterm/`. **Neither is ever loaded into a
session by default** — that is the whole point, and it is also why the archive
is allowed to be enormous.

To call any of it back:

```bash
# in-repo history (closed bug narratives, superseded designs)
rg -n '<what you remember>' docs/archive/

# archived memory (closed campaigns, retired findings)
rg -n '<what you remember>' ~/.claude/memory-archive/-home-user-gh-yggterm/

# the real record of what shipped, always authoritative over any prose
git log --oneline --all --grep '<what you remember>'
git log -S '<a symbol or string the fix touched>' --oneline
```

⚠ **A history rewrite can orphan work, and a stale doc is the only symptom.**
Compare by SUBJECT, never by hash, across a rewrite boundary — the recipe is in
[[finding-relicense-rewrite-orphaned-34-commits]] and in the field guide.

## Enforcement

`scripts/check-docs-ssot.sh` runs in the test suite
(`docs_ssot::the_bug_file_lists_only_open_items`). It fails when:

1. `docs/pending-bugs.md` contains a closure marker (`✅`, `~~…~~`, `CLOSED`,
   `FIXED AND VERIFIED`, `FOUND AND FIXED`, `SHIPPED`) — a closed entry belongs
   in git, not in the queue;
2. an entry carries no `Status:` line, or one outside the vocabulary above;
3. a second tracked file advertises itself as a list of open bugs.

The check is not advisory. If it is failing, the queue is lying, and a lying
queue is what this file exists to prevent.
