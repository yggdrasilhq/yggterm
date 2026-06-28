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
- A light gradient system is preferred for stable desktop shells. Do not ship
  compositor blur or alpha-driven transparency in the stable path; keep blur
  experiments on an explicit experimental branch until they are deterministic
  across focus changes, restore, and platform compositors.
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
- Double-clicking the preview pad should be able to add a color stop.
- The preview pad should use a visible grid, not a blank field, so stop placement feels intentional.
- Dragging color stops should live-preview the shell background.
- Light and dark shell modes should remain selectable independently of the custom gradient.
- Theme edits apply live so the shell can be judged in place. Closing the editor
  persists the current theme; reset returns to the base theme.
- Reset should always return to the project’s base shell theme, not an empty placeholder state.
- The active portable theme should be stored in `~/.yggterm/settings.json` under the `theme` object.
- If no custom colors exist, the shell should fall back to the system gradient cleanly.
- Stable Yggterm exposes brightness only as a scalar control. Alpha,
  translucency, grain, and blur controls are experimental and must not affect
  stable shell rendering.
- The theme editor dialog itself should be opaque to the app behind it, while
  still applying shell edits live around the dialog.

#### Theme surfaces

- The outer shell background should be theme-driven.
- Supporting chrome should inherit the shell gradient subtly without blur.
- Auto-hidden titlebar reveal is chrome, not layout. It must draw over the
  workspace with the same shell tint/gradient language as the visible
  titlebar, and must not resize or vertically shift terminal content.
- The revealed auto-hide titlebar floats on a soft drop shadow ALONE — never a
  hard 1px hairline along its bottom edge. A bright (or even faintly tinted)
  separator line reads as a stray white hairline, most visibly where the chrome
  overhangs the lighter sidebar. The bottom border stays transparent; depth is
  the shadow's job (`titlebar_autohide_chrome_shadow`).
- Transparent desktop chrome must never be alpha-only. The stable material
  stack is theme tint, gradient wash, and enough fill opacity to stay readable
  without compositor blur.
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

There is ONE standard segmented control, `segmented_control_track_style` +
`segmented_control_segment_style`. The track is "snug": it is only a hair larger
than the active segment (3px track padding is the only gap), and the active
segment is a near-edge-to-edge fill with NO drop shadow. The titlebar Web
View/Terminal toggle is the reference look. Every multi-segment MODE switch uses
it — titlebar view mode, the agent-mode selector, Settings Light/Dark,
Notifications App/Both/System. Do not hand-roll a segmented pill with an opaque
track + a lifted (`0 3px 10px`) active chip; that reads as a bg pill much larger
than the selection, which we deliberately retired.

`segment_style(grow, on_chrome)`: `grow` fills the track evenly inside a settings
row; `on_chrome` uses luminance-aware text against the variable titlebar chrome
(vs plain palette text on a card).

This is distinct from a binary on/off SWITCH (track + sliding thumb,
`inline_toggle_*`), used for Auto-hide Titlebar, Sound, etc. — leave those alone.

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
- Titlebar search is centered against the full titlebar, not the remaining space between left and right controls. At narrower widths it must shrink or simplify neighboring controls before it overlaps Connect SSH, overflow, settings, metadata, or window controls.
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
- clean theme-aware surface
- subtle shadow
- compact but breathable row sizing
- strong label clarity

Avoid:

- giant floating glass blobs
- top-left fallback placement
- labels that invent confusing product language
- hard-coded light styling in dark mode

### Motion and interaction

Motion should be functional, not decorative.

