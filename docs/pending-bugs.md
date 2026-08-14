# Pending bugs

**This file is the ONE answer to "what is open".** It lists open items only; an
entry is deleted in the same commit as its verified fix, and git remembers it.
The rules, the owner table for every other question, and how to search the
archive are in [`docs-ssot.md`](docs-ssot.md). `scripts/check-docs-ssot.sh`
enforces them.

Statuses: **OPEN** · **FIXED IN CODE — LIVE PROOF OWED** (name the observation
that would falsify it) · **AWAITING A DECISION** (name who decides).

Closed narratives from before 2026-08-02 are in
[`archive/pending-bugs-closed-2026-08-02.md`](archive/pending-bugs-closed-2026-08-02.md).

## ⚠ [9.2→6.x] `session rename` IS ASYNC W.R.T. `rows`, SO THE DOCUMENTED VERIFY STEP RETURNS A FALSE NEGATIVE

**Status:** OPEN

*measured 2026-08-14 with a controlled probe on a live agent row*

The fleet contract says **every row verb reports the REQUEST, not the EFFECT, so
read the state back, every time.** That is right, and it is **not sufficient**:
`server app session rename` returns before `server app rows` reflects it, so a
read-back taken immediately returns the **previous** label.

Probe: rename to a marker, read back three times, restore.

```
BEFORE:                              '<previous label>'
IMMEDIATELY after rename (no sleep): '<previous label>'   <- STALE
after 2s:                            '<marker>'           <- landed
restored:                            '<previous label>'
```

⇒ **The stale read is indistinguishable from a failed write**, which is exactly
the failure mode the read-back law exists to catch. An agent that follows the
law literally, with no delay, gets a **false negative** and reports a defect that
does not exist. **That happened: a rename was reported as failed, a cause was
hypothesised, and a successor was asked to investigate a non-problem.** The
second attempt "worked" only because it happened to carry a `sleep 1`.

**Fix options, cheapest first.**

1. **Doc-only, and it costs nothing:** the read-back clause should say *read it
   back after a beat*, and name a figure. **1 s was sufficient in every trial;
   0 s never was.**
2. **Make the verb synchronous in its own reply**: have `rename` return the
   post-write label it can already see, so the caller needs no second call.
3. **Make `rows` read-your-writes** for the calling client.

⚠ **Option 1 alone closes the reporting hazard.** 2 and 3 are the real fix and
are a design call for whoever owns the control plane.

**Falsifier:** if a 0 s read-back ever returns the new label on an unloaded
daemon, the timing is load-dependent rather than structural and the doc figure
needs to be a bound, not a constant.

⚠ **Scope stated rather than implied:** measured on ONE host, ONE row, ONE
daemon, with the GUI busy. Nobody has checked whether other mutating verbs
(`outline`, `remove`, `notify --job`) share the property. **They plausibly do**,
and if so this is a control-plane-wide clause and not a `rename` note.

## ⛔⛔⛔ [6.9→6.7] THE HANDLER SPAN MEASURES 9% OF THE HANDLER, IN THE WRONG UNIT

**Status:** OPEN

*measured 2026-08-14; instrument, controls and the two refuted candidates in
[`idle-cost-model.md`](idle-cost-model.md) §6j — spec is §S6*

A connection-handler thread on a LIVE daemon burns **20–44 ms of CPU**. The
`PerfGuard` span over it covers **~2.4 ms**, because the guard **drops when
`handle_request` returns** and records **wall time**, which cannot distinguish
work from waiting.

⛔⛔ **THE "93.8% KERNEL" THAT MOTIVATED THIS ENTRY IS RETRACTED** (§6j-3). It came
from `/proc/<tid>/stat` utime/stime — 10 ms ticks, **each truncated
independently** — which annihilates the smaller component below ~10 ms and drives
the share to 100% for the larger. Against `getrusage(RUSAGE_THREAD)` on a known
4 ms/20 ms mix: true 83.3%, ticks report **100.0%**. ⇒ **Do not describe this cost
as kernel time.** The magnitude survives (two methods agree); the composition does
not, and S6's in-process span is the instrument that can answer it.

⇒ **~1.8–1.9 cores of the daemon population are spent where no instrument
looks.** That number is stable: two 60 s runs over 20 daemons put the
dead-thread term at 1.793 and 1.944 cores, per-daemon reproducible to ~5–10%.

**Fix (§S6), two small parts:** add a `CLOCK_THREAD_CPUTIME_ID` reading to
`PerfGuard` beside the wall reading (**priced at 570 ns/call on this host — two
calls is 0.05% of a ~2.4 ms span**), and wrap the whole closure at
`spawn_unix_client_handler` (`daemon.rs:785`) instead of just `handle_request`.

**Expected effect: ZERO cores — do not promise any.** It converts an
unattributable 1.8 cores into an attributed one.

⭐⭐ **AND IT IS NOW THE ONLY INSTRUMENT THAT CAN ANSWER THE LIVE QUESTION**
(§6j-8). A baseline-subtracted external estimator was pointed at live daemons and
came back **VOID**: bracketing baselines on one daemon moved 0.4240 → 1.1437
cores and produced an impossible **negative** per-request cost. The baseline on a
live daemon *is* the per-session reader term, and it drifts ~35x faster than the
signal. (⚠ that term is **not** episodic — see the correction below; it is simply
large enough that its ordinary variation swamps a per-request quantity.) ⇒ **Per-request cost is below the noise floor of any external
instrument; only an in-process span can reach it.** ⛔ Treat the remaining live
figure (20–44 ms per dying thread) as **provisional** until S6's live record
lands.

**Prediction S6 must satisfy:** ~0.7 ms on an empty daemon, and a live figure that
either confirms 20–44 ms or replaces it.

⛔ **Ruled out by measurement, so do not re-propose them:** `malloc_trim(0)` at
`daemon.rs:19103` (**0.020–0.039 ms** at 30 threads/360 MB — ~600x too small),
the page-faults it induces (353–852/request ≈ 0.5–1.7 ms), the lock wait itself
(`try_lock` then a **blocking** `lock`; a block costs no CPU), and the trace
writer (handles cached, no stat or reopen per call).

⚠ **The request MIX is RESTATED, not refuted** (§6j-6): `snapshot` is a
**per-ROW** verb — **32.6 µs/row against `status`'s 5.0**, measured with sessions
held at ZERO in both rungs, so a 264-row daemon owning one session pays 8.7 ms
per snapshot for rows it does not own. ⛔ **PRICED AND NOT WORTH BUILDING:** the
live rate is **0.37/s** (unsampled, and §6a's independent trace figure of
0.25–0.32/s agrees) ⇒ **0.0032 cores, ~0.1%** — it dies where S5 died.
⭐ **And check which telemetry stream a count comes from before quoting a ratio:**
`status`/`ping`/`working_flags`/`terminal_read` spans are **sampled** (kept at
≥8 ms or 1-in-50) so their `perf-summary` counts are undercounts; `snapshot` is
not on that list, and the event trace is never sampled at all.
⛔⛔ **AND A "~1.7 ms PER-CONNECTION COST OUTSIDE THE CLOSURE" THAT DOES NOT
EXIST** — I got it by subtracting a closure span from a process counter that was
charging concurrent reader/chore work to whatever connection was open. Measured
from inside in three pieces (parent accept+spawn 44–48 µs · child pre-closure
39–55 µs · closure 61–126 µs), the **whole per-connection floor is ≈150–230 µs**,
~8x smaller. ⇒ **S4's old note was right, but by luck at ~50 µs and now by
measurement; schedule S4 on stability and observability, NEVER on cores**
(<0.001 cores/daemon).

## ⛔ ONE DIRECTORY, TWO READERS, AND ONLY ONE WAS TOLD ABOUT THE STAGING DIR

**Status:** OPEN

*Found 2026-08-14 by the sidebar lane, which could not run a single app-control
verb against a freshly created GUI until it deleted a directory by hand.*

`client-instances/<scope>/` holds one JSON record per attached client, published
by an atomic write that stages into a `tmp/` subdirectory of that same folder.
**Two functions enumerate that directory and they disagree about what a
non-file entry means:**

| reader | on a directory entry |
|---|---|
| `cleanup_stale_client_instances` (yggterm-shell) | skips it — *"the atomic-write staging dir (and anything else that is not a plain file) is never a record"*, with a test that pins it |
| `collect_client_instance_records` (yggterm-server) | `fs::read` returns **EISDIR (os error 21)**, which is neither `NotFound` nor a parse failure, so it **propagates and fails the whole enumeration** |

**Reproduced twice, on two independently created sandbox homes at 3.0.154.**
Every `server app` verb died with `reading client instance record …/tmp: Is a
directory`, and `rmdir` on that one empty directory made them all work.

⛔ **The consequence is larger than a failed verb, and it is the drain's.** The
comment directly above that read explains that an error there is deliberately
NOT treated as "no clients": `daemon_should_idle_shutdown` reads an error as *"I
could not ask, so do not retire."* ⇒ **a daemon whose staging directory exists at
the sampling moment can never retire**, and that gate is the one the constitution
rests the drain on.

**Fix:** the one-line `entry.file_type()` skip the other reader already has. ⚠
Two readers of one directory is the second encoding; the durable fix is one
traversal both call, since this is exactly the shape where a rule learned on one
side does not reach the other.

**Falsifier:** create `client-instances/<scope>/tmp/` on a live home and run any
`server app` verb.

## ⛔ [6.3] A SESSION KEY IS BEING USED AS A ROW KEY, AND DUAL PRESENCE MAKES IT AMBIGUOUS

**Status:** OPEN

*Filed by the lane that widened it, in the same session. Read the dual-presence
half of `AGENTS.md` before touching this.*

A live session renders **twice on purpose**: once under Live Sessions, once in
its cwd folder. Both rows carry the same `full_path`, deliberately — that path is
the SESSION's identity, and single source of truth applies to the session object
rather than its display location.

⛔ **So `full_path` cannot answer a question about a ROW.** Roughly 150 sites
resolve a row with `find(|row| row.full_path == …)` and some then read fields
that are properties of the PLACEMENT rather than of the session: `depth`,
`child_count`, row-set headship, and the row's INDEX in the list. Those get the
first match, which is the rail copy only because the rail happens to be pushed
before the stored rows.

**It has misfired once already.** When a verb force-expanded the tree,
`resolve_app_control_row` matched the cwd-tree copy — which heads no set — and
`row-expanded` began refusing. See the row-set collapse entry, which records it.

⚠ **Exposure grew when the cwd-tree regression was fixed.** Before that fix only
a session with no transcript on disk was dual-present in the local tree; now
every local live agent session is, which is the correct behaviour and also means
the ambiguity is no longer rare. **Nothing observed is broken today** — the rail
is first, so first-match is currently right — but the correctness rests on list
order rather than on anything that states it.

**Two candidate shapes, and the choice is the work:**

1. Make the ordering a CONTRACT — assert rail-before-tree in the merged list and
   have the resolvers say which copy they want, so the rule is written down
   instead of inherited.
2. Give a row an identity distinct from its session's, and let consumers that
   want the session keep asking by `session_id`.

⛔ **Do NOT "fix" this by de-duplicating the tree.** That is the regression this
entry's sibling just removed, and it is a spec violation in as many words.

**Falsifier:** with a live local agent session, resolve its path against a
force-expanded row list and assert the row you get is the one that heads its set.

## ⛔ A LOCAL LAUNCH INTO A DIRECTORY THAT DOES NOT EXIST LEAVES A KEEP-ALIVE HUSK

**Status:** OPEN

*Found 2026-08-14 while root-causing the cwd-tree entry below it; it is the
session that entry was filed against.*

An agent session was created **locally on the GUI host with a working directory
that exists only on the build host**. The directory is absent there, so the agent
CLI never started and never wrote a transcript. What remains is a row that is
`live`, `keep_alive`, idle forever, and has never had a process.

**Three things it costs, none of them cosmetic:**

1. The row holds a keep-alive seat and a sidebar number that read as a working
   lane.
2. **The condition is already detectable and is already being swallowed.** A
   local CC row's `session_id` IS its transcript id from birth
   (`id_assigned_at_birth`, `claude --session-id <uuid>`), so
   `local_cc_session_jsonl_path(&session.id)` returning `None` means the CLI
   never wrote anything. `daemon.rs` names that exact signal *"stuck launch
   hint"* — and then `continue`s past it inside a title poll, so nothing
   surfaces and the row keeps looking healthy.
3. It was the only session in the whole inventory exempt from a dedup guard that
   keyed on having a transcript, so it rendered where all its healthy siblings
   had been deleted, and **the anomaly got filed as the bug**.

⛔ **There is NO row-identity component, and an earlier draft of this entry said
there was.** It claimed the `session_id` stays a daemon runtime key that only a
successful start repoints. That is wrong for Claude Code on both the local and
remote paths: the id is assigned up front and handed to the CLI, so the row id is
the authoritative transcript id from birth and no rebind is involved. The
transcript is absent because nothing ever ran, not because the id names a
different namespace. ⇒ **This is launch validation end to end**, and the sidebar
lane holds no part of it.

**What should happen instead:** a launch whose cwd does not exist on the target
host fails loudly at launch, rather than yielding a session object that will
never run. ⚠ Whether the deeper defect is the missing existence check or a launch
that chose the local host for a path belonging to another machine is not settled
here.

**Falsifier:** create a local agent session with `--cwd` naming a path that does
not exist, and read the row back.

## ⛔⛔ [6.4] THREE PRIVATE IDENTIFIERS REACHED origin/main AND ARE STILL IN HISTORY

**Status:** OPEN

*HEAD is clean; the published history is not.*

Three private identifiers sat in this file on `origin/main` and were removed from
HEAD 2026-08-14 (`c6720880`, reachable from `main` via `6e2d9c81`). Removal from
HEAD shrinks the surface and **revokes nothing**: the blobs remain fetchable at
their pre-removal commits.

### ⛔ Why a rewrite alone does not close this

**A force-push revokes nothing.** Pre-rewrite SHAs stay fetchable to anyone who
already has them or who requests them directly. Only a support request to the
forge removes published blobs. ⚠ **A previous request was deflected for being
filed under the wrong category** — it is **private information removal**, not
repository housekeeping, and the category is what decides whether it is actioned.

### ⛔ Why the rewrite has not been run

Four lanes are live on this repository. **A history rewrite landing under live
lanes is how in-flight work gets orphaned**, and the leak-removal seat's own
record says do not re-run repositories that have already been swept. Sequencing
this needs a quiet window and an owner for the whole operation, not a spare hour.

### ⭐ How it survived the earlier sweep — the transferable half

The pre-push guard **scans a push's own commits by design**, so **a term already
on `main` is permanently invisible to it**. ⇒ **A clean guard run on a push is
not evidence the file is clean.** These were found by scanning the whole file
rather than a diff, which is the method that should be used whenever a file is
being audited rather than merely edited.

⚠ Second trap, hit while fixing this one: the removal was first pushed to a
**lane branch**, and reported as done, while `main` still carried the terms.
**For a removal, verify by ancestry against `origin/main`** —
`git merge-base --is-ancestor <sha> origin/main` — never by the success of a push.

**Falsifier:** the identifiers are absent from `git log -p` over all refs, and the
forge returns 404 for the pre-removal blob URLs.

## ⚠ [6.9→6.0] A BARE `unsubscribe` IS NOT A STAND-DOWN, AND THE READ-BACK CANNOT TELL

**Status:** OPEN

*measured 2026-08-14 on this seat's own stand-down; the shape is
[[finding-a-read-back-that-passes-on-its-way-to-being-wrong]]*

A finishing seat ran `ygg-booter.py unsubscribe` **and** `ygg-monitor.py
unsubscribe` and **read both planes back at 0**. One hour later a coverage sweep
wrote a fresh booter subscription for the same row — `kind: monitor`, note
*"unattended delegate, live process"* — and **booted it**. The monitor
unsubscribe held; the booter one did not.

⛔ **No tool defect — the verbs differ and the weaker one was used.**
`unsubscribe` means *stop watching now*; **`optout` means *and do not start
again***, writing `booter-disarmed.tsv`, which **both `subscribe` and `arm`
screen** (`ygg-booter.py:718`). `optout`'s own docstring names the case: *a
delegate that finished its work … arming it would resurrect a row that stopped on
purpose.*

⇒ **A row standing down must `optout --row … --note …`** (or `ygg-claim.sh
--no-booter` from inside). **Fix wanted:** say so at the stand-down step of the
fleet skill, because the failure is silent and the read-back looks right.

⛔ **THE LOOP, which is why it is filed rather than just fixed locally:** the
sweep arms a row **because its process is live**, the boot keeps the process
live, and the standing rule *"a monitor is never finished while the thing it
watches is live"* then reads as justification. **The watch's own subject becomes
the evidence for the watch**, and a genuinely finished seat cannot leave by
unsubscribing.

⭐ **METHOD, and it generalises past the booter: VERIFY A PROHIBITION BY TRYING TO
VIOLATE IT.** A read-back of 0 passed *both* times — it cannot distinguish "not
subscribed" from "not subscribed *yet again*". What settled it was attempting the
thing that must fail: a re-`subscribe` after the opt-out came back **REFUSED**
with the count still 0. **An absence proves nothing about what will be allowed
next; only a refusal does.**

## ⛔ [6.9→6.1] AN UNREADABLE CLIENT-RECORD DIRECTORY READS AS "NO CLIENTS" AND PERMITS RETIREMENT

**Status:** OPEN

*source reading, and the withdrawal of the anomaly that led here, in
[`idle-cost-model.md`](idle-cost-model.md) §6k-1..§6k-3*

`daemon_should_idle_shutdown` (`daemon.rs:11537`) is careful: if
`active_client_instance_records` returns `Err` it returns `false` — *if you
cannot tell whether clients exist, do not retire.*

**The callee guarantees that arm never fires.**
`active_client_instance_records_from_dir` (`lib.rs:20737`) ends every failure in
`let Ok(entries) = entries else { return Ok(()) };`, and drops per-entry errors
with `.flatten()`. It has no `Err` path, so neither does its caller.

⇒ An unreadable client-instances directory — permissions, fd exhaustion, ENOMEM —
is reported as **an empty set of clients**, which satisfies gate 3 and **permits
retirement while a client may be connected**. Two halves of one decision disagree
about what an unreadable directory means and the careless half wins.

⚠ **This is the INVERSE of the "a failing read means it never retires" reading.**
That one cannot happen today; this one can. Both come from the same `Result` being
vestigial.

**Fix:** let the callee distinguish *absent* (legitimately no clients ⇒ `Ok`) from
*unreadable* (⇒ `Err`), so the caller's existing caution becomes reachable.

⛔ **NOT A DEFECT, CLOSED: "a daemon with all three retire gates open did not
retire".** I filed that and it was wrong twice over. `client_instance_dirs_for_scan`
scans **every** directory under the client-instances root, so a record filed under
another endpoint version is still in scope; and `daemon_is_superseded` needs a
**live** newer daemon, which that home did not have. Records non-empty and not
superseded ⇒ gate 3 correctly returns `false` indefinitely. Demonstrated: a daemon
with no record retires at **+90.2 s**, one with a record naming a live process was
still running at **+204.8 s**.
⛔⛔ **And the probe that manufactured it: `/proc/<pid>` existence is TRUE FOR A
ZOMBIE.** The harness called a daemon "still alive at 200 s" that its own trace
shows retiring at +90.2 s. `/proc/<pid>` answers *has this been reaped*, not *is
this running* — and calling `poll()` to check is itself what reaps it. **Prefer
the subject's own lifecycle record over any external liveness probe.**

## ⛔⛔⛔ [6.9→6.7] EVERY PEER `status` POLL REBUILDS THE WHOLE ROW INVENTORY

**Status:** OPEN

⛔ **This heading used to end "AND THE POLLS GO AS N²". That half is RETRACTED
(see below) and was removed from the title on 2026-08-14** — it had been left
standing over a body that withdrew it, which is the most quotable line in the
entry contradicting the entry.

*measured 2026-08-14; derivation, controls and the N=1 arm in
[`idle-cost-model.md`](idle-cost-model.md) §6 — spec is §S5*

**`status` is 70% of every request the daemons serve** (49.7–56.0/s fleet-wide
across two 60 s windows), and each daemon receives **3.4–4.2/s regardless of what
it owns** — a 261-row daemon is polled no more often than a 73-row one.

- ⛔⛔ **"THE COST IS QUADRATIC IN N" WAS FILED HERE AND IS WITHDRAWN.** The
  single-daemon control that supported it also had `OWNED=0, PRESV=0, ROWS=0`, so
  it never separated "no peers" from "nothing to poll about"; and a deliberate
  causal arm (five daemon versions, isolated home, zero sessions, only N varied)
  measured **0.00/s at N=1 and 0.17/s at N=2** against 0.57 predicted — peer
  polling is real but **46x too small** to be the fleet's 3.9/s per daemon.
  ⚠ N could not be raised past 2: a daemon owning no sessions retires.
  ⇒ **What sets the poll rate is OPEN.** Ruled out: bare N (arm), the receiver's
  own sessions/rows (flat across OWNED 1–9, ROWS 70–261), the sender's preserved
  owners (outbound churn flat across PRESV 0–29), client count (3 clients, 1 GUI
  on this host). Untested candidate: the **density of cross-daemon references**.
  ⛔ **Do not plan a drain around an early-payoff curve** — nothing measured
  supports it.
- ⛔ **`fn status(&self)` (`daemon.rs:4154`) is unconditional and rebuilds, per
  call:** `stored_sessions_persisted()`, a clone per row, **`.sort()`+`.dedup()`
  (O(R log R))**, `persisted_state()` — *the entire persistence payload* — and a
  second sort/dedup pass. The reply carries four `*_keys` vectors **and the full
  `PersistedStoredSession` records** (path, kind, id, cwd, title).
- ⇒ **This is BOTH unattributed terms of the cost model at once.** The floor =
  poll rate × per-poll fixed cost (3.6 × ~38 ms = 0.137 cores, against a fitted
  0.116). The per-row term = poll rate × per-row cost (**≈94 µs of CPU per row
  per reply**). ROWS is frozen at each daemon's birth, so a 261-row daemon
  serialises 261 rows on every poll while owning 5.
- ⭐ **The four zero-cost daemons are not a "bare process" baseline** — they are
  on unreachable socket paths, so nobody polls them. **The floor is the cost of
  being REACHABLE, not of existing.**
- ⚠ **It is not demand-driven.** A lead-lag test against a shuffled null puts
  `working_flags`→`status` at ratio **0.82, below its own null**. It is a timer.
  ⛔ **The exact periodic call site is NOT located**: the fan-out helper is
  `reachable_versioned_daemon_statuses[_excluding_endpoint]` (`daemon.rs:13144`)
  and it has **18 call sites**. Finding which one ticks is the first build step.

**Fix (§S5):** move the row inventory off `status` onto a separate `census`
request that only reconcile/handover/adoption call. ⚠ **Version-gate it** — the
fields are `#[serde(default)]`, so a pre-split daemon reading a slimmed reply
sees an empty `stored_terminal_session_keys`, which reads as *"this peer holds no
dormant rows"* — the exact input that has dropped rows in a handover before, and
**14 daemons that will never restart still speak the old shape**.

⛔⛔ **S5 IS DECIDED AGAINST — DO NOT BUILD IT. THE COST IS NOT HERE.** Three
successive estimates of the per-row cost of a `status` reply, each refuting the
last: **94 µs/row** (coefficient ÷ poll rate — a ratio), **~11 µs/row** (naive
fit on `PerfGuard` records), **≈4.65 µs/row** (inverse-probability-weighted
re-fit at 4.71, and an independent unsampled sandbox counter at 4.645 on CPU not
wall, r=+0.9981 — agreeing within 1.4%).

⛔ **The naive fit was biased by the sampling predicate.** `("daemon_request",
"status")` is on `perf_span_is_high_frequency_noise`, and
`perf_span_should_record` keeps a span when `duration_ms >=
NOISY_SPAN_RECORD_FLOOR_MS` (8.0) **or** on a 1-in-50 sample
(`crates/yggterm-core/src/perf.rs:54–116`). ⇒ **records are kept for being SLOW**,
high-row daemons clear the floor more often (13.5–16.8% vs 3.5–7.6%), and the
sampling therefore correlates with the variable being fitted. **It cannot cancel.**
⭐ **Rule: read the sampling predicate before deciding whether sampling cancels —
a duration threshold is a filter on the dependent variable.**

⇒ **All of status serving is ≈0.057 cores of the 3.47-core population (1.6%), and
request serving explains under 1% of the fitted floor. Neither unattributed term
is `status`.** The census split would buy ~1.5% while paying with a version-gated
protocol change whose `#[serde(default)]` failure mode is a documented row-loss
hazard at handover. ⚠ **Reversal condition:** re-open if status serving ever
exceeds ~10% of daemon cost (a far larger population, or rows well beyond ~260).

⭐ **What survives:** the row inventory *is* rebuilt on every reply (O(R log R)
plus `persisted_state()`), which was worth establishing — it is simply cheap.
⇒ **The real question is now open and unowned: ~93% of daemon CPU is
unattributed.** Not request serving (<1%), not daemon churn (5.7%, measured).
`idle-cost-model.md` §6g and §6h.
**Falsifier:** re-run the paired comparison (1-owned daemons, low vs high rows,
both controls in one run). If the per-row coefficient does not fall to ~0,
`status` was not what the row term was buying.

## ⛔⛔⛔ [6.9→6.7] THE LOCK-CONTENTION INSTRUMENT IS THE LARGEST WRITER IN THE SYSTEM, AND IT REPORTS ZERO

**Status:** OPEN

⭐ **S2 landed on `main` (670fa66d) — the falsifier below is UNRUN and unclaimed.**

*measured 2026-08-14; derivation and controls in [`idle-cost-model.md`](idle-cost-model.md) §4*

`lock_daemon_runtime_for_request` (`crates/yggterm-server/src/daemon.rs:17628`)
traces only when `try_lock()` returns `WouldBlock`. Its doc comment states the
fast path therefore costs nothing. **`WouldBlock` fires 322.8 times per second**
on a live daemon, so the fast path is not the path being taken. Each firing
costs a `resolve_yggterm_home()`, two `serde_json::json!` allocations and **two
file appends**.

- `event-trace.jsonl` grows at **95.3 KB/s** and `perf-telemetry.jsonl` at
  **45.9 KB/s** — **12.49 GB/day combined**. ⛔ **A write rate, not disk: both
  streams rotate and hold ~20 MB.** The win is CPU, IO and SSD wear; nothing is
  reclaimed. ⚠ And it is **22x smaller on the desktop host** (6 KB/s vs
  133 KB/s), tracking session count — so it is a fleet-host defect, **not the
  answer to the fan**.
- ⭐ **It is ONE code path and the arithmetic closes exactly.** The `PerfGuard`
  sits inside the same contended branch, so one contention writes three records
  across two files: 141.1 KB/s ÷ 322.8/s = **437 bytes per contention**. A fix
  treating them as two problems leaves 4.1 GB/day behind.
- **93.9% of the lock_wait events report `waited_ms: 0`.** The field is integer
  milliseconds and nearly every wait is sub-millisecond, so **the instrument
  built to measure contention prints zero on almost all the contention it is
  recording.** The count is the signal; the value is blind.
- **98.6% of the contention is `terminal_read`** — reading a PTY serializes
  against every other request through one global `Mutex<DaemonRuntime>`.

⭐ **Fix the resolution and the flush BEFORE touching the lock** (`idle-cost-model.md`
§S2 then §S3): changing contention while the only instrument that can see it
rounds to zero would leave the result unmeasurable.

**Falsifier:** if `event-trace.jsonl` growth does not fall ≥90x after S2,
lock_wait was not 98.8% of the volume.

### ✅ S2 IS FIXED IN CODE — LIVE PROOF OWED (the falsifier above is the proof)

⛔⛔ **"NOT IN RELEASE 3.0.154 — THE CODE WAS NEVER IN THAT BUILD" STOOD HERE AND
IS WITHDRAWN, 2026-08-14 (6.1). S2 IS IN 3.0.154 AND HAS BEEN RUNNING
FLEET-WIDE.** It contradicted this entry's own opening line — *S2 landed on
`main` (670fa66d)* — and the opening line was right.

**The test, with a control that had to fail and did:** the deployed binary
reports its own provenance, so ask it rather than the calendar —
`--build-commit` on the 3.0.154 binary gives `0a912e38`, and
`git merge-base --is-ancestor 670fa66d 0a912e38` answers YES. Control: a commit
known to post-date that build answers NO in the same run, so the probe can
report both outcomes.

⛔ **HOW IT WENT WRONG, because the method was right and the input was not.**
"Verified by symbol, not by commit list" is the correct instinct; the symbol
chosen was `persisted_live_sessions`, which **does not appear anywhere in
670fa66d** — it belongs to different work. ⇒ **A symbol that was never in the
change reads absent on a perfectly correct build**, so its absence was evidence
about nothing. Same family as the field guide's warning that grepping a binary
for a `json!` KEY can read absent on a correct build: **pick a literal the
change actually introduces, and confirm it is present somewhere before trusting
its absence anywhere.**

⭐⭐ **WHAT THIS UNBLOCKS, AND IT IS THE POINT.** The falsifier was believed to
need 3.0.155, which is owner-held — so the proof looked parked behind a gate
nobody may open. It is not: the code is deployed on every host **now**, and the
measured ≥90× drop can be taken against the running fleet without any release.
⚠ Do not carry "S2 ships in the next release" into a brief again; it shipped
in the last one.

⚠ **Separately, and still true:** `origin/lane/dev/6.7-resource` is **35 commits
ahead of `origin/main` and unmerged**, so other 6.7 work is genuinely not in any
build. That is a real fact about that lane; it was never a fact about S2. ⛔ Do
not resolve that lane's merge here — ask for it rebased.

**Status of this entry: FIXED IN CODE, awaiting the measured ≥90× drop.**

All three writes removed in one change, as the correction demanded — the two
`append_trace_event` calls *and* the `PerfGuard` that sat in the same branch.
What replaces them:

- **An interval-flushed aggregate**, per request name: `count`, `mean_us`,
  `max_us`, and p50/p95/p99 as **bucket UPPER BOUNDS** (named that way in the
  payload — a histogram cannot say more than which bucket a percentile landed
  in, and dressing a bucket edge up as an exact figure is how a rounded number
  gets quoted as a measurement). One record per 60 s window instead of three per
  contention.
- ⭐ **Flushed LAZILY, by the next contention — never by a timer.** A thread
  waking to check whether it should flush is precisely the idle cost this lane
  exists to remove. ⚠ Consequence, stated rather than discovered later: a window
  that ends with no further contention is not emitted until the next one
  arrives.
- **`waited_us`, not `waited_ms`** — the unit that made 93.9 % of records read
  exactly 0.
- ⭐ **A wait ≥ 50 ms still writes its own timestamped `lock_wait_slow` record.**
  This function exists because a 34.4 s hold left no trace at all and a starved
  daemon read as an idle one; aggregating everything would re-create that
  blindness at a coarser grain. The forensic case is preserved and the volume is
  not.

⛔ **A pre-existing test REQUIRED the three writes** (`a_request_parked_on_the_
runtime_lock_is_visible_in_the_trace`) and is updated rather than deleted: its
reasoning was right and only its mechanism is superseded, so it now asserts the
aggregate, the slow-path record and the microsecond unit — the same guarantee by
a means that does not scale with contention. ⭐ **A test that encodes WHY should
be re-pointed, not removed**; deleting it would have discarded the argument along
with the assertion.

### ⛔⛔ THE FALSIFIER NEEDS A SAME-MOMENT CONTROL — THE BASELINE MOVES ON ITS OWN

*Peer-measured and independently confirmed here by ancestry, 2026-08-14.*

**Measuring against the original 141.1 KB/s would credit this fix with something
it did not do.** Re-measured with the fix **absent**: 23.6 KB/s combined — a **6×
fall with no code change at all**, because the session population fell 57 → 39
and contention is 98.6 % `terminal_read`, which is **per session**. ⇒ A
before/after taken hours apart on this host measures the session count at least
as much as it measures the change.

⇒ **The falsifier is therefore: a daemon carrying the fix against one that does
not, AT THE SAME MOMENT, normalised per owned session.** A sequential
before/after cannot separate the two effects and must not be quoted as if it
could.

⛔ **AND CHECK DEPLOYMENT BY ANCESTRY, NEVER BY VERSION NUMBER.** `3.0.152` was
built and deployed *before* this fix was committed, so every host reported a
version that looked current while running a binary without the change —
`git merge-base --is-ancestor <fix> <build-commit>` is the only honest test, and
`yggterm --build-commit` is what it needs. This entry was briefly described as
deployed on the strength of the version alone.

⚠ **Trap paid twice while writing this, both caught, both worth carrying.**
(1) The scan read the function's own comment — which necessarily NAMES the
removed writes — as the offence, so it now strips comment lines; the
self-matching-literal warning already in that test was about a string literal
and did not cover a comment. (2) A `cargo test <filter>` that matched a
*different* test was read as a passing mutant check. **Prove the run happened
before reading its silence** — the mutant was in fact caught the moment the real
test name was used.

⛔⛔ **THE BASELINE HAS MOVED, AND MEASURING AGAINST 141.1 KB/s WOULD CREDIT S2
WITH SOMETHING ELSE.** Re-measured 2026-08-14 over 30 s with **S2 not running**
(the live daemon's build predates 670fa66d — checked by ancestry, not by version
number): **event-trace 22.2 KB/s + perf-telemetry 1.4 KB/s = 23.6 KB/s.** That
is already a **6x fall with the fix absent**, because the session population went
57 → 39 and contention is 98.6% `terminal_read`, which is per-session.
⇒ **The falsifier needs a same-moment control, not a historical baseline:**
compare a daemon carrying 670fa66d against one that does not, **at the same
moment, normalised per owned session.** A before/after across hours on this host
measures the session count as much as the fix.

## ⛔⛔ [6.9→6.1] THE DAEMON POPULATION IS 83% OF THE IDLE FOOTPRINT, AND ITS COST IS PER-DAEMON

**Status:** OPEN

*measured 2026-08-14; model, controls and limits in [`idle-cost-model.md`](idle-cost-model.md) §1–§3*

15 census daemons and 57 sessions on one host: **daemons are 3.468 of the
4.167 total cores (83%)**, 74% of it kernel time. Joint model
(**R² = 0.939**): `cores = 0.116 + 0.0104·sessions + 0.000337·rows`.

⛔ **Age explains nothing and its slope is NEGATIVE** (R² 0.323). Older daemons
cost slightly *less*. **This is the opposite of the GUI's shape**, whose idle
cost climbs 7.4x over its life. Two different defects behind one hot machine —
leak-hunting in the daemon is aimed at nothing.

| | daemons | sessions | cores | cores/session |
|---|---|---|---|---|
| legacy (pre-3.0.149) | 14 | 34 | 3.012 | **0.0886** |
| current (3.0.151) | 1 | 23 | 0.456 | **0.0198** |

⇒ **A session costs 4.5x more on a near-empty daemon than on a shared one.**
Consolidating all 57 onto one daemon models at **0.864 cores — 2.60 reclaimable
(75%)**.

⚠ **Two honest limits, both in the model doc:** no census daemon owns zero
sessions, so the floor term is real but **unattributed** between "per-daemon"
and "paid by the first session"; and one outlier daemon is excluded and wants
its own look.

⛔ **This is a priced justification for the lifecycle work, NOT a licence to reap
daemons holding other agents' live sessions.** The constitution's guarantee
governs how they retire.

**Falsifier:** consolidate and re-measure. If daemon cores do not fall ≥2.0, the
per-daemon model is wrong.

### ⭐ THE FLOOR TERM IS NOT INTRINSIC — IT IS CONDITIONAL ON BEING REACHABLE

**Measured 2026-08-14 (6.1), passively, over 60 s windows on 19 daemons.** The
`0.116` intercept above reads as a per-daemon floor and 6.7 measured no intrinsic
floor from inside the request path. Both are right, and this reconciles them:

| what it is | cores | its whole process subtree |
|---|---|---|
| 10 legacy daemons, sockets bound, 73–246 rows each | **~2.5 total** | essentially **idle** |
| 2 current daemons, ~200 descendants between them | **0.26 total** | **2.3 cores of real work** |
| 3 daemons that lost the bind lock (no socket) | **0.000** | idle |

⇒ **A daemon nobody can reach costs nothing at all**, so the floor is not a
property of *being a daemon*; it is paid by daemons that are reachable while
holding a row inventory.

⛔⛔ **AND I WITHDRAW MY OWN NEXT SENTENCE, WHICH READ "the current daemons are
the cheap ones".** Corrected 2026-08-14 against two independent quiet runs (6.9,
`3cb42114`): **per daemon the groups are indistinguishable — legacy 0.189/0.239,
current 0.201/0.227, a ratio of 0.94–1.05x.** My table above is TOTALS, and
reading a per-daemon conclusion out of a total is the arithmetic error the
column headings invite. My single window happened to catch the current pair low,
one of them at 0.077 while it was still starting up.

⇒ **What is actually true: the legacy population costs more because there are
FOURTEEN of them.** The model that survives is

    daemon population cost  ≈  N_reachable × floor(~0.2 cores)

and every other measurement fits it without strain — an unpolled daemon at
0.00017 cores, the unreachable daemons here at 0.000, the request path under 1%,
and the failure of any lane to find a per-request signal, because **the floor is
not spent per request.**

⚠ **The honest limit, carried rather than dropped:** n=2 on the current side and
its per-daemon range (0.116–0.285) overlaps legacy's entirely, so
*indistinguishable* means **no detectable difference**, not *identical*. The
population conclusion does not rest on that comparison — it rests on the count,
14 vs 2, which is not in doubt.

⭐ **The part of my measurement that stands, because it changed the unit of
account:** the legacy subtrees are IDLE while the current pair's carry 2.3 cores
of real work. Every arm before this measured the daemon; none measured what the
daemon was FOR, and a cost model that never looks at the work being served cannot
tell *expensive because busy* from *expensive because numerous*.

⛔ **THIS CORRECTS "THE DRAIN MOVES WORK, IT DOES NOT REMOVE IT."** For the
legacy population the work is not session work — nothing they own is asking for
it — so draining them **reclaims** it rather than relocating it. The earlier
warning stands only for the sessions themselves.

⭐⭐ **S1'S −2.60 CORES IS THEREFORE VINDICATED BY A ROUTE SHARING NOTHING WITH
ITS DERIVATION, AND IT NOW HAS A MECHANISM RATHER THAN A CORRELATION:** 14 legacy
daemons × ~0.19–0.24 cores = **2.64–3.35 cores, genuinely reclaimable**, because
nobody is being served by them. ⛔ The claim to make is NOT "reclaim leaked
cores" — there is still no leak — but **"stop paying a reachability floor
fourteen times over for daemons whose sessions are idle."**

⚠ **The MODEL is settled; the CODE PATH is not, and I am not guessing it.**
Knowing the floor is paid for reachability does not say what spends it. 6.7's
per-connection figure (150–230 µs) is three orders of magnitude too small to
reach 0.2 cores at any poll rate I can justify, and *what sets the poll rate* is
already an open question in the 6.9 lane. Two facts constrain whoever takes it:
**per-thread accounting sees only ~20% of the process total** here (0.054 vs
0.281 cores on one daemon) — which is the documented *cost hides in exited
threads* entry, so ⛔ never price this from `/proc/<pid>/task`; and the
socket-less zeros above make "reachable" the discriminator worth chasing first.

### ⛔⛔ THE DRAIN HAS A VERSION FLOOR: 3.0.32. BELOW IT, SESSIONS CAN ONLY LEAVE BY ENDING

**Measured 2026-08-14 (6.1), two independent artefacts, with a positive control
in the same run.** A session leaves a daemon either by RELEASE (kill + re-resume,
which `session_is_migratable` refuses for anything without a store — every plain
shell) or by the PTY FD HANDOFF, which needs no store. So the handoff is the only
road out for a shell. It does not exist below 3.0.32:

- `strings /proc/<pid>/exe` on the running 2.12.14 daemon: **0** occurrences of
  `pty-handoff`. The same probe on the current 3.0.154 binary: **2**. (⚠ The
  control matters — this literal builds a socket path, so an absent count on a
  correct build was the failure mode to rule out.)
- The oldest `pty-handoff-*.sock` ever created on this host is **`3-0-32`**, and
  **no `2.x` handoff socket exists at all** — an independent artefact agreeing.

**What that costs, against the inventory (34 plain shells and 23 agent sessions
across the population):**

| | daemons | sessions | ~cores | reachable by the drain? |
|---|---|---|---|---|
| pre-3.0.32 legacy | **8** | 17 (14 shells, 3 agent) | ~1.6 | ⛔ **no** — no handoff path exists |
| 3.0.36–3.0.62 legacy | 6 | 13 (11 shells, 2 agent) | ~1.2 | ✅ yes, once 3.0.155 lands |

⇒ **About half of S1's 2.64–3.35 cores is behind a door the machinery cannot
open.** Those 17 sessions end when their shells end, and 14 of them are plain
shells the owner opened weeks ago. ⛔ This is not an argument for killing them —
the constitution's first-class-shell guarantee is exactly what makes it his call.
It IS the argument for stating the price when he is asked: → `docs/owner-attention.md`.

⭐ **ONE daemon is separately and confirmably pathological**: the 2.12.14
instance re-reads **69 MB/s** (rchar, zero disk — all page cache) with three idle
shells attached, against 0.15–1.20 MB/s for every other daemon measured. That is
the full-corpus-read defect executing in the present tense, on a version that
predates its fix. ⇒ It is the first drain target, and its three sessions are
plain shells, which `release_session_for_migration` refuses — so draining it is
an owner-facing question about three shells, not a migration.

## ⛔⛔ [6.9→6.1] 14 LEGACY DAEMONS STILL RUN THE DEFECTS THEIR VERSION PREDATES

**Status:** OPEN

*measured 2026-08-14; attribution in [`idle-cost-model.md`](idle-cost-model.md) §5*

The `DAEMON-1` full-corpus-read defect — `summarize_perf_telemetry` answering a
question about the last 60 s by reading the whole retained corpus every 30 s —
was root-caused and **fixed on 2026-07-26** by `jsonl_read_paths_since`
(`retention.rs:141`). It is **still running in production right now**, in a
daemon that started the same day the fix landed and has never restarted:

| | rchar per 90 s | read syscalls/s | bytes per read |
|---|---|---|---|
| the 2.12.14 daemon (451 h old) | **437.8 MB** | 0.6 | **8.1 MB** |
| a daemon owning 0 sessions (control) | 14.0 MB | 0.2 | — |

437.8 MB/90 s is **above the 312.9 MB/90 s the defect measured when it was
found**. The excess shows as *user* time (0.396 cores, 12x its peers) because
the cost is parsing what it just read.

⛔ **The general form is the defect.** Each of the 14 legacy daemons carries
every fix landed since its own version — 2.12.14 through 3.0.62, against a
current 3.0.151. **The queue and CHANGELOG record these as closed; for 14
processes holding 34 live sessions they are not.** No surface reports which
fixes a running daemon is missing.

⇒ **A third independent argument for consolidation, and the only one that does
not saturate:** cost and row-deaths are bounded by the population size, but the
set of missing fixes grows with every release.

⚠ **Interacts with the lock-tracer entry above:** that tracer fills
`perf-telemetry.jsonl` at 4.06 GB/day, and the retained corpus is exactly what
this monitor parses. Fixing the tracer reduces this cost on every daemon —
including the ones that cannot themselves be fixed.

## ⛔ [6.9→6.7] DAEMON CPU HIDES IN EXITED THREADS — ONE OS THREAD PER CONNECTION

**Status:** OPEN

*derivation in [`idle-cost-model.md`](idle-cost-model.md) §3*

`spawn_unix_client_handler` (`daemon.rs:785`) spawns a **fresh OS thread per
accepted connection**. Measured churn: **4.23 threads/s** on a 1-session daemon,
**25.2/s** on a 23-session daemon, **exactly 0.00 on a daemon owning nothing** —
a clean negative control.

⇒ **Process-level CPU exceeds the sum over live threads by ~5x** (0.202 vs
0.041 cores), across six flat windows, because `/proc/<pid>/stat` counts exited
threads and `/proc/<pid>/task` cannot. **Any per-thread profile of a daemon is
currently missing ~80% of its CPU.**

⚠ **Do not promise cores for the pool fix.** Thread spawn is ~50 µs, so 4/s is
~0.0002 cores. The **~25 ms per handler is the work, not the spawn.** The value
is that CPU stops being invisible to per-thread instruments.

⛔ **UPDATED §6j — the "38 ms" in this entry was a retracted ratio; the measured
figure is 20–44 ms** on a live daemon, ~1.4 ms on a sandbox. ⛔ **The "93.8%
kernel" is RETRACTED** — per-thread tick fields truncate user and kernel
independently and annihilate the smaller below ~10 ms (§6j-3). ⭐ **And a handler on an EMPTY daemon costs 0.70 ms**, so ~95% of the cost
travels with what the daemon HOLDS, not with the connection — a pool would keep
that 95%. ⭐ **The row term is now settled three ways at ~4.5 µs/row** (a causal
seeded arm, an IPW re-fit of field data, and the optimisation lane's own arm), so
rows are ~1.2 ms of a 25 ms thread.

⛔⛔ **THE LEDGER INVERTED 2026-08-14 (§6j-9) — READ THIS FIRST.** Daemon cost is
**N_reachable x a ~0.2-core floor**, not session work. Per daemon, legacy and
current are **indistinguishable (0.94–1.05x)**; the legacy population costs
2.6–3.3 cores because there are **14 of them**, and the owner's subtree
measurement shows their subtrees are **idle**. ⇒ **"The population is expensive
because it is BUSY" and "the drain moves work rather than removing it" are BOTH
WITHDRAWN — the drain genuinely reclaims ~2.6–3.3 cores.** ⭐ **S1's original
−2.60 is vindicated by an independent route and now has a mechanism.**
⛔ My reader-thread headline came from a sandbox where I flooded ptys with `yes`:
**that is the ceiling, not the operating point.** In quiet live data the handler
term is 57–75% and readers are the remainder.

✅ **THE SESSIONS ARM IS RUN (§6j-7) AND THE PER-SESSION TERM IS NAMED: one PTY READER
THREAD PER SESSION**, cost proportional to that session's OUTPUT VOLUME. On a
sandbox with 4 flooding sessions the long-lived threads hold **3.364 of 3.449
cores (97.5%)**, and the top four are **0.838/0.837/0.836/0.836 — exactly one per
session**. ⛔ **CORRECTED: the "25x swings between adjacent windows" that this
originally closed the loop on were MY OWN LOAD GENERATORS** — quiet replication
reads 1.03–1.12x on the same daemons, and only the 2.12.14 outlier bursts (4.15x)
on its own. The per-session reader thread is still the term; the burstiness it was
said to explain was contamination. ⭐ **The observer is part of the baseline: never
take the "after" window of a comparison in the wake of your own generator.**
That closes the loop on
§6i's threads alternating `clock_nanosleep` with on-CPU bursts, and on §4's
contention being 98.6% `terminal_read`.
⚠ **Do not quote 0.84 cores as the cost of a session** — `yes` saturates a pty at
a rate no agent CLI approaches. The transferable claim is the SHAPE (one thread
per session, cost ∝ output), not the magnitude; live per-thread figures are
0.02–0.19 cores.
⛔ **A first pass at this arm read 120 ms/request and was wrong by 29x** — it
divided the daemon's whole CPU delta by a request count while a flooding session
burned CPU regardless of requests. **Measure the no-request background rate at
each rung and subtract it.** The conclusion of this entry is unchanged and now rests on measurement.
⛔ **A version/RSS split of the per-request cost was measured, then FAILED TO
REPLICATE (8.5 ms then 33.3 ms on the same daemon) and is withdrawn** — do not
resurrect it from the first run.


# THE 2026-08-13 BATCH — reported after a restart lost the campaign rows

Fifteen items were reported together, after a GUI restart failed to restore its
rows and the recovery had to be done by hand during a rate-limited window. They
are grouped into the **6.x clusters** the campaign is being run as; the cluster
tag on each entry is the delegate that owns it. The cluster scheme itself is
documented in the fleet skill (`.agents/skills/yggterm-agent-fleet/SKILL.md`
§10 — the N.x orchestrator).

⚠ **The batch's own framing matters:** every one of these was found *while
trying to recover from the previous one*. The restore failure is what forced the
start page to be used as a recovery tool, which is where its ordering and search
gaps were found, which is what made the manual repair take long enough to matter
during a rate limit. Fixing the restore path is therefore upstream of most of
the rest, and 6.1 is ordered first for that reason.

## ⚠ [6.6] A PROCESS-GLOBAL ENV WRITE MAKES THE LAUNCH-COMMAND TESTS FLAKY IN PARALLEL

**Status:** OPEN

Mechanism identified, deliberately not fixed here: it is the Claude Code
daemon-runtime lane's designed round-trip, not the arsenal lane's to change.

`sync_claude_extra_args_for_request` (`daemon.rs`) carries one request's
configured flags by writing them into the **whole process's** environment:

```rust
unsafe { std::env::set_var(ENV_YGGTERM_CC_EXTRA_ARGS, args); }
```

and the launch builder reads them back out of that same environment. The comment
calls it *"same process-wide-env pattern as terminal identity"*, so it is a
pattern rather than an oversight — but a test binary runs every test in ONE
process, so any test that drives a CC daemon-runtime request mutates state that
every other test's launch composition then reads. Which tests lose the race
depends on scheduling.

**Measured 2026-08-14**, same binary, same code, no rebuild between runs:

| run | result |
|---|---|
| parallel ×4 | 2 green; 2 red, on **different** tests each time (`local_cc_relaunch_rebuild_collapses_poisoned_identity_to_row_id`, `refresh_terminal_identity_updates_restored_remote_launch_commands`) |
| `--test-threads=1` ×2 | 1096 passed, 0 failed, both times |

Every test named above passes when run individually.

⇒ **The queue's "flaky in parallel" category is not one property of one test.**
At least this slice of it is a single mechanism with a name, and it is the same
shape as the kind-blind leak fixed alongside this entry: **a per-launch value
carried in process-global state outlives the launch it belonged to.** The
durable fix is to pass the value as a request field the whole way down — which
the generic (non-CC) lane already does, via `configured_override`.

### ⭐ UPDATE 2026-08-14 — THE HARNESS HALF IS MITIGATED, THE PRODUCT HALF IS NOT

`codex_cli::env_test_guard()` (a single mutex over every env-touching test, with
a source scan that fails the build if a *second* rival lock appears) now
serialises the tests that were racing. Re-measured on the merged tree:
**3 parallel runs, 1114 passed / 0 failed, all three** — where the same suite
gave 2-of-4 red before.

⛔ **That is a harness fix, and this entry is not about the harness.** The
production pattern is untouched: a per-request value is still written into the
**whole process's** environment and read back by the launch composer. The guard
makes the tests stop *reporting* it; nothing stops a second request from
overwriting the first request's value in a live daemon. ⇒ Keep OPEN. The durable
fix remains passing the value as a request field the whole way down, which the
generic (non-CC) lane now does via `configured_override`.

⚠ A red parallel run is still not, by itself, evidence of a regression. Re-run
the named test individually before believing it, and quote which way you ran it.

## ⛔⛔⛔ [6.3] A SUBTREE STOPS TRACKING ITS STATE FOREVER — THE BLANK RIGHT RAIL, ROOT-CAUSED

**Status:** OPEN

⇒ Recurrence is fixed in code and sandbox-proven. What keeps this OPEN is that a
GUI which has already taken a fault stays damaged — the owner's has — and no
code change reaches it. Repair, or his next restart, is the cure.

*Owner-reported: the right sidebar has been absent for hours (2026-08-14). It is
his #1 item. The rail is where the damage happened to land; it is not a rail bug
and the next surface it eats will look nothing like this one.*

### ⭐ THE CAUSE, IN ONE PARAGRAPH

Edits reach the webview as batches over a websocket, and the webview
acknowledges each one. If applying a batch **throws partway through**, the
acknowledgement was sent anyway (deliberately — a withheld ack starves the whole
VirtualDom, which is the freeze `vendor/dioxus-desktop` already fixed once). So
the host records those mutations as landed. **Nothing re-sends them, ever** —
the next diff compares against a model in which they arrived. From that instant
one subtree is frozen at whatever it happened to contain, every later patch aimed
at it addresses nodes that were never inserted, and every field in
`server app state` keeps confidently reporting what it *should* be showing.

⇒ The trade was made knowingly and priced wrong. The commit that introduced the
unconditional ack called the cost *"one stale frame"*. It is not one frame; it
is permanent, silent, and unbounded.

### ⛔ THE CONTRADICTION THAT PROVES IT, BEFORE ANY MECHANISM IS ARGUED

`[data-yggui-side-rail-content]` holds exactly two nodes: a Dioxus
`<!--placeholder-->` and the 8 px resize handle. No `[data-yggui-rail-header]`
or `[data-yggui-rail-scroll]` exists anywhere in the document, while
`data-yggui-side-rail-visible` reads `1`.

`visible` and the body are stamped by ONE render from ONE `rail_render_view`
destructure. `docked == true` forces `rendered_mode == requested_mode != Hidden`,
`RightPanelMode` has six variants, the dispatch chain has an arm for all five
non-`Hidden` ones, and every arm opens with an unconditional `RailHeader`. So
`visible=1` with `hdr=0` **cannot both be products of the same render.** They are
not: the attribute landed and the body's mutation did not.

### ⇒ REPRODUCED ON DEMAND, AND IT IS THE 2026-08-08 SIGHTING TOO

In a clean sandbox at the current build, make exactly one edit throw — patch
`Element.prototype.replaceWith` to throw once — then drive the panel:

| | before the fix | after the fix |
|---|---|---|
| host learns of the fault | **never** | `webview_edit_faults: 1` + an `ERROR` log line |
| uncaught errors that follow | 2, rising to 5 while cycling modes | **0** |
| operands abandoned on the interpreter stack | 4 (depth 1 → 5) | **0** (depth 1 → 1) |
| the faulted subtree | frozen permanently | frozen permanently |

With one fault induced, the rail body froze on one mode's header and **never
changed again** through every subsequent `server app panel` command, while
`server app state` reported each new mode correctly. That is verbatim the
discriminator recorded on 2026-08-08 for *"the session metadata rail renders its
header and nothing else"* — **same bug, earlier truncation point.** That entry is
folded into this one; do not re-open it separately.

⇒ The escalation is the abandoned operands. A faulted batch leaves its operands
on the interpreter stack, and every later `replace_with` / `append_children`
splices `stack.length - n` — so one abandoned operand silently mis-addresses
**every batch that follows**, which is how "the header survived" (2026-08-08)
becomes "the whole body is a placeholder" (2026-08-14).

### ⛔ WHAT SHIPPED

- **The ack no longer lies.** It carries a fault byte and the JS error text. The
  host counts it, logs it at `ERROR` with the webview id, and exposes
  `webview_edit_faults` at the top level of `server app state`.
- **A faulted batch no longer poisons its successors.** Operands are popped back
  to the depth the batch started at.
- **One bad buffer no longer eats its siblings.** The drain empties the queue
  before applying it, so an escaping exception used to destroy every buffer
  behind the failing one — batches the host had already been told were delivered.
- ⚠ **The guard had to move branches to be worth anything.** `rafEdits` has a
  `headless` path and a `requestAnimationFrame` path, and `headless` is baked
  into the page from `!window.visible` at load — **it is `true` on every yggterm
  webview, including the visible desktop one.** The first version of this fix
  guarded only the rAF branch and changed nothing in production; the counter
  stayed at 0 through an induced fault. Both branches now share one apply.

### ⚠ WHAT IS *NOT* FIXED, STATED PLAINLY

**A GUI that has already taken a fault stays damaged.** The mutations are gone
and nothing can re-derive them; the host's model is self-consistent and wrong.

⭐ **The divergence is bounded by the GUI PROCESS, and that is measured, not
inferred.** Induce a fault in a sandbox, confirm the rail body frozen on one
header while `rendered_mode` reports another, then kill the GUI and relaunch the
same binary against the SAME `YGGTERM_HOME` and daemon: `webview_edit_faults`
1 → 0 and the header goes `Session Metadata` → `Settings`, matching state, on the
first frame. Same sessions, same rows — only the page rebuilt. ⇒ A relaunch is a
complete cure and nothing short of it is any cure at all. So the owner's rail is still blank
and no code change reaches it — ⛔ and a restart must not be taken from under
him: he has an unsent half-typed draft in a live composer (`owner-attention.md`).

⇒ **What he can do today:** nothing that restores the body — every docked mode is
blank, so `server app panel <mode>` cannot recover it. `hidden` returns the
272 px to the terminal instead of spending them on a blank band, and
`server app panel settings` puts it back after his next restart.

**The open work is repair, not detection:** a resync that rebuilds a diverged
subtree without dropping the page. A `key` bump on the top-level chrome regions
would force Dioxus to tear down and re-create them, which is the shape to try
first. ⚠ Falsifier before building it: the tear-down's own `replace_with` targets
nodes that were never inserted, so prove the repair does not fault on the very
damage it is repairing.

### ⛔ DEAD ENDS — MEASURED, DO NOT RE-DERIVE

- **Not a build regression, and the bisect is retracted.** 4338 changed lines
  across the suspected window touch `rail_render_view`, the `*RailBody`
  components, `SideRailShell`, `RailHeader`, `RailScrollBody`, `rendered_mode`
  and `right_panel_mode` **zero** times, and a clean sandbox at a NEWER build
  draws the rail correctly.
- **Not mode pinning, not data-dependent.** All four built-in modes sampled with
  state and DOM *together* give `requested = rendered = <mode>`, `docked=True`,
  and a placeholder body. `connect` is a static form touching neither
  notifications nor contributed panes.
- **Not a dead webview and not a frozen VirtualDom.** The left tree paints text
  in the same frame, and `data-yggui-side-rail-visible` tracks `docked` exactly
  (1 → 0 → 1) as the mode is driven.
- **Not "the rail is per-selected-session and nothing is selected"** — a clean
  sandbox reports `active_session_path` absent too and still draws its body.
- ⛔ **Detached-node counts are NOT the instrument, and a control says so.** A
  damaged instance had 84 % of its registered nodes detached from the document,
  which reads as decisive until a *clean* instance measures 41 % — Dioxus retains
  entries for removed nodes as a matter of course. Neither is the interpreter
  stack depth: clean sits at 1, not 0. **Read `webview_edit_faults`.**

## ⛔⛔⛔ [6.7] THE READINESS PROBE TYPES OVER THE HUMAN — THIS IS THE "I CANNOT TYPE" BUG, ROOT-CAUSED

**Status:** FIXED IN CODE — LIVE PROOF OWED

*Caught live 2026-08-14 in the owner's own keystrokes, which arrived shredded:*

```
yggterm_ready_probeBy yggterm_ready_probese yggterm_ready_probesi yggterm_ready_probeon…
```

That is `By session…` interleaved with **our own marker**. Reported alongside it:
*"blinking profusely and I could not type"*, and — decisively — *"by session
blinking I mean the VIEWPORT blinking"*.

⛔ **`submit_prompt_echo_verified_with` (`terminal.rs`) writes into a composer a
human is using, and erases it.** Per attempt it writes `yggterm_ready_probe`,
sleeps 180 ms, and if the marker has not echoed writes **`\u{15}` — Ctrl+U, which
clears the line the human is composing** — then retries every ~300 ms until the
timeout. A single 30 s submit against a row someone is typing at is **~100
injected markers and ~100 erased lines**.

⇒ **ONE cause, all three reported symptoms:**

| symptom | mechanism |
|---|---|
| the VIEWPORT blinks | the loop painting the marker and wiping it, ~3×/s |
| cannot type | Ctrl+U erases the line; the marker interleaves with the keys |
| *"5–10 min after a restart"* | rows are not consuming yet, so **every** submit spins its FULL timeout |

⭐ **That last row is the owner's standing #1** (*"all sessions refuse input for
the first 5-10 minutes after a restart"*). Nothing is wrong with the sessions:
after a restart nothing echoes yet, so every automated submit enters its retry
loop at once and the rows are hammered for as long as the timeouts last. **The
restart storm is not a restart problem, it is a retry-loop problem.**

⚠ **Reproduced by the reporter's own instruments, not inferred.** Two
`terminal submit` calls to a busy row each returned `submitted:false` after
`waited_ms: 30103` — the failing case IS the hammering case, and the reply says
"not submitted" while ~200 writes have already landed. ⇒ **`submitted:false` does
not mean "nothing was written"**, and that is exactly backwards from how every
caller reads it.

### The fix

- **`human_draft` guard, re-checked before EVERY write** (not once at the top —
  the echo wait is 180 ms and a person can start typing inside it). `Some(false)`
  = confirmed empty; **`None` is NOT permission** — an unreadable composer is
  where being wrong is least affordable, so it refuses too.
- **New `PromptSubmitOutcome::HumanTyping`**, which **requeues exactly like
  `NotReady`**: nothing was written, so the record is not spent and deferring to
  the person at the keyboard costs them nothing.
- **Exponential backoff to a 2 s ceiling**: same deadline, ~12 writes instead of
  ~100, and the surface is left alone in between.

⭐ **The lock asserts on the WRITES, not the return value** — a version returning
the right enum after already stomping the composer would pass a verdict-only test
and still ruin the sentence someone was typing. **The damage IS the write.**
Proven to fail on the mutant that disables the guard (i.e. on the shipped
behaviour), while the "still submits normally" control passed.

### ⚠ THE GUARD COUNTED THE PROBE AS THE HUMAN — FIXED 2026-08-14, RECORDED HERE BECAUSE THIS ENTRY LOOKED FINISHED

`session_has_pending_input_draft` is reconstructed from whatever passes through
`PtySessionRuntime::write`, and the echo-verified submit wrote its own marker
through that same `write`. So the guard above read back a flag **the probe itself
had just set** and refused its own submit with `HumanTyping { waited_ms: 180 }` —
and 180 ms is `PROBE_SETTLE`, so the number was the mechanism's signature rather
than a measurement. Two `pipeline_integration` tests had been failing on main.

⚠ **Direction matters, and it favours the protection:** the contamination pushed
toward REFUSING to type, never toward typing, so nothing about the draft
protection or the release hold was ever less safe than believed. What it cost is
the repair's EFFECTIVENESS — a composer that has drawn its prompt but is not yet
reading could never be waited out, which is precisely the case echo-verification
exists to catch.

✅ Fixed by the exemption this write path's own comment already describes for
DA/DSR auto-responses: a `write_daemon_originated` that enqueues identically and
does not touch the draft flag, used by the echo-verified submit and the
hot-restart repair. **Only who may SET the flag changed; every reader still
refuses on a real human draft.** ⛔ The tests were not relaxed — the one expecting
a SUCCESSFUL submit is what showed the refusal was wrong rather than renamed.
⚠ Two tests guard the wiring and one guards the semantics, measured: re-pointing
the submit at `write` fails the first two and the third still passes, so neither
is redundant.

### ⛔⛔⛔ THE FIX COVERED ONE OF TWO COPIES — AND THE UNFIXED ONE IS THE COPY THE OWNER HITS

*Found 2026-08-14 after the symptom was reported still live, and still worsening
with uptime, on a build whose installed release already contained the fix above.*

**There are TWO independent implementations of this probe loop:**

| copy | who runs it | reached by | state |
|---|---|---|---|
| `yggterm-server/src/terminal.rs` `submit_prompt_echo_verified_with` | the **daemon** | the hot-restart repair's `continue` | **fixed** (the section above) |
| `yggterm-shell/src/shell.rs` `probe_terminal_input_consumption` | the **GUI** | **`server app terminal submit`** | **was still unfixed** |

⇒ `server app terminal submit` is an **app-control** command, and app-control
commands are handled by the GUI process (the reply carries `handled_by_pid` to
say so). **Every automated submit in the fleet therefore went through the copy
that had never been fixed**, which is why the symptom survived a release
containing the fix. The GUI copy was, verbatim, the original defect:

```
write(marker) → sleep 180 ms → snapshot → write(Ctrl+U) → sleep 120 ms → repeat
```

**≈3.3 marker-and-erase cycles per second for the full timeout**, and its
`guard_draft` parameter was passed `false` by `SubmitTerminalPrompt` **on
purpose**, on the reasoning that a caller told to send text is entitled to clear
the composer line. ⛔ **That reasoning is what the incident refutes:** the
instruction to send comes from an agent, the half-typed sentence belongs to the
person at the keyboard, and a submit that erases it has not done what was asked
— it has destroyed something else. The flag is **removed**, not defaulted: the
refusal is a property of the probe, never a caller's option.

⭐ **The measured cost, from the lock rather than from an estimate:** a 30 s
submit against a silent row went from **200 writes to 24**.

⚠ **This corrects the deploy-ordering note below on one point.** "The GUI
relaunch does not type" is true of the relaunch itself, and the daemon handover
is still the dangerous *event* — but the GUI is the process that performs the
destructive writes for every ordinary submit, so **the GUI build is the one that
has to carry this fix** for the reported symptom to stop.

⭐ **The general shape, and it is the third time this campaign has paid for it:**
a fix proven on one lane is not proven. The commit, the test and the release were
all real; the search for *other implementations of the same concept* was the step
never taken. **Before closing a defect, grep for a second encoding of the thing
you just fixed** — this file's own SSOT law exists because two copies of one
concept drift, and here they drifted into one being fixed and one not.

⛔ **UNTIL A DAEMON CARRYING THIS IS DEPLOYED, THE MITIGATION IS BEHAVIOURAL:**
do not `terminal submit` to a row that is not consuming input. A row reading
`busy: agent_working_daemon` will not echo, so the submit will hammer it for the
full timeout. Check first, and never retry a `submitted:false` by sending again.

### ⛔⛔ THE FIX CANNOT BE DEPLOYED WHILE AN UNSENT DRAFT IS OPEN — AND THAT IS NOT A PARADOX, IT IS THE ORDERING

**A daemon handover TYPES.** §5's hot-restart repair submits `continue` to rows
after a handover (`hot_restart_repair_continue`), through the very function this
entry is about. On any build that does not yet carry the guard — **including
every build installed right now** — that submit has no draft check, so a deploy
can splice `continue` plus its retry barrage into a half-typed sentence.

⇒ **Deploying the fix runs the unfixed path one last time.** The protection
begins at the handover AFTER the one that installs it.

### ⭐ MEASURED 2026-08-14: A **GRACEFUL** HANDOVER DOES NOT TYPE — THE `continue` RIDES THE INTERRUPTED RECORD

*Taken after a full fleet daemon bump (3.0.154, 01:26:57Z) that was performed
while an unsent draft was open — i.e. against this very prohibition. It is
recorded because the outcome narrows the rule rather than excusing the breach.*

| probe | reading |
|---|---|
| `hot-restart-interrupted.json` | **does not exist** |
| `hot_restart_repair_continue` | **0** events, whole corpus |
| `hot_restart_forced_past_deadline` | **0** events, ever |
| events naming the drafted row after the bump | 3, all `live_session_birth` with `launch_now: false` — ledger registration, no write |

**Nothing was typed into any row.** The mechanism: the repair submits `continue`
only to sessions named in the **interrupted-sessions record**, and that record is
written on the **forced** cold shutdown past the deadline. A handover that settles
gracefully interrupts nothing, so it records nothing, so it submits nothing.

⇒ **The dangerous action is a handover that FAILS to converge, not a bump as
such.** ⚠ That distinction is load-bearing and cuts both ways: the daemon that
cannot converge is exactly the one carrying the poisoned-session deadlock filed
by 6.1, so *"a stuck daemon"* and *"the thing that types over a draft"* are the
same population. **The rule to keep is: do not bump while a draft is open UNLESS
the outgoing daemons can be shown to converge gracefully.**

⛔ **This is NOT permission to bump during a draft.** One graceful observation
does not license the general case, and the cost of being wrong is a person's
sentence. It was luck that this handover converged; the check was made after the
fact, which is the wrong order and is the actual lesson here.

⛔ **So while a human has an unsent draft in a live composer, do not deploy a
daemon** — not even this fix, and not "quickly". The correct order is: the draft
is sent or cleared, THEN the daemon is bumped, THEN the guard is live for every
handover after that.

⚠ **This generalises past this entry.** "Do not relaunch the GUI, the owner has
an unsent draft" is the right instinct applied to the wrong process — the GUI
relaunch does not type, and the DAEMON handover does. **A daemon bump is the more
dangerous of the two for a draft**, which is the opposite of how it is usually
treated. Same reasoning as the settle-window asymmetry above: a fix that lives in
the predecessor cannot protect the handover that installs it.

### ⚠ "BLINKING" NOW HAS TWO UNRELATED CAUSES — TELL THEM APART BEFORE FILING HERE

The handover settle window (6.1, `40cbcaf0` + `07218756`) widened the
double-claim window from 250 ms to **~10 s, and it is untested under a live
GUI**. Its predicted falsifier is *rows flickering or doubling for the length of
`YGGTERM_HANDOFF_SETTLE_MS` during a handover* — which a reporter will also call
"blinking", and this entry is what a search for that word finds.

**They are distinguishable on sight, and the difference is the SURFACE:**

| | this bug (the probe) | the settle-window residual |
|---|---|---|
| what blinks | the **VIEWPORT** — composer text appearing and being wiped | the **SIDEBAR ROWS** — entries flickering or appearing twice |
| you can see | literal `yggterm_ready_probe` in the text | row identity/count wobbling, text intact |
| when | any submit to a row that is not echoing | **only during a handover**, for ~10 s |
| typing | broken (Ctrl+U erases your line) | unaffected |

⇒ **Row flicker during a handover is 6.1's falsifier firing: RECORD IT against
the settle window, do not file it here and do not fix it silently.** Composer
text being wiped is this bug.

⚠ **And a deploy of this fix cannot prove the settle window.** The settle fix
lives in the PREDECESSOR, so installed 3.0.148 hands over the old way — the next
bump still runs the old path and the guarantee starts from the one after. Do not
read a clean handover during that bump as evidence for either fix.

## ⛔ `/context` RESETS THE BOOTER'S IDLE CLOCK — inspecting a row blinds the defence that watches it

**Status:** OPEN

**Audited 2026-08-13 on request, and the answer splits — one half is immune, the other is not.**

- ✅ **The anti-flap counter is IMMUNE.** `progress_marks` counts turns that **used a tool or spent
  output tokens** — not bytes, not mtime. Its own docstring records why: *"did the file grow" is NOT
  "did the agent work"*, because a refused turn writes three rows in 5–66 ms. A `/context` produces
  **no mark**.
- ⛔ **The IDLE AGE is mtime-derived** (`turn_state` → `path.stat().st_mtime`). ⇒ **Typing
  `/context` into a row resets its idle clock**, so a dead row that someone inspected looks freshly
  active to the booter and will not be considered for another window.

⚠ **This is a NEW TRIGGER for a defect the code already documents.** The same comment records a
context-exhausted session answering *"Prompt is too long"* in 5–66 ms, writing three rows, resetting
mtime, and being classified `WORKING 0.1m` **while dead for two hours** — the booter kicked that
corpse every ~10 min **for ten hours**. ⇒ **An observation that corrupts the signal beside it**: an
audit sweep running `/context` across the fleet would leave every row it touched looking recently
active while warming none of them.

⭐ **THE DETECTOR THAT WORKS, and it needs no new hook:** the **last transcript record carrying a
`usage` block**. A `usage` block exists only where an inference actually happened, so a slash command
that never reaches the model produces none. Measured on a live row the day of the audit: **26 min by
mtime, 76 min by last real inference — a 50-minute lie.**

**Fix:** publish `last_real_inference_at` as a field **distinct from any file timestamp**, and have
the classifier's age read *that* rather than `st_mtime`. ⚠ The gauge file
(`~/.claude/context-gauge/<session>.json`) already exists and carries `pct`/`used`/`window`/`verdict`
/`dead` — **but no timestamp**, and whether a slash command rewrites it is **untested**. Do not
assume its mtime is safe; it is written by a prompt-submit hook.

## ⛔⛔ THE SPAWN PATH AND THE PROTECTION PATH ARE DISCONNECTED — `terminal new` cannot arm the booter

**Status:** OPEN

Measured 2026-08-13: **47 agent rows on the fleet, 5 subscribed to the booter** — and 4 of the 5
belonged to the campaign that wrote the tool. **~11% coverage on what is described as the last line
of defence**, which makes it a local convention rather than a defence.

⛔ **It is structural, not a discipline problem.** `ygg-claim.sh` arms a row that **claims itself**.
`server app terminal new` — **the verb an orchestrator actually uses to spawn a cluster** — has **no
booter option at all**. So an orchestrator fanning out N rows gets **zero** coverage by default, with
nothing in the verb to remind it. Coverage clusters inside the campaign that knows the tool exists,
exactly as that shape predicts.

**Owner ruling the same day, which sets the target:** *"THE POLICY IS AUTO ARM AND DISARM WITH
REASON."* ⇒ Not "arm it for relay work" — **armed by the act of existing**, with disarming requiring
a stated, recorded reason. This **replaces** the 2026-08-10 opt-in inversion; do not reconcile them.

**Done already (the half that lives outside the binary):** `ygg-claim.sh` arms by default,
`--booter` is a no-op, `--no-booter` requires a reason, refuses to swallow a following flag as one,
and appends it to `booter-disarmed.tsv`.

**What is still owed, and it is in the app-control verb:**
1. `terminal new` arms the spawned row, without a flag.
2. If a flag survives it is `--no-booter <reason>`, and the reason is recorded where the next
   session reads it.
3. ⛔ **`defer` is not `disarm`.** A long build **defers** with a note and **stays armed**. Keep that
   sharp, or a reason-requirement drives people to disarm when they meant to defer.

⚠ **A row that quietly unsubscribed is indistinguishable from one that was never armed** — the same
absent-vs-stalled ambiguity that runs through this queue, and the reason the 42 unwatched rows went
unnoticed.

**Falsifier:** spawn a row with `terminal new` and read `ygg-booter.py list` — it must appear without
anyone having typed a second command.

### ⭐ DESIGN SETTLED 2026-08-14 (6.1), AND IT IS NOT THE VERB — WITH A PREREQUISITE NOBODY HAS WRITTEN DOWN

Measured before deciding, so the next session need not re-derive any of it:

- ⛔ **The product binary has NO dependency on the relay plane today.** Nothing under `crates/` or
  `apps/` mentions `.agents/skills`, `ygg-booter`, or `~/.yggterm/relay` (grepped). Teaching
  `terminal new` to arm makes **yggterm the product** depend on an agent-fleet convention that ships
  in a skills directory — a new coupling in the wrong direction, and a second encoding of the
  booter's subscription record if the Rust side writes it directly.
- ⚖ **The owner's ruling is "armed by the act of EXISTING", and the verb cannot deliver that.** A
  verb arms only what *it* spawned. Rows created by any other path — `ygg-claim.sh`, a restore, a
  future surface — stay unarmed, and coverage again depends on which door was used. **Enumeration
  satisfies the ruling; a verb approximates it.**
- ⇒ **The fix belongs in `ygg-booter.py`: enumerate live rows and arm the unarmed ones**, on `list`,
  on `tick`, and on `status`. That passes the falsifier above (a `list` right after a spawn absorbs
  it), needs no product change, keeps the record schema with its owner, and retroactively covers
  rows that already exist.

⛔⛔ **THE PREREQUISITE, AND IT IS LOAD-BEARING: `booter-disarmed.tsv` IS WRITE-ONLY TODAY.**
`ygg-claim.sh` appends to it (`--no-booter <reason>`); **nothing reads it** (grepped across the whole
skill directory). ⇒ **Auto-arm shipped without a reader would silently re-arm every row that was
deliberately disarmed with a stated reason, one tick later** — turning the owner's "disarm WITH
REASON" into a no-op while appearing to honour it. **Write the reader first, and test the
disarm→enumerate→still-disarmed path before the arming path.**

⛔ **AND THE SAFETY CONSTRAINT THAT MAKES BLANKET AUTO-ARM DANGEROUS:** the booter's whole function
is to TYPE INTO a stalled session. The owner's own interactive rows must never be armed — "never
whoop his viewport" — so the enumerator needs a positive test for *agent* rows rather than "every row
I can see". ⚠ `ygg-babysit.py` exposes only `resolve_row_path` / `row_exists` / `row_host`, no
metadata, so that filter has to come from the raw `server app rows` JSON (kind, tenancy provenance),
not from the existing helpers.

⇒ **Order of work:** (1) read the disarm ledger, (2) agent-row filter with a proven negative case
against a human row, (3) enumerate-and-arm on `list`/`tick`/`status`, (4) run the falsifier above.

### ⛔⛔⛔ STEP (2) IS NOT IMPLEMENTABLE AS SPECIFIED — MEASURED 2026-08-14, AND IT STOPS THE DESIGN

The proven negative case was run, and **the filter fails it.** `server app rows`
was pulled on the GUI host (378 rows) and the human-attended row was compared
field-by-field against a delegate agent row. Every one of the 31 fields is
either identical, free text, identity, or transient:

| identical on both | `kind` (`Session`) · **`icon_kind` (`claude-code`)** · `presence` · `live_member` · `live_keep_alive` · `remote_deploy_state` · `depth` · `child_count` · `document_kind` · `group_kind` · `draggable` · `machine_*` · `host_label` |
|---|---|
| differ, but useless | `busy`/`busy_reason` (**transient** — the human row was merely idle at that second) · `hidden_by_collapsed_set` (a UI folding state) · `outline_prefix` (a seat number an agent types itself) · `label`/`session_title`/`session_cwd`/`session_id`/`path` (identity and free text) |

⇒ **There is no field that says "a person types here."** The owner's copilot row
and an unattended delegate are the same `kind`, the same `icon_kind`, the same
tenancy, the same everything a machine can see. The entry above guessed the
discriminator would be "kind, tenancy provenance" in the raw JSON; **it is not
there.**

⛔ **So `never-arm.tsv` is not a backstop UNDER the filter — it IS the filter**,
and that makes enumerate-and-arm **fail-open**: any human-attended row nobody
has hand-listed yet gets armed, and the remedy for being armed is being typed
over. `never_arm()`'s own docstring already rules on this — *"if an arming path
ever classifies one of these as armable, that path is wrong and nothing
downstream ships"* — and a metadata enumerator classifies every unlisted human
row as armable by construction.

⇒ **Invert it: arm from a POSITIVE ATTESTATION, never from enumeration minus a
deny-list.** Something has to record "this row is an unattended delegate" at the
moment such a row is created, by the thing that knows — the spawner. Enumeration
then still satisfies the owner's *armed by the act of EXISTING* ruling, because
it sweeps for **attested rows whose subscribe never landed** (the real gap the
entry opened with) instead of inventing agenthood for everything it can see.
A row with no attestation is left alone, which is the safe direction to fail.

⚠ **Do not "fix" this by widening the deny-list.** The safety it replaced was an
accident of omission; a hand-maintained list that must be COMPLETE before an
automatic sweep runs is the same accident with more steps.

## ⚠ `terminal submit`'s "no agent composer row appeared" HAS TWO OPPOSITE CAUSES AND ONE MESSAGE

**Status:** OPEN

Reported 2026-08-13, measured across a 3.0.128 → 129 → 132 → 146 → 148 churn inside one session.

The refusal is returned both when the row is **busy** (wait, it will drain at its turn boundary) and
when the row's **daemon socket was superseded by a fleet update** (it will never drain; the fix is a
restart of the terminal). **Opposite causes, opposite responses, one message** — and the message's own
wording (*"the row is mid-output, in a menu, or is not an agent CLI, so input readiness is
unanswerable rather than false"*) is correct about being unanswerable and gives the caller nothing to
act on.

⭐ **The discriminator that worked, and it needs no new verb:** **transcript ROW-COUNT growth.** A busy
row grows; an unreachable one is frozen. Sample twice a few seconds apart.

**Fix:** name the two states in the reply. The daemon endpoint the submit resolved to is already known
at that point, so "the row I addressed is served by a socket nothing has bound" is answerable.

## ⚠ A HANDOVER THAT LOSES ITS SUCCESSOR STILL LOSES THE SECONDS THAT SUCCESSOR ALREADY READ

**Status:** OPEN

The window that destroyed seven agent sessions is closed — a retiring daemon now
holds its descriptors until the successor has SURVIVED a settle interval, and
wakes its own readers again if it has not (see the CHANGELOG entry and
`crate::terminal::ReaderPark`). What remains is smaller and is stated here so
nobody reads the fix as total.

**What is still lost, measured in the sandbox 2026-08-14.** The successor was
signalled 1.5 s into its settle window. All three sessions survived and kept
running on the predecessor — and **two seconds of output were gone from their
scrollback**: the ticks the successor had already consumed in the 1.5 s it was
alive. They cannot be recovered, because the predecessor was deliberately not
reading (that is what stops the two daemons racing for the same bytes), and the
successor died holding them.

⇒ **The trade is explicit: a bounded hole in scrollback instead of the whole
session.** The same run measured `bytes_stolen_after_park: 0`, and the ordinary
handover — successor lives — showed **no gap at all** across the swap.

**Partly measured, and widened by the same change:** the predecessor now lives
~10 s longer while both daemons hold the same runtimes, where before it was
250 ms. Both answer for those sessions during that window.

⭐ **Probed one layer under the sidebar, mid-window:** a client snapshot taken
while both daemons held all three runtimes reported **three distinct session
paths — no duplicate identity.** That is where a duplicated row would have to
come from, so the mechanism most likely to break is measured, not assumed.

✅ **AND THE PIXEL WAS TAKEN — the falsifier ran, 2026-08-14.** A full GUI under a
private headless compositor (`scripts/underglass-sandbox.sh`), four rows on a
3.0.150 daemon, handed over to a 3.0.151 successor. Frames captured before, four
times across the window, and after: **the sidebar rendered the same four rows
throughout — no duplicate row, no flicker, no change in the count.** The
predecessor exited only after `settled: true`, the successor then owned all four,
and `successor_identified: true` — the first live exercise of the pid +
start-time identity path rather than the fallback.

⚠ **One instrument in those frames is NOT evidence and must not be cited:** the
Session Metadata panel showed an identical daemon uptime in the mid-window and
post-window captures, so that panel does not repoll on its own. The row list is
what was measured; the panel's daemon fields say nothing about which daemon was
serving.

## ⛔⛔⛔ [6.7] A ROW CAN BE ALIVE, IDLE-LOOKING, AND NOT READING ITS PTY — THE "I CANNOT TYPE" BUG

**Status:** OPEN

The instance was recovered live; the cause is NOT established.

*Owner-reported and diagnosed live 2026-08-13: "Why cant I type on 2.0 row for over
7m18sec". Owner's ruling: this is not a one-off, fix the root cause.*

An agent row was **wedged**: its CLI process alive, its composer drawn on
screen, and it had stopped reading its PTY. Typing into it did nothing, for
tens of minutes, with no error anywhere.

### ⛔⛔ EVERY STATE FIELD SAID IT WAS HEALTHY. ALL OF THEM WERE TRUE AND ALL OF THEM WERE USELESS

Checked while the row was unusable, and each one said "fine":

| instrument | reading |
|---|---|
| sidebar row | `busy: false`, `busy_reason: idle`, `remote_deploy_state: Ready` |
| CLI process | alive, `state=S`, parked in `epoll_wait` on its PTY — *the normal state for a CLI awaiting keys* |
| daemon PTY masters | held, 3 fds for that tty-index, same as every healthy session |
| `host_stdin_enabled` / `foreground_input_ready` | `true` |
| daemon hot-restart blockers | listed the row as `recently_active`, 82 s |
| transcript | **static for 29 minutes**, last record `system` |

⇒ **`server app terminal input-check` answered it in 6 seconds**, and is the only
instrument that did: `wedged: true`, *"session never echo-confirmed it was
consuming input within the timeout (composer is displayed, so the row is WEDGED:
alive, idle-looking, and not reading its PTY)"*. It **writes a marker and waits
for the echo** — it tests the thing the user does, instead of asking a field.
⭐ A control row in the SAME run answered `wedged: false` with its own named
refusal (it held an unsent draft), so the instrument was not collapsed to a
constant.

### ⛔ WHAT IS ESTABLISHED, AND WHAT IS NOT

**Established:** `FIONREAD` on the session's PTY slave read **0 pending bytes**
while the row was unusable. Had the daemon been writing keystrokes that the CLI
was not reading, they would be queued there. **The bytes never reached the PTY.**
So the break is upstream of the CLI, not inside it.

⚠ **NOT established: where upstream.** This is a `remote-cc` row, so the path is
GUI → its daemon → ssh → the session host's daemon → PTY, and this entry does
**not** name which hop dropped the bytes. ⛔ Six causal stories have collapsed on
this campaign in one evening; this is not the seventh.

⭐ **The shape worth testing first**, because the code says it is possible:
`enqueue_terminal_write` (`terminal.rs`) returns `Ok(())` as soon as
`try_send` succeeds, for every non-`Flushed` write. **It reports the REQUEST,
not the EFFECT.** A keystroke accepted into a queue whose consumer is not
draining is indistinguishable from a delivered one until the queue fills — and
only then does it report backpressure. That is the same family as
[the queue whose consumer is older than its producer]. Falsifier: instrument
queue depth against the writer's drain marker on a wedged row.

**Recovery that WORKED, in order** — worth automating, since today it is manual:
1. `input-check` to confirm the wedge (6 s, non-destructive, submits nothing).
2. Confirm the transcript is **static** — proof the agent is not mid-turn, so
   nothing is lost. ⛔ Do not skip this: killing a working agent destroys a turn.
3. Kill the wedged CLI **by PID** (`--resume` replays the transcript from disk).
4. `server app open` the row — it re-resumes clean.
5. `input-check` again: `consuming_input: true`. **That is the proof, not the render.**

### ⇒ WHAT THIS COSTS UNTIL IT IS FIXED

A wedged row is **invisible**: it reads `idle · Ready` in the sidebar. Nothing
runs `input-check` on a quiet row, so the only detector is a human trying to
type and failing. **The sidebar should say WEDGED, not `idle`**, and a row that
looks idle with a composer shown is exactly the cheap trigger for an automatic
`input-check`.

### ✅ THE PASSIVE DETECTOR NOW EXISTS — and it found a second defect on the way

⛔⛔ **`last_activity_ms` was stamped by the WRITER as well as the reader**, so a
row that has stopped reading its PTY looks **maximally active for exactly as long
as a human keeps typing into it**. The daemon could not tell a live session from
a wedged one, *and the owner's own keystrokes were what kept the dead row looking
alive.* ⇒ that is why the hot-restart gate listed the unusable row
`recently_active` **and let it block a deploy**.

**Shipped:** `last_output_ms`, stamped only on the reader side (5 reader sites +
`seed_snapshot`), never by `write()`. From the pair:
`input_unanswered_ms()` — how long the row has been written to without answering
— and `wedge_suspected(threshold)`, on both the runtime and the manager.

⭐ **It writes nothing into anyone's session**, needs no marker, and needs no
human to notice. It is the passive form of what `input-check` establishes by
typing and waiting for an echo. ⚠ **A trigger, never a verdict**: a child may
legitimately take input and stay silent (password prompt, echo off), so this
points at a row and `input-check` settles it.

**Locked by** `input_that_goes_unanswered_is_visible_without_typing_a_marker`,
which asserts the invariant the split exists for — **a write moves the input
clock and must NOT move the output clock** — plus both threshold directions, so
a detector wired to a constant cannot pass.

### ✅ THE FIRST CONSUMER IS WIRED — the deploy gate no longer jams on a deaf row

⭐ **This is the owner's symptom seen from the inside.** He reported *"all sessions
refuse input for the first 5–10 minutes after a restart"*. It presents as a
**duration** rather than a failure because his keystrokes kept the dead row
looking alive, nothing on the host could tell a deaf row from a busy one, and it
only ended when something else re-resumed the session.

The `recently_active` hot-restart blocker read `session_idle_for_ms` — the
conflated field — while `HotRestartBlocker::idle_ms` was *documented* as "how
long since this session last produced output". **The doc was right and the code
was not.** ⇒ the unusable row reported near-zero idle for as long as it was typed
at, and blocked the one mechanism that would have cleared it.

**Now:** the blocker reads `session_output_idle_for_ms`, so the field means what
it says, and carries `input_unanswered_ms` beside it — a large value next to a
small `idle_ms` is the deaf-row signature, visible without anyone typing a probe.
⚠ A drafted input line is protected separately and deliberately
(`session_has_pending_input_draft`), so reading output-idle here does not expose
an unsent prompt to a restart.

**Locked by** `a_row_being_typed_at_is_idle_by_output_even_though_activity_says_busy`,
which asserts **both clocks from the same state** — activity says "busy 0 ms ago",
output says "silent 5 s" — so a reader wired to either field alone cannot pass.

⇒ **Still open:** the SIDEBAR. A wedged row still renders `idle · Ready`, so the
human remains the only detector of the state the daemon can now see. That is the
next wiring, and `input_unanswered_ms()` is already there to read.

## ⛔⛔ [6.8] THE YEDIT DOCUMENT SURFACE PAINTS GARBAGE WHILE ITS MODEL HOLDS 34 CHARACTERS

**Status:** OPEN

*Owner-reported 2026-08-14 with two screenshots, on a FRESH GUI at 3.0.154 — so it
is **not** the stale-binary defect fixed the same morning.*

**The Document view of a `New Yedit` row renders a screenful of corrupted glyph
clusters** -- dense mojibake laid out in terminal-like rows across the top of the
canvas -- while the surface's own status bar reads:

```
7 words · 4 lines · 34 chars
```

⇒ **The model is tiny and the RENDER is garbage.** Whatever is being painted is not
the document.

### ⭐ The discriminating evidence, and it is already in hand

- **The Terminal view of the SAME row is clean.** It shows the launch cleanly:
  `yedit: document surface opened` preceded by a secrets-fetch failure the app
  emits at startup. So the row, the PTY and the terminal renderer are all fine.
- ⇒ **The corruption is confined to the Document surface**, and the layout of the
  garbage is terminal-shaped -- cells, not prose.

### Falsifier / where to start

1. Open a document surface on a scratch row and capture a **faithful** frame
   (`capture_faithful: true`, backend `xterm_canvas_composite_over_dom`). ⛔ A
   `faithful:false` frame is canvas-blind and cannot settle this.
2. ⚠ **Suspect the compositing path before the text path.** The screenshot backend
   composites the xterm canvas with the DOM; a Document view should not be painting
   terminal cells at all, and what is on screen looks like exactly that -- a cell
   grid full of uninitialised or mis-decoded content.
3. Check whether the status-bar counts and the painted content come from the same
   buffer. They disagree by three orders of magnitude, so at least one of them is
   reading something it does not own.

⚠ **The owner also reports a recurring dropped-glyph issue alongside this.** Do not
assume the two are one defect: dropped glyphs were also visible on the stale
12-hour GUI and improved when it was retired, so there may be a render-path bug
that the stale binary made worse rather than caused.

## ⛔⛔ [6.7] THE BOOTER TYPES A BARE `\r` INTO A ROW THAT MAY BE SHOWING A BILLING CHOICE

**Status:** OPEN

*Owner-raised 2026-08-14, and the owner named the mechanism himself.*

*His question: when a session hits the plan limit, the CLI shows a three-option
prompt -- stop and wait for the reset, switch to a team account, or use API
billing -- and the first must be dismissed with Enter. "I was thinking when booter
hits such a session, what happens. Maybe only the question gets dissolved with the
passed return key and on the next booter the actual booting happens ... It happened
to us last night and I do not know how all the sessions recovered."*

### ⛔ THE ANSWER IS WORSE THAN THE HYPOTHESIS, AND THAT IS WHY THIS IS FILED HIGH

`wake()` writes a message and then a **lone `\r`**. Against a row parked on a
modal choice, that `\r` **selects whatever option is highlighted**. The owner's
model -- one boot spent dismissing the dialog, the real boot on the next tick -- is
correct *only if the highlighted option is the harmless one*.

⇒ **If the highlight is not on "stop and wait", a watchdog silently selects a
BILLING CHANGE on the owner's account.** No agent decided that; a timer did. That
is categorically different from a wasted boot, and it is the reason this cannot sit
behind "it seemed to recover".

### ⚠ THE EXISTING QUOTA GUARD DOES NOT COVER THIS

The booter already classifies `RATE_LIMITED` and holds the whole fleet on one
sighting -- but it is keyed on the CLI's own `apiErrorStatus: 429` record, **not on
the interactive dialog**. A row parked on the choice prompt has not necessarily
recorded a 429 in the shape the classifier reads, so the guard can be entirely
correct and still not fire on the state the owner is describing.

⛔ **And the shapes are indistinguishable from outside:** a row sitting on a modal
is idle, its transcript is not growing, and its last record is a completed turn.
That is the same signature as a stall, which is exactly what the booter exists to
act on.

### Falsifier / where to start

1. Reproduce the dialog on a scratch row (never the owner's) and read what the
   screen holds -- `terminal read-buffer <row> --mode screen`. The dialog text is
   on the screen even when the transcript says nothing.
2. ⭐ **Screen-content detection is the discriminator, and it is cheap.** The
   classifier reads the transcript; this state is only visible in the *screen*.
3. ⛔ **Then REFUSE, do not get clever.** A row showing a choice the user must make
   is a row the watchdog must not touch -- the same rule as the never-arm ledger,
   for the same reason: this thing types.

### ⇒ THE SPEC, so it is settled before the code is

- ⛔ **Never send a bare `\r` into a row whose screen is showing a prompt this
  watchdog did not put there.** Detect, hold, and report.
- ⛔ **A quota window is not a stall and must never be booted** -- the account
  cannot spend, so waking the session buys a refused turn and burns a boot.
- ⭐ **Recovery must be OBSERVABLE.** The owner's real complaint is not the boot,
  it is *"I do not know how all the sessions recovered."* Whatever the booter does
  here has to leave a record that answers that afterwards.

## ⛔⛔ [6.7] APP ROWS ARE BORN `keep_alive: false`, SO A GUI RESTART DESTROYS THE USER'S OWN GROUP

**Status:** OPEN

*Owner-hit 2026-08-14. He lost a row group he had built -- one `New Yedit` header and
four `New Ychrome` rows -- to a GUI restart, and reported it in those words: "a row
group of mine is gone ... I cannot see it after the restart".*

**The rows were `kind: shell`, `keep_alive: false`.** Second-class rows die with the
GUI by design, and that design is correct for a scratch shell. It is wrong for a row
the user **deliberately created from an app verb** and arranged into a group: from his
side there is nothing "scratch" about it, and nothing in the UI says it is disposable.

```
"title": "New Ychrome", "kind": "shell", "keep_alive": false, "cwd": "/home/user"
```

⛔ **THE CONSTITUTION SAYS THE OPPOSITE IN AS MANY WORDS:** *"Plain shells are
first-class and must survive a bump like anything else."* These did not.

⚠ **It is not recorded as a removal either.** `removed-rows.json` has no entry for
them, so nothing distinguishes "the user closed this" from "this evaporated" -- and
`sessions restore` answers `not_found`, because the ids are gone from the GUI's live
set even though `server-state.json` still lists them under `live_sessions`. **Two
stores disagree about whether the row exists**, which is its own defect.

### What is NOT lost, and what is

- ✅ **Profile data is safe.** Each row was a shell running `ychrome`; the profile
  directories persist on disk. Nothing the user had stored was destroyed.
- ⛔ **Which profiles were open is NOT recoverable** from state: the persisted record
  carries `title`, `kind`, `cwd` and `ssh_target` and **no app arguments**, so a
  restore cannot reconstruct which profile each row held.

### Falsifier / where to start

1. `server app launch-app ychrome new`, then read the row back out of
   `~/.yggterm/server-state.json` -- it must not be born `keep_alive: false`.
2. There is **no CLI verb to set keep-alive on an existing row** (`--keep-alive` is
   documented only as unnecessary for agent-CLI kinds, which are born keep-alive).
   A user who wants to protect a row they already have cannot.
3. The persisted record needs to carry the app **verb and args**, or a restore can
   only ever produce a blank instance of the app.

⇒ **Three separable fixes, and the first is the one the owner felt:** rows created
from an app verb are born keep-alive; a keep-alive toggle exists for any row; and the
stored record round-trips the launch arguments.

## ⛔⛔ [6.7] THE GUI IS BURNING A WHOLE CORE, AND IT IS USER-TIME, NOT SYSCALLS

**Status:** OPEN

*Measured on the live desktop host 2026-08-13, windowed `/proc/<pid>/stat`.*

| | |
|---|---|
| total | **99.9% of one core** |
| user | **88.3%** |
| kernel | 11.6% |
| GUI age | 3,599 s |

Recorder agrees independently: `gui total=99.4% user=87.9` over 30 samples /
5 min, against `web_content` 24.1%. Earlier the same evening the same GUI read
**13.9%**.

⛔ **This is a DIFFERENT shape from the regression this lane already documented.**
That one was kernel-dominant (58.8% kernel, `clock_gettime` at 95.8% of syscall
time, amplified 45.8× by an `hpet` clocksource). This is **user-dominant by 7.6×**
— compute, not syscalls — so the clock-price finding does not explain it and the
established fix does not apply.

The busiest thread by lifetime context switches is the **main `yggterm` thread**
(744,729), ahead of `ReceiveQueue` (511,777) and the tokio workers.

### ✅ THE PROBE IS ALIVE, AND IT RECORDED A 21-MINUTE STORM WITH NO CAUSE

*Answered on the live host 2026-08-13 23:18–23:46. `app_render_rate` is emitting
normally — the earlier silence was a query fault, not a dead probe.*

The whole life of one GUI process, one sample per minute:

| window | renders/s | what it is |
|---|---|---|
| the long middle | **1.0 – 3.0** | rest, and it agrees with the ~2/s already on record |
| brief spikes | 40 – 50 | transient, self-clearing within one sample |
| **23:18:23 → ~23:39:40** | **69.3 – 79.6, unbroken for 21 min** | the storm |
| after | 1.6 / 4.6 / 2.6 | rest again |

⛔ **IT LATCHES, AND IT LETS GO ON ITS OWN.** This is not a slow climb and not a
spike — it is a step change into a stable high regime and a step back out, with
no deploy, no restart and no operator action at either edge (the offset preceded
the session's first command on the host by ~80 s). A *rate* that is flat while
it is wrong is the signature of a self-sustaining loop running at whatever a
full-tree re-render costs (~13 ms), not of a leak and not of a growing backlog.

⇒ **The regime table above is the answer to the REGIME warning, not a bypass of
it:** rest and storm were sampled by the same always-on probe in one unbroken
series, so the two figures are comparable. The ≥20/s autopsy arm is what cannot
sample rest; `app_render_rate` can and does.

### ⛔⛔ THE STORM IS INVISIBLE TO EVERY INSTRUMENT WE HAVE — THAT IS THE FINDING

Three autopsies inside the storm (23:09:34, 23:27:15, 23:37:30) each report the
same thing, and it is not nothing — it is a very specific nothing:

```
renders_observed: 512   unattributed: 511   forced_wakes: 0
changed_fields: {}      shellstate_mut: {}
```

**512 consecutive full re-renders in ~6 s during which not one of the 26 watched
`ShellState` fields changed hash, no attributed write occurred, and our own
`schedule_update()` was never called.**

And an every-event-kind correlation over the storm against the rest windows
either side finds **no cause there either**:

| event kind | pre /s | STORM /s | post /s | ratio |
|---|---|---|---|---|
| `render_fail_pattern/detected` | 0.00 | 0.02 | 0.00 | 4.1× |
| `app_control/request_stage` | 0.54 | 0.71 | 0.56 | 1.3× |
| `terminal_io/dispatch` | 2.14 | 2.61 | 1.24 | 1.2× |
| `app_declare/daemon_declare_absent` | 0.85 | 0.84 | 0.59 | 1.0× |

The only kind that rises is **the storm detector reporting the storm** — self-
referential, not a cause. Nothing else moves. A 35× step in render rate is
accompanied by no change in any traced activity whatsoever.

⇒ **Read against the autopsy's own documented rule** (`forced_wakes` ≈ 0 beside a
high `unattributed` count ⇒ *the wakes come from inside Dioxus, a future or eval
resolving, not from us*), the evidence points at a Dioxus-internal waker. ⚠ Held
as the leading hypothesis, not a verdict: the fingerprint cannot yet separate it
from **a raw `state.with_mut()` write to an unwatched field**, which would print
exactly the same empty autopsy. `SHELLSTATE_MUT_TOTAL` is in the watched set but
is bumped only by `safe_shell_mut` — and `crates/yggterm-shell/src/shell.rs`
holds **516 raw `.with_mut(` sites against 130 `safe_shell_mut`**, so the
attributed path covers ~20% of the writers.

⇒ **The next deliverable is the discriminator, not another measurement.** Until a
write to `state` is unmissable, both hypotheses print `{}` and a fourth autopsy
will die exactly as the first three did. ⛔ Do not "fix" this by capping the
render rate: that hides a loop whose cause is still unnamed.

⚠ **A falsified guess, recorded so it is not re-run.** `daemon_declare_absent`
looked like the engine — 112 events in the onset window, and the first three are
~24 ms apart, which matches the render period. It is not: across the storm it is
**flat at 0.8/s**, identical before, during and after. Three clustered samples
are not a rate. ⭐ *Take the rate over the window, never off the first few
timestamps.* (It is separately worth asking why a declare against an absent
daemon retries forever at 0.8/s for one `remote-cc` session, but it is not this.)

## ⛔ [6.7] A RESTART THAT RESOLVES NO RUNTIME SHUTS NOTHING DOWN AND REPORTS SUCCESS — FIXED IN CODE

**Status:** FIXED IN CODE — LIVE PROOF OWED

⛔ **HARVEST ATTEMPTED IN THE 3.0.154 WINDOW — NOT TAKEN.** The verb itself is
unreachable from the CLI on dev: `server terminal restart` fails with *"local
yggterm daemon did not become reachable"* — and so does `server terminal restart
--help`, before it parses anything — while `server daemons` and `server status`
answer normally from the same binary against the same running daemons. ⇒ The
proof is blocked on a second defect in the verb's own reachability, not on the
fix. **That blocker is now the thing to fix first**, and it is not this entry.

*Found by using the remedy above and watching it fail, 2026-08-13.*

`input-check` diagnoses a wedge and recommends
`server terminal restart '<session>'` as the remedy. **That verb could not clear
a wedge**, and said it had:

```
{"accepted":true,"message":"restarted remote-cc://dev/…; launch_refreshed=false; …"}
```

After it: the wedged `claude` was **still alive at the same pid**, still owning
`/dev/pts/5`, still wedged — and a **second** wrapper had been spawned beside it.

**The mechanism**, `restart_session_with_size` (`terminal.rs`):

```rust
if let Some(runtime) = self.sessions.remove(key) {
    runtime.shutdown(stop_command)?;   // skipped entirely when the key does not resolve
}
let runtime = PtySessionRuntime::spawn(...)?;   // replacement spawned regardless
```

`remove` answers `None` for any key this manager does not hold — an orphaned
key, or a `remote-*` row whose runtime belongs to **the daemon on its own host**.
The restart then shuts nothing down, spawns a replacement, and the process that
was serving the key is left alive and orphaned beside its successor. ⇒ **It is
not a restart, and calling it one is why a wedged row survives its own remedy.**

**The fix.** `restart_session*` now returns `TerminalRestartOutcome
{ replaced_existing }`, the daemon's reply carries `replaced_existing=<bool>`,
and a `restart_replaced_nothing` trace event fires on the empty case. The verb
still spawns — recovery legitimately promotes a scanned remote session that has
no local runtime yet — but it can no longer *claim* to have restarted something
it never touched.

**Locked by** `a_restart_reports_whether_it_replaced_anything`, which asserts
**both halves**: an unheld key reports `false`, and a restart over a runtime the
manager holds reports `true`. Without the second half the test would pass on a
constant.

⚠ **Not claimed:** that this fixes the wedge. It fixes the *remedy's honesty* —
the operator is no longer told a wedged row was cleared when it was not, and the
`false` case now names the real question: **which daemon owns this row.**

## ⛔ [6.7] AN AGENT OPENING A ROW STEALS THE OWNER'S KEYBOARD

**Status:** OPEN

*Observed 2026-08-13 while diagnosing the entry above.*

The owner reported he could not type into a row. The row was fine: the GUI's
`active_session_path` had moved to **a different agent's row**, so his
keystrokes were being delivered to another session with no visible sign of it.

⇒ Two things are separate and are treated as one: **the row the sidebar shows as
selected** and **the session that receives the keyboard.** They drift apart, and
nothing surfaces the disagreement.

**What moves it:** `server app open` re-targets the keyboard, and every agent
row can call it — the diagnosis above did it twice to the owner's own session,
and `server terminal restart` moved it a third time as a side effect.

⚠ **Measured, so this is not folklore:** with no agent touching rows, the active
session held steady across 16 samples over 64 s. It moves when agents act.

⇒ **An agent-initiated activation must not take the keyboard from a human.**
`terminal new` already has `--no-activate`; `open` has no such affordance, and
the side-effecting verbs do not declare that they steal focus at all. This is
the plainest form of the standing "never take his viewport" rule, and the row
plane currently cannot honour it.

## ⛔ THE HOOK INSTALLER EXISTS TWICE, THE TWO COPIES DISAGREE, AND ONE CRASHES ON A WORKTREE

**Status:** OPEN

Measured 2026-08-13, both directions, one run.

The pre-push leak gate can be installed two ways and they are different code:

| | `ygg-privacy-guard install` | `scripts/install-privacy-guard.sh` |
|---|---|---|
| normal checkout | works, and reports PUBLIC/private | works |
| **git worktree** | ⛔ **raises** — it builds `<repo>/.git/hooks`, and in a worktree **`.git` is a FILE** | works — resolves via `git rev-parse --git-common-dir` |
| non-github remote | installs anyway | skips |

⇒ **This is a second encoding of one chore and it diverged immediately.** The shell copy was written
without checking that the guard already owned the job, and its first version invoked the guard as
`pre-push` — a subcommand the guard does not accept — so the guard printed its usage text and exited
non-zero, **which git reads as "the hook refuses". Every repo it touched could no longer push at
all.** Fixed within minutes, but the shape is the point.

⚠ **It failed CLOSED, which is the correct direction for a leak gate**, so this was an availability
fault and never an exposure. ⭐ **The tell worth keeping: an inert hook and a working one are
identical on disk**, and the broken one emitted a wall of the guard's own text that looked exactly
like the guard running. **An installer is proven by a real push or not at all** — a syntax check and
a successful write prove nothing about whether the hook works.

⚠ **The coverage gap this exists to fix is now CLOSED, and the first figure reported for it was
wrong in both directions** — worth recording, because it is the same disease as the rest of this
entry. *"2 of 34 github-remoted repos"* **understated** it by counting repos rather than checkouts
(one hook covers 13 lane worktrees via `core.hooksPath`, which is where the lane work actually
happens) and **overstated** it by putting 22 third-party clones in the denominator, where a leak
gate is meaningless. **The honest figure was 8 of our own repos unguarded**, 7 of them public.
✅ All are guarded now, each verified by invoking the hook directly rather than by observing that a
file exists.

⚠ **And the coverage detector itself lied twice:** `git rev-parse --git-common-dir` returns a path
**relative to the repo**, so resolving it against the caller's cwd makes every lookup miss and
answers "no hook" for repos that are guarded — use `--path-format=absolute`. ⭐ **The control that
caught it costs nothing: point the sweep at a repo whose answer you already know.** Three were known
and it disagreed with all three.

**Fix:** collapse the two into the guard (it is the owner), taking the worktree-correct path
resolution and the github-remote filter with it, and delete the shell copy. Blocked on the guard
getting a tracked home — see `owner-attention.md`.

⛔⛔ **AND THE TRAP IN INVESTIGATING THIS IS WORSE THAN THE BUG.** A probe that reconstructed the
hook's invocation with a grep that dropped the `hook` subcommand called the guard bare, got exit 2
and a wall of usage text — **reproducing the broken-installer symptom exactly, on a HEALTHY repo.**
Stopping there would have reported the fix as failed, with evidence.

⇒ **The guard's usage output is not evidence of a broken hook. It is evidence that SOMETHING called
it wrong — and the caller is as likely to be your probe as the installer.** A probe that calls the
thing slightly wrong is indistinguishable from the thing being broken, because both produce
usage-and-non-zero. What separates them is reconstructing the REAL call — argv plus the ref line on
stdin, the way git actually invokes it — rather than an approximation of it.

⭐ **And a hook must be proven in BOTH directions, because either alone proves nothing:** a hook that
refuses everything passes the block test, and a hook that does nothing passes the pass test. Plant a
term → must refuse; clean range → must push.

**Falsifier:** run each installer against a git worktree and against a plain checkout in the same
run, then push from both.

## ⛔⛔ [6.7→6.1] EVERY DAEMON BUMP ORPHANS EXACTLY ONE VERSION — THE ONE THAT WAS SERVING

**Status:** OPEN

*owner-reported 2026-08-13 19:15 ("I can't type in many sessions"). The outage
itself is over; the mechanism that caused it is untouched and recurs on every
deploy. Filed from 6.7, belongs with the daemon-lifecycle work.*

### The defect

Sessions address the daemon by a **version-keyed socket path**
(`server-3-0-<n>.sock`) — the version string is a rendezvous key, already a filed
bug class. `refresh_legacy_server_socket_aliases` (`socket_sweep.rs`) keeps older
versions reachable by **aliasing every version that has no socket** to the live
one.

⛔ **A gap-filling pass cannot cover its own predecessor, because a REAL SOCKET
IS INVISIBLE TO A PASS THAT ONLY FILLS GAPS.** The version serving at the moment
aliases are written is correctly skipped — it is not a gap. Then it retires, its
real socket is unlinked, and nothing ever replaces it.

⇒ **Every bump orphans exactly one version: whichever was serving.** Any client
pinned to it strands, and **a snapshot mitigation re-breaks itself once per
deploy, forever.**

**Observed end to end on the desktop host, 2026-08-13:**

| | |
|---|---|
| 22 aliases written while **3.0.130 was live** | 130 correctly skipped — it was real |
| 130 retired, socket unlinked | nothing replaced it |
| clients pinned to 130 | `Error: connecting to …/server-3-0-130.sock` |
| slots 126–129 | all aliased and healthy |
| slot **130** | **missing** — the one that had been serving |

⭐ **This also falsifies the hope that the aliases carry themselves forward**: the
written symlinks *are* surviving files, so the next sweep should inherit them —
but the bump was watched and **130 was orphaned within minutes anyway**, because
its slot was never a gap when the pass ran.

### The fix

**A retiring daemon must alias its own version to its successor as part of
retiring** — that is the only moment the system holds both names. Anything done
later is a sweep guessing from whatever files remain.

⛔ **And the enumeration defect underneath stays open regardless:** the candidate
set is derived from **surviving socket files**, so it inherits the sweeper's
retention policy — ancient sockets were never cleaned and get aliases, recent
ones are deleted on each bump and get none. **The museum stays addressable while
the current generation is orphaned.** ⇒ **Enumerate from the versions that
EXIST, never from the files that happen to REMAIN.**

### ⭐⭐ THE ARROW RULE — which alias is legitimate

**An alias pointing at a NEWER daemon is the design. An alias pointing at an
OLDER one is a proxy wearing the design's clothes.** Both directions were live on
the fleet within one hour and the same rule decides them opposite ways:

| | daemon | clients want | alias arrow | verdict |
|---|---|---|---|---|
| desktop host, 19:15 | 3.0.118 | `…-128.sock` | → **older** | ⛔ **refused** |
| build host | 3.0.130 | `…-128.sock` | → **newer** | ✅ created (30) |
| desktop host, 19:45 | 3.0.131 | `…-130.sock` | → **newer** | ✅ created |

⇒ **Same rule, opposite answers, because the FACT changed and not the rule** —
which is the argument for stating rules as directions rather than as verdicts
about particular versions. Refusing the first was right: a current-client →
older-daemon proxy has already returned nothing silently here, and it would have
converted loudly-broken rows into possibly-silently-wrong ones while hiding the
defect. Guard every such alias with an abort unless the live daemon has a **real,
non-symlink** socket.

### ⛔⛔ TWO INSTRUMENTS MISREAD IN ONE EVENING, BY TWO SESSIONS, IN OPPOSITE DIRECTIONS

Both readings were published before either was checked against the code:

1. **A socket-error count of zero** was taken ~70 s after a daemon swap and read
   as *"the outage is over"*. It was a **transient before clients reconnected**;
   the errors returned minutes later.
2. **`launch_phase == RemoteBootstrap`** on 39 of 49 rows was read as *"39 rows
   are stranded"*. ⛔ **It does not mean stranded.** It is the resting state of a
   row **not yet attached in this GUI generation** — `daemon.rs` asserts a fresh
   shell row carries it, and the open path sets it on rows nobody has clicked.
   With ~46 sidebar rows and ~10 open, 39 is the expected number.

⭐ **The decisive check was a self-counterexample, not an argument:** rows
*demonstrably executing work* — including the session writing this entry — read
`RemoteBootstrap`, and one still displayed the stale socket error in its
scrollback while fully functional. **The error text is buffer residue, not
state.** ⇒ **The only instrument that stayed correct throughout is whether a row
RESPONDS.** Prefer it to any status field, and read the code that sets a field
before quoting it in either direction.

### ⚠ THE OBSERVER JOINS THE SET IT IS MEASURING

`hot_restart_blockers` listed the diagnosing session itself, `kind: working`,
idle_ms 119. That is a property of the measurement: any session able to run the
probe is by definition active, and therefore extends the window whose failure to
close it is investigating. **A quiet-window drain cannot be observed from inside
the host it gates.** Sample off-host, or account for your own contribution.

### ⛔ A CAUSAL STORY THIS ENTRY CARRIED AND WITHDREW

It previously said the drain *"can never converge"* because it defers while any
session was active in the last 300 s, and that the agents making the host busy
were the ones resetting the window. **The measurements were sound**
(`hot_restart_blockers`, `kind: recently_active`, `threshold_ms: 300000`,
`attempts: 0`) — **the mechanism bolted onto them was not.** The idle gate was
innocent; the daemon converged by a deliberate version-coexistence route while
that story predicted it could not.

⇒ **The real cause is narrower and better: the swap queue's only drainer is a
daemon's retire poll, and the newest daemon was twelve versions older than the
code that reads the queue.** Producer (the current GUI) and consumer (a stale
daemon) sit on opposite sides of the very version skew the queue exists to fix.
`attempts: 0` for 17 minutes was **nobody reading**, not deferral. ⇒ **A queue
whose consumer can be older than its producer has no floor**, and no amount of
correctness in the gate can help it — the third time on this project a suspected
gate turned out to be a bystander.

⚠ **The general shape, since two sessions hit it within one hour:** *when two
readings both fit the data, test the one that licenses the action you already
wanted to take* — and a brief may carry MEASUREMENTS, never a causal theory
dressed as fact.

## ⭐⭐ [6.1] ONE DEATH IS NOW EXPLAINED END TO END — THE DEADLOCK CREATES A DUPLICATE, AND THE DUPLICATE-PRUNE KILLS THE PTY

**Status:** OPEN

*The 464 s death after the 3.0.154 deploy, traced to its exact exit. This is the
first session death this campaign can account for mechanically rather than by
coincidence in time.*

### The chain, every step evidenced

| time (UTC) | what the trace says |
|---|---|
| 01:26:57 | 3.0.154 deployed; handover starts |
| 01:28:17 | sweep: successor **accepted the fd but never acknowledged it (after the commit point)**. Predecessor books a failure and **resumes its own readers** |
| 01:29:17 → 01:34:17 | **seven consecutive sweeps**, one a minute, every one `NoneMoved` on the same key, with the successor logging `pty_handoff_refused` — *"already runs a live PTY for it"* |
| 01:33:02 | the seat's last assistant record, mid `tool_use` |
| **01:34:41** | `terminal_runtime_dropped` on the **predecessor**, `reason: duplicate_legacy_owned_runtime_prune`, **`removed_terminal: true`** — driven by the successor's `duplicate_legacy_owned_runtimes_pruned` (`reason: terminal_ensure_new`) naming that predecessor as owner |
| 01:34:41 | the transcript's last write. Same second. |

⇒ **The lost ack leaves the key genuinely owned by BOTH daemons.** The
duplicate-pruning path then does exactly what it is for — resolves a duplicate —
and resolves it by dropping the runtime **with the terminal**, which is the PTY
the agent was mid-turn on.

⭐ **So the killer is not the handover and not the idle gate. It is the
DEDUPLICATION of a duplicate that the handover's own failure created.** Every
component behaved as designed; the defect is that one of them manufactures the
input another one is entitled to destroy.

### ⭐ THE SECOND FIX, AND IT IS THE ONE THAT TOUCHES THE KILLER (`d9966a7c`)

The prune calls `drop_terminal_runtime` on the OWNER, and a drop is
`remove_session` → `shutdown` → **`kill`**. So the prune does not merely forget a
duplicate record — **it kills the child.** After a lost ack that child is the one
the successor has just adopted and is serving.

⚠ **The same code fired twice with opposite outcomes**, which is what narrows it:

| | duplicate created by | prune dropped | outcome |
|---|---|---|---|
| the cut seat | a **lost ack** — successor took the fd, never acknowledged | the side a **live process was on** | **fatal, mid `tool_use`** |
| an unrelated row | a **genuine re-launch** (resume started 2 s BEFORE the drop) | the **stale** side | **correct**, process survived 66 min |

⇒ **In the lost-ack case neither side is stale**, so *"drop the duplicate"* has no
right answer to give. The caller must not ask the question, rather than the prune
trying to choose better.

**Adoption is the discriminator, and it was already representable** —
`PtyChildHandle::is_adopted` existed as dead code. A runtime we forked owns its
child alone; an adopted one is the same process on the same pty as the
predecessor's copy. The prune now skips adopted keys and **traces the skip**, so
a standing duplicate is visible bookkeeping rather than a silent one.

### What this means for the fix already landed

`22f97c41` (idempotent re-adoption of the same child) **removes the
precondition**: the 01:29:17 retry would have succeeded, the predecessor would
have released cleanly, and there would never have been a duplicate to prune.
⚠ That is an argument, not a measurement — it is unproven until a handover with
a lost ack is observed converging.

### ⛔ HONEST SCOPE — IT DOES NOT EXPLAIN THE OTHER DEATHS

`removed_terminal: true` appears **exactly twice in the entire trace corpus**,
both `duplicate_legacy_owned_runtime_prune`, both from the same predecessor:
this key at 01:34:41, and `local://75445547…` at 01:28:18 (one second after the
first failed sweep). ✅ **Checked: not a casualty.** Its `claude --resume`
started **two seconds BEFORE** the drop, so the resume is what created that
duplicate and the prune removed the **stale** side; the process was still alive
66 minutes later, and its last assistant record predates the drop by 4.3 hours,
so nothing was interrupted. ⚠ Its transcript mtime sits just after the drop and
would have read as "died mid-stride" — the mtime-is-teardown trap.

The 00:22:13 death has **zero** such events. ⇒ **Either a second mechanism
exists, or that death predates trace coverage of this event.** Do not
generalise this to the 272 s and 402 s instances; they remain unexplained, and
the standing verdict on them is unchanged — coincidence in time with a matching
signature.

## ⛔⛔ [6.1] THE OTHER SEAT DEATHS: EXIT PATH STILL UNEXPLAINED

**Status:** OPEN

⛔ Root cause NOT established. Read the falsification below before adopting
any mechanism.

*Measured 2026-08-14 from the dev trace corpus. This entry replaces the
"deploys converge the idle window and release PTYs" theory, which is refuted.*

### What was actually observed

| | |
|---|---|
| `cc-runtime://1c7780ea-…` (a delegate seat) | last `tool_use` at **00:22:13Z**, no tool result — cut mid-command |
| its recorded owner | **pid 1099216, version 3.0.151**, which named it in `handoff_runtime_keys` on every handoff |
| pid 1099216's last trace event | `daemon_self_retire` at **00:22:11Z**, then total silence |

Two seconds, and the owner is the daemon that names that session as one of the
41 live runtimes it was preserving. The ownership link and the timing link are
both solid. **What is not established is how 1099216 exited.**

### ⛔ THE INHERITED MECHANISM IS REFUTED — do not re-adopt it

The carried theory was: *a release starts a handover, the predecessor defers
while sessions look active (300 s idle window), converges, exits, and releases
the PTYs it still owns.* Against the record:

- pid 1099216 emitted **160 `daemon_self_retire` events and ZERO
  `daemon_cold_shutdown_deferred_idle_gate` events.** The idle gate **never
  deferred it once**. There was no 300 s convergence to wait for, because the
  gate was never engaged — a daemon only reaches the gate on a poll where the
  swap lane returns `Failed`, and that happened at most once in 53 minutes.
- `hot_restart_forced_past_deadline` fired **0 times** in the whole corpus
  (positive control: 175 `daemon_self_retire` hits by the same grep), and
  `hot-restart-interrupted.json` does not exist. The forced-past-deadline path
  did not run.
- At 00:22:11 the two `Lingering` short-circuits both applied — the lane's
  retry interval (last attempt 00:19:11, 180 s < the 300 s
  `HOT_RESTART_SWAP_RETRY_INTERVAL_MS`) returns `Lingering` before the
  cold-shutdown gate is reachable at all. **So the cold path was not reached,
  and the tidy diagnosis dies on its own timing.**

### ⚠ What has NOT been tested, and why the obvious instrument is blind

`dmesg` is **not readable in this container** (`read kernel buffer failed:
Operation not permitted`) — an earlier "no OOM kills" reading here was a blind
instrument, not a negative result. **An OOM or an external kill of pid 1099216
remains untested and is currently the leading rival.** The falsifier: capture
the exit of a preserved-owner daemon with an instrument that survives it
(process accounting, a `PR_SET_PDEATHSIG`-free supervisor, or a cgroup event
reader), not by inferring from the trace going quiet.

⇒ **Silence in the trace is not an exit path.** The cold shutdown emits no
trace event of its own, so "no shutdown event" cannot distinguish a cold
shutdown from a SIGKILL. Give the shutdown path its own event before the next
attempt to attribute a death.

### ⛔⛔ A THIRD INSTANCE, 2026-08-14 — AND MY "NO ROWS DIED" WAS WRONG

The 3.0.154 deploy (01:26:57Z) was reported by me as causing no deaths. **It
did.** A delegate seat's last assistant record is `01:33:02Z` **ending on
`tool_use`**, its transcript stops at `01:34:41Z`, and its process is gone —
with a positive control in the same sweep (the identical scan for a live uuid
returned three processes, so the absence is real and not a collapsed probe).

    release 01:26:57Z → last write 01:34:41Z → gap 464 s

⛔ **The instrument, not the observation, was the mistake.** I watched "past" the
danger window and said so — but my window was fitted to the two known deaths at
**272 s and 402 s**, and this one landed at **464 s**. The recorded gaps are now
272 / 402 / 464, all ending on `tool_use`, and **they are not converging
downward**, so a window fitted to the previous maximum will always be one
instance behind.

⇒ **"No rows died" cannot be established by watching for a fixed interval.** The
honest form is a **census by uuid before and after, re-checked at the next relay
boundary** — a row that dies at 600 s is invisible to any watch that ends at 500.
⚠ Same evidential standing as the first two and no stronger: coincidence in time
with a matching signature. Cause is still **not established**, an independent
crash is still not excluded, and `dmesg` is still unreadable here.

### ⛔⛔ THE EVIDENCE FOR THE EARLIER DEATHS WAS NEVER MISSING — IT HAD BEEN DELETED

**Measured 2026-08-14 (6.1).** Every conclusion above about "no such events in the
corpus" for the 166/242/272/402 s deaths rests on a corpus that **does not reach
them**:

| trace corpus on dev | 8 generations + live, 247.9 MiB against a 256 MiB cap |
|---|---|
| earliest record | **01:05:11Z** |
| the four deaths | 16:55Z, 19:50Z, 00:21Z, 00:22Z — **all before it** |

The write rate is **83.1 MiB/h (1.95 GiB/day)**, against the retention comment's
claimed *"~80 MiB/day"* — wrong by a factor of 25 — so the retained window was
**2.98 h, not the 3 days** the constant advertised, and the 3-day age rule had
never once been the binding condition. ⇒ **The absence of a mechanism's events in
those deaths is STRUCTURAL, not evidential.** It supports nothing, in either
direction, and any argument built on it must be withdrawn.

**Why it collapses:** the byte budget is per HOME while the write rate is per
DAEMON, and every daemon on the host writes the same file. The window is roughly
`cap / (per-daemon rate × N daemons)` — so the diagnostic window shrinks exactly
as the daemon pile grows, and it is shortest during the releases it is needed for.
⭐ That is a further argument for the drain that S1 did not have.

**Landed:** the cap is now 1 GiB (~12 h at the measured rate) with the mechanism
written into the constant, and `scripts/pin-trace-window.sh` hard-links the trace
window around a deploy — `deploy-fleet.sh` calls it automatically, following for
15 min, so a death in the danger window survives retention. Links, not copies:
the pruner's unlink then frees nothing. ⚠ Both were falsified before being
believed — the follow loop's first version linked NOTHING (an unexported `DEST`
in the detached shell) while reporting a clean first pass.

### ⛔⛔ A CODE-GROUNDED CANDIDATE: THE STALE-DAEMON SWEEP KILLS, AND BOTH ITS GATES DISCARD ERRORS

**Not proven — no observed instance — but it is the first candidate with a traced
trigger path rather than a timing coincidence, and its fail-open half is real.**

`cleanup_legacy_linux_daemon_processes` runs **on every daemon start** and
`terminate_linux_process`es any same-home daemon it finds "legacy" — SIGTERM,
120 ms, then SIGKILL. "Legacy" is **relative to the sweeper**: a daemon is legacy
when its `argv[0]`/exe is not the sweeper's own binary path, so a daemon started
from a build tree classifies every installed daemon as legacy. The single gate
between that and a killed PTY is `has_clients`, and **both of its terms threw
their errors away**:

- `active_client_instance_records_for_endpoint_scope(..).ok()` ⇒ an unreadable
  client list arrived as *"no clients"*. This is the same seam fixed one layer
  up on 2026-08-14; that fix made the error travel, and this caller — the one
  that kills rather than merely permits a retirement — dropped it on the floor.
- `reachable_versioned_daemon_statuses` drops a daemon whose `status` call errors
  (`.ok()?`), and **every protective branch** of
  `linux_daemon_runtime_activity_protected_for_cleanup` requires a status to be
  present. A daemon serves one request at a time, so the daemons most likely to
  miss a status probe are the ones mid-handover — which is what a release makes
  every daemon do at once.

**The measurement that makes this urgent, from 8 sweeps in the live corpus:**
`skipped_active_client_legacy` was **0 in every one**, while
`skipped_runtime_activity_legacy` was **16**. ⇒ The client gate contributes
nothing on this fleet; sixteen live daemons were each **one unanswered status
probe** away from SIGTERM, and every PTY they own with them.

**Landed:** both terms now fail closed — an unreadable client list and an
unanswered status both count as *something to protect* — and the sweep reports
`skipped_unreadable_clients` and `skipped_no_status` separately, so a sweep that
protected everything because it could see nothing can never again be read as a
sweep that found nothing to do.

⇒ **What would settle it:** at the next release, with the trace window pinned,
read the sweeps in the 0–600 s band. `killed_legacy` or `killed_orphan` ≥ 1 at a
death's timestamp is the proof; `skipped_no_status` ≥ 1 says the fix is what
stood between us and it. ⛔ Until then this is a candidate, not the cause, and it
does not retire the OOM/external-kill rival above.

## ⚠ [6.1] TWO SIBLING FUNCTIONS DISAGREE ABOUT AN UNREACHABLE OWNER, AND ONE OF THEM PRUNES

**Status:** OPEN

*Latent, not observed firing. Found while fixing the same class in the
stale-daemon sweep, 2026-08-14.*

The preserved-owner registry is what tells the sweep *this daemon owns live work,
do not signal it*. Two functions maintain it, twenty lines apart, and they treat
"the owner did not answer" oppositely:

| function | on a status `Err` |
|---|---|
| `prune_unrepresented_preserved_owner_runtime_sessions` | traces `preserved_owner_runtime_prune_status_failed` and **skips** — correct |
| `retain_represented_keys`'s predicate | the cached status is `None`, `is_some_and(..)` is **false**, so the key is **PRUNED** |

⇒ **A busy owner that misses one probe can have its preserved-runtime records
removed**, and the miss is then cached (`mark_preserved_owner_unreachable`,
"pay the timeout once per owner per window"), so the effect persists rather than
self-correcting on the next call. Those records feed `preserved_owner_pids` /
`owner_registry_guard_active` — the guard the reap path leans on — so the same
"no answer means nothing is there" error sits inside the bookkeeping of the gate
that protects live daemons from SIGTERM.

⚠ **Not observed:** `preserved_owner_registry_pruned` appears **zero times** in
the live corpus (only `preserved_owner_deep_reconcile_deferred_on_load`, ×9), so
this is a latent disagreement rather than a running defect. ⛔ And that corpus is
~3 h wide, so its silence is weak evidence either way — see the retention entry
above.

**Recommended fix, not applied here because it needs one fact this entry cannot
cheaply establish:** an unreachable owner should KEEP its entry (matching its
sibling), and pruning should be driven by **the owner's process being gone**, not
by its socket being quiet — the same absent-versus-unreadable split that the
retire gate and the reap gate now use. Applying only the first half would let a
dead owner's keys accumulate for ever, which is why it wants the pid-liveness
check (`linux_preserved_owner_process_ids_for_home` already resolves those pids)
rather than a one-line flip.

## ⛔⛔ [6.1] THE GUARD PROTECTING A PRESERVED PTY OWNER IS A HOST-SHARED FILE, SO A PEER CAN ERASE IT

**Status:** FIXED IN CODE — LIVE PROOF OWED

Fixed by `544b11c0`. ⚠ It ships in a daemon built from this commit; **the 15 daemons already running on dev keep the old
behaviour and cannot be retrofitted**, so the live proof must come from a
daemon born after the deploy, not from the current population.

**Falsified by:** a `hot_restart_swap_queue_satisfied` event whose
`target_source` is `queue_file` on a daemon whose entry a peer had already
cleared (it should read `process_memory`), or any `SwapStep::Failed` taken by a
process that has handed off. Both are now traced.

*The same root cause as the non-terminating retire loop below — one host-shared
single-slot file used as per-process state. Filed as two entries because the two
consequences are independently serious: this one kills PTYs, that one multiplies
daemons.*

The safety property is stated outright at `crates/yggterm-server/src/daemon.rs`
(the `SwapStep` doc comment):

> *Only `SwapStep::Failed` falls through to the cold-shutdown gate … a daemon
> that has ALREADY handed off is a preserved PTY owner, and letting a later
> failed retry drop it into the cold path would kill the very PTYs the first
> handoff preserved.*

The thing that implements it is one arm of one match:

```rust
None if queued.is_some() => SwapStep::Lingering,   // the whole protection
None                     => SwapStep::Failed,      // → cold shutdown → PTYs die
```

`queued` is `hot_restart_queue::load(home_dir)` — **a file in the host's
`~/.yggterm`, shared by every daemon on the machine.** The adjacent `Converged`
arm calls `hot_restart_queue::clear(home_dir)` on that same shared file. The
process-local backstop (`HOT_RESTART_SWAP_LANE_SETTLED`) is set **only in the
process that converged**.

⇒ **One daemon's convergence deletes every peer's proof that it already handed
off.** Measured on dev tonight: pid 2410824 fired
`hot_restart_swap_queue_satisfied` **12 times** in ~90 minutes, on a host
carrying 15 daemons — twelve chances to disarm fourteen peers.

**The fix:** "have I already handed off?" is a fact about **this process**, and
must be answered from this process's own state (it is a preserved PTY owner /
it holds live PTY children) — never from a host-shared file that a peer writes.
⛔ And the deeper form: **a daemon that still parents live PTY children must
never take the cold-shutdown exit at all**, whatever any flag says. Ownership of
a child process is the only fact that matters there, and it is directly
observable.

## ⛔⛔⛔ [6.1] ONE POISONED SESSION BLOCKS A WHOLE DAEMON'S DRAIN, PERMANENTLY — THIS IS WHY DAEMONS NEVER EMPTY THEIR HANDS

**Status:** OPEN

⭐ **Caught live during the 3.0.154 deploy, reproducing once a minute**, which is
what makes it the drain lane's central defect rather than a curiosity. The
one-off "accepted but never acknowledged" filed below is the FIRST HALF of it;
this entry is what that failure turns into on the next attempt.

### The two-step deadlock, from the trace

```
01:28:17  3.0.153 → 3.0.154  stood_down=11 resumed=11
  NoneMoved { "cc-runtime://<key>: successor accepted the fd but never
               acknowledged it (AFTER the commit point — the fd is gone)" }
01:29:17  3.0.153 → 3.0.154  stood_down=10 resumed=10
  NoneMoved { "cc-runtime://<key>: successor took the fd and refused to seat it:
               refusing to adopt <key>: this daemon already runs a live PTY for it" }
```

1. The successor **takes the fd and seats it**, but the ack does not get back.
   The predecessor books it as a failure and keeps its own runtime.
2. On every retry the successor now **refuses**, because it already runs a live
   PTY for that key — *which is proof that step 1 actually succeeded.*

⇒ **The refusal reason is the evidence that the move worked**, and the code
reads it as the reason the move failed. The state is stable, so it never clears.

### ⛔ AND ONE FAILURE ABORTS THE WHOLE SWEEP

`readers_stood_down: 11` with `moved: 0`: eleven runtimes were parked, the first
one failed, and **all eleven were resumed**. `classify_handoff_sweep` takes a
`first_failure` and the sweep stops there. So a single permanently-poisoned
session pins **every other session on that daemon**, and the daemon can never
reach the empty hands that would let it retire.

⇒ **This is the mechanism behind the standing hazard.** The 14 pre-settle-window
daemons are not lingering because their sessions are busy; a daemon needs only
ONE key in this state to be pinned for ever. It also explains the zero
`progressive_migration_session_released` events in the whole corpus.

### The fix — and why the obvious one is NOT safe as written

The tempting change is to treat *"this daemon already runs a live PTY for it"*
as a **success**. It usually is. ⛔ But the message cannot distinguish:

- the successor is holding **the fd we sent it** (dropping ours is correct and
  harmless — same PTY, one redundant descriptor), from
- the successor has **its own, different PTY** for that key (it re-resumed the
  session independently). Dropping ours then closes a PTY whose child is still
  on it.

⇒ **The predecessor must confirm IDENTITY, not just presence** before counting
it moved — compare the seated runtime's child pid or pty device against the one
it handed over (`terminal_process_id` is already in the status payload). With
identity confirmed, "already seated" is the success it looks like.

**Independently worth doing:** the sweep should attempt **every** session and
classify at the end, rather than abandoning ten good moves because the first one
failed. ⚠ That makes `Partial` more common, and `Partial` is the outcome that
must never exit the process — which the code already knows.

⭐ **Deeper fix, upstream of both:** the ack is the only thing that makes the
commit point meaningful, and it is being lost after the fd has already
transferred. Either the seat-and-ack must be atomic from the predecessor's point
of view, or the predecessor needs a **re-query** path — *"do you have this key,
and is it the one I sent?"* — so a lost ack costs a round trip instead of
permanently poisoning the session.

## ⛔ [6.1] A HANDED-OFF fd CAN BE ACCEPTED AND NEVER ACKNOWLEDGED, PAST THE POINT OF NO RETURN

**Status:** OPEN

Observed exactly once, with the sweep naming its own victim.

The only `superseded_self_retire_sweep` in the dev corpus:

```
Partial { moved: 6, reason: "local://…: successor accepted the fd but never
          acknowledged it (AFTER the commit point — the fd is gone)" }
readers_stood_down: 7   readers_resumed: 1   all_moved: false
```

⚠ **The settle window still reported `settled: true` with
`bytes_stolen_after_park: 0` ten seconds later** — both true, and both
irrelevant to the session whose fd went missing. A green settle window is not a
statement that every runtime moved; `all_moved` is the field that says that, and
it was `false`.

⇒ Two things are owed: a **commit point that is not reached until the successor
acknowledges** (or a reclaim path for the window between accept and ack), and a
settle-window verdict that **cannot read as success while `all_moved` is false.**

## ⛔ [6.1] THE SELF-RETIRE LOOP NEVER TERMINATES — A FRESH DAEMON EVERY ~5 MINUTES, INDEFINITELY

**Status:** FIXED IN CODE — LIVE PROOF OWED

Fixed by `544b11c0`.

### The mechanism, settled

The predecessor asks a **host-shared file** what **this process** did. The
successor deletes that file the moment it recognises itself as satisfying the
entry (`satisfied_by: "self"`, a second emitter of the same event name), which
is routinely *before* the predecessor's next 20 s poll. The predecessor then
reads "you never handed off", hands off again, and spawns another successor —
which clears the file again.

⭐ `HOT_RESTART_SWAP_LANE_SETTLED`'s own doc comment predicts this in words:
*"reading its absence as 'nothing has happened yet' is how clearing the entry
would make this daemon start asking for a brand-new swap on the very next poll,
forever."* **The flag was correct and unreachable** — the only code that SET it
ran inside `if let Some(queued)`, so a process whose entry had already been
cleared could never get there. The guard against forgetting was reachable only
through the thing that gets forgotten.

**The fix:** remember the handoff target in the process that made it (keeping the
highest ever asked for), evaluate convergence from either half, and answer the
cold-shutdown guard from the same memory.

⚠ **It ships in the daemon that does the retiring, so it stops FUTURE
generations only.** The 14 legacy daemons already looping on dev run their own
old code and will keep generating successors until they are drained. ⇒ **the
drain is still required, and this fix is what stops the drained set from
refilling.**

**Falsified by:** the daemon population on a host growing across an hour with no
deploy in it, measured as the row count of `server daemons`.

⚠ **RUN 2026-08-14, AND IT DOES NOT DEMONSTRATE THE FIX.** 16 daemons, flat
across 40 minutes (01:41→02:16Z), no deploy in the window. That satisfies the
falsifier as written and is **consistent with** the fix — but it is weak
evidence, because the **pre-fix daemon in the same window is not generating
either.** The generator is a race (the successor clears the shared file before
the predecessor's next poll) and it simply was not provoked. ⇒ **A control that
also passes supports nothing**, so the falsifier above needs strengthening: the
demonstrative test is a **3.0.154 daemon acting as a PREDECESSOR**, which cannot
happen until the next release supersedes it, with
`target_source: "process_memory"` as the positive signal.

⛔ **And do not count the row lines.** `server daemons` interleaves indented
`deferring:` continuation lines with daemon rows (25 raw lines for 16 daemons),
and their number varies with session activity — so a naive line count shows
phantom population growth purely from sessions becoming busy, which looks exactly
like the generator being tested for. ⚠ An `awk` filtering on the version column
is also wrong: the serving daemon's row is prefixed `*`, which shifts every field
by one, so such a filter silently drops **the daemon actually serving**. ⛔ Do **not** try to
measure this by counting distinct pids in the trace — `main_enter` is emitted by
one-shot CLI invocations too, and that instrument reported 614 "daemons" in two
hours on a host that had 15.

`retire_trigger: "disk_binary_replaced"` is derived from `exe_link` reading
`… (deleted)`, which stays true forever once the binary on disk is replaced. So
the retire poll re-fires every 20 s for the life of the process and the handoff
re-fires every `HOT_RESTART_SWAP_RETRY_INTERVAL_MS` (300 s), **each time
spawning a brand-new successor daemon**.

Measured on one daemon (pid 1099216, 3.0.151) over ~53 minutes:

| `daemon_self_retire` | 160 (every 20 s) |
|---|---|
| `daemon_self_retire_handoff_ok` | 11 |
| successor daemons spawned | 11 |
| `progressive_migration_drain_already_running` | 11 (the drain refuses each retry) |
| `daemon_cold_shutdown_deferred_idle_gate` | **0** |

Each successor is born, reads the row ledger, reconciles, and is itself
superseded ~5 minutes later. ⭐ **The retry interval's own doc comment names the
fork-bomb risk and prices it at "one successor every five minutes" as the safe
rate — but that rate is only safe if the loop TERMINATES**, and nothing here
terminates it: convergence clears the queue file, the trigger re-derives `true`
from the same deleted `exe_link`, and the next interval spawns another.

⇒ This is a standing contributor to the daemon population the drain lane exists
to reduce, and it runs on every host with a replaced binary, not just after a
deploy. **A retire that has handed off must stop retiring**, and the terminating
condition has to be durable across the queue file being cleared by a peer.

## ⛔⛔ [6.1] SOME ROWS WILL NOT RESTORE, AND A RESTART DOES NOT RECOVER THEM

**Status:** FIXED IN CODE — LIVE PROOF OWED

*reported 2026-08-13, live on the desktop host · root cause found and fixed
2026-08-13 (3.0.114)*

**ROOT CAUSE, reproduced deterministically.** The resolver that turns a row's
path into the key its PTY is addressed by answers with a name nothing holds.
`YggtermServer::terminal_runtime_key_for_path` folds a runtime-lane key
(`cc-runtime://<id>`, `codex-runtime://<id>`) down to `local://<id>` whenever the
session map holds no row for it — right for a legacy adopted runtime, wrong for a
daemon-owned one, and it cannot tell them apart because the difference lives in
the terminal map it cannot see. Four rows on the desktop host had a live PTY the
CURRENT daemon owned as `cc-runtime://<id>` and no session row, so every read
resolved to `local://<id>` and answered
`Error: terminal session not found: local://<id>` — the whole of the retained
screen, 110 bytes, painted into the viewport. A restart re-runs the same fold,
which is exactly why restarting never cleared it. Reproduced on demand by
running the row's own launch line by hand: `yggterm server remote resume-cc <id>
<cwd> --require-existing` → the same error, every time, while
`server status --endpoint 3.0.112` listed `cc-runtime://<id>` among the daemon's
own `owned_terminal_session_keys`.

**How the orphan is made**, and it is the same defect one layer up: the close
path resolved the key the same way, so `terminals.remove_session()` removed
nothing and left the real PTY running with no row anywhere. Every close of a row
in this state manufactured the next one.

**Fixed:** the daemon now prefers whichever spelling its own terminal map answers
to (`DaemonRuntime::terminal_runtime_key_for_path`), and every in-daemon caller
— close, keep-alive, snapshot runtime-truth, identity refresh — goes through it
rather than the session map's raw fold. A request that still cannot be served
emits `daemon/terminal_runtime/request_refused` naming which map held what.

**LIVE PROOF, 2026-08-13.** `remote-cc://dev/23b20d7c…` — one of the two campaign
rows that could not be resumed to its relay point — was opened from the desktop
host and painted: reveal `outcome: ready`, `first_output_ms: 33859`, cold tier,
`failure_reason: null`, confirmed on a faithful screenshot. Its previous seven
attempts inside nine minutes all recorded `first_output_ms: null` while
reporting `ready`. All four rows are live in the row list again.

⚠ **What that proves and what it does not.** The rows recovered when the daemon
holding the orphaned PTYs died, which cleared the orphans — so the screenshot
proves the ROWS are usable, not that the resolver change is what made them so.
What the change does is stop the state recurring, and that is proven separately:
the truth table
(`a_rewritten_runtime_key_never_beats_one_the_terminal_map_actually_holds`), the
structural lock that keeps every in-daemon caller on the corrected resolver, and
the close path no longer being able to remove the wrong key.

**Residual, and it is the honest one:** no 3.0.114+ daemon has yet been observed
holding an orphan of this shape, because the fix removes the way they are made.
The observation that would falsify it: a row whose PTY a 3.0.114+ daemon owns
under `cc-runtime://<id>`, with no session row, that still refuses to open — and
`daemon/terminal_runtime/request_refused` now names exactly that case if it
happens.

After the GUI restarted, a subset of rows never came back to a usable state.
Restarting again does not clear it: the same rows fail the same way, so this is
not a transient of one handover. The reporter's marker was *"the top three
rows"* of the Live Sessions group at the time of the report.

**What is known from `server app rows` on the live host (44 rows):**

- The failing rows are present in the row list, at `depth: 1`, `busy: false`,
  `busy_reason: "idle"`, with a populated `detail_label` beginning `Kept alive ·`
  and carrying a full generated summary. ⇒ **the row's metadata survived**; what
  did not survive is whatever makes it openable.
- The group row above them reads `busy: true`,
  `busy_reason: "group_descendant_working"`, `child_count: 38`.

**Why this is the most expensive item in the batch.** Two campaign sessions —
this campaign's own row and a sibling's — could not be resumed to their last
relay point. A relay that cannot be resumed is a relay that has to be re-briefed
from artefacts, which is precisely the cost the baton relay exists to avoid.

**Where to look, in order:** the restore/reveal path that turns a persisted row
into an attached PTY; the `Kept alive` persistence class specifically, since
every observed failure carries it; and whether the failure is per-row state or
per-daemon (five of the daemons on that host are older than the current binary
— see the pre-3.0 daemon entry below).

**Falsifier:** a row that reports `Kept alive` in `detail_label` and refuses to
open must emit a trace event naming the refusal. Silence here is itself the bug —
the reporter had no way to tell "this row is gone" from "this row is slow".

## ⛔ deploy-fleet CANNOT RECOGNISE ITSELF UNDER AN ALIAS IT CANNOT RESOLVE

**Status:** OPEN

*narrowed 2026-08-13: the ⛔-storm and the "three-quarters success" misread are
fixed; a host whose alias does not resolve from itself still needs one word*

**What was wrong.** The local-host test was `[ "$host" = "$(hostname -s)" ]`, and
on this fleet the ssh alias and the kernel hostname differ — so a deploy run on a
host tried to ssh to itself, failed all four copies with `Could not resolve
hostname`, and printed ⛔ for the very machine doing the deploying while the
other two landed. The operator reads three-quarters success and moves on.

**What shipped.** Identity is no longer a string comparison. The script drops a
unique token in `$HOME` — the filesystem the four copies land in — and asks each
candidate channel whether it can see it. An unreachable host is now reported
**once, by name**, with the remedy, and its copies are marked SKIPPED rather than
failed; the census prints `(unreachable — nothing was written here, and nothing
is claimed about it)` instead of a row that looks like a reading.

⛔⛔ **AND THE OBVIOUS IDENTITY IS THE WRONG ONE — it nearly shipped.**
`/etc/machine-id` answers *"is this the same machine image?"*, and the question a
deploy depends on is *"do these two paths name the same FILE?"*. Two hosts on
this fleet report a **byte-identical** `/etc/machine-id` and have **different
filesystems**. A machine-id test called the second host "self", which would have
written its four copies into the first host's disk, read them back through the
same wrong door, and printed four ✅ for a host that was never touched — a total
deploy failure wearing a green census, strictly worse than the ⛔ storm it was
replacing. It was caught by falsifying a positive result rather than accepting
it: the probe was made to say SELF and NOT-SELF in one run, and the pair it had
to tell apart was exactly the pair that fooled the earlier design.

**What remains.** When a host's alias cannot be resolved *from that host*, no
local signal can bridge the two names — the alias belongs to the fleet, the
kernel name belongs to the machine, and neither knows about the other. The run
now says so and names `export YGG_FLEET_SELF=<alias>` as the permanent fix, but
until that is set on such a host, its own four copies are still skipped.

⚠ **Rejected as unsound:** inferring self by elimination ("every other host in
the list probed REMOTE, so the unreachable one must be me"). It is wrong the
moment the deploy runs from a machine that is not in the list, and its failure
mode is the catastrophic one above — writing this machine's disk under another
host's name and reporting success.

**Falsifier:** a deploy run on a host whose alias does not resolve there writes
that host's four copies, or refuses in a way no reader can mistake for success.

## ⛔⛔ [6.3] yedit's VIEWPORT PAINTS NOTHING WHILE ITS FILE RAIL IS FULL

**Status:** OPEN

*reported 2026-08-13 with a screenshot*

The document area is solid black. The right rail beside it is fully populated —
the Markdown/Split/Text toggle, the regex search box, and a FILES list of ~28
named documents, one of them selected and highlighted. The status bar under the
empty viewport reads a real measurement: **8570 words · 457 lines · 61018
chars**.

⇒ **The document is loaded and measured; only the paint is missing.** This is
the metadata-vouches-for-clipped-content shape: every instrument around the
viewport reports success, and the viewport is empty.

**Cost:** this is why the report that produced this batch was composed in an
external editor rather than in yedit.

**Falsifier:** a viewport reporting a non-zero line and character count must
paint at least one glyph. If it cannot, it must say so rather than render black.

⭐⭐ **A CHEAP, DETERMINISTIC REPRO — 2026-08-13, deploy-identity cluster.** The
original sighting needed a 61,018-character document, which makes every check
expensive and every difference arguable. The same state reproduces with a
**152-character** file authored for the purpose, in four commands, provided the
shell is ON THE GUI HOST (see the loopback entry below — a surface driven from
another machine reads a daemon that does not exist there, which is what made
this look unreproducible):

```sh
# on the GUI host
printf '# probe\n\nOne paragraph.\n\n- a\n- b\n' > /tmp/<probe>.md
yggterm server app terminal new --kind shell --cwd /tmp --purpose '<why>' --pid $GUI
yggterm server app terminal send <row> --data 'yedit /tmp/<probe>.md' --pid $GUI
# then a LONE carriage return — the text alone sits at the prompt unsubmitted
yggterm server terminal write <row> --data "$(printf '\r')"
```

Measured result: the FILES rail fills, the probe appears **selected and
highlighted**, and the status bar reads **30 words · 10 lines · 152 chars** —
the file's true measurement. So the loading half is sound and the discrepancy is
downstream of it, on a document small enough to hold in the head.

⚠ **A clean document-VIEW frame is still owed** and was not taken here: another
lane held a modal over the viewport at capture time, and closing another
session's dialog mid-flight is not a thing to do for a screenshot. One
`server app open <row> --view preview --pid $GUI` plus one screenshot completes
it from the state above.

⭐ **THE APP PLANE ITSELF PAINTS ON THE REAL GUI — measured 2026-08-13 by the
deploy-identity cluster at 3.0.133, on the desktop host, faithful frame.** A
freshly launched yedit surface rendered its whole rail: the toolbar icons, the
regex search field, the Markdown/Split/Text toggle, the FILES heading with its
`+` control, the empty-state line, and the Wrap control. ⇒ **Whatever is wrong
here is not "a document surface cannot render on a real GUI"**, which is worth
knowing before anyone spends a session on the widget layer.

⚠ **AND THE LOADED STATE COULD NOT BE REACHED, for a reason that turned out to
be neither the CLI nor the paint** (own entry below): the shell driving that
surface was on a different machine from the GUI, and a surface declares a
**loopback** control URL — so the GUI resolved it on its own host, found nothing
listening, and rendered *No files open* with every widget healthy. The file had
been stored correctly the whole time, in the other host's daemon.

⇒ So this sighting says nothing about the black viewport: an EMPTY rail pointed
at the wrong daemon is a different state from the full rail above. **Recorded
so the next reader does not mistake it for one**, and because the first version
of that entry blamed the CLI for dropping the path, which it does not do.

## ⛔⛔ A DOCUMENT SURFACE DECLARES A LOOPBACK URL, SO A CROSS-MACHINE SURFACE RENDERS EMPTY AND HEALTHY

**Status:** OPEN

*found 2026-08-13; supersedes this entry's own first diagnosis, which was wrong*

⛔ **CORRECTION FIRST, because the wrong version of this entry was published and
would send a reader into the wrong file.** It said `yedit <path>` accepts the
path, reports `document surface opened`, and drops it. **It does not drop it.**
Asked directly, the daemon takes the file and says so:

```
POST /open {"path":"/tmp/<probe>.md"}  →  {"ok":true,"id":"…","document_version":"8:false"}
/ping before → "document_version":"7:false"   /ping after → "8:false"
```

The client resolves the path against its own cwd and posts it; the route stores
it; the version advances. Every link in that chain works.

**What is actually wrong.** The control endpoint a surface declares is a
**loopback address with no host in it** — `http://127.0.0.1:<port>`. The declare
rides the PTY byte stream to the GUI, and the GUI then resolves that address on
**its own** machine. Measured across two hosts:

| | |
|---|---|
| daemon on the host running the shell | `http://127.0.0.1:46219` |
| daemon on the host running the GUI | `http://127.0.0.1:46335` |
| listeners on `46219` on the GUI host | **none** |

⇒ The GUI was pointed at a port nothing on it has ever bound. **And the surface
rendered as EMPTY rather than as unreachable:** full rail — toolbar, regex
search, Markdown/Split/Text toggle, FILES heading with its `+`, *No files open*,
Wrap. Every widget healthy, describing a daemon that does not exist on that
machine.

⚠ **This is why the campaign-wide document-body comparison has stayed open.**
Two lanes tried to compare a shadow-rendered body against a real-GUI one and
recorded it blocked, because nothing would load a document into a surface. The
loading was never the problem — **the GUI was reading a different daemon.** A
surface driven from a shell ON the GUI host does not take this path.

⛔ **It is a distinct defect from a libyggterm app launching on the wrong
machine, and fixing that one does not fix this.** Any manual `ssh <host>` into a
yggterm PTY — which the OSC declare explicitly supports via `LC_*` forwarding —
lands the same way. A declare that crosses a machine boundary must carry an
address the GUI can resolve, or the surface must refuse and say the endpoint is
unreachable.

**Wanted:** the declare carries a reachable endpoint, or the surface reports the
endpoint it could not reach. Either is fine; rendering an empty document with
healthy widgets is not.

**Falsifier:** a surface declared from a shell on another machine either shows
that machine's open documents, or names the endpoint it failed to reach.

## ⛔⛔ [6.3] EVERY SIDEBAR BUTTON OPENS THE NOTIFICATION SIDEBAR

**Status:** OPEN

*reported 2026-08-13, "in EVERY yggterm session no matter what sidebar I click"*

Whatever sidebar is requested, the notification sidebar is what opens. Reported
as appearing suddenly, on every session, after the fleet had been running for
two days without a restart.

**Prior art in this file:** the right panel is a **global slot** — one app's rail
renders over another app's row (tracked separately below). This is almost
certainly the same slot, now failing closed onto one occupant instead of
occasionally showing the wrong one. ⇒ the two should be diagnosed together, and
if they are one bug, the entries collapse into one.

**Falsifier:** click each sidebar affordance in turn and read back which rail
the GUI believes it opened. If the GUI's own model says "files" while the
notification rail is on screen, the bug is in the render slot; if the model
itself says "notifications", it is in the request.

## ⛔ [6.3] ychrome's VAULT AND SETTINGS RAILS SAY "Loading…" FOREVER

**Status:** OPEN

*reported 2026-08-13 with a screenshot*

The Vault rail renders its header and the word `Loading...` and never resolves.
The page beside it is loaded and interactive, so the webview itself is alive.

**Prior art:** the web eval bridge can die on every page (`bug-class-web-eval-
bridge-dead-all-pages`), and the session-metadata rail rendering its header and
nothing else is already tracked below. Same family: **a rail whose content comes
over a bridge shows its chrome and never its payload.** Check whether one dead
bridge explains all three before writing three fixes.

## ⛔ [6.3] A COLLAPSED SET REMOVED ITS MEMBERS FROM EVERY VERB THAT READS ROWS

**Status:** OPEN

*Owner-visible 2026-08-13 as nine live agent rows missing from the sidebar and from
`server app rows`. The listing half is FIXED at 3.0.140 and verified live; this entry stays open for
the render-side question below.*

**The mechanism, proven by controlled intervention rather than inferred.** `server app rows` answered
from the RENDERED row list, so folding a row set removed its members from the answer given to every
consumer — the booter, the supervision plane, seat audits, orchestration scripts. With all five books
folded, only the five heads were reported.

```
  rows, 5 books all folded  → 5 seats
  expand exactly ONE set
  rows, immediately after   → 14 seats, the nine members back
```

The persisted collapse set held exactly five session paths at that moment, one per book — a 1:1 match
with the five surviving seats. ⇒ It cost real state: the booter reaped nine subscriptions in six
seconds, each a working session, having been handed an honest answer to a question it was not asking.

**Fixed at 3.0.140:** the verb builds its own list with every row set OPEN and reports
`hidden_by_collapsed_set` per row. Verified live — 31 seated rows reported, 14 visible, 17 flagged
folded, where those 17 were previously absent entirely. The collapse itself is untouched and still
persists; three of those folds are the owner's own.

⚠ **A trap on the way, and the reason this entry names it:** the first attempt reached for the search
path's force-expansion to open the row sets and opened every folder and machine with them. 47 rows
became 1454, every session doubled through its dual presence, and `resolve_app_control_row` began
matching the cwd-tree copy of a row — which heads no set — so `row-expanded` started refusing.
⇒ **Widening a data verb means showing what a FOLD hides, not unfolding the tree.**

**What is still open.** Only the app-control verb was corrected. `snapshot.rows` is still the rendered
list, so any OTHER consumer reading it inherits the same defect. ⇒ Decide whether the snapshot itself
should carry every row with collapse as a field and let each renderer hide what it likes — which is
the general form of the rule — or whether the verb is the only consumer that needs it. **Do not answer
it by making collapse stop persisting: that is the user's state and he sets it deliberately.**

## ⛔⛔ [6.3] THE DOT READS THE SNAPSHOT, AND ONLY THE OTHER REQUEST ASKS THE OWNING DAEMON

**Status:** OPEN

*Owner-reported 2026-08-13: "only one or two are blinking. So are all the relay rows sitting and the
orchestrator chilling?" They were not — eight rows were mid-turn.*

**The mechanism, and it is a wiring gap rather than a missing capability:**

| request | working state | who reads it |
|---|---|---|
| `ServerRequest::WorkingFlags` | `working_flags_including_proxied()` — **asks the owning daemon**, has a passing test (`a_proxied_rows_working_flag_is_taken_from_its_owner`) | not the dot |
| `ServerRequest::Snapshot` | the raw screen scrape, no proxy | **the sidebar dot** |

⇒ **The proxied answer is built, tested, and the surface that needs it asks the other question.** Same
family as a verb implemented at every layer with no dispatch arm: complete, correct, and not reachable
from the one caller that wanted it.

**Why the scrape has nothing to say.** `working` is `screen_text.map(|s| descriptor.screen_shows_working(s))`,
so no screen ⇒ `None` ⇒ the GUI correctly declines to blink. Measured on the GUI host's own daemon
(⚠ NOT the one a CLI resolves — a first pass on the wrong daemon reported 254 of 260 unknown and meant
nothing), the two groups separate perfectly:

| | `launch_phase` | `terminal_process_id` | count |
|---|---|---|---|
| has a value | `Running` | a pid | 11 |
| **`working: null`** | **`RemoteBootstrap`** | **`None`** | **15** |

Every null row also carries `status_line: "… waiting for terminal host …"` and five `terminal_lines` of
launch preamble rather than a screen. **Six daemons were coexisting on that host**, and the ssh clients
holding the agent PTYs were spread across them (`TERM_PROGRAM_VERSION` 3.0.76 on one, 3.0.132 on
others) — so the dark rows are sessions owned by an OLDER daemon, which is the version-coexistence case
the constitution explicitly promises to keep first-class.

⇒ **The gap in the proxy itself:** `working_flags_including_proxied` skips any row where
`preserved_owner_endpoint_for_request` yields no owner, so a row whose owner this daemon does not know
is never asked about.

⚠ **The obvious cost question, which the fix has to answer:** snapshots are frequent and proxying is
network I/O to sibling daemons. Do not put an unbounded per-snapshot fan-out on that path — cache the
proxied flags with a short TTL, or have the owner push them.

### ⚠ PARTLY FIXED AT 3.0.134, AND MEASURED NOT TO BE ENOUGH

The snapshot now merges the proxied flags (it previously answered from the raw
scrape alone), and that is deployed and running. **It did not close the defect.**
Falsifier re-run on the live host at 3.0.135, 31 seated rows, transcript growth
over 30s against `working` in the same run:

```
  agree 26 · missed 3 · false positives 2
  rows still reporting working=None: 21 of 31
```

⇒ **The wiring was necessary and not sufficient.** `working_flags_including_proxied`
skips any row where `preserved_owner_endpoint_for_request` yields no owner, so a
row whose owner this daemon has no record of is still never asked about — and
that is most of them. Wiring the proxy into the snapshot cannot widen the
proxy's own coverage.

⇒ **The remaining step is discovery, not transport:** a daemon must be able to
find the owner of a row it holds no preserved-owner record for — asking its live
siblings rather than only endpoints it already knows. ⚠ Bound the fan-out; the
TTL cache is already in place for the asking half.

### ⭐ THE DISCOVERY IS SETTLED, AND THREE TRAPS ARE MEASURED — read before building

**Discovery costs no round trips and is already built.** `socket_sweep` proves
daemon liveness POSITIVELY from **one `/proc/net/unix` read**
(`SocketCensus::gather`), which is how it sweeps ~700 socket files without
issuing a request. ⛔ **Do NOT enumerate peers with `status`** — that is the gate
measured to hang 16 live daemons at once.

**Measured on the GUI host 2026-08-14: 5 live listening daemon sockets, one of
them self.** So the fan-out is FOUR peers, not a fleet.

1. ⛔ **THE OLD DAEMONS DO ANSWER, AND ONLY A REQUEST REACHES THEM.**
   `ServerRequest::WorkingFlags` has existed since **2.10.2** (2026-07-10), older
   than every live daemon on that host, so each answers for the rows it owns.
   ⇒ **A "have the owner PUSH its flags" design cannot work**, however much
   cheaper it looks: the daemons owning the dark rows shipped before any push
   would exist and will never write it. Asking is the only mechanism that
   reaches the population that matters.
2. ⛔⛔ **FANNING OUT RECURSES.** `ServerRequest::WorkingFlags` is served by
   `working_flags_including_proxied` itself (`daemon.rs`), so once two daemons
   both discover each other they ask each other forever. It is invisible today
   only because proxying landed 2026-08-13 and exactly ONE daemon on that host is
   new enough to do it — **the cycle arrives as the fleet upgrades.** ⇒ A daemon
   SERVING this request must answer from local plus already-cached flags and
   issue no new peer request, which also caps cross-daemon depth at 1. That costs
   nothing, because a discovering daemon reaches every owner directly.
3. ⚠ **A SLOW PEER MUST NOT BLOCK THE DOT.** `WorkingFlags` takes the default
   client IO timeout, so one hung peer stalls a refresh that runs every 1.5 s.
   ⇒ Discovery belongs on the chore thread, following
   `run_preserved_owner_revalidation_if_due` — the precedent in this file for
   exactly this, and its doc comment states the rule: *talks ONLY to other
   daemons' sockets and the trace file, never to this daemon's in-memory state.*
   The fast path then asks only endpoints already known to own a wanted row.

⇒ **Shape:** a chore-thread discovery pass records `session_path → endpoint` for
what each sibling answered; `working_flags_including_proxied` consults preserved
owners first and that memo second. ⛔ The memo is derived state of THIS
subsystem and must never be written into `preserved_terminal_owners` — that
registry drives hot-update handover, and a learned dot-owner is not an ownership
claim.

### ⚠ THAT SHAPE IS NOW BUILT, AND THE ENTRY STAYS OPEN UNTIL IT IS MEASURED

Landed: `run_working_flag_owner_discovery_if_due` on the existing chore tick,
the `discovered_working_flag_owners` memo consulted after the registry, and the
recursion cut — `WorkingFlags` is now served from local screens plus the cache
and issues no peer request, so fan-out happens on the refresh path alone. Unit
tests cover the selection (self excluded, non-daemon sockets ignored, stable
order) and both structural rules, and both structural tests were confirmed
FAILING against the pre-fix shape.

⛔ **None of that is the defect's own falsifier, which is why this is still
OPEN.** What is proven is that the mechanism exists and cannot recurse. What is
NOT proven is the number this entry is actually about: whether the 21-of-31 rows
reporting `working: None` becomes near-zero. That needs the transcript-growth
run above, against rows this session did not start, on a GUI old enough to have
the dark rows — and the sidebar is drawn by the GUI process, so it needs a GUI
carrying the build. ⚠ **Do not mark this fixed on the strength of the tests**;
they measure the wiring, and the wiring was already measured necessary and
insufficient once.

⚠ **And the ground truth got coarser as the fleet grew.** Two rows now read as
false positives against the growth test, which the earlier 21-row sample did not
show. A row mid-turn inside one long tool call appends nothing for 30s, so
"grew" under-reports work — **the growth test is a floor on activity, not a
census**, and a small disagreement in that direction is the instrument, not
necessarily the dot.

⛔ **TWO DIRECTIONS ALREADY REFUTED — do not spend a session re-deriving them:**

1. **"The owning daemon knows, propagate over the remote-session index."** Measured on both daemons for
   the same session ids: the other host reports `None` for essentially every row too, including rows
   where the GUI host has a real answer. The GUI host is the better-informed side.
2. **"Fall back to transcript freshness (`modified_epoch` / `last_used_epoch_by_session_id`)."** That map
   lags roughly twenty minutes: a row measured mid-turn showed a scan age of 1262s, and nothing in the
   sample was under ~1000s. It orders "recent work"; it cannot signal activity, and using it would
   produce a stale blink — worse than a missing one, because a blink that should not be there cannot be
   noticed.

**Falsifier — and it must run against rows you did NOT start.** A row that begins working while you watch
is the case that already works. Sample seated rows for transcript growth over ~25s (a file that grows is
a session mid-turn) against the dot state in the same run: every growing row blinks, every still row does
not. ⛔ Keep `None` meaning *nobody knows*: collapsing the tri-state to a bool trades a missing blink for
a permanent one.

## ⭐ [6.3] ROW SETS CANNOT BE ARRANGED BY HAND OR BY A VERB

**Status:** OPEN

*asked 2026-08-13; extended twice the same day; narrowed once the default
arrangement shipped*

⛔ **The whole rule is in `DESIGN.md` §"Row sets".** It is settled and it is not
re-opened here: the noun is **`row set`** (`section` collides with
`AppPaneWidget::Section`; `group` is splits and `folder` is the cwd tree); a set
means NOTHING but arrangement; a split is a view and a row set is an
arrangement, and **neither may relocate the other**; each set keeps its own
collapsed flag through an outer collapse.

The model, the inside-band drag and the persistence are built. **What is left is
the rest of the vocabulary the owner asked for**, and the entry closes when all of
it is live-proven together:

1. **Right-click → ungroup**, both halves in one menu: on a HEAD it dissolves the
   set and its members become top level; on a MEMBER it removes just that row.
2. **The verb twin.** `DESIGN.md`: *both halves exist or neither is real* — a
   delegate must be able to group and ungroup rows as a hand can. ⛔ Build it
   with its dispatch arm and exercise it end to end: this cluster has just been
   bitten by a verb that existed at every layer with **no caller at all**, and a
   gesture with no verb is the same defect mirrored.
3. **Live proof of all three gestures** on the GUI host, plus the un-numbered
   case — grouping rows that carry no seat is the whole point, since those are
   the rows an agent may never renumber.

**Falsifier for the finished feature:** arrange an outer set holding a collapsed
inner set and an expanded one, collapse the outer, expand it again, and find both
inner sets exactly as they were.

## ⛔⛔ [6.4] A libyggterm APP SPAWNED FOR A ROW RUNS ON THE WRONG MACHINE

**Status:** OPEN

*reported 2026-08-13*

An app launched from a row's right-click menu — or from anywhere else — must run
on **that row's host**. ychrome and yRDP do not: they run where the GUI is.

**Why this is a correctness claim and not a preference.** A libyggterm app is
sold as running on the machine whose data it is showing. An app that silently
lands on the GUI's host is showing the wrong machine's world while labelled with
the right machine's row. Partially tracked already for one case (*"open yRDP
here may open on guihost instead of the row's host"*) — this entry generalises it to
every app and every launch path, and that generalisation is the fix: **one host
resolution, used by every launcher**, rather than a per-app patch.

⚠ Related and already tracked: `server app launch-app --cwd` is ignored and the
row inherits the active session's cwd. Same root shape — the launcher does not
carry the launch context it was given. Fix them together.

## ⛔ [6.4] A CONTEXT MENU IS ATTACHED TO APPS THAT ARE NOT FILE OPENERS

**Status:** OPEN

*reported 2026-08-13*

yggdrasil-maker carries a context-menu entry. It is not a special-file-opening
app; it is a program in its own right, so appearing in a file's context menu is
meaningless. The same must not be inherited by yggtopo.

**The rule this settles:** a libyggterm app *may* integrate into yggterm's
context menus, and some earn it — ychrome is not a file opener either, but it is
useful there. So membership is a **choice**, not a property of being an app.

⇒ **Make it a config file, like a GTK/KDE desktop entry.** An app declares
whether it wants a context-menu slot and under what conditions, and the menu is
built from those declarations. Compiling the membership in is what makes the
current state un-fixable without a rebuild.

**Default for the two apps named here:** yggtopo and yggdrasil-maker declare **no**
context-menu integration. If either turns out to be useful there later, it is a
config edit and not a code change — that is the test that the fix worked.

## ⭐ [6.4] yRDP OPENS ON A SEARCH BAR OVER AN EMPTY LIST

**Status:** OPEN

*reported 2026-08-13*

yRDP shows no machines, just a search bar. Requested shape: a **start-page-like
surface** that inherits the same gradient, with a **large-icon view** (the shape
GNOME uses for its window switcher) where each tile shows **what that machine's
screen last looked like**.

⇒ The thumbnail is the point: it is what makes the surface lively rather than a
list of hostnames, and it answers "which machine was I on" the way the eye
answers it.

## ⛔⛔ [6.5] A DOCUMENT SURFACE'S BODY DOES NOT PAINT, AND ITS CONTRACT SAYS TWO DIFFERENT THINGS

**Status:** OPEN

*measured 2026-08-13 on a real GUI (an isolated sandbox, not the owner's)*

Reported symptom: an app's viewport pane renders its top bar and leaves the body
blank, while every telemetry field reads healthy — `has_schema: true`,
`stale: false`, `error: null`. Two lanes have now spent time on it. It has been
attributed to the read-only shadow client; **it is not the shadow.**

**Half of it is a contract that disagrees with itself, and that half is
actionable now.** yggterm's own deserialiser says:

> Chrome widgets (tabs, buttons, toggles, labels) form a top bar; `markdown` and
> multiline `text-input` widgets are the scrolling body.

`.agents/skills/libyggterm-surfaces/SKILL.md` says multiline `text-input` **and
`list-row`** "render at document scale". ⇒ **`list-row` is chrome in a viewport
pane.** An app that believes the prose declares a list of rows as its document
body, gets a blank page, and has no way to find out why — nothing failed. The
host is the SSOT; the prose is wrong and should be corrected.

**The other half is a real defect and is NOT explained by the above.** With the
rows replaced by a `markdown` widget the body is *still* blank:

| what was checked | result |
|---|---|
| the schema actually served | `markdown` widget, `id` + `source`, **928 chars** of real content over HTTP |
| the same schema in the RAIL, same app, same minute | renders **completely** — cards, sections, status dots, rows, tabs, search box, footer |
| the field names | `section.text`, `tabs.active`, `markdown.source` — all matched against the host's enum |
| "the refetch is racing the re-stamp" | **falsified** — same blank with the app re-stamping every 120 s |

⇒ The same-app rail control is what separates this from an app bug: the schema
is good, and the viewport placement is not painting a widget it says it paints.

⛔ **NOT the lost-edit-batch class — checked 2026-08-14, and the check is now one
field.** The [6.3] entry root-causes a different "renders nothing while state
reports it correctly": a webview edit batch that throws is acked as applied, so
the host never re-sends it and that subtree freezes for the life of the process.
**Two of this entry's own measurements rule it out.** A frozen subtree is frozen
because ITS mutations were lost — it cannot render the identical schema
completely in the rail in the same minute; and the class requires a fault to have
already happened, whereas this reproduces on demand. ⇒ **Read
`webview_edit_faults` in `server app state` when you next reproduce it.** Zero
confirms this is a real render-path defect in the viewport placement, and closes
the question for good; non-zero would mean the opposite and this entry folds into
[6.3]. ⚠ Stated as reasoning plus an instrument, NOT as a run: 6.3 has the
fault-injection harness but no way to declare a document surface, which needs
this lane's app.

⚠ **Falsifier, not yet run:** drive the shipped pilot editor through the
identical path in the same sandbox and confirm ITS markdown body paints. That
attempt stalled on the editor's own daemon (`Loading…` in the rail, control
endpoint never answered) and was not retried. If its body is blank too, this is
the document-surface body path for every app, not one widget.

## ⚠ [6.5] THE APP SCAFFOLDING HAS THREE HAND-COPIES AND ONE OF THEM FAILED SILENTLY

**Status:** OPEN

*measured 2026-08-13*

The platform's migration order calls for the widget schema to be lifted into a
typed contract crate once a second consumer exists. There are now three, each
carrying its own hand-written copy of the same ~200 lines, and the third one
paid the predicted price on its first live render:

- `section` was given `title`; the field is `text`. **The whole pane refused**,
  and said so — the loud failure, and the survivable one.
- `tabs` was given `selected`; the field is `active`. **Silently defaulted.** The
  pane rendered with no tab highlighted and nothing anywhere said why.

⇒ **The silent-default half is the argument, not the loud half.** A refused pane
sends you to the contract; a defaulted field sends you nowhere. A typed schema
turns both into compile errors.

⚠ A field-name test now pins every name against the host's enum, in the third
consumer. **That is a second encoding of the contract and should be deleted the
day the typed one lands** — it is a splint, not a fix.

## ⛔⛔ [6.7] AT REST THE GUI BURNS 93% OF A CORE, AND TWO THIRDS OF IT IS THE KERNEL ANSWERING `clock_gettime`

**Status:** OPEN

*measured 2026-08-13 on the desktop host*

Reported symptom: for two days, with the account rate-limited and therefore
almost nothing running, the laptop fan ran continuously and yggterm kept
consuming power.

**Measured, same host, 2026-08-13 11:21:**

| what | RSS | age |
|---|---|---|
| `yggterm` (the GUI) | **1,042,728 KB** | 36.6 h |
| its `WebKitWebProcess` | **850,648 KB** | 36.6 h |
| 6 × `yggterm-headless server daemon` | 9–32 MB each | up to **7.9 days** |
| 13 × `WebKitNetworkProcess` | ~9 MB each | — |

System state: load 2.23 on an idle machine, **10 GB of 15 GB swap in use** with
14 GB of RAM. That swap pressure is the known audio-residual condition, which is
the likeliest explanation for the lagged, distorted notification audio reported
in the same batch — the analog signal is not lagging, the machine is.

⛔ **REFUTED 2026-08-13 — the audio has its own cause, and it is ours.** See the
entry below; the swap explanation is not needed and does not fit the evidence.

⚠ **The reported "a million yggterm, a zillion WebKitWebProcess" is an
instrument artefact, and naming it is part of the fix.** There is **one** GUI
process and **four** `WebKitWebProcess`. htop lists threads by default and the
GUI has **88** of them, so it fills the screen with rows that share one PID. The
real defect is not the count — it is that the one process is **1 GB**, and that
is worse news than a hundred small ones. Any optimisation work that starts from
the count will optimise the wrong thing.

**The mandate for this cluster is explicitly wider than one leak:** telemetry,
diagnostics, and pattern research into how yggterm spends CPU, memory and power
at rest. The standard named is Apple-grade — an idle app should cost nothing
measurable.

⭐ **Escalated the same day, and the escalation contains the diagnosis:** the app
is *"barely usable now, behaving more and more jank as it hogs more resources by
just doing nothing."* Two claims there, and the second is the one to design
around:

- **Monotonic.** It gets worse over uptime, not worse under load. That is a
  leak or an unbounded accumulation, not a hot loop — a hot loop costs the same
  in hour 36 as in hour 1.
- **At rest.** The cost is incurred while doing nothing, which rules out the
  work itself and points at whatever runs on a timer, subscribes without
  unsubscribing, or retains per-event state that is never released.

⇒ **Growth-over-time is the measurement that matters**, not a point sample.
A single RSS reading names a symptom; RSS plotted against uptime names a class.

⚠ **One correction from the measurement below, because it changes where to
look.** The dichotomy above — *"a leak or an unbounded accumulation, **not** a
hot loop, because a hot loop costs the same in hour 36 as in hour 1"* — is the
one thing the evidence does not support. It **is** a hot loop, and it **does**
get worse with uptime, because its iteration count is proportional to
accumulated state: the same loop measures 7.3% of a core on a fresh process and
58.8% after 36.9 h on the same machine. ⇒ Do not look for a leak *instead of* a
loop. Look for **what the loop walks**, and why that keeps growing.

### ROOT CAUSE, measured 2026-08-13 — it is CPU, and memory is the amplifier

⚠ **The heading this entry was filed under was the wrong instrument.** RSS named
a symptom; it did not name the cost. Three measurements reframe it.

**1. The idle cost is CPU, not memory.** Sampling `utime+stime` from
`/proc/<pid>/stat` over 45 s on a quiet machine — *not* `ps %CPU`, which is a
lifetime average and has misled this campaign before:

| process | CPU at rest |
|---|---|
| the GUI | **92.6% of one core** |
| its `WebKitWebProcess` | **33.0% of one core** |
| 6 × daemon | 0.9–7.5% each, **13.9% together** |

⇒ **~1.4 cores burned continuously with nothing running.** That is the fan, and
that is the power drain. An idle app should cost nothing measurable.

**2. Two thirds of the GUI's cost is in the kernel, and it is one syscall.**
The 45 s split is `utime` 33.5% / `stime` **58.8%** — 64% of the GUI's CPU is
kernel time. `strace -c` on the main thread names it: **222,293
`clock_gettime` calls in 6 s**, 95.8% of all syscall time, 178,211 of them
`CLOCK_MONOTONIC`.

**3. Each of those calls costs 45.8× what it should, because this host has no
usable TSC.**

| host | clocksource | `CLOCK_MONOTONIC` cost | calls to burn one core-second |
|---|---|---|---|
| build host | `tsc` (vDSO) | **26.7 ns** | 37,400,000 |
| desktop host | `hpet` | **1222.5 ns** | **818,000** |

`available_clocksource` on the desktop host is `hpet acpi_pm` — TSC is not
registered at all, so `CLOCK_MONOTONIC` **cannot be served from the vDSO** and
every query is a real syscall reading a 14.3 MHz MMIO counter. At 1222.5 ns,
58.8% of a core is **≈481,000 clock syscalls per second at idle**.

⇒ **The same code costs ~1.3% of a core on a TSC machine and 58.8% here.** The
clocksource is a boot-config matter and is logged in `docs/owner-attention.md`;
**the defect that is ours is making half a million clock syscalls a second at
all.** Apple-grade means not being 45× sensitive to the platform clock. Under
the same instrument (`strace -c`, same host) the 2026-07-21 session measured
~4,200 `clock_gettime`/s; today it is ~37,000/s — **≈9× more clock queries than
July**, which is a code-side regression on top of the machine-side amplifier.

**Where the spin comes from.** `eu-stack` on the pegged main thread, 70 samples:
the dominant stack is `tao EventLoop::run → gtk_main_iteration_do →
g_main_context_iteration → clock_gettime`, with `ControlFlow::Wait` set — so the
loop is *supposed* to block. It does not: the instrument-independent ratio is
**1,554 `clock_gettime` per `ppoll`**, i.e. the loop spins hundreds of times
between blocking polls. Every future the VirtualDom wakes turns into a
`UserWindowEvent::Poll` through `tao_waker` (`vendor/dioxus-desktop/src/waker.rs`),
and the GUI has **162 `spawn(` sites** plus three app-root `use_future` loops.

**And each render is enormous.** `root_render_count` moves **4.4 renders/sec at
complete rest** — 131 renders in 30 s with nobody touching the machine. Against
33.5% of a core in user time that is **~76 ms of CPU per render**. The profile
shows why: `ShellState::snapshot → SessionBrowserState::all_rows → flatten_rows`
(6 frames deep) `→ session_id_suffix`, which allocates a `Vec<char>` **and** a
`String` per session per call, plus `SessionTitleStore::open` and
`summary_timeline_for_session_id` **on the main thread during render**. The
matching syscall churn is visible in the same trace: 59 `mkdir` (59 EEXIST),
1,030 `newfstatat` (686 ENOENT), 490 `pread64` in 6 s.

**Why it renders at all when nothing changed.** The app-root timer loops call
`state.with_mut(...)` unconditionally — `INPUT_GATE_DEADLINE_TICK_MS = 1_000`
and `HOT_WARM_CHECK_INTERVAL_MS = 5_000` (`crates/yggterm-shell/src/shell.rs`).
`with_mut` on a Dioxus signal marks it dirty on drop **whether or not the
closure changed anything**, and the signal is the whole `ShellState` that the
root reads. A tick that decides nothing still costs a full-tree re-render.

⇒ **Two independent defects, either survivable alone:** renders that had nothing
to render, and a render that is O(whole tree) with per-row allocation.

**4. The memory number was also understated, because a third of it is swapped.**
RSS alone cannot see it:

| | RSS | swapped | real anon footprint |
|---|---|---|---|
| GUI | 1,011,896 KB | 257,912 KB | **1.27 GB** |
| `WebKitWebProcess` | 901,008 KB | 873,956 KB | **1.77 GB** |
| together | | | **3.04 GB** |

983 MB of the GUI's total is a **single glibc `[heap]` brk arena** — glibc does
not return brk memory to the OS unless the top is free, so freed memory stays
resident. And the GUI's thread census shows accumulation directly: **11
`WebsiteDataStore` + 13 `ReceiveQueue` threads** for what should be a handful of
web contexts. ⇒ **that is the monotonic half** — web contexts created and never
released — and it is what makes a constant hot loop *feel* worse over uptime, by
pushing its working set into swap.

### The before/after that settles what KIND of defect this is

Same host, same HPET clock, same session set. Before = 36.9 h of uptime;
after = the same build relaunched, measured at 3 min idle.

| | before (36.9 h) | after (3 min) |
|---|---|---|
| GUI CPU at rest | **92.6% of a core** | **17.2%** |
| ↳ of which kernel (`stime`) | **58.8%** | **7.3%** |
| ↳ of which user (`utime`) | 33.5% | 9.9% |
| `WebKitWebProcess` CPU | 33.0% | 18.4% |
| GUI RSS + swap | 1.27 GB | **345 MB** |
| web process RSS + swap | 1.77 GB | **292 MB** |
| **combined anon footprint** | **3.04 GB** | **637 MB** |
| system swap in use | 10,304 MB | 8,193 MB |
| GUI `WebsiteDataStore` threads | **11** | **0** |
| GUI `ReceiveQueue` threads | **13** | **0** |

⇒ **~2.4 GB and ~75% of a core reclaimed by a restart costing about ten
seconds.** That is relief, not a fix.

⚠ **Both columns are the SAME BUILD — checked, because they nearly were not.**
The desktop GUI now reports 3.0.113, which would make the table a cross-version
comparison and worth nothing. It is not: the measured process started 11:53 and
was gone by 12:05, while the binary was replaced at **12:04:33** by another
cluster's deploy and the current GUI started **12:05:37** from it. Both columns
are 3.0.112. ⇒ **On a machine where other clusters deploy continuously, "same
build" is an assumption with a short shelf life** — stamp the binary mtime and
the pid's start time alongside any A/B, or the table quietly becomes a
version diff.

⭐ **And it answers the question the batch was filed on.** Kernel time fell
**8×** on a machine whose clock did not change, so the ~481,000 clock syscalls
per second were *not* a constant cost — **the spin rate itself grows with
uptime.** The reporter's two claims are therefore both true and they are the
same defect: state accumulates (11 → 0 `WebsiteDataStore` threads is that
accumulation caught directly), each main-loop wake has more to walk, and the
loop that costs 7.3% of a core on a fresh process costs 58.8% after a day and a
half. **A hot loop whose iteration count is proportional to accumulated state
looks exactly like "worse the longer it runs".**

⚠ **17.2% + 18.4% of a core is still the floor on a FRESH process with nobody
touching it.** The restart hides the growth; it does not reach the floor, and
the floor is what the Apple-grade standard is actually about.

### ⛔⛔ THE INSTRUMENT ALREADY EXISTED, IT RECORDED ALL OF THIS, AND NOTHING READ IT

`crates/yggterm-core/src/render_probe.rs` samples every role every 60 s with
`core_fraction`, `mem_kb` and a GPU gauge, and it already encodes both traps
this investigation re-derived from scratch (`ps %CPU` is a lifetime average;
RSS undercounts against swap). **It was running the whole time.** Reduced from
`~/.yggterm/perf-telemetry*.jsonl`, hour-averaged over the GUI's 22 h life:

| hour | `core_fraction` | `mem_kb` |
|---|---|---|
| 0 | 0.123 | 151 MB |
| 4 | 0.128 | 164 MB |
| 5 | 0.176 | 199 MB |
| 7 | 0.202 | 556 MB |
| 9 | 0.243 | 922 MB |
| 10 | 0.255 | **962 MB** |
| 13 | 0.475 | 956 MB |
| 16 | 0.584 | 955 MB |
| 19 | 0.755 | 954 MB |
| 20 | **0.910** | 955 MB |
| *(restart)* | **0.181** | **285 MB** |

⇒ **The regression was fully instrumented, faithfully recorded, and invisible
for 22 hours because nothing reads this file and nothing alarms on it.** The
telemetry gap in this mandate is therefore NOT a missing sampler — building
another one would have been the wrong deliverable. It is that **no idle-cost
budget exists**: no threshold, no alarm, no surface. That is the thing to build.

⭐ **And the curve corrects the mechanism — including as first written in this
entry.** CPU and memory growth are **decoupled**:

- **Memory saturates at ~955 MB by hour 10 and is then FLAT for eleven hours.**
- **CPU keeps climbing long after it — 0.255 at h10 to 0.910 at h20, a 3.6×
  rise against flat memory**, and 7.4× over the process's life.

⇒ **Swap pressure is not what makes the loop slow.** An earlier reading in this
entry — that accumulated memory makes a constant loop feel worse by pushing its
working set into swap — does not survive the curve. Whatever the main loop walks
**keeps growing after the heap stops growing**, so it is a population of *cheap*
objects, not bytes: sources attached to the main context, timers, subscriptions,
tasks, listener registrations. **Look for something that grows in COUNT while
costing almost no memory.** The 11 `WebsiteDataStore` / 13 `ReceiveQueue` threads
are one confirmed instance of exactly that shape.

⚠ **Honest note on how that restart happened.** It was intended and measured,
but it was triggered by running the *GUI* binary with an unrecognised subcommand
(`yggterm update --help`) instead of `yggterm-headless`. The GUI binary does not
print help for an unknown verb — it **launches a client instance**, which then
took the running GUI down with it and left a `SIGABRT` coredump of its own. Use
`yggterm-headless` for every control verb; see `docs/agent-field-guide.md`.

### ⛔ THE RE-RENDER IS NOT WHAT GROWS — measured 2026-08-13, and it redirects the fix

The reading this entry was about to act on — *the app root re-renders several
times a second at rest, so stopping the unconditional re-render is the win* — is
**refuted by the app's own always-on probe.** `app_render_rate` (shell.rs, no
`render_trace_enabled()` gate, emits `renders_per_sec` once a minute) had already
recorded the regression's whole render history. Reduced from the aged GUI's
trace, the one that reached 0.910 core:

| hour | renders/sec |
|---|---|
| 1 | 3.0 |
| 3 | 2.1 |
| 5 | 1.9 |
| 7 | 2.0 |
| 9 | 1.9 |
| 11 | 2.1 |

⇒ **739 samples over 12.3 h: the render rate is FLAT at ~2/s while CPU climbs
3.6×.** A constant-rate loop cannot be the thing that grows, so **capping the
render rate cannot recover the regression** — it is worth a slice of the fresh
floor and nothing of the climb.

**The `utime`/`stime` split says the same thing and says where to go instead.**
Fresh process measured today, against the aged split already in this entry:

| | fresh (5 min) | aged (36.9 h) | growth |
|---|---|---|---|
| user | 13.9% | 33.5% | **2.4×** |
| kernel | 7.9% | 58.8% | **7.4×** |
| total | **21.8%** | 92.6% | 4.2× |

Rendering is user time; the clock-syscall storm is kernel time. **The kernel half
grows three times faster than the user half** ⇒ the growth lives in the wake/poll
path, not in Dioxus. This agrees with the 1,554 `clock_gettime` per `ppoll`
ratio already recorded above, and it is what "a population of cheap objects"
predicts: more live futures/sources per wake, each priced at 1222 ns on `hpet`.

⚠ **A note on the two figures, because they are not in conflict and reading them
as one number loses the diagnosis.** ~2/s is the rate **at rest**; the 20–33/s in
the storm autopsies is a different regime, and the autopsy arms only at ≥20/s by
construction, so it can never sample rest. Quote which regime a render figure
came from or it means nothing.

**Reproduced minimally, so the shape is not session-scale.** A GUI with **zero
sessions** in the sandbox (`scripts/underglass-sandbox.sh start --env
YGGTERM_TRACE_RENDER=1`, `YGGTERM_GUI_BIN` pointed at a GUI build — the installed
binary on a headless host is not one) renders **1.67/s at rest** and reports
**246 of 258 renders `unattributed`** with `forced_wakes` unchanged: the root
re-renders while no watched field changed and nothing of ours scheduled it. The
inter-render gaps name the driver exactly: over 8,167 steady gaps, **31.3% land at
1001–1002 ms** — `INPUT_GATE_DEADLINE_TICK_MS`, whose `state.with_mut(...)`
closure early-returns without mutating while `with_mut` marks the signal dirty
on drop regardless.

✅ **FIXED and measured (commit `b04d363c`): 1.65 → 0.65 renders/s at rest, a 61%
cut in idle re-renders**, with the 1001 ms gap population going **31.3% → 0%**.
⭐ That gap fingerprint is what *attributes* the win: the before/after binaries
were not the same build, so the rate delta alone would not have been admissible
— but the specific period vanishing from the histogram is attributable on the
after-binary alone. **Use the fingerprint, not the rate, when a same-build
control is unavailable.**
⚠ CPU on `dev` moved only 1.5% → 1.4%, and that number does **not** travel:
`dev` has `tsc`, where a render is cheap. On the `hpet` desktop host the same
renders cost 45.8× more per clock syscall, so the CPU share there is
**unmeasured** — do not extrapolate it. And this caps a FLAT rate, so it buys a
slice of the fresh floor and nothing of the climb.

⚠ **One inference here was wrong and the fix disproved it.** The 350–699 ms
population *pairs* to ~1001 ms (419+587, 424+577, 441+560, 464+538…), which was
read as a second independent 1 Hz timer. It is not: removing the ONE timer
removed BOTH populations. A single 1 Hz timer with any irregular source
interleaved produces exactly that pairing, because the pairs are **bracketed by
consecutive firings of the same timer**. ⇒ **To count timers, remove one and
re-measure — the pairing statistic cannot tell you.** What drives the remaining
0.65/s is unidentified; its dominant period is now **2501 ms**. Ruled out by
measurement: the under-glass eval-rebind loops (an under-glass ON/OFF A/B gave
an identical 1.6–1.7/s).

⚠ **And it explains why three storm autopsies in a row died undiagnosed.** They
all reported "unattributed, empty histogram", which reads as *nothing wrote* —
but `shell_mut_hist` counts only `safe_shell_mut`, and **raw `state.with_mut` is
invisible to it**. A no-op raw `with_mut` produces exactly that signature. The
instrument was not saying "nothing wrote"; it was saying "nothing I can see
wrote". Closing that blind spot is a prerequisite for the next autopsy meaning
anything.

### ⭐ AN IDLE GUI DOES NOT GROW — the accumulation is per-EVENT, not per-second

The obvious next hypothesis after the above — *something is attached to the main
loop every second and never released* — is **refuted**, and cheaply. A GUI with
**zero sessions** left alone in the sandbox for **66 minutes**:

| | 2 min | 66 min |
|---|---|---|
| CPU | 1.5% of a core | **1.5%** |
| render rate | 1.67/s | **1.6/s** |
| threads | 59 | **59** |
| RSS | — | 174 MB |

⇒ **Time alone accumulates nothing.** Whatever grows is created by *work* —
a surface opening, a session attaching, a tab — and never released. That kills
the whole "a timer/source leaks per tick" family and redirects the hunt to
per-event registrations. It also matches the one confirmed instance: a
`WebsiteDataStore` thread is born with a web context, not with a clock tick.

⇒ **The next experiment is therefore a CYCLE, not a wait:** open and close the
same surface/session N times in the sandbox and watch the population ratchet.
A vigil on an idle GUI cannot reproduce this and will read as "no bug".

**Still to attribute (this cluster's next load):** what grows in COUNT in the
wake path, hunted per-event. Baseline census for the diff, live GUI at 220 s:
**59 threads, 49 fds, 3 timerfd, 10 eventfd, 2 `ReceiveQueue`, 1
`WebsiteDataStore`** (aged: 13 and 11).

⚠ **guihost is a moving target while the batch runs** — three GUI pids inside one
hour on 2026-08-13, other clusters deploying. Growth measurements over hours
need either a quiet host or the sandbox on `dev`; a GUI that vanishes mid-sample
is usually another cluster's deploy, not your own doing.

## ⛔⛔ A PRIVATE INFRASTRUCTURE NAME SITS IN THE PUBLIC REPO AS A TEST FIXTURE

**Status:** OPEN

⚠ The working tree is clean; the PUBLISHED HISTORY is not.

⚠ **Scope corrected 2026-08-13 by a full-object sweep** (every blob from every
ref plus unreachable objects, every path name, every commit message, across all
17 owner-controlled public repos). This was filed as two fixtures. It is not:
**nine repositories carried private material, and this repository's own
published history carries 96 blobs and 13 commit messages** naming private
infrastructure, a personal home path, and — in shipped source, not only docs — a
bank hostname. The current branch is clean; anyone reading the history is not
reading a clean repo.

**Seven repositories are DONE** (rewritten and force-pushed 2026-08-13, each
verified by commit-count, ref-set and SHA-normalized subject parity, and by a
fresh anonymous clone showing zero hits). **This repository and one sibling are
orchestrator-gated** on a window where no cluster is pushing, because a rewrite
landing under a live lane is how work gets orphaned.

⛔ **A rewrite is only half the remedy.** A force-push shrinks the discoverable
surface and **revokes nothing**: pre-rewrite commits stay retrievable by SHA from
the host indefinitely — verified on a sibling repo three days after its own
force-push, by two independent methods. A support request under the
private-information-removal process was filed 2026-08-13; an earlier request was
deflected because it had been categorised as a repository-access issue.

⭐ **The method, the four controls, and the traps are written down** and are not
in this repository, deliberately — the replacement rules necessarily name the
strings being removed. A successor can execute cold from the runbook.

*found 2026-08-13 by the privacy guard, while it was blocking an unrelated push*

`crates/yggterm-core/src/titles.rs` (~line 1381) contains a test fixture whose
sample strings name a **real component of the private data fabric**. It arrived
with the 3.0.117 titles fix and is **already on `origin/main`**, so it is already
public — this is a cleanup, not a prevention.

⛔ **The term is deliberately not repeated here**, because this file is in the
same public repo. Read the line; the guard names it on any push that touches
that range.

**The rule it breaks** is the standing one: *invent every example.* Fixtures get
copied from whatever the author had in their head that day, and a fixture is
exactly the kind of line nobody re-reads. Replace the sample strings with
invented ones.

⚠ **Second, separate defect exposed by the same event, and it is the more
dangerous of the two:** the guard blocked a **later pusher** for an **earlier
author's** line — my range was clean (`git log -S` over `origin/main..HEAD`
empty) yet the push was refused, so the only way forward was
`YGG_PRIVACY_ALLOW=1`. ⇒ **The guard teaches the override to the one person who
did nothing wrong.** That is how an override becomes reflex, and the next
genuine hit gets waved through. The scan range must be the push's own commits.

## ⛔⛔ A ROW STOPS ACCEPTING INPUT AND THIS ONE IS *NOT* THE INPUT GATE

**Status:** OPEN

*owner-reported twice on 2026-08-13; second instance captured live on 3.0.128*

Owner: *"Once in a while a session stops responding to inputs. One of the worst
annoyances."* Symptom: no typing, no scrolling, a near-blank viewport after
switching to the row.

⛔ **THE FIRST INSTANCE WAS THE INPUT GATE AND IS FIXED (3.0.128). THIS SECOND
ONE IS NOT, AND CONFLATING THEM WOULD BURY IT.** Captured while stuck, on a GUI
whose build identity was read in the same command (`md5 44c6deb71a23`):

- **Zero `input_gate_*` events for the affected row**, in any log, for the whole
  episode. The gate is not refusing it — the gate has no opinion about it.
- `launch_phase: Running`, row reads `busy: agent_working_daemon`.
- The composer line on the daemon's own screen reads
  **`❯ Theping from 9.3re`** — fragments of several different strings
  interleaved into one line. Input is reaching *something* and landing
  scrambled; this is not silence.

⇒ **Two different faults produce the same user experience**, and the
distinguishing probe is one grep: an `input_gate_stuck_unrestorable` event for
that session path. Present ⇒ the gate (fixed). **Absent ⇒ this entry.**

### What was checked and did NOT explain it — so nobody re-walks these

- `terminal_host_mode: Unsupported` — **expected**, not a defect: that is the
  *Ghostty* native host, and this row runs `backend: Xterm`.
- `terminal_foreground_active: false` — **the documented misread.** That field
  family was renamed once already because it means "this host holds focus right
  now", not "the user can type". It is false on a perfectly usable unfocused row.
- `terminal_lines: 8` — a **snapshot truncation limit**, not the daemon's screen
  size. It briefly looked like a grid mismatch against escapes addressing rows
  48–65; the PTY is 169×65 and the escapes are consistent with it. No mismatch.

### The lead worth taking first

The interleaved composer line is the same shape as the already-recorded finding
that *the garble is substitution, not drift*. ⇒ Start by asking whether the
row's writer and its screen model disagree about the cursor, not by looking at
input plumbing — the keystrokes are arriving.

⚠ **Do not "fix" this by restarting the GUI.** A restart re-resumes every row
and clears the symptom without touching the cause, which is how it has survived
long enough to be reported as routine.

## ⛔⛔ [6.7] THE WEB PROCESS IS THE LARGER HALF OF THE IDLE COST AND NOTHING EXPLAINS IT YET

**Status:** OPEN

*measured on the desktop host 2026-08-13, 40 s window, build identity checked in
the same command*

⚠ **The fan is NOT this entry's question and is not answered by these numbers** —
see § *THE HOST RUNS AT 90+°C WITH 14 OF ITS 16 CORES IDLE*, which owns it. The
heat is a TLB-shootdown storm that CPU percentage cannot see. This entry owns
only the CPU share itself.

The owner reports the fan still running. On the desktop host, on a GUI **29 min
old** with the visible row sitting at a bare prompt:

| process | total | user | kernel |
|---|---|---|---|
| `WebKitWebProcess` (the GUI's own UI webview) | **25.0%** of a core | 16.3% | 8.7% |
| the GUI | 14.0% | 9.1% | 4.8% |

⚠ **Sample over a WINDOW.** A 2 s sample of the same machine read 49% + 20%;
40 s reads 25% + 14%. Short windows catch spikes — quote the window or the
number is not comparable to anything.

⚠ **There is exactly ONE `WebKitWebProcess`, and it is the shell's own UI
webview** (parent = the GUI), not a web surface. So this is our own chrome:
the sidebar DOM, the xterm canvas, and the JS we inject.

### What is ruled OUT, each by measurement

- **The `hpet` clock penalty does not explain it.** This load is
  **user-dominant** (16.3 user vs 8.7 kernel). The 45.8× penalty is a
  *kernel-time* effect on `clock_gettime`; it cannot inflate user compute.
- **Not terminal streaming.** The active row was at a prompt with 8 terminal
  lines — nothing to paint.
- **Not app-level work.** `ui-telemetry.jsonl` recorded **zero events in
  300 s**. The webview burns a quarter of a core while the app reports nothing
  at all, so the cost is *below* the event layer: style, layout, paint, or a
  timer that emits nothing.
- **Not the status-dot blink on its own.** The `:root { animation: … 1100ms
  step-end infinite }` is injected unconditionally and therefore also runs in a
  sandbox GUI — where the web process costs **1.7%**. (⚠ Not fully cleared: the
  blink animates an **inherited custom property** on `:root`, and the sandbox
  has ~0 dependent rows against the desktop host's ~45. A cost that is O(rows)
  would be invisible in the sandbox by construction. Refuted as a *whole*
  explanation, still live as an O(N) one.)

### What that leaves, and it is the shape to hunt

| | web process | GUI |
|---|---|---|
| sandbox, **zero sessions** | **1.7%** | 1.2% |
| desktop host, ~45 rows | **25.0%** | 14.0% |

⇒ **~15× on the web process, in USER time, and it tracks CONTENT rather than
uptime.** Whatever it is, it is proportional to what is on screen or in the
tree, and it runs with no events, no output and no user present.

⚠ **CORRECTION 2026-08-13, from the continuous recorder — the web process is
NOT twice the GUI, and the earlier reading here said so because the GUI's own
number was diluted.** The first recorder build classified processes by command
line, and every agent row is launched by an `ssh … /yggterm server remote
resume-cc …` wrapper, so a dozen near-idle ssh and bash processes were averaged
in as "gui". Keyed on `comm` instead, over a 10-minute window:

| role | corrected | as first reported |
|---|---|---|
| `web_content` | **32.5%** (n=56, 767 MB) | 28.3% |
| `gui` | **25.7%** (n=63, 152 MB) | 13.2% |

⇒ **~1.26×, not 2×.** Both halves are large and the GUI half is NOT a rounding
error next to the web half. Any plan that treats the webview as the only target
is working from the diluted number. ⛔ The instrument built to find this bug had
to be debugged twice before its numbers could be quoted — see the commits; the
lesson is that a role's SAMPLE COUNT must be printed beside its mean, because a
single stale row renders as a full role.

### SINGLE-BUILD READING, and why it does NOT settle whether the fix helped

Measured with the build identity read at **both ends** of the window, so a
mid-sample swap invalidates rather than silently corrupts it — `md5_start ==
md5_end == 1b54ac5de5db`, same pid throughout, 3.0.122, my fix proven an
ancestor of that build:

| | WebKit | GUI | total |
|---|---|---|---|
| 3.0.120, no fix, age 29 min | 25.0% | 14.0% | 39.0% |
| **3.0.122, fix present, age 10 min** | **15.8%** | **20.8%** | **36.6%** |

⛔ **Do not read this as a before/after. It is not one, and saying otherwise
would be the third wrong conclusion this campaign has drawn from an
uncontrolled pair.** The two samples differ in process age (29 vs 10 min) and,
far more importantly, in **workload** — three clusters were landing work on this
machine and the number of live rows, streaming sessions and open surfaces was
not held constant. The GUI reading even moved the *wrong* way for the growth
model (the older process was the cheaper one), which is the tell that workload
is dominating uptime here.

⇒ **What is honestly established:** the fix's effect is measured *in the
sandbox* (61% fewer idle re-renders, attributed by gap fingerprint), and its
**CPU share on the desktop host has never been isolated**. What this window does
establish is the standing figure the mandate is judged against: **yggterm costs
~37% of a core with nobody using it**, which is nowhere near "nothing
measurable".

⭐ **What would settle it:** a controlled A/B on this host — same row count, same
active session, alternating builds, md5-guarded at both ends of every window.
That needs the quiet host the orchestrator has offered twice and this cluster
has not yet claimed. **Claim it before attempting another before/after here.**

**Next instrument, and the gap that blocks it:** there is no way to evaluate JS
in the shell's own webview — `server app web eval` and `web devtools` both
target *session surfaces*, not the chrome. So "which JS timer / which style
invalidation" cannot currently be asked at all on a live GUI. **Either add a
shell-webview eval/devtools verb, or reproduce the row count in the sandbox**
(spawn N probe rows, watch whether the web process cost scales with N — that
also settles the O(rows) blink question above).

## ⛔⛔ [6.7] THE HOST RUNS AT 90+°C WITH 14 OF ITS 16 CORES IDLE, AND THE HEAT IS NOT CPU

**Status:** OPEN

*measured on the desktop host 2026-08-13 18:15–18:30, windows quoted per figure*

The entry above opens with "the owner reports the fan still running" and then
measures CPU percentage. **CPU percentage does not explain the fan, and chasing
it cannot.** Over a 30 s window the machine was **9.96% busy — 1.59 cores of
16** — while two independent sensors read **k10temp 93.6°C / acpitz 91.0°C**.
Across 1,204 recorder rounds spanning 3.3 h: min 49.0°C, **mean 69.6°C, max
97.1°C, 28 alarm rounds**, 6.6% of rounds above 85°C.

### ⭐ THE HEAT IS NOT CORRELATED WITH OUR CPU AT ALL, AND THAT IS THE FINDING

Temperature against concurrent whole-machine CPU, joined on 10 s buckets over
the recorder's full span:

| CPU band | n | mean temp | max temp |
|---|---|---|---|
| 25–50% of a core | 395 | 69.3°C | 93.6°C |
| 50–100% | 590 | 69.4°C | 95.0°C |
| 100–200% | 180 | 71.2°C | 97.1°C |

**Pearson r = 0.071 across n=1,170.** Mean temperature moves 1.9°C across a 4×
range of CPU, and the machine reaches 93.6°C inside its *lowest* band. **57 of
the 80 hot rounds (>85°C) happened while the entire machine was under one core
of load.**

⇒ **No optimisation in the 0.5–2 core range can move the thermals on this host.**
That does not retire the efficiency mandate — the CPU, memory and thread findings
below stand on their own merits — but it does mean **the fan is not a scoreboard
for this campaign's code work**, and treating it as one will burn sessions.

### ⛔⛔ A MECHANISM THIS ENTRY ASSERTED, AND THE EXPERIMENT THAT KILLED IT

**The claim, filed here earlier the same session and now withdrawn:** that the
heat was a TLB-shootdown storm from page migration keeping cores out of C3. It
had everything a good hypothesis has, which is exactly why it is recorded:

- 78% of all interrupts were software IPIs (**CAL 12,084/s, TLB 7,831/s**)
  against ~1,100/s for every hardware IRQ combined.
- The arithmetic fit almost exactly: **3,858 migrations/s × 2 shootdowns each =
  7,716 predicted vs 7,831 measured.**

**The experiment:** `vm.compaction_proactiveness=0` + `vm.watermark_boost_factor=0`
at runtime, which halts proactive compaction on demand.

| | before | after |
|---|---|---|
| `pgmigrate_success` | 3,858/s | **0.0/s** |
| `compact_migrate_scanned` | 1,436,800/s | **6,397/s** |
| TLB irq/s | 7,831 | 3,602 |
| C-state idle | 90.60% | 90.12% |
| **k10temp** | **93.6°C** | **92.6°C** |

**Migrations went to exactly zero and the temperature did not move.** Three
lessons, each of which cost a wrong claim:

1. ⛔ **The clean arithmetic fit was a coincidence of one window.** With
   migrations at zero, TLB still ran **3,602/s** — so a second, larger shootdown
   source was always there and the tidy 2× ratio hid it. **A ratio that fits is
   not a mechanism that holds.**
2. ⛔ **The TLB "improvement" is not claimable either.** Before-change windows
   ranged **1,210–7,831/s**; 3,602 sits inside that. Only `pgmigrate=0` and the
   225× scan drop are outside burst variance — because those are *mechanically
   forced* by the sysctl, not inferred from a rate.
3. ⚠ **Cores were ~90% C-state idle in BOTH arms.** A story about C-state exits
   should have been checked against C-state residency before it was written.

### ⚠ THE COUNTERS ARE BURSTY — SINGLE-WINDOW RATES PROVE NOTHING HERE

Two windows minutes apart, same build, same idle machine, **no change between
them**: TLB 7,831 → 1,210/s, migrations 3,858 → 809/s, compaction scanning
1,436,800 → 453,916/s. **6.5× between adjacent samples.** Any before/after on
these rates produces a confident number meaning nothing. Use a monotonic proxy
(cumulative `pgmigrate_success` per unit of `pgalloc_normal`) or many alternating
windows.

### What actually remains as a heat candidate — and WHO OWNS IT

**`/sys/firmware/acpi/platform_profile` tracks the POWER SOURCE, and the fan is
an AC-power phenomenon.** Observed directly:

| time | power source | profile |
|---|---|---|
| 18:15–18:39 | mains (USB-C charging) | `performance` |
| 19:06 | battery, discharging | `balanced` |

⛔ **CORRECTION TO THIS ENTRY'S OWN EARLIER CLAIM.** It said *"nothing owns this
setting"* on the strength of `systemctl is-active power-profiles-daemon tuned`
returning `inactive`. **That was a true answer to the wrong question.**
`org_kde_powerdevil` is running as a **KDE session process started by `kded6`,
not by systemd**, so systemd correctly reports it inactive while it is very much
alive. Something — PowerDevil's built-in default or the EC itself; neither
`powerdevilrc` nor `powermanagementprofilesrc` declares a per-source power
profile, so it is not yet distinguished — switches the profile on AC↔battery.
⇒ **Ask the process table, not the service manager, whether a desktop component
is running.**

⇒ The machine sits in `performance` **whenever it is plugged in**, which is the
desk state in which the fan was reported.

### ⭐ THE PROFILE ARMS — SETTLED BY AN INTERLEAVED A/B

⛔ **The earlier sequential windows were replaced, not extended.** The machine
went from mains to battery mid-session, which changes power source *and* profile
together, and the battery then charged — so any sequential arm comparison
straddles a moving thermal load. **Arms alternating every 5 minutes** put that
drift on both arms equally. Interleaving was unaffordable for the build A/B
(every swap cost a GUI restart); here a swap is one write and no restart, so the design
that was correct-but-impossible there is correct-and-free.

| arm | n | mean | p50 | peak | **>85°C** | >80°C |
|---|---|---|---|---|---|---|
| `performance` | 90 | 71.9°C | 70 | **92°C** | **9 (10.0%)** | 25 (27.8%) |
| `balanced` | 71 | 65.2°C | 65 | **83°C** | **0 (0.0%)** | 2 (2.8%) |

**`balanced` eliminates the >85°C band entirely — 0/71 against 9/90, Fisher
exact p≈0.004.** Mains throughout, `arm_overridden=0`, `off_mains=0`, verified
per sample rather than assumed.

⭐ **The mean moved this time (−6.7°C) where five earlier windows said it did
not.** The earlier windows were uncontrolled; this is what a controlled
comparison bought, and it is the direct answer to *"why does a 90%-idle machine
run hot"* — because the firmware profile, not the workload, sets the ceiling.

⚠ **An owner report arrived mid-trial and matched the schedule he could not
see:** "very hot … cooled down just now" at ~19:54, against a `performance`
block running 19:48–19:53 (peaks 91, 92°C) and a `balanced` block beginning
19:53:03 (62–74°C). ⛔ **The trial was then STOPPED EARLY and the profile pinned
to `balanced`** — it was 60 seconds from deliberately reproducing a condition the
owner had just reported as unusable. **An experiment does not outrank the
machine's owner using the machine.**

⛔ **AND THE HEAT DID NOT CAUSE THE TYPING FAILURE, though they resolved minutes
apart.** The same report said he could not type; the cause was a **poisoned
composer** — `Error: connecting to …/server-3-0-130.sock` sitting *inside* the
`❯` box of the row he was typing into — cleared by the socket alias at ~19:45,
not by the cooling at ~19:53. **Two faults, two fixes, one coincidence.** The
tempting single-cause story would have been wrong in both directions.

**What the trial changes and how to revert it:** `platform_profile` is a runtime
firmware setting; writing it caps the sustained power limit and fan curve and
changes nothing about software. `echo performance | sudo tee
/sys/firmware/acpi/platform_profile` restores it, and **a power-source
transition rewrites it regardless of what any agent set**. Nothing was left
applied by this campaign: the 19:06 write was a no-op because the machine had
already switched itself to `balanced` on unplugging.

⇒ **Owner gate**, in `owner-attention.md`: whether the *AC* profile should be
`balanced` rather than `performance`. That is a persistent preference about his
machine's power behaviour, not a defect fix.

### The compaction waste is real, but it is a separate and much smaller item

Independent of the heat: `/proc/buddyinfo` zone Normal holds **2 free order-9
blocks** (the 2 MB huge-page order) against 5,652 at order-7, and kcompactd was
scanning 1.4M pages/s at a **100% failure rate** (`compact_stall` 4.1/s ==
`compact_fail` 4.1/s) chasing blocks that do not exist. Stopping that removed
genuine waste and cost nothing observable. ⛔ **But do not file it as CPU** —
kcompactd measures **0.93% of a core**, kswapd 0.87%. It is worth stopping
because it is pointless, not because it is expensive.

⚠ Swap keeps growing regardless (`pswpout` 963/s, `pgmajfault` 1,469/s after the
change), which is the web-process entry below and is the item with real value in
it.

## ⛔ [6.7] THE RESOURCE RECORDER'S `temp_alarm` IS THE RAM SENSOR, NOT THE CPU

**Status:** OPEN

*found 2026-08-13 while reading the recorder's own history*

`scripts/resource-recorder.py::temps()` sets `alarm = 1` when **any** sensor
under `/sys/class/hwmon` reaches its own `temp*_max`. On the desktop host exactly
one sensor has a ceiling low enough to matter:

```
nvme     temp1 = 59C   max = 79C
spd5118  temp1 = 56C   max = 55C   <== the only one ever alarming
```

⇒ **`temp_alarm` reports "the DIMM sensor is 1°C over nominal", a near-permanent
condition, while reading as a CPU thermal alarm.** It fired in 29 of 29 rounds
during a window in which the CPU ran *cooler* than baseline (64–75.6°C), which is
how it was caught — the flag went **up** as the machine got colder.

⛔ This is the field-guide family: *the probe answers a different question than
its name suggests*, in the campaign's own first-act instrument (`ygg-resource
-recorder status`, the monitoring ritual). Anyone quoting an alarm count is
quoting the RAM.

**Fix:** record WHICH sensor alarmed, not a bare bit — the sample that is one
degree over a DIMM's nominal is not the sample that matters, and a count with no
source cannot be triaged. Not done in-session on purpose: the service was mid-
trial and collecting the data the entries above rest on, so restarting it would
have destroyed the measurement.

## ⛔⛔ [6.7] THE DESKTOP-HOST IDLE-CPU A/B CANNOT RESOLVE ITS OWN EFFECT — STOP SCHEDULING IT

**Status:** AWAITING A DECISION

*Decided by whoever owns cluster sequencing: the orchestrator holds a granted
window for this A/B, and this entry argues it should not be spent here.*

*measured 2026-08-13 from the recorder's own history; no new instrument needed*

Three attempts at a desktop-host before/after have now come out confounded. The
reason is not workload discipline, it is arithmetic: **the quantity being
measured has a noise floor an order of magnitude larger than the effect.**

### The do-nothing floor, same pid, same build, nothing changed

| | windows | min | mean | max | **sd** |
|---|---|---|---|---|---|
| GUI, 5-min windows | 33 | 11.5% | 31.1% | 57.9% | **12.65** |
| GUI, 30-min windows | 5 | 12.2% | 31.6% | 47.7% | **11.65** |
| web_content, 5-min | 33 | 17.0% | 27.1% | 38.1% | 5.29 |

**The GUI's idle CPU swings 11.5% → 57.9% of a core on ONE build with no change
of any kind.** ⭐ **And lengthening the window does not help: sd falls only
12.65 → 11.65 from 5 to 30 minutes.** That is the whole finding — the variance is
**low-frequency workload drift, not sampling noise**, so it does not average out
and a longer window buys almost nothing.

### The power calculation, stated BEFORE any arm is run

At sd = 11.65, 80% power, α = 0.05, `n ≈ 16·(sd/effect)²` per arm:

| effect | windows/arm | wall-clock at 30 min/window |
|---|---|---|
| **3 pp** (the plausible effect) | **241** | **~10 days per arm** |
| 10 pp | 22 | ~22 h per arm |

**The fix's measured effect is 0.1 pp on the build host** (1.5% → 1.4%). Even
scaling generously for the `hpet` clock penalty the arm difference is ~1–3 pp —
**below what this design can resolve, by roughly 10×.**

⇒ **A "result" from a handful of windows here is noise with a sign.** That is how
three prior attempts produced confident, contradictory numbers, including one in
which the *older* process was the cheaper one.

### What to do instead

1. ⭐ **The fix is ALREADY attributed by the instrument that can attribute it.**
   The sandbox gap fingerprint is deterministic, not statistical: idle renders
   **1.65 → 0.65/s (61%)**, with the 1001 ms gap population going **31.3% → 0%**.
   Killing a timer removes its period from the histogram — that is causation, and
   it needed no quiet host at all.
2. **If a host-level number is genuinely wanted, the only valid design is
   INTERLEAVED, not sequential** — alternate arms every ~10 min for many hours so
   drift lands on both arms equally, and compare *adjacent* pairs. Sequential
   arms cannot beat a drifting confound at any sample size. ⚠ That means a GUI
   restart every 10 minutes for hours on the owner's daily driver, which is a
   far larger intrusion than the number is worth.
3. **Spend a quiet host on something whose signal exceeds its noise instead** —
   e.g. the web-process plateau test, where the quantity grows ~366 MB/h
   monotonically against a much smaller variance.

⛔ **The general rule this entry exists to install:** *state the do-nothing floor
before designing the comparison.* One query against the recorder answers it, and
it would have saved three attempts. A perf number quoted without its floor is not
a measurement.

## ⛔⛔ [6.7] OUR MANAGED npm CACHE IS 13.3 GB ACROSS THE FLEET AND NOTHING EVER PRUNES IT

**Status:** OPEN

*Found 2026-08-14 while independently re-measuring a peer's trace-growth figure —
the growth reproduced, but the directory it was blamed for filling turned out to
be full of something else entirely.*

**It is the largest single consumer of `~/.yggterm` on BOTH hosts**, and it is
ours: `codex_cli.rs` points the managed Codex CLI install at
`~/.yggterm/npm-cache` (`MANAGED_NPM_CACHE_DIRNAME`).

| host | `~/.yggterm` total | `npm-cache` | share |
|---|---|---|---|
| build host | 9.5 GB | **7.6 GB** | 80 % |
| desktop host | 7.8 GB | **5.7 GB** | 73 % |

On the desktop host that is **5.7 GB in `_cacache` across 1,476 files, with
content dating from 2026-03-28 to today** — every tarball of every version ever
provisioned, kept forever. `npm cache clean`, `--cache-min`, and any retention
or GC rule are **absent from the whole file**: nothing has ever removed anything
from it.

⛔ **The fix is a retention rule, NOT a delete.** The cache exists so CLI
provisioning is not a fresh download every time; emptying it on a timer trades
disk for network and start latency on the very path that has to be fast. What is
missing is a bound — the same shape as the memory entry below, and the same
error: an unbounded store whose growth nobody is watching.

⭐ **Falsifier, cheap and decisive:** record the cache size, run a provisioning
pass, and record it again. If the delta is ~one package set, the accumulation is
purely historical and a retention rule reclaims essentially all of it; if it is
large, provisioning itself is re-downloading and the entry is about the wrong
layer.

### ⚠ AND IT CORRECTS WHAT THE TRACE-GROWTH NUMBER MEANS

The trace write rate is real and reproduced independently on the build host
(**133 KB/s combined across `event-trace.jsonl` + `perf-telemetry.jsonl`, ≈11
GB/day**), but ⛔ **it is a WRITE-RATE cost, not an occupancy cost** — those two
files are only **5 MB and 9 MB** on disk, because they rotate. The 9.5 GB is not
them. ⇒ Fixing the trace volume buys **CPU, I/O and SSD wear**, and buys back
**no meaningful disk**; the disk is bought back here instead. Two different
frugality wins that were about to be quoted as one number.

⭐ **AND IT DOES NOT TRANSFER BETWEEN HOSTS.** Measured in the same 15 s window:
the build host writes **133 KB/s** and the desktop host writes **6 KB/s** — a 22×
gap, tracking session count (340 sessions / 21 daemons versus 2 / 7), not
hardware. **The trace-volume defect is real and is essentially absent from the
owner's laptop right now**, so it must not be sold to him as his own machine's
problem. This is the campaign's standing rule earning itself again: a claim
proven on one host is not proven, and a rate must be quoted with the host it was
taken on.

## ⛔⛔ [6.7] THE WEB PROCESS'S MEMORY BOUND CANNOT HOLD, BECAUSE SWAP MAKES ITS FOOTPRINT LIE

**Status:** OPEN

### ✅ CONFIRMED LIVE 2026-08-14 — THE BOUND IS ALREADY BEING EXCEEDED, INVISIBLY

Previously this was settled from `strings` on the shipped `.so` (the bound polls
`VmRSS`). It is now measured on the running host, and the process has **already
crossed the limit without the limit noticing**:

| yggterm's web process (child of the GUI) | |
|---|---|
| `VmRSS` — **what the bound compares against** | **947 MB** |
| `VmSwap` | 1,118 MB |
| **committed (`RSS + swap`)** | **2,065 MB** |
| the configured bound (`MemTotal/8`) | **1,889 MB** |

⇒ **176 MB over the bound, and the bound reads 947 MB — barely half.** The
overage is not merely unseen, it is unseeable *by construction*: the amount by
which the process exceeds its budget is precisely the amount the kernel swapped
out, which is exactly what an RSS-valued comparison stops counting. **The metric
subtracts the evidence of the thing it is measuring.**

⛔ **This is why "tune the constant" is the wrong instinct and remains refused.**
No value of `MemTotal/8` fixes a comparison whose left side shrinks as the
problem grows — lowering it does nothing once the excess is in swap. Either the
comparison becomes swap-inclusive (a cgroup, which counts what the machine
actually committed) or the bound stays decorative.

### ✅ THE CGROUP OPTION IS CONFIRMED REACHABLE — SPEC'D, WITH THE NUMBERS THAT SETTLE IT

*Live on the desktop host 2026-08-14, read-only.* The kernel is **already
measuring the exact quantity WebKit cannot**:

```
GUI cgroup      /user.slice/user-1000.slice/session-<id>.scope
controllers     cpu memory pids          <- memory controller present
memory.current        1,029 MB
memory.swap.current   2,307 MB           <- the half an RSS poll cannot see
                    = 3,336 MB committed
memory.high           max                <- no bound at all
```

⇒ **Everything the honest fix needs is present**: the controller is enabled, and
`memory.current` + `memory.swap.current` is precisely the committed figure that
`VmRSS` structurally under-reports. There is nothing to invent — only to bound.

**Two facts block doing it in place, and they define the shape of the fix:**

1. ⛔ **The scope is not ours.** It is the login `session-<id>.scope`, shared with
   ~15 unrelated processes, so a bound set there would police the desktop rather
   than yggterm.
2. ⛔ **`memory.high` is not writable** by the process — systemd owns it.

⇒ **THE FIX: launch the GUI in its OWN systemd user scope**, e.g.
`systemd-run --user --scope -p MemoryHigh=… -p MemorySwapMax=…`, which every
WebKit child inherits — one bound covering the whole family, enforced by the
kernel against committed memory instead of by WebKit against residency.
⭐ Use **`MemoryHigh` (reclaim pressure), not `MemoryMax` (OOM-kill)**: the goal is
frugality, and a hard cap on a browser engine turns a memory spike into a dead
web surface.

⚠ **NOT YET IMPLEMENTED, and deliberately so.** The change is in the LAUNCH path,
whose failure mode is *the GUI does not start* — and it cannot be verified
without relaunching the GUI, which is an owner gate. ⛔ Shipping an unverifiable
launcher change is the one move here with a worse downside than the bug: the bug
costs memory, a bad launcher costs the whole app. Land it behind a default-off
switch and turn it on inside a relaunch window.

⚠ **Scope, stated honestly so nobody over-claims it.** The whole yggterm family
(GUI + 4 WebKit children) commits **≈3.5 GB, of which ≈2.0 GB is swapped**, on a
14.8 GB host carrying **12.3 GB of swap in use**. So yggterm is a significant
share of the swap pressure, **not the cause of it** — the rest is spread across
many unrelated desktop processes. ⇒ Fixing this bound makes yggterm frugal; it
will not by itself bring the machine out of swap, and claiming otherwise would
set up a measurement that is guaranteed to disappoint.

*single-pid lifetime measurement, desktop host 2026-08-13, restart-free*

`configure_linux_webkit_memory_policy` (`apps/yggterm/src/main.rs`) sets
`YGGTERM_WEBKIT_CACHE_MODEL=web-browser` and a limit of
`(MemTotal_MB / 8).clamp(768, 3072)` — **1,888 MB** on this 15 GB host — with
conservative/strict thresholds at 0.75/0.90 (**1,416 / 1,699 MB**). The code
comment states the intended contract outright: *"the bound on it is not this
knob but the memory policy below … so caching more does not mean growing
without end."*

⛔ **That contract is not holding, and the cache model is NOT the defect.**
`docs/optimization-pass.md` §9f settled `web-browser` deliberately — a cacheless
model made every navigation refetch every byte. **Do not reach for that knob.**

### The measurement

One `WebKitWebProcess`, one pid, tracked across its own lifetime (so no restart
straddles the samples):

| age | RSS | swap | **committed** |
|---|---|---|---|
| 0.0 h | 516 MB | 11 MB | **526 MB** |
| 1.0 h | 776 MB | 243 MB | **1,019 MB** |
| 2.0 h | 604 MB | 655 MB | **1,259 MB** |
| 2.2 h (`smaps_rollup`) | 588 MB | 931 MB | **1,519 MB** |

**+733 MB in two hours, monotonic, no plateau — ~366 MB/h.**

⚠ **RSS peaks at hour 1 and then FALLS while the total keeps climbing.** Watching
RSS alone would have shown this leak *reversing*. The campaign's own rule earns
itself again: the number that must fit in the machine is `rss + swap`.

### Why the bound cannot fire

At 2.2 h the process holds **1,519 MB committed — already past the 1,416 MB
conservative threshold — and no reclaim has occurred.** Its RSS is 588 MB, 31%
of the limit, and would have to *more than double* before a resident-based
threshold could trip.

⇒ **Empirically WebKit is not reacting to committed memory.** The loop is
self-reinforcing and gets worse exactly when it should get better: swap pressure
evicts the cache → RSS falls → WebKit reads more headroom → it caches more →
more swap pressure.

### ✅ SETTLED 2026-08-13: THE POLLED QUANTITY IS **RESIDENT ONLY**

The entry above said this was inferred from behaviour and demanded it be settled
before any fix. It is now settled from the implementation, two independent ways:

1. **Upstream source.** WTF's `linux/CurrentProcessMemoryStatus.cpp` opens
   `/proc/self/statm` and parses `size, resident, shared, text, lib, data, dt`.
   `statm` has **no swap field at all** — swapped-out pages are simply absent.
2. **The shipped library** (2.52.5, the one actually loaded). The only
   `/proc/self/status` field name in the whole binary is **`VmRSS:`**. There is
   no `VmSwap`, no `smaps_rollup`, no `statm`-adjacent swap read anywhere in it.

⇒ **The bound is evaluated against resident memory, and resident memory is
exactly the quantity the kernel is free to shrink by swapping.** The two forks
collapse to one: the metric is resident, so the bound is not "firing and
failing" — it is structurally unreachable on a host that swaps.

**Corroborated by the recorder, on the one web process long-lived enough to
show a curve** (per-pid lifetime buckets, so no restart straddles it):

| age | RSS | swap | committed |
|---|---|---|---|
| 0 h | 586 MB | 63 MB | 649 MB |
| 1 h | 714 MB | 327 MB | 1,042 MB |
| 2 h | 647 MB | 715 MB | 1,362 MB |

**RSS is flat across the whole climb while swap grows 11×.** The conservative
threshold is 1,416 MB *of RSS*; RSS never leaves the 586–714 MB band. The
footprint doubles past the threshold while the number WebKit reads does not move.

⭐ **The original falsifier is now answerable on paper**: a plateau near 1,888 MB
committed was never possible, because nothing compares committed to 1,888 MB.

### ⭐⭐ THE FORK WAS FALSE — NO CONSTANT CAN FIX THIS, AND THE CURRENT ONE IS NOT THE DEFECT

This entry previously said the fix was "blocked on a choice" between a
swap-inclusive limit and lower fractions. **That framing was wrong and is
withdrawn.** Both candidates try to fix a *metric* problem with a *number*, and
that cannot work in principle: the threshold is compared against RSS, and the
kernel can push RSS arbitrarily far below any threshold by swapping the cache
out. **There is no constant that bounds a footprint through an RSS-valued
comparison.**

⇒ **`(MemTotal_MB / 8).clamp(768, 3072)` is already a derived rule, not a fitted
constant.** It is a function of what the machine supplies, it is deterministic,
and it travels across hosts. Read as what it actually is — *"this process may
hold an eighth of RAM **resident**"* — it is defensible and should stay.

**The reductio, from this lane's own measurement.** For conservative (0.75 ×
limit) to trip against the observed 586–714 MB RSS band, the limit would have to
be ≤ ~781 MB — i.e. a 15 GB host would have to be given the **768 MB floor the
policy reserves for the smallest machines it supports.** Tuning the constant
until it fires means abolishing the rule that derives it. That is the whole
argument against the tuning route, and it needs no new measurement.

⇒ **This is therefore NOT an owner item.** The open question is not *"what
number"* — which would be his guesswork substituted for engineering — it is
*"which mechanism"*, which is ours. Two honest options remain:

1. **Change what the metric means.** Scope the process with a cgroup
   (`memory.high`, not `memory.max` — throttle and reclaim, never OOM-kill) and
   deny swap to that scope, so RSS becomes the true footprint again and the
   existing thresholds start meaning what they say. This is the only option that
   makes the *already-correct* derived limit work as written. ⛔ Unmeasured; it
   changes the failure mode, so it needs its own falsifier before anyone builds
   it, and `memory.max` would trade unbounded growth for a killed tab.
2. **Accept the bound as a residency cap and say so.** Fix the comment in
   `configure_linux_webkit_memory_policy`, which currently claims the policy
   means "caching more does not mean growing without end". It does not, and a
   comment asserting a guarantee the code cannot make is how two relays came to
   treat a working knob as a broken one.

⭐ **Option 2 is owed regardless of whether anyone builds option 1** — it costs
nothing and removes the false belief that caused this entry.

## ⛔ [6.7] THE JAR-LESS WEB CONTEXT GOT NO MEMORY BOUND AT ALL — FIXED IN CODE, LIVE PROOF OWED

**Status:** FIXED IN CODE — LIVE PROOF OWED

⛔ **HARVEST ATTEMPTED IN THE 3.0.154 WINDOW — NOT TAKEN, AND DELIBERATELY SO.**
The observation needs a web process born from the new build, which means
relaunching the GUI. The running GUI holds the owner's **unsent half-typed
draft**, and restarting it would destroy that. ⇒ Owed until the GUI is next
restarted for a reason of its own; do not schedule a restart to collect it.

*Found by reading the consumer of the policy the entry above configures, 2026-08-13.*

The entry above shows the memory bound is too weak to fire. On one path it was
not merely weak — **it was never applied.**

`WebContextImpl::new` (`vendor/wry/src/webkitgtk/web_context.rs`) applied the
configured memory-pressure settings; `WebContextImpl::new_ephemeral`, six lines
below it, applied only the cache model and built a bare
`WebContext::new_ephemeral()`. **Two constructors, one policy, applied in one of
them.**

⇒ **Reachable, not theoretical.** The surface host picks the ephemeral
constructor for every surface whose context key has no profile directory —
the jar-less (temp) profile:

```rust
None => (Rc::new(RefCell::new(WebContext::new_ephemeral())), true),
```

So a temp-profile surface ran a web process with **no configured limit and no
thresholds** — inheriting the engine's defaults — while every persistent-profile
surface got the policy. Nothing reports this at runtime: the settings are
construct-only properties with no getter, so an unbounded engine and a bounded
one are indistinguishable from outside.

**The fix.** One helper, `context_builder_with_memory_policy()`, is now the only
place a context builder is created, and both constructors go through it. It
applies **both** halves, which are not duplicates of each other: the builder
property bounds the **web** processes, the static
`WebsiteDataManager::set_memory_pressure_settings` bounds the **network**
process.

⚠ **The bound was not bought with ephemerality.** `webkit_web_context_new_ephemeral()`
is defined as "a context whose website data manager is ephemeral", so the
replacement composes the engine's own `WebsiteDataManager::new_ephemeral()`
rather than setting an `is-ephemeral` property by hand — the jar-less guarantee
still comes from the engine's constructor, not from this file. Trading it away
would have been a worse bug than the one being fixed: it would put temp-profile
browsing on disk.

**Locked by** `every_vendored_web_context_is_built_with_the_memory_policy`
(4 clauses: one builder site; the helper applies both halves; **both**
constructors route through it; and the ephemeral one still composes the engine's
ephemeral data manager).

⛔ **The lock lives in `yggterm-shell`, not in the file it scans**, because
`vendor/wry` is **not a workspace member** — a test written inside it would
never run. That is the same failure this repo's own manifest records against the
other vendored crate, where 67 tests sat green and unrun for weeks.

**Live proof owed:** open a temp-profile surface on a build carrying this and
confirm it still writes **no jar to disk**. That is the risk this change
introduces, and it is the half that a source scan cannot settle.

## ⛔ [6.7] A DEAD PTY'S WRITER THREAD — FIXED IN CODE, LIVE PROOF OWED

**Status:** FIXED IN CODE — LIVE PROOF OWED

⛔ **HARVEST ATTEMPTED IN THE 3.0.154 WINDOW — NOT TAKEN, BECAUSE THE CONTROL
ALSO PASSED.** Counting `pty-writer-*` against `pty-reader-*` in
`/proc/<pid>/task/*/comm` on six live daemons: 3.0.154 gave 2 = 2, and so did
every **pre-fix** control — 3.0.153 (10 = 10), 3.0.62, 3.0.52, 2.12.24 and
2.12.14, at 100-450 h uptime. ⚠ **A favourable reading against a negative
control that also passes supports nothing**: the leak is simply not reproducing
on this host, so the run cannot separate "the fix works" from "there was nothing
to see". The original defect was measured as 22 writers against 19 readers on
the GUI host ⇒ **take this proof there, on a daemon with real session churn.**

*Falsified by: a daemon that has served and closed sessions for hours showing
`pty-writer-*` and `pty-reader-*` thread counts that disagree. They are created
in pairs, so equal counts is the whole claim.*

**The leak.** `spawn_terminal_writer_thread` (`terminal.rs`) is called from
inside the reader-spawn path, one writer and one reader per terminal. The reader
exits on `Ok(0)` (PTY EOF); the writer runs `while let Ok(request) = rx.recv()`,
which ends only when **every** `SyncSender` clone has dropped. The reader's clone
drops at EOF — but the clone the terminal entry holds does not, so a PTY that
dies while its entry survives **parks its writer on `recv()` forever**, holding a
thread and its stack.

⇒ A writer whose PTY died *does* exit if anything writes to it (the write fails
and it breaks). It only leaks when **nothing ever writes again**, which is
exactly the state a closed row is in.

**Measured before the fix**, live daemon vs an idle one of the same build:

| thread | live daemon | idle daemon |
|---|---|---|
| `pty-writer-remote…` | **22** | 1 |
| `pty-reader-remote…` | **19** | 1 |

**Three writers outliving their readers**, and it only grows. Alongside it, on
the same daemon: threads **31 → 59 over 3 h with CPU 4.0% → 9.8% and memory
flat** — the campaign's predicted "hot loop proportional to accumulated state",
with the accumulating population named.

**The fix.** `TerminalWriteRequest` gains a `shutdown` flag; the reader sends one
**after its loop, so EOF and the read-error path are both covered**, and the
writer breaks on it. ⛔ **Deliberately not a timeout poll** — a writer that wakes
periodically to check a flag is precisely the idle cost this lane exists to
remove, so the writer still blocks on `recv()` and is woken by a message. The
shutdown uses `try_send`: if the queue is full the writer is mid-write against a
dead PTY and will fail and break by itself, and blocking there would strand the
reader instead.

**Locked by** `a_writer_retires_on_shutdown_even_while_another_sender_is_alive`,
which asserts **both halves** — that a surviving sender alone leaves the thread
parked (so the test cannot pass trivially), and that the shutdown retires it with
that sender still alive. 86 `terminal::` tests pass.

⚠ **Not claimed: that this is the whole thread growth.** The GUI shows the same
family — threads 63 → 76 in 2 h with memory flat, including **29
`tokio-rt-worker`** where one multi-thread runtime on a 16-core host spawns 16 —
and that is a different population in a different process. This fix is the
daemon's `pty-writer` half only.

⭐ **CORRECTION 2026-08-13 — the GUI's tokio thread COUNT is not the cost, and
capping `worker_threads` would have been a fix for nothing.** A per-thread
context-switch census on the live GUI at rest (`voluntary_ctxt_switches` deltas
over 20 s, every task in `/proc/<pid>/task`) shows the workers are not a pool of
idle threads sharing the load:

| thread | switches / 20 s |
|---|---|
| `tokio-rt-worker` (**one** of them) | **2,486** |
| `ReceiveQueue` (one) | 1,594 |
| main `yggterm` | 1,391 |
| `VBlankMonitor` | 1,267 |
| every other `tokio-rt-worker` | 26 – 110 each |

⇒ **One worker wakes ~124×/s at rest; the rest are parked and cost only their
stacks.** Reducing the pool would relocate that task, not remove it. The thing
worth finding is the ~8 ms-period task driving that one worker. ⛔ Filed as an
observation, NOT a mechanism — the period has not been attributed to a callsite,
and this lane has three dead causal stories behind it that all began with a
number this clean.

⚠ The GUI's *runtime* is `vendor/dioxus-desktop/src/launch.rs`
(`Builder::new_multi_thread()` with no `worker_threads`), so the pool is
`num_cpus`. That is the right thing to know before touching it and the wrong
thing to touch first.

⚠ **Suspected, NOT proven:** that the retained entry is the same orphaned-key
family as the restore bug — a close that resolves to a key nothing holds removes
nothing and leaves the entry behind. If so, this fix stops the thread leaking
while the orphaned entry itself remains.

## ⛔ [6.7] THE WEB-CONTEXT SHARING INSTRUMENT IS BLIND TO THE CASE IT EXISTS TO CATCH

**Status:** OPEN

*found by code inspection 2026-08-13, against the live reading in the entry above*

`web_surface_contexts` read **0** while 5 surfaces and 36 tabs were live. That
looks like a broken counter. It is worse: it is an accurate count of the wrong
population.

`WebSurfaceHost::web_context_count()` returns `self.contexts.borrow().len()` —
the **keyed** map only (`vendor/dioxus-desktop/src/web_surface.rs`). Its own
doc-comment states the invariant it is for: *"with N tabs open on one session
this must read 1, not N."* But the key is
`web_context_key(profile_dir, socks_port, signer_base)`, which returns `None`
the moment `profile_dir` is `None`, and that arm takes

```rust
None => (Rc::new(RefCell::new(WebContext::new_ephemeral())), true),
```

— **a fresh `WebContext` per surface, never inserted into the map.** So an
ephemeral surface gets its own `WebsiteDataManager` and process pool, and is
counted **zero** times. ⇒ *0 contexts with 41 surfaces live* does not mean "no
contexts"; it is exactly what **41 unshared contexts** looks like, which is the
failure the instrument was built to detect. It cannot distinguish "sharing is
working" from "sharing is not happening at all".

⚠ **And the ephemeral arm is reached in normal operation, not just for temp
profiles.** `shell.rs` hands a surface `profile_dir: None` whenever the
profile's **write-lock is held elsewhere** — another GUI, a shadow client, a
second cluster. On this fleet that is routine, so a long-lived GUI accumulates
unshared contexts through ordinary use. That is a per-event population that
grows with work and not with time, which is the shape the entry above says to
hunt.

⚠ **Not yet proven to be the leak.** The surface *retains* its context
(`_ctx: Some(ctx_cell)`, line ~5199), so a context should die with its surface;
this is a confirmed **measurement** defect and a strong lead, not a confirmed
leak. What settles it: cycle a jar-less surface open/closed N times and watch
`WebsiteDataStore` thread count — if it ratchets, the retention is the bug; if
it does not, only the counter is.

**The fix for the instrument half is not optional either way:** count every live
`WebContext`, keyed and ephemeral alike, or rename the field to say it counts
shared ones. A reader cannot currently tell the healthy reading from the worst
one.

## ⛔ [6.7] A DAEMON RE-DROPPED THE SAME UNRECOVERABLE SESSIONS 3×/s — FIXED IN CODE, LIVE PROOF OWED

**Status:** FIXED IN CODE — LIVE PROOF OWED

⛔ **HARVEST ATTEMPTED IN THE 3.0.154 WINDOW — NOT TAKEN, AND THE FIRST READING
WAS CONTAMINATED.** A corpus-wide count of `live_session_persist_dropped`
returned 2 events over 2 distinct keys, which looks exactly like the fixed
behaviour. **It is not a daemon.** Every one of those events came from pid
3516098, a **`cargo test` binary** writing into the same `~/.yggterm` trace
directory — its sibling events carry fixture paths like `wedge-signal-probe`.
Live daemons emitted **none**, so the measurement is vacuous: with no
unrecoverable session on the host, a fixed and a broken daemon both print zero.
⇒ Needs a daemon that actually holds an unrecoverable key. See the field guide
on the shared trace directory.

*Falsified by: `grep -c live_session_persist_dropped` on a fresh daemon's
`event-trace.jsonl` after an hour at rest. It must equal the number of DISTINCT
unrecoverable keys, not grow with time.*

**Measured before the fix.** `live_session_persist_dropped` fired **92 times in
30 s (184/min)** from one daemon — the same two keys, always
`"reason":"not_recoverable"`. **15% of all trace lines.** `event-trace.jsonl`
grew **13.8 MB/hour** and all `~/.yggterm/*.jsonl` together **28 MB/hour**, at
rest, with several daemon generations each writing their own.

**Root cause.** `persisted_state_with_update_protection` runs on every persist
pass and calls `trace_drop` for each live session that fails
`managed_live_session_is_recoverable`. There was no dedup, so a permanently
unrecoverable session was re-judged, re-dropped and **re-logged forever**.

⭐ **The compute was never the cost — the WRITE was.** The recoverability check
is cheap; the trace is disk I/O on a machine with nothing running. A fix aimed at
the loop rather than at the logging would have optimised the wrong half.

**The fix.** `persist_drop_already_traced(key, reason)` keeps the **first**
occurrence and drops the repetitions. The first line is the whole diagnostic —
it names which gate dropped which key, which is what the 2026-06-11 incident
needed — and the repetitions carry no information it does not.

⛔ **Keyed on `(key, reason)`, not on `key`.** A session later dropped for a
*different* reason is a new fact and must still be logged. Bounded by the number
of session keys, so the set cannot grow without limit.

**Locked by** `a_repeated_persist_drop_is_traced_once_but_a_new_reason_is_traced_again`,
which asserts all three halves — first traced, repeat not traced, new reason
traced again. Full suite: 1069 pass.

⚠ **Checked, because the dedup is a process-global static and a test binary is
one process:** no test anywhere asserts on persist-drop traces, so this
introduces no order-dependence in the suite. (Two tests failed on one run and
passed on the next; both manipulate process-global env vars — `TERM`,
`COLORFGBG` — behind a guard, a pre-existing parallel-execution flake unrelated
to this change.)

⚠ **Not claimed: that this removes the 28 MB/hour.** It removes the largest
single contributor (15% of lines) from *one* event. Several daemon generations
each holding their own trace open is a separate item.

## ⛔ [6.7] UNEXPLAINED: HAND-MADE SOCKET ALIASES AT A JUST-RETIRED NAME WERE DELETED, SIBLINGS WERE NOT

**Status:** OPEN

*observation retained after the hypothesis built on it was falsified — the
hypothesis is dead, the data is not*

**What was seen**, on the GUI host in the minutes after 3.0.131 retired:

| alias | outcome |
|---|---|
| at `server-3-0-131.sock` (the just-retired name) | **deleted 3×**, within seconds to ~100 s of creation |
| at 3 other versions, created in the **same command** | survived untouched |
| the same alias at the same path, 7 min later | survived 150 s |

⇒ Whatever deleted them is **transient, fires in a window around a retirement,
and targets exactly the name a retiring daemon writes.** Siblings born in the
same syscall batch were spared, so it is not a blanket sweep.

⛔ **The mechanism proposed for it is FALSIFIED and must not be repeated:** that
`classify_socket_entry` condemns an entry by NAME on a re-proved dead sighting
and executes that sentence later against a different file. At the next handover
the falsifier ran in the wild on two hosts — `retiring_daemon_aliased_own_socket`
fired, the alias at the retiring name resolved to the live successor, and it
**survived 535 s**. The case that mattered is not affected.

⚠ **So this is a curiosity, not a blocker** — nothing is sequenced behind it and
the daemon-written bequest is not being eaten. It is recorded because **a
hypothesis dying does not kill the observation that prompted it**, and an
unexplained deletion in the socket lifecycle is worth having on file the next
time something goes missing there.

**Falsifier for the remaining question:** create an alias at a just-retired
version's name and watch it for two minutes, with the sweep's own trace on. It
either dies again — and the deleter is findable — or it does not, and this was
tied to a condition that no longer exists.

## ⛔ [6.7] AN UNCORKED AUDIO STREAM HELD FOREVER — FIXED IN CODE, LIVE PROOF OWED

**Status:** FIXED IN CODE — LIVE PROOF OWED

⛔ **HARVEST ATTEMPTED IN THE 3.0.154 WINDOW — NOT TAKEN.** Same constraint as
the jar-less web context above: it needs the GUI running the new build, and the
running GUI holds the owner's unsent draft. The entry's own warning still
stands and now names the release: the deployed **binary** is 3.0.154, but the
**running GUI process** is not.

*Falsified by: with no notification playing and the GUI idle past the awake
window, `pactl list sink-inputs` must show no uncorked yggterm stream and the
filter sink must reach `SUSPENDED`. ⛔ **The running GUI does NOT carry this
fix** — it is in the lane, not deployed.*

**What was wrong.** The shell kept ONE long-lived `AudioContext` and left it
**running forever**, so the webview held an uncorked playback stream with
nothing in it: `Corked: no` on a GUI 11 minutes old, 8 GB free, no pressure of
any kind. Two costs — a pipeline that never suspends (continuous laptop draw),
and a stream fed nothing for hours that underruns, so the chime that finally
arrives plays through a starving pipeline. That is the "lagged and distorted"
notification, and it is a missed real-time deadline, not a memory condition.

### ⛔ THE OBVIOUS FIX IS THE ONE ALREADY TRIED AND REVERTED

This entry previously suggested "build it per notification and drop it after".
**That was the original design and it produced a defect the owner reported by
ear:** a context closed shortly after the last note while a Bluetooth sink still
held 100–300 ms of buffered audio — the **clipped ending**. A second prior fix
resumes the context because WebKitGTK starts one SUSPENDED until a user gesture,
and without it every chime scheduled while the user was away was dropped
**silently**. ⇒ **Any fix here has to keep both of those intact**, which rules
out `close()` and rules out per-chime construction.

### The fix, and why the deadline is not a new number

`suspend()` — never `close()` — on an idle timer, reset by each chime. The
context object is kept and reused, so the clipped-ending fix stands; the
existing `resume()` path already wakes a suspended context, so the silent-drop
fix stands.

⭐ **The suspend deadline IS `NOTIFICATION_PREROLL_LINK_AWAKE_WINDOW_MS`**, the
constant the Bluetooth pre-roll decision already uses — not a second tunable
beside it. That makes the two agree *by construction*: while the context is
running, the next chime falls inside the awake window and correctly skips the
pre-roll; once it suspends, the next chime falls outside it and correctly
pre-rolls a cold link. A separate constant could drift and give a chime that
skips the pre-roll onto a link that has gone to sleep — the clipping defect,
reintroduced by disagreement rather than by design.

**Locked by** three assertions on the generated script: that it suspends, that
it contains no `close()`, and that the deadline is literally the awake-window
constant. 13 notification tests pass.

⚠ **The acoustic half needs an ear, not an instrument** — whether the chime still
sounds right after a suspend/resume cycle is owner-verifiable only. The
measurable half above stands on its own and does not wait on it.

## ⛔⛔ [6.7] THE PRIVACY GUARD SCANS ALL OF HISTORY ON A NEW BRANCH, SO EVERY LANE PUSH FAILS AND TEACHES THE OVERRIDE

**Status:** OPEN

**Re-opened:** closed at `2d81b5a1` on the headline claim, which was then measured
false — but a real residual survives it: the range derivation on a branch with **no
upstream**. Kept open for the residual, not the headline. ⚠ This line is also what
stops `check-queue-resurrection.sh` reporting the entry every run; it was doing so
"ever since, and nobody ran it", which is how a real resurrection would have hidden
behind the noise.

⛔⛔ **THE HEADLINE CLAIM — "EVERY LANE PUSH FAILS" — WAS MEASURED FALSE ON 2026-08-14,
AND THIS ENTRY'S OWN FALSIFIER IS WHAT REFUTED IT.** The falsifier below says *push a
branch and the guard must pass without an override.* A lane did exactly that: **15 commits
pushed first try, clean, `YGG_PRIVACY_ALLOW` never used**, on a branch **with** an upstream.
The guard reported `scanned <range> → …` — **it scans the pushed RANGE, not the tip's
ancestry.**

⚠ **This entry was CLOSED once already** (`2d81b5a1`, *"both fixed and live-proven"*) and
**came back through a lane merge** — `scripts/check-queue-resurrection.sh` has been
reporting it ever since and nobody ran it. ⛔ **It then cost a lane an entire session of
not pushing**, because the block was inherited as fact from this entry and a relayed note,
and nobody ran the one command that refutes it. *An inherited "blocked" is a claim.*

⇒ **NOT deleted here, deliberately:** a real residual defect remains (the range derivation
on a branch with **no** upstream is still worth fixing), and the corrected text lives on
`lane/dev/6.7-resource` (`87394d0a`, *"the guard defect is real, 'every lane push fails' is
not"*), which is unmerged pending the release. **That lane's version supersedes this entry
on merge.** Until then: do not treat this as a reason not to push.

*found 2026-08-13 while pushing a lane branch; affects every cluster in this batch*

`ygg-privacy-guard hook` derives its scan range from the pre-push stdin line:

```python
rng = f"{rsha}..{lsha}" if not rsha.startswith("0000") else lsha
```

**On a new branch `rsha` IS `0000…`**, so the range collapses to `lsha` — the
branch tip — and the guard scans **every commit reachable from it**, i.e. the
whole repository history, rather than the commits actually being pushed.

⇒ Pushing `lane/dev/6.7-resource` (one commit, 247 added lines) was refused for
private terms sitting in **commit messages that are already ancestors of
`origin/main` and have been public for weeks**. Verified: `origin/main..HEAD`
contained exactly one commit, and the guard's own `scan --rev-range
origin/main..HEAD` returned *"✅ no private data found in what is being pushed."*

**Why this is ⛔⛔ and not a nuisance.** The only escape is
`YGG_PRIVACY_ALLOW=1`. A guard that refuses **every** new branch regardless of
content trains every agent to set that variable reflexively, and the override is
a single environment variable that suppresses the whole scan — so the guard
stops working precisely when someone finally does push something private. **A
check that cries wolf on every lane branch is worse than no check**, because it
manufactures the habit that defeats it. All eight clusters in this batch push
lane branches.

**The fix:** for a new branch, ask what the remote does not already have —
`git rev-list <lsha> --not --remotes=origin` — instead of walking the tip's
entire ancestry. Failing closed is right; failing closed on already-published
history is not failing closed, it is failing blind.

**Falsifier:** create a branch with one innocuous commit and push it. The guard
must pass without an override.

⇒ **Separately, and this one is owner-gated:** the terms it caught are real and
they are *already public* in `origin/main` commit messages (a graph name, a bank
hostname). Removing them means rewriting published history, which has been done
once before on a sibling repo and orphaned 34 commits. Not a relay action.
→ `docs/owner-attention.md`.

## ⛔ [6.7] THE GUI SEGFAULTS IN THE HARDWARE-GL DRAW PATH — AND THE STUCK HANDLERS ARE NOT OURS

**Status:** OPEN

*re-measured 2026-08-13 evening; this entry previously conflated two things*

### ⛔ THE STUCK HANDLERS ARE KDE'S, AND THEY ARE RESIDUE

Sixteen `drkonqi-coredump-launcher` processes are resident, **all parented to the
KDE session manager**, ages **5.1 to 11.5 days — and nothing newer than 5.1
days.** They are KDE's crash-reporter UI failing to exit, one per crash, days
ago. **Not a yggterm process, not accumulating, and not ours to fix.** They are a
symptom of our crash COUNT, and they vanish with a session restart.

⇒ **What is ours is the crashing, not the handler.**

### ⭐ A CONFIRMED SIGSEGV TODAY, WITH NO YGGTERM FRAME IN IT

`coredumpctl` on the GUI that died at **19:03:42**, core present, 48.2 MB:

```
#0-1    libEGL_mesa.so.0
#2-13   libgallium-26.1.5
#14     gdk_cairo_draw_from_gl        (libgdk-3)
#15-17  libwebkit2gtk-4.1
#18+    gtk_container_propagate_draw  (libgtk-3)
```

**Every frame is Mesa, GTK or WebKit — none is ours.** The GUI dies inside GTK's
GL→cairo bridge while compositing the webview, on the **hardware** GL path.

⚠ **Correction to a claim made in this session:** that GUI restart was attributed
to another cluster taking the host for a deploy. **It was not a restart — it was
a segfault**, and the core says so. A process that vanishes and comes back is not
evidence of who restarted it.

### What the census does and does not support

Top frame across the 12 most recent `yggterm`/`WebKit` cores:

| top frame | count |
|---|---|
| `libgallium` / `libgdk-3` (the GL path) | **4** |
| `libc` (aborts, not this signature) | 6 |
| unresolved | 2 |

⇒ **The GL cluster is real but it is not all of them.** One backtrace is
confirmed end to end; the rest are a crude top-frame sample and must not be
quoted as though they were full traces.

### ⛔ THE FIX IS A TABLE CHANGE, NOT A FLAG FLIP

`docs/presentation-policy.md` is the SSOT and the standing law is explicit:
**never set `WEBKIT_DISABLE_DMABUF_RENDERER`, `LIBGL_ALWAYS_SOFTWARE` or any
`PRESENTATION_VARS` entry against the owner's running GUI.** So nothing here is
to be "tried" on his machine.

⭐ **The case to answer is already half-written in `optimization-pass.md` §9f/§1a:
the live host runs `YGGTERM_WEBKIT_GL_POLICY=hardware_gl_probed`, and hardware GL
measured NO BETTER than software there.** ⇒ If hardware GL buys nothing
measurable and crashes the GUI in Mesa's draw path, the policy TABLE should carry
a different default for this host — with this core as the measurement in the row.

**Falsifier:** run the sandbox (`scripts/underglass-sandbox.sh`) on both GL arms
under the same webview workload and compare crash counts. If the software arm
crashes at the same rate, the GL path is incidental and the abort cluster is the
real story.

## ⚠ [6.6] GROK BUILD: THE TITLE FIELD IS REAL, AND WE WERE READING THE OTHER ONE

**Status:** OPEN

*The last of the 2026-08-13 intake's declared unknowns. The falsifier ran
2026-08-14 and answered something the entry had not asked.*

### ⭐ ANSWERED — AND `Generated` STAYS, FOR A BETTER REASON

The entry asked whether `session_summary` is a placeholder grok never fills. It
is not. Grok's shipped binary carries a title generator
(`crates/codegen/xai-grok-shell/src/session/summary.rs`, `goal_summarizer.rs`), a
`session_summary_generated` event, and — decisively — a log line for exactly the
state observed: **"session closed before its title was generated"**. The field is
written asynchronously and a short session simply ends first.

⇒ `title_authority` stays **`Generated`**, but the reason changes: not *"the CLI
may never fill this"*, but *"the CLI often has not filled it YET"*. A `Store`
authority would leave those rows nameless. Unchanged conclusion, different fact.

### ⛔ AND THE READER WAS TAKING THE WRONG FIELD — fixed

Grok writes **two**, and its own binary names them together: *"`session_summary`
and `generated_title` — the session summary and its model-generated title"*.
yggterm read the **summary**, so a row would have been named with a paragraph
whenever grok had an actual title for it. The reader now prefers
`generated_title` and keeps the summary as a fallback; empty stays absent.
Pinned by `the_grok_reader_prefers_the_title_over_the_summary`.

### What was measured, and the honest limit

Two sessions on a signed-in fleet host, two turns each: no title/summary events
fired, `session_summary` empty, `generated_title` absent entirely.

⛔ **What is still NOT observed is a POPULATED field.** The generator's existence
is established from the shipped binary, not from a filled `summary.json`, so the
preference order above is implemented on grok's own documentation of its schema
rather than on a sighting. **This entry stays OPEN for that one observation**: a
grok session long enough to generate a title. If `generated_title` appears where
expected, this closes as a fact about grok; if the title lands somewhere else,
the reader needs the real path.

⭐ **The instrument worth reusing:** the answer came from `strings` on the
vendor's own binary, asking whether anything WRITES the field — the same
setter-grep that settled a launch-flag leak the same day. A field's emptiness
says nothing; the presence of a generator and its "closed before generated" log
line says everything.

### ✅ Closed 2026-08-13, both by their own falsifiers

- **The session store is READ.** The gap claimed this needed a login only the
  owner holds. It did not: a fleet host was already signed in, and the layout is
  `~/.grok/sessions/<percent-encoded-cwd>/<uuid>/summary.json`, whose `info.cwd`
  is a plain absolute path and whose `info.id` equals its own directory name. ⇒
  **the bucket name is a second encoding of a value the file already states**, so
  the reader reads the file and never decodes the path. ⚠ The glob targets
  `summary.json` alone; the session directory also holds `chat_history.jsonl`,
  `events.jsonl` and `updates.jsonl`, and globbing those would yield three
  entries for one session.
- **Auto-provisioning works.** Proven on the one fleet host that had no grok at
  all: `@xai-official` is now in the managed npm prefix and `grok --version`
  answers there. ⚠ The 67 MB payload is fetched on first real run, not at
  install, so an empty `~/.grok/downloads` beside a working `--version` is the
  expected intermediate state and not a failed install.

⇒ **The lesson worth keeping: the gap was written as an owner gate and was never
one.** Nobody had asked the hosts. An inherited "blocked" is a claim, not a fact.

## ⛔ THE BINARY ROSTER REPLACED A HARDCODED LIST WITH A NAME PREFIX, AND STRANDS THE SAME WAY

**Status:** OPEN

*Found 2026-08-13 by seat 6.8, on installing an app that is not named `y…`.*

The fleet's binary sync discovers its roster instead of hand-listing it, and its
own comment says why: *a hardcoded roster silently strands every app nobody
remembered to add.* That was the right diagnosis. **The replacement globs
`~/.local/bin/y*`**, so it strands every app whose name does not begin with `y`
— by the same silence, for a whole class rather than for forgotten individuals.

Verified rather than assumed: the shipped editor matches the glob, and a
newly-installed app named without the prefix does not. It was installed by hand
on all three hosts, and **it will drift the moment it is rebuilt on one of
them**, because nothing will carry the new build across.

⇒ **This is a discovery mechanism that finds nothing for a whole category and
reports no error** — the shape this queue already carries four instances of. The
convention is real (the platform's apps are `y…`), but a convention is not a
membership test, and the roster is treating it as one.

**Fix, in preference order:** (1) discover by MANIFEST — an app that has written
`~/.yggterm/apps/<name>.json` has declared itself, which is a statement of
membership rather than a guess from spelling; (2) keep the glob and add an
explicit ALLOW list beside the existing DENY, so an off-convention app is one
line rather than invisible.

⚠ Not fixed here: this is fleet tooling, not this repo, and the owning row
should make the call. Recorded so the next off-convention app does not spend an
evening wondering why its upgrade never reached the other hosts.

## ⭐ THE RENDERER'S LOAD-BEARING FEATURE IS BARE-NAME LINK RESOLUTION — measured, not argued

**Status:** OPEN

*Measured 2026-08-13 by seat 6.8 against a collection in daily use. Structure and
shapes were counted; no note bodies were read and no targets recorded.*

Filed here because **there is no live row for the renderer**, so a finding aimed
at "whoever owns it" routes to nobody. This survives until someone opens that
door.

The link syntaxes a real markdown collection actually uses, as a share of link
volume:

| shape | share |
|---|---|
| **bare `[[name]]`** | **~98%** |
| pathed `[[folder/name]]` | ~1.8% |
| aliased `[[name\|label]]` | ~0.3% |
| heading `[[name#heading]]` | **0 — not once, in thousands of links** |
| embed `![[name]]` | 3 in total |

⇒ **A renderer that cannot resolve a bare name across a whole collection is
useless for this corpus; one that lacks aliases and heading-links loses under
half a percent.** That is a priority ordering derived from use rather than from
taste, and it inverts the order these features would naturally be built in —
heading-links and aliases *look* like core wiki-link functionality and are
respectively never and almost never used.

⚠ **One number must not be over-read.** Cross-collection embedding was the origin
story of the flatten decision, and the embed count is 3. That is consistent with
*the old containment made it impractical* and equally consistent with *nobody
wanted it*. **The count does not distinguish them**, and it should not be quoted
as if it did.

**Falsifier:** count the same shapes on a second independent collection. If bare
names are not dominant there, this is one corpus's habit rather than a property
of the format, and the priority does not transfer.

## ⛔ A CLEAN MERGE PRODUCED A SELF-CONTRADICTING DOCUMENT

**Status:** OPEN

*Hit 2026-08-13 by seat 6.8, landing five lane commits onto a queue 62 commits
ahead.*

Merging main into a lane that had edited **the same region of this file five
times** completed with **no conflicts, no markers and no warning** — and
silently duplicated a 41-line block, keeping both a superseded paragraph and the
text that replaced it. The entry then said *"the private side is done"* in one
place and *"Next: the private side"* forty lines later.

**Every gate passed on the corrupted text.** `check-docs-ssot.sh` reported ok;
the heading count was exactly what it should be (+1 for the one new entry); no
other lane's entry was harmed. The duplication was invisible to every automatic
check because **duplicating a block breaks no rule any of them enforce.**

⇒ This is the merge counterpart of a known law here: *a tool reporting success
is not the same as the content being right.* A merge resolves TEXT, and a
document whose sections are semantically ordered — supersession, "next steps",
status lines — can be textually merged into nonsense.

**What caught it:** reading the merged entry, then a script that hashed every
paragraph over 80 characters and counted repeats. That check found seven
duplicated paragraphs in one entry and, once fixed, zero across the whole file.

⭐ **Cheap enough to be a gate.** A duplicate-paragraph check over this file
costs nothing and would have failed the merge — worth adding to
`check-docs-ssot.sh`, which already owns the question *is this file internally
sound*. Not added here because that script is the docs law's SSOT and changing
it belongs with whoever owns that law, not with the lane that tripped over it.

⚠ ALSO: the first probe that looked like a lost section was a **case-sensitive
grep miss** — the text was there, capitalised. Do not conclude loss from a
failed search until the search itself is controlled; that is the entry below,
arriving during the fix for this one.

## ⛔ A READER THAT FINDS NOTHING LOOKS EXACTLY LIKE A THING THAT HAS NOTHING

**Status:** OPEN

*Three instances 2026-08-13, two of them found independently within the hour by
seats 6.8 and 6.2.*

**The generalisation first, because it is worth more than any of its instances
and it is what a reader should carry away:**

> **An empty result and a broken reader are the same picture.** Nothing errors,
> every count is plausible, and the failure is invisible precisely because the
> system is designed to survive emptiness gracefully.

| the reader | its silence looked like |
|---|---|
| a corpus reader looking for the wrong facts filename | two populated collections holding nothing |
| an editor handed a path it accepted and dropped | a rendering bug — exit 0, empty surface |
| a viewer painting no document body | an app that declared no content |
| **a shadow client that had died** | **a rail stuck on "Loading…" — a slow fetch, then a regression** |

⚠ **That last row was measured 2026-08-13 and nearly published as a false
finding.** A rail that had rendered fully an hour earlier came back reading
*Loading…* on two different corpora. The app was serving its schema correctly
throughout — `curl` proved it — so the obvious reading was a host regression
between 3.0.132 and 3.0.140, and there was even a plausible culprit (a widget
added in between). **The shadow had simply died**, and `--client <name>` answered
*no live client by that name* the moment anything asked. ⇒ **A dead instrument
does not return an error; it returns a plausible picture.** The only thing that
caught it was checking the instrument's own health **in the same run** as the
measurement — and the control that exonerated the new widget (a corpus without
it, stalling identically) was itself taken through the dead instrument and
proved nothing either.

⇒ **When adding any reader, ask what its silence would look like** — and if the
answer is "the same as success on empty input", give it a way to say *I looked
and found nothing* distinct from *there was nothing*. Every entry below is a
consequence of nobody having asked that question.

### The instance this entry was opened for: the shadow does not paint a body

*Measured 2026-08-13 by seat 6.8, with a control, while trying to obey two rules
at once. GUI at 3.0.132.*

The shadow-probe law says probe through the shadow, never the operator's GUI.
The field guide says a visual symptom needs a faithful pixel. **For a document
surface those two rules point at an instrument that cannot answer**: the shadow
paints the surface's BAR and the app's RAIL pane, and leaves the BODY blank.

What was measured, on one shadow, in one sitting:

| surface | declared | painted on the shadow |
|---|---|---|
| an app's rail pane | button, section, three rows | ✅ every row, with live content |
| the document surface's **bar** | two `section` widgets | ✅ and it re-rendered on a route change |
| the document surface's **body** | six `list-row`, then a `markdown` | ⛔ nothing |
| the pilot editor's document body | one multiline `text-input`, 3,625 chars | ⛔ nothing |

⇒ **The control is what makes this a finding rather than a guess.** A new app
rendering an empty body is a new app's bug until the SHIPPED pilot renders an
equally empty body through the identical path — same shadow, same verbs, same
minute. It did. Two independent apps, both declaring body-class widgets with
real content, both blank; and the same shadow painting bar and rail correctly,
which is the positive control that it can paint an app's widgets at all.

**Why it misleads rather than merely limiting.** Every telemetry field agrees the
surface is fine — `has_schema: true`, `stale: false`, `error: null`,
`visible: true` — and the bar visibly updates when the route changes, so the
surface is demonstrably live and refetching. An author following the two laws
correctly is handed a blank frame with a clean bill of health and no reason to
suspect the instrument. The shadow's own documentation says its terminal
viewport was fixed and its screenshots are valid pixel proof for a terminal bug;
nothing says the document body is exempt, so the exemption reads as an app bug.

**Falsifier:** open any document-surface app on a shadow and capture. The body
must contain what the app declared. Until then, treat a blank document body on a
shadow as unproven rather than broken — and note that the honest alternative,
foregrounding the operator's GUI, is forbidden by the law that sent you here, so
this gap has no workaround that does not break a different rule.

### ⚠ A REAL-GUI CAPTURE EXISTS NOW, AND IT ANSWERS A DIFFERENT QUESTION

*Taken 2026-08-13 by seat 6.2 at 3.0.133. The halves are kept apart here on
purpose, because this entry will be read by people who ran neither.*

**Established:** the app plane paints on a real GUI. A freshly launched pilot
editor rendered its entire rail on a faithful frame — toolbar, search field,
view toggle, headings, the empty-state line. So *"a document surface cannot
render on a real GUI"* is refuted, and whatever the shadow is doing, it is not
that.

⛔ **NOT established, and it is exactly the half this entry is about.** That
capture could not reach the LOADED state, so it shows **an empty surface
painting its rail** — not **a populated body painting its content**. The
observation above is a surface with 3,625 declared characters painting nothing.
Those are two different pictures and neither substitutes for the other.

⇒ **Do not read this entry as "the body is proven on a real GUI."** The
shadow-versus-real-GUI comparison is **still open**, and the instrument that
would close it is a surface that will actually load a document.

⚠ **Why that instrument is currently unavailable is another lane's finding, not
this seat's measurement:** the editor accepts a path argument, reports success,
and does not load it — filed separately by the seat that hit it. It is named
here only because it is what blocks the decisive test, and it is itself the
second row of this entry's own table. Two seats found the same shape in the same
hour on different planes, which is the argument for the headline.

**A likely cause, and the experiment that was deliberately NOT run.** A shadow
declares itself with a role that the daemon's gate holds to read-only — it never
sends terminal-ensure, resize or focus, and paints from the read stream. A
document surface is not a PTY, but it is plausible the body sits behind the same
gate. The clean test is a full client, not a shadow, in its own compositor.

⛔ **It was not run, on purpose.** A second full client on the operator's machine
contends for session ownership, and the one-viewer-per-session assumption is
precisely the thing the constitution names as unfixed — a spare full client
yanking the operator's live session is a known shape here, not a hypothetical.
The diagnostic is not worth that risk from a lane that only needed to know
whether its own app was broken, and the control already answered that. Recorded
so the next reader does not have to re-derive the risk before deciding, and so
that "nobody tried it" is not mistaken for "it did not work."

## ⭐⭐ [6.8] THE KASTEN APPS ARE WAITING TO BE BUILT

**Status:** OPEN

*requested 2026-08-13*

**Where it stands 2026-08-13, after the first build session:** the engine exists,
is tested, is pushed, and its surface is live-proven as far as the instruments
allow — see the progress note at the end of this entry.

Two related surfaces, requested as their own relay:

1. **`ztlkasten`** — the journalling rule-set repo, which already has a design
   and a door in the campaign memory.
2. **A kasten-style overview for each private corpus** — five of them, one per
   domain, and the request says *same shape, one per corpus*.

⇒ These are the softwares the private campaigns are waiting on, and they are
listed here so the queue reflects that the wait is real work and not a
dependency on someone else.

⛔ **The private corpora's surfaces stay in private repos.** Anything that
resolves to a real path, person or case is a privacy defect in a public repo —
this is the guard that has already failed once by scanning tracked files only.

⚠ **This entry leaked the roster it forbids, and the leak is fixed above.** The
original wording listed the five corpora by their short names in a sentence that
appends the common suffix for the reader. The wordlist guard holds each full
name and would have caught them; it cannot catch a roster written in pieces, so
nothing fired. **A count and a shape carry the whole engineering fact** — that
there are five consumers of one contract — and the roster carried only identity.
Never pushed: caught on the lane branch. Routed to the guard's owner as a dream.

### The scope, written before any code so it can be corrected cheaply

*written 2026-08-13 by seat 6.8, which is this entry's owner*

**The call that shapes everything else: this is ONE app with N configurations,
not two apps and not six.** The request already said *same shape, one per
corpus*; the measurement agrees. Both consumers are a corpus of typed nodes in
folders, addressed by slug, cross-linked, with prose attached — the journalling
vault addresses its nodes as markdown files whose grouping vocabulary is *also*
files (a tag is a note), and a private corpus addresses its nodes as a
structured-facts file plus an optional prose file under a folder that names the
kind. **Same shape, two vocabularies.** So the vocabulary becomes data.

⇒ **`kasten.toml`, a manifest at a corpus root, is the ONE contract** between the
engine and any corpus. It declares the collections and what each is called, how a
node's title, status and date are read, and which sub-collections may never enter
a publication path (§5 of the app's own spec already requires that property to be
declared and enforceable rather than an accident of layout). The engine ships in
the public repo and never learns a corpus name; each private corpus commits a
manifest of a few lines **in its own private repo**. That is what keeps this
entry's own prohibition structurally true rather than a thing an author has to
remember.

**Tier A, by the app-architecture spec's own question** — *who must paint the
pixels, and why?* Collections, rows, prose, counts and a search box are what the
host already draws. The app therefore ships **no UI code at all**: a headless
daemon plus a thin CLI, an OSC declare, a loopback control endpoint serving the
widget schema, rendered on the document surface. Chrome is the schema either way;
here the content is too.

**What gets built, in this order:**

1. This scope. ✅
2. The manifest schema, and a synthetic corpus fixture with **invented** names —
   the fixture is the classic leak because a realistic string feels like good
   practice, so the fixture is fiction end to end.
3. The engine: read the manifest, index the corpus, emit the schema. Proven
   against the fixture, in the public repo, with no private corpus in reach.
4. The scaffolding — ensure-daemon, OSC emitter, manifest writer, loopback
   server. The pilot app already carries these ~200 lines and the platform's
   migration order wants them extracted **once a second consumer exists**. This
   lane is that consumer. ⚠ Copy first and file the extraction; doing it inline
   would put a platform refactor inside an app's first commit.
5. Live proof on the GUI host: a faithful screenshot of the document surface
   showing the fictional corpus, per the visual-symptom law.
6. Only then the private side: one manifest per corpus that fits, committed
   there, never here.

**What does NOT get built, stated so the boundary is correctable now and not
after a week:**

- **No editor.** Reading and capture only. Editing a buffer is another app's job
  and duplicating it would be a second source of truth for the same gesture.
- **No collaboration or arbiter.** Owner-settled: that primitive belongs to the
  host, and it is the same two-live-viewers problem the constitution already
  names as this project's highest-value work.
- **No markdown handling of its own.** Owner-settled: the renderer organ is the
  SSOT, and an app that grows its own breaks the property that makes the pipeline
  worth having.
- **No boards, calendars or dashboards first.** They demo well and serve neither
  hot path. The design value is flow, priced in decisions and keystrokes on
  *capture a thought* and *find a thing again*; a feature that cannot pay on both
  is a feature for some other program.
- **No new host widget kinds.** If the content needs one, it is filed as a
  vocabulary gap for the host to admit, not drawn inside a native surface. A
  graph view is the obvious candidate and it is explicitly **not** opened here.
- **No writes into any private corpus.** Read-only in the first pass. Each corpus
  already has a CLI that owns its mutations, and a second writer is a second
  source of truth for the same records.

**The honest exception, found by measuring rather than assuming: one of the five
is not a corpus.** Four hold hundreds of node files between them; the fifth holds
three prose files, a database and a tree of pipeline logs and queues. It is a
**pipeline**, and an overview of it is a job-and-run view, which is a different
program. Building the node overview and calling it five would be a false claim
about coverage, so this lane delivers four and says which one it did not.

**Falsifier for the whole scope:** the same binary, unchanged, renders two
corpora with disjoint folder vocabularies from their manifests alone — and a
`grep` of the public repo for any private corpus name, path or node title returns
nothing, with a positive control proving the grep can find a planted one.

### Progress, 2026-08-13 — the engine, and what its proof does and does not cover

**Done, and pushed to the app repo.** A single binary; a corpus manifest that
carries every corpus's vocabulary; two fixture corpora invented end to end with
no collection id in common; 32 tests. The scope's build order is at step 5.

**The falsifier's first half is met, twice over.** One binary, checked by hash,
renders both fixtures from their manifests alone — and the test that asserts it
fails loudly if the two fixtures ever grow a shared collection id, because then
it would prove nothing. The second half is met with both controls: the app
repo's privacy checker reports clean, reports DIRTY on a planted leak, and
reports clean again once the plant is removed.

⚠ **The checker was reporting clean for the wrong reason until this session.**
It scans untracked files deliberately, and the repo had no `.gitignore` because
it had no code — so the first build buried every real finding under hundreds of
dependency paths, which is the "cries wolf, gets switched off" failure its own
comments warn about. Fixed there; worth knowing wherever else that checker is
ported, because the defect arrives with the first line of code and not before.

**Live-proven on the GUI host** — built there from the pushed commit, run in a
backgrounded session, opened on a shadow so no operator viewport moved:

- the app declares as a contribution with both panes, one of them a viewport
  pane, and the host reaches its control endpoint;
- the **rail pane renders the fiction corpus** — the corpus name, an Overview
  button, and every collection with its node count;
- the **document bar renders and re-renders**: a route change posted straight to
  the app's own endpoint moved the bar's headings, which proves the refetch path
  end to end without touching the GUI at all.

⛔ **NOT proven, and not claimed: the document BODY.** It rendered blank — and so
did the shipped pilot editor's, through the identical path in the same sitting.
That control is why this is filed as its own entry above rather than carried
here as a kasten defect. The body remains unproven either way until someone
captures a document surface on a real GUI.

**The private side is done, and it is four manifests, not four programs.**
Rather than hand-writing one per corpus, the engine grew an `init` verb that
proposes a manifest by surveying what a corpus actually contains — the same
chore, done the same way every time, instead of assembled from primitives once
per corpus and got subtly wrong on the third. It proposes to stdout, refuses to
overwrite an existing manifest, and emits no source it did not observe. Each
manifest was then edited for the two things no generator can see from disk:
what a corpus is CALLED, and which of its collections may never be published.

Four corpora now carry one, each committed in its own repository and none of
them named here. Three pushed; the fourth's repository is deliberately
remote-less and its commit is local by design, which is a standing decision and
not an omission.

**Running it against real corpora paid for itself twice, and both were silent
failures rather than errors:**

1. **A directory node names its facts file after the node KIND**, not
   `index.toml`. Two populated collections were reported as holding nothing.
   Fixed by declaring it. The obvious alternative — a hard-coded list of likely
   filenames — is the trap this whole design exists to avoid: it works on the
   corpus it was written against and reads zero nodes on the next, which looks
   exactly like an empty collection.
2. **A collection rooted at the corpus itself swallowed the corpus's own index
   files as nodes.** They parse, so the count was merely wrong and nothing
   complained. A declared entry file now also means *a node is a directory*,
   which is what it was always saying.

**The fifth is confirmed not a corpus, by measurement rather than by repeating
this entry's own earlier claim.** Pointed at it, `init` finds a single
collection of two prose files and reports its other directories as holding
nothing — they are queues, logs and a work area. It is a pipeline; a
job-and-run view is a different program and this lane does not pretend to
deliver one.

### Capture is built, and live-proven end to end

*2026-08-13, later the same session.*

The overview served retrieval only, which made the app a reader rather than a
journal. **Both hot paths now exist.** A thought lands in today's dated entry in
a declared collection with **one command and no decisions** — no title, no
folder, no tag. The entry opens with its own date as a heading, which is what a
note collection reads its title from, so the writer never names one; each
thought gets a time marker they never type, which costs nothing on the hot path
and is the only way to recover the order of a day's thinking later.

**One declaration, and everything else is derived rather than restated.** The
capture target names a collection; the file name, the heading and the ordering
all come from what that collection already says about itself. Three
misconfigurations are refused at manifest LOAD rather than at capture time,
because a corpus set up to file thoughts somewhere that will never show them
would fail silently and the writer would find out late or never.

**Live proof on the GUI host, before and after, on one rail:**

- the capture box renders at the top of the rail, above navigation, identical on
  every route — a thought arriving three collections deep must not require the
  reader to go anywhere;
- the rail read **Journal 3**; the capture action was posted exactly as the rail
  posts it; the rail read **Journal 4**, and the day's entry was on disk with
  its date heading and time marker.

That is the surface path, not just the CLI — box, action, write, re-index,
and the count the writer sees change.

⛔ **Still not claimed: the document BODY**, for the reason in the entry above.
The rail is what the available instrument can prove, and it is what is claimed.

**And it is now invocable, which it was not.** Everything above was true of a
binary living in a build tree, reachable by nobody: `command -v` found nothing on
any host. **A journal that cannot be invoked is not a journal**, so the app is
installed on all three fleet hosts, one hash across all of them, and proven from
a clean login shell on each — a corpus created on the spot, a thought captured
into it, the entry read straight back. That is the step between *built* and
*usable*, and it is easy to skip because every earlier proof passes without it.

⚠ **It will not stay in sync by itself** — see the roster entry above. A rebuild
on one host reaches only that host until the roster question is settled.

## ⛔ [6.4] `server app start-page` IS A NAVIGATION VERB SHELVED AMONG THE READ VERBS

**Status:** OPEN

*found 2026-08-13 while writing this batch*

`server app`'s verb list reads `clients`, `desktop-identity`, `state`, `rows`,
`start-page` — four reads and one that is not. `start-page` **navigates the live
GUI to the start page**; it answers `{"accepted": true, "selected_paths": [...]}`,
which reads like a query result and is in fact a report of what it just did.

**It disturbed the operator's view during this session's recon**, called as a
read. The standing directive is not to disturb the running viewport, and the
verb surface made that directive impossible to follow from the name alone.

⇒ **Fix by renaming, not by documenting.** `show-start-page` (or a `navigate`
noun that groups it with `open`) puts it in the family it belongs to. This is
the same class as every other entry in the field guide's instrument table: *the
verb answers a different question than its name suggests.*

✅ **RE-CONFIRMED BY MEASUREMENT, 2026-08-13 at 3.0.139**, during a sweep for
entries filed off a single field. It navigates:

```
PRE-CALL:  Terminal | local://<uuid>      ← a live row, the operator's view
AFTER:     Rendered | None                ← the start page
```

⭐ **And the sweep caught the reverse error in its own auditor, which is the part
worth keeping.** The sweeping cluster had recorded in its own notes that this
verb *reads and does not navigate* — inferred from one call made while the GUI
was **already on the start page**, where a navigating verb and a reading verb are
indistinguishable. A single observation taken in the one state that cannot
discriminate became a durable "instrument fact" and was nearly used to retract a
correct entry. ⇒ **Test a state-changing verb from a state it would have to
change**, and the viewport was restored immediately afterwards.


## ⛔ `deploy-fleet.sh` CANNOT RECOGNISE AN ALIAS FOR THE HOST IT IS RUNNING ON

**Status:** OPEN

*Reported 2026-08-13 by cluster 6.2, which lost a deploy to it.*

The script takes `--hosts "a b c"` and ssh-es to each. When one of those names is
an **alias for the machine the script is already running on**, the ssh fails and
every copy in that run fails with it — because `hostname -s` returns the machine's
own name, not the alias the fleet addresses it by.

⇒ A host can be reachable by a name the script cannot recognise as *itself*. The
fix is a self-alias check: resolve each target and, when it resolves to this
machine, copy locally instead of dialling out.

**Falsifier:** run the deploy on each fleet member naming every member including
itself. All copies must land, on every host, with no ssh attempt to self.

## ⛔ `server app screenshot` VOUCHES FOR THE PIXELS AND SAYS NOTHING ABOUT THE SUBJECT

**Status:** OPEN

*Reported 2026-08-13 by cluster 6.2; independently hit by the orchestrator the
same day, which received a photograph of a text editor while trying to capture
the app.*

On a **non-terminal** view the verb returns `capture_faithful: true` while
photographing **whatever window currently holds focus**. The flag is true about
the pixels and silent about the subject, so a caller that checks it — which is
exactly what the field guide instructs — is told the frame is trustworthy when it
is a picture of something else entirely.

⇒ This is the instrument family's signature shape: *the field answers a different
question than its name suggests.* `capture_faithful` answers "were these pixels
composited honestly", not "is this the thing you addressed".

**Fix, in preference order:** (1) `--pid` implies window targeting, so the
addressed client is what is photographed; (2) failing that, return
`faithful: false` — or a separate `subject_verified` — whenever the captured
window is not the addressed client.

⚠ Until then, a screenshot of a non-terminal view proves nothing unless the
window was foregrounded first, and foregrounding the operator's GUI is its own
prohibition.

## ⛔⛔ A REAPED ROW COMES BACK HOLDING A LIVE AGENT, INTO A WORKTREE ITS SUCCESSOR IS EDITING

**Status:** OPEN

*Measured 2026-08-13 by the orchestrator, on its own reap.*

A row was retired with `session remove`, which answered `row_still_listed: false`,
`verified: true`, `live_processes: []` — the clean verdict. Roughly an hour later
the row was **back in the sidebar**, and it was not a stale entry: it held a live
process pair, `yggterm server remote resume-cc <uuid> --require-existing` and a
running agent CLI, frozen at the sentence it had been writing when it was reaped.

⛔ **The danger is not the row, it is the tree.** The resurrected row and its own
successor were both live in the **same git worktree** — two agents editing one
checkout, which is precisely the clobber that separate worktrees exist to prevent.
It went unnoticed because every liveness instrument reported the fleet healthy:
the successor was working, the orchestrator was working, and nothing anywhere asks
*"is this row supposed to exist?"*

**The likely path in**, and it needs confirming rather than assuming: a handover
killed several rows (tracked separately as agent rows dying across a handover),
and a `sessions restore` / `app open` recovery re-opened the **tombstoned
predecessor** rather than the live successor. `sessions restore` is already known
to refuse rows as `declined_closed` because the tombstone plane cannot tell a
deletion from a GUI death — this is the same seam, failing in the opposite
direction: it cannot tell a deletion from a *recoverable* row either.

**Second defect, in the same reap:** the verdict was wrong twice, in opposite
directions. The first removal answered `verified: true, live_processes: []` while
the CLI was alive; the second answered `verified: false, live_processes: []` while
a `terminate-cc` was still running and did eventually succeed. ⇒ **`live_processes`
was empty in both the false-positive and the false-negative case**, so it is not
carrying the signal its name promises, and a caller cannot use it to decide
anything.

**Falsifier:** reap a row, then attempt every recovery path the GUI offers
(`sessions restore`, `app open`, a daemon handover). None may produce a live
process for a tombstoned session. And `session remove` must not answer
`verified: true` while a process it named survives.

⭐ **Detection shipped meanwhile**, in `ygg-monitor.py`'s seat audit: a live agent
process whose session is in the orchestrator's number space but subscribed to
nothing is now reported every tick. That is what caught this one.

## ⛔ `server app state` TIMES OUT AT 15 s WHILE `app clients` ANSWERS INSTANTLY

**Status:** OPEN

*Flagged 2026-08-13 by cluster 6.7, which did not chase it — it was not their lane.*

On the desktop host, `yggterm-headless server app state` hit its 15 s timeout while
`server app clients` answered immediately against the same GUI. So the transport
and the client routing are fine; something in composing `state` specifically is
slow or blocking.

⚠ **`state` is one of the four verbs an agent reaches for first**, and a 15 s
timeout on it reads as "the GUI is wedged" to anyone who does not also try
`clients`. That misreading is expensive: it is the shape that starts a false
investigation into a healthy app.

**Where to look:** whatever `state` gathers that `clients` does not — most likely a
per-session walk that grows with row count, or a lock held by another request.
Note the daemon serves one request at a time and a hot-restart request holds it
for ~11 s, which is already tracked and would produce exactly this.

**Falsifier:** time both verbs against the same GUI at several row counts. `state`
must not diverge from `clients` by more than the work it genuinely does more of.

## ⛔⛔ EVERY VERSION BUMP ORPHANS ITS IN-FLIGHT CLI CALLERS, AND THEIR STDERR LANDS IN AN AGENT'S COMPOSER

**Status:** OPEN

*Observed by a downstream campaign on two of its rows, 2026-08-13. Root cause confirmed in code and
against the live filesystem before filing.*

**The symptom.** Two `claude-code` rows, both mid-turn, each carrying this **inside the composer
box** — not in scrollback:

```
Error: connecting to <YGGTERM_HOME>/server-3-0-122.sock
Caused by: No such file or directory (os error 2)
```

⭐ **The two rows named DIFFERENT versions (122 and 126), and that is the whole tell.** The daemon
socket name is version-pinned, so a CLI caller resolves a path that exists only while that exact
daemon version is listening.

⛔ **THE CONSEQUENCE IS A REACHABILITY FAILURE, NOT A COSMETIC ONE.** A row whose composer holds
that text is **not addressable by `terminal submit`** — it answers `composer_shown:false`, *"the row
is mid-output, in a menu, or is not an agent CLI"*. Messages to both rows were refused for **over
forty minutes across ~40 attempts each** while both were alive and working normally. ⇒ **From the
outside, a poisoned composer is indistinguishable from a busy row.** And since a deploy campaign
generates this condition, **a deploy can make unreachable the very rows needed to coordinate a
deploy.**

### ROOT CAUSE — the back-alias mechanism can only preserve a version whose file still exists

The design already intends old callers to keep working: `refresh_legacy_server_socket_aliases`
(`crates/yggterm-server/src/daemon.rs:434`) re-points legacy version sockets at the running daemon
on every bind. It draws candidates from **two** sources
(`versioned_server_socket_alias_candidates`, same file, line 384):

1. **files already present in `$YGGTERM_HOME`** that parse as a versioned server socket name;
2. **scope directories under `$YGGTERM_HOME/client-instances/`.**

⇒ **Source 1 is self-erasing.** A retiring daemon's socket file goes away, and once it is gone the
version is no longer a candidate, so the *next* daemon never aliases it. Source 2 is the only
durable one — and it is not being populated for current versions.

**Measured on a live host, and the correlation is exact:**

```
client-instances scope dirs      24   (a historical set; the newest is 3-0-80)
server-3-0-80.sock               SYMLINK -> server-3-0-128.sock   ← has a scope dir, so it was aliased
server-3-0-118 … 3-0-127.sock    ABSENT, every one, no alias      ← no scope dir, file already gone
server-3-0-128.sock              real socket (the live daemon)
```

⇒ A caller pinned to 3.0.80 still resolves **today**. A caller pinned to any version from 3.0.118
onward is orphaned the moment its daemon retires. The mechanism is faithfully aliasing a museum
while every modern version falls through it.

### ⛔⛔ AND SOURCE 2 IS WRITE-DEAD: NOTHING HAS POPULATED IT FOR ~48 VERSIONS

*Question closed 2026-08-13 by the reporting campaign, verified here before the ranking was changed.*

**No production code writes a `client-instances/` scope dir.** Of 69 references across `crates/`
and `apps/`, every one is a read, a scan, or a path join. The **only** `create_dir_all` on that path
in the tree is `crates/yggterm-server/src/daemon.rs:25296` — inside
`#[test] fn versioned_server_socket_alias_candidates_include_client_instance_versions()`, seeding a
fixture.

⇒ **The 24 scope dirs on disk are residue from a writer that no longer exists**, and the newest
being `3-0-80` dates its disappearance. ⛔ **So both inputs are dead in different ways: source 1
erases itself, source 2 is never written.** The mechanism is not degraded, it is **inert — and it
has been inert for roughly forty-eight versions while appearing to work**, because the residue kept
answering for exactly the ancient versions nobody runs.

### Fixes — and the ranking below was INVERTED once the above was known

1. ⛔⛔ **ALREADY BUILT — DO NOT BUILD IT AGAIN. Measured 2026-08-13, and the ranking below was
   written without checking.** "A caller that cannot reach its pinned socket falls back to the
   current daemon" is `resolve_client_daemon_endpoint`, and it works **in both version
   directions**. Reproduced against real binaries in an isolated `YGGTERM_HOME`, with the pinned
   socket not merely dead but NON-EXISTENT:

       daemon 3.0.132 live, no 3-0-128 socket at all   → the 3.0.128 CLI's `server status` answers,
                                                          and its `server attach` gets a session the
                                                          3.0.132 daemon owns (`local://…` in its
                                                          `owned_terminal_session_keys`)
       daemon 3.0.128 live, no 3-0-132 socket at all   → the 3.0.132 CLI answers and attaches too,
                                                          reporting the mismatch rather than failing
       both directions                                  → NOTHING on stderr, so neither can be what
                                                          lands in an agent's composer

   ⇒ **The ENOENT could not be reproduced from a CLI in either direction.** What remains is the
   state the fallback is *correct* to fail in: **no reachable daemon at ANY version**, which is
   exactly where the GUI host sat for 5.5 hours — the newest daemon twelve versions behind every
   client, and no successor because nothing on the host could drain the swap queue. ⇒ **The cure
   was a daemon at the current version EXISTING, not a cleverer client**, so the primary fix is
   the queue-consumer one (§"A QUEUE WHOSE CONSUMER CAN BE OLDER THAN ITS PRODUCER" in the
   hot-restart entry), and this line is closed.
   ⚠ **Still unexplained, and it is the honest remainder:** whether any non-CLI caller bypasses
   the resolver. The GUI's recovery walk has its own fallback
   (`runtime_status_can_serve_current_app`, which deliberately refuses a daemon BEHIND the client
   so an owed deploy is not hidden). **Reproduce before building anything here.**
   ⚠ The old caveat still applies to whatever is built next: **"same host, same home" must be
   CHECKED, not assumed** — installs across `~/.local/bin` and `~/.yggterm/bin` have been observed
   disagreeing, and a fallback that attached a caller to a daemon owning none of its sessions
   would be **worse than the honest error, because it would succeed.**
2. **SECONDARY: make the alias set durable** (a ledger of bound versions, or a writer for
   `client-instances/`). ⛔ **This was ranked first and should not be**: it means reinstating the
   component whose silent disappearance caused this, then depending on it again. **The alias table
   is a CACHE, and repairing a cache leaves you depending on the cache** — one whose writer vanished
   unnoticed for forty-eight versions. With fix 1 in place this is a latency nicety and an inert
   table stops being an outage.
   ⭐ **PARTLY LANDED, 3.0.132 — a retiring daemon bequeaths its own name.** Not a ledger and not
   a sweep: `alias_own_socket_to_successor` runs on both handover exits, immediately before the
   process releases its socket, and symlinks this version's socket at the successor's. It needs no
   durable table because **it is the one instant when both names are known at once** — the
   successor is bound and answering, the predecessor is about to unlink — and a pass run at daemon
   START structurally cannot know it.
   ⚠ **The gap it closes is the one a gap-filling pass cannot: its own predecessor.** A live socket
   is invisible to a pass that only fills gaps, so **every handover orphaned exactly one version —
   whichever was serving.** Measured twice in one evening on two hosts under two different numbers,
   each time naming the version that had just retired; a manual alias sweep therefore re-broke
   itself once per deploy. Traced as `retiring_daemon_aliased_own_socket` with the successor's
   socket and an `aliased` flag, so a handover that could not bequeath its name says so.
   ⭐⭐ **LIVE-PROVEN IN THE WILD at the very next handover (3.0.132 → 3.0.133), on TWO hosts:**

       retiring_daemon_aliased_own_socket {aliased: true, server_version: "3.0.132",
           successor_version: "3.0.133", successor_socket: ".../server-3-0-133.sock"}

   and on both hosts `server-3-0-132.sock` is now a SYMLINK to `server-3-0-133.sock`, with
   `server status --endpoint .../server-3-0-132.sock` answering **3.0.133**. ⇒ a client pinned to
   the version that just retired reaches a live daemon instead of an ENOENT. **Survived 535 s and
   counting.** Contrast the generation before it, which had no bequest: after 3.0.131 → 3.0.132 the
   131 name went ABSENT and had to be re-aliased by hand.
   ⚠ **This also FALSIFIES the sweeper hypothesis in its strong form, and the earlier measurement
   stands unexplained rather than being quietly dropped.** A HAND-MADE alias at the just-retired
   version was deleted three times within seconds-to-100 s, while aliases created in the same
   command at three other versions survived; the DAEMON-WRITTEN one at the same kind of path
   survives fine. So whatever deleted the manual symlinks does not delete these, and the guess that
   `classify_socket_entry` condemns a name on a stale dead sighting is **not confirmed and no
   longer load-bearing**. It is left recorded because an unexplained deletion is worth more written
   down than forgotten.
   ⇒ Still owed here: fix 1, which deletes the class rather than the instance; and 6.7's line for
   whatever sweep remains — **enumerate an alias set from the versions that EXIST, never from the
   files that happen to REMAIN.**
3. ⛔ **INDEPENDENT, DO IT REGARDLESS: a CLI's stderr must never reach an agent row's PTY.** It is
   the blast-radius fix — the difference between *"a call failed"* and *"a row became unreachable
   and looked busy for forty minutes"* — and it is orthogonal to whichever of 1 or 2 lands.
4. ⚠ **Ruled out already, so nobody repeats it:** the claim script's title watcher is innocent (it
   is already fully redirected), and the booter's run path captures output. The writer is elsewhere.

⚠ **Separately, and it is why this sat unnoticed: there is NO SAFE COMPOSER CLEAR today.** A
kill-line ate a person's half-typed sentence when used for exactly this repair, and Escape
interrupts a live turn. ⇒ Until the owner's yank-and-restore ruling is built (see the `terminal
send` entry), **a poisoned row is reachable only by dropping a file it reads** — and the sender owns
removing that file once consumption is confirmed.

⚖ **Relation to the constitution.** *"Other agents' sessions survive our restarts"* is the standing
guarantee, and this is a live counter-example: our own deploys are degrading other agents' rows.
⇒ It belongs with the hot-restart/daemon-lifecycle work, not with the deploy-identity work.


✅ **THE BEQUEST HOLDS — measured 2026-08-13 across at least eight version moves,
which is the observation this entry was waiting for.** A retiring daemon
re-points its own version name at its successor (shipped 3.0.132); the open
question was whether that survives the NEXT bump, and it did. On the GUI host,
after 3.0.133 → 3.0.141:

```
server-3-0-133.sock → answers: server_version 3.0.141, build a0c782b013d5
server-3-0-137.sock → answers: server_version 3.0.141, build a0c782b013d5
server-3-0-134.sock → no socket        server-3-0-136.sock → no socket
```

⇒ A CLI pinned to 3.0.133 still resolves a live path, and it reaches the CURRENT
daemon. Both controls are in the same run: names that answer, and versions that
never ran a daemon here and correctly have no socket, so the probe is not
returning a constant.

⚠ **What this does NOT close.** The bequest is performed BY a retiring daemon, so
it can only cover a handover the daemon lives through. A daemon that is killed,
crashes, or is evicted leaves no successor pointer, and the poisoned-composer
symptom would return exactly there — which is the case worth testing next, and
it is not this measurement.

⭐ **Read the daemon's BUILD, not its version, when checking this.** Both surviving
names answer `3.0.141`, and that string alone cannot say whether one is a stale
listener; the build commit is what makes them provably the same process.

**Falsifier:** after a bump, a socket named for a version whose daemon retired
gracefully fails to connect.
## ⛔⛔ NOTHING PREVENTS A LOCAL TAG FROM REPUBLISHING A PRE-SCRUB LINEAGE

**Status:** OPEN

*Found by the leaks cluster 2026-08-13 and confirmed independently before filing. The tags
themselves have since been corrected on every host; what is open is that nothing stops it
recurring, and no check would notice.*

Two annotated tags in a working clone pointed at the pre-rewrite lineage and were **the only
thing keeping ~1900 commits of pre-scrub history alive in that clone**. None of those commits
was reachable from any origin ref, so every branch-level check read clean.

⛔ **WHY THIS IS WORSE THAN THE EQUIVALENT PROBLEM ON A BRANCH.** A stale branch needs someone to
name it: `git push <branch>`. **A tag rides along.** `git push --tags` and `git push --follow-tags`
send these without anyone deciding to, and either one **republishes the pre-scrub lineage** —
re-leaking, in one habitual command, exactly what a history rewrite was run to remove, *after*
that rewrite has reported success.

⇒ **The re-leak needs no decision. It needs only a habit.**

**What is still open:**

- **No guard.** A fresh clone, a restored backup, or the next rewrite re-arms this, and nothing
  in the ship path checks for it. The check is cheap: commits reachable from local tags and from
  no origin ref must be **0**.
- ⛔ **A scrub's file scope must cover vendored trees IF they are tracked.** Verified for this
  repo — its harness `node_modules` is gitignored with 0 tracked files, so it was correctly out
  of scope — but *"it is third-party"* is not the same answer as *"it is not tracked"*, and a
  private term inside a committed `vendor/` directory publishes like any other. One `git ls-files`
  settles it.

**Fix:** `git fetch --tags --force` on every host, as part of any post-rewrite reset step —
resetting branches while leaving tags on the old lineage leaves the hazard fully armed and the
checkout looking clean. Then confirm per host rather than assuming a sweep covered them.

### ⚠ THE INSTRUMENT NOTE, because this was nearly dismissed as a false alarm

An initial check compared `for-each-ref %(objectname)` against `ls-remote`'s non-`^{}` line and
reported **0 differing tags of 2** — and that zero was a **false negative from a join that matched
no keys at all**, not a finding. The verification could not have expressed a difference.

⇒ **Annotated tags carry two SHAs** — the tag object, and the commit it dereferences to — and a
comparison that mixes the two levels answers confidently and wrongly in either direction.
**Compare `refs/tags/X^{}` against `ls-remote refs/tags/X^{}`**, and run a positive control before
trusting a zero.

⛔ **AND DO NOT WRITE A PRE-SCRUB SHA INTO A PUBLIC FILE.** A force-push revokes nothing: those
commits stay fetchable from the forge by hash. A table of before/after SHAs published here would
hand any reader a direct handle to the content the rewrite removed — the leak re-opening through
the document that reports it closed. Describe the shape; never publish the hashes.

## ⛔ `session remove` CAN REPLY WITH TWO CONCATENATED JSON OBJECTS

**Status:** OPEN

*Reported by a relay 2026-08-13 while reaping a row it had just created.*

The reply was **not parseable as a single JSON document** — two objects run together in one
response body. A caller doing the ordinary thing (`json.load`) gets an exception, and a caller
doing the careless thing (`json.loads(out[out.find("{"):])`) silently reads only the first object
and never learns a second existed.

⇒ The relay could not read the reply at all, and fell back to instruments that cannot lie: the row
absent from `server app rows`, no process matching the uuid, and **no transcript file ever
written**. That is the correct response, and it is also the second-best outcome — **a verb whose
reply cannot be parsed is a verb that reports nothing**, and this one is already in the family of
verbs that report the request rather than the effect.

⛔ **AND THE SUPERVISION PLANE ITSELF USES THE UNSAFE IDIOM.** `ygg-monitor.py`'s `ygg()` helper
parses every reply with `json.loads(out[out.find("{"):])`, which on a two-object reply reads the
first and discards the second **without raising**. ⇒ The watchdog would read a truncated answer
from this verb and carry on confidently. Fix the caller as well as the verb; the caller is ours
and it is one line.

**Fix:** one response, one JSON document. ⚠ Until then, callers must not assume `find("{")` plus a
single parse is safe on this verb — that idiom reads the first object and discards the rest
without error, which is worse than failing.


### ⚠ NOT REFUTED, BUT BOUNDED — 2026-08-13 sweep

Seven `session remove` calls across 3.0.133–3.0.146, every one parsed by a plain
`json.load` with no exception, and every one carried a `verified` that matched
reality (the row really left the census, `live_processes: []`).

⛔ **That is not a refutation and must not be read as one.** The entry says the
verb **CAN** answer this way; an intermittent fault is not disproved by clean
samples, and seven is a small number. What the run does is **bound the rate**:
this is not the ordinary reply shape, so a caller hitting it is hitting something
conditional — most likely a second write racing the first, which is where a
re-test should look rather than at the happy path I just exercised seven times.

⚠ **And the unsafe idiom the entry names is still the one to fix regardless of
frequency.** `json.loads(out[out.find("{"):])` reads the first object and never
learns a second existed, so a caller using it cannot even detect the fault it is
victim to. That half needs no reproduction to justify.
## ⛔⛔ A SCRUB'S VERIFIER ASKS WHETHER THE STRING IS GONE, NEVER WHETHER THE REPLACEMENT IS VALID

**Status:** OPEN

*Found while authoring a hostname scrub, 2026-08-13, before it ran.*

The private name appeared **890 times across 34 `.rs` files**, and not only in prose — it sat
**inside identifiers**: `<name>_live`, `<name>Payload`, and a test function named
`terminal_host_problem_rejects_<name>_sparse_prompt_after_update`.

⛔ **The privacy guard's own suggested replacement contains a HYPHEN.** Applying it would have
produced `gui-host_live` and `gui-hostPayload` — **not valid Rust** — across 34 files. ⇒ **The tool
that flags the leak proposes the fix that breaks the build.**

⚠ **And every count-based check would have passed.** Commit parity, ref parity, subject parity and
the residual term scan all ask *"is the string gone?"*. **None of them asks whether what replaced
it is legal.** The failure appears only at compile time, which no orphan proof reaches.

**The law, which is the existing "never replace an ordinary word" rule turned around to face the
other side:**

> **A replacement must be legal in every syntactic position the original occupies. If the original
> is ever an identifier fragment, the replacement must be alphanumeric.**

### ⭐ COUNT THE CONTEXTS, NOT THE FILES

> **Enumerate the distinct syntactic CONTEXTS a term appears in before choosing the replacement.
> The count of FILES tells you the size of the job; the count of CONTEXTS tells you whether one
> replacement string can even do it.**

Those are different questions and **only the second can invalidate the choice** — yet the file
count is the one every scan reports, so it is the one that gets used.

⚠ **Outside code the same class bites differently and is easier to miss.** A term scrubbed out of
prose also lives in **wikilink targets, filenames and YAML keys**, where a replacement containing a
space, a slash or a colon breaks the link graph or the frontmatter parser rather than a compiler.
⇒ **The failure surfaces late in every case**, which is what makes the class dangerous: the scrub
reports success and the breakage is found by whoever next uses the feature.

**Fixes:** the guard must not suggest a replacement that is illegal where the term actually
appears — at minimum it should warn when a term occurs inside identifiers. And the gate must be
chosen to match the contexts rather than copied from the last repo: `cargo check --workspace
--all-targets` where the term is an identifier (it compiles **test** targets, and a mangled
test-function name has no other symptom); resolve the lookups and parse the frontmatter where it
is a filename or a key. **A gate that cannot fail on the context in question is not a gate.**

## ⛔⛔ A HANDOVER BRIEF CARRIES THE ORCHESTRATOR'S UUID AS A LITERAL, SO SUCCESSION REINTRODUCES THE ORPHAN

**Status:** OPEN

*Fleet tooling — `ygg-monitor.py subscribe`. Repaired for existing subscribers on the morning of
2026-08-13, and back within two hours through a new one.*

`ygg-monitor.py succeed` moves every subscriber from a retired orchestrator to its successor — but
it can only repair subscriptions that already exist. **A relay's handover brief inlines the
orchestrator's uuid as a literal**, so when a cluster relays, the successor subscribes itself with
whatever uuid its brief carried.

⇒ Measured ~2 h after the original fix: a cluster's successor came up subscribed with `escalate_to`
pointing at an orchestrator **reaped at 15:58**. Its own predecessor had been correctly re-pointed
hours earlier; the brief had not, because a brief is a frozen document. **Every escalation from
that row would have gone nowhere while the plane looked healthy** — the exact failure the morning's
fix was for.

**Fix:** `subscribe` must **verify `--escalate-to` names a LIVE row** and refuse (or warn loudly
and fall back to escalating to a human) when it does not. The identical check already exists in
`escalate()`; it belongs at subscribe time too, where it is cheapest and where the stale value
enters the system. ⚠ Guard it the same way: an empty row listing is an instrument failure, not a
dead target, so require positive evidence that the row plane answered.

⭐ **The general shape, worth more than the fix:** *a repair that sweeps existing STATE does not
stop new state arriving stale from a frozen DOCUMENT.* Any identifier copied into a brief is a
snapshot, and briefs outlive the thing they name. **Validate identifiers at the point of entry,
not only in the store.**

## ⛔⛔ `sessions sort --dry-run` REPORTS `changed:false` ON A LIST THAT IS NOT SORTED

**Status:** OPEN

*Measured by another campaign 2026-08-13, sampled SIMULTANEOUSLY — both verbs launched from one
shell, so the difference cannot be time.*

```
  server app rows          2.0 2.1 [3.2] 3.0 3.1 3.3 3.4   <- wrong, and it matched the screen
  sessions sort --dry-run  changed:false, rendered_order lists 3.2 in the RIGHT place
  sessions sort (apply)    changed:true — the row moved, order correct afterwards
```

⇒ **The dry run reports the order it WOULD PRODUCE as though it were the order that EXISTS. It
cannot report a disagreement it is itself the source of.**

⛔ **And the help text closes the trap:** it tells the caller *"sorting a sorted list reports
`changed:false` — the success case, not a no-op to chase."* True of a real sort, **false of the
dry run**, so it instructs the caller to stop looking exactly when they should look harder.

**Fix:** have the dry run diff its computed order against the **rendered** order it would replace,
and report `changed` from that — which is what every caller already assumes it does.

⚠ **Ordering note:** a proposal to have `ygg-claim.sh` run `sessions sort` as its last act is
sensible and is NOT taken yet, because this defect makes the sort's behaviour untrustworthy to
reason about. **Fix the dry run first**, then the claim script's new duty can be verified rather
than assumed.


⚠ **A NAIVE RE-TEST WILL "FALSIFY" THIS ENTRY AND BE WRONG — recorded 2026-08-13
after walking into it twice.** On an already-sorted list, `changed:false` is the
CORRECT answer, and a dry run that reports reality and one that reports its own
computed order are **indistinguishable there**. Two attempts to build the
discriminating condition both collapsed back to sorted:

1. Sampling `rows` and `sessions sort --dry-run` simultaneously on the live
   sidebar — the list was in order, so `changed:false` carried no information.
2. Creating a row via `--insert-after <last row>` and then numbering it `0.1` to
   put its POSITION and its NUMBER in disagreement. ⛔ **`server app session
   outline` RE-SEATS the row as it numbers it** (its help says so), so the list
   was sorted again before the dry run could see otherwise.

⇒ **The precondition is a row whose position disagrees with its number, and no
read-only door creates one.** The remaining route is `server app sessions
reorder` with a deliberately wrong order — which rewrites the WHOLE rendered
order, including rows belonging to other sessions, so it is not a probe to fire
casually on a live sidebar.

⛔ **Do not retract this entry on a `changed:false` observed over a sorted
list.** The original measurement was taken while the order was genuinely wrong,
with both verbs launched from one shell so the difference could not be time.

**Falsifier:** with a row whose position and number disagree, the dry run reports
`changed:true`.
## ⛔⛔ A ROW-TABLE WRITE IS INVISIBLE TO THE NEXT READ, AND `rename`+`outline` CORRUPTS THE TITLE

**Status:** OPEN

Six rows renamed in one batch; the `rows` read issued immediately after showed **all six
unchanged**. A second read moments later showed **all six correct, with nothing re-sent.**
⇒ **An immediate read-back is a FALSE NEGATIVE** — and the field guide has just told the caller to
read state back, so the instrument and the instruction disagree.

**The corollary is real corruption, reproduced in two calls:**

```
rename  → "<new title>"    accepted: true
outline → 4.2.1            (composes onto the STALE title it can still see)
rows    → session_title = "4.2.1 Agent unnamed shell"
```

⇒ The seat is stored **inside `session_title`** and the rename is lost, so the sidebar renders the
seat **twice** the next time the prefix is composed.

⭐ **This is the known double-numbering hazard arriving by a route its own warnings do not name.**
Not an author writing the seat into the title on purpose — **two correct writes racing.**
⚠ **`ygg-claim.sh` survives it only by accident**: its watch loop re-asserts the title and absorbs
the lag. **That accident is currently load-bearing**, so simplifying that watcher would expose the
corruption fleet-wide.

**Fix:** `outline` must compose against the authoritative title rather than a possibly-stale read —
or the two writes must be one call. **A caller cannot fix this from outside**, because the read
that would verify it is the read that lies.

⚠ **Unmeasured and deliberately left so:** a row displaced despite `seat.honoured:true`. The obvious
theory — an unnumbered row above the numbered block shifting the insert index — **was tested and
refuted** (probe seated exactly right). **No cause is recorded, because none was measured.**

## ⭐ CONTEXT-MENU SESSIONS SHOULD READ "New Claude Code Session", NOT "New claude-code session"

**Status:** OPEN

*Owner-requested 2026-08-13. Small, cosmetic, and explicitly wanted.*

A session launched from the right-click context menu is named **`New {kind} session`**, which is
confirmed to be **the correct behaviour** — the request concerns only the rendering of the kind.

⇒ **Wanted:** `New Claude Code Session`, `New Codex Session`, `New Kimi Session` — the CLI's
**display name**, title-cased, not its slug.

⚠ So this needs a **display-name mapping per CLI kind**, not a `replace('-',' ')` and title-case:
`claude-code → Claude Code`, `codex → Codex`, `kimi → Kimi`. A naive transform gets `Claude Code`
right by luck and will get later kinds wrong. ⛔ **The kind slug stays the SSOT** — this is a
presentation layer over it, and the slug must not be renamed to make the label easier.

⛔⛔ **DO NOT BUILD THE MAPPING — IT ALREADY EXISTS** (checked 2026-08-13 by the lane that would
otherwise have built a second one). `AgentCliDescriptor::display_name` is that field, and three
things already read it: `new_session_label()` composes `New Claude Code Session` for the menu
entries, `terminal_kind_title_suffix()` uses it for row titles, and `session_metadata_label` is
locked to `"<display_name> Session"` by a test. ⇒ **This entry is a CALLSITE that is not reading
it**, not a missing mapping — find the one composing the title from `session_kind_label()` (the
SLUG accessor) and hand it the descriptor instead. A second mapping is the defect the slug-as-SSOT
rule exists to prevent.

## ⚠ A ROW ADOPTED BY A CAMPAIGN MAY HAVE BEEN THE OWNER'S, AND THE FIELD THAT WOULD SETTLE IT IS UNKNOWN

**Status:** AWAITING A DECISION

A row titled `Agent unnamed shell` (uuid tail `0462c0fb66e1`) was seated under another campaign's
sub-seat, re-titled by that campaign, and is now driving a live surface for it.

⛔ **It may be an owner-created row that a delegate simply attached to**, rather than a stray. The
standing row-hygiene law is that an agent may name its own row and the rows it spawned and nothing
else — and **adopting** a row is that same act wearing a different name, so it needs the same
permission. Only the owner can say which this was.

**Recommendation, and what the campaign is doing meanwhile: leave it exactly as it is.** The row is
in active use, the current title is accurate, and reversing it mid-flow costs more than waiting.
**Reversal is two calls** — `rename` back to `Agent unnamed shell`, then `outline ""` to clear the
seat — after which the campaign opens its own surface instead.

⚠ **And the identifying chips are NOT stored in `detail_label`.** Dumped before any rename, six
near-identical rows all already read the generic *"Interactive shell rooted at …"* boilerplate. The
one chip that proved recoverable came from the application's own banner line on the row's screen,
not from the row record. ⇒ **Where the sidebar renders those chips from is unknown, and finding it
is the real engineering here** — it is the field that actually carries a human's identification of
a row, so no restore of a mis-tidied row can be promised until it is known.

## ⚠ ~1700 COMMITS LIVE ONLY IN ONE CLONE'S REMOTE-TRACKING REFS — KEEP OR DROP?

**Status:** OPEN

*A decision, not a defect. Nothing is broken and nothing is at risk today.*

Five branches are reachable from one host's `refs/remotes/<peer>/*` and from **no origin ref**:
two `fix/*` lanes, a first-launch overflow fix, and two hardware-experiment lanes. **They no longer
exist on the peer they name** — that host now carries only `main`. So the remote-tracking refs are
stale pointers to branches that are gone, and **that one object store is the only place ~1700
commits live.**

**Measured, because the urgency was initially overstated as "one `gc` from gone":**

- `git gc` collects only **unreachable** objects, and `refs/remotes/*` **are refs**, so those
  objects are reachable (5,983 from all refs on that clone).
- Neither `remote.<peer>.prune` nor `fetch.prune` is set, so nothing prunes automatically.

⇒ **The hazard is a deliberate `git remote prune` / `fetch --prune`, not background maintenance.**
Lower urgency, and a much more specific thing to warn people away from.

⛔⛔ **AND THE OBVIOUS WAY TO PRESERVE THEM IS THE ONE THING THAT MUST NOT HAPPEN.** These are
**pre-rewrite lineage**: they carry the private strings the history rewrite exists to remove.
**Pushing them to origin to save them would republish, at scale, exactly what was just scrubbed —
and do it after the rewrite reported success.** Anyone thinking *"let's not lose 1700 commits"*
reaches for precisely that. ⇒ They stay local and unpushed. If they are ever judged worth keeping,
they are **scrubbed first and pushed second**, never the reverse.

**Recommendation:** leave them in place and decide deliberately later. They cost nothing where they
are, no work is blocked, and destroying another agent's abandoned lanes is not a call to make in
passing. ⚠ Whoever decides should check whether the work landed by another route first — an
abandoned branch whose content is already on `main` is a different question from one that is not.

## ⛔ A DIRTY CHECKOUT ONLY WARNS, SO A RELEASED BINARY STAMPS ITSELF `-dirty`

**Status:** OPEN

*Found 2026-08-13 when a released build could not be traced to a commit.*

`deploy-fleet.sh` **detected the dirty checkout and warned in exactly the right words**, naming the
modified lockfile and the untracked file — then deployed anyway, because a dirty tree is a WARNING
while the ancestry check is a REFUSAL. The binary that reached the desktop host carries
`<sha>-dirty`, so *"one version must mean one build"* holds in the code and not in the artefact.

⚠ **The untracked file was an orchestrator's dropped brief**, so the agent best placed to override
the warning was the one that had caused it. ⇒ A warning is worth little against a caller who
already believes the tree is fine.

⛔ **And the misdiagnosis is part of the entry.** It was first routed to the deploy-identity cluster
as a stamping defect. That cluster was innocent and its guards had worked correctly; the briefed
cluster supplied the real cause. **A `-dirty` stamp is evidence about the TREE, not about the
stamping code.**

**Recommendation:** refuse for a release build, warn for a local one — mirroring the ancestry guard,
which earns its keep precisely by refusing. ⭐ Cheaper still, and it removes the cause rather than
the symptom: briefs must not be written into a build checkout (see the fleet skill on file drops).

## ⛔⛔ `terminal send` SPLICES INTO A HUMAN'S HALF-TYPED SENTENCE, AND THE `\r` SUBMITS THE FUSION AS THEIR OWN TURN

**Status:** OPEN

*Reported by a campaign row 2026-08-13, with the cost already incurred. **Confirmed from the
target's own transcript before filing** — the fused turn is present at the reported timestamp,
286 characters, one `user` record.*

**An agent row can have a human typing in it, and `send` shares that one input buffer with their
keystrokes with no interlock.** The documented two-write rule — text, pause, then a bare `\r` —
becomes a splice-and-commit when the owner is mid-sentence:

1. The agent's text is **inserted into the middle of a word** the human is typing.
2. The bare `\r` **submits the fusion as a single human turn.**
3. ⇒ **The agent's words end up in the transcript attributed to the human.** That is provenance
   corruption in an orchestrator's own context, and the exact inverse of the standing rule that a
   relay of the owner's words is not the owner's ruling.

⛔ **Every field reported success.** `send` answered `accepted: true`, a byte count, a chunk count
and an accepted read-nudge, while the effect was a splice into a person's keyboard buffer. This is
the *verb reports the request, not the effect* family, and it is a new member.

⚠ **The repair is itself unsafe, and there is no safe clear.** Removing the spliced text needs more
keystrokes written into the live box: a kill-line ate a partial sentence of the human's outright,
and the alternative was ~177 backspaces. **Escape is not available** — on an agent row mid-turn it
interrupts the turn, and the row in question was nine minutes into an expensive one. So every
clearing tool also eats human text.

⚠ **And oversized pastes vanish instead of splicing.** Two multi-kilobyte single-line sends never
reached the box or the transcript, `accepted: true` both times. **Both failure modes report
success**, in opposite directions, which is why neither is visible to a caller.

### ⛔⛔ THE DROP IS BIDIRECTIONAL, WHICH RAISES THE SEVERITY

*Second instance, same day, opposite direction.* A row composed a **4,090-byte reply** to another —
an acceptance, a ruling that a shared-file write had destroyed a third row's entries, a next-task
assignment with five constraints, and a hazard warning about two repos. **It never arrived through
any channel.** It was recovered only because the intended recipient happened to audit the sender's
scratchpad for an unrelated reason, and would otherwise have finished its session without the task,
without the ruling, and without the warning.

⇒ **So this is not "an agent can corrupt a human's typing".** It is that **the row-to-row channel
drops payloads in both directions while reporting success.** Corruption is the visible half; the
silent half is worse, because nothing anywhere records that a message existed.

⛔ **The save-and-restore requirement fixes the corruption half and does nothing for this one.**
Both need the same missing thing:

> **A DELIVERY RECEIPT READ FROM THE RECEIVER'S STATE, NOT FROM THE SENDER'S RETURN VALUE.**

⭐ The cheap version already works as a discipline and should become a flag (`--verify-landed`):
after sending, **grep the target's transcript for a marker string from the message.** That is how
the splice was proven to have landed, and it is how this drop would have been caught.

⚠ **The supervision plane is the sharpest case of it.** `ygg-monitor.py`'s escalation uses this
channel, so if escalations can splice they can also vanish — and **a watchdog whose alert silently
drops is worse than no watchdog**, because the fleet reads its silence as health. ⇒ Escalations
need the receipt more than ordinary messages do, not less.

**⭐⭐ THE REQUIRED BEHAVIOUR IS OWNER-SPECIFIED (2026-08-13), AND IT IS NOT "REFUSE":**

> **Before writing, check whether the human has typed something. If they have, YANK it, send the
> message, then PASTE it back into the next prompt as if nothing had happened.**

⇒ **Preserve the delivery AND the keystrokes.** The reporter's first preference — refuse while the
buffer is non-empty — was the obvious fix and is the wrong one: it protects the human by dropping
the message, so a row with someone sitting at it becomes unreachable exactly when a delivery may
matter most. **Save-and-restore keeps both.** ⛔ The restore is not optional decoration; a yank that
fails to put the text back is strictly worse than the splice, because the loss is then silent.

**The supporting pieces, re-ranked against that requirement:**

1. **An atomic text-and-submit** (`--enter`), so nothing can interleave between the write and the
   submit. Save-and-restore needs this: a non-atomic version has a window where the human resumes
   typing into a box the caller believes it owns. ⭐ It also retires the two-write rule, which
   exists only because the one-write form does not submit.
2. **Expose `input_buffer_len` / `human_typing_since_ms`** on `terminal read-buffer` or on `send`'s
   reply. The check the requirement opens with needs a field to read, and there is none today.
3. **Refuse or chunk oversized pastes** rather than dropping them silently — save-and-restore does
   nothing for a message that never arrives.
4. ⚠ **The clear must not be a kill-line.** The repair path that ate a human's sentence used one.
   Whatever performs the yank has to capture the buffer first and be reversible, and **Escape is not
   available** as a clear on an agent row mid-turn.

⛔ **THE SUPERVISION PLANE IS ITSELF A CALLER OF THIS VERB.** `ygg-monitor.py`'s `escalate()` writes
text and then a bare `\r` to an orchestrator's row — the exact splice path — and an orchestrator's
row is among the likeliest places for a human to be typing. Two escalations took that path on
2026-08-13. ⇒ Whoever implements the requirement must fix the monitor's send with it, or the
supervision plane keeps the defect after the verb is repaired.

⭐ **The doc fix matters more than the feature, because it is the layer an agent actually reads.**
The messaging sections of the fleet skill and the data-fabric skill both present `terminal send` as
the normal way to reach another row. **Neither says that a row with a human at it is not addressable
this way at all.** The reporter read both before sending and still walked into it. ⇒ Crossings to
such a row go **by file** — dropping the brief into the target's own scratchpad worked and cost
nothing. ⚠ A file drop leaves litter; the sender owns removing it.

## ⛔⛔ `ListAgents` OMITS LIVE ROWS, SO "PICK THE PLAUSIBLE ONE FROM THE LIST" IS UNSAFE

**Status:** OPEN

*Reported by a downstream campaign row 2026-08-11.*

A row spawned minutes earlier — running, transcript growing, holding its campaign's booter
subscription, seated and titled in `server app rows` — **did not appear in `ListAgents` at all**.
An unrelated row with a similar title did. The caller picked the visible one and messaged it.

**Cost, and it is not theoretical.** Cross-session messages wake the recipient. That send woke an
**idle** row, which paid a **cold** cache re-read of its whole context to answer a question it had
no stake in. One such mistaken wake on a ~2 MB idle row was priced at several US dollars, incurred
in about a second. A listing that silently omits reachable rows makes this failure the DEFAULT for
any caller that does not already hold the recipient's UUID.

**What is true and what is not:**
- `server app rows` had the row, with `outline_prefix` and `full_path`. So the daemon knew.
- The spawn's own reply had returned `session_path` for it moments earlier.
- ⇒ the gap is in what `ListAgents` enumerates, not in whether the row existed.

**Suggested fix, in preference order:** (1) enumerate from the same source `server app rows` uses,
so the two cannot disagree; (2) failing that, make the omission VISIBLE — a count of rows known to
the daemon but not listed, so a caller can tell "no such row" from "not shown here"; (3) accept a
UUID as a `to:` address, so a caller holding a spawn's `session_path` never has to consult a list.

⚠ Until then the documented workaround is in the fleet skill: resolve the recipient's UUID from
`server app rows` or the spawn reply, address on that, and **deliver by file if it cannot be
resolved** rather than guessing from a title.

### ⭐ AND THE SIBLING FAILURE: THE PEER NAME IS A DIFFERENT NAMESPACE FROM THE ROW TITLE

*Found 2026-08-13 by an orchestrator that was about to address four rows at once.*

The listing did **not** omit anything that time. It listed every row — **by peer NAME**, and a peer
name is derived independently of the row title. So the two namespaces disagree in both directions:

- some rows list under their full title, so a caller assumes titles are what it shows;
- others list under a short derived slug (`repo-xy`) that **names the repository a session's cwd is
  in, not the work it is doing**.

⇒ **A slug that looks like it belongs to your campaign can belong to a different one entirely.** In
the observed case a row slugged for this repo was another campaign's orchestrator, sharing only a
working directory. Addressing it on the strength of the slug would have delivered a cluster brief
to an unrelated tree — and, because a send is a wake, paid for the privilege.

**The authoritative local mapping, and it is cheap:**

```sh
# name -> sessionId -> cwd, for every CC session on this machine
python3 - <<'PY'
import json, glob, os
for f in glob.glob(os.path.expanduser("~/.claude/sessions/*.json")):
    try: d = json.load(open(f))
    except Exception: continue
    if d.get("sessionId"):
        print(f"{d.get('name','?'):<28} {d['sessionId'][:8]}  {d.get('status','?'):<6} {d.get('cwd','?')}")
PY
```

⚠ **What that answers, and what it does not.** cwd tells you **who is in the blast radius** of an
action on a directory — which is the right question before a history rewrite, a worktree reset or a
deploy. It does **not** tell you who is in your campaign: a row whose work is in a vault or a graph
has a cwd nowhere near the repo it is reasoning about, and will be missing from a cwd-keyed answer
while being entirely relevant to a subject-keyed one. **Pick the key that matches the question.**

⇒ This strengthens suggestion (3) above rather than replacing it: a `to:` that accepts a UUID
removes both failures at once, because a UUID is the only identifier in this system that belongs to
exactly one namespace.


### ⚠ RE-TESTED 2026-08-13 — the DANGER half is confirmed outright; the OMISSION half could not be measured, and the reason is the finding

**Confirmed, from a single listing taken today:** the enumeration contains
**two separate pairs of entries carrying byte-identical names**, distinguishable
only by their `[ref]`. So "pick the plausible one from the list" is not merely
risky, it is undecidable for those four — and the cost the entry prices (a
mistaken cross-session message forcing a cold context re-read on an idle row) is
paid by whoever guesses wrong.

✅ **A mitigation has since appeared and is worth recording:** sending to a bare
name that matches more than one entry is now **refused**, with the tool printing
the candidate and its ref and asking for confirmation. Measured tonight — a send
to a bare name was rejected until re-sent with the ref. That turns the dangerous
guess into a prompt.

⛔ **The OMISSION half could not be tested, and the obstacle is the same defect
one layer down.** The listing's entries are **session names** — for most rows an
auto-generated slug, not the row's title — and the listing carries **no session
path and no uuid**. So a row from `server app rows` and an entry in the listing
**cannot be matched by any field either of them publishes.** Counting is all that
remains, and counting cannot separate "omitted" from "named differently":

```
claude-code rows the GUI reports on that machine   49   (30 of them seated)
entries in the listing                             30
```

⇒ **The reason the original reporter had to guess by title is that there is
nothing else to guess by**, and that is also why the claim cannot be checked.
⭐ **The fix that makes this entry testable is the same one that makes it safe:
put the row's session path in the listing.**

⚠ **`live_member` IS NOT A LIVENESS FLAG** — noted while attempting this. All 49
rows report `live_member: true`, including ones with no running CLI; it means
"member of the Live group", not "the process is running". Anyone re-testing this
by filtering on it will compare the wrong population.

**Falsifier:** a listing entry and a row can be matched by a published field, and
a row known live by uuid is present in the listing.
## ⛔⛔ THE BOOTER KICKED A CONTEXT-DEAD SESSION EVERY 10 MINUTES FOR TEN HOURS, AND ITS OWN LOG SAID "WORKING"

**Status:** OPEN

⭐ Defects 1, 2 and 4 FIXED 2026-08-10; defect 3 turned out to be the same fix as 1.
What remains open is the monitor-subscription question filed separately below.

**Reported by row 8 (practice campaign) 2026-08-10, MEASURED not theoretical**, on
`ygg-booter` / `ygg-babysit` in `.agents/skills/yggterm-agent-fleet/` — which this
campaign owns. Return address `remote-cc://dev/4ff2ecbd-aee7-48f9-b8f5-e68142828863`.
Evidence: `~/.yggterm/relay/booter.log` lines 225-390 (00:00-02:30), and 9
`Prompt is too long` rows in that session's transcript between 00:00:37 and 02:20:54.

**The incident.** Session `569e15eb` ran a relay 8.5 h with `autoCompactEnabled:false`
and grew 49k → **976,493 tokens**. From 00:00:37Z every turn returned
*"Prompt is too long"* — unrecoverable, nothing armed to compact it. The booter then
kicked the corpse every ~10 min for **ten hours**, and the owner found it by looking
at a screen. (This is the same row whose viewport also carried the untouchable-resume
bug; the two are unrelated.)

### ⭐ DEFECT 1 — A CORPSE ANSWERS FASTER THAN A WORKER, SO THE CLASSIFIER CALLS IT WORKING

`ygg-babysit.classify()` derives liveness from `os.path.getmtime(transcript)`. A
REFUSED turn writes three rows (user + synthetic assistant + turn_duration) in
**5-66 ms**, so the mtime resets and age goes to ~0. The log literally reads
`WORKING 0.1m 569e15eb` about a session dead for two hours, alternating
WORKING / JUST_ENDED / IDLE→BOOT#1 forever.

⇒ **THE GENERALISING LAW, and it is the reusable part: an error returned FASTER than
a success looks like HEALTH to anything that measures ACTIVITY rather than OUTCOME.**
Same family as this repo's own *verbs report the request, not the effect*.

### ⭐ DEFECT 2 — THE ANTI-FLAP COUNTER IS DEFEATED BY THE SAME WRITE

In `tick()`, `grew = size > last_size`, and both `if grew: s["boots"]=0` and the
WORKING/JUST_ENDED branch reset the counter. **The rejection GROWS the file**, so
`boots` never accumulates. Fingerprint: every boot in the log is `BOOT#1` — never #2,
never #3 — so `MAX_BOOTS`/escalate could not fire for the real reason. It escalated
hours later only once boots stopped landing at all, and the subscription died at
10:46 on `--max-hours 12`, not on a diagnosis.
⇒ *"Did the file grow"* is not *"did the agent work"*. Candidate discriminator:
growth containing no `tool_use` and no non-zero `usage` is not progress.

⭐ **VERIFIED against `booter.log` by this row, not taken on trust:** in the stated
00:00-02:30 window there are **8 boots and all 8 are `BOOT#1`**. Across the whole
log the counter *does* reach #2 and #3 — 43×#1, 6×#2, 4×#3 — which corroborates the
mechanism rather than contradicting it: the counter accumulates only when the file
does NOT grow, i.e. on a genuinely idle session, and resets on every context-death
rejection.

### ⭐ DEFECT 3 — THERE IS NO CONTEXT-DEATH STATE, AND IT IS THE ONE A BOOT CANNOT FIX

Booting a context-exhausted session is not merely useless — it is the only case where
retrying is **guaranteed** to fail forever. It needs its own terminal state: stop
booting, escalate ONCE, and say the true thing (*"this session is unrecoverable,
relay it"*) instead of *"did not wake after 3 boots"*.

### ✅ FIXED 2026-08-10 — what shipped

- **`ygg-babysit.classify()` reads the gauge BEFORE the transcript** and returns a
  terminal **`CONTEXT_DEAD`**. That is defects 1 and 3 at once: the corpse is now
  identified by OUTCOME, not by activity.
- **`ygg-booter` escalates ONCE** with *"context exhausted and UNRECOVERABLE — relay
  the campaign to a fresh session"* and then **unsubscribes**, instead of kicking a
  grave every ten minutes. A watchdog that keeps barking at a grave teaches its owner
  to ignore it.
- **The anti-flap counter stopped trusting bytes.** `progress_marks()` counts only
  turns that used a tool or spent output tokens. ⇒ Proven against the REAL incident
  transcript: appending its 9 genuine `Prompt is too long` rows grows the file by
  **5,640 bytes** and moves marks **79 → 79**. So `MAX_BOOTS` can finally fire for the
  reason it exists.
- **Defect 4 (the skill) was already fixed by row 8**; §2 now names the gauge as the
  automatic path. Corrected further so it no longer claims babysit *must* infer from
  mtimes — it doesn't any more.
- ⚠ **A missing or stale gauge is NO INFORMATION, never "healthy"** — pinned by test:
  an absent gauge falls through to the transcript classifier rather than reading clean.

### ⭐ THE SEAM — already built, do not re-derive it

An external watchdog cannot see a token count; it exists only inside the CLI, which is
why it infers from mtimes. Row 8 shipped a `UserPromptSubmit` hook
(`~/.claude/hooks/context-relay-gauge.py`, registered in `~/.claude/settings.json`,
proved end-to-end on a live session) that writes on EVERY prompt:
`~/.claude/context-gauge/<session_id>.json` →
`{"pct":98,"used":976493,"window":1000000,"verdict":"CRITICAL","dead":true,...}`
`verdict` is OK/NOTICE/LAND/CRITICAL; `dead:true` means the tail already carries
*"Prompt is too long"*. ⇒ **"Is this row about to die" becomes a lookup, one `open()`.**
⚠ Staleness is ours to handle: the file is only as fresh as that row's last prompt.

### ⚠ DEFECT 4 — SKILL, NOT CODE: the only context instrument in the fleet is MANUAL

`yggterm-agent-fleet/SKILL.md` §2 tells an agent to submit `/context` to its own row
and then chase it with `continue` or stall its own loop. **A measurement that costs a
round trip, can stall the caller, and must be REMEMBERED is one an agent under load
skips** — and this one skipped it for 8.5 hours. §8 step 3 already forbids the outcome
(*"I ran low on context does not license skipping the handoff"*) but **a prohibition
with no trigger is unenforceable**. §2 should point at the gauge as the automatic path
and keep `/context` as the interactive one.

## ⛔⛔ A NEW SESSION IS ASKED FOR, THE ROW APPEARS `running · idle`, AND NO PROCESS IS EVER BORN

**Status:** OPEN

reported 2026-08-10: *"even a new session does not want to start"*. Traced
on the failing attempt: `start-cc 0eb607df…` at **13:00:40**, then **no birth, no
error, no process** — while the row reported `running · idle`. A row that claims
to be running a session that does not exist is the worst possible failure mode,
because nothing in the UI contradicts it and the owner waits.

⚠ **This entry exists BECAUSE a plausible cause was found, measured, fixed — and
then did not fit.** A daemon used to walk the machine's whole transcript corpus
before binding its socket, so for ~15 s after spawn it answered nothing at all;
that is fixed and proven in 3.0.94 (15.1 s → 0.14 s). But the daemon serving that
13:00:40 request had started at **11:19:07** and did its only corpus walk at
**11:19:15** — 100 minutes earlier. **So the startup stall cannot be this.** Do
not let the shipped fix close this entry; the symptom has not been reproduced
since and its cause is unknown.

Where to look next, in order:
1. **What `start-cc` returns when it fails.** No error surfaced anywhere — so
   either the request never reached a daemon, or a daemon accepted it and dropped
   it silently. Those are different bugs and the trace should distinguish them;
   if it cannot, that is the first thing to fix.
2. **Who set the row to `running · idle`.** A row is being created optimistically
   and never reconciled against whether a process appeared. The row's state is a
   claim about a PID; something must check it.
3. Whether the GUI addressed a daemon version whose socket had gone.

**Falsifier:** a `start-cc` that spawns no process must leave either an error the
user sees or a trace event naming the refusal — never a row that says `running`.

## ⛔ A TEST'S MUTEX GUARDS IT AGAINST ITSELF, NOT AGAINST THE SUITE

**Status:** OPEN

`shell::tests::sidebar_search_context_memo_skips_rebuild_on_unchanged_inputs`
fails intermittently in a full `cargo test -p yggterm-shell` run and passes every
time in isolation. Observed 2026-08-11: one failure, then two consecutive clean
full runs with no code change between them.

Its own comment names the hazard and then does not close it — *"Serialized via a
mutex because the function touches process-global statics shared with other
tests"*. The mutex is a `static TEST_LOCK` **declared inside the test function**,
so it excludes only other invocations of the same test. Every OTHER test that
calls `set_sidebar_search_context` walks straight past it and moves
`SIDEBAR_SEARCH_CONTEXT_REBUILD_COUNT` and
`LAST_SIDEBAR_SEARCH_INPUT_FINGERPRINT` underneath the assertion.

⇒ the assertion `after_first - before == 1` is a race, not a fact. The fix is
either a lock shared by every test that touches those statics, or a harness that
does not read process-global counters at all — a memo gate can be stated as a
pure function of (inputs, last fingerprint) and tested without them.

⚠ **Why this matters more than one flaky test:** it fails in the full run, which
is the run a release gate uses, and it passes on the retry — the exact shape that
teaches a session to re-run instead of look.

## ⛔ THE WEBVIEW DIAGNOSES A SHUT INPUT GATE AND TELLS NOBODY

**Status:** OPEN

The terminal script measures `input_dead_ms` (caught live at **24,032 ms**),
records `input_dead_active_element`, and sets `passive_focus_recovery_state =
rust_gate_closed_while_window_focused` — a precise, positive fault. Then it
declines to act, and its own comment says why: *"the webview cannot repair this
one, only the rust policy can"*.

⇒ **`rust_gate_closed_while_window_focused` appears NOWHERE in Rust** — only in
the JS that emits it and a test asserting the string exists. The handoff it was
named for was never built, so the diagnosis has been reaching no one. 3.0.110
added the Rust side's own report (`input_gate_stuck_unrestorable`, fired live at
`denied_for_ms: 26531`), which covers the case from the other end, but the JS
signal is still inert.

⚠ The webview could not have named the true cause anyway: `remote_resume_input_ready`
is a Rust-only signal and is invisible to it. So the handoff worth building is
the reverse one — the gate telling the surface why it is shut — not the surface
guessing.

## ⛔⛔ STRAY GLYPHS APPEAR IN THE TUI AND CLEAR ON SCROLL, AND THE HEAL CLAIMED SUCCESS EVERY TIME

**Status:** OPEN

**Reported 2026-08-11, verbatim:** *"Renders in TUI are weird characters
appearing here and there and going away on scrolll."*

⛔⛔ **AND THE SEVERITY IS FAR WORSE THAN THAT SENTENCE — HE THEN SENT THE
FRAME.** It is not stray cells: **the ENTIRE terminal viewport was garbled, every
glyph wrong, top to bottom.** The structure survives perfectly — word shapes,
line lengths, indentation, colours, emoji placement, the highlighted user blocks
— while every character is a different character. The DOM sidebar and the
notification panel in the same screenshot render flawlessly. ⇒ **the WebGL glyph
atlas is being indexed wholesale wrong**, which is the textbook full form of
`webgl-stale-atlas-garble`, and the terminal is simply unreadable while it lasts.
⚠ **Do not size this bug from the reported sentence.** "Weird characters here and
there" describes the mild case; the pasted frame is the same defect at full
amplitude, and it is the one that matters. The mild case and the full case are
the same mechanism at different atlas-staleness depths.

**Confirmed in a faithful pixel**, not from telemetry — `server app screenshot`
on the GUI host, `capture_faithful: true`. Two artifact classes, both present in
one frame:

1. **Orphan single characters that belong to no word on their row.**
2. **A row painted twice.** The CLI's own footer hint line rendered as two
   identical adjacent rows.

⭐⭐ **ROOT-CAUSED AND FIXED (3.0.108): THE SHIPPED xterm SCORES EMOJI ONE CELL
WIDE.** Measured against the exact vendored bundle, not inferred:

    activeVersion = 6 | registered = ["6"]      ← the ONLY table it has
      wcwidth(⭐ U+2B50)  = 1     ← Unicode 9+ says 2
      wcwidth(⛔ U+26D4)  = 1     ← 2
      wcwidth(✅ U+2705)  = 1     ← 2
      wcwidth(🚀 U+1F680) = 1     ← 2
      wcwidth(中 U+4E2D)  = 2     ← CJK was never wrong
      wcwidth(⚠ U+26A0)   = 1     ← correct: text presentation

⇒ Claude Code writes `⭐` believing it consumed **two** columns; xterm advances
**one**. From the first emoji on a line the writer and the renderer disagree
about where every later cell is, and a partial repaint strands the old glyph in
the orphaned column. That is the stray character — and it is why it **clears on
scroll**: a full-line repaint re-lays the row out consistently in the renderer's
own terms.

⭐ **Every detail of the owner's two frames follows from this**, which is what
raised it from plausible to established: the strays sat immediately after `⭐`
and `⛔` (both Emoji_Presentation, both scored 1), while `⚠` in the same frames
rendered correctly (text presentation, correctly 1). Agent CLIs emit emoji
bullets constantly, so this fires all day.

**Fix:** a Unicode-11 provider registered over the bundle's own v6 table,
widening **exactly** the Emoji_Presentation=Yes set (83 ranges, binary searched)
and delegating everything else to v6 — rather than substituting a second full
width table that could drift. ⚠ `charProperties` is overridden alongside
`wcwidth` because the RENDERER reads the packed properties; a `wcwidth`-only
override would look right and paint the old widths.
⛔ **Text-presentation symbols must stay narrow** (`⚠`, `✻`, `❯`, `✔`, `ℹ`) —
widening those creates the identical misalignment in the opposite direction.
Guarded against the real bundle in `tools/xterm-harness/emoji_cell_width.test.js`
(which also pins the pre-fix widths, so a bundle upgrade that fixes this upstream
fails the test and tells you to drop the provider) and in
`shell::tests::terminal_eval_script_widens_emoji_to_two_cells`.

⚠ **The daemon's vt100 mirror (`vt100` 0.16.2) has NOT been checked for the same
disagreement.** It feeds `terminal_lines`, the working-indicator and the title
heuristics — not the paint — so it cannot cause this symptom, but if it also
scores emoji at 1 then every screen-text instrument is subtly misaligned on
emoji rows. Unmeasured; worth one probe.

⚠ The doubled row and the orphans were **two bugs sharing one report**. Only the
doubled row belonged to the rAF-gap atlas staleness fixed in 3.0.105. The orphans have their own mechanism and are **untouched by that fix**.

⭐ **THE DUPLICATE IS NOT CONTENT — SO IT IS OURS.** The daemon's vt100 screen is
the SSOT for what the terminal HOLDS, and it holds that row exactly **once**
(`server snapshot` → `active_session.terminal_lines`, one match). The canvas
painted two. ⇒ the extra row is painted-but-not-in-buffer: stale canvas cells,
not a CLI repaint and not a grid mismatch. This is the falsification that
matters, because a resize mismatch would have put a real duplicate in the buffer
too — and it did not.

**The renderer is WebGL** on this host (`xterm_renderer_mode: canvas`,
`xterm_renderer_policy_reason: xterm_webgl_enabled_for_wayland`), so this is the
`webgl-stale-atlas-garble` family (docs/xterm-bugs.md).

⛔⛔ **AND THE DETECTOR HAS BEEN VOUCHING FOR A REPAIR NOBODY MEASURED.**
`render_fail_pattern/detected` carries **8 × `stale_atlas_paint`** on this host,
every one of them `healed: true` — while the owner was looking at garble. That
field was **the literal `true`, written into the payload before the
`setTimeout` that performs the heal had even been armed.** It asserted an
intent and was read as an outcome, which is why this symptom survived every
"the trace says it is being handled" pass. [[finding-a-set-is-not-a-fill]]

**Root cause of the misses, code-cited (`crates/yggterm-shell/src/shell.rs`,
the `term.onRender` stale-atlas block):** detection also required
`staleAtlasNowMs - rafGapMonitor.lastGapEndedAtMs < 600` — the render had to
land within 600 ms of the rAF-throttle gap ENDING. But what makes a paint
garble is that the glyph atlas has not been cleared since the gap BEGAN, and
that stays true until something clears it. The everyday shape is exactly the one
the window excluded: the window is occluded, rAF throttles, and the terminal does
not repaint until the agent writes its next output *seconds* later. That paint is
equally garbled and was never detected, never healed, never traced.
[[finding-a-guard-that-cannot-see-the-moment-it-guards]]

**Fixed in 3.0.105:** the proximity window is gone (the per-episode latch, not a
clock, is what bounds the heal), the detection record says `heal_scheduled` and
carries `render_lag_after_gap_ms`, and the heal traces its own outcome
separately as `stale_atlas_heal_outcome{atlas_cleared, rows_refreshed,
duration_ms}`. Guarded by
`shell::tests::terminal_eval_script_wires_stale_atlas_paint_detector`.

⭐⭐ **AND 3.0.106 STOPS REPAIRING IT AND PREVENTS IT.** Detection is inherently
too late: it can only fire once a garbled frame has been painted and *seen*, and
what he saw was every glyph on the screen. The rAF gap monitor's own tick is the
first frame after the throttle ends, so it runs before any render can use the
stale atlas — every mounted host's atlas is now cleared *there*, and the clear
stamps the same `lastAtlasClearAtMs` the detector reads, so the repair path
correctly stands down when prevention already did the work. The repair path stays
as the backstop for staleness that arrives by some other route.

⭐⭐ **RE-TESTED INDEPENDENTLY ON 3.0.110, AND THE EMOJI-WIDTH CHANGE IS NOT THE
ORIGIN.** A previous session cleared its own 3.0.108 change with a harness run
showing identical buffer content with and without it. ⚠ That proves the BUFFER
is coherent, not the PAINT, and the session had an obvious motive to clear its
own work — so it was re-tested from the other end, on a faithful pixel captured
on the desktop host after 3.0.110 shipped (`capture_faithful: true`).

**The frame still garbles, and its MORPHOLOGY settles the question.** What is
wrong is *which glyph* is painted, never *where*:

- single characters replaced in place, with the word's remaining letters on
  their correct columns;
- a run of line-numbers whose leading digit paints as a letter;
- line lengths, indentation, wrapping and colour runs all intact.

⇒ **substitution, not drift.** A wcwidth error can only ever produce DRIFT:
every cell after the first mis-scored character shifts by one column, and the
damage is confined to lines containing such a character. The observed damage is
on lines with no emoji at all and preserves every column. The two hypotheses
predict visibly different pictures, and the picture is the atlas one.

⇒ **Test to apply before blaming any width change for a garble:** ask whether
the text is in the WRONG CELLS or the WRONG GLYPHS. Only the first can come from
a width table. This also means the 3.0.105/106 atlas work has NOT closed the
family — prevention on the rAF-gap edge is not catching every route to a stale
atlas, and the next investigation should start from which route this frame took,
not from whether the atlas is the mechanism.

## ⛔⛔ 3.0.106 CAUSED A SECOND RENDER BUG, AND THE HONEST TRACE IS WHAT CAUGHT IT

**Status:** OPEN

⚠ The regression itself is fixed in 3.0.107; this stays OPEN until the fix is
proven on the owner's screen after a real occlusion episode.

**Reported 2026-08-11, ~13 minutes after the 3.0.106 restart:** digits
eaten out of a diff's line-number gutter (`152824` → `1 2824` → `1 2 26`;
`118486` → `18486`) and **rectangular holes punched through the added-line
highlight**. Cells MISSING glyphs and background — the inverse of the orphan-cell
bug above, where cells held glyphs they should not.

⭐ **Caused by 3.0.106, and the instrument this campaign had just taught to stop
lying is what proved it.** The trace:

    raf_gap_ms: 1794                 ← the SAME gap, every firing
    render_lag_after_gap_ms:  483054 → 507941 → 723745 → 776131
    atlas_age_ms: -1                 ← "never cleared"
    atlas_cleared: true, rows_refreshed: 64   ← 18 times

⇒ the heal re-fired against **one 13-minute-old rAF gap**, wiping the glyph atlas
mid-session over and over. Every wipe re-rasterizes every glyph, and cells painted
before their glyph lands come out **blank**. ⚠ **An unnecessary atlas clear is not
a harmless one** — that is the whole lesson, and it is why `heal_scheduled` +
`stale_atlas_heal_outcome` + `render_lag_after_gap_ms` (3.0.105) paid for
themselves within a day: under the old `healed: true` literal this regression
would have been invisible.

**Two latent bugs that dropping the 600 ms window exposed, both fixed in 3.0.107:**
1. ⛔ **`atlasClearedAtMs === 0` means the atlas was built fresh at mount and has
   never needed clearing — the HEALTHIEST state — and `0 < gapStartedAtMs` read it
   as the stalest.** So every freshly mounted host healed itself immediately, and
   kept doing it. Now a never-cleared atlas only counts as stale if the host
   actually existed during the gap (`mountedAtMs`, newly recorded on the entry).
2. ⛔ **The latch was a per-host CLOSURE variable**, so it reset to 0 on every
   mount and re-armed whatever ancient gap the page still held. It now lives on
   the page-global `rafGapMonitor`, beside the gap it latches.
3. The preventive clear also skips hosts that mounted after the gap — their atlas
   cannot be stale, and wiping it is pure cost paid in blank cells.

⚠ **Lesson for whoever touches this next:** the 600 ms proximity window was crude
but it was *masking* both bugs. Removing a guard is only safe once you have read
what the guard was hiding — [[finding-a-guard-that-cannot-see-the-moment-it-guards]]
cuts both ways.

⭐ **THE ATLAS DEFENCE IS NOW READABLE (3.0.107).** It was not:
`preemptiveClearCount` lived only on the page-global monitor and
`staleAtlasHealCount` only on the host entry, and **neither reached any
instrument** — not `server app state`, not the trace, and not
`server app web eval` (checked: that verb lands in a web *surface*, a different
page, where `window.__yggtermRafGapMonitor` is `null`). So "did the defence run?"
had no answer in either direction. Both now ride host health and aggregate into
`runtime_truth` as **`active_host_preemptive_atlas_clear_count`** and
**`active_host_stale_atlas_heal_count`**, with
`last_preemptive_atlas_clear_at_ms` per host.
**How to read them:** zero clears on a GUI that has been occluded ⇒ prevention is
not firing. A count that climbs while garble is still reported ⇒ it fires and does
not cure it, a different bug. A count climbing *fast* against one unchanging
`raf_gap_ms` ⇒ the 3.0.106 regression above, which is exactly how that was caught.

**Restarted and verified 2026-08-11 00:15 (owner said go).** GUI relaunched onto
3.0.106; it answered in ~13 s (against a documented 17-156 s range), **47 rows
back**, daemon swapped to 3.0.106 as well, `7 owned · 13 total · 6 preserved`,
nothing lost. Faithful screenshot: glyphs clean, no orphans, no doubled row.
⚠ **That is NOT proof the fix works.** A freshly restarted GUI has a freshly
built atlas, so clean glyphs at t+20 s prove only that the restart was clean. The
fix can only be judged after an occlusion episode — which is exactly what the
unobservable counter above currently prevents.

⚠ **STILL OPEN.** What is owed:
1. A faithful screenshot taken AFTER a real occlusion episode (not after a
   restart) with no orphan glyphs and no doubled row.
2. `stale_atlas_heal_outcome` records present with `atlas_cleared: true` — the
   first evidence in this bug's history that a heal did anything at all.
3. ⛔ **The orphans are a SEPARATE, UNFIXED bug** — see the wide-glyph finding
   above. 3.0.105 does nothing for them. They are the half the owner is most
   likely to keep seeing, because agent CLIs emit emoji bullets constantly.

**Next step for the orphan half, and it needs no new instrument:** the overhang
cell of a wide glyph must paint blank. Find who writes that cell in the WebGL
path and why the previous frame's glyph survives there — a damage-region that
marks only the emoji's FIRST cell dirty would produce exactly this. A repro is
cheap and does not need the owner's screen: print a line of emoji-space-text,
overwrite it in place with a shorter line, and look at the overhang cells.

## ⛔⛔ SWITCHING ROWS HAS A FAT TAIL TO 60 s, AND 11 REVEALS NEVER FINISHED AT ALL

**Status:** OPEN

**Reported 2026-08-10, verbatim:** *"ALL Switches take ~18-20s time to
spawn with a swap message."* He also said the swap message *"is probably a lie"*.
**That was correct about the lie** — that half is fixed below — and the slowness
underneath it is real and unexplained.

### Measured from every reveal on the GUI host's disk trace (n=106)

| tier / outcome | n | p50 | p90 | max |
|---|---|---|---|---|
| hot · ready | 75 | 676 ms | 2,260 ms | **63,863 ms** |
| cold · ready | 20 | 2,625 ms | **35,497 ms** | 61,983 ms |
| cold · **failed** | 7 | 62,098 ms | 97,950 ms | **97,950 ms** |
| hot · **failed** | 4 | 63,837 ms | 122,174 ms | **122,174 ms** |

⇒ The median switch is fine. **The tail is the product.** A cold p90 of 35 s is
his "18-20 s" and worse, and **11 of 106 reveals never became ready** — those sit
at ~60-122 s, which is a user staring at nothing for two minutes.

### ⛔ STILL TRUE AT 3.0.101, AND THE 60 s CEILING IS NOW A COIN FLIP

Owner-supplied 2026-08-10 21:20 (his notification panel). ONE agent row, three
consecutive reveals — read from `server app state` → `reveal_log`:

| finished | outcome | first output | total |
|---|---|---|---|
| 21:22:29 | **failed** — *"did not become interactive in time"* | — | 60,057 ms |
| 21:22:33 | ready | **63,092 ms** | 64,004 ms |
| 21:24:26 | ready | 36,548 ms | 45,963 ms |

⭐ **The first two are the SAME reveal, and the timeout is not the observer — it
is the CURE.** The full trace, and it changes what the fix is:

    21:21:29.387  open_path_resolve                       click
    21:21:32.260  remote_pty_resize_failed                "terminal session not
                                                           found: local://<id>"
    21:21:32.484  resume_recovery_begin  attempt 1, reason=no_output_stall
    21:21:32.485  resume_recovery_end    ← 1 ms. It did nothing.
      ……  45 SECONDS WITH NO EVENT AT ALL  ……
    21:22:17.904  remote_pty_resize_failed  "reading daemon response … Resource
                                             temporarily unavailable (os error 11)"
    21:22:29.449  reveal_failed + resume_timeout_cleared_inflight   ← the 60 s ceiling
    21:22:29.500  resume_recovery_begin  attempt 2,
                    reason=protected_runtime_careful_restore_after_timeout
    21:22:29.878  resume_recovery_end    ← 378 ms, and it WORKED
    21:22:32.484  attach_ready
    21:22:33.396  reveal_ready  first_output_ms 63,092

⇒ **Recovery attempt 1 is a no-op: it begins and ends in ONE MILLISECOND and
achieves nothing, then nothing happens for 45 s.**

⛔ **CORRECTION, and it changes the fix.** This entry first said *"had attempt 2's
strategy run at 21:21:32 this would have been 3.5 s"*. **There is no second
strategy.** Both call the SAME `terminal_attempt_resume_recovery_async`
(`shell.rs`), whose whole body is `terminal_ensure_with_retry_async`; `reason` is
a TRACE LABEL and is never read as behaviour. Attempt 2 differs only in *when* it
ran — by then the far daemon could answer. ⚠ Two trace events with different
`reason` strings are not two mechanisms; read the callee before believing the
label.

**The real root cause, code-cited:**

1. ⭐ **The recovery asks a question whose answer was already "yes."**
   `terminal_ensure_async` returned Ok in **1 ms**: the daemon confirmed the
   terminal RECORD exists. Existence is not output, and the session was producing
   none — [[finding-a-set-is-not-a-fill]] on the reveal path. **A recovery whose
   success condition is satisfied by the state that provoked it recovers nothing.**
2. Stall recovery is also capped at one attempt (`… && resume_recovery_attempts
   < 1`, `shell.rs`), so nothing re-tries until the 60 s ceiling — ⛔ **but see
   below: lifting that cap is INERT**, because the thing it would repeat is the
   1 ms no-op above.

⛔⛔ **AND LIFTING THAT CAP IS INERT — do not build it.** This entry previously
said the fix was "retry stall recovery on a backoff instead of once", gated on
first measuring what `terminal_ensure_async` does against an unresponsive far
daemon. **The code answers that without a measurement, and the answer kills the
fix.** For a remote session `terminal_ensure_with_retry_async` is ALREADY a
bounded retry — `attempt_timeout_ms` **30,000 ms**, `max_attempts` **8** — so it
neither hammers nor gives up early. In this incident it never entered that path
at all: **it returned `Ok` in 1 ms** because the far daemon answered immediately
that the record exists. ⇒ **Retrying a call that returns `Ok` in 1 ms returns
`Ok` in 1 ms again.** Raising `resume_recovery_attempts < 1` changes nothing.

⚖ **What that leaves, and the confidence on each:**
- **HIGH — client-side retry is not the fix.** The recovery already resets the
  read stream (`cursor = 0`, `terminal_has_visible_output = false`) and it still
  saw nothing for 45 s. The client did the correct thing once and waiting was all
  that was left.
- **HIGH — `ensure` is the wrong verb for stall recovery.** Its postcondition is
  "a record exists", which was already true. A recovery whose success condition
  is satisfied by the state that PROVOKED it cannot recover anything.
- ⛔ **FALSIFIED — "the far host was unavailable" is FALSE.** It was the one
  measurement this entry said was owed, it was taken on the far host, and it went
  the other way. Across 21:21:25-21:22:40 that host's trace carries **9,226
  events, including 4,416 matched request `begin`/`end` pairs**, spread through
  the whole 45-second gap. It was serving other callers continuously while this
  one session received nothing. ⇒ **The gap is SPECIFIC to this session's read
  stream, not a property of the host**, and every hypothesis in this entry that
  blamed something global has now been wrong.
### ⭐⭐ THE OWNING DAEMON HAD THE BYTES 50 SECONDS BEFORE THE CLIENT DID

That narrower measurement is **DONE**. Filtering the far host's trace to the one
owning daemon gives a complete attribution, and it is not any of the four
hypotheses above:

| far-host event (owning daemon) | at | Δ |
|---|---|---|
| `live_session_birth` | 21:21:32.248 | — |
| `request_terminal_launch` | 21:21:32.272 | |
| `ensure_session_begin` · `terminal_spec_resolved` · `spawn` | 21:21:41.632 | **+9.4 s hole** |
| **`first_bytes`** | 21:21:42.421 | +0.8 s — the spawn itself is FAST |
| `ensure_session_end` | 21:22:06.350 | **+23.9 s hole**, with bytes already flowing |
| *client's* first output | ~21:22:32.5 | **+26.2 s hole** |

⇒ ⭐ **The runtime produced its first bytes at 21:21:42 and the client did not
render any until 21:22:32 — a 50-second delivery gap on a stream whose source was
ready the whole time.** The actual work (spawn → first bytes) took **0.79 s**. The
other 62 seconds are three separate holes, none of them the work.

⛔ **So it is not the client's retry, not the host's availability, and not the
ceiling.** The bytes existed and were not delivered. ⚠ *Why* each hole exists is
NOT established — that is the next measurement, and it is now a delivery-path
question (daemon→stream→client), not a reveal-path one.

### ⭐⭐⭐ ROOT CAUSE, CODE-CITED: ONE MUTEX, HELD FOR THE WHOLE REVEAL

**The daemon was not slow. It was DEAF.** `daemon_request_response`
(`crates/yggterm-server/src/daemon.rs`) takes a single global
`Arc<Mutex<DaemonRuntime>>` and holds it for the entire `handle_request`. The
reveal path — `ensure_remote_runtime_cc_session` → CLI provisioning → spec
resolve → PTY spawn → `persist` — runs start to finish **inside that lock**.

    21:21:32.222  ensure_remote_runtime_cc_session  begin   ← lock acquired
    21:21:42.421  first_bytes                               ← the bytes EXIST
    21:22:06.350  ensure_session_end
    21:22:06.589  last event this daemon ever wrote         ← lock held 34.4 s

⇒ **In those 34.4 s pid 3508483 served no other request of any kind** — and its
`status` polls had been arriving every 10-200 ms right up to the moment the lock
was taken. The user's bytes sat in the daemon for 24 s because the poll that
would carry them could not be answered.

⛔ **This was invisible, and that is a second defect.** `handle_request` emits its
`request`/`begin` trace **after** acquiring the lock, so a request parked on the
mutex leaves no record at all. `begin` does not mean *"a client asked"* — it means
*"a client asked AND the daemon was free"*. A starved daemon and an idle one are
byte-identical in the trace.

**Falsified the obvious alternative ("no client asked"):** three
`yggterm-headless server terminal resize` processes spawned *during* the silence
— 21:21:34.788, 21:21:57.275, 21:22:30.746 — and **not one logged a `begin`**.
They were all parked on the mutex. Clients were asking throughout.

**Fixed in 3.0.103 (the instrument):** `lock_daemon_runtime_for_request` tries the
lock first (uncontended stays free — `status` alone runs 620k times per
fleet-day) and, only on contention, brackets the wait with
`request`/`lock_wait_begin` + `lock_wait_end{waited_ms}` and a
`daemon_lock_wait/<request>` perf row. Contention is now rankable in
`server perf-summary` beside the work starving it. Guarded by
`daemon::tests::a_request_parked_on_the_runtime_lock_is_visible_in_the_trace`.

**STILL OPEN — the cure:** get the reveal path off the global lock. The precedent
is in the same file: the Lane-A/WPE plane is answered *without* the runtime lock,
and its comment already names this exact failure — *"one `terminal_ensure` held
this loop 30.7s on 2026-06-11 and the user watched a shadow the whole time"*.
WPE was moved off the lock; `terminal_ensure`, the request actually named, was
left on it.

⛔ **BUT NOT BY THE PATTERN THIS ENTRY KEPT RECOMMENDING — that design is
FALSIFIED, do not build it.** The advice was to copy
`daemon_queue_remote_machine_refresh`: take the lock, do the cheap work, release,
run the slow half on a worker thread and answer early. **It does not cure this.**
The slow half here is `ensure_terminal_for_path_with_initial_size` + `persist`,
both `&mut DaemonRuntime` methods, so the worker thread must re-acquire the very
same global lock and holds it for exactly as long. An early `Ack` would improve
the *reveal's own* reported latency and leave the daemon deaf for the same 34 s —
and deafness is the defect, because the poll that carries the user's bytes is
what gets starved. `daemon_queue_remote_machine_refresh` works only because
`scan_remote_machine_refresh` genuinely needs no runtime; that is not true here.
⇒ **The cure is to make the work inside the lock small, or to give the runtime
finer-grained locks.** Everything below is the first half of that.

⭐ **THE FIRST HALF IS DONE (3.0.104), AND IT MOVED THE BIGGEST TERM.** `persist()`
was re-deriving every live Claude Code session's identity from scratch on every
call — a `/proc` subtree walk, a scan of every `~/.claude/projects/*` dir, and a
**full read + JSON parse of each transcript: 49.9 MB across 9 owned rows** on the
dev fleet host, inside the lock, on every state-changing request, growing all day
as the transcripts grow. The codex twin
(`refresh_live_codex_runtime_identities_for_persistence`) has been memoized by pid
since it was written; the CC one was added *as its mirror*, copied the loop, and
did not copy the cache. Now memoized and — unlike codex — revalidated before each
reuse against CC's own `~/.claude/sessions/<pid>.json`, because `/clear` rebinds a
live process to a new session id and `/cd` re-homes its transcript, and a blind
cache would point `claude -r` at the wrong one. Also landed: the 2.26 MB state
backup is a hard link instead of a full copy (−4.5 MB of I/O per write), and the
rename-failure fallback was fixed to stop writing *through* the state path, which
once linked would have rewritten the backup too.

**Where the 34.4 s goes** (both holes are inside the lock):
⛔ **AND THE 3.0.104 INSTRUMENT HAS NOW WEAKENED BOTH ATTRIBUTIONS BELOW.**
First numbers from 3.0.104+ daemons owning real sessions: `daemon/persist_state_only`
**p50 7.9 ms, max 11.4 ms** · `daemon_persist/serialize` **p50 3.8 ms, max 6.3 ms**
(for the whole 2.26 MB — so serialization was NEVER the cost this entry blamed it
for) · `daemon_persist/write` **p50 1.1 ms** · `daemon_persist/cc_identity_refresh`
**p50 25.0 ms, max 272.6 ms** (the memoized path; pre-fix it re-read 49.9 MB).
⇒ On a healthy daemon the whole persist is ~30 ms, against the 153.7 ms p50 this
entry measured before 3.0.104. **But a 9-second `persist_state_only` is nowhere in
that distribution**, so "the +9.4 s hole is the persist" — reached by ELIMINATION,
never by measurement — is now doubtful rather than supported. Either it needs a
condition these samples do not contain (cold cache, disk contention, lock
convoy), or it was never the persist. ⚠ Sample counts are small (n=18-36) and the
daemons are young. **Re-take both holes from the ranked rows on the next
occurrence; do not re-derive them by elimination.**

- **+9.4 s hole** — after `local_cc_relaunch_command_rebuilt` the only
  substantial call before `terminal_spec_resolved` is `persist_state_only()`.
  `server-state.json` is **2.26 MB**; `write_persisted_state_if_changed`
  serialises the whole state to pretty JSON *before* it can tell whether anything
  changed, then `fs::copy`s the old file to `.previous.json` and writes the new
  one — ~6.8 MB of I/O per persist. Measured `daemon/persist` p50 153 ms, p99
  **2,310 ms**, max **44,833 ms**. A 9 s persist is a p99+ sample, not an anomaly.
- **+23.9 s hole** — between `first_bytes` and `ensure_session_end`. The only
  substantial call there is a second `persist_state_only()`: the grid-mismatch
  branch did not fire (no `reattach_grid_resync`) and `forward_remote_pty_resize`
  spawns a worker rather than blocking. ⚠ Still attributed by ELIMINATION, not
  measurement — `persist_state_only` carried no perf span at all, which is the
  gap 3.0.104 closes.
- ⚠ **Both attributions above are pre-3.0.104 and must be re-taken.** The next
  occurrence can be ranked instead of argued: `daemon/persist_state_only`,
  `daemon_persist/cc_identity_refresh`, `daemon_persist/serialize` and
  `daemon_persist/write` are now separate rows in `server perf-summary`.

⛔⛔ **RE-TAKEN BY RANKING, 2026-08-11 — THE PERSIST IS EXONERATED AND THERE IS A
BETTER CANDIDATE.** Every span carrying a `duration_ms` in the three newest perf
files on `dev`, ranked by tail, and then filtered to the 8–11 s band the hole
occupies:

    daemon/persist_state_only          p50    7.5 ms   max     10 ms   n=20
    daemon_persist/serialize           p50    4.4 ms   max      6 ms   n=21
    background/local_tree_scan         p50 8,764 ms    max 11,155 ms   n=30
    daemon/background_copy_chore       p50   71.3 ms   max 11,155 ms   n=807
    daemon/runtime_load                p50 3,159 ms    max  8,494 ms   n=10

⇒ `persist_state_only` is **three orders of magnitude** away from a 9 s hole and
cannot be it under any sampling. The only spans that reach the band are
`background/local_tree_scan` and the chore that contains it — and they share
their durations exactly (9,379 · 9,274 · 8,931 · 8,540 · 8,509 ms appear in
both), so the chore IS the scan. Its shape fits the hole precisely: **median
8.76 s when it runs long, and ~19% of wall-clock time on `dev` is spent inside
one** (396 s of scan across a 2,075 s window, 723 samples).

⚠ **What this does NOT yet prove:** that a scan occupied *that* hole. A span of
the right size that recurs often enough to be likely is a candidate, not an
attribution — which is the exact error this re-take was ordered to undo. The
finishing move is an OVERLAP test: take a reveal's hole interval and ask whether
a `local_tree_scan` span was in flight across it, from the two `ts_ms` values.
Both are timestamped, so it can be answered without a new instrument.

⭐⭐ **THE SECOND LOCK HOG IS ROOT-CAUSED AND FIXED (3.0.109): `remove_session`
CALLS EVERY DEAD DAEMON ON THE HOST.** `close_live_session_row` →
`prune_unrepresented_preserved_owners` issues a **blocking `status()` socket
request to every other daemon owning a preserved runtime**, inside the global
runtime lock, once per distinct endpoint. Where deploys have left 24+ superseded
daemons alive, each dead one costs the full request timeout and the daemon serves
nobody meanwhile. ⇒ that is the p99 10.6 s / max 45 s, and **every agent row
claim performs a remove**, so it is on the owner's path too.
⭐ The negative cache that fixes it already existed twenty lines up —
`preserved_owner_for_runtime_key` has consulted
`preserved_owner_unreachable_until_ms` since the 2026-06-11 incident where one
`terminal_ensure` held the loop 30.7 s — and had never been applied here. Second
time in this campaign a mitigation failed to travel from one function to its
sibling (the first: CC vs codex identity memoization, 3.0.104), so the guard
walks BOTH probe sites.
⚖ Cost only, not outcome: a failed probe already yielded `None`, and skipping a
probe known to fail yields `None`, so every prune decision is unchanged.
⛔ **LIVE PROOF STILL OWED, AND THE ATTEMPT ON 2026-08-11 FOUND OUT WHY IT IS
HARD.** Two things a re-measurement must do, both learned by getting them wrong:

- **Measure on the right host.** The p99 blowup is a `dev` phenomenon and does
  not exist on the GUI host at all. Every daemon generation on the desktop host,
  before and after the fix, sits at p50 89–226 ms with a max of 531 ms across
  nine generations — because the cause is a fleet of superseded daemons to probe,
  and that is what `dev` has. A "no regression" reading taken there is measuring
  a host that never had the symptom.
- **⛔ Resolve the SERVING daemon's binary, because the sample mixes code
  versions.** On `dev` at 12:03–12:05, after 3.0.109 shipped, `remove_session`
  still returned 8,570 · 8,125 · 5,585 · 4,750 · 4,111 ms. Every one of those
  pids resolves to `/proc/<pid>/exe → …/yggterm-headless (deleted)` — a
  superseded daemon still serving pre-fix code, which is version coexistence
  working as designed, not the fix failing. **The perf record carries no version
  field**, so a host-wide percentile over a mixed fleet cannot attribute
  anything to either side. Take the sample from daemons whose `exe` is NOT
  `(deleted)`, resolved while they are alive.

⇒ the honest status is UNPROVEN, not "no change". The numbers below are the
pre-fix baseline.

**The original measurement, kept as the baseline:** The
3.0.103 instrument's first live harvest (dev, one daemon, 7.6 min) found five
starvation episodes ≥300 ms; the worst outside startup was **3,913 ms held by
`remove_session`**, with `terminal_read` parked behind it. Across the host
`daemon_request/remove_session` is p50 **447 ms**, p99 **10,590 ms**, max
**44,991 ms** (n=1,220) — the second-largest total of any daemon request after
`status`, and a far worse tail. In the sampled episode ~3.6 s sat between
`live_session_row_tombstoned` and `preserved_owner_registry_pruned`. **Not
attributed; not started.** Every agent row claim removes a row, so this is on the
owner's critical path too.

⛔ **FALSIFIED — "this reveal was a RELAUNCH, not a reattach" is WRONG.** The
birth is CORRECT: the row existed in the sidebar (scanned from JSONL) but had no
live PTY, so revealing it must create one — that is `claude -r <uuid>`, the
product's core value proposition, and `--require-existing` was passed.
⛔ **FALSIFIED — "236 `live_session_birth` in 75 s" is not phantom spawns.** They
belong to pid **1854247**, a *successor* daemon restoring rows from the ledger
(`live_row_order_restored_from_ledger`, then
`pre_daemon_swap_row_order_snapshot_written{reason:"superseded_daemon_takeover",
row_count:267}` at 21:22:49), all with `launch_now:false, activate:false`. A
daemon handover, not a spawn storm.
⚖ **A deploy WAS rolling** — `prepare_update_restart` walked ~24 daemons serially
21:22:56 → 21:23:12 — but the owning daemon went deaf at 21:21:32, **84 s before
that walk began**. The deploy did not cause this gap; it killed the daemon that
was already stuck in it. The serial supersede walk remains a separate real cost.

⭐ **Corroborated by the app's own notifications, and it rules out the resource
explanation.** His GUI at 22:58 on 2026-08-10 carried three `Slow terminal
reveal` cards — **38.8 s, 46.0 s and 64.0 s** — each of which already states
*"Memory is not the cause"*, with **8.7-9.3 GB of 15.1 GB free and the kernel
stalled on reclaim 0.00% of the time**. ⇒ The machine was not short of anything.
A daemon with RAM to spare, idle reclaim, and a 64-second reveal is a daemon that
is **blocked**, not loaded — which is what the lock measurement says.

⚠ **Trace hygiene defect found while measuring:** unit tests write into the live
`~/.yggterm/event-trace*.jsonl` — pids 3387039 / 3895848 emit
`live_session_birth` for `abc123`, `kept-samplenotes`, `synthetic-runtime`. Any
host-wide count over the trace is contaminated by test fixtures. Filter to the
owning pid, always.

⚠ **Method note, because this entry has now had three fixes proposed and two
retracted:** each "the fix is X" survived until one more layer was read — the
trace label (retracted), then the retry cap (retracted here). **Read the callee,
then the callee's bounds, BEFORE proposing.** The corrections cost less than the
implementations would have, but only because none was built.

⛔ **So the ceiling is MISLABELLED, not mistimed.** It is announced to the user as
*"did not become interactive in time"* at the exact instant it triggers the repair
that makes the terminal interactive 3 s later. Raising the ceiling makes the
product SLOWER (later repair); lowering it makes it faster. **The fix is to make
`no_output_stall` recovery actually attempt the restore that
`protected_runtime_careful_restore_after_timeout` does, and to stop calling a
successful repair a failure.**
⚠ Not the shape-B false toast (fixed, confirmed below): here the row WAS the
active terminal, so the stand-down correctly did not apply. Different bug, same
toast.

### ⛔ WHY THE FAR DAEMON WAS UNREACHABLE: A TURNOVER WALKS EVERY STALE DAEMON

The three `remote_pty_resize_failed` errors in that window are three DIFFERENT
faults, and the last two are the far host being unable to answer at all:

1. `terminal session not found: local://<id>` — ⭐ **a scheme mismatch.** The row
   is `remote-cc://dev/<id>`; the remote lookup asked for `local://<id>`. The
   live agent runtime on the far host is keyed `cc-runtime://<id>`. Same family as
   the resolver bug fixed in 3.0.101 — two ids, one name.
2. `reading daemon response … Resource temporarily unavailable (os error 11)`
3. `connecting to ~/.yggterm/server-3-0-<n>.sock … Connection refused`

Measured on the far host at the same moment: **one daemon turnover serially sends
`prepare_update_restart` to 24 stale daemons and takes 23 seconds** (21:22:49 →
21:23:13, `superseded_daemon_takeover` `prepared: 24`), and a third daemon spawned
into the same socket and lost the bind race (`bind_lock_busy`). While that walk
runs, those daemons are doing swap work instead of answering — which is the
`os error 11` above, and then a window with no listener on the version socket at
all, which is the `Connection refused`.

⇒ ⭐ **THE FAT TAIL ON REMOTE ROWS IS COUPLED TO THE STALE-DAEMON COUNT ON THE FAR
HOST.** Every turnover costs ~23 s of degraded responsiveness there, and the cost
scales with how many dead daemons have piled up (24 on that host). That links this
entry to the daemon-pile-up entry, which was being tracked as unrelated. ⚠ Any
deploy is a turnover, so an agent deploying while the owner is switching rows is
buying him a 60 s reveal — this one was measured ~90 minutes after a deploy, with
a second same-version supersede landing in the window.

⛔ **Memory is excluded by the instrument itself** on all three: 8,741-8,958 MB of
15,110 MB available, PSI `full avg60` **0.00%**, reclaim posture comfortable. A
DIFFERENT agent row in the same window took **38,135 ms** to first output, so it
is not one sick row. All four reveals are `remote-cc://dev/*`, i.e. the reach
into `dev` — where 24 replaced daemons were alive alongside the live one.
⇒ **That measurement is now DONE — see the trace above.** The 63 s is not spread
across ssh-connect/resolve/attach at all: it is 3 s of real work, a 45 s dead gap
caused by a one-shot recovery cap, and a repair that only starts when the ceiling
fires. **Do not re-measure it; fix the cap and the ensure postcondition.**
⚠ **A GUI restart takes 17-156 s to first answer, against the owner's 3 s bar.**
Measured on the same host: **18,473 ms**, **16,460 ms**, and then **156,524 ms**
— the last one bad enough that the owner intervened mid-restart (*"I had to
restart it seeing not restarting for so long"*). Not an outlier: a property with
a very long tail.
⇒ **CONFIRMED OWNER: `startup/initial_server_sync`.** On that 156 s restart it
recorded **37,297 ms** with `{"ok": true}` — it succeeds, it is just enormous.
Across the host: p90 **48,542 ms**, max **126,378 ms** (n=37). A WALL-clock span,
so unlike the render spans it may legitimately be placed on a timeline.

⇒ ⭐ **AND THIS IS WHAT "FLAWLESS RENDERING BUT UNTYPEABLE" IS.** The GUI paints
its restored snapshot immediately and only becomes *usable* when
`initial_server_sync` finishes. Everything on screen is real and correct; none of
it is connected yet. Any instrument that samples the painted surface will report
health throughout — which is why this symptom has survived so many "looks fine to
me" checks.

⭐ **THE STARTUP HALF IS ROOT-CAUSED AND FIXED IN CODE (3.0.97) — see CHANGELOG.**
`daemon_reuse` fires **9 ms** before the span ends, so 37,287 of the 37,296 ms
was spent before any data was fetched: a hot-update handoff the GUI (3.0.96)
requested from the 3.0.91 daemon, promising its own version while the child is
spawned from the neighbouring `yggterm-headless` — 3.0.92 on that host. The child
refused the regression and exited 1; the requester waited out every deadline in
the stack. **The refusal was correct; asking was the bug.** ⚠ The `daemon.log`
tail quoted in the failure text belonged to an unrelated 3-0-32 daemon from days
earlier, because the child's stderr went to `/dev/null` — both fixed.
⛔ **LIVE PROOF STILL OWED on the GUI host**, and it needs a 3.0.97 GUI binary
installed to BOTH `~/.local/bin/yggterm` and `~/.yggterm/bin/yggterm`.

⚠ **`daemon_request/hot_restart` is pinned at ~10.2 s** — 10,229 and 10,216 ms on
this restart, against p50 10,166 / max 10,441 across the host. **A near-constant
duration is a timeout, not work** (the constant-anomaly law). **Still open**: the
3.0.97 fix stops *doomed* handoffs from being requested at all, so this no longer
lands on the startup path — but a handoff that CAN succeed still pays the same
10.2 s, and nobody has yet found which deadline that is.

### ⛔ Why this was never investigated: the instrument blamed memory, every time

`swap_pressured()` is `swap_used_kb > 512 MB`. `swap_used` is a **history
counter** — the doc comment on `reclaim_pressured` in `terminal_observe.rs`
already said so, and the reclaim path was fixed to stop reading it. **The
notification kept reading it.** Across all 106 reveals `swap_pressured` was true
**106 times** — a predicate true on every sample discriminates nothing. Of the 21
reveals that took 6 s or more:

- `reclaim_pressured`: true on **0**
- PSI `full avg60` at or above the 10% thrash line: **0** (worst seen: 0.23%)
- memory available: median **9,362 MB of 15,110 MB**

So every "Free RAM to speed up reveals" notice fired on a machine with ~9 GB
free. ⇒ **A diagnostic that asserts an unmeasured cause is worse than silence: it
ends the investigation with the wrong answer.** Same family as
*a constant anomaly is a measurement bug*.

⭐ **FIXED in the notification:** it now reports the duration always, and names
memory as the cause only when `reclaim_pressured` agrees; otherwise it says what
was *ruled out* (`"Memory is not the cause: 9362 MB of 15110 MB available and the
kernel stalled on reclaim 0.00% of the time"`) so nobody re-chases it. Pinned by
`slow_reveal_with_stale_swap_but_free_memory_does_not_blame_memory` carrying the
live numbers, and its opposite so a genuinely short machine is still told.

### ⭐ THE SPLIT, DONE — the surface mounts fast; the WAIT FOR OUTPUT is the tail

`surface_mounted_ms` vs `first_output_ms` on the 22 slow reveals settles which
half is guilty, and it is not the GUI's surface:

| slow reveal | mount | then wait for first output |
|---|---|---|
| 6,507 ms | 166 ms | **6,110 ms** |
| 11,104 ms | 427 ms | **10,677 ms** |
| 35,497 ms | 244 ms | **34,516 ms** |
| 61,983 ms | 959 ms | **59,484 ms** |
| 12,058 ms | 153 ms | **11,673 ms** |

Mount is sub-second on 13 of the 15 slow reveals that recorded it (153-959 ms).
**Two exceptions worth their own look: 20,770 ms and 63,852 ms of mount.**
And **all 11 failures carry the same reason: `The live terminal on dev did not
become interactive in time.`**

⇒ The GUI paints its surface promptly and then waits on the REMOTE side. That
moves the question off the renderer and onto "why does a live terminal on the
remote host take 6-60 s — or forever — to become interactive", which is very
likely the same defect as § *A NEW SESSION IS ASKED FOR … AND NO PROCESS IS EVER
BORN* (his failed `start-cc` at 13:00:40 sits between failures at 12:59 and
13:02). **Treat them as one investigation until something separates them.**

⛔ **A TRAP THAT ALREADY CAUGHT ONE PASS — do not repeat it.** Correlating these
failures against concurrent perf spans appears to indict `render/gui` and
`render/web_content` (15-53 s, overlapping every failure). **That correlation is
void.** `render` spans carry **CPU milliseconds** in `duration_ms`, by design —
`perf.rs` says so and locks it with
`a_cpu_time_span_is_never_judged_by_the_wall_clock_rules`. Deriving a start as
`end - duration` fabricates a timeline for them, so any overlap computed that way
is an artefact. Only wall-clock spans may be placed on a timeline; of the ones
that overlapped, `daemon_request/terminal_restart` (45,051 ms) and
`daemon_request/hot_restart` (10,307 ms) are wall spans and remain real leads.

### ⭐ AND THE 11 FAILURES ARE TWO DIFFERENT BUGS, NOT ONE

First: **~60 s is the TIMEOUT, not the work.** `REMOTE_TERMINAL_RESUME_FAIL_MS`
is 60,000, and each deferral adds one 30 s
`REMOTE_TERMINAL_RESUME_OUTPUT_PROGRESS_GRACE_MS`. That decodes every failure
duration exactly — 60,003 / 60,095 / 60,152 / 61,173 / 62,098 / 63,691 / 63,837 /
68,323 are the bare ceiling, and 97,950 / 122,174 are the ceiling plus deferrals.
⇒ **Nobody should read those numbers as "it took 60 seconds".** They mean "it
never arrived, and this is when we stopped waiting."

Split them by whether output ever came, and they are clearly two populations:

| shape | n | `first_output_ms` | what it means |
|---|---|---|---|
| **A — silent** | 5 | `None` | the remote PTY produced NOTHING for 60 s+ |
| **B — spoke, then never readied** | 4 | 2,279 / 3,105 / 5,004 / 27,039 | output flowed within seconds and the readiness gate still never latched |

### ⭐⭐ SHAPE B, ROOT-CAUSED: THE SPAWN IS SKIPPED AND NOBODY TELLS THE WAITER

Read from the trace, not inferred. The contrast is the whole proof:

| | reveal that readied (11:19) | reveal that never readied (12:39) |
|---|---|---|
| +0.0 s | `bootstrap_spawn_**scheduled**` | `bootstrap_spawn_**skipped**_existing_lease` |
| +3.8 s | — | `bootstrap_reset` → `bootstrap_spawn_**skipped**_inactive_retained_host` |
| `js_ready` / `paint_ready` | **+0.5 s** | ⛔ **never** |
| ends | ready | `reveal_failed` at +63.8 s |

⇒ `terminal_session_should_bootstrap_host` returns false when the session is not
the active one, and the skip branch **writes a trace event and nothing else** —
it never touches the in-flight `terminal_open_attempt`. So the attempt waits for
a mount that has been decided against, and the only thing that can end it is the
60 s ceiling. **The spawn decision and the wait have no shared owner.**

⚠ **`first_output_ms` on these is NOT the session speaking.** At +2.3 s the trace
shows `forward_protocol_only_output`, and
`mark_terminal_open_attempt_first_output_for_session` stamps `first_output_at_ms`
**before** it looks at the `protocol_only` flag. So our own protocol chatter from
the retained host is what made a never-mounted attempt look like it had spoken —
the same shape as the OSC-7717 heartbeat that pinned the idle gate shut. The
struct already carries `first_protocol_only_output_at_ms` and a separate
meaningful-output latch, so the distinction exists and this reader ignores it.

⇒ **This also explains the 15:01 burst of three simultaneous failures.** At most
one session can be the active one; three rows revealing at once means two get
skipped, and both then sit until the ceiling.

⛔ **The near-miss worth reading before fixing:** the doc comment on
`terminal_session_should_bootstrap_host` records that this exact class was hit
before ("the session did not mount" bug) and fixed by WIDENING the activeness
test to accept either the live value or the render snapshot. That made the skip
rarer; it did not make the skip tell the waiter. **Widening a predicate is not
the same as closing the hole it falls through.**

⭐ **SHAPE B IS FIXED IN CODE (3.0.98, completed in 3.0.100) — see CHANGELOG;
live proof still owed.**
The skip branch now resolves the in-flight attempt instead of abandoning it, and
the three behaviour calls that were open are settled:
1. **Cancel, not fail.** A cancelled reveal gets its own attempt state and its
   own trace event (`reveal_cancelled`), because the failure log's only question
   is "did the remote side fail to answer" and a switch-away is not that.
   ⇒ To reverse: drop the `Cancelled` variant and route the skip through
   `fail_terminal_open_attempt_for_session` instead.
2. **After a 1.5 s grace, not on the edge** (`INACTIVE_BOOTSTRAP_SKIP_CANCEL_GRACE_MS`).
   The activeness test can read false transiently for the session the user JUST
   clicked — that is the near-miss above — so cancelling on the first false
   reading would cancel the reveal it exists to protect. The guard is re-asserted
   inside the cancel, not trusted from the caller. ⇒ To reverse: set the constant
   to 0 for an immediate cancel, or raise it to forgive a longer glance-away.
3. **The toast is suppressed at the timer, not at the attempt.** The 60 s timer
   raises the toast itself and only then latches the failure, so gating on
   attempt state would not have stopped it; it now stands down on
   `terminal_resume_timeout_should_stand_down` and clears any stale resume
   notice. A cancelled attempt counts as ABSENT to the next bootstrap, so the
   next reveal begins a fresh attempt and keeps its own ceiling.

⚠ **AND THE TRACE HELD A SECOND POPULATION NOBODY HAD SPLIT OFF** — found while
taking the before/after baseline on the GUI host, 2026-08-10 18:20:38 and
17:00:12. A session that never had an open attempt at all (created by
`terminal new`, never active) reaches the same ceiling: `resume_timeout_cleared_
inflight` at exactly 60 s with **no `reveal_failed` beside it**, because the
failure latch found nothing to latch — and the toast fires anyway, since the
timer raises it before calling the latch. ⇒ **`reveal_failed` UNDERCOUNTS this
bug**; grep `resume_timeout_cleared_inflight` as well, or the population is
invisible. The stand-down covers it (no attempt + not the active terminal) and
still runs the whole cleanup.

**LIVE ON THE GUI HOST, 2026-08-10 (3.0.100 deployed to both guihost paths, GUI
restarted, 46→46 rows preserved):**
- ✅ **Deploy + faithful render.** A shadow-client reveal on the 3.0.100 binary
  paints a full, correct terminal (scrollback + live prompt, no squish/blank/
  broken-bottom); metadata pane reads `Client 3.0.100 · daemon 3.0.91`. The
  reveal-path change did not break the normal reveal.
- ✅ **No fast-path regression, no false cancel, no false toast.** Multiple
  live switch-aways (reveal A, immediately reveal B) on the shadow: every
  abandoned-but-already-ready session resolved `reveal_ready`, the harmless
  *outgoing*-session `bootstrap_spawn_skipped_inactive_retained_host` triggered
  **zero** `reveal_cancelled` and **zero** `reveal_failed`, and the shadow
  raised **0** notifications. The `ready_at_ms` / active-session guards hold on
  the live binary.
- ✅ **THE IN-FLIGHT ABANDON IS NOW LIVE-CAUGHT — 2026-08-10 21:31, on the GUI
  host, in the owner's own use.** It was filed as unmanufacturable (on a healthy
  fast `dev` the daemon retains every host, so a reveal readies in ~1.1 s, before
  the skip fires) and the prediction was that the trace would confirm it the next
  time a slow resume was abandoned. It did, unprompted, and the whole chain is in
  one 59-second window:

      21:31:02  bootstrap_skip_cancelled_open_attempt   ×2
      21:31:10  bootstrap_skip_cancelled_open_attempt
      21:32:01  resume_timeout_stood_down

  The GUI's own reveal log matches those to the millisecond — three entries whose
  `failure_reason` is *"the reveal was cancelled: this session stopped being the
  active terminal before its host could mount"* (`total_ms` 3,133 / 4,519 /
  9,132; two with **no first output at all**, i.e. genuinely mid-flight, which is
  exactly shape B). **The 60 s ceiling then arrived and STOOD DOWN** rather than
  raising *"did not become interactive"*. Fleet totals on that host:
  **13 `bootstrap_skip_cancelled_open_attempt`, 5 `resume_timeout_stood_down`,
  16 `reveal_cancelled`** — and `reveal_cancelled` is not counted as
  `reveal_failed`, so the toast population is unaffected.
  ⇒ 3.0.98 + 3.0.100 are confirmed on the path they were written for.
  ⭐ **The capture came from the OWNER sending a screenshot of his notification
  panel**, not from an agent probe — the surface an agent never looks at was
  holding the evidence for an entry marked unprovable. See the reveal-log
  instrument below; it is `server app state` → `reveal_log`, and it is far more
  legible than the trace (label, `first_output_ms`, `total_ms`, failure reason,
  memory posture, per reveal).

⇒ **Shape A is the one that most likely IS the owner's "new session never
starts"**: no process, no output, no error — the same silence. **It is
untouched by the 3.0.98 fix** — shape A never spoke at all, so there was no
skipped bootstrap behind it; do not read a quiet cancel as evidence about it.

⛔ Do not start from the memory angle again; it has been measured and excluded.

**Falsifier:** cold-tier p90 under 3 s and zero `reveal_failed` over a comparable
sample on the live host.

## ⭐ A NEW SESSION'S ROW IS NAMED AFTER THE ROW YOU RIGHT-CLICKED

**Status:** FIXED IN CODE — LIVE PROOF OWED

**Reported 2026-08-10, verbatim:** *"spawning a new session should say
New {Claude, Codex, …, Terminal} Session, etc. on start instead of adding some words on
the spawnee session row on which right click context menu was issued. They should then
change the name as they get their first title by whatever mechanism for each entry."*

**Confirmed in his screenshot:** a freshly spawned Claude Code session
(`0eb607df-…`, its own uuid, its own PID) shows
`Title: 6. optimization: finish the pass — guihost fan 24x7 + jank claude-code` — **my
row's title**, because the context menu was opened on my row. Two sidebar entries then
read almost identically, which is how a 40-row sidebar stops being a working
instrument.

**Wanted:**
1. On birth a row is named for WHAT IT IS — `New Claude Session`, `New Codex Session`,
   `New Terminal`, per kind — never derived from the row the menu was invoked on.
2. It renames itself when its first real title arrives, by whichever mechanism owns
   titles for that kind (the CC/codex title sync, the app declare, etc.).

⚠ Related but NOT the same defect: the ` claude-code` suffix seen on a superseded row
earlier the same day, and the duplicate `local://<uuid>` twin births
(§ *the untouchable row*). All three make the sidebar unreadable; only this one is
about the name chosen AT BIRTH.

**The cause, and the fix (3.0.113).** One composer:
`group_session_title_hint` returned `format!("{} {}", row.label, slug)` — the
label of whichever row the context menu was opened on, plus the CLI slug. It was
written for a *group* row, where `row.label` is a folder name and
`"widgets codex"` reads sensibly, and nothing stopped it being handed a session
row — which is what the menu people actually use does. The naming rule is now
`new_session_birth_title` in `yggterm-core`, beside the registry that knows every
CLI's display name, and it is derived from the new row's own kind: `New Codex
Session`, `New Claude Code Session`, `New Terminal`. Wanted-2 (rename on first
real title) already held — the placeholder is replaced by whichever mechanism
owns titles for that kind (`TitleAuthority`), which is why this only ever showed
for the few seconds before the CLI titled itself.

⚠ **A shell keeps being titled by its cwd**, which is a separate deliberate rule
(`live_session_default_title`) and not spawner-derived, so it is untouched.

**Falsifier:** right-click a *session* row, start a session of any CLI, and the
new row reads `New <CLI> Session` — never a variant of the clicked row's title.

⚠ **THAT FALSIFIER IS THE ONLY THING STILL OPEN HERE, AND IT CANNOT BE DRIVEN.**
The right-click path needs the sidebar row menu, and no app-control verb raises
it (§ *no app-control verb raises the sidebar row menu*) — so this stays
`LIVE PROOF OWED` on that entry's account, not on its own. **The re-report that
generalised this to "a freshly spawned row is named after whoever spawned it"
is closed:** the agent-plane birth title was root-caused and live-proven on the
desktop host at 3.0.120, three rows created in one command with their purposes
ending `A`, `B` and `C`, all three rendering their purpose (CHANGELOG 3.0.120).

⚠ **THREE SIBLING COMPOSERS SURVIVE, AND THEY ARE LATENT — NOT FIXED, NOT THE
REPORTED SYMPTOM.** `terminal_launch_context_for_row` and its `active_session`
twin still build `format!("{} terminal", row.label)`,
`format!("{} terminal", active.title)` and `format!("{} ssh", row.label)` — the
same spawner-derived shape, in the plain-terminal paths.

They do not produce the reported symptom because every one of them is a
`Shell`/`SshShell` launch, and `live_session_default_title` (yggterm-server)
titles those kinds by their **cwd**, using the hint only when the cwd is empty —
which these paths always set. That is also why the report named
`{claude-code,codex,…}` rows specifically: an agent kind KEEPS the fallback, so
only there did the spawner's name reach the sidebar.

⇒ **Left standing deliberately, and recorded rather than silently fixed**, for
two reasons: they are unreachable in normal use, and the fallback they carry is
*descriptive* (`dev ssh`) where a generic `New Terminal` would be less useful in
the one case that reaches it. **If the empty-cwd case is ever hit, this is the
entry it belongs to.**

## ⛔ APP-CONTROL CANNOT TYPE INTO THE SHELL'S OWN CHROME

**Status:** OPEN

*found 2026-08-13, blocking live proof of the start page's search box*

There is no verb that puts a keystroke into a Dioxus shell DOM field — a search
box, a rename field, a settings input. The neighbours all answer a different
question:

- `server app terminal probe-type` / `terminal send` target a **PTY**, not the
  chrome around it.
- `server app web find --text` and the `wpe` verbs drive a **web surface** (a
  contributed app's page), not yggterm's own shell.
- `server app click` reaches a coordinate, which can focus a field but cannot
  put text in it.

⇒ **A shell-chrome affordance can be proven to RENDER and not to WORK**, which
is precisely the gap that left the start page's search at "box is live, filter
is unit-tested" instead of live-proven. Any future chrome input — and the
campaign is adding them — inherits the same hole.

⚠ It also means the field guide's rule "for a visual bug the proof is a faithful
screenshot" has no counterpart for an INTERACTIVE chrome bug: there is no probe
that exercises one.

**Wanted:** `server app chrome type <selector-or-testid> <text> [--pid]` and a
matching read-back, keyed on the `data-yggterm-*` attributes the chrome already
stamps (the start page search carries `data-yggterm-start-page-search`, and its
result count carries `data-yggterm-start-page-recent-count`, precisely so a probe
could assert the pair).

**Falsifier:** typing a word that appears only in one session's generated summary
leaves exactly that card standing, asserted from the CLI without a human.

## ⛔⛔ THE REMOTE HELPER DIALS A SOCKET NAMED FOR ITS OWN VERSION — SO UPGRADING IT ALONE KILLS EVERY REMOTE COMMAND

**Status:** OPEN

**Caused by me, 2026-08-10, and it took the owner's "even a new session does not want to
start" from a symptom to a total outage.** Recorded because the trap is structural, not
a slip.

To fix image paste I set `dev`'s `~/.yggterm/bin/yggterm` to **3.0.92** while
deliberately leaving its daemon at **3.0.91** (replacing a running daemon's binary arms
the cold-shutdown cascade — see the deploy entry). The remote helper then dialled the
socket named for **its own** version:

```
remote_pty_resize_failed
  remote yggterm command failed for dev:
  Error: connecting to $HOME/.yggterm/server-3-0-92.sock
  Caused by: No such file or directory (os error 2)
```

`server-3-0-92.sock` does not exist — the running daemon binds `server-3-0-91.sock`.
⇒ **Every remote command to that host failed**: `terminal resize`, `remote start-cc`,
everything. A new row was created, reported `queued:true` / `launch.applied:true` /
`running · idle`, and **no process was ever spawned** — verified by the absence of ANY
ssh bridge for it on the GUI host while three other rows had theirs.

**Measured before/after the revert, same verb, same host:**

| | helper 3.0.92 vs daemon 3.0.91 | helper reverted to 3.0.91 |
|---|---|---|
| `terminal input-check` | `consuming_input:false` after **35,000 ms** | `consuming_input:true` in **249 ms** |
| processes on the remote | none | `start-cc` + wrapper + `claude --session-id` |

⇒ **VERSION IS THE RENDEZVOUS KEY** ([[finding-version-string-as-rendezvous-key]]), and
it binds the helper to the DAEMON, not just to the protocol. So the helper and the
daemon on a host are a matched pair that cannot be upgraded independently — which
collides head-on with the paste bug, whose fix wanted the helper upgraded alone.

**Fix directions:**
1. **The helper should discover the daemon**, not assume its own version's socket —
   fall back to the newest reachable versioned socket it is compatible with. That
   decouples the pair and makes a mixed fleet work, which the constitution requires.
2. Failing that, **refuse to install a helper whose version has no daemon**, loudly, at
   install time — rather than succeeding and breaking every later command.
⚠ Either way the error must name the pairing. *"No such file or directory"* on a socket
path is a true statement that tells the reader nothing about what to do.

## ⛔ FIVE PRE-3.0 DAEMONS STILL WALK THE WHOLE TRANSCRIPT CORPUS, AND A DEPLOY CANNOT REACH THEM

**Status:** AWAITING A DECISION

**Decided by:** the owner — only he can say whether the sessions these daemons
still hold may be dropped.

⭐ **The startup half of this entry is FIXED and proven — see CHANGELOG 3.0.94**
(daemon time-to-first-answer 15.1 s → 0.14 s). What is left is the part a
deploy cannot fix, because the daemons in question run frozen old binaries.

### What was actually wrong, and how the first diagnosis missed it

The original entry accused `daemon_copy_chore_should_scan_local_tree` of failing
to gate the walk. **That gate is correct and was never the thing running** —
falsified against the live perf stream, not argued: across the last ~7 h of
`perf-telemetry`, **every** `daemon/background_copy_chore` record that carries
meta reads `local_tree_scanned: false` (8,723 of them), while
`background/local_tree_scan` fired **385 times** in the same window at p50 8.3 s.
A gate reporting "I did not do it" 8,723 times next to 385 walks means a
**different caller**, and the pid field said which: 351 of those 385 walks were
written by a process **emitting no `pid` at all** — the field was added
2026-07-26, so the writer predates it.

⇒ Sampling every daemon's open fds for 60 s named them. Five daemons hold the
whole `~/.claude/projects` tree open for ~40-50% of wall-clock, 100-150 distinct
files each; the current 3.0.91 daemons hold **one** file each, which is the
legitimate per-owned-session read.

| pid | socket | started | still owns |
|---|---|---|---|
| 1412446 | `server-2-11-1` | Jul 13 | 0 sessions |
| 3752038 | `server-2-12-2` | Jul 22 | 0 sessions |
| 122902 | `server-2-12-5` | Jul 23 | 25 (18 CC, 7 shell) |
| 776952 | `server-2-12-8` | Jul 23 | 40 (31 CC, 9 shell) |
| 219756 | `server-2-12-14` | Jul 26 09:27 | 73 (55 CC, 18 shell) |

The gate shipped in 38885207 at **2026-07-26 10:46** — 79 minutes after the
youngest of these started. Every one of them predates it and always will.

**The user-visible cost, measured**: polled `server status` against each once a
second for 3 minutes. p50 is 0.01-0.02 s, but the maxima are **6.78 s, 4.26 s and
7.79 s** — those are the windows in which that daemon holds its runtime lock
across the walk (the lock was moved off the walk in a3f58fb7, also after they
started). The current 3.0.91 daemon's max in the same window was **0.07 s**.

### Why it cannot be fixed by shipping

A 3.0.x client connects to `server-3-0-<n>.sock` and only falls back to an older
daemon when its own socket is unreachable. These five are therefore **invisible
to the current GUI** — their rows cannot be opened, resumed or reaped through the
product, yet they keep re-reading a 17.8 GB corpus forever.

⇒ **The decision that is owed:** pids 122902, 776952 and 219756 still claim 138
sessions between them, most `keep_alive: true` — but **none of them has a live
grandchild process**, so every agent under them has already exited and what
survives is the registration plus an idle `bash -i`. Killing the two with **zero**
sessions (1412446, 3752038) is free. Killing the other three drops rows the owner
cannot see or reach anyway; it is still his call, because "unreachable" is our
inference and the scrollback is his.

**RECOMMENDATION**: reap all five, oldest first, after capturing each one's
`server status` JSON to `~/.yggterm/manual-snapshots/`. **What was done meanwhile:**
nothing was killed — the measurement is filed and the startup fix shipped, which
is the half that does not need him.

**Falsifier for the fix that DID ship:** a daemon started from 3.0.94 must answer
`server status` in well under a second on this corpus. Measured: 0.14 s, three
runs, against 15.1 s for 3.0.91 on the same machine and the same corpus.

⚠ **Do not "fix" this by widening the gate's env check** until candidate 1 is settled —
the gate may be correct and simply not the thing running.

## ⛔⛔ EVERY GUI DEPLOY BREAKS IMAGE PASTE TO EVERY HOST YOU HAVE NOT UPDATED YET

**Status:** OPEN

**Reported 2026-08-10** — *"now I cannot ctrl+V screenshots"*. Found in the GUI's
own launch log, which names it exactly:

```
WARN terminal clipboard paste request  session=remote-cc://dev/<uuid>
WARN terminal clipboard paste failed   session=remote-cc://dev/<uuid>
     error=remote yggterm protocol mismatch for dev:
           expected 3.0.92@12072927657283082749, got 3.0.91@12072927657283082749
```

⚠ **TEXT paste kept working** (`terminal clipboard text paste accepted` in the same
log) — only the IMAGE path does a remote handshake, which is why this reads as "paste
is broken" rather than "the remote is stale".

### The mechanism

`remote_descriptor_is_protocol_compatible` (`lib.rs:15609`) is `remote >= local` on the
parsed version triple. The GUI is by construction the FIRST thing updated (he restarts
it; the daemons and remote helpers lag deliberately, per the constitution's
version-coexistence guarantee). So **the moment the GUI moves, every remote that has
not moved yet fails the test**, and image paste to those hosts dies until each one is
chased. A guarantee that old and new must coexist, and a check that forbids it.

### ⛔ The obvious fix is NOT yet justified — measured, do not skip this

The two sides reported the **same `build_id`** (`12072927657283082749`), so "accept when
the build ids match" looks right. **It is not established.** Probing both binaries
directly:

```
.yggterm/bin/yggterm            -> {"build_id":12072927657283082749,"version":"3.0.92"}
.yggterm/bin/yggterm.rollback   -> {"build_id":12072927657283082749,"version":"3.0.91"}
```

Two genuinely different builds, same id. So `build_id` here is **not** a content hash
(despite `current_local_build_id()` hashing binary bytes — a different notion under the
same word), and it is **not** `STAMPED_SHAPE_HASH` either (12005138361312769415 ≠
12072927657283082749). Until somebody establishes what this id actually denotes,
"equal id ⇒ wire compatible" is an assumption, and weakening a compatibility guard on
an assumption is how a mixed-version fleet starts failing silently.

### ✅ ANSWERED 2026-08-10 — `build_id` DESCRIBES A DIFFERENT FILE THAN THE BINARY ANSWERING

`run_remote_protocol_version` (`lib.rs:16803`) emits
`{"version": SERVER_PROTOCOL_VERSION, "build_id": current_local_build_id()}`, and
`current_local_build_id()` hashes `local_remote_bootstrap_executable()` — the
neighbouring **`yggterm-headless` bootstrap payload**, *not* the running binary.

**Decisive experiment** — two byte-identical `yggterm` 3.0.92 binaries, differing only
in which `yggterm-headless` sits beside them:

```
~/.yggterm/bin/yggterm  (neighbour headless 3.0.91) -> {"build_id":12072927657283082749,"version":"3.0.92"}
~/.local/bin/yggterm    (neighbour headless 3.0.92) -> {"build_id":12300672197703280345,"version":"3.0.92"}
```

⇒ **The descriptor pairs one fact about ME with one fact about MY NEIGHBOUR.** The
matching ids in the owner's paste failure were a coincidence of the shared neighbour,
not evidence of compatibility — so "accept when build ids match" would have compared
the wrong file entirely. Another member of the instrument family: *this probe answers a
different question than its name suggests.*

⚠ `build_id` should still be RENAMED to `bootstrap_payload_id`: a field called
`build_id` sitting beside `version` reads as "the build of the thing that just
answered", and that is false.

### ⛔ CORRECTION — THE VERSION CHECK IS NOT THE BUG. BOOTSTRAP IS.

**Earlier in this same session I filed "the fix is a declared MINIMUM COMPATIBLE
VERSION". That is WRONG**, and recorded here rather than quietly deleted, because
leaving it would send the next session down a dead path.

The version ordering ALREADY has the right escape, and the design comment above
`remote_descriptor_is_protocol_compatible` states the intent: *"an OLDER remote falls
through to bootstrap, which is a benign monotonic upgrade rather than a bail."* And it
does — `is_remote_protocol_probe_recoverable("3.0.91")` returns **true** via
`looks_like_version(text) && text != SERVER_PROTOCOL_VERSION` (`lib.rs:15581`). So
neither probe site (`lib.rs:16059`, `:16100`) bails on a merely-older remote.

⇒ **The bail the owner hit can only be the THIRD site, `lib.rs:16118` — the POST-bootstrap
re-probe, which has no recoverable escape, correctly: if bootstrap just ran and the
remote is STILL old, something is genuinely wrong.**

⇒ **So the real defect is that `bootstrap_remote_yggterm` ran against the remote and
did not upgrade it.** The version mismatch was the symptom, reported honestly.

### ✅ CHAIN CLOSED — THE GUI CHECKS ITS OWN VERSION BUT SHIPS ITS NEIGHBOUR'S BINARY

`bootstrap_remote_yggterm` (`lib.rs:15782`) resolves
`local_remote_bootstrap_executable()` — **the GUI host's `yggterm-headless`** — and
uploads THAT to both remote paths (`$HOME/.yggterm/bin/yggterm` and
`…/yggterm-headless`). Measured on the live host at the time of the failure:

```
guihost GUI      = 3.0.92     <- the version the compatibility check demands
guihost headless = 3.0.91     <- the payload it actually ships to remotes
```

⇒ The GUI demanded 3.0.92, bootstrapped `dev` with **3.0.91**, re-probed, found 3.0.91,
and bailed. Every step honest; the two facts simply came from **two different files**.

⛔ **THE STRUCTURAL BIND, and it is why this is not a one-line fix.** The payload the
GUI bootstraps with IS ALSO the running daemon's binary. Replacing it to keep the two
in step is exactly the act that renames a live daemon's `/proc/self/exe` and arms the
cold-shutdown cascade (see the deploy entry above). So "just keep them matched" trades
one outage for another.

**Fix directions, none free:**
1. ✅ **DONE 3.0.93 — the check now asks the PAYLOAD's version, not the GUI's.**
   `local_bootstrap_payload_version()` interrogates the binary the bootstrap would
   actually upload, falling back to our own version when it cannot be read (i.e. the
   previous behaviour). The decision is split into a pure
   `remote_version_is_compatible_with_payload` and pinned by a test carrying the live
   failure: payload 3.0.91 + remote 3.0.91 must be COMPATIBLE, while remote 3.0.89
   must still bootstrap. ⇒ The unreachable success condition is gone.
2. **Ship the payload as a separate versioned file** the daemon never executes, so it
   can be replaced at any time without touching a running process.
3. Keep them matched and solve the running-daemon replacement properly (that is the
   constitution's lane, not this one).

**Immediate state:** dev's `~/.yggterm/bin/yggterm` was set to 3.0.92 by hand, so the
first probe now succeeds and bootstrap never runs — his paste works. ⚠ This is a
patched symptom on ONE host; any other remote the GUI has outrun will hit it again.

**What was done meanwhile:** dev's `~/.yggterm/bin/yggterm` was brought to 3.0.92 to
match the GUI, which makes the existing test pass deterministically. Nothing was
weakened. **To reverse:** `~/.yggterm/bin/yggterm.rollback-3.0.92-pre` is in place.

## ⛔⛔ A DEPLOY ARMS THE OLD DAEMON'S COLD SHUTDOWN, WHICH MASS-RE-RESUMES EVERY AGENT ~5 MINUTES LATER

**Status:** OPEN

**Reported 2026-08-10** — *"while you were idle, you were untypable for half an
hour"*, and a session restart he attempted came back failed. **Caused by this
session's own deploy**, and the trace names every step:

```
12:27:36 → 12:31:52  pid 1751132 (3.0.89)  daemon_self_retire ×12,
                     each deferring: blockers = recently_active, idle_ms 50s → 288s
12:31:54  pid 1751132  run_end                ← COLD SHUTDOWN
12:32:42  pid 2785975  live_session_birth     ← fresh daemon re-resumes everything
12:32:47  live_session_birth
12:34:15  live_session_birth
```

**The mechanism, and none of it is a malfunction.** Deploying 3.0.91 gave the running
3.0.89 daemon a strictly-newer sibling, so `retire_trigger = newer_daemon_live` fired.
It had no lossless handoff (that ships in 3.0.90+), so its only exit was the cold
shutdown — which the code documents as *"kills this daemon's PTY children and makes
the next client recovery-spawn a daemon that RE-RESUMES every agent on a fresh
PTY"*. The idle gate held it off for five minutes and then correctly opened, because
the sessions really were idle past 300 s. **Idle is not the same as unattended**: the
owner was reading, and every agent row on the host was re-resumed under him.

⇒ **A deploy therefore has a DELAYED blast radius nobody watches for.** The hot-restart
at deploy time looks clean and preserves sessions; the damage lands ~5 minutes later
when the superseded daemon finally clears its gate. Anyone who checks right after the
deploy sees success.

### What to change

1. ⭐ **A superseded daemon whose successor can ADOPT should never reach the cold
   shutdown.** 3.0.90+ has `spawn_superseded_self_retire_sweep` (lossless fd handoff,
   exits only on `AllMoved`) — the gap is that a PRE-3.0.90 daemon cannot use it, and
   the fleet is full of them. Options: teach the successor to PULL from a superseded
   predecessor, or accept that the pile must age out and never deploy onto a host
   still carrying old daemons that own live rows.
2. **The idle gate's 300 s is a proxy for "nobody is looking", and it is a bad one.**
   A row the user has open and is reading is idle. Consider gating on the ACTIVE row
   / a viewer being attached, not only on output silence — the same
   classification the settled relay-gate design already calls for
   (`docs/spec-hot-restart-relay-gate.md`).
3. **Deploy discipline meanwhile:** one **DAEMON-BINARY** deploy per session, and
   watch the trace for `daemon_self_retire` on the superseded daemon for the
   following ~6 minutes rather than declaring the deploy done when the hot-restart
   returns.
   ⛔ **This does NOT gate a GUI restart or a GUI-binary install** — those have
   none of this blast radius, and the owner has settled that they need no
   permission (`settled-calls.md`). A session read this line as covering the GUI
   and stopped restarting; he had to ask why. Say DAEMON, every time.

⚠ **Do not "fix" this by making the gate never open** — that is the immortal-daemon
bug this campaign just spent a day removing.

## ⛔⛔ A PLAIN SHELL IS NOT GETTING ITS PERMANENT BLOCKER, AND 3.0.90 REMOVED THE ACCIDENT THAT WAS COVERING FOR IT

**Status:** OPEN

⛔ **BLOCKS BROAD DEPLOYMENT OF A 3.0.90+ DAEMON ONTO SHELL-OWNING HOSTS.** Found
2026-08-10 by checking the blast radius of this session's own OSC-heartbeat fix,
rather than by a report.

### The measurement

`server status --endpoint 1837801` (v2.12.24, owns **9 plain `bash -i` shells**):

```
ver 2.12.24  owns 9 shells;  permanent_blocker_count = 0
  local://2fd6638d…  -> NO BLOCKER
  local://808d6ea7…  -> recently_active perm=False
  … 6 × recently_active …
  local://cf3c17a6…  -> NO BLOCKER
  local://f42611f7…  -> NO BLOCKER
```

**Every one of those nine should be `not_restorable, permanent=true`.** The cold
shutdown path says so in its own comment: *"for a session that CANNOT be re-resumed —
a plain shell — 'once idle' is not a safe moment, it is just the moment we would have
destroyed it. Those defer permanently, override or not."* And
`session_kind_state_survives_pty_loss(kind) = kind.is_agent()` (`daemon.rs:14278`) is
the correct rule. So the RULE is right and its INPUT is wrong:
`live_session_kind(key)` must be returning an agent-ish kind for these shell rows,
because a `None` would `.unwrap_or(false)` into the permanent blocker and be SAFE.

### ⛔ Why this is urgent now, and it is this session's own doing

Six of the nine were held only by `recently_active` — **the ychrome OSC heartbeat**,
i.e. an ACCIDENT, not the designed protection. 3.0.90 correctly stops that heartbeat
from bumping the idle clock (see the daemon-pile entry). ⇒ On a 3.0.90+ daemon in the
same situation **all nine would read NO BLOCKER**, and the cold-shutdown retire —
which explicitly *"kills this daemon's PTY children"* — becomes free to fire on plain
shells that cannot be brought back.

That is **the 3.0.81 bug returning through a side door**, and it can take a background
ychrome with it, which is the owner's hard constraint #1 (*"suppose I have a youtube
playlist running in background while I work here"*).

⚠ **Not yet observed firing.** No 3.0.90+ daemon has owned a plain shell yet — the
current one owns 6 agent rows and blocks correctly on the 4 that are working. So this
is a demonstrated GAP plus a mechanism, not a demonstrated incident. Do not soften it
on that account: the gap is measured and the mechanism is read from the code.

### What was done about it meanwhile

The 3.0.92 daemon was deliberately **NOT** deployed onto any running daemon path.
`~/.local/bin` (which is what `PATH` resolves) was brought to 3.0.92 fleet-wide;
`~/.yggterm/bin/yggterm-headless`, which the live daemons run from, was left at 3.0.91
on all three hosts. That was originally to avoid disturbing sessions; it is now also
the thing keeping this gap out of the fleet.

### Fix direction — ⭐ RECOMMENDED SHAPE, and why it was NOT shipped blind

**Narrowed 2026-08-10.** Current code is SAFE in two of the three cases:
`live_session_kind` returning `None` → `.unwrap_or(false)` → permanent blocker ✓;
`Some(Shell)` → `is_agent()` false → permanent blocker ✓. The ONLY unsafe case is
`Some(<agent kind>)` for a runtime that is actually a `bash -i` — i.e. **the row
registry disagreeing with the process**. That is what the 2.12.24 measurement shows;
it is NOT yet demonstrated on current code, because no 3.0.90+ daemon has owned a
plain shell.

⭐ **Recommended:** verify the registry against process truth using the discriminator
that already exists and is tested — `session_tenancy`'s `command_is_shell` /
`oldest` (the oldest NON-shell tenant, `session_tenancy.rs:955-963`). A runtime whose
tree contains **no non-shell tenant** is a plain shell whatever the registry says.
Reuse it; do not invent a launch-command matcher.

⛔ **Why it was not shipped in the same session that found it.** The failure is
BIDIRECTIONAL. Marking a real agent row "not restorable" pins its daemon **permanently**
— which is precisely the immortal-daemon bug this campaign spent the day removing. An
agent row whose CLI has exited leaves only its wrapper shell and would trip exactly
that. So the change needs a grace period and a live case to test against, and there is
no current daemon owning a plain shell to test on. Guessing on a path that KILLS PTYs
is how the 3.0.81/3.0.90 pendulum swings again.

**What was done meanwhile:** the 3.0.92 daemon was deliberately NOT deployed to any
running daemon path (`~/.yggterm/bin/yggterm-headless` stays 3.0.91 fleet-wide), so the
gap cannot reach the fleet.
**To reverse:** nothing to reverse; no behaviour changed.
**Falsifier, unchanged:** a daemon owning any plain shell must report
`permanent_blocker_count >= 1`. The 2.12.24 daemon reports 0 with 9 shells.

## ⭐ "UNSUBSCRIBE WHEN THE WORK IS DONE" IS WRONG FOR A MONITOR — AND THE SESSION ALWAYS SAYS YES

**Status:** OPEN

**Reported by a sibling campaign's row, 2026-08-10, measured.** Their full write-up (evidence,
three fix shapes, caveats) is in that campaign's own `crossings/` note; this entry
owns the yggterm side. ⛔ It is a CONTRACT gap, not a bug — `ygg-booter` did exactly
what it documents.

A relay row armed the booter, was booted once, worked, finished — and then ran
`ygg-booter.py unsubscribe` on itself at 00:40:43, following the contract verbatim:
*"Sometimes you may feel that the work is done so you need to unsubscribe."*
At 02:33 the thing it was watching died. At 09:15 the market opened. **7h43m of
blindness, straight through the open**, ending only when the owner hand-booted it.

⇒ **The generic finding: "unsubscribe when the work is done" is right for work with a
TERMINAL STATE (build, review, migration) and wrong for a MONITOR, where "done" is
never true while the watched thing is live.** So *"am I done?"* is the wrong question
to hand such a session, and any agent asked it eventually answers yes — at the moment
the task list happens to look empty.

⚠ **Do not fix this with a better instruction.** A rule saying *"do not unsubscribe"*
is the same class of object that just failed. Their own fix built a verb answering a
different question instead.

**Three shapes offered, no preference expressed; ⭐ marks their lean:**
1. doc caveat only;
2. ⭐ **a subscription KIND** — `subscribe --kind monitor` makes `unsubscribe` require
   `--force` and state why, putting the refusal where the mistake is made;
3. conditional expiry (they call it probably over-built).

**Two measured notes if this is touched:**
- `max_hours` fires on a wall clock unrelated to the work. Theirs would have expired
  ten minutes before the window it existed to cover. Renewing while work remains turns
  expiry into a **dead-man's switch** — a dead session still expires, a live one
  cannot.
- ⛔ **`defer` expiring by itself is RIGHT — do not make deferrals sticky.** The window
  reverts on its own, so a relay tick that never happens fails toward MORE watching.
  That failure direction is deliberate and it is what saved their lane.

**Their one open question, ANSWERED here from `booter.log` so nobody re-runs the
experiment:** boots ARE delivered to `remote-cc://dev/...` rows via `pty-write`, and
row 8's context-death incident is direct evidence they ARRIVE — 9 refused turns are 9
prompts that landed. ⚠ Delivery is not wakefulness: the 2026-08-09 `\n`-vs-`\r`
defect proves a delivered boot can sit unsent in the composer. Treat *"the booter says
it booted"* as a request, not an effect — which is exactly the caution they applied.

## ⛔⛔ REPORTED 2026-08-10: "SHELL SESSIONS NEVER BREAK, OUR SPECIAL SESSIONS ONLY BREAK — OUR PIPELINE IS BACKWARDS"

**Status:** OPEN

Cause NARROWED by a 9-agent investigation whose headline answer was REFUTED 3/3.
What survives is better than what was proposed, and it is mostly **already in this
repo in the owner's own words.**

His framing, verbatim: *"shell claude code sessions run smoothly. NEVER bottom
broken. Our special sessions ONLY break. Simplicity triumphs in shell sessions
rendering. It is not just CC, all other CLIs are same too. So our pipeline is
backwards. We are fighting these issues from the first month of yggterm."*

### ⛔ THE ATTRACTIVE ANSWER, AND WHY IT IS WRONG — record this before re-deriving it

The synthesis said: *agent CLIs get the daemon's reconstructed vt100 screen as
AUTHORITATIVE while shells get raw bytes, and that is backwards because a TUI is a
differential renderer.* Three independent verifiers refuted it:

1. **The discriminator does not exist.** `shell.rs:94756-94758` schedules a reveal
   reconcile that is **UNCONDITIONAL** — every session, every kind, 1.6 s after
   every mount, and `screen_reconcile_decision` (`shell.rs:673-681`) takes only the
   screen TEXT, no `SessionKind`. **Plain shells are repainted from the daemon's
   authoritative screen too.** So "who gets the authoritative screen" cannot be why
   shells are immune.
2. **The payload is a faithful ABSOLUTE repaint**, not a relative one:
   `state_formatted()` emits `\x1b[m` + `\x1b[H\x1b[J` + every row at absolute
   `CSI r;cH` + a final absolute CUP, and
   `viewport_reconcile_replay_restores_daemon_screen_and_cursor_on_desynced_client`
   (`terminal.rs:4609-4647`) is an executable test asserting the cursor returns to
   daemon truth so relative diffs re-anchor.
3. **It is not novel and its remedy already failed.** `docs/xterm-bugs.md:2256`
   states it almost verbatim, STATUS OPEN, and `a5137e03` demoted it with a live
   measurement: on the rows he actually looks at, the accused write is REFUSED by
   `SkipUnwritable` and the corruption is there anyway.

### ⭐ WHAT SURVIVED — and the one observation that decides it is HIS

**"A TUI refresh fixes it every time"** (`docs/pending-bugs.md`, the requirement). A
client-side `term.refresh()` adds no bytes and changes no buffer content. **If the
buffer had been reseeded wrongly, a refresh would repaint the WRONG content
faithfully. It heals — therefore the BUFFER IS RIGHT AND THE PAINT IS WRONG.**
The repo already says this twice in the requirement: *"the daemon's screen is right and
the CLIENT is painting less than it holds"*.

⇒ **The discriminator is not AUTHORITY, it is RECOVERY.** A shell appends at the
cursor unconditionally, so any partial paint is overwritten at the next prompt —
it self-heals within one line. An agent CLI redraws in place using cursor-forward
for runs of spaces (measured on his own stream:
`❯ On\x1b[C the\x1b[C meta\x1b[C page`), and **cells that CUF skips keep whatever
was already in them**. So a partial paint LATCHES, permanently. Every lossy step a
shell shrugs off is forever for a TUI. That is exactly why "shells never break and
agent CLIs only break", and it explains the owner's 2026-08-10 screenshot where a
composer's **first line was perfect and the wrapped second line lost ~half its
characters in irregular gaps** — the gaps are CUF-skipped cells that were never
painted.

### ⭐ FOUND AND FIXED IN 3.0.92 — the discriminator was one boolean

`recentFrameLikeWriteUntilMs` is armed for ≥600 ms by **any** payload containing
`\x1b[?25l` (hide cursor). **Every TUI emits hide-cursor before every redraw**, so
on an agent-CLI row that flag is re-armed on every frame and is effectively always
true; a plain shell, which does not bracket its output that way, almost never arms
it. The forced full refresh — **the only thing that repairs a partial paint** — was
gated `&& !recentFrameLikeWrite`. ⇒ **an agent-CLI-only suppression of the only
repair path.** That is the owner's "shells never break, our special sessions ONLY
break", in one boolean.

And the refusal branch **destroyed** the demand rather than deferring it:
`pendingVisiblePaintForceFullRefresh` is cleared at the top of the rAF, and the
`else if` only called `recordVisiblePaintRefreshSkipped`. `input_hot` re-armed
itself; `frame_like` and `rate_limited` did not. **The same latch-loss the
function's own header says it was restructured to prevent, surviving one layer
down.**

**The fix (3.0.92):** the refusal re-arms the latch and schedules a recovery for
when the refusing condition can have lapsed, and a
`VISIBLE_PAINT_FULL_REFRESH_DEADLINE_MS = 1500` ceiling means a continuously
redrawing TUI cannot defer the repair forever. Throttling is preserved; dropping is
not. ⚠ The guard test `terminal_eval_script_throttles_hot_render_bridge_work` **had
pinned the defect verbatim** and now pins the fix, including a structural assertion
that the refusal branch re-arms.

**Live proof:** GUI 3.0.92 on the owner's host, faithful capture
(`capture_faithful:true`, `xterm_canvas_composite_over_dom`) of an agent row
mid-turn — the frame-like-hot state that used to suppress the repair — full
viewport, no missing middle, no broken bottom, `session_view_contract_violations:[]`.
⚠ **One clean frame is not proof the intermittent corruption is gone.** It is
intermittent by construction; the real falsifier is the owner typing for an
extended stretch without a hole appearing.

⛔ **Do NOT try SIGWINCH / resize-nudge again.** Tried three times; 3.0.28 shipped
and 3.0.29 reverted it the same hour.
**Falsifier that killed the wrong class:** if the buffer were wrong, `term.refresh()`
would preserve the corruption. It does not. Any candidate cause implying a wrong
buffer is dead on arrival.

## ⛔⛔ REPORTED, LIVE 2026-08-09: "I CANNOT USE YGGTERM. IT IS SO JANK" — the tmpfs, the daemon pile, and the two hot loops

**Status:** OPEN

The requirement: *"the daemon and switching system is infuriating and slowly becoming
unusable … Why switching tax is insane? Why daemon tax is insane? … Everything
hits swap. I ONLY HAVE YGGTERM RUNNING and KDE DE. I cannot go more minimal than
this!! I have 16GB of RAM and an extremely fast last gen SSD."*

**Measured on his laptop the same evening, so the next session argues with
numbers rather than adjectives.** Two causes are FIXED (git remembers them); the
rest is what stays open.

### What the measurement found, in order of size

1. ⭐ **FIXED — the GUI deep-copied a 19.2 MB adblock ruleset per web surface,
   3.3x a second, on the main thread.** `web_surface_policy_gate` returned the
   policy by value into a per-tick snapshot. Stack-sampling his GUI: 11 of 24
   samples in that path; after the fix, **0 of 24**, with the main thread idle
   in `ppoll` 22 of 24. Settled CPU 17.75% -> 11.75% on the same 45 rows.
2. ⭐ **FIXED — the daemon wrote every response one JSON token at a time**, 528
   syscalls for a 2,169-byte reply; an idle daemon was issuing ~7,000 one-byte
   `sendto` calls a second, times every daemon alive.
3. ⛔ **`/tmp` IS A tmpfs AND IT HAD 4.5 GB IN IT — that is RAM, and it can only
   ever go to swap, never be reclaimed.** This is the direct answer to "why does
   everything hit swap". At the time of the report: 7,793 MB of swap in use
   against 6,810 MB of FREE RAM — the machine was not under pressure, it was
   carrying pages evicted during an earlier spike that never came back. Process
   swap accounted for only 3,068 MB; `/tmp` was the missing ~4.5 GB.
   ⇒ **297 entries of it were OURS** — deploy staging directories, one per
   version, back to 3.0.0, each holding a copy of both binaries (~74 MB) — plus
   21 more under other names, 864 MB. Reclaiming them took swap from 7,791 MB to
   5,366 MB with no other change. **Nothing sweeps `/tmp`**, and the fleet
   already ships `ygg-bak-sweep` and `ygg-build-sweep`, so the gap is a missing
   sibling, not a missing idea.
4. ⭐ **FIXED in 3.0.90 — the daemon pile, and it was never really about
   shells.** Re-measured 2026-08-10: **27 daemons on `dev`** (oldest 27 days and
   25 versions old) burning **8.12 cores and 23 GB** between them, and **6 on the
   GUI host**, where yggterm was **71% of all idle CPU** (0.46 of the machine's
   0.64 cores) — the fan. Three defects in one chain, each fixed:
   - ⭐ **The idle gate was being bumped by OUR OWN control traffic.** A
     declaring app re-emits its full OSC 7717 payload on a **~4 s heartbeat**, by
     design, and the PTY reader stamped `last_activity_ms` on every chunk. So
     five `bash -i` shells nobody had touched in weeks reported `idle_ms` of 266,
     1079, 1387, 1711 and 3433 against a 300,000 ms threshold. The gate is an AND
     over owned sessions ⇒ **it could never open.** THE QUIET-GATE LAW, with the
     app as the thing that is never quiet. Fixed: a chunk that is nothing but our
     own declares does not move the idle clock
     (`app_declare::chunk_is_only_app_declares`), one-directionally — a split
     sequence still counts as activity, because discounting real output is the
     dangerous error.
   - ⭐ **A daemon owning a PTY had NO exit at all.** `daemon_should_idle_shutdown`
     refuses while `terminal_session_count > 0`, and the only handoff was a
     `HotUpdateHandoff` RPC nothing sends periodically, behind an opt-in env var
     nobody set. `pty_handoff.rs` was built, tested and unreachable. Fixed:
     `spawn_superseded_self_retire_sweep` hands every PTY to the newest live
     successor and exits, and the fd handoff now defaults ON (`HandoffSweep`
     carries the safety: all-or-nothing, exit only on `AllMoved`).
   - ⭐ **O(N²·M) peer gossip.** Two loops (20 s and 5 s) answered "is a newer
     daemon live?" by pulling every peer's FULL `ServerRuntimeStatus`, which
     carries the machine's entire live-session roster (~100 KB × 26 peers ×
     27 daemons ≈ 140 status round trips a second). Measured: 3 connects to every
     peer socket per 10 s per daemon; 372,631 `sendto` in 12 s from one daemon.
     Fixed: `live_newer_daemon_version` filters by the version in the socket NAME
     first, so a current daemon probes nothing and an old one probes 0-2, newest
     first, stopping at the first confirmation.
   ⚠ **The 3.0.81 framing in the old version of this entry was too narrow** and
   is corrected here: the pile was NOT mostly orphan shells. Of 27 daemons only 6
   held pure orphans — the rest were holding the owner's **live agent rows**,
   which is version coexistence working as designed. The bug was never that they
   existed; it was that **nothing ever moved that work forward**, so the count
   could only grow.
   ⛔ **STILL OPEN: the EXISTING pile does not drain itself.** The self-retire
   ships in 3.0.90, and the 27 daemons already running are older binaries that do
   not have it. They must be drained by hand or as their sessions end.
5. ⛔ **Twelve `[ssh] <defunct>` zombies, 10.9 hours old**, under the armed
   daemon — the SIGCHLD reaper gap, still owed.
6. ⭐ **FIXED in 3.0.90 — and the banner was lying about the cause.** Reported
   again 2026-08-10 on two rows: one waited **3,293 s** overnight and had to be
   killed by hand, the other **1,101 s and counting** while it was diagnosed.
   - ⛔ **The session was never stranded.** `AgentResumeHolderKind::StrandedYggtermOwned`
     means only that the process's environ marker names this session — it never
     checked whether any daemon had exited. Row 8's holder was owned by the
     **live 3.0.89 daemon**, which was holding its PTY the whole time, so
     *"whose daemon exited without handing it over … this clears itself"* was
     false in every clause. Reworded to what is actually known.
   - ⭐ **The real cause is a SECOND KEY NAMESPACE for one session.** A CC session
     born in the remote lane is owned as `cc-runtime://<id>`; the same session
     born as a local session on that machine — what an agent spawning a row
     does — is owned as `local://<id>`. Both render as `remote-cc://<host>/<id>`.
     `remote_runtime_bridge_owner_from_statuses` matched one spelling, so the
     "bind to the existing runtime first" guard (which is correct, and was
     already there) found no owner, decided this was a cold resume, and collided
     with the session's own healthy `claude`. Fixed: `agent_runtime_key_aliases`,
     matched one-directionally so a plain shell is never mistaken for an agent
     runtime, returning the key the OWNER holds.
   - ⭐ **The wait had no deadline** — an absence-gate waiting for a process to
     EXIT, which a working session never does. Now bounded at 120 s and it
     refuses rather than falling through into a transcript-corrupting second
     resume.

⚖ **What is NOT the cause, so nobody re-chases it:** yggterm's own resident
footprint on his laptop is small — 554 MB RSS and 158 MB of swap across 17
processes, against KDE/Plasma's 1,098 MB of swap. **The pressure was never
yggterm sitting still; it was what yggterm ALLOCATES (the 19 MB tick) and what
it LEAVES BEHIND (the tmpfs staging).** Same shape as
[[finding-dbus-autolaunch-leak-and-memory-probe]].

**Falsifier for what remains:** a fleet deploy adds nothing to `/tmp`, the daemon
count on a host does not grow across a version bump, and a switch to a cold
session reveals without a multi-second stall.

⇒ **The daemon-count half of that falsifier is now testable**: on 3.0.90+, deploy
twice and the count must go DOWN, not up, because each superseded daemon hands
its PTYs to the newest and exits.

## ⛔ THE DAEMON PANEL OFFERS AN UPGRADE THAT HAS NOTHING TO UPGRADE — `hot_restart_pending` outranks the version comparison

**Status:** OPEN

reported 2026-08-09, alongside the performance report: *"Why we update same
client and daemon (see screenshot NO DAEMON TO UPGRADE)?"*

`DaemonMetadataGroup` builds the Version line as:

    match (daemon.hot_restart_pending, versions_agree) {
        (true, _) => format!("{} · newer build on disk", daemon.version),
        ...

**`hot_restart_pending` is checked FIRST and swallows the version comparison.**
So when the client and the daemon are the same version — nothing to upgrade, by
the only definition the user cares about — the panel still says *"newer build on
disk"* and offers the action, which then reports there is no daemon to upgrade.

⇒ The line answers two different questions in one field: *"is my client talking
to an out-of-date daemon?"* (`versions_agree`) and *"is there a newer binary on
the filesystem?"* (`hot_restart_pending`). The second is our bookkeeping and,
per the constitution, is precisely what the user is never supposed to have to
know about.

**Falsifier:** with client and daemon on the same version and a newer binary
staged on disk, the panel must not offer an upgrade whose own answer is that
there is nothing to upgrade.

## ⚠ THE STABLE-THEME PIN LEFT FOR libyggterm AND NOTHING GUARDS IT THERE — a tag bump can change the theme silently

**Status:** OPEN

Surfaced 2026-08-09 while repairing this repo's architecture guard (that repair
is shipped; git remembers it). The four assertions that pinned the stable theme —
`STABLE_THEME_ALPHA = 0.96`, `STABLE_THEME_GRAIN = 0.0`, and the two clamp lines
that force a saved profile back onto them before rendering — named
`crates/yggui/src/theme.rs`, which left this repo in the 3.0.0 separation
(`3a51d499`, 2026-08-02). They could not be re-pointed: `yggui` now arrives as a
git dependency **pinned by tag** (`Cargo.toml`: `tag = "v0.12.1"`), and a
contract here cannot reach into a pinned dep's source. They were deleted.

⚠ **The invariant is real and is now unguarded on both sides.** The constants
still hold their values in `libyggterm/crates/yggui/src/theme.rs`, but a grep of
that repo's `scripts/` and `.github/workflows/` finds nothing asserting them —
it ships one `gallery-shot.sh` and a `ci.yml` that does not. ⇒ **whoever bumps
the `yggui` tag can change the stable theme's alpha or grain, or drop the clamp
that overrides a saved profile, and no check anywhere will notice.**

⚖ **The guard belongs in libyggterm's CI, not here** — the assertion has to sit
beside the source it protects. It is tracked in *this* queue because this repo is
what depends on the tag and what breaks when the invariant moves, and because
libyggterm has no live campaign row to route it to.

**Falsifier:** in a libyggterm checkout, change `STABLE_THEME_ALPHA` to `0.5` and
run its CI locally. Something must go red. Today nothing does.

## ⭐ THE HEADLESS SURFACE CAN CREATE A SESSION AND CANNOT REMOVE ONE

**Status:** OPEN

The last of the three daemon-plane gaps filed 2026-08-09. The other two shipped:
the census (`server daemons`, 3.0.82) and `--endpoint` targeting for the
read-only verbs (3.0.84). Git remembers both.

`yggterm-headless server attach <uuid> <cwd>` makes a live `local://` row on
whatever daemon it finds. There is no `server session remove` anywhere on that
surface — removal lives only under `server app`, which refuses outright on a host
with no GUI client. Writing `exit` into the terminal frees the runtime
(`owned_terminal_session_keys` drops, the block reason clears) and **leaves the
session record listed forever**.

⇒ **An agent on a headless host can make a row it has no way to unmake.**
Measured 2026-08-09 on `dev` while live-proving the retire gate: the probe row's
record was still listed after its runtime was gone, and nothing on the CLI could
retire it. (It never reached the GUI's sidebar — a `local://` session on dev's own
daemon is not in guihost's row list — so it cost the owner nothing this time. On the
GUI host it would have been a row he had to close by hand.)

⭐ **The test it passes:** an agent hand-assembled the chore from primitives and
still could not finish it. Same shape as the five verbs in
`docs/agent-field-guide.md` §*this instrument answers a different question*.

**Falsifier:** a session created by `server attach` can be removed from the same
surface that created it, and `server status` stops listing it.

## ⭐ A FAILED `server app` VERB ANSWERS IN PROSE ON stderr WHILE EVERY SUCCESS ANSWERS IN JSON ON stdout — so a JSON caller parses nothing and blames the parser

**Status:** OPEN

**Reported by the `practice` campaign row 2026-08-09**, as: *"`server app rows`
prints 'no live Yggterm GUI client is registered … (dev)' to stderr and EXITS
0 … mine died downstream in `json.load` with a `JSONDecodeError` pointing at
nothing."*

⛔ **The exit-code half does NOT reproduce, and the correction matters more than
the report.** Measured on `dev` 2026-08-09 across four binaries — `yggterm`
3.0.78, `yggterm-headless` 3.0.78, both `*.rollback-3.0.78-pre` (3.0.77), and
the build-tree `target/release/yggterm-headless`: **every one exits 1** with
exactly the quoted message. So a caller that read `0` read it from something
else — most likely the last stage of a pipeline
([[feedback-locks-survive-contract-changes]], the `cargo test | tail` shape) or
a `subprocess.run` without `check`. ⇒ **Do not "fix" the exit code; it is
already right.**

⭐ **What IS real, and is the whole entry:** the verb has TWO answer shapes and
the caller cannot tell which it will get. A success is a JSON envelope on
stdout (`request_id`, `handled_by_pid`, `data`); a pre-flight refusal is three
lines of English on stderr with empty stdout. Every `server app` caller in the
fleet is a JSON consumer, so the refusal — the single most useful message we
print, naming the host and the fix — is the one output none of them can read.
The reporter's `JSONDecodeError` was that gap, and it points the agent at its
own parser instead of at the host it ran on.

**The fix:** a refusal emits a JSON error envelope on stdout as well (same
`request_id` shape, an `error` member carrying the prose), and keeps both the
non-zero exit and the human text on stderr. One question, two audiences, no
second encoding to diverge.

**Falsifier:** `yggterm server app rows` on a host with no GUI, piped to
`python3 -c 'import json,sys; print(json.load(sys.stdin)["error"])'`, prints the
host-and-fix sentence instead of raising.

## ⚠ THE `yggterm-shell` TEST TARGET READS THE DEVELOPER'S LIVE `~/.claude/projects` — 40 MINUTES, AND GROWING FOREVER

**Status:** OPEN

Measured on `dev` 2026-08-09, during a routine `cargo test --workspace
--no-fail-fast`:

- `yggterm-shell --lib` alone reported `finished in 2386.60s` — **39.8 minutes**,
  the whole cost of the suite for practical purposes;
- while it ran, the test binary held **open file descriptors on real session
  transcripts** under `$HOME/.claude/projects/<project-slug>/*.jsonl`;
- that store is **960 MB across 655 `.jsonl` files** on this host today.

⇒ A test is walking the developer's **actual Claude Code history** rather than a
fixture. That breaks `CLAUDE.md` §*No non-determinism* — *"do not introduce
behavior that differs based on timing, environment, or ordering that the code
does not control"* — in the most literal way available: the input is a live,
per-machine, monotonically growing directory that no test controls. Two agents on
two hosts run different tests; the same agent runs a slower one every week.

⚠ **WHAT IS NOT PINNED, stated so nobody quotes this as more than it is: WHICH
test opens those files.** The open fds prove the target does it; they do not name
the case. Grepping `yggterm-shell` for `home_dir()` finds only `store.home_dir()`
(the *yggterm* home, a different path), so the read most likely arrives through a
shared crate — `AgentCliDescriptor.session_store_globs` is
`".claude/projects/*/*.jsonl"` (`yggterm-core/src/agent_cli.rs:1296`) and is the
obvious place to start.

**Falsifier, cheap:** run `-p yggterm-shell --lib` with `HOME` pointed at an empty
scratch dir. If the runtime collapses, the scan is the cost and the fix is a
fixture; if it does not, the 40 minutes are real work and this entry is wrong.
⛔ Do that before optimising anything — a slow suite everyone pays for is worth
one measurement, not a guess.

## ⛔ THE WORKING FLAG HAS TWO WRITERS AND THE BLIND ONE WINS — a healthy agent reads as stalled once its owning daemon is gone

**Status:** OPEN

Split out 2026-08-09 from the row-stranding entry, whose other halves are fixed
and live-proven. **Root-caused by code reading 2026-08-09 ~17:20; the mechanism
below replaces the original one-line guess ("the dot is fed by PTY output from
the owning daemon, so it simply stops"). That guess was right about the symptom
and wrong about why, and the difference decides the fix.**

⭐ **`ManagedSessionView.working` is written by TWO paths that know different
things, and they overwrite each other:**

1. **The daemon's snapshot.** `refresh_snapshot_session_*`
   (`daemon.rs:4039`/`4049`) sets `working` from **its own `terminals` screen** —
   `Some(true)`/`Some(false)` for a runtime it owns, and **`None` for anything it
   does not**. `None` here honestly means *"I hold no live screen for this row"*.
2. **The GUI's 2.5 s working-flags poll.** `working_flags_including_proxied`
   (`daemon.rs:3677`) is the informed one: for a row the serving daemon does not
   own it **asks the preserved owner over its socket** and merges the answer.
   The GUI applies it with `apply_live_session_working_flags`.

⛔ **And `apply_snapshot` does `self.sessions.clear()` and rebuilds every session
from the snapshot (`lib.rs:6074`).** So every snapshot apply overwrites the
polled value with the daemon's — including overwriting a *known* `Some(true)`
with an *unknown* `None`. The dot then reads idle, because the GUI blinks only on
`Some(true)` and collapses `Some(false)` and `None` to idle
(`shell.rs:86613`, deliberately, to stop a frozen frame blinking forever).

⇒ **While the owner is reachable the poll keeps re-winning the race, so the dot
mostly works. The moment the owner becomes unreachable the poll contributes
nothing for that row, the snapshot's `None` stands unopposed, and the dot goes
dark and stays dark** — on a session that may be mid-turn. That is what sent the
owner to check on a session that was fine.

⚠ **Two writers, one field: this is the SSOT law, and the less-informed writer
wins.** `None` is *"I don't know"* and must never overwrite *"I asked the owner
and was told"*.

⛔ **The obvious fix is a trap.** "On apply, keep the old `working` when the
incoming is `None`" resurrects the bug the 86613 comment records — a session
blinking forever on a frozen last frame after its turn ended, because a truly
dead owner also produces `None` for ever. Any preservation needs a bound: an age,
or a positive "the owner answered" signal, not silence.

⭐ **The instrument that was RIGHT throughout the incident reads a third source:**
the booter, which watches transcript activity, logged `WORKING` at 11:46 · 11:51
· 11:56 · 12:01 · 12:06 · 12:11 while the on-screen dot said nothing.
[[finding-agent-session-liveness-is-invisible-to-os-signals]] ⚠ Its cost is real
and must be priced before it is chosen: for a `remote-cc://<host>/<uuid>` row the
transcript lives on the FAR host, so this is an ssh hop per row per poll unless
it is routed through that host's own daemon.

⚖ **And the design slot already exists, unwired.** `DESIGN.md` §*Status indicator
vocabulary* reserves `ORANGE/AMBER` for *"recovery in progress, degraded
runtime"*, and a row whose owner cannot be reached is exactly a degraded runtime.
An amber dot cannot restore the signal, but it can stop the app asserting a
liveness verdict it does not hold — which is the half that cost the owner his
time. ⛔ Wiring it means editing that DESIGN.md section first; it says so.

⛔ **NEGATIVE RESULT, measured 2026-08-09 ~17:40 — do not re-chase this.** The
tempting next step is "the race must be visible as a flickering dot, so fix the
clobber and watch the flicker stop". **It is not visible at 1 Hz.** On the GUI
host, whose daemon owns **0 of 39 rows** — so every single row is proxied and
every one is subject to the race — `server app rows` sampled once a second for 14
seconds returned a *stable* busy set (7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 6, 7, 7, 7),
and the single 6 is consistent with one agent genuinely ending a turn.

⇒ The mechanism above is real and code-cited, but the snapshot apply is **rare
relative to the 2.5 s poll**, so the clobber is a brief dropout rather than a
sustained flicker, and a working row's dot mostly holds its polled value. ⚠ **So
the visible payoff of fixing the clobber alone is UNMEASURED**, and a fix shipped
on the flicker theory would be shipped on nothing. Measure the dropout window
first (sub-second sampling of the `data-sidebar-live-session-working` attribute,
or trace the apply), or go straight for the part that IS user-visible: the
permanent dark dot once the owner is unreachable.

⛔ **SECOND NEGATIVE RESULT, measured 2026-08-13 — the clobber did not show up
on a whole afternoon of live sampling, and the symptom it was blamed for had a
different cause.** The dot's failure that day was reported as *"the 6.x rows are
not blinking, I cannot tell if they are working"*, and this entry was the
standing suspect. It was not the cause: the blink had not painted **at all**
since 2026-07-21, for every row, working or not (fixed in 3.0.122 — see the
CHANGELOG). Separately, `busy` was sampled against the owning daemon's own
screen for every seated row, correlated in the same moment, and **they agreed on
every sample** — including on rows the GUI's daemon does not own. ⇒ The
mechanism below is still real in the code, but it has now failed to produce an
observable twice. **Before spending a session on it, get a reproduction.** The
cheapest one is still the falsifier below; the second-cheapest is to compare
`server app rows` against `server terminal screen` for the same row in one
command, which is what was run here and what came back clean.

**Falsifier:** with a row's owning daemon retired and the agent mid-turn, the
row's indicator still blinks. **Cheaper intermediate falsifier for the mechanism
above:** on a GUI whose daemon owns few of its rows, sample the dot's
`data-sidebar-live-session-working` attribute FASTER THAN THE SNAPSHOT CADENCE
and look for dropouts on a row the poll is reporting as working — 1 Hz is too
slow, as measured above. (`working_edge` telemetry cannot see it: that trace
skips `None` entirely, so the clobber leaves no edge.)

## ⛔ A DAEMON SERVES ONE REQUEST AT A TIME, AND A HOT-RESTART REQUEST HOLDS IT FOR ~11 SECONDS — so anything else asking that daemon waits

**Status:** OPEN

Measured on `dev` 2026-08-09 while fixing the click path, and it is the reason
the first fix there did not work.

The click's stale-owner upgrade was moved off the click's thread, so the
endpoint resolved at **+2.96 s**. First paint still landed at **+15.12 s**. The
gap is the owner daemon: the background thread's `hot_restart` request reached
it first and held it, and the foreground's own pre-bridge calls to that **same
daemon** — the appearance check and the identity-profile sync — queued behind
it. Whichever thread reached the socket first decided whether the user's click
was fast. ⛔ A race, not a fix.

3.0.80 works around it by holding the upgrade until the bridge's first paint
(`stale_runtime_owner_hot_update_released released_by:"bridge_first_paint"`),
which is correct for that path but does not make the daemon concurrent. **Any
other caller that asks a daemon anything while it prepares a handoff still
waits ~11 s**, and nothing tells it why.

⚖ **And the question underneath, which is the owner's to settle:** should a
CLICK drive a daemon swap at all? The swap that cost 55 PTYs this morning was
started by the owner clicking a row (12:09:40 click → 12:10:16 retire). The
settled relay-gate design makes a swap **an appointment at a relay boundary**,
not a search — and a click is not a relay boundary. The deploy path already
upgrades every daemon by itself, so the click-driven upgrade may be redundant
as well as dangerous. ⇒ `docs/owner-attention.md`; not changed unilaterally,
because removing it is a policy change and this was a latency fix.

**Falsifier:** `server status` against a daemon that is mid-`hot_restart`
answers within a second.

## ⛔ `guihost` IS CARRYING THE STRANDING BUG ARMED: ONE DAEMON'S BINARY *IS* THE BACKUP FILE, AND `rm -f *.old.*` IS THE TRIGGER

**Status:** OPEN

Measured on `guihost` 2026-08-09 14:2x, read-only, while proving out the deploy path
for the entry above.

| pid | version | `/proc/<pid>/exe` | owns |
|---|---|---|---|
| 523808 | 3.0.76 | `…/yggterm-headless` (live) | 5 — every `remote-cc://dev/*` agent row |
| **426042** | **3.0.75** | **`…/yggterm-headless.old.522814` (live)** | **2** |
| 3844228 | 3.0.70 | `…/yggterm-headless (deleted)` | 1 |
| 2824474 | 3.0.59 | `…/yggterm-headless (deleted)` | 1 |
| 169710 | 3.0.29 | `…/yggterm-headless (deleted)` | 1 |

⛔ **426042 is running FROM the backup file.** A past deploy renamed the live
binary to `yggterm-headless.old.522814` while that daemon held it, so its exe link
followed the rename onto a path no install will ever write again. It is quiet
today only because the file still exists: `binary_replaced` is false, so it never
tries to retire.

⇒ **Deleting `~/.local/bin/yggterm-headless.old.522814` fires it.** The link
becomes `(deleted)`, the retire triggers, the pre-3.0.78 derivation lands on the
file that was just removed, the handoff is skipped, and the cold shutdown kills
its 2 PTYs. That deletion is the **last step of the very deploy dance that created
this** (`rm -f ~/.yggterm/bin/*.old.*`), so the ordinary next deploy is the
trigger.

⚠ **And an automated sweep already knows the file by name.** The
`fleet-binary-sync` startup hook lists `yggterm-headless.old.522814` and
`yggterm.old.522814` in its 11-binary roster, so it is a candidate for exactly
the kind of tidy-up that would fire this.

**The safe moves, in order:**
1. **Do not delete `*.old.*` on `guihost` while 426042 lives.** Leave the file.
2. Deploy 3.0.78 with an **in-place `mv`** over `~/.local/bin/yggterm-headless`.
   It does not write the `.old.522814` path, so it disarms nothing and arms
   nothing, and the four daemons whose links point at the canonical path hand off
   correctly even on their old code (measured — see the entry above).
3. Retire 426042 deliberately once its 2 rows are drained, then remove the file.

⚠ The other three daemons already read `(deleted)` on the CANONICAL path, which
is the case the old code gets right — they are not armed. That they have not
retired anyway is the separate long-standing `old daemon never retires` shape, and
their non-empty `preserved_terminal_owner_keys` say they are lingering as
preserved owners rather than stuck.

**Falsifier:** `readlink /proc/426042/exe` no longer ends in `.old.522814`, or the
pid is gone. Either way this entry is spent.

## ⛔⛔ A PLAIN SHELL DIES ACROSS A HANDOVER AND A WEB-SURFACE ROW BESIDE IT SURVIVES — SAME PRESERVED LIST, OPPOSITE OUTCOMES

**Status:** OPEN

⚖ **This is the CONSTITUTION's *"plain shells are first-class and must survive a
bump like anything else"* failing, measured cleanly for the first time.**
`CLAUDE.md` records *"a plain shell's row was lost outright"* from 2026-07-26 as
part of the hot-restart story; nothing in this queue owned it, and the
hot-restart gate entry is about the gate not FIRING, which is a different
question from what happens when it does.

**Measured 2026-08-09 across four daemon handovers (3.0.70 → 71 → 72 → 73 → 74),
each a clean `hot-restart` with the GUI left untouched.** Rows 45 → 41. One loss
was deliberate (a retired predecessor row). The other three were not:

| row | kind | survived? |
|---|---|---|
| three plain shells rooted at the repo | terminal, `icon_text: "$_"` | ⛔ **all three gone** |
| three web-surface rows | ychrome | ✅ all three survived |
| every numbered agent row (12 of them) | codex / claude-code | ✅ all survived |

⭐ **The discriminator, and it is what makes this worth chasing:** all six of the
first two groups were named TOGETHER in the same `preserved_terminal_owner_keys`
list in the handoff's own reply — the daemon said it was preserving all of them,
and then preserved half. So this is not "plain shells are not preserved"; it is
**the preservation path reporting success for a class it does not actually carry
across.** The agent rows survive for a different reason entirely (their state is
in their own JSONL and the resume path rebuilds them), so their survival is not
evidence that the preservation mechanism works.

⚠ **Why the asymmetry points somewhere specific:** a ychrome row is an APP row
with a declared app identity to restore from, and an agent row has a transcript.
A plain shell has neither — it is the one class whose entire existence is the
live PTY, which is exactly the class `retention.rs` notes cannot be re-derived.
⇒ Look at what `restore_live_session` does when a preserved record has no app
token and no session id to resume, rather than at whether the record was written.

**Falsifier:** open three plain shells, note their `full_path`s, force a daemon
handover, and read `server app rows` back. Three rows with the same paths is a
pass; the reply's own `preserved_terminal_owner_keys` echoing them is NOT — that
is precisely the field that was already true while they died.

⛔⛔ **AGENT ROWS ARE NOT SAFE EITHER, AND THE LOSS IS RECORDED AS THE USER'S
DOING — measured 2026-08-13, 3.0.114 → 3.0.116 on the desktop host.** The table
above says every numbered agent row survived; that is no longer the whole story.
`server app update restart` reported the request and did nothing — the GUI sat
at `hot_update_handoff_active` for four minutes with the same pid, never
swapping — so the GUI was terminated and relaunched by hand. Rows went **48 →
46**, and the two that did not come back were both **`remote-cc://` agent
rows**: another cluster's live 6.7 row, and a 6.2 row.

⭐ **SECOND SIGHTING, AND THE STATE NAMES THE GATE — 2026-08-13, GUI 3.0.116
with 3.0.118 on disk.** `server app update restart` again answered and did
nothing. `daemon_update_state` at that moment:
`state: hot_update_handoff_active`, `hot_update_pending: true`,
`hot_update_pending_reason: "session_survival_preserved_owner"`,
`session_survival_required: true`, `preserved_owner_pids: [<one daemon>]`,
`preserved_runtime_keys` holding 12 rows — **and `can_hand_off: true` in the
same reply.** ⇒ The verb is not silently failing; it is deferring on preserved
sessions, forever, while the field that says whether a handoff is possible reads
`true`. Two fields answering opposite questions with no name to tell them apart
is why this reads as "the verb did nothing". **Look at what clears
`session_survival_preserved_owner`** — it is a gate that stays shut while any
preserved owner exists, and on a machine that always has one it can never open,
which is the shape the constitution forbids (`docs/spec-hot-restart-relay-gate.md`).

⚠ **The second half is worse than the loss.** `server app sessions restore` —
the verb built for exactly this — refused both, answering `declined_closed` with
`restorable: 0`. Nobody closed them. The tombstone plane cannot tell *the user
deleted this row* from *this row was lost when its GUI died*, so it files an
involuntary loss under the one heading that makes it unrecoverable, and the
recovery verb then declines by design. `server app open` restored the 6.7 row in
one call, which is the tell: the row was perfectly openable, and only the
deny-list stood in the way.

⛔ **THE TOMBSTONE HALF IS NOT WHAT IT LOOKS LIKE — checked in the trace,
2026-08-13, and the mechanism above is wrong.** A GUI death does not tombstone
anything: `PrepareClientClose` (the path a closing GUI takes) never calls
`close_live_session_row`, a hand-killed GUI does not run it at all, and a
structural lock already forbids the migration and adoption paths from
tombstoning. The one writer is the explicit close — and every row involved here
has that close in the trace, by name:

| row | what the trace shows |
|---|---|
| `remote-cc://oc/ebf6c53e…` | `app_control` `request_begin` `{"kind":"remove_session", …}` |
| `remote-cc://dev/7cf25693…` | `live_session_close_preflight`, then `explicit_remote_session_close_warning` |
| `remote-cc://dev/7bcecf20…` | `live_session_close_preflight`, then `explicit_remote_session_close_requested` |

⇒ These rows were not filed as deletions by mistake. **Something called
`session remove` on them**, which is the verb that MEANS "delete this row", so
the deny-list did exactly its job. The real defect is one level up and it is
worth naming precisely: **after a hand-kill, tooling tidies up rows it judges
dead using the same verb a person uses to delete one** — and the daemon cannot
tell those apart, because at the wire they are the same request. Nothing below
`session remove` can fix that; the caller has to stop reaping corpses with the
delete verb, or the verb needs a separate spelling for "this is already gone".

⚠ Two other rows reported lost this way were checked and are **not tombstoned at
all** — `sessions restore` returns both as `restorable`. So the symptom is not
uniform, which is another reason to trust the per-row trace over the pattern.

**Recovery that exists today:** `server app sessions restore <path>...
--include-closed` restores a deliberately-closed row and reports it under
`overridden_closed`, for exactly this case. Shipped 3.0.117.

⇒ Two owners: the handover half — rows dying at all — belongs here. The
deny-list half is answered above and needs no further work in the restore
lifecycle; what remains is the CALLER question, which belongs to whichever
tooling runs the post-kill tidy.

**Falsifier for this half:** kill a GUI holding agent rows, relaunch, and every
row returns. If a row is missing, read its trace for `remove_session` /
`live_session_close_preflight` BEFORE blaming the deny-list — absent that, the
tombstone theory is disproved for that row.

⭐ **A SECOND, SIMPLER CANDIDATE CAUSE — measured 2026-08-09, and it fits every
column of the table above.** Look at *when* the shells died rather than at the
handover itself. Until 3.0.81 the predecessor's retire loop cold-shut-down as
soon as its owned sessions passed 300 s of silence, and a cold shutdown kills its
PTY children. So the sequence is: the handoff honestly preserves all six rows
(hence `preserved_terminal_owner_keys` being true at the time) → the preserved
owner keeps polling → the shells fall quiet → the gate opens → **the preserved
owner retires by destroying them.** Agent rows survive because they are
re-resumed; ychrome rows survive because an app row declares an identity to
relaunch from; a plain shell has neither, which is the asymmetry this entry
already noticed.

⇒ **Re-run the falsifier on 3.0.81 before spending a session inside
`restore_live_session`.** The cold-kill is now refused
([[finding-hot-update-never-converges-idle-gate]], the plain-shell retire entry),
so if the three shells now survive, this entry is closed and the preservation
path was never the defect. If they still die, the cause is where this entry says
it is and the ground is now clear.

## ⛔⛔ THE CLI DRIFT REPORTER RESOLVES AGAINST A `PATH` THE LAUNCH STOPPED USING IN 3.0.70 — SO IT NOW CRIES DRIFT ON A CORRECT MACHINE

**Status:** OPEN

Surfaced 2026-08-09 by the first fleet sweep, which is the first thing that ever
ran a refresh on every machine and so the first thing to read this reporter's
output in bulk. Measured on the GUI host, 3.0.73:

```
effective_cli_version_drift  tool=codex  managed_version=0.147.0
                             effective_path=~/.local/bin/codex  effective_version=0.144.6
  "the refresh updated the managed copy, but the login shell resolves a
   DIFFERENT install — sessions on this machine run the version reported as
   effective_version"
effective_cli_unresolvable   tool=pi        managed_version=0.84.1
effective_cli_unresolvable   tool=opencode  managed_version=1.18.15
effective_cli_unresolvable   tool=qwen      managed_version=0.21.8
  "the login shell resolves no such binary, so a session cannot launch this
   CLI on this machine"
```

**All four sentences are false, and the machine is fine.** Probed directly:

| path | codex version |
|---|---|
| `~/.yggterm/npm/bin/codex` (managed) | **0.147.0** |
| `~/.local/bin/codex` (a second npm prefix) | 0.144.6 |
| `bash -lc 'command -v codex'` | `~/.local/bin/codex` → 0.144.6 |

⇒ A LOGIN SHELL does resolve 0.144.6. **A yggterm session does not run a login
shell's `PATH` any more.** 3.0.70 changed the launch to compose
`'~/.yggterm/npm/bin':'~/.local/bin':…:"$PATH"` with the managed directory
FIRST, so a launched codex row runs 0.147.0 — the version the reporter says it
does not. Same for pi, opencode and qwen: they live only in the managed npm
directory, which is exactly why 3.0.70 put it on the launch path, and the
reporter calls all three unlaunchable.

⚖ **This is the 3.0.70 root cause reappearing one layer out, in the instrument
instead of the launch.** That bug was *"a gate and a launch that resolve against
different sets can only disagree"*; `login_shell_resolved_cli`'s doc comment
still says it *"reproduces the resolution a real launch performs, deliberately
WITHOUT the managed prefix"* — which was TRUE when it was written and stopped
being true when the launch was fixed underneath it. ⇒ **When a resolution rule
changes, every instrument that models that rule is part of the change.**

**Why it matters rather than being cosmetic:** it fires on every refresh, on
every machine, for 3 of 9 CLIs plus any CLI with a second install — and now the
sweep runs it fleet-wide on a clock. An alarm that is always on cannot report
the case it exists for, which is a managed install that a session genuinely
cannot reach.

⚠ **Do NOT "fix" it by deleting the reporter.** The condition it was built for is
real, and the drift it would catch is the silent kind. Fix it by resolving
against the SAME composition the launch uses — there is one owner of that string
already — and keep the login-shell answer only as the separate question it
actually is (*"what would the user's own terminal run"*), reported as that.

**Falsifier for a fix:** on this host, a refresh must emit no
`effective_cli_version_drift` for codex and no `effective_cli_unresolvable` for
pi/opencode/qwen, while a genuinely unreachable managed CLI still raises one.

## ⛔⛔ `terminal new` REPORTS A TIMEOUT AND CREATES THE ROW ANYWAY — SO A RETRY LOOP IS A ROW BOMB

**Status:** OPEN

Measured 2026-08-09 on a live 3.0.69/3.0.70 GUI, and the owner saw the damage
before the measurement did: *"Why are you spawning a billion kimi sessions?"*

`server app terminal new` answers `Error: timed out waiting for app control
response <id> after 15000 ms` — and the row **is created**, every time. Six
"failed" attempts left six rows. Ten rows in total had to be reaped by hand.

⛔ **This is the [[finding-a-set-is-not-a-fill]] family inverted, and the
inverted form is far more expensive.** A verb that reports success on failure
wastes a verification; a verb that reports FAILURE ON SUCCESS invites the caller
to retry, and every retry is another real, side-effecting create. The natural
agent reflex — "the create failed, try again" — is precisely what turns one
slow response into a sidebar full of orphans on the owner's screen.

Two things are wrong and they are separable:

1. **The 15 s app-control deadline is shorter than a create takes on a busy
   GUI.** `server app rows` answered throughout, so the GUI was not wedged; it
   was rehydrating 44 rows plus remote-machine refreshes. The create itself
   completed — only the reply missed the window.
2. **A timed-out app-control request has no outcome the caller can read.** There
   is no "what did request `<id>` actually do?" lookup, so the caller cannot
   distinguish *never ran* from *ran and the answer was late*. `~/.yggterm/
   app-control-responses` exists; nothing points the CLI at it on timeout.

**Fix shape:** the timeout message must carry the request id AND tell the caller
how to resolve it, and `terminal new` must be idempotent per request id (a
retried create with the same id adopts the existing row rather than making a
second). ⛔ Do not "fix" this by raising the deadline alone — that only moves
the cliff, and the row bomb happens at whatever the new number is.

⚠ Until then, **never loop on a `terminal new` timeout.** Read `server app rows`
first; the row is probably already there.

## ⛔⛔ TWO TOASTS AT TWO ANCHORS AT ONCE — AND THE TOP-RIGHT ONE IS NOT A TOAST

**Status:** OPEN

Reported 2026-08-08 alongside a blank session-metadata rail. ⛔ **The rail half
of that report is the same defect as the [6.3] entry above and is folded into
it** — a lost edit batch, root-caused and reproduced 2026-08-14. Only the toast
half is still its own question, and it was never explained.

His second screenshot has `Restoring Remote Terminal` at top-CENTRE and
`Image Staged` at top-RIGHT at the same time, the right-hand one painted OVER the
rail's header. Two live toast surfaces at two anchors is the "double
notifications" he named.

⛔ **`ToastAnchor` HAS NO TopRight** — it is `TopCenter | BottomLeft |
BottomRight` (libyggterm `notifications.rs`), and `toast_anchor()` returns
TopCenter or a BOTTOM anchor by sidebar edge. ⇒ **the top-right card is not a
`ToastViewport` toast**, so "who mounts a second toast host" is the wrong
question. Ask what draws at top-right that is not a toast.

⛔ **AND IT IS NOT A DESKTOP NOTIFICATION EITHER — hypothesis raised and
FALSIFIED 2026-08-08.** "Image Staged" *is* a yggterm notification (clipboard
image paste) and yggterm can deliver to the desktop via `notify-send`
(`yggterm-platform/src/lib.rs`), which KDE draws top-right over the window — a
clean fit. But the live host's `~/.yggterm/settings.json` reads
`in_app_notifications: true, system_notifications: false`, and
`notification_delivery_mode()` maps `(true, false)` to `InApp`. **No system
notification is being sent on this machine.** Do not re-derive this.

⚠ **INSTRUMENT GAP found while falsifying it:** the delivery mode is **not
exposed in `server app state` at all** — no `notification_delivery`,
`in_app_notifications` or `system_notifications` field anywhere in the payload.
The only way to read it is `~/.yggterm/settings.json` on the GUI host, which no
remote agent can reach. Worth closing when this area is next touched.

⇒ Still open, and the next capture will answer it: the rail watcher now freezes
every `[data-yggui-toast*]` element's rect and text at break time.

## `sessions regenerate-copy --budget 0` IGNORES THE BUDGET AND CALLS THE LLM

**Status:** OPEN

Found 2026-08-08 while looking for a safe way to force one local session scan.
`server sessions regenerate-copy --budget 0 --skip-remote --json` began
**requesting titles from litellm immediately**:

    INFO resolving session title session_id="…" force=false file_path=…
    INFO requesting title from litellm session_id="…" context_chars=7933

A budget of zero should be the natural "scan, generate nothing" mode. Instead
`--budget 0` behaves as unlimited or as the default — a classic zero-means-unset
bug, and it is the second time in this repo that a flag reported an intent it
did not carry.

**Why it matters beyond the surprise:** the external LLM endpoint rate-limits
with HTTP 429 under quick successive calls, and the daemon's own chore tick is
deliberately capped at 3 generations per tick for exactly that reason. A verb
that silently ignores its cap can burn that budget from the CLI side. It had to
be killed by hand.

⚠ **Also missing, and the reason the flag was reached for at all:** there is no
cheap way to force a local tree scan. `local_tree_scan` is throttled behind
`local_tree_scanned:false` / `superseded:true`, so an agent that changes the
session store cannot ask the product to re-read it. A read-only
`sessions rescan` would have made this entry unnecessary.


✅ **ROOT-CAUSED AT SOURCE 2026-08-13, without firing a single LLM call** — the
live repro would have burned a rate-limited endpoint other campaigns share, and
a zero-means-unset bug is settleable from the code. It is two halves, and only
both together produce the surprise:

```
apps/yggterm/src/bin/yggterm-headless.rs   .unwrap_or(0)      ← absent flag becomes 0
apps/yggterm/src/main.rs                   .unwrap_or(0)      ← same, in the other binary
crates/yggterm-core/src/lib.rs             let limit = if budget == 0 { usize::MAX } else { budget };
```

⇒ **`--budget 0` is byte-identical to passing no flag at all, and zero is the
encoding for UNLIMITED.** The flag that reads as *"generate nothing"* is the flag
for *"generate everything"*, and no caller can express the first at all.

⚠ **The fix is NOT flipping the `== 0` arm.** Absent-flag currently relies on
`0 ⇒ unlimited`, so changing the core's reading silently caps every caller that
passes nothing. The shape that works is to distinguish ABSENT from EXPLICIT ZERO
at the CLI — parse to `Option<usize>`, keep absent ⇒ unlimited, and give explicit
`0` a path that generates nothing — which is a change to a shared contract with
two call sites, not a one-line patch.

**Falsifier:** `--budget 0` scans and generates nothing, while omitting the flag
still generates without a cap.
## ⛔⛔ REPORTED, LIVE 2026-08-09: A DEPLOY MADE THE OPERATOR'S OWN ROW UNREACHABLE FOR 5-10 MINUTES — "the pain we go through is IMMENSELY irritating"

**Status:** OPEN

The requirement: *"The hot restart needs to be seamless. The pain that we go through is
IMMENSELY irritating. YOU were stuck for ~5-10 mins and I could not communicate
to you."*

⚖ **This is the CONSTITUTION's second obligation failing on the owner himself** —
*"They never stall their work waiting for ours. A restart of ours must not
interrupt, reset, or destroy what another agent is doing."* The agent he could
not reach was the yggterm campaign row, and what made it unreachable was that
row's own deploy.

**What that session did, so the next one can reproduce it rather than guess:**
three deploys in one sitting (3.0.81, 3.0.82, 3.0.84), each a `mv`-in-place over
four paths on three hosts, each followed by a daemon handover under live rows —
including the row he types into. Interleaved with 2-14 minute release builds and
a 844 s test suite, all of which hold the turn.

### ⭐ MEASURED 2026-08-09 ~20:30 — BOTH FILED CANDIDATE CAUSES ARE FALSIFIED

The entry named two candidates ((1) the turn was busy, (2) the handover broke the
row's path) and said to measure before building. That has now been done, from
that row's own transcript and the booter's log. **Neither is what happened.**

**(1) is falsified 4 for 4.** Every message he sent that day landed on a row
whose turn had **already ended** — idle before his message: 28.3 m, 67.9 m,
14.0 m, 3.3 m. Not once was his input queued behind a build, a deploy or the
844 s suite. A busy turn was never the thing in his way.

**(2) is falsified at the moment it names.** The turn that preceded the long
outage ended at **18:00:29 with a complete, successful report** — two minutes
after the 3.0.82 deploy and its handover. The daemon swap neither killed the turn
nor broke the path; his text arrived the instant he typed it.

⇒ **The real mechanism, and it is a third thing: the row STOPPED, and the
watchdog that exists to restart it could not press Enter.** The turn ended at
18:00:29; the booter fired five boots (18:09, 18:20, 18:26, 19:01, 19:07) and
**none woke it**, because it wrote `\n` where an agent CLI in raw mode needs a
discrete `\r`. The text landed in the composer and sat there. He walked over at
19:08 and described exactly that: *"I just saw `continue, the booter booted` and
a empty line. The enter key did not send the prompt."* **68 minutes, not 5-10.**

⭐ **The A/B is already in `booter.log` and needs no new run:** before the fix,
`pty-write` boots woke the session **0 of 5**; after `efb02e26` (19:16) they woke
it **2 of 2**, the second one flushing four backed-up boot lines in a single
message. ⇒ **the cause was fixed at 19:16 by the very session that received the
complaint at 20:16, and neither it nor this entry realised the two were the same
thing.**

⚠ **What that leaves genuinely open**, with the dead framing removed:

- **His falsifier has still not been run end to end** (below). The keystroke path
  was never the proven defect, but it was never measured across a fleet bump
  either, and his sentence is about the deploy.
- ⭐ **He still has no channel that outruns a held turn**, and no way to tell a
  *busy* row from a *stopped* one from a *stopped-with-text-stuck-in-the-composer*
  one. All three look identical in the sidebar. This is the part of the entry that
  survives measurement intact.
- ⚠ **After 3 failed boots the booter escalates and then stops booting entirely** —
  correct in principle ("a human owns it now"), but it turned a 7-minute recovery
  into a 68-minute one, and the escalation is a notify card he did not see for 37
  minutes.

⭐ **And the same wrong encoding of Enter was found living in the DAEMON**, where
it is worse — see the paste-draft entry below. Fixed in 3.0.85.

**Falsifier:** deploy a version bump across the fleet while he types into a row,
and every keystroke is acknowledged inside a second.

## ⛔ A ROW WITH A DRAFT IN IT CAN NEVER BE BOOTED AGAIN, AND NOTHING SAYS SO — the booter skips it silently, forever

**Status:** OPEN

`ygg-booter.py`'s `tick` handles a draft refusal correctly as far as it goes: a
refusal is not a failed boot, so it gives the attempt back (`s["boots"] -= 1`)
and retries next tick. **But there is no exit from that state.** `boots` can
never reach `MAX_BOOTS`, so the escalation branch is unreachable, `escalated` is
never set, and the row is skipped on every tick from then on — visible only as a
`SKIP:drafting` line in a log nobody reads.

⇒ **He half-types a sentence into a row, walks away, and that row is removed from
the watchdog's care permanently without anyone being told.** The shape is the
one this project keeps re-finding: a guard that is right about the moment and
wrong about the *forever* — see
[`agent-field-guide.md`](agent-field-guide.md) and the retire-gate entry.

⚠ **This was widened by the 3.0.85 paste-draft fix, and that is why it is filed
here rather than left for someone to trip over.** Before it, a pasted multi-line
draft read as "no draft", so those rows were still bootable — by accident, via a
bug. Now that the flag is correct, a pasted draft correctly refuses boots, which
makes this silent-skip state genuinely reachable for the first time. **A correct
signal reaching a reader with no exit condition is not neutral.**

⚖ Note the two arms want different answers and neither is "boot anyway": a
draft the OWNER typed must never be typed over (that is the whole point of
`--refuse-if-draft`), but it should be *escalated* — he is the only one who can
clear it. A draft left by a failed boot is our own litter and should be cleared
by us, not treated as his sentence. The booter cannot currently tell the two
apart, and distinguishing them is the real work here.

**Falsifier:** leave an unsubmitted draft in a subscribed row and let it go idle
past its boot window. Today the log prints `SKIP:drafting` forever and no card is
ever sent. It should escalate once, name the row, and say the draft is what is
blocking it.

## ⛔ THE DAEMON'S MODEL OF THE COMPOSER STILL READS A BARE `\n` AS A SUBMIT — right for a shell, wrong for every agent row

**Status:** OPEN

The other half of the paste-draft fix that shipped in 3.0.85. `input_line_has_unsent_draft_after`
(`yggterm-core/src/lib.rs`) reconstructs "is there unsent text in the composer"
from the bytes the client forwards — it is a MODEL of the composer, not a read of
one — and on the un-escaped stream it still does:

    b'\r' | b'\n' if paste_depth == 0 => draft = false,

**That is correct for a `local://` shell and wrong for every agent row.** A shell's
tty is in canonical mode, where `ICRNL` really does turn a bare `\n` into a
submit. An agent CLI runs its tty in RAW mode and reads bytes itself, so `\n` is
inserted as a literal newline and the draft REMAINS — the identical fact that
`efb02e26` fixed in the watchdogs, and that the owner watched happen:
*"the enter key did not send the prompt."*

⇒ **Anything that writes a bare `\n` to an agent row leaves a real draft the
daemon believes is not there.** The consequence is not cosmetic: `--refuse-if-draft`
then types over his text, and `session_is_migratable`'s clause (b) — the one whose
own docstring calls losing unsent work *"the cardinal sin"* — is free to release
that session across a hot restart.

⚠ **Why 3.0.85 did not just fix it too, rather than this being an oversight.**
The bracketed-paste half needed no session kind: `ESC [ 200~` exists precisely so
a receiver can tell a pasted newline from a pressed Enter, so newlines inside the
markers are unambiguously content. This half genuinely needs to know *what kind
of session this is*, and the function is deliberately kind-agnostic — the
docstring one screen above it, on `screen_text_shows_agent_working`, is the
project already warning that a kind-agnostic predicate is the trap
(*"Where the kind IS known … call `AgentCliDescriptor::screen_shows_working`
instead"*). Threading the kind in is the fix; guessing at it in a shared
predicate is how the shell case breaks instead.

⚖ **Blast radius today is smaller than it was this morning** and that is worth
stating rather than overselling the entry: the two writers that actually sent
bare `\n` to agent rows were the booter and babysit, both fixed in `efb02e26`.
What remains is the class — any future writer, any script, any new surface — plus
the fact that the daemon's model and the terminal's reality disagree about the
single most common keystroke there is.

**Falsifier:** write `b"typed\n"` to a live `cc-runtime://` row, then
`server terminal write <row> --stdin --refuse-if-draft`. It answers
`refused_for_draft: false` while the composer visibly holds `typed`. Measured on
3.0.84 and still true on 3.0.85 (arm C of `draftprobe.sh`).

## ⛔ REPORTED: SWITCHING ROWS IS "STILL JANK, BUT MUCH IMPROVED" — p50 711 ms, p90 12 s, AND THE TAIL IS ONE PHASE

**Status:** OPEN

The requirement: *"Probe the switch to active sessions. It is still jank.
But much improved."* ⇒ the recent reveal work (3.0.97/98/100) moved it, and what
is left cannot currently be measured, only felt.

⛔ **FIRST, THE RED HERRING, KILLED — do not re-chase it.** On guihost's trace,
`request_terminal_launch_for_active_begin` fires **58** times and
`..._end` only **17**, which reads like 41 switches that never finished. **It is
not a span.** There are **11 `return` statements** between the two emit sites
(`yggterm-server/src/lib.rs`, ~9489 → ~10008), and every `remote-cc://` row —
which is *every agent row on the GUI host* — takes `parse_remote_cc_session_path`
and returns long before the end event. The 41 are by construction.
⇒ And the 17 that DO pair read **0–1 ms**, because that pair measures a
synchronous bookkeeping call, not a user-visible switch. **An event named
`_begin`/`_end` is not a span until you have read what returns between them**
([[finding-a-substring-test-reads-failure-as-success]] family: the shape looked
like a measurement and was not one).

⛔ **AND THE SECOND RED HERRING WAS MINE: I FILED "nothing measures this". IT
DOES.** `record_terminal_open_attempt_event` (`yggterm-shell/src/shell.rs`
~25506) has emitted a full phase breakdown all along — `request_to_ready_ms`,
`request_to_surface_mounted_ms`, `surface_mounted_to_first_output_ms`,
`request_to_first_meaningful_output_ms`. I concluded it was missing after reading
one server-side bookkeeping pair. ⚠ **It is also not in `event-trace.jsonl`:** the
GUI writes GENERATION-SUFFIXED files (`event-trace.g<gen>.jsonl`), so a scan of
the obvious filename returns zero and reads exactly like "the event does not
exist". ⇒ *glob the generations, and grep the emitter, before declaring an
instrument absent.*

### ⭐ MEASURED ON THE OPERATOR'S OWN SWITCHES — "much improved" is the median, "jank" is the tail

262 switches carrying a `request_to_ready_ms` on the GUI host, 2026-08-10:

| p50 | p75 | p90 | p99 | max |
|---|---|---|---|---|
| **711 ms** | 1,864 ms | **12,058 ms** | 63,863 ms | 315,779 ms |

89 over 1 s · 58 over 3 s · **31 over 10 s**. One switch in ten takes 12 s+.

**The tail is ONE phase, and it is not the mount** (fast = `<=1s`, n=173;
slow = `>10s`, n=31; medians):

| phase | fast | slow |
|---|---|---|
| request → surface mounted | 376 ms | **290 ms** |
| surface mounted → first output | 202 ms | **11,673 ms** |
| request → first *meaningful* output | 613 ms | **22,210 ms** |
| first output → ready | 0 ms | 914 ms |

⇒ **The canvas is up in ~300 ms in BOTH populations — the slow ones marginally
FASTER.** The entire tail is spent on a *mounted, empty* terminal waiting for the
session to emit. `rearm_count` and `observations` are 0 in both, so it is not a
retry loop; `source` is `hot_open_row` in both, so it is not a startup path.

⚖ **TWO EXPLANATIONS SURVIVE THIS DATA AND THEY DEMAND OPPOSITE FIXES — do not
skip to code.**
1. **Real jank:** the viewport is not seeded from the daemon's held screen on
   mount, so the user stares at an empty or stale canvas until the next live byte
   arrives.
2. **A metric measuring the AGENT, not us:** if the viewport IS seeded and
   `first_output` only means "the next live byte", then on an IDLE agent row that
   number is just how long until the agent next spoke — and 315 s is a row that
   said nothing for five minutes, not a five-minute hang.

### ⭐ THE DISCRIMINATOR WAS RUN FROM THE TRACE, AND IT POINTS AT (1)

The mount-time seed exists — `daemon_screen_snapshot_replay_considered`
(`shell.rs` ~94255) picks between the retained buffer and the daemon's screen and
replays it into the canvas. **It fired TWICE across every trace generation on the
host, against 262 switches.** In practice the canvas is not seeded on mount.

And the machinery that would otherwise paint the daemon's held content is
overwhelmingly declining to run:

| `terminal_mount` reconcile outcome | count |
|---|---|
| `screen_reconcile_deferred_recent_output` | 619 |
| `screen_reconcile_skipped_working_surface` | 553 |
| `screen_reconcile_forced_deadline` | 499 |

⇒ **the mechanism is a gate that defers exactly the thing the user is waiting
for**, and one firing in three only happens because a deadline forces it. That is
THE QUIET-GATE LAW's shape again, on the render path this time.

### ⭐⭐ IT IS NOT A BLANK SCREEN — IT IS A STALE ONE, AND NOTHING SAYS SO

Read from the attempt payloads themselves, so no viewport was driven:

| field, over the same populations | slow >10 s (n=31) | fast ≤1 s (n=231) |
|---|---|---|
| `last_overlay_visible` | **False, 31/31** | False, 231/231 |
| `last_surface_problem` | **None, 31/31** | None, 231/231 |
| `last_observed_reason` | `terminal_surface_mounted` | `terminal_surface_mounted` |

⇒ **No spinner, no overlay, no reported problem — the GUI believes the terminal
is fine for the whole 11.7 s.** And the mount is not starting from nothing:
`bootstrap_spawn_skipped_inactive_retained_host` fires **249** times, i.e. the
client REUSES a retained surface, so the canvas carries the row's previous
content.

⇒ **The hypothesis that now fits every number: you switch to a row and see its
LAST-KNOWN screen, correct-looking and unlabelled, which is stale by however long
you were away — and it only catches up when output arrives or a deferred
reconcile finally fires.** That is exactly "much improved, but still jank": it is
no longer blank (the old bug), it is confidently wrong. And it is worse than a
spinner, because a spinner tells you to wait while a stale screen does not
([[bug-class-metadata-vouches-for-clipped-content]] — the surface vouches for
content that is not current).

**The falsifier, and it needs no new code:** switch to a row whose agent produced
output while you were away, and watch whether the visible screen jumps forward
seconds later. If it does, the seed/reconcile gating is the bug. Alternatively,
correlate per switch: `reveal_screen_reconcile` (153 firings) against
`screen_reconcile_deferred_recent_output` (619) for the same session — if the
slow population is dominated by deferrals, the gate is the mechanism.

**And the refresh that would fix a stale screen mostly does not run.** Correlating
`terminal_mount` reconcile events against the 31 slow switches, within each
switch's own window:

| event | fired for slow switches | median offset from switch |
|---|---|---|
| `reveal_screen_reconcile` | **3 of 31** | 1,831 ms |
| `screen_reconcile_deferred_recent_output` | 18 | 31,938 ms |
| `screen_reconcile_skipped_working_surface` | 10 | 30,996 ms |
| `screen_reconcile_forced_deadline` | 10 | 44,115 ms |

⇒ **28 of 31 slow switches never got a reveal-time screen reconcile at all.** That
is the load-bearing row; the reveal-time reconcile is the thing that would replace
a stale surface with the daemon's current screen, and on the slow population it
essentially does not fire.
⚠ **Read the other three rows with care and do NOT quote them as "the catch-up
arrives at 44 s".** Their median offsets exceed the switch's own `request_to_ready_ms`,
which means the window used (`start … ready + 2 s`) is catching reconcile activity
that belongs to the period AFTER the switch completed, not to the switch. The
honest claim from this table is the first row only.

### ⛔⛔ ROOT CAUSE: THE RETRY BUDGET EXPIRES AT ~6.4 s AND THE SCREEN BECOMES WRITABLE AT ~11.7 s

⛔ **This SUPERSEDES an earlier claim in this entry's history that the repaint is
"armed inside a branch that never runs". That was wrong and the data killed it.**
Counting events once per switch, within 8 s of the switch starting:

| event | slow >10 s (n=31) | fast ≤1 s (n=175) |
|---|---|---|
| `bootstrap_spawn_scheduled` | 17/31 | 166/175 |
| **`screen_reconcile_skipped_unwritable`** | **19/31** | **0/175** |
| `reveal_screen_reconcile` (the write happening) | 2/31 | 94/175 |
| `bootstrap_spawn_skipped_inactive_retained_host` | 11/31 | 24/175 |

⇒ the bootstrap DOES spawn and the repaint IS armed. **`skipped_unwritable` at
19/31 vs 0/175 is a near-perfect separator** — the write is armed and then
REJECTED because the daemon screen is empty/launch-seed at the settle deadline.

**The arithmetic, and it is the whole bug** (`shell.rs`):

    const REVEAL_SCREEN_RECONCILE_SETTLE_MS: u64 = 1600;   // :617
    let retry = screen_reconcile_unwritable_retries < 3;    // :95497

⇒ one settle plus three retries ≈ **6.4 s of total patience**, against a measured
`surface_mounted_to_first_output_ms` median of **11,673 ms** on this population.
**The guard gives up roughly halfway to the moment it is waiting for**, then
leaves the stale surface up until live output arrives.
⭐ This is [[finding-a-deadline-shorter-than-its-release-condition]] exactly: a
countdown that can expire before its release condition can physically occur. The
code's own comment even records the symptom — *"live timeline showed a 15 s stale
window the user 'fixes' with a forced refresh"* — and the 3-strike retry was the
fix for it. **The fix was calibrated to a window it does not cover.**

**THE FIX, specified so the next turn is mechanical:** replace the fixed strike
count with a DEADLINE on the condition (keep re-arming while the screen is
unwritable, up to a bound comfortably past the observed p90), or better, re-arm the
reconcile on `attach_ready` (212 firings) — the event that means the remote attach
finished and the daemon screen became writable. ⛔ Keep the WORKING-surface guard
untouched: overwriting mid-turn tears, and that arm is not implicated (2/175 fast,
0 slow). The unwritable arm is safe to extend by construction, because
"unwritable" means there was nothing usable to paint anyway.

⛔ **NOT SHIPPED HERE, deliberately.** It is a render-path change and this
project's law is that a visual fix needs a faithful pixel; the proof requires
switching rows on HIS GUI, and he was typing into it. Ship it with a before/after
screenshot of a row left idle, not on the strength of this arithmetic alone.

⚠ **Still a hypothesis, not a finding.** It is consistent with all six measured
quantities, and no competing explanation now fits `overlay_visible: False` plus a
retained host plus an 11.7 s output wait — but nobody has yet watched a screen
and seen it happen.

⚠ **What this does NOT yet prove**, and the next session must not skip it: that
a human has WATCHED a stale screen catch up during those 11.7 s. The counts prove
the seed did not run and the reconcile deferred; they do not prove what was on
screen. A faithful screenshot taken on an idle row, at switch time, closes it —
and it must be HIS row on HIS GUI, which means asking or waiting for a moment he
is not typing.

**The discriminator, and it is cheap:** switch to a row whose agent is provably
idle and look at the canvas. Content there immediately ⇒ (2), and the metric
needs a different end-marker (seeded-and-typeable, not first-output). Empty or
stale until the agent speaks ⇒ (1), and the seed is the bug. ⚠ His verdict
outranks the number either way — he says it is jank, which already argues for (1)
or for a mixture.
⛔ Note the connection to the reveal ceiling: if `ready` requires first output,
an IDLE row can never become ready promptly, and the 60 s "did not become
interactive" timer is aimed at a condition that row cannot satisfy — the same
family as the shape-B skip fixed in 3.0.98/3.0.100.

## ⭐ THERE IS NO WAY TO MINT A TEST AGENT SESSION, SO THE GATE'S AGENT ARMS CANNOT BE SANDBOXED

**Status:** OPEN

Three tooling gaps found while trying to live-prove 3.0.101's `orchestrating`
blocker. Each is the dream test — *an agent hand-assembled this chore from
primitives and got it wrong* — so each wants to be a verb.

1. **No agent-kind session outside the real launch path.** `server attach
   <cc-runtime://…> <cwd>` mints a `shell` (it genuinely spawns one, so calling
   it an agent would be a lie), and pre-seeding `server-state.json` with
   `kind: claude_code` is **overwritten on attach**. ⇒ every predicate keyed on
   `session_kind_state_survives_pty_loss` — the whole restorable/not-restorable
   axis the gate turns on — is unreachable in a sandbox, and the 3.0.81 agent arm
   that the campaign memory calls *"the load-bearing one"* had to be run against
   a live daemon on the real host. A `server attach --kind <k>` (sandbox-only, or
   refusing unless `YGGTERM_HOME` is non-default) would make the gate's arms
   ordinary tests instead of expeditions.
2. **No way to ask the product what it thinks a transcript is.** Checking that
   `newest_transcript_writer` / `newest_subagent_transcript` agree with reality on
   real data required building a throwaway cargo crate against `yggterm-core`.
   A read-only `server sessions classify <session-path|file>` printing the writer,
   the newest sub-agent file and its age would have answered it in one line — and
   is the instrument the NEXT person debugging a wrong `orchestrating` verdict
   will need anyway.
3. ⛔ **RETRACTED BEFORE IT WAS PUSHED — the verb was innocent and the READER was
   wrong, which is the more useful finding.** This was filed as "`server app
   session remove` answered with every field null". It did not: the response
   envelope puts the payload under **`data`**, and the extractor read `result`,
   so every field came back `None` from a dict that never had them. Re-run raw,
   the same call answers `verified: true`, `row_still_listed: false`,
   `live_processes: []`, `message: "no live session for …"`.
   ⇒ **An all-null read is a shape mismatch until proven otherwise.** Filing it
   as a defect would have sent the next session to audit a verb that works, and
   the retraction is cheaper than the audit. Kept rather than deleted because the
   envelope really does have two plausible keys (`data` vs `result`) and the next
   reader will guess the same way.

⚠ Also: the pre-push privacy guard's `aadhaar-like` rule fires on **invented
UUIDs** whose last group is twelve digits with no hex letters in it. Failing closed is
correct and the override was NOT used — the ids were rewritten with hex letters.
But the cheap fix is guidance, not code: *invent test ids with letters in them.*

**Falsifier:** `server attach --kind claude-code <key> <cwd>` exists and the
resulting row reports a non-permanent blocker set; `server sessions classify`
exists. (The third item is retracted, not open.)

## ⚖ THE WORKING DOT — the discovery half, and it is NOT a detector defect

**Status:** OPEN

⚠ The open question is a SPEC question, not a bug — see the bottom of this entry.

The owner asked for a working indicator. 6.3 owns the RENDER half; this entry
owns the prior question — **what makes a session's working-state knowable at
all**, and why `working` is `None` on most rows.

⛔⛔ **THIS ENTRY WAS FILED TWICE WITH A WRONG PREMISE AND IS REWRITTEN FROM
MEASUREMENT. Both retractions are kept, because the retracted versions were
circulated to another lane before they were checked.**

### The answer: a live PTY is the discriminator, and `None` is honest

Measured on the GUI host, agent rows only:

    live PTY present  →  7 answered / 7      (100%)
    no live PTY       →  2 answered / 43     (the 2 are proxy-filled)
    by locus:  local 9/9 answered · remote 9 answered, 41 None

⇒ **Every row with a live PTY has a working answer.** A row with no PTY is not
open — nothing is running for any daemon to observe — and `None` is the correct
report, exactly as the snapshot's own code says: *"`None` = no live screen
(preserved/foreign-owned) so the GUI must NOT blink it."*
⇒ **The "21 of 31 rows report None" count is dominated by rows that are not
open.** There is no detector to fix.

### ⛔ RETRACTED #1 — "a row this daemon does not own is unaskable"

False. `refresh_proxied_working_flags` + `working_flags_including_proxied`
already ask the OWNING daemon for exactly the rows this one cannot answer, and
cache the result because a per-snapshot fan-out would cost a round trip per row
per frame. That capability was added for this very symptom.

### ⛔ RETRACTED #2 — "the `}?;` drop conflates unknown with idle"

Also false, and this one nearly shipped as a code change. `working_flags` ends in
`?`, so an unreadable session is absent rather than reported — and **absent is
the right encoding for both of its consumers**: the proxy refresh uses it to
decide which rows still need fetching (a session reported as unknown would mark
its own hole as covered), and the 2.5 s poll only wants definite answers to stamp
freshness with. Meanwhile the SNAPSHOT — which is what the dot actually reads —
computes its own three-state `Option<bool>` separately and documents each state.
⇒ **Two paths, two correct encodings.** A change making `working_flags` return
`Option<bool>` was written, compiled, and reverted unshipped once the premise
collapsed: nothing consumed the new information, and unused churn in this
subsystem is worse than none.

### ⇒ WHAT IS ACTUALLY OPEN, and it belongs to the owner, not to a fix

The dot can only mean "working" for rows that are OPEN — **16 of 50 on the GUI
host**. For the other 34 the honest states are "not running" and "closed", which
is a design question: *what should a closed row's dot say, and should it be the
same shape as a running-but-idle one?* Until that is answered, any render is
guessing, and the guess it makes today is attachment — which is what the owner is
reading as activity.
⚠ **Not a third state invented at the view layer.** If a new distinction is
wanted, its source is the snapshot's existing three-valued `working` plus
whether a PTY exists; both are already on the wire.

## ⚖⚖ THE HOT-RESTART GATE IS UNBUILT — THE DESIGN IS NOW SETTLED

**Status:** OPEN

**This is the CONSTITUTION's unmet guarantee and the highest-value work in the
project.** The design was settled 2026-08-08 and is no longer an open
question: [`spec-hot-restart-relay-gate.md`](spec-hot-restart-relay-gate.md) owns
it, [`settled-calls.md`](settled-calls.md) owns his ruling. ⛔ The former
prohibition on deadlining the gate is SUPERSEDED; do not cite it.

**Live evidence, measured while the spec was written:** GUI **3.0.67**, daemon
**3.0.65**, older daemons alive at 3.0.62 / 3.0.59 / 3.0.29, and

    hot_restart_pending      true
    hot_restart_blockers     []
    hot_restart_block_reason null
    last successful swap     241 minutes earlier

⭐ **A gate reporting that nothing blocks it while not firing is worse than one
held by a named blocker** — there is nothing to clear, so no human or agent can
help it along. Spec §8 makes "report a nameable reason, or fire" a requirement.

**What has to be built** (spec sections in brackets): drive swaps from relay
boundaries rather than polling for silence [§2] · classify sessions
idle/blocked-on-human/working/orchestrating instead of inferring from output
[§3] · queue requests so none is lost [§4] · a 30-minute deadline that forces
the swap **and** injects `continue` into exactly the sessions it interrupted
[§5] · an unbounded wait for sessions running sub-agents [§6].

⚠ **Two things that must be true before it ships** and are easy to get wrong:
sub-agent detection must be POSITIVE (it is the state with an unbounded wait, so
a merely-busy session must not reach it), and **the interrupted set must be
computed before the old daemon dies** — after the swap every interrupted session
looks idle, so the list cannot be re-derived.

**Landed so far — §3's first increment, in 3.0.81.** The gate's release condition
now asks what a session *is* before asking how quiet it has been. A session whose
state does not outlive its PTY (`session_kind_state_survives_pty_loss` — a plain
shell) blocks the cold shutdown **permanently**, and `HotRestartBlocker` carries
`permanent: true` so a reader can tell "waiting for a moment that will come" from
"lingering on purpose, forever". The summary names a clearable blocker first, so
the headline is never a session the user is not supposed to close.

⭐ **Fresh live evidence, and it is better than the 2026-08-08 sample above
because the blocker is NAMED.** On the GUI host, daemon pid 426042 (3.0.75, a
successor at 3.0.80 already live) logged `daemon_cold_shutdown_deferred_idle_gate`
**823 times across 275 minutes** — one every 20 s, never once opening — with the
same single blocker every time: a `local://` ychrome shell, `idle_ms: 632`
against a 300 000 ms window. ⇒ the 2026-08-08 reading *"0 of 40 samples"* is now
0 of 823, and the QUIET-GATE LAW's premise is not a theory about agent CLIs — a
plain shell hosting a browser is just as never-silent.

**Landed — §6 and the sub-agent half of §3, in 3.0.101.** ORCHESTRATING is a
named blocker (`HOT_RESTART_BLOCKER_ORCHESTRATING`), read POSITIVELY from the
agent's own `subagents/agent-*.jsonl` and checked BEFORE the
`YGGTERM_HOT_UPDATE_IGNORE_IDLE_GATE` override, because the override waives
waiting and §6's wait is a refusal to strand another agent's delegates. It is
NOT `permanent` — it clears when the delegates stop writing — so `server daemons`
still separates "deferring" from "lingering". `hot_restart_blocker_is_deadline_exempt`
is the predicate §5's deadline must read; it is derived from the kind so the gate
and the deadline cannot come to disagree about who may be interrupted.

⭐ **The measurement that justifies the unbounded wait, taken on `dev` across
every Claude Code transcript on the host:** of **73,764 sub-agent records in 33
sessions, 21,453 (29.1%) were written while the parent transcript was silent for
over a minute**, and the longest such silence around a live sub-agent record was
**30.6 minutes** — past the gate's 300 s window and past §5's 30-minute deadline.
Those sessions had no blocker of any kind before 3.0.101.

⛔ **And the instrument that looked obvious is the wrong file.** `isSidechain` in
the PARENT transcript is `false` on **all 179,392 records on this host**, across
sessions that made **195 `Agent` and 29 `Workflow` calls** — sub-agents write to
`<session-id>/subagents/agent-<id>.jsonl` instead. A gate built on the parent
would have compiled, shipped, read as correct in review, and never once fired.
[[finding-a-set-is-not-a-fill]] shape: the field was present everywhere and
carried no signal.

⚠ **What 3.0.101 is PROVEN by, stated exactly, because the last arm is missing.**
Proven: five mutation-falsified parser tests (scan-direction, detector-disabled
and admission-test mutations each caught by a *different* subset); and three arms
run through the SHIPPED functions against REAL fleet data — a genuine sub-agent
transcript classifies `SubAgent`, its parent `MainLoop`, and a delegate that
finished 16 days ago correctly ages out of the window rather than pinning the
daemon. **NOT proven: the `orchestrating` blocker observed appearing in a live
daemon's `server status`.** That needs a live session with a live sub-agent, and
the agent doing this work operates under a standing instruction not to spawn
sub-agents — so the input cannot be manufactured, only waited for.
⇒ **Falsifier for whoever has one running:** on a host whose newest daemon owns a
local `cc-runtime://` agent row, have that agent launch a delegate, then
`yggterm-headless server status` — the row must appear with
`kind: "orchestrating"`, `permanent: false`, and a reason saying it is *"waited
for without a deadline"*. It must disappear within the idle window after the
delegate finishes. ⛔ A blocker that appears and never clears is the disease, not
the fix; check the clearing half too.

⚠ **Found while trying to run that arm, and it is a separate defect — a freshly
created row is `not_restorable` on the daemon that owns its PTY.** On guihost
2026-08-10, `server app terminal new --kind claude-code` produced
`local://fe774cfd-…`; the 3.0.101 daemon reported it as an owned blocker of kind
`not_restorable` while **that same daemon's `server snapshot` did not contain the
row at all**. So the daemon owns the runtime, cannot resolve a session record for
it, and `live_session_kind` → `None` → the safety bias (correctly) refuses. The
bias is right; the state it fires in is not. **A permanent blocker on every
newly-created agent row would pin that daemon's cold shutdown for as long as the
row lives**, which is the exact shape 3.0.81 fixed for shells.
**Falsifier:** create a row with `server app terminal new --kind claude-code` and
compare, on the daemon that reports it as a blocker, `hot_restart_blockers`
against `server snapshot`. Today the key appears in the first and not the second.
⚠ Not yet established: whether the record promotes later (making this a startup
race) or never (making it durable). Measure before fixing.

⚠ **HOW FAR 3.0.101 IS PROVEN, and where it stops.** Everything below the gate
is proven on REAL data via the shipped functions: a genuine sub-agent transcript
classifies `SubAgent`, its parent `MainLoop`, a session that never delegated
answers `None`, and a delegate that finished **16 days ago** correctly ages out
(the arm that stops an unbounded wait from being unbounded). Five parser tests
are mutation-falsified — scan-direction, detector-disabled and the record
admission test are each caught by a different subset. The NEGATIVE arm is
live-proven at fleet scale: guihost's 3.0.101 daemon owning **6 real agent rows**
reports only `working`/`recently_active`, with no spurious `orchestrating`.

⛔ **NOT proven live: the blocker appearing in a daemon's `hot_restart_blockers`.**
It is not manufacturable from here, and the reason is worth recording so the next
session does not spend the same hours on it. `server attach` mints a `shell` by
design (correctly — it really does spawn one), and a shell is `not_restorable`,
which blocks FIRST and hides the state under test; seeding an agent-kind record
into `server-state.json` does not survive, because attach rewrites `kind`. The
real launch path (`server app terminal new --kind claude-code`) does create a
genuine `claude --session-id <row-id>` process — ⭐ **so a LOCAL CC row's yggterm
id IS its CC session id**, which is why the existing `local://` branch already
resolved — but a fresh session writes no transcript until its first turn, and the
one thing that would complete the arm is a session actually running a delegate.
⇒ **Catch it in the wild instead:** `orchestrating` in `server status`
`hot_restart_blockers`, or `daemon_cold_shutdown_deferred_idle_gate` trace events
whose `blockers[].kind` is `orchestrating`. Neither existed before 3.0.101, so
the first occurrence is the proof.

### ⭐ THE ROOT CAUSE OF THE VERSION SKEW, FOUND 2026-08-13 — ONE SHOT, NO TARGET

The gate was never what kept the hosts stale. **The swap intent was, and it was
being thrown away twice on every host.**

**(a) The self-retire handoff never named its target.**
`attempt_self_retire_preserving_handoff` called `hot_restart_detailed(…,
expected_version: None, …)` on the stated reasoning that *"we cannot cheaply read
the successor's version here (the on-disk binary already IS the new version, so
the spawned successor comes up correct)"*. Both halves were false:
`yggterm_executable_reported_version` is one `--version` on the very binary about
to be spawned — and `try_startup_stale_daemon_hot_swap` **already paid it, on its
own lane, for this exact decision** ([[finding-a-claim-proven-on-one-lane-is-not-proven]]).
With `None` the handler never promotes install-state, so under a managed Direct
install the spawned child re-execs back to the OLD active version, finds the
socket bound and exits 0; and the handler's "a live daemon at or above the target
IS the successor" shortcut cannot evaluate, so the doomed spawn is the only path.

**(b) The retire poll `break`s after the first accepted handoff.** The one thread
that could notice the swap had not landed, and retry, exited.

**Live trace, GUI host, 2026-08-13** — daemon 2232011 (3.0.118), whose on-disk
binary was 3.0.120 and whose GUI was 3.0.120:

    13:45:52  daemon_self_retire            {retire_trigger: "disk_binary_replaced"}
    13:46:02  hot_update_handoff_prepared   {spawn_ok: true, expected_version: null}
    13:46:02  daemon_self_retire_handoff_ok {outcome: "preserved_owner_handoff"}
              …and nothing further, ever.

⭐ **`daemon_self_retire` fired exactly ONCE in that daemon's lifetime.** Forty-five
minutes later the host still had no daemon at the GUI's version, and nothing
anywhere recorded that one was owed. The workshop host had stacked **eighteen
coexisting daemons the same way, the oldest alive 20.6 days**; the GUI host five,
the oldest 193.9 h. `spawn_ok: true` is the tell — it records that the *spawn*
worked, which is a different question from whether a successor exists.

⛔ **And the daemons already stacked cannot be reached by any of this.** The GUI
host's four lingerers run 3.0.29/3.0.70/3.0.75/3.0.76 — all pre-3.0.81, so they
have neither `not_restorable` nor `permanent` and never will. A gate change
reaches the NEXT generation of daemons only; the standing pile is residue, and on
the GUI host it is pinned by ychrome shells whose PTYs may not be destroyed, so
lingering is the correct end state for them.

⚠ **A second, narrower defect found beside it, not yet fixed:** the progressive
migration drain is started only from a handoff (`retire_trigger ==
"disk_binary_replaced"`, or a `HotRestart` RPC). A daemon retiring under
`retire_trigger == "newer_daemon_live"` — the deploy shape that RENAMES the old
binary aside rather than unlinking it, so `/proc/self/exe` is not `(deleted)` —
never arms the drain at all. Measured: pid 426042 (3.0.75) logged
`daemon_self_retire {retire_trigger: "newer_daemon_live"}` every 20 s for
**100.6 hours** with no drain ever running.

⭐ **§4 IS LIVE-PROVEN END TO END, on the workshop host, 2026-08-13.** Three real
daemons, three real deploys, one queue slot throughout:

    15:52:29  pid 3795572 (3.0.125)  hot_restart_swap_queued  {decision: "queued",     target: 3.0.126}
    15:58:27  pid 3922471 (3.0.126)  hot_restart_swap_queued  {decision: "superseded", target: 3.0.127}
    15:58:54  pid 4009904 (3.0.127)  hot_restart_swap_queue_satisfied
                                     {satisfied_by: "self", waited_ms: 38009}

and `hot_update_handoff_prepared` now carries `expected_version: "3.0.126"` where
it used to carry `null`. While the swap was owed, `server daemons` printed
`swap owed → 3.0.127: queued 0m ago by disk_binary_replaced, 1 attempt(s), last:
handoff requested; successor not yet confirmed live`; when it was satisfied the
line went away. Supersede replaced the older target **in the same slot** rather
than adding a second entry.

⛔ **And the live proof found a defect the tests could not — the record outlived
its own satisfaction.** On the first run the successor came up and adopted all
NINE of the writer's sessions; the writer then had empty hands, fell through to
the cold-shutdown gate, found nothing blocking, and exited — taking the only
process that would ever have cleared the entry with it. `server daemons` went on
printing `swap owed → 3.0.126` while 3.0.126 was serving every row on the host.
⇒ **The entry is cleared by whoever SATISFIES it, never by whoever wrote it.**
Every daemon now asks on every poll whether IT satisfies the queued swap. ⚠ That
is not a second copy of the check inside the swap lane: that one asks *"is a
successor live?"* on behalf of a daemon still holding PTYs, this one asks *"am I
the successor?"* — and only the successor is guaranteed to still be running when
the answer turns true.

**Landed — §4, the queue, in 3.0.124–3.0.127.** `hot_restart_queue` is the host's durable
record of the one swap it owes (`~/.yggterm/hot-restart-queue.json`): a single
slot, superseded by a newer target rather than appended to, and ⛔ **a re-request
for the target already queued must not move `requested_at_ms`**, because §5's
deadline is measured from it and a clock that resets on every poll is the
never-converging gate rebuilt one layer up. The retire poll no longer `break`s: it
queues, lingers, and retries on a five-minute floor that is enforced
process-locally as well as in the file, so a peer's write cannot hand this process
a fresh allowance to spawn another successor. `server daemons` prints the owed
swap and the reason the last attempt gave (§8: *"something must be nameable as the
thing it waits for"*). The self-retire handoff now probes the replacement binary
and passes its version.

**Still unbuilt:** the rest of §3 — **blocked-on-human is not yet a state**, and
an agent session's WORKING state is still inferred from silence rather than from
a positive signal.

⛔ **AND BLOCKED-ON-HUMAN MUST NOT BE BUILT BLIND — there is currently no
instrument that can validate the recognizer it needs.** Attempted 2026-08-13 and
stopped deliberately, so the next session does not spend the same time
rediscovering it. What was measured, and why none of it is a corpus:

- **The gate reads a source no external agent can audit.**
  `hot_update_idle_gate_blockers` classifies from
  `terminals.session_screen_snapshot(runtime_path)` — the live in-daemon vt100
  screen. `server snapshot`'s `live_sessions[].terminal_lines` is a DIFFERENT
  field: of 225 agent sessions on one host, **205 last lines were a stored
  summary line, not screen text at all**, and only ~20 carried real
  escape-bearing screen tails. Auditing the gate's input from outside is
  therefore not possible today with any shipped verb.
- **The obvious sample says nothing.** Across those same 225 sessions,
  **0 matched any question / permission / numbered-choice pattern** and 10
  contained `esc to interrupt`. That is not evidence that agents never park at a
  question — it is [[finding-a-set-is-not-a-fill]] again: the field was present
  on every record and carried no signal, because most of it was never a screen.
- **The failure mode is asymmetric and expensive.** A false BLOCKED-ON-HUMAN
  means a session that is genuinely mid-turn is classified as not-working and
  cold-killed. A recognizer written against invented prompt strings would
  compile, review as correct, and fire on the wrong screens — the exact shape
  that made `isSidechain` read `false` on all 179,392 records.

⭐ **THE PREREQUISITE IS BUILT — `server gate-screen`, 3.0.132.**
`yggterm-headless server gate-screen [<session-key>] [--tail <n>] [--json]`
answers, per owned session, with the screen
`hot_update_idle_gate_blockers` classified from AND the blocker that
classification produced — the blocker taken from the gate's own function rather
than re-derived, so a session this verb calls unblocked is unblocked in the
gate's eyes. Escape sequences are kept: a parked-at-a-permission-prompt screen
is told apart partly by how it is DRAWN, and a reading stripped for legibility
would hand the recognizer a corpus the gate never sees.
⛔ **Read-only, on demand, and the screen never reaches a trace event** — pinned
by `the_gate_screen_verb_never_writes_a_screen_to_the_trace`, because
`event-trace*.jsonl` is durable, is copied around in bundles, and a session's
screen carries whatever the person typed.
⛔ **Denied to a shadow client, and the reason is SCOPE, not read-only-ness**
(`a_shadow_client_may_not_read_every_session_on_the_host`): a shadow is a viewer
of ONE session, and this answers with every session the daemon owns.
⇒ **THE HARVEST WAS RUN, AND IT ARGUES AGAINST BUILDING THE RECOGNIZER YET.**
`server gate-screen` was swept across all three hosts: **52 sessions, 51 with a
readable screen** — the corpus that did not exist before (the old instrument gave
205-of-225 stored summary lines rather than screen text).

    esc-to-interrupt      10 / 51        blocker kinds: working 10 · recently_active 12
    numbered-choice        2 / 51                       not_restorable 13 · none 17
    trailing '?'           2 / 51        shows_agent_working: 10
    permission wording     1 / 51

⛔ **Zero validated parked-at-a-question screens, and the two pattern hits are
FALSE POSITIVES.** Inspected structurally: 3 numbered lines among 11 and 36, **no
line ending in `?`, and no selection caret on any numbered line** — ordinary
numbered output, not a prompt. Both were also already `blocker: none`, i.e. the
gate was not holding anything for them.
⇒ **A recognizer written today would be validated against nothing and would fire
on ordinary output** — precisely the asymmetric failure this entry warned about,
where a false BLOCKED-ON-HUMAN gets a mid-turn session cold-killed.

⭐ **And one structural measurement changes what a recognizer should look at:
50 of 51 screens end MID-OUTPUT**, while 37 of 51 show a composer glyph somewhere
in the tail. So "is the last line a prompt" is the wrong shape of question. ⇒ The
discriminator worth testing when real samples arrive is **the selection caret on
a choice line** — a structural mark, not a phrase — which is exactly what both
false positives lacked. That also explains why stripping escapes for legibility
would destroy the signal, and why `gate-screen` keeps them.

⇒ **What is left of §3, in order:** keep sampling until real prompts are caught
(a periodic harvest is cheap, bounded and risks nothing), then write the
recognizer against the caret hypothesis. **Do not write it from invented prompt
strings** — the corpus says the strings are not there to be guessed at. ⚠ The recognizer also cannot be
manufactured on demand without driving a live session into a permission prompt,
which is not something to do to another agent's row.

**Landed in code, LIVE PROOF OWED — §2, the relay boundary as the appointment.**
`server relay-boundary [--by <who>] [--wait-secs <n>] [--json]` declares that a
hand-off just happened, and the queued swap is then attempted on the next 20 s
drainer poll instead of waiting out `HOT_RESTART_SWAP_RETRY_INTERVAL_MS`.
`ygg-claim.sh` declares one after a predecessor is retired **and reaped** — the
rename is not a quiet point, a reaped predecessor is. The boundary is a field on
the existing queue entry, not a second file, because it does not change WHAT the
host owes, only when it may next try.
⛔ **Two floors, and releasing one is releasing none.** The file's
`attempt_is_due` and each drainer's process-local
`HOT_RESTART_SWAP_LAST_ATTEMPT_MS` both gate the retry; a bypass that cleared
only the first would print success and change nothing. Pinned by
`a_relay_boundary_releases_the_process_local_floor_too`.
⛔ **One boundary buys exactly one attempt**, derived as
`relay_boundary_at_ms > last_attempt_ms` rather than stored as a flag — a
boundary that stayed unspent would release the floor on every poll, which is the
fork bomb the floor exists to prevent, rebuilt by the thing meant to bypass it
safely.
⭐ **§2 IS LIVE-PROVEN, 3.0.130, and the falsifier ran exactly as written.** A real
daemon in a real retire loop, owning a real PTY, with a queue entry it could not
converge (the replacement binary reported a newer version and never came up — the
"successor that never arrives" case the retry exists for), in an isolated
`YGGTERM_HOME` so no fleet row was touched:

    19:18:33  attempt 1        last_outcome: "handoff requested; successor not yet confirmed live"
    19:19:16 … 19:20:37        attempts PINNED at 1 across five 20 s polls — the floor holding
    19:20:49.937              server relay-boundary --by … → Declared {waiting_ms: 136779}
    19:21:03.363  attempt 2   last_outcome: "a relay boundary was declared; retrying the handoff at it"
    19:21:13 … 19:21:50       attempts STAY at 2 — one boundary bought exactly one attempt

⭐ **13.4 s from declaration to retry against a 5-minute floor**, i.e. the next
drainer poll. Both floors were released by the one boundary and both had to be:
the process-local static lives in that same daemon process, which had recorded
its own attempt 136 s earlier, so a bypass of the file alone would have printed
success and changed nothing. And the boundary being SPENT is what stops it there
— `boundary_at_ms < last_attempt_ms` after the retry, so the floor re-engages
rather than releasing on every poll.
⚠ Honest scope: the queue's CLI-level behaviour (no-op on a converged host,
`requested_at_ms` untouched, the census naming an unspent boundary) was already
proven; what this adds is **the daemon draining at the boundary**, which was the
open half. The synthetic element is the successor binary, not the daemon, the
queue, the floors or the poll.

**Landed in code, LIVE PROOF OWED — §5, the deadline AND its repair.** They ship
together or not at all. `hot_restart_deadline_verdict` applies
`HOT_RESTART_FORCED_SWAP_DEADLINE_MS` (30 min) to a blocked **cold shutdown**;
`crates/yggterm-server/src/hot_restart_repair.rs` is the durable record of who it
interrupted, and every daemon's poll dispatches a `continue` to exactly those
sessions, once.
⚠ **Scoped to the cold-shutdown gate on purpose, NOT to the progressive
migration.** Under `disk_binary_replaced` the successor is already serving — the
swap has happened, and only ownership tidiness remains, which is not worth
interrupting a live turn for. The path where nothing has swapped and the host
stays stale is the cold gate, and that is what the deadline is aimed at.
⛔ **One exempt blocker vetoes the whole force.** A cold shutdown kills every PTY
this daemon owns, so there is no "interrupt the working session but spare the
shell beside it"; §6's orchestrating wait and a plain shell both hold it open
indefinitely, and the verdict NAMES which one (§8).
⛔ **The interrupted set is recorded BEFORE the shutdown**, pinned source-level by
`the_forced_swap_records_its_interrupted_set_before_shutting_down`, because the
process that knows the list is the one about to exit and §8 says it cannot be
re-derived.
⛔ **The record is spent on DISPATCH, and it EXPIRES** (`REPAIR_WINDOW_MS`, 10
min). A lost repair beats a `continue` typed twice into someone's session, and a
repair arriving half an hour late is not a repair — it is the unprompted nudge §5
forbids.
⚠ **The `continue` goes through the echo-verified submit**, which was extracted
into `submit_prompt_echo_verified_with` so the daemon can drive it while holding
its runtime lock only per-write. A just-resumed agent CLI draws its composer
before its input loop is live, so a `continue` written at "prompt shown" is
swallowed silently.
⭐ **THE REPAIR HALF IS LIVE-PROVEN, 3.0.130, all the way down to the PTY** — and
it is the half that could not be proven any other way, because everything
interesting about it happens between a daemon's runtime lock and a program's
input loop. Isolated `YGGTERM_HOME`, a real daemon owning a real shell, an
interrupted record naming that key and a foreign `recorded_by_pid`:

    server daemons   repair owed: `continue` for 1 session(s) interrupted 0s ago
                     by pid <other> (3.0.130), window 600s — local://<key>
    trace            hot_restart_repair_continue {outcome: "submitted", error: null}
    the session's own stream, in order:
                     yggterm_ready_probe   ← written, and ECHOED (input loop live)
                     ^U                    ← the probe cleared
                     continue  \r          ← the text, then a LONE carriage return
                     bash: continue: only meaningful in a `for', `while', or `until' loop

⭐ The shell's complaint is the proof: the `continue` did not merely reach the
PTY, it was **submitted as a line**, which is the thing a concatenated `\r` does
not do. The record was cleared on dispatch, so the next poll took nothing.

⭐⭐ **§5 IS NOW LIVE-PROVEN END TO END, BOTH HALVES — and the proof found a real
defect in the second one, which is what it was for.** Run in
`scripts/underglass-sandbox.sh`'s private sway with a private `YGGTERM_HOME` and
a sandbox-owned binary pair, so nothing of the fleet's was touched:

    hot_restart_forced_past_deadline {
        retire_trigger: "disk_binary_replaced", owed_for_ms: 1862006,
        reason: "local://<row> was active 44s ago (idle window 300s)",
        interrupted: ["local://<row>"], repair_recorded: true }
    …the daemon cold-shuts-down; a successor comes up and adopts the key…
    hot_restart_repair_continue { session_key: "local://<row>", outcome: "not_ready" }

⭐ **The recipe, because the entry said for days that this could not be built:**
a full GUI in the private sandbox, `server app terminal new --kind claude-code`
for a genuine agent row, `YGGTERM_DISABLE_SELF_RETIRE_HANDOFF=1` so the lane
reaches the cold gate, a queue entry stamped 31 minutes old (the file's own
documented clock), and a `mv` over the SANDBOX copy of the binary.
⛔ **And the belief that blocked it was stale.** The entry recorded that a freshly
created agent row is `not_restorable` — permanent, exempt, vetoing the force.
Measured now: it classifies **`recently_active`, `permanent: false`**, i.e. an
ordinary interruptible blocker. That defect was fixed at some point and the note
outlived it, which is why `owed_for_ms: 1862006` reached a `Force` at all.

⛔⛔ **THE DEFECT THE FALSIFIER FOUND: `outcome: "not_ready"`, and the record was
already spent.** A just-re-resumed agent CLI had not brought its input loop up
inside `HOT_RESTART_REPAIR_SUBMIT_TIMEOUT` (20 s) — exactly the hazard the code's
own comment predicts — so the session was **interrupted by the deadline and never
repaired**. That is the deadline shipping alone.
**Fixed (3.0.136+): a `NotReady` key goes back on the record**
(`requeue_unsubmitted`). It does not weaken at-most-once: `NotReady` is *proof
nothing was written* — the probe never echoed and the submit clears the composer
on its way out — so a retry cannot double-type, where a `Submitted`, `NoSession`
or `Err` outcome stays spent because it is ambiguous.
⛔ **The requeue never restamps.** `REPAIR_WINDOW_MS` is measured from the
interruption; a requeue that moved the stamp would keep a failing repair owed for
ever and eventually type into a session that has been back for an hour — the
never-converging clock this project has already fixed once. Pinned by
`a_continue_that_was_never_written_goes_back_on_the_record` and
`a_requeue_never_drags_a_newer_interruption_backwards`.
⇒ **Still owed on this half:** a `hot_restart_repair_continue {outcome:
"submitted"}` observed after a real forced swap — the requeue makes it reachable
but has not been watched converging.

⚠ **And the deadline's clock only starts where the swap lane gives up.** The cold
gate is reached from `SwapStep::Failed`, and a host with a queue entry it keeps
retrying returns `Lingering` instead — so on that host the deadline is never
evaluated at all. Its real trigger is a daemon retiring under
`newer_daemon_live` (or with empty hands, or with the handoff kill-switch set),
where `hot_restart_retire_owed_for_ms` falls back to the queue entry's
`requested_at_ms`, or to this process's first poll when nothing is queued.

⭐ **§4's second producer IS live-proven, on the GUI host, 3.0.130.**
`queue_startup_swap_intent` records the intent when
`reconcile_stale_daemon_on_startup` declines; a source-level test pins the call
site (`the_startup_reconcile_queues_the_swap_it_declines_to_take`), which is the
right shape because the defect is a missing call on an early return. What the
live host showed:

    startup_hot_swap_declined_swap_queued {decision: "superseded", target_version: "3.0.130",
        stale_pid: …, stale_version: "3.0.118", owned_terminal_session_count: 9}
    …then TWICE more {decision: "unchanged"} — and requested_at_ms did NOT move

⭐ The `unchanged` repeats are the second half of the same proof: §5's deadline is
measured from `requested_at_ms`, and a GUI that re-declines every poll would have
reset that clock forever. It does not.

### ⛔⛔ A QUEUE WHOSE CONSUMER CAN BE OLDER THAN ITS PRODUCER HAS NO FLOOR

**Open. The manual repair below works and is safe; the automatic one is unbuilt.**

Measured on the GUI host 2026-08-13, and it reached the owner as *"I cannot type
in many sessions"*: the host sat **twelve versions behind for 5.5 hours** with a
queue entry reading `attempts: 0`. The reading everyone reached for — *"the idle
gate is deferring, and the agents that make the host busy are the same agents
whose activity resets the 300 s window, so an observer joins the blocker set by
observing"* — is a good story and it was **not what was happening**. `attempts: 0`
is nobody reading, not deferral.

⭐ **The queue's only consumer is a daemon's retire poll.** The producer is
whatever notices the host is stale — including the GUI, which is always current
because the user just launched it. So the two sides of the record sit on opposite
sides of the very version skew the record exists to close, and **a host whose
newest daemon predates the queue can write down what it owes and can never act on
it.** No amount of correctness in the gate reaches that; the gate is a bystander.

⚠ **Its shape as a rule, because it will recur wherever a durable record spans a
version boundary:** a producer can always be assumed current (someone just
started it), a consumer cannot (it is the thing being replaced). ⇒ **A record
whose only reader is the component being superseded needs a reader that is not.**

**The manual repair, and it is safe, additive and reversible** — start a daemon
at the on-disk version alongside the stale one, then let the existing machinery
converge:

    # ⛔ THE ENVIRONMENT IS THE LOAD-BEARING PART, NOT A DETAIL.
    # A daemon FREEZES its launch environment and every session it ever spawns
    # inherits it — see [[finding-daemon-frozen-env-poisons-sessions]]. A daemon
    # started from a helpful ssh shell has no WAYLAND_DISPLAY, no session bus and
    # the wrong PATH, and it poisons that host's sessions permanently, with no
    # error anywhere. Launch it with the GUI's OWN environ:
    python3 - <<'EOF'
    import subprocess
    raw = open("/proc/<gui-pid>/environ","rb").read().decode(errors="replace")
    env = dict(x.split("=",1) for x in raw.split("\0") if "=" in x)
    subprocess.Popen([DAEMON_BIN,"server","daemon"], env=env, start_new_session=True)
    EOF

What followed on its own, inside 60 s, with no eviction and no PTY deaths:

    pty_handoff_listener_bound                    ← the missing server-<v>.sock now exists
    pty_handoff_adopted                       x7  ← the stale daemon's PTYs, handed over ALIVE
    preserved_owner_live_sessions_restored    x4
    superseded_daemon_takeover
    hot_restart_swap_queue_satisfied {satisfied_by: "self", attempts: 0, waited_ms: 1027179}

⭐ **17 minutes of preserved intent, satisfied by the successor asking "am I the
successor?"** — the §4 clause that exists because the writer cannot own the
record's lifecycle. The superseded daemon then retired on its own terms and left
the table; the host's four ancient lingerers were untouched, still holding their
shells. ⇒ **A second daemon does not disturb a pending hot-restart handshake, it
completes it**, which is the constitution's version-coexistence clause behaving
exactly as written.

⭐ **LANDED, 3.0.135 — the GUI's startup decline now STANDS THE SUCCESSOR UP.**
`start_successor_for_declined_swap` runs beside `queue_startup_swap_intent`, and
the outcome rides in the same `startup_hot_swap_declined_swap_queued` event
(`successor: spawned | already_live | floored | spawn_failed`) so one line tells
the whole story. **The GUI is the right process to do it**: it is the only
component on a stale host guaranteed to be current, it already knows the target
version and the daemon executable, and a daemon it spawns inherits ITS
environment — which is the load-bearing half of the manual recipe above, got for
free instead of reconstructed from `/proc`.
⛔ **Two guards, and they fail differently** (`successor_spawn_verdict`, pure and
unit-tested): a liveness check at-or-above the target, or every decline spawns a
peer beside a successor that already exists; and a five-minute process-local
floor, because the startup reconcile was measured re-running **three times in
ninety seconds** and three daemons is a fork bomb wearing a repair. Neither is a
one-shot — a repair that fired once and failed is gone rather than slow, so the
floor retries.
**Still owed, and an attempt to close it narrowed the scope instead.** The call
is source-pinned and the guards are unit-tested, but **the spawn has not been
watched happening.** A full GUI was run in `scripts/underglass-sandbox.sh`'s
private sway against a private home holding exactly the failing shape — a
3.0.128 daemon owning a live runtime, no newer socket, a 3.0.135 GUI — and the
GUI **did not decline**: it took the ordinary `startup_hot_swap_requested` path
and a 3.0.135 daemon came up that way. ⇒ the decline branch needs an
**unaccounted** runtime key specifically, not merely a version gap, and an
attached session is accounted for. That is worth knowing: the branch this fix
guards is the rarer one, and reproducing it means creating a runtime the stale
daemon has not yet persisted.
**Falsifier, unchanged:** `successor: "spawned"` in
`startup_hot_swap_declined_swap_queued`, followed by a daemon at the GUI's
version appearing in `server daemons`.
⚠ **Do NOT reach for a socket alias instead.** Aliasing an absent version onto a
live daemon is only sound in the older-client → newest-daemon direction; pointing
a current client at an older daemon is the backwards cross-version proxy that has
already returned nothing silently on this project.

⚠ **What that producer exists for: the GUI's startup reconcile drops the intent,
and one unrecognised session key is enough to do it.** `startup_daemon_hot_swap_reason_with_authorized_keys`
answers `None` — silently, with no trace — when the stale daemon owns terminal
runtimes whose keys are not ALL in the authorized set
(`server-state.json` live sessions ∪ `hot-update-terminal-owners.json` entries),
and `reconcile_stale_daemon_on_startup` then returns `false` and forgets.
Measured on the GUI host: of the 3.0.118 daemon's **9 owned keys, 8 were
authorized and 1 was not** — `local://6b91a415-…`, a row created since the last
state persist. `runtime_status_owned_runtime_is_authorized` requires `all()`, so
one such key vetoes the whole host's daemon upgrade for as long as it lives.
⇒ **Do not relax the predicate** — it guards against handing off runtimes nobody
can account for. The intent is queued instead, so the daemon's own retry finds
the moment when that key has been persisted. **Falsifier:** compare a stale daemon's
`owned_terminal_session_keys` against those two files; the swap is declined iff
any key is missing from the union.
⚠ Two known gaps in what landed, both in the safe direction (they answer "not
orchestrating", which is exactly the pre-3.0.101 behaviour, so nothing regressed):
a `remote-cc://` row's transcript lives on the FAR host and reading it is an ssh
hop per session per 20 s poll; and codex is not id-addressable (its rollout is
named by timestamp, not session id) — it has no sub-agent plane, so this costs
nothing today but will the moment a second CLI grows one.
⚠ The positive WORKING signal has a known cost too: `session_transcript_activity`
answers `Unknown` for every agent session on purpose. The booter reads exactly
this signal and was right throughout the
2026-08-09 stranding incident, so the source is proven — the transport is not.

## ⭐ THE `.bak.` RECLAIM IS DONE FLEET-WIDE — WHAT REMAINS IS THE ENGINE

**Status:** OPEN

**Stated "I say the word" 2026-08-08 and it was executed the same session.**
Tool: `~/.local/bin/ygg-bak-sweep` (python3, fleet-deployed; `--dry-run`,
`--quiet`, `--root`). It is **not in this repo** — it follows `ygg-build-sweep`'s
precedent of living in `~/.local/bin` under fleet-binary-sync, and it is
temporary: C0 belongs in the daemon engine, per
[`spec-sweep-policy.md`](spec-sweep-policy.md) §3.

| host role | swept | kept, and why |
|---|---|---|
| GUI | 624 copies / 3.2 GB logical (~2.0 GB disk) | 127: **125 no-base survivors**, 1 base-shorter, 1 diverged |
| workshop | 36 copies / 357 MB | 16: 6 no-base (now trashed as noise), 10 diverged |
| integrator | 145 copies / **51.1 GB** (32 GB of disk) | 6.1 GB: 5 base-shorter, **76 diverged**; 24 no-base repaired (7 restored, 16 trashed) |

**Fleet converged: 0 orphans on all three hosts.** `~/.codex/sessions` went
47G→14G on the integrator, 4.0G→2.0G on the GUI host, 875M→689M on the workshop.
⛔ **DO NOT re-run any host.** What remains open is the ENGINE (next entry) and
the resume-id finding below — the one-shot reclaim is finished.

### ⛔ THE FIRST PROOF WAS WRONG, AND ITS REFUSAL WAS CORRECT

Redundancy was first proved by **byte-prefix** (a rollout is append-only, so a
backup should be a prefix of it). It **refused 624 of 753** copies. The premise,
not the copies, was wrong: **codex re-serialised its entire store on 2026-03-14
with a different JSON key order**, so a `.bak.` is the pre-migration text of the
same conversation. Same records, same count, different bytes — and that same
migration is what flattened every mtime in the store.

⇒ The shipped proof is a **canonical-JSON record-prefix**, streamed in lockstep.
Full statement, and the general law behind it, in `spec-sweep-policy.md` §9.6.

### ⚠ THE MIGRATION LOST 67 SESSIONS — REPAIRED ON THE GUI HOST, NOT YET ELSEWHERE

The 125 no-base copies were **67 distinct sessions the 2026-03-14 codex
migration LOST**: it wrote the backup and never produced the replacement. None
has a row in codex's own `threads` table, so codex does not know them either.

Owner-ruled on being shown them (`settled-calls.md`): lost sessions **show**,
noise sessions are **deleted**. Executed on the GUI host — 5 restored (623 live,
up from 618), 118 noise copies trashed to `~/.yggterm/session-trash`, 0 orphans
left, manifest at `~/.yggterm/bak-restore-manifest.jsonl`.

### ⚠ THE SWEEP SURFACED EVIDENCE OF RESUME-ID COLLISION — NOT CHASED

The proof refuses copies whose records diverge, and the **refusal rate is wildly
asymmetric**: 1 on the GUI host, 10 on the workshop, **75 on the integrator** —
the host that resumes most. Inspecting the workshop's divergent pairs:

- record counts are **identical** (7234/7234, 9751/9751, 8696/8696)
- `base_instructions` are **identical** (12,217 chars both sides)
- the only differing key, in records 0-3, is **`id`**
- the filenames share ONE uuid with timestamps **seconds apart**
  (`…T12-22-56-019c7e88…`, `…T12-22-59-019c7e88…`, `…T12-23-08-019c7e88…`)

⇒ The same conversation exists under **different session ids**, written seconds
apart. `retention.rs`'s own header already names *"agent resume UUID conflicts"*
as an incident class worth correlating, which makes this a second sighting from
a completely independent direction.

⚠ **HYPOTHESIS, NOT PROVEN:** that a resume forks the rollout rather than
appending to it, under some condition the integrator hits most. Not chased — the
sweep's behaviour is already correct (it KEEPS every divergent copy), so nothing
is at risk while this is open. What would settle it: whether the ids collide in
codex's `threads` table, and what the 4th differing record carries.

**Still open:**
⭐ **LIVE-PROVEN on the GUI host:** `local_tree_scan`'s own `codex_sessions`
annotation now reads **623**, up from 618, across five consecutive scans. The
restored sessions are in the tree. This also confirms the source reading that
predicted it: `store_file_name_is_session` rejects `.bak.` (that WAS the
invisibility), a renamed file passes the glob, and `build_local_cwd_tree` groups
by the cwd STRING with no `is_dir()` check — which mattered, because **all three
restored cwds no longer exist on disk**.

⚠ Two instruments that lied on the way there, both now in the field guide:
`server app state` does NOT carry the tree's contents, only its UI state (width,
rename, selection), so grepping it for a session id is a BLIND instrument rather
than a negative result; and the scan annotation is throttled behind
`local_tree_scanned:false` / `superseded:true`, so a stale count reads exactly
like a current one — **check `ts_ms` before believing it.**
- ⚠ `regenerate-copy --budget 0` is not a safe way to force a scan — it has its
  own entry above.

## THERE IS NO SESSION SWEEP, AND THE BUILD SWEEP RUNS TOO RARELY TO KEEP UP

**Status:** OPEN

Owner-requested 2026-08-08: *"we need a systematic session sweeping system …
intelligently dropping most unimportant and least touched sessions"*, and
*"the stale builds of yggterm everywhere where we are building should be sweeped
up intelligently too to save GBs of space."* The design is settled and lives in
[`spec-sweep-policy.md`](spec-sweep-policy.md); the owner's own calls are in
[`settled-calls.md`](settled-calls.md). What follows is only what is UNBUILT.

**Nothing exists for sessions.** `clipboard_sweep.rs` and `socket_sweep.rs` are
the two working precedents and the shape to copy (per-host, own `$YGGTERM_HOME`
only, positive liveness proof, fail-safe on any read that cannot complete), but
no code reclaims an agent transcript.

**The build sweep is not broken, it is under-scheduled.** `ygg-build-sweep` ran
on the integrator on 2026-08-02 and reclaimed 28.7 GB with `0 skipped as
active`. It is on a **weekly** timer against a host that regenerates ~13 GB/day
of incremental cache, so `target/debug/incremental` was back to **80 GB** six
days later. ⛔ Do not go looking for a quiet-gate bug here; the 2 h grace window
was checked and the directory's own mtime was ~7 h old at measurement time. The
fix is cadence (daily) plus the `target/debug` budget, both settled.

Suggested order, each independently shippable:

1. **C0 `.bak.` copies** — the entry above. 61 GB, zero risk, no new engine.
2. **C0/C1 build classes** — daily `incremental/`, 40 GB `target/debug` budget,
   plus the ad-hoc binary copies in `$YGGTERM_HOME/{bin,binbak,deploy-backup,
   versions}` (~260 MB of `.rollback-3051` / `.pre-inputdead-<epoch>` /
   `.pre-phaseE` names with no convention and no retention rule). ⛔ §9.3: the
   retention set is load-bearing for the CONSTITUTION's version-coexisting
   daemons — deleting the binary an older live daemon would restart from turns
   housekeeping into another agent's outage.
3. **The session index and score** (§4) — and this is where the real work is,
   because `mtime` cannot be used (see the field guide) and both substance and
   touch-frequency have to be read out of the transcripts themselves.
4. **Compaction with verified rehydration** (§5) — last, because its failure
   mode lands on the resume path, which is the product.

⚠ **Unproven and required before (4) ships:** that codex accepts a rehydrated
rollout at all, and that rehydration fits inside a click. Neither has been
tested.

## 3.0.63's APP-ROW PERSISTENCE IS UNPROVABLE WHILE THE QUIET GATE HOLDS THE DAEMON

**Status:** FIXED IN CODE — LIVE PROOF OWED

3.0.63 is built and deployed to both `~/.local/bin` and `~/.yggterm/bin`, and the
GUI is running it — the metadata rail reports it of itself:
`Client Version 3.0.63 · daemon is on 3.0.62`.

**The proof cannot be taken, and the reason is structural, not effort.** The fix
is DAEMON-side (`PersistedLiveSession` now carries the `app:<name>:<verb>` token
and `restore_live_session` re-derives against the current registry), so it is
inert until a daemon owns it — the "a daemon-side fix is inert under proxy" rule.
The live daemon is still 3.0.62, reports `Restart deferred`, and owns 10 live
sessions. Confirmed inert by measurement: **zero records in
`~/.yggterm/server-state.json` carry an `app:` token** after creating an app row
against the 3.0.62 daemon.

⇒ Forcing the swap is the only way to run the falsifier, and forcing it now would
put ~10 live agent sessions through the very mechanism the constitution records
as still destructive (`kill -TERM` cost ~7 agent PTYs because the idle gate never
converges under load). **This falsifier is therefore BLOCKED BY the quiet-gate
bug** — which is worth stating plainly, because it is the first case where the
gate is not merely slowing a deploy but preventing a fix from being verified at
all.

⚠ Falsifier, once a 3.0.63 daemon owns the row: create an app row
(`server app launch-app ychrome --cwd <dir>`), force a daemon swap, then read the
row — **a bash prompt with no app means it did not hold**. Check
`server-state.json` for the `app:` token first; absent, the swap proves nothing.

## `server app launch-app --cwd` IS IGNORED — THE ROW INHERITS THE ACTIVE SESSION'S CWD

**Status:** OPEN

`launch-app ychrome --cwd /home/user/gh/yggterm` answered `accepted:true` with
`launch_command: /home/user/.local/bin/ychrome` and created a row whose
`session_cwd` was **`/home/user/data/otherlane`** — the ACTIVE session's cwd, not the
one passed (measured 2026-08-08 on 3.0.63). The reply's `shell.launch_app` block
does not carry the cwd at all, so it reports a good launch either way.

This sits directly beside the already-known "with no `--cwd` it answers
`accepted:true` and creates nothing". Together they say the cwd argument reaches
the accept path but never reaches the row: ⇒ **the row is placed by inheritance,
and `--cwd` decides only whether the create is attempted.** An agent using this
door to exercise the app-launch path therefore cannot put an app row anywhere in
particular, which is most of what the door is for.

⚠ Falsifier for a fix: create with `--cwd <dir>` while a row with a DIFFERENT cwd
is active, then read `session_cwd` back from `server app rows` — echoing the
request is not proof, the row must be re-read.

## ⭐ OWNER-REQUESTED: SELECTABLE TEXT — THE RAIL HEADER WAS THE LAST REAL GAP

**Status:** FIXED IN CODE — LIVE PROOF OWED

⇒ Rails shipped 3.0.64. The rail HEADER shipped 3.0.67. What is below is the
measurement that closed the rest of the list, and it is mostly a correction.

The requirement: *"The metadata sidebar entries or text in general (mostly
anywhere) should be selectable"*, and again the same day: *"I still cannot select
any text on session metadata other than the connect code row."*

### ⭐⭐ THE PREVIOUS VERSION OF THIS ENTRY WAS WRONG, AND IT WAS WRONG BY ASSUMING

It said the remainder was *"the start page, dialogs, the titlebar's own text,
notification cards"*. **Nobody had measured that.** A whole-document sweep on the
live host (3.0.65, before the fix) walked all 122 text-bearing elements and
bucketed them by computed `-webkit-user-select`:

| surface | text elements | unselectable | verdict |
|---|---|---|---|
| start page | 5571 | **0** | already fine — never broken |
| toast / notification cards | 2 | **0** | already fine — never broken |
| rail scroll body | — | 0 | 3.0.64, working |
| **rail HEADER** | 1 | **1** | ⭐ the one real gap |
| sidebar / cwd tree | 62 | 62 | ⛔ MUST stay — drag-to-reorder |
| titlebar | 8 | 8 | ⛔ cannot change — see below |

⛔ **There is NO global `user-select:none`, and the old entry's premise that
"the shell root sets it and every surface inherits" is false.** Measured: `body`
and `[data-yggterm-shell]` both compute `text`. The `none` is set INLINE on
individual chrome containers — `[data-yggui-side-rail]` for the rail, and the
libyggterm titlebar's own root. That is why the start page, which is main
content and has no such ancestor, was selectable all along.

### ⭐ THE RAIL HEADER — what 3.0.67 fixes

`.yggui-rail-scroll` is the rail's SCROLL BODY. The heading is a **sibling** of
it under `[data-yggui-side-rail]`, which is where the `none` is set. So opting
the body in left the rail's own title unselectable while every value under it
selected. The rail container's four children measured:

| child | `-webkit-user-select` |
|---|---|
| `[data-yggui-rail-header]` | **none** ← the gap |
| the scrollbar `<style>` node | none (no visible text) |
| `[data-yggui-rail-scroll]` | text (3.0.64, working) |
| `[data-rail-resize-handle]` | none (a DRAG handle — must stay) |

**Live-proven on guihost, GUI 3.0.67:** the header span computes
`-webkit-user-select: text` and a Range over it returns `"Yedit"`. Before, the
same probe on the same element returned `none` and an empty string.

### ⛔⛔ THE TITLEBAR CANNOT HAVE THIS, AND THE REASON IS MECHANICAL

Do not re-attempt it. The titlebar's root div IS the window drag handle
(libyggterm `crates/yggui/src/chrome.rs:141`):

    onmousedown  -> evt.prevent_default()               /* cancels the selection gesture */
    onmousemove  -> past TITLEBAR_DRAG_THRESHOLD_PX -> window().drag()

**Each half defeats selection independently**: `prevent_default()` on mousedown
cancels the browser's selection gesture before it starts, and the native drag
seizes the pointer mid-gesture. Selectable title text therefore costs dragging
the window by its title. The same strings are selectable in the metadata rail,
so no text is actually out of reach. A test asserts the rule never names the
titlebar, and states this reason.

⚠ **The trap that still holds for any future opt-out:** an inline
`user-select:none` with no `-webkit-` twin is INERT on WebKitGTK — 15 of the
shell's 41 sites write only the unprefixed property. Independently corroborated
by the sweep: `getComputedStyle` returns `""` for the unprefixed property on
every element and a real value only for the prefixed one.

⛔ **Do not "simplify" this by flipping a root to `user-select:text`.** It would
break drag-to-reorder on the 62 sidebar elements, and it would un-protect those
15 inert opt-outs, which work only by inheriting a container's `none`.

**Live proof still owed:** the header was proven on the metadata/Yedit rail. The
other rails (settings, connect, notifications, tab rail, contributed app panes)
share the same `RailHeader` component and so are covered by construction, but
none has been observed directly.

## ⛔ `server app clients` ANSWERS IN A DIFFERENT ENVELOPE FROM EVERY OTHER APP VERB

**Status:** OPEN

Measured by another campaign row, 2026-08-08, which wrote the bug and only caught it
because it tested against a host it KNEW had a live GUI. `server app clients`
answers with a top-level `{clients, count}`; every other app verb wraps its
payload in `{data: {...}}`. So the obvious parser reads `data.clients`, finds
nothing, and concludes there is no GUI on any host — **indistinguishable from a
real "GUI not running"**. Either one envelope for every app verb, or `clients`
says plainly in its own `--help` that it is special.

✅ **STILL TRUE AT 3.0.139**, measured on the GUI host in one call each:

```
clients → ['clients', 'count']
state   → ['completed_at_ms', 'data', 'error', 'handled_by_pid', 'output_path', 'request_id']
rows    → ['completed_at_ms', 'data', 'error', 'handled_by_pid', 'output_path', 'request_id']
```

**Falsifier:** every app verb answers in one envelope, or `clients --help` says
it is the exception.

## ⛔ `session remove` ORPHANS THE FAR-SIDE RUNTIME AND REPORTS IT IN A FIELD NOBODY READS

**Status:** OPEN

5th sighting in the fleet record; measured again 2026-08-08. Removing a
`remote-cc` row answered `row_still_listed:false` AND `verified:false` with
`verified_refusal:"remote_runtime_survived"`, `reaped_processes` naming only the
ssh transport. Both fields are true and they answer different questions: the ROW
left the sidebar, the AGENT kept running on `dev` holding its context, and the
caller reaped it by `/proc` by hand. Either `remove` reaps the far-side runtime,
or it says plainly *"row removed; runtime orphaned at pid N on host H"* so the
caller knows there is a second step. Today the caller has to already know to look.

## ⭐ A ROW'S SEAT SHOULD BE A VERB — `session claim`

**Status:** OPEN

Every campaign session hand-assembles the same chore from primitives: find my row
by uuid, derive a seat number that does not collide, rename, read the title back
because the verb reports the request rather than the effect, re-assert against the
CLI's self-title, and when superseding an older row remove it and reap what
`remove` leaves behind. It has been shipped as a script
(`.agents/skills/yggterm-agent-fleet/ygg-claim.sh`) — which is the wrong layer,
because it re-derives from primitives what the app already knows.

⚖ This is the owner's standing test applied exactly: *did an agent hand-assemble
this chore from primitives and get it wrong?* It did, repeatedly. ⇒ Make it a
verb: `session claim --title T [--seat N] [--replace UUID]`. His reason for
preferring a verb to a documented recipe: **an agent's discipline resets every
session; a verb's does not.**

⚠ A related trap the same investigation surfaced, and it is why the seat must be
first-class: an ssh helper written as `ssh host "yggterm $*"` lets the REMOTE
shell re-split the title on whitespace, so `rename` keeps only the first word —
and a truncation at a turn boundary is **indistinguishable from the app
re-titling itself**. Anyone driving app control from another host will
misdiagnose it the same way.

## ⛔ `outline_prefix` DOES NOT SURVIVE, AND `rows.label` MAKES IT LOOK LIKE IT DID

**Status:** OPEN

Measured 2026-08-08: a seat set via `session outline` and read back correctly at
19:18 was gone by 20:02 with the row untouched. `PersistedLiveSession.outline_prefix`
exists and is serialized, so this is not a missing field — something clears it, or
it is lost on a path that rebuilds the row.

⚠ **The report that came with it blamed the wrong thing, and the correction
matters more than the bug.** It was filed as *"`label` composes a prefix the
sidebar never renders"*. It does render: `build_sidebar_rows` re-composes the
outline onto `row.label` as its LAST act (`shell.rs` ~49500), specifically so a
CLI re-titling itself cannot drop the number, and the sidebar draws `row.label`.
⇒ **Do not "fix" this by dropping the composed label or by adding a second
renderer.** The API and the screen agreed; the PREFIX had vanished between the
two readings. One bug, in durability.

⚖ The generalisable lesson is still the reporter's, and it is worth keeping: an
API read taken at a different moment from the screenshot is not a verification of
the screen.

## ⭐ A STALL IS A RECOVERABLE STATE AND NOTHING RECOVERS IT

**Status:** OPEN

recorded 2026-08-08, with a live instance he noticed himself: *"the yggterm
session stalled suddenly. Our relay system or monitor system should yank a
continue intelligently for such edge cases (cli bug, API error, DEMOTED etc.)."*

A row whose turn ENDED with the work unfinished and no error in its transcript is
one `continue` from resuming. Today the monitor DETECTS that and tells a human.
The discriminators already exist in the transcript: turn ended + no activity past
a threshold, api errors, `model_refusal_fallback`.

⛔ Three guards, and each has already been named as the way this goes wrong: an
ASSIGNED row that stalls gets the nudge and a PARKED row idling by design must
never get one; the nudge fires ONCE per stall, not per poll (a watcher that
re-nudges every tick is worse than one that never nudges); and it escalates if the
row does not wake.

## ⛔⛔ REPORTED: THE RIGHT PANEL IS A GLOBAL SLOT — one app's rail renders over another app's row, and "I cannot see any files" is that same bug

**Status:** OPEN

⭐ **The tenancy half is FIXED IN CODE (3.0.60) and LIVE PROOF IS OWED.**
`RightPanelMode::AppPane` now carries `AppPaneRef { session, pane }`, stamped at
open; every tenancy test, the reveal resolution, the vanished-pane release, the
rail component's own check and `app_pane_fetch_schema` (which now takes the
OWNING session, with its page context) ask whose pane it is. Lock:
`a_pane_is_a_tenant_of_the_session_that_declared_it_never_of_a_namesake`,
mutation-proven red. ⛔ **Not yet deployed** — see the app-identity entry below:
a GUI restart currently resurrects the owner's live app rows as bare bash, so
the two must land together and deploy once. Falsifier for the live proof: open
the `New Yedit` row and read the rail; ychrome's omnibox/tabs/`tables` folder
appearing there again means this did not close it.

⚠ **STILL OPEN underneath it: the second half is now MEASURABLE but not yet
measured.** The photographed rail also painted ychrome's chrome while
`right_panel_mode` reported `hidden`. **3.0.61 built the instrument that
separates the two candidates** — `server app state` grew a `shell.right_panel`
block reporting `requested_mode` / `rendered_mode` / `docked`, the rendered pane
with the session that DECLARED it, and `web_tabs_overlay_session` read off the
overlay's own stamp. It also settled the thing that made the old readings look
contradictory: **a rail that is not docked STILL RENDERS A BODY** (the reveal
card draws `reveal_mode`), so `hidden` beside a painted rail was never a
contradiction, and the probe simply could not see which body was up.
⛔ **What remains is the measurement**, on a deployed 3.0.61: reproduce, read
`shell.right_panel`, and name the carrier. If it names the overlay, session-key
`RightPanelMode::WebTabs` exactly as `AppPane` now is — **do not widen to it
before that reading**.

reported 2026-08-08, with a screenshot: *"Right click context menu ychrome
launch launches plain terminal. yedit launch opens blank viewport on libyggterm
surface; I cannot see any files at all. Weird bugs: I see ychrome tabs sidebar in
terminal (which is supposed to be yedit). I think the context menu wiring of the
clis and the apps and app renderings are fucked up."*

The screenshot: the `New Yedit` row is SELECTED, the viewport is a bash prompt
showing `yedit: document surface opened`, and the right panel is **ychrome's**
chrome — a Tabs rail listing Khan Academy tabs, a `tables` folder, and a URL bar
on `khanacademy.org`.

### ⭐ THE UNIFICATION: the file list IS the rail, so two of his symptoms are one bug

yedit's FILES list is not a separate widget — it is the `notes` **rail pane** the
app contributes. The rail is a **global slot keyed by pane id with no session**,
so when its tenancy check fails it falls back to whatever else holds the slot,
which on his machine was ychrome's tab rail. ⇒ *"I cannot see any files at all"*
and *"I see ychrome tabs sidebar where yedit should be"* are the **same defect**,
not two. A live trace caught the mirror image as well: yedit's `AppPane("notes")`
rail displayed while the ACTIVE session was the **ychrome** row.

`right_panel_mode` is one window-global field. The rail's schema fetch resolves
its control endpoint from the **active** session while the document's fetch
resolves from the **owning** session — two identity models for the two panes of
one declare.

**Fix shape (do not widen it):** an app-contributed pane is addressed by the
session that DECLARED it. `RightPanelMode::AppPane { session_path, pane_id }`,
the app-pane schema slot keyed by `(session_path, pane_id)`, and
`app_pane_fetch_schema` taking the owning `session_path` like its document twin
already does. ⛔ Do not fix this by masking the panel after the fact — the panel
mode must be a function of the active session, not global state with a filter.

### The blank viewport is a SECOND, smaller defect underneath it

`DocumentSurfaceBody` (`shell.rs` ~123815) renders an **opaque empty layer** when
it has no schema: there is no loading, empty, or error state. So a fetch that
never landed and a document that is genuinely empty are pixel-identical, and both
read as "blank viewport". It needs an explicit tri-state body — fetching / error /
no-content — driven off `DocumentSurfaceSnapshot { schema, error }`.

⚠ And a `New Yedit` with no argument has nothing to show anyway (manifest args
are `[]`, no active note, no recents), so an actionable empty state is the honest
rendering, not a blank rectangle.

### ✅ WHAT THE LIVE PROBE SETTLED, 2026-08-08 (do not re-derive)

- **yedit is HEALTHY and serving.** Its control endpoint answers `/ping` 200,
  `/pane/doc` with the owner's real note text, and `/pane/notes` with the full
  files rail (toolbar, search box, footer word count). The app is not the bug;
  the GUI is not showing what the app is offering.
- **The declare arrives and is consumed.** `server terminal app-declares` holds
  yedit's `sidebar;declare` with both panes (`doc` @ viewport, `notes` @ rail),
  and the GUI logged exactly ONE `sidebar_contribution/declare`
  (`source: terminal_stream`) for it. `seq: 1`, and nothing after — yedit
  declares once and exits, so `last_seen_ms` is frozen while the app lives, and
  only the `/ping` heartbeat refreshes it.
- **`server app panel pane:notes` DOES open the pane** on that row (mode goes
  `hidden` → `app_pane`), so the contribution is present and offers `notes`.
  The rail nonetheless painted ychrome. ⇒ the failure is on the RENDER side of
  the mode, not in whether the pane exists.
- **The rail's reported mode and its pixels disagree** — `hidden` while
  ychrome's tab rail and omnibox are drawn. That disagreement is the fault to
  chase, and it needs the instrument named above.

### ⛔ FIVE HYPOTHESES ALREADY FALSIFIED — do not re-derive them
- **"one OSC declare, TWO parsers"** — the JS forwarder and the Rust wire parser
  agree field-for-field on both `sidebar` and `web-surface`. The live trace shows
  yedit's declare arriving and producing a contribution. NOT a parser skew.
- **The 13-day-old `~/.local/bin/yedit` binary** — does not predate any wire or
  contract change; refuted on every axis. Do not spend a rebuild on it. (Residual
  and untested: the six-day-old yedit *daemon process*.)
- **Commit `ac624b85` (3.0.59)** — does not touch the app launch path at all.
  Plainly ruled out as the regression.
- **A key-spelling mismatch between the snapshot map and the mount gate** —
  `snapshot.active_session` is *filtered* to equal `active_session_path`
  (`shell.rs` ~18286), so those two cannot disagree.
- **`icon_kind: "terminal"` / `document_kind: null` on an app row** — these are
  CORRECT. See the next entry: an app row is a shell by design.

### The instrument that was missing is now shipped (3.0.60)
`server app state` reports `document_surfaces` and `sidebar_contributions`, each
contribution carrying `document_surface_visible_live` beside `in_snapshot_map`.
`true` with `false` is the bug shape, named. ⚠ Still missing and worth adding:
the right-panel mode does not report its **pane id**, and the document channel and
the contribution sweep emit **no trace events at all**.

### ⛔ THE INSTRUMENT ABOVE IS ONLY READABLE ON THE **ACTIVE** ROW — corrected 2026-08-09

Re-reported by the owner the same day (*"yedit viewport is still not working. It
either shows the terminal or empty viewport. The editing surface is gone."*), and
the first reading of the new instrument was **misread by the session doing the
re-measuring**, so the correction is recorded here rather than learned twice.

`server app state` reported three Yedit contributions, every one
`document_surface_visible_live: true` with `in_snapshot_map: false` — the exact
pair named as the bug shape — and `document_surfaces: []`. That reading proves
**nothing**: `snapshot.document_surfaces` is built ONLY for the CO-VISIBLE set
(the active session plus the active split group's members, `shell.rs` ~18859,
and the debug block says so at ~54004). Every one of those yedit rows was in the
background, so `in_snapshot_map: false` was the CORRECT answer for all three.

⇒ **`document_surface_visible_live: true` + `in_snapshot_map: false` is the bug
shape ONLY when the row is the ACTIVE one.** On a background row it is the
contract. Any future reading must first make the yedit row active
(`server app open <path>`) and then re-read; a sweep across all rows will report
the bug shape for every background app row and mean nothing by it.

⚠ The rail half of this entry has since been implemented — `RightPanelMode::
AppPane(AppPaneRef)` carries the declaring `session_path` (locked by
`a_rail_that_is_not_docked_still_renders_a_body_and_the_overlay_names_its_owner`).
What the owner is still reporting is the **viewport**, i.e. the second, smaller
defect above: `DocumentSurfaceBody`'s opaque empty layer. Start there.

## ⛔⛔ AN APP ROW IS A SHELL WITH NO APP IDENTITY — so a restart resurrects bare bash, and a missing app binary cannot be refused

**Status:** OPEN

Split out of the owner report above. `spawn_launch_app_verb` (`shell.rs` ~37495)
asks the daemon for `SessionKind::Shell` (~37521) and then types the app's command
into the new PTY out-of-band via `write_app_verb_command` (~37556). This is
deliberate and documented — *"the same thing the user would do by hand"* — and the
row being a shell is NOT itself the bug. **The bug is that nothing on the wire
says "this row is ychrome".** Three consequences, each live-proven:

1. ⭐ **FIXED IN CODE (3.0.61) — LIVE PROOF OWED.** A local app launch now goes
   through `start_command_session_placed`, so the row is BORN holding
   `local_app_verb_launch_command(cmd)` as its own launch command with
   `Source: app:<name>` stamped on it. The command keeps an interactive shell
   after the verb — apps declare and EXIT, so a bare verb would end the PTY on
   arrival. Lock: `an_app_row_holds_its_app_command_so_a_restart_brings_the_app_back`,
   mutation-proven red three ways. **Falsifier:** launch an app row, restart the
   GUI, and read the row — a bash prompt with no app means this did not close it.
   ⚠ **NOT retroactive:** a row created before 3.0.61 still stores
   `exec '/bin/bash' -i` and still comes back as bare bash.
   ⚠ The REMOTE (ssh) arm still types the command out-of-band, deliberately —
   untouched and unproven.
2. **The missing-binary refusal shipped in 3.0.59 cannot speak for an app.**
   `local_managed_cli_tool_for` (`lib.rs` ~15555) returns `None` for a Shell, so
   the exact failure `ac624b85` fixed for CLIs — a binary that prints, exits, and
   leaves bash alive looking healthy — is still wide open for apps.
3. **The sidebar icon is derived from the kind**, so an app row is visually a
   shell (`tree_icon_kind`, `shell.rs` ~86250).

**Fix shape:** route the launch through `start_command_session` (`lib.rs` ~7521),
which already HOLDS the command as `session.launch_command` and stamps a `Source`
label — it is used today only by the terminal-recipe door. ⚠ Cross-layer: the app
path currently goes through the `start_local_session_placed` endpoint RPC while
`start_command_session` is a direct state mutation, and the app path needs the
`insert_after` placement the RPC provides. `SessionKind::Shell` staying the kind
is fine; the missing piece is row-level app IDENTITY, not a new enum variant.

### Two more launch defects found in the same sweep
- **`write_app_verb_command` types into whatever the daemon calls the ACTIVE
  session, not into the session it just created**, and returns silently when it
  can resolve neither. A row that never received its command line is a bare prompt
  forever. ⚠ NOT confirmed to have fired in the live incident.
  ⭐ **Retired for the LOCAL arm in 3.0.61** — a create that carries its own
  command has nothing left to infer. **Still live on the REMOTE (ssh) arm**,
  which is the open half of this bullet.
- **A bare `ychrome` takes the PROFILE-PICKER path, and the daemon's retention
  allow-list DISCARDS that declare** (`app_declare.rs` ~87), so a bare-ychrome row
  never gets a retained declare at all. Either the `new` verb stops being
  argument-less, or `retention_for` gains the picker verb with the pick made
  idempotent. ⚠ The declare-wire parity lock (`shell.rs` ~149698) covers the two
  parsers but NOT this allow-list, which is the third reader of the same wire and
  the only one that disagrees.

### ⚠ Structural, and the reason this class keeps recurring
**yedit has NO libyggterm dependency** — the app tier is a hand-copied protocol,
not a shared crate, so every wire contract exists twice with nothing keeping the
two in step. A wire change should be a compile error in the apps; today it is a
silent null. libyggterm needs an app-side crate that yedit, ychrome and yrdp all
depend on.


## ★★ REPORTED: THE FIRST TAB REFUSES CLOSE, DUPLICATE AND DRAG — the refusals are right, the AFFORDANCES are the bug

**Status:** OPEN

⭐ **Reported 2026-08-08:** *"the first tab of ychrome session is a diva. It
cannot be closed, duplicated, dragged around, etc."*

Every one of those refusals is deliberate and has its own reason written at the
callsite: `web_surface_close_tab` returns early on `index == 0` (closing the app
is the overlay ✕, which sends a real Ctrl+C); `web_surface_duplicate_tab` refuses
`WEB_TAB_APP_TAB_ID` because `persist_web_tabs` saves tabs[0] only as the MARKED
app-tab row, so a copy would persist as a user tab and resurrect a stale start
page on the next visit; the drag lands `DragDropPlacement::After` on it for the
same reason. The ✕ on that row is a **go home**, not a close, and it lost and
regained that meaning once already.

**So the defect is not the policy — it is that the row LOOKS like every other
tab and answers differently.** A user cannot see an invariant. Two candidate
shapes, and the choice is a spec call rather than a patch:

1. **Mark it.** Render the app tab as what it is — pinned, no drag handle, its ✕
   labelled and shaped as *go home* — so nothing is offered that will be refused.
2. **Free it, per app kind.** For a BROWSER the app tab is a start page and
   nothing depends on it surviving; for yedit it is the editor and everything
   does. `AgentCliDescriptor`-style per-app declaration could say which, and
   ychrome would declare its first tab ordinary.

⚠ Shape 2 is the one the report actually asks for, and it is the one that moves
`persist_web_tabs`' marked-row invariant — do not start it without deciding what
a ychrome session with NO app tab is.

## ⛔ A REFUSED LAUNCH IS RECORDED IN THE DAEMON AND STILL RENDERS AS `running` IN THE GUI

**Status:** OPEN

Found 2026-08-08 while shipping the missing-binary launch gate (3.0.59), by
looking at the screen instead of at the reply.

**What DOES work, live-proven on guihost** — do not re-do it: `terminal new --kind
muse` is refused by name with Muse's install method, `--kind kimi` is refused
with kimi's *different* (uv) one, and `pgrep` finds **no process at all** for the
refused row. The `/bin/bash` that used to print `command not found` and then
outlive the CLI is gone. That was the owner's report and it is closed.

**What does NOT work:** a refused row LEFT IN THE SIDEBAR renders with
`Status: running · —` in the session inspector and a blank viewport.
`record_launch_refusal_for_path` sets `launch_phase = Failed`,
`last_launch_error`, the status line and viewport lines — **in the DAEMON's copy
of the session.** The GUI renders from its OWN in-process `ManagedSessionServer`,
and the create that would have carried a fresh snapshot back returned an ERROR
instead, so the GUI keeps its pre-refusal state. Same shape as
`[[finding-daemon-side-fix-inert-under-proxy]]`: the fix is real and lands on the
wrong side of a seam.

⚠ **`friendly_launch_phase` is what the inspector's "Status" is derived from**,
so this is the field that decides whether a user believes their CLI started. The
`Failed` variant now exists (3.0.59) and is correct — it just never reaches the
renderer on this path.

**Fix shape:** a refused ensure must return a snapshot, not only an error, so the
GUI applies the row's Failed state. ⛔ Do not "fix" it by having the GUI re-derive
the refusal locally — that mints a second owner for "is this CLI installed", on a
machine that may not be the one the CLI runs on.

**Falsifier:** `terminal new --kind muse`, then read the row's `launch_phase` out
of `server app state`. `Failed` means fixed; `Running`/`RemoteBootstrap` means
this entry stands.

⚠ **And the daemon's own trace file is blind on this path** — the live daemon
(pid held on `event-trace.g<ts>.jsonl`) had **2 startup lines and nothing else**
while 18 `before_request_terminal_launch` events went to an OLDER file. Any agent
using the trace to decide whether a daemon code path fired will read a zero and
believe it. That is worth its own look; it made this fix briefly appear not to
run at all.

## THE INTERFACE LLM IS HARD-WIRED TO ONE HTTP PROVIDER — it needs a provider dropdown

**Status:** OPEN

⭐ **Recorded 2026-08-08.**

The interface LLM — the model yggterm itself calls for titles, summaries and the
working indicator — is fixed to an endpoint + API key + model (LiteLLM). His
ask: *"the interface settings system (currently we use litellm) should be
preceded by a dropdown of litellm (selected default) or any cli sdk (those have
available, like claude code, codex, etc.) and the model to be used."*

**Why it is more than a convenience:** a CLI SDK provider needs **no key and no
endpoint** — the CLI is already logged in on that host — so the interface stops
depending on one HTTP service that has already rate-limited this fleet with 429s.

Design + the measured eligibility list (a CLI qualifies iff it has a
non-interactive mode AND a model selector — claude `-p`, `codex exec`, pi
`--print`, qwen `-p`, opencode `run`/`serve` all do; kimi is documented but not
installed; antigravity and muse do not):
[`spec-settings-model-providers.md`](spec-settings-model-providers.md) §2.

⛔ Three traps written into that spec because each has already bitten something:
a process spawn per title is not an HTTP call (keep the chore caps), a failed
generation never persists a heuristic, and a CLI provider only works on hosts
where that CLI is installed — the setting must say which.

## THE CODEX BACKEND NEEDS A `codex ↔ Anything` SLIDER

**Status:** OPEN

⭐ **Recorded 2026-08-08:** *"codex sessions should have an extra slider in
settings codex ↔ Anything."* This is the other half of removing `codex-anything`
from the kind list: the capability needs a home before its CLI-hood is deleted.

⚠ The same choice is currently encoded **three times** — a `--kind` value, a
separate binary (`~/.yggterm/npm/bin/codex-litellm → @avikalpa/codex`, a private
fork), and a provider key in `~/.codex/config.toml` (`[model_providers.litellm]`).
Part of this work is deciding which one is the mechanism and retiring the others;
the spec refuses to pick for the implementer but requires the reason be written
down. → [`spec-settings-model-providers.md`](spec-settings-model-providers.md) §1.

## `codex-anything` IS A CLI KIND AND THE OWNER SAYS IT IS NOT A CLI

**Status:** OPEN

Settled 2026-08-08 (`settled-calls.md`): *"we should not have codex-litellm as
another CLI. It is a special codex session flip switch … a codex ONLY sessions
superpower."* The name is **locked as `codex-anything`** for every human-facing
surface; `codex-litellm` stays only as repo/binary/provider identifiers.

Today it is a first-class `--kind` value — `terminal new` names it in its own
refusal message (`expected shell or one of: codex, codex-litellm, claude-code,
pi, opencode, qwen-code, kimi, muse, antigravity`) — and it has its own
provisioned binary at `~/.yggterm/npm/bin/codex-litellm` on all three hosts.

**The work:** remove it from the agent-CLI kind list, the session submenu and the
extra-args modal, and re-express it as a **flip on a codex session** — the
capability stays, its CLI-hood goes. The LiteLLM endpoint/API-key/interface-model
settings that feed it stay where they are; they are codex configuration.

**The flip's home is a `codex ↔ Anything` slider** in settings —
[`spec-settings-model-providers.md`](spec-settings-model-providers.md) — not a
row in the session menu.

⚠ Rename the LABEL, never the identifier: the binary stays
`~/.yggterm/npm/bin/codex-litellm` and the codex provider key stays
`[model_providers.litellm]`. A label that leaks into an identifier is how one
thing becomes two.

## NOTHING KEEPS yggterm's OWN BINARY AT PARITY ACROSS THE FLEET — measured by md5, not by `--version`

**Status:** OPEN

Owner, 2026-08-08: *"yggterm needs to keep its binary copies regularly updated
across all the connected fleet … See how we handle codex or CC's binary copy."*

**The asymmetry he is pointing at is real: yggterm keeps OTHER people's binaries
current better than its own.** The CLI provisioner fetches and updates
`claude`, `codex`, `codex-litellm` (and now `pi`, `opencode`, `qwen`) into
`~/.yggterm/npm/bin` on demand, per host. Nothing does that for `yggterm` itself.

Measured 2026-08-08 12:45 IST by **md5 + mtime** — deliberately not `--version`,
which this repo has already filed as a blind instrument (it is a pure builtin and
reports the binary you typed):

| host | `~/.local/bin/yggterm` | `~/.local/bin/yggterm-headless` | `~/.yggterm/bin/*` |
|---|---|---|---|
| guihost | `a2c67e3c` 08-08 03:31 | `a262a396` 08-08 03:31 | current |
| dev | `c0ffc9ea` **08-06 22:16** | `fddd8fd0` **08-07 05:11** | `a262a396` 08-08 03:32 |
| oc | `c0ffc9ea` **08-06 22:17** | `ef4c1803` **08-06 22:17** | `a262a396` 08-08 03:32 |

⇒ **three different headless binaries are live on the fleet under one name**, and
on oc the PATH copy is the stale one. The `~/.yggterm/bin` copies are current
everywhere — so the deploy is not broken, it is *partial*, and nothing is
watching the difference.

**This is not the same entry as the deploy-writes-three-copies bug below.** That
one is about a single deploy missing a path. This one is about there being **no
recurring parity sweep at all**: a fleet-wide property (binary, config, cert)
needs an AUDIT that runs on its own schedule, because the deploy that would fix
it can be structurally blocked on the busiest host while nothing says so.

**Fix shape:** a scheduled parity check over every host × every install path,
comparing **hashes**, that either self-heals or reports which host is behind —
and it must survive the case it exists for, a host where the hot-restart gate
never opens.

## THE `--kind` REPLY OVER-REPORTS AND DOES NOT ROUND-TRIP

**Status:** OPEN

Found 2026-08-08; the `--help` half closed 2026-08-13.

`--kind opencode` answers `session_kind:"open_code"`; `--kind qwen-code` answers
`"qwen_code"`; `--kind grok-build` answers `"grok_build"`. Feeding any of them
back is refused: `unsupported app-control terminal kind "open_code"`. **A caller
that reads a row's kind and launches another like it fails on exactly the kinds
whose slug carries a hyphen** — three of ten now, and every future hyphenated
slug joins them.

The kind vocabulary has three encodings — flag token, enum debug name, help text
— and no owner. Collapse them onto the descriptor's `slug`, which is already the
SSOT the intake built.

✅ The help half is fixed: both usage blocks said `--kind <shell|codex|claude-code>`
while the parser accepted nine, and they now say `<shell|<agent-cli>>` and point
at the refusal string, which is generated from the registry and has always been
correct.

## TESTS ARE FLAKY UNDER PARALLEL EXECUTION — they pass alone and fail in the suite

**Status:** OPEN

⚠ **It is not one test, and that is the finding.** `yggterm-server`'s suite joins
it: on 2026-08-13, three separate full runs of an unchanged tree failed on a
DIFFERENT test each time —
`tests::refresh_terminal_identity_updates_restored_remote_launch_commands` (twice,
`refresh_terminal_identity_launch_commands()` returned 0 where 1 was expected) and
`tests::local_cc_relaunch_rebuild_collapses_poisoned_identity_to_row_id` — while
five other runs of the same tree passed clean. Both touch the process-global
terminal identity env that `codex_cli::env_test_guard` exists to serialize, so
either a writer is not taking the guard or the guard is not the only door. The
failures cluster when the host is under build load, which changes interleaving —
so the suite is quietest exactly when nobody is deploying and loudest when
somebody is.

⇒ **A different test failing each run is the signature to look for**: it rules out
a regression in any one of them and points at the shared state they have in
common.

`shell::tests::a_pane_is_a_tenant_of_the_session_that_declared_it_never_of_a_namesake`
(`crates/yggterm-shell/src/shell.rs`) is the original instance.

Measured 2026-08-08 across three runs with **`shell.rs` unchanged between
them** (`git log` on the file shows nothing since `b3f96ec4`):

| run | result |
|---|---|
| full suite, ~21:44 | **PASS** |
| full suite, ~23:05 | **FAIL** — `displayed_right_panel_mode()` returned `Metadata`, expected `AppPane(local://yedit / notes)`, *"back on its own row the pane returns without a re-open"* |
| that test ALONE, ~23:20 | **PASS** (`1 passed; 1855 filtered out`) |

⇒ Not a regression and not load: **it is order- or shared-state-dependent
under the harness's parallel threads.** The test binary runs the same tests
either way, so what differs between the two full runs is thread interleaving.

⛔ This is the project's own **"No non-determinism"** rule (`CLAUDE.md`) broken
in a test rather than in the product, and it is the more expensive place for it:
a flaky test in a suite that already carries documented reds is indistinguishable
from a seventh documented red, so it will be inherited as "known" and never
looked at. The fix is to find the shared state — a `static`, a global panel-mode
cache, or a thread-local the pane lookup reads — and give the test its own.

⚠ Do not "fix" it by asserting less. The assertion is the CONTRACT (a pane
returns to its own row without a re-open); what is wrong is that something
outside the test can change the answer.

## AUTO-PROVISIONING COVERS THREE OF THE SIX NEW CLIs — the other three land in a shell that stays

**Status:** OPEN

Measured 2026-08-08 by launching all six locally on guihost, which had none of them
installed at the start. `~/.yggterm/npm/bin` afterwards: `claude codex
codex-litellm opencode pi qwen` — **`pi`, `opencode` and `qwen` were provisioned
on demand and launched.** The other three showed, as the whole of the session:

```
/bin/bash: line 1: kimi: command not found
/bin/bash: line 1: muse: command not found
/bin/bash: line 1: agy: command not found
```

The row stays `healthy`, the shell stays open at a prompt, and **nothing above
the terminal screen says the CLI is missing** — no launch error, no row state,
no trace event. The only instrument that can answer "did my CLI start?" is
reading the screen text.

⛔⛔ **THE OWNER OVERRULED THE "declare it host-provided" ESCAPE HATCH,
2026-08-08:** *"Trying to launch Muse CLI shows notification to install it.
yggterm should auto install, update ALL clis in all connected systems including
localhost."* ⇒ **there is no per-descriptor opt-out to write.** Every CLI is
auto-installed, and Muse is no longer parked in `owner-attention.md` (only its
LOGIN is still his). Full ruling: [`settled-calls.md`](settled-calls.md).

### ✅ Built in 3.0.65 — the LOCAL lane, live proof owed

1. ✅ **The provisioner dispatches per method.** `install_latest` partitions by
   `ProvisionStep`: npm tools still share ONE batched `npm install -g` line, uv
   tools get `uv tool install --upgrade <pkg>`, and a `VendorScript` is fetched
   over pinned HTTPS (`--proto '=https'`, `--tlsv1.2`) and executed. A failure in
   one CLI's method is COLLECTED, not short-circuited, so one vendor installer
   dying no longer stops the next CLI's update.
2. ✅ **The superseded clause is rewritten, not merely outvoted.** The
   `VendorScript` doc comment now carries the ruling and says why the old text is
   gone, and `every_cli_says_how_it_is_installed` FAILS if the words "never runs
   that unattended" come back — the refusal cannot be re-derived from the type's
   own documentation.
3. ✅ **The `Manual` audit answered a question the entry did not ask.** `agy
   --help` on guihost advertises `update  Update CLI`. So `Manual` is right about
   ARRIVAL — yggterm cannot fetch a 166 MB binary served behind a sign-in — and
   wrong as a verdict on the CLI: it keeps itself current perfectly. Arrival and
   staying-current are now two registry axes (`CliInstall` + `CliUpdate`), a
   self-updater is PREFERRED over re-running the install method, and the registry
   fails its own test if any CLI is both unfetchable and unupdatable.
4. ✅ **`probe_tool` reads the LAUNCH-PARITY PATH.** An npm install lands in
   `~/.yggterm/npm/bin` and was found by path; a uv or vendor install lands in
   `~/.local/bin`, which the daemon's own `PATH` omits. Without this the probe
   would report a CLI we had just installed as absent and `ensure_local_managed_cli`
   would bail with *"did not become available after the managed install
   finished"* — on a SUCCESSFUL install.
5. ✅ **The `npm is unavailable` gate is per-tool and names the right thing.**
   One global "is npm here" was wrong twice over on a uv CLI: npm's absence is
   not why `kimi` is missing, and npm's presence would not have fixed it.

### ⭐⭐ ROOT-CAUSED 2026-08-08: THE REMOTE LANE IS A DEFAULT THAT OUTLIVED ITS ROLE

**It is NOT a binary-distribution problem, and it is NOT a missing capability.**
Both hypotheses were tested and both are false. Do not re-derive either.

The remote lane's provisioning route is exempted at
`local_managed_cli_tool_for` (lib.rs ~15591): a `remote-session://` /
`remote-cc://` path, **and any `SessionSource::LiveSsh` row** (which is how all
six new CLIs are born), returns `None`, so the attach funnel's
`ensure_managed_cli_for_session_path` never provisions them. The doc comment
justifies that exemption in one sentence: *"Background machine refreshes keep the
remote toolchains current."*

**That refresh runs, and is structurally forbidden from installing anything.**
The GUI's `maybe_spawn_missing_managed_cli_refreshes` (shell.rs ~36196) does
iterate remote machines and does fire — but it calls
`refresh_managed_cli(.., background = true)`, and
`refresh_local_managed_cli` then hits:

    if !skipped_recently && !install_deferred && !background_install_enabled {
        install_deferred = true;          // trace: refresh_defer_background_install
    }

`managed_cli_background_install_enabled()` reads
`YGGTERM_MANAGED_CLI_BACKGROUND_INSTALL` and **defaults to false**. The variable
is set nowhere in the repo or on any fleet host. ⇒ every remote refresh is
probe-only, forever.

**LIVE-PROVEN from the mechanism's own trace, on both remote hosts:**

| host | `refresh_begin` | `refresh_defer_background_install` |
|---|---|---|
| integrator | 2 | **2** |
| workshop | 13 | **13** |

Payload: `{"background":true,"reason":"background_install_opt_in_required",
"env":"YGGTERM_MANAGED_CLI_BACKGROUND_INSTALL"}`. Outcome agrees: both remotes
carried exactly `codex`, `claude`, `agy` and **none** of the six new CLIs.

⚖ **The default was CORRECT when it was written** and the CHANGELOG says why:
*"Keep background managed-Codex refresh probe-only by default, so live terminal
recovery and remote scans cannot spawn `npm install @latest` and blow the
fan/CPU budget."* The ruling changed the component's role, so the
default became the bug — [[finding-our-own-policy-was-the-bug]] exactly.

### ⛔ THE CAPABILITY HAS BEEN PRESENT ALL ALONG — nothing calls it

`server remote ensure-managed-cli <slug>` is routed
(`remote_cli.rs:243/353` → `run_remote_ensure_managed_cli` →
`ensure_local_managed_cli`) and **works on the OLD remote binaries**. Run by
hand 2026-08-08 against the integrator (3.0.64) and the workshop (3.0.62), it
installed `qwen` 0.21.8 on both in seconds; verified by file
(`~/.yggterm/npm/bin/qwen`), not by the command's own echo.

⇒ One hand-run command did what months of background refreshes could not.
**The remote binary is capable; the launch path simply never asks.**

### ⛔ THE OBVIOUS FIX IS THE WRONG ONE

Do **not** flip `managed_cli_background_install_enabled()` to default true. That
re-creates precisely the fan/CPU problem the CHANGELOG records, on every machine,
on every background refresh, for all nine CLIs at once — and the owner notices
when the GUI host runs hot.

**The symmetric fix is the local lane's own shape:** provision ONE CLI, ON
DEMAND, at the moment a remote agent row is launched, by invoking the verb that
already exists. ⚠ It must hang off the CREATE path, not the focus path:
`ensure_terminal_for_path_with_initial_size_and_seed` is the funnel for both, and
an ssh round trip per focus is the ~85-910 ms regression
`local_managed_cli_tool_for`'s own comment was written to prevent. A TTL cache
keyed by `(machine_key, tool)` is what makes create-vs-focus separable, mirroring
`ensure_local_managed_cli_for_focus`.

### ✅ Built in 3.0.66 — the REMOTE lane, live proof owed

1. ✅ **`remote_managed_cli_tool_for` is the EXACT COMPLEMENT of
   `local_managed_cli_tool_for`.** Between them every agent row is claimed by
   exactly one provisioning lane — `every_agent_row_is_provisioned_by_exactly_one_lane`
   asserts the partition over all nine agent kinds × six row arms, so neither a
   gap (a row nobody provisions — this bug) nor an overlap (a remote row paying
   for a local `<cli> --version` per focus) can reopen as row kinds grow.
2. ✅ **The machine comes from `session.ssh_target`, and that was MEASURED, not
   assumed.** It is the exact input `machine_key_from_ssh_target` →
   `remote_target_for_machine_key` consume, so it round-trips to the target that
   made the row. `host_label` is `SshConnectTarget::label`, a DISPLAY string read
   elsewhere through the looser `machine_key_from_labelish`, and it coincides
   only on fleets whose labels equal their ssh aliases. Live daemon snapshot:
   **32 of 32 `LiveSsh` rows carried a non-null `ssh_target`; zero nulls.** All
   seven non-test `LiveSsh` birth sites set it.
3. ✅ **The ensure is cached per `(machine_key, tool)`**, so only the FIRST launch
   of a CLI on a machine costs an ssh hop and every focus inside the TTL costs
   nothing — create-vs-focus separated without plumbing an "is this a create"
   flag through the funnel.
4. ⭐ **The local lane's negative-cache rule DOES NOT TRANSFER, and copying it
   would have been the regression.** `managed_cli_focus_cache_entry_is_fresh` is
   `available && …` — locally a negative is never cached, which is right when a
   miss is a filesystem stat and the install is kicked to a thread. Remotely a
   miss is an SSH ROUND TRIP, so an uncached negative charges one on EVERY focus
   of a row whose CLI is missing — the ~85-910 ms regression, on the machine
   where it hurts most. Two TTLs instead: 6 h for present, 60 s retry for absent.
   `a_missing_remote_cli_is_cached_so_focus_never_pays_per_click` locks it, and
   goes red when the local rule is pasted in.
5. ⭐ **The install runs in the BACKGROUND, and the brief's "foreground" was
   refuted by measurement.** The funnel is `&mut self` on the daemon's request
   path, so an unbounded foreground ensure would stall every other row on the
   machine for a whole first-run npm install; a *bounded* one would `kill()` that
   install mid-flight and leave a half-written npm tree. It therefore mirrors
   `spawn_background_managed_cli_refresh`, deduped per `(machine_key, tool)`.
6. ✅ **Provisioning cannot fail a launch.** `ensure_remote_managed_cli_for_session_path`
   returns `()`, not `Result`: a briefly unreachable host yields a warning and a
   60 s negative entry, never a dead row.

### What 3.0.66 has and has NOT been proven to do

✅ **PROVEN LIVE, by file.** The verb and slug this code sends —
`server remote ensure-managed-cli qwen`, the slug being
`ManagedCliTool::binary_name()` — installed Qwen Code 0.21.8 on a host that had
never carried it, read back as
`~/.yggterm/npm/bin/qwen -> ../lib/node_modules/@qwen-code/qwen-code/cli-entry.js`
and NOT from the command's own echo. That also confirms `binary_name()` is an
accepted slug for `parse_managed_cli_tool`.

✅ **UNIT-LOCKED, each mutation-proven red.** The lane partition
(`every_agent_row_is_provisioned_by_exactly_one_lane`), the two-TTL negative
cache (`a_missing_remote_cli_is_cached_so_focus_never_pays_per_click`), and the
`ssh_target`-not-`host_label` resolution against a real server object
(`a_remote_row_provisions_on_the_machine_its_ssh_target_names`, whose fixture
sets `host_label` to a deliberately wrong value and passes only if the resolver
round-trips the machine key back to its target).

⛔ **STILL NOT PROVEN, and 2026-08-09 found out WHY — the blocker is a second
defect, not the restart.** The GUI restart WAS taken (3.0.67, watcher left
running deliberately and it rode through). The attempt then failed for a reason
nobody had predicted:

**There is no headless route that can birth a remote agent row for these six
CLIs.** `server app terminal new --machine-key <host> --kind opencode` created
an `ssh_shell` wearing the label "Agent unnamed opencode" — see
[the silent-downgrade entry](#-a-remote-agent-row-silently-became-a-plain-shell--six-of-nine-clis).
So the row under test was never an opencode row, and **the provisioning lane
declined it CORRECTLY**: `ManagedCliTool::from_session_kind(SshShell)` is `None`.
Zero provisioning trace events fired, which was the right behaviour on the data
it was given.

⚖ **Read that carefully before treating it as a failure of this lane.** Nothing
was learned against the provisioning code; the test was invalid. What WAS
learned is that the falsifier needs a genuine remote agent row, and today only
`codex` and `claude-code` can be born remotely at all — neither of which is
absent on any fleet host, so neither can demonstrate an install.

⇒ **The proof is blocked on giving the six CLIs a remote start contract**, which
is the remaining work in the silent-downgrade entry. Until then the honest
statement is: the verb and slug are proven by hand, the resolution is proven by
unit test against a real server object, and **the funnel-to-install chain has
never been observed end to end.**

⚠ Confirmed en route, so it need not be re-derived: the session record for a
`--machine-key` row DOES carry `ssh_target` (non-null, equal to the machine
key) and
`source: LiveSsh`. The predecessor's `ssh_target`-not-`host_label` finding holds
on this birth path too — the kind was the only thing wrong.

⚠ **An isolated-daemon harness was tried and does NOT substitute.** A synthetic
`live_sessions` row rehydrates as a plain local shell (`exec '/bin/bash' -i`),
and `server attach` takes a UUID — it prefixes `local://`, producing
`local://remote-session://…`, which the remote lane then correctly declines.
`server connect`, the headless twin of clicking a row, lives in the GUI binary
and is not in `yggterm-headless`. ⇒ a headless daemon cannot birth a real remote
agent row; do not spend another session rebuilding this harness.

⚠ **STATE CHANGED ON A THIRD HOST, 2026-08-08 — declared.** The falsifier above
installed `qwen` on one of the two previously-untouched remotes. It now carries
`codex claude agy qwen`. A later session must not read it as pristine.
- ⚠ **A second, smaller gap sits behind it, now MEASURED rather than assumed.**
  The remote binaries are 3.0.64 and 3.0.62, so their `ensure_local_managed_cli`
  predates 3.0.65's per-method dispatch. Run by hand against both remotes:

  | method | CLI | old remote binary answers |
  |---|---|---|
  | npm | `qwen`, `pi`, `opencode` | **installs** (0.21.8 / 0.84.1 / 1.18.15) |
  | uv | `kimi` | `Error: … yggterm does not provision it — install it yourself from kimi-cli` |
  | vendor | `muse` | `Error: … install it yourself from <vendor installer>` |

  ⇒ the uv/vendor half needs 3.0.65+ ON THE REMOTE HOST; the npm half needs no
  new bytes anywhere. The lane being dead for npm CLIs too is what makes the
  wiring the first fix and the distribution the second.

- ⚠ **RE-MEASURED 2026-08-08, and the inherited baseline was wrong twice.**
  Probed BY FILE across the launch-parity dirs on every remote machine the
  daemon knows, not just the two that had been looked at:

  | machine | yggterm | carries |
  |---|---|---|
  | integrator | **3.0.65** (not 3.0.64 as recorded) | codex claude agy qwen pi opencode |
  | workshop | 3.0.62 | codex claude agy qwen pi opencode |
  | two further hosts | 3.0.62 | codex claude agy **only** |

  ⇒ **There are FOUR remote machines in the registry, not two.** The two that
  were never hand-touched still lack all six new CLIs, so they are the clean
  falsifier targets — and being 3.0.62 they can still install the npm three
  without any remote deploy at all. The integrator being 3.0.65 also means the
  uv half is testable there TODAY.

- ⛔ **yggterm bootstraps a MISSING remote binary; it never upgrades a STALE
  one** — which is why remote hosts sit at whatever version first bootstrapped
  them. `resolve_remote_yggterm_binary` caches per `local_build_id`, but the
  revalidation it runs is `check_remote_protocol_version` — a PROTOCOL check.
  A remote at 3.0.62 whose protocol still matches is accepted and never
  re-uploaded, so a release with new provisioning behaviour does not reach it.
  This is a distinct defect from the lane wiring and is what the uv/vendor half
  is actually blocked on.

⚠ **STATE CHANGED ON TWO REMOTE HOSTS, 2026-08-08 — declared, not hidden.** The
by-hand `ensure-managed-cli` falsifier installed `qwen`, `pi` and `opencode` on
both the integrator and the workshop. They now carry 6 of the 8 (`codex`,
`claude`, `agy`, `qwen`, `pi`, `opencode`; `kimi` and `muse` still absent per the
table). A later session must not read those hosts as pristine, and must not read
the presence of those three as evidence that the lane works — **it does not; a
human ran the verb.**
- ⛔ **Live proof on guihost** per the falsifier below — read BY FILE on the host
  itself, not an echo of the launch.

⚠ Scope word he used: **"all connected systems including localhost"**. Localhost
is named because the local path and the remote-provisioning path are different
code, and a fix proven on one is not proven on the other.

⚠ Falsifier for a fix (do not accept an echo): on a host with `muse` absent,
launch a Muse row and then read the binary **BY FILE on that host** — plus
`~/.config/muse/auth.json` presence to tell "installed" from "authenticated".

⛔⛔ **DO NOT use `command -v <cli>` over `ssh host 'cmd'` — it is a BLIND
INSTRUMENT, measured 2026-08-08.** A non-interactive ssh runs no login shell, so
`PATH` omits both `~/.local/bin` and `~/.yggterm/npm/bin`; it reported every CLI
ABSENT on a host that was carrying three. Test each launch-parity directory as a
FILE instead. This is the same root as 3.0.65's `probe_tool` fix, one layer out —
and it is why the line above no longer prescribes `command -v`.
The existing 3.0.59 refusal-by-name will otherwise fail the row cleanly and look
like correct behaviour.

✅ The other half — **a failed exec must surface as a row-level launch failure,
not as a line of scrollback in a shell that outlives it** — shipped in 3.0.59.
A local agent launch now probes the binary at the one PTY funnel and refuses by
name; `kimi`, `muse` and `agy` fail their rows instead of becoming a shell. That
does NOT close this entry: a refusal is the right answer only for a CLI yggterm
genuinely cannot install, and `kimi` and `agy` are not that.

## ⛔⛔ `terminal new --prompt` DROPPED A DELEGATE'S ENTIRE BRIEF AND REPORTED A GOOD LAUNCH — 8 HOURS LOST

**Status:** OPEN

Measured 2026-08-07 21:15 IST, live, on the campaign relay itself. A successor session was spawned with a 192-line runbook on
`--prompt-stdin`. **The runbook never arrived.** The delegate's first and only
user message was:

```
yggterm_ready_probe\x15yggterm_ready_probe\x15yggterm_ready_probe\x15
yggterm_ready_probe\x15yggterm_ready_probe\x15yggterm_ready_probe\x15
```

It answered `Ready.` and stopped. Nobody noticed for **eight hours**, which is
the whole cost of this entry.

### The probe is not side-effect-free, and that is the bug

`submit_prompt_echo_verified` (`crates/yggterm-server/src/terminal.rs`) proves a
CLI is consuming input by writing `yggterm_ready_probe`, checking it echoes to
the daemon screen, then clearing with Ctrl+U. Its doc says the clear is
"self-healing across retries".

⭐ **It is not, because "not ready" does not mean "your bytes were discarded" —
it means they are QUEUED.** A CLI that has not started reading still has a PTY
behind it, and the PTY buffers. So the six probe+Ctrl+U pairs sat in the buffer,
the echo never appeared (nothing was rendering yet), the gate correctly
concluded not-consuming and correctly declined to write the real prompt — and
then the buffer flushed the probes into the composer, where they were submitted
as the delegate's opening message.

**The gate's own instrument poisoned the row it had just refused to use.** Any
readiness probe that WRITES cannot be non-destructive against a program that is
not yet reading.

⚠ Ctrl+U also did not clear Claude Code's composer here — six probes
accumulated rather than replacing one another.

### The second half: nothing in the reply said it failed

`terminal new`'s reply carries a `launch` block — `applied`, `model`,
`permission_mode`, `launch_command` — and its `--help` says it "reports what the
ROW was born with". **It says nothing about whether the prompt was delivered.**
So `launch.applied: true` is true and useless: the row was born exactly as
asked, holding no brief.

⚖ `shell.rs`'s own `app_control_created_seat_report` doc already lists
**`--prompt-stdin`.delivered** as one of six verbs "measured reporting the
request rather than the effect". The list was right; this path never got the fix.

**Owed:** `terminal new --prompt*` must answer with a `prompt` block re-read
from the row — `{submitted, waited_ms, reason}` — and a non-zero exit when
`submitted:false`. A delegate launcher cannot verify what the launcher will not
report.

### ✅ THE WORKING SEQUENCE, PROVEN 2026-08-08 — use it until this is fixed

`terminal submit` (which exists, reports honestly, and is **missing from
`terminal`'s own verb list** in `--help` — a third, smaller defect):

```sh
# 1. create with NO --prompt
P=$(… terminal new --kind claude-code --no-activate --model … --permission-mode bypass …)
# 2. WAIT for the row to actually be reading. Measured: 5.9 s on a cold claude-code row.
…terminal input-check "$P" --check-timeout-ms 20000     # want consuming_input:true
# 3. submit, and READ `submitted`
… | …terminal submit "$P" --stdin                        # answers submitted:true, waited_ms
# 4. ⛔ VERIFY BY TRANSCRIPT CONTENT, never by the reply
grep -q '<a distinctive token from your brief>' ~/.claude/projects/<cwd-slug>/<uuid>.jsonl
```

Step 4 is the one that would have caught this in 30 seconds. The failed launch
DID produce a transcript file, 28 KB of it — **existence proved nothing.**

⛔ Do not "fix" this by lengthening the readiness timeout. The gate was not too
impatient; it was writing into a buffer it could not see and could not take back.


## THE SUITE BASELINE — FOUR RED AT HEAD, AND TWO OF THEM REACH THE NETWORK

**Status:** OPEN

⭐ **This entry is the ONE owner of "what is red at HEAD", and the number in it
is load-bearing** — every brief quotes it, and the first thing an agent does with
an unexplained red is suspect its own diff, which has cost a session a stash, a
rebuild and a re-run more than once. **A wrong baseline is worse than none.**

### ✅ MEASURED 2026-08-09 ON THE INTEGRATOR, `cargo test --workspace --no-fail-fast`

⛔ Use `--no-fail-fast`. Plain `cargo test` is fail-fast **across targets**, so a
red target hides every target behind it and the tail still prints `ok`
([[finding-a-red-target-hides-every-test-behind-it]]).

**24 targets · 3501 passed · 4 failed · 1 ignored.**

```
daemon::tests::daemon_binary_is_legacy_allows_deleted_current_install_path
tests::start_remote_claude_session_assigns_authoritative_session_id
tests::start_remote_codex_session_uses_remote_start_codex_launch_contract
shell::tests::a_pane_is_a_tenant_of_the_session_that_declared_it_never_of_a_namesake   (own entry: flaky in parallel)
```

### ⭐ NINE TESTS THIS ENTRY USED TO LIST AS RED ARE GREEN — VERIFIED ONE BY ONE

The previous baseline said **six** yggterm-shell retention tests plus **six**
yggterm-server tests. Each of the nine that are no longer failing was re-run
**individually by name** rather than inferred from the totals
([[feedback-never-bulk-close-by-category]]), and all nine pass:

- the six retention tests (`inactive_retained_ready_session_…`,
  `prune_terminal_attach_in_flight_…`, `retained_background_session_trickles_…`,
  `shell_snapshot_retains_live_local_stored_codex_sessions`,
  `shell_snapshot_trims_inactive_live_payloads_…`,
  `sync_live_terminal_retention_keeps_active_…`);
- the three the entry called *"a real, unfiled regression"* because the resume
  SUBCOMMAND had gone missing from the built launch string
  (`legacy_agent_launch_command_uses_best_effort_cwd_resolution`,
  `remote_resume_shell_command_wraps_prefix_and_cwd`,
  `stored_codex_litellm_sessions_use_litellm_resume_command`).

⚠ **Honest limits on that.** Nobody bisected what fixed them — the launch-string
three are plausibly the 3.0.70 launch-composition work, but that is a guess and
is recorded as one. And this was measured on the INTEGRATOR; the previous
measurement was on the GUI host, and two of the four survivors are known to be
environment-dependent, so *green here* is not *green everywhere*
([[finding-a-claim-proven-on-one-lane-is-not-proven]]).

### ✅ SETTLED AND FIXED 2026-08-14 [6.6] — THE FORK IS RETIRED, THE ANSWER WAS A THIRD OPTION

⛔ **Do not carry the two-way fork forward.** This section offered *"either the
table composes the resume subcommand differently (a live regression), or the
tests assert a path the product no longer takes (stale tests)"*, and instructed
that the entry must not guess between them. That was the right refusal and the
wrong menu — **the answer was on neither branch**:

- **"Stale tests" is FALSE.** The path is very much live; every agent CLI resumes
  through it.
- **"Live regression" is TRUE**, but it was not the table composing the
  subcommand differently.
- ⭐ **THE THIRD OPTION, which is the finding:** the assertion encoded a **FALSE
  INVARIANT** — `<binary>` immediately followed by `<subcommand>`, which the
  composer never promised — so it **read ambient HOST state**, from two
  independent sources. Its greenness was a fact about who ran it and on what
  machine, not about the code. That is why it oscillated for months with nobody
  changing it, and why "it fixed itself" kept being recorded.

⇒ **A test that reads ambient state is not flaky — it is measuring the host.**

- **The live bug.** `configured_cli_extra_arg_tokens` read
  `YGGTERM_AGENT_EXTRA_ARGS` out of ambient process environment for **every**
  `SessionKind` — one variable, nine CLIs — so one CLI's permission flag was
  returned verbatim for another. Nothing in the daemon ever *sets* that variable
  (unlike its Claude Code sibling, which is written from the request and guarded
  on kind), so an ambient value could only ever be pollution inherited from the
  spawning process.
  ⛔⛔ **AND THE POLLUTION IS LIVE, NOT LATENT — say it that way.** Measured
  independently by two seats: **3 of 20 running daemons carry
  `YGGTERM_AGENT_EXTRA_ARGS` set to a Claude-only permission flag, and one of
  them is the current-release daemon serving by default**; another owns nine
  sessions. The interactive shells that spawned them carry it too, which closes
  the inheritance chain. ⇒ **It is one non-Claude launch away from real.** The
  code path is fixed here, but a running process keeps its frozen environment:
  **the fix reaches those daemons only on a bump**, and a bump is held. A reader
  who sees "latent" will not know that.
- **Why the subcommand "went missing".** Extra args compose BETWEEN binary and
  subcommand by design, so the injected flag displaced the adjacency the
  assertion required. Nothing was ever missing.
- **Why it oscillated for months.** The needle `codex resume -C "$PWD"` encoded a
  false invariant, so it read ambient host state — the settings store
  (long-standing: configuring extra args for a CLI, a *supported* setting, turns
  it red with zero code change) and the env var (added the day before). "It fixed
  itself" was always someone running it from a different shell.

Fixed in `6574d385`: forwarded flags travel only as the request field both
wrapper entrypoints already pass (`configured_override`), and the tests are
re-pinned to the real invariants. Verified across five ambient conditions
(variable set/unset × settings empty/configured), two of which were red before.
⇒ `main` is no longer red here. Full account:
[[finding-a-test-that-reads-ambient-host-state-is-not-flaky]].

### The two that measure the network, which is the real defect

Confirmed 2026-08-08 during the six-CLI intake, and confirmed NOT caused by it:
`git diff` touches none of the three functions under test.

```
tests::start_remote_codex_session_uses_remote_start_codex_launch_contract
tests::start_remote_claude_session_assigns_authoritative_session_id
daemon::tests::daemon_binary_is_legacy_allows_deleted_current_install_path
```

**The first two are environment-dependent, which is the real defect.**
`normalize_remote_attach_cwd` SHELLS OUT over ssh to resolve the cwd. On a host
that cannot reach the remote it fails closed and the assertion passes; on a host
that CAN, the resolver walks up from a path that does not exist there and returns
`/home`, so `contains("…/gh/yggterm")` fails. Verified by running the resolver
script over ssh by hand. **A unit test whose verdict depends on whether the
machine has network reach to another machine is not a unit test** — it passes for
the wrong reason on the machine where it passes. Fix by injecting the resolver,
not by making the assertion looser.

The third asserts `!daemon_binary_is_legacy(current, <current install path>,
Some(<same path>))`; neither `daemon_binary_is_legacy` nor
`daemon_expected_binary_paths` is in the diff.

⛔ Do not silence these. The first two hide a test that measures the network.

## NO APP-CONTROL VERB RAISES THE SIDEBAR ROW MENU — its UI cannot be live-proven

**Status:** OPEN

Found 2026-08-08 while trying to satisfy `CLAUDE.md`'s own rule that a UI change
is not done until a live screenshot confirms it. The row menu gained a second
layer that day, and there is **no way for an agent to open it**.

`server app` can screenshot, read rows, drive a contributed pane, click inside a
web surface, show/hide the KeyTip overlay and audit the KeyTip tree. None of
those raises `ContextMenuOverlay` on a cwd-tree row. The two paths that do are a
mouse `oncontextmenu` on the row and the `ALT,E` chord, and the chord is caught
by a window-level JS keydown listener that app-control cannot feed.

**What this costs:** the row menu is one of the densest interaction surfaces in
the product — five mounts, keytip badges, disabled-item tooltips, and now a page
turn — and every claim about how it LOOKS is currently unfalsifiable from an
agent session. `keytips audit` proves the tree; it does not prove a pixel.

**The fix is a verb, not a workaround:** `server app row-menu open <row-path>
[--page <opener-id>]` / `close`, driving the same `open_context_menu` the mouse
does. It would also give the KeyTip badge painter its first end-to-end test.

⚠ Do not close this by screenshotting after asking the owner to right-click. The
point is that the campaign runs unattended.

## THE TITLEBAR `+` MENU IS THE LAST HAND-LISTED CLI SURFACE

**Status:** OPEN

Found 2026-08-08 while landing the six-CLI intake (pi, opencode, qwen-code, kimi,
muse, antigravity). Every other surface that offers "start a session with CLI X"
is now derived from `AGENT_CLIS`: the cwd-tree row menu, its ALT submenu, the
start page's session family, and the sidebar icon. The titlebar `+` menu is not.

It is hand-rolled `rsx!` — one `button` and one `on_start_*` callback per entry —
rather than a list of `RowMenuItem`, so it never joined the registry the way the
other menus did. Its KeyTip node is the literal `"insert.claude"`
(`build_keytip_scopes`, with the comment "not in the enum registry yet"), and its
dispatch is `if key == "insert.claude"`.

**What a user sees:** the six new CLIs appear in the row menu's `Open Session
Here ▸` submenu and on the start page, and are absent from the `+` menu, which
offers Codex, Claude Code and Terminal only. The `+` menu is therefore quietly
telling the user that three CLIs exist.

**The fix is not to add six more buttons** — that reproduces the shape at nine.
The `+` menu should draw `RowMenuItem`s like the four other menus in the app, at
which point `agent_session_menu_items()` feeds it and the KeyTip declarations
fall out of the list exactly as the row menu's do. `docs/spec-adding-an-agent-cli.md`
§2 step 7 names it as the remaining surface.

## ⛔⛔ THE VENDORED `dioxus-desktop` NO LONGER CROSS-COMPILES FOR ANDROID — drillkit-rs cannot cut an APK

**Status:** OPEN

Found 2026-08-07 by the drillkit-rs EXAM exam-console row while following
drillkit-rs's own standing rule that a client-side change must ship an Android APK release.

`vendor/dioxus-desktop` declares and uses the yggterm web-surface module **unconditionally**,
but that module is Linux/WebKitGTK-only by construction — its own header says so ("This is the
Linux/WebKitGTK path"), and it builds on `gtk::Overlay` / `build_gtk`. There is no GTK on
`aarch64-linux-android`, so the module cannot compile there and every reference to it fails:

```
error[E0433]: cannot find `web_surface` in `crate`          x12
error[E0609]: no field `web_surface_host` on type `&DesktopService`   x2
error[E0282]: type annotations needed                        x1   (let web_surface_backdrop;)
error: could not compile `dioxus-desktop` (lib) due to 15 previous errors
```

The three unconditional sites:

- `vendor/dioxus-desktop/src/lib.rs:28` — `mod web_surface;` (and `:29`, `:37` `pub use web_surface::{...}`)
- `vendor/dioxus-desktop/src/desktop_context.rs:82` — the `web_surface_host` field on `DesktopService`
- `vendor/dioxus-desktop/src/desktop_context.rs:134,162` — `install_web_surface_host`, `SurfaceUserscript`

**This is a REGRESSION, and it is datable.** drillkit-rs release `v0.1.0-beta.22` (16 Jun 2026)
carries a working `drillkit-rs-android-arm64-dev.apk`; `vendor/dioxus-desktop/src/lib.rs` was last
touched by `4cb67d00` (2026-08-01, "feat(web): the legacy browser keys, and a screenshot on the
page menu"). So Android built before that work and does not now.

⚠ **Not fixed here on purpose.** The fix is a `#[cfg]` pass over the module, the `DesktopService`
field and its callers, and a wrong gate silently disables web surfaces on Linux — yggterm's own
core feature. That belongs to whoever owns the web-surface plane, not to a consumer repo's exam
row. **The falsifier is concrete:** `cd ~/git/drillkit-rs && ./scripts/build-android-armv8.sh`
produces a signed `target/release/drillkit-rs-android-arm64-dev.apk`.

⭐ **Worth knowing for anyone consuming this vendor dir:** the WEB target is unaffected
(`dx build --web` was run and verified live the same day). Only Android is down, and it is down
hard — no APK release is possible from any fleet host until this is gated.

⚠ Second, smaller thing found in the same run, listed so it is not rediscovered: `oc` (the only
host with the Android SDK/NDK) has **Java 17**, while `drillkit-rs/scripts/build-android-armv8.sh`
defaults `JAVA_HOME` to `java-21-openjdk-amd64`. It honours an explicit `JAVA_HOME`, so
`JAVA_HOME=/usr/lib/jvm/java-17-openjdk-amd64` gets past the dependency check and Gradle is happy;
the Rust compile above is what actually stops the build.

## ★★ WEBKIT'S OWN POPUP BLOCKER EATS `window.open` BEFORE OUR POPUP PIPELINE SEES IT — and no agent can tell

**Status:** OPEN

**Operator-reported 2026-08-07**, twice in one session, both halves in the requirement:
> *"I tried clicking the receipt and label from the webapp and the webapp complains popup is
> getting blocked. So ychrome also needs a pipeline to direct popups to new tabs."*
> *"Also agents need to know that popup has fired, like you said you cannot tell anything."*

⚠ **The pipeline he is asking for ALREADY EXISTS and is good** — `surface_new_window_handler` →
`build_popup_webview` → `take_web_surface_popups()` → `web_surface_adopt_popup_tab`, related-view
so `window.opener` and `window.close()` are live, adopted rather than re-navigated so a POSTed
OAuth callback is not replayed. **That is why this is worth fixing rather than building: the whole
mechanism is one setting away from running.**

### Half 1 — the setting. `create` never fires, so none of that code runs

```
$ grep -rn 'javascript_can_open' --include=*.rs vendor/ crates apps
vendor/wry/src/webkitgtk/mod.rs:461:        settings.set_javascript_can_access_clipboard(true);
   # …_can_access_clipboard. Nothing anywhere sets _can_open_windows_automatically.
```

`WebKitSettings:javascript-can-open-windows-automatically` **defaults to FALSE**, and with it false
WebKit refuses any `window.open` that is not inside a **live user-activation window** — before
emitting `create`. So:

- the page receives **`null`** and prints its own *"popup blocked"* message (what the operator saw);
- `connect_create` never fires, so `surface_new_window_handler` never runs, so nothing is ever
  adopted as a tab, and **the trace event `web_surface.popup_adopted` cannot be emitted either**.

⛔ **The blocked case is the NORMAL case for real apps, not an edge.** A modern SPA does
`const r = await fetch(...); window.open(r.url)` — and the `await` spends the activation. Measured
on `app.indiapost.gov.in` 2026-08-07: **the payment gateway popup was blocked** (a booking
tab existed; no second tab appeared in `web_surface_tabs`), and the **Receipt and Label buttons in
My Bookings are simply unreachable**. The gateway leg only completed because the agent
monkey-patched `window.open` to steal the URL and relaunched it through the ychrome CLI.

⚠ **ychrome's ENGINE already made this exact call and wrote down why** —
`ychrome/src/engine/host.rs::arm_new_window`: *"That is the shape a bank-payment gateway takes — the
merchant form targets a popup — so an agent driving a payment saw a successful click and a page that
never moved."* ⇒ **the `ctl` engine can take a popup and the GUI surface cannot**, which inverts the
usual asymmetry and strands the one plane that has the card rail.

**Ask:** set `javascript_can_open_windows_automatically(true)` on the surface's `Settings` so
`create` reaches the handler we already have. The blocking that matters is not WebKit's heuristic —
it is ours, in `surface_new_window_handler`, which can already `Deny` with a reason and does.

### Half 2 — an agent cannot observe a popup at all, and reads silence as "the button is dead"

Even when adoption works, **nothing about it reaches the agent control plane.** `popup_adopted` goes
to the trace file; `server app state | jq .web_surface_tabs` shows the new tab with no hint it was a
popup or who opened it; there is no `web popups` verb and no `web wait --until popup`. An agent's
only instrument today is patching `window.open` in the page itself.

**The cost, measured:** the agent that booked and paid the India Post article recorded in the
widgets run note that *"the Receipt / Label buttons produce nothing — no `window.open`, no
dialog, no download"* and filed it as an unsolved site quirk. **That finding was WRONG**, and it was
wrong in the direction that poisons the next run: the popup was blocked, the site is fine, and a
site-lore entry now had to be corrected. **A blocked popup and a dead button are indistinguishable
from the agent side, and the agent will always guess "dead button".**

**Ask, smallest useful shape:**
- `web popups --session <s>` → the recent popup decisions: `{opener_tab_id, url, outcome:
  adopted|denied|blocked_by_engine, new_tab_id?, reason?, at}`. **`blocked_by_engine` must be
  reportable even after half 1 lands**, because our own handler still denies (a dead opener, a
  failed build) and those must not look like nothing happening either.
- `web wait --until popup` / `popup:<url-substring>`, so a payment or OAuth hop is awaited rather
  than polled.
- `popup_of: <tab_id>` on the `web_surface_tabs` rows, so a listing answers "where did this tab come
  from" without a trace file.

⇒ Same law as everywhere on this plane: **an operation's own success field is an assumption; the
observable is the state.** A popup currently has no observable at all.

Field context: `~/data/widgets/graph/notes/indiapost-rti-a-booking-run-2026-08-07.md`;
site-lore `app.indiapost.gov.in` slug `drop-off-booked-and-paid-surcharge-and-input-rungs`.


## ⛔⛔ REGRESSION: A YEDIT ROW SHOWS ITS TERMINAL WHERE ITS DOCUMENT SHOULD BE — and the titlebar says Document

**Status:** OPEN

reported 2026-08-07 with a screenshot, called an emergency: *"yedit switching
views does not work. The edit view is not working. This is a regression."*

**What he sees.** The `New Yedit` row is open. The titlebar switch reads
`Document | Terminal` with **Document selected**. The Yedit RAIL renders correctly
on the right (Markdown|Split|Text slider, the FILES list). The main viewport shows
**the bash prompt**, with `yedit: document surface opened` printed twice — he ran
the command again trying to get his editor back.

### The measurements, live on guihost at 3.0.48

| probe | answer |
|---|---|
| yedit daemon | alive, `GET /ping` → `{"ok":true,"app_name":"Yedit","document_version":"53:false"}`, with and without a query string |
| the declare | `terminal_runtime app_declare_ingested … verb:"sidebar"` fires on EVERY `yedit` run (14:58:20, 14:58:51, 15:17:25) |
| the contribution | ALIVE — the rail is on screen, and the right-rail element measures 278×1200 |
| `[data-document-surface]` | ⛔ **absent from the DOM entirely** |
| the terminal host's `data-document-surface-owns-viewport` | ⛔ **`"true"`** |
| `split_view` | `{"active_group_id": null, "groups": []}` — no split, so the split branch is not eating it |
| `server app state` `active_session_path` | the yedit row |

### ⭐ THE CONTRADICTION IS THE BUG, AND IT IS ONE LINE APART

Two derivations of *"is the document surface showing"*, both documented as sharing
one owner, disagreeing at the same instant on the same session:

- `data-document-surface-owns-viewport` (`shell.rs` ~91339) reads the LIVE state —
  `state.read().document_surface_visible_for(&host_session_path)` → **true**. It is
  true enough that the terminal host is put on `pointer-events:none` because of it,
  which is why the viewport also feels dead to the mouse.
- The mount gate (`shell.rs` ~88519) reads the SNAPSHOT —
  `snapshot.document_surfaces.get(path).is_some_and(|s| s.pane.visible)` → **false**,
  so `DocumentSurfaceBody` never mounts.

`toast_anchor` (`shell.rs` ~17095) reads the snapshot too and independently agrees
it is false. So the snapshot's `document_surfaces` map genuinely lacks a visible
entry for a session whose live derivation says it has one. Both are built from
`document_surface_visible_for`; the snapshot builds its map only for `co_visible`
= the active session + split members (`shell.rs` ~18615). **Start there: something
makes the snapshot skip the row it is actively rendering.**

⇒ The user-visible shape of the defect: the chrome, the pointer policy and the
titlebar all behave as if the document surface owns the viewport, and no document
surface exists. The terminal is not "showing through" — it is what was always
there, now unclickable.

### ⛔ THREE REMEDIES TRIED, ALL FALSIFIED — do not re-derive them
- **`yedit --close` then `yedit`** (a fresh declare from a clean slate): no change.
- **Switching to another row and back** (forces a fresh snapshot, so this rules out
  simple snapshot staleness): no change.
- **Re-running `yedit`** repeatedly: the daemon ingests each declare; the GUI never
  creates a second contribution because it still holds the first.

### ⚠ WHAT I COULD NOT SEPARATE, AND SAY SO
Three GUI restarts (3.0.46 → 3.0.47 → 3.0.48) happened before he reported this, so
**this session may have caused it**. Against that: the rail survives those restarts
and rebuilds correctly, and nothing in the three changes touches the document
surface. Not settled either way — reproduce on a GUI that has not been bumped
before believing either story.

### ⚠ AND AN INSTRUMENT GAP THAT COST TIME HERE
`server app state` exposes **no** `sidebar_contributions` and **no**
`document_surfaces`, so the two disagreeing derivations can only be compared by
dom-eval on a `data-` attribute plus inference. Both maps belong in the state dump;
without them, this class is diagnosed by guessing selectors. ⚠ And `data-app-pane-id`
does NOT exist — the real hooks are `data-app-pane-toolbar` / `-tab` / `-input`; a
probe built on the invented name reports an empty rail over a rail that is on screen.

### ⛔ STILL OPEN AT 3.0.148 — RE-REPORTED BY THE OPERATOR, 100 VERSIONS LATER, AND THE SHAPE HAS CHANGED

Re-reported 2026-08-13 in his own words: *"sidebars are broken once I launch yedit. yedit is badly
broken and does nothing."* ⇒ **This entry did not decay into "probably fixed by a later version";
it is live at 3.0.148.** Recording that explicitly, because an entry sitting a hundred versions
behind its last measurement invites exactly that assumption.

**The owner's terminal, read from the daemon's own screen — run three times:**

```
user@host:~$ yedit
yedit: document surface opened — `yedit --close` to close it.      (×3)
```

⚠ **The measurement has MOVED since the 3.0.48 table above, and the new reading is worse:**

| probe | 3.0.48 (original) | **3.0.148 (now)** |
|---|---|---|
| the declare / contribution | declare fires, **rail ALIVE and on screen**, 278×1200 | ⛔ **no rail — the right pane is blank** |
| `server app state` `active_surface_requests` | (not captured) | ⛔ **`[]`** |
| `document_surface` records in the daemon snapshot | (not captured) | ⛔ **zero** |

⇒ At 3.0.48 the contribution existed and only the BODY failed to mount. **At 3.0.148 the GUI holds
no surface record at all**, while `yedit` still reports `document surface opened` each time. ⚠ Whether
that is the same defect progressing or a second one layered on it is **not established** — it is a
different reading of the same probe family, taken on a host that had just been through a daemon
handover, and that confound is real.

⭐ **A REMEDY THAT WORKED, where the three above are recorded as falsified — but note what it fixed.**
His main viewport was painting **nothing** (a different row was active, not the yedit row). One
`server app open <the already-active row> --view terminal` restored it fully, verified by faithful
screenshot. ⚠ **That clears a PAINT STALL; it is untested against the document surface itself**, and
the yedit row was deliberately not opened because doing so takes over the operator's viewport. ⇒
Worth trying as a fourth remedy, and worth NOT recording as one until someone runs it on the yedit
row.

⛔ **AND A SCOPING DEFECT THAT MAKES THIS WORSE TO LIVE WITH:** `yedit --close` closes *"this
terminal session's"* surface. So phantom surfaces can only be cleared **from the exact terminal that
opened them** — an agent cannot clear them for the operator from any other shell, and typing into a
terminal a human is sitting at splices into whatever they are mid-way through. **A surface an agent
can open but only the person at that one terminal can close.**

## ⭐ OPERATOR-REPORTED, LIVES IN ychrome: a vault CARD item is unreadable and uneditable in the sidebar

**Status:** OPEN

⚖ **The fix belongs in `ychrome`**, filed there with the measurements:
`~/gh/ychrome/docs/pending-bugs.md` § *A CARD ITEM IS UNREADABLE AND UNEDITABLE IN THE SIDEBAR*.
Listed here only so the yggterm dev agent sees it, because the operator meets it through **this**
GUI's sidebar and will report it against yggterm.

the requirement, 2026-08-07, while paying an India Post booking by card:
> *"ychrome-vault edit/or see details of card is broken and no detail other than note can be seen
> in the GUI sidebar."*

Two halves, neither of which is a secrets-policy question:
1. **`view_tab_widgets` renders no card section at all** — brand, cardholder, expiry and **last4**
   are invisible, though `ychrome-vault card <name>` already answers all five secret-free. With two
   IDFC WOW cards in the vault (his and his sister's), **last4 is the only thing that tells them
   apart**, so the pane withholds the one field that prevents charging the wrong person.
2. **`ychrome-vault edit` has no card options whatsoever** — only rename/user/uri/totp/notes/
   custom-field — so a card item can never be updated by this client, and every card expires.

Context: `~/data/widgets/graph/notes/indiapost-rti-a-booking-run-2026-08-07.md`.
⚠ Do not fix half of it: seeing an expiry you cannot change moves the dead end one screen later.


## ★★★ A STALE INSTALL STATE `exec`ed EVERY CLI VERB INTO A 2.11.0 BINARY — and `--version` could not see it

**Status:** FIXED IN CODE — LIVE PROOF OWED

**The observation owed:** on guihost carrying 3.0.44+, `~/.yggterm/bin/yggterm-headless
server app terminal new --kind shell --ephemeral --title x` must answer
`Error: --ephemeral needs a rule this daemon can honestly check…` **without**
`YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF=1` in the environment. Today it creates a row.

Found 2026-08-07 while creating an ordinary hygiene-compliant probe row. Three
flags `docs/agent-row-hygiene.md` REQUIRES of every agent-created row came back
dead in one reply:

```
server app terminal new --kind shell --no-activate --purpose "…" \
    --ephemeral --ephemeral-idle-ttl-secs 900 --title "6.probe: flagcheck"
→ {"activated": true, "purpose": null, …}          # and no `tenancy` key at all
```

`--purpose` null, `--no-activate` ignored (the probe row **took the user's
viewport**), `--ephemeral` neither honoured nor refused. So an agent following
the hygiene contract believes it made a detached, self-describing, self-reaping
row and actually made an attached, anonymous, permanent one.

**Root cause, and it is one line of state.**

```
guihost ~/.yggterm/install-state.json   "active_version":    "2.11.0"
                                     "active_executable": ~/.yggterm/versions/2.11.0/yggterm
```

`maybe_handoff_to_preferred_headless_executable` `exec`s every non-builtin verb
into the sibling of that path — a **2.11.0 binary from 2026-07-22**. Every deploy
since has been a direct copy into `~/.local/bin` + `~/.yggterm/bin`, and **that
path writes no install state**, so the file is sixteen minors stale and wins.
The handoff compared PATHS and never versions, so nothing noticed.

**Discriminator that settles it in one command** (bare `--ephemeral` is refused
by name by the current parser and unknown to 2.11.0):

```
ssh guihost '~/.yggterm/bin/yggterm-headless server app terminal new --kind shell --ephemeral --title x'
  → creates a row                                    # ran 2.11.0
ssh guihost 'YGGTERM_SKIP_ACTIVE_EXEC_HANDOFF=1 …same…'
  → Error: --ephemeral needs a rule …                # ran 3.0.43
```

Same binary by md5 on dev refuses correctly, so this is host state, not a build.

⚠ **THE BLAST RADIUS IS NARROW AND THAT IS WHY IT SURVIVED.**
`find_direct_install_state` walks the EXECUTABLE's ancestors, so only a binary
living under `~/.yggterm/` ever finds that state file:

| invocation | diverted? |
|---|---|
| guihost `~/.yggterm/bin/yggterm-headless` | **yes → 2.11.0** |
| guihost `~/.local/bin/yggterm-headless` (what the daemons run) | no — genuinely 3.0.43 |
| dev / oc, any path | no — neither host has an `install-state.json` |

So the daemons were never wrong, and every `~/.local/bin/…` recipe in
`.agents/skills/yggui-app-control/SKILL.md` was fine. The poisoned intersection
is exactly *the `~/.yggterm/bin` binary, on the GUI host* — which is what an
agent is told to prefer when the PATH copy on dev/oc lags, and the only host
where live proof is taken.

⛔ **AND THE INSTRUMENT IS BLIND TO IT BY CONSTRUCTION.** `--version` is a *pure
builtin*, exempted from the handoff (`builtin_cli_command_is_pure`), so it
reports the binary you TYPED while every other verb runs a different one. The
session-start `fleet-daemon-audit` hook reads `--version` and called the host
current. ⇒ **on this fleet, "I checked the version" is not evidence that the
code you are testing is the code that ran.** Prove a build by a BEHAVIOURAL
discriminator — a flag or refusal that exists only in the build you mean.

**Fixed** by `yggterm_core::handoff_target_is_not_a_downgrade`, read by **three**
sites: both binaries' exec-handoff, and `server app launch`'s GUI resolution. A
handoff exists to route an OLD invocation into the ACTIVE newer install, never a
new one into an older binary; an unparseable version refuses rather than guesses.

⚠ **The third site was found by reaching for the verb, not by reading.**
`preferred_gui_executable_from_headless` prefers the recorded executable, and
`~/.yggterm/versions/2.11.0/yggterm` EXISTS on guihost — so an agent relaunching the
GUI through app control would have put a **2.11.0 window in front of a 3.0.44
daemon**. Nothing in the first two fixes would have caught it: same stale fact,
third consumer. If a fourth consumer of `preferred_executable` appears, it needs
the same guard.

### ⛔⛔ THE VERSION-ONLY GUARD WAS DEFEATED WITHIN THE HOUR — the record LIES, it is not merely stale

The first fix compared `CARGO_PKG_VERSION` against the record's
`active_version`. Deployed, live-proven, and then broken by the very next GUI
restart. Measured on guihost at 01:55:

```text
"active_version":    "3.0.44"                                    <- bumped
"active_executable": "/home/user/.yggterm/versions/2.11.0/yggterm" <- NOT moved
```

The GUI's daemon had already re-exec'd into `versions/2.11.0/yggterm-headless`
(the original bug), and the hot-update then promoted `expected_version` 3.0.44
against **that** path — `promote_direct_install_active_version` writes both
fields from its `target_executable`, so a handoff that landed on the old path
stamps the new version onto it. ⇒ **the corruption is self-perpetuating**: every
promote re-bumps the version and leaves the path, and a guard reading the
version waves it straight through forever.

⇒ **Trust the PATH, which cannot be bumped without a move.** The managed layout
is `versions/<v>/<binary>`, so the directory name is the layout's own statement
about what lives there. `handoff_target_is_usable` refuses when the target's
declared version is older than ours, and refuses outright when the record's
version and its own path disagree — a self-contradictory record is unusable, and
there is no way to know which half is true.

⚠ **Live mitigation applied to guihost** (`install-state.json.bak-2026-08-07` holds
the corrupt copy): `active_executable` repointed to `~/.yggterm/bin/yggterm`, the
GUI that is really running. Verified: `~/.yggterm/bin/yggterm-headless server app
terminal new --kind shell --ephemeral` refuses by name again, with no skip env.

⏳ **Still open underneath, and it is the real single-source-of-truth defect:**
`install-state.json` claims to answer "which executable is active" while the
deploy path everyone uses never writes it — and the hot-update path writes it
from wherever the handoff happened to land. Either the direct-deploy path must
write it, or the file must stop claiming to answer that question. ⚠ Until one of
those, `InstallContext::current_version` also still reports the RECORDED version
rather than `CARGO_PKG_VERSION` — a second lie in the same struct, and anything
reading it for "what am I" is wrong.

⚠ **`promote_direct_install_active_version` deserves its own look**: it will
happily write a `(new version, old path)` pair, which is the corruption itself.
It should refuse to promote a version onto a path that declares a different one.

### ⚠ FOUND BY THE USER'S EYES: the Settings panel is a FIFTH consumer, and it offered a DOWNGRADE

Reported 2026-08-07 from the screen — *"GUI version shows 2.11.0"* — while the
process rendering that panel was 3.0.44, under a button reading **"Restart now
to update · The updated build is installed and waiting to replace this
process."**

Two separate defects, both fixed:

1. **The displayed version was the RECORD, not the build.** `InstallUpdateRow`
   read `snapshot.install_context.current_version`. Meanwhile
   `daemon_update_state.current_gui_version` had been correct all along because
   it uses `current_version()` — **two encodings of "what version am I", one
   right and one wrong, rendered in the same window.**
2. ⛔ **`pending_restart_from_active_install_state` compared PATHS ONLY** — no
   version check at all — so it offered a restart into
   `versions/2.11.0/yggterm`. **A downgrade wearing an update's label, on the
   one control the user presses.** Now guarded by `handoff_target_is_usable`.

⚖ **This is the lesson of the whole lane.** Five consumers read one dishonest
record — two exec handoffs, `server app launch`, the version display, and the
update button — and each was found by a different accident: a probe, a verb I
reached for, and finally the user looking at the screen. **When a fact is wrong,
enumerate its readers**; fixing the ones you tripped over is not fixing the
fact. ⚠ A sixth reader would still be wrong today: `InstallContext::current_version`
still reports the record rather than `CARGO_PKG_VERSION`, so the struct itself
remains the trap.

## ⛔⛔ THE DEPLOY WRITES THREE COPIES AND MISSES THE ONE ON `PATH` — and it faked a second bug

**Status:** OPEN

Reported by the orchestrator 2026-08-07, **re-measured here the same hour** and worse than
reported: it is on TWO hosts, not one.

```
dev   ~/.local/bin/yggterm            3.0.40   <- what `which yggterm` resolves
      ~/.local/bin/yggterm-headless   3.0.44
      ~/.yggterm/bin/yggterm          3.0.44
      ~/.yggterm/bin/yggterm-headless 3.0.44
oc    ~/.local/bin/yggterm            3.0.40   <- BOTH ~/.local copies stale
      ~/.local/bin/yggterm-headless   3.0.40
```

Every delegate that types `yggterm` on dev runs **3.0.40** and binds a 3.0.40 socket among the
daemons there. ⚖ **This is the mirror image of `edcc4927`** ("the refresh was updating a binary no
session ever runs") — it now updates the binaries nothing *types* while missing the one it does.

### ⭐ AND IT WAS WEARING A SECOND BUG'S CLOTHES — that half is CLOSED, do not build for it

A delegate reported *"cannot reach the yggterm GUI, version says 3.0.40"* and the version was
filed as the cause. It was not, and the proposed fix — *"`server app` verbs on a host with zero
GUI clients should refuse BY NAME"* — **already shipped in 3.0.44** (`317755f8`). Measured here on
dev, both binaries present, same host, same minute:

```
3.0.40  → Error: no live Yggterm GUI client is registered
3.0.44  → Error: no live Yggterm GUI client is registered for app control on this host (dev).
          App control is served by the yggterm GUI PROCESS, not by the daemon … run this verb
          on the GUI host … `server app clients` answers "is the GUI here?" directly.
```

⇒ **The delegate saw the pre-3.0.44 sentence because it was running the pre-3.0.44 binary.** The
two findings are one finding: **fixing the split install is what delivers the refusal fleet-wide.**
⚠ Generalise the lesson rather than the fix — *a stale binary does not report itself as stale; it
reports the WORLD as broken, in the vocabulary of whatever it was doing.* Any bug report that
quotes an error string must be re-read on a binary proven current before the string is believed.

**Shipped alongside:** the refusal now names candidate GUI hosts from the daemon's own ssh-target
list instead of the placeholder `<gui-host>` (no ssh probing — a refusal that hangs is worse than
a vague one), and the session-start `fleet-daemon-audit` hook was rewritten to **report
DISAGREEMENT rather than a version**: it enumerates both binary names in both directories plus
whatever `PATH` resolves, and leads with `⛔ <host> SPLIT INSTALL`. The old hook read
`yggterm-headless` at two paths and **never read `yggterm` at all**, which is why it printed "all
audited hosts on 3.0.44" over this. ⚠ While rewriting it, a second silent no-op surfaced: its
running-daemon probe read `SERVER_VERSION` from `/proc/<pid>/environ`, **a variable that is not in
a daemon's environment**, so that line had never printed once. It now groups daemons by
`/proc/<pid>/exe`.

⏳ **The actual fix is still open: the deploy path must write every copy, or stop writing some.**
Until then `~/.local/bin/yggterm` on dev and both `~/.local` copies on oc are stale.

### ⚠ 30 DAEMONS LIVE ON dev, TWO OF THEM FROM A BUILD TREE

Confirmed by the rewritten audit: `2x ~/gh/yggterm/target/release/yggterm-headless`,
`27x ~/.yggterm/bin/yggterm-headless`, all marked `(deleted)`. **A daemon running out of
`target/release` can never be updated by any deploy**, because no deploy writes there. Same lane as
the never-retire entry below (fd-handoff step 3).

## ⛔⛔ AN AGENT ROW STOPS CONSUMING INPUT AFTER A TURN ENDS — alive, idle-looking, and DEAF

**Status:** OPEN

⚖ **This outranks every other row-management item in this file**, on the decision and on the
arithmetic: a wedged row **silently drops every instruction sent to it**, so any orchestration
built on the row plane is unreliable in a way nothing reports.

**TWICE in one night, 2026-08-07**, on two different rows, owner-observed both times:

| row | symptom | cleared by |
|---|---|---|
| levers row 4 | ~30 min frozen | — |
| yggterm row 6 (this session) | alive, turn ended, not reading input | `server terminal restart` |

**What is true of the wedged row, and it is the hard part:** the process is ALIVE, the turn has
ENDED, and the row looks IDLE — which is indistinguishable from a healthy row waiting for work.
Every OS-level signal says fine. Same family as
[[finding-agent-session-liveness-is-invisible-to-os-signals]].

**The instrument table, measured:**

| verb | on a wedged row | verdict |
|---|---|---|
| `terminal send` | `error: null` | ⛔ **LIES** — reports success, delivers nothing |
| `terminal submit` | names it (no echo-confirm within the deadline) | ✅ the only honest one |
| `server terminal restart` | clears the wedge, transcript intact | ✅ the remedy |

⇒ **`submit`'s echo-confirm is the ONLY thing that can see this**, which is the strongest argument
yet for the entry below: `send`'s `error: null` is not merely uninformative here, it actively
conceals a dropped instruction. An orchestrator that sends with `send` and believes the reply has
no way to learn that its delegate never heard it.

**What to build, in order:**

1. ✅ **A WEDGED-vs-IDLE discriminator — SHIPPED 3.0.48 as `server app terminal input-check`**,
   live-proven on guihost against a deliberately frozen CLI (`kill -STOP` on the remote `claude`:
   alive, composer displayed, consuming nothing — the wedge's exact signature, reproducible on
   demand for the first time). All four verdicts measured:

   | row state | answer |
   |---|---|
   | healthy Claude Code row | `consuming_input:true` in **248 ms** |
   | `kill -STOP`ped CLI | `wedged:true` + the named remedy |
   | plain shell row | `composer_shown:false` — *unanswerable*, explicitly NOT wedged |
   | composer holding typed text | refuses by name rather than probing |

   ⚖ **`wedged` is a POSITIVE claim, deliberately**: composer displayed AND no echo AND no draft.
   A busy row mid-output answers `composer_shown:false` instead of `wedged:true`, because the
   false positive is what would justify a reaper killing live work (item 3).
   ⭐ And the contrast was captured in one sitting, on one frozen row, seconds apart:
   `input-check` said `wedged:true` while `send` said `error:null, accepted:true, bytes:14`.
   The instrument table below is no longer anecdote.

   ⛔ **Two live corrections it cost, both worth carrying:**
   - **The draft guard must read the SGR, not the text.** The probe types a marker and clears with
     Ctrl+U, so it must refuse on an unsent draft — but a blunt "any text after the glyph is a
     draft" was measured WRONG on two of three real composers: Claude Code draws its placeholder
     (`Try "write a test for shell.rs"`) and the ghost of the last sent message BOTH as text after
     the glyph, `ESC[2m` faint. The blunt rule refused every real row, i.e. it made the wedge
     undetectable in exactly the case the guard exists to protect. Faint = the CLI's chrome;
     normal intensity = the human's words. Locked with the verbatim raw lines.
   - **It cannot see a row owned by an OLDER daemon** — the snapshot goes to the GUI's endpoint,
     and a row whose PTY lives on a preserved predecessor answers `composer_shown:false`.
     [[finding-daemon-side-fix-inert-under-proxy]], fifth sighting. Not fixed; say so when reading
     an "unanswerable" verdict.

   ⏳ **Still owed from this item:** the verdict is on-demand only and is NOT cached as row state,
   so the sidebar still cannot show a wedge and neither can the working-indicator
   ([[spec-title-summary-working-indicator]]) or the three-tools-three-answers entry below. The
   probe is intrusive (it types), so a background poll is NOT the way to get there — cache the
   last verdict on the session and carry it on the snapshot instead.
2. **`send` must not report success without delivery** (see below).
3. **An automatic remedy, gated carefully.** `server terminal restart` clears it, but ⛔ a reaper
   that restarts a row it wrongly believes wedged would destroy live work — so this needs the
   positive-signal discipline of §THE QUIET-GATE LAW, never an absence-of-output timer.

⚠ **Not yet root-caused, and do not assume the CLI.** Both wedges followed a turn ENDING, which is
when the CLI returns to its read loop — so the suspect set includes our PTY read path and the
handoff after a turn, not only the agent CLI. **The cheap first probe:** on a wedged row, compare
the daemon's vt100 screen (`server terminal screen`) against what a `send` writes — if the bytes
reach the PTY and the screen never changes, the CLI stopped reading; if the bytes never reach the
PTY, it is ours.

## ⭐⭐ `terminal send` IS SILENTLY LOSSY AND `terminal submit` IS NOT — change the runbook

**Status:** OPEN

Found by row 5.1 with a controlled pair, then paid for by the orchestrator on itself: **two long
findings sent to row 6 with the two-write pattern returned `error: null` and never landed**, and
were reported to the owner as sent. `terminal submit` to the same row answered
`submitted: true, waited_ms: 308` and arrived.

⇒ **`error: null` on `send` is not delivery. `submitted: true` is.**

⚖ **This is the SEVENTH verb in the report-the-request-not-the-effect family, and `submit` is the
first one that got it right** — so it is the reference implementation, not just the workaround.
What it does that the other six do not: it **waits for the session to echo-confirm it is consuming
input**, it carries a **deadline**, and on failure it reports a **named reason** rather than a
falsy field (*"session never echo-confirmed it was consuming input, prompt may be displayed but
codex not yet reading"*, `submitted:false` after 30,084 ms). Positive signal, bounded wait, named
refusal — the three properties §THE QUIET-GATE LAW asks for.

**The fix is not to document `send` away.** Either `send` grows the same confirmation, or it
refuses by name on an agent-CLI row and points at `submit`. A verb whose success field means
"bytes were written somewhere" is a trap for every future caller.

⚠ **`submit` does not exist in the 3.0.40 CLI**, so the runbook fix cannot land fleet-wide until
the split install above is fixed. The two entries are sequenced, not independent.

## ⚠ `ygg-unwedge` CANNOT FIND THE GUI ON THE HOST THE GUI RUNS ON

**Status:** OPEN

Reported by row 5.1, untouched here. On **guihost**, `ygg-unwedge` answers *"no yggterm GUI supervisor
running"* — with and without `DISPLAY=:1` — while `server app clients` on that same host lists a
live client on display `:1`. **The remedy tool is blind on the one machine it exists to remedy.**

✅ **ROOT-CAUSED 2026-08-13 at 3.0.139, and it is two defects in one line.** The
tool opens with `sup=$(pgrep -f "yggterm --supervise" | head -1)` and exits on
empty.

1. ⛔ **NOTHING SUPERVISES THE GUI ANY MORE.** On the GUI host the process is
   `/home/…/yggterm` with **PPID 1** — `server app launch` detaches it and init
   adopts it, so no `--supervise` process exists to find. The tool's premise is
   stale, not its resolver. Verified in both directions: a clean run prints the
   filed symptom verbatim (*"no yggterm GUI supervisor running"*) while
   `server app clients` lists a live client on that same host.

2. ⛔⛔ **`pgrep -f <pattern>` MATCHES THE CALLER'S OWN COMMAND LINE, so the tool
   can "find" a supervisor that is the shell asking the question** — and then
   walks `pgrep -P` to an unrelated child and reports a confident verdict about
   it. Hit live while measuring this: a diagnostic run whose command line
   contained the pattern answered *"GUI <pid> has no edit socket; not the
   flush-gate freeze"* — a specific, plausible, entirely wrong diagnosis about a
   pid that was not the GUI. The clean run needs the pattern kept out of the
   caller's argv (build it at runtime, or invoke through a script file).

⇒ The fix is to find the GUI the way everything else does — the client-instances
record — not to repair the supervisor search. ⚠ And the `pgrep -f` half is
generic: any fleet tool that discovers a process this way can diagnose the shell
that asked.

**Falsifier:** on a host with a live GUI client, the tool names that GUI's pid.

## UNIT TESTS WRITE TRACE EVENTS INTO THE DEVELOPER'S REAL `~/.yggterm`

**Status:** OPEN

Found 2026-08-07 while root-causing the five notification tests that failed on a
clean `main`. Those failed because `ShellState::new` restored the developer's
REAL notification backlog — `resolve_yggterm_home()` falls back to `~/.yggterm`
when `YGGTERM_HOME` is unset, as it is under `cargo test`. That read is fixed at
the seam (`load_persisted_notifications` returns empty under `cfg!(test)`).

The WRITES are not fixed. `append_trace_event` resolves the same home from a
dozen call sites in `shell.rs`, so running the suite appends trace events to the
user's live `~/.yggterm/event-trace.*.jsonl`. It corrupts no test — nothing
asserts on those files — but it means the suite mutates the user's real state,
and a future test that DOES read a trace would fail the same drifting way.

**The fix is not another `cfg!(test)` gate**: it is to give the test binary an
isolated `YGGTERM_HOME`. Note `std::env::set_var` is unsafe under edition 2024
and the suite is multi-threaded, so a `Once` in a bootstrap helper races with
concurrent `resolve_yggterm_home()` readers — this needs a real answer, not a
quick one.

**Falsifier:** `ls -l --time-style=full-iso ~/.yggterm/event-trace.*.jsonl`
before and after `cargo test -p yggterm-shell`; if no mtime moves, this is wrong.

## ★★★ FIVE VERBS REPORT THE REQUEST, NOT THE EFFECT — one rule, not five patches

**Status:** AWAITING A DECISION

*(The shape of the fix is the decision: a response-layer rule, or five separate
patches. recorded framing, 2026-08-07.)*

Owner, watching the tool calls scroll past: *"for common tasks that we are
failing again and again like row org, we should have yggui automation supplied
tooling shortcuts for doing these common chores so that no mistakes do not
happen."* He is not asking for better agent discipline — he watched an agent
hand-assemble one chore out of five primitives, get it wrong, and retry.

**Every failure on the night of 2026-08-07 was an agent believing a success
field, and the owner finding the truth by looking at his screen:**

| verb | field | what it actually meant |
|---|---|---|
| `session remove` | `verified` | answered `false / remote_runtime_survived` with `live_processes: []` in the SAME reply, 3 of 3 |
| `session rename` | `accepted: true` | on a call the daemon had a named reason to reject |
| `server sessions reorder` | `changed: true` | a daemon field moved; the sidebar did not, 4 times |
| `terminal new --prompt-stdin` | `delivered: true` | `submitted:true, waited_ms:19802` — and the transcript never gained a user row, twice in one minute |
| `terminal send` | `accepted: true` | bytes written; into an agent row that is one Enter PER LINE |

✅ **TWO OF THE FIVE RE-TESTED AT 3.0.141 AND BOTH NOW REPORT THE EFFECT**
(2026-08-13 sweep, on a row created for the purpose and removed after):

| verb | reply | read-back |
|---|---|---|
| `session rename` | `accepted: true` | label reads `ygg-sweep-rename-probe` — the change is real **for a row that EXISTS**; see the rename entry below, where the same reply is returned for a path that does not |
| `session remove` | `verified: true`, `live_processes: []` | row absent from the census; 6th consecutive clean sample that day |

⚠ **THREE WERE NOT RE-TESTED, AND THE REASONS ARE PART OF THE RESULT.**
`sessions reorder` rewrites the WHOLE rendered order including rows owned by
other sessions, so it is not a probe to fire on a live sidebar;
`terminal new --prompt-stdin` and `terminal send` need a live agent row, and the
`terminal send` row is documented behaviour now (one Enter per line) rather than
a false success. **Do not read the two ✅ as four.**

⛔ **AND THE FIRST ATTEMPT AT THIS RE-TEST PRODUCED A FALSE RESULT, which is the
part worth carrying.** The read-back interpolated the session path through two
shells into a python comparison; it matched nothing, and reported
`<row not in census>` for a row that was plainly there — a check that FAILS OPEN
and reads exactly like a real finding. It was caught only by adding a positive
control: *can this comparison find a row I know exists?* ⇒ **A verification
harness needs its own positive control before its negatives mean anything**,
which is the same rule as the entries it is verifying, one level up.

### ⛔ THE ROW PLANE WAS UNREACHABLE FROM EVERY NON-GUI HOST, AND THE REFUSAL DID NOT SAY SO

Measured by a delegate, 2026-08-07: row 5.1 (on dev) tried to message row 5.2
and got `no live Yggterm GUI client is registered for app control`. It concluded
*"the row plane is still unreachable from here, so files remain the channel"*,
invented a `crossings/` directory, and passed a structured file to its sibling.

**Sensible improvisation, and a total feature loss.** App control is served by
the **GUI process**, not the daemon, so it only answers on the host where the GUI
runs — and every delegate on this fleet runs on dev while the GUI is on guihost. So
the cross-row messaging contract every delegate is briefed with was **impossible
for all of them**, and the refusal never mentioned the word "host".

⇒ **Nothing was broken except that nobody was told where to stand.** The refusal
now names the host it is on, states that app control follows the GUI process,
and points at `ssh <gui-host>` plus `server app clients` (which answers "is the
GUI here?" directly). Locked by
`the_no_client_refusal_names_the_host_problem_and_the_way_out`.

⏳ **Still open, and the better fix:** cross-row messaging should RESOLVE the GUI
host from any fleet host rather than refuse. The daemon already holds
`remote_machines` with ssh targets; what it does not record is which of them runs
a GUI. ⚠ Same family as the binary/daemon resolution item — **resolution fails
silently and the caller invents a workaround** — and the workaround is the
expensive part, because a file channel nobody designed becomes load-bearing.

### ⭐ A LAW THAT LIVES ONLY IN A MEMORY FILE IS RE-BROKEN BY EVERY SESSION THAT HAS NOT READ IT

Added 2026-08-07: row 1 (widgets) took a **fourth** cyber demotion
(`claude-fable-5` → `claude-opus-5`, `dir=retry`, unpaired with any 529) minutes
after being handed a portal errand. The standing law is that **that lobe launches
on Opus 5 ALWAYS, precisely because it demotes** — and the row was on Fable,
because nothing enforced the law at launch.

⇒ **`spawn-delegate` should refuse to be born on a model its lobe is known to
refuse**, reading a per-lobe model policy rather than trusting the launcher to
remember. This is the sharpest instance of the whole pattern: the cost of
"remember to pass `--model`" is paid once per session that forgets, forever, and
it is invisible until someone reads a demotion counter.

⚠ Design note before anyone builds it: the policy has to key on something the
launch verb can actually see. The lobe is not a yggterm concept — the row's
`--purpose`, its `cwd` (`~/data/<lobe>`), or an explicit `--lobe` are the
candidates, and `cwd` is the one that cannot be forgotten because the delegate
needs it anyway.

⇒ **The candidate rule: a mutating verb answers against RE-READ state, or it
answers with a named refusal.** Never against the request it just made. The
app-path reorder shipped in 3.0.44 is the first verb built this way — `changed`
is a before/after comparison of the rendered list — and it is the pattern to
copy or to generalise.

⚖ **And the reason the owner wants these as VERBS rather than agent habits:**
*"an agent's discipline resets every session, but a verb does not."* Anything an
agent must remember to do in the right order is a defect waiting for a tired
session. Same argument as the row-hygiene work: **the janitorial work must
become tooling.** The named dreams, each with a measured cost from that night:
`spawn-delegate` (one verb, delivery PROVEN not reported) · `outline --apply`
(must move the GUI) · a `rename` that cannot lose the self-title race ·
`send --file` · a row state distinguishing **WAITING-ON-INPUT from idle**
(a monitor stayed silent through the state that mattered) · an honest `reap` ·
binary/daemon resolution that never fails silently.


### ⛔ A SIXTH, MEASURED 2026-08-14: `server app session remove` CANNOT REPORT FAILURE

Against the fiction `local://deadbeef-dead-4bee-8dad-deadbeefdead` — a path that has
never existed — it answers:

```
row_still_listed: false   verified: true   error: null
```

**Removing nothing trivially satisfies "not listed".** The verb confirms its own
postcondition without ever checking that it acted, so *success* and *there was nothing
there* are the same answer.

⚠ **It has already cost a real reap:** a row was removed using a uuid whose suffix had
been fabricated from an 8-character display prefix; the call answered `verified: true`
and the row was still on screen. ⇒ **Confirm a reap by re-reading `server app rows`,
never by this verb's own answer** — and resolve a full uuid from the artefact
(`ls ~/.claude/projects/*/<prefix>*.jsonl`) rather than composing one from a listing.
## ⚠ AN EXPLICIT TITLE CAN STILL BE OVERWRITTEN ON A ROW WHOSE AGENT HAS EXITED

**Status:** OPEN

Measured by the orchestrator 2026-08-07: two rows renamed at 00:55 reverted
~2.5 h later to CLI-generated titles. ⭐ **The discriminator rules out the
obvious cause — the reverted row had ZERO live claude processes**, so this was
not the CLI re-titling itself; a generated title overwrote a human-set one on a
session with no agent running.

**Root cause, partly located.** `session_title_is_explicit` reads
`self.sessions` — the LIVE row. Once an agent exits, the only surviving copy is
the scanned mirror (`RemoteScannedSession`), and it carried **no provenance at
all**, exactly as the predecessor's own doc comment said (*"the scanned mirror
carries no provenance of its own"*). So every downstream guard was reduced to
`looks_like_generated_fallback_title` — the shape heuristic that provably cannot
separate `6. yggterm: campaign` from a real conversation title like
`Ping from orchestrator`.

**Shipped: the missing precondition.** `RemoteScannedSession::title_is_explicit`
(`#[serde(default)]`), stamped by `set_session_title_explicit`, and consumed by
the merge in `promote_remote_codex_live_session_to_scanned`.

⛔ **NOT PROVEN FIXED, and do not close it on this.** I did not reproduce the
observed revert, and the merge I guarded is a live→scanned promotion, not
necessarily the site that fired. The remaining work is to find which path
consumes the mirror on a machine rescan / state restore
(`load_remote_machine_sessions_from_mirror`, `overlay_mirrored_remote_sessions`
around `lib.rs:5628` are the unexamined candidates) and make it read the flag.
**Falsifier that would close it:** rename a `remote-cc://` row whose agent has
exited, force a machine rescan, and read the title back from `server app rows`.

## ★★★ A NEW ROW ALWAYS LANDS AT THE HEAD — ten front-insert sites, one missing owner

**Status:** FIXED IN CODE — LIVE PROOF OWED

*(The seating half shipped in 3.0.45. The sort verb, the `outline_prefix` setter and the collapse
buckets are still OPEN and listed at the bottom of this entry — one lane, one ticket.)*

✅ **LIVE-PROVEN on guihost 3.0.45, 2026-08-07 06:4x**, with two ephemeral probe rows created in the
order `6.9` then `6.8`:

```
seat A → {"honoured": true, "outline_prefix": "6.9", "live_index": 0}
seat B → {"outline_prefix": "6.8", "live_index": 1}
rendered → 0: 5.3 gadgets (unnumbered)   1: '6.8' probe B   2: '6.9' probe A
```

⇒ **the later row seated ABOVE the earlier one**, which is the whole feature, and the unnumbered
row above them was not disturbed. `outline_prefix` is present on `server app rows`. Both probes
removed afterwards, `verified: true`, `live_processes: []`.

⛔ **The proof also caught two defects of its own, both now fixed and locked:**

1. **`seat.honoured` was a FALSE NEGATIVE.** It required that no row above sort after this one —
   counting UNNUMBERED rows, which sort last — so a correctly seated `6.8` reported
   `honoured: false` merely because an unnumbered row sat above it. That contradicted
   `outline_seat_for`, which deliberately does not move unnumbered rows out of a numbered row's
   way. ⚖ **A verb whose success field disagrees with the behaviour it reports on is this lane's
   own defect, committed by this lane** — the predicate must model the rule, not a stricter one.
2. ⛔ **THE NUMBER WAS INVISIBLE IN THE SIDEBAR.** Probe A rendered as *"Local Shell Script
   Debugging"* — no number — one minute after creation, while still carrying
   `outline_prefix: "6.9"`. `compose_outline_prefix` runs when the row is BUILT, and
   `enrich_sidebar_rows_with_live_titles` then overwrites `label` with the generated title. **So
   the outline decayed on exactly the event it was built to survive: a CLI re-titling itself.**
   Fixed with one re-compose pass at the END of enrichment (the last writer of the label) rather
   than a call at each of its branches, because a future branch would drop it again silently.
   Locked by `enrichment_re_composes_the_outline_number_it_would_otherwise_overwrite`.

⚠ **Two instrument notes for the next session**, both of which cost time here: the app-control
default timeout is **15 s and a create needs more** — `--timeout-ms 90000` is the difference
between a clean `seat` reply and `Error: timed out …` over a row that was in fact created; and
piping `ssh … 2>&1` into a JSON parser corrupts the payload with stderr, so use `2>/dev/null`.

recorded build order 2026-08-07: *"sorts are cheap and never break, and spawnee sessions are
grouped as a collapseable tree with the session as the parent element."* Hardened the same night
after he dragged a mis-seated row back into place by hand at 05:30: *"From now on, we need to spawn
session at the exact row we want. The second 6 session looked odd. I manually dragged it."*

**Reproduced under measurement**, guihost 3.0.44, one ephemeral spawn:

```
BEFORE: 0  1  1.1  2  4  5.1  5.2  6  7.1  7.2
AFTER : ZZ 0  1    1.1 2  4    5.1  5.2  6   7.1        <- new row at the HEAD
```

⇒ **`grep -n "live_session_order.insert(0" crates/yggterm-server/src/lib.rs` returned TEN sites.**
Every birth path independently answered "where does a new row go?" with "the front". That is the
single-source-of-truth defect stated plainly: ten copies of one decision, and the outline had no
say in any of them.

**SHIPPED — `seat_new_live_session` is the one owner**, called by all ten former sites. It seats a
row by its `outline_prefix` and falls back to the front when the row has no number, so **behaviour
is unchanged on any sidebar that has not adopted the outline** — the safest possible rollout under
live agent work. The sort key
(`yggterm_core::session_outline`) compares dotted segments as **integers**; the ten-lobe trap
(`"10" < "2"` is correct string order and wrong outline order) is locked by
`a_tenth_lobe_does_not_sort_before_the_second`.

⚖ **SEATING IS A PROPERTY OF CREATION**, per the owner: a create-then-reorder sequence is refused
as a design even when both halves work, because the wrong order is on screen in between and the
second half is the half that fails. So `terminal new` grew `--outline <prefix>` and
`--insert-after <path>`, carried in the create request itself (`RowSeatRequest`) and applied by the
daemon before it answers. The reply's `seat` block is **RE-READ from the order the GUI holds**,
never composed from the request.

⛔ **Every refusal is named.** A prefix the sort cannot read, an anchor that is not a row, and
`--outline` + `--insert-after` together are all refused BY NAME — the last one *before* the row is
created, so nothing is ever half-placed.

**ALSO SHIPPED in 3.0.45, the rest of the numbering surface:**

- `server app session outline <path> <prefix>` — number a row that already exists (empty clears
  it), which **re-seats it** rather than labelling it in place. Until this, a row could only get
  its number at birth.
- `outline_prefix` is now on `server app rows`, beside `session_id`. It was stored and survived a
  restart but was invisible, so the number lived only inside the title string.
- `server app sessions sort [--dry-run]` — the owner's shortcut, re-deriving the Live order from
  the rows' numbers. Idempotent: an already-sorted list reports `changed: false`, which is the
  success case, not a no-op to chase.

⚠ **A small render defect found while reading the compose rule, not yet reproduced.**
`compose_outline_prefix` suppresses the prefix when `label.trim_start().starts_with(prefix)` —
correct and necessary for idempotence (a row is relabelled on every snapshot, and `2. cogs: 2.
cogs:` was the bug it closes), but it is a PREFIX match on a bare string. So a row numbered `2`
whose CLI titles it *"2026 audit"* renders with no visible number while carrying one, and a row
numbered `1` titled *"12 things"* does the same. The fix is to require a separator after the match
(`2.`/`2 `), not to drop the idempotence guard.

⏳ **Still open in this lane:**

1. **The collapse buckets.** The tree vocabulary already exists: `server app rows` returns `depth`,
   `child_count`, `expanded` and `group_kind`, the Live Sessions group already collapses, and
   **every live row is `depth: 1`** — that flatness is the entire gap. A collapsed bucket also
   already has its work-aggregation signal, `busy_reason: "group_descendant_working"`, so work
   cannot hide inside one. ⚠ Two durability bars: **collapse state must survive a GUI restart**
   (the same bar `outline_prefix` needed, which was persisted-then-dropped on restore), and a
   collapsed parent must stay clickable through to its child — the constitution's *"click it and
   co-browse it"* applies to a nested row too.

⭐ **THE DRAG WAS THE PROVEN WRITE PATH, AND THE VERBS NOW TRAVEL IT.** Owner, 2026-08-07: manual
drag is the only row-ordering affordance that worked end to end while `server reorder` needed a GUI
restart and `server app sessions reorder` was a no-op. Reading `queue_drop_current_drag_target`
(`shell.rs`) showed why — the drag takes **three steps and the verb took one**:

1. an optimistic `replace_live_session_order` on the GUI's OWN copy, so the sidebar moves
   immediately;
2. `reorder_live_sessions_scoped(endpoint, paths, Some(gui_row_order_scope()))` — **scoped to this
   GUI's ledger**;
3. `apply_snapshot` of what the daemon returns.

The app-control verb called the **UNSCOPED** `reorder_live_sessions` and deliberately skipped step
1. ⇒ `row_order_ledger`'s `reconcile_order_with_remembered` restores this GUI's remembered
arrangement on the next rebuild and **reverts anything written outside that scope** — a far cheaper
explanation than the peer-aggregation hypothesis below, and it explains the restart-only visibility
exactly. **Shipped:** `apply_live_order_the_way_a_drag_does` is now the one route, used by both
`sessions reorder` and the new `sessions sort`. ⚠ **Live proof owed** — this is a hypothesis with a
mechanism, not yet a measurement; the falsifier is that the verb still fails to move the sidebar on
guihost after 3.0.45.

⛔ **AND A CORRECTION, because two of us have now acted on it.** It was reported that a single
spawn *re-scrambles the whole live order* (`0 1 2 4 5.1` → `5.1 4 2 1 0`). **The reproduction above
falsifies that**: every unrelated row kept its relative position. The observed reversal is almost
certainly a **reversed-order control applied by the orchestrator at ~03:30** while testing the
reorder verb, rendered later at a GUI restart — the reversal matches that input exactly, and only a
restart renders a stored order. ⇒ **the insertion path prepended; it did not rewrite.** Fixing the
wrong one of those would have cost a day.

⚖ **This is one lane with the parentage entry below, not two tickets.** `parent_session_path` gives
placement at spawn, the sort key, the collapse bucket and sweep-my-children — four features, one
field.

## ★★★ `server reorder` WRITES WHERE THE GUI DOES NOT READ — same version, different DAEMON

**Status:** OPEN

Owner, twice: *"the rows are not placed in the correct order in the Live session
area. This error needs to be corrected for future too."* Root-caused by the
orchestrator (four reorder round-trips, all answering `changed:true`), corrected
here on one measurement.

**The orchestrator's falsification was measured on the wrong pair.** They ruled
out [[finding-version-string-as-rendezvous-key]] because both binaries answer
3.0.44 — which is true and which is not the question:

```
GUI  3.0.44  ->  daemon 3417351 @ 3.0.41     <- what the user sees
CLI  3.0.44  ->  daemon 3755280 @ 3.0.44, owns 0 sessions
```

⇒ **Two binaries of the SAME version resolved to two DIFFERENT daemons.** The
CLI's reorder lands on a daemon that owns nothing and that the GUI never reads,
so `changed:true` is honest about a field nobody renders. Same rendezvous defect,
one level down: **binary version parity does not imply daemon parity, and the
version is not the thing to compare — the resolved `server_pid` is.**

**The rendered order is the GUI's own in-process list, and `server app rows`
reports it faithfully.** Measured against a faithful screenshot with the two
copies deliberately divergent: sidebar and `server app rows` both read
`5.1, 5.2, 0., 1., 2., 4., 6.` (new rows PREPENDED at the head) while the daemon
held `0., 7.1, 7.2, 1., 2., 5.1, 5.2`. ⚠ So `server app rows` is NOT a second
encoding to distrust here — it is the accurate report, and `server snapshot` is
the misleading one, because the snapshot answers from whichever daemon the CLI
resolved.

⚠ **A running GUI CAN pick up a daemon reorder — when it is bound to the same
daemon.** At 02:47 a `server sessions reorder` moved the sidebar with no restart
(screenshot-proven), because GUI and CLI were both on the 3.0.44 daemon then. A
later GUI restart bound it to the 3.0.41 daemon, and every reorder since has been
invisible. **That intermittency is the whole "version dance" the owner reported**,
and it is why it looks like a deploy-window behaviour: a deploy restarts the GUI,
which re-reads its daemon's order at startup.

**Half shipped, and the remaining half is now isolated.**
`server app sessions reorder <order.json>` exists (3.0.44): it forwards to the
daemon **this GUI is bound to**, so "which daemon" stops being a guess, and it
reports `changed` by comparing the RENDERED row list before and after rather
than by parsing the daemon's reply.

⛔ **But the sidebar still does not move, and that isolates the real defect: a
RUNNING GUI never re-reads the daemon's live order.** Measured 2026-08-07 with
the forwarding verb, on the GUI's own daemon, no restart: daemon updated, sidebar
unchanged. A GUI **restart** does adopt it (observed immediately after — the
sidebar came up in the orchestrator's outline).

⛔ **AND THE "GUI REFUSES TO ADOPT" THEORY IS FALSIFIED — read this before
building an epoch.** The obvious fix is a generation counter so the GUI adopts
"when the DAEMON's order changed". **The code already adopts unconditionally**:

- `apply_snapshot` (`lib.rs:5295`) sets `self.live_session_order` from
  `snapshot.live_sessions` on **every** snapshot — no guard, no epoch.
- `merge_hot_sidebar_sessions` (`shell.rs:39888`) is a pass-through
  (`live_sessions.to_vec()`), and the Live region renders from that.

So the chain *daemon order → `live_session_order` → sidebar* is intact, and an
epoch would guard a door that is already open. ⇒ **The divergence is UPSTREAM:
the snapshot the GUI polls does not carry the reorder.**

✅ **THE REPLY IS NOW HONEST, live-proven on guihost 2026-08-07** with a
deliberately reversed control: `changed:false`, `matches_request:false`, and
`rendered_order` returning the ACTUAL rendered list (head `29b04124…`) rather
than the request. Before this, the same control answered ok/applied with
`rendered_order` equal to the request. ⇒ **the verb still does not move the
sidebar, and now says so** — which is the whole point of the response-layer rule.

⭐ **The surviving hypothesis, and the next probe.** guihost runs four daemons and
rows are owned across them, so the serving daemon **aggregates** rows it does not
own. If that aggregation rebuilds order from its own adoption sequence, it
overwrites `live_session_order` on every poll — which explains all of it: the
reorder lands, the next aggregation discards it, a **restart** reads the
persisted order once before aggregation takes over, and new rows arrive at the
head because that is where adoption appends them.
**Probe:** reorder, then compare `live_session_order` against the order of
`snapshot.live_sessions` on the serving daemon across two consecutive polls —
if the field holds and the projection does not, the aggregation is the writer.
Start at `append_restored_live_session_order` and the peer-adoption path.

⚠ Do not simply pin the order in the GUI either: drags are the one row
interaction the user performs by hand, and a pin would strand them.

⚠ **An earlier claim in this entry was wrong and is retracted**: I reported a
02:47 case where a running GUI DID adopt a reorder without restarting. It does
not survive re-measurement, and the forwarding test above contradicts it. The
orchestrator's model stands.

⚖ **Third verb in this family**, and worth one fix at the response layer rather
than three: `session remove` grew `verified` for exactly this, `session rename`
still answers `accepted:true` on failures, and `reorder` reports a daemon field.
**All three report the REQUEST, not the EFFECT.**

⇒ **And it is the argument for DERIVING the outline** (the parentage entry
below): a stored order exists in two copies that drift, is re-typed after every
launch, and gets prepended to by every new row. A derived order re-derives after
any restart and cannot diverge, because there is no second copy.

⚠ Enabling condition, not cause: guihost runs four daemons (3.0.29, 3.0.32, 3.0.41,
3.0.44) because *a daemon owning a plain `local://` shell can never be retired* —
see that entry. With N daemons each holding an order, "which copy does a restart
restore from" is ambiguous, which is what makes this unreproducible day to day.

## ★★★ ROW PARENTAGE: NOTHING RECORDS WHO SPAWNED A ROW

**Status:** OPEN

Owner, 2026-08-06, looking at the sidebar: *"We need the 0, 1, 1.1, 2, … top to
bottom for the agent rows sanitization and somehow to track who further spawns
nested rows of ychrome, yRDP, etc. and organize like this."*

**Half of it works today and needs nothing:** `server sessions reorder` applied a
hand-built outline on guihost (37 rows, `matches_request: true`, none skipped). The
verb is right.

⛔ **The half that cannot be faked: there is no parent link, anywhere.**
`CreatorStamp` (`session_tenancy.rs:65`) records pid + host + purpose, and its
own doc says the pid is *"dead by the time anyone reads this — the value is the
AUDIT trail, not a handle."* A grep for `parent_session` / `spawned_by` /
`parent_row` across `crates/` returns nothing. So:

- the outline numbers are **hand-typed**, and the next orchestrator has no way
  to know `6.1` belongs under `6`;
- a ychrome or yRDP surface opened BY a delegate arrives as a **top-level
  orphan** — four were on the table unattributable (2× "Agent unnamed shell"
  badged `tenant-sample`, 2× "New Ychrome" badged `sample-a`/`sample-b`);
- any restart re-scrambles it.

**Cheapest-first, and the order matters:**

1. `parent_session_path` on `CreatorStamp` — the spawning ROW, not its pid,
   written at create time. ⚠ Critically by the **surface-creation paths**
   (ychrome, yRDP), not just `terminal new`, because that is where nesting
   actually comes from.
2. Expose it on `server app rows` beside `session_id`.
3. **Derive** the outline from the parent chain, depth-first — so `6.1` is a
   fact about who spawned whom and re-derives after any restart, instead of
   being someone's typing.
4. Optionally `sessions reorder --outline` to sort by the derived numbers.

⛔ **A TRAP FOUND BEFORE IT WAS SPRUNG, 2026-08-07 — do not stamp a GUI launch
as an agent creation.** The obvious implementation of step 1 is to have
`spawn_launch_app_verb` declare a tenancy on the row it just made, since it
ALREADY holds the parent: `insert_after` is the anchor row's `full_path`, passed
through `launch_anchor_row` → `start_local_session_placed` / `start_ssh_session_placed`.
So the parent link needs no new plumbing on that path at all.

But `RowHygieneVerdict` reads *"no creator stamp ⇒ a human or the GUI opened
this — NEVER a plate, at any age"* (`session_tenancy.rs`). Hanging parentage off
`CreatorStamp` therefore turns **every ychrome the user opened by right-clicking
a row** into an agent-created row the sweep may consider. That is a regression
the sanity system would deliver quietly, on the user's own table.

⇒ **Parentage and provenance are two questions and want two fields.** "Who
spawned this row" is orthogonal to "did an agent make it", and the hygiene
classifier keying off the presence of a stamp is the proof that they must not
share one. Give the parent its own metadata label rather than widening
`CreatorStamp`'s meaning — the field predecessor A added there stays for the
CLI-create path, which genuinely is an agent creation.

⚖ **It also fixes the sanity system's cross-host problem from a better angle.**
That was patched on 2026-08-06 by asking the session's `source`/`host_label`,
which works — but a row that knows its PARENT knows which host its child runs on
by construction, and the same stamp is what would let a delegate be swept
*together with everything it spawned*.

## ⚠ `server app session remove` REPORTS A TIMEOUT ON WORK IT COMPLETED

**Status:** OPEN

Found 2026-08-06 clearing rows on guihost: the verb exited 1 with
`timed out waiting for app control response … after 15000 ms` on rows it had in
fact **already removed** — the table went 42 → 37, exactly as asked.

A timeout that reads as failure on completed work is the inverse of the
lie-of-success shape and costs just as much: **a caller that retries on it
chases ghosts**, and an agent scripting a bulk clear will either double-remove
or report a failure the user then investigates. The verb should confirm against
the resulting row set before reporting a timeout, or say plainly that the
request outlived its response window without asserting the work failed.

### ⚠ IT HAS NOW COST A LIVE ORPHAN — reproduced twice more, 2026-08-06 night

The orchestrator hit it on **all four** of four lobe removals: 15 s timeout
reported, every row really gone, every pid really dead. **The damage was not the
timeout — it was what the timeout taught the caller to do.** Reading "row gone"
off the row list instead of the reply's own `verified` field let row 6.1's
`claude` survive its removal as an **orphan**: alive, no row, invisible to the
owner, until it was hunted by hand.

⇒ **`verified` is the field, not the row list.** Reproduced deliberately while
clearing a probe row: the reply said `verified: false` with
`verified_refusal: "remote_runtime_survived"`, and the remote `claude` was
indeed still running on the other host. **The instrument is honest and the row
list is not** — a row leaves the table before its runtime is confirmed dead.

Two more measured facts for whoever takes this:

- `live_processes: []` was reported **in the same reply** as
  `verified_refusal: "remote_runtime_survived"`. Those two cannot both be
  right; the empty list is the one that lies, and a caller trusting it would
  conclude the reap was clean.
- The daemon then **respawns `server remote terminate-cc <uuid>` in a loop**
  against a runtime that will not die — several were alive at once and new ones
  appeared after each kill. A retry with no ceiling is its own defect and it
  hides the first one.

## ⛔ `session rename` TELLS THE GUI IT FAILED AND THE CALLER IT SUCCEEDED

**Status:** OPEN

gadgets row, 2026-08-06, while executing rule 1 of
[`agent-row-hygiene.md`](agent-row-hygiene.md) §The outline contract:

```
server app session rename 'remote-cc://dev/does-not-exist' 'probe'
  → {"accepted": true, "reason": null, "session_path": "…/does-not-exist"}
```

**The daemon is not confused — the check works.** `server app state` carries the
notification it raised at the same millisecond:

```
{"title": "Rename Failed", "message": "paper not found: remote-cc://dev/does-not-exist",
 "tone": "Error", "id": 38, "persistent": false}
```

⛔ **The two channels disagree, and they are pointed at the wrong parties.** The
refusal goes to the GUI as a toast — to the human, who did not issue the call
and cannot act on it — while the **caller that can** retry, correct its path, or
stop claiming the rule is done is handed `accepted: true`. The response even has
the slot: `reason` is `null` on a call the daemon had a named reason to reject.

No row is created (`server app rows` holds 35 before and after), so this is a
silent no-op rather than corruption. That is also what makes it expensive to
catch: the only way an agent learns the truth today is to re-read the table.

✅ **STILL TRUE AT 3.0.141, and now with BOTH HALVES MEASURED IN ONE SESSION,
which states the defect more sharply than the original report could:**

```
rename <a row that exists>      → accepted: true, reason: null   … and the label really changes
rename <a path that does not>   → accepted: true, reason: null   … and nothing happens
```

⇒ **The two replies are identical in every field a caller reads.** The defect is
not that a failure is reported as a success — it is that **success and no-op are
indistinguishable from the reply**, so no amount of careful checking on the
caller's side can tell them apart. `reason` is `null` in both, and it is the slot
that was built to hold the difference.

⚠ **The toast half was NOT re-observed** and I am not claiming it is gone: I
looked for `notifications` / `toasts` in `app state` and found neither key
populated, which says my probe did not find them, not that the daemon stopped
raising them.

**Falsifier:** a rename of a path that does not exist answers with `accepted:
false`, or a `reason` a caller can read.

⚖ **Same family as the neighbouring `session remove` entry, one layer up.**
There the lesson was *`accepted` is the request being understood, not the work
being done*, and `remove` grew a `verified` field for it. `rename` has no such
field — and unlike `remove`, its failure is already computed and simply not
returned.

**Fix:** fill `accepted: false` + `reason: "paper_not_found"` from the same
branch that raises the toast. The GUI notification is fine to keep; it is the
CLI's silence that is the bug.

⚠ **My probe was untargeted, and that is on me, not on the daemon.** The
`data-fabric` skill already carries the law — *"an agent testing notifications
with no target is not testing anything; it is posting to the seat he is sitting
at"* — and `session rename` takes `--pid`, so the probe should have been aimed
at a shadow client. It was not, and the `Rename Failed` card landed on the
user's own panel. **The rule generalises past `notify`: any probe of a verb that
can raise a toast needs the same targeting**, which is not obvious from
`rename`'s signature.

⚠ **Still a real gap underneath it:** an agent can *raise* a notification but
there is **no verb to dismiss one** (checked against `server app --help`, which
is generated from the dispatcher). So even a correctly-targeted probe cannot
clear its own card. Worth a `notify --dismiss <id>` for the same
agent-cleans-its-own-plate reason the hygiene contract is built on.

## `server sessions reorder` WITH NO FILE REPORTS ITSELF AS NONEXISTENT

**Status:** OPEN

Orchestrator, 2026-08-06: `server sessions reorder` with no file argument answers
**`unsupported server sessions action: reorder`** — because the branch requires
`argc >= 4`, so an arity miss falls through to the unknown-action arm and a verb
that exists denies existing.

That is a discovery cost paid by every future caller: the honest answer is a
usage error naming the missing argument. ⚠ Same shape as any parser that treats
"wrong number of arguments" as "no such thing" — worth a scan for siblings while
fixing it.

✅ **STILL TRUE AT 3.0.139:** `server sessions reorder` →
`Error: unsupported server sessions action: reorder`, verbatim.

**Falsifier:** the arity miss names the missing argument instead of denying the
verb exists.

## ★★ WHO OWNS "IS THIS ROW WORKING?" — three tools, three answers

### ⛔ 2026-08-07 — I BECAME THE FOURTH TOOL, ON THE OWNER'S OWN ROW

`terminal input-check` shipped this morning, and within the hour I reported his
PAUSED delegate as *"grinding the licence audit"*. He caught it: *"first figure
out why you mis-read the pause as grinding and fix the root cause, otherwise the
same bug will bite us later."*

**Root cause: a LIVENESS verb answered a PROGRESS question, and its reply
invited that.** `consuming_input: true` is exactly what a healthy IDLE row
reports — the verb's own docs say so — but the reply read
`consuming_input: true, wedged: false, reason: "the session echo-confirmed it is
consuming input"`, which is a green light to anyone holding a progress question.

⭐ **The signal that would have answered correctly was in the same daemon screen
the probe already reads.** Three live rows in one guihost snapshot:

```
idle       ⏵⏵ bypass permissions on (shift+tab to cycle)
PAUSED     ⏵⏵ bypass permissions on (shift+tab to cycle) · ← 1 agent
WORKING    ⏵⏵ bypass permissions on (shift+tab to cycle) · esc to interrupt · ← 1 agent
```

Shipped in 3.0.51: `activity: working|idle|unknown` on the reply, read from
`AgentCliDescriptor::working_footer_hints` — per-CLI DATA, so a CLI that words
it differently declares its own instead of being silently reported quiet, and an
UNMEASURED CLI answers `unknown`, never `idle`. Codex's phrase is unmeasured and
its list is deliberately empty. The reply also carries an `answers` field naming
what the verb does and does not settle.

⚠ **Still open, and this entry stays open for it:** `activity` is a FOURTH
answer to "is this row working", beside the working-indicator, `busy_reason` and
the title/summary pipeline. It is the first one derived from what the CLI itself
draws, so it is the best candidate to become the single owner — but until the
other three are collapsed into it, this entry is not closed.

⚠ **And a false start worth not repeating:** I first suspected my own draft
guard (that a faint composer might be his unsent draft, so the probe's Ctrl+U
would eat it). Falsified by controlled test — typing into a live composer
renders at NORMAL intensity (`❯\u{a0}THIS_IS_AN_UNSENT_DRAFT`, no `ESC[2m`), so
faint really is chrome and the guard is correct.


**Status:** OPEN

Orchestrator, 2026-08-06, after telling the owner that delegates were working
when **3 of ~33 processes** were actually mid-turn, and being corrected by him.

**The failure is a granularity mismatch, and it is partly ours.** `row-health.py`
(data-fabric) verdicts are **per-cwd**, so `WORKING <cwd>` only means *some*
transcript under that directory moved — and in the orchestrator's own cwd it
reported the orchestrator back to itself as evidence of delegate progress.

Their replacement (`~/.claude/skills/data-fabric/scripts/row-work.py`) links
pid → transcript by EVIDENCE (session id from argv, else the runbook path
matched inside the transcript) and judges by **turn state**: walk back past
system rows; assistant text = turn ended = IDLE; `tool_use` / user
`tool_result` = mid-turn.

⇒ **yggterm should own this verdict, not a skill script.** The row model already
holds the pid, the host and the session id; `server app rows` already carries a
`working` field. One honest answer there replaces three tools that disagree, and
it is the same field the working-indicator already wants
([[spec-title-summary-working-indicator]]).

⚠ **Do not import their STUCK verdicts as-is.** `--resume` can fork a
transcript, so the reported ages (4-15 days) are most likely MISLINKS rather
than wedged sessions. Settle the fork case before any reaper trusts STUCK —
this is the same family as
[[finding-agent-session-liveness-is-invisible-to-os-signals]], where an
absence was mistaken for a state.

## ⚠ ~9 RAW `session_path == row.full_path` COMPARISONS ARE UNAUDITED

**Status:** OPEN

Found while fixing the rename persist gate, 2026-08-06. A sidebar row can spell
a session `local::<id>` while the live record spells it `local://<id>` — that is
the entire reason `normalize_live_session_path` exists. The rename gate compared
them RAW and therefore read false for those rows, silently never telling the
daemon about the rename. **That one is fixed** (`live_session_matches_row_path`
is now the one comparison owner).

⛔ **The other ~9 sites in `shell.rs` are NOT fixed and NOT audited**, and this
entry exists so nobody assumes the class was swept. Each needs its own reading:
some compare a row against a live record (suspect), some may legitimately want
identity on one spelling. `grep -n "session.session_path == row.full_path"`.
A blanket normalize would be its own bug — the point is that no one has looked.

## ★★ THE RENDER PIPELINE STILL INTERLEAVES CHARACTERS FROM AN OLDER FRAME

**Status:** OPEN

User-reported 2026-08-06 with a faithful screenshot, after a GUI restart and a
long scroll: *"Rendering pipeline still needs work."*

**The fingerprint, and it is unmistakable once seen** — words from two different
frames fused character by character, spaces eaten:

```
BothsX'spcalledthefsameedelete,aso "stop covering my screen"tando"forget this happened"
Itsalsoecaughtttthato--job forcestpersistentoandinever chimesi(so0--silentdisia no-op there)
The history dies with the GUI.nIt'srannin-memoryiVec, southisrveryedeployeemptied,yourcpanel.
```

Read it as `Both X's called the same delete` overwritten INTO the previous
frame's text, one cell at a time. Every SPACE holds a character from an older
frame — the same fingerprint already recorded for the bottom-rendering bug in
`docs/xterm-bugs.md`, so this is that family, not a new one.

**What is new and worth carrying:** it appeared on a FRESH GUI (3.0.35, minutes
old) on a session that had just been re-resumed after a restart, while a long
answer streamed. That is the same window the field guide already warns about —
*a daemon swap re-resumes the CLI on a fresh PTY, and that re-resume window IS
the corruption* — which makes this a strong, cheap repro rather than a mystery:
**restart the GUI, then stream a long answer into a re-resumed row.**

⛔ Do NOT reach for `reconcile` on a session showing this — it is a destructive
full reset+re-seed and has blanked a live viewport before. And do not repeat the
3.0.28 remedy (a resize nudge after the reconcile): SIGWINCH makes an in-place
CLI redraw only the region it still owns, which turned the interleaved bottom
into a BLANK transcript and was reverted at 3.0.29. The atlas/full-refresh path
is the suspect, not the daemon's screen — the daemon's copy is correct, the
CLIENT paints less than it holds.

### ⭐⭐ 2026-08-07 — THE TRIGGER IS A LAPTOP SUSPEND, AND THAT NAMES THE WINDOW

Re-reported by the owner with a faithful frame, same fingerprint, on the
orchestrator row `remote-cc://dev/1275246b-…`: *"I saw this on the orchestrator
session when I woke up laptop after myself waking up from sleep."*

**The trace settles what happens, and it is not a mystery any more.** guihost's
event trace has three wall-clock holes this morning — 09:01:39→10:42:45,
10:43:45→11:52:58, 11:53:58→12:46:16 — and each wake is followed within seconds
by the SAME chain on the affected row:

```
10:42:45 suspend_wake      bridges_respawned          {suspend_ms: 6058164}
10:42:45 terminal_runtime  suspend_wake_bridge_respawn {path: …1275246b…, cols:170, rows:65}
10:42:45 terminal_runtime  spawn / replace_exited_runtime          <- a NEW PTY
10:42:52 terminal_mount    terminal_stream_cursor_rewound
                            {previous_cursor: 10106, next_cursor: 1, chunk_count: 1}
10:42:52 terminal_mount    bootstrap_reset {mount_epoch: 2}        <- client re-mounts
```

⇒ **a suspend is a daemon-swap-equivalent.** `respawn_ssh_carried_sessions`
deliberately kills and respawns every ssh-carried bridge on wake (the TCP
connections are dead and ssh would take ~45 s of ServerAlive to notice), so the
CLI is RE-RESUMED on a fresh PTY — and the field guide already says the
re-resume window IS this corruption. The value of the finding is that the window
is now **named, dated and reproducible on demand: suspend the laptop, wake it,
look at a busy agent row.** No more waiting for it to happen.

**Two suspects FALSIFIED today, cheaply — do not re-derive them:**

- **The `batch_terminal_chunks` excision is NOT doing this.**
  `terminal_forward_divergence` has fired **zero** times across every retained
  trace file on guihost. The 679-byte whole-line excision measured in 2026-07-11 is
  real but is not the mechanism here, so the parity rework is not the gate on
  this particular report.
- **The idle trim is NOT doing this either.** A suspend does make every session
  read as idle at once (`trim_idle_buffer` compares WALL clock against
  `last_activity_ms`, and 101 minutes of sleep clears any threshold), and
  `terminal_buffers/idle_trim` did fire at the wake instant — but
  `trim_idle_buffer` skips `launch_command_looks_like_remote_resume_attach`
  sessions, so the agent rows are already exempt (`trimmed_sessions: 1`, and not
  this one).

**Surviving hypothesis, stated so it can be attacked:** the re-mounted host is
seeded from the RETAINED surface (the previous transcript), and the re-resumed
CLI then paints a diff-style frame over that base rather than a full clear.
Every cell the new frame does not rewrite keeps the retained character — which
is exactly "every space holds a character from an older frame". The repair that
exists for this is the screen reconcile, and the trace shows it being held off
constantly: `screen_reconcile_deferred_recent_output` 738,
`screen_reconcile_skipped_working_surface` 664,
`screen_reconcile_skipped_unwritable` 297 — against
`screen_reconcile_forced_deadline` 551. That is §THE QUIET-GATE LAW again, and
the deadline is the only reason it ever runs at all on an agent row.

⚠ One neighbouring defect fell out of the same trace and is worth fixing on its
own: `respawn_ssh_carried_sessions` re-creates each bridge at the dying
runtime's `current_cols/current_rows`, and one row came back at the DEFAULT
**120×36** (`{path: …1f4a3c27…, cols:120, rows:36}`) while its siblings came
back at 170×65 — because a row whose PTY was never resized still carries the
default. A CLI re-resumed into a 120-column PTY re-wraps its whole transcript at
120 and paints it into a 170-column client grid. That is not what the owner
photographed (his row respawned at 170×65) but it is a second, independent way
to get text at the wrong columns after a wake.

## ★★★ A DRAG-SELECT OVER A STREAMING SESSION SELECTS THE WHOLE STREAM

**Status:** OPEN

*(Half of it shipped: the viewport pin. The runaway selection itself is what
remains open, and it is the half the user feels.)*

User-reported 2026-08-05: *"CC paste is working but selection makes the Ux lag
and sometimes guihost angry."* Their guess — frame writes — is the right
neighbourhood; the amplifier is the selection SIZE.

**Measured on a shadow client, identical drag geometry, same session, mid-screen
so xterm's drag-scroll never arms:**

| terminal | chars selected | `onSelectionChange` fired | viewport intent |
|---|---|---|---|
| idle | **608** | 2 | `UserScrollback/selection_active` |
| streaming | **902,649** | **0** | was `PromptFollow/focus` |

A 2.4 s drag selected **909,143 chars over 10,036 lines**, and each
`term.getSelection()` on a selection that size costs **18-23 ms** on the same
webview thread as the xterm write pump — more than a frame budget, on every
flush. That is the felt lag, and the user also silently gets ~300x more text
than they dragged over.

**Root cause, in two layers.**

1. ⛔ **THE QUIET-GATE LAW again.** The "a selection pins the viewport" guard
   hung only off `term.onSelectionChange`, and that event **does not arrive**
   while an agent CLI streams — zero firings across a drag that selected 902,649
   chars. So the pin never armed. **FIXED**: both arm and release now hang off
   the pointer gesture (`applySelectionScrollbackIntent`, one owner, called by
   pointer-down, pointer-up and the selection-change path). A second defect at
   the release site is fixed with it — the reached-bottom escape dropped ANY
   `UserScrollback` pin the instant the viewport sat at base, which during a
   stream is continuously true; traced live, the pin died **116 ms** after it
   armed. It now survives via `selectionOwnsScrollbackPin`.
2. ⏳ **STILL OPEN — the pin is not sufficient.** It governs only *our* scrolls.
   xterm follows the tail itself whenever `ydisp === ybase`, and a drag's end
   anchor is `(pointer viewport row + ydisp)`, so the same screen position keeps
   resolving to a larger buffer row and the selection swallows every line
   emitted during the gesture. Verified after the fix: intent now correctly
   reads `UserScrollback/selection_active` for the whole drag and the selection
   is **still 902,649 chars**.

⛔ **DO NOT "fix" it by freezing the viewport during the drag — that was tried
and it OVERCORRECTS TO NOTHING.** Forcing `ydisp` back to the drag-start row on
each write made the streaming drag select **0 characters**: the forced scroll
perturbs the same anchors the drag is building. A drag that selects nothing is
worse for the user than one that selects too much, so it was reverted rather
than shipped. The next attempt should constrain the SELECTION END (clamp it to
the row the pointer is actually over, in content terms) rather than move the
viewport under a live gesture.

**The arithmetic that names the right fix.** The selection grows by EXACTLY the
number of lines the viewport scrolled during the drag: the end anchor is
`(pointer screen row + ydisp)` and `ydisp` advances with the stream, while the
start anchor is a fixed absolute buffer coordinate. Re-anchoring the start by the
same delta would hold the selection constant — but xterm's public API is only
`select(col,row,len)` and `selectLines(a,b)`, neither of which can express an
arbitrary `(col,row)→(col,row)` range, so the model cannot be corrected from
outside.

⭐ **Therefore the likely answer is to hold the WRITES, not the viewport:** while
the pointer is down, queue PTY output instead of writing it into xterm, and
flush on pointer-up. Nothing is written, so `ydisp` cannot move, so the end
anchor cannot advance, and the user selects exactly what they dragged over. A
drag is a second or two and the user is demonstrably not reading new output
during it. ⚠ This must carry a DEADLINE and a byte cap that flush anyway —
§THE QUIET-GATE LAW forbids gating on an absence, but this gate is legal
because its release is a positive signal (pointer-up) with a bounded fallback.
Not attempted yet; it touches the write pump and deserves its own careful pass.

**The instrument that settles any future attempt** — a shadow, an ephemeral row,
a `while :; do echo …; done`, and a synthetic drag. ⚠ Two instrument traps, both
of which produced a confident wrong reading first: xterm's `handleMouseDown`
requires **`detail: 1`** on the synthetic `MouseEvent` (`1===e.detail` selects
the single-click branch) or no selection is ever made and the arm reads as "no
cost"; and the handler is bound to **`mousedown`**, not `pointerdown`. Verify the
counter is bound to the CURRENT `term` object — a remount silently orphans it.

## ★★ YCHROME PLAYS A YOUTUBE VIDEO TWICE, AND THE SECOND ONE CANNOT BE STOPPED

**Status:** OPEN

User-reported 2026-08-06: *"ychrome plays double youtube video with the next
same video unpausable or mutable. I have to close ychrome to kill this phantom
youtube tab."*

**What the symptom already tells us, before any probe.** A page's own pause and
mute controls act on the DOM they can reach. A video that ignores them is
therefore **not in the tab being looked at** — it is a second surface. That it
survives everything short of killing ychrome says it has no row to close.

**So this is very likely an already-filed defect wearing a new symptom** — see
*A LIVE, LEASED WEB SURFACE CAN EXIST WITH NO ROW* and *A SECOND VIEWER STILL
BUILDS ITS OWN WEBVIEWS* in this file. Check those before opening a new lane.

**Is it the cause of the YouTube judder?** ⚠ Plausible, unproven, and worth
taking seriously rather than assuming. The judder entry in the render batch
below already **falsified the decode explanation** — stats-for-nerds reports
almost no dropped frames, so the decoder is keeping up and the fault is in
PRESENTATION. Two simultaneous presentations of the same video is exactly a
presentation-layer cause, and "overlaps" is what a second presenter would look
like. It would also explain why the judder never correlated with anything in the
decode pipeline.

⛔ Not observable on demand yet: `server app state → web_surfaces` read **0**
while ychrome held 4 anchored sessions, because a surface only registers once
its session is activated. **Reproduce first, then measure** — with the double
play live, count webviews for that profile, and read stats-for-nerds on the
VISIBLE player with and without the phantom. If judder is present only when a
phantom exists, that closes both this and the judder entry at once.

## `~/.yggterm`'s 700 sockets are ALIASES, not corpses — the real growth is `client-instances`

**Status:** AWAITING A DECISION

*(The user decides. Pruning the alias source is a behaviour change, not a
cleanup — see the decision paragraph below.)*

⛔ **THE ORIGINAL PREMISE OF THIS ENTRY WAS WRONG AND IS CORRECTED HERE.** It
said "~700 `server-<version>.sock` files … every other socket is a file no
process will ever bind again", and prescribed a sweep. Measured on guihost
2026-08-06:

```
server-*.sock total=674   symlinks=670   real sockets=4
listening yggterm unix sockets: 7
client-instances scope dirs: 675   of which EMPTY: 628
```

**670 of the 674 are SYMLINK ALIASES resolving to the LIVE daemon, and all four
real sockets are listening. Zero are dead.** So the sweep the entry asked for is
correct and collects nothing — the right verdict for all 674 is KEEP.

**Where the growth actually comes from:** `refresh_legacy_server_socket_aliases`
regenerates one alias per version on **every daemon bind**, seeded from the 675
scope directories under `$YGGTERM_HOME/client-instances/` — **628 of them
empty**. Deleting alias files alone is futile; they are recreated on the next
bind. The count only comes down by pruning the `client-instances` registry.

⚠ **THE DECISION, and it is the user's** — retiring a live-pointing alias is a
behaviour change, not a cleanup: an older client that loses its alias **falls
back to spawning its own daemon**, which is precisely the daemon-proliferation
the fleet already suffers from. So "prune the empty scope dirs" needs a rule for
what an empty scope dir MEANS (a client that never registered? one that exited
cleanly? one from a version no longer installed?) before anything is unlinked.

**SHIPPED IN THE MEANTIME, and it fixed a live hazard rather than the cosmetics:**
`socket_sweep.rs`, which **replaces** `cleanup_dead_versioned_server_sockets` —
a function that ran on every daemon start and unlinked a socket **whenever
`status()` failed**, i.e. exactly the thing this entry's own warning forbade. A
daemon mid-restart has a moment with no listener, and the old code would delete
its address there. The new predicate issues **no `connect` at all**: liveness is
proved positively from one read of `/proc/net/unix`, a path must be dead in two
rounds ≥24 h apart, and an unreadable census keeps everything.

Related: `docs/agent-row-hygiene.md` (the same class of accumulation, for rows).

## ★★★ A DAEMON OWNING A PLAIN SHELL CAN NEVER BE RETIRED — increment 2, steps 2-3

**Status:** OPEN

Step 1 of 3 is shipped and tested; steps 2-3 are the open work.

This is the CONSTITUTION's *"the user must never have to know which daemon owns
what"*, and it is the reason the session rail reads **"daemon is on 3.0.22 ·
older than this client"** on the user's own GUI while a 3.0.26 daemon serves
beside it.

**What the user sees.** They press **Hot-restart daemon** in the rail. It
answers success. Nothing changes. Measured 2026-08-04: five guihost daemons each
returned `hot update handoff started: preserving N live terminal runtime(s)`
with `spawn_ok: true` / `successor_already_live: true`, and afterwards the
predecessor still held **all 14** of its PTYs and the successor held **none**.
Three of those attempts were the user's own button presses.

**Why.** The drain (`spawn_progressive_session_migration`) converges by
RELEASING a session so a newer daemon re-resumes it, and
`session_kind_is_migratable_agent` admits only Codex/CC — correctly, since an
agent's state is in its own JSONL. **A plain shell has no such persistence, so
it is never released, and there is no other way for a PTY to leave its daemon.**
The owner therefore lingers on its old version for as long as that shell lives,
which for a keep-alive shell is forever. guihost's preserved set was 8 `local://`
shells to 2 `remote-cc://` rows, so this is the common case, not the corner.

⛔ **CORRECTION, measured 2026-08-09 — the sentence above was wrong in the one
way that mattered, and the fix is shipped in 3.0.81.** There *was* a second way
for a PTY to leave its daemon: the retire loop's **cold shutdown**, which killed
it. The cold-shutdown gate deferred only while a session was *recently active*,
so a shell that fell silent for 300 s cleared the gate and the daemon retired
**by destroying it** — the absence-gate treating idleness as safety for the one
class of session where idleness proves nothing at all. So a daemon owning a
plain shell did not linger forever; it lingered until the user stopped typing.

Falsified live, both arms, in an isolated `YGGTERM_HOME` with
`YGGTERM_HOT_UPDATE_IDLE_THRESHOLD_MS=1` and one live `local://` shell:

| binary | `hot_restart_blockers` |
|---|---|
| shipped 3.0.80 | `[]` — nothing blocking; the next tick would have killed it |
| 3.0.81 | one `not_restorable`, `permanent: true` |

3.0.81 makes this entry's own premise TRUE: `session_kind_state_survives_pty_loss`
is now the single fact both PTY-destroying paths read, so the retire loop refuses
exactly what migration already refused. ⚠ **That closes a data-loss path; it does
not converge anything** — a daemon holding a shell now genuinely never retires,
which is why step 3 below is still the answer and is now the *only* one.
⚠ `YGGTERM_HOT_UPDATE_IGNORE_IDLE_GATE` deliberately does NOT clear this blocker;
it was a licence to skip prompt-cache freshness, never to destroy a live shell.
⚠ Guard the regression the other way too: an AGENT session must stay clearable.
Proven in the same sandbox — a `cc-runtime://` row reports `blockers: []`.

**Steps 1 and 2 are done.**

- **Step 1** (`0767a868`) — `pty_handoff_wire` moves a master fd between daemons
  over `SCM_RIGHTS` with `MSG_CMSG_CLOEXEC`, four tests including the spike's
  negative control.
- **Step 2** (`d39bdb53`) — `TerminalManager::adopt_session` installs a runtime
  around a received fd: `PtySessionRuntime::spawn` split at its seam so `spawn`
  and `adopt` share one assembly, carried screen replayed before the reader
  thread starts, and refusals rather than guesses for a live runtime already
  under the key or a `(pid, start_time)` that cannot be confirmed alive.
  Acceptance: a real bash, a real `SCM_RIGHTS` transfer, predecessor's master
  dropped, and the shell still EVALUATES — mutation-checked to fail in 10 s
  when the command is never sent.

**What is left:**

3. **The send side.** A `HotUpdateHandoff` that owns un-migratable runtimes
   should, per runtime: write the transcript line, `sendmsg` the master fd,
   then drop its runtime **without killing the child** so it re-parents to init,
   and retire once its hands are empty.

⚠ **The send side is where a bug destroys data, and it has one hazard the
receive side does not:** dropping the predecessor's `PtySessionRuntime` must not
run the ordinary shutdown path, which kills the child. Dropping the master alone
only `SIGHUP`s the foreground group.

⛔ **Do not build the send side first.** `sendmsg` success is the commit point
(settled in `settled-calls.md`), so a send whose receiver cannot install the
runtime **destroys the user's live shell** with no way back. That failure mode
is worse than the bug.

Inherited and not to be re-litigated: transcript travels BEFORE the fd; the
child re-parents to init; identity is `(pid, start_time)`, never the pid alone.


## ★★★ A CLIENT ONLY EVER GETS THE FULL RECORD FOR THE *DAEMON'S* ACTIVE SESSION

**Status:** OPEN

This is the CONSTITUTION's co-browse guarantee, measured.

Found 2026-08-03 while fixing the Web View's 2-of-N truncation, which is the
same defect seen from the other side.

**The shape.** `ServerUiSnapshot` has exactly one uncapped session record:
`active_session`, and it is the record for the path the DAEMON considers
active. Every other session arrives through `live_sessions[]`, which
`snapshot_live_session_view` clips to `LIVE_SNAPSHOT_PREVIEW_BLOCK_LIMIT` (2)
preview blocks of 6 lines. A client whose viewport is its OWN — which is
precisely what `viewport_is_client_owned_for_role` grants a Shadow, and the
whole reason a shadow exists — can therefore never receive more than 2 clipped
turns of the session it is looking at.

**So the agentic surface cannot render the product's primary content.** That
collides with two standing rules at once: *the agentic surface is the default
test surface*, and the CONSTITUTION's *"click it and CO-BROWSE it"*. A Web View
change cannot be pixel-proven anywhere except the user's own GUI, which the
shadow-probe law exists to keep us off.

**Half of it is fixed; this is the half that is not.** A shadow used to be
refused `RefreshPreview` outright (`shadow_cannot_own`), so its remote rows sat
on launch scaffold forever — that gate is now `Allow`. What remains is the
payload shape: hydration fills the DAEMON's store, and the snapshot still hands
this client the clipped copy of its own viewport.

⚠ **AND THE GATE FIX ONLY LANDS WHEN THE SESSION'S OWNER CARRIES IT.** Measured
on guihost 2026-08-03 with a 3.0.2 daemon SERVING and a 3.0.1 daemon still OWNING
the rows: a shadow's `refresh_preview` was still refused, and the trace names
the refuser — `pid 1325229, component daemon, shadow_refused` — which is the
OLD daemon. The request is proxied to the row's owner, so the answer comes from
the owner's compiled-in `role_gate`, not the server's. **A daemon-side gate
change is therefore inert for every session an older daemon still owns**, which
on a version-coexisting fleet is all of them until ownership migrates. Same
family as the pre-2.12.10 declare-proxy failure: check WHICH pid answered
before concluding a daemon-side fix is live.

⛔ **Do not fix this by widening the read-only hatch or by adding a second
"full record" field next to `active_session`** — two fields answering "what is
this session's record" is the same duplication that caused the truncation bug.
The CONSTITUTION already names the right shape: **per-viewer geometry and
per-viewer records over a shared session**, i.e. the snapshot serves the
requesting client's viewport, not the daemon's. That is a protocol change and
deserves its own design pass.

**Falsification that would close it:** on a shadow whose viewport is a
`remote-cc://` row that the daemon's active session is NOT, read
`data-preview-window-total` — it must equal the block count the reader reports
for that transcript, not 2.

### ⚠ IT IS NOT A SHADOW-ONLY DEFECT — measured on the USER'S GUI, 2026-08-03

Filed above as an agentic-surface problem. It is not: the same 2-block payload
reaches the user's own window whenever the daemon's active session is not the
row they are looking at.

Measured on guihost, client 3.0.16 against a 3.0.15 daemon, with
`remote-cc://dev/91527c7b…` open in the Web View:

- `server app state` → `dom.preview_visible_block_count` = **2**, block ids `-0`
  and `-1`, `preview_viewport_rect.height` = 429 inside a ~1160px pane.
- The same transcript reads **298 blocks** through `server remote preview-tail`
  on the host that owns the file.
- Still 2 after a 45 s settle, so it is not a hydration race.

**The user's report is the symptom, in their words: "the content is not rendered
in half of the screen."** Two turns render, the rest of the pane is empty — and
it looks like a layout bug, which is what sent the first investigation at the
block-height estimator instead of at the payload. Anything diagnosing a
short/blank transcript must read `preview_visible_block_count` FIRST; a surface
drawing exactly 2 blocks is this bug and no amount of layout work will move it.

That also raises the priority. The rule above says a Web View change "cannot be
pixel-proven anywhere except the user's own GUI" — that consolation is gone,
because the user's own GUI hits it too the moment another row is active.


## ★★ TWO `web fill-vault` CALLS IN A ROW INTERLEAVE, AND CORRUPT BOTH FIELDS

**Status:** OPEN

Found 2026-08-02 driving the IP India TM-A filing — on a login form, with a
vault password, which is the worst place for it.

**What happened.** Two fills fired back-to-back against the same page:

```
web fill-vault --field username --selector '#TBUserName'   # vault holds "avikalpa"
web fill-vault --field password --selector '#TBPassword'   # vault holds 14 chars
```

The username field read back **`avikalpad`** — one character too many — and the
password field read **15** characters on one probe and **16** on the next read
of the same field, against a vault value of **14**. Filling them again with a
4-second settle between the calls produced exactly `avikalpa` / 14, repeatably.

**So the verb returns before its typing has landed.** It types with real key
events (that is the point — a synthetic `value` set does not survive React or
ASP.NET validators), but the reply comes back on dispatch rather than on
completion, so a second fill starts while the first is still emitting and the
keystrokes cross fields.

⛔ **The reply is the LIE-OF-SUCCESS shape**: both calls answered
`"is_trusted": true` with no error, and the `"matched": false` field says
nothing about the damage. Nothing in the envelope indicates the field now holds
something other than what the vault holds.

**Why it matters more than a typo.** A password field silently gaining a
character is an authentication failure the user reads as a wrong password — and
on portals that lock after N attempts, three scripted retries burn the account.
It also defeats the one defence the co-browse plane has, which is reading the
page-side effect back: the read itself races the fill, so a readback taken too
early reports a length that is neither the old nor the final value (15, then 16,
then settling at 14).

**Owed:**
1. `web do fill` / `web fill-vault` / `web fill-card` must not answer until the
   last keystroke has been dispatched to the page — the verb owns the
   completion, not the caller's `sleep`.
2. Until then the envelope should carry the resulting field LENGTH so a caller
   can compare without a second round trip that races.
3. ⚠ Do not "fix" this by going back to setting `.value` directly; the trusted
   typing is load-bearing (`web do click` refuses a zero-size input for the same
   family of reasons).

**Interim recipe for anyone filing a form:** one fill per call, settle ~3-4 s
between them, and read back the LENGTH against the vault's own
(`ychrome-vault get <item> <user> --field password | tr -d '\n' | wc -c`).


## eMudhra's video-KYC recording timer never advanced — NOT explained by the hang

**Status:** OPEN

The residual of the `getUserMedia` hang, which is fixed (git: "the camera ask
the engine never raises"). The hang was real, root-caused and closed; **it does
not account for what the user actually saw**, and this entry exists so nobody
assumes it did.

**What he saw, 2026-08-02.** Doing the eMudhra video KYC for his eSign
enrolment, in a surface he was looking at, with the camera preview live and his
face in it. He pressed START RECORDING and the `0:00 / 0:40` timer never
advanced, so STOP never armed and the recording could not be submitted. He
finished the KYC in Helium (Chromium). **A statutory identity verification
failed in our browser and worked in someone else's.**

**Why the hang does not explain it.** The hang needs a surface WebKit has not
presented; his was presented, which is exactly why his preview was live.

### Hypotheses already falsified — do not re-derive these

| Hypothesis | What killed it |
|---|---|
| A missing codec / `MediaRecorder` gap | On a presented surface it records: 5 chunks / 21.6 MB in 5 s (vp8), and 345 KB in 4.9 s off a 640x480@30 track in the other lane's run. `isTypeSupported` answers true for webm, vp8, vp9, mp4. |
| A dead or unreachable camera | `getUserMedia` resolves in 215 ms with `video:Integrated Camera (V4L2)`. Audio+video resolves too. |
| He switched away mid-recording, and hiding the surface stopped it | **Measured false.** A recorder already running keeps producing at the same rate while its window is hidden: ~2.1 MB/s before, during and after, `track.readyState:"live"`, `muted:false` throughout. |
| A lost per-origin grant | The grant DOES persist — a fresh `emudhradigital.com` surface answered `verdict:"allow"` from ychrome's remembered decision. |
| `permissions.query({name:'camera'}) == "prompt"` means something | It reads `prompt` on the arm that WORKS too. |

### One measured fact worth carrying into the next attempt

**Our vp8 bitrate is enormous: ~2.1 MB/s at 640x480 (~17 Mbit/s).** A 40 s KYC
clip is therefore ~85 MB. That is not a timer explanation, but it is a plausible
SUBMISSION explanation, and it is the kind of thing a site with an upload cap
rejects without saying so. Chromium's default for the same constraints is one to
two orders of magnitude smaller.

### What to do next — with a camera, on the real page

⛔ **Do not open a fresh investigation from telemetry.** This needs the actual
site. The one probe that would settle it: on a PRESENTED surface at
`emudhradigital.com`, start the recording and watch, in this order — whether
`MediaRecorder.state` reaches `"recording"`, whether `ondataavailable` fires,
whether the `<video>` element's `currentTime` advances, and whether the page's
own timer is driven by `requestAnimationFrame` (which a compositing stall would
starve) or by a plain interval (which nothing here would touch). The answer is
in which of those four stops first.

⚠ The user completed the enrolment elsewhere, so this is not blocking him. It is
open because "the browser we ship could not do a video KYC" is worth closing
properly, and because the next person to hit it should start from the table
above rather than from `getUserMedia`.


## The launcher OFFERS the GUI host's apps for a row on another machine

**Status:** FIXED IN CODE — LIVE PROOF OWED

**The observation owed:** on a daemon+GUI carrying this fix, right-click a
`remote-cc://dev/…` row and see dev's own apps in the menu — specifically
`yggdrasil-maker` (installed on dev, absent on the GUI host) PRESENT, and the
GUI host's own `yrdp` ABSENT. The GUI reads `RemoteMachineSnapshot::apps`, which
only a daemon can fill, so this needs a daemon handover as well as a GUI swap;
the GUI host went off the network before either could be done, and the binary is
deployed and waiting.

**The remote half IS proven** (2026-08-02, new binary installed at
`~/.yggterm/bin/yggterm` on both dev and oc):

```
dev: {"name":"ychrome",…} {"name":"yedit",…} {"name":"yggdrasil-maker",…}
oc : {"name":"ychrome",…} {"name":"yedit",…} {"name":"yrdp",…}
```

Two machines, two different registries, each its own — and `yggdrasil-maker` is
exactly the app that a right-click on a dev row could never show before. The
back-compat path is proven the same way: oc on its OLD binary answered
`Error: unsupported server command: remote`, a clean refusal, which is what
makes `fetch_remote_machine_apps` return `None` and keep that machine's previous
list instead of blanking its menu.

User-reported 2026-08-02: *"why does ychrome only launch on guihost even if I right
click on dev or oc sessions. This is undesired behavior."*

⛔ **The first filed root cause was HALF WRONG, and the wrong half is the one
the title carried.** It claimed the right-click "offers, and launches, the GUI
host's ychrome". The OFFER half is true. **The LAUNCH half is false**, and
believing it would have sent the next session to rewrite a launch path that is
already correct.

**What the repro actually proved** (2026-08-02, guihost 2.12.24 + dev):
`terminal_launch_context_for_row` resolves a `remote-cc://dev/…` row through
`remote_machine_for_sidebar_row`, which falls through to `row.host_label`
(`"dev"`), finds the machine, and returns
`Remote { ssh_target: "dev" }` — locked by
`a_remote_row_offers_its_own_machines_apps_and_launches_there`. Driven live: a
session created on dev, `echo MACHINE=$(hostname)` → `MACHINE=dev`, then the
manifest's own command typed in → `ychrome` running **on dev** (pid on dev, not
on the GUI host), which then declared its surface up the ssh chain and
`web ensure` on the GUI host answered
`rebuilt_from_daemon_declare: true, tabs: 1`. The remote path works end to end.

⚠ **The first repro was a false negative and is worth remembering.** It launched
bare `ychrome`, which stops at the profile picker, and the picker declare is
`("web-surface", "pick") => Retention::Ignore` — the daemon deliberately never
retains a prompt awaiting a human. So `web ensure` answered `no_declare` for a
reason that had nothing to do with the machine, and on a `--no-activate` session
there was no mounted xterm host to parse it live either. **A discriminator that
answers the same way on the working host is not a discriminator.** Re-run with
`--profile <name>` so the real `open` declare fires.

**What was actually broken.** `cached_app_registry()`
(`crates/yggterm-server/src/lib.rs`) scans the DAEMON'S OWN home, and every
launcher surface read that one list for every row. So the menu beside a dev row
was drawn from the GUI host's registry: an app installed only on dev never
appeared, an app installed only on the GUI host was offered for execution on
dev, and the manifest's ABSOLUTE `binary` path — which by contract means
something only on the host that wrote it — was the path typed into dev's PTY.
On this fleet the paths happen to coincide, which is exactly why it looked like
it worked and read as "it always launches guihost's ychrome".

That is the single-source-of-truth mismatch `CLAUDE.md` forbids: "which apps
exist" was keyed to a HOST while "where does this session run" is keyed to a
MACHINE, with nothing making them agree.

**The fix.** A machine reports its own registry (`server remote apps`, one
manifest per line, pruned by the same scanner the local host uses), the daemon
fetches it on the existing refresh and stores it on `RemoteMachineSnapshot::apps`,
and the GUI resolves "which apps does this row have" through
`app_registry_for_row` — which calls **the same `remote_machine_for_sidebar_row`
that decides where the launch runs**. Offer and execution are now two readings of
one fact. `resolve_app_verb_for_row` closes the other half: a clicked entry
resolves against the registry it was drawn from, so a remote app's menu item can
never be a silent no-op. A failed fetch keeps the machine's previous list
(`None` ≠ `Some(vec![])`), so one flaky ssh round trip cannot blank a host's menus.

⚠ Adjacent, do not conflate: the ychrome ENGINE not existing off the GUI host is
a separate, already-closed ychrome entry.

⚠ **Still open and NOT this entry:** the browser surface itself always renders in
the GUI host's process, so a dev-launched ychrome uses the GUI host's web
profiles and cookie jars while its vault/settings panes come from dev's ychrome
daemon. That split is architectural, was not reported, and needs the user's call
before anyone "fixes" it.

## A ychrome session whose last tab is closed should close itself

**Status:** OPEN

*(the yggterm half is built and locked; the ychrome half is not, and the item
does not close until both are live)*

User-reported 2026-08-02: *"In a specific ychrome session, if all tabs are
closed then ychrome session itself should close itself."*

Today the ychrome CLI keeps its session alive after its last tab goes, leaving a
row that owns nothing — the inverse of the settled rule that a row with no
runtime is fine and desirable (`docs/settled-calls.md` call #4). That rule is
about a row the USER can click to restart; this is a live session with a live
process and nothing to show, which is different and is clutter.

**The yggterm half, built 2026-08-02.** `WebSurfaceUiState::last_content_tab_closed`
is latched in `web_surface_close_tab` — the ONE removal path, so every close verb
reports it by construction and no bulk close has to remember — and cleared by the
next tab that opens, including a popup and an undo. It rides the `/ping` the GUI
already sends per session as `&last_tab_closed=1`, on every ping while set rather
than once, so a dropped tick costs nothing and the app owes no acknowledgement.
An app that does not know the param ignores it like any unknown query value.

⛔ **It is a latched EVENT and must never become the count `tabs.len() == 1`.**
A surface holds nothing but its app tab in the window between the app declaring
and its first page arriving, so a signal derived from the count would order every
ychrome to quit at launch. Locked by
`closing_the_last_content_tab_signals_the_app_but_having_none_yet_does_not`,
whose second half is exactly that case, and mutation-proven by removing the latch.

**What ychrome still owes.** Its `/ping` handler (`src/daemon.rs`, the
`request.path == "/ping"` arm) reads `session` and `ack` from the query and must
also read `last_tab_closed`, recording it on that `SessionEntry`. The view
client's `drive_surface` loop already talks to the daemon every ~4 s through
`declare_current`, so that reply is where the answer belongs: on seeing it, the
loop sets its `stop` flag and falls into the shutdown it already has
(`emit_close` → `deregister` → the `close` OSC), which is precisely what Ctrl+C
does. Nothing new needs writing on the teardown side.

⚠ Do not implement this by having the app poll for tabs — that is a second
encoding of a count the GUI already owns.

⚠ Live proof owed for the whole item, and it needs both halves plus a GUI swap.


## Two supernumerary daemons persist holding unmigratable local:// shells

**Status:** OPEN

**Two supernumerary daemons persist** holding unmigratable `local://` shells.
That is the durable half of the chaining bug, still open.
- ✅ **The vault agents on dev and guihost are current and unlocked (2026-07-31).**
Neither needed an unlock in the end. guihost was already satisfied; dev's binary
predated the `socket` field that the card-fill path's socket lookup reads, so
it was rebuilt, installed, and moved across with `ychrome-vault handover` —
the unlocked session hands to the new binary instead of re-locking, so the
refresh cost ZERO unlocks. Both now report `agent_stale:false`,
`state:unlocked`, `undecryptable:0`, `socket:…/vault/agent.sock`, and both
agree at 1116 items (dev resynced from 1115 on the handover).
⚠ The handover verb is the way to refresh a vault binary. Do NOT
`stop-agent` for a version bump — that re-locks and costs the user an unlock.
- ✅ **The five could-only-pass locks are all closed.** The last one — the
web-surface reclaim family, where reverting all four production call sites
left the suite green — is replaced by `shell::web_surface_reclaim_locks`,
eight tests that drive `web_surface_reclaim_background_pass` (the function the
reconcile loop calls) through a fake host, plus a structural lock on the loop's
own argument list. Twenty-one mutations, one per production call site, each
proven RED and restored. Field guide §7.1 has the shape.
⚠ **Not live-verified on guihost** — this was a test-discipline lane, product
behaviour is unchanged and the deploy happens separately.
**The habit stands regardless: before trusting ANY test in a report, mutate
the production call site yourself.**


>
> Earlier rounds closed: the tab rail becoming the cwdtree with folder icons,
> nesting, the drag gesture and the density pass; Cloudflare challenges;
> userscripts not injecting and the YouTube 2x-ads symptom; adblock/SponsorBlock
> exhaustiveness; open-webui sidebar switching; fullscreen chrome over the
> picture; tab placement and the row menu; the mis-clicked hidden duplicate;
> `ychrome-vault totp` on a skewed clock; the two HTTP caches; the frosted
> close-button chip; and background tabs destroyed on a clock.

**TWO functional items remain, plus one design call and a live render batch.**


## A degraded profile cannot be made genuinely READ-ONLY

**Status:** OPEN

**A degraded profile cannot be made genuinely READ-ONLY — DESIGN CALL, decided
2026-08-01.** The silence half is done (`WebSurfaceJarMode` owns the decision,
the spelling and the notice). WebKitGTK has no read-only jar, so "genuinely
read-only" means giving the loser a COPY of the profile's cookies — and the
objection was that every agent shadow surface would then duplicate the user's
live session cookies to a second place on disk, in a browser carrying
brokerage sessions.
**Decision: option 2, narrowed — copy the `cookies` file ONLY, into a scratch
dir wiped at teardown, and ONLY for a surface the USER opened; an agent shadow
surface keeps today's jarless behaviour and its notice.** That fixes the
reported symptom (a second surface on a held profile stays logged in for its
life) while removing the objection outright, because the shadow path was the
whole exposure. A startup sweep clears crash leftovers.

### ⭐ THE RENDER-PIPELINE BATCH (user, 2026-08-01) — untouched for a long time

The user's words: *"gating sessions have become ridiculously buggy. We have not
touched the rendering pipeline for a long time and bugs have piled up."* Three
symptoms, and the last two are strongly suspected to share a root:

1. **The stuck viewport after a copy.** Selecting and copying in a session
 leaves the viewport pinned: it shows **"2 new messages (ctrl+End) ↓"** while
 output keeps arriving and never follows it. Switching sessions unsticks it —
 which points at a remount clearing state that nothing else clears. Suspect
 the follow-prompt / user-scrollback guard treating a selection (or the scroll
 a selection causes) as sticky and never releasing after the copy completes.
 Screenshot also shows `sent 50 chars via OSC 52`.
2. ✅ **Claude Code ALWAYS starts with a broken bottom**, plus glyph corruption
 while switching into CC sessions. A TUI refresh fixes it every time — so the
 daemon's screen is right and the CLIENT is painting less than it holds.

 **ROOT-CAUSED AND FIXED IN CODE 2026-08-01 (`lane/dev/render-pipeline`). It
 is the GATE — the user's two complaints were one bug.** The handover veil
 does not merely cover the viewport; while `handoverPaintSuspended` is true
 the host does *no visible paint at all*. On release it used to run
 `requestVisiblePaint(false)` — a damage-tracked partial paint. Every row the
 read loop wrote during the veil is already in the buffer with its damage
 consumed, so the resume presents **less than the client holds**. That is the
 broken bottom, and it is DETERMINISTIC ("always") because the gate arms on
 `preserved_terminal_owner_count > 0`, a steady state, so every mount goes
 through it.

 The glyph half is the same line. `clearTerminalTextureAtlas()` lives *inside*
 the forced-refresh branch of the visible-paint funnel, and its own comment
 already names the symptom: while a window is backgrounded the WebGL glyph
 atlas goes stale, so a switch-in that does not force a refresh "paints cells
 against a stale atlas -> wrong-glyph garble". A non-forced resume skips the
 heal. **One dropped `forceFullRefresh` produces both halves.**

 Compounding it, `requestVisiblePaint` checked `handoverPaintSuspended` and
 returned *above* the `pendingVisiblePaintForceFullRefresh` latch — the one
 thing that survives coalescing — so a full refresh demanded during the veil
 was DESTROYED, not deferred. The drop site's comment claimed "the resume path
 repaints from the daemon's own bytes"; the resume path deliberately does no
 daemon replay (field guide §5) and passed `false`. Two sites owned "who
 repaints after the veil" and they disagreed.

 **Live evidence on guihost 2.12.22, the user's own GUI (pid 2094127):**
 `daemon_handover/handover_paint_suspended` → `handover_paint_resumed` at
 16:12:44→16:14:15, 16:28:37→16:30:09 and 18:03:10→18:04:41 — three windows of
 **~91 s each in which terminals painted nothing**, every one released by
 `resumed_timed_out` (the 90 s `suspend_ceiling_ms`), every one with
 `fingerprint == resolved_fingerprint == pid=2050347:2.12.22`: **same daemon,
 same version, no update in flight.** Reproduced on a shadow client
 (`agent-render`, pid 2184903, 18:00:52) — screenshot shows the veil over the
 viewport beside a rail reading Client 2.12.22 / Daemon 2.12.22 / uptime 2h23m
 / "5 owned · 9 total · 4 preserved".

 **The fix, two edits in the terminal host script:** latch the full-refresh
 demand *before* the suspension can return (drop the FRAME, never the DEMAND),
 and resume with `redrawTerminal('handover-paint-resume')` — the exact repaint
 the user performs by hand, atlas clear + `term.refresh(0, rows-1)` over the
 CLIENT's own buffer. It is **not** a daemon-screen replay, and it is
 deliberately **not** gated on output silence: an agent CLI is never silent
 (see §THE PATTERN BEHIND THREE SEPARATE BUGS below), and this is not
 speculative correction — it is the settle of a window we ourselves blanked.
 Locks: `a_suspended_host_defers_a_full_refresh_demand_instead_of_destroying_it`
 and `a_handover_paint_resume_redraws_the_whole_client_buffer`, both red-proven
 by restoring the two production statements.

 ⚠ **Two things still owed.** (a) **Not live-verified** — guihost runs 2.12.22,
 which predates both this and the false-gate arming fix (`c88324e`, on main,
 undeployed). After the next deploy, confirm by opening a CC session and
 grepping the trace for a manual-redraw with reason `handover-paint-resume`,
 and confirm the veil no longer arms on a steady preserved-owner count.
 (b) `c88324e` stops the *false* arming; this fix is still required, because a
 REAL handover would leave exactly the same broken bottom without it.

3. ⚠ **YouTube frame judder with "overlaps"** while YouTube's own stats-for-nerds
 reports almost no dropped frames. ⚠ That reading FALSIFIES the decode
 explanation: if frames are not being dropped, the decoder is keeping up and
 the fault is in PRESENTATION, not decode. "Overlaps" reads as stale frame
 content persisting, i.e. damage/compositing, not pipeline. The
 `GST_PLUGIN_FEATURE_RANK` default shipped in 2.12.22 is still correct on its
 own merits but is NOT the explanation for this.

 **NOT ROOT-CAUSED. It does NOT share a root with symptom 2** — that was the
 working hypothesis and it is dead: symptom 2 is deterministic and lives in
 the handover veil, which is off most of the time. What the 2026-08-01 pass
 found instead is a real, previously-unread presentation-layer suspect, and
 what it eliminated:

 ⚠ **STILL LIVE ON 2026-08-11, ten days later.** The GUI host's trace carries
**32 `app_render_storm` detections and 17 autopsies**, the sampled one at
**4,661 renders in 60 s = 77.7/s** — same order as the 2026-08-01 measurement,
so nothing since has touched it. Found while investigating the TUI glyph-garble
entry above; not otherwise re-examined.

**`app_render_storm` is live, large and unexplained.** On guihost 2026-08-01 the
 Dioxus root rendered at **85–118 renders/s for a continuous 30 minutes**
 (16:57:40→17:27:40) and in 202 one-minute windows across the trace, against a
 calm baseline of 0.8–0.9/s. Measured cost while it ran at 33–47/s: the GUI's
 **main thread at ~42% of one core** (`/proc/<pid>/task/<pid>/stat`, 2 s
 deltas — not the `ps` lifetime average, which lies). That thread is the GTK
 main loop, which is where the UI process composites every web surface's
 DMABuf, so it is a plausible mechanism for frames that decode on time and
 *present* late or twice. **Plausible is not proven — nothing here measured a
 frame.**

 **The autopsy has been shipped since run 4 and was never read (see §Residual
 threads). It has now been read, and it answers its own discriminator:**
 `forced_wakes: 0`, `unattributed: 506–510 of 512`, `shellstate_mut: 1–6`. Per
 the arm site's own comment that means **NOT a caller of ours over-scheduling**
 — do not go audit `schedule_update` call sites, that lane is closed.

 Eliminated, each with the measurement that killed it:
 - **Terminal output forwarding** — 2.0 forwards/s while the root rendered at
   85/s. Decoupled.
 - **`safe_shell_mut` / any ShellState field** — 1–6 mutations per 512 renders.
 - **The handover veil** — 191 of 202 storm windows fall outside every paint
   suspension (19% storm rate inside vs 5% outside: enriched, not causal).

 Left standing: an app()-scope `use_signal` written outside `safe_shell_mut`,
 or a Dioxus-internal wake (a task/eval/future resolving every frame).
 ⚠ **The instrument cannot currently tell those apart, and its blind spot is
 load-bearing:** `FORCED_WAKE_TOTAL` only wraps the `schedule_update()`
 closure app() hands to its own 21 callers, so "forced_wakes: 0" means "none
 of *our* 21 asked" — it can never see a Dioxus-internal wake. Next step is to
 widen the autopsy (per-`use_signal` write attribution, or a Dioxus scope-wake
 hook), not to guess. Strongest correlate to chase first:
 `terminal_mount/forward_protocol_only_output` runs **75× higher** during
 storms (15.1/min vs 0.2/min) while `terminal_io/dispatch` is flat.


## A HOVER-REVEALED CONTRIBUTED RAIL PANE DRAWS ITS HEADER AND NONE OF ITS ROWS

**Status:** OPEN

**A HOVER-REVEALED CONTRIBUTED RAIL PANE DRAWS ITS HEADER AND NONE OF ITS ROWS**
(found 2026-08-01 while live-verifying the hover-reveal context-menu fix; NOT
caused by it — reproduced identically on the deployed 2.12.23 binary and on
the fixed build, on two separate shadows). Open a yedit session so the rail
shows its contributed `notes` pane: docked it has 27 `[data-app-pane-row]`
rows; hide the rail and hover-reveal it and the card reads `notes` with
**zero** rows and 7 nodes of content total. The reveal resolves the right MODE
(`right_panel_reveal_mode`) but the pane's schema is not there to render.
Consequence for verification as well as for the user: the one rail surface
with right-clickable rows cannot be exercised in the hidden+revealed state at
all, which is why `rail_autohide_pinned`'s new menu term is unit-proven but
not yet live-proven end-to-end.


## ⚠ yRDP: "open yRDP here" may open on guihost instead of the row's host

**Status:** OPEN

User-reported 2026-08-02: *"all working fleet sessions' right click context menu
and open yRDP here was opening yRDP in guihost instead of that host. I do not know
if this issue is fixed or not."*

Unverified either way — recorded so it is not lost. What is known so far: the
`yRDP` string in `shell.rs` around 152610 is a **test fixture**
(`/home/gui-host/.local/bin/yrdp`), not the live path. Real dispatch goes through
the manifest-based per-host app registry via `resolve_app_verb_for_row`, which
does take the row, so the plumbing to honour the row's host exists.

**What to check:** whether the manifest the verb resolves to is the ROW host's or
the GUI host's. "Always opens on guihost" is exactly what resolving against the GUI
host would look like, and guihost is the GUI host.

Reproduce on a `dev` or `oc` row from the guihost GUI, then compare against the same
verb invoked on a guihost-local row.

## ⚠ FEATURE: the document split gutter does not drag yet

**Status:** OPEN

User-requested 2026-08-02, as an end-to-end test of the
yggterm ⇄ libyggterm ⇄ yedit ecosystem: *"In Markdown/text split mode the split
bar should be in the center of the viewport, and should be draggable by the user
or agent via yggui as they feel fit."*

**Half of it is built** (`lane/dev/yedit-split-gutter`). The split view no longer
hardcodes `flex:1 1 50%` with a `border-right` pretending to be a bar. There is a
real gutter element carrying the `yggui_contract::document_split_stamps`, the
halves are sized from `AppPaneSchema.split_ratio`, and absent a declaration it is
centred. libyggterm `v0.2.0` owns the stamps, the centred default and the clamp —
the clamp lives there because a host and an app that disagree about the minimum
produce a gutter that snaps back under the pointer.

**What remains:**

1. **Drag.** This codebase does pointer drags with Dioxus handlers, not injected
   JS (see the `onpointerdown` around `shell.rs:86063`). Needs
   pointerdown/move/up, a live ratio signal so the halves track the pointer, and
   pointer capture so a fast drag does not escape the gutter.
2. **Report the release back** as an action so the app can persist the ratio —
   the app owns the value, the host only reports the gesture.
3. **`yedit` must emit `split_ratio`** and persist it; today every split still
   opens centred because nothing declares one.
4. **Agent path via yggui** — the stamps exist so an agent can drive the gutter
   exactly as a pointer does; that path is unproven.

⚠ Not live-verified. The gutter compiles and is a strict improvement over the
border, but no screenshot has confirmed it renders where intended.

## ⚠ TOOLING: agents have no first-class access layer for DELEGATE sessions

**Status:** OPEN

Feature debt, user-requested 2026-08-02: *"easy for you (agents) access layer
on yggui automations"*.

The delegate-session pattern (a guiding agent launches an interactive
CC/codex session in a yggterm row for the user to answer) is now the standing
work pattern, and most steps of it are still hand-rolled: a pending
AskUserQuestion is invisible off-screen (JSONL gets it only when answered;
client read-buffer is blank for never-activated rows), row order is only a
debug field, and `terminal send`'s `accepted:true` cannot see whether the old
child still owns the PTY. Ranked, costed feature asks:
**[`docs/agent-bg-sessions-dream-2026-08-02.md`](agent-bg-sessions-dream-2026-08-02.md)**.
Interim recipe + traps: data-fabric skill §THE BG-SESSION PLANE.

✅ **Ask #1 CLOSED 2026-08-06** — the LAUNCH itself is first-class now:
`terminal new --kind claude-code|codex` takes `--model`,
`--permission-mode` (per-launch, never the global setting) and `--prompt`,
so a delegate no longer inherits the user's default model and no longer needs
the `--kind shell` + printf workaround. The remaining asks above are the
OBSERVE and STEER halves, which is why this entry stays open.

## ⚠ TOOLING: `app open --view preview` never settles, so a Web View change cannot be verified through the documented instrument

**Status:** OPEN

Found 2026-08-03 while live-proving the Web View rework, on guihost 3.0.0, against
a shadow client on the new binary.

`server app open <session> --view preview` **always** exits non-zero with
`ready:false, reason:"preview surface not mounted"`, on every session tried
(stored `.jsonl` rows, live `remote-cc://` rows, a plain shell), and it also
reports `last_error=timed out waiting for app control response … after 750 ms`.

**Falsified, so this is narrow and not a general app-control failure.** The same
verb with `--view terminal` on the same session, same client, same second, DID
switch the row (`ACTIVE: remote-cc://dev/a033a728-… Terminal`). And a second
`--view preview` call against the row that was ALREADY active DID flip the mode
(`… Rendered`) — so the switch itself works; it is the READINESS GATE that never
closes.

**Where it is.** `terminal_observe.rs` refuses `ready` when
`dom.preview_scroll_count == 0 && document_editor_count == 0`.

### ⚠ THE DIAGNOSIS ABOVE WAS HALF WRONG — corrected 2026-08-03 (later)

This entry used to say the DOM "returns **no `preview_*` keys at all**" and that
it was therefore **NOT** the same bug as the `dom_debug_snapshot_timeout` entry
below. Both halves are false, and the correction is worth carrying because the
wrong half sent a session looking for a stamp that had never gone away:

- **The DOM publishes `data-preview-scroll="1"` to this day.** Measured on the
  live GUI host with `dom-eval`:
  `{any_preview_scroll: 1, values: ["1"], strict_count: 1}`.
- **The keys are absent because the SNAPSHOT is absent.** A timed-out capture
  returns `{"error": "dom_debug_snapshot_timeout"}` and nothing else, so every
  `preview_*` key is missing together — which is exactly what "no preview keys"
  looked like. It IS the entry below.

**FIXED IN CODE — the gate no longer lies.** `preview_scroll_count` was read
through `.unwrap_or(0)`, which turned a FAILED MEASUREMENT into a confident
claim about the page. It now answers
`preview readiness unmeasured (dom_debug_snapshot_timeout)`. That does not make
the verb settle — the underlying timeout below is still the blocker — but the
verb now names the instrument that failed instead of accusing the surface.

**What is still OPEN here** is therefore only the timeout below. Fix that and
this verb settles.

⛔ **`dom-eval` was never the dead instrument either.** The note that it
"returns null even for the control `dom-eval "1+1"`" was a mis-written control:
the verb takes a **`return`-style body**, so `1+1` is an expression statement
evaluating to `undefined`. `dom-eval "return 1+1"` answers `{"result": 2}`.
Every DOM question in this file can be answered today.

## ⚠ TOOLING: app state's DOM debug snapshot times out on guihost, so every DOM

**Status:** OPEN

**⚠ TOOLING: `app state`'s DOM debug snapshot times out on guihost, so every DOM
probe field is unreadable there** (found 2026-08-01 while verifying the yedit
gutter). `dom_debug_snapshot_timeout` comes back on BOTH a shadow client and
the user's own GUI, and it takes the pre-existing `document_editor_count` with
it, so this is not new and not caused by any one lane. The cost is that a
field wired into the snapshot cannot be verified through the documented
instrument — the gutter's `document_wrap_gutters` had to be proven through
`dom-eval` instead. **Fix the timeout, or `app state` quietly stops being the
probe the field guide says it is.**

✅ **STILL TRUE AT 3.0.139** — `degraded_reason: dom_debug_snapshot_timeout` on
the live GUI, with `document_editor_count` absent.

⚠ **BUT THE TITLE OVERSTATES IT, AND THAT MATTERS MORE THAN THE CONFIRMATION.**
*Every* DOM probe field is NOT unreadable: **66 `dom` keys came back in the same
reply**. The snapshot degrades and drops NAMED fields; it does not go dark. An
agent reading the headline abandons a working instrument and goes to `dom-eval`
for things `app state` would have answered — which is a second cost the entry
was creating rather than describing.

⇒ Read `degraded_reason` and then **check for the field you actually want**; its
absence is the signal, not the presence of the timeout.

**Falsifier:** with `degraded_reason` set, count the `dom` keys — if it is not
near zero, the loss is specific and can be named.


## ★★ THE APP ACTION POST DOES NOT NAME ITS SESSION, AND THE DOCUMENT CHANNEL IS

**Status:** OPEN

**★★ THE APP ACTION POST DOES NOT NAME ITS SESSION, AND THE DOCUMENT CHANNEL IS
SESSION-SCOPED (oc, 2026-08-01). Cost: a 2.5-hour silent hang in yRDP.**
`document_pane_run_action` (`shell.rs:49802`) and `app_pane_run_action`
(`shell.rs:50203`) both send `{"pane", "action", "values", "value_keys"}` —
**no session** — and `app_pane_schema_url` adds none either. Yet the document
channel is session-scoped on OUR side and says so in its own doc comment: the
GUI resolves `control_url` *from* `session_path`
(`sidebar_control_url(&session_path)`), fetches per session, and applies the
reply per session. We know exactly which session acted and decline to say.
**What it costs an app.** A libyggterm app whose daemon is per-HOST but whose
view clients are per-SESSION cannot address its own answer. yRDP declares
`{"session": …}` on the OSC, does the work, and then has nowhere to send the
result: the connect outcome was filed under `""` while the client polled
`/events?session=<its id>`. The operator watched **"Connecting"** for two and a
half hours with the guest up, the RDP session live, and the viewer URL built
and finished in a mailbox with no reader. **No error, no log, nothing wrong to
find** — and the placeholder text ("a guest that is not running is started
first") actively pointed at the wrong subsystem.
**The fix**: echo the session the app declared back on the wire — the pane
fetch and the action POST of the document channel at minimum. It is page
context exactly like `host`/`zoom`/`secure`, which we already send, and it is
the one piece of context the app cannot derive.
⚠ **Every future libyggterm app with a durable per-host daemon hits this**, and
it fails SILENTLY and looks like the app's bug. yRDP now works around it by
registering clients on their own poll and refusing by name when it is genuinely
ambiguous (`daemon.py:route` in github.com/yggdrasilhq/yRDP, and §5 of its
`docs/architecture.md`) — that fallback is written to go cold the moment this
is fixed, so it does not need removing first.


## ★★ "YCHROME SUDDENLY QUIT TO TERMINAL"

**Status:** OPEN

**★★ "YCHROME SUDDENLY QUIT TO TERMINAL" — a fleet binary deploy arms a
refuse-exit landmine (user-reported 2026-07-30 night; live-diagnosed and
CURED on the live host, the DESIGN fix still owed).** Deploying a new
`ychrome` binary makes the RUNNING daemon stale; `ensure()` then "refuses
loudly on stale+busy" (round 27) — which means **every fresh `ychrome
<url>` invocation exits immediately** until someone runs `ychrome daemon
restart` by hand. To the user that is "ychrome suddenly quits to the
terminal", hours after an agent deployed binaries. Compounding it: two of
the live host's ychrome rows were LOCAL-VARIANT placeholder wedges (the
same class as the remote-row wedge below — planning banner, launch never
fired), so their relaunches went into dead PTYs. What cured it live:
`ychrome daemon restart` (honest handover, surfaces re-registered), then
cycling each session's CLI onto the new binary (`routable=yes` returns; a
session with no saved page comes back at the profile picker — honest).
**The design fix owed:** a new invocation against a stale-but-busy daemon
should ROUTE into the running daemon with a one-line stale warning — the
old code is still serving perfectly well — and retirement should happen on
the user's schedule, never as a precondition for opening a page. A refusal
is only honest when routing is genuinely impossible.
⚠ Related trap for agents: after ANY ychrome fleet deploy, the daemon on
every GUI host is stale by definition — hand it over as part of the deploy
(clients and daemon together per the round-29 mixed-version note), or the
user hits the landmine.


## ★★★ REMOTE ROWS WEDGE IN RemoteBootstrap AFTER A DAEMON VERSION HANDOVER

**Status:** OPEN

**★★★ REMOTE ROWS WEDGE IN `RemoteBootstrap` AFTER A DAEMON VERSION HANDOVER
(found live on the 2.12.18 → 2.12.19 bump, 2026-07-30 night).** After the
GUI+daemon swap, every `remote-cc://` / `remote-session://` row the new
daemon adopted sat permanently on the 5-line planning placeholder ("Queue
remote Yggterm resume … Daemon PTY: request main viewport terminal stream"):
the adopted row's launch request never fired, so no ssh chain was ever
spawned for the new owner, while the OLD daemon's viewer chain kept
streaming into a grid nobody views. 15 rows wedged; the user-facing symptom
is a blank viewport on every remote agent row — the product's core handoff,
dead until manual recovery. **Recovery that works, verbatim, per row:**
`yggterm-headless server terminal restart '<session>'` (all 15 accepted,
all reached `Running` with real content; a freshly re-attached CC repaints
fully on its next input/output — nudge an idle row with a bare Enter).
Root-cause candidates to investigate BEFORE the next bump: the adopted
row's `request_terminal_launch_for_path` queue never draining for
preserved-owner rows, and the one-viewer contract holding dev-side slots
for the old daemon's chain. ⚠ **Do NOT bump dev/oc daemons until this is
fixed** — their next version transition hits the same wedge, and
fleet-binary-sync may carry 2.12.19 binaries there on its own.
⚠ Related, observed in the same window: `server snapshot` on the NEW daemon
shows the placeholder for preserved-owner rows (the documented
snapshot-lies trap), and `update-daemons --force` PRESERVES runtimes on the
old socket rather than migrating them — neither is the recovery; the
per-row `terminal restart` is.

### ⚠ NOT REPRODUCED on 3.0.1 → 3.0.2 (guihost, 2026-08-03) — do not assume it is live

A GUI relaunch onto a 3.0.2 client spawned a 3.0.2 daemon (pid 1657885) beside
the 3.0.1 owner (1325229), unintentionally — see the correction below. It is
the cleanest observation of this transition anyone has recorded, and **nothing
wedged**:

| Check | Result |
|---|---|
| `claude` processes on the remote host | **47 alive**, one per session sampled |
| Remote rows re-attaching AFTER the handover | ssh chains for `90a6a23f` and `ddbdb609` created at 14:40, i.e. post-swap — so the launch request DOES fire for an adopted row |
| Sessions lost | none |

⛔ **The reading that looks like the wedge is the snapshot-lies trap.** Every
`remote-cc://` row reports `launch_phase: RemoteBootstrap` when you ask the
daemon that does NOT own it — `apply_terminal_runtime_truth_to_snapshot` stamps
that on any row whose runtime the answering daemon lacks. 16 of 16 rows read
`RemoteBootstrap` while their processes were all alive. **Ask WHICH daemon owns
the row before reading a phase as a symptom.**

So this entry stays OPEN because the 2.12.18→2.12.19 evidence was real and the
root cause was never found — not because it was seen again. Anyone re-testing
should mount a remote row in TERMINAL view after a handover (the wedge is a
terminal-viewport symptom; the Web View path was exercised here and was fine).

⚠ **A client-only deploy does NOT avoid a handover.** `resolve_client_daemon_endpoint`
documents a newer client falling back to a reachable older daemon, and that is
NOT what happens end to end: relaunching the GUI on a newer binary brought up a
daemon of its own version, which then took over the older `server-<v>.sock`
aliases. Plan a client deploy as a daemon deploy.


## ★ THE SUPERVISOR DIES WITH ITS CHILD

**Status:** OPEN

**★ THE SUPERVISOR DIES WITH ITS CHILD — confirmed twice in one day (guihost,
2.12.18, 2026-07-27, ~17:15 and ~23:10).** `kill -TERM <gui-child>` is the
documented GUI-swap recipe ("the supervisor relaunches the new binary"), but
both times the `yggterm --supervise` parent exited WITH the child and nothing
relaunched — the desktop went GUI-less until a manual
`setsid yggterm --supervise` with the desktop env re-exported. Round 26
recorded the recipe working, so either a regression or the supervisor treats
a TERM'd child as deliberate shutdown. Find the supervisor's child-exit
policy: a child that exits on SIGTERM during a binary swap must be
relaunched; only a supervisor-addressed TERM is a shutdown order. Recovery
recipe that works, verbatim: read WAYLAND/XDG/DBUS env off a live desktop
process → `setsid ~/.local/bin/yggterm --supervise </dev/null &`.

⚠ **CONDITIONALLY LIVE — and the condition is HOW THE GUI WAS STARTED, which
this entry never states. Measured 2026-08-13.** There are **two** desktop entries
on the GUI host with different Exec lines:

```
dev.yggterm.Yggterm.desktop   Exec=…/yggterm --supervise      ← supervised
yggterm.desktop               Exec=…/yggterm                  ← not
```

and the GUI actually running was started by neither: `server app launch` detaches
it, so it has **PPID 1** and **zero** processes match the supervise pattern. ⇒ On
an agent-launched GUI there is no supervisor to die, and the recovery recipe
above would put the host into a mode it was not in.

⛔ **Do not read this as "fixed".** A desktop-icon launch still takes the
supervised path, so the child-exit policy question is real for that half — it is
simply not reachable from the state an agent finds the host in. **Say which
launch path a measurement was taken on**, because the two produce different
process trees and the entry's symptom only exists in one of them.

⇒ Same stale premise as [`ygg-unwedge` being blind on the GUI host](#): that
tool searches for the `--supervise` parent, which an agent-launched GUI does not
have. One fact, two entries.

**Falsifier:** on a desktop-launched (supervised) GUI, `kill -TERM <child>` is
followed by a relaunch.


## ★★ WEBAUTHN / PASSKEYS ARE UNREACHABLE ON AN AGENT-CREATED SURFACE

**Status:** OPEN

**★★ WEBAUTHN / PASSKEYS ARE UNREACHABLE ON AN AGENT-CREATED SURFACE
(2026-07-28).** Full field report:
**[`docs/agent-passkey-gap-2026-07-28.md`](agent-passkey-gap-2026-07-28.md)**.
Written from a real deadline job (minting a Cloudflare DNS-01 token to renew
an expiring wildcard certificate). The passkey machinery is built and
correct; it is simply **never wired to a surface an agent makes**:
1. **Surface policy is bound ONCE, at `open_web_surface` time**
   (`crates/yggterm-shell/src/shell.rs:8715`). A surface built while
   `web_surface_policy_gate()` is still `Pending` gets `userscripts: []` AND
   `signer_base: None`, permanently — nothing re-fits it when the policy
   lands. Our surface's own trace says `{"policy":false,"signer":null}` on
   every tab. Result: `window.PublicKeyCredential` is **undefined**, and the
   relying party (Cloudflare) renders "your browser does not support security
   key". A human wins that race by sitting still for a second; an agent never
   does. This is why "⚠ still owed: full crypto E2E against a real relying
   party" is still owed.
2. **A hand-injected shim cannot rescue it** — `yggterm-appctl://signer` is
   not a registered scheme on such a webview (`TypeError: Load failed`), so
   the fix must be in the construction path, not in a userscript.
3. **`web ensure` silently reset a live, logged-in page to `about:blank`**
   after the 600 s lease lapsed, reporting `healed: false, leased: true`. It
   discarded a half-finished 2FA. Survived only because the cookie jar is
   per-profile.
4. The 600 s lease is **unreadable and un-renewable**; `web eval` returns
   `null` for statement-form scripts (`if (…) {…}` has no completion value),
   which makes a click that DID fire look like a failure.
Smallest fix that makes passkeys real for agents: **have `web ensure` await
`SurfacePolicyGate::Ready`** before returning.

→ **Fix built (`lane/dev/ensure-policy-gate`), awaiting live verification.**
`web ensure` now awaits the policy gate before arming a build: bounded 8 s
wait, exhausted fetches are re-armed and re-driven in the same call, and a
gate that never lands refuses with `reason: policy_gate_not_ready` naming
the gate state — never a silent unprotected build. The exhausted state is
now named (`SurfacePolicyGate::Abandoned`) instead of folding into `Absent`,
and every ensure envelope reports `policy_gate`. Items 1–2 of this entry are
covered on the agent path; item 3 (silent `about:blank` reset) and item 4
(lease invisibility, `eval` statement-form nulls) are NOT fixed. Remove this
entry only after a live passkey ceremony on an agent-created surface.


## ★★ CO-BROWSE: NO FILE-UPLOAD VERB, AND TWO SILENT OTP-READER FAULTS (2026-08-06)

**Status:** OPEN

2 of the 5 defects in this entry were fixed in-run; the 3 named below are what
keeps it open. (The count used to ride on the status line itself, which broke
`check-docs-ssot.sh` — one entry, exactly one status word.)

## A fleet update makes every IN-FLIGHT row unaddressable while it keeps running

**Status:** OPEN

Field report: **[`docs/agent-row-unreachable-versioned-socket-split-2026-08-07.md`](agent-row-unreachable-versioned-socket-split-2026-08-07.md)**.
Raised by the widgets lobe delegate on 2026-08-07 after **53 minutes** of a perfectly healthy
agent being treated as dead.

**The socket is versioned and superseded ones are not forwarded.** The CLI follows the new version
to `server-3-0-48.sock`; the row's PTY stays with the daemon holding `server-3-0-45.sock.lock`.
The new daemon has never heard of the session, so `server terminal screen` answers
**`running: false` with an empty buffer** — honestly, and about a session that is not its own.
Measured: the owning daemon (pid 3492432) was ALIVE and was the *grandparent* of the live agent;
every 3.0.x socket from 3-0-36 to 3-0-45 is a real socket with its own daemon and **only 3-0-46
was symlinked forward to 3-0-48**. The update landed mid-row (row launched ~13:22, symlinks
re-pointed 14:18).

✅ **`terminal submit` behaved correctly and is the only reason this was diagnosable** — it refused
with *unanswerable* rather than guessing *false*. ⛔ Do not turn that refusal into a boolean.

⚠ Second finding in the same report: **28 daemons on dev, 26 running DELETED binaries, oldest up
24 days.** Same mechanism compounding — an old daemon cannot exit while it owns live PTYs. The
SessionStart hook's `DELETED` count is this bug's accumulator, not cosmetic.

**Top ask:** a session lookup that MISSES must say *"this daemon does not own that session"*, not
`running: false`. §5 of the report has a one-command `strace` test that proves or kills the
diagnosis.

⛔⛔ **RECURRED WITHIN THE HOUR — see the ADDENDUM (§8-9).** Same row, 15:27-16:22, another 55
minutes. The CLI walked **3.0.45 -> 3.0.48 -> 3.0.49 -> 3.0.50 in about three hours** while the row
stayed pinned to 3.0.45 throughout. At this update cadence every long-running row is orphaned
roughly hourly; this is not a race anyone has to wait for.

⚠ **The second symptom was WORSE and must not be folded in:** occurrence 1 was unreachable but
WORKING; occurrence 2 was unreachable **and NOT PROGRESSING** (533 transcript rows at 15:27, 533 at
16:22). The socket split explains unaddressability, **not** non-progress. Capture
`/proc/<pid>/wchan`, `/stack` and `fd/1` BEFORE restarting a stalled row — the restart destroys the
evidence.

⭐ **FOUR LIVENESS INSTRUMENTS DISAGREED, and it is the deeper finding.** (i) **transcript mtime is
not progress** — a file can be touched without gaining rows, so any stall check must compare ROW
COUNT; (ii) `row-health.py` derives `WORKING` from mtime **while already computing the row count
and not using it**, and its own docstring names the fallacy (*"pgrep proves a LAUNCH, never
PROGRESS"*); (iii) `row-health.py` resolves **per-CWD, not per-session** — newest `.jsonl` in the
project dir, any `claude` pid sharing the cwd (13 share `/home/user/gh/yggterm`), so a dead row
inherits a sibling's liveness; (iv) `pgrep` counts the querying shell. **Same shape as the socket
split every time: asked about one thing, answered about another.** Only `terminal submit`'s
`unanswerable` currently keeps the law.


### ⚠ DOES NOT REPRODUCE IN STEADY STATE AT 3.0.146 — but read what that covers

A cross-daemon read now works. On the GUI host, five daemons coexisting, one of
them **200 hours and a hundred versions old**:

```
session owned by the 3.0.29 daemon   → running: true, 27 lines, 2441 chars
session owned by the 3.0.146 daemon  → running: true,  3 lines,  422 chars
```

asked through one CLI that resolves to the current daemon by default. **Different
values in the same run**, so the instrument is discriminating rather than
answering `true` to everything — and the old-daemon session is the one with real
content, which is the case that used to come back `running: false` with an empty
buffer.

⛔ **THIS IS NOT THE ENTRY'S SCENARIO AND MUST NOT CLOSE IT.** The report is about
the **mid-update instant** — a row launched at 13:22, symlinks re-pointed at
14:18, the split opening underneath a live row. What is shown above is a STEADY
state with an old owner still alive. Cross-daemon addressing having been repaired
does not establish that the handover window is safe, and the window is where the
53 minutes were lost.

⚠ **AND THE RESIDUAL IS THIS LANE'S OWN DISEASE:** the reply does not name WHICH
daemon answered. There is no `served_by` / `owner_pid` field, so a recurrence
would once again be indistinguishable from a correct answer — exactly the shape
the running-build identity work exists to end, one plane over. **A reply that
cannot name its own subject cannot be used to detect this defect coming back.**

**Falsifier for what remains:** a row launched before a version bump answers
correctly *through* the bump, and the reply names the daemon that served it.
## A POPUP-based re-auth cannot be completed on a web surface — the parent never resumes

**Status:** OPEN

Raised 2026-08-08 · **operator-confirmed by doing it in Chromium instead**.

Field report: **[`docs/agent-cobrowse-gaps-2026-08-08.md`](agent-cobrowse-gaps-2026-08-08.md)**.
Filed from another campaign row after a third-party-portal task was driven to the
last step four times and abandoned. The requirement: *"I manually removed the Anthony profile on a
chromium browser. It was ychrome browser shortcoming."*

**The shape, and it is general — not a Google bug and not a payments bug.** Any flow of the form
`window.open(reauth) → user verifies → popup closes → parent resumes` dies at the last arrow. The
re-auth itself SUCCEEDS on the agent plane (measured: password filled to a page-side length of 40
in the popup tab, `accounts.google.com/CheckCookie` reached, popup self-closes) and
**`window.opener` is correctly wired** (a same-origin popup reads `openerOrigin` fine). What never
happens is the parent's continuation: its document is replaced and every injected global is gone,
with no dialog and no error.

⭐ **Leading hypothesis, stated so it can be falsified cheaply:** on a `--no-activate` surface
`visibilityState` is `hidden`, rAF never fires, and **the opener is never sent `focus` or
`visibilitychange` when its popup closes** — so a continuation gated on any of the three cannot
run. Shimming all four page-side made the flow's own dialog lay out larger (448×80 → 560×80), so
the frozen frame clock was demonstrably affecting it; the flow still did not resume, and **the
parent's navigation wipes any page-side shim anyway.** ⇒ this class cannot be fixed from the page;
it needs a "presented" contract in the engine.

Two more defects in the same report, both costed: **(2)** when a popup tab closes, the ACTIVE tab
lands on a `no_webview` ghost and every verb answers *"web surface not live (session backgrounded
or not yet revealed)"* about a page that is alive — no verb takes `--tab`, and the recovery
(`web close --session`, which closes the ghost and re-activates the real tab) is discoverable only
by guessing; **(3)** an agent's OWN `web do click` later counts as `seat_input_on_unrevealed_surface`
and locks it out of `fill-vault`, with a refusal that prescribes revealing the surface — the one
act the detached-by-default doctrine forbids.

## `server app web fill-card` answers `matched:false` on fills that landed perfectly

**Status:** OPEN

Field report: **[`docs/agent-cobrowse-gaps-2026-08-07.md`](agent-cobrowse-gaps-2026-08-07.md)**.
Raised by the operator on 2026-08-07 after an India Post booking driven end to end on
`ychrome ctl` had to be handed to a second agent **on his own laptop** purely to pay Rs 23.

⚠ **First, the non-issue, because it was the operator's suspicion and it is worth closing:**
the fleet IS uniform. `ychrome-vault card` works on dev, and dev and guihost run a **byte-identical**
yggterm binary with an **identical `web` verb set including `fill-card`**. Nothing is missing from
dev's install.

⚖ **TRIAGED 2026-08-07 — two owners, and only one of them is this queue.**

- **Asks 1 and 2 were ychrome's** (`ctl fill-card`; `ctl fill` missing from the engine's usage
  banner). Both are engine verbs, so they lived in `ychrome/docs/pending-bugs.md`, the one answer to
  what is open for ychrome. Cross-referenced, never duplicated. **Both have SHIPPED** — see git
  there; the entry that owned them is deleted, and this entry's title changed with them, because a
  heading that still said *"the engine cannot pay by card"* would answer a status question this
  file does not own.
- **Ask 3 stays here:** `server app web fill-card` is a yggterm verb, and it is what remains below.

⭐ **AND THE REPORT CLOSED AN OPEN ENTRY IN THIS FILE.** *"Agent engine: ctl fill is documented but
has no route"* sat at **FIXED IN CODE — LIVE PROOF OWED**, blocked on exactly one observation:
`ychrome ctl fill page_id=<p> entry=<item>` answering `{"filled":"filled"}` against a real login
form. The report delivers it verbatim — `{"entry":"…","filled":"filled","ok":true}`, on a real
login, today — so the ychrome daemon restart it was waiting on evidently happened. Entry deleted.
⇒ **a field report from another lobe IS live proof; read an incoming report against your own OWED
list before filing anything new from it.**

> ### ⚠ REOPENED IN PART, 2026-08-07 — the OWED observation was the wrong observation
>
> **The closure above stands for what it actually proved, and the deleted entry stays deleted:**
> the entry was *"documented but has no route"*, and the response does prove a route exists.
>
> ⛔ **But `{"filled":"filled","ok":true}` is NOT evidence the fill was correct**, and it was
> being read that way. gadgets row 5.2 observed the identical response on a **wrong write** —
> a 31-character value into a field whose vault secret is 20 characters, with the confirm field
> left **empty** — caught only by a hand-written page-side readback. Filed as
> `ctl fill REPORTS SUCCESS ON A WRONG WRITE` at the top of `ychrome/docs/pending-bugs.md`.
>
> ⭐ **The lesson is about the OWED list, not about ychrome.** "Live proof owed" named a
> *response string* as the falsifier. A response string can only ever prove the route; it cannot
> prove the effect. ⇒ **when writing a LIVE PROOF OWED line, name an observation of the EFFECT
> (a readback, a resulting state), never an observation of the ANSWER.** Every remaining OWED
> entry in this file is worth re-reading against that distinction.
>
> ⇒ And it sharpens item 1 below rather than contradicting it: `web fill-card` reporting
> `matched:false` on a good fill and `ctl fill` reporting success on a bad one are **the same
> defect with opposite signs** — a status field with no readback behind it. The optimistic
> direction is the dangerous one, because nothing prompts a retry.

**What remains here:**

1. **`server app web fill-card` answers `matched:false` on fills that landed perfectly** (measured
   on the IDFC 3DS page, RUN 6, 2026-08-07 00:15 IST — the field held the full value and the
   payment succeeded). Either fix the matcher or say plainly that `matched` is not an observation.
   ⚠ **Wrong in the PESSIMISTIC direction is the expensive one**: it invites a retry of a fill that
   already worked, on a payment page.

   ⇒ **Same verb family as the `web fill-vault` interleave entry below**, which already records
   that `"matched": false` "says nothing about the damage" — the second report that this field is
   uninformative, and the two want one fix. Neither ask touches the PAN boundary, which is right
   as it stands.

Field report from filing a CPC-ITR grievance end to end on the services-desk portal
(succeeded — ack 26390914 — but cost ~3 h and two failed attempts):
**[`docs/agent-cobrowse-gaps-2026-08-06.md`](agent-cobrowse-gaps-2026-08-06.md)**.
The headline four:

1. ⭐ **There is no file-upload verb** (`web do` is click/type only), so an agent
   cannot attach a document on any portal that asks for one. The grievance went
   in **without its s.154 order PDF**, which was staged and ready.
2. ⛔ **`termux-sms.py watch --code-only` returned the MATCH STRING, not the
   code** — so `itr-portal.sh` typed `OTP for Aadhaar` one char per OTP box and
   **no unattended login could ever have completed.** FIXED in-run.
3. ⛔ **One dropped ssh poll aborted an entire `watch` in ~5 s**, printing
   exactly what "nothing arrived" prints (`fetch()` raises `SystemExit`,
   uncaught). **This retires every "no OTP arrived within N s" ever recorded by
   that path as evidence** — and it manufactured a false "UIDAI stopped
   delivering" finding that two sessions then built theory on. FIXED in-run.
   ⇒ *A watcher that cannot distinguish "I waited and saw nothing" from "I never
   looked" will be believed, and its silence blamed on whatever was suspected.*
4. ⛔ **`~/.claude/skills/data-fabric/scripts/*` is in no git repo on any host**,
   so these fixes had to be `scp`'d to guihost and oc by hand; until then `oc` ran
   a broken reader while the skill text claimed the path worked.

## ★★ AGENT CO-BROWSE CANNOT COMPLETE AN OTP LOGIN

**Status:** OPEN

**★★ AGENT CO-BROWSE CANNOT COMPLETE AN OTP LOGIN — the logged-in plane stops
at the door (2026-07-28).** Full field report, seven confirmed defects and
nine costed feature asks: **[`docs/agent-cobrowse-gaps-2026-07-28.md`](agent-cobrowse-gaps-2026-07-28.md)**.
Written from a real job (building two diagnostic-lab orders end to end), not a
synthetic test. The headline four:
1. `web do fill --selector-set` refuses `surface_not_mapped` on a shadow
   surface, and the eval fallback fills segmented OTP boxes **visibly but
   without updating React state**, so the form posts an empty code and the
   site shows no error. Reads like a wrong OTP; is not. Same wall already
   recorded at a services portal. **An agent can read the SMS code off the phone in
   five seconds and then cannot type it.**
2. `el.click()` silently no-ops on many React handlers; a full
   `pointerover→…→pointerdown→mousedown→pointerup→mouseup→click` sequence at
   real coordinates works. Should be `web do click --gesture full`.
3. **ychrome is single-instance per profile and silently reuses the running
   session** — a second `ychrome --profile X <url>` replaces the existing
   page instead of opening a tab. Destroyed a live page mid-job.
   **✅ FIXED IN-TREE (ychrome merge d3dae32, 2026-07-31):** a routed url
   reports "opened as a new tab in the running session" and exits 0; an
   unrouted url on a stream with another pid's live anchor REFUSES by name
   (never a silent hijack); every anchor-here fallback names its reason.
   4 locks red-proven. ⚠ Live verify owed; residuals in the lane report
   (suspended-sibling anchor, no-arg picker path).
4. `YGGTERM_APP_CONTROL_PID` is honoured by `terminal new` but NOT by
   `web ensure`, which then refuses while naming that same variable.
   → **Fixed on `lane/dev/ensure-policy-gate`, awaiting live verification.**
   Root cause: BOTH binaries' `server app` dispatch blocks REMOVED the
   exported variable whenever the invocation carried no `--pid` flag, so the
   ambient default never survived to resolution and whether a verb appeared
   to honour it depended on the client roster. Targeting now goes through
   one owner (`yggterm_server::apply_app_control_target_overrides`): an
   explicit flag wins, no flag leaves the exported environment standing.
Highest-value asks, in order: trusted input into an unmapped surface (D1),
`--gesture full` (D2), verb-level `--expect` post-conditions (D3 — this run
reported five "successful" add-to-cart clicks that had all failed), and
multiple tabs per profile (D4).


## ★★ THE DAEMON'S ENVIRONMENT IS FROZEN AT LAUNCH AND POISONS EVERY SESSION IT

**Status:** OPEN

**★★ THE DAEMON'S ENVIRONMENT IS FROZEN AT LAUNCH AND POISONS EVERY SESSION IT
EVER SPAWNS — including across hot-restarts (oc, 2.12.18, 2026-07-28).**
Observed: on oc, `claude` in every yggterm-launched session died with
`Failed to authenticate. API Error: 403 ... Received Model Group=vercel/maa/deepseek-v4-pro`
— a retired custom-gateway config the user had already deleted from
`~/.profile` and `~/.bashrc`. Editing the rc files changed nothing, because
the rc files are not on the launch path at all.

Mechanism, all three links confirmed in the source:
1. `~/.profile` used to `. ~/.claude_code_env`, which exported
   `ANTHROPIC_BASE_URL` / `ANTHROPIC_API_KEY` / `ANTHROPIC_*_MODEL`. The
   daemon (PID 2397674, started Jul 27 17:09) captured that env at exec time
   and is orphaned to PID 1. The user deleted the file the next morning; the
   running daemon kept its copy.
2. `terminal.rs::shell_command()` builds `bash -c '<launch_command>'` — a
   **non-interactive, non-login** shell that never sources `~/.bashrc` or
   `~/.profile`. It calls `env_remove` only for
   `terminal_identity_env_removals()` (the TERM/appearance keys). Everything
   else is inherited from the daemon verbatim.
3. `lib.rs::spawn_daemon_process_from_executable()` (the hot-restart spawn
   path) does no `env_clear`/`env_remove` either — so a hot-restart *copies
   the stale environment onto its own successor*. Once a daemon is poisoned,
   the poison is immortal on that host; only a full daemon death breaks it,
   which the constitution forbids while sessions are live.

Net effect: any variable exported in whatever shell first started the daemon
becomes permanent, invisible, host-wide configuration for every agent CLI
yggterm launches, and the user has no rc-file edit that can reach it.

**Worked around, not fixed.** oc's `~/.claude/settings.json` now carries an
`env` block pinning `ANTHROPIC_BASE_URL` back to `https://api.anthropic.com`
and blanking the rest; Claude Code's settings `env` beats the inherited
process env (verified by running `claude` under the daemon's exact
`/proc/<pid>/environ` — the poisoned `ANTHROPIC_BASE_URL` is still inherited
and the call still authenticates through the subscription). That is a
Claude-Code-specific patch on one host; it does nothing for codex, for other
vars, or for the next host that catches this.

**The real fix is a design call, not yet made:** should the session-spawn
environment be re-derived from the user's login shell (allowlist) rather than
inherited from the daemon, and should hot-restart re-exec its successor with a
fresh environment instead of copying its own? guihost and dev daemons are
currently clean, so this is latent everywhere, live nowhere.


## A SECOND VIEWER STILL BUILDS ITS OWN WEBVIEWS (residual of the J8a entry;

**Status:** OPEN

**A SECOND VIEWER STILL BUILDS ITS OWN WEBVIEWS (residual of the J8a entry;
the STRANDING half is fixed, see below).** Webviews are per CLIENT, so a
shadow — or a second GUI — showing a 10-tab surface builds a second full set
(J8a: 11 more processes). That is what makes co-browsing work at all on the
current per-client surface model, and both sets are governed by the same
reclaim lane. It is also recorded because the memory arithmetic is easy to
forget: a second viewer of a heavy session roughly doubles the GUI-side
web-process bill for as long as both are shown.

### ⛔⛔ "A COST RATHER THAN A DEFECT" WAS WRONG, AND THE USER PAID FOR IT

This entry used to dispose of the duplicate set as a mere cost. **It is a
defect, and the disposition is corrected here rather than in a new entry.** The
second viewer does not just spend RAM — **it opens the page's video again in its
own WebKit process and plays it into the user's speakers**, where nothing they
can press will stop it. The page's pause, the mute button and the media keys all
act on the FIRST client's webview, because that is the one holding the media
session.

**User-reported 2026-08-06:** *"ychrome plays double youtube video with the next
same video unpausable or mutable… I had tried media keys, etc. many ways to stop
that bg video. There is no way to stop it unless I close the session row."*

Confirmed in the trace, twice, and **we caused it ourselves**:

```
1785964054633  pid 1510425 (the user's GUI)  native_open  media.example.invalid/watch?v=VIDEOID0001
1785964154970  pid 1690716 (an AGENT shadow) native_open  video.example.invalid/watch?v=VIDEOID0001   ← the phantom, +100.3 s
1785964425582  pid 1699947 (another shadow)  native_open  video.example.invalid/watch?v=VIDEOID0001   ← again
```

⚠ **There is no ~60s timer** — the user's "after ~1min" is the agent's own shadow
launch cadence (100.3 s and 110.6 s measured), not a reclaim constant. Do not go
looking for one. `WEB_SURFACE_DEFAULT_BACKGROUND_HOLD_SECS` (600) and the
pressured/thrash constants are not involved.

⚠ **Stashing does not help and must not be mistaken for a fix:** the shadow
stashed its copy 16 s in, and stash is a paint decision that is explicitly never
a mute — so the copy went INVISIBLE and stayed AUDIBLE. Only the cross-client
tombstone sweep reaped it, one second after the user closed the row, which is
why closing the row was the only thing that worked.

⛔ **THIS IS OUR OWN DOCTRINE'S BILL.** `feedback-agentic-surface-is-the-default`
tells every agent to probe through a shadow client instead of the user's GUI. So
**any agent taking a screenshot while the user watches a video reproduces this.**
Until the fix below is live-proven, an agent should not hold a shadow open on a
session the user is watching media in.

**FIX WRITTEN, NOT YET LIVE-PROVEN** (`set_muted` in the vendored web-surface
host, a `mute_web_surface` passthrough, and a mute at CREATE — not at stash —
for any `client_is_shadow_viewer()`, traced as
`native_open_muted_for_shadow_viewer`). One client owns the speakers; every
additional viewer is silent. ⏳ **Two things still owed:** a live proof that the
path fires (a shadow must be made to build a surface on demand, which did not
reproduce in the window available), and a check that WebKitGTK's `set_is_muted`
silences an ALREADY-PLAYING `<video>` rather than only new sources.

**The half that WAS already known to be a defect** — `session remove` answering `verified:true`
while the other client kept its set alive forever, with no row anywhere —
is fixed on 2026-08-01 (lane `lane/dev/webview-leaks`): every client now
sweeps the sessions it holds webviews for against the tombstone plane, in the
same conjunction `web ensure` refuses to revive on. Measured in the sandbox
before: GUI 2 → 1 webviews on a verified removal while the shadow stayed at 2;
after: the shadow drops with it. See `docs/web-surfaces.md` §Three ways a
process was minted for nothing.


## GUI process died mid-J8a with 51 webviews applied

**Status:** OPEN

**GUI process died mid-J8a with 51 webviews applied (guihost, 2.12.17 GUI 27779
→ fresh 325652 at 12:17:22, 2026-07-27). Cause UNDETERMINED** — no panic in
the trace, no readable OOM record; the 50-webview ramp stage had completed
one minute earlier, so the correlation is owned, not proven. The daemon
never blinked and every row survived (the constitution held).
**2026-08-01: the PRECONDITION is gone, the cause is not.** The only path that
could apply ~50 webviews to one GUI in a single call was `web ensure`'s
per-tab mint, fixed on `lane/dev/webview-leaks` and measured at 1 → 15 web
processes before / 1 → 1 after on a 13-tab surface. So this state is no longer
reachable by an agent verb — but nothing here explains WHY that GUI died, and
a user with 50 revealed tabs can still reach a similar count one reveal at a
time. Still a watch item: if a fresh GUI dies again near a large applied
webview count, this becomes the top entry.


## The profile picker CARD ITSELF still cannot be raised or photographed from

**Status:** OPEN

**The profile picker CARD ITSELF still cannot be raised or photographed from
the plane (successor to the J8b entry closed 2026-08-01).** Its row-menu
WRITES are now reachable — `server app web profile <list|show|avatar|protect|
unprotect>`, which is what closed the avatar-persistence hole — but nothing an
agent can drive opens the picker SURFACE, so the card's rendered avatar cannot
be screenshot-verified. The write is provable; the paint is not. Small, and
strictly narrower than the entry it replaces: `unknown_keys` in the verb's
answer covers the contract that had no proof at all. Fix when convenient: an
addressable route that reveals a picker surface (the rail/strip badge opens
the profile SWITCHER menu, `webprofile:<name>` entries only), after which the
existing `app screenshot --client <shadow>` does the rest.

⚠ **NARROWED 2026-08-13 — "nothing an agent can drive" is too strong, and the
difference changes who can fix it.** The route IS addressable: `server app open
webprofile:<name>` is accepted, sets `active_session_path: "webprofile:<name>"`
and `active_view_mode: "Rendered"`, and then fails with a NAMED reason:

```
timed out waiting for app open to settle for webprofile:<name>:
  {"active_session_path":"webprofile:<name>","active_view_mode":"Rendered",
   "ready":false,"reason":"preview surface not mounted"}
```

⇒ The missing piece is the **surface mount**, not an addressable route, and the
verb already says so in its own words. That is a much smaller thing than
"nothing exists", and it means the screenshot half may be one fix away rather
than one feature away.

⚠ **The probe moves the operator's view** — it sets the active session before it
fails — so restore the previous row immediately after. Done here.

**Falsifier:** `open webprofile:<name>` settles, and `app screenshot` returns the
picker card.


## A WebKitNetworkProcess OUTLIVES the WebContext that started it, and

**Status:** OPEN

**A `WebKitNetworkProcess` OUTLIVES the `WebContext` that started it, and
nothing we own can reap it (WebKitGTK behaviour, measured 2026-08-01).** The
residual of the "accumulates per profile churn" entry, which is otherwise
fixed. Our half was real and is gone: every destroy-and-recreate used to mint
a fresh `WebContext` (5 reloads = 5 network processes; J8a's 3 → 10 is 7
recreates), because the sweep ran inside `close` and took the engine in the
gap before the create. With the sweep moved to the tick, five reloads leak
zero. What remains is that dropping the LAST reference to a context leaves its
network process running: with `web_context_count()` at 0 and every surface
gone, the GUI still held 2. So the standing bill is **one network process per
distinct `web_context_key` the GUI has EVER opened**, not per live context.
Small (they are far lighter than web processes) and bounded by profile count
per GUI generation, but a long-lived GUI that cycles many profiles pays it.
Not obviously ours to fix — the next step, if it ever matters, is whether
`WebsiteDataManager`/`WebContext` disposal has an explicit terminate we are
not calling, or whether webkit2gtk simply keeps them for reuse.


## server app open on a REMOVED row times out instead of naming the reason

**Status:** OPEN

**`server app open` on a REMOVED row times out instead of naming the reason
(minor, guihost 2.12.17, 2026-07-27).** Opening a deleted session path correctly
does NOT resurrect the row and correctly leaves the active session untouched
(no select/activate events fire), but the CLI answers
`Error: timed out waiting for app open to settle …`. Compare `web ensure` on
the same class of dead path, which is exemplary: `accepted:false`,
`reason:"session_closed"`, `row_close_remembered:true`, plus prose naming why
and what to do instead. Make `app open` refuse in that shape rather than
time out.


## ★★★ AN UNREVEALED AGENT SURFACE REPORTS visibilityState: "visible", SO

**Status:** OPEN

**★★★ AN UNREVEALED AGENT SURFACE REPORTS `visibilityState: "visible"`, SO
ITS PAGE ANIMATES AT FULL RATE AND THE GUI COMPOSITES IT — measured
2026-07-26 night, and this is very likely THE mechanism behind every
"agents make the GUI host hot" report in this campaign. ⏳ FIXED IN-TREE AT
2.12.17; THE LIVE A/B IS OWED AND IS THE ONLY THING THAT CLOSES THIS.**
Ground truth: a payment-gateway page on a headless, never-revealed surface
the user cannot even see (no row — see the entry above) reported
`visibilityState: "visible"` with **1 running animation** (a spinner). Cost,
measured over 20 s from `/proc` (never `ps %CPU`): **web content 0.241 cores
+ GUI 0.399 cores = 0.85 cores total against guihost's ~0.5-core idle floor**,
Tctl 61.6 °C — the user's fan spun up on a machine that had been silent all
evening, while they were touching nothing.
⛔⛔ **RE-MEASURED 2026-08-08 17:22 IST ON guihost, AND IT IS WORSE THAN THE 0.85
CORES ABOVE — the owner asked why his fan was loud, unprompted.** Taken from
`/proc` over 20 s with NO build running and the user touching nothing:

```
yggterm GUI              0.597 cores
WebKitWebProcess (6)     0.902 cores   <- ONE of them is 0.637 alone
yggterm-headless (6)     0.068 cores
TOTAL                    1.567 cores
```

A per-process pass puts **0.637 cores in a single web surface** and 0.269 in the
shell's own Dioxus webview; the other four surfaces are at zero. So the cost is
not spread — it is one surface painting, which is precisely the shape this entry
describes.

⚠ **Honest attribution, because the agent measuring it was also the load:** the
owner's report arrived during ~50 minutes of continuous `cargo` builds and two
6-minute test suites (load average 6-11), and THAT was the dominant heat. The
1.567 cores above is the residual measured after the builds stopped, so it is
the part that does NOT go away when the agent stops. Both were true at once, and
saying only the first would have been convenient and wrong.

⇒ **This is the live A/B the entry has been waiting for, half-done: the "after"
arm on 3.0.59 still costs 1.567 cores.** What is still missing is the controlled
comparison (same surfaces, throttle forced on vs off) and the identity of pid
2886588's surface — capture `engine_visible` from the trace for each surface
before assuming it is the same unrevealed-surface mechanism.

**Why it is a product bug, not the page's fault:** every browser throttles
`requestAnimationFrame`, CSS animations and timers on a hidden page — that is
the Page Visibility contract, and it is the ONLY thing that makes background
tabs cheap. Our unrevealed surfaces claim to be visible, so the page has no
way to know it is not on screen and paints forever, and the shell composites
every frame of a surface nobody is looking at.
**THE FIX, AS BUILT.** WebKitGTK derives `document.visibilityState` from
**widget mapping** — there is no page-visibility setter on this API — so
"hidden to the engine" means "the inner webview is not mapped". Three
independent halves each kept that from ever happening, and fixing any one
alone would have left the bug alive:
1. **Creates were born visible** — `open` ended in an unconditional
   `show_all()`, so even a headless create was realized and mapped. An
   unrevealed create now hides the inner view immediately.
2. **The headless create demoted but never throttled** — `demote` is a
   Z-order move, not a visibility one. The reconciler now throttles beside
   the demote and records `engine_visible:false` in the trace.
3. **The reclaim pass could never reach it later, and exempted the leased** —
   a headless surface is marked stashed in the same breath, so the background
   plan classified it `Wait` forever; and when reached, `throttle: !leased`
   exempted every leased surface while `web ensure` leases unconditionally.
   **A LEASE IS A CLAIM ON EXISTENCE, NOT EVIDENCE OF A VIEWER** — it says the
   surface must keep existing and nothing about anyone looking at it.
**The trap that makes or breaks it:** an unmapped webview silently DROPS
synthesized events, and hiding is exactly what unmaps — so this would have
turned every `do`/`fill`/`type`/`key` into `surface_not_mapped` on precisely
the surfaces agents drive. The engine host therefore **wakes a view it hid for
the length of an injection burst and re-hides it after** — borrow-and-give-
back with a per-surface re-arm token, the same shape as the keyboard-focus
loan, and the same rule that a give-back only takes back what is still ours.
If the wake does not map, it is undone and the injection REFUSED: a refusal is
honest, a dropped event is not. Visibility gates RENDERING, never the drive
path; the audio veto is untouched, and the decision is per GUI process, never
a daemon query.
⚠ Do NOT "fix" this by navigating agent surfaces to `about:blank` between
actions — that is the workaround (correct for an agent to do voluntarily,
and it is now in the agent brief) but it hides the defect and breaks any
flow whose page state must survive. Nothing in the shipped fix navigates:
DOM, scroll, JS heap and in-memory bearers survive hiding untouched.
⚠ **THE LIVE A/B THAT CLOSES THIS, owed after the bump** — telemetry alone
cannot settle a heat claim: (a) an unrevealed surface reports
`visibilityState:"hidden"`; (b) `web do` and `capture-element` both succeed on
that SAME still-hidden surface (the wake/re-hide working, not a surface that
was quietly revealed); (c) a `/proc` cores delta against the captured 2.12.16
baseline (0.241 web + 0.399 GUI against a ~0.5-core idle floor) under the same
spinner page; (d) a faithful screenshot across background → reveal, because a
page that stops painting while hidden must come back correct; and (e) audio
keeps playing on an unmapped view.


## ★★★ A LIVE, LEASED WEB SURFACE CAN EXIST WITH NO ROW

**Status:** OPEN

**★★★ A LIVE, LEASED WEB SURFACE CAN EXIST WITH NO ROW — the user cannot see
or reach an agent that is browsing with their profile (found live
2026-07-26 night by a filing agent; user-reported the same hour as "why is
the agent row not in my Live Sessions, I cannot connect to it").**
Sequence, all reproducible: a previous run closed its work session (correct
hygiene — the row is TOMBSTONED in `removed-rows.json` and its PTY is dead,
`running:false, line_count:0`), and the next run called
`web ensure --session <that dead path>`, which happily **revived and leased
the surface anyway** (`already has a live surface`, generation 1). Result:
an agent drove a real payment gateway for an hour, on the user's
cookie profile, with **zero rows containing that session id** — nothing in
`server app rows` reflected that a surface was alive and being driven.
**The state "surface alive, row absent" should not be representable.** Two
candidate fixes, one must be chosen: (a) a surface holding a lease KEEPS (or
resurrects) a row for as long as the lease lives — which also satisfies the
constitution's UX test that the user can SEE an agent's session and click in
to co-browse it; or (b) `web ensure` REFUSES a session whose runtime is dead
and whose row is tombstoned, naming that reason, so an agent must create its
own session (and therefore its own visible row) instead.
**⚠ COROLLARY, same incident:** `web fill-card` then began refusing
`accepted:false, reason:"preempted"` — *"the user took this surface"* — on a
surface the user **cannot see, click, or have touched**. The agent-input
arbiter's preempt marker can be set on an unrowed orphan, so the human is
blamed for taking something invisible to them; and because the lane is keyed
`(session_path, generation)` with `forget()` only on close/recreate, the only
cure is a new surface generation. This is the same credit-ledger class as the
entry on injection credits leaking across the inter-verb gap — fix them
together.
**⚠ Practice note that made this worse, now corrected in the agent brief:**
every filing run tore its session down as a courtesy, so by the third run the
only thing left to attach to was an orphan. Agents should create ONE session
per run and LEAVE IT UP; visibility beats tidiness.



⚠ **RE-CHECKED 2026-08-13 AND THE RESULT IS INCONCLUSIVE — recorded so the zero
is not banked as a fix.** On the live GUI host: `agent_leases: 0`,
`web surface entries: 0`, orphans found: **none**. But **there were no leased
surfaces at all**, so "no orphan" is the answer this check gives in a world where
the bug is impossible to observe. It discriminates nothing. (The matching logic
was positive-controlled first — it can find a row that exists — so the zero is a
real zero, just an uninformative one.)

**The decisive test, and the two reasons I did not run it here:**

```
create a row → remove it (tombstoning it) → web ensure --session <dead path>
             → does a live surface now exist with no row?
```

1. ⛔ **Never on the default profile.** The original incident drove a payment
   gateway on the user's own cookie jar; an unqualified surface IS `default`.
   Use an `agent-<n>` profile.
2. ⚠ **It creates a real web surface on the shared GUI host**, which is the
   subject matter of the open idle-cost investigation — a surface appearing
   mid-A/B corrupts someone else's measurement. It wants a window, not an
   opportunistic run.

**Falsifier:** after `web ensure` on a tombstoned session path, either no surface
is alive, or a row exists for it.

### ✅ THE NAMED SEQUENCE IS NOW REFUSED — fix (b) shipped, measured 2026-08-13

Ran the entry's own sequence on the live host under a granted window: create a
row → `session remove` it (tombstoning it) → `web ensure --session <dead path>`.

```
accepted: false
detail: "…'s row was closed by the user and its runtime is gone, so reviving a
         web surface under it would give you a live page with no row the user
         can see or click into. Create your own session … and drive its surface
         instead"
```

⇒ **The refusal restates this entry's own justification**, so option (b) was the
one taken. ⭐ **No surface came into being at all** — `agent_leases: 0`,
`active_surface_requests: 0` after the attempt — so the check cost nothing and
touched no profile.

**Control, same run:** a session path that never existed refuses with a
*different* reason (`the daemon has no web-surface declare for …`), so the verb
is discriminating between cases rather than refusing everything by default.

⚠ **TWO THINGS THIS DOES NOT CLOSE, and the entry stays open for them.**

1. **Option (a) was not implemented.** A surface holding a lease does not keep or
   resurrect a row; the orphan is prevented at one door rather than made
   unrepresentable. The constitution's UX test — the user can SEE an agent's
   session and click in to co-browse it — is met here by refusal, not by the row
   surviving.
2. **The other sequence is untested:** removing a row that ALREADY has a live
   surface. `web ensure` refuses a dead path, but nothing here shows what happens
   to a live surface when its row is removed underneath it. That needs a
   web-capable row (a plain shell has no declare at all — the control above says
   so), so it creates a real browser surface and wants its own window and an
   `agent-<n>` profile.

**Falsifier for what remains:** remove a row whose surface is live, and no
surface survives without a row.
## ★★★ web do FIDELITY ON RE-RENDERING DOMs

**Status:** OPEN

**★★★ `web do` FIDELITY ON RE-RENDERING DOMs — three reproducible defects,
one family (a live portal filing run, 2026-07-26 ~15:30-16:00 IST, guihost 2.12.15,
session `local://b556fb1b`, all self-reported SUCCESS while wrong):**
1. **`do fill` DROPS and INVENTS characters on React controlled inputs.**
   `fill --selector '#street' --text "Sample Fixture Road"` → response
   `chars:19, delivered:true, is_trusted:true, cleared_verified:[true]`, field
   held **"Ja"**. Earlier `#username` fill reported chars:10, field ended
   **"0000000000hg"** — two stray chars never passed in any `--text`, which
   then poisoned the portal API call (404). ⚠ The strays coincided with the
   live focus-theft window (entry below) — possible seat/agent input
   cross-contamination; the focus investigation owns that half.
2. **Clear-verification false-negative:** batch fill on an EMPTY `#Landmark`
   aborted `clear_failed (box(es) [0] of 1 still hold text)` — the field was
   empty; likely verifying the previously-focused element, not the target.
3. **`--role option --label X` resolves a STALE RECT in a scrolled MUI
   listbox:** `--label PASSPORT` → `accepted:true, delivered:true,
   is_trusted:true`, nothing happened. Working recipe: tag the `li` by id via
   eval + `scrollIntoView({block:'center'})` + `do click --selector`.
**Common shape: the verb resolves/verifies against DOM state that has moved
(framework re-render, scroll, focus change) and its self-report cannot go
red.** Fix direction: verify-by-readback of the TARGET's final value against
the requested text (honest failure when mismatched), clear-verify the
resolved target element only, re-resolve rects after scroll before injecting.
4. **Duplicate DOM ids across repeated form blocks break `--selector`
   targeting.** The portal renders two party form blocks with the SAME
   ids (`#Name`, `#District`…); `#id` selectors silently hit the FIRST, so
   the agent drove the complainant's field while aiming at the OP's — twice.
   Agent-side workaround: strip injected ids from previous holders before
   re-tagging; address via `querySelectorAll("[id='X']")[n]`. Verb-plane
   want: `--nth` on `--selector` (or an ambiguity warning in the response
   when a selector matches >1 node).
5. **A stale MUI popper stays mounted and poisons the next pick** — after a
   failed pick, `li[role=option]` still returns the OLD listbox's options,
   so the next selection silently matches the wrong list. Proven recipe:
   `web do key --key Escape` before each pick (that verb works). Candidate
   verb-plane fix: role/option resolution scoped to the NEWEST open listbox
   (aria-expanded owner), not the first match in document order.
Falsified the other way (keep): MUI async Autocompletes ARE drivable via
`do click` + `--role option`; headless file upload via DataTransfer works
(379 KB PDF through one `web eval --stdin`, no GTK chooser).

**STATUS 2026-07-26 — ALL FIVE HALVES ARE CODE-FIXED, NONE LIVE-VERIFIED.**
The fix is one mechanism, not five patches: **the matcher runs ONCE per verb
and its result is PINNED** (`window.__yggDoPins`), and every later step —
clear, clear-verification, the write, the readback, the rect re-measure —
addresses that handle instead of re-running the selector. A re-render can no
longer substitute a twin between any two steps of a verb.
- (1) `fill` now READS THE FIELD BACK through the pin and reports
  `verified` / `verify_reason` / `requested` / `held` / `first_mismatch`.
  `delivered: true` and `verified: false` co-exist and that is the point.
  Plain text/textarea inputs are written with the **native value-setter +
  bubbling `input`/`change`/blur** (`mechanism: native_setter`), the filing
  agent's proven workaround; real keys stay for segmented widgets and for
  secrets, and the response always names which ran.
- (2) clear-verification reads the PINNED nodes' state; a node the framework
  re-rendered away is `node_replaced`, its own refusal, never `clear_failed`.
- (3) resolution is TWO phases — pin+scroll, settle 120 ms, RE-MEASURE the
  pin — and `web_do_resolved_from_info` REFUSES any payload not stamped
  `phase: post_scroll`, so collapsing them back cannot pass silently. The
  response carries `resolved.rect_phase` + `is_connected`.
- (4) CSS targets resolve via `querySelectorAll(sel)[nth]`; `--nth` works on
  `--selector` (wire: `{"css":…,"nth":…}`, bare string still means nth 0) and
  every addressed response carries `match {matches,nth,hidden,ambiguous}`.
- (5) `role=option`/`menuitem` pools are filtered for liveness and scoped to
  the listbox an `aria-expanded` combobox owns (else the last visible one);
  a pool of only stale options refuses `stale_listbox_only`, never a click.
⚠ **Live verification is OWED**: no live portal re-run, no guihost deploy. The
daemon/GUI on guihost still runs the old behaviour until the next bump.
Remaining agent-side: the INVENTED characters in (1) are still attributed to
the concurrent focus-theft bug — the readback now catches them, it does not
prevent them.


## ★★ THERE IS NO CLIENT TO RENDER AGENT SURFACES INTO ON dev

**Status:** OPEN

**★★ THERE IS NO CLIENT TO RENDER AGENT SURFACES INTO ON dev (2026-07-26).**
The data-fabric default "co-browse on a SHADOW surface on dev" is currently
unusable: `server app clients` on dev → count 0 (no GUI, no shadow client),
so the filing agent had to fall back to the user's live GUI host. Fresh
evidence for settled call #6 (drive shadow surfaces with the GUI closed /
server-side rendering, docs/optimization-pass.md WS2): today agent browsing
physically requires the user's GUI host.

✅ **STILL TRUE 2026-08-13**: `server app clients` on that headless host answers
`{"clients": [], "count": 0}`. Re-measured during a sweep, and it is a
state-of-the-world claim rather than a defect report — it stays true until
server-side rendering lands, so it should be re-read as a standing constraint,
not as a bug awaiting a fix.

**Falsifier:** a headless host answers `count > 0` without the user's GUI running.


## ★★ THE DAEMONS CHAIN, AND ONE IDLE bash -i IS WHY (root-caused

**Status:** OPEN

**★★ THE DAEMONS CHAIN, AND ONE IDLE `bash -i` IS WHY (root-caused
2026-07-25; the RPC half FIXED, the durable half OPEN).**
⛔ **First, a correction to this entry's own earlier wording.** I filed the
observed "13 Running -> 9" across the 2.12.11 swap as *"a hot restart kills
live PTYs, violating keep-alive."* That is WRONG. The trace shows those seven
are `progressive_migration_session_released` events — the **designed**
kill-and-re-resume by which an agent session is handed to the successor. That
is exactly why click = resume recovered one at 168x63 with real scrollback.
Rows never dropped (24 -> 26). Nothing is violated by the release itself.
**What IS wrong:** the drain that performs those releases had exactly one
call site — the `disk_binary_replaced` self-retire branch — so an explicit
`HotRestart` RPC (what a deploy sends) preserved its PTYs and started no
drain at all. guihost only appeared to migrate because the middle daemon still
had a thread alive from an earlier self-retire. **FIXED** — the accept loop
now starts the drain on a preserving handoff, locked both directions.
**STILL OPEN — the durable half.** `session_kind_is_migratable_agent`
(`daemon.rs`) admits only `Codex | CodexLiteLlm | ClaudeCode`: a plain shell
is not re-resumable, and there is **no fd passing anywhere in the tree**
(`SCM_RIGHTS`/`sendmsg` -> zero hits), so the only way to move a PTY is
kill-and-re-resume. Therefore **one idle `bash -i` pins its daemon at its
birth version forever**, and the daemon can never reach empty hands:
`daemon_should_idle_shutdown` refuses while any terminal session remains, and
the stale-daemon sweep refuses a local shell. Live on guihost: three of the four
stranded keys are `bash -i`. Fixing this needs lossless fd-handoff
(`SCM_RIGHTS`) — that is the real work. A cheaper MITIGATION, and a policy
call not an obvious win: let a **non-keep-alive** shell on a lingering
predecessor be reaped so the daemon can drain, trading that shell's live
scrollback for convergence. It must never touch a keep-alive shell.
**Diagnose** by `~/.yggterm/hot-update-terminal-owners.json` (runtime key ->
owner socket + pid) and PTY ancestry — never by row count, which stays
healthy throughout.

**★ THE CHEAPER MITIGATION WAS COSTED AND DELIBERATELY NOT BUILT
(2026-07-26). Do not re-propose it without re-reading this.** Ground truth
from each daemon's own socket that morning: the 2.12.10 daemon (51 h) owned
exactly two sessions, both `kind=shell keep_alive=false` — "Secrets Fetch
Failure Debug" and "Workspace Shell". The 2.12.13 daemon owned an agent
session **and `local://1c17bfad` "New Yedit", `kind=shell keep_alive=TRUE`**.
So the reap frees **ONE** of the two supernumerary daemons; the other holds a
keep-alive shell it may never touch. Say "one daemon", never "36% of the
total".
**And what it now recovers is close to nothing.** Both of that daemon's
costs were global loops, not ownership, and both are fixed above it: the
perf-incident monitor's whole-corpus re-read (measured 334.7 MB per 90 s,
byte-identical in all three daemons) and the machine-wide transcript walk
(908.1 / 908.1 / 454.0 MB per 90 s). With those gone a superseded daemon
holding two idle shells costs ~0.001 cores and ~33 MB of RSS. **Weigh that
against killing a live PTY with 51 hours of state in it, named "Secrets Fetch
Failure Debug" — plausibly a human's debugging session. It is not worth it.**
**The defect underneath is real and stays open:** the non-keep-alive reap has
exactly ONE call site, `ServerRequest::PrepareClientClose` (`daemon.rs`;
`non_keep_alive_live_session_paths` has no other caller). A GUI that is
SIGKILLed, crashes, or is swapped by a deploy never sends it, so a shell the
user never marked keep-alive outlives the GUI it contracted to die with —
AGENTS.md: second-class sessions "survive GUI death IFF marked keep-alive".
Both 51-hour shells on guihost are that bug, not a policy gap. The right fix is
to close the path (reap on the successor's first tick when the predecessor's
client is provably gone), NOT to add a scheduled killer: pace it one per tick,
oldest-idle-first, with an idle-age floor, gated on `daemon_is_superseded`,
tracing the title and idle age of every reap. Never touch `keep_alive=true`
(shell OR agent), an agent kind (those are RELEASED for lossless re-resume by
`select_next_migration_candidate`, never killed), `remote-session://` /
`SshShell` rows, or the ~1,167 `server-*.sock` entries — those are symlink
ALIASES forming the cross-version compat plane, not litter.


## ★★ A SESSION STRANDED ON A preserved OWNER HAS NO DECLARES, AND THE RAIL

**Status:** OPEN

**★★ A SESSION STRANDED ON A `preserved` OWNER HAS NO DECLARES, AND THE RAIL
REBUILD FAILS SILENTLY (found 2026-07-25).** The declare-rebuild
(`1c88d4a`) asks the daemon that answers `terminal_app_declares`. But after a
hot restart that could not hand over every PTY, the old daemon keeps owning
the leftovers: they appear under `preserved_terminal_owner_keys` on the new
daemon, NOT `owned_terminal_session_keys`. An app running in such a session
declares to the OLD daemon, so the new one answers "no declares" — and
because "no declare at all" is not a refusal, **nothing traces**. Observed on
guihost: `right-panel pane:notes` on a shadow dispatched
`terminal_app_declares`, completed in ~860 ms, applied nothing, and emitted
no `daemon_declare_*` reason. The rail simply never appeared.
⛔ **CORRECTION to this entry's own fix list.** It proposed "have the
surviving daemon proxy declares for the sessions it lists as preserved."
**That proxy already exists** and shipped with the feature (`cb4eff9`):
`ServerRequest::TerminalAppDeclares` resolves
`preserved_owner_endpoint_for_request` and forwards to the owning daemon. So
the design was never missing. Two other things were, both now proven from the
live trace:
1. **The owner could not answer.** `TerminalAppDeclares` shipped in 2.12.10,
   and the stranded session's owner was **2.12.9** — it cannot deserialize the
   request, so it writes nothing. The proxy dutifully reported
   `preserved_owner_request_failed {error: "parsing daemon response: \"\""}`.
2. **The client threw that away.** Both rebuild paths ended in
   `.unwrap_or_default()`, collapsing "the fetch FAILED" into "there are no
   declares" — which is the whole of "no error, no trace." **FIXED**: they now
   branch, and trace `daemon_declare_unavailable` (with the error) separately
   from `daemon_declare_absent` (reached the owner, genuinely nothing there).
So the remaining gap is only that a pre-2.12.10 owner is unanswerable, which
the daemon could detect up front from the recorded
`PreservedTerminalOwnerEntry.owner_server_version` instead of issuing a
request it knows will fail. Once the daemon chain converges (entry above),
this stops arising at all.
**Diagnose** by comparing the two key lists in `server status` — a session in
`preserved_terminal_owner_keys` is on the old owner. **Unblock** with a fresh
session on the current daemon (proven: same yedit, fresh session, full rail
rebuilt on a shadow that never saw the declare).


## ★★★ THE FIFTH FOCUS PATH

**Status:** OPEN

**★★★ THE FIFTH FOCUS PATH — IT IS NOT JAVASCRIPT. Root-caused 2026-07-26;
✅ THE FOCUS-BORROW FIX IS SHIPPED AND USER-CONFIRMED LIVE ON guihost. What keeps
this entry open is the SECOND bug it filed — the injection-credit ledger —
plus the unexplained `fill` corruption at the end.**
The user, mid-session:
*"the shadow session spawn took focus away from my viewport and this session
… it is stealing my focus again and again while working."* Four earlier
rounds all found JS thieves, and the guard that came out of round four
(`UI_FOCUS_OWNER_SELECTORS` + the source scan) is intact and innocent here —
because **this thief never touches the DOM**.
**The mechanism.** A native web surface is a WebKitGTK webview parented in the
SAME GtkWindow as the shell's own webview. `gtk_widget_grab_focus` on it sets
the **GtkWindow's focus widget**, so keyboard focus leaves the shell webview
while the window stays active and the shell's `activeElement` stays exactly
where it was. Two call sites did it:
1. **At birth.** wry's `WebViewAttributes` default is `focused: true`
   (`vendor/wry/src/lib.rs:853`), which `grab_focus()`es in `new_gtk`
   (`vendor/wry/src/webkitgtk/mod.rs:385`). Nothing in the tree ever called
   `with_focused`, so a **headless** `web ensure` surface — created and
   demoted in the same tick, never revealed, no pixel on screen — took the
   keyboard the instant it was built. That is the "spawn took focus away".
2. **Per verb.** `inject_key` grabbed the focus for every injected keystroke
   and never gave it back, under a comment asserting the grab was
   "widget-local — it does not move the seat's global focus on screen". True
   of the SEAT, false of the TOPLEVEL. `do type` / `do fill` / `fill-vault` /
   `fill-card` / `totp` all route here; `do click` re-takes it through
   WebKit's own focus-on-button-press.
**The instrument that finally saw it** (every JS-side probe is blind to this,
and so is `active_session_path`, which never moved): read
`document.hasFocus()` in the shell AND in the surface **at the same moment**.
Live on guihost, 16:04: shell `hasFocus:false` / `activeElement`
`textarea.xterm-helper-textarea` / `window_focused_at_last_watchdog:true`,
while the invisible agent surface reported `hasFocus:true`,
`activeElement:INPUT#identityproof`. A surface reporting `hasFocus:true`
**falsifies** "the window is simply unfocused" — the window is active and the
agent's page owns its keyboard. The user's terminal recorded its last
keystroke at 15:47:57 and none for the next 40 minutes
(`input_batch_flush_count` frozen at 236). 17 focus-taking verbs ran on that
surface between 15:46:42 and 15:59:24, plus the birth grab at 15:45:29.381.
**The rule now encoded:** *an agent may BORROW the window's keyboard focus
around an injection; it may never keep it, and a surface nobody can see never
gets it at all.* `note_focus_owner_before_injection` books the lender,
`schedule_focus_giveback` returns it 150 ms after the burst's last event (one
give-back per burst, so a multi-key fill still costs the page one `blur`, not
one per character), and it refuses to take focus back off any widget the human
moved it to meanwhile. `open()` now takes `focused`, which the shell wires to
`want_visible`. Locked by
`no_web_surface_takes_the_window_keyboard_focus_without_giving_it_back`.
✅ **VERIFIED LIVE AND CONFIRMED BY THE USER** on the deployed GUI: a
480-verb agent burst driven against a headless surface while the user worked
in their own session, and they felt nothing — no steal, no interruption —
with zero focus/select trace events inside the burst windows and screenshots
taken across them. The user's own experience is the instrument that settles
this one: every JS-side probe is blind to a GtkWindow focus move, which is
how four earlier rounds all missed it.
⚠⚠ **It IS keystroke cross-contamination, one direction, CAUGHT LIVE.** A
passive `keydown` recorder installed in the agent's page logged three
`isTrusted:true` `Escape` presses — 16:09:35.815, 16:09:51.001, 16:23:42.600 —
with **no agent verb within ±8 s of any of them** (the agent's last verb ran
at 16:05:46). That is the human, pressing Escape at a terminal that had
stopped answering, and landing in an invisible the portal form instead. The
other direction is structurally impossible and stays that way: `synth_key`
hands the event to the surface widget with `gtk_widget_event`, which never
traverses the toplevel's focus chain, so an agent's characters can never
reach the user's terminal.
⚠⚠⚠ **AND THE ARBITER DID NOT NOTICE — the second bug, and the reason this
entry is still here. ⏳ FIXED, BUT ON THE UNMERGED LEASED-SURFACE LANE, NOT IN
main AT 2.12.17, AND NEVER LIVE-VERIFIED.** Real seat
input on a surface is supposed to increment the arbiter's counter and refuse
the agent's next `do` with `preempted`. Zero `agent_input/preempted` events
exist for `local://b556fb1b…` and no verb was refused, across the whole
incident. The reason is the **injection-credit ledger leaking across the
inter-verb gap**: `grant_injection_credits` books one credit per injected
event, `note_seat_input` spends a credit instead of counting a human, and
unspent credits are only dropped by `take_seat_input_count` — whose own
comment names the hazard exactly ("carrying it forward would let it swallow a
LATER real gesture, turning a fix for the agent into a bug for the user") and
which the shell nevertheless calls only at the START of the next verb
(`web_do_open_lane`'s gate, and between a batch's actions), never at the end
of the verb that granted them. `do fill --text "0000000000"` grants a dozen
credits (select-all + delete + ten characters) and — because delivery is
synchronous, so the lexical `INJECTING_EVENT` flag already suppressed every
one of them — leaves the whole dozen sitting there. The user's next dozen
real keystrokes are then silently absorbed as "ours". **That is why
`0000000000hg` took two of the user's characters into the field with no
preempt and no journal**, and it is a live co-browse defect
in its own right: on a surface the human is genuinely sharing, their first N
keystrokes after any agent verb are invisible to the gate that exists to
protect them.
**THE FIX, AS WRITTEN:** credits expire on a short clock. Each credit is
recorded with the millisecond it was granted, and anything older than
`INJECTION_CREDIT_TTL_MS` (**250 ms**) is dropped before spending — a credit
covers ONE injected event GTK may deliver late, not the whole gap until the
next verb. The clock is injected at the entry points so the expiry is tested
exactly rather than by sleeping. ⚠ **It is NOT in main.** It rides the
leased-surface-with-no-row lane, which is still hardening its locks and has
not been merged, so nothing in 2.12.17 changes this behaviour — and the
ledger's own doc comment in main still says, correctly for main, that nothing
here expires on a clock. **Live proof owed after that merge and a bump:** a
real co-browse loop where the user types immediately after an agent verb and
the arbiter counts every one of their keystrokes.
The other reported corruption — `fill --text "Sample Fixture Road"` reporting
`chars:19 delivered:true` while the field held `Ja` — is **not explained by
either of the above** (17 characters lost, not 2 gained) and stays open; look
at the page's controlled-input re-render, not at focus.
⚠ **Focus-safe verbs, for anyone driving a surface over a working human:**
`web eval` / `read` / `wait` / `frames` / `screenshot` / `capture-element`
never touch GTK focus (guest-JS `element.focus()` sets DOM focus only). Only
`do` and the `fill*`/`totp` family take it.


## ★★ AGENT WEB-SURFACE AUTOMATION HARD-CRASHES THE GUI (WebKitGTK

**Status:** OPEN

**★★ AGENT WEB-SURFACE AUTOMATION HARD-CRASHES THE GUI (WebKitGTK
segfault) — diagnosed 2026-07-24 on guihost; LAYER 1 (crash surface) FIXED +
LIVE-VERIFIED at 2.12.8 (`c3c7086`), LAYER 2 (routing/isolation) OPEN.**
**UPDATE 2026-07-24 (dev agent):** the raw-coordinate `do click` path was the
culprit — it synthesized a native GDK button event with NO hit-test, unlike
`ClickSelector`. Fixed in `web_surface_do_for`: the `Click{x,y}` arm now evals
`document.elementFromPoint(vx,vy)` FIRST and refuses (never injecting) if it
returns null or the eval fails — which both confirms a live element is present
AND round-trips through the web content process, so a page that cannot lay out
fails there instead of taking a synthetic click into a dying frame. Live-proven
on the fixed GUI (guihost pid 3290202, GUI-only swap, daemon + all 6 sessions
preserved): a blind click at (5000,5000) into a MAPPED 1mg surface is refused
with "no live element … refusing a blind native click"; a valid `--selector`
click succeeds; the GUI survives every blind click that previously segfaulted
it. Prefer `do click --selector`. **STILL OPEN (layer 2):** a WebKit-internal
race on a *valid* element is not fully preventable from the UI process — the
ultimate belt is process isolation (run agent web surfaces in a shadow/child
process that can die alone) or GUI auto-restart (the transient scope has no
`Restart=`), plus the SHADOW-PROBE routing so agent web verbs never drive the
user's foreground surface. Those are the remaining fixes.
A `web_surface_do` synthetic click injected into a `local://<uuid>` web
surface segfaulted WebKitGTK and killed the entire GUI process. dmesg:
`yggterm[<pid>]: segfault at 48 ... error 4 in
libwebkit2gtk-4.1.so.0.21.8` — a null-pointer read (deref at struct
offset 0x48) inside WebKit. The GUI's last two trace events before death
were the trigger: a `web_surface_eval` DOM scrape
(`document.querySelectorAll("*")`) then a `web_surface_do` primary click
at (122, 514). **Not OOM** (no oom-kill at crash time, memory healthy);
**not a Rust panic** (`panic.log` untouched — a native C++ crash bypasses
the Rust panic hook, so the process just takes SIGSEGV). The GUI runs as
a one-shot transient systemd scope (`app-dev.yggterm.Yggterm@<uuid>`, no
`Restart=`), so once it died nothing relaunched the window. The daemon
(separate process) survived and kept owning every PTY, so all live agent
sessions were unaffected — the crash was cosmetic to the work, but the
user lost the window.
**Two failure layers, both need a fix:**
(1) *Crash surface:* a synthetic-click / DOM-eval into a WebKitGTK web
surface can null-deref inside WebKit. The injection path must be guarded
(validate the target surface/element is live before dispatch; catch/
isolate the webview call so a bad injection cannot take down the whole
GUI process — ideally the web surface is a child that can die alone).
(2) *Routing violation:* this web-surface automation was aimed at the
user's **active GUI** instead of a shadow view-client — exactly what the
SHADOW-PROBE LAW forbids (untargeted verbs route to the active client =
the user's GUI). `web do/eval/wait` verbs should refuse to drive the
active user GUI and require a shadow/backgrounded target, or spawn one.
**Broader pattern:** this is not a one-off — ~20 yggterm segfaults in a
single day's dmesg (webkit/glib/libc) and dozens of `failed`
`Yggterm@*.service` scopes; the web-surface automation path (landed
2026-07-23 in the agent-client no-activate + shadow-probe commits) is the
freshest suspect. **Recovery gotcha (found live):** a leftover shadow
view-client intercepts GUI relaunch — a plain `yggterm` launch and
`server app launch` both get handled by the registered shadow (it tries
to focus its own headless `wayland-1` window and fails) instead of
spawning the primary GUI. Tear the shadow down first
(`scripts/shadow-client.sh stop --name agent-1`), then launch the primary
GUI with the KDE `wayland-0` env — it re-attaches to the surviving daemon
with no re-resume (live-verified: 6 owned · 6 total · 0 preserved).


## ★ USER RE-CONFIRMED 2026-07-23 (during the 2.12.7 session): codex sessions

**Status:** OPEN

**★ USER RE-CONFIRMED 2026-07-23 (during the 2.12.7 session): codex sessions
still paint COLD-START JSON GIBBERISH** — raw conversation prose as wrapped
plain text, duplicated turns, no codex TUI chrome, on a cold launch. This is
the motivating repro of `docs/spec-agent-cli-harness.md` (§7.6: the attach
seed has TWO WRITERS by construction — daemon seed + client reveal replay),
and its structural fix is the spec's phase 0/3. The spec build is gated on
the user's explicit go; when given, the acceptance test is: a cold-launched
codex session must be pixel-indistinguishable from a manual
`ssh -t <machine> codex resume <UUID>`.
**Same report, swap-window frames:** two clipboard frames captured at 13:41
(broken bottom-line interleave, then a blank viewport) fall inside the
GUI-swap settling window ~1–3 min after the 2.12.7 GUI relaunch; the surface
settled clean by 13:47 (faithful screenshot, bottom intact) and mount churn
stopped. Deploy-window transients are a documented class (field guide §4.4);
what changed in 2.12.7 is that input returns in seconds, births mount once,
and a detected ring gap reconciles — the remaining swap-window paint
transient is the attach-seed seam the harness spec owns.


## libyggterm apps over a MANUAL ssh hop say "not inside yggterm"

**Status:** OPEN

**libyggterm apps over a MANUAL ssh hop say "not inside yggterm"
(user-confirmed 2026-07-23).** Spawn a local yggterm terminal, `ssh <host>`,
run `yedit` there → detection fails because `YGGTERM_SESSION_ID` does not
cross a user-typed ssh hop. TWO halves:
1. **Detection — ACTIVE on guihost-local (2026-07-23, 2.12.8 daemon swap):**
   the daemon exports `LC_YGGTERM_SESSION_ID` at PTY spawn (the iTerm2
   `LC_TERMINAL` trick — stock OpenSSH forwards `LC_*` both ways by
   default), and yedit falls back to it. Live-proven: a fresh guihost PTY
   echoes the session key from `$LC_YGGTERM_SESSION_ID`. ⚠ PTYs owned by
   REMOTE machines' daemons (dev/oc fleet, B1-parked) still predate the
   export until those daemons bump.
2. **Control-channel attribution — DESIGNED, NOT BUILT:** even with
   detection, the app's declared control endpoint is loopback on the REMOTE
   host, and the GUI resolves forwards from the SESSION's `ssh_target` —
   which is local for a manual hop, so the fetch dials the wrong machine and
   the surface dies as "not responding". Design: the declare payload carries
   the app host's identity (`gethostname()`); the GUI maps it to a known
   remote machine (requires a hostname↔machine mapping the remote-machine
   registry does not hold yet — `RemoteMachineSnapshot` has `ssh_target` and
   `label` only, and oc's hostname ≠ its alias) and spawns the `ssh -L`
   against that machine. Until built, the honest state is: detection works
   (post-bump), surface takeover over a manual hop does not; running the app
   in a session yggterm itself opened on that host works fully.


## Live-path frame corruption on busy CC sessions

**Status:** OPEN

**Live-path frame corruption on busy CC sessions (guihost, 2026-07-10).** While
an agent streams heavily, the CLIENT xterm buffer accumulates single-cell
holes (`t ik` for `think`, including the user's own composer echo), merged
rows, and whole frames interleaved at wrong positions — while the daemon
vt100 screen stays clean and no `resync_required`/`cursor_rewound` events
fire. So bytes are lost/mutated between the daemon read and `term.write` in
the GUI. The ATTACH-seed variant of this class is fixed in 2.10.4 (viewport
reconcile chunk); the live-path variant is still open. Prime suspects:
(a) `batch_terminal_chunks` sanitizers rewriting live frames (the
`observation` rejoin converts `\r\n`→`\n` and strips "noise" lines whenever
a batch lacks alt-screen/hide-cursor/high-volume markers — content-triggered,
so yggterm-dev sessions whose transcripts CONTAIN transport-noise phrases are
hit hardest); (b) `terminal_write_bridge.stage_or_immediate` ordering under
frame-budget mode. 2.10.4 ships the probes to convict: mine
`terminal_forward_divergence` + `terminal_write_send_failed` in
`event-trace.jsonl` and run the client-buffer vs daemon-screen diff recipe in
`.agents/skills/yggui-app-control/SKILL.md` while a session streams.
**UPDATE 2026-07-11 (telemetry campaign run 1): suspect (a) CONFIRMED.**
`terminal_forward_divergence` fired on guihost (4/5 events on `local://`/`live::`
sessions, drops of 1-11 bytes), and code trace convicted the sanitizers:
`strip_internal_terminal_transport_noise_lines` did `.replace("\r\n","\n")` over
the whole batch (content-gated on transport phrases, so it hits local dev
sessions), and `strip_low_signal_terminal_noise_lines` used `str::lines().join`
- both drop carriage returns, so xterm paints the next line at the wrong column
(the staircase/interleave garble). Fixed in 2.10.13: both now `split('\n')`
(CR-faithful); regression test
`batch_terminal_chunks_preserves_carriage_returns_in_kept_lines`; the probe now
emits `cr_dropped`. Suspect (b) not yet investigated.

**UPDATE 2026-07-11 (run 2): the CR fix was NOT the whole bug — the excision
itself is.** User re-reported (in different words): "local sessions are dropping
chars sometimes and replacing the rendering with spaces." Run 1 sized the drops
at 1-11 bytes and assumed CR loss was the entire mechanism. Re-mining
`terminal_forward_divergence` found the real magnitude on the user's OWN session:

    local://20e56a8b   raw 9153  → forwarded 8474   = 679 bytes dropped
    local://20e56a8b   raw 23991 → forwarded 23312  = 679 bytes dropped

679 bytes is a whole-line EXCISION, not a lost `\r`. Mechanism:
`strip_internal_terminal_transport_noise_lines` content-matches three phrases
(`terminal session not found`, `ignoring stale yggterm daemon…`, `hot update
failed…`) and on a hit ALSO sets `drop_following_transport_tail_lines = 3` —
deleting the matched line **plus the next three lines** of whatever the CLI was
painting. A Claude Code session whose conversation quotes those phrases (an agent
working on this very bug does) has four lines removed mid-frame. The daemon vt100
screen stays clean, so every daemon-side instrument reports the session healthy —
which is why this survived a run. Making the excision CR-faithful stopped the
staircase garble but not the deletion.

**Why it was NOT fixed in 2.10.14:** the excision cannot simply be removed. `ssh`
writes `Shared connection to <ip> closed.` into the PTY, and yggterm's remote
helper prints `Error: terminal session not found: <key>` to its stdout, which IS
the PTY. Both arrive inside cursor-hide control batches, so no content-based or
branch-based rule separates them from CLI output (5 existing tests lock this).
The real fix is **per-session attach-phase state** — sanitize only while the
launch wrapper owns the PTY, be a faithful pipe once the CLI does. That is the
"collapse the forks / delete the accreted fixes" step of
`campaign-render-pipeline-parity-rework`, which the user sequenced AFTER the
parity harness. Deliberately not rushed into a deploy. The measurement, the
mechanism, and the reason it can't be a one-liner are recorded in code at
`batch_terminal_chunks`. **This is the next thing to do on that campaign.**

**UPDATE 2026-07-20 (run 5): now USER-BLOCKING, and it reproduces hardest on
the busiest remote-CC session.** The user reported a session that "100% never
renders", where closing and reopening the GUI — their standing workaround —
had stopped working. Named session: `remote-cc://dev/029a3955…`
("libyggterm Rebase"). Evidence gathered this run:

- **The corruption is in the client BUFFER, not the paint.** `app terminal
  read-buffer --mode screen` shows three different screen states interleaved
  character-by-character on the same rows (an old report, a test-code frame, a
  `/context` usage panel, plus a stray line-number column). The faithful
  screenshot merely renders that corrupt buffer honestly, so this is NOT a
  canvas/renderer problem — do not chase the renderer again.
- **It survives every repair that does not fix the pipe.** Two real SIGWINCHes
  (PTY winsize verified changing 63×167 → 62×166 → 63×167 on dev, so CC
  definitely re-authored its frame) left the buffer byte-identical in the
  corrupt regions; GUI restarts and repeated `app open` reveals do not stick.
  The attach/replay seed is clean (fixed in 2.10.4), so a fresh reveal paints
  correctly and then **re-corrupts within seconds** of live streaming.
- **Why THIS session and not the neighbouring one.** CC on dev is writing
  ~1.2 MB/s (`/proc/<pid>/io` write_bytes +6 MB in 5 s). High throughput means
  more batches, and the excision is content-triggered — and this session's
  transcript is saturated with the exact transport phrases the sanitizer
  matches ("dropped", "eval failed", "never armed", and it literally quotes
  `terminal session not found`). The calm local session in the same window
  showed no such corruption. That is the "hit hardest" prediction above,
  confirmed on a session the user cannot use.

**CORRECTION, same run — the sanitizers are NOT the cause of THIS symptom.**
It was tempting to file the above under suspect (a) because it matches the
narrative, but the probe refuses it: `terminal_forward_divergence` fired
**3 times in the whole trace, all on an unrelated `live::5d0e22ed…` plain
shell, and ZERO times on `remote-cc://dev/029a3955`**. The GUI forwards the
daemon's bytes faithfully for the corrupted session. Two further facts clear
the excision specifically: the per-line predicate requires a SCHEME-QUALIFIED
match (`local://`, `remote-session://`, `codex-runtime://` — note
`cc-runtime://` is absent), so prose quoting the phrase is already guarded by
`batch_terminal_chunks_keeps_prose_about_missing_sessions`. An attach-phase
gate for `batch_terminal_chunks` was written and then **reverted unshipped**
because it fixed a bug this session does not have. Suspect (a) remains real
for the sessions where divergence DOES fire; it is simply not this.

**The actual mechanism, read off the raw stream.** The agent CLI paints by
skipping unchanged cells with cursor-forward, not by overwriting them — the
daemon-side bytes for this session are literally
`❯ On\x1b[C the\x1b[C meta\x1b[C page` and `t\x1b[8C html`, i.e. every space
and every run of spaces is a CUF. **Cells that CUF skips keep whatever was
already in them.** So once the client buffer's base state diverges from the
frame the CLI believes is on screen, every skipped region shows stale content
and the CLI never rewrites it — permanent, character-by-character
interleaving, exactly what is on screen. It re-corrupts within seconds of a
clean reveal because the very next diff frame paints against the wrong base.

**Next step (unverified hypothesis, do not ship on it):** find where the
post-attach live stream resumes relative to where the attach replay stopped.
A seam — overlap or gap — between the replayed snapshot and the live stream
would leave the client buffer holding a base the CLI never authored, which is
all it takes. A gap is consistent with a high-throughput session being hit
hardest (~1.2 MB/s here). Note that two real SIGWINCHes did NOT repair it,
which needs explaining: a resize normally forces a full repaint, so either CC
did not receive it or its own full repaint is also CUF-based against a stale
model. Settle that first — it discriminates between "client base is wrong"
and "CLI model is wrong".

**FIX SHIPPED 2026-07-23 (2.12.7): the seam is the chunk-ring mid-stream
gap, and `read()` now appends the viewport reconcile after the surviving
tail whenever `resync_required` fires** — the live-path twin of the 2.10.4
attach-seed reconcile (viewport-only, alt-screen-safe, no history
injection, so it does not re-open the 2.8.12/14 trap). Daemon trace
`mid_stream_gap_reconciled` fires per reconcile; lock:
`pty_read_with_trimmed_middle_appends_viewport_reconcile_after_tail`. Full
design + trap analysis:
[`docs/xterm-bugs.md#chunk-ring-trim-drops-mid-stream`](xterm-bugs.md#chunk-ring-trim-drops-mid-stream).
**Remove this entry once re-measured under a busy streaming session**
(read-buffer vs daemon-screen diff staying clean while
`mid_stream_gap_reconciled` fires; the SIGWINCH question is answered by the
mechanism — CC's repaint is diff-based against its own model, so only
re-anchoring the client base can help, which is exactly what the reconcile
does).

**★★ UPDATE 2026-07-25 — A SECOND MECHANISM IN THIS FAMILY, FOUND WITH THE
SHADOW LANE, ROOT-CAUSED AND FIXED. It also answers this entry's own open
question about the SIGWINCHes, and that answer is NOT the one guessed above.**
Reproduced on guihost against a live `remote-cc` session, and settled with GROUND
TRUTH for once: the CC transcript on the remote says `of exam manipulation`
and the terminal painted `uof examrnmanipulation`. Full write-up, including
the socket-probe recipe that measures a screen payload's true width:
[`docs/xterm-bugs.md#screen-model-wider-than-viewer`](xterm-bugs.md#screen-model-wider-than-viewer).
- **The daemon's vt100 SCREEN MODEL had drifted wider than its own PTY** —
  model ~204 columns against a 168x63 PTY and a 168x63 viewer. Everything past
  column 168 is a ghost from when the grid was wider, because the CLI cannot
  paint wider than the grid it was handed.
- **Why that garbles rather than overflows:** the screen is serialized with
  absolute `CSI r;cH` per row and `CSI nC` for runs of blanks. In a narrower
  terminal each over-long row WRAPS, shifting every row below; the later
  absolute jumps land on that spill, and the blank-runs skip cells instead of
  clearing them, so the spill shows through in the gaps. Same CUF mechanism
  this entry already names — but the wrong base is manufactured INSIDE a
  single reconcile write, not inherited from a stream seam.
- **⛔ The SIGWINCH answer above is wrong.** It is not (only) that CC repaints
  diff-wise: `TerminalSession::resize` returned `resize_noop` after comparing
  the PTY alone, so a resize to the size the PTY already had **never touched
  the stale model**. Two real SIGWINCHes could not have repaired it.
- **FIXED in three layers** (2 daemon, 1 client): the served screen is clipped
  to the session's own PTY width at the one place it is served
  (`screen_snapshot_clipped_to_pty_width`); the resize fast path now compares
  the model too and repairs it (`resize_screen_model_repaired`); and the client
  reconcile measures the payload and refuses to paint one wider than its own
  grid (`screen_reconcile_clipped_to_viewer_width`) — which is what protects a
  viewer attached to an OLDER daemon, the live case here.
- ✅ **All three layers are now DEPLOYED on guihost (2.12.13, daemon pid
  1152900, 2026-07-25 evening).** ⚠ But read the next line before assuming a
  given session is covered.
- ⚠ **A daemon-side fix only covers the sessions that daemon OWNS.** After the
  swap, `local://5220ce5d` (the 120x36 shell with the 295-wide model) is still
  served at 295, because a plain shell is not migratable and stays with its
  2.12.12 birth daemon — the durable half of the daemon-chaining bug above.
  For every such stranded session the CLIENT clip is the whole protection, and
  it is proven. Post-swap the two daemon-side events are correctly SILENT: a
  daemon that has just started has no drifted model to clip, so silence here
  is the expected reading, not evidence of a working fix.
- ✅ **The guard HAS now refused an oversized payload live (2026-07-25
  evening, before the 2.12.13 deploy).** Walking every session on the live
  2.12.12 daemon found a **plain `bash -i` shell** — not an agent session —
  with a **120x36 PTY and a model still painting to column 295**. Revealed on a
  read-only shadow running the 2.12.13 client (pinned to the daemon's 120-wide
  grid), it traced `screen_reconcile_clipped_to_viewer_width
  {"screen_max_column":295,"viewer_cols":120}` and painted clean text — with
  the user's GUI untouched. Two consequences worth carrying: **this class is
  not CC-specific** (any session that outlives a window resize can carry it),
  and the mixed-version case layer 3 was written for is now demonstrated, not
  argued (the rail read `Client 2.12.13 · daemon is on 2.12.12`).
- ⚠ **Why it reads as intermittent:** the drift heals on any resize whose grid
  DIFFERS from the cached one; only a resize to the size the PTY already has
  hits the `resize_noop` hole. Same session garbles, "fixes itself" after a
  window resize, garbles again.
- ★ **Method note:** this was found on a shadow client with the user's GUI
  untouched, and the decisive step was reading the daemon's screen off the
  socket instead of trusting any summary field. `server snapshot` is NOT that
  instrument — for a session on a preserved owner it answers with the stale
  stored launch seed, which looks like a healthy session with nothing wrong.


## Remote CC session stays permanently blank: resume-cc deadlocks before it

**Status:** OPEN

**Remote CC session stays permanently blank: `resume-cc` deadlocks before it
launches the CLI (dev, 2026-07-20).** User-reported as "it never renders", and
it is NOT a render bug — the xterm buffer is genuinely empty (0 non-whitespace
chars), so the blank viewport is honest. On the remote host the wrapper
`yggterm server remote resume-cc <uuid> <cwd> --require-existing` sits in
`unix_stream_read_generic` (blocked on a daemon unix socket) for many minutes
with **no children** — it never spawns `claude` at all, so the PTY produces
nothing forever. `Status` in the metadata rail reads `bootstrapping · idle`.

**Neither workaround clears it.** Re-clicking the row just logs
`terminal_bootstrap_existing_lease_skip` ("bootstrap skipped because an
existing attach lease ...") — three attempts in a row did that here, none
reaching `ready`. A full GUI restart does NOT fix it either (verified: fresh
GUI, re-open, still 0 chars), which rules out GUI-side in-memory lease state
as the blocker and matches the user's "even the workarounds do not work".

**Recovery that DOES work:** kill the stuck wrapper on the remote host
(`pgrep -af "resume-cc <uuid>"`, it has no children and holds no user work);
the next open spawns a fresh wrapper which does launch `claude --resume`, and
the session comes back with full scrollback. Confirmed end-to-end on
`remote-cc://dev/75874380…`.

**Prime suspect: the dev daemon fleet.** dev is still running **six**
`yggterm-headless server daemon` processes (the consolidation item carried
from telemetry run 3, [[finding-adopt-gap-untypeable-fixed-2113]]). A helper
that connects to a stale/wrong daemon socket and blocks forever on read is
exactly this signature. Fix direction: (1) consolidate dev's daemons, (2) give
`resume-cc` a connect/read deadline so it can never block indefinitely before
spawning the CLI, and (3) make `terminal_bootstrap_existing_lease_skip`
reclaim a lease whose attach never reached ready, instead of deferring to it
forever.

**FIXES SHIPPED 2026-07-23 (2.12.7, both halves of the recorded direction):**
(2) the wrapper bridge now bails after 120 s if the daemon claims `running`
but the runtime has produced ZERO output ever
(`bridge_running_no_output_deadline` trace; idle-but-healthy sessions are
unaffected — the flag is has-ever-produced-output), so the next open spawns
a fresh wrapper instead of requiring a manual pkill; deployed to dev's
`~/.yggterm/bin` where the wrapper runs. (3) a re-click now RECLAIMS a
bootstrap lease whose attach never reached ready after 45 s
(`terminal_bootstrap_lease_reclaimed_stale_attach`; lock:
`terminal_bootstrap_lease_reclaims_stale_never_ready_attach`). (1) dev
daemon consolidation stays parked with B1 (user call: investigate-only).
Remove this entry once a wedged resume recovers without manual intervention.


## 3.0.0

**Status:** OPEN

3.0.0 — the product does not build for Windows or macOS (NOT NOW; ~2 months out)

**Verified 2026-07-31 against GitHub Actions. User-scheduled: cross-platform is
IN SCOPE for 3.0.0, but 3.0.0 is at least two months away and there is a lot of
work ahead of it — do NOT start this lane unprompted.** Recorded here so it is
not rediscovered a third time. Recovered from the archived memory
`ci-release-cross-platform-failing`.

Two separable facts:

1. **Windows x86_64/aarch64 and macOS aarch64 fail to COMPILE** — not to package.
 Every `release.yml` run has ended in `failure` since ≥2.8.14; the Linux jobs
 pass, so the red went unread for twenty releases. Run 29164529909 (v2.11.0)
 ends with `could not compile yggterm-server (lib) due to 7 previous errors`:
 `env_flag_truthy`, `retire_stale_daemons`, `run_duplicate_legacy_owned_runtime_prune`,
 `versioned_server_socket_alias_candidates`, `parse_versioned_server_socket_name`
 (×3), and `no variant UnixSocket for ServerEndpoint`.
2. **No GitHub release has published since v2.11.0 (2026-07-11)** while guihost runs
 2.12.19 — eight versions exist only as local binaries and fleet `scp`s. This
 half is independent of the compile failure and cheaper to fix.

**The shape, as far as it is actually verified:** the versioned-unix-socket
daemon layer is `#[cfg(unix)]` on its DEFINITIONS — `ServerEndpoint::UnixSocket`
(daemon.rs:934), `parse_versioned_server_socket_name` (daemon.rs:348),
`versioned_server_socket_alias_candidates` (daemon.rs:362),
`refresh_legacy_server_socket_aliases` (daemon.rs:412), `retire_stale_daemons`
(daemon.rs:11791) — and at least one caller is unconditional:
**`lib.rs:81` imports `retire_stale_daemons` with no `cfg`.**

⚠ **The full unguarded-caller list is NOT enumerated yet, and do not guess it
from grep.** An earlier pass in this file cited daemon.rs:432/811/816/962 as
unguarded; every one of those is in fact inside a `#[cfg(unix)]` function or
match arm, and the claim was wrong. Get the real list from the compiler:
`rustup target add x86_64-pc-windows-msvc` works on the fleet (no MSVC linker
needed for `cargo check`), then
`cargo check --target x86_64-pc-windows-msvc -p yggterm-server`.

**When the lane opens:** fix by giving the socket/daemon-topology layer a windows
arm or gating the call sites — not by adding more `#[cfg]` to definitions, which
is what produced this. Add a CI job that `cargo check`s the windows target on
every PR; without that lock it regresses the moment it is fixed.


## ⭐ False/stale gates

**Status:** FIXED IN CODE — LIVE PROOF OWED

**⭐ False/stale gates — PROVEN 2026-08-01 by user screenshot.** The veil
("Daemon updating. Sessions will settle in a moment.") covered the viewport
while the pane beside it read **Client 2.12.22 / Daemon 2.12.22, uptime 35m** —
same version, nothing updating. The giveaway is on the same pane: **"3 owned ·
8 total · 5 preserved"**. `runtime_status_handoff_active()` is
`preserved_terminal_owner_count > 0`, and preserved sessions are a STEADY
STATE, so the veil is armed permanently and every mount shows it. User: *"Daemons
are updating even when same daemon is present"*, *"gating itself is so annoying"*.
**Fix:** arm on a genuine daemon IDENTITY transition (`pid:version` differs
from last observed), and let the awaiting-key slice only SCOPE which surfaces
are veiled. The notice is also raised unconditionally without consulting
`active_view_mode`, which is why it covered a yedit document and claimed "the
terminal is paused". ⚠ The self-check must run IN-PROCESS on the 2.5 s tick:
`server app state` REFRESHES the observation, so an external probe cannot
measure staleness and can itself arm the gate.


## ★★ THE YCHROME VIEWPORT Z-ORDER

**Status:** FIXED IN CODE — LIVE PROOF OWED

**★★ THE YCHROME VIEWPORT Z-ORDER — UNDER-GLASS ARMED ON THE LIVE HOST,
USER CONFIRMATION OWED (was: "every hidden un-hidden trigger breaks and
recomputes the viewport", 2026-07-30).** Phase F under-glass IS the fix the
user described (page at the BACK of the z-order, chrome floating above it,
the terminal-canvas property) and it is now ARMED on the live host
(relaunch env + `~/.config/plasma-workspace/env/yggterm-underglass.sh` for
future logins; `YGGTERM_WEB_SURFACE_UNDER_GLASS=0` reverts).
**Acceptance evidence (2026-07-30 night):**
- Sandbox (headless sway + persistent wlr virtual pointer +
  `scripts/underglass-sandbox.sh`, REAL seat input): click-through the
  glass hole reaches the page (counter incremented); titlebar auto-hide
  reveal via genuine top-edge pointer motion painted ONLY the titlebar
  band — page pixels BIT-IDENTICAL during the reveal, and the whole window
  BIT-IDENTICAL after the cycle; session-switch hide/unhide returned the
  page BIT-IDENTICAL; 5-cycle reveal soak: zero page-region drift; corner
  molding (rounded glass hole) pixel-proven; INCIDENT GUARD: a second,
  never-revealed surface painted ZERO pixels, and closing the active
  surface left no bleed.
- Live host: armed relaunch, rows intact, full-window compositor capture
  + all four viewport-corner crops show the page compositing under the
  chrome with molded corners.
**STILL OWED before closing:** the user's own by-eye/feel confirmation on
real hardware across THEIR triggers (sidebar overlays included — the
sandbox exercised the titlebar reveal and session switches; sidebar
overlays ride the same floats-over-glass machinery but were not separately
driven), and a few days' soak against the 2026-07-26 incident class (a
shell that cannot paint shows whatever is behind it — visibility-truth
keeps unrevealed pages unmapped, and the sandbox guard held, but the
incident fired on the LIVE host's env, so the soak is the honest closer).
Instruments now in-repo: `scripts/underglass-sandbox.sh` (isolated armed
GUI + real-pointer + fast grim frames; see the script header for the
virtual-pointer recipe and its traps).


## ★★ AGENT-SPAWNED TENANTS INSIDE DAEMON-OWNED ROWS ARE IMMORTAL

**Status:** FIXED IN CODE — LIVE PROOF OWED

**★★ AGENT-SPAWNED TENANTS INSIDE DAEMON-OWNED ROWS ARE IMMORTAL — the leak
class behind recurring "mystery heat" (convicted 2026-07-27, user-spotted —
⏳ FIXED IN-TREE AT 2.12.17, LIVE VERIFICATION OWED).**
Seven aged `ssh <fleet-host>` clients (oldest ~5 days) were found hanging
under `bash -i` shell rows on the integrator host, one of them holding a
13.6-hour remote `htop` at 0.16 cores on the GUI host — the user's fan paid
for a probe an agent abandoned days earlier. **Mechanism, and why it is
structural:** an agent uses a shell row for an interactive probe
(`ssh <host>` → a TUI), then abandons the row. Daemon-owned PTYs are
deliberately immortal — the row surviving IS the feature (the GTA-5 model)
— so everything RUNNING INSIDE the row becomes an immortal tenant that no
surface accounts for. The session-start ritual now sweeps this class, but a
sweep repeated every session is an unfixed bug by definition. Product fix,
three pieces, each respecting the settled row doctrine (rows themselves are
never touched) — **all three are now built (2.12.17)**:
1. **Per-row tenant cost visibility (instrumentation, no policy).**
   `server terminal tenants [<session>]`. ONE `/proc` reading serves every
   row, on demand: no loop, no cache, no timer, zero idle cost. It reports
   the foreground command, the whole descendant tree with per-process CPU,
   and the age of the oldest NON-SHELL tenant (the row's own shell is
   discounted, or every row looks aged). A row it cannot walk reports a
   NAMED gap (`preserved_owner_daemon`, `no_local_runtime`,
   `runtime_not_running`, `root_pid_unavailable`, `root_pid_not_in_proc`,
   `proc_unreadable`, `not_supported_on_platform`) with **every number left
   empty** — a faked zero reads as "this row is cheap", which is the lie the
   verb exists to end. A row whose runtime belongs to an older preserved
   owner is PROXIED to that daemon rather than referred to it: a referral
   that the caller must chase by hand is the same archaeology dig this
   replaces.
2. **Ownership stamping on headless creates.** Every agent CLI `terminal new`
   records the creating pid, this host and an optional `--purpose` into the
   row's metadata, and the stamp rides the persisted row across a daemon
   handover (including the preserved-owner adoption import) — so provenance
   outlives its creator, which is the whole point.
3. **Pre-declared ephemerality, opt-in at creation.** `terminal new
   --ephemeral --ephemeral-owner-pid <pid>` or `--ephemeral-idle-ttl-secs
   <n>` = the agent explicitly declares AT CREATION "reap this session when
   my owner is gone / after N idle seconds". **A BARE `--ephemeral` is
   REFUSED** (`EPHEMERAL_NEEDS_AN_EXPLICIT_RULE`): measured, not reasoned
   about — under `bash -c "<cli>"` the parent this CLI would have recorded is
   the wrapper bash, gone in milliseconds, and under `ssh <host> "<cli>"` it
   is sshd-session, gone at disconnect, so the convenient default armed
   owner-gone against a corpse and killed the row on the next chore tick. The
   reap rides the EXISTING background chore tick and closes through the
   daemon's ONE close path (`close_live_session_row`, tombstone before
   remove), tracing `ephemeral_owner_gone` / `ephemeral_idle_ttl` — so it is
   consistent with the requirement-3 ruling: the close is agent-declared up
   front, an explicit close scheduled early. The DEFAULT is unchanged: leave
   the row up, visibility beats tidiness; a declaration is write-once and
   only the agent CLI create path can make one, so unmarked and user-created
   rows are untouchable (the no-reap ruling stands).
Non-product half already done: the ritual sweep gained the aged-ssh probe,
and the twin duty (an interactive probe is exited by the task that opened
it) is recorded in the fleet memory.
⚠ **LIVE VERIFICATION OWED — this entry stays until all four are done on the
live host** (2.12.17 is not deployed; the running daemon has none of this):
a `tenants` walk that actually finds an aged `ssh` tenant under a real row
and names its age; a create-then-stamp round trip read back after the
creating process is gone; ONE real TTL reap observed end to end (declaration
→ chore tick → tombstone → row gone, with the trace event); and the negative,
which is the one that matters most — **unmarked rows, including the user's,
untouched across that same tick.**


## ★★ AN AGENT'S TEARDOWN CAN REPORT SUCCESS AND LEAVE BOTH THE ROW AND THE

**Status:** FIXED IN CODE — LIVE PROOF OWED

**★★ AN AGENT'S TEARDOWN CAN REPORT SUCCESS AND LEAVE BOTH THE ROW AND THE
APP PROCESS ALIVE (user-reported 2026-07-26 ~23:50, third variant of the
same class that night — ⏳ BOTH HALVES FIXED IN-TREE AT 2.12.17, LIVE PROOF
OWED).** A background agent's final report said "work session removed"; the
user still saw the row hours later. Ground truth: the row was live, and the
app process it hosted was still running under its `bash -i`, parented by the
daemon. Two things made it invisible to search:
1. **`terminal new --kind shell` names every session "Workspace Shell"**, so
   an agent's scratch row is indistinguishable by title from a human's shell
   — and the campaign record separately flags "Workspace Shell" as a name a
   HUMAN debugging session has used, which makes blind cleanup dangerous.
   ⇒ Give agent-created sessions a title carrying the agent identity and
   purpose (the chip already carries the app profile — the TITLE should too).
   **FIXED:** creation through the app-control plane — and only that plane,
   since every human door funnels through `start_local_session_placed` and
   never reaches it — synthesizes `Agent <identity> <kind>[: <purpose>]` from
   the request's own `agent` field plus a new `--purpose` flag, parsed
   identically by both binaries. An explicit `--title` still wins, and the
   synthesizer asks `looks_like_generated_fallback_title` about its OWN
   output before shipping it, because a title the copy layer discards falls
   straight back to the humanized cwd leaf — the exact bug it exists to
   prevent.
2. The row's only records marker was the small profile chip, so every
   title-based probe missed it while the user's eyes found it instantly.
**Also wanted, and now built:** a teardown verb that is verified, not
asserted. `session remove` used to hardcode `"accepted": true` on any
successful round trip — transport success and nothing else; it read true
while the daemon's own message said "no live session for this path", and true
while the PTY teardown (which signals ONLY the direct child, never its
descendants) left the hosted app running. It now answers from evidence:
census the PTY child's process tree from `/proc` before, re-read the row and
re-probe each censused pid after (matched on command name so a recycled pid
cannot pose as a survivor, rejecting zombies so a corpse cannot), with a
bounded settle so a child still handling the hangup is not misreported. One
pure owner, `verify_session_removal`, turns that into
`{verified, refusal, reaped, still_running}`, and `verified:false` carries a
NAMED refusal: `row_still_listed`, `processes_survived`, or
`runtime_pid_unobservable` — the last being the cross-version case the
constitution warns about (a row whose runtime belongs to an older preserved
owner reports no local pid, and that is **unverifiable, not clean**).
Reporting only: the verb does not kill survivors, because escalating to that
changes what a removal does to a human's shell and is a separate call.
⚠ **LIVE PROOF OWED (this entry stays until then):** on the live host, TRY TO
MAKE IT LIE — remove a session whose shell forked a process that outlives the
PTY, and confirm the verb says `verified:false` with `processes_survived` and
names them; and confirm an agent-created row wears its own name in the
sidebar the user is looking at.
Pairs with the leased-surface-with-no-row entry: the two failure modes are
opposites (invisible surface vs invisible-to-search row), and both are
fixed by making agent-owned artifacts NAME themselves.


## ★★ web fill-card ADVErecordsSED WHAT THE CREDENTIAL PLANE FORBADE

**Status:** FIXED IN CODE — LIVE PROOF OWED

**★★ `web fill-card` ADVErecordsSED WHAT THE CREDENTIAL PLANE FORBADE (found live
2026-07-26 at a real payment gateway's card form — FIXED IN-TREE, LIVE
VERIFICATION AND A DEPLOY PENDING).** The verb's help offered `--field
number|expiry|code|holder` while every agent call came back
`vault_cli_no_card_op`: yggterm reached the vault through the **CLI**, which
deliberately has no card op, while `card-secret` existed all along as an
**agent-socket** op the ychrome sidebar was already using. The agent burned a
staged application and an OTP discovering this at the last step.
**Route (b) was taken and then simplified by the user's ruling:** every
Bitwarden client can read a card cipher, ychrome-vault is one, so the UNLOCK
is the boundary and the only one — no grant, no per-use consent. `fill-card`
now speaks the agent socket directly, the field set is
`number|code|holder|exp-month|exp-year|expiry`, the only policy refusal is
`vault_locked` (which names `ychrome-vault unlock`), and every release leaves
one line in `~/.yggterm/vault/audit.log` naming field names, never values.
ychrome branch `agent-card-path`, commit `13a3bfe`.
**What is still owed:** neither repo is pushed or deployed, and no PAN has
crossed the path into a real form. The yggterm half works against the ALREADY
RUNNING vault agent (it only uses `card-secret`, which ships today) except for
the socket-path lookup, which reads `socket` from `ychrome-vault status` — a
field the deployed ychrome-vault does not yet report. **So a live run needs
the new `ychrome-vault` installed + `ychrome-vault handover` first** (cheap:
handover keeps the unlock), or the verb refuses with
`vault_agent_socket_unknown` naming exactly that. yggterm deliberately does
NOT fall back to a hard-coded `~/.yggterm/vault/agent.sock`: ychrome owns that
path, and a second copy of it is what goes quietly wrong the day it moves.


## ★★ A --no-activate CREATE MADE WHILE NO SESSION IS ACTIVE STILL ACTIVATES

**Status:** FIXED IN CODE — LIVE PROOF OWED

**★★ A `--no-activate` CREATE MADE WHILE NO SESSION IS ACTIVE STILL ACTIVATES
THE NEW ROW — the adjacent gap left behind when the sidebar-selection jump was
fixed (⏳ FIXED IN-TREE AT 2.12.17, LIVE PROOF OWED).** With the start page
showing, an agent's `terminal new --no-activate` pulled the viewport onto the
agent's row. Selection was preserved in that case; activation was not.
**CAUSE.** The create's hand-back captured the user's view as
`Option<(path, view_mode)>`, where `None` meant BOTH "no session was active"
and "nothing to hand back" — and the restore read the second meaning, so it
no-opped on exactly the case that needed it, leaving the daemon snapshot's
activation of the new row standing.
**FIX (GUI-only, no daemon or protocol change).** The viewport becomes a
NAMED state: `PreservedViewport` is either `StartPage` or
`Session { path, view_mode }`, so the outer `Option<PreservedUserView>` is the
only thing that still means "this create hands nothing back". The start-page
restore goes through the same SSOT setter the viewport history's own
`StartPage` entry uses, and `show_start_page_when_no_live_sessions` is forced
FALSE rather than restored — while that flag is true, every later snapshot
promotes the first live row back to active, which is precisely the row the
create was told not to activate, so restoring it would re-open the bug on the
next poll. The create response's `null` active path is now true as well as
honest: it already reported `null` while the shell had in fact activated the
new row.
**RESIDUAL, stated rather than hidden:** the hand-back is client-local. The
daemon still marks a newly started session active whatever the flag said, so
any path that adopts daemon truth wholesale re-adopts the new row. The honest
fix for that half is daemon-side.
⚠ **LIVE PROOF OWED at the next bump (a J7 item covers it):** with the GUI on
the start page, `terminal new --no-activate` must leave the start page
rendered and report a null active path.


## 💬 DISCUSSION for the dev agent

**Status:** AWAITING A DECISION

💬 DISCUSSION for the dev agent — a remote desktop wears browser chrome, and the protocol cannot say otherwise (2026-08-01)

**Not a bug report and not a decided fix — a design call that needs one.** Filed
from an end-to-end UX pass over yRDP driven the way a person drives it (a
throwaway shadow client, a scratch Xvfb+VNC target, real pointer clicks through
`server app pointer`), because the thing that stood out was not broken, it was
*wrong-looking*.

**What you see.** Connect to a remote desktop from the yRDP chooser and the
desktop is revealed as a web surface — correct, that IS the transport (x11vnc →
websockify → a noVNC page on loopback). What comes with it is the whole browser:

- an address bar reading `http://127.0.0.1:6102/index.html?quality=9&compression=0&bg=262a33`
- back / forward / reload / history buttons
- the tab rail, listing this "tab" beside unrelated ones

None of that is addressable by the user in any useful way. The URL is a bridge
detail — a port yRDP chose seconds ago — and it is the one piece of text in the
window that looks like something you could type into. Reload re-dials the
bridge; Back has nowhere to go. A Windows desktop is not a page you browse, and
the frame says it is.

**Why it cannot be fixed app-side today.** `TerminalEvent::WebSurface` carries
`{action, session, url, title, profile, start_page}` — there is no presentation
field, so an app has no way to say "this surface is not a web page". On the
GUI's side `web_chrome_hidden` already exists and already does exactly the right
thing (omnibox, find bar and tab strip all collapse) but it is wired to ONE
input: `snapshot.page_fullscreen`, i.e. an element-fullscreen page. The
mechanism is built; nothing but the engine can reach it.

**The shape of the decision** (the dev agent's to make, and the reason this is
here rather than in a commit):

1. **A declared flag** — `bare: true` / `presentation: "surface"` on the
 web-surface open, feeding the existing `web_chrome_hidden`. Smallest change,
 and it puts the choice with the app that knows what the surface IS. Cost: a
 protocol field, and a permanent question of who else gets to claim it (a page
 that hides the address bar is also how a phishing surface would like to
 render — worth stating that the flag comes off the PTY, which a page cannot
 write, so the trust boundary is the same one the declare already has).
2. **Infer it** — no chrome for a surface whose URL is loopback and whose opener
 declared a viewport pane. No protocol change; a heuristic that will be wrong
 for the next app, and inference is what the geometry contract's whole story
 is about not doing.
3. **A third surface kind** beside terminal and document — honest, and much more
 work: it needs its own context menu, its own switch in the titlebar, its own
 place in the presentation policy.
4. **Leave it.** Defensible while yRDP is the only consumer and the operator is
 the only user. It stops being defensible the moment a remote desktop is
 something a customer sees.

**What is NOT in question:** the surface plumbing itself is right — one canonical
session, N viewers, scaled never resized. This is only about what the GUI draws
around it.

Evidence and repro live in this session's yRDP UX pass; the scratch target
recipe (two `*.toml` files, Xvfb + x11vnc, `YRDP_TARGETS_DIR`/`YRDP_STATE_DIR`
pointed at a temp dir) reproduces the whole flow on any host with no guest and
no risk to a live one.

## ⚠ `ygg-claim.sh` PRINTS RAW `/proc` ERRORS INTO A SUCCESSFUL CLAIM — and they sit ABOVE the line that says it worked

**Status:** OPEN

**Reported by the `practice` campaign row 2026-08-09** (filed here rather than
sent, because row 6 was not consuming input at the time — `input-check` said
`consuming_input:false`, `activity:unknown`, transcript idle ten minutes).

Real output from today's claim on `guihost` (replacing `8ffdb7e3`):

```
15:42:22 ygg-claim remove: row_still_listed=False verified=False remote_runtime_survived
ygg-claim.sh: line 275: /proc/3270168/cmdline: No such file or directory
15:42:22 ygg-claim reaping surviving agent pids:2777668 2777671
ygg-claim.sh: line 275: /proc/3270508/cmdline: No such file or directory
ygg-claim.sh: line 275: /proc/3270545/cmdline: No such file or directory
15:42:24 ygg-claim predecessor retired and reaped clean
```

The pids exit between the listing and the `/proc/<pid>/cmdline` read. That race
is **normal and expected** — the script is reaping them, and it succeeds. But
the unguarded read turns it into three `No such file` lines interleaved with the
progress log, and the verdict that says it actually worked (`reaped clean`) is
printed **below** them. A caller scanning for failure reads the errors first.

**Why this is worth a line and not a shrug:** this is the **first output of every
relay session on every campaign**, and it is exactly where the reader is deciding
"did my claim work". The script's own conclusion is correct; the noise contradicts
it. It is the same family as the entry at the top of this file — an instrument
reporting in a register that makes success look like failure.

**Fix:** guard the read (`[ -r "/proc/$pid/cmdline" ] || continue`, or redirect and
skip). Not a logic change — the reaping is right.

**Falsifier when fixed:** a `ygg-claim.sh --replace <uuid>` run whose predecessor
exits mid-reap prints no `No such file` lines.

## ⭐ `ygg-claim.sh` SPILLS RAW /proc READ ERRORS DURING A SUCCESSFUL REAP — THE GUARD IS ORDERED AFTER THE REDIRECT IT GUARDS

**Status:** OPEN

**Reported by another campaign row 2026-08-10 15:23 on the GUI host**, relayed via
a third row; observed while claiming a row with `--replace`:

```
15:23:28 ygg-claim remove: row_still_listed=False verified=False remote_runtime_survived
…/ygg-claim.sh: line 275: /proc/335446/cmdline: No such file or directory
15:23:28 ygg-claim reaping surviving agent pids:2954590 2954602
…/ygg-claim.sh: line 275: /proc/336084/cmdline: No such file or directory
…/ygg-claim.sh: line 275: /proc/336139/cmdline: No such file or directory
15:23:29 ygg-claim predecessor retired and reaped clean
```

The claim SUCCEEDED (`predecessor retired and reaped clean`, seat read back verified) —
but three raw bash errors mid-run read as a partial failure, and the next agent's
instinct is to re-run the claim or start hand-checking `/proc`: exactly the
hand-walking the script exists to prevent. Same family as fleet skill §7 — the
instrument must not report something other than what happened.

**Root cause (diagnosed at filing; deliberately not fixed from the reporting
session):** in `agent_pids()`:

```sh
c="$(tr '\0' ' ' < "/proc/$p/cmdline" 2>/dev/null)" || continue
```

The `2>/dev/null` is a NO-OP for precisely the failure it was written for.
Redirections apply left to right: when `< /proc/$p/cmdline` fails to OPEN — the
EXPECTED case, since pids enumerated by pgrep exit moments later while a reap is in
progress — the shell reports the error to whatever stderr is in force *at that
moment*, which is still the script's own; the trailing stderr redirect is never
reached. The `|| continue` then works, which is why the run still succeeds.

Fix shape: order the stderr redirect before the input redirect
(`tr '\0' ' ' 2>/dev/null < "/proc/$p/cmdline"`) or wrap the read in a
`{ …; } 2>/dev/null` group so a vanished pid silently `continue`s — and sweep the
script for the same redirect ordering on any other `/proc` read. Falsifier for the
fix: a `--replace` claim over a predecessor with live children exits clean with no
bash diagnostics on stderr.

---

## ⛔ THE BOOTER WILL NOT WAKE A SESSION WHOSE TURN HAS ENDED, IF ITS TRANSCRIPT WAS WRITTEN RECENTLY — `WORKING` IS DECIDED BY FILE GROWTH

**Status:** OPEN

*Measured 2026-08-11 11:09–11:11 by a relay row on a private campaign. This is the
INVERSE of "DEFECT 1 — a corpse answers faster than a worker": there a dead session
looked alive; here a session that has **stopped taking turns** looks alive for the
same reason — recency of transcript growth.*

**The decision line, from the booter's own log:**

```
11:09:02 ygg-booter WORKING           6.8m  -            <row>  win=25m/<window-note>
11:09:02 ygg-booter WORKING           0.8m  -            <row2> win=7m
```

**What was actually true at 11:09.** That row's turn had **ENDED** minutes earlier
(mid-handover, having created a successor row and stopped before submitting its
brief). It was not working; it was finished and idle. The booter saw a transcript
last written 6.8 minutes ago, classified `WORKING`, and declined to boot — so the
relay stayed asleep and its half-finished handover sat there, with an empty
successor row holding nothing.

⇒ **`WORKING` means "the file grew recently", which is a proxy for two very
different states**: a session mid-turn (must NOT be interrupted) and a session
whose turn ended seconds ago (SHOULD be booted). The one signal cannot separate
them, and the failure is silent in the direction that matters — an unattended lane
stays unattended.

### ⭐ THIS LIKELY EXPLAINS AN EARLIER UNEXPLAINED GAP, NOW THAT THE LOG IS BACK

On the morning of 2026-08-11 another relay row measured a **~27-minute** delay
between `armed + poll interval` and its actual next wake, listed three candidate
causes — missed polls, *"a turn not seen as ENDED"*, or a boot issued and not
delivered — and could not choose between them **because the decision log was dead**
(stale since 2026-08-10 12:57 while the heartbeat was live). ⇒ **The log is ALIVE
again** (fresh entries at 11:09:02 and 11:11:39), and the surviving evidence names
the second candidate. Worth confirming against that row's window before closing it.

### ⚠ AND A SECOND, SMALLER DEFECT THAT BLOCKS EXACTLY THIS AUDIT

**The `boots` counter is reset by a re-arm/defer.** Same subscriber file,
`subscribed_at` UNCHANGED at 09:49:15 throughout:

```
10:59:34  <row>  sibling  age=1.2h  boots=1      <- a boot HAD been delivered
11:11:44  <row>  sibling  age=1.4h  boots=0      <- after a routine defer/arm
```

A campaign that re-arms every turn therefore reports `boots=0` forever. ⇒ **the one
field that would prove delivery is destroyed by normal operation**, which is why
the earlier investigation had nothing to count. Keep a monotonic
`boots_total` (or `last_boot_utc`) that a defer never touches.

### Fix shape

- Separate **"mid-turn"** from **"turn ended, idle"**. The CLI already knows when a
  turn ends; if the transcript's last record type is available, an ended turn is
  readable from it rather than inferred from mtime. Failing that, treat
  `WORKING` as requiring growth **within the last poll interval**, not within
  ~7 minutes, and let the deferral window govern the rest.
- Never let a subscriber's own `arm`/`defer` clear delivery evidence.
- Falsifier: a subscribed row whose turn has ended and whose transcript was last
  written 5–10 minutes ago must be booted at its deferral deadline, and the log
  must show why.

# THE 6.9 BATCH — found while building the phone's transport

Four tooling defects, none of which blocked the phone lane, all found by using
the CLI as an instrument rather than by reading it. Each is filed against the
thing that owns it, not against the lane that tripped over it.

## ⛔⛔ [6.9] `server status --json` IS NOT A FLAG, AND IT FAILS BY GOING QUIET

**Status:** OPEN

*found 2026-08-13 while measuring response weights for the phone*

`server status` already emits JSON. Passing `--json` — which every neighbouring
verb accepts (`server daemons --json`, `server gate-screen --json`,
`server perf-summary --json`) — prints usage and leaves **stdout empty**.

**Why this is worse than a plain error.** The natural way to measure a response
is `server status --json | wc -c`. That returns `0`. A previous session did
exactly this, got `0` for both `status` and `snapshot`, and wrote the conclusion
into a design document as a measured fact: *"no daemon is running on the host
this session sits on."* A daemon was running, with hundreds of sessions. The
document then carried that as the reason a load-bearing number could not be
obtained.

⇒ **A flag a tool does not have is indistinguishable from a service that is not
there, if the only thing you look at is the byte count.**

**Fix:** accept `--json` as a no-op on the verbs that already emit JSON, *or*
write the usage to stderr and exit non-zero. Either removes the silent-empty
path. The first is friendlier, because the flag is guessable precisely because
its neighbours take it.

**Falsifier:** `server status --json | wc -c` returns a number greater than zero,
or the command exits non-zero.

## ⛔ [6.9] THE ROW-CLEANUP VERB IS NAMED IN THREE PLACES AND EXISTS IN NONE OF THEM

**Status:** OPEN

*found 2026-08-13 cleaning up a throwaway session*

The documented cleanup step for a probe row is written as `session remove`. That
is not a verb. Three separate corrections are needed before it works:

1. `server session remove …` answers `unsupported server command: session`. The
   real verb is **`server app session remove`**.
2. It is app-control, so it only answers **on the host where the GUI runs**. ⭐
   The error for this is genuinely good — it names the candidate hosts and the
   exact command to identify the right one. That message is the model the rest of
   this entry should be held to.
3. It removes the row **from the GUI**, and the owning daemon still lists the
   session afterwards. Ending the process took writing `exit` and then a lone
   carriage return into the PTY.

⇒ Three different questions — *what is the verb*, *where does it answer*, *what
does it actually remove* — and the documented name answers none of them. This is
the standing pattern: **a row verb reports the request, not the effect.**

**Fix:** correct the name wherever the cleanup step is written, and say both the
GUI-host constraint and the fact that removal is a GUI-side operation. If a
caller wants the runtime gone, that needs to be a separate, named thing.

**Falsifier:** the documented cleanup line, copied verbatim, removes a probe row
and its runtime.

## ⚠ [6.9] `server attach` DOUBLE-PREFIXES A KEY THAT ALREADY CARRIES A SCHEME

**Status:** OPEN

*found 2026-08-13 creating a throwaway session*

Creating a session with a key of the form `local://<name>` produces a session
whose key is `local://local://<name>`. It is then addressable only by the
doubled form, so every later call has to repeat the mistake to work.

**Fix:** strip an existing scheme, or refuse the argument. Silently concatenating
is the one option that makes the caller carry the error forward.

**Falsifier:** attaching with a scheme-qualified key yields a session listed
under exactly that key.

## ⛔⛔ [6.9] A REPOSITORY THAT HAS NEVER HOSTED AN AGENT SESSION CANNOT HOST A ROW

**Status:** OPEN

⚠ Root cause NOT found. The inherited explanation is wrong and is falsified
below; do not build around it.

*reported by the orchestrator 2026-08-13: three attempts to create a row with a
cwd in one particular repository, three failures to ever consume input, against
five contemporaneous launches into sibling worktrees that came up normally —
same host, same verb, same model, same minute*

**What is falsified.** The claim as stated — that this repository cannot host a
row — is not about the repository and not about the agent CLI. Run directly in
that directory, the CLI answers a prompt normally and exits zero; the identical
control in a working worktree behaves the same. So the failure is in the
**row-launch path**, not in the destination.

**The one hard datum.** No project namespace directory existed for that cwd
despite three launch attempts, so those launches never reached session
initialisation at all. A row appeared; nothing behind it started.

**A hypothesis tested and rejected**, recorded so it is not re-derived: the
obvious candidate is a first-run trust gate on an unseen directory. It does not
hold. None of the sibling worktrees that host working rows has a trust entry
either, so the presence or absence of one does not separate the failures from
the successes.

⚠ **RE-TESTING THAT CWD IS NOW CONTAMINATED.** Probing it created the namespace
that was previously absent, so it no longer starts from the original condition. A
clean reproduction needs a cwd that has never hosted an agent session.

**Falsifier:** a row created with a cwd that has never hosted an agent session
reaches a composer and consumes input.

## ⛔⛔ [6.1] `server attach --help` CREATES A SESSION NAMED `--help` AND SPAWNS A LIVE SHELL

**Status:** OPEN

*Hit on a real daemon by accident, 2026-08-14.*

`attach` reads its flag as a positional session name. Asking a verb for its usage
therefore **mutates the machine**: a session called `--help` is created and a live
`bash -i` is spawned on the daemon. Recovery was manual — identify the pid and
`SIGHUP` it, because **an interactive bash ignores `SIGTERM`**.

⇒ A help flag is the one argument a user types when they do *not* know what the
verb does, so it is the worst possible argument to give side effects. Same family
as the rest of this file: **the verb answers a different question than its name
suggests**, and here the wrong answer is durable.

**Falsifier:** `server attach --help` prints usage, exits non-zero or zero without
creating a session, and `server sessions` shows no new row afterwards.

## ⛔ [6.1] SUPERSESSION CANNOT FIRE WHEN THE SOCKET PATH EXCEEDS THE SOCKADDR LIMIT

**Status:** OPEN

`live_newer_daemon_socket` scans `$YGGTERM_HOME` for `server-*.sock`. But
`default_endpoint` **relocates** the socket to `/run/user/<uid>/yggterm/h-<hash>/`
whenever the home path would exceed the **108-byte `sockaddr_un` limit**. When it
relocates, the scan looks in a directory the socket is no longer in, finds
nothing, and **no daemon ever retires — silently**.

⚠ **Harmless on the real hosts** (`~/.yggterm` is short), which is why it has
never been seen in production. It is not harmless for testing: it makes any
long-path sandbox **structurally unable to exercise handover at all**, and it cost
a full run before the cause was found. ⇒ The enumerator must ask
`default_endpoint` where the socket IS, never assume where it would be — the same
shape as *enumerate an alias set from the versions that EXIST, never from the
files that happen to REMAIN*.

**Falsifier:** a sandbox whose `YGGTERM_HOME` exceeds 108 bytes still supersedes a
predecessor daemon.

## ⚠ [6.1] A PTY-SPAWNING TEST MUST HOLD THE TERMINAL-IDENTITY GUARD

**Status:** OPEN

A test that spawns a pty reads the **process-wide** terminal-identity env, so
without `codex_cli::env_test_guard` it makes the two identity tests in `lib.rs`
flaky — **not itself**. Six tests did exactly this.

⭐ **The tidy explanation was wrong and was killed by a control:** *"extra tests
just perturb the schedule"* did not survive **six DECOY tests that left the suite
green**. A flake that moves when you add unrelated tests still has a cause.
