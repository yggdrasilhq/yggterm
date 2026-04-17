# DESIGN.md

## Purpose

This file is the reusable visual and interaction source of truth for Yggdrasil applications.

Use it in two layers:

1. `Core System`: reusable design rules that should transfer cleanly across projects.
2. `Project Overlay`: product-specific vocabulary, workflows, and UI emphasis.

When this file is copied into another repo, the default move is:

- keep `Core System`
- replace or trim `Project Overlay`

Do not bury project-only nouns in the reusable sections.

## Core System

### Brand intent

Yggdrasil apps should feel:

- calm
- modern
- lightly premium
- youthful without being toy-like
- crisp rather than ornamental
- soft around the edges, but not soft-headed

They should not feel like:

- a Linux utility panel
- a web admin dashboard
- a noisy IDE clone
- a skeuomorphic toy
- a stack of nested cards inside more cards

The target impression is:

- one clear main workspace
- supportive chrome around it
- low-friction controls
- light, breathable, polished surfaces

### Visual structure

#### Main workspace

The main workspace is the focus.

- It should read like a calm sheet, canvas, or stage.
- In light mode it should generally be white or near-white.
- It may have a soft shadow and mild radius.
- It should feel like it is floating slightly above the surrounding chrome rather than being boxed into it.
- It should not be crowded by decorative headers, nested boxes, or redundant toolbars.
- Whatever the app’s core artifact is, it should feel native to the main canvas rather than pasted inside a widget frame.

#### Supporting chrome

The surrounding chrome should feel supportive, not dominant.

- Side rails should be lighter and quieter than the main canvas.
- A faint blue-to-green fresh tint over a muted neutral base is desirable.
- A light gradient plus blur system is preferred when the platform supports it.
- Rails should avoid heavy borders.
- The shell should feel visually unified rather than partitioned into harsh boxes.
- Titlebar, side rails, and utility surfaces should feel like one seamless scaffold around the floating main canvas.

#### Shape language

- Rounded corners are welcome, but should stay restrained and OS-friendly.
- Outer shell rounding should feel closer to modern KDE/Windows than to exaggerated mobile UI.
- In maximized state, outer window corner radius should collapse to zero.
- Inner radii should be smaller than outer shell radii.

### Color direction

Light mode is the primary reference unless a project explicitly says otherwise.

- Prefer white and pale blue-grey foundations.
- Accent color can lean clean blue.
- Background tint may gently lean sky-blue to green.
- Use contrast carefully; avoid washed-out unreadable controls.
- Keep the main canvas and supporting chrome visually coherent.

Avoid:

- muddy greys
- purple-heavy defaults
- overly opaque frosted layers that bury hierarchy
- gratuitous gradients inside the main content region

### Theming system

Yggdrasil shells should support a reusable visual theme editor.

- Theme editing should be centered on a small floating modal, not a full settings page takeover.
- The editor should feel Arc-like or Zen-like: compact, visual, tactile.
- The core interaction model is:
  - a preview pad
  - draggable color stops
  - a lightweight color library
  - a brightness control
  - a single grain dial control
- Double-clicking the preview pad should be able to add a color stop.
- The preview pad should use a visible grid, not a blank field, so stop placement feels intentional.
- Dragging color stops should live-preview the shell background.
- Light and dark shell modes should remain selectable independently of the custom gradient.
- Saving should persist the theme; cancel should revert live preview.
- Reset should always return to the project’s base shell theme, not an empty placeholder state.
- The active portable theme should be stored in `~/.yggterm/settings.json` under the `theme` object.
- If no custom colors exist, the shell should fall back to the system gradient cleanly.

#### Theme surfaces

- The outer shell background should be theme-driven.
- Supporting chrome should inherit the shell gradient subtly through transparency and blur.
- The main workspace should remain calmer and more neutral than the shell chrome.
- Theme accent can be derived from the dominant gradient stop for lightweight emphasis.
- The theme modal itself should not blur the background. The surrounding UI should remain clearly visible, with a calm blue active-state halo around the modal to signal focused editing.

### Typography

#### Interface font

- Linux: `Inter Variable`
- macOS/Windows: default platform system UI font

#### General text guidance

- small text must still feel antialiased and intentional
- avoid overly thin utility-rail typography
- headings should feel clean and editorial, not shouty
- labels should be concise and legible

Project overlays can define additional content fonts, such as terminal, code, map, or data fonts.

#### Preferred monospace font

- `JetBrains Mono` is the preferred monospace across all platforms unless a project explicitly overrides it.

### Control language

#### Segmented controls

Segmented pills are preferred for compact mode switches.

They should:

- clearly show the active segment
- have a clean outer shell
- avoid muddy selected states
- feel stable and precise

#### Primary buttons

Primary actions should look unmistakably clickable.

- blue background is acceptable for the main affirmative action
- white text
- clear contrast
- enough padding to feel intentional

If a user says “this does not look like a button”, that is a design failure.

#### Inputs

