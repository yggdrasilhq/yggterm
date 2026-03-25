use crate::chrome::{
    ChromePalette, HoveredChromeControl as HoveredControl, TitlebarChrome,
    WindowControlsStrip, search_input_style,
};
use crate::drag_tree::{
    DragDropPlacement, DragDropTarget, TreeDropPlacement as WorkspaceDropPlacement,
    TreeReorderItem, TreeReorderPlanItem, build_tree_reorder_plan,
    resolve_drag_drop_target as resolve_tree_drag_drop_target,
    resolve_tree_drop_placement, tree_parent_path, tree_path_contains,
    valid_drop_target as valid_tree_drop_target,
};
use crate::drag_visuals::{DragGhostCard, DragGhostPalette, TreeDropZones};
use crate::notifications::{
    TOAST_CSS, ToastCard, ToastItem as ToastNotification, ToastPalette,
    ToastTone as NotificationTone, ToastViewport,
};
use crate::rails::{RailHeader, RailScrollBody, RailSectionTitle, SideRailShell};
use crate::theme::{
    THEME_EDITOR_SWATCHES, append_theme_stop, clamp_theme_spec, default_theme_editor_spec,
    dominant_accent, gradient_css, preview_surface_css, shell_tint,
};
use crate::window_icon;
use anyhow::{Result, anyhow};
use dioxus::desktop::{
    Config, LogicalSize, WindowBuilder, WindowEvent as DesktopWindowEvent, use_window,
    use_wry_event_handler, window,
};
use dioxus::document;
use dioxus::html::{InteractionElementOffset, input_data::MouseButton};
use dioxus::prelude::*;
use keyboard_types::{Key, Modifiers};
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;
use tao::event::Event as TaoEvent;
use tao::event::ElementState;
use tao::keyboard::Key as TaoKey;
use tao::keyboard::KeyCode as TaoKeyCode;
use tao::window::ResizeDirection;
use tokio::task;
use tokio::time::sleep;
use tracing::{info, warn};
use yggterm_core::{
    AgentSessionProfile, AppSettings, BrowserRow, BrowserRowKind, InstallContext, PerfSpan,
    SessionBrowserState, SessionNode, SessionStore, UiTheme, WorkspaceDocumentInput,
    WorkspaceDocumentKind, WorkspaceGroupKind, check_for_update, save_settings_file,
    install_release_update, looks_like_generated_fallback_title, refresh_desktop_integration,
    unique_session_short_ids_for_pairs, update_command_hint, YgguiThemeSpec,
};
use yggterm_platform::DockRect;
use yggterm_server::{
    GhosttyTerminalHostMode, ManagedSessionView, PreviewTone, RemoteMachineHealth,
    RemoteMachineSnapshot, RemoteScannedSession, ServerEndpoint, ServerRuntimeStatus,
    ServerUiSnapshot, SessionKind, SessionMetadataEntry, SessionPreviewBlock,
    SessionRenderedSection, SshConnectTarget, TerminalBackend, WorkspaceViewMode, YggtermServer,
    cleanup_legacy_daemons, connect_ssh_custom, focus_live, open_remote_session,
    open_stored_session, ping, persist_remote_generated_copy, refresh_remote_machine,
    remove_ssh_target, request_terminal_launch, set_all_preview_blocks_folded,
    set_view_mode as daemon_set_view_mode, shutdown as daemon_shutdown,
    snapshot as daemon_snapshot, stage_remote_clipboard_png, start_command_session,
    start_local_session, start_local_session_at, status, switch_agent_session_mode,
    terminal_ensure, terminal_read, terminal_resize, terminal_write,
    toggle_preview_block as daemon_toggle_preview_block,
};

static BOOTSTRAP: OnceCell<ShellBootstrap> = OnceCell::new();
static PASSIVE_COPY_SUSPENDED: AtomicBool = AtomicBool::new(false);
const SIDE_RAIL_WIDTH: usize = 292;
const EDGE_RESIZE_HANDLE: usize = 5;
const CORNER_RESIZE_HANDLE: usize = 10;
const XTERM_CSS: &str = include_str!("../../../assets/xterm/xterm.css");
const XTERM_JS: &str = include_str!("../../../assets/xterm/xterm.js");
const XTERM_FIT_JS: &str = include_str!("../../../assets/xterm/addon-fit.js");
static XTERM_ASSETS_BOOTSTRAPPED: OnceCell<()> = OnceCell::new();
const TREE_LOADING_DOT_CSS: &str = "@keyframes yggterm-tree-loading-dot { 0%, 80%, 100% { opacity: 0.28; transform: translateY(0px); } 40% { opacity: 1; transform: translateY(-1px); } }";
const BACKGROUND_COPY_RETRY_MS: u64 = 300_000;
const BACKGROUND_COPY_CONTINUE_MS: u64 = 15_000;
const BACKGROUND_COPY_IDLE_MS: u64 = 120_000;
const THEME_EDITOR_PAD_SIZE: f64 = 286.0;
type WorkspaceReorderPlanItem = TreeReorderPlanItem<BrowserRowKind>;

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
    generated_summaries: BTreeMap<String, String>,
    title_requests_in_flight: HashSet<String>,
    precis_requests_in_flight: HashSet<String>,
    summary_requests_in_flight: HashSet<String>,
    passive_copy_suspended: bool,
    passive_copy_failures: HashSet<String>,
    copy_retry_after_ms: HashMap<String, u64>,
    terminal_image_paste_ms: HashMap<String, u64>,
    remote_machine_refresh_requests: HashSet<String>,
    drag_paths: Vec<String>,
    drag_hover_target: Option<DragDropTarget>,
    optimistic_drag_paths: Vec<String>,
    optimistic_drag_target: Option<DragDropTarget>,
    drag_pointer: Option<(f64, f64)>,
    suppress_tree_click_until_ms: u64,
    pending_delete: Option<PendingDeleteDialog>,
    tree_rename_path: Option<String>,
    tree_rename_value: String,
    theme_editor_open: bool,
    theme_editor_draft: YgguiThemeSpec,
    theme_editor_selected_stop: Option<usize>,
    theme_editor_drag_stop: Option<usize>,
    background_copy_scan_in_flight: bool,
    next_background_copy_scan_after_ms: u64,
    browser_tree_loading_in_flight: bool,
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

