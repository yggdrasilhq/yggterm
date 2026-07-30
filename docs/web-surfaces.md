# Web Surfaces (libyggterm pilot — OSC 7717)

A program running in any yggterm terminal can turn that session's viewport
into a web view. This is the first libyggterm app surface; the pilot client is
[ychrome](https://github.com/yggdrasilhq/ychrome).

## UX

```
# in any yggterm terminal (local or ssh)
$ ychrome http://localhost:8000
```

The session's viewport swaps to a web view of the URL, resolved from the
machine the command runs on. Ctrl+C (or the overlay's ✕, which sends a real
Ctrl+C) hands the terminal back.

## Transport: the PTY byte relay

The control channel is an OSC escape sequence emitted on the app's stdout:

```
ESC ] 7717 ; web-surface ; <action> ; <base64 json> BEL
```

- `<action>`: `open` | `heartbeat` | `close`
- json payload: `{"session": "<YGGTERM_SESSION_ID>", "url": "...", "title": "..."}`

Because the transport is the terminal byte stream itself, it works identically
for local and remote sessions (remote daemon → ssh bridge → local daemon →
xterm.js) with no new RPC lane, and it is invisible in plain terminals —
unknown OSCs are ignored, which is the degradation story.

The GUI consumes the OSC in the xterm.js parser (never printed), forwards it
as a `web_surface` terminal event to the shell, and keys surface state by the
session the bytes arrived on. **The stream is the identity truth**; the
payload `session` field is diagnostic (a remote session's env id lives in the
remote daemon's namespace and is not comparable to the GUI session path).

## Session-identity handshake

The daemon exports into every PTY it owns (the `$TMUX` pattern):

- `YGGTERM_SESSION_ID` — the daemon's session key
- `YGGTERM_BIN` — the daemon's own executable path

Presence of `YGGTERM_SESSION_ID` is how a libyggterm app decides thin-client
vs standalone mode. Both survive ssh because the *remote* daemon owns the PTY.

## Lifecycle

- `open` registers/updates the surface; the overlay renders over the terminal
  viewport (the PTY is untouched underneath).
- `heartbeat` every ~4s is the liveness truth. Surfaces expire after 15s
  without one (`WEB_SURFACE_STALE_AFTER_MS`), so a SIGKILLed app never leaks a
  stuck overlay. Heartbeats carry the full payload, so a terminal remount
  (scrollback replay) self-heals the surface.
- `close` removes the surface immediately. Scrollback replay of an
  open→close pair is order-preserving, so replays converge to the right state.
- The overlay ✕ button removes the surface and writes `\x03` to the PTY —
  the terminal-native way to end the foreground app, which then emits its own
  `close`.

### Backgrounding: what stops, and what survives (2026-07-26)

The axis is **PAINT, not existence**. A page the user cannot see must stop
costing the compositor a frame; it must not stop being the thing they left
running.

Backgrounding a session's surface therefore does two separable things:

- **Stop painting.** Under glass the container stays attached and demoted, and
  the inner webview is hidden, so WebKitGTK marks the page
  `visibilityState: 'hidden'` — `requestAnimationFrame` pauses and timers
  throttle. Audio, network and JS heap are untouched. Under real memory pressure
  the container is DETACHED instead (unmapped), which throttles the same way and
  additionally lets the destroy below reclaim.
- **Eventually destroy.** `web_surface_reap_due` decides this, and it has three
  independent survival claims. The **hold** clock (default 600 s, config
  `background_hold_secs`, 5 s under real pressure), an **agent lease**, and
  **media**. Hold and lease are `max`, never `min` — a lease only ever extends
  life. Media is an absolute veto: a surface WebKit reports as playing audio is
  never destroyed, whatever either clock says.

#### The reclaim domain is per (session, TAB), not per session

Backgrounding used to be a question about SESSIONS, and that left the biggest
hole in the whole mechanism: every tab of the session the user is looking at was
exempt forever. A hundred-tab browsing day therefore meant a hundred live WebKit
web processes for the life of the GUI, however long ago the user had last looked
at ninety-nine of them. Lazy creation already made a never-visited tab free
(`web_surface_tab_create_rect`: no webview until someone is shown it, or an agent
asks for it); nothing ever gave a VISITED one back.

`web_surface_background_candidates` owns the domain now. A realized surface is a
reclaim candidate unless someone is being SHOWN it:

| Situation | Candidate? | Reason label | Hold |
|---|---|---|---|
| Session not active-visible | yes | `session_backgrounded` | `background_hold_secs` |
| Session shown, tab not on screen | yes | `tab_backgrounded` | `tab_background_hold_secs` |
| The tab on screen (active tab, or pinned into a live split pane) | **never** | — | — |

Everything else is the SAME machinery: one pressure predicate, one thrash guard,
one audio veto, one lease arithmetic. The only thing the reason selects is which
configured hold applies, and both trace events (`native_stash`, `native_close`)
now carry a `domain` field so a per-tab reclaim is not mistaken for a session
one.

Three properties that are easy to break and are locked:

- **The session domain is NOT narrowed by the tab exemptions.** A split-pinned
  tab of a *backgrounded* session is off screen like every other surface that
  session owns, and reclaiming it is shipped behaviour.
- **"On screen" agrees with placement.** `web_surface_tab_on_screen` is the
  state-authoritative twin of `web_surface_tab_place_rect` — it has to be its own
  function because reclaim runs BEFORE the tick's DOM geometry eval, and it has
  to give the same answer, or the set that gets painted and the set that gets
  destroyed disagree about who the user is looking at.
- **A session whose active tab is unknown yields no candidates.** The pass must
  never act on a surface it cannot classify.

The domain is only as good as its two inputs, and both have owners of their own
because both were, briefly, derivations inside the async reconcile loop that no
test could reach:

- **`web_surface_active_tab_by_session`** — which tab each session's page area
  shows, read from the same `desired` snapshot the placement loop iterates. This
  is what identifies the ONE tab per visible session that is exempt. Answer `0`
  for every session and the pass demote+throttles the tab the user is reading
  (with a zero hold, destroys it) while the genuine background tabs become exempt
  forever — the bug this section exists to fix, restored, with the page under the
  user's eyes hidden from its engine as a bonus. The argument name at the call
  site is unchanged by such a mutation, so the seam needs a behavioural lock, not
  a structural one.
- **`ShellState::split_pinned_web_tabs`** — the ONE answer to "this tab paints in
  its own pane", read by the reclaim domain and by the per-tab instrument.
  Emptying it makes a visibly painted pane a candidate: stash + throttle on the
  tick, unstash on the next, a churn on a pane the user is looking at, plus a
  wrong `split_pinned` column in the listing.

What a reclaimed tab loses is page state — scroll position, form contents, JS
heap. What it keeps is its identity: the URL and title live in the tab model and
on disk (`SavedWebTab`), so selecting it recreates a fresh webview on the same
page through the ordinary lazy-create path. That is a documented limitation, not
a bug to be papered over with a session-state FFI.

**Config**, both in `~/.yggterm/web-surface.json`, both in seconds, one file
reader (`web_surface_config_raw`) and one parser (`web_surface_config_hold_ms`,
pure — it takes the config body, so which KEY a hold reads and what it falls back
to are answerable by a test instead of by the test machine's `$HOME`):

```json
{ "background_hold_secs": 600, "tab_background_hold_secs": 600 }
```

`tab_background_hold_secs` is the knob a 100-tab day turns down; `0` destroys a
tab's webview as soon as it leaves the screen. They are two knobs, not one
wearing two names: each hold ignores the other's key, and that is locked in both
directions — a tab hold that quietly read `background_hold_secs` would make the
documented knob do nothing while every behavioural test still passed, because
those are handed holds as arguments.

#### The per-tab instrument

`server app state` → `web_surface_tabs` lists every DESIRED tab joined against
the realized webview registry: `state` (`visible` / `stashed` / `live` /
`no_webview`), `stashed_for_ms` (how long it has been off screen — the age the
hold is read against), `reaps_in_window`, plus `active_tab`, `split_pinned` and
`leased`. Desired-first on purpose: the registry alone can only list what exists,
which is the wrong end of the question, since most tabs SHOULD be missing from
it. `tabs_without_webview` is the number the lane is judged on. The render probe
carries the same pair per sample (`web_surface_tabs` against
`web_surface_views` / `web_surface_view_sessions` / `web_surface_contexts`).

⚠ **There is no per-tab RSS, and the payload says so in words.** WebKitGTK pools
web processes per `WebContext` and every tab of one profile shares that context,
so bytes are not attributable to a tab. What is attributable is (views,
contexts) plus the process-level RSS the render probe already samples; any finer
number would be invented.

Two rules that keep the media veto honest:

- It vetoes **destroy**, never **throttle**. An audible background tab still
  stops painting. That is the cheap half and there is no reason to give it up.
- `webkit_web_view_set_is_muted` is **never** used as a reclaim tool. Muting a
  playlist to save CPU is precisely the failure the veto exists to prevent.

⚠ **Memory pressure is `reclaim_pressured`, not `swap_pressured`.** Swap-*used*
is a history counter: it latches TRUE after one bad afternoon and never clears.
On the live host that meant every backgrounded surface was hard-detached and
destroyed five seconds after a switch, on a machine sitting on 45% free RAM —
**every** `native_stash` event in the retained trace (19 of 19) carried
`detached:true swap_pressured:true hold_ms:5000`, alongside 53
`background_hold_expired` closes, so the 600 s soft-stash hold has no recorded
execution at all. The reclaim predicate reads current headroom (`MemAvailable`
under 15% of `MemTotal`) or the kernel's PSI **`full avg60`** stall accounting at
or above 10% *while* headroom is already under 30% — `full` (every non-idle task
stalled), never `some` (at least one task stalled, which a large build produces
on a machine with gigabytes free), and headroom VETOES the PSI route because PSI
is system-wide and says nothing about whose memory is short. An ABSENT snapshot
reads FALSE: the action here is destroying the user's pages, and ignorance is not
a licence to do that. The
`native_stash` trace event now publishes `reclaim_pressured` and `media_active`
so the decision is readable after the fact.

#### Where the decision lives, and why it is one function

`web_surface_reclaim_background_pass` is the ONE owner of a reclaim pass. The
reconcile loop reads `/proc`, reads both configured holds, builds its candidate
domain with `web_surface_background_candidates` and calls it; the pass gathers
each surface's reap history and lease, asks `web_surface_background_plan`, and
applies the answer through a `WebSurfaceBackgroundHost` (destroy / stash /
demote / throttle / clear-loading / trace).

That shape is not an aesthetic preference. Round 24 shipped the headroom
predicate, the treadmill guard and the audio veto with tests that called the new
helpers directly, and an adversarial review reverted all four production call
sites with the suite still green — the wiring was the defect and no test could
see the wiring, because the loop is `async` and holds a live `DesktopContext`.
Anything that can hold a decision belongs in the pass; the live host impl is four
one-line methods and is locked structurally by
`the_reclaim_pass_call_site_is_wired_to_the_live_machine`. See field guide §7.1.

## The egress rule

**A surface's network egress is the invoking host's network — for ALL URLs.**
A remote session's surface gets ONE `ssh -N -D <port>` SOCKS tunnel to the
session's machine, shared by every tab of that session, and the tabs' webviews
proxy every request through it via `ProxyConfig::Socks5`. The *remote sshd*
resolves every hostname and originates every connection on that machine —
loopback URLs reach the REMOTE loopback. If the SOCKS tunnel cannot be
established, loopback URLs fall back to the older `ssh -N -L` per-URL forward,
and anything else falls back to direct load from the GUI host — a traced egress
gap (`egress_gap` in the `open`/`tab_navigate` trace events), not a silent one.
Local sessions load directly, no proxy.

**The tunnel is the SESSION's, and it dies with its LAST tab** (2026-07-26). It
used to be per-TAB: `web_surface_new_tab` mints `socks_port: None` and reuse was
keyed on the tab, so every new tab of a remote session spawned another
`ssh -N -D` — another child here, another handshake, another sshd on the remote,
another listening loopback port, and the remote's `MaxStartups` reached well
before any tab count a user would call "a lot". A tab with no tunnel now adopts
the session's (`adopt_web_surface_session_socks`, donor chosen by lowest tab id
so the answer does not follow a user-reorderable strip), and the `Arc` on the
child is the refcount: `kill_forward` tears the tunnel down only when its handle
is the last one (`web_surface_forward_is_last_holder`). A tab that already has
the tunnel keeps it untouched, which is the older and still-necessary property —
re-spawning would churn the port and force a webview destroy+recreate, dropping a
just-set login cookie before it flushed.

Sharing the tunnel is also what lets tabs of a remote session share one
`WebContext`: the SOCKS port is part of `web_context_key`, so a per-tab tunnel
forced a per-tab context.

## Browser chrome: tabs + address bar

The overlay carries a minimal Chrome-like UI (v2.9.54):

- **Tab strip.** `tabs[0]` is the *app tab*, owned by the OSC stream — when the
  app emits a new URL, the app tab retargets and user tabs survive. The app tab
  has no per-tab close button; the overlay ✕ (real Ctrl+C) is how the app ends.
  `+` opens a user tab (blank page, address bar focused for input).
- **Address bar.** http(s) URLs load as-is; bare hosts get a scheme (http for
  loopback, https otherwise); anything else goes to a web search
  (html.duckduckgo.com, which permits framing). Address-bar navigations honor
  the same egress rule as OSC opens: loopback URLs on a remote session resolve
  through a fresh `ssh -L` on the session's machine.
- **Back / forward / reload.** The nav stack covers *yggterm-driven*
  navigations only (address bar, OSC retargets). In-surface link clicks
  navigate the native webview directly and are invisible to the shell, so the
  address bar does not follow them — documented gap. Reload bumps the tab's
  `reload_nonce`; the reconciler calls `WebView::reload` on the surface.
- **Input ownership.** While a surface covers the active terminal, the
  terminal input policy disarms the xterm textarea
  (`web_surface_active` in `ActiveTerminalInputPolicySignature`) — keystrokes
  belong to the surface.

Because each tab is a real top-level webview (not an iframe), sites that
refuse framing (X-Frame-Options / frame-ancestors: google.com, most login
pages) render normally.

## Sidebars (decision, 2026-07-04; contribution shipped 2026-07-09)

Web surfaces keep the generic yggterm sidebars: settings, notifications
(pan-yggterm), and metadata (already per-session-type by design). Those four —
plus Connect — are yggterm's own and are the only `RightPanelMode` variants
left. (The Settings main-zoom control auto-labels for what the viewport holds:
"Terminal Zoom", "Paper Zoom", or an app's own name for a live web surface —
"Ychrome Global Zoom"; see "Per-site zoom" below.)

Everything app-specific is a **contribution**: the app declares its panes over
`OSC 7717 ; sidebar` and serves each schema from a loopback control endpoint.
ychrome contributes two (vault, settings). `RightPanelMode::Vault` and
`::AppSidebar` were both deleted once the contribution covered them. See
`.agents/skills/libyggterm-surfaces/SKILL.md`.

## Ad blocking and userscripts belong to the APP (2026-07-10)

The GUI no longer reads `~/.yggterm/web-adblock/*` or
`~/.yggterm/web-userscripts/*`. Those files live on the host the app RUNS on,
which over ssh is not the GUI's host — the old arrangement had an ychrome
editing remote files that nothing ever read.

Instead the app ships its *effective* policy:

```
declare  { ..., policy_version: "<stamp>" }     # OSC, ~4s heartbeat
GET <control>/policy -> { adblock_rules, userscripts }
```

- `policy_version` is a stat-only stamp (paths + lengths + mtimes + the
  enabled/disabled decision). The GUI refetches `/policy` only when it moves,
  so a 10 KB ruleset never rides a 4s heartbeat.
- `adblock_rules` is `null` when the app says no — master switch off, profile
  opted out, or no ruleset installed. Three reasons, one answer; the GUI never
  re-derives it.
- The GUI spills the rules to a content-addressed cache under
  `~/.yggterm/web-adblock-cache/<sha256>.json` because WebKit's
  `UserContentFilterStore` compiles from a path. That cache is the ONLY thing
  yggterm persists, and deleting it costs one recompile.

**The app must declare before it opens.** Userscripts inject at
document-start, so the reconciler *holds* a surface's lazy create until the
policy lands (`SurfacePolicyGate::Pending`). A surface opened before its
contribution exists is created unblocked and runs without userscripts for its
whole life. After `MAX_POLICY_FETCH_ATTEMPTS` failed fetches the gate opens
anyway — a page with no adblock beats no page — and the user is notified.

A web surface opened by a **non-browser** app gets no adblock and no
userscripts. That is correct: adblock is browsing config, and a dashboard is
not browsing.

Changing the adblock *ruleset content* still needs a GUI restart: WebKit
compiles the filter once per process (`ensure_compiled`'s `started` flag).
Toggling it off, and every userscript change, take effect on the next surface
(re)create — reload the page.

## The User-Agent rides the same policy (2026-07-11)

`/policy` also answers `user_agent: string|null`, and the GUI hands it to
`WebViewBuilder::with_user_agent` at surface creation. Same ownership as the
ruleset: browsing config, so the app decides; only the GUI can apply it (WebKit
fixes the UA when the webview is built), so it must ride the policy. It is part
of `policy_version`'s stamp, and changing it needs `reload_surface` — an in-page
reload cannot change what the browser says it is.

**Why it exists.** WebKitGTK's default UA describes *Safari on X11/Linux*, a
browser that does not exist, and UA-allowlisting edges refuse it outright.
Verified against the live edge: claude.ai answers that UA
`403 {"error":{"type":"forbidden","message":"Request not allowed"}}` — the exact
error the user reported — while the SAME request from a macOS-Safari UA is served,
and so is Chrome-on-Linux. Only the nonexistent pair is denied, so a modern
`Version/` does not help; the platform token is the thing.

ychrome (`src/useragent.rs`) defaults to **Safari on macOS**: the engine really is
WebKit, so a site that sniffs serves WebKit-compatible code and anti-bot
fingerprinting finds an engine matching the claim. A Chrome UA over a WebKit
engine is the inconsistent one. Chrome and the raw engine default remain as
presets in YChrome Settings ▸ Browser identity.

## Per-site zoom belongs to the APP (2026-07-11)

yggterm owns one global web-surface zoom (`AppSettings.web_surface_zoom_percent`,
the Settings main-zoom control). A per-site number — some sites read better at
130%, some at 80% — is browsing config, so it lives on the app's host, declared
the same shape as the policy:

```
declare  { ..., app_name: "Ychrome", zoom_version: "<stamp>" }   # OSC, ~4s heartbeat
GET <control>/zoom -> { sites: { host: percent } }
```

- `zoom_version` is a change-detector stamp over the site map; the GUI refetches
  `/zoom` only when it moves, exactly like `policy_version`. Unlike the policy,
  the zoom fetch is **non-gating**: it never holds a surface's creation, and the
  OLD map stays applied while a refetch is in flight (no flicker to global).
- The GUI does the match itself (`zoom_override_for_host`, the twin of ychrome's
  `webzoom::zoom_for_host`): longest-suffix, so an entry for `youtube.com` covers
  `music.youtube.com`; a bare TLD is never consulted. On each navigation the
  reconciler applies the override for the page's host via `WebView::zoom`, or the
  global when a site has none. One rule, so the pane and the reconciler agree
  about which pages a stored zoom governs.
- `app_name` labels the main zoom control ("Ychrome Global Zoom"), so the user
  reads the global as the fallback the per-site overrides refine. yggterm
  hardcodes no app name.
- An action reply may set `refetch_zoom: true` (the pane's `−`/`+`/`Reset`): the
  GUI re-reads `/zoom` and applies it to the live page at once. The GUI injects
  the active surface's live effective zoom as `values.zoom` on every action so a
  pane control steps from what is on screen.

## Vertical tabs — the TAB TREE rail (reworked 2026-07-11)

Vertical mode moves the tabs OUT of the viewport into a real side rail
(`RightPanelMode::WebTabs`, titlebar button ⊟), where they are a tree with the
user's own **virtual folders** — the cwd tree's organizational grammar applied to
tabs. The first cut of this feature put the tree in a pane *inside* the page
overlay; that pane is deleted. A tree that behaves like the cwd tree belongs in a
sidebar, not in the viewport.

The rail IS the mode: opening it turns `web_surface_vertical_tabs` on, closing it
turns it off (`toggle_web_tabs_panel` → `request_web_surface_vertical_tabs`), so
there is no way to have vertical tabs with nowhere to put them. Two live restarts
proved how easily that invariant breaks, and both paths are now tested:

- A GUI that STARTS with the pref already on collapsed the strip and opened
  nothing. `upsert_web_surface` raises the rail when the pref is on.
- Opening the app's settings pane EVICTED the rail (one slot), and closing it left
  the tabs homeless. In vertical mode the rail's resting state is the tab tree: a
  pane borrows the slot and hands it back (`set_right_panel_mode`).

The address/nav bar stays in the viewport in BOTH modes — only the tabs move.
Folder affordances:
create, inline rename (double-click), collapse, delete (**the tabs return to the
root; deleting organization never deletes content**), "+" for a new tab inside a
folder, and mouse-drag a tab onto a folder to file it (the same mouse-driven drag
the cwd tree uses, not HTML5 DnD).

### Who owns what

yggterm owns the tabs, the tree, the folders and this chrome — it always did (the
tab strip, the omnibox, the history and the per-tab webviews are all GUI-side,
because WebKit runs in the GUI process). An app owns browsing *config* (ruleset,
userscripts, per-site zoom, UA) and contributes it through `/policy`.

The two **controls** nevertheless live in the app's own settings pane, because
that is where a user looks for a browser setting. The mechanism is generic, not
ychrome-specific:

- The GUI injects its prefs as page context — `?vertical_tabs=&restore_tabs=` on
  the schema GET, `values.vertical_tabs` / `values.restore_tabs` on an action —
  exactly like `values.zoom` and `values.host`.
- An action reply may carry `surface_prefs: {vertical_tabs?, restore_tabs?}`, and
  the GUI applies it to its own `AppSettings`. An absent field means "leave it
  alone", never "set it false".
- The app keeps NO copy: it renders the injected values and echoes the requested
  state back in its reply schema so the switch lands under the finger. The next
  GET re-reads the truth from the GUI.

### Classic mode, and the switch out of vertical

The classic strip has nowhere to draw a folder, so it renders **root tabs only**;
the filed ones go into an overflow menu (`🗂 N ⌄`, grouped by folder) that sits
where the old ⊟ toggle was, and appears only when something is in it. Leaving
vertical mode while folders exist raises `ClassicTabsSwitchOverlay` first, which
says exactly that. The dialog counts in `has_modal_over_viewport`: a native
surface draws above ALL DOM, so a modal over a browsing session is invisible
unless the reconciler stashes the surface.

### Tab persistence — "continue tabs from last time"

`~/.yggterm/web-profiles/<profile>/tabs.json` (GUI-side, beside `history.jsonl`
and the cookie jar) holds `{folders, tabs:[{url,title,folder,active,app_tab}]}`.
`folder: null` is a ROOT tab. Older files lack `active`/`app_tab`; both are
serde-defaulted, so they load unchanged.

**The rule:** a tab filed in a folder is *organization* and survives; a root tab
is the *browsing session* and does not. `AppSettings::web_surface_restore_tabs`
(default OFF = start fresh) decides which set a new surface reopens
(`WebTabStore::tabs_to_open`, unit-tested). A fresh start writes the purge through
immediately, so a GUI kill cannot resurrect it. The app tab's LAUNCH page is
never saved — it belongs to the app, which supplies it on the next `open` — but
the page the user *navigated* tab 0 to is the browsing session (for a plain
launch, all of it), and it is saved carrying `app_tab: true`. That mark is how a
rebuild knows which saved row the re-minted tab 0 already IS: the marked row is
handed back to tab 0 (`plan_web_tab_restore` adopts it) and never reopened as a
user tab beside it — reopening it was the duplicate-first-tab bug, one copy per
rebuild once the launch URL's redirect made the two look distinct. Adopting also
collapses the loose root copies that bug already minted (exact URL match at the
root only; a filed copy or a different page is untouched). A restored tab carries
no live handle: it is a URL in the tree until it is activated, so restoring thirty
tabs costs thirty rows, not thirty webviews.

A tab's URL IS what the tree saves, so **navigation is a tree change**: the store
is written when a tab navigates, when the page reports its real (redirected) URL
and title, when a tab is closed, and on every folder edit. Filing used to be the
only thing that persisted a tab, which meant a tab you opened and browsed was
saved as the page you started on — or, at the root, never saved at all.

**A restore is a PLACE, not just a set of rows (2026-07-13).** Restoring every tab
and then landing the user on the app's start page is not continuing where they
left off; it is stacking a page nobody asked for on top of their session. A saved
tab therefore carries `active`, and `plan_web_tab_restore` (pure, unit-tested)
decides:

- restore OFF: filed tabs only, land on the app tab. There is no session to
  return to, so a stale `active` must not drag the user into one.
- restore ON, launch carried a URL (`ychrome <url>`): every saved tab comes back,
  but the app tab keeps what was asked for and stays in front. A request outranks
  a restore.
- restore ON, launch carried NO URL (the app says so with `start_page` on the OSC
  open — only the app knows the difference): tab 0 adopts its own marked row and
  the user lands where they were standing. In a store written before the mark
  existed: if the active row was a ROOT tab the app tab ADOPTS it, so no start
  page is opened at all; if it was FILED, it is selected where it sits — adopting
  it onto the always-root app tab would quietly pull it out of its folder.
- A **re-attach** outranks all of the above for tab 0. A heartbeat rebuilding a
  surface after a GUI restart, or the daemon-retained declare replayed by the
  restore tick (`web_surface_open_kind_for_action`: the retained action
  `heartbeat` means the app has been running; only a real `open` is a launch),
  continues a run that never ended: tab 0 adopts its marked row regardless of
  the fresh-start setting — the declared URL is the run's old launch page, not a
  request — and the marked row rides through `tabs_to_open`'s fresh-start filter
  for exactly that reason. The rest of the loose session stays governed by
  `restore`.

**A restored tab has no `effective_url`.** Egress (a SOCKS tunnel, an `ssh -L`
forward) belongs to a run, not to a saved tree, so it cannot be persisted.
Selecting a restored tab therefore has to resolve it exactly as the address bar
would — `select_web_surface_tab` is the ONE door every tab home selects through,
and it does. Without that, the reconciler built the tab's webview against an empty
URL: a restored tab opened blank, which is the same as not restoring it.

### The settings file had a hand-written writer (fixed 2026-07-11)

`web_surface_restore_tabs` did not persist, and neither did `vertical_tabs` — nor
`web_surface_zoom_percent`, which had shipped as "a persisted preference" for
weeks and never was. `serialize_settings_value` in `yggterm-core` lists its fields
BY HAND, beside a parser that also lists them by hand, so a field added to
`AppSettings` alone is silently never saved.
`every_settings_field_is_written_to_the_file` compares the writer's keys against
the struct's own and fails the build the next time it happens.

## History viewer — an internal "chrome://history" page (2026-07-11)

Browsing history is generic web-surface chrome, not app-specific: yggterm already
writes it (`~/.yggterm/web-profiles/<profile>/history.jsonl`, on the GUI host, as
the reconciler follows in-page navigation) and the omnibox already reads it. The
🕘 button beside the omnibox opens a Session-Buddy-style viewer of it — entries
grouped by day, newest first, each a clickable link, with a client-side search
filter.

- The page is rendered by `render_web_history_page` (pure, unit-tested) as
  self-contained HTML (inline CSS/JS, theme-aware, every user string escaped) and
  carried to the surface's webview as a **`data:` URL**. No custom URI scheme, no
  vendored-webkit change: it loads like any URL through `navigate_web_surface_tab`.
- That nav has an internal-page guard: a `data:` URL skips egress resolution (it
  loads locally, tunnels nothing) and keeps the tab's existing egress, and is
  elided from the trace (it would otherwise write a multi-KB blob per navigation).
- The omnibox relabels it "History" (`web_surface_internal_page_label`) rather
  than showing the base64 blob; clicking a row navigates to the real URL normally.
- Capped at `WEB_HISTORY_PAGE_LIMIT` entries so the `data:` URL stays bounded.

## Renderer and security

Each tab's page is a **native child webview** (wry `build_gtk` into the main
window's `gtk::Overlay` — vendored `dioxus-desktop/src/web_surface.rs`), NOT
an iframe in the app's webview. The DOM keeps only the chrome (tab strip, nav
row, omnibox) plus a white `[data-ws-page]` placeholder div marking the page
rect. A single reconciler loop in `app()`
(`web_surface_native_reconcile_loop`) is the ONE writer of native surfaces:
it diffs `ShellState::web_surfaces` + the placeholder's
`getBoundingClientRect` against applied state and drives
create/navigate/reload/bounds/visibility/destroy. The rect is the visibility
oracle — placeholder laid out ⇒ active tab's surface shown at that rect; no
rect (session switched away, start page, other view mode) ⇒ hidden. Surfaces
are created lazily on first visibility and kept alive (hidden) across tab
switches, so page state survives like `display:none` iframes did.

Security properties:

- Surface content lives in its own top-level webview with its own
  `WebContext` — it has no handle on the app's main frame, so the old iframe
  sandbox and the vendored http(s) navigation gate
  (`set_webview_http_navigation_open`) are retired; the main webview's
  navigation policy stays fully closed.
- Per-surface `WebContext` also means per-surface cookies/storage and a
  per-surface network proxy — the SOCKS egress substrate.
- Z-order caveat (v1): native surfaces paint above ALL DOM, including dialogs
  and context menus that overlap the page rect.

Known accepted risk (v0): any program that can write to the PTY can emit the
OSC (same class as OSC 777 fake notifications) — e.g. `cat`ing a crafted file
opens a surface pointing at an attacker URL. The surface is visibly labeled
with its URL and one keypress (Ctrl+C) removes it.

### The app control token: a page must not be able to drive an app's pane (2026-07-27)

`yggterm-appctl://` is registered on every surface's `WebContext` and proxies to
that session's app control endpoint (`app_control_proxy`, vendored
`web_surface.rs`). It exists for the passkey shim — a userscript in the RP's page
POSTing `/fido2/get` — but it is a **generic proxy**, so page JS could address
any route on that endpoint. ychrome gated only `/fido2/get|create`, which left
`POST /action` open to every page in the surface: ad blocking off, userscripts
deleted, identity switched, and a vault `fill` whose reply is an `eval` the GUI
injects **into the requesting page** — a plaintext credential handed to whoever
asked.

The contract, GUI side (the app side is `ychrome/docs/protocol.md`, which owns
the route table and the refusal semantics; this half owns how the GUI proves who
it is):

- The `sidebar ; declare` OSC carries `control_token`. It is the ONE secret a
  declare may carry, because the PTY stream is exactly the channel a page cannot
  read — the same provenance as the passkey `request_id` behind `/fido2/grant`.
- It is stored on `SidebarContributionState` and read ONLY through
  `sidebar_control_token()` — a second accessor beside `sidebar_control_url()`,
  because the url is quoted into traces all over `shell.rs` and a token
  travelling with it would end up in one of them. It is set-if-present, so a
  ping never clears it, and REFRESHED by every declare that carries one: an app
  daemon that respawned onto the same port mints a new token, and the url
  identity check would not notice.
- **Every** request the GUI makes to an app's control endpoint presents it as
  `X-Ychrome-Control`; the app decides per route whether it demands one. The two
  credential-free calls are named in `the_settings_panes_action_presents_the_declared_token`:
  the picker's `/open` (a different server) and the pre-contribution liveness
  probe.
- The appctl bridge forwards a **closed allow-list** of page headers
  (`FORWARDED_HEADERS` = `X-Ychrome-Fido2`, `Content-Type`) and refuses a request
  target or token value containing a control character, so a page can neither
  send the control header nor smuggle one through a CRLF in the request line.
  The signer's token IS forwarded and grants nothing: every page in the profile
  already holds it, baked into its own shim userscript.

This closes the WEB boundary only. A same-uid process on the app's host can
reach the vault socket directly and always could; that was never this endpoint's
threat model.

## Profile picker (no-arg `ychrome`)

`ychrome` with no URL serves a **profile picker** instead of opening a blank
page. In thin-client mode it binds a loopback HTTP server on the invoking host
and emits OSC action `pick`, whose payload URL is that server's **control
endpoint** rather than a page to display: yggterm renders a NATIVE profile
picker in the viewport, and the user's choice makes the GUI `GET /open?url=&profile=`
on the endpoint. ychrome's handler re-emits OSC `open` with the chosen
url+profile, and the app tab retargets (same profile → navigate; different
profile → the surface's `WebContext` is rebuilt, per host-owned profiles). This
also fixes the old no-arg case: ychrome no longer emits `about:blank`, which
`web_surface_url_scheme_allowed` rejects (only http/https pass).

### Profile metadata: `web-profiles/<name>/profile.json`

A profile's jar carries an optional sidecar, `profile.json`. Its ONE owner is
`yggterm_core::web_profile::ProfileMeta` — format, defaults and policy — for the
same reason `normalize_web_profile` lives there: a profile means one thing to
every process that opens it.

| key | meaning |
| --- | --- |
| `emoji` | the owner's chosen avatar. Absent ⇒ derived, see below |
| `protected` | this profile refuses deletion |
| `display_name` | a label to show instead of the directory name (decoration only; identity, locks and paths still key on the directory name) |

Five rules, each of which has a lock:

1. **Unknown keys survive a rewrite.** `agent_drive` (`ychrome/docs/agent-engine.md`
   §7) is specced into this same file and is written by a different process. A
   blind overwrite from the GUI would silently re-grant agent driving on a
   profile whose owner denied it, so every write is a read-modify-write through
   `ProfileMeta`, which round-trips keys this build has never heard of.
2. **The default avatar is derived, never stored.** It is FNV-1a over the
   *normalized* name, modulo a curated 48-emoji table
   (`WEB_PROFILE_AVATAR_EMOJI`) — no clock, no randomness, no enumeration order,
   and no `DefaultHasher` (whose output is not guaranteed stable across Rust
   releases). The same profile therefore looks the same in the GUI, in the
   daemon and in the next process. Reordering or resizing the table re-assigns
   every derived avatar: treat it as a user-visible change.
3. **A stored avatar is validated on READ, not only on write.** The typed field
   passes `web_profile_emoji_is_valid`, but by rule 1 this file is written by
   processes that never saw that predicate, so a `profile.json` this build never
   wrote can carry a paragraph in `emoji` — and the badge pills are 9.5 px chips.
   `web_profile_stored_avatar` asks the SAME predicate on the read side and the
   renderer falls back to the derived default. The foreign bytes still
   round-trip verbatim: declining to paint a value is not licence to delete
   another process's write.
4. **Permanence is a LIST, not a comparison.** `WEB_PROFILE_PERMANENT` (today:
   `default`) is the only place that says which profiles are permanent, read
   through `web_profile_is_protected_by_construction`. `web_profile_is_protected`
   and `web_profile_delete_refusal` both derive from it, and so does the picker's
   protect toggle — a render site that re-spelled `name == WEB_PROFILE_DEFAULT`
   would keep offering a verb the delete guard refuses the day a second name
   joins the list. Permanence does not consult the file, so a missing (or
   hostile) `profile.json` cannot unprotect it.
5. **A refusal is NAMED.** Deletion policy — unsafe name, ephemeral, permanent,
   protected — is `web_profile_delete_refusal`, and every refusal carries a
   sentence the UI shows verbatim, on the picker's subtitle line
   (`data-web-picker-notice`). A guard that silently does nothing is
   indistinguishable from a broken button.

The GUI draws the avatar in three places — the native picker card, the classic
tab-strip identity pill and the vertical rail's — through ONE function,
`web_surface_profile_avatar`. The old "first letter on a gradient" avatar is
gone: it was a second encoding of identity that only the picker implemented, so
the badges could never match it. Right-clicking a picker card raises the shared
`ContextMenuOverlay` with *Change avatar…* and the protect toggle. Right-clicking
the avatar FIELD is a different gesture: the input stops the card's handler
without cancelling the event, so WebKit's own Copy/Paste menu opens (the picker
container is in `NATIVE_CONTEXT_MENU_OWNER_SELECTORS` for exactly that reason,
and pasting is how a user without an emoji IME enters one).

#### `RowMenuItem::disabled` — shown, dimmed, and inert in BOTH views

The protect toggle on a permanent profile is the first user of `disabled`, but
the field lives in the SHARED `RowMenuItem` vocabulary, so its contract is
app-wide and has two enforcement points, neither of them CSS:

- **Mouse:** `ContextMenuOverlay`'s onclick asks `context_menu_item_dispatches`
  before calling `on_action`. Dispatch-level, not `pointer-events:none` — a
  styling accident can undo a style, and the refusal must not be a style.
- **Keyboard (ALT/KeyTip):** `build_keytip_scopes` does not DECLARE a disabled
  item in the `rowmenu` scope, so no letter resolves to it and the badge painter
  never paints an accelerator that would do nothing. `dispatch_row_menu_action`
  is reached only through a declared node, so the terminus stays a single
  spelling of "run this id".

Dimming is `context_menu_item_style` (every branch emits the identical style key
set — Dioxus applies `style` property-by-property and never clears a dropped
key), plus a `data-context-menu-disabled` attribute that both a live probe and
the stylesheet read: the shared `.yggterm-menu-item:hover` highlight is
suppressed for a dimmed entry, because a highlight is the strongest "clickable"
signal the menu has.

⚠ **ychrome's HTML fallback picker still derives a first letter** (`picker_html`
in that repo). It is a separate repository and a separate binary; until it is
synced to `ProfileMeta`, a profile can look one way in yggterm's native picker
and another in ychrome's standalone one.

### A control endpoint is not a webview URL

The GUI fetches a control endpoint **itself**, over a hand-rolled `TcpStream`.
That is a different resolution problem from a URL the *webview* loads:

| | resolver | remote-session mechanism |
| --- | --- | --- |
| webview URL | `resolve_web_surface_effective_url` | URL untouched; webview is pointed at an `ssh -D` SOCKS proxy |
| control endpoint | `resolve_control_endpoint_url` | loopback URL rewritten to the local end of an `ssh -L` forward |

The GUI's HTTP client speaks no SOCKS, so running a control endpoint through the
webview resolver hands back `http://127.0.0.1:<port>/…` unchanged and the GUI
then connects to **its own** loopback — the wrong machine, silently. Anything
the GUI fetches (the picker's `/open`, and the sidebar-contribution surface's
schema/action routes) must use `resolve_control_endpoint_url`.

## Resolved in 2.9.61

- **Reload paints white with 2+ tabs** — FIXED. WebKitGTK composited a reloaded
  frame offscreen but never re-blit it while a sibling surface webview shared the
  `gtk::Overlay`; GTK-level nudges (`queue_resize`, hide/show remap, 1px
  `set_bounds`, throwaway overlay child) all left it white. Only **destroying a
  webview** forces the survivors to re-composite, so reload now = **destroy +
  recreate the tab's webview**. Made lossless by preserving the per-profile
  `WebContext` across the rebuild (persistent jar under `~/.yggterm/web-profiles/`).
- **Local sessions spawned pointless SOCKS tunnels** — FIXED. A `local://`
  session no longer gets a non-null `socks_port`; its surface egresses directly
  (`ssh_target = localhost` no longer routes through `ssh -N -D`).

## Screenshot caveat for agents

Native surfaces are invisible to `server app screenshot`'s default in-process
composite (`xterm_canvas_composite_over_dom` pastes the xterm canvas over a
DOM snapshot — a native GTK widget is in NEITHER layer). Verifying a web
surface needs a compositor-level grab: `server app screenshot --backend os`
(KWin/Spectacle path, v2.9.57+), or the `web_surface` trace events (`open` /
`close` / `native_open` / `native_close` in event-trace.jsonl).

**The response now says so itself.** When a native surface is visible and the
backend is not the compositor, the capture reports
`capture_native_web_surface_visible: true` and forces `capture_faithful: false`
with a reason naming `--backend os`. It used to answer `capture_faithful: true`,
which is how the resize bug below survived a "live-verified" review: every crop of
the right rail looked perfect because the page painted over it was not in the frame.

## Native surfaces can be moved AND resized (fixed 2026-07-10)

A surface's geometry is driven by the `[data-ws-page]` placeholder rect. Applying
it must update the **webview's GTK size request**, not just the container's —
see `apply_bounds` in `vendor/dioxus-desktop/src/web_surface.rs`.

`wry`'s `WebView::set_bounds` on a `GtkFixed` parent only `size_allocate`s the
webview; it never touches the size request that `add_to_container` set when the
webview was built. `GtkFixed` allocates children at their natural size, and a
widget's natural size IS its size request — so the next layout pass (the
`queue_resize` every caller issues immediately afterwards) snapped the webview
straight back to the size it was born with.

