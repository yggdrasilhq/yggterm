# Changelog

This file tracks user-visible changes in `yggterm`.

## Unreleased

## 2.1.106

- Restored remote live-session startup prewarm as a background load path instead of skipping it entirely. Remote live sessions are eligible to attach before they are selected, while the startup path still refuses saved-transcript prefill so Codex TUI surfaces do not repaint `USER:`/`ASSISTANT:` artifacts.

## 2.1.105

- Kept startup live-session restore within the daemon latency budget by deferring remote live-session ensure work out of daemon startup prewarm. Restored remote terminal runtimes remain visible to the app, but `Status`, `server-list`, and latency checks are not blocked by slow remote terminal attach work.

## 2.1.104

- Fixed post-update daemon observability when legacy socket aliases point at the same current daemon socket: `server-list` and latency checks now dedupe symlink aliases before probing, preventing a hot-restart/install check from blocking on repeated aliases.

## 2.1.103

- Removed saved Codex JSONL transcript prefill from live remote terminal restore, so retained/live Codex sessions wait for or restart the real PTY instead of painting `USER:` / `ASSISTANT:` transcript artifacts into xterm.
- Treat transcript-browser and role-labeled transcript text as terminal-surface failures in app-control state, even when the xterm host is mounted, input-enabled, and has scrollback.
- Prewarm restored live terminal sessions by default on daemon startup, not only the active session, so Live Sessions stay attached in the background instead of repeatedly entering visible recovery when selected.
- Tightened the terminal smoke harness and focused CI gates to reject transcript artifacts during `/status`, retained session switching, and app-control readiness checks before accepting a terminal as interactive.

## 2.1.101

- Preserved rich TUI glyph rendering by decoding PTY output as a continuous UTF-8 stream instead of lossy-decoding each read chunk, preventing split box-drawing and progress glyphs from turning into replacement characters.
- Tightened the `/status` terminal smoke so it fails on duplicate visible Codex status panels, replacement characters, shell fallback, or pre-polluted retained user sessions before accepting a viewport as fixed.
- Added the regression-first workflow rule to `AGENTS.md` so future UI/runtime fixes update the harness or CI gate before the runtime patch.

## 2.1.100

- Fixed retained remote Codex restore/input after stale runtime reuse: `resume-codex --require-existing` now goes through the daemon ensure/restart checks before bridging, so shell-prompt or interrupted runtimes are restarted instead of being exposed as an input-enabled xterm.
- Tightened terminal readiness observability so a previously ready terminal open attempt is demoted when later app-control state reports a non-ready surface, and the smoke harness now fails that contradictory state.
- Kept terminal input proof on app-owned paths by default. The xterm probe uses `--ctrl-u`/`--enter`, saved transcript transport errors no longer mask visible-echo failures, and desktop-wide keyboard synthesis is blocked unless explicitly opted in for a local unsafe run.
- Prevented inactive retained xterm hosts from accepting hidden/collapsed fit geometry, preserving remote scrollback and avoiding remount-style recovery when switching away to a local session and back.
- Exposed skipped terminal fit decisions through app-control as `last_skipped_fit` and added `resize-window` to `yggterm-headless` so resize/viewport hang proofs can use the same app-owned control path as other terminal smokes.

## 2.1.99

- Fixed remote retained-session switching so xterm replays daemon-retained scrollback instead of remounting from a visible-only vt100 snapshot.
- Added app-control scrollback expectation fields and smoke assertions that fail when `probe-scroll` is merely accepted but the viewport does not move.
- Reduced repeated full-canvas repaint nudges during terminal resize/replay paths to keep viewport drag and session switching latency bounded.
- Seed remote retained Codex sessions from saved JSONL transcript prefill when the live multiplexer snapshot is empty, avoiding shallow prompt-only restores after clean daemon starts.
- Strip remote attach protocol markers such as `__YGGTERM_ATTACH_READY__` before vt100 parsing, retained replay, app-control text samples, and smoke assertions.
- Added `server terminal write <session>` as the app-owned terminal input path and disabled desktop-wide synthetic typing unless explicitly opted in, preventing jojo/KDE input from leaking into other apps.

## 2.1.98

- Restored native Codex TUI color richness by disabling xterm.js minimum-contrast palette rewriting for terminal surfaces.
- Switched the xterm font stack to installed monospace faces first so WebKit does not resolve a missing Iosevka family to a proportional fallback on jojo.

## 2.1.97

### Fixed

- fit xterm.js directly from the live terminal host geometry so Codex/TUI surfaces expand from the old 80x24 bootstrap canvas to the full viewport, restoring rich status panels and prompt layout on large Yggterm windows
- coalesce terminal resize observer events and rate-limit PTY resize notifications so dragging the terminal viewport no longer forces a daemon/TUI redraw for every intermediate DOM resize
- add a focused terminal viewport resize smoke that fails if the wrapper grows while the xterm canvas stays stale, covering compact and wide window sizes with app-control screenshots

## 2.1.96

### Fixed

- hydrate GUI relaunches from the active Linux desktop environment when they are started from SSH/app-control, so KDE Wayland restart handoffs pick the transparent Xwayland window profile instead of falling back to square opaque corners
- hide the transparent KDE/Xwayland window until its first configure/shape pass completes, preventing the launch-time square-corner flash before KWin settles the rounded frame
- expose hydrated desktop environment fields in app-control desktop identity output to make future KDE launch/corner regressions easier to diagnose

## 2.1.95

### Fixed

- keep KDE/X11 startup windows hidden through the WebKitGTK child-window bootstrap, preventing the transient square-corner flash before Yggterm applies its rounded native shape
- size xterm.js rows from fractional viewport geometry with a bottom guard, and report sub-pixel row-fit overflow through app-control so clipped prompt/footer rows fail smoke coverage

## 2.1.94

### Fixed

- treat an externally installed direct-update version as a pending GUI restart, so a running app can restart into the active `install-state.json` executable instead of trying to overwrite a running headless helper

## 2.1.93

### Fixed

- keep active Codex/TUI terminal output on the real xterm.js canvas instead of switching visible sessions to the lossy low-power text overlay, fixing corrupted repeated text such as incremental `Booting` fragments and false “input-enabled without a prompt-ready surface” readiness failures
- return a stable `session_path` from app-control terminal creation even while the server snapshot is still catching up, so Codex interaction and latency smokes can deterministically probe the newly created terminal
- keep managed Codex CLI refresh/check work out of the foreground terminal-launch path, so creating a Codex session uses the available binary immediately while release/update refreshes continue in the background
- hide transparent KDE/X11 windows until the first native rounded-corner shape pass succeeds, preventing the visible square-window startup flash before the retry shape timer settles

## 2.1.92

### Fixed

- fix remote Codex resume launch wrapping so the local tty-size settle prelude is not executed as a command name, preventing `exec: __yggterm_initial_tty_size=...: not found` from leaving restored sessions input-disabled
- show Linux desktop windows only after the first corner-shape preparation pass and retry the native shape faster during startup, reducing the transient square-corner artifact on KDE/X11
- make app-control terminal probes deterministic in background jojo smokes, including exact printable keyboard synthesis, a visible daemon-write fallback, and no low-power TUI overlay for ordinary prompt traffic

## 2.1.91

### Fixed

- render active high-volume alternate-screen TUI frames through the low-power terminal surface instead of repainting the xterm canvas on every frame, cutting jojo active-TUI CPU below the smoke budget while keeping input routed through the live terminal
- bound retained paint-repair refreshes for frame-like terminal output so TUI redraws do not multiply into repeated full-canvas refresh work

## 2.1.90

### Fixed

- recover remote terminal open attempts when the xterm surface becomes interactive after an earlier timeout, clearing stale “Remote Terminal Needs Attention” toasts instead of leaving input disabled
- stop a retained non-prompt host snapshot from re-poisoning a remote attach after an attach-ready marker has already been observed, preventing Codex sessions from getting stuck after delayed welcome-frame redraws
- drop offscreen protocol-only/TUI control chatter without forcing xterm render probes, reducing WebKit CPU and typing latency while inactive TUI sessions continue running
- let `probe-select` use xterm buffer text in the default canvas renderer, so xterm smoke latency/readability checks no longer misclassify canvas terminals as missing rows

## 2.1.89

### Fixed

- restore runtime-owned remote/Codex terminal attaches from the daemon current-screen snapshot instead of a partial retained xterm replay tail, preventing duplicate bare Codex prompt markers and stale prompt fragments when reopening live sessions
- extend the xterm embed smoke with an active-host Codex prompt layout check so restored/live sessions fail the gate when a retained prompt artifact reappears

