# The idle cost model: what yggterm spends at rest, and on what

**Owner: this file answers "what does the running system cost when nobody is
using it, and which term dominates".** It does not say what is open (the queue
owns that) or what shipped (git + CHANGELOG own that). Entries in
`docs/pending-bugs.md` point here for the derivation rather than copying it.

Measured 2026-08-14 on a fleet host carrying 15 census daemons and 57 sessions.
Every figure below is a windowed `/proc` delta, never `ps %CPU`.

## 0. The instrument, and its controls

Every sample in this file ships **both controls in the same run**: a spinner
that must read ~1.00 cores and a sleeper that must read ~0.00. A run whose
controls do not bracket is void and is not quoted here.

    controls: spinner 1.002 cores, sleeper 0.000 cores -> valid

Host clocksource is `tsc`, so `clock_gettime` is served from the vDSO and the
45.8x `hpet` penalty documented elsewhere does **not** apply to any number in
this file. ⛔ **Nothing here transfers to an `hpet` host without re-measurement.**

## 1. Where the cost actually is — the daemon population, not the GUI

25 s window, all yggterm-family processes, grouped by role:

| class | n | cores | user | kernel | RSS |
|---|---|---|---|---|---|
| **daemon** | 19 | **3.468** | 0.889 | **2.580** | 5.48 GB |
| web process | 9 | 0.387 | 0.334 | 0.053 | 2.68 GB |
| ychrome | 11 | 0.238 | 0.117 | 0.122 | 0.64 GB |
| CLI client (ssh-carried) | 11 | 0.039 | 0.015 | 0.024 | 0.60 GB |
| **gui** | **1** | **0.010** | 0.009 | 0.002 | 0.15 GB |
| net process | 20 | 0.002 | 0.002 | 0.000 | 1.53 GB |
| other | 37 | 0.023 | 0.015 | 0.008 | 1.40 GB |
| **total** | 108 | **4.167** | 1.380 | 2.787 | 12.48 GB |

⛔ **CORRECTED — the first version of this table classified on the command line
and got the `gui` row wrong.** All 12 processes it called `gui` were
`yggterm server remote start-cc|resume-cc` wrappers, whose command line contains
the binary path. **And the known remedy — key on `comm`, not the command line —
would ALSO have failed here**, because `comm` is `yggterm` for the GUI *and* for
an ssh-carried CLI client. ⇒ **Only the SUBCOMMAND separates them.** The daemon
row was never affected (it matches `server daemon`, which no wrapper carries),
so the 83% headline is unchanged at **83.2%**.

⇒ **The daemon population is 83% of the whole footprint, and 74% of its own cost
is kernel time.** Prior work in this area measured the GUI and the web process,
which are the visible surfaces. On a host that accumulates daemons, they are the
minority term.

## 2. The model: cost is per-DAEMON, not per-session

Least squares over the 14 census daemons (one excluded as a user-time outlier,
see §2a):

| model | coefficients | R² |
|---|---|---|
| cores ~ AGE | −0.000408 /h, intercept 0.291 | **0.323** |
| cores ~ ROWS | 0.000739 /row, floor 0.102 | 0.473 |
| cores ~ OWNED | 0.0125 /session, floor 0.153 | 0.863 |
| **cores ~ 1 + OWNED + ROWS** | **floor 0.116, 0.0104/session, 0.000337/row** | **0.939** |

⛔ **Age has no explanatory power, and its slope is NEGATIVE.** Older daemons
cost slightly *less*. This is the single most important line in this file,
because it contradicts the growth story that holds for the GUI: the GUI's idle
cost climbs 7.4x over its life, and **the daemon's does not climb at all**. Two
different defects wearing one symptom (a hot machine). A fix aimed at leak-hunting
in the daemon is aimed at nothing.

The consequence, stated as the ratio that survives instrument error:

| | daemons | sessions | cores | cores/session |
|---|---|---|---|---|
| legacy (pre-3.0.149) | 14 | 34 | 3.012 | **0.0886** |
| current (3.0.151) | 1 | 23 | 0.456 | **0.0198** |

⇒ **A session costs 4.5x more when it is one of few on its own daemon than when
it is one of many on a shared one.** The cost follows the process, not the work.

**What consolidation is worth.** All 57 sessions on one daemon, per the joint
model: **0.864 cores against 3.468 today — 2.60 cores reclaimable, 75%.**

### 2a. Two honest limits on the model

1. **The intercept cannot be read as a bare-process cost.** Four daemon
   processes on this host are absent from the census
   (they are build-tree binaries on unreachable socket paths). They measure
   **0.000 cores** — a clean natural zero. But no *census* daemon owns zero
   sessions, so the fit cannot separate "a per-daemon floor" from "a step paid
   by the first session".
   ✅ **ANSWERED IN §6e — and it is NEITHER of those.** The floor is the cost of
   answering peer `status` polls, paid at a rate set by the daemon *population*.
   The four zero-cost daemons are unreachable, so nobody polls them.
   ⛔ **CORRECTED §6j: "they own zero sessions" was FALSE of three of the four.**
   Each holds live interactive shells as children. Their zero is explained by
   *not being polled*, not by *owning nothing* — and this section had already
   said so in its next clause while the sentence above contradicted it.
   **Two adjacent claims, one true, and the wrong one was the memorable one.**
   ⛔ **The experiment proposed here would have given the wrong answer:** a
   reachable census daemon parked at zero owned sessions still pays the full
   floor, which would have been recorded as a per-daemon constant rather than as
   a per-poll cost — the right number attached to the wrong cause.
2. **One daemon is excluded** (3 sessions, 0.648 cores, 0.396 of it user time
   — 12x the user time of its peers). Including it drops R² from 0.863 to 0.240
   on its own. It is a genuine outlier with a different shape, ⇒ **now
   attributed in §5**, and it should stay out of the fleet fit.

### 2b. The per-ROW term is REAL and ROBUST — and NOT yet attributed

**Robustness, because n=14 with 3 parameters deserves it.** Leave-one-out over
all 14 census daemons:

| coefficient | min | max | sign stable |
|---|---|---|---|
| per-row | +0.000293 | +0.000390 | yes |
| per-session | +0.0076 | +0.0108 | yes |
| floor | +0.110 | +0.124 | — |

`corr(OWNED, ROWS) = +0.48` — moderate, not collinear enough to make the joint
fit meaningless. Dropping the highest-leverage point entirely (the 23-session
daemon) moves per-row only to **0.000322**. ⇒ **The term survives every single
deletion.**

**What it is worth.** Rows summed over the 15 census daemons = **1,953**, at
0.000337 cores/row ⇒ **≈0.66 cores spent on row-scaled work**. ⛔ **SUPERSEDED —
that figure is a fitted coefficient, and the quantity was later measured directly
at 4.6–4.7 µs/row (§6h), which makes the term ≈0.033 cores.** Do not quote 0.66. ⛔ **And each
daemon's ROWS is frozen at its birth** (the bequest carries the list forward), so
a 261-row daemon owning 23 sessions is carrying 238 rows it does not own.

### ⛔ A HYPOTHESIS TESTED AND REFUTED — do not re-run it

`run_background_copy_chore` (`BACKGROUND_COPY_CHORE_MS = 12_000`) runs in every
daemon and its own comment says it walks *"every codex + Claude Code transcript
on this machine"*. That is exactly the right shape — host-scaled work paid by
every daemon — so it was the obvious candidate.

**It is not the driver.** Sampling `rchar` at 1 Hz for 48 s (four full chore
cycles), the read spikes on the 246-row daemon land at **~5–6 s intervals, not
12 s**, at a suspiciously constant ~655 KB each:

| daemon | reads over 48 s | spike period |
|---|---|---|
| 1 session, **246 rows** | 7.8 MB | **~5–6 s** |
| 1 session, **100 rows** | 1.0 MB | none (one isolated spike) |
| **0 sessions (control)** | 4.5 MB | none (one isolated spike) |

⇒ **The period does not match, so the 12 s chore is not what the per-row term is
buying.** Two further cautions from the same run: the **control read 4.5 MB
while costing 0.001 cores**, so reads are *not* a proxy for this cost; and the
gap between the 100-row and 246-row daemons is far larger than 2.5x, so whatever
scales with rows is **not linear in reads** even though the cost term fits
linearly.

⇒ **Next probe, and it is a period hunt, not a byte hunt:** find what fires
every ~5–6 s per daemon and re-reads a constant ~655 KB. ⭐ Take the rate over
the whole window rather than off the first few timestamps — three clustered
samples are not a period, a mistake this campaign has already paid for once.

✅ **ATTRIBUTED IN §6d/§6e — and the period hunt was the wrong quarry.** The
per-row term is not background work on a timer at all. It is **per-request work
multiplied by a request rate the daemon does not set**: every peer `status` poll
rebuilds and sorts the daemon's entire row inventory, ~3.6 times a second.
⛔ **The ~5–6 s / 655 KB read spike remains unexplained and no longer matters** —
this section had already established that reads do not proxy this cost (the
0-session control read 4.5 MB while costing 0.001 cores), which was the standing
reason not to spend another window on bytes. The quantity that mattered was a
**request rate**, and it was in the trace the whole time.

## 3. The mechanism: one OS thread per connection, and the CPU hides in the dead ones

**The instrument disagreement that found it.** Sampled over the *identical*
window, process-level CPU exceeds the sum over live threads by ~5x:

| daemon | process cores (6 windows) | Σ live threads | verdict |
|---|---|---|---|
| A (1 session) | 0.208 0.204 0.212 0.232 0.146 0.208 → **0.202** | 0.041 | DIVERGE |
| B (1 session) | flat → **0.207** | 0.043 | DIVERGE |
| C (9 sessions) | flat → **0.214** | 0.122 | DIVERGE |
| D (23 sessions) | flat → **0.460** | 0.159 | DIVERGE |
| E (0 sessions) | **0.001** | 0.000 | OK |

The six windows are flat, so this is **not** burstiness averaging out — it is a
real disagreement. `/proc/<pid>/stat` counts threads that have already exited;
`/proc/<pid>/task` cannot. ⇒ **The missing CPU is being spent in threads that
are born and die between two samples.**

Confirmed directly, 30 s at 50 Hz, fully passive:

| daemon | live threads at start | distinct TIDs seen | new | churn/s |
|---|---|---|---|---|
| A (1 session) | 10 | 137 | 127 | **4.23** |
| B (1 session) | 10 | 129 | 119 | 3.97 |
| C (9 sessions) | 26 | 157 | 131 | 4.36 |
| D (23 sessions) | 55 | 812 | 757 | **25.22** |
| **E (0 sessions)** | 12 | 12 | **0** | **0.00** |

