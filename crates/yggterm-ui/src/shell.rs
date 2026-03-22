use crate::window_icon;
use anyhow::{Result, anyhow};
use dioxus::desktop::{
    Config, LogicalSize, WindowBuilder, WindowEvent as DesktopWindowEvent, use_window,
    use_wry_event_handler, window,
};
use dioxus::document;
use dioxus::prelude::*;
use keyboard_types::{Key, Modifiers};
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use tao::event::Event as TaoEvent;
use tao::window::ResizeDirection;
use tokio::task;
use tokio::time::sleep;
use tracing::{info, warn};
use yggterm_core::{
    AgentSessionProfile, AppSettings, BrowserRow, BrowserRowKind, InstallContext,
    SessionBrowserState, SessionNode, SessionStore, UiTheme, WorkspaceDocumentInput,
    WorkspaceDocumentKind, WorkspaceGroupKind, check_for_update, save_settings_file,
    update_command_hint,
};
use yggterm_platform::DockRect;
use yggterm_server::{
    GhosttyTerminalHostMode, ManagedSessionView, PreviewTone, ServerEndpoint, ServerRuntimeStatus,
    ServerUiSnapshot, SessionKind, SessionMetadataEntry, SessionPreviewBlock, SshConnectTarget,
    TerminalBackend, WorkspaceViewMode, YggtermServer, connect_ssh_custom, focus_live,
    open_stored_session, ping, request_terminal_launch, set_all_preview_blocks_folded,
    set_view_mode as daemon_set_view_mode, snapshot as daemon_snapshot, start_command_session,
    start_local_session, start_local_session_at, status, switch_agent_session_mode,
    terminal_ensure, terminal_read, terminal_resize, terminal_write,
    toggle_preview_block as daemon_toggle_preview_block,
};

static BOOTSTRAP: OnceCell<ShellBootstrap> = OnceCell::new();
const SIDE_RAIL_WIDTH: usize = 292;
const EDGE_RESIZE_HANDLE: usize = 5;
const CORNER_RESIZE_HANDLE: usize = 10;
const XTERM_CSS: &str = include_str!("../../../assets/xterm/xterm.css");
const XTERM_JS: &str = include_str!("../../../assets/xterm/xterm.js");
const XTERM_FIT_JS: &str = include_str!("../../../assets/xterm/addon-fit.js");
static XTERM_ASSETS_BOOTSTRAPPED: OnceCell<()> = OnceCell::new();
const TOAST_CSS: &str = r#"
@keyframes yggterm-toast-fade {
  0% { opacity: 0; transform: translateY(-4px); }
  8% { opacity: 1; transform: translateY(0); }
  78% { opacity: 1; transform: translateY(0); }
  100% { opacity: 0; transform: translateY(-6px); }
}
"#;