## 2.1.88

### Fixed

- keep remote Codex resume bridges waiting when the daemon snapshot is only the bare Codex prompt/footer, so restored sessions repaint with the full Codex frame instead of getting stranded in a prompt-only failed state
- preserve fresh Codex welcome frames through xterm write coalescing while MCP/status lines stream in, and flag tall prompt-only Codex surfaces as app-control failures instead of treating them as ready

## 2.1.87

### Fixed

- retry the remote Codex bridge current-screen repaint after early control-only output, so fresh or resumed Codex TUIs do not get stranded in a sparse prompt-only redraw while the daemon already has the full screen
- stop fresh SSH-backed Codex terminals from writing the local Codex scaffold into the xterm host while the real remote bridge is still loading
- add bridge trace points for initial-screen snapshot readiness, success, and give-up paths to make future jojo redraw incidents diagnosable from telemetry
- seed requested SSH targets into temp-home Linux smoke runs so remote Codex timeline checks exercise the intended machine instead of depending on the user's live profile

## 2.1.86

### Fixed

- keep the compact titlebar usable in very small windows by moving crowded controls into the overflow menu and preserving the search field width
- keep the compact Settings rail opaque and inside the content area, with a shorter search placeholder that fits narrow titlebars
- restore the Linux always-on-top toggle by clearing keep-below before applying keep-above
- keep Settings terminal-theme dropdowns keyboard-filterable and scrolled into view when opened near the bottom of the rail
- replace Settings zoom steppers with numeric text inputs that reject non-digits and clamp values to supported zoom bounds
- synchronize daemon PTY resize requests with xterm fit geometry after compact window resizes, preventing stale row/column sizes from corrupting prompt rendering
- repaint resumed remote Codex sessions from the daemon current-screen snapshot before replaying retained bridge chunks, preventing sparse or scrollback-shaped TUI restores
- fill the thin edge fringe in the 512px app icon so the panel icon renders with a cleaner border
- extend the jojo/KDE smoke coverage for compact chrome and Settings controls

## 2.1.85

### Fixed

- recover local startup-restore terminal mounts that get stuck behind a stale same-session surface request, preventing blank selected terminals and high render churn after restart/handoff races
- make terminal ensure attempts bounded for local sessions so daemon IPC stalls clear attach state instead of leaving input disabled indefinitely
- strengthen the UI latency smoke with a readiness gate that rejects blank xterm hosts before measuring typing latency

## 2.1.84

### Fixed

- make `probe-type --per-char` dispatch character-level keyboard events without artificial per-character sleeps, so latency smoke reports the app/input path instead of its own pacing
- expose `server app update <check|restart>` through `yggterm-headless`, matching the direct launcher path used for server/app-control commands

## 2.1.83

### Added

- add a UI latency smoke that measures app-control state, sidebar rows, search, panel switching, and visible terminal typing latency
- extend `probe-type --per-char` so terminal typing proof reports xterm-buffer visible echo timing instead of trusting canvas-empty DOM text

## 2.1.82

### Fixed

- remove unkept update-restored remote sessions from `Live Sessions` after a fresh remote scan proves their runtime is no longer live
- trace stale temporary remote live-session pruning so panic reports can distinguish recoverable keep-alive rows from stale loading rows

## 2.1.81

### Fixed

- refuse to bridge remote Codex runtime stdio through stale-version daemons, preventing 2.1.80 clients from hanging on live sessions still owned by older 2.1.78 daemons
- make remote terminal resume timeouts clear attach/request state and latch the terminal-open failure instead of leaving sessions in an indefinite loading state
- stop persistent no-progress loading and attention toasts from running an infinite progress animation while a session is already stuck

## 2.1.80

### Fixed

- show live local Codex/LiteLLM sessions under `Live Sessions` even when their active runtime is backed by a stored `.codex` or `.codex-litellm` transcript path
- keep idle stored Codex transcript rows historical until explicitly opened, then move the resulting live runtime into `Live Sessions` without duplicating the row in the stored tree
- improve light-theme live-session close affordance contrast and extend the live-session tree smoke to reject duplicate live rows outside the live group

## 2.1.79

### Fixed

- stop fresh SSH-backed Codex sessions from seeding the terminal viewport with the local Codex scaffold before the real remote runtime produces output
- classify local Codex scaffold text as stale/non-meaningful terminal output, so app-control and the shell no longer treat it as a loaded or interactive session
- extend the remote Codex spawn timeline smoke to fail if scaffold text appears in any sampled host surface

## 2.1.78

### Fixed

- accept freshly spawned remote Codex welcome/prompt surfaces as live interactive terminals, so new SSH-backed Codex sessions stop falling into a false "Remote Terminal Needs Attention" timeout after the prompt appears
- add app-control and smoke coverage for `server app terminal new --machine-key <machine> --kind codex`, including sub-1s/1s/2s/5s/ready/post-timeout screenshots and state captures

## 2.1.77

### Fixed

- accept even more aggressively truncated Codex permission-selector tails that start inside the `auto-reviewer` line, using the stable `Full Access`/confirm/escape markers to clear remote startup resume without waiting for a timeout

## 2.1.76

### Fixed

- accept truncated lower-half Codex model-permission setup tails during remote resume, so retained startup surfaces that only expose `Full Access` plus the confirm/escape hint still clear the remote-attention gate and keep input enabled

## 2.1.75

### Fixed

- let retained, runtime-running Codex model-permission setup screens finish remote terminal resume without waiting for an attach-ready visual deadline, so the false "Remote Terminal Needs Attention" toast clears and input re-enables while the permissions selector is visible

## 2.1.74

### Fixed

- let Codex model-permission setup menus complete remote terminal resume even when the selector sits mid-screen with many blank rows below the hidden cursor, keeping input enabled without weakening stale transcript detection

## 2.1.73

### Fixed

- recognize Codex model-permission setup menus as interactive terminal surfaces, so new remote Codex sessions do not disable input or show a false "Remote Terminal Needs Attention" timeout while the permissions selector is visible

## 2.1.72

### Fixed

- classify split canvas transcript-browser surfaces as interactive when the header and footer land in different app-control text samples, so responsive remote Codex sessions are no longer reported as not prompt-ready

## 2.1.71

### Fixed

- accept retained Codex transcript-browser terminal surfaces as interactive when the remote runtime is running, so hot-restarted/restored sessions do not stay stuck behind a stale resume notification while the visible transcript UI is usable

## 2.1.70

### Fixed

- accept focused, input-enabled Codex transcript-browser surfaces in app-control readiness checks so Yggterm reports responsive remote resumes as interactive instead of flagging a false prompt-ready problem

## 2.1.69

### Fixed

- keep live Codex transcript-browser resumes interactive after acceptance by trusting explicit resume-ready paths, marking the terminal open attempt ready, and blocking stale slow/timeout notifications from re-poisoning input

## 2.1.68

### Fixed

- treat a live, runtime-running Codex transcript browser as an interactive terminal surface instead of a stale retained transcript, so remote resume clears the restoring notification and re-enables input

## 2.1.67

### Added

- replace the misleading stored-session preview on empty startup with a start page that offers recent sessions, a new Codex session, a local terminal, and SSH connect actions

### Fixed

- clear stored-only active-session snapshots during startup/background sync so launching with no live sessions does not show `xterm.js backend reserved` or a stale remote transcript as the active workspace
- expose start-page visibility and recent-session rows through app-control DOM state for deterministic smoke coverage

## 2.1.66

### Fixed

- clamp xterm canvas row fitting on WebKit/KDE so the restored Codex prompt row cannot fall below the visible terminal host after scroll/redraw
- schedule a bounded repaint repair for the first retained-session and bulk terminal writes, avoiding half-painted restored terminals until the user manually scrolls
- expand app-control terminal diagnostics with row/column, viewport/base, cursor overflow, canvas layer, fit-guard, and retained-paint-repair fields so redraw and prompt clipping incidents are directly observable

## 2.1.65

### Added

- add `yggterm-headless server app desktop-identity` as a read-only KDE/app-control incident report for pinned launcher ids, desktop file fields, live client app ids, and update-handoff environment flags

### Fixed

- keep direct-install KDE launches grouped under the pinned `dev.yggterm.Yggterm` launcher during update handoff instead of creating an isolated app id from `YGGTERM_ALLOW_MULTI_WINDOW`
- allow a runtime-running prompt-ready remote terminal surface to complete resume, clear the attention toast, and re-enable input instead of timing out while visible content is already loaded
- switch Linux desktop entries to the canonical theme icon name and refresh the icon edge pixels so the installed app icon no longer shows a pale jagged border
- add focused CI regression tests for KDE desktop identity and the remote-resume prompt readiness path

