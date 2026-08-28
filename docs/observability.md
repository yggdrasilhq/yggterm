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
| What must a NON-Rust layer do to emit onto this plane? | `docs/spec-trace-plane-contract.md` |

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
(`ytrace query|top|flame|timeseries|tail|incidents|health`) or Python `notebooks/ytrace_helpers.py`,
which resolve it the one way the writer does.

---

## 2. Probe inventory

### 2.1 Declared

`perf::ytrace_provider()` (`crates/yggterm-core/src/perf.rs:14`) pre-registers 37 probes so that
sampling policy and clock are attached before the first emission:

| Group | Probes | Clock | Sampling |
|---|---|---|---|
| daemon hot paths | `daemon_request/{status,ping,terminal_read,terminal_write,terminal_snapshot,working_flags}` | wall | `floor 8 ms` + `1:50` |
| render cost | `render/{gui,web_content}` | **cpu** | always |
| render faults | `render/storm`, `ui/{render_fail_pattern,app_render_rate}` | wall | always |
| attach faults | `terminal_mount/retained_rehydrate_{skipped_live_connected,skipped_pre_resize,skipped_inactive,begin,empty,retry_scheduled,refused}` | wall | always |
| title lifecycle | `title/{untitled_session,resolve_attempt,llm_rescue,cli_store_hit,generation}` | wall | always |
| input latency | `input/{keystroke,pty,render,loop_block,unconsumed}` | wall | always |
| per-CLI wiring | `cli/{agy_title,agy_resume,codex_geometry,codex_resume,persisted_identity}` | wall | always |
| web surface & sidebar | `web_surface/{liveness,lifecycle}`, `sidebar/liveness` | wall | always |

`resource_governor.rs:59` registers three more from the daemon: `row_resource/{hot,oom}` (cpu) and
`daemon/resource_governor` (wall).

Every `render/*` sample carries both web-surface planes. The reconciler plane is
`web_surface_views_{visible,stashed}`. The engine readback plane is
`web_surface_engines_{present,missing,hidden}`,
`web_surface_engine_widgets_{visible,mapped}` and
`web_surface_web_processes_responsive`. The two explicit disagreement gauges are
`web_surface_engine_visibility_mismatches` and
`web_surface_widget_visibility_mismatches`. A stashed count is a request; only the
engine fields establish whether WebKit actually received it. The same per-surface
facts are exposed by `server app state` under `web_surface_tabs.rows[]`.

### 2.1a The universal CLI plane

`category:"cli"` is the cross-CLI lifecycle, not the legacy five registered
fault probes in the table above. Read it with:

```bash
ytrace tail --category cli --since 30m --json
```

Its shared grammar is `birth` → `launch` → `identity_poll` → `title` /
`title_sweep` → `projection` / `projection_sweep` → `restore`.

* `identity_poll` explains a self-minted-id join without exposing cwd or title:
  `target_rows`, `identities_seen`, `identities_with_birth_alias`,
  `exact_alias_candidates`, `cwd_candidates`, `rebinds`, and
  `newly_exhausted`. It is emitted once per registered CLI kind considered in
  the tick, even though each machine is queried only once. An identity count
  above zero with both candidate counts at zero means discovery worked and the
  join failed. `local_identity_bind` is the owning-daemon edge that confirms a
  late CLI-minted id was persisted; it records kind and id origin, never cwd,
  transcript path, or title.
* `title` / `title_sweep` expose each registry CLI's local- and remote-store
  outcome without title text. A remote `no_title_in_store` carries
  `retry:"unconfirmed_until_store_title"`: a negative lookup is eventual
  state, not confirmation, and remains eligible for the bounded idle retry.
  `skipped_title_settled` is valid only after a positive store lookup for the
  row's current logical id (or when the store already agrees with the row).
  `title_apply_refused` identifies a proposal that reached the writer but did
  not land, carrying only `row_resolved` and `owner_titled`; it exists to split
  an intentional provenance refusal from an alias-resolution defect.
  Remote misses also carry `candidate_count` and `probe_line_count`: the first
  distinguishes an empty store from candidates rejected by title policy, and
  neither field contains transcript or title content.
