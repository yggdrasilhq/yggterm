# Yggterm Sessions

Yggterm sessions are durable handles for a terminal routine, not a second rendering model. A session should feel like a snappy automation of `ssh <machine>; cd <cwd>; codex resume <uuid>` or the equivalent local shell task.

## Identity

- The daemon owns live PTY identity and I/O.
- Codex transcript JSONL identity is the saved-session identity when it is available.
- Synthetic runtime keys such as `codex-runtime://...` are terminal I/O keys only. They must not become user-facing saved-session identity.
- Generic remote shell runtimes may use daemon keys such as `live::...`; the key remains the terminal runtime handle, but the UI must still classify it as one live SSH session and project it into Live Sessions plus the matching machine/cwd group when cwd is known.
- Sidebar rows may appear in Live Sessions and in their cwd/machine group, but both views must resolve to the same session id and metadata record.
- The `Live Sessions` row order is user-owned once the user drags rows. The
  daemon persists that order and restore must replay it exactly. Focusing,
  switching, refreshing, or restart recovery must not silently convert the list
  back into recency order. New live runtimes may enter at the top until the user
  moves them.

The cross-system ownership table is in
`docs/architecture-audit-2026-05-16.md`. Session code must not infer saved
identity from whichever projection appears first. A live row, cwd row,
keep-alive flag, retained terminal host, generated title, summary, or runtime
key is evidence about the session; none of those replaces the saved-session
identity or metadata record.

On restart or daemon snapshot refresh, the GUI may temporarily see projections
arrive out of order. It must reconcile them to the same saved-session identity
before allowing rename, copy regeneration, keep-alive state, cwd placement, or
terminal open to mutate durable state. If reconciliation fails, surface an
incident state instead of promoting the first projection to truth.

The shell-side heuristics that decide whether visible title copy is low-signal,
whether passive copy generation may start, and how generic terminal labels are
humanized live in `crates/yggterm-shell/src/session_copy_policy.rs`. UI code may
project those decisions into rows, cards, and menus, but it must not keep a
second low-signal title classifier or passive-generation gate.

## Titles

- Every session has a UUID immediately.
- A new session can show a UUID fallback while there is not enough transcript context.
- After the first meaningful prompt/task has completed, Yggterm should generate a human title from the session JSONL and persist it in `~/.yggterm/session-titles.db`.
- A manually renamed title is authoritative until the user regenerates or clears it.
- Remote scan previews and generated copy jobs may fill missing or fallback
  titles, but they must not overwrite a human/live title already attached to the
  session. Keep-alive rows such as `samplenotes` and `erome systemd` must survive scan
  refreshes unless the user explicitly renames or regenerates them.
- If generated and manual titles are unavailable or rejected as low-signal, the sidebar falls back to the short UUID for both live and machine/cwd rows.

## Summaries

- Every session should have a human-readable summary.
- Summaries are stored as current summary plus an append-only timeline.
- Each generated summary is a dated handoff paragraph: objective, concrete progress/findings, and current blocker or next step.
- Summary refresh cadence is three days for sessions active since the last major update, or earlier when the user explicitly regenerates.
- Manual summary edits append to the same timeline and update the current summary.
- `yggterm-headless server sessions regenerate-copy` regenerates missing titles, precis, and summaries. Use `--reset-summary-history` when the timeline quality is not yet trustworthy and needs to be rebuilt from transcript truth.
- The maiden bookkeeping pass for a bounded app graph is `yggterm-headless server sessions regenerate-copy --skip-local --reset-summary-history`. It rebuilds summary history for app-discovered remote machines without walking every historical local Codex archive.
- A full local archive rebuild is explicit: omit `--skip-local`, preferably with `--budget <n>` during smoke runs. Unbounded local history regeneration is an archive maintenance task, not the default release gate.
- The pass generates missing human titles and keeps UUID labels only for sessions where transcript context or model generation is insufficient.
- The GUI should run the same missing-copy bookkeeping opportunistically in bounded background jobs after startup and after fresh daemon snapshots. It may defer work for terminal responsiveness, but it must not require clicking a generic `Yggterm Codex` or `Yggterm Shell` row before the session can ever receive a real title/summary.
- Generated-copy jobs may fill missing title or summary data only after identity
  reconciliation. They must not overwrite manual titles, live user labels, or
  existing high-signal generated titles just because a scan found a lower-signal
  transcript excerpt.

## Closing

- Closing a live session closes the daemon-owned runtime. It does not delete the stored transcript, durable session metadata, manual title, generated title, or summary timeline.
- Closing a background live session must not change the active viewport.
- Closing the active live session must never leave the closed session selected in Web View, terminal recovery, or a busy refresh loop.
- The active-close fallback is the most recent valid viewport target before the closed session: another live/stored session in its prior mode, then the last scoped Startpage, then the global Startpage.
- Viewport history is a stack of stable targets, not a second source of truth. It stores session paths or Startpage scopes only so close can choose a replacement; the daemon and metadata store remain authoritative for runtime and copy.
- When a session is closed, every matching session path or normalized live-runtime alias is pruned from viewport history before fallback selection. This keeps the sequence `open A`, `open B`, `close A`, `close B` from falling back to the already closed A.

Remote cwd bookmarks created from a machine/folder context menu are saved as local metadata, but they render in the owning remote machine tree. The synthetic storage path is an implementation detail; the sidebar must not leak it as a local `/__remote_folder__/...` row. A saved remote bookmark should remain visible even if the remote scan currently has no session under that cwd.

The remote bookmark projection uses the complete saved workspace model, not the
currently expanded local sidebar rows. Collapsing a local tree branch, restarting
the GUI, or filtering visible local rows must not hide a saved remote cwd folder
from its owning machine tree. The visible local tree and the remote projection
are separate read models over the same saved metadata.

Renaming a remote cwd bookmark changes the cwd bookmark path, not only the
display title. A bookmark created under `practice:/home/pi` and renamed to
`git/samplers` represents `practice:/home/pi/git/samplers`. Startpage
actions launched from that scoped folder must pass that exact remote cwd to the
daemon for both Codex sessions and generic SSH terminals.
- The daemon must not invent an arbitrary replacement active session after removing a runtime. If a GUI wants to focus another session, it must do so explicitly from the validated viewport history.

## UI Contract

- Startpage cards show the title, long UUID, cwd/host context, and the current summary/timeline preview.
- A scoped Startpage's recent-work list is scoped by both machine and cwd. A
  local folder such as `/home/pi` may show only local stored/live sessions under
  that cwd; remote sessions on `dev`, `practice`, or any other machine require a
  remote machine/folder scope even when their cwd string is identical. The list
  is computed from the full sidebar session tree, not just currently expanded
  rows, so collapsing a folder must not make its Startpage look empty. Local
  Codex session rows are ordered by their source JSONL mtime, not by filesystem
  tree order.
- Title rename uses a pencil action beside the title.
- Summary edit/append uses a pencil action on the right side of the card and in the titlebar summary surface.
- The titlebar summary surface presents a scrollable timeline; selecting a timeline entry shows that paragraph without changing terminal focus.
- UUIDs are degraded fallback labels, not a normal steady state.
- Startpage actions are for local/recent work and the currently scoped folder: new Codex session, local terminal, folder creation, rename, and summary edits. SSH connection belongs in the titlebar/right rail or context surfaces, not as a Startpage button.

## Non-Goals

- Do not invent alternate prompt, cursor, or summary overlays to hide PTY/xterm defects.
- Do not let sidebar labels, live rows, daemon runtime keys, and transcript ids diverge into separate sources of truth.