## 2.1.64

### Fixed

- keep active full-screen TUI terminals such as `htop` on the real xterm canvas instead of replacing them with the low-power text overlay, preventing garbled rows and needless redraw churn while the user is watching the terminal
- route direct-install `yggterm --version` through the active headless sibling so version probes do not touch the GUI binary or live desktop state
- fold the panic-management monitor scenarios into `yggterm-headless server monitor`, stop shipping the separate mock CLI binary, and expose `yggterm-headless` directly from direct installs and `.deb` packages

## 2.1.63

### Changed

- update the direct SHA-2 dependency to sha2 0.11.0 so the release carries the latest available hashing stack alongside the Dioxus 0.7.6 desktop runtime refresh

## 2.1.62

### Changed

- update the desktop runtime stack to Dioxus 0.7.6, Wry 0.55.0, Tao 0.35.0, WebKitGTK 2.0.2, reqwest 0.13.3, rusqlite 0.39.0, png 0.18, and refreshed transitive dependencies while preserving Yggterm's local Dioxus/Wry observability patches

## 2.1.61

### Added

- add daemon-backed `yggterm-mock-cli` control scenarios for panic reports, listing reachable versioned servers, hot-restarting daemons with a replacement headless binary, waiting for a session to load, probing daemon latency, repeated interval monitoring, and refreshing managed Codex CLI tools
- add a daemon `hot_restart` request that persists restart-safe state, acknowledges the client, exits the current listener, and spawns the requested replacement daemon

### Changed

- run a best-effort managed Codex CLI refresh/check during release packaging, with `YGGTERM_RELEASE_CODEX_REFRESH=0` as the opt-out

## 2.1.60

### Fixed

- prevent retained remote terminal text from being treated as an interactive prompt, so stale Codex output cannot clear the resume toast or re-enable input before a prompt-ready surface is visible
- add a remote-side non-blocking scan lock and keep “scan already in progress” out of the Python fallback path, preventing repeated remote refreshes from piling up SSH scan processes
- stop Linux legacy-daemon cleanup from treating a bare versioned socket as live runtime ownership while still preserving daemons with active bridges or terminal runtimes
- bound daemon request socket IO so status/runtime probes against stale daemons fail instead of blocking scan and cleanup paths indefinitely

## 2.1.59

### Fixed

- keep terminal input responsive when a child PTY stops accepting writes by moving blocking PTY writes off the daemon request thread and failing fast under sustained input backpressure
- keep daemon `ping`, `status`, and terminal writes responsive while remote machine refreshes run by queueing slow SSH scans outside the daemon runtime lock
- coalesce queued remote machine refreshes, time out hung remote scans, and cool down the shell retry loop so one offline or slow SSH target cannot spawn repeated background scans
- let fresh local shell terminals become interactive as soon as the real prompt is visible, instead of holding input disabled behind a prompt-only readiness loop
- keep the active terminal input-armed when passive side rails are open or the window-focus observer lags, while still avoiding forced autofocus unless the terminal actually owns focus
- scope document-level clipboard paste handling to the active terminal host so settings/sidebar paste events cannot leak large payloads into a running terminal
- keep the direct-install launcher path compatible with terminal focus/type/scroll/select app-control probes by exposing those actions through `yggterm-headless`

## 2.1.53

### Fixed

- stretch the approved Yggi mascot icon to fill the packaged 512px canvas with only a small safety margin, so KDE, Windows, and macOS launchers no longer render it as a tiny padded tile
- keep Settings text fields owned by the field being edited instead of re-focusing or leaking keystrokes into the active terminal, and expose right-rail field/menu geometry in app-control so this path is now smoke-tested
- make Interface/Terminal zoom numbers directly editable and replace the native terminal-theme select with an in-rail menu that stays inside the settings panel
- budget all high-volume full-screen/TUI terminal frames, including remote-resume frames before the overlay dismisses, so WebKit does not spin hot on jojo after `htop`/Codex-style output
- refine the terminal/settings smoke so it proves settings typing, viewport reclaim, blank-Enter spinner behavior, hidden-cursor TUI recovery, render budgets, and WebKit child RSS on KDE/X11
- document stale binary execution as destructive in `AGENTS.md`, requiring future version checks and live-install investigations to use canonical metadata or isolated homes instead of launching archived GUI artifacts

## 2.1.52

### Fixed

- prune stale direct-install version directories during install and desktop integration, preventing archived old GUI binaries from being accidentally executed and rewriting modern session state
- route `yggterm server ...` launcher invocations through the active `yggterm-headless` sibling, while keeping `server app launch` on the GUI path, so app-control/status probes cannot start an unintended desktop shell
- make stale versioned `yggterm-headless` binaries hand off to the active installed headless binary before opening the session store or daemon state
- write daemon state through a temporary file and preserve `server-state.previous.json` before overwrites, giving future live-session state regressions a recoverable last-good copy instead of a single point of failure
- prefer a live remote session over a stale stored preview when both share the same `remote-session://...` path in Terminal view, avoiding blank surfaces and session/view contract violations after partial state recovery

## 2.1.51

### Fixed

- throttle high-volume full-screen terminal output through a low-power TUI render path, keeping jojo/KDE idle and active TUI WebKit CPU within budget instead of leaving `WebKitWebProcess` hot after `htop`-class output
- restore xterm newline semantics with `convertEol: true` and add a sidebar-switch regression assertion for horizontal line drift, so spaces and table output keep their columns after switching sessions
- wait for a real Codex prompt before marking local agent terminals ready, preventing banner-only Codex surfaces from leaving stale resume notifications or half-mounted prompt regions
- keep remote Codex "New Session" actions on the daemon-owned `server remote start-codex` path and preserve that launch contract through restart/scan hydration instead of opening a plain SSH shell
- refresh the app icon assets around a centered friendly terminal prompt mark while keeping the design rule face-free for packaged KDE, Windows, and macOS assets

## 2.1.50

### Fixed

- prevent active stored remote-session rows from taking the local hot-terminal focus path, which could corrupt a restored SSH Codex session into `LiveLocal` and block app-control terminal open with a session/view contract violation
- promote stored remote previews through the remote `LiveSsh` resume path when Terminal view is requested, keeping the session in `Live Sessions` and preserving the remote runtime handoff
- repair legacy remote-session snapshots that already carry the impossible `LiveLocal` source, so v2.1.49-corrupted update state normalizes back to `LiveSsh` on the next launch
- skip redundant synchronous remote binary probing when a healthy cached remote launch expression is already present, avoiding unnecessary SSH work on terminal open
- retire superseded same-home GUI clients on the same display before the replacement desktop shell reaches GTK/Dioxus launch, so old v2.1.45-v2.1.49 windows cannot keep the canonical KDE app id and leave the updated client registered but invisible

## 2.1.49

### Fixed

- scan all reachable same-home versioned daemon sockets when detecting live remote Codex runtimes, so update handoff can see sessions still owned by an older daemon instead of reporting them dead
- bridge `server remote resume-codex --require-existing` directly to the older daemon that still owns `codex-runtime://<session-id>`, preventing duplicate Codex runtimes after a direct-install restart
- relax the app-control session/view contract so a restored `LiveSsh` terminal row is allowed to reconnect while the latest remote scan is temporarily stale
- add regression coverage for old-daemon remote-runtime bridging and stale remote-scan recovery during update handoff

## 2.1.48

### Fixed

- make `yggterm server status` read-only again, so status checks cannot spawn a replacement daemon, sweep older daemons, or rewrite live-session state while diagnosing an update
- stop daemon startup from immediately rewriting restored `server-state.json`, preserving recoverable live-session records until an explicit open/focus/update action owns the transition
- keep reachable older versioned daemon sockets alive during startup cleanup, so a freshly installed client no longer sends `shutdown` to the daemon still holding live terminal runtimes
- treat same-home terminal runtimes and `server remote resume-codex` bridge processes as active ownership in the Linux daemon sweep, preventing update probes from killing live Codex/SSH sessions
- prevent unknown/dev-channel launches from repairing the user direct launcher, while still detecting old launchers that fell back to a repo `target/debug/yggterm` binary
- add regression coverage for read-only status, reachable legacy sockets, remote-resume bridge detection, stale debug launcher detection, and old-daemon terminal-runtime preservation

## 2.1.47

### Fixed

