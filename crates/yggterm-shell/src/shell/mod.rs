// crates/yggterm-shell/src/shell/mod.rs
// Modularized shell components and state engine.

#[cfg(test)]
pub(crate) const SHELL_SOURCE: &str = concat!(
    include_str!("state.rs"),
    "\n",
    include_str!("launch.rs"),
    "\n",
    include_str!("titlebar.rs"),
    "\n",
    include_str!("sidebar.rs"),
    "\n",
    include_str!("viewport.rs"),
    "\n",
    include_str!("terminal_scripts.rs"),
    "\n",
    include_str!("startpage.rs"),
    "\n",
    include_str!("right_rail.rs"),
    "\n",
    include_str!("overlays.rs")
);

include!("state.rs");
include!("launch.rs");
include!("titlebar.rs");
include!("sidebar.rs");
include!("viewport.rs");
include!("terminal_scripts.rs");
include!("startpage.rs");
include!("right_rail.rs");
include!("overlays.rs");

#[cfg(test)]
include!("tests.rs");

#[cfg(test)]
include!("key_plane_tests.rs");
