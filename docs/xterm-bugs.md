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
| [scrollback-lost-on-gui-restart](#scrollback-lost-on-gui-restart) | Scroll position lost when GUI restarts (daemon survives) | FIXED 2026-05-26 |
| [resume-gate-too-restrictive](#resume-gate-too-restrictive) | Resuming a session that's mid-output (no prompt visible) takes 60-160s to clear "not ready" gate | FIXED 2026-05-25 |
| [scroll-jump-on-input](#scroll-jump-on-input) | Typing in a session yanks viewport to a "particular spot" (flicker between spot and prompt); scroll-lock variant kicks user back when scrolling | PARTIALLY FIXED 2026-05-26 — input-snap skipped when user is reading scrollback |
| [dom-leak-on-session-start](#dom-leak-on-session-start) | Portion of *prior* message context flashes briefly during session start/switch then goes away | OPEN, uninvestigated |
| [clipboard-double-paste](#clipboard-double-paste) | Class: text select + middle-click pastes selection THEN clipboard (double); Ctrl+Shift+V double paste; selection-vs-clipboard ordering bugs | OPEN, investigating |
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
Both paths now have regression coverage:
- `repaintActiveEntry` guard is asserted at shell.rs ~69861.
- `followPromptForEntry` guard is asserted by
  `retained_replay_script_followPromptForEntry_guards_user_scrollback`
  (added 2026-05-26). Fails if either the function definition or the
  `String(entry.scrollbackIntent || 'PromptFollow') === 'UserScrollback'`
  early-return guard is removed.

### Telemetry
None yet. A `xterm_scrollback_lost_on_switch` event could be added by
sampling the buffer's `yDisp` before and after repaint and emitting when
it changes by more than a small delta despite no user input.

### Related memory
`[[xterm-scrollback-bug]]`

---

## scrollback-lost-on-gui-restart

**STATUS:** FIXED 2026-05-26 — localStorage-based persistence verified live on jojo.

### Symptom
User scrolls up in session A, agent deploys a GUI binary (kills GUI,
relaunches; daemon stays alive), user reopens session A — scroll position
is at the bottom again. User loses their scrollback position.

### Reproduction
1. Long-running session with rich scrollback.
2. Scroll up so the live cursor is well off-screen.
3. Restart the GUI process (daemon survives).
4. After GUI re-launches, navigate to the original session.
5. Before fix: viewport snapped to bottom; user's scroll position gone.
6. After fix: viewport restored to ~same row distance from bottom.

### Root cause
`scrollbackIntent` and `viewportY` were JS-side state inside
`window.__yggtermXtermSessionSnapshots[sessionPath]` — a process-local
dictionary. When the GUI process dies, the dict is gone. The on-mount
restore path needed an out-of-process store.

### Fix
WebKitGTK-backed localStorage **does persist** across GUI restarts (file
at `~/.local/share/dev.yggterm.Yggterm/localstorage/dioxus_index.html_0.localstorage`).
We piggyback on it:

- **Save** on every `setScrollbackIntent` call, every `captureSessionXtermSnapshot`
  call, and every `term.onScroll` event (throttled to 200ms when
  `scrollbackLocked`). Saved value:
  `{intent, viewportY, baseY, distanceFromBottom, locked, reason, savedAtMs}`.
- **Restore** in `restoreXtermSessionSnapshotOnConstructed` when the
  in-memory snapshot is absent: pre-arm `scrollbackIntent='UserScrollback'`
  immediately so initial replay doesn't auto-scroll to bottom, then poll
  at 1s/2s/3s/.../8s. Each poll waits for `baseY >= distanceFromBottom`
  AND baseY-stable-for-600ms (replay finished), then `forceXtermViewportY(baseY - distanceFromBottom)`.
- **Restore-window guard** suppresses save during the 8s deadline so
  post-restart replay doesn't overwrite the user's saved position.

### Code locations
- `crates/yggterm-shell/src/shell.rs:~57110` — `setScrollbackIntent` calls `persistScrollStateToLocalStorage` (with restore-in-flight guard)
- `crates/yggterm-shell/src/shell.rs:~57110` (just above) — `persistScrollStateToLocalStorage` / `loadScrollStateFromLocalStorage` helpers
- `crates/yggterm-shell/src/shell.rs:~57790` — `restoreXtermSessionSnapshotOnConstructed` localStorage fallback path
- `crates/yggterm-shell/src/shell.rs:~57790` (just above) — `tryApplyPendingPersistedScrollRestore` (stability gate)
- `crates/yggterm-shell/src/shell.rs:~61220` — `term.onScroll` listener (throttled persist + opportunistic apply)

### Tests
`terminal_eval_script_persists_scroll_state_to_localstorage` in
yggterm-shell asserts the three helper names (`persistScrollStateToLocalStorage`,
`loadScrollStateFromLocalStorage`, `tryApplyPendingPersistedScrollRestore`)
and the `yggterm-scroll:` localStorage key prefix all appear in the
generated terminal-eval script. Fails if any are removed.

### Telemetry
`scrollback_intent` debug event already fires on every change. Restore
emits `persisted_scroll_restored host=... target=... distance=... reason=...`
debug event. Host-state fields: `persistedScrollRestorePending`,
`persistedScrollRestoreApplied`, `persistedScrollRestoreTargetViewportY`.

### Verification (2026-05-26, live on jojo)
- Scroll up 40 lines in `remote-session://dev/019d0000-0000-7000-8000-000000000001` → `viewport_y=960, base_y=1000`.
- localStorage SQLite row written: `{viewportY: 960, baseY: 1000, distanceFromBottom: 40, ...}`.
- SIGTERM GUI, relaunch. After ~6s settle: `viewport_y=960, base_y=1000, scrollback_intent=UserScrollback, scrollback_locked=true, last_viewport_force_reason=persisted_scroll_restore:poll_2000`.
- Screenshot confirms user is reading scrollback (not at bottom prompt).

### Related memory
`[[xterm-scrollback-bug]]`

---

## resume-gate-too-restrictive

**STATUS:** FIXED 2026-05-25 (commit `332072e`) — verified live, 740x faster.

### Symptom
User opens (or resumes) a remote session that is in the middle of long
output (e.g., a pytest run, a Codex agent mid-reply, anything that isn't
showing a fresh prompt). The terminal CONTENT renders quickly, but the
session is gated as "not ready" for 60–160+ seconds. The viewport may
look correct visually while the readiness state machine keeps reporting
`active_view_mode: Terminal, ready: false, reason: "active remote
terminal is input-enabled without a prompt-ready surface"`.

### Reproduction (observed live on jojo, 2026-05-25)
1. Open remote Codex session that's mid-output.
2. Check `~/.local/bin/yggterm server app state | jq .data.terminal_open_attempt`.
3. Observe `first_meaningful_output_to_ready_ms: 161324` (i.e., 161s
   from first output to "ready") and `terminal_settled_kind: "problem"`.

### Root cause
The readiness check `terminal_surface_has_prompt_ready_text` requires
the visible surface text to match a prompt pattern
(`terminal_chunk_has_prompt_output`, `terminal_chunk_has_codex_prompt_output`,
or `terminal_chunk_is_codex_interactive_setup_prompt`).

For a session with active output (output rows scrolling past, no prompt
visible at the cursor row), none of these match. The recovery loops at
shell.rs:44701 and ~44841 spin up to 60 attempts each, polling every
750–1500 ms. With multiple concurrent loops, total wait reaches 161s+.

Specifically:
```
retained_remote_surface_should_wait_for_prompt_ready(...)
    -> terminal_surface_has_prompt_ready_text(host_surface_text) == false
    -> stays "waiting for prompt-ready"
```

The gate's intent is to avoid presenting a stale-looking surface as
ready, but it conflates "session is usable" with "prompt is currently
visible at the cursor line". A session with live output IS usable.

### Workaround / fix
**Not yet implemented.** Sketch of the fix:

1. Treat "live, growing transcript with recent PTY bytes" as a valid
   ready signal (in addition to prompt-ready). If the daemon reports
   active terminal output within the last N seconds AND the surface
   has any non-empty text, the session is ready.
2. Specifically, `remote_retained_surface_fault_should_invalidate`
   should not return true for "input-enabled without prompt-ready
   surface" when recent PTY bytes have arrived; that's a real
   running session, not a fault.
3. Keep the prompt-ready check as the criterion for *resume* surfaces
   that haven't seen PTY bytes yet, but allow already-streaming
   sessions to bypass it.

### Code locations
- `crates/yggterm-shell/src/shell.rs:~7597` — `terminal_surface_has_prompt_ready_text`
- `crates/yggterm-shell/src/shell.rs:~7651` — `retained_remote_surface_should_wait_for_prompt_ready`
- `crates/yggterm-shell/src/shell.rs:~7854` — `remote_retained_surface_fault_should_invalidate`
- `crates/yggterm-shell/src/shell.rs:~44701` — recovery loop #1 (server snapshot prompt-ready replay), up to 60 attempts
- `crates/yggterm-shell/src/shell.rs:~44841` — recovery loop #2 (daemon snapshot recovery), up to 60 attempts
- `crates/yggterm-shell/src/shell.rs:~3192` — `mark_terminal_open_attempt_ready_for_session` (where ready is set)

### Tests
TBD. Test should set up a remote session with live mid-output PTY bytes
(no prompt) and assert `mark_terminal_open_attempt_ready_for_session`
fires within a small bound, not 60+ seconds.

### Telemetry
The existing `terminal_open_attempt.first_meaningful_output_to_ready_ms`
already captures this perfectly. A regression dashboard can simply alert
on values > 5000.

### Related
Reported live by user 2026-05-25: "Why is this session gated? This
gating on resuming needs to take as less time as possible, but this
getting stuck is also another xterm bug painpoint."

---

## scroll-jump-on-input

**STATUS:** PARTIALLY FIXED 2026-05-26 (commit 6c757b1) — the "snap to
bottom on input while reading scrollback" variant is fixed; flicker and
scroll-lock variants still need real-repro telemetry to attribute.

### Fix (variant 1: snap-to-bottom on input while scrolled up)
In the `term.onData` handler in shell.rs (the JS code generated for each
xterm host), before firing `setScrollbackIntent('PromptFollow', 'input')`
and `scrollLiveCursorIntoView(true, 'input')`, check:

```js
const _scrollJumpUserIsReadingScrollback =
    scrollbackIntent === 'UserScrollback'
    && (baseY - viewportY) > 5;
if (!_scrollJumpUserIsReadingScrollback) {
    // existing snap-to-bottom logic
} else {
    // keystroke still goes to PTY via queueTerminalInputData; viewport stays.
    sendTerminalEvent({ kind: 'debug', message: `input_snap_skipped ...` });
}
```

5-row threshold: at small distances the user probably wants the prompt
visible while typing; at larger distances they're intentionally reading.

Records `inputSnapSkippedCount`, `lastInputSnapSkippedAtMs`,
`lastInputSnapSkippedDistanceRows` on the host entry for app-control
visibility.

### Symptom
A class of related bugs where the viewport "jumps to a particular spot"
unexpectedly. Variants reported by user:

1. **Flicker-jump on type.** User is reading scrollback; pressing a key
   causes the viewport to flicker very fast between "the particular spot"
   and the prompt. Looks like two competing scroll handlers fighting per
   keystroke.
2. **Scroll-lock variant.** User tries to scroll down a little; after a
   small delta the viewport is yanked back to the same "particular spot".
3. **Random scrollback during session switch.** Switching to a session
   sometimes lands on a stale viewport instead of bottom or last-known.

### Reproduction
Not yet captured deterministically. Reproduce path under investigation:
mid-output session, scroll up partway, type any key, watch viewport.
Live host `jojo`, active session
`remote-session://dev/019d0000-0000-7000-8000-000000000001`.

### Root cause
Unknown. Hypotheses:

- **Two competing handlers per input.** `handleExternalReadNudge` at
  shell.rs:~60054 fires `setScrollbackIntent('PromptFollow', 'external_input')`
  + `scrollLiveCursorIntoView(true, 'external_input')` (force to bottom).
  At the same time, the data event at shell.rs:~61490 fires
  `setScrollbackIntent('PromptFollow', 'input')` +
  `scrollLiveCursorIntoView(true, 'input')`. Both call
  `forceXtermViewportY(baseY)`. If the first lands and the second
  re-resolves baseY after a write, viewport flickers.
- **Snapback to in-memory snapshot.** If snapshot capture ran while user
  was at viewport=X, a later restore-from-snapshot path could pull viewport
  back to X — the "particular spot" — every time it fires.
- **Visual-mismatch-at-bottom.** `syncScrollbackLock` at shell.rs:~57318
  detects `publicViewportY >= baseY && viewportY < baseY` and flips
  `scrollbackLocked = false` even when user is genuinely scrolled-up.
  This racing with `forceXtermViewportY` retries could cause the lock to
  toggle false→true→false repeatedly, fighting the user.

### Workaround / fix
Not yet implemented. Next steps:

1. Add per-frame telemetry: emit `xterm_scroll_jump` event whenever
   `forceXtermViewportY` is called with reason involving 'input'/'external_input'
   and the buffer's baseY changes within 200ms of the call. Capture
   `(reason, before_viewport, after_viewport, baseY)` to identify the
   competing caller.
2. Add a "user is scrolled up" guard at the data-event input path so a
   keystroke does not auto-scroll to bottom when user is still scrolled
   up beyond N rows.
3. Investigate whether `handleExternalReadNudge` and the data input both
   need to force-follow, or if only one should.

### Code locations
- `crates/yggterm-shell/src/shell.rs:~60054` — `handleExternalReadNudge` (PromptFollow + cursor scroll)
- `crates/yggterm-shell/src/shell.rs:~61490` — terminal input data event (PromptFollow + cursor scroll)
- `crates/yggterm-shell/src/shell.rs:~57318` — `syncScrollbackLock` visual-mismatch path
- `crates/yggterm-shell/src/shell.rs:~57647` — `forceXtermViewportY` definition

### Tests
None yet. Need a JSDOM-level test that drives a keystroke into a
scrolled-up xterm and asserts viewport doesn't change.

### Telemetry
Proposed: `xterm_scroll_jump_after_input` debug event with `before_y`,
`after_y`, `base_y`, `reason`, `dt_ms`.

### Related
Reported by user 2026-05-26: "when I type the xterm buffer jumps to a
random selected particular spot... scroll lock which also this session
has; which means if I try to scroll down I will get kicked into this
spot after trying a little bit."

---

## dom-leak-on-session-start

**STATUS:** OPEN — reported live 2026-05-26 on jojo.

### Symptom
When starting or switching to a session after a long time, during the
startup window a portion of *prior* message context appears briefly in a
weird way. After session restore + further input it goes away.

Reads like stale DOM rows from a previous session's `.xterm-rows`
children being left attached during the swap, or innerHTML from a
previous snapshot being injected before the new buffer fully renders.

### Reproduction
Not yet captured deterministically. Conditions: session left idle for
"long time", then switch to it. The xterm host DOM remains mounted; a
swap or replay populates it; for a few frames the OLD rows are visible.

### Root cause
Unknown. Hypotheses:

- **Retained-host DOM swap timing.** When we swap session-bound state
  inside the same xterm host (`__yggtermXtermHosts[hostId]`), the new
  session's `term.reset()/clear()` may run a frame after the new
  innerHTML/buffer is mounted, leaving the old `.xterm-rows` rendered.
- **Snapshot innerHTML reattach.** If `captureSessionXtermSnapshot`
  stored an `innerHTML` blob and `restoreXtermSessionSnapshotOnConstructed`
  injected it before `term.reset()`, the prior session's text would
  paint for one frame.
- **Inactive-host hidden but not cleared.** If we visually hide one
  host and reveal another without clearing the hidden one's buffer,
  the brief overlap (during fade/transition) shows the wrong content.

### Workaround / fix
Not yet implemented. Next steps:

1. Add a "first paint" telemetry hook that captures the visible host's
   first 3 frames as text samples and emits `xterm_first_paint host=... text_sample=...`.
2. Compare those samples with the captured snapshot from the PRIOR
   session; if they match prior, we have a leak.
3. Audit `term.reset()` / `term.clear()` ordering vs first xterm.write
   in `restoreXtermSessionSnapshotOnConstructed` (shell.rs:~57803-57815).
4. Confirm whether retained-host swap clears `host.innerHTML` before
   the new term is constructed.

### Code locations
- `crates/yggterm-shell/src/shell.rs:~57788` — `restoreXtermSessionSnapshotOnConstructed`
- `crates/yggterm-shell/src/shell.rs:~57803` — `term.reset()/term.clear()` call sites
- `crates/yggterm-shell/src/shell.rs:~55539` — `entry.sessionPath = host.getAttribute("data-terminal-session-path")` (host rebind)

### Tests
None yet. Hard to assert; will need a probe that captures the host
innerText immediately on session switch and asserts it doesn't contain
substrings from the previous session's last screen.

### Telemetry
Proposed: `xterm_first_paint_sample` capturing first 256 chars of
`host.innerText` 0/16/64 ms after host-rebind, compared against the
prior session's known last screen.

### Related
Reported by user 2026-05-26: "when I start or switch to a session after
a long time and during startup I see a portion of my message context in
a weird way randomly. Upon session restore and after geting it goes
away."

---

## clipboard-double-paste

**STATUS:** OPEN — long-standing class with multiple variants. Currently
investigating; telemetry hooks to be added per the plan below.

### Symptom — class with multiple variants
Yggterm's clipboard plumbing has a recurring failure mode where a single
user intent (paste once) results in **two paste operations**, with content
from different sources concatenated or interleaved.

Variants reported by the user (collected over time):

1. **Selection + middle-click double-paste (current variation, 2026-05-26).**
   User selects text in the terminal (PRIMARY selection set) and middle-clicks
   to paste. Result: the SELECTED text gets pasted first, immediately
   followed by the CLIPBOARD contents. Expected: only PRIMARY should paste
   on middle-click; CLIPBOARD should be untouched.

2. **`Ctrl+Shift+V` double-paste (past variation).** User presses
   `Ctrl+Shift+V`. Result: clipboard contents paste twice. Expected: one paste.

3. **Selection-vs-clipboard ordering** (suspected related): paths that
   should consult only one of PRIMARY/CLIPBOARD end up consulting both,
   merging or re-emitting content.

### Reproduction (current variation, on jojo 2026-05-26)
1. Open any session with text content.
2. Select a non-empty range with the mouse (sets PRIMARY).
3. Middle-click anywhere in the prompt area.
4. Observe: the selected text appears at the prompt, followed by whatever
   was in the clipboard. Should be just the selected text.

### Root cause
Unknown — to be confirmed. Hypotheses:

- **Two listeners both consume the same event.** xterm.js has built-in
  middle-click → PRIMARY paste, AND Yggterm-side has its own pointer
  handlers (e.g. `recordPrimarySelectionFromXterm`, primary-selection
  listeners) that may also call a paste path. If both fire on the same
  middle-click, you get two pastes (one from each handler).
- **`yggterm-shell`'s `primary_selection_paste` handler dispatches via
  both the dioxus side and the xterm side.** If both think they own the
  event, both `term.paste(...)` calls fire.
- **`Ctrl+Shift+V` variant**: similar — keymap binding fires `term.paste`
  AND a Yggterm-side IPC paste request, both completing.
- **PRIMARY/CLIPBOARD confusion**: the middle-click handler might call
  `read_clipboard()` instead of (or in addition to) `read_primary()`,
  emitting clipboard content where only primary was wanted.

### Workaround / fix (planned)
1. **Add telemetry** to attribute future repros: emit
   `xterm_paste_event { source: primary|clipboard, triggered_by:
   middle_click|ctrl_shift_v|context_menu|js_term_paste|external_input,
   payload_length, dt_since_previous_ms }` on every paste-path entry.
   When `dt_since_previous_ms < 300` and source differs, log as
   `xterm_paste_double_fire` — that's the diagnostic signature.
2. **Single owner for middle-click**: pick exactly one path
   (xterm.js built-in OR Yggterm primary-selection-paste) and disable
   the other for middle-click.
3. **Single owner for `Ctrl+Shift+V`**: same — pick one.
4. **Selection vs clipboard separation**: a middle-click handler MUST
   only read PRIMARY; a `Ctrl+Shift+V` handler MUST only read CLIPBOARD.
   Any code path that reads both for one trigger is a bug.

### Code locations (suspected — to be confirmed by repro + telemetry)
- `crates/yggterm-shell/src/shell.rs:~59311` —
  `primarySelectionSessionPath` and `primary_selection_paste` related code
- `crates/yggterm-shell/src/shell.rs:~59367` — `setScrollbackIntent('PromptFollow', 'primary_selection_paste')`
- `crates/yggterm-shell/src/shell.rs` — search for `term.paste(`,
  `read_primary`, `read_clipboard`, `ClipboardOwnerKind`, `paste_primary`
  to find all paste entry points.

### Tests
None yet. Need a JSDOM-level test that simulates middle-click on a
selected terminal range and asserts exactly ONE paste fires with PRIMARY
content (no clipboard concatenation).

### Telemetry
Proposed (not yet shipped): `xterm_paste_event` and `xterm_paste_double_fire`
debug events as described above. Will be added in the same change that
adds telemetry hooks at every paste entry point.

### Related
Reported live by user 2026-05-26: "text select copy paste or ctrl+shift+c/v
copy paste. This is a great bug class and has many variations that I have
faced, asked to fixed over the time. Currently text select and middle
click paste pastes the selected text first and then the clipboard next.
In the past, I have seen double clipboard paste on ctrl+shift+v etc."

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
