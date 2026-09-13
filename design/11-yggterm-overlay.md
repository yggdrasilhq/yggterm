<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Project overlay: yggterm

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

### Row sets (collapsible arrangements of live rows)

A **row set** is a set of live-session rows collected under one of them and
collapsible as a unit. The head is an ordinary session row; the members hide
under it until it is expanded.

⛔ **THE NOUN IS `row set`, and the two words next to it are already taken.**
Terminal splits are **groups**. The cwd tree has **folders**. A third meaning of
either word makes every future bug report ambiguous, which the owner flagged
before any of this was built. `section` was the first suggestion and it is also
taken — `AppPaneWidget::Section` is a contributed-pane widget and "section
cards" are a form pattern in this document. Code says `RowSet` / `row_set_id`;
the CLI verb is `row-set`; the sidebar shows no noun at all, only a disclosure
control on the head.

**A row set means NOTHING but arrangement.** No ownership, no lifecycle, no
supervision, no effect on what any session is or does. Membership is arbitrary
and the user may put any rows together — the outline numbers (`6`, `6.1`, `6.2`)
are the *default* arrangement, never a restriction on it. ⇒ Nothing else in the
app may read set membership to decide behaviour, and nothing else in the app may
rewrite it as a side effect.

**Sets NEST, arbitrarily deep**, because orchestration is recursive and seats go
`N.x.y.z…`. The model is a containment relation — a set holds rows *and other
sets* — never a boolean or a single non-recursive parent field.

**THE DEFAULT ARRANGEMENT IS THE OUTLINE ITSELF, and it is not decoration.** The
owner's rule, and the standing principle he settled it as: *group `N.x` rows
under `N.0` as the header, and `N.x.y` rows under their `N.x` header where
applicable* — this replaces ad-hoc row tidying as the way a fleet's structure is
made legible. So a head is found by looking, in order, for the **`.0` seat of the
row's own level** (`6.1` and `6.2` sit under `6.0`; `6.1.1` sits under `6.1.0`
when a sub-orchestrator holds that seat), and failing that the **bare parent
seat** (`6.1.1` under `6.1`). One rule at every depth, which is what makes the
nesting recursive rather than two hard-coded levels.

- ⛔ **Derived every frame from `outline_prefix`, never stored and never parsed
  out of a title.** The seats already carry this; a written-down copy would go
  stale the moment a delegate is reseated or reaped, and a seat read back out of
  prose is destroyed by the next re-title. `row_set_outline` is the ONE place a
  seat may produce a containment edge.
- ⛔ **A head is a ROW, not a heading.** If nobody holds `6.0`, then `6.1` and
  `6.2` stay top level rather than collecting under an invented placeholder: a
  synthetic head could not be clicked, closed or driven.
- **Membership stays arbitrary.** This is the *default*, so an explicit
  arrangement — the verb and the drag — overrides it per row and never has to
  fight it.

- **Each set owns its own collapsed flag**, and collapsing an outer set hides
  everything beneath it without touching the inner flags. Re-expanding restores
  each inner set exactly as it was. ⛔ Flattening the inner sets open on expand
  is the failure users notice, and it is the one that gets skipped.
**⛔ THE TRAFFIC LIGHTS ARE A COLUMN OF THEIR OWN, FAR LEFT, IN A FIXED AREA.**
Owner-directed, and it is a layout MODEL rather than a rule about padding. A
live row has two zones, and they are SIBLINGS in the markup:

| zone | holds | behaviour |
|---|---|---|
| **gutter** | the status dot, nothing else | fixed width, flush to the row's own left edge, identical on every row at every depth — including rows in no set |
| **content** | icon, title, trailing controls | begins after the gutter, and is the only thing nesting moves |

Structural separation is the point: a shared leading pad that merely happens to
be equal on every row is one zone pretending to be two, and the dots move again
the moment anything needs to sit before the icon. That is not hypothetical — the
first build of row sets put the disclosure control at the head's leading edge,
which pushed that row's dot, icon and label right, so **a header drew further
right than its own members** and dots stood in two columns.

