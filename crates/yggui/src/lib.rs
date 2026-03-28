pub mod chrome;
pub mod drag_tree;
pub mod drag_visuals;
pub mod notifications;
pub mod rails;
mod shell;
pub mod theme;
mod window_icon;

pub use chrome::{
    ChromeControlIcon, ChromePalette, HoveredChromeControl, TitlebarChrome, WindowControlsStrip,
    search_input_style,
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
pub use shell::{PendingUpdateRestart, ShellBootstrap, initial_server_sync, launch_shell};
pub use theme::{
    MAX_THEME_STOPS, THEME_EDITOR_SWATCHES, append_theme_stop, clamp_theme_spec,
    default_theme_editor_spec, dominant_accent, gradient_css, preview_surface_css, shell_tint,
};
pub use window_icon::{load_window_icon_from_png, load_yggterm_window_icon};
