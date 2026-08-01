# The optimization pass: render where nobody is looking

**Status: DESIGN SETTLED 2026-07-25. Workstream 1 STARTED (`render_probe`, live-proven
on jojo). Workstreams 2 to 4 unbuilt.**

The user's frame: yggterm must hold 50+ sessions plus several ychrome browsers "like a
heavyweight champion game like GTA 5 and not be Crysis". The felt symptom: *"jojo has
nothing other than yggterm running and the fan spins and spins because agents are
working in ychrome."*

The method he set, and this doc keeps: **baseline, optimize, baseline again.** An
improvement that is asserted rather than measured does not count.

## 1. The mechanism (why jojo burns)

> ⛔ **CORRECTED 2026-07-25 — READ §1a FIRST. The paragraph below is true but it is
> NOT the dominant term.** WebKit painting on the laptop is a real cost. But we were
> also making that painting **4x to 22x more expensive than it needs to be**, by
> forcing WebKit onto a software rasterizer on a host whose GPU works. Read §1a before
> costing any workstream here — it changes what "move the render off jojo" is worth.

The GUI lives on jojo. ychrome surfaces are native child webviews composited into the
jojo viewport. So when an agent drives ychrome, **WebKit paints on the laptop**, at
whatever rate the page asks for, whether or not a human is looking at it.

That is the bug. The fix is not to throttle painting harder. Agent browsing should
never have been on jojo at all.

## 1a. We disabled the GPU ourselves (root cause, 2026-07-25)

`configure_linux_webkit_compositing()` (`apps/yggterm/src/main.rs:4046-4053`) forces
`LIBGL_ALWAYS_SOFTWARE=1` + `GALLIUM_DRIVER=llvmpipe` + `WEBKIT_DISABLE_DMABUF_RENDERER=1`
unless `YGGTERM_ENABLE_WEBKIT_COMPOSITING=1` is set, on the stated premise that the GUI
host's iGPU "exposes only llvmpipe". **That premise is false.** The EGL platform matrix on
jojo:

| EGL platform | renderer |
|---|---|
| GBM | `llvmpipe` |
| **Wayland** | **AMD Radeon 780M (radeonsi, phoenix, ACO)** |
| Surfaceless | AMD Radeon 780M |
| Device | AMD Radeon 780M |

Only GBM fails, and only because it probes `card0` and takes `EACCES` on
`DRM_IOCTL_AMDGPU_INFO` while the compositor holds DRM master. Every ioctl on
`/dev/dri/renderD128` succeeds. **One EACCES on the wrong node was generalized into
"this host has no GPU."**

Measured, same page and duration, CPU-seconds from `/proc` (never `ps %CPU`):

