# mock-tui (OpenTUI) — the deterministic agent-CLI yggterm can interrogate

Status: SPEC + scaffold shipped 2026-09-05 (cli-integration campaign, Issue 37
wave). Runtime: `tools/mock-tui-opentui/`. Companion to
[`docs/integration-testing.md`](integration-testing.md) (the Rust-side
`mock-tui` byte source and its daemon-pipeline harness) — that one feeds the
DAEMON pipeline tests; this one is a full TUI PROGRAM for the live client
stack.

## The problem it exists for

Every cli-integration riddle (docs/cli-integration.md, Issues 28–37) has the
same testing shape: **yggterm observes a foreign TUI and must prove what it
saw.** Today the only faithful stimulus is the real CLI — opencode, codex,
agy — and a real CLI is a bad test fixture:

- it is slow to spawn and versioned against us (a store migration can flip a
  scenario mid-campaign);
- its behavior is private and can change without notice (the `OC | <title>`
  plane was measured in its app.tsx, not promised);
- several of them cannot run headless at all, so CI never sees them;
- a falsifier that needs "switch a TUI now" needs a human hand.

The Rust `crates/yggterm-server/src/bin/mock-tui.rs` already proved the
pattern for BYTE streams: deterministic, scenario-keyed, replay-able. But it
is a byte source, not a TUI: it cannot answer what a MODERN TUI FRAMEWORK
actually emits on the wire — and the agent CLIs we integrate have converged
on exactly one such framework: **Anomaly's OpenTUI** (opencode v2 renders
with it). A mock built on the same engine reproduces the real stimulus
classes — its alternate-screen entry, cursor-mode pushes, keyboard-protocol
flags, mouse-mode DECSETs, title OSCs and resize repaints are the framework's
own, not our hand-rolled approximation of them.

## The one architectural law: the mock is the STIMULUS, yggterm is the WITNESS

The mock-tui emits **nothing into the ytrace plane**. It is a foreign process
by construction — that is its entire value, and the bus law
(`ci test runs emit to the fleet bus unless cfg(not(test))`-gated) must never
be tempted by it. Observability comes from yggterm watching the PTY it
already watches:

- `ui/terminal_mount` → `mouse_mode_probe` (every DECSET 1000/1002/1003/1006
  the client applied), `frame_hash_probe` (daemon grid vs client frame);
- `server/terminal_runtime` → `resize*` events with `origin` +
  `hash_before/after`, `resize_repaint_nudge`;
- `TerminalSnapshot.pty_in_alternate_screen` → `overlay_proxied_pty_truth`;
- the `cli/*` chain (`title`, `identity_poll`, `projection`, `resume_decision`)
  fires whenever the mock's emitted signals (titles, cwd, store files) give it
  something to see — see the pairing column in the matrix below.

Every scenario in this spec therefore ships with its WITNESS recipe: the ytrace
query or `server` probe that falsifies the claim. A scenario without a witness
recipe is a demo, not a test, and does not get merged.

## Runtime shape

```
tools/mock-tui-opentui/
  package.json          # bin: mock-tui; @opentui/core is an OPTIONAL engine
  src/mock-tui.js       # entry: arg parse, engine select, run loop
  src/ops.js            # the scenario op vocabulary + the ANSI compiler
  src/scenarios.js      # the scenario registry (the matrix, as code)
  README.md
```

- `--scenario <name>` — one of the registry (below).
- `--engine opentui|raw` — `raw` (default fallback, zero deps) compiles the
  same ops to ANSI bytes by hand; `opentui` renders them through
  `@opentui/core` and is the engine the spec exists for. Engine selection is
  per-run, and a scenario must behave IDENTICALLY at the witness level under
  both — that identity is itself a test (the raw engine is the executable
  spec of what the opentui engine must produce).
- `--hold-ms N` — stay alive after the last frame (for `server app
  screenshot` and manual falsifiers). Default: hold until stdin closes or
  `q`.
- stdin lines beginning `:` drive transitions mid-run
  (`:scenario alt-screen`, `:title OC | driven`, `:fill red`) so a test can
  script a SEQUENCE of stimuli over one PTY without restarting the process.
