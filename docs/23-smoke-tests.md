# 23 Smoke Tests

The 23 smoke test system is the release gate for promoting a patch train into a
new `x.y.0` release. The number 23 is a convention, not a product limit: it is
large enough to expose session graph, terminal rendering, keep-alive, restore,
and resource-budget bugs on a real multi-machine workspace without turning the
gate into a full soak test.

This protocol is infrastructure-neutral. Local names such as `jojo`, `dev`, and
`local` are examples from the project maintainer's environment, not requirements
for another operator.

## Scope

Run this gate against a real running Yggterm installation that has at least one
GUI client, one daemon, and one or more machine trees available through the
sidebar/session graph. The test must exercise the combined cwd tree across all
reachable machines, not a hand-picked single directory.

Passing this gate means the candidate is eligible for a minor release bump:
`x.(y-1).z` becomes `x.y.0`. Future releases may add more checks to this file,
but should keep this base contract intact.

## Preconditions

- Snapshot user state before changing a live install:
  `~/.yggterm/server-state*.json`, `session-titles.db`,
  `event-trace.jsonl`, install metadata, and relevant app-control proof files.
- Confirm the active GUI, daemon, launcher, and headless binary versions before
  testing.
- Confirm direct-install launcher identity before creating sessions. If
  `~/.local/share/yggterm/direct/install-state.json` exists, `~/.local/bin/yggterm`
  and `~/.local/bin/yggterm-headless` must be either current launcher scripts or
  point at the active executable pair in install-state. A symlink or copied
  binary that still resolves into an older `direct/versions/<version>/` directory
  is a release-blocking stale-binary failure.
- Start resource logging before opening any test session so baseline and
  cause/effect data are comparable. Record idle GUI, daemon, WebKit child,
  remote daemon, memory, swap, and app-control latency before the first test
  click.
- Record `yggterm-headless server monitor --scenario server-list` at baseline,
  after GUI restore, and after cleanup. The same runtime key must not appear in
  more than one daemon's `owned_terminal_session_keys`; preserved-owner entries
  are allowed only when exactly one daemon directly owns the PTY.
- Reset/generate title-summary copy only for the app graph under test unless
  the release is explicitly validating local archive maintenance:
  `yggterm-headless server sessions regenerate-copy --skip-local --reset-summary-history`.
  If local archive regeneration is included, preserve the budget and elapsed
  time in the proof bundle.
- Prefer app-control operations over desktop-wide pointer or keyboard
  automation on a user's active desktop.
- Keep every test-created session identifiable so it can be closed or removed at
  cleanup.

## Selection

1. Build the candidate's combined cwd list from every reachable machine tree in
   the running system.
2. Randomly select 23 cwd targets from that combined list.
3. Preserve the random seed, selected machine/cwd pairs, and candidate version in
   the proof bundle.
4. If the system exposes fewer than 23 usable cwd targets, use all available
   targets and record the reduced coverage as a limitation.

## Session Load

Open 23 sessions from the selected cwd targets.

Choose 7 of those sessions as heavier terminal workloads. In each heavy session,
run the first deterministic TUI available on that target:

- the checked-in smoke harness's Python curses fixture, when Python curses is
  present
- `htop`
- `top`
- a locally available or already cached `codex-session-tui`

Do not let this gate depend on network package download latency. The remaining
16 sessions should run ordinary shell workloads such as directory listing,
prompt interaction, short commands, or PowerShell equivalents on Windows.

The 7-heavy-session cap is intentional. It is enough to expose TUI redraw,
resize, and background-pipe issues while keeping the gate usable on machines with
limited CPU headroom.

## First Pass Checks

For every opened session, collect app-control state plus a screenshot or terminal
probe that proves the terminal output is readable and current.

Check at minimum:

- sidebar membership and cwd placement are deterministic
- active session identity matches saved-session identity
- prompt, cursor, and typed echo are visible
- the active terminal renderer is the default DOM row path unless the run
  explicitly opts into canvas; if canvas is enabled, screenshot pixels,
  app-control buffer text, and foreground/background contrast must agree
- full-width TUI lines remain coherent after settle
- scrollback does not jump without intentional scrolling
- background sessions do not burn CPU when cooled
- live/keep-alive status is represented from one source of truth
- no stale daemon, duplicate restored runtime, or ghost session becomes active