- Prefer clean rectangular or softly rounded input boxes.
- Avoid pill-shaped text fields unless there is a strong reason.
- Inputs must remain visible against the supporting chrome.

#### Search in chrome

- If the product has a global or sidebar search, the default preference is a centered search field in the titlebar.
- The search field should feel like part of the shell, not a floating badge.
- Search should generally be the visual anchor of the center titlebar slot.
- In its idle state, search should read as a single compact field, not a stacked control with helper copy always visible.
- In its focused state, the search result surface should wrap the search field itself into one continuous shell, closer to VS Code command/search behavior than to a detached popover under the field.
- Search typography in chrome should err slightly larger and crisper than default web utility text. Tiny soft-looking placeholder or helper text is a design miss.
- When an app has an active primary artifact such as a session, terminal, paper, or preview, its title should live in the titlebar to the left of the search field rather than consuming a duplicate header inside the main canvas.
- Hovering the title control should expose the summary via tooltip, and clicking it may open a compact dropdown with the fuller summary and related actions.
- Avoid showing both a titlebar title and a second in-canvas title card for the same artifact unless the inner canvas is itself an editor that must edit the title as content.

#### Titlebar density

- Titlebars should be compact and deliberate, with as little dead vertical padding as practical.
- The search field should feel vertically centered with roughly balanced top and bottom breathing room.
- When height must be shaved, remove it from the titlebar scaffold before shrinking the search field into a cramped control.

#### Workspace edge behavior

- When a supporting side rail or right inspector is hidden, the main workspace should run flush to that edge.
- Do not preserve stale gutters where a hidden panel used to be. They read like layout bugs, not breathing room.

#### Context menus

Context menus should feel closer to modern Microsoft app menus than generic web popovers.

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

### Motion and interaction

Motion should be functional, not decorative.

- side panels can ease in and out
- notifications should stack and reflow smoothly
- drag-and-drop should show clear make-way affordances
- state changes should feel crisp, not rubbery

### Notifications

Notifications are reusable shell components, not one-off project afterthoughts.

- In-app toast notifications should be supported by default.
- Toasts should have clear tone coloring.
- Toast stacks should animate upward when items leave.
- Notification history panels are acceptable when the product benefits from persistent event history.
- Clear-one and clear-all actions should be supported when a notification panel exists.
- In-app toasts should usually sit horizontally centered near the top of the app, not pinned to a screen edge.
- Long-running work such as generation, caching, indexing, sync, or remote bootstrap should use reusable job notifications with a visible progress bar.
- Background jobs should not be silent; if the work may take more than a moment, the shell should make that work legible.
- Job notifications should coalesce by task identity instead of stacking duplicate progress cards.

### Update system

Update UX is a reusable shell concern, not project-specific glue.

- Direct-install update flows should reuse the notification and chrome systems.
- Installing an update must not immediately tear down a running productive workspace.
- Preferred behavior is:
  - install in the background
  - notify that the update is ready
  - expose an explicit restart affordance
- Update state should be readable from shell chrome without feeling alarmist.
- If a restart is required, the app should say so plainly instead of silently relaunching itself.

### Debug telemetry

Debug-only telemetry is a design-support component, not just an engineering detail.

- Instrumentation should help explain interaction failures such as drag, selection, layout, or context-menu issues.
- Debug telemetry should be local-first and easy to inspect.
- It should be safe to remove or gate behind debug builds without affecting the product UI.
- If a complex interaction is likely to be reused, the telemetry strategy should be reusable too.
- Debug telemetry must stay physically bounded on disk. Multi-GB observability files are a product bug, not just a debug inconvenience.
- Telemetry files should rotate automatically, and smoke coverage should fail before a workspace can silently accumulate runaway local state.

### Long-running workspaces

Yggterm should be designed for sessions that stay alive for days, weeks, or months.

- A long-lived workspace must survive local daemon restarts, stale sockets, transient helper failures, and app relaunches without dropping into a dead terminal whenever recovery is still possible.
- Restore flows should prefer bounded retry and self-healing over fatal blank or frozen terminals when the underlying failure is a transient local-helper problem.
- Performance work only counts if restore and interaction stay reliable over long runtimes. A faster shell that strands active sessions is not a win.
- Smoke and proof coverage for terminal work should include long-running failure modes, especially daemon-loss recovery and bounded observability retention.

### Drag and drop

If a project has drag-and-drop tree or list reordering:

- explicit `before / inside / after` snap zones are preferred
- a floating drag card is preferred over invisible drags
- hover affordances should show where the item will land
- adjacent snap boundaries must behave predictably
- multi-select drag can use stacked-card visuals
- the final placement must match the visible snap indicator exactly

### Preview surfaces

If a project has a conversation preview surface:

- preview reading mode and runtime/live mode should share one header system
- generated title and summary should be treated as refreshable navigational aids
- preview content should render like content, not raw log lines
- headings, bullets, task items, quotes, and code fences should each have distinct treatment
- overview/graph mode should feel structural, not like the same chat list in a second skin
- overview mode should highlight summary, counts, and message progression before full transcript detail