⇒ **The disclosure control lives at the TRAILING edge beside the ✕, and shares
its hover reveal.** That is where the reclaimed width comes from: the gutter is
the row's own horizontal padding rather than a `base + step` leading run, so
every live row's title starts further left than before. Measured on the live
host, light theme, 2026-08-13: the dot column moved from x=40 to x=25 and became
a single value for every row; a top-level row's first text pixel moved from
x=82 to x=65, and a nested row's to x=77 — still left of where the flat list
started. ⭐ With a real gutter the dots form one unbroken vertical line down the
sidebar, so **a kink at any row means the zones were never separated.**

- **Indentation is budgeted.** The sidebar runs out of horizontal room before
  the outline runs out of levels. Indent the first two levels; past that, hold
  the indent and let the head's own number carry the depth — a title clipped to
  nothing is worse than a column that stops stepping. ⚠ The rows past the budget
  are still fully NESTED — drawn under their head and hidden with it; only the
  left edge stops moving.

#### Row sets and splits are orthogonal, and neither may relocate the other

A **split is a VIEW** — what is painted in the viewport. A **row set is an
ARRANGEMENT** — where a row sits in the sidebar. They answer different questions
and may overlap freely, including the case that prompted this rule: splitting a
set's head with a member of a different set.

⇒ **A split never moves a row and never changes its membership.** Participants
are shown in place, each under its own set, with an affordance saying the row is
currently sharing a viewport. Not by pulling rows together, not by lifting a row
out of its set. The rationale is the one above: a structure that means nothing
must not be silently rewritten by an action about something else, and a row that
moves when you split it is a row the user can no longer find.

The edge cases, each with a rule, because "undefined" is how two structures
start disagreeing:

- **Collapsing a set whose head is live in a split** — the split keeps painting.
  Collapsing hides *rows*, not viewports. A hidden row is still a live session.
- **Removing the head of a non-empty set** — the set DISSOLVES and its members
  are promoted to where the head sat, in order. Not refused (the user asked to
  close a session, not to be told about bookkeeping) and not silently deleting
  the members with it.
- **Dropping a row into a collapsed set** — the set spring-loads open under the
  pointer, exactly as a shut folder does in the cwd tree ("Drag and drop", the
  `ROW_DRAG_SPRING_MS` rule). One gesture, one grammar.
- **A member dragged out** — it leaves the set and lands where it was dropped.
  Leaving a set is an ordinary reorder.

#### An agent arranges rows as easily as a hand does

Both halves exist or neither is real: the user drags, and a delegate calls a
verb. ⚠ **The drag half does not exist yet** — measured 2026-08-13:
`row_drop_placement_for_offset` returns `Into` only when the target row
`is_group`, and a live-session row is not a group, so today a row dropped on
another row can only land Before or After it. Dragging one live row onto another
REORDERS; it forms nothing. Building row sets therefore means giving session
rows an inside band, which is new behaviour rather than an existing gesture to
reuse.

### Status indicator vocabulary (traffic signal + blue/orange)

One coherent light vocabulary for session state, used by Live Sessions today and Automated Sessions later. The status dot in the live-session rail is the canonical instance; any future surface that signals session state reuses these meanings and colors rather than inventing new ones.

