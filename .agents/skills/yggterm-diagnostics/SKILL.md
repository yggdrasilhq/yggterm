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
  `--region terminal` crop is auto-dropped. NOTE: `--region/--crop/--scale` are parsed by
  the GUI binary but NOT yet by `yggterm-headless` (it ignores them → native-res PNG); the
  composite is already at devicePixelRatio so it's legible without upscale. Spectacle
  remains a last-resort fallback (needs yggterm focused — fails over SSH, the old trap).
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
