# The sweep policy — how yggterm reclaims disk, per host

**This file is the ONE owner of "what may be deleted, in what order, and on
whose authority".** Code that reclaims space reads its thresholds from here and
adds none of its own. What is currently open is in
[`pending-bugs.md`](pending-bugs.md); what the owner settled is in
[`settled-calls.md`](settled-calls.md); the instruments that lie about file age
are in [`agent-field-guide.md`](agent-field-guide.md).

## 1. Why this exists, and the four measurements that shaped it

A fleet of three hosts, measured 2026-08-08. Hosts are named by role, per the
public-repo rule in `SECURITY.md`.

**(a) An importance ordering alone reclaims almost nothing.** On the GUI host
the codex session store is 618 files / 3.1 GB, and **the smallest 75% of those
files total 2.4 MB, or 0.1% of the store**. Every trivial session from every
period, deleted together, frees two megabytes. **Two files hold 74% of it.**

⇒ Sweeping least-important-first is correct as ethics and useless as space. The
ladder in §3 therefore orders by **reclaim per unit of regret**, not by regret.

**(b) `mtime` is a lying instrument on an agent session store.** 575 of those
618 files share a single mtime *minute*: a fleet copy flattened them. Ask the
filesystem when a session was last touched and it reports that they were all
touched at once, which is false for every one of them.

⇒ **`mtime` is not an input to any decision in this document.** Birth comes from
the store's own path/filename convention; last-touch comes from the final
record's `timestamp` field. Entry filed in the field guide.

**(c) The bulk of a large session is attachments, not reasoning.** The largest
file is 36,288 lines, of which **388 lines carry 96.4% of the bytes**, across 579
`data:image` occurrences. Deleting that session to reclaim space destroys a
year-old conversation in order to discard pasted images.

⇒ Compaction precedes destruction (§5), exactly as compression precedes eviction
in ZFS.

**(d) The largest reclaim in the fleet needs no session deleted at all.** Codex
writes `.bak.<timestamp>` copies of its own rollouts and nothing has ever pruned
them. yggterm already knows they exist and merely ignores them
(`AgentCliDescriptor::store_excluded_name_fragments = [".bak."]`). Fleet total:
**61.1 GB**, including one rollout present in five byte-identical copies on the
integrator host.

| host role | codex `.bak.` copies | `target/` | sessions (real `.jsonl`) |
|---|---|---|---|
| integrator | 57.3 GB (250 files) | 204 GB (175 debug, 80 of that `incremental/`) | 14.9 GB / 111 |
| GUI | 3.3 GB (751 files) | 18 GB | 3.1 GB / 618 |
| workshop | 0.5 GB (54 files) | 3.6 GB | 0.9 GB |

## 2. One engine, and what it is not

There is **one sweep engine per host**, running in the host daemon's chore
thread beside the sweeps that already live there
(`clipboard_sweep.rs`, `socket_sweep.rs`). It is not three sweepers with three
policies; those two modules become **domain adapters** registered with it, and
this file becomes the one place their thresholds are stated.

It is **per host, and only its own**. A daemon sweeps its own `$YGGTERM_HOME`
and the stores on its own filesystem. No sweep ever reaches across a connection
to delete on another host — each connected host runs its own daemon and sweeps
itself. This is the same boundary `clipboard_sweep.rs` already keeps, and it is
what makes a sandbox daemon unable to touch a real store.

It is **not a background process of its own**, not a systemd timer as its
primary trigger, and not something the user schedules.

## 3. The class ladder — the policy table

The engine walks classes **in order** and stops as soon as the pressure that
woke it is relieved. It never weighs a junk session against a build cache; a
class is exhausted before the next is considered. Because C0 alone is 61 GB
fleet-wide, the classes that can cause regret are rarely reached at all.

