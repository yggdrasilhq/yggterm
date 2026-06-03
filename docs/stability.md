# Yggterm Stability Contract

Yggterm feature work is frozen until the session/runtime model is stable enough to field test without repeating the same GUI failures. This document is the working contract for that stabilization pass.

## Current Diagnosis

The repeated bugs come from ownership ambiguity. The same visible session is currently described by persisted browser rows, daemon live-session rows, remote scan records, retained xterm hosts, active shell selection, preview copy jobs, and restore/reconnect paths. When those surfaces disagree, the app can show one row, render another session, regenerate copy for the wrong target, or expose destructive wording for a runtime close.

The fix is not a larger feature. The fix is to make invalid state impossible or at least immediately visible to tests and app-control.

## Minimal Terminal Promise

The core product is intentionally small: a Yggterm session automates the repeated routine of connecting to a machine, entering a working directory, and resuming the same terminal task, for example:

```bash
ssh dev
cd gh/yggterm
codex resume <uuid>
```

Everything else is supporting machinery. Sidebar rows, titles, summaries, keep-alive flags, hot-update handoff, screenshots, and app-control probes exist to make that routine durable and observable. They must not become alternate renderers, alternate input targets, or alternate session identities.

The hot-update handoff contract is specified in `docs/protocol.md`. A version update that stalls, hides, replaces, or loses the active PTY is a release-blocking protocol violation, not a successful hot update.

The remote careful-restore window is one minute. Before that deadline the shell
may show a slow/restore notification and run normal attach recovery, but it must
not decide that a running protected runtime is disposable. At the one-minute
mark, a protected Keep Alive or temporary update-restored runtime gets a
non-destructive restore request only: reattach, resize, refresh, and keep input
gated until the same PTY is readable. Killing the process or spawning another
`codex resume` under the same label is allowed only when daemon truth says the
runtime is gone, or when a user/harness explicitly asks for a force restart.

Switching a live session should feel local. If daemon truth already contains a
live runtime and the retained xterm host is valid, row selection should display
the cached terminal surface immediately while attach/readiness runs in the
background. SSH latency is only part of the path when Yggterm has to create,
recover, or reattach a missing remote runtime. A slow switch is therefore not
"just SSH"; it usually means the shell entered one of these gates: daemon
snapshot/status, remote ensure, retained replay validation, xterm remount,
resize/geometry settle, input focus/readiness, or recovery after a stale
preserved-owner handoff. `scripts/smoke_ui_latency.py` is the proof path for
this contract.

Attach/recovery gates are foreground controller state. A retained live runtime
may exist in the background, but an old `terminal_attach_in_flight` entry for a
background row must not keep future switches or unrelated sessions behind a
restore notification. Switching focus prunes background attach/bootstrap leases;
the daemon runtime and retained-session identity remain untouched.

Collapsed retained scrollback is a diagnostic unless it prevents the current
prompt from being readable and input-ready. A remote terminal with a visible
current prompt, mounted xterm surface, and enabled input should become
interactive immediately. App-control must still expose `scrollback_expected`,
`base_y`, and probe-scroll results so lost history can be debugged, and explicit
wheel/scroll failures remain hard terminal problems.

For terminal rendering bugs, the repair order is strict:

1. Prove the selected session identity and daemon runtime key.
2. Prove the PTY byte stream or retained daemon scrollback for that same runtime.
3. Prove xterm.js buffer, renderer mode, theme mapping, fit/resize state, scroll intent, cursor position, and input focus.
4. Only then change shell code.

Do not fix prompt background, cursor visibility, resize redraw, Codex `Working` animation, typed echo, or scrollback bugs by drawing independent shell UI over the terminal merely to satisfy a screenshot. A Yggterm-owned overlay is acceptable only as a narrow compatibility shim when the PTY/xterm truth remains authoritative, app-control exposes the shim explicitly, and the smoke test still proves real terminal input/output behavior. If the overlay becomes the only reason the viewport looks correct, the terminal is still broken.

## Single-Source Surface Contract

Yggterm has two major session surfaces, and they intentionally have different truth sources.