#[derive(Clone, PartialEq, Eq)]
struct PendingDeleteDialog {
    document_paths: Vec<String>,
    group_paths: Vec<String>,
    ssh_machine_keys: Vec<String>,
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
    show_loading_tree: bool,
    ssh_connect_target: String,
    ssh_connect_prefix: String,
    pending_update_restart: Option<PendingUpdateRestart>,
    last_terminal_debug: String,
    last_tree_debug: String,
    active_title: Option<String>,
    active_precis: Option<String>,
    active_summary: Option<String>,
    drag_paths: Vec<String>,
    drag_hover_target: Option<DragDropTarget>,
    optimistic_drag_paths: Vec<String>,
    optimistic_drag_target: Option<DragDropTarget>,
    drag_pointer: Option<(f64, f64)>,
    pending_delete: Option<PendingDeleteDialog>,
    tree_rename_path: Option<String>,
    tree_rename_value: String,
    theme_editor_open: bool,
    theme_editor_draft: YgguiThemeSpec,
    theme_editor_selected_stop: Option<usize>,
    theme_accent: String,
    shell_tint: String,
    shell_gradient: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeSelectionMode {
    Replace,
    ExtendRange,
    Toggle,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ContextMenuPlacement {
    left: Option<f64>,
    top: Option<f64>,
    right: Option<f64>,
    bottom: Option<f64>,
}

#[derive(Clone)]
struct CopyGenerationTarget {
    session_path: String,
    session_id: String,
    cwd: String,
    title: String,
    remote_context: Option<String>,
    remote_machine: Option<RemoteMachineSnapshot>,
}

#[derive(Clone)]
enum BackgroundCopyJob {
    Title(CopyGenerationTarget),
    Precis(CopyGenerationTarget),
    Summary(CopyGenerationTarget),
}

fn background_copy_retry_key(prefix: &str, session_path: &str) -> String {
    format!("{prefix}:{session_path}")
}

fn background_copy_retry_ready(shell: &ShellState, prefix: &str, session_path: &str) -> bool {
    let key = background_copy_retry_key(prefix, session_path);
    !PASSIVE_COPY_SUSPENDED.load(Ordering::Relaxed)
        && !shell.passive_copy_suspended
        && !shell.passive_copy_failures.contains(&key)
        && shell
            .copy_retry_after_ms
            .get(&key)
            .copied()
            .is_none_or(|retry_after| retry_after <= current_millis())
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

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum TerminalJsCommand {
    Reset {
        title: String,
        background: String,
        foreground: String,
        cursor: String,
        selection: String,
        black: String,
        red: String,
        green: String,
        yellow: String,
        blue: String,
        magenta: String,
        cyan: String,
        white: String,
        bright_black: String,
        bright_red: String,
        bright_green: String,
        bright_yellow: String,
        bright_blue: String,
        bright_magenta: String,
        bright_cyan: String,
        bright_white: String,
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
    Clipboard { action: String, chars: usize },
    ClipboardImageRequest,
    ClipboardError { action: String, message: String },
    Debug { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreviewContentBlock {
    Heading { level: u8, text: String },
    Paragraph(String),
    Bullet(String),
    Numbered { number: usize, text: String },
    Task { done: bool, text: String },
    Quote(String),
    Code { language: Option<String>, code: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MachineHealth {
    Healthy,
    Cached,
    Offline,
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
        PASSIVE_COPY_SUSPENDED.store(false, Ordering::Relaxed);
        let initial_yggui_theme = clamp_theme_spec(&bootstrap.settings.yggui_theme);
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
            generated_summaries: BTreeMap::new(),
            title_requests_in_flight: HashSet::new(),
            precis_requests_in_flight: HashSet::new(),
            summary_requests_in_flight: HashSet::new(),
            passive_copy_suspended: false,
            passive_copy_failures: HashSet::new(),
            copy_retry_after_ms: HashMap::new(),
            terminal_image_paste_ms: HashMap::new(),
            remote_machine_refresh_requests: HashSet::new(),
            drag_paths: Vec::new(),
            drag_hover_target: None,
            optimistic_drag_paths: Vec::new(),
            optimistic_drag_target: None,
            drag_pointer: None,
            suppress_tree_click_until_ms: 0,
            pending_delete: None,
            tree_rename_path: None,
            tree_rename_value: String::new(),
            theme_editor_open: false,
            theme_editor_draft: initial_yggui_theme,
            theme_editor_selected_stop: None,
            theme_editor_drag_stop: None,
            background_copy_scan_in_flight: false,
            next_background_copy_scan_after_ms: 0,
            browser_tree_loading_in_flight: true,
        };
        if let Some(path) = state.browser.selected_path().map(ToOwned::to_owned) {
            state.selected_tree_paths.insert(path.clone());
            state.selection_anchor = Some(path);
        }
        state.refresh_tree_debug("init");
        state.server_daemon_detail = state.bootstrap.server_daemon_detail.clone();
        state.hydrate_generated_copy_from_remote_cache();
        if !state.needs_initial_server_sync {
            state.seed_dynamic_top_level_expansions();
            state.ensure_active_session_visible();
            state.record_restore_issue_telemetry("init_state");
            state.record_preview_issue_telemetry("init_state");
        }
        if let Some(update) = state.pending_update_restart.clone() {
            state.last_action = format!("update {} ready", update.version);
            state.push_notification(
                NotificationTone::Success,
                "Update Ready",
                format!(
                    "Yggterm {} was installed. Restart when you are ready.",
                    update.version
                ),
            );
        }
        state.sync_browser_settings();
        state
    }

    fn snapshot(&self) -> RenderSnapshot {
        let active_theme_spec = if self.theme_editor_open {
            clamp_theme_spec(&self.theme_editor_draft)
        } else {
            clamp_theme_spec(&self.settings.yggui_theme)
        };
        let palette = palette(self.settings.theme);
        let mut expanded_paths = self.browser.expanded_path_set();
        expanded_paths.extend(self.active_session_visibility_paths());
        let live_sessions = self.server.live_sessions();
        let rows = merged_sidebar_rows(
            self.browser.rows(),
            self.server.remote_machines(),
            self.server.ssh_targets(),
            &live_sessions,
            &expanded_paths,
        );
        let selected_path = if let Some(active) = self.server.active_session() {
            if active.source == yggterm_server::SessionSource::LiveSsh
                || active.session_path.starts_with("remote-session://")
            {
                Some(active.session_path.clone())
            } else {
                self.browser.selected_path().map(ToOwned::to_owned)
            }
        } else {
            self.browser.selected_path().map(ToOwned::to_owned)
        };
        let active_session = self.server.active_session().cloned();
        let active_title = active_session
            .as_ref()
            .and_then(|session| resolved_session_title(self, session));
        let active_precis = active_session
            .as_ref()
            .and_then(|session| resolved_session_precis(self, session));
        let active_summary = active_session
            .as_ref()
            .and_then(|session| resolved_session_summary(self, session));

        RenderSnapshot {
            palette: palette,
            search_query: self.search_query.clone(),
            sidebar_open: self.sidebar_open,
            right_panel_mode: self.right_panel_mode,
            rows,
            selected_path,
            selected_row: self.browser.selected_row().cloned(),
            active_session,
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
            show_loading_tree: (self.needs_initial_server_sync && self.server_busy)
                || self.browser_tree_loading_in_flight,
            ssh_connect_target: self.ssh_connect_target.clone(),
            ssh_connect_prefix: self.ssh_connect_prefix.clone(),
            pending_update_restart: self.pending_update_restart.clone(),
            last_terminal_debug: self.last_terminal_debug.clone(),
            last_tree_debug: self.last_tree_debug.clone(),
            active_title,
            active_precis,
            active_summary,
            drag_paths: self.drag_paths.clone(),
            drag_hover_target: self.drag_hover_target.clone(),
            optimistic_drag_paths: self.optimistic_drag_paths.clone(),
            optimistic_drag_target: self.optimistic_drag_target.clone(),
            drag_pointer: self.drag_pointer,
            pending_delete: self.pending_delete.clone(),
            tree_rename_path: self.tree_rename_path.clone(),
            tree_rename_value: self.tree_rename_value.clone(),
            theme_editor_open: self.theme_editor_open,
            theme_editor_draft: self.theme_editor_draft.clone(),
            theme_editor_selected_stop: self.theme_editor_selected_stop,
            theme_accent: dominant_accent(&active_theme_spec, palette.accent),
            shell_tint: shell_tint(self.settings.theme, &active_theme_spec),
            shell_gradient: gradient_css(self.settings.theme, &active_theme_spec),
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
        let was_initial_sync = self.needs_initial_server_sync;
        match result {
            Ok((snapshot, message)) => {
                self.server.apply_snapshot(snapshot);
                self.hydrate_generated_copy_from_remote_cache();
                self.server_busy = false;
                self.needs_initial_server_sync = false;
                self.next_background_copy_scan_after_ms = 0;
                if was_initial_sync {
                    self.seed_dynamic_top_level_expansions();
                }
                self.ensure_active_session_visible();
                self.record_restore_issue_telemetry("apply_daemon_snapshot_result");
                self.record_preview_issue_telemetry("apply_daemon_snapshot_result");
                if let Some(message) = message {
                    self.last_action = message;
                }
            }
            Err(error) => {
                self.server_busy = false;
                self.needs_initial_server_sync = false;
                self.next_background_copy_scan_after_ms = current_millis() + 2_000;
                self.last_action = format!("server sync failed: {error}");
                self.push_notification(
                    NotificationTone::Error,
                    "Server Sync Failed",
                    error.to_string(),
                );
            }
        }
    }

    fn seed_dynamic_top_level_expansions(&mut self) {
        let mut paths = self
            .server
            .remote_machines()
            .iter()
            .map(|machine| format!("__remote_machine__/{}", machine.machine_key))
            .collect::<Vec<_>>();
        if self
            .server
            .live_sessions()
            .iter()
            .any(|session| session.kind != SessionKind::SshShell)
        {
            paths.push("__live_sessions__".to_string());
        }
        self.browser.ensure_expanded_paths(paths);
        self.sync_browser_settings();
    }

    fn ensure_active_session_visible(&mut self) {
        let remote_paths = self.active_session_visibility_paths();
        if !remote_paths.is_empty() {
            self.browser.ensure_expanded_paths(remote_paths);
        } else if let Some(active) = self.server.active_session().cloned() {
            self.browser.ensure_visible_path(&active.session_path);
        }
        if let Some(active) = self.server.active_session().cloned() {
            self.selected_tree_paths.clear();
            self.selected_tree_paths.insert(active.session_path.clone());
            self.selection_anchor = Some(active.session_path.clone());
            if !active.session_path.starts_with("remote-session://") {
                self.browser.select_path(active.session_path.clone());
            }
        }
        self.sync_browser_settings();
    }

    fn active_session_visibility_paths(&self) -> Vec<String> {
        let active_path = self
            .server
            .active_session()
            .map(|session| session.session_path.clone())
            .or_else(|| self.server.active_session_path().map(ToOwned::to_owned));
        let Some(active_path) = active_path else {
            return Vec::new();
        };
        let Some((machine_key, session_id)) = parse_remote_scanned_session_path(&active_path) else {
            return Vec::new();
        };
        let mut paths = vec![format!("__remote_machine__/{machine_key}")];
        let active_cwd = self.server.active_session().map(|active| metadata_value(active, "Cwd"));
        if let Some(machine) = self
            .server
            .remote_machines()
            .iter()
            .find(|machine| machine.machine_key == machine_key)
        {
            let cwd = active_cwd.unwrap_or_else(|| {
                machine
                    .sessions
                    .iter()
                    .find(|session| {
                        session.session_path == active_path || session.session_id == session_id
                    })
                    .map(|session| session.cwd.clone())
                    .unwrap_or_default()
            });
            paths.extend(
                compressed_remote_folder_paths(
                    &SidebarRemoteMachine {
                        key: machine.machine_key.clone(),
                        label: machine.label.clone(),
                        health: match machine.health {
                            RemoteMachineHealth::Healthy => MachineHealth::Healthy,
                            RemoteMachineHealth::Cached => MachineHealth::Cached,
                            RemoteMachineHealth::Offline => MachineHealth::Offline,
                        },
                        scanned_sessions: machine.sessions.clone(),
                    },
                    &cwd,
                )
                .into_iter()
                .map(|path| format!("__remote_folder__/{machine_key}{path}")),
            );
        } else {
            let cwd = active_cwd.unwrap_or_default();
            let mut current = String::new();
            for segment in cwd.split('/').filter(|segment| !segment.is_empty()) {
                current.push('/');
                current.push_str(segment);
                paths.push(format!("__remote_folder__/{machine_key}{current}"));
            }
        }
        paths
    }

    fn hydrate_generated_copy_from_remote_cache(&mut self) {
        for machine in self.server.remote_machines() {
            for session in &machine.sessions {
                if let Some(precis) = session.cached_precis.as_ref() {
                    self.generated_precis
                        .insert(session.session_path.clone(), precis.clone());
                }
                if let Some(summary) = session.cached_summary.as_ref() {
                    self.generated_summaries
                        .insert(session.session_path.clone(), summary.clone());
                }
            }
        }
    }

    fn select_row(&mut self, row: &BrowserRow) {
        self.browser.select_path(row.full_path.clone());
        self.record_ui_telemetry(
            "tree_activate",
            json!({
                "path": row.full_path,
                "kind": format!("{:?}", row.kind),
                "selected_paths": self.selected_tree_paths.iter().cloned().collect::<Vec<_>>(),
            }),
        );
        match row.kind {
            BrowserRowKind::Group => {
                if is_synthetic_sidebar_row(row) {
                    self.browser.toggle_virtual_group(&row.full_path);
                } else {
                    self.browser.toggle_group(&row.full_path);
                }
                self.suppress_tree_click_until_ms = current_millis().saturating_add(220);
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
        PASSIVE_COPY_SUSPENDED.store(false, Ordering::Relaxed);
        self.passive_copy_suspended = false;
        self.passive_copy_failures.clear();
        self.copy_retry_after_ms.clear();
        self.persist_settings();
        self.last_action = "updated LiteLLM endpoint".to_string();
    }

    fn update_litellm_api_key(&mut self, value: String) {
        self.settings.litellm_api_key = value;
        PASSIVE_COPY_SUSPENDED.store(false, Ordering::Relaxed);
        self.passive_copy_suspended = false;
        self.passive_copy_failures.clear();
        self.copy_retry_after_ms.clear();
        self.persist_settings();
        self.last_action = "updated LiteLLM API key".to_string();
    }

    fn update_interface_llm_model(&mut self, value: String) {
        self.settings.interface_llm_model = value;
        PASSIVE_COPY_SUSPENDED.store(false, Ordering::Relaxed);
        self.passive_copy_suspended = false;
        self.passive_copy_failures.clear();
        self.copy_retry_after_ms.clear();
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

    fn set_ui_theme(&mut self, theme: UiTheme) {
        self.settings.theme = theme;
        self.persist_settings();
        self.last_action = match theme {
            UiTheme::ZedLight => "light theme".to_string(),
            UiTheme::ZedDark => "dark theme".to_string(),
        };
    }

    fn open_theme_editor(&mut self) {
        self.theme_editor_open = true;
        self.theme_editor_draft = clamp_theme_spec(&self.settings.yggui_theme);
        self.theme_editor_selected_stop = None;
        self.theme_editor_drag_stop = None;
        self.last_action = "theme editor opened".to_string();
    }

    fn close_theme_editor(&mut self) {
        self.theme_editor_open = false;
        self.theme_editor_drag_stop = None;
        self.last_action = "theme editor closed".to_string();
    }

    fn save_theme_editor(&mut self) {
        self.settings.yggui_theme = clamp_theme_spec(&self.theme_editor_draft);
        self.theme_editor_open = false;
        self.theme_editor_drag_stop = None;
        self.persist_settings();
        self.last_action = "theme updated".to_string();
        self.push_notification(
            NotificationTone::Success,
            "Theme Updated",
            "Yggui shell theme applied.".to_string(),
        );
    }

    fn reset_theme_editor(&mut self) {
        self.theme_editor_draft = default_theme_editor_spec();
        self.theme_editor_selected_stop = self.theme_editor_draft.colors.first().map(|_| 0);
        self.theme_editor_drag_stop = None;
        self.last_action = "theme reset to base".to_string();
    }

    fn seed_theme_editor(&mut self) {
        self.theme_editor_draft = default_theme_editor_spec();
        self.theme_editor_selected_stop = self.theme_editor_draft.colors.first().map(|_| 0);
        self.last_action = "theme starter added".to_string();
    }

    fn add_theme_stop(&mut self, color: Option<&str>) {
        let next = append_theme_stop(&self.theme_editor_draft, color);
        if next.colors.len() == self.theme_editor_draft.colors.len() {
            return;
        }
        self.theme_editor_draft = next;
        self.theme_editor_selected_stop = self.theme_editor_draft.colors.len().checked_sub(1);
        self.last_action = format!("theme color {}", self.theme_editor_draft.colors.len());
    }

    fn add_theme_stop_at(&mut self, x: f32, y: f32) {
        self.add_theme_stop(None);
        if let Some(index) = self.theme_editor_selected_stop
            && let Some(stop) = self.theme_editor_draft.colors.get_mut(index)
        {
            stop.x = x.clamp(0.0, 1.0);
            stop.y = y.clamp(0.0, 1.0);
        }
        self.theme_editor_draft = clamp_theme_spec(&self.theme_editor_draft);
    }

    fn select_theme_stop(&mut self, index: usize) {
        if index < self.theme_editor_draft.colors.len() {
            self.theme_editor_selected_stop = Some(index);
        }
    }

    fn begin_theme_drag(&mut self, index: usize) {
        self.select_theme_stop(index);
        self.theme_editor_drag_stop = Some(index);
    }

    fn move_theme_stop(&mut self, x: f32, y: f32) {
        let Some(index) = self.theme_editor_drag_stop else {
            return;
        };
        if let Some(stop) = self.theme_editor_draft.colors.get_mut(index) {
            stop.x = x.clamp(0.0, 1.0);
            stop.y = y.clamp(0.0, 1.0);
        }
    }

    fn end_theme_drag(&mut self) {
        self.theme_editor_drag_stop = None;
    }

    fn remove_selected_theme_stop(&mut self) {
        let Some(index) = self.theme_editor_selected_stop else {
            return;
        };
        if index >= self.theme_editor_draft.colors.len() {
            return;
        }
        self.theme_editor_draft.colors.remove(index);
        self.theme_editor_selected_stop = if self.theme_editor_draft.colors.is_empty() {
            None
        } else {
            Some(index.min(self.theme_editor_draft.colors.len() - 1))
        };
        self.last_action = "theme color removed".to_string();
    }

    fn update_selected_theme_color(&mut self, color: String) {
        let Some(index) = self.theme_editor_selected_stop else {
            return;
        };
        if let Some(stop) = self.theme_editor_draft.colors.get_mut(index) {
            stop.color = color;
        }
        self.theme_editor_draft = clamp_theme_spec(&self.theme_editor_draft);
    }

    fn update_theme_brightness(&mut self, value: f32) {
        self.theme_editor_draft.brightness = value.clamp(0.0, 1.0);
    }

    fn update_theme_grain(&mut self, value: f32) {
        self.theme_editor_draft.grain = value.clamp(0.0, 1.0);
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
        self.record_ui_telemetry(
            "context_menu_open",
            json!({
                "row_path": self.context_menu_row.as_ref().map(|row| row.full_path.clone()),
                "row_kind": self.context_menu_row.as_ref().map(|row| format!("{:?}", row.kind)),
                "position": { "x": position.0, "y": position.1 },
                "selected_paths": self.selected_tree_paths.iter().cloned().collect::<Vec<_>>(),
            }),
        );
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

    fn clear_job_notification(&mut self, job_key: &str) {
        self.notifications
            .retain(|notification| notification.job_key.as_deref() != Some(job_key));
    }

    fn clear_notifications(&mut self) {
        self.notifications.clear();
    }

    fn select_tree_row(&mut self, row: &BrowserRow, mode: TreeSelectionMode) {
        if mode == TreeSelectionMode::ExtendRange && is_workspace_row(row) {
            self.extend_tree_selection(row);
            return;
        }
        if mode == TreeSelectionMode::Toggle && is_workspace_row(row) {
            if self.selected_tree_paths.contains(&row.full_path) {
                self.selected_tree_paths.remove(&row.full_path);
            } else {
                self.selected_tree_paths.insert(row.full_path.clone());
                self.selection_anchor = Some(row.full_path.clone());
            }
            if self.selected_tree_paths.is_empty() {
                self.selected_tree_paths.insert(row.full_path.clone());
                self.selection_anchor = Some(row.full_path.clone());
            }
            self.browser.select_path(row.full_path.clone());
            if cfg!(debug_assertions) {
                info!(
                    path=%row.full_path,
                    selected=%self.selected_tree_paths.len(),
                    "tree row toggled"
                );
            }
            self.record_ui_telemetry(
                "tree_select_toggle",
                json!({
                    "path": row.full_path,
                    "selected_paths": self.selected_tree_paths.iter().cloned().collect::<Vec<_>>(),
                }),
            );
            self.refresh_tree_debug("toggle_tree_row");
            return;
        }
        self.selected_tree_paths.clear();
        self.selected_tree_paths.insert(row.full_path.clone());
        self.selection_anchor = Some(row.full_path.clone());
        if !is_synthetic_sidebar_row(row) {
            self.browser.select_path(row.full_path.clone());
        }
        if cfg!(debug_assertions) {
            info!(path=%row.full_path, mode=?mode, "tree row selected");
        }
        self.record_ui_telemetry(
            "tree_select",
            json!({
                "path": row.full_path,
                "mode": format!("{mode:?}"),
                "selected_paths": self.selected_tree_paths.iter().cloned().collect::<Vec<_>>(),
            }),
        );
        self.refresh_tree_debug("select_tree_row");
    }

    fn extend_tree_selection(&mut self, row: &BrowserRow) {
        let rows = merged_sidebar_rows(
            self.browser.rows(),
            self.server.remote_machines(),
            self.server.ssh_targets(),
            &self.server.live_sessions(),
            &self.browser.expanded_path_set(),
        );
        let anchor = self
            .selection_anchor
            .clone()
            .or_else(|| self.browser.selected_path().map(ToOwned::to_owned))
            .unwrap_or_else(|| row.full_path.clone());
        let Some(anchor_ix) = rows
            .iter()
            .position(|candidate| candidate.full_path == anchor)
        else {
            self.select_tree_row(row, TreeSelectionMode::Replace);
            return;
        };
        let Some(target_ix) = rows
            .iter()
            .position(|candidate| candidate.full_path == row.full_path)
        else {
            self.select_tree_row(row, TreeSelectionMode::Replace);
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
        if !is_synthetic_sidebar_row(row) {
            self.browser.select_path(row.full_path.clone());
        }
        if cfg!(debug_assertions) {
            info!(anchor=%anchor, target=%row.full_path, selected=%self.selected_tree_paths.len(), "tree range selected");
        }
        self.record_ui_telemetry(
            "tree_select_range",
            json!({
                "anchor": anchor,
                "target": row.full_path,
                "selected_paths": self.selected_tree_paths.iter().cloned().collect::<Vec<_>>(),
            }),
        );
        self.refresh_tree_debug("extend_tree_selection");
    }

    fn selected_workspace_rows(&self) -> Vec<BrowserRow> {
        let rows = merged_sidebar_rows(
            self.browser.rows(),
            self.server.remote_machines(),
            self.server.ssh_targets(),
            &self.server.live_sessions(),
            &self.browser.expanded_path_set(),
        );
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

    fn selected_saved_ssh_target_machine_keys(&self) -> (Vec<String>, Vec<String>) {
        let rows = merged_sidebar_rows(
            self.browser.rows(),
            self.server.remote_machines(),
            self.server.ssh_targets(),
            &self.server.live_sessions(),
            &self.browser.expanded_path_set(),
        );
        let selected_rows = if self.selected_tree_paths.is_empty() {
            self.browser
                .selected_path()
                .and_then(|path| rows.iter().find(|row| row.full_path == path).cloned())
                .into_iter()
                .collect::<Vec<_>>()
        } else {
            rows.into_iter()
                .filter(|row| self.selected_tree_paths.contains(&row.full_path))
                .collect::<Vec<_>>()
        };

        let mut machine_keys = Vec::new();
        let mut labels = Vec::new();
        for row in selected_rows {
            if let Some(machine_key) = saved_ssh_target_machine_key(&row, self.server.ssh_targets())
            {
                machine_keys.push(machine_key);
                labels.push(row.label.clone());
            }
        }
        (machine_keys, labels)
    }

    fn begin_drag(&mut self, row: &BrowserRow, pointer: (f64, f64)) {
        if !is_workspace_row(row) {
            self.drag_paths.clear();
            self.drag_hover_target = None;
            self.drag_pointer = None;
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
        self.optimistic_drag_paths.clear();
        self.optimistic_drag_target = None;
        self.drag_pointer = Some(pointer);
        if cfg!(debug_assertions) {
            info!(drag_count=%self.drag_paths.len(), anchor=%row.full_path, "tree drag started");
        }
        self.record_ui_telemetry(
            "tree_drag_begin",
            json!({
                "anchor": row.full_path,
                "drag_paths": self.drag_paths,
            }),
        );
        self.refresh_tree_debug("begin_drag");
    }

    fn update_drag_pointer(&mut self, pointer: (f64, f64)) {
        if !self.drag_paths.is_empty() {
            self.drag_pointer = Some(pointer);
        }
    }

    fn set_drag_hover_target(
        &mut self,
        row: &BrowserRow,
        pointer: (f64, f64),
        placement: DragDropPlacement,
    ) {
        self.drag_pointer = Some(pointer);
        let rows = merged_sidebar_rows(
            self.browser.rows(),
            self.server.remote_machines(),
            self.server.ssh_targets(),
            &self.server.live_sessions(),
            &self.browser.expanded_path_set(),
        );
        self.drag_hover_target =
            resolve_drag_drop_target(&rows, self.drag_paths.as_slice(), row, placement);
        self.optimistic_drag_target = self.drag_hover_target.clone();
        if cfg!(debug_assertions)
            && let Some(target) = self.drag_hover_target.as_ref()
        {
            info!(target=%target.path, placement=?target.placement, drag_count=%self.drag_paths.len(), "tree drag hover");
        }
        self.record_ui_telemetry(
            "tree_drag_hover",
            json!({
                "target": self.drag_hover_target.as_ref().map(|target| target.path.clone()),
                "placement": self.drag_hover_target.as_ref().map(|target| format!("{:?}", target.placement).to_ascii_lowercase()),
                "drag_paths": self.drag_paths,
            }),
        );
        self.refresh_tree_debug("set_drag_hover_target");
    }

    fn clear_drag_state(&mut self) {
        let had_drag = !self.drag_paths.is_empty();
        self.drag_paths.clear();
        self.drag_hover_target = None;
        self.optimistic_drag_paths.clear();
        self.optimistic_drag_target = None;
        self.drag_pointer = None;
        if had_drag {
            self.suppress_tree_click_until_ms = current_millis().saturating_add(220);
        }
        self.refresh_tree_debug("clear_drag_state");
    }

    fn consume_suppressed_tree_click(&mut self) -> bool {
        if current_millis() < self.suppress_tree_click_until_ms {
            self.suppress_tree_click_until_ms = 0;
            true
        } else {
            false
        }
    }

    fn open_delete_dialog(&mut self, hard_delete: bool) {
        let (document_paths, group_paths, mut labels) = self.selected_workspace_delete_paths();
        let (ssh_machine_keys, ssh_labels) = self.selected_saved_ssh_target_machine_keys();
        labels.extend(ssh_labels);
        if document_paths.is_empty() && group_paths.is_empty() && ssh_machine_keys.is_empty() {
            return;
        }
        self.pending_delete = Some(PendingDeleteDialog {
            document_paths,
            group_paths,
            ssh_machine_keys,
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
                ssh_machines=%self.pending_delete.as_ref().map(|pending| pending.ssh_machine_keys.len()).unwrap_or(0),
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
        self.select_tree_row(row, TreeSelectionMode::Replace);
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
            "{source} | active={} | view={:?} | selected={} [{}] | drag={} [{}] | hover={} | pending_delete={} | rename={}",
            self.server.active_session_path().unwrap_or("none"),
            self.server.active_view_mode(),
            self.selected_tree_paths.len(),
            join_debug_paths(self.selected_tree_paths.iter().cloned().collect()),
            self.drag_paths.len(),
            join_debug_paths(self.drag_paths.clone()),
            self.drag_hover_target
                .as_ref()
                .map(|target| format!("{}:{:?}", target.path, target.placement))
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

    fn record_restore_issue_telemetry(&self, source: &str) {
        let mut effective_expanded = self.browser.expanded_path_set();
        effective_expanded.extend(self.active_session_visibility_paths());
        let live_sessions = self.server.live_sessions();
        let rows = merged_sidebar_rows(
            self.browser.rows(),
            self.server.remote_machines(),
            self.server.ssh_targets(),
            &live_sessions,
            &effective_expanded,
        );
        let active_path = self.server.active_session_path().map(ToOwned::to_owned);
        let active_row_visible = active_path
            .as_ref()
            .is_some_and(|path| rows.iter().any(|row| row.full_path == *path));
        let active_remote_paths = self.active_session_visibility_paths();
        let payload = json!({
            "source": source,
            "active_session_path": active_path,
            "active_view_mode": format!("{:?}", self.server.active_view_mode()),
            "selected_path": self.browser.selected_path(),
            "browser_expanded_paths": self.browser.expanded_paths(),
            "effective_expanded_paths": effective_expanded.iter().cloned().collect::<Vec<_>>(),
            "active_remote_paths": active_remote_paths,
            "active_row_visible": active_row_visible,
            "merged_row_count": rows.len(),
            "merged_row_paths_head": rows.iter().take(32).map(|row| row.full_path.clone()).collect::<Vec<_>>(),
            "remote_machine_count": self.server.remote_machines().len(),
        });
        self.record_ui_telemetry("restore_debug", payload.clone());
        info!(source, payload=%payload, "restore debug");
    }

    fn record_preview_issue_telemetry(&self, source: &str) {
        let payload = if let Some(session) = self.server.active_session() {
            json!({
                "source": source,
                "active_session_path": session.session_path,
                "active_view_mode": format!("{:?}", self.server.active_view_mode()),
                "session_source": format!("{:?}", session.source),
                "preview_blocks": session.preview.blocks.len(),
                "preview_placeholder": remote_preview_needs_refresh(session),
                "rendered_sections": session.rendered_sections.len(),
                "active_title_len": resolved_session_title(self, session).map(|v| v.len()),
                "active_precis_len": resolved_session_precis(self, session).map(|v| v.len()),
                "active_summary_len": resolved_session_summary(self, session).map(|v| v.len()),
                "preview_summary_len": preview_summary_text(session).len(),
                "terminal_precis_len": terminal_precis(session).len(),
                "last_tree_debug": self.last_tree_debug,
            })
        } else {
            json!({
                "source": source,
                "active_session_path": Value::Null,
                "active_view_mode": format!("{:?}", self.server.active_view_mode()),
                "last_tree_debug": self.last_tree_debug,
            })
        };
        self.record_ui_telemetry("preview_debug", payload.clone());
        info!(source, payload=%payload, "preview debug");
    }

    fn record_ui_telemetry(&self, event: &str, payload: Value) {
        let telemetry = json!({
            "ts": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
                .to_string(),
            "event": event,
            "payload": payload,
        });
        if let Ok(store) = SessionStore::open_or_init() {
            let path = store.home_dir().join("ui-telemetry.jsonl");
            if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
                let _ = writeln!(file, "{}", telemetry);
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
            let now = current_millis();
            if let Some(existing) = self.notifications.iter_mut().rev().find(|notification| {
                notification.tone == tone
                    && notification.title == title
                    && notification.message == message
            }) {
                existing.created_at_ms = now;
            } else {
                self.notifications.push(ToastNotification {
                    id: self.next_notification_id,
                    tone,
                    title: title.clone(),
                    message: message.clone(),
                    created_at_ms: now,
                    job_key: None,
                    progress: None,
                    persistent: false,
                });
                self.next_notification_id += 1;
            }
        }
        if self.settings.system_notifications {
            emit_system_notification(&title, &message);
        }
        if self.settings.notification_sound {
            emit_notification_chime();
        }
        if self.notifications.len() > 1000 {
            let overflow = self.notifications.len() - 1000;
            self.notifications.drain(0..overflow);
        }
    }

    fn upsert_job_notification(
        &mut self,
        job_key: impl Into<String>,
        tone: NotificationTone,
        title: impl Into<String>,
        message: impl Into<String>,
        progress: Option<f32>,
        emit_system: bool,
    ) {
        let job_key = job_key.into();
        let title = title.into();
        let message = message.into();
        let now = current_millis();
        let mut created = false;
        if self.settings.in_app_notifications {
            if let Some(existing) = self
                .notifications
                .iter_mut()
                .find(|notification| notification.job_key.as_deref() == Some(job_key.as_str()))
            {
                existing.tone = tone;
                existing.title = title.clone();
                existing.message = message.clone();
                existing.progress = progress.map(|value| value.clamp(0.0, 1.0));
                existing.persistent = true;
                existing.created_at_ms = now;
            } else {
                self.notifications.push(ToastNotification {
                    id: self.next_notification_id,
                    tone,
                    title: title.clone(),
                    message: message.clone(),
                    created_at_ms: now,
                    job_key: Some(job_key.clone()),
                    progress: progress.map(|value| value.clamp(0.0, 1.0)),
                    persistent: true,
                });
                self.next_notification_id += 1;
                created = true;
            }
        }
        if emit_system && created && self.settings.system_notifications {
            emit_system_notification(&title, &message);
        }
        if self.notifications.len() > 1000 {
            let overflow = self.notifications.len() - 1000;
            self.notifications.drain(0..overflow);
        }
    }

    fn finish_job_notification(
        &mut self,
        job_key: &str,
        tone: NotificationTone,
        title: impl Into<String>,
        message: impl Into<String>,
        emit_completion: bool,
    ) {
        self.clear_job_notification(job_key);
        if emit_completion {
            self.push_notification(tone, title, message);
        }
    }
}

fn safe_push_notification(
    state: Signal<ShellState>,
    tone: NotificationTone,
    title: impl Into<String>,
    message: impl Into<String>,
) {
    let title = title.into();
    let message = message.into();
    let title_for_write = title.clone();
    let message_for_write = message.clone();
    if let Err(error) = safe_shell_mut(state, "push_notification", move |shell| {
        shell.push_notification(tone, title_for_write, message_for_write);
    }) {
        warn!(
            title=%title,
            message=%message,
            panic_payload=?error,
            "suppressed notification panic"
        );
    }
}

fn safe_upsert_job_notification(
    state: Signal<ShellState>,
    job_key: impl Into<String>,
    tone: NotificationTone,
    title: impl Into<String>,
    message: impl Into<String>,
    progress: Option<f32>,
    emit_system: bool,
) {
    let job_key = job_key.into();
    let title = title.into();
    let message = message.into();
    let title_for_write = title.clone();
    let message_for_write = message.clone();
    let job_key_for_write = job_key.clone();
    if let Err(error) = safe_shell_mut(state, "upsert_job_notification", move |shell| {
        shell.upsert_job_notification(
            job_key_for_write,
            tone,
            title_for_write,
            message_for_write,
            progress,
            emit_system,
        );
    }) {
        warn!(
            job_key=%job_key,
            title=%title,
            message=%message,
            panic_payload=?error,
            "suppressed job notification panic"
        );
    }
}

fn safe_shell_mut<R>(
    mut state: Signal<ShellState>,
    context: &'static str,
    operation: impl FnOnce(&mut ShellState) -> R,
) -> std::thread::Result<R> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        state.with_mut(operation)
    }))
    .map_err(|error| {
        warn!(%context, panic_payload=?error, "suppressed shell state panic");
        error
    })
}

fn safe_shell_read<R>(
    state: Signal<ShellState>,
    context: &'static str,
    operation: impl FnOnce(&ShellState) -> R,
) -> Option<R> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let shell = state.read();
        operation(&shell)
    })) {
        Ok(result) => Some(result),
        Err(error) => {
            warn!(%context, panic_payload=?error, "suppressed shell read panic");
            None
        }
    }
}

fn queue_title_generation(state: Signal<ShellState>, row: BrowserRow, force: bool) {
    let target = safe_shell_read(state, "queue_title_generation_read", |shell| {
        copy_generation_target_for_browser_row(&shell.server, &row)
    })
    .flatten();
    if let Some(target) = target {
        spawn_title_generation_for_target(state, target, force, force, true);
    }
}

fn spawn_deferred_title_generation(state: Signal<ShellState>, row: BrowserRow, force: bool) {
    spawn(async move {
        queue_title_generation(state, row, force);
    });
}

fn queue_active_session_title_generation(state: Signal<ShellState>, force: bool) {
    let Some(target) = safe_shell_read(state, "queue_active_session_title_generation_read", |shell| {
        let session = shell.server.active_session().cloned()?;
        copy_generation_target_for_session(&shell.server, &session)
    })
    .flatten() else {
        return;
    };
    spawn_title_generation_for_target(state, target, force, force, true);
}

fn spawn_deferred_active_session_title_generation(state: Signal<ShellState>, force: bool) {
    spawn(async move {
        queue_active_session_title_generation(state, force);
    });
}

fn spawn_title_generation_for_target(
    mut state: Signal<ShellState>,
    target: CopyGenerationTarget,
    force: bool,
    announce: bool,
    priority: bool,
) {
    let session_path = target.session_path.clone();
    let session_title = target.title.clone();
    let settings = state.read().settings.clone();
    let should_start = state.with_mut(|shell| {
        if shell.title_requests_in_flight.contains(&session_path)
            || (!force
                && !priority
                && !background_copy_retry_ready(shell, "title", &session_path))
        {
            false
        } else {
            shell.title_requests_in_flight.insert(session_path.clone());
            true
        }
    });
    if !should_start {
        return;
    }
    let job_key = format!("copy:title:{session_path}");
    let job_title = if announce || priority {
        "Generating Title"
    } else {
        "Background Cache"
    };
    let job_message = if announce || priority {
        format!("Generating a better title for {session_title}.")
    } else {
        format!("Caching a title for {session_title}.")
    };
    safe_upsert_job_notification(
        state,
        job_key.clone(),
        NotificationTone::Info,
        job_title,
        job_message,
        None,
        announce,
    );
    if announce {
        state.with_mut(|shell| {
            shell.last_action = if force {
                format!("regenerating title for {}", target.title)
            } else {
                format!("generating title for {}", target.title)
            };
        });
    }
    info!(session_path=%target.session_path, force, "queueing title generation");
    let perf_home = perf_home_dir(&state.read().bootstrap.settings_path);
    spawn(async move {
        let perf = PerfSpan::start(&perf_home, "copy_generation", "title");
        let target_for_task = target.clone();
        let settings_for_task = settings.clone();
        let outcome = task::spawn_blocking(move || -> Result<(Option<String>, Option<SessionNode>)> {
            info!(session_path=%target_for_task.session_path, force, "running title generation task");
            let store = SessionStore::open_or_init()?;
            if let Some(context) = target_for_task.remote_context.as_deref() {
                let title = store.generate_title_for_context(
                    &settings_for_task,
                    &target_for_task.session_id,
                    &target_for_task.cwd,
                    context,
                    force,
                )?;
                if let (Some(machine), Some(title_text)) =
                    (target_for_task.remote_machine.as_ref(), title.as_deref())
                {
                    persist_remote_generated_copy(
                        machine,
                        &target_for_task.session_id,
                        &target_for_task.cwd,
                        Some(title_text),
                        None,
                        None,
                        &settings_for_task.interface_llm_model,
                    )?;
                }
                Ok((title, None))
            } else {
                let title = store.generate_title_for_session_path(
                    &settings_for_task,
                    &target_for_task.session_path,
                    force,
                )?;
                let browser_tree = store.load_codex_tree(&settings_for_task)?;
                Ok((title, Some(browser_tree)))
            }
        }).await;

        perf.finish(json!({
            "session_path": session_path.clone(),
            "force": force,
            "announce": announce,
            "ok": outcome.as_ref().is_ok_and(|result| result.is_ok()),
        }));
        let emit_completion = announce;
        state.with_mut(|shell| match outcome {
            Ok(Ok((Some(title), browser_tree))) => {
                shell.title_requests_in_flight.remove(&session_path);
                shell
                    .passive_copy_failures
                    .remove(&background_copy_retry_key("title", &session_path));
                shell
                    .copy_retry_after_ms
                    .remove(&background_copy_retry_key("title", &session_path));
                if let Some(browser_tree) = browser_tree {
                    restore_browser_tree(shell, browser_tree, Some(&target.session_path));
                }
                shell.server.set_session_title_hint(&target.session_path, &title);
                shell.last_action = if force {
                    "regenerated title".to_string()
                } else {
                    "generated title".to_string()
                };
                shell.finish_job_notification(
                    &job_key,
                    NotificationTone::Success,
                    "Title Regenerated",
                    format!("Session is now titled “{title}”."),
                    emit_completion,
                );
            }
            Ok(Ok((None, _))) => {
                shell.title_requests_in_flight.remove(&session_path);
                if !announce && !force {
                    PASSIVE_COPY_SUSPENDED.store(true, Ordering::Relaxed);
                    shell.passive_copy_suspended = true;
                    shell
                        .passive_copy_failures
                        .insert(background_copy_retry_key("title", &session_path));
                }
                shell.copy_retry_after_ms.insert(
                    background_copy_retry_key("title", &session_path),
                    current_millis() + BACKGROUND_COPY_RETRY_MS,
                );
                warn!(session_path=%target.session_path, "title generation produced no usable title");
                shell.finish_job_notification(
                    &job_key,
                    NotificationTone::Warning,
                    "No Title Generated",
                    "The model did not return a usable short title for this session.",
                    true,
                );
            }
            Ok(Err(error)) => {
                shell.title_requests_in_flight.remove(&session_path);
                if !announce && !force {
                    PASSIVE_COPY_SUSPENDED.store(true, Ordering::Relaxed);
                    shell.passive_copy_suspended = true;
                    shell
                        .passive_copy_failures
                        .insert(background_copy_retry_key("title", &session_path));
                }
                shell.copy_retry_after_ms.insert(
                    background_copy_retry_key("title", &session_path),
                    current_millis() + BACKGROUND_COPY_RETRY_MS,
                );
                shell.last_action = format!("title generation failed: {error}");
                warn!(session_path=%target.session_path, error=%error, "title generation failed");
                shell.finish_job_notification(
                    &job_key,
                    NotificationTone::Error,
                    "Title Generation Failed",
                    error.to_string(),
                    true,
                );
            }
            Err(error) => {
                shell.title_requests_in_flight.remove(&session_path);
                if !announce && !force {
                    PASSIVE_COPY_SUSPENDED.store(true, Ordering::Relaxed);
                    shell.passive_copy_suspended = true;
                    shell
                        .passive_copy_failures
                        .insert(background_copy_retry_key("title", &session_path));
                }
                shell.copy_retry_after_ms.insert(
                    background_copy_retry_key("title", &session_path),
                    current_millis() + BACKGROUND_COPY_RETRY_MS,
                );
                shell.last_action = format!("title generation task failed: {error}");
                warn!(session_path=%target.session_path, error=%error, "title generation task join failed");
                shell.finish_job_notification(
                    &job_key,
                    NotificationTone::Error,
                    "Title Task Failed",
                    error.to_string(),
                    true,
                );
            }
        });
        maybe_spawn_background_copy_generation(state);
    });
}

fn spawn_precis_generation(state: Signal<ShellState>, session: ManagedSessionView, force: bool) {
    let Some(target) = copy_generation_target_for_session(&state.read().server, &session) else {
        return;
    };
    spawn_precis_generation_for_target(state, target, force, force, true);
}

fn spawn_precis_generation_for_target(
    mut state: Signal<ShellState>,
    target: CopyGenerationTarget,
    force: bool,
    announce: bool,
    priority: bool,
) {
    let session_path = target.session_path.clone();
    let session_title = target.title.clone();
    let settings = state.read().settings.clone();
    let should_start = state.with_mut(|shell| {
        if (!force && shell.generated_precis.contains_key(&session_path))
            || shell.precis_requests_in_flight.contains(&session_path)
            || (!force
                && !priority
                && !background_copy_retry_ready(shell, "precis", &session_path))
        {
            false
        } else {
            if force {
                shell.generated_precis.remove(&session_path);
            }
            shell.precis_requests_in_flight.insert(session_path.clone());
            true
        }
    });
    if !should_start {
        return;
    }
    let job_key = format!("copy:precis:{session_path}");
    safe_upsert_job_notification(
        state,
        job_key.clone(),
        NotificationTone::Info,
        if announce || priority {
            "Generating Precis"
        } else {
            "Background Cache"
        },
        if announce || priority {
            format!("Generating a short precis for {session_title}.")
        } else {
            format!("Caching a precis for {session_title}.")
        },
        None,
        announce,
    );
    let perf_home = perf_home_dir(&state.read().bootstrap.settings_path);
    spawn(async move {
        let perf = PerfSpan::start(&perf_home, "copy_generation", "precis");
        let target_for_task = target.clone();
        let settings_for_task = settings.clone();
        let outcome = task::spawn_blocking(move || -> Result<Option<String>> {
            let store = SessionStore::open_or_init()?;
            if let Some(context) = target_for_task.remote_context.as_deref() {
                let precis = store.generate_precis_for_context(
                    &settings_for_task,
                    &target_for_task.session_id,
                    &target_for_task.cwd,
                    context,
                    force,
                )?;
                if let (Some(machine), Some(precis_text)) =
                    (target_for_task.remote_machine.as_ref(), precis.as_deref())
                {
                    persist_remote_generated_copy(
                        machine,
                        &target_for_task.session_id,
                        &target_for_task.cwd,
                        None,
                        Some(precis_text),
                        None,
                        &settings_for_task.interface_llm_model,
                    )?;
                }
                return Ok(precis);
            }
            if !force
                && let Some(precis) = store.resolve_precis_for_session_path(&target_for_task.session_path)?
            {
                return Ok(Some(precis));
            }
            store.generate_precis_for_session_path(&settings_for_task, &target_for_task.session_path, force)
        }).await;

        perf.finish(json!({
            "session_path": session_path.clone(),
            "force": force,
            "announce": announce,
            "ok": outcome.as_ref().is_ok_and(|result| result.is_ok()),
        }));
        state.with_mut(|shell| {
            shell.precis_requests_in_flight.remove(&session_path);
            match outcome {
                Ok(Ok(Some(precis))) => {
                    shell.generated_precis.insert(session_path.clone(), precis);
                    if let Some(precis) = shell.generated_precis.get(&session_path).cloned() {
                        shell.server.set_session_precis_hint(&session_path, &precis);
                    }
                    shell
                        .passive_copy_failures
                        .remove(&background_copy_retry_key("precis", &session_path));
                    shell
                        .copy_retry_after_ms
                        .remove(&background_copy_retry_key("precis", &session_path));
                    shell.finish_job_notification(
                        &job_key,
                        NotificationTone::Success,
                        "Precis Regenerated",
                        "Updated the terminal header precis from recent session context.",
                        announce,
                    );
                }
                Ok(Ok(None)) => {
                    if !announce && !force {
                        PASSIVE_COPY_SUSPENDED.store(true, Ordering::Relaxed);
                        shell.passive_copy_suspended = true;
                        shell
                            .passive_copy_failures
                            .insert(background_copy_retry_key("precis", &session_path));
                    }
                    shell.copy_retry_after_ms.insert(
                        background_copy_retry_key("precis", &session_path),
                        current_millis() + BACKGROUND_COPY_RETRY_MS,
                    );
                    shell.finish_job_notification(
                        &job_key,
                        NotificationTone::Warning,
                        "No Precis Generated",
                        "The model did not return a usable precis.",
                        true,
                    );
                }
                Ok(Err(error)) => {
                    if !announce && !force {
                        PASSIVE_COPY_SUSPENDED.store(true, Ordering::Relaxed);
                        shell.passive_copy_suspended = true;
                        shell
                            .passive_copy_failures
                            .insert(background_copy_retry_key("precis", &session_path));
                    }
                    shell.copy_retry_after_ms.insert(
                        background_copy_retry_key("precis", &session_path),
                        current_millis() + BACKGROUND_COPY_RETRY_MS,
                    );
                    shell.last_action = format!("precis generation failed: {error}");
                    shell.finish_job_notification(
                        &job_key,
                        NotificationTone::Error,
                        "Precis Generation Failed",
                        error.to_string(),
                        true,
                    );
                }
                Err(error) => {
                    if !announce && !force {
                        PASSIVE_COPY_SUSPENDED.store(true, Ordering::Relaxed);
                        shell.passive_copy_suspended = true;
                        shell
                            .passive_copy_failures
                            .insert(background_copy_retry_key("precis", &session_path));
                    }
                    shell.copy_retry_after_ms.insert(
                        background_copy_retry_key("precis", &session_path),
                        current_millis() + BACKGROUND_COPY_RETRY_MS,
                    );
                    shell.last_action = format!("precis task failed: {error}");
                    shell.finish_job_notification(
                        &job_key,
                        NotificationTone::Error,
                        "Precis Task Failed",
                        error.to_string(),
                        true,
                    );
                }
            }
        });
        maybe_spawn_background_copy_generation(state);
    });
}

fn spawn_summary_generation(state: Signal<ShellState>, session: ManagedSessionView, force: bool) {
    let Some(target) = copy_generation_target_for_session(&state.read().server, &session) else {
        return;
    };
    spawn_summary_generation_for_target(state, target, force, force, true);
}

fn spawn_summary_generation_for_target(
    mut state: Signal<ShellState>,
    target: CopyGenerationTarget,
    force: bool,
    announce: bool,
    priority: bool,
) {
    let session_path = target.session_path.clone();
    let session_title = target.title.clone();
    let settings = state.read().settings.clone();
    let should_start = state.with_mut(|shell| {
        if (!force && shell.generated_summaries.contains_key(&session_path))
            || shell.summary_requests_in_flight.contains(&session_path)
            || (!force
                && !priority
                && !background_copy_retry_ready(shell, "summary", &session_path))
        {
            false
        } else {
            if force {
                shell.generated_summaries.remove(&session_path);
            }
            shell.summary_requests_in_flight.insert(session_path.clone());
            true
        }
    });
    if !should_start {
        return;
    }
    let job_key = format!("copy:summary:{session_path}");
    safe_upsert_job_notification(
        state,
        job_key.clone(),
        NotificationTone::Info,
        if announce || priority {
            "Generating Summary"
        } else {
            "Background Cache"
        },
        if announce || priority {
            format!("Generating a summary for {session_title}.")
        } else {
            format!("Caching a summary for {session_title}.")
        },
        None,
        announce,
    );
    let perf_home = perf_home_dir(&state.read().bootstrap.settings_path);
    spawn(async move {
        let perf = PerfSpan::start(&perf_home, "copy_generation", "summary");
        let target_for_task = target.clone();
        let settings_for_task = settings.clone();
        let outcome = task::spawn_blocking(move || -> Result<Option<String>> {
            let store = SessionStore::open_or_init()?;
            if let Some(context) = target_for_task.remote_context.as_deref() {
                let summary = store.generate_summary_for_context(
                    &settings_for_task,
                    &target_for_task.session_id,
                    &target_for_task.cwd,
                    context,
                    force,
                )?;
                if let (Some(machine), Some(summary_text)) =
                    (target_for_task.remote_machine.as_ref(), summary.as_deref())
                {
                    persist_remote_generated_copy(
                        machine,
                        &target_for_task.session_id,
                        &target_for_task.cwd,
                        None,
                        None,
                        Some(summary_text),
                        &settings_for_task.interface_llm_model,
                    )?;
                }
                return Ok(summary);
            }
            if !force
                && let Some(summary) = store.resolve_summary_for_session_path(&target_for_task.session_path)?
            {
                return Ok(Some(summary));
            }
            store.generate_summary_for_session_path(&settings_for_task, &target_for_task.session_path, force)
        }).await;

        perf.finish(json!({
            "session_path": session_path.clone(),
            "force": force,
            "announce": announce,
            "ok": outcome.as_ref().is_ok_and(|result| result.is_ok()),
        }));
        state.with_mut(|shell| {
            shell.summary_requests_in_flight.remove(&session_path);
            match outcome {
                Ok(Ok(Some(summary))) => {
                    shell.generated_summaries.insert(session_path.clone(), summary);
                    if let Some(summary) = shell.generated_summaries.get(&session_path).cloned() {
                        shell.server.set_session_summary_hint(&session_path, &summary);
                    }
                    shell
                        .passive_copy_failures
                        .remove(&background_copy_retry_key("summary", &session_path));
                    shell
                        .copy_retry_after_ms
                        .remove(&background_copy_retry_key("summary", &session_path));
                    shell.finish_job_notification(
                        &job_key,
                        NotificationTone::Success,
                        "Summary Regenerated",
                        "Updated the preview summary from recent session context.",
                        announce,
                    );
                }
                Ok(Ok(None)) => {
                    if !announce && !force {
                        PASSIVE_COPY_SUSPENDED.store(true, Ordering::Relaxed);
                        shell.passive_copy_suspended = true;
                        shell
                            .passive_copy_failures
                            .insert(background_copy_retry_key("summary", &session_path));
                    }
                    shell.copy_retry_after_ms.insert(
                        background_copy_retry_key("summary", &session_path),
                        current_millis() + BACKGROUND_COPY_RETRY_MS,
                    );
                    shell.finish_job_notification(
                        &job_key,
                        NotificationTone::Warning,
                        "No Summary Generated",
                        "The model did not return a usable summary.",
                        true,
                    );
                }
                Ok(Err(error)) => {
                    if !announce && !force {
                        PASSIVE_COPY_SUSPENDED.store(true, Ordering::Relaxed);
                        shell.passive_copy_suspended = true;
                        shell
                            .passive_copy_failures
                            .insert(background_copy_retry_key("summary", &session_path));
                    }
                    shell.copy_retry_after_ms.insert(
                        background_copy_retry_key("summary", &session_path),
                        current_millis() + BACKGROUND_COPY_RETRY_MS,
                    );
                    shell.last_action = format!("summary generation failed: {error}");
                    shell.finish_job_notification(
                        &job_key,
                        NotificationTone::Error,
                        "Summary Generation Failed",
                        error.to_string(),
                        true,
                    );
                }
                Err(error) => {
                    if !announce && !force {
                        PASSIVE_COPY_SUSPENDED.store(true, Ordering::Relaxed);
                        shell.passive_copy_suspended = true;
                        shell
                            .passive_copy_failures
                            .insert(background_copy_retry_key("summary", &session_path));
                    }
                    shell.copy_retry_after_ms.insert(
                        background_copy_retry_key("summary", &session_path),
                        current_millis() + BACKGROUND_COPY_RETRY_MS,
                    );
                    shell.last_action = format!("summary task failed: {error}");
                    shell.finish_job_notification(
                        &job_key,
                        NotificationTone::Error,
                        "Summary Task Failed",
                        error.to_string(),
                        true,
                    );
                }
            }
        });
        maybe_spawn_background_copy_generation(state);
    });
}

fn spawn_initial_server_sync(state: Signal<ShellState>) {
    let endpoint = state.read().bootstrap.server_endpoint.clone();
    let perf_home = perf_home_dir(&state.read().bootstrap.settings_path);
    spawn(async move {
        let perf = PerfSpan::start(&perf_home, "startup", "initial_server_sync");
        let outcome = task::spawn_blocking(move || initial_server_sync(endpoint)).await;
        perf.finish(json!({
            "ok": outcome.as_ref().is_ok_and(|result| result.is_ok()),
        }));
        let _ = safe_shell_mut(state, "initial_server_sync_complete", |shell| match outcome {
            Ok(Ok((snapshot, runtime, detail))) => {
                let message = runtime
                    .map(|runtime| format!("server ready · {}", runtime.host_kind))
                    .unwrap_or_else(|| "server ready".to_string());
                shell.apply_daemon_snapshot_result(Ok((snapshot, Some(message))));
                shell.server_daemon_detail = detail;
                shell.next_background_copy_scan_after_ms = current_millis() + 2_500;
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
        maybe_spawn_missing_remote_machine_refreshes(state);
    });
}

fn spawn_initial_browser_tree_load(state: Signal<ShellState>) {
    let settings = state.read().settings.clone();
    let perf_home = perf_home_dir(&state.read().bootstrap.settings_path);
    spawn(async move {
        let perf = PerfSpan::start(&perf_home, "startup", "initial_browser_tree_load");
        let outcome = task::spawn_blocking(move || -> Result<SessionNode> {
            let store = SessionStore::open_or_init()?;
            store.load_codex_tree(&settings)
        })
        .await;
        perf.finish(json!({
            "ok": outcome.as_ref().is_ok_and(|result| result.is_ok()),
        }));
        let _ = safe_shell_mut(state, "initial_browser_tree_load_complete", |shell| {
            shell.browser_tree_loading_in_flight = false;
            match outcome {
                Ok(Ok(browser_tree)) => {
                    let selected_hint = shell.browser.selected_path().map(str::to_string);
                    restore_browser_tree(shell, browser_tree, selected_hint.as_deref());
                    shell.last_action = "loaded session tree".to_string();
                }
                Ok(Err(error)) => {
                    shell.last_action = format!("session tree load failed: {error}");
                }
                Err(error) => {
                    shell.last_action = format!("session tree task failed: {error}");
                }
            }
        });
    });
}

fn restart_into_pending_update(mut state: Signal<ShellState>) {
    let pending = state.read().pending_update_restart.clone();
    let Some(update) = pending else {
        return;
    };
    state.with_mut(|shell| {
        shell.last_action = format!("restarting into {}", update.version);
    });
    spawn(async move {
        let next_exe = update.executable.clone();
        let next_version = update.version.clone();
        let launched = task::spawn_blocking(move || Command::new(&next_exe).spawn()).await;
        state.with_mut(|shell| match launched {
            Ok(Ok(_)) => {
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
        if state.read().pending_update_restart.as_ref().map(|u| u.version.as_str())
            == Some(next_version.as_str())
        {
            state.with_mut(|shell| {
                shell.pending_update_restart = None;
            });
        }
    });
}

fn spawn_desktop_integration_refresh(mut state: Signal<ShellState>) {
    let install_context = state.read().bootstrap.install_context.clone();
    let perf_home = perf_home_dir(&state.read().bootstrap.settings_path);
    spawn(async move {
        let perf = PerfSpan::start(&perf_home, "startup", "refresh_desktop_integration");
        let outcome =
            task::spawn_blocking(move || refresh_desktop_integration(&install_context)).await;
        perf.finish(json!({
            "ok": outcome.as_ref().is_ok_and(|result| result.is_ok()),
        }));
        if let Ok(Err(error)) = outcome {
            warn!(error=%error, "desktop integration refresh failed");
            state.with_mut(|shell| {
                shell.last_action = format!("desktop integration refresh failed: {error}");
            });
        }
    });
}

fn spawn_auto_update_install_check(mut state: Signal<ShellState>) {
    let install_context = state.read().bootstrap.install_context.clone();
    let perf_home = perf_home_dir(&state.read().bootstrap.settings_path);
    spawn(async move {
        let perf = PerfSpan::start(&perf_home, "startup", "auto_update_install");
        let outcome = task::spawn_blocking(move || -> Result<Option<PendingUpdateRestart>> {
            let Some(update) = check_for_update(&install_context)? else {
                return Ok(None);
            };
            let next_exe = install_release_update(&install_context, &update)?;
            Ok(Some(PendingUpdateRestart {
                version: update.version,
                executable: next_exe,
            }))
        })
        .await;
        let pending = match outcome {
            Ok(Ok(pending)) => pending,
            Ok(Err(error)) => {
                warn!(error=%error, "auto update install failed");
                None
            }
            Err(error) => {
                warn!(error=%error, "auto update task failed");
                None
            }
        };
        perf.finish(json!({
            "installed": pending.as_ref().is_some(),
        }));
        if let Some(update) = pending {
            state.with_mut(|shell| {
                shell.pending_update_restart = Some(update.clone());
                shell.last_action = format!("update {} ready", update.version);
                shell.push_notification(
                    NotificationTone::Success,
                    "Update Ready",
                    format!(
                        "Yggterm {} was installed. Restart when you are ready.",
                        update.version
                    ),
                );
            });
        }
    });
}

fn spawn_notify_only_update_check(mut state: Signal<ShellState>) {
    let install_context = state.read().bootstrap.install_context.clone();
    let perf_home = perf_home_dir(&state.read().bootstrap.settings_path);
    spawn(async move {
        let perf = PerfSpan::start(&perf_home, "startup", "notify_only_update_check");
        let outcome = task::spawn_blocking(move || check_for_update(&install_context)).await;
        perf.finish(json!({
            "ok": outcome.as_ref().is_ok_and(|result| result.is_ok()),
        }));
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
    ensure_daemon_running(&endpoint)?;

    let mut runtime = status(&endpoint).ok();
    if runtime
        .as_ref()
        .is_some_and(|runtime| {
            runtime.server_version != env!("CARGO_PKG_VERSION")
                || runtime.server_build_id != current_build_id()
        })
    {
        restart_daemon(&endpoint)?;
        runtime = status(&endpoint).ok();
    }

    let (mut snapshot, _) = match daemon_snapshot(&endpoint) {
        Ok(snapshot) => snapshot,
        Err(_) => {
            restart_daemon(&endpoint)?;
            daemon_snapshot(&endpoint)?
        }
    };
    if snapshot_needs_remote_restore_restart(&snapshot) {
        restart_daemon(&endpoint)?;
        snapshot = daemon_snapshot(&endpoint)?.0;
        runtime = status(&endpoint).ok();
    }
    let detail = match &endpoint {
        #[cfg(unix)]
        ServerEndpoint::UnixSocket(path) => format!("server connected via {}", path.display()),
        ServerEndpoint::Tcp { host, port } => format!("server connected via {host}:{port}"),
    };
    Ok((snapshot, runtime, detail))
}

fn current_build_id() -> u64 {
    std::env::current_exe()
        .ok()
        .and_then(|path| std::fs::metadata(path).ok())
        .and_then(|meta| meta.modified().ok())
        .and_then(|ts| ts.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|dur| dur.as_secs())
        .unwrap_or_default()
}

fn snapshot_needs_remote_restore_restart(snapshot: &ServerUiSnapshot) -> bool {
    snapshot.remote_machines.is_empty()
        && snapshot.ssh_targets.is_empty()
        && snapshot
            .live_sessions
            .iter()
            .any(|session| session.kind == SessionKind::SshShell)
}

fn ensure_daemon_running(endpoint: &ServerEndpoint) -> Result<()> {
    let current_exe = std::env::current_exe()?;
    cleanup_legacy_daemons(endpoint, &current_exe)?;
    if ping(endpoint).is_ok() {
        return Ok(());
    }
    spawn_daemon_process()?;
    wait_for_daemon(endpoint)
}

fn restart_daemon(endpoint: &ServerEndpoint) -> Result<()> {
    let current_exe = std::env::current_exe()?;
    cleanup_legacy_daemons(endpoint, &current_exe)?;
    let _ = daemon_shutdown(endpoint);
    thread::sleep(Duration::from_millis(200));
    spawn_daemon_process()?;
    wait_for_daemon(endpoint)
}

fn spawn_daemon_process() -> Result<()> {
    let current_exe = std::env::current_exe()?;
    Command::new(current_exe)
        .arg("server")
        .arg("daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

fn wait_for_daemon(endpoint: &ServerEndpoint) -> Result<()> {
    for _ in 0..20 {
        thread::sleep(Duration::from_millis(150));
        if ping(endpoint).is_ok() {
            return Ok(());
        }
    }
    anyhow::bail!("daemon did not become reachable")
}

fn maybe_spawn_missing_remote_machine_refreshes(state: Signal<ShellState>) {
    let Some(pending) = safe_shell_read(
        state,
        "maybe_spawn_missing_remote_machine_refreshes_read",
        |shell| {
            let known_remote_machines = shell
                .server
                .remote_machines()
                .iter()
                .map(|machine| {
                    (
                        machine.machine_key.clone(),
                        machine.health == RemoteMachineHealth::Healthy
                            || machine.health == RemoteMachineHealth::Offline,
                    )
                })
                .collect::<HashMap<_, _>>();

            let mut desired_machine_keys = shell
                .server
                .ssh_targets()
                .iter()
                .map(|target| machine_key_from_labelish(&target.ssh_target))
                .collect::<Vec<_>>();
            desired_machine_keys.extend(
                shell.server
                    .live_sessions()
                    .into_iter()
                    .filter(|session| session.kind == SessionKind::SshShell)
                    .filter_map(|session| {
                        session
                            .ssh_target
                            .as_deref()
                            .map(machine_key_from_labelish)
                            .or_else(|| {
                                if session.host_label.trim().is_empty() {
                                    None
                                } else {
                                    Some(machine_key_from_labelish(&session.host_label))
                                }
                            })
                    }),
            );
            desired_machine_keys.sort();
            desired_machine_keys.dedup();
            desired_machine_keys
                .into_iter()
                .filter(|machine_key| !machine_key.is_empty())
                .filter(|machine_key| !shell.remote_machine_refresh_requests.contains(machine_key))
                .filter(|machine_key| {
                    !known_remote_machines
                        .get(machine_key)
                        .copied()
                        .unwrap_or(false)
                })
                .collect::<Vec<_>>()
        },
    ) else {
        return;
    };
    for machine_key in pending {
        let _ = safe_shell_mut(state, "queue_remote_machine_refresh", |shell| {
            shell.remote_machine_refresh_requests.insert(machine_key.clone());
        });
        spawn_background_remote_machine_refresh(state, machine_key);
    }
}

fn spawn_background_remote_machine_refresh(state: Signal<ShellState>, machine_key: String) {
    let endpoint = state.read().bootstrap.server_endpoint.clone();
    spawn(async move {
        let request_machine_key = machine_key.clone();
        let outcome = task::spawn_blocking(move || refresh_remote_machine(&endpoint, &request_machine_key)).await;
        let _ = safe_shell_mut(state, "remote_machine_refresh_complete", |shell| {
            shell.remote_machine_refresh_requests.remove(&machine_key);
            match outcome {
                Ok(Ok((snapshot, message))) => {
                    shell.server.apply_snapshot(snapshot);
                    if let Some(message) = message {
                        shell.last_action = message;
                    }
                }
                Ok(Err(error)) => {
                    shell.last_action = format!("remote refresh failed: {error}");
                }
                Err(error) => {
                    shell.last_action = format!("remote refresh task failed: {error}");
                }
            }
        });
        maybe_spawn_missing_remote_machine_refreshes(state);
    });
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
        let _ = safe_shell_mut(state, "server_snapshot_action_complete", |shell| match outcome {
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
        maybe_spawn_missing_remote_machine_refreshes(state);
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
    let prefer_terminal = state.read().server.active_view_mode() == WorkspaceViewMode::Terminal;
    state.with_mut(|shell| {
        shell.browser.select_path(row.full_path.clone());
        shell.context_menu_row = None;
        shell.sync_browser_settings();
        shell.server_busy = true;
        shell.last_action = format!("opening {}", row.label);
    });
    spawn_server_snapshot_action(state, format!("opening {}", row.label), move |endpoint| {
        let opened = if let Some((machine_key, session_id)) = parse_remote_scanned_session_path(&row.full_path) {
            open_remote_session(
                &endpoint,
                machine_key,
                session_id,
                row.session_cwd.as_deref(),
                Some(row.label.as_str()),
            )
        } else {
            open_stored_session(
                &endpoint,
                session_kind_for_row(row.kind),
                &row.full_path,
                row.session_id.as_deref(),
                row.session_cwd.as_deref(),
                Some(row.label.as_str()),
            )
        }?;
        if prefer_terminal {
            request_terminal_launch(&endpoint)
        } else if row.full_path.starts_with("remote-session://") {
            daemon_set_view_mode(&endpoint, WorkspaceViewMode::Rendered)
        } else {
            Ok(opened)
        }
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
                "Enter an SSH target such as dev, pi@raspberry, or user@ip.",
            );
        });
        return;
    }
    state.with_mut(|shell| {
        shell.server_busy = true;
        shell.last_action = format!("connecting {target}");
        shell.push_notification(
            NotificationTone::Info,
            "Connecting SSH",
            format!("Opening {target} and syncing its session index…"),
        );
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

fn is_remote_scanned_sidebar_row(row: &BrowserRow) -> bool {
    row.full_path.starts_with("remote-session://")
}

fn is_remote_machine_group_row(row: &BrowserRow) -> bool {
    row.kind == BrowserRowKind::Group && row.full_path.starts_with("__remote_machine__/")
}

fn is_synthetic_sidebar_row(row: &BrowserRow) -> bool {
    row.full_path.starts_with("__remote_machine__/")
        || row.full_path.starts_with("__remote_folder__/")
        || row.full_path == "__live_sessions__"
}

fn saved_ssh_target_machine_key(row: &BrowserRow, ssh_targets: &[SshConnectTarget]) -> Option<String> {
    let machine_key = row.full_path.strip_prefix("__remote_machine__/")?;
    ssh_targets
        .iter()
        .any(|target| machine_key_from_labelish(&target.ssh_target) == machine_key)
        .then(|| machine_key.to_string())
}

fn is_local_stored_session_row(row: &BrowserRow) -> bool {
    row.kind == BrowserRowKind::Session && !is_live_sidebar_row(row) && !is_remote_scanned_sidebar_row(row)
}

fn supports_generated_session_copy(session: &ManagedSessionView) -> bool {
    session.kind.is_agent() || session.session_path.starts_with("remote-session://")
}

fn copy_generation_target_for_session(
    server: &YggtermServer,
    session: &ManagedSessionView,
) -> Option<CopyGenerationTarget> {
    if !supports_generated_session_copy(session) {
        return None;
    }
    let preview_context = preview_context_from_session(session);
    Some(CopyGenerationTarget {
        session_path: session.session_path.clone(),
        session_id: session.id.clone(),
        cwd: metadata_value(session, "Cwd"),
        title: session.title.clone(),
        remote_context: remote_scanned_session_context(server, &session.session_path)
            .or(preview_context),
        remote_machine: remote_machine_for_session_path(server, &session.session_path),
    })
}

fn copy_generation_target_for_browser_row(
    server: &YggtermServer,
    row: &BrowserRow,
) -> Option<CopyGenerationTarget> {
    let session_id = row.session_id.clone()?;
    if is_remote_scanned_sidebar_row(row) {
        let machine = remote_machine_for_session_path(server, &row.full_path)?;
        let remote = machine
            .sessions
            .iter()
            .find(|session| session.session_path == row.full_path)?;
        return Some(CopyGenerationTarget {
            session_path: row.full_path.clone(),
            session_id,
            cwd: row
                .session_cwd
                .clone()
                .unwrap_or_else(|| remote.cwd.clone()),
            title: row.label.clone(),
            remote_context: (!remote.recent_context.trim().is_empty())
                .then(|| remote.recent_context.clone()),
            remote_machine: Some(machine),
        });
    }
    if !is_local_stored_session_row(row) {
        return None;
    }
    Some(CopyGenerationTarget {
        session_path: row.full_path.clone(),
        session_id,
        cwd: row.session_cwd.clone().unwrap_or_default(),
        title: row.label.clone(),
        remote_context: None,
        remote_machine: None,
    })
}

fn restore_browser_tree(shell: &mut ShellState, browser_tree: SessionNode, selected_hint: Option<&str>) {
    let selected_path = shell.browser.selected_path().map(str::to_string);
    let expanded_paths = shell.browser.expanded_paths();
    let synthetic_expanded_paths = expanded_paths
        .iter()
        .filter(|path| path.starts_with("__remote_machine__/") || path.starts_with("__remote_folder__/"))
        .cloned()
        .collect::<Vec<_>>();
    let filter_query = shell.search_query.clone();
    shell.browser = SessionBrowserState::new(browser_tree);
    shell.browser.restore_ui_state(
        &expanded_paths,
        selected_path.as_deref().or(selected_hint),
    );
    shell.browser.ensure_expanded_paths(synthetic_expanded_paths);
    shell.browser.set_filter_query(filter_query);
    shell.ensure_active_session_visible();
    shell.record_restore_issue_telemetry("restore_browser_tree");
    shell.sync_browser_settings();
}

fn remote_preview_needs_refresh(session: &ManagedSessionView) -> bool {
    session.session_path.starts_with("remote-session://")
        && (session.preview.blocks.is_empty()
            || session
                .preview
                .blocks
                .iter()
                .any(|block| block.timestamp == "server:launch"))
}

fn background_copy_job_for_target(
    store: &SessionStore,
    target: &CopyGenerationTarget,
    copy_retry_after_ms: &HashMap<String, u64>,
    now: u64,
) -> Option<BackgroundCopyJob> {
    let stored_title = store.resolve_title_for_session_id(&target.session_id).ok().flatten();
    let title_missing = looks_like_generated_fallback_title(&target.title)
        && stored_title
            .as_deref()
            .is_none_or(looks_like_generated_fallback_title);
    if title_missing {
        let retry_key = background_copy_retry_key("title", &target.session_path);
        if copy_retry_after_ms
            .get(&retry_key)
            .copied()
            .is_none_or(|retry_after| retry_after <= now)
        {
            return Some(BackgroundCopyJob::Title(target.clone()));
        }
    }
    let stored_precis = store
        .resolve_precis_for_session_id(&target.session_id)
        .ok()
        .flatten();
    let precis_missing = stored_precis.is_none()
        && target
            .remote_machine
            .as_ref()
            .and_then(|machine| {
                machine
                    .sessions
                    .iter()
                    .find(|session| session.session_path == target.session_path)
                    .and_then(|session| session.cached_precis.clone())
            })
            .is_none();
    if precis_missing {
        let retry_key = background_copy_retry_key("precis", &target.session_path);
        if copy_retry_after_ms
            .get(&retry_key)
            .copied()
            .is_none_or(|retry_after| retry_after <= now)
        {
            return Some(BackgroundCopyJob::Precis(target.clone()));
        }
    }
    let stored_summary = store
        .resolve_summary_for_session_id(&target.session_id)
        .ok()
        .flatten();
    let summary_missing = stored_summary.is_none()
        && target
            .remote_machine
            .as_ref()
            .and_then(|machine| {
                machine
                    .sessions
                    .iter()
                    .find(|session| session.session_path == target.session_path)
                    .and_then(|session| session.cached_summary.clone())
            })
            .is_none();
    if summary_missing {
        let retry_key = background_copy_retry_key("summary", &target.session_path);
        if copy_retry_after_ms
            .get(&retry_key)
            .copied()
            .is_none_or(|retry_after| retry_after <= now)
        {
            return Some(BackgroundCopyJob::Summary(target.clone()));
        }
    }
    None
}

fn next_background_copy_job(
    _settings: &AppSettings,
    _local_root: &SessionNode,
    remote_machines: &[RemoteMachineSnapshot],
    active_target: Option<CopyGenerationTarget>,
    copy_retry_after_ms: &HashMap<String, u64>,
) -> Option<BackgroundCopyJob> {
    let store = SessionStore::open_or_init().ok()?;
    let mut targets = Vec::new();
    for machine in remote_machines {
        for session in &machine.sessions {
            targets.push(CopyGenerationTarget {
                session_path: session.session_path.clone(),
                session_id: session.session_id.clone(),
                cwd: session.cwd.clone(),
                title: session.title_hint.clone(),
                remote_context: (!session.recent_context.trim().is_empty())
                    .then(|| session.recent_context.clone()),
                remote_machine: Some(machine.clone()),
            });
        }
    }
    let now = current_millis();
    if let Some(active_target) = active_target
        && let Some(job) = background_copy_job_for_target(
            &store,
            &active_target,
            copy_retry_after_ms,
            now,
        )
    {
        return Some(job);
    }
    for target in targets {
        if let Some(job) = background_copy_job_for_target(
            &store,
            &target,
            copy_retry_after_ms,
            now,
        ) {
            return Some(job);
        }
    }
    None
}

fn maybe_spawn_background_copy_generation(mut state: Signal<ShellState>) {
    let scan = {
        let shell = state.read();
        let now = current_millis();
        if shell.background_copy_scan_in_flight
            || shell.needs_initial_server_sync
            || PASSIVE_COPY_SUSPENDED.load(Ordering::Relaxed)
            || shell.passive_copy_suspended
            || shell.server.active_view_mode() == WorkspaceViewMode::Terminal
            || !shell.title_requests_in_flight.is_empty()
            || !shell.precis_requests_in_flight.is_empty()
            || !shell.summary_requests_in_flight.is_empty()
            || shell.next_background_copy_scan_after_ms > now
        {
            None
        } else {
            Some((
                shell.settings.clone(),
                shell.browser.root().clone(),
                shell.server.remote_machines().to_vec(),
                shell
                    .server
                    .active_session()
                    .and_then(|session| copy_generation_target_for_session(&shell.server, session)),
                shell.copy_retry_after_ms.clone(),
                perf_home_dir(&shell.bootstrap.settings_path),
            ))
        }
    };
    let Some((settings, local_root, remote_machines, active_target, copy_retry_after_ms, perf_home)) = scan else {
        return;
    };
    state.with_mut(|shell| shell.background_copy_scan_in_flight = true);
    safe_upsert_job_notification(
        state,
        "background:copy_scan",
        NotificationTone::Info,
        "Background Cache",
        "Scanning sessions for missing titles, precis, and summaries.",
        None,
        false,
    );
    spawn(async move {
        let perf = PerfSpan::start(&perf_home, "background", "copy_scan");
        let outcome = task::spawn_blocking(move || {
            next_background_copy_job(
                &settings,
                &local_root,
                &remote_machines,
                active_target,
                &copy_retry_after_ms,
            )
        })
        .await;
        let job = outcome.ok().flatten();
        perf.finish(json!({
            "job": job.as_ref().map(|job| match job {
                BackgroundCopyJob::Title(target) => format!("title:{}", target.session_path),
                BackgroundCopyJob::Precis(target) => format!("precis:{}", target.session_path),
                BackgroundCopyJob::Summary(target) => format!("summary:{}", target.session_path),
            }),
        }));
        state.with_mut(|shell| {
            shell.background_copy_scan_in_flight = false;
            shell.clear_job_notification("background:copy_scan");
            shell.next_background_copy_scan_after_ms = if job.is_some() {
                current_millis() + BACKGROUND_COPY_CONTINUE_MS
            } else {
                current_millis() + BACKGROUND_COPY_IDLE_MS
            };
        });
        match job {
            Some(BackgroundCopyJob::Title(target)) => {
                spawn_title_generation_for_target(state, target, false, false, false)
            }
            Some(BackgroundCopyJob::Precis(target)) => {
                spawn_precis_generation_for_target(state, target, false, false, false)
            }
            Some(BackgroundCopyJob::Summary(target)) => {
                spawn_summary_generation_for_target(state, target, false, false, false)
            }
            None => {}
        }
    });
}

fn remote_scanned_session_context(server: &YggtermServer, session_path: &str) -> Option<String> {
    server.remote_machines().iter().find_map(|machine| {
        machine
            .sessions
            .iter()
            .find(|session| session.session_path == session_path)
            .and_then(|session| {
                (!session.recent_context.trim().is_empty()).then(|| session.recent_context.clone())
            })
    })
}

fn remote_generated_copy(
    server: &YggtermServer,
    session_path: &str,
) -> Option<(String, Option<String>, Option<String>)> {
    server.remote_machines().iter().find_map(|machine| {
        machine.sessions.iter().find_map(|session| {
            (session.session_path == session_path).then(|| {
                (
                    session.title_hint.clone(),
                    session.cached_precis.clone(),
                    session.cached_summary.clone(),
                )
            })
        })
    })
}

fn resolved_session_title(shell: &ShellState, session: &ManagedSessionView) -> Option<String> {
    if !session.title.trim().is_empty() && !looks_like_generated_fallback_title(&session.title) {
        return Some(session.title.clone());
    }
    let remote_title = remote_generated_copy(&shell.server, &session.session_path)
        .map(|(title, _, _)| title)
        .filter(|title| !title.trim().is_empty() && !looks_like_generated_fallback_title(title));
    if remote_title.is_some() {
        return remote_title;
    }
    if let Some(row_title) = shell
        .browser
        .rows()
        .iter()
        .find(|row| row.full_path == session.session_path)
        .map(|row| row.label.clone())
        .filter(|title| !title.trim().is_empty() && !looks_like_generated_fallback_title(title))
    {
        return Some(row_title);
    }
    None
}

fn resolved_session_precis(shell: &ShellState, session: &ManagedSessionView) -> Option<String> {
    shell
        .generated_precis
        .get(&session.session_path)
        .cloned()
        .or_else(|| remote_generated_copy(&shell.server, &session.session_path).and_then(|(_, precis, _)| precis))
}

fn resolved_session_summary(shell: &ShellState, session: &ManagedSessionView) -> Option<String> {
    shell
        .generated_summaries
        .get(&session.session_path)
        .cloned()
        .or_else(|| remote_generated_copy(&shell.server, &session.session_path).and_then(|(_, _, summary)| summary))
}

fn remote_machine_for_session_path(
    server: &YggtermServer,
    session_path: &str,
) -> Option<RemoteMachineSnapshot> {
    server.remote_machines().iter().find_map(|machine| {
        machine
            .sessions
            .iter()
            .any(|session| session.session_path == session_path)
            .then(|| machine.clone())
    })
}

fn spawn_active_session_copy_hydration(mut state: Signal<ShellState>, session: ManagedSessionView) {
    let session_path = session.session_path.clone();
    let remote_cached = remote_generated_copy(&state.read().server, &session_path);
    let needs_local_lookup = is_local_stored_session_row(&BrowserRow {
        kind: BrowserRowKind::Session,
        full_path: session_path.clone(),
        label: session.title.clone(),
        detail_label: String::new(),
        document_kind: None,
        group_kind: None,
        session_title: Some(session.title.clone()),
        depth: 0,
        host_label: session.host_label.clone(),
        descendant_sessions: 0,
        expanded: false,
        session_id: Some(session.id.clone()),
        session_cwd: Some(metadata_value(&session, "Cwd")),
    });
    spawn(async move {
        let path_for_task = session_path.clone();
        let outcome = task::spawn_blocking(move || -> Result<(Option<String>, Option<String>, Option<String>)> {
            let store = SessionStore::open_or_init()?;
            if let Some((title, precis, summary)) = remote_cached {
                let resolved_title = if title.trim().is_empty() {
                    store.resolve_title_for_session_id(&session.id)?
                } else {
                    Some(title)
                };
                let resolved_precis = if precis.as_ref().is_some_and(|value| !value.trim().is_empty()) {
                    precis
                } else {
                    store.resolve_precis_for_session_id(&session.id)?
                };
                let resolved_summary = if summary.as_ref().is_some_and(|value| !value.trim().is_empty()) {
                    summary
                } else {
                    store.resolve_summary_for_session_id(&session.id)?
                };
                return Ok((resolved_title, resolved_precis, resolved_summary));
            }
            if !needs_local_lookup {
                return Ok((
                    store.resolve_title_for_session_id(&session.id)?,
                    store.resolve_precis_for_session_id(&session.id)?,
                    store.resolve_summary_for_session_id(&session.id)?,
                ));
            }
            Ok((
                store.resolve_title_for_session_path(&path_for_task)?,
                store.resolve_precis_for_session_path(&path_for_task)?,
                store.resolve_summary_for_session_path(&path_for_task)?,
            ))
        }).await;

        state.with_mut(|shell| {
            let Ok(Ok((title, precis, summary))) = outcome else {
                return;
            };
            if let Some(title) = title
            {
                shell.server.set_session_title_hint(&session_path, &title);
            }
            if let Some(precis) = precis {
                shell.generated_precis.insert(session_path.clone(), precis);
                if let Some(precis) = shell.generated_precis.get(&session_path).cloned() {
                    shell.server.set_session_precis_hint(&session_path, &precis);
                }
            }
            if let Some(summary) = summary {
                shell.generated_summaries.insert(session_path.clone(), summary);
                if let Some(summary) = shell.generated_summaries.get(&session_path).cloned() {
                    shell.server.set_session_summary_hint(&session_path, &summary);
                }
            }
        });
    });
}

fn preview_context_from_session(session: &ManagedSessionView) -> Option<String> {
    let lines = session
        .preview
        .blocks
        .iter()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .flat_map(|block| {
            let role = block.role.trim();
            block.lines.iter().filter_map(move |line| {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    None
                } else if role.is_empty() {
                    Some(trimmed.to_string())
                } else {
                    Some(format!("{role}: {trimmed}"))
                }
            })
        })
        .collect::<Vec<_>>();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn parse_remote_scanned_session_path(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix("remote-session://")?;
    let (machine_key, session_id) = rest.split_once('/')?;
    Some((machine_key, session_id))
}

fn stage_local_clipboard_png(home: &PathBuf, png_bytes: &[u8]) -> Result<String> {
    let clipboard_dir = home.join("clipboard");
    fs::create_dir_all(&clipboard_dir)?;
    let filename = format!("clipboard-{}.png", current_millis());
    let path = clipboard_dir.join(filename);
    fs::write(&path, png_bytes)?;
    Ok(path.to_string_lossy().to_string())
}

fn read_native_clipboard_png() -> Result<Vec<u8>> {
    #[cfg(target_os = "linux")]
    {
        let output = Command::new("xclip")
            .args(["-selection", "clipboard", "-t", "image/png", "-o"])
            .output()
            .map_err(|error| anyhow!("failed to launch xclip for clipboard image paste: {error}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(anyhow!(
                "clipboard unavailable: {}",
                if stderr.is_empty() {
                    "xclip could not read image/png from the clipboard".to_string()
                } else {
                    stderr
                }
            ));
        }
        let png_bytes = output.stdout;
        if png_bytes.len() < 8 || &png_bytes[..8] != b"\x89PNG\r\n\x1a\n" {
            return Err(anyhow!(
                "clipboard does not currently contain a PNG image"
            ));
        }
        return Ok(png_bytes);
    }

    #[allow(unreachable_code)]
    Err(anyhow!(
        "native clipboard image paste is not implemented on this platform yet"
    ))
}

fn stage_terminal_clipboard_image(
    state: Signal<ShellState>,
    session_path: &str,
    png_bytes: &[u8],
) -> Result<String> {
    let bootstrap = BOOTSTRAP.get().expect("shell bootstrap initialized");
    if let Some(machine) = remote_machine_for_session_path(&state.read().server, session_path) {
        return stage_remote_clipboard_png(
            &machine.ssh_target,
            machine.prefix.as_deref(),
            png_bytes,
        );
    }
    if let Some(session) = state.read().server.active_session().cloned()
        && session.session_path == session_path
        && let Some(ssh_target) = session.ssh_target
    {
        return stage_remote_clipboard_png(&ssh_target, session.ssh_prefix.as_deref(), png_bytes);
    }
    let home = bootstrap
        .settings_path
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    stage_local_clipboard_png(&home, png_bytes)
}

fn merged_sidebar_rows(
    stored_rows: &[BrowserRow],
    remote_machines: &[RemoteMachineSnapshot],
    ssh_targets: &[SshConnectTarget],
    live_sessions: &[ManagedSessionView],
    expanded_paths: &HashSet<String>,
) -> Vec<BrowserRow> {
    if ssh_targets.is_empty() && remote_machines.is_empty() {
        return stored_rows.to_vec();
    }

    let mut rows = Vec::with_capacity(stored_rows.len() + remote_machines.len() * 6 + 8);

    let mut machine_rows = BTreeMap::<String, SidebarRemoteMachine>::new();
    for target in ssh_targets {
        let machine_key = machine_key_from_labelish(&target.ssh_target);
        machine_rows
            .entry(machine_key.clone())
            .or_insert_with(|| SidebarRemoteMachine {
                key: machine_key.clone(),
                label: ssh_target_machine_label(target),
                health: MachineHealth::Cached,
                scanned_sessions: Vec::new(),
            });
    }
    for machine in remote_machines {
        machine_rows.insert(
            machine.machine_key.clone(),
            SidebarRemoteMachine {
                key: machine.machine_key.clone(),
                label: machine.label.clone(),
                health: match machine.health {
                    RemoteMachineHealth::Healthy => MachineHealth::Healthy,
                    RemoteMachineHealth::Cached => MachineHealth::Cached,
                    RemoteMachineHealth::Offline => MachineHealth::Offline,
                },
                scanned_sessions: machine.sessions.clone(),
            },
        );
    }
    merge_remote_live_sessions(&mut machine_rows, ssh_targets, live_sessions);
    for machine in machine_rows.into_values() {
        push_remote_machine_rows(&mut rows, &machine, expanded_paths);
    }
    rows.extend_from_slice(stored_rows);
    rows
}

fn merge_remote_live_sessions(
    machine_rows: &mut BTreeMap<String, SidebarRemoteMachine>,
    ssh_targets: &[SshConnectTarget],
    live_sessions: &[ManagedSessionView],
) {
    for session in live_sessions {
        let Some((machine_key, session_id)) = parse_remote_scanned_session_path(&session.session_path)
        else {
            continue;
        };
        let machine = machine_rows
            .entry(machine_key.to_string())
            .or_insert_with(|| SidebarRemoteMachine {
                key: machine_key.to_string(),
                label: ssh_targets
                    .iter()
                    .find(|target| machine_key_from_labelish(&target.ssh_target) == machine_key)
                    .map(ssh_target_machine_label)
                    .unwrap_or_else(|| format!("{machine_key} [ok]")),
                health: MachineHealth::Healthy,
                scanned_sessions: Vec::new(),
            });
        if machine
            .scanned_sessions
            .iter()
            .any(|existing| existing.session_path == session.session_path)
        {
            continue;
        }
        machine
            .scanned_sessions
            .push(remote_scanned_session_from_live(machine_key, session_id, session));
    }
}

fn remote_scanned_session_from_live(
    _machine_key: &str,
    session_id: &str,
    session: &ManagedSessionView,
) -> RemoteScannedSession {
    let cwd = metadata_value(session, "Cwd");
    let storage_path = metadata_value(session, "Storage");
    let started_at = metadata_value(session, "Started");
    let recent_context = preview_context_from_session(session).unwrap_or_default();
    let user_message_count = session
        .preview
        .blocks
        .iter()
        .filter(|block| block.tone == PreviewTone::User)
        .count();
    let assistant_message_count = session
        .preview
        .blocks
        .iter()
        .filter(|block| block.tone == PreviewTone::Assistant)
        .count();
    RemoteScannedSession {
        session_path: session.session_path.clone(),
        session_id: session_id.to_string(),
        cwd,
        started_at,
        modified_epoch: 0,
        event_count: session.preview.blocks.len(),
        user_message_count,
        assistant_message_count,
        title_hint: session.title.clone(),
        recent_context,
        cached_precis: None,
        cached_summary: None,
        storage_path,
    }
}

#[derive(Debug)]
struct SidebarRemoteMachine {
    key: String,
    label: String,
    health: MachineHealth,
    scanned_sessions: Vec<RemoteScannedSession>,
}

#[derive(Debug, Default)]
struct RemoteFolderTree {
    children: BTreeMap<String, RemoteFolderNode>,
    root_session_indices: Vec<usize>,
}

#[derive(Debug, Clone)]
struct RemoteFolderNode {
    name: String,
    full_path: String,
    children: BTreeMap<String, RemoteFolderNode>,
    session_indices: Vec<usize>,
    descendant_sessions: usize,
}

fn ssh_target_machine_label(target: &SshConnectTarget) -> String {
    let raw = if !target.label.trim().is_empty() {
        target.label.trim()
    } else {
        target
            .ssh_target
            .rsplit('@')
            .next()
            .unwrap_or(target.ssh_target.as_str())
            .trim()
    };
    format!("{raw} [ok]")
}

fn push_remote_machine_rows(
    rows: &mut Vec<BrowserRow>,
    machine: &SidebarRemoteMachine,
    expanded_paths: &HashSet<String>,
) {
    let machine_label = apply_machine_health_suffix(&machine.label, machine.health);
    let machine_path = format!("__remote_machine__/{}", machine.key);
    let machine_expanded = expanded_paths.contains(&machine_path);
    rows.push(BrowserRow {
        kind: BrowserRowKind::Group,
        full_path: machine_path.clone(),
        label: machine_label.clone(),
        detail_label: String::new(),
        document_kind: None,
        group_kind: None,
        session_title: None,
        depth: 0,
        host_label: "live".to_string(),
        descendant_sessions: machine.scanned_sessions.len(),
        expanded: machine_expanded,
        session_id: None,
        session_cwd: None,
    });
    if !machine_expanded {
        return;
    }

    let remote_short_ids = unique_session_short_ids_for_pairs(
        &machine
            .scanned_sessions
            .iter()
            .map(|session| (session.session_path.clone(), session.session_id.clone()))
            .collect::<Vec<_>>(),
    );
    let folder_tree = build_remote_folder_tree(&machine.scanned_sessions);

    for child in folder_tree.children.values() {
        append_remote_folder_rows(
            rows,
            machine,
            child,
            expanded_paths,
            &remote_short_ids,
            1,
        );
    }
    for &session_idx in &folder_tree.root_session_indices {
        if let Some(scanned) = machine.scanned_sessions.get(session_idx) {
            rows.push(BrowserRow {
                kind: BrowserRowKind::Session,
                full_path: scanned.session_path.clone(),
                label: remote_scanned_session_label(scanned, &remote_short_ids),
                detail_label: String::new(),
                document_kind: None,
                group_kind: None,
                session_title: Some(remote_scanned_session_label(scanned, &remote_short_ids)),
                depth: 1,
                host_label: machine.key.clone(),
                descendant_sessions: 1,
                expanded: true,
                session_id: Some(scanned.session_id.clone()),
                session_cwd: Some(scanned.cwd.clone()),
            });
        }
    }
}

fn remote_scanned_session_label(
    session: &RemoteScannedSession,
    short_ids: &HashMap<String, String>,
) -> String {
    let title = session.title_hint.trim();
    if !title.is_empty() && !looks_like_generated_fallback_title(title) {
        return session.title_hint.clone();
    }
    short_ids
        .get(&session.session_path)
        .cloned()
        .unwrap_or_else(|| session.title_hint.clone())
}

fn build_remote_folder_tree(sessions: &[RemoteScannedSession]) -> RemoteFolderTree {
    let mut tree = RemoteFolderTree::default();
    for (session_idx, session) in sessions.iter().enumerate() {
        let segments = session
            .cwd
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        if segments.is_empty() {
            tree.root_session_indices.push(session_idx);
            continue;
        }
        let mut current = String::new();
        let mut children = &mut tree.children;
        for (ix, segment) in segments.iter().enumerate() {
            current.push('/');
            current.push_str(segment);
            let label = if ix == 0 {
                format!("/{segment}")
            } else {
                (*segment).to_string()
            };
            let node = children
                .entry(current.clone())
                .or_insert_with(|| RemoteFolderNode {
                    name: label,
                    full_path: current.clone(),
                    children: BTreeMap::new(),
                    session_indices: Vec::new(),
                    descendant_sessions: 0,
                });
            if ix + 1 == segments.len() {
                node.session_indices.push(session_idx);
            }
            children = &mut node.children;
        }
    }
    compress_remote_folder_children(&mut tree.children);
    for child in tree.children.values_mut() {
        populate_remote_folder_descendant_counts(child);
    }
    tree
}

fn compress_remote_folder_children(children: &mut BTreeMap<String, RemoteFolderNode>) {
    let keys = children.keys().cloned().collect::<Vec<_>>();
    for key in keys {
        if let Some(child) = children.get_mut(&key) {
            compress_remote_folder_node(child, true);
        }
    }
}

fn compress_remote_folder_node(node: &mut RemoteFolderNode, can_compress_self: bool) {
    let child_keys = node.children.keys().cloned().collect::<Vec<_>>();
    for key in child_keys {
        if let Some(child) = node.children.get_mut(&key) {
            compress_remote_folder_node(child, true);
        }
    }
    if !can_compress_self {
        return;
    }
    while node.session_indices.is_empty() && node.children.len() == 1 {
        let (_, child) = node.children.pop_first().expect("child exists");
        node.name = remote_folder_join_label(&node.name, &child.name);
        node.full_path = child.full_path;
        node.session_indices = child.session_indices;
        node.descendant_sessions = child.descendant_sessions;
        node.children = child.children;
    }
}

fn remote_folder_join_label(left: &str, right: &str) -> String {
    if left == "/" {
        return format!("/{right}");
    }
    if left.ends_with('/') {
        format!("{left}{right}")
    } else {
        format!("{left}/{right}")
    }
}

fn populate_remote_folder_descendant_counts(node: &mut RemoteFolderNode) -> usize {
    let mut count = node.session_indices.len();
    for child in node.children.values_mut() {
        count += populate_remote_folder_descendant_counts(child);
    }
    node.descendant_sessions = count;
    count
}

fn compressed_remote_folder_paths(machine: &SidebarRemoteMachine, cwd: &str) -> Vec<String> {
    let tree = build_remote_folder_tree(&machine.scanned_sessions);
    let mut paths = Vec::new();
    for child in tree.children.values() {
        if collect_compressed_remote_folder_paths(child, cwd, &mut paths) {
            break;
        }
    }
    paths
}

fn collect_compressed_remote_folder_paths(
    node: &RemoteFolderNode,
    cwd: &str,
    paths: &mut Vec<String>,
) -> bool {
    if cwd != node.full_path
        && !cwd
            .strip_prefix(node.full_path.as_str())
            .is_some_and(|suffix| suffix.starts_with('/'))
    {
        return false;
    }
    paths.push(node.full_path.clone());
    for child in node.children.values() {
        if collect_compressed_remote_folder_paths(child, cwd, paths) {
            return true;
        }
    }
    true
}

fn append_remote_folder_rows(
    rows: &mut Vec<BrowserRow>,
    machine: &SidebarRemoteMachine,
    node: &RemoteFolderNode,
    expanded_paths: &HashSet<String>,
    short_ids: &HashMap<String, String>,
    depth: usize,
) {
    let row_path = format!("__remote_folder__/{}{}", machine.key, node.full_path);
    let is_expanded = expanded_paths.contains(&row_path);
    rows.push(BrowserRow {
        kind: BrowserRowKind::Group,
        full_path: row_path,
        label: node.name.clone(),
        detail_label: String::new(),
        document_kind: None,
        group_kind: None,
        session_title: None,
        depth,
        host_label: machine.key.clone(),
        descendant_sessions: node.descendant_sessions,
        expanded: is_expanded,
        session_id: None,
        session_cwd: Some(node.full_path.clone()),
    });
    if !is_expanded {
        return;
    }
    for child in node.children.values() {
        append_remote_folder_rows(rows, machine, child, expanded_paths, short_ids, depth + 1);
    }
    for &session_idx in &node.session_indices {
        if let Some(scanned) = machine.scanned_sessions.get(session_idx) {
            rows.push(BrowserRow {
                kind: BrowserRowKind::Session,
                full_path: scanned.session_path.clone(),
                label: remote_scanned_session_label(scanned, short_ids),
                detail_label: String::new(),
                document_kind: None,
                group_kind: None,
                session_title: Some(remote_scanned_session_label(scanned, short_ids)),
                depth: depth + 1,
                host_label: machine.key.clone(),
                descendant_sessions: 1,
                expanded: true,
                session_id: Some(scanned.session_id.clone()),
                session_cwd: Some(scanned.cwd.clone()),
            });
        }
    }
}

fn machine_key_from_labelish(value: &str) -> String {
    value.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn queue_remove_saved_ssh_target(mut state: Signal<ShellState>, row: BrowserRow) {
    let Some(machine_key) = saved_ssh_target_machine_key(&row, state.read().server.ssh_targets()) else {
        state.with_mut(|shell| {
            shell.close_context_menu();
            shell.push_notification(
                NotificationTone::Info,
                "No Saved Target",
                format!("{} is not backed by a saved SSH target.", row.label),
            );
        });
        return;
    };

    state.with_mut(|shell| {
        shell.select_tree_row(&row, TreeSelectionMode::Replace);
        shell.pending_delete = Some(PendingDeleteDialog {
            document_paths: Vec::new(),
            group_paths: Vec::new(),
            ssh_machine_keys: vec![machine_key],
            labels: vec![row.label.clone()],
            hard_delete: false,
        });
        shell.close_context_menu();
        shell.last_action = "confirm delete".to_string();
        shell.refresh_tree_debug("open_delete_dialog_saved_ssh_target");
    });
}

fn apply_machine_health_suffix(label: &str, health: MachineHealth) -> String {
    let base = machine_label_text(label).unwrap_or_else(|| label.to_string());
    format!(
        "{} {}",
        base,
        match health {
            MachineHealth::Healthy => "[ok]",
            MachineHealth::Cached => "[cached]",
            MachineHealth::Offline => "[offline]",
        }
    )
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
    state.with_mut(|shell| {
        shell.server_busy = true;
        shell.last_action = format!("adding separator in {}", row.label);
        shell.record_ui_telemetry(
            "separator_create_requested",
            json!({
                "source_path": row.full_path,
                "source_kind": format!("{:?}", row.kind),
                "source_group_kind": row.group_kind.map(|kind| format!("{kind:?}")),
            }),
        );
        shell.close_context_menu();
    });

    let settings = state.read().settings.clone();
    spawn(async move {
        let row_for_task = row.clone();
        let outcome =
            task::spawn_blocking(move || -> Result<(String, yggterm_core::SessionNode)> {
                let store = SessionStore::open_or_init()?;
                let virtual_path = new_separator_virtual_path_for_row(&row_for_task);
                if cfg!(debug_assertions) {
                    info!(source=%row_for_task.full_path, target=%virtual_path, "creating separator");
                }
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
                shell.record_ui_telemetry(
                    "separator_create_finished",
                    json!({
                        "source_path": row.full_path,
                        "selected_path": selected_path,
                    }),
                );
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

fn queue_move_selected_items_to_group(
    mut state: Signal<ShellState>,
    placement: WorkspaceDropPlacement,
    target_label: String,
) {
    let (selected_rows, selected_source_paths, selected_final_paths, reorder_plan) = {
        let shell = state.read();
        let selected_rows = shell.selected_workspace_rows();
        let rows = merged_sidebar_rows(
            shell.browser.rows(),
            shell.server.remote_machines(),
            shell.server.ssh_targets(),
            &shell.server.live_sessions(),
            &shell.browser.expanded_path_set(),
        );
        let reorder_plan = build_workspace_reorder_plan(&rows, &selected_rows, &placement);
        let selected_source_paths = selected_rows
            .iter()
            .map(|row| row.full_path.clone())
            .collect::<Vec<_>>();
        let selected_final_paths = reorder_plan
            .as_ref()
            .map(|plan| {
                plan.iter()
                    .filter(|item| selected_source_paths.iter().any(|path| path == &item.from_path))
                    .map(|item| item.final_path.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        (
            selected_rows,
            selected_source_paths,
            selected_final_paths,
            reorder_plan,
        )
    };
    if selected_rows.is_empty() {
        state.with_mut(|shell| {
            shell.record_ui_telemetry(
                "tree_drop_ignored",
                json!({
                    "reason": "selected_rows_empty",
                    "target_label": target_label,
                }),
            );
        });
        return;
    }
    let Some(reorder_plan) = reorder_plan else {
        state.with_mut(|shell| {
            shell.record_ui_telemetry(
                "tree_drop_ignored",
                json!({
                    "reason": "reorder_plan_none",
                    "target_label": target_label,
                    "selected_rows": selected_rows.iter().map(|row| row.full_path.clone()).collect::<Vec<_>>(),
                }),
            );
        });
        return;
    };
    state.with_mut(|shell| {
        shell.record_ui_telemetry(
            "tree_drop_requested",
            json!({
                "target_label": target_label,
                "selected_rows": selected_source_paths,
                "plan_len": reorder_plan.len(),
                "plan_preview": reorder_plan.iter().take(8).map(|item| json!({
                    "from": item.from_path,
                    "final": item.final_path,
                })).collect::<Vec<_>>(),
                "selected_final_paths": selected_final_paths,
            }),
        );
    });

    state.with_mut(|shell| {
        shell.server_busy = true;
        shell.last_action = format!(
            "moving {} item(s) to {}",
            selected_rows.len(),
            target_label
        );
        shell.close_context_menu();
        shell.drag_hover_target = None;
        shell.optimistic_drag_paths = selected_rows
            .iter()
            .map(|row| row.full_path.clone())
            .collect();
        shell.optimistic_drag_target = None;
    });

    let settings = state.read().settings.clone();
    spawn(async move {
        let reorder_plan_for_task = reorder_plan.clone();
        let outcome = task::spawn_blocking(
            move || -> Result<(Vec<String>, yggterm_core::SessionNode)> {
                let store = SessionStore::open_or_init()?;
                let moved_paths = apply_workspace_reorder_plan(&store, &reorder_plan_for_task)?;
                let browser_tree = store.load_codex_tree(&settings)?;
                Ok((moved_paths, browser_tree))
            },
        )
        .await;

        state.with_mut(|shell| match outcome {
            Ok(Ok((moved_paths, browser_tree))) => {
                shell.record_ui_telemetry(
                    "tree_drop_succeeded",
                    json!({
                        "moved_paths": moved_paths,
                        "selected_final_paths": selected_final_paths,
                        "target_label": target_label,
                    }),
                );
                let expanded_paths = shell.browser.expanded_paths();
                shell.browser = SessionBrowserState::new(browser_tree);
                shell
                    .browser
                    .restore_ui_state(&expanded_paths, selected_final_paths.first().map(String::as_str));
                shell.selected_tree_paths = selected_final_paths.iter().cloned().collect();
                if let Some(path) = selected_final_paths.first() {
                    shell.browser.select_path(path.clone());
                    shell.selection_anchor = Some(path.clone());
                }
                shell.sync_browser_settings();
                shell.server_busy = false;
                shell.clear_drag_state();
                shell.last_action = format!(
                    "moved {} item(s) to {}",
                    selected_final_paths.len(),
                    target_label
                );
                shell.push_notification(
                    NotificationTone::Success,
                    "Items Moved",
                    format!(
                        "Moved {} item(s) near {}.",
                        selected_final_paths.len(),
                        target_label
                    ),
                );
            }
            Ok(Err(error)) => {
                shell.record_ui_telemetry(
                    "tree_drop_failed",
                    json!({
                        "target_label": target_label,
                        "error": error.to_string(),
                    }),
                );
                shell.server_busy = false;
                shell.optimistic_drag_paths.clear();
                shell.optimistic_drag_target = None;
                shell.clear_drag_state();
                shell.last_action = format!("move failed: {error}");
                shell.push_notification(NotificationTone::Error, "Move Failed", error.to_string());
            }
            Err(error) => {
                shell.record_ui_telemetry(
                    "tree_drop_failed",
                    json!({
                        "target_label": target_label,
                        "error": error.to_string(),
                        "task": true,
                    }),
                );
                shell.server_busy = false;
                shell.optimistic_drag_paths.clear();
                shell.optimistic_drag_target = None;
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

fn queue_drop_current_drag_target(mut state: Signal<ShellState>) {
    let (placement, target_label) = {
        let shell = state.read();
        let Some(target) = shell.drag_hover_target.clone() else {
            drop(shell);
            state.with_mut(|shell| {
                shell.record_ui_telemetry(
                    "tree_drop_ignored",
                    json!({
                        "reason": "no_drag_hover_target",
                    }),
                );
            });
            return;
        };
        let rows = merged_sidebar_rows(
            shell.browser.rows(),
            shell.server.remote_machines(),
            shell.server.ssh_targets(),
            &shell.server.live_sessions(),
            &shell.browser.expanded_path_set(),
        );
        let Some(row) = rows.iter().find(|row| row.full_path == target.path).cloned() else {
            drop(shell);
            state.with_mut(|shell| {
                shell.record_ui_telemetry(
                    "tree_drop_ignored",
                    json!({
                        "reason": "target_row_not_found",
                        "target": target.path,
                    }),
                );
            });
            return;
        };
        let Some(placement) = resolve_workspace_drop_placement(&rows, &target) else {
            drop(shell);
            state.with_mut(|shell| {
                shell.record_ui_telemetry(
                    "tree_drop_ignored",
                    json!({
                        "reason": "drop_placement_unresolved",
                        "target": target.path,
                        "placement": format!("{:?}", target.placement).to_ascii_lowercase(),
                    }),
                );
            });
            return;
        };
        let target_label = match target.placement {
            DragDropPlacement::Into => format!("inside {}", row.label),
            DragDropPlacement::Before => format!("before {}", row.label),
            DragDropPlacement::After => format!("after {}", row.label),
        };
        (placement, target_label)
    };
    queue_move_selected_items_to_group(state, placement, target_label);
}

fn queue_delete_selected_items(mut state: Signal<ShellState>, hard_delete: bool) {
    let pending = if let Some(pending) = state.read().pending_delete.clone() {
        pending
    } else {
        let mut shell = state.write();
        let (document_paths, group_paths, mut labels) = shell.selected_workspace_delete_paths();
        let (ssh_machine_keys, ssh_labels) = shell.selected_saved_ssh_target_machine_keys();
        labels.extend(ssh_labels);
        if document_paths.is_empty() && group_paths.is_empty() && ssh_machine_keys.is_empty() {
            return;
        }
        let pending = PendingDeleteDialog {
            document_paths,
            group_paths,
            ssh_machine_keys,
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
        let item_count =
            pending.document_paths.len() + pending.group_paths.len() + pending.ssh_machine_keys.len();
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
        let endpoint = state.read().bootstrap.server_endpoint.clone();
        let outcome = task::spawn_blocking(
            move || -> Result<(usize, Option<yggterm_core::SessionNode>, Option<(ServerUiSnapshot, Option<String>)>)> {
                let mut deleted = 0usize;
                let mut browser_tree = None;
                let mut daemon_result = None;

                if !pending_for_task.document_paths.is_empty()
                    || !pending_for_task.group_paths.is_empty()
                {
                let store = SessionStore::open_or_init()?;
                    deleted += store.delete_workspace_items(
                    &pending_for_task.document_paths,
                    &pending_for_task.group_paths,
                )?;
                let settings = store.load_settings().unwrap_or_default();
                    browser_tree = Some(store.load_codex_tree(&settings)?);
                }
                for machine_key in &pending_for_task.ssh_machine_keys {
                    daemon_result = Some(remove_ssh_target(&endpoint, machine_key)?);
                    deleted += 1;
                }
                Ok((deleted, browser_tree, daemon_result))
            },
        )
        .await;

        state.with_mut(|shell| match outcome {
            Ok(Ok((deleted, maybe_browser_tree, maybe_daemon_result))) => {
                if let Some(browser_tree) = maybe_browser_tree {
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
                }
                if let Some(result) = maybe_daemon_result {
                    shell.apply_daemon_snapshot_result(Ok(result));
                }
                shell.server_busy = false;
                shell.clear_drag_state();
                shell.last_action = format!("deleted {deleted} item(s)");
                shell.refresh_tree_debug("delete_finished");
                shell.push_notification(
                    NotificationTone::Success,
                    "Items Deleted",
                    format!("Removed {deleted} item(s)."),
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
            document_parent_base(row).unwrap_or_else(|| "/workspace".to_string())
        }
        BrowserRowKind::Session => row
            .session_cwd
            .clone()
            .unwrap_or_else(|| "/workspace".to_string()),
        BrowserRowKind::Document => {
            document_parent_base(row).unwrap_or_else(|| "/workspace".to_string())
        }
    };
    format!(
        "{}/folder-{}",
        base.trim_end_matches('/'),
        unique_workspace_leaf_suffix()
    )
}

fn new_separator_virtual_path_for_row(row: &BrowserRow) -> String {
    let (base, anchor_leaf, top_of_folder) = match row.kind {
        BrowserRowKind::Group if row.full_path == "local" => ("/workspace".to_string(), None, true),
        BrowserRowKind::Group if row.full_path.starts_with("__live_") => {
            ("/workspace".to_string(), None, true)
        }
        BrowserRowKind::Group => (row.full_path.clone(), None, true),
        BrowserRowKind::Separator => (
            document_parent_base(row).unwrap_or_else(|| "/workspace".to_string()),
            workspace_leaf_name(&row.full_path),
            false,
        ),
        BrowserRowKind::Session => (
            row.session_cwd
                .clone()
                .unwrap_or_else(|| "/workspace".to_string()),
            None,
            true,
        ),
        BrowserRowKind::Document => (
            document_parent_base(row).unwrap_or_else(|| "/workspace".to_string()),
            workspace_leaf_name(&row.full_path),
            false,
        ),
    };
    let suffix = unique_workspace_leaf_suffix();
    if let Some(anchor_leaf) = anchor_leaf {
        format!(
            "{}/{}~separator-{}",
            base.trim_end_matches('/'),
            anchor_leaf,
            suffix
        )
    } else if top_of_folder {
        format!("{}/!separator-{}", base.trim_end_matches('/'), suffix)
    } else {
        format!("{}/~separator-{}", base.trim_end_matches('/'), suffix)
    }
}

fn unique_workspace_leaf_suffix() -> String {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

fn workspace_leaf_name(path: &str) -> Option<String> {
    path.rsplit('/')
        .find(|segment| !segment.is_empty())
        .map(ToOwned::to_owned)
}

fn workspace_parent_path(path: &str) -> Option<String> {
    tree_parent_path(path)
}

fn direct_workspace_parent_path(row: &BrowserRow) -> Option<String> {
    match row.kind {
        BrowserRowKind::Group | BrowserRowKind::Document | BrowserRowKind::Separator => {
            workspace_parent_path(&row.full_path)
        }
        BrowserRowKind::Session => row.session_cwd.as_deref().and_then(workspace_parent_path),
    }
}

fn browser_row_tree_item(row: &BrowserRow) -> Option<TreeReorderItem<BrowserRowKind>> {
    if !is_workspace_row(row) && !is_drop_target_row(row) {
        return None;
    }
    Some(TreeReorderItem {
        kind: row.kind,
        path: row.full_path.clone(),
        parent_path: direct_workspace_parent_path(row),
        accepts_drop_inside: row.kind == BrowserRowKind::Group
            && row.group_kind != Some(WorkspaceGroupKind::Separator)
            && !row.full_path.starts_with("__live_")
            && row.full_path != "local",
        droppable: is_drop_target_row(row),
    })
}

fn resolve_drag_drop_target(
    rows: &[BrowserRow],
    drag_paths: &[String],
    row: &BrowserRow,
    placement: DragDropPlacement,
) -> Option<DragDropTarget> {
    let items = rows
        .iter()
        .filter_map(browser_row_tree_item)
        .collect::<Vec<_>>();
    let row_item = browser_row_tree_item(row)?;
    resolve_tree_drag_drop_target(&items, drag_paths, &row_item, placement)
}

fn resolve_workspace_drop_placement(
    rows: &[BrowserRow],
    target: &DragDropTarget,
) -> Option<WorkspaceDropPlacement> {
    let items = rows
        .iter()
        .filter_map(browser_row_tree_item)
        .collect::<Vec<_>>();
    resolve_tree_drop_placement(&items, target)
}

fn context_menu_drop_placement(row: &BrowserRow) -> Option<WorkspaceDropPlacement> {
    match row.kind {
        BrowserRowKind::Group if is_drop_target_row(row) => {
            Some(WorkspaceDropPlacement::TopOfGroup(row.full_path.clone()))
        }
        BrowserRowKind::Document | BrowserRowKind::Separator => {
            Some(WorkspaceDropPlacement::AfterPath(row.full_path.clone()))
        }
        BrowserRowKind::Group | BrowserRowKind::Session => None,
    }
}

fn build_workspace_reorder_plan(
    rows: &[BrowserRow],
    selected_rows: &[BrowserRow],
    placement: &WorkspaceDropPlacement,
) -> Option<Vec<WorkspaceReorderPlanItem>> {
    let items = rows
        .iter()
        .filter_map(browser_row_tree_item)
        .collect::<Vec<_>>();
    let selected_items = selected_rows
        .iter()
        .filter_map(browser_row_tree_item)
        .collect::<Vec<_>>();
    build_tree_reorder_plan(&items, &selected_items, placement, &unique_workspace_leaf_suffix())
}

fn apply_workspace_reorder_plan(
    store: &SessionStore,
    plan: &[WorkspaceReorderPlanItem],
) -> Result<Vec<String>> {
    for item in plan {
        match item.kind {
            BrowserRowKind::Document => {
                store.move_document(&item.from_path, &item.temp_path)?;
            }
            BrowserRowKind::Group | BrowserRowKind::Separator => {
                store.move_group(&item.from_path, &item.temp_path)?;
            }
            BrowserRowKind::Session => {}
        }
    }

    let mut moved_paths = Vec::new();
    for item in plan {
        match item.kind {
            BrowserRowKind::Document => {
                let document = store.move_document(&item.temp_path, &item.final_path)?;
                moved_paths.push(document.virtual_path);
            }
            BrowserRowKind::Group | BrowserRowKind::Separator => {
                let group = store.move_group(&item.temp_path, &item.final_path)?;
                moved_paths.push(group.virtual_path);
            }
            BrowserRowKind::Session => {}
        }
    }
    Ok(moved_paths)
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
    match row.kind {
        BrowserRowKind::Group => {
            row.group_kind != Some(WorkspaceGroupKind::Separator)
                && !row.full_path.starts_with("__live_")
                && row.full_path != "local"
        }
        BrowserRowKind::Document | BrowserRowKind::Separator => true,
        BrowserRowKind::Session => false,
    }
}

fn workspace_path_contains(parent: &str, child: &str) -> bool {
    tree_path_contains(parent, child)
}

fn valid_drop_target(drag_paths: &[String], target_row: &BrowserRow) -> bool {
    let Some(item) = browser_row_tree_item(target_row) else {
        return false;
    };
    valid_tree_drop_target(drag_paths, &item)
}

fn context_menu_placement(
    position: (f64, f64),
    window_size: (f64, f64),
    menu_size: (f64, f64),
) -> ContextMenuPlacement {
    let margin = 12.0;
    let min_top = 44.0 + margin;
    let can_anchor_right = position.0 + menu_size.0 > window_size.0 - margin;
    let can_anchor_bottom = position.1 + menu_size.1 > window_size.1 - margin;
    let left = (!can_anchor_right).then(|| position.0.clamp(margin, (window_size.0 - menu_size.0 - margin).max(margin)));
    let top = (!can_anchor_bottom).then(|| position.1.clamp(min_top, (window_size.1 - menu_size.1 - margin).max(min_top)));
    let right = can_anchor_right.then(|| (window_size.0 - position.0).clamp(margin, (window_size.0 - menu_size.0 - margin).max(margin)));
    let bottom = can_anchor_bottom.then(|| (window_size.1 - position.1).clamp(margin, (window_size.1 - menu_size.1 - margin).max(margin)));
    ContextMenuPlacement {
        left,
        top,
        right,
        bottom,
    }
}

fn context_menu_position_style(placement: ContextMenuPlacement) -> String {
    let mut parts = Vec::new();
    if let Some(left) = placement.left {
        parts.push(format!("left:{left}px;"));
    } else if let Some(right) = placement.right {
        parts.push(format!("right:{right}px;"));
    }
    if let Some(top) = placement.top {
        parts.push(format!("top:{top}px;"));
    } else if let Some(bottom) = placement.bottom {
        parts.push(format!("bottom:{bottom}px;"));
    }
    parts.join(" ")
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
    let normalize_legacy_parent = |path: String| {
        path.replace("/notes/notes/", "/notes/")
            .replace("/notes/notes", "/notes")
            .replace("/local/", "/")
    };
    match row.kind {
        BrowserRowKind::Session => row.session_cwd.clone(),
        BrowserRowKind::Document => {
            parent_virtual_path(&row.full_path).map(normalize_legacy_parent)
        }
        BrowserRowKind::Separator => {
            parent_virtual_path(&row.full_path).map(normalize_legacy_parent)
        }
        BrowserRowKind::Group => {
            if row.full_path.starts_with("__live_") {
                Some("/documents".to_string())
            } else {
                Some(row.full_path.clone())
            }
        }
    }
}

fn resolve_creation_context_row(rows: &[BrowserRow], row: &BrowserRow) -> BrowserRow {
    match row.kind {
        BrowserRowKind::Group
            if row.group_kind != Some(WorkspaceGroupKind::Separator)
                && !row.full_path.starts_with("__live_") =>
        {
            row.clone()
        }
        BrowserRowKind::Document | BrowserRowKind::Separator => {
            nearest_workspace_group_row(rows, &row.full_path).unwrap_or_else(|| row.clone())
        }
        BrowserRowKind::Session => row
            .session_cwd
            .as_deref()
            .and_then(|cwd| nearest_workspace_group_row(rows, cwd))
            .unwrap_or_else(|| row.clone()),
        BrowserRowKind::Group => row.clone(),
    }
}

fn nearest_workspace_group_row(rows: &[BrowserRow], path: &str) -> Option<BrowserRow> {
    let mut cursor = Some(path.to_string());
    while let Some(candidate) = cursor {
        if let Some(row) = rows.iter().find(|row| {
            row.kind == BrowserRowKind::Group
                && row.full_path == candidate
                && row.group_kind != Some(WorkspaceGroupKind::Separator)
                && !row.full_path.starts_with("__live_")
        }) {
            return Some(row.clone());
        }
        cursor = parent_virtual_path(&candidate);
    }
    None
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

fn perf_home_dir(settings_path: &std::path::Path) -> PathBuf {
    settings_path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
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
    let mut browser_tree_load_started = use_signal(|| false);
    let mut update_check_started = use_signal(|| false);
    let mut desktop_refresh_started = use_signal(|| false);
    let mut dock_pulse_started = use_signal(|| false);
    let mut window_epoch = use_signal(|| 0_u64);
    let mut terminal_mount_epoch = use_signal(|| 0_u64);
    let mut last_terminal_session_path = use_signal(|| None::<String>);
    let mut last_open_recovery_path = use_signal(|| None::<String>);
    let mut last_preview_refresh_path = use_signal(|| None::<String>);
    use_effect(move || {
        if XTERM_ASSETS_BOOTSTRAPPED.get().is_none() {
            let _ = XTERM_ASSETS_BOOTSTRAPPED.set(());
            let _ = document::eval(&xterm_assets_bootstrap_script());
        }
    });
    use_wry_event_handler(move |event, _| {
        if let TaoEvent::WindowEvent { event, .. } = event {
            match event {
                DesktopWindowEvent::KeyboardInput { event, .. } => {
                    if event.state == ElementState::Pressed
                        && (event.logical_key == TaoKey::Delete
                            || event.physical_key == TaoKeyCode::Delete)
                        && state.read().tree_rename_path.is_none()
                    {
                        queue_delete_selected_items(state, false);
                    }
                    window_epoch.with_mut(|epoch| *epoch += 1);
                }
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
        let should_start = {
            let shell = state.read();
            shell.browser_tree_loading_in_flight && !*browser_tree_load_started.read()
        };
        if should_start {
            browser_tree_load_started.set(true);
            spawn_initial_browser_tree_load(state);
        }
    });
    use_effect(move || {
        if *desktop_refresh_started.read() {
            return;
        }
        desktop_refresh_started.set(true);
        spawn_desktop_integration_refresh(state);
    });
    use_effect(move || {
        if *update_check_started.read() {
            return;
        }
        let should_start = {
            let shell = state.read();
            shell.bootstrap.install_context.channel != yggterm_core::InstallChannel::Unknown
        };
        if should_start {
            update_check_started.set(true);
            let install_context = state.read().bootstrap.install_context.clone();
            if install_context.update_policy == yggterm_core::UpdatePolicy::NotifyOnly {
                spawn_notify_only_update_check(state);
            } else if install_context.update_policy == yggterm_core::UpdatePolicy::Auto
                && std::env::var_os("YGGTERM_SKIP_SELF_UPDATE").is_none()
            {
                spawn_auto_update_install_check(state);
            }
        }
    });
    use_effect(move || {
        let snapshot = state.read().snapshot();
        let current_terminal_session = if snapshot.active_view_mode == WorkspaceViewMode::Terminal {
            snapshot
                .active_session
                .as_ref()
                .map(|session| session.session_path.clone())
        } else {
            None
        };
        if *last_terminal_session_path.read() != current_terminal_session {
            last_terminal_session_path.set(current_terminal_session);
            terminal_mount_epoch.with_mut(|epoch| *epoch += 1);
        }
    });
    use_effect(move || {
        let active = state.read().server.active_session().cloned();
        let Some(session) = active else {
            last_preview_refresh_path.set(None);
            return;
        };
        let (has_precis, has_summary, has_good_title) = {
            let shell = state.read();
            (
                resolved_session_precis(&shell, &session).is_some(),
                resolved_session_summary(&shell, &session).is_some(),
                resolved_session_title(&shell, &session)
                    .as_deref()
                    .is_some_and(|title| !looks_like_generated_fallback_title(title)),
            )
        };
        if !has_good_title || !has_precis || !has_summary {
            spawn_active_session_copy_hydration(state, session.clone());
        }
        let has_generation_context = copy_generation_target_for_session(&state.read().server, &session)
            .and_then(|target| target.remote_context)
            .is_some_and(|context| !context.trim().is_empty())
            || preview_context_from_session(&session).is_some();
        if supports_generated_session_copy(&session)
            && !has_good_title
            && (!session.session_path.starts_with("remote-session://") || has_generation_context)
        {
            queue_active_session_title_generation(state, false);
        }
        if !has_precis
            && (!session.session_path.starts_with("remote-session://") || has_generation_context)
        {
            spawn_precis_generation(state, session.clone(), false);
        }
        if has_summary {
            state.with_mut(|shell| shell.next_background_copy_scan_after_ms = current_millis());
            maybe_spawn_background_copy_generation(state);
            return;
        }
        if session.session_path.starts_with("remote-session://") && !has_generation_context {
            state.with_mut(|shell| shell.next_background_copy_scan_after_ms = current_millis());
            maybe_spawn_background_copy_generation(state);
            return;
        }
        spawn_summary_generation(state, session, false);
        state.with_mut(|shell| shell.next_background_copy_scan_after_ms = current_millis());
        maybe_spawn_background_copy_generation(state);
    });
    use_effect(move || {
        let (active, view_mode, server_busy) = {
            let shell = state.read();
            (
                shell.server.active_session().cloned(),
                shell.server.active_view_mode(),
                shell.server_busy,
            )
        };
        let Some(session) = active else {
            last_preview_refresh_path.set(None);
            return;
        };
        if view_mode != WorkspaceViewMode::Rendered {
            last_preview_refresh_path.set(None);
            return;
        }
        if server_busy {
            state.with(|shell| shell.record_preview_issue_telemetry("preview_refresh_wait_busy"));
            return;
        }
        if !session.session_path.starts_with("remote-session://") {
            state.with(|shell| shell.record_preview_issue_telemetry("preview_refresh_skip_non_remote"));
            last_preview_refresh_path.set(None);
            return;
        }
        if *last_preview_refresh_path.read() == Some(session.session_path.clone()) {
            state.with(|shell| shell.record_preview_issue_telemetry("preview_refresh_skip_duplicate"));
            return;
        }
        state.with(|shell| shell.record_preview_issue_telemetry("preview_refresh_request"));
        last_preview_refresh_path.set(Some(session.session_path.clone()));
        spawn_server_snapshot_action(
            state,
            if remote_preview_needs_refresh(&session) {
                "refreshing preview".to_string()
            } else {
                "syncing preview".to_string()
            },
            move |endpoint| daemon_set_view_mode(&endpoint, WorkspaceViewMode::Rendered),
        );
    });
    use_effect(move || {
        let active_path = {
            let shell = state.read();
            shell.server.active_session_path().map(ToOwned::to_owned)
        };
        let Some(active_path) = active_path else {
            return;
        };
        let row_id = sidebar_row_dom_id(&active_path);
        let _ = document::eval(&format!(
            "requestAnimationFrame(() => document.getElementById({row_id:?})?.scrollIntoView({{ block: 'nearest', inline: 'nearest' }}));"
        ));
    });
    use_effect(move || {
        let (selected_row, active_session_path, server_busy) = {
            let shell = state.read();
            (
                shell.browser.selected_row().cloned(),
                shell.server.active_session_path().map(|path| path.to_string()),
                shell.server_busy,
            )
        };
        let Some(row) = selected_row else {
            last_open_recovery_path.set(None);
            return;
        };
        if active_session_path.is_some() || server_busy {
            last_open_recovery_path.set(None);
            return;
        }
        if !matches!(row.kind, BrowserRowKind::Session | BrowserRowKind::Document) {
            last_open_recovery_path.set(None);
            return;
        }
        if *last_open_recovery_path.read() == Some(row.full_path.clone()) {
            return;
        }
        last_open_recovery_path.set(Some(row.full_path.clone()));
        spawn_open_session_row(state, row);
    });
    use_effect(move || {
        let shell = state.read();
        let _ = shell.server.remote_machines().len();
        let _ = shell.server.active_session_path().map(str::len);
        let _ = shell.generated_precis.len();
        let _ = shell.generated_summaries.len();
        let _ = shell.title_requests_in_flight.len();
        let _ = shell.precis_requests_in_flight.len();
        let _ = shell.summary_requests_in_flight.len();
        let _ = shell.browser.metrics().rebuild_count;
        drop(shell);
        maybe_spawn_background_copy_generation(state);
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
    let dock_desktop = desktop.clone();
    use_effect(move || {
        let _ = *window_epoch.read();
        let snapshot = state.read().snapshot();
        let request = ghostty_dock_request(&dock_desktop, &snapshot);
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
    let inner = desktop.inner_size();
    let context_menu_window_size = (inner.width as f64, inner.height as f64);
    let titlebar_snapshot = snapshot.clone();
    let sidebar_snapshot = snapshot.clone();
    let main_snapshot = snapshot.clone();
    let metadata_snapshot = snapshot.clone();
    let preferred_agent_kind = preferred_agent_session_kind(&snapshot.settings);
    let maximized = snapshot.maximized;
    let shell_radius = if maximized { 0 } else { 11 };
    let context_menu_overlay = snapshot.context_menu_row.clone().map(|row| {
        let context_row = resolve_creation_context_row(&snapshot.rows, &row);
        (row, context_row)
    });

    rsx! {
        div {
            id: "yggterm-shell-root",
            tabindex: "0",
            style: format!(
                "position: fixed; inset: 0; overflow: hidden; border-radius:{}px; \
                 background-color:{}; background-image:{}; backdrop-filter: blur(30px) saturate(165%); \
                 -webkit-backdrop-filter: blur(30px) saturate(165%);",
                shell_radius, snapshot.shell_tint, snapshot.shell_gradient
            ),
            onmouseup: move |_| {
                if !state.read().drag_paths.is_empty() {
                    queue_drop_current_drag_target(state);
                    state.with_mut(|shell| shell.clear_drag_state());
                }
            },
            onkeydown: move |evt| {
                if evt.key() == Key::Delete && state.read().tree_rename_path.is_none() {
                    evt.prevent_default();
                    queue_delete_selected_items(state, false);
                    return;
                }
                let is_accel = evt.modifiers().contains(Modifiers::CONTROL)
                    || evt.modifiers().contains(Modifiers::META);
                let is_terminal_shortcut = is_accel
                    && evt.modifiers().contains(Modifiers::SHIFT)
                    && matches!(evt.key(), Key::Character(ref key) if key.eq_ignore_ascii_case("c") || key.eq_ignore_ascii_case("x"));
                if is_terminal_shortcut && state.read().snapshot().active_view_mode != WorkspaceViewMode::Terminal {
                    let (title, message) = match evt.key() {
                        Key::Character(key) if key.eq_ignore_ascii_case("x") => (
                            "Cut to Clipboard",
                            "Moved the current UI selection to the clipboard.",
                        ),
                        _ => (
                            "Copied to Clipboard",
                            "Copied the current UI selection to the clipboard.",
                        ),
                    };
                    state.with_mut(|shell| {
                        shell.push_notification(NotificationTone::Success, title, message);
                    });
                }
            },
            oncontextmenu: |evt| {
                evt.prevent_default();
                evt.stop_propagation();
            },
            style { "{TOAST_CSS}" }
            div {
                style: shell_style(snapshot.palette, shell_radius, &snapshot.shell_tint, &snapshot.shell_gradient),
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
                    on_restart_update: move || restart_into_pending_update(state),
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
                        on_select_row: move |(row, mode): (BrowserRow, TreeSelectionMode)| {
                            let row_for_log = row.clone();
                            if let Err(error) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                let should_continue = {
                                    let mut continue_open = true;
                                    state.with_mut(|shell| {
                                        if shell.consume_suppressed_tree_click() {
                                            continue_open = false;
                                            return;
                                        }
                                        shell.select_tree_row(&row, mode);
                                    });
                                    continue_open
                                };
                                let _ = document::eval(
                                    "document.getElementById('yggterm-shell-root')?.focus?.(); document.getElementById('yggterm-sidebar')?.focus?.();",
                                );
                                if !should_continue {
                                    return;
                                }
                                if mode != TreeSelectionMode::Replace {
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
                                let should_generate =
                                    (is_local_stored_session_row(&row) && row.session_title.is_none())
                                        || is_remote_scanned_sidebar_row(&row);
                                match row.kind {
                                    BrowserRowKind::Group | BrowserRowKind::Separator => state.with_mut(|shell| shell.select_row(&row)),
                                    BrowserRowKind::Session | BrowserRowKind::Document => spawn_open_session_row(state, row.clone()),
                                }
                                if should_generate {
                                    spawn_deferred_title_generation(state, row.clone(), false);
                                }
                            })) {
                                warn!(
                                    path=%row_for_log.full_path,
                                    kind=?row_for_log.kind,
                                    mode=?mode,
                                    panic_payload=?error,
                                    "suppressed sidebar row select panic"
                                );
                            }
                        },
                        on_delete_selected_items: move |hard_delete: bool| queue_delete_selected_items(state, hard_delete),
                        on_open_context_menu: move |(row, position): (BrowserRow, (f64, f64))| {
                            state.with_mut(|shell| {
                                if !shell.selected_tree_paths.contains(&row.full_path) {
                                    shell.select_tree_row(&row, TreeSelectionMode::Replace);
                                }
                                shell.open_context_menu(row, position)
                            })
                        },
                        on_start_drag: move |(row, pointer): (BrowserRow, (f64, f64))| {
                            state.with_mut(|shell| shell.begin_drag(&row, pointer))
                        },
                        on_drag_hover: move |(row, pointer, placement): (BrowserRow, (f64, f64), DragDropPlacement)| {
                            state.with_mut(|shell| shell.set_drag_hover_target(&row, pointer, placement))
                        },
                        on_drag_move: move |pointer: (f64, f64)| {
                            state.with_mut(|shell| shell.update_drag_pointer(pointer))
                        },
                        on_drag_leave: move |_row: BrowserRow| {},
                        on_drop_into_row: move |_| queue_drop_current_drag_target(state),
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
                        state,
                        snapshot: main_snapshot,
                        terminal_mount_epoch: *terminal_mount_epoch.read(),
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
                        on_refresh_title: move |_| {
                            spawn_deferred_active_session_title_generation(state, true)
                        },
                        on_refresh_precis: move |_| {
                            if let Some(session) = state.read().server.active_session().cloned() {
                                spawn_precis_generation(state, session, true);
                            }
                        },
                        on_refresh_summary: move |_| {
                            if let Some(session) = state.read().server.active_session().cloned() {
                                spawn_summary_generation(state, session, true);
                            }
                        },
                    }
                    RightRail {
                        snapshot: metadata_snapshot,
                        on_endpoint_change: move |value: String| state.with_mut(|shell| shell.update_litellm_endpoint(value)),
                        on_api_key_change: move |value: String| state.with_mut(|shell| shell.update_litellm_api_key(value)),
                        on_model_change: move |value: String| state.with_mut(|shell| shell.update_interface_llm_model(value)),
                        on_set_ui_theme: move |theme: UiTheme| state.with_mut(|shell| shell.set_ui_theme(theme)),
                        on_open_theme_editor: move |_| state.with_mut(|shell| shell.open_theme_editor()),
                        on_set_notification_delivery: move |mode: NotificationDeliveryMode| {
                            state.with_mut(|shell| shell.update_notification_delivery(mode))
                        },
                        on_set_notification_sound: move |enabled: bool| {
                            state.with_mut(|shell| shell.update_notification_sound(enabled))
                        },
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
                if let Some((row, context_row)) = context_menu_overlay.clone() {
                    ContextMenuOverlay {
                        row: row.clone(),
                        position: snapshot.context_menu_position.unwrap_or((18.0, 60.0)),
                        window_size: context_menu_window_size,
                        selected_row: snapshot.selected_row.clone(),
                        selected_tree_paths: snapshot.selected_tree_paths.clone(),
                        can_remove_saved_ssh_target: saved_ssh_target_machine_key(&row, &snapshot.ssh_targets).is_some(),
                        palette: snapshot.palette,
                        on_close: move |_| state.with_mut(|shell| shell.close_context_menu()),
                        on_create_group_codex: {
                            let row = context_row.clone();
                            let preferred_agent_kind = preferred_agent_kind;
                            move |_| {
                                spawn_start_group_session(state, row.clone(), preferred_agent_kind)
                            }
                        },
                        on_create_group: {
                            let row = context_row.clone();
                            move |_| queue_new_group_for_row(state, row.clone())
                        },
                        on_create_group_shell: {
                            let row = context_row.clone();
                            move |_| spawn_start_group_session(state, row.clone(), SessionKind::Shell)
                        },
                        on_create_group_document: {
                            let row = context_row.clone();
                            move |_| queue_new_document_for_row(state, row.clone())
                        },
                        on_create_group_recipe: {
                            let row = row.clone();
                            move |_| queue_new_separator_for_row(state, row.clone())
                        },
                        on_move_selected_document_here: {
                            let row = context_row.clone();
                            move |_| {
                                if let Some(placement) = context_menu_drop_placement(&row) {
                                    queue_move_selected_items_to_group(
                                        state,
                                        placement,
                                        format!("near {}", row.label),
                                    );
                                }
                            }
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
                                spawn_deferred_title_generation(state, row.clone(), true);
                            }
                        },
                        on_refresh_remote_machine: {
                            let row = row.clone();
                            let machine_key = row
                                .full_path
                                .strip_prefix("__remote_machine__/")
                                .map(ToOwned::to_owned);
                            move |_| {
                                if let Some(machine_key) = machine_key.clone() {
                                    spawn_server_snapshot_action(
                                        state,
                                        format!("refreshing {}", row.label),
                                        move |endpoint| {
                                            refresh_remote_machine(&endpoint, &machine_key)
                                        },
                                    );
                                }
                            }
                        },
                        on_remove_ssh_target: {
                            let row = row.clone();
                            move |_| queue_remove_saved_ssh_target(state, row.clone())
                        },
                        on_delete_item: {
                            let row = row.clone();
                            move |_| {
                                state.with_mut(|shell| {
                                    shell.select_tree_row(&row, TreeSelectionMode::Replace);
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
                if snapshot.theme_editor_open {
                    ThemeEditorOverlay {
                        snapshot: snapshot.clone(),
                        on_close: move |_| state.with_mut(|shell| shell.close_theme_editor()),
                        on_save: move |_| state.with_mut(|shell| shell.save_theme_editor()),
                        on_reset: move |_| state.with_mut(|shell| shell.reset_theme_editor()),
                        on_seed: move |_| state.with_mut(|shell| shell.seed_theme_editor()),
                        on_set_ui_theme: move |theme: UiTheme| state.with_mut(|shell| shell.set_ui_theme(theme)),
                        on_add_stop: move |_| state.with_mut(|shell| shell.add_theme_stop(None)),
                        on_remove_stop: move |_| state.with_mut(|shell| shell.remove_selected_theme_stop()),
                        on_pick_stop: move |index: usize| state.with_mut(|shell| shell.select_theme_stop(index)),
                        on_begin_drag_stop: move |index: usize| state.with_mut(|shell| shell.begin_theme_drag(index)),
                        on_drag_stop: move |(x, y): (f32, f32)| state.with_mut(|shell| shell.move_theme_stop(x, y)),
                        on_end_drag_stop: move |_| state.with_mut(|shell| shell.end_theme_drag()),
                        on_double_click_pad: move |(x, y): (f32, f32)| state.with_mut(|shell| shell.add_theme_stop_at(x, y)),
                        on_update_stop_color: move |value: String| state.with_mut(|shell| shell.update_selected_theme_color(value)),
                        on_pick_swatch: move |value: String| state.with_mut(|shell| shell.update_selected_theme_color(value)),
                        on_set_brightness: move |value: f32| state.with_mut(|shell| shell.update_theme_brightness(value)),
                        on_set_grain: move |value: f32| state.with_mut(|shell| shell.update_theme_grain(value)),
                    }
                }
                if !snapshot.notifications.is_empty() {
                    ToastViewport {
                        items: snapshot.notifications.clone(),
                        palette: ToastPalette {
                            text: snapshot.palette.text,
                            muted: snapshot.palette.muted,
                            accent: snapshot.palette.accent,
                        },
                        center_offset: toast_center_offset(snapshot.right_panel_mode),
                        max_age_ms: 7000,
                        max_visible: 3,
                        now_ms: current_millis(),
                        on_clear: move |id: u64| state.with_mut(|shell| shell.clear_notification(id)),
                    }
                }
                if !snapshot.drag_paths.is_empty() {
                    DragGhost {
                        snapshot: snapshot.clone(),
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
    on_restart_update: EventHandler<()>,
    on_toggle_maximized: EventHandler<()>,
    on_toggle_always_on_top: EventHandler<()>,
    maximized: bool,
) -> Element {
    rsx! {
        TitlebarChrome {
            background: snapshot.palette.titlebar.to_string(),
            zoom_percent: zoom_percent_f32(snapshot.settings.ui_font_size, 14.0),
            on_toggle_maximized: on_toggle_maximized,
            left: rsx! {
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
                    div { style: "flex:1; min-width:56px; height:100%;" }
                }
            },
            center: rsx! {
                div {
                    style: "flex:1; display:flex; align-items:center; justify-content:center; gap:10px; padding:0 16px;",
                    div { style: "flex:1; min-width:84px; height:100%;" }
                    input {
                        r#type: "text",
                        value: "{snapshot.search_query}",
                        placeholder: "Search sessions…",
                        style: search_input_style(snapshot.palette.text),
                        onmousedown: |evt| evt.stop_propagation(),
                        ondoubleclick: |evt| evt.stop_propagation(),
                        oninput: move |evt| on_search.call(evt.value()),
                    }
                    if let Some(update) = snapshot.pending_update_restart.clone() {
                        button {
                            style: format!(
                                "display:inline-flex; align-items:center; gap:7px; height:28px; padding:0 11px; border:none; border-radius:10px; \
                                 background:rgba(255,255,255,0.82); color:{}; font-size:11px; font-weight:700; cursor:pointer; \
                                 box-shadow: inset 0 0 0 1px rgba(170,190,212,0.24); white-space:nowrap;",
                                snapshot.palette.accent
                            ),
                            onmousedown: |evt| evt.stop_propagation(),
                            ondoubleclick: |evt| evt.stop_propagation(),
                            onclick: move |_| on_restart_update.call(()),
                            "Restart to Use {update.version}"
                        }
                    }
                    div { style: "flex:1; min-width:84px; height:100%;" }
                }
            },
            right: rsx! {
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
                    div { style: "flex:1; min-width:48px; height:100%;" }
                    WindowControlsStrip {
                        palette: ChromePalette {
                            titlebar: snapshot.palette.titlebar,
                            text: snapshot.palette.text,
                            muted: snapshot.palette.muted,
                            accent: snapshot.palette.accent,
                            close_hover: snapshot.palette.close_hover,
                            control_hover: snapshot.palette.control_hover,
                        },
                        hovered: hovered(),
                        on_hover_control: on_hover_control,
                        on_toggle_maximized: on_toggle_maximized,
                        on_toggle_always_on_top: on_toggle_always_on_top,
                        maximized: maximized,
                        always_on_top: snapshot.always_on_top,
                    }
                }
            },
        }
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
    on_select_row: EventHandler<(BrowserRow, TreeSelectionMode)>,
    on_delete_selected_items: EventHandler<bool>,
    on_open_context_menu: EventHandler<(BrowserRow, (f64, f64))>,
    on_start_drag: EventHandler<(BrowserRow, (f64, f64))>,
    on_drag_hover: EventHandler<(BrowserRow, (f64, f64), DragDropPlacement)>,
    on_drag_move: EventHandler<(f64, f64)>,
    on_drag_leave: EventHandler<BrowserRow>,
    on_drop_into_row: EventHandler<()>,
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
    let drag_active = !snapshot.drag_paths.is_empty();
    rsx! {
        div {
            id: "yggterm-sidebar",
            style: format!(
                "width:{}px; min-width:{}px; max-width:{}px; display:flex; flex-direction:column; \
                 background:{}; overflow:hidden; transition: opacity 180ms ease, transform 180ms ease; \
                 opacity:{}; transform:{}; pointer-events:{}; zoom:{}%; user-select:none; \
                 -webkit-user-select:none;",
                width, width, width, snapshot.palette.sidebar, opacity, translate,
                if snapshot.sidebar_open { "auto" } else { "none" },
                zoom_percent_f32(snapshot.settings.ui_font_size, 14.0)
            ),
            tabindex: "0",
            onkeydown: move |evt| {
                if evt.key() == Key::Delete {
                    evt.prevent_default();
                    on_delete_selected_items.call(false);
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
                onmousemove: move |evt| {
                    if drag_active {
                        let coords = evt.client_coordinates();
                        on_drag_move.call((coords.x, coords.y));
                    }
                },
                onmouseup: move |_| {
                    if drag_active {
                        on_drop_into_row.call(());
                        on_end_drag.call(());
                    }
                },
                if snapshot.show_loading_tree {
                    SidebarLoadingState { palette: snapshot.palette }
                } else {
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
                                drop_target: snapshot
                                    .drag_hover_target
                                    .as_ref()
                                    .filter(|target| target.path == row.full_path)
                                    .map(|target| target.placement),
                                dragging: snapshot.drag_paths.iter().any(|path| path == &row.full_path),
                                drag_active: !snapshot.drag_paths.is_empty(),
                                renaming: snapshot.tree_rename_path.as_deref() == Some(row.full_path.as_str()),
                                rename_value: snapshot.tree_rename_value.clone(),
                                palette: snapshot.palette,
                                on_select: move |evt: MouseEvent| {
                                    let mode = if evt.modifiers().contains(Modifiers::SHIFT) {
                                        TreeSelectionMode::ExtendRange
                                    } else if evt.modifiers().contains(Modifiers::CONTROL)
                                        || evt.modifiers().contains(Modifiers::META)
                                    {
                                        TreeSelectionMode::Toggle
                                    } else {
                                        TreeSelectionMode::Replace
                                    };
                                    on_select_row.call((select_row.clone(), mode));
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
                                    move |evt: MouseEvent| {
                                        let coords = evt.client_coordinates();
                                        on_start_drag.call((row.clone(), (coords.x, coords.y)))
                                    }
                                },
                                on_drag_hover: {
                                    let row = row.clone();
                                    move |(placement, evt): (DragDropPlacement, MouseEvent)| {
                                        let coords = evt.client_coordinates();
                                        on_drag_hover.call((row.clone(), (coords.x, coords.y), placement))
                                    }
                                },
                                on_drag_leave: {
                                    let row = row.clone();
                                    move |_| on_drag_leave.call(row.clone())
                                },
                                on_drop_into_row: move |_| on_drop_into_row.call(()),
                                on_end_drag: move |_| on_end_drag.call(()),
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
fn SidebarLoadingState(palette: Palette) -> Element {
    rsx! {
        style {
            "{TREE_LOADING_DOT_CSS}"
        }
        div {
            style: "display:flex; align-items:center; gap:10px; padding:12px 10px; border-radius:14px; background:rgba(95,168,255,0.08);",
            div {
                style: format!("font-size:12px; font-weight:800; color:{};", palette.accent),
                "Loading"
            }
            div {
                style: "display:flex; align-items:center; gap:4px;",
                for ix in 0..3 {
                    span {
                        style: format!(
                            "width:6px; height:6px; border-radius:999px; background:{}; display:inline-block; \
                             animation:yggterm-tree-loading-dot 1.1s ease-in-out infinite; animation-delay:{}ms;",
                            palette.accent,
                            ix * 140
                        ),
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
    drop_target: Option<DragDropPlacement>,
    dragging: bool,
    drag_active: bool,
    renaming: bool,
    rename_value: String,
    palette: Palette,
    on_select: EventHandler<MouseEvent>,
    on_open_context_menu: EventHandler<(f64, f64)>,
    on_begin_rename: EventHandler<MouseEvent>,
    on_update_rename: EventHandler<String>,
    on_commit_rename: EventHandler<()>,
    on_cancel_rename: EventHandler<()>,
    on_start_drag: EventHandler<MouseEvent>,
    on_drag_hover: EventHandler<(DragDropPlacement, MouseEvent)>,
    on_drag_leave: EventHandler<MouseEvent>,
    on_drop_into_row: EventHandler<()>,
    on_end_drag: EventHandler<()>,
) -> Element {
    let indent = row.depth * 12 + 12;
    let draggable = is_workspace_row(&row);
    let drop_hovered = drop_target.is_some();
    let can_drop_inside =
        row.kind == BrowserRowKind::Group && row.group_kind != Some(WorkspaceGroupKind::Separator);
    let top_line = drop_target == Some(DragDropPlacement::Before);
    let bottom_line = drop_target == Some(DragDropPlacement::After);
    let fill_target = drop_target == Some(DragDropPlacement::Into);
    if row.kind == BrowserRowKind::Separator {
        return rsx! {
            div {
                id: "{sidebar_row_dom_id(&row.full_path)}",
                style: format!(
                    "width:100%; display:flex; align-items:center; gap:10px; border:none; background:transparent; cursor:{}; \
                     padding:8px 9px 8px {}px; margin:0; opacity:{}; border-radius:12px; background:{}; \
                     box-sizing:border-box; min-width:0; overflow:hidden; user-select:none; -webkit-user-select:none; \
                     transition: transform 140ms ease, background 140ms ease, opacity 140ms ease, box-shadow 140ms ease; \
                     transform:translateY(0px); box-shadow:{}; position:relative;",
                    if dragging { "grabbing" } else if draggable { "grab" } else { "default" },
                    indent
                    , if dragging { "0.58" } else { "1" },
                    if selected || fill_target { palette.accent_soft } else { "transparent" },
                    if top_line && drag_active {
                        format!("inset 0 2px 0 {}", palette.accent)
                    } else if bottom_line && drag_active {
                        format!("inset 0 -2px 0 {}", palette.accent)
                    } else {
                        "none".to_string()
                    },
                ),
                draggable: false,
                onclick: move |evt| on_select.call(evt),
                onmousedown: move |evt| {
                    if draggable
                        && evt.trigger_button() == Some(MouseButton::Primary)
                        && !evt.modifiers().contains(Modifiers::SHIFT)
                    && !evt.modifiers().contains(Modifiers::CONTROL)
                    && !evt.modifiers().contains(Modifiers::META)
                    {
                        evt.prevent_default();
                        on_start_drag.call(evt);
                    }
                },
                ondoubleclick: move |evt| on_begin_rename.call(evt),
                oncontextmenu: move |evt| {
                    evt.prevent_default();
                    evt.stop_propagation();
                    let coords = evt.client_coordinates();
                    on_open_context_menu.call((coords.x, coords.y));
                },
                onmouseleave: move |evt| {
                    on_drag_leave.call(evt);
                },
                onmouseup: move |_| {
                    if drag_active {
                        on_drop_into_row.call(());
                        on_end_drag.call(());
                    }
                },
                TreeDropZones {
                    drag_active: drag_active,
                    can_drop_inside: false,
                    on_drag_hover: on_drag_hover,
                    on_drop: on_drop_into_row,
                    on_end_drag: on_end_drag,
                }
                div {
                    style: format!(
                        "flex:1; min-width:0; height:{}px; background:{}; opacity:{};",
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
                            "min-width:0; font-size:10.5px; font-weight:700; letter-spacing:0.04em; color:{}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;",
                            if drop_hovered || selected { palette.accent } else { palette.muted }
                        ),
                        "{row.label}"
                    }
                }
                div {
                    style: format!(
                        "flex:1; min-width:0; height:{}px; background:{}; opacity:{};",
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
    let machine_label = machine_label_text(&row.label);
    let machine_health = machine_health_from_label(&row.label);
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
        div {
            id: "{sidebar_row_dom_id(&row.full_path)}",
            style: format!(
                "width:100%; display:flex; flex-direction:column; align-items:stretch; gap:2px; \
                 border:none; border-radius:12px; background:{}; padding:6px 9px 6px {}px; margin:0; opacity:{}; cursor:{}; \
                 box-sizing:border-box; min-width:0; overflow:hidden; user-select:none; -webkit-user-select:none; \
                 transition: transform 140ms ease, background 140ms ease, opacity 140ms ease, box-shadow 140ms ease; \
                 transform:translateY(0px); box-shadow:{}; position:relative;",
                if fill_target { "rgba(95, 168, 255, 0.14)" } else { background },
                indent,
                if dragging { "0.58" } else { "1" },
                if dragging { "grabbing" } else if draggable { "grab" } else { "default" },
                if top_line && drag_active {
                    format!("inset 0 2px 0 {}", palette.accent)
                } else if bottom_line && drag_active {
                    format!("inset 0 -2px 0 {}", palette.accent)
                } else {
                    "none".to_string()
                },
            ),
            draggable: false,
            onclick: move |evt| on_select.call(evt),
            onmousedown: move |evt| {
                if draggable
                    && evt.trigger_button() == Some(MouseButton::Primary)
                    && !evt.modifiers().contains(Modifiers::SHIFT)
                    && !evt.modifiers().contains(Modifiers::CONTROL)
                    && !evt.modifiers().contains(Modifiers::META)
                {
                    evt.prevent_default();
                    on_start_drag.call(evt);
                }
            },
            ondoubleclick: move |evt| on_begin_rename.call(evt),
            oncontextmenu: move |evt| {
                evt.prevent_default();
                evt.stop_propagation();
                let coords = evt.client_coordinates();
                on_open_context_menu.call((coords.x, coords.y));
            },
            onmouseleave: move |evt| {
                on_drag_leave.call(evt);
            },
            onmouseup: move |_| {
                if drag_active {
                    on_drop_into_row.call(());
                    on_end_drag.call(());
                }
            },
            TreeDropZones {
                drag_active: drag_active,
                can_drop_inside: can_drop_inside,
                on_drag_hover: on_drag_hover,
                on_drop: on_drop_into_row,
                on_end_drag: on_end_drag,
            }
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
                            "{machine_label.clone().unwrap_or_else(|| row.label.clone())}"
                        }
                    }
                    if let Some(health) = machine_health {
                        span {
                            style: format!(
                                "display:inline-flex; width:6px; min-width:6px; height:6px; border-radius:999px; background:{}; box-shadow:0 0 0 1.5px rgba(255,255,255,0.74); opacity:0.96;",
                                match health {
                                    MachineHealth::Healthy => "#16a34a",
                                    MachineHealth::Cached => "#f59e0b",
                                    MachineHealth::Offline => "#ef4444",
                                }
                            ),
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

fn machine_health_from_label(label: &str) -> Option<MachineHealth> {
    if label.ends_with("[ok]") {
        Some(MachineHealth::Healthy)
    } else if label.ends_with("[cached]") {
        Some(MachineHealth::Cached)
    } else if label.ends_with("[offline]") {
        Some(MachineHealth::Offline)
    } else {
        None
    }
}

fn machine_label_text(label: &str) -> Option<String> {
    machine_health_from_label(label).map(|_| {
        label.rsplit_once(" [")
            .map(|(base, _)| base.to_string())
            .unwrap_or_else(|| label.to_string())
    })
}

#[component]
fn DragGhost(snapshot: RenderSnapshot) -> Element {
    let Some((x, y)) = snapshot.drag_pointer else {
        return rsx! {};
    };
    let dragged_rows = snapshot
        .rows
        .iter()
        .filter(|row| snapshot.drag_paths.iter().any(|path| path == &row.full_path))
        .cloned()
        .collect::<Vec<_>>();
    if dragged_rows.is_empty() {
        return rsx! {};
    }
    let primary_label = dragged_rows
        .first()
        .map(|row| row.label.clone())
        .unwrap_or_else(|| "Move item".to_string());
    let extra_count = dragged_rows.len().saturating_sub(1);
    let drop_target_hint = snapshot.drag_hover_target.as_ref().map(|target| {
        let placement = match target.placement {
            DragDropPlacement::Before => "before",
            DragDropPlacement::Into => "inside",
            DragDropPlacement::After => "after",
        };
        let leaf = workspace_leaf_name(&target.path).unwrap_or_else(|| "item".to_string());
        format!("Drop {placement} {leaf}")
    });
    rsx! {
        DragGhostCard {
            x: x,
            y: y,
            primary_label: primary_label,
            extra_count: extra_count,
            target_hint: drop_target_hint,
            palette: DragGhostPalette {
                text: snapshot.palette.text,
                muted: snapshot.palette.muted,
                accent: snapshot.palette.accent,
                accent_soft: snapshot.palette.accent_soft,
            },
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
    state: Signal<ShellState>,
    snapshot: RenderSnapshot,
    terminal_mount_epoch: u64,
    on_expand_preview: EventHandler<()>,
    on_collapse_preview: EventHandler<()>,
    on_toggle_preview_block: EventHandler<usize>,
    on_set_preview_layout: EventHandler<PreviewLayoutMode>,
    on_save_document: EventHandler<(String, WorkspaceDocumentInput)>,
    on_run_recipe_document: EventHandler<(String, WorkspaceDocumentInput, bool)>,
    on_switch_agent_mode: EventHandler<SessionKind>,
    on_refresh_title: EventHandler<()>,
    on_refresh_precis: EventHandler<()>,
    on_refresh_summary: EventHandler<()>,
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
                            style: "display:flex; flex-direction:column; min-width:0; min-height:0; width:100%; height:100%;",
                            div {
                                style: "display:flex; align-items:flex-start; justify-content:space-between; gap:16px; padding:22px 26px 14px 26px; border-bottom:1px solid rgba(170,190,212,0.16);",
                                div {
                                    style: "display:flex; flex-direction:column; gap:16px; min-width:0; flex:1;",
                                    SessionHeaderCopy {
                                        title: snapshot
                                            .active_title
                                            .clone()
                                            .unwrap_or_else(|| session.title.clone()),
                                        subtitle: snapshot
                                            .active_summary
                                            .clone()
                                            .unwrap_or_else(|| preview_summary_text(&session)),
                                        palette: snapshot.palette,
                                        on_refresh_title: move |_| on_refresh_title.call(()),
                                        on_refresh_subtitle: move |_| on_refresh_summary.call(()),
                                    }
                                }
                                PreviewToolbar {
                                    palette: snapshot.palette,
                                    preview_layout: snapshot.preview_layout,
                                    server_busy: snapshot.server_busy,
                                    on_expand_preview: move |_| on_expand_preview.call(()),
                                    on_collapse_preview: move |_| on_collapse_preview.call(()),
                                    on_set_preview_layout: move |mode| on_set_preview_layout.call(mode),
                                }
                            }
                            div {
                                style: "display:flex; flex-direction:column; gap:18px; min-width:0; min-height:0; overflow:auto; padding:24px;",
                                if snapshot.preview_layout == PreviewLayoutMode::Chat {
                                    div {
                                        style: "display:flex; flex-direction:column; gap:18px; min-width:0; width:min(980px, 100%); margin:0 auto;",
                                        if !session.rendered_sections.is_empty() {
                                            RenderedSectionsStrip {
                                                sections: session.rendered_sections.clone(),
                                                palette: snapshot.palette,
                                            }
                                        }
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
                                    div {
                                        style: "width:min(980px, 100%); margin:0 auto;",
                                        PreviewGraph {
                                            session: session.clone(),
                                            palette: snapshot.palette,
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            WorkspaceViewMode::Terminal => rsx! {
                div {
                    key: "{session.session_path}:{snapshot.settings.terminal_font_size}:{snapshot.active_view_mode as u8}:{terminal_mount_epoch}",
                    style: "display:flex; flex-direction:column; min-width:0; min-height:0; width:100%; height:100%;",
                    div {
                        style: "display:flex; align-items:flex-start; justify-content:space-between; gap:16px; padding:22px 26px 14px 26px; border-bottom:1px solid rgba(170,190,212,0.16);",
                        SessionHeaderCopy {
                            title: snapshot
                                .active_title
                                .clone()
                                .unwrap_or_else(|| session.title.clone()),
                            subtitle: snapshot
                                .active_precis
                                .clone()
                                .unwrap_or_else(|| terminal_precis(&session)),
                            palette: snapshot.palette,
                            on_refresh_title: move |_| on_refresh_title.call(()),
                            on_refresh_subtitle: move |_| on_refresh_precis.call(()),
                        }
                        if session.kind.is_agent() {
                            AgentModeSelector {
                                selected: session.kind,
                                palette: snapshot.palette,
                                disabled: snapshot.server_busy,
                                on_select: on_switch_agent_mode,
                            }
                        }
                    }
                    TerminalCanvas {
                        key: "{terminal_instance_key(&session.session_path, snapshot.settings.terminal_font_size)}:{terminal_mount_epoch}",
                        session: session.clone(),
                        snapshot: snapshot.clone(),
                        state,
                        mount_epoch: terminal_mount_epoch,
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
fn SessionHeaderCopy(
    title: String,
    subtitle: String,
    palette: Palette,
    on_refresh_title: EventHandler<MouseEvent>,
    on_refresh_subtitle: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:10px; min-width:280px; flex:1; min-height:0;",
            div {
                style: "display:flex; align-items:flex-start; justify-content:space-between; gap:12px;",
                div {
                    style: "display:flex; flex-direction:column; gap:4px; min-width:0;",
                    div {
                        style: "display:flex; align-items:center; gap:8px; min-width:0;",
                        div {
                            style: format!("font-size:22px; font-weight:700; color:{}; line-height:1.2; min-width:0;", palette.text),
                            "{title}"
                        }
                        RefreshInlineButton {
                            label: "Regenerate title".to_string(),
                            palette,
                            onclick: move |evt| on_refresh_title.call(evt),
                        }
                    }
                }
            }
            div {
                style: "display:flex; align-items:flex-start; gap:8px; min-width:0; min-height:0;",
                div {
                    style: format!("font-size:12px; line-height:1.72; color:{}; white-space:pre-wrap; overflow-wrap:anywhere; min-width:0; flex:1; max-height:48vh; overflow:auto; padding-right:6px; scrollbar-gutter:stable;", palette.muted),
                    "{subtitle}"
                }
                RefreshInlineButton {
                    label: "Regenerate detail".to_string(),
                    palette,
                    onclick: move |evt| on_refresh_subtitle.call(evt),
                }
            }
        }
    }
}

#[component]
fn RefreshInlineButton(
    label: String,
    palette: Palette,
    onclick: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        button {
            title: "{label}",
            style: format!(
                "display:inline-flex; align-items:center; justify-content:center; width:22px; height:22px; \
                 border:none; border-radius:999px; background:rgba(255,255,255,0.72); color:{}; \
                 box-shadow: inset 0 0 0 1px rgba(170,190,212,0.2); padding:0; flex:0 0 auto;",
                palette.muted
            ),
            onclick: move |evt| onclick.call(evt),
            svg {
                width: "12",
                height: "12",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                path { d: "M21 2v6h-6" }
                path { d: "M3 12a9 9 0 0 1 15.55-6.36L21 8" }
                path { d: "M3 22v-6h6" }
                path { d: "M21 12a9 9 0 0 1-15.55 6.36L3 16" }
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
    let summary_entries = session
        .preview
        .summary
        .iter()
        .filter(|entry| {
            !matches!(
                entry.label,
                "Session" | "Storage" | "Cwd" | "Started" | "Updated"
            ) && !entry.value.trim().is_empty()
        })
        .take(4)
        .cloned()
        .collect::<Vec<_>>();
    let timeline_excerpt = session
        .preview
        .blocks
        .iter()
        .rev()
        .find_map(|block| preview_block_excerpt(block, 140))
        .unwrap_or_else(|| preview_summary_text(&session));
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:18px; padding:6px 0 4px 0;",
            div {
                style: "display:flex; flex-direction:column; gap:14px;",
                div {
                    style: "display:flex; align-items:flex-start; justify-content:space-between; gap:16px; flex-wrap:wrap;",
                    div {
                        style: "display:flex; flex-direction:column; gap:6px; min-width:0; flex:1;",
                        div {
                            style: format!("font-size:13px; font-weight:700; letter-spacing:0.04em; text-transform:uppercase; color:{};", palette.muted),
                            "Conversation Overview"
                        }
                        div {
                            style: format!("font-size:15px; line-height:1.7; color:{}; max-width:720px; white-space:pre-wrap;", palette.text),
                            "{timeline_excerpt}"
                        }
                    }
                    div {
                        style: "display:flex; align-items:center; gap:10px; flex-wrap:wrap; justify-content:flex-end;",
                        OverviewStatCard {
                            label: "Messages".to_string(),
                            value: metadata_value(&session, "Messages"),
                            palette,
                        }
                        OverviewStatCard {
                            label: "Blocks".to_string(),
                            value: metadata_value(&session, "Preview Blocks"),
                            palette,
                        }
                        OverviewStatCard {
                            label: "Updated".to_string(),
                            value: metadata_value(&session, "Updated"),
                            palette,
                        }
                    }
                }
                if !summary_entries.is_empty() {
                    div {
                        style: "display:flex; gap:10px; flex-wrap:wrap;",
                        for entry in summary_entries {
                            div {
                                style: "display:flex; flex-direction:column; gap:4px; min-width:140px; max-width:240px; padding:12px 14px; border-radius:16px; background:rgba(255,255,255,0.78); box-shadow:inset 0 0 0 1px rgba(170,190,212,0.14);",
                                div {
                                    style: format!("font-size:11px; font-weight:700; letter-spacing:0.04em; text-transform:uppercase; color:{};", palette.muted),
                                    "{entry.label}"
                                }
                                div {
                                    style: format!("font-size:13px; line-height:1.5; color:{}; white-space:pre-wrap; overflow-wrap:anywhere;", palette.text),
                                    "{entry.value}"
                                }
                            }
                        }
                    }
                }
                if !session.rendered_sections.is_empty() {
                    RenderedSectionsStrip {
                        sections: session.rendered_sections.clone(),
                        palette,
                    }
                }
            }
            div {
                style: "display:flex; flex-direction:column; gap:14px; padding-top:2px;",
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
                            if let Some(excerpt) = preview_block_excerpt(block, 220) {
                                div {
                                    style: "font-size:13px; line-height:1.62; white-space:pre-wrap;",
                                    "{excerpt}"
                                }
                            }
                            if block.lines.len() > 2 {
                                div {
                                    style: format!("font-size:11px; color:{};", palette.muted),
                                    "+ {block.lines.len() - 2} more lines"
                                }
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
fn RenderedSectionsStrip(sections: Vec<SessionRenderedSection>, palette: Palette) -> Element {
    rsx! {
        div {
            style: "display:grid; grid-template-columns:repeat(auto-fit, minmax(220px, 1fr)); gap:12px;",
            for section in sections {
                RenderedSectionCard {
                    section,
                    palette,
                }
            }
        }
    }
}

#[component]
fn RenderedSectionCard(section: SessionRenderedSection, palette: Palette) -> Element {
    let is_commands = section.title.to_ascii_lowercase().contains("command");
    let visible_lines = section.lines.iter().take(3).cloned().collect::<Vec<_>>();
    let visible_count = visible_lines.len();
    rsx! {
        div {
            style: format!(
                "display:flex; flex-direction:column; gap:10px; min-width:0; padding:14px 16px; border-radius:18px; \
                 background:rgba(255,255,255,0.82); box-shadow:inset 0 0 0 1px rgba(170,190,212,0.14);"
            ),
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:12px;",
                div {
                    style: format!("font-size:12px; font-weight:700; letter-spacing:0.03em; text-transform:uppercase; color:{};", palette.muted),
                    "{section.title}"
                }
                if section.lines.len() > visible_count {
                    div {
                        style: format!("font-size:11px; color:{};", palette.muted),
                        "{section.lines.len()} lines"
                    }
                }
            }
            div {
                style: "display:flex; flex-direction:column; gap:8px; min-width:0;",
                for line in visible_lines {
                    if is_commands {
                        div {
                            style: format!(
                                "font-size:12px; line-height:1.6; white-space:pre-wrap; font-family:'JetBrains Mono', 'Iosevka Term', monospace; \
                                 color:{}; padding:8px 10px; border-radius:12px; background:rgba(15,23,42,0.06); overflow-wrap:anywhere;",
                                palette.text
                            ),
                            "{line}"
                        }
                    } else {
                        div {
                            style: format!("font-size:13px; line-height:1.62; white-space:pre-wrap; color:{};", palette.text),
                            "{line}"
                        }
                    }
                }
                if section.lines.len() > visible_count {
                    div {
                        style: format!("font-size:11px; font-weight:600; color:{};", palette.muted),
                        "+ {section.lines.len() - visible_count} more"
                    }
                }
            }
        }
    }
}

#[component]
fn OverviewStatCard(label: String, value: String, palette: Palette) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:4px; min-width:110px; padding:12px 14px; border-radius:16px; background:rgba(255,255,255,0.82); box-shadow:inset 0 0 0 1px rgba(170,190,212,0.14);",
            div {
                style: format!("font-size:11px; font-weight:700; letter-spacing:0.04em; text-transform:uppercase; color:{};", palette.muted),
                "{label}"
            }
            div {
                style: format!("font-size:14px; font-weight:700; color:{}; line-height:1.4;", palette.text),
                "{value}"
            }
        }
    }
}

fn preview_summary_text(session: &ManagedSessionView) -> String {
    let mut candidates = session
        .preview
        .summary
        .iter()
        .filter_map(|entry| {
            let value = entry.value.trim();
            if value.is_empty()
                || entry.label == "Session"
                || entry.label == "Storage"
                || entry.label == "Started"
                || entry.label == "Updated"
                || entry.label == "UUID"
                || entry.label == "Target"
                || entry.label == "Prefix"
                || entry.label == "Host"
                || entry.label == "Deploy"
            {
                None
            } else {
                Some(format!("{}: {}", entry.label, value))
            }
        })
        .collect::<Vec<_>>();

    if candidates.is_empty() {
        for block in &session.preview.blocks {
            let joined = block
                .lines
                .iter()
                .map(|line| line.trim())
                .filter(|line| !line.is_empty())
                .take(2)
                .collect::<Vec<_>>()
                .join(" ");
            if !joined.is_empty() {
                candidates.push(joined);
            }
            if candidates.len() >= 2 {
                break;
            }
        }
    }

    if candidates.is_empty() {
        format!(
            "{} · {} messages · {} blocks",
            session.host_label,
            metadata_value(session, "Messages"),
            session.preview.blocks.len()
        )
    } else {
        candidates
            .into_iter()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ")
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
    let user_block = block.tone == PreviewTone::User;
    let background = if user_block {
        "rgba(231, 243, 255, 0.98)"
    } else {
        "rgba(255, 255, 255, 0.96)"
    };
    let badge_background = if user_block {
        "rgba(37, 99, 235, 0.12)"
    } else {
        "rgba(15, 23, 42, 0.06)"
    };
    let badge = if user_block { palette.accent } else { palette.text };
    let outline = if user_block {
        "rgba(66, 153, 225, 0.18)"
    } else {
        "rgba(148, 163, 184, 0.14)"
    };
    let row_justify = if user_block { "flex-end" } else { "flex-start" };
    let card_width = if user_block { "min(76%, 760px)" } else { "min(92%, 900px)" };
    let avatar_bg = if user_block {
        "linear-gradient(180deg, rgba(73,138,255,0.18) 0%, rgba(73,138,255,0.10) 100%)"
    } else {
        "linear-gradient(180deg, rgba(255,255,255,0.96) 0%, rgba(248,252,255,0.88) 100%)"
    };
    let avatar_fg = if user_block { palette.accent } else { palette.text };
    let avatar_label = if user_block { "U" } else { "A" };

    rsx! {
        div {
            style: format!("display:flex; justify-content:{}; width:100%;", row_justify),
            div {
                style: format!("display:flex; align-items:flex-start; gap:12px; width:{};", card_width),
                if !user_block {
                    div {
                        style: format!(
                            "width:34px; height:34px; border-radius:999px; background:{}; color:{}; \
                             display:flex; align-items:center; justify-content:center; font-size:12px; font-weight:800; \
                             flex:0 0 auto; box-shadow: inset 0 0 0 1px rgba(170,190,212,0.18);",
                            avatar_bg,
                            avatar_fg
                        ),
                        "{avatar_label}"
                    }
                }
                button {
                    style: format!(
                        "width:100%; border:none; text-align:left; background:{}; border-radius:20px; \
                         padding:18px 20px; box-shadow: inset 0 0 0 1px {}, 0 16px 32px rgba(148,163,184,0.08);",
                        background, outline
                    ),
                    onclick: move |evt| on_toggle.call(evt),
                    div {
                        style: "display:flex; align-items:center; justify-content:space-between; gap:12px; margin-bottom:12px;",
                        div {
                            style: "display:flex; align-items:center; gap:8px;",
                            span {
                                style: format!(
                                    "display:inline-flex; align-items:center; justify-content:center; min-width:58px; height:23px; \
                                     border-radius:999px; background:{}; color:{}; font-size:11px; font-weight:700;",
                                    badge_background,
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
                        PreviewContent { lines: block.lines.clone(), palette }
                    }
                }
                if user_block {
                    div {
                        style: format!(
                            "width:34px; height:34px; border-radius:999px; background:{}; color:{}; \
                             display:flex; align-items:center; justify-content:center; font-size:12px; font-weight:800; \
                             flex:0 0 auto; box-shadow: inset 0 0 0 1px rgba(170,190,212,0.18);",
                            avatar_bg,
                            avatar_fg
                        ),
                        "{avatar_label}"
                    }
                }
            }
        }
    }
}

#[component]
fn PreviewContent(lines: Vec<String>, palette: Palette) -> Element {
    let blocks = preview_content_blocks(&lines);
    rsx! {
        div {
            style: format!("display:flex; flex-direction:column; gap:10px; color:{};", palette.text),
            for block in blocks {
                match block {
                    PreviewContentBlock::Heading { level, text } => rsx! {
                        div {
                            style: format!(
                                "font-size:{}px; line-height:1.38; font-weight:750; letter-spacing:-0.01em; color:{}; white-space:pre-wrap; padding-top:{}px;",
                                match level {
                                    1 => 18,
                                    2 => 16,
                                    _ => 14,
                                },
                                palette.text,
                                if level == 1 { 4 } else { 2 }
                            ),
                            "{text}"
                        }
                    },
                    PreviewContentBlock::Paragraph(text) => rsx! {
                        div {
                            style: "font-size:13px; line-height:1.66; white-space:pre-wrap;",
                            "{text}"
                        }
                    },
                    PreviewContentBlock::Bullet(text) => rsx! {
                        div {
                            style: "display:flex; align-items:flex-start; gap:10px;",
                            div {
                                style: format!("width:6px; height:6px; border-radius:999px; background:{}; margin-top:8px; flex:0 0 auto;", palette.accent_soft),
                            }
                            div {
                                style: "font-size:13px; line-height:1.66; white-space:pre-wrap;",
                                "{text}"
                            }
                        }
                    },
                    PreviewContentBlock::Numbered { number, text } => rsx! {
                        div {
                            style: "display:flex; align-items:flex-start; gap:10px;",
                            div {
                                style: format!("min-width:22px; color:{}; font-size:12px; font-weight:700; line-height:1.66; flex:0 0 auto;", palette.accent),
                                "{number}."
                            }
                            div {
                                style: "font-size:13px; line-height:1.66; white-space:pre-wrap;",
                                "{text}"
                            }
                        }
                    },
                    PreviewContentBlock::Task { done, text } => rsx! {
                        div {
                            style: "display:flex; align-items:flex-start; gap:10px;",
                            div {
                                style: format!(
                                    "width:16px; height:16px; border-radius:5px; margin-top:2px; flex:0 0 auto; \
                                     display:flex; align-items:center; justify-content:center; font-size:10px; font-weight:800; \
                                     color:{}; background:{}; box-shadow:inset 0 0 0 1px {};",
                                    if done { "white" } else { "transparent" },
                                    if done { palette.accent.to_string() } else { "rgba(255,255,255,0.82)".to_string() },
                                    if done { palette.accent.to_string() } else { "rgba(170,190,212,0.28)".to_string() }
                                ),
                                {if done { "✓" } else { "" }}
                            }
                            div {
                                style: format!(
                                    "font-size:13px; line-height:1.66; white-space:pre-wrap; color:{};",
                                    if done { palette.muted } else { palette.text }
                                ),
                                "{text}"
                            }
                        }
                    },
                    PreviewContentBlock::Quote(text) => rsx! {
                        div {
                            style: "display:flex; align-items:stretch; gap:12px; padding:2px 0;",
                            div {
                                style: format!("width:3px; border-radius:999px; background:{}; flex:0 0 auto;", palette.accent_soft),
                            }
                            div {
                                style: format!("font-size:13px; line-height:1.68; white-space:pre-wrap; color:{};", palette.muted),
                                "{text}"
                            }
                        }
                    },
                    PreviewContentBlock::Code { language, code } => rsx! {
                        div {
                            style: "display:flex; flex-direction:column; gap:8px; border-radius:16px; background:rgba(15,23,42,0.92); color:#e5eef8; overflow:hidden;",
                            div {
                                style: "display:flex; align-items:center; justify-content:space-between; gap:12px; padding:10px 14px; border-bottom:1px solid rgba(255,255,255,0.08);",
                                div {
                                    style: "font-size:11px; font-weight:700; letter-spacing:0.04em; text-transform:uppercase; color:rgba(226,232,240,0.78);",
                                    "{language.clone().unwrap_or_else(|| \"Code\".to_string())}"
                                }
                            }
                            pre {
                                style: "margin:0; padding:14px 16px 16px 16px; overflow:auto; white-space:pre-wrap; font-size:12px; line-height:1.68; font-family:'JetBrains Mono', 'Iosevka Term', monospace;",
                                code { "{code}" }
                            }
                        }
                    },
                }
            }
        }
    }
}

fn preview_content_blocks(lines: &[String]) -> Vec<PreviewContentBlock> {
    let mut blocks = Vec::new();
    let mut paragraph = Vec::<String>::new();
    let mut code = Vec::<String>::new();
    let mut code_language = None::<String>;
    let mut in_code = false;

    let flush_paragraph = |blocks: &mut Vec<PreviewContentBlock>, paragraph: &mut Vec<String>| {
        if paragraph.is_empty() {
            return;
        }
        let text = paragraph.join("\n").trim().to_string();
        if !text.is_empty() {
            blocks.push(PreviewContentBlock::Paragraph(text));
        }
        paragraph.clear();
    };

    for raw in lines {
        let line = raw.trim_end();
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if in_code {
                blocks.push(PreviewContentBlock::Code {
                    language: code_language.take(),
                    code: code.join("\n"),
                });
                code.clear();
                in_code = false;
            } else {
                flush_paragraph(&mut blocks, &mut paragraph);
                let language = trimmed.trim_start_matches('`').trim();
                code_language = (!language.is_empty()).then_some(language.to_string());
                in_code = true;
            }
            continue;
        }
        if in_code {
            code.push(line.to_string());
            continue;
        }
        if trimmed.is_empty() {
            flush_paragraph(&mut blocks, &mut paragraph);
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("### ") {
            flush_paragraph(&mut blocks, &mut paragraph);
            blocks.push(PreviewContentBlock::Heading {
                level: 3,
                text: heading.trim().to_string(),
            });
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("## ") {
            flush_paragraph(&mut blocks, &mut paragraph);
            blocks.push(PreviewContentBlock::Heading {
                level: 2,
                text: heading.trim().to_string(),
            });
            continue;
        }
        if let Some(heading) = trimmed.strip_prefix("# ") {
            flush_paragraph(&mut blocks, &mut paragraph);
            blocks.push(PreviewContentBlock::Heading {
                level: 1,
                text: heading.trim().to_string(),
            });
            continue;
        }
        if let Some(task) = trimmed
            .strip_prefix("- [ ] ")
            .or_else(|| trimmed.strip_prefix("* [ ] "))
        {
            flush_paragraph(&mut blocks, &mut paragraph);
            blocks.push(PreviewContentBlock::Task {
                done: false,
                text: task.trim().to_string(),
            });
            continue;
        }
        if let Some(task) = trimmed
            .strip_prefix("- [x] ")
            .or_else(|| trimmed.strip_prefix("* [x] "))
            .or_else(|| trimmed.strip_prefix("- [X] "))
            .or_else(|| trimmed.strip_prefix("* [X] "))
        {
            flush_paragraph(&mut blocks, &mut paragraph);
            blocks.push(PreviewContentBlock::Task {
                done: true,
                text: task.trim().to_string(),
            });
            continue;
        }
        if let Some(bullet) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            flush_paragraph(&mut blocks, &mut paragraph);
            blocks.push(PreviewContentBlock::Bullet(bullet.trim().to_string()));
            continue;
        }
        if let Some((number, item)) = parse_numbered_preview_item(trimmed) {
            flush_paragraph(&mut blocks, &mut paragraph);
            blocks.push(PreviewContentBlock::Numbered {
                number,
                text: item,
            });
            continue;
        }
        if let Some(quote) = trimmed.strip_prefix("> ") {
            flush_paragraph(&mut blocks, &mut paragraph);
            blocks.push(PreviewContentBlock::Quote(quote.trim().to_string()));
            continue;
        }
        paragraph.push(line.to_string());
    }

    if in_code {
        blocks.push(PreviewContentBlock::Code {
            language: code_language,
            code: code.join("\n"),
        });
    } else {
        flush_paragraph(&mut blocks, &mut paragraph);
    }

    blocks
}

fn parse_numbered_preview_item(line: &str) -> Option<(usize, String)> {
    let (number, text) = line.split_once(". ")?;
    if number.is_empty() || !number.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some((number.parse().ok()?, text.trim().to_string()))
}

fn preview_block_excerpt(block: &SessionPreviewBlock, max_chars: usize) -> Option<String> {
    let mut parts = Vec::new();
    for line in block.lines.iter().map(|line| line.trim()).filter(|line| !line.is_empty()) {
        if line.starts_with("```") {
            continue;
        }
        parts.push(line.to_string());
        let joined = parts.join(" ");
        if joined.len() >= max_chars {
            return Some(truncate_preview_excerpt(&joined, max_chars));
        }
        if parts.len() >= 3 {
            return Some(joined);
        }
    }
    (!parts.is_empty()).then(|| truncate_preview_excerpt(&parts.join(" "), max_chars))
}

fn truncate_preview_excerpt(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let excerpt = trimmed.chars().take(max_chars.saturating_sub(1)).collect::<String>();
    format!("{}…", excerpt.trim_end())
}

#[component]
fn TerminalCanvas(
    session: ManagedSessionView,
    snapshot: RenderSnapshot,
    state: Signal<ShellState>,
    mount_epoch: u64,
) -> Element {
    let endpoint = BOOTSTRAP
        .get()
        .expect("shell bootstrap initialized")
        .server_endpoint
        .clone();
    let session_path = session.session_path.clone();
    let host_id = terminal_host_id(&session_path);
    let instance_key = format!(
        "{}:{}",
        terminal_instance_key(&session_path, snapshot.settings.terminal_font_size),
        mount_epoch
    );
    let terminal_title = session.title.clone();
    let future_host_id = host_id.clone();
    let theme = terminal_theme(
        snapshot.settings.theme,
        snapshot.palette,
        snapshot.settings.terminal_font_size,
    );
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
        let mut state = state;
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
                black: theme.black.clone(),
                red: theme.red.clone(),
                green: theme.green.clone(),
                yellow: theme.yellow.clone(),
                blue: theme.blue.clone(),
                magenta: theme.magenta.clone(),
                cyan: theme.cyan.clone(),
                white: theme.white.clone(),
                bright_black: theme.bright_black.clone(),
                bright_red: theme.bright_red.clone(),
                bright_green: theme.bright_green.clone(),
                bright_yellow: theme.bright_yellow.clone(),
                bright_blue: theme.bright_blue.clone(),
                bright_magenta: theme.bright_magenta.clone(),
                bright_cyan: theme.bright_cyan.clone(),
                bright_white: theme.bright_white.clone(),
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
                            Ok(TerminalJsEvent::Clipboard { action, chars }) => {
                                let (title, message) = if action == "cut" {
                                    (
                                        "Cut to Clipboard",
                                        if chars == 0 {
                                            "Terminal selection moved to the clipboard.".to_string()
                                        } else {
                                            format!("Moved {chars} character(s) from the terminal selection.")
                                        },
                                    )
                                } else {
                                    (
                                        "Copied to Clipboard",
                                        if chars == 0 {
                                            "Terminal selection copied to the clipboard.".to_string()
                                        } else {
                                            format!("Copied {chars} character(s) from the terminal selection.")
                                        },
                                    )
                                };
                                safe_push_notification(
                                    state,
                                    NotificationTone::Success,
                                    title,
                                    message,
                                );
                            }
                            Ok(TerminalJsEvent::ClipboardImageRequest) => {
                                let should_handle = state.with_mut(|shell| {
                                    let now = current_millis();
                                    let last = shell
                                        .terminal_image_paste_ms
                                        .get(&session_path)
                                        .copied()
                                        .unwrap_or(0);
                                    if now.saturating_sub(last) < 1_500 {
                                        false
                                    } else {
                                        shell.terminal_image_paste_ms.insert(session_path.clone(), now);
                                        true
                                    }
                                });
                                if !should_handle {
                                    continue;
                                }
                                match read_native_clipboard_png()
                                    .and_then(|png_bytes| stage_terminal_clipboard_image(state, &session_path, &png_bytes))
                                {
                                    Ok(path) => {
                                        let _ = terminal_write(
                                            &endpoint,
                                            &session_path,
                                            &format!("{path} "),
                                        );
                                        safe_push_notification(
                                            state,
                                            NotificationTone::Success,
                                            "Image Staged",
                                            format!("Staged clipboard image at {path} and pasted its path into the terminal."),
                                        );
                                    }
                                    Err(error) => {
                                        let message = format!("Failed to paste image: {error}");
                                        let _ = terminal_write(&endpoint, &session_path, &format!("{message}\r\n"));
                                        safe_push_notification(
                                            state,
                                            NotificationTone::Error,
                                            "Image Paste Failed",
                                            error.to_string(),
                                        );
                                    }
                                }
                            }
                            Ok(TerminalJsEvent::ClipboardError { action, message }) => {
                                let title = if action == "cut" {
                                    "Cut Failed"
                                } else if action == "paste_image" {
                                    "Image Paste Failed"
                                } else {
                                    "Copy Failed"
                                };
                                safe_push_notification(
                                    state,
                                    NotificationTone::Error,
                                    title,
                                    message,
                                );
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
            key: "{instance_key}",
            style: "display:flex; flex-direction:column; min-height:0; height:100%;",
                    div {
                        style: format!(
                            "display:flex; flex-direction:column; min-height:0; height:100%; gap:0; border-radius:11px; \
                             background:{}; overflow:hidden; padding-left:6px;",
                            theme.background
                        ),
                div {
                    key: "{host_id}",
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
    black: String,
    red: String,
    green: String,
    yellow: String,
    blue: String,
    magenta: String,
    cyan: String,
    white: String,
    bright_black: String,
    bright_red: String,
    bright_green: String,
    bright_yellow: String,
    bright_blue: String,
    bright_magenta: String,
    bright_cyan: String,
    bright_white: String,
}

fn terminal_theme(ui_theme: UiTheme, palette: Palette, font_size: f32) -> TerminalTheme {
    match ui_theme {
        UiTheme::ZedLight => TerminalTheme {
            background: palette.panel.to_string(),
            foreground: palette.text.to_string(),
            cursor: palette.accent.to_string(),
            font_size: font_size.max(5.0),
            selection: "rgba(107,165,255,0.16)".to_string(),
            black: "#2f3a46".to_string(),
            red: "#d14d5a".to_string(),
            green: "#2f9d68".to_string(),
            yellow: "#b57f00".to_string(),
            blue: "#2f7cf6".to_string(),
            magenta: "#8b5cf6".to_string(),
            cyan: "#0f8fa6".to_string(),
            white: "#dbe4ec".to_string(),
            bright_black: "#697785".to_string(),
            bright_red: "#ea5f6c".to_string(),
            bright_green: "#44b57f".to_string(),
            bright_yellow: "#d89d18".to_string(),
            bright_blue: "#4a90ff".to_string(),
            bright_magenta: "#a77cff".to_string(),
            bright_cyan: "#2ab3cc".to_string(),
            bright_white: "#ffffff".to_string(),
        },
        UiTheme::ZedDark => TerminalTheme {
            background: "#1f2329".to_string(),
            foreground: "#abb2bf".to_string(),
            cursor: "#61afef".to_string(),
            font_size: font_size.max(5.0),
            selection: "rgba(97,175,239,0.24)".to_string(),
            black: "#282c34".to_string(),
            red: "#e06c75".to_string(),
            green: "#98c379".to_string(),
            yellow: "#e5c07b".to_string(),
            blue: "#61afef".to_string(),
            magenta: "#c678dd".to_string(),
            cyan: "#56b6c2".to_string(),
            white: "#dcdfe4".to_string(),
            bright_black: "#5c6370".to_string(),
            bright_red: "#e06c75".to_string(),
            bright_green: "#98c379".to_string(),
            bright_yellow: "#e5c07b".to_string(),
            bright_blue: "#61afef".to_string(),
            bright_magenta: "#c678dd".to_string(),
            bright_cyan: "#56b6c2".to_string(),
            bright_white: "#ffffff".to_string(),
        },
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

fn sidebar_row_dom_id(path: &str) -> String {
    let mut id = String::from("yggterm-sidebar-row-");
    for ch in path.chars() {
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
    let theme = terminal_theme(
        snapshot.settings.theme,
        snapshot.palette,
        snapshot.settings.terminal_font_size,
    );
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
    let black = serde_json::to_string(&theme.black).expect("serialize terminal black");
    let red = serde_json::to_string(&theme.red).expect("serialize terminal red");
    let green = serde_json::to_string(&theme.green).expect("serialize terminal green");
    let yellow = serde_json::to_string(&theme.yellow).expect("serialize terminal yellow");
    let blue = serde_json::to_string(&theme.blue).expect("serialize terminal blue");
    let magenta = serde_json::to_string(&theme.magenta).expect("serialize terminal magenta");
    let cyan = serde_json::to_string(&theme.cyan).expect("serialize terminal cyan");
    let white = serde_json::to_string(&theme.white).expect("serialize terminal white");
    let bright_black =
        serde_json::to_string(&theme.bright_black).expect("serialize terminal bright black");
    let bright_red =
        serde_json::to_string(&theme.bright_red).expect("serialize terminal bright red");
    let bright_green =
        serde_json::to_string(&theme.bright_green).expect("serialize terminal bright green");
    let bright_yellow =
        serde_json::to_string(&theme.bright_yellow).expect("serialize terminal bright yellow");
    let bright_blue =
        serde_json::to_string(&theme.bright_blue).expect("serialize terminal bright blue");
    let bright_magenta =
        serde_json::to_string(&theme.bright_magenta).expect("serialize terminal bright magenta");
    let bright_cyan =
        serde_json::to_string(&theme.bright_cyan).expect("serialize terminal bright cyan");
    let bright_white =
        serde_json::to_string(&theme.bright_white).expect("serialize terminal bright white");
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
                black: {black},
                red: {red},
                green: {green},
                yellow: {yellow},
                blue: {blue},
                magenta: {magenta},
                cyan: {cyan},
                white: {white},
                brightBlack: {bright_black},
                brightRed: {bright_red},
                brightGreen: {bright_green},
                brightYellow: {bright_yellow},
                brightBlue: {bright_blue},
                brightMagenta: {bright_magenta},
                brightCyan: {bright_cyan},
                brightWhite: {bright_white},
            }},
        }});
        const fitAddon = new window.FitAddon.FitAddon();
        term.loadAddon(fitAddon);
        term.open(host);
        term.attachCustomKeyEventHandler((event) => {{
            const accel = event.ctrlKey || event.metaKey;
            const key = (event.key || '').toLowerCase();
            if (!accel) {{
                return true;
            }}
            if (!event.shiftKey && key === 'v') {{
                dioxus.send({{ kind: "clipboard_image_request" }});
                return false;
            }}
            if (!event.shiftKey || (key !== 'c' && key !== 'x')) {{
                return true;
            }}
            const selection = term.getSelection ? term.getSelection() : "";
            if (!selection) {{
                dioxus.send({{
                    kind: "clipboard_error",
                    action: key === 'x' ? "cut" : "copy",
                    message: "Select terminal text before using the clipboard shortcut.",
                }});
                return false;
            }}
            const action = key === 'x' ? "cut" : "copy";
            const finish = () => {{
                try {{
                    if (action === "cut" && term.clearSelection) {{
                        term.clearSelection();
                    }}
                }} catch (_error) {{}}
                dioxus.send({{ kind: "clipboard", action, chars: selection.length }});
            }};
            const fail = (error) => {{
                dioxus.send({{
                    kind: "clipboard_error",
                    action,
                    message: error && error.message ? error.message : "Clipboard access was denied.",
                }});
            }};
            try {{
                if (navigator.clipboard && navigator.clipboard.writeText) {{
                    navigator.clipboard.writeText(selection).then(finish).catch(fail);
                }} else {{
                    fail(new Error("Clipboard API is unavailable."));
                }}
            }} catch (error) {{
                fail(error);
            }}
            return false;
        }});
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
                    black: message.black,
                    red: message.red,
                    green: message.green,
                    yellow: message.yellow,
                    blue: message.blue,
                    magenta: message.magenta,
                    cyan: message.cyan,
                    white: message.white,
                    brightBlack: message.bright_black,
                    brightRed: message.bright_red,
                    brightGreen: message.bright_green,
                    brightYellow: message.bright_yellow,
                    brightBlue: message.bright_blue,
                    brightMagenta: message.bright_magenta,
                    brightCyan: message.bright_cyan,
                    brightWhite: message.bright_white,
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
        black = black,
        red = red,
        green = green,
        yellow = yellow,
        blue = blue,
        magenta = magenta,
        cyan = cyan,
        white = white,
        bright_black = bright_black,
        bright_red = bright_red,
        bright_green = bright_green,
        bright_yellow = bright_yellow,
        bright_blue = bright_blue,
        bright_magenta = bright_magenta,
        bright_cyan = bright_cyan,
        bright_white = bright_white,
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
    let black = serde_json::to_string(&theme.black).expect("serialize terminal black");
    let red = serde_json::to_string(&theme.red).expect("serialize terminal red");
    let green = serde_json::to_string(&theme.green).expect("serialize terminal green");
    let yellow = serde_json::to_string(&theme.yellow).expect("serialize terminal yellow");
    let blue = serde_json::to_string(&theme.blue).expect("serialize terminal blue");
    let magenta = serde_json::to_string(&theme.magenta).expect("serialize terminal magenta");
    let cyan = serde_json::to_string(&theme.cyan).expect("serialize terminal cyan");
    let white = serde_json::to_string(&theme.white).expect("serialize terminal white");
    let bright_black =
        serde_json::to_string(&theme.bright_black).expect("serialize terminal bright black");
    let bright_red =
        serde_json::to_string(&theme.bright_red).expect("serialize terminal bright red");
    let bright_green =
        serde_json::to_string(&theme.bright_green).expect("serialize terminal bright green");
    let bright_yellow =
        serde_json::to_string(&theme.bright_yellow).expect("serialize terminal bright yellow");
    let bright_blue =
        serde_json::to_string(&theme.bright_blue).expect("serialize terminal bright blue");
    let bright_magenta =
        serde_json::to_string(&theme.bright_magenta).expect("serialize terminal bright magenta");
    let bright_cyan =
        serde_json::to_string(&theme.bright_cyan).expect("serialize terminal bright cyan");
    let bright_white =
        serde_json::to_string(&theme.bright_white).expect("serialize terminal bright white");
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
            black: {black},
            red: {red},
            green: {green},
            yellow: {yellow},
            blue: {blue},
            magenta: {magenta},
            cyan: {cyan},
            white: {white},
            brightBlack: {bright_black},
            brightRed: {bright_red},
            brightGreen: {bright_green},
            brightYellow: {bright_yellow},
            brightBlue: {bright_blue},
            brightMagenta: {bright_magenta},
            brightCyan: {bright_cyan},
            brightWhite: {bright_white},
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
        black = black,
        red = red,
        green = green,
        yellow = yellow,
        blue = blue,
        magenta = magenta,
        cyan = cyan,
        white = white,
        bright_black = bright_black,
        bright_red = bright_red,
        bright_green = bright_green,
        bright_yellow = bright_yellow,
        bright_blue = bright_blue,
        bright_magenta = bright_magenta,
        bright_cyan = bright_cyan,
        bright_white = bright_white,
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
    on_set_ui_theme: EventHandler<UiTheme>,
    on_open_theme_editor: EventHandler<MouseEvent>,
    on_set_notification_delivery: EventHandler<NotificationDeliveryMode>,
    on_set_notification_sound: EventHandler<bool>,
    on_adjust_ui_zoom: EventHandler<i32>,
    on_adjust_main_zoom: EventHandler<i32>,
    on_connect_ssh_custom: EventHandler<MouseEvent>,
    on_ssh_target_change: EventHandler<String>,
    on_ssh_prefix_change: EventHandler<String>,
    on_clear_notification: EventHandler<u64>,
    on_clear_notifications: EventHandler<MouseEvent>,
) -> Element {
    let visible = snapshot.right_panel_mode != RightPanelMode::Hidden;
    rsx! {
        SideRailShell {
            visible: visible,
            width_px: SIDE_RAIL_WIDTH,
            zoom_percent: zoom_percent_f32(snapshot.settings.ui_font_size, 14.0),
            body: rsx!{
            if snapshot.right_panel_mode == RightPanelMode::Metadata {
                MetadataRailBody { snapshot: snapshot.clone() }
            } else if snapshot.right_panel_mode == RightPanelMode::Settings {
                SettingsRailBody {
                    snapshot: snapshot.clone(),
                    on_endpoint_change,
                    on_api_key_change,
                    on_model_change,
                    on_set_ui_theme,
                    on_open_theme_editor,
                    on_set_notification_delivery,
                    on_set_notification_sound,
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
}

#[component]
fn MetadataRailBody(snapshot: RenderSnapshot) -> Element {
    let session = snapshot.active_session;

    rsx! {
        RailHeader { title: "Session Metadata".to_string(), color: snapshot.palette.text.to_string() }
        RailScrollBody {
            content: rsx!{
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
}

#[component]
fn SettingsRailBody(
    snapshot: RenderSnapshot,
    on_endpoint_change: EventHandler<String>,
    on_api_key_change: EventHandler<String>,
    on_model_change: EventHandler<String>,
    on_set_ui_theme: EventHandler<UiTheme>,
    on_open_theme_editor: EventHandler<MouseEvent>,
    on_set_notification_delivery: EventHandler<NotificationDeliveryMode>,
    on_set_notification_sound: EventHandler<bool>,
    on_adjust_ui_zoom: EventHandler<i32>,
    on_adjust_main_zoom: EventHandler<i32>,
) -> Element {
    rsx! {
        RailHeader { title: "Interface Settings".to_string(), color: snapshot.palette.text.to_string() }
        RailScrollBody {
            content: rsx!{
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
            ThemeSettingsSection {
                palette: snapshot.palette,
                selected_theme: snapshot.settings.theme,
                accent: snapshot.theme_accent.clone(),
                custom_stop_count: snapshot.settings.yggui_theme.colors.len(),
                on_select: on_set_ui_theme,
                on_open_editor: on_open_theme_editor,
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
                                .as_ref()
                                .map(|target| format!("{}:{:?}", target.path, target.placement))
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
        RailHeader { title: "Notifications".to_string(), color: snapshot.palette.text.to_string() }
        div {
            style: "padding:0 16px 8px 16px; display:flex; justify-content:flex-end;",
            button {
                style: chip_style(snapshot.palette, false),
                onclick: move |evt| on_clear_notifications.call(evt),
                "Clear All"
            }
        }
        RailScrollBody {
            content: rsx!{
            if snapshot.notifications.is_empty() {
                div {
                    style: format!("font-size:12px; line-height:1.5; color:{};", snapshot.palette.muted),
                    "No notifications yet."
                }
            } else {
                for notification in snapshot.notifications.iter().cloned().rev() {
                    ToastCard {
                        item: notification.clone(),
                        palette: ToastPalette {
                            text: snapshot.palette.text,
                            muted: snapshot.palette.muted,
                            accent: snapshot.palette.accent,
                        },
                        on_clear: move |_| on_clear_notification.call(notification.id),
                    }
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
        RailHeader { title: "Connect SSH".to_string(), color: snapshot.palette.text.to_string() }
        RailScrollBody {
            content: rsx!{
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
                        "font-size:11px; font-weight:700; letter-spacing:0.02em; color:{}; \
                         text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                        snapshot.palette.muted
                    ),
                    "Example"
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
                    placeholder: "dev or pi@raspberry or user@192.168.1.15",
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
}

#[component]
fn ContextMenuOverlay(
    row: BrowserRow,
    position: (f64, f64),
    window_size: (f64, f64),
    selected_row: Option<BrowserRow>,
    selected_tree_paths: Vec<String>,
    can_remove_saved_ssh_target: bool,
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
    on_refresh_remote_machine: EventHandler<MouseEvent>,
    on_remove_ssh_target: EventHandler<MouseEvent>,
    on_delete_item: EventHandler<MouseEvent>,
) -> Element {
    let placement = context_menu_placement(position, window_size, (224.0, 420.0));
    let placement_style = context_menu_position_style(placement);
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
                    "position:absolute; {}; min-width:188px; max-width:220px; max-height:calc(100vh - 24px); overflow:auto; padding:6px; border-radius:10px; \
                     background:rgba(248,249,252,0.98); box-shadow: 0 18px 38px rgba(57,78,98,0.18), inset 0 0 0 1px rgba(214,220,228,0.9); \
                     backdrop-filter: blur(20px) saturate(150%); -webkit-backdrop-filter: blur(20px) saturate(150%);",
                    placement_style
                ),
                onmousedown: |evt| evt.stop_propagation(),
                onclick: |evt| evt.stop_propagation(),
                div {
                    style: format!("padding:6px 12px 8px 12px; font-size:11px; font-weight:700; color:{}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;", palette.muted),
                    "{menu_title}"
                }
                if is_remote_machine_group_row(&row) {
                    button {
                        style: context_menu_action_style(palette, false),
                        onclick: move |evt| on_refresh_remote_machine.call(evt),
                        "Refresh Remote Sessions"
                    }
                    if can_remove_saved_ssh_target {
                        div {
                            style: format!("height:1px; margin:6px 4px; background:{}; opacity:0.7;", palette.border),
                        }
                        button {
                            style: context_menu_action_style_destructive(palette),
                            onclick: move |evt| on_remove_ssh_target.call(evt),
                            "Delete…"
                        }
                    }
                } else if can_create_in_context {
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
                if is_local_stored_session_row(&row) {
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
    let item_count =
        pending.document_paths.len() + pending.group_paths.len() + pending.ssh_machine_keys.len();
    let preview = pending.labels.iter().take(4).cloned().collect::<Vec<_>>();
    let deleting_ssh_targets =
        !pending.ssh_machine_keys.is_empty() && pending.document_paths.is_empty() && pending.group_paths.is_empty();
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
                            if deleting_ssh_targets {
                                "This will permanently remove the selected SSH targets from the sidebar."
                            } else {
                                "This will permanently remove the selected items from the workspace tree."
                            }
                        } else {
                            if deleting_ssh_targets {
                                "This will remove the selected SSH targets from the sidebar. Hold Shift while pressing Delete to skip this dialog."
                            } else {
                                "This will remove the selected items from the workspace tree. Hold Shift while pressing Delete to skip this dialog."
                            }
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
fn ThemeEditorOverlay(
    snapshot: RenderSnapshot,
    on_close: EventHandler<MouseEvent>,
    on_save: EventHandler<MouseEvent>,
    on_reset: EventHandler<MouseEvent>,
    on_seed: EventHandler<MouseEvent>,
    on_set_ui_theme: EventHandler<UiTheme>,
    on_add_stop: EventHandler<MouseEvent>,
    on_remove_stop: EventHandler<MouseEvent>,
    on_pick_stop: EventHandler<usize>,
    on_begin_drag_stop: EventHandler<usize>,
    on_drag_stop: EventHandler<(f32, f32)>,
    on_end_drag_stop: EventHandler<MouseEvent>,
    on_double_click_pad: EventHandler<(f32, f32)>,
    on_update_stop_color: EventHandler<String>,
    on_pick_swatch: EventHandler<String>,
    on_set_brightness: EventHandler<f32>,
    on_set_grain: EventHandler<f32>,
) -> Element {
    let selected_stop = snapshot
        .theme_editor_selected_stop
        .and_then(|index| snapshot.theme_editor_draft.colors.get(index).cloned());
    let preview_surface = preview_surface_css(snapshot.settings.theme, &snapshot.theme_editor_draft);
    let brightness_percent = (snapshot.theme_editor_draft.brightness * 100.0).round() as i32;
    let grain_percent = (snapshot.theme_editor_draft.grain * 100.0).round() as i32;
    let accent = snapshot.theme_accent.clone();
    let preview_has_stops = !snapshot.theme_editor_draft.colors.is_empty();
    let overlay_wash = match snapshot.settings.theme {
        UiTheme::ZedLight => "rgba(228,237,245,0.16)",
        UiTheme::ZedDark => "rgba(10,14,18,0.18)",
    };
    let editor_surface = match snapshot.settings.theme {
        UiTheme::ZedLight => "rgba(250,252,255,0.96)",
        UiTheme::ZedDark => "rgba(28,34,41,0.96)",
    };
    let editor_shadow = match snapshot.settings.theme {
        UiTheme::ZedLight => "0 0 0 1px rgba(215,229,243,0.96), 0 0 0 10px rgba(129,188,255,0.18), 0 26px 60px rgba(55,83,112,0.20), inset 0 0 0 1px rgba(214,223,232,0.92)",
        UiTheme::ZedDark => "0 0 0 1px rgba(59,87,112,0.90), 0 0 0 10px rgba(124,200,255,0.16), 0 26px 60px rgba(0,0,0,0.42), inset 0 0 0 1px rgba(68,84,99,0.94)",
    };

    rsx! {
        div {
            style: format!(
                "position:fixed; inset:0; z-index:98; display:flex; align-items:center; justify-content:center; background:{};",
                overlay_wash
            ),
            onclick: move |evt| on_close.call(evt),
            div {
                style: format!(
                    "width:min(460px, calc(100vw - 44px)); display:flex; flex-direction:column; gap:14px; padding:14px; \
                     border-radius:22px; background:{}; color:{}; \
                     box-shadow:{}; \
                     font-family:{};",
                    editor_surface,
                    snapshot.palette.text,
                    editor_shadow,
                    interface_font_family()
                ),
                onmousedown: |evt| evt.stop_propagation(),
                onclick: |evt| evt.stop_propagation(),
                div {
                    style: "display:flex; align-items:center; justify-content:space-between; gap:12px;",
                    div {
                        style: "display:flex; align-items:center; gap:8px;",
                        div {
                            style: format!(
                                "width:11px; height:11px; border-radius:999px; background:{}; box-shadow:0 0 0 4px rgba(128,175,212,0.12);",
                                accent
                            ),
                        }
                        div {
                            style: "display:flex; flex-direction:column; gap:3px;",
                            div {
                                style: format!("font-size:15px; font-weight:800; letter-spacing:-0.01em; color:{};", snapshot.palette.text),
                                "Edit Theme"
                            }
                            div {
                                style: format!("font-size:11px; line-height:1.45; color:{};", snapshot.palette.muted),
                                "Shape the shell gradient, brightness, and grain for Yggui. The active theme is saved in ~/.yggterm/settings.json under the theme object."
                            }
                        }
                    }
                    div {
                        style: "display:flex; align-items:center; gap:8px;",
                        div {
                            style: format!(
                                "display:flex; align-items:center; gap:4px; padding:4px; border-radius:999px; \
                                 background:rgba(236,242,247,0.92); box-shadow: inset 0 0 0 1px rgba(210,221,232,0.9);"
                            ),
                            button {
                                style: segmented_pill_button_style(snapshot.palette, snapshot.settings.theme == UiTheme::ZedLight),
                                onclick: move |_| on_set_ui_theme.call(UiTheme::ZedLight),
                                "Light"
                            }
                            button {
                                style: segmented_pill_button_style(snapshot.palette, snapshot.settings.theme == UiTheme::ZedDark),
                                onclick: move |_| on_set_ui_theme.call(UiTheme::ZedDark),
                                "Dark"
                            }
                        }
                        button {
                            style: icon_button_style(snapshot.palette),
                            onclick: move |evt| on_close.call(evt),
                            "✕"
                        }
                    }
                }
                div {
                    style: "display:flex; gap:14px; align-items:stretch;",
                    div {
                        style: format!(
                            "width:{}px; min-width:{}px; display:flex; flex-direction:column; gap:10px;",
                            THEME_EDITOR_PAD_SIZE as i32,
                            THEME_EDITOR_PAD_SIZE as i32
                        ),
                        div {
                            style: format!(
                                "position:relative; width:{}px; min-width:{}px; height:{}px; border-radius:20px; overflow:hidden; \
                                 background:{}; box-shadow: inset 0 0 0 1px rgba(255,255,255,0.56), 0 18px 38px rgba(84,113,137,0.12);",
                                THEME_EDITOR_PAD_SIZE as i32,
                                THEME_EDITOR_PAD_SIZE as i32,
                                THEME_EDITOR_PAD_SIZE as i32,
                                preview_surface
                            ),
                            onmousemove: move |evt| {
                                let point = evt.element_coordinates();
                                on_drag_stop.call((
                                    normalize_theme_editor_axis(point.x),
                                    normalize_theme_editor_axis(point.y),
                                ));
                            },
                            onmouseup: move |evt| on_end_drag_stop.call(evt),
                            ondoubleclick: move |evt| {
                                let point = evt.element_coordinates();
                                on_double_click_pad.call((
                                    normalize_theme_editor_axis(point.x),
                                    normalize_theme_editor_axis(point.y),
                                ));
                            },
                            div {
                                style: "position:absolute; inset:0; background-image: linear-gradient(rgba(144,173,199,0.18) 1px, transparent 1px), linear-gradient(90deg, rgba(144,173,199,0.18) 1px, transparent 1px); background-size: 24px 24px; opacity:0.78; pointer-events:none;",
                            }
                            div {
                                style: "position:absolute; inset:0; background-image: linear-gradient(rgba(255,255,255,0.24) 1px, transparent 1px), linear-gradient(90deg, rgba(255,255,255,0.24) 1px, transparent 1px); background-size: 96px 96px; opacity:0.52; pointer-events:none;",
                            }
                            if !preview_has_stops {
                                div {
                                    style: format!(
                                        "position:absolute; inset:0; display:flex; align-items:center; justify-content:center; padding:18px; \
                                         text-align:center; font-size:12px; font-weight:700; line-height:1.6; color:{};",
                                        snapshot.palette.text
                                    ),
                                    "Double-click to add a color"
                                }
                            }
                            for (index, stop) in snapshot.theme_editor_draft.colors.iter().enumerate() {
                                button {
                                    key: "theme-stop-{index}",
                                    style: format!(
                                        "position:absolute; left:calc({:.2}% - 11px); top:calc({:.2}% - 11px); width:22px; height:22px; \
                                         border-radius:999px; border:{}; background:{}; box-shadow:0 10px 22px rgba(42,67,88,0.16);",
                                        stop.x * 100.0,
                                        stop.y * 100.0,
                                        if snapshot.theme_editor_selected_stop == Some(index) {
                                            format!("3px solid {}", accent)
                                        } else {
                                            "2px solid rgba(255,255,255,0.86)".to_string()
                                        },
                                        stop.color
                                    ),
                                    onmousedown: move |evt| {
                                        evt.stop_propagation();
                                        on_begin_drag_stop.call(index);
                                    },
                                    onclick: move |_| on_pick_stop.call(index),
                                }
                            }
                        }
                        div {
                            style: "display:flex; align-items:center; justify-content:space-between; gap:12px; padding:8px 10px; border-radius:14px; background:rgba(247,250,253,0.9); box-shadow:inset 0 0 0 1px rgba(214,223,232,0.92);",
                            div {
                                style: "display:flex; flex-direction:column; gap:3px;",
                                div {
                                    style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", snapshot.palette.muted),
                                    "Gradient Stops"
                                }
                                div {
                                    style: format!("font-size:11px; color:{};", snapshot.palette.muted),
                                    if snapshot.theme_editor_draft.colors.is_empty() {
                                        "Start with one color, then add more only when the gradient needs them."
                                    } else {
                                        "{snapshot.theme_editor_draft.colors.len()} stop(s) in this gradient"
                                    }
                                }
                            }
                            div {
                                style: "display:flex; align-items:center; gap:8px;",
                                ThemeDialogButton {
                                    primary: false,
                                    enabled: true,
                                    palette: snapshot.palette,
                                    accent: accent.clone(),
                                    onclick: move |evt| on_add_stop.call(evt),
                                    prefix: Some("+".to_string()),
                                    "Add Stop"
                                }
                                ThemeDialogButton {
                                    primary: false,
                                    enabled: snapshot.theme_editor_selected_stop.is_some(),
                                    palette: snapshot.palette,
                                    accent: accent.clone(),
                                    onclick: move |evt| on_remove_stop.call(evt),
                                    prefix: Some("−".to_string()),
                                    "Remove"
                                }
                            }
                        }
                    }
                    div {
                        style: "flex:1; display:flex; flex-direction:column; gap:12px; min-width:0;",
                        div {
                            style: "display:flex; flex-wrap:wrap; gap:8px;",
                            for (index, stop) in snapshot.theme_editor_draft.colors.iter().enumerate() {
                                button {
                                    key: "theme-chip-{index}",
                                    style: format!(
                                        "display:flex; align-items:center; gap:8px; height:32px; padding:0 10px; border:none; border-radius:999px; \
                                         background:{}; color:{}; box-shadow:{};",
                                        if snapshot.theme_editor_selected_stop == Some(index) {
                                            "rgba(255,255,255,0.96)"
                                        } else {
                                            "rgba(246,249,252,0.84)"
                                        },
                                        snapshot.palette.text,
                                        if snapshot.theme_editor_selected_stop == Some(index) {
                                            format!("inset 0 0 0 2px {}", accent)
                                        } else {
                                            "inset 0 0 0 1px rgba(214,223,232,0.92)".to_string()
                                        }
                                    ),
                                    onclick: move |_| on_pick_stop.call(index),
                                    span {
                                        style: format!("width:14px; height:14px; border-radius:999px; background:{}; box-shadow: inset 0 0 0 1px rgba(255,255,255,0.88);", stop.color),
                                    }
                                    span {
                                        style: format!("font-size:11px; font-weight:700; color:{};", snapshot.palette.text),
                                        "Color {index + 1}"
                                    }
                                }
                            }
                        }
                        div {
                            style: "display:flex; flex-direction:column; gap:8px;",
                            div {
                                style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", snapshot.palette.muted),
                                "Color Library"
                            }
                            div {
                                style: "display:flex; flex-wrap:wrap; gap:8px;",
                                for swatch in THEME_EDITOR_SWATCHES {
                                    button {
                                        key: "theme-swatch-{swatch}",
                                        style: format!(
                                            "width:24px; height:24px; border-radius:999px; border:2px solid rgba(255,255,255,0.92); background:{}; box-shadow:0 8px 16px rgba(45,67,88,0.12);",
                                            swatch
                                        ),
                                        onclick: move |_| on_pick_swatch.call(swatch.to_string()),
                                    }
                                }
                            }
                        }
                        div {
                            style: "display:flex; flex-direction:column; gap:8px;",
                            div {
                                style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", snapshot.palette.muted),
                                "Selected Color"
                            }
                            input {
                                r#type: "color",
                                value: selected_stop.as_ref().map(|stop| stop.color.clone()).unwrap_or_else(|| accent.clone()),
                                style: "width:100%; height:42px; border:none; border-radius:12px; background:transparent;",
                                oninput: move |evt| on_update_stop_color.call(evt.value()),
                            }
                        }
                        div {
                            style: "display:flex; flex-direction:column; gap:8px;",
                            div {
                                style: "display:flex; align-items:center; justify-content:space-between; gap:10px;",
                                div {
                                    style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", snapshot.palette.muted),
                                    "Brightness"
                                }
                                div {
                                    style: format!("font-size:11px; font-weight:700; color:{};", accent),
                                    "{brightness_percent}"
                                }
                            }
                            div {
                                style: "position:relative; display:flex; align-items:center; height:34px;",
                                div {
                                    style: "position:absolute; inset:9px 0 9px 0; border-radius:999px; background:linear-gradient(90deg, rgba(213,224,235,0.58) 0%, rgba(255,255,255,0.92) 50%, rgba(203,227,214,0.66) 100%);",
                                }
                                div {
                                    style: "position:absolute; left:0; right:0; top:50%; height:14px; transform:translateY(-50%); background:transparent; pointer-events:none;",
                                    svg {
                                        width: "100%",
                                        height: "14",
                                        view_box: "0 0 320 14",
                                        path {
                                            d: "M0 7 C 20 -1, 40 -1, 60 7 S 100 15, 120 7 S 160 -1, 180 7 S 220 15, 240 7 S 280 -1, 300 7 S 320 15, 340 7",
                                            fill: "none",
                                            stroke: "rgba(132,156,180,0.38)",
                                            stroke_width: "2",
                                        }
                                    }
                                }
                                input {
                                    r#type: "range",
                                    min: "0",
                                    max: "100",
                                    value: "{brightness_percent}",
                                    style: "position:relative; z-index:1; width:100%; height:34px; appearance:none; background:transparent;",
                                    oninput: move |evt| {
                                        let value = evt.value().parse::<f32>().unwrap_or(56.0) / 100.0;
                                        on_set_brightness.call(value);
                                    },
                                }
                            }
                        }
                        div {
                            style: "display:flex; flex-direction:column; gap:8px; align-items:center;",
                            div {
                                style: "display:flex; align-items:center; justify-content:space-between; gap:10px; width:100%;",
                                div {
                                    style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", snapshot.palette.muted),
                                    "Grain"
                                }
                                div {
                                    style: format!("font-size:11px; font-weight:700; color:{};", accent),
                                    "{grain_percent}"
                                }
                            }
                            div {
                                style: format!(
                                    "position:relative; width:92px; height:92px; border-radius:999px; background:conic-gradient({} 0deg, {} {:.1}deg, rgba(224,232,240,0.92) {:.1}deg 360deg); box-shadow: inset 0 0 0 1px rgba(214,223,232,0.92);",
                                    accent,
                                    accent,
                                    snapshot.theme_editor_draft.grain.clamp(0.0, 1.0) * 360.0,
                                    snapshot.theme_editor_draft.grain.clamp(0.0, 1.0) * 360.0
                                ),
                                input {
                                    r#type: "range",
                                    min: "0",
                                    max: "100",
                                    value: "{grain_percent}",
                                    style: "position:absolute; inset:0; width:100%; height:100%; opacity:0; cursor:pointer;",
                                    oninput: move |evt| {
                                        let value = evt.value().parse::<f32>().unwrap_or(12.0) / 100.0;
                                        on_set_grain.call(value);
                                    },
                                }
                                div {
                                    style: "position:absolute; inset:14px; border-radius:999px; background:rgba(250,252,255,0.94); box-shadow: inset 0 0 0 1px rgba(220,228,236,0.9); display:flex; flex-direction:column; align-items:center; justify-content:center; gap:2px;",
                                    span {
                                        style: format!("font-size:11px; font-weight:800; color:{};", snapshot.palette.text),
                                        "{grain_percent}"
                                    }
                                    span {
                                        style: format!("font-size:9px; font-weight:700; letter-spacing:0.04em; text-transform:uppercase; color:{};", snapshot.palette.muted),
                                        "grain"
                                    }
                                }
                            }
                        }
                    }
                }
                div {
                    style: "display:flex; align-items:center; justify-content:space-between; gap:10px;",
                    div {
                        style: format!("font-size:11px; line-height:1.5; color:{};", snapshot.palette.muted),
                        if preview_has_stops {
                            "Double-click the pad to add another color, drag the dots to reshape the gradient, then save."
                        } else {
                            "Start empty or use the starter palette, then drag colors until the shell feels right."
                        }
                    }
                    div {
                        style: "display:flex; align-items:center; gap:8px;",
                        if !preview_has_stops {
                            ThemeDialogButton {
                                primary: false,
                                enabled: true,
                                palette: snapshot.palette,
                                accent: accent.clone(),
                                onclick: move |evt| on_seed.call(evt),
                                prefix: None,
                                "Use Starter"
                            }
                        }
                        ThemeDialogButton {
                            primary: false,
                            enabled: true,
                            palette: snapshot.palette,
                            accent: accent.clone(),
                            onclick: move |evt| on_reset.call(evt),
                            prefix: None,
                            "Reset to Base"
                        }
                        ThemeDialogButton {
                            primary: true,
                            enabled: true,
                            palette: snapshot.palette,
                            accent: accent.clone(),
                            onclick: move |evt| on_save.call(evt),
                            prefix: None,
                            "Apply Theme"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn ThemeDialogButton(
    primary: bool,
    enabled: bool,
    palette: Palette,
    accent: String,
    prefix: Option<String>,
    onclick: EventHandler<MouseEvent>,
    children: Element,
) -> Element {
    let style = if primary {
        primary_action_style(palette)
    } else {
        theme_editor_action_button_style(palette, &accent, enabled, false)
    };
    rsx! {
        button {
            disabled: !enabled,
            style: style,
            onclick: move |evt| {
                if enabled {
                    onclick.call(evt);
                }
            },
            if let Some(prefix) = prefix {
                span {
                    style: format!(
                        "font-size:14px; font-weight:800; color:{};",
                        if enabled { accent.clone() } else { palette.muted.to_string() }
                    ),
                    "{prefix}"
                }
            }
            {children}
        }
    }
}

fn toast_center_offset(right_panel_mode: RightPanelMode) -> i32 {
    if right_panel_mode == RightPanelMode::Hidden {
        0
    } else {
        -146
    }
}

fn normalize_theme_editor_axis(value: f64) -> f32 {
    ((value / THEME_EDITOR_PAD_SIZE).clamp(0.0, 1.0)) as f32
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
fn ThemeSettingsSection(
    palette: Palette,
    selected_theme: UiTheme,
    accent: String,
    custom_stop_count: usize,
    on_select: EventHandler<UiTheme>,
    on_open_editor: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:8px;",
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:8px;",
                div {
                    style: format!("font-size:11px; font-weight:700; letter-spacing:0.02em; color:{};", palette.muted),
                    "Theme"
                }
                span {
                    style: format!("font-size:10px; font-weight:700; color:{};", accent),
                    if custom_stop_count == 0 { "System Gradient" } else { "Custom Gradient" }
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
                        style: segmented_pill_button_style(palette, selected_theme == UiTheme::ZedLight),
                        onclick: move |_| on_select.call(UiTheme::ZedLight),
                        "Light"
                    }
                    button {
                        style: segmented_pill_button_style(palette, selected_theme == UiTheme::ZedDark),
                        onclick: move |_| on_select.call(UiTheme::ZedDark),
                        "Dark"
                    }
                }
                button {
                    style: format!(
                        "display:flex; align-items:center; justify-content:space-between; height:34px; padding:0 12px; \
                         border:none; border-radius:11px; background:rgba(255,255,255,0.86); color:{}; \
                         box-shadow: inset 0 0 0 1px rgba(208,219,229,0.85); font-size:12px; font-weight:700;",
                        palette.text
                    ),
                    onclick: move |evt| on_open_editor.call(evt),
                    span { "Edit Theme" }
                    span { style: format!("color:{};", accent), "↗" }
                }
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
            RailSectionTitle { title: title, muted_color: palette.muted.to_string() }
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
            sidebar_hover: "rgba(124,200,255,0.12)",
            panel: "#161c22",
            panel_alt: "rgba(22,28,34,0.76)",
            border: "#2d3946",
            text: "#dde8f3",
            muted: "#a2b4c4",
            accent: "#7cc8ff",
            accent_soft: "rgba(124,200,255,0.16)",
            gradient: "linear-gradient(180deg, rgba(56,79,91,0.94) 0%, rgba(55,88,79,0.90) 54%, rgba(39,46,52,0.96) 100%)",
            close_hover: "#e81123",
            control_hover: "rgba(255,255,255,0.10)",
            shadow: "0 26px 72px rgba(0,0,0,0.38)",
            panel_shadow: "0 22px 52px rgba(0,0,0,0.28)",
        },
    }
}

fn shell_style(palette: Palette, radius: u8, shell_tint: &str, shell_gradient: &str) -> String {
    format!(
        "position:fixed; inset:0; display:flex; flex-direction:column; overflow:hidden; \
         border-radius:{}px; background-color:{}; background-image:{}; box-shadow:{}; backdrop-filter: blur(30px) saturate(165%); \
         -webkit-backdrop-filter: blur(30px) saturate(165%); font-family:{};",
        radius,
        shell_tint,
        shell_gradient,
        palette.shadow,
        interface_font_family()
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

fn theme_editor_action_button_style(
    palette: Palette,
    accent: &str,
    enabled: bool,
    primary: bool,
) -> String {
    format!(
        "display:flex; align-items:center; gap:7px; height:30px; padding:0 11px; border:none; border-radius:11px; \
         background:{}; color:{}; font-size:11px; font-weight:700; box-shadow:{}; opacity:{};",
        if primary {
            "rgba(255,255,255,0.98)"
        } else if enabled {
            "rgba(235,242,248,0.96)"
        } else {
            "rgba(244,247,250,0.84)"
        },
        if primary {
            accent
        } else if enabled {
            palette.text
        } else {
            palette.muted
        },
        if primary {
            format!("inset 0 0 0 1px rgba(214,223,232,0.92), 0 8px 18px rgba(81,113,138,0.08)")
        } else if enabled {
            format!("inset 0 0 0 1px rgba(214,223,232,0.92)")
        } else {
            "inset 0 0 0 1px rgba(224,231,238,0.9)".to_string()
        },
        if enabled || primary { "1" } else { "0.72" },
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
     font-size:13px; font-weight:500;"
        .to_string()
}

fn delete_confirm_button_style(_palette: Palette, hard_delete: bool) -> String {
    format!(
        "height:34px; padding:0 16px; border:none; border-radius:12px; background:{}; color:#ffffff; \
         font-size:13px; font-weight:500;",
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
        let theme = terminal_theme(UiTheme::ZedLight, palette(UiTheme::ZedLight), 5.0);
        assert_eq!(theme.font_size, 5.0);

        let clamped = terminal_theme(UiTheme::ZedLight, palette(UiTheme::ZedLight), 2.0);
        assert_eq!(clamped.font_size, 5.0);
    }

    #[test]
    fn terminal_eval_script_bakes_font_size_into_xterm_constructor() {
        let theme = terminal_theme(UiTheme::ZedLight, palette(UiTheme::ZedLight), 13.0);
        let script = terminal_eval_script("yggterm-terminal-test", &theme);
        assert!(script.contains("fontSize: 13"));
        assert!(script.contains("brightWhite"));
    }

    #[test]
    fn terminal_apply_script_updates_live_xterm_font_size() {
        let theme = terminal_theme(UiTheme::ZedLight, palette(UiTheme::ZedLight), 5.0);
        let script = terminal_apply_script("yggterm-terminal-test", &theme);
        assert!(script.contains("entry.term.options.fontSize = 5"));
        assert!(script.contains("brightBlack"));
    }

    #[test]
    fn terminal_theme_uses_one_dark_palette_for_dark_mode() {
        let theme = terminal_theme(UiTheme::ZedDark, palette(UiTheme::ZedDark), 13.0);
        assert_eq!(theme.background, "#1f2329");
        assert_eq!(theme.foreground, "#abb2bf");
        assert_eq!(theme.blue, "#61afef");
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

    #[test]
    fn document_rows_are_valid_drop_targets_for_reorder() {
        let target = BrowserRow {
            kind: BrowserRowKind::Document,
            full_path: "/home/pi/gh/notes/paper-b".to_string(),
            label: "paper-b".to_string(),
            detail_label: String::new(),
            document_kind: Some(WorkspaceDocumentKind::Note),
            group_kind: None,
            session_title: None,
            depth: 3,
            host_label: String::new(),
            descendant_sessions: 0,
            expanded: false,
            session_id: Some("paper-b-id".to_string()),
            session_cwd: Some("/home/pi/gh/notes".to_string()),
        };
        assert!(valid_drop_target(
            &["/home/pi/gh/notes/paper-a".to_string()],
            &target
        ));
    }

    #[test]
    fn ordered_workspace_item_path_uses_flat_index_prefix() {
        let item = BrowserRow {
            kind: BrowserRowKind::Document,
            full_path: "/home/pi/gh/notes/paper-a".to_string(),
            label: "paper-a".to_string(),
            detail_label: String::new(),
            document_kind: Some(WorkspaceDocumentKind::Note),
            group_kind: None,
            session_title: None,
            depth: 3,
            host_label: String::new(),
            descendant_sessions: 0,
            expanded: false,
            session_id: Some("paper-a-id".to_string()),
            session_cwd: Some("/home/pi/gh/notes".to_string()),
        };
        let destination =
            crate::drag_tree::ordered_tree_child_path("/home/pi/gh/notes", &item.full_path, 0);

        assert_eq!(destination, "/home/pi/gh/notes/0000-paper-a");
    }

    #[test]
    fn before_target_resolves_after_previous_sibling() {
        let rows = vec![
            BrowserRow {
                kind: BrowserRowKind::Document,
                full_path: "/home/pi/gh/notes/paper-a".to_string(),
                label: "paper-a".to_string(),
                detail_label: String::new(),
                document_kind: Some(WorkspaceDocumentKind::Note),
                group_kind: None,
                session_title: None,
                depth: 3,
                host_label: String::new(),
                descendant_sessions: 0,
                expanded: false,
                session_id: Some("paper-a-id".to_string()),
                session_cwd: Some("/home/pi/gh/notes".to_string()),
            },
            BrowserRow {
                kind: BrowserRowKind::Document,
                full_path: "/home/pi/gh/notes/paper-b".to_string(),
                label: "paper-b".to_string(),
                detail_label: String::new(),
                document_kind: Some(WorkspaceDocumentKind::Note),
                group_kind: None,
                session_title: None,
                depth: 3,
                host_label: String::new(),
                descendant_sessions: 0,
                expanded: false,
                session_id: Some("paper-b-id".to_string()),
                session_cwd: Some("/home/pi/gh/notes".to_string()),
            },
        ];
        let placement = resolve_workspace_drop_placement(
            &rows,
            &DragDropTarget {
                path: "/home/pi/gh/notes/paper-b".to_string(),
                placement: DragDropPlacement::Before,
            },
        );
        assert_eq!(
            placement,
            Some(WorkspaceDropPlacement::AfterPath(
                "/home/pi/gh/notes/paper-a".to_string()
            ))
        );
    }

    #[test]
    fn before_first_target_resolves_to_top_of_parent() {
        let rows = vec![BrowserRow {
            kind: BrowserRowKind::Document,
            full_path: "/home/pi/gh/notes/paper-a".to_string(),
            label: "paper-a".to_string(),
            detail_label: String::new(),
            document_kind: Some(WorkspaceDocumentKind::Note),
            group_kind: None,
            session_title: None,
            depth: 3,
            host_label: String::new(),
            descendant_sessions: 0,
            expanded: false,
            session_id: Some("paper-a-id".to_string()),
            session_cwd: Some("/home/pi/gh/notes".to_string()),
        }];
        let placement = resolve_workspace_drop_placement(
            &rows,
            &DragDropTarget {
                path: "/home/pi/gh/notes/paper-a".to_string(),
                placement: DragDropPlacement::Before,
            },
        );
        assert_eq!(
            placement,
            Some(WorkspaceDropPlacement::TopOfGroup(
                "/home/pi/gh/notes".to_string()
            ))
        );
    }

    #[test]
    fn reorder_plan_keeps_position_when_anchor_is_dragged_row_boundary() {
        let gg = BrowserRow {
            kind: BrowserRowKind::Document,
            full_path: "/home/pi/gh/notes/untitled-gg".to_string(),
            label: "gg".to_string(),
            detail_label: String::new(),
            document_kind: Some(WorkspaceDocumentKind::Note),
            group_kind: None,
            session_title: None,
            depth: 3,
            host_label: String::new(),
            descendant_sessions: 0,
            expanded: false,
            session_id: Some("gg-id".to_string()),
            session_cwd: Some("/home/pi/gh/notes".to_string()),
        };
        let separator = BrowserRow {
            kind: BrowserRowKind::Separator,
            full_path: "/home/pi/gh/notes/separator-a".to_string(),
            label: "Separator".to_string(),
            detail_label: String::new(),
            document_kind: None,
            group_kind: Some(WorkspaceGroupKind::Separator),
            session_title: None,
            depth: 3,
            host_label: String::new(),
            descendant_sessions: 0,
            expanded: false,
            session_id: None,
            session_cwd: None,
        };
        let rows = vec![
            BrowserRow {
                kind: BrowserRowKind::Document,
                full_path: "/home/pi/gh/notes/paper-a".to_string(),
                label: "paper-a".to_string(),
                detail_label: String::new(),
                document_kind: Some(WorkspaceDocumentKind::Note),
                group_kind: None,
                session_title: None,
                depth: 3,
                host_label: String::new(),
                descendant_sessions: 0,
                expanded: false,
                session_id: Some("paper-a-id".to_string()),
                session_cwd: Some("/home/pi/gh/notes".to_string()),
            },
            gg.clone(),
            separator.clone(),
        ];

        let placement = resolve_workspace_drop_placement(
            &rows,
            &DragDropTarget {
                path: separator.full_path.clone(),
                placement: DragDropPlacement::Before,
            },
        )
        .expect("placement");

        let plan = build_workspace_reorder_plan(&rows, std::slice::from_ref(&gg), &placement)
            .expect("plan");

        let gg_plan = plan
            .iter()
            .find(|item| item.from_path == gg.full_path)
            .expect("gg plan item");
        assert_eq!(gg_plan.final_path, "/home/pi/gh/notes/0001-untitled-gg");
    }

    #[test]
    fn creation_context_for_paper_resolves_to_parent_folder() {
        let folder = BrowserRow {
            kind: BrowserRowKind::Group,
            full_path: "/home/pi/gh/notes".to_string(),
            label: "notes".to_string(),
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
        let paper = BrowserRow {
            kind: BrowserRowKind::Document,
            full_path: "/home/pi/gh/notes/paper-123".to_string(),
            label: "Untitled paper".to_string(),
            detail_label: String::new(),
            document_kind: Some(WorkspaceDocumentKind::Note),
            group_kind: None,
            session_title: None,
            depth: 4,
            host_label: String::new(),
            descendant_sessions: 0,
            expanded: false,
            session_id: None,
            session_cwd: None,
        };
        let resolved = resolve_creation_context_row(&[folder.clone(), paper.clone()], &paper);
        assert_eq!(resolved.full_path, folder.full_path);
    }

    #[test]
    fn creation_context_for_separator_resolves_to_parent_folder() {
        let folder = BrowserRow {
            kind: BrowserRowKind::Group,
            full_path: "/home/pi/gh/notes".to_string(),
            label: "notes".to_string(),
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
        let separator = BrowserRow {
            kind: BrowserRowKind::Separator,
            full_path: "/home/pi/gh/notes/separator-123".to_string(),
            label: "Separator".to_string(),
            detail_label: String::new(),
            document_kind: None,
            group_kind: Some(WorkspaceGroupKind::Separator),
            session_title: None,
            depth: 4,
            host_label: String::new(),
            descendant_sessions: 0,
            expanded: false,
            session_id: None,
            session_cwd: None,
        };
        let resolved =
            resolve_creation_context_row(&[folder.clone(), separator.clone()], &separator);
        assert_eq!(resolved.full_path, folder.full_path);
    }

    #[test]
    fn new_separator_virtual_path_sorts_after_regular_children() {
        let folder = BrowserRow {
            kind: BrowserRowKind::Group,
            full_path: "/home/pi/gh/notes".to_string(),
            label: "notes".to_string(),
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

        let path = new_separator_virtual_path_for_row(&folder);

        assert!(path.starts_with("/home/pi/gh/notes/!separator-"));
    }

    #[test]
    fn new_separator_virtual_path_for_document_anchors_after_clicked_row() {
        let document = BrowserRow {
            kind: BrowserRowKind::Document,
            full_path: "/home/pi/gh/notes/untitled-1774153234".to_string(),
            label: "Untitled note".to_string(),
            detail_label: String::new(),
            document_kind: Some(WorkspaceDocumentKind::Note),
            group_kind: None,
            session_title: None,
            depth: 4,
            host_label: String::new(),
            descendant_sessions: 0,
            expanded: false,
            session_id: Some("paper-id".to_string()),
            session_cwd: Some("/home/pi/gh/notes".to_string()),
        };

        let path = new_separator_virtual_path_for_row(&document);

        assert!(path.starts_with("/home/pi/gh/notes/untitled-1774153234~separator-"));
    }

    #[test]
    fn context_menu_position_clamps_inside_window_bounds() {
        let placement = context_menu_placement((1010.0, 770.0), (1100.0, 820.0), (224.0, 420.0));

        assert_eq!(placement.left, None);
        assert_eq!(placement.top, None);
        assert_eq!(placement.right, Some(90.0));
        assert_eq!(placement.bottom, Some(50.0));
    }

    #[test]
    fn preview_content_blocks_render_markdown_shapes() {
        let blocks = preview_content_blocks(&[
            "# Heading".to_string(),
            String::new(),
            "- bullet".to_string(),
            "1. first".to_string(),
            "- [x] done".to_string(),
            "> quoted".to_string(),
            "```rust".to_string(),
            "fn main() {}".to_string(),
            "```".to_string(),
        ]);

        assert!(matches!(
            blocks.first(),
            Some(PreviewContentBlock::Heading { level: 1, text }) if text == "Heading"
        ));
        assert!(blocks.iter().any(|block| matches!(block, PreviewContentBlock::Bullet(text) if text == "bullet")));
        assert!(blocks.iter().any(|block| matches!(block, PreviewContentBlock::Numbered { number: 1, text } if text == "first")));
        assert!(blocks.iter().any(|block| matches!(block, PreviewContentBlock::Task { done: true, text } if text == "done")));
        assert!(blocks.iter().any(|block| matches!(block, PreviewContentBlock::Quote(text) if text == "quoted")));
        assert!(blocks.iter().any(|block| matches!(block, PreviewContentBlock::Code { language: Some(language), code } if language == "rust" && code == "fn main() {}")));
    }

    #[test]
    fn preview_block_excerpt_ignores_code_fence_markers() {
        let block = SessionPreviewBlock {
            role: "ASSISTANT",
            timestamp: "now".to_string(),
            tone: PreviewTone::Assistant,
            folded: false,
            lines: vec![
                "```bash".to_string(),
                "cargo test".to_string(),
                "```".to_string(),
                "Runs the focused regression suite.".to_string(),
            ],
        };

        assert_eq!(
            preview_block_excerpt(&block, 120).as_deref(),
            Some("cargo test Runs the focused regression suite.")
        );
    }

    #[test]
    fn merged_sidebar_rows_include_saved_ssh_machine_roots() {
        let expanded_paths = HashSet::from(["__remote_machine__/pi-raspberry".to_string()]);
        let rows = merged_sidebar_rows(
            &[],
            &[],
            &[SshConnectTarget {
                label: "raspberry".to_string(),
                kind: SessionKind::SshShell,
                ssh_target: "pi@raspberry".to_string(),
                prefix: None,
                cwd: None,
            }],
            &[],
            &expanded_paths,
        );

        assert!(rows.iter().any(|row| {
            row.kind == BrowserRowKind::Group
                && row.depth == 0
                && row.label == "raspberry [cached]"
        }));
    }

    #[test]
    fn merged_sidebar_rows_include_scanned_remote_sessions_under_machine_root() {
        let expanded_paths = HashSet::from([
            "__remote_machine__/pi-raspberry".to_string(),
            "__remote_folder__/pi-raspberry/srv/app".to_string(),
        ]);
        let rows = merged_sidebar_rows(
            &[],
            &[RemoteMachineSnapshot {
                machine_key: "pi-raspberry".to_string(),
                label: "raspberry".to_string(),
                ssh_target: "pi@raspberry".to_string(),
                prefix: None,
                health: RemoteMachineHealth::Healthy,
                sessions: vec![RemoteScannedSession {
                    session_path: "remote-session://pi-raspberry/019caa6f".to_string(),
                    session_id: "019caa6f".to_string(),
                    cwd: "/srv/app".to_string(),
                    started_at: "2026-03-23T10:00:00Z".to_string(),
                    modified_epoch: 123,
                    event_count: 22,
                    user_message_count: 11,
                    assistant_message_count: 10,
                    title_hint: "019caa6f".to_string(),
                    recent_context: "USER: test\nASSISTANT: reply".to_string(),
                    cached_precis: None,
                    cached_summary: None,
                    storage_path: "/home/pi/.codex/sessions/a.jsonl".to_string(),
                }],
            }],
            &[],
            &[],
            &expanded_paths,
        );

        assert!(rows.iter().any(|row| row.kind == BrowserRowKind::Group && row.label == "raspberry [ok]"));
        assert!(rows.iter().any(|row| {
            row.kind == BrowserRowKind::Group
                && row.full_path == "__remote_folder__/pi-raspberry/srv/app"
                && row.label == "/srv/app"
        }));
        assert!(rows.iter().any(|row| row.kind == BrowserRowKind::Session && row.full_path == "remote-session://pi-raspberry/019caa6f"));
    }

    #[test]
    fn merged_sidebar_rows_expand_remote_hash_labels_until_unique() {
        let expanded_paths = HashSet::from([
            "__remote_machine__/jojo".to_string(),
            "__remote_folder__/jojo/home".to_string(),
            "__remote_folder__/jojo/home/pi".to_string(),
        ]);
        let rows = merged_sidebar_rows(
            &[],
            &[RemoteMachineSnapshot {
                machine_key: "jojo".to_string(),
                label: "jojo".to_string(),
                ssh_target: "pi@jojo".to_string(),
                prefix: None,
                health: RemoteMachineHealth::Healthy,
                sessions: vec![
                    RemoteScannedSession {
                        session_path: "remote-session://jojo/a".to_string(),
                        session_id: "019d0a2e-bd7b-7260-925e-1f0ed8d3189b".to_string(),
                        cwd: "/home/pi".to_string(),
                        started_at: "2026-03-23T10:00:00Z".to_string(),
                        modified_epoch: 123,
                        event_count: 22,
                        user_message_count: 11,
                        assistant_message_count: 10,
                        title_hint: "Qd3189b".to_string(),
                        recent_context: "USER: test\nASSISTANT: reply".to_string(),
                        cached_precis: None,
                        cached_summary: None,
                        storage_path: "/home/pi/.codex/sessions/a.jsonl".to_string(),
                    },
                    RemoteScannedSession {
                        session_path: "remote-session://jojo/b".to_string(),
                        session_id: "019d0fff-bd7b-7260-925e-1f0ed8d3189b".to_string(),
                        cwd: "/home/pi".to_string(),
                        started_at: "2026-03-23T10:05:00Z".to_string(),
                        modified_epoch: 124,
                        event_count: 23,
                        user_message_count: 12,
                        assistant_message_count: 10,
                        title_hint: "Qd3189b".to_string(),
                        recent_context: "USER: test2\nASSISTANT: reply2".to_string(),
                        cached_precis: None,
                        cached_summary: None,
                        storage_path: "/home/pi/.codex/sessions/b.jsonl".to_string(),
                    },
                ],
            }],
            &[],
            &[],
            &expanded_paths,
        );

        let labels = rows
            .iter()
            .filter(|row| row.kind == BrowserRowKind::Session)
            .map(|row| row.label.clone())
            .collect::<Vec<_>>();
        assert_eq!(labels.len(), 2);
        assert_ne!(labels[0], labels[1]);
        assert!(labels.iter().all(|label| label.len() > "Qd3189b".len()));
    }

    #[test]
    fn merged_sidebar_rows_hide_nested_remote_children_when_folder_collapsed() {
        let expanded_paths = HashSet::from(["__remote_machine__/pi-raspberry".to_string()]);
        let rows = merged_sidebar_rows(
            &[],
            &[RemoteMachineSnapshot {
                machine_key: "pi-raspberry".to_string(),
                label: "raspberry".to_string(),
                ssh_target: "pi@raspberry".to_string(),
                prefix: None,
                health: RemoteMachineHealth::Healthy,
                sessions: vec![RemoteScannedSession {
                    session_path: "remote-session://pi-raspberry/019caa6f".to_string(),
                    session_id: "019caa6f".to_string(),
                    cwd: "/srv/app".to_string(),
                    started_at: "2026-03-23T10:00:00Z".to_string(),
                    modified_epoch: 123,
                    event_count: 22,
                    user_message_count: 11,
                    assistant_message_count: 10,
                    title_hint: "019caa6f".to_string(),
                    recent_context: "USER: test\nASSISTANT: reply".to_string(),
                    cached_precis: None,
                    cached_summary: None,
                    storage_path: "/home/pi/.codex/sessions/a.jsonl".to_string(),
                }],
            }],
            &[],
            &[],
            &expanded_paths,
        );

        assert!(rows.iter().any(|row| row.full_path == "__remote_folder__/pi-raspberry/srv/app"));
        assert!(!rows.iter().any(|row| row.full_path == "remote-session://pi-raspberry/019caa6f"));
    }

    #[test]
    fn saved_ssh_target_machine_key_only_matches_persisted_remote_machine_rows() {
        let row = BrowserRow {
            kind: BrowserRowKind::Group,
            full_path: "__remote_machine__/pi-raspberry".to_string(),
            label: "raspberry [cached]".to_string(),
            detail_label: String::new(),
            document_kind: None,
            group_kind: None,
            session_title: None,
            depth: 0,
            host_label: String::new(),
            descendant_sessions: 1,
            expanded: true,
            session_id: None,
            session_cwd: None,
        };
        let targets = vec![SshConnectTarget {
            label: "raspberry".to_string(),
            kind: SessionKind::SshShell,
            ssh_target: "pi@raspberry".to_string(),
            prefix: None,
            cwd: None,
        }];

        assert_eq!(
            saved_ssh_target_machine_key(&row, &targets).as_deref(),
            Some("pi-raspberry")
        );
        assert_eq!(saved_ssh_target_machine_key(&row, &[]), None);
    }

    #[test]
    fn merged_sidebar_rows_do_not_render_live_ssh_sessions_under_machine_roots() {
        let expanded_paths = HashSet::from([
            "__remote_machine__/dev".to_string(),
        ]);
        let rows = merged_sidebar_rows(
            &[],
            &[RemoteMachineSnapshot {
                machine_key: "dev".to_string(),
                label: "dev".to_string(),
                ssh_target: "dev".to_string(),
                prefix: None,
                health: RemoteMachineHealth::Healthy,
                sessions: Vec::new(),
            }],
            &[SshConnectTarget {
                label: "dev".to_string(),
                kind: SessionKind::SshShell,
                ssh_target: "dev".to_string(),
                prefix: None,
                cwd: None,
            }],
            &[ManagedSessionView {
                id: "session-1".to_string(),
                session_path: "ssh://dev/session-1".to_string(),
                title: "dev".to_string(),
                kind: SessionKind::SshShell,
                host_label: "dev".to_string(),
                source: yggterm_server::SessionSource::LiveSsh,
                backend: TerminalBackend::Xterm,
                bridge_available: false,
                launch_phase: yggterm_server::TerminalLaunchPhase::Running,
                remote_deploy_state: yggterm_server::RemoteDeployState::Ready,
                launch_command: "ssh dev".to_string(),
                status_line: "attached".to_string(),
                terminal_lines: Vec::new(),
                rendered_sections: Vec::new(),
                preview: yggterm_server::SessionPreview {
                    summary: Vec::new(),
                    blocks: Vec::new(),
                },
                metadata: Vec::new(),
                terminal_process_id: None,
                terminal_window_id: None,
                terminal_host_token: None,
                terminal_host_mode: GhosttyTerminalHostMode::Unsupported,
                embedded_surface_id: None,
                embedded_surface_detail: None,
                last_launch_error: None,
                last_window_error: None,
                ssh_target: Some("dev".to_string()),
                ssh_prefix: None,
            }],
            &expanded_paths,
        );

        assert!(!rows.iter().any(|row| row.full_path == "__live_sessions__"));
        assert!(!rows.iter().any(|row| row.full_path == "ssh://dev/session-1"));
        let machine_rows = rows
            .iter()
            .filter(|row| row.full_path.starts_with("__remote_machine__/dev") || row.host_label == "dev")
            .collect::<Vec<_>>();
        assert_eq!(
            machine_rows
                .iter()
                .filter(|row| row.kind == BrowserRowKind::Session)
                .count(),
            0
        );
    }

    #[test]
    fn remote_folder_rows_show_descendant_session_counts() {
        let expanded_paths = HashSet::from([
            "__remote_machine__/jojo".to_string(),
            "__remote_folder__/jojo/home/pi".to_string(),
        ]);
        let rows = merged_sidebar_rows(
            &[],
            &[RemoteMachineSnapshot {
                machine_key: "jojo".to_string(),
                label: "jojo".to_string(),
                ssh_target: "jojo".to_string(),
                prefix: None,
                health: RemoteMachineHealth::Healthy,
                sessions: vec![
                    RemoteScannedSession {
                        session_path: "remote-session://jojo/1".to_string(),
                        session_id: "1".to_string(),
                        cwd: "/home/pi".to_string(),
                        started_at: "2026-03-24T10:00:00Z".to_string(),
                        modified_epoch: 1,
                        event_count: 1,
                        user_message_count: 1,
                        assistant_message_count: 1,
                        title_hint: "1".to_string(),
                        recent_context: "USER: one".to_string(),
                        cached_precis: None,
                        cached_summary: None,
                        storage_path: "a".to_string(),
                    },
                    RemoteScannedSession {
                        session_path: "remote-session://jojo/2".to_string(),
                        session_id: "2".to_string(),
                        cwd: "/home/pi".to_string(),
                        started_at: "2026-03-24T10:01:00Z".to_string(),
                        modified_epoch: 2,
                        event_count: 1,
                        user_message_count: 1,
                        assistant_message_count: 1,
                        title_hint: "2".to_string(),
                        recent_context: "USER: two".to_string(),
                        cached_precis: None,
                        cached_summary: None,
                        storage_path: "b".to_string(),
                    },
                ],
            }],
            &[],
            &[],
            &expanded_paths,
        );

        let home_row = rows
            .iter()
            .find(|row| row.full_path == "__remote_folder__/jojo/home/pi")
            .expect("home row");
        assert_eq!(home_row.descendant_sessions, 2);
    }

    #[test]
    fn merged_sidebar_rows_compress_single_child_remote_folder_chains() {
        let expanded_paths = HashSet::from([
            "__remote_machine__/jojo".to_string(),
            "__remote_folder__/jojo/run/smb4k/data/smbfs/dada/obsidian/codex".to_string(),
        ]);
        let rows = merged_sidebar_rows(
            &[],
            &[RemoteMachineSnapshot {
                machine_key: "jojo".to_string(),
                label: "jojo".to_string(),
                ssh_target: "jojo".to_string(),
                prefix: None,
                health: RemoteMachineHealth::Healthy,
                sessions: vec![RemoteScannedSession {
                    session_path: "remote-session://jojo/1".to_string(),
                    session_id: "1".to_string(),
                    cwd: "/run/smb4k/data/smbfs/dada/obsidian/codex".to_string(),
                    started_at: "2026-03-24T10:00:00Z".to_string(),
                    modified_epoch: 1,
                    event_count: 1,
                    user_message_count: 1,
                    assistant_message_count: 1,
                    title_hint: "1".to_string(),
                    recent_context: "USER: one".to_string(),
                    cached_precis: None,
                    cached_summary: None,
                    storage_path: "a".to_string(),
                }],
            }],
            &[],
            &[],
            &expanded_paths,
        );

        assert!(rows.iter().any(|row| {
            row.kind == BrowserRowKind::Group
                && row.full_path == "__remote_folder__/jojo/run/smb4k/data/smbfs/dada/obsidian/codex"
                && row.label == "/run/smb4k/data/smbfs/dada/obsidian/codex"
        }));
        assert!(!rows.iter().any(|row| row.full_path == "__remote_folder__/jojo/run"));
    }

    #[test]
    fn merged_sidebar_rows_include_live_remote_session_when_scan_snapshot_is_missing_it() {
        let rows = merged_sidebar_rows(
            &[],
            &[RemoteMachineSnapshot {
                machine_key: "oc".to_string(),
                label: "oc [ok]".to_string(),
                ssh_target: "oc".to_string(),
                prefix: None,
                health: RemoteMachineHealth::Healthy,
                sessions: vec![],
            }],
            &[],
            &[ManagedSessionView {
                id: "019cf672-8d68-70a1-bd8b-68487c4fc63d".to_string(),
                session_path: "remote-session://oc/019cf672-8d68-70a1-bd8b-68487c4fc63d".to_string(),
                title: "Can You Change Timezone Host".to_string(),
                kind: SessionKind::Codex,
                host_label: "oc".to_string(),
                source: yggterm_server::SessionSource::LiveSsh,
                backend: TerminalBackend::Xterm,
                bridge_available: true,
                launch_phase: yggterm_server::TerminalLaunchPhase::Running,
                remote_deploy_state: yggterm_server::RemoteDeployState::Ready,
                launch_command: String::new(),
                status_line: String::new(),
                terminal_lines: vec![],
                rendered_sections: vec![],
                preview: yggterm_server::SessionPreview {
                    summary: vec![],
                    blocks: vec![],
                },
                metadata: vec![
                    SessionMetadataEntry { label: "Cwd", value: "/home/pi".to_string() },
                    SessionMetadataEntry { label: "Storage", value: "/home/pi/.codex/sessions/foo.jsonl".to_string() },
                    SessionMetadataEntry { label: "Started", value: "now".to_string() },
                ],
                terminal_process_id: None,
                terminal_window_id: None,
                terminal_host_token: None,
                terminal_host_mode: GhosttyTerminalHostMode::Unsupported,
                embedded_surface_id: None,
                embedded_surface_detail: None,
                last_launch_error: None,
                last_window_error: None,
                ssh_target: Some("oc".to_string()),
                ssh_prefix: None,
            }],
            &HashSet::from_iter([
                "__remote_machine__/oc".to_string(),
                "__remote_folder__/oc/home/pi".to_string(),
            ]),
        );

        assert!(rows.iter().any(|row| row.full_path == "remote-session://oc/019cf672-8d68-70a1-bd8b-68487c4fc63d"));
    }

    #[test]
    fn merged_sidebar_rows_break_compression_when_middle_folder_has_sessions() {
        let expanded_paths = HashSet::from([
            "__remote_machine__/jojo".to_string(),
            "__remote_folder__/jojo/home/pi".to_string(),
            "__remote_folder__/jojo/home/pi/data/smbfs".to_string(),
        ]);
        let rows = merged_sidebar_rows(
            &[],
            &[RemoteMachineSnapshot {
                machine_key: "jojo".to_string(),
                label: "jojo".to_string(),
                ssh_target: "jojo".to_string(),
                prefix: None,
                health: RemoteMachineHealth::Healthy,
                sessions: vec![
                    RemoteScannedSession {
                        session_path: "remote-session://jojo/1".to_string(),
                        session_id: "1".to_string(),
                        cwd: "/home/pi".to_string(),
                        started_at: "2026-03-24T10:00:00Z".to_string(),
                        modified_epoch: 1,
                        event_count: 1,
                        user_message_count: 1,
                        assistant_message_count: 1,
                        title_hint: "1".to_string(),
                        recent_context: "USER: one".to_string(),
                        cached_precis: None,
                        cached_summary: None,
                        storage_path: "a".to_string(),
                    },
                    RemoteScannedSession {
                        session_path: "remote-session://jojo/2".to_string(),
                        session_id: "2".to_string(),
                        cwd: "/home/pi/data/smbfs".to_string(),
                        started_at: "2026-03-24T10:01:00Z".to_string(),
                        modified_epoch: 2,
                        event_count: 1,
                        user_message_count: 1,
                        assistant_message_count: 1,
                        title_hint: "2".to_string(),
                        recent_context: "USER: two".to_string(),
                        cached_precis: None,
                        cached_summary: None,
                        storage_path: "b".to_string(),
                    },
                ],
            }],
            &[],
            &[],
            &expanded_paths,
        );

        assert!(rows.iter().any(|row| {
            row.kind == BrowserRowKind::Group
                && row.full_path == "__remote_folder__/jojo/home/pi"
                && row.label == "/home/pi"
        }));
        assert!(rows.iter().any(|row| {
            row.kind == BrowserRowKind::Group
                && row.full_path == "__remote_folder__/jojo/home/pi/data/smbfs"
                && row.label == "data/smbfs"
        }));
        assert!(!rows.iter().any(|row| row.full_path == "__remote_folder__/jojo/home"));
    }

    #[test]
    fn compressed_remote_folder_paths_follow_rendered_chain() {
        let machine = SidebarRemoteMachine {
            key: "jojo".to_string(),
            label: "jojo".to_string(),
            health: MachineHealth::Healthy,
            scanned_sessions: vec![RemoteScannedSession {
                session_path: "remote-session://jojo/1".to_string(),
                session_id: "1".to_string(),
                cwd: "/run/smb4k/data/smbfs/dada/obsidian/codex".to_string(),
                started_at: "2026-03-24T10:00:00Z".to_string(),
                modified_epoch: 1,
                event_count: 1,
                user_message_count: 1,
                assistant_message_count: 1,
                title_hint: "1".to_string(),
                recent_context: "USER: one".to_string(),
                cached_precis: None,
                cached_summary: None,
                storage_path: "a".to_string(),
            }],
        };

        assert_eq!(
            compressed_remote_folder_paths(&machine, "/run/smb4k/data/smbfs/dada/obsidian/codex"),
            vec!["/run/smb4k/data/smbfs/dada/obsidian/codex".to_string()]
        );
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn interface_font_family() -> &'static str {
    "system-ui, sans-serif"
}