#[derive(Debug, Clone)]
pub struct ShellBootstrap {
    pub tree: SessionNode,
    pub browser_tree: SessionNode,
    pub settings: AppSettings,
    pub install_context: InstallContext,
    pub settings_path: PathBuf,
    pub server_endpoint: ServerEndpoint,
    pub initial_server_snapshot: Option<ServerUiSnapshot>,
    pub theme: UiTheme,
    pub ghostty_bridge_enabled: bool,
    pub ghostty_embedded_surface_supported: bool,
    pub ghostty_bridge_detail: String,
    pub server_daemon_detail: String,
    pub prefer_ghostty_backend: bool,
    pub pending_update_restart: Option<PendingUpdateRestart>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingUpdateRestart {
    pub version: String,
    pub executable: PathBuf,
}

#[derive(Clone)]
struct ShellState {
    bootstrap: ShellBootstrap,
    browser: SessionBrowserState,
    server: YggtermServer,
    settings: AppSettings,
    search_query: String,
    sidebar_open: bool,
    right_panel_mode: RightPanelMode,
    last_action: String,
    maximized: bool,
    always_on_top: bool,
    notifications: Vec<ToastNotification>,
    next_notification_id: u64,
    selected_tree_paths: HashSet<String>,
    selection_anchor: Option<String>,
    context_menu_row: Option<BrowserRow>,
    context_menu_position: Option<(f64, f64)>,
    preview_layout: PreviewLayoutMode,
    server_busy: bool,
    server_daemon_detail: String,
    needs_initial_server_sync: bool,
    ssh_connect_target: String,
    ssh_connect_prefix: String,
    pending_update_restart: Option<PendingUpdateRestart>,
    docked_window_id: Option<String>,
    dock_sync_in_flight: bool,
    last_dock_signature: Option<DockSignature>,
    last_terminal_debug: String,
    last_tree_debug: String,
    generated_precis: BTreeMap<String, String>,
    precis_requests_in_flight: HashSet<String>,
    drag_paths: Vec<String>,
    drag_hover_target: Option<String>,
    pending_delete: Option<PendingDeleteDialog>,
    tree_rename_path: Option<String>,
    tree_rename_value: String,
}

#[derive(Clone, PartialEq, Eq)]
struct DockSignature {
    pid: Option<u32>,
    host_token: Option<String>,
    rect: DockRect,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RightPanelMode {
    Hidden,
    Metadata,
    Settings,
    Connect,
    Notifications,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NotificationTone {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PreviewLayoutMode {
    Chat,
    Graph,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum NotificationDeliveryMode {
    InApp,
    Both,
    System,
}

#[derive(Clone, PartialEq)]
struct ToastNotification {
    id: u64,
    tone: NotificationTone,
    title: String,
    message: String,
    created_at_ms: u64,
}

#[derive(Clone, PartialEq, Eq)]
struct PendingDeleteDialog {
    document_paths: Vec<String>,
    group_paths: Vec<String>,
    labels: Vec<String>,
    hard_delete: bool,
}

#[derive(Clone, PartialEq)]
struct RenderSnapshot {
    palette: Palette,
    search_query: String,
    sidebar_open: bool,
    right_panel_mode: RightPanelMode,
    rows: Vec<BrowserRow>,
    selected_path: Option<String>,
    selected_row: Option<BrowserRow>,
    active_session: Option<ManagedSessionView>,
    active_view_mode: WorkspaceViewMode,
    ssh_targets: Vec<SshConnectTarget>,
    live_sessions: Vec<ManagedSessionView>,
    total_leaf_sessions: usize,
    last_action: String,
    ghostty_embedded_surface_supported: bool,
    ghostty_bridge_detail: String,
    server_daemon_detail: String,
    settings: AppSettings,
    install_context: InstallContext,
    maximized: bool,
    always_on_top: bool,
    notifications: Vec<ToastNotification>,
    selected_tree_paths: Vec<String>,
    context_menu_row: Option<BrowserRow>,
    context_menu_position: Option<(f64, f64)>,
    preview_layout: PreviewLayoutMode,
    server_busy: bool,
    ssh_connect_target: String,
    ssh_connect_prefix: String,
    pending_update_restart: Option<PendingUpdateRestart>,
    last_terminal_debug: String,
    last_tree_debug: String,
    active_precis: Option<String>,
    drag_paths: Vec<String>,
    drag_hover_target: Option<String>,
    pending_delete: Option<PendingDeleteDialog>,
    tree_rename_path: Option<String>,
    tree_rename_value: String,
}

#[derive(Clone, Copy, PartialEq)]
struct Palette {
    shell: &'static str,
    titlebar: &'static str,
    sidebar: &'static str,
    sidebar_hover: &'static str,
    panel: &'static str,
    panel_alt: &'static str,
    border: &'static str,
    text: &'static str,
    muted: &'static str,
    accent: &'static str,
    accent_soft: &'static str,
    gradient: &'static str,
    close_hover: &'static str,
    control_hover: &'static str,
    shadow: &'static str,
    panel_shadow: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HoveredControl {
    AlwaysOnTop,
    Minimize,
    Maximize,
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WindowControlIcon {
    AlwaysOnTop,
    Minimize,
    Maximize,
    Restore,
    Close,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TerminalJsCommand {
    Reset {
        title: String,
        background: String,
        foreground: String,
        cursor: String,
        selection: String,
        font_size: f32,
    },
    Write {
        data: String,
    },
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TerminalJsEvent {
    Ready,
    Input { data: String },
    Resize { cols: u16, rows: u16 },
    Debug { message: String },
}

impl ShellState {
    fn new(bootstrap: ShellBootstrap) -> Self {
        let settings = bootstrap.settings.clone();
        let pending_update_restart = bootstrap.pending_update_restart.clone();
        let right_panel_mode = if settings.show_settings {
            RightPanelMode::Settings
        } else {
            RightPanelMode::Metadata
        };
        let sidebar_open = settings.show_tree;
        let mut browser = SessionBrowserState::new(bootstrap.browser_tree.clone());
        browser.restore_ui_state(
            &settings.expanded_browser_paths,
            settings.selected_browser_path.as_deref(),
        );

        let mut server = YggtermServer::new(
            &bootstrap.browser_tree,
            bootstrap.prefer_ghostty_backend,
            yggterm_server::GhosttyHostSupport::shadow(
                bootstrap.ghostty_bridge_detail.clone(),
                bootstrap.ghostty_embedded_surface_supported,
                bootstrap.ghostty_bridge_enabled,
            ),
            bootstrap.theme,
        );
        if let Some(initial_server_snapshot) = bootstrap.initial_server_snapshot.clone() {
            server.apply_snapshot(initial_server_snapshot);
        }

        let needs_initial_server_sync = bootstrap.initial_server_snapshot.is_none();
        let mut state = Self {
            settings,
            bootstrap,
            browser,
            server,
            search_query: String::new(),
            sidebar_open,
            right_panel_mode,
            last_action: "ready".to_string(),
            maximized: false,
            always_on_top: false,
            notifications: Vec::new(),
            next_notification_id: 1,
            selected_tree_paths: HashSet::new(),
            selection_anchor: None,
            context_menu_row: None,
            context_menu_position: None,
            preview_layout: PreviewLayoutMode::Chat,
            server_busy: needs_initial_server_sync,
            server_daemon_detail: String::new(),
            needs_initial_server_sync,
            ssh_connect_target: String::new(),
            ssh_connect_prefix: String::new(),
            pending_update_restart,
            docked_window_id: None,
            dock_sync_in_flight: false,
            last_dock_signature: None,
            last_terminal_debug: "terminal debug idle".to_string(),
            last_tree_debug: "tree debug idle".to_string(),
            generated_precis: BTreeMap::new(),
            precis_requests_in_flight: HashSet::new(),
            drag_paths: Vec::new(),
            drag_hover_target: None,
            pending_delete: None,
            tree_rename_path: None,
            tree_rename_value: String::new(),
        };
        if let Some(path) = state.browser.selected_path().map(ToOwned::to_owned) {
            state.selected_tree_paths.insert(path.clone());
            state.selection_anchor = Some(path);
        }
        state.refresh_tree_debug("init");
        state.server_daemon_detail = state.bootstrap.server_daemon_detail.clone();
        if let Some(update) = state.pending_update_restart.clone() {
            state.last_action = format!("update {} ready", update.version);
            state.push_notification(
                NotificationTone::Success,
                "Update Ready",
                format!("Yggterm {} was installed. Restarting now…", update.version),
            );
        }
        state.sync_browser_settings();
        state
    }

    fn snapshot(&self) -> RenderSnapshot {
        let live_sessions = self.server.live_sessions();
        let rows = merged_sidebar_rows(self.browser.rows(), &live_sessions);
        let selected_path = if let Some(active) = self.server.active_session() {
            if active.source == yggterm_server::SessionSource::LiveSsh {
                Some(active.session_path.clone())
            } else {
                self.browser.selected_path().map(ToOwned::to_owned)
            }
        } else {
            self.browser.selected_path().map(ToOwned::to_owned)
        };
        RenderSnapshot {
            palette: palette(self.bootstrap.theme),
            search_query: self.search_query.clone(),
            sidebar_open: self.sidebar_open,
            right_panel_mode: self.right_panel_mode,
            rows,
            selected_path,
            selected_row: self.browser.selected_row().cloned(),
            active_session: self.server.active_session().cloned(),
            active_view_mode: self.server.active_view_mode(),
            ssh_targets: self.server.ssh_targets().to_vec(),
            live_sessions,
            total_leaf_sessions: self.browser.total_sessions(),
            last_action: self.last_action.clone(),
            ghostty_embedded_surface_supported: self.bootstrap.ghostty_embedded_surface_supported,
            ghostty_bridge_detail: self.bootstrap.ghostty_bridge_detail.clone(),
            server_daemon_detail: self.server_daemon_detail.clone(),
            settings: self.settings.clone(),
            install_context: self.bootstrap.install_context.clone(),
            maximized: self.maximized,
            always_on_top: self.always_on_top,
            notifications: self.notifications.clone(),
            selected_tree_paths: self.selected_tree_paths.iter().cloned().collect(),
            context_menu_row: self.context_menu_row.clone(),
            context_menu_position: self.context_menu_position,
            preview_layout: self.preview_layout,
            server_busy: self.server_busy,
            ssh_connect_target: self.ssh_connect_target.clone(),
            ssh_connect_prefix: self.ssh_connect_prefix.clone(),
            pending_update_restart: self.pending_update_restart.clone(),
            last_terminal_debug: self.last_terminal_debug.clone(),
            last_tree_debug: self.last_tree_debug.clone(),
            active_precis: self
                .server
                .active_session()
                .and_then(|session| self.generated_precis.get(&session.session_path).cloned()),
            drag_paths: self.drag_paths.clone(),
            drag_hover_target: self.drag_hover_target.clone(),
            pending_delete: self.pending_delete.clone(),
            tree_rename_path: self.tree_rename_path.clone(),
            tree_rename_value: self.tree_rename_value.clone(),
        }
    }

    fn set_search(&mut self, query: String) {
        self.search_query = query;
        self.browser.set_filter_query(self.search_query.clone());
        self.last_action = if self.search_query.is_empty() {
            "search cleared".to_string()
        } else {
            format!("filtered {}", self.search_query)
        };
    }

    fn toggle_sidebar(&mut self) {
        self.sidebar_open = !self.sidebar_open;
        self.sync_browser_settings();
        self.last_action = if self.sidebar_open {
            "sidebar opened".to_string()
        } else {
            "sidebar hidden".to_string()
        };
    }

    fn toggle_metadata_panel(&mut self) {
        self.right_panel_mode = if self.right_panel_mode == RightPanelMode::Metadata {
            RightPanelMode::Hidden
        } else {
            RightPanelMode::Metadata
        };
        self.settings.show_settings = false;
        self.persist_settings();
        self.last_action = match self.right_panel_mode {
            RightPanelMode::Metadata => "metadata opened".to_string(),
            RightPanelMode::Hidden => "right panel hidden".to_string(),
            RightPanelMode::Settings => "settings opened".to_string(),
            RightPanelMode::Connect => "ssh connect opened".to_string(),
            RightPanelMode::Notifications => "notifications opened".to_string(),
        };
    }

    fn toggle_settings_panel(&mut self) {
        self.right_panel_mode = if self.right_panel_mode == RightPanelMode::Settings {
            RightPanelMode::Hidden
        } else {
            RightPanelMode::Settings
        };
        self.settings.show_settings = self.right_panel_mode == RightPanelMode::Settings;
        self.persist_settings();
        self.last_action = match self.right_panel_mode {
            RightPanelMode::Settings => "settings opened".to_string(),
            RightPanelMode::Hidden => "right panel hidden".to_string(),
            RightPanelMode::Metadata => "metadata opened".to_string(),
            RightPanelMode::Connect => "ssh connect opened".to_string(),
            RightPanelMode::Notifications => "notifications opened".to_string(),
        };
    }

    fn toggle_connect_panel(&mut self) {
        self.right_panel_mode = if self.right_panel_mode == RightPanelMode::Connect {
            RightPanelMode::Hidden
        } else {
            RightPanelMode::Connect
        };
        self.settings.show_settings = false;
        self.persist_settings();
        self.last_action = match self.right_panel_mode {
            RightPanelMode::Connect => "ssh connect opened".to_string(),
            RightPanelMode::Hidden => "right panel hidden".to_string(),
            RightPanelMode::Metadata => "metadata opened".to_string(),
            RightPanelMode::Settings => "settings opened".to_string(),
            RightPanelMode::Notifications => "notifications opened".to_string(),
        };
    }

    fn update_ssh_connect_target(&mut self, value: String) {
        self.ssh_connect_target = value;
        self.last_action = "editing ssh target".to_string();
    }

    fn update_ssh_connect_prefix(&mut self, value: String) {
        self.ssh_connect_prefix = value;
        self.last_action = "editing ssh prefix".to_string();
    }

    fn toggle_notifications_panel(&mut self) {
        self.right_panel_mode = if self.right_panel_mode == RightPanelMode::Notifications {
            RightPanelMode::Hidden
        } else {
            RightPanelMode::Notifications
        };
        self.last_action = match self.right_panel_mode {
            RightPanelMode::Notifications => "notifications opened".to_string(),
            RightPanelMode::Hidden => "right panel hidden".to_string(),
            RightPanelMode::Metadata => "metadata opened".to_string(),
            RightPanelMode::Settings => "settings opened".to_string(),
            RightPanelMode::Connect => "ssh connect opened".to_string(),
        };
    }

    fn set_preview_layout(&mut self, mode: PreviewLayoutMode) {
        self.preview_layout = mode;
        self.last_action = match mode {
            PreviewLayoutMode::Chat => "chat view".to_string(),
            PreviewLayoutMode::Graph => "graph view".to_string(),
        };
    }

    fn apply_daemon_snapshot_result(&mut self, result: Result<(ServerUiSnapshot, Option<String>)>) {
        match result {
            Ok((snapshot, message)) => {
                self.server.apply_snapshot(snapshot);
                self.server_busy = false;
                self.needs_initial_server_sync = false;
                if let Some(message) = message {
                    self.last_action = message;
                }
            }
            Err(error) => {
                self.server_busy = false;
                self.needs_initial_server_sync = false;
                self.last_action = format!("server sync failed: {error}");
                self.push_notification(
                    NotificationTone::Error,
                    "Server Sync Failed",
                    error.to_string(),
                );
            }
        }
    }

    fn select_row(&mut self, row: &BrowserRow) {
        self.browser.select_path(row.full_path.clone());
        match row.kind {
            BrowserRowKind::Group => {
                self.browser.toggle_group(&row.full_path);
                self.sync_browser_settings();
                self.last_action = format!("toggled {}", row.label);
            }
            BrowserRowKind::Separator => {
                self.context_menu_row = None;
                self.last_action = format!("selected {}", row.label);
            }
            BrowserRowKind::Session | BrowserRowKind::Document => {
                self.context_menu_row = None;
                self.apply_daemon_snapshot_result(open_stored_session(
                    &self.bootstrap.server_endpoint,
                    session_kind_for_row(row.kind),
                    &row.full_path,
                    row.session_id.as_deref(),
                    row.session_cwd.as_deref(),
                    Some(row.label.as_str()),
                ));
                self.sync_browser_settings();
                self.last_action = format!("opened {}", row.label);
            }
        }
    }

    fn update_litellm_endpoint(&mut self, value: String) {
        self.settings.litellm_endpoint = value;
        self.persist_settings();
        self.last_action = "updated LiteLLM endpoint".to_string();
    }

    fn update_litellm_api_key(&mut self, value: String) {
        self.settings.litellm_api_key = value;
        self.persist_settings();
        self.last_action = "updated LiteLLM API key".to_string();
    }

    fn update_interface_llm_model(&mut self, value: String) {
        self.settings.interface_llm_model = value;
        self.persist_settings();
        self.last_action = "updated interface llm".to_string();
    }

    fn update_notification_delivery(&mut self, mode: NotificationDeliveryMode) {
        apply_notification_delivery_mode(&mut self.settings, mode);
        self.persist_settings();
        self.last_action = match mode {
            NotificationDeliveryMode::InApp => "in-app notifications enabled".to_string(),
            NotificationDeliveryMode::Both => "in-app and system notifications enabled".to_string(),
            NotificationDeliveryMode::System => "system notifications enabled".to_string(),
        };
    }

    fn update_notification_sound(&mut self, enabled: bool) {
        self.settings.notification_sound = enabled;
        self.persist_settings();
        self.last_action = if enabled {
            "notification sound enabled".to_string()
        } else {
            "notification sound disabled".to_string()
        };
    }

    fn adjust_ui_zoom(&mut self, delta_steps: i32) {
        self.settings.ui_font_size =
            clamp_zoom_value(self.settings.ui_font_size + delta_steps as f32);
        self.persist_settings();
        self.last_action = format!(
            "interface zoom {}%",
            zoom_percent(self.settings.ui_font_size, 14.0)
        );
    }

    fn adjust_main_zoom(&mut self, delta_steps: i32) {
        let before = self.settings.terminal_font_size;
        self.settings.terminal_font_size =
            clamp_zoom_value_main(self.settings.terminal_font_size + delta_steps as f32);
        self.persist_settings();
        self.last_terminal_debug = if let Some(session) = self.server.active_session() {
            format!(
                "zoom {} -> {} for {} ({})",
                before,
                self.settings.terminal_font_size,
                session.session_path,
                terminal_host_id(&session.session_path)
            )
        } else {
            format!(
                "zoom {} -> {} with no active session",
                before, self.settings.terminal_font_size
            )
        };
        info!(before, after=self.settings.terminal_font_size, debug=%self.last_terminal_debug, "terminal zoom changed");
        self.last_action = format!(
            "terminal zoom {}%",
            zoom_percent(self.settings.terminal_font_size, 10.0)
        );
    }

    fn toggle_maximized(&mut self) {
        window().toggle_maximized();
        self.maximized = !self.maximized;
    }

    fn toggle_always_on_top(&mut self) {
        self.always_on_top = !self.always_on_top;
        window().set_always_on_top(self.always_on_top);
        self.last_action = if self.always_on_top {
            "always on top enabled".to_string()
        } else {
            "always on top disabled".to_string()
        };
    }

    fn open_context_menu(&mut self, row: BrowserRow, position: (f64, f64)) {
        self.context_menu_row = Some(row);
        self.context_menu_position = Some(position);
        self.refresh_tree_debug("open_context_menu");
    }

    fn close_context_menu(&mut self) {
        self.context_menu_row = None;
        self.context_menu_position = None;
        self.refresh_tree_debug("close_context_menu");
    }

    fn clear_notification(&mut self, id: u64) {
        self.notifications
            .retain(|notification| notification.id != id);
    }

    fn clear_notifications(&mut self) {
        self.notifications.clear();
    }

    fn select_tree_row(&mut self, row: &BrowserRow, extend_range: bool) {
        if extend_range && is_workspace_row(row) {
            self.extend_tree_selection(row);
            return;
        }
        self.selected_tree_paths.clear();
        self.selected_tree_paths.insert(row.full_path.clone());
        self.selection_anchor = Some(row.full_path.clone());
        self.browser.select_path(row.full_path.clone());
        if cfg!(debug_assertions) {
            info!(path=%row.full_path, extend_range, "tree row selected");
        }
        self.refresh_tree_debug("select_tree_row");
    }

    fn extend_tree_selection(&mut self, row: &BrowserRow) {
        let rows = merged_sidebar_rows(self.browser.rows(), &self.server.live_sessions());
        let anchor = self
            .selection_anchor
            .clone()
            .or_else(|| self.browser.selected_path().map(ToOwned::to_owned))
            .unwrap_or_else(|| row.full_path.clone());
        let Some(anchor_ix) = rows
            .iter()
            .position(|candidate| candidate.full_path == anchor)
        else {
            self.select_tree_row(row, false);
            return;
        };
        let Some(target_ix) = rows
            .iter()
            .position(|candidate| candidate.full_path == row.full_path)
        else {
            self.select_tree_row(row, false);
            return;
        };
        let (start, end) = if anchor_ix <= target_ix {
            (anchor_ix, target_ix)
        } else {
            (target_ix, anchor_ix)
        };
        self.selected_tree_paths.clear();
        for candidate in rows.iter().skip(start).take(end - start + 1) {
            if is_workspace_row(candidate) {
                self.selected_tree_paths.insert(candidate.full_path.clone());
            }
        }
        if self.selected_tree_paths.is_empty() {
            self.selected_tree_paths.insert(row.full_path.clone());
        }
        self.browser.select_path(row.full_path.clone());
        if cfg!(debug_assertions) {
            info!(anchor=%anchor, target=%row.full_path, selected=%self.selected_tree_paths.len(), "tree range selected");
        }
        self.refresh_tree_debug("extend_tree_selection");
    }

    fn selected_workspace_rows(&self) -> Vec<BrowserRow> {
        let rows = merged_sidebar_rows(self.browser.rows(), &self.server.live_sessions());
        let selected = rows
            .into_iter()
            .filter(|row| self.selected_tree_paths.contains(&row.full_path))
            .filter(is_workspace_row)
            .collect::<Vec<_>>();
        selected
            .iter()
            .filter(|candidate| {
                !selected.iter().any(|other| {
                    other.full_path != candidate.full_path
                        && workspace_path_contains(&other.full_path, &candidate.full_path)
                })
            })
            .cloned()
            .collect()
    }

    fn selected_workspace_delete_paths(&self) -> (Vec<String>, Vec<String>, Vec<String>) {
        let rows = self.selected_workspace_rows();
        let mut document_paths = Vec::new();
        let mut group_paths = Vec::new();
        let mut labels = Vec::new();
        for row in rows {
            labels.push(row.label.clone());
            match row.kind {
                BrowserRowKind::Document => document_paths.push(row.full_path),
                BrowserRowKind::Group | BrowserRowKind::Separator => {
                    group_paths.push(row.full_path)
                }
                BrowserRowKind::Session => {}
            }
        }
        (document_paths, group_paths, labels)
    }

    fn begin_drag(&mut self, row: &BrowserRow) {
        if !is_workspace_row(row) {
            self.drag_paths.clear();
            self.drag_hover_target = None;
            self.refresh_tree_debug("begin_drag_ignored");
            return;
        }
        if !self.selected_tree_paths.contains(&row.full_path) {
            self.selected_tree_paths.clear();
            self.selected_tree_paths.insert(row.full_path.clone());
            self.selection_anchor = Some(row.full_path.clone());
            self.browser.select_path(row.full_path.clone());
        }
        self.drag_paths = self
            .selected_workspace_rows()
            .into_iter()
            .map(|candidate| candidate.full_path)
            .collect();
        self.drag_hover_target = None;
        if cfg!(debug_assertions) {
            info!(drag_count=%self.drag_paths.len(), anchor=%row.full_path, "tree drag started");
        }
        self.refresh_tree_debug("begin_drag");
    }

    fn set_drag_hover_target(&mut self, row: &BrowserRow) {
        self.drag_hover_target =
            valid_drop_target(self.drag_paths.as_slice(), row).then(|| row.full_path.clone());
        if cfg!(debug_assertions)
            && let Some(target) = self.drag_hover_target.as_deref()
        {
            info!(target=%target, drag_count=%self.drag_paths.len(), "tree drag hover");
        }
        self.refresh_tree_debug("set_drag_hover_target");
    }

    fn clear_drag_state(&mut self) {
        self.drag_paths.clear();
        self.drag_hover_target = None;
        self.refresh_tree_debug("clear_drag_state");
    }

    fn open_delete_dialog(&mut self, hard_delete: bool) {
        let (document_paths, group_paths, labels) = self.selected_workspace_delete_paths();
        if document_paths.is_empty() && group_paths.is_empty() {
            return;
        }
        self.pending_delete = Some(PendingDeleteDialog {
            document_paths,
            group_paths,
            labels,
            hard_delete,
        });
        self.close_context_menu();
        self.last_action = if hard_delete {
            "deleting selected items".to_string()
        } else {
            "confirm delete".to_string()
        };
        if cfg!(debug_assertions) {
            info!(
                hard_delete,
                documents=%self.pending_delete.as_ref().map(|pending| pending.document_paths.len()).unwrap_or(0),
                groups=%self.pending_delete.as_ref().map(|pending| pending.group_paths.len()).unwrap_or(0),
                "tree delete dialog opened"
            );
        }
        self.refresh_tree_debug("open_delete_dialog");
    }

    fn cancel_delete_dialog(&mut self) {
        self.pending_delete = None;
        self.refresh_tree_debug("cancel_delete_dialog");
    }

    fn begin_tree_rename(&mut self, row: &BrowserRow) {
        if !is_workspace_row(row) {
            return;
        }
        self.select_tree_row(row, false);
        self.tree_rename_path = Some(row.full_path.clone());
        self.tree_rename_value = row.label.clone();
        self.close_context_menu();
        self.refresh_tree_debug("begin_tree_rename");
    }

    fn update_tree_rename_value(&mut self, value: String) {
        self.tree_rename_value = value;
        self.refresh_tree_debug("update_tree_rename");
    }

    fn cancel_tree_rename(&mut self) {
        self.tree_rename_path = None;
        self.tree_rename_value.clear();
        self.refresh_tree_debug("cancel_tree_rename");
    }

    fn refresh_tree_debug(&mut self, source: &str) {
        self.last_tree_debug = format!(
            "{source} | selected={} [{}] | drag={} [{}] | hover={} | pending_delete={} | rename={}",
            self.selected_tree_paths.len(),
            join_debug_paths(self.selected_tree_paths.iter().cloned().collect()),
            self.drag_paths.len(),
            join_debug_paths(self.drag_paths.clone()),
            self.drag_hover_target
                .clone()
                .unwrap_or_else(|| "none".to_string()),
            self.pending_delete
                .as_ref()
                .map(|pending| format!(
                    "docs:{} groups:{} hard:{}",
                    pending.document_paths.len(),
                    pending.group_paths.len(),
                    pending.hard_delete
                ))
                .unwrap_or_else(|| "none".to_string()),
            self.tree_rename_path
                .clone()
                .unwrap_or_else(|| "none".to_string())
        );
    }

    fn generate_session_titles(&mut self) {
        if self.settings.litellm_endpoint.trim().is_empty()
            || self.settings.litellm_api_key.trim().is_empty()
            || self.settings.interface_llm_model.trim().is_empty()
        {
            self.last_action = "configure LiteLLM settings first".to_string();
            self.push_notification(
                NotificationTone::Warning,
                "LiteLLM Not Configured",
                "Fill the endpoint, API key, and interface model before generating titles.",
            );
            return;
        }

        let selected_path = self.browser.selected_path().map(str::to_string);
        let filter_query = self.search_query.clone();

        match SessionStore::open_or_init().and_then(|store| {
            let generated = store.generate_missing_codex_titles(&self.settings, 8)?;
            let browser_tree = store.load_codex_tree(&self.settings)?;
            Ok((generated, browser_tree))
        }) {
            Ok((generated, browser_tree)) => {
                self.browser = SessionBrowserState::new(browser_tree);
                self.browser.set_filter_query(filter_query);
                if let Some(path) = selected_path {
                    self.browser.select_path(path);
                }
                self.last_action = if generated == 0 {
                    "titles already cached".to_string()
                } else {
                    format!("generated {generated} titles")
                };
                self.push_notification(
                    NotificationTone::Success,
                    "Session Titles Updated",
                    if generated == 0 {
                        "All visible titles were already cached.".to_string()
                    } else {
                        format!("Generated {generated} new session titles.")
                    },
                );
            }
            Err(error) => {
                self.last_action = format!("title generation failed: {error}");
                self.push_notification(
                    NotificationTone::Error,
                    "Title Generation Failed",
                    error.to_string(),
                );
            }
        }
    }

    fn regenerate_title_for_row(&mut self, _row: &BrowserRow) {
        self.context_menu_row = None;
    }

    fn persist_settings(&self) {
        let _ = save_settings_file(&self.bootstrap.settings_path, &self.settings);
    }

    fn sync_browser_settings(&mut self) {
        self.settings.show_tree = self.sidebar_open;
        self.settings.selected_browser_path = self.browser.selected_path().map(ToOwned::to_owned);
        self.settings.expanded_browser_paths = self.browser.expanded_paths();
        self.persist_settings();
    }

    fn push_notification(
        &mut self,
        tone: NotificationTone,
        title: impl Into<String>,
        message: impl Into<String>,
    ) {
        let title = title.into();
        let message = message.into();
        if self.settings.in_app_notifications {
            self.notifications.push(ToastNotification {
                id: self.next_notification_id,
                tone,
                title: title.clone(),
                message: message.clone(),
                created_at_ms: current_millis(),
            });
        }
        if self.settings.system_notifications {
            emit_system_notification(&title, &message);
        }
        if self.settings.notification_sound {
            emit_notification_chime();
        }
        self.next_notification_id += 1;
        if self.notifications.len() > 1000 {
            let overflow = self.notifications.len() - 1000;
            self.notifications.drain(0..overflow);
        }
    }
}

fn queue_title_generation(mut state: Signal<ShellState>, row: BrowserRow, force: bool) {
    if row.kind != BrowserRowKind::Session {
        return;
    }

    let settings = state.read().settings.clone();
    if settings.litellm_endpoint.trim().is_empty()
        || settings.litellm_api_key.trim().is_empty()
        || settings.interface_llm_model.trim().is_empty()
    {
        state.with_mut(|shell| {
            shell.push_notification(
                NotificationTone::Warning,
                "LiteLLM Not Configured",
                "Open settings and configure LiteLLM before generating chat titles.",
            );
        });
        return;
    }

    state.with_mut(|shell| {
        shell.last_action = if force {
            format!("regenerating title for {}", row.label)
        } else {
            format!("generating title for {}", row.label)
        };
    });
    info!(session_path=%row.full_path, force, "queueing title generation");

    spawn(async move {
        let row_for_task = row.clone();
        let settings_for_task = settings.clone();
        let outcome = task::spawn_blocking(
            move || -> Result<(Option<String>, yggterm_core::SessionNode)> {
                info!(session_path=%row_for_task.full_path, force, "running title generation task");
                let store = SessionStore::open_or_init()?;
                let title = store.generate_title_for_session_path(
                    &settings_for_task,
                    &row_for_task.full_path,
                    force,
                )?;
                let browser_tree = store.load_codex_tree(&settings_for_task)?;
                Ok((title, browser_tree))
            },
        )
        .await;

        state.with_mut(|shell| match outcome {
            Ok(Ok((Some(title), browser_tree))) => {
                let selected_path = shell.browser.selected_path().map(str::to_string);
                let expanded_paths = shell.browser.expanded_paths();
                let filter_query = shell.search_query.clone();
                shell.browser = SessionBrowserState::new(browser_tree);
                shell.browser.restore_ui_state(
                    &expanded_paths,
                    selected_path
                        .as_deref()
                        .or(Some(row.full_path.as_str())),
                );
                shell.browser.set_filter_query(filter_query);
                shell.apply_daemon_snapshot_result(open_stored_session(
                    &shell.bootstrap.server_endpoint,
                    SessionKind::Codex,
                    &row.full_path,
                    row.session_id.as_deref(),
                    row.session_cwd.as_deref(),
                    Some(&title),
                ));
                shell.last_action = if force {
                    "regenerated title".to_string()
                } else {
                    "generated title".to_string()
                };
                shell.sync_browser_settings();
                shell.push_notification(
                    NotificationTone::Success,
                    if force {
                        "Title Regenerated"
                    } else {
                        "Title Generated"
                    },
                    format!("Session is now titled “{title}”."),
                );
            }
            Ok(Ok((None, _))) => {
                warn!(session_path=%row.full_path, "title generation produced no usable title");
                shell.push_notification(
                    NotificationTone::Warning,
                    "No Title Generated",
                    "The model did not return a usable short title for this session.",
                );
            }
            Ok(Err(error)) => {
                shell.last_action = format!("title generation failed: {error}");
                warn!(session_path=%row.full_path, error=%error, "title generation failed");
                shell.push_notification(
                    NotificationTone::Error,
                    "Title Generation Failed",
                    error.to_string(),
                );
            }
            Err(error) => {
                shell.last_action = format!("title generation task failed: {error}");
                warn!(session_path=%row.full_path, error=%error, "title generation task join failed");
                shell.push_notification(
                    NotificationTone::Error,
                    "Title Task Failed",
                    error.to_string(),
                );
            }
        });
    });
}

fn spawn_precis_generation(mut state: Signal<ShellState>, session: ManagedSessionView) {
    if !session.kind.is_agent() {
        return;
    }

    let session_path = session.session_path.clone();
    let settings = state.read().settings.clone();
    if settings.litellm_endpoint.trim().is_empty()
        || settings.litellm_api_key.trim().is_empty()
        || settings.interface_llm_model.trim().is_empty()
    {
        return;
    }

    let should_start = state.with_mut(|shell| {
        if shell.generated_precis.contains_key(&session_path)
            || shell.precis_requests_in_flight.contains(&session_path)
        {
            false
        } else {
            shell.precis_requests_in_flight.insert(session_path.clone());
            true
        }
    });
    if !should_start {
        return;
    }

    spawn(async move {
        let path_for_task = session_path.clone();
        let settings_for_task = settings.clone();
        let outcome = task::spawn_blocking(move || -> Result<Option<String>> {
            let store = SessionStore::open_or_init()?;
            if let Some(precis) = store.resolve_precis_for_session_path(&path_for_task)? {
                return Ok(Some(precis));
            }
            store.generate_precis_for_session_path(&settings_for_task, &path_for_task, false)
        })
        .await;

        state.with_mut(|shell| {
            shell.precis_requests_in_flight.remove(&session_path);
            match outcome {
                Ok(Ok(Some(precis))) => {
                    shell.generated_precis.insert(session_path.clone(), precis);
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    shell.last_action = format!("precis generation failed: {error}");
                }
                Err(error) => {
                    shell.last_action = format!("precis task failed: {error}");
                }
            }
        });
    });
}

fn spawn_initial_server_sync(mut state: Signal<ShellState>) {
    let endpoint = state.read().bootstrap.server_endpoint.clone();
    spawn(async move {
        let outcome = task::spawn_blocking(move || initial_server_sync(endpoint)).await;
        state.with_mut(|shell| match outcome {
            Ok(Ok((snapshot, runtime, detail))) => {
                shell.server.apply_snapshot(snapshot);
                shell.server_daemon_detail = detail;
                shell.server_busy = false;
                shell.needs_initial_server_sync = false;
                if let Some(runtime) = runtime {
                    shell.last_action = format!("server ready · {}", runtime.host_kind);
                } else {
                    shell.last_action = "server ready".to_string();
                }
            }
            Ok(Err(error)) => {
                shell.server_busy = false;
                shell.needs_initial_server_sync = false;
                shell.server_daemon_detail = format!("server unavailable: {error}");
                shell.last_action = format!("server sync failed: {error}");
                shell.push_notification(
                    NotificationTone::Warning,
                    "Server Unavailable",
                    error.to_string(),
                );
            }
            Err(error) => {
                shell.server_busy = false;
                shell.needs_initial_server_sync = false;
                shell.server_daemon_detail = format!("server task failed: {error}");
                shell.last_action = format!("server task failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Server Task Failed",
                    error.to_string(),
                );
            }
        });
    });
}

fn spawn_notify_only_update_check(mut state: Signal<ShellState>) {
    let install_context = state.read().bootstrap.install_context.clone();
    spawn(async move {
        let outcome = task::spawn_blocking(move || check_for_update(&install_context)).await;
        state.with_mut(|shell| match outcome {
            Ok(Ok(Some(update))) => {
                let hint = update_command_hint(shell.bootstrap.install_context.channel);
                let suffix = if hint.is_empty() {
                    shell
                        .bootstrap
                        .install_context
                        .manager_hint
                        .clone()
                        .unwrap_or_else(|| {
                            "Use your package manager to update Yggterm.".to_string()
                        })
                } else {
                    format!("Run `{hint}` to update.")
                };
                shell.push_notification(
                    NotificationTone::Warning,
                    "Update Available",
                    format!("Yggterm {} is available. {suffix}", update.version),
                );
            }
            Ok(Ok(None)) => {}
            Ok(Err(error)) => {
                warn!(error=%error, "notify-only update check failed");
            }
            Err(error) => {
                warn!(error=%error, "notify-only update task failed");
            }
        });
    });
}

fn initial_server_sync(
    endpoint: ServerEndpoint,
) -> Result<(ServerUiSnapshot, Option<ServerRuntimeStatus>, String)> {
    if ping(&endpoint).is_err() {
        let current_exe = std::env::current_exe()?;
        Command::new(current_exe)
            .arg("server")
            .arg("daemon")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;

        for _ in 0..20 {
            thread::sleep(Duration::from_millis(150));
            if ping(&endpoint).is_ok() {
                break;
            }
        }
    }

    if ping(&endpoint).is_err() {
        anyhow::bail!("daemon did not become reachable")
    }

    let runtime = status(&endpoint).ok();
    let (snapshot, _) = daemon_snapshot(&endpoint)?;
    let detail = match &endpoint {
        #[cfg(unix)]
        ServerEndpoint::UnixSocket(path) => format!("server connected via {}", path.display()),
        ServerEndpoint::Tcp { host, port } => format!("server connected via {host}:{port}"),
    };
    Ok((snapshot, runtime, detail))
}

fn spawn_server_snapshot_action<F>(mut state: Signal<ShellState>, pending_label: String, request: F)
where
    F: FnOnce(ServerEndpoint) -> Result<(ServerUiSnapshot, Option<String>)> + Send + 'static,
{
    let endpoint = state.read().bootstrap.server_endpoint.clone();
    state.with_mut(|shell| {
        shell.server_busy = true;
        shell.last_action = pending_label.clone();
    });

    spawn(async move {
        let outcome = task::spawn_blocking(move || request(endpoint)).await;
        state.with_mut(|shell| match outcome {
            Ok(result) => shell.apply_daemon_snapshot_result(result),
            Err(error) => {
                shell.server_busy = false;
                shell.last_action = format!("server task failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Server Task Failed",
                    error.to_string(),
                );
            }
        });
    });
}

fn spawn_set_view_mode(mut state: Signal<ShellState>, mode: WorkspaceViewMode) {
    state.with_mut(|shell| {
        shell.server.set_view_mode(mode);
        shell.server_busy = true;
        shell.last_action = match mode {
            WorkspaceViewMode::Rendered => "switching to preview".to_string(),
            WorkspaceViewMode::Terminal => "switching to terminal".to_string(),
        };
    });
    spawn_server_snapshot_action(state, "syncing view mode".to_string(), move |endpoint| {
        if mode == WorkspaceViewMode::Terminal {
            request_terminal_launch(&endpoint)
        } else {
            daemon_set_view_mode(&endpoint, mode)
        }
    });
}

fn spawn_open_session_row(mut state: Signal<ShellState>, row: BrowserRow) {
    state.with_mut(|shell| {
        shell.browser.select_path(row.full_path.clone());
        shell.context_menu_row = None;
        shell.sync_browser_settings();
        shell.server_busy = true;
        shell.last_action = format!("opening {}", row.label);
    });
    spawn_server_snapshot_action(state, format!("opening {}", row.label), move |endpoint| {
        open_stored_session(
            &endpoint,
            session_kind_for_row(row.kind),
            &row.full_path,
            row.session_id.as_deref(),
            row.session_cwd.as_deref(),
            Some(row.label.as_str()),
        )
    });
}

fn spawn_connect_ssh_custom(mut state: Signal<ShellState>) {
    let target = state.read().ssh_connect_target.trim().to_string();
    let prefix = state.read().ssh_connect_prefix.trim().to_string();
    if target.is_empty() {
        state.with_mut(|shell| {
            shell.push_notification(
                NotificationTone::Warning,
                "SSH Target Needed",
                "Enter an SSH target such as dev, pi@jojo, or user@ip.",
            );
        });
        return;
    }
    state.with_mut(|shell| {
        shell.server_busy = true;
        shell.last_action = format!("connecting {target}");
    });
    let endpoint = state.read().bootstrap.server_endpoint.clone();
    spawn(async move {
        let target_for_request = target.clone();
        let target_for_message = target.clone();
        let outcome = task::spawn_blocking(move || {
            connect_ssh_custom(
                &endpoint,
                &target_for_request,
                (!prefix.is_empty()).then_some(prefix.as_str()),
            )
        })
        .await;
        state.with_mut(|shell| match outcome {
            Ok(Ok((snapshot, message))) => {
                shell.server.apply_snapshot(snapshot);
                shell.server_busy = false;
                shell.needs_initial_server_sync = false;
                shell.last_action = message
                    .clone()
                    .unwrap_or_else(|| format!("connected {target_for_message}"));
                shell.ssh_connect_target.clear();
                shell.ssh_connect_prefix.clear();
                shell.push_notification(
                    NotificationTone::Success,
                    "SSH Connected",
                    format!("Ready: {target_for_message}"),
                );
            }
            Ok(Err(error)) => {
                shell.server_busy = false;
                shell.last_action = format!("ssh connect failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "SSH Connection Failed",
                    error.to_string(),
                );
            }
            Err(error) => {
                shell.server_busy = false;
                shell.last_action = format!("ssh task failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "SSH Task Failed",
                    error.to_string(),
                );
            }
        });
    });
}

fn spawn_switch_agent_session_mode(
    mut state: Signal<ShellState>,
    path: String,
    target_kind: SessionKind,
) {
    state.with_mut(|shell| {
        shell.server_busy = true;
        shell.last_action = format!(
            "switching to {}",
            match target_kind {
                SessionKind::Codex => "codex",
                SessionKind::CodexLiteLlm => "codex-litellm",
                _ => "session",
            }
        );
    });
    spawn_server_snapshot_action(state, "switching agent mode".to_string(), move |endpoint| {
        switch_agent_session_mode(&endpoint, &path, target_kind)
    });
}

fn spawn_start_group_session(mut state: Signal<ShellState>, row: BrowserRow, kind: SessionKind) {
    if row.kind == BrowserRowKind::Separator
        || row.group_kind == Some(WorkspaceGroupKind::Separator)
    {
        return;
    }
    let cwd = group_session_cwd(&row);
    let title_hint = Some(group_session_title_hint(&row, kind));
    let pending = format!(
        "starting {} in {}",
        session_kind_action_label(kind),
        row.label
    );
    state.with_mut(|shell| shell.close_context_menu());
    spawn_server_snapshot_action(state, pending, move |endpoint| {
        start_local_session_at(&endpoint, kind, cwd.as_deref(), title_hint.as_deref())
    });
}

fn preferred_agent_session_kind(settings: &AppSettings) -> SessionKind {
    match settings.default_agent_profile {
        AgentSessionProfile::CodexLiteLlm => SessionKind::CodexLiteLlm,
        AgentSessionProfile::Codex => SessionKind::Codex,
    }
}

fn session_kind_for_row(kind: BrowserRowKind) -> SessionKind {
    match kind {
        BrowserRowKind::Session => SessionKind::Codex,
        BrowserRowKind::Document => SessionKind::Document,
        BrowserRowKind::Group => SessionKind::Codex,
        BrowserRowKind::Separator => SessionKind::Codex,
    }
}

fn session_kind_action_label(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Codex | SessionKind::CodexLiteLlm => "session",
        SessionKind::Shell => "terminal",
        SessionKind::SshShell => "ssh session",
        SessionKind::Document => "paper",
    }
}

fn is_live_sidebar_row(row: &BrowserRow) -> bool {
    row.full_path.starts_with("ssh://")
        || row.full_path.starts_with("local://")
        || row.full_path.starts_with("codex://")
        || row.full_path.starts_with("codex-litellm://")
}

fn merged_sidebar_rows(
    stored_rows: &[BrowserRow],
    live_sessions: &[ManagedSessionView],
) -> Vec<BrowserRow> {
    if live_sessions.is_empty() {
        return stored_rows.to_vec();
    }

    let mut rows = Vec::with_capacity(stored_rows.len() + live_sessions.len() + 1);
    rows.push(BrowserRow {
        kind: BrowserRowKind::Group,
        full_path: "__live_sessions__".to_string(),
        label: "live".to_string(),
        detail_label: String::new(),
        document_kind: None,
        group_kind: None,
        session_title: None,
        depth: 0,
        host_label: String::new(),
        descendant_sessions: live_sessions.len(),
        expanded: true,
        session_id: None,
        session_cwd: None,
    });
    push_live_group_rows(
        &mut rows,
        "agent sessions",
        "__live_agents__",
        1,
        live_sessions.iter().filter(|session| {
            matches!(session.kind, SessionKind::Codex | SessionKind::CodexLiteLlm)
        }),
    );
    push_live_group_rows(
        &mut rows,
        "shell sessions",
        "__live_shells__",
        1,
        live_sessions
            .iter()
            .filter(|session| session.kind == SessionKind::Shell),
    );
    push_live_group_rows(
        &mut rows,
        "ssh sessions",
        "__live_ssh__",
        1,
        live_sessions
            .iter()
            .filter(|session| session.kind == SessionKind::SshShell),
    );
    rows.extend_from_slice(stored_rows);
    rows
}

fn push_live_group_rows<'a, I>(
    rows: &mut Vec<BrowserRow>,
    label: &str,
    group_path: &str,
    depth: usize,
    sessions: I,
) where
    I: Iterator<Item = &'a ManagedSessionView>,
{
    let collected = sessions.collect::<Vec<_>>();
    if collected.is_empty() {
        return;
    }
    rows.push(BrowserRow {
        kind: BrowserRowKind::Group,
        full_path: group_path.to_string(),
        label: label.to_string(),
        detail_label: String::new(),
        document_kind: None,
        group_kind: None,
        session_title: None,
        depth,
        host_label: "live".to_string(),
        descendant_sessions: collected.len(),
        expanded: true,
        session_id: None,
        session_cwd: None,
    });
    for session in collected {
        let label = if session.title.trim().is_empty() {
            session.id.clone()
        } else {
            session.title.clone()
        };
        rows.push(BrowserRow {
            kind: BrowserRowKind::Session,
            full_path: session.session_path.clone(),
            label,
            detail_label: session.status_line.clone(),
            document_kind: None,
            group_kind: None,
            session_title: Some(session.title.clone()),
            depth: depth + 1,
            host_label: "live".to_string(),
            descendant_sessions: 1,
            expanded: true,
            session_id: Some(session.id.clone()),
            session_cwd: session
                .metadata
                .iter()
                .find(|entry| entry.label == "Cwd")
                .map(|entry| entry.value.clone()),
        });
    }
}

fn queue_session_note_creation(mut state: Signal<ShellState>, row: BrowserRow) {
    if row.kind != BrowserRowKind::Session {
        return;
    }

    state.with_mut(|shell| {
        shell.last_action = format!("creating note for {}", row.label);
        shell.server_busy = true;
    });

    spawn(async move {
        let row_for_task = row.clone();
        let settings = state.read().settings.clone();
        let outcome = task::spawn_blocking(
            move || -> Result<(yggterm_core::WorkspaceDocument, yggterm_core::SessionNode)> {
                let store = SessionStore::open_or_init()?;
                let document = store.save_document(
                    &session_note_virtual_path(&row_for_task),
                    Some(&format!("{} notes", row_for_task.label)),
                    &session_note_template(&row_for_task),
                )?;
                let browser_tree = store.load_codex_tree(&settings)?;
                Ok((document, browser_tree))
            },
        )
        .await;

        state.with_mut(|shell| match outcome {
            Ok(Ok((document, browser_tree))) => {
                let expanded_paths = shell.browser.expanded_paths();
                shell.browser = SessionBrowserState::new(browser_tree);
                shell
                    .browser
                    .restore_ui_state(&expanded_paths, Some(&document.virtual_path));
                shell.browser.select_path(document.virtual_path.clone());
                shell.selected_tree_paths.clear();
                shell
                    .selected_tree_paths
                    .insert(document.virtual_path.clone());
                shell.selection_anchor = Some(document.virtual_path.clone());
                shell.sync_browser_settings();
                shell.apply_daemon_snapshot_result(open_stored_session(
                    &shell.bootstrap.server_endpoint,
                    SessionKind::Document,
                    &document.virtual_path,
                    Some(&document.id),
                    Some(&document.virtual_path),
                    Some(&document.title),
                ));
                shell.last_action = format!("created note for {}", row.label);
                shell.push_notification(
                    NotificationTone::Success,
                    "Session Note Created",
                    format!("Opened {}.", document.title),
                );
            }
            Ok(Err(error)) => {
                shell.server_busy = false;
                shell.last_action = format!("note creation failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Note Creation Failed",
                    error.to_string(),
                );
            }
            Err(error) => {
                shell.server_busy = false;
                shell.last_action = format!("note task failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Note Task Failed",
                    error.to_string(),
                );
            }
        });
    });
}

fn queue_session_recipe_creation(mut state: Signal<ShellState>, row: BrowserRow) {
    if row.kind != BrowserRowKind::Session {
        return;
    }

    state.with_mut(|shell| {
        shell.last_action = format!("creating recipe for {}", row.label);
        shell.server_busy = true;
    });

    spawn(async move {
        let row_for_task = row.clone();
        let settings = state.read().settings.clone();
        let outcome = task::spawn_blocking(
            move || -> Result<(yggterm_core::WorkspaceDocument, yggterm_core::SessionNode)> {
                let store = SessionStore::open_or_init()?;
                let document = store.save_document_input(
                    &session_recipe_virtual_path(&row_for_task),
                    WorkspaceDocumentInput {
                        title: Some(format!("{} recipe", row_for_task.label)),
                        kind: WorkspaceDocumentKind::TerminalRecipe,
                        body: session_recipe_template(&row_for_task),
                        source_session_path: Some(row_for_task.full_path.clone()),
                        source_session_kind: Some(
                            inferred_session_kind_name(&row_for_task).to_string(),
                        ),
                        source_session_cwd: row_for_task.session_cwd.clone(),
                        replay_commands: initial_replay_commands_for_row(&row_for_task),
                    },
                )?;
                let browser_tree = store.load_codex_tree(&settings)?;
                Ok((document, browser_tree))
            },
        )
        .await;

        state.with_mut(|shell| match outcome {
            Ok(Ok((document, browser_tree))) => {
                let expanded_paths = shell.browser.expanded_paths();
                shell.browser = SessionBrowserState::new(browser_tree);
                shell
                    .browser
                    .restore_ui_state(&expanded_paths, Some(&document.virtual_path));
                shell.browser.select_path(document.virtual_path.clone());
                shell.sync_browser_settings();
                shell.apply_daemon_snapshot_result(open_stored_session(
                    &shell.bootstrap.server_endpoint,
                    SessionKind::Document,
                    &document.virtual_path,
                    Some(&document.id),
                    Some(&document.virtual_path),
                    Some(&document.title),
                ));
                shell.last_action = format!("created recipe for {}", row.label);
                shell.push_notification(
                    NotificationTone::Success,
                    "Session Recipe Created",
                    format!("Opened {}.", document.title),
                );
            }
            Ok(Err(error)) => {
                shell.server_busy = false;
                shell.last_action = format!("recipe creation failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Recipe Creation Failed",
                    error.to_string(),
                );
            }
            Err(error) => {
                shell.server_busy = false;
                shell.last_action = format!("recipe task failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Recipe Task Failed",
                    error.to_string(),
                );
            }
        });
    });
}

fn queue_new_document(mut state: Signal<ShellState>) {
    state.with_mut(|shell| {
        shell.server_busy = true;
        shell.last_action = "creating document".to_string();
    });

    let selected_row = state.read().browser.selected_row().cloned();
    let active_session = state.read().server.active_session().cloned();
    let settings = state.read().settings.clone();

    spawn(async move {
        let outcome = task::spawn_blocking(
            move || -> Result<(yggterm_core::WorkspaceDocument, yggterm_core::SessionNode)> {
                let store = SessionStore::open_or_init()?;
                let virtual_path =
                    new_document_virtual_path(selected_row.as_ref(), active_session.as_ref());
                let document = store.save_document(
                    &virtual_path,
                    Some("Untitled note"),
                    "# Untitled note\n\nStart writing here.\n",
                )?;
                let browser_tree = store.load_codex_tree(&settings)?;
                Ok((document, browser_tree))
            },
        )
        .await;

        state.with_mut(|shell| match outcome {
            Ok(Ok((document, browser_tree))) => {
                let expanded_paths = shell.browser.expanded_paths();
                shell.browser = SessionBrowserState::new(browser_tree);
                shell
                    .browser
                    .restore_ui_state(&expanded_paths, Some(&document.virtual_path));
                shell.browser.select_path(document.virtual_path.clone());
                shell.sync_browser_settings();
                shell.apply_daemon_snapshot_result(open_stored_session(
                    &shell.bootstrap.server_endpoint,
                    SessionKind::Document,
                    &document.virtual_path,
                    Some(&document.id),
                    Some(&document.virtual_path),
                    Some(&document.title),
                ));
                shell.last_action = format!("created {}", document.title);
                shell.refresh_tree_debug("created_document");
            }
            Ok(Err(error)) => {
                shell.server_busy = false;
                shell.last_action = format!("document creation failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Document Creation Failed",
                    error.to_string(),
                );
            }
            Err(error) => {
                shell.server_busy = false;
                shell.last_action = format!("document task failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Document Task Failed",
                    error.to_string(),
                );
            }
        });
    });
}

fn queue_new_document_for_row(state: Signal<ShellState>, row: BrowserRow) {
    queue_new_workspace_document_for_row(
        state,
        row,
        WorkspaceDocumentKind::Note,
        "Untitled note",
        "# Untitled note\n\nStart writing here.\n".to_string(),
        None,
    );
}

fn queue_new_workspace_document_for_row(
    mut state: Signal<ShellState>,
    row: BrowserRow,
    kind: WorkspaceDocumentKind,
    title: &str,
    body: String,
    replay_commands: Option<Vec<String>>,
) {
    let action_label = match kind {
        WorkspaceDocumentKind::Note => "document",
        WorkspaceDocumentKind::TerminalRecipe => "recipe",
    };
    state.with_mut(|shell| {
        shell.server_busy = true;
        shell.last_action = format!("creating {action_label} in {}", row.label);
        shell.close_context_menu();
    });

    let settings = state.read().settings.clone();
    let title = title.to_string();
    spawn(async move {
        let row_for_task = row.clone();
        let outcome = task::spawn_blocking(
            move || -> Result<(yggterm_core::WorkspaceDocument, yggterm_core::SessionNode)> {
                let store = SessionStore::open_or_init()?;
                let virtual_path = new_document_virtual_path_for_row(&row_for_task);
                let document = store.save_document_input(
                    &virtual_path,
                    WorkspaceDocumentInput {
                        title: Some(title),
                        kind,
                        body,
                        replay_commands: replay_commands.unwrap_or_default(),
                        ..WorkspaceDocumentInput::default()
                    },
                )?;
                let browser_tree = store.load_codex_tree(&settings)?;
                Ok((document, browser_tree))
            },
        )
        .await;

        state.with_mut(|shell| match outcome {
            Ok(Ok((document, browser_tree))) => {
                let expanded_paths = shell.browser.expanded_paths();
                shell.browser = SessionBrowserState::new(browser_tree);
                shell
                    .browser
                    .restore_ui_state(&expanded_paths, Some(&document.virtual_path));
                shell.browser.select_path(document.virtual_path.clone());
                shell.selected_tree_paths.clear();
                shell
                    .selected_tree_paths
                    .insert(document.virtual_path.clone());
                shell.selection_anchor = Some(document.virtual_path.clone());
                shell.sync_browser_settings();
                shell.apply_daemon_snapshot_result(open_stored_session(
                    &shell.bootstrap.server_endpoint,
                    SessionKind::Document,
                    &document.virtual_path,
                    Some(&document.id),
                    Some(&document.virtual_path),
                    Some(&document.title),
                ));
                shell.last_action = format!("created {}", document.title);
                shell.refresh_tree_debug("created_workspace_document");
            }
            Ok(Err(error)) => {
                shell.server_busy = false;
                shell.last_action = format!("{action_label} creation failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Workspace Create Failed",
                    error.to_string(),
                );
            }
            Err(error) => {
                shell.server_busy = false;
                shell.last_action = format!("{action_label} task failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Workspace Task Failed",
                    error.to_string(),
                );
            }
        });
    });
}

fn queue_new_group_for_row(mut state: Signal<ShellState>, row: BrowserRow) {
    state.with_mut(|shell| {
        shell.server_busy = true;
        shell.last_action = format!("creating group in {}", row.label);
        shell.close_context_menu();
    });

    let settings = state.read().settings.clone();
    spawn(async move {
        let row_for_task = row.clone();
        let outcome =
            task::spawn_blocking(move || -> Result<(String, yggterm_core::SessionNode)> {
                let store = SessionStore::open_or_init()?;
                let virtual_path = new_group_virtual_path_for_row(&row_for_task);
                store.save_group_with_kind(
                    &virtual_path,
                    Some("New Folder"),
                    WorkspaceGroupKind::Folder,
                )?;
                Ok((virtual_path, store.load_codex_tree(&settings)?))
            })
            .await;

        state.with_mut(|shell| match outcome {
            Ok(Ok((selected_path, browser_tree))) => {
                let expanded_paths = shell.browser.expanded_paths();
                shell.browser = SessionBrowserState::new(browser_tree);
                shell
                    .browser
                    .restore_ui_state(&expanded_paths, Some(&selected_path));
                shell.browser.select_path(selected_path.clone());
                shell.selected_tree_paths.clear();
                shell.selected_tree_paths.insert(selected_path.clone());
                shell.selection_anchor = Some(selected_path.clone());
                shell.sync_browser_settings();
                shell.server_busy = false;
                shell.last_action = format!("added folder in {}", row.label);
                shell.refresh_tree_debug("added_folder");
            }
            Ok(Err(error)) => {
                shell.server_busy = false;
                shell.last_action = format!("folder creation failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Folder Creation Failed",
                    error.to_string(),
                );
            }
            Err(error) => {
                shell.server_busy = false;
                shell.last_action = format!("folder task failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Folder Task Failed",
                    error.to_string(),
                );
            }
        });
    });
}

fn queue_new_separator_for_row(mut state: Signal<ShellState>, row: BrowserRow) {
    if row.kind == BrowserRowKind::Separator
        || row.group_kind == Some(WorkspaceGroupKind::Separator)
    {
        return;
    }
    state.with_mut(|shell| {
        shell.server_busy = true;
        shell.last_action = format!("adding separator in {}", row.label);
        shell.close_context_menu();
    });

    let settings = state.read().settings.clone();
    spawn(async move {
        let row_for_task = row.clone();
        let outcome =
            task::spawn_blocking(move || -> Result<(String, yggterm_core::SessionNode)> {
                let store = SessionStore::open_or_init()?;
                let virtual_path = new_separator_virtual_path_for_row(&row_for_task);
                store.save_group_with_kind(
                    &virtual_path,
                    Some("Separator"),
                    WorkspaceGroupKind::Separator,
                )?;
                Ok((virtual_path, store.load_codex_tree(&settings)?))
            })
            .await;

        state.with_mut(|shell| match outcome {
            Ok(Ok((selected_path, browser_tree))) => {
                let expanded_paths = shell.browser.expanded_paths();
                shell.browser = SessionBrowserState::new(browser_tree);
                shell
                    .browser
                    .restore_ui_state(&expanded_paths, Some(&selected_path));
                shell.browser.select_path(selected_path.clone());
                shell.selected_tree_paths.clear();
                shell.selected_tree_paths.insert(selected_path.clone());
                shell.selection_anchor = Some(selected_path.clone());
                shell.sync_browser_settings();
                shell.server_busy = false;
                shell.last_action = format!("added separator in {}", row.label);
                shell.refresh_tree_debug("added_separator");
            }
            Ok(Err(error)) => {
                shell.server_busy = false;
                shell.last_action = format!("separator creation failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Separator Creation Failed",
                    error.to_string(),
                );
            }
            Err(error) => {
                shell.server_busy = false;
                shell.last_action = format!("separator task failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Separator Task Failed",
                    error.to_string(),
                );
            }
        });
    });
}

fn queue_tree_rename(mut state: Signal<ShellState>, row: BrowserRow, label: String) {
    let trimmed = label.trim().to_string();
    if trimmed.is_empty() {
        state.with_mut(|shell| shell.cancel_tree_rename());
        return;
    }

    state.with_mut(|shell| {
        shell.server_busy = true;
        shell.last_action = format!("renaming {}", row.label);
    });

    let settings = state.read().settings.clone();
    spawn(async move {
        let row_for_task = row.clone();
        let trimmed_for_task = trimmed.clone();
        let outcome = task::spawn_blocking(move || -> Result<SessionNode> {
            let store = SessionStore::open_or_init()?;
            match row_for_task.kind {
                BrowserRowKind::Document => {
                    let existing = store
                        .load_document(&row_for_task.full_path)?
                        .ok_or_else(|| anyhow!("paper not found: {}", row_for_task.full_path))?;
                    store.save_document_input(
                        &row_for_task.full_path,
                        WorkspaceDocumentInput {
                            title: Some(trimmed_for_task.clone()),
                            kind: existing.kind,
                            body: existing.body,
                            source_session_path: existing.source_session_path,
                            source_session_kind: existing.source_session_kind,
                            source_session_cwd: existing.source_session_cwd,
                            replay_commands: existing.replay_commands,
                        },
                    )?;
                }
                BrowserRowKind::Group | BrowserRowKind::Separator => {
                    store.save_group_with_kind(
                        &row_for_task.full_path,
                        Some(&trimmed_for_task),
                        row_for_task
                            .group_kind
                            .unwrap_or(WorkspaceGroupKind::Folder),
                    )?;
                }
                BrowserRowKind::Session => {}
            }
            store.load_codex_tree(&settings)
        })
        .await;

        state.with_mut(|shell| match outcome {
            Ok(Ok(browser_tree)) => {
                let expanded_paths = shell.browser.expanded_paths();
                shell.browser = SessionBrowserState::new(browser_tree);
                shell
                    .browser
                    .restore_ui_state(&expanded_paths, Some(&row.full_path));
                shell.browser.select_path(row.full_path.clone());
                shell.selected_tree_paths.clear();
                shell.selected_tree_paths.insert(row.full_path.clone());
                shell.selection_anchor = Some(row.full_path.clone());
                shell.tree_rename_path = None;
                shell.tree_rename_value.clear();
                shell.sync_browser_settings();
                shell.server_busy = false;
                shell.last_action = format!("renamed {}", trimmed);
                shell.refresh_tree_debug("commit_tree_rename");
            }
            Ok(Err(error)) => {
                shell.server_busy = false;
                shell.last_action = format!("rename failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Rename Failed",
                    error.to_string(),
                );
            }
            Err(error) => {
                shell.server_busy = false;
                shell.last_action = format!("rename task failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Rename Task Failed",
                    error.to_string(),
                );
            }
        });
    });
}

fn queue_move_selected_items_to_group(mut state: Signal<ShellState>, target_group: BrowserRow) {
    if !valid_drop_target(&state.read().drag_paths, &target_group)
        && !is_drop_target_row(&target_group)
    {
        return;
    }
    let selected_rows = state.read().selected_workspace_rows();
    if selected_rows.is_empty() {
        return;
    }

    state.with_mut(|shell| {
        shell.server_busy = true;
        shell.last_action = format!(
            "moving {} item(s) to {}",
            selected_rows.len(),
            target_group.label
        );
        shell.close_context_menu();
        shell.drag_hover_target = None;
    });

    let settings = state.read().settings.clone();
    spawn(async move {
        let selected_for_task = selected_rows.clone();
        let target_for_task = target_group.clone();
        let outcome = task::spawn_blocking(
            move || -> Result<(Vec<String>, yggterm_core::SessionNode)> {
                let store = SessionStore::open_or_init()?;
                let mut moved_paths = Vec::new();
                for row in &selected_for_task {
                    let destination = moved_workspace_item_virtual_path(row, &target_for_task);
                    match row.kind {
                        BrowserRowKind::Document => {
                            let document = store.move_document(&row.full_path, &destination)?;
                            moved_paths.push(document.virtual_path);
                        }
                        BrowserRowKind::Group | BrowserRowKind::Separator => {
                            let group = store.move_group(&row.full_path, &destination)?;
                            moved_paths.push(group.virtual_path);
                        }
                        BrowserRowKind::Session => {}
                    }
                }
                let browser_tree = store.load_codex_tree(&settings)?;
                Ok((moved_paths, browser_tree))
            },
        )
        .await;

        state.with_mut(|shell| match outcome {
            Ok(Ok((moved_paths, browser_tree))) => {
                let expanded_paths = shell.browser.expanded_paths();
                shell.browser = SessionBrowserState::new(browser_tree);
                shell
                    .browser
                    .restore_ui_state(&expanded_paths, moved_paths.first().map(String::as_str));
                shell.selected_tree_paths = moved_paths.iter().cloned().collect();
                if let Some(path) = moved_paths.first() {
                    shell.browser.select_path(path.clone());
                    shell.selection_anchor = Some(path.clone());
                }
                shell.sync_browser_settings();
                shell.server_busy = false;
                shell.clear_drag_state();
                shell.last_action = format!(
                    "moved {} item(s) to {}",
                    moved_paths.len(),
                    target_group.label
                );
                shell.push_notification(
                    NotificationTone::Success,
                    "Items Moved",
                    format!(
                        "Moved {} item(s) into {}.",
                        moved_paths.len(),
                        target_group.label
                    ),
                );
            }
            Ok(Err(error)) => {
                shell.server_busy = false;
                shell.clear_drag_state();
                shell.last_action = format!("move failed: {error}");
                shell.push_notification(NotificationTone::Error, "Move Failed", error.to_string());
            }
            Err(error) => {
                shell.server_busy = false;
                shell.clear_drag_state();
                shell.last_action = format!("move task failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Move Task Failed",
                    error.to_string(),
                );
            }
        });
    });
}

fn queue_delete_selected_items(mut state: Signal<ShellState>, hard_delete: bool) {
    let pending = if let Some(pending) = state.read().pending_delete.clone() {
        pending
    } else {
        let mut shell = state.write();
        let (document_paths, group_paths, labels) = shell.selected_workspace_delete_paths();
        if document_paths.is_empty() && group_paths.is_empty() {
            return;
        }
        let pending = PendingDeleteDialog {
            document_paths,
            group_paths,
            labels,
            hard_delete,
        };
        if !hard_delete {
            shell.pending_delete = Some(pending);
            shell.close_context_menu();
            shell.last_action = "confirm delete".to_string();
            shell.refresh_tree_debug("queue_delete_selected_items");
            return;
        }
        pending
    };

    state.with_mut(|shell| {
        shell.server_busy = true;
        shell.pending_delete = None;
        let item_count = pending.document_paths.len() + pending.group_paths.len();
        shell.last_action = if hard_delete {
            format!("permanently deleting {item_count} item(s)")
        } else {
            format!("deleting {item_count} item(s)")
        };
        shell.close_context_menu();
        shell.refresh_tree_debug("delete_confirmed");
    });

    spawn(async move {
        let pending_for_task = pending.clone();
        let outcome =
            task::spawn_blocking(move || -> Result<(usize, yggterm_core::SessionNode)> {
                let store = SessionStore::open_or_init()?;
                let deleted = store.delete_workspace_items(
                    &pending_for_task.document_paths,
                    &pending_for_task.group_paths,
                )?;
                let settings = store.load_settings().unwrap_or_default();
                let browser_tree = store.load_codex_tree(&settings)?;
                Ok((deleted, browser_tree))
            })
            .await;

        state.with_mut(|shell| match outcome {
            Ok(Ok((deleted, browser_tree))) => {
                let expanded_paths = shell.browser.expanded_paths();
                shell.browser = SessionBrowserState::new(browser_tree);
                shell.browser.restore_ui_state(&expanded_paths, None);
                shell.selected_tree_paths.clear();
                if let Some(path) = shell.browser.selected_path().map(ToOwned::to_owned) {
                    shell.selected_tree_paths.insert(path.clone());
                    shell.selection_anchor = Some(path);
                } else {
                    shell.selection_anchor = None;
                }
                shell.sync_browser_settings();
                shell.server_busy = false;
                shell.clear_drag_state();
                shell.last_action = format!("deleted {deleted} workspace item(s)");
                shell.refresh_tree_debug("delete_finished");
                shell.push_notification(
                    NotificationTone::Success,
                    "Items Deleted",
                    format!("Removed {deleted} item(s) from the workspace tree."),
                );
            }
            Ok(Err(error)) => {
                shell.server_busy = false;
                shell.last_action = format!("delete failed: {error}");
                shell.refresh_tree_debug("delete_failed");
                shell.push_notification(
                    NotificationTone::Error,
                    "Delete Failed",
                    error.to_string(),
                );
            }
            Err(error) => {
                shell.server_busy = false;
                shell.last_action = format!("delete task failed: {error}");
                shell.refresh_tree_debug("delete_task_failed");
                shell.push_notification(
                    NotificationTone::Error,
                    "Delete Task Failed",
                    error.to_string(),
                );
            }
        });
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AfterSaveAction {
    SaveOnly,
    RunHere,
    RunNewSession,
}

fn queue_document_save(
    mut state: Signal<ShellState>,
    virtual_path: String,
    input: WorkspaceDocumentInput,
    after_save: AfterSaveAction,
) {
    state.with_mut(|shell| {
        shell.server_busy = true;
        shell.last_action = format!(
            "saving {}",
            input
                .title
                .clone()
                .unwrap_or_else(|| "document".to_string())
        );
    });

    let settings = state.read().settings.clone();
    spawn(async move {
        let outcome = task::spawn_blocking(
            move || -> Result<(yggterm_core::WorkspaceDocument, yggterm_core::SessionNode)> {
                let store = SessionStore::open_or_init()?;
                let document = store.save_document_input(&virtual_path, input)?;
                let browser_tree = store.load_codex_tree(&settings)?;
                Ok((document, browser_tree))
            },
        )
        .await;

        let mut should_run_here = false;
        let mut run_new_session: Option<(Option<String>, String, String, String)> = None;
        state.with_mut(|shell| match outcome {
            Ok(Ok((document, browser_tree))) => {
                let expanded_paths = shell.browser.expanded_paths();
                let filter_query = shell.search_query.clone();
                shell.browser = SessionBrowserState::new(browser_tree);
                shell
                    .browser
                    .restore_ui_state(&expanded_paths, Some(&document.virtual_path));
                shell.browser.set_filter_query(filter_query);
                shell.browser.select_path(document.virtual_path.clone());
                shell.sync_browser_settings();
                shell.apply_daemon_snapshot_result(open_stored_session(
                    &shell.bootstrap.server_endpoint,
                    SessionKind::Document,
                    &document.virtual_path,
                    Some(&document.id),
                    Some(&document.virtual_path),
                    Some(&document.title),
                ));
                shell.last_action = format!("saved {}", document.title);
                shell.push_notification(
                    NotificationTone::Success,
                    "Document Saved",
                    format!("Updated {}.", document.title),
                );
                if matches!(after_save, AfterSaveAction::RunHere) {
                    shell.last_action = format!("running {}", document.title);
                    should_run_here = true;
                } else if matches!(after_save, AfterSaveAction::RunNewSession) {
                    if !document.replay_commands.is_empty() {
                        run_new_session = Some((
                            document.source_session_cwd.clone(),
                            format!("{} session", document.title),
                            document.replay_commands.join("\n"),
                            document.title.clone(),
                        ));
                        shell.last_action = format!("starting {}", document.title);
                    }
                }
            }
            Ok(Err(error)) => {
                shell.server_busy = false;
                shell.last_action = format!("document save failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Document Save Failed",
                    error.to_string(),
                );
            }
            Err(error) => {
                shell.server_busy = false;
                shell.last_action = format!("document task failed: {error}");
                shell.push_notification(
                    NotificationTone::Error,
                    "Document Task Failed",
                    error.to_string(),
                );
            }
        });

        if should_run_here {
            spawn_set_view_mode(state, WorkspaceViewMode::Terminal);
        } else if let Some((cwd, title, launch_command, source_title)) = run_new_session {
            state.with_mut(|shell| {
                shell.server_busy = true;
                shell.last_action = format!("starting {}", source_title);
            });
            spawn_server_snapshot_action(state, format!("starting {title}"), move |endpoint| {
                start_command_session(
                    &endpoint,
                    cwd.as_deref(),
                    Some(&title),
                    &launch_command,
                    Some(&source_title),
                )
            });
        }
    });
}

fn session_note_virtual_path(row: &BrowserRow) -> String {
    let base = row
        .session_cwd
        .clone()
        .unwrap_or_else(|| "/documents".to_string());
    let slug = row
        .label
        .to_ascii_lowercase()
        .chars()
        .map(|ch| match ch {
            'a'..='z' | '0'..='9' => ch,
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let slug = if slug.is_empty() {
        "session-note".to_string()
    } else {
        slug
    };
    format!("{}/notes/{}", base.trim_end_matches('/'), slug)
}

fn session_note_template(row: &BrowserRow) -> String {
    let location = row
        .session_cwd
        .clone()
        .unwrap_or_else(|| row.full_path.clone());
    format!(
        "# {title}\n\nSession: {title}\nLocation: {location}\n\n## Goals\n- \n\n## Commands\n- \n\n## Notes\n- \n",
        title = row.label,
        location = location,
    )
}

fn session_recipe_virtual_path(row: &BrowserRow) -> String {
    let base = row
        .session_cwd
        .clone()
        .unwrap_or_else(|| "/documents".to_string());
    let slug = row
        .label
        .to_ascii_lowercase()
        .chars()
        .map(|ch| match ch {
            'a'..='z' | '0'..='9' => ch,
            _ => '-',
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let slug = if slug.is_empty() {
        "terminal-recipe".to_string()
    } else {
        slug
    };
    format!("{}/recipes/{}", base.trim_end_matches('/'), slug)
}

fn session_recipe_template(row: &BrowserRow) -> String {
    let location = row
        .session_cwd
        .clone()
        .unwrap_or_else(|| row.full_path.clone());
    let kind = inferred_session_kind_name(row);
    format!(
        "# {title} recipe\n\nSource Session: {title}\nSession Kind: {kind}\nLocation: {location}\n\n## Intent\n- \n\n## Replay Plan\n- \n\n## Expected State\n- \n",
        title = row.label,
        kind = kind,
        location = location,
    )
}

fn inferred_session_kind_name(row: &BrowserRow) -> &'static str {
    if row.full_path.starts_with("codex-litellm://") {
        "codex_litellm"
    } else if row.full_path.starts_with("codex://") {
        "codex"
    } else if row.full_path.starts_with("local://") {
        "shell"
    } else if row.full_path.starts_with("ssh://") {
        "ssh_shell"
    } else if row.kind == BrowserRowKind::Session {
        "codex"
    } else {
        "session"
    }
}

fn initial_replay_commands_for_row(row: &BrowserRow) -> Vec<String> {
    if row.full_path.starts_with("codex-litellm://") {
        vec!["CODEX_HOME=\"$HOME/.codex-litellm\" codex-litellm".to_string()]
    } else if row.full_path.starts_with("codex://") || row.kind == BrowserRowKind::Session {
        vec!["codex".to_string()]
    } else if row.full_path.starts_with("local://") {
        vec!["$SHELL -l".to_string()]
    } else {
        Vec::new()
    }
}

fn new_document_virtual_path(
    selected_row: Option<&BrowserRow>,
    active_session: Option<&ManagedSessionView>,
) -> String {
    let base = selected_row
        .and_then(document_parent_base)
        .or_else(|| active_session.and_then(active_session_document_base))
        .unwrap_or_else(|| "/papers".to_string());
    format!(
        "{}/paper-{}",
        base.trim_end_matches('/'),
        unique_workspace_leaf_suffix()
    )
}

fn new_document_virtual_path_for_row(row: &BrowserRow) -> String {
    let base = document_parent_base(row).unwrap_or_else(|| "/papers".to_string());
    format!(
        "{}/paper-{}",
        base.trim_end_matches('/'),
        unique_workspace_leaf_suffix()
    )
}

fn new_group_virtual_path_for_row(row: &BrowserRow) -> String {
    let base = match row.kind {
        BrowserRowKind::Group if row.full_path == "local" => "/workspace".to_string(),
        BrowserRowKind::Group if row.full_path.starts_with("__live_") => "/workspace".to_string(),
        BrowserRowKind::Group => row.full_path.clone(),
        BrowserRowKind::Separator => {
            parent_virtual_path(&row.full_path).unwrap_or_else(|| "/workspace".to_string())
        }
        BrowserRowKind::Session => row
            .session_cwd
            .clone()
            .unwrap_or_else(|| "/workspace".to_string()),
        BrowserRowKind::Document => {
            parent_virtual_path(&row.full_path).unwrap_or_else(|| "/workspace".to_string())
        }
    };
    format!(
        "{}/folder-{}",
        base.trim_end_matches('/'),
        unique_workspace_leaf_suffix()
    )
}

fn new_separator_virtual_path_for_row(row: &BrowserRow) -> String {
    let base = match row.kind {
        BrowserRowKind::Group if row.full_path == "local" => "/workspace".to_string(),
        BrowserRowKind::Group if row.full_path.starts_with("__live_") => "/workspace".to_string(),
        BrowserRowKind::Group => row.full_path.clone(),
        BrowserRowKind::Separator => {
            parent_virtual_path(&row.full_path).unwrap_or_else(|| "/workspace".to_string())
        }
        BrowserRowKind::Session => row
            .session_cwd
            .clone()
            .unwrap_or_else(|| "/workspace".to_string()),
        BrowserRowKind::Document => {
            parent_virtual_path(&row.full_path).unwrap_or_else(|| "/workspace".to_string())
        }
    };
    format!(
        "{}/separator-{}",
        base.trim_end_matches('/'),
        unique_workspace_leaf_suffix()
    )
}

fn unique_workspace_leaf_suffix() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn moved_workspace_item_virtual_path(item_row: &BrowserRow, target_group: &BrowserRow) -> String {
    let name = item_row
        .full_path
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .unwrap_or("item");
    format!("{}/{}", target_group.full_path.trim_end_matches('/'), name)
}

fn is_workspace_row(row: &BrowserRow) -> bool {
    match row.kind {
        BrowserRowKind::Document | BrowserRowKind::Separator => true,
        BrowserRowKind::Group => {
            !row.full_path.starts_with("__live_")
                && row.full_path != "local"
                && row.full_path != "/"
                && row.group_kind.is_some()
        }
        BrowserRowKind::Session => false,
    }
}

fn is_drop_target_row(row: &BrowserRow) -> bool {
    row.kind == BrowserRowKind::Group
        && row.group_kind != Some(WorkspaceGroupKind::Separator)
        && !row.full_path.starts_with("__live_")
        && row.full_path != "local"
}

fn workspace_path_contains(parent: &str, child: &str) -> bool {
    child == parent
        || child
            .strip_prefix(parent)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn valid_drop_target(drag_paths: &[String], target_row: &BrowserRow) -> bool {
    if !is_drop_target_row(target_row) || drag_paths.is_empty() {
        return false;
    }
    drag_paths.iter().all(|path| {
        path != &target_row.full_path
            && !workspace_path_contains(path, &target_row.full_path)
            && parent_virtual_path(path).as_deref() != Some(target_row.full_path.as_str())
    })
}

fn group_session_cwd(row: &BrowserRow) -> Option<String> {
    if row.full_path.starts_with("__live_") {
        return None;
    }
    if row.full_path == "local" {
        return std::env::var_os("HOME").map(|path| path.to_string_lossy().into_owned());
    }
    Some(row.full_path.clone())
}

fn group_session_title_hint(row: &BrowserRow, kind: SessionKind) -> String {
    let suffix = match kind {
        SessionKind::Codex => "codex",
        SessionKind::CodexLiteLlm => "codex-litellm",
        SessionKind::Shell => "shell",
        SessionKind::SshShell => "ssh",
        SessionKind::Document => "document",
    };
    format!("{} {}", row.label, suffix)
}

fn document_parent_base(row: &BrowserRow) -> Option<String> {
    match row.kind {
        BrowserRowKind::Session => row.session_cwd.clone(),
        BrowserRowKind::Document => parent_virtual_path(&row.full_path),
        BrowserRowKind::Separator => parent_virtual_path(&row.full_path),
        BrowserRowKind::Group => {
            if row.full_path.starts_with("__live_") {
                Some("/documents".to_string())
            } else {
                Some(row.full_path.clone())
            }
        }
    }
}

fn active_session_document_base(session: &ManagedSessionView) -> Option<String> {
    if session.kind == SessionKind::Document {
        parent_virtual_path(&session.session_path)
    } else {
        session
            .metadata
            .iter()
            .find(|entry| entry.label == "Cwd")
            .map(|entry| entry.value.clone())
    }
}

fn parent_virtual_path(path: &str) -> Option<String> {
    let normalized = path.trim_end_matches('/');
    let parent = normalized.rsplit_once('/')?.0;
    if parent.is_empty() {
        Some("/documents".to_string())
    } else {
        Some(parent.to_string())
    }
}

#[derive(Clone)]
struct GhosttyDockRequest {
    pid: Option<u32>,
    known_window_id: Option<String>,
    host_token: Option<String>,
    rect: DockRect,
    retry_budget: u8,
}

impl GhosttyDockRequest {
    fn signature(&self) -> DockSignature {
        DockSignature {
            pid: self.pid,
            host_token: self.host_token.clone(),
            rect: self.rect,
        }
    }
}

fn spawn_dock_sync(mut state: Signal<ShellState>, request: GhosttyDockRequest) {
    if state.read().dock_sync_in_flight {
        return;
    }

    state.with_mut(|shell| shell.dock_sync_in_flight = true);
    spawn(async move {
        let request_for_task = request.clone();
        let outcome = task::spawn_blocking(move || {
            yggterm_platform::sync_docked_ghostty_window(
                request_for_task.pid,
                request_for_task.known_window_id.as_deref(),
                request_for_task.host_token.as_deref(),
                request_for_task.rect,
            )
        })
        .await;

        let mut retry_request = None;
        state.with_mut(|shell| {
            shell.dock_sync_in_flight = false;
            match outcome {
                Ok(Ok(window)) => {
                    let first_dock =
                        shell.docked_window_id.as_deref() != Some(window.window_id.as_str());
                    shell.docked_window_id = Some(window.window_id.clone());
                    shell.last_dock_signature = Some(request.signature());
                    if first_dock {
                        shell.last_action = format!("ghostty docked {}", window.window_id);
                    }
                }
                Ok(Err(error)) => {
                    shell.last_dock_signature = None;
                    if is_transient_dock_error(&error.to_string()) {
                        retry_request = Some(request.clone());
                    } else {
                        shell.last_action = format!("ghostty dock failed: {error}");
                    }
                }
                Err(error) => {
                    shell.last_action = format!("ghostty dock task failed: {error}");
                }
            }
        });
        if let Some(request) = retry_request {
            spawn_dock_retry(state, request);
        }
    });
}

fn spawn_dock_retry(state: Signal<ShellState>, request: GhosttyDockRequest) {
    if request.retry_budget == 0 {
        return;
    }

    spawn(async move {
        let _ = task::spawn_blocking(|| std::thread::sleep(Duration::from_millis(250))).await;
        let should_retry = {
            let shell = state.read();
            if shell.server.active_view_mode() != WorkspaceViewMode::Terminal {
                false
            } else {
                shell
                    .server
                    .active_session()
                    .map(|session| {
                        session.backend == TerminalBackend::Ghostty
                            && session.terminal_process_id == request.pid
                    })
                    .unwrap_or(false)
            }
        };

        if should_retry {
            let mut next = request.clone();
            next.retry_budget = next.retry_budget.saturating_sub(1);
            spawn_dock_sync(state, next);
        }
    });
}

fn spawn_dock_hide(mut state: Signal<ShellState>, pid: Option<u32>, window_id: String) {
    if state.read().dock_sync_in_flight {
        return;
    }

    state.with_mut(|shell| shell.dock_sync_in_flight = true);
    spawn(async move {
        let outcome = task::spawn_blocking(move || {
            yggterm_platform::hide_docked_ghostty_window(pid, &window_id)
        })
        .await;
        state.with_mut(|shell| {
            shell.dock_sync_in_flight = false;
            shell.docked_window_id = None;
            shell.last_dock_signature = None;
            if let Ok(Err(error)) = outcome
                && !is_transient_dock_error(&error.to_string())
            {
                shell.last_action = format!("ghostty hide failed: {error}");
            }
        });
    });
}

fn is_transient_dock_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("not found yet")
        || lower.contains("returned no window")
        || lower.contains("no window id")
        || lower.contains("not installed on path")
}

fn ghostty_dock_request(
    desktop: &dioxus::desktop::DesktopContext,
    snapshot: &RenderSnapshot,
) -> Option<GhosttyDockRequest> {
    if !cfg!(target_os = "linux") || snapshot.active_view_mode != WorkspaceViewMode::Terminal {
        return None;
    }

    let session = snapshot.active_session.as_ref()?;
    if session.backend != TerminalBackend::Ghostty {
        return None;
    }
    if session.terminal_host_mode != GhosttyTerminalHostMode::ControlledDock {
        return None;
    }

    let pid = session.terminal_process_id?;
    let inner = desktop.inner_size();
    let scale = desktop.scale_factor();
    let left_sidebar = if snapshot.sidebar_open {
        SIDE_RAIL_WIDTH as f64
    } else {
        0.0
    };
    let right_sidebar = if snapshot.right_panel_mode != RightPanelMode::Hidden {
        SIDE_RAIL_WIDTH as f64
    } else {
        0.0
    };
    let titlebar_height = 44.0;
    let top_padding = 12.0;
    let right_padding = 12.0;
    let bottom_padding = 10.0;
    let frame_inset = 1.0;

    let x = ((left_sidebar + frame_inset) * scale).round() as i32;
    let y = ((titlebar_height + top_padding + frame_inset) * scale).round() as i32;
    let width = inner.width.saturating_sub(
        ((left_sidebar + right_sidebar + right_padding + frame_inset * 2.0) * scale).round() as u32,
    );
    let height = inner.height.saturating_sub(
        ((titlebar_height + top_padding + bottom_padding + frame_inset * 2.0) * scale).round()
            as u32,
    );

    if width < 240 || height < 180 {
        return None;
    }

    Some(GhosttyDockRequest {
        pid: Some(pid),
        known_window_id: session.terminal_window_id.clone(),
        host_token: session.terminal_host_token.clone(),
        rect: DockRect {
            x,
            y,
            width,
            height,
        },
        retry_budget: 16,
    })
}

pub fn launch_shell(bootstrap: ShellBootstrap) -> Result<()> {
    let _ = BOOTSTRAP.set(bootstrap);

    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            Config::new().with_window(
                WindowBuilder::new()
                    .with_title("yggterm")
                    .with_window_icon(Some(window_icon::load_window_icon()))
                    .with_transparent(true)
                    .with_decorations(false)
                    .with_resizable(true)
                    .with_inner_size(LogicalSize::new(1460.0, 920.0))
                    .with_min_inner_size(LogicalSize::new(1024.0, 720.0)),
            ),
        )
        .launch(app);

    Ok(())
}

fn app() -> Element {
    let bootstrap = BOOTSTRAP
        .get()
        .expect("shell bootstrap not initialized")
        .clone();
    let mut state = use_signal(|| ShellState::new(bootstrap));
    let desktop = use_window();
    let mut hovered = use_signal(|| None::<HoveredControl>);
    let mut startup_sync_started = use_signal(|| false);
    let mut update_check_started = use_signal(|| false);
    let mut update_restart_started = use_signal(|| false);
    let mut dock_pulse_started = use_signal(|| false);
    let mut window_epoch = use_signal(|| 0_u64);
    use_effect(move || {
        if XTERM_ASSETS_BOOTSTRAPPED.get().is_none() {
            let _ = XTERM_ASSETS_BOOTSTRAPPED.set(());
            let _ = document::eval(&xterm_assets_bootstrap_script());
        }
    });
    use_wry_event_handler(move |event, _| {
        if let TaoEvent::WindowEvent { event, .. } = event {
            match event {
                DesktopWindowEvent::Moved(_)
                | DesktopWindowEvent::Resized(_)
                | DesktopWindowEvent::Focused(_)
                | DesktopWindowEvent::ScaleFactorChanged { .. } => {
                    window_epoch.with_mut(|epoch| *epoch += 1);
                }
                _ => {}
            }
        }
    });
    use_effect(move || {
        let should_start = {
            let shell = state.read();
            shell.needs_initial_server_sync && !*startup_sync_started.read()
        };
        if should_start {
            startup_sync_started.set(true);
            spawn_initial_server_sync(state);
        }
    });
    use_effect(move || {
        if *update_check_started.read() {
            return;
        }
        let should_start = {
            let shell = state.read();
            shell.bootstrap.install_context.update_policy == yggterm_core::UpdatePolicy::NotifyOnly
                && shell.bootstrap.install_context.channel != yggterm_core::InstallChannel::Unknown
        };
        if should_start {
            update_check_started.set(true);
            spawn_notify_only_update_check(state);
        }
    });
    use_effect(move || {
        if *update_restart_started.read() {
            return;
        }
        let pending = state.read().pending_update_restart.clone();
        let Some(update) = pending else {
            return;
        };
        update_restart_started.set(true);
        spawn(async move {
            sleep(Duration::from_millis(1800)).await;
            let next_exe = update.executable.clone();
            let next_version = update.version.clone();
            let launched = task::spawn_blocking(move || Command::new(&next_exe).spawn()).await;
            state.with_mut(|shell| match launched {
                Ok(Ok(_)) => {
                    shell.last_action = format!("restarting into {}", next_version);
                    window().close();
                }
                Ok(Err(error)) => {
                    shell.pending_update_restart = None;
                    shell.last_action = format!("restart failed: {error}");
                    shell.push_notification(
                        NotificationTone::Error,
                        "Update Restart Failed",
                        error.to_string(),
                    );
                }
                Err(error) => {
                    shell.pending_update_restart = None;
                    shell.last_action = format!("restart task failed: {error}");
                    shell.push_notification(
                        NotificationTone::Error,
                        "Update Restart Failed",
                        error.to_string(),
                    );
                }
            });
        });
    });
    use_effect(move || {
        let active = state.read().server.active_session().cloned();
        let Some(session) = active else {
            return;
        };
        let has_precis = state
            .read()
            .generated_precis
            .contains_key(&session.session_path);
        if has_precis {
            return;
        }
        spawn_precis_generation(state, session);
    });
    use_effect(move || {
        if *dock_pulse_started.read() {
            return;
        }
        dock_pulse_started.set(true);
        spawn(async move {
            loop {
                let active = {
                    let shell = state.read();
                    shell.server.active_view_mode() == WorkspaceViewMode::Terminal
                        || shell.docked_window_id.is_some()
                };
                if active {
                    window_epoch.with_mut(|epoch| *epoch += 1);
                }
                let _ = task::spawn_blocking(|| thread::sleep(Duration::from_millis(350))).await;
            }
        });
    });
    use_effect(move || {
        let _ = *window_epoch.read();
        let snapshot = state.read().snapshot();
        let request = ghostty_dock_request(&desktop, &snapshot);
        let (should_sync, window_to_hide) = {
            let shell = state.read();
            let should_sync = request.as_ref().is_some_and(|request| {
                shell.docked_window_id.is_none()
                    || shell.last_dock_signature.as_ref() != Some(&request.signature())
            });
            let window_to_hide = if request.is_none() {
                shell.docked_window_id.clone().map(|window_id| {
                    (
                        shell
                            .server
                            .active_session()
                            .and_then(|session| session.terminal_process_id),
                        window_id,
                    )
                })
            } else {
                None
            };
            (should_sync, window_to_hide)
        };
        if let Some(request) = request
            && should_sync
        {
            spawn_dock_sync(state, request);
        } else if let Some((pid, window_id)) = window_to_hide {
            spawn_dock_hide(state, pid, window_id);
        }
    });
    let snapshot = state.read().snapshot();
    let titlebar_snapshot = snapshot.clone();
    let sidebar_snapshot = snapshot.clone();
    let main_snapshot = snapshot.clone();
    let metadata_snapshot = snapshot.clone();
    let preferred_agent_kind = preferred_agent_session_kind(&snapshot.settings);
    let maximized = snapshot.maximized;
    let shell_radius = if maximized { 0 } else { 11 };

    rsx! {
        div {
            style: format!(
                "position: fixed; inset: 0; overflow: hidden; border-radius:{}px; \
                 background-color:{}; background-image:{}; backdrop-filter: blur(30px) saturate(165%); \
                 -webkit-backdrop-filter: blur(30px) saturate(165%);",
                shell_radius, snapshot.palette.shell, snapshot.palette.gradient
            ),
            oncontextmenu: |evt| {
                evt.prevent_default();
                evt.stop_propagation();
            },
            style { "{TOAST_CSS}" }
            div {
                style: shell_style(snapshot.palette, shell_radius),
                WindowResizeHandles {}
                Titlebar {
                    snapshot: titlebar_snapshot,
                    hovered: hovered,
                    on_toggle_sidebar: move || state.with_mut(|shell| shell.toggle_sidebar()),
                    on_search: move |value: String| state.with_mut(|shell| shell.set_search(value)),
                    on_hover_control: move |control: Option<HoveredControl>| hovered.set(control),
                    on_set_view_mode: move |mode: WorkspaceViewMode| spawn_set_view_mode(state, mode),
                    on_toggle_meta: move || state.with_mut(|shell| shell.toggle_metadata_panel()),
                    on_toggle_settings: move || state.with_mut(|shell| shell.toggle_settings_panel()),
                    on_toggle_connect: move || state.with_mut(|shell| shell.toggle_connect_panel()),
                    on_toggle_notifications: move || state.with_mut(|shell| shell.toggle_notifications_panel()),
                    on_toggle_maximized: move || state.with_mut(|shell| shell.toggle_maximized()),
                    on_toggle_always_on_top: move || state.with_mut(|shell| shell.toggle_always_on_top()),
                    maximized: maximized,
                }
                div {
                    style: "display: flex; flex: 1; min-height: 0; overflow: hidden;",
                    Sidebar {
                        snapshot: sidebar_snapshot,
                        on_start_session: move |_| spawn_server_snapshot_action(
                            state,
                            "starting session".to_string(),
                            move |endpoint| start_local_session(&endpoint, preferred_agent_kind),
                        ),
                        on_start_terminal: move |_| spawn_server_snapshot_action(
                            state,
                            "starting terminal".to_string(),
                            move |endpoint| start_local_session(&endpoint, SessionKind::Shell),
                        ),
                        on_create_paper: move |_| queue_new_document(state),
                        on_select_row: move |(row, extend_range): (BrowserRow, bool)| {
                            state.with_mut(|shell| shell.select_tree_row(&row, extend_range));
                            if extend_range && is_workspace_row(&row) {
                                return;
                            }
                            if is_live_sidebar_row(&row) {
                                spawn_server_snapshot_action(
                                    state,
                                    format!("focusing {}", row.label),
                                    move |endpoint| focus_live(&endpoint, &row.full_path),
                                );
                                return;
                            }
                            let should_generate = row.kind == BrowserRowKind::Session && row.session_title.is_none();
                            match row.kind {
                                BrowserRowKind::Group | BrowserRowKind::Separator => state.with_mut(|shell| shell.select_row(&row)),
                                BrowserRowKind::Session | BrowserRowKind::Document => spawn_open_session_row(state, row.clone()),
                            }
                            if should_generate {
                                queue_title_generation(state, row.clone(), false);
                            }
                        },
                        on_delete_selected_items: move |hard_delete: bool| queue_delete_selected_items(state, hard_delete),
                        on_open_context_menu: move |(row, position): (BrowserRow, (f64, f64))| {
                            state.with_mut(|shell| {
                                if !shell.selected_tree_paths.contains(&row.full_path) {
                                    shell.select_tree_row(&row, false);
                                }
                                shell.open_context_menu(row, position)
                            })
                        },
                        on_start_drag: move |row: BrowserRow| state.with_mut(|shell| shell.begin_drag(&row)),
                        on_drag_hover: move |row: BrowserRow| state.with_mut(|shell| shell.set_drag_hover_target(&row)),
                        on_drag_leave: move |row: BrowserRow| state.with_mut(|shell| {
                            if shell.drag_hover_target.as_deref() == Some(row.full_path.as_str()) {
                                shell.drag_hover_target = None;
                            }
                        }),
                        on_drop_into_row: move |row: BrowserRow| queue_move_selected_items_to_group(state, row),
                        on_end_drag: move |_| state.with_mut(|shell| shell.clear_drag_state()),
                        on_begin_rename: move |row: BrowserRow| state.with_mut(|shell| shell.begin_tree_rename(&row)),
                        on_update_rename: move |value: String| state.with_mut(|shell| shell.update_tree_rename_value(value)),
                        on_commit_rename: move |row: BrowserRow| {
                            let label = state.read().tree_rename_value.clone();
                            queue_tree_rename(state, row, label);
                        },
                        on_cancel_rename: move |_| state.with_mut(|shell| shell.cancel_tree_rename()),
                    }
                    MainSurface {
                        snapshot: main_snapshot,
                        on_expand_preview: move || spawn_server_snapshot_action(
                            state,
                            "expanding preview".to_string(),
                            |endpoint| set_all_preview_blocks_folded(&endpoint, false),
                        ),
                        on_collapse_preview: move || spawn_server_snapshot_action(
                            state,
                            "collapsing preview".to_string(),
                            |endpoint| set_all_preview_blocks_folded(&endpoint, true),
                        ),
                        on_toggle_preview_block: move |ix: usize| {
                            state.with_mut(|shell| {
                                shell.server.toggle_preview_block(ix);
                                shell.server_busy = true;
                            });
                            spawn_server_snapshot_action(
                                state,
                                format!("toggling preview block {}", ix + 1),
                                move |endpoint| daemon_toggle_preview_block(&endpoint, ix),
                            )
                        },
                        on_set_preview_layout: move |mode: PreviewLayoutMode| state.with_mut(|shell| shell.set_preview_layout(mode)),
                        on_save_document: move |(path, input): (String, WorkspaceDocumentInput)| {
                            queue_document_save(state, path, input, AfterSaveAction::SaveOnly)
                        },
                        on_run_recipe_document: move |(path, input, run_new_session): (String, WorkspaceDocumentInput, bool)| {
                            queue_document_save(
                                state,
                                path,
                                input,
                                if run_new_session {
                                    AfterSaveAction::RunNewSession
                                } else {
                                    AfterSaveAction::RunHere
                                },
                            )
                        },
                        on_switch_agent_mode: move |kind: SessionKind| {
                            let (active_session, active_path) = {
                                let shell = state.read();
                                (
                                    shell.server.active_session().cloned(),
                                    shell.server.active_session_path().map(ToOwned::to_owned),
                                )
                            };
                            if let (Some(session), Some(active_path)) = (active_session, active_path) {
                                if session.kind != kind {
                                    state.with_mut(|shell| {
                                        shell.settings.default_agent_profile = match kind {
                                            SessionKind::CodexLiteLlm => AgentSessionProfile::CodexLiteLlm,
                                            _ => AgentSessionProfile::Codex,
                                        };
                                        shell.persist_settings();
                                    });
                                    spawn_switch_agent_session_mode(state, active_path, kind);
                                }
                            }
                        },
                    }
                    RightRail {
                        snapshot: metadata_snapshot,
                        on_endpoint_change: move |value: String| state.with_mut(|shell| shell.update_litellm_endpoint(value)),
                        on_api_key_change: move |value: String| state.with_mut(|shell| shell.update_litellm_api_key(value)),
                        on_model_change: move |value: String| state.with_mut(|shell| shell.update_interface_llm_model(value)),
                        on_set_notification_delivery: move |mode: NotificationDeliveryMode| {
                            state.with_mut(|shell| shell.update_notification_delivery(mode))
                        },
                        on_set_notification_sound: move |enabled: bool| {
                            state.with_mut(|shell| shell.update_notification_sound(enabled))
                        },
                        on_generate_titles: move |_| state.with_mut(|shell| shell.generate_session_titles()),
                        on_adjust_ui_zoom: move |delta: i32| state.with_mut(|shell| shell.adjust_ui_zoom(delta)),
                        on_adjust_main_zoom: move |delta: i32| {
                            state.with_mut(|shell| shell.adjust_main_zoom(delta));
                            apply_active_terminal_zoom(state);
                        },
                        on_connect_ssh_custom: move |_| spawn_connect_ssh_custom(state),
                        on_ssh_target_change: move |value: String| state.with_mut(|shell| shell.update_ssh_connect_target(value)),
                        on_ssh_prefix_change: move |value: String| state.with_mut(|shell| shell.update_ssh_connect_prefix(value)),
                        on_clear_notification: move |id: u64| state.with_mut(|shell| shell.clear_notification(id)),
                        on_clear_notifications: move |_| state.with_mut(|shell| shell.clear_notifications()),
                    }
                }
                if let Some(row) = snapshot.context_menu_row.clone() {
                    ContextMenuOverlay {
                        row: row.clone(),
                        position: snapshot.context_menu_position.unwrap_or((18.0, 60.0)),
                        selected_row: snapshot.selected_row.clone(),
                        selected_tree_paths: snapshot.selected_tree_paths.clone(),
                        palette: snapshot.palette,
                        on_close: move |_| state.with_mut(|shell| shell.close_context_menu()),
                        on_create_group_codex: {
                            let row = row.clone();
                            let preferred_agent_kind = preferred_agent_kind;
                            move |_| {
                                spawn_start_group_session(state, row.clone(), preferred_agent_kind)
                            }
                        },
                        on_create_group: {
                            let row = row.clone();
                            move |_| queue_new_group_for_row(state, row.clone())
                        },
                        on_create_group_shell: {
                            let row = row.clone();
                            move |_| spawn_start_group_session(state, row.clone(), SessionKind::Shell)
                        },
                        on_create_group_document: {
                            let row = row.clone();
                            move |_| queue_new_document_for_row(state, row.clone())
                        },
                        on_create_group_recipe: {
                            let row = row.clone();
                            move |_| queue_new_separator_for_row(state, row.clone())
                        },
                        on_move_selected_document_here: {
                            let row = row.clone();
                            move |_| queue_move_selected_items_to_group(state, row.clone())
                        },
                        on_create_note: {
                            let row = row.clone();
                            move |_| {
                                state.with_mut(|shell| shell.close_context_menu());
                                queue_session_note_creation(state, row.clone());
                            }
                        },
                        on_create_recipe: {
                            let row = row.clone();
                            move |_| {
                                state.with_mut(|shell| shell.close_context_menu());
                                queue_session_recipe_creation(state, row.clone());
                            }
                        },
                        on_regenerate: {
                            let row = row.clone();
                            move |_| {
                                state.with_mut(|shell| shell.regenerate_title_for_row(&row));
                                queue_title_generation(state, row.clone(), true);
                            }
                        },
                        on_delete_item: {
                            let row = row.clone();
                            move |_| {
                                state.with_mut(|shell| {
                                    shell.select_tree_row(&row, false);
                                    shell.open_delete_dialog(false);
                                });
                            }
                        },
                    }
                }
                if let Some(pending_delete) = snapshot.pending_delete.clone() {
                    DeleteConfirmOverlay {
                        pending: pending_delete,
                        palette: snapshot.palette,
                        on_cancel: move |_| state.with_mut(|shell| shell.cancel_delete_dialog()),
                        on_confirm: move |_| queue_delete_selected_items(state, true),
                    }
                }
                if !snapshot.notifications.is_empty() {
                    ToastViewport {
                        notifications: snapshot.notifications.clone(),
                        palette: snapshot.palette,
                        right_inset: toast_right_inset(snapshot.right_panel_mode),
                        on_clear_notification: move |id: u64| state.with_mut(|shell| shell.clear_notification(id)),
                    }
                }
            }
        }
    }
}

#[component]
fn Titlebar(
    snapshot: RenderSnapshot,
    hovered: Signal<Option<HoveredControl>>,
    on_toggle_sidebar: EventHandler<()>,
    on_search: EventHandler<String>,
    on_hover_control: EventHandler<Option<HoveredControl>>,
    on_set_view_mode: EventHandler<WorkspaceViewMode>,
    on_toggle_meta: EventHandler<()>,
    on_toggle_settings: EventHandler<()>,
    on_toggle_connect: EventHandler<()>,
    on_toggle_notifications: EventHandler<()>,
    on_toggle_maximized: EventHandler<()>,
    on_toggle_always_on_top: EventHandler<()>,
    maximized: bool,
) -> Element {
    let mut drag_armed = use_signal(|| false);
    rsx! {
        div {
            style: format!(
                "display:flex; align-items:center; justify-content:space-between; height:44px; \
                 padding:0 12px; background:{}; zoom:{}%; user-select:none; -webkit-user-select:none;",
                snapshot.palette.titlebar,
                zoom_percent_f32(snapshot.settings.ui_font_size, 14.0)
            ),
            onmousedown: move |_| drag_armed.set(true),
            onmouseup: move |_| drag_armed.set(false),
            onmouseleave: move |_| drag_armed.set(false),
            onmousemove: move |_| {
                if drag_armed() {
                    drag_armed.set(false);
                    window().drag();
                }
            },
            ondoubleclick: move |_| {
                drag_armed.set(false);
                on_toggle_maximized.call(());
            },
            div {
                style: "display:flex; align-items:center; gap:12px; width:340px; min-width:340px;",
                button {
                    style: icon_button_style(snapshot.palette),
                    onmousedown: |evt| evt.stop_propagation(),
                    ondoubleclick: |evt| evt.stop_propagation(),
                    onclick: move |_| on_toggle_sidebar.call(()),
                    "☰"
                }
                div {
                    style: toggle_slider_style(snapshot.palette),
                    onmousedown: |evt| evt.stop_propagation(),
                    button {
                        style: toggle_slider_end_style(
                            snapshot.palette,
                            snapshot.active_view_mode == WorkspaceViewMode::Rendered
                        ),
                        ondoubleclick: |evt| evt.stop_propagation(),
                        onclick: move |_| on_set_view_mode.call(WorkspaceViewMode::Rendered),
                        "Preview"
                    }
                    button {
                        style: toggle_slider_end_style(
                            snapshot.palette,
                            snapshot.active_view_mode == WorkspaceViewMode::Terminal
                        ),
                        ondoubleclick: |evt| evt.stop_propagation(),
                        onclick: move |_| on_set_view_mode.call(WorkspaceViewMode::Terminal),
                        "Terminal"
                    }
                }
                div {
                    style: "flex:1; min-width:56px; height:100%;",
                    ondoubleclick: move |_| {
                        drag_armed.set(false);
                        on_toggle_maximized.call(());
                    },
                }
            }
            div {
                style: "flex:1; display:flex; align-items:center; justify-content:center; gap:10px; padding:0 16px;",
                div {
                    style: "flex:1; min-width:84px; height:100%;",
                    ondoubleclick: move |_| {
                        drag_armed.set(false);
                        on_toggle_maximized.call(());
                    },
                }
                input {
                    r#type: "text",
                    value: "{snapshot.search_query}",
                    placeholder: "Search sessions…",
                    style: search_style(snapshot.palette),
                    onmousedown: |evt| evt.stop_propagation(),
                    ondoubleclick: |evt| evt.stop_propagation(),
                    oninput: move |evt| on_search.call(evt.value()),
                }
                if let Some(update) = snapshot.pending_update_restart.clone() {
                    div {
                        style: format!(
                            "display:inline-flex; align-items:center; gap:6px; height:28px; padding:0 11px; border-radius:10px; \
                             background:rgba(255,255,255,0.82); color:{}; font-size:11px; font-weight:700; \
                             box-shadow: inset 0 0 0 1px rgba(170,190,212,0.24); white-space:nowrap;",
                            snapshot.palette.accent
                        ),
                        "Updating to {update.version}…"
                    }
                }
                div {
                    style: "flex:1; min-width:84px; height:100%;",
                    ondoubleclick: move |_| {
                        drag_armed.set(false);
                        on_toggle_maximized.call(());
                    },
                }
            }
            div {
                style: "display:flex; align-items:center; justify-content:flex-end; gap:10px; width:372px; min-width:372px;",
                button {
                    style: connect_button_style(
                        snapshot.palette,
                        snapshot.right_panel_mode == RightPanelMode::Connect
                    ),
                    onmousedown: |evt| evt.stop_propagation(),
                    ondoubleclick: |evt| evt.stop_propagation(),
                    onclick: move |_| on_toggle_connect.call(()),
                    "Connect SSH"
                }
                button {
                    style: utility_icon_style(
                        snapshot.palette,
                        snapshot.right_panel_mode == RightPanelMode::Notifications
                    ),
                    onmousedown: |evt| evt.stop_propagation(),
                    ondoubleclick: |evt| evt.stop_propagation(),
                    onclick: move |_| on_toggle_notifications.call(()),
                    BellIcon {}
                }
                button {
                    style: utility_icon_style_sized(
                        snapshot.palette,
                        snapshot.right_panel_mode == RightPanelMode::Settings,
                        17
                    ),
                    onmousedown: |evt| evt.stop_propagation(),
                    ondoubleclick: |evt| evt.stop_propagation(),
                    onclick: move |_| on_toggle_settings.call(()),
                    "⚙"
                }
                button {
                    style: utility_icon_style(
                        snapshot.palette,
                        snapshot.right_panel_mode == RightPanelMode::Metadata
                    ),
                    onmousedown: |evt| evt.stop_propagation(),
                    ondoubleclick: |evt| evt.stop_propagation(),
                    onclick: move |_| on_toggle_meta.call(()),
                    "ⓘ"
                }
                div {
                    style: "flex:1; min-width:48px; height:100%;",
                    ondoubleclick: move |_| {
                        drag_armed.set(false);
                        on_toggle_maximized.call(());
                    },
                }
                WindowControls {
                    palette: snapshot.palette,
                    hovered: hovered(),
                    on_hover_control: on_hover_control,
                    on_toggle_maximized: on_toggle_maximized,
                    on_toggle_always_on_top: on_toggle_always_on_top,
                    maximized: maximized,
                    always_on_top: snapshot.always_on_top,
                }
            }
        }
    }
}

#[component]
fn WindowControls(
    palette: Palette,
    hovered: Option<HoveredControl>,
    on_hover_control: EventHandler<Option<HoveredControl>>,
    on_toggle_maximized: EventHandler<()>,
    on_toggle_always_on_top: EventHandler<()>,
    maximized: bool,
    always_on_top: bool,
) -> Element {
    rsx! {
        div {
            style: "display:flex; align-items:stretch; gap:0;",
            WindowControl {
                icon: WindowControlIcon::AlwaysOnTop,
                hovered: hovered == Some(HoveredControl::AlwaysOnTop),
                active: always_on_top,
                hover_tone: HoveredControl::AlwaysOnTop,
                palette,
                on_hover_control,
                on_press: move |_| on_toggle_always_on_top.call(()),
            }
            WindowControl {
                icon: WindowControlIcon::Minimize,
                hovered: hovered == Some(HoveredControl::Minimize),
                active: false,
                hover_tone: HoveredControl::Minimize,
                palette,
                on_hover_control,
                on_press: move |_| window().set_minimized(true),
            }
            WindowControl {
                icon: if maximized {
                    WindowControlIcon::Restore
                } else {
                    WindowControlIcon::Maximize
                },
                hovered: hovered == Some(HoveredControl::Maximize),
                active: false,
                hover_tone: HoveredControl::Maximize,
                palette,
                on_hover_control,
                on_press: move |_| on_toggle_maximized.call(()),
            }
            WindowControl {
                icon: WindowControlIcon::Close,
                hovered: hovered == Some(HoveredControl::Close),
                active: false,
                hover_tone: HoveredControl::Close,
                palette,
                on_hover_control,
                on_press: move |_| window().close(),
            }
        }
    }
}

#[component]
fn WindowControl(
    icon: WindowControlIcon,
    hovered: bool,
    active: bool,
    hover_tone: HoveredControl,
    palette: Palette,
    on_hover_control: EventHandler<Option<HoveredControl>>,
    on_press: EventHandler<MouseEvent>,
) -> Element {
    let is_close = hover_tone == HoveredControl::Close;
    let background = if hovered {
        if is_close {
            palette.close_hover
        } else {
            palette.control_hover
        }
    } else if active {
        "transparent"
    } else {
        "transparent"
    };
    let color = if hovered && is_close {
        "#ffffff"
    } else if active {
        palette.accent
    } else {
        palette.text
    };

    rsx! {
        button {
            style: format!(
                "width:34px; height:30px; border:none; border-radius:0; background:{}; color:{}; \
                 display:flex; align-items:center; justify-content:center; font-size:13px; font-weight:600; \
                 user-select:none; -webkit-user-select:none;",
                background, color
            ),
            onmousedown: |evt| evt.stop_propagation(),
            ondoubleclick: |evt| evt.stop_propagation(),
            onmouseenter: move |_| on_hover_control.call(Some(hover_tone)),
            onmouseleave: move |_| on_hover_control.call(None),
            onclick: move |evt| on_press.call(evt),
            WindowControlGlyph { icon: icon }
        }
    }
}

#[component]
fn WindowControlGlyph(icon: WindowControlIcon) -> Element {
    match icon {
        WindowControlIcon::AlwaysOnTop => rsx! {
            svg {
                width: "12",
                height: "12",
                view_box: "0 0 12 12",
                fill: "none",
                xmlns: "http://www.w3.org/2000/svg",
                path {
                    d: "M3.1 5.2L6 2.4L8.9 5.2",
                    stroke: "currentColor",
                    stroke_width: "1.2",
                    stroke_linecap: "round",
                    stroke_linejoin: "round",
                }
                path {
                    d: "M3.1 9.2L6 6.4L8.9 9.2",
                    stroke: "currentColor",
                    stroke_width: "1.2",
                    stroke_linecap: "round",
                    stroke_linejoin: "round",
                }
            }
        },
        WindowControlIcon::Minimize => rsx! {
            svg {
                width: "11",
                height: "11",
                view_box: "0 0 10 10",
                fill: "none",
                xmlns: "http://www.w3.org/2000/svg",
                path {
                    d: "M2 5.5H8",
                    stroke: "currentColor",
                    stroke_width: "1.1",
                    stroke_linecap: "round",
                }
            }
        },
        WindowControlIcon::Maximize => rsx! {
            svg {
                width: "11",
                height: "11",
                view_box: "0 0 10 10",
                fill: "none",
                xmlns: "http://www.w3.org/2000/svg",
                rect {
                    x: "2.1",
                    y: "2.1",
                    width: "5.8",
                    height: "5.8",
                    stroke: "currentColor",
                    stroke_width: "1.1",
                }
            }
        },
        WindowControlIcon::Restore => rsx! {
            svg {
                width: "11",
                height: "11",
                view_box: "0 0 10 10",
                fill: "none",
                xmlns: "http://www.w3.org/2000/svg",
                path {
                    d: "M3.2 2.1H7.7V6.6",
                    stroke: "currentColor",
                    stroke_width: "1.1",
                    stroke_linejoin: "round",
                }
                path {
                    d: "M2.3 3.4H6.8V7.9H2.3V3.4Z",
                    stroke: "currentColor",
                    stroke_width: "1.1",
                    stroke_linejoin: "round",
                }
            }
        },
        WindowControlIcon::Close => rsx! {
            svg {
                width: "11",
                height: "11",
                view_box: "0 0 10 10",
                fill: "none",
                xmlns: "http://www.w3.org/2000/svg",
                path {
                    d: "M2.6 2.6L7.4 7.4",
                    stroke: "currentColor",
                    stroke_width: "1.1",
                    stroke_linecap: "round",
                }
                path {
                    d: "M7.4 2.6L2.6 7.4",
                    stroke: "currentColor",
                    stroke_width: "1.1",
                    stroke_linecap: "round",
                }
            }
        },
    }
}

#[component]
fn WindowResizeHandles() -> Element {
    rsx! {
        ResizeHandle {
            style: format!("position:absolute; top:0; left:0; width:{}px; height:{}px; z-index:120; cursor:nwse-resize;", CORNER_RESIZE_HANDLE, CORNER_RESIZE_HANDLE),
            direction: ResizeDirection::NorthWest,
        }
        ResizeHandle {
            style: format!("position:absolute; top:0; right:0; width:{}px; height:{}px; z-index:120; cursor:nesw-resize;", CORNER_RESIZE_HANDLE, CORNER_RESIZE_HANDLE),
            direction: ResizeDirection::NorthEast,
        }
        ResizeHandle {
            style: format!("position:absolute; bottom:0; left:0; width:{}px; height:{}px; z-index:120; cursor:nesw-resize;", CORNER_RESIZE_HANDLE, CORNER_RESIZE_HANDLE),
            direction: ResizeDirection::SouthWest,
        }
        ResizeHandle {
            style: format!("position:absolute; bottom:0; right:0; width:{}px; height:{}px; z-index:120; cursor:nwse-resize;", CORNER_RESIZE_HANDLE, CORNER_RESIZE_HANDLE),
            direction: ResizeDirection::SouthEast,
        }
        ResizeHandle {
            style: format!("position:absolute; top:0; left:{}px; right:{}px; height:{}px; z-index:119; cursor:ns-resize;", CORNER_RESIZE_HANDLE, CORNER_RESIZE_HANDLE, EDGE_RESIZE_HANDLE),
            direction: ResizeDirection::North,
        }
        ResizeHandle {
            style: format!("position:absolute; bottom:0; left:{}px; right:{}px; height:{}px; z-index:119; cursor:ns-resize;", CORNER_RESIZE_HANDLE, CORNER_RESIZE_HANDLE, EDGE_RESIZE_HANDLE),
            direction: ResizeDirection::South,
        }
        ResizeHandle {
            style: format!("position:absolute; top:{}px; bottom:{}px; left:0; width:{}px; z-index:119; cursor:ew-resize;", CORNER_RESIZE_HANDLE, CORNER_RESIZE_HANDLE, EDGE_RESIZE_HANDLE),
            direction: ResizeDirection::West,
        }
        ResizeHandle {
            style: format!("position:absolute; top:{}px; bottom:{}px; right:0; width:{}px; z-index:119; cursor:ew-resize;", CORNER_RESIZE_HANDLE, CORNER_RESIZE_HANDLE, EDGE_RESIZE_HANDLE),
            direction: ResizeDirection::East,
        }
    }
}

#[component]
fn ResizeHandle(style: String, direction: ResizeDirection) -> Element {
    rsx! {
        div {
            style: "{style}",
            onmousedown: move |evt| {
                evt.stop_propagation();
                let _ = window().drag_resize_window(direction);
            },
            ondoubleclick: |evt| evt.stop_propagation(),
        }
    }
}

#[component]
fn Sidebar(
    snapshot: RenderSnapshot,
    on_start_session: EventHandler<MouseEvent>,
    on_start_terminal: EventHandler<MouseEvent>,
    on_create_paper: EventHandler<MouseEvent>,
    on_select_row: EventHandler<(BrowserRow, bool)>,
    on_delete_selected_items: EventHandler<bool>,
    on_open_context_menu: EventHandler<(BrowserRow, (f64, f64))>,
    on_start_drag: EventHandler<BrowserRow>,
    on_drag_hover: EventHandler<BrowserRow>,
    on_drag_leave: EventHandler<BrowserRow>,
    on_drop_into_row: EventHandler<BrowserRow>,
    on_end_drag: EventHandler<()>,
    on_begin_rename: EventHandler<BrowserRow>,
    on_update_rename: EventHandler<String>,
    on_commit_rename: EventHandler<BrowserRow>,
    on_cancel_rename: EventHandler<()>,
) -> Element {
    let width = if snapshot.sidebar_open {
        SIDE_RAIL_WIDTH
    } else {
        0
    };
    let opacity = if snapshot.sidebar_open { "1" } else { "0" };
    let translate = if snapshot.sidebar_open {
        "translateX(0)"
    } else {
        "translateX(-14px)"
    };
    rsx! {
        div {
            style: format!(
                "width:{}px; min-width:{}px; max-width:{}px; display:flex; flex-direction:column; \
                 background:{}; overflow:hidden; transition: opacity 180ms ease, transform 180ms ease; \
                 opacity:{}; transform:{}; pointer-events:{}; zoom:{}%;",
                width, width, width, snapshot.palette.sidebar, opacity, translate,
                if snapshot.sidebar_open { "auto" } else { "none" },
                zoom_percent_f32(snapshot.settings.ui_font_size, 14.0)
            ),
            tabindex: "0",
            onkeydown: move |evt| {
                if evt.key() == Key::Delete {
                    let hard_delete = evt.modifiers().contains(Modifiers::SHIFT);
                    on_delete_selected_items.call(hard_delete);
                }
            },
            div {
                style: "padding:12px 12px 0 12px; display:flex; gap:8px;",
                SidebarQuickAction {
                    label: "+Session".to_string(),
                    palette: snapshot.palette,
                    onclick: on_start_session,
                }
                SidebarQuickAction {
                    label: "+Terminal".to_string(),
                    palette: snapshot.palette,
                    onclick: on_start_terminal,
                }
                SidebarQuickAction {
                    label: "+Paper".to_string(),
                    palette: snapshot.palette,
                    onclick: on_create_paper,
                }
            }
            div {
                style: "flex:1; min-height:0; overflow:auto; padding:12px 12px 12px 12px;",
                for row in snapshot.rows.iter().cloned() {
                    {
                        let select_row = row.clone();
                        let context_row = row.clone();
                        rsx! {
                            SidebarRow {
                                row: row.clone(),
                                selected: snapshot.selected_tree_paths.iter().any(|path| path == &row.full_path)
                                    || (
                                        snapshot.selected_tree_paths.is_empty()
                                            && snapshot.selected_path.as_deref() == Some(row.full_path.as_str())
                                    ),
                                drop_hovered: snapshot.drag_hover_target.as_deref() == Some(row.full_path.as_str()),
                                dragging: snapshot.drag_paths.iter().any(|path| path == &row.full_path),
                                renaming: snapshot.tree_rename_path.as_deref() == Some(row.full_path.as_str()),
                                rename_value: snapshot.tree_rename_value.clone(),
                                palette: snapshot.palette,
                                on_select: move |evt: MouseEvent| {
                                    let extend = evt.modifiers().contains(Modifiers::SHIFT);
                                    on_select_row.call((select_row.clone(), extend));
                                },
                                on_open_context_menu: move |coords: (f64, f64)| on_open_context_menu.call((context_row.clone(), coords)),
                                on_begin_rename: {
                                    let row = row.clone();
                                    move |_| on_begin_rename.call(row.clone())
                                },
                                on_update_rename: move |value: String| on_update_rename.call(value),
                                on_commit_rename: {
                                    let row = row.clone();
                                    move |_| on_commit_rename.call(row.clone())
                                },
                                on_cancel_rename: move |_| on_cancel_rename.call(()),
                                on_start_drag: {
                                    let row = row.clone();
                                    move |_| on_start_drag.call(row.clone())
                                },
                                on_drag_hover: {
                                    let row = row.clone();
                                    move |_| on_drag_hover.call(row.clone())
                                },
                                on_drag_leave: {
                                    let row = row.clone();
                                    move |_| on_drag_leave.call(row.clone())
                                },
                                on_drop_into_row: {
                                    let row = row.clone();
                                    move |_| on_drop_into_row.call(row.clone())
                                },
                                on_end_drag: move |_| on_end_drag.call(()),
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn SidebarRow(
    row: BrowserRow,
    selected: bool,
    drop_hovered: bool,
    dragging: bool,
    renaming: bool,
    rename_value: String,
    palette: Palette,
    on_select: EventHandler<MouseEvent>,
    on_open_context_menu: EventHandler<(f64, f64)>,
    on_begin_rename: EventHandler<MouseEvent>,
    on_update_rename: EventHandler<String>,
    on_commit_rename: EventHandler<()>,
    on_cancel_rename: EventHandler<()>,
    on_start_drag: EventHandler<DragEvent>,
    on_drag_hover: EventHandler<DragEvent>,
    on_drag_leave: EventHandler<DragEvent>,
    on_drop_into_row: EventHandler<DragEvent>,
    on_end_drag: EventHandler<DragEvent>,
) -> Element {
    let indent = row.depth * 12 + 12;
    let draggable = is_workspace_row(&row);
    if row.kind == BrowserRowKind::Separator {
        return rsx! {
            button {
                style: format!(
                    "width:100%; display:flex; align-items:center; gap:10px; border:none; background:transparent; \
                     padding:8px 9px 8px {}px; margin:4px 0; opacity:{}; border-radius:12px; background:{};",
                    indent
                    , if dragging { "0.58" } else { "1" },
                    if selected { palette.accent_soft } else { "transparent" }
                ),
                draggable: draggable,
                onclick: move |evt| on_select.call(evt),
                ondoubleclick: move |evt| on_begin_rename.call(evt),
                oncontextmenu: move |evt| {
                    evt.prevent_default();
                    evt.stop_propagation();
                    let coords = evt.client_coordinates();
                    on_open_context_menu.call((coords.x, coords.y));
                },
                ondragstart: move |evt| on_start_drag.call(evt),
                ondragover: move |evt| {
                    evt.prevent_default();
                    on_drag_hover.call(evt);
                },
                ondragleave: move |evt| on_drag_leave.call(evt),
                ondrop: move |evt| {
                    evt.prevent_default();
                    on_drop_into_row.call(evt);
                },
                ondragend: move |evt| on_end_drag.call(evt),
                div {
                    style: format!(
                        "flex:1; height:{}px; background:{}; opacity:{};",
                        if drop_hovered { 2 } else { 1 },
                        if drop_hovered || selected { palette.accent } else { palette.border },
                        if drop_hovered || selected { "0.96" } else { "0.72" }
                    ),
                }
                if renaming {
                    input {
                        style: format!(
                            "width:140px; height:28px; border:none; border-radius:10px; background:rgba(255,255,255,0.92); \
                             color:{}; font-size:11px; font-weight:600; padding:0 10px; box-shadow: inset 0 0 0 1px rgba(204,214,224,0.9);",
                            palette.text
                        ),
                        value: rename_value,
                        onmounted: move |evt| async move {
                            let _ = evt.set_focus(true).await;
                        },
                        oninput: move |evt| on_update_rename.call(evt.value()),
                        onkeydown: move |evt| {
                            if evt.key() == Key::Enter {
                                on_commit_rename.call(());
                            } else if evt.key() == Key::Escape {
                                on_cancel_rename.call(());
                            }
                        },
                        onblur: move |_| on_commit_rename.call(()),
                        onclick: |evt| evt.stop_propagation(),
                        onmousedown: |evt| evt.stop_propagation(),
                        oncontextmenu: |evt| {
                            evt.prevent_default();
                            evt.stop_propagation();
                        },
                    }
                } else {
                    span {
                        style: format!(
                            "font-size:10.5px; font-weight:700; letter-spacing:0.04em; color:{}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;",
                            if drop_hovered || selected { palette.accent } else { palette.muted }
                        ),
                        "{row.label}"
                    }
                }
                div {
                    style: format!(
                        "flex:1; height:{}px; background:{}; opacity:{};",
                        if drop_hovered { 2 } else { 1 },
                        if drop_hovered || selected { palette.accent } else { palette.border },
                        if drop_hovered || selected { "0.96" } else { "0.72" }
                    ),
                }
            }
        };
    }
    let background = if drop_hovered {
        "rgba(95, 168, 255, 0.14)"
    } else if selected {
        palette.accent_soft
    } else if row.kind == BrowserRowKind::Group && row.depth == 0 {
        palette.panel_alt
    } else {
        "transparent"
    };
    let icon_color = if row.kind == BrowserRowKind::Group && row.depth == 0 && row.expanded {
        palette.accent
    } else if selected {
        palette.text
    } else {
        palette.muted
    };
    let label_color = if row.kind == BrowserRowKind::Group && row.depth == 0 && row.expanded {
        palette.accent
    } else if selected {
        palette.text
    } else if row.kind == BrowserRowKind::Group && row.depth > 0 {
        palette.muted
    } else {
        palette.text
    };

    rsx! {
        button {
            style: format!(
                "width:100%; display:flex; flex-direction:column; align-items:stretch; gap:2px; \
                 border:none; border-radius:12px; background:{}; padding:6px 9px 6px {}px; margin-bottom:2px; opacity:{};",
                background, indent, if dragging { "0.58" } else { "1" }
            ),
            draggable: draggable,
            onclick: move |evt| on_select.call(evt),
            ondoubleclick: move |evt| on_begin_rename.call(evt),
            oncontextmenu: move |evt| {
                evt.prevent_default();
                evt.stop_propagation();
                let coords = evt.client_coordinates();
                on_open_context_menu.call((coords.x, coords.y));
            },
            ondragstart: move |evt| on_start_drag.call(evt),
            ondragover: move |evt| {
                evt.prevent_default();
                on_drag_hover.call(evt);
            },
            ondragleave: move |evt| on_drag_leave.call(evt),
            ondrop: move |evt| {
                evt.prevent_default();
                on_drop_into_row.call(evt);
            },
            ondragend: move |evt| on_end_drag.call(evt),
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:8px;",
                div {
                    style: "display:flex; align-items:center; gap:8px; min-width:0;",
                    div {
                        style: format!(
                            "display:inline-flex; align-items:center; justify-content:center; width:19px; min-width:19px; height:19px; color:{};",
                            icon_color
                        ),
                        TreeIcon { row: row.clone() }
                    }
                    if renaming {
                        input {
                            style: format!(
                                "flex:1; min-width:0; height:28px; border:none; border-radius:10px; background:rgba(255,255,255,0.92); \
                                 color:{}; font-size:11px; font-weight:600; padding:0 10px; box-shadow: inset 0 0 0 1px rgba(204,214,224,0.9);",
                                palette.text
                            ),
                            value: rename_value,
                            onmounted: move |evt| async move {
                                let _ = evt.set_focus(true).await;
                            },
                            oninput: move |evt| on_update_rename.call(evt.value()),
                            onkeydown: move |evt| {
                                if evt.key() == Key::Enter {
                                    on_commit_rename.call(());
                                } else if evt.key() == Key::Escape {
                                    on_cancel_rename.call(());
                                }
                            },
                            onblur: move |_| on_commit_rename.call(()),
                            onclick: |evt| evt.stop_propagation(),
                            onmousedown: |evt| evt.stop_propagation(),
                            oncontextmenu: |evt| {
                                evt.prevent_default();
                                evt.stop_propagation();
                            },
                        }
                    } else {
                        span {
                            style: format!(
                                "font-size:11px; color:{}; font-weight:{}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;",
                                label_color,
                                if row.kind == BrowserRowKind::Group && row.depth == 0 { 600 } else { 500 }
                            ),
                            "{row.label}"
                        }
                    }
                }
                if row.kind == BrowserRowKind::Group {
                    span {
                        style: format!("font-size:10px; color:{};", palette.muted),
                        "{row.descendant_sessions}"
                    }
                }
            }
            if row.kind == BrowserRowKind::Group && !row.detail_label.is_empty() {
                div {
                    style: format!(
                        "font-size:10px; color:{}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;",
                        palette.muted
                    ),
                    "{row.detail_label}"
                }
            }
        }
    }
}

#[component]
fn SidebarQuickAction(
    label: String,
    palette: Palette,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        button {
            style: format!(
                "flex:1; height:28px; border:none; border-radius:10px; background:{}; color:{}; \
                 font-size:11px; font-weight:700; white-space:nowrap;",
                palette.panel_alt, palette.text
            ),
            onclick: move |evt| onclick.call(evt),
            "{label}"
        }
    }
}

#[component]
fn TreeIcon(row: BrowserRow) -> Element {
    if row.kind == BrowserRowKind::Separator {
        return rsx! {
            span {
                style: "display:inline-flex; align-items:center; justify-content:center; font-size:12px; font-weight:700; line-height:1;",
                "—"
            }
        };
    }

    if row.kind == BrowserRowKind::Session {
        if row.full_path.starts_with("codex-litellm://") {
            return rsx! {
                span {
                    style: "display:inline-flex; align-items:center; justify-content:center; font-size:13px; font-weight:700; line-height:1;",
                    "✦"
                }
            };
        }
        if row.full_path.starts_with("local://") {
            return rsx! {
                span {
                    style: "display:inline-flex; align-items:center; justify-content:center; font-size:13px; font-weight:700; line-height:1;",
                    "⌘"
                }
            };
        }
        if row.full_path.starts_with("ssh://") {
            return rsx! {
                span {
                    style: "display:inline-flex; align-items:center; justify-content:center; font-size:13px; font-weight:700; line-height:1;",
                    "⇄"
                }
            };
        }
        return rsx! {
            span {
                style: "display:inline-flex; align-items:center; justify-content:center; font-size:13px; font-weight:700; line-height:1;",
                ">_"
            }
        };
    } else if row.kind == BrowserRowKind::Document {
        if row.document_kind == Some(WorkspaceDocumentKind::TerminalRecipe) {
            return rsx! {
                svg {
                    width: "18",
                    height: "18",
                    view_box: "0 0 18 18",
                    fill: "none",
                    xmlns: "http://www.w3.org/2000/svg",
                    rect { x: "3.2", y: "3.2", width: "11.6", height: "11.6", rx: "2.2", stroke: "currentColor", stroke_width: "1.15" }
                    path { d: "M6 7.2L8 9L6 10.8", stroke: "currentColor", stroke_width: "1.1", stroke_linecap: "round", stroke_linejoin: "round" }
                    path { d: "M9.5 10.8H12", stroke: "currentColor", stroke_width: "1.1", stroke_linecap: "round" }
                }
            };
        }
        return rsx! {
            svg {
                width: "18",
                height: "18",
                view_box: "0 0 18 18",
                fill: "none",
                xmlns: "http://www.w3.org/2000/svg",
                rect { x: "4", y: "2.75", width: "10", height: "12.5", rx: "1.6", stroke: "currentColor", stroke_width: "1.15" }
                path { d: "M6.4 6.5H11.6", stroke: "currentColor", stroke_width: "1.05", stroke_linecap: "round" }
                path { d: "M6.4 9H11.6", stroke: "currentColor", stroke_width: "1.05", stroke_linecap: "round" }
                path { d: "M6.4 11.5H10.2", stroke: "currentColor", stroke_width: "1.05", stroke_linecap: "round" }
            }
        };
    }

    if row.depth == 0 {
        if row.full_path == "__live_sessions__" {
            return rsx! {
                span {
                    style: "display:inline-flex; align-items:center; justify-content:center; font-size:13px; font-weight:700; line-height:1;",
                    "◉"
                }
            };
        }
        return rsx! {
            svg {
                width: "18",
                height: "18",
                view_box: "0 0 18 18",
                fill: "none",
                xmlns: "http://www.w3.org/2000/svg",
                rect {
                    x: "2.75",
                    y: "3.25",
                    width: "12.5",
                    height: "8.5",
                    rx: "1.4",
                    stroke: "currentColor",
                    stroke_width: "1.15",
                }
                path {
                    d: "M6.2 14.1H11.8",
                    stroke: "currentColor",
                    stroke_width: "1.15",
                    stroke_linecap: "round",
                }
                path {
                    d: "M9 11.95V14.05",
                    stroke: "currentColor",
                    stroke_width: "1.15",
                    stroke_linecap: "round",
                }
            }
        };
    }

    if row.expanded {
        rsx! {
            svg {
                width: "18",
                height: "18",
                view_box: "0 0 16 16",
                fill: "none",
                xmlns: "http://www.w3.org/2000/svg",
                path {
                    d: "M1.9 5.2C1.9 4.59249 2.39249 4.1 3 4.1H6.35L7.6 5.35H13C13.6075 5.35 14.1 5.84249 14.1 6.45V11.8C14.1 12.4075 13.6075 12.9 13 12.9H3C2.39249 12.9 1.9 12.4075 1.9 11.8V5.2Z",
                    fill: "currentColor",
                    fill_opacity: "0.84",
                    stroke_linejoin: "round",
                }
                path {
                    d: "M2.4 6.25H14.05",
                    stroke: "currentColor",
                    stroke_width: "0.95",
                    stroke_opacity: "0.18",
                }
            }
        }
    } else {
        rsx! {
            svg {
                width: "18",
                height: "18",
                view_box: "0 0 18 18",
                fill: "none",
                xmlns: "http://www.w3.org/2000/svg",
                path {
                    d: "M2.1 5.45C2.1 4.78726 2.63726 4.25 3.3 4.25H6.65L7.95 5.55H14.05C14.7127 5.55 15.25 6.08726 15.25 6.75V12.05C15.25 12.7127 14.7127 13.25 14.05 13.25H3.3C2.63726 13.25 2.1 12.7127 2.1 12.05V5.45Z",
                    stroke: "currentColor",
                    stroke_width: "1.15",
                    stroke_linejoin: "round",
                }
                path {
                    d: "M2.6 6.5H14.75",
                    stroke: "currentColor",
                    stroke_width: "1.0",
                    stroke_opacity: "0.42",
                }
            }
        }
    }
}

#[component]
fn BellIcon() -> Element {
    rsx! {
        svg {
            width: "15",
            height: "15",
            view_box: "0 0 16 16",
            fill: "none",
            xmlns: "http://www.w3.org/2000/svg",
            path {
                d: "M8 2.4C6.18 2.4 4.95 3.73 4.95 5.56V6.42C4.95 7.11 4.67 7.95 4.31 8.53L3.45 9.96C3.1 10.55 3.39 11.2 4.03 11.2H11.97C12.61 11.2 12.9 10.55 12.55 9.96L11.69 8.53C11.33 7.95 11.05 7.11 11.05 6.42V5.56C11.05 3.73 9.82 2.4 8 2.4Z",
                stroke: "currentColor",
                stroke_width: "1.15",
                stroke_linejoin: "round",
            }
            path {
                d: "M6.65 12.55C6.9 13.24 7.39 13.58 8 13.58C8.61 13.58 9.1 13.24 9.35 12.55",
                stroke: "currentColor",
                stroke_width: "1.15",
                stroke_linecap: "round",
            }
        }
    }
}

#[component]
fn MainSurface(
    snapshot: RenderSnapshot,
    on_expand_preview: EventHandler<()>,
    on_collapse_preview: EventHandler<()>,
    on_toggle_preview_block: EventHandler<usize>,
    on_set_preview_layout: EventHandler<PreviewLayoutMode>,
    on_save_document: EventHandler<(String, WorkspaceDocumentInput)>,
    on_run_recipe_document: EventHandler<(String, WorkspaceDocumentInput, bool)>,
    on_switch_agent_mode: EventHandler<SessionKind>,
) -> Element {
    let body = if let Some(session) = snapshot.active_session.clone() {
        match snapshot.active_view_mode {
            WorkspaceViewMode::Rendered => {
                if session.kind == SessionKind::Document {
                    rsx! {
                        DocumentEditor {
                            key: "{session.session_path}",
                            session: session.clone(),
                            palette: snapshot.palette,
                            server_busy: snapshot.server_busy,
                            on_save: on_save_document,
                            on_run_recipe_document,
                        }
                    }
                } else {
                    rsx! {
                        div {
                            style: "display:flex; flex-direction:column; gap:18px; min-width:0; width:min(980px, 100%); margin:0 auto;",
                            div {
                                style: "display:flex; align-items:flex-start; justify-content:space-between; gap:16px; flex-wrap:wrap;",
                                PreviewSummary { session: session.clone(), palette: snapshot.palette }
                                PreviewToolbar {
                                    palette: snapshot.palette,
                                    preview_layout: snapshot.preview_layout,
                                    server_busy: snapshot.server_busy,
                                    on_expand_preview: move |_| on_expand_preview.call(()),
                                    on_collapse_preview: move |_| on_collapse_preview.call(()),
                                    on_set_preview_layout: move |mode| on_set_preview_layout.call(mode),
                                }
                            }
                            if snapshot.preview_layout == PreviewLayoutMode::Chat {
                                div {
                                    style: "display:flex; flex-direction:column; gap:18px;",
                                    for (ix, block) in session.preview.blocks.iter().cloned().enumerate() {
                                        PreviewBlock {
                                            block_ix: ix,
                                            block: block.clone(),
                                            palette: snapshot.palette,
                                            on_toggle: move |_| on_toggle_preview_block.call(ix),
                                        }
                                    }
                                }
                            } else {
                                PreviewGraph {
                                    session: session.clone(),
                                    palette: snapshot.palette,
                                }
                            }
                        }
                    }
                }
            }
            WorkspaceViewMode::Terminal => rsx! {
                div {
                    style: "display:flex; flex-direction:column; min-width:0; min-height:0; width:100%; height:100%;",
                    TerminalHeader {
                        session: session.clone(),
                        precis: snapshot
                            .active_precis
                            .clone()
                            .unwrap_or_else(|| terminal_precis(&session)),
                        palette: snapshot.palette,
                        server_busy: snapshot.server_busy,
                        on_switch_agent_mode,
                    }
                    TerminalCanvas {
                        key: "{terminal_instance_key(&session.session_path, snapshot.settings.terminal_font_size)}",
                        session: session.clone(),
                        snapshot: snapshot.clone(),
                    }
                }
            },
        }
    } else {
        rsx! {
            EmptyState { palette: snapshot.palette }
        }
    };

    rsx! {
        div {
            style: format!(
                "flex:1; min-width:0; min-height:0; display:flex; flex-direction:column; background:transparent; padding:12px 12px 10px 0;",
            ),
            div {
                style: format!(
                    "flex:1; min-height:0; overflow:{}; padding:{}; background:{}; border-radius:11px; box-shadow:{};",
                    if snapshot.active_view_mode == WorkspaceViewMode::Terminal { "hidden" } else { "auto" },
                    if snapshot.active_view_mode == WorkspaceViewMode::Terminal { "0" } else { "24px" },
                    snapshot.palette.panel, snapshot.palette.panel_shadow
                ),
                {body}
            }
        }
    }
}

#[component]
fn TerminalHeader(
    session: ManagedSessionView,
    precis: String,
    palette: Palette,
    server_busy: bool,
    on_switch_agent_mode: EventHandler<SessionKind>,
) -> Element {
    rsx! {
        div {
            style: "display:flex; align-items:flex-start; justify-content:space-between; gap:16px; padding:22px 26px 14px 26px; border-bottom:1px solid rgba(170,190,212,0.16);",
            div {
                style: "display:flex; flex-direction:column; gap:6px; min-width:0;",
                div {
                    style: format!("font-size:22px; font-weight:700; color:{}; line-height:1.2;", palette.text),
                    "{session.title}"
                }
                div {
                    style: format!("font-size:12px; line-height:1.6; color:{}; max-width:720px; white-space:pre-wrap;", palette.muted),
                    "{precis}"
                }
            }
            if session.kind.is_agent() {
                AgentModeSelector {
                    selected: session.kind,
                    palette,
                    disabled: server_busy,
                    on_select: on_switch_agent_mode,
                }
            }
        }
    }
}

#[component]
fn IconToggleButton(
    active: bool,
    icon: &'static str,
    label: String,
    palette: Palette,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        button {
            title: "{label}",
            style: format!(
                "display:inline-flex; align-items:center; justify-content:center; width:34px; height:34px; \
                 border:none; border-radius:12px; background:{}; color:{}; font-size:15px; font-weight:700; \
                 box-shadow: inset 0 0 0 1px {};",
                if active { "rgba(95, 168, 255, 0.18)" } else { "rgba(255,255,255,0.68)" },
                if active { palette.accent } else { palette.muted },
                if active { "rgba(95, 168, 255, 0.22)" } else { "rgba(255,255,255,0.6)" }
            ),
            onclick: move |evt| onclick.call(evt),
            "{icon}"
        }
    }
}

#[component]
fn PreviewToolbar(
    palette: Palette,
    preview_layout: PreviewLayoutMode,
    server_busy: bool,
    on_expand_preview: EventHandler<MouseEvent>,
    on_collapse_preview: EventHandler<MouseEvent>,
    on_set_preview_layout: EventHandler<PreviewLayoutMode>,
) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:10px; margin-left:auto; min-width:250px;",
            div {
                style: "display:flex; align-items:center; justify-content:flex-end; gap:8px;",
                IconToggleButton {
                    active: preview_layout == PreviewLayoutMode::Chat,
                    icon: "▤",
                    label: "Chat View".to_string(),
                    palette,
                    onclick: move |_| on_set_preview_layout.call(PreviewLayoutMode::Chat),
                }
                IconToggleButton {
                    active: preview_layout == PreviewLayoutMode::Graph,
                    icon: "◎",
                    label: "Graph View".to_string(),
                    palette,
                    onclick: move |_| on_set_preview_layout.call(PreviewLayoutMode::Graph),
                }
            }
            div {
                style: "display:flex; justify-content:flex-end; gap:8px;",
                button {
                    style: chip_style(palette, server_busy),
                    onclick: move |evt| on_expand_preview.call(evt),
                    "Expand All"
                }
                button {
                    style: chip_style(palette, server_busy),
                    onclick: move |evt| on_collapse_preview.call(evt),
                    "Collapse All"
                }
            }
        }
    }
}

#[component]
fn DocumentEditor(
    session: ManagedSessionView,
    palette: Palette,
    server_busy: bool,
    on_save: EventHandler<(String, WorkspaceDocumentInput)>,
    on_run_recipe_document: EventHandler<(String, WorkspaceDocumentInput, bool)>,
) -> Element {
    let mut title = use_signal(|| session.title.clone());
    let initial_body = session
        .rendered_sections
        .first()
        .map(|section| section.lines.join("\n"))
        .or_else(|| {
            session
                .preview
                .blocks
                .first()
                .map(|block| block.lines.join("\n"))
        })
        .unwrap_or_default();
    let initial_replay = session
        .rendered_sections
        .iter()
        .find(|section| section.title == "Replay Commands")
        .map(|section| section.lines.join("\n"))
        .unwrap_or_default();
    let mut body = use_signal(|| initial_body);
    let mut replay = use_signal(|| initial_replay);
    let storage_path = session.session_path.clone();
    let document_kind = metadata_value(&session, "Kind");
    let source_session_path = metadata_value(&session, "Source Session");
    let source_session_kind = metadata_value(&session, "Source Kind");
    let source_session_cwd = metadata_value(&session, "Source Cwd");
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:18px; min-width:0; width:min(980px, 100%); max-width:100%; height:100%; margin:0 auto; overflow:hidden;",
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:14px; flex-wrap:wrap;",
                div {
                    style: "display:flex; flex-direction:column; gap:6px; min-width:0;",
                    input {
                        r#type: "text",
                        value: "{title}",
                        placeholder: "Document title",
                        style: format!(
                            "height:40px; min-width:min(440px, 100%); max-width:100%; box-sizing:border-box; padding:0 14px; border:none; border-radius:10px; \
                             background:rgba(255,255,255,0.76); color:{}; font-size:18px; font-weight:700; outline:none; \
                             box-shadow: inset 0 0 0 1px rgba(170,190,212,0.18);",
                            palette.text
                        ),
                        oninput: move |evt| title.set(evt.value()),
                    }
                    div {
                        style: format!("font-size:11px; color:{};", palette.muted),
                        "{storage_path}"
                    }
                }
                div {
                    style: "display:flex; align-items:center; gap:8px; flex-wrap:wrap;",
                    if document_kind == "terminal recipe" && !replay().trim().is_empty() {
                        button {
                            style: chip_style(palette, server_busy),
                            onclick: {
                                let storage_path = storage_path.clone();
                                let document_kind = document_kind.clone();
                                let source_session_path = source_session_path.clone();
                                let source_session_kind = source_session_kind.clone();
                                let source_session_cwd = source_session_cwd.clone();
                                move |_| on_run_recipe_document.call((
                                    storage_path.clone(),
                                    build_document_input(
                                        &document_kind,
                                        title(),
                                        body(),
                                        &source_session_path,
                                        &source_session_kind,
                                        &source_session_cwd,
                                        replay(),
                                    ),
                                    false,
                                ))
                            },
                            "Run Here"
                        }
                        button {
                            style: chip_style(palette, server_busy),
                            onclick: {
                                let storage_path = storage_path.clone();
                                let document_kind = document_kind.clone();
                                let source_session_path = source_session_path.clone();
                                let source_session_kind = source_session_kind.clone();
                                let source_session_cwd = source_session_cwd.clone();
                                move |_| on_run_recipe_document.call((
                                    storage_path.clone(),
                                    build_document_input(
                                        &document_kind,
                                        title(),
                                        body(),
                                        &source_session_path,
                                        &source_session_kind,
                                        &source_session_cwd,
                                        replay(),
                                    ),
                                    true,
                                ))
                            },
                            "Run In New Session"
                        }
                    }
                    button {
                        style: chip_style(palette, server_busy),
                        onclick: {
                            let storage_path = storage_path.clone();
                            let document_kind = document_kind.clone();
                            let source_session_path = source_session_path.clone();
                            let source_session_kind = source_session_kind.clone();
                            let source_session_cwd = source_session_cwd.clone();
                            move |_| on_save.call((
                                storage_path.clone(),
                                build_document_input(
                                    &document_kind,
                                    title(),
                                    body(),
                                    &source_session_path,
                                    &source_session_kind,
                                    &source_session_cwd,
                                    replay(),
                                ),
                            ))
                        },
                        "Save"
                    }
                }
            }
            textarea {
                value: "{body}",
                wrap: "soft",
                style: format!(
                    "flex:1; min-height:560px; width:100%; max-width:100%; box-sizing:border-box; resize:none; overflow-x:hidden; padding:18px 20px; border:none; border-radius:14px; \
                     background:rgba(255,255,255,0.72); color:{}; outline:none; font-size:14px; line-height:1.7; \
                     box-shadow: inset 0 0 0 1px rgba(170,190,212,0.16); font-family:{}; white-space:pre-wrap; overflow-wrap:anywhere;",
                    palette.text, interface_font_family()
                ),
                oninput: move |evt| body.set(evt.value()),
            }
            if document_kind == "terminal recipe" {
                div {
                    style: "display:flex; flex-direction:column; gap:8px;",
                    div {
                        style: format!("font-size:11px; font-weight:700; color:{};", palette.muted),
                        "Replay Commands"
                    }
                    textarea {
                        value: "{replay}",
                        wrap: "soft",
                        style: format!(
                            "min-height:120px; width:100%; max-width:100%; box-sizing:border-box; resize:vertical; overflow-x:hidden; padding:14px 16px; border:none; border-radius:14px; \
                             background:rgba(255,255,255,0.66); color:{}; outline:none; font-size:13px; line-height:1.6; \
                             box-shadow: inset 0 0 0 1px rgba(170,190,212,0.16); font-family:'JetBrains Mono', 'Iosevka Term', monospace; white-space:pre-wrap; overflow-wrap:anywhere;",
                            palette.text
                        ),
                        oninput: move |evt| replay.set(evt.value()),
                    }
                }
            }
        }
    }
}

fn build_document_input(
    document_kind: &str,
    title: String,
    body: String,
    source_session_path: &str,
    source_session_kind: &str,
    source_session_cwd: &str,
    replay: String,
) -> WorkspaceDocumentInput {
    WorkspaceDocumentInput {
        title: Some(title),
        kind: if document_kind == "terminal recipe" {
            WorkspaceDocumentKind::TerminalRecipe
        } else {
            WorkspaceDocumentKind::Note
        },
        body,
        source_session_path: if source_session_path.is_empty() {
            None
        } else {
            Some(source_session_path.to_string())
        },
        source_session_kind: if source_session_kind.is_empty() {
            None
        } else {
            Some(source_session_kind.to_string())
        },
        source_session_cwd: if source_session_cwd.is_empty() {
            None
        } else {
            Some(source_session_cwd.to_string())
        },
        replay_commands: replay
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
    }
}

#[component]
fn PreviewGraph(session: ManagedSessionView, palette: Palette) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:14px; padding:12px 0 4px 0;",
            for (ix, block) in session.preview.blocks.iter().enumerate() {
                div {
                    style: "display:grid; grid-template-columns:56px 1fr; gap:18px; align-items:flex-start;",
                    div {
                        style: "display:flex; flex-direction:column; align-items:center; gap:8px;",
                        div {
                            style: format!(
                                "width:38px; height:38px; border-radius:999px; display:flex; align-items:center; justify-content:center; \
                                 background:{}; color:{}; font-size:14px; font-weight:700; box-shadow:0 8px 18px rgba(148,163,184,0.18);",
                                if block.tone == PreviewTone::User { "rgba(73,138,255,0.16)" } else { "rgba(255,255,255,0.95)" },
                                if block.tone == PreviewTone::User { palette.accent } else { palette.text }
                            ),
                            {if block.tone == PreviewTone::User { "U" } else { "A" }}
                        }
                        if ix + 1 != session.preview.blocks.len() {
                            div {
                                style: "width:2px; min-height:54px; border-radius:999px; background:linear-gradient(180deg, rgba(120,153,189,0.42) 0%, rgba(120,153,189,0.08) 100%);"
                            }
                        }
                    }
                    div {
                        style: format!(
                            "display:flex; flex-direction:column; gap:10px; padding:16px 18px; border-radius:18px; \
                             background:{}; box-shadow:0 16px 28px rgba(148,163,184,0.08), inset 0 0 0 1px rgba(255,255,255,0.62);",
                            if block.tone == PreviewTone::User { "rgba(232,244,255,0.96)" } else { "rgba(255,255,255,0.92)" }
                        ),
                        div {
                            style: "display:flex; align-items:center; justify-content:space-between; gap:12px;",
                            span {
                                style: format!("font-size:12px; font-weight:700; color:{};", palette.text),
                                "{block.role}"
                            }
                            span {
                                style: format!("font-size:11px; color:{};", palette.muted),
                                "{block.timestamp}"
                            }
                        }
                        div {
                            style: format!("display:flex; flex-direction:column; gap:8px; color:{};", palette.text),
                            for line in block.lines.iter().take(3) {
                                div {
                                    style: "font-size:13px; line-height:1.55; white-space:pre-wrap;",
                                    "{line}"
                                }
                            }
                            if block.lines.len() > 3 {
                                div {
                                    style: format!("font-size:11px; color:{};", palette.muted),
                                    "+ {block.lines.len() - 3} more lines"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn PreviewSummary(session: ManagedSessionView, palette: Palette) -> Element {
    let started = metadata_value(&session, "Started");
    let messages = metadata_value(&session, "Messages");
    let preview_blocks = session.preview.blocks.len();

    rsx! {
        div {
            style: format!(
                "display:flex; flex-direction:column; gap:8px; flex:1; min-width:280px; \
                 background:linear-gradient(180deg, rgba(255,255,255,0.78) 0%, rgba(248,252,255,0.68) 100%); \
                 border:none; border-radius:18px; padding:18px 20px; \
                 box-shadow: inset 0 0 0 1px rgba(255,255,255,0.48);",
            ),
            div {
                style: format!("font-size:11px; font-weight:700; letter-spacing:0.04em; text-transform:uppercase; color:{};", palette.muted),
                "Session Preview"
            }
            div {
                style: format!("font-size:22px; font-weight:700; color:{}; line-height:1.18;", palette.text),
                "{session.title}"
            }
            div {
                style: format!("font-size:12px; color:{};", palette.muted),
                "{session.host_label} · {started}"
            }
            div {
                style: "display:flex; flex-wrap:wrap; gap:8px;",
                StatusPill { label: "Messages".to_string(), value: messages, palette }
                StatusPill { label: "Blocks".to_string(), value: preview_blocks.to_string(), palette }
                StatusPill { label: "Mode".to_string(), value: "Rendered".to_string(), palette }
            }
        }
    }
}

fn terminal_precis(session: &ManagedSessionView) -> String {
    let mut candidates = Vec::new();
    for block in &session.preview.blocks {
        for line in &block.lines {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                candidates.push(trimmed.to_string());
            }
        }
    }
    for section in &session.rendered_sections {
        for line in &section.lines {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                candidates.push(trimmed.to_string());
            }
        }
    }
    candidates
        .into_iter()
        .find(|line| {
            !line.starts_with("Resume Codex session")
                && !line.starts_with("Open live terminal")
                && !line.starts_with("Preview mode renders")
                && !line.starts_with("GUI selection asks")
                && !line.starts_with("Launch command prepared")
        })
        .unwrap_or_else(|| session.status_line.clone())
}

#[component]
fn AgentModeSelector(
    selected: SessionKind,
    palette: Palette,
    disabled: bool,
    on_select: EventHandler<SessionKind>,
) -> Element {
    rsx! {
        div {
            style: toggle_slider_style(palette),
            button {
                style: toggle_slider_end_style(palette, selected == SessionKind::Codex),
                disabled: disabled,
                onclick: move |_| on_select.call(SessionKind::Codex),
                "Codex"
            }
            button {
                style: toggle_slider_end_style(palette, selected == SessionKind::CodexLiteLlm),
                disabled: disabled,
                onclick: move |_| on_select.call(SessionKind::CodexLiteLlm),
                "Codex LiteLLM"
            }
        }
    }
}

#[component]
fn PreviewBlock(
    block_ix: usize,
    block: SessionPreviewBlock,
    palette: Palette,
    on_toggle: EventHandler<MouseEvent>,
) -> Element {
    let background = match block.tone {
        PreviewTone::User => "rgba(230, 242, 255, 0.98)",
        PreviewTone::Assistant => "rgba(255, 255, 255, 0.94)",
    };
    let badge = match block.tone {
        PreviewTone::User => palette.accent,
        PreviewTone::Assistant => palette.muted,
    };
    let outline = match block.tone {
        PreviewTone::User => "rgba(66, 153, 225, 0.18)",
        PreviewTone::Assistant => "rgba(148, 163, 184, 0.18)",
    };

    rsx! {
        button {
            style: format!(
                "width:100%; border:none; text-align:left; background:{}; border-radius:18px; \
                 padding:20px 20px; box-shadow: inset 0 0 0 1px {}, 0 14px 28px rgba(148,163,184,0.08);",
                background, outline
            ),
            onclick: move |evt| on_toggle.call(evt),
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:12px; margin-bottom:12px;",
                div {
                    style: "display:flex; align-items:center; gap:8px;",
                    span {
                        style: format!(
                            "display:inline-flex; align-items:center; justify-content:center; min-width:54px; height:22px; \
                             border-radius:999px; background:{}; color:{}; font-size:11px; font-weight:700;",
                            if block.tone == PreviewTone::User { "rgba(37,99,235,0.12)" } else { "rgba(108,114,127,0.10)" },
                            badge
                        ),
                        "{block.role}"
                    }
                    span {
                        style: format!("font-size:11px; color:{};", palette.muted),
                        "{block.timestamp}"
                    }
                }
                span {
                    style: format!("font-size:11px; color:{};", palette.muted),
                    {if block.folded { format!("Expand {}", block_ix + 1) } else { format!("Collapse {}", block_ix + 1) }}
                }
            }
            if block.folded {
                div {
                    style: format!("font-size:12px; color:{};", palette.muted),
                    "{block.lines.len()} lines hidden"
                }
            } else {
                div {
                    style: format!("display:flex; flex-direction:column; gap:9px; color:{};", palette.text),
                    for line in block.lines.iter() {
                        div {
                            style: "font-size:13px; line-height:1.62; white-space:pre-wrap;",
                            "{line}"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn TerminalCanvas(session: ManagedSessionView, snapshot: RenderSnapshot) -> Element {
    let endpoint = BOOTSTRAP
        .get()
        .expect("shell bootstrap initialized")
        .server_endpoint
        .clone();
    let session_path = session.session_path.clone();
    let host_id = terminal_host_id(&session_path);
    let instance_key = terminal_instance_key(&session_path, snapshot.settings.terminal_font_size);
    let terminal_title = session.title.clone();
    let future_host_id = host_id.clone();
    let theme = terminal_theme(snapshot.palette, snapshot.settings.terminal_font_size);
    let future_theme = theme.clone();
    info!(
        session=%session_path,
        host_id=%host_id,
        instance_key=%instance_key,
        font_size=theme.font_size,
        "rendering terminal canvas"
    );
    {
        let host_id = host_id.clone();
        let theme = theme.clone();
        use_effect(move || {
            let _ = document::eval(&terminal_apply_script(&host_id, &theme));
        });
    }
    use_future(move || {
        let endpoint = endpoint.clone();
        let session_path = session_path.clone();
        let host_id = future_host_id.clone();
        let title = terminal_title.clone();
        let theme = future_theme.clone();
        async move {
            if let Err(error) = terminal_ensure(&endpoint, &session_path) {
                warn!(session=%session_path, error=%error, "failed to ensure terminal");
                return;
            }
            let mut eval = document::eval(&terminal_eval_script(&host_id, &theme));
            let _ = eval.send(TerminalJsCommand::Reset {
                title,
                background: theme.background.clone(),
                foreground: theme.foreground.clone(),
                cursor: theme.cursor.clone(),
                selection: theme.selection.clone(),
                font_size: theme.font_size,
            });
            let mut cursor = 0u64;
            loop {
                tokio::select! {
                    event = eval.recv::<TerminalJsEvent>() => {
                        match event {
                            Ok(TerminalJsEvent::Ready) => {
                                if let Ok((next_cursor, chunks)) = terminal_read(&endpoint, &session_path, 0) {
                                    cursor = next_cursor;
                                    for chunk in chunks {
                                        let _ = eval.send(TerminalJsCommand::Write { data: chunk.data });
                                    }
                                }
                            }
                            Ok(TerminalJsEvent::Input { data }) => {
                                let _ = terminal_write(&endpoint, &session_path, &data);
                            }
                            Ok(TerminalJsEvent::Resize { cols, rows }) => {
                                let _ = terminal_resize(&endpoint, &session_path, cols, rows);
                            }
                            Ok(TerminalJsEvent::Debug { message }) => {
                                info!(session=%session_path, %message, "terminal js debug");
                            }
                            Err(error) => {
                                warn!(session=%session_path, error=%error, "terminal eval bridge closed");
                                break;
                            }
                        }
                    }
                    _ = sleep(Duration::from_millis(60)) => {
                        match terminal_read(&endpoint, &session_path, cursor) {
                            Ok((next_cursor, chunks)) => {
                                cursor = next_cursor;
                                for chunk in chunks {
                                    let _ = eval.send(TerminalJsCommand::Write { data: chunk.data });
                                }
                            }
                            Err(error) => {
                                warn!(session=%session_path, error=%error, "terminal read failed");
                                break;
                            }
                        }
                    }
                }
            }
        }
    });
    rsx! {
        div {
            style: "display:flex; flex-direction:column; min-height:0; height:100%;",
            div {
                style: format!(
                    "display:flex; flex-direction:column; min-height:0; height:100%; gap:0; border-radius:11px; \
                     background:{}; overflow:hidden;",
                    theme.background
                ),
                div {
                    id: "{host_id}",
                    style: format!(
                        "flex:1; min-height:0; width:100%; height:100%; background:{}; overflow:hidden;",
                        theme.background
                    )
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct TerminalTheme {
    background: String,
    foreground: String,
    cursor: String,
    font_size: f32,
    selection: String,
}

fn terminal_theme(palette: Palette, font_size: f32) -> TerminalTheme {
    TerminalTheme {
        background: palette.panel.to_string(),
        foreground: palette.text.to_string(),
        cursor: palette.accent.to_string(),
        font_size: font_size.max(5.0),
        selection: "rgba(107,165,255,0.16)".to_string(),
    }
}

fn xterm_assets_bootstrap_script() -> String {
    let css = serde_json::to_string(XTERM_CSS).expect("serialize xterm css");
    let xterm = serde_json::to_string(XTERM_JS).expect("serialize xterm js");
    let fit = serde_json::to_string(XTERM_FIT_JS).expect("serialize xterm fit addon");
    format!(
        r#"
        (() => {{
          const styleId = "yggterm-xterm-style";
          if (!document.getElementById(styleId)) {{
            const style = document.createElement("style");
            style.id = styleId;
            style.textContent = {css};
            document.head.appendChild(style);
          }}
          if (!window.Terminal) {{
            window.eval({xterm});
          }}
          if (!window.FitAddon || !window.FitAddon.FitAddon) {{
            window.eval({fit});
          }}
        }})();
        "#
    )
}

fn terminal_host_id(session_path: &str) -> String {
    let mut id = String::from("yggterm-terminal-");
    for ch in session_path.chars() {
        if ch.is_ascii_alphanumeric() {
            id.push(ch.to_ascii_lowercase());
        } else {
            id.push('-');
        }
    }
    id
}

fn terminal_instance_key(session_path: &str, font_size: f32) -> String {
    format!("{session_path}-{font_size:.1}")
}

fn current_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn notification_delivery_mode(settings: &AppSettings) -> NotificationDeliveryMode {
    match (settings.in_app_notifications, settings.system_notifications) {
        (true, true) => NotificationDeliveryMode::Both,
        (false, true) => NotificationDeliveryMode::System,
        _ => NotificationDeliveryMode::InApp,
    }
}

fn apply_notification_delivery_mode(settings: &mut AppSettings, mode: NotificationDeliveryMode) {
    match mode {
        NotificationDeliveryMode::InApp => {
            settings.in_app_notifications = true;
            settings.system_notifications = false;
        }
        NotificationDeliveryMode::Both => {
            settings.in_app_notifications = true;
            settings.system_notifications = true;
        }
        NotificationDeliveryMode::System => {
            settings.in_app_notifications = false;
            settings.system_notifications = true;
        }
    }
}

fn emit_notification_chime() {
    let _ = document::eval(
        r#"
        (() => {
          try {
            const ctx = new (window.AudioContext || window.webkitAudioContext)();
            const osc = ctx.createOscillator();
            const gain = ctx.createGain();
            osc.type = "sine";
            osc.frequency.value = 880;
            gain.gain.value = 0.03;
            osc.connect(gain);
            gain.connect(ctx.destination);
            osc.start();
            setTimeout(() => {
              try { osc.stop(); } catch (_error) {}
              try { ctx.close(); } catch (_error) {}
            }, 85);
          } catch (_error) {}
        })();
        "#,
    );
}

fn emit_system_notification(title: &str, message: &str) {
    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("notify-send")
            .arg(title)
            .arg(message)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification {} with title {}",
            serde_json::to_string(message).unwrap_or_else(|_| "\"\"".to_string()),
            serde_json::to_string(title).unwrap_or_else(|_| "\"Yggterm\"".to_string())
        );
        let _ = Command::new("osascript")
            .arg("-e")
            .arg(script)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
    }
}

fn apply_active_terminal_zoom(state: Signal<ShellState>) {
    let snapshot = state.read().snapshot();
    if snapshot.active_view_mode != WorkspaceViewMode::Terminal {
        return;
    }
    let Some(session) = snapshot.active_session else {
        return;
    };
    let host_id = terminal_host_id(&session.session_path);
    let theme = terminal_theme(snapshot.palette, snapshot.settings.terminal_font_size);
    info!(
        session=%session.session_path,
        host_id=%host_id,
        font_size=theme.font_size,
        "applying live terminal zoom"
    );
    let _ = document::eval(&terminal_apply_script(&host_id, &theme));
}

fn terminal_eval_script(host_id: &str, theme: &TerminalTheme) -> String {
    let background =
        serde_json::to_string(&theme.background).expect("serialize terminal background");
    let foreground =
        serde_json::to_string(&theme.foreground).expect("serialize terminal foreground");
    let cursor = serde_json::to_string(&theme.cursor).expect("serialize terminal cursor");
    let selection = serde_json::to_string(&theme.selection).expect("serialize terminal selection");
    let constructed_debug = if cfg!(debug_assertions) {
        "dioxus.send({ kind: \"debug\", message: `constructed host=${hostId} fontSize=${term.options.fontSize} cols=${term.cols} rows=${term.rows}` });"
    } else {
        ""
    };
    let reset_debug = if cfg!(debug_assertions) {
        "dioxus.send({ kind: \"debug\", message: `reset host=${hostId} fontSize=${term.options.fontSize}` });"
    } else {
        ""
    };
    format!(
        r#"
        const hostId = {host_id:?};
        const host = document.getElementById(hostId);
        if (!host) {{
            dioxus.send({{ kind: "ready" }});
            return;
        }}
        if (!window.Terminal || !window.FitAddon || !window.FitAddon.FitAddon) {{
            host.innerHTML = '<div style="padding:18px;color:#fca5a5;font:13px system-ui;">xterm.js assets failed to load.</div>';
            dioxus.send({{ kind: "ready" }});
            while (true) {{
                await dioxus.recv();
            }}
        }}
        if (window.__yggtermXtermCleanup) {{
            try {{
                window.__yggtermXtermCleanup();
            }} catch (_error) {{}}
        }}
        host.innerHTML = "";
        const term = new window.Terminal({{
            allowTransparency: true,
            convertEol: true,
            cursorBlink: true,
            fontFamily: "'JetBrains Mono', 'Iosevka Term', 'Fira Code', monospace",
            fontSize: {font_size},
            scrollback: 5000,
            theme: {{
                background: {background},
                foreground: {foreground},
                cursor: {cursor},
                selectionBackground: {selection},
            }},
        }});
        const fitAddon = new window.FitAddon.FitAddon();
        term.loadAddon(fitAddon);
        term.open(host);
        window.__yggtermXtermHosts = window.__yggtermXtermHosts || {{}};
        window.__yggtermXtermHosts[hostId] = {{ term, fitAddon }};
        const emitResize = () => {{
            try {{
                fitAddon.fit();
                dioxus.send({{ kind: "resize", cols: term.cols, rows: term.rows }});
            }} catch (_error) {{}}
        }};
        const resizeObserver = new ResizeObserver(() => emitResize());
        resizeObserver.observe(host);
        term.onData((data) => dioxus.send({{ kind: "input", data }}));
        window.__yggtermXtermCleanup = () => {{
            try {{
                resizeObserver.disconnect();
            }} catch (_error) {{}}
            try {{
                term.dispose();
            }} catch (_error) {{}}
            try {{
                if (window.__yggtermXtermHosts) {{
                    delete window.__yggtermXtermHosts[hostId];
                }}
            }} catch (_error) {{}}
            host.innerHTML = "";
        }};
        emitResize();
        {constructed_debug}
        dioxus.send({{ kind: "ready" }});
        while (true) {{
            const message = await dioxus.recv();
            if (!message) {{
                continue;
            }}
            if (message.kind === "reset") {{
                term.clear();
                term.options.fontSize = message.font_size;
                term.options.theme = {{
                    background: message.background,
                    foreground: message.foreground,
                    cursor: message.cursor,
                    selectionBackground: message.selection,
                }};
                {reset_debug}
                requestAnimationFrame(() => emitResize());
            }} else if (message.kind === "write") {{
                term.write(message.data);
            }}
        }}
        "#,
        font_size = theme.font_size,
        background = background,
        foreground = foreground,
        cursor = cursor,
        selection = selection,
        constructed_debug = constructed_debug,
        reset_debug = reset_debug
    )
}

fn terminal_apply_script(host_id: &str, theme: &TerminalTheme) -> String {
    let background =
        serde_json::to_string(&theme.background).expect("serialize terminal background");
    let foreground =
        serde_json::to_string(&theme.foreground).expect("serialize terminal foreground");
    let cursor = serde_json::to_string(&theme.cursor).expect("serialize terminal cursor");
    let selection = serde_json::to_string(&theme.selection).expect("serialize terminal selection");
    format!(
        r#"
        (() => {{
          const hostId = {host_id:?};
          const registry = window.__yggtermXtermHosts || {{}};
          const entry = registry[hostId];
          if (!entry || !entry.term) {{
            return;
          }}
          entry.term.options.fontSize = {font_size};
          entry.term.options.theme = {{
            background: {background},
            foreground: {foreground},
            cursor: {cursor},
            selectionBackground: {selection},
          }};
          try {{
            entry.fitAddon && entry.fitAddon.fit();
          }} catch (_error) {{}}
          window.__yggtermLastApply = {{
            hostId,
            fontSize: entry.term.options.fontSize,
            appliedAt: Date.now(),
          }};
        }})();
        "#,
        host_id = host_id,
        font_size = theme.font_size,
        background = background,
        foreground = foreground,
        cursor = cursor,
        selection = selection,
    )
}

#[component]
fn StatusPill(label: String, value: String, palette: Palette) -> Element {
    rsx! {
        div {
            style: format!(
                "display:inline-flex; align-items:center; gap:6px; padding:6px 10px; border-radius:999px; \
                 background:rgba(255,255,255,0.62); box-shadow: inset 0 0 0 1px rgba(255,255,255,0.46);"
            ),
            span {
                style: format!("font-size:11px; font-weight:700; color:{};", palette.muted),
                "{label}"
            }
            span {
                style: format!("font-size:11px; font-weight:700; color:{};", palette.text),
                "{value}"
            }
        }
    }
}

#[component]
fn TerminalCard(title: String, subtitle: String, lines: Vec<String>, palette: Palette) -> Element {
    rsx! {
        div {
            style: format!(
                "display:flex; flex-direction:column; gap:12px; background:{}; border:none; \
                 border-radius:14px; padding:15px 16px; box-shadow: inset 0 0 0 1px rgba(255,255,255,0.38);",
                palette.panel_alt
            ),
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:12px;",
                div {
                    style: format!("font-size:13px; font-weight:700; color:{};", palette.text),
                    "{title}"
                }
                div {
                    style: format!("font-size:11px; color:{};", palette.muted),
                    "{subtitle}"
                }
            }
            div {
                style: format!(
                    "display:flex; flex-direction:column; gap:8px; padding:12px; background:{}; \
                     border-radius:12px; box-shadow: inset 0 0 0 1px rgba(255,255,255,0.62);",
                    palette.panel
                ),
                for (ix, line) in lines.iter().enumerate() {
                    div {
                        style: format!(
                            "font-size:12px; line-height:1.45; color:{}; white-space:pre-wrap;",
                            if line.starts_with('$') { palette.accent } else { palette.text }
                        ),
                        "{ix + 1:02}  {line}"
                    }
                }
            }
        }
    }
}

#[component]
fn EmptyState(palette: Palette) -> Element {
    rsx! {
        div {
            style: "display:flex; align-items:center; justify-content:center; height:100%;",
            div {
                style: format!(
                    "display:flex; flex-direction:column; align-items:center; gap:8px; background:{}; \
                     border:none; border-radius:16px; padding:22px 26px; box-shadow: inset 0 0 0 1px rgba(255,255,255,0.42);",
                    palette.panel_alt
                ),
                div {
                    style: format!("font-size:14px; font-weight:700; color:{};", palette.text),
                    "No Session Selected"
                }
                div {
                    style: format!("font-size:12px; color:{};", palette.muted),
                    "Choose a stored Codex session or connect over SSH from the sidebar."
                }
            }
        }
    }
}

#[component]
fn RightRail(
    snapshot: RenderSnapshot,
    on_endpoint_change: EventHandler<String>,
    on_api_key_change: EventHandler<String>,
    on_model_change: EventHandler<String>,
    on_set_notification_delivery: EventHandler<NotificationDeliveryMode>,
    on_set_notification_sound: EventHandler<bool>,
    on_generate_titles: EventHandler<MouseEvent>,
    on_adjust_ui_zoom: EventHandler<i32>,
    on_adjust_main_zoom: EventHandler<i32>,
    on_connect_ssh_custom: EventHandler<MouseEvent>,
    on_ssh_target_change: EventHandler<String>,
    on_ssh_prefix_change: EventHandler<String>,
    on_clear_notification: EventHandler<u64>,
    on_clear_notifications: EventHandler<MouseEvent>,
) -> Element {
    let visible = snapshot.right_panel_mode != RightPanelMode::Hidden;
    let width = if visible { SIDE_RAIL_WIDTH } else { 0 };
    let opacity = if visible { "1" } else { "0" };
    let translate = if visible {
        "translateX(0)"
    } else {
        "translateX(14px)"
    };
    rsx! {
        div {
            style: format!(
                "width:{}px; min-width:{}px; max-width:{}px; display:flex; flex-direction:column; \
                 background:transparent; overflow:hidden; text-rendering:optimizeLegibility; \
                 -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale; \
                 transition: opacity 180ms ease, transform 180ms ease; \
                 opacity:{}; transform:{}; pointer-events:{}; zoom:{}%;",
                width, width, width, opacity, translate,
                if visible { "auto" } else { "none" },
                zoom_percent_f32(snapshot.settings.ui_font_size, 14.0)
            ),
            if snapshot.right_panel_mode == RightPanelMode::Metadata {
                MetadataRailBody { snapshot: snapshot.clone() }
            } else if snapshot.right_panel_mode == RightPanelMode::Settings {
                SettingsRailBody {
                    snapshot: snapshot.clone(),
                    on_endpoint_change,
                    on_api_key_change,
                    on_model_change,
                    on_set_notification_delivery,
                    on_set_notification_sound,
                    on_generate_titles,
                    on_adjust_ui_zoom,
                    on_adjust_main_zoom,
                }
            } else if snapshot.right_panel_mode == RightPanelMode::Connect {
                ConnectRailBody {
                    snapshot: snapshot.clone(),
                    on_connect_ssh_custom,
                    on_ssh_target_change,
                    on_ssh_prefix_change,
                }
            } else if snapshot.right_panel_mode == RightPanelMode::Notifications {
                NotificationsRailBody {
                    snapshot: snapshot.clone(),
                    on_clear_notification,
                    on_clear_notifications,
                }
            }
        }
    }
}

#[component]
fn MetadataRailBody(snapshot: RenderSnapshot) -> Element {
    let session = snapshot.active_session;

    rsx! {
        div {
            style: format!(
                "padding:16px 16px 10px 16px; font-size:12px; font-weight:700; letter-spacing:0.01em; color:{}; \
                 text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                snapshot.palette.text
            ),
            "Session Metadata"
        }
        div {
            style: "flex:1; overflow:auto; padding:10px 16px 14px 16px; display:flex; flex-direction:column; gap:16px; \
             text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
            if let Some(session) = session {
                MetadataGroup {
                    title: "Overview".to_string(),
                    entries: session.metadata.iter().take(6).cloned().collect(),
                    palette: snapshot.palette,
                }
                MetadataGroup {
                    title: "Runtime".to_string(),
                    entries: session.metadata.iter().skip(6).cloned().collect(),
                    palette: snapshot.palette,
                }
            } else {
                MetadataGroup {
                    title: "Overview".to_string(),
                    entries: vec![SessionMetadataEntry {
                        label: "State",
                        value: "No session selected".to_string(),
                    }],
                    palette: snapshot.palette,
                }
            }
        }
    }
}

#[component]
fn SettingsRailBody(
    snapshot: RenderSnapshot,
    on_endpoint_change: EventHandler<String>,
    on_api_key_change: EventHandler<String>,
    on_model_change: EventHandler<String>,
    on_set_notification_delivery: EventHandler<NotificationDeliveryMode>,
    on_set_notification_sound: EventHandler<bool>,
    on_generate_titles: EventHandler<MouseEvent>,
    on_adjust_ui_zoom: EventHandler<i32>,
    on_adjust_main_zoom: EventHandler<i32>,
) -> Element {
    rsx! {
        div {
            style: format!(
                "padding:16px 16px 10px 16px; font-size:12px; font-weight:700; letter-spacing:0.01em; color:{}; \
                 text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                snapshot.palette.text
            ),
            "Interface Settings"
        }
        div {
            style: "flex:1; overflow:auto; padding:10px 16px 14px 16px; display:flex; flex-direction:column; gap:14px; \
             text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
            MetadataGroup {
                title: "Install".to_string(),
                entries: vec![
                    SessionMetadataEntry {
                        label: "Version",
                        value: snapshot.install_context.current_version.clone(),
                    },
                    SessionMetadataEntry {
                        label: "Channel",
                        value: format!("{:?}", snapshot.install_context.channel).to_lowercase(),
                    },
                    SessionMetadataEntry {
                        label: "Updates",
                        value: match snapshot.install_context.update_policy {
                            yggterm_core::UpdatePolicy::Auto => "Automatic on launch".to_string(),
                            yggterm_core::UpdatePolicy::NotifyOnly => snapshot
                                .install_context
                                .manager_hint
                                .clone()
                                .unwrap_or_else(|| "Notify only".to_string()),
                        },
                    },
                ],
                palette: snapshot.palette,
            }
            SettingsField {
                label: "LiteLLM Endpoint".to_string(),
                value: snapshot.settings.litellm_endpoint.clone(),
                placeholder: "https://litellm.example/v1".to_string(),
                secret: false,
                palette: snapshot.palette,
                on_change: on_endpoint_change,
            }
            SettingsField {
                label: "API Key".to_string(),
                value: snapshot.settings.litellm_api_key.clone(),
                placeholder: "sk-...".to_string(),
                secret: true,
                palette: snapshot.palette,
                on_change: on_api_key_change,
            }
            SettingsField {
                label: "Interface LLM".to_string(),
                value: snapshot.settings.interface_llm_model.clone(),
                placeholder: "openai/gpt-5.4-mini".to_string(),
                secret: false,
                palette: snapshot.palette,
                on_change: on_model_change,
            }
            NotificationSettingsSection {
                palette: snapshot.palette,
                selected: notification_delivery_mode(&snapshot.settings),
                sound_enabled: snapshot.settings.notification_sound,
                on_select: on_set_notification_delivery,
                on_change: on_set_notification_sound,
            }
            ZoomSettingRow {
                label: "Interface Zoom".to_string(),
                percent: zoom_percent(snapshot.settings.ui_font_size, 14.0),
                palette: snapshot.palette,
                on_decrease: move |_| on_adjust_ui_zoom.call(-1),
                on_increase: move |_| on_adjust_ui_zoom.call(1),
            }
            ZoomSettingRow {
                label: "Terminal Zoom".to_string(),
                percent: zoom_percent(snapshot.settings.terminal_font_size, 10.0),
                palette: snapshot.palette,
                on_decrease: move |_| on_adjust_main_zoom.call(-1),
                on_increase: move |_| on_adjust_main_zoom.call(1),
            }
            if cfg!(debug_assertions) {
                MetadataGroup {
                    title: "Terminal Debug".to_string(),
                    entries: vec![
                        SessionMetadataEntry {
                            label: "State",
                            value: snapshot.last_terminal_debug.clone(),
                        },
                        SessionMetadataEntry {
                            label: "Active",
                            value: snapshot.active_session.as_ref().map(|session| session.session_path.clone()).unwrap_or_else(|| "none".to_string()),
                        },
                        SessionMetadataEntry {
                            label: "Host",
                            value: snapshot.active_session.as_ref().map(|session| terminal_host_id(&session.session_path)).unwrap_or_else(|| "none".to_string()),
                        },
                        SessionMetadataEntry {
                            label: "Font",
                            value: format!("{:.1}", snapshot.settings.terminal_font_size),
                        },
                    ],
                    palette: snapshot.palette,
                }
                MetadataGroup {
                    title: "Tree Debug".to_string(),
                    entries: vec![
                        SessionMetadataEntry {
                            label: "State",
                            value: snapshot.last_tree_debug.clone(),
                        },
                        SessionMetadataEntry {
                            label: "Selected",
                            value: if snapshot.selected_tree_paths.is_empty() {
                                "none".to_string()
                            } else {
                                snapshot.selected_tree_paths.join(", ")
                            },
                        },
                        SessionMetadataEntry {
                            label: "Drag Target",
                            value: snapshot
                                .drag_hover_target
                                .clone()
                                .unwrap_or_else(|| "none".to_string()),
                        },
                        SessionMetadataEntry {
                            label: "Pending Delete",
                            value: snapshot
                                .pending_delete
                                .as_ref()
                                .map(|pending| format!(
                                    "{} item(s), hard={}",
                                    pending.document_paths.len() + pending.group_paths.len(),
                                    pending.hard_delete
                                ))
                                .unwrap_or_else(|| "none".to_string()),
                        },
                    ],
                    palette: snapshot.palette,
                }
            }
            button {
                style: format!(
                    "height:32px; border:none; border-radius:10px; background:{}; color:{}; \
                     font-size:11px; font-weight:700; text-align:center;",
                    snapshot.palette.accent_soft, snapshot.palette.text
                ),
                onclick: move |evt| on_generate_titles.call(evt),
                "Generate Session Titles"
            }
            div {
                style: format!("font-size:11px; line-height:1.5; color:{};", snapshot.palette.muted),
                "Yggterm caches generated titles under ~/.yggterm/session-titles.db and only refreshes them when you ask for it here."
            }
        }
    }
}

#[component]
fn NotificationsRailBody(
    snapshot: RenderSnapshot,
    on_clear_notification: EventHandler<u64>,
    on_clear_notifications: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        div {
            style: format!(
                "padding:16px 16px 10px 16px; font-size:12px; font-weight:700; letter-spacing:0.01em; color:{};",
                snapshot.palette.text
            ),
            "Notifications"
        }
        div {
            style: "padding:0 16px 8px 16px; display:flex; justify-content:flex-end;",
            button {
                style: chip_style(snapshot.palette, false),
                onclick: move |evt| on_clear_notifications.call(evt),
                "Clear All"
            }
        }
        div {
            style: "flex:1; overflow:auto; padding:10px 16px 14px 16px; display:flex; flex-direction:column; gap:10px;",
            if snapshot.notifications.is_empty() {
                div {
                    style: format!("font-size:12px; line-height:1.5; color:{};", snapshot.palette.muted),
                    "No notifications yet."
                }
            } else {
                for notification in snapshot.notifications.iter().cloned().rev() {
                    NotificationCard {
                        notification: notification.clone(),
                        palette: snapshot.palette,
                        on_clear: move |_| on_clear_notification.call(notification.id),
                    }
                }
            }
        }
    }
}

#[component]
fn ConnectRailBody(
    snapshot: RenderSnapshot,
    on_connect_ssh_custom: EventHandler<MouseEvent>,
    on_ssh_target_change: EventHandler<String>,
    on_ssh_prefix_change: EventHandler<String>,
) -> Element {
    rsx! {
        div {
            style: format!(
                "padding:16px 16px 10px 16px; font-size:12px; font-weight:700; letter-spacing:0.01em; color:{};",
                snapshot.palette.text
            ),
            "Connect SSH"
        }
        div {
            style: "flex:1; overflow:auto; padding:10px 16px 14px 16px; display:flex; flex-direction:column; gap:14px; \
             text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
            div {
                style: "display:flex; flex-direction:column; gap:10px; padding-bottom:10px;",
                div {
                    style: format!(
                        "font-size:11px; font-weight:700; letter-spacing:0.02em; color:{}; \
                         text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                        snapshot.palette.muted
                    ),
                    "Guide"
                }
                div {
                    style: format!(
                        "font-size:12px; line-height:1.55; color:{}; white-space:pre-wrap;",
                        snapshot.palette.text
                    ),
                    "Use `user@ip`, `user@host`, or an SSH config alias such as `dev` in the target field. Yggterm will SSH there and open or focus a live terminal session."
                }
                div {
                    style: format!(
                        "font-size:12px; line-height:1.55; color:{}; white-space:pre-wrap;",
                        snapshot.palette.text
                    ),
                    "The prefix field is optional. Think of it as the command that should run immediately after SSH lands on the remote machine."
                }
                div {
                    style: format!(
                        "font-size:12px; line-height:1.55; color:{}; white-space:pre-wrap;",
                        snapshot.palette.text
                    ),
                    "Example: if `dev` is your SSH host and the real work happens inside an LXC guest there, enter `dev` as the target and use `lxc exec yggdrasil -- bash` as the prefix. Yggterm will SSH into `dev`, run that prefix, and continue from inside the container."
                }
                div {
                    style: format!(
                        "font-size:12px; line-height:1.55; color:{}; white-space:pre-wrap;",
                        snapshot.palette.text
                    ),
                    "The same pattern works for tmux (`tmux new-session -A -s yggterm`), Docker (`docker exec -it web sh`), or systemd/machinectl shells (`sudo machinectl shell prod /bin/bash`)."
                }
            }
            div {
                style: format!(
                    "display:flex; flex-direction:column; gap:8px; padding-bottom:10px;"
                ),
                div {
                    style: format!(
                        "font-size:11px; font-weight:700; letter-spacing:0.02em; color:{}; \
                         text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                        snapshot.palette.muted
                    ),
                    "Target"
                }
                input {
                    r#type: "text",
                    value: "{snapshot.ssh_connect_target}",
                    placeholder: "dev or pi@jojo or user@192.168.1.15",
                    style: format!(
                        "height:36px; padding:0 12px; border:1px solid {}; border-radius:10px; background:{}; color:{}; \
                         font-size:12px; outline:none; box-shadow: inset 0 1px 0 rgba(255,255,255,0.55);",
                        snapshot.palette.border, snapshot.palette.panel, snapshot.palette.text
                    ),
                    oninput: move |evt| on_ssh_target_change.call(evt.value()),
                }
            }
            div {
                style: "display:flex; flex-direction:column; gap:8px; padding-bottom:10px;",
                div {
                    style: format!(
                        "font-size:11px; font-weight:700; letter-spacing:0.02em; color:{}; \
                         text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                        snapshot.palette.muted
                    ),
                    "Optional Prefix"
                }
                input {
                    r#type: "text",
                    value: "{snapshot.ssh_connect_prefix}",
                    placeholder: "Optional prefix, e.g. sudo machinectl shell prod",
                    style: format!(
                        "height:36px; padding:0 12px; border:1px solid {}; border-radius:10px; background:{}; color:{}; \
                         font-size:12px; outline:none; box-shadow: inset 0 1px 0 rgba(255,255,255,0.55);",
                        snapshot.palette.border, snapshot.palette.panel, snapshot.palette.text
                    ),
                    oninput: move |evt| on_ssh_prefix_change.call(evt.value()),
                }
            }
            div {
                style: "display:flex; flex-direction:column; gap:8px; padding-bottom:10px;",
                button {
                    style: primary_action_style(snapshot.palette),
                    onclick: move |evt| on_connect_ssh_custom.call(evt),
                    div {
                        style: "display:flex; flex-direction:column; align-items:flex-start; gap:3px; min-width:0;",
                        span {
                            style: "font-size:12px; font-weight:800; color:white;",
                            "Proceed ->"
                        }
                        span {
                            style: "font-size:11px; line-height:1.35; color:rgba(255,255,255,0.88); white-space:pre-wrap;",
                            "Yggterm will open or focus a terminal session for this target."
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn NotificationCard(
    notification: ToastNotification,
    palette: Palette,
    on_clear: EventHandler<MouseEvent>,
) -> Element {
    let (tone_accent, tone_fg) = notification_tone_colors(notification.tone, palette);
    rsx! {
        div {
            style: format!(
                "display:flex; flex-direction:column; gap:7px; padding:12px 12px 11px 12px; border-radius:14px; \
                 background:rgba(249,250,252,0.86); backdrop-filter: blur(28px) saturate(165%); \
                 -webkit-backdrop-filter: blur(28px) saturate(165%); box-shadow: 0 18px 38px rgba(49,67,82,0.14), inset 0 0 0 1px rgba(255,255,255,0.72);",
            ),
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:8px;",
                div {
                    style: "display:flex; align-items:center; gap:8px; min-width:0;",
                    div {
                        style: format!(
                            "width:8px; height:8px; border-radius:999px; background:{}; flex:none;",
                            tone_accent
                        ),
                    }
                    div {
                        style: format!(
                            "font-size:12px; font-weight:700; color:{}; text-rendering:optimizeLegibility; \
                             -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                            tone_fg
                        ),
                        "{notification.title}"
                    }
                }
                button {
                    style: format!(
                        "width:22px; height:22px; border:none; border-radius:8px; background:rgba(241,244,247,0.92); color:{}; font-size:12px; font-weight:700;",
                        tone_fg
                    ),
                    onclick: move |evt| on_clear.call(evt),
                    "×"
                }
            }
            div {
                style: format!(
                    "font-size:11px; line-height:1.45; color:{}; text-rendering:optimizeLegibility; \
                     -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                    palette.text
                ),
                "{notification.message}"
            }
        }
    }
}

#[component]
fn ContextMenuOverlay(
    row: BrowserRow,
    position: (f64, f64),
    selected_row: Option<BrowserRow>,
    selected_tree_paths: Vec<String>,
    palette: Palette,
    on_close: EventHandler<MouseEvent>,
    on_create_group: EventHandler<MouseEvent>,
    on_create_group_codex: EventHandler<MouseEvent>,
    on_create_group_shell: EventHandler<MouseEvent>,
    on_create_group_document: EventHandler<MouseEvent>,
    on_create_group_recipe: EventHandler<MouseEvent>,
    on_move_selected_document_here: EventHandler<MouseEvent>,
    on_create_note: EventHandler<MouseEvent>,
    on_create_recipe: EventHandler<MouseEvent>,
    on_regenerate: EventHandler<MouseEvent>,
    on_delete_item: EventHandler<MouseEvent>,
) -> Element {
    let menu_left = position.0.clamp(10.0, 1400.0);
    let menu_top = position.1.clamp(48.0, 980.0);
    let drag_paths = if selected_tree_paths.is_empty() {
        selected_row
            .as_ref()
            .filter(|selected| is_workspace_row(selected))
            .map(|selected| vec![selected.full_path.clone()])
            .unwrap_or_default()
    } else {
        selected_tree_paths
    };
    let can_move_selected_document = valid_drop_target(&drag_paths, &row);
    let can_create_in_context = !row.full_path.starts_with("__live_")
        && matches!(
            row.kind,
            BrowserRowKind::Group | BrowserRowKind::Document | BrowserRowKind::Separator
        );
    let selected_count = drag_paths.len().max(1);
    let menu_title = if selected_count > 1 && drag_paths.iter().any(|path| path == &row.full_path) {
        format!("{selected_count} selected items")
    } else {
        row.label.clone()
    };
    rsx! {
        div {
            style: "position:fixed; inset:0; z-index:90; background:transparent;",
            onclick: move |evt| on_close.call(evt),
            div {
                style: format!(
                    "position:absolute; top:{}px; left:{}px; min-width:188px; max-width:220px; padding:6px; border-radius:10px; \
                     background:rgba(248,249,252,0.98); box-shadow: 0 18px 38px rgba(57,78,98,0.18), inset 0 0 0 1px rgba(214,220,228,0.9); \
                     backdrop-filter: blur(20px) saturate(150%); -webkit-backdrop-filter: blur(20px) saturate(150%);",
                    menu_top, menu_left
                ),
                onmousedown: |evt| evt.stop_propagation(),
                onclick: |evt| evt.stop_propagation(),
                div {
                    style: format!("padding:6px 12px 8px 12px; font-size:11px; font-weight:700; color:{}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;", palette.muted),
                    "{menu_title}"
                }
                if can_create_in_context {
                    if can_move_selected_document {
                        button {
                            style: context_menu_action_style(palette, true),
                            onclick: move |evt| on_move_selected_document_here.call(evt),
                            "Move Selected Here"
                        }
                    }
                    button {
                        style: context_menu_action_style(palette, false),
                        onclick: move |evt| on_create_group.call(evt),
                        "Add Folder"
                    }
                    button {
                        style: context_menu_action_style(palette, false),
                        onclick: move |evt| on_create_group_codex.call(evt),
                        "New Session"
                    }
                    button {
                        style: context_menu_action_style(palette, false),
                        onclick: move |evt| on_create_group_shell.call(evt),
                        "New Terminal"
                    }
                    button {
                        style: context_menu_action_style(palette, true),
                        onclick: move |evt| on_create_group_document.call(evt),
                        "New Paper"
                    }
                    div {
                        style: format!("height:1px; margin:6px 4px; background:{}; opacity:0.7;", palette.border),
                    }
                    button {
                        style: context_menu_action_style(palette, false),
                        onclick: move |evt| on_create_group_recipe.call(evt),
                        "Add Separator"
                    }
                } else if row.kind == BrowserRowKind::Session {
                    button {
                        style: context_menu_action_style(palette, false),
                        onclick: move |evt| on_create_note.call(evt),
                        "New Paper"
                    }
                    button {
                        style: context_menu_action_style(palette, false),
                        onclick: move |evt| on_create_recipe.call(evt),
                        "New Terminal Plan"
                    }
                }
                if row.kind == BrowserRowKind::Session {
                    button {
                        style: context_menu_action_style(palette, true),
                        onclick: move |evt| on_regenerate.call(evt),
                        "Regenerate Title"
                    }
                }
                if is_workspace_row(&row) {
                    div {
                        style: format!("height:1px; margin:6px 4px; background:{}; opacity:0.7;", palette.border),
                    }
                    button {
                        style: context_menu_action_style_destructive(palette),
                        onclick: move |evt| on_delete_item.call(evt),
                        "Delete…"
                    }
                }
            }
        }
    }
}

#[component]
fn DeleteConfirmOverlay(
    pending: PendingDeleteDialog,
    palette: Palette,
    on_cancel: EventHandler<MouseEvent>,
    on_confirm: EventHandler<MouseEvent>,
) -> Element {
    let item_count = pending.document_paths.len() + pending.group_paths.len();
    let preview = pending.labels.iter().take(4).cloned().collect::<Vec<_>>();
    rsx! {
        div {
            style: "position:fixed; inset:0; z-index:95; display:flex; align-items:center; justify-content:center; background:rgba(230,239,248,0.28); backdrop-filter: blur(18px) saturate(130%); -webkit-backdrop-filter: blur(18px) saturate(130%);",
            onclick: move |evt| on_cancel.call(evt),
            div {
                style: format!(
                    "width:min(460px, calc(100vw - 40px)); display:flex; flex-direction:column; gap:14px; \
                     padding:22px; border-radius:18px; background:rgba(250,252,255,0.96); color:{}; \
                     box-shadow:0 24px 54px rgba(55,83,112,0.18), inset 0 0 0 1px rgba(214,223,232,0.9); \
                     font-family:{};",
                    palette.text,
                    interface_font_family()
                ),
                onmousedown: |evt| evt.stop_propagation(),
                onclick: |evt| evt.stop_propagation(),
                div {
                    style: "display:flex; flex-direction:column; gap:6px;",
                    div {
                        style: format!(
                            "font-size:18px; font-weight:700; letter-spacing:-0.01em; color:{};",
                            palette.text
                        ),
                        if pending.hard_delete { "Delete Permanently?" } else { "Delete Selected Items?" }
                    }
                    div {
                        style: format!("font-size:12px; line-height:1.6; color:{};", palette.muted),
                        if pending.hard_delete {
                            "This will permanently remove the selected items from the workspace tree."
                        } else {
                            "This will remove the selected items from the workspace tree. Hold Shift while pressing Delete to skip this dialog."
                        }
                    }
                }
                div {
                    style: "display:flex; flex-direction:column; gap:6px; max-height:180px; overflow:auto; padding-right:4px;",
                    for label in preview {
                        div {
                            style: format!("font-size:12px; line-height:1.5; color:{}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;", palette.text),
                            "• {label}"
                        }
                    }
                    if item_count > 4 {
                        div {
                            style: format!("font-size:11px; color:{};", palette.muted),
                            "+ {item_count - 4} more"
                        }
                    }
                }
                div {
                    style: "display:flex; justify-content:flex-end; gap:10px;",
                    button {
                        style: cancel_confirm_button_style(palette),
                        onclick: move |evt| on_cancel.call(evt),
                        "Cancel"
                    }
                    button {
                        style: delete_confirm_button_style(palette, pending.hard_delete),
                        onclick: move |evt| on_confirm.call(evt),
                        if pending.hard_delete { "Delete Permanently" } else { "Delete" }
                    }
                }
            }
        }
    }
}

#[component]
fn ToastViewport(
    notifications: Vec<ToastNotification>,
    palette: Palette,
    right_inset: usize,
    on_clear_notification: EventHandler<u64>,
) -> Element {
    let now = current_millis();
    let items = notifications
        .into_iter()
        .rev()
        .filter(|notification| now.saturating_sub(notification.created_at_ms) <= 7000)
        .filter(|notification| notification.tone != NotificationTone::Info)
        .take(3)
        .collect::<Vec<_>>();
    rsx! {
        div {
            style: format!(
                "position:fixed; top:56px; right:{}px; z-index:80; display:flex; flex-direction:column; gap:10px; width:280px; pointer-events:none;",
                right_inset
            ),
            for notification in items {
                div {
                    style: "pointer-events:auto; animation:yggterm-toast-fade 7s ease forwards;",
                    NotificationCard {
                        notification: notification.clone(),
                        palette,
                        on_clear: move |_| on_clear_notification.call(notification.id),
                    }
                }
            }
        }
    }
}

fn toast_right_inset(right_panel_mode: RightPanelMode) -> usize {
    let _ = right_panel_mode;
    18
}

#[component]
fn SettingsField(
    label: String,
    value: String,
    placeholder: String,
    secret: bool,
    palette: Palette,
    on_change: EventHandler<String>,
) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:6px;",
            div {
                style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", palette.muted),
                "{label}"
            }
            input {
                r#type: if secret { "password" } else { "text" },
                value: "{value}",
                placeholder: "{placeholder}",
                style: settings_input_style(palette),
                oninput: move |evt| on_change.call(evt.value()),
            }
        }
    }
}

#[component]
fn NotificationSettingsSection(
    palette: Palette,
    selected: NotificationDeliveryMode,
    sound_enabled: bool,
    on_select: EventHandler<NotificationDeliveryMode>,
    on_change: EventHandler<bool>,
) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:8px;",
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:8px;",
                div {
                    style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", palette.muted),
                    "Notifications"
                }
                span {
                    style: format!("font-size:10px; font-weight:700; color:{};", palette.accent),
                    "In-App Recommended"
                }
            }
            div {
                style: format!(
                    "display:flex; flex-direction:column; gap:10px; padding:10px; border-radius:14px; \
                     background:rgba(255,255,255,0.56); box-shadow: inset 0 0 0 1px rgba(196,214,228,0.42);"
                ),
                div {
                    style: format!(
                        "display:flex; align-items:center; gap:5px; height:34px; padding:4px; border-radius:999px; \
                         background:rgba(236,242,247,0.94); box-shadow: inset 0 0 0 1px rgba(210,221,232,0.92);"
                    ),
                    button {
                        style: segmented_pill_button_style(palette, selected == NotificationDeliveryMode::InApp),
                        onclick: move |_| on_select.call(NotificationDeliveryMode::InApp),
                        "App"
                    }
                    button {
                        style: segmented_pill_button_style(palette, selected == NotificationDeliveryMode::Both),
                        onclick: move |_| on_select.call(NotificationDeliveryMode::Both),
                        "Both"
                    }
                    button {
                        style: segmented_pill_button_style(palette, selected == NotificationDeliveryMode::System),
                        onclick: move |_| on_select.call(NotificationDeliveryMode::System),
                        "System"
                    }
                }
                div {
                    style: "display:flex; align-items:center; justify-content:space-between; gap:10px;",
                    div {
                        style: format!("font-size:11px; font-weight:700; color:{};", palette.muted),
                        "Sound"
                    }
                    button {
                        style: inline_toggle_style(palette, sound_enabled),
                        onclick: move |_| on_change.call(!sound_enabled),
                        span {
                            style: format!("font-size:10px; font-weight:700; color:{};", if sound_enabled { palette.accent } else { palette.muted }),
                            if sound_enabled { "On" } else { "Off" }
                        }
                        div {
                            style: format!(
                                "width:34px; height:18px; border-radius:999px; background:{}; display:flex; align-items:center; justify-content:{}; padding:0 2px;",
                                if sound_enabled { palette.accent } else { "rgba(189,201,212,0.92)" },
                                if sound_enabled { "flex-end" } else { "flex-start" }
                            ),
                            div {
                                style: "width:14px; height:14px; border-radius:999px; background:white; box-shadow:0 2px 8px rgba(36,48,58,0.18);",
                            }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ZoomSettingRow(
    label: String,
    percent: i32,
    palette: Palette,
    on_decrease: EventHandler<MouseEvent>,
    on_increase: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:6px;",
            div {
                style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", palette.muted),
                "{label}"
            }
            div {
                style: format!(
                    "display:flex; align-items:center; justify-content:space-between; height:32px; padding:0 6px; \
                     border:none; border-radius:10px; background:rgba(255,255,255,0.58); box-shadow: inset 0 0 0 1px rgba(255,255,255,0.34);"
                ),
                button {
                    style: zoom_button_style(palette),
                    onclick: move |evt| on_decrease.call(evt),
                    "−"
                }
                span {
                    style: format!("font-size:12px; font-weight:600; color:{};", palette.text),
                    "{percent}"
                }
                button {
                    style: zoom_button_style(palette),
                    onclick: move |evt| on_increase.call(evt),
                    "+"
                }
            }
        }
    }
}

#[component]
fn MetadataGroup(title: String, entries: Vec<SessionMetadataEntry>, palette: Palette) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:8px; padding-bottom:10px;",
            div {
                style: format!(
                    "font-size:11px; font-weight:700; letter-spacing:0.02em; color:{}; \
                     text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                    palette.muted
                ),
                "{title}"
            }
            for entry in entries.into_iter() {
                div {
                    style: "display:flex; flex-direction:column; gap:4px;",
                    span {
                        style: format!(
                            "font-size:11px; font-weight:600; color:{}; text-rendering:optimizeLegibility; \
                             -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                            palette.muted
                        ),
                        "{entry.label}"
                    }
                    span {
                        style: format!(
                            "font-size:12px; font-weight:500; color:{}; white-space:pre-wrap; line-height:1.5; \
                             text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                            palette.text
                        ),
                        "{entry.value}"
                    }
                }
            }
        }
    }
}

fn palette(theme: UiTheme) -> Palette {
    match theme {
        UiTheme::ZedLight => Palette {
            shell: "rgba(244,248,250,0.92)",
            titlebar: "transparent",
            sidebar: "transparent",
            sidebar_hover: "rgba(134,186,202,0.14)",
            panel: "#ffffff",
            panel_alt: "rgba(255,255,255,0.68)",
            border: "#dfe5ea",
            text: "#24303a",
            muted: "#6f7c86",
            accent: "#2f7cf6",
            accent_soft: "rgba(114,190,215,0.18)",
            gradient: "linear-gradient(180deg, rgba(232,243,248,0.94) 0%, rgba(232,244,238,0.90) 48%, rgba(237,240,244,0.94) 100%)",
            close_hover: "#e81123",
            control_hover: "rgba(36,48,58,0.10)",
            shadow: "0 24px 52px rgba(72,102,118,0.16)",
            panel_shadow: "0 18px 44px rgba(69,108,136,0.18)",
        },
        UiTheme::ZedDark => Palette {
            shell: "rgba(39,46,52,0.92)",
            titlebar: "transparent",
            sidebar: "transparent",
            sidebar_hover: "rgba(131,198,205,0.14)",
            panel: "#f8fafc",
            panel_alt: "rgba(255,255,255,0.14)",
            border: "#3b4755",
            text: "#dce6ee",
            muted: "#9fb0bd",
            accent: "#73b9ff",
            accent_soft: "rgba(115,185,255,0.14)",
            gradient: "linear-gradient(180deg, rgba(68,95,106,0.92) 0%, rgba(75,102,94,0.88) 52%, rgba(58,65,73,0.94) 100%)",
            close_hover: "#e81123",
            control_hover: "rgba(255,255,255,0.08)",
            shadow: "0 26px 72px rgba(0,0,0,0.34)",
            panel_shadow: "0 22px 52px rgba(0,0,0,0.22)",
        },
    }
}

fn shell_style(palette: Palette, radius: u8) -> String {
    format!(
        "position:fixed; inset:0; display:flex; flex-direction:column; overflow:hidden; \
         border-radius:{}px; background-color:{}; background-image:{}; box-shadow:{}; backdrop-filter: blur(30px) saturate(165%); \
         -webkit-backdrop-filter: blur(30px) saturate(165%); font-family:{};",
        radius,
        palette.shell,
        palette.gradient,
        palette.shadow,
        interface_font_family()
    )
}

fn search_style(palette: Palette) -> String {
    format!(
        "width:min(560px, 100%); height:32px; padding:0 12px; border-radius:8px; \
         border:none; background:rgba(255,255,255,0.66); color:{}; outline:none; font-size:12px; \
         box-shadow: inset 0 0 0 1px rgba(255,255,255,0.36); user-select:text; -webkit-user-select:text;",
        palette.text
    )
}

fn icon_button_style(palette: Palette) -> String {
    format!(
        "width:28px; height:28px; border:none; border-radius:8px; background:transparent; color:{}; font-size:13px; \
         user-select:none; -webkit-user-select:none; pointer-events:auto;",
        palette.muted
    )
}

fn utility_icon_style(palette: Palette, selected: bool) -> String {
    utility_icon_style_sized(palette, selected, 13)
}

fn utility_icon_style_sized(palette: Palette, selected: bool, font_size_px: u8) -> String {
    format!(
        "width:28px; height:28px; border:none; border-radius:10px; background:transparent; color:{}; font-size:{}px; font-weight:{}; \
         user-select:none; -webkit-user-select:none; pointer-events:auto;",
        if selected {
            palette.accent
        } else {
            palette.muted
        },
        font_size_px,
        if selected { 800 } else { 700 }
    )
}

fn connect_button_style(palette: Palette, selected: bool) -> String {
    format!(
        "height:28px; padding:0 11px; border:none; border-radius:10px; background:transparent; color:{}; \
         font-size:11px; font-weight:700; white-space:nowrap; user-select:none; -webkit-user-select:none; pointer-events:auto;",
        if selected {
            palette.accent
        } else {
            palette.muted
        }
    )
}

fn chip_style(palette: Palette, selected: bool) -> String {
    format!(
        "height:24px; padding:0 10px; border-radius:999px; border:1px solid {}; background:{}; \
         color:{}; font-size:11px; font-weight:600;",
        if selected {
            palette.accent
        } else {
            "rgba(255,255,255,0.10)"
        },
        if selected {
            palette.accent_soft
        } else {
            "rgba(255,255,255,0.28)"
        },
        if selected {
            palette.text
        } else {
            palette.muted
        }
    )
}

fn primary_action_style(palette: Palette) -> String {
    format!(
        "width:100%; border:none; border-radius:12px; background:{}; color:white; padding:10px 12px; text-align:left; \
         box-shadow: 0 12px 28px rgba(47,124,246,0.24), inset 0 0 0 1px rgba(255,255,255,0.18); \
         text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased;",
        palette.accent
    )
}

fn toggle_slider_style(_palette: Palette) -> String {
    format!(
        "display:flex; align-items:center; gap:4px; padding:3px; border:none; border-radius:999px; background:rgba(255,255,255,0.34); box-shadow: inset 0 0 0 1px rgba(255,255,255,0.28); user-select:none; -webkit-user-select:none;"
    )
}

fn toggle_slider_end_style(palette: Palette, selected: bool) -> String {
    format!(
        "height:26px; min-width:82px; padding:0 12px; border:none; border-radius:999px; background:{}; color:{}; font-size:11px; font-weight:700; \
         user-select:none; -webkit-user-select:none;",
        if selected {
            palette.panel
        } else {
            "transparent"
        },
        if selected {
            palette.text
        } else {
            palette.muted
        }
    )
}

fn settings_input_style(palette: Palette) -> String {
    format!(
        "height:30px; padding:0 10px; border:none; border-radius:8px; background:rgba(255,255,255,0.58); \
         color:{}; outline:none; font-size:11px; box-shadow: inset 0 0 0 1px rgba(255,255,255,0.34);",
        palette.text
    )
}

fn notification_tone_colors(
    tone: NotificationTone,
    palette: Palette,
) -> (&'static str, &'static str) {
    match tone {
        NotificationTone::Info => (palette.accent, "#315066"),
        NotificationTone::Success => ("#2f9e62", "#315066"),
        NotificationTone::Warning => ("#d79b24", "#315066"),
        NotificationTone::Error => ("#d95c5c", "#315066"),
    }
}

fn zoom_button_style(palette: Palette) -> String {
    format!(
        "width:22px; height:22px; border:none; border-radius:8px; background:rgba(255,255,255,0.36); \
         color:{}; font-size:13px; font-weight:700; display:inline-flex; align-items:center; justify-content:center;",
        palette.text
    )
}

fn segmented_pill_button_style(palette: Palette, selected: bool) -> String {
    format!(
        "flex:1; height:26px; border:none; border-radius:999px; background:{}; color:{}; font-size:11px; font-weight:{}; \
         display:inline-flex; align-items:center; justify-content:center; box-shadow:{};",
        if selected {
            "rgba(255,255,255,0.98)"
        } else {
            "transparent"
        },
        if selected {
            palette.text
        } else {
            palette.muted
        },
        if selected { 700 } else { 600 },
        if selected {
            "0 3px 10px rgba(89,111,132,0.12), inset 0 0 0 1px rgba(216,227,236,0.96)"
        } else {
            "none"
        }
    )
}

fn context_menu_action_style(palette: Palette, emphasized: bool) -> String {
    format!(
        "width:100%; height:32px; border:none; border-radius:8px; background:{}; color:{}; \
         font-size:12px; font-weight:{}; text-align:left; padding:0 12px; margin-bottom:4px;",
        if emphasized {
            "rgba(233,241,255,0.98)"
        } else {
            "transparent"
        },
        if emphasized {
            palette.accent
        } else {
            palette.text
        },
        if emphasized { 700 } else { 600 }
    )
}

fn context_menu_action_style_destructive(_palette: Palette) -> String {
    format!(
        "width:100%; height:32px; border:none; border-radius:8px; background:transparent; color:{}; \
         font-size:12px; font-weight:700; text-align:left; padding:0 12px; margin-bottom:4px;",
        "#c23f4d"
    )
}

fn cancel_confirm_button_style(_palette: Palette) -> String {
    "height:34px; padding:0 16px; border:none; border-radius:12px; background:#5fa8ff; color:#ffffff; \
     font-size:12px; font-weight:700;"
        .to_string()
}

fn delete_confirm_button_style(_palette: Palette, hard_delete: bool) -> String {
    format!(
        "height:34px; padding:0 16px; border:none; border-radius:12px; background:{}; color:#ffffff; \
         font-size:12px; font-weight:700;",
        if hard_delete { "#b3263f" } else { "#c23f4d" }
    )
}

fn join_debug_paths(mut paths: Vec<String>) -> String {
    if paths.is_empty() {
        return "none".to_string();
    }
    paths.sort();
    if paths.len() > 4 {
        let extra = paths.len() - 4;
        paths.truncate(4);
        format!("{}, +{} more", paths.join(", "), extra)
    } else {
        paths.join(", ")
    }
}

fn inline_toggle_style(_palette: Palette, enabled: bool) -> String {
    format!(
        "display:flex; align-items:center; gap:10px; height:28px; padding:0 2px 0 0; border:none; background:transparent; \
         justify-content:flex-end; width:auto; opacity:{};",
        if enabled { "1" } else { "0.92" }
    )
}

fn clamp_zoom_value(value: f32) -> f32 {
    value.clamp(7.0, 20.0)
}

fn clamp_zoom_value_main(value: f32) -> f32 {
    value.clamp(5.0, 20.0)
}

fn zoom_percent(value: f32, base: f32) -> i32 {
    ((value / base) * 100.0).round() as i32
}

fn zoom_percent_f32(value: f32, base: f32) -> f32 {
    (value / base) * 100.0
}

fn metadata_value(session: &ManagedSessionView, label: &str) -> String {
    session
        .metadata
        .iter()
        .find(|entry| entry.label == label)
        .map(|entry| entry.value.clone())
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn interface_font_family() -> &'static str {
    "\"Inter Variable\", \"Inter\", system-ui, sans-serif"
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn interface_font_family() -> &'static str {
    "system-ui, sans-serif"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_zoom_clamps_to_fifty_percent_floor() {
        assert_eq!(clamp_zoom_value_main(0.0), 5.0);
        assert_eq!(clamp_zoom_value_main(4.0), 5.0);
        assert_eq!(zoom_percent(clamp_zoom_value_main(4.0), 10.0), 50);
    }

    #[test]
    fn interface_zoom_clamps_to_fifty_percent_floor() {
        assert_eq!(clamp_zoom_value(0.0), 7.0);
        assert_eq!(clamp_zoom_value(6.0), 7.0);
        assert_eq!(zoom_percent(clamp_zoom_value(6.0), 14.0), 50);
    }

    #[test]
    fn terminal_theme_uses_requested_font_size_down_to_floor() {
        let theme = terminal_theme(palette(UiTheme::ZedLight), 5.0);
        assert_eq!(theme.font_size, 5.0);

        let clamped = terminal_theme(palette(UiTheme::ZedLight), 2.0);
        assert_eq!(clamped.font_size, 5.0);
    }

    #[test]
    fn terminal_eval_script_bakes_font_size_into_xterm_constructor() {
        let theme = terminal_theme(palette(UiTheme::ZedLight), 13.0);
        let script = terminal_eval_script("yggterm-terminal-test", &theme);
        assert!(script.contains("fontSize: 13"));
    }

    #[test]
    fn terminal_apply_script_updates_live_xterm_font_size() {
        let theme = terminal_theme(palette(UiTheme::ZedLight), 5.0);
        let script = terminal_apply_script("yggterm-terminal-test", &theme);
        assert!(script.contains("entry.term.options.fontSize = 5"));
    }

    #[test]
    fn terminal_instance_key_changes_with_font_size() {
        let small = terminal_instance_key("/tmp/example", 10.0);
        let large = terminal_instance_key("/tmp/example", 20.0);
        assert_ne!(small, large);
        assert!(small.ends_with("-10.0"));
        assert!(large.ends_with("-20.0"));
    }

    #[test]
    fn workspace_rows_exclude_live_groups() {
        let live_group = BrowserRow {
            kind: BrowserRowKind::Group,
            full_path: "__live_shells__".to_string(),
            label: "shell sessions".to_string(),
            detail_label: String::new(),
            document_kind: None,
            group_kind: None,
            session_title: None,
            depth: 1,
            host_label: String::new(),
            descendant_sessions: 1,
            expanded: true,
            session_id: None,
            session_cwd: None,
        };
        let folder = BrowserRow {
            kind: BrowserRowKind::Group,
            full_path: "/home/pi/gh/notes/folder-a".to_string(),
            label: "folder-a".to_string(),
            detail_label: String::new(),
            document_kind: None,
            group_kind: Some(WorkspaceGroupKind::Folder),
            session_title: None,
            depth: 2,
            host_label: String::new(),
            descendant_sessions: 0,
            expanded: true,
            session_id: None,
            session_cwd: None,
        };
        assert!(!is_workspace_row(&live_group));
        assert!(is_workspace_row(&folder));
    }

    #[test]
    fn drop_target_rejects_self_and_descendants() {
        let target = BrowserRow {
            kind: BrowserRowKind::Group,
            full_path: "/home/pi/gh/notes/folder-a/sub".to_string(),
            label: "sub".to_string(),
            detail_label: String::new(),
            document_kind: None,
            group_kind: Some(WorkspaceGroupKind::Folder),
            session_title: None,
            depth: 3,
            host_label: String::new(),
            descendant_sessions: 0,
            expanded: true,
            session_id: None,
            session_cwd: None,
        };
        assert!(!valid_drop_target(
            &["/home/pi/gh/notes/folder-a".to_string()],
            &target
        ));
        assert!(!valid_drop_target(
            &["/home/pi/gh/notes/folder-a/sub".to_string()],
            &target
        ));
        assert!(valid_drop_target(
            &["/home/pi/gh/notes/other-paper".to_string()],
            &target
        ));
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn interface_font_family() -> &'static str {
    "system-ui, sans-serif"
}
