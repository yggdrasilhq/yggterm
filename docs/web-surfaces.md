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

**A surface's network egress is the invoking host's network.** For a remote
session with a loopback URL, the GUI spawns `ssh -N -L <local>:<host>:<port>`
to the session's machine — the *remote sshd* resolves the host and originates
the target connection on that machine — and points the iframe at the local
end. The forward dies with the surface. Non-loopback URLs currently load
directly from the GUI host (documented v0 gap; the general fix is a
per-surface SOCKS egress, see the ychrome repo's protocol doc).

## Renderer and security

The surface is an iframe overlay inside the GUI webview. Two guards:

- dioxus-desktop's navigation policy vetoes all http(s) navigations (subframe
  loads included, on WebKitGTK); each surface origin is registered on a
  process-global allowlist (`allow_webview_navigation_prefix`, vendored
  dioxus-desktop) to permit exactly those loads in-frame.
- The iframe carries `sandbox` WITHOUT `allow-top-navigation`, so embedded
  content cannot use the allowlist to navigate the app's main frame.

Known accepted risk (v0): any program that can write to the PTY can emit the
OSC (same class as OSC 777 fake notifications) — e.g. `cat`ing a crafted file
opens a surface pointing at an attacker URL. The surface is visibly labeled
with its URL and one keypress (Ctrl+C) removes it.

## Screenshot caveat for agents

`server app screenshot` composites the xterm canvas OVER the DOM in the
terminal rect — a DOM overlay inside that rect is invisible in the composite.
Verifying a web surface needs an OS-level capture (Spectacle) or the
`web_surface` trace events (`open` / `close` / `iframe_load` in
event-trace.jsonl).
