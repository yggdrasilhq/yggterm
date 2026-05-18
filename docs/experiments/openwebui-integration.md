# OpenWebUI Integration Experiment

Branch: `experimental/openwebui-integration`

Worktree: `~/gh/yggterm--openwebui-integration`

Channel binary: `yggterm-openwebui`

## Goal

Explore how OpenWebUI workflows should live inside a remote-first Yggterm
workspace.

## Product Shape

The branch should make OpenWebUI feel reachable from machines, sessions, or
workspace metadata without turning terminal sessions into browser tabs.

Potential surfaces:

- configured local or remote OpenWebUI endpoints
- embedded web surface or external-browser launch, depending on auth and runtime
  constraints
- metadata-tree entries for OpenWebUI workspaces or conversations
- clipboard and screenshot handoff between terminals and OpenWebUI

## Guardrails

- OpenWebUI content is not terminal truth, session identity, or PTY output.
- Auth tokens, endpoint URLs, and cookies must be handled as secrets.
- Remote endpoints should be represented as metadata, not ad hoc shell text.
- The channel must use isolated experimental state unless testing a migration
  after snapshotting stable state.

## First Milestone

- Configure one endpoint.
- Open the endpoint from a Yggterm tree node or command.
- Preserve enough metadata to reopen it.
- Prove state, screenshot, and any secret redaction behavior.
