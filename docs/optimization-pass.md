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
- **Lane A is a timeboxed spike.** True WPE headless (`WPEDisplayHeadless`), no display
  server at all. `libwpewebkit-2.0-dev` 2.52.4-1 is available in sid and **installed
  nowhere**, and there is no mature Rust binding. Verify reachability from Rust before
  committing to it.

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

## 6. Small bug to fix in the same pass

An agent launching a session without revealing it still steals the user's session
**focus**. `terminal new --no-activate` correctly does not switch the viewport, but the
active-session follows the new session anyway. Start at the `--no-activate` spawn path
and whatever sets active-session after a birth.

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
