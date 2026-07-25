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

1. **The floor has a one-line fix that no workstream here proposed.** Setting
   `YGGTERM_ENABLE_WEBKIT_COMPOSITING=1` unlocks all three gates at once (launcher stops
   exporting `WEBKIT_DISABLE_COMPOSITING_MODE=1`; the GL net is skipped; under-glass arms,
   clearing the SHM force). Do this before costing WS2, which is a large architectural
   move justified partly by a number this changes.
2. ⚠⚠ **The three settings are ONE decision — do not split them.** Hardware GL + SHM
   measured 15.82 s (no better than software); software GL + DMABUF measured **34.14 s,
   the worst of the four**. The guard's logic was right; only its premise was wrong.
3. **The real fix is to stop hard-coding the premise.** Probe the Wayland/Surfaceless EGL
   platform at startup and choose from what the host reports, rather than defaulting to the
   slowest configuration and requiring an opt-out that nobody knew to set. `render_probe`
   is the natural owner, and it closes the same instrument gap WS1 exists for.

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

jojo GUI pid 776144, 15-second window, `cargo run -p yggterm-core --example render_top`:

| role | procs | cores | PSS |
|---|---|---|---|
| `gui` | 1 | 0.220 | 568 MB |
| `web_content` | 3 | 0.272 | 714 MB |
| `web_network` | 3 | 0.006 | 82 MB |
| **total** | **7** | **0.498** | **1364 MB** |

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

Deliberate limit: **per-process, never per-surface.** WebKitGTK runs one web process per
profile serving every surface on it, so a per-surface CPU number would be a fabrication.
Surface counts ride along as caller-supplied context.

Remaining in WS1:

- **Wire continuous sampling.** Today it is a one-shot example. It needs a tick in the
  GUI (the allocator-trim chore near `shell.rs:23753` is the pattern to copy) passing
  live/stashed surface counts and window visibility as context.
- **`server render-top`**, promoting `examples/render_top.rs` into a real command.
- ~~**`server perf-incidents`**, a reader for the 183 records already on disk.~~
  ✅ **SHIPPED 2026-07-25** and run live — see the update box in §3c. Groups by
  trigger, ranked by count, `--list`/`--json` for raw records.
- **Collapse the duplicate `/proc` parser.** `shell.rs:37685
  current_process_memory_sample` / `process_memory_sample_from_smaps_rollup` parses
  `smaps_rollup` for self only; `render_probe` parses it per pid. Single source of truth
  says one owner: shell.rs should call into `render_probe`.

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
- `copy_scan`: incremental off mtime, skip unchanged stores, back off when nothing
  changed.
- `daemon/persist`: dirty-flag or debounce; the state is re-serialized far more often
  than it changes.
- `snapshot_response`: memoize by generation.
- `copy_generation/title` + `summary`: cache by content hash, never regenerate for an
  unchanged transcript.
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
# render side, on the GUI host
GUI=$(pgrep -x yggterm | head -1); render_top "$GUI" 15000
# Rust side
yggterm-headless server perf-summary --category render
yggterm-headless server perf-summary
```

A win is a moved number in the table above, on the same host, over a comparable window,
with the app doing comparable work. Anything else is an anecdote.
