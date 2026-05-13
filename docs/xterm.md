# xterm.js Terminal Notes

This document is the working contract for Yggterm's xterm.js path. Keep it in
sync when terminal rendering, resize, PTY identity, app-control probes, or smoke
tests change.

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

## Interaction Contracts

Primary selection is not the clipboard. On Linux-style middle-click paste,
Yggterm records non-empty `term.getSelection()` text as
`window.__yggtermPrimarySelection`, then handles middle mouse down by calling
`term.paste(text)`. That keeps bracketed-paste handling inside xterm.js and
sends the resulting bytes through the normal terminal input path. The fallback
path is xterm's core `triggerDataEvent`, then a direct terminal input event; none
of these read from `navigator.clipboard`.

Right-click on a terminal is an xterm surface event that opens Yggterm's row
context menu for that terminal session. The terminal host captures the native
`contextmenu` event, prevents WebKit's browser menu, records the host-local
counter/coordinates, and sends a `context_menu` terminal JS event through the
same Rust bridge as input/resize/clipboard. The shell then opens the existing
session context menu; it does not create a second menu implementation inside the
terminal DOM.

Active terminal write coalescing must stay below the threshold where typed input
or Codex's `Working` animation feels stepped. Background terminals can keep the
large CPU-saving frame budget, but the active visible terminal defaults to a
160 ms write frame and the inline status-animation hot path stays at the smaller
animation budget. App-control and latency smokes reject active visible terminals
above 220 ms.

Layout-driven resize is not user scrollback. Titlebar auto-hide hover,
fit-addon row changes, visible-paint refits, and resize-observer bursts arm a
short prompt-follow layout guard. While the host is in `PromptFollow`, scroll
events from those programmatic changes are suppressed as user-scroll evidence
and the viewport is forced back to the active buffer bottom after the fit
settles. If the user has explicitly entered `UserScrollback`, the same layout
changes preserve that reading position instead of snapping to bottom.

Regression coverage:

- `terminal_eval_script_supports_xterm_primary_selection_middle_paste`
- `terminal_eval_script_bridges_xterm_right_click_context_menu`
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

In 2.2.6, the daemon answers only OSC 10/11 default foreground/background color
queries from the PTY stream before GUI attach, using the same dark/light terminal
identity that Yggterm exports to the child process. The daemon strips those
query bytes from retained output and writes xterm-style `rgb:rrrr/gggg/bbbb`
responses back through the PTY writer. This closes the Codex startup race where
Codex asked `/dev/tty` for the terminal background before the xterm.js viewport
was mounted, timed out, cached `None`, and then emitted `49m` instead of a real
prompt background attribute. This is terminal-emulator protocol handling, not a
shell overlay or prompt decoration.

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
terminal launch keeps using the available managed or PATH binary until an
explicit foreground ensure/refresh path is used.

In 2.2.45, retained replay gained an explicit unsafe-skip state for resize
recovery. A large cursor-addressed retained snapshot is not safe to replay into
xterm, but if the current xterm buffer already has a prompt-ready cursor row,
Yggterm must treat that existing PTY surface as the truth instead of marking the
skipped snapshot as expected scrollback. App-control exposes
`retained_replay_unsafe_skip_prompt_ready` and
`retained_replay_rejected_visible_text`; a prompt-ready unsafe skip may keep the
terminal interactive, while the same base-y-zero state without that diagnostic is
still a retained scrollback-loss failure.

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
- Remote and restore launch paths must pass the same terminal appearance
  contract as fresh local starts. A daemon default of light is acceptable only
  before the app has synced the effective terminal identity.
- Existing preserved processes cannot have their environment corrected without
  restart. For hot update, session survival wins. The UI should treat stale
  runtime identity as observable state, not as permission to kill the runtime.
  Use `yggterm-headless server terminal restart <session> --force-remote` only
  when the session is known safe to replace.
- Startup prewarm may use a cheaper no-snapshot path for background remote
  sessions, but the restored active remote terminal must seed retained daemon
  PTY scrollback. A prompt-only active restore with `scrollback_expected=true`
  is not terminal-ready.
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
- Scroll controls are shell controls, not terminal rendering. The YggUI scroll
  controller may appear when the user is intentionally far from prompt-follow,
  but it must only call xterm viewport APIs such as `scrollToLine` or
  `forceXtermViewportY`. It must not draw terminal text, cursors, prompt
  backgrounds, or line-repair layers.

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
