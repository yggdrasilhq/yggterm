use anyhow::Result;
use dioxus::desktop::{Config, LogicalSize, WindowBuilder, window};
use dioxus::prelude::*;
use once_cell::sync::OnceCell;
use yggterm_core::{
    BrowserRow, BrowserRowKind, ManagedSessionView, PreviewTone, SessionBrowserState, SessionNode,
    SshConnectTarget, UiTheme, WorkspaceViewMode, YggtermServer,
};

static BOOTSTRAP: OnceCell<ShellBootstrap> = OnceCell::new();

#[derive(Debug, Clone)]
pub struct ShellBootstrap {
    pub tree: SessionNode,
    pub browser_tree: SessionNode,
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
    search_query: String,
    sidebar_open: bool,
    right_panel_open: bool,
    last_action: String,
}

#[derive(Clone, PartialEq)]
struct RenderSnapshot {
    palette: Palette,
    search_query: String,
    sidebar_open: bool,
    right_panel_open: bool,
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
    Minimize,
    Maximize,
    Close,
}

impl ShellState {
    fn new(bootstrap: ShellBootstrap) -> Self {
        Self {
            browser: SessionBrowserState::new(bootstrap.browser_tree.clone()),
            server: YggtermServer::new(
                &bootstrap.browser_tree,
                bootstrap.prefer_ghostty_backend,
                bootstrap.ghostty_bridge_enabled,
                bootstrap.theme,
            ),
            bootstrap,
            search_query: String::new(),
            sidebar_open: true,
            right_panel_open: true,
            last_action: "ready".to_string(),
        }
    }

    fn snapshot(&self) -> RenderSnapshot {
        RenderSnapshot {
            palette: palette(self.bootstrap.theme),
            search_query: self.search_query.clone(),
            sidebar_open: self.sidebar_open,
            right_panel_open: self.right_panel_open,
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
        self.last_action = if self.sidebar_open {
            "sidebar opened".to_string()
        } else {
            "sidebar hidden".to_string()
        };
    }

    fn toggle_right_panel(&mut self) {
        self.right_panel_open = !self.right_panel_open;
        self.last_action = if self.right_panel_open {
            "metadata opened".to_string()
        } else {
            "metadata hidden".to_string()
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
                self.last_action = format!("toggled {}", row.label);
            }
            BrowserRowKind::Session => {
                self.server.open_or_focus_session(
                    &row.full_path,
                    row.session_id.as_deref(),
                    row.session_cwd.as_deref(),
                );
                self.server.set_view_mode(WorkspaceViewMode::Rendered);
                self.last_action = format!("opened {}", row.label);
            }
        }
    }