Terminal mode is a runtime attachment. It is analogous to opening Ghostty, SSHing to a machine, and attaching to a durable `screen`/`tmux` session. The user-visible xterm viewport must be fed by exactly one source: daemon-owned PTY bytes for the selected runtime, plus daemon-owned retained scrollback derived from those same bytes. If the runtime is unavailable, Terminal mode may show a runtime error. It may not fall back to preview text.

Preview/Web View mode is session presentation. It is read-only inspection by
default, shaped like a chat transcript or document view. It may render stored
Codex JSONL, generated summaries, `USER`/`ASSISTANT` message blocks,
timestamps, and other display copy. That data is useful for inspection, but it
is not terminal output.

In chat layout, transcript turns come first. Generated goals, summaries,
rendered context, and other secondary sections can enrich the reader, but they
must sit below the actual transcript turns and must not be presented as the
primary conversation.

Readable remote scan previews are valid Web View presentation data. They should
render immediately instead of being hidden behind a loading gate. If the live
session metadata includes a saved transcript `Storage` path, the shell may fetch
and hydrate a bounded recent JSONL transcript window in the background; that
hydration updates Web View presentation only and must not seed, repair, restart,
or otherwise mutate the Terminal-mode PTY runtime.

Remote transcript hydration for the active Web View must stay bounded for
interactive use. A large transcript can be tens of megabytes and must not be
sent as a normal daemon snapshot just to make the reader usable. The active Web
View should request a recent transcript window, keep any readable fallback
visible while it arrives, and avoid loading toasts that block selection or
reading. Legacy or Terminal-mode preview refresh remains cache-only so SSH fetch
latency cannot enter restore or typing paths. Promoting a stored remote Web View
back to Terminal must also promote the row into live-session order before
focusing it.
If a daemon snapshot contains both a hydrated `active_session` and a matching
shallow `live_sessions` row, the shell must use the hydrated `active_session`
for Preview/Web View and the live row for Terminal. Those rows are two read
models over one identity, not two interchangeable sources of truth.
For remote Codex Web View, a mounted recent-tail transcript window outranks
older head, scan, loading, or empty projections. Refreshes may improve the
reader with a fresher tail, but they must not downgrade a readable active Web
View to the start of an old transcript or to generated context.

Retained terminal switching should be visually quiet when the existing xterm
surface has ready history or meaningful visible output. Restore notifications
are observers for genuinely slow or failed recovery, not normal switching gates;
they must not cover a readable retained terminal while final interactive-ready
observation settles.
The careful restore timeout is not fatal while the PTY stream is still making
recent progress. Protocol-only terminal handshakes are not readiness proof, but
they are evidence that the attach path is alive; a timeout in that window should
stay in recovering/slow state and let the terminal bootstrap loop reach the next
meaningful-output or hard-failure decision.

KDE/Linux live-session retention keeps daemon PTYs and live rows as the durable
truth, but it must not keep every previously visited xterm host mounted. Hidden
terminal renderers are caches only. The stable default is active-host retention:
the active terminal stays mounted, inactive live sessions remount from daemon
PTY/retained history on switch, and resource telemetry should not grow linearly
with the number of visited live sessions.

The Web View UI is a modular conversation surface. Each mounted conversation
declares a provider and capability set before rendering: transcript providers
such as Codex and terminal scrollback are read-only, while future API-backed
providers such as OpenWebUI or the SAMPLENOTES webapp may opt into `send` capability.
Capability declarations do not change terminal truth. A read-only transcript
provider must never mount a composer or route input to a PTY, and an interactive
API provider must send through its provider API rather than by typing hidden
shell text into xterm.

Live runtime presence proves that Terminal mode is available; it does not force
the active viewport to stay in Terminal mode. A user may switch a live session
to Web View to read transcript/provider content while the daemon-owned PTY
continues in the background. Switching back to Terminal reattaches to the same
runtime key. Web View may not terminate, detach, restart, or otherwise mutate the
runtime.

The forbidden bridge is the important part:

- Preview/transcript blocks must never seed, repair, or replace a Terminal-mode xterm buffer.
- Live-session Web View may inspect saved transcript/provider data, but it must
  leave the daemon runtime table and preserved-owner routing untouched.
- Web View conversation providers must not infer write capability from session
  kind, row title, or terminal focus. Write capability is an explicit provider
  contract.