* `attachment_sweep` is the cross-CLI liveness census. Every registered kind is
  present even at zero, with `running`, `preserved`, `exited_runtime`,
  `missing_runtime`, `unbound_presence`, and `not_expected` counts. Projected
  remote rows are not expected on this daemon. A legacy/birth `local://` row
  with no owner is reported separately as unbound presence: it is a visible
  row-identity anomaly, but does not prove that this daemon dropped an owned
  PTY. `attachment` records the rare bounded-restart refusal that deliberately
  keeps the old runtime seated. `orphan_runtime_row_recovered` is the reverse
  edge: a PTY descriptor crossed a handoff but no managed row represented its
  key, so the successor restored addressable presence without launching a
  process. It carries only CLI kind, runtime scheme, and identity origin.
* `startpage_observers/faithful_read` proves the Startpage CLI's optional GUI
  witnesses were read in-process. It reports browser-row count, daemon-snapshot
  and app-state availability, elapsed milliseconds, and the bounded
  app-control timeout. It never contains row identity or content. Absence means
  the store-only fallback should answer; it must not trigger daemon startup.
* `version_probe` proves managed metadata work is bounded (`completed`,
  `timed_out`, or `failed`, with elapsed and ceiling). `runtime_conflict` and
  `resume_refusal` classify CLI-owned resume errors; they never carry screen
  samples, transcript content, cwd, or launch commands.
* `projection` is the GUI end of the chain. Initial healthy rows stay in the
  aggregate; a bad edge and its recovery emit once per row/presence. It reports
  `title_quality`, `kind_source`, `icon_kind`, `expected_icon_kind`, and
  `icon_matches_kind`, never the rendered title text. Presence is taken from
  the concrete row occurrence inside/outside the Live Sessions region, not from
  its session path: the required live-rail and cwd-tree copies share identity.
* `projection_sweep` names every `AGENT_CLIS` slug including zero rows. Its
  unchanged heartbeat is twenty minutes because the all-CLI payload is larger;
  any count change emits immediately. The byte-budget lock in `cli_plane.rs`
  keeps the whole CLI plane below 1% of the measured live trace rate.

The on-demand row witness carries the same final facts under `server app rows`:
`session_kind`, `session_kind_source`, `title_quality`,
`expected_icon_kind`, and `icon_matches_session_kind`. These fields describe
the projection after live-title enrichment. They do not replace the store or
daemon as title/identity authority.

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

### 2.3 The foreign layers — probes that are not emitted from Rust

Two layers inside the webview emit onto the trace plane through the bridge in
`docs/spec-trace-plane-contract.md` (which owns the grammar; this table owns what exists). They
are distinguished by the record's `layer` field, **not** by `component` — both write
`component: "ui"`, which is exactly why the extra tag had to exist.

| `layer` | Probe | Kind | What it measures |
|---|---|---|---|
| `xterm` | `xterm_write/enqueue_window` | window | write-queue arrivals: `count`, `chars`, `max_depth`, `max_backlog_age_ms` |
| `xterm` | `xterm_write/enqueue_backlog` | point | one arrival that crossed the depth or backlog-age floor — the outlier the window would otherwise average away |
| `xterm` | `xterm_write/flush` | span (wall) | one bridge flush: latency, chars written, chars still pending, paint-repair reason. ⚠ **Rationed** — kept for a flush at/over 8 ms, a repaired flush, or any flush inside the 4 s window a screen boundary arms; the rest fold into `flush_window` |
| `xterm` | `xterm_write/flush_window` | window | the flushes the line above did not keep: `count`, `total_ms`, `max_ms`, `chars` |
| `xterm` | `xterm_render/frame_window` | window | painted frames: `count`, `max_rows_painted`, `full_canvas_frames`, `max_gap_ms` |
| `xterm` | `xterm_render/frame_gap` | point | a gap between painted frames past the stutter floor |
| `xterm` | `xterm_screen/reset` | point | the canvas was wiped, with the reason |
| `xterm` | `xterm_screen/{replay_reset,replay_reseed}` | point | the wipe-and-refill pair of a retained replay |
| `xterm` | `xterm_attach/stream_sample` | point | the CONTROL structure of the bytes the canvas was handed, tagged `reseed` / `live_stream` / `restore`, with an SGR census |
| `xterm` | `xterm_paint/mount_open` | point | `term.open()` returned and the surface is BLANK — the anchor the mount chain is measured from, carrying `script_to_host_ms`, `host_to_open_ms`, the grid, and whether the open needed a retry |
| `xterm` | `xterm_paint/first_frame` | span (wall) | the empty surface to the first glyphs on it. `duration_ms` is open→frame; the payload splits it into `open_to_write_ms`, `write_to_parsed_ms`, `parsed_to_frame_ms` |
| `xterm` | `xterm_paint/settle` | window | did this mount actually PAINT: `rows_covered` against `rows_with_content`, `rows_content_unpainted`, `complete`, `painted`, `blank_frames_before_write`, plus `overshoot_ms` |
| `dioxus` | `dioxus_render/component_window` | window | per-component render cost (`renders`, `total_ms`, `max_ms`, `mean_ms`, hottest first) **and** the invalidation causes (`causes[]`, `root_renders`, `renders_unattributed`) |
| `rust` | `session/activation` | point | **the active row changed, and WHY**: `from`, `to`, `origin`, `origin_site`, `user_gesture`, `previous_origin`, `ms_since_previous_activation`, `redundant_since_previous` |
| `rust` | `trace_bridge/foreign_batch_faults` | point | what the boundary refused or repaired, and how far behind the emitter was running |

