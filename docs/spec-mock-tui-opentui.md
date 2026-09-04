# mock-tui (OpenTUI) — the deterministic agent-CLI yggterm can interrogate

Status: SPEC + scaffold shipped 2026-09-05 (cli-integration campaign, Issue 37
wave; instrumented-finding wave same day). Runtime:
`tools/mock-tui-opentui/`. Companion to
[`docs/integration-testing.md`](integration-testing.md) (the Rust-side
`mock-tui` byte source and its daemon-pipeline harness) — that one feeds the
DAEMON pipeline tests; this one is a full TUI PROGRAM for the live client
stack.

## The problem it exists for

Every cli-integration riddle (docs/cli-integration.md, Issues 28–37 + the
pending-bugs paint/mouse classes) has the same testing shape: **yggterm
observes a foreign TUI and must prove what it saw.** Real CLIs are bad
fixtures: slow, versioned against us, private, often un-headless. A mock
built on **Anomaly's OpenTUI** (the framework opencode v2 renders with)
reproduces the real stimulus classes — alternate-screen entry, cursor-mode
pushes, keyboard-protocol flags, mouse-mode DECSETs, title OSCs, resize
repaints — at whatever SPEED the riddle needs (the "ultrafast paintjob"
dial), deterministically, forever.

## The observability law (v2, owner-directed 2026-09-05)

**Full-stack observability: the mock-tui is wired into ytrace on BOTH ends.**

- yggterm's side is already wired: `mouse_mode_probe` (every DECSET the
  client parser applied), `frame_hash_probe` (daemon grid vs client frame),
  `resize*` with origin fields, `pty_in_alternate_screen`, the `cli/*` chain.
- The mock's side: each scenario writes a structured JSONL trace
  (`~/.yggterm/mock-tui/<session>.jsonl`, category `mock`) — frame emitted,
  SIGWINCH seen, byte counts, scenario state machine transitions — so a
  falsifier joins MOCK timeline ↔ YGGTERM timeline by timestamp and sees
  exactly which side dropped what. (The stimulus/witness split stays: the
  mock's file is ITS OWN plane, never written into the daemon's trace
  stream.)
- The production dream this serves: **yggterm ships a mock-tui row type that
  only agents launch** (launcher-registry manifest + the yggui automation
  verbs, never a human menu entry) — a permanent instrumented TUI living in
  the fleet's pocket: when ANY paint/switch/mouse bug is reported, the
  falsifier is one `terminal new` away, and the mock's own trace answers
  "did the TUI draw it?" before anyone suspects the terminal.

## The scenario ↔ riddle ↔ witness matrix

