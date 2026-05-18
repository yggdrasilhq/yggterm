# Paper Integration Experiment

Branch: `experimental/paper-integration`

Worktree: `~/gh/yggterm--paper-integration`

Channel binary: `yggterm-paper`

Standalone target: `~/gh/paper`, `github.com/avikalpa/paper`, Apache-2.0

## Product Shape

Paper is a Notion-like WYSIWYG planning and scratchpad app. Its standalone form
should be an app-grade product with YggUI-style chrome, two sidebars, and a main
document viewport. The editing model should favor structured WYSIWYG blocks over
Markdown as the primary user experience.

## Yggterm Integration

The Yggterm integration embeds Paper as a first-class surface next to terminal
sessions.

- Yggterm owns the left navigation through its metadata tree.
- The integrated Paper surface does not bring its standalone left sidebar.
- Paper documents opened from Yggterm store their data in SQLite under the
  selected `YGGTERM_HOME`, not as standalone Paper files.
- Paper nodes should behave like durable metadata tree entries, similar to
  sessions and terminal nodes.
- App-control should expose enough state to prove open document identity,
  dirty/saved state, viewport focus, and storage location.

## Non-Goals

- Do not use Paper data as terminal/session truth.
- Do not route terminal input through Paper surfaces.
- Do not collapse the standalone Paper product into Yggterm-only code.

## First Milestone

- Create/open one Paper node from the Yggterm tree.
- Edit WYSIWYG content in the main viewport.
- Persist and restore the content from the Yggterm database.
- Prove the workflow with app-control state and a screenshot.