- Codex cards, `/status` panels, model banners, prompt examples, or weekly-limit strings are not correctness contracts.
- The active shell's display copy must never outrank the daemon's current runtime stream for Terminal mode.
- A retained xterm host is a cache keyed by runtime identity and stream epoch. When it is stale or blank, rebuild from daemon runtime bytes, not from presentation data.
- App-control must expose enough source metadata to prove which surface is being rendered, for example `terminal_source=runtime_stream` or `preview_source=presentation`.

This is the core determinism rule: Terminal tests should use fake PTY streams, sequence numbers, and visible byte markers. Preview tests should use stored transcript/presentation fixtures. They should not assert on live Codex wording.

## Feature Freeze Rules

- No new user-facing terminal/session feature work until the stability gates below pass on Linux/KDE, Windows, and macOS.
- Selection is allowed to change focus. It must not regenerate title/summary, relaunch a runtime, or switch terminal/preview mode unless the user action explicitly requested that side effect.
- Passive title/precis/summary generation is enabled by default only through the bounded background bookkeeping loop. Selection may hydrate already-cached copy, but it must not start LLM work. The app-control `generation.copy_generation_start_count`, in-flight path arrays, and passive-copy suspension flags are the proof surface for this contract.
- Title, precis, and summary are display copy only. They are never identity and never decide which runtime receives input.
- Live Sessions are daemon-owned runtimes. Closing one kills that runtime and removes it from the Live Sessions group. It must not imply stored transcript deletion unless the user requested a hard delete.
- Fresh live terminals are runtime-only by default. They are restored across normal app close only after the user explicitly marks them `Keep Alive`; clearing keep-alive must remove them from persisted live-session state without killing the currently running terminal.
- Keep Alive is not a shield against the row close action. Closing a live row means terminate/remove that selected runtime; closing the Yggterm window is the detach/preserve path for kept live sessions.
- Remote live sessions are visible in `Live Sessions` and, when their cwd is known, are also projected under that remote cwd folder so folder-scoped work remains findable while the runtime is live. Keep Alive controls daemon retention/durability only; it must not decide whether a live remote session appears under its cwd. Local historical transcript rows still must not become duplicate stored-tree rows just because a runtime exists.
- Remote Codex saved-session truth requires a real transcript storage path. A fresh `start-codex` runtime that is still in onboarding, permission setup, or any other pre-transcript state is a live runtime only. Closing it removes the runtime; it must not create a phantom stored row that later fails with `no terminal spec for session`.
- Codex onboarding, authentication, and setup menus are pre-transcript but
  input-ready. Resume/loading gates must clear when the xterm buffer shows one
  of those explicit menus; blocking input until a normal prompt appears turns a
  correct fresh Codex start into a locked terminal. The classifier must also
  accept xterm tails that start mid-menu after logo art, because viewport
  sampling can expose `tGPT ... Device Code ... API key ... Press enter to
  continue` instead of the full welcome header.
- Startpage recent-work cards are durable saved-session truth only. Runtime
  projections from `Live Sessions` and storage-less remote Codex rows may appear
  in the sidebar tree, but they must not become Startpage session cards or UUID
  fallbacks until a real saved-session identity exists.
- When a live remote runtime is projected in both `Live Sessions` and its machine/cwd folder, dragging one visual row must not make the other visual row appear to drag. The duplicate projection is a read model over the same runtime, not two separate drag sources.
- Remote cwd bookmarks created by `Add Folder` are local metadata rows with a synthetic remote-folder path, but they must render only inside the owning remote machine tree. They must not appear as local filesystem rows, and remote scans must not be required before the saved bookmark is visible.
- The sidebar merge has two inputs for saved workspace metadata: the currently
  visible local rows for local tree rendering, and the complete saved row model
  for remote cwd bookmark projection. Remote `Add Folder` must never depend on
  a hidden local `/__remote_folder__/...` implementation branch being expanded.
- Remote cwd bookmark rename and launch are one contract. The synthetic storage
  path is the durable cwd bookmark; renaming a newly added folder to a relative
  path such as `git/samplers` moves the bookmark under the selected remote
  cwd, and Startpage `New Codex Session` / `New Terminal` must launch with that
  resolved remote cwd rather than the previous active session's cwd.