If any terminal output, session identity, restore state, or resource trace is not
fine, stop the release gate, file the defect in the current todo or issue plan,
fix it, and restart this smoke test from a clean candidate.

## Quirk Pass

The 23-session run must also include a deliberate pass over the small behaviors
that have previously hidden release-blocking defects:

- reveal and hide the autohidden titlebar while a terminal is active; the hover
  chrome must use the same background/gradient as the visible titlebar and must
<<<<<<< HEAD
  not resize the terminal grid or shift shell content. Stable builds must not
  report compositor blur, CSS backdrop blur, or a nonzero material blur budget.
- open the theme editor, reset the theme, change brightness through
  app-control, verify the brightness slider/manual field is visible, verify
  alpha/grain remain pinned to stable defaults even if legacy values are set,
  verify no repeated grain layer is emitted, verify the saved/effective theme
  and shell CSS variables change, then reset it again
=======
  not resize the terminal grid or shift shell content. Native compositor blur
  must remain active during the hover transition, and native blur paths must not
  mix in CSS `backdrop-filter` fallbacks. The alpha dial remains allowed to
  choose higher or lower material alpha as long as the blur path is active.
- open the theme editor, reset the theme, change brightness/alpha/grain through
  app-control, verify the grain dial can reach the high range, verify alpha is
  applied as a blur-backed material control where alpha 50 yields roughly 50%
  shell material plus computed blur, with a readable fallback when blur is
  unavailable, verify grain changes a repeated shell background layer
  (`background-image`, `background-size`, and `background-repeat`), verify the
  saved/effective theme and shell CSS variables change, then reset it again
>>>>>>> c162185 (Snapshot alpha blur experiment)
- click the active terminal viewport at random positions; the viewport must not
  flicker-scroll or settle at an unintended scrollback location
- resize or nudge the window; prompt-follow sessions must return to the prompt,
  TUI sessions must redraw coherently, and full-width separator lines must not
  be broken by the final terminal-ready phase
- switch away from and back to at least one `htop` or `codex-session-tui`
  session after the background pipes cool; the TUI must still be readable and
  interactive
- close an active live session and a background live session; the active close
  must redirect to the previous valid viewport or startpage, and the background
  close must not steal focus
- open the live-session close dialog with a session that also appears under a
  cwd projection; each runtime must appear exactly once in the confirmation
  copy
- inspect sidebar and startpage copy; generated titles must be meaningful
  engineering noun phrases, summaries must be human-readable paragraphs, and
  short UUIDs should appear only as explicit metadata or as documented fallback
  when generation fails
- run the app-control terminal probes for typed echo, scroll, selection/context
  menu, and xterm row style truth; the screenshot, probe, and state JSON must
  agree
<<<<<<< HEAD
- reject any state where app-control reports daemon-backed buffer text while
  the screenshot shows a blank terminal, or where canvas mode reports
  low-contrast foreground/background colors over a dark terminal surface
- query `~/.yggterm/telemetry/terminal.sqlite3` for the run window; every opened
  terminal must have `terminal_open_attempt/begin` and either
  `terminal_open_attempt/ready` or a documented failure/recovery event
- after the first `terminal_open_attempt/ready` for a retained remote terminal,
  reject a burst of new retained-fault `begin` events in the settle window. A
  transient post-ready blank sample may appear as
  `retained_fault_recovery_suppressed_after_ready`, but repeated remounts are a
  first-attach failure and a CPU/fan-budget defect.
- before the first ready event, reject retained rehydrate failures whose error
  is the current daemon socket being unreachable. A startup run may log
  `terminal_io/retained_rehydrate_daemon_ready_wait`, but it must not need a
  retained-fault watchdog remount to make the same preserved PTY snapshot
  readable. If the watchdog deadline fires during that wait, the only accepted
  event is `retained_fault_recovery_rearm_deferred_daemon_ready`; a
  `retained_fault_recovery_rearm` before daemon-ready is a failed smoke run.
- reject prompt-follow recovery if app-control shows the DOM viewport is already
  at the prompt while xterm's public `viewportY` is stale. In that case
  `viewport_y_source=dom_visual` is the accepted proof; a retained-fault remount
  caused only by the stale public counter is a failed smoke run.
