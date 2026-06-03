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
- Plain terminal runtime: a generic local shell or SSH shell started from a cwd.
  It has a live PTY key and launch cwd, but no saved-session identity unless a
  future explicit save/pin feature creates one.
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
- Retained daemon scrollback must be preserved separately from the current
  screen. A cursor-safe current-screen replay may repair the visible grid, but
  it must not replace a longer retained PTY history or reset xterm to an empty
  scrollback buffer.
- Preview mode may render transcript JSONL, generated copy, and metadata, but it
  must never seed or repair Terminal-mode xterm content.
- Synthetic runtime keys are I/O keys only. They must not replace saved-session
  identity in sidebar rows, cwd projection, title/summary storage, restore, or
  deduplication.
- Durable sessions may appear in both `Live Sessions` and their cwd/machine
  group, but both rows must resolve to the same saved-session identity,
  metadata record, live runtime status, and keep-alive state.
- Plain terminal runtimes appear only in `Live Sessions`. Their cwd is used to
  launch the shell and to display context; it is not a saved workspace row,
  Startpage card, or title/summary identity.
- Keep Alive is a daemon retention request. It preserves the live PTY across GUI
  close/update handoff, but it does not make a plain terminal durable or decide
  cwd projection.
- App-control and screenshots are proof surfaces. If they disagree with daemon
  runtime truth, the disagreement is a bug to investigate, not an alternate
  source of truth.

The source-of-truth audit for these boundaries is
`docs/architecture-audit-2026-05-16.md`. Runtime and hot-update changes must
start from this ownership model. A probe may expose a mismatch, but only daemon
runtime truth and the handoff state machine may decide ownership, adoption,
retirement, or input readiness.

The stable shell-side owner for startup stale-daemon selection and hot-update
handoff priority is `crates/yggterm-shell/src/hot_update_policy.rs`. Launch
orchestration may execute the selected action, but it must not keep a second
copy of preserved-owner or stale-daemon selection rules.

## Session Close Definition

Closing a live session is a runtime operation, not a mode switch. For ordinary
row/session close, the daemon removes the selected PTY runtime and leaves saved
transcript identity, cwd projection, title, summary, and summary timeline intact.
This is true even when the row is marked Keep Alive: Keep Alive controls what
happens when the Yggterm window closes, not what the user's close button means.
The detach/preserve boundary is GUI close or update handoff, where kept
terminals stay available without terminating their PTY runtime.

Required close behavior:

- Closing a background live session must not change the active viewport.
- Closing a kept live session uses the same destructive runtime close semantics
  as any other live-session close. It must terminate/remove that selected
  runtime and clear preserved-owner entries for it. Closing the GUI is the
  action that detaches/preserves kept sessions.
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

The current daemon socket is part of session preservation. A daemon that still
owns a PTY runtime must keep a reachable socket for that runtime, or it must be
registered as a reachable preserved owner before a replacement daemon becomes
the current route. A process that still has a PTY child but has removed its
`server-<version>.sock` path is not preserving the session in a usable form.
That state is a failed handoff: the GUI may show a retained row, but terminal
read, write, resize, redraw, and restore cannot be trusted until a reachable
owner is restored or the saved session is explicitly resumed.

During daemon hot restart, the old daemon must close the listener, remove the
old socket path, and release the socket bind lock before spawning the
replacement process. Spawning while the old process still holds the lock creates
the worst split-brain shape: the old process owns PTY children but has no socket,
and the replacement exits with `bind_lock_busy`. Smoke tests must treat
`socket path missing + bind lock busy + live PTY child` as a handoff failure,
not as a preserved owner.

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

Terminal geometry is part of hot-update readiness. The xterm grid size, daemon
cached size, and kernel PTY size must converge before current remote Codex output
is accepted as settled. A daemon resize no-op is valid only when the kernel PTY
already reports the requested rows and columns. If the cache matches but the
kernel PTY is still at a bootstrap size such as `36x120`, the daemon must repair
the resize rather than letting Codex draw old-width line art into the restored
session.

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

