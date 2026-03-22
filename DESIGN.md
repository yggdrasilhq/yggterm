# DESIGN.md

## Purpose

This file is the visual and interaction source of truth for Yggterm.

When UI work, wording, spacing, colors, controls, or interaction polish are in question, prefer this file over ad hoc invention. `AGENTS.md` defines the engineering/product mission; `DESIGN.md` defines how the app should feel, read, and look.

This file is intentionally reusable across projects. It should capture stable brand and taste preferences, not one-off bug notes.

## Brand intent

Yggterm should feel:

- calm
- modern
- lightly premium
- youthful without being toy-like
- crisp rather than ornamental
- soft around the edges, but not soft-headed

The app should not feel like:

- a Linux utility panel
- a web admin dashboard
- a noisy IDE clone
- a skeuomorphic terminal toy
- a stack of nested cards inside more cards

The core impression should be:

- one clear main workspace
- supportive chrome around it
- low-friction controls
- light, breathable, polished surfaces

## Visual structure

### Main workspace

The main viewport is the focus of the product.

- It should read like a calm sheet or canvas.
- It should generally be white or near-white in light mode.
- It may have a soft shadow and mild radius.
- It should not be crowded by decorative headers, nested boxes, or redundant toolbars.
- Terminal mode should feel like the terminal is part of the main canvas, not a foreign widget pasted inside a card.

### Supporting chrome

The surrounding chrome should feel supportive, not dominant.

- Side rails should be lighter and quieter than the main canvas.
- A faint blue-to-green fresh tint over a muted neutral base is desirable.
- The rails should avoid heavy borders.
- The shell should feel visually unified, not partitioned into many harsh boxes.

### Shape language

- Rounded corners are welcome, but should stay restrained and OS-friendly.
- The shell should feel closer to modern KDE/Windows rounding than exaggerated mobile rounding.
- In maximized state, outer window corner radius should collapse to zero.
- Inner radii should be smaller than outer shell radii.

## Color direction

Light mode is the primary reference.

- Prefer white and pale blue-grey foundations.
- Accent color can lean clean blue.
- Background tint may gently lean sky-blue to green.
- Use contrast carefully; avoid washed-out unreadable controls.
- The main canvas and terminal surface should visually cohere.

Avoid:

- muddy greys
- purple-heavy defaults
- over-opaque frosted layers that bury hierarchy
- gratuitous gradients inside the main content region

## Typography

### Interface font

- Linux: `Inter Variable`
- macOS/Windows: default platform system UI font

### Terminal font

- `JetBrains Mono`
- terminal zoom should be explicit, predictable, and visually obvious

### General text guidance

- small text must still feel antialiased and intentional
- avoid overly thin right-rail typography
- headings should feel clean and editorial, not shouty
- labels should be concise and legible

## Control language

### Segmented pills

Segmented pill controls are preferred for small mode switches.

Examples:

- `Preview / Terminal`
- `Codex / Codex LiteLLM`
- notification delivery modes

They should:

- clearly show the active segment
- have a clean outer pill
- avoid awkward double-border or muddy selected states
- feel stable and precise

### Primary buttons

Primary actions should look unmistakably clickable.

- blue background is acceptable for the main affirmative action
- white text
- clear contrast
- enough padding to feel intentional

If a user says “this does not look like a button”, that is a design failure, not a user error.

### Inputs

- Prefer clean rectangular or softly rounded input boxes.
- Avoid pill-shaped text fields unless there is a very strong reason.
- Inputs must remain visible against the supporting chrome.

### Context menus

Context menus should feel closer to modern Microsoft app context menus than generic web popovers.

That means:

- open at the cursor
- modest radius
- clean light surface
- subtle shadow
- compact but breathable row sizing
- strong label clarity

Avoid:

- giant floating glass blobs
- top-left fallback placement
- labels that invent confusing product language

## Terminology

Do not invent new nouns casually.

Current preferred user-facing terms:

- `Session`: a live or stored terminal/agent context
- `Group`: a virtual container in the tree
- `Paper`: a lightweight text/note canvas
- `Runbook`: an executable or replay-oriented document

Terms to avoid unless deliberately revisited:

- `Space`
- `Workspace` as a tree-item label

Reason:

- `Group` is concrete and understandable
- `Paper` is softer and more intentional than `Document`
- `Runbook` communicates “instructions with intent”, not just “text file”
- `Space` is vague and overloaded

### “New” vs “Create”

Do not mix them arbitrarily.

Preferred rule:

- use `New ...` in menus and context menus for object creation
- reserve `Create ...` only for future cases where something is explicitly derived from another thing and the derivation matters

Default recommendation:

- standardize on `New`

## Tree and sidebar behavior

The left sidebar is a real workspace organizer, not a file browser clone.

- It should be dense but calm.
- Icons should be grayscale by default.
- Expanded root emphasis may use blue subtly.
- The tree should support right-click as a first-class workflow.
- Sessions, groups, papers, and runbooks should be visually distinguishable.
- Session rows should not drown users in hashes or duplicate metadata lines.

## Header behavior

Terminal mode should have a header area above the terminal.

That header should contain:

- the session title
- a short precis
- the session mode selector on the right when relevant

It should not contain:

- literal markdown markers like `#`
- noisy fake status cards
- gratuitous terminal framing

The precis should ideally come from the interface model when available, with a sensible local fallback.

## Notifications

Notifications should reduce anxiety, not create it.

- transient in-app toasts should fade away
- notification backlog can exist in a dedicated panel
- sound should be optional
- in-app notifications are the recommended default
- system notifications should be optional, not forced

For fast-moving operations like self-update:

- silent success is not enough
- users should get calm but explicit update feedback

## Motion

- sidebar and right-rail open/close transitions should be modern and restrained
- no flashy animation for its own sake
- state changes should feel smooth, not theatrical

## Debug-only telemetry

Debug-only UI telemetry is encouraged when a UI bug is hard to communicate or verify.

Examples:

- terminal host/font/debug state
- live zoom/debug values
- render/runtime state summaries

Rules:

- telemetry should exist only in debug builds
- it should help reason about the UI without needing screenshots every time
- it must never leak into release builds

## Design workflow rule

Whenever a user corrects visual taste, naming, control behavior, or interaction style in a way that seems durable rather than one-off, update this file.

The goal is that future projects can reuse this document and get the same visual/design interpretation without retraining from scratch.