Every new thread is named `yggterm-daemon-*`; 96 of 100 caught were in state
`R`, actively running, not blocked. The source is one site:

    crates/yggterm-server/src/daemon.rs:785
    fn spawn_unix_client_handler(...) {
        let thread_name = format!("yggterm-daemon-client-{}", current_millis());
        std::thread::Builder::new().name(thread_name).spawn(...)

**A fresh OS thread per accepted connection**, each taking
`Arc<Mutex<DaemonRuntime>>` — one global lock. At 4.23 threads/s carrying
0.161 cores, each handler burns **~38 ms of CPU**. The zero-session daemon
churns exactly zero threads and costs exactly zero, which is the negative
control for the whole mechanism.

## 4. The largest writer in the system is an instrument reporting "no problem"

20 s window on the live daemon, parsing the trace as it was written:

| | |
|---|---|
| `event-trace.jsonl` growth | **95.3 KB/s** → 8.43 GB/day |
| `perf-telemetry.jsonl` growth | **45.9 KB/s** → 4.06 GB/day |
| **combined** | **141.1 KB/s** → **12.49 GB/day** |
| events parsed (event-trace) | 13,091 |
| `lock_wait_begin` / `lock_wait_end` | 6,456 / 6,456 |
| **failed `try_lock()` per second** | **322.8** |
| share of event-trace that is lock_wait | **98.8%** |
| share of perf-telemetry that is `terminal_read` | **96.4%** |
| both streams' footprint **on disk** | **~20 MB** (they rotate) |

Contention by request kind: `terminal_read` 12,736 · `status` 164 · `snapshot` 12.
⇒ **98.6% of all lock contention in the daemon is `terminal_read`** — reading a
PTY serializes against every other operation through one mutex.

### It is ONE code path, and the arithmetic closes exactly

`perf-telemetry.jsonl` records `terminal_read` **322/s** — the same rate as the
failed `try_lock()`. That is not a coincidence: the `PerfGuard` is constructed
**inside the contended branch**, beside the two `append_trace_event` calls. So a
single contention writes three records across two files:

    141.1 KB/s ÷ 322.8 contentions/s = 437 bytes written per contention

⇒ **All 12.49 GB/day is attributable to the contended branch of one function.**
Both files, both instruments, one cause. A fix that treats them as two problems
will fix one and leave 4 GB/day behind.

### ⛔ And the field that would have shown this reads zero

`waited_ms` distribution across 6,456 waits:

| waited_ms | 0 | 1 | 2 | 3–9 | ≥10 |
|---|---|---|---|---|---|
| count | **6,060** | 236 | 48 | ~91 | ~21 |

⇒ **93.9% of contention events report `waited_ms: 0`.** The field is integer
milliseconds and essentially every wait is sub-millisecond, so the instrument
built to measure lock contention **prints zero on the overwhelming majority of
the contention it is recording.** Anyone reading a `lock_wait_end` record
concludes there is no contention problem. The *count* is the signal; the
*value* is blind.

⚠ **The doc comment on the function is now false, and it is false in the
direction that hides the cost.** It states:

> *Fast path costs nothing … an uncontended acquisition traces nothing and
> allocates nothing. Only a request that actually has to wait pays for — and
> reports — its own wait.*

The code does implement that (`try_lock()` first, trace only on `WouldBlock`).
The premise that failed is the assumption that `WouldBlock` is rare: it fires
**322.8 times a second**. Each one costs a `resolve_yggterm_home()`, two
`serde_json::json!` allocations, and **two file appends — to report a 0 ms
wait.** The fast path is not the path being taken.

### ⚠ It is 22x smaller on the host that actually gets complained about

Same 15 s window, both hosts, measured independently by the implementing lane:

| host | sessions / daemons | combined write rate |
|---|---|---|
| this (integrator) host | 340 / 21 | **133 KB/s** |
| the desktop host | 2 / 7 | **6 KB/s** |

⇒ **22x, and it tracks session count, not hardware.** That is a *confirmation*
of §2 rather than a contradiction of it: contention is 98.6% `terminal_read`,
and `terminal_read` is per-session, so the trace volume must scale with sessions
and it does.

⛔ **But it means this defect is close to absent on the machine the fan
complaint came from.** Fix it on its own merits — CPU, IO and SSD wear on the
host that carries the fleet. **Do not offer it as the explanation for his fan.**

### ✅ 4a. S2 IS MEASURED AND PASSES — 493x, against a 90x threshold

*2026-08-14, on the fleet host, on the one daemon whose build carries
`670fa66d` — confirmed by `git merge-base --is-ancestor`, never by version
number.*

| | |
|---|---|
| windows emitted | 31, covering **1,861 s** |
| contentions **counted by the aggregate itself** | **131,243** (70.5/s) |
| bytes the new code actually wrote | **116,344 B — 62.5 B/s** |
| bytes the old code would have written | 131,243 × 437 B = **30.1 KB/s** |
| **reduction** | **493x** |

⭐ **The control the entry demanded turned out to be unnecessary, because the
fix's own record carries the number of events it replaced.** An aggregate that
reports `count` makes its own counterfactual *countable* — old volume is
`count × bytes-per-old-event`, measured on the same daemon in the same window,
with no second daemon, no earlier baseline, and no session-population confound
to argue about. ⇒ **When a fix replaces N events with one summary, put N in the
summary.** That is what turned an unrunnable comparison into a one-daemon
measurement.

⚠ **The one inherited constant is 437 B/contention** (§4's 141.1 KB/s ÷ 322.8/s),
and the verdict does not depend on it being right: at a deliberately pessimistic
**100 B**/contention the reduction is still **113x**, above the threshold. So a
4.4x error in the constant would not change the answer.

⛔ **And the cross-daemon control that was specified would have MISLED, for a
reason worth keeping.** Run at the same moment over 180 s, the 14 pre-fix
daemons wrote **no `lock_wait` records at all** — their entire trace volume is
`begin`/`end` request tracing. There was nothing on the control arm to compare
against, because contention is a property of *load*, and the load had moved to
the one busy daemon. A per-owned-session normalisation on top of that would have
been worse than useless: total write volume was roughly **flat at 1.0–2.5 KB/s
regardless of owned count (1 to 10)**, so dividing a near-constant by a varying
denominator manufactures a trend that is not there. ⇒ **Normalise only by a
quantity the numerator has been shown to scale with.**

⭐ **Residual, and it is not this defect:** `lock_wait_slow` — the deliberately
preserved forensic path — is **84% of the remaining 62.5 B/s**, and it is
earning its keep. In one hour: **520 slow `terminal_read` waits, 79 slow
`status` waits, and a `status` that waited 9.89 s** for the runtime lock. That
is §S3's evidence, now visible where it previously rounded to `waited_ms: 0`.

## 5. A SHIPPED fix is not a RUNNING fix — the outlier, identified

§2a excluded one daemon as a user-time outlier (3 sessions, 0.648 cores, 0.396
of it *user* — 12x its peers). It is now attributed, and it turns out to be the
strongest argument in this file for §S1.

Its hot thread is `yggterm-perf-incident-monitor` (`daemon.rs:17016`), which
wakes every 30 s and calls `summarize_perf_telemetry` for the last 60 s. Reading
bytes, 30 s window:

| daemon | rchar MB/s | **per 90 s** | read syscalls/s | bytes per read |
|---|---|---|---|---|
| **outlier, 2.12.14, 451 h** | **4.86** | **437.8 MB** | **0.6** | **8.1 MB** |
| 2.12.17, 430 h | 0.39 | 34.9 MB | 0.5 | 780 KB |
| 3.0.52, 137 h | 1.35 | 121.9 MB | 5,161 | 262 B |
| 3.0.151, 3 h (live) | 2.58 | 232.4 MB | 37,064 | 70 B |
| **0 sessions (control)** | 0.16 | 14.0 MB | 0.2 | — |

⇒ **437.8 MB per 90 s, at 0.6 read syscalls per second.** That is not streaming;
that is whole files being swallowed at 8 MB a read. It is above the **312.9 MB
per 90 s** that the DAEMON-1 defect measured when it was found — and that defect
was **root-caused and fixed on 2026-07-26**, by `jsonl_read_paths_since`
(`retention.rs:141`), which decides from a generation's filename which files a
windowed read must open at all.

**This process started ~451 h ago — the same day as the fix.** It has never
restarted, so it is still running the pre-fix behaviour: a question about the
last 60 seconds paid for with a read of the entire retained corpus, every 30 s,
forever. The excess is **user** time because the cost is parsing the JSON it
just read.

⛔ **The general form is the finding, not this one daemon.** Each of the 14
legacy daemons carries every defect fixed since its own version — 2.12.14
through 3.0.62, against a current 3.0.151. **A fix that shipped is not a fix
that is running.** The queue and CHANGELOG record these as closed, and for 14
processes holding 34 live sessions they are not.

⇒ **This is a third independent argument for §S1**, and it is the one that does
not saturate: cost and row-deaths are bounded by the population, but the set of
fixes a legacy daemon is missing grows with every release.

⚠ **And it interacts with §4.** `perf-telemetry.jsonl` is being filled at
4.06 GB/day *by the lock tracer*. The retained corpus is what the incident
monitor reads. So §4 inflates the very corpus §5 pays to parse — fixing §S2
reduces §5's cost on every daemon, including the ones that cannot be fixed.

## 6. THE POPULATION POLLS ITSELF — one mechanism behind BOTH unattributed terms

§2a left the floor unattributed and §2b left the per-row term unattributed. They
are **the same mechanism**, and it is not work the daemon does for itself: it is
work the daemon does *for its peers*.

### 6a. `status` is 70% of everything the daemons are asked

Parsing the trace as it was written, two independent 60 s windows:

| | window A | window B |
|---|---|---|
| `request/begin`, all daemons | 70.5/s | 82.7/s |
| of which **`status`** | **49.7/s (70%)** | **56.0/s (68%)** |
| of which `working_flags` | 20.5/s | 26.4/s |
| `snapshot` | 0.25/s | 0.32/s |

⛔ **And the per-daemon rate is FLAT.** Every census daemon receives
**3.4–4.2 `status`/s**, and the spread does not track anything the daemon owns:

| daemon | owned | rows | status/s |
|---|---|---|---|
| oldest legacy | 3 | 73 | 3.85 |
| mid legacy | 1 | 90 | 4.03 |
| late legacy | 1 | 246 | 4.07 |
| current | 5 | 261 | 4.70 |

⇒ A daemon carrying 261 rows is polled at the same rate as one carrying 73.
Whatever sets this rate is **outside** the daemon.

### 6b. The N=1 control — the poller is the daemon population itself

The same host runs an **isolated single-daemon installation** (a sandbox arm with
its own `YGGTERM_HOME`), same binaries, same kernel, same `tsc` clocksource, with
a live GUI attached to it. Both arms measured over the same 60 s:

| arm | daemons | request rate | **`status`/s** |
|---|---|---|---|
| isolated installation | **1** | 0.07/s | **0.00** |
| census installation | **15** | 82.7/s | **56.0** |

⇒ **A single-daemon installation with a GUI running against it receives zero
status polls.** The load is not the GUI's and not the clients'. It appears only
when there is a *population*, which means the daemons are polling each other.

If each of N daemons swept its N−1 peers every T seconds, each daemon would
receive (N−1)/T, and the fleet numbers fit that: 4.0/s at N=15 ⇒ T = 3.5 s,
predicting 60/s against 51–56/s measured. The independently observed burst
period is **3.24 s** (19 bursts in 60 s, median 121 requests over 14 *distinct*
daemons) — the same T, arrived at without assuming it.

### ⛔⛔ RETRACTED: "the cost is quadratic in N". THE CONTROL WAS CONFOUNDED AND THE CAUSAL ARM DID NOT REPRODUCE IT

**This section first claimed the cost goes as N(N−1). That claim is withdrawn.**
Two failures, both mine, found by continuing to test my own result:

1. ⛔ **The N=1 arm above is CONFOUNDED.** That installation has
   **`OWNED=0, PRESV=0, ROWS=0`** as well as N=1. "No peers" and "nothing to poll
   about" are not separated by it, so its 0.00/s supports the quadratic reading
   no better than it supports a session-driven or reference-driven one.
2. ⛔ **A deliberate causal arm failed to reproduce the scaling.** Daemons of five
   distinct versions were started in an **isolated home with zero sessions**,
   varying only N:

   | N | sessions | status/s | N(N−1)/3.5 predicts |
   |---|---|---|---|
   | 1 | 0 | **0.00** | 0 |
   | 2 | 0 | **0.17** | 0.57 |

   Peer polling is **real but small**: two daemons with nothing to do poll each
   other at 0.085/s each, against the fleet's **3.9/s — 46x higher**. Scaling
   N=2 → N=15 by N(N−1) predicts 17.9/s; the fleet measures 51–56/s.
   ⚠ **N could not be pushed past 2:** a daemon owning no sessions retires, which
   is precisely why the fleet's legacy daemons persist — their sessions block it.
   Adding sessions to raise N would have reintroduced the confound the arm
   existed to remove.

⇒ **What the poll rate scales with is NOT ESTABLISHED.** It is not bare N (arm),
not the receiving daemon's own sessions or rows (flat across OWNED 1–9, ROWS
70–261), not the sender's preserved-owner count (outbound churn is flat across
PRESV 0–29, and the daemon with **zero** preserved owners has among the highest),
and not client count (this host carries **3** client processes and **1** GUI,
which has its own home). The remaining candidate — untested — is the **density of
cross-daemon references**: preserved owners and rows whose runtime lives on
another daemon. That grows with the population *and* with accumulated state,
which would explain the arm reading ~zero while the fleet reads 3.9/s.

⭐ **WHAT SURVIVES, AND IT IS THE PART THE SPECS REST ON.** Every claim in §6d–§6e
is about **cost per poll at an observed poll rate**, and none of it depends on
knowing who polls or how the count scales:

- `status` is 70% of requests at 51–56/s fleet-wide — a measured count.
- Every daemon receives 3.4–4.2/s — a measured count.
- `fn status(&self)` rebuilds the whole row inventory per call — read from source.

⇒ **§S5 is unaffected** (it cuts the cost of each poll). **§S1 keeps its original
linear justification and loses the revision this section briefly gave it.**

### 6c. It is not demand-driven — a null control says so

The obvious story is that a client request fans out. It does not. Lead-lag over
60 s, counting `status` begins at *other* pids within 300 ms of each candidate
trigger, against a null built by jittering the trigger timestamps:

| trigger | n | observed | null | ratio |
|---|---|---|---|---|
| `working_flags` | 1,408 | 13.16 | 16.14 | **0.82** |
| `snapshot` | 16 | 10.38 | 15.62 | 0.66 |
| `status` | 3,203 | 20.73 | 15.83 | 1.31 |

⇒ `working_flags` sits **below** its null, so it triggers nothing. Only the mild
`status`→`status` self-clustering survives, which is the burst shape itself. **It
is a timer inside each daemon, not a reaction to a request.** ⚠ Without the null,
"13 statuses follow every working_flags" would have read as a smoking gun.

### 6d. Each poll costs a thread AND the whole row inventory

**The thread.** Thread births are as flat as the poll rate — 2.37–3.80/s across
all 15 daemons, matching status/s one-for-one. Every poll is a fresh unix
connection and a fresh OS thread at `daemon.rs:785` (§3's mechanism, now with its
driver named: §3 asked what churns threads on an idle daemon, and the answer is
that the daemon is not idle — it is answering its peers).

**The rows.** `fn status(&self)` (`daemon.rs:4154`) is unconditional and, on
*every* call, rebuilds:

    let stored_terminal_sessions = self.server.stored_sessions_persisted();
    let mut stored_terminal_session_keys = stored_terminal_sessions
        .iter().map(|s| s.path.clone()).collect::<Vec<_>>();
    stored_terminal_session_keys.sort();          // O(R log R), every poll
    stored_terminal_session_keys.dedup();
    let live_terminal_sessions = self.server.persisted_state().live_sessions;
    //                            ^ the ENTIRE persistence payload, rebuilt

plus a second clone/extend/sort/dedup for `terminal_session_keys`. The response
struct carries `owned_/terminal_/preserved_/stored_terminal_session_keys` **and
the full `PersistedStoredSession` records** (path, kind, id, cwd, title).

⇒ **Every peer poll reconstructs and serialises the daemon's whole row
inventory.** The doc comment explains the design honestly — the records ride on
status "because the reconcile already fetches status" — and that is exactly the
trap: a field placed where a *rare* operation would find it convenient, on a
request that turned out to run **3.6 times a second forever**.

### ⛔⛔ 6e. REFUTED BY §6g — "both terms are `status`" is FALSE. Kept for the derivation trail only.

**All of status serving is ≈1.6% of the population's cost and under 1% of the
fitted floor.** Neither the floor nor the per-row term is the peer poll. The
arithmetic below "closed" because both of its inputs were derived quantities
(§6e's own 94 µs/row, and §3's 38 ms/handler) — two ratios agreeing is not
corroboration when both inherit the same unexamined assumption.

    cost ~= (N-1)/T x (fixed + k x ROWS)

- **Floor.** 3.6 polls/s x per-poll fixed cost. The model fitted floor 0.116;
  §3's independently measured ~38 ms per connection handler gives
  3.6 x 38 ms = **0.137 cores**. Same quantity, two routes.
- ⭐ **And this is why the four zero-owned daemons measure exactly 0.000.** §2a
  read that as a clean natural zero for a bare process. It is not: those four sit
  on **unreachable socket paths**, so no peer can poll them. **The floor is not
  the cost of existing — it is the cost of being REACHABLE.** That is §2a's open
  question answered, and it answers it in a way the proposed experiment (park a
  census daemon at zero owned) would have got **wrong**: a reachable daemon owning
  nothing would still have paid the floor, and the floor would have been recorded
  as per-daemon rather than per-poll.
- **Per-row.** 0.000337 cores/row / 3.6 polls/s = **~94 µs of CPU per row per
  reply** — a plausible price for cloning, sorting and serialising one record.
  For a 246-row daemon: 23 ms per poll, 0.083 cores continuous.
  ⛔⛔ **DIRECTLY MEASURED AT ~11 µs/row, NOT 94 — SEE §6g. The inference above
  is an order of magnitude high and §S5's headline rests on it.**

### ⛔ 6g. THE PER-POLL COST WAS MEASURED, AND IT IS ~8x SMALLER THAN §6e INFERRED

§6e derived ~94 µs/row by dividing the model's coefficient by the poll rate. The
daemons already write a `PerfGuard` **duration per request**, so it can be read
instead of inferred. 90 s window, grouped by pid, paired against each daemon's
ROWS:

| rows (band) | median handler duration |
|---|---|
| 70–101 (10 daemons) | **2.09–2.57 ms** |
| 243–246 (3 daemons) | **2.92–3.17 ms** |

    median_ms ~ ROWS:  slope 11.0 us/row,  intercept 1.32 ms,  r = +0.683

⇒ **The row term is REAL and directly confirmed — duration rises monotonically
with rows — but the slope is 11 µs/row against the 94 µs/row §6e inferred.**
Serving one `status` costs ~2.4 ms, not the ~38 ms §6e assumed from §3.

⇒ **If this holds, §S5 returns ≈0.08 cores, not ≈0.66**, and status handling is
~5% of a daemon's idle cost rather than most of it — which reopens the larger
question of **where the other ~95% of daemon CPU goes.**

⛔ **WHY THIS IS FLAGGED AND NOT YET APPLIED TO §S5's HEADLINE.** The instrument
records **`daemon_request/status` at 1.8/s against 49.7/s arriving — ~3.6% of
requests** (n=5–22 per daemon where ~324 were served). `PerfGuard` has no
sampling logic; the shortfall is unexplained, and an instrument recording 3.6% of
events is not one to rewrite a headline on. ⚠ Two further scope limits: the guard
drops when `handle_request` returns, so **JSON serialisation and the socket write
are outside it** (serialising ~50 KB adds well under 1 µs/row, so this does not
close the 8x gap); and `duration_ms` is **wall time, not CPU**, so it is an upper
bound on handler CPU — which makes §S5 smaller still, never larger.

⛔⛔ **AND THE SLOPE WAS WRONG TOO — THE SAMPLING IS A FILTER ON THE DEPENDENT
VARIABLE.** I kept 11 µs/row on the grounds that *"a uniform under-recording
factor cancels in a between-daemon comparison"*. **The fragile/robust split was
the right question; my assignment was wrong, because the keep-rule is not
uniform.** Read from source (`crates/yggterm-core/src/perf.rs:54–116`):

    const NOISY_SPAN_RECORD_FLOOR_MS: f64 = 8.0;
    const NOISY_SPAN_SAMPLE_RATE: u64 = 50;

    fn perf_span_should_record(category, name, duration_ms) -> bool {
        if !perf_span_is_high_frequency_noise(category, name) { return true }
        if duration_ms >= NOISY_SPAN_RECORD_FLOOR_MS { return true }   // <-- the trap
        COUNTER.fetch_add(1) % NOISY_SPAN_SAMPLE_RATE == 0
    }

`("daemon_request", "status")` is on that noise list. ⇒ **A record is kept when
it is SLOW.** High-row daemons are slower, so a larger share of their replies
clears the 8 ms floor and they are preferentially recorded — **the sampling
correlates with the very variable being fitted, so it cannot cancel.** Measured
enrichment on one live stream: 243–246-row daemons put **13.5–16.8%** of records
above the floor against **3.5–7.6%** for the 70–101-row ones.

| estimate | method | µs/row |
|---|---|---|
| §6e inference | coefficient ÷ poll rate | 94 |
| naive fit on sampled records | this section, as first written | ~10–11 |
| **inverse-probability weighted re-fit** | same field data, corrected | **4.71** |
| **independent sandbox counter** | CPU not wall, six seeded row counts, unsampled | **4.645** (r=+0.9981) |

⇒ **~4.65 µs/row is the measurement**; 94 and 11 are both refuted, and the two
sound methods agree within 1.4%.

⛔⛔ **THEREFORE §6e's HEADLINE IS REFUTED: NEITHER TERM IS `status`.** All of
status serving is **≈0.057 cores of the 3.47-core population — 1.6%**, and
request serving explains **under 1%** of the fitted floor. The floor is not the
cost of answering peers, and the per-row term is not the row inventory on the
poll path. ⇒ **§S5 has been decided AGAINST** (see its entry): a census split
buys ~1.5% while paying with a version-gated protocol change whose
`#[serde(default)]` failure mode is a documented row-loss hazard at handover.

⭐ **THE RULE, WHICH IS BIGGER THAN THIS FIT: read the sampling predicate before
deciding whether sampling cancels.** A rule that keeps records above a duration
threshold is not a uniform filter — it is a **filter on the dependent variable**,
and any slope fitted through it is biased toward whatever makes the quantity
large.

⇒ **§2b's per-row term is real and now attributed, and the refuted candidate
stays refuted for a better reason than its period.** `run_background_copy_chore`
was never going to be it: the row term is not background work at all, it is
**per-request work multiplied by a request rate the daemon does not control**.
⛔ **The period hunt in §2b was the wrong quarry** — chasing a ~5–6 s read spike
of ~655 KB. Reads were already known not to proxy this cost (§2b's own caution:
a 0-session control read 4.5 MB while costing 0.001 cores). The quantity that
mattered was a **request rate**, and it was visible in the trace the whole time.

### ✅ 6h. MEASURED DIRECTLY: 4.645 µs/row, and status serving is 1.6% of the daemon population

*2026-08-14, fleet host. A counter in `fn status()` itself — every reply, nothing
sampled, `CLOCK_THREAD_CPUTIME_ID` rather than wall. Six seeded row counts,
isolated homes, one variable.*

⚠ **The probe was priced on the host before it was trusted**, because the fleet
spread on this call is 45.8×: `CLOCK_THREAD_CPUTIME_ID` **578 ns**,
`CLOCK_PROCESS_CPUTIME_ID` **619 ns**, `CLOCK_MONOTONIC` **26.7 ns** (vDSO,
`tsc`). Two calls per reply at 3.6 replies/s is ~4 µs/s — four orders of
magnitude below what it measures.

| ROWS | CPU µs/reply (payload build) | CPU µs/reply (whole request) |
|---|---|---|
| 0 | 36 | 383 |
| 50 | 236 | 664 |
| 100 | 409 | 898 |
| 250 | 955 | 1,582 |
| 500 | 2,133 | 3,089 |
| 1000 | 4,676 | 6,247 |

    payload build   slope 4.645 us/row   intercept  -63 us   r = +0.9981
    whole request   slope 5.857 us/row   intercept  289 us   r = +0.9985

⇒ **The row term is real, linear over the fleet's range, and small.** At the
measured poll rate across the census's **1,956 rows**: **0.033–0.041 cores**.
Adding the per-reply floor (289 µs × 3.6/s × 15 daemons = 0.016 cores), **all of
status serving is ≈0.057 cores of the 3.47-core daemon population — 1.6%**, and
request serving explains **under 1%** of §2's fitted per-daemon floor.

⭐ **Three numbers for one slope, and the two that agree were reached
independently.** 94 µs/row (§6e, inferred from the model — refuted);
**10.2–11.0 µs/row** (§6g, mean over `PerfGuard` records — biased, below);
**4.645 µs/row** here, against **4.71 µs/row** from re-fitting §6g's own field
data with each record inverse-probability weighted. Field-plus-reweighting and a
controlled counter are not the same instrument, and they land 1.4% apart.

⛔ **§6g's shortfall was not unexplained, and its slope does not survive
either.** `("daemon_request", "status")` is on
`perf_span_is_high_frequency_noise`'s list (`crates/yggterm-core/src/perf.rs:60`):
a span is written only at **≥ 8 ms** or on a **1-in-50** sample — which is where
the "~3.6% of requests" comes from. One live stream held 5,604 sub-floor records
against 2,769 tail records: recorded tail share **33% against a true 0.98%**.
§6g kept the slope on the grounds that *"a uniform under-recording factor
cancels in a between-daemon comparison."* **The factor is not uniform** — the
keep-rule is a threshold on duration, and duration is what rows drive: the
243–246-row daemons put **13.5–16.8%** of their records above the floor where
the 70–101-row daemons put **3.5–7.6%**. Each daemon is biased differently, in
the same direction as the effect being estimated.

⛔ ⇒ **§6e's headline claim — that `status` is BOTH unattributed terms of the
model — is refuted. It is neither.** The row term is attributed to it and is
worth ~1.5%; the floor is not, and **where the daemon's CPU actually goes is
open**.