| class | contents | cost of being wrong | speaks? |
|---|---|---|---|
| **C0 redundant** | codex `.bak.<ts>` rollout copies · `target/**/incremental/` · `$YGGTERM_HOME/npm-cache/_cacache` · stale versioned sockets (owned by `socket_sweep.rs`) · clipboard trash past its second TTL (owned by `clipboard_sweep.rs`) · rollback binaries beyond the retention set of §9.3 | none: provably a duplicate, or regenerated on next use | never |
| **C1 derived** | `target/debug` past budget · cross-compile triple dirs for targets not in the current release matrix · `target/release` output for non-HEAD | one rebuild, named in the plan | never |
| **C2 noise** | sessions below the substance floor that were never resumed and are past the noise TTL | a trivial session is gone | never |
| **C3 cold** | real sessions, scored per §4, evicted by value density, via the trash hop of §9.2 | a real conversation is in the trash for the second TTL | one trace line |
| **C4 never** | held sessions · sessions with a live PTY or an owning daemon · anything the reference check proves referenced · the `emd-renderer` cache (§8) · the running binary and the constitutional rollback set (§9.3) | n/a | n/a |

## 4. C3: the importance score

Two axes, taken from ARC's balance of recency against frequency. The owner's
rule *"a decade-old session, if touched, is important"* is ARC's MFU list
outranking its MRU list, and it is implemented as a **hard gate, not a weight**,
so that it cannot be outvoted by an accumulation of small penalties:

> **The recency floor.** A session whose last-touch is inside the floor window
> is in C4 and is not scored at all, whatever its birth date, size, or
> substance. Age alone can never make a session a candidate.

For everything below the floor:

- **substance** = count of **user turns**, log-scaled. Not bytes: measurement
  (c) proves bytes track attachments, so a byte-weighted score would call an
  image dump important and a long argument trivial.
- **frequency** = count of **distinct days on which the transcript was appended
  to**, which is its resume-episode count. A file returned to across a year is
  precious even if each visit was short.
- **recency** = exponential decay on the age of the **final record's
  timestamp**. Never `mtime` (measurement (b)).

`importance = w_s·log(1+substance) + w_f·frequency + w_r·decay(recency)`

**Eviction key = `bytes / importance`, descending.** Largest reclaim per unit of
regret goes first. This is the clause that makes the ladder actually free space:
under a pure importance ordering the engine would delete 500 trivial sessions,
free 2 MB, and still be facing the two files that hold 74% of the store.

The **substance floor** for C2 is a session that never exceeded a handful of
user turns and was never resumed. That is the "hi" / "testing" tier. It is swept
for index hygiene and scan speed, and the plan must not claim a space win for
it.

## 5. Compaction, and the rehydration contract

**Owner-settled 2026-08-08** (recorded in `settled-calls.md`): yggterm **may**
rewrite an agent CLI's transcript to externalize blobs, **provided it restores
the file before handing off to the CLI**.

This is licensed by the product's own shape. yggterm already owns the moment
between a click and `codex resume <uuid>`; that handoff *is* the product. So a
compacted transcript is rehydrated inside the handoff and the CLI is handed a
byte-identical file, which is why this is lossless where a one-way strip would
not be.

    rollout-<uuid>.jsonl   1.41 GB  →  51 MB  +  blobs/<sha256>…

    on resume:
      rehydrate <uuid>              → restores byte-identical
      ssh <host> "cd <cwd> && codex resume <uuid>"

**⛔ The law that makes this safe: no destroy without a verified restore.**
Before the original is released, the engine must rehydrate to a temporary file
and match its digest against the pre-strip digest. A mismatch, a missing blob,
or an unreadable sidecar leaves the original **untouched**. This is ZFS's
checksum discipline: the copy is not trusted because it was written, it is
trusted because it was read back and verified.

**⚠ The honest risk, unproven as of this writing.** The failure mode of this
choice lands on the resume path, which is the product's core value. Nothing here
has yet demonstrated that a rehydrated rollout is accepted by codex, nor that
rehydration completes fast enough to sit inside a click. Both are required
before any compaction is enabled on a real store, and the fallback if
rehydration fails at click time is to hand over the **intact original**, which
means the original is retained until the first successful verified round-trip.

## 6. Triggers: a watermark AND a budget

A single trigger is insufficient, and the measurements say why.

- **Pool watermark.** The GUI host's pool is at 64% capacity with 51%
  fragmentation, so a free-space watermark is the right instrument there. Below
  the low watermark nothing runs at all.
- **Per-domain budget.** The integrator's pool is 8 TB at 9% used while 204 GB
  of build output sits in one directory. No watermark will ever fire there. A
  budget is `zfs set quota` applied per domain: `target/` is capped regardless
  of how much free space exists.

Whichever fires, wakes the engine; the class ladder decides what it takes. A
timer remains only as a heartbeat for hosts under neither pressure, so that C0
does not accumulate for a year on an idle machine.