- side panels can ease in and out
- notifications should stack and reflow smoothly
- drag-and-drop should show clear make-way affordances
- state changes should feel crisp, not rubbery
- for shell chrome, prefer fast desktop durations with Material 3 style curves: emphasized decelerate when something enters or is revealed, emphasized accelerate when it exits, and the standard curve for small state shifts
- hide/show motion should read as purposeful structure changes, not bouncy flourish; the workspace should feel tighter and more exact after motion, not more playful

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
- Restarting into an update must temporarily protect every recoverable live runtime, whether or not the user explicitly marked it Keep Alive.
- This temporary protection is not the same as Keep Alive. Keep Alive is durable cold-start restore. Update protection is a one-restart safety net.
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
- Live terminal runtimes and durable workspace organization are separate concepts. New terminals are ephemeral runtime attachments by default; a user must explicitly choose `Keep Alive` before a live terminal is restored across restart.
- A normal final client close starts graceful shutdown for live sessions that are not marked `Keep Alive`, removes them from durable restore state, notifies the user, and schedules force cleanup after one hour. This is intentionally different from update restart.
- `Close Terminal`, `Remove From Sidebar`, and `Delete Permanently` must stay distinct. Runtime close kills the daemon-owned PTY; it must not imply stored transcript or workspace-item deletion.
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

### Web View Surfaces

If a project has a conversation Web View surface:

- Web View reading mode and runtime/live mode should share one header system
- generated title and summary should be treated as refreshable navigational aids
- Web View content should render like content, not raw log lines
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
- optional titlebar auto-hide is acceptable, but it should collapse to a thin top-edge hover strip and return with the same chrome background/gradient as the visible titlebar, using a restrained desktop-fast reveal rather than snapping or peeking unpredictably

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

### Brand and mascot

The Yggterm app icon should not read as a generic black terminal square, but it also should not look like a spooky character.

- Mascot name: `Yggi`.
- Role: a small Yggdrasil sprout that keeps sessions alive, protects context, and guides work across machines.
- Personality: alert, warm, capable, and calm. The mark should never feel childish, ominous, or creature-like.
- Core icon shape: the supplied full-color Yggi mascot tile with the checkerboard background removed, using the same friendly sprout character and terminal window composition. The `>_` prompt should remain a strong read and the mascot should stay warm, cute, and professional rather than spooky.
- The icon must still read at 16px and 32px in KDE panels, Windows taskbar, and macOS Dock. At those sizes the prompt and sprout silhouette are the primary signals.
- Keep the app icon visually full-size against neighboring desktop icons: the visible tile should fill the 512px canvas with only a small transparent safety margin, not sit inside a padded thumbnail.
- Keep the app icon full-color and characterful. Keep internal tree/workspace glyphs restrained and mostly grayscale unless a state needs color.
- Maintain the exact transparent Yggi raster under `assets/brand/yggterm-icon-512.png`; `assets/brand/yggterm-icon.svg` may be a packaging wrapper around that raster so Linux scalable icon lookup cannot fall back to an older mark.

### Stability-first product rules

Yggterm is in a stability freeze. New terminal/session features must wait until the existing shell can be daily-driven without losing work, mutating titles unexpectedly, or making terminal input feel unreliable.

### Minimal terminal promise

A Yggterm session should be understood as a durable, snappy automation of a simple terminal routine:

```bash
ssh dev
cd gh/yggterm
codex resume <uuid>
```

The shell may add sidebar placement, metadata, restore state, hot-update protection, screenshots, and app-control observability, but those features are supporting structure. They must not change the fundamental promise: a selected session attaches to the real daemon-owned PTY for that work, renders through xterm.js, accepts normal terminal input, keeps scrollback coherent, and survives view switches without becoming a transcript viewer or a semantic mock.

When debugging terminal rendering, the goal is to make xterm.js render the PTY truth correctly. Do not cover terminal defects with Yggterm-owned decorative layers just to make a screenshot pass. Live terminal prompt backgrounds, cursors, selection, input echo, resize redraws, and Codex status animation must be painted by xterm.js from PTY bytes, terminal attributes, or xterm.js-native renderer APIs such as decorations, not by Yggterm overlay DOM. If a diagnostic compatibility shim is ever needed, it must stay behind an explicit development flag, be rejected by release smokes, and never become a second source of terminal content truth. If a Codex prompt background, cursor, resize redraw, working animation, or typed input is wrong, first trace the PTY bytes, xterm buffer, theme mapping, renderer mode, fit/resize state, and retained-host identity before changing shell chrome.