| workload | soft GL + SHM (today) | hw GL + DMABUF | ratio |
|---|---|---|---|
| **WebGL glyph grid (= xterm.js 6's renderer)** | **151.56 s / 20 s = 756% of a core** | 6.85 s (34%) | **22x** |
| CSS animation | 15.33 s (77%) | 4.12 s (21%) | 3.7x |
| DOM/JS-heavy | 11.13 s (45%) | 6.44 s (26%) | 1.7x |
| static idle page | 1.36 s (5.4%) | 0.96 s (3.8%) | 1.4x |

**This explains the §3a baseline that §3a could not explain.** The 0.220 cores attributed
to "the Dioxus shell repainting" and the 0.272 cores in one web process are the *same*
phenomenon: xterm 6 removed the 2D canvas renderer, so the **terminal** paints through the
WebGL addon, and under llvmpipe every frame is rasterized on CPU across 16 threads. The
idle floor is not chrome being wasteful — it is the GPU being switched off.

Three consequences for this pass:

1. **The floor has a fix no workstream here proposed.** Do it before costing WS2,
   which is a large architectural move justified partly by a number this changes.
2. ⚠⚠ **The three settings are ONE decision — do not split them.** Hardware GL + SHM
   measured 15.82 s (no better than software); software GL + DMABUF measured **34.14 s,
   the worst of the four**. The guard's logic was right; only its premise was wrong.
3. **The real fix is to stop hard-coding the premise** — probe the host and choose from
   what it reports, rather than defaulting to the slowest configuration behind an
   opt-out nobody knew to set.

### ⛔ CORRECTED 2026-07-26 — the render win was an ArecordsFACT. Read this before quoting any number from this pass.

**What I first reported and what is actually true:**

| claim | status |
|---|---|
| `render/web_content` p50 9,870 → 530 ms (18x) | ⛔ **artifact** |
| `render/gui` p50 7,350 → 1,580 ms (5x) | ⛔ **artifact** |
| idle CPU 0.449 → 0.065 cores | ⛔ **not idle-vs-idle** |
| "the machine runs cooler" | ⛔ **confounded** |
| DRM fds 0 → 7, GPU genuinely rasterizing | ✅ **holds** |
| `copy_scan` p50 220.9 → 17.8, max 32,492 → 47.7 ms | ✅ **holds** (its OWN fix) |
| "a ~2.3x GUI-role CPU **regression** from hardware GL" | ⛔ **WITHDRAWN 2026-07-26** — unfocused vs focused |

**The mechanism of the error, because it is the whole lesson.** The "after"
window was 8.7 hours overnight; the "before" was an evening of real use. Measured
from the daemon's own event rates across the boundary — same daemon on both
sides, so they are comparable — `daemon_request/terminal_read` ran at **9.22/min
before and 0.40/min after: 23x less terminal activity.**

The kill shot is internal to the after-window itself: **`gpu_ms` was zero in 523
of its 532 render ticks.** The CPU did not move to the GPU. It simply was not
being spent, because nothing was painting. Every tick with GPU work is one where
`window_focused=True`, and all of those are from 09:18 onward.

**At matched exposure** (9.22 vs 9.25 terminal_read/min) the render tree went
**0.297 → 0.264 cores, about 11%** — and that rests on 8 post-side ticks, so
treat it as a hint, not a result.

#### ⛔ AND THE "REGRESSION" IS WITHDRAWN TOO (2026-07-26)

This section used to end by saying the plateaued GUI read **0.358-0.373 cores
against a pre-fix evening p50 of 0.297**, "on its face not better, unexplained".
A separate probe turned that into a claimed **~2.3x GUI-role CPU regression from
hardware GL**. Both are withdrawn: **the two sides differ in `window_focused`,
and focus moves the number by more than the effect being claimed.**

Bucketing every `render/gui` row in the retained corpus by GUI generation
(`payload.hot_pid`), from a copy of jojo's own `perf-telemetry*.jsonl`:

| generation | start | n | focused | `gpu_ms`>0 | `render/gui` p50 |
|---|---|---|---|---|---|
| 12 pre-fix generations | 07-25 15:12 → 19:03 | **1,131** | **0** | **0** | 0.059–1.015 |
| 1419187 (hw GL, overnight) | 07-26 00:34 | 532 | 8 | 8 | 0.026 |
| 1560015 (hw GL, morning) | 07-26 09:28 | 21 | **21** | 21 | 0.179 |
| 1659308 (hw GL, morning) | 07-26 09:54 | 32 | **32** | 32 | 0.151 |

Corpus totals: **1,256 `render/gui` rows, 1,194 of them unfocused.** Every single
pre-fix row is `window_focused=false`; every number quoted as "after" comes from a
focused window.

The focus effect, measured two ways and both larger than the withdrawn claim:

- **Within one generation** — same process, same GL arm, same session set —
  1419187 reads p50 **0.026 unfocused (n=524)** vs **0.080 focused (n=8)**: 3.1x.
- **Across generations on the same hardware arm**, 0.026 → **0.179**: 7x.

So the comparison was between two states that differ by a 3–7x variable, and it
produced a 2.3x "regression". It measures the variable, not the GL flip.

**Focus is not a label — it changes the workload.** `tick_hot_warmer`
(`shell.rs`) returns empty unless `effective_window_focused()`, so an unfocused
GUI does no SSH warming at all; `terminal_foreground_should_defer_background_refreshes`
keys on it too. ⚠ And the render probe emits the PHYSICAL `window_focused`
(`render_probe_shell_context`), not `effective_window_focused` — so a
force-foreground app-control run reports `false` while behaving as focused.

**This falsifies the EVIDENCE for a regression, not the regression.** Hardware GL
may well cost the UI process more: the GUI process now holds DRM render-node fds
and submits GPU work it never did under llvmpipe, which is a genuinely new path
inside the role that reads as more expensive. That is unmeasured. Declaring "there
was no regression" on the strength of this section alone would be the same class
of error as the original claim, pointing the other way. `scripts/gl_ab_*` is how
it gets settled.

**`copy_scan` was NOT the clean control I claimed**, for a bigger reason than the
schedule argument: it was **treated in the same deploy** (`1d174b0`), so its
improvement measures its own fix. It also shows near-zero sensitivity to render
load across three prior days (p50 179-220 ms regardless of busy or quiet), so it
could never have certified a render change either way. Its own numbers are real;
its use as a control was wrong.

**What a real verification needs:** the same terminal_read rate on both sides,
several hundred render ticks, and `window_focused` held constant. Nothing short
of that settles it.

### The standing measurement traps

Four confounders have now each produced a confident wrong number on this one
question. Every render comparison must hold all four, and the analyzer
(`scripts/gl_ab_analyze.py`) refuses a run that does not.

| # | trap | what it faked | how to hold it |
|---|---|---|---|
| 1 | **exposure** — terminal paint rate | an 18x "win" (9.22 vs 0.40 `terminal_read`/min) | drive a deterministic emitter; refuse arms whose median `xterm_write_flush` differs by >20% |
| 2 | **idle ≠ software** — `gpu_ms==0` | "the GPU is not rasterizing" on a host that had just switched to hardware | read the DRM **fd count** first (structural); a hardware arm with `gpu_ms==0` in >10% of samples is an IDLE window, not a GPU arm |
| 3 | **a control treated in the same deploy** | `copy_scan` "certifying" a render change while measuring its own fix | a control must be untouched by the deploy AND demonstrably sensitive to the variable |
| 4 | **`window_focused`** (added 2026-07-26) | a ~2.3x "GUI-role regression" that is a 3–7x focus effect | reconstruct focus as a step function from `ui/window_focus/transition` and drop samples not wholly inside a focused interval — do not average across the boundary |

⚠ Trap 4 is the one that survived a withdrawal: the correction box for traps 1–3
still quoted "0.358-0.373 against a pre-fix evening p50 of 0.297" as an open
puzzle, and that puzzle was itself trap 4. A confounder you have not named will
be re-derived by the next reader.

⚠ There is also a fifth, not on the list because it voids a run rather than
biasing it: **the env scrub.** `shm_force_for_arming` returns `Keep` on an
already-set `WEBKIT_DISABLE_DMABUF_RENDERER`, and an agent shell inherits the GL
keys from the GUI that spawned its terminal — so an unscrubbed "hardware" arm
reports `hardware_gl_probed` while actually presenting over SHM. Assert the
ABSENCE of the keys, not just the policy string.

### The numbers as first recorded (kept so the artifact is auditable)

### The shell-probe view of the same swap

GUI-only swap on the GUI host (daemon untouched, no version bump, no PTY handed
off). Before → after, same host, both idle:

| | before | after |
|---|---|---|
| DRM render-node fds | **0** | **7** (`amdgpu`, `/dev/dri/renderD128`) |
| GPU engine time | **0 ns** | GUI 335 ms · web content 698 ms |
| VRAM | — | 268 MB |
| idle CPU (whole tree) | **0.449 cores** | **0.065 cores** |
| policy on the read surface | — | `hardware_gl_probed` |

The fd count is the claim that holds: llvmpipe never opens a DRM node, so this
is structural rather than a workload artefact. The CPU figure is real but
confounded — the new GUI was minutes old with one terminal host mounted against
an old one up 5.5 h — so re-measure over hours before quoting a ratio.

Two instrument corrections fell out of proving it, both now in the tooling:
`drm-engine-*` is per-DRM-CLIENT and duplicated fds each repeat the same
cumulative value (naive per-fd summing over-counts 4-5x, measured), and **zero
engine time in a window means idle, not software** — the first post-swap read
called a freshly hardware-accelerated host "software rasterization" because
nothing happened to paint. Read the fd count first, engine time second.

### What shipped for §1a (2026-07-25, code committed)

- `crates/yggterm-core/src/gl_probe.rs` — a one-shot EGL capability probe in a child
  process. **Surfaceless platform only** (never GBM, the sole origin of the false
  premise), `renderD*` only (never `card0`, the node the EACCES came from), no disk
  cache (a stale GL belief is the failure mode being fixed), no `eglinfo` dependency.
  A hang, a crash or an inconclusive answer all report `Unknown`, which stays on
  software: promoting "we could not tell" to "probably fine" is the original bug.
- `linux_webkit_gl_policy_from_input` (`apps/yggterm/src/main.rs`) — the ONE policy.
  Precedence: `WEBKIT_DISABLE_COMPOSITING_MODE` › `YGGTERM_FORCE_SOFTWARE_GL` ›
  `YGGTERM_ENABLE_WEBKIT_COMPOSITING` › the probe. Its reason is exported as
  `YGGTERM_WEBKIT_GL_POLICY` and read back by the startup trace and published to
  `server app desktop-identity` through the client's own `webkit_gl_environment`
  (NOT through `/proc/<pid>/environ`, which only ever holds the exec-time
  environment — see `gl_probe::webkit_gl_environment_from_process`).
- `shm_force_for_arming` now takes `hardware_gl`, so SHM is refused on a probed-hardware
  host whatever arming decides. The cross-product test asserts consequence 2 above in
  every cell rather than trusting it to prose.
- **The launchers stopped deciding.** Five shell + three python re-encodings of the
  premise are gone, and the launcher marker is `v4` so installed launchers are
  rewritten. ⚠ Installed users were in a FIFTH combination outside the measured matrix
  (hardware GL libraries, compositing off, WebGL selected and unable to present), so
  the 22x figure above does NOT describe their before-state.
- **The GPU gauge is a repo instrument**: `render_top` prints `gpu_ms` per role from
  `drm-engine-*` in `/proc/<pid>/fdinfo/*`. A `-` means unreadable, never zero.

⚠ Flipping to hardware GL arms Phase F under-glass for the first time in production on
the GUI host. `YGGTERM_WEB_SURFACE_UNDER_GLASS=0` is the fallback and now genuinely
lands on hardware GL + DMABuf; `YGGTERM_FORCE_SOFTWARE_GL=1` restores the old behaviour
entirely.

⚠ The safety net is also not buying its stated benefit: 26 GUI coredumps in 10 days (still
crashing 2026-07-25), **24 with zero GL/Mesa/EGL frames**; the one genuine WebKit SEGV is in
JavaScriptCore GC. Verify any claimed win with `drm-engine-gfx` in `/proc/<webproc>/fdinfo/*`
(nonzero ⇒ the GPU really is rasterizing) plus a CPU-seconds delta.

**Secondary, same file:** `configure_linux_webkit_memory_policy()` sets
`CacheModel::DocumentViewer` (WebKit's most cache-shedding mode, meant for a single-document
viewer) and a 320 MB `MemoryPressureSettings` cap — conservative 0.33 ≈ 105 MB, strict
0.50 ≈ 160 MB. Live web processes run 400-650 MB, i.e. permanently past "strict", so they
GC-thrash by construction. For a daily-driver browser the browser-shaped setting is
`web-browser`. Not yet measured in isolation; worth a number before changing.

### The line this pass draws

> **Server-render everything nobody is looking at. Local-render what the eye is on.
> Stream pixels only when the pixels originate remotely anyway (yRDP), or the page is
> heavy and the human is purely observing.**

The counter-finding that line encodes, because getting it wrong is expensive: for a
surface a human is *watching*, moving the render to a server and streaming pixels back
is usually a **net loss**. Encode on the server, plus network, plus decode on jojo,
exceeds jojo simply painting the page. A discrete GPU on the server side makes streaming
*viable*, not *free*, and as of 2026-07-25 the hardware encode path is **unverified**
(`vainfo` was not installed on either server host). Do not build a pixel-stream lane for
human-visible browsing on the assumption that it is cheap.

## 2. Machine reality (corrected 2026-07-25, and it matters)

A deployment's server-side hosts may be **containers sharing one physical machine**, in
which case each reports the *host's* full core count and memory. Two such hosts look like
two machines and are one. The GUI host, by contrast, is typically a laptop: fewer cores,
an order of magnitude less RAM, a battery and a fan.

Two consequences that shape every decision below:

- **Moving render off the GUI host is a real win**: different silicon, mains power, no
  thermal complaint.
- **Moving work between two co-located server hosts is not a win, it is bookkeeping.** A
  plan that "spreads load across the server fleet" may be measuring one machine twice.

Before assuming parallel capacity exists, check the operator's own topology record for
which hosts are distinct silicon. Do not infer it from `nproc` or `free` inside a
container, and treat identical uptime across two "hosts" as the tell that they are one.

## 3. Measured baseline

### 3a. The render side (new, from `render_probe`)

jojo GUI pid 776144, 15-second window. Taken with the prototype example; the same
read is now `yggterm-headless server render-top --interval-ms 15000` (§7):

| role | procs | cores | PSS |
|---|---|---|---|
| `gui` | 1 | 0.220 | 568 MB |
| `web_content` | 3 | 0.272 | 714 MB |
| `web_network` | 3 | 0.006 | 82 MB |
| **total** | **7** | **0.498** | **1364 MB** |

This baseline was taken with the software-GL forcing still active and before the
probe's `gpu_ms` column existed; the same command at the same 15 s window is
directly comparable after, and a comparable "after" MUST also show `gpu_ms`
nonzero. Cores falling with `gpu_ms` still at zero would mean something else got
quieter, not that the GPU took over.

Two things fall out immediately:

1. **One web process holds all 0.272 cores; the other two sit at exactly 0.000** while
   still costing 50 MB and 69 MB. Idle profiles are a memory cost, not a CPU cost.
2. **The GUI itself burns 0.220 cores continuously.** That is chrome, not content: the
   Dioxus shell repainting. An idle app should be near zero. This is the same family as
   the blink-clock finding (N phases = N full-window blits) and is the most likely home
   of a cheap, large win.

**Read this against the lifetime average, and note the difference.** `ps` reported the
GUI at 69.9% and WebKit at 35.7% over 1h48m of elapsed time. Those are *lifetime
averages*: real (the GUI genuinely consumed ~75 minutes of CPU in 108 minutes) but not
current. The live delta is 0.498 cores. Both matter and they describe different
problems: a ~0.5-core **idle floor** that never lets the laptop settle, plus **bursts**
that spin the fan. Optimizing only the floor will not stop the fan; optimizing only the
bursts leaves the battery drain.

### 3b. The Rust side (from `perf-summary`, jojo)

| category / name | count | p50 | max | total ms |
|---|---|---|---|---|
| `background/copy_scan` | 14356 | 188.9 | 29641.9 | 3,235,760 |
| `copy_generation/title` | 2007 | 1312.9 | 10940.2 | 2,817,018 |
| `remote/resolve_yggterm_binary` | 22841 | 0.1 | 56295.1 | 1,539,343 |
| `daemon/background_copy_chore` | 12829 | 0.0 | 2704.3 | 1,319,684 |
| `daemon/persist` | 9556 | 93.3 | 576.4 | 1,133,168 |
| `daemon/snapshot_response` | 19021 | 37.7 | 1309.9 | 781,226 |

### 3b-i. The idle daemons: 100% of their cost was two GLOBAL loops (2026-07-26)

The three chained daemons on the live host burned CPU almost entirely on work
that has nothing to do with what they own. Measured per THREAD, `rchar` deltas
over one 90 s window, read from `/proc/<pid>/task/<tid>/io`:

| thread | pid 1152900 | pid 1558420 | pid 3535306 | what it is |
|---|---|---|---|---|
| `yggterm-perf-incident-monitor` | **334.7 MB** | **334.7 MB** | **334.7 MB** | re-reads the WHOLE retained telemetry corpus every 30 s |
| background copy chore | **908.1 MB** | **908.1 MB** | 454.0 MB | `load_codex_tree` over ~4 GB / 621 codex transcripts |
| whole process | 1242.8 MB | 1242.7 MB | 788.7 MB | |

**3,274 MB of file reads per 90 s — 36 MB/s — across three daemons, and the two
loops are all of it.** Byte-identical figures across a daemon owning three agent
sessions and one owning two idle shells is the tell: neither loop is per-session
work.

- **The monitor**: `summarize_perf_telemetry` read every rotated generation and
  `serde_json`-parsed every line BEFORE applying `since_ms`, to answer a question
  about the last **60 seconds**, every 30 s. The corpus is 110.8 MB across seven
  files against a 144 MiB cap, so 3 ticks x 110.8 = 332.4 MB against a measured
  334.7 MB — the attribution is arithmetic, and it gets WORSE as telemetry
  accumulates. Fixed: a generation's filename already carries the instant it
  closed, so `jsonl_read_paths_since` skips it without a read, and a raw-byte
  `"ts_ms":` upper bound skips lines inside the straddling file before serde. Replayed
  against a copy of jojo's actual corpus: **one 60 s window reads 10,156,747 of
  110,818,506 bytes — 9.2%.**
- **The chore's tree walk**: its only consumer sits behind the LLM-generation
  opt-in, and **no daemon on jojo has `YGGTERM_ENABLE_BACKGROUND_COPY_CHORE`
  set** (checked in `/proc/<pid>/environ`) — so the walk built a 4 GB answer that
  was dropped, unread, at the top of the function it was passed to. Fixed by
  `daemon_copy_chore_should_scan_local_tree`, which also refuses on a superseded
  daemon (a newer one is scanning the same corpus into the same store). The chore
  TICK is untouched — the CC title sync is the SSOT for CC titles.

⚠ Neither is deployed. The code is on `main`; every daemon on the live host is
still running the old loops, and the "after" figures above are an offline replay
plus a gate that provably does not call the walk — not a live delta. Confirm
after the next daemon bump by re-reading the same per-thread `rchar` and by
`local_tree_scanned:false` on the `daemon/background_copy_chore` perf span.

⚠ **This is also why `perf` events now carry a `pid`.** They did not, so
"`daemon/background_copy_chore` ran 12,829 times" in §3b could not be split
across the three daemons, and the attribution above had to come from a live
`/proc` walk — an instrument that works only while the process is still alive.

### 3c. The incident log, which nobody had read

> **UPDATE 2026-07-25 — BOTH ITEMS IN THIS SECTION ARE BUILT.** `server
> perf-incidents` exists and was run against the live log; the SWR fix below is on
> `main` with its policy unit-tested, and lands with the next daemon bump (it is
> daemon-side, so a GUI-only swap does not activate it).
>
> The reader's own output on jojo, which independently reproduces the hand-parse
> below and adds what it missed:
>
> | trigger | span | incidents | worst_ms |
> |---|---|---|---|
> | `span_busy` | `remote/resolve_yggterm_binary` | **65** | **483,265** |
> | `span_busy` | `startup/initial_server_sync` | 33 | 193,919 |
> | `span_busy` | `daemon_request/shutdown` | 30 | 118,531 |
> | `copy_generation_busy` | — | 23 | 51,802 |
> | `span_busy` | `daemon_request/remove_session` | 10 | 83,323 |
> | `span_busy` | `daemon_request/hot_restart` | 10 | 50,906 |
> | `span_stall` | `remote/resolve_yggterm_binary` | 4 | 31,756 |
>
> 192 incidents now. **`resolve_yggterm_binary` is 69 of 192 across both trigger
> kinds — 36% — and its worst single window is 483 SECONDS**, eight minutes of
> wall-clock inside one 60s sample window (i.e. many calls piled up). The
> hand-parse under-counted it by missing the `span_stall` rows.

`~/.yggterm/perf-incidents.jsonl` on jojo holds **183 recorded load incidents** and the
writer has been live all along (`record_perf_incident_if_hot`). Only the CLI reader is
missing: `server perf-incidents` returns *"unsupported server command"*. That log is
the durable catch for exactly the "random fan-angry" moments the user cannot predict,
and it names a clear winner:

| trigger | incidents |
|---|---|
| `span_busy remote/resolve_yggterm_binary` | 65 |
| `span_busy startup/initial_server_sync` | 33 |
| `span_busy daemon_request/shutdown` | 27 |
| `span_busy daemon_request/hot_restart` | 10 |
| `span_stall background/copy_scan` | 4 |

`remote/resolve_yggterm_binary` triggered **65 of 183** incidents and appears in the
top-3 spans of **119 of 183**. Newest incident: 13 calls, 48,992 ms total, 12,622 ms
worst.

**Root cause, read from the code.** `REMOTE_COMMAND_CACHE_VERIFY_TTL_MS` is 10 minutes
(`crates/yggterm-server/src/lib.rs:133`). Every 10 minutes per remote target the entry
goes stale and the next caller pays a full ssh round trip to
`check_remote_protocol_version`. But the cache entry **is already keyed on
`local_build_id`**, which is the thing that actually changes what the remote needs. The
time-based revalidation is re-proving something the build id already proves, and it
does so in the foreground of whatever the user was doing.

Fix direction, in order of preference:

1. **Stale-while-revalidate.** Serve the cached value immediately, revalidate in the
   background. Removes the user-visible stall entirely; worst case is one call using a
   10-minute-stale path, which `local_build_id` already guards.
2. **Raise the TTL to hours** and rely on `local_build_id` plus explicit invalidation on
   deploy / hot-restart.
3. **Negative-cache unreachable targets with backoff**, so a sleeping host costs one
   timeout rather than one per call.

Note honestly what these two instruments measure. The incident log's number one is a
**latency and stall** driver, felt as "the app hung". The render probe's 0.5-core floor
is a **thermal and battery** driver, felt as "the fan spins". They are different
problems with different fixes. Do not let a win on one be reported as a win on the
other.

## 4. Workstreams

### WS1: the instrument (STARTED)

`crates/yggterm-core/src/render_probe.rs`, committed 2026-07-25 with 14 tests and live
proof on jojo. Per-process render cost, delta-based, role-classified, emitted under the
`render` category with `duration_ms` set to CPU milliseconds so the existing aggregator
handles it unchanged.

Deliberate limit: **per-process, never per-surface.** The kernel attributes CPU to a
process; the probe reports that and lets the caller record what was realized alongside
it (`web_surface_views`, `web_surface_views_visible`, `web_surface_views_stashed`,
`web_surface_contexts`).

**`web_surface_views_visible` became load-bearing on 2026-07-27.** It is now the
page-visibility denominator: every realized view outside it has been told it is off
screen, so its `requestAnimationFrame` is paused and its timers are throttled by the
engine. Before that date an unrevealed surface reported `visibilityState: "visible"` to
its own page and animated forever — one spinner on a surface nobody had ever revealed
measured **0.241 cores of web content + 0.399 cores of GUI compositing = 0.85 cores**
against a ~0.5-core idle floor. Reading a sample: `web_surface_views_visible` equal to
`web_surface_views` while the window is showing one session is the SHAPE of that bug.
There is deliberately no separate "hidden" count — it is `web_surface_views` minus
`web_surface_views_visible`, and a second field could disagree with the first.

Measuring the change: the /proc walk (`server render-top`) over a fixed window, quoting
`web_surface_contexts` so the sample's regime is legible. Never `ps %CPU` (a lifetime
average), and never an injected `rAF` probe — a `rAF` loop installed to measure `rAF`
sustains itself at the refresh rate and measures its own existence.