The preserved-owner registry is evidence, not permission to kill. A process id
named by the preserved-owner registry is a session-survival root and cleanup
must not kill it merely because a newer daemon can describe the same runtime
key. Current-daemon exact-key coverage can retire only non-registry duplicate
owners; it is not proof that the original PTY has been adopted. If cleanup finds
a stale same-home daemon that directly reports an owned PTY runtime key missing
from the registry, cleanup must preserve that daemon and classify the missing
key as a handoff recovery incident.
If an old daemon can still answer status and its snapshot contains the runtime
as a live row, that runtime is running even when the replacement daemon's local
state was truncated. The replacement daemon must recover it as a temporary
update-restore row and register the old daemon as owner before considering any
cleanup.

Startup restore must not eagerly launch or resume every remembered live
terminal. Bulk remote prewarm is opt-in only because it can saturate the daemon
control plane, block app-control truth, and turn a session-preserving update
into a terminal storm. The active visible terminal may be restored on demand;
background rows stay metadata/live-row truth until opened or otherwise
explicitly requested.

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

Internal lifecycle, recovery, and stale-daemon messages must not be written into
PTY application output. If such text reaches xterm, that is a transport-boundary
incident. UI-side sanitizers may quarantine the symptom for safety, but they are
not the fix and must not be treated as the normal protocol path.

## Handoff State Machine

A direct-install update moves through these states:

1. `InstalledButNotActive`: new artifacts are staged and checksummed; launcher
   metadata has not yet disconnected the old runtime owner.
2. `RestartPending`: the current GUI/daemon has been asked to prepare for a
   restart and has written update intent.
3. `HandoffPrepare`: all current live runtimes are marked protected for this
   handoff, including non-Keep-Alive sessions.
4. `PreserveExistingRuntimeOwners`: old runtime-owner daemons keep PTYs alive.
   They must not be killed by cleanup, and each preserved owner must remain
   reachable through a socket. A PTY-owning process with no reachable socket is
   degraded state, not a preserved owner.
5. `NewDaemonReady`: the new daemon is reachable and exposes protocol version,
   socket, pid, build id, and live-state load result.
6. `AdoptOrAliasRuntimes`: each pre-update runtime is adopted by the new daemon
   or routed through an explicit compatibility owner entry.
7. `GUIReattached`: the GUI active session, sidebar rows, and terminal host all
   point at the same saved-session identity and runtime route.
8. `Verified`: monitor, app-control state, screenshot, terminal probe, and
   resource sample all pass. Session-switch proof must use the settled
   app-control open path (`server app open <path> --view terminal`) rather than
   desktop pointer automation or `terminal focus`, which only reclaims input for
   the already active terminal host.
9. `RetireOldOwners`: only old daemons with no live PTYs and no required
   compatibility owner entries may be gracefully retired.

Skipping `Verified` is not allowed for critical clients. Retiring old owners
before `Verified` is a fatal update bug.

Old-owner retirement must not use the session `Shutdown` request. `Shutdown`
is allowed to stop terminals and remote Codex sessions, so it is reserved for
user-requested or harness-owned teardown. Hot-update cleanup must use the
daemon-retire path, which exits only the stale daemon process after proving that
all runtime keys it reports are already owned by the current daemon. For older
daemons that do not understand daemon-retire, a process-level fallback is valid
only when the stale daemon reports zero owned runtime keys or every reported
runtime key is covered by the current daemon. That fallback must be observable
in monitor output and must never be used for the only reachable runtime owner.

Duplicate-owner pruning must not use the user-facing `RemoveSession` request.
`RemoveSession` is close-session semantics and may terminate a remote Codex
session before dropping local metadata. When the current daemon already owns a
runtime key that an older daemon also reports, the cleanup operation is
local-only: send `DropTerminalRuntime` to the stale owner for that runtime key,
then retire the stale daemon only after it reports zero owned PTYs. If the old
daemon still owns another unique runtime key, it remains a preserved owner for
that key only. It is not allowed to keep participating in hot update for keys
already owned by the current daemon.

