# Changelog

This file tracks user-visible changes in `yggterm`.

## Unreleased

## 2.8.6

- Session-preserving hot-update handoffs now work on managed (Direct) installs.
  Previously a handoff spawned the new-version daemon but it re-exec'd back to
  the *old* active version, so it bound the old socket and deferred to the live
  old daemon — the update silently didn't land. The handoff now promotes the
  managed install's active version to the update target before spawning, so the
  successor stays on the target version, binds its own socket, and adopts the
  preserved sessions. (The normal "check for updates" flow already did this; the
  gap was in direct/agent-triggered handoffs.)

- Sidebar rows now highlight the moment you press the mouse, instead of waiting
  for the release — clicking a session (especially the active/top Live Session)
  feels instant. The session still opens on release, and starting a multi-row
  drag keeps the existing selection.
- Hot updates now wait for agent CLI sessions to be idle before swapping the
  daemon, so an update never lands on top of a Codex/Claude Code turn in
  progress (and avoids throwing away a still-warm prompt cache). A session
  blocks the update while it shows `esc to interrupt` or produced output within
  the idle window (default 5 min, set `YGGTERM_HOT_UPDATE_IDLE_THRESHOLD_MS`;
  override entirely with `YGGTERM_HOT_UPDATE_IGNORE_IDLE_GATE=1`).
- Fix hot-update handoffs being reported as failed/deferred when they actually
  succeeded: with many live sessions the daemon took longer than the 10s client
  timeout to prepare the handoff, so the success response was missed and the
  update looked like a no-op. The handoff now uses the long request budget.

## 2.8.4

- Claude Code sessions now show the working/busy indicator in the sidebar
  while the agent is processing a turn (previously only Codex/shell sessions
  did — CC's live status was never sampled).
- Fix new Codex sessions failing to cold-attach after their first turn: the
  stored resume command kept the placeholder session id instead of the real
  one once known, so a cold resume could fail until the session was reopened.
- Rename a session straight from automation via
  `yggterm server app session rename <path> <title>` (drives the same rename
  pipeline as the sidebar).
- Right-click a live session → "Restart Session" to force a manual restart.

## 2.8.3

- Claude Code session titles are now integrated both ways. yggterm reads CC's
  own title (a `/rename` or resume-picker Ctrl+R inside Claude Code shows up in
  the sidebar), and renaming a session in yggterm writes that title back into
  Claude Code's session log — so the two stay in sync. Works for local and
  remote CC sessions. (Remote requires the updated yggterm binary on the remote
  machine.)

## 2.8.2

- Fix "new terminal here" on a local live session erroring out: the local
  launch path used the row's `local://<uuid>` identifier as the working
  directory instead of its real cwd (the remote path was already correct).
  Both paths now resolve the cwd the same way.
- Session switching is smoother: switching back to a recently-used session no
  longer triggers a repeated cold remount loop. The previously-rendered host
  is revealed instead of rebuilt, preserving its scrollback. (A brief one-time
  refresh can still occur on the first switch back; further work tracked.)

## 2.8.1

- Fix resumed Claude Code (`remote-cc`) sessions rendering blank / losing their
  prompt on mount and session-switch: the retained-replay readiness layer was
  Codex-only and never recognized Claude's prompt surface. Added a Claude
  prompt-surface recognizer so the retained snapshot replays correctly.
- Reduce xterm.js input/render latency toward native-terminal feel: cut the
  active write-frame budget from 160 ms to 16 ms, tightened the focused-session
  PTY read cadence (60 ms → 16 ms) and the post-keystroke echo poll (45 ms →
  8 ms).
- Enable the GPU canvas renderer by default on Wayland (the DOM renderer is
  xterm.js's slowest backend; the X11 idle-CPU regression that gated it off does
  not reproduce on Wayland). Fixed the render-health heuristic that falsely
  reported low contrast for canvas-rendered surfaces.
- Stop a false "needs your attention" notification when a foreground shell rings
  the terminal BEL in response to the user's own keystroke (e.g. bash readline
  tab-tab with no completion). Background/explicit OSC notifications are
  unaffected.

## 2.7.48

- Fix live-session snapshot projections so shallow Web View/sidebar previews
  summarize the latest transcript tail instead of exporting stale head blocks
  while claiming tail hydration.

## 2.7.47

- Fix remote Codex Web View opening on the wrong end of a hydrated transcript by
  materializing the latest transcript window for the first chat frame instead of
  depending on WebKit scroll timing.

## 2.7.46

- Fix remote Codex Web View latest-turn opening through a mounted latest
  transcript anchor instead of relying only on a global post-render scroll
  script.

## 2.7.45

- Fix remote Codex Web View latest-turn pinning for large transcripts by
  seeding the virtual reader at the estimated transcript tail and repeatedly
  pinning the DOM scroller until the reader settles on the latest hydrated
  block.

## 2.7.44

- Open remote Codex Web View chat readers at the latest hydrated transcript
  turn, so old transcript head blocks do not masquerade as the active
  conversation after switching from Terminal to Web View.

## 2.7.43

- Keep Web View on the hydrated recent transcript tail during remote preview
  refresh. Older head/scan projections from sidebar or daemon snapshots can no
  longer replace the active reader after a successful recent-tail hydration.

## 2.7.42

- Keep Terminal mode interactive when app-control samples a top xterm row that
  is intentionally clipped by the auto-hidden titlebar while the cursor/input
  row remains visible. This fixes Web View -> Terminal live-session cycling
  falsely gating readable daemon-owned PTYs as not paint-visible.

## 2.7.41

- Finish the daemon bind-before-reconcile fix by removing current-alias owner
  retarget scans from runtime load. Updated daemons now bind their current
  socket before any old-daemon inspection, so Web View and terminal restore do
  not wait forever on a missing post-update endpoint.

## 2.7.40

- Make remote Codex Web View hydration bounded and recent by default, so large
  transcripts render as chat blocks without pushing a 30 MB snapshot through
  daemon IPC.
- Add a `server remote preview-tail` helper and guard Web View refresh as a
  single-flight request per session, preventing overlapping preview retries,
  EAGAIN failures, and CPU-heavy loading gates.
- Bind the replacement daemon's current socket before deep preserved-owner
  reconciliation, so stale or busy old daemons cannot leave the updated GUI
  waiting on a missing endpoint during Web View or terminal restore.

## 2.7.38

- Keep preferred-executable handoff scoped to GUI entry launches. Server and
  app-control CLI commands now run from the executable the operator invoked, so
  a stale direct-install state cannot silently route probes or relaunches
  through an older GUI binary.

## 2.7.37

- Keep Web View on the fully hydrated active transcript when a matching live
  row only has shallow preview blocks. Terminal mode still prefers the live
  runtime projection for xterm attachment.
- Defer the remote-terminal fatal resume timeout when recent PTY output shows
  the attach path is still alive, avoiding false failure gates just before
  meaningful output arrives.

## 2.7.36

- Expose focus-capture hit-target and xterm selection-layer diagnostics in the
  basic app-control state snapshot, so selection regressions remain visible even
  when the full DOM snapshot is not used.

## 2.7.35

- Restore xterm-owned text selection on active terminals by making the
  focus-capture layer and context-menu backdrop non-hit-target observers.
- Tighten the selection smoke probe so it must drive xterm pointer gestures and
  observe real xterm selection-layer rectangles instead of passing through a
  synthetic DOM range.
- Let primary terminal clicks close a visible context menu without stealing the
  click from xterm, reducing delayed right-click/selection recovery paths.

## 2.7.34

- Preserve explicit terminal scrollback intent during restore/switch settle, so
  pending prompt-follow repairs cannot snap the viewport back after the user
  scrolls.
- Keep prompt-ready Codex surfaces interactive when a scroll probe/no-op finds
  no xterm scrollback, and accept visible current input rows even when the
  cursor-row sample is temporarily empty.

## 2.7.33

- Enforce KDE active-host terminal retention at the retention primitive, so
  late ready/focus callbacks from inactive sessions cannot re-add hidden xterm
  hosts after a switching sweep. This keeps daemon PTY restore durable while
  preventing WebKit CPU from growing with every visited live session.

## 2.7.32

- Limit KDE live-terminal xterm retention to the active viewport by default.
  Daemon PTYs and live rows remain preserved, but hidden full-size xterm hosts
  are unmounted so a switching sweep cannot leave WebKit repainting many stale
  terminal surfaces.

## 2.7.31

- Treat a clean retained prompt-follow xterm surface as visible even if a stale
  remote-terminal resume notification still exists. The notification is pruned
  as observer noise instead of holding the input gate closed.

## 2.7.30

- Suppress initial remote-terminal resume notices for retained sessions that
  already have ready history or meaningful visible output. Healthy retained
  switches stay visually quiet; slow/failure paths still surface notifications.

## 2.7.29

- Make remote Web View transcript hydration an explicit daemon IPC contract.
  Web View sync now requests a full remote payload, while legacy and cache-only
  refresh requests remain backward-compatible.

## 2.7.28

- Let the daemon perform the full remote transcript fetch only for the active
  Web View surface. Terminal-mode refresh remains cache-only, and stored remote
  sessions promoted back to Terminal are restored to the live-session order.

## 2.7.27

- Refresh the Web View full-hydration predicate so readable remote scan content
  still upgrades to the full saved transcript through live-session Storage
  metadata instead of staying at the initial scan excerpt.

## 2.7.26

- Render readable remote-scanned Web View conversations immediately, while
  using live-session `Storage` metadata to hydrate the full saved transcript in
  the background without touching the daemon-owned PTY.
- Treat terminal attach-in-flight as foreground controller state only. Switching
  away from a retained live session now prunes stale background attach gates
  without dropping the retained runtime.
- Stop blocking a readable, input-ready remote prompt behind a collapsed
  scrollback recovery gate. Suspicious scrollback remains observable through
  app-control/probes, and explicit scroll failures still fail, but prompt-ready
  restore is allowed to become interactive immediately.

## 2.7.25

- Clear Web View's toolbar loading state when saved transcript/context fallback
  is already readable and no preview request is in flight.

## 2.7.24

- Treat saved transcript/context fallback as real readable Web View content, so
  background hydration cannot show the large remote-session loading/failure
  gate over or under an already readable conversation.

## 2.7.23

- Allow live sessions to stay in Web View as read-only conversation surfaces
  without closing, detaching, restarting, or hiding the daemon-owned PTY.
- Tighten the xterm embed smoke so a live-session Web View request must settle
  in rendered mode while the same daemon runtime remains present, and switching
  back to Terminal must reattach to that runtime.
- Update app-control viewport readiness so live-session Web View is not reported
  as a terminal failure when the read-only conversation surface is mounted.
- Stop treating readable `remote:scan` transcript content as a blocking loading
  gate; Web View now renders scanned content immediately and auto-refreshes only
  true empty/loading placeholders.

## 2.7.22

- Reserved during live Web View handoff testing; superseded by 2.7.23 before
  publication.

## 2.7.21

- Make Web View a provider-backed conversation surface. Codex and terminal
  transcript providers remain read-only, and future OpenWebUI/JYAS-style API
  chat providers must declare send capability explicitly before the UI can
  expose chat input.
- Add conversation-provider app-control attributes and contract tests so Web
  View presentation cannot become terminal/xterm truth.

## 2.7.20

- Restore preserved-owner scrollback before declaring a remote session
  interactive when a short live read wins the first paint race. A retained
  history seed may now run once for collapsed-scrollback recovery while input
  remains gated; it is still blocked after input is enabled or hot.
- Keep xterm-owned session snapshots labeled as `xterm_session_snapshot`
  observer cache instead of promoting them to `daemon_pty` truth.

## 2.7.19

- Treat versioned daemon socket aliases by their filesystem target, not their
  path string. If a legacy socket such as `server-2-1-0.sock` points at the
  current daemon, it can no longer masquerade as a preserved PTY owner; startup
  retargets those registry entries to a reachable real owner when one reports
  the runtime key.
- Capture terminal right-click before xterm.js/WebKit can prepare the helper
  textarea for native paste. Right-click now opens the Yggterm
  terminal/session context menu without sending clipboard bytes to the PTY;
  middle-click remains the primary-selection paste path.

## 2.7.18

- Treat live-connected remote xterm hosts as a hard stop for UI retained
  rehydrate as well as delayed daemon retained replay. A retained snapshot may
  seed an empty restore, but it is discarded once daemon PTY truth is live.

## 2.7.17

- Stop delayed retained replay from writing over a remote terminal after the
  live xterm bridge has accepted input, preventing settled Codex sessions from
  snapping back into old retained wall-text.
- Keep terminal-open attempts from carrying a previous session's active xterm
  host id into the newly selected session.
- Tighten hot-update duplicate-owner handling: current daemon runtime I/O may
  bypass a stale preserved-owner route, but the preserved-owner registry is
  removed only after targeted local-only duplicate runtime pruning succeeds.

## 2.7.16

- Treat terminal paint as observer-only for geometry. Paint events no longer
  resize daemon-owned PTYs, and unfocused ResizeObserver transients are skipped
  once a terminal already has a usable grid so app-control/screenshot/layout
  probes cannot bounce a live Codex TUI between stale terminal sizes.

## 2.7.15

- Reopen maximized windows as maximized by applying persisted window state at
  native window construction and syncing window-manager close state before the
  shell enters shutdown.
- Disable the remote prompt-gap PTY resize nudge. Retained restore must settle
  xterm viewport/render truth without resizing daemon-owned live PTYs behind
  Codex/TUI sessions.
- Stop visible-paint repair from refitting xterm or notifying the daemon PTY of
  a new grid, and prefer rendered xterm cell metrics over stale fallback CSS
  when explicit resize/refit paths compute terminal geometry.
- Tighten app-control geometry diagnostics so stale prompt-follow debug records
  cannot hide a cursor row that has drifted below the visible viewport.
- Add a smoke check for maximized close/relaunch persistence.

## 2.7.14

- Stop retained replay from continuing prompt-follow or repaint work after
  trusted live input promotes a terminal surface back to daemon-owned PTY
  truth. This prevents idle Codex terminals from snapping back into stale
  wall-text snapshots after typing stops.
- Keep the active terminal on the active input frame budget while typing is
  hot, even when Codex status animation bookkeeping is present.

## 2.7.13

- Repair prompt-follow terminal scroll truth when WebKit reports a DOM
  viewport beyond xterm's own scrollback base. Yggterm now clamps and resyncs
  that impossible DOM state instead of letting typing jump the active session
  high into scrollback.
- Reset retained-terminal recovery timing when a replacement xterm surface is
  mounted, so stale first-output timestamps from discarded empty surfaces do
  not make restore telemetry or watchdog decisions lie.
- Persist the actual window maximized state on app-control and graceful close,
  so a maximized Yggterm window reopens maximized after restart/handoff.
- Ignore late close-path window resize/decorator events when persisting window
  placement, preventing Linux detach cleanup from overwriting maximized state.

## 2.7.9

- Remove the stable theme alpha/transparency leak that kept the shell frame and
  gradient on translucent CSS material. Stable Yggterm now exports opaque shell
  chrome and single-layer opaque gradients, leaving blur/alpha/grain work on
  the experimental branch and reducing compositor/WebKit repaint cost on jojo.

## 2.7.8

- Protect the daemon named in the hot-update preserved-owner registry from
  Linux cleanup even when another daemon can describe the same runtime key.
  Session survival now wins over stale-daemon cleanup, preventing keep-alive
  Codex conversations from being killed and relaunched as interrupted sessions
  during update/restart handoff.

## 2.7.7

- Stop stale xterm viewport telemetry from classifying a prompt-follow terminal
  as cursor-clipped after a matched force-follow pass. The classifier now trusts
  the prompt-follow force result when row fit is clean, preventing retained
  terminal recovery from remounting a healthy viewport and burning CPU.

## 2.7.6

- Stop missing-saved-session remote Codex rows from auto-spawning doomed PTYs.
  Yggterm now keeps the live row and launch error visible, but the daemon gates
  restart/new-spawn paths once the saved transcript probe has failed so the GUI
  does not enter a retry/repaint loop.

## 2.7.5

- Preserve live remote Codex rows when their saved transcript metadata is
  missing. Running daemon PTY truth now wins over missing saved-session
  metadata, so clicking a live row records a launch error instead of deleting
  the row and spawning duplicate renamed copies.
- Scope sidebar selection and drag state to the visual live row when the same
  session also appears as a cwd projection. Cwd projections can still focus the
  session, but only the canonical `Live Sessions` row reorders the live group.
- Remember whether the app window was maximized and restore that state on the
  next launch.

## 2.7.4

- Stop active terminal write flushes while the Yggterm window is unfocused or
  app-control-backgrounded. This keeps daemon PTY truth intact while preventing
  WebKit/xterm repaint work from burning CPU in the background.
- Deduplicate terminal text paste gestures at the xterm bridge and expose
  clipboard paste/copy telemetry counters without logging clipboard contents.

## 2.7.3

