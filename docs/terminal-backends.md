# Terminal Backend Notes

Yggterm's default terminal path is daemon-owned PTY plus embedded xterm.js. That
is the product contract for input, output, scrollback, resize, cursor, prompt
styling, and terminal observability.

Ghostty remains useful reference material and a possible future backend, but it
is not the active embedded terminal surface for Yggterm.

## Ghostty Status

The local Ghostty review from 2026-03-19 found that upstream is splitting the
project into reusable layers:

- `libghostty-vt`: a reusable virtual-terminal core for parsing, terminal
  state, scrollback, input encoding, formatting, modes, and related APIs. It is
  promising and portable, but explicitly unstable.
- full `libghostty`: still shaped around Ghostty's own app/runtime ABI. The
  macOS app consumes it, but it is not a documented general-purpose embedding
  surface, and Linux does not expose an equivalent stable widget/runtime API.

Linux Ghostty is GTK-runtime centered, not a small embeddable surface we can
drop into Dioxus without accepting major upstream friction. macOS is the most
plausible future full-embedding path because Ghostty's own macOS app already
uses `libghostty`, but even there the API should be treated as unstable.

## Yggterm Policy

- Keep `yggterm-server` as the owner of sessions, PTYs, retained scrollback, and
  runtime lifecycle.
- Keep xterm.js as the embedded viewport until a backend change is explicitly
  designed and smoke tested.
- Do not route default terminal behavior through Ghostty internals to fix a
  rendering bug.
- If `libghostty-vt` is revisited, treat it as a terminal-core experiment first,
  with Yggterm still responsible for renderer, PTY integration, responses, and
  app-control proof.
- Any alternate backend must preserve `docs/xterm.md`'s single-source terminal
  truth: the terminal surface is fed by runtime bytes, not preview transcript
  text or shell-owned decorative repair layers.
