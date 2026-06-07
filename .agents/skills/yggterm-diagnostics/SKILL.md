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

- `server snapshot` — per-session daemon view: `launch_phase`, `remote_deploy_state`,
  **`pty_cols`/`pty_rows`** (the SQUISH gauge — the PTY's real grid), `terminal_lines`
  (the daemon's authoritative vt100 screen, escapes inline — strip before diffing),
  `metadata`, `ssh_target`. The "is the daemon healthy / what does it actually hold" probe.
- `server app state` — the active session + `active_terminal_hosts[]`: `cols`/`rows`,
  `base_y`, `viewport_y`, `scrollback_intent`, `retained_replay_source`, `text_tail`,
  `xterm_session_snapshot_nonblank_line_count`, `window_focused`/`document_focused`;
  plus `active_view_mode` and **`session_view_contract_violations`**.
- `server app terminal probe-scroll <path> --lines 0` — the **`viewport_force_log`**
  ring (every viewport move: reason/target/base/before/after/noop) + per-host counters
  (e.g. `settleSelfHealCount`). **THE reliable instrument for scroll/jump/lock bugs** —
  push-on-move, not a pollable snapshot.
- `server terminal screen <key>` — the daemon's authoritative vt100 screen (compare
  vs the client's painted bottom to prove a client paint break).
- `server trace tail` — the event trace (daemon + `ui` events). Time-order it to see a
  reveal/reconcile/replay sequence. (Rotates — grep `~/.yggterm/trace/*.jsonl` for older.)
- `server app rows` — browser/sidebar rows (kind, label, full_path).
- `server app session <remove|delete> <path>` — delete a session (e.g. a phantom).
- `server app screenshot --region terminal|full --crop x,y,w,h --scale N` — app-level
  capture; on KDE Wayland it uses Spectacle (see `yggui-app-control`). A 1920px full
  frame is illegibly small — crop + upscale.
- `server status` — daemon version/uptime. `server monitor --scenario panic-report|
  server-list|latency-check|wait-session|hot-restart` — incident triage (see AGENTS.md).

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
- **Screenshots lie on Wayland** unless via the activation+Spectacle path
  (`finding-app-screenshot-unfaithful-on-wayland`).
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