- Expandable sidebar rows have a split hit-zone contract. Clicking the visible row name selects the row and opens that group's scoped Startpage without closing live runtimes; clicking the icon, disclosure/count control, machine/live-session affordance area, or trailing empty row surface toggles expansion. This applies uniformly to cwd folders, machine groups, and `Live Sessions`, with expansion state still keyed only by the row path.
- Sidebar scroll position is presentation state, not tree truth. When rows shrink
  after launch, refresh, search, or machine expansion changes, the visible
  scroller must clamp stale offsets back inside its current bounds. A tree whose
  rows fit in the sidebar must have `sidebar_scroll_top == 0`; top rows becoming
  clipped until another expansion forces a scrollbar is a layout regression.
- Normal final-client close must notify the user, remove non-Keep-Alive live rows from durable restore state, send graceful runtime shutdown, and schedule force cleanup after one hour.
- Update restart is different from Keep Alive. Before a direct-install restart, the daemon must persist every recoverable live runtime with a temporary update-restore marker only when current runtime truth still says that unkept runtime exists. That marker allows the next daemon to restore the session once, but it must not silently convert unkept terminals into durable Keep Alive sessions or serialize stale in-memory rows that no longer have a daemon runtime key. After a fresh remote scan reports that an unkept temporary remote runtime is not live, that row must leave `Live Sessions` instead of remaining as a degraded/loading recovery target.
- A hot-update preserved-owner registry is a terminal I/O handoff map, not durable session truth. Persisted live-session state is the startup allow-list when it contains runtime keys; `hot-update-terminal-owners.json` is only a fallback when persisted live state is empty. Before filtering runtime truth, a replacement daemon must query reachable old owners: a directly owned runtime that still appears in the old owner's daemon snapshot is running and must be recovered as a temporary update-restore row instead of being killed as unrepresented. During handoff, every represented `terminal_session_key` must be written or retargeted to the current outgoing handoff daemon endpoint; chained older sidecar entries must not become direct owner truth for the replacement daemon. Closing a live session or clearing Keep Alive must remove that key from the preserved-owner map. Daemon load must restore persisted live state before judging a registry version mismatch, prune only unrepresented entries from `hot-update-terminal-owners.json`, and retarget surviving entries to the current version instead of carrying latent old runtimes forward or wiping still-represented owners.
- Daemon boot has a stricter ordering than preserved-owner cleanup. The replacement daemon must bind and answer on its current endpoint before any deep cross-daemon snapshot, recovery, or prune work. A stale or busy old owner can delay reconciliation, but it must never leave the updated GUI waiting on a missing current socket.
- A temporary update-restored live session has the same survival priority as an explicit Keep Alive session until the handoff verifies or fails. Early saved-session mismatch text from a preserved owner may gate input or keep recovery visible, but it must not detach/remove the owner entry or spawn a duplicate remote resume while the temporary update-restore marker is present.
- Startup reconciliation must prefer the active/default preserved-only sidecar daemon for retargeting over older orphaned PTY owners. An older same-patch-line daemon should be selected for hot-update handoff only when it actually owns terminal runtimes that the active sidecar does not already represent and those owned runtime keys are authorized by the current preserved-owner registry or persisted live-session state. A ghost-owned runtime for a closed session is not a session-survival reason.
- Daemon cleanup is home-scoped. An app may reap same-home duplicate, legacy, or orphan daemons, but it must not signal a daemon from another `YGGTERM_HOME`, and it must not reap a legacy daemon that still has registered GUI clients in that daemon's exact endpoint scope or is the current hot-update PTY owner endpoint. App-control may scan legacy client-instance scopes for handoff discovery, but cleanup must not treat the replacement GUI as a client for every stale versioned daemon. During startup cleanup, the newest preserved-only sidecar whose terminal keys are exactly authorized by the current owner registry must remain available as the retarget bridge; older preserved-only sidecars with `owned_terminal_session_count == 0` must not be protected just because the same home has recoverable runtime activity elsewhere.
- Multi-version daemon discovery is read-only observability, not an attach target. A current remote client may list stale versioned daemons for incident reports, but it must not bridge a live terminal through a daemon whose `server_version` differs from the current protocol version.
- Stored sessions and remote scanned sessions open as preview unless an explicit terminal launch/resume action promotes them to a live runtime.
- Remote scanned sessions may appear in Terminal mode only when the remote scan says the runtime is live and the active session source is `LiveSsh`.
- A retained terminal host may stay mounted only if its session identity still matches a live session or a deliberate recovery state.
- Preview mode is read-only by default. Switching preview/terminal may not rewrite the session title, summary, identity, or runtime target.
- Terminal mode and Preview mode must not repair each other. Preview can inspect the transcript of a runtime; Terminal can attach to a runtime. Neither surface is a fallback renderer for the other.
- Saved transcript/context fallback is readable Preview content. Remote hydration
  may show a small background status while that fallback is visible, but it must
  not replace the reader with a blocking loading or failure gate unless there is
  no readable Preview content at all. Once no preview request or failure retry is
  pending, that readable fallback also clears the toolbar loading state.