- Fix fresh remote Codex onboarding when xterm's visible tail starts mid-menu.
  A truncated auth menu such as `tGPT ... Device Code ... API key ... Press
  enter to continue` is still interactive PTY truth, so Yggterm now clears the
  resume notification/input gate instead of repeatedly remounting the terminal.

## 2.7.2

- Let fresh remote Codex onboarding and sign-in menus accept input. These
  surfaces are interactive PTY truth even before Codex has emitted a normal
  prompt or transcript storage path, so Yggterm now clears the resume gate for
  them instead of leaving the viewport locked behind a recovery notification.
- Keep Startpage saved-session cards limited to durable saved sessions. Live
  terminal/runtime projections still appear in `Live Sessions` and their
  machine/cwd group, but a generic SSH terminal or pre-transcript Codex runtime
  no longer looks like a saved UUID-backed session on the Startpage.

## 2.7.1

- Prevent fresh remote Codex onboarding sessions from becoming phantom saved
  sessions. Remote Codex rows now enter machine/cwd saved-session truth only
  after Codex exposes a real transcript storage path, and restore filters old
  storage-less rows.
- Keep drag feedback row-local when a live remote terminal appears in both
  `Live Sessions` and its machine/cwd projection.

## 2.7.0

- Harden protected remote restores with a one-minute careful-restore boundary.
  Keep Alive and temporary update-restored runtimes get one non-destructive
  reattach/resize/refresh attempt after the timeout; Yggterm must not kill or
  duplicate a still-running runtime unless daemon truth says it is gone or the
  user/harness explicitly force-restarts it.
- Move terminal selection copy off WebKit's browser Clipboard API. `Ctrl+Shift+C`
  and `Ctrl+Shift+X` now bridge xterm selection text into a native clipboard
  owner thread, keeping app-control and the shell responsive when Remmina or the
  desktop clipboard stack stalls.
- Repair stale sidebar scroll offsets after launch, refresh, search, or tree
  shrink so top rows such as `Live Sessions` cannot remain clipped until another
  expansion forces a scrollbar.
- Update terminal and stability contracts for switch latency, protected restore,
  terminal selection-copy proof, and sidebar scroll-bounds smoke coverage.

## 2.6.83

- Harden hot-update session survival. Reachable old daemons that still report a
  live PTY now recover missing rows as temporary update-restore sessions instead
  of being treated as disposable stale state.
- Fix daemon hot-restart handoff so the replacement daemon is spawned only after
  the old daemon releases the current socket lock. This prevents live PTYs from
  staying alive behind a missing `server-<version>.sock`.
- Promote synthetic remote Codex runtime rows to the real Codex transcript id for
  normal Keep Alive persistence, not only update-restart persistence. Restores now
  resume the real saved session instead of replaying `start-codex` under the old
  synthetic key.
- Infer Codex SQLite `state_*.sqlite` files from the sibling `logs_*.sqlite` fd
  when Codex no longer holds the state DB open, so runtime identity can still be
  discovered from the live process tree.
- Keep terminal mode rendered while the GUI is closing instead of briefly swapping
  the viewport into Web View as part of the KDE close path.
- Block automatic remote-runtime restarts for still-running temporary
  update-restored sessions. Stale, blank, or spec-mismatched output now remains
  a visible recovery incident until the runtime exits or a user/harness action
  explicitly restarts it.
- Remove the broad preserved-owner cleanup fallback that could call
  `PrepareClientClose` on an old owner and kill unrelated running sessions.
- Detect active remote Codex scroll-lock incidents where wheel input reaches an
  xterm surface with `base_y=0`, and allow daemon-retained replay for reused
  `start-codex` runtimes so stale fresh-start metadata cannot suppress
  scrollback restoration.
- Keep daemon-retained replay from being clobbered by the blank-surface
  recovery watchdog. Once retained history has been staged into xterm, a late
  empty DOM sample no longer resets the terminal and overwrites scrollback with
  a short screen-only read.
- Stop treating a prompt-ready daemon-retained history snapshot as unsafe just
  because input is enabled. That false positive rearmed recovery and remounted
  the live terminal back to a short `daemon_terminal_read`.
- Tighten app-control terminal focus diagnostics so stale helper focus on a
  retained offscreen host no longer masquerades as an active-session mismatch,
  and pause hidden loading animations while the window is unfocused or
  app-control-backgrounded.

## 2.6.79

- Fix a jojo fan-budget regression in the desktop render path. Retained
  terminal canvases now receive a slim terminal-only snapshot instead of the
  full workspace snapshot, avoiding repeated Dioxus equality checks across the
  entire sidebar/session graph.
- Remove render-time title/summary database opens from the active session copy
  generation path while focus-time generation remains disabled. This stops the
  idle GUI from repeatedly touching `~/.yggterm/session-titles.db`.
- Add regression tests that block full `RenderSnapshot` terminal-canvas props
  and active render-path `SessionStore` opens from returning.

## 2.6.78

- Fix the left sidebar clipping upward after live-session/tree changes. The
  sidebar now has explicit stretch geometry, app-control exposes the scroller
  bounds, and the autoscroll repair targets the sidebar scroller directly
  instead of relying on generic `scrollIntoView`.
- Tighten the xterm/sidebar smoke contract so selected live rows and the `Live
  Sessions` group fail when they exist in app-control state but are clipped
  outside the visible shell frame.

## 2.6.77

- Restore the focused xterm block cursor fill on Codex prompt rows. Cursor
  blinking remains disabled for idle CPU, but the native block cursor now paints
  with the terminal cursor theme color instead of inheriting the styled prompt
  cell background and becoming invisible.

## 2.6.76

- Add a terminal-surface CSS backstop for xterm cursor blinking. Some retained
  DOM renderer paths still attach the `xterm-cursor-blink` class even when the
  xterm option is false; Yggterm now forces that cursor animation off without
  drawing an overlay or changing PTY bytes.
- Expose `xterm_cursor_blink` in app-control terminal host snapshots so cursor
  option truth can be checked alongside the CSS animation census.

## 2.6.75

- Disable xterm.js cursor CSS blinking in the desktop shell. The cursor remains
  a native xterm block cursor, but no longer keeps WebKit/GTK hot on idle
  Wayland sessions.
- Tighten the xterm smoke contract so cursor CSS animations are treated as an
  idle/fan-budget regression rather than a harmless visual detail.

## 2.6.74

- Fix the Codex prompt cursor on dim placeholder text: `.xterm-dim` on a cursor
  span is no longer treated as blink-off state, so the block cursor keeps
  blinking while the prompt placeholder remains dim.
- Tighten the cursor smoke contract to inspect the sampled cursor cell
  background for styled prompt rows instead of using `.xterm-dim` as a blink
  signal.

## 2.6.73

- Fix the xterm cursor blink-off contract: focused block cursors now let
  xterm's native dim/off state go transparent instead of painting a terminal
  background tile through styled Codex prompt rows.
- Keep dim-row contrast normalization away from `.xterm-cursor` so terminal
  text can be helped without taking over cursor rendering.

## 2.6.72

- Keep terminal input/cursor focus alive when the autohidden titlebar covers the
  first xterm row; a visible prompt/cursor now wins over the covered-row sample.
- Tighten the xterm smoke cursor check so a Codex prompt-row blink-off cursor
  cannot collapse to the terminal background.

## 2.6.71

- Keep the active terminal cursor visible through restore/focus drift by using
  xterm.js' native blinking block cursor and block inactive cursor instead of
  the low-contrast inactive outline.

## 2.6.70

- Drop all retained xterm render state for a live terminal as soon as a close
  starts, so a removed session cannot stay mounted as a zombie DOM surface.
- Give live terminal close/remove requests the long daemon response budget,
  preventing slow PTY teardown from succeeding underneath while the GUI reports
  `Delete Failed`.
- Expand daemon request warnings to include the full error chain for future
  response-timeout and serialization incidents.

## 2.6.69

- Restore lossless xterm writes for synchronized Codex/TUI repaint bursts. The
  write bridge may still pace large terminal frames, but it no longer deletes or
  rewrites PTY bytes before xterm.js applies them.
- Treat a blank active xterm surface as not launch-settled, and recover stale
  local hot-open leases without requiring a manual switching pass.
- Stop preview image rendering from probing session URI strings as filesystem
  image paths during Dioxus renders.

## 2.6.68

- Collapse repeated synchronized Codex repaint bursts to the latest xterm frame
  while preserving real scrollback/output, reducing WebKit/GUI CPU during active
  Working/status animations.
- Expose coalesced payload size in app-control snapshots and tighten the inline
  animation smoke budget so oversized repaint bursts are caught.

## 2.6.67

- Repair blank DOM xterm surfaces where the PTY/xterm buffer still has live text
  but WebKit has lost the `.xterm-rows` renderer layer, and make manual Redraw
  Terminal attempt a bounded xterm-native renderer-surface recovery.
- Tighten live-session close/delete preflight so a closing terminal cannot be
  relaunched by stale open-attempt state before the daemon remove completes.
- Add app-control and smoke coverage for missing DOM renderer text layers and
  stale PromptFollow visual scroll locks.

## 2.6.66

- Fix retained remote Codex prompt readiness so prompt-ready surfaces that pass
  every visual, geometry, runtime, and transcript gate actually clear the resume
  overlay instead of falling through to a false negative.

## 2.6.65

- Restore generic shell prompt replay for non-Codex remote terminals while
  keeping Codex resume replay gated to Codex prompt surfaces, fixing the CI
  regression that slipped into 2.6.64.
- Force one bounded xterm-native repaint when retained replay accepts already
  visible daemon-backed text, preventing a restore from settling with DOM/buffer
  text present but a blank WebKit paint.
- Keep app-control snapshots from becoming an alternate terminal source of
  truth: terminal text tails now prefer the xterm buffer over DOM renderer
  chrome, and unfocused read-only probes no longer trigger input-gated recovery.
- Cool long-running Codex inline status animation to a slower xterm write cadence
  after it has been visible for several seconds, reducing the active WebKit/GUI
  fan budget without slowing fresh typing or terminal echo.

## 2.6.64

- Recover active runtime-owned terminal rows when the current daemon has no
  live PTY for them during terminal reads. Internal bridge output such as
  `terminal session not found` is now treated as a failed attach path, not as
  valid terminal content that can keep a stale retained viewport alive.

## 2.6.63

- Recover from stale preserved-owner daemons that report a live runtime but fail
  terminal reads with `terminal session not found`. The current daemon now drops
  that stale owner and falls back to the saved Codex resume path instead of
  leaving the selected terminal blank and non-writable.

## 2.6.62

- Keep app-control-ready remote terminal sessions writable even when a stale
  attach-in-flight flag was left behind by recovery. A visible, ready xterm
  surface now reconciles the local input signals instead of requiring a manual
  focus nudge.
- Limit the "accepted input without daemon echo" health alarm to broken sparse
  prompt layouts, so normal current Codex prompt rows do not disable input after
  typing settles.
- Register preserved PTY owners for already-visible keep-alive/update-restore
  rows during hot-update recovery. A live row is no longer treated as proof that
  the new daemon owns the PTY.

## 2.6.60

- Ship the emergency hot-update/session-preservation patch tested live on jojo:
  stale-daemon cleanup now preserves directly-owned PTY runtimes, startup
  terminal prewarm is opt-in instead of bulk-resuming every remembered remote
  live session, and terminal readiness no longer gates input on a healthy xterm
  surface just because shell chrome overlays a top-edge sample.
- Keep the terminal readiness probe from rejecting a healthy xterm surface just
  because the auto-hidden titlebar/session chip overlays a top-edge row sample.

## 2.6.59

- Make startup terminal prewarm opt-in so update/restart cannot eagerly launch
  every remembered remote live session before the active terminal settles.

## 2.6.58

- Protect directly-owned PTY runtimes from Linux stale-daemon cleanup even when
  a separate preserved-owner registry exists, unless the current daemon already
  owns the exact same runtime key.
- Fix the terminal embed cursor-cell sampler by defining its transparent-color
  helper inside the xterm.js runtime script.

## 2.6.57

- Keep the xterm.js focused block cursor blink-off state on the prompt-row
  background by styling the native block-dim cursor from the sampled xterm cell.
- Refresh the cursor-cell background after restored terminal mount as well as
  render/write callbacks, with bounded retries while xterm finishes painting.

## 2.6.56

- Refresh the xterm cursor-cell background after xterm render/write events so
  the native cursor inherits settled prompt-row paint instead of an early
  transparent sample.

## 2.6.55

- Preserve Codex prompt-row background under xterm.js outline/off cursor states
  by sampling the cursor cell background from xterm buffer state instead of
  letting a transparent cursor reveal the terminal theme background.
- Expose the sampled cursor-cell background in app-control terminal snapshots
  so one-cell cursor paint regressions can be proved without overlays.
- Document the cursor-cell source-of-truth rule in `docs/xterm.md`.

## 2.6.54

- Persist explicit `Live Sessions` drag order through the daemon, and restore
  rows in the saved order instead of reversing them during restart recovery.
- Keep focusing/switching live sessions from silently promoting the focused row
  to the top, so user-arranged live rows stay stable across normal work.

## 2.6.53

- Put control-only synchronized terminal repaint bursts back under the xterm
  frame budget, including sub-256 byte Codex repaint chunks that previously
  bypassed batching and overheated the foreground GUI/WebKit render loop.
- Throttle per-render `main_surface` trace writes and repeated
  `forward_protocol_only_output` trace events so observability cannot become
  its own foreground CPU incident while a terminal is actively repainting.

## 2.6.52

- Promote pending sidebar drags from the global pointer stream, so dragging the
  first live-session row downward still starts even if the pointer leaves the
  source row before the row-local 6px threshold fires.

## 2.6.51

- Fix live-session sidebar dragging when the same session is visible in both
  Live Sessions and its CWD tree entry; duplicate drag sources are now collapsed
  before reorder planning.
- Reduce foreground CPU during sidebar dragging by ignoring unchanged hover
  targets instead of recording telemetry and refreshing tree debug state on
  every pointer tick.
- Allow dragging from the session title text itself, so the first live-session
  row can be moved down with the same hit target as lower rows.

## 2.6.50

- Suppress frontend xterm.js OSC color-query replies for palette/default-color
  probes so terminal protocol responses cannot echo into cooked shell prompts
  as visible `rgb:` text.
- Answer OSC 4 palette queries in the daemon protocol filter and keep palette
  set sequences intact for xterm.js, preventing split palette traffic from
  corrupting normal shell output.
- Strip legacy visible `rgb:` protocol-reply pollution from retained terminal
  replay payloads before writing them into xterm.js.
- Expose suppressed terminal-protocol response diagnostics in app-control timing
  snapshots.

## 2.6.49

- Keep active terminal input enabled when app-control detects a slow visible
  write budget; slow write cadence is now reported as `performance_problem`
  instead of being mixed into terminal geometry/liveness truth.
- Keep Codex live-session busy icons driven by mounted xterm activity even when
  hot TUI frames are intentionally not copied into sidebar summaries.
- Ignore stale prompt-follow scrollback locks when xterm's measured viewport is
  already at the live buffer bottom.

## 2.6.48

- Make remote-resume retained-history surfaces promote to `daemon_pty` only
  when Rust's terminal readiness policy explicitly opens input, preventing
  direct pointer/focus events from turning a replayed screen into a fake live
  terminal.
- Expose `rust_input_gate_open` and retained-replay promotion diagnostics in
  app-control terminal host snapshots.

## 2.6.47

- Allow Live Sessions rows to be reordered by drag/drop, with the daemon-side
  live-session order as the source of truth.
- Add a regression guard that rejects retained-history terminal replay when it
  is still input-enabled, so app-control gates input until the real PTY surface
  is current.
- Keep remote-resume xterm hosts visible during recovery and make app-control
  reject mounted terminal hosts that are transparent.
- Make direct-install state loading prefer the coherent newest executable when
  mixed old/new state fields are present, and keep exact `yggterm-headless
  --version` probes from re-execing into another version.

## 2.6.46

- Add modal-backed deletion to Startpage recent-session cards, including a bin
  icon next to summary edit and right-click context menu access from the card.
- Treat transparent terminal focus-capture layers as non-obstructing in
  app-control paint hit-tests, so healthy xterm DOM rows are not reported as
  invisible just because the click-capture layer is topmost.
- Make manual terminal redraw force prompt-follow recovery back to the cursor
  row, covering stale viewport states where the public xterm viewport and the
  visible DOM scroll position disagree.
- Stop rename-input focus retries from repeatedly reselecting the whole title
  after the user has moved the caret or started editing.
- Show a persistent "Closing Terminals" progress notification while bulk live
  closes are still running.

## 2.6.43

- Expose DOM row/cursor paint hit-test diagnostics in the default basic
  app-control snapshot, not only in the full DOM path, so live smoke tests can
  catch rows-present-but-not-painted failures without opting into expensive
  snapshots.

## 2.6.42

