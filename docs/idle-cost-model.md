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
   by the first session". ⇒ The experiment that settles it: park a census
   daemon at zero owned sessions and re-measure. Until then the floor term is
   real but its cause is not attributed.
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

## 6. The specs this justifies

Ordered by cores returned per unit of risk. Each carries the number it should
move and the falsifier that would show it did nothing.

### S1 — Cap the daemon population (2.60 cores, highest value, lowest code risk)

**Change:** nothing in the hot path. Retire census daemons whose owned sessions
can be migrated, and make the retirement path converge without a quiet window
(the settle-window mechanism is already production-proven for exactly this).
**Expected effect:** 3.468 → ~0.86 cores on this host, **−2.60 cores (−75%)**.
**Falsifier:** consolidate and re-run §1. If total daemon cores do not fall by
≥2.0, the per-daemon floor is not per-daemon and §2 is wrong.
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
**Expected effect:** removes 4–25 thread creations/s per daemon. ⚠ **Expected
CPU win is the SMALLER half** — thread spawn is ~50 µs, so 4/s is ~0.0002 cores.
The 38 ms per handler is the *work*, not the spawn. ⇒ **This is a stability and
observability fix, not a CPU fix**; do not promise cores for it. Its real value
is that CPU stops hiding in exited threads, so §3's instrument gap closes.

## 7. What this file does NOT claim

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