### 2.3b The same-frame paint record — the one instrument that is NOT a probe

⛔⛔ **WHY IT EXISTS: THE TWO SAMPLES WERE NEVER ONE MOMENT.** A faithful screenshot costs
**~6.8 s**; a `terminal read-buffer` costs **~116 ms**. On a live agent row the two can never be
sequenced into one frame, so every *"the buffer says X but the screen shows Y"* reading ever taken
on a busy row compared two moments about seven seconds apart. One such pair returned
`Nesting… 1m42s` from the buffer beside `Whirring… 29s` from the pixels — **two different turns of
the same agent, presented as a contradiction.** Some unknown share of every historical divergence
in this class was TIME, not a paint fault, and there was no way to tell which.

**What it is.** `server app screenshot <out.png>` now writes `<out.png>.paint-frame.json` beside
the PNG whenever the faithful path ran. The composite script reads the drawn host's xterm buffer in
the **same synchronous JS turn** as the drawImage loop and `toDataURL`. xterm parses PTY bytes on
its own task queue, which cannot interleave with synchronous script, so the text in the sidecar is
exactly the text those pixels were composited from — **the two are one moment by construction, not
by luck.** The screenshot response carries a `paint_frame` summary pointing at it.

    scripts/paint-diff.py <out.png>              # per-cell verdicts
    scripts/paint-diff.py <out.png> --all-rows   # every row, not just disagreements
    scripts/paint-diff-selftest.py               # planted-fault proof of the analysis half

⭐ **THE UNIT IS A CELL, NOT A LINE.** The buffer side is an ink mask built from the cells
themselves, never from `translateToString` — that trims trailing runs and gives a wide glyph one
char across two cells, so a column index taken from the string does not address the cell the
renderer drew. Verdicts per row: `MISSING` (buffer holds glyphs, pixels hold none), `PARTIAL`
(pixels hold far fewer), `GHOST` (pixels hold glyphs the buffer does not), `ok`, `blank`.

⛔ **INK IS WITHIN-CELL CONTRAST, NOT DARKNESS, and that is not a detail.** A TUI's chrome — status
bars, footers, selected rows — is mostly cells that are STYLED and EMPTY. A test asking "does this
differ from the terminal background" calls every one of them a ghost and reports a screen full of
faults on a perfectly painted terminal. A cell holding a glyph has internal contrast; a cell
holding a flat colour has none, whatever colour it is.

⛔⛔ **`atlas_clears` COUNTS OUR CALLS, NOT THE ADDON'S CLEARS — read `page0v` beside it.**
`forced_atlas_clear_count` is incremented by our own refresh funnel at the moment it calls the
addon. `TextureAtlas.clearTexture()` then opens with a guard — it does nothing at all while the
first page is still at its origin — so a run can report N atlas clears having wiped nothing. An
experiment built on that number would "refute" a cause it never applied, which is the same failure
shape as a harness reporting clean results it never rendered. The frame now also carries
`atlas_page0_row_x/y` — the fill cursor, which is what the guard reads — and `atlas_page0_version`.

⚠ **Read those two correctly, because the obvious reading of the version is wrong.**
`Page.version` is bumped by `Page.clear()` **and by every glyph rasterised into the page**, so it
is a "this page changed" counter, not a clear counter: one run here moved it 1,088 → 8,573 across
nine atlas clears. The evidence that a clear actually RAN is the fill cursor **moving backwards** —
`page0row` went `8,102` → `8,75`, and a page that is only being filled can never go down.
**A refutation of anything atlas-shaped is only valid if the fill cursor was seen to reset.**

