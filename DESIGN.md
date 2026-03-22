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
- It should not be crowded by decorative headers, nested boxes, or redundant toolbars.
- Whatever the app’s core artifact is, it should feel native to the main canvas rather than pasted inside a widget frame.

#### Supporting chrome

The surrounding chrome should feel supportive, not dominant.

- Side rails should be lighter and quieter than the main canvas.
- A faint blue-to-green fresh tint over a muted neutral base is desirable.
- Rails should avoid heavy borders.
- The shell should feel visually unified rather than partitioned into harsh boxes.

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

### Drag and drop

If a project has drag-and-drop tree or list reordering:

- explicit `before / inside / after` snap zones are preferred
- a floating drag card is preferred over invisible drags
- hover affordances should show where the item will land
- adjacent snap boundaries must behave predictably
- multi-select drag can use stacked-card visuals
- the final placement must match the visible snap indicator exactly

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
- rail/panel primitives
- menu and toast primitives

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

Terminal mode should have a header area above the terminal that can contain:

- the session title
- a short precis
- a session mode selector when relevant

It should not contain:

- literal markdown markers like `#`
- noisy fake status cards
- gratuitous terminal framing

### Paper surfaces

`Paper` is not just a note blob.

It should be able to grow toward richer canvas modes such as:

- writing
- checklist/planning
- calendar views
- kanban-style organization
- spreadsheet-like surfaces

If a paper surface gains structured tools, prefer a ribbon-like strip beneath the titlebar over scattered floating controls.
