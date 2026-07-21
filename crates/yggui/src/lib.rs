#![recursion_limit = "256"]

pub mod chrome;
pub mod drag_tree;
pub mod drag_visuals;
pub mod motion;
pub mod notifications;
pub mod rails;
pub mod theme;

pub use chrome::{
    ChromeControlIcon, ChromePalette, HoveredChromeControl, TitlebarChrome, WindowControlsStrip,
    search_field_shell_style, search_input_style,
};
pub use drag_tree::{
    DragDropPlacement, DragDropTarget, TreeDropPlacement, TreeReorderItem, TreeReorderPlanItem,
    build_tree_reorder_plan, canonical_tree_leaf_name, join_tree_child_path,
    ordered_tree_child_path, resolve_drag_drop_target, resolve_tree_drop_placement,
    staging_tree_child_path, tree_parent_path, tree_path_contains, valid_drop_target,
};
pub use drag_visuals::{DragGhostCard, DragGhostPalette, TreeDropZones};
pub use motion::{
    MOTION_EMPHASIZED_DECELERATE, MOTION_ENTER_DURATION_MS, emphasized_enter_transition,
    emphasized_exit_transition, standard_accelerate_transition, standard_decelerate_transition,
    standard_transition, transition,
};
pub use notifications::{TOAST_CSS, ToastCard, ToastItem, ToastPalette, ToastTone, ToastViewport};
pub use rails::{RailHeader, RailScrollBody, RailSectionTitle, SideRailReveal, SideRailShell};
pub use theme::{
    MAX_THEME_STOPS, THEME_EDITOR_SWATCHES, append_theme_stop, chrome_material_tint,
    clamp_theme_spec, default_theme_editor_spec, dominant_accent, gradient_background_repeat_css,
    gradient_background_size_css, gradient_css, hex_to_rgb, live_blur_gradient_css,
    material_blur_radius_px, preview_surface_css, shell_tint,
};