- Keep retained xterm hosts in the normal paint tree with light layout
  containment instead of strict/offscreen compositor isolation, so switching
  sessions cannot leave daemon-backed DOM rows mounted but visually blank.
- Add app-control DOM paint hit-test telemetry for xterm row and cursor samples,
  and reject terminal-ready states where buffered/DOM text exists but the row
  paint is not topmost at the visible text point.
- Update the Dioxus 0.7 lockline to 0.7.9 for the desktop shell stack.

## 2.6.41

- Make prompt-follow recovery use an effective xterm viewport that cross-checks
  the DOM viewport scroll position against xterm's public `viewportY`, avoiding
  false retained-fault remounts when WebKit leaves the public viewport counter
  stale while the DOM renderer is already scrolled to the prompt.
- Expose public, visual, effective, and source viewport diagnostics in
  app-control so future scrollback/prompt-follow splits are visible without a
  switching pass.

## 2.6.40

- Treat retained rehydrate daemon-readiness as a first-class terminal recovery
  gate, so the retained-fault watchdog defers instead of remounting while the
  current daemon socket is still becoming reachable after hot update or GUI
  restart.
- Add terminal telemetry for retained-fault watchdog deferrals caused by
  daemon-readiness waits, making startup socket races distinguishable from real
  blank xterm surfaces.

## 2.6.39

- Gate retained remote terminal rehydrate on the current daemon endpoint
  becoming reachable before replaying a preserved PTY snapshot. This removes
  the pre-ready path where initial retained reads failed, the 5-second watchdog
  remounted xterm, and the terminal only became correct after a delayed recovery
  pass.
- Add terminal telemetry for retained-rehydrate daemon readiness waits and
  failures so startup endpoint races are visible in
  `~/.yggterm/telemetry/terminal.sqlite3`.

## 2.6.38

- Stop retained-fault recovery from reopening a remote terminal immediately
  after host-health has already marked it ready. Transient blank/retained
  samples inside the settle grace are now telemetry, not another xterm remount,
  which removes the startup retry storm and the associated CPU spike.

## 2.6.37

- Add durable telemetry for renderer-health splits where xterm has buffered text
  but the mounted surface reports blank or low-contrast pixels, so future
  canvas/DOM visibility regressions leave a queryable incident instead of
  relying on screenshots alone.

## 2.6.36

- Disable xterm canvas as the default Wayland renderer. Canvas remains an
  explicit diagnostic opt-in, while release builds use DOM rows by default so
  screenshot proof and visible terminal text share the same path.
- Add app-control coverage for canvas terminals with buffered text and a
  low-contrast foreground/background contract.

## 2.6.35

- Stop retained-fault recovery from promoting non-prompt terminal snapshots to
  an interactive remote xterm. Those snapshots are now telemetry/debug evidence
  only; the live PTY stream must prove prompt-ready input before Yggterm clears
  recovery or enables typing.
- Add app-control and terminal telemetry coverage for session/host identity
  mismatches and retained-recovery watchdog remounts, so a selected session can
  no longer silently point at another xterm host.

## 2.6.34

- Stop retained-history replay from overwriting a freshly mounted remote xterm
  after the terminal has already staged scrollback. App-control now rejects an
  input-enabled remote terminal whose final content source is
  `daemon_retained_history_screen_snapshot`; retained history remains a
  scrollback seed, not the interactive terminal truth.

## 2.6.33

- Preserve reachable prior hot-update PTY owners during the next handoff even
  when the current daemon no longer has their live rows. Handoff now writes only
  runtimes the outgoing daemon directly owns, recovers kept/update-restored rows
  from reachable owner snapshots when a prior registry was truncated, and keeps
  terminal reads off the saved-session mismatch probe hot loop.

## 2.6.32

- Restore missing live-session rows from the hot-update preserved PTY owner
  before filtering runtime truth. A kept session that is still owned by an
  older daemon now remains a live terminal target after GUI/daemon replacement
  instead of degrading into a blank saved-session preview.

## 2.6.31

- Stop treating overlapping app-control xterm samples as one ordered transcript.
  A wrapped Codex prompt sampled through `text_tail` now keeps the remote
  terminal ready instead of re-entering recovery and resume-notification churn.
- Do not force xterm prompt-follow scrolling on each typed byte when the cursor
  is already visible. This removes the one-line flicker seen while typing into
  long wrapped Codex prompts.

## 2.6.30

- Keep legacy remote cwd bookmark labels aligned with the repaired cwd path.
  A stale generated bookmark renamed to `git/samplers` now projects as
  `/home/pi/git/samplers` without duplicating the renamed path segments into
  a `git/git/samplers` row.

## 2.6.29

- Treat remote workspace folder renames as cwd bookmark moves. A folder created
  under `practice:/home/pi` and renamed to `git/samplers` now resolves to
  `/home/pi/git/samplers`, and Startpage `New Codex Session` / `New
  Terminal` launch from that selected remote cwd instead of falling back to the
  parent `/home/pi`.

## 2.6.28

- Hide the synthetic `/__remote_folder__` storage root from the visible local
  sidebar while still using its saved rows as the remote cwd bookmark source.
  Remote `Add Folder` bookmarks now project only under their machine tree.

## 2.6.27

- Make saved remote cwd bookmarks a separate sidebar projection input instead
  of deriving them from currently expanded local rows. `Add Folder` on a remote
  folder such as `practice:/home/pi` now makes the saved folder visible under
  that remote machine tree, including after restart, without leaking the
  synthetic `/__remote_folder__/...` storage path into the local tree.
- Add app-control coverage for the selected-row `folder` start action so the
  remote Add Folder path can be smoke-tested without desktop-wide pointer
  automation.

## 2.6.26

- Keep preserved terminal-owner entries during daemon startup even when the
  registry was written for the previous patch. The daemon now restores persisted
  live-session truth first, prunes only keys no longer represented by live
  metadata, then retargets the surviving registry to the current version. This
  prevents a fresh direct-install launch from wiping the handoff map and
  spawning duplicate `ssh ... codex resume` processes.
- Route `yggterm-headless server app launch` through the active GUI companion
  instead of failing after advertising the command in help, so live update
  harnesses can use the app-owned relaunch path rather than ad hoc shell
  backgrounding.

## 2.6.25

- Harden direct-install hot update against chained stale daemons: handoff now
  retargets every represented live runtime key to the current outgoing owner
  daemon, so the replacement daemon does not select an older sidecar directly
  and spawn duplicate remote Codex resumes.
- Treat temporary update-restored sessions as session-survival protected during
  preserved-owner validation. Saved-session mismatch heuristics can keep
  recovery visible, but they no longer remove the preserved owner or replace the
  PTY before handoff verification.

## 2.6.24

- Share the retained-scrollback replay selection rule between preserved-owner
  hot-update rehydrate and daemon-retained xterm replay, closing the split path
  that allowed short current-screen reads to collapse restored scrollback after
  a later startup pass.
- Document and expose the settled app-control `open` command as the required
  smoke-test path for switching live sessions without desktop pointer
  automation.
- Restore end-user close semantics for live terminals: clicking close
  terminates the selected runtime even when it is marked Keep Alive; Keep Alive
  is only the GUI-close/update preservation contract.
- Project saved remote cwd folders back into their owning machine tree, so
  `Add Folder` on a remote folder such as `practice:/home/pi` creates a visible
  remote cwd bookmark instead of disappearing or leaking as a local synthetic
  row.

## 2.6.23

- Finish the retained scrollback repair for preserved-owner hot updates. The
  collapsed-scrollback rehydrate path now builds the same history-plus-current
  screen replay as the daemon-retained mount path, and short initial-read
  refreshes can no longer overwrite the restored scrollback immediately after
  replay.

## 2.6.22

- Preserve retained xterm scrollback when a cursor-addressed daemon snapshot
  needs a safer visible-screen replay. The GUI now seeds plain retained PTY
  history as scrollback, clears only the visible screen, then writes the current
  daemon screen snapshot instead of replacing all history with a 25-line screen.
- Allow active-terminal wheel scrolling while keyboard input remains gated, so
  a readable retained/recovering terminal can still move through existing
  scrollback without weakening paste or typed-input readiness.

## 2.6.21

- Make ordinary close/remove of an explicitly kept live session a viewport
  detach only. Kept sessions no longer call remote Codex shutdown, terminal
  runtime removal, preserved-owner removal, or live-row metadata removal through
  the normal close path.

## 2.6.20

- Repair the keep-alive hot-update adoption path that 2.6.19 still missed:
  before startup prewarm or focus can spawn a fresh remote resume command for a
  kept remote runtime, the daemon now scans reachable old owners, records the
  preserved owner, and routes terminal I/O there.
- Keep preserved keep-alive owners attached even when their early snapshot looks
  saved-session mismatched. That mismatch is recovery evidence, not permission
  to detach a still-running work session.

## 2.6.19

- Preserve explicit keep-alive remote runtimes when early resume output looks
  stale or mismatched. The daemon now treats that as a recovery/input-gating
  signal instead of permission to restart the transport under the same session
  label.
- Hide the mounted xterm host during remote-resume recovery until the surface is
  current or failed, preventing stale retained DOM/xterm text from flashing on
  startup while keeping the host mounted for layout and probes.

## 2.6.18

- Sort scoped local Startpage recent work by the source Codex JSONL mtime, not
  by sidebar/tree order. Local `/home/pi` now surfaces jojo's actual latest
  local sessions while still excluding remote sessions from matching cwd text.

## 2.6.17

- Finish the local Startpage scope fix by using the full sidebar session tree
  for Startpage recent work. Collapsed local folders now still show their local
  sessions while continuing to exclude remote sessions with the same cwd.

## 2.6.16

- Keep local Startpage recent work scoped to local sessions. A selected local
  cwd no longer pulls in `dev`, `practice`, or other remote sessions just
  because they share the same cwd string.
- Treat daemon PTY output without a current prompt row as readable but not
  input-ready. App-control now rejects input-enabled remote surfaces when xterm
  only has retained/current output and no prompt-ready row.
- Refresh the local Yggterm-managed Codex CLI on explicit local session launch,
  so stale managed binaries do not surface Codex's own interactive update
  prompt inside the terminal.

## 2.6.15

- Tighten terminal scroll probe truth. `movement_expected` now reflects whether
  the mounted xterm viewport can actually move, and `scroll_probe_moved` only
  passes when `viewport_y` or DOM `viewport_scroll_top` changes. Wheel events,
  scroll event counters, and live output text churn remain diagnostic signals,
  not proof that scrollback moved.

## 2.6.14

- Make hot-restart fleet cleanup reject empty duplicate-runtime coverage. A
  stale daemon may only be retired as a duplicate when the current daemon
  explicitly owns every runtime key being guarded; an empty `covered_runtime_keys`
  set is not proof that session shutdown is safe.

## 2.6.13

- Treat wrapped Codex prompt input as a prompt-ready xterm surface. Long user
  prompts can wrap across continuation rows before the Codex footer; app-control
  now accepts that live PTY state instead of leaving resume notifications and
  input/scroll gates in a false recovering state.

## 2.6.12

- Recover retained remote sessions that mount as an empty xterm after a hot
  restart without waiting for incidental PTY output. Retained-fault bootstraps
  can now rehydrate from daemon-retained terminal truth even before the new GUI
  has rebuilt local ready-path history, and explicit empty-surface faults choose
  retained snapshot recovery.

## 2.6.11

- Prevent retained scrollback recovery from being overwritten by the blank-host
  current-screen fallback. Retained replay now marks the terminal surface as
  staged/connected and emits host-health so readiness can be verified through
  the normal xterm probe path.

## 2.6.10

- Restore retained remote scrollback after a ready-history remount collapses the
  active xterm buffer. Yggterm now treats prompt-only, empty, stale, or
  no-current-input-row retained surfaces as daemon-retained PTY replay recovery
  targets instead of letting stale ready history suppress recovery.
- Move retained replay decisions into `terminal_retained_replay_policy.rs` so
  shell orchestration executes one policy rather than growing separate replay
  gates.

## 2.6.9

- Restore strict PTY byte fidelity in the xterm write path. Rust and JavaScript
  write-frame helpers now batch only flush timing; they no longer collapse,
  trim, or rewrite synchronized Codex repaint frames that xterm.js must parse in
  order.
- Disable active recovery PTY snapshots as an authoritative terminal replay
  source. They remain observability evidence, not a replacement terminal truth.

## 2.6.8

- Preserve manually named session titles from generated-copy churn. Remote
  title upserts now keep existing `manual` titles unless the incoming update is
  also manual, and the fallback detector rejects low-signal generated titles
  such as `While Those Are Generating Can` and `Current Status Live ...`.

## 2.6.7

- Stop no-op ResizeObserver events from scheduling repeated prompt-follow
  scroll work. The xterm mount now follows the prompt only after an actual grid
  or row-fit change, and coalesces delayed follow-up callbacks so a quiet
  retained terminal cannot flicker or burn CPU from resize churn alone.

## 2.6.6

- Fix stale daemon cleanup during hot update. Duplicate old PTY owners are now
  pruned with a local-only runtime-drop request instead of the user-facing
  close-session path, so cleanup does not try to terminate the remote Codex
  session and leave stale daemons protected forever.
- Put a hard budget on retained terminal remount recovery. If a retained xterm
  surface stays blank after controlled remount attempts, Yggterm records an
  observable terminal failure instead of spinning the render loop and burning
  CPU.

## 2.6.5

- Keep daemon-owned remote Codex runtimes tied to the terminal appearance that
  requested them. Yggterm now refuses to bridge into an existing runtime whose
  launch command advertises the wrong dark/light terminal identity, so Codex
  prompt bands do not inherit a stale light theme inside a dark terminal.
- Stop remote scan previews from replacing an existing human/live session title
  with generated copy. Scanned titles may still fill empty or fallback labels,
  but they cannot rename keep-alive sessions such as `samplenotes` or `erome systemd`.

## 2.6.4

- Move the alpha/blur/grain theme experiment out of the stable release path and
  ship a high-opacity, brightness-only theme editor. Alpha, grain, and blur
  settings are pinned off in stable builds so focus changes, hover states, and
  compositor differences cannot make the shell material nondeterministic.
- Accept current daemon screen snapshots for quiet remote Codex restores without
  requiring a manual switch pass, while keeping strict stale-retained replay
  guards for old scrollback.
- Replay the current daemon vt screen on initial attach for full-screen live TUI
  surfaces when the retained raw tail only contains incremental cursor-addressed
  deltas, while preserving real scrollback.
- Batch xterm.js user input through the terminal bridge and expose batch
  telemetry so fast typing, pasted text, and space-at-line-end regressions have
  deterministic app-control evidence.
- Harden KDE desktop identity by setting GTK/GDK process identity before launch
  and making `server app desktop-identity` fail when a pinned live app lacks
  current or rotated app-id trace proof.
- Retire duplicate old daemon PTY owners one runtime key at a time when a mixed
  old daemon owns both duplicated and unique preserved PTYs, and make the
  23-smoke fail on direct multi-daemon runtime ownership.
- Exclude the current daemon socket and its legacy aliases from duplicate-owner
  pruning probes so hot-update cleanup cannot block the daemon on its own
  request loop.
- Rename the Startpage local terminal action to `New Terminal`.

## 2.5.0

- Let native compositor blur show through the transparent shell by lowering the
  full-window material tint and gradient alpha only when the compositor blur
  region is active. The fallback path stays high-alpha for readability on
  backends without live blur.
- Tighten the background-blur smoke so native compositor blur, CSS material
  blur, and high-alpha fallback paths are checked as separate contracts.

## 2.4.56

- Preserve whitespace-only PTY output batches so remote shell prompts advance
  correctly when a user types standalone spaces in the xterm.js viewport.

## 2.4.55

- Update the Dioxus desktop stack to 0.7.9 while preserving Yggterm's
  vendored WebView/runtime patches for protocol probes, early visibility,
  WebKit compatibility, and direct-install desktop behavior.

## 2.4.54

- Retire duplicate or preserved-only stale hot-update daemons without issuing
  session shutdown. The monitor now uses a daemon-retire protocol path, with a
  guarded Linux process fallback for old sidecars whose live runtime keys are
  already owned by the current daemon.

## 2.4.53

- Cool restored remote terminal polling after the resume overlay is dismissed
  and the Yggterm window is unfocused. Preserved hot-update PTY owners should
  no longer be polled on the 220ms interactive cadence while the GUI is in the
  background.

## 2.4.52

- Reject remote Codex daemon PTY output as interactive when it contains an old
  prompt followed by assistant output and the current xterm cursor row is blank.
  Busy daemon output may still render during recovery, but input cannot be
  enabled until the current prompt/input row is visible.

## 2.4.51

- Reject the remaining hot-update restore failure where a remote Codex terminal
  accepted a large daemon PTY scrollback frame as interactive even though the
  cursor/input row was blank. App-control now keeps that state failed instead
  of enabling input on stale retained history.

## 2.4.50

