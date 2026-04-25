# Yggterm Stability Contract

Yggterm feature work is frozen until the session/runtime model is stable enough to field test without repeating the same GUI failures. This document is the working contract for that stabilization pass.

## Current Diagnosis

The repeated bugs come from ownership ambiguity. The same visible session is currently described by persisted browser rows, daemon live-session rows, remote scan records, retained xterm hosts, active shell selection, preview copy jobs, and restore/reconnect paths. When those surfaces disagree, the app can show one row, render another session, regenerate copy for the wrong target, or expose destructive wording for a runtime close.

The fix is not a larger feature. The fix is to make invalid state impossible or at least immediately visible to tests and app-control.

## Feature Freeze Rules

- No new user-facing terminal/session feature work until the stability gates below pass on Linux/KDE, Windows, and macOS.
- Selection is allowed to change focus. It must not regenerate title/summary, relaunch a runtime, or switch terminal/preview mode unless the user action explicitly requested that side effect.
- Passive title/precis/summary generation is disabled by default. Selection may hydrate already-cached copy, but it must not start LLM work. The app-control `generation.copy_generation_start_count` counter is the proof surface for this contract.
- Title, precis, and summary are display copy only. They are never identity and never decide which runtime receives input.
- Live Sessions are daemon-owned runtimes. Closing one kills that runtime and removes it from the Live Sessions group. It must not imply stored transcript deletion unless the user requested a hard delete.
- Stored sessions and remote scanned sessions open as preview unless an explicit terminal launch/resume action promotes them to a live runtime.
- Remote scanned sessions may appear in Terminal mode only when the remote scan says the runtime is live and the active session source is `LiveSsh`.
- A retained terminal host may stay mounted only if its session identity still matches a live session or a deliberate recovery state.

## Executable Invariants

`validate_server_ui_snapshot` in `crates/yggterm-server/src/lib.rs` is the first executable contract. A server UI snapshot is invalid when:

- `active_session_path` and `active_session.session_path` disagree.
- Terminal mode is active without an active session.
- Terminal mode is active for a stored/non-live session, except document terminal recipes.
- A remote scanned terminal session is not backed by a `LiveSsh` session and a remote scan `live_runtime == true`.
- `live_sessions` contains a stored session, document node, duplicate path, or remote scanned row that the scan does not mark live.
- An active live session is missing from `live_sessions`.

These checks should move closer to the reducer/state transitions over the next passes. For now they are intentionally snapshot-level so both unit tests and GUI smoke tests can catch cross-layer disagreement.

The shell also exposes a copy-generation budget contract through `server app state`: `generation.implicit_copy_generation_enabled`, `generation.copy_generation_start_count`, and the title/precis/summary in-flight path arrays. Opening or selecting a row without an explicit regenerate action must leave the start counter unchanged.

Inline rename is also part of the observability contract. While rename mode is active, `server app state` must expose the controlled `shell.tree_rename_value`; when DOM snapshots degrade under KDE load, the action fallback should still expose `dom.tree_rename_input_value` for the visible input or leave the shell value available for smoke assertions.

Titlebar search has the same proof requirement. When `shell.search_query` is non-empty or `shell.search_focused` is true, a degraded DOM snapshot must still expose the active search input rect and focused input value so the slow-typing regression cannot hide behind app-control timeouts.

## Stability Gates

1. Model gate: server and shell unit tests cover the invariants above, live-close semantics, and the no-implicit-copy-generation policy.
2. Local terminal gate: second-X11 typing smoke proves local shell input reaches an interactive terminal quickly without retry/disconnect toasts.
3. Remote session gate: switching between stored preview, live remote terminal, and retained live terminal keeps row, active path, and terminal text aligned.
4. Cross-platform gate: Windows and macOS smoke runs prove install, launch, local terminal creation, close, and update paths still work.
5. Release gate: every field-test release includes the `.deb`, checksums, curated release notes, and proof paths for Linux/KDE, Windows, and macOS.

## Pass Plan

- Pass 1: add snapshot invariants, live-close wording, and targeted tests for the defects that escaped v2.1.33.
- Pass 2: move session/view changes through a single reducer-style API so shell selection cannot mutate runtime state indirectly.
- Pass 3: split session identity, runtime lifecycle, and display copy into separate structs.
- Pass 4: make preview hydration and title/summary generation event driven, rate limited, and impossible to trigger from plain selection.
- Pass 5: promote the smokes for local speed, session switching, KDE close, Windows, and macOS into a release-blocking stability suite.