⛔ **CORRECTION (2026-07-26).** This section previously read "WebKitGTK runs one web
process per profile serving every surface on it, so a per-surface CPU number would be a
fabrication", and §5.3 below drew the conclusion that profiles are not the lever because
isolation "comes from the process model". **The premise was false.**
`WebSurfaceHost::open` called `WebContext::new(profile_dir)` unconditionally, once per
SURFACE, and a `WebContext` is a process pool — so two tabs of one ychrome session (which
share a profile by construction: `web_surface_new_tab` copies the first tab's profile)
got two `WebKitWebProcess`es, two `WebKitNetworkProcess`es, and two in-memory cookie
jars writing the same on-disk `cookies` file. jojo's own telemetry corroborates it:
`web_content` and `web_network` process counts moved 1→2→3 in lockstep with realized
surfaces. So per-process WAS per-tab, profile partitioning was never the lever, and the
process model was a no-op dial while every context had its own pool.

Contexts are now shared per `(profile jar, egress, control endpoint)` — see
`web_context_key` in `vendor/dioxus-desktop/src/web_surface.rs`. Two consequences for
anyone reading numbers out of this doc:

- **The free per-tab CPU attribution is gone.** It existed only because the bug existed.
  Any per-surface claim now has to come from page-level accounting, not from `/proc`.
- **`web_surface_contexts` says which regime a sample was taken in.** Read it before
  comparing any two samples. A sample where `web_surface_contexts` tracks
  `web_surface_views` is from the old regime (or from tabs on different sessions), and a
  before/after across that boundary is not a measurement.

Remaining in WS1:

- ~~**Wire continuous sampling.**~~ ✅ **SHIPPED** (`a216eb9`, `spawn_render_probe_loop`):
  a 60 s tick in the GUI, read-only, `/proc` walk off the UI thread, per-role events.
  Live-proven on the GUI host — `gui` ~0.220 cores, `web_content` ~0.272. Still owed on
  the tick: the CONTEXT. It passes `{web_surfaces, live_sessions}` where `web_surfaces`
  counts SESSIONS, not realized webviews, with no stashed/visible split and no window
  visibility — which is what §4 WS1 actually asked for.
- ~~**`server render-top`**, promoting `examples/render_top.rs` into a real command.~~
  ✅ **BUILT 2026-07-25 — LANDED, NOT SHIPPED: the arm has never executed once.**
  `--pid` names any process tree; with no `--pid` the registered-client registry picks
  the GUI, through the same `choose_app_control_pid` the `server app` verbs use.
  `--json` prints the same report the table is built from. The example is deleted: one
  read path, not two. It reads /proc in-process and is in the no-handoff carve-out.
  What is actually under test, so nobody has to re-derive it: the rollup and ranking
  (`render_probe::tests::render_top_report_rolls_up_roles_and_ranks_processes_by_cpu`,
  `render_top_ranking_is_stable_for_equal_cost_processes`), the untargeted GUI choice
  including the read-vs-mutation split (`choose_app_control_pid` tests in
  `yggterm-server`), and the carve-out
  (`local_state_readers_never_hand_off_to_the_installed_binary`).
  What is NOT: the flag parsing and its defaults (`--interval-ms` 5000, `--top` 10),
  which live inline in the CLI arm and are only reachable by running it.
- ~~**`server perf-incidents`**, a reader for the 183 records already on disk.~~
  ✅ **SHIPPED 2026-07-25** and run live — see the update box in §3c. Groups by
  trigger, ranked by count, `--list`/`--json` for raw records.
- ~~**Collapse the duplicate `/proc` parser.**~~ ✅ **SHIPPED 2026-07-25.**
  `render_probe::ProcMemory` / `read_process_memory` is the one owner of "how much
  memory does pid N use"; the shell's chore calls in for its own pid, and the tree walk
  reads one file per pid instead of two. The `status` VmRSS fallback is LABELLED in the
  returned value, so its zeroes cannot pass for a PSS reading nobody took.
- ~~**The sampler's clock.**~~ ✅ **FIXED 2026-07-25.** The GUI tick fed
  `current_millis()` — a `SystemTime` wall clock — into a contract that said monotonic,
  so `interval_ms` inflated across a suspend/resume while CPU ticks stood still and
  `core_fraction` under-reported after every wake. The probe owns its clock now and
  `observe` no longer accepts one.
- ~~**The render probe was polluting the incident log.**~~ ✅ **FIXED 2026-07-25.**
  `render` spans carry CPU milliseconds in `duration_ms`; `detect_perf_incident` judged
  them by wall-clock rules, producing 35 of 222 incidents on the live host (worst:
  `span_stall render/web_content max_ms=70850`, which is 1.18 cores of ordinary work).
  Because incidents debounce five minutes and the detector returns the FIRST match,
  those could also MASK a genuine stall. A span's time base is now owned by
  `perf_span_time_base`, CPU spans are judged in cores (`span_cpu_hot`, ≥1.2), and they
  are tried last so a real stall always wins the slot. `perf-summary` grows a `clock`
  column.

### WS2: move agent rendering to the server

This is the answer to "as much rendering to the server", and it already has an approved
spec: `ychrome/docs/agent-engine.md` (2026-07-13, nothing built). It stops being a
feature and becomes the optimization deliverable, because an agent-driven page that
renders on a server host costs the GUI host exactly zero.

Two lanes, sequenced so the risky one cannot block the useful one:

- **Lane B ships first, because it works today.** The installed WebKitGTK 2.52.4 plus a
  headless compositor on a server host (`sway --headless` or Xvfb, both already proven in the
  shadow-client work). Cost: one compositor process per host.
- **Lane A is a timeboxed spike.** True WPE headless, no display server at all. There
  is no mature Rust binding. Verify reachability from Rust before committing to it.
  **✅ RESOLVED 2026-07-26 — Lane A is REACHABLE; see `docs/spikes/wpe-lane-a/`.**
  A dependency-free Rust binary loaded a page, reached `WEBKIT_LOAD_FINISHED`, read
  the title and got 2 exported frames, with no X and no Wayland: 278 ms warm, on a
  hardware render node. Two corrections the spike forced, recorded here because the
  text above stated both wrongly:
  - **`WPEDisplayHeadless` is NOT the mechanism.** Debian builds WPE without
    `ENABLE_WPE_PLATFORM` — zero `wpe_display_*` symbols exist. The working route is
    the legacy **libwpe + WPEBackend-fdo exportable backend** (in-process nested
    Wayland compositor, buffers exported to our callbacks). Drop `WPEDisplayHeadless`
    from the plan.
  - Package is **2.52.5-1**, and the fdo dev package is the **1.0** ABI
    (`libwpebackend-fdo-1.0-dev`); a `1.1` does not exist.

**★ USER-SETTLED 2026-07-26: WPE is the destination, not a spike.** The user
raised it unprompted ("a lower primitive of WebKitGTK") and agreed with the
full philosophy, so Lane A is promoted from timeboxed spike to the target
architecture, with Lane B as the bootstrap that must not wait for it. The
settled reasoning, so it is not re-derived:

- WPE is the same engine core (WebCore + JSC, same GObject API family minus
  the GtkWidget), so the ychrome verb plane and site lore carry over — this is
  NOT an engine switch.
- What it kills structurally: the GTK-widget focus-grab class (the fifth focus
  path — views are not widgets, input routing becomes our code), the
  under-glass widget-stacking hazard (we composite exported DMABuf textures
  deliberately), and the GUI-host burn / dev-no-client gap (agent surfaces
  render server-side with no compositor at all).
- What it does not fix, recorded honestly: JSC/WebCore heap crashes are
  engine-core and identical; the GL crash surface changes owner (ours, not
  GTK's); nothing for Windows (WebView2 remains the Windows engine — WPE is
  the Linux/server plane only).
- The "no Rust binding" blocker is a bounded maintenance cost, not a wall.
  **Corrected 2026-07-26 by the Lane-A spike:** it is bounded for a better
  reason than the gir plan assumed. Debian ships **no `.gir`/typelib for WPE
  at all** (`gir1.2-webkit-*` are the GTK port only), so "regenerate gir
  bindings and vendor them" cannot be executed without rebuilding WPE from
  source with introspection. It does not need to be: the surface actually
  required is **19 hand-written `extern "C"` declarations** plus one `repr(C)`
  callback struct — no bindgen, no gir toolchain, no new dependency. Hand-written
  FFI is the recommendation; see `docs/spikes/wpe-lane-a/README.md`.
- Sequence: agent engine on WPE first (agents are the only consumers — lowest
  risk, highest immediate win), then GUI web-surface hosting migrates
  view-by-view (WebKitGTK remains under the Dioxus chrome until last), each
  step reversible and each killing a named bug class.
- **ghostty is NEVER the answer here** (user-settled the same day): no
  native-terminal-renderer detour rides this workstream.

### WS3a: two recorded speedups for agent web flows (user-raised 2026-07-26)

1. **Lore-compiled flows (the primary item).** A portal's KNOWN flow gets
   compiled from site-lore into a deterministic `web batch` script — steps,
   assertions, readback checks — so the model re-enters only at genuine
   branches (captcha, OTP, unexpected state). The measured cost of live runs
   is agent ROUND TRIPS, not verb latency; this attacks exactly that, with
   more reliability rather than less. The skillify idea, applied to the verb
   plane.
2. **The lore-anchored pixel rung (the deliberate "dirty" fallback).** For
   DOM-hostile surfaces only (canvas, cross-origin frames, obfuscated
   markup): lore caches pixel anchors from `capture-element` crops; a click
   runs a MILD DOM identity check first (is this the page/region lore says),
   then `do click --x --y` under the existing elementFromPoint guard. Slower
   and less stable than addressed clicks — never the primary mode — but it is
   the same primitive yRDP's Windows plane needs (no DOM exists there at
   all), so it is built once and shared. Do not reach for it where a selector
   works; the post-2.12.16 pinned/readback DOM plane is the fast path.