| Scenario | The riddle it exercises | Witness recipe (yggterm side) | Status |
|---|---|---|---|
| `bg-fill` | The full-bleed contract (Issue 37): a TUI's own background must reach every pixel of the card | `server app screenshot` → pixel-sample the card corners; verified FIXED 2026-09-05 on the active GUI (all corners TUI-colored, alt-screen + nudge included) | ✅ falsifier PASSED |
| `alt-screen` | Alt-screen truth: `pty_in_alternate_screen` flips, D-pad/scrollbar stand down, grid survives enter/exit | `server snapshot` → `pty_in_alternate_screen`; scroll-controller `data-yggterm-scroll-controller-visible=false` | ✅ verified hide-on-alt + nudge-while-alt |
| `title-cycle` | Per-TUI identity from the title plane (Issue 34) | `ytrace tail --category cli` → `title`/`row_rebound_to_title_session` | scaffolded |
| `winch-witness` | Resize physics ([11.39]): repaint ONLY on SIGWINCH; the nudge must produce a new frame | `server terminal resize --nudge`; `resize_repaint_nudge` + `WINCH_FRAME_<n>`; live falsifier needs the ACTIVE client (shadow mounts never nudge — D8) | instrument fixed (`--winch-secs 0`); active-client run owed |
| `mouse-probe` | Mouse-code TUIs ([11.59]): DECSET 1000/1002/1006 armed + clicks/wheel must encode SGR to the app | `mouse_mode_probe` events; app echoes SGR bytes; `server app pointer click` (REAL OS input) | 🔴 root-caused 2026-09-05 — see pending-bugs [11.59] |
| `fast-paint` | The switching paint-break class: full-frame repaints at 30–120 fps while rows/clients/daemons switch | `frame_hash_probe` mismatch-at-quiescence; `paint` events; per-switch join of mock trace vs client frame hash | scenario spec — build next |
| `composer` | The codex-inline pattern (committed lines scroll + bottom live region via absolute CUP) | Rust pipeline `codex-inline` tests; live reveal/composer pinning | scaffolded |
| `kitty-keys` | Keyboard-protocol pushes/pops must not wedge the input gate | keys echo decoded; `input/loop_block` quiet | scaffolded |
| `paste-bracketed` | Bracketed paste round-trip | multi-line paste lands as ONE paste | scaffolded |
| `session-imitator` | The metadata-integration layer (Issues 28–36): imitate codex/claude/opencode session switching — titles (`OC | <session>`), store files, resume ladders — then cross-check against the real CLI | `cli/identity_poll`, `resume_decision`, `projection`, store-scan counts | spec — the mock ships its OWN store shape and the daemon reader contract test; never imitate a private db |

## Measured findings this instrument already produced (2026-09-05)

1. **[11.59] mouse-code TUIs are deaf** (root-caused, fix owed): with DECSET
   1002 armed (xterm `mouseTrackingMode=drag`, SGR encoding latched,
   protocol+encoding verified in the live `coreMouseService`), a REAL
   OS-level click (`server app pointer click`) reached the app LATE and
   DUPLICATED (press→release→press — a stuck button state), and wheels
   dispatched on the canvas never reached the app at all — while wheels
   dispatched directly on `.xterm` DID produce SGR. The DOM chain is
   `.xterm-screen → .xterm-scrollable-element → .xterm`: the xterm.js 6
   ScrollableElement consumes canvas-originated wheels even when the mouse
   protocol's own wheel handler (bound on `.xterm` by `bindMouse()`) should
   receive them. `mouse_mode_probe` stayed dark the whole time (its events
   never fired despite the parser applying the DECSETs) — the observability
   defect that let this hide.
2. **Full-bleed + D-pad verified healthy** on the deployed build: alt-screen
   TUI fills every card pixel (active GUI, nudge included); the scroll
   D-pad hides on alt entry and stays hidden through a nudge; the
   element-intersection sweep found nothing painting over the grid.
3. **Shadow mounts never repaint-nudge** (D8 — shadows never resize the
   PTY): [11.39]'s live falsifier needs the active client. The Rust
   winch-repaint witness got `--winch-secs 0` (the hardcoded 30s deadline
   retired it before mounts, three times measured).

## The dream — every cli-integration riddle reproducible without a live CLI

1. **Deterministic Issue repros.** Each landed Issue (28–37) and each
   pending-bugs paint/mouse class gets its regression scenario here; the
   scenario matrix above is the roadmap. A new agent CLI's onboarding
   starts from a mock that already speaks title planes, stores, and resume
   ladders.
2. **The agent-only mock-tui row in production.** yggterm ships the
   manifest; agents spawn instrumented TUI rows beside the suspect CLI and
   A/B them under the same switches/swaps — the mock's own ytrace-side file
   answers "did the stimulus draw it?" so the terminal's guilt is provable
   per incident, not per campaign.
3. **Two-engine identity as a spec.** The raw engine is the executable
   statement of the bytes; the opentui engine proves the same contract
   survives a real TUI framework's render loop. A divergence IS a finding.
4. **The CI plane.** xterm-harness (client) + pipeline tests (daemon) +
   this mock (live stack) — every falsifier scripted, none costing owner
   hands.