Operational xterm.js notes, fixtures, and current terminal-rendering hypotheses live in `docs/xterm.md`. Cross-layer source-of-truth failures and the banned shortcut classes live in `docs/architecture-audit-2026-05-16.md`.

The product has three separate identities that must not be conflated:

- `Workspace row`: the durable place in the sidebar tree.
- `Runtime`: the daemon-owned PTY or SSH session that receives bytes.
- `Display copy`: title, precis, summary, Web View text, and generated labels.

Selecting a row may focus or hydrate already-cached data. It must not rename, regenerate, relaunch, or move a runtime unless the user took an explicit action for that side effect.

Each app surface has exactly one source of truth:

- Terminal mode is a live runtime attachment. Its viewport is fed only by daemon-owned PTY bytes, daemon-owned retained scrollback for that same runtime, or an explicit runtime-unavailable error. It must never be fed by generated Web View copy, Codex JSONL transcript blocks, semantic status-card guesses, or display-copy fallbacks.
- Web View mode is a read-only presentation of a session for inspection, similar to a chat transcript. Its source of truth is stored/generated presentation data, not the live PTY. It may show `USER`/`ASSISTANT` style blocks when presenting an agent transcript, but those blocks are illegal in Terminal mode. Internal schemas may still use the legacy `Preview` name for compatibility, but user-facing UI should say `Web View`.
- Display copy is metadata. It can label, summarize, and help users re-enter work, but it never decides which runtime receives input and never repairs a terminal viewport.

The Session Metadata rail is a view-aware, useful summary — not a raw dump of
every stored field. It surfaces, in order: **Session** identity (friendly kind,
machine + local/remote, working dir, title); **Connect** — the verbatim handoff
command to reattach this runtime's PTY from any shell (the daemon's authoritative
`Restore` string, or a literal `ssh <machine>` + `cd <cwd>` for plain shells),
rendered as selectable monospace because re-entering work is the product's core
value; **Runtime** (status, PTY grid size, PID, resume id); and kind-specific
**History** (transcript counts, started/last-active, persistence, rollout file).
Internal bookkeeping (Bytes, Preview Blocks, Launch Error: none, the multi-line
launch shell script, backend internals) is implementation detail and stays out.
- Retained xterm hosts are display caches. If the cache is missing, stale, or corrupt, the rebuild source is the daemon runtime stream/scrollback for that runtime, not Web View text.
- Codex-class semantic state is advisory. Codex welcome cards, `/status` output, prompt wording, and model banners are not stable contracts and must not be used as the primary proof that a terminal is healthy.

### Live sessions and updates

`Live Sessions` is a runtime monitor, not the user's only home for a session.

- Every live local and SSH runtime should appear there while it is alive.
- The original workspace row remains the user's visual bookmark.
- Dragged row order in `Live Sessions` is durable user layout. Focusing or
  switching sessions must not reorder the list; only explicit drag/drop and new
  runtime creation may change it.
- The `X` affordance in `Live Sessions` kills the runtime after confirmation. It does not delete stored transcript history.
- Closing a background live runtime must not move the active viewport.
- Closing the active live runtime should fall back through the validated viewport history: previous live/stored session in its prior mode, previous scoped Startpage, then global Startpage. Closed session paths and aliases must be pruned before choosing this fallback.
- The daemon should not choose an arbitrary replacement active session after removing a runtime. The GUI owns close-time viewport history; the daemon owns runtime truth.
- Keep Alive means durable restore after a normal cold restart.
- Normal app close prunes non-Keep-Alive live rows and gracefully closes their runtimes with a one-hour force-cleanup deadline.
- Update restart protection temporarily treats all recoverable live runtimes as restorable. It must not silently turn unkept sessions into durable Keep Alive sessions.