- preserve remote live-session records across update restart even when the remote scan is late, so SSH Codex sessions stay in `Live Sessions` as resumable runtime sessions instead of disappearing after relaunch
- persist manual SSH session renames into the remote session metadata mirror, preventing restart-time title hydration from reverting renamed sessions back to generated or original labels
- keep KDE on the canonical `dev.yggterm.Yggterm` app id after update handoff and terminate superseded same-home GUI clients, so pinned task grouping does not split into a second Yggterm icon
- reclaim terminal focus after clicking a live/session row, including already-selected rows, so typing, spaces, Delete, paste, and scroll stay owned by the xterm viewport instead of the sidebar
- show the Live Sessions busy spinner for active Codex sessions whose terminal status line says `Working`, without reviving the stale blank-Enter/activity-spinner regression
- add a focused KDE/X11 smoke that switches sidebar sessions and proves terminal focus, literal spaces, Delete-key ownership, and scrollback after the switch
- center the app icon's warm `>_` prompt mark, regenerate the packaged PNG asset, and lock the prompt-first icon rule into `DESIGN.md`

## 2.1.46

### Fixed

- move terminal image-paste deduplication into the Rust shell path shared by browser paste events, shortcut fallback, and app-control paste requests, so a delayed duplicate event from one physical `Ctrl+V` cannot stage a second image or paste a second path
- extend the keyboard clipboard smoke to force the delayed duplicate paste path and reject any second `.png` prompt insertion or second `Image Staged` notification
- stop re-upserting the `Resuming Remote Terminal` toast once a terminal session already has a ready open attempt or completed visual resume, preventing live Codex surfaces from staying dimmed behind stale resume state
- isolate hidden retained terminal canvases with strict containment, z-order, and offscreen transforms so inactive live Codex hosts cannot visually bleed into the active terminal surface
- default the embedded xterm surface to the canvas renderer, with `YGGTERM_ENABLE_XTERM_CANVAS=0` retained as a field-test escape hatch, so fast terminal output does not burn the WebKit DOM renderer path
- refresh the vendored xterm fit/canvas assets as a matched set and load the canvas addon after opening the terminal, preserving readable WebKitGTK rows while keeping the canvas renderer active
- keep explicit terminal-focus reclaim active across transient KDE/Xwayland focus-observer false events while still clearing it on app-control background, so automation and viewport reclaim do not drop input before paste/typing

## 2.1.45

### Fixed

- make direct-install update restarts launch the replacement client as a waiter on the old GUI PID, so KDE can keep the canonical `dev.yggterm.Yggterm` app id and pinned task grouping instead of spawning a second Yggterm icon
- keep terminal `Delete` owned by the active xterm host/helper textarea, preventing stale sidebar focus state from opening a close/delete modal while the user is editing terminal input
- gate terminal `Ctrl+V`/`Cmd+V` so browser paste events and native clipboard fallback cannot both stage the same image from one physical paste
- preserve deliberate Codex/session titles against passive remote-preview and generated-cache hydration paths that were still promoting generated titles into user-visible row names
- replace the generic app icon with the warmer `Yggi` sprout-and-prompt mark, regenerate the canonical PNG asset, and document the brand/icon identity rules for KDE, Windows, and macOS packaging checks
- extend the local terminal UX checklist for duplicate paste, terminal Delete ownership, update-restart KDE grouping, and cross-platform icon identity proof

## 2.1.44

### Fixed

- launch the updated KDE/X11 desktop client with an isolated app id when an older live client from the same Yggterm home is still registered, avoiding hidden 10x10 activation windows after direct-install updates
- keep update-restart window close guarded by the force-exit watchdog so the old GUI cannot remain alive indefinitely while owning the canonical desktop app id
- stop stale-daemon version recovery from sending a daemon shutdown request; the new release now removes only the current-version socket alias and leaves the older daemon and its live PTYs alone
- require daemon socket alias reuse to match the current Yggterm version, preventing a newly installed client from binding itself to an older versioned daemon socket

## 2.1.43

### Fixed

- lock the stability-first GUI design rules into `DESIGN.md` and `docs/stability.md`, including the keyboard-proof contract for slash-command terminal regressions
- protect all recoverable live sessions during direct-install update restarts without mutating the user's explicit Keep Alive choices
- preserve deliberate session titles and summaries when passive preview/open hints arrive, so selecting a row no longer spends LLM budget or rewrites saved copy
- route native `Ctrl+V` / `Cmd+V` through the desktop clipboard path for text and image paste instead of browser clipboard fallbacks
- make context menus, Live Sessions close buttons, and keep-alive markers theme-aware and observable in app-control
- avoid synchronous settings writes on the titlebar auto-hide toggle and demote unfinished terminal-recipe drag persistence behind an explicit feature flag
- split app-control terminal input around `Ctrl+C` and require `/status` proof through real keyboard injection, avoiding the dropped-character path seen in Codex prompts
- expose `terminal_hosts[].text_tail` in full/basic app-control snapshots and update the terminal smokes so bottom-of-viewport `/status` panels are proven from state plus screenshot

## 2.1.42

### Fixed

- default KDE sessions with Wayland and Xwayland available to the X11 desktop backend unless explicitly overridden, avoiding the compositor/restart path that was still crashing Plasma after update restarts on jojo
- use a KDE/X11 transparent shell profile for direct launches so the rounded Yggterm frame no longer leaves small white square artifacts at the four window corners
- keep the direct-install desktop app id stable during update handoff, so KDE pinned icons and task grouping do not split into a second-class smoke/update icon
- make the Always on Top titlebar control set and clear KDE/X11 `_NET_WM_STATE_ABOVE` and `_NET_WM_STATE_STAYS_ON_TOP`, with app-control proof for both states
- close the Live Sessions Keep Alive context menu immediately after toggling and prove the keep marker changes without leaving the menu stuck open
- keep plain local-terminal input from showing an optimistic busy spinner after blank Enter while preserving real remote/activity indicators
- release terminal input focus when the app window is backgrounded/minimized, cutting idle terminal work on KDE while keeping refocus fast
- enforce xterm row whitespace and cursor contrast contracts so terminal spaces, TUIs, resize/redraw, and light-theme cursors stay readable in the embedded surface
- keep titlebar search typing literal slash characters while focused and keep inline rename ownership stable through slow real keyboard input
- preserve SSH machine labels separately from per-session titles so opening a session no longer mutates the machine name in the sidebar
- extend the KDE release gate with corner-pixel sampling, always-on-top X11 state proof, keep-alive menu proof, hidden-cursor TUI proof, slash search, rename, renderer whitespace, spinner, idle CPU, cleanup, and Plasma PID stability checks

## 2.1.41

### Fixed

- scope stale-daemon cleanup to the matching `YGGTERM_HOME` and skip live daemons with active clients, so old helper windows and smoke-owned clients cannot kill a newly updated KDE session daemon from another home
- trace spawned daemon child exits and cleanup decisions in the server/app-control event stream, making KDE restart and shutdown regressions diagnosable from proof bundles instead of process-list guesses
- keep `Live Sessions` as the top sidebar group while making fresh live terminals runtime-only by default; only explicitly kept sessions persist across cold starts, with a visible keep-alive marker and close confirmation
- preserve the user's sidebar visual bookmark during rename/title refresh churn, including kept-alive live-session labels after title enrichment
- reduce the terminal activity spinner and live-session snapshot nudge loop after Enter/input events, so blank Enter does not show a busy state and idle focused terminals settle quickly
- pin the Dioxus desktop dependency edge to the vendored 0.7.3 build used by the KDE desktop patches, avoiding accidental broad updates that bypass local desktop fixes
- extend the terminal UX smoke coverage for keep-alive toggles, Live Sessions close affordances, blank-Enter spinner behavior, terminal typing, and idle CPU proof

## 2.1.40

### Fixed

- keep stored local Codex transcript paths out of the promoted `Live Sessions` group, even after an explicit terminal open, so old `.codex/sessions` rows no longer turn into a wall of duplicate live terminals
- reject stored Codex transcript paths in the server/app-control live-session contract and extend the Linux/KDE smoke proof to require `stored_tree` placement, no hidden title/summary generation, and no live close affordance on stored rows
- repair Linux direct-install desktop metadata by making the canonical `dev.yggterm.Yggterm.desktop` entry visible and hiding the legacy `yggterm.desktop` entry, so KDE task grouping/pinning uses the same app id the running window reports
- harden the Linux X11 native-shape window profile so KDE keeps rounded shell corners without switching the normal path back to a higher-CPU transparent window
- defer startup background refreshes for managed CLI and remote metadata so launching Yggterm and opening a first local shell stay quiet instead of competing with terminal interaction
- disable the daemon-side passive background-copy chore by default, requiring `YGGTERM_ENABLE_BACKGROUND_COPY_CHORE=1` before it can spend CPU or generate hidden title/summary copy
- keep KDE close, terminal lifecycle, and idle-CPU proof in the release gate for this regression class, including Plasma PID stability and visible `×` affordance coverage