- Stop post-ready daemon retained-snapshot replay from overwriting remote Codex
  xterm surfaces. Remote Codex resume now waits for live PTY/current prompt
  truth instead of replaying cursor-addressed screen snapshots after the prompt
  was already visible.
- Tighten app-control terminal readiness so a remote Codex surface with input
  enabled but a non-prompt cursor row is reported as a problem instead of
  accepted as interactive.

## 2.4.49

- Stop stale retained-scrollback diagnostics from remounting an already clean
  interactive daemon PTY surface. App-control can now report the old diagnostic
  string without starting another retained-fault recovery when the active
  viewport is already ready and input-enabled.

## 2.4.48

- Make the remote-resume clean-ready input gate idempotent. A terminal surface
  that is already marked ready, has no resume notification, and has synced
  local readiness signals no longer re-sends input-enable/state mutations every
  host-health tick, preventing the blank/remount loop seen after hot update.

## 2.4.47

- Treat daemon screen snapshots as authoritative retained terminal state. A
  visible prompt or scrollback in a reused xterm host no longer lets stale rows
  survive below the current Codex TUI after switch-back or hot-update recovery.

## 2.4.46

- Reconcile render-local terminal readiness signals after app-control has
  already accepted a clean daemon PTY surface. This fixes the remaining
  hot-update case where the terminal was visible and clean but Dioxus kept the
  xterm input bridge disabled until another render-side event happened.

## 2.4.45

- Open the input gate immediately when app-control has observed a clean visible
  daemon PTY surface and xterm host-health proves a current prompt. This avoids
  the delayed hot-update recovery state where the terminal looked correct but
  stayed input-disabled until a later retained snapshot/read pass.

## 2.4.44

- Accept daemon PTY Codex scrollback that ends in a real current prompt during
  hot-update recovery even when the prompt is not in the bottom three rows.
  This breaks the input-disabled resume deadlock without accepting sparse
  prompt-only surfaces as ready.

## 2.4.43

- Remove the server-prompt snapshot fallback from the terminal-ready path so
  session metadata can no longer arm input or mask a missing daemon PTY/xterm
  surface during hot update.
- Tighten retained Codex replay recovery so a visible cursor on a blank row is
  not accepted as prompt-ready; Codex replays must prove the current prompt row
  before the surface can settle.

## 2.4.42

- Preserve the full debug form of xterm bootstrap eval failures in the
  terminal incident trace so Dioxus communication errors expose their embedded
  JavaScript parse/runtime reason.
- Add a local syntax gate for the generated xterm bootstrap script by parsing
  it as the same `AsyncFunction("dioxus", body)` shape used by Dioxus desktop.

## 2.4.41

- Keep retained-terminal xterm bootstrap evals alive while their Dioxus host
  node is missing or being replaced. The bridge now waits for the real host
  instead of returning early, so hot-update restores cannot lose their first
  xterm mount before Rust starts reading eval events.
- Trace unexpected JavaScript eval returns separately from channel receive
  closure, exposing real bootstrap syntax/runtime failures in the terminal
  incident trace instead of collapsing them into `EvalError::Finished`.

## 2.4.40

- Start the long-lived xterm.js eval bridge before the awaited daemon
  `ensure` step during remote retained-session recovery. This keeps the
  desktop document channel alive while the daemon prepares the PTY and prevents
  blank hosts that close with `EvalError::Finished` before xterm can mount.
- Harden the xterm bootstrap script against a missing `dioxus` channel global
  so a bootstrap can report a real mount/assets problem instead of aborting
  before telemetry is emitted.

## 2.4.39

- Stop hot-update retained remote terminals from blanking themselves during
  first bootstrap: daemon/server prompt replay and the fast empty-surface
  watchdog now require a recorded xterm surface mount before they can mark a
  recovery ready or remount again.
- Preserve the fast recovery path for genuinely mounted-but-empty retained
  surfaces while giving replacement bootstraps the full startup window, so a
  slow remote `ensure` cannot be interrupted before xterm.js finishes mounting.

## 2.4.38

- Recover blank retained remote terminals from the daemon's live snapshot when
  a hot-updated GUI mounts an empty xterm host but the daemon still owns real
  PTY output. Managed live-session metadata remains barred from replay; the
  recovery source must be a fresh daemon snapshot for the active remote
  runtime.

## 2.4.37

- Stop unsafe cursor-addressed retained Codex frames from silently passing as
  ready when the xterm body is blank: Yggterm now replays an xterm-owned
  switchback snapshot or the daemon's VT screen snapshot, and otherwise keeps
  the retained-scrollback fault visible to app-control instead of accepting a
  prompt-only surface.

## 2.4.36

- Stop retained remote Codex recovery from re-entering the empty-surface
  watchdog loop after xterm has mounted: a real xterm paint now clears the
  fast remount condition, and prompt-ready retained Codex snapshots may render
  after geometry is established even when the idle PTY has not emitted fresh
  post-resize bytes.

## 2.4.35

- Stop retained remote Codex recovery from remounting a live xterm after the
  prompt is already visible: a mounted, geometry-valid prompt row now clears
  the poisoned retry state before the later attach marker arrives, preventing
  duplicate prompt bands, resize churn, and manual switching-pass recovery.

## 2.4.34

- Stop remote Codex saved-session semantic checks from auto-restarting a live
  runtime: explicit failed-resume output can still recover, but missing
  transcript fragments are now an observability signal instead of a kill loop.
- Allow a prompt-ready first Codex frame to render when pre-resize filtering
  would otherwise leave a freshly restored xterm blank after hot update.

## 2.4.33

- Fix fresh live SSH terminals that had a daemon PTY but no mounted xterm
  surface: bootstrap lease replacement now covers `live::` remote terminals,
  app-control reports that absent surface as a terminal fault, and terminal
  open attempts are recorded to the local telemetry SQLite database when
  telemetry is enabled.

## 2.4.32

- Recover blank retained terminals without requiring a manual switching pass:
  empty xterm-surface faults now keep their fault reason on the recovery
  attempt, use a fast retained-surface watchdog, and trigger an immediate
  daemon ensure/read cycle when the runtime already has output but the viewport
  is blank.

## 2.4.31

- Reduce Codex terminal fan load and first-paint fragility: retained/live
  synchronized repaint bursts now collapse to the latest xterm frame while
  preserving real scrollback, long-running status animation cools after the
  initial smooth window, and frame-budgeted repaint output no longer wakes
  remote preview bookkeeping.

## 2.4.30

- Keep idle focused terminals compatible with the low-power write policy:
  app-control now requires a hot input/output/animation signal before flagging
  a visible terminal for using the background write budget.

## 2.4.29

- Keep app-control budget checks aligned with the low-power policy: focused
  active terminals must use the fast write budget, while unfocused visible
  terminals may cool down without being marked as broken.

## 2.4.28

- Treat an unfocused live SSH shell prompt as a healthy visible terminal in
  app-control instead of a retained-session recovery failure.

## 2.4.27

- Arm the stale retained-terminal recovery watchdog even when a prior bootstrap
  lease is still present, so a preserved live terminal cannot remount as a
  blank/new-looking session after GUI relaunch.

## 2.4.26

- Stop the Linux transparent-window blur/shape pass from reapplying on every
  root render; the compositor blur helper now redraws only when it creates or
  changes the blur region.
- Add a hard rearm path for stale retained-terminal recovery attempts so a
  kept live SSH/Codex terminal cannot stay forever in attach-in-flight with no
  mounted xterm after a GUI/server relaunch.

## 2.4.25

- Lower the CPU cost of the external Codex owner wait loop by filtering
  candidate `codex resume` processes before walking `/proc` ancestors and by
  polling at a slower safety interval.

## 2.4.24

- Guard daemon-owned Codex resume against an already-active external
  `codex resume <session>` process so Yggterm waits instead of starting a
  second writer for the same transcript.
- Treat the external-active wait notice as an intentional blocked terminal
  state, not a failed resume that should be restarted in a loop.
- Add regression coverage for external Codex owner detection, Yggterm-owned
  descendant exclusion, and the external-active terminal guard message.

## 2.4.23

- Distinguish saved daemon-owned Codex runtimes from true fresh starts when
  repairing `codex-runtime://<session-id>` launch commands, so a kept saved
  session resumes without turning every fresh daemon runtime into a resume.
- Reject a hot-update preserved owner when its visible PTY output is a fresh
  Codex surface for an existing saved session, then restart that runtime through
  the corrected `codex resume` path instead of preserving the wrong PTY.
- Extend saved-session output mismatch detection to `codex-runtime://...` keys,
  covering the dev-side daemon that owns jojo's remote Codex terminal stream.

## 2.4.22

- Repair restored daemon-owned remote Codex runtime rows so
  `codex-runtime://<session-id>` always relaunches with persistent
  `codex resume` semantics after keep-alive restore or hot update, even if
  stale metadata still says the original action was `start-codex`.
- Strip stale fresh-start metadata from restored Codex runtime rows before the
  daemon resolves the terminal spec, preventing a kept session from appearing
  as a brand-new Codex prompt until close/reopen.

## 2.4.21

- Restart a remote Codex bridge when a restored saved-session runtime has real
  output but that output does not match the requested Codex session after the
  attach grace window, so a keep-alive session cannot settle as a fresh-looking
  Codex prompt until close/reopen.
- Make no-op prompt-follow viewport repairs skip full xterm refresh when the
  terminal is already at the target scroll position, preventing click-to-focus
  from producing short scroll/flicker bursts.

## 2.4.20

- Reject startup hot-update handoff from a stale daemon that only represents a
  partial subset of persisted live sessions, preventing older preserved owners
  from overwriting `server-state.json` and dropping generic SSH terminals such
  as `practice ssh`.
- Treat generic `live::...` SSH sessions as remote runtimes for terminal
  recovery, so keep-alive SSH shells take the same remount/rearm path as
  `remote-session://...` rows instead of reusing stale drawing state.

## 2.4.19

- Re-arm the active xterm.js write budget from terminal input focus and
  input-hot state, fixing Wayland/app-control cases where a selected terminal
  kept the 4000ms background cadence and appeared to drop trailing spaces.
- Extend terminal input probes to treat whitespace-at-end as cursor advancement,
  not visible text, so the smoke harness catches end-of-line space regressions.

## 2.4.18

- Keep terminal paste single-owned by Yggterm's native clipboard bridge so text
  paste cannot be delivered once by xterm.js and again by the app-control paste
  path, while leaving image paste on the existing file-transfer path.
- Tighten active Codex inline-status frame budgets and app-control budget
  diagnostics so sustained "Working" animations remain responsive without
  turning focused terminals into high-latency repaint loops.
- Constrain the theme editor controls inside the modal surface and add smoke
  coverage for the brightness, alpha, and grain dials.

## 2.4.17

- Cool sustained Codex synchronized repaint animations onto the long-running
  animation frame budget, reducing jojo GUI/WebKit CPU while keeping prompt and
  TUI output on the native xterm.js path.

## 2.4.16

- Preserve durable SSH targets and remote machine stubs across update handoff
  snapshot syncs, so a successfully connected zero-session machine such as
  `practice` remains in the sidebar after restart instead of being treated as
  transient live-row state.

## 2.4.15

- Keep Codex synchronized-output row repaints on the frame-budgeted xterm.js
  path even when they contain cursor-addressed row clears, fixing the active
  typing latency/resource spike that still slipped through 2.4.14.

## 2.4.14

- Require SSH Connect to verify or bootstrap the remote Yggterm binary before
  creating/focusing a live terminal, so a missing `~/.yggterm/bin/yggterm` on
  targets such as `practice` reports a real connection failure instead of a
  false success plus disappearing sidebar entry.
- Keep bulk Codex repaint batches on the async xterm.js write path after
  coalescing, reducing focused GUI/WebKit latency spikes without adding a
  Yggterm-owned terminal overlay.

## 2.4.13

- Coalesce repeated Codex synchronized-output repaint frames before they reach
  xterm.js, and cool sustained inline-status animation budgets after the first
  few seconds so active “Working” prompts stay native but stop pegging jojo's
  GUI/WebKit CPU.

## 2.4.12

- Repair remote terminal geometry after update/restart by verifying the kernel
  PTY size before accepting same-size resize no-ops, and hold remote SSH resume
  launch until the xterm viewport resize has time to reach the PTY.

## 2.4.11

- Cool active terminal write/read budgets when the Yggterm window is unfocused,
  so Codex inline-status animations do not keep WebKit hot in the background
  while preserving the focused xterm-native animation path.

## 2.4.10

- Add KDE/KWin's Wayland `org_kde_kwin_blur_manager` fallback for native
  compositor blur on Plasma sessions that do not advertise the newer
  `ext-background-effect-v1` protocol.

## 2.4.9

- Add native KDE/Wayland `ext-background-effect-v1` blur regions for transparent shell windows, exposing compositor blur truth through app-control while keeping CSS material blur limited to in-window chrome.

## 2.4.8

- Split CSS material blur from actual compositor live blur in app-control truth, and use a no-filter material fallback for transparent Linux shells so KDE/Wayland cannot degrade into unstable alpha-only chrome or burn CPU on full-window CSS blur.

## 2.4.7

- Make active Codex inline-status reads obey the same xterm frame budget used for writes, reducing WebKit wakeups during long “Working” animations while keeping the terminal on the PTY/xterm path.

## 2.4.6

- Stop the terminal input-policy effect from rebuilding broad shell snapshots and rereading `session-titles.db` on every render tick, fixing the jojo 2.4.5 GUI main-thread CPU loop while preserving hot-update daemon PTYs.

## 2.4.5

- Lower the default active Codex inline-status animation cadence to a 10 FPS xterm-native budget, keeping the “Working” line responsive without driving WebKit CPU continuously during long tasks.

## 2.4.4

- Restore retained remote terminal readiness when a clean daemon PTY surface is visible but unfocused, preventing stale recovery state from remounting Codex sessions and collapsing the viewport during session switches.

## 2.4.3

- Make startup hot-update authorization prefer persisted live-session runtime keys over stale preserved-owner registry entries, so a stale handoff cache cannot cause the new daemon to re-resume and interrupt live Codex PTYs.


## 2.4.2

- Keep retained live xterm bridges mounted across session switches while pausing hidden reads, preventing switch-back from resetting xterm and collapsing scrollback.
- Strengthen the remote switch smoke so `base_y` and retained terminal text cannot collapse after switching away and back.

## 2.4.1

- Recover stale xterm canvas layers after active live-session switches with a bounded xterm-native activation repaint, and add smoke coverage for history-backed blank upper canvases.
- Add theme alpha as a first-class Yggui theme scalar, wire brightness/alpha/grain through app-control, and harden the theme editor smoke against no-op controls and bright modal wash.
- Make autohidden titlebar reveal use integrated translucent chrome with blur and no content shift, with smoke assertions for background, gradient, blur, and terminal-grid stability.
- Move Ghostty/backend and yggterm-headless panic-management notes into durable docs and remove the duplicate `agent_docs/` source.

## 2.4.0

- Blend the autohidden titlebar with the same shell chrome background/gradient as the visible titlebar while keeping hover reveal from resizing the terminal grid.
- Deduplicate live-session close confirmation rows when the same runtime is also projected under a cwd group.
- Harden Codex session title/summary generation so malformed labels such as quoted bad generated titles and tiny summary fragments are rejected before they reach the sidebar.
- Extend `yggterm-headless server sessions regenerate-copy` to refresh local plus app-discovered remote Codex session copy, support remote-only `--skip-local` release-gate runs, and reset remote summary timelines when requested.
- Keep remote `regenerate-copy` non-force retries cached so release smokes do not re-run every remote precis/summary generation job after a successful reset pass.
- Keep daemon-owned live runtimes durable while limiting fresh GUI xterm retention to the active terminal or already-mounted render state, fixing the 23-session restore case where daemon truth existed but the active xterm host was not mounted yet.
- Disable SSH ControlMaster multiplexing for new Yggterm remote PTYs so closing smoke/test sessions cannot interact with an unrelated user SSH master.
- Treat connected daemon-PTY output as a valid visible terminal surface even when Codex is busy and no prompt row is visible, preventing false resume overlays on readable retained sessions.
- Make the 23-smoke heavy-TUI detector accept real `codex-session-tui` Browser/Preview frames while rejecting command-echo-only false positives.
- Gate Linux-only Dioxus DMA-BUF workaround code and Unix process-extension imports behind the correct cfgs, and prevent the release workflow from publishing partial assets when any package leg fails.
- Document the 23-smoke release gate, including resource budgets, terminal quirks, restore checks, and title/summary quality checks.

## 2.2.66

- Treat a running same-version daemon whose Linux `/proc/<pid>/exe` target is reported as the active install path plus ` (deleted)` as current, preventing remote helper reinstall from spinning on false stale-daemon detection.
- Keep stale-daemon lifecycle detection in trace evidence instead of warning into terminal-attached helper commands.
- Add regression coverage for deleted-path same-version daemon detection in both app and daemon cleanup paths.
- Use the short UUID fallback, not a generic `Codex Session` label, when remote scanned sessions still have no meaningful generated copy.

