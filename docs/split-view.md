# Split view

Two (MVP) sessions sharing the viewport as one surface — side by side or stacked —
with the sidebar row acting as a miniature map of that geometry. Shipped 2.10.7.
Campaign record: `[[campaign-split-view-groups]]`.

## The single source of truth

A split is a **GUI-level** object, not a daemon one. `yggterm_core::SplitGroup`:

```rust
SplitGroup { group_id, axis: SplitAxis, ratio: f32, members: Vec<String>,
             active_pane: usize, prior_keep_alive: BTreeMap<String, bool> }
```

`ShellState::split_groups: Vec<SplitGroup>` is the SSOT. The compound sidebar row,
the viewport pane layout, keep-alive, and persistence all DERIVE from it — there is
no per-session split flag anywhere. Members keep their own daemon-owned PTYs
(possibly on different machines: cross-host split is the differentiator tmux cannot
match); only the GUI composes them.

Persisted in `AppSettings::split_groups` (settings file), restored on launch so a
built workspace reopens as the intentional artifact the user shaped. `active_pane`
is remembered-focus state — kept in sync with `active_session_path` while the group
is the active surface, genuinely distinct only while it is backgrounded.

## `active_session_path` keeps its exact meaning

The whole delicate single-active-session render machinery stays intact because
`active_session_path` still names the ONE focused pane (the input target). A group
only declares which sessions are co-visible with it. Two predicates widen to cover
the co-visible siblings; nothing else changes:

- `terminal_active_visible_for_session` — true for the focused pane OR any member of
  the active split group. This is the gate for reads, recovery, and reveal, so a
  co-visible pane reads and recovers as a FOREGROUND surface (never a background
  host — that is what would reintroduce the killed blink class). Decision c of the
  campaign: visible panes are ALL foreground for paint.
- The viewport's terminal layer (`MainSurface`) — when `active_split_group` is
  `Some`, each member is laid into a pane rect (`split_pane_rect_css`) and all are
  visible; the focused pane gets input + the focus ring, the sibling gets a hairline
  border. Non-member retained hosts stay hidden exactly as before.

The panes' `ResizeObserver` fires on the half-width/half-height rect, so the daemon
reflows each pane's grid (side-by-side ≈ half cols, stacked ≈ half rows) with no new
resize plumbing.

## The compound sidebar row

Within Live Sessions, a group's member rows collapse into ONE compound row
(`collapse_live_sessions_into_split_rows`, bounded to the live region) rendered by
`SplitGroupRow`:

- side-by-side → cells separated by `|`; stacked → two stacked lines.
- click a cell → focus that pane; hover → full title.
- an always-green traffic dot (grouping IS the keep-alive declaration).
- **NO ×.** All structural ops live behind the right-click menu (ungroup, close a
  pane, close all) so a built workspace is hard to lose by accident.

The cwd-tree member rows (dual presence) are untouched and would carry a split-glyph
badge. A group with fewer than two members present in Live Sessions does not compound
(degrades to plain rows).

## Lifecycle ops

- **Create:** multi-select two live-terminal rows → context menu *Split side by
  side* / *Split stacked*. Grouping forces keep-alive on every member and remembers
  each one's prior setting.
- **Focus a pane:** click a compound-row cell or a viewport pane. Both panes are
  already mounted, so this is a cheap reveal/focus flip, not a remount.
- **Divider:** a draggable strip re-balances the panes; the ratio is clamped to
  `[0.15, 0.85]` and persisted. JS drives the drag visually and commits the ratio
  to Rust once on release (no per-frame re-render / disk write).
- **Ungroup:** dissolves to individual rows, restoring each member's pre-group
  keep-alive. **Close a pane / close all:** closes the session(s); the group SSOT is
  reconciled by `prune_split_groups_against_live` on the next daemon snapshot (the
  "known" set is live + retained + cached-hot + active, so a transient snapshot gap
  never false-prunes a live member).

## Faithful screenshot (per-pane composite)

`eval_active_terminal_canvas_composite` composites EVERY visible terminal pane over
the `[data-yggterm-main-surface-body]` frame, so `server app screenshot` shows the
whole split, not just the focused pane. The pre-split composite grabbed only the
active host.

## The stale-atlas heal

Splitting resizes BOTH members to half at once. The pane that is not the active
render host repaints its glyphs from a stale GPU atlas and comes back garbled/
staircased (`docs/xterm-bugs.md#webgl-stale-atlas-garble`). `spawn_heal_split_panes`
issues a `redrawTerminal` (clearTextureAtlas + refit + refresh) per member over a
widening window after a create / restore / ratio change; focusing a pane always
re-renders it crisp regardless.

## Headless drive + verify

- `server app split create [--axis side-by-side|stacked] <path> <path>`
- `server app split focus <session_path>`
- `server app split ratio <group_id> <0.15..0.85>`
- `server app split ungroup <group_id>`
- `server app state` → `data.split_view` = `{active_group_id, groups[]}`.

## Out of scope (MVP)

2×2 via drop-onto-cell, nesting beyond a single split, daemon-side layout, and any
JSONL/agent-handoff changes are all deliberately excluded (see the campaign's
Non-goals).
