---
name: yggui-changelog-demo
description: Capture deterministic proof bundles, screenshots, traces, and curated changelog notes for YggUI app changes.
---

# YggUI Changelog Demo

Use this workflow when a `yggui` app feature or fix should ship with proof, screenshots, and a curated changelog entry.

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
- When a startup restore fails before PTY attach, include the daemon `remote_saved_session_preflight_*` trace events and the UI overlay excerpt. Missing-session truth should be proven from the preflight path, not inferred later from a blank terminal timeout.
- For fresh remote full-screen attaches, also capture whether `ui/terminal_mount` emitted `resize_nudge_begin` / `resize_nudge_end`. The nudge is part of the product contract now: it forces a repaint before Yggterm concludes that a live TUI attach is still blank.
- Do not treat a visible terminal failure overlay as final proof if `shell.terminal_attach_in_flight` still contains the active session path. That is an in-flight recovery state, not a finished verdict.
- If terminal settle happens through the saved-context chip, record that explicitly as `terminal_settled_kind == "overlay_context"` and keep `active_terminal_surface.live_problem` in the bundle so the proof preserves both the user-visible result and the live PTY caveat.
- Remote startup can also settle as `overlay_context` through host-rendered saved context with the live chip already hidden. In that case, prove `ready: true`, `interactive: false`, and `terminal_settled_kind == "overlay_context"` from app state instead of expecting a visible chip in the screenshot.
- Do not call a terminal `overlay_context` just because the host has meaningful text. `overlay_context_visible` now means the saved-context fallback is still the user-visible truth because the overlay chip still carries context or the host is still on staged prefill while the live surface is not yet trustworthy.
- For startup timing, prefer the app trace `startup/window_spawned` event over slower X11 root-tree detection when both are available. Use X11 tree timing only as fallback evidence.
- For terminal session-switch bugs, capture both the source and destination terminal surfaces on a second X11 display and verify the destination screenshot text matches the destination `active_session_path`, not stale text from the previous session.
- When a proof bundle uses `server app screenshot` on Linux X11, state whether the branch includes the real-window screenshot path. Older WebKit-only captures could miss embedded xterm content and produce false blank-terminal evidence.
- For terminal geometry or overdraw bugs, include `terminal_hosts[].host_rect`, `terminal_hosts[].screen_rect`, and `terminal_hosts[].viewport_rect` alongside the screenshot and attempt ledger.
- Include `terminal_hosts[].host_content_width`, `host_content_height`, `host_padding_left_px`, `host_padding_right_px`, `host_padding_top_px`, and `host_padding_bottom_px` when the fix uses xterm gutter compensation or any host-content-box adjustment.
- For typing/cursor visibility bugs, also include `terminal_hosts[].viewport_y` and `terminal_hosts[].base_y` so the proof shows whether the live cursor fell below the visible viewport.
- For xterm input-hitbox or overtyping bugs, also include `terminal_hosts[].helpers_rect` and `terminal_hosts[].helper_textarea_rect`. A drifted helper textarea is now a classified geometry failure, not a cosmetic quirk.
- Treat any non-null `active_terminal_surface.geometry_problem` as a failed terminal proof, even if the surface otherwise looks rendered.
- For startup latency claims, include whether the daemon emitted `daemon/startup_prewarm begin|end|error` for the active terminal. Startup restore should now be prewarmed after the control socket binds instead of waiting for the first UI mount to pay the whole cost.
- For remote terminal startup restore, also capture whether the initial attach stream included `__YGGTERM_ATTACH_READY__`. That server marker now means the PTY attach itself is live even when Codex is sitting on low-signal idle/footer chrome.
- For loading-truth bugs, capture one state while `active_surface_requests` still contains the terminal request and one after settle so the bundle shows that the UI did not silently drop the request before attach finished.
- Keep changelog language user-visible and concise.
- Treat demo assets as release material, not disposable debugging leftovers.
- When a result is not live-verified, say so explicitly.