## 2.2.65

- Derive remote stored-session sidebar labels from cached summaries or recent transcript context before falling back to a generic `Codex Session` label.
- Keep app-control synthesized remote rows on the same generated-title path as the visible sidebar rows.
- Add regression coverage for summary/recent-context title fallback after generic remote labels are rejected.

## 2.2.64

- Treat generic `Yggterm Codex` / `Yggterm Shell` sidebar labels as generated fallbacks, so generated remote titles from `session-titles.db` can replace them instead of being blocked as if they were user-authored names.
- Add regression coverage for replacing generic remote Codex rows with generated titles.

## 2.2.63

- Keep passive title/summary/precis generation bounded per session instead of globally suspending all background copy work after one no-context or failed transcript.
- Add regression coverage so passive copy failures stay on the per-session retry path and cannot disable the scheduler.

## 2.2.62

- Treat wrapped `Error: connecting to .../server-*.sock` output from stale remote Yggterm daemons as terminal transport failure, not meaningful Codex output.
- Extend remote Codex hot-update recovery so rejected preserved-owner surfaces, including stale socket errors and generic title-card output, cannot linger as progress and trigger one controlled force-remote restart after the hard-fail window.
- Add regression coverage for the jojo stale-socket Codex surface and rejected preserved-owner restart decision.

## 2.2.61

- Treat remote Codex prompt-only hot-update surfaces as recovery failures, not ready terminals, and perform one controlled force-remote restart after the hard-fail window instead of lingering on a sparse prompt.
- Keep full-screen cursor-home terminal frames on the frame-budgeted xterm path while preserving the faster Codex inline status animation path.
- Add regression coverage for prompt-only Codex handoffs, attach confirmation, hard-fail restart decisions, and full-screen frame budgeting.

## 2.2.60

- Make `server sessions ... --help` non-mutating for both the GUI launcher CLI and `yggterm-headless`, so asking for title/summary bookkeeping help cannot accidentally run a regeneration pass.
- Add CLI regression coverage for the sessions help path that slipped during jojo bookkeeping validation.

## 2.2.59

- Let the hot-restart monitor retire a stale duplicate daemon when an expected-version daemon already owns the same terminal runtime keys, preserving the sessions without keeping an obsolete sidecar alive forever.
- Report owned terminal runtime counts/keys in monitor JSON and add regression coverage for the duplicate-owner cleanup decision.

## 2.2.58

- Keep raw xterm protocol input separate from user-input readiness in app-control: a busy remote Codex PTY with visible daemon output but no focused prompt now settles as visible, not as an input-enabled terminal problem.
- Add regression coverage for the jojo post-update busy-output state where the raw xterm bridge is open while the user prompt is not ready.

## 2.2.57

- Route app-control session removal through the same live-session close fallback contract as the sidebar close affordance, so active close returns to validated viewport history and background close leaves the viewport unchanged under automation too.
- Add regression coverage for the app-control live-close path that the jojo proof harness uses.

## 2.2.56

- Accept current daemon-owned remote PTY output as a visible live terminal even when Codex is still busy and no prompt row is present yet, avoiding a false stale-retained recovery gate after hot update.
- Add a regression fixture for the jojo busy-Codex resume state: real daemon PTY bytes, input still gated, no current prompt row, and a stale resume notification.

## 2.2.55

- Define and enforce live-session close navigation: active close now falls back through validated viewport history, background close leaves the viewport alone, and closed paths are pruned so chained closes cannot return to dead sessions.
- Stop the daemon from choosing an arbitrary replacement active session after runtime removal; the GUI now makes any close-time focus choice explicitly from viewport history.
- Pump bounded passive title/summary generation from the GUI background loop so missing session copy can converge after startup/snapshots without a manual row click.
- Remove the `Connect SSH` action from Startpage and document Startpage as a recent/scoped local work surface.

## 2.2.54

- Settle focused restored xterm cursors through a throttled prompt-follow repaint even when the active input-policy update is otherwise unchanged, so the cursor does not stay on an old row until the first typed byte.
- Expose and smoke-check the input-policy no-op prompt-follow counter for Codex prompt layouts, making the cursor-settle path observable before publishing.
- Strip and classify internal stale-daemon startup warnings as terminal transport noise, preventing version-handoff diagnostics from appearing in the active PTY after update.
- Bound the no-op cursor-settle repair per mounted terminal and make the read-only latency smoke fail if it keeps repainting while no terminal writes are happening, preserving the fan/idle budget.

## 2.2.51

- Treat stale remote-runtime hot-update failures as preserved-owner fallback, so a retryable bridge/update error cannot stall the active Codex PTY before attachment.
- Classify leaked hot-update bridge errors as terminal transport output in app-control, making the 2.2.50 jojo failure release-blocking instead of passable as retained text.
- Reject same-version daemons that are running from deleted or non-current executables during mutating recovery, and keep CLI JSON output parseable by sending recovery logs to stderr.
- Add `docs/protocol.md` with the hot-update/session-preservation contract and mark active PTY loss during update as a protocol violation.

## 2.2.50

- Restore visible autohide titlebar chrome on hover while preserving the terminal grid height, so hovering the hidden titlebar no longer triggers terminal resize churn.
- Add the canonical session identity/title/summary contract in `docs/sessions.md`, including UUID fallback rules and timeline-style summary history.
- Persist summary timelines, expose a headless `server sessions regenerate-copy` pipeline, and enable budgeted passive copy generation by default so new Codex sessions converge away from generic or short-UUID titles.
- Show long UUIDs and pencil edit actions on Startpage session cards, and render titlebar summaries as a selectable timeline.
- Stop terminal pointer-release focus repair from forcing prompt-follow scroll, avoiding the click-induced viewport flicker seen on jojo.

## 2.2.49

- Treat app-control open as a foreground terminal intent after an app-control background/cooling pass, so switch-back smokes cannot leave the target terminal in a cooled blank state.
- Add regression coverage for the exact background-then-open path that hid a live htop TUI during jojo proof.

## 2.2.48

- Keep not-yet-measured and inactive alternate-screen TUI frames on the xterm.js path when the low-power TUI renderer is disabled, fixing blank htop/top startup and switch-back surfaces without adding an overlay renderer.
- Tighten the terminal bridge smoke assertion so disabled low-power TUI mode cannot still drop offscreen frame-like PTY bytes.

## 2.2.47

- Force one xterm renderer refresh when the first observed terminal visual state is an alternate-screen TUI or hidden-cursor TUI, preventing htop/top from existing in xterm's buffer while WebKit's canvas remains blank until a manual redraw.
- Add regression coverage for the first alternate-screen TUI paint path that slipped through the prior smoke pass.

## 2.2.46

- Keep xterm scroll anchoring and titlebar auto-hide on the xterm viewport contract, so titlebar reveal and small resizes no longer force terminal grid remounts or throw the viewport into stale scrollback.
- Preserve remote Codex session identity when daemon-owned live sessions close, promoting synthetic runtime paths back to real Codex transcript ids so sidebar rows, shutdown, rename, and summary truth stay durable.
- Replace short UUID-style sidebar fallbacks with cwd/kind-derived labels, and make manual title plus summary edits write through the same durable title store.
- Add startpage/titlebar/context actions for session title and summary edits, plus a startpage entry point for creating scoped folders.
- Add a YggUI scroll controller over xterm.js viewport APIs for page/top/bottom navigation while keeping terminal text, cursor, prompt, and redraw owned by xterm.js.
- Extend app-control and smoke coverage for retained replay, scroll controller state, prompt-ready unsafe-skip diagnostics, and manual copy-edit entry points.

## 2.2.45

- Treat prompt-ready retained xterm surfaces as live PTY truth when a large cursor-addressed snapshot is intentionally skipped after resize, so app-control does not gate input or trigger recovery on a usable Codex prompt.
- Expose retained-replay unsafe-skip diagnostics in app-control state for resize and scrollback smoke coverage.

## 2.2.44

- Keep background managed-Codex refresh probe-only by default, so live terminal recovery and remote scans cannot spawn `npm install @latest` and blow the fan/CPU budget. Unattended background installs now require `YGGTERM_MANAGED_CLI_BACKGROUND_INSTALL=1`.

## 2.2.43

- Sync daemon terminal identity from the effective xterm theme before warm-start and initial restore, so a light Yggterm shell using a dark terminal theme launches remote Codex with dark `YGGTERM_TERMINAL_APPEARANCE`/`COLORFGBG` instead of producing low-contrast prompt bands.
- Let app-control terminal focus use the active xterm write budget even when Wayland refuses native window focus, keeping terminal readiness smokes from conflating compositor focus with terminal interactivity.

## 2.2.42

- Require direct PTY ownership before retiring an older hot-update daemon; preserved/runtime-known keys no longer count as proof that the updated daemon has adopted the session.
- Tighten duplicate-owner tests so cleanup cannot kill a preserved owner while the current daemon only has metadata for that runtime.

## 2.2.41

- Retire stale hot-update daemons when the current daemon already owns the same live runtime keys, preventing duplicate daemons from re-registering themselves as preserved owners after `hot-restart --all`.
- Tighten cleanup coverage for preserved-owner registries so session survival does not become a second source of runtime truth once the updated daemon has taken over.

## 2.2.40

- Seed retained remote Codex scrollback for the restored active terminal during startup prewarm, while keeping background prewarm on the lighter latency path.
- Add prewarm coverage for the active-versus-background remote snapshot contract so a prompt-only restored viewport cannot be mistaken for a ready terminal after hot update.

## 2.2.39

- Cool active terminal read/write cadence when the Yggterm window is unfocused, while keeping remote resume on the fast path only until the restore surface is connected.
- Make xterm write-budget observability reflect document focus so an unfocused visible terminal no longer masquerades as an active 160 ms render budget.

## 2.2.38

- Treat terminal stream cursor rewind as a runtime restart boundary: the daemon replays initial chunks when a client cursor belongs to a previous runtime, and the GUI clears/remounts the stale xterm host instead of preserving old broken pixels after a forced restart.

## 2.2.37

- Keep remote Codex bridge PTYs in raw input mode while restoring `opost onlcr` output newline processing, so bare-LF Codex/TUI frames do not repaint as diagonal/right-shifted line stacks in xterm.js.
- Normalize captured terminal snapshot emission to CRLF before writing it back through a PTY, matching terminal line-discipline semantics without adding a renderer overlay.

## 2.2.36

- Tighten active-recovery snapshot replay for remote Codex/TUI sessions so compact cursor-addressed full-screen snapshots are rejected before they can repaint old-width line stacks.

## 2.2.35

- Stop the daemon from using its vt100 side-parser snapshot as the initial viewport replay for remote Codex/TUI sessions; xterm now receives retained raw PTY bytes only, and attach-ready is emitted only after the remote helper actually reports it.

## 2.2.34

- Stop titlebar auto-hide reveal from resizing the terminal canvas; the chrome now overlays the top edge instead of adding content padding.
- Disable the lossy low-power TUI overlay/frame-drop path so inactive htop-like TUIs keep flowing through xterm.js rather than returning to a stale screen.
- Reject cursor-addressed multi-row Codex recovery snapshots that can repaint old-width output over a good live xterm surface.

## 2.2.33

- Rearm active keep-alive remote terminals immediately when daemon snapshot truth says the runtime is in recovery/bootstrap, even if the retained xterm host still has an old ready ledger entry.
- Tighten the read-only latency smoke so an active remote terminal with no daemon runtime cannot pass as healthy while stale retained text remains visible.

## 2.2.32

- Mark keep-alive remote sessions whose daemon runtime is missing as recovery/bootstrap targets instead of `Running`, so hot-updated clients remount and recreate the PTY rather than reusing stale retained xterm content.
- Make the shell refuse retained-host reuse for active remote sessions in a recovery launch phase, closing the gap where app-control correctly reported `active_runtime_present=false` but the viewport stayed stuck on old scrollback.

## 2.2.31

- Treat `codex resume ...` continuation text in a remote Codex viewport as an exited runtime, not as an interactive terminal, and restart the daemon-owned PTY instead of accepting the stale surface.
- Stop advertising exited daemon PTYs as live runtime truth, so app-control, preserved-owner handoff, and smoke tests cannot pass while the active terminal process is already gone.
- Tighten the terminal status smoke to reject Codex resume-instruction surfaces and avoid using Ctrl+C as a prompt-clear shortcut on live Codex sessions.

## 2.2.30

- Start forced remote Codex restarts at the mounted xterm geometry and ignore same-size resize nudges, so the hot-update recovery path does not immediately mark fresh restart output as pre-resize.

## 2.2.29

- Recover blank hot-update remote Codex handoffs by treating filtered pre-resize output as non-progress and performing one controlled force-remote restart instead of spinning on an empty xterm.

## 2.2.28

- Stop treating hot-update remote Codex pre-resize PTY scrollback as current terminal truth during preserved-owner handoff, so old-width retained output cannot block the resize/recovery path.
- Make app-control and the read-only latency smoke reject gated remote Codex recovery tails with no current input row, even when user input is already disabled.

## 2.2.27

- Reject input-enabled remote Codex recovery surfaces when the visible tail contains an old prompt followed by assistant output instead of a current input row, and route that state through retained-surface recovery instead of accepting it as interactive.

## 2.2.26

- Treat retained active-recovery PTY prompt-follow snapshots as visible terminal truth after hot update, so stale resume notifications clear only when the mounted xterm surface contains real PTY output and never by blessing stale scrollback.

## 2.2.25

- Reject input-enabled remote Codex PTY scrollback when the cursor row is blank and the latest daemon frame is only terminal control traffic, and remount/recover that surface instead of treating stale scrollback as current prompt truth.

## 2.2.24

- Accept daemon-owned remote Codex PTY scrollback as prompt-ready when xterm exposes visible cursor geometry and real Codex output after a hot-update handoff, even if the cursor row text is empty after replay.

## 2.2.23

- Treat a hot-update retained PTY replay as terminal-ready when app-control proves prompt-followed real PTY output even if the cursor line text is empty after pre-resize replay, so preserved Codex sessions regain input after GUI replacement.

## 2.2.22

- Preserve hot-update Codex viewports when the session owner has not produced post-resize output yet by allowing retained pre-resize PTY bytes only for explicit preserved-owner handoff keys; ordinary resize recovery still waits for post-resize output.

## 2.2.21

- Remount a hot-updated remote terminal only when app-control proves the retained xterm host is mounted but empty, preserving the generic remote attach guard while recovering blank live Codex viewports after a session-preserving GUI/daemon update.

## 2.2.20

- Route terminal right-click through xterm.js into Yggterm's existing terminal context menu, and add app-control/smoke proof for right-click plus middle-click terminal shortcuts.
- Lower the active visible terminal write-frame budget to keep typing and Codex TUI animation responsive while preserving heavy coalescing for background terminals.

## 2.2.19

- Add xterm-owned primary-selection paste: selecting terminal text records a terminal-local primary selection, and middle-click pastes it through xterm.js without touching the clipboard.
- Keep PromptFollow terminals pinned to the live bottom across titlebar auto-hide hover/layout resizes, so chrome nudges cannot leave the viewport at the top of scrollback.

## 2.2.18

- Keep xterm scroll events caused by command output from being classified as user scrollback while input or write flushing is hot, so `/status`-style terminal output stays prompt-following after it renders.

## 2.2.17

- Fence remote Codex restore/replay on post-resize daemon PTY output, so old-width retained TUI separators do not settle as the current xterm screen after resize.

## 2.2.16

- Keep remote Codex visual-reveal recovery gated until post-attach output proves a Codex prompt-ready surface, preventing stale transcript/prose bytes from marking a restarted terminal interactive.

## 2.2.15

- Reject stale active-recovery Codex snapshots unless they prove a real Codex prompt-ready tail, so old transcript/prose text cannot be replayed into a restarted live terminal as if it were the current PTY screen.

## 2.2.14

- Treat daemon-retained replay as post-ready scrollback repair only: a remote session must have a clean observed interactive viewport before retained daemon scrollback can be replayed, preventing stale Codex transcript text from filling the terminal during restart/reconnect.

## 2.2.13

- Block daemon-retained replay while a remote resume notification is still active, preventing stale Codex transcript/prose from painting under the reconnect overlay before the live prompt-ready surface arrives.

## 2.2.12

- Add `LogLevel=ERROR` to Yggterm-owned remote SSH terminal launch commands so OpenSSH control-master close notices such as `Shared connection ... closed.` cannot leak into Codex xterm content when a remote bridge exits or is interrupted.

## 2.2.11

- Keep xterm host-health throttling from suppressing the all-empty retained remote surface sample, so the live mount can remount a blank retained Codex viewport before any app-control state probe observes it.

## 2.2.10

- Let the live terminal mount itself remount a previously-ready retained remote xterm when host-health sees an all-empty active surface, so recovery does not depend on an external app-control state probe.

## 2.2.9