### WS3: agent dev tools and the read ladder

The "cli dev tools++ for agents" the user asked for.

- **`ychrome ctl top`**: a `top` for logical pages. CPU, PSS, live/parked, owning agent,
  last verb. The single most useful tool for the stated complaint.
- **Per-verb trace spans** (`navigate`, `wait`, `eval`, `screenshot`) with duration and
  bytes, so "why was that flow slow" has an answer instead of a guess.
- **The read ladder.** Each rung is roughly an order of magnitude dearer than the last:

  1. HTTP / API / JSON, no engine at all
  2. DOM text (`innerText`) via eval
  3. structured DOM snapshot
  4. cropped screenshot of one element
  5. full viewport pixel

  Rungs 1 and 2 are the default; a full pixel should be a deliberate act. This is not
  theory: the existing `1mg.com` site-lore already works this way (URL search plus an
  `innerText` regex, zero pixels), which is the evidence the ladder matches practice.

### WS4: the cheap caching wins

Independent of the render work, so it can run in parallel.

- ~~`resolve_yggterm_binary`: stale-while-revalidate, per section 3c.~~ ✅ **BUILT
  2026-07-25**, unit-tested, **not yet active** — it is daemon-side, so it starts
  paying off at the next daemon bump. Past the TTL the resolver now returns the
  entry it just used and revalidates on a background thread; a changed
  `local_build_id` still resolves in the foreground, and staleness is bounded at
  six hours.
- ~~`copy_scan`: incremental off mtime, skip unchanged stores, back off when nothing
  changed.~~ ✅ **BUILT 2026-07-25, NOT DEPLOYED.** It was not a caching problem at
  all. `shell.rs` put a whole `RemoteMachineSnapshot` on every copy target — 644
  targets × a 1.75 MB machine ≈ 1.1 GB of allocation per scan — and each target then
  opened three sqlite connections (~2151 per scan) through the store's one-shot
  resolver wrappers. Targets now carry a session-list-free `RemoteMachineRef` and the
  sweep holds ONE open `SessionTitleResolver`. The mtime-incremental idea is still
  open, and is now measurable: `build_local_cwd_tree` has its own
  `background/local_tree_scan` span, and the daemon chore no longer runs it inside the
  runtime read lock (which is also why `daemon/background_copy_chore`'s p50 will RISE
  from 0.0 — the span finally contains the work).
- ~~`daemon/persist`: dirty-flag or debounce; the state is re-serialized far more often
  than it changes.~~ ✅ **BUILT 2026-07-25, NOT DEPLOYED.** Content-hash gate plus a
  file re-stat: an unchanged persist writes nothing and takes no backup. The
  unconditional primitive is kept for `PrepareUpdateRestart` and the handover paths.
- `snapshot_response`: memoize by generation. **Half done.** The per-session screen
  work under it is memoized on `(output seq, resize seq, PTY width, model size)` —
  built 2026-07-25, not deployed. Precisely what the memo removes on a hit: the
  `screen_state` lock, the `formatted_screen_max_column` walk and the clip
  rewrite. It does **not** remove the clone — the hit path still hands back an
  owned `String` copy of the formatted screen, so the allocation is unchanged.
  (Commit `aaf3906`'s opening line reads as if the clone went away; it did not.
  Killing it means handing callers the `Arc<str>` the memo already holds, which
  is a separate change with its own caller ripple.) The remaining cost is the 2.1 MB
  `remote_machines` deep copy in `snapshot()` itself; the fix there is `Arc` +
  copy-on-write, NOT a generation counter (there are 15+ mutation sites and a
  hand-bumped counter that one of them forgets serves a stale session list to the
  sidebar). **Still open.**
