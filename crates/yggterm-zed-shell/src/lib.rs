use anyhow::Result;
use assets::Assets;
use editor::{Editor, EditorElement, EditorStyle};
use gpui::{
    AnyElement, App, Bounds, Context, CursorStyle, Decorations, Entity, FocusHandle, Focusable, Global,
    HitboxBehavior, Hsla, KeyBinding, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point,
    ResizeEdge, SharedString, Size, TextStyle, Tiling, Window, WindowBounds, WindowDecorations,
    WindowOptions, actions, canvas, div, point, prelude::*, px, size, transparent_black,
    StatefulInteractiveElement, relative,
};
use gpui_platform::application;
use platform_title_bar::PlatformTitleBar;
use serde::{Deserialize, Serialize};
use settings::Settings;
use std::fs;
use std::path::PathBuf;
use theme::{
    ActiveTheme, Appearance, ThemeRegistry, ThemeSelection, ThemeSettings,
};
use ui::{
    Button, ButtonCommon, ButtonSize, ButtonStyle, Clickable, Color, ContextMenu, Disclosure,
    Disableable, Divider, FixedWidth, Icon, IconButton, IconButtonShape, IconName, IconSize, Label,
    LabelCommon, LabelSize, ListHeader, ListItem, ListItemSpacing, ListSubHeader, PopoverMenu,
    PopoverMenuHandle, StyledExt, Toggleable, Tooltip, h_flex, v_flex,
};
use workspace::CloseWindow;
use yggterm_core::{
    BrowserRow, BrowserRowKind, PreviewTone, SessionMetadataEntry, SessionNode,
    SessionBrowserState, SessionSource, TerminalBackend, UiTheme, WorkspaceViewMode,
    YggtermServer,
};

actions!(
    yggterm_shell,
    [
        CloseCommandPalette,
        FocusSearch,
        OpenCommandPalette,
        SwitchToRenderedView,
        SwitchToTerminalView,
        ToggleSidebar,
        ToggleThemeMode
    ]
);

#[derive(Debug, Clone)]
pub struct ZedShellPlan {
    pub uses_upstream_workspace_item: bool,
    pub uses_upstream_project_panel: bool,
    pub uses_upstream_terminal_view: bool,
    pub center_viewport_replaced_by_terminals: bool,
    pub uses_gpui_shell_scaffold: bool,
    pub uses_virtual_session_tree: bool,
    pub integrates_ghostty_bridge_status: bool,
}

impl Default for ZedShellPlan {
    fn default() -> Self {
        Self {
            uses_upstream_workspace_item: true,
            uses_upstream_project_panel: true,
            uses_upstream_terminal_view: true,
            center_viewport_replaced_by_terminals: true,
            uses_gpui_shell_scaffold: true,
            uses_virtual_session_tree: true,
            integrates_ghostty_bridge_status: true,
        }
    }
}

pub fn shell_plan() -> ZedShellPlan {
    ZedShellPlan::default()
}

#[derive(Debug, Clone)]
pub struct ShellBootstrap {
    pub tree: SessionNode,
    pub theme: UiTheme,
    pub ghostty_bridge_enabled: bool,
    pub prefer_ghostty_backend: bool,
}

pub fn launch_gpui_shell(bootstrap: ShellBootstrap) -> Result<()> {
    let ui_config = ShellUiConfig::load().unwrap_or_default();
    application().with_assets(Assets).run(move |cx: &mut App| {
        component::init();
        settings::init(cx);
        PlatformTitleBar::init(cx);
        theme::init(theme::LoadThemes::All(Box::new(Assets)), cx);
        Assets
            .load_fonts(cx)
            .expect("failed to load bundled Zed fonts");
        cx.bind_keys([
            KeyBinding::new("ctrl-shift-p", OpenCommandPalette, None),
            KeyBinding::new("ctrl-l", FocusSearch, None),
            KeyBinding::new("escape", CloseCommandPalette, None),
        ]);
        apply_initial_theme(&ui_config, bootstrap.theme, cx);

        let bounds = Bounds::centered(None, size(px(1460.), px(920.)), cx);
        let shell = bootstrap.clone();
        let shell_ui_config = ui_config.clone();

        cx.open_window(
            WindowOptions {
                titlebar: Some(gpui::TitlebarOptions {
                    title: None,
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(9.), px(9.))),
                }),
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_background: cx.theme().window_background_appearance(),
                window_decorations: Some(WindowDecorations::Client),
                window_min_size: Some(size(px(1024.), px(720.))),
                app_id: Some("dev.yggterm".into()),
                ..Default::default()
            },
            move |window, cx| {
                theme::setup_ui_font(window, cx);
                cx.new(|cx| GpuiShell::new(shell.clone(), shell_ui_config.clone(), window, cx))
            },
        )
        .expect("failed to open GPUI shell window");

        cx.activate(true);
    });

    Ok(())
}

#[cfg(feature = "zed-upstream")]
pub fn upstream_type_markers() -> [&'static str; 6] {
    [
        "gpui::App",
        "ui::TabBar",
        "ui::ListItem",
        "settings::SettingsStore",
        "theme::ThemeSettings",
        "workspace::Workspace",
    ]
}

