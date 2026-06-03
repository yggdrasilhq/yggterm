# Terminal Backend Notes

Yggterm's default terminal path is daemon-owned PTY plus embedded xterm.js. That
is the product contract for input, output, scrollback, resize, cursor, prompt
styling, and terminal observability.

Ghostty remains useful reference material and a possible future backend, but it
is not the active embedded terminal surface for Yggterm.

## Shell-terminal persistence: host daemon is the target; tmux is a stopgap

Per the decentralized host-resident daemon architecture (see `AGENTS.md` →
"Terminal multiplexer positioning"), the persistence layer for **plain shell
terminals** is — by design — the host's own `yggterm-headless` daemon. The daemon
runs on the machine where the shell lives (local or each SSH host), owns the
shell PTY, retains its scrollback, and keeps it alive across GUI/client
disconnect and SSH death. A GUI client reaches it over SSH (SSH is the auth
layer); metadata lives in that host's `~/.yggterm`.

**Current reality (stopgap, to be removed):** shell terminals are launched via
`yggterm attach <uuid>` → `crates/yggterm-server/src/attach.rs::exec_tmux`, which
execs into `tmux new-session` when tmux is available. This was a shortcut to get
shell persistence (especially remote, across SSH drops) before the host daemon
owned shell PTYs end-to-end. It is **harmful and not a design choice**:

- tmux owns the scrollback (copy-mode), so xterm.js sees only the current screen
  → "no scrollback buffer" for shell sessions.
- tmux draws its own status bar — an unintended, un-yggterm UX surface.
- tmux's screen model fights xterm.js scroll/selection.
- It is pure redundancy for *local* shells, where the daemon already owns the PTY.

**Target end state:** the host `yggterm-headless` owns shell PTYs directly — no
tmux process, no tmux status bar, no tmux-owned scrollback. Remote-shell
survival across SSH death comes from the daemon running ON THE REMOTE HOST (which
yggterm already self-installs there, see `[[spec-cli-binary-auto-provisioning]]`),
not from tmux. Until that path is fully wired, do not deepen the tmux dependency;
prefer removing it (local first) and, where remote persistence still needs a
host-side holder, route it through the host daemon rather than tmux.

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
