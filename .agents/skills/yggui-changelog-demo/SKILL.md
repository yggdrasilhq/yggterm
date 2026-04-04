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
   - when terminal open/restore is involved, the exact `terminal_open_attempt` object, `active_terminal_surface`, `interactive`, and `terminal_settled_kind`
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
- If terminal settle happens through the saved-context chip, record that explicitly as `terminal_settled_kind == "overlay_context"` and keep `active_terminal_surface.live_problem` in the bundle so the proof preserves both the user-visible result and the live PTY caveat.
- Keep changelog language user-visible and concise.
- Treat demo assets as release material, not disposable debugging leftovers.
- When a result is not live-verified, say so explicitly.