#[cfg(not(feature = "zed-upstream"))]
pub fn upstream_type_markers() -> [&'static str; 5] {
    [
        "gpui::App",
        "ui::TabBar",
        "ui::ListItem",
        "settings::SettingsStore",
        "theme::ThemeSettings",
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShellUiConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    theme_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    theme_mode: Option<String>,
    right_panel_open: bool,
    bottom_panel_open: bool,
}

impl Default for ShellUiConfig {
    fn default() -> Self {
        Self {
            theme_name: None,
            theme_mode: None,
            right_panel_open: true,
            bottom_panel_open: false,
        }
    }
}

impl ShellUiConfig {
    fn path() -> PathBuf {
        dirs::config_dir()
            .or_else(|| dirs::home_dir().map(|home| home.join(".config")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("yggterm")
            .join("settings.toml")
    }

    fn load() -> Result<Self> {
        let path = Self::path();
        if !path.exists() {
            return Ok(Self::default());
        }

        Ok(toml::from_str(&fs::read_to_string(path)?)?)
    }

    fn save(&self) -> Result<()> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

fn apply_initial_theme(ui_config: &ShellUiConfig, fallback_theme: UiTheme, cx: &mut App) {
    if let Some(theme_name) = ui_config.theme_name.as_deref() {
        set_theme_name(theme_name, cx);
        return;
    }

    apply_theme_selection(fallback_theme, cx);
}

fn apply_theme_selection(theme_name: UiTheme, cx: &mut App) {
    let theme_name = match theme_name {
        UiTheme::ZedDark => theme::default_theme(Appearance::Dark),
        UiTheme::ZedLight => theme::default_theme(Appearance::Light),
    };
    set_theme_name(theme_name, cx);
}

fn set_theme_name(theme_name: &str, cx: &mut App) {
    let mut theme_settings = ThemeSettings::get_global(cx).clone();
    theme_settings.theme = ThemeSelection::Static(theme::ThemeName(theme_name.into()));
    ThemeSettings::override_global(theme_settings, cx);
}

fn toggle_theme_mode(cx: &mut App) {
    let next_theme_name = match cx.theme().appearance {
        Appearance::Light => theme::default_theme(Appearance::Dark),
        Appearance::Dark => theme::default_theme(Appearance::Light),
    };
    set_theme_name(next_theme_name, cx);
}

fn persist_theme_selection(theme_name: Option<String>) {
    let mut config = ShellUiConfig::load().unwrap_or_default();
    config.theme_name = theme_name;
    config.theme_mode = None;
    let _ = config.save();
}

fn set_theme_name_and_save(theme_name: &str, cx: &mut App) {
    set_theme_name(theme_name, cx);
    persist_theme_selection(Some(theme_name.to_string()));
}

fn toggle_theme_mode_and_save(cx: &mut App) {
    toggle_theme_mode(cx);
    persist_theme_selection(Some(cx.theme().name.to_string()));
}

fn available_theme_names(cx: &App) -> Vec<String> {
    let mut themes = ThemeRegistry::global(cx)
        .list()
        .into_iter()
        .map(|theme| theme.name.to_string())
        .collect::<Vec<_>>();
    themes.sort();
    themes
}

fn open_settings_window_global(cx: &mut App) {
    let _ = cx.open_window(
        WindowOptions {
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Yggterm Settings".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(12.), px(12.))),
            }),
            window_bounds: Some(WindowBounds::centered(size(px(980.), px(760.)), cx)),
            window_background: cx.theme().window_background_appearance(),
            window_decorations: Some(WindowDecorations::Client),
            app_id: Some("dev.yggterm.settings".into()),
            ..Default::default()
        },
        |window, cx| {
            theme::setup_ui_font(window, cx);
            cx.new(|cx| SettingsWindow::new(cx))
        },
    );
}

#[derive(Debug, Clone)]
struct DockEntry {
    title: &'static str,
    body: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DragTarget {
    Sidebar,
    RightPanel,
    BottomDock,
}

#[derive(Debug, Clone, Copy)]
struct PanelDrag {
    target: DragTarget,
    start_mouse: Point<Pixels>,
    start_size: Pixels,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RightPanel {
    Metadata,
    Extensions,
}

#[derive(Clone)]
struct GpuiShell {
    bootstrap: ShellBootstrap,
    ui_config: ShellUiConfig,
    focus_handle: FocusHandle,
    titlebar: Entity<PlatformTitleBar>,
    chrome_menu_handle: PopoverMenuHandle<ContextMenu>,
    search_editor: Entity<Editor>,
    palette_editor: Entity<Editor>,
    browser: SessionBrowserState,
    server: YggtermServer,
    dock_entries: Vec<DockEntry>,
    selected_dock_ix: usize,
    sidebar_open: bool,
    bottom_panel_open: bool,
    right_panel_open: bool,
    command_palette_open: bool,
    selected_right_panel: RightPanel,
    sidebar_width: Pixels,
    right_panel_width: Pixels,
    bottom_dock_height: Pixels,
    active_panel_drag: Option<PanelDrag>,
    last_action: String,
}

impl GpuiShell {
    fn new(
        bootstrap: ShellBootstrap,
        ui_config: ShellUiConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let browser = SessionBrowserState::new(bootstrap.tree.clone());
        let focus_handle = cx.focus_handle();
        let titlebar = cx.new(|cx| PlatformTitleBar::new("yggterm-platform-titlebar", cx));
        let search_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Search sessions…", window, cx);
            editor
        });
        let palette_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Search commands and themes…", window, cx);
            editor
        });
        focus_handle.focus(window, cx);
        let server = YggtermServer::new(
            &bootstrap.tree,
            bootstrap.prefer_ghostty_backend,
            bootstrap.ghostty_bridge_enabled,
            bootstrap.theme,
        );

        let dock_entries = vec![
            DockEntry {
                title: "Logs",
                body: "Server-managed session attach, Ghostty host lifecycle, and restore events will land here.",
            },
            DockEntry {
                title: "Clipboard",
                body: "Image and text paste transport for remote sessions will be surfaced through the server host.",
            },
            DockEntry {
                title: "State",
                body: "Session metadata under ~/.yggterm backs the browser tree, server restore state, and future multiplexing.",
            },
        ];

        Self {
            bootstrap,
            ui_config: ui_config.clone(),
            focus_handle,
            titlebar,
            chrome_menu_handle: PopoverMenuHandle::default(),
            search_editor,
            palette_editor,
            browser,
            server,
            dock_entries,
            selected_dock_ix: 0,
            sidebar_open: true,
            bottom_panel_open: ui_config.bottom_panel_open,
            right_panel_open: ui_config.right_panel_open,
            command_palette_open: false,
            selected_right_panel: RightPanel::Metadata,
            sidebar_width: px(300.),
            right_panel_width: px(286.),
            bottom_dock_height: px(168.),
            active_panel_drag: None,
            last_action: "ready".to_string(),
        }
    }

    fn selected_row(&self) -> Option<&BrowserRow> {
        self.browser.selected_row()
    }

    fn active_session(&self) -> Option<&yggterm_core::ManagedSessionView> {
        self.server.active_session()
    }

    fn selected_dock(&self) -> &DockEntry {
        &self.dock_entries[self.selected_dock_ix.min(self.dock_entries.len().saturating_sub(1))]
    }

    fn set_last_action(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.last_action = message.into();
        cx.notify();
    }

    fn save_ui_config(&mut self) {
        self.ui_config.right_panel_open = self.right_panel_open;
        self.ui_config.bottom_panel_open = self.bottom_panel_open;
        let _ = self.ui_config.save();
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        self.titlebar.update(cx, |titlebar, cx| {
            titlebar.set_workspace_sidebar_open(self.sidebar_open, cx);
        });
        self.set_last_action(
            if self.sidebar_open {
                "sidebar opened"
            } else {
                "sidebar collapsed"
            },
            cx,
        );
    }

    fn select_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.browser.rows().get(ix).cloned() {
            match row.kind {
                BrowserRowKind::Group => {
                    self.browser.toggle_group(&row.full_path);
                    self.set_last_action(format!("toggle group {}", row.label), cx);
                }
                BrowserRowKind::Session => {
                    self.browser.select_path(row.full_path.clone());
                    self.server.open_or_focus_session(&row.full_path);
                    self.server.set_view_mode(WorkspaceViewMode::Rendered);
                    self.set_last_action(format!("session {}", row.label), cx);
                }
            }
        }
    }

    fn select_dock(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.dock_entries.len() {
            self.bottom_panel_open = if self.selected_dock_ix == ix {
                !self.bottom_panel_open
            } else {
                true
            };
            self.selected_dock_ix = ix;
            self.set_last_action(format!("dock {}", self.dock_entries[ix].title), cx);
            self.save_ui_config();
        }
    }

    fn toggle_right_panel(&mut self, panel: RightPanel, cx: &mut Context<Self>) {
        if self.selected_right_panel == panel {
            self.right_panel_open = !self.right_panel_open;
        } else {
            self.selected_right_panel = panel;
            self.right_panel_open = true;
        }
        self.set_last_action(
            if self.right_panel_open {
                match self.selected_right_panel {
                    RightPanel::Metadata => "right panel metadata",
                    RightPanel::Extensions => "right panel extensions",
                }
            } else {
                "right panel closed"
            },
            cx,
        );
        self.save_ui_config();
    }

    fn set_view_mode(&mut self, mode: WorkspaceViewMode, cx: &mut Context<Self>) {
        self.server.set_view_mode(mode);
        self.set_last_action(
            match mode {
                WorkspaceViewMode::Terminal => "terminal view",
                WorkspaceViewMode::Rendered => "rendered view",
            },
            cx,
        );
    }

    fn toggle_preview_block(&mut self, block_ix: usize, cx: &mut Context<Self>) {
        self.server.toggle_preview_block(block_ix);
        self.set_last_action(format!("preview block {}", block_ix + 1), cx);
    }

    fn connect_ssh_target(&mut self, target_ix: usize, cx: &mut Context<Self>) {
        if let Some(session_key) = self.server.connect_ssh_target(target_ix) {
            self.set_last_action(format!("ssh session {}", session_key.trim_start_matches("live::")), cx);
        }
    }

    fn focus_live_session(&mut self, session_key: &str, cx: &mut Context<Self>) {
        self.server.focus_live_session(session_key);
        self.set_last_action(
            format!("live {}", session_key.trim_start_matches("live::")),
            cx,
        );
    }

    fn set_all_preview_blocks_folded(&mut self, folded: bool, cx: &mut Context<Self>) {
        self.server.set_all_preview_blocks_folded(folded);
        self.set_last_action(
            if folded {
                "preview collapsed"
            } else {
                "preview expanded"
            },
            cx,
        );
    }

    fn preview_query(&self) -> &str {
        self.browser.filter_query()
    }

    fn preview_block_matches(
        &self,
        block: &yggterm_core::SessionPreviewBlock,
        query: &str,
    ) -> bool {
        if query.is_empty() {
            return true;
        }
        let query = query.to_ascii_lowercase();
        block.role.to_ascii_lowercase().contains(&query)
            || block.timestamp.to_ascii_lowercase().contains(&query)
            || block
                .lines
                .iter()
                .any(|line| line.to_ascii_lowercase().contains(&query))
    }

    fn set_browser_filter(&mut self, query: impl Into<String>, cx: &mut Context<Self>) {
        let query = query.into();
        self.browser.set_filter_query(query.clone());
        self.set_last_action(
            if query.is_empty() {
                "session filter cleared".to_string()
            } else {
                format!("session filter {query}")
            },
            cx,
        );
    }

    fn clear_browser_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_editor.update(cx, |editor, cx| {
            editor.set_text("", window, cx);
        });
        self.set_browser_filter("", cx);
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_editor
            .update(cx, |editor, cx| window.focus(&editor.focus_handle(cx), cx));
        self.set_last_action("focus search", cx);
    }

    fn sync_browser_filter_from_editor(&mut self, cx: &App) {
        let query = self.search_editor.read(cx).text(cx);
        if query != self.browser.filter_query() {
            self.browser.set_filter_query(query);
        }
    }

    fn focus_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette_editor
            .update(cx, |editor, cx| window.focus(&editor.focus_handle(cx), cx));
    }

    fn palette_query(&self, cx: &App) -> String {
        self.palette_editor.read(cx).text(cx)
    }

    fn open_settings_window(&mut self, cx: &mut Context<Self>) {
        self.command_palette_open = false;
        self.set_last_action("settings window", cx);
        cx.defer(open_settings_window_global);
    }

    fn toggle_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.command_palette_open = !self.command_palette_open;
        if self.command_palette_open {
            self.focus_palette(window, cx);
        }
        self.set_last_action(
            if self.command_palette_open {
                "command palette"
            } else {
                "command palette dismissed"
            },
            cx,
        );
    }

    fn dismiss_command_palette(&mut self, cx: &mut Context<Self>) {
        if self.command_palette_open {
            self.command_palette_open = false;
            self.set_last_action("command palette dismissed", cx);
        }
    }

    fn begin_panel_drag(
        &mut self,
        target: DragTarget,
        start_mouse: Point<Pixels>,
        start_size: Pixels,
        cx: &mut Context<Self>,
    ) {
        self.active_panel_drag = Some(PanelDrag {
            target,
            start_mouse,
            start_size,
        });
        self.set_last_action("pane resize", cx);
    }

    fn update_panel_drag(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.active_panel_drag else {
            return;
        };

        match drag.target {
            DragTarget::Sidebar => {
                let delta = position.x - drag.start_mouse.x;
                self.sidebar_width = (drag.start_size + delta).clamp(px(220.), px(420.));
            }
            DragTarget::RightPanel => {
                let delta = drag.start_mouse.x - position.x;
                self.right_panel_width = (drag.start_size + delta).clamp(px(220.), px(520.));
            }
            DragTarget::BottomDock => {
                let delta = drag.start_mouse.y - position.y;
                self.bottom_dock_height = (drag.start_size + delta).clamp(px(120.), px(420.));
            }
        }

        window.refresh();
        cx.notify();
    }

    fn end_panel_drag(&mut self, cx: &mut Context<Self>) {
        if self.active_panel_drag.take().is_some() {
            self.set_last_action("pane resize complete", cx);
        }
    }

    fn active_mode_label(&self) -> &'static str {
        match self.server.backend() {
            TerminalBackend::Ghostty => "Ghostty host",
            TerminalBackend::Mock => "Mock host",
        }
    }

    fn total_leaf_sessions(&self) -> usize {
        self.browser.total_sessions()
    }

    fn active_theme_label(&self, cx: &App) -> SharedString {
        cx.theme().name.clone()
    }

    fn titlebar_children(&self, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        vec![
            h_flex()
                .w(px(520.))
                .flex_none()
                .items_center()
                .justify_start()
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            self.chrome_icon(
                                "toggle-nav",
                                IconName::WorkspaceNavOpen,
                                self.sidebar_open,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
                        )
                        .child(self.chrome_menu(cx)),
                )
                .into_any_element(),
            h_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .child(self.titlebar_search(window, cx))
                .into_any_element(),
            h_flex()
                .w(px(520.))
                .flex_none()
                .items_center()
                .justify_end()
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            Button::new("titlebar-restore-all", "Restore All")
                                .style(ButtonStyle::Subtle)
                                .size(ButtonSize::Compact)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_last_action("restore all", cx)
                                })),
                        )
                        .child(
                            Button::new("titlebar-paste-screenshot", "Paste Screenshot")
                                .style(ButtonStyle::Subtle)
                                .size(ButtonSize::Compact)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_last_action("paste screenshot", cx)
                                })),
                        )
                        .child(
                            Button::new(
                                "view-mode-toggle",
                                match self.server.active_view_mode() {
                                    WorkspaceViewMode::Terminal => "Terminal",
                                    WorkspaceViewMode::Rendered => "Rendered",
                                },
                            )
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Compact)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let next_mode = match this.server.active_view_mode() {
                                    WorkspaceViewMode::Terminal => WorkspaceViewMode::Rendered,
                                    WorkspaceViewMode::Rendered => WorkspaceViewMode::Terminal,
                                };
                                this.set_view_mode(next_mode, cx);
                            })),
                        )
                        .child(
                            self.chrome_icon(
                                "metadata-panel",
                                IconName::Info,
                                self.right_panel_open
                                    && self.selected_right_panel == RightPanel::Metadata,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_right_panel(RightPanel::Metadata, cx)
                            })),
                        )
                        .child(
                            self.chrome_icon(
                                "extensions-panel",
                                IconName::Code,
                                self.right_panel_open
                                    && self.selected_right_panel == RightPanel::Extensions,
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_right_panel(RightPanel::Extensions, cx)
                            })),
                        )
                        .child(
                            Label::new(self.active_theme_label(cx))
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                        .child(
                            self.chrome_icon("window-settings", IconName::Settings, false)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.open_settings_window(cx)
                                })),
                        ),
                )
                .into_any_element(),
        ]
    }

    fn window_chrome(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let children = self.titlebar_children(window, cx);
        self.titlebar.update(cx, |titlebar, cx| {
            titlebar.set_workspace_sidebar_open(self.sidebar_open, cx);
            titlebar.set_children(children);
        });
        self.titlebar.clone().into_any_element()
    }

    fn chrome_menu(&self, cx: &mut Context<Self>) -> PopoverMenu<ContextMenu> {
        let is_dark = matches!(cx.theme().appearance, Appearance::Dark);

        PopoverMenu::new("yggterm-chrome-menu")
            .with_handle(self.chrome_menu_handle.clone())
            .anchor(gpui::Corner::BottomLeft)
            .trigger_with_tooltip(
                self.chrome_icon("window-menu", IconName::Menu, self.chrome_menu_handle.is_deployed()),
                Tooltip::text("Open Window Menu"),
            )
            .menu({
                let active_theme = self.active_theme_label(cx);
                move |window, cx| {
                    Some(ContextMenu::build(window, cx, {
                        let active_theme = active_theme.clone();
                        move |menu, _, _| {
                            menu.entry("Toggle Sidebar", None, |window, cx| {
                                window.dispatch_action(Box::new(ToggleSidebar), cx)
                            })
                            .separator()
                            .entry(
                                if is_dark {
                                    "Switch to Light Theme"
                                } else {
                                    "Switch to Dark Theme"
                                },
                                None,
                                |_, cx| toggle_theme_mode_and_save(cx),
                            )
                            .entry("Open Window Menu", None, move |window, _| {
                                window.show_window_menu(point(px(28.), px(28.)))
                            })
                            .separator()
                            .entry(
                                format!("Active Theme: {active_theme}"),
                                None,
                                |_window, _cx| {},
                            )
                        }
                    }))
                }
            })
    }

    fn titlebar_search(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let settings = ThemeSettings::get_global(cx);
        let colors = cx.theme().colors();
        let is_focused = self.search_editor.read(cx).is_focused(window);
        let text_style = TextStyle {
            color: colors.text,
            font_family: settings.buffer_font.family.clone(),
            font_features: settings.buffer_font.features.clone(),
            font_fallbacks: settings.buffer_font.fallbacks.clone(),
            font_size: ui::rems(0.875).into(),
            font_weight: settings.buffer_font.weight,
            ..TextStyle::default()
        };
        let editor_style = EditorStyle {
            background: colors.toolbar_background,
            local_player: cx.theme().players().local(),
            text: text_style,
            syntax: cx.theme().syntax().clone(),
            ..EditorStyle::default()
        };

        h_flex()
            .w(px(392.))
            .h(px(26.))
            .items_center()
            .gap_2()
            .px_2()
            .on_mouse_down(
                gpui::MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.focus_search(window, cx);
                    cx.stop_propagation();
                }),
            )
            .rounded_md()
            .border_1()
            .border_color(if is_focused {
                colors.border_focused
            } else {
                colors.border_variant
            })
            .bg(colors.toolbar_background)
            .child(
                Icon::new(IconName::MagnifyingGlass)
                    .size(IconSize::Small)
                    .color(Color::Muted),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .child(EditorElement::new(&self.search_editor, editor_style)),
            )
            .child(
                IconButton::new(
                    "titlebar-search-clear",
                    if self.browser.filter_query().is_empty() {
                        IconName::ListFilter
                    } else {
                        IconName::Close
                    },
                )
                .shape(IconButtonShape::Square)
                .icon_size(IconSize::XSmall)
                .icon_color(Color::Muted)
                .style(ButtonStyle::Transparent)
                .disabled(self.browser.filter_query().is_empty())
                .tooltip(Tooltip::text("Clear Search"))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.clear_browser_filter(window, cx)
                })),
            )
            .into_any_element()
    }

    fn sidebar(&self, cx: &mut Context<Self>, colors: &theme::ThemeColors) -> AnyElement {
        if !self.sidebar_open {
            return div().into_any_element();
        }

        let ssh_targets = self.server.ssh_targets().to_vec();
        let live_sessions = self.server.live_sessions();
        let rows = self
            .browser
            .rows()
            .iter()
            .enumerate()
            .map(|(ix, row)| self.session_tree_row(ix, row, cx))
            .collect::<Vec<_>>();

        div()
            .w(self.sidebar_width)
            .h_full()
            .flex()
            .flex_col()
            .bg(colors.panel_background)
            .border_r_1()
            .border_color(colors.border)
            .child(
                h_flex()
                    .h(px(42.))
                    .items_center()
                    .justify_between()
                    .px_2()
                    .child(
                        Label::new("Remote Session Tree")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .child(
                        self.chrome_icon("sidebar-filter", IconName::MagnifyingGlass, false)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_last_action("session filter", cx)
                            })),
                    ),
            )
            .child(
                div()
                    .px_2()
                    .pt_1()
                    .pb_1()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                ListHeader::new("yggterm")
                                    .start_slot(Icon::new(IconName::FolderOpen).size(IconSize::Small))
                                    .end_slot(
                                        h_flex()
                                            .gap_1()
                                            .child(
                                                self.chrome_icon("new-session", IconName::Plus, false)
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        this.set_last_action("new session", cx)
                                                    })),
                                            )
                                            .child(
                                                self.chrome_icon(
                                                    "session-filter",
                                                    IconName::ListFilter,
                                                    false,
                                                )
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.set_last_action("session filter", cx)
                                                })),
                                            ),
                                    ),
                            )
                            .child(div()),
                    )
                    .child(
                        Button::new(
                            "mock-filter",
                            if self.browser.filter_query().is_empty() {
                                "Filter virtual sessions".to_string()
                            } else {
                                self.browser.filter_query().to_string()
                            },
                        )
                            .icon(IconName::MagnifyingGlass)
                            .style(ButtonStyle::Subtle)
                            .full_width()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.focus_search(window, cx)
                            })),
                    )
                    .child(
                        ListSubHeader::new("Remote Session Tree")
                            .left_icon(Some(IconName::TerminalGhost))
                            .end_slot(
                                Label::new(self.total_leaf_sessions().to_string())
                                    .size(LabelSize::Small)
                                    .color(Color::Muted)
                                    .into_any_element(),
                            ),
                    )
                    .into_any_element(),
            )
            .child(Divider::horizontal())
            .child(
                v_flex()
                    .flex_1()
                    .id("session-tree-scroll")
                    .overflow_y_scroll()
                    .px_1()
                    .py_2()
                    .gap_1()
                    .children(rows),
            )
            .child(Divider::horizontal())
            .child(
                div()
                    .px_2()
                    .py_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        ListSubHeader::new("Restore Preview")
                            .left_icon(Some(IconName::HistoryRerun))
                            .end_slot(
                                Label::new("last_session")
                                    .size(LabelSize::Small)
                                    .color(Color::Muted)
                                    .into_any_element(),
                            ),
                    )
                    .child(
                        ListItem::new("restore-group")
                            .spacing(ListItemSpacing::Dense)
                            .start_slot(Icon::new(IconName::RotateCcw).size(IconSize::Small))
                            .child(Label::new("workspace-2026-03-17").size(LabelSize::Small))
                            .end_slot(
                                Label::new("4 panes")
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.select_dock(2, cx)
                            })),
                    )
                    .child(
                        Label::new(
                            "This mock tree is the future filesystem-backed session model under ~/.yggterm.",
                        )
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                    )
                    .child(Divider::horizontal().inset())
                    .child(
                        ListSubHeader::new("Connect SSH")
                            .left_icon(Some(IconName::Server))
                            .end_slot(
                                Label::new(ssh_targets.len().to_string())
                                    .size(LabelSize::Small)
                                    .color(Color::Muted)
                                    .into_any_element(),
                            ),
                    )
                    .children(
                        ssh_targets
                            .into_iter()
                            .enumerate()
                            .map(|(ix, target)| {
                                ListItem::new(format!("ssh-target-{ix}"))
                                    .spacing(ListItemSpacing::Dense)
                                    .start_slot(Icon::new(IconName::ArrowUpRight).size(IconSize::Small))
                                    .child(Label::new(target.label).size(LabelSize::Small))
                                    .end_slot(
                                        Label::new(
                                            target
                                                .prefix
                                                .clone()
                                                .unwrap_or_else(|| String::from("plain ssh")),
                                        )
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.connect_ssh_target(ix, cx);
                                    }))
                                    .into_any_element()
                            })
                            .collect::<Vec<_>>(),
                    )
                    .child(Divider::horizontal().inset())
                    .child(
                        ListSubHeader::new("Live Sessions")
                            .left_icon(Some(IconName::ToolTerminal))
                            .end_slot(
                                Label::new(live_sessions.len().to_string())
                                    .size(LabelSize::Small)
                                    .color(Color::Muted)
                                    .into_any_element(),
                            ),
                    )
                    .children(
                        live_sessions
                            .into_iter()
                            .map(|session| {
                                let session_key = format!("live::{}", session.id);
                                let session_id = session.id.clone();
                                ListItem::new(format!("live-session-{}", session.id))
                                    .spacing(ListItemSpacing::Dense)
                                    .start_slot(Icon::new(IconName::TerminalGhost).size(IconSize::Small))
                                    .child(Label::new(session_id).size(LabelSize::Small))
                                    .end_slot(
                                        Label::new(session.host_label)
                                            .size(LabelSize::Small)
                                            .color(Color::Muted),
                                    )
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.focus_live_session(&session_key, cx);
                                    }))
                                    .into_any_element()
                            })
                            .collect::<Vec<_>>(),
                    )
                    .into_any_element(),
            )
            .into_any_element()
    }

    fn workspace(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
        colors: &theme::ThemeColors,
    ) -> AnyElement {
        div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .bg(colors.surface_background)
            .child(
                div()
                    .flex_1()
                    .flex()
                    .overflow_hidden()
                    .child(self.primary_body(cx, colors)),
            )
            .when(self.bottom_panel_open, |this| {
                this.child(self.bottom_dock_resize_handle(cx, colors))
                    .child(self.bottom_dock(cx, colors))
            })
            .child(self.status_bar(cx, colors))
            .into_any_element()
    }

    fn primary_body(&self, cx: &mut Context<Self>, colors: &theme::ThemeColors) -> AnyElement {
        div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(
                div()
                    .flex_1()
                    .flex()
                    .overflow_hidden()
                    .child(self.viewport(cx, colors))
                    .when(self.right_panel_open, |this| {
                        this.child(self.right_panel_resize_handle(cx, colors))
                            .child(self.inspector(colors))
                    }),
            )
            .into_any_element()
    }

    fn sidebar_resize_handle(
        &self,
        cx: &mut Context<Self>,
        colors: &theme::ThemeColors,
    ) -> AnyElement {
        div()
            .w(px(6.))
            .h_full()
            .cursor(CursorStyle::ResizeLeftRight)
            .flex_shrink_0()
            .bg(colors.surface_background)
            .child(div().mx_auto().w(px(1.)).h_full().bg(colors.border))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| {
                this.begin_panel_drag(DragTarget::Sidebar, event.position, this.sidebar_width, cx);
                cx.stop_propagation();
            }))
            .into_any_element()
    }

    fn right_panel_resize_handle(
        &self,
        cx: &mut Context<Self>,
        colors: &theme::ThemeColors,
    ) -> AnyElement {
        div()
            .w(px(6.))
            .h_full()
            .cursor(CursorStyle::ResizeLeftRight)
            .flex_shrink_0()
            .bg(colors.surface_background)
            .child(div().mx_auto().w(px(1.)).h_full().bg(colors.border))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| {
                this.begin_panel_drag(
                    DragTarget::RightPanel,
                    event.position,
                    this.right_panel_width,
                    cx,
                );
                cx.stop_propagation();
            }))
            .into_any_element()
    }

    fn viewport(&self, cx: &mut Context<Self>, colors: &theme::ThemeColors) -> AnyElement {
        let selected_path = self
            .active_session()
            .map(|session| match session.source {
                SessionSource::Stored => session.session_path.clone(),
                SessionSource::LiveSsh => format!("{} · {}", session.id, session.host_label),
            })
            .or_else(|| self.selected_row().map(|row| row.full_path.clone()))
            .unwrap_or_else(|| "~/.yggterm/sessions".to_string());
        let active_session = self.active_session();
        let preview_query = self.preview_query().trim().to_string();
        let mode_label = match self.server.active_view_mode() {
            WorkspaceViewMode::Terminal => "Terminal View",
            WorkspaceViewMode::Rendered => "Rendered View",
        };

        div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .bg(colors.editor_background)
            .child(
                div()
                    .w_full()
                    .h(px(34.))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .bg(colors.surface_background)
                    .border_b_1()
                    .border_color(colors.border)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                Icon::new(match self.server.active_view_mode() {
                                    WorkspaceViewMode::Terminal => IconName::TerminalGhost,
                                    WorkspaceViewMode::Rendered => IconName::BookCopy,
                                })
                                .size(IconSize::Small),
                            )
                            .child(
                                Label::new(selected_path)
                                    .size(LabelSize::Small)
                                    .color(Color::Default),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .when(
                                self.server.active_view_mode() == WorkspaceViewMode::Rendered,
                                |this| {
                                    this.child(
                                        Button::new("preview-expand-all", "Expand All")
                                            .style(ButtonStyle::Subtle)
                                            .size(ButtonSize::Compact)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.set_all_preview_blocks_folded(false, cx);
                                            })),
                                    )
                                    .child(
                                        Button::new("preview-collapse-all", "Collapse All")
                                            .style(ButtonStyle::Subtle)
                                            .size(ButtonSize::Compact)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.set_all_preview_blocks_folded(true, cx);
                                            })),
                                    )
                                },
                            )
                            .when(!preview_query.is_empty(), |this| {
                                this.child(
                                    Button::new(
                                        "preview-clear-query",
                                        format!("Matches: {}", preview_query),
                                    )
                                    .style(ButtonStyle::Subtle)
                                    .size(ButtonSize::Compact)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.clear_browser_filter(window, cx);
                                    })),
                                )
                            })
                            .child(
                                Button::new("runtime-mode", mode_label)
                                    .icon(IconName::ToolTerminal)
                                    .style(ButtonStyle::Subtle)
                                    .size(ButtonSize::Compact)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let next_mode = match this.server.active_view_mode() {
                                            WorkspaceViewMode::Terminal => WorkspaceViewMode::Rendered,
                                            WorkspaceViewMode::Rendered => WorkspaceViewMode::Terminal,
                                        };
                                        this.set_view_mode(next_mode, cx);
                                    })),
                            )
                            .child(
                                self.chrome_icon("viewport-menu", IconName::Ellipsis, false)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.set_last_action("viewport menu", cx)
                                    })),
                            ),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .id("viewport-scroll")
                    .overflow_y_scroll()
                    .px_4()
                    .py_3()
                    .gap_3()
                    .children(match active_session {
                        Some(session) if self.server.active_view_mode() == WorkspaceViewMode::Terminal => vec![
                            session_block(
                                match session.source {
                                    SessionSource::Stored => "Server Terminal",
                                    SessionSource::LiveSsh => "Ghostty Launch Request",
                                },
                                &session.terminal_lines,
                                Some(&session.status_line),
                                colors,
                            ),
                            session_block(
                                "Ghostty Integration",
                                &[
                                    if self.bootstrap.ghostty_bridge_enabled {
                                        "libghostty bridge is available in this build."
                                    } else {
                                        "libghostty bridge is not linked in this build."
                                    },
                                    "The shell color scheme is synchronized to the active Zed light/dark theme.",
                                    "The Yggterm server owns terminal launch requests and would hand live sessions to Ghostty surfaces here.",
                                ],
                                Some(self.active_mode_label()),
                                colors,
                            ),
                        ],
                        Some(session) => {
                            let matching_blocks = session
                                .preview
                                .blocks
                                .iter()
                                .enumerate()
                                .filter(|(_, block)| {
                                    self.preview_block_matches(block, preview_query.as_str())
                                })
                                .collect::<Vec<_>>();
                            let mut blocks = Vec::new();
                            blocks.push(session_preview_summary(
                                &session.preview.summary,
                                preview_query.as_str(),
                                matching_blocks.len(),
                                session.preview.blocks.len(),
                                colors,
                            ));
                            blocks.extend(
                                matching_blocks.into_iter().map(|(ix, block)| {
                                    self.session_preview_block_element(ix, block, preview_query.as_str(), cx, colors)
                                }),
                            );
                            if blocks.len() == 1 {
                                blocks.push(session_block(
                                    "No Preview Matches",
                                    &[
                                        "The active search query does not match this session preview.",
                                        "Clear the search field to return to the full transcript.",
                                    ],
                                    Some("search"),
                                    colors,
                                ));
                            }
                            blocks
                        }
                        None => vec![session_block(
                            "No Session Selected",
                            &[
                                "Select a saved session from the sidebar to open its rendered preview first.",
                                "The terminal surface lives alongside the preview and will take over this viewport once Ghostty is embedded.",
                            ],
                            Some("idle"),
                            colors,
                        )],
                    }),
            )
            .into_any_element()
    }

    fn inspector(&self, colors: &theme::ThemeColors) -> AnyElement {
        let metadata_rows = self
            .active_session()
            .map(|session| session.metadata.clone())
            .unwrap_or_else(|| {
                vec![
                    SessionMetadataEntry {
                        label: "Host",
                        value: "n/a".to_string(),
                    },
                    SessionMetadataEntry {
                        label: "Session",
                        value: "none selected".to_string(),
                    },
                ]
            });

        div()
            .w(self.right_panel_width)
            .h_full()
            .flex()
            .flex_col()
            .bg(colors.panel_background)
            .child(
                div()
                    .px_3()
                    .py_2()
                    .child(
                        ListHeader::new(match self.selected_right_panel {
                            RightPanel::Metadata => "Session Metadata",
                            RightPanel::Extensions => "Session Extensions",
                        })
                        .start_slot(
                            Icon::new(match self.selected_right_panel {
                                RightPanel::Metadata => IconName::Info,
                                RightPanel::Extensions => IconName::Code,
                            })
                            .size(IconSize::Small),
                        ),
                    ),
            )
            .child(Divider::horizontal())
            .child(
                v_flex()
                    .flex_1()
                    .id("inspector-scroll")
                    .overflow_y_scroll()
                    .px_2()
                    .py_2()
                    .gap_1()
                    .children(match self.selected_right_panel {
                        RightPanel::Metadata => metadata_rows
                            .iter()
                            .map(|entry| metadata_row(entry.label, entry.value.clone()))
                            .chain([
                                Divider::horizontal().inset().into_any_element(),
                                ListSubHeader::new("Next Lift")
                                    .left_icon(Some(IconName::ToolHammer))
                                    .end_slot(
                                        Label::new("from Zed")
                                            .size(LabelSize::Small)
                                            .color(Color::Muted)
                                            .into_any_element(),
                                    )
                                    .into_any_element(),
                                Label::new(
                                    "The sidebar now follows a codex-session-tui style browser model, but pane groups and true Ghostty surfaces still need upstream-backed integration.",
                                )
                                .size(LabelSize::Small)
                                .color(Color::Default)
                                .into_any_element(),
                            ])
                            .collect::<Vec<_>>(),
                        RightPanel::Extensions => vec![
                            metadata_row("Metadata", "session-info.ygg"),
                            metadata_row("Clipboard", "remote-image-paste"),
                            metadata_row("Restore", "workspace-rehydrate"),
                            metadata_row("Rendered View", "rich transcript surface"),
                            Divider::horizontal().inset().into_any_element(),
                            Label::new(
                                "This panel will host Yggterm-specific session actions, metadata extensions, and remote capabilities rather than editor extensions.",
                            )
                            .size(LabelSize::Small)
                            .color(Color::Default)
                            .into_any_element(),
                        ],
                    }),
            )
            .into_any_element()
    }

    fn bottom_dock(&self, cx: &mut Context<Self>, colors: &theme::ThemeColors) -> AnyElement {
        let selected = self.selected_dock();

        div()
            .w_full()
            .h(self.bottom_dock_height)
            .flex()
            .flex_col()
            .bg(colors.panel_background)
            .border_t_1()
            .border_color(colors.border)
            .child(
                div()
                    .w_full()
                    .h(px(34.))
                    .px_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .bg(colors.toolbar_background)
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(Label::new(selected.title).size(LabelSize::Small))
                            .child(
                                Label::new("status-bar buffer")
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(
                                self.chrome_icon("dock-close", IconName::Close, false)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        let ix = this.selected_dock_ix;
                                        this.select_dock(ix, cx)
                                    })),
                            )
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .id("dock-scroll")
                    .overflow_y_scroll()
                    .px_4()
                    .py_3()
                    .gap_2()
                    .child(Label::new(selected.body).size(LabelSize::Small).color(Color::Default))
                    .child(
                        Label::new(
                            "Mock dock content stays fake on purpose until the Zed shell shape is stable enough to host real terminal surfaces.",
                        )
                        .size(LabelSize::Small)
                        .color(Color::Default),
                    ),
            )
            .into_any_element()
    }

    fn bottom_dock_resize_handle(
        &self,
        cx: &mut Context<Self>,
        colors: &theme::ThemeColors,
    ) -> AnyElement {
        div()
            .w_full()
            .h(px(6.))
            .cursor(CursorStyle::ResizeUpDown)
            .flex_shrink_0()
            .bg(colors.surface_background)
            .child(div().my_auto().h(px(1.)).w_full().bg(colors.border))
            .on_mouse_down(MouseButton::Left, cx.listener(|this, event: &MouseDownEvent, _, cx| {
                this.begin_panel_drag(
                    DragTarget::BottomDock,
                    event.position,
                    this.bottom_dock_height,
                    cx,
                );
                cx.stop_propagation();
            }))
            .into_any_element()
    }

    fn status_bar(&self, cx: &mut Context<Self>, colors: &theme::ThemeColors) -> AnyElement {
        let dock_tabs = self
            .dock_entries
            .iter()
            .enumerate()
            .map(|(ix, entry)| {
                Button::new(format!("status-dock-{}", entry.title), entry.title)
                    .style(if self.bottom_panel_open && ix == self.selected_dock_ix {
                        ButtonStyle::Subtle
                    } else {
                        ButtonStyle::Transparent
                    })
                    .size(ButtonSize::Compact)
                    .on_click(cx.listener(move |this, _, _, cx| this.select_dock(ix, cx)))
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        div()
            .w_full()
            .h(px(30.))
            .px_2()
            .flex()
            .items_center()
            .justify_between()
            .bg(colors.status_bar_background)
            .border_t_1()
            .border_color(colors.border)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .children(dock_tabs)
                    .child(Divider::vertical().inset())
                    .child(
                        Label::new(format!("{} saved sessions", self.total_leaf_sessions()))
                            .size(LabelSize::Small)
                            .color(Color::Default),
                    )
                    .child(
                        Label::new(format!(
                            "selected {}",
                            self.active_session()
                                .map(|session| session.title.as_str())
                                .unwrap_or("none")
                        ))
                            .size(LabelSize::Small)
                            .color(Color::Default),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Label::new(match self.server.active_view_mode() {
                            WorkspaceViewMode::Terminal => "terminal",
                            WorkspaceViewMode::Rendered => "rendered",
                        })
                        .size(LabelSize::Small)
                        .color(Color::Default),
                    )
                    .child(
                        Label::new(self.last_action.clone())
                        .size(LabelSize::Small)
                        .color(Color::Default),
                    )
                    .child(self.chrome_icon(
                        "status-remote",
                        match self.server.backend() {
                            TerminalBackend::Ghostty => IconName::TerminalGhost,
                            TerminalBackend::Mock => IconName::Disconnected,
                        },
                        false,
                    )),
            )
            .into_any_element()
    }

    fn chrome_icon(
        &self,
        id: impl Into<gpui::ElementId>,
        icon: IconName,
        selected: bool,
    ) -> IconButton {
        IconButton::new(id, icon)
            .shape(IconButtonShape::Square)
            .icon_size(IconSize::Small)
            .icon_color(if selected {
                Color::Accent
            } else {
                Color::Muted
            })
            .toggle_state(selected)
            .style(ButtonStyle::Transparent)
    }

    fn session_preview_block_element(
        &self,
        block_ix: usize,
        block: &yggterm_core::SessionPreviewBlock,
        query: &str,
        cx: &mut Context<Self>,
        colors: &theme::ThemeColors,
    ) -> AnyElement {
        let border = match block.tone {
            PreviewTone::User => colors.text_accent.opacity(0.32),
            PreviewTone::Assistant => colors.border_variant,
        };
        let bg = match block.tone {
            PreviewTone::User => colors.text_accent.opacity(0.08),
            PreviewTone::Assistant => colors.surface_background,
        };

        v_flex()
            .gap_2()
            .p_3()
            .rounded_md()
            .bg(bg)
            .border_1()
            .border_color(border)
            .child(
                Button::new(
                    format!("preview-block-{block_ix}"),
                    format!(
                        "{}  {}  {}",
                        if block.folded { "▶" } else { "▼" },
                        block.role,
                        block.timestamp
                    ),
                )
                .style(ButtonStyle::Transparent)
                .full_width()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_preview_block(block_ix, cx);
                })),
            )
            .when(!query.is_empty(), |this| {
                this.child(
                    Label::new(format!("query: {query}"))
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
            })
            .when(!block.folded, |this| {
                this.children(
                    block
                        .lines
                        .iter()
                        .map(|line| {
                            Label::new(line.clone())
                                .size(LabelSize::Small)
                                .color(if !query.is_empty()
                                    && line.to_ascii_lowercase().contains(&query.to_ascii_lowercase())
                                {
                                    Color::Accent
                                } else {
                                    Color::Default
                                })
                                .into_any_element()
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .into_any_element()
    }

    fn command_palette_overlay(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let active_theme = cx.theme().name.to_string();
        let theme_names = available_theme_names(cx);
        let palette_query = self.palette_query(cx);
        let palette_query_lower = palette_query.to_lowercase();
        let query_matches = |candidate: &str| {
            palette_query_lower.is_empty() || candidate.to_lowercase().contains(&palette_query_lower)
        };
        let settings = ThemeSettings::get_global(cx);
        let palette_focused = self.palette_editor.read(cx).is_focused(window);
        let palette_text_style = TextStyle {
            color: cx.theme().colors().text,
            font_family: settings.buffer_font.family.clone(),
            font_features: settings.buffer_font.features.clone(),
            font_fallbacks: settings.buffer_font.fallbacks.clone(),
            font_size: ui::rems(0.875).into(),
            font_weight: settings.buffer_font.weight,
            line_height: relative(1.3),
            ..TextStyle::default()
        };
        let palette_editor_style = EditorStyle {
            background: cx.theme().colors().elevated_surface_background,
            local_player: cx.theme().players().local(),
            text: palette_text_style,
            syntax: cx.theme().syntax().clone(),
            ..EditorStyle::default()
        };
        let wash = match cx.theme().appearance {
            Appearance::Light => Hsla {
                h: 0.,
                s: 0.,
                l: 0.,
                a: 0.12,
            },
            Appearance::Dark => Hsla {
                h: 0.,
                s: 0.,
                l: 0.,
                a: 0.38,
            },
        };

        div()
            .absolute()
            .inset_0()
            .occlude()
            .bg(wash)
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                this.dismiss_command_palette(cx);
            }))
            .child(
                v_flex()
                    .size_full()
                    .justify_center()
                    .items_center()
                    .child(
                        v_flex()
                            .w(px(720.))
                            .max_h(px(520.))
                            .occlude()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .elevation_3(cx)
                            .bg(cx.theme().colors().elevated_surface_background)
                            .rounded_lg()
                            .border_1()
                            .border_color(cx.theme().colors().border)
                            .overflow_hidden()
                            .child(
                                h_flex()
                                    .w_full()
                                    .h(px(46.))
                                    .px_3()
                                    .items_center()
                                    .gap_2()
                                    .border_b_1()
                                    .border_color(cx.theme().colors().border_variant)
                                    .child(h_flex()
                                        .flex_1()
                                        .h(px(32.))
                                        .items_center()
                                        .gap_2()
                                        .px_2()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(if palette_focused {
                                            cx.theme().colors().border_focused
                                        } else {
                                            cx.theme().colors().border_variant
                                        })
                                        .bg(cx.theme().colors().elevated_surface_background)
                                        .child(
                                            Icon::new(IconName::MagnifyingGlass)
                                                .size(IconSize::Small)
                                                .color(Color::Muted),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .h_full()
                                                .child(EditorElement::new(
                                                    &self.palette_editor,
                                                    palette_editor_style,
                                                )),
                                        )
                                        .child(
                                            IconButton::new(
                                                "palette-search-clear",
                                                IconName::Close,
                                            )
                                            .shape(IconButtonShape::Square)
                                            .icon_size(IconSize::XSmall)
                                            .icon_color(Color::Muted)
                                            .style(ButtonStyle::Transparent)
                                            .disabled(palette_query.is_empty())
                                            .tooltip(Tooltip::text("Clear Search"))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.palette_editor.update(cx, |editor, cx| {
                                                    editor.set_text("", window, cx);
                                                });
                                                this.focus_palette(window, cx);
                                            })),
                                        ),
                                    )
                                    .child(div().flex_1())
                                    .child(
                                        Label::new(active_theme)
                                            .size(LabelSize::Small)
                                            .color(Color::Muted),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .w_full()
                                    .max_h(px(514.))
                                    .id("command-palette-scroll")
                                    .overflow_y_scroll()
                                    .px_2()
                                    .py_2()
                                    .gap_2()
                                    .child(
                                        ListSubHeader::new("Actions")
                                            .left_icon(Some(IconName::Settings)),
                                    )
                                    .when(query_matches("Open Settings"), |this| {
                                        this.child(
                                            Button::new("palette-settings", "Open Settings")
                                                .style(ButtonStyle::Subtle)
                                                .full_width()
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.open_settings_window(cx);
                                                })),
                                        )
                                    })
                                    .when(query_matches("Toggle Default Light Dark Theme"), |this| {
                                        this.child(
                                            Button::new(
                                                "palette-toggle-theme-mode",
                                                "Toggle Default Light/Dark Theme",
                                            )
                                            .style(ButtonStyle::Subtle)
                                            .full_width()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                toggle_theme_mode_and_save(cx);
                                                this.dismiss_command_palette(cx);
                                            })),
                                        )
                                    })
                                    .when(query_matches("Switch to Terminal View"), |this| {
                                        this.child(
                                            Button::new(
                                                "palette-terminal-view",
                                                "Switch to Terminal View",
                                            )
                                            .style(ButtonStyle::Subtle)
                                            .full_width()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.set_view_mode(WorkspaceViewMode::Terminal, cx);
                                                this.dismiss_command_palette(cx);
                                            })),
                                        )
                                    })
                                    .when(query_matches("Switch to Rendered View"), |this| {
                                        this.child(
                                            Button::new(
                                                "palette-rendered-view",
                                                "Switch to Rendered View",
                                            )
                                            .style(ButtonStyle::Subtle)
                                            .full_width()
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.set_view_mode(WorkspaceViewMode::Rendered, cx);
                                                this.dismiss_command_palette(cx);
                                            })),
                                        )
                                    })
                                    .when(query_matches("Focus Session Search"), |this| {
                                        this.child(
                                            Button::new(
                                                "palette-focus-search",
                                                "Focus Session Search",
                                            )
                                            .style(ButtonStyle::Subtle)
                                            .full_width()
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.dismiss_command_palette(cx);
                                                this.focus_search(window, cx);
                                            })),
                                        )
                                    })
                                    .child(Divider::horizontal())
                                    .child(
                                        ListSubHeader::new("Bundled Zed Themes")
                                            .left_icon(Some(IconName::SwatchBook)),
                                    )
                                    .children(
                                        theme_names
                                            .into_iter()
                                            .filter(|theme_name| query_matches(theme_name))
                                            .map(|theme_name| {
                                                let is_active = theme_name == cx.theme().name.to_string();
                                                Button::new(
                                                    format!("palette-theme-{theme_name}"),
                                                    theme_name.clone(),
                                                )
                                                .style(if is_active {
                                                    ButtonStyle::Outlined
                                                } else {
                                                    ButtonStyle::Subtle
                                                })
                                                .full_width()
                                                .on_click({
                                                    let theme_name = theme_name.clone();
                                                    cx.listener(move |this, _, _, cx| {
                                                        set_theme_name_and_save(&theme_name, cx);
                                                        this.dismiss_command_palette(cx);
                                                    })
                                                })
                                                .into_any_element()
                                            })
                                            .collect::<Vec<_>>(),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn session_tree_row(
        &self,
        ix: usize,
        row: &BrowserRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let is_session = row.kind == BrowserRowKind::Session;
        let is_selected = self
            .browser
            .selected_path()
            .is_some_and(|path| path == row.full_path);
        let start_slot = if is_session {
            h_flex()
                .items_center()
                .gap_1()
                .child(Icon::new(IconName::TerminalGhost).size(IconSize::Small))
                .into_any_element()
        } else {
            h_flex()
                .items_center()
                .gap(px(2.))
                .child(Disclosure::new(format!("disclosure-{}", row.full_path), row.expanded))
                .child(Icon::new(IconName::FolderOpen).size(IconSize::Small))
                .into_any_element()
        };

        ListItem::new(format!("session-{}", row.full_path))
            .spacing(ListItemSpacing::Dense)
            .indent_level(row.depth)
            .start_slot(start_slot)
            .toggle_state(is_selected)
            .on_click(cx.listener(move |this, _, _, cx| this.select_row(ix, cx)))
            .child(
                v_flex()
                    .gap(px(2.))
                    .child(
                        Label::new(row.label.clone())
                            .size(LabelSize::Small)
                            .color(Color::Default),
                    )
                    .child(
                        Label::new(row.detail_label.clone())
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .end_slot(
                Label::new(row.host_label.clone())
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .into_any_element()
    }

}

impl Render for GpuiShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors().clone();
        self.server.sync_theme(match cx.theme().appearance {
            Appearance::Light => UiTheme::ZedLight,
            Appearance::Dark => UiTheme::ZedDark,
        });
        yggterm_client_side_decorations(
            div()
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(|this, _: &OpenCommandPalette, window, cx| {
                    this.toggle_command_palette(window, cx);
                }))
                .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                    this.focus_search(window, cx);
                }))
                .on_action(cx.listener(|this, _: &SwitchToTerminalView, _, cx| {
                    this.set_view_mode(WorkspaceViewMode::Terminal, cx);
                }))
                .on_action(cx.listener(|this, _: &SwitchToRenderedView, _, cx| {
                    this.set_view_mode(WorkspaceViewMode::Rendered, cx);
                }))
                .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                    this.toggle_sidebar(cx);
                }))
                .on_action(cx.listener(|this, _: &CloseCommandPalette, _, cx| {
                    this.dismiss_command_palette(cx);
                }))
                .on_action(|_: &CloseWindow, window, _| {
                    window.remove_window();
                })
                .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                    this.update_panel_drag(event.position, window, cx);
                }))
                .on_mouse_up(MouseButton::Left, cx.listener(|this, _, _, cx| {
                    this.end_panel_drag(cx);
                }))
                .map(|this| {
                    self.sync_browser_filter_from_editor(cx);
                    this
                })
                .size_full()
                .flex()
                .flex_col()
                .bg(colors.background)
                .overflow_hidden()
                .child(self.window_chrome(window, cx))
                .child(
                    div()
                        .flex_1()
                        .w_full()
                        .flex()
                        .overflow_hidden()
                        .when(self.sidebar_open, |this| {
                            this.child(self.sidebar(cx, &colors))
                                .child(self.sidebar_resize_handle(cx, &colors))
                        })
                        .child(self.workspace(window, cx, &colors)),
                )
                .when(self.command_palette_open, |this| {
                    this.child(self.command_palette_overlay(window, cx))
                }),
            window,
            cx,
            Tiling::default(),
        )
    }
}

