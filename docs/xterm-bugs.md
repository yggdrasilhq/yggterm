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
| [scrollback-lost-on-session-switch](#scrollback-lost-on-session-switch) | User-scrolled scrollback collapses to live cursor when switching sessions | PARTIALLY FIXED |
| [scrollback-lost-on-gui-restart](#scrollback-lost-on-gui-restart) | Scroll position lost when GUI restarts (daemon survives) | OPEN, needs persistence |
| [slow-jitter](#slow-jitter) | Some sessions exhibit visible per-frame jitter under steady PTY output | OPEN, uninvestigated |
| [blank-rendering-region](#blank-rendering-region) | Region inside an active session goes blank until forced redraw | OPEN, uninvestigated |

---

## scrollback-lost-on-session-switch

**STATUS:** PARTIALLY FIXED — within-session-life session switch is now
guarded; scroll position is still lost across GUI restart (see
[scrollback-lost-on-gui-restart](#scrollback-lost-on-gui-restart)).

### Symptom
User scrolls up through scrollback in session A, switches to session B,
switches back to A. The scrollback position resets to the live cursor;
previously-visible scrollback rows are gone from the viewport.

### Reproduction
1. Long-running session with rich scrollback (>100 rows above viewport).
2. Scroll up so the live cursor is well off-screen.
3. Click another session in the sidebar.
4. Click back to the original session.
5. Before fix: viewport snaps back to live cursor; user's scroll position lost.

### Root cause
Two distinct paths reset the viewport:

1. **`repaintActiveEntry`** — activation repaint after session switch.
   Was calling `forcePromptFollow` unconditionally. Already guarded by
   `scrollbackIntent !== 'UserScrollback'` check (committed pre-2026-05-25).

2. **`followPromptForEntry`** — retained-replay path. Called from
   ~7 sites (`retained_replay_xterm_session_snapshot`,
   `retained_replay_cached_visible`, `retained_replay_existing_visible`,
   `retained_replay_existing_scrollback`, `retained_replay_write`,
   etc.). Was UNGUARDED until 2026-05-25 — every retained replay (which
   fires on session switch when xterm needs to re-mount/re-apply
   snapshot) yanked the viewport to the bottom. **This was the actual
   biting bug** (commit 36dfe61 only fixed path 1, path 2 was still
   resetting scroll).

### Workaround / fix
Both code paths now early-return when
`entry.scrollbackIntent === 'UserScrollback'`. Specifically:

- `repaintActiveEntry` guards inline (shell.rs ~62660).
- `followPromptForEntry` guards at function entry, so ALL ~7 retained-replay
  callers inherit the guard (shell.rs ~62236).

### Code locations
- `crates/yggterm-shell/src/shell.rs:~62236` — `followPromptForEntry`
  guard (`XTERM-BUG:` anchor)
- `crates/yggterm-shell/src/shell.rs:~62660` — `repaintActiveEntry`
  guard (`XTERM-BUG:` anchor)
- `crates/yggterm-shell/src/shell.rs:~59831` — `forcePromptFollow` JS
  definition
- `crates/yggterm-shell/src/shell.rs:~57111` — `setScrollbackIntent`
  (sets `UserScrollback` when user wheels/PageUps)

### Tests
The test assertion at ~69861 covers `repaintActiveEntry`. A new assertion
for `followPromptForEntry`'s guard is needed (TODO: assert
"if (entry && String(entry.scrollbackIntent || 'PromptFollow') === 'UserScrollback')"
appears inside the followPromptForEntry definition).

### Telemetry
None yet. A `xterm_scrollback_lost_on_switch` event could be added by
sampling the buffer's `yDisp` before and after repaint and emitting when
it changes by more than a small delta despite no user input.

### Related memory
`[[xterm-scrollback-bug]]`

---

## scrollback-lost-on-gui-restart

**STATUS:** OPEN — needs a persistence layer.

### Symptom
User scrolls up in session A, agent deploys a GUI binary (kills GUI,
relaunches; daemon stays alive), user reopens session A — scroll position
is at the bottom again. User loses their scrollback position.

### Reproduction
1. Long-running session with rich scrollback.
2. Scroll up so the live cursor is well off-screen.
3. Restart the GUI process (daemon survives).
4. After GUI re-launches, navigate to the original session.
5. Observe: viewport is at the bottom; user's scroll position is gone.

### Root cause
`scrollbackIntent` and `viewportY` are JS-side state inside
`window.__yggtermXtermSessionSnapshots[sessionPath]`. This dictionary is
in-process. When the GUI process dies, it's gone.

The on-mount restore path (shell.rs ~57846) DOES support restoring
`UserScrollback` intent + `viewportY` — but only when it has a snapshot to
restore from, which the in-process dict doesn't provide after restart.

### Workaround / fix
**Not yet implemented.** Two viable plans:

1. **Daemon-side persistence.** Add `scrollback_intent` and
   `scrollback_viewport_y` fields on the daemon's `ManagedSessionView`.
   GUI debounces (~500ms) and sends `ScrollState { session_path, intent,
   viewport_y }` to daemon. Daemon stores per session, includes in
   `SnapshotSessionView` sent to GUI. GUI on terminal mount checks the
   snapshot for prior scroll state and applies it via the existing
   restore path. Survives GUI restart whenever daemon stays alive.
   Survives daemon hot-restart because `persisted_state_for_update_restart`
   serializes the full ManagedSessionView.

2. **GUI-only file persistence.** GUI writes
   `~/.yggterm/xterm-scrollback-state.json` on scroll change (debounced),
   reads on startup. Simpler but doesn't survive a daemon-only restart.

Plan 1 is the right one — it's the single source of truth path. Estimated
~5-file change: ManagedSessionView field, IPC request type, daemon
handler, snapshot field, GUI mount-time restore call.

### Code locations
- `crates/yggterm-shell/src/shell.rs:~56917` — snapshot construction
  (where scrollbackIntent is captured into the in-memory snapshot)
- `crates/yggterm-shell/src/shell.rs:~57846` — restore path (already
  correctly applies `UserScrollback` + viewportY when present)
- `crates/yggterm-server/src/lib.rs` — `ManagedSessionView` (add fields)
- `crates/yggterm-server/src/lib.rs` — `SnapshotSessionView` (add fields)

### Tests
None yet.

### Telemetry
Proposed: emit `xterm_scrollback_state_persisted` debounce events to
trace, plus `xterm_scrollback_state_restored` on successful restore.

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