- Clipboard paste is an owned runtime operation. `Ctrl+V`/`Cmd+V` must route through the native clipboard reader so images can be staged locally or through the remote Yggterm helper, and text can still paste normally.
- Terminal paste gestures must be single-owner. Once the active xterm host claims
  a paste event, browser default handling and xterm's browser paste path must be
  stopped, and duplicate paste events from WebKit/portal/remote-desktop stacks
  must be suppressed for that gesture. App-control and terminal telemetry expose
  request and duplicate counters; neither surface may include clipboard text.
- Terminal input, scroll, focus, and retained-host recovery are one controller. A terminal that only scrolls, only types, or loses scrollback while composing input is an invalid user-visible state.
- An unfocused Yggterm window must cool the GUI terminal bridge even if a live
  session remains the active row. Daemon PTY output can accumulate as runtime
  truth; the GUI must not keep reading and repainting xterm frames at active
  cadence until the window is focused again or a deliberate app-control attach
  recovery is in flight.
- Terminal selection copy must not use the browser Clipboard API. `Ctrl+Shift+C`
  and `Ctrl+Shift+X` route xterm selection text through the Rust terminal event
  bridge into a native clipboard owner thread; the WebKit renderer must stay out
  of `navigator.clipboard.writeText` so Remmina/portal clipboard hangs cannot
  freeze the shell.
- Context menus and destructive runtime affordances must use theme primitives. Hard-coded light-mode menus or live-session close buttons are regressions.
- Chrome/titlebar settings changes must be transactional and non-blocking. Persisting a toggle may not freeze the UI thread.
- Codex-class sessions must expose semantic running/completed state, with notification and optional sound when work completes.
- Terminal recipes remain experimental. Ordinary drag/drop or row movement must not create recipes unless an explicit development flag enables that path.

## Executable Invariants

`validate_server_ui_snapshot` in `crates/yggterm-server/src/lib.rs` is the first executable contract. A server UI snapshot is invalid when:

- `active_session_path` and `active_session.session_path` disagree.
- Terminal mode is active without an active session.
- Terminal mode is active for a stored/non-live session, except document terminal recipes.
- A remote scanned terminal session is not backed by a `LiveSsh` session and a remote scan `live_runtime == true`.
- `live_sessions` contains a historical `Stored` session, document node, duplicate path, or remote scanned row that the scan does not mark live. A `LiveLocal` Codex/LiteLLM runtime may still use a stored transcript path; in that case it belongs in `Live Sessions`. A remote live runtime may have a second visible sidebar row under its cwd folder, kept or unkept, but the server snapshot still has one `live_sessions` entry.
- An active live session is missing from `live_sessions`.

These checks should move closer to the reducer/state transitions over the next passes. For now they are intentionally snapshot-level so both unit tests and GUI smoke tests can catch cross-layer disagreement.

The shell also exposes a copy-generation budget contract through `server app state`: `generation.implicit_copy_generation_enabled`, `generation.copy_generation_start_count`, and the title/precis/summary in-flight path arrays. Opening or selecting a row without an explicit regenerate action must leave the start counter unchanged.

Terminal source ownership is an app-control contract. A Terminal-mode active host must report a runtime-backed source, a matching active session path, input routing to that same runtime, and a visible stream sequence or fingerprint that agrees with the daemon's current runtime snapshot. A Terminal-mode host that contains `USER:`/`ASSISTANT:` transcript labels, saved JSONL preview text, generated summaries, or other presentation-only blocks is invalid even if `foreground_input_ready=true`.