⭐ **AND THE WARMED RANGE DECIDES WHETHER A FIXTURE CAN TEST THE ATLAS AT ALL.** `_doWarmUp()`
rasterises codepoints **33..125 at DEFAULT fg/bg/ext** in a fixed order, and `clearTexture()` sets
`_didWarmUp = false` so it runs again after every clear — returning default-coloured ASCII to the
same slots. A co-owner's stale glyph coordinates for that range therefore stay valid **by
construction**. Any fixture drawn in plain default-coloured ASCII is testing the one case where
atlas sharing is provably harmless; a fixture meant to exercise it has to draw outside the warmed
set — non-ASCII, or any coloured or bold cell, since fg/bg/ext are part of the cache key.

⚠ **What it cannot see, so that nothing is built on it:** it does not read the glyphs, so a cell
painted with the WRONG character reads `ok` — it answers "was something drawn here", not "was it
right". A native web surface draws above all DOM and is absent from the frame. And
`capture_faithful: false` makes the whole frame canvas-blind, where every row reads `MISSING` and
none of it means anything.

⭐ **`png_space` is load-bearing.** The two faithful backends write different rectangles —
`window` (the merged chrome snapshot, scale by `png_width / win_w`) and `frame` (the terminal-only
composite, where the PNG *is* the frame rect). A row→pixel band computed against the wrong one
lands on the wrong row and every verdict after it is confidently wrong.

⭐ **Reading `dioxus_render/component_window` — three numbers, in this order.**

1. `root_renders` is the denominator. **A component whose `renders` equals it was memoized by
   nothing** and re-rendered on every pass — that is "which component invalidates", answered
   directly.
2. `renders_unattributed` counts root renders with **no state write in front of them**: a forced
   wake, or a second pass over one change. That is render amplification measured rather than
   inferred, and it is the number a coalescing fix has to move.
3. `causes[]` names who wrote the signal, ranked by **`renders_preceded`** — how many root renders
   the site actually caused — not by `writes`. ⛔ The pair is the instrument and the totals alone
   are a trap: ten writes before one render and one write before each of ten renders have the same
   `writes` and opposite costs, so ranking by `writes` puts the harmless site above the expensive
   one. A high-`writes`, low-`renders_preceded` site is chatty and cheap; the reverse is the storm.

⚠ **`full_canvas_frames` is the one to read first on a corruption report.** The reported symptom is
a whole viewport of unreadable output, not a damaged line, so a session repainting everything it
owns over and over is the shape being looked for — and the old render counter, which counted frames
without their row range, could not tell that apart from a healthy busy terminal.

⭐ **`xterm_attach/stream_sample` carries a census, not a screen — read the census first.** Its
`sgr_colour` count answers the ghost-frame question on its own: zero on a `reseed` sample means the
colour was already gone before the canvas saw the bytes, non-zero means the bytes carried it and the
fault is in applying the attributes. ⛔ The `sample` field is **redacted**: every run of printable
text is replaced by its length, CSI sequences are verbatim, and an OSC is reduced to its opcode and
length. Do not read it as a transcript — it is deliberately not one, and the reason is in
`docs/spec-trace-plane-contract.md` §8.

⭐⭐ **`session/activation` answers the other half of the same question: did the person switch, or did the
app?** Until it existed, a hand clicking between rows and the app moving on its own produced **exactly
the same trace** — which is why the mount-churn entry can prove the re-mounting half and not the
switching half. `user_gesture` is the field; the origin vocabulary behind it is fixed and tested:

| `origin` | means |
|---|---|
| `user_gesture` | a hand — a sidebar row, a start-page card, a key, a notification card |
| `app_control` | the control plane asked. ⚠ **NOT a gesture**: an orchestrator opening a row while somebody reads is the SYMPTOM this entry is about, not the control |
| `history` | back/forward through the viewport history |
| `launch` · `restore` · `recovery` | a session starting, a startup/snapshot restore, a repair after the active row vanished |
| `internal` | anything else — and `origin_site` still names the line, so it is never a dead end |

⛔ **The record exists because the FIELD has exactly one writer, not because somebody remembered to
log.** An origin stamped at the call sites an author thought of would leave an unexplained switch and
an *uninstrumented* one looking identical — the same blind spot restated one layer up. A source test
(`the_active_session_path_has_exactly_one_writer`) fails on a second assignment anywhere in the
server.

