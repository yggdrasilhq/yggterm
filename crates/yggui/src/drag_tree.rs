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

/// Reorder a FLAT list: move `moved` to sit before/after `target`, returning the
/// new order, or `None` when the drop is a no-op or names an id the list does
/// not hold.
///
/// The tree engine above is path-and-parent shaped because the cwd tree nests.
/// A contributed app rail does not: it is one flat sequence of row ids the app
/// owns, and the whole reorder is "take this out, put it back there". Keeping
/// that here rather than in the GUI means the meaning of a drop is unit-tested
/// without a webview, and both the tree and the rail speak one
/// [`DragDropPlacement`] vocabulary.
///
/// `Into` has no meaning in a flat list — nothing can be dropped *inside* a row
/// — so it is treated as `After`, which is what the pointer bands either side of
/// a row's midpoint already produce.
pub fn reorder_flat_list(
    order: &[String],
    moved: &str,
    target: &str,
    placement: DragDropPlacement,
) -> Option<Vec<String>> {
    if moved == target || !order.iter().any(|id| id == moved) {
        return None;
    }
    let mut remaining: Vec<String> = order.iter().filter(|id| *id != moved).cloned().collect();
    let target_index = remaining.iter().position(|id| id == target)?;
    let insert_at = match placement {
        DragDropPlacement::Before => target_index,
        DragDropPlacement::Into | DragDropPlacement::After => target_index + 1,
    };
    remaining.insert(insert_at.min(remaining.len()), moved.to_string());
    (remaining != order).then_some(remaining)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
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
        let order = ids(&["a", "b", "c", "d"]);
        assert_eq!(
            reorder_flat_list(&order, "d", "b", DragDropPlacement::Before),
            Some(ids(&["a", "d", "b", "c"]))
        );
        assert_eq!(
            reorder_flat_list(&order, "a", "c", DragDropPlacement::After),
            Some(ids(&["b", "c", "a", "d"]))
        );
    }

    // A flat list has no inside; the row bands either side of the midpoint are
    // what the pointer actually produces, and `Into` must not silently drop the
    // row on the floor.
    #[test]
    fn flat_reorder_treats_into_as_after() {
        let order = ids(&["a", "b", "c"]);
        assert_eq!(
            reorder_flat_list(&order, "a", "b", DragDropPlacement::Into),
            reorder_flat_list(&order, "a", "b", DragDropPlacement::After)
        );
    }

    // No-ops must be `None`, not `Some(unchanged)`: the caller uses that to
    // decide whether to POST at all, and a POST per settled drag would make the
    // app rewrite its store on every mouse-up.
    #[test]
    fn flat_reorder_reports_no_op_drops_as_none() {
        let order = ids(&["a", "b", "c"]);
        assert_eq!(
            reorder_flat_list(&order, "b", "b", DragDropPlacement::Before),
            None,
            "a row dropped on itself changes nothing"
        );
        assert_eq!(
            reorder_flat_list(&order, "a", "b", DragDropPlacement::Before),
            None,
            "dropping before the row that already follows it changes nothing"
        );
        assert_eq!(
            reorder_flat_list(&order, "c", "b", DragDropPlacement::After),
            None,
            "dropping after the row that already precedes it changes nothing"
        );
    }

    #[test]
    fn flat_reorder_rejects_ids_the_list_does_not_hold() {
        let order = ids(&["a", "b"]);
        assert_eq!(
            reorder_flat_list(&order, "zz", "a", DragDropPlacement::Before),
            None
        );
        assert_eq!(
            reorder_flat_list(&order, "a", "zz", DragDropPlacement::Before),
            None
        );
    }

    #[test]
    fn flat_reorder_can_move_a_row_to_either_end() {
        let order = ids(&["a", "b", "c"]);
        assert_eq!(
            reorder_flat_list(&order, "c", "a", DragDropPlacement::Before),
            Some(ids(&["c", "a", "b"]))
        );
        assert_eq!(
            reorder_flat_list(&order, "a", "c", DragDropPlacement::After),
            Some(ids(&["b", "c", "a"]))
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
