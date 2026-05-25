# xterm.js Terminal Notes

This document is the working contract for Yggterm's xterm.js path. Keep it in
sync when terminal rendering, resize, PTY identity, app-control probes, or smoke
tests change.

> **For specific xterm.js bugs and the workarounds we've shipped, see
> [`docs/xterm-bugs.md`](xterm-bugs.md).** That file is the structured
> registry: one section per bug, cross-linked to inline `// XTERM-BUG:`
> anchors in code. Add a new entry there before closing any xterm-related
> regression.

## Core Contract

Yggterm terminal content has one primary truth:

1. The daemon owns the runtime and PTY stream.
2. xterm.js owns terminal parsing, buffer state, cursor, selection, and painting.
3. The shell owns layout, focus, session identity, metadata, and observability.

Do not repair a terminal screenshot with shell-owned overlays. Prompt
backgrounds, cursors, typed echo, scrollback, resize redraws, and Codex status
animation must come from PTY bytes, xterm.js cell attributes, theme mapping, or
xterm.js-native APIs that remain part of the terminal surface. Any diagnostic
shim must be opt-in, observable, and rejected by release smokes.

Cursor and inactive states must preserve xterm's native cursor shape. Yggterm
uses xterm's native block cursor with CSS cursor blinking disabled in the
desktop shell. A static visible native cursor is preferable to a shell overlay
or PTY-byte rewrite, and it avoids WebKit/GTK idle CPU burn from a perpetual
cursor animation on KDE/Wayland. The constructor requests `cursorBlink: false`;
the terminal surface also applies an xterm-scoped CSS backstop that sets cursor
animation to `none` because retained DOM renderer paths can keep cursor blink
classes or option truth in surprising states after restore. If blinking is
reintroduced, blink/off must be truly off: no shell-owned cell fill, no
terminal-theme patch, and no prompt-background hole. Yggterm may sample the
current xterm buffer/DOM cell background for observability and inactive-outline
fallback, but it must not infer a Codex prompt row from text or draw a separate
cursor/prompt overlay. Restored xterm surfaces must refresh this sample after
mount as well as after render/write events because a preserved session can
become visible without a fresh xterm render callback. App-control exposes the
sampled cursor cell background and the xterm `cursorBlink` option as
diagnostics; the release gate is no cursor-driven running CSS animation, no
one-cell prompt-background hole, and no shell-owned cursor overlay.

Focused block cursors must paint with xterm's cursor theme color even when the
cursor span is attached to a dim Codex placeholder cell. The sampled cursor cell
background is diagnostic/inactive-outline data; it must not become the focused
block cursor fill, or the cursor disappears into styled prompt rows while still
existing in DOM.

Do not treat `.xterm-dim` on a cursor span as blink/off state. xterm may attach
the cursor span to a dim prompt placeholder cell, so the cursor can legitimately
carry `.xterm-dim` while it is visible. With cursor blinking disabled, a focused
terminal may still expose an xterm-owned `xterm-cursor-blink` class on some DOM
renderer paths, but app-control must show no cursor-driven running CSS
animation. If blinking is re-enabled later, the only blink/off proof is the
rendered cursor node becoming hidden through xterm's native opacity/animation
state; the underlying sampled cursor cell background must still match the prompt
cell, not the terminal viewport background.

An auto-hidden titlebar covering the first xterm row is not proof that the
terminal is unpainted. App-control may report the covered row, but if the prompt
row or cursor cell is still topmost inside `.xterm-rows`, the surface remains
paint-visible and must not lose input/focus. The cursor smoke also checks styled
Codex prompt rows: the sampled cursor cell background for a `›` prompt row must
preserve the prompt cell background and must not collapse to the terminal theme
background.

Foreground input belongs to the selected active terminal host only. Retained
offscreen hosts may still have stale `.xterm-helper-textarea` focus after a
switch, WebKit focus loss, or app restart; that stale helper focus is diagnostic
noise unless the host is the active session host or still reports enabled/raw
input. A different-session host with `input_enabled=true` or
`raw_input_enabled=true` is an identity violation. A different-session retained
host with disabled input and stale helper focus must release focus on the next
Rust policy pass and must not keep the active viewport in recovery.

Codex onboarding and authentication menus are interactive xterm surfaces. A
fresh remote Codex start may show `Welcome to Codex`, sign-in choices, or a
permission/setup menu before any prompt-ready transcript identity or storage path
exists. xterm-visible tails may start mid-menu after logo art or scrollback
sampling, for example `tGPT ... Device Code ... API key ... Press enter to
continue`; those tails are still the same interactive auth surface. That state
is not a saved session yet, but it is still input-ready PTY truth. Resume gates,
loading notifications, and recovery policy must clear for those explicit menus
so the user can press the requested key. The fix belongs in the
terminal-readiness classifier over xterm-visible PTY text; do not add a
shell-owned prompt overlay or alternate input target.

This document owns terminal-rendering law. The cross-system source-of-truth
audit lives in `docs/architecture-audit-2026-05-16.md`; if a terminal fix needs
different behavior than this file describes, update this file before changing
runtime code. App-control fields, telemetry rows, screenshots, retained
snapshots, and smoke probes are witnesses. They must not become terminal
content, a terminal input path, or an alternate replay source.

Manual redraw is not a terminal repair primitive. It may refresh xterm's current
buffer, renderer, texture atlas, and viewport. It cannot recreate PTY bytes that
were filtered, coalesced, dropped, replayed from the wrong source, or routed to
the wrong runtime. If manual redraw fixes a surface, classify the incident as a
renderer-settle or activation bug. If it does not, continue investigation at the
PTY stream, runtime identity, geometry, or retained-replay boundary.

One narrow renderer repair is allowed inside manual redraw: if app-control proves
the xterm DOM renderer still has a screen node and buffered PTY text, but has lost
both `.xterm-rows` and canvas layers, redraw may reattach xterm.js' own renderer
row/selection containers and ask xterm to repaint. This is not a Yggterm overlay
or alternate renderer; it is recovery of xterm's missing render surface, and must
be observable through `renderer_surface_missing`, `renderer_surface_recovery_count`,
and `active_terminal_surface.render_health.reason`.

## Interaction Contracts

Primary selection is not the clipboard. On Linux-style middle-click paste,
Yggterm records non-empty `term.getSelection()` text as
`window.__yggtermPrimarySelection`, then handles middle mouse down by calling
`term.paste(text)`. That keeps bracketed-paste handling inside xterm.js and
sends the resulting bytes through the normal terminal input path. The fallback
path is xterm's core `triggerDataEvent`, then a direct terminal input event; none
of these read from `navigator.clipboard`.

Primary selection itself must come from xterm.js pointer handling. The shell may
reclaim terminal input focus on pointer down, but transparent focus helpers,
context-menu backdrops, and app-control probes must never become the hit target
for drag selection or double-click word selection. The focus-capture element is
observer/focus scaffolding only and must keep `pointer-events: none`.
`probe-select` is therefore a pointer-gesture probe: it dispatches a real drag
or double-click against visible xterm rows, requires non-empty
`term.getSelection()` plus xterm selection-layer rectangles, and treats DOM
Range selection or buffer-only text as diagnostic data, not a pass.

