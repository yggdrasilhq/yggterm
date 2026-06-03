# Design: shell-terminal persistence via the host yggterm server (remove tmux)

Status: DESIGN / not yet built (2026-06-03). Author note: this is the concrete
plan behind `AGENTS.md` → "Decentralized host-resident daemon architecture" and
the tmux-stopgap section of `docs/terminal-backends.md`. Review before code.

## Goal

Plain **shell** terminals must be held alive by the **host's own yggterm server**
(daemon), not by tmux. Remove the tmux dependency entirely:
- no tmux process, no tmux status bar, no tmux-owned scrollback;
- full xterm.js scrollback (the daemon's vt100 ring is the source of truth);
- survives client disconnect AND SSH death;
- yggterm's own UX end to end.

Nomenclature (see AGENTS.md): **yggterm** = GUI client, **yggterm-headless** =
headless client for agents, **yggterm server** = the session-holding daemon.

## Why tmux is there today (and why it's wrong)

Shell terminals launch via `yggterm attach <uuid>` →
`crates/yggterm-server/src/attach.rs::exec_tmux` → `tmux new-session`. tmux was a
shortcut to get persistence — especially *remote* persistence across SSH drops —
before the host server owned shell PTYs end to end. It is harmful: tmux owns the
scrollback (xterm sees only the current screen → "no scrollback"), draws its own
status bar, and its screen model fights xterm scroll. For *local* shells it is
pure redundancy (the server already owns the PTY).

The objection "the local server can't keep a remote shell alive across an SSH
drop" is not a reason for tmux — it is the reason the **server runs on the remote
host**. yggterm already self-installs its binary on remotes
(`[[spec-cli-binary-auto-provisioning]]`), so the remote server is half-built.

## Target architecture

For BOTH local and remote shells, the shell PTY is owned by the yggterm server
**on that host**:

```
client (yggterm GUI / yggterm-headless)
   └── SSH (auth + transport) ──> host yggterm server (daemon)
                                     └── owns shell PTY + vt100 scrollback ring
                                          └── /bin/$SHELL (no tmux)
```

- **Local shells:** the local yggterm server spawns and owns the PTY directly
  (it already does this for the daemon-owned PTY model). Just stop routing through
  `exec_tmux`; spawn the shell directly (like `exec_shell`, the existing
  tmux-absent fallback path).
- **Remote shells:** the GUI opens an SSH channel to the remote host and speaks
  the yggterm server protocol to the remote `yggterm-headless server` (already the
  pattern for remote agent CLIs). The remote server spawns/owns the shell PTY and
  retains its scrollback. The shell survives because the remote server (not the
  SSH channel) owns it.

## What changes

1. **Launch:** retire `exec_tmux`; shell sessions spawn the shell directly under
   the host server's PTY ownership. `attach.rs` keeps the metadata/`exec_shell`
   path; the tmux branch is deleted (or env-gated off, then deleted once the
   remote-server path is proven).
2. **Remote server lifecycle:** the GUI must ensure a `yggterm-headless server`
   is running on each remote host it talks to, start it on demand over SSH if
   absent (idempotent; auto-provisioned binary), and reconnect to it. This is the
   biggest new piece — model it on the existing remote agent-CLI session path
   plus the daemon idle/retire lifecycle (`[[bug-class-old-daemon-never-retires]]`,
   now with the supersession self-retire).
3. **Reconnect over SSH:** on SSH drop, the client re-dials and re-attaches to the
   remote server by session id (the server kept the PTY + cursor). Reuse the
   chunk-cursor read protocol (`terminal.rs::read(cursor)`) — including the
   alt-screen-aware gap re-sync just added — so a reconnect after the server
   trimmed its ring re-syncs cleanly.
4. **Scrollback:** the daemon vt100 ring (`DAEMON_VT_SCROLLBACK_ROWS = 10k`) is
   already the retention source; with tmux gone, xterm.js renders that scrollback
   directly. Verify the keepalive-restart-viewport-only path
   (docs/xterm-bugs.md#keepalive-restart-viewport-only) holds for shells.

## Migration / risk

- Stage behind a flag: keep tmux launch available but default OFF for local first
  (lowest risk — local server already owns the PTY), prove it, then remote.
- Remote is the hard part (server lifecycle + reconnect). Build the offline +
  isolated-host test harness FIRST (per this session's lesson: never ship
  server/PTY pipeline changes to a live machine unverified).
- Existing tmux-backed sessions: they keep working on the old path until migrated;
  new shells use the host-server path once enabled.

## Open questions for review

- Remote server: one shared server per host, or per-user? (Decentralized model →
  per-user under that user's `~/.yggterm`.)
- How aggressively to start a remote server on first contact vs. on first shell
  open (latency vs. surprise).
- Authentication beyond SSH? (Per the model: SSH access IS the authorization;
  no separate auth.)