#[derive(Clone, Copy)]
struct GlobalResizeEdge(Option<ResizeEdge>);
impl Global for GlobalResizeEdge {}

fn yggterm_client_side_decorations(
    element: impl IntoElement,
    window: &mut Window,
    cx: &mut App,
    border_radius_tiling: Tiling,
) -> gpui::Stateful<gpui::Div> {
    const BORDER_SIZE: Pixels = px(1.0);
    let decorations = window.window_decorations();
    let tiling = match decorations {
        Decorations::Server => Tiling::default(),
        Decorations::Client { tiling } => tiling,
    };

    match decorations {
        Decorations::Client { .. } => {
            window.set_client_inset(theme::CLIENT_SIDE_DECORATION_SHADOW)
        }
        Decorations::Server => window.set_client_inset(px(0.0)),
    }

    div()
        .id("window-backdrop")
        .bg(transparent_black())
        .map(|div| match decorations {
            Decorations::Server => div,
            Decorations::Client { .. } => div
                .when(
                    !(tiling.top
                        || tiling.right
                        || border_radius_tiling.top
                        || border_radius_tiling.right),
                    |div| div.rounded_tr(theme::CLIENT_SIDE_DECORATION_ROUNDING),
                )
                .when(
                    !(tiling.top
                        || tiling.left
                        || border_radius_tiling.top
                        || border_radius_tiling.left),
                    |div| div.rounded_tl(theme::CLIENT_SIDE_DECORATION_ROUNDING),
                )
                .when(
                    !(tiling.bottom
                        || tiling.right
                        || border_radius_tiling.bottom
                        || border_radius_tiling.right),
                    |div| div.rounded_br(theme::CLIENT_SIDE_DECORATION_ROUNDING),
                )
                .when(
                    !(tiling.bottom
                        || tiling.left
                        || border_radius_tiling.bottom
                        || border_radius_tiling.left),
                    |div| div.rounded_bl(theme::CLIENT_SIDE_DECORATION_ROUNDING),
                )
                .when(!tiling.top, |div| {
                    div.pt(theme::CLIENT_SIDE_DECORATION_SHADOW)
                })
                .when(!tiling.bottom, |div| {
                    div.pb(theme::CLIENT_SIDE_DECORATION_SHADOW)
                })
                .when(!tiling.left, |div| {
                    div.pl(theme::CLIENT_SIDE_DECORATION_SHADOW)
                })
                .when(!tiling.right, |div| {
                    div.pr(theme::CLIENT_SIDE_DECORATION_SHADOW)
                })
                .on_mouse_move(move |e, window, cx| {
                    let size = window.window_bounds().get_bounds().size;
                    let new_edge =
                        resize_edge(e.position, theme::CLIENT_SIDE_DECORATION_SHADOW, size, tiling);
                    let edge = cx.try_global::<GlobalResizeEdge>().and_then(|edge| edge.0);
                    if new_edge != edge {
                        cx.set_global(GlobalResizeEdge(new_edge));
                        let _ = window.window_handle().update(cx, |view, _, cx| {
                            cx.notify(view.entity_id());
                        });
                    }
                    window.refresh();
                })
                .on_hover(|hovered, window, _| {
                    if !*hovered {
                        window.refresh();
                    }
                })
                .on_mouse_down(MouseButton::Left, move |e, window, _| {
                    let size = window.window_bounds().get_bounds().size;
                    let Some(edge) =
                        resize_edge(e.position, theme::CLIENT_SIDE_DECORATION_SHADOW, size, tiling)
                    else {
                        return;
                    };
                    window.start_window_resize(edge);
                }),
        })
        .size_full()
        .child(
            div()
                .cursor(CursorStyle::Arrow)
                .map(|div| match decorations {
                    Decorations::Server => div,
                    Decorations::Client { .. } => div
                        .border_color(cx.theme().colors().border)
                        .when(
                            !(tiling.top
                                || tiling.right
                                || border_radius_tiling.top
                                || border_radius_tiling.right),
                            |div| div.rounded_tr(theme::CLIENT_SIDE_DECORATION_ROUNDING),
                        )
                        .when(
                            !(tiling.top
                                || tiling.left
                                || border_radius_tiling.top
                                || border_radius_tiling.left),
                            |div| div.rounded_tl(theme::CLIENT_SIDE_DECORATION_ROUNDING),
                        )
                        .when(
                            !(tiling.bottom
                                || tiling.right
                                || border_radius_tiling.bottom
                                || border_radius_tiling.right),
                            |div| div.rounded_br(theme::CLIENT_SIDE_DECORATION_ROUNDING),
                        )
                        .when(
                            !(tiling.bottom
                                || tiling.left
                                || border_radius_tiling.bottom
                                || border_radius_tiling.left),
                            |div| div.rounded_bl(theme::CLIENT_SIDE_DECORATION_ROUNDING),
                        )
                        .when(!tiling.top, |div| div.border_t(BORDER_SIZE))
                        .when(!tiling.bottom, |div| div.border_b(BORDER_SIZE))
                        .when(!tiling.left, |div| div.border_l(BORDER_SIZE))
                        .when(!tiling.right, |div| div.border_r(BORDER_SIZE))
                        .when(!tiling.is_tiled(), |div| {
                            div.shadow(vec![gpui::BoxShadow {
                                color: Hsla {
                                    h: 0.,
                                    s: 0.,
                                    l: 0.,
                                    a: 0.4,
                                },
                                blur_radius: theme::CLIENT_SIDE_DECORATION_SHADOW / 2.,
                                spread_radius: px(0.),
                                offset: point(px(0.0), px(0.0)),
                            }])
                        }),
                })
                .on_mouse_move(|_, window, cx| {
                    cx.stop_propagation();
                    window.refresh();
                })
                .size_full()
                .child(element),
        )
        .map(|div| match decorations {
            Decorations::Server => div,
            Decorations::Client { tiling, .. } => div.child(
                canvas(
                    |_bounds, window, _| {
                        window.insert_hitbox(
                            Bounds::new(
                                point(px(0.0), px(0.0)),
                                window.window_bounds().get_bounds().size,
                            ),
                            HitboxBehavior::Normal,
                        )
                    },
                    move |_bounds, hitbox, window, cx| {
                        let mouse = window.mouse_position();
                        let size = window.window_bounds().get_bounds().size;
                        let edge =
                            resize_edge(mouse, theme::CLIENT_SIDE_DECORATION_SHADOW, size, tiling);
                        cx.set_global(GlobalResizeEdge(edge));
                        let Some(edge) = edge else {
                            return;
                        };
                        window.set_cursor_style(
                            match edge {
                                ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
                                ResizeEdge::Left | ResizeEdge::Right => {
                                    CursorStyle::ResizeLeftRight
                                }
                                ResizeEdge::TopLeft | ResizeEdge::BottomRight => {
                                    CursorStyle::ResizeUpLeftDownRight
                                }
                                ResizeEdge::TopRight | ResizeEdge::BottomLeft => {
                                    CursorStyle::ResizeUpRightDownLeft
                                }
                            },
                            &hitbox,
                        );
                    },
                )
                .size_full()
                .absolute(),
            ),
        })
}