Terminal selection copy is also not a browser-clipboard operation. `Ctrl+Shift+C`
and `Ctrl+Shift+X` may read xterm's selected text, but the xterm embed must send
that text to Rust over the terminal JS event bridge and return immediately. The
renderer must not call `navigator.clipboard.writeText`; on remote desktops that
can block inside WebKit/portal/Remmina clipboard synchronization and freeze the
app. Rust owns a dedicated native clipboard owner thread for terminal selection
copy, so a blocked desktop clipboard stack cannot stall the shell render loop or
app-control response path.

Text paste follows the same ownership rule. Browser paste events are captured
only when the active xterm host owns terminal input, stopped before WebKit/xterm
can also consume them, then converted into one native Rust clipboard read. The
JS side keeps a short duplicate-event window because WebKit/portal/remote
desktop stacks can surface more than one paste event for a single user gesture.
That dedupe is observable through app-control fields such as
`clipboard_paste_event_count`,
`clipboard_paste_duplicate_suppressed_count`,
`native_clipboard_paste_request_count`, and
`native_clipboard_paste_request_deduped_count`; it must not log clipboard text.

Right-click on a terminal is an xterm surface event that opens Yggterm's row
context menu for that terminal session. The terminal host captures secondary
pointer events before xterm.js's helper-textarea right-click handler and also
captures the native `contextmenu` event. It prevents WebKit's browser menu and
native paste path, records the host-local counter/coordinates, and sends a
`context_menu` terminal JS event through the same Rust bridge as
input/resize/clipboard. The shell then opens the existing session context menu;
it does not create a second menu implementation inside the terminal DOM. A
right-click must not call `term.paste`, `triggerDataEvent`, native clipboard
read, or direct terminal input. Middle-click remains the only primary-selection
paste gesture.

The context-menu backdrop is shell chrome, not a terminal gesture owner. It must
not block primary pointer events that belong to xterm rows. When a primary
terminal pointer begins while a context menu is visible, the xterm bridge asks
the shell to close the menu and lets the same gesture continue to xterm.

Active terminal write batching must stay below the threshold where typed input
or Codex's `Working` animation feels stepped. Batching is only a flush-timing
tool: it may delay a chunk briefly for CPU, but it must never drop, reorder,
deduplicate, trim, or rewrite PTY bytes. Background terminals can keep the large
CPU-saving frame budget, but the active visible terminal defaults to a 160 ms
write frame and the inline status-animation hot path stays at the smaller
animation budget. Long-running inline status animation may cool after the first
few seconds because the user's input path and session truth are more important
than keeping a spinner at the initial cadence for minutes.

Codex synchronized output frames (`CSI ?2026 h` ... `CSI ?2026 l`) are not
semantic frames that Yggterm may collapse. They can be partial terminal deltas:
one frame may clear rows, the next may paint only a cursor suffix or prompt
fragment. If Yggterm drops the earlier frame, xterm.js can show missing letters
or a broken prompt even though the PTY accepted the input. Deliver synchronized
frames to xterm in PTY order and let xterm parse them.

Layout-driven resize is not user scrollback. Titlebar auto-hide hover,
fit-addon row changes, and resize-observer bursts arm a short prompt-follow
layout guard. While the host is in `PromptFollow`, scroll events from those
programmatic changes are suppressed as user-scroll evidence and the viewport is
forced back to the active buffer bottom after the fit settles. If the user has
explicitly entered `UserScrollback`, the same layout changes preserve that
reading position instead of snapping to bottom. Visible-paint repair is not a
resize source: it may refresh or probe the mounted xterm surface, but it must
not call fit logic or notify the daemon PTY of a new grid.

Paint events are observers, not geometry authority. A paint probe may report the
mounted xterm grid for diagnostics, but it must not call the daemon resize API.
Once xterm has a usable grid, unfocused `ResizeObserver` transients must also be
ignored until the window/document is focused again; app-control state,
screenshots, hidden-titlebar hover, and compositor snapshotting are observers
and must never bounce a live PTY between stale grid sizes.

Prompt-gap recovery must not resize the daemon PTY as a repair tactic. If
app-control sees a stale public viewport counter, a large blank gap, or a
cursor-expected rectangle outside the host while xterm's visible DOM rows are
still readable, the fix belongs in xterm viewport/render settle. Yggterm must
not nudge the PTY row count down and back up to force Codex to repaint; that is
a second geometry source of truth and can break a live TUI after input resumes.

Regression coverage:

- `terminal_eval_script_supports_xterm_primary_selection_middle_paste`
- `terminal_eval_script_bridges_xterm_right_click_context_menu`
- `terminal_select_probe_uses_xterm_pointer_gesture_not_dom_range`
- `terminal_eval_script_keeps_prompt_follow_during_layout_resize`
- `scripts/smoke_xterm_embed_faults.py --only-check primary_selection_paste`
- `scripts/smoke_xterm_embed_faults.py --only-check terminal_context_menu`
- `scripts/smoke_xterm_embed_faults.py --only-check titlebar_autohide_prompt_follow`

## Jojo Finding, 2026-05-12

The live jojo 2.2.33 viewport showed a current Codex prompt but old status
output above it was broken into old-width stacks. App-control agreed the surface
was not healthy: the active host reported `terminal_settled_kind=problem`,
`terminal_content_source=active_recovery_pty_snapshot`, and the failure reason
`active remote terminal accepted multi-row replay without scrollback`.

That failure was not an xterm paint bug. A retained/recovery snapshot containing
cursor-addressed, multi-row Codex output was being treated as a ready surface and
could reset/replay into xterm after the live PTY had already produced a better
screen. The fix is to reject cursor-addressed multi-row Codex recovery snapshots
as replay sources unless they are explicit retained scrollback. Yggterm must wait
for daemon PTY bytes at the current geometry instead of rewriting xterm with an
old-width snapshot.

The 2.2.34 live proof exposed a second, deeper version of the same class: the
daemon's vt100 side parser can produce a formatted screen snapshot that looks
like terminal bytes, but it is not the xterm.js buffer and it can repaint
cursor-addressed Codex/TUI rows incorrectly. Remote Codex/TUI initial attach
must therefore replay retained raw PTY bytes only. The daemon may keep the
parser for observability, but it must not promote `screen().state_formatted()`
to viewport content or synthesize attach-ready just because a remote runtime key
exists. Attach-ready is valid only after the remote helper actually reports it.

The 2.2.35 live proof then showed the same compact cursor-addressed frame
leaking through the shell's active-recovery snapshot path. That path must use the
same rule: a Codex/TUI recovery snapshot containing clear-screen plus absolute
cursor movement is not scrollback and is not a ready proof, even if it contains a
visible `›` prompt. Wait for real PTY bytes at the mounted xterm geometry or
keep the terminal in recovery.

Auto-hidden titlebar reveal also must not change terminal host geometry. The
titlebar is absolute chrome; hover reveal may visually cover the top edge, but it
must not add content padding or cause a PTY/xterm resize. Resizes remain a PTY
contract only when the xterm grid actually changes.

The old low-power TUI path was also disabled for release builds. Alternate-screen
programs such as `htop` must keep their bytes flowing through xterm.js while the
session is inactive or the app is unfocused. Dropping frames or drawing a
Yggterm-owned text overlay creates a second terminal truth and leaves stale TUI
state when the user switches back.

