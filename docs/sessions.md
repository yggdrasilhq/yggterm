# Yggterm Sessions

Yggterm sessions are durable handles for a terminal routine, not a second rendering model. A session should feel like a snappy automation of `ssh <machine>; cd <cwd>; codex resume <uuid>` or the equivalent local shell task.

## Identity

- The daemon owns live PTY identity and I/O.
- Codex transcript JSONL identity is the saved-session identity when it is available.
- Synthetic runtime keys such as `codex-runtime://...` are terminal I/O keys only. They must not become user-facing saved-session identity.
- Generic local and remote shell runtimes may use daemon keys such as
  `live::...`. These keys are terminal runtime handles only. Plain terminals
  appear in `Live Sessions` while they are running and do not create cwd tree,
  Startpage, title/summary, or saved-session rows. The cwd is launch
  provenance, not workspace identity.
- Durable sessions and app-grade surfaces may appear in `Live Sessions` and in
  their cwd/machine group, but both views must resolve to the same session id
  and metadata record.
- A fresh remote Codex start is runtime-only until Codex exposes a real transcript storage path. If the user closes the onboarding surface before `Codex Session` and `Storage` metadata exist, the daemon must remove the live runtime and must not create a saved `remote-session://...` row.
- Fresh remote Codex onboarding, sign-in, and setup menus are still interactive
  PTY surfaces. The GUI must allow input and dismiss resume/loading gates when
  xterm shows one of those menus, even though the runtime is not yet a durable
  saved session and has no prompt-ready transcript identity. This includes
  truncated visible tails of the same auth menu after Codex logo art; a partial
  xterm sample must not turn the menu into stale non-prompt text.
- The `Live Sessions` row order is user-owned once the user drags rows. The
  daemon persists that order and restore must replay it exactly. Focusing,
  switching, refreshing, or restart recovery must not silently convert the list
  back into recency order. New live runtimes may enter at the top until the user
  moves them.

### Runtime-identity rebind

When yggterm launches a new Codex or Claude Code session it synthesizes a
`Uuid::new_v4()` live-session id before the CLI assigns its own real session id
to the transcript. Left alone, the two never reconcile and a restart tries to
`resume` an id the CLI never created. The daemon closes this drift by rebinding
the synthesized id to the real CLI session id discovered from the running
process, through one source of truth
(`apply_codex_runtime_identity_to_live_session` /
`apply_claude_code_runtime_identity_to_live_session`). The synthetic
`codex-runtime://...` key stays the terminal I/O key; only the saved-session
identity and metadata are rewritten.

Identity is discovered per locality:

- **Local Codex / Claude Code** — the daemon walks the live PTY's process tree
  and reads the open `~/.codex/sessions/.../<id>.jsonl` (or
  `~/.claude/projects/<cwd>/<id>.jsonl`). Runs inside `persist()`.
- **Remote Codex** — the daemon cannot walk a remote process tree, so it
  SSH-invokes `yggterm server remote local-codex-identities` on the owning
  machine (which enumerates that machine's running Codex/Claude Code processes
  and emits their real session ids), then matches each live remote-Codex row to
  a running transcript by cwd. This runs on a background chore, never on the
  synchronous request path, and is self-limiting: a row stops being polled as
  soon as it is rebound, and an un-matchable row is abandoned after a bounded
  number of attempts. Disable with `YGGTERM_DISABLE_REMOTE_CODEX_IDENTITY_POLL`.

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

When the same durable live runtime is projected in both Live Sessions and a
machine/cwd folder, drag/drop feedback is row-local. Dragging the Live Sessions
copy may not also show a ghost drag on the cwd projection, even though both rows
resolve to the same runtime path. Plain terminals have no cwd projection, so
their row order is only the `Live Sessions` order.

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
- Web View is a provider-backed conversation surface. Codex and terminal-backed
  sessions render stored transcript/context blocks read-only; they do not expose
  a composer and do not type through the terminal. Future chat apps can reuse the
  same Web View shell only after declaring an explicit API provider and send
  capability.
- Switching a live session to Web View must not close, detach, restart, or hide
  the daemon-owned runtime. The live runtime remains in `Live Sessions` and can
  be restored by switching back to Terminal; Web View is only read-only
  presentation over saved transcript/provider data.
- When the active Web View has a hydrated provider transcript window, that
  active session payload is the authoritative Web View read model. `Live
  Sessions` and machine/cwd rows are sidebar/runtime projections and may carry
  only shallow preview blocks. Shell reconciliation must not replace the
  hydrated active transcript with a shallow live-row projection just because
  both rows share the same session path. For remote Codex sessions, interactive
  hydration should use a bounded recent transcript window rather than shipping a
  full multi-megabyte JSONL transcript through daemon snapshot IPC. Once that
  recent-tail window is mounted, older head, scan, loading, or empty preview
  projections are downgrades and must not clobber the active reader. Terminal
  mode keeps the inverse rule: prefer the live runtime projection for xterm
  attachment and PTY status.
- If a live-session snapshot must truncate transcript preview blocks for
  sidebar/runtime projection size, it must truncate from the head and keep the
  latest tail. A shallow projection must not expose old transcript-head blocks
  under `Preview Hydration=tail`; that creates a false observer that can replace
  the active Web View conversation during restore.
- Web View chat mode must lead with provider transcript turns. Generated goals,
  summaries, and rendered context are secondary presentation sections and must
  appear after the transcript turns so they never masquerade as the conversation
  itself.
- Live Codex Web View opens at the latest hydrated transcript turn. Older
  transcript blocks remain provider data, but the first viewport after Terminal
  -> Web View must not look like the start of an old unrelated conversation
  when a recent-tail transcript window is present. Large transcript readers
  must materialize the latest transcript window for the first frame, then may
  also seed/pin the real scroll container through the mounted latest transcript
  anchor. The global post-render scroll script is only a secondary nudge. A
  shallow head projection or scroll position zero is not an acceptable first
  frame for a hydrated tail.
- Startpage saved-session cards are durable saved-session rows only. Live
  runtime projections, generic SSH terminals keyed by `live::...`, and fresh
  remote Codex starts without transcript `storage_path` stay out of saved UUID
  cards on the Startpage. Generic SSH/local terminals are visible only through
  `Live Sessions`; durable remote Codex starts may project into machine/cwd
  only after transcript identity is known.
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
- Startpage actions are for local/recent work and the currently scoped folder:
  new Codex session, New Terminal, folder creation, rename, and summary edits.
  New Terminal starts a transient live PTY in that cwd; it does not create a
  saved session card. SSH connection belongs in the titlebar/right rail or
  context surfaces, not as a Startpage button.

## Non-Goals

- Do not invent alternate prompt, cursor, or summary overlays to hide PTY/xterm defects.
- Do not let sidebar labels, live rows, daemon runtime keys, and transcript ids diverge into separate sources of truth.