The preserved-owner registry must not be pruned merely because the current
daemon reports the same runtime key. Key presence alone can be a duplicate
runtime, a partial handoff, or a damaged restore. While the current daemon owns a
running runtime for that exact key, terminal read/write/resize may use the
current daemon directly and bypass the preserved-owner route, but the registry
entry is removed only after the old owner has explicitly accepted the
local-only `DropTerminalRuntime` for that key or after the owner proves the key
is gone. This keeps the registry as survival evidence while preventing it from
outranking a verified current runtime in the hot byte path.

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

A preserved owner is valid only while it can actually answer terminal
read/write/resize for the runtime key. If a preserved-owner request fails with
`terminal session not found: <runtime-key>`, the current daemon must remove that
owner even if the owner's status payload still lists the key. The next terminal
read must fall back to current-daemon recovery: use the saved Codex transcript
resume path when available, or expose a clear degraded state when it is not.
Retained xterm text must not keep the surface marked writable after this
failure.

A current daemon terminal read has the same obligation. If the selected row is
Terminal mode but the current daemon has no running runtime for that
runtime-owned path, the daemon must recreate the runtime before returning
terminal bytes. An exited runtime whose only output is internal attach noise
such as `terminal session not found` is a failed bridge, not user-visible
terminal content. It may trigger current-daemon recovery; it must not keep
retained xterm text marked live or writable.

Terminal retained replay during adoption has the same single-source rule. If a
preserved owner can provide both retained PTY history and a current screen
snapshot, the GUI must seed xterm with history-as-scrollback followed by the
current screen. It may strip terminal control sequences from the history portion
only to avoid replaying stale cursor addressing into old rows; the current
screen portion remains daemon PTY-derived terminal content. A screen-only
snapshot is an emergency visible-grid fallback, not proof that scrollback
survived.

A later short `terminal_read` from the same compatibility owner is a
current-screen refresh, not a replacement scrollback source. It must not reset
the xterm buffer seeded from retained history or collapse `base_y` back to
zero. Conversely, a later retained-history replay must not overwrite a fresh
interactive read. Retained history may stage scrollback, but the final
input-enabled source for a remote interactive terminal must be a fresh PTY/read
source such as `daemon_terminal_read` or `daemon_pty`, not
`daemon_retained_history_screen_snapshot`. App-control proof for a retained
adoption must show `base_y > 0`, a moving `probe-scroll`, no input-enabled
retained-history final source, and no duplicate replay writers racing the same
xterm mount.

If a short `daemon_terminal_read` wins the first paint race before the retained
history replay, `CollapsedScrollbackRecovery` may still seed the first retained
history snapshot for that mount while user input is still gated. This is not a
second terminal truth: the bytes still come from the current daemon/preserved
owner path and exist only to restore xterm scrollback before the live PTY stream
is declared interactive. The exception ends as soon as retained history has
already been staged, input is enabled, or user input becomes hot.

The remote resume gate must not depend on app-control polling. Once xterm is
mounted, geometry is usable, and the GUI has observed meaningful current daemon
PTY output for the selected runtime, the product render loop must have enough
truth to clear `terminal_attach_in_flight`, remove the resume notification, and
record the terminal-open attempt as ready. Window focus may still decide whether
keystrokes are accepted immediately, but focus outside the terminal is not a
reason to keep the session behind `Restoring Remote Terminal`.

Retained-fault recovery follows the same rule. A non-prompt snapshot from a
recovering remote runtime is diagnostic evidence only. It must not reset the
xterm buffer, mark the open attempt ready, or enable input. The selected
runtime's live PTY stream must prove the prompt-ready surface. App-control must
also reject a focused or input-enabled terminal host whose `session_path` does
not match the selected `active_session_path`; that is an identity split, not a
healthy retained surface.

Startup prewarm and focus are part of the same contract. Before either path is
allowed to spawn a remote `resume-codex` command for an explicit Keep-Alive or
temporary update-restored remote runtime, the new daemon must scan reachable
same-home daemon owners for that runtime key. If an old owner still reports the
runtime, the new daemon must record a preserved-owner route and use it. A
missing handoff-cache entry is not permission to spawn a duplicate remote Codex
process.

