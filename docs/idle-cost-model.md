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
   processes on this host own zero sessions and are absent from the census
   (they are build-tree binaries on unreachable socket paths). They measure
   **0.000 cores** — a clean natural zero. But no *census* daemon owns zero
   sessions, so the fit cannot separate "a per-daemon floor" from "a step paid
   by the first session".
   ✅ **ANSWERED IN §6e — and it is NEITHER of those.** The floor is the cost of
   answering peer `status` polls, paid at a rate set by the daemon *population*.
   The four zero-cost daemons are unreachable, so nobody polls them.
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
0.000337 cores/row ⇒ **≈0.66 cores spent on row-scaled work**. ⛔ **And each
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

*dev, 2026-08-14, on the one daemon that carries `670fa66d` (pid 2945182,
build `4e801c13`, confirmed by `git merge-base --is-ancestor`, never by version
number).*

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

### 6e. Both terms of the model, derived

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

⭐ **The SLOPE is the robust part.** A uniform under-recording factor cancels in a
between-daemon comparison, and the low-row and high-row bands separate cleanly.
⇒ **Treat 11 µs/row as the measurement and 94 µs/row as the refuted inference,
while treating the absolute per-daemon share as unconfirmed.**

**The check that settles it, before anyone builds §S5:** instrument the status
path directly — a counter incremented per reply with the row count and elapsed
CPU (not wall), read back over a known window — and confirm both the slope and
what fraction of daemon CPU the path accounts for.

⇒ **§2b's per-row term is real and now attributed, and the refuted candidate
stays refuted for a better reason than its period.** `run_background_copy_chore`
was never going to be it: the row term is not background work at all, it is
**per-request work multiplied by a request rate the daemon does not control**.
⛔ **The period hunt in §2b was the wrong quarry** — chasing a ~5–6 s read spike
of ~655 KB. Reads were already known not to proxy this cost (§2b's own caution:
a 0-session control read 4.5 MB while costing 0.001 cores). The quantity that
mattered was a **request rate**, and it was visible in the trace the whole time.

### ✅ 6h. MEASURED DIRECTLY: 4.645 µs/row, and status serving is 1.6% of the daemon population

*dev, 2026-08-14. A counter in `fn status()` itself — every reply, nothing
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

## 7. The specs this justifies

Ordered by cores returned per unit of risk. Each carries the number it should
move and the falsifier that would show it did nothing.

### S5 — Take the row inventory off `status` (≈0.66 cores, no lifecycle risk) ⭐ DO THIS FIRST

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
