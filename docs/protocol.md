# Yggterm Protocol

This document defines the runtime protocol that must hold across GUI launches,
daemon restarts, self-update, remote restore, and app-control proof. It is the
protocol-level companion to `docs/stability.md`, `docs/sessions.md`, and
`docs/xterm.md`.

The product promise is intentionally narrow: a Yggterm session is durable,
snappy automation for a terminal routine such as:

```bash
ssh dev
cd ~/gh/yggterm
codex resume <uuid>
```

Metadata, sidebar placement, summaries, screenshots, hot update, and
observability support that routine. They must never become alternate terminal
renderers, alternate input targets, alternate session identities, or an excuse
to lose the PTY.

## Vocabulary

- Client or GUI: the Dioxus desktop process that owns chrome, sidebar,
  Startpage, metadata editing, app-control registration, and the xterm.js
  embed.
- Daemon: the `yggterm-headless` server process that owns PTY lifecycle,
  terminal byte I/O, live-session snapshots, runtime keys, and remote commands.
- PTY runtime: the real local or remote terminal process tree.
- Runtime owner: the daemon process that currently owns a PTY runtime.
- Preserved owner: an older daemon that is intentionally kept alive because it
  owns a PTY that has not been adopted by the newer daemon.
- Terminal I/O key: the key used to route bytes to one PTY runtime, for example
  `codex-runtime://...`.
- Saved-session identity: the stable user-facing session identity. For
  Codex-owned sessions, this is the transcript JSONL id when available.
- Sidebar projection: a visible row in `Live Sessions`, a cwd/machine group, or
  Startpage. Multiple projections may exist, but they must resolve to one
  saved-session identity and one metadata record.
- App-control state: an observability surface. It can prove or disprove runtime
  truth, but it is not itself the truth source.
- Handoff: the update/restart interval where a new client or daemon takes over
  without losing any live runtime.

## Core Invariants

- The daemon owns PTYs, runtime keys, process lifecycle, retained PTY scrollback,
  and terminal byte I/O.
- xterm.js owns terminal rendering, cursor, prompt background, selection,
  scrollback, viewport, renderer mode, and resize behavior.
- The GUI owns shell chrome, row selection, metadata editing, and user-visible
  recovery presentation.
- Terminal mode has exactly one terminal truth source: daemon-owned PTY bytes,
  plus retained daemon scrollback derived from those same bytes.
- Preview mode may render transcript JSONL, generated copy, and metadata, but it
  must never seed or repair Terminal-mode xterm content.
- Synthetic runtime keys are I/O keys only. They must not replace saved-session
  identity in sidebar rows, cwd projection, title/summary storage, restore, or
  deduplication.
- A session may appear in both `Live Sessions` and its cwd/machine group, but
  both rows must resolve to the same saved-session identity, metadata record,
  live runtime status, and keep-alive state.
- Keep Alive is a daemon retention request. It is not the source of whether a
  live session appears under its cwd.
- App-control and screenshots are proof surfaces. If they disagree with daemon
  runtime truth, the disagreement is a bug to investigate, not an alternate
  source of truth.

## Session Close Definition

Closing a live session is a runtime operation, not a mode switch. The daemon
removes the selected PTY runtime and leaves saved transcript identity, cwd
projection, title, summary, and summary timeline intact.

Required close behavior:

- Closing a background live session must not change the active viewport.
- Closing the active live session must never leave the closed session selected
  in Web View, Terminal recovery, or a busy refresh loop.
- The GUI owns active-close fallback because it owns viewport history. The
  fallback order is the most recent valid viewport target before the closed
  session, then the last scoped Startpage, then the global Startpage.
- Viewport history is not runtime truth. It is only a bounded stack of stable
  session paths and Startpage scopes used to choose a post-close viewport.
- Before choosing a fallback, the GUI must prune every closed session path and
  normalized runtime alias from viewport history. The sequence `open A`, `open
  B`, `close A`, `close B` must not return to A.
- The daemon must not invent an arbitrary replacement active session after
  runtime removal. If another session becomes active, the GUI must explicitly
  sync that target from validated viewport history.

## Hot Update Definition

A hot update is successful only when session preservation survives the update.
Installing a new binary, relaunching a GUI, changing `install-state.json`, or
showing a readable retained buffer is not enough.

Required outcome:

- The active GUI process is running the intended new version and executable
  path.
- The active daemon executable path, pid, version, and socket are known and
  reachable.
