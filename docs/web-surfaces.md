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

## The egress rule

**A surface's network egress is the invoking host's network — for ALL URLs.**
Each tab of a remote session's surface gets its own `ssh -N -D <port>` SOCKS
tunnel to the session's machine, and the tab's webview (private `WebContext`)
proxies every request through it via `ProxyConfig::Socks5`. The *remote sshd*
resolves every hostname and originates every connection on that machine —
loopback URLs reach the REMOTE loopback. The tunnel dies with the tab. If the
SOCKS tunnel cannot be established, loopback URLs fall back to the older
`ssh -N -L` per-URL forward, and anything else falls back to direct load from
the GUI host — a traced egress gap (`egress_gap` in the `open`/`tab_navigate`
trace events), not a silent one. Local sessions load directly, no proxy.

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

## Vertical tabs — a browsing-mode toggle (2026-07-11)

A per-user preference (`AppSettings::web_surface_vertical_tabs`, persisted) that
flips the web-surface chrome between classic top tabs and a left-hand pane. The
⊟ button in the top strip turns it on; the ▭ button in the pane turns it off.

In vertical mode the overlay's outer flex direction becomes `row`: a fixed 224px
left pane holds a mini-omnibox, the tabs as a **domain-grouped tree**
(`web_surface_tab_tree`, unit-tested — domains in first-appearance order, tabs in
order within), a "+ New tab", and an extension slot (history today). The top tab
bar and address bar collapse away (`max-height`→0 transition), and the pane
slides in (`ygg-vtab-slide-in`). The native page rect (`[data-ws-page]`, still
`flex:1`) simply fills the space right of the pane, so the reconciler follows
without any surface-placement change. All tab actions reuse the same state
methods as the classic strip (`web_surface_select_tab` / `_close_tab` / `_new_tab`).

Because it is generic web-surface chrome, it applies to every web-surface app.
The mini-omnibox is intentionally minimal (type-and-Enter, no dropdown yet). The
setting is exposed in `app state` as `web_surface_vertical_tabs`.

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