⚠ **Naming the row that is already active is NOT a switch and emits nothing.** Those are counted per
origin and carried on the next real switch as `redundant_since_previous`, because an idempotent click
and a repair loop both look like that — and `request_terminal_launch_for_active` alone fired 13 times
in 1.3 minutes on the GUI host, which as records would be pure restatement that nothing moved.

⛔ **Absence of the probe is not absence of a switch on an older build.** A daemon or GUI predating
this emits nothing here, and a window with no `session/activation` at all means "this build cannot
answer", never "nothing switched". Check that the category exists before reading a quiet window.

⛔⛔ **THREE WAYS TO MISREAD IT, all three measured live on 2026-08-21 rather than imagined.**

1. **ONE SWITCH CAN APPEAR TWICE, FROM TWO PROCESSES.** The GUI and the daemon each hold a
   `YggtermServer`, so each records its own view. A single sidebar click produced
   `pid=<gui> user_gesture/sidebar_row_select` **and** `pid=<daemon> app_control/attach_seed_remote_snapshot`
   — the same `from → to`, two origins, and the second one reads like the app switching by itself.
   ⇒ **Read activation from the GUI's pid** (`server app clients` names it). The daemon's record
   describes its own bookkeeping, not what moved on screen.
2. **`user_gesture` NAMES THE CODE PATH, NOT THE HUMAN.** `server app pointer click` and
   `server app grid click` drive the GUI's real DOM/Dioxus click path on purpose — that is what
   they are for — so an agent clicking a row lands on `sidebar_row_select` and records
   `user_gesture: true`. This is not fixable by guessing at the origin, and guessing is worse than
   the ambiguity. ⇒ **The discriminator is on the plane**: an `app_control/request_stage` with
   `command: "pointer"` or `"grid"` immediately precedes such a click. A `user_gesture` with no
   such request in front of it is a hand.
3. **EVERY SHORT-LIVED CLI PROCESS EMITS ONE.** `yggterm-headless <anything>` builds a
   `YggtermServer` and restores state, which is a real activation in that process — observed as
   `restore/restore_persisted_state` from a pid that lived for one command. Same trap as §4.4c
   for startup events; the cure is the same, filter by the pid you are actually asking about.

⭐ **Cost, measured rather than argued: 0.86% of records and 1.21% of trace BYTES** in a window
dense with switches. It is bounded by how often the active row CHANGES, and a redundant
re-activation costs nothing — which is the whole reason the no-op case is a counter and not a
record.

⭐⭐ **`xterm_paint/*` answers ONE question the rest of this table cannot: did the glyphs arrive.**
⚠ Not because the canvas was uninstrumented — the `xterm_write` and `xterm_render` rows above are
real and this probe is built on them. It is that all of them count EVENTS in a running terminal,
and every native probe stops at the bridge, so "the mount began" and "the mount painted" were the
same event to all of them — while a mount **begins with an empty surface**. That is why a ghost
frame and a broken TUI paint could only ever be judged from a photograph.

⛔⛔ **A FRAME COUNT IS NOT A PAINT, and this is the misreading the category exists to prevent.**
The renderer repaints only the rows it marked dirty, so `frames > 0` says nothing about how much of
the viewport was covered — a mount that painted two rows and stopped has frames, a render window,
and a perfectly healthy `frame_gap` profile. `settle` answers the question the eye asks instead:

* `rows_with_content` — rows the terminal HOLDS text on, read from the buffer.
* `rows_covered` — rows any frame since the mount has painted, unioned over the frame ranges.
* `rows_content_unpainted` — the difference, and **the field to read first**. Positive means the
  screen is showing less than the session contains.

The test is only sound because it is scoped to a **mount**: the surface started blank, so every row
holding text must be painted at least once. It is not a claim about steady state, and it is not
computed there.

⛔⛔ **`painted` means "a frame landed AFTER bytes reached the canvas", not "a frame happened" — and
the first cut of this probe got that wrong in the direction that flatters it.** Latching the first
frame outright, the very first live mount reported `open → frame` of 218 ms with
`writes_before_frame: 0`: a span that measured **the canvas painting itself empty**, which is the
one event the probe exists to tell apart from glyphs arriving. Frames before the first write are now
counted as `blank_frames_before_write` and never latched — and that count is worth reading on its
own, because *a blank surface repainting is what a ghost frame is*. A mount with frames, no writes
and an empty buffer would otherwise have reported itself `complete` for having faithfully painted
nothing.