- Remount retained remote xterm surfaces immediately when app-control observes an empty active xterm after update/restart, while keeping the two-sample guard for ambiguous scrollback checks.

## 2.2.8

- After a forced remote restart takes over preserved hot-update sessions, rerun Linux daemon cleanup so older duplicate daemons that only advertise already-owned terminal keys are retired instead of remaining as a second source of runtime truth.

## 2.2.7

- Make forced remote terminal restarts terminate plain `yggterm server remote resume-codex/start-codex <session>` bridge processes on the remote host, so halted keep-alive Codex sessions are actually recreated instead of reattaching to stale cached TUI state.

## 2.2.6

- Answer OSC 10/11 default foreground/background color queries in the daemon PTY path before GUI attach, and strip those terminal-emulator queries from retained output so Codex can render prompt backgrounds from real xterm cell attributes instead of shell overlays.

## 2.2.5

- Make remote bootstrap prefer the active direct-install headless binary from install metadata instead of the caller process's adjacent binary, preventing an older preserved daemon from reinstalling an old helper onto a remote machine during hot update.

## 2.2.4

- Let forced terminal restart recover a remote Codex session from the scanned remote-session row before terminating or recreating the runtime, so a partially migrated live membership does not make a known session unrestartable.

## 2.2.3

- Give forced remote terminal restarts the long daemon response budget so headless control does not time out while the server is safely terminating a remote Codex runtime before recreating it.

## 2.2.2

- Carry terminal appearance into remote-runtime daemon requests so a dark jojo xterm session does not ask a stale remote dev daemon to launch Codex with light `COLORFGBG`/terminal identity, and make remote Codex termination scan versioned daemons before a forced restart.

## 2.2.1

- Refresh daemon-owned remote Codex launch commands when the GUI syncs terminal identity, and add a headless `server terminal restart` path so halted kept sessions can be restarted with the current xterm identity instead of preserving stale `COLORFGBG`/terminal-appearance exports.

## 2.2.0

- Ship the terminal stabilization line: byte-exact app-control terminal sends, app-control-backgrounded low-power terminal budgets, stricter idle CPU/resource probes, and second-display proof for terminal readability, cursor visibility, input, scrollback, resize, and inline status animation smoothness.

## 2.1.250

- Let app-control terminal sends read multiline/control-byte payloads from stdin, and make the idle CPU smoke use that byte-exact path so terminal proof commands cannot be corrupted by shell-quoted carriage returns.
- Accept an app-control-backgrounded TUI sample with no new xterm frames only when the state proves input is disabled, active write budgets are off, xterm counters are flat, the PTY workload is still alive, and CPU is under budget.

## 2.1.249

- Slow the unfocused TUI drop-drain cadence and pause sidebar loading animations while app-control has backgrounded a proof window, keeping background resource samples quiet without changing active terminal behavior.

## 2.1.248

- Make app-control-backgrounded terminal hosts publish the inactive write-budget truth in app-control state, and make the idle CPU smoke fail if a lowered proof window still reports active terminal input or active write budgets.

## 2.1.247

- Treat app-control-backgrounded windows as inactive for terminal read/write budgets and input policy, and cap background multiline read bursts, so lowered smoke windows do not keep consuming active-terminal CPU after launching TUI probes.

## 2.1.246

- Wake the terminal read loop during app-control multiline sends and report write-chunk/read-nudge telemetry in the response, so background terminal probes can distinguish a missing PTY write from a stale mounted xterm surface.

## 2.1.245

- Make daemon terminal writes acknowledge only after the PTY writer thread flushes the bytes, so app-control multiline commands cannot report success while the heredoc terminator is still queued behind the daemon writer.

## 2.1.244

- Pace app-control multiline terminal sends as line writes, avoiding PTY echo backpressure that could leave background heredoc/TUI probes visibly stuck partway through a command despite the send being accepted.

## 2.1.243

- Keep the visible active terminal on active read cadence even when toolkit window-focus truth is stale, preventing selected terminals from degrading to background-FPS output after app-control focus/background transitions.

## 2.1.242

- Give app-control multiline terminal sends a longer bounded fast-read window, so background heredoc/TUI commands do not exhaust the short input-echo burst before real command output and frame-drop telemetry arrive.

## 2.1.241

- Normalize app-control terminal-send newlines to PTY Enter bytes before writing, so multiline heredoc/TUI probes execute after the terminator instead of visibly stopping at the secondary prompt.

## 2.1.240

- Preserve multiline app-control terminal sends as one PTY payload, splitting only around Ctrl-C, so long heredoc/TUI probes cannot report the whole command accepted while the background terminal only receives the first prompt line.

## 2.1.239

- Keep the terminal read loop on the fast input-echo cadence across the first non-empty command echo, so background app-control sends for multiline/TUI commands do not fall back to the slow unfocused poll before real output arrives.

## 2.1.238

- Acknowledge app-control terminal read nudges before reporting daemon-side input writes as complete, making background terminal sends wake the xterm read loop deterministically for resource and latency probes.
- Tighten the Linux idle CPU smoke's background TUI detector to compare low-power frame counters against a per-phase baseline instead of treating stale foreground counters as proof.

## 2.1.237

- Keep active inline terminal status animations on the low-latency read-poll cadence even when toolkit focus is stale, so Codex `Working`-style redraws keep producing smooth xterm flushes.

## 2.1.236

- Wake the mounted xterm read loop after app-control terminal sends, so daemon-side injected input paints promptly instead of waiting on the unfocused idle poll.

## 2.1.235

- Include terminal font family, weight, line-height, and contrast settings in terminal fallback app-control snapshots so readability smokes keep enforcing the viewport typography contract.

## 2.1.234

- Preserve live-session close/keep-alive affordance geometry in terminal fallback app-control snapshots, keeping sidebar contract probes strict even when terminal activity forces the lightweight DOM path.

## 2.1.233

- Keep sidebar row truth available in terminal-focused app-control fallback snapshots, so live-session tree smokes still prove the sidebar contract when full DOM probes time out during terminal activity.

## 2.1.232

- Keep viewport probe typing on the same terminal busy contract as real input, so a foreground command injected through app-control flips the live-session busy row and cannot be hidden by stale idle daemon snapshots.

## 2.1.231

- Tighten terminal typing observability so app-control only reports visible input when the marker is in the viewport or visible cursor line, rejects inactive hosts, and smoke-tests typing from scrollback back to the prompt.

## 2.1.230

- Carry the effective terminal palette identity through daemon terminal-start requests, so fresh local/remote Codex launches export dark `YGGTERM_TERMINAL_APPEARANCE`/`COLORFGBG` when the viewport is dark even if the outer shell is light.

## 2.1.229

- Match Codex launch identity to the effective terminal palette instead of the shell chrome theme, so dark terminal viewports advertise dark `COLORFGBG`/`YGGTERM_TERMINAL_APPEARANCE` even inside the light Yggterm shell.
- Add a canvas-rendered Codex input-line band plus app-control/smoke assertions for its visibility, catching missing prompt contrast before release.

## 2.1.228

- Clear stale remote resume notifications and attach gates once app-control sees clean daemon-owned PTY output, so a hot-updated terminal does not remain visible-but-input-disabled after the overlay disappears.

## 2.1.227

- Strip cursor-addressed internal terminal attach errors that are appended after xterm control sequences in retained daemon snapshots, preserving useful prompt/footer text while removing stale `terminal session not found` replay residue.

## 2.1.226

- Strip wrapped internal SSH attach footers such as bare `Shared connection to ...` fragments from retained/live terminal replay and classify them in app-control/smoke probes, so contaminated Codex surfaces cannot reopen with stale transport lines after hot update.

## 2.1.225

- Hard-clear the xterm screen and scrollback before sanitized retained/live replay when a visible internal transport leak is detected, avoiding stale painted attach errors that survive xterm reset.

## 2.1.224

- Reset a contaminated visible xterm buffer before applying clean live terminal writes, so stale internal attach failures from a prior hot update cannot remain painted after the daemon resumes streaming real output.

## 2.1.223

- Suppress remote resume transport-error batches even after the resume overlay has been dismissed, preventing the live read loop from writing internal attach failures into xterm while input remains gated.

## 2.1.222

- Strip internal retained-replay transport residue even when Codex/xterm rewrites place the error after a bare carriage return, closing the remaining hot-update leak that could keep `terminal session not found` visible and input-gated.

## 2.1.221

- Reject already-visible retained xterm buffers that contain internal attach/SSH transport residue, forcing sanitized daemon replay instead of letting a hot-update restore reopen on `terminal session not found` or `Shared connection ... closed` lines.
- Add CI-focused guards for the retained replay rejection path and KDE Wayland transparent corner profile so these regressions fail before release packaging.

## 2.1.220

- Treat orphaned line-shaped SSH close text as replay-only transport residue during retained daemon-terminal restore, so a previously contaminated hot-update replay cannot keep reopening to `Shared connection ... closed` as the active cursor line.

## 2.1.219

- Strip the paired `Shared connection to ... closed` line from contaminated retained terminal replay even when escape traffic or blank lines separate it from the internal attach error, while keeping ordinary user transcripts with SSH close text intact.

## 2.1.218

- Sanitize retained daemon-terminal replay before script-based restore and direct xterm snapshot writes, preventing hot-update rehydration from showing stale internal `terminal session not found` transport lines in live Codex terminals.

## 2.1.217

- Apply the internal terminal attach-error sanitizer to raw terminal-control payloads too, so retained Codex/xterm replay cannot leak `terminal session not found` lines through the control-forwarding path after hot update.

## 2.1.216

- Strip leaked internal terminal attach errors from live terminal replay, detect tail-position `terminal session not found` transport leaks in app-control/read-only smokes, and keep the surrounding user scrollback intact.
- Restore lightly rounded KDE Wayland window corners by using the transparent Wayland profile, and require final xterm smoke screenshots to run the corner proof.

## 2.1.215

- Retarget stale versioned socket symlink aliases without pinging their old target during daemon startup, preventing a failed update sidecar from slowing or blocking the next hot-update attempt.

## 2.1.214

- Avoid hot-update daemon startup deadlocks when old versioned socket aliases already point at the freshly bound daemon socket, so the updated daemon does not ping itself before its accept loop is running.

## 2.1.213

- Keep Codex inline `Working` animations on the low-latency terminal path across split/color-only rewrite frames, lower the active animation budget to 40ms, and add smoke coverage that fails when app-control sees a hot inline status animation using the slower TUI budget.

## 2.1.212

- Defer Linux legacy-daemon cleanup until after the new daemon has bound its current socket, preventing stale wedged sockets from blocking fresh daemon startup during hot update.

## 2.1.211

- Dispatch accepted Unix daemon clients off the accept loop and collect request outcomes asynchronously, so one partial or wedged GUI connection cannot block ping, status, hot-update, or terminal recovery requests.

## 2.1.210

- Bound partial Unix daemon requests after the readiness poll, so a client that sends incomplete JSON cannot wedge the synchronous accept loop during hot update or app startup recovery.

## 2.1.209

- Refuse stale hot-update handoff target regressions, so an older launcher cannot overwrite a newer session-survival owner registry after the current update has already prepared a newer daemon target.

## 2.1.208

- Treat a stale remote daemon that still owns a Codex runtime as a hot-update owner during remote stdio attach, preserving the PTY first and routing through the current daemon when the handoff becomes available instead of spawning a duplicate failed resume path.

## 2.1.207

- Clear stale remote-disconnect resume notifications when the daemon PTY is visibly showing a real Codex prompt with real scrollback after a retry, so hot-update recovery does not leave an interactive-looking terminal input-gated.

## 2.1.206

- Pause hidden loading-dot animations so retained invisible UI cannot keep WebKit's animation clock hot while the terminal is idle.
- Persist remote generated copy hints asynchronously so daemon status and runtime truth are not blocked by SSH copy-hint writes during hot update.

## 2.1.205

- Add an app-control CSS/Web animation census to state/read-only CPU probes so live fan spikes can distinguish terminal write/render work from compositor animation churn.

## 2.1.204

- Guard daemon Unix client reads with an explicit poll timeout before parsing requests, preventing a silent or half-open local client from monopolizing the daemon accept loop during hot update.

## 2.1.203

- Give visible Codex inline `Working` status animations a separate low-latency write budget and expose the animation-hot state in app-control/read-only probes, while keeping background and full-frame TUI throttling intact.

## 2.1.202

- Retire duplicate non-registry daemon owners once the registry's real PTY owner is clean, so older stale daemons cannot keep claiming the same live runtime keys after hot update.

## 2.1.201

- Short-circuit unchanged terminal input-policy syncs so idle active xterm views stop rescheduling focus, resize, and visible-paint work; read-only CPU smokes now expose input-policy apply/no-op churn.
- Retire preserved-only startup bridge sidecars once the registry's real PTY owner already cleanly matches the preserved runtime set, reducing stale daemon fan load without sacrificing hot-update session survival.

## 2.1.200

- Protect the daemon accept loop from silent local clients by timing out accepted request reads, preventing a stuck observability/status connection from wedging hot-update session recovery.

## 2.1.199

- Make `yggterm-headless server app ... --help` non-mutating, so live observability commands cannot accidentally execute screenshot or probe actions when the operator asks for command help.

## 2.1.198

- Start daemon-retained scrollback replay from the terminal loop's own visual-ready signal instead of waiting for an app-control state probe, so hot-updated remote sessions recover prompt-follow scrollback and manual viewport scrolling on their own.

## 2.1.197

- Split expandable sidebar row hit targets: clicking the visible row name selects the group and opens its scoped Startpage, while the icon, disclosure/count control, and trailing row surface toggle expansion. The same contract now applies to cwd folders, machine groups, and `Live Sessions`.

## 2.1.196

- Project remote live sessions with a known cwd under that cwd whether or not they are marked Keep Alive, so selecting a live session from its folder does not make the folder row vanish. Keep Alive now controls daemon retention only, not cwd findability.

## 2.1.195

- Keep hot update pointed at the stale daemon that explicitly owns a newly launched live PTY, even when an older owner allow-list would otherwise make a preserved-only sidecar look cleaner; this preserves sessions such as `muhurta` across direct-install replacement.

## 2.1.194

- Preserve manual Codex session titles when a live remote runtime is promoted from a synthetic Yggterm id to the real transcript id, and keep fallback hash titles from overwriting existing meaningful copy.
- Make sidebar rows for live remote sessions prefer daemon live-session title truth over stale scanned remote row titles, so renames such as `muhurta` remain visible after restore/reopen.

## 2.1.193

- Bound app-control DOM snapshot fallback latency during live terminal load, and carry retained replay prompt-follow diagnostics through the terminal fallback snapshot so read-only CPU/state smokes stay both fast and truthful.

## 2.1.192

- Fall back to the preserved owner daemon's client-close cleanup when direct ghost-runtime removal is blocked by already-gone remote Codex shutdown errors, but only when all owner-protected registry keys are explicitly keep-alive.

## 2.1.191

- Prune stale PTYs from hot-update owner daemons when their runtime keys are no longer represented by the current live-session graph, and route explicit live-session closes to the preserved owner so non-keep-alive sessions cannot survive as ghost sidecar truth.

## 2.1.190

- Force retained xterm replay to follow the live prompt after hot update, exposing viewport-force/replay-follow diagnostics and failing CPU/latency smoke tests when retained scrollback exists but the cursor is still below the visible viewport.

## 2.1.189

- Trim idle xterm canvas compositing on software WebKit by hiding non-text full-viewport canvas layers when selection/link/cursor overlays are inactive, replacing the cursor layer with a tiny Yggterm-owned overlay, and exposing the visible canvas-layer contract through app-control/read-only CPU smoke evidence.

## 2.1.188

- Honor Yggterm's selected Linux desktop backend inside the vendored Dioxus Wayland DMA-BUF workaround, so KDE Wayland launches can disable WebKit DMA-BUF without forcing the WebKit child back to X11 and recreating canvas idle CPU/fan burn.

## 2.1.187

- Force the native Wayland backend on KDE Wayland+Xwayland launches before choosing the xterm canvas renderer, preventing X11 WebKit canvas idle CPU/fan burn while preserving the low-CPU native Wayland path.

## 2.1.186

- Keep the newest clean preserved-owner sidecar alive during startup cleanup and reject legacy owners whose inferred runtime set contains unauthorized closed-session keys, so hot update can retarget without reserializing ghost sessions.

## 2.1.185

- Scope Linux daemon cleanup client checks to the candidate daemon's own client-instance directory, so a replacement GUI no longer protects every stale versioned sidecar from cleanup.

## 2.1.184

- Ignore unauthorized ghost-owned runtimes when selecting a startup hot-update handoff target, so stale daemons that only own closed sessions cannot outrank the clean preserved sidecar.

## 2.1.183

- Retire stale preserved-only Linux daemon sidecars during cleanup while keeping the actual hot-update PTY owner protected, reducing jojo fan load without sacrificing session survival.

## 2.1.182