- Exit codes and a final stdout `MOCKTUI <scenario> <frames> ok` line make it
  assertable from shell pipelines (the Rust pipeline tests' pattern).

## The scenario ↔ riddle ↔ witness matrix

| Scenario | The riddle it exercises (Issue) | Witness recipe (yggterm side) |
|---|---|---|
| `bg-fill` | **The full-bleed contract (Issue 37)**: a TUI that paints its own background must reach every pixel of the card — no shell-owned strip on the right or bottom | Run it, `server app screenshot`, pixel-sample the card corner: fill color to the edges; `frame_hash_probe` `mismatch:false` at quiescence |
| `alt-screen` | Alt-screen truth: `pty_in_alternate_screen` flips, scrollback D-pad/overlay thumb stand down, grid survives enter/exit | `server snapshot` → `pty_in_alternate_screen`; `overlay_proxied_pty_truth` on a proxied row |
| `title-cycle` | Per-TUI identity from the title plane (Issue 34): `OC \| mock-<n>` via OSC 0/1/2 per PTY; the mirror must bind the row the title names | `ytrace tail --category cli` → `title`/`row_rebound_to_title_session`; switch two mock rows, row ids follow within a tick |
| `winch-witness` | Resize physics (the squish class, [11.39]): the TUI repaints ONLY on SIGWINCH, so a silent identical-geometry resize produces zero bytes and the nudge produces a new frame | `server terminal resize --nudge`; watch `resize_repaint_nudge` + the `WINCH_FRAME_<n>` line; `frame_hash_probe` agreement after |
| `mouse-probe` | Mouse-mode arming and SGR click reporting (the "clicks do nothing" class): arms DECSET 1000+1006, decodes and echoes clicks | `mouse_mode_probe` events (`enabled:true` then real transitions); clicks echoed on screen |
| `composer` | The codex-inline pattern (committed lines scroll + a fixed bottom live region repainted in place via absolute CUP) — reveal/composer integration without codex | Rust pipeline tests with `codex-inline` scenario (existing); live: reveal serves history, composer pinned |
| `kitty-keys` | Keyboard-protocol pushes/pops (progressive enhancement flags) — nothing may wedge the input gate when a CLI pops what it pushed | Keys echoed decoded; `input/loop_block` quiet; no `input_enabled` drift in `app state` |
| `paste-bracketed` | Bracketed paste round-trip (the paste path TUIs actually use) | `app_control_terminal_input*` tests; live: multi-line paste lands as ONE paste |
| `flowing` | **The history-retention stimulus (owner directive 2026-09-06 late)**: codex-shaped streaming — committed, numbered, token-stamped gibberish lines every 500 ms (`MOCKTUI_TOKEN` stamps the run, `MOCKTUI_EVERY_MS` the cadence). The token makes the content its own witness: whatever a switch retains is readable in the text | Run with a token, note the newest counter, switch the row away and back (or rotate the daemon): PASS = pre-switch counters still in scrollback AND the counter continues (same PTY, same process). FAIL = the codex signature — counter restarts at 00001 and/or the token changed (a fresh spawn ate the history). MEASURED 2026-09-06 ~00:15 on a live row (token `rot-falsifier-1`): switch away+back PASSed — pre-switch 00023 retained, counter continued past 84 to 113. The daemon-rotation arm: leave the row flowing through the next natural deploy and read the buffer after |

## The dream — every cli-integration riddle reproducible without a live CLI

The end state this scaffold is pointed at: **the whole of
docs/cli-integration.md's matrix becomes falsifiable in CI and on-demand
against a stimulus we own.**

1. **Deterministic Issue repros.** Each landed Issue (28–37) gets its
   regression scenario here — when a new agent CLI ships next quarter, its
   onboarding (docs/spec-adding-an-agent-cli.md) starts from a mock that
   already speaks title planes, stores, and resume ladders, and the
   integration bugs surface in the harness, not on the owner's desktop.
2. **Two-engine identity as a spec.** The raw engine is the executable
   statement of "what the witness must see"; the opentui engine proves the
   same contract survives a real TUI framework's render loop. When they
   diverge, the DIVergENCE is the finding (that is exactly how the epoch-type
   and user-home falsifications happened — fixture vs reality).
3. **Store/identity simulation grows last, and honestly.** Title planes and
   alt-screen are wire behavior — a mock can emit them faithfully. Store
   schemas (Issue 36's sqlite candidate) are PRIVATE per CLI: the mock should
   ship its OWN store shape + the daemon-side reader contract test, not
   imitate opencode's db. Method law (measure the store's real representation
   before writing the reader) applies to mocks twice over: a mock that
   encodes the implementer's assumption proves nothing.
4. **The CI plane.** `tools/xterm-harness` asserts the client layer, the Rust
   pipeline tests assert the daemon layer, and this mock closes the middle:
   a real TUI program through the LIVE client stack, headless-drivable on any
   row, with every assertion witnessed on the trace plane. The falsifier
   round that needed a human to "switch a TUI now" becomes a scripted
   sequence — and the campaign's falsifiers stop costing owner hands.