Terminal focus ownership is the matching foreground contract. A retained host from a different session may temporarily expose stale helper-textarea focus after a switch or window blur, but it is not foreground truth unless it is the active session host or still reports enabled/raw input. App-control must reject a different-session host with `input_enabled=true` or `raw_input_enabled=true`, and it must not fail the selected session just because an inactive retained host has stale DOM focus while user input is disabled.

Preview source ownership is the mirror contract. Preview mode should expose presentation metadata and chat/document blocks, and it should not mount an input-enabled xterm host for the same surface unless the user explicitly switches to Terminal mode.

Inline rename is also part of the observability contract. While rename mode is active, `server app state` must expose the controlled `shell.tree_rename_value`; when DOM snapshots degrade under KDE load, the action fallback should still expose `dom.tree_rename_input_value` for the visible input or leave the shell value available for smoke assertions.

Titlebar search has the same proof requirement. When `shell.search_query` is non-empty or `shell.search_focused` is true, a degraded DOM snapshot must still expose the active search input rect and focused input value so the slow-typing regression cannot hide behind app-control timeouts.

Update restart protection is observable through persisted daemon state. A normal persisted snapshot may contain only explicit Keep Alive live sessions. A pre-update persisted snapshot must contain all recoverable live sessions and must mark non-Keep-Alive sessions as temporary update restores. Remote scan reconciliation must also emit `server/remote_machine prune_temporary_stale_live_sessions` when it removes temporary update-restored remote rows whose scanned session no longer has `live_runtime=true`.

Hot-update preserved-owner state is a survival bridge, not a second runtime
truth. If the updated daemon directly owns the same terminal runtime key as an
older daemon, the older daemon must drop that duplicate key instead of writing a
new `hot-update-terminal-owners.json` entry. If the old daemon owns a mix of
duplicated and unique preserved PTYs, only the duplicated keys are retired; the
unique PTYs stay alive. Preserved/runtime-known keys are not adoption proof; the
old owner must stay alive until the updated daemon owns the PTYs or the sessions
are deliberately restarted. `server monitor --scenario server-list` should
converge back to no duplicate direct ownership after that direct ownership
condition is true.

During this preserved-owner interval, short current-screen reads are allowed as
screen refreshes only. They must not replace a longer retained PTY snapshot as
the scrollback source for a restored xterm surface. A healthy retained restore
therefore has two layers of proof: server-list shows the runtime key routed
through the preserved owner, and app-control `probe-scroll` shows non-zero
`base_y` with actual viewport movement.

The preserved-owner registry is endpoint-specific. If daemon A is the registered
preserved owner for one runtime and daemon B is the registered preserved owner
or current direct owner for another runtime, daemon A must not keep a duplicate
copy of daemon B's runtime. The 23-smoke server-list gate treats the same runtime
key appearing in multiple daemons' `owned_terminal_session_keys` as a
release-blocking failure. Duplicate-owner pruning must never probe the current
daemon or a legacy socket alias that resolves to the current daemon while it is
already handling a request; probing self is a daemon-loop deadlock.

Native paste is observable through terminal events and app-control paste commands. A browser `Ctrl+V`/`Cmd+V` must emit the native paste request instead of relying on xterm.js to guess clipboard contents.

Terminal typing proof is a viewport contract. Smokes that claim user-facing typing behavior should use `probe-type --mode keyboard --per-char` and require `visible_echo_observed=true` plus bounded `timings.visible_echo_ms`. In canvas renderer mode the proof must come from the xterm buffer/cursor sample, not `host.innerText`, because DOM rows are absent by design. `--per-char` dispatches character-level keyboard events without artificial per-character sleeps; if it reports slow echo, treat that as app/input-path latency rather than probe pacing. App-control direct PTY sends may prepare state, but interrupt bytes are split from following command bytes so prompt recovery cannot hide a dropped first character. For Codex-class live smoke tests, prefer a non-submitted marker echo plus clear-line proof over `/status` output; `/status` text is Codex UI and is not deterministic enough for CI.

