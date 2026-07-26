---
name: yggterm-diagnostics
description: The yggterm terminal/xterm diagnostic toolkit — deterministic harnesses (mock-tui + pipeline_integration + xterm-harness), extracted decision specs, and live daemon/app-control probes. Use this BEFORE reasoning from code alone or asking the user to eyeball a symptom. Reach for the deterministic harness first; use live probes (not screenshots) for ground truth; know which instruments lie.
---

# Yggterm Diagnostics

The toolkit for diagnosing terminal/xterm.js behavior — scrollback, reveal/reseed,
follow/scroll, squish, broken-bottom, blink, latency. **Reach for these BEFORE
reasoning from code alone, and before asking the user to observe/judge a symptom.**
The campaign-long lesson (`campaign-xterm-dealbreakers`, `audit-viewport-scroll-control-flow`
in memory): passing-test ≠ live-fixed, and screenshots lie — so reproduce
deterministically, then confirm against daemon ground truth.

Sibling skill: `yggui-app-control` (the agent's hands+eyes on the live GUI —
screenshots, open/send, restart loop). This skill is the **diagnostic instruments**.

## Decision order (which tool, when)

1. **Reproduce deterministically first** — `mock-tui` + `pipeline_integration` (daemon
   pipeline) and/or `xterm-harness` (xterm.js client layer). A green deterministic
   repro that fails-then-passes is the only durable proof. Extract the relevant
   decision into a pure module so it's unit-testable (see "Extracted decision specs").
2. **Then confirm on the live host** with daemon/app-control probes — never from
   screenshots alone (instruments lie; see Caveats).
3. **Cross-validate against what the user sees.** If they're using a session right
   now it cannot be "unusable." A claimed break must be visible to a human.

## 1. Deterministic harnesses

### mock-tui — a codex-like deterministic TUI byte source
`crates/yggterm-server/src/bin/mock-tui.rs`. The server spawns it in place of
codex/CC/a shell so the read/replay/recovery pipeline is testable reproducibly.
**It is already codex-like — do NOT clone the codex repo to model TUI behavior.**
Scenarios (`--scenario`): `alt-screen`, `alt-screen-exit`, `normal-scrollback --rows N`,
`clear-storm --count N`, `burst --kb N`, `prompt-box`, `working`, `echo`, `menu`,
`delayed-prompt`, `composer` (interactive codex-style char-echo + Ctrl+U + Enter),
`codex-inline` (the codex inline-viewport pattern: committed lines scroll + a fixed
bottom live region — composer + status — repainted IN PLACE via absolute CUP).
Also `--replay <path>` to emit a recorded real-PTY byte stream verbatim. `--hold-ms`
keeps the PTY open. See `docs/integration-testing.md`.

### pipeline_integration — the daemon pipeline (pre-xterm.js)
`crates/yggterm-server/tests/pipeline_integration.rs` (run: `cargo test -p yggterm-server`).
Drives mock-tui through the daemon and asserts daemon-side truth: scrollback growth,
alt-screen, clear-storm final frame, codex reveal serving history, base_y semantics,
grid preservation across restart, echo-verified submit, etc. This guards everything
**before** xterm.js renders it.

