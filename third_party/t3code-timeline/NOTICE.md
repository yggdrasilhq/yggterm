# t3code timeline renderer — vendored

This directory contains source **copied from T3 Code** and adapted for yggterm.

- **Upstream:** <https://github.com/pingdotgg/t3code>
- **Upstream commit:** see `UPSTREAM_COMMIT`
- **License:** MIT — full text in `LICENSE.t3code`
- **Copyright:** © 2026 T3 Tools Inc.

The MIT license permits this reuse. It also *requires* that the copyright
notice and permission notice travel with the copy, which is what
`LICENSE.t3code` is for. Do not delete it, and do not relicense these files
under yggterm's own terms — they remain T3 Tools' code under MIT.

## Why only part of it

We take T3 Code's **transcript renderer**, not their application. Their
`ChatView` is coupled to their product model — git branches and worktrees,
pull requests, a tanstack router and store, and a large WebSocket contract
served by their own backend. yggterm's model is different: a session is the
agent CLI's own JSONL, and the CLI owns its TUI.

`MessagesTimeline` is the piece that is genuinely separable. Its only real
input is `deriveTimelineEntries(messages, proposedPlans, workEntries)`, and
`ChatMessage` is a small, honest shape:

```ts
{ id, role: "user" | "assistant" | "system", text, createdAt, streaming }
```

which both Claude Code and Codex transcripts map onto cleanly. That is the
whole reason this integration is affordable.

## What is vendored

| path | upstream path |
|---|---|
| `src/vendor/components/MessagesTimeline.tsx` | `apps/web/src/components/chat/MessagesTimeline.tsx` |
| `src/vendor/components/ChatMarkdown.tsx` | `apps/web/src/components/ChatMarkdown.tsx` |
| `src/vendor/session-logic.ts` | `apps/web/src/session-logic.ts` (trimmed to the timeline half) |

…plus the sibling cards, `ui/` primitives and small helpers those two import.
Each vendored file keeps a header comment naming its upstream path.

## Local changes

Kept deliberately small, so re-syncing with upstream stays possible:

1. **Electron/native seams stubbed.** `nativeApi`, `editorPreferences` and
   "open in editor" resolve through a single `host.ts` shim instead of
   Electron IPC. yggterm is not Electron.
2. **Theme is a prop, not a hook.** `useTheme` read their store; ours takes
   the resolved `"light" | "dark"` from the mount call, because yggterm owns
   the theme (`DESIGN.md`).
3. **Git-only affordances are inert.** Turn diffs, checkpoint revert and
   worktree actions have no meaning here — yggterm does not manage branches
   for a session — so those props receive no-ops and empty maps. The code
   paths are left intact rather than deleted, so an upstream re-sync is a
   copy, not a merge.

Nothing else is edited. Anything we want to *change* about the look belongs
in our own wrapper (`src/`), not in `src/vendor/`.
