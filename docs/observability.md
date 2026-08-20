# Observability — the probe map, the units, and the instruments that lie

**Status:** ACTIVE 2026-08-20 · **Owner:** yggterm-core `perf.rs` / `render_probe.rs` + the
`ytrace` provider · **Reads with:** `docs/telemetry.md`, `docs/spec-ytop-design.md`

This file answers exactly one question: **which probes exist, what each one measures, in what
unit, where it is emitted, and where the bytes land.** It is the map every profiling notebook in
`notebooks/` stands on.

It deliberately does **not** answer:

| Question | Owner |
|---|---|
| What is the durable terminal *incident* log, and its SQLite schema? | `docs/telemetry.md` |
| What should the `ytop` console look like? | `docs/spec-ytop-design.md` |
| What is OPEN right now? | `docs/pending-bugs.md` |
| Which instruments lie *in general*? | `docs/agent-field-guide.md` — the probe-specific ones are below |

---

## 1. The three streams, and which one a reader should open

A single yggterm process writes three different things that all look like telemetry, and choosing
the wrong one is the most common way a measurement comes out wrong.

| Stream | Path (under the yggterm home) | Shape | What it is for |
|---|---|---|---|
| **perf** | `perf-telemetry.jsonl` + `.g<ts>.jsonl` | `{ts_ms, pid, category, name, payload}` | span durations and rollups, the numeric stream |
| **trace** | `event-trace.jsonl` + `.g<ts>.jsonl` | `{ts_ms, pid, component, category, name, payload}` | lifecycle narrative — what happened, in order |
| **ytrace** | `ytrace.jsonl` + `.g<ts>.jsonl` | the `v:1` wire (§3) | the cross-app probe bus that `ytrace`/`ytop` query |

`ytrace` is **not a fourth copy**: `perf::append_perf_event` writes the perf record and mirrors the
same event onto the ytrace bus in one call, so the two cannot drift in content. They *can* differ in
**retention** — the perf stream and the ytrace stream rotate on their own budgets.

### ⛔ The home is resolved, not configured — and the losing candidate still holds old bytes

`ytrace::compat::resolve_home("yggterm")` picks the first of:

1. `$YTRACE_HOME/yggterm`
2. **the yggterm home (`$YGGTERM_HOME`, else `~/.yggterm`) — if it already exists**
3. `$XDG_DATA_HOME/ytrace/yggterm`
4. `~/.local/share/ytrace/yggterm`

On any host that has ever run yggterm, rule 2 wins, so the live bytes are in the **yggterm home**
and `~/.local/share/ytrace/yggterm` is a **stale orphan** left from before that directory existed.

⚠ Measured 2026-08-20: both fleet hosts carried an orphan there whose newest record was **one to two
days old**, alongside a live stream in the yggterm home. It is well-formed, it parses, it has the
right probe names, and it is silently out of date — a reader who globs `~/.local/share/ytrace/**`
gets a confident answer about yesterday. **Never hand-resolve this path.** Ask the CLI
(`ytrace query|tail|incidents|health`), which resolves it the one way the writer does.

---

## 2. Probe inventory

### 2.1 Declared

`perf::ytrace_provider()` (`crates/yggterm-core/src/perf.rs:14`) pre-registers 26 probes so that
sampling policy and clock are attached before the first emission:

| Group | Probes | Clock | Sampling |
|---|---|---|---|
| daemon hot paths | `daemon_request/{status,ping,terminal_read,terminal_write,terminal_snapshot,working_flags}` | wall | `floor 8 ms` + `1:50` |
| render cost | `render/{gui,web_content}` | **cpu** | always |
| render faults | `render/storm`, `ui/{render_fail_pattern,app_render_rate}` | wall | always |
| attach faults | `terminal_mount/retained_rehydrate_{skipped_live_connected,skipped_pre_resize,skipped_inactive,begin}` | wall | always |
| title lifecycle | `title/{untitled_session,resolve_attempt,llm_rescue,cli_store_hit,generation}` | wall | always |
| input latency | `input/{keystroke,pty,render}` | wall | always |
| per-CLI wiring | `cli/{agy_title,agy_resume,codex_geometry,codex_resume,persisted_identity}` | wall | always |

`resource_governor.rs:59` registers three more from the daemon: `row_resource/{hot,oom}` (cpu) and
`daemon/resource_governor` (wall).

### 2.2 Observed — and the gap is the point

⭐ **Registration is not emission.** A probe is registered at provider construction; whether it ever
fires depends on a code path being reached. Measured over a 36-minute live window on the GUI host:

| Probe group | Registered | Observed | Reading |
|---|---|---|---|
| `render/{gui,web_content,web_network,other}` | 2 of 4 | **all 4**, ~1/min | the probe emits one row per *role*; `web_network` and `other` are emitted but never registered, so they carry the default policy rather than the intended one |
| `input/pty` | ✅ | **60** | daemon saw writes |
| `input/keystroke`, `input/render` | ✅ | **0** | see §4.1 — not necessarily a defect |
| `title/*` | ✅ | **0** | the title work that *did* run emitted under `copy_generation/title`, a different name |
| `cli/*` | ✅ | **0** | no CLI-specific fault fired in the window |
| `render/storm` | ✅ | **0** | no storm in the window |
| `row_resource/*`, any incident | ✅ | **0** | **no incident has ever been recorded on either host** |