## 2.1.39

### Fixed

- stop the Linux direct-install integration path from forcing global Plasma shell/cache refreshes during self-update, and use the KDE-safe detach/hide close path when restarting into an installed update
- keep inline session rename commits from expanding hidden ancestors or autoscrolling to the duplicate `Live Sessions` row, preserving the user's sidebar visual bookmark
- restore the selected live-session close `×` contrast in light theme and expose its text/color in app-control so the smoke suite rejects blank close circles
- reject malformed title-generation fragments such as `How Use Skills Discovery The`, use the same low-signal gate for transcript and explicit context generation, and extend the regeneration smoke to fail low-quality title/summary output
- add focused KDE proof coverage for v2.1.38 field-test regressions: live-session tree/close affordance contrast, titlebar regeneration quality, titlebar rename, context-menu rename, and Plasma PID stability

## 2.1.38

### Fixed

- make inline session rename usable under KDE: the current title is selected once, typing overwrites instead of appending, Ctrl+A stays owned by the input, Enter commits, click-away commits, and row interaction no longer expands neighboring folders while renaming
- let the active titlebar session title and the title inside the title/summary popover enter inline rename directly, while keeping the popover chevron/action area available for title/summary details and explicit regeneration
- make explicit title/summary regeneration show immediate queued/completed feedback and prove it does not run hidden copy generation on passive row selection
- harden app-control snapshots and keyboard injection for rename, titlebar, and KDE degraded DOM paths so the smoke suite can prove selection ranges, click targets, Enter commits, corner rounding, and sidebar cursor state without guessing
- keep sidebar rows on a normal pointer cursor while idle, slightly reduce default sidebar label density, and preserve stored Codex row targeting so opening a row does not accidentally expand or activate a neighbor
- add release proof coverage for the v2.1.37 KDE notes: combined titlebar/context rename smoke, stored Codex/sidebar cursor smoke, terminal lifecycle smoke, idle CPU thresholds, rounded-corner pixel sampling, and a 180-second Plasma/kwin live watch

## 2.1.37

### Fixed

- stop cold-start sidebar selection from auto-opening the first stored Codex transcript, so a freshly updated KDE launch does not resume a session or spawn Codex before explicit user action
- open stored Codex transcript rows through the terminal path by default when they support a PTY, while keeping stale remote-scanned transcript rows out of the promoted Live Sessions group
- expose sidebar row cursor styles in app-control and use a normal pointer cursor for idle rows, so draggable sessions do not advertise drag as the primary click action
- add deterministic Linux/KDE smoke coverage for stored Codex session opening, no hidden copy generation, no startup auto-open, sidebar cursor contracts, and Plasma PID stability

## 2.1.36

### Fixed

- restore rounded KDE/X11 shell corners while keeping the Linux opaque window profile, eliminating the white corner artifacts seen after update restarts
- reduce idle CPU burn from the desktop shell by backing off app-control, live-session, background refresh, terminal-read, and WebKit memory polling loops when the app is idle
- make long `YGGTERM_HOME` paths work by moving overlong Linux daemon sockets to a short per-home runtime socket while keeping state in the real home directory
- add a Linux idle-CPU smoke and persist root-window corner pixel proof alongside screenshots, so KDE corner artifacts and fan-level idle regressions become release gates

## 2.1.35

### Fixed

- disable passive title/precis/summary generation by default and expose a copy-generation start counter in app-control, so selecting or opening sessions can be proven not to spend LLM budget unless the user explicitly regenerates copy
- add a focused Linux/KDE smoke check for the selection copy budget, alongside session/view contract proof, so future releases fail if a row selection starts hidden title or summary work
- preserve inline-rename and titlebar-search observability under KDE DOM snapshot timeouts by exposing the controlled rename value in app-control and adding a tiny action fallback for rename/menu/delete/search proofs

## 2.1.34

### Fixed

- clear copied-profile daemon socket symlink chains before shell startup pings or aliases any endpoint, so KDE/profile-copy launches stop reconnecting to the real-home daemon after updates
- keep stale remote-scanned Codex sessions out of `Live Sessions` unless the remote daemon proves an active runtime, opening old sessions as rendered previews instead of relaunching terminals or spending LLM budget
- preserve active folder/session rename inputs across background snapshots and commit inline renames only on Enter, stopping mid-typing selection resets and lost characters
- restore terminal focus/input after search, settings, titlebar, live-session close, and hot-session switching paths so local shell typing stays immediately interactive after UI navigation
- refresh stale or system-managed Codex CLI installs on local Codex launch/resume while suppressing npm update/audit/fund chatter in managed sessions
- expand the Linux/KDE smoke proof to cover titlebar search, active title/summary copy, live-session close confirmation, drag-to-folder persistence, folder rename/collapse, explicit title/summary regeneration, local runtime health, hot switching, real Codex `/status` typing, cleanup, and Plasma PID stability

## 2.1.33

### Fixed

- reject stale post-update daemons whose reported server version does not match the launched app, and stop old daemons from aliasing future versioned sockets back to themselves
- make Linux/KDE direct and smoke-owned multi-window launches use isolated GTK application ids so they do not collide with an already-running user Yggterm instance
- default Linux shells to opaque chrome and require explicit opt-in for live blur/transparency, preventing KDE/Wayland windows from bleeding through the Yggterm surface when compositor blur is unavailable
- harden the jojo X11 smoke launcher so it carries the real desktop session environment, records app-owned launch visibility honestly, and still proves terminal lifecycle behavior with Plasma PID stability

## 2.1.32

### Fixed

- harden KDE/Xwayland app-owned launches by making `server app launch --wait-visible` prove a visible app-control state instead of only client registration
- stop the KDE terminal close probe from leaving a half-closed Yggterm GUI alive, and fail the smoke if the probe panics, survives close, or forces a direct-shell fallback
- make the vendored Dioxus desktop init path tolerate duplicate init delivery, avoiding the `Virtualdom should be set before initialization` panic seen during KDE close-path proof

## 2.1.31

### Fixed

- keep live sessions promoted at the top of the sidebar with visible close affordances while avoiding duplicate local live rows in the stored local tree
- fix inline rename and titlebar search typing so focused inputs no longer reselect/collapse to the last typed character during real keyboard entry
- keep session titles and summaries stable on selection; automatic background generation is no longer triggered just by selecting a row, while explicit title/summary regeneration still works
- make folder-scoped new sessions and dragged live-session recipes persist under the chosen workspace folder instead of falling back to the root tree
- harden the Linux second-X11 smoke suite for live-session close, drag-to-folder persistence, titlebar search typing, hot local terminal switching, and real Codex `/status` typing with screenshot proof
- add a `Codex Extra Args` setting and apply it to Codex/Codex-LiteLLM launch commands, so direct installs can pass flags such as sandbox policy consistently
- write release checksum sidecars with portable artifact basenames instead of build-machine absolute paths, including native macOS and `.deb` packaging

## 2.1.30

### Fixed

- keep Windows direct-install desktop integration quiet and complete by passing normal `C:\...` paths, not `\\?\...` extended-length paths, to the Start Menu shortcut and GUI launcher creation code

## 2.1.29

### Fixed

- keep local terminal startup and typing off slow cleanup/background paths by fast-pinging the current daemon before legacy socket cleanup, removing GUI-startup cleanup work, and preserving background copy cooldowns instead of repeatedly scanning the same summary target
- stop stale PID-targeted app-control requests from being handled by a later GUI client, so remote smoke/watch cleanup requests no longer poison the next launch
- keep KDE live-session retention bounded on X11 and Wayland while preserving the promoted `Live Sessions` group and close affordances for active sessions
- make direct-install packaging more complete across platforms: Windows archives now include the mock CLI companion, Windows resource/icon generation fails soft when cross tools are missing, platform packaging prefers `cargo-zigbuild` when available, and the POSIX installer launchers avoid GNU-only `find -printf`/`sort -V`
- launch plain Windows local terminals into the real interactive `cmd.exe` prompt instead of a quoted-command error screen, and make the Windows install smoke reject that failure class from screenshot/app-control text
- harden Linux live-watch proof so a run with no successful app-control state sample is a failure instead of a false green

## 2.1.28

### Fixed