### xterm-harness — the xterm.js client layer (post-daemon)
`tools/xterm-harness/` (run: `cd tools/xterm-harness && npm test`). Node + jsdom over
the **exact vendored** `assets/xterm/xterm.js` (byte-identical to the GUI's
`include_str!`'d bundle) — so buffer/scrollback/reflow behavior asserted here is what
actually runs in the WebKit webview. Helpers in `harness.js`: `createTerminal`,
`write`, `bufferText`, `lineText`, `baseY`, `cellBg`. Use it to settle xterm.js
questions deterministically (e.g. "does a codex frame survive a row-resize?",
"does broken-bottom self-correct on the next CUP frame?", "does a written bg survive
a widen reflow?"). To test client *decision* logic, extract it into a small module
(below) and assert it here.

### Extracted decision specs (pure, unit-testable; the JS mirrors them)
The client scroll/replay decision logic lives in big `format!` JS strings in
`shell.rs` — untestable inline. Extract the DECISION into a pure Rust module with
unit tests + a guard test that asserts the generated JS string contains the wired
logic. Existing examples:
- `crates/yggterm-shell/src/scroll_mode.rs` — the consolidated scroll-mode controller
  spec (Following|Pinned|Selecting, transitions, `should_follow_now`, `should_settle_follow`).
- `crates/yggterm-shell/src/terminal_retained_replay_policy.rs` — retained-replay /
  rehydrate / blank-host-replay decisions (daemon-screen vs client-snapshot selection).
This is the README's prescribed path for D1/D4/D6-class behavioral guards.

## 2. Live daemon + app-control probes (ground truth)

Run via `yggterm-headless server …` on the host (or the active launcher). Prefer
these over screenshots.

**The perf trio — reach for these before guessing why the fan is loud.** All three
read LOCAL logs in-process and never touch the daemon, so they answer even when it
is busy (and they must run from the NEWEST binary — see the no-handoff carve-out in
`yggterm-headless.rs`, or you get `unsupported server command` on the very host you
are profiling).

- `server perf-summary [--category render] [--since-ms N] [--top N] [--json]` — the
  rolling aggregate, ranked by total. The `clock` column says whether a row's
  milliseconds are WALL or CPU time; `render` rows are CPU.
- `server perf-incidents [--list] [--json]` — the durable snapshots of "the app went
  hot", grouped by trigger and ranked by count. `span_cpu_hot` is a CPU-time span past
  1.2 cores; `span_busy`/`span_stall` are wall-clock triggers.
- `cargo run -p yggterm-core --example render_top -- <gui-pid> 15000` — LIVE per-process
  render cost of the GUI tree: cores, PSS and **`gpu_ms`** per role. Every number is a
  delta between two samples. (Being promoted to `server render-top`; until then the
  example is the reader.)

- `server snapshot` — the daemon view. `active_session` (and `live_sessions[]`) carry
  per-session `launch_phase`, `remote_deploy_state`, **`pty_cols`/`pty_rows`** (the SQUISH
  gauge — the PTY's real grid), and **`terminal_lines`** (the daemon's authoritative
  vt100 screen, escapes inline — strip before diffing; this IS the daemon-screen ground
  truth — there is NO separate `server terminal screen` CLI verb), `metadata`, `ssh_target`.
  The "is the daemon healthy / what does it actually hold" probe. Parse: the JSON is
  flat at top level (`active_session`, `live_sessions`, `remote_machines`), NOT under a
  `data` key — but `server app …` responses ARE wrapped in `data`. Mind the difference.
- `server app terminal reconcile <path>` (alias `reconcile-from-daemon`, since v2.8.45)
  — **repair a squish / broken-bottom**: reads the daemon's authoritative screen and
  replays it into the client xterm via the `daemon_screen_snapshot` path (the same
  reconcile machinery the reveal path uses). Unlike `redraw` (renderer re-fit only) this
  repaints CONTENT. Returns `{accepted, source, bytes, line_count, running, looked_working}`.
  CAUTION: it re-seeds the client to the CURRENT screen → collapses base_y to 0 (drops
  retained-replay history; harmless for codex which owns no real scrollback, but it IS a
  buffer reset). A REPAIR tool, not a routine op — only run it on an actually-broken surface.
- `server terminal resize <key> --cols N --rows N [--nudge]` (since v2.8.47) — resize the
  LOCAL daemon PTY, which sends a SIGWINCH down the ssh channel to the REMOTE agent CLI.
  **The confirm+recover tool for a "squish"** where the remote codex renders at a stale
  smaller grid (e.g. default ~120×36) than the client/daemon (e.g. 167×63) after a
  re-resume/daemon-restart — the daemon PTY can read 167×63 while the remote codex never
  got the SIGWINCH/repaint. `--nudge` first resizes to (cols-1,rows-1) then to the target,
  forcing a fresh SIGWINCH when the daemon PTY size already matches. Confirm via a faithful
  screenshot before/after (does codex reflow to full width / composer drop to the bottom?).
  See `finding-codex-squish-post-restart-pty-size`.
- `server app state` — the active session + `active_terminal_hosts[]`: `cols`/`rows`,
  `base_y`, `viewport_y`, `scrollback_intent`, `retained_replay_source`, `text_tail`,
  `xterm_session_snapshot_nonblank_line_count`, `window_focused`/`document_focused`;
  plus `active_view_mode` and **`session_view_contract_violations`**.
- `server app terminal probe-scroll <path> --lines 0` — the **`viewport_force_log`**
  ring (every viewport move: reason/target/base/before/after/noop) + per-host counters
  (e.g. `settleSelfHealCount`). **THE reliable instrument for scroll/jump/lock bugs** —
  push-on-move, not a pollable snapshot.
- For the daemon's authoritative vt100 screen use **`server snapshot` → `active_session.terminal_lines`**
  (above). The `server terminal screen` and `server app terminal read-buffer` CLI verbs
  referenced in older notes are NOT wired in the shipped headless binary (they return
  "unsupported command") — do not rely on them; use `server snapshot` / `server app state`.
- `server terminal tenants [<session>]` (since v2.12.17) — **what is RUNNING inside a
  row, and what it has cost.** The immortal-tenant probe (`docs/pending-bugs.md`, the
  aged-`ssh`/`htop` class): per row it reports the foreground command, every descendant
  with per-process CPU seconds and age, the age+command of the oldest NON-SHELL tenant
  ("something has been squatting in here for days"), plus the row's creator stamp and any
  ephemerality declaration. ON DEMAND ONLY — one `/proc` reading serves every row in the
  answer and nothing polls, so asking is the entire cost. A row it cannot measure reports
  a NAMED reason (`no_local_runtime`, `runtime_not_running`, `root_pid_unavailable`,
  `root_pid_not_in_proc`, `proc_unreadable`, `not_supported_on_platform`,
  `runtime_unreachable`) with every number left EMPTY — never a zero, because a zero
  reads as "this row is cheap". A row whose PTY another daemon owns is PROXIED to that
  owner and merged under our row identity, so the answer never tells you to go ask
  elsewhere; `runtime_unreachable` means that owner did not answer. Read-only, and
  allowed for shadow clients — it is the verb an agent uses to audit what its
  predecessors left running. KNOWN UNDER-COUNT: the walk is descendants-only, so a
  tenant that reparented away (daemonised, orphaned to pid 1) is not counted.
- `server app terminal new … [--purpose <text>] [--ephemeral (--ephemeral-owner-pid <pid>
  | --ephemeral-idle-ttl-secs <n>)]` — every agent-CLI create stamps provenance on the row
  (creator pid + host + purpose, persisted across a daemon handover; read it back with
  `terminal tenants`). `--ephemeral` additionally opts the row IN to reaping and is
  REFUSED on its own: there is no default owner, because under `bash -c "<cli>"` the
  parent is the wrapper bash and under `ssh host "<cli>"` it is sshd-session, both dead
  within milliseconds of the create — a defaulted owner would reap the row on the next
  chore tick. Name a pid you know outlives the create (your own), or give a TTL. The
  create response echoes what was armed under `tenancy.declared`; `declared: false`
  means NOTHING was armed, with the daemon's reason alongside.
- `server trace tail` — the event trace (daemon + `ui` events). Time-order it to see a
  reveal/reconcile/replay sequence. (Rotates — grep `~/.yggterm/trace/*.jsonl` for older.)
- `server app rows` — browser/sidebar rows (kind, label, full_path).
- `server app session <remove|delete> <path>` — delete a session (e.g. a phantom).
- `server app screenshot [out.png]` — app capture. **Since v2.8.46, when the active view
  is a terminal and the canvas renderer is on, this composites the xterm canvas layers
  IN-PROCESS (`capture_backend=xterm_canvas_composite`, `capture_faithful=true`) — a
  faithful terminal pixel on EVERY platform with NO Spectacle, NO window focus.** This is
  the instrument that ends agent-blindness: take it, `scp` it back, and Read the PNG to
  SEE squish/broken-bottom/blank with your own eyes (never declare a visual state from
  telemetry — see CLAUDE.md missteps). The image IS the terminal region; the redundant
  `--region terminal` crop is auto-dropped. The IMAGE POST-PROCESS PIPELINE
  (`--region terminal|full`, `--crop x,y,w,h`, `--scale N`) is wired into `yggterm-headless`
  since v2.8.47 (earlier it was GUI-binary-only) — use `--crop`+`--scale` to zoom into a
  suspect region (composer row, right edge) since a full frame reads small. The composite is
  at devicePixelRatio so it's legible even without upscale. Spectacle remains a last-resort
  fallback (needs yggterm focused — fails over SSH, the old trap).
  - **Split view (v2.10.7):** the composite draws EVERY visible terminal pane over the
    main-surface frame, so a split renders both panes in one faithful frame — not just the
    focused one (that was the pre-split behavior). `server app state` → `data.split_view`
    reports the group SSOT (`active_group_id`, `groups[].{axis,ratio,members,active_pane}`);
    per-pane cols/rows off `active_terminal_hosts[].cols` prove the reflow (side-by-side ≈
    half cols, stacked ≈ half rows). Drive splits headlessly with `server app split
    create|focus|ratio|ungroup` ([[campaign-split-view-groups]], `docs/split-view.md`). A
    non-active pane can flash stale-atlas garble right after the split reflow; the group heal
    clears it, and focusing the pane always re-renders it crisp.
- `server status` — daemon version/uptime. `server monitor --scenario panic-report|
  server-list|latency-check|wait-session|hot-restart` — incident triage (see AGENTS.md).

## ⚠️ Match the Linux display backend when launching the GUI (recurring mistake)

**Before launching/relaunching the GUI for a test, detect the session's display
backend and launch to match it. Forcing the wrong one is a recurring error that
breaks clipboard/paste, screenshot faithfulness, and native compositing.**

- **Detect:** `ls /run/user/$(id -u)/wayland-*` → if a `wayland-*` socket exists, the
  session is **Wayland** (jojo is KDE Wayland). `XDG_SESSION_TYPE` over an SSH shell
  reads `tty` and is USELESS for this — check the socket, or the running GUI's
  `/proc/<pid>/environ`.
- **On Wayland, launch with Wayland env — do NOT `export DISPLAY=:0`.** `DISPLAY=:0`
  forces the app under **XWayland**, and the symptom is exactly what bit us: **paste
  fails** (X11↔Wayland clipboard mismatch; the GUI shows a "can't paste" notification),
  plus unfaithful screenshots and disabled compositing. Correct form:
  ```sh
  ssh <host> 'XDG_RUNTIME_DIR=/run/user/$(id -u) WAYLAND_DISPLAY=wayland-0 GDK_BACKEND=wayland \
      ~/.local/bin/yggterm-headless server app launch'
  ```
  (unset/omit `DISPLAY`, or `GDK_BACKEND=wayland` overrides it). Verify after launch:
  `tr '\0' '\n' < /proc/<gui-pid>/environ | grep -E 'WAYLAND_DISPLAY|DISPLAY|GDK_BACKEND'`
  — `WAYLAND_DISPLAY` should be set and `GDK_BACKEND` should be `wayland`, NOT a bare
  `DISPLAY=:0`.
- **On a real X11 session** (only `/tmp/.X11-unix/X0`, no wayland socket): `DISPLAY=:0`
  is correct; do not force `GDK_BACKEND=wayland`.
- A GUI launched in the wrong backend must be relaunched correctly — clipboard/paste
  and screenshot fidelity won't work until it is. See `finding-app-screenshot-unfaithful-on-wayland`.

## 3. Caveats — which instruments lie (hard-won)

- **`ps %CPU` is a LIFETIME AVERAGE, not current load.** A process that pegged a core
  for two hours and then idled reads identically to one pegging it now, and a busy GUI
  on a 16-core box reads a reassuring `load average: 0.79`. This is how "105% of a core"
  misled a whole campaign. Use `render_top` (deltas) or CPU-seconds from `/proc`.
- **A render number is per PROCESS, never per surface.** WebKitGTK runs one web process
  per profile serving every surface on it, so a per-surface CPU number would be a
  fabrication. Surface counts ride along as caller-supplied context.
- **Low CPU does not mean the GPU is working.** `drm-engine-gfx` in
  `/proc/<webproc>/fdinfo/*` — nonzero and RISING across two reads — is the only proof
  the GPU is rasterizing. `render_top`'s `gpu_ms` prints `-` when that counter was
  unreadable, which is NOT a zero: conflating "no permission" with "no GPU work" is
  exactly how this product ran with its GPU switched off for months.
- **Which GL path a window is on is a published field — and /proc CANNOT answer it.**
  `server app desktop-identity` gives each client a `webkit_gl_environment` map
  (`YGGTERM_WEBKIT_GL_POLICY`, `LIBGL_ALWAYS_SOFTWARE`, `GALLIUM_DRIVER`,
  `WEBKIT_DISABLE_DMABUF_RENDERER`, `YGGTERM_WEB_SURFACE_UNDER_GLASS`) that the GUI
  publishes from its OWN environment. ⚠⚠ Do not read these out of
  `/proc/<pid>/environ` — or out of the `exec_environ` map next to it, which is the
  same thing. `setenv`/`unsetenv` move the environ array to the heap while the kernel
  keeps exposing the exec-time copy, so `/proc` shows nothing on a fresh launch and
  the PREDECESSOR's values after a hot restart or `server app launch`. A missing key
  in `webkit_gl_environment` means the process REMOVED it (hardware GL clears the
  software force); an empty map means the client published nothing, never "software".

- **`app state` `viewport_y` is STALE when the window is backgrounded.** It can disagree
  with what the user sees. Use the `viewport_force_log` (probe-scroll) and the user's
  eyes for live scroll position; never trust `viewport_y` alone when unfocused.
- **PUBLIC vs EFFECTIVE viewport.** `buffer.active.viewportY` (public) is the buffer
  position; `effectiveXtermViewportY` (render/ydisp) is what's painted. They diverge on
  a stale-render strand (bg→fg) — public reads at-bottom while the render is stranded
  above. Measure strands with the EFFECTIVE value (what `app state` reports).
- **Wayland focus trap.** On KDE Wayland a visible FOREGROUND window reports
  `document.hasFocus()=false` (`document_focused=false`). NEVER gate layout/render
  mutations on focus — gate on VISIBILITY (`hostLooksUsable`). And you CANNOT synthesize
  the OS window-focus (bg→fg) trigger eye-free on jojo (wmctrl/xdotool are X11) — that
  one transition needs a user trigger; everything else is agent-instrumentable.
- **Daemon screen = authoritative; client buffer can be stale.** A "broken bottom" is
  almost always client-paint vs a correct daemon screen — diff them.
- **Screenshots: FIXED for the terminal (v2.8.46).** `server app screenshot` now
  composites the xterm canvas in-process (`xterm_canvas_composite`, faithful) — works over
  SSH, unfocused, any platform. The old "screenshots lie on Wayland" trap
  (`finding-app-screenshot-unfaithful-on-wayland`) was the Spectacle path needing window
  focus the agent can't hold; that's now bypassed for the terminal. (Full-app/non-terminal
  chrome still uses the webkit/Spectacle path — faithful for DOM, canvas-blind only if you
  capture the terminal region via the full-app path instead of the composite.)
- **Passing deterministic test ≠ live-fixed** — verify the ACTUAL live path/source the
  symptom uses (the 2.8.26 reconcile passed its string test but the live reveal carried
  a different `retained_replay_source`).
- **Don't free-list issues from raw telemetry fields** — a field name may not mean what
  it says (`input_enabled` once meant focus-ownership, not "user can type"). Read the
  code that sets it or falsify against a live probe before citing it.

## Pointers
`docs/integration-testing.md` (harness usage), `docs/xterm-bugs.md` (the xterm.js bug
registry — every workaround has an `// XTERM-BUG: <id>` anchor + entry), `docs/xterm.md`
(rendering/PTY bytes). Memory: `campaign-xterm-dealbreakers` (the master plan + which
bugs recur), `audit-viewport-scroll-control-flow` (the scroll/follow class + the
consolidated controller design + live captures).
