<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Core system

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
- **A recolour tool appears when there is something to recolour.** The colour
  library and the colour well both drive the selected stop, and their handler
  returns early when nothing is selected — so on an empty selection they were not
  merely redundant, they were inert: swatches that can be clicked and do nothing,
  which is worse than controls that are not there. They are disclosed on the
  selection, and the library is headed with the stop it will repaint ("Color 2")
  rather than an anonymous "Color Library", so it is never ambiguous which of
  several dots a swatch is about to change. Every route into having a stop —
  starter, double-click, clicking a dot — selects it, so the tools are never
  withheld from someone who has something to edit.
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
- **A hidden sidebar is an auto-hide sidebar.** Hiding the session tree or the
  metadata rail does not remove it; it collapses to a thin hover strip on its
  own window edge and reveals on hover, over the workspace, on the z axis. There
  is no settings toggle — this is simply what hidden means. Both edges use the
  same reveal state machine as the titlebar (`AutoHideSignals`) and the same
  restrained desktop-fast motion.
- **The revealed panel is a floating island, not a slab.** Modelled on Zen's
  hover sidebar: an inset card with a margin on all four sides, rounded corners,
  and a soft ambient drop shadow on every side (`sidebar_card_shadow`) — never a
  border hairline (a 1px border paints the chrome fill and reads as a bright
  separator line; the titlebar's 2026-06-27 lesson). The workspace shows around
  it. **Always-visible (docked) is unchanged — same seamless in-flow panel as
  before.** Only the hover-reveal is a card.
- **The card's corner radius matches the viewport's** (`SIDEBAR_OVERLAY_RADIUS_PX`
  == `terminal_frame_style` host_radius, 10px). The floating panel and the
  terminal it floats over must read as the same rounded language (user 2026-07-21).
- **Both sidebars are independently draggable.** Each resizes from its INNER
  edge — the one facing the workspace — with its own clamp and its own persisted
  setting (`tree_width`, `rail_width`). A drag widens a panel when it moves AWAY
  from the edge that panel is docked against, so the sign belongs to the edge
  (`SidebarEdge::resize_delta_sign`) and not to the panel; that is what lets the
  chrome mirror flip both drags without either panel learning about it.
- A revealed sidebar overlay must never resize the workspace. It is positioned
  out of flow, so the terminal keeps its exact `cols × rows` before, during and
  after a reveal. This is a correctness rule, not a preference: a reflowing
  sidebar would re-fit the xterm and push a new PTY grid to the daemon on every
  hover.
- **One geometry, fixed CSS keys across every state** (`sidebar_panel_outer_style`
  / `sidebar_panel_card_style`, keyed by `SidebarPanelMode`). Docked, collapsed
  and revealed must emit the SAME property keys — only values change. Dioxus
  applies `style` property-by-property and does not clear a key the next render
  drops, so a divergent key set left overlay props (`position:absolute`,
  `z-index`, `box-shadow`) lingering when a rail toggled back to docked, floating
  it as a transparent ghost over the viewport (fixed 2026-07-21).
- Transparent desktop chrome must never be alpha-only. The stable material
  stack is theme tint, gradient wash, and enough fill opacity to stay readable
  without compositor blur.
- The main workspace should remain calmer and more neutral than the shell chrome.
- Theme accent can be derived from the dominant gradient stop for lightweight emphasis.
- The theme modal itself should not blur the background. The surrounding UI should remain clearly visible, with a calm blue active-state halo around the modal to signal focused editing.

#### Mirrored chrome (the vertical-axis flip)

The app chrome can be reflected about the window's vertical centre line, as a
persisted user preference (Settings → Window Chrome → **Mirror Chrome**). Some
people want the tree under their dominant hand; this is a taste, so it is a
setting rather than a redesign.

- **ONE owner decides which side anything is on:** `ChromeOrientation` in
  `yggui-contract`, persisted as `AppSettings::chrome_orientation`. Surfaces ASK
  it a question — `orientation.edge(ChromeSlot::Tree)` — and are handed a
  `SidebarEdge`. Nothing else may test the mirror to pick a side. If two places
  could answer "which side is the sidebar on?", collapse them into this type.
- **`ChromeSlot` names WHAT, `SidebarEdge` names WHERE.** A slot is `Tree` or
  `Rail`; an edge is `Left` or `Right` of the *screen*. Any identifier that
  fuses the two ("LeftSidebar", "sidebar-right") is a lie waiting for the first
  user who flips the toggle, in a place the compiler cannot see.
- **The search box is the axis and never moves.** Everything left of it goes
  right and everything right of it goes left: the cwd tree with its `☰` toggle,
  the two-phase Web View/Terminal toggle, the `+` menu and the session chip on
  one side; the metadata/settings/notifications/app-pane rail with its trigger
  buttons on the other.
- **A mirror reflects the ARRANGEMENT of controls, not the inside of a
  control.** The titlebar clusters swap edges *and* reverse, so the button
  nearest the search box stays nearest the search box and the `☰` stays against
  the tree it opens. A single control's own parts keep their order — the
  segmented Web View | Terminal toggle, the window-button strip, a row's label
  and its trailing actions. Those are one thing, not an arrangement.
- **The window buttons do not mirror.** Minimise / maximise / close belong to
  the platform and stay where the platform puts them, physically outermost on
  their own edge, riding whichever app cluster shares it. The macOS traffic-light
  inset follows the physical left edge for the same reason.
- **Everything directional follows the edge, not the panel**: the panel's
  anchor, its collapse slide, its drop shadow, its resize grip and drag sign,
  the side a titlebar popover grows toward, the square corner of a panel
  attached under a tab, and which panel a page-edge hover reveals. A page-edge
  claim is read off the panel's OWN RECT rather than its selector, so the
  geometry the web surface is placed against can never disagree with the DOM.
- **Every mirrored style must emit an identical property-key set in both
  orientations** — `left` *and* `right`, `flex-direction` in both arms. Dioxus
  applies `style` property-by-property and never clears a key the next render
  drops, so a one-sided anchor survives un-mirroring forever. This is the same
  trap as `SidebarPanelMode`, and it bites harder here because the toggle is
  designed to be flipped back and forth.
- **Content is not mirrored.** The interface stays left-to-right: tree rows
  indent from their leading edge, disclosure chevrons still mean down-is-open
  and right-is-closed, back/forward arrows keep their browser meaning, and text
  stays left-aligned. This is a chrome preference, not an RTL locale, and
  treating it as one would make every label read wrong.

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

#### The surface switch has ONE home

"What is this session's viewport showing" is answered by exactly one control:
the titlebar slot (`.yggterm-titlebar-view-toggle`, driven by
`TitlebarSurfaceSwitch`). It shows an agent CLI's **Web View | Terminal**, or a
libyggterm app's **Document | Terminal**, or nothing — and when it shows
nothing it keeps its footprint, hidden and inert, so the titlebar does not
shuffle.

⛔ **Never float a second copy over a viewport.** A terminal and a document
surface both fill their rect edge to edge and reserve no space for chrome, so an
`position:absolute` pill lands on top of the content — which is exactly what
happened to yedit: the Document|Terminal pill drew over the first line of the
document it controlled. Chrome that needs space takes it in FLOW, in the
titlebar, or it does not exist.

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
  - Standing exception: the web-surface (ychrome) address bar is a pill —
    the surface deliberately mimics Chrome's omnibox vocabulary so users
    don't miss the browser UI they know.
- Inputs must remain visible against the supporting chrome.

##### A field is one control, and it ANSWERS

There is ONE text field in a Yggdrasil app. The omnibox, the titlebar search, a
Settings box, an app pane's input, the vault's form and yedit's editor are the
same control wearing different geometry, and they take their skin from one
owner: `text_field_css` in `yggterm-shell`, reached by wearing
`data-yggui-field`. A surface that hand-rolls a fill and a hairline is a second
encoding, and it always drifts (the in-place rename field carried a white fill
and a grey border, which was a light-theme-only box, until 2026-08-02).

**The skin is a STYLESHEET, and it has to be.** Hover, focus and placeholder are
CSS *states*; an inline `style` attribute cannot express one. That is why every
field in this app was flat and inert until the user reported it (2026-08-02:
*"I want the input box colors on ychrome omnibox, vaults, and in yedit to look
youthful and not lifeless too"*). A field that does not answer the pointer or
the keyboard is a picture of a field.

⛔ **The style function emits the BOX, never the FILL.** An inline `background`
out-specifies a stylesheet, so a single one silently kills hover and focus for
that surface. `settings_input_style` and `web_chrome_input_style` therefore emit
padding, radius and type — and no `background`, no `box-shadow`.

The tokens, all derived from the theme accent so a re-themed shell re-themes its
fields:

| Token | Light | Dark |
|---|---|---|
| Resting fill | `accent 5% / white 72%` | `accent 7% / white 4.5%` |
| Hover fill | `accent 9% / white 88%` | `accent 12% / white 7.5%` |
| Focus fill | `accent 7% / white` | `accent 15% / white 8.5%` |
| Resting hairline (inset 1px) | `accent 22% / slate 28%` | `accent 26% / white 14%` |
| Hover hairline | `accent 42% / slate 30%` | `accent 48% / white 18%` |
| Focus ring | `0 0 0 1px accent, 0 0 0 3px accent 26%` | same |
| Placeholder | `muted` at 0.9 | `muted` at 0.9 |
| Radius / padding | 10px / 8px 11px (rail), 9px / 32px tall (settings) | same |
| Transition | 140ms `cubic-bezier(0.2,0,0,1)` on fill + shadow | same |

- **The focus ring is the dialog focus ring's vocabulary** (see *Keyboard focus
  ring*): the accent, outside the control, following the control's own radius.
  One ring language in the product, not one per control family.
- **A pill keeps its own border** (`data-yggui-field="pill"`): on the omnibox and
  the find bar that border carries STATE — red when a find has no matches — and
  a second inset hairline would double it. It takes the fill, the hover and the
  focus ring like everything else.
- Fields sit on **section cards** in a form (see below), never floating on the
  bare rail.

##### A stored value: mask dots, an eye, a copy — ON the field

A field that holds something it is not showing draws a **fixed-length run of
mask dots**, and its verbs sit inside its trailing edge. This is Bitwarden's
Edit Login shape and it is what the user asked for by name (2026-08-02: *"I
cannot see passwords in edit mode"*), about a form whose boxes were all blank —
a blank box says "there is nothing here", which was a lie about the entry.

- **The dots are a PLACEHOLDER, never a value.** A placeholder cannot be
  submitted, cannot be read back by an action, and vanishes on the first
  keystroke. That is what lets a secret-free form show that a secret exists
  without ever holding one.
- **The length is fixed**, because a real value's length is itself information
  about it.
- **A revealed value is display-only.** It reaches the DOM so the user can read
  and copy it, and never the form draft — otherwise pressing the eye and then
  Save would re-send a password nobody touched.
- **The verbs are quiet at rest and lit on hover**, in the accent, and are
  reserved room by the box so a long value ellipsizes behind them rather than
  running under them.
- **No eye for a value that is not there.** A field the entry does not hold is
  an ordinary empty box that ADDS one, with its own placeholder.

#### Section cards (a form is a card, a list is not)

A form group — heading plus the fields under it — sits in a card:
`settings_section_card_style`'s fill and inset hairline, 14px radius, 11–12px
padding. The Settings rail is the reference; the user named it as the in-house
liveliness standard (2026-08-02) against a vault pane that read as *"lifeless
with dullness everywhere"*: one undifferentiated grey column of prose and blank
boxes with no rhythm to it.

- **Opt-in, per section, and it stays opt-in.** A card around a long list is a
  stack of nested boxes, which the Brand intent rules out by name. A file tree,
  a tab rail, an item list: no card.
- **The heading is the structural voice** — 10px, 800, uppercase, tracked
  0.07em, in the TEXT colour. Muted-on-muted headings are what made the column
  read as one block.
- **Explanatory prose is kept and demoted**: muted, 10.5px, `text-wrap:pretty`,
  under the control it explains rather than above it.
- **A form's primary action is PINNED**, not scrolled: a rail form is taller
  than the rail, so Save lives in the pinned footer bar and wears the accent.

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

##### Structure: grouped by intent, destructive last

A menu is a sentence about a thing, and it reads in one order:

1. **create** — the verbs that make something new
2. **act on this thing** — reload, copy, duplicate, split
3. **arrange** — file it, rename it, move it
4. **destroy** — close, delete

Separators mark the group boundaries and nothing else. A divider that is not a
change of intent is decoration.

The destructive group is **last, always**. Opening a menu with its irreversible
verb puts that verb under the pointer the instant the menu appears, and the
pointer is already moving.

##### Icons: an opt-in column, greyscale, never emoji

Modern Microsoft menus have an icon column, and so may ours — but it is a
property of the **menu**, not of the item. A menu draws the column when at least
one of its entries carries a mark, and then every entry reserves the slot,
including the ones with no mark: a half-indented list reads worse than none. A
menu that opts out is drawn exactly as it would be if the column did not exist.

Marks are stroked SVG paths in `currentColor` on a shared box, from a **named
set** — never path data invented at the call site, which is how a menu ends up
with three different close marks. `currentColor` is what makes a mark inherit
its row's tone, so the destructive red and the dimmed grey reach the icon
without anyone maintaining a second palette. Emoji fail both halves of this:
full colour, and a different metric on every platform.

##### Depth: a submenu is a page of the same menu

There is ONE menu component. A nested list is that component, at the same
anchor, showing a different list — a page turn, with a `Back` row and a heading
that says where you are. The entry into a page ends in `▸`.

This is not merely an implementation convenience. A submenu that scales with
something the user controls (their folders, their profiles) must not be
flattened into the parent list, where it pushes the item's own verbs off the
bottom; and it must not become a second popover, which is a second thing to
place, dismiss, theme and badge.

##### An unavailable verb is shown, dimmed, and says why — in the tooltip

Never omit an item because it would not work right now. A menu whose shape
depends on state teaches the user that a verb exists only by accident, and an
item that just goes dim is indistinguishable from a bug. Dim it and give it a
reason. A dimmed item also loses its accelerator — the keyboard must never reach
a verb the mouse cannot.

**The reason belongs in the tooltip, never in the label.** A label is the
command's NAME; our justification for dimming it is not part of that name, and
the user must never read one as the other. Appending it produced entries like
`Close tab — this is the app's own tab; quitting the app closes it` in a menu
~216px wide, which rendered as `Close tab — this is the app's own ta…` — not one
dimmed verb in the menu could be read.

Follow through on the same rule for anything else that is *about* a command
rather than its name: a consequence worth stating ("its tabs return to the
root") is a note on the tooltip, not a parenthetical in the label.

##### A label must fit the narrowest menu we draw

Menus are narrow — a rail-banded menu is about 216px, and the icon column and
padding take ~60 of that. Any label the shell authors has to fit what is left.
Ellipsis and a tooltip are the safety net for labels carrying *user* text (a
folder's name, a page title), whose length is not ours to choose; they are not a
licence to write long ones.

Every entry carries a `title` regardless, so a name the box did have to
ellipsize is still readable somewhere.

##### A heading only when it says what the row cannot

A context menu opens at the pointer, directly under the row it was raised on —
a row that is highlighted and already showing its own name. Repeating that name
as the menu's header stacks the same words twice and spends a line of a narrow
box on nothing.

So a heading has to earn its place by saying something no row on screen does:

- **which page** you have walked into (`Move to folder`)
- **how many** rows the menu will act on (`3 selected items`)
- **which surface** raised it, when the surface has no labelled row at all
  (`Terminal`, `Editor`, `Profile`)

A menu with nothing of that kind to say draws no header at all, and takes the
whole row with it — not a blank band of padding where the header used to be.

##### Undo, not "are you sure?"

A bulk destructive verb names its count in the label ("Close 12 other tabs") and
is **reversible**, rather than guarded by a confirmation dialog. A modal in
front of every destructive action taxes the correct ones to catch the rare wrong
one. The toast that reports the action names the undo, at the moment it happens
— telling the user afterwards where the escape hatch is beats making them find
it.

#### KeyTip badges

A KeyTip badge is a context menu containing nothing but one letter. It inherits
the context-menu treatment above at badge scale: modest radius, clean theme-aware
surface, subtle shadow, strong glyph clarity.

That means:

- its own little block, floating above the chrome — never inline text inside a button
- painted in an absolutely-positioned overlay, so showing the badges moves nothing underneath
- anchored to the affordance it names, overlapping its lower-leading corner, nudged inward at viewport edges
- one block per node, even when the tip is two glyphs (`AL`, or a contested group's `N`)
- uppercase, tabular, high contrast against the badge surface

Avoid:

- a scrim or dimming pass over the app (Excel does not dim to show keytips)
- badges that reflow the titlebar or any other chrome when the layer opens
- low-contrast pills that read as decoration rather than as a key to press

Spec: `docs/alt-keytips.md`.

#### Keyboard focus ring (dialogs in Form mode)

Every dialog offers two keyboard routes (`docs/alt-keytips.md` §12.4): its own
ALT+ badge layer, and a focus path with Tab and arrows. Whenever the second one
is in use the dialog must show where the keyboard is. The ring is one treatment,
not a per-dialog invention:

- a 2px ring in the theme accent, offset 2px outside the control, following the
  control's own corner radius
- shown for keyboard focus only, and NOT by `:focus-visible` alone: that
  pseudo-class does not match focus seated programmatically, so a dialog opened
  with the mouse showed no indicator at all. The dialog is stamped the moment our
  keyboard machinery moves focus, and the stamp clears on a pointer press inside
  — so a mouse click still leaves no rings scattered behind the pointer
- it overrides an inline `outline:none` (many controls set one), because an
  invisible focus indicator is not a style preference, it is the navigation
  being broken
- on a colour chip or swatch, the ring sits OUTSIDE the swatch so it never
  changes the colour being judged
- the focused item of a group is the one Tab returns to (roving `tabindex`), so
  the ring and the selection styling must be visibly different: selection is the
  chip's own fill/border, focus is the ring around it

Avoid:

- removing the ring for looks (`outline: none` with nothing in its place)
- a ring so faint it cannot be found on a busy dialog
- reusing the selection treatment as the focus treatment — the user then cannot
  tell what pressing Space would do

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
- **The anchor belongs to the viewport, not to the toast.** Top-centre is the
  default and is right over a terminal, whose newest output and prompt live at
  the BOTTOM. Over a document the top of the viewport is the title and the first
  line being read, so the stack moves to the bottom corner **on the rail's
  edge** — directional chrome follows the mirror. `ToastAnchor` owns all three
  placements and every arm emits an identical style-key set; a bottom anchor
  reverses the stack so the newest toast stays nearest its edge.
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

### Session-style rows (the shared row engine)

Every session-style list row — cwdtree sidebar rows, the WebTabs rail, app-pane
`list-row`s — is drawn by ONE vocabulary: `[indent] [status-dot] [icon]
[title(+subtitle)] [badge] [actions]`. The source of truth is
`SessionRowDensity` + `session_row_metrics` + the `session_row_*_style`
functions and the `SessionStyleRow` component in shell.rs.

- Two densities of the same vocabulary: `Sidebar` (the cwdtree main row's
  numbers — 20px icon box, 9px status column, font 12, radius 12, 12px indent
  base and step) and `Rail` (right-rail lists — padding 5/8, radius 8, 8px
  indent base, 19px indent step, NO separate status column).
  **TYPOGRAPHY AND THE ICON BOX ARE IDENTICAL ACROSS DENSITIES** — an 11px rail
  title next to the 12px tree read as a different font (user-caught
  2026-07-17), and that rule does not bend. Selection is the tint, never a
  weight change.
- **A density may differ in its LEADING ANATOMY, and one does.** The cwdtree
  shows TWO leading marks at once — a live session's keep-alive dot BESIDE its
  kind icon — so it pays for two columns: a 9px status rail, then the 20px icon
  box. A right-rail row never shows two: a folder has a glyph and no dot, a web
  tab has a loading dot and no glyph. Paying there for a column that is empty in
  every row that draws cost 15px of a ~220px rail, which is what the user
  reported on 2026-07-31 ("significant waste of horizontal space on each row").
  So `Rail` has ONE mark column, 20px wide: the icon sits in it, and the status
  dot rides it — centred when it is the only mark, a bottom-right BADGE when it
  shares the column with an icon.
- **The leading slot is laid out even when the row has no mark at all**, in both
  anatomies, so an appearing dot never shoves the title sideways and two rows of
  one list never start their titles at different x. Dot paint comes from the
  status-indicator vocabulary (`live_session_status_dot_style` /
  `web_tab_loading_dot_style`).
- **An EMPTY ELEMENT IS NOT AN ABSENT SLOT.** A surface that has nothing for a
  slot passes `None`; `rsx!{}` passes something that happens to draw nothing,
  and the row dutifully reserves its box. That is precisely how every web-tab
  row in the rail came to carry a 20px icon box and a chevron gap it never
  drew.
- **A row's leading mark is the page's own identity, or nothing.** The web-tab
  rail's rows (a group's head row included) wear the page's FAVICON from the
  engine's own database — never a folder glyph standing in for a page, and
  never an invented icon (user report 2026-08-29: "there should not be a
  folder icon in the row header"). A row the database has served nothing for
  has an ABSENT mark: the always-laid-out mark column keeps titles aligned and
  the loading/media dot rides it.
- **A SCROLLER WHOSE ROWS CARRY RIGHT-EDGE VERBS MAY NOT USE AN OVERLAY
  SCROLLBAR.** An overlay scrollbar paints and hit-tests over the right strip
  of its content, so on the web-tab rail it silently swallowed every click on
  a row's ✕ / + / chevron — the user could not close a row or collapse a group
  at all (2026-08-29). The contract: the scroller is stamped
  `data-web-tabs-scroll` and styled by `WEB_TABS_SCROLL_CSS` (fixed 9px slot,
  viewport tokens); a STYLED scrollbar lays out beside the rows instead of
  over them. Any future scroller whose right edge is interactive inherits this
  rule, not a copy.
- Selection tint is the palette's `accent_soft` everywhere. Never a
  per-consumer mix.
- Trailing actions use `session_row_action_button_style`; a trailing pill uses
  `session_row_badge_style` (ychrome profile badges live here).
- **THE TITLE TRACK IS THE WHOLE ROW, AT REST AND ON HOVER ALIKE.** The trailing
  `actions` are IN FLOW and `display:none` at rest. Hiding them with
  `opacity:0` is NOT enough and never was: an invisible button is still a layout
  box, and on the cwdtree the hidden ✕ was taking 18px plus its 6px gap off
  every live-session row's title (user report + measurement, 2026-08-01: *"rows
  [should] use the horizontal real estate to display the heading occupied by the
  'X' to the entire width"*). `display:none` costs no width at all, so the title
  still gets the whole row at rest — which was the ask. Revealed, the verbs take
  their space and the title ellipsizes to fit.
  - **⛔ NO BACKGROUND BEHIND THE VERBS. Not a chip, not a fade, not a
    `backdrop-filter`, not a `linear-gradient`, not a `color-mix` wash.** This
    is a settled user decision and it has been reverted once already, so it is
    written as a prohibition rather than a preference.
    The first cut floated the verbs OUT of flow on a frosted chip precisely so
    the title would never reflow on reveal — blur plus a wash of the surface
    colour, feathered by a mask, bled over the row's padding to wear its rounded
    edge. The reasoning was sound and the result was rejected on sight, on every
    cwdtree at once (user, 2026-08-01, with a screenshot): *"the white bg effect
    on the close buttons in both cwdtrees look so ugly. Let us not have a close
    button bg like it used to and truncate the row line to make room for the
    button."* The title ran UNDER the chip and the ✕ read as a smudge rather
    than a button.
  - **The reflow-on-hover is the POINT, not a regression.** Yes, revealing the
    verbs re-measures the title; that is what a truncating row is supposed to
    do and what every file tree does. It is what makes the ✕ legible against the
    row rather than against the text under it. Do not "fix" it by floating the
    verbs again — that is this exact bug, and `session_row_hover_css` carries a
    test (`the_row_reveal_rule_has_one_owner_and_reaches_every_surface`) that
    bans `backdrop-filter`, `linear-gradient` and `color-mix` from the rule by
    name so the chip cannot come back quietly.
  - **The rule reaches every row family from ONE owner**, which is why this bug
    presented in three places at once: the yggterm sidebar cwdtree, ychrome's
    tab-rail group rows (whose expand / + / ✕ buttons sit in the same
    `actions` container) and yedit's tree all wear it. Fix the owner,
    every surface inherits; there is no per-app copy to chase.
  - **THREE reveal triggers, and all three are required**: row `:hover`, the row
    being active/selected, and `:focus-within` on the ROW. Mouse-only would
    strand the ALT/KeyTip keyboard layer — a row reached by keyboard would offer
    verbs nobody can see. One owner, `SESSION_ROW_HOVER_CSS`, injected ONCE at
    the shell root; the cwdtree inherits it by wearing the same
    `data-session-row*` marks as the rails. A per-surface copy of this rule is
    what let the ychrome tab rail show a ✕ on every row at rest while the tree
    three pixels away hid its own.
- New list surfaces MUST consume the engine (component, or the style functions
  when the interactivity is bespoke, as the cwdtree row does). Do not write a
  fourth row style.
- **A row list is a TREE, and there is one of them.** Nesting is `depth` on the
  row (indent comes from `session_row_metrics`, never a hand-written padding).
  The cwdtree sidebar, the WebTabs rail and contributed `list-row` panes all
  draw this way; a surface that grows its own indent arithmetic is the bug this
  rule exists to prevent (the rail hand-rolled it until 2026-07-31).
- **How MUCH a level indents is the density's answer, and the rail's is bigger
  than the tree's** (19px vs 12px, user-asked 2026-07-31: "2 spaces worth of
  more indentation in the folder" — the row's own 12px Inter measures 3.375px
  per space, so two of them is 6.75px, and 12 + 6.75 rounds to 19). The tree
  spends a DIFFERENT icon on every level — machine, then folder, then session
  kind — so indent is only part of what carries depth there. Every rail row
  wears the same mark, so indent is the whole of it and has to carry more.
  This is a per-density number, not a per-surface one: a second surface at
  `Rail` density inherits it rather than choosing again.
- **A GROUP ROW WEARS TWO MARKS, and the cwdtree owns both.** The LEADING icon
  slot carries `RowFolderIcon` — **FILLED when the group is open, OUTLINE when
  it is shut** — and the TRAILING `expander` slot carries
  `RowDisclosureChevron` (down = open, right = shut) beside the group's count.
  The fill is the state at rest; the chevron is the control. Both slots are
  shared components with exactly one owner each: a surface that inlines a
  folder path or a triangle of its own is the drift this rule ends (the rail
  drew a chevron and no folder at all until 2026-07-31, and the user reported
  it as "folders should have a folder icon … just like yggterm cwdtree").
  The `expander` slot is ALWAYS visible, unlike `actions`, which are
  hover-revealed verbs: expand/collapse is not something you should have to
  hover to discover.
- **Folders sit ABOVE loose rows AT EVERY LEVEL** in any list that has both
  (user, 2026-07-30). Organization first, then the working set — a folder's own
  sub-folders precede its leaf rows exactly as root folders precede root rows.
- **A folder holds folders, arbitrarily deep** (user, 2026-07-31). Depth is not
  capped anywhere: not in the model, not in the draw walk, not in the drop
  rules. The single move that is refused is a group into its OWN descendant,
  which would erase the subtree.
- **Renaming happens IN PLACE**, in the row, with the existing text SELECTED —
  a row born with a placeholder name ("New folder") must take the first
  keystroke as a replacement. Double-click is the gesture; the row's context
  menu is the discoverable route to the same thing.

### Drag and drop

If a project has drag-and-drop tree or list reordering:

- explicit `before / inside / after` snap zones are preferred
- a floating drag card is preferred over invisible drags
- hover affordances should show where the item will land
- adjacent snap boundaries must behave predictably
- multi-select drag can use stacked-card visuals
- the final placement must match the visible snap indicator exactly
- **before/after draw as a 2px accent LINE on the edge that will receive the
  row; inside draws as a 2px accent RING around the whole group.** A line under
  a folder header would promise "beside the folder" while the drop lands in it.
- **EXACTLY ONE ROW WEARS THE INDICATOR, AND THE FUNCTION THAT DRAWS IT TAKES
  THE `Option`.** The no-drop state is a state of the same style function, not
  the absence of a call: `drop_edge.map(…).unwrap_or_default()` emits an empty
  string, Dioxus applies `style` property-by-property and never clears a
  property the next render omits, so the last drop's `box-shadow` stayed painted
  on every row the pointer had crossed. One drag left an accent line above ~10
  rows at once (user screenshot, 2026-08-01). Same hole cost `border-radius`,
  which only the `inside` ring ever wrote. **Every state emits every key, values
  only differing** — see "the fixed-property-key invariant" wherever a style
  varies with state.
- **One drag grammar per window, and it is ONE OBJECT.** Every row list drives
  `yggui::RowDragGesture` — the WebTabs rail, every contributed app pane, and
  anything added later. There is one gesture in the shell because there is one
  pointer. A surface supplies its scope, its rows and its own meaning for
  "expand"; it supplies nothing else. The full experience the gesture owns:
  - **press-travel threshold** — a press is a click until the pointer travels
    `yggui::DRAG_BEGIN_THRESHOLD_PX` (6px, measured as distance, not per axis);
  - **the dragged row DIMS** to the shared row engine's `opacity:0.58`, never a
    hand-written fade;
  - **a floating GHOST CARD** follows the pointer from the window ROOT — not
    from the list — so it does not freeze at the list's edge, and it names the
    landing ("Drop inside Work") from one owner, `row_drop_target_hint`;
  - **the drop indicator** is the line/ring rule above, drawn on the row's OUTER
    box so a border radius cannot clip it;
  - **SPRING-LOADED AUTO-EXPAND** — rest a drag inside a SHUT group for
    `yggui::ROW_DRAG_SPRING_MS` and it opens under the pointer, once per group
    per gesture. Without it a collapsed folder is a wall and filing something
    two levels down costs one drop-open-drag-again per level. yggterm decides
    WHEN; the surface performs the expansion, because "expand" means a
    different write in each one;
  - **ESCAPE abandons** a live drag, everywhere, at once;
  - **release over nothing is abandoned** — never a guess, never a silent move
    to the root — and leaving the list forgets the TARGET while keeping the
    gesture (row-to-row movement inside a list produces leave events too);
  - **the drag's own release does not also CLICK the row.** A committed drop
    suppresses the click for `yggui::ROW_DRAG_CLICK_SUPPRESS_MS`, so moving a
    row never also opens it.
  A drop is resolved by `yggui::reorder_row_tree` (a flat list is its degenerate
  case), reached THROUGH the gesture, never around it. A surface with its own
  ordering arithmetic — or its own copy of any affordance above — is a second
  source of truth.
- **The contributed-pane wire schema is how an app inherits all of it.** A pane
  declares `list-row` with `depth` / `expanded` / `expand_action` /
  `reorder_action` and gets the whole experience with no app-side code. Absent
  `depth`/`expanded` means a flat list, which is what every pane written before
  nesting declares.

### Web View Surfaces

If a project has a conversation Web View surface:

- Web View reading mode and runtime/live mode should share one header system
- generated title and summary should be treated as refreshable navigational aids
- Web View content should render like content, not raw log lines
- headings, bullets, task items, quotes, and code fences should each have distinct treatment
- overview/graph mode should feel structural, not like the same chat list in a second skin
- overview mode should highlight summary, counts, and message progression before full transcript detail

#### A conversation is a TIMELINE, not a list of messages

**Most of an agent session is not prose.** Roughly 57% of a Codex rollout and
96% of a Claude Code transcript is the agent working — commands it ran, files it
changed, what it was thinking. A Web View that draws only the prose is not a
tidier conversation, it is *half* of one, and it reads as stale next to the
terminal beside it. So the surface draws three kinds of entry, and they are
deliberately unequal:

- **Prose** — the conversation. Reading typography, full width, the visual
  weight. This is what the user came for.
- **A tool call** — one row, FOLDED by default: `[mark] Tool — headline`, in the
  monospace face, at a size and tone BELOW prose. It is context, not content.
- **Reasoning** — the same row treatment, labelled `Thinking`. Both CLIs render
  their own thinking collapsed; a transcript view that splays it open is
  showing the user something they did not ask to see.

**The folded row must identify the call on its own.** A tool call whose folded
line cannot say WHICH command or WHICH file is a row the user has to expand to
recognise, which defeats folding — so the reader extracts a headline (the
command, the path, the query) and the row shows it. The headline is *user* text
of unbounded length: it ellipsizes on one line and keeps a `title` tooltip, per
the label rule above. It never wraps the row into a paragraph.

**The whole row is the control.** Expand/collapse is not something a reader
should have to hover to discover — the same reason the row engine's `expander`
slot is always visible while `actions` are hover-revealed.

**A change shows its stat, and its files.** A call that edited something carries
`+N −M` on the folded row and its changed files as chips when expanded — trailing
path segments, not whole absolute paths, which are identical for every file in a
repo and push the identifying part off the end. Chips cap at four and then count
(`+7`), exactly as an overflowing list does everywhere else.

**Marks come from a named set keyed by what the tool DOES** — command, file
change, file read, search, thinking — never by its name, so a CLI calling its
shell tool `exec_command` and one calling it `Bash` wear the same mark. Stroked
paths in `currentColor` on a shared box, as the context-menu rule already
requires; the row's own tone reaches the glyph without a second palette.

**A failed call is dimmed toward warning, not painted `RED`.** `RED` is reserved
by the status-indicator vocabulary for a dead runtime; a command that exited
non-zero is a normal event in a working session and must not borrow the signal
that means the session is gone. The diff stat's *removed* side reuses that same
warning tone rather than introducing a third red — both mean "this went away",
and two reds in one row would read as two meanings.

**Every message can be copied.** A transcript exists to be taken somewhere else.
Without a per-message copy the user is re-selecting prose by hand out of a
virtualised list, where the rows they are dragging across may not be mounted.

#### The conversation is a SHARED component set, and it lives in `yggui`

⛔ **This surface is not shell markup.** It is
`yggui::conversation` in libyggterm — `ConversationColumn`,
`UserTurn`, `AssistantTurn`, `SystemTurn`, `WorkGroup`, `WorkRow`, `DiffStat`,
`ChangedFileChips`, `TurnDivider`, `WorkingIndicator`,
`ConversationEmptyState`, all reading from one `ConversationTokens`. yggterm's
Web View is an ADAPTER onto it, and so is every other Yggdrasil app that shows
an agent timeline. A second hand-rolled chat in any of them is the bug this
extraction exists to prevent.

The module owns the SHAPE of a conversation and deliberately not the message
BODY: each host keeps its own content pipeline (`PreviewContent` here) and hands
it in as an `Element`. That is the seam that actually needs sharing — one design
language over two different content models, with neither importing the other.

`cargo run -p yggui --example conversation_gallery`, or
`libyggterm/scripts/gallery-shot.sh`, renders every component in both themes
against fixture data. **A design change to this surface is argued from that
screenshot**, not from source and not by rebuilding a host.

#### The rules that surface encodes

**A person asks; the document answers.** The user's turn is a bounded card
(≤78% of the column, ≤560px) set against the page and right-aligned, with the
bottom-right corner flattened so it points back at who wrote it. The assistant's
turn has no card at all — it IS the page, full column, one step up in size.
Two facing bubbles is a messenger idiom, and it makes a 900-line answer look
like a text message while making a two-line one look like small talk.

**One reading column, 720px.** A measure of ~78 characters at the prose size.
Prose that runs the width of a maximised window is not read, it is scanned. The
column is a token, not a literal at three call sites.

**Three faces, one meaning each — mono is the machine, sans is the person,
serif is the answer.** A face never changes for decoration. This is why the work
seam is monospace at 11px and the answer is serif at 15.5px: the reader can tell
what kind of thing they are looking at before they read a word of it.

**Metadata is 10px, wide-tracked, and dim.** Timestamps, counts, group labels
and controls all sit at the same quiet level, in tabular figures so a changing
number does not shift the row under it. Nothing at this level is ever
load-bearing enough to be always-on: turn actions are hover-revealed and always
in flow, so revealing one never moves the turn.

**Decoration must not move text.** The live rule down a streaming answer changes
its COLOUR only; it used to change the padding as well, so every answer slid
12px sideways the moment it finished — a jump under the reader's eye, on every
turn.

**No component invents a colour.** `ConversationTokens::from_palette` takes the
host's `ink`/`muted`/`accent` and derives every surface, hairline, tint and
semantic pair per theme. The previous surface spelled `rgba(255,255,255,0.72)`
at four call sites in one header, which is how a light-only design survives a
theme switch looking broken.

### Native page surfaces (the browser viewport)

A native page surface — a real web page rendered by the engine and layered over
the viewport (ychrome) — is **not** a widget inside the main canvas. It **is**
the canvas for as long as it is up. The rules above about a floating sheet with
a mild radius describe the TERMINAL viewport; a page is the other kind of main
artifact, and applying the terminal's frame to it is what produced the border
users kept reporting.

- **The molded frame is opt-out for web surfaces, and web surfaces take the
  opt-out.** No inset, no corner radius, no drop shadow around a page. The
  terminal's 4px frame padding and 10px radius exist to make a canvas of TEXT
  read as a sheet; around a page they are a visible strip of shell (near-white
  on the light chrome — the white border around a dark site) and the radius was
  never real anyway, because a native child webview paints its full container
  box and cannot be clipped by CSS `border-radius`.
- **The seam is the page's colour.** Every surface that can be seen behind or
  beside a page — the frame wrapper, the page placeholder, the native fill
  under a webview that has not painted its first frame — takes ONE colour,
  derived from the page's own declared `theme-color` (else its painted body
  background). They must never disagree; a disagreement between two of them is
  exactly what the eye reads as a border.
- **The seam fallback is dark, never white.** When a page has not said what
  colour it is yet, the seam is a dark neutral. A dark seam against a light page
  reads as a window edge; a white seam against a dark page reads as damage.
- **The page chrome keeps the app appearance.** Tab strip and omnibox carry text
  and stay on the light/dark chrome palette. Only the surfaces that BACK the
  page take the seam.
- **A transient reveal must not reflow the page.** Hover-revealed chrome
  translates the page (its far edge crops for the duration of the hover);
  only chrome held open by a standing gesture may resize it.
- **Element fullscreen owns the whole window.** Every chrome claim — including
  the auto-hide reveal sensors — stands down, and the page fills the window
  edge to edge until it leaves fullscreen, which restores the exact prior
  layout.

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
