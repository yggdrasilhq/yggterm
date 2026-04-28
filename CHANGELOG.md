# Changelog

This file tracks user-visible changes in `yggterm`.

## Unreleased

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