- `copy_generation/title` + `summary`: cache by content hash, never regenerate for an
  unchanged transcript. **Still open**, and the 429 rule governs it: the negative row
  may only be written on an outcome that came back from the MODEL, never on a 429 and
  never on a heuristic returned after a transport failure.
- The GUI's 0.220-core idle floor: hunt full-window blits in the Dioxus shell.

## 5. Constraints that outrank being fast

1. **Never nuke a background ychrome.** *"suppose I have a youtube playlist running in
   background while I work here."* So "invisible implies suspend" is WRONG. The axis is
   **paint**, not existence: stop compositing and rAF for an unseen surface, leave
   audio, timers and network alone. Any policy must distinguish *not visible* from
   *not wanted*.
2. **Agent work is normal load, not an anomaly.** Optimize for it; do not throttle it
   away.
3. **Profiles stay as they are.** They are ychrome's multi-account feature. Isolation
   comes from the **process model**, which yggterm has never set, and the policy reads
   the machine: process-per-site is harmful on a 14 GB laptop and free on a
   large-memory server. That is literally what a settings-scaling game engine does.
   ⚠ Note the ordering the correction in WS1 forces: setting the process model was a
   **no-op** while every surface had its own `WebContext` (a context is its own pool, so
   you got process-per-tab whatever the model said). It only becomes a real dial now
   that contexts are shared, and `webkit_web_view_new_with_related_view` — already used
   in-tree for OAuth popups — is the finer-grained control than either model setting.
4. **The lease must not be optimized away.** `c068f67` made verbs renew a surface's
   lease so an actively-driven surface is not reaped. That deliberately raises
   residency. Do not "optimize" it back into reaping mid-flow.

## 6. Small bug from the same pass — fixed, with one residual

An agent launching a session without revealing it used to steal the user's session
**focus**. `terminal new --no-activate` now hands the user's view back as ONE value —
active session, view mode and selected row together — in the same mutation window as
the create's snapshot apply, so nothing flashes. The last case to fall was a create
made while NO session was active: the start page was captured as an absence rather
than as a state, so the hand-back had nothing to restore to and the new row's
activation stood. It is now a named viewport, restored through the same setter the
viewport history uses.

**Residual, deliberately outside that scope.** The hand-back is client-local. The
daemon still marks a newly started session active whatever the flag said, so the GUI's
viewport and the daemon's active path disagree until the user opens something, and any
path that adopts daemon truth wholesale re-adopts the new row. The honest fix is
daemon-side: a create that says "do not activate" must not move the daemon's active
session either.

## 7. Verifying a claimed win

Re-run, and quote, both instruments:

```sh
# render side, on the GUI host (no --pid: the client registry picks the GUI).
# The gpu_ms column is the GPU gauge; a `-` means unreadable, never zero.
# ⚠ render-top, NOT the in-app render probe: RENDER_PROBE_INTERVAL_MS is fixed
# at 60_000, so "several hundred ticks" from the in-app sampler is several
# hundred MINUTES. An A/B drives the interval from outside.
yggterm-headless server render-top --interval-ms 15000
# is the GPU actually rasterizing? nonzero and RISING across two reads
grep -H drm-engine /proc/<webproc>/fdinfo/*
# which GL path is this window even on? (the client's own view, NOT /proc environ,
# which only ever holds the exec-time environment)
yggterm-headless server app desktop-identity | grep -A8 webkit_gl_environment
# Rust side (the `clock` column says whether a row is wall or CPU time;
# the `pids` column says WHICH process burned it — three daemons and a GUI
# append to one perf-telemetry.jsonl, and until 2026-07-26 they could not say)
yggterm-headless server perf-summary --category render
yggterm-headless server perf-summary
```

A win is a moved number in the table above, on the same host, over a comparable window,
with the app doing comparable work. Anything else is an anecdote.

For the GL question specifically, do not hand-roll it: `scripts/gl_ab_experiment.sh`
runs the S/H/G/S2 arms and `scripts/gl_ab_analyze.py` gates the result on all four
standing traps plus a drift control. It is built to answer **"this settles nothing"**,
and that is a valid outcome — three earlier hand-rolled attempts each returned a
confident number instead.

## 8. What the agent-load incident taught, and what is still owed (2026-07-26)

The pass was chasing a render number while the actual felt problem was
elsewhere. Under a real swarm of agents the machine went to **124 kB of free
swap out of 16.7 GB**, 76.6 °C, and the GUI could not answer `server app state`
for 25 seconds. What was consuming it was not painting:

- **The reaper was causing the pressure it reacted to** — 166 destroy/recreate
  cycles in fifteen minutes, one churned WebKit process ballooning to 3.9 GB.
  Fixed (real-pressure predicate + per-surface hysteresis + audio veto).
- **One WebContext per TAB** — see `docs/web-surfaces.md`. This is where the
  memory ceiling for "hundreds of tabs" actually lives, not in the renderer.
- **Three chained daemons burned 0.205 cores** — 36% of yggterm's total 0.564
  — while owning almost nothing, because each re-read the whole retained
  perf-telemetry corpus every 30 s and each walked the full transcript store
  regardless of what it owned. Both fixed; measured after, the new daemon runs
  0.0130 cores against the old ones' 0.0183/0.0157 — **~25%, not the large win
  the finding implied**, because the corpus is smaller now than when it was
  measured. Re-measure when the log grows before quoting it.

**Still owed, and it is the honest gate on this whole pass:** a matched-load
A/B. `scripts/gl_ab_experiment.sh` + `gl_ab_measure.sh` + `gl_ab_analyze.py`
exist and are built to be able to answer "this settles nothing". They need a
quiet host, `window_focused` held constant on both arms, several hundred render
ticks, and the same `daemon_request/terminal_read` rate on each side.
`YGGTERM_FORCE_SOFTWARE_GL=1` is the A/B switch, so both arms run on ONE build.

⚠ Do not start that experiment while agents are working: the one arm that did
paint was contaminated by five concurrent agents on the host, and the arm after
it measured a GUI displaying a different session entirely.

## 9. Webapp launch latency: it is not the cache (2026-07-31)

**The report:** *"I see helium (chromium fork or any chrome based browsers) they
launch webapps so fast. We need a stellar caching solution like chrome."*

The observation is right and the named cause is wrong. Measured against Helium
0.14.7.1 (Chromium 150) on the same host, the same page and the same protocol,
**our HTTP disk cache already works and contributes almost nothing to the gap.**
Roughly 100% of the warm-launch difference is engine STARTUP, paid before the
page's navigation clock even begins.

### 9a. The instrument

- `crates/yggterm-webprobe` — a binary whose whole lifetime is ONE launch. It
  reports the Rust-side phases (`gtk_init`, `WebContext::new`,
  `WebViewBuilder::build_gtk`), the WebKit auxiliary-process spawns observed
  from `/proc` (not inferred), wry's `PageLoadEvent`s, and the page's own
  `PerformanceTiming` / `PerformanceResourceTiming` / paint entries over IPC.
  `--second-url` launches a second surface in the same process, which is the
  only way to tell a per-process cost from a per-surface one.
  `--adblock` times WebKit's content-filter store.
