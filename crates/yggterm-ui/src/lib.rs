pub mod chrome;
pub mod drag_tree;
pub mod drag_visuals;
pub mod notifications;
pub mod rails;
mod shell;
mod window_icon;

pub use chrome::{
    ChromeControlIcon, ChromePalette, HoveredChromeControl, TitlebarChrome,
    WindowControlsStrip, search_input_style,
};
pub use drag_tree::{
    DragDropPlacement, DragDropTarget, TreeDropPlacement, TreeReorderItem, TreeReorderPlanItem,
    build_tree_reorder_plan, canonical_tree_leaf_name, join_tree_child_path,
    ordered_tree_child_path, resolve_drag_drop_target, resolve_tree_drop_placement,
    staging_tree_child_path, tree_parent_path, tree_path_contains, valid_drop_target,
};
pub use drag_visuals::{DragGhostCard, DragGhostPalette, TreeDropZones};
pub use notifications::{TOAST_CSS, ToastCard, ToastItem, ToastPalette, ToastTone, ToastViewport};
pub use rails::{RailHeader, RailScrollBody, RailSectionTitle, SideRailShell};
pub use shell::{PendingUpdateRestart, ShellBootstrap, launch_shell};