The surface could therefore be moved but never resized. Opening the right rail
over a live web surface left the page painted across the rail (a native child
draws above all DOM); closing the rail left a blank gap. Recreating the surface
(reload, profile or proxy change) hid the bug, because a fresh webview is born at
the current rect.

## A native surface is a TENANT of the viewport (2026-07-13)

A native child webview paints above **all** DOM. Everything else follows from
that:

- **The auto-hidden titlebar is `position:absolute` over the content**, and a
  web surface would swallow it whole — it could not even be hovered back,
  because the reveal sensor was under the webview too. The titlebar stays the
  same floating overlay everywhere (an in-flow variant re-laid-out the whole
  window on every hover — rejected 2026-07-13); instead the reconciler's rect
  eval **clamps the native webview below the titlebar's live bottom edge**
  (`[data-titlebar-auto-hide-enabled="true"]`'s rect). Collapsed, that keeps
  the 6px hover sensor real DOM; revealed, the titlebar sits on top of
  everything and only the page dips under it.
- **The web overlay takes NO inset and NO radius** (2026-07-13): the terminal
  frame's 4px inset, painted in the chrome colour, was the "border around every
  page" the user kept reporting. A page runs edge-to-edge in its viewport; a
  native rect cannot be corner-clipped anyway, so the radius was already a
  fiction at the page's corners.

This is load-bearing, not cosmetic: the reconciler re-measures the placeholder
every tick, so a surface that is a tenant of the viewport follows a window resize
and a split; one that is a lid on top of it does not.

## Popups: `window.opener` and `window.close()` (2026-07-13)

A link opened with `target="_blank"`, a middle/ctrl-click, or `window.open`
becomes a TAB — but the webview is built inside WebKit's `create` handler,
**related to its opener**, and handed straight back
(`NewWindowResponse::Create`). The shell then ADOPTS it
(`web_surface_adopt_popup_tab`); it does not open one.

This is not a detail. The old path denied the window and reopened the URL in a
fresh webview, which produced a tab with no relation to its opener:
`window.opener` was `null`, so an OAuth callback's `opener.postMessage(...)` went
nowhere, and `window.close()` had nothing to close. Every popup-based sign-in
(claude.ai -> Google) hung exactly there: the user authenticated, the popup sat
there forever, and the page that started the flow never learned it had won. The
cookie landed, so the NEXT launch was silently signed in — which is how a broken
channel disguised itself as a flaky login.

An adopted popup inherits the opener's profile and `socks_port`, because a
related view shares the opener's WebContext (its jar, its proxy, its web
process). Recording anything else would make the reconciler see a proxy change
and destroy the very webview the opener relationship lives in. The egress rule
therefore still holds: the popup rides the opener's tunnel.