- `scripts/webapp_launch_fixture.py` — a deterministic heavy webapp: N distinct
  copies of this repo's real minified bundles, `immutable` on the assets and
  `no-cache` on the shell (the convention Chromium's code cache is built for),
  every script tag bracketed in `performance.mark`s.
- `scripts/webapp_launch_bench.py` — runs both engines, cold and warm, and
  computes `launch_ms = (timeOrigin - spawn_epoch) + loadEventEnd` identically
  on each side so neither gets to define "loaded" its own way.

> ⚠ **Two instrument traps, both of which produced a confident wrong answer here
> before being fixed.** (1) The probe originally exited the instant `load` fired;
> WebKit's network process writes cache records asynchronously **after** that, so
> the cache held nothing but its 8-byte salt and the next run refetched
> everything. That artifact read exactly like "WebKitGTK never caches". Hence
> `--settle-ms`, default 2000. (2) **WebKitGTK reports `transferSize` AND
> `decodedBodySize` as 0 for a cache hit**, where Chromium reports
> `transferSize == 0, decodedBodySize > 0`. A cache-hit detector written against
> Chromium's semantics scores every WebKit hit as a miss. `network_bytes == 0`
> is the portable test.

### 9b. The numbers

Fixture (8 bundles, 2.95 MB of real minified JS), localhost, Xvfb on `dev`,
median of 5. ⚠ The host carried `loadavg ~30` from other lanes' builds, so the
absolute figures are inflated; the RATIOS and the phase shares are the result.

| arm | exec→nav | TTFB | FCP | interactive | JS exec | load | **LAUNCH** | net bytes |
|---|---|---|---|---|---|---|---|---|
| webkit cold | 2147 | 2 | – | 510 | 491 | 536 | **2701** | 2,947,452 |
| webkit warm | 1118 | 0 | – | 335 | 325 | 346 | **1396** | **0** |
| chromium cold | 619 | 102 | 492 | – | 585 | – | **1902** | 0 |
| chromium warm | 473 | 92 | 252 | 285 | **151** | 285 | **758** | 755 |

Warm against warm, which is the user's actual case:

- **Total gap 638 ms.** Startup accounts for **645 ms** of it (1118 vs 473); the
  whole page half accounts for **61 ms** (346 vs 285). The gap is startup.
- **Our cache works.** `net bytes` is 0 on the webkit warm arm: the entire
  2.95 MB came off disk. Cold→warm buys us 1305 ms. Caching is not the deficit.
- The one place Chromium's *code* cache shows up is `JS exec`: Chromium
  585 → 151 cold→warm (3.9x), ours 491 → 325 (1.5x). On identical warm bytes
  we spend **325 ms to Chromium's 151 ms**. Real, and second order: ~27% of the
  gap, against startup's ~100%.

Phase decomposition on a QUIET host (5 runs, `about:blank` — no content at all,
so every millisecond below is fixed cost):

| phase | ms |
|---|---|
| `gtk_init` | 70–87 |
| window realize (cumulative) | ~110 |
| `WebContext::new` | 46–47 |
| `WebViewBuilder::build_gtk` | 56 |
| NetworkProcess spawn observed | ~125 |
| WebProcess spawn observed | ~175 |
| navigation requested | ~207 |
| **`load-changed: Started`** | **~860** |

**~650 ms passes between our asking for the navigation and WebKit beginning it,
on a blank page.** Repeatable to ±15 ms across 5 runs.

### 9c. The root cause, and the falsification that survived

> **Every web surface spawns its own WebKit WebProcess and waits ~650 ms for it
> to initialize before the page's navigation can start. WebKitGTK 2.52.5 has no
> working way to have that process ready in advance.**

The falsification attempt that mattered: *if this were a once-per-process
startup cost, a SECOND surface in an already-running process would skip it.*
`--second-url` says it does not. A second surface spawns its own WebProcess
(seen in the spawn timeline) and still waits ~710–790 ms before `load` starts,
whether it shares the first `WebContext` or gets its own. Sharing a context
shares only the **NetworkProcess** (1 vs 2 in the two arms). So the cost is
per-SURFACE, and prewarming one spare process would not have covered it either.

Why WebKitGTK cannot currently avoid it, verified against our installed 2.52.5:

1. **`WebProcessCache` — the mechanism that would reuse a departing surface's
   process — has capacity 0.** `ENABLE(WEBPROCESS_CACHE)` is on and our cache
   model already qualifies, but the capacity calculation bails out; the string
   `WebProcessCache::updateCapacity: Cache is disabled because process swap on
   navigation is disabled` is present in our linked `libwebkit2gtk-4.1.so.0`.
   PSON is reachable only through the **construct-only** GObject property
   `process-swap-on-cross-site-navigation-enabled` (default `FALSE`); the GTK
   constructor engages a `std::optional<bool>` with `false`, which permanently
   shadows the WebPreference, so no runtime toggle can reach it. Were it on,
   capacity would be `min(RAM/256MB, 30)`.
2. **Prewarming does nothing on GTK.** `WebProcess::prewarmGlobally()` — the
   function that would build the JSC VM, heap and normal world in advance — is
   `#if PLATFORM(COCOA)`. `ProcessType::PrewarmedWebContent` is assigned only in
   `WebProcessCocoa.mm`. A "prewarmed" process on our port is a pre-forked,
   pre-IPC-connected, otherwise stone-cold process. (The symbol is in our `.so`
   because the body compiles unconditionally — a standing reason not to reason
   from `strings` alone.)
3. **JavaScriptCore has no persistent bytecode cache in the WebKitGTK port.**
   This is the honest bound on how close to Chromium we can get on `JS exec`:
   it is not a setting we have failed to turn on, it is a feature that does not
   exist. It caps the achievable win at roughly the 174 ms measured above and
   no configuration change will move it.

Ranked by measured contribution to the 638 ms warm gap:

| # | term | measured | reachable? |
|---|---|---|---|
| 1 | WebProcess startup per surface | ~650 ms | only via PSON + `WebProcessCache`, a security/architecture change |
| 2 | JS parse+compile with no code cache | ~174 ms | **no** — JSC has no equivalent |
| 3 | HTTP disk cache | **0 ms** | already working |

### 9d. What WAS fixed, because it is far larger than any of the above

The adblock content filter was **compiled from source at every GUI start**.
`webkit_user_content_filter_store_save` COMPILES; it is not "write if absent",
and `vendor/dioxus-desktop/src/web_surface.rs` called it unconditionally.
Measured with `yggterm-webprobe --adblock` on 146,748 rules (13.9 MB):

| path | wall |
|---|---|
| `save` (compile) | **17,180 ms** |
| `load` of the same, already compiled | **3.7–4.3 ms** |

**~4,300x**, at every launch, during which nothing was filtered. The old code
carried a comment that page loads are slower than the compile; that was true
against the 60-rule file it was written for and is now wrong by three orders of
magnitude.

Fixed by loading first and compiling only on a miss, keyed on a content stamp:
the store identifier IS `yggterm-adblock-<sha256(rules)[..32]>`, so a hit proves
byte-identical source and there is no separate stamp file that could disagree
with the bytecode it describes. Changed rules simply miss and compile once
(verified: an appended rule re-compiles). Stale generations are pruned after the
filter is in hand, never before.

⚠ **Scale this against §9b before ranking it.** 17 s is ~12x the entire
cold launch it sits next to. It is a GUI-START cost, not a per-webapp cost, so
it does not appear in the per-launch table — but for "why does this feel slow"
it dominates everything else in this section combined.

### 9e. The real-site check, and why the fixture is the result

The same benchmark against `khanacademy.org` (one of the user's own profiles),
median of 3, same host and display:

| arm | exec→nav | TTFB | FCP | interactive | load | **LAUNCH** | net bytes |
|---|---|---|---|---|---|---|---|
| webkit cold | 2167 | 362 | 2093 | 663 | 3300 | **6841** | 100,399 |
| webkit warm | 1959 | 358 | – | 504 | 1638 | **3597** | **0** |
| chromium cold | 1303 | 786 | 2120 | 1040 | 3996 | **5771** | 100,582 |
| chromium warm | 498 | 573 | 1356 | 709 | 3769 | **4227** | 10,968 |

**On a real site we come out AHEAD end to end (3597 vs 4227 ms), and the
startup gap still reproduces (1959 vs 498 exec→nav, ~4x).** Both of those are
worth saying plainly, and the first is why this section leads with the fixture:

- A real site does not serve the two engines the same bytes. Khan Academy
  user-agent-sniffs; our page half (1638 ms) beating Chromium's (3769 ms) most
  likely measures a lighter page, not a faster engine. There is no way to hold
  content constant on someone else's server, which is the entire reason
  `webapp_launch_fixture.py` exists.
- Chromium's warm arm still pulled 10,968 bytes, so it was only partly warm —
  the script says so rather than quietly averaging it in.

So: **the fixture is the measurement and the real site is the sanity check.**
The one term that survives both, unchanged in sign and roughly in magnitude, is
engine startup. That is the finding.

### 9f. What was deliberately NOT changed

- **PSON / `WebProcessCache`.** Turning on
  `process-swap-on-cross-site-navigation-enabled` at `WebContext` construction
  is the only route to reusing a WebProcess, and it is worth ~650 ms per launch.
  It is not a perf tweak: it changes the process/isolation model for every
  surface, and there is a source-level report that with sandbox + PSON both on
  `tryTakePrewarmedProcess` orphans a process per navigation (unverified at
  runtime here; we set neither today). It also needs `web_context_key` in
  `web_surface.rs` to be re-reasoned, which four lanes were editing. **Plan:
  land it alone, behind an env arm, with this section's bench as the gate.**
- **The GUI's own boot assets.** The Dioxus shell serves its HTML/JS over a
  custom URI scheme, and custom-scheme responses do not go through
  `NetworkCache` — they write zero cache records, so the shell re-fetches and
  JSC re-compiles them at every start. That is a GUI-start cost like §9d, not a
  webapp-launch cost, and the fix is to shrink/split what runs at startup rather
  than to reach for a WebKit setting. Not measured here.
- **`WEBKIT_DISABLE_DMABUF_RENDERER=1` on the live host** while
  `YGGTERM_WEBKIT_GL_POLICY=hardware_gl_forced` (read from jojo's WebKit child
  environs, 2026-07-31). §1a says those three settings are ONE decision and that
  hardware GL + SHM measured no better than software. This is the render lane's
  question, not this one's, but it is recorded here because it was found here.

> ⚠ **Correction to a premise this lane was handed.** "The cache model is
> env-optional and currently UNSET, so we inherit WebKit's default" is FALSE.
> `configure_linux_webkit_memory_policy` (`apps/yggterm/src/main.rs:5091`) sets
> `YGGTERM_WEBKIT_CACHE_MODEL=web-browser` with `set_var` before any
> `WebContext` exists. It does not appear in `/proc/<gui>/environ` because that
> only ever holds the EXEC-time environment — the standing trap in §1a — but it
> is plainly there in the environ of the WebKit children the GUI forked
> afterwards, which is where it was verified on jojo. The cache model is already
> explicit and already correct.

## 10. SPA navigation on open-webui: it is not ours, and it is not the launch bug (2026-07-31)

**The report:** *"the site as given is slow on ychrome but fast on chromium
apps. The slowness comes when I click something in the sidebar (another chat
to switch into openwebui)."* The site is open-webui.

⚠ **This is a different bug from §9 and the two must not be merged.** §9's
~650 ms is WebProcess startup, paid once per surface before navigation begins.
Clicking a chat in the sidebar starts no process, opens no surface and performs
no top-level navigation, so none of §9 can reach it. Measured separately,
root-caused separately, and the causes have nothing in common.

### 10a. The instrument

- `scripts/spa_nav_probe.js` — one engine-agnostic in-page probe. Its spine is
  a **4 ms self-rescheduling timer**: a timer that cannot fire is a main thread
  that is busy, so the gap distribution IS the blocking profile, in any engine.
  Deliberately nothing here uses `longtask`, `event` timing or
  `long-animation-frame` — Chromium has them, WebKit does not, and a
  decomposition that only works on one side answers nothing. It also carries a
  fetch/XHR wrapper, a resource-timing byte sweep, a mutation timeline, an
  observer-free element-count poll, a forced-reflow accountant with optional
  stack capture, and an optional pre-click stylesheet for mechanism probes.
- `scripts/spa_nav_bench.py` — four arms, one probe: `webkit` (plain
  WebKitGTK via PyGObject, same profile jar and cache model as the app, with
  **none of yggterm around it**), `chromium` (Helium over CDP, same display,
  same size, localStorage copied from the WebKit jar so both engines start the
  app in the same state), `ychrome` (the product path, through
  `server app web eval`), `layout` (the controlled microbenchmark).
- `scripts/spa_nav_layout_bench.js` — a flat/deep synthetic document used to
  price layout SHAPE apart from layout SIZE.

`yggterm-webprobe` could not be reused for this: its whole lifetime is one
launch, and this bug lives entirely after the launch is over.

### 10b. The numbers

a self-hosted open-webui instance on the LAN, the user's own profile jar, 1600x1000, Xvfb on `dev`,
32 switches per engine over 8 chats x 2 rounds x {MutationObserver on, off},
medians. ⚠ `dev` carried `loadavg` 20-30 from other lanes throughout, so the
absolute figures are inflated; the RATIOS are the result.

| p50 | WebKitGTK 2.52.5 (plain) | Chromium 150 (Helium) | ychrome (under-glass) |
|---|---|---|---|
| route change | 158 ms | 49 ms | 567 ms |
| **first message node in the DOM** | **1,454 ms** | **211 ms** | 1,674 ms |
| **message list rendered and stable** | **2,086 ms** | **366 ms** | 2,604 ms |
| **main-thread blocking** | **2,282 ms** | **210 ms** | 3,074 ms |
| longest single task | 1,334 ms | 161 ms | 1,722 ms |
| forced full relayout, same DOM | 356 ms | 65 ms | 937 ms |
| chat payload fetch | ~150 ms | ~150 ms | ~150 ms |
| DOM after switch | 4,132 nodes | 4,128 nodes | 3,656 nodes |

**The user's felt latency is the "rendered and stable" row: 2.1 s against
0.37 s, 5.7x.** Blocking is **96.6% of WebKit's wall time and 15.4% of
Chromium's** (per-run medians; WebKit's worst run 98.5%, its best 84%) — so this is a busy main thread, not the network, not paint, and
not scheduling. The chat payload arrives in ~150 ms on both engines and the
route changes in ~50-160 ms; everything after that is one enormous synchronous
task. A representative WebKit timeline, gaps as `[start_ms, duration_ms]`:

```
[0, 161] [224, 1994] [2218, 657] [2880, 585] [3509, 682]
```

**`ychrome` is the same order as plain WebKit, not worse in kind.** It reads
~1.25x the plain arm, but its driver polls the result back through
`server app web eval` every 400 ms, which runs JS on the page's own main
thread — an observer cost the other two arms do not carry. Treat that column
as an order-of-magnitude check. What it establishes is the only thing it needs
to: **the deficit is fully present with no yggterm in the picture at all.**

### 10c. The root cause

> **open-webui's chat-switch effect flush takes about four geometry reads while
> the DOM is dirty. Each one forces a synchronous style recalc + layout of the
> whole document, and on this document WebKitGTK needs ~180-500 ms per forced
> layout where Chromium needs ~6-65 ms. Four of them is the 1.2-2.5 s task.**

Instrumenting the layout-forcing accessors (`spa_nav_probe.js`, `opts.reflow`)
puts **66% of WebKit's blocking time inside those getters**, and naming the
callers via `opts.stack_for` names the exact call sites in the app bundle:

| accessor | calls/switch | direct caller | WebKit |
|---|---|---|---|
| `scrollWidth` | 3 | `_app/immutable/chunks/ci5FwYEI.js:61` — a Svelte effect doing `el.scrollLeft = el.scrollWidth` on a horizontal scroller | **162 ms/call** |
| `clientWidth` | 1 | `ci5FwYEI.js:126` — restoring the chat-controls pane width from `localStorage.chatControlsSize` | **273 ms/call** |
| `clientWidth`/`getBoundingClientRect` | ~380 | ProseMirror `updateState`/`scrollToSelection` (`BEpREC_0.js`) | **0.0 ms** |

Note the third row, because it corrected a wrong first hypothesis: the editor
performs by far the most reads and costs **nothing**, because by then layout is
already clean. Volume is not the problem; *when* a read happens is.

**And the amplifier is the app's own stylesheets, not its size.** Same page,
same 3,863-node DOM, WebKit, stylesheets toggled off and back on in place:

| | 18 sheets / 1,438 rules | `styleSheets[i].disabled = true` |
|---|---|---|
| forced full layout | **475 ms** | 156 ms |
| `scrollWidth` after a mutation | **181 ms** | **0 ms** |

### 10d. Four falsifications, all recorded because three of them changed the answer

1. **"Your MutationObserver is the cost."** Ruled out: the `observe:"none"`
    arm settles on an element-count poll instead, and reads blocking p50 2,307
    against 2,260 with the observer. No difference.
2. **"Your reflow wrappers are the cost."** Ruled out by running the identical
    wrappers on Chromium: **same page, same call counts** (`scrollWidth` 64 in
    both arms, `scrollTo` 48 in both) and **28 ms of wrapper-measured time
    against WebKit's 1,825 ms**. A wrapper cannot be 65x more expensive in one
    engine. Corroborated by the un-wrapped root-font-size probe, which
    reproduces the ratio without touching any accessor.
3. **"It is DOM size — WebKit's layout is just slow."** ⛔ **FALSE, and this is
    the one worth keeping.** On a flat `rem`-based synthetic document WebKit's
    full layout is only **1.0x-2.5x** Chromium's — 1,004 nodes: 33 ms vs
    13 ms; 8,004 nodes of long text: **282 ms vs 276 ms, i.e. 1.02x** — and a
    `scrollWidth`-while-dirty read costs **10 ms vs 3 ms**. At 40 levels of nested flexbox — the real app's depth — WebKit
    costs *less* per node than flat. The engines are broadly comparable at raw
    layout. The 40x is specific to this document's style resolution.
4. **"`contain: layout style` on the message list will scope the invalidation."**
    ⛔ **FALSE — do not ship this.** A 24-run interleaved A/B (control and
    contained alternating, so both arms see the same host load) moved nothing:
    blocking 2,348 → 2,288, content-settle 1,843 → 2,013, forced layout
    324 → 333. There is no cheap CSS lever here.

⚠ **And one instrument bug, caught in flight, recorded so it is not
reintroduced.** The layout microbenchmark originally alternated the root
font-size between two fixed values and took a median; from the third rep on it
was assigning a string the style already held, which invalidates nothing. It
reported **0.0 ms in Chromium and 58 ms in WebKit** and both were fiction. The
fixture also used absolute lengths, so a root font-size change legitimately
invalidated nothing in Chromium — the real app is Tailwind, i.e. `rem`
everywhere. Both are fixed in `spa_nav_layout_bench.js` with the reasoning in
the comment. **A "no measurable difference" result is exactly what a broken
invalidation looks like; check that the probe still dirties what it claims to.**

### 10e. Ranked, and what we can actually do

| # | term | measured share of the 2.1 s | ours? |
|---|---|---|---|
| 1 | ~4 forced full layouts, at 180-500 ms each on this document | ~66% of blocking | **no** — app schedules them, engine prices them |
| 2 | the rest of the app's render JS (Svelte + markdown + sanitize) | ~34% of blocking | **no** |
| 3 | route change + chat payload fetch | ~300 ms, same on both engines | **no** |
| 4 | ychrome / under-glass hosting | not separable from its own probe cost | ~0 in kind |

**The honest answer is that this is open-webui's JS meeting WebKitGTK's style
resolution, and there is nothing in yggterm to fix.** That is a result, not a
failure: it stops us optimising the wrong layer, and it is the reason this
section leads with the plain-engine arm rather than with ychrome.

What is left open, and exactly what would settle it: **why does style
resolution against 1,438 rules cost WebKit ~180 ms on 3,863 elements when the
same engine resolves a rule-free document of twice the size in ~10 ms?** The
stylesheet ablation proves the sheets are the amplifier but not which
construct in them is. Settling it needs either a WebKit build with
`--enable-developer-mode` style-recalc counters, or a bisect of the served CSS
(disable sheets one at a time, then rule ranges within the guilty one) using
the same `spa_nav_probe.js` ablation harness. Neither is a yggterm change; both
would produce an upstream bug report worth filing — against open-webui for
reading `scrollWidth` inside an effect, and against WebKitGTK for the style
cost.

⛔ **Do not reach for a per-site CSS shim in ychrome site-lore on the strength
of this section.** The one containment shim that looked obvious was measured
and does nothing (falsification 4). Any future shim needs the same A/B before
it ships.

## 11. "Same pages on reload look like they are reloading afresh": a ten-minute timer, not a cache (2026-08-01)

**The report:** *"ychrome does not cache so well as chromium based browser. Same
pages on reload look like they are reloading afresh. We should use aggressive
caching like chrome with a local CDN override."*

Same shape as §9: the observation is right and the named cause is wrong. §9
settled that the HTTP disk cache works; this section settles what the user is
actually watching rebuild. **It is our own reaper, destroying pages on a wall
clock while the machine has memory to spare.**

⚠ Do not merge this with §9 or §10. §9's ~650 ms is per-surface WebProcess
startup, §10 is open-webui's forced layouts, and this is a page that no longer
exists being built again from a URL. Different mechanisms, and only this one is
ours to fix.

### 11a. The evidence, from the live host's own trace

Every retained `event-trace.*.jsonl` on jojo, all `category: web_surface`:

| | |
|---|---|
| `native_close` events | 182 |
| ...of them `background_hold_expired` | **134 (74%)** |
| ...of those with `reclaim_pressured: true` | **0** |
| `hold_ms` on every one | 600000 |
| machine state while they fired | 9.2 GB of 15.1 GB available (61%), PSI `full avg60` 0.16% |

And they were not abandoned pages. Pairing each timer-kill with the next
`native_open` on the same `(session_path, tab_id)`:

| | |
|---|---|
| surfaces killed by the timer | 132 |
| **the user came back to** | **109 (83%)** |
| within 30 minutes | 49 |
| median time to return | 2,389 s |

The reaper's own stated purpose is to reclaim memory. On this host it reclaimed
memory nobody was short of, from pages the user was demonstrably still using.

**The falsification that mattered.** `reclaim_pressured: false` on those 134
events only proves the machine was above the 15% floor; it does NOT prove it was
above the 30% line where the fix changes anything. So the honest question is
"would the fix actually have kept those pages?", and the trace can answer it:
403 recorded `MemoryPressureSnapshot`s ride the reveal-latency path in the same
corpus.

| | |
|---|---|
| memory snapshots recorded | 403 |
| ...classified `Comfortable` under the new posture | **403 (100%)** |
| available headroom | min **30.6%**, p05 47.5%, **p50 59.8%**, max 75.0% |
| timer-kills with a snapshot within 10 min | 112 of 140 |
| ...where the machine was `Comfortable` | **112 (100%)** |

So on this host every single measured destroy happened on a machine the fix
classifies as comfortable, and none of them would have fired. ⚠ Note the
minimum: **30.6%**, barely above the `Comfortable` line. The threshold is not
generous, and a slightly heavier day puts jojo back on the ten-minute clock by
design.

### 11b. What each of those returns costs

`yggterm-webprobe` on the deterministic fixture (8 bundles, 2.95 MB of real
minified JS), Xvfb on `dev`, median of 3. ⚠ Host carried `loadavg ~21` from
other lanes, so absolutes are inflated; the shape is the result.

| arm | nav→loadStart | loadStart→load | **nav→load** | network bytes | cache bytes written |
|---|---|---|---|---|---|
| cold (first-ever visit) | 725 | 264 | **989** | 2,947,452 | 5,898,384 |
| warm (new process, warm cache) | 687 | 265 | **949** | **0** | **0** |
| **second surface in a LIVE process** | **222** | 256 | **496** | **0** | – |

Two readings, and the second is the one this section is about:

1. **The cache is not the deficit, again.** The warm arm pulls **zero bytes** off
   the network and writes zero new records: the entire 2.95 MB comes off disk.
   Cold→warm buys 40 ms here only because the fixture is on loopback, where
   bytes are free; the point is that the bytes ARE being served from cache.
2. **Rebuilding a destroyed surface inside a live GUI costs ~496 ms on a trivial
   page with everything cached**, 222 ms of which is spent waiting for the
   WebProcess before the navigation even starts. That is the floor. A real app
   adds its own render bill on top — §9e measured khanacademy.org's page half at
   1,638 ms warm — plus scroll position, form state and SPA state, which no cache
   restores at all.

⚠ **A number here disagrees with §9c and is recorded rather than reconciled.**
§9c reports a second surface in a live process still waiting ~710–790 ms before
`load` starts; the same flag on the same tool measured **222 ms** here. Different
day, different host load. Neither figure has been re-run against the other, so
treat the per-surface wait as "somewhere between 200 and 800 ms" until someone
holds the load constant and settles it.

### 11c. The root cause

> **`web_surface_background_hold_ms_for` returned a ten-minute deadline that
> consulted no memory reading at all.** `MemoryPressureSnapshot::reclaim_pressured`
> decided whether to DETACH a backgrounded surface and how short the hold should
> be under pressure — but with no pressure the destroy still ran, off a wall
> clock, on a machine with 61% of its RAM free.

The rule it was breaking was already written down in this codebase, on
`RECLAIM_AVAILABLE_FLOOR_PCT`: *"Above it, reclaiming a user's pages costs them
work to buy memory nobody is asking for."* The pressure path obeyed it. The
clock never read it.

### 11d. The fix, and the ceiling it does not remove

`ReclaimPosture` (`terminal_observe.rs`) is now the ONE answer to "how badly does
this machine need a page's memory back", and `reclaim_pressured` is its top band:

| posture | reading | destroy clock |
|---|---|---|
| `Pressured` | available < 15%, or PSI `full avg60` ≥ 10% while available < 30% | 5 s (unchanged) |
| `Tight` | available < 30% | the configured hold, default 600 s (unchanged) |
| `Comfortable` | available ≥ 30% | **none** |

- **An explicit knob always wins**, in every posture, including
  `background_hold_secs: 0`. Only an ABSENT knob lets the posture decide, which
  is why `web_surface_config_hold_ms` now returns `Option` instead of folding the
  default in: "they chose ten minutes" and "they chose nothing" had become the
  same value, and they are the difference the fix turns on.
- **`Comfortable` starts at `RECLAIM_PSI_HEADROOM_CEILING_PCT`, not a new
  number.** 30% is already this codebase's line for "a memory stall here is not
  our problem"; it is therefore also the line for "our backgrounded pages are not
  what this machine is short of".
- **The memory ceiling is re-imposed by the posture, with hysteresis for free.**
  As live surfaces accumulate, headroom falls, the posture goes `Tight` and the
  600 s clock resumes; below the floor it collapses to 5 s. Memory decides, not a
  clock — which is the mechanism Chromium's tab discarding uses, and the reason
  its background tabs come back instantly.
- **Nothing about backgrounding changed.** The surface is still demoted and
  throttled, so the page is `document.hidden`, its rAF is paused and its timers
  are throttled. §5 constraint 1 holds: the axis is PAINT, not existence. Only
  the destroy is gone.
- The trace now carries `memory_posture` on `native_close` and `native_stash`, so
  a pass that reaped nothing SAYS why. A destroy recorded as `comfortable` is
  this bug, back.

**The measured cost.** One live `WebKitWebProcess` on jojo holds **674 MB PSS**;
§8 records a single Meta surface at ~1.3 GB. So keeping background surfaces is
expensive, and the honest bound is arithmetic: jojo has 9.2 GB available against
a `Comfortable`→`Tight` boundary at 30% of 15.1 GB = 4.53 GB, i.e. **~4.7 GB of
slack, roughly 7 to 15 additional live surfaces**, before the old ten-minute
behaviour resumes by itself.
⚠ **Per-surface CPU is not measurable on this substrate** (WS1's correction:
contexts are shared, and webkit2gtk exposes no web-process id), so "a kept
surface is idle" rests on the throttle mechanism being unchanged by this patch,
not on a per-surface reading. Do not quote one.

### 11e. The local CDN override: measured, and the recommendation is DON'T

The user asked for "aggressive caching like chrome with a local CDN override".
The first half is already true. The second was costed rather than argued.

Across all 15 profiles on jojo that have a cache, hashing every blob's CONTENTS
(the file NAMES cannot answer this — each profile has its own random 8-byte
`salt`, so byte-identical bodies get different filenames):

| | |
|---|---|
| cached blob bytes | 276.0 MB |
| distinct contents | 1,955 |
| contents present in more than one profile | 173 |
| **redundant bytes** | **29.6 MB (10.7%)** |

So a shared local mirror would save ~10.7% of cache DISK, and would save LATENCY
only on the first visit to a given asset in a given profile — after that the
profile's own cache already serves it at zero network bytes (§11b). Against that:

- **It is a cross-profile timing side channel**, and profiles exist here
  precisely to separate identities. Chromium has spent since 2020 moving the
  other way, partitioning its HTTP cache by top-level site to close exactly this
  class of probe. "Cache like Chrome" and "share one cache across identities" are
  opposite requests.
- Intercepting CDN assets over HTTPS means terminating TLS with our own
  certificate for hosts we do not own.
- It puts a new component in the path of every request, with its own failure
  mode, to re-solve a problem `NetworkCache` is already solving.

**Recommendation: do not build it.** If the 29.6 MB ever matters it is a disk
problem with a disk answer (filesystem-level dedupe / reflinks), which costs no
protocol change and opens no side channel.

### 11f. What is NOT proven: the fix has never been observed running

Honest status, because the deploy protocol says to say this rather than "shipped".

The GUI binary on the live host is a SHARED, contended resource, and during this
lane's window `~/.local/bin/yggterm` changed identity four times in twenty-three
minutes as other lanes deployed over it (md5 `29df3685` mine 13:51 → `19681422`
13:53 → `29df3685` mine ~14:04 → `9cccf654` 14:14). In the one window where the
running GUI was confirmed by md5 to be this lane's build (pid 1943396), a
backgrounded surface was watched from 100 s to **587,471 ms of its 600,000 ms
hold** — and then that GUI generation was replaced at 14:14:28 by another lane's
restart, 13 seconds short of the crossing that would have settled it.

So: suggestive, not proof. What IS proven is everything except the last step —
the bug (§11a), the cost (§11b), the posture arithmetic against the live host's
own 403 memory snapshots (§11a), and 1,718 green tests including a behavioural
lock built from those numbers.

⚠ **A lane build must not be forced onto the live host to close this.** Doing so
un-ships whatever else landed on `main` in the meantime, which is how the binary
kept flipping in the first place. The verification belongs to the deploy that
carries this lane's merge to `main`, and the check takes one command: watch a
backgrounded surface past 600 s in `server app state`'s `web_surface_tabs`, and
confirm `memory_posture: "comfortable"` on its `native_stash` trace line.

⚠ **The event trace is not a reliable instrument for this on jojo.** The GUI
generation running at 14:04 wrote no `web_surface` events to any readable
`event-trace.*.jsonl` at all, so "no `memory_posture` in the trace" was a blind
instrument, not a negative result. `server app state` → `web_surface_tabs` is
the reliable read: it reports `state`, `stashed_for_ms` and `reaps_in_window`
per surface.
⚠ And `stashed_for_ms: null` is AMBIGUOUS — it means "not stashed", which covers
both "destroyed" and "the user just revealed it". Disambiguate on the ROW's
existence and on the GUI's pid, or a routine GUI restart reads exactly like a
reap. It did once here.
⚠⚠ **Compare `stashed_for_ms` NUMERICALLY, never with a shell glob.** A watcher
written here matched `*stashed_for_ms=6[5-9]*` intending "650,000 ms or more"
and fired on **65,846 ms** — 65 seconds — then printed `PASSED 600s STILL
ALIVE`. It was a wrong answer to the exact question this section exists to
settle, and it would have read as confirmation of the fix. `[ "$v" -gt 640000 ]`
is the test. Three instrument ambiguities in one measurement is the standing
warning: on this host, verify the verifier before quoting it.

### 11g. Still open after this section

- **The per-surface WebProcess wait**, 222 vs 790 ms unreconciled (§11b). It is
  the largest single term in a rebuild and nobody has held host load constant
  across the two measurements.
- **PSON / `WebProcessCache`** remains the only route to removing that wait, and
  §9f's reasons for leaving it alone are unchanged.
- **`Comfortable` is a headroom reading, and jojo swaps.** `MemAvailable` stays
  healthy while the kernel swaps other things out, so a host that is thrashing
  for reasons of its own can still read `Comfortable`. The PSI route is vetoed by
  headroom by design (see `RECLAIM_PSI_HEADROOM_CEILING_PCT`), so this is
  deliberate, not an oversight — but it is the assumption to check first if
  kept-alive surfaces are ever implicated in a freeze.