fn resize_edge(
    pos: Point<Pixels>,
    shadow_size: Pixels,
    window_size: Size<Pixels>,
    tiling: Tiling,
) -> Option<ResizeEdge> {
    let bounds = Bounds::new(Point::default(), window_size).inset(shadow_size * 1.5);
    if bounds.contains(&pos) {
        return None;
    }

    let corner_size = size(shadow_size * 1.5, shadow_size * 1.5);
    let top_left_bounds = Bounds::new(Point::new(px(0.), px(0.)), corner_size);
    if !tiling.top && top_left_bounds.contains(&pos) {
        return Some(ResizeEdge::TopLeft);
    }

    let top_right_bounds = Bounds::new(
        Point::new(window_size.width - corner_size.width, px(0.)),
        corner_size,
    );
    if !tiling.top && top_right_bounds.contains(&pos) {
        return Some(ResizeEdge::TopRight);
    }

    let bottom_left_bounds = Bounds::new(
        Point::new(px(0.), window_size.height - corner_size.height),
        corner_size,
    );
    if !tiling.bottom && bottom_left_bounds.contains(&pos) {
        return Some(ResizeEdge::BottomLeft);
    }

    let bottom_right_bounds = Bounds::new(
        Point::new(
            window_size.width - corner_size.width,
            window_size.height - corner_size.height,
        ),
        corner_size,
    );
    if !tiling.bottom && bottom_right_bounds.contains(&pos) {
        return Some(ResizeEdge::BottomRight);
    }

    if !tiling.top && pos.y < shadow_size {
        Some(ResizeEdge::Top)
    } else if !tiling.bottom && pos.y > window_size.height - shadow_size {
        Some(ResizeEdge::Bottom)
    } else if !tiling.left && pos.x < shadow_size {
        Some(ResizeEdge::Left)
    } else if !tiling.right && pos.x > window_size.width - shadow_size {
        Some(ResizeEdge::Right)
    } else {
        None
    }
}