Latency is also a smoke-test contract. `scripts/smoke_ui_latency.py` measures state, rows, search, right-panel, and active terminal input latency against app-control budgets. Before typing, it rejects the blank-host failure class by requiring the active terminal to be rendered, interactive, out of `terminal_attach_in_flight`, backed by a mounted xterm viewport, and input-enabled. In read-only drawing mode it records root-render churn, browser rebuild churn, combined GUI/WebKit CPU, and top per-thread CPU so a GUI reactive loop is caught even when xterm write/render counters are idle. Read-only drawing proof may run while the desktop window is unfocused, such as after a Remmina restart; in that case it still requires the xterm viewport and drawing surface, but it must not fail only because the input gate is closed by window focus policy. App-control wakeups must be worker-aware: a request targeted at a different live GUI PID must not wake this client or schedule root renders. A degraded `dom.snapshot_mode == "terminal-fallback"` state is acceptable for live terminal proof only when the fallback is bounded within the state budget and still carries active terminal geometry, canvas counters, retained replay prompt-follow fields, and viewport-force diagnostics; sidebar claims must pair it with `server app rows`. Use `--clear-after` for live terminal probes so the smoke clears the prompt before and after short marker samples, preventing line wrapping from hiding an otherwise visible echo. Use it for live incident reports and CI-style regressions instead of relying on subjective typing feel alone.
The default budgets are tuned for live SSH-driven app-control proof: 1200 ms for state/rows/search/panel command round trips, 500 ms for any individual terminal visible echo, and 450 ms for terminal visible-echo p95. Tighten those flags for local CI runs that do not include SSH/process-start overhead.

Sidebar busy indicators are part of the idle/fan contract too. Live row and tree
busy marks must not run infinite CSS animations on the stable channel; on
Wayland-over-Remmina and similar remote desktops, even tiny WebKit animations can
keep the compositor hot while the terminal is idle. App-control screenshots may
show static busy marks, but `dom.css_running_animation_count` should decay to
zero once modal/probe activity settles.

Canvas renderer idle state is part of the same contract. App-control must expose `visible_canvas_layer_count`, `hidden_canvas_layer_count`, `software_canvas_layer_optimization_active`, and `software_canvas_cursor_overlay_*` without making the cheap/basic snapshot sample canvas pixels. A canvas-rendered idle terminal should hide inactive full-viewport selection/link/cursor layers and use the small Yggterm-owned cursor overlay, so read-only CPU smokes must fail when no active terminal host is mounted, when software layer optimization is inactive, or when more than two full-viewport canvas layers remain visible.

Linux backend selection is part of the idle/fan contract. On KDE sessions that expose both Wayland and Xwayland, the default GUI path must force the native Wayland toolkit backend (`GDK_BACKEND=wayland`, `WINIT_UNIX_BACKEND=wayland`) unless the user explicitly sets a backend or `YGGTERM_FORCE_X11_BACKEND=1`. The vendored Dioxus Wayland DMA-BUF workaround may disable DMA-BUF, but it must not overwrite a backend that Yggterm already selected. The trace should show `linux_desktop_backend_policy.policy == "kde_wayland_native_default"` with `xterm_canvas_policy == "xterm_canvas_enabled_for_wayland"`; a canvas-mounted xterm running under an X11 WebKit child is a renderer-policy mismatch and should be treated as an idle CPU regression.

Remote terminal recovery must make terminal-open truth converge. When a remote resume times out, the matching `terminal_attach_in_flight` entry, bootstrap lease, and terminal surface request must clear, and the open-attempt ledger must latch a failure. A stuck notification may explain a failure, but it must not keep the UI in a permanent loading state or drive an infinite render loop.

Local startup restore has the same convergence requirement. If a local startup-restore attempt stays pending or recovering past the recovery window, a same-session terminal surface request or nonzero open request id must not block recovery; the shell must clear the stale attach lease and retry the mount instead of leaving a blank xterm host and high render churn.

## Stability Gates