- `GREEN` (`#22c55e`): keep-alive — the session survives the GUI (durable runtime).
- `BLUE` (`#3b82f6`): live but transient — the session lives only while the GUI does.
- `BLINKING` (the `yggterm-status-dot-blink` pulse): the agent is working right now. Blink is an orthogonal modifier — a green or blue dot blinks while its session works and returns to steady when idle.
- `ORANGE/AMBER` (`#f59e0b`): attention states — recovery in progress, degraded runtime, held ghost frame, or pending user decision. **Wired 2026-08-13 for its first case: a session that has been WRITTEN TO and has said nothing back** (`input_unanswered_ms` past `INPUT_UNANSWERED_WEDGE_SUSPECT_MS`). Amber **replaces** the durability colour for as long as the condition holds, because a row that may not be listening or whose visible frame is not yet current is a more urgent fact about it than whether it would survive the GUI; the underlying green/blue is unchanged and returns when the attention condition clears. A short healthy in-flight input queue is observability, not attention, and must not flash amber on each typable keystroke. A held ghost frame may remain amber while typing is live: that light identifies display freshness, not an input failure. Remaining attention cases (save conflict, recovery in progress) join this same slot.
  - ⛔ **Steady, never blinking.** Blink means *working*, while attention includes a row that has gone deaf or is holding a ghost frame. An amber blink would confuse display/recovery attention with active work.
  - ⚠ **It marks a SUSPICION, not a verdict**, and the tooltip must say so: a child may legitimately consume input in silence (echo off, a password prompt). `server app terminal input-check` is what settles it, by marker and echo.
- `RED`: reserved for dead/error (runtime lost, unrecoverable). Same rule: reserved, not yet wired.

Rules: color encodes durability class, blink encodes activity, and reserved colors are introduced only with a spec update here. Automated Sessions (experimental/automations) must adopt this vocabulary unchanged so a user reads one signal system across the whole sidebar.

**Durability, not "session-ness".** The vocabulary describes what happens to the *thing* when the app goes away, so it extends to any surface with that question — including a contributed app's rows and a document's save state. For an editor's file list:

- `GREEN` = saved: the content IS the file on disk, and outlives the app.
- `BLUE` = unsaved: the content lives only inside the app's own store (yedit keeps a full-content row in its sqlite drafts table). Exactly the same meaning as a session that lives only while the GUI does.
- No dot = nothing to signal yet: a brand-new note never typed into is neither in the store nor on disk. The SLOT is still laid out.