**Two known biases, both stated because both make the number an under-read:**
the seeded rows carry short synthetic strings where real ones carry longer
titles and cwds; and the harness drives 69–290 replies/s where the fleet runs at
3.6/s, so caches stay warmer. Generous allowance for both still leaves status
serving under ~4.5%. ⚠ The harness also seeds **empty** `session_pty_grids`,
`ssh_targets` and `remote_machines`, which zeroes three terms the live path
pays — they are tens of µs per reply, under 0.001 cores fleet-wide.

⇒ **What was built instead of §S5** is in the queue entry: `status` was asking
for the whole persistence payload to use one field of it. Splitting out
`persisted_live_sessions()` removes a second full stored-row walk, a PTY-grid
clone and sort, two table clones and a `HashSet` per reply — **same wire, no
version gate, no protocol risk.**

**Its paired A/B, on the identical harness, same host, same seeds, binary the
only difference:**

| ROWS | CPU µs/reply before | after | | whole request before | after |
|---|---|---|---|---|---|
| 0 | 36 | 31 | | 383 | 353 |
| 100 | 409 | 386 | | 898 | 842 |
| 250 | 955 | 943 | | 1,582 | 1,564 |
| 1000 | 4,676 | 4,402 | | 6,247 | 5,912 |

    payload build   4.645 -> 4.365 us/row   (-6.0%)   both fits r = 0.998
    whole request   5.857 -> 5.530 us/row   (-5.6%)

⚠ **Read this as code hygiene, not as a performance win, and the numbers say so
themselves.** Six percent of a path that is 1.6% of the daemon population is
~0.002 cores — below anything a user could perceive, and **no CHANGELOG entry
was written for it.** ⛔ **Quote the paired SLOPE, never a single arm:** the
per-arm deltas run −1.3% to −13.9% and are not monotone in ROWS, which is
run-to-run noise of a few percent; the slope over six paired points is the only
part that survives it.

⭐ **The durable half is a dependency removed, not microseconds saved.** The
reply path no longer scales with the PTY-grid table, the ssh-target list or the
machine table at all — three terms this harness seeds EMPTY and the live fleet
does not, so the A/B above understates the change on a real daemon while
overstating nothing.

### ⛔ 6i. THE NEGATIVE CONTROL: A LONE DAEMON SPENDS NOTHING — AND IN COMPANY IT CANNOT EXIT

*2026-08-14, fleet host. The control §6h needs: same binary, same seeded row
counts, own `YGGTERM_HOME` so nobody polls it — and **no request sent**. If the
cost is request serving, this reads ~0. If it is intrinsic, this is where it
shows up.*

| seeded ROWS | idle cores over 60 s | ticks of CPU |
|---|---|---|
| 0 | 0.00017 | 1 |
| 100 | 0.00000 | 0 |
| 264 | 0.00017 | 1 |
| 1000 | 0.00100 | 6 |