    fn connect_ssh_target(&mut self, target_ix: usize) {
        if let Some(key) = self.server.connect_ssh_target(target_ix) {
            self.last_action = format!("connected {key}");
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
    let maximized = window().is_maximized();

    rsx! {
        div {
            style: "position: fixed; inset: 0; background: transparent; overflow: hidden;",
            div {
                style: shell_style(snapshot.palette),
                Titlebar {
                    snapshot: titlebar_snapshot,
                    hovered: hovered,
                    on_toggle_sidebar: move || state.with_mut(|shell| shell.toggle_sidebar()),
                    on_search: move |value: String| state.with_mut(|shell| shell.set_search(value)),
                    on_hover_control: move |control: Option<HoveredControl>| hovered.set(control),
                    on_set_view_mode: move |mode: WorkspaceViewMode| state.with_mut(|shell| shell.set_view_mode(mode)),
                    on_toggle_meta: move || state.with_mut(|shell| shell.toggle_right_panel()),
                    maximized: maximized,
                }
                div {
                    style: "display: flex; flex: 1; min-height: 0; overflow: hidden;",
                    if sidebar_snapshot.sidebar_open {
                        Sidebar {
                            snapshot: sidebar_snapshot,
                            on_select_row: move |row: BrowserRow| state.with_mut(|shell| shell.select_row(&row)),
                            on_connect_ssh: move |ix: usize| state.with_mut(|shell| shell.connect_ssh_target(ix)),
                            on_focus_live: move |id: String| state.with_mut(|shell| shell.focus_live_session(&id)),
                        }
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
                    if metadata_snapshot.right_panel_open {
                        MetadataRail { snapshot: metadata_snapshot }
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
    maximized: bool,
) -> Element {
    rsx! {
        div {
            style: format!(
                "display:flex; align-items:center; justify-content:space-between; height:44px; \
                 padding:0 12px; background:{};",
                snapshot.palette.titlebar
            ),
            onmousedown: move |_| window().drag(),
            div {
                style: "display:flex; align-items:center; gap:12px; width:300px; min-width:300px;",
                button {
                    style: icon_button_style(snapshot.palette),
                    onmousedown: |evt| evt.stop_propagation(),
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
                        onclick: move |_| on_set_view_mode.call(WorkspaceViewMode::Rendered),
                        "Preview"
                    }
                    button {
                        style: toggle_slider_end_style(
                            snapshot.palette,
                            snapshot.active_view_mode == WorkspaceViewMode::Terminal
                        ),
                        onclick: move |_| on_set_view_mode.call(WorkspaceViewMode::Terminal),
                        "Terminal"
                    }
                }
            }
            div {
                style: "flex:1; display:flex; justify-content:center; padding:0 16px;",
                input {
                    r#type: "text",
                    value: "{snapshot.search_query}",
                    placeholder: "Search sessions…",
                    style: search_style(snapshot.palette),
                    onmousedown: |evt| evt.stop_propagation(),
                    oninput: move |evt| on_search.call(evt.value()),
                }
            }
            div {
                style: "display:flex; align-items:center; justify-content:flex-end; gap:10px; width:300px; min-width:300px;",
                button {
                    style: metadata_toggle_style(snapshot.palette, snapshot.right_panel_open),
                    onmousedown: |evt| evt.stop_propagation(),
                    onclick: move |_| on_toggle_meta.call(()),
                    if snapshot.right_panel_open { "Metadata" } else { "Inspect" }
                }
                WindowControls {
                    palette: snapshot.palette,
                    hovered: hovered(),
                    on_hover_control: on_hover_control,
                    maximized: maximized,
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
    maximized: bool,
) -> Element {
    let maximize_glyph = if maximized { "❐" } else { "▢" };

    rsx! {
        div {
            style: "display:flex; align-items:stretch; gap:0;",
            WindowControl {
                glyph: "—",
                hovered: hovered == Some(HoveredControl::Minimize),
                hover_tone: HoveredControl::Minimize,
                palette,
                on_hover_control,
                on_press: move |_| window().set_minimized(true),
            }
            WindowControl {
                glyph: maximize_glyph,
                hovered: hovered == Some(HoveredControl::Maximize),
                hover_tone: HoveredControl::Maximize,
                palette,
                on_hover_control,
                on_press: move |_| window().toggle_maximized(),
            }
            WindowControl {
                glyph: "✕",
                hovered: hovered == Some(HoveredControl::Close),
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
    glyph: &'static str,
    hovered: bool,
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
    } else {
        "transparent"
    };
    let color = if hovered && is_close {
        "#ffffff"
    } else {
        palette.text
    };

    rsx! {
        button {
            style: format!(
                "width:34px; height:30px; border:none; border-radius:0; background:{}; color:{}; \
                 display:flex; align-items:center; justify-content:center; font-size:13px; font-weight:600;",
                background, color
            ),
            onmousedown: |evt| evt.stop_propagation(),
            onmouseenter: move |_| on_hover_control.call(Some(hover_tone)),
            onmouseleave: move |_| on_hover_control.call(None),
            onclick: move |evt| on_press.call(evt),
            "{glyph}"
        }
    }
}

#[component]
fn Sidebar(
    snapshot: RenderSnapshot,
    on_select_row: EventHandler<BrowserRow>,
    on_connect_ssh: EventHandler<usize>,
    on_focus_live: EventHandler<String>,
) -> Element {
    rsx! {
        div {
            style: format!(
                "width:258px; min-width:258px; max-width:258px; display:flex; flex-direction:column; \
                 background:{}; overflow:hidden;",
                snapshot.palette.sidebar
            ),
            div {
                style: "padding:14px 14px 10px 14px;",
                div {
                    style: format!("font-size:11px; color:{}; margin-bottom:6px;", snapshot.palette.muted),
                    "Codex Sessions"
                }
                        div {
                            style: format!("font-size:12px; font-weight:600; color:{};", snapshot.palette.text),
                            {format!("{} stored · {} visible", snapshot.total_leaf_sessions, snapshot.rows.len())}
                        }
            }
            div {
                style: "flex:1; min-height:0; overflow:auto; padding:8px 10px;",
                for row in snapshot.rows.iter().cloned() {
                    SidebarRow {
                        row: row.clone(),
                        selected: snapshot.selected_path.as_deref() == Some(row.full_path.as_str()),
                        palette: snapshot.palette,
                        on_select: move |_| on_select_row.call(row.clone()),
                    }
                }
            }
            div {
                style: format!(
                    "padding:12px 10px 12px 10px; display:flex; flex-direction:column; gap:10px;",
                ),
                if !snapshot.ssh_targets.is_empty() {
                    div {
                        div {
                            style: format!("font-size:11px; color:{}; margin-bottom:6px;", snapshot.palette.muted),
                            "Connect SSH"
                        }
                        div {
                            style: "display:flex; flex-direction:column; gap:6px;",
                            for (ix , target) in snapshot.ssh_targets.iter().cloned().enumerate() {
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
                                                style: "display:flex; flex-direction:column; align-items:flex-start; min-width:0;",
                                                span {
                                                    style: format!("font-size:12px; font-weight:600; color:{};", snapshot.palette.text),
                                                    "{target.label}"
                                                }
                                                span {
                                                    style: format!("font-size:11px; color:{}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;", snapshot.palette.muted),
                                                    "{target_detail}"
                                                }
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
                        div {
                            style: format!("font-size:11px; color:{}; margin-bottom:6px;", snapshot.palette.muted),
                            "Live Sessions"
                        }
                        div {
                            style: "display:flex; flex-direction:column; gap:6px;",
                            for session in snapshot.live_sessions.iter().cloned() {
                                button {
                                    style: sidebar_action_style(snapshot.palette),
                                    onclick: move |_| on_focus_live.call(session.session_path.clone()),
                                    div {
                                        style: "display:flex; flex-direction:column; align-items:flex-start; min-width:0;",
                                        span {
                                            style: format!("font-size:12px; font-weight:600; color:{};", snapshot.palette.text),
                                            "{session.title}"
                                        }
                                        span {
                                            style: format!("font-size:11px; color:{}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;", snapshot.palette.muted),
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
    }
}

#[component]
fn SidebarRow(
    row: BrowserRow,
    selected: bool,
    palette: Palette,
    on_select: EventHandler<MouseEvent>,
) -> Element {
    let disclosure = match row.kind {
        BrowserRowKind::Group if row.expanded => "▾",
        BrowserRowKind::Group => "▸",
        BrowserRowKind::Session => "•",
    };
    let indent = row.depth * 14 + 10;
    let background = if selected {
        palette.accent_soft
    } else if row.kind == BrowserRowKind::Group && row.depth == 0 {
        palette.panel_alt
    } else {
        "transparent"
    };

    rsx! {
        button {
            style: format!(
                "width:100%; display:flex; flex-direction:column; align-items:stretch; gap:3px; \
                 border:none; border-radius:12px; background:{}; padding:8px 10px 8px {}px; margin-bottom:4px;",
                background, indent
            ),
            onclick: move |evt| on_select.call(evt),
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:8px;",
                div {
                    style: "display:flex; align-items:center; gap:8px; min-width:0;",
                    span {
                        style: format!("font-size:11px; color:{};", palette.muted),
                        "{disclosure}"
                    }
                    span {
                        style: format!(
                            "font-size:12px; color:{}; font-weight:{}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;",
                            if selected { palette.text } else if row.kind == BrowserRowKind::Group && row.depth > 0 { palette.muted } else { palette.text },
                            if row.kind == BrowserRowKind::Group && row.depth == 0 { 600 } else { 500 }
                        ),
                        "{row.label}"
                    }
                }
                if row.kind == BrowserRowKind::Group {
                    span {
                        style: format!("font-size:11px; color:{};", palette.muted),
                        "{row.descendant_sessions}"
                    }
                } else if !row.host_label.is_empty() {
                    span {
                        style: format!("font-size:11px; color:{};", palette.muted),
                        "{row.host_label}"
                    }
                }
            }
            if !row.detail_label.is_empty() {
                div {
                    style: format!(
                        "font-size:11px; color:{}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;",
                        palette.muted
                    ),
                    "{row.detail_label}"
                }
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
                    "flex:1; overflow:auto; padding:24px; background:{}; border-radius:22px; box-shadow:{};",
                    snapshot.palette.panel, snapshot.palette.panel_shadow
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
fn MetadataRail(snapshot: RenderSnapshot) -> Element {
    let session = snapshot.active_session;

    rsx! {
        div {
            style: format!(
                "width:236px; min-width:236px; max-width:236px; display:flex; flex-direction:column; \
                 background:{}; overflow:hidden;",
                snapshot.palette.sidebar
            ),
            div {
                style: format!(
                    "padding:14px 16px 12px 16px; font-size:12px; font-weight:700; color:{};",
                    snapshot.palette.text
                ),
                "Session Metadata"
            }
            div {
                style: "flex:1; overflow:auto; padding:12px 16px; display:flex; flex-direction:column; gap:16px;",
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
}

#[component]
fn MetadataGroup(
    title: String,
    entries: Vec<yggterm_core::SessionMetadataEntry>,
    palette: Palette,
) -> Element {
    rsx! {
        div {
            style: format!(
                "display:flex; flex-direction:column; gap:8px; padding-bottom:10px;",
            ),
            div {
                style: format!("font-size:11px; font-weight:700; color:{};", palette.muted),
                "{title}"
            }
            for entry in entries.into_iter() {
                div {
                    style: "display:flex; flex-direction:column; gap:3px;",
                    span {
                        style: format!("font-size:11px; color:{};", palette.muted),
                        "{entry.label}"
                    }
                    span {
                        style: format!("font-size:12px; color:{}; white-space:pre-wrap;", palette.text),
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

fn shell_style(palette: Palette) -> String {
    format!(
        "position:fixed; inset:0; display:flex; flex-direction:column; overflow:hidden; \
         border-radius:11px; background-color:{}; background-image:{}; box-shadow:{}; backdrop-filter: blur(30px) saturate(165%); \
         -webkit-backdrop-filter: blur(30px) saturate(165%);",
        palette.shell, palette.gradient, palette.shadow
    )
}

fn search_style(palette: Palette) -> String {
    format!(
        "width:min(560px, 100%); height:32px; padding:0 12px; border-radius:8px; \
         border:none; background:rgba(255,255,255,0.66); color:{}; outline:none; font-size:12px; \
         box-shadow: inset 0 0 0 1px rgba(255,255,255,0.36);",
        palette.text
    )
}

fn icon_button_style(palette: Palette) -> String {
    format!(
        "width:28px; height:28px; border:none; border-radius:8px; background:transparent; color:{}; font-size:13px;",
        palette.muted
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
        "width:100%; border:none; border-radius:12px; background:{}; padding:9px 10px; text-align:left;",
        palette.sidebar_hover
    )
}

fn toggle_slider_style(_palette: Palette) -> String {
    format!(
        "display:flex; align-items:center; gap:4px; padding:3px; border:none; border-radius:999px; background:rgba(255,255,255,0.34); box-shadow: inset 0 0 0 1px rgba(255,255,255,0.28);"
    )
}

fn toggle_slider_end_style(palette: Palette, selected: bool) -> String {
    format!(
        "height:26px; min-width:82px; padding:0 12px; border:none; border-radius:999px; background:{}; color:{}; font-size:11px; font-weight:700;",
        if selected { palette.panel } else { "transparent" },
        if selected { palette.text } else { palette.muted }
    )
}

fn metadata_toggle_style(palette: Palette, selected: bool) -> String {
    format!(
        "height:28px; padding:0 12px; border:none; border-radius:10px; background:{}; color:{}; font-size:11px; font-weight:700;",
        if selected { "rgba(255,255,255,0.46)" } else { "transparent" },
        if selected { palette.text } else { palette.muted }
    )
}

fn metadata_value(session: &ManagedSessionView, label: &str) -> String {
    session
        .metadata
        .iter()
        .find(|entry| entry.label == label)
        .map(|entry| entry.value.clone())
        .unwrap_or_default()
}