The busiest observed probes in that window, which is what a notebook actually has to work with:

| Probe | Count | Clock | p50 | p95 |
|---|---:|---|---:|---:|
| `request/{begin,end}` | 4893 / 4831 | wall | event | event |
| `terminal_io/dispatch` | 3708 | wall | event | event |
| `daemon_request/terminal_app_declares` | 3656 | wall | 0.20 ms | 0.36 ms |
| `app_declare/daemon_declare_absent` | 1827 | wall | event | event |
| `sidebar/merge_rows` | 1344 | wall | 9.70 ms | 21.36 ms |
| `background/copy_scan` | 87 | wall | 108 ms | 214 ms |
| `copy_generation/title` | 87 | wall | **8676 ms** | **9747 ms** |
| `daemon/background_copy_chore` | 58 | wall | 303 ms | 426 ms |
| `remote/resolve_yggterm_binary` | 183 | wall | 0.16 ms | **304 ms** |

---

## 3. Units — the part that is most often got wrong

### 3.1 `duration_ms` is on two different clocks and the field does not say which

`clock` says `wall` or `cpu`, and `perf::perf_span_time_base(category)` is its one owner, keyed on
the **category** so that a reclassification applies retroactively to bytes already on disk.

* `wall` — elapsed time between two `Instant`s. A latency.
* `cpu` — **CPU milliseconds consumed during a sampling interval.** Not a latency. The only category
  on this clock is `render`.

⛔ **A cpu-ms number is meaningless without its interval.** `render/gui duration_ms: 18300` is not
"the GUI took 18.3 seconds"; it is "the GUI burned 18.3 CPU-seconds during the interval this sample
covers". Divide by `interval_ms` to get the **core fraction**, which is the only comparable unit:

```
core_fraction = duration_ms / interval_ms
```

### 3.2 ⛔ Ranking by `total_ms` mixes the two clocks into one order

`ytrace query` and `ytrace health` rank probes by summed `total_ms` across **all** categories. A
`render` row (cpu-ms) and a `copy_generation` row (wall-ms) therefore sort against each other as if
they were the same quantity. They are not, and the top of that list is not "the hottest probe" —
it is "the probe with the largest number, whatever the number means".

⇒ **Read the `clock` field on every row before comparing two rows.** The notebooks in `notebooks/`
split the ranking by clock for this reason, and say so in their verdict cells.

### 3.3 Sampling multiplies the count

Probes registered `Sample::noisy()` keep every span at or above an **8 ms floor** plus a **1:50**
sample of the rest. A consumer that wants a *rate* from those probes must multiply the sub-floor
count by 50. The wire never records that sampling happened, so this is a property of the probe name,
not of the record — the table in §2.1 is where it is written down.

---

## 4. Instruments that lie, and how each one lies

### 4.1 `input/pty` without `input/keystroke` is not always a freeze

The input-latency chain spans **two processes**:

* `input/keystroke` — GUI, `shell/viewport.rs:7431`, fired from `TerminalJsEvent::Input`, i.e.
  xterm.js `onData`. **This is the human typing, and only the human.**
* `input/pty` — daemon, `daemon.rs:7166`.
* `input/render` — GUI, `shell/viewport.rs:10833`, bytes about to be painted.

A keystroke with no matching `input/pty` is the input-freeze signature. **The converse is not a
defect:** `input/pty` with no `input/keystroke` is the ordinary shape of an *agent* write arriving
through app-control, which never passes through xterm.js. A notebook that reads bare counts will
report a phantom freeze on a host where agents are working and nobody is typing.

⇒ Match on `session_path` within a bounded window, and when no keystroke was observed at all,
report **insufficient data** rather than a latency.

### 4.2 A probe payload can be empty because of the mirror, not the writer

Until 2026-08-20 the ytrace mirror in `append_perf_event` extracted `payload["meta"]` and sent that
as the span payload. Exactly one writer in the tree nests its context under `meta`; every other one
is flat. So **every mirrored span arrived on the bus with `payload: null`** — including `render/*`,
which meant `interval_ms` was stripped and §3.1 could not be applied at all. Fixed: the whole
payload crosses. Bytes written before that fix still have `payload: null` and cannot be repaired.

⇒ When a span's payload is null, check the record's `app_version` before concluding the writer is
at fault.

### 4.3 The registry answers "who is alive" by reading everything that ever was

`ytrace::registry::heartbeat()` appends one line per provider every 15 s to
`$XDG_RUNTIME_DIR/ytrace/registry.jsonl`, and **nothing ever prunes or rotates it** — the
generational retention that bounds the trace streams was never applied to the registry.
`registry::list()` then reads and JSON-parses the entire file to return the handful of entries
inside the staleness window.

