# ALT+ KeyTips — the keyboard accelerator layer

**This is *the* ALT+ KeyTips spec.** Campaign memory: `campaign-alt-keytips-layer`.
Source of truth in code: `crates/yggterm-shell/src/keytip.rs` (the declaration
model + assignment resolver) and its render/chord integration in `shell.rs`.

An earlier throwaway trial run shipped a static registry of 15 shell-chrome
commands with inline badges (2.10.12). It proved the trigger, the reserved-letter
namespace, the keymap file, and the `command invoke` probe, but it could not grow
into the layer this project needs — for the reasons set out in §2 — so it is being
**replaced** by this spec, not extended. Where the sections below contrast with
"the trial run" they mean that shipped experiment; there is no versioned "v1/v2".

## 0. Thesis

**Every affordance carries a char. The ALT layer harvests them.**

Nothing yggterm can do with a mouse is unreachable from the keyboard, and an app
that yggterm has never heard of extends the layer without yggterm changing.

## 1. Model

Five nouns, and no others:

- **Declaration** — the thing a render site emits alongside an interactable:
  `(scope, key, title, hint, target)`. The char is attached to the element by the
  code that draws the element.
- **Scope** — a set of declarations shown together; one chord level. The root
  scope is what a clean ALT tap opens. Every container that can open (a menu, a
  panel, a modal, an app surface) is a scope.