⚠ **Read the resolution before the numbers.** `CLK_TCK` is 100, so one tick is
10 ms and the floor of this instrument over a 60 s window is **0.00017 cores**.
Three of the four arms are at or below one tick — **indistinguishable from
zero**, not measured as zero. That blindness does not matter here only because
the bound it gives is already **~680x below** the 0.116-core floor being tested.
Only the 1000-row arm (4x the fleet's largest daemon) rises clear of it, at
0.001 cores.

⇒ **There is no intrinsic per-daemon floor.** A daemon holding up to 264 rows
and no sessions, that nobody talks to, costs less than 0.0002 cores.
⇒ Combined with §6h — a served poll costs 383 µs, so 3.6/s is **0.0014
cores/daemon** — **neither existing nor being polled explains 0.116.**
⇒ **So the intercept is either attached to owned sessions, or it is a fitting
artefact.** §2a already said the fit cannot separate a per-daemon floor from a
step paid by the first session, because no census daemon owns zero; and the
replication that took R² from 0.939 to 0.109 is what a spurious intercept looks
like. ⛔ **§2a's ✅ "ANSWERED IN §6e — the floor is the cost of answering peer
polls" is withdrawn.** Answering peer polls is 1.2% of it.

⚠⚠ **READ THE SCOPE, BECAUSE THE HEADLINE IS EASY TO OVER-READ.** *"A lone
daemon spends nothing"* is **not** *"daemons spend nothing"*. This daemon owns
**zero sessions and zero preserved owners**, and that is where the difference
with the live population lives:

- The machine-wide corpus walk is **not** the difference — `daemon_background_
  copy_chore_enabled()` reads an env var that is **unset by default**
  (`daemon.rs:11354`), so it is off in this sandbox *and* off on the live
  daemons alike.
- What IS different is the part of the same chore that is **not** gated: the
  per-owned-session halves — the CC title sync, the live/remote candidate
  collection, the preserved-owner revalidation — which that function's own doc
  says *"keep running regardless"*. A daemon owning nothing does none of it.

⇒ **This control rules out an intrinsic per-DAEMON cost. It says nothing about
per-SESSION cost, by construction** — and the live-population arm's finding
(periodic chore threads carrying the bulk) sits exactly there. The two results
agree rather than compete: the standing cost tracks what a daemon **owns**, not
that it exists and not that it is polled.

#### ⭐ And the control found the mechanism that keeps the population alive

Every arm **retired itself after 75 s**, logging `daemon idle shutdown,
idle_shutdown_ms=90000`. That is not a fault — it is
`daemon_should_idle_shutdown` (`crates/yggterm-server/src/daemon.rs:11589`)
doing exactly what it says. Two gates:

1. `if terminal_session_count > 0 { return false }` — **a daemon owning any
   session, including a preserved hot-update PTY, never idle-retires.**
2. Otherwise it needs **90 s with no request**. And `mark_daemon_activity` is
   called on **every** request, unconditionally, before dispatch
   (`daemon.rs:19423`).

⇒ **In a population, gate 2 is unreachable.** Peers poll each daemon at
**3.4–4.2/s** against a 90 s window, so the timer is reset roughly 300 times
over before it could ever fire. **A daemon drained to zero sessions still would
not idle-retire while any peer remains** — the population keeps its own members
alive, and this control is the proof of the converse: remove the peers and the
same binary exits in 90 s having burned nothing.

⚠ **Scope, stated so it is not over-read.** This is the *idle-shutdown* path,
not the *successor-retire* path (`daemon.rs:15274`, 20 s poll), which is what
actually drains superseded daemons and carries its own deferral gate. The claim
is that **idle-shutdown cannot be the drain's mechanism in a live fleet** — not
that nothing can retire a daemon. ⇒ It matters to §S1: a drain that migrates
sessions off a daemon and then waits for it to notice it is idle will wait
forever.

#### ⚠ What this control does NOT cover — the OUTBOUND half of a poll

§6h measured the daemon **serving** a `status`. Nothing has measured a daemon
**sending** one, and the population sends as many as it receives (~3.4/s each).
A first bound, through the nearest available proxy: `server daemons` — a CLI
that runs the same fan-out over all 15 peers and then does strictly more
(`/proc/<pid>/exe` per daemon, formatting) — costs **0.117 s of CPU, ~7.8 ms per
peer**.

⛔ **That is an UPPER BOUND obtained through a proxy, and it is DERIVED when
multiplied by a rate — treat it as neither measured nor attributed.** Even taken
at the bound, 7.8 ms × 3.4/s × 15 daemons is **0.40 cores, ~11%** of the daemon
population. ⇒ **The outbound half does not close the gap either**, but it is the
largest unmeasured term left in the request path and it should be measured in
the daemon rather than bounded from outside.

### ✅ 6l. S6 BUILT — THE HANDLER, MEASURED END TO END IN CPU, PER VERB

*The instrument §S6 asked for: `CLOCK_THREAD_CPUTIME_ID` around the WHOLE closure
at `spawn_unix_client_handler`, not `handle_request`'s wall-time `PerfGuard`.
Expected effect **zero cores** — it attributes cost, it removes none.*

Sandbox, 264 seeded rows, no sessions, two consecutive 60 s windows:

| verb | n | CPU µs/conn | wall µs/conn | kernel share |
|---|---|---|---|---|
| `status` | 10,184 | **1,385** | 1,423 | **7.6%** |
| `ping` | 5,092 | **126** | 128 | 3.3% |

⭐ **`ping` is the per-connection FLOOR, measured: 126 µs** — accept, spawn the
thread, read, dispatch, serialise, write, tear down, for a request that does
nothing. And `status` at 264 rows costs **1,385 µs**, of which §6h's payload
build is 1,070 µs; the residual ~315 µs is that floor plus serialising the
reply. **The three numbers close.**

⛔ **PER VERB, AND THE FIRST VERSION WAS NOT — the tell was arithmetic, not
suspicion.** A single mean over every connection came out **below the cost of
the payload build it contains**, which is impossible: 3,385 `status` handlers at
1,374 µs were being averaged with 1,694 `ping` handlers at 125 µs. ⇒ **A mean
over a mixed population of verbs is not a handler cost**, and it reads low by
whatever fraction of traffic is cheap.

⛔⛔ **THE ~94% KERNEL FIGURE THAT MOTIVATED §S6 IS RETRACTED — and the reason is
this section's own coarse-field trap, one level down.** It came from
`/proc/<tid>/stat` fields 14/15, where **`utime` and `stime` are each floored to
a 10 ms tick INDEPENDENTLY**. Flooring the two components of one thread
separately annihilates the smaller and drives the share to 100% for the larger.
Measured against `getrusage(RUSAGE_THREAD)` on threads built with a known mix:

| true mix | true kernel share | what the ticks said |
|---|---|---|
| 100 ms user / 100 ms kernel | 76.5% | 78.9% — fine |
| 4 ms user / 20 ms kernel | 83.3% | **100.0% — user annihilated** |
| 1 ms / 2 ms | — | **both components read ZERO** |

⇒ A handler carrying ~1.6 ms of user time loses all of it. ⭐ **My 3–8% and that
94% were never in conflict about the same thing:** on a sandbox at 264 rows the
tick instrument recorded **622 and 824 consecutive dying handler threads each
reading ZERO ticks** while genuinely burning ~1.4 ms apiece. *An instrument that
reports nothing for 622 consecutive events is not measuring the events.*

⚠ **What survives, and what does not.** The **magnitude** stands and the
**composition** does not: truncation is a floor, so the live figure is a *lower
bound* of ≥21.2 ms per handler, and an independent process-level subtraction
gives 20–44 ms. Two methods agree on size. ⇒ **A sandbox handler is ~1.4 ms and a
live-daemon handler is 20–44 ms — a real 15–30× gap — but it can no longer be
called kernel time. What that gap IS, is open**, and this instrument's live
record is what settles it.

⛔ **And my own proposed explanation for the gap was WRONG, which is worth more
than the guess was.** I offered `comm` truncation — `TASK_COMM_LEN` is 16, so
`yggterm-daemon-client-…` truncates and unnamed threads inherit the parent's
`comm`, letting a by-name classifier merge reader threads into the handler
bucket. It does not apply: that bucket is defined by **lifetime**, not by name —
every thread that *dies* in-window — and reader threads do not die, so they
cannot be in a dying-thread bucket. ⇒ **The observation that actually killed the
figure was the coarse-field one, not the mechanism I attached to it.** A correct
suspicion with a wrong mechanism is still a wrong claim; it was right to send it
as a candidate rather than file it as a cause.

#### ⛔ THE FAN-OUT HYPOTHESIS FOR THE 15–30× GAP — MINE, TESTED, REFUTED

`ServerRequest::Snapshot` calls `refresh_proxied_working_flags`
(`daemon.rs:8024` → `:4308`), which makes **one outbound round trip per
preserved owner, inside the connection-handler thread**. A live daemon carries up
to 29 preserved owners, and 29 × ~1.4 ms lands squarely on the 20–44 ms live
figure. The arithmetic fitted, the code path is real, and it explained why a lone
sandbox daemon could not reproduce the gap.

**The control says no.** Driven on a sandbox with **zero peers and zero preserved
owners**, so the fan-out cannot fire at all:

| verb | n | CPU µs/conn | max | kernel share |
|---|---|---|---|---|
| `snapshot` | 1,715 | **9,633** | 33,225 | 11.3% |
| `status` | 5,147 | 1,533 | 3,577 | 8.0% |
| `ping` | 5,146 | 118 | 428 | 3.6% |

⇒ **`snapshot` costs 6.3× a `status` and 82× a `ping` with nothing to fan out
to.** The verb is intrinsically expensive; the fan-out is not needed to explain
it. ⭐ **A mean over a live request mix that contains snapshots reaches 20–44 ms
without needing any fan-out story — or any kernel-time story.**

⛔ **That is the second mechanism I have proposed for this gap and had refuted,
and both were sent as candidates rather than filed as causes.** The first
(`comm` truncation) died to a bucket defined by lifetime; this one died to its
own control. **The cheap part was proposing them; the part that cost anything
would have been believing them.**

### ⭐ 6m. `snapshot` IS A PER-ROW VERB, AND IT COSTS 6.5x WHAT `status` DOES PER ROW

*Two arms, **sessions held at zero in both**, so ROWS is the only variable.
Warm windows, per-verb thread CPU, nothing sampled.*

| verb | rows=0 | rows=264 | per-row |
|---|---|---|---|
| `ping` | 61 µs | 116 µs | — (no row work) |
| `status` | 101 µs | 1,421 µs | **5.0 µs/row** |
| `snapshot` | 123 µs | **8,720 µs** | **32.6 µs/row** |

⭐ **The status column is an internal consistency check and it passes:** 5.0 µs/row
against the 4.645 measured independently in §6h and 4.71 from the field re-fit.
The same run therefore prices `snapshot` on a scale already validated.

⇒ **`snapshot` is a per-ROW verb.** Both arms carry **no sessions**, so a
per-session explanation cannot account for any of this. ⛔ It was proposed that
*"snapshot is cheap without sessions and expensive with them, so the aggregate's
p50 is a session effect wearing a verb's name"* — **rows alone are sufficient**,
measured with sessions held at zero. A session effect may exist on top; it is not
needed.

⇒ **And per row it is a far larger term than §S5's.** `status` at 4.6 µs/row was
declined at ~1.5% of daemon CPU; `snapshot` costs **6.5x that per row**, and ROWS
is frozen at each daemon's birth — so a 264-row daemon owning one session pays
**8.7 ms per snapshot for rows it does not own.**

### ⛔ AND THE RATE PRICES IT OUT — SAME VERDICT AS §S5, FOR THE SAME REASON

**The multiplier was asked for before any cores figure was quoted, and it kills
the optimisation.** Live, fleet-wide: **`snapshot` runs at 0.37/s.**

    0.37/s x 8,720 us = 0.0032 cores  ->  ~0.1% of daemon CPU

⇒ **Do not build a fix for it.** Even allowing a live snapshot to cost 10x the
sandbox figure it is ~1%. ⭐ **The finding survives as a fact about the verb and
dies as an optimisation** — which is exactly why the rate was asked for first.

⭐ **That rate is quotable where the others are not, and the asymmetry is
load-bearing.** `perf_span_is_high_frequency_noise` lists
`("daemon_request", "terminal_snapshot")` — **not** `("daemon_request",
"snapshot")`. So snapshot counts are **complete and unsampled**, while `status`,
`ping`, `working_flags` and `terminal_read` are kept only at ≥8 ms or 1-in-50 and
their counts are **undercounts**. ⇒ **A status:snapshot ratio read off that
aggregate is not real**, because only one side is fully recorded.

⚠ **One gap not hidden:** live snapshots show **320.9 ms p50 WALL** against
8.7 ms of sandbox CPU — 37x. Wall ≥ CPU and live snapshots carry sessions, so
live snapshot CPU is probably higher than the sandbox figure. At 100 ms of CPU
the term would be 0.037 cores, ~1%. **Still not worth a version-gated change**,
and the live CPU figure cannot currently be measured from outside the process
(below).

⛔ **THE PRIOR ORDERING WAS OBTAINED BY A KNOWN DEFECT, ON BOTH SIDES OF THE
DISAGREEMENT.** The "snapshot is cheaper than status" figure (3.67 vs 5.07 ms)
came from dividing process CPU by a request count with **no zero-request
baseline** — and worse than a constant offset, the two verbs ran at different
wall rates, so the windows had different durations and a constant background
loads the longer one more, which can invert an ordering on its own. ⚠ A
re-measurement with the baseline subtracted found the background was 0.00000
cores, so that defect did not bite — **the ordering survived by luck, not by
method.** ⇒ Neither figure should have been quoted from the weaker instrument.

### ⛔⛔ 6o. AN EXTERNAL ESTIMATOR CANNOT PRICE A LIVE DAEMON'S REQUESTS — THE SIGNAL IS BELOW THE BASELINE'S DRIFT

A baseline-subtracted estimator pointed at live daemons, with baselines
**bracketing** the load so drift could not pass as request cost, **refused**: the
brackets moved 0.4240 → 1.1437 cores across one arm and the net came out
**negative, −254 ms/request**. Impossible ⇒ the arm is void and nothing is quoted
from it, including the rungs that looked plausible.

⭐ **The reason is structural, and it retro-explains three separate failed
arms.** A baseline-subtracted estimator needs a **stationary** baseline. The
signal (100 requests × ~2 ms = 0.2 core-seconds) is ~35x smaller than the
baseline's observed drift over the same 10 s (~7 core-seconds). ⇒ **The quantity
sits below the noise floor of any external instrument.** Not three unlucky arms:
one arm attempted three ways against a baseline moving faster than the signal.

⛔⛔ **AND THE FIRST VERSION OF THIS PARAGRAPH BLAMED THE WRONG BASELINE — MINE
TO CORRECT, BECAUSE I WROTE IT.** It said the baseline *is* "the bursty
per-session reader term, swinging 25x between adjacent 60 s windows". **That 25x
is WITHDRAWN.** It was measured immediately after the measurer's own load
generators had been driving those daemons; re-measured with nothing of the
measurer's running, the daemons that "burst" are flat — **1.03x, 1.05x, 1.12x**
against the 3.0x/2.8x/8.9x published, and six consecutive 30 s samples of the
population total span only **1.22x**.

⇒ **The void above is real and directly observed; its EXPLANATION was not.** The
drift that swamped the signal was substantially **the observer's own load**, not
an intrinsic property of the daemons. ⭐ **That is the same error as §6n's
~1.7 ms, from the other direction:** a process-level counter charging concurrent
work to whatever the measurer happened to be doing. There it charged reader
threads to open connections; here it charged the measurer's own generators to the
daemons' chores. ⇒ **The version where you contaminate yourself is the one nobody
checks for**, and it is a clause of its own in the field guide.

⚠ **One burst survives quiet conditions, and it is a single daemon:** the
2.12.14 outlier, **4.15x** — the same daemon §5 independently identifies as
running a fixed-but-still-shipped full-corpus-read defect. ⇒ **Burstiness is a
property of ONE KNOWN-BAD DAEMON, not of chores.** Another argument for the
drain, not for a chore fix.

⇒ **§6l's in-process span is therefore not merely the better instrument, it is
the ONLY one that can price a live daemon's requests**, because it measures the
REQUEST rather than the PROCESS and is immune to what the rest of the daemon is
doing. ⚠ The remaining live figure (20–44 ms per dying thread) is **PROVISIONAL**
until that record lands.

### ⛔ 6n. THE THREAD'S PRE-CLOSURE COST IS ~50 µs, MEASURED — SO THE ~1.7 ms GAP IS NOT THE CHILD

Recording `CLOCK_THREAD_CPUTIME_ID`'s **absolute** value at closure end, beside
the closure delta, makes the thread's pre-closure life a measured field:

    pre_closure = 39-55 us, flat across all three verbs and both row counts

⇒ **The standing note that thread spawn is ~50 µs is vindicated on the child
side**, and it was measured rather than assumed.

#### ✅ AND THE PARENT SIDE IS NOW MEASURED TOO — THE PATH IS CLOSED

The accept loop's own cost per connection, from the same window as the handlers,
so the two halves cannot be paired across different moments:

    accept + spawn = 44-48 us/conn, flat across row counts, max 280-662 us
    n matches the handler count EXACTLY (8,207/8,207 - 13,126/13,126)

⭐ That exact match is the record's own completeness check: every connection is
accounted for on both sides.

**The whole per-connection floor, measured in three pieces:**

| piece | cost |
|---|---|
| parent accept + spawn | **44–48 µs** |
| child pre-closure (thread entry) | **39–55 µs** |
| child closure, `ping` — a request that does nothing | **61–126 µs** |
| **total** | **≈150–230 µs** |

⛔⛔ **So the ~1.7 ms "spent outside the closure" is NOT in the request path at
all.** It came from subtracting a closure span from a process counter, and the
process counter was charging concurrent background work — reader threads, chores
— to connections that merely happened to be open at the time. ⇒ **Same lesson as
§6o from the other direction: a process-level counter cannot isolate a
per-connection quantity while anything else in the daemon is running.** ⚠ **A
difference between two instruments is not a measurement**, and here both halves
of the difference are now measured directly and they do not add up to it.

⇒ **§S4 should be scheduled on stability and observability grounds, not on
cores.** The per-connection overhead it would remove is ~150–230 µs at ~4
connections/s — under **0.001 cores per daemon** — and the note that said so was
right for a better reason than it gave.

**Two harness defects fixed in the same pass, both of which report a fired
instrument as one that did not fire:**
- ⛔ A single failed request used to `break` the arm; the only symptom was
  *"no window flushed"*. It now counts failures and says so.
- ⛔ The reader ignored **rotated** trace siblings. At a high drive rate the file
  rotates mid-arm and carries the flushed windows out with it — same symptom,
  different cause, and it cost an arm.

⭐ **And the zero-request BASELINE is now taken in the same run, warm**
(`0.00000 cores` at both 0 and 264 rows). A daemon burns CPU whether or not
anyone asks it anything, so dividing a process-wide delta by your own request
count charges that background to requests you caused — an error worth 29× on a
neighbouring arm. ⚠ Warm matters: taken immediately after start it read
**0.029 cores**, which is state restore, not background.

⛔ **A field too coarse for its quantity, walked into while documenting the
trap.** The user/kernel split first shipped on `/proc/thread-self/stat` fields
14/15, justified as *"summed across a window, hundreds of handlers make it
meaningful"*. Those are `CLK_TCK` units — 10 ms — and **every** handler is
shorter than one tick, so each sample truncated to zero and the sum was zero:
`ticks=0, kernel_share=n/a` across ~26,500 handlers carrying ~2.5 s of CPU.
⇒ **Truncation is applied per sample BEFORE the sum: a sum of floors is not the
floor of a sum.** `getrusage(RUSAGE_THREAD)` answers the same question per
thread at microsecond resolution.

### 6f. Honest limits on this section

1. ⛔ **The exact periodic call site is NOT located.** The fan-out helper is
   `reachable_versioned_daemon_statuses[_excluding_endpoint]`
   (`daemon.rs:13144`/`13166`) — one call issues one `status()` per versioned
   socket — and it has **18 call sites**. §6c rules out request-driven fan-out,
   so the driver is one of the periodic paths, but which one is unestablished.
   Nothing above depends on knowing it; the fix in §S5 does.
2. ⚠ **Core figures come from the one VALID window** (8 x 20 s, spinner 1.000 /
   sleeper 0.000). **Four later attempts were VOID** — spinner 0.75–0.87 under a
   load average of 83.8 on 32 CPUs — and none of their numbers are quoted here.
   ⭐ **The structural claims do not depend on that**: §6a–§6d are trace
   *counts*, which host contention cannot bias.
3. ⚠ **Cost and rate are not from the identical window.** Status rate was stable
   at 3.4–4.7/s in every run across ~40 minutes, so the combination is
   defensible — but it is a combination, and `ms/status` inherits it.
4. ⚠ **The joint model did not replicate cross-sectionally.** Re-fit on the same
   host six hours later: `cores ~ OWNED+ROWS` fell from **R²=0.939 to 0.109**,
   and the AGE slope flipped sign. The population had lost its highest-leverage
   point (a 23-session daemon; tonight's maximum was 9), and per-daemon CV is
   ~18% with a **common mode** — all daemons swing together window to window.
   ⇒ **The like-for-like comparison survives and the cross-sectional fit does
   not.** Among 1-owned legacy daemons: rows 86–101 mean 0.151 cores, rows
   244/246 mean 0.207 ⇒ **0.00036 cores/row**, reproducing 0.000337. Quote the
   per-row coefficient from the paired comparison, ⛔ **never the R².**

## 6h. THE POPULATION IS NOT A STANDING SET — IT CHURNS, AND A CENSUS CANNOT SEE IT

A census samples daemons that exist at the moment it runs. A daemon spawned,
handed off and retired between two census reads is invisible to it — the same
blindness as §3's CPU hiding in exited *threads*, one level up, in exited
**processes**. Sampling the `server daemon` process set at 4 Hz (subcommand, not
`comm`), two runs, both controls in each:

| window | census start → end | BORN | DIED | standing cores | transient cores | churn share |
|---|---|---|---|---|---|---|
| 180 s | 19 → 19 | 3 (60/h) | 3 (60/h) | — | **0.189** | — |
| 240 s | 19 → 19 | 3 (45/h) | 3 (45/h) | **2.798** | **0.170** | **5.7%** |

⇒ **One daemon is born and one retires roughly every minute, while the census
reads a perfectly stable 19.** Independently root-caused by the hot-restart lane
as a non-terminating self-retire loop: a per-process fact stored in a host-shared
file makes a predecessor hand off, see the successor clear the entry, and hand
off again — **11 successors from one process in 53 minutes**, on any host with a
replaced binary. Fixed there; this section is the cost side.

⇒ **Confirmed as a real phenomenon and priced at ~5.7% of daemon CPU** — a young
daemon burns 0.02–0.30 cores while alive, comparable to a standing one.
⛔ **But it is NOT the missing majority.** Standing daemons still account for
~94%, and their cost is not request serving (§6g: under 1% of the floor).
⇒ **After §6g and §6h, roughly 93% of daemon CPU remains unattributed.** That is
the open question, and it is now the only one worth the next window.
⚠ Churn *would* fit three of the model's unexplained properties at once — kernel-
heavy, not growing with age, invisible to a per-request instrument — which is
exactly why it needed pricing rather than adoption. It fits the shape and does
not fill the magnitude.

## 6i. THE STANDING COST IS IN PERIODIC CHORE THREADS, NOT IN SERVING ANYTHING

The live-population half of the arm split with the optimisation lane. Per-thread
`/proc` sampling, both controls in each quoted run.

**First, a correction to my own reading one step earlier.** A by-name pass put
49% of daemon CPU in a bucket labelled `yggterm-headles` and I read that as *the
main thread*. It is not. **Rust's `thread::spawn` without `.name()` leaves the
child with the parent's `comm`**, so that bucket is *every unnamed background
chore*. Measured directly, the real main thread (`tid == pid`) is:

| daemon | main-thread cores | user | kernel | top syscall |
|---|---|---|---|---|
| four census daemons | **0.0007–0.0012** | ~0.0002 | ~0.0005 | **`poll` 99–100%** |

⇒ **The accept loop costs nothing and is asleep in `poll`.** ⛔ A truncated
`comm` is not a thread identity — `TASK_COMM_LEN` is 16, so
`yggterm-daemon-client-…` and the process name both truncate to something that
looks like a category and is not one.

**Where it actually is.** 60 s at 30 Hz, controls spinner 1.000 / sleeper 0.000:

| cores | pid | comm | syscall profile |
|---|---|---|---|
| **0.1894** | 2.12.14 outlier | unnamed spawned | `clock_nanosleep` 81%, **on-CPU 19%** |
| 0.0526 | 3.0.52 | unnamed spawned | `clock_nanosleep` 93%, on-CPU 6% |
| 0.0436 / 0.0411 | 2.12.14 outlier | unnamed spawned | `clock_nanosleep` 93%, on-CPU 5% |
| 0.0400 | 3.0.39 | unnamed spawned | `clock_nanosleep` 94%, on-CPU 5% |
| 0.0376 | 3.0.62 | unnamed spawned | `clock_nanosleep` 94%, on-CPU 5% |
| 0.0355 | 2.12.14 outlier | `yggterm-perf-in` | `clock_nanosleep` 96%, on-CPU 4% |
| — | — | **13 threads > 0.0005** | **0.4475 cores over 4 daemons** |

⭐ **The histogram carries its own consistency check.** `/proc/<tid>/syscall`
reads as unparseable exactly when the thread is **on CPU rather than in a
syscall**, so that bucket is a duty cycle: the 0.1894-core thread reads **19%
on-CPU**, and 0.19 cores *is* 19% of a core. The two agree without being fitted
to each other, on every row.

⇒ **These are periodic chores that wake, burn USERSPACE CPU, and sleep** — not
request handlers, not the accept loop, not anything triggered from outside.
⭐ **The hottest single thread in the fleet (0.1894 cores) is on the 2.12.14
outlier**, which is where §5 already placed a fixed-but-still-running
full-corpus-read defect. Same daemon, arrived at by a different instrument.

⛔ **WHAT IS NOT YET ESTABLISHED, and I am not quoting a number for it.** The
split between *chore threads* and *connection handlers* needs a run whose
controls bracket, and four attempts at 15–50 Hz read spinner **0.805–0.819 —
VOID**. The host was carrying an unrelated OCR workload (six processes at
160–230% CPU, load average 65 on 32 cores), which is also a reminder that **"the
machine is hot" is not by itself a statement about this program.**

⇒ **Standing ledger: request serving <1% · daemon churn 5.7% · periodic chore
threads now the leading candidate for the bulk, un-split and unquantified.**

## 6j. THE SPLIT, DONE — AND THE EXPENSIVE HALF IS OLD CODE, NOT THE ARCHITECTURE

§6i left one question: of daemon CPU, how much is *periodic chore threads* and
how much is *connection handlers*? Both prior sections claimed the bulk — §3 put
it in threads that die between samples, §6i in long-lived chores — and they
cannot both own the same cores.

### 6j-1. The instrument, and the control the four VOID attempts lacked

Two snapshots suffice, with no high-frequency sampling to be perturbed by host
load. `/proc/<pid>/stat` is **group-wide and counts exited threads**;
`/proc/<pid>/task/*/stat` cannot. So over one window:

    P = process delta                                     (includes dead threads)
    A = sum over tids present at BOTH ends of their delta  -> long-lived (chores)
    B = sum over tids present only at the end, full cpu    -> born and survived
    R = P - A - B                                          -> DIED in-window

⛔ **R is obtained by SUBTRACTION, which is exactly the shape that has cost this
seat four claims.** So R gets a **positive control**: a helper that burns a known
quantity of CPU in short-lived threads and exits them. If the harness cannot
recover that known number, the run is void.

| control | required | first build | after fix |
|---|---|---|---|
| spinner | ~1.000 | 0.9991 | **0.9994** |
| sleeper | ~0.000 | 0.0000 | **0.0000** |
| **residual (known dead-thread CPU)** | ~100% | 80.4% | **100.2%** |

⭐ **The 80.4% was not an 80%-accurate instrument** — it was the control starting
its burn 1.0 s before the window opened, so 1.0 of 5.0 core-seconds fell outside.
Delaying the burn until after t0 recovers 100.0%. **A control that reads slightly
wrong is worth debugging rather than tolerating; the "20% loss" was entirely an
artefact of when the control ran, and would have been carried into every number
below as a silent 1.25x correction factor nobody could see.**

### 6j-2. The split — ⛔ REPORT CORES, NOT SHARES, BECAUSE ONE TERM IS BURSTY

Two 60 s runs over all 20 daemons, all three controls held in both
(spinner 0.9994/0.9991, residual 100.2%/100.2%):

| term | run 1 | run 2 |
|---|---|---|
| long-lived threads (chores + main) | 1.357 | **3.800** |
| born and survived | 0.000 | 0.000 |
| died in-window (connection handlers) | **1.793** | **1.944** |
| total daemon population | 3.150 | 5.744 |

⛔⛔ **I FIRST WROTE THIS AS "43% chores / 57% handlers". THAT SHARE IS NOT A
PROPERTY OF THE SYSTEM.** The replication moved it to 66%/34% — not because the
handlers changed, but because **the chore term nearly tripled while the handler
term moved 8%.** Per daemon:

| term | reproducibility across the two runs |
|---|---|
| **handlers (DIED)** | **stable — every daemon within ~5–10%** (0.1774→0.1896, 0.1971→0.2151, 0.0965→0.1005) |
| **chores (LONGLIVED)** | **bursty — 0.0368→0.9215 (25x), 0.0512→0.7046 (14x), 0.0228→0.2021 (9x)** on individual daemons, flat on others |

⇒ **A share is a ratio of two measurements, and this one divides a stable
quantity by an episodic one.** The honest statement is in cores: **the handler
term is ~1.8–1.9 cores and reproduces; the chore term is episodic and sampled
anywhere between 1.4 and 3.8 cores by a 60 s window.** ⭐ **§6i's 0.4475 cores
over 4 daemons was one such 60 s sample of a bursty process, and should be read
as a sample, not a level.** Pricing the chore term needs a long window or many.

⭐ **The four unpolled daemons read 0.0002–0.0007 cores, and two produced a
slightly NEGATIVE residual (−0.0003).** Not an error to hide: it bounds the
harness's own noise floor at ~0.0003 cores, **three orders below the live
daemons' 0.08–0.20**, which is what licenses reading R as signal at all.

⛔ **Neither §3 nor §6i was wrong; each had measured a real half and generalised.**

### 6j-3. What a handler thread spends, measured directly rather than divided

Per dying handler thread (100 Hz, 40 s, spinner 0.9997):

| | per thread |
|---|---|
| user | **1.6 ms** |
| kernel | **23.4 ms** |
| total | **25.0 ms — 93.8% KERNEL** |

and 75.8% of handler samples read *on-CPU*, not blocked — so this is kernel work
being **done**, not waited on. ⭐ The syscall census separates the long-lived
threads cleanly at the same time: exactly one thread each parked in `read`,
`accept4` and `futex`, and ~6 in `clock_nanosleep` — the chores of §6i — leaving
`recvfrom` and on-CPU as the handler population.

⛔ **The identity rests on the spawn-site census, NOT on `comm`.** `comm`
truncates at 16 bytes, so every `yggterm-daemon-*` thread wears one label; the
source has exactly one per-connection spawn site (`yggterm-daemon-client-{ms}`),
the rest being one-off or test-only. **§6i's rule held: a truncated comm is not
an identity, and this time it was checked before being relied on.**

### 6j-4. ⛔ TWO CANDIDATES THAT FIT EVERY CLUE, PRICED, AND BOTH REFUTED

`handle_unix_stream` ends with an **unconditional `malloc_trim(0)`**
(`daemon.rs:19103`, and again on the TCP path at 19177), *after* `write_response`
and therefore outside the traced span. It fit every qualitative clue at once:
per-request, unconditional, kernel-heavy, invisible to `PerfGuard`.

**It was priced and it is not the cost.**

| | measured |
|---|---|
| `malloc_trim(0)`, 30 threads / 360 MB heap | **0.020–0.039 ms** |
| needed to explain the handler | ~23 ms |

⇒ **~600x too small.** A first benchmark reported a flat `0.00 ms` and that was
**a resolution artefact, not a result** — `getrusage` ticks at ~1 ms, the trap
already on file as *a field too coarse for its quantity reports zero*. It was
also **single-threaded**, so glibc gave it one arena where a daemon has dozens;
both defects had to be fixed before the zero meant anything.

The follow-up — *trim is cheap to CALL but forces the next request to re-fault
every page it returned* — was priced too: **353–852 minor faults per request,
i.e. ~0.5–1.7 ms**, with syscalls adding ~0.05–1.8 ms. Also too small.

⭐ **Both were adopted-looking and both died to a price.** The rule this section
exists to defend: *a candidate that explains every qualitative clue has earned a
measurement, not a conclusion.*

### 6j-5. THE CAUSAL ARM — and ⛔ THE SUB-GROUP CLAIM IT DID NOT SUPPORT

Every previous attempt at "what does one request cost" divided somebody else's
CPU by an assumed request rate. Here **the load is mine**: baseline, then a
generator adding `status` calls, with the handler-thread count and the CPU
measured in the same window — so the slope owes nothing to an assumed rate.
Added load raised the death rate on every daemon (arm not void); spinners
0.9990–0.9996 in every phase.

**The first run, on four daemons, looked like a headline.** Row counts within
7% of each other, and a 5–6x cost split that lined up perfectly with version and
with resident heap:

| version | RSS | rows | ms/request (run 1) | **ms/request (run 2)** |
|---|---|---|---|---|
| 3.0.154 | 29–36 MB | 263 | 8.1 | **14.9** |
| 3.0.153 | 44 MB | 261 | 8.5 | **33.3** |
| 3.0.52 | 382 MB | 243 | — | **44.2** |
| 3.0.39 | 691 MB | 100 | — | **28.8** |
| 3.0.0 | 507 MB | 86 | — | **26.8** |
| 2.12.24 | 417 MB | 85 | — | **28.6** |
| 2.12.17 | 414 MB | 70 | — | **29.2** |
| 2.12.14 | 53 MB | 73 | — | **12.4** |

⛔⛔ **IT DID NOT REPLICATE. The same daemon read 8.5 ms and then 33.3 ms,
twenty minutes apart, on the same arm.** Widening from 4 daemons to 8 destroyed
the version story and the RSS story with it: 44 MB reads 33.3 while 691 MB reads
28.8. **"The expensive handler is old code still running" is WITHDRAWN before
it was ever quoted anywhere but here.**

⭐ **Why it failed, and it is not the design.** The arm is causal and the load is
genuinely mine, but the estimator is a **slope between two noisy points**: Δcores
is 0.02–0.08 against a per-daemon swing of ~18% with a common mode, and the
generator's `status` fans out to peers, so the dose each daemon receives is
observed (0.65–2.25/s) rather than set. ⇒ **A causal design does not confer
precision. Two noisy points are still two noisy points, and a sub-group claim
read off n=2 per group is a sub-group claim read off noise.** It was caught only
because n=2 felt thin enough to widen — the replication was the whole instrument.

⇒ **WHAT SURVIVES, and it is the number the specs need:** across 12 daemon-arms
the per-request cost is **~25–30 ms**, spread 8–48, with **no established
dependence on version, RSS, or row count**. That central value is independently
confirmed by the direct per-thread measurement of §6j-3 (**25.0 ms**, a different
instrument that never sees a request rate) and by the flatness of the DIED term
across daemons spanning 36–690 MB of heap and 70–263 rows.

⚠ **THE RETRACTED "38 ms" SITS INSIDE THAT SPREAD, AND ITS RETRACTION STILL
STANDS.** It came from dividing all daemon CPU by the thread rate, which assumes
the whole of daemon CPU is handler work — false, and it would have produced the
same figure whatever the truth was. ⇒ *A number can land in the right range and
still deserve its retraction, because a method that cannot be wrong about the
right things is not evidence.* Quote §6j-3's 25.0 ms, which was measured.

### 6j-6. ⭐⭐ THE EMPTY-DAEMON CONTROL: the request PATH is ~1.7 ms; the rest is CARGO

A daemon started on a private home from the **current** binary, owning nothing
and polled by nobody, is the one arm where the baseline is a true zero — so a
per-request cost needs no noisy subtraction, and rows can be varied on their own
while age, heap, version and session count stay fixed. That is the confound the
live population cannot escape and a cross-sectional fit over it already failed
once (§7 note 1).

⚠ **A first pass read 2.30 ms and that figure was COLD.** Measured minutes after
the daemon started, it carried first-call lazy init and trace-file creation. With
a discarded warm-up pass the steady state is **0.70 ms**. ⭐ *An empty-daemon
control is only a control once it is warm* — the same class of mistake as the
residual control that started its burn before the window opened (§6j-1), caught
twice in one session by the same habit of re-reading a control that looks fine.

**Rows, varied alone, 200 requests per point after a warm-up pass:**

| seeded rows | 0 | 80 | 260 | 1000 |
|---|---|---|---|---|
| **ms CPU / request** | **0.70** | **1.10** | **1.70** | **5.20** |

⇒ slope **4.5 µs/row**, intercept **0.70 ms**, monotonic and near-linear
(3.3–5.0 µs/row between adjacent points).

⭐⭐ **THREE INDEPENDENT METHODS NOW AGREE ON THE ROW TERM:** this causal arm at
**4.5 µs/row**, §6g's inverse-probability-weighted re-fit of sampled field data
at **4.71**, and the optimisation lane's own seeded arm at **4.645**. Nothing is
shared between those three but the subject. ⛔ **The 94 µs/row of §6e is wrong by
20x and must never be quoted again.**

⇒ **THE REQUEST PATH IS CHEAP.** At a live daemon's 263 rows the whole `status`
reply is **~1.7 ms**, of which the row inventory is ~1.2 ms — against **~25 ms
for a handler thread on a live daemon**. ⛔ **This is the strongest reason S5
stays decided against:** the row inventory a fix would target is ~1.2 ms of a
25 ms thread, and it is already the *best-measured* term in the model.

**And it is not the request MIX either.** `snapshot` shows p50 **259.8 ms** in
the daemons' own aggregate, which invites "the expensive handlers are the other
verbs". On the sandbox, `snapshot` costs **3.67 ms against `status`'s 5.07 ms at
1000 rows — it is CHEAPER.** So that 259.8 ms is not intrinsic to the verb.

⇒ **WHAT THE RESIDUAL TRACKS IS SESSIONS, NOT ROWS AND NOT THE VERB.** It is the
one dimension the sandbox never reproduced: `snapshot` gathers terminal state per
session, §4 found contention is **98.6% `terminal_read`** which is per-session,
and §3 found thread churn rising from 4.23/s at 1 session to 25.22/s at 23. ⛔
**Stated as the next arm, not as a result** — no session-seeded sandbox was built,
so nothing here prices it.

⛔ **THE SPECIFIC CALL IS NOT ESTABLISHED, AND I AM NOT NAMING ONE.** Ruled out
by measurement, not by argument: `malloc_trim` (§6j-4), induced page-faults
(§6j-4), the lock wait itself (`lock_daemon_runtime_for_request` does one
`try_lock` then a **blocking** `lock` — a block costs no CPU), and the trace
writer (handles are cached in a map, rotation runs off an in-memory counter, so
there is no stat or reopen per call). What remains unpriced is named in §7.

⚠ **`perf-summary --since-ms` DOES NOT WINDOW THE AGGREGATE.** Asking for the
last 40 minutes returned counts within 1% of the lifetime figures, so the
recent-activity reading it invites is not available from that verb. The one place
it moved was `daemon_request/status`, whose count rose by exactly my own injected
load — **the instrument's only visible response was to the observer.**

### 6j-7. ⭐⭐ THE BURSTY TERM IS ONE PTY READER THREAD PER SESSION — arm run, loop closed

§6j-6 named sessions as the next arm and declined to price them. Priced here, on
a private sandbox with rows held at **0** so sessions are the only variable, each
session flooding its pty (`yes`) to saturate the path §4 identified.

⛔⛔ **THE FIRST RUN WAS WRONG BY 14–29x AND THE ERROR WAS MINE AGAIN.** It divided
the daemon's whole CPU delta by my request count — but a flooding session burns
CPU **whether or not anyone sends a request**, so pty work was being charged to
requests. Subtracting a no-request baseline measured at each rung:

| sessions | quiet ms/req | noisy ms/req (uncontrolled) | **noisy ms/req (controlled)** | background |
|---|---|---|---|---|
| 0 | 0.67 | 0.67 | **0.67** | 0.000 cores |
| 2 | 0.78 | ~~13.47~~ | **0.96** | **2.035 cores** |
| 6 | 0.84 | ~~120.40~~ | **4.15** | **5.176 cores** |

⇒ **The dominant cost is not per-request at all.** ⭐ *The same habit that produced
four retracted claims produced a 120 ms/request figure; the only reason it did not
survive is that a background rate was measured instead of assumed.*

**And the split instrument says exactly where it sits.** Same sandbox, 4 flooding
sessions, 30 s, spinner 0.9996 / residual 100.2%:

| term | cores |
|---|---|
| **long-lived threads** | **3.364 (97.5%)** |
| died in-window (handlers) | 0.085 (2.5%) |
| total | 3.449 |

with the **top four threads at 0.838, 0.837, 0.836, 0.836 — exactly four, one per
session.**

⇒ ⭐⭐ **THE BURSTY LONG-LIVED TERM OF §6j-2 IS A PER-SESSION PTY READER THREAD,
and its cost tracks that session's OUTPUT VOLUME.** That closes the loop on three
separate observations at once: the 25x swings between adjacent 60 s windows
(**agent output is bursty**), §6i's 13 threads across 4 daemons alternating
`clock_nanosleep` with on-CPU bursts (**a reader polling between reads**), and
§4's contention being **98.6% `terminal_read`**.

⚠ **DO NOT QUOTE 0.84 CORES AS THE COST OF A SESSION.** `yes` saturates a pty at a
rate no agent CLI approaches; this is the **ceiling**, and the live population's
per-thread figures (0.02–0.19 cores) are what ordinary output costs. The
transferable claim is the **shape** — one thread per session, cost proportional
to output volume — not the magnitude.

⛔⛔ **THE LEDGER THIS PARAGRAPH USED TO CARRY IS WITHDRAWN.** It read: *"nothing
here is a leak; the population is expensive because it is BUSY, and per-session
reader threads are the only large term standing."* Both halves are gone.

**What replaced it, from a unit of account no lane had used** — each daemon
measured against its **whole process subtree**, reproduced over two quiet runs.
Per daemon, legacy (n=14) and current (n=2) are **indistinguishable at 0.19–0.24
against 0.20–0.23 cores.** The legacy population costs 2.6–3.3 cores because
**there are fourteen of them**, and their subtrees are **idle** while the current
pair's carry 2.3 cores of real work.

    daemon population cost  ~=  N_reachable x floor(~0.2 cores)
    the floor being THE PRICE OF BEING REACHABLE WHILE HOLDING A ROW INVENTORY

⚠ **Honest limit:** n=2 on the current side, and its per-daemon range overlaps
legacy's entirely. *"Indistinguishable"* means **no detectable difference**, not
"identical" — and the population conclusion rests on the **count** (14 vs 2), not
on that comparison.

⭐⭐ **AND IT EXPLAINS WHY EVERY ARM IN §6h–§6n CAME BACK EMPTY, WITHOUT
CONTRADICTING ONE OF THEM.** Request serving <1%, snapshot ~0.1%, a 150–230 µs
per-connection floor, no intrinsic per-daemon floor, an unpolled daemon at
0.00017 cores, unreachable daemons at 0.000, and a total failure to find any
per-request signal on a live daemon — **every one of those fits, because the
floor is not spent per request.** Those arms did not fail to find the cost; they
correctly established it is **not in the path they measured.**

⇒ ⭐ **And the reconciliation this lane can add: the floor is in the SUBTREE, not
in the daemon process.** Every arm here measured the daemon *process*, and the
sandbox daemons **spawn no children at all** — no sessions, no wrappers, no
remote clients — so they have no subtree to carry a floor. That is why
0.00017 cores was reproducible and honest and still missed a 0.2-core term: **the
quantity was never inside the process being sampled.**

⇒ **§S1's justification INVERTS, and it is still not a leak:** not *"reclaim
leaked cores"* but **"stop paying a reachability floor fourteen times over for
daemons whose sessions are idle."**

⭐ **The method note, and it is the cousin of "the dangerous copy is in a
different file type":** this was seen only by **changing the unit of account**.
Every lane measured the daemon; none measured what the daemon was *for*. **A cost
model that never looks at the work being served cannot distinguish "expensive
because busy" from "expensive because numerous."**

## 6k. WHY AN UNPOLLED DAEMON DOES NOT RETIRE — the answer owed to the build lane

The optimisation lane's sandbox arms all exited after ~75 s
(`idle_shutdown_ms=90000`), and since `mark_daemon_activity` fires on every
request it inferred that **peer polling is what keeps every daemon alive**. That
sat against four daemons here which nobody polls and which have survived for
weeks. The test agreed on was: *do those four report `OWNED + PRESV == 0`?*

**Read from source rather than inferred** (`daemon_should_idle_shutdown`,
`daemon.rs:11526`) — retirement needs **three** gates to pass, and peer polling
is only one of them:

1. `terminal_session_count == 0` — owned *and* preserved sessions, and the
   comment is explicit that hot-update PTYs count here;
2. `idle_for_ms >= idle_shutdown_ms` — this is the one `mark_daemon_activity`
   feeds, default **90 s**;
3. `active_client_instance_records(home, endpoint).is_empty()` — **or** the
   daemon is superseded by a strictly-newer one.

⇒ **ANSWER: NO — three of the four own live interactive shells as children, so
gate 1 alone explains them, and every daemon in the census owns at least one
session too.** Peer polling is not what keeps the population alive; **owning a
session is.** The build lane's mechanism is not refuted, it is *not reached* —
its arms owned nothing, so for them gate 1 was open and gate 2 decided.

⭐ **The two arms never competed.** A seeded-but-unpolled daemon prices the cost
of *existing*; the live population prices the cost of *being used*. Both can be
true, and both are.

⚠ **One genuine anomaly, logged not explained.** The fourth daemon reports
`stored_sessions=0, live_sessions=0` in its own state file, holds **only LISTEN
sockets with zero established connections**, and its home's single
client-instance record is filed under a *different* endpoint version than the one
it serves — so all three gates appear open, and it has nonetheless been alive for
48 minutes against a 90 s window. Either the check is not reached on that path or
one of the three inputs is not what its state file says. ⇒ **queue item, not a
conclusion.**

Ordered by cores returned per unit of risk. Each carries the number it should
move and the falsifier that would show it did nothing.

### ⛔ S5 — DECIDED AGAINST (was: take the row inventory off `status`)

**Do not build this.** §6g measures all of status serving at **≈0.057 cores of
3.47 — 1.6%** of the population, so the census split buys ~1.5% while paying with
a version-gated protocol change whose `#[serde(default)]` failure mode is a
documented row-loss hazard at handover. **The cost is not where this spec aimed.**
⚠ **Reversal condition:** if a future measurement puts status serving above ~10%
of daemon cost — a much larger population, or a row count far beyond today's
~260 — re-open it; the O(R log R) per reply is still real, it is just cheap.
⭐ **What it got right and what to keep:** the row inventory *is* rebuilt on every
reply, and that was worth knowing. The error was pricing it from a ratio.

### S5 — the original spec, kept for the trail (⛔ superseded above)

**Why it outranks S1.** S1 is worth more but belongs to another lane, needs a
drain, and cannot be retrofitted to the daemons that carry the cost. S5 is a
request-handler change in 6.7's own lane, touches no daemon lifecycle, and
**helps every future daemon including the ones S1 can never reach.**

**Change:** split `ServerRuntimeStatus` in two. `status` keeps the scalars a
peer poll actually reads — version, build id, pid, counts, block reason. The
**row inventory moves to a separate request** (`census`) that only the rare
consumers call: reconcile, handover, adoption, retire-coverage. Concretely, drop
`stored_terminal_sessions`, `live_terminal_sessions` and the four `*_keys`
vectors from the poll path, and stop rebuilding `persisted_state()` and running
two sort/dedup passes per reply.

⛔⛔ **EXPECTED EFFECT IS DISPUTED BY §6g — MEASURE BEFORE YOU BUILD.** The
figure below divides the model's per-row coefficient by the poll rate. A direct
read of the handler's own `PerfGuard` duration gives **11 µs/row, not 94**, which
would make this spec worth **≈0.08 cores, not ≈0.66** — an order of magnitude.
§6g explains why that measurement is flagged rather than adopted (the instrument
records ~3.6% of requests), and names the check that settles it. ⇒ **Run that
check first.** The spec is still worth doing — the row term is directly confirmed
and per-reply work does drop from O(R log R) to O(1) — but **do not promise 0.66
cores for it**, and do not let it displace higher-value work on that number.

**Expected effect, as originally derived (⚠ see above):** removes the row-scaled
term — 0.000337 cores/row x **1,953 rows summed over the census** ⇒ **≈0.66
cores**, about 25% of the measured 2.64-core daemon footprint, scaling with every
row the fleet accumulates because ROWS is frozen at each daemon's birth.

**Falsifier:** re-run the §6f paired comparison after the change — 1-owned
legacy-shaped daemons, low rows vs high rows, both controls in the same run. If
the per-row coefficient does not fall to ~0, `status` was not what the row term
was buying and §6e is wrong.

⚠ **Compatibility is the whole risk, and it is a real one.** A pre-split daemon
asked for `status` by a post-split peer must still receive what it expects, and
§5's lesson applies with force: **14 daemons that will never restart still speak
the old shape.** The fields are `#[serde(default)]`, so an old daemon reading a
slimmed reply gets empty vectors — and an empty `stored_terminal_session_keys`
reads as *"this peer holds no dormant rows"*, which is exactly the input that
made a handover drop rows before. ⛔ **Version-gate the slimming on the
requester's advertised version; do not let absence mean zero.**

### S1 — Cap the daemon population (highest value, lowest code risk)

**Change:** nothing in the hot path. Retire census daemons whose owned sessions
can be migrated, and make the retirement path converge without a quiet window
(the settle-window mechanism is already production-proven for exactly this).

⛔⛔ **A "REVISED BY §6, THE SAVING IS QUADRATIC" CLAIM STOOD HERE AND IS
WITHDRAWN.** It gave a table showing 15→8 daemons banking −73% and 15→4 banking
−94%, on the strength of `polls = N(N−1)/T`. **§6b now retracts that scaling** —
its N=1 control was confounded and a deliberate causal arm did not reproduce it.
⇒ **S1 stands on its ORIGINAL linear estimate (−2.60 cores) and on the two
independent arguments below.** ⚠ **Do not plan a drain around an early-payoff
curve**: nothing measured supports "most of the win arrives on the first few
retirements", and stopping a drain early on that belief would bank less than
expected.

**Expected effect:** 3.468 → ~0.86 cores on this host, **−2.60 cores (−75%)**.
**Falsifier:** consolidate and re-run §1. If total daemon cores do not fall by
≥2.0, the per-daemon floor is not per-daemon and §2 is wrong.
⭐ **Prefer a falsifier denominated in COUNTS**: drain to N and re-count
`status`/s in the trace alongside the cores. A trace count is immune to the host
contention that voided four of this session's CPU windows, and it is the
measurement that would settle §6b's open question at the same time.
⚠ **Owner boundary:** daemon lifecycle belongs to the hot-restart lane, not to
6.7. This spec is the *justification* for that work, priced. It is not a licence
to reap daemons holding other agents' live sessions.

⭐ **SECOND, INDEPENDENT AXIS — the same population is what has been killing
rows.** Established separately from this cost work, by measurement of the row
deaths themselves: two daemon releases each cut live rows against the 300 s
idle-deferral window (10 rows across 4 campaigns at 402 s; one seat cut
mid-`tool_use` at ~272 s). The standing hazard is exactly the population priced
above — **the daemons predating the settle window, holding 34 live sessions**.
Because the settle window ships in the *predecessor*, those cannot be
retrofitted; they can only be drained.

⇒ **Consolidation is justified twice over, on two arguments that do not depend
on each other:** 2.60 cores of continuous cost, and the row deaths. Two
independent cases for one action are worth more than either alone, so neither
should be argued alone.

⛔ **And the second axis sharpens the boundary rather than loosening it.**
Killing a daemon that holds live sessions is *precisely* the failure mode above.
The action is a **drain** — move sessions off, then let the emptied daemon
retire — never an eviction.

### S2 — Give `waited_ms` microsecond resolution, and stop appending per event

**Change:** (a) record `waited_us`, not `waited_ms`; (b) replace the two
per-event `append_trace_event` calls **and the `PerfGuard` beside them** with an
in-memory counter flushed once per interval, carrying count + percentiles.
**Expected effect:** combined write load **141 KB/s → <2 KB/s, 12.49 GB/day →
~150 MB/day**, and the contention becomes visible instead of rounding to zero.
⛔ **The win is CPU, IO and SSD wear — NOT disk.** Both streams rotate and hold
~20 MB; nothing is reclaimed by fixing this. ⚠ And it is **22x smaller on the
desktop host** (§4), so it is a fleet-host win, not an answer to the fan.
**Falsifier:** if combined growth does not fall by ≥90x, the 437-bytes-per-
contention attribution in §4 is wrong.
⛔ **Fix both files in one change.** They are one code path; treating them as two
problems fixes 8.4 GB/day and leaves 4.1 GB/day behind.
⭐ **Do this before S3** — S3 changes contention, and right now there is no
instrument that can measure whether it helped.

### S3 — Take `terminal_read` off the global runtime mutex

**Change:** `terminal_read` is 98.6% of contention and does not need the whole
`DaemonRuntime`. Give the PTY read path its own lock or a lock-free ring per
session, so a read does not serialize against `status`, `snapshot` and every
other request.
**Expected effect:** failed `try_lock()` **322.8/s → <10/s**; the marginal
per-session term (0.0104 cores) should fall, since it is paid on every read.
**Falsifier:** re-fit §2 after the change. If the per-session coefficient does
not move, `terminal_read` contention was not what that term was buying.

### S4 — Bound the connection handler threads

**Change:** replace thread-per-connection at `daemon.rs:785` with a bounded
worker pool.
⭐ **Its driver is now named (§6d):** the churn is not the daemon's own work, it
is one thread per peer `status` poll. So S1 and S5 both reduce it as a side
effect, and S4 should be scheduled *after* them rather than against them.
**Expected effect:** removes 4–25 thread creations/s per daemon. ⚠ **Expected
CPU win is the SMALLER half** — thread spawn is ~50 µs, so 4/s is ~0.0002 cores.
The 38 ms per handler is the *work*, not the spawn. ⇒ **This is a stability and
observability fix, not a CPU fix**; do not promise cores for it. Its real value
is that CPU stops hiding in exited threads, so §3's instrument gap closes.
⛔ **§6j-5 updates the number: quote ~25 ms, not 38 ms**, and note that a handler
on an EMPTY daemon costs 0.70 ms — so a pool would keep ~95% of the cost, which
travels with the thread's work rather than with its creation. The conclusion
("not a CPU fix") is unchanged and now rests on a measurement instead of an
inference.

### ⭐ S6 — MEASURE THE HANDLER IN CPU TIME, AND FOR ITS WHOLE LIFE (build this first)

**The problem is an instrument, not a regression.** `PerfGuard` records **wall
time** and its guard **drops when `handle_request` returns**. So for a handler
thread that burns 25.0 ms of CPU, the existing span covers **~2.4 ms** — under 10%
— and reports it in a unit that cannot tell 25 ms of work from 25 ms of waiting.
**That is why §6j-4 could refute candidates but not name the cost**, and why four
sections of this file have argued about a quantity nothing measures.

**Change**, in two parts, both small:

1. Add a **CPU-time reading** to `PerfGuard` beside the wall reading —
   `clock_gettime(CLOCK_THREAD_CPUTIME_ID)` at construction and at drop. Emit
   both. ⭐ **Priced on this host: 570 ns per call**, so two calls per span cost
   **~1.1 µs against a ~2.4 ms span — 0.05%**, and the guard already takes two
   wall readings.
2. Wrap the **whole handler closure** at `spawn_unix_client_handler`
   (`daemon.rs:785`), not just `handle_request`, so the span covers thread entry
   to thread exit and stops excluding `write_response` and everything after it.

**Expected effect: zero cores.** ⛔ **Do not promise any**, and that is the point
— it converts an unattributable ~1.8 cores into an attributed one. The prediction
it must satisfy: **the new CPU-time span should read ~25 ms on a loaded daemon
and ~0.7 ms on an empty one**, reproducing §6j-3 and §6j-6 from inside the
process.

**Falsifier, denominated in counts so it runs on a busy host:** if the widened
CPU-time span still sums to well under the process's own `utime+stime` delta over
the same window, the cost is **not** in the handler thread at all and §6j-2's
subtraction is measuring something else — in which case say so and stop, rather
than widening the span again.

### S7 — The chore term must be measured over a LONG window, and two chores are named

**Not a code change yet — a measurement contract, because the seat kept sampling
a bursty process with a 60 s window and reading the sample as a level.** Per-
daemon chore CPU moved **25x between two adjacent 60 s runs** (§6j-2) while the
handler term moved 8%.

⇒ **Any claim about chore cost must quote a window of at least 10 minutes, or a
distribution over many windows — never a single 60 s figure.** §6i's 0.4475 cores
is hereby re-labelled a *sample*.

**Where to look first,** from the daemons' own aggregate: `background_copy_chore`
(p50 44.5 ms but **p95 11.2 s**) and `background/local_tree_scan` (**p50 11.6 s**,
178 events). Both are heavily skewed, which is the signature of the burst.
⚠ **`perf-summary --since-ms` cannot window this** (§6j-6) — it returned lifetime
counts for a 40-minute request, so the long-window instrument has to come from
outside the daemon, or that flag has to start working.

## 8. What this file does NOT claim

- **Nothing here measures the GUI.** The GUI runs on another host; the
  12 "gui/client" processes in §1 are CLI clients, not the shell.
- **No `hpet` extrapolation.** This host is `tsc`. The kernel-time share in §1
  would be far larger on an `hpet` host, but by an unmeasured factor.
- **The 21-minute render storm is untouched by this.** That is a user-time,
  GUI-side, latching phenomenon; nothing in the daemon model explains it, and
  the numbers here should not be offered as if they do.
- ⛔ **`~/.yggterm` at 9.5 GB is NOT the trace.** An earlier version of this file
  put that figure beside the write rates, which invited the reading that §4
  fills the disk. It does not: both streams rotate and hold **~20 MB**. The
  9.5 GB is dominated by a **managed npm cache at 7.6 GB** with no retention
  rule — a separate defect, separately filed, not this one. ⇒ **§S2's win is
  write rate — CPU, IO and SSD wear — and essentially no reclaimed disk.**
  Quoting it as "12 GB/day of disk reclaimed" would be measured later and found
  false.
