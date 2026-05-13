# Yggterm Sessions

Yggterm sessions are durable handles for a terminal routine, not a second rendering model. A session should feel like a snappy automation of `ssh <machine>; cd <cwd>; codex resume <uuid>` or the equivalent local shell task.

## Identity

- The daemon owns live PTY identity and I/O.
- Codex transcript JSONL identity is the saved-session identity when it is available.
- Synthetic runtime keys such as `codex-runtime://...` are terminal I/O keys only. They must not become user-facing saved-session identity.
- Generic remote shell runtimes may use daemon keys such as `live::...`; the key remains the terminal runtime handle, but the UI must still classify it as one live SSH session and project it into Live Sessions plus the matching machine/cwd group when cwd is known.
- Sidebar rows may appear in Live Sessions and in their cwd/machine group, but both views must resolve to the same session id and metadata record.

## Titles

- Every session has a UUID immediately.
- A new session can show a UUID fallback while there is not enough transcript context.
- After the first meaningful prompt/task has completed, Yggterm should generate a human title from the session JSONL and persist it in `~/.yggterm/session-titles.db`.
- A manually renamed title is authoritative until the user regenerates or clears it.
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

## Closing

- Closing a live session closes the daemon-owned runtime. It does not delete the stored transcript, durable session metadata, manual title, generated title, or summary timeline.
- Closing a background live session must not change the active viewport.
- Closing the active live session must never leave the closed session selected in Web View, terminal recovery, or a busy refresh loop.
- The active-close fallback is the most recent valid viewport target before the closed session: another live/stored session in its prior mode, then the last scoped Startpage, then the global Startpage.
- Viewport history is a stack of stable targets, not a second source of truth. It stores session paths or Startpage scopes only so close can choose a replacement; the daemon and metadata store remain authoritative for runtime and copy.
- When a session is closed, every matching session path or normalized live-runtime alias is pruned from viewport history before fallback selection. This keeps the sequence `open A`, `open B`, `close A`, `close B` from falling back to the already closed A.
- The daemon must not invent an arbitrary replacement active session after removing a runtime. If a GUI wants to focus another session, it must do so explicitly from the validated viewport history.

## UI Contract

- Startpage cards show the title, long UUID, cwd/host context, and the current summary/timeline preview.
- Title rename uses a pencil action beside the title.
- Summary edit/append uses a pencil action on the right side of the card and in the titlebar summary surface.
- The titlebar summary surface presents a scrollable timeline; selecting a timeline entry shows that paragraph without changing terminal focus.
- UUIDs are degraded fallback labels, not a normal steady state.
- Startpage actions are for local/recent work and the currently scoped folder: new Codex session, local terminal, folder creation, rename, and summary edits. SSH connection belongs in the titlebar/right rail or context surfaces, not as a Startpage button.

## Non-Goals

- Do not invent alternate prompt, cursor, or summary overlays to hide PTY/xterm defects.
- Do not let sidebar labels, live rows, daemon runtime keys, and transcript ids diverge into separate sources of truth.
