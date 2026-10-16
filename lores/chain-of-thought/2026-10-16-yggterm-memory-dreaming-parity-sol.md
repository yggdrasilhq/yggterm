**Accept-with-corrections overall.** Keep the composer, critic, provenance, bounded writes, and ownership partition. The missing pieces are reliable progress, temporal validity, an owner correction loop, and retrieval that demonstrably changes agent behavior.

OpenAI’s June article establishes the three behavioral objectives and review controls; it does not disclose enough implementation detail to reproduce V3. Treat parity as a measurable behavioral target. [OpenAI’s dreaming announcement](https://openai.com/index/chatgpt-memory-dreaming/)

I independently confirmed that sync reports “up-to-date” while the namespace’s NOW.md remains dated October 6. It also recommends H_vocab work that this brief explicitly rules out. The other incident measurements below come from your brief; this is an architectural consultation, not a code audit.

**Q1 — Accept-with-corrections: rank gaps by what the owner experiences.**

| Rank | Gap | Owner-visible consequence |
|---|---|---|
| 1 | No enforced synthesis-progress contract | Ten days of conversations can leave a new session behaving as though nothing happened. |
| 2 | No complete temporal-validity mechanism | Agents treat past plans, temporary conditions, and old operational measurements as current. |
| 3 | No immediate, durable correction and preference path | The owner corrects an agent, but another session repeats the same error. |
| 4 | No demonstrated continuity across compaction | Useful older context can disappear when the composer sees only recent deltas. |
| 5 | No review surface connecting remembered claims to evidence and controls | The owner cannot cheaply discover, correct, or suppress a mistaken detail. |

Two qualifications matter:

- **The missing mechanisms are not all proven absent.** The brief establishes no end-to-end guarantee for them. Audit existing retrieval and owner-memory handling before adding parallel machinery.
- **NOW.md is an orientation budget, not a sufficient memory store.** Parity requires relevant detail to remain retrievable beyond those 2,600 bytes.

The live stale H_vocab recommendation is a particularly useful regression case: an earlier proposal survived as an apparent next action after a later ruling invalidated it. A freshness mechanism must catch **supersession**, not merely age.

**Q2 — Accept-with-corrections: computed ages help, but validity does the real work.**

The minimum addition is a deterministic **read-time temporal envelope**, backed by small extensions to the existing CLAIMS records:

```text
Now: 2026-10-16, Asia/Kolkata
Synthesis: 10d old | source coverage through: <cursor>
[c17 · evidence 10d old · recheck due] ...
[c23 · validity ended Oct 15] ...
```

Use three distinct dates:

- **Evidence time:** when the underlying observation or statement was made.
- **Validity boundary:** when it stops being usable as current context, if known.
- **Review deadline:** when it needs checking; passing this does not prove it false.

Keep generation time separate. Rewriting a claim today must never make October 6 evidence appear newly observed.

Extend CLAIMS with optional `valid_from`, `valid_until`, `review_after`, and `supersedes`. Store these outside the tight NOW prose budget. Calculate ages and status when memory is delivered; do not spend composer tokens maintaining changing age strings.

A composer `STALE_BY` input is useful **as computed context**, with the earliest next expiry persisted for scheduling. It need not become a fifth NOW section.

The indispensable trigger change is:

```text
work_due = new_source_delta OR temporal_deadline_due
```

Otherwise, a birthday plan never ages when conversations stop. Read-time validity filtering must still work while the composer is unavailable.

Distinguish:

- **LAG:** relevant source changes have not reached synthesis.
- **ROT:** a claim is expired, contradicted, superseded, or due for review.

A contradiction invalidates the affected claim immediately; it need not wait for contradiction density to cross a threshold.

Finally, time alone cannot establish that a plan happened. “Trip scheduled to end yesterday; current location unknown” is justified. “You returned home yesterday” requires evidence. Durable preferences also should not expire merely because they are old.

**Q3 — Accept-with-corrections: build the review surface, with an authoritative correction path underneath.**

“Always current” cannot mean knowing unreported changes. It should mean:

> Incorporate available changes promptly, expose uncertainty, and stop presenting expired information as current.

One fast-rotting owner file cannot carry everything. Use two logical layers, reusing existing stores where possible:

| Layer | Contents | Lifecycle |
|---|---|---|
| Durable owner context | Preferences, constraints, recurring working practices, important background, surfacing instructions | Persists until corrected or superseded; scoped to relevant tasks |
| Owner-now | Current commitments, temporary availability, travel, explicitly reported energy or mood | Short validity, explicit evidence dates, expiry or review deadlines |

Select both across namespaces through one owner identity. A campaign dream must not overwrite a newer owner update learned elsewhere.

The minimum review surface is a generated page or command showing:

- What agents currently use about the owner.
- Changed, expired, and contested details.
- Claim IDs, evidence dates, and expandable source references.
- Controls to **correct**, **dismiss**, and specify **when to surface** a detail.

Default to owner context and changed campaign highlights; expose full NOW diffs on demand.

Corrections should enter a small authoritative owner-authored record and affect the **next memory read**, without waiting for quiescence, cooldown, or composition. Dreams subsequently reconcile against that record. Old archives must not resurrect a dismissed claim; retain the correction or suppression reference.

A board post can transport a correction, but an ordinary agent post is not an owner instruction. Authenticate the owner action and record its target and scope.

A single-file interface is acceptable if it edits **canonical owner input**. Do not invite edits to generated `owner-now.md` or a hub projection. Prior memory records that direct projection edits can be reverted; implementation should verify this behavior locally before choosing the edit path.

“Rich enough” means agents retrieve the relevant established context without re-interviewing him. It does not require a longer personality essay. Preserve concrete preferences, decisions, and examples with provenance.

**Q4 — Accept-with-corrections: require both execution receipts and progress deadlines.**

The deterministic law should be:

> Every expected tick produces a bounded execution receipt; every pending namespace exposes its coverage and next progress deadline. Missing receipts, execution failures, and overdue progress are independently visible.

The receipt needs only:

```text
tick / start / finish / outcome / reason / retry_due
namespace / source_head / covered_through / oldest_pending_at
```

Valid outcomes include idle, deferred with a deadline, running with a bounded lease, failed, and applied. Backoff remains a failure state with a retry time; it must never refresh a “healthy” timestamp.

Use two separate checks:

1. **Execution liveness:** Did the expected controller tick complete, fail, or exceed its deadline?
2. **Synthesis progress:** With work pending, is source coverage advancing within the declared service bound?

An independent existing canary or supervisor must check these. A dreamer cannot report its own absence. “Within one tick” is achievable for a detectable failure under that monitor’s availability, with an explicit execution grace period; it cannot be an unconditional guarantee if the monitor also stops.

Keep receipts outside ordinary dream inputs so monitoring cannot create journal churn.

**Bounded catch-up is correct—and a small tail remains the steady-state requirement.** Recover in bounded, resumable batches. Advance coverage only through inputs durably accounted for; a byte-limited brief must never advance its watermark to an unprocessed journal head.

Do not subtract 11,534 from 14,169 without checking overlap and remaining meaningful events. Remove verified duplicate work deterministically while retaining its audit trail.

Two existing policies also need scrutiny:

- Selecting only the largest backlog can starve smaller namespaces. Include oldest pending age and deadlines.
- Fifteen-minute quiescence plus a moving version fence can indefinitely prevent progress.

Preserve writer protection. Fence the actual namespace dependencies, classify indefinite deferral as degraded freshness, and expose it. If the current policy prohibits synthesis throughout continuous activity, freshness parity is impossible during that activity. Monitoring cannot repair that scheduling contradiction.

**Q5 — Accept-with-corrections: add incremental evidence compaction before adding more summary hierarchy.**

A byte budget controls call cost. It does not establish coverage, preserve useful history, or guarantee correct carry-forward.

Use this flow:

```text
Source events
  → deterministic deduplication and bounded batches
  → evidence-linked claim updates
  → existing composer + critic
  → NOW and owner-context projections
  → task-relevant retrieval
```

The key additions are:

1. **Coverage manifests.** Every batch records which events it represented, deferred, or excluded and why. An acknowledged event need not appear in NOW, but its disposition must be recoverable.
2. **Persistent claim state.** Supply relevant retained CLAIMS alongside new evidence. If CLAIMS are currently emitted but never reused, persistence and retrieval are the first repair.
3. **Correction and supersession edges.** Preserve “A was replaced by B,” including the owner’s authority, rather than choosing whichever text was ingested last.
4. **Selective historical retrieval.** Keep a small initial context while retaining access to evidence and less frequently used detail.

Excluding generated NOW from raw-delta ingestion is sound loop prevention. Excluding all retained verified state from composition would make every dream relearn the world from its newest batch. Reuse evidence-linked claim state without treating previous summary prose as new evidence or refreshing its dates.

For exceptionally large backlogs, intermediate claim extraction can use bounded sol/GLM passes. Preserve source references and unresolved disagreements through the merge. Do not create a recursively summarized narrative whose errors become untraceable.

Reuse existing campaign §STATE doors before creating per-campaign sub-NOWs. Add another summary level only when measured retrieval failures justify it.

Also verify source coverage: can an ordinary conversational preference reach the dreamer without an explicit “remember this” or manual door update? If not, pipeline reliability alone cannot provide the requested continuity.

**Q6 — Accept: build these four slices, in order.**

| Slice | What changes | Effect-level proof | Cheapest falsifier |
|---|---|---|---|
| **1. Observable, bounded progress** | Add execution receipts, independent monitoring, per-namespace coverage and oldest-pending age, fair scheduling, and resumable catch-up. Preserve writer protection; make starvation visible. | Inject the actual dash-leading namespace failure and a stopped controller. The owner sees failure by the next monitoring tick. Recover meaningful backlog without skipping a seeded event; a small namespace also progresses. | Kill the controller. If health remains green, reject the slice. |
| **2. Time-aware memory delivery** | Extend CLAIMS with validity/review metadata; add read-time date, ages, supersession handling, and deadline-triggered work without new deltas. | Advance a controlled clock across a temporary-context expiry. A fresh session stops using it as current, preserves a durable preference, and accurately describes what remains unknown. | Advance time with **zero journal writes**. If the agent still treats the expired context as current, reject it. |
| **3. Correctable owner context** | Separate durable context from owner-now; add a minimal review view and authoritative corrections, dismissals, and surfacing instructions applied on reads. | The owner changes one preference once. Fresh sessions in two namespaces follow it; replaying an old archive does not restore the dismissed version. | Disable composition immediately after correction. If the next session still uses the old preference, reject it. |
| **4. Evidence-preserving compaction and behavioral evaluation** | Reuse retained CLAIMS, bounded manifests, deduplication, and selective source retrieval. Add a small replay suite covering continuity, preferences, temporal change, and corrections. | Replay a multi-sitting history containing a rare useful fact, an implicit preference, a later correction, and irrelevant churn. Fresh sessions perform the corresponding tasks correctly with bounded initial context and traceable sources. | Put the decisive correction across a batch boundary and the useful fact in an early batch. If either disappears or reverses, reject it. |

Run those behavioral cases through the **actual managed-session startup path**. Correct files alone do not establish that a new agent received the context or acted on it.

The October 6 H_vocab recommendation should be an immediate regression fixture. Repair it through the dreamer-owned publication path using the later ruling as evidence. None of these slices requires reviving H_vocab telemetry, Hebbian decay, or the three-strike abstraction engine.