- **Node** — one resolved entry in a scope: a letter plus a target.
- **Target** — `Run` (act and dismiss), `RunAndDescend` (act *and* open the
  container's scope), or `Group` (a synthesized disambiguation node, §6).
- **Assignment** — the pure function that turns a scope's ordered declarations
  into its final letters (§5).

The **KeyTip tree** is the per-frame composition of the open scopes. It is built
in Rust during render — never scraped from the DOM. The DOM is consulted only to
*measure where to paint a badge*, reusing the click-grid's measurement path
(`docs/yggui-click-grid.md`). A DOM-derived tree would violate both the SSOT rule
and the determinism rule; the char lives in the render tree, and the DOM carries
only a `data-keytip-node="<scope>/<key>"` anchor for measurement and audit.

## 2. Why v1 cannot grow (the ownership inversion)

In v1 the registry is a static table of **global commands** and each render site
asks it "what letter do I paint?" (`keytip_badge("sidebar.toggle")`, twelve
callsites). That model can express exactly one thing: a fixed, compile-time set of
app-wide commands. It structurally cannot express the three things the layer must
do:

| Need | Example | Why v1 fails |
|------|---------|--------------|
| **Instances** | "Launch CC *here*" | One command id, N live targets (cwd rows). |
| **Dynamic sets** | one entry per installed app, per theme, per live session | The set is not known at compile time. |
| **Foreign declarations** | an app's own commands | The shell has never heard of them. |

So ownership inverts. The **declaration** becomes the SSOT for *what exists*; the
registry keeps the SSOT for *default letters and user overrides* of the stable
chrome commands. `keytip_badge(id)` at every callsite disappears, replaced by one
declaration wrapper around the element.

## 3. Declaration API (shape, not final signature)

```rust
pub struct KeyTip {
    pub scope: ScopeId,          // Root, InsertMenu, Settings, Settings_Theme, App("ychrome"), …
    pub key: KeyTipKey,          // stable within the scope: "sidebar.toggle", "theme.dark",
                                 // "app.ychrome.new-here". Rides keymap.json and `command invoke`.
    pub title: String,           // shown in the legend, the editor, and used by assignment
    pub hint: Option<char>,      // the letter the declarer WANTS. Registry default, user
                                 // override, or an app manifest's `keytip`. May be denied.
    pub target: KeyTipTarget,
}

pub enum KeyTipTarget {
    Run(Action),
    RunAndDescend { action: Action, scope: ScopeId },
    Group(Vec<KeyTipKey>),       // synthesized by the resolver on collision — never declared
}
```

An interactable that deliberately has no accelerator declares
`keytip_exempt("<reason>")` instead. There is no third option: an affordance that
declares neither fails the audit (§12).

## 4. Scopes — "a command that opens a container descends into it"

v1's `opens_submenu` flag is this idea, used exactly once (the `+` menu).
Generalized, it is the whole navigation model, and it is what answers the three
questions this spec was written for:

| You want | Chord | Mechanism |
|----------|-------|-----------|
| Unhide the titlebar | *(automatic)* | The auto-hidden titlebar already pins in while the overlay is up (`titlebar_autohide_pinned`). New: `titlebar.autohide` command so the *setting* is bindable too. |
| Change the theme | `ALT, G, T, <letter>` | `G` opens Settings **and descends into its scope**; `T` is the theme control; each theme option is a leaf. |
| Launch CC here | `ALT, I, C` | `I` opens the New… scope, generated from the built-in agents plus `~/.yggterm/apps/*.json`. |
| The row's right-click menu | `ALT, E, <letter>` | `E` opens the **row menu** on the "here" row and descends into it. Its items are the `rowmenu` scope — one list (`row_menu_items`) that the mouse menu draws and the resolver declares, so the two can never disagree. |
| Switch to another live session | `ALT, J`, then ↑/↓/PgUp/PgDn, `Enter` | `J` enters **jump mode**, a navigation scope (§8). |

WHICH scope a command descends into is registry data (`CommandSpec::descends_into`),
not a `match` arm in the renderer.

**"Here" is a binding, not a vibe.** It resolves to the sidebar's focused row, else
the ACTIVE session's row — one function (`here_row`), read by every door: the
titlebar `+`, the New… scope, the row menu, the Ctrl+Shift accelerators and the
start page. It fixes **two** things, not one:

- the **cwd** the new session opens in, and
- the **sidebar position** it lands in — directly below the "here" row
  (`insert_after`), never at the top of Live Sessions.

That is why `ALT, I, T`, `Ctrl+Shift+T`, the `+` menu's "New Terminal" and the row
menu's "Open Terminal Here" all put the new row in the same place: they are one
launch path (`spawn_start_session_for_row`), not four.

## 5. Assignment (deterministic, pure, unit-testable)

For each scope, over its declarations **in render order** (which is stable):

1. a **pinned** assignment from `keymap.json` (§6), if the slot is still free;
2. a **user override** from `keymap.json`;
3. the declared **hint**, if free (and, at the root scope, not an Excel-reserved
   letter claimed by a shell command — §7);
4. the first free **letter of the title**, skipping non-alphanumerics;
5. the first free letter **a–z**;
6. **digits** 0–9;
7. a **two-letter tip** (Excel's `AL`, `AZ` behaviour) once single chars run out.

Ties break by declaration order. The same input list always produces the same
letters — this is a hard invariant, tested, not an aspiration.

## 6. Collisions, numbering, and pinning

**The shell never numbers.** Shell chrome letters are documented, fixed muscle
memory. An app requesting a letter a shell command already owns *in a shell scope*
loses outright and falls to step 4 of the ladder.

**App-vs-app collisions become a group.** When two or more app declarations
request the same letter `L` in one scope and no shell command owns `L`, `L`
becomes a **Group** node. Nobody gets the bare letter; the claimants are numbered
`1..n`:

```
ALT, I  →  [S] New session
           [T] New terminal
           [N] New …            ◀ contested: 2 apps
ALT, I, N  →  [1] New Ychrome here
              [2] New Cellulose here
```

An uncontested letter stays bare — a lone ychrome gets `ALT, I, N` with no number.
Installing cellulose then *lengthens* that chord to `ALT, I, N, 1`. This is
deliberate and is the reason the group design beats "first claimant keeps the bare
letter":

> **A chord never silently changes its target. It may only grow a disambiguation
> step.** With a group, a contested letter can never launch the wrong app — your
> finger lands on a picker, and continues one key further. With a bare-letter
> winner, installing an app could silently repoint a chord you had memorized.

**Numbers are pinned on first materialization**, so once you have learned
`I,N,1 = ychrome` it never moves:

```jsonc
// ~/.yggterm/keymap.json — the `pinned` section (full file shape in §11.5)
{
  "pinned": { "insert.menu/n/ychrome": 1,
              "insert.menu/n/cellulose": 2 }
}
```

A third app that also wants `N` takes the next free number (3). Uninstalling
ychrome leaves a hole at 1 rather than renumbering the survivors. The ALT+ editor
gets a **recompact numbers** action for anyone who wants the hole closed.

## 7. Reserved letters (v1 rule, unchanged)

Cellulose (a future libyggterm Excel clone) must be able to mimic Excel's
keybindings 100%, so shell chrome must never collide with Excel's top-level ribbon
letters. Shell KeyTips draw ONLY from `B,C,D,E,G,I,J,K,L,O,S,T,U,V,Z` + digits;
`F,H,N,P,M,A,R,W,X,Y,Q` (`EXCEL_RESERVED_LETTERS`) are held for app contributions.
`assert_shell_namespace_clean` fails the build if a shell default lands on a
reserved letter. One flat top-level namespace, no mode switch, no second leader.

## 8. Lists are navigated, not badged

Unbounded lists — cwd-tree rows, live-session rows, ychrome tabs — get **no
per-item badges**. They are reached with arrow-key navigation; the ALT layer badges
only named chrome, and its actions apply to the **focused** item:

```
ALT             → badges on chrome only
↓ ↓ ↓           → focus a cwd row
ALT, I, C       → Claude Code in THAT row's cwd
```

Rationale: a fifty-row cwd tree under two-letter tips is an unreadable overlay and
an unlearnable chord set. Row items declare `keytip_exempt("list-item")`.

Consequence, and it is a real piece of work: **the sidebar tree needs proper
keyboard focus** (a focus ring, arrow/Home/End navigation, and a focused-row
concept the shell can read). Today it has none.

### 8.1 Jump mode — the live list, navigated

`ALT, J` descends into `ScopeId::SessionJump`, a scope that holds **no
declarations at all** — the point of §8. The overlay stays up, and while it is the
open scope the ALT bridge forwards ↑/↓, PageUp/PageDown, Home/End and Enter into
the shell instead of merely swallowing them:

```
ALT, J          → ⌨ ALT › J › Live 3/12  yggterm shell   · ↑↓ PgUp/PgDn · Enter · Esc
↑ ↓ PgUp PgDn   → walk the Live Sessions list (wraps at the ends)
Enter           → open the highlighted session
Esc             → dismiss
```

The highlight IS the sidebar selection (`select_tree_row(Replace)`), so the row
lights up in the tree, "here" follows the cursor exactly as it does for
arrow-navigation, and there is no second highlight concept to keep in sync. It
never takes DOM focus: the bridge owns the keyboard while the overlay is up, so
stealing focus would only strand the terminal's cursor after Esc.

**Why not a bare `Alt+PgUp` / `Alt+PgDn` accelerator** (the obvious ask): §11.2
forbids it. Bare ALT+key is the Meta/ESC-prefix the PTY owns — `mc`, `weechat` and
`irssi` all read `Alt+PageUp` — so `assert_accels_pty_safe` would fail the build.
Jump mode gets the same ergonomics with zero new global chords, because every key
it uses is captured *inside* an overlay that is already up. `Ctrl+Alt+PgDn` /
`Ctrl+Alt+PgUp` (PTY-safe, §11.4) remain the instant one-key switch.

## 9. Badge rendering — blocks, not inline pills

v1 paints the letter as an inline `<span>` *inside* the button. That is wrong twice
over: it does not read as a keytip, and because it sits in layout flow, tapping ALT
**reflows the entire titlebar** (buttons shift as the badges appear). Both die in
v2.

A KeyTip badge is **its own little block** — literally a context menu containing
nothing but one letter. Per DESIGN.md ▸ Control language ▸ KeyTip badges, it
inherits the context-menu surface treatment (modest radius, theme-aware surface,
subtle shadow, strong label clarity) at badge scale.

- Painted in an **absolutely-positioned overlay layer above the chrome**. Zero
  layout impact: nothing under the overlay moves when ALT is tapped.
- **Anchored** to its element's measured box, offset to overlap the element's
  lower-leading corner, nudged inward at viewport edges so a badge is never
  clipped.
- A multi-key tip (`AL`, or a group's `N`) renders **one block, both glyphs** — not
  two blocks.
- **No scrim.** Excel does not dim the app to show keytips and neither do we.
- The chord breadcrumb (v1) stays, and gains the resolved "here" cwd when the
  current scope is target-sensitive.

## 10. App contribution

Two feeds, one tree:

**Static — the manifest.** `~/.yggterm/apps/<name>.json` gains a `keytip` field on
the app and on each `AppVerb`. This is what the `+` menu needs, because it lists
apps that are **not running**. No protocol, no process: the shell reads the disk
registry it already scans.

**Dynamic — OSC 7717, while focused.** A running app declares its own commands over
the existing surface-contribution channel (the same one that already carries
`{pane, id}` for sidebar panes). A focused Cellulose claims the Excel-reserved
letters at the top level and is 100% Excel-faithful; an unfocused app contributes
nothing.

**Central assignment, local painting.** The shell resolves the whole tree —
including collisions, numbering, and pins — and hands each running app the *final*
letters for its own nodes. The app paints its own badges inside its own webview.
This is not a preference: yggterm **cannot** overlay a native child web surface
(the same blindness that makes `app screenshot` unable to see one — see
`finding-native-web-surface-cannot-resize-and-screenshot-lied`). libyggterm ships
the badge painter so every app gets the identical look for free.

Held-ALT **direct chords** (Excel's `ALT+H` with the key held) fire on a focused
non-terminal surface. On a focused **terminal**, held ALT+key always passes through
to the PTY as Meta/ESC-prefix — readline, emacs, and helix must never break. That
rule is absolute and predates this spec.

## 11. The direct-accelerator layer (second class, deliberately)

`Ctrl+Shift+T` = new terminal here. Users arrive with that muscle memory from
gnome-terminal, konsole, and Windows Terminal, and no amount of Excel-style elegance
substitutes for it. So the layer exists — but it is **explicitly second class** and
does not participate in the chord philosophy at all.

| | **KeyTips** (ALT layer) | **Accelerators** (direct chords) |
|---|---|---|
| Shape | hierarchical: leader, level, level… | flat: one chord, one action |
| Coverage | **exhaustive** — audit must read zero (§12) | **sparse** — only what is genuinely common |
| Discovery | the overlay shows you everything | the modal and menu accelerator hints |
| Purpose | reach anything without a mouse | do the frequent thing without thinking |

**One registry, two bindings.** A command declares both, and neither layer is a
second encoding of the other — the command is the SSOT and these are two views of
it, exactly as Excel has both `ALT,H,…` and `Ctrl+C`:

```rust
pub struct KeyTip {
    …
    pub hint:  Option<char>,   // the ALT layer's letter
    pub accel: Option<Chord>,  // the direct layer's chord. Sparse: most are None.
}
```

"Here" resolves identically in both layers (focused sidebar row, else the active
session's cwd), so `Ctrl+Shift+T` and `ALT, I, T` land in the same place. One
resolution rule, two doors.

### 11.1 This layer already exists, ad hoc — the registry absorbs it

`Ctrl+Shift+C` / `Ctrl+Shift+V` (copy/paste) and `Ctrl+Alt+PgUp` / `Ctrl+Alt+PgDn`
(session nav) are **already hardcoded** in `shell.rs`. That is a second encoding of
things the registry should own. Phase 1 migrates them into it; no chord may be
hardcoded at a callsite afterwards.

### 11.2 PTY-safety is a build-time rule, not a convention

A bare `Ctrl+<letter>` belongs to the PTY, permanently. `Ctrl+T` is readline's
transpose-chars; `Ctrl+B` is backward-char. A shell that steals them breaks
readline, emacs, and helix — which §15 already forbids.

This is why every terminal emulator puts its own chrome on `Ctrl+Shift`: in the
legacy encoding a terminal cannot distinguish `Ctrl+Shift+T` from `Ctrl+T` at all,
so `Ctrl+Shift` is *free by construction*.

> **A shell accelerator must be PTY-safe: `Ctrl+Shift+…`, `Ctrl+Alt+…`, `Super+…`,
> or a function key. Bare `Ctrl+<letter>` is forbidden.**

`assert_accels_pty_safe` fails the build on a violation — the exact counterpart of
`assert_shell_namespace_clean`, and it protects the PTY byte contract forever
rather than by vigilance. (A focused non-terminal surface has no PTY to protect, so
an app may claim whatever it likes *within its own surface* — §11.3.)

### 11.3 Precedence: the focused surface wins

`Ctrl+Shift+T` is "new terminal here" to yggterm and "reopen closed tab" to Chrome.
ychrome will want it. That collision is real and the rule is the same one that makes
Cellulose 100% Excel-faithful:

> **A focused app surface wins every accelerator it claims.** The shell's identical
> accelerator is shadowed while that surface has focus.

Against that, the shell keeps a **short, non-negotiable escape set** no app may
shadow — the keys that always get you back out: the clean ALT tap, `Ctrl+Alt+PgUp` /
`Ctrl+Alt+PgDn` (session nav), and the sidebar toggle. This is the reserved-letters
rule inverted: there the apps hold the reserved set, here the shell holds a tiny one.

Two **apps** can never collide on an accelerator, because only one is focused at a
time. So there is **no numbering in this layer** — a chord cannot grow a
disambiguation step the way a KeyTip can. A shell-scope accelerator claimed by two
app *manifests* is resolved by app-id order, the loser simply gets none, and the
Keymaps modal flags the conflict by name so it can be rebound rather than silently
lost.

### 11.4 Shipping defaults (sparse on purpose)

| Chord | Command |
|---|---|
| `Ctrl+Shift+T` | New terminal here |
| `Ctrl+Shift+N` | New session here |
| `Ctrl+Shift+W` | Close the active session |
| `Ctrl+Shift+C` / `Ctrl+Shift+V` | Copy / paste *(migrated from hardcode)* |
| `Ctrl+Shift+F` | Focus search |
| `Ctrl+Shift+B` | Toggle sidebar |
| `Ctrl+Alt+PgDn` / `Ctrl+Alt+PgUp` | Next / previous live session *(migrated)* |
| `F11` | Fullscreen |

Every one is PTY-safe. The list stays short by policy: a command earns an
accelerator by being used constantly, not by existing. Everything else is reachable
through the ALT layer, which is what the ALT layer is *for*.

### 11.5 Config and modal — side by side

Both layers live in one file and one editor, which is what makes them feel like one
system rather than two:

```jsonc
// ~/.yggterm/keymap.json — version 2
{
  "version": 2,
  "keytips":      { "sidebar.toggle": "b" },            // ALT letters (v1's "bindings",
                                                        // still read as a legacy alias)
  "pinned":       { "insert.menu/n/ychrome": 1 },       // materialized group numbers (§6)
  "accelerators": { "insert.terminal": "Ctrl+Shift+T" } // direct chords
}
```

Settings ▸ **Keymaps** shows one row per command with **two columns** — its ALT
chord and its accelerator — each rebindable, each validated in place: a KeyTip
letter is rejected for reserved-letter or same-level clashes (v1 behaviour), an
accelerator is rejected for not being PTY-safe or for duplicating another command's
chord. Accelerators also surface as right-aligned hints in the menus that contain
them, which is where people actually learn them.

## 12. Enforcement — the no-orphan-affordance audit

"Wire the whole UI" is a vibe until it is a number that must be zero.

`server app keytips audit` walks the visible DOM for interactables
(`button`, `[role=button]`, `a[href]`, `input`, `select`, and click-bound rows) and
reports every one carrying neither `data-keytip-node` nor `data-keytip-exempt`.
Violations are listed with their scope and label. **The audit must read zero**, and
it is the definition of done for Phase 1 — the same species of enforcement as the
existing `assert_shell_namespace_clean` test, which is what has kept the namespace
rule honest.

## 13. Invariants (each one testable)

1. Assignment is a pure function of `(ordered declarations, keymap, pins)`.
2. A chord never silently changes its target; it may only grow a disambiguation step.
3. A pinned number never moves while its app stays installed.
4. No shell top-level keytip lands on an Excel-reserved letter.
5. The overlay never changes layout — zero reflow on ALT.
6. Every visible interactable is declared or explicitly exempt (audit = 0).
7. Held ALT+key in a focused terminal always reaches the PTY.
8. No shell accelerator is a bare `Ctrl+<letter>` — the PTY keeps them (§11.2).
9. No chord is hardcoded at a callsite; the registry owns every binding in both layers.
10. "Here" resolves identically for a KeyTip and for an accelerator.
11. **A clean ALT tap opens the overlay regardless of which surface has focus.** A
    focused terminal must NOT swallow the tap — see §13.1.

## 13.1 Known defect at handoff — a focused terminal eats the ALT tap

Reported by the user 2026-07-11; open. **Symptom:** with a terminal focused, tapping
ALT does not open the overlay; the tap is consumed by the terminal. The overlay
works when focus is on shell chrome.

**Root-cause hypothesis (well-founded, not yet falsified live):** both clean-tap
drivers — `DesktopWindowEvent::ModifiersChanged` (the primary GTK path) and
`KeyboardInput` — are **tao window-level** events (`shell.rs:45766` and `:45678`).
When the xterm.js webview holds keyboard focus, WebKitGTK consumes the ALT key
events inside the webview and the window-level tao handlers never fire (or fire
inconsistently). This is precisely the risk the original spec flagged: *webviews
consume keys, so tap detection must sit at the GTK/window level.* The v1 Xvfb proof
missed it because the test's focus sat on the shell root, not a live xterm textarea
with a running PTY.

**Fix direction (Phase 1):** move tap detection BELOW the webview. Either a
GTK-level key event controller / capture-phase filter on the `GtkWindow` that sees
ALT before the webview does, or an xterm.js-side forwarder (the JS bridge already
used elsewhere) that reports a bare-ALT keyup to the shell. Whichever is chosen,
invariant 7 must survive it: a **held** ALT+key in a terminal still reaches the PTY;
only the clean, keyless tap is intercepted. Falsify the hypothesis first with a
key-event trace on a live focused terminal before building the fix (CLAUDE.md
investigate discipline: no fix without root cause).

## 14. Phasing

**Phase 1 — 2.11.x, shell-wide.** Floating block badges (overlay layer, zero
reflow); the declaration API replacing the twelve `keytip_badge` callsites; a scope
per panel, modal, and menu; sidebar keyboard focus; the "here" binding; manifest
keytips in the `+` menu with collisions, numbering, and pinning (all of which work
off `~/.yggterm/apps/*.json` with no app running); the accelerator layer (§11) with
the hardcoded chords migrated in and `assert_accels_pty_safe` guarding it; the
two-column Keymaps modal; the orphan audit driven to zero.

**Phase 2 — 3.0.0, with libyggterm.** Running-app OSC contribution; app-side badge
painting shipped in libyggterm; held-ALT direct chords on a focused surface;
focused-surface accelerator precedence (§11.3).

## 15. Non-goals

- **No ribbon UI clone.** Legal trade-dress line. Cellulose's ribbon is a sidebar
  contribution (`docs/web-surfaces.md`), not a horizontal ribbon.
- **No rebinding of what terminal apps receive.** The PTY byte contract is
  untouched; the layer intercepts the clean tap and (Phase 2) GUI-surface chords
  only.
- **No hardcoded per-app keytips in yggterm-shell.** Contribution protocol only —
  the same one-rule as `project-libyggterm-platform-vision`.
- **No badges on unbounded lists** (§8).