- stop manual live-session renames from being overwritten by the next background snapshot, so the sidebar title, active title, and persisted title/summary stay stable after rename and after switching away to another live session and back
- preserve multiple live shell sessions of the same kind during persisted-state restore instead of collapsing them by `(kind, host, prefix)`, so local and same-machine remote terminals stop disappearing out of the live tree during snapshot/restore churn
- keep synthetic live-session group expansion state intact across tree restores, so rename and snapshot updates stop collapsing the `Live Sessions` section while the sidebar is being refreshed

## 2.1.27

### Fixed

- reserve the titlebar lane while auto-hide is revealed, so the restored search, title/summary, session chip, and window controls push the viewport, sidebar, and right rail down together instead of floating over the content surface
- stop applying the Linux native rounded-window shape mask on Wayland, so KDE/Wayland close-path runs avoid the unstable X11-style shape/input path that could coincide with Plasma restarts and square-edge artifacts
- harden the Linux jojo proof runners with a real revealed-titlebar push assertion, targeted `--only-check` smoke runs that can skip unrelated session bootstrap, and plasmashell PID churn detection while avoiding the unstable remote-SSH `spectacle` path

## 2.1.26

### Fixed

- stop live restored remote sessions from reissuing background `server remote generation-context` SSH work on every active-session hydration tick, which was leaking file descriptors on KDE/Wayland until Yggterm died with `Too many open files` and could destabilize Plasma
- harden the Linux live desktop watcher with owned-client FD tracking, `generation-context` helper counts, and a `--reuse-existing-home` mode, so compositor-crash regressions now fail against the real restored-profile launcher path instead of only passing staged temp-home runs

## 2.1.25

### Fixed

- ship the macOS `Yggterm.app` bundle with the headless and mock-cli companions, and fail the shared bootstrap smoke unless the installed app can create a real local terminal, so release bundles stop opening into the `serializing daemon request` dead-end
- keep the wide titlebar utility actions inline on macOS and Windows instead of collapsing them into the overflow menu at ordinary laptop widths, with the shared cross-platform smoke now failing on that regression directly
- launch Windows direct installs and background helpers as real GUI/background processes instead of visible console-style helpers, so Start/search launches feel first-class and the smoke now rejects stray visible console windows after terminal creation
- harden the remote Windows and macOS runners around proxy-jump transport, multiline PowerShell execution, and real terminal-host readiness, so cross-machine proof exercises the same installed builds and workflows that manual testing uses

## 2.1.24

### Fixed

- restore the shared titlebar search shell to a real flexing field instead of a collapsed `26px` pill, so Windows and macOS fresh installs keep the same full-width idle chrome as Linux
- keep the focused search overlay and attached titlebar modal parity backed by the tightened shared smoke contract, so the broken active-search shape and `+` menu seam regressions stay caught before release

## 2.1.23

### Fixed

- force the shipped Windows `yggterm.exe` onto the GUI subsystem at link time and validate that in CI/release packaging, so Start Menu and search launches stop opening the old console-hosted second-class app path
- add an in-process macOS cached-display screenshot fallback ahead of `screencapture`, and reject blank PNGs from every macOS screenshot backend, so remote proof can capture the live app without collapsing on transparent zero-byte-equivalent window captures
- add explicit `--proxy-jump` and `--ssh-port` routing controls plus stale-asset version guards to the shared Linux, macOS, and Windows remote smoke runners, so cross-machine GUI proof no longer depends on brittle per-host `~/.ssh/config` aliases or silently re-tests old `dist/` builds
- tighten the shared titlebar search/modal smoke around focused-field geometry and attached overlay visibility, so the broken active-search pill shape and missing attached modal now fail deterministically instead of slipping through visual review

## 2.1.22

### Fixed

- reject macOS CoreGraphics window captures that silently decode to an all-zero PNG and fall back to the next capture backend, so remote proof stops accepting a black `Yggterm` window as if it were a valid screenshot
- harden the shared app-control bootstrap plus the remote macOS and Windows runners to fail on blank screenshot evidence instead of only checking that a file exists, which closes the false-green proof hole that was hiding macOS capture regressions

## 2.1.21

### Fixed

- keep an empty direct-install home visible as a real `local` root instead of rendering a zero-row sidebar, so fresh Windows and macOS installs no longer boot into a blank, unusable shell before any sessions exist
- refresh Windows direct installs with a stable `Yggterm.vbs` GUI launcher and point the Start Menu shortcut at it, so Start/search launches stop showing Yggterm as a console-hosted second-class app
- tune the shared native macOS window builder with a traffic-light inset and matching titlebar leading inset, so the unmaximized native controls sit cleanly inside the unified chrome instead of looking clipped or misaligned
- harden the remote Windows live-app and macOS smoke helpers around noisy SSH/PowerShell and attach-only control paths, so stale control transport bugs stop masking the real platform regressions

## 2.1.20

### Fixed

- replace the fragile macOS `screencapture -l` screenshot path with an app-owned CoreGraphics window capture first, while keeping `screencapture` only as a fallback, so remote app-control proof can capture the real native mac window without dying on host privacy/window-server edge cases
- expose the winning macOS screenshot backend through app-control, mirroring Windows backend reporting so cross-platform smoke runs can tell whether they captured the real native window or only fell back to a legacy path

## 2.1.19

### Fixed

- keep the empty `local` workspace root visible on fresh homes instead of collapsing the sidebar to zero rows on first launch, which was making Windows and macOS look blank and unusable before any sessions existed
- stop routing the local background managed-Codex refresh through the daemon transport during GUI startup, so first boot no longer surfaces spurious `Codex Tool Refresh Failed` notifications from fragile local socket handshakes
- promote Windows GUI launches to a first-class desktop app by setting an explicit AppUserModelID, hiding the inherited console on no-arg GUI entry, embedding a real executable icon resource, and wiring the taskbar icon from the shared shell window builder
- flush the shared macOS shell surface into the native transparent titlebar when the window is not maximized, which removes the extra inset/shadow treatment that was distorting the traffic-light area
- harden the shared bootstrap, remote Windows smoke, and remote macOS smoke so zero-row sidebars and refresh-failed startup notifications are treated as release blockers instead of slipping through as “launch succeeded”

## 2.1.18

### Fixed

- move macOS onto the shared transparent-window startup profile instead of the opaque `non_linux` path, so the next dev builds can exercise native blur/unified-chrome behavior instead of hardwiring an opaque shell
- ship the mac manual-download app bundle under a lowercase `yggterm-macos-*.app.zip` asset name so the release workflow actually uploads it alongside the other platform artifacts
- harden the remote macOS and Windows smoke runners around real release assets: suppress PowerShell progress noise for Windows zip extraction, clean stale harness-owned mac temp clients before launch, send desktop notifications around mac automation, prefer bundle-first mac launches, and prove owned clients are gone after close instead of leaking background daemons
- tighten the shared bootstrap blur gate so a platform now fails when it claims live blur support but still comes up non-transparent with no backdrop blur, and surface the real mac screenshot-capture failure instead of a misleading missing-file copy error

## 2.1.17

### Fixed

- add manual-download-friendly platform packages for the next dev releases: macOS now emits a real `Yggterm.app.zip`, while Windows now emits a `.zip` that keeps `WebView2Loader.dll` beside the shipped executables instead of relying on users to keep loose files together
- teach the remote macOS and Windows smoke runners to prefer those packaged artifacts, so cross-machine proof runs exercise the same bundle layouts users actually download instead of silently testing a nicer staging-only path

## 2.1.16

### Fixed

- harden remote Windows proof runs so startup system-error dialogs and fresh Application-log crashes are treated as release blockers instead of slipping through green screenshots
- stage macOS remote smoke launches as a real `Yggterm.app`, add native bundle icon generation for direct installs, and fail fast when the frontmost app name still leaks the raw artifact name instead of `Yggterm`
- stop cleaning up live macOS GUI clients as if they were stale Linux `/proc` entries, and move the native mac window onto a unified full-size transparent titlebar path in the shared shell layer

## 2.1.15

### Fixed

- restore local live sessions under both the local tree and `Live Sessions`, so prompt-ready local shells stop leaving an empty promoted group after restore
- keep the managed Codex tool refresh off the hot path after a successful install by persisting a refresh TTL and proving the skip path in perf telemetry
- tighten Linux WebKit memory pressure defaults so repeated same-client runs stay under the child RSS soak budget instead of drifting upward between smoke cycles
- harden the second-X11 smoke around context-menu delete recovery, maximized-start titlebar contracts, idle IO/render sampling, and X11 click-origin drift so the release gate catches real regressions without false reds

## 2.1.14

### Fixed