⚠ **`rows_with_content: -1` is "the buffer could not be read", which is NOT `0`.** Blind is not
empty — a reader that collapses the two reads an unreadable buffer as a terminal with nothing in
it, i.e. as a perfectly painted one. `verdict: blind` in `scripts/paint-chain.py` is that case kept
separate on purpose.

⚠ **An invisible host is EXPECTED not to paint.** The mount churn re-mounts rows nobody is looking
at and their renderer is idle by design, so those mounts report `painted: false` truthfully; the
`visible` field is what separates a fault from a cost. It is also why no recheck follows an
invisible mount, and why the first record says so in `recheck_scheduled` — a missing record and a
healthy one look identical.

⛔ **`overshoot_ms` is not a paint measurement — it is a UI-thread stall.** The settle timer runs on
the very thread whose stalls are under investigation, so it is never assumed to have fired on time:
it reports the measured window beside its nominal deadline. What it cannot report is a thread that
never comes back at all, and that is what `mount_open` is for — **a mount with no `first_frame`
after it never painted**, and the absence is legible from the native side by joining on `host_id`
alone. Neither half is sufficient; the pair is the instrument.

⭐ **Read the whole chain with `scripts/paint-chain.py`** (`--since 10m`, `--visible-only`,
`--json`). It joins native `terminal_mount/*` to `xterm_paint/*` on `host_id` — which already
encodes the mount epoch as `<host>-m<epoch>`, so no second identity was introduced — and prints one
line per mount with a verdict of `painted` / `partial` / `unpainted` / `blind` / `open`.

⛔ **`xterm_screen/replay_reset` and `replay_reseed` are a PAIR, and the gap between them is the
question.** Any record whose `seq` falls between the two wrote into a screen that was
mid-replacement; a `replay_reset` with no `replay_reseed` after it is a screen that was emptied and
never refilled. Order these on `seq`, never on `ts_ms` — all three routinely share a millisecond,
which is precisely why the emitter numbers its own output.

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

### 4.2b ⛔⛔ THE REFRESH-SKIP PROBE IS THROTTLED BY THE CONDITION IT REPORTS ON

**Measured on the GUI host, 2026-08-22: across the three newest trace files,
`xterm_forced_refresh` and `xterm_forced_refresh_skipped` appear ZERO times, while `xterm_render`
appears 3,872 times in the same files.** The query was checked against that control before the
absence was believed.

**Why zero, and why it is not "nothing happened".** Both probes go out through `emitPerf`, which
lists them as hot high-frequency events and throttles them whenever `recentFrameLikeWrite` is hot.
That flag is armed by any payload containing hide-cursor (`ESC[?25l`), which every TUI emits before
every redraw — so for an agent CLI it is **effectively always hot**. And it is the *same flag* that
suppresses the forced full refresh in the first place.

⇒ **The instrument that would tell you the repair is being suppressed is silenced by the
suppressing condition.** This is the campaign's own law — *an instrument that runs on the thing it
measures reads zero* — in a new place: ask what STOPS being reported when the fault engages.

⚠ **And the four hot events share ONE rate-limit slot** (`lastPerfEventAtMs`, throttle 900–2200 ms).
`xterm_write_flush` fires far more often than the other three, so it consumes the budget and
starves the refresh probes specifically under load — which is precisely when the answer matters.

⛔ **So in the trace, "the repair was suppressed for the whole window" and "no repair was ever
demanded" are the SAME READING: nothing.** Do not conclude either one from a zero here.

⭐ **The way round it, and it needs no change to the throttle:** the host entry keeps monotonic
counters (`forcedRefreshCount`, `forcedRefreshSkippedCount`, `skippedPerfEventCount`) that no
rate-limit can erase. The same-frame paint record (§2.3b) reads them directly, so a captured frame
carries the true balance of repairs against suppressions even when the trace carries none.

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

### 4.4b ⛔ …AND A LARGE `--lines` CAN HAND YOU A COMPLETE-LOOKING WINDOW THAT ENDED HOURS AGO

**This is the other face of §4.4, and it bites the remedy §4.4 prescribes.** Told
to "always pass an explicit `--lines` large enough to cover the window", the
natural move is a big number. But `ytrace` reads across **rotated generation
files** (`event-trace.g<gen>.jsonl`, of which a busy host holds many). Ask for
`--lines 200000` and it can return exactly 200,000 well-formed records **whose
newest timestamp predates the live file's own tail** — a full-looking result from a
window that closed hours ago.

