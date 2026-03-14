#[derive(Debug, Clone)]
pub struct ZedShellPlan {
    pub uses_upstream_workspace_item: bool,
    pub uses_upstream_project_panel: bool,
    pub uses_upstream_terminal_view: bool,
    pub center_viewport_replaced_by_terminals: bool,
}

impl Default for ZedShellPlan {
    fn default() -> Self {
        Self {
            uses_upstream_workspace_item: true,
            uses_upstream_project_panel: true,
            uses_upstream_terminal_view: true,
            center_viewport_replaced_by_terminals: true,
        }
    }
}

pub fn shell_plan() -> ZedShellPlan {
    ZedShellPlan::default()
}

#[cfg(feature = "zed-upstream")]
pub fn upstream_type_markers() -> [&'static str; 3] {
    [
        "workspace::Workspace",
        "project_panel::ProjectPanel",
        "terminal_view::TerminalView",
    ]
}

#[cfg(not(feature = "zed-upstream"))]
pub fn upstream_type_markers() -> [&'static str; 1] {
    ["zed-upstream feature disabled"]
}