In 2.6.1, the 23-smoke restore pass exposed the inverse failure for daemon-owned
live terminals: after a GUI close/relaunch, a fresh xterm could attach to an
already-running TUI and receive only future incremental cursor-addressed deltas
(`Mem[...] 59%`, frame counters, and similar fragments). The daemon's vt100 side
parser already had the current full screen, while the retained raw tail no
longer had the clear/full-frame bytes needed to seed a fresh renderer. Initial
attach may therefore replay the current daemon vt screen image only for
full-screen live or runtime-owned surfaces, only when no retained scrollback is
being preserved, and only as terminal bytes fed into xterm.js. This is not a
shell overlay and must not replace retained scrollback or synthetic UI state.
Regression coverage:
`initial_live_tui_attach_replays_current_screen_snapshot_over_incremental_tail`
and the 23-smoke restore pass for seven TUI-heavy terminals.

For explicitly kept live sessions, closing the Yggterm window is the detach
boundary: xterm may unmount from the GUI, but the daemon PTY, remote Codex
process, preserved-owner route, and live metadata must remain available for the
next attach. The sidebar close affordance is different. It means close the
selected live terminal runtime, even when that runtime is marked Keep Alive.
Reattaching to a kept session after a GUI close must reuse the same PTY owner or
stay in recovery; it must not spawn a fresh `codex resume` as a substitute for
the lost owner.

GUI close must not change Terminal mode into Web View as an intermediate
rendering state. Closing may detach the xterm surface, hide the window, or hand
the runtime to a preserved owner, but the rendered snapshot remains Terminal
until the window is gone. A visible close path that briefly shows the active
session as Web View is a shell state bug: it creates a second surface truth and
can make users believe the terminal was replaced by transcript/preview content.

In 2.6.22, the live jojo scroll incident tightened the retained-replay rule:
cursor-addressed retained snapshots are dangerous as a visible screen seed, but
replacing them with a current screen-only snapshot is also wrong because it
destroys xterm scrollback. The safe recovery shape is two-part terminal input:
write plain retained PTY history into xterm as scrollback, then clear only the
visible screen with `CSI 2 J; CSI H` and write the daemon's current screen
snapshot. Do not use `CSI 3 J` in this path because the history rows are the
scrollback being preserved. If the history cannot be extracted safely, the
surface must stay observable as degraded instead of pretending a screen-only
replay is a full terminal restore.

Wheel scrolling is a viewport operation, not a shell input operation. A terminal
may have user input gated while a remote resume or retained replay is being
validated, but its existing xterm scrollback must still respond to wheel and
app-control `probe-scroll` actions when that host is the active terminal. Paste,
keyboard input, and terminal writes remain readiness-gated.

A reused `Remote Launch Action=start-codex` row is not proof that the runtime is
fresh once the daemon reports existing runtime output or the shell has observed a
ready viewport. In that state, daemon PTY retained bytes remain the terminal
history truth and may be replayed after ready-settle even if the original launch
action string still says `start-codex`. Fresh-start metadata must not suppress
retained scrollback hydration for an already-running remote Codex session.

An active remote Codex terminal that receives wheel input while xterm reports
`base_y=0` is not a healthy prompt-follow state when the visible tail contains
prior output. Classify it as retained scrollback loss: the viewport cannot move
because the restored xterm buffer has no history. The recovery path is retained
PTY replay followed by the current screen seed, not another screen-only ready
classification.

The same rule applies to retained-ready rehydrate modes. A preserved-owner
`CollapsedScrollbackRecovery` must convert cursor-addressed retained history to
history-plus-current-screen before it reaches xterm. A later short
`InitialRead`/`daemon_terminal_read` is allowed to update the current screen
only after the restored xterm already has real scrollback; it must not be the
final replay source for a session whose daemon retained snapshot still has
hundreds of lines.

The inverse is also part of the contract. Once a retained rehydrate has already
staged daemon retained history into xterm for a mount, the separate daemon
retained replay worker must not run again and overwrite the fresh
`daemon_terminal_read` screen. A retained-history replay is a scrollback seed,
not an input-ready terminal truth. If app-control ever reports a remote surface
with `input_enabled=true` and
`terminal_content_source=daemon_retained_history_screen_snapshot`, the shell has
promoted a retained replay too far and must re-enter recovery instead of
accepting user input.

Trusted live input is also a retained-replay boundary. When the xterm host
promotes a retained source to `daemon_pty`, any pending retained replay for that
session must be marked superseded and stop retrying prompt-follow or repaint
work. App-control exposes this as
`retained_replay_superseded_by_daemon_pty=true`. A live daemon PTY may retain
scrollback seeded from daemon history, but idle/focus repaint must not keep
using retained replay as a second viewport controller.

Accepted live input is the same hard boundary. Once the mounted xterm bridge has
successfully written user input to the daemon for a remote resume session, the
GUI must treat that host as live-connected and must not run any delayed daemon
retained replay task for that session. A delayed retained replay may still log
`daemon_retained_replay_skipped_live_connected`, but it must not write bytes,
force prompt-follow, or replace the xterm buffer after the user has resumed
typing.

The UI retained-rehydrate path follows the same boundary. It may read from the
daemon to seed an empty retained host while the surface is not live-connected,
but it must check the live-connected flag again immediately before reading and
again before writing to xterm. If a concurrent daemon read or accepted input has
already promoted the host to `daemon_pty`, retained rehydrate must log
`retained_rehydrate_skipped_live_connected` or
`retained_rehydrate_result_discarded_live_connected` and return without
mutating xterm.

The narrow scrollback-recovery exception is preserved-owner
`CollapsedScrollbackRecovery`: if a short live `daemon_terminal_read` painted
the current screen first, but no retained history has been staged and input is
still gated, the retained-history seed may still write daemon-derived history
plus the current screen into xterm. This restores the xterm buffer before the
session becomes interactive. It must not run after input is enabled or hot, and
an xterm-owned session snapshot must remain `xterm_session_snapshot`, never
`daemon_pty`.

Remote resume readiness has two separate buffers. The replay buffer is allowed
to clear after attach-ready so Yggterm does not duplicate bytes already written
to xterm. That clear must not erase the readiness proof. A bounded
observed-output sample from the same daemon PTY read may continue to prove
visual reveal until input is enabled or recovery resets the mount. App-control
may later report the same visible surface, but app-control must not be the only
thing that can clear a stale `Restoring Remote Terminal` gate.

Retained-fault recovery must not use a non-prompt screen snapshot as an xterm
writer. That snapshot can be logged for telemetry, but the visible interactive
surface must come from the live PTY stream. The old shortcut reset the terminal,
wrote the snapshot, and called the session ready; that created repeated
remounts, flicker, and occasional selected-session/xterm identity splits.

Retained rehydrate must also wait for the current daemon endpoint before it
asks for `terminal_read`, `terminal_snapshot`, or `terminal_retained_snapshot`.
During hot update the current daemon may be a sidecar that forwards terminal I/O
to an older preserved PTY owner. The GUI must read through the current daemon so
the owner map and saved-session mismatch checks remain single-source, but it
must not sample the current socket before that daemon is reachable. A failure
like `connecting to ~/.yggterm/server-<current>.sock` before the daemon is ready
is a startup boundary issue, not a reason to reset xterm or wait for a
5-second retained-fault watchdog remount. The daemon-ready wait is an explicit
shell state: while it is in flight, retained-fault watchdogs must defer and log
that deferral instead of bumping the xterm mount epoch. Once the wait clears,
the same watchdog may remount only if the surface is still stale or blank.