- add a real auto-hide titlebar contract on Linux, including hover-reveal, empty-lane drag, double-click maximize/restore, and matching right-rail motion so custom chrome behaves like a native workspace shell instead of a decorative header
- fix the `+` menu seam and adjacent title/summary chip styling so the active launcher popover keeps the same rounded visual contract as the rest of the chrome instead of collapsing into a hard edge
- harden Linux maximize/restore and rounded-corner shape handling so flush-shell chrome survives round-trips without GTK shape warnings or broken input regions
- cap repeated WebKitGTK memory growth with a document-viewer cache model plus memory-pressure settings, and block regressions with a same-client `WebKitWebProcess` RSS soak gate
- extend the second-X11 smoke bundle to prove titlebar hover behavior, sidebar entry/exit animation, modal parity, maximize layout truth, and renderer memory budgets before packaging

## 2.1.13

### Fixed

- stop restored stored terminals from re-scheduling duplicate startup bootstrap work while the retained host lease is still active
- keep titlebar search, settings fields, sidebar actions, and terminal reclaim from stealing focus from each other during live interaction, so click-drag selection and terminal refocus still work after opening settings
- restore the Linux flush-shell corner contract after maximize round-trips and lock the smoke to the real `10px` radius/root-window proof instead of a DOM-only check
- keep dark-theme terminal rows readable by overriding low-contrast inline row backgrounds in the xterm DOM theme bridge
- harden the codex smoke against screenshot/state prompt races and dispatch-coupled idle render samples while still blocking semantic churn, inactive-host input leaks, and excessive terminal I/O

## 2.1.12

### Fixed

- catch the white perimeter halo on the real root window by sampling outer edge bands, while treating XRDP-safe opaque shells as a distinct flush-window profile instead of a false transparent-window failure
- remove the Linux opaque-window halo by making the nontransparent shell frame sit flush to the window bounds instead of keeping transparent-mode inset, rounding, and shadow chrome
- recover stale local PTY runtimes for app-control send/paste flows and tighten the plain-shell smoke so a visible prompt is not considered healthy unless the runtime is still writable

## 2.1.11

### Fixed

- add a real live terminal-zoom smoke that proves zoom changes resize visible rows, preserve the retained xterm host, keep the session interactive, and restore cleanly back to 100%
- harden rounded-corner artifact detection with repeated root-window captures so transient startup flashes are recorded while persistent square-corner failures still block packaging
- apply a one-time Linux transparent-window reconfigure pulse after spawn so the fresh client comes up with stable rounded corners instead of needing a manual minimize/restore nudge

## 2.1.10

### Fixed

- catch flaky square-corner window artifact regressions from the real root window instead of letting them slip past cropped app screenshots
- harden the real `:10.0` terminal smoke so late-suite Codex TUI vitality checks use contrast-aware paintedness instead of brittle dark-pixel assumptions on the light theme
- keep the release gate honest with richer sidebar and terminal observability for focus ownership, shell-frame geometry, and renderer diagnostics while leaving the readable DOM terminal path as the default

## 2.1.9

### Fixed

- keep live-session titles and summaries consistent across the active surface, sidebar, and restored session memory instead of letting the same session render with conflicting metadata
- harden retained-terminal behavior across startup restore, hot-session switching, titlebar search/settings focus, and duplicate terminal bootstrap scheduling so live sessions stay responsive instead of drifting into reloads or launcher boilerplate
- restore native-feeling terminal chrome by fixing the `+` menu shell, block-cursor contrast/inversion behavior, and related xterm light-theme regressions that slipped through the previous dev builds
- strengthen the real `:10.0` smoke gate so it catches the repeated menu, cursor, metadata, hot-session, and bootstrap regressions before packaging while skipping external `codex-session-tui` config failures that are not Yggterm bugs

## 2.1.8

### Fixed

- make the theme editor apply changes live, remove the stale `Apply Theme` step, and preserve grain-driven shell chrome even when the theme has no custom color stops
- keep clipboard image paste failures out of the PTY surface, stage local clipboard images through the local path instead of a bogus localhost SSH upload, and strengthen the related smoke coverage
- harden the xterm embed smoke so theme-editor, clipboard-image, and Codex TUI vitality checks run together and fail on the exact regressions that slipped into the last dev build

## 2.1.7

### Fixed

- restore direct-install self-update on Linux by shipping `yggterm-mock-cli` in the GitHub release archives again, so curl-installed machines like `jojo` can actually advance to the latest published version instead of aborting during update extraction
- remove the unconditional Cairo dependency from non-Linux desktop targets and keep the CI release packager aligned with the direct installer payload, reducing cross-platform release drift and unblocking the follow-up packaging pass

## 2.1.6

### Fixed

- filter unrecoverable local/document pseudo-live sessions out of restore and persisted daemon state, so fresh debug launches stop reopening empty `Live Sessions` ghosts, blank terminal rows, and stale remote-failure toasts
- harden the embedded xterm selection contract by forcing non-selectable terminal rows/canvas on the live DOM nodes and proving `user-select: none` through app-control, so browser text-selection artifacts stop leaking into the terminal surface
- strengthen the fresh-local terminal smoke to fail on empty live-session groups, wrong sidebar placement, missing busy-spinner recovery, or browser DOM selection leaking into the mounted xterm host

## 2.1.5

### Fixed

- stop helper-style CLI commands like `server snapshot` and `--help` from accidentally falling through into desktop window launch, so debug and packaging runs do not leave stray Yggterm windows behind
- add a GUI-side daemon watchdog for long-running desktop clients, so older windows recover when their helper daemon disappears instead of silently losing terminal input later
- remove the synthetic cursor overlay and keep the native xterm cursor as the visible contract, fixing the light-theme cursor artifacts and the hidden-cursor/TUI corruption path
- harden the `/status` terminal smokes so they require a real live Codex runtime, reject shell fallbacks like `bash: /status`, and accept alternate-buffer hidden-cursor proof from restore counters when a transient frame is missed
- stop low-signal boilerplate from winning local live-shell title generation, and keep local live shells anchored under the local tree instead of drifting into a live-sessions-only state

### Added

- add an in-repo demo and changelog evidence structure under `docs/demos/`, `artifacts/demos/`, and `.agents/skills/` so preview fixes, automation work, and future `yggui` features can ship with proof bundles instead of hand-written release-note guesses

### Docs

- document the shared `yggui` changelog/demo pipeline direction, including proof bundle format, scene style, and release-page ingestion, so `yggterm` can act as the first reusable template for future YggdrasilHQ desktop apps

## 2.1.4

### Added

- add `scripts/live_mode_cycle_check.py`, an SSH-driven app-control harness that flips a live Yggterm window from terminal to preview and back again, captures screenshots at each step, and records the actual usable timings instead of relying on guesswork

### Fixed

- stop `SetViewMode(Rendered)` from forcing a synchronous remote preview refresh through the daemon, so preview switches stop turning into hidden SSH refresh work and become usable again in about half a second on jojo
- remove the remaining extra terminal wrapper styling and keep the terminal viewport as a single surface, so terminal mode matches preview mode instead of carrying a second shell frame
- disable blurred/translucent overlay effects for KDE Wayland safe mode across the shell, context menus, delete overlay, toasts, and drag ghost chrome, reducing the compositor pressure that was still destabilizing Plasma on launch
- keep startup background remote refreshes out of the visible preview/terminal mode cycle, so the release-candidate harness no longer reproduces the old notification cascade or 40-second terminal reattach path

## 2.1.3

### Fixed

- restore the titlebar session title and summary dropdown even when the active session metadata arrives ahead of the full active-session object, so the title/summary controls stop disappearing during live remote restores
- remove the extra VS Code Light+ terminal shell framing, including the blue-tinted outer border treatment, so the light xterm surface returns to a cleaner single-surface terminal view
- stop sending `exit` into restored remote Codex sessions during app shutdown, using `/quit` for `remote-session://...` terminals instead so repeated Yggterm test runs do not litter the active Codex transcript
- harden remote helper bootstrap and launch reuse so broken `~/.yggterm/bin/yggterm` installs recover automatically and startup no longer explodes into remote helper mismatch storms
- reuse per-machine remote launch metadata for sidebar/live restore flows instead of re-resolving the same SSH helper path over and over during startup and remote refreshes

## 2.1.2

### Fixed