- reject any active DOM-rendered terminal where xterm text exists but
  `dom_paint_hit_test_problem` is non-empty. The screenshot, row/cursor
  hit-test stack, and terminal surface summary must agree before the terminal is
  treated as drawable. If app-control screenshot capture is suspected to be a
  background/occlusion artifact, record an OS-level screenshot and classify that
  separately; do not turn a blank app-control capture into a pass silently.
=======
- query `~/.yggterm/telemetry/terminal.sqlite3` for the run window; every opened
  terminal must have `terminal_open_attempt/begin` and either
  `terminal_open_attempt/ready` or a documented failure/recovery event
>>>>>>> c162185 (Snapshot alpha blur experiment)

## Keep-Alive And Restore Pass

After the first pass is clean:

1. Randomly tag 23 combinations of sessions and terminals as keep-alive.
2. Record the exact keep-alive set.
3. Close the Yggterm GUI without killing daemon-owned keep-alive runtimes.
4. Wait 5 minutes.
5. Respawn the Yggterm GUI.
6. Repeat the first pass checks against the restored workspace.

The restore pass must prove that keep-alive protected the selected runtimes and
that non-keep-alive sessions followed the documented session-closing and saved
metadata contract.

## Resource Budget

<<<<<<< HEAD
Resource logging must cover five windows:
=======
Resource logging must cover four windows:
>>>>>>> c162185 (Snapshot alpha blur experiment)

- baseline before opening test sessions
- active workload while the 23 sessions are visible/reachable
- cooled period after the GUI is closed and before it is respawned
<<<<<<< HEAD
- respawn burst immediately after the GUI is restored
- respawn settled after the restored sessions have had a short settle period
=======
- respawn settle after the GUI is restored
>>>>>>> c162185 (Snapshot alpha blur experiment)

Record CPU, memory, swap, daemon process list, GUI process list, and app-control
latency. A pass requires no unexplained sustained CPU spike, no swap growth that
survives cleanup, and no fan-level load from idle cooled or freshly respawned
sessions. The release script must preserve the configured budget for each
<<<<<<< HEAD
window, not just the cooldown window. The respawn burst budget may be higher
than the settled budget, but the proof must show the load decays rather than
turning into a render loop.
=======
window, not just the cooldown window.
>>>>>>> c162185 (Snapshot alpha blur experiment)

For each resource window, include enough samples to distinguish transient work
from a leak or loop. The proof bundle should mark the causal boundary between
baseline, session spawning, heavy workload start, GUI close, cooldown, GUI
respawn, and cleanup.

## Pass Criteria

The gate passes only when:

- all 23 selected sessions open and render correctly
- all 7 heavy terminal workloads remain readable and interactive
- the other 16 shell workloads remain readable and interactive
- keep-alive tagging is deterministic and survives GUI close/respawn
- sidebar cwd/live grouping remains deterministic before and after restore
- titles, summaries, cwd placement, and long UUID metadata remain durable
- no stale runtime or ghost session becomes live
- app-control state, probes, screenshots, and resource logs agree
- terminal telemetry contains no unhandled live-truth split for the run,
  including stored session without runtime ownership, healthy remote machine
  with no drawable terminal after terminal launch, or empty xterm surface that
  only recovers after a manual switching pass
- cleanup closes/removes every test-created non-user session

When this passes, the candidate can be promoted to the next `x.y.0` release.

## Artifacts

Keep a proof bundle for each run with:

- candidate version, git commit, install path, and checksum
- random seed and selected machine/cwd targets
- command plan for the 7 heavy and 16 ordinary sessions
- screenshots and app-control JSON before and after restore
- terminal telemetry query output for opened sessions, recovery warnings, and
  errors during the run window
<<<<<<< HEAD
- resource logs for baseline, active workload, cooldown, respawn burst, and
  respawn settled
=======
- resource logs for baseline, active workload, cooldown, and respawn
>>>>>>> c162185 (Snapshot alpha blur experiment)
- keep-alive set and restore comparison
- defect notes or explicit "no defect found" summary
- cleanup report

## Current Project Lab Example

For this repository, the maintainer's live gate is expected to run on the
available private machines, currently including systems such as `jojo` and
`dev`. Those names must not be baked into the implementation of the smoke test:
automation should discover machines and cwd targets from the running Yggterm
session graph.