### Status indicator vocabulary (traffic signal + blue/orange)

One coherent light vocabulary for session state, used by Live Sessions today and Automated Sessions later. The status dot in the live-session rail is the canonical instance; any future surface that signals session state reuses these meanings and colors rather than inventing new ones.

- `GREEN` (`#22c55e`): keep-alive — the session survives the GUI (durable runtime).
- `BLUE` (`#3b82f6`): live but transient — the session lives only while the GUI does.
- `BLINKING` (the `yggterm-status-dot-blink` pulse): the agent is working right now. Blink is an orthogonal modifier — a green or blue dot blinks while its session works and returns to steady when idle.
- `ORANGE/AMBER`: reserved for attention states (recovery in progress, degraded runtime, pending user decision). Not yet wired; when an attention signal is needed, use this slot — do not repurpose green/blue.
- `RED`: reserved for dead/error (runtime lost, unrecoverable). Same rule: reserved, not yet wired.

Rules: color encodes durability class, blink encodes activity, and reserved colors are introduced only with a spec update here. Automated Sessions (experimental/automations) must adopt this vocabulary unchanged so a user reads one signal system across the whole sidebar.

### Stage-curtain loading rule

Session loads must look like a stage production: the audience never sees the mess. Concretely:

- A loading or rebuilding viewport may show, in order of preference: (1) the correct final frame immediately ("so posh we need no curtain"), (2) the previous faithful frame held perfectly still (ghost), or (3) a flat background-colored veil. Nothing else.
- The forbidden in-between states: DOM leaks, partial/truncated rows, stale frames that later "correct", broken bottoms, and any blink between a covering layer and the final frame. A wrong frame must never paint, even for one frame — latency is preferred over flicker.
- The curtain comes down (cover attaches) before any teardown/rebuild churn starts, and is pulled (released) as soon as — and only when — the daemon-sourced final frame is fully painted underneath.
- The endgame is curtainless: host/eval reuse so reveals repaint in place with no rebuild to hide. Curtains are the contract until each load path earns that.

### Startpage

Startpage is a re-entry and scoped creation surface, not a connection-settings surface.

- It may offer recent sessions, new Codex session, local terminal, folder creation, rename, and title/summary editing.
- It should not show `Connect SSH`. SSH connection belongs in titlebar/right-rail/context controls where connection state and settings are available.
- Selecting a folder opens a scoped Startpage without closing or hiding live runtimes.
- Startpage must never be used as a terminal recovery fallback for a closed or broken runtime; it is chosen only by explicit folder/startpage focus or by the close-navigation fallback contract.

### Web View and copy

Web View mode is read-only by default.

- Switching Terminal -> Web View -> Terminal must preserve session identity, title, summary, runtime, scroll intent, and input routing.
- Web View hydration may update the Web View body from existing cache.
- Web View hydration must not rewrite a user title or start LLM copy generation.
- Generated copy is an explicit background job with visible state and a bounded budget, not an incidental selection effect.
- Web View hydration must not write into Terminal-mode buffers, retained xterm buffers, or terminal recovery paths. Web View and Terminal are sibling views over the same session identity, not fallback renderers for each other.

Terminal recipes are experimental. They should not be created implicitly from drag/drop or ordinary session movement unless an explicit development flag enables that behavior.

### Clipboard and media paste

Image paste is a first-class terminal operation.

