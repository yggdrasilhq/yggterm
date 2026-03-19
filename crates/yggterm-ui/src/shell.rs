use anyhow::Result;
use dioxus::desktop::{Config, LogicalSize, WindowBuilder, window};
use dioxus::prelude::*;
use once_cell::sync::OnceCell;
use std::path::PathBuf;
use tao::window::ResizeDirection;
use tokio::task;
use tracing::{info, warn};
use yggterm_core::{
    AppSettings, BrowserRow, BrowserRowKind, ManagedSessionView, PreviewTone, SessionBrowserState,
    SessionNode, SessionStore, SshConnectTarget, UiTheme, WorkspaceViewMode, YggtermServer,
    save_settings_file,
};

static BOOTSTRAP: OnceCell<ShellBootstrap> = OnceCell::new();
const SIDE_RAIL_WIDTH: usize = 292;
const EDGE_RESIZE_HANDLE: usize = 5;
const CORNER_RESIZE_HANDLE: usize = 10;

#[derive(Debug, Clone)]
pub struct ShellBootstrap {
    pub tree: SessionNode,
    pub browser_tree: SessionNode,
    pub settings: AppSettings,
    pub settings_path: PathBuf,
    pub theme: UiTheme,
    pub ghostty_bridge_enabled: bool,
    pub ghostty_embedded_surface_supported: bool,
    pub ghostty_bridge_detail: String,
    pub prefer_ghostty_backend: bool,
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
    context_menu_row: Option<BrowserRow>,
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

#[derive(Clone, PartialEq)]
struct ToastNotification {
    id: u64,
    tone: NotificationTone,
    title: String,
    message: String,
}

#[derive(Clone, PartialEq)]
struct RenderSnapshot {
    palette: Palette,
    search_query: String,
    sidebar_open: bool,
    right_panel_mode: RightPanelMode,
    rows: Vec<BrowserRow>,
    selected_path: Option<String>,
    active_session: Option<ManagedSessionView>,
    active_view_mode: WorkspaceViewMode,
    ssh_targets: Vec<SshConnectTarget>,
    live_sessions: Vec<ManagedSessionView>,
    total_leaf_sessions: usize,
    last_action: String,
    ghostty_embedded_surface_supported: bool,
    ghostty_bridge_detail: String,
    settings: AppSettings,
    maximized: bool,
    always_on_top: bool,
    notifications: Vec<ToastNotification>,
    context_menu_row: Option<BrowserRow>,
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

impl ShellState {
    fn new(bootstrap: ShellBootstrap) -> Self {
        let settings = bootstrap.settings.clone();
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
                bootstrap.ghostty_bridge_enabled,
                bootstrap.theme,
            );
        if let Some(row) = browser.selected_row().cloned() {
            if row.kind == BrowserRowKind::Session {
                server.open_or_focus_session(
                    &row.full_path,
                    row.session_id.as_deref(),
                    row.session_cwd.as_deref(),
                    Some(row.label.as_str()),
                );
                server.set_view_mode(WorkspaceViewMode::Rendered);
            }
        }

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
            context_menu_row: None,
        };
        state.sync_browser_settings();
        state
    }

