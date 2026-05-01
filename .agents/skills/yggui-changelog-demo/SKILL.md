---
name: yggui-changelog-demo
description: Capture deterministic proof bundles, screenshots, traces, and curated changelog notes for YggUI app changes.
---

# YggUI Changelog Demo

Use this workflow when a `yggui` app feature or fix should ship with proof, screenshots, and a curated changelog entry.

Observability note: the terminal attempt ledger, viewport classifier, and app-control terminal-surface helpers now live in `crates/yggterm-shell/src/terminal_observe.rs`. Do not keep spelunking only in `shell.rs` when validating or extending proof semantics.

## Goals

- capture deterministic evidence
- produce a reusable proof bundle
- draft changelog text from real artifacts
- keep documentation and workflow in sync when automation grows

## Inputs

- a user-visible feature or fix
- the relevant macro or app-control path
- a target proof bundle id under `artifacts/demos/unreleased/`

## Workflow

1. Identify the user-visible claim.
2. Choose or write the deterministic macro path.
3. Capture:
   - screenshots
   - optional recording
   - app-state snapshot
   - event trace / perf evidence
   - `active_surface_requests` when a terminal load/restore claim depends on whether the request is still truthfully in flight
   - when terminal open/restore is involved, the exact `terminal_open_attempt` object, `active_terminal_surface`, `interactive`, and `terminal_settled_kind`
   - for terminal geometry bugs, include whether `active_terminal_surface.geometry_problem` was set
   - for input/focus bugs, include `dom.active_element`, `terminal_hosts[].helper_textarea_focused`, and `terminal_hosts[].host_has_active_element`
   - for startup input-contract bugs, also include `terminal_hosts[].input_enabled`
   - for session/view ownership bugs, include `session_view_contract_violations` and reject the proof unless it is empty
   - for session-selection/copy-budget bugs, include `generation.copy_generation_start_count`, `generation.implicit_copy_generation_enabled`, and the title/precis/summary in-flight path arrays before and after selection
   - for inline rename bugs, include `shell.tree_rename_value`, `dom.tree_rename_input_value`, `dom.tree_rename_input_focused`, `dom.tree_rename_input_selection_start`, `dom.tree_rename_input_selection_end`, and any `snapshot_mode == "action-fallback"` evidence if KDE forced a degraded snapshot
   - for titlebar search typing bugs, include `shell.search_query`, `shell.search_focused`, `dom.active_element.value`, `dom.titlebar_search_active`, and any `snapshot_mode == "action-fallback"` evidence if KDE forced a degraded snapshot
4. Create or update the proof bundle:
   - `manifest.json`
   - `summary.md`
   - `captures/`
   - `trace/`
5. Update `CHANGELOG.md` with a concise user-facing note.
6. If new automation or capture powers were required, update:
   - `docs/demos/ARCHITECTURE.md`
   - `docs/demos/FORMAT.md`
   - `docs/demos/STYLE.md`
   - this skill file

## Standards