### Two things WebKitGTK does not do (proven on the harness, not read)

1. **It never emits its `close` signal for a `window.close()`** — not even for a
   window a script opened, the one case every browser honors. `load-changed`
   fires on the very same webview object while `close` never does, so this is the
   engine's refusal, not a missed connection. A browser that cannot close a popup
   strands every OAuth sign-in ever written, so the PAGE reports it (a
   `window.close` shim over a script-message channel) and the **host decides**:
   only a tab a script opened may be closed this way (Chrome's rule). The engine's
   native `close()` is deliberately not called through — it tears the page down
   while telling the embedder nothing, so a refusal that called it would leave the
   user staring at a white rectangle where their tab used to be.
2. **A related view gets its OPENER's user-content manager**, so a popup's script
   message arrives on the OPENER's channel (the popup was surface 2; its close
   arrived as surface 1). The channel cannot say who is asking, so the page names
   itself — `href` plus whether `window.opener` is live — and the shell resolves
   which tab that is.

## An open app pane follows the page (2026-07-13)

The GUI reports the page context (host, live zoom, HTTPS) and the app renders its
pane from it, so the moment the page moves, the pane the app drew describes
somewhere the user no longer is. It used to be fetched only when the pane was
OPENED, which is why the vault pane went on offering claude.ai's logins after a
sign-in popup took the front on accounts.google.com. It was not wrong about its
page; nobody had told it the page had changed.

