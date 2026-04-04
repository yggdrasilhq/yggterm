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
- Do not treat a visible terminal failure overlay as final proof if `shell.terminal_attach_in_flight` still contains the active session path. That is an in-flight recovery state, not a finished verdict.
- If terminal settle happens through the saved-context chip, record that explicitly as `terminal_settled_kind == "overlay_context"` and keep `active_terminal_surface.live_problem` in the bundle so the proof preserves both the user-visible result and the live PTY caveat.
- For startup timing, prefer the app trace `startup/window_spawned` event over slower X11 root-tree detection when both are available. Use X11 tree timing only as fallback evidence.
- For terminal session-switch bugs, capture both the source and destination terminal surfaces on a second X11 display and verify the destination screenshot text matches the destination `active_session_path`, not stale text from the previous session.
- When a proof bundle uses `server app screenshot` on Linux X11, state whether the branch includes the real-window screenshot path. Older WebKit-only captures could miss embedded xterm content and produce false blank-terminal evidence.
- For terminal geometry or overdraw bugs, include `terminal_hosts[].host_rect`, `terminal_hosts[].screen_rect`, and `terminal_hosts[].viewport_rect` alongside the screenshot and attempt ledger.
- Include `terminal_hosts[].host_content_width`, `host_content_height`, `host_padding_left_px`, `host_padding_right_px`, `host_padding_top_px`, and `host_padding_bottom_px` when the fix uses xterm gutter compensation or any host-content-box adjustment.
- Treat any non-null `active_terminal_surface.geometry_problem` as a failed terminal proof, even if the surface otherwise looks rendered.
- For loading-truth bugs, capture one state while `active_surface_requests` still contains the terminal request and one after settle so the bundle shows that the UI did not silently drop the request before attach finished.
- Keep changelog language user-visible and concise.
- Treat demo assets as release material, not disposable debugging leftovers.
- When a result is not live-verified, say so explicitly.
