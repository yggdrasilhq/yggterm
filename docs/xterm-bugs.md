# xterm.js Bug Registry

This file is the central index of every xterm.js integration bug that has
required (or still requires) a Yggterm-side workaround. **One section per
bug.** Inline code comments at workaround sites point back here via:

```rust
// XTERM-BUG: <short-id>
// See docs/xterm-bugs.md#<short-id>
```

Keep this file specific and bug-shaped. For the broader contract — what
xterm.js owns vs what the shell owns, cursor/prompt semantics, etc. — see
[`docs/xterm.md`](xterm.md).

## How to use this file

- **Reading**: search for an `XTERM-BUG: <id>` comment in code, then find the
  matching `## <id>` section here.
- **Adding a new entry**: copy the template at the bottom. Required fields:
  Symptom, Reproduction, Workaround, Code locations. Optional but
  encouraged: Upstream link, Telemetry, Tests.
- **Closing an entry**: when an upstream xterm.js fix lands and we drop the
  workaround, set `STATUS: HISTORICAL` and keep the section as institutional
  memory.
- **Per [AGENTS.md](../AGENTS.md):** every regression should add an
  inline-comment anchor AND a registry entry before the runtime fix is
  considered done. See also memory `[[spec-xterm-bug-registry]]`.

## Index

| ID | Symptom | Status |
|----|---------|--------|
| [scrollback-lost-on-session-switch](#scrollback-lost-on-session-switch) | User-scrolled scrollback collapses to live cursor when switching sessions | FIXED, guard in place |
| [slow-jitter](#slow-jitter) | Some sessions exhibit visible per-frame jitter under steady PTY output | OPEN, uninvestigated |
| [blank-rendering-region](#blank-rendering-region) | Region inside an active session goes blank until forced redraw | OPEN, uninvestigated |

---

## scrollback-lost-on-session-switch

**STATUS:** FIXED — guard in `repaintActiveEntry` prevents regression.

### Symptom
User scrolls up through scrollback in session A, switches to session B,
switches back to A. The scrollback position has been reset to the live
cursor (bottom of buffer); previously-visible scrollback rows are gone from
the viewport even though they're still in the buffer.

### Reproduction
1. Long-running session with rich scrollback (>100 rows above viewport).
2. Scroll up so the live cursor is well off-screen.
3. Click another session in the sidebar.
4. Click back to the original session.
5. Observe: viewport snaps back to live cursor; user's scroll position lost.

### Root cause
Session-switch repaint was calling `forcePromptFollow` unconditionally,
which scrolls the live cursor into view as a side effect. The intent of
the call was to ensure the prompt was visible *for sessions where the user
was already at the bottom*, but with no guard it also reset scroll position
for users who were actively reading scrollback.

### Workaround / fix
`repaintActiveEntry` only calls `forcePromptFollow` when a user-scroll
guard reports false (i.e., the user is not currently in scrollback). The
`forcePromptFollow` JS-side helper is at `entry.forcePromptFollow(reason)`.

### Code locations
- `crates/yggterm-shell/src/shell.rs:69857` — the inline regression comment
  ("Activation repaint must NOT unconditionally call forcePromptFollow")
- `crates/yggterm-shell/src/shell.rs:69861` — the assertion that documents
  the contract for tests
- `crates/yggterm-shell/src/shell.rs:59817` — `forcePromptFollow` JS
  definition (`(reason = 'prompt_follow') => scrollLiveCursorIntoView(true, reason)`)

### Tests
Test asserts that activation repaint emits the guard before any
`forcePromptFollow` call (see assertion at shell.rs:69861).

### Telemetry
None yet. A `xterm_scrollback_lost_on_switch` event could be added by
sampling the buffer's `yDisp` before and after repaint and emitting when
it changes by more than a small delta despite no user input.

### Related memory
`[[xterm-scrollback-bug]]`

---

## slow-jitter

**STATUS:** OPEN — symptom observed, root cause unknown.

### Symptom
Some live sessions exhibit visible per-frame jitter under steady PTY
output. Rows shift a few pixels vertically, or cursor position lags one
frame. Not all sessions are affected; not reliably reproducible.

### Reproduction
Not yet captured deterministically. Anecdotal: jojo (Wayland) under
heavy Codex output. Needs a repro fixture.

### Root cause
Unknown. Hypotheses to investigate:
- WebKit/GTK render-loop coalescing on Wayland
- xterm.js dirty-row tracking interacting with our retained-host swap
- CSS scaling/transform on the parent affecting subpixel layout

### Workaround / fix
None yet.

### Code locations
TBD — first step is adding telemetry that captures per-frame yDisp +
buffer length to identify whether jitter is xterm-side or compositor-side.

### Tests
None yet.

### Telemetry
Proposed: `xterm_render_jitter` event emitted from the render callback
when consecutive frames show inconsistent yDisp vs scroll-bottom expectation.

---

## blank-rendering-region

**STATUS:** OPEN — symptom observed, root cause unknown.

### Symptom
A rectangular region inside an active session viewport renders blank
(theme background color, no glyphs) even though buffer rows exist for
those rows. A forced redraw (resize, focus toggle, scroll) fills it in.

### Reproduction
Not yet captured deterministically. Anecdotal during long sessions or
after resize/restore cycles.

### Root cause
Unknown. Hypotheses:
- xterm.js DOM renderer leaving stale `.xterm-rows` children after a
  retained-host swap
- WebKit GPU layer being torn but not invalidated
- Our retained-replay path emitting rows out of order with xterm's
  internal expectation

### Workaround / fix
None yet. Current escape hatches that work but are expensive:
- Resize event triggers full redraw
- Focus toggle (alt-tab) often clears it

### Code locations
TBD.

### Tests
None yet.

### Telemetry
Proposed: `xterm_blank_region` event emitted when an app-control probe
detects DOM `.xterm-rows` children with no rendered glyphs while the
buffer reports non-empty content at those row indices.

---

## Template (copy for new entries)

```markdown
## <short-id>

**STATUS:** OPEN | FIXED | HISTORICAL

### Symptom
What the user sees.

### Reproduction
Numbered steps. If not reproducible, say "Not yet captured deterministically"
and link any related incident traces.

### Root cause
Upstream xterm.js cause if known, plus link to upstream issue. Otherwise
list hypotheses to investigate.

### Workaround / fix
Concept-level description. Don't paste the code; point to file:line.

### Code locations
- `crates/.../...:NNN` — what lives here

### Tests
Regression tests that fail when this bug regresses.

### Telemetry
Event names + when they fire.

### Related memory
`[[memory-name]]` links to any related memories.
```
