# Excalidraw Obsidian Integration Experiment

Branch: `experimental/excalidraw-obsidian-integration`

Worktree: `~/gh/yggterm--excalidraw-obsidian-integration`

Channel binary: `yggterm-excalidraw-obsidian`

## Goal

Explore Yggterm workflows that connect terminal sessions, Obsidian vault notes,
and Excalidraw diagrams.

## Product Shape

This is an integration branch, not a clone of Obsidian or Excalidraw. Yggterm
should make planning artifacts easy to open, link, and update from a terminal
workspace.

Potential surfaces:

- metadata entries for vaults, notes, canvases, or diagrams
- session-linked notes and diagrams
- screenshot or clipboard handoff into an Obsidian/Excalidraw workflow
- app-control proof that a linked artifact opened correctly

## Guardrails

- External vault files remain external content. Yggterm may store metadata,
  links, and workflow state under `YGGTERM_HOME`.
- Do not make Obsidian or Excalidraw content a source of terminal/session truth.
- Keep file mutation explicit and observable.

## First Milestone

- Configure one vault path.
- Open or reveal one note/diagram from Yggterm metadata.
- Attach a session link or screenshot handoff.
- Prove the flow with app-control state and artifact paths.