When a daemon that already routes through older preserved owners prepares the
next handoff, it must write only the PTY runtime keys it directly owns as
handoff keys for itself. Reachable prior-owner entries remain attached to their
actual owner endpoint unless the outgoing daemon has taken over that exact
runtime. A handoff writer must never rewrite an older live PTY owner to itself
merely because the current daemon represented the row through a preserved-owner
route.

Daemon startup must not clear `hot-update-terminal-owners.json` merely because
the file was written for a previous patch. The daemon must first restore
persisted live-session state, retarget the surviving owner registry to the
current version, repair any registry entries whose endpoint is only a filesystem
alias of the current daemon, recover missing live rows from reachable preserved
owners, and only then prune entries that are neither represented nor reported by
a real old owner. A version mismatch in the handoff cache is not evidence that
the PTY is disposable.

Versioned socket paths are not owner identity. On Unix, preserved-owner endpoint
comparison must use the canonical socket target, not just the path string. A
compatibility symlink such as `server-2-1-0.sock -> server-2-7-19.sock` is the
current daemon and must not be used as a preserved owner. If another reachable
daemon reports the runtime key, startup must retarget that registry entry to
the real owner endpoint. If no real owner reports the key, the entry is
diagnostic evidence only and must not proxy read/write/resize back into the
current daemon as if it were an old owner.

If persisted live-session state has already been damaged or truncated during an
incomplete update, the preserved-owner registry is still a session-survival
signal. Before filtering runtime truth, the replacement daemon must query the
registered preserved owner, recover any matching live-session rows from that
owner's daemon snapshot, and only then decide whether the runtime is
represented. A reachable owner reporting the runtime key outranks a missing row
in the current daemon's local session tree; otherwise the GUI can demote a real
live PTY into a saved-session preview and spawn the same nondeterministic
"blank/new session" failure under a different patch version.

If the registry itself was already truncated, startup may scan reachable
same-home daemons for directly owned terminal keys, but it may recover only
snapshot rows marked Keep-Alive or temporary update-restored. This scan is
session survival, not ghost resurrection. Unkept rows without the update-restore
marker must stay closed even if an old process still reports a terminal key.

Preserved-owner `TerminalRead` is a hot-loop byte path. It must proxy PTY bytes
directly to the owner and must not perform saved-session mismatch snapshots or
semantic identity probes on every poll. Those checks belong to ensure/snapshot
recovery paths where they are observable and bounded.

Saved-session mismatch heuristics are not destructive authority during direct
update. If a live row carries the temporary update-restore marker, a mismatch
between early preserved-owner screen text and the saved Codex identity is
recovery evidence only; it must not remove the preserved owner or spawn a fresh
remote resume before the handoff has either verified or explicitly failed.

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
- During startup reconciliation, persisted live-session state is the
  authoritative allow-list for which old daemon-owned PTYs may be preserved.
  `hot-update-terminal-owners.json` is a handoff cache and may be stale; it may
  be used only when persisted live-session state has no runtime keys.
- For an explicit Keep-Alive or temporary update-restored remote runtime that
  is still running, stale, mismatched, prompt-only, blank-after-grace, or
  spec-mismatched early output is not permission to restart the transport or
  spawn another resume command under the same session label. Those conditions
  must keep the surface in recovery, gate input, and remain observable. After
  one minute of not reaching a readable, interactive surface, the GUI may issue
  one non-destructive careful-restore request against the daemon. That request
  may reattach, resize, or refresh the same runtime, but while the runtime is
  still running it must not kill the process or spawn a duplicate resume
  command. Restart is allowed only after the runtime process is gone or after
  an explicit user/harness-owned force-restart action. This applies equally to
  direct current-daemon runtimes and compatibility routes through preserved old
  owners.

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
- `foreground_input_ready=true` only after runtime truth is current.
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