Measured 2026-08-20: **297 MB / 2.37 M lines** on the GUI host and **194 MB / 1.50 M lines** on the
compute host, growing about **31 MiB/day**, and one `ytrace registry --json` costs **660–1120 ms**
of CPU to answer with about five rows.

⛔ **`$XDG_RUNTIME_DIR` is a tmpfs, so all of that is resident memory**, on the host whose stated
resource priority is memory first. The trace streams are bounded; the discovery index that exists to
*find* them is not.

### 4.3b "Did X ever happen?" against a single file misses everything that rotated

The live stream is `ytrace.jsonl`; everything older is `ytrace.g<ts>.jsonl`. A query that opens only
the live file is asking "did X happen *recently enough to still be in the current generation*",
which is a different question and gives the same answer shape. It has already produced a false
negative on another lane: a cache sweep that HAD run was reported as never having run, because the
harness checking for it read one file.

⭐ **The `ytrace` CLI is rotation-safe.** `query::collect_records` reads the live file **and** globs
every `ytrace.g*.jsonl` beside it, so `query`, `tail`, `incidents` and `health` all see the full
retained history. That settles the rule:

⇒ **Go through the CLI, never through the files.** It already handles rotation, and it already
resolves the home the one way the writer does (§1). A raw read gets both of those wrong at once,
and both failures return well-formed, plausible, incomplete answers. The notebooks in `notebooks/`
make no raw trace reads for exactly this reason.

⚠ If you must read files, glob `ytrace*.jsonl` — never `ytrace.jsonl`.

### 4.4 `ytrace tail --since` is silently capped by a `--lines` default you did not set

`ytrace tail` computes its limit as `lines.unwrap_or(20)` **before** applying the
`--since` window, so `ytrace tail --since 1h` returns the **last twenty records**, not an hour of
them. Nothing warns. The result is well-formed, correctly ordered, correctly filtered by category —
and describes the last few seconds while claiming to describe an hour.

⚠ This is the worst shape an instrument can take: the flag you set is overridden by a default you
did not, and the output looks exactly like a correct answer. Every rate, timeline and percentile
built on it is wrong in the same invisible way, and nothing downstream can detect it.

⇒ **Always pass an explicit `--lines` large enough to cover the window.** `notebooks/` sets
`TAIL_LINES_DEFAULT = 100_000` for this reason and never calls `tail` without it.

### 4.3c The probe NAME does not tell you the record KIND, and a temporal test needs a point event

`request/lock_wait_slow` and `request/lock_wait_window` read as two views of one thing. They are not:

| probe | shape | timestamp means | usable for correlation? |
|---|---|---|---|
| `request/lock_wait_slow` | `{request, waited_us}` | **the moment of the stall** | yes |
| `request/lock_wait_window` | `{window_ms, requests:{name:{count,mean_us,max_us,p50/p95/p99}}}` | **the end of a summary window** | **no** |

⚠ A correlation run over both — matching on the substring `lock_wait`, which is the obvious thing to
write — compares an event against a *bookkeeping tick*. It produced a confident "no correlation"
that meant nothing, and the aggregates outnumbered nothing: 83 windows against 204 point events in
one sample. The same test restricted to point events gave a genuinely interpretable answer.

⇒ **Before any temporal analysis on `request/*`, establish whether the probe is a point or a
window.** The rule generalises: an aggregate carries the numbers it summarises faithfully, so its
*values* are trustworthy while its *timestamp* is not, and nothing in the record says so.

⭐ **And always compare a hit count against the chance base rate.** With N events over a window T,
the expected number of blocks with an event inside `gap + 2W` is
`Σ (1 − exp(−(N/T)·(gapᵢ + 2W)))`. Without it, "3 of 16 blocks had a lock wait nearby" reads as a
finding when 3 is exactly what randomness produces.

### 4.4 A probe that is registered but never emitted looks identical to one that is healthy

There is no "declared but silent" report. §2.2 exists because the only way to notice was to diff the
registration list against the observed stream. A notebook that queries `title/*` and gets zero rows
cannot distinguish "the title path is healthy" from "the title path emits under another name" —
which is the actual answer here (`copy_generation/title`).

---

## 5. Retention and cost

| Stream | Bound | Where |
|---|---|---|
| ytrace live | 8 MiB, then rotate to a generation | `ytrace::DEFAULT_RETENTION` |
| ytrace generations | 64 MiB total, 3-day ceiling | same |
| perf / trace | own rotation budget | `perf::PERF_TELEMETRY_RETENTION` |
| **registry** | **none — see §4.3** | `ytrace::registry` |

Pruning happens at rotation and at the first write of each process, so it is one append per event
with no scan on the hot path. The budget is **per app home while the write rate is per process**:
with N processes sharing a home the window is `budget / (per-process rate × N)`. ⇒ **Size a budget
in bytes at the observed rate, never in days.**

---

## 6. The notebooks

`notebooks/` holds the executable analyses that stand on this map. They run headless
(`notebooks/run.sh`), read live fleet data through the `ytrace` verbs, and each ends in a **verdict
cell** with explicit thresholds so the reading is a conclusion rather than a plot. See
`notebooks/README.md`.
