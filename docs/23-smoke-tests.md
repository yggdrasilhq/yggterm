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
- Start resource logging before opening any test session so baseline and
  cause/effect data are comparable. Record idle GUI, daemon, WebKit child,
  remote daemon, memory, swap, and app-control latency before the first test
  click.
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
run either:

- `htop`
- `npx codex-session-tui`

Choose the command that is valid for the target machine and cwd. The remaining
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
  not resize the terminal grid
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

Resource logging must cover three windows:

- baseline before opening test sessions
- active workload while the 23 sessions are visible/reachable
- cooled period after the GUI is closed and before it is respawned

Record CPU, memory, swap, daemon process list, GUI process list, and app-control
latency. A pass requires no unexplained sustained CPU spike, no swap growth that
survives cleanup, and no fan-level load from idle cooled sessions.

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
- cleanup closes/removes every test-created non-user session

When this passes, the candidate can be promoted to the next `x.y.0` release.

## Artifacts

Keep a proof bundle for each run with:

- candidate version, git commit, install path, and checksum
- random seed and selected machine/cwd targets
- command plan for the 7 heavy and 16 ordinary sessions
- screenshots and app-control JSON before and after restore
- resource logs for baseline, active workload, cooldown, and respawn
- keep-alive set and restore comparison
- defect notes or explicit "no defect found" summary
- cleanup report

## Current Project Lab Example

For this repository, the maintainer's live gate is expected to run on the
available private machines, currently including systems such as `jojo` and
`dev`. Those names must not be baked into the implementation of the smoke test:
automation should discover machines and cwd targets from the running Yggterm
session graph.