Prompt-follow checks must also use the visible xterm viewport, not only
xterm.js's public `buffer.active.viewportY`. On WebKit DOM rendering the public
counter can temporarily stay at `0` while the `.xterm-viewport` element is
already scrolled to the prompt. Treating the public value as sole truth causes a
false "stuck in scrollback" recovery and remounts a good PTY surface. The shell
therefore computes an effective viewport from the DOM scroll position when the
two disagree, and app-control exposes `public_viewport_y`,
`visual_viewport_y`, `effective_viewport_y`, and `viewport_y_source` for proof.

Mounted xterm hosts must remain in WebKit's normal paint tree. Do not hide the
active terminal by moving it offscreen with transforms, and do not wrap active
xterm DOM rows in strict paint containment. The accepted active wrapper contract
is light layout/style containment plus normal visibility. Hidden retained hosts
are renderer caches only, not session truth. On KDE/Linux the stable default is
to keep only the active xterm host mounted and remount inactive live sessions
from daemon PTY/retained history when selected, because many hidden full-size
xterm DOM trees raise WebKit CPU and reintroduce stale-render ambiguity.
Strict/offscreen compositor isolation can produce the bad split where xterm row
DOM, buffer text, and app-control state exist while the user-visible paint is
blank or stale after a session switch.

App-control must expose DOM paint hit-tests for active terminal hosts. The
snapshot samples the row and cursor rects with `document.elementsFromPoint` and
reports `dom_paint_hit_test_problem` plus the paint stacks. A state with
non-empty xterm text and a non-empty paint-hit problem is not terminal-ready,
even if daemon runtime truth, xterm buffer text, and geometry are otherwise
healthy. One exception is expected chrome coverage from the auto-hidden titlebar:
if the top row sample is covered by Yggterm titlebar chrome but the cursor/prompt
sample is topmost in the xterm rows, the terminal is still paint-ready because
that chrome is allowed to visually cover the top edge without resizing the PTY.
This diagnostic is only evidence; it must not draw terminal content or become a
fallback renderer.

In 2.6.65, jojo reproduced the same split on the DOM renderer during
post-update retained replay: app-control reported daemon-backed xterm rows,
prompt text, and clean input readiness, while the first WebKit screenshot still
captured a blank dark terminal rectangle. The fix is not a shell overlay and
not a second renderer. When retained replay accepts already-visible text instead
of writing a new payload, the shell forces one bounded xterm-native refresh
immediately, on the next animation frame, and once more after 120 ms. The
refresh is observable as `last_retained_replay_paint_refresh_debug` and must
remain a one-shot paint flush tied to retained replay, not a periodic recovery
loop.

In 2.6.66, jojo reproduced the harder DOM-renderer failure: xterm's buffer still
contained the live Codex output, but WebKit had lost the DOM renderer's `.xterm-rows`
layer entirely and no canvas layer existed. App-control now classifies that exact
state as `dom_renderer_missing_text_layer_with_buffer_text`; manual redraw and
host rebind attempt an xterm-native renderer-surface repair before accepting the
surface as readable.

## Jojo Finding, 2026-05-17

Wayland xterm canvas is no longer a default product renderer. A live 2.6.35
jojo client exposed a split where app-control could read daemon-backed xterm
buffer text, but the user-perspective screenshot showed a blank dark terminal.
That means canvas cannot be accepted as ready unless screenshot pixels and
app-control agree. Canvas remains an explicit diagnostic opt-in through
`YGGTERM_ENABLE_XTERM_CANVAS=1`; the default release path uses DOM rows so
terminal text, screenshot proof, selection, and app-control visibility share one
observable surface.

App-control must reject canvas hosts that report buffered text but a visibly
low-contrast foreground/background contract, such as black text on the dark
terminal surface. State-only buffer text is not enough terminal proof.

## Jojo Finding, 2026-05-13

The live 2.4.0 jojo viewport could become visually blank across the upper xterm
canvas after switching between live sessions while remaining functional at the
prompt. App-control showed the daemon and xterm buffer still had history,
`input_enabled=true`, and no geometry problem; a manual app-control
`terminal redraw <session>` restored the missing rows through xterm.js native
refresh. This is a stale renderer/canvas texture problem after active host
switch, not session loss and not a reason to draw shell-owned terminal text.

The shell must schedule a bounded activation repaint when the active terminal
session changes. That repaint is allowed to call the mounted host's xterm-native
redraw/refresh/texture-atlas clearing paths and prompt-follow logic. It must be
bounded and observable through app-control fields such as
`activation_repaint_count`, `last_activation_repaint_reason`, and
`last_activation_repaint_at_ms`; it must not become a periodic render loop.

Regression coverage must include a screenshot-pixel check that detects a
history-backed xterm buffer whose upper canvas is blank after session switching.
`scripts/smoke_xterm_embed_faults.py` therefore samples the active host's
history text and verifies visible non-background pixels above the prompt after
hot session switch and return.

## Jojo Finding, 2026-05-09

The active jojo session was visually dark, used the xterm canvas renderer, and
had a healthy daemon runtime. The prompt line did not have a grey background
because the recent PTY payload cleared bottom rows with default background:

```text
ESC[0m ESC[49m ESC[K
```

There was no `48;...m` background attribute in the sampled prompt repaint. In
that state xterm.js is doing the correct thing: it paints the prompt cells with
the terminal default background. The missing prompt band is therefore upstream
of painting: terminal environment, Codex palette selection, PTY bytes, or a
retained runtime launched with stale identity.

The same live session exposed a second clue. The visible terminal theme was dark,
but preserved remote Codex processes still had light terminal identity:

```text
COLORFGBG=0;15
TERM_PROGRAM=vscode
TERM_PROGRAM_VERSION=2.1.163
```

The 2.2.0 remote yggterm parent was also observed with `COLORFGBG=0;15` for the
kept runtime, while the visible session metadata showed a dark launch recipe.
That is a retained-runtime identity mismatch. Hot update must not kill a kept
session just to correct palette environment, but new and restored attach paths
must not synthesize light terminal identity for a dark xterm viewport.

In 2.2.1, GUI terminal-identity sync also refreshes daemon-owned remote Codex
launch commands, and headless exposes an explicit `server terminal restart`
request. Restart remains deliberate because preserved runtimes are real user
sessions: session survival wins by default, and identity correction happens only
for future launches or for sessions the operator explicitly restarts.

In 2.2.2, remote-runtime daemon requests also carry the terminal appearance.
This matters because the SSH wrapper can have dark `COLORFGBG=15;0` while the
remote daemon that actually spawns Codex was still running with a stale light
environment. The remote daemon must sync identity before creating a new
`codex-runtime://...` PTY. Forced remote restart must also terminate matching
Codex runtimes across versioned remote daemons; otherwise the bridge can
reattach to a stale light runtime instead of replacing it.

In 2.2.3, forced remote terminal restarts use the long daemon response budget.
The server can legitimately spend more than the normal control timeout while it
contacts a remote machine, terminates the matching Codex runtime, and recreates
the local bridge. Timing out the headless client in that window makes the
control surface nondeterministic even when the server-side restart succeeds.

In 2.2.4, forced terminal restart also treats the remote scan row as a valid
recovery source before it terminates anything. If a hot handoff or previous
partial restart drops live membership but the session is still visible under
`remote-session://<machine>/<id>`, the daemon promotes that row back into a live
terminal spec first and only then recreates the runtime.

