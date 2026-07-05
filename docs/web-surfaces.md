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

## Sidebars (decision, 2026-07-04)

Web surfaces keep the generic yggterm sidebars: settings (zoom controls are
named "Viewport Zoom", not "Terminal Zoom", for exactly this reason),
notifications (pan-yggterm), and metadata (already per-session-type by
design). libyggterm apps may later contribute *additional* per-app sidebars
and their own app icon — Cellulose is the first expected consumer (one
unified scrollable ribbon sidebar to pair with ALT+ navigation).

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

## Screenshot caveat for agents

Native surfaces are invisible to `server app screenshot`'s default in-process
composite (`xterm_canvas_composite_over_dom` pastes the xterm canvas over a
DOM snapshot — a native GTK widget is in NEITHER layer). Verifying a web
surface needs a compositor-level grab: `server app screenshot --backend os`
(KWin/Spectacle path, v2.9.57+), or the `web_surface` trace events (`open` /
`close` / `native_open` / `native_close` in event-trace.jsonl).