- The desktop clipboard is read by the local shell/server, not by brittle terminal text hacks.
- Local sessions receive staged files under the local Yggterm home.
- SSH sessions receive staged files through the remote Yggterm helper when available, with the resulting remote path inserted into the terminal.
- Text paste and image paste share the same intentional paste path so `Ctrl+V`/`Cmd+V` behaves predictably across Linux, Windows, and macOS.
- Linux-style primary selection is terminal-local and separate from the desktop clipboard. Selecting text in xterm.js records a primary selection, and middle-click pastes it through xterm.js terminal input so bracketed paste and PTY input semantics remain terminal-owned.
- Terminal right-click opens the normal Yggterm terminal/session context menu through the xterm event bridge. It must suppress the browser/WebKit context menu and xterm helper-textarea paste path on the terminal surface, but it must not paste clipboard text, create terminal-rendering overlays, or create a second menu implementation.

### Terminal control

Terminal focus, input, scroll, selection, and retained-host recovery must have one active controller.

- A terminal that can scroll but cannot type is a broken state.
- A terminal that can type but cannot scroll while the user is reading scrollback is also broken.
- An active visible terminal with a write-frame budget high enough to make typing or TUI animation feel stepped is broken. Write budgets may batch flush timing, but they must never coalesce, trim, deduplicate, reorder, or rewrite PTY bytes before xterm.js parses them.
- Retained terminal hosts may stay mounted only while their active session identity and input policy match the shell state.
- Programmatic layout changes such as titlebar auto-hide reveal/collapse, fit-addon resize, and visible-paint refits must not be interpreted as user scrollback. When the host is in PromptFollow, these changes must converge back to the live buffer bottom; when the user is explicitly in scrollback, the app must preserve that reading position.
- A scroll controller may appear when the user is intentionally away from the prompt, but it is only a YggUI control surface over xterm viewport APIs. It must not draw terminal content, prompt backgrounds, cursors, or line repairs, and release proof must still come from xterm/app-control/screenshot truth.
- Live session switching should feel like attaching Ghostty or xterm to an already-running `screen`/`tmux` session: if the runtime is alive, focusing it attaches to the current stream without relaunching, regenerating, previewing, or replaying transcript text.
- Activity indicators represent real work: `idle`, `running`, `recent-output`, `recovering`, or `kept`. They should not spin for cosmetic debounce after a blank Enter or already-rendered keypress.
- App-control typing proofs should use the same viewport keyboard path a user exercises. Direct PTY writes are still useful for controlled setup, but interrupt bytes such as `Ctrl-C` must not be batched with later line-editing or command bytes.

### Codex-class sessions

Codex and LiteLLM sessions are terminal sessions with extra semantic state.

- The shell should expose whether the session is waiting, thinking, streaming output, running a tool, complete, or recovering.
- Completion should produce a notification and optional sound when notifications are enabled.
- Terminal bell/OSC notifications should flow through the reusable notification system instead of being ignored.
- Codex semantic state must never replace the daemon runtime identity as the input target.

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
- focusing a folder should open that folder's scoped Startpage, clear the active terminal viewport, and leave live runtimes untouched; folder expansion belongs to the disclosure control and keyboard arrows, not the row focus action

### Tree creation language

Primary quick actions:

- `+Session`
- `+Terminal`
- `+Paper`

Folder context menu defaults:

- `New Codex Session`
- `New Terminal`
- `New Paper`
- `Add Folder`
- `Add Separator`

Sidebar iconography is semantic and greyscale by default.

- Use a compact boxed SVG mark with `>_` text for Session and Codex session rows, including live Codex sessions and stored Codex transcripts. The box is the SVG outline; do not encode literal `[` or `]` characters into the mark.
- Use a compact boxed SVG mark with `$_` text for Terminal rows, including local shells and SSH terminals. The box is the SVG outline; do not encode literal `[` or `]` characters into the mark.
- Keep Paper/document icons as the current page mark until the Paper surface is developed further.
- Busy state may temporarily replace the mark with a static spinner-shaped mark, but stable-channel sidebar rows must not run infinite CSS animations. Idle rows must return to the correct boxed mark.

### Header behavior

Web View mode and Terminal mode should share the same header system.

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

### Web View surfaces

Session Web View should move toward the quality bar of Open WebUI:

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