    fn snapshot(&self) -> RenderSnapshot {
        RenderSnapshot {
            palette: palette(self.bootstrap.theme),
            search_query: self.search_query.clone(),
            sidebar_open: self.sidebar_open,
            right_panel_mode: self.right_panel_mode,
            rows: self.browser.rows().to_vec(),
            selected_path: self.browser.selected_path().map(ToOwned::to_owned),
            active_session: self.server.active_session().cloned(),
            active_view_mode: self.server.active_view_mode(),
            ssh_targets: self.server.ssh_targets().to_vec(),
            live_sessions: self.server.live_sessions(),
            total_leaf_sessions: self.browser.total_sessions(),
            last_action: self.last_action.clone(),
            ghostty_embedded_surface_supported: self.bootstrap.ghostty_embedded_surface_supported,
            ghostty_bridge_detail: self.bootstrap.ghostty_bridge_detail.clone(),
            settings: self.settings.clone(),
            maximized: self.maximized,
            always_on_top: self.always_on_top,
            notifications: self.notifications.clone(),
            context_menu_row: self.context_menu_row.clone(),
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

    fn set_view_mode(&mut self, mode: WorkspaceViewMode) {
        self.server.set_view_mode(mode);
        self.last_action = match mode {
            WorkspaceViewMode::Rendered => "preview mode".to_string(),
            WorkspaceViewMode::Terminal => "terminal mode".to_string(),
        };
    }

    fn select_row(&mut self, row: &BrowserRow) {
        self.browser.select_path(row.full_path.clone());
        match row.kind {
            BrowserRowKind::Group => {
                self.browser.toggle_group(&row.full_path);
                self.sync_browser_settings();
                self.last_action = format!("toggled {}", row.label);
            }
            BrowserRowKind::Session => {
                self.context_menu_row = None;
                self.server.open_or_focus_session(
                    &row.full_path,
                    row.session_id.as_deref(),
                    row.session_cwd.as_deref(),
                    Some(row.label.as_str()),
                );
                self.server.set_view_mode(WorkspaceViewMode::Rendered);
                self.sync_browser_settings();
                self.last_action = format!("opened {}", row.label);
            }
        }
    }

    fn connect_ssh_target(&mut self, target_ix: usize) {
        if let Some(key) = self.server.connect_ssh_target(target_ix) {
            self.last_action = format!("connected {key}");
            self.push_notification(
                NotificationTone::Success,
                "SSH Session Started",
                format!("Opened live session {key}."),
            );
        }
    }

    fn focus_live_session(&mut self, key: &str) {
        self.server.focus_live_session(key);
        self.last_action = format!("focused {key}");
    }

    fn toggle_preview_block(&mut self, block_ix: usize) {
        self.server.toggle_preview_block(block_ix);
        self.last_action = format!("preview block {}", block_ix + 1);
    }

    fn expand_preview_blocks(&mut self) {
        self.server.set_all_preview_blocks_folded(false);
        self.last_action = "expanded preview".to_string();
    }

    fn collapse_preview_blocks(&mut self) {
        self.server.set_all_preview_blocks_folded(true);
        self.last_action = "collapsed preview".to_string();
    }

    fn request_terminal_launch(&mut self) {
        self.server.request_terminal_launch_for_active();
        self.server.set_view_mode(WorkspaceViewMode::Terminal);
        self.last_action = "requested ghostty".to_string();
    }

    fn resolve_ghostty_window(&mut self) {
        self.last_action = self.server.sync_external_terminal_window_for_active();
    }

    fn focus_ghostty_window(&mut self) {
        self.last_action = self.server.raise_external_terminal_window_for_active();
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

    fn adjust_ui_zoom(&mut self, delta_steps: i32) {
        self.settings.ui_font_size = clamp_zoom_value(self.settings.ui_font_size + delta_steps as f32);
        self.persist_settings();
        self.last_action = format!("interface zoom {}%", zoom_percent(self.settings.ui_font_size, 14.0));
    }

    fn adjust_main_zoom(&mut self, delta_steps: i32) {
        self.settings.terminal_font_size =
            clamp_zoom_value_main(self.settings.terminal_font_size + delta_steps as f32);
        self.persist_settings();
        self.last_action = format!("main zoom {}%", zoom_percent(self.settings.terminal_font_size, 13.0));
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

    fn open_context_menu(&mut self, row: BrowserRow) {
        self.context_menu_row = Some(row);
    }

    fn close_context_menu(&mut self) {
        self.context_menu_row = None;
    }

    fn clear_notification(&mut self, id: u64) {
        self.notifications.retain(|notification| notification.id != id);
    }

    fn clear_notifications(&mut self) {
        self.notifications.clear();
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
        self.notifications.push(ToastNotification {
            id: self.next_notification_id,
            tone,
            title: title.into(),
            message: message.into(),
        });
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
        let outcome = task::spawn_blocking(move || -> Result<(Option<String>, yggterm_core::SessionNode)> {
            info!(session_path=%row_for_task.full_path, force, "running title generation task");
            let store = SessionStore::open_or_init()?;
            let title = store.generate_title_for_session_path(
                &settings_for_task,
                &row_for_task.full_path,
                force,
            )?;
            let browser_tree = store.load_codex_tree(&settings_for_task)?;
            Ok((title, browser_tree))
        })
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
                shell.server.open_or_focus_session(
                    &row.full_path,
                    row.session_id.as_deref(),
                    row.session_cwd.as_deref(),
                    Some(&title),
                );
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

pub fn launch_shell(bootstrap: ShellBootstrap) -> Result<()> {
    let _ = BOOTSTRAP.set(bootstrap);

    dioxus::LaunchBuilder::desktop()
        .with_cfg(
            Config::new().with_window(
                WindowBuilder::new()
                    .with_title("yggterm")
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
    let bootstrap = BOOTSTRAP.get().expect("shell bootstrap not initialized").clone();
    let mut state = use_signal(|| ShellState::new(bootstrap));
    let mut hovered = use_signal(|| None::<HoveredControl>);
    let snapshot = state.read().snapshot();
    let titlebar_snapshot = snapshot.clone();
    let sidebar_snapshot = snapshot.clone();
    let main_snapshot = snapshot.clone();
    let metadata_snapshot = snapshot.clone();
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
            div {
                style: shell_style(snapshot.palette, shell_radius),
                WindowResizeHandles {}
                Titlebar {
                    snapshot: titlebar_snapshot,
                    hovered: hovered,
                    on_toggle_sidebar: move || state.with_mut(|shell| shell.toggle_sidebar()),
                    on_search: move |value: String| state.with_mut(|shell| shell.set_search(value)),
                    on_hover_control: move |control: Option<HoveredControl>| hovered.set(control),
                    on_set_view_mode: move |mode: WorkspaceViewMode| state.with_mut(|shell| shell.set_view_mode(mode)),
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
                        on_select_row: move |row: BrowserRow| {
                            let should_generate = row.kind == BrowserRowKind::Session && row.session_title.is_none();
                            state.with_mut(|shell| shell.select_row(&row));
                            if should_generate {
                                queue_title_generation(state, row.clone(), false);
                            }
                        },
                        on_open_context_menu: move |row: BrowserRow| state.with_mut(|shell| shell.open_context_menu(row)),
                    }
                    MainSurface {
                        snapshot: main_snapshot,
                        on_expand_preview: move || state.with_mut(|shell| shell.expand_preview_blocks()),
                        on_collapse_preview: move || state.with_mut(|shell| shell.collapse_preview_blocks()),
                        on_toggle_preview_block: move |ix: usize| state.with_mut(|shell| shell.toggle_preview_block(ix)),
                        on_request_terminal: move || state.with_mut(|shell| shell.request_terminal_launch()),
                        on_focus_ghostty: move || state.with_mut(|shell| shell.focus_ghostty_window()),
                        on_resolve_ghostty: move || state.with_mut(|shell| shell.resolve_ghostty_window()),
                    }
                    RightRail {
                        snapshot: metadata_snapshot,
                        on_endpoint_change: move |value: String| state.with_mut(|shell| shell.update_litellm_endpoint(value)),
                        on_api_key_change: move |value: String| state.with_mut(|shell| shell.update_litellm_api_key(value)),
                        on_model_change: move |value: String| state.with_mut(|shell| shell.update_interface_llm_model(value)),
                        on_generate_titles: move |_| state.with_mut(|shell| shell.generate_session_titles()),
                        on_adjust_ui_zoom: move |delta: i32| state.with_mut(|shell| shell.adjust_ui_zoom(delta)),
                        on_adjust_main_zoom: move |delta: i32| state.with_mut(|shell| shell.adjust_main_zoom(delta)),
                        on_connect_ssh: move |ix: usize| state.with_mut(|shell| shell.connect_ssh_target(ix)),
                        on_focus_live: move |id: String| state.with_mut(|shell| shell.focus_live_session(&id)),
                        on_clear_notification: move |id: u64| state.with_mut(|shell| shell.clear_notification(id)),
                        on_clear_notifications: move |_| state.with_mut(|shell| shell.clear_notifications()),
                    }
                }
                if let Some(row) = snapshot.context_menu_row.clone() {
                    ContextMenuOverlay {
                        row: row.clone(),
                        palette: snapshot.palette,
                        on_close: move |_| state.with_mut(|shell| shell.close_context_menu()),
                        on_regenerate: move |_| {
                            state.with_mut(|shell| shell.regenerate_title_for_row(&row));
                            queue_title_generation(state, row.clone(), true);
                        },
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
    on_select_row: EventHandler<BrowserRow>,
    on_open_context_menu: EventHandler<BrowserRow>,
) -> Element {
    let width = if snapshot.sidebar_open { SIDE_RAIL_WIDTH } else { 0 };
    let opacity = if snapshot.sidebar_open { "1" } else { "0" };
    let translate = if snapshot.sidebar_open { "translateX(0)" } else { "translateX(-14px)" };
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
            div {
                style: "flex:1; min-height:0; overflow:auto; padding:14px 12px 12px 12px;",
                for row in snapshot.rows.iter().cloned() {
                    {
                        let select_row = row.clone();
                        let context_row = row.clone();
                        rsx! {
                            SidebarRow {
                                row: row.clone(),
                                selected: snapshot.selected_path.as_deref() == Some(row.full_path.as_str()),
                                palette: snapshot.palette,
                                on_select: move |_| on_select_row.call(select_row.clone()),
                                on_open_context_menu: move |_| on_open_context_menu.call(context_row.clone()),
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
    palette: Palette,
    on_select: EventHandler<MouseEvent>,
    on_open_context_menu: EventHandler<MouseEvent>,
) -> Element {
    let indent = row.depth * 12 + 12;
    let background = if selected {
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
                 border:none; border-radius:12px; background:{}; padding:6px 9px 6px {}px; margin-bottom:2px;",
                background, indent
            ),
            onclick: move |evt| on_select.call(evt),
            oncontextmenu: move |evt| {
                evt.prevent_default();
                on_open_context_menu.call(evt);
            },
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
                    span {
                        style: format!(
                            "font-size:11px; color:{}; font-weight:{}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;",
                            label_color,
                            if row.kind == BrowserRowKind::Group && row.depth == 0 { 600 } else { 500 }
                        ),
                        "{row.label}"
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
fn TreeIcon(row: BrowserRow) -> Element {
    if row.kind == BrowserRowKind::Session {
        return rsx! {
            span {
                style: "display:inline-flex; align-items:center; justify-content:center; font-size:13px; font-weight:700; line-height:1;",
                ">_"
            }
        };
    }

    if row.depth == 0 {
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
    on_request_terminal: EventHandler<()>,
    on_focus_ghostty: EventHandler<()>,
    on_resolve_ghostty: EventHandler<()>,
) -> Element {
    let body = if let Some(session) = snapshot.active_session.clone() {
        match snapshot.active_view_mode {
            WorkspaceViewMode::Rendered => rsx! {
                div {
                    style: "display:flex; flex-direction:column; gap:16px; min-width:0;",
                    div {
                        style: "display:flex; align-items:flex-start; justify-content:space-between; gap:16px;",
                        PreviewSummary { session: session.clone(), palette: snapshot.palette }
                        div {
                            style: "display:flex; gap:8px;",
                            button {
                                style: chip_style(snapshot.palette, false),
                                onclick: move |_| on_expand_preview.call(()),
                                "Expand All"
                            }
                            button {
                                style: chip_style(snapshot.palette, false),
                                onclick: move |_| on_collapse_preview.call(()),
                                "Collapse All"
                            }
                        }
                    }
                    div {
                        style: "display:flex; flex-direction:column; gap:14px;",
                        for (ix, block) in session.preview.blocks.iter().cloned().enumerate() {
                            PreviewBlock {
                                block_ix: ix,
                                block: block.clone(),
                                palette: snapshot.palette,
                                on_toggle: move |_| on_toggle_preview_block.call(ix),
                            }
                        }
                    }
                }
            },
            WorkspaceViewMode::Terminal => rsx! {
                div {
                    style: "display:flex; flex-direction:column; gap:16px; min-width:0;",
                    div {
                        style: "display:flex; gap:8px; justify-content:flex-end;",
                        button {
                            style: chip_style(snapshot.palette, false),
                            onclick: move |_| on_request_terminal.call(()),
                            "Request Ghostty"
                        }
                        button {
                            style: chip_style(snapshot.palette, false),
                            onclick: move |_| on_focus_ghostty.call(()),
                            "Focus Ghostty"
                        }
                        button {
                            style: chip_style(snapshot.palette, false),
                            onclick: move |_| on_resolve_ghostty.call(()),
                            "Resolve Window"
                        }
                    }
                    TerminalCard {
                        title: "Server Terminal".to_string(),
                        subtitle: session.status_line.clone(),
                        lines: session.terminal_lines.clone(),
                        palette: snapshot.palette,
                    }
                    TerminalCard {
                        title: "Ghostty Integration".to_string(),
                        subtitle: if snapshot.ghostty_embedded_surface_supported {
                            "embedded surface available".to_string()
                        } else {
                            "external window path".to_string()
                        },
                        lines: vec![
                            snapshot.ghostty_bridge_detail.clone(),
                            "Yggterm server opens and tracks session-owned Ghostty launches here.".to_string(),
                        ],
                        palette: snapshot.palette,
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
                    "flex:1; overflow:auto; padding:24px; background:{}; border-radius:11px; box-shadow:{}; zoom:{}%;",
                    snapshot.palette.panel, snapshot.palette.panel_shadow,
                    zoom_percent_f32(snapshot.settings.terminal_font_size, 13.0)
                ),
                {body}
            }
        }
    }
}

#[component]
fn PreviewSummary(session: ManagedSessionView, palette: Palette) -> Element {
    let started = metadata_value(&session, "Started");
    let messages = metadata_value(&session, "Messages");

    rsx! {
        div {
            style: format!(
                "display:flex; flex-direction:column; gap:6px; min-width:280px; max-width:360px; \
                 background:{}; border:none; border-radius:14px; padding:14px 16px; box-shadow: inset 0 0 0 1px rgba(255,255,255,0.38);",
                palette.panel_alt
            ),
            div {
                style: format!("font-size:13px; font-weight:700; color:{};", palette.text),
                "{session.title}"
            }
            div {
                style: format!("font-size:11px; color:{};", palette.muted),
                "{session.host_label} · {started}"
            }
            div {
                style: format!("font-size:11px; color:{};", palette.muted),
                "{messages} · {session.preview.blocks.len()} blocks"
            }
        }
    }
}

#[component]
fn PreviewBlock(
    block_ix: usize,
    block: yggterm_core::SessionPreviewBlock,
    palette: Palette,
    on_toggle: EventHandler<MouseEvent>,
) -> Element {
    let background = match block.tone {
        PreviewTone::User => palette.accent_soft,
        PreviewTone::Assistant => palette.panel_alt,
    };
    let badge = match block.tone {
        PreviewTone::User => palette.accent,
        PreviewTone::Assistant => palette.muted,
    };

    rsx! {
        button {
            style: format!(
                "width:100%; border:none; text-align:left; background:{}; border-radius:14px; \
                 padding:15px 16px; box-shadow: inset 0 0 0 1px rgba(255,255,255,0.42);",
                background
            ),
            onclick: move |evt| on_toggle.call(evt),
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:12px; margin-bottom:10px;",
                div {
                    style: "display:flex; align-items:center; gap:8px;",
                    span {
                        style: format!(
                            "display:inline-flex; align-items:center; justify-content:center; min-width:54px; height:22px; \
                             border-radius:999px; background:{}; color:{}; font-size:11px; font-weight:700;",
                            if block.tone == PreviewTone::User { "rgba(37,99,235,0.14)" } else { "rgba(108,114,127,0.12)" },
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
                    style: format!("display:flex; flex-direction:column; gap:7px; color:{};", palette.text),
                    for line in block.lines.iter() {
                        div {
                            style: "font-size:13px; line-height:1.45; white-space:pre-wrap;",
                            "{line}"
                        }
                    }
                }
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
    on_generate_titles: EventHandler<MouseEvent>,
    on_adjust_ui_zoom: EventHandler<i32>,
    on_adjust_main_zoom: EventHandler<i32>,
    on_connect_ssh: EventHandler<usize>,
    on_focus_live: EventHandler<String>,
    on_clear_notification: EventHandler<u64>,
    on_clear_notifications: EventHandler<MouseEvent>,
) -> Element {
    let visible = snapshot.right_panel_mode != RightPanelMode::Hidden;
    let width = if visible { SIDE_RAIL_WIDTH } else { 0 };
    let opacity = if visible { "1" } else { "0" };
    let translate = if visible { "translateX(0)" } else { "translateX(14px)" };
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
                    on_generate_titles,
                    on_adjust_ui_zoom,
                    on_adjust_main_zoom,
                }
            } else if snapshot.right_panel_mode == RightPanelMode::Connect {
                ConnectRailBody {
                    snapshot: snapshot.clone(),
                    on_connect_ssh,
                    on_focus_live,
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
                    entries: vec![yggterm_core::SessionMetadataEntry {
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
            ZoomSettingRow {
                label: "Interface Zoom".to_string(),
                percent: zoom_percent(snapshot.settings.ui_font_size, 14.0),
                palette: snapshot.palette,
                on_decrease: move |_| on_adjust_ui_zoom.call(-1),
                on_increase: move |_| on_adjust_ui_zoom.call(1),
            }
            ZoomSettingRow {
                label: "Main Zoom".to_string(),
                percent: zoom_percent(snapshot.settings.terminal_font_size, 13.0),
                palette: snapshot.palette,
                on_decrease: move |_| on_adjust_main_zoom.call(-1),
                on_increase: move |_| on_adjust_main_zoom.call(1),
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
    on_connect_ssh: EventHandler<usize>,
    on_focus_live: EventHandler<String>,
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
            style: "flex:1; overflow:auto; padding:10px 16px 14px 16px; display:flex; flex-direction:column; gap:16px;",
            if !snapshot.ssh_targets.is_empty() {
                div {
                    style: "display:flex; flex-direction:column; gap:8px;",
                    for (ix, target) in snapshot.ssh_targets.iter().cloned().enumerate() {
                        {
                            let target_detail = if let Some(prefix) = target.prefix.as_ref() {
                                format!("{} · {}", target.ssh_target, prefix)
                            } else {
                                target.ssh_target.clone()
                            };
                            rsx! {
                                button {
                                    style: sidebar_action_style(snapshot.palette),
                                    onclick: move |_| on_connect_ssh.call(ix),
                                    div {
                                        style: "display:flex; flex-direction:column; align-items:flex-start; gap:3px; min-width:0;",
                                        span {
                                            style: format!("font-size:12px; font-weight:600; color:{};", snapshot.palette.text),
                                            "{target.label}"
                                        }
                                        span {
                                            style: format!("font-size:11px; line-height:1.35; color:{}; white-space:pre-wrap;", snapshot.palette.muted),
                                            "{target_detail}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if !snapshot.live_sessions.is_empty() {
                div {
                    style: "display:flex; flex-direction:column; gap:8px;",
                    div {
                        style: format!("font-size:11px; font-weight:700; color:{};", snapshot.palette.muted),
                        "Live Sessions"
                    }
                    for session in snapshot.live_sessions.iter().cloned() {
                        button {
                            style: sidebar_action_style(snapshot.palette),
                            onclick: move |_| on_focus_live.call(session.session_path.clone()),
                            div {
                                style: "display:flex; flex-direction:column; align-items:flex-start; gap:3px; min-width:0;",
                                span {
                                    style: format!("font-size:12px; font-weight:600; color:{};", snapshot.palette.text),
                                    "{session.title}"
                                }
                                span {
                                    style: format!("font-size:11px; line-height:1.35; color:{}; white-space:pre-wrap;", snapshot.palette.muted),
                                    "{session.status_line}"
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
    palette: Palette,
    on_close: EventHandler<MouseEvent>,
    on_regenerate: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        div {
            style: "position:fixed; inset:0; z-index:90; background:transparent;",
            onclick: move |evt| on_close.call(evt),
            div {
                style: format!(
                    "position:absolute; top:60px; left:18px; min-width:180px; padding:8px; border-radius:14px; \
                     background:rgba(255,255,255,0.96); box-shadow: 0 18px 44px rgba(69,108,136,0.18);"
                ),
                onmousedown: |evt| evt.stop_propagation(),
                onclick: |evt| evt.stop_propagation(),
                div {
                    style: format!("padding:4px 8px 8px 8px; font-size:11px; font-weight:700; color:{}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;", palette.muted),
                    "{row.label}"
                }
                button {
                    style: format!(
                        "width:100%; height:30px; border:none; border-radius:10px; background:{}; color:{}; \
                         font-size:12px; font-weight:600; text-align:left; padding:0 10px;",
                        palette.accent_soft, palette.text
                    ),
                    onclick: move |evt| on_regenerate.call(evt),
                    "Regenerate Title"
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
    let items = notifications
        .into_iter()
        .rev()
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
                    style: "pointer-events:auto;",
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
fn MetadataGroup(
    title: String,
    entries: Vec<yggterm_core::SessionMetadataEntry>,
    palette: Palette,
) -> Element {
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
        radius, palette.shell, palette.gradient, palette.shadow, interface_font_family()
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
        if selected { palette.accent } else { palette.muted },
        font_size_px,
        if selected { 800 } else { 700 }
    )
}

fn connect_button_style(palette: Palette, selected: bool) -> String {
    format!(
        "height:28px; padding:0 11px; border:none; border-radius:10px; background:transparent; color:{}; \
         font-size:11px; font-weight:700; white-space:nowrap; user-select:none; -webkit-user-select:none; pointer-events:auto;",
        if selected { palette.accent } else { palette.muted }
    )
}

fn chip_style(palette: Palette, selected: bool) -> String {
    format!(
        "height:24px; padding:0 10px; border-radius:999px; border:1px solid {}; background:{}; \
         color:{}; font-size:11px; font-weight:600;",
        if selected { palette.accent } else { "rgba(255,255,255,0.10)" },
        if selected { palette.accent_soft } else { "rgba(255,255,255,0.28)" },
        if selected { palette.text } else { palette.muted }
    )
}

fn sidebar_action_style(palette: Palette) -> String {
    format!(
        "width:100%; border:none; border-radius:12px; background:{}; padding:8px 10px; text-align:left; \
         text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased;",
        palette.sidebar_hover
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
        if selected { palette.panel } else { "transparent" },
        if selected { palette.text } else { palette.muted }
    )
}

fn settings_input_style(palette: Palette) -> String {
    format!(
        "height:30px; padding:0 10px; border:none; border-radius:8px; background:rgba(255,255,255,0.58); \
         color:{}; outline:none; font-size:11px; box-shadow: inset 0 0 0 1px rgba(255,255,255,0.34);",
        palette.text
    )
}

fn notification_tone_colors(tone: NotificationTone, palette: Palette) -> (&'static str, &'static str) {
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

fn clamp_zoom_value(value: f32) -> f32 {
    value.clamp(10.0, 20.0)
}

fn clamp_zoom_value_main(value: f32) -> f32 {
    value.clamp(10.0, 20.0)
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

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn interface_font_family() -> &'static str {
    "system-ui, sans-serif"
}