In 2.2.5, remote bootstrap chooses the active direct-install headless binary
from install metadata before falling back to the caller process. This prevents a
preserved older daemon from copying its older adjacent binary back onto a remote
machine while the GUI has already moved to a newer hot-update version.

In 2.2.6, the daemon answers OSC 10/11 default foreground/background color
queries from the PTY stream before GUI attach, using the same dark/light terminal
identity that Yggterm exports to the child process. In 2.6.50, that same daemon
protocol filter also answers OSC 4 palette queries and preserves non-query
palette set sequences for xterm.js. The daemon strips query bytes from retained
output and writes xterm-style `rgb:rrrr/gggg/bbbb` responses back through the PTY
writer. The xterm.js frontend also suppresses its own OSC 4/10/11 fallback
responses, because frontend terminal replies sent through `onData` can echo
into cooked shells and become visible `rgb:` text. This closes the Codex startup
race where Codex asked `/dev/tty` for the terminal background before the
xterm.js viewport was mounted, timed out, cached `None`, and then emitted `49m`
instead of a real prompt background attribute. This is terminal-emulator
protocol handling, not a shell overlay or prompt decoration.

In 2.6.53, terminal identity carries the effective xterm color profile, not only
the light/dark label. The shell syncs foreground, background, and the 16 ANSI
palette colors into the daemon, and launch commands export those values with
`YGGTERM_TERMINAL_COLOR_*`. The daemon's OSC 4/10/11 answers must therefore
match the active terminal theme used by xterm.js. TUI editors such as `edit`
depend on this: they query `/dev/tty` for palette/default colors and then render
with that inherited theme. If an older preserved daemon cannot answer OSC 4
palette queries, the frontend may let xterm.js provide its fallback response only
while the terminal is clearly in a frame-like or alternate-screen TUI state; the
normal shell prompt path still suppresses frontend replies so cooked shells do
not print `rgb:` protocol text.

Some long-lived daemons can already have visible `rgb:` protocol replies in
their retained screen snapshot. The GUI may strip that legacy pollution before
replaying a retained payload into xterm.js; it must not strip ordinary PTY output
that does not match visible OSC color-response text.

In 2.2.43, the identity source was tightened again: daemon warm-start, watchdog
restart, initial GUI/server sync, and restored launch-command refresh use the
effective xterm terminal theme rather than the outer Yggterm UI theme. A light
shell with a dark terminal theme such as Andromeda must export
`YGGTERM_TERMINAL_APPEARANCE=dark` and `COLORFGBG=15;0` before a remote Codex
runtime is launched or restarted. A screenshot with a dark xterm background and
a pale Codex input band whose text is also pale is a terminal-identity failure,
not a renderer layering problem.

The same release separates compositor focus from app-control terminal proof on
Wayland. If app-control has explicitly reclaimed terminal focus, the mounted
xterm host may use the active write budget even when native window activation is
denied by the compositor. Explicit app-control backgrounding still disables that
path and remains the low-CPU/fan-budget proof mode.

In 2.2.44, managed Codex background refresh became probe-only by default. Remote
terminal recovery, machine scans, and idle refresh loops must not run `npm
install @latest` just because a managed Codex binary exists. That work can spike
CPU on the remote machine and compete with the PTY/xterm path. Unattended
background installs require `YGGTERM_MANAGED_CLI_BACKGROUND_INSTALL=1`; normal
background refresh keeps using the available managed or PATH binary until an
explicit foreground ensure/refresh path is used. Explicit local terminal launch
is such a foreground path: before launching or resuming a local Codex session,
Yggterm must run the managed CLI ensure path so a stale `~/.yggterm/npm/bin/codex`
does not show Codex's own interactive update prompt inside xterm.

In 2.2.45, retained replay gained an explicit unsafe-skip state for resize
recovery. A large cursor-addressed retained snapshot is not safe to replay into
xterm, but if the current xterm buffer already has a prompt-ready cursor row,
Yggterm must treat that existing PTY surface as the truth instead of marking the
skipped snapshot as expected scrollback. App-control exposes
`retained_replay_unsafe_skip_prompt_ready` and
`retained_replay_rejected_visible_text`; a prompt-ready unsafe skip may keep the
terminal interactive, while the same base-y-zero state without that diagnostic is
still a retained scrollback-loss failure.

In 2.6.15, scroll probe truth was tightened after a false-positive live probe.
`movement_expected` means the current xterm viewport can actually move in the
requested direction: `viewport_y > 0` or DOM `viewport_scroll_top > 0` for
scrollback-up, and `viewport_y < base_y` for return-to-bottom. Historical raw
line counts or `scrollback_expected` may indicate an unhealthy collapsed surface,
but they are not proof that the mounted viewport can move. `scroll_probe_moved`
must only become true when `viewport_y` or DOM `viewport_scroll_top` changes.
Wheel/scroll event counters and `text_head`/`text_tail` churn are diagnostics
only; live output during a probe is not scroll movement.

In 2.6.16, remote daemon PTY output without a current prompt row became
readable-only. A mounted xterm that contains meaningful daemon output can be a
valid evidence surface while `input_enabled=false`, but it must not be reported
as user-input-ready unless the current prompt/input row is visible. App-control
must reject `input_enabled=true` on a remote daemon PTY surface whose cursor row
is empty or non-prompt text, even if retained replay came from daemon PTY bytes.

In 2.2.7, forced remote restarts also terminate plain remote bridge processes
whose command line is `yggterm server remote resume-codex/start-codex <session>`.
The 2.2.6 live proof showed that killing only daemon-owned or tmux/screen
runtimes was insufficient: jojo recreated the local bridge, but dev still had
the old remote `resume-codex` process alive, so Codex kept its cached palette
state and stale retained TUI. `--force-remote` now means the matching remote
bridge process must go away before the session is relaunched.

In 2.2.8, a forced remote restart that empties the hot-update preserved-owner
registry also schedules the Linux daemon cleanup pass again. The first 2.2.7
jojo proof moved both sessions to the current daemon, but the old 2.2.6 daemon
remained reachable and advertised duplicate terminal keys restored from state.
That is not a prompt-rendering problem, but it is a multiple-truth problem for
terminal observability and later control-plane routing. Once the current daemon
owns the same keys, older duplicate daemons should retire automatically.

In 2.2.9, retained remote surfaces recover immediately when app-control observes
`active terminal host exists but xterm surface is empty` after an update or
forced restart. This is a high-confidence stale visual cache: the retained host
exists, but the mounted xterm has no screen to paint. Yggterm now bumps the
mount epoch and starts retained-fault recovery on the first observation for that
case, while still requiring a second observation for ambiguous scrollback-loss
checks that can be transient during replay.

In 2.2.10, the live terminal mount applies the same rule from host-health
samples. A previously-ready retained remote session that reports an all-empty
cursor line and buffer tail with the cursor stranded high above the prompt row
invalidates its retained xterm host directly, without waiting for an external
app-control state request to classify the viewport. Fresh remote starts are not
covered by this path because they do not have ready history yet.

In 2.2.11, host-health throttling keeps its frame-like/TUI spam guard but no
longer suppresses the all-empty retained-surface sample. A clear-screen
protocol-only write can leave a reused retained xterm with no text, hidden
cursor, and many blank rows below the cursor; that sample must reach Rust so the
2.2.10 remount rule can run before app-control observes the defect.

