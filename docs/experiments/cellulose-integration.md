# Cellulose Integration Experiment

Branch: `experimental/cellulose-integration`

Worktree: `~/gh/yggterm--cellulose-integration`

Channel binary: `yggterm-cellulose`

Standalone target: `~/gh/cellulose`, `github.com/avikalpa/cellulose`,
Apache-2.0

## Product Shape

Cellulose is a modern spreadsheet app for finance workflows: a simpler Excel
built from the ground up, with finance-world standards compliance and FMI
standards as the default modeling posture.

The standalone app should use YggUI-style automation and observability so it can
be driven as both a human-facing spreadsheet and a Codex-friendly DSL.

Standalone Cellulose should have:

- Yggterm-like shell chrome, including hideable titlebar behavior
- a left sidebar for the sheets tree
- a main spreadsheet viewport
- a right sidebar for structured Excel-ribbon-like commands
- ALT+ command organization with configurable hotkeys
- a hotkey configuration file so default bindings can be close to familiar
  spreadsheet workflows without shipping a Microsoft-branded or copied keymap
- an explicit compatibility path for user-supplied keymaps, subject to separate
  licensing/IP review before distribution

Excel subset compatibility is priority one. The branch should prefer correct
spreadsheet semantics over novel UI when those goals conflict.

## Yggterm Integration

The integrated Yggterm version embeds Cellulose as a single-sheet surface inside
the Yggterm workspace.

- Yggterm owns the left navigation through its metadata tree.
- The integrated surface does not bring the standalone sheets sidebar.
- A Yggterm Cellulose node is single-sheet. Multiple sheets belong to standalone
  Cellulose unless this scope is explicitly revisited.
- The right command/ribbon sidebar remains available in Yggterm.
- Integrated Cellulose data is stored in one or more SQLite databases under the
  selected `YGGTERM_HOME`.
- App-control should expose sheet identity, active cell/range, formula/value
  state, dirty/saved state, and storage location.

## Non-Goals

- Do not make spreadsheet data terminal/session truth.
- Do not build a general Office clone before the finance modeling core is
  coherent.
- Do not ship legal-risk keymap or branding defaults inside the project.

## First Milestone

- Create/open one single-sheet Cellulose node from the Yggterm tree.
- Edit cells, formulas, and formatting for the first Excel-compatible subset.
- Persist and restore the sheet from the Yggterm database.
- Prove the workflow with app-control state and a screenshot.

## Open Decisions

- Name the exact FMI standard corpus and examples that define the first
  compliance target.
- Decide the formula engine compatibility test suite.
- Decide whether standalone Cellulose stores workbooks as files, SQLite
  databases, or both.