- Every pre-update live PTY runtime is either directly owned by the new daemon
  or explicitly preserved by a reachable old owner.
- The active saved-session identity is still present in daemon live rows,
  sidebar rows, and app-control active-session truth.
- The active terminal can be reopened in Terminal mode.
- The active terminal is readable, interactive, and backed by `daemon_pty`
  content.
- Input is enabled only after the selected runtime is reachable and current.
- No user session is silently replaced by a fresh shell, placeholder, preview
  renderer, or metadata-only row.
- Titles, summaries, cwd placement, keep-alive flags, and sidebar projections
  still resolve through the same saved-session identity.

If any of these fail, the update is not hot. It is a failed handoff incident.

A remote Codex surface that contains only the sparse prompt/footer, for example
`› Write tests for @filename` plus the model/cwd line, is not hot-update ready.
It may be real PTY output, but it is missing enough current xterm/Codex state
to prove a usable restored session. The GUI must keep input gated, keep the
session in recovery, and after the hard-fail window may perform one controlled
force-remote restart. A plain remote shell prompt may settle as ready; this
Codex prompt-only rule is specific to Codex-like TUI sessions.

A preserved-owner Codex surface that contains generic Codex title-card chrome or
an internal Yggterm socket error such as `Error: connecting to
.../server-*.sock` is also not hot-update ready. That output means the handoff
route is stale or incomplete, not that the user's Codex session is interactive.
It must be classified as a rejected preserved-owner surface, keep input gated,
and may perform the same one controlled force-remote restart after recovery has
failed.

## Priority Order

Hot update priority is strict:

1. Preserve live PTYs.
2. Keep the GUI attached to the only reachable owner of each live PTY.
3. Keep terminal input disabled until the selected runtime is current and
   reachable.
4. Keep terminal content sourced from daemon PTY truth.
5. Preserve saved-session identity, cwd projection, title, summary, and
   keep-alive metadata.
6. Complete update bookkeeping.
7. Retire old daemons.

Update completion is lower priority than session survival. An old daemon that
owns a live PTY is not disposable. It is a preserved runtime owner until the new
daemon has adopted the PTY, a compatibility route has been proven, or the user
explicitly closes that session.

## Preflight Protocol

Before touching a live install or restarting a GUI:

- Snapshot `~/.yggterm/server-state*.json`, `session-titles.db`,
  `event-trace.jsonl`, install metadata, and relevant app-control state.
- Record the active GUI pid, executable path, app id, display/session, and
  version.
- Record the active daemon pid, executable path, version, socket, protocol
  version, and build id.
- List all reachable same-home daemons and identify which ones own live PTYs.
- Record active session path, saved-session identity, terminal I/O key, cwd,
  remote machine, live-session rows, keep-alive flags, and preserved-owner
  registry entries.
- Record baseline resource usage before starting the update test.
- Verify release artifacts from canonical metadata and checksums.
- Stage new binaries into their versioned install directory before changing the
  active launcher state.

Do not run archived, globbed, backup, or stale binaries against the live user
state. If an executable must be probed, use the active launcher or isolate it
with a temporary `HOME` and `YGGTERM_HOME`.

A daemon is current only when both its protocol version and executable identity
match the active install. A same-version daemon running from a deleted file,
backup copy, temporary stress path, archived direct-install path, or otherwise
non-current executable is stale. Read-only commands such as `server status` may
report that process as observed state, but mutating recovery commands must
reject it before ping/snapshot/app-control work and start or route through the
canonical current daemon instead. JSON-producing CLIs must keep JSON on stdout;
recovery warnings and lifecycle logs belong on stderr.

## Handoff State Machine

A direct-install update moves through these states:

1. `InstalledButNotActive`: new artifacts are staged and checksummed; launcher
   metadata has not yet disconnected the old runtime owner.
2. `RestartPending`: the current GUI/daemon has been asked to prepare for a
   restart and has written update intent.
3. `HandoffPrepare`: all current live runtimes are marked protected for this
   handoff, including non-Keep-Alive sessions.
4. `PreserveExistingRuntimeOwners`: old runtime-owner daemons keep PTYs alive.
   They must not be killed by cleanup.
5. `NewDaemonReady`: the new daemon is reachable and exposes protocol version,
   socket, pid, build id, and live-state load result.
6. `AdoptOrAliasRuntimes`: each pre-update runtime is adopted by the new daemon
   or routed through an explicit compatibility owner entry.