In 2.2.12, remote SSH launch commands set `LogLevel=ERROR`. OpenSSH control
master notices such as `Shared connection ... closed.` are transport noise, not
PTY application content, and must not be painted into xterm when a remote bridge
is interrupted or exits.

In 2.2.13, daemon-retained replay is blocked while the session's remote-resume
notification is still active. A previous Codex transcript/prose snapshot is not
a prompt-ready terminal, so it must not be painted under the reconnect overlay
while the live bridge is still settling.

In 2.2.14, daemon-retained replay is further restricted to post-ready scrollback
repair. A remote session must have a clean observed interactive viewport before
daemon-retained scrollback can replay into the active xterm host. Stale
`terminal_resume_ready_paths` memory from a previous mount is not sufficient,
because a forced restart can otherwise fill the viewport with old Codex
transcript text before the current PTY attach has produced a prompt-ready
surface.

In 2.6.10, retained remote scrollback recovery has a separate policy from the
initial retained-host read. If a retained remote host once reached ready history
and is later demoted by app-control as a collapsed or prompt-only xterm surface,
that ready history must not block a daemon-retained PTY replay. The recovery is
keyed separately from the initial mount read, uses daemon-owned PTY retained
snapshot data for collapsed-scrollback repair, and still does not enable input
by itself. This is the fix class for active terminals where wheel events reach
xterm but `base_y == 0`, `viewport_y == 0`, and the daemon still owns retained
PTY history. The pure policy owner is
`crates/yggterm-shell/src/terminal_retained_replay_policy.rs`; `shell.rs` may
execute the chosen replay but must not grow a second retained-replay policy.

In 2.6.11, the current-screen blank-host fallback is explicitly lower priority
than retained scrollback recovery. A small daemon `terminal_snapshot` may repair
a truly blank host, but it must not overwrite a staged retained replay from
`terminal_retained_snapshot`; doing so collapses xterm back to `base_y == 0` and
breaks user scroll even though the daemon still had history. Retained replay
therefore marks the terminal surface as staged/connected, emits xterm
host-health, and the blank-host fallback gates live in
`terminal_retained_replay_policy.rs` beside the retained replay policy.

In 2.6.12, a retained-fault bootstrap is itself a valid retained rehydrate
target even before the new GUI process has rebuilt `terminal_resume_ready_paths`.
Hot restart can remount the active remote xterm as an empty shell, clear local
ready-path memory, and then wait until incidental PTY output happens to arrive.
That is not a terminal truth boundary. If the current fault is an explicit empty
xterm surface, retained rehydrate must use the daemon-retained snapshot path, not
the plain initial-read path, so scrollback is restored from daemon PTY history
without requiring a manual switch pass or new user output.

In 2.6.13, prompt readiness includes wrapped Codex input regions. A long user
prompt may occupy several xterm rows between the last `›` prompt marker and the
Codex footer. That is still daemon PTY truth and must not be classified as
"input-enabled without a prompt-ready surface" just because the cursor is on a
continuation row. The classifier accepts this only for the tail prompt region
and still rejects obvious assistant/output rows after the prompt marker.

In 2.6.14, hot-restart fleet cleanup must not retire a stale daemon through an
empty runtime-coverage proof. A duplicate-runtime retire is only safe when the
monitor can name the guarded runtime keys and the expected-version daemon
directly owns every one of them. `covered_runtime_keys=[]` is unknown, not safe;
the session-survival rule wins over daemon cleanup.

In 2.2.15, the same rule applies to active-recovery snapshots for Codex-class
remote sessions. A Codex restart/reconnect snapshot must prove a Codex
prompt-ready tail, such as the current `›` prompt with model/cwd metadata or the
interactive setup prompt. Generic shell prompts or old status/prose transcript
text are rejected even when the daemon reports a running PTY, because replaying
that text would create a second, false terminal truth in xterm before the live
PTY has drawn its real screen.

In 2.2.16, the visual-reveal gate follows the same Codex rule. The mount loop may
receive post-attach bytes before xterm has a stable host-health sample, but for
Codex-class remote sessions those bytes still cannot mark the viewport
interactive unless they contain a Codex prompt-ready tail. This keeps old
transcript/prose replay from unlocking input after a forced restart.

In 2.2.17, Codex-class remote restore also respects a PTY resize fence. When the
desktop xterm host attaches or resizes, the daemon records the terminal stream
sequence at that resize. Until output produced after that sequence arrives,
Codex restore skips retained snapshots and filters pre-resize stream chunks.
This avoids reopening on old-width TUI output, such as full-width separators
drawn before the current xterm column count was known. The fix deliberately does
not rewrite xterm history, patch line art, or draw an overlay; the source of
truth remains the daemon PTY stream after the current size has been applied.

In 2.4.12, resize truth also checks the kernel PTY, not only the daemon's cached
`current_cols/current_rows`. A same-size resize request may be a repair request:
if xterm reports `110x50`, the daemon cache says `110x50`, but `get_size()` on
the PTY master still returns `120x36`, the daemon must send the resize again and
record `resize_cache_mismatch_repair`. Remote SSH launch waits on the initial
default `36x120`/`24x80` size long enough for xterm's first fit and follow-up
repair resize to reach the PTY before the remote `resume-codex` command starts.
This keeps Codex from drawing its TUI against stale columns after update or GUI
restart.

In 2.4.13, the write bridge treats Codex synchronized-output repaint regions
(`ESC[?2026h` ... `ESC[?2026l`) as xterm-native animation frames. If one PTY
read contains several complete repaint regions and the discarded prefix contains
only frame separators, title changes, cursor/erase/style controls, or a short
status gap, Yggterm keeps the newest region and drops the redundant intermediate
regions before calling `term.write`. If real scrollback or normal text precedes
that repaint suffix, it is preserved and only the redundant suffix is collapsed.
That is an emulator scheduling optimization, not a terminal rewrite: normal
text, newlines, prompts, scrollback, and meaningful visible output outside the
frame run must not be dropped. Sustained inline-status animation also cools from
the active animation frame budget to the sustained and long-running frame
budgets after the initial smooth window, so long Codex background work does not
keep WebKit hot for minutes.

In 2.2.18, xterm `onScroll` events are treated as ambiguous unless there is a
real user scroll signal. xterm fires the same event for output-driven viewport
movement, so while terminal input is hot or a write flush is in flight, that
event must not switch the host to `UserScrollback`. Wheel, PageUp, and explicit
scroll probes still set user intent. Command output such as `/status` therefore
keeps following the prompt after the bytes render instead of leaving the visible
viewport on old transcript rows.

In 2.2.24, hot-update readiness accepts a narrower live-daemon PTY case: a
remote Codex surface may be prompt-ready with an empty `cursor_line_text` when
the mounted xterm host is fed by `daemon_pty`, has real Codex prompt/output
bytes, has scrollback/current-buffer evidence, exposes visible cursor geometry,
and has no transport, loading, transcript, generic-idle, or overlay markers.
This is not permission to bless arbitrary retained prose. The source must still
be daemon-owned PTY truth, and focus/input must be proved separately through
app-control before typing into the live session.