- Prefer exact screenshots and traces over vague prose.
- For terminal restore claims, bind the proof to one attempt id and fail the claim if that attempt latched any failure, even if a later state looks healthy.
- For startup restore work, prove the app did not issue a second reopen of the already-active terminal. One startup mount sequence is correct. A duplicate reopen is a bug, even if a later attempt recovers.
- For remote startup restore, the hot path no longer blocks on a separate saved-session existence probe. Expect `remote_saved_session_preflight_elided_runtime_launch` in the daemon trace, then prove missing-session truth from the runtime launch itself, the attempt ledger, and the overlay excerpt.
- For fresh remote full-screen attaches, also capture whether `ui/terminal_mount` emitted `resize_nudge_begin` / `resize_nudge_end`. The nudge is part of the product contract now: it forces a repaint before Yggterm concludes that a live TUI attach is still blank.
- Do not treat a visible terminal failure overlay as final proof if `shell.terminal_attach_in_flight` still contains the active session path. That is an in-flight recovery state, not a finished verdict.
- In `Terminal` mode, saved preview context is no longer accepted as a terminal-ready settle. Expect `terminal_settled_kind == "recovering"` until the resume chip clears and the live terminal is visually revealed.
- If the terminal host already has staged transcript bytes while the resume chip is still up, treat that as `recovering`, not `overlay_context` and not `interactive`.
- Do not call a terminal `overlay_context` just because the host has meaningful text. `overlay_context_visible` only applies when the saved-context fallback is still the user-visible truth. Terminal-mode recovery should now stay `recovering` instead.
- Codex model-permission setup selectors are interactive terminal surfaces even when they sit mid-screen with many blank rows below the hidden cursor, no attach-ready visual deadline, or only the lower half of the selector visible to the live health tail, including tails that start inside the `auto-reviewer` line. A proof should show `terminal_settled_kind == "interactive"`, no remote-attention notification, and `terminal_hosts[].input_enabled == true` while the selector text is visible.
- The main viewport should stay available during terminal recovery. Resume progress belongs in notifications/toasts, not as a full-viewport curtain over the host. If a proof screenshot shows the terminal surface replaced by a recovery card, treat that as a UX regression.
- For startup timing, prefer the app trace `startup/window_spawned` event over slower X11 root-tree detection when both are available. Use X11 tree timing only as fallback evidence.
- For terminal session-switch bugs, capture both the source and destination terminal surfaces on a second X11 display and verify the destination screenshot text matches the destination `active_session_path`, not stale text from the previous session.
- For KDE/restart lifecycle bugs, include the `linux_daemon_sweep` trace slice plus any `spawned_daemon_child` / `spawned_daemon_exit` or `local_spawned_daemon_child` / `local_spawned_daemon_exit` events. The proof should show same-home daemon cleanup only, no cross-home orphan reap, and no lingering temp-home GUI/daemon after the bundle closes.
- For daemon hot-update, multi-version control, hung-session, or latency incident claims, start the proof with `yggterm-headless server monitor --scenario panic-report --expect-path <session-path> --jsonl-out <path>`. Include `server-list`, the matching `hot-restart` result when lifecycle recovery is used, and a post-restart `latency-check --all` or `wait-session` proof so the bundle shows both the incident picture and the recovered server surface. For KDE duplicate-icon claims, add `yggterm-headless server app desktop-identity` so the bundle captures pinned launchers, desktop file fields, live client app ids, and update-handoff env.
- For stale multi-version remote runtime incidents, prove that the current client refused the stale bridge: include `server/remote_runtime skip_stale_runtime_daemon`, `server-list` or `latency-check --all` showing both versions, and a final state where the terminal-open attempt either reaches `ready` on the current daemon or latches a failure with `terminal_attach_in_flight` and `active_surface_requests` cleared.
- For update-restored remote Live Sessions that are no longer really live, capture a fresh remote scan or app state after scan. Unkept temporary update-restore rows must disappear from `Live Sessions` once the scan reports `live_runtime=false`, and the trace should include `server/remote_machine prune_temporary_stale_live_sessions`. Explicit keep-alive rows may remain as recovery targets, but the proof must show `live_session_snapshot_debug[].keep_alive == true`.
- For title/summary budget regressions, prove selection did not start LLM work by showing `generation.copy_generation_start_count` unchanged across the open/select action. Cached copy hydration is allowed; title, precis, or summary generation is not allowed unless the user used an explicit regenerate action.
- For stored Codex transcript regressions, prove two separate moments: the cold-start selected row must stay idle with no `active_session_path`, no matching `terminal_open_attempt`, and no matching `active_surface_requests`; then an explicit `server app open <path>` without `--view` must promote the row to `Terminal`, move the resulting `LiveLocal` runtime under `Live Sessions` without leaving a duplicate stored-tree row, keep `generation.copy_generation_start_count` unchanged, and show sidebar row cursors as normal `pointer` values rather than idle `grab`/`grabbing`.
- For inline rename regressions, prove the initial title is selected from `0..len(title)`, each typed prefix survives app-control observation, Ctrl+A selects inside the input rather than the sidebar, and Enter or click-away clears `shell.tree_rename_path`. If KDE forces `dom_debug_snapshot_timeout`, the proof may use the action fallback or `shell.tree_rename_value`, but a state that loses both DOM and shell rename values while `shell.tree_rename_path` is set is a failed proof.
- For titlebar rename regressions, capture `dom.titlebar_title_rect` and `dom.titlebar_summary_title_rect`, then prove clicking either the title chip text or the title/summary modal title enters the same focused inline rename contract as the sidebar context menu.
- For titlebar search typing regressions, prove the shell query and focused DOM input value advance together. If the app falls back to `snapshot_mode == "action-fallback"`, it must still include the active search input rect and active element value.
- When a proof bundle uses `server app screenshot` on Linux X11, state whether the branch includes the real-window screenshot path. Older WebKit-only captures could miss embedded xterm content and produce false blank-terminal evidence.
- For terminal geometry or overdraw bugs, include `terminal_hosts[].host_rect`, `terminal_hosts[].screen_rect`, and `terminal_hosts[].viewport_rect` alongside the screenshot and attempt ledger.
- Include `terminal_hosts[].host_content_width`, `host_content_height`, `host_padding_left_px`, `host_padding_right_px`, `host_padding_top_px`, and `host_padding_bottom_px` when the fix uses xterm gutter compensation or any host-content-box adjustment.
- For typing/cursor visibility bugs, also include `terminal_hosts[].viewport_y` and `terminal_hosts[].base_y` so the proof shows whether the live cursor fell below the visible viewport.
- For xterm input-hitbox or overtyping bugs, also include `terminal_hosts[].helpers_rect` and `terminal_hosts[].helper_textarea_rect`. A drifted helper textarea is now a classified geometry failure, not a cosmetic quirk.
- For terminal input bugs, also prove focus ownership. The good state is an active `xterm-helper-textarea` inside the active host plus `helper_textarea_focused: true` and `host_has_active_element: true`.
- For remote terminal input bugs, also capture one deterministic `server app terminal send ... --data "__SENTINEL__"` proof and the matching terminal text sample after settle. When multiple GUI clients exist, first capture `server app clients` and then target the proof with `--pid <pid>` so automation cannot bleed into the wrong desktop window. If local input works but remote input does not, inspect `server/remote_stdio_bridge` events such as `bridge_stdin_raw_mode_enable`, `bridge_stdin_raw_mode_skip`, and `bridge_stdin_raw_mode_restore` in `~/.yggterm/event-trace.jsonl`.
- `server app terminal probe-type` and `probe-scroll` now exercise the active xterm host in the main viewport. `probe-type` first uses xterm core data injection, then falls back to the mounted input path only if needed, with optional `--per-char`, `--ctrl-c`, `--tab`, and `--enter`. It reports `visible_echo_observed` plus `timings.visible_echo_ms` from the xterm buffer/cursor sample, so canvas-rendered terminals cannot pass by returning empty `host.innerText`. On 2.1.84+ `--per-char` does not add artificial per-character sleeps; slow visible echo means the app/input path is slow, not the probe loop.
- For latency reports, run `scripts/smoke_ui_latency.py --host <host> --pid <pid> --clear-after` against the live client or a second-display client. `--clear-after` clears the prompt before and after short marker samples, so marker runs cannot wrap and create false missing-echo failures. The proof should include state/rows/search/panel timings, terminal sample p50/p95/max, and the active session path.
- The default latency-smoke budgets are for live SSH app-control proof: 1200 ms for state/rows/search/panel command round trips, 500 ms for any individual terminal visible echo, and 450 ms for terminal visible-echo p95. Tighten the flags for local CI runs.
- On direct installs, run terminal probe actions through the public launcher on 2.1.55+ so the headless path can use the real X11 keyboard probe. On 2.1.52-2.1.54, the launcher/headless path cannot dispatch `focus`, `probe-type`, `probe-scroll`, or `probe-select`, so use the exact active GUI executable from `install-state.json` for those actions and state that limitation in the proof.
- For terminal input regressions, raise the proof bar further: use `server app terminal probe-type --mode keyboard --data '/status' --enter` on a fresh second-X11 client and require the resulting screenshot plus state to show `/status` in the prompt area, the live Codex status panel, no transcript-resume footer, no shell fallback like `bash: /status`, a visible cursor at the next prompt, and a still-interactive terminal.
- During partial `/status` typing, Codex may keep updating slash-command suggestions. Do not require app-control samples to be fully quiescent between chunks; require the typed prefix, focused helper textarea, enabled input, cursor evidence, and the screenshot instead.
- For prompt/cursor regressions, also run the partial-input loop: type `/sta` without Enter, capture screenshot + state, then scroll and capture again. Reject the fix if the typed partial input is not visible on `cursor_line_text`, if the cursor row drifts out of the prompt band near the bottom of the viewport, or if focus/input drops during the scroll step.
- `server app terminal probe-select` now selects real text inside the mounted xterm rows and reports the selected excerpt/length/contrast. Use it for “text only visible when selected” regressions and pair it with the low-contrast span diagnostics from app state.
- For browser-selection leak regressions on embedded xterm, also capture `terminal_hosts[].xterm_root_user_select`, `rows_user_select`, `selection_range_count`, `selection_layer_count`, and `selection_layer_rect_count`. Reject the fix if the mounted host can still accumulate a browser DOM range selection or if the xterm root/rows stop reporting `user-select: none`.
- `scripts/smoke_xterm_embed_faults.py` is now the top-level fault-model suite for embedded xterm regressions. Use it when the bug spans multiple symptoms such as cursor drift, invisible text, geometry mismatch, focus/input breakage, scroll failure, and theme/readability regressions at once.
- For isolated second-display labs, pass `--home /tmp/...` to the smoke script and prefix any follow-up `server app ...` commands with the same `YGGTERM_HOME=/tmp/...` so the proof does not accidentally target your real desktop client.
- For fresh local-terminal regressions, keep one detached second-display proof that uses `server app terminal new` and reject the fix unless the screenshot shows the prompt in the main viewport within a few seconds, the runtime row appears under the first `Live Sessions` group with a close affordance, the active host reports non-empty `text_sample`/`text_tail` or canvas-mode `buffer_text_sample`/`cursor_line_text`, fresh terminals are not marked keep-alive until explicitly toggled, blank Enter does not leave the row spinning, and the same row can enter the rotating `busy` icon during a foreground command and recover back to `plain-terminal` once the prompt returns.
- For local startup-restore regressions, also run `scripts/smoke_terminal_local_restart.py` (or an equivalent second-display proof) and reject the fix unless the same local session survives app restart, reopens without a blank xterm host, and agrees three ways: `active_session_path`, the DOM-selected sidebar row, and `browser.selected_row` must all point at the same session.
- The renderer contract defaults to `canvas` and treats `dom` as an explicit opt-out path. For canvas mode, inspect `terminal_hosts[].buffer_text_sample` and `cursor_line_text` because `.xterm-rows` is absent by design; still reject any proof where the screenshot is visually blank, geometry is wrong, or cursor/input evidence is missing.
- The sidebar proof now has an explicit idle contract for local shells: after probe traffic settles back to a prompt, the selected row must recover from the rotating `busy` icon to the macOS-command `plain-terminal` icon, even when the active summary is condensed as `pi@host$ >.`.
- Cursor visibility now has explicit native-cursor evidence too: `terminal_hosts[].cursor_sample_rect`, `cursor_sample_text`, `cursor_sample_color`, `cursor_node_rects`, and `xterm_cursor_hidden`. For light-theme terminal readability fixes, reject the proof unless the screenshot itself shows the cursor, `cursor_sample_rect` is visible while input is enabled, and `xterm_cursor_hidden` agrees with what the screenshot shows.
- Cursor alignment now has explicit native-cursor evidence too: compare `terminal_hosts[].cursor_sample_rect` against `cursor_expected_rect`, and use `cursor_node_rects` as supporting evidence when xterm exposes additional raw cursor DOM spans. Reject the fix if the visible native cursor drifts away from the expected cursor cell.
- Retained terminal hosts can coexist. Do not assume `terminal_hosts[0]` is the active terminal. Select the host that matches the active session path and focused input ownership, or use an explicit active-host marker if present.
- When xterm emits a very wide raw `.xterm-cursor` span, do not fail on width alone. Fail only if that wide span is still visually active via background, border, outline, or box-shadow. The native xterm cursor is now the visible cursor contract.
- Do not trust the `probe-type` response by itself. Always pair it with a follow-up `server app state` and `server app screenshot`, then judge the bug from the resulting screenshot plus `terminal_hosts[].text_sample`, `terminal_hosts[].text_tail`, and in canvas mode `terminal_hosts[].buffer_text_sample`/`cursor_line_text`.
- For UI-theme or terminal-theme claims, prefer `server app theme light|dark --pid <pid>` over click-based toggles during proof capture. The resulting app state now exposes `settings.theme`, `settings.terminal_light_theme_name`, `settings.terminal_dark_theme_name`, `settings.effective_terminal_theme_name`, and the mounted xterm renderer fields `terminal_hosts[].xterm_font_family`, `xterm_font_weight`, `xterm_font_weight_bold`, `xterm_line_height`, `xterm_theme_background`, and `xterm_theme_foreground`. Also inspect the actual rendered row sample fields `terminal_hosts[].rows_sample_font_family`, `rows_sample_font_weight`, `rows_sample_font_feature_settings`, `rows_sample_letter_spacing`, `rows_sample_line_height`, `rows_sample_color`, `rows_sample_class_name`, `rows_sample_style_attr`, `dim_sample_*`, `cursor_sample_*`, `low_contrast_span_count`, `low_contrast_min_contrast`, and `low_contrast_span_samples` (with the older `rows_*` fields as fallback), or run `scripts/smoke_terminal_theme_ui.py`, so the proof covers the actual rendered xterm rows and not just terminal option values. Reject any proof where the sampled row font family is still a single doubly-quoted literal stack, the cursor styling is transparent, visible low-contrast spans remain, or the mounted screen width drifts far from the host viewport.
- Still keep one second-X11-display proof in the loop for GUI fixes, but do not rely on flaky `xdotool` focus alone when the viewport probe can prove the same input path more deterministically.
- For startup restore, the healthy recovery state is a visible toast plus `input_enabled == false` until the live terminal actually settles interactive.
- For fresh Codex startup, the `Update Model Permissions` selector is an interactive surface when it shows the Default/Auto-review/Full Access options plus the "Press enter to confirm or esc to go back" hint, even if app-control reports many blank rows below the hidden cursor because the selector sits mid-screen. Proof should show `terminal_settled_kind == "interactive"`, no remote-attention timeout notification, and `input_enabled == true` for that mounted host instead of treating the menu as stale retained transcript text.
- For fresh remote Codex startup over SSH, use `server app terminal new --machine-key <machine> --kind codex` or the `codex_spawn_timeline` smoke. Capture sub-1s, 1s, 2s, 5s, ready, and post-30s states/screenshots, then reject the proof if the visible host shows the local Codex scaffold text, if the session-specific `Remote Terminal Needs Attention` notification appears, if `terminal_settled_kind` is not `interactive` after settle, if `terminal_hosts[].input_enabled` is not true, or if the active surface problem remains set.
- If a remote terminal drops back into retry/recovery after a bad intermediate surface, the resume toast should stay visible until the session reaches the real visual reveal again. Do not accept a run where the toast disappears while `input_enabled == false`, `terminal_settled_kind != "interactive"`, or the terminal request is still truthfully recovering.
- If a remote resume times out, the attention toast may remain as user-facing error state, but the open-attempt ledger must move to failed and the matching `terminal_attach_in_flight`, bootstrap lease, and terminal surface request must clear. Reject proof where a no-progress loading toast stays in `active_surface_requests` indefinitely or drives high idle render counts.
- For terminal-resume toast regressions, also verify the inverse case: once there are no visible notifications, the screenshot should not show an empty blurred/white toast shell still hanging under the titlebar. Capture both `notifications_count` and a screenshot of the same moment.
- Treat any non-null `active_terminal_surface.geometry_problem` as a failed terminal proof, even if the surface otherwise looks rendered.
- Exception: the stable retained-xterm layout may present `screen_rect/helpers_rect` about `16px` narrower than `host_rect` while `viewport_rect` still matches the host. That compensated gap is now accepted and should not be treated as a failed proof by itself.
- For startup latency claims, include whether the daemon emitted `daemon/startup_prewarm begin|end|error` for the active terminal. Startup restore should now be prewarmed after the control socket binds instead of waiting for the first UI mount to pay the whole cost.
- For remote terminal startup restore, also capture whether the initial attach stream included `__YGGTERM_ATTACH_READY__`. That server marker now means the PTY attach itself is live even when Codex is sitting on low-signal idle/footer chrome.
- Once `__YGGTERM_ATTACH_READY__` has arrived, a quiet attached terminal is allowed to settle after the reveal grace deadline only when the retained host surface is prompt-ready. Retained non-prompt text from a previous Codex answer is stale evidence: it may remain visible, but it must not clear the resume toast, mark the attempt interactive, or enable input.
- For loading-truth bugs, capture one state while `active_surface_requests` still contains the terminal request and one after settle so the bundle shows that the UI did not silently drop the request before attach finished.
- Keep changelog language user-visible and concise.
- Treat demo assets as release material, not disposable debugging leftovers.
- When a result is not live-verified, say so explicitly.