- Treat preserved-only stale daemons as retarget/reconcile candidates during startup instead of skipping them and selecting older orphaned PTY owners, preventing old ghost daemons from repopulating closed sessions during direct-install replacement.

## 2.1.181

- Limit update-restart protection for unkept live rows to sessions that still have current daemon runtime truth, preventing stale in-memory live rows from being serialized back into the next daemon during direct-install replacement.
- Filter carried preserved-owner sidecar entries through the current runtime status before handoff or retarget, so hidden stale owner records cannot be reintroduced by the next patch-line update.

## 2.1.180

- Prune unrepresented hot-update preserved-owner entries from disk on daemon load and after live-session keep/close changes, so old non-keep-alive sessions cannot remain latent in `hot-update-terminal-owners.json`.
- Expose `live_keep_alive` in app-control sidebar row snapshots and keep the regression harness focused on the allowed duplicate shape: an explicitly kept remote live row in `Live Sessions` and its own cwd folder.
- Add a two-kept-session sidebar regression covering `dev:/home/pi/git/samplenotes` and `dev:/home/pi/git/samplescripts` so kept remote terminals cannot be projected under the wrong cwd folder.

## 2.1.179

- Keep hot-update preserved-owner runtime keys subordinate to current live-session metadata so sessions closed or no longer kept alive cannot resurrect after a restart.
- Allow Keep Alive toggles for live terminals whose PTY is still owned by a preserved hot-update daemon, preserving the session instead of rejecting the action because the new daemon does not own the PTY locally.
- Show explicitly kept remote live sessions under their remote cwd folder as well as under `Live Sessions`, so a kept `dev:/home/pi/git/samplenotes` terminal remains findable from the `samplenotes` folder after restart.

## 2.1.178

- Default KDE sessions with native Wayland available to the Wayland backend, keeping `YGGTERM_FORCE_X11_BACKEND=1` as the explicit fallback for X11-only investigations.
- Gate xterm canvas off on X11 by default while keeping canvas active on Wayland and available through `YGGTERM_ENABLE_XTERM_CANVAS=1`, reducing idle WebKit/GUI CPU in visible terminal sessions.
- Stamp the resolved xterm canvas policy into the GUI process environment and expose `xterm_canvas_renderer_requested` through app-control/idle smokes, so Wayland fan-budget tests fail on requested-vs-mounted renderer mismatches instead of reporting ambiguous CPU truth.

## 2.1.177

- Make direct-install hot update choose the stale daemon behind the active GUI client instead of an older orphaned same-home daemon when multiple versioned sockets are alive.
- Scan all same-home client-instance scopes during replacement GUI startup so older versioned windows are retired only after their active terminal state has been captured for handoff.
- Keep daemon startup prewarm from changing the active terminal or live-session row order while it prepares background live terminals.

## 2.1.176

- Resolve suffixed packaged headless companions such as `yggterm-headless-linux-x86_64` so remote bootstrap and timeline cleanup use the exact matched release helper instead of falling back to an older installed remote binary.

## 2.1.175

- Slow the unfocused disposable TUI drain cadence to keep backgrounded terminal workloads under the jojo fan budget while still bounding PTY backlog growth.

## 2.1.174

- Preserve the outgoing GUI's active terminal during direct-install handoff by deferring superseded-client retirement to the shell path that captures app-control active state before terminating the old process.
- Prevent daemon retained scrollback replay from using stale ready history on a new unready terminal mount, avoiding transient visible retained snapshots before the current viewport is actually interactive.

## 2.1.173

- Suspend periodic browser-tree scans while a terminal is the active viewport, eliminating minute-bound saved-tree refresh wakeups from focused/background terminal CPU samples.

## 2.1.172

- Drain backgrounded disposable TUI output every 2s while xterm rendering is suppressed, preventing large PTY backlogs from intermittently spiking GUI/WebKit CPU and reducing prompt-restore lag after stopping the TUI.
- Add remote CPU sample timestamps to the Linux idle/fan smoke so CPU windows can be aligned with remote event and perf traces without guessing across machine clock skew.

## 2.1.171

- Slow the browser-tree refresh cadence while the shell is on the quiet Startpage with no live sessions, reducing empty-window launch-idle CPU and root-render churn before terminal work starts.

## 2.1.170

- Preserve the outgoing GUI's active live terminal during direct-install hot-update handoff by capturing the retiring client's app-control state before termination and seeding the replacement client and daemon with that live path.
- Prevent normal persistence from writing an active session path that points to a live runtime excluded from durable `live_sessions`, while keeping update-restart persistence protective for all live runtimes.

## 2.1.169

- Route Linux idle CPU smoke probes through the matched `yggterm-headless` sibling so fan-budget tests cannot launch extra GUI clients while measuring the target window.
- Retarget stale busy live-session snapshot timers after terminal output quiets, preventing a full background daemon snapshot from firing during the quiet CPU budget window.
- Allow idle CPU smoke runs to pass through render tracing so root-render evidence can be aligned with CPU phases when investigating fan spikes.
- Split Codex launch timeline CPU into pre-test baseline, live-profile, isolated test-profile, SSH, Codex, and WebKit categories so jojo fan spikes can be traced before and after each test phase.
- Make the Codex launch timeline smoke classify focus-gated rendered prompts separately from terminal input failures, report focus-command/state disagreement, and verify generated remote Codex worker processes are gone during cleanup.
- Prefer real X11 window activation for Linux app-control focus requests when `xdotool` is available, while still reporting the fallback backend and native window id in focus proof.
- Keep remote timeline smoke artifacts and temp profiles under `/home/pi/.cache` by default, with storage preflight output in the report, to avoid `/tmp` pressure corrupting staged proof runs.
- Treat xterm focus-in/focus-out control bytes as terminal protocol traffic rather than user input, avoiding false "input accepted without echo" failures on healthy remote Codex prompts.
- Have the Codex launch timeline reclaim terminal focus through app-control before captures so screenshot evidence, active-host focus, and input-readiness probes can be compared directly.
- Expose `effective_input_focus` and the app-control terminal input override in state/smoke reports so X11/toolkit focus drift is distinguished from a genuinely unfocused xterm helper.
- Record a post-screenshot app-control state in Codex launch timeline captures so screenshot evidence is reconciled against the same settled viewport instead of the pre-screenshot probe response.

## 2.1.168

- Start Codex launch timeline resource logging before app launch or terminal creation, with phase-trace boundaries for baseline, launch, capture, and cleanup.
- Summarize Codex launch resource usage by phase in the smoke report so baseline, app launch, capture, and cleanup CPU spikes can be compared without hand-joining JSONL traces.
- Add pre-test host resource baselines and explicit phase events to the Linux idle CPU smoke so fan/CPU regressions have cause/effect evidence before the app mutates the target.
- Split daemon status into owned terminal runtimes and preserved terminal owners, and make hot restart preserve only daemons that actually own PTYs while retargeting the preserved-owner registry across safe sidecar restarts.
- Make app-control focus truth follow the native window focus result instead of marking the shell focused when the compositor refused focus.
- Route Codex launch timeline app-control probes through the matched `yggterm-headless` sibling so state, rows, and screenshot captures do not launch extra GUI clients or perturb focus.
- Forward xterm protocol replies through the raw terminal bridge even while user input is readiness-gated, preventing Codex startup handshakes from stalling behind an input-disabled viewport.
- Accept retained remote Codex scrollback only when real PTY bytes, meaningful output, and prompt-layout geometry agree, while preserving sparse prompt-only failure coverage.
- Tighten the Codex launch timeline smoke so a rendered prompt is not accepted until terminal input is ready, focus the owned proof window before captures, and separate sampler overhead from measured Yggterm CPU.
- Treat fresh `start-codex` remote attach markers as transport control just like `resume-codex`, preventing attach handshakes from leaking into terminal-output truth.

## 2.1.167

- Force WebKit DOM-rendered xterm rows to paint with `currentColor` text fill so hot-updated retained terminals cannot keep buffer text while the real viewport appears blank.
- Add app-control row text-fill diagnostics and mark DOM terminals unhealthy when retained buffer text is present but row text fill is transparent.
- Require hot-session switch smoke tests to prove prompt-region screenshot pixels after switching away and back, catching state-ready but visually blank retained surfaces.

## 2.1.166

- Add an isolated Codex launch timeline smoke that captures state, rows, screenshots, CPU, and cleanup truth at 0.5s through 60s for fresh local and remote Codex terminals.
- Keep terminal-emulator protocol handshakes, Codex welcome frames, prompt frames, and prompt-like setup frames out of the unfocused TUI drop path, preventing blank mounted xterm surfaces while Codex burns CPU.
- Retry transient app-control DOM eval-finished failures in basic/action snapshots and make the timeline smoke report DOM snapshot failures explicitly instead of misclassifying them as terminal-host loss.
- Gate app-control `input_enabled` behind actual surface readiness while exposing `raw_input_enabled` separately so protocol responses can keep flowing during startup without claiming user-input readiness.
- Restrict retained remote-surface recovery to actually retained or previously ready sessions, so fresh remote Codex launches are not remounted as stale retained failures.
- Back off active high-volume terminal frame writes to a 500ms default, keep chunked alt-screen output on the frame-budgeted read cadence, route bulk/frame-like xterm writes through the async parser path, and throttle frame-like render probes, perf events, buffer reads, and canvas health sampling so active TUI output cannot keep WebKit and the GUI pegged.
- Render active plain local full-screen TUI frames through a low-power text surface after the xterm control prefix is applied, while keeping Codex and remote sessions on the normal xterm path so saved-session and input semantics stay exact.
- Suppress sidebar/sample churn from hot frame-like host-health events, slow the idle app-control watchdog fallback, disarm terminal input when the desktop window is unfocused, and lengthen the local idle read backoff so refocus idle CPU settles instead of keeping jojo's fan hot.
- Keep background frame-budgeted local TUI streams on the unfocused 16s read cadence instead of accidentally clamping them back to the 3s local idle cap, reducing GUI/WebKit wakeups while the window is not focused.
- Add a post-state cooldown plus active/effective frame-budget fields to the remote Linux idle CPU smoke so app-control state probes do not contaminate the CPU sample they are meant to measure and the proof can show whether active TUI output is actually using the intended budget.
- Extend the remote Linux idle CPU smoke with per-thread CPU rows, render-counter deltas, low-power TUI overlay state, hot host-health suppression counters, background TUI frame-signal gating, and a post-interrupt drain before background-idle sampling.

## 2.1.165

- Treat sparse remote Codex prompt-only xterm surfaces as unhealthy when the `OpenAI Codex` welcome frame is missing and most rows below the prompt are blank, even if the PTY delivered real bytes.
- Extend read-only UI latency smoke output with root-render churn, browser rebuild churn, and top per-thread CPU deltas so GUI/WebKit spin can be diagnosed when terminal write/render counters stay idle.
- Make app-control wakeups worker-aware so requests targeted at a different live GUI PID do not keep unrelated clients scheduling root renders while the terminal itself is idle.
- Treat `server app <command> --help` as help instead of executing the app-control command, preventing accidental live GUI launches during diagnostics.

## 2.1.164

- Fail closed when an update-restored remote Codex session only has a synthetic `start-codex` launch marker: update restore now strips the fresh-start action and queues `resume-codex --require-existing` instead of silently starting a new Codex session.
- Add a regression test for the fatal update-restore class where a synthetic remote runtime key could reconnect as a generic fresh Codex surface rather than the saved transcript identity.

## 2.1.163

- Start daemon hot-update as a fault-tolerant handoff when live PTYs exist: the new daemon comes up on the updated socket while the old daemon remains the preserved terminal owner, and terminal read/write/resize calls route through that owner instead of killing the session.
- Expose active hot-update handoff through daemon status, app-control `daemon_update_state`, and `yggterm-headless server monitor --scenario hot-restart` with preserved owner/runtime keys, so release proof can distinguish real hot update from a fatal session restart.
- Defer update completion, not session ownership, when a live runtime cannot be handed off safely; app-control reports `hot_update_pending` instead of treating session-preserving deferral as a clean version match.
- Make `yggterm-headless server monitor --scenario hot-restart` refuse the destructive prepare/shutdown fallback when live runtimes are present; session survival takes priority over completing an update.
- Extend read-only UI latency smoke coverage with idle render/write churn and combined GUI/WebKit CPU sampling, so live sessions can fail deterministic proof when they are visually readable but burning CPU or rendering too slowly.
- Keep unfocused remote frame streams on the slow unfocused read cadence instead of clamping them back to the 3s local idle poll, reducing background xterm/WebKit churn while live sessions remain preserved.

## 2.1.162

- Preserve same-major/minor stale daemons that still own live terminal runtimes during update handoff, avoiding hot-restart drops of keep-alive PTYs while the new GUI reconnects.
- Gate daemon retained snapshot replay until the active terminal is already ready, and keep retained replay from enabling input or seeding fresh remote Codex starts before the real prompt-ready surface exists.
- Report `daemon ready` for xterm.js daemon-backed sessions independently of the legacy Ghostty bridge flag, so live terminal status reflects the active runtime contract.
- Make read-only terminal drawing smoke fail when retained daemon snapshots become visible before readiness settles.

## 2.1.161

- Keep the selected visible terminal on the fast active write-frame budget even when the desktop window is not focused, so watching an active Codex/TUI viewport cannot fall back to the 4s background cadence.
- Restore the VS Code-style focused titlebar search palette so real clicks elevate it into a usable command/search surface instead of leaving it clipped to the idle titlebar field.
- Keep focused search centered and wide through compact resize cycles while preserving the non-overlap contract for the idle titlebar search field.

## 2.1.160

- Give visible focused terminal TUI/Codex frames a separate fast write-frame budget while keeping the slow protective budget for hidden/background output.
- Expose active/effective terminal write-frame budgets through app-control timing so low-FPS terminal incidents are observable from state, telemetry, and smoke proofs.
- Make read-only terminal drawing smoke fail when an active frame-like terminal is still using the background write budget.

## 2.1.159

- Make manual terminal redraw restore prompt-follow scrollback before and after repaint, so a retained Codex/xterm buffer cannot stay blank or scrolled away from the prompt after a redraw.
- Treat `PromptFollow` terminals that remain scrollback-locked as app-control geometry failures while still allowing explicit user scrollback to inspect history.
- Tighten redraw smoke coverage to fail when redraw leaves the prompt cursor below the visible viewport.

## 2.1.158

- Keep the titlebar search centered after focus and resize cycles, with focused search resize coverage in the small-window chrome smoke.
- Make the read-only terminal drawing smoke wait for retained replay/render readiness while preserving the initial transient state in the proof report, so post-relaunch drawing latency is measured instead of misreported as a settled bad frame.

## 2.1.157

- Mirror app-control terminal host truth at both the viewport and compatibility state levels, and derive active input/runtime counters from the focused active xterm host so drawing probes no longer miss a visible terminal.
- Add prompt-band and redraw timing diagnostics for live xterm hosts, including manual redraw settle timing, render deltas, content-source truth, fit overflow, cursor geometry, and low-power overlay state.
- Add a read-only drawing mode to the UI latency smoke so live Codex sessions can be checked with state, rows, screenshot timing, and drawing diagnostics without typing into the active prompt.

## 2.1.156

- Stop replaying managed session/status metadata into xterm surfaces during remote live restore; terminal bytes now come from daemon PTY reads or retained PTY snapshots, not sidebar/server prompt summaries.
- Expose terminal content-source and retained-replay source through app-control, make manual redraw reject non-PTY server prompt snapshots, and teach terminal observability to flag that mismatch explicitly.
- Extend redraw and shell regression coverage so stale server prompt prose cannot be accepted as a repaired terminal frame.
- Tighten the Linux installed-profile smoke cleanup so it ignores daemon/SSH terminal transport children that inherit smoke env, avoiding accidental live-session disruption during harness proof.

## 2.1.155

- Make focusing a sidebar folder show the scoped start page instead of preserving or reactivating the previously focused terminal, so folder navigation has deterministic Startpage context and no hidden terminal input target.
- Split sidebar folder focus from expansion: clicking a folder opens the scoped Startpage, while the disclosure control and arrow keys expand or collapse the tree.
- Add app-control smoke coverage for the folder-focus Startpage contract, including active-session clearing, selected-folder truth, terminal input gating, and a successful `server app open <folder>` settle path.

## 2.1.154

- Preserve the real Codex saved-session id for app-created `local://...` live Codex runtimes during update restart restore, so the daemon keeps the terminal runtime key but launches `codex resume <saved-session>` instead of a fresh bare Codex process.

## 2.1.153

- Persist the real Codex saved-session id and transcript path for daemon-owned `codex-runtime://...` live sessions before update restarts, then restore them with `codex resume <saved-session>` while preserving the runtime key.
- Keep server restore and terminal-launch persistence on `codex-runtime://...` plus Terminal mode, avoiding restart states that revive a kept live session as Web View or as a fresh Codex process under an old row.

## 2.1.152