In 2.2.54, restored focused terminals run a throttled prompt-follow repaint even
when the input-policy update is otherwise a no-op. The failure mode was a
retained xterm host whose buffer, viewport, and focus state were already
correct, but WebKit/xterm's visual cursor layer stayed on an older row until the
first typed byte triggered the input path. The fix reuses the same
`scrollLiveCursorIntoView(true, ...)`/`term.refresh()` path as real typing, but
only for focused prompt-follow hosts, never for explicit user scrollback, and
with a small per-mounted-terminal cap so the repair cannot become an idle repaint
loop.
This is a renderer-settle repair, not an alternate cursor renderer. The same
release also treats `WARN ignoring stale yggterm daemon for current app version`
as internal transport noise: it can be emitted by version-handoff probes, but it
must never become PTY application content or a prompt-ready cursor row.
The same class includes `terminal session not found: local://`,
`terminal session not found: remote-session://`, and
`terminal session not found: codex-runtime://`. Those messages are transport
failures. They must be stripped from replay, reported through observability, and
used to recover/recreate the daemon PTY before xterm.js is allowed to accept
input for the row.

In 2.2.61, a remote Codex prompt-only hot-update surface is explicitly not a
ready terminal. A surface that only shows the Codex input row/footer can come
from the PTY, but it does not prove the restored Codex TUI frame, scrollback,
prompt background, or current input contract. The shell therefore keeps input
disabled, refuses to mark the attach ready, and after the hard-fail window uses
one `--force-remote` restart for that session. This rule is Codex-specific:
plain remote shell prompts can still be ready when the PTY is running and
visible.

In 2.2.62, the same rejected-surface rule covers stale remote Yggterm socket
errors and generic Codex title-card output during preserved-owner handoff. A
wrapped line such as `Error: connecting to .../server-*.sock` may appear in the
xterm buffer after a multi-version daemon bridge fails; app-control and the
mount loop must classify it as terminal transport failure, not meaningful
Codex output. After the recovery attempt and hard-fail window, the shell may
restart that one remote Codex runtime with `--force-remote` rather than letting
the stale owner linger indefinitely.

The same pass tightened frame detection: large cursor-home redraws with hidden
cursor are full-screen TUI frames, not Codex inline status animation frames.
They stay on the xterm.js path, but they are frame-budgeted so WebKit does not
burn CPU on background or recovery redraw floods. Small carriage-return
`Working`/status updates remain eligible for the fast inline animation cadence.

In 2.4.7, that inline animation cadence is still xterm-native but it is no
longer allowed to poll faster than the active xterm write frame budget. The
jojo live incident showed that a broad shell render loop was one bug, but even
after fixing that loop, Codex's `Working` wave could keep WebKit hot if Rust
continued draining the PTY at the generic 60 ms active-output cadence. Active
inline-status reads now share the same bounded frame cadence as writes, so
Yggterm does not create extra xterm/WebKit wakeups just to observe animation
frames. That cadence is lossless: it changes when bytes flush, not which bytes
xterm receives.

In 2.6.0, ordinary user input is batched at the xterm.js boundary before it is
sent to Rust and the daemon. The batch window is deliberately tiny, and Enter,
interrupt/control keys, protocol replies, and cleanup paths flush immediately.
This keeps fast Codex typing from creating a Rust IPC hop for every character
while preserving terminal ordering. App-control exposes `pending_input_bytes`,
`input_batch_flush_count`, `last_input_batch_length`,
`last_input_batch_flush_reason`, and `last_input_batch_at_ms`; health checks
must use those counters instead of treating raw `data_event_count` as daemon
delivery proof. A no-later-echo alarm is valid only when the visible prompt
layout is sparse or broken. It must not disable input on a current prompt row
whose cursor line is visible and whose blank rows below the cursor are within
the prompt-layout budget.

The same release accepts current daemon screen snapshots for blank retained
Codex hosts even when the quiet session has not produced new post-resize bytes.
That relaxation applies only to daemon-owned screen/PTY truth that already
matches the prompt-ready filters. Stored retained replay remains strict so old
prose or transcript snapshots cannot become an alternate terminal renderer.

In 2.6.9, the jojo fast-typing incident proved that both the Rust write bridge
and embedded xterm script must be lossless. The live Codex prompt accepted all
typed bytes, but the screen showed missing letters because older synchronized
repaint frames were collapsed before xterm parsed them. The fix is not a prompt
overlay and not a redraw trick: Rust pending writes now concatenate and flush in
order, and the JavaScript high-volume helpers return the original payload.
`active_recovery_pty_snapshot` is observability/recovery evidence only; it must
not be promoted as an authoritative terminal replay source.

In 2.6.38, the jojo 2.6.37 launch proof exposed a retained-recovery retry
storm: host health marked a retained remote xterm ready, then a transient blank
sample milliseconds later reopened the same terminal and remounted xterm. That
made the terminal eventually look healthy but burned CPU during startup and
looked like another manual switching-pass fix. The retained-fault invalidation
gate now treats transient post-ready retained faults inside the settle grace as
telemetry, not another recovery. If the blank surface survives beyond the grace
window, normal retained-fault recovery still runs.

The stable Rust ownership boundary for this is
`crates/yggterm-shell/src/terminal_write_policy.rs` for classification and
`crates/yggterm-shell/src/terminal_write_bridge.rs` for staging. New terminal
write throttling or CPU-budget work belongs there first, not in shell chrome,
session metadata, or telemetry observers.

In 2.2.37, the jojo diagonal line-stack failure was traced to PTY line
discipline, not xterm painting. Remote Codex bridge PTYs were put into raw mode
to make input pass through immediately, but raw mode also disabled `ONLCR`
output translation. Codex/TUI frames that contained bare `\n` then reached
xterm as line-feed-without-carriage-return, so rows advanced vertically while
staying at the previous column. The bridge now keeps raw input but restores
`opost onlcr` output processing, and captured snapshot emission normalizes bare
LF to CRLF before replay. This is still terminal-native truth: Yggterm is
fixing PTY bytes before xterm sees them, not drawing a prompt or line-art
overlay after the fact.

In 2.2.38, forced terminal restart exposed a separate stale-host boundary. A
daemon runtime restart resets the terminal stream sequence, but the mounted GUI
host may still hold a cursor from the previous runtime and a retained xterm
buffer. When a read returns a lower cursor than the GUI last used, that is now
classified as a runtime restart boundary. The daemon treats the stale cursor as
an initial attach so the new runtime's initial chunks are available, and the GUI
clears/remounts the xterm host before accepting new readiness. This prevents a
healthy restarted PTY from being displayed on top of old broken pixels.

## VS Code Reference

VS Code does not appear to solve this by drawing a separate prompt overlay.
Useful reference files in `microsoft/vscode`:

- `src/vs/workbench/contrib/terminal/browser/xterm/xtermTerminal.ts`: constructs
  `Terminal`, sets theme/font/cursor/window options, writes raw data via
  `raw.write(data)`, and resizes with `raw.resize(cols, rows)`.
- `src/vs/workbench/contrib/terminal/browser/terminalResizeDebouncer.ts`:
  separates visible resize from expensive PTY column resize. Small buffers and
  explicit resizes are immediate; larger visible terminals resize rows first and
  debounce columns.
- `src/vs/workbench/contrib/terminal/browser/terminalProcessManager.ts`: sends
  dimensions to the PTY only after the process is ready, and treats resize as a
  process contract.
- `src/vs/platform/terminal/node/ptyService.ts`: keeps a headless xterm buffer
  for persistent terminal state and resizes that buffer with the PTY.

The lesson for Yggterm is conservative: feed correct bytes and identity into
xterm, keep resize ordering deterministic, and let xterm paint.

## Local Lab

Use `tools/xterm-lab/` to reproduce renderer behavior outside the app:

```bash
python3 -m http.server 8765
```

Open `http://127.0.0.1:8765/tools/xterm-lab/`.

The lab loads the vendored `assets/xterm/` scripts, supports DOM and canvas
renderers, and includes fixtures for:

- plain prompt with default background
- truecolor prompt background
- jojo's observed `49m` bottom-row clear payload
- partial repaint after resize
- full repaint after resize
- Codex-style inline status animation

The diagnostics panel samples public xterm buffer state plus private render
dimensions when available. It is a lab, not product code.

## Implementation Rules

- Terminal appearance identity must match the effective xterm theme, not the
  outer shell theme. A light shell with a dark terminal theme must export dark
  terminal identity.
- Terminal color identity must also match the effective xterm theme. Daemon OSC
  4/10/11 replies should use the synced xterm foreground, background, and ANSI
  palette, not a generic built-in dark/light palette.
- Remote and restore launch paths must pass the same terminal appearance
  contract as fresh local starts. A daemon default of light is acceptable only
  before the app has synced the effective terminal identity.
- A daemon-owned remote Codex runtime may only be bridged when the runtime's
  recorded launch command matches the attaching terminal's dark/light identity.
  If a dark terminal asks to attach and the existing runtime advertises light
  identity (or vice versa), Yggterm must recreate/restart that daemon-owned PTY
  through the normal server path instead of reusing the stale process.
- Existing preserved processes cannot have their environment corrected without
  restart. For hot update, session survival wins. The UI should treat stale
  runtime identity as observable state, not as permission to kill the runtime.
  Use `yggterm-headless server terminal restart <session> --force-remote` only
  when the session is known safe to replace.
- Startup prewarm may use a cheaper no-snapshot path for background remote
  sessions, but the restored active remote terminal should seed retained daemon
  PTY scrollback when that history is safe to replay. A prompt-ready active
  restore with `scrollback_expected=true` and `base_y=0` is still a scrollback
  diagnostic, but it must not block interactivity by itself. It becomes a hard
  terminal problem when the current prompt is not readable/input-ready, when an
  unsafe retained replay path is involved, or when a scroll probe proves the user
  cannot move through expected xterm scrollback.
- User scrollback is an explicit xterm intent. Pending prompt-follow repairs
  from input-focus or resize settle must be cancelled when the user scrolls
  away from the bottom; otherwise the terminal appears scroll-locked even though
  xterm briefly moved. Returning to bottom may restore prompt-follow, but only
  through xterm viewport state, not by force-redrawing terminal text.
- A retained remote terminal with ready history may still require retained
  daemon replay when the current xterm buffer collapses. Ready history is not a
  replay suppressor after app-control reports a prompt-only, empty, stale, or
  no-current-input-row surface.
- App-control state must expose enough evidence to distinguish "xterm did not
  paint a background" from "the PTY never sent a background attribute".
- Canvas renderer validation cannot rely on `.xterm-rows`. Use xterm buffer
  cells, renderer dimensions, screenshot pixels, and app-control probes.
- A resize fix is not valid if it only fixes geometry metrics. It must prove the
  visible prompt row, footer/status ordering, cursor position, and scrollback
  intent after settle.
- Avoid repeated full-canvas refresh loops. `term.refresh()` is a recovery tool
  after explicit resize/viewport repair, not a render-loop heartbeat.
- Keep write batching lossless for control sequences. Bracketed paste markers,
  cursor movement, erase-line, alternate-screen transitions, and inline status
  rewrite batches must preserve ordering.
- Synchronized repaint batching may drop superseded repaint frames when the gap
  between frames is control-only. Preserve real scrollback/output before the
  burst and keep the latest complete `?2026h`...`?2026l` frame; do not collapse
  alternate-screen transitions or frames separated by visible text.
- Scroll controls are shell controls, not terminal rendering. The YggUI scroll
  controller may appear when the user is intentionally far from prompt-follow,
  but it must only call xterm viewport APIs such as `scrollToLine` or
  `forceXtermViewportY`. It must not draw terminal text, cursors, prompt
  backgrounds, or line-repair layers.
- Active-session switch repaint is a bounded xterm recovery action. It may clear
  texture atlas state and refresh the mounted xterm viewport after selection
  changes, but it must be keyed to activation and exposed to app-control.
- Retained live terminals must keep daemon PTY/runtime truth while hidden. On
  KDE/Linux, switching away may unmount the hidden xterm renderer to keep the
  WebKit budget low. Switching back must remount from daemon PTY/retained
  history without shell-owned placeholder text, prompt overlays, or session
  identity substitution, and it must preserve `base_y`/scrollback when daemon
  retained history exists.
- A "switching pass" must never be required for correctness. If app-control or
  host health sees an active retained remote host with an empty xterm surface
  while the daemon runtime is present or has output, the shell must keep that
  fault reason on the retained-fault recovery attempt, use the fast
  empty-surface watchdog, and immediately ask the daemon to ensure/read the PTY.
  The fallback may remount the xterm bridge, but it must still replay daemon PTY
  bytes rather than shell-owned placeholder text.
- During remote-resume recovery, the xterm host may stay mounted so geometry,
  probes, and lifecycle hooks remain stable, but it must be visually hidden
  until the surface is current or recovery has failed. A mounted host with stale
  retained bytes is not terminal truth; app-control must expose host opacity,
  visibility, and pointer-events so smokes can prove stale retained xterm text is
  not visible during the startup leak window.
- For explicit Keep-Alive remote sessions, daemon-side output classifiers such
  as prompt-only, stale retained text, saved-session mismatch, or launch-spec
  mismatch are recovery signals only. They must not restart a still-running
  runtime or send a second resume command. The terminal can remain gated and
  recovering, but the user's live PTY is the source of truth.
- Startup prewarm must obey the same rule before xterm is even attached. If a
  reachable old daemon reports the kept remote runtime key, the current daemon
  must route terminal read/write/resize through that preserved owner and keep
  xterm recovering. It must not create a fresh SSH attach just because its local
  `TerminalManager` is empty.
- A preserved owner that returns `terminal session not found` for the runtime
  key is no longer a valid terminal source, even if its status endpoint still
  lists the key. Remove that owner, recover through the current daemon/saved
  transcript path, and keep app-control input disabled until fresh PTY bytes
  make the xterm surface current again.
- App-control text truth must prefer xterm buffer reads over DOM `innerText`.
  xterm's DOM renderer inserts style and measurement nodes, so host-level DOM
  text can contain renderer internals that are not PTY output. Likewise, a
  read-only app-control snapshot taken while the document is unfocused must not
  classify a prompt as input-gated just because the probe observed disabled
  input; recovery should only react to gated input when the terminal or document
  is actually focused.

## Next Fixes To Apply

1. Add a deterministic app-control probe that compares active xterm theme
   appearance with the terminal identity exported to the live runtime.
2. Add a resize smoke that triggers a real viewport resize, waits for settle,
   and asserts that the prompt row is below any `Worked for...` footer/status
   line rather than being followed by stale footer text.
3. Keep a live Codex proof in the release bundle that shows raw prompt-row
   styling contains a `48;...m` background attribute after a fresh dark-identity
   launch, with `software_canvas_input_line_overlay_present=false` and
   `xterm_input_line_decoration_present=false`.
4. Keep `tools/xterm-lab/` fixtures current as small reproductions for every
   terminal nuance that becomes release-blocking.