1. Model gate: server and shell unit tests cover the invariants above, live-close semantics, explicit keep-alive persistence, and the no-implicit-copy-generation policy.
2. Local terminal gate: second-X11 typing smoke proves local shell input reaches an interactive terminal quickly without retry/disconnect toasts, and blank Enter does not leave a stale live-row spinner behind.
3. KDE lifecycle gate: update/restart and app-owned smoke launch keep `plasmashell` stable, protect all live runtimes during the restart, leave no stale temp-home automation clients behind, and show `linux_daemon_sweep` skipping cross-home daemons.
4. Remote session gate: switching between stored preview, live remote terminal, and retained live terminal uses `server app open <path> --view <terminal|preview>` or the matching app-control command, waits for settled state, and keeps row, active path, runtime truth, scrollback, and terminal text aligned. `terminal focus` is focus reclamation only; it must not be used as a session-switch proof.
   For a live-profile restart, this gate must be run over every top-level
   `Live Sessions` row, not only the active row. The failure signature from the
   2026-05-22 jojo incident is explicitly banned: live rows visible, no daemon
   runtime keys observed by app-control, `needs_initial_server_sync` still true
   after settle, an empty xterm host in Terminal mode, and repeated
   `startup_terminal_restore_recover` remounts. That state is a failed restore
   loop, not a degraded but usable terminal.
5. Clipboard gate: text and screenshot paste work in local, SSH, and Codex sessions through the native paste path, with the resulting staged image path visible in the receiving terminal.
6. Notification gate: Codex-class completion and terminal notification/bell events create the expected in-app notification and sound when enabled.
7. Cross-platform gate: Windows and macOS smoke runs prove install, launch, local terminal creation, close, paste, and update paths still work.
8. Release gate: every field-test release includes the `.deb`, checksums, curated release notes, and proof paths for Linux/KDE, Windows, and macOS.

## Panic Management Runbook

`yggterm-headless server monitor` is the SSH-safe first response tool for live
terminal incidents. Use it before guessing from screenshots or editing restore,
rendering, or daemon lifecycle code.

Use it when a terminal is hung, missing after restore, input-lagged, visually
blank, slow to snapshot, or when GUI/daemon/install versions look mismatched:

```bash
yggterm-headless server monitor \
  --scenario panic-report \
  --expect-path "<session-path>" \
  --jsonl-out /tmp/yggterm-incident.jsonl
```

For intermittent failures, collect repeated evidence:

```bash
yggterm-headless server monitor \
  --scenario panic-report \
  --expect-path "<session-path>" \
  --iterations 30 \
  --interval-ms 1000 \
  --jsonl-out /tmp/yggterm-watch.jsonl
```

Triage from the report:

- no reachable daemon or stale version: inspect server-list, sockets, and install
  metadata before using `hot-restart --all`
- expected session missing: run `wait-session --expect-path <session-path>`
- slow status/snapshot: run `latency-check --all` and inspect trace/perf data
- healthy daemon but blank or stale screen: use `server app state`,
  `screenshot`, `probe-type`, `probe-scroll`, or `probe-select`
- KDE pinning or duplicate app identity: run `server app desktop-identity`.
  The report must prove the canonical `dev.yggterm.Yggterm.desktop` launcher,
  matching `Icon` and `StartupWMClass`, a live app-control client, and a
  canonical Linux desktop app id for that live PID. The live client-instance
  record is durable identity proof; `linux_desktop_app_id` and
  `linux_desktop_identity_applied` trace events are supporting startup proof and
  may rotate during long smokes. A missing client-record app id is a
  release-blocking regression on KDE because the shell may open under a second
  taskbar icon even when the desktop file itself is correct.
- managed Codex/tooling issue: use explicit foreground refresh paths, not
  unattended background installs

The monitor establishes facts. Keep incident commands read-only until evidence
clearly points to daemon lifecycle recovery, session restore, or managed CLI
refresh as the right next step.

## Pass Plan

- Pass 1: add snapshot invariants, live-close wording, and targeted tests for the defects that escaped v2.1.33.
- Pass 2: move session/view changes through a single reducer-style API so shell selection cannot mutate runtime state indirectly.
- Pass 3: split session identity, runtime lifecycle, and display copy into separate structs.
- Pass 4: make preview hydration and title/summary generation event driven, rate limited, and impossible to trigger from plain selection.
- Pass 5: make update restart, native paste, Codex completion notification, and terminal focus/scroll/input state release-blocking smokes.
- Pass 6: promote the smokes for local speed, session switching, KDE close, Windows, and macOS into a release-blocking stability suite.