7. `GUIReattached`: the GUI active session, sidebar rows, and terminal host all
   point at the same saved-session identity and runtime route.
8. `Verified`: monitor, app-control state, screenshot, terminal probe, and
   resource sample all pass.
9. `RetireOldOwners`: only old daemons with no live PTYs and no required
   compatibility owner entries may be gracefully retired.

Skipping `Verified` is not allowed for critical clients. Retiring old owners
before `Verified` is a fatal update bug.

## Adoption And Compatibility

The preferred outcome is direct new-daemon ownership of every pre-update PTY.
When direct ownership is not possible, a compatibility route may proxy or alias
to the preserved owner, but it must be explicit and observable:

- The saved-session identity and terminal I/O key being routed.
- The old owner pid, version, socket, and executable path.
- The new daemon pid, version, socket, and protocol version.
- The retry/adoption state and last error.
- Whether input is allowed, and why.

Compatibility routing is a survival bridge, not a second source of truth. It
must not create duplicate live rows, duplicate title records, duplicate summary
records, or a second terminal content source.

## Retryable Handoff Errors

The following are retryable handoff errors, not permission to drop a session:

- `EAGAIN` or `Resource temporarily unavailable`.
- Timeout while reading a daemon response.
- Partial socket reads or writes.
- Temporary old-owner busy state.
- New daemon not yet ready.
- GUI not yet registered with app-control.

The update path must retry with bounded backoff or fall back to the preserved
old owner. It must not print internal bridge/adoption errors into the user's
terminal viewport as if they were PTY content. Internal errors belong in
notifications, incident state, logs, and app-control fields.

## Failure Handling

If the active session is missing from the new daemon:

- Do not synthesize a new shell under the same sidebar row.
- Do not mark the terminal ready from retained text.
- Do not clear metadata, title, summary, keep-alive flag, or cwd placement.
- Do not delete preserved-owner state.
- Keep the failed handoff visible as an incident state.
- Preserve old daemon sockets and owner pids until the session is recovered or
  the user approves cleanup.

If retained xterm text contains an update/bridge error:

- Treat it as a failed handoff.
- Gate input.
- Classify the visible text as internal error leakage, not PTY truth.
- Run monitor evidence before attempting restart, adoption, or cleanup.

If the old owner is gone and the PTY cannot be recovered:

- Preserve transcript/session metadata.
- Mark runtime loss explicitly with cause, timestamp, session identity,
  terminal I/O key, daemon versions, and attempted recovery steps.
- Never hide the loss by opening a replacement terminal.

## Fatal Hot-Update Violations

Any of these block publication:

- Active or Keep-Alive session disappears from daemon status, app rows, sidebar,
  or cwd projection after update.
- Active terminal shows an internal bridge/transport/update error as terminal
  viewport content.
- Input remains disabled after settle because retained text, stale recovery, or
  missing runtime truth gates the terminal.
- Old owner is killed before the new daemon owns or routes the PTY.
- New GUI is active but the selected terminal is not readable and interactive.
- A session exists only as metadata after update while the live runtime is gone.
- A fresh shell, preview renderer, or metadata card replaces the user's live
  PTY without explicit consent.
- Saved-session identity changes from transcript id to synthetic runtime key.
- Rename/title/summary/cwd placement is lost because live rows and stored rows
  diverged.

The string `hot update failed before bridging stale remote runtime ... Resource
temporarily unavailable` is an example of a fatal violation when it appears in
the terminal viewport for the active session.

## Observability Requirements

App-control, monitor, and trace output must expose enough data to prove the
protocol:

- Active GUI pid, version, executable path, display/session, and app-control
  client id.
- Active daemon pid, version, executable path, socket, protocol version, and
  build id.
- All same-home daemon endpoints and whether they own PTYs.
- Per-terminal saved-session identity, terminal I/O key, cwd, remote machine,
  runtime owner pid/version/socket, and preserved-owner status.
- Current handoff state.
- Active session path and selected sidebar/startpage row.
- Live-session rows and cwd projections.
- Bridge retry count, last bridge error, next retry deadline, and fallback
  owner.
- Input gate reason.
- Terminal content source.
- Visible internal-error classification.
- Resource sample after settle.

Trace events should include:

