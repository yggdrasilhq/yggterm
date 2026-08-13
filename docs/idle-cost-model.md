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
| ychrome | 12 | 0.238 | 0.117 | 0.122 | 0.64 GB |
| gui/client | 12 | 0.049 | 0.024 | 0.025 | 0.75 GB |
| net process | 20 | 0.002 | 0.002 | 0.000 | 1.53 GB |
| other | 36 | 0.022 | 0.015 | 0.007 | 1.40 GB |
| **total** | 108 | **4.167** | 1.380 | 2.787 | 12.48 GB |

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
2. **Daemon 219756 is excluded** (3 sessions, 0.648 cores, 0.396 of it user
   time — 12x the user time of its peers). Including it drops R² from 0.863 to
   0.240 on its own. It is a genuine outlier with a different shape and wants
   its own investigation, not averaging into a fleet model.

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
| `event-trace.jsonl` growth | **95.0 KB/s** |
| events parsed | 13,091 |
| `lock_wait_begin` / `lock_wait_end` | 6,456 / 6,456 |
| **failed `try_lock()` per second** | **322.8** |
| share of trace volume that is lock_wait | **98.8%** |
| projected | **8.3 GB/day, from this one file** |
| `~/.yggterm` on this host today | **9.5 GB** |

Contention by request kind: `terminal_read` 12,736 · `status` 164 · `snapshot` 12.
⇒ **98.6% of all lock contention in the daemon is `terminal_read`** — reading a
PTY serializes against every other operation through one mutex.

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

## 5. The specs this justifies

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

### S2 — Give `waited_ms` microsecond resolution, and stop appending per event

**Change:** (a) record `waited_us`, not `waited_ms`; (b) replace the two
per-event `append_trace_event` calls with an in-memory counter flushed once per
interval, carrying count + percentiles.
**Expected effect:** trace write load **95 KB/s → <1 KB/s, ~8.3 GB/day → ~90 MB/day**,
and the contention becomes visible instead of rounding to zero.
**Falsifier:** if `event-trace.jsonl` growth does not fall by ≥90x, lock_wait was
not 98.8% of the volume and the reduction in §4 is wrong.
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

## 6. What this file does NOT claim

- **Nothing here measures the GUI.** The GUI runs on another host; the
  12 "gui/client" processes in §1 are CLI clients, not the shell.
- **No `hpet` extrapolation.** This host is `tsc`. The kernel-time share in §1
  would be far larger on an `hpet` host, but by an unmeasured factor.
- **The 21-minute render storm is untouched by this.** That is a user-time,
  GUI-side, latching phenomenon; nothing in the daemon model explains it, and
  the numbers here should not be offered as if they do.
- **`~/.yggterm` at 9.5 GB is reported, not attributed.** §4 accounts for
  8.3 GB/day of *current* write rate; how much of the 9.5 GB standing total is
  lock_wait trace versus session state was not measured.