The refetch lives in the native-surface reconcile tick, which is the one place
that sees every way a page can move: a navigation, a tab switch, a popup taking
the front, a session switch.

## Split-tabs: a tab pinned to its own pane (2026-07-17, libyggterm Phase 3)

Every tab is already its own webview; the reconciler normally shows the active
one and hides the rest. A split group member may now be `(session, Web{tab})` —
a pane PINNED to one tab, independent of the surface's active tab — so one
browsing session can show two of its tabs side by side ("two tab webviews, two
rects"). No app involvement: tabs are GUI chrome by doctrine.

- **Geometry**: the pinned pane renders pure page (no strip/omnibox — the
  session's terminal-view pane keeps the chrome) with an inner
  `[data-ws-pinned-session]` + `[data-ws-pinned-tab]` placeholder, the
  (session, tab)-keyed twin of `[data-ws-page]`. The reconciler's placement
  rule (`web_surface_tab_place_rect`): a pane pinned to exactly this tab wins
  its rect; else the surface's page area shows the ACTIVE tab; on the
  degenerate collision (the strip switched onto the pinned tab) the pinned
  pane wins — one webview cannot sit at two rects.
- **Creation**: `yggterm-headless server app split web-tab <session> <tab>
  [--axis ...]`. Refused when the tab does not exist or the session is already
  grouped.
- **Focus**: pane-INDEX-keyed (`focused_pane_index`) — a session seated in two
  panes rings only one. `server app split focus <session> [pane]` is the only
  headless (and currently only reliable) way to focus a pinned pane: its
  native webview swallows pointer events, so click-to-focus works only on
  regions the webview does not cover. The focus ring stays an outward shadow
  in the Phase-0 gutter, so it survives the webview painting above all DOM.
- **Focus tenancy**: `web_surface_host_label` — the one owner of page context
  (vault `values.host`, the app-pane refetch, chrome host) — answers from the
  FOCUSED pane's tab via `focused_web_tab_id`. Focusing the pinned docs pane
  makes an open vault pane refetch for the pinned page's host on the next
  reconcile tick. (Omnibox/address-commit and keyboard tenancy are not yet
  pane-keyed — the omnibox still shows and edits the surface's active tab.)
- **Lifetime**: pinned panes are SESSION-LIFETIME. Tab restore re-mints tab
  ids, so a persisted pin has no durable referent: `prune_web_view_panes`
  drops a pin when its tab closes, its surface retires (close/sweep/Ctrl+Z),
  or across a GUI restart, and a group below 2 panes dissolves.

## The `server app web` verb plane (2026-07-25)

The agent-facing surface of a web surface. `yggterm server app web --help`
renders from `WEB_ACTIONS` in `apps/yggterm/src/main.rs`, and a test
(`every_web_action_appears_in_the_usage_string`) fails when the dispatcher's own
match arms disagree with it — because a stale usage block already caused one
"not deployed" misdiagnosis in the field (docs/agent-control-plane.md:1275). If
you add a verb and skip the usage entry, the build fails. That is deliberate.

### App-control is a FILESYSTEM DROPBOX, not RPC

The CLI writes `~/.yggterm/app-control-requests/<uuid>.json`; the GUI polls it.
The daemon is **not** in this path and never deserializes `AppControlCommand`.
Two consequences an implementer will otherwise rediscover the hard way:

- **CLI and GUI must be swapped together.** A newer CLI's verb reaching an older
  GUI is a version mismatch, not a bug.
- **It answers honestly now, for the kind AND for the payload.** A well-formed
  request with an unknown `kind` deserializes into
  `AppControlCommand::Unsupported` and is REFUSED with
  `unsupported_command_kind`. A KNOWN kind whose FIELDS this build cannot read
  is salvaged from the envelope into `AppControlCommand::Unreadable` and refused
  with `unreadable_command_payload` plus the serde error (`invalid type: map,
  expected a string`) as the clue — that is the mismatch a changed field shape
  produces, e.g. `do click --text` against a GUI that types `selector` as a bare
  string. Before that, both were deleted unread and the caller saw a bare
  timeout. Malformed JSON, and any file whose envelope is not a request, is
  still deleted — a corrupt file is not a version mismatch.
- **A `#[serde(default)]` field added to an EXISTING command is silently
  DROPPED by an older GUI**, which is worse than a timeout: the verb succeeds
  and does the wrong thing. Every such field must be ECHOED in the response and
  the CLI must hard-fail when the echo is missing. `--frame` is the worked
  example (`frame_resolved`).

### Which verbs need a MAPPED surface

| Needs a mapped surface | Works on a soft-stashed / never-revealed one |
|---|---|
| `do`, `batch`, `fill-vault` (they synthesize GDK events; an unmapped webview drops them, so they fail closed with `surface_not_mapped`) | everything eval-backed: `eval`, `await`, `read`, `frames`, `wait`, `capture-element`, `cookies`, `screenshot`, `lease`, `ensure` |

Explicit JS eval keeps running on a throttled (hidden) view — see
`WebSurfaceHost::set_throttled`. That is why `capture-element` works on a
surface the user has never seen.

### Addressing an element

One type, `WebElementRef`, carried by every selector-shaped field of a `do`
action. A bare string is a CSS selector (so every payload written before this
existed still parses); an object addresses by visible text or by role+label.
Resolution happens IN THE PAGE, immediately before injection — a rect resolved
in an earlier request is never reused.

Ties are broken deterministically: candidates in document order, any candidate
that CONTAINS another candidate dropped (so a substring match never selects
`<body>` over the button inside it), then `--nth`. For role, exact label matches
are preferred over substring ones as a fixed rule.

`do click` reports `resolved: {css_path, tag, rect, on_target, is_connected}`
next to `delivered`. Keep the three questions apart:

- `accepted` — the injector ran.
- `resolved.*` — what the DOM said about the node at click time.
- `delivered` — what the page's own listener observed.

A node that resolves but is not in the document is REFUSED (`detached_node`), because
a React re-render drops agent-injected ids and an event fired at a detached node
delivers nothing.

### `batch`: what the envelope means

`web batch` answers `{accepted, requested, attempted, succeeded, failed,
actions, aborted_at, abort_reason}`. The counts are separate on purpose:

- `requested` — actions asked for.
- `attempted` — actions that actually ran (`succeeded + failed`). Short of
  `requested` only when the batch stopped early.
- `succeeded` / `failed` — what came of them.
- **`accepted` is strict: the batch ran to the end AND every action it
  attempted succeeded.** A partial batch is `accepted: false`. This is the one
  field a caller that does not walk `actions[]` will read, so it must never be
  true for a run that delivered nothing — with the default
  `stop_on_error: false`, a 31-field fill in which all 31 selectors miss is
  `accepted: false, attempted: 31, succeeded: 0, failed: 31`.

The human wins at the batch's START as well as mid-run. Seat input is read
BEFORE the lane reset, so a click that landed between the agent's last verb and
its `web batch` refuses the batch (`preempted`) instead of being absorbed by the
reset the verb performs on the agent's own behalf. That refusal consumes the
count, so the next `batch` (or `do --new-batch`) opens normally — one refusal is
the whole cost of yielding.

### Frames

`web read` with no `--frame` searches EVERY reachable frame and returns
`frames: [{frame:{path,url,accessible}, result, error}]`, top document first
(frame `[]`), **and keeps answering `result`** — the top document's answer,
which is exactly the pre-frames shape. `frames` is additive; dropping `result`
would have handed every existing caller a silent `null`, which is the failure
class this verb family exists to kill. A frame it cannot read is REPORTED with
`accessible:false`, not omitted — "there is a frame here I cannot read" and "there is no frame here" are
different facts, and a silent `[]` from the top document reads as "the site does
not offer this". `web frames` gives per-frame element and interactable counts.

`--frame` addresses SAME-ORIGIN frames on `eval` and `read`. Cross-origin frames
are enumerated but not addressable, and `--frame` on `do` is not implemented:
`do` synthesizes an event at widget coordinates, so a frame-relative rect must be
composed through every ancestor's `getBoundingClientRect`. Workaround: read the
frame to find the target, then click the TOP document's coordinates of the
iframe element offset by the frame-relative rect.

### Cookies: the export is NOT complete, and the import may hit the user's jar

`web cookies --import <jar> | --export <jar>` speaks Netscape format both ways —
what `curl -c`/`-b` writes and reads. `crates/yggterm-shell/src/netscape_cookie_jar.rs`
is the one owner of that format. Two things to know:

- **`export_scope: "root_path_per_domain"`.** WebKitGTK 4.x has no
  dump-the-whole-jar API: `cookies()` is per-URI and libsoup enforces the
  cookie's path against it. So an export is every root-path cookie of every
  domain the jar knows, and path-scoped cookies are missing. Do not reach for
  the on-disk sqlite jar to close the gap — it is a second encoding of the
  cookie store and blind to unflushed in-memory state.
- ⚠ **The jar is per-`WebContext` = per-PROFILE, and a surface with no explicit
  profile is `default` — the user's own browsing jar.** Drive agent work on a
  `--profile agent-<n>` surface before importing. The response reports which
  profile was written; check it. The trace records domains and counts and never
  a name or a value.

### Waiting through a navigation

An eval failure during `wait` is NOT-YET, not the end: a multi-origin
auto-submit chain tears down and rebuilds the content process at every hop, and
the old code returned on the first commit. `eval_errors` is reported so a wait
that spent its whole budget failing is legible.

`--until url:matches:<regex>` and `--until settled:<ms>` are read from the
ENGINE's own page state, with no page eval at all, which is what makes them
answerable while the content process is being rebuilt. `url:contains:<s>` is
sugar compiled into the same regex predicate at the CLI.

### Async

`eval` returns a script's COMPLETION value, and a Promise is not one — the
engine answers `WEBKIT_JAVASCRIPT_ERROR_INVALID_RESULT`. `web await` is the ONE
async bridge; do not re-invent stash-and-poll. A poll failure is not-yet, and a
stash missing after a document change is `document_replaced`, never a fabricated
result.

`eval`'s refusals are now distinct: `js_result_unsupported` (the script ran and
returned something unserializable — the PAGE is fine) versus
`webview_unreachable` (nobody answered). Those two shared one string, and the
ambiguity cost a field run ten minutes.

### Recovering a surface

`web ensure` probes LIVENESS, not emptiness: tabs → handle → engine liveness →
**a bounded eval round trip**. The first three are UI-process facts and stay
true over a dead content process, which is how `ensure` used to hand back a
corpse and report success. Compare `generation_before` with `generation_after`
(and `healed`) to tell a new page from the same one. `web reload` and `web close`
reach the same recovery directly.

Refusals name which fact failed: `no_declare`, `declare_stale` (the app EXITED —
relaunch it, do not retry), `declare_url_scheme_refused`,
`declare_without_url`, `daemon_declare_unavailable` (a failed FETCH is not an
absent declare), and `session_closed`.

⛔ **`session_closed` is not retryable.** `ensure` refuses when the session's
runtime is gone AND the user's close of its row is remembered. Reviving a
surface there produces the one state that must not exist — a live, leased page
with no row anywhere reflecting it — which happened: an agent drove a real page
for an hour on the user's profile with zero rows showing that session. Create
your OWN session (`server app terminal new`) and drive its surface; a closed row
is never resurrected, and the refusal carries both facts
(`runtime_running`, `row_close_remembered`) so you can see which one applied.
The check is CONJUNCTIVE, so the two legitimate revivals still work: a live but
backgrounded session whose surface the reaper collected, and a session that
never mounted a terminal host. An unreachable owner reports neither — a failed
declare fetch is not a dead runtime, and refusing on it would take `ensure` away
from every session owned by a predecessor daemon.

**Closing a session ends its surfaces.** A live-session close now tears down
every web surface declared under that session (any equivalent spelling of the
path), which ends the surface's lease and drops its headless-create claim. An
agent whose surface disappears can tell why: the trace carries
`web_surface/closed_with_session`. Only an EXPLICIT close does this — surfaces
are never pruned against a daemon snapshot, because a row owned by a preserved
predecessor daemon drops out of that snapshot while its session is fine.

⛔ **The declare's URL scheme gate is not negotiable.** It guards two
PTY-authored paths, and any process that can write a session's PTY can emit that
OSC (see "Renderer and security" above). Allowing `file://` there would let a
crafted byte stream open a local secret in a webview that `web read --as text`
then exfiltrates. Serve a fixture over loopback http instead; the allowlist
already permits it.

### Deploying while an agent is driving

`server app state | jq .agent_leases` answers "is someone mid-flow" in one call,
from the same field the reaper reads. `server app update restart` and the
preserving-close door both REFUSE with `agent_lease_active` while a lease is
live, unless `--force`. Honest limit: this stops the app-control restart door,
not `pkill yggterm`.

## What a tab actually costs, and where the ceiling is (2026-07-26)

Recorded because two sessions have now re-derived it, and one workstream was
costed on a comment that is false.

**⛔ `render_probe`'s "WebKitGTK runs one web process per profile, serving every
surface on it" is WRONG.** `WebSurfaceHost::open` calls `WebContext::new(profile_dir)`
unconditionally, **once per SURFACE**, and a surface is keyed `(session_path, tab_id)`.
wry's `WebContextImpl::new` builds a fresh context every time, so two tabs of one
profile get two distinct `WebKitWebContext`s pointing at the same directory: two
process pools, two web processes, two network processes, and **two in-memory
cookie jars writing the same file**. Per-process IS per-tab. Profile partitioning
is therefore not the lever; sharing one context per profile is.

**Tabs live in yggterm, not in ychrome.** ychrome has no concept of a tab — it
emits OSC 7717, heartbeats, and serves policy/zoom over a loopback endpoint.
The tab tree, omnibox, webviews, profile jars, egress tunnels and lifecycle are
all in `crates/yggterm-shell/src/shell.rs` and `vendor/dioxus-desktop`. Any
"ychrome performance" work is ~95% in this repo.

**One tunnel per session, not per tab.** `ssh -N -D` is the session's egress.
Reuse used to be keyed on the TAB while `web_surface_new_tab` mints
`socks_port: None`, so every new tab spawned its own — N ssh children here, N
sshds and N loopback ports on the remote, tripping the remote's MaxStartups
before anything on this side notices. `web_surface_socks_egress_donor` picks the
donor deterministically by tab id (never strip order, which the user can drag),
and `web_surface_forward_is_last_holder` is the refcount that stops any tab
tearing down a tunnel its siblings are still using.

**Invisible is not unwanted.** The axis is PAINT, not existence: stop
compositing and rAF for an unseen surface; leave audio, timers and network
alone. `webkit_web_view_is_playing_audio` is read at the moment of the decision
(a cached flag would be worse than none) and vetoes DESTROY only — the throttle
still runs, so a background playlist stops painting and keeps playing.

**Reclaim needs hysteresis.** See `docs/agent-field-guide.md` §7.5: a surface
that keeps being re-created after each reap is demonstrably wanted, and
destroying it again reclaims nothing while paying for a fresh web process.

**Still unbuilt, in value order:** per-tab lifecycle (the governor is keyed on
SESSIONS, so a foreground session's N tabs live forever); tab discard + restore
via `WebKitWebViewSessionState` — the only mechanism that changes the order of
magnitude; a process-model policy that reads the machine (process-per-site is
harmful on a 14 GB laptop and free on a large-memory server); and an instrument
that can see a tab at all.

## Downloads (2026-07-27)

Until this, a download **landed somewhere else, silently** — and getting the
archaeology right matters, because this file is the record. wry's
`WebViewAttributes::default()` carried `download_started_handler:
Some(Box::new(|_, _| true))`, `attach_handlers` registered it on the shared
context unconditionally, and its `decide-destination` computed the path itself:
`dirs::download_dir().unwrap_or_else(current_dir)` followed by
`PathBuf::push(suggested)`. So under a bare compositor, where
`XDG_DOWNLOAD_DIR` is unset, a downloaded file went to **the GUI's working
directory**, under whatever name the server asked for (`../../x` walks straight
out of a `push`), with no toast, no trace row and nothing anywhere to say a
transfer had happened. The transfer was never dropped on the floor; it was
**unowned and unannounced**, which is worse, because there was nothing to
notice. That is a hard blocker for using ychrome as the main browser, so
downloads now have a plane — and, like every other concept here, exactly one
owner.

**Where a file goes** is decided by `download_destination` in
`vendor/dioxus-desktop/src/web_surface.rs` and by nothing else:
`$HOME/Downloads` (created if missing; deliberately not `XDG_DOWNLOAD_DIR`,
which is unset under a bare compositor and would move the folder depending on
how the GUI was launched), a SANITIZED basename, and a uniquified name.

- **Sanitize** because the suggested name is attacker-controlled
  (`Content-Disposition: filename=...`). Path separators are cut to the last
  segment (`../../.ssh/authorized_keys` → `authorized_keys`, backslashes too —
  the name may come from a Windows server), leading dots are stripped so a
  download cannot become a dotfile, control characters (NUL included, which
  would truncate the path at the syscall) go, and an empty result falls back to
  a fixed name rather than to anything derived from the URL.
- **Uniquify, never overwrite**: `report.pdf`, then `report (1).pdf`. Multi-dot
  names keep their whole extension (`archive (1).tar.gz`). "Taken" is
  `symlink_metadata`, not `Path::exists`: `exists` FOLLOWS links and answers
  `false` for a **dangling** one, so a symlink planted at
  `~/Downloads/report.pdf` and pointing outside would have read as a free name
  and the write would have landed through it — the traversal `sanitize` exists
  to prevent, arriving by the other door.

**Connected once per `WebContext`, not per webview.** `download-started` is a
signal on the CONTEXT, and the tabs of one session share a context — connecting
per surface would decide one transfer once per tab. The vendored wry's own
default download handler is switched OFF (`download_started_handler: None`) for
the same reason: it computed its own destination with `PathBuf::push`, which a
`../../` name walks straight out of, and it returned `true` from
`decide-destination` — so leaving it in place would have put a second,
unsanitized policy on the same signal, and the unsanitized one would win. The
cost of that switch is that a webview WITHOUT this plumbing (the shell's own
window) no longer downloads at all, which is the honest state: it never
surfaced a download to the user anyway.

**The failure reason is the ENGINE'S.** `failed` carries a `GError` and fires
before `finished`; the reason is parked and the single terminal event is emitted
from `finished`, so a transfer can never produce both a completion and a
failure. A failure also SWEEPS THE PArecordsAL: WebKitGTK writes straight to the
destination (no `.part` staging), so a transfer that dies halfway leaves a
truncated file with the right name and the wrong contents — a download
masquerading as complete.

**The sweep is gated on OWNERSHIP**, and that gate is load-bearing rather than
decorative. Because `decide-destination` sets `set_allow_overwrite(false)`,
"the destination already exists" is a first-class WebKit failure — and it is
exactly the failure in which the file at the destination is *somebody else's*:
a sibling transfer that decided the same name in the same main-loop turn (the
uniquifier reads the directory, and WebKit does not create the file until after
`decide-destination` returns), or any other process that wrote it in that
window. An unconditional `remove_file` would delete a stranger's file on the one
path where the failure MEANS a stranger's file. So the engine's own
`created-destination` signal — which fires when and only when WebKit created the
file it is about to write — parks a flag, and only that flag lets the sweep run.
This is the single place in the plane where a bug destroys data instead of
misplacing it.

**A download outlives its tab.** A running transfer holds its `WebContext`
(`DownloadInFlight`), which is also what keeps the context sweep
(`retain_held_contexts`, all `prune_contexts` does — an entry survives while
anyone besides the map holds it) from taking the network process out from under
it. WebKitGTK has no "detach" verb, so *outliving the surface* is spelled as an
owner that outlives it. If the engine gives up anyway when the view is
destroyed, that arrives as `failed` and the partial is swept; the third outcome,
a truncated file under the full name, is ruled out either way. The rule is
driven, not just scanned: a lock builds the registry entry, drops the tab's
hold, runs the sweep and finds the engine still standing — then drops the
transfer's hold and finds the next sweep taking it. What that lock does **not**
prove is the engine half: no real WebKit transfer has been observed continuing
past its view's destruction, because that needs a display. It is on the
live-proof list below.

**What the user and the agent see.** The shell drains the queue each reconcile
tick and sends every transition to the two planes it already has: a toast
through `push_notification` (started names the file; completion names the file
and the folder; failure names the file and the engine's reason) and a trace row
per transition — `download_started`, `download_completed` (with `bytes`),
`download_failed` (with `reason`), all carrying `file_name`, `path`, `url` and
the owning session/tab when the surface is still alive. No downloads UI of its
own: a browser with two places to look is a browser you have to be taught. A
`server app downloads` listing verb is the natural next step for agents — the
trace answers "what happened" but not "what is running right now"
(`web_surface_downloads_in_flight` is the count that verb would report).

**Not live-proven, and here is exactly what that means.** Every DECISION in this
section is driven end to end by a lock — the destination against a real
localhost server answering with a real `Content-Disposition`, the directory
policy by calling `downloads_dir()` itself under a `HOME` the lock owns and an
`XDG_DOWNLOAD_DIR` decoy it plants, the failure path against a real transfer
killed mid-flight, the ownership gate and the detach rule against the
production functions. What has never been observed firing is the ENGINE side:
`decide-destination`, `created-destination`, `failed` and `finished` need a
display and a WebKit process, and the wiring that hands our decisions to them is
locked by anchored source scans over product lines only. Live proof still owed
on guihost: click a real download link in a surface, screenshot the toast, confirm
the file and the three trace rows, then kill the network mid-transfer and
confirm no partial survives — and confirm a transfer continues after its tab is
closed.
