//! Reusable drag-and-drop tree reorder engine for Yggterm-style sidebars.
//!
//! This module is intentionally UI-framework-light: it models drop zones, target
//! resolution, and stable sibling reordering using metadata that any tree view can
//! provide.
//!
//! Integration pattern:
//!
//! 1. Adapt your tree rows into [`TreeReorderItem`] values.
//! 2. Feed pointer hover state into [`resolve_drag_drop_target`].
//! 3. Convert the resulting [`DragDropTarget`] into a [`TreeDropPlacement`].
//! 4. Build a reorder plan with [`build_tree_reorder_plan`].
//! 5. Apply the returned `from -> temp -> final` paths in your own store.
//!
//! The module is path-based on purpose because Yggterm's tree model is metadata-first
//! and persists virtual paths instead of in-memory list positions.

use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DragDropPlacement {
    Before,
    Into,
    After,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DragDropTarget {
    pub path: String,
    pub placement: DragDropPlacement,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeDropPlacement {
    TopOfGroup(String),
    AfterPath(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeReorderItem<K> {
    pub kind: K,
    pub path: String,
    pub parent_path: Option<String>,
    pub accepts_drop_inside: bool,
    pub droppable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeReorderPlanItem<K> {
    pub kind: K,
    pub from_path: String,
    pub temp_path: String,
    pub final_path: String,
}

pub fn tree_leaf_name(path: &str) -> Option<String> {
    path.rsplit('/')
        .find(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
}

pub fn canonical_tree_leaf_name(path: &str) -> String {
    let leaf = tree_leaf_name(path).unwrap_or_else(|| "item".to_string());
    let unanchored = leaf.rsplit('~').next().unwrap_or(leaf.as_str());
    let mut stripped = unanchored.trim_start_matches('!');
    while stripped.len() > 5
        && stripped.as_bytes().get(4) == Some(&b'-')
        && stripped.as_bytes()[0..4]
            .iter()
            .all(|byte| byte.is_ascii_digit())
    {
        stripped = &stripped[5..];
    }
    if stripped.is_empty() {
        "item".to_string()
    } else {
        stripped.to_string()
    }
}

pub fn join_tree_child_path(base: &str, leaf: &str) -> String {
    let trimmed = base.trim_end_matches('/');
    if trimmed.is_empty() || trimmed == "/" {
        format!("/{leaf}")
    } else {
        format!("{trimmed}/{leaf}")
    }
}

pub fn tree_parent_path(path: &str) -> Option<String> {
    let normalized = path.trim_end_matches('/');
    if normalized.is_empty() || normalized == "/" {
        return None;
    }
    let parent = normalized.rsplit_once('/')?.0;
    if parent.is_empty() {
        Some("/".to_string())
    } else {
        Some(parent.to_string())
    }
}

pub fn tree_path_contains(parent: &str, child: &str) -> bool {
    child == parent
        || child
            .strip_prefix(parent)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

pub fn valid_drop_target<K>(drag_paths: &[String], target_row: &TreeReorderItem<K>) -> bool {
    if !target_row.droppable || drag_paths.is_empty() {
        return false;
    }
    drag_paths.iter().all(|path| {
        path != &target_row.path
            && !tree_path_contains(path, &target_row.path)
            && tree_parent_path(path).is_some()
    })
}

pub fn resolve_drag_drop_target<K>(
    items: &[TreeReorderItem<K>],
    drag_paths: &[String],
    row: &TreeReorderItem<K>,
    placement: DragDropPlacement,
) -> Option<DragDropTarget> {
    if !valid_drop_target(drag_paths, row) {
        return None;
    }
    let target = DragDropTarget {
        path: row.path.clone(),
        placement,
    };
    resolve_tree_drop_placement(items, &target).map(|_| target)
}

pub fn resolve_tree_drop_placement<K>(
    items: &[TreeReorderItem<K>],
    target: &DragDropTarget,
) -> Option<TreeDropPlacement> {
    let target_index = items.iter().position(|row| row.path == target.path)?;
    let target_row = items.get(target_index)?;
    match target.placement {
        DragDropPlacement::Into => {
            if target_row.accepts_drop_inside {
                Some(TreeDropPlacement::TopOfGroup(target_row.path.clone()))
            } else {
                Some(TreeDropPlacement::AfterPath(target_row.path.clone()))
            }
        }
        DragDropPlacement::After => Some(TreeDropPlacement::AfterPath(target_row.path.clone())),
        DragDropPlacement::Before => {
            let parent = target_row.parent_path.clone()?;
            let previous_sibling = items[..target_index]
                .iter()
                .rev()
                .find(|candidate| candidate.parent_path.as_deref() == Some(parent.as_str()));
            if let Some(previous) = previous_sibling {
                Some(TreeDropPlacement::AfterPath(previous.path.clone()))
            } else {
                Some(TreeDropPlacement::TopOfGroup(parent))
            }
        }
    }
}

pub fn ordered_tree_child_path(parent: &str, path: &str, index: usize) -> String {
    let leaf = canonical_tree_leaf_name(path);
    join_tree_child_path(parent, &format!("{index:04}-{leaf}"))
}

pub fn staging_tree_child_path(parent: &str, path: &str, token: &str, index: usize) -> String {
    let leaf = canonical_tree_leaf_name(path);
    join_tree_child_path(parent, &format!("__yggtmp-{token}-{index:04}-{leaf}"))
}

pub fn build_tree_reorder_plan<K: Clone>(
    items: &[TreeReorderItem<K>],
    selected_items: &[TreeReorderItem<K>],
    placement: &TreeDropPlacement,
    temp_token: &str,
) -> Option<Vec<TreeReorderPlanItem<K>>> {
    if selected_items.is_empty() {
        return Some(Vec::new());
    }
    let moved_set = selected_items
        .iter()
        .map(|row| row.path.clone())
        .collect::<HashSet<_>>();
    let target_parent = match placement {
        TreeDropPlacement::TopOfGroup(path) => path.clone(),
        TreeDropPlacement::AfterPath(path) => tree_parent_path(path)?,
    };

    let mut siblings_by_parent = BTreeMap::<String, Vec<TreeReorderItem<K>>>::new();
    for row in items.iter() {
        if let Some(parent) = row.parent_path.clone() {
            siblings_by_parent
                .entry(parent)
                .or_default()
                .push(row.clone());
        }
    }

    let original_target_siblings = siblings_by_parent
        .get(&target_parent)
        .cloned()
        .unwrap_or_default();

    for siblings in siblings_by_parent.values_mut() {
        siblings.retain(|row| !moved_set.contains(&row.path));
    }

    let moved_rows = selected_items.to_vec();
    let target_siblings = siblings_by_parent.entry(target_parent.clone()).or_default();
    let insert_at = match placement {
        TreeDropPlacement::TopOfGroup(_) => 0,
        TreeDropPlacement::AfterPath(anchor) => {
            original_target_siblings
                .iter()
                .take_while(|row| row.path != *anchor)
                .filter(|row| !moved_set.contains(&row.path))
                .count()
                + usize::from(!moved_set.contains(anchor))
        }
    };
    for (offset, row) in moved_rows.iter().cloned().enumerate() {
        target_siblings.insert(insert_at + offset, row);
    }

    let mut affected_parents = selected_items
        .iter()
        .filter_map(|row| row.parent_path.clone())
        .collect::<HashSet<_>>();
    affected_parents.insert(target_parent);

    let mut plan = Vec::new();
    let mut temp_index = 0usize;

    for parent in affected_parents {
        let Some(siblings) = siblings_by_parent.get(&parent) else {
            continue;
        };
        for (index, row) in siblings.iter().enumerate() {
            let final_path = ordered_tree_child_path(&parent, &row.path, index);
            if final_path == row.path {
                continue;
            }
            let original_parent = row.parent_path.clone().unwrap_or_else(|| parent.clone());
            let temp_path =
                staging_tree_child_path(&original_parent, &row.path, temp_token, temp_index);
            temp_index += 1;
            plan.push(TreeReorderPlanItem {
                kind: row.kind.clone(),
                from_path: row.path.clone(),
                temp_path,
                final_path,
            });
        }
    }

    Some(plan)
}

/// How far the pointer must travel from the press before a press becomes a
/// DRAG. Under it the gesture is still a click.
///
/// ONE number for every draggable surface. The cwd tree waited 6px while the
/// contributed rail began dragging on contact, so every click on a rail row
/// dimmed it and armed a drop target — the same gesture behaving two ways in
/// one window, and the reason an ordinary click could commit a reorder.
pub const DRAG_BEGIN_THRESHOLD_PX: f64 = 6.0;

/// Has the pointer travelled far enough from the press for this to be a drag?
pub fn drag_threshold_reached(origin: (f64, f64), pointer: (f64, f64)) -> bool {
    let dx = pointer.0 - origin.0;
    let dy = pointer.1 - origin.1;
    dx.hypot(dy) >= DRAG_BEGIN_THRESHOLD_PX
}

/// One row of a rendered ROW LIST, as the reorder engine sees it.
///
/// The list is given in DRAW ORDER — the tree already flattened, parents before
/// their children — and a row names its parent rather than owning a nested
/// `children` array. That is deliberate and it is the SINGLE tree model every
/// row surface in the product now shares: the sidebar's contributed panes
/// declare a flat `Vec` of widgets in draw order, and the WebTabs rail draws a
/// flat `Vec` of rows; a nested schema would have to be flattened by every
/// renderer before it could be drawn, which is a second encoding of the same
/// shape and exactly what this codebase forbids.
///
/// Rows hidden inside a COLLAPSED group still belong here. Only visible rows
/// can be dropped ON, but every row has to be in the model or a reorder would
/// silently drop the ones the user cannot currently see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowTreeRow {
    pub id: String,
    /// The group this row is filed in; `None` = the list's own root band.
    pub parent: Option<String>,
    /// This row is a GROUP: something dropped INTO it becomes its child.
    /// A row that is not a group has no inside, and `Into` degrades to `After`
    /// — the rule a flat list has always followed.
    pub group: bool,
}

impl RowTreeRow {
    /// A leaf at the root — the whole of what a FLAT list's row is.
    pub fn leaf(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            parent: None,
            group: false,
        }
    }
}

/// What a drop MEANT: the moved row's new parent, and the list's whole new
/// draw order. Both, because a drop can re-parent, re-order, or do both at
/// once, and a caller that only learned one of the two would have to
/// reconstruct the other — a second answer to the same question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowTreeDrop {
    pub parent: Option<String>,
    pub order: Vec<String>,
}

/// Move `moved` before/into/after `target` in a hierarchical row list,
/// returning the new parent and the whole new draw order — or `None` when the
/// drop is a no-op, names an id the list does not hold, or would file a group
/// inside its own subtree.
///
/// THE one ordering algorithm for id-keyed row lists. A FLAT list is its
/// degenerate case — every row a root leaf — and has no entry point of its own,
/// so a flat rail and a nested one cannot disagree about what dropping a row on
/// another row means.
///
/// The rules, in the same vocabulary [`resolve_tree_drop_placement`] uses for
/// the path-keyed cwd tree:
///
/// - `Into` a GROUP files the row at the TOP of that group (`TopOfGroup`).
/// - `Into` a non-group has no inside, so it is `After`.
/// - `After` puts the row immediately after the target AND everything nested
///   under it, so dropping below a folder lands beside the folder, not in it.
/// - `Before` puts the row immediately above the target, among its siblings.
pub fn reorder_row_tree(
    rows: &[RowTreeRow],
    moved: &str,
    target: &str,
    placement: DragDropPlacement,
) -> Option<RowTreeDrop> {
    if moved == target {
        return None;
    }
    let moved_row = rows.iter().find(|row| row.id == moved)?;
    let target_row = rows.iter().find(|row| row.id == target)?;
    // A group cannot be filed inside itself. Without this a folder dragged onto
    // one of its own tabs would become its own descendant and vanish from the
    // flatten, taking every row under it with it.
    if row_tree_descends_from(rows, target, moved) {
        return None;
    }

    // Children per parent, in draw order. `None` (the root band) is keyed by
    // the empty string: an id is never empty, so the two can never collide.
    let mut children: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        children
            .entry(row.parent.clone().unwrap_or_default())
            .or_default()
            .push(row.id.clone());
    }

    let (new_parent, anchor) = match placement {
        DragDropPlacement::Into if target_row.group => (Some(target_row.id.clone()), None),
        DragDropPlacement::Into | DragDropPlacement::After => (
            target_row.parent.clone(),
            Some((target_row.id.clone(), true)),
        ),
        DragDropPlacement::Before => (
            target_row.parent.clone(),
            Some((target_row.id.clone(), false)),
        ),
    };

    let old_parent_key = moved_row.parent.clone().unwrap_or_default();
    if let Some(siblings) = children.get_mut(&old_parent_key) {
        siblings.retain(|id| id != moved);
    }
    let new_parent_key = new_parent.clone().unwrap_or_default();
    let siblings = children.entry(new_parent_key).or_default();
    let insert_at = match &anchor {
        // Top of the group: the same landing `TreeDropPlacement::TopOfGroup`
        // gives the path-keyed tree.
        None => 0,
        Some((anchor_id, after)) => match siblings.iter().position(|id| id == anchor_id) {
            Some(index) => index + usize::from(*after),
            // The anchor is the moved row's own former slot — it was just
            // lifted out — so the row lands where it already was.
            None => return None,
        },
    };
    siblings.insert(insert_at.min(siblings.len()), moved.to_string());

    let order = flatten_row_tree(&children);
    // A drop that changes nothing must be `None`, never `Some(unchanged)`: the
    // caller decides from that whether to write anything at all, and a write
    // per settled drag would rewrite the store on every mouse-up.
    let unchanged_order = order.len() == rows.len()
        && order
            .iter()
            .zip(rows.iter())
            .all(|(id, row)| id == &row.id);
    if unchanged_order && new_parent == moved_row.parent {
        return None;
    }
    Some(RowTreeDrop {
        parent: new_parent,
        order,
    })
}

/// Does `id` sit anywhere under `ancestor`? Bounded by the row count so a
/// malformed parent cycle terminates instead of hanging the render thread.
fn row_tree_descends_from(rows: &[RowTreeRow], id: &str, ancestor: &str) -> bool {
    let mut cursor = Some(id.to_string());
    for _ in 0..=rows.len() {
        let Some(current) = cursor else {
            return false;
        };
        if current == ancestor {
            return true;
        }
        cursor = rows
            .iter()
            .find(|row| row.id == current)
            .and_then(|row| row.parent.clone());
    }
    false
}

/// Depth-first flatten of a `parent -> children` map back into draw order.
fn flatten_row_tree(children: &BTreeMap<String, Vec<String>>) -> Vec<String> {
    let mut order = Vec::new();
    let mut stack: Vec<String> = children
        .get("")
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .rev()
        .collect();
    let mut guard = 0usize;
    let bound = children.values().map(Vec::len).sum::<usize>();
    while let Some(id) = stack.pop() {
        guard += 1;
        if guard > bound {
            break;
        }
        order.push(id.clone());
        if let Some(kids) = children.get(&id) {
            for kid in kids.iter().rev() {
                stack.push(kid.clone());
            }
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    /// A FLAT list, through the ONE engine: every row a root leaf. There is no
    /// separate flat entry point — a flat list IS a tree with no groups, and a
    /// second function for it is how two orderings come to disagree. These
    /// tests are therefore tree tests as much as list tests.
    fn reorder_flat(
        order: &[&str],
        moved: &str,
        target: &str,
        placement: DragDropPlacement,
    ) -> Option<Vec<String>> {
        let rows: Vec<RowTreeRow> = order.iter().map(|id| RowTreeRow::leaf(*id)).collect();
        reorder_row_tree(&rows, moved, target, placement).map(|drop| drop.order)
    }

    // A press is not a drag. The rail used to begin one on contact while the
    // cwd tree waited, so every click on a note row dimmed it and armed a drop
    // target — and the release then committed a reorder nobody gestured.
    #[test]
    fn a_press_becomes_a_drag_only_once_it_travels() {
        let origin = (100.0, 100.0);
        assert!(
            !drag_threshold_reached(origin, origin),
            "a press is not a drag"
        );
        assert!(
            !drag_threshold_reached(origin, (103.0, 103.0)),
            "4.24px of hand-shake is still a click"
        );
        // The boundary is inclusive, and it is measured as DISTANCE — not per
        // axis, or a diagonal press would need 8.5px while a straight one needs
        // 6px.
        assert!(!drag_threshold_reached(origin, (105.9, 100.0)));
        assert!(drag_threshold_reached(
            origin,
            (100.0 + DRAG_BEGIN_THRESHOLD_PX, 100.0)
        ));
        assert!(drag_threshold_reached(
            origin,
            (100.0, 100.0 - DRAG_BEGIN_THRESHOLD_PX)
        ));
        assert!(
            drag_threshold_reached(origin, (104.3, 104.3)),
            "6.08px diagonally is a drag"
        );
    }

    #[test]
    fn flat_reorder_moves_a_row_before_and_after_its_target() {
        let order = &["a", "b", "c", "d"];
        assert_eq!(
            reorder_flat(&order[..], "d", "b", DragDropPlacement::Before),
            Some(ids(&["a", "d", "b", "c"]))
        );
        assert_eq!(
            reorder_flat(&order[..], "a", "c", DragDropPlacement::After),
            Some(ids(&["b", "c", "a", "d"]))
        );
    }

    // A flat list has no inside; the row bands either side of the midpoint are
    // what the pointer actually produces, and `Into` must not silently drop the
    // row on the floor.
    #[test]
    fn flat_reorder_treats_into_as_after() {
        let order = &["a", "b", "c"];
        assert_eq!(
            reorder_flat(&order[..], "a", "b", DragDropPlacement::Into),
            reorder_flat(&order[..], "a", "b", DragDropPlacement::After)
        );
    }

    // No-ops must be `None`, not `Some(unchanged)`: the caller uses that to
    // decide whether to POST at all, and a POST per settled drag would make the
    // app rewrite its store on every mouse-up.
    #[test]
    fn flat_reorder_reports_no_op_drops_as_none() {
        let order = &["a", "b", "c"];
        assert_eq!(
            reorder_flat(&order[..], "b", "b", DragDropPlacement::Before),
            None,
            "a row dropped on itself changes nothing"
        );
        assert_eq!(
            reorder_flat(&order[..], "a", "b", DragDropPlacement::Before),
            None,
            "dropping before the row that already follows it changes nothing"
        );
        assert_eq!(
            reorder_flat(&order[..], "c", "b", DragDropPlacement::After),
            None,
            "dropping after the row that already precedes it changes nothing"
        );
    }

    #[test]
    fn flat_reorder_rejects_ids_the_list_does_not_hold() {
        let order = &["a", "b"];
        assert_eq!(
            reorder_flat(&order[..], "zz", "a", DragDropPlacement::Before),
            None
        );
        assert_eq!(
            reorder_flat(&order[..], "a", "zz", DragDropPlacement::Before),
            None
        );
    }

    #[test]
    fn flat_reorder_can_move_a_row_to_either_end() {
        let order = &["a", "b", "c"];
        assert_eq!(
            reorder_flat(&order[..], "c", "a", DragDropPlacement::Before),
            Some(ids(&["c", "a", "b"]))
        );
        assert_eq!(
            reorder_flat(&order[..], "a", "c", DragDropPlacement::After),
            Some(ids(&["b", "c", "a"]))
        );
    }

    // ===================================================================
    // The ONE row-list ordering engine. Everything above this line already
    // exercises it through `reorder_flat`, so the flat locks are tree locks.
    // ===================================================================

    fn group(id: &str, parent: Option<&str>) -> RowTreeRow {
        RowTreeRow {
            id: id.to_string(),
            parent: parent.map(str::to_string),
            group: true,
        }
    }

    fn leaf(id: &str, parent: Option<&str>) -> RowTreeRow {
        RowTreeRow {
            id: id.to_string(),
            parent: parent.map(str::to_string),
            group: false,
        }
    }

    /// Folders above tabs, one folder holding two tabs, two loose tabs after.
    /// The WebTabs rail's exact shape.
    fn rail() -> Vec<RowTreeRow> {
        vec![
            group("f1", None),
            leaf("t1", Some("f1")),
            leaf("t2", Some("f1")),
            group("f2", None),
            leaf("t3", None),
            leaf("t4", None),
        ]
    }

    #[test]
    fn a_row_dropped_into_a_group_is_filed_at_its_top() {
        let drop = reorder_row_tree(&rail(), "t4", "f1", DragDropPlacement::Into).expect("drop");
        assert_eq!(drop.parent.as_deref(), Some("f1"));
        assert_eq!(drop.order, ids(&["f1", "t4", "t1", "t2", "f2", "t3"]));
    }

    // The defect this whole engine exists to fix: the rail's hand-rolled drag
    // could only ever RE-PARENT. A drop must be able to change the row's slot
    // among its siblings without changing its parent at all.
    #[test]
    fn a_drop_reorders_siblings_without_re_parenting() {
        let drop = reorder_row_tree(&rail(), "t1", "t2", DragDropPlacement::After).expect("drop");
        assert_eq!(
            drop.parent.as_deref(),
            Some("f1"),
            "reordering inside a folder must not unfile the row"
        );
        assert_eq!(drop.order, ids(&["f1", "t2", "t1", "f2", "t3", "t4"]));
    }

    // …and the other half: a filed row dropped beside a loose one comes OUT of
    // its folder. Re-parent and re-order are one gesture, not two features.
    #[test]
    fn a_filed_row_dropped_beside_a_root_row_returns_to_the_root() {
        let drop = reorder_row_tree(&rail(), "t1", "t3", DragDropPlacement::Before).expect("drop");
        assert_eq!(drop.parent, None);
        assert_eq!(drop.order, ids(&["f1", "t2", "f2", "t1", "t3", "t4"]));
    }

    // `After` a GROUP means beside the group, never inside it — otherwise the
    // band under a folder header would file rows the user meant to place next
    // to the folder.
    #[test]
    fn after_a_group_lands_beside_it_not_inside_it() {
        let drop = reorder_row_tree(&rail(), "t4", "f1", DragDropPlacement::After).expect("drop");
        assert_eq!(drop.parent, None);
        assert_eq!(
            drop.order,
            ids(&["f1", "t1", "t2", "t4", "f2", "t3"]),
            "the row sits after the folder's whole subtree"
        );
    }

    // A non-group has no inside. This is the rule a flat list has always
    // followed, now stated once for lists and trees alike.
    #[test]
    fn into_a_non_group_is_after_it() {
        assert_eq!(
            reorder_row_tree(&rail(), "t4", "t3", DragDropPlacement::Into),
            reorder_row_tree(&rail(), "t4", "t3", DragDropPlacement::After),
        );
    }

    // Groups reorder among themselves, carrying their contents with them.
    #[test]
    fn a_group_drags_its_whole_subtree() {
        let drop = reorder_row_tree(&rail(), "f1", "f2", DragDropPlacement::After).expect("drop");
        assert_eq!(drop.parent, None);
        assert_eq!(drop.order, ids(&["f2", "f1", "t1", "t2", "t3", "t4"]));
    }

    // A folder filed inside itself would disappear from the flatten, taking
    // every tab in it along.
    #[test]
    fn a_group_cannot_be_filed_inside_its_own_subtree() {
        assert_eq!(
            reorder_row_tree(&rail(), "f1", "t1", DragDropPlacement::Into),
            None
        );
        assert_eq!(
            reorder_row_tree(&rail(), "f1", "t2", DragDropPlacement::Before),
            None
        );
        assert_eq!(
            reorder_row_tree(&rail(), "f1", "f1", DragDropPlacement::After),
            None
        );
    }

    #[test]
    fn tree_reorder_reports_no_op_drops_as_none() {
        assert_eq!(
            reorder_row_tree(&rail(), "t1", "t2", DragDropPlacement::Before),
            None,
            "before the row that already follows it changes nothing"
        );
        assert_eq!(
            reorder_row_tree(&rail(), "t3", "t4", DragDropPlacement::Before),
            None
        );
        assert_eq!(
            reorder_row_tree(&rail(), "t1", "f1", DragDropPlacement::Into),
            None,
            "already the first child of that folder"
        );
        assert_eq!(
            reorder_row_tree(&rail(), "t9", "t1", DragDropPlacement::After),
            None,
            "an id the list does not hold"
        );
    }

    // The rail's collapsed folders hide rows that still have to survive the
    // reorder — feeding only the VISIBLE rows would silently delete them.
    #[test]
    fn rows_hidden_in_a_collapsed_group_survive_a_drop_elsewhere() {
        let drop = reorder_row_tree(&rail(), "t4", "t3", DragDropPlacement::Before).expect("drop");
        assert_eq!(drop.order, ids(&["f1", "t1", "t2", "f2", "t4", "t3"]));
        assert_eq!(
            drop.order.len(),
            rail().len(),
            "every row comes back, visible or not"
        );
    }

    fn item(path: &str, parent_path: &str) -> TreeReorderItem<&'static str> {
        TreeReorderItem {
            kind: "doc",
            path: path.to_string(),
            parent_path: Some(parent_path.to_string()),
            accepts_drop_inside: false,
            droppable: true,
        }
    }

    #[test]
    fn ordered_tree_child_path_uses_flat_index_prefix() {
        assert_eq!(
            ordered_tree_child_path("/home/user/gh/notes", "/home/user/gh/notes/paper-a", 0),
            "/home/user/gh/notes/0000-paper-a"
        );
    }

    #[test]
    fn before_target_resolves_after_previous_sibling() {
        let items = vec![
            item("/home/user/gh/notes/paper-a", "/home/user/gh/notes"),
            item("/home/user/gh/notes/paper-b", "/home/user/gh/notes"),
        ];
        let placement = resolve_tree_drop_placement(
            &items,
            &DragDropTarget {
                path: "/home/user/gh/notes/paper-b".to_string(),
                placement: DragDropPlacement::Before,
            },
        );
        assert_eq!(
            placement,
            Some(TreeDropPlacement::AfterPath(
                "/home/user/gh/notes/paper-a".to_string()
            ))
        );
    }

    #[test]
    fn reorder_plan_keeps_position_when_anchor_is_dragged_row_boundary() {
        let gg = item("/home/user/gh/notes/untitled-gg", "/home/user/gh/notes");
        let separator = TreeReorderItem {
            kind: "sep",
            path: "/home/user/gh/notes/separator-a".to_string(),
            parent_path: Some("/home/user/gh/notes".to_string()),
            accepts_drop_inside: false,
            droppable: true,
        };
        let items = vec![
            item("/home/user/gh/notes/paper-a", "/home/user/gh/notes"),
            gg.clone(),
            separator.clone(),
        ];
        let placement = resolve_tree_drop_placement(
            &items,
            &DragDropTarget {
                path: separator.path.clone(),
                placement: DragDropPlacement::Before,
            },
        )
        .expect("placement");
        let plan = build_tree_reorder_plan(&items, std::slice::from_ref(&gg), &placement, "test")
            .expect("plan");
        let gg_plan = plan
            .iter()
            .find(|item| item.from_path == gg.path)
            .expect("gg plan item");
        assert_eq!(gg_plan.final_path, "/home/user/gh/notes/0001-untitled-gg");
    }

    #[test]
    fn same_parent_folder_is_valid_into_target_for_reorder() {
        let folder = TreeReorderItem {
            kind: "group",
            path: "/home/user/gh/notes".to_string(),
            parent_path: Some("/home/user/gh".to_string()),
            accepts_drop_inside: true,
            droppable: true,
        };
        let separator = TreeReorderItem {
            kind: "sep",
            path: "/home/user/gh/notes/separator-a".to_string(),
            parent_path: Some("/home/user/gh/notes".to_string()),
            accepts_drop_inside: false,
            droppable: true,
        };

        assert!(valid_drop_target(
            std::slice::from_ref(&separator.path),
            &folder,
        ));

        let target = resolve_drag_drop_target(
            &[folder.clone(), separator.clone()],
            std::slice::from_ref(&separator.path),
            &folder,
            DragDropPlacement::Into,
        )
        .expect("target");

        assert_eq!(target.path, folder.path);
        assert_eq!(target.placement, DragDropPlacement::Into);
    }
}