- prioritize active remote terminal restore over background remote-machine and managed-Codex refresh work, so a relaunched live SSH/Codex session paints before sidebar and tool-update churn kicks in
- remove duplicate startup terminal remounts and hide the sidebar "Refreshing tree..." chip while terminal mode is already live, so relaunches stop bouncing the terminal surface and shoving the tree around for no user benefit
- trim the GNU Screen resume path for remote Codex sessions and keep terminal reads on the dedicated terminal worker path, reducing the visible lag between xterm bootstrap and first meaningful remote output
- cache remote `yggterm` binary resolution across startup bursts instead of re-probing the same host repeatedly, cutting needless SSH round-trips during startup and remote refreshes
- make perf, trace, and UI telemetry appends atomic so `perf-telemetry.jsonl`, `event-trace.jsonl`, and UI telemetry stay machine-readable under concurrent startup and background activity
- strengthen the built-in VS Code-style light terminal palette so Codex content reads with clearer separation instead of washing into a flat white terminal slab

## 2.1.0

### Added

- add `server app state`, `server app focus`, and `server app screenshot` as the first SSH-reachable YggUI app-control verbs, so a running desktop client can be inspected and driven through its own control plane instead of external desktop guesswork

### Fixed

- replace the Linux app screenshot path with a native WebKitGTK surface capture, so Yggterm can screenshot itself without depending on `spectacle`, `gnome-screenshot`, `import`, or DOM-to-canvas fallback code

## 2.0.29

### Fixed

- switch the light xterm theme to a VS Code-style light palette, so Codex terminal surfaces regain the expected input-region contrast instead of blending into a flat white canvas
- tighten sidebar row spacing and trim the adjacent tree icon size slightly, so dense remote/session trees read more like a workspace navigator and less like a roomy file browser

## 2.0.28

### Fixed

- stop biasing in-app toast notifications to the left when a right rail is open, so progress and clipboard notifications stay visually centered in the window
- collapse the preview header summary down to the stored precis once the preview body scrolls, so long summaries stop dominating the top of the session while reading deeper into the thread
- stop re-running sidebar active-row auto-scroll on every reactive update, which was forcing the tree back to the current session and causing the flicker/dancing bug while trying to browse elsewhere
- retune the light xterm palette toward a cleaner GitHub-style base so Codex composer surfaces regain visible contrast instead of blending into the terminal background

## 2.0.27

### Added

- add `server screenshot app [output_path]` so a live Yggterm window can capture itself on demand, letting remote debugging and support bundles include the actual in-app state instead of guessing from the desktop around it
- make the screenshot capture path cross-platform with Linux, macOS, and Windows native backends plus a frontend fallback, so the same tracing workflow can travel with Yggterm instead of depending on one host compositor setup

### Fixed

- centralize the shipped icon assets behind one canonical SVG-plus-generated-PNG workflow, so window chrome, desktop integration, and future `yggui` apps stop drifting onto different icon artwork again
- expose a reusable `yggui` window-icon loader from PNG bytes, so future apps using the shell layer can plug in their own icon without copying the decode boilerplate or falling back to stale raster assets

## 2.0.26

### Added

- add an always-on `event-trace.jsonl` probe stream under `~/.yggterm`, with timestamped GUI, daemon, remote-session, managed-cli, and UI-surface events that can be tailed live without attaching a debugger
- add `server trace tail`, `server trace follow`, and `server trace bundle --screenshot` commands so sluggish runs can be inspected remotely over SSH and bundled with recent perf telemetry, UI telemetry, daemon state, panic logs, and a best-effort screenshot
- mirror high-value UI telemetry events into the shared trace stream so slow tree, preview, and session-open flows can be correlated against the daemon-side work instead of guessing across separate logs

### Fixed

- rotate the event trace automatically once it grows past a safe size, so long-running dogfooding sessions keep probes enabled without the log itself becoming a new source of startup or IO drag

## 2.0.25

### Fixed

- scope remote Codex session discovery to the actual SSH login user's `~/.codex`, so a machine no longer advertises sessions from a different account that the selected SSH target cannot resume
- preserve `remote-session://...` restore identity for cached live remotes even when the scanned session has disappeared, so launch prep still takes the remote resume path instead of silently downgrading to the wrong attach flow
- fall back from `codex resume <id>` to the interactive `codex resume` picker when a saved remote session ID is gone, keeping the terminal alive instead of closing the SSH tab on a stale restore

## 2.0.24

### Fixed

- keep only `yggterm.desktop` visible in Linux menus while leaving `dev.yggterm.Yggterm.desktop` as a hidden compatibility entry, so KDE no longer has two equally named menu candidates fighting over icon resolution
- force KDE desktop caches harder during integration by clearing stale per-user sycoca/icon caches, rebuilding with `--noincremental`, and nudging Plasma to refresh the current shell after the desktop files change

## 2.0.23

### Fixed

- point both Linux desktop entries at the shipped SVG file directly instead of relying on icon-theme name lookup, so KDE panel and menu paths can render the same icon artwork that already shows up correctly in desktop grid views

## 2.0.22

### Fixed

- disable the crashing GTK accessibility bridge by default on Linux unless `YGGTERM_ENABLE_ACCESSIBILITY=1` is set, which keeps jojo-style KDE/Wayland launches from dying in `libatk-bridge` before the window appears
- add a `yggterm.desktop` compatibility entry alongside `dev.yggterm.Yggterm.desktop`, plus extra SVG icon copies, so KDE menu and launcher lookup has both the strict desktop-id path and the plain legacy name available
- make direct-install launchers and packaged wrappers carry the same Linux accessibility guard before GTK/WebKit boot, so launcher/menu starts behave like direct binary starts instead of crashing differently
- ship `yggterm-mock-cli` in release archives, direct installs, and the `.deb`, so native startup, daemon, and integration issues can be diagnosed with the same installed tool users have on their machines

## 2.0.21

### Fixed

- corrected the shipped PNG icon so the runtime window icon and KDE launcher icon finally match the canonical SVG artwork instead of showing the broken raster fallback
- aligned Linux desktop integration around the `dev.yggterm.Yggterm` desktop identity, including duplicate icon names for the desktop file and theme cache refreshes that KDE can actually resolve
- replaced the old hide-on-close desktop behavior with a real client shutdown path, so closing one live client no longer leaves a hidden stale process behind
- kept restart-into-update from tearing down the daemon in between client handoff, so a client restart does not unnecessarily kill running sessions
- added a lightweight `--version` / `-V` CLI path so launcher and diagnostic checks do not accidentally boot the full GUI

## 2.0.20

### Fixed

- replaced the direct-install Linux launcher with a stable wrapper that reads `install-state.json`, so stale symlinks can no longer leave the desktop entry or shell command pinned to an older broken binary after self-update
- made direct self-update hand desktop integration refresh to the freshly installed binary, instead of trusting the older running binary to rewrite launchers and icons correctly
- aligned the shipped PNG window/launcher icon asset with the canonical SVG source so the runtime icon and installed desktop icon stop drifting
- stopped remote scan helper commands from panicking on broken stdout pipes during startup and shutdown races

## 2.0.19

### Fixed

- normalize remote machine aliases during restore and connection so alternate host aliases map to canonical machine identities consistently, improving remote session continuity across reconnects.

## 2.0.18

### Fixed

- make Linux direct installs register the desktop launcher like a direct app instead of a distro package: stable `Exec` via `~/.local/bin/yggterm` and a stable absolute `Icon` path under `~/.local/share/yggterm/direct/`
- keep the theme/pixmaps icon copies as fallback, but stop relying on KDE theme-name resolution alone for the primary launcher icon
- publish release checksum sidecars consistently so direct self-update does not fail on missing `.sha256` assets
- only shut down Yggterm server sessions when the last live client closes, so closing one of multiple open windows no longer tears down the others
- treat Linux termination signals like a graceful close path too, so KDE panel/taskbar close does not bypass daemon shutdown semantics

## 2.0.17

### Fixed

- make remote Codex discovery resilient when an SSH target logs in as a different user than the one that owns the session archive, so default `~/.codex` scans still find machine-wide homes like `/home/pi/.codex`
- keep `yggterm-headless` robust for root-login remotes by letting the remote scan path fall back to real machine user homes instead of reporting a misleading healthy `0 sessions`

## 2.0.16

### Fixed

- make stale `offline` SSH machines refresh again on startup instead of being treated as already-known forever
- add a cooldown to automatic remote machine refresh retries so a bad host does not spin forever
- refresh Linux desktop integration more aggressively for KDE by installing the themed icon into `pixmaps/` and forcing both icon and desktop menu cache updates
- keep direct self-update installing `yggterm-headless` alongside `yggterm`

### Docs

- added a standalone product thesis in `PRODUCT_THESIS.md`
- rewrote the README opening to better explain the core user, pain, and wedge