- `update_preflight_snapshot`
- `install_state_changed`
- `handoff_prepare`
- `runtime_owner_preserved`
- `runtime_adopt_attempt`
- `runtime_adopt_success`
- `runtime_adopt_failure`
- `runtime_alias_created`
- `gui_reattached`
- `handoff_verification_pass`
- `handoff_verification_fail`
- `old_owner_retired`
- `old_owner_kept`

## Verification Gate

A hot-update proof bundle must contain:

- Pre-update snapshot path.
- GUI pid and executable before and after update.
- Daemon pid/version/socket before and after update.
- Same-home server list.
- Panic-report monitor for the active session path.
- Latency check for all reachable endpoints.
- `server app state` for the active GUI.
- `server app screenshot` for the active GUI.
- The relevant terminal probe on a second X11 display or equivalent live GUI
  display.
- A real viewport typing smoke when the terminal is expected to be interactive.
- Resource baseline and post-settle resource sample.

The terminal proof bar is:

- Active session path matches selected terminal.
- `expected_path.listed == true` and `expected_path.terminal_keyed == true` for
  a reachable owner route.
- `input_enabled=true` only after runtime truth is current.
- `problem=null`.
- `geometry_problem=null`.
- `terminal_content_source=daemon_pty`.
- No internal bridge/transport/update error is visible in the terminal
  viewport.
- Screenshot shows real terminal content, visible cursor or current prompt row,
  and no blocking recovery toast.

State-only proof is insufficient for terminal correctness. Screenshot-only proof
is insufficient for runtime ownership.

## Smoke Tests Before Critical Clients

Before updating critical clients such as jojo, run the update protocol in an
isolated profile with disposable runtimes:

- Isolated `YGGTERM_HOME` update from previous patch to current patch.
- Local PTY session preservation.
- Remote PTY session preservation.
- Codex transcript identity preservation.
- Non-Keep-Alive live runtime preservation during update only.
- Keep-Alive live runtime preservation across app close and update.
- Forced stale-daemon scenario with a preserved owner.
- Forced same-version deleted-binary daemon scenario; mutating recovery must
  replace it with the current daemon while `server status` remains read-only and
  parseable JSON.
- Bridge fault injection for `EAGAIN`, partial read, timeout, and old-owner busy.
- Old owner cleanup only after new ownership/routing is verified.
- `/status` or equivalent real-viewport keyboard proof for interactive Codex
  sessions.
- Alt-screen TUI switch-away/background/switch-back proof.
- Autohidden titlebar hover proof that does not resize the terminal grid.
- Resource baseline and post-update fan/CPU budget proof.

A critical-client update is blocked until these smokes pass and their proof
paths are recorded.

## UX During Update

During a handoff, the GUI may show recovery status, but it must not lie:

- Use wording like `Preserving sessions while updating`.
- If adoption is pending, route to the old owner when possible.
- If adoption is failed, show explicit recovery state outside the terminal
  viewport.
- Never write internal protocol errors into xterm as terminal content.
- Never remove a session from the sidebar because the new daemon has not adopted
  it yet.
- If an old owner holds the session, show that preserved-owner state
  observably.

## Release Gate

A release candidate is not publishable if any hot-update proof shows:

- Active session missing from daemon live rows.
- Active session missing from terminal runtime keys.
- `active_terminal_surface.problem != null`.
- Retained error text used as terminal content.
- Input disabled after settle.
- Preserved owner killed before adoption.
- Sidebar identity/title/summary regression for the active session.
- Resource usage outside the recorded budget after settle.

The correct release action is to block publication, document the failure,
reproduce in an isolated profile, add a deterministic harness for the defect
class, fix the protocol path, and only then retry critical clients.

## 2.2.50 Jojo Incident Class

On 2026-05-13, updating jojo from 2.2.49 to 2.2.50 relaunched the GUI on
2.2.50 but did not preserve the active session as an interactive terminal:

- Active session:
  `remote-session://dev/019dfde8-d02a-7c23-a270-bab0539e7025`
- New GUI path:
  `/home/pi/.local/share/yggterm/direct/versions/2.2.50/yggterm`
- Observed viewport text:
  `hot update failed before bridging stale remote runtime ... Resource temporarily unavailable`
- Observed failure class: active session missing from the new daemon terminal
  keys, retained internal handoff error visible in xterm, and input disabled.

That is a protocol violation and release blocker. It must be used as a
regression class for the next implementation pass: partial bridge reads and
adoption failures must preserve or recover the old PTY owner instead of
stalling or losing the active session.
