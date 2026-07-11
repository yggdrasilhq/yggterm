# ALT+ KeyTips — the keyboard accelerator layer

SHIPPED 2.10.10 (shell chrome only). Spec: `campaign-alt-keytips-layer` memory.
Source of truth in code: `crates/yggterm-shell/src/command_registry.rs`.

## What the user sees

Tap **ALT** — a clean press and release with no other key — and hint letters pop
onto every visible piece of chrome (the auto-hidden titlebar pops in so its `+`
and utility buttons show their hints). Press a highlighted letter to run that
command or descend into a submenu, Excel-style. A breadcrumb at the top shows the
chord typed so far. **Esc**, a click, or moving focus to another window exits.

Default shell KeyTips (the Excel-familiar preset):

| Key | Command | id |
|-----|---------|----|
| B | Toggle sidebar | `sidebar.toggle` |
| V | Web view | `view.web` |
| T | Terminal view | `view.terminal` |
| C | Connect SSH | `connect.toggle` |
| L | Notifications | `notifications.toggle` |
| G | Settings | `settings.toggle` |
| D | Session metadata | `metadata.toggle` |
| U | Toggle fullscreen | `window.fullscreen` |
| O | Always on top | `window.always-on-top` |
| K | Edit ALT+ keys | `keymap.editor` |
| I | New… menu | `insert.menu` |
| I S | New session | `insert.session` |
| I T | New terminal | `insert.terminal` |
| Ctrl+Alt+PgDn | Next live session | `session.next` |
| Ctrl+Alt+PgUp | Previous live session | `session.prev` |

## The reserved-letters namespace (spec decision a)

Cellulose (a future libyggterm Excel clone) must be able to mimic Excel's
keybindings 100%. So yggterm's shell chrome must never collide with Excel's
top-level ribbon letters. Shell KeyTips draw ONLY from
`B,C,D,E,G,I,J,K,L,O,S,T,U,V,Z` + digits; the Excel letters
`F,H,N,P,M,A,R,W,X,Y,Q` (`EXCEL_RESERVED_LETTERS`) are held in reserve for app
contributions. `assert_shell_namespace_clean` (a unit test) fails the build if a
shell default keytip lands on a reserved letter. This is why the pre-spec draft's
`n`(notifications)→`l`, `m`(metadata)→`d`, `f`(fullscreen)→`u`, `a`(always-on-top)→`o`.

## The command registry is the SSOT

`command_registry::SHELL_COMMANDS` is the one table mapping a command to its id,
title, default KeyTip letter, and chord parent. Everything else is a VIEW:

- **KeyTip badges** — `keytip_badge(&snapshot, "<id>")` returns the letter to
  paint (from the in-force keymap), or `None` when the overlay is not at that
  command's chord level. No callsite hardcodes a letter.
- **Resolver** — `Keymap::resolve(sequence)` walks the registry: exact chord →
  `Command`, valid prefix → `Pending`, otherwise `Invalid`.
- **Config file** — `~/.yggterm/keymap.json` (`{ "version": 1, "bindings": {
  "<id>": "<letter>" } }`) is a sparse override layer over the preset, loaded at
  startup and written through by the settings modal. A corrupt file falls back to
  the preset; it never bricks the accelerators.
- **Settings modal** — Settings ▸ "ALT+ Keys" explores every command, rebinds any
  leaf letter (rejecting reserved letters and same-level clashes with an inline
  error), and resets to the preset.
- **Agent probe** — `server app command invoke <id>` fires a command by id
  (the keyboard analogue of the click grid); `server app command list` enumerates
  ids, titles, and chords.

## Clean-tap trigger (spec decision c)

The overlay opens on a clean ALT **release**, not the press. The window-level tao
keyboard handler arms `alt_tap_candidate` on an ALT press and clears it on the
first other key; the release fires the overlay only if the candidate survived. A
held **ALT+<key>** chord in a focused terminal therefore never trips the overlay —
the first non-ALT key cancels candidacy and the Meta prefix reaches the PTY
untouched (readline/emacs/helix keep working). On activation, focus moves to the
shell root so chord letters land on the root `onkeydown` (which can
`prevent_default`) rather than a terminal's helper textarea.

## Deliberately deferred to 3.0.0 (with app contribution)

- An app claiming Excel's reserved letters via an OSC 7717 KeyTip contribution.
- Held-ALT+<key> DIRECT chords on a focused native web surface (a browser/spreadsheet
  answering `ALT+H` itself). In 2.x, yggterm's own chrome is the only registry
  consumer — extraction-not-construction.
