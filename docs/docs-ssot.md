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
| What is waiting on the OWNER? | `docs/owner-attention.md` | link to it |
| What shipped, and when? | git history + `CHANGELOG.md` | link to it |
| Which instruments lie? What traps cost time? | `docs/agent-field-guide.md` | link to it |
| How is the product SUPPOSED to behave? | the spec docs + `DESIGN.md` + `docs/spec-human-agent-interface-divergence.md` (human vs agent surface map) | link to it |
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

## ⛔ A CLOSED ENTRY THAT COMES BACK — the one thing the SSOT check cannot see

`check-docs-ssot.sh` asks whether every entry is **well-formed**. A resurrected
entry is perfectly well-formed — it was well-formed the first time it was
written. *Should this entry still be here at all* is not answerable from the
file's contents, only from its history.

**It has happened:** an entry closed, live-proven and deleted was re-added
verbatim by a later whole-file write from a stale copy, in a checkout several
clusters share. The work was done; the queue said it was not.

⇒ **`scripts/check-queue-resurrection.sh`** reports any heading that a commit
deleted and that is present again, naming the commits that deleted it. It
**reports rather than fails** by default, because deliberately re-opening an
entry is legitimate — but a re-opened entry must **say so in the entry**, or the
next reader cannot tell a decision from a clobber.

⭐ The cause is a shared checkout plus a whole-file write, not carelessness
about queues. The queue is a merge surface like any other file, and it is the
one whose silent regression costs most — it decides what everybody works on.

### ⛔ RUN THAT SCRIPT AFTER EVERY QUEUE MERGE — it already catches this and nobody runs it

**Measured 2026-08-14:** the check was reporting two resurrected entries and had
been for some time. One of them — a guard entry closed as *"fixed and
live-proven"*, re-added through a lane merge — then **cost a lane an entire
session of not pushing**, because the block was inherited as fact from the queue
and never re-tested. The tool worked; nothing invoked it.

⇒ `check-docs-ssot.sh` asks whether entries are **well-formed** and passes a
duplicated or resurrected one happily. **The two checks answer different
questions and running only the enforced one is how this class survives.**

### ⛔⛔ MERGING THE QUEUE: "keep both sides" is safe ONLY when both sides are pure ADDITIONS

A queue conflict usually looks like two lanes appending in the same place, and
for that, keeping both is right. **It stops being right the moment one side
RENAMED an entry the other still contains** — then keeping both does not merge
them, it **duplicates** the entry, and the copy that survives from the stale side
carries whatever the rename was correcting.

**It happened, in the worst available form:** a lane renamed an entry to drop a
claim it had just **retracted**; the merge kept both sides; the result
**republished the retracted claim as a live entry**, well-formed and statused.
The enforced gate saw nothing wrong with it. A second entry in the same merge was
left as a **heading with an empty body** — that one the gate did catch, which is
the only reason the merge was inspected at all.

⇒ **After resolving any `pending-bugs.md` conflict, before committing:**

1. diff the **heading set** against **both** parents — nothing may vanish from
   either side except an entry one side deliberately renamed or closed;
2. `grep '^## ' docs/pending-bugs.md | sort | uniq -d` — **must be empty**;
3. run `check-queue-resurrection.sh`;
4. restore a mangled entry **verbatim from its own branch** rather than retyping
   it from memory.

⚠ And prefer not to be in this position: **rebase a lane before pushing it**, so
the orchestrator is never the one adjudicating hunks in somebody else's entries.