### Reusable shell guidance

If a project has:

- a main canvas
- left or right rails
- titlebar actions
- reorderable tree/list structures

then the shell should be designed as reusable primitives rather than one-off page markup.

Preferred reusable boundaries:

- drag/reorder engine
- drag ghost / drop-zone visuals
- titlebar primitives
- window control primitives
- rail/panel primitives
- menu and toast primitives
- update-state primitives
- telemetry hooks for interaction-heavy components

### Window chrome specifics

If a project owns its own titlebar/chrome:

- the main viewport should sit visually above a seamless titlebar + rail scaffold
- the preferred top-right control order is:
  - always-on-top
  - minimize
  - maximize / restore
  - close
- these controls should use crisp simple line icons
- minimize/maximize/always-on-top should stay neutral by default
- close should gain a red background with a white `X` on hover
- outer radii should disappear in maximized state

## Project Overlay Interface

Each project should define the following explicitly.

### 1. Main artifact

What is the main canvas actually for?

Examples:

- terminal
- map
- graph
- document
- dashboard

### 2. Navigation model

What lives in the left rail?

Examples:

- sessions
- folders
- machines
- topology nodes
- boards

### 3. Right rail modes

What modes can the right rail switch between?

Examples:

- metadata
- settings
- notifications
- inspector
- filters

### 4. Vocabulary

Define the user-facing nouns here, not in the reusable sections.

Examples:

- session
- terminal
- paper
- folder
- separator

### 5. Domain-specific control rules

Document:

- quick action labels
- context menu labels
- titlebar actions
- view toggles

### 6. Domain content typography

If the main artifact needs a special font, define it here.

Examples:

- terminal font
- map label font
- monospace editor font

## Project Overlay: Yggterm

This section is intentionally project-specific.

### Main artifact

- daemon-owned terminal and session canvas

### Navigation model

- vertical sidebar of sessions, papers, folders, separators, and related terminal workflows

### Preferred user-facing terms

- `Session`
- `Terminal`
- `Paper`
- `Folder`
- `Separator`

Avoid by default:

- `Space`
- `Group` as the primary tree noun
- `Runbook` as the main executable-document noun
- `Workspace` as a tree item label

### Tree behavior

- the tree is a real workspace organizer, not a filesystem browser clone
- it should be dense but calm
- icons should be grayscale by default
- expanded root emphasis may use blue subtly
- sessions should not drown users in hashes or duplicate metadata lines

### Tree creation language

Primary quick actions:

- `+Session`
- `+Terminal`
- `+Paper`

Folder context menu defaults:

- `New Session`
- `New Terminal`
- `New Paper`
- `Add Folder`
- `Add Separator`

### Header behavior

Preview mode and terminal mode should share the same header system.

That shared header may contain:

- the session title
- a generated summary
- a session mode selector when relevant
- refresh affordances for generated title/summary copy

Generated UI copy is not static decoration. It should be treated as refreshable state because long-running sessions drift over time.

Keyboard-first command access should be discoverable.

- pressing `Alt` should enter a visible command-hint mode instead of doing nothing
- hint chips should appear on the live controls they target, not in a detached cheat sheet alone
- multi-step overlays are preferred for creation flows; `Alt` then `I` should expose insert/create actions
- the overlay should stay lightweight and reversible with `Esc`

Hash-like fallback titles are placeholders, not real metadata.

- A fallback short hash should only be used until generated copy exists.
- Short-hash labels must widen until unique among visible session siblings or the active session set.
- Two visible sessions should never share the same temporary hash label.
- Keep this uniqueness rule reusable so sibling apps such as `codex-session-tui` can share the same session-label behavior.

The refresh affordance should:

- be lightweight and inline, not a loud primary button
- sit at the end of the title or secondary line it refreshes
- use the same visual language in preview and terminal mode

Remote-first shell behavior should prefer a Yggterm-owned server path on SSH targets over terminal-text workarounds.

- If a remote machine has `yggterm` available, metadata sync, generated copy persistence, and clipboard/image staging should go through explicit remote Yggterm commands first.
- Shell-typed fallbacks are acceptable only as compatibility bridges, not as the long-term design center.

The header should not contain:

- literal markdown markers like `#`
- noisy fake status cards
- gratuitous terminal framing

### Preview surfaces

Session preview should move toward the quality bar of Open WebUI:

- a clean chat-like message stack for the main reading mode
- a strong graph/overview mode for branch or flow understanding
- one calm shared header above both modes
- generated summary text that helps users re-enter a long conversation quickly

### Paper surfaces

`Paper` is not just a note blob.

It should be able to grow toward richer canvas modes such as:

- writing
- checklist/planning
- calendar views
- kanban-style organization
- spreadsheet-like surfaces

If a paper surface gains structured tools, prefer a ribbon-like strip beneath the titlebar over scattered floating controls.
