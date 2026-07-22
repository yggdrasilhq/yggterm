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
| [remote-cc-replay-codex-only](#remote-cc-replay-codex-only) | Resumed Claude Code (remote-cc) viewport renders without its prompt box / blanks on mount+remount because the retained-replay readiness gate only recognizes Codex prompts | FIX PENDING LIVE VERIFY 2026-05-31 |
| [xterm-pipeline-latency](#xterm-pipeline-latency) | Interactive feel ~6fps vs ghostty/VSCode: DOM renderer forced + 160ms write-frame latch over xterm's own scheduler | FRAME BUDGET SHIPPED+VERIFIED 2026-05-31 (160→16ms); canvas renderer deferred (readiness heuristic) |
| [scrollbar-not-draggable](#scrollbar-not-draggable) | Sleek thin scrollbar visible but cannot be dragged | FIXED 2026-05-28 |
| [content-scooped-on-session-switch](#content-scooped-on-session-switch) | Switching sessions: middle rows disappear, top + bottom remaining text presented as continuous | OPEN, telemetry added |
| [keepalive-restart-viewport-only](#keepalive-restart-viewport-only) | After GUI restart, keep-alive sessions show only viewport's worth of content; daemon had retained more in vt100 ring but didn't serve it | FIXED & VERIFIED LIVE 2026-05-28 on jojo 2.7.62 — local shell base_y 893→893, codex base_y 144→144 across kill+launch. Earlier reopen was misdiagnosed: the avikalpa_opc "only viewport" symptom was a stale resume-codex wiring issue on that specific session, not a scrollback retention gap. |
| [surface-recovery-false-positive-on-transient](#surface-recovery-false-positive-on-transient) | "Shadow" blank flash + multi-second re-gate, and (worse) input-disable → re-resume → exhaust → session yanked closed, all triggered by a TUI's normal clear+redraw transient misread as a broken/empty/non-prompt surface | FIXED 2026-06-03 (settle-gates) — empty-surface 2.8.11, non-prompt 4-point fix |
| [persisted-scroll-restore-fights-follow](#persisted-scroll-restore-fights-follow) | After GUI restart, every click/keystroke flickers between a saved scroll offset and the live bottom | FIXED 2026-06-02 |
| [xterm-host-registry-leak](#xterm-host-registry-leak) | Switching/restarting sessions accumulates orphaned xterm.js instances (cleanup keyed to mount epoch that changes on remount) → growing latency on selection/paste/switch | FIXED 2026-06-02 |
| [chunk-ring-trim-drops-mid-stream](#chunk-ring-trim-drops-mid-stream) | Middle chunks of TUI output silently missing: yggterm-server chunk ring trims oldest while a client's read-cursor is behind the trim, and read(cursor) returns only surviving chunks with no gap signal | LAYER 1 DONE 2026-06-04 (read() detects + signals `resync_required`, no longer silent; tested) — LAYER 2 pending (propagate to client + re-attach, live-risky) |
| [squish-and-bottom-paint-on-reresume](#squish-and-bottom-paint-on-reresume) | After an update re-resumes a session, codex renders narrow (squish) + composer bg-split (bottom paint) | FIXED 2026-06-05 (v2.8.25) — daemon resizes PTY to client grid on re-attach; deterministic test |
| [seed-connection-state-in-terminal](#seed-connection-state-in-terminal) | yggterm's own launch/connection seed boilerplate ("Launching live … session", "Terminal surface: embedded xterm.js", "Runtime owner: yggterm daemon") is written into the xterm buffer as prefill before the PTY paints | FIXED 2026-06-06 (D4) — local prefill source + render gate reject the daemon launch seed; deterministic fail-then-pass test |
| [detached-term-element-blank-viewport](#detached-term-element-blank-viewport) | Viewport entirely blank while every health field reports healthy: `term.element` is detached from its host and an empty `.xterm` husk (viewport only, no screen) occupies it, defeating all three repair guards | SPECIES A FIXED 2026-07-22 — provenance root-caused: the husk is born in a PARTIAL `term.open()` (root appended first, screen fragment last), pinned by `tools/xterm-harness/husk_is_born_in_a_partial_open.test.js`; mount now retries after discarding the husk and the surface owner rebuilds rather than moves one. SPECIES B OPEN — a fully-opened terminal that later loses its screen has an armed early-return guard, so only a remount helps (reported as `rebuild_from_husk_failed`) |
| [blank-viewport-client-snapshot-poison](#blank-viewport-client-snapshot-poison) | On reveal/switch-back of a cursor-addressed (codex) session, the viewport is clipped from the middle / blank above the bottom rows: the client restores a sparse cached xterm_session_snapshot instead of reconciling the daemon's authoritative screen frame; trips "viewport beyond scrollback base" → blink/reseed/restart | CAPTURE+RESTORE GUARDS SHIPPED (66d765c3); CODEX RECONCILE FIX 2026-06-06 (Bug 1) — reveal reconciles from daemon screen frame before the client snapshot; NEEDS LIVE VERIFY |

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

**STATUS:** OPEN (root cause unknown) — DETECTION + HEAL WIRED 2026-07-10
(`glyph_gap_rows` render fail pattern).

### Symptom
A rectangular region inside an active session viewport renders blank
(theme background color, no glyphs) even though buffer rows exist for
those rows. A forced redraw (resize, focus toggle, scroll) fills it in.
The same detector also catches heavy glyph DROPPING (whole rows painted
empty while neighbours paint — the 2.9.63 jojo screenshot with letters
missing from words is the sub-row variant when it degrades far enough
to blank entire rows).

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
Detection + targeted heal shipped 2026-07-10: `detectAndHealGlyphGapRows`
in the terminal eval script compares buffer text rows against text-layer
ink bands (ONE bulk `getImageData` per scan — a single GPU sync — instead
of hundreds of 1px reads). ≥3 text rows with zero ink while other rows
have ink → traces a `glyph_gap_rows` render anomaly and heals with a
targeted atlas clear + row-range `term.refresh`, latched to one heal per
10s so it can never loop. Scans run at most every 5s, active host only,
and only when the aggregate ink sample is healthy (the fully blank case
is `canvas_blank_with_buffer_text`, handled by render health recovery).
Escape hatches that also work but are expensive: resize, focus toggle.

### Code locations
- `crates/yggterm-shell/src/shell.rs` — `detectAndHealGlyphGapRows` (next
  to `sampleCanvasInk`), called from `updateRenderHealth`.

### Tests
- `terminal_eval_script_wires_glyph_gap_rows_detector` (shell.rs) — guards
  the detector wiring, the scan throttle, the heal latch, and the
  render-health recovery backoff.

### Telemetry
`render_fail_pattern` / `detected` with `anomaly.pattern = "glyph_gap_rows"`
(gap row count + sample, text/inked row counts, heal count). Grouped by
`scripts/render_fail_patterns.py` alongside `stale_atlas_paint`,
`redraw_burst`, and `app_render_storm`.

---

## scrollbar-not-draggable

**STATUS:** FIXED 2026-05-28

### Symptom
The sleek thin scrollbar (added for fast drag-scroll in long sessions)
appears visually but cannot actually be dragged with the mouse — clicks
on the scrollbar track have no effect.

### Reproduction
1. Build a session with enough output to need scrollback (>2 screens).
2. Try to click-and-drag the scrollbar thumb on the right edge of the
   terminal viewport. Before fix: nothing happens.

### Root cause
`stretchXtermRoot()` in `terminal_eval_script` deliberately HID the
scrollbar by:
1. Sizing the `.xterm-viewport` to `calc(100% + gutter)` width.
2. Pulling it left with `marginRight: -gutter`, so the scrollbar slot
   ends up outside the host's `overflow: hidden` clip region.
3. Setting `scrollbar-width: none` inline, overriding our `thin` CSS.

This was correct when the D-pad was the only intended scroll control. It
became a bug once we added the sleek scrollbar CSS — the CSS was being
clobbered by the JS and the scrollbar was clipped off-screen even when
the CSS happened to win.

### Workaround / fix
Removed the gutter compensation in `stretchXtermRoot()`: viewport, screen,
and helpers all stay at natural `100%` width and `marginRight: 0px`.
Inline `scrollbar-width: none` and `-ms-overflow-style` are explicitly
cleared so CSS `scrollbar-width: thin` wins. The scrollbar now lives
inside the host's clip region and is draggable normally.

### Code locations
- `crates/yggterm-shell/src/shell.rs` — `stretchXtermRoot()`, the
  scrollbar CSS block (~line 55811–55938).

### Tests
`terminal_eval_script_scrollbar_is_draggable_not_pushed_off_screen` in
`crates/yggterm-shell/src/shell.rs` asserts the JS no longer sets
`scrollbar-width: none` inline, no longer pushes the viewport off-screen
via negative margin, and clears any stale inline `scrollbar-width` so the
CSS thin scrollbar is authoritative.

### Telemetry
None — fix is structural.

### Related memory
`[[spec-xterm-bug-registry]]`

---

## content-scooped-on-session-switch

**STATUS:** OPEN, telemetry added 2026-05-28

### Symptom
While switching between sessions, content from the middle of the terminal
appears "scooped out" — rows go missing in the middle and the remaining
top and bottom text is presented as one continuous block, hiding the gap.
Effectively the user sees: top-of-buffer + (silent missing rows) +
bottom-of-buffer, joined without any indicator that rows were lost.

### Reproduction
1. Have two sessions A and B with substantial scrollback in each (eg.
   long codex output or shell history).
2. While focused on session A, switch to session B and back.
3. Sometimes the visible scrollback of session A shows continuous text
   that is actually composed of two non-adjacent regions of the original
   buffer joined together; the middle section is missing.

Not yet captured deterministically on a clean install — needs the new
`xterm_resize` telemetry below to confirm whether it correlates with a
host resize during switch.

### Root cause (hypothesis)
Most likely candidate is xterm.js wrapped-line reflow on resize: when
the host is briefly hidden (display switched away) the cached host
dimensions can differ slightly from real ones. On switch back,
`fitTerminalToHost` calls `term.resize(cols, rows)`. If `cols` changed,
xterm.js re-wraps every line in the buffer — wrapped logical lines
collapse into shorter row counts at the new width, which can drop rows
from the middle of buffered scrollback even if the visible text is
preserved at the edges.

Other hypotheses to investigate as telemetry comes in:
- `repaintActiveEntry` `heavy: true` path triggering an erase-in-display.
- `restoreXtermSessionSnapshotOnConstructed` racing with
  `terminal_replay_retained_data_script_for_session` and writing
  partial overlapping content.
- Localised buffer reflow when the cursor cell is on a wrapped line and
  `term.resize` truncates.

### Workaround / fix
Not yet shipped. Investigation gated on telemetry.

### Code locations
- `crates/yggterm-shell/src/shell.rs:emitResize` — telemetry instrumented
  here.
- `crates/yggterm-shell/src/shell.rs:fitTerminalToHost` — calls
  `term.resize`, which is the suspected reflow trigger.
- `crates/yggterm-shell/src/shell.rs:repaintActiveEntry` — heavy redraw
  on session switch, possible secondary contributor.

### Tests
None yet — repro is still uncaptured.

### Telemetry
`xterm_resize` event extended with:
- `prev_cols`, `prev_rows` (dimensions before fit)
- `buffer_length_before`, `buffer_length_after`, `buffer_length_delta`
- `base_y_before`, `base_y_after`
- `viewport_y_before`, `viewport_y_after`
- `suspect_content_scoop` (true when |delta| >= 4 AND cols changed)

When `suspect_content_scoop` fires, an `xterm_content_scoop_suspect`
debug line is also emitted with all of the above inline for easy grep.

### Related memory
`[[spec-xterm-bug-registry]]`, `[[spec-tmux-parity-and-beyond]]`

---

## keepalive-restart-viewport-only

**STATUS:** FIXED & VERIFIED LIVE 2026-05-28 on jojo 2.7.62.

### Live proof (added after misdiagnosis)
I reopened this earlier in the same day claiming two gaps (daemon-handoff resets vt100 ring; GUI prefers daemon_pty over snapshot). Live testing on jojo proved the gaps did not matter for the user's actual concern:

- Test 1: local shell, `for i in {1..300}; do echo line-$i scrollback-test; done` → `base_y: 893`. SIGTERM GUI, `app launch`, `app open` → `base_y: 893` (identical screenshots).
- Test 2: codex resume of avikalpa_opc via fresh shell → `base_y: 144`, conversation rendered. SIGTERM GUI, `app launch`, `app open` → `base_y: 144` (identical screenshots).

The retention path that wins in practice is the GUI-side localStorage scroll-state persistence (commit 5a6e19f) which keeps xterm's buffer + scroll position across GUI restart — the daemon doesn't need to re-serve the history. Combined with the daemon-side `TerminalScreenState::history_and_screen_replay` (commit e69dc0e) for the snapshot path when localStorage is empty, GUI restarts preserve scrollback for both plain shells and TUIs that actually emit content.

The earlier `avikalpa_opc base_y: 0` observation was on a session that hit a stale resume-codex wiring path, not a scrollback fix gap. Running `codex resume <UUID>` in a clean shell prints the conversation (verified live). The right diagnosis would have been to investigate avikalpa_opc's launch path, not refactor the retention machinery.

### Original FIXED claim (correct after all, retained for context)

### Symptom
After the user closes and reopens the GUI (or hot-restarts it), every
keep-alive session shows only the last viewport's worth of content. The
scrollback is empty. tmux/screen with the equivalent history-limit
preserve full scrollback across `tmux attach` cycles — yggterm did not.

### Reproduction
1. Run a long-output session (codex working on a hard task; long `make`;
   `ls -R /` etc) until well over a viewport of output has scrolled.
2. Close yggterm GUI.
3. Reopen yggterm GUI.
4. Before fix: scrollback is gone, only the bottom viewport remains.

### Root cause
The daemon owned a 10 000-row vt100 scrollback ring per session (sized
in `DAEMON_VT_SCROLLBACK_ROWS`), but the replay path
(`screen_snapshot_chunk` in `crates/yggterm-server/src/terminal.rs`)
emitted only `screen.state_formatted()`, which the vt100 crate caps at
the visible viewport (its `visible_rows()` always tops out at
`rows_len`). For TUI sessions (codex, ssh attaches) the daemon's
`read()` path also prefers the screen snapshot over raw chunks, so the
GUI received viewport-only content with no chance to reconstruct
history.

### Workaround / fix
Added `TerminalScreenState::vt_scrollback_plain_rows` and
`::history_and_screen_replay` that walk the vt100 scrollback ring
oldest-to-newest (by stepping `set_scrollback(k)` and grabbing the
topmost row each step — necessary because vt100 doesn't expose the
scrollback iterator publicly), then build a payload of
`{plain history rows joined with \r\n}\x1b[2J\x1b[H{state_formatted}`.
`screen_snapshot_chunk` now serves this composite payload. The GUI
writes it through xterm.js as one chunk: plain history flows into the
xterm scrollback buffer, `\x1b[2J\x1b[H` clears only the visible
viewport (NOT scrollback), then the formatted state repaints the live
viewport with cursor + attrs.

This closes the [[spec-tmux-parity-and-beyond]] history-limit parity
gate.

### Code locations
- `crates/yggterm-server/src/terminal.rs:vt_scrollback_plain_rows`
- `crates/yggterm-server/src/terminal.rs:history_and_screen_replay`
- `crates/yggterm-server/src/terminal.rs:screen_snapshot_chunk` (uses
  the new helper)

### Tests
- `vt_scrollback_returns_empty_when_no_lines_have_scrolled_off`
- `vt_scrollback_returns_scrolled_off_rows_oldest_first`
- `history_and_screen_replay_returns_none_when_terminal_is_empty`
- `history_and_screen_replay_prepends_scrollback_before_clear_and_viewport`

### Telemetry
Existing `terminal_retained_bytes` counters apply. Future telemetry
could add a `scrollback_rows_served` per attach so we can watch for
silent regressions when this path degrades.

### Related memory
`[[spec-tmux-parity-and-beyond]]`, `[[xterm-scrollback-bug]]`

---

## remote-cc-replay-codex-only

**STATUS:** FIX PENDING LIVE VERIFY — TWO LAYERS now addressed in code:
(1) the Rust readiness recognizer (`terminal_chunk_is_claude_prompt_surface`)
landed on `main` (da0e8bd9 / 2.8.1); (2) the GUI-side replay IDEMPOTENCY gap
(`replayVisibleInEntry`/`promptNeedle` were Codex-caret-only) is FIXED
2026-06-06 (campaign D5) — see "Open follow-ups" below. Unit/string-guard tests
pass (fail-then-pass proven); dev/jojo live verification still owed before "fixed"
(cycle a real remote-cc session: confirm transient-flash → settled, not blank).

### Symptom
A resumed Claude Code session (`remote-cc://…`) renders with its prompt
box border / question / option numbers MISSING — only the assistant `●`
bullet and bare option text survive. Forcing a repaint (session
re-open → xterm host remount) collapses the viewport further to just `●`,
fully blank. The full prompt still lives on the daemon/PTY side, so this
is reconstruction loss, not Claude output, and not `blank-rendering-region`
(the xterm BUFFER itself is missing the rows).

### Reproduction
1. Resume a Claude Code session on a remote (live-confirmed on jojo
   2.7.86, `remote-cc://dev/654669a2…`, sitting at a tool-permission
   prompt).
2. Observe the prompt box is not fully reconstructed.
3. `yggterm server app open <same remote-cc path>` to remount the host.
4. Before fix: buffer dump (`app state` →
   `active_terminal_hosts[].buffer_text_sample`) collapses to `●`,
   `data_event_count: 0`; screenshot shows a blank viewport.

### Root cause
The remote retained-replay / resume-surface readiness layer in
`crates/yggterm-shell/src/shell.rs` is **Codex-shaped**. Every "is this a
replayable prompt surface?" gate
(`remote_resume_blank_host_snapshot_is_replayable`,
`remote_resume_snapshot_is_replayable_for_session`,
`terminal_surface_has_prompt_ready_text`,
`retained_remote_surface_has_non_prompt_text`) decides via
`terminal_chunk_is_codex_prompt_surface` (requires the "OpenAI Codex"
header), `terminal_chunk_has_codex_prompt_output` (requires Codex's `›`
marker), or `terminal_chunk_has_prompt_output` (a ≤2-line bare shell
prompt). A Claude permission prompt (box-drawn, `❯` caret, numbered
options, "Tab to amend") matches NONE, so the snapshot is judged
"non-prompt / not replayable" and the blank host stays blank.
`codex_like_session` (shell.rs ~44888) excludes `SessionKind::ClaudeCode`.
There was no Claude-prompt recognizer anywhere — `SessionKind::ClaudeCode`
was made first-class for launch/icons/routing but this readiness layer was
never taught Claude's prompt shape (an SSOT / holistic-spec gap). Present
in `main` too — it is NOT fixed by the 2.7.86→2.8.0 bump.

### Workaround / fix
- New recognizer `terminal_chunk_is_claude_prompt_surface`
  (`crates/yggterm-shell/src/terminal_observe.rs`) keyed on Claude-specific
  markers ("? for shortcuts" idle footer; `❯` caret + a permission
  affordance) — low false-positive against shell/Codex surfaces.
- OR'd into `terminal_surface_has_prompt_ready_text` and the
  `prompt_ready_snapshot` test inside
  `remote_resume_blank_host_snapshot_is_replayable` (shell.rs).
- LIVE-RECOVERY for a stuck session: resize the GUI window (SIGWINCH →
  Claude full repaint via the LIVE write path, which works) — the bug is
  replay-only.

### Code locations
- `crates/yggterm-shell/src/terminal_observe.rs` —
  `terminal_chunk_is_claude_prompt_surface` (definition + test).
- `crates/yggterm-shell/src/shell.rs` —
  `terminal_surface_has_prompt_ready_text`,
  `remote_resume_blank_host_snapshot_is_replayable` (wiring).

### Open follow-ups
- ✅ FIXED 2026-06-06 (campaign D5): GUI-side replay JS
  (`terminal_replay_retained_data_script_for_session`) used Codex's `›` only for
  `replayVisibleInEntry`/`promptNeedle`, so a correctly-replayed Claude buffer
  was judged not-visible → completion never fired → the 100ms retry loop
  `term.reset()+clear()+rewrite` every tick (flash/churn) until the 15s deadline.
  `replayVisibleInEntry` now also recognizes the Claude caret `❯` (U+276F) and
  the `"? for shortcuts"` idle footer, and `promptNeedle` is derived from the
  later of the Codex/Claude carets. (`replayPromptReadyInEntry` already routed
  Claude through `promptViewportReadyInEntry` geometry-only, so visibility was
  the sole remaining gate.) Live-verify on jojo still owed.
- Consider generalizing `codex_like_session` → an agent-CLI-agnostic
  `agent_like_session` per SSOT instead of accreting per-CLI recognizers.

### Tests
`terminal_observe::tests::terminal_chunk_is_claude_prompt_surface_recognizes_claude_surfaces`;
`shell::tests::retained_replay_script_recognizes_claude_caret_not_only_codex`
(fail-then-pass — the generated replay script now recognizes `❯` + `"? for shortcuts"`).

### Related memory
`[[finding-remote-cc-retained-replay-codex-only]]`,
`[[spec-cwd-tree-agent-cli-unified]]`,
`[[spec-agent-cli-wrapper-render-parity]]`, `[[content-scooped-on-session-switch]]`

---

## xterm-pipeline-latency

**STATUS:** MITIGATED (frame budget) — SHIPPED & VERIFIED LIVE on jojo
2026-05-31: active write-frame default 160ms→16ms; live state shows
`terminal_active_write_frame_ms: 16` on remote-cc and codex sessions.
GPU canvas renderer on Wayland was TRIED and DEFERRED (see below).
Relates to `slow-jitter`.

### Symptom
Interactive typing/output in the xterm.js viewport feels markedly laggier
than ghostty or VSCode's xterm.js — lower effective FPS, perceptible echo
delay, especially inside agent-CLI (Claude/Codex) sessions.

### Root cause
Two stacked costs in `crates/yggterm-shell/src/shell.rs`:
1. **DOM renderer forced on.** `terminal_xterm_canvas_renderer_enabled_from_env`
   falls through to `false` in every Linux branch (canvas/WebGL addon is
   bundled but gated off due to a past X11 idle-CPU regression — test
   `xterm_canvas_renderer_is_gated_off_on_x11_idle_cpu_path`). The DOM
   renderer is xterm.js's slowest backend. Live: `canvas_count: 0`.
   VSCode uses WebGL; ghostty is native GPU.
2. **A coarse write-framing layer over xterm's own scheduler.**
   `TerminalWriteBridge` (`terminal_write_bridge.rs`) staged PTY output and
   flushed at `terminal_active_write_frame_ms` = **160ms (~6fps)** when
   focused/active, and the batching LATCHES on for the entire life of any
   alt-screen / cursor-hidden TUI. xterm.js already coalesces writes to the
   display refresh (~60fps), so this second layer only added latency.

### Workaround / fix
- `terminal_active_write_frame_ms` default 160ms → 16ms (one display
  frame). Keeps the protective coalescing that shields the Rust→webview
  `document::eval` bridge from per-frame floods while removing the
  perceptible per-session lag. Tunable via
  `YGGTERM_TERMINAL_ACTIVE_WRITE_FRAME_MS`.
- GPU canvas renderer on Wayland: TRIED 2026-05-31, DEFERRED. It activated
  (`canvas_count: 4`, idle CPU ~1.3% vs 0.0% DOM — X11 idle-CPU fear did NOT
  reproduce on Wayland), but tripped the `canvas_low_contrast_foreground_with_buffer_text`
  render-health heuristic (terminal_observe.rs) and left sessions
  `ready: False` because yggterm's readiness/screenshot/contrast heuristics
  read DOM `.xterm-rows`, which the canvas renderer does not populate.
  Re-enabling is blocked on making those heuristics canvas-aware (trust
  buffer-text + canvas-ink totals when the canvas renderer is active) and
  confirming whether the low-contrast sample is a false positive. Still
  reachable via `YGGTERM_ENABLE_XTERM_CANVAS=1` for that work.

### Code locations
- `crates/yggterm-shell/src/shell.rs` — `terminal_active_write_frame_ms`,
  `terminal_xterm_canvas_renderer_enabled_from_env`.
- `crates/yggterm-shell/src/terminal_write_bridge.rs` — frame latch.

### Tests
Existing relational frame-budget tests
(`terminal_output_read_poll_slows_unfocused_streams_after_resume`, etc.)
remain green with the new default.

### Related memory
`[[finding-xterm-latency-dom-renderer-write-framing]]`,
`[[spec-tmux-parity-and-beyond]]`, `[[spec-xterm-gating-ux]]`

---

## surface-recovery-false-positive-on-transient

STATUS: FIXED 2026-06-03 (settle-gates). Supersedes the "shadow session" reports
under [blank-rendering-region](#blank-rendering-region) and much of
[dom-leak-on-session-start](#dom-leak-on-session-start) on the cold/switch path.

### Symptom
On cold attach AND mid-session (e.g. scroll-up to select text), a remote session
would: flash a blank "shadow" surface, gate for ~2–3s while it "recovered", and
in the worst case **disable keyboard input**, **re-resume the session (interrupting
codex)**, churn rehydrate/DOM cycles, and finally — on recovery-budget exhaustion —
**mark the session Failed and yank it closed**.

### Root cause
A full-screen TUI (codex/agent) clears+redraws constantly (`\x1b[2J\x1b[H`). The
host-health poll could sample the buffer in the one-frame gap *after the clear and
before the repaint*: cursor home, every row blank (diagnostic captured
`cursor_line_len=0, text_tail_len=0, blank_rows_below_cursor=62/63`). Two recovery
predicates were **point-in-time checks with no persistence guard**, so a single
transient sample escalated:
- `retained_ready_remote_empty_surface_should_recover` → empty-surface recovery
  (re-seed churn → shadow flash + re-gate).
- `retained_remote_surface_should_wait_for_prompt_ready` → non-prompt-surface
  recovery, which is far more destructive: disables input → snapshot replay →
  `resume_recovery` (RE-RESUME, interrupts codex) → on exhaustion marks the
  attempt Failed and tears the session down.

### Workaround (fix)
Require the bad condition to **persist across a settle window** before recovering;
the TUI's redraw fills within a frame and resets the timer, so transients never
trigger recovery, while a genuinely broken surface persists and still self-heals.
- Empty-surface: `RETAINED_EMPTY_SURFACE_SETTLE_MS = 800ms`.
- Non-prompt-surface: `RETAINED_NON_PROMPT_SETTLE_MS = 1500ms` (longer — the
  escalation is destructive), PLUS: never re-resume a connected live session on a
  transient, and on budget exhaustion **never fail/close a session whose daemon
  still owns the PTY** (accept the surface as-is, keep it alive + input enabled;
  only a provably-dead runtime is failed).

### Code locations
- `crates/yggterm-shell/src/shell.rs`: `RETAINED_EMPTY_SURFACE_SETTLE_MS`,
  `RETAINED_NON_PROMPT_SETTLE_MS`; the host-health poll settle gates
  (`retained_empty_surface_settle_wait` / `retained_non_prompt_surface_settle_wait`);
  `rearm_stale_retained_fault_recovery` (keep-alive-on-exhaustion branch gated on
  `daemon_owns_session_runtime`).

### Telemetry
`retained_empty_surface_settle_wait`, `retained_non_prompt_surface_settle_wait`
(deferred), vs `retained_empty_surface_recovery_begin` /
`retained_non_prompt_surface_recovery_begin` (fired). The diag fields
`diag_cursor_line_len` / `diag_text_tail_len` / `blank_rows_below_cursor` on
`retained_empty_surface_recovery_begin` were what pinned the transient.

### Tests
`retained_fault_recovery_exhausts_remount_budget_instead_of_spinning` (updated to
the keep-alive-on-exhaustion spec) + the empty-surface/non-prompt suite.

### Second cause (2026-06-03): low-confidence read of a non-foreground host
The settle-gates fix the *transient* (clear+redraw frame-gap) cause. A second,
independent cause is a **buffer read taken while the host is not the foreground
input owner**. When a session is not the focused host, `set_input_enabled(false)`
captures the xterm snapshot on blur (`captureSessionXtermSnapshot('input_disabled')`,
reason renamed conceptually to "focus released") and `term.buffer.active`/
`translateToString` can read back **empty or a single sparse row** even though the
canvas is painting live content the user sees and uses. The app-control surface
detector then classified that sparse read as a definite problem — observed live as
`active terminal host exists but xterm surface is empty` (and, on partial reads,
`...only showing a plain shell prompt`), driving empty-surface fault-recovery on a
perfectly healthy session. This is the *false-positive illusion*: the instrument,
not the session, was broken. **A field name made it worse twice** — the old
`input_enabled` read as "user can type" but actually meant "this host currently holds
input focus/stdin"; both the probe and the detector conflated focus-ownership with
health. That flag has since been **renamed** (see "Rename" below) so it can't be
misread again.

Fix: a **couldn't-observe guard** in `terminal_host_problem_for_app_control`
(`crates/yggterm-shell/src/terminal_observe.rs`) — abstain (return `None`) when ALL of:
- the read is low-confidence — the host does **not** hold input focus and the
  window is **not** focused (`!raw_input_enabled && !helper_textarea_focused &&
  !host_has_active_element && !document_focused`); a snapshot captured on blur reads
  back empty/sparse/placeholder even while the canvas paints live;
- a **live daemon paint frame is present** for this surface
  (`!last_raw_payload_sample.is_empty()` with `canvas_count > 0 || render_event_count
  > 0`) — the decisive "healthy, just-not-cleanly-readable" signal. A genuinely
  stuck/stale surface (codex never reached a prompt, retained prose, gated tail)
  has buffered/retained text but **no** current live paint frame and is still flagged;
- it is **not** a transport/error string (those are unambiguous and surface regardless
  of focus).

Genuine geometry/transparency/paint faults are checked before this guard and still
surface; the guard strictly *reduces* spurious recovery. When the user actually
focuses the window (`document_focused`), the read is trusted again and real problems
re-surface. Test: `terminal_host_problem_abstains_on_sparse_read_of_unfocused_rendered_daemon_fed_surface`
(asserts both directions: abstains unfocused, still diagnoses when focused).

**Rename (done):** the misleading `input_enabled` was split into two accurately-named
app-control fields and a clearer snapshot reason:
- per-host `terminal_hosts[].input_enabled` → **`host_stdin_enabled`** ("this host is
  the active input target / xterm stdin is enabled"; mirrors `term.disableStdin`);
- summary `active_terminal_surface.input_enabled` (the aggregate
  `raw_input_enabled && effective_input_focus && problem.is_none()`) →
  **`foreground_input_ready`** ("the foreground surface holds focus and is healthy");
- `xterm_session_snapshot_reason: "input_disabled"` → **`"focus_released"`** (captured
  on blur).
`raw_input_enabled` / `effective_input_focus` keep their (already-accurate) names. The
internal JS variable `inputEnabled` is intentionally NOT renamed (no compile check on
the generated string template; the *emitted key* is what was surfaced and confusing).

Still open: the client buffer read should be made reliable for non-foreground hosts
(or tagged with a confidence the detector honors). Drive sessions with `server app terminal send` (direct PTY write),
not `probe-type` (a JS-keypress diagnostic whose `visible_echo_missing` does not mean
input is unsendable).

### Related memory
`[[finding-hot-switch-latency-remount]]`, `[[audit-viewport-scroll-control-flow]]`

---

## persisted-scroll-restore-fights-follow

STATUS: FIXED 2026-06-02.

### Symptom
After a GUI restart, returning to a session dropped the viewport at a "random"
saved scroll offset, then every click/keystroke flickered between that offset and
the live bottom for ~8s.

### Root cause
On GUI restart a session arms `pendingPersistedScrollRestore` (saved offset from
localStorage) and polls up to 8s to apply it. During that window the prompt-follow
cascade forced the viewport to the bottom while the pending restore re-applied the
offset — no coordination, so they fought on every interaction.

### Workaround (fix)
In `tryApplyPendingPersistedScrollRestore`, abandon the pending restore the moment
`scrollbackIntent !== 'UserScrollback'` (the user engaged the prompt — typed,
pasted, scrolled to bottom — or live output arrived). A passive restored session
stays in `UserScrollback`, so position-restore still works for the case it was
designed for.

### Code locations
`crates/yggterm-shell/src/shell.rs::tryApplyPendingPersistedScrollRestore`.

### Related memory
`[[audit-viewport-scroll-control-flow]]`

NOTE: a *separate*, still-OPEN viewport bug remains — left-click jumping the
viewport to a random scrollback position + scroll-down-goes-up — which is the
"no single owner of viewport position" issue tracked in
`[[audit-viewport-scroll-control-flow]]` (needs the consolidated
FOLLOWING/PINNED/SELECTING controller). It is NOT fixed by this entry.

---

## xterm-host-registry-leak

STATUS: FIXED 2026-06-02.

### Symptom
The app got gradually laggier with use — selection, paste, and session switching
slowed over time. Restarting/switching sessions was the trigger.

### Root cause
A host's cleanup is registered in `__yggtermXtermCleanups[hostId]` and only runs
when that EXACT hostId re-initializes. But hostId embeds the mount epoch (`-m<N>`);
every restart/switch bumps the epoch → a new hostId → the prior epoch's entry is
abandoned, its cleanup never runs, and its xterm.js Terminal (buffer + renderer +
listeners) leaks. The registry grew unbounded (measured 5→20+). Every global pass
over the registry (selection/paste/switch) then got slower.

### Workaround (fix)
On (re)mount of a session path, reap any OTHER registry entry for the SAME path
whose DOM host element is gone (a dead prior epoch) — dispose its Terminal and
delete its registry+cleanup entries. Other paths' warm-retained entries are
untouched. A `dom_census` field was added to the `terminal probe-scroll` snapshot
to measure this class.

### Code locations
`crates/yggterm-shell/src/shell.rs` host-init reaper (near the
`__yggtermXtermCleanups` re-init), `dom_census` in the probe-scroll snapshot.

### Related memory
`[[finding-hot-switch-latency-remount]]`

---

## chunk-ring-trim-drops-mid-stream

STATUS: OPEN — found by the xterm.js ← yggterm-server pipeline audit (2026-06-03).
This is a **yggterm-server (daemon) protocol** bug, not an xterm.js bug, but it
manifests as the user-visible symptom "TUI content clipped / chunks in the middle
absent" so it is registered here.

### Symptom
While working a remote session (codex/agent or shell), chunks of output in the
MIDDLE are silently absent — confirmed via codex's Ctrl+T transcript. Often paired
with the viewport jumping up during output (a separate scroll-controller issue,
see `[[audit-viewport-scroll-control-flow]]`).

### Data path being audited
codex/CC/shell (remote host) → **yggterm server** reads PTY into a per-session
chunk ring (each chunk has a monotonic `seq`) → SSH → client (`yggterm` GUI or
`yggterm-headless`) read-bridge → xterm.js. (Nomenclature: yggterm = GUI client,
yggterm-headless = headless client for agents, yggterm server = the session-holding
daemon.)

### Root cause
`terminal.rs::PtySessionRuntime::read(cursor)` for an incremental read does:
`chunks.iter().filter(|c| c.seq > cursor).collect()`. The chunk ring is trimmed
(oldest evicted) by `trim_chunk_buffer` under the live cap (`MAX_BUFFER_BYTES` =
16 MB) and, more aggressively, by idle-trim (`IDLE_TRIM_MAX_BYTES` = 128 KB). If
the ring is trimmed while a client's `cursor` is BEHIND the new oldest `seq`
(e.g. the client switched away / disconnected, the session idle-trimmed, then the
client resumes from its stale cursor), the evicted chunks between `cursor` and the
oldest survivor are **silently dropped** — `read` returns only the surviving tail
and `TerminalReadResult` has **no gap/reset field** to signal it. The client
applies a tail that begins mid-stream → xterm state diverges → missing/garbled
middle rows. Ruled out as NOT the cause this round: GUI transport
(`transport_leak_dropped_write_count = 0`), frame coalescing
(`coalesce_high_volume_terminal_frames` is a no-op). Note: a full-screen TUI in
the alternate buffer (`base_y = 0`) legitimately has no scrollback — that part is
by design, not this bug.

### Fix — layer 1 DONE (2026-06-04): detect + signal at the source of truth
Why signal-and-re-attach instead of the 2.8.12/14 in-band replay: those re-sent
history+screen inside `read()` and corrupted alternate-screen TUIs (normal-buffer
history written into the alt screen) and cleared normal-buffer scrollback (`\x1b[3J`),
and fired during recovery churn — the TRAP (see
`[[incident-gap-fix-cascade-2026-06-03]]`). Detecting the gap and letting the client
recover via its EXISTING re-attach path (which is already alt-screen-correct and reads
the full vt100 scrollback, `DAEMON_VT_SCROLLBACK_ROWS` = 10 000) avoids all three.

`PtySessionRuntime::read(cursor)` now computes
`resync_required = effective_cursor > 0 && oldest_retained.seq > effective_cursor + 1`
and returns it on `TerminalReadResult`. So the gap is no longer SILENT at the source.
Covered by `tests/pipeline_integration.rs::read_from_cursor_never_silently_drops_trimmed_middle_chunks`
(drives the ring past `MAX_CHUNKS` with a client behind the trim; asserts contiguous
OR `resync_required`). No runtime behavior change yet — the flag is not consumed
downstream (see layer 2).

### Fix — layer 2a DONE (carry + observe): `resync_required` threaded end-to-end
The flag now propagates through the whole read path with NO behavior change:
`ServerResponse::TerminalStream` carries it (`#[serde(default)]` → cross-version safe);
the daemon request handler sets it from `stream.resync_required` (and forwards it on
the preserved-owner path); `terminal_read` / `terminal_read_with_local_daemon_recovery`
(lib.rs) and `terminal_read_async` (shell.rs) extend their tuples; the GUI read-bridge
loop binds it and emits a `terminal_stream_resync_required` trace event when a gap
fires. So we can now OBSERVE real-world gaps (event-trace) without changing behavior.

### Fix — layer 2b PENDING: client re-attach action (the live-risky part)
The only remaining step: in the GUI read-bridge, when `resync_required` is set, trigger
a clean re-attach (read from cursor 0 / fresh screen snapshot) to recover the trimmed
middle from the vt100 scrollback, instead of appending the discontiguous tail. This
changes runtime behavior and must be live-verified against the real repro (a
BACKGROUNDED session streaming past the ring cap, then switch-back) before shipping —
do NOT rush it (the in-band-replay variants of this were the 2.8.12/14 TRAP). Longer
term this folds into real-scrollback retention (`[[spec-tmux-parity-and-beyond]]`) so
the ring/idle-trim never evicts live content a connected client still needs.

### Code locations
`crates/yggterm-server/src/terminal.rs`: `PtySessionRuntime::read` (gap detection +
`resync_required`), `trim_chunk_buffer`, `MAX_BUFFER_BYTES`, `IDLE_TRIM_MAX_BYTES`,
`TerminalReadResult.resync_required`. Layer 2: `lib.rs`
`terminal_read_with_local_daemon_recovery` + the bridge stream loop; `shell.rs`
read-bridge.

### Related memory
`[[spec-tmux-parity-and-beyond]]`, `[[finding-hot-switch-latency-remount]]`

---

## squish-and-bottom-paint-on-reresume

**STATUS:** FIXED 2026-06-05 (v2.8.25)

### Symptom
After a yggterm update/daemon-restart re-resumes a session, codex renders narrow
(~120 cols) inside the real viewport (squish), with blank right margin, and the
composer input row shows a dark terminal-bg patch after the cursor (bg-split).
Recurred on every update.

### Reproduction
mock-tui / pipeline_integration: create a PTY at 120x36, then call
`ensure_session_with_size` again with 159x63 — the session stays 120x36
(`ensure_session_keeps_existing_grid_so_reattach_must_resize_to_client_grid`).
Live: any daemon restart that re-resumes a remote codex session.

### Root cause
The successor daemon auto-resumes a restored session at the DEFAULT 120x36 grid.
`TerminalManager::ensure_session_with_size` only applies the requested grid when
it CREATES the PTY, so the later client (re)attach that carries the client's real
grid was a no-op against the already-running PTY → codex repaints at 120 cols and
the partial reflow drops composer cell bg. The squish is the ROOT of the bg-split.

### Workaround / fix
Daemon `ensure_terminal_for_path_with_initial_size_and_seed` (daemon.rs, after the
`ensure_session_with_size` call) now resizes the PTY to the client's grid when the
client provides one that differs from the running session — codex repaints
full-width and clean. Best-effort; traces `reattach_grid_resync`.

### Code locations
- crates/yggterm-server/src/daemon.rs — reattach grid-resync after ensure_session.
- crates/yggterm-server/src/terminal.rs:879 restart_session_with_size (in-process grid preserve).

### Tests
- crates/yggterm-server/tests/pipeline_integration.rs:
  `ensure_session_keeps_existing_grid_so_reattach_must_resize_to_client_grid`,
  `restart_preserves_session_grid_instead_of_default`.

### Related
- memory: campaign-xterm-dealbreakers (D1), finding-codex-squish-post-restart-pty-size,
  finding-codex-composer-bg-split-reflow, finding-blank-on-restart-split-brain-daemon.

## seed-connection-state-in-terminal

**STATUS:** FIXED 2026-06-06 (campaign D4) — client-side; structural SSOT follow-up noted below.

### Symptom
During the pre-first-paint launch window the terminal viewport shows yggterm's OWN
connection-state / launch seed text instead of program output — e.g. `$ codex`,
`Launching live Codex session <id>`, `Workspace: <cwd>`, `Deploy state: …`,
`Launch phase: …`, `Terminal surface: embedded xterm.js`, or the daemon-owned
variants (`Resume daemon-owned …`, `Runtime owner: yggterm daemon`, `Transport
bridge: stdio attach to daemon PTY`). When the real stream stalls/strands, the
seed persists in-buffer (seen as the lingering line during the split-brain blank).

### Reproduction
Rust unit (`local_terminal_prefill_text_rejects_daemon_launch_seed_lines`): build a
`ManagedSessionView { source: LiveLocal, … }` whose `terminal_lines` hold the
`build_live_terminal_lines` seed (PTY not yet painted) → before the fix
`local_terminal_prefill_text` returns `Some(seed)` and `terminal_prefill_should_render_to_host`
returns `true`, so the seed is written to the xterm buffer.

### Root cause
The daemon overloads `session.terminal_lines` as BOTH the launch/connection seed AND
the PTY screen tail. It only overwrites `terminal_lines` with the real PTY tail once
`session_screen_snapshot()` is `Some` (daemon.rs); before first paint it holds the
boilerplate from `build_live_terminal_lines` (lib.rs:19126) or the `terminal_lines =
vec![..]` seed sites. On the client, `local_terminal_prefill_text` blindly maps
`session.terminal_lines` → prefill text, and the prefill is written as a real
`TerminalJsCommand::Write` (shell.rs:48547). The render gate `terminal_resume_output_excerpt`
did not recognize the seed phrases, so the metadata leaked into the buffer.
(The REMOTE summary/preview leak is already dead — `remote_terminal_placeholder_text`
and the `*_prompt_ready_replay_text` fns are neutered to `None`; the bootstrap-warning
"assets host=…"/"Connecting SSH" strings are DOM chrome / toasts, NOT buffer writes.)

### Workaround / fix
`terminal_chunk_is_daemon_launch_seed(&str)` recognizes high-specificity
yggterm-internal seed markers (phrases a CLI never emits). `local_terminal_prefill_text`
returns `None` when any `terminal_lines` entry matches it (primary fix at the source),
and `terminal_resume_output_excerpt`'s reject block also drops it (defense in depth).
**Preferred SSOT follow-up (not yet done, larger blast radius):** stop overloading
`session.terminal_lines` — route launch/connection seeds to a non-terminal overlay
field so `terminal_lines` only ever carries real PTY content; that makes the leak
impossible at the source rather than filtered client-side.

### Code locations
- `crates/yggterm-shell/src/shell.rs` — `terminal_chunk_is_daemon_launch_seed` (predicate),
  `local_terminal_prefill_text` (primary guard), `terminal_resume_output_excerpt` (defense in depth).
- `crates/yggterm-server/src/lib.rs:19126` `build_live_terminal_lines` + the `terminal_lines = vec![..]`
  seed sites — the source of the boilerplate (SSOT follow-up target).

### Tests
- `crates/yggterm-shell/src/shell.rs`:
  `local_terminal_prefill_text_rejects_daemon_launch_seed_lines` (fail-then-pass, proven to bite),
  `terminal_chunk_is_daemon_launch_seed_matches_seed_not_real_cli_output` (no false positives on agent text),
  `local_terminal_prefill_text_uses_hot_session_terminal_lines_and_status` (real CLI tail still prefills — kept green).

### Related
- memory: campaign-xterm-dealbreakers (D4), finding-blank-on-restart-split-brain-daemon.

## blank-viewport-client-snapshot-poison

**STATUS:** Capture-side + restore-side poison guards SHIPPED (66d765c3, 2026-06-04).
CODEX RECONCILE FIX 2026-06-06 (campaign Bug 1) — reveal reconciles from the daemon
authoritative screen frame before the client snapshot. **NEEDS LIVE VERIFY on jojo**
(the test is a generated-script string+order assertion, not a behavioral xterm.js run;
a `tools/xterm-harness` behavioral guard is the follow-up hardening).

### Symptom
On reveal — going away and back, or switching sessions and back — a cursor-addressed
agent session (codex/CC) renders clipped from the middle: only the bottom few rows
paint, the region above is blank, and the daemon holds the full frame. Sometimes it
"blinks" and reseeds, or yggterm escalates to restarting the session. Non-deterministic
(a later reveal may heal it).

### Reproduction
Live (jojo 2.8.25): foreground a working codex session, switch away and back a few
times. App state on the stuck host: `retained_replay_source = xterm_session_snapshot`,
`xterm_session_snapshot_nonblank_line_count` small (e.g. 4–7),
`retained_replay_promoted_to_daemon_pty_count = 0`, while `server snapshot` shows the
full CUP frame. `server trace tail` describe_state: `surface_problem = "active terminal
viewport is beyond the xterm scrollback base"`, `viewport_ready = false`,
`render_health = healthy` (viewport-readiness detector, not the renderer).
Deterministic: `shell::tests::retained_replay_reconciles_from_daemon_screen_before_client_snapshot`.

### Root cause
In `attemptReplay` (the generated `terminal_replay_retained_data_script_for_session`),
the cursor-addressed collapsed-recovery branch
(`collapsedScrollbackNeedsReplay && retainedReplayCursorAddressedScrollbackRisk()`)
preferred the cached client `xterm_session_snapshot` and, failing that, just
`retryLater()`. It never wrote the daemon's authoritative screen frame in this branch.
The client snapshot can be a collapsed/sparse frame (nonblank far below the real
screen) that the `<=1`-nonblank poison guard does not catch — so the sparse frame is
latched, the viewport sits past the buffer content, and `terminal_observe.rs`
(`viewport_beyond_xterm_base`) raises the surface problem that drives blink/reseed/restart.
NOTE: the poison guard cannot be tightened with a fraction-of-`priorMax` rule —
`priorMax` is monotonic (`Math.max`), so a legitimately-shorter frame after a big turn
would be poisoned forever. The fix is at the source choice, not the guard threshold.

### Workaround / fix
On a cursor-addressed collapsed-recovery reveal, when the reveal carries an
AUTHORITATIVE daemon screen frame (`replaySource` is `daemon_screen_snapshot` or
`daemon_retained_history_screen_snapshot`), write it (`writePayloadIntoEntry(entry, data)`)
and promote it BEFORE falling back to the client snapshot. A daemon screen snapshot is
the real current frame (not cursor-addressed scrollback history, which stays risky to
replay), is safe to write, and self-corrects on the next codex repaint. Capture/restore
poison guards remain as defense in depth.
Escalation gate (DONE 2026-06-06): `terminal_host_problem_for_app_control` no longer
escalates a transient `viewport_beyond_xterm_base` for a cursor-addressed codex surface
with a small `base_y` (codex owns its scrollback) — it stays OBSERVABLE via the
`geometry_problem` field but does not feed the clean-output/recovery path, so a reseed
artifact can't drive a remount/restart of a working session. A real-scrollback session
(`base_y` ≫ rows) with the same reading still escalates.
TODO (not yet done): fix the transient `active_session_path` set without an
`active_session` on switch-back.

### Code locations
- `crates/yggterm-shell/src/shell.rs` — `attemptReplay` cursor-addressed branch
  (reconcile-from-daemon-screen), `xtermSessionSnapshotIsCollapsedPoison` (capture/restore/replay
  copies), `writePayloadIntoEntry`.
- `crates/yggterm-shell/src/terminal_observe.rs` — `viewport_beyond_xterm_base` surface-problem detector.

### Tests
- `crates/yggterm-shell/src/shell.rs`:
  `retained_replay_reconciles_from_daemon_screen_before_client_snapshot` (fail-then-pass:
  strings absent on HEAD; asserts the reconcile branch exists AND precedes the client-snapshot fallback);
  plus the existing capture/restore poison-guard string tests.
- `crates/yggterm-shell/src/terminal_observe.rs`:
  `host_problem_suppresses_transient_viewport_beyond_base_for_codex_reseed` (the geometry
  detector still observes it; the host-problem consumer suppresses it for a low-base_y codex
  surface but still escalates a real-scrollback session) — and the existing
  `app_control_terminal_surface_flags_viewport_beyond_xterm_base` stays green.

### Telemetry
`retained_replay_reconcile_from_daemon_screen` host-health event; `retained_replay_source`,
`retained_replay_promoted_to_daemon_pty_count`, `xterm_session_snapshot_nonblank_line_count` in `app state`.

### Related memory
`[[finding-blank-viewport-client-snapshot-poison]]`, `[[campaign-xterm-dealbreakers]]` (Bug 1, D3/D4),
`[[finding-blank-on-restart-split-brain-daemon]]`, `[[finding-codex-scroll-lock-no-client-scrollback]]`.

## cc-alt-screen-no-scrollback

**STATUS:** HISTORICAL (expected behavior by spec decision, 2026-06-11)

### Symptom
A Claude Code session's xterm buffer holds only one viewport: scrolling up
shows nothing (no scrollbar), and content that scrolled past appears "lost".
After some bg→fg cycles CC itself handles scrolling (vim-like pager).

### Reproduction
1. Open any Claude Code session (local or remote) in yggterm.
2. Let CC print more than one screen of conversation.
3. Try to scroll the terminal viewport up.

### Root cause
NOT a yggterm bug. Claude Code renders on the ALTERNATE screen, and
alt-screen content never accumulates xterm scrollback — by terminal
semantics, in every terminal. The wrapper-vs-manual parity rule decides
this: a manual `ssh -t <machine> claude -r <uuid>` in ghostty/xterm behaves
identically (no terminal scrollback; scrolled-away content lives only in
CC's own pager, reachable through CC's scroll keys). Wheel events over an
alt-screen app are converted by xterm.js to arrow keys (standard behavior),
so CC's own pager receives them.

### Workaround / fix
None to build in the terminal path. Spec decision per the wrapper-vs-manual
parity rule: yggterm must render exactly what the manual invocation renders;
"fixing" this would mean parsing CC's JSONL into the viewport, which yggterm
never does. Full scrollable history of a CC conversation is the WEB VIEW's
job (the JSONL pretty-formatting surface).

### Code locations
- None (behavioral spec entry).

### Tests
- None (no code path to lock); parity is the invariant.

### Related memory
`[[spec-agent-cli-wrapper-render-parity]]`, `[[run8-findings-and-user-bug-batch]]` (user bug 8),
`[[finding-codex-owns-scrollback-not-term-program]]` (same class: the CLI owns its history).

## remote-cc-pty-never-resized

**STATUS:** FIXED (code landed; activates on the next daemon changeover)

### Symptom
A remote **Claude Code** session renders a frame built for a *smaller screen*: the
composer and the `bypass permissions on` status line float in the MIDDLE of the
viewport, with a band of dead rows below them and a hard content edge short of
the right margin. Every CC spinner tick repaints that undersized frame, so the
screen churns in the wrong place and the bottom never fills — what the user
called "this rendering dance." It cannot be fixed from the client, and no
client-side fix ever helped, because **nothing is wrong on the client**: it is
faithfully rendering bytes that Claude Code authored for a grid it genuinely has.

Live-caught on jojo (2026-07-11): client viewport **167x63**, `claude` PTY on dev
pinned at **147x50** — CC emitting `\x1b[50;1H` for its composer because, to it,
row 50 *was* the bottom.

### Reproduction
1. Open a `remote-cc://` session while the terminal viewport is grid **A**.
2. Change the viewport to grid **B** (resize the window, toggle a panel/sidebar).
3. Re-open the session. It renders at grid **A** forever.

Only a session *born* at the wrong grid shows this — one spawned while the
viewport already was B is correct, which is why it looks intermittent.

### Root cause
The remote PTY's size has exactly ONE writer — `forward_remote_pty_resize` — and
that writer was **Codex-only in two independent places**:

1. It resolved its target via `remote_shutdown_target_for_path`, which parses
   `remote-session://` only. A `remote-cc://` path returned `None`, so the
   function returned before doing anything.
2. Its delivery fn hardcoded `remote_runtime_codex_session_key(id)`
   (`codex-runtime://…`). A CC runtime is keyed `cc-runtime://…`, so even when
   reached it asked the remote daemon to resize a key that does not exist
   (`terminal session not found`) — and the caller discarded that error
   (`let _ = …`), making the failure completely silent.

So a Claude Code session had **no writer of its remote PTY size after birth.**
The size was set once at spawn and never again.

This was invisible to every daemon-side instrument: the daemon resizes its own
vt100 mirror and its local ssh PTY correctly, so `pty_rows`/`pty_cols`,
`session_view_contract_violations`, and the daemon screen all report the client's
real grid and call the session healthy. The only place the truth lives is the
`claude` process's actual PTY on the remote host (CLAUDE.md misstep #3: the
daemon screen is not what the CLI painted).

### Workaround / fix
Immediate, no deploy: `TIOCSWINSZ` the remote `claude` PTY to the client grid —
CC SIGWINCH-repaints itself and the frame is correct instantly.

Real fix: make the single writer kind-aware.
- `remote_agent_pty_target_for_path` resolves BOTH remote agent schemes (the
  scheme alone implies the kind, per `session_path_is_remote_agent`). The Codex
  branch delegates to the old resolver, so codex behavior is byte-identical.
- `resize_remote_agent_session_pty` builds the runtime key from `SessionKind`.
- The forward failure is now traced (`remote_pty_resize_failed`) instead of
  swallowed.

No second writer is introduced — the size-war lesson still holds.

### Code locations
- `crates/yggterm-server/src/lib.rs` — `remote_agent_pty_target_for_path`,
  `resize_remote_agent_session_pty`
- `crates/yggterm-server/src/daemon.rs` — `forward_remote_pty_resize`

### Tests
- `remote_cc_session_pty_has_a_resize_target_and_uses_the_cc_runtime_key`
  (`yggterm-server`) — asserts a `remote-cc://` path resolves to a resize target
  with `SessionKind::ClaudeCode` + the `cc-runtime://` key, and pins that the
  Codex-only resolver cannot see a CC path.
- `ensure_session_keeps_existing_grid_so_reattach_must_resize_to_client_grid`
  (pipeline integration) — already encoded this invariant; the remote forward
  just never implemented it for CC.

### Telemetry
- `remote_pty_resize_forwarded` (daemon/terminal_resize) — now carries `kind`.
  **Its total absence for a machine means no remote PTY has ever been resized.**
- `remote_pty_resize_failed` (daemon/terminal_resize) — NEW; the forward failing
  used to be silent.

### Related memory
`[[campaign-render-pipeline-parity-rework]]` (this is that campaign's thesis
confirmed: `remote` is the NAMED concept and everything else is the unnamed
fallback — here the axis is Codex-vs-CC), `[[campaign-telemetry-infinite]]`,
`[[spec-unify-local-remote]]` (drive from `SessionKind`, not a URL prefix).

## detached-term-element-blank-viewport

**STATUS:** OPEN — but the repair half is now FIXED and proven. 2026-07-22
(later): the reason no repair path ever healed this was found and closed —
`term.open()` is a **no-op** on an already-opened terminal, so every
"wipe the host, then re-open" recovery in `shell.rs` rebuilt nothing. Fixed by
one owner, `attachTerminalSurfaceToHost`, with a deterministic harness guard.
**Still open:** what creates the husk in the first place.

### Symptom

The terminal viewport is entirely blank — background colour only, no glyphs, no
cursor — while the sidebar, metadata rail and the rest of the app render
normally. The session is genuinely alive: the daemon holds the correct screen,
the agent keeps working, and reattaching from a shell shows real content. Every
health field says the session is fine.

### Reproduction

Not yet captured deterministically. Observed live 2026-07-22 on the desktop
host: a Claude Code session remounted (mount epoch 2), went blank at mount, and
stayed blank for 16 minutes across two `window_foreground` repaint cycles until
the user noticed. In the trace generation covering that day: **14 healthy mounts
vs 1 broken** — roughly a 7% mount race.

### Root cause

Two independent layers.

**1. The state.** `term.element` is DETACHED from the DOM — `isConnected ===
false`, rect 0×0 — while still holding its canvases and a full live buffer that
keeps receiving daemon writes. What occupies the host div instead is an empty
**husk**: `div.terminal.xterm` containing only `.xterm-viewport` — no
`.xterm-screen`, no `.xterm-rows`, no canvas. Document-wide `.xterm-rows` count
= 0 and canvas count = 0. Nothing on the page can paint a glyph.

The signature in the trace is a 20 ms "renderer flip" that is not a flip at all:

```
renderer_decision phase=init        actual=gpu_canvas  canvas_elements=3
renderer_decision phase=after_paint actual=dom         canvas_elements=0
```

`renderer_decision` counts canvases *under the host*; by `after_paint` the term
had already left. All four `xterm_first_paint_sample` probes (t0/t16/t64/t256)
read `len=0 text=""` — it never painted once.

**2. Why it never self-heals** (predicates evaluated against the live broken
DOM, all three false):

| `rebindCurrentHost` reopen guard | value | why |
|---|---|---|
| `hostMissingXtermRoot` | false | the husk matches `.xterm` |
| `hostMissingRenderableLayer` | false | the check *requires* `.xterm-screen`; the husk has none, so it short-circuits |
| `termElementDisconnected && !host.querySelector('.xterm')` | false | the husk matches `.xterm` |

→ `sameHostNeedsReopen === false`, so the repair early-returns and never
re-appends. The husk is exactly the shape the guards do not model: an `.xterm`
root **with no screen**. Separately `ensureVisibleHost` returns at its first
branch because `emitPaint()` reports success (`paintCount` 43,
`lastVisiblePaint` true) — `visible` is satisfied by any child in the host,
including the husk — so the `host.innerHTML=""` + `term.open(host)` rebuild
never runs.

**Why telemetry scored it healthy.** Every health field read the xterm *object*,
never the DOM: `render_health_status: "healthy"` with
`render_health_ink_sample: {sampled_pixels: 0, canvas_count: 0}` — a zero sample
means "could not measure", but was scored as healthy;
`renderer_surface_missing: false`; `dom_paint_hit_test_problem: ""`;
`cursor_line_text`, `text_tail`, `launch_phase: Running` and the daemon's
`terminal_lines` all correct. The truth was already in app state and unused:
`rows_present: false`, `rows_rect: null`, `screen_present: false`,
`canvas_count: 0`.

**3. Why no repair path could ever heal it — PROVEN 2026-07-22 (later).**
Even once the guards were widened, every recovery still failed, because all of
them were built on an assumption that is false in the vendored xterm.js:

```js
open(e) { if (!e) throw ...;
  if (e.isConnected || this._logService.debug(...),
      this.element?.ownerDocument.defaultView && this._coreBrowserService)
    return void (...)          // ← EARLY RETURN. No appendChild(e).
```

**`term.open(parent)` does nothing at all once `term.element` exists** — it does
not re-parent, does not rebuild, does not throw. So `host.innerHTML = ""`
followed by `term.open(host)` is pure loss: an empty host and a stranded
`term.element`. `ensureVisibleHost`'s `rebuild_blank_host` — the *last-resort*
recovery, capped at one attempt per mount — was exactly this shape, so the one
path meant to rescue a blank viewport was guaranteed to make it permanent.
(It happens never to have fired in 20 trace generations, so it was a latent
landmine rather than the observed cause; `rebindCurrentHost`'s conditional
re-append is what saved most cases.)

Proven deterministically against the shipped bundle, not inferred:
`tools/xterm-harness/host_reopen_is_a_noop.test.js`.

### NOT determined — open questions for the next agent

1. **~~Which code path leaves the husk?~~ ANSWERED 2026-07-22 — a PARTIAL
   `term.open()`.** No longer a lead: it is pinned deterministically against the
   shipped bundle by `tools/xterm-harness/husk_is_born_in_a_partial_open.test.js`.
   `open()` appends the bare root to the parent FIRST and the viewport/screen
   fragment LAST, so any throw inside that window leaves exactly the husk —
   `xterm_roots=1 screen_in_host=false rows_in_host=false screen_canvases=0`,
   with `term.element` set AND connected, which is why every placement guard read
   it as healthy. The mount's `term.open(host)` was unguarded, so the same throw
   also abandoned the rest of the mount — matching the "born at mount, in one
   millisecond" autopsy exactly.
   **The orphan roots are a consequence, not a separate mystery.** `open()`'s
   early-return guard is `this.element && this._coreBrowserService`, and
   `_coreBrowserService` is assigned late *inside* `open()`. A partial open sets
   `element` without arming the guard, so the next `open()` falls through and
   builds a SECOND root while the husk stays parented — an orphan whose owner no
   longer matches. This explains the 18/18 `constructed ≥2×` correlation without
   needing two live closures for one `hostId`.
   **Fixed** by `terminalSurfaceIsComplete` + a mount retry that discards the
   husk first, and by `attachTerminalSurfaceToHost` rebuilding rather than moving
   a husk. **Species B remains open:** a terminal that opened completely and lost
   its screen afterwards has an armed guard, so `open()` is a no-op and only a
   remount helps; the code now reports that honestly as
   `mode=rebuild_from_husk_failed` instead of claiming a repair.
2. **~~Was the reveal ghost involved?~~ NO — falsified 2026-07-22.** The
   `reveal_ghost_attached` ≫ `reveal_ghost_released` gap looked damning (27 vs 9
   in one generation) but is an accounting artefact: `releaseRevealGhost` is
   gated on `revealGhostFrame.isConnected`, so any host wipe that already removed
   the ghost suppresses the release event. Measured live: **zero**
   `.yggterm-reveal-ghost` nodes in the document. The ghost is also a bare
   `<canvas class="yggterm-reveal-ghost">`, not an `.xterm` div, so it cannot be
   the orphan root regardless.
3. **Why does it only hit some mounts?** Two measurements narrow it:
   - **The husk is born AT MOUNT, not on switch-back.** Every earliest-episode
     autopsy shows the same same-millisecond sequence: `constructed` →
     `renderer_decision phase=init` → `xterm_session_snapshot_restored` →
     `rebind_host reason=emit_paint same_host=true term_disconnected=true
     term_outside_host=true` → the detach event. This **corrects** the earlier
     "only on heavy streaming, only on switch-BACK (large backlog replay)" lead.
   - **18 of 18 husked hosts were `constructed` more than once at the SAME mount
     epoch**, and the reconstruct gap preceding a husk is far tighter than
     baseline (median 5.4 s vs 219 s; p75 101 s vs 1400 s). 127 of 225 hosts
     remount ≥2× without husking, so a tight same-epoch re-mount is **necessary
     but not sufficient**. Suspect two live closures for one `hostId`.

### Workaround / fix

**Shipped 2026-07-22: one owner for restoring the surface into a host.**
`attachTerminalSurfaceToHost(targetHost, site, allowOpen)` moves `term.element`
back (`appendChild` re-parents a node that lives elsewhere) and only falls back
to `term.open()` when the terminal has genuinely never been opened. Both
recovery paths — `rebindCurrentHost` and `ensureVisibleHost`'s
`rebuild_blank_host` — now go through it, unconditionally, in the same
synchronous task as their wipe. **Invariant: a host wipe must be followed by a
restore; there is no branch under which leaving the host empty is correct.** The
old rebind re-appended only under three conditions and otherwise trusted
`term.open()`; the old rebuild-blank path trusted it exclusively.

⛔ Superseded advice: the earlier "one-line repair" (make
`!liveHost.contains(term.element)` a reopen trigger) **shipped as `820d0d5` and
was a regression** — it fires forever on backgrounded hosts, whose host leaves
the DOM entirely. Gated on `liveHost.isConnected` in `f0aca70` and bounded by a
circuit breaker in `f27d5db`. Do not re-derive it.

Live recovery without any restart, for a session already stuck: re-append
`term.element` into the host and drop the husk via `app dom-eval`.

### Code locations

- `crates/yggterm-shell/src/shell.rs` — `terminalHostAttachmentState()`: DOM-truth
  attachment probe, husk signature, and the three repair guards evaluated live
- `crates/yggterm-shell/src/shell.rs` — `syncHostAttachmentEntry()`: episode
  tracking + the `terminal_host_element_detached` trace event
- `crates/yggterm-shell/src/shell.rs` — `updateRenderHealth()`: detached element
  is now its own unhealthy verdict; zero ink samples marked `unsampleable`
- `crates/yggterm-shell/src/shell.rs` — `emitPaint()`: records
  `lastVisiblePaintTermElementAttached` / `lastVisiblePaintWasHusk`
- `crates/yggterm-shell/src/shell.rs` — `window.__yggtermRecordHostMutation`:
  breadcrumb (site + stack) at every host wipe and every `term.open`
- `rebindCurrentHost` / `ensureVisibleHost` — the two repair paths that decline

### Tests

`terminal_eval_script_probes_detached_term_element` — asserts the attachment
probe, the alarm condition, the guard record, the detach event, the
`unsampleable` ink marking, the husk companion field, and that every mutation
site carries a breadcrumb defined before the first wipe.

### Telemetry

- `terminal_host_element_detached` — once per detach episode then at most every
  30 s while it persists. Carries `unrepairable`, `orphan_root_without_screen`,
  `xterm_roots`, `screen_in_host`, `rows_in_host`, `screen_canvases`,
  `repair_would_reopen`, and the correlated `last_mutation_site` /
  `last_mutation_age_ms` / `last_mutation_stack`.
- `app state` → `active_terminal_hosts[]`: `host_attachment_state`,
  `term_element_connected`, `host_contains_term_element`,
  `term_element_detached_since_ms`, `term_element_detached_count`,
  `last_visible_paint_term_element_attached`, `last_visible_paint_was_husk`,
  `last_host_mutation`, `host_mutation_count`.
- `render_health_status` now goes `unhealthy` with reason
  `term_element_detached_from_host` (or `…_unrepairable` when every repair guard
  declines). **Alarm on `unrepairable_detached` — it means permanently blank
  until a manual remount.**

### Related memory

`[[campaign-telemetry-infinite]]`, `[[feedback-verify-visual-with-faithful-pixel]]`,
`[[xterm-host-registry-leak]]` (same registry, different failure).

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