⚠ **The two failure modes point in opposite directions and look identical from the
output.** Too small a `--lines` describes the last few seconds while claiming an
hour; too large can describe an old generation while claiming the present. Neither
warns, both are correctly ordered and correctly filtered.

**The tell:** you got back *exactly* the number you asked for, and its newest
record is old. A correct window almost never returns exactly the cap.

⇒ **Check the newest returned timestamp against the wall clock before trusting any
window**, and for a before/after read prefer the live file directly:

```bash
tail -N ~/.yggterm/ytrace.jsonl          # unambiguous: one file, newest last
tail -1 ~/.yggterm/ytrace.jsonl          # ...and confirm it is current
```

**The instance (2026-08-20).** A post-deploy acceptance read used
`ytrace tail --lines 200000` and returned zero events after the deploy moment,
across every category. The natural reading was "the host went silent" — a serious
claim about a live machine. The live `ytrace.jsonl` was current **to the second**.
The instrument was reading elsewhere, and the size of the request is what sent it
there.

### 4.4c ⛔ A STARTUP EVENT IS EMITTED BY EVERY SHORT-LIVED CLI PROCESS, NOT ONLY BY THE APP

`gui/startup/*` — `main_enter`, `linux_desktop_backend_policy`,
`linux_memory_scope` — is emitted by **any** invocation of the binary, and the
same binary serves the CLI. On a host where agents drive app-control, that is
**207 distinct pids emitting `main_enter` in a five-minute window**. Every one of
them also publishes a desktop-backend policy decision, and a CLI invocation over
ssh has no desktop: `wayland_display_present: False`, `display_present: False`.

**`linux_memory_scope` payload (2026-08-28):** `outcome` (`entered` / `inherited` /
`opted_out` / `not_attempted` / `fallback`), `bounded` (from the cgroup readback,
never from the marker — `max`, empty and unreadable all mean NO bound),
`inherited_unit`, `fallback_reason` (a re-arm failure names the unit the GUI was
rescued from: `re-arm after inheriting unbounded <unit>: …`), and — since the
family shape — `family`: `{armed, children, web_high_bytes, web_swap_max_bytes,
error}` for the GUI's own launch, **null on everything else** (one-shots, an
opted-out or unbounded GUI). Null means not attempted, which is a different
finding from `armed: false`. The runtime sweep emits `gui/memory/family_migration`
per move (`pid`, `comm`, `from`, `to`) — rare, discrete, and the standing proof
that the WebKit children really sit in the `web` child.

⇒ **`tail | grep <name> | tail -1` therefore answers about a random CLI process,
not about the app.** Read after relaunching a GUI, it says the GUI came up blind.

**The near-miss (2026-08-20).** A GUI was relaunched and its arming checked this
way. The last `linux_desktop_backend_policy` record showed no display at all —
which, if reported, would have been a false alarm that the user's desktop had been
replaced by a headless window. Filtered to the launched pid, the same stream said
`policy: kde_wayland_native_default`, `gdk_backend: wayland`,
`wayland_display_present: True`. Both records were true; only one was about the
GUI.

⇒ **Filter every startup event by the pid you are asking about**, and prefer the
one you launched:

```bash
# the pid `server app launch` returned — not "the most recent record"
… | python3 -c '…; [print(d) for d in recs if d["pid"] == LAUNCHED_PID]'
```

⚠ Same family as §4.4/§4.4b: the record is well-formed and true, and the question
you asked is not the question it answers. Here the discriminator is **whose pid**,
not which window.

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

### 4.5 CLI orphan-row repair outcomes

`cli/orphan_runtime_row_recovered` is an edge event after a daemon accepts an
already-live agent PTY. Its `row_repair` field separates a missing row restored
as `recovered` from a narrow wrong-kind placeholder correction reported as
`reclassified`. The event is content-free: kind, runtime scheme, id origin, and
repair outcome are sufficient. It must not contain launch commands, transcript
text, titles, or prompts.

The classifier feeding this repair treats absolute paths as indivisible words;
`/home/user/pi/...` is not evidence for the Pi CLI. A reclassification is permitted
only for a derived update-restart placeholder with the same runtime birth id,
so the event must never be interpreted as authority to rewrite ordinary rows.

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
