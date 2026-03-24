# yggui

`yggui` contains Dioxus shell primitives and reusable interaction logic for Yggdrasil applications.

## Reusable drag tree

The sidebar drag-and-drop engine is intentionally reusable and lives in:

- [src/drag_tree.rs](/home/pi/gh/yggterm/crates/yggui/src/drag_tree.rs)

Use it when another app needs Yggterm-style tree reordering.

### What it provides

- explicit drop zones: `before`, `inside`, `after`
- path-based target resolution
- stable sibling reorder planning
- two-phase `temp -> final` rewrite planning for persistence layers
- regression-tested handling for adjacent snap boundaries and dragged-anchor cases

## Reusable drag visuals

The drag UI primitives live in:

- [src/drag_visuals.rs](/home/pi/gh/yggterm/crates/yggui/src/drag_visuals.rs)

They provide:

- `DragGhostCard` for the floating drag card
- `TreeDropZones` for before/inside/after hover strips

The current sidebar in `src/shell.rs` is the reference integration for both the engine and the visuals.

## Reusable chrome

The reusable titlebar and window-control primitives live in:

- [src/chrome.rs](/home/pi/gh/yggterm/crates/yggui/src/chrome.rs)

They provide:

- `TitlebarChrome`
- `WindowControlsStrip`
- `search_input_style`

## Reusable notifications

The toast system lives in:

- [src/notifications.rs](/home/pi/gh/yggterm/crates/yggui/src/notifications.rs)

It provides:

- `ToastViewport`
- `ToastCard`
- reusable `ToastTone` and `ToastPalette`
- shared `TOAST_CSS`

These primitives are also the base layer for update-ready and background-job feedback in shell chrome.

## Reusable rails

The side-rail shell primitives live in:

- [src/rails.rs](/home/pi/gh/yggterm/crates/yggui/src/rails.rs)

They provide:

- `SideRailShell`
- `RailHeader`
- `RailScrollBody`
- `RailSectionTitle`

## Reusable update UX

Yggdrasil shells should treat update state as a reusable shell primitive.

- background install can surface a lightweight chrome affordance
- restart-required state should be explicit and user-controlled
- update-ready notifications should reuse the toast system instead of inventing one-off dialogs

### Integration steps

1. Adapt your tree rows into `TreeReorderItem<K>`.
2. Feed hover state into `resolve_drag_drop_target(...)`.
3. Convert the result into `TreeDropPlacement`.
4. Build a plan with `build_tree_reorder_plan(...)`.
5. Apply the returned `from_path`, `temp_path`, and `final_path` to your own store.

### Notes

- The engine is path-oriented because Yggterm persists virtual paths instead of raw list indices.
- The UI layer is free to style drag ghosts, make-way gaps, or stacked-card affordances independently.
- The current Yggterm sidebar in `src/shell.rs` is the production reference integration.