- Keep daemon-owned `codex-runtime://...` sessions visible under Live Sessions without rewriting them to `local://...`, so the sidebar, active terminal, app-control contract, and daemon runtime key stay on one identity.
- Make `server app launch --wait-settled` wait for initial daemon sync, clean session/view contract truth, and live runtime rows before returning harness success; Linux app-owned launches now detach the GUI process from the CLI session so the proof window remains inspectable after launch.

## 2.1.151

- Preserve daemon-owned Codex runtime keys as `codex-runtime://...` during update restore, and adopt any legacy `local://...` PTY under the canonical runtime key instead of spawning a second Codex process for the same session.

## 2.1.150

- Resolve daemon-owned Codex live sessions from the current Codex SQLite log/state files when no transcript JSONL fd is open yet, keeping saved-session identity deterministic for newer Codex CLIs without turning snapshots into full-table log scans.
- Cache resolved Codex process identities in the daemon and keep the app-owned terminal write path compatible with Codex carriage-return submission, so `/status`-class smoke proof uses the real PTY path.

## 2.1.149

- Serialize daemon socket ownership with a lifetime bind lock and existing-owner check, so update/hot-restart recovery cannot spawn multiple same-version daemons that all replay the same kept Codex sessions.
- Resolve daemon-owned Codex live sessions through the PTY process tree's real transcript JSONL, so freshly started sessions appear in remote scans/search by their saved Codex id and `resume-codex` reuses the existing runtime instead of opening a duplicate session.

## 2.1.148

- Flush input-hot prompt echoes through the xterm write bridge immediately even when a frame-like TUI payload is pending, so SSH/Codex terminals do not hold typed characters behind the high-volume frame timer.
- Resolve daemon-owned Codex live sessions through the PTY process tree's real transcript JSONL, so freshly started sessions appear in remote scans/search by their saved Codex id and `resume-codex` reuses the existing runtime instead of opening a duplicate session.

## 2.1.147

- Keep daemon-owned terminal runtimes from idling out when no GUI/client record is present, so preserved remote Codex sessions survive the bridge gap during GUI and daemon restarts.
- Report daemon-owned remote Codex runtimes as live in remote scans even when an older runtime exposed its terminal key as `local://...`, keeping Live Sessions and remote session refresh truth aligned.
- Tighten the live keep-alive recovery harness so an active kept Terminal must have a clean active terminal surface, not just a matching daemon runtime key.
- Reduce high-volume plain-terminal TUI repaint churn by lengthening the frame budget, preserving frame-mode coalescing across chunked alt-screen/cursor-hidden PTY rows, slowing unfocused output reads and active-terminal sidebar tree refreshes, suppressing stateful background TUI frames before they cross into WebView while the app does not own focus, avoiding full health probes for dropped background frames, keeping the visible low-power overlay off active terminals, and avoiding redundant full-canvas refreshes after frame-like writes.

## 2.1.146

- Restore remote Codex keep-alive sessions as live runtime targets even when their persisted storage path points at the remote `~/.codex/sessions` tree, so preserved GUI restarts keep the intended remote Terminal active instead of falling back to the local update-restart shell.

## 2.1.145

- Keep visible active Codex/TUI sessions on the xterm render path even while the Yggterm window is unfocused, preventing the low-power overlay from corrupting kept-alive terminal output.

## 2.1.144

- Accept live daemon runtime truth during implicit startup restore even when the shell first rendered the start page, so keep-alive sessions reactivate as Terminal instead of staying collapsed while `Live Sessions` shows the row.
- Keep foreground Codex/TUI terminals on the real xterm render path when the terminal host owns input focus, even if the WebView document focus signal is stale.
- Clear the low-power TUI overlay during manual redraws and make the live keep-alive harness reject foreground terminals that still expose that overlay.

## 2.1.143

- Persist restart-safe daemon state before closing a GUI with live-session preservation, so direct-install handoffs keep the intended keep-alive Terminal target instead of briefly restoring stale Web View state.
- Sanitize implicit startup snapshots so stored sessions are not auto-opened as Web View; startup now falls back to a matching live Terminal row or the start page unless the user explicitly opens a stored preview.

## 2.1.142

- Keep explicit live-session keep-alive rows in Terminal recovery mode across GUI/update restarts until their daemon runtime is recreated, avoiding nondeterministic Web View launches.
- Route Terminal view launches to the shell's selected session path instead of the daemon's stale active path, so recovery cannot unexpectedly switch into a different live shell.

## 2.1.141

- Throttle unfocused high-volume terminal output and route visible-but-unfocused TUI frames through the observable low-power surface instead of repainting xterm canvas at full rate.
- Extend the remote Linux idle CPU smoke with a background active-TUI phase and app-control counters for unfocused TUI frame drops.

## 2.1.140

- Keep newly requested start-page terminal sessions in daemon snapshots while their runtime is still mounting, so Agent and Terminal buttons no longer create a session and immediately hide it as stale.
- Reuse the selected remote folder or live-session cwd when starting terminal sessions from the start page, and let the harness pin start-page proof to a configured remote cwd.

## 2.1.139

- Scope start-page Recent Work to the selected remote folder and let remote scan metadata win over duplicate sidebar rows, so cwd-specific sessions are sorted by last-used time instead of stale sidebar order.
- Make the app-control start-page route immediately queue remote refresh work, giving the live harness a deterministic path to prove Recent Work against transcript mtimes.

## 2.1.138

- Drain remote Codex scan stdout/stderr while the SSH helper is still running, so large remote transcript histories refresh instead of timing out and falling back to stale cached sessions.
- Add an app-control start-page command and harness route so Recent Work truth checks and start-page action smokes can run against the live profile without desktop-wide input-device automation.

## 2.1.137

- Refresh healthy remote machines while the start page is visible instead of treating their cached Codex session list as permanently fresh, so newer cwd sessions appear without waiting for manual interaction.
- Sort start-page Recent Work from remote session `modified_epoch`/`started_at` metadata rather than sidebar traversal order, and add a harness truth check that can compare the daemon and UI against remote Codex transcript mtimes for a configured machine/cwd.

## 2.1.136

- Apply the compact focused-search width limit to the 520px breakpoint as well, preventing the active search shell from colliding with the right titlebar controls.

## 2.1.135

- Tighten the 620px titlebar search width so the focused search shell stays centered without overlapping the compact right controls.

## 2.1.134

- Keep the compact right settings rail below the revealed auto-hide titlebar, and make the compact chrome smoke assert terminal-theme controls only when the active surface is a terminal.

## 2.1.133

- Mirror right-panel scroll app-control in the headless companion so installed launchers can run compact titlebar harness checks through the same single-source control path.

## 2.1.132

- Preserve the full selected sidebar set when context-menu Delete is invoked from an already-selected folder or session, and expose pending delete paths/counts through app-control so the harness can prove the modal before canceling.
- Show local stored Codex transcript rows under their metadata cwd, including the home folder, without requiring the hidden `.codex` storage root to be expanded.
- Keep titlebar search centered against the full titlebar and shrink it responsively before it overlaps Connect SSH or the right titlebar controls.
- Add app-control `tree select` plus live harness checks for multi-select Delete and local stored-session visibility.
- Let the remote X11 harness keep an installed-profile smoke window open after a passing run, so the final proof can also leave the updated desktop app running.

## 2.1.131

- Search sidebar rows with the same local/remote session context used for previews and generated copy, so version strings and recent transcript text can find collapsed Codex sessions under their cwd folders.
- Add initial context-menu Delete and titlebar-search harness coverage; 2.1.132 completes the multi-selection and compact titlebar fixes.

## 2.1.130

- Mirror app-control `pointer`, `key`, and `start-action` commands in the headless companion so direct launchers can run live harness checks without desktop input-device automation.

## 2.1.129

- Add a Yggterm-owned `server app start-action` harness command for start-page Agent, Terminal, and SSH actions so live desktop proof does not require KDE input-device control.

## 2.1.128

- Render sidebar Session and Terminal marks as boxed SVG icons with `>_` and `$_` text, avoiding literal bracket characters inside the icon payload.

## 2.1.127

- Fix sidebar and start-page creation actions so Codex sessions and terminals launch through the selected local/remote context, keep live-session close in Terminal mode when another runtime remains, refresh stored Codex transcript rows deterministically, and use boxed greyscale `>_` / `$_` SVG sidebar marks.
- Add installed-profile X11 harness coverage for start-page action buttons so live desktop proof exercises the updated target profile instead of only an isolated temporary home.

## 2.1.126

- Fix sidebar and start-page creation actions so Codex sessions and terminals launch through the selected local/remote context, keep live-session close in Terminal mode when another runtime remains, refresh stored Codex transcript rows deterministically, and use boxed greyscale `>_` / `$_` SVG sidebar marks.

## 2.1.125

- Keep collapsed `.codex` storage roots authoritative in the merged sidebar, so stored transcript leaves no longer stay visible as project-cwd siblings after collapse or restart.
- Preserve the live-runtime identity contract while filtering stored transcript rows: live Codex terminals remain `local://<id>` rows with `Storage` metadata, without duplicating the stored transcript row.

## 2.1.122

- Fix a Dioxus nested-borrow crash when restoring or switching a live session between Web View and Terminal, so the GUI no longer aborts seconds after launch.
- Keep live runtime rows authoritative in Terminal mode even if a stale Web View request is replayed during startup or app-control open.
- Rename user-facing Preview mode copy to Web View while keeping legacy `preview` app-control and CLI aliases compatible.

## 2.1.121

- Make daemon terminal runtime identity deterministic: local shell and local Codex live sessions now use `local://<id>` runtime keys, while stored `.codex/sessions/*.jsonl` paths stay transcript metadata instead of becoming live PTY keys.
- Migrate legacy/raw Codex live runtime keys on restore and keep `Live Sessions` rows tied to daemon runtime truth, including keep-alive rows that must have a matching terminal runtime.
- Add manual terminal redraw through app-control and the live-terminal context menu, with xterm refit/row-guard/texture-refresh recovery for blank or stale canvases.
- Report render-health problems when app-control sees non-empty terminal buffer text but a blank or low-ink canvas, so stale-pixel failures are visible in state instead of only screenshots.
- Keep large stored `.codex` transcript trees collapsed unless the user explicitly expands them, even when a stored transcript is selected after restart.
- Extend focused Rust and smoke coverage for live-session identity, stored transcript promotion, keep-alive restore, tree-collapse persistence, redraw routing, and render-health classification.

## 2.1.120

- Make `Live Sessions` rows runtime-truth-only: daemon snapshots now filter live metadata against real terminal runtime keys, and restored remote metadata without a runtime downgrades to preview instead of a blank terminal shell.
- Make the shell stop reviving missing live rows from cached retained hosts, so explicit close and lost runtimes remove the live row instead of preserving stale UI state.
- Keep retained xterm hosts as visual caches only: inactive retained sessions now release their Rust bridge, and focus return bootstraps a fresh bridge so typed input cannot disappear into a stale DOM handler.
- Harden the terminal JS bridge against long-lived handler drift by capturing the Dioxus channel at mount time and ignoring stray app-control payloads instead of poisoning the terminal event loop.
- Flush locally toggled keep-alive state before client close so a kept session survives the close/reopen path instead of being reaped by stale daemon state.
- Reduce typing-time snapshot churn by replacing the immediate live-session snapshot nudge with an input-hot window, plus app-control counters for live-row/runtime counts, input-hot state, and forced refresh activity.
- Extend regression coverage for stale remote metadata overcounting, runtime-missing restore, cached live-row rejection, retained-host cleanup, and input-hot background snapshot scheduling.
- Extend terminal smoke coverage for close confirmation, keep-alive dot placement, stale retained bridge regressions, app-control payload isolation, and jojo debug-build input latency after live-session switching.

## 2.1.119

- Restore remote live-session scrollback from daemon-retained terminal history instead of relying on a small screen/status tail after a session is already marked ready.
- Make retained replay hydrate collapsed xterm buffers even when the prompt is visible, so “usable but unscrollable” sessions fail the smoke harness instead of passing as ready.
- Make GUI restarts preserve live sessions, and make superseded-client handoffs hard-kill only the old window process instead of letting older GUI signal handlers run daemon shutdown cleanup.
- Extend app-control and CI coverage for retained scrollback replay evidence.

## 2.1.118

- Stabilize terminal redraw after resize/session switching by forcing settled xterm geometry to converge with the PTY resize notification path.
- Reduce typing-time client churn by throttling input-hot terminal perf/health sampling and extending the latency smoke to measure render/write churn and process CPU.
- Move Live Sessions keep-alive markers into a fixed leading status rail so kept sessions do not show jagged title-width-dependent dots.

## 2.1.117

- Keep daemon control-plane requests responsive while remote session previews refresh: preview refresh now uses cached scan/session data instead of running SSH scan or preview fetches under the daemon lock.
- Add a regression test for remote live-session preview refresh against an invalid SSH target so CI fails if the preview path tries remote I/O again.
- Time-bound the Python remote-scan fallback helper so failed remote Yggterm scan fallback cannot run indefinitely.

## 2.1.116

- Keep user scrollback under user control: wheel/page scrolling now records explicit scrollback intent, passive terminal output no longer snaps the viewport back to the prompt, and the smoke harness fails if release causes a bottom snap after wheel release.
- Reduce live terminal typing latency by keeping small echo writes out of the full-canvas retained-session paint repair path and skipping expensive full refresh work while input is hot.
- Improve live-session lifecycle truth with terminal-specific close notifications and startup daemon hot-swap scaffolding for stale same-profile daemons.

## 2.1.115

- Force the active xterm viewport back to the live cursor when terminal input or focus occurs, so switching/typing into a live session cannot leave the prompt hidden in scrollback.
- Extend the latency smoke readiness gate to fail when the active terminal is scrollback-locked away from the prompt or reports the cursor outside the visible viewport.

## 2.1.114

- Classify the first post-open terminal latency token as warmup with its own budget, while keeping strict steady-state visible-echo max and p95 budgets.
- Add CI syntax coverage for the latency smoke script so harness regressions are caught before release.

## 2.1.113

- Remove artificial per-character settle sleeps from app-control keyboard latency probes so the live latency gate measures terminal echo instead of harness delay.
- Add CI coverage for the per-character probe path.

## 2.1.112

- Route remote live-session input through the already-running local SSH bridge/runtime before falling back to remote-direct writes, removing per-character remote command startup from the hot typing path.
- Add CI coverage for the remote live input write strategy so latency regressions cannot silently reintroduce remote-direct writes while a local runtime is hot.

## 2.1.111

- Treat focused Codex conversation-interrupted input surfaces as live interactive terminals even when Codex does not render the normal prompt glyph.
- Keep foreground terminal latency work gated while a live terminal is active, and keep the canvas-rendered dim prompt text readable.
- Add CI coverage for interrupted Codex input readiness, foreground refresh deferral, no-echo detection, and canvas dim prompt readability.

## 2.1.110

- Fixed canvas-rendered Codex prompt readability by keeping xterm.js dim text close to Ghostty-style terminal contrast instead of halving prompt opacity.
- Deferred remote-machine and managed-Codex background refresh work while a focused terminal is active, reducing latency spikes from SSH binary/version probes competing with interactive terminal input.
- Tightened app-control observability so a remote terminal that accepts input but does not receive a following daemon stream echo is reported as unhealthy instead of ready.
- Added focused CI regression gates for canvas dim prompt contrast, foreground terminal refresh deferral, and accepted-input-without-stream-echo detection.

## 2.1.109

- Enforced the terminal single-source contract: Terminal mode is backed by the daemon-owned PTY/runtime stream and no longer accepts generated Codex card/status-copy surfaces as prompt-ready terminal truth.
- Kept retained live remote terminal hosts attached while inactive, so switching away and back does not leave a visually cached xterm with a stopped read loop that accepts input but never receives stream output.
- Replaced brittle Codex `/status` card regression gates with deterministic daemon-stream marker proof and retained-session hot-switch coverage.
- Updated the YggUI app-control and changelog proof workflows so generic terminal input regressions are verified with deterministic marker echo/clear probes instead of Codex-specific output.

## 2.1.108

- Restored the intended live-session lifecycle split: normal final app close prunes and gracefully closes non-Keep-Alive live sessions with one-hour force cleanup, while update restart still temporarily protects every recoverable live session.
- Tightened terminal readiness so an interrupted Codex banner without a real prompt/status/setup surface cannot be accepted as an interactive retained terminal.
- Restored the native block cursor with Ghostty-style cursor fill/text theme handling, including cursor-text parsing from bundled Ghostty themes and prompt screenshot smoke coverage.
- Added focused CI gates for normal-close persistence, update-restart restore, interrupted-banner prompt readiness, and cursor theme contract regressions.

## 2.1.107

- Fixed live-session persistence so recoverable live terminals survive normal app restarts and are eligible for startup background prewarm even when the user has not explicitly toggled keep-alive. This closes the gap where restored sidebar sessions existed but no terminal runtimes loaded until the user selected them.

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