**Owner-settled budgets 2026-08-08:** `target/**/incremental/` swept **daily**
(the integrator regenerates ~13 GB/day, against a weekly sweep that reclaimed
28.7 GB in one run), and `target/debug` under a **40 GB** budget, oldest
artifacts evicted past the cap.

## 7. The silence policy

The owner's requirement is that this is never brought to his attention. So:

- **C0, C1 and C2 never speak.** Not a toast, not a notification, not a summary
  on next launch.
- **C3 writes one line to the daemon trace** and nothing else.
- **`degraded` always speaks.** A sweep that could not complete a reference
  check, verify a restore, or read a store it was asked to sweep must say so,
  because that is the one state in which silence is a false report of health.
  This mirrors `ClipboardSweepOutcome::degraded`, which already exists.

Everything the engine did is readable on demand through §10 whether or not it
spoke.

## 8. Exemptions

**The `emd-renderer` low-resolution webview cache is exempt, by owner decision
(2026-08-08): it is an infinite, never-swept cache and is to remain one.**

⚠ **It does not exist on disk yet.** The owner's words are *"I proposed this. I
think it should stay this way"*, and no such cache is present in any checkout at
the time of writing. The exemption is recorded **ahead of** the thing it
protects, deliberately: the engine and the cache will be built by different
sessions, and a cache that appears after the sweep does is exactly the kind of
directory a thorough agent sweeps for being unaccounted for.

Recording it as an **explicit exemption rather than an omission** is the
difference between a decision and an oversight. The engine must treat an
unrecognised directory under `$YGGTERM_HOME` as C4 by default for the same
reason: unknown means keep, never means collect.

## 9. The safety laws

**9.1 Fail-safe bias.** Deletion requires proof of non-reference; absence of
proof keeps the file. Inherited verbatim from `clipboard_sweep.rs`, where a
transcript that fails to read causes that round to trash nothing.

**9.2 Deferred destroy, and holds.** Reclaim is two-phase, as
`clipboard_sweep.rs` already implements: a trash hop with an explicit
`.trashed-<ms>` suffix (a rename preserves mtime, so mtime cannot carry this),
then a second TTL before the bytes are actually released. A **hold** is
`zfs hold`: a held session is C4 and no class may take it.

**9.3 ⛔ Never delete a binary that something is running, and never break the
constitution's rollback set.** `CLAUDE.md`'s constitution requires
**version-coexisting daemons**: an older daemon stays alive owning sessions that
are mid-flight, and the user must never learn that two daemons exist. That
guarantee is made of binaries on disk. A sweep of "stale builds" that deletes
the binary an older live daemon would restart from converts a housekeeping task
into an outage for another agent's work.

The retention set is therefore: every binary held by a running process
(`/proc/*/exe`, identified — never counted), the currently deployed version, the
last known-good rollback, and the N most recent versions. Everything else in
`$YGGTERM_HOME/{bin,binbak,deploy-backup,versions}` is C0.

⚠ Those four directories today hold ~260 MB of hand-named copies
(`.rollback-3051`, `.pre-inputdead-<epoch>`, `.pre-phaseE`, `.<version>.bak`)
with no shared convention and no retention rule. **The absence of a convention is
the bug**; the engine adopts the existing names once and no new ad-hoc name is
created after that.

**9.4 Own host, own stores.** §2.

**9.5 Determinism.** `plan` is a pure function of the store state and the clock:
the same inputs yield the same plan, per the no-non-determinism rule in
`CLAUDE.md`. The executor applies a plan; it does not make decisions of its own.

## 10. The verbs

Accounting is a first-class read, as `zfs list -o space` is. Nothing here is the
user's routine responsibility; these exist so that the automatic behaviour is
inspectable, and so that "clear some space" is a sentence that works.

    server sweep status                    # what is using space, what goes next; touches nothing
    server sweep plan [--class cN]         # the deterministic plan; deletes nothing
    server sweep run [--class cN] [--dry-run]
    server sweep hold <session> | release <session>
    server sweep restore <trashed>         # undo a trash hop within its TTL

## 11. Not in scope

- **Cross-host deletion.** §2.
- **Anything under `docs/pending-bugs.md`'s definition of open work.** This file
  says how it should behave; it is not a status document.
- **The `emd-renderer` cache.** §8.