A contributed row names the CLASS (`"durable"` / `"transient"` on `list-row`'s `status`) and yggterm paints it from `live_session_status_dot_style`, the same function the Live Sessions rows use — an app never picks a colour. A token yggterm does not paint (including the reserved amber/red above) renders the empty slot rather than a guess. A save conflict is the natural future `ORANGE` ("pending user decision"), and wiring it means editing this section first.

**Status never lives in the title.** Apps that had no status slot were prefixing the row title with a literal `●`, which paints in the row's text colour (near-black in the light theme, so "the traffic dot is black") and shifts the name sideways relative to rows without it. If a row needs to signal something, it needs a slot — not a glyph in its name. "This row is the one in use" is `selected`, not a dot.

**One clock for every blink.** All blinking indicators — live-session dots, machine dots, group dots, web-tab loading lights — flip on the *same* tick. The app owns exactly ONE clock: a timer parks the class `yggterm-blink-off` on the document element for half of each 2400 ms cycle, and one stylesheet rule stamps every marked indicator to `opacity: 0 !important` while it is there. An indicator joins by carrying the marker `--yggterm-status-dot-blink` in its inline style, never by declaring an animation of its own. This is both a design rule (a sidebar of dots pulsing in unison reads as one system; dots blinking at random phases read as noise) and a hard performance constraint: on a software-GL host every opacity flip costs a full-window CPU blit, so N independently-phased indicators cost N times the frames of one. Any new indicator MUST join the shared clock.

⛔ **The clock may not be a CSS animation of a custom property.** That was the shape from 2026-07-21 to 2026-08-13 and it did not blink at all: WebKitGTK advances such an animation in the style system — `getComputedStyle` returns a perfect square wave — but never marks the `var()` consumers dirty for paint, so every working dot froze at whatever phase its last unrelated re-render sampled, usually the invisible one. Registering the property with `@property` does not change it. Whatever drives the phase must be a change the paint path cannot ignore. ⭐ **The period is 2400 ms and that is a verdict, not a default.** 1100 ms was chosen while the blink did not paint, so nobody had watched it; the first sight of it running drew *"they blink waaay too fast"*. A hard square wave that takes the dot fully dark reads as a strobe near 0.9 Hz and as a heartbeat near 0.4 Hz. The shape stays settled — full off, full on, no fade — and only the period moved. ⚠ And a blink is proved with a burst of screenshots, never with `getComputedStyle`: reading the computed value forces the very style recalculation whose absence is the bug, so that probe can only ever answer yes.

### Agent CLI brand colours

Every registered agent CLI owns a colour, and a solid control belonging to a
session paints in its CLI's colour with a white label. The canonical instance is
the start page's *Open this … Session* button; the "New … Session" split-button
family uses the same values.

**The colour is a CLI's identity, so the CLI declares it once.** The values live
on `AgentCliDescriptor::brand_color` in
`crates/yggterm-core/src/agent_cli.rs`, and every surface reads them from there —
`session_kind_primary_bg` is the shell's one accessor. This table is the prose
record of *why* each value is what it is; the registry is what the code reads.
A tenth CLI adds a row to the registry and a line here, and needs no new branch
anywhere.

| CLI | Colour | Contrast vs white | Source of the hue |
|---|---|---|---|
| Codex | `#0f766e` | 5.47:1 | OpenAI's teal, darkened to clear AA |
| Codex-LiteLLM | `#0369a1` | 5.93:1 | a cool sibling of Codex — same family, separable at a glance |
| Claude Code | `#c2410c` | 5.18:1 | Claude's clay, at the darkest step that still reads as the brand |
| Pi | `#be185d` | 6.04:1 | nearest available |
| OpenCode | `#4338ca` | 7.90:1 | nearest available |
| Qwen Code | `#6d28d9` | 7.10:1 | Qwen's violet |
| Kimi | `#1e40af` | 8.72:1 | Moonshot's deep blue |
| Muse Code | `#86198f` | 8.24:1 | nearest available |
| Antigravity | `#1557b0` | 6.95:1 | Google's blue, darkened one step |

Three rules, and the first is not negotiable:

- **A brand colour clears WCAG AA (≥4.5:1) against white**, because it always
  carries a white label at 12px — a size that is *not* WCAG "large text", so the
  3:1 large-text allowance does not apply. `#d97706`, the amber this vocabulary
  replaced, sat at **3.19:1** and failed AA for the label it carried. "Nearest
  available brand colour" is licensed for the HUE; it is never licensed for the
  contrast. The test `the_brand_colours_clear_wcag_aa_against_white` enforces
  this and a new entry must pass it.
- **No two CLIs share a colour.** A shared colour identifies neither, which is
  the same defect as having no colour — enforced by
  `no_two_clis_share_a_brand_colour`.
- **A kind with no CLI keeps the theme accent on a panel background** (a plain
  shell, a terminal recipe, a document). The solid brand fill is what says "this
  is an agent session", so it must not be spent on things that are not.

These are deliberately dark, mid-saturation values rather than the brighter
marketing hues. They sit on `panel_alt` cards in both themes and must stay
legible against each; the accessibility floor is what keeps them theme-agnostic,
since a value that clears AA against white is dark enough to hold its own shape
on a light card and bright enough not to vanish into a dark one.

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

**Startpage is also the RECOVERY surface, and that governs the list.** It is
what a user falls back to when the sidebar has failed them, so the recent list
is an instrument for *finding a session again*, not a decorative history:

- **Most recently used leads, and the page SAYS SO.** The rule is printed beside
  the "Recent work" heading. An ordering the reader cannot name is one they
  cannot trust — the list was once ordered alphabetically by session uuid, which
  looks like no order at all, and the report that found it could only call it
  "weird".
- **The list is searchable, over contents as well as titles.** After a failed
  restore a user knows what a session was *doing*, not what it was called, so
  the query runs against each row's generated summary, title, folder, host and
  session id — the same `row_search_blob` the cwd tree's search uses. **One
  predicate serves both surfaces**; a query that matches in the sidebar and not
  here (or the reverse) is a bug, not a feature of scope.
- **Only sessions.** libyggterm app rows (ychrome, yedit) are excluded: an app
  row is not something anyone resumes, so on the one surface for picking a
  session it is noise. The discriminator is the row's persisted `Source` stamp,
  never its title.
- **The open verb names its CLI and wears its colour** — *Open this Codex
  Session*, *Open this Claude Code Session* — per § *Agent CLI brand colours*.
  A bare "Open" is reserved for rows that genuinely have no CLI.
- **A search that finds nothing says so in its own words.** "No saved sessions
  yet" is a sentence about the store; shown to someone whose query simply missed,
  on the surface they reached *because* work went missing, it reads as the page
  having lost their sessions.
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

### Browser tab behavior (native page surfaces)

Tabs are a vocabulary users already own from every other browser. Match it
exactly; do not invent.

**Where a new tab goes** is one rule with one owner, not a decision each opening
path makes for itself:

- a tab opened BY a tab — a menu's "new tab below", a middle- or ctrl-clicked
  link, a duplicate, a `window.open` — lands **immediately below its opener**
- successive tabs from the same opener **cascade**: the second goes after the
  first, not between the opener and it. The group is the whole subtree, so a
  grandchild is stepped over too
- a child is born in its **opener's folder**. Opening a link from a filed tab
  must not scatter the folder
- a tab nothing opened — a header "+" — appends. A folder row's "+" appends to
  that folder
- the relationship **expires** when the user picks a row by hand: they have
  re-declared where they are, and the next link belongs beside that tab. The
  front moving because a verb the user ran finished its work is not that

**The middle button does the same action, somewhere else.** It is not a separate
verb and it does not get a separate menu entry — it is the modifier the user
already owns from every browser, and it applies to the nav controls beside the
omnibox exactly as it applies to a link:

- **back** and **forward** open the entry they would have stepped to. In a new
  tab, so the page you are on stays where it is — which is the whole reason to
  reach for the middle button rather than the left one
- **reload** opens this page again, in a new tab. **History** opens the history
  page in one
- every one of them lands **below the active tab** and joins its group, so a
  second middle-click cascades after the first. Same destination, same owner as
  a middle-clicked link — a nav control does not get a placement rule of its own
- and it is a **background** open, per the rule above: the user asked to have
  the page, not to go to it

A control that has an in-place action and no middle-click is an unfinished
control, not a deliberate one.

**Where the keyboard goes** on a new tab is the other half, and it is decided by
the tab's DESTINATION:

- a **blank** tab opens typing-ready: the address bar is in edit mode and holds
  the keyboard, from every path that opens one — a "+", a row menu, a folder
  row, a keyboard route. "Only some of the buttons do it" is the bug
- a tab that already has a **URL on its way** never takes the keyboard. The page
  is loading and the user is reading the tab they are on; moving their caret out
  of it is theft, and it is what makes middle-click unusable
- neither does a **background** open, for the same reason

**Both tab homes are one feature.** The vertical rail and the classic strip show
the same tabs; a verb that exists in one and not the other is a bug in the one
that lacks it, not a difference in kind.

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

### Window corners

The app draws its own chrome, so its window corners are its own problem. They
are part of the product's shape, not a platform default it inherits.

- **Unmaximized windows are rounded at 10px** (`UNMAXIMIZED_SHELL_RADIUS_PX`) on
  every platform that can express it.
- **Maximized windows square off.** A maximized window is edge-to-edge with the
  screen; a rounded corner there reveals nothing behind it and reads as damage,
  not as shape. This is the contract, not a regression.
- **The radius is applied in the page, not on the frame.** The shell root carries
  both `border-radius` and `clip-path: inset(0 round Npx)`. The clip-path is the
  load-bearing half: GTK's own `border-radius` rounds a widget's background and
  does **not** clip a child WebView, so without the clip the web content paints
  square straight over a rounded frame.

**Per-platform, how the corner is actually cut:**

| Platform | Mechanism | State |
|---|---|---|
| Linux / Wayland | Transparent (RGBA) surface + the page's `clip-path`. There is no Shape extension, so this is the only path. | Works on every compositor. |
| Linux / X11 | `shape_combine_region` on the GDK window (the Shape extension), opaque surface. KDE/X11 takes the transparent path instead, for its compositor's blur. | Works. |
| Windows 11 | Transparent surface + the page's `clip-path`, **and** `DwmSetWindowAttribute(DWMWA_WINDOW_CORNER_PREFERENCE)` so DWM knows the corner too. | Rounds, with the system shadow and snap-layout affordance. |
| Windows 10 | Transparent surface + the page's `clip-path`. The DWM attribute does not exist and the call fails harmlessly. | Rounds. |
| macOS | Native window rounding — the window is `decorated` with a hidden title and a full-size content view, so AppKit cuts the corner itself. | Free. |

⛔ **Never gate the corner on which desktop the session claims to be.** Alpha
compositing is core to Wayland — it is not an extension a compositor may decline
— so there is nothing to detect. Gating on desktop identity is what made this
setting cycle fixed→broken for years: every desktop not named in the list shipped
square corners, and the one that *was* named was recognised from scraped
environment that a daemon-launched GUI does not have, so the same machine
rendered both ways on different launches. Key on the capability.

⭐ **On Windows the corner is drawn twice, and both must agree.** The page's
`clip-path` is what makes the corner visible; `DWMWA_WINDOW_CORNER_PREFERENCE` is
what makes it real to the system, so the drop shadow follows the curve and the
maximize button offers snap layouts. A corner DWM does not know about is a shape
drawn inside a square window, with a square shadow around it. Because DWM is a
second painter, it is told `DWMWCP_DONOTROUND` when the window is maximized —
otherwise it would round a corner the page has already squared.

⚠ **Windows 10 has no such attribute and the call fails there.** That is the
designed outcome, not a gap to paper over with a version probe: the CSD path is
what rounds the window on 10, it is already in force, and a probe would only buy
the privilege of skipping a call whose failure costs nothing.

**The corner is a tested contract, and it must stay one.** `scripts/corner-contract.sh`
asserts it in pixels against a headless Wayland-native compositor. Do not
"verify" a corner change from `dom.shell_root_border_radius` or from
`server app screenshot`: the first reports the CSS that was *asked for* and reads
`10px` on a window whose corners are square, and the second returns RGB, so the
snapshot flattens the very alpha that distinguishes a rounded corner from a
square one. Only a compositor grab can tell them apart.

### Gradient pad (theme editor)

The pad is the theme engine's direct-manipulation surface: gradient stops are
dragged on a square canvas, and it takes its cues from Arc's gradient editor.

- **The grid is magnetic, not quantised.** A stop within 7px of the visible 24px
  gridline is pulled onto it; further away it moves freely. A pad that rounds
  *every* point to the grid cannot express a stop at 37% and has taken the pen
  out of the designer's hand — the magnet snaps the placements that wanted to be
  exact and leaves the rest alone.
- **Both pad edges are snap targets.** The pad is not a whole number of cells, so
  the nearest-gridline arithmetic can never land on the far edge — and the corner
  is exactly the placement that most wants to be exact.
- **Alt suspends the magnetism** for the placement that genuinely belongs between
  two lines.
- **A snapped stop says so**, with a thin accent halo. A magnet the eye cannot
  read is worse than none: the point moves by an amount the hand did not ask for
  and nothing explains why. The halo is derived from the stop's coordinates, not
  remembered from the drag that placed it, so a theme loaded from disk reads the
  same as one just dragged.
- **The grid the magnet uses and the grid the pad paints are the same number.** A
  stop must never snap to a line the eye cannot see.