struct SettingsWindow {
    theme_names: Vec<String>,
    config_path: String,
}

impl SettingsWindow {
    fn new(cx: &mut Context<Self>) -> Self {
        Self {
            theme_names: available_theme_names(cx),
            config_path: ShellUiConfig::path().display().to_string(),
        }
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active_theme = cx.theme().name.to_string();

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(cx.theme().colors().background)
            .child(
                div()
                    .px_4()
                    .py_3()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .child(Label::new("Settings").size(LabelSize::Large)),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .px_4()
                    .py_3()
                    .gap_3()
                    .child(metadata_row("Config", self.config_path.clone()))
                    .child(metadata_row("Theme", active_theme.clone()))
                    .child(
                        Label::new("Bundled Zed Themes")
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                    .children(
                        self.theme_names
                            .iter()
                            .map(|theme_name| {
                                Button::new(
                                    format!("theme-settings-{theme_name}"),
                                    theme_name.clone(),
                                )
                                .style(if *theme_name == active_theme {
                                    ButtonStyle::Outlined
                                } else {
                                    ButtonStyle::Subtle
                                })
                                .full_width()
                                .on_click({
                                    let theme_name = theme_name.clone();
                                    move |_, _, cx| set_theme_name_and_save(&theme_name, cx)
                                })
                                .into_any_element()
                            })
                            .collect::<Vec<_>>(),
                    ),
            )
    }
}

fn metadata_row(label: &'static str, value: impl Into<SharedString>) -> AnyElement {
    ListItem::new(format!("meta-{label}"))
        .spacing(ListItemSpacing::Dense)
        .selectable(false)
        .start_slot(
            Label::new(label)
                .size(LabelSize::Small)
                .color(Color::Muted),
        )
        .child(
            Label::new(value)
                .size(LabelSize::Small)
                .color(Color::Default),
        )
        .into_any_element()
}

fn session_block<S: AsRef<str>>(
    title: &'static str,
    lines: &[S],
    badge: Option<&str>,
    colors: &theme::ThemeColors,
) -> AnyElement {
    let badge = badge.unwrap_or("session").to_string();
    v_flex()
        .gap_2()
        .p_3()
        .rounded_md()
        .bg(colors.surface_background)
        .border_1()
        .border_color(colors.border_variant)
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(Label::new(title).size(LabelSize::Small))
                .child(
                    Label::new(badge)
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                ),
        )
        .children(
            lines
                .iter()
                .enumerate()
                .map(|(ix, line)| {
                    let line = line.as_ref();
                    Label::new(format!("{:02}  {line}", ix + 1))
                        .size(LabelSize::Small)
                        .color(if line.starts_with('$') {
                            Color::Accent
                        } else {
                            Color::Default
                        })
                        .into_any_element()
                })
                .collect::<Vec<_>>(),
        )
        .into_any_element()
}

fn session_preview_summary(
    summary: &[SessionMetadataEntry],
    query: &str,
    matching_blocks: usize,
    total_blocks: usize,
    colors: &theme::ThemeColors,
) -> AnyElement {
    v_flex()
        .gap_1()
        .p_3()
        .rounded_md()
        .bg(colors.surface_background)
        .border_1()
        .border_color(colors.border_variant)
        .child(
            Label::new("Preview")
                .size(LabelSize::Small)
                .color(Color::Default),
        )
        .when(!query.is_empty(), |this| {
            this.child(
                Label::new(format!(
                    "Filtered by \"{query}\" · {matching_blocks}/{total_blocks} blocks"
                ))
                .size(LabelSize::Small)
                .color(Color::Muted),
            )
        })
        .children(
            summary
                .iter()
                .map(|entry| metadata_row(entry.label, entry.value.clone()))
                .collect::<Vec<_>>(),
        )
        .into_any_element()
}
