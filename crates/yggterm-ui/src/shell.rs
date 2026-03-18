use anyhow::Result;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, Bounds, Context, CursorStyle, Decorations, FocusHandle,
    HitboxBehavior, InteractiveElement, IntoElement, KeyBinding, MouseButton, ParentElement,
    Pixels, Render, ResizeEdge, ScrollStrategy, Size, StatefulInteractiveElement, Styled,
    UniformListScrollHandle, Window, WindowBackgroundAppearance, WindowBounds, WindowDecorations,
    WindowOptions, actions, canvas, div, point, px, size, transparent_black, uniform_list,
};
use gpui_platform::application;
use yggterm_core::{
    BrowserRow, BrowserRowKind, ManagedSessionView, PreviewTone, SessionBrowserState, SessionNode,
    UiTheme, WorkspaceViewMode, YggtermServer,
};

use crate::{
    ChatBubbleTone, TitlebarIcon, UiPalette, chat_preview_card, metadata_section_card,
    preview_summary_card, statusbar_frame, terminal_surface_card, titlebar_frame,
    titlebar_icon_button, toolbar_chip_button, window_controls,
};

actions!(
    search_input,
    [SearchPrev, SearchNext, SearchAccept, SearchClear]
);

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

pub fn launch_shell(bootstrap: ShellBootstrap) -> Result<()> {
    application().run(move |cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("up", SearchPrev, Some("SearchInput")),
            KeyBinding::new("down", SearchNext, Some("SearchInput")),
            KeyBinding::new("enter", SearchAccept, Some("SearchInput")),
            KeyBinding::new("escape", SearchClear, Some("SearchInput")),
        ]);
        let bounds = Bounds::centered(None, size(px(1460.), px(920.)), cx);
        let shell = bootstrap.clone();
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_background: WindowBackgroundAppearance::Transparent,
                window_decorations: Some(WindowDecorations::Client),
                window_min_size: Some(size(px(1024.), px(720.))),
                app_id: Some("dev.yggterm".into()),
                ..Default::default()
            },
            move |_window, cx| cx.new(|cx| GpuiShell::new(shell.clone(), cx)),
        )
        .expect("failed to open yggterm shell");
    });

    Ok(())
}

struct GpuiShell {
    bootstrap: ShellBootstrap,
    browser: SessionBrowserState,
    server: YggtermServer,
    sidebar_open: bool,
    right_panel_open: bool,
    sidebar_scroll_handle: UniformListScrollHandle,
    search_focus: FocusHandle,
    search_query: String,
    last_action: String,
}

impl GpuiShell {
    fn new(bootstrap: ShellBootstrap, cx: &mut Context<Self>) -> Self {
        let search_focus = cx.focus_handle();
        let this = Self {
            browser: SessionBrowserState::new(bootstrap.browser_tree.clone()),
            server: YggtermServer::new(
                &bootstrap.browser_tree,
                bootstrap.prefer_ghostty_backend,
                bootstrap.ghostty_bridge_enabled,
                bootstrap.theme,
            ),
            bootstrap,
            sidebar_open: true,
            right_panel_open: true,
            sidebar_scroll_handle: UniformListScrollHandle::new(),
            search_focus,
            search_query: String::new(),
            last_action: "ready".to_string(),
        };

        cx.observe_keystrokes(|this, event, window, cx| {
            if !this.search_focus.is_focused(window) {
                return;
            }

            let key = event.keystroke.key.as_str();
            match key {
                "backspace" => {
                    this.search_query.pop();
                    this.apply_search(cx);
                }
                "escape" => {
                    if !this.search_query.is_empty() {
                        this.search_query.clear();
                        this.apply_search(cx);
                    }
                }
                "enter" => {
                    if let Some(ix) = this.browser.selected_session_index().or_else(|| {
                        this.browser
                            .rows()
                            .iter()
                            .position(|row| row.kind == BrowserRowKind::Session)
                    }) {
                        this.select_row(ix, cx);
                    }
                }
                "up" => {
                    this.move_search_selection(-1, cx);
                }
                "down" => {
                    this.move_search_selection(1, cx);
                }
                _ => {
                    if event.keystroke.modifiers.control
                        || event.keystroke.modifiers.platform
                        || event.keystroke.modifiers.alt
                    {
                        return;
                    }

                    if let Some(text) = event.keystroke.key_char.as_ref() {
                        if !text.chars().any(|ch| ch.is_control()) {
                            this.search_query.push_str(text);
                            this.apply_search(cx);
                        }
                    }
                }
            }
        })
        .detach();

        this
    }

    fn palette(&self) -> UiPalette {
        match self.bootstrap.theme {
            UiTheme::ZedDark => UiPalette::dark(),
            UiTheme::ZedLight => UiPalette::light(),
        }
    }

    fn active_session(&self) -> Option<&ManagedSessionView> {
        self.server.active_session()
    }

    fn selected_row(&self) -> Option<&BrowserRow> {
        self.browser.selected_row()
    }

    fn metadata_value<'a>(&self, session: &'a ManagedSessionView, label: &str) -> &'a str {
        session
            .metadata
            .iter()
            .find(|entry| entry.label == label)
            .map(|entry| entry.value.as_str())
            .unwrap_or("")
    }

    fn total_leaf_sessions(&self) -> usize {
        self.browser.total_sessions()
    }

    fn apply_search(&mut self, cx: &mut Context<Self>) {
        self.browser.set_filter_query(self.search_query.clone());
        self.last_action = if self.search_query.is_empty() {
            "search cleared".to_string()
        } else {
            format!("filtered {}", self.search_query)
        };
        cx.notify();
    }

    fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.search_focus, cx);
        self.last_action = "search focused".to_string();
        cx.notify();
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_query.clear();
        self.browser.set_filter_query("");
        window.focus(&self.search_focus, cx);
        self.last_action = "search cleared".to_string();
        cx.notify();
    }

    fn move_search_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let selected_ix = if delta < 0 {
            self.browser.select_previous_session()
        } else {
            self.browser.select_next_session()
        };

        if let Some(ix) = selected_ix {
            self.sidebar_scroll_handle
                .scroll_to_item(ix, ScrollStrategy::Nearest);
            if let Some(row) = self.browser.rows().get(ix) {
                self.last_action = format!("selected {}", row.label);
            }
            cx.notify();
        }
    }

    fn accept_search_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(ix) = self.browser.selected_session_index().or_else(|| {
            self.browser
                .rows()
                .iter()
                .position(|row| row.kind == BrowserRowKind::Session)
        }) {
            self.select_row(ix, cx);
        }
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_open = !self.sidebar_open;
        self.last_action = if self.sidebar_open {
            "sidebar opened"
        } else {
            "sidebar hidden"
        }
        .to_string();
        cx.notify();
    }

    fn toggle_right_panel(&mut self, cx: &mut Context<Self>) {
        self.right_panel_open = !self.right_panel_open;
        self.last_action = if self.right_panel_open {
            "metadata opened"
        } else {
            "metadata hidden"
        }
        .to_string();
        cx.notify();
    }

    fn set_view_mode(&mut self, mode: WorkspaceViewMode, cx: &mut Context<Self>) {
        self.server.set_view_mode(mode);
        if mode == WorkspaceViewMode::Terminal {
            self.server.request_terminal_launch_for_active();
        }
        self.last_action = match mode {
            WorkspaceViewMode::Rendered => "preview mode",
            WorkspaceViewMode::Terminal => "terminal mode",
        }
        .to_string();
        cx.notify();
    }

    fn select_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(row) = self.browser.rows().get(ix).cloned() else {
            return;
        };

        match row.kind {
            BrowserRowKind::Group => {
                self.browser.toggle_group(&row.full_path);
                self.last_action = format!("toggled {}", row.label);
            }
            BrowserRowKind::Session => {
                self.browser.select_path(row.full_path.clone());
                self.server.open_or_focus_session(
                    &row.full_path,
                    row.session_id.as_deref(),
                    row.session_cwd.as_deref(),
                );
                self.server.set_view_mode(WorkspaceViewMode::Rendered);
                self.last_action = format!("selected {}", row.label);
            }
        }
        cx.notify();
    }

    fn toggle_preview_block(&mut self, block_ix: usize, cx: &mut Context<Self>) {
        self.server.toggle_preview_block(block_ix);
        cx.notify();
    }

    fn expand_preview_blocks(&mut self, cx: &mut Context<Self>) {
        self.server.set_all_preview_blocks_folded(false);
        self.last_action = "expanded preview".to_string();
        cx.notify();
    }

    fn collapse_preview_blocks(&mut self, cx: &mut Context<Self>) {
        self.server.set_all_preview_blocks_folded(true);
        self.last_action = "collapsed preview".to_string();
        cx.notify();
    }

    fn resolve_ghostty_window(&mut self, cx: &mut Context<Self>) {
        self.last_action = self.server.sync_external_terminal_window_for_active();
        cx.notify();
    }

    fn focus_ghostty_window(&mut self, cx: &mut Context<Self>) {
        self.last_action = self.server.raise_external_terminal_window_for_active();
        cx.notify();
    }

    fn search_bar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        palette: &UiPalette,
    ) -> AnyElement {
        let mut right_status = div().flex().flex_row().gap_2().items_center();
        if !self.search_query.is_empty() {
            right_status = right_status.child(
                toolbar_chip_button("clear-search", "Clear", false, palette)
                    .on_click(cx.listener(|this, _, window, cx| this.clear_search(window, cx))),
            );
        }
        let right_status = right_status.child(
            div()
                .text_xs()
                .text_color(palette.text_muted)
                .child(format!("{} rows", self.browser.rows().len())),
        );

        div()
            .id("search-bar")
            .w(px(344.))
            .h(px(27.))
            .flex()
            .items_center()
            .justify_between()
            .px_2()
            .rounded_lg()
            .bg(palette.element_background)
            .border_1()
            .border_color(if self.search_focus.is_focused(window) {
                palette.border_focused
            } else {
                palette.border_variant
            })
            .text_sm()
            .text_color(palette.text_muted)
            .focusable()
            .key_context("SearchInput")
            .track_focus(&self.search_focus)
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .on_action(
                cx.listener(|this, _: &SearchPrev, _, cx| this.move_search_selection(-1, cx)),
            )
            .on_action(cx.listener(|this, _: &SearchNext, _, cx| this.move_search_selection(1, cx)))
            .on_action(
                cx.listener(|this, _: &SearchAccept, _, cx| this.accept_search_selection(cx)),
            )
            .on_action(
                cx.listener(|this, _: &SearchClear, window, cx| this.clear_search(window, cx)),
            )
            .on_click(cx.listener(|this, _, window, cx| this.focus_search(window, cx)))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .gap_2()
                    .items_center()
                    .child(div().text_color(palette.text_muted).child("⌕"))
                    .child(
                        div()
                            .text_sm()
                            .text_color(if self.search_query.is_empty() {
                                palette.text_muted
                            } else {
                                palette.text
                            })
                            .child(if self.search_query.is_empty() {
                                "Search sessions…".to_string()
                            } else {
                                self.search_query.clone()
                            }),
                    ),
            )
            .child(right_status)
            .into_any_element()
    }

    fn titlebar(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        palette: &UiPalette,
    ) -> AnyElement {
        let left = div()
            .flex()
            .flex_row()
            .gap_1()
            .items_center()
            .when(cfg!(target_os = "macos"), |div| {
                div.child(window_controls(window, palette))
            })
            .child(
                titlebar_icon_button("toggle-sidebar", TitlebarIcon::Sidebar, palette)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
            .into_any_element();

        let right =
            div()
                .flex()
                .flex_row()
                .gap_1p5()
                .items_center()
                .child(
                    toolbar_chip_button(
                        "view-preview",
                        "Preview",
                        self.server.active_view_mode() == WorkspaceViewMode::Rendered,
                        palette,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_view_mode(WorkspaceViewMode::Rendered, cx)
                    })),
                )
                .child(
                    toolbar_chip_button(
                        "view-terminal",
                        "Terminal",
                        self.server.active_view_mode() == WorkspaceViewMode::Terminal,
                        palette,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.set_view_mode(WorkspaceViewMode::Terminal, cx)
                    })),
                )
                .child(
                    titlebar_icon_button("toggle-meta", TitlebarIcon::Info, palette)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_right_panel(cx))),
                )
                .when(!cfg!(target_os = "macos"), |div| {
                    div.child(window_controls(window, palette))
                })
                .into_any_element();

        titlebar_frame(
            left,
            self.search_bar(window, cx, palette),
            right,
            palette.window_background,
            palette.border_variant.opacity(0.92),
        )
        .into_any_element()
    }

    fn sidebar(&self, cx: &mut Context<Self>, palette: &UiPalette) -> AnyElement {
        if !self.sidebar_open {
            return div().into_any_element();
        }

        let row_count = self.browser.rows().len();
        let selected_row = self.selected_row().cloned();
        let header_title = selected_row
            .as_ref()
            .map(|row| row.label.clone())
            .unwrap_or_else(|| "Codex Sessions".to_string());
        let header_detail = selected_row
            .as_ref()
            .and_then(|row| {
                if row.detail_label.is_empty() {
                    None
                } else {
                    Some(row.detail_label.clone())
                }
            })
            .unwrap_or_else(|| {
                format!(
                    "{} stored · {} visible",
                    self.browser.total_session_count(),
                    self.total_leaf_sessions()
                )
            });
        let filter_active = !self.search_query.is_empty();
        let filter_summary = if filter_active {
            format!("{} matches for “{}”", row_count, self.search_query)
        } else {
            format!("{} visible rows", row_count)
        };
        div()
            .w(px(196.))
            .h_full()
            .flex()
            .flex_col()
            .bg(palette.window_background)
            .border_r_1()
            .border_color(palette.border_variant.opacity(0.84))
            .child(
                div()
                    .px_3()
                    .py_1p5()
                    .border_b_1()
                    .border_color(palette.border_variant.opacity(0.84))
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.text_muted)
                            .child("Codex Sessions"),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.text)
                            .line_clamp(1)
                            .text_ellipsis()
                            .child(header_title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.text_muted)
                            .line_clamp(1)
                            .text_ellipsis()
                            .child(header_detail),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_1p5()
                            .items_center()
                            .child(
                                div()
                                    .px_1p5()
                                    .py(px(3.))
                                    .rounded_md()
                                    .bg(if filter_active {
                                        palette.text_accent.opacity(0.14)
                                    } else {
                                        palette.element_background
                                    })
                                    .text_xs()
                                    .text_color(if filter_active {
                                        palette.text_accent
                                    } else {
                                        palette.text_muted
                                    })
                                    .child(if filter_active { "filtered" } else { "tree" }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(palette.text_muted)
                                    .line_clamp(1)
                                    .text_ellipsis()
                                    .child(filter_summary),
                            ),
                    ),
            )
            .child(if row_count == 0 {
                div()
                    .flex_1()
                    .flex()
                    .justify_center()
                    .items_center()
                    .px_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(palette.text)
                                    .child("No matching sessions"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(palette.text_muted)
                                    .child("Adjust the search query or clear the filter."),
                            ),
                    )
                    .into_any_element()
            } else {
                uniform_list(
                    "shell-sidebar-rows",
                    row_count,
                    cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                        let palette = this.palette();
                        range
                            .map(|ix| this.sidebar_row(ix, cx, &palette))
                            .collect::<Vec<_>>()
                    }),
                )
                .track_scroll(&self.sidebar_scroll_handle)
                .h_full()
                .into_any_element()
            })
            .into_any_element()
    }

    fn sidebar_row(&self, ix: usize, cx: &mut Context<Self>, palette: &UiPalette) -> AnyElement {
        let Some(row) = self.browser.rows().get(ix).cloned() else {
            return div().into_any_element();
        };
        let is_selected = self
            .browser
            .selected_path()
            .is_some_and(|path| path == row.full_path);

        let disclosure = match row.kind {
            BrowserRowKind::Group if row.expanded => "▾",
            BrowserRowKind::Group => "▸",
            BrowserRowKind::Session => {
                if is_selected {
                    "●"
                } else {
                    "○"
                }
            }
        };

        let detail = if row.kind == BrowserRowKind::Session {
            row.detail_label.clone()
        } else {
            row.detail_label.clone()
        };
        let is_top_group = row.kind == BrowserRowKind::Group && row.depth == 0;
        let has_detail = !detail.is_empty();
        let label_color = if is_selected {
            palette.text
        } else if is_top_group {
            palette.text
        } else if row.kind == BrowserRowKind::Session {
            palette.text
        } else {
            palette.text_muted
        };
        let disclosure_color = if is_selected {
            palette.text_accent
        } else {
            palette.text_muted
        };
        let guide_color = if is_selected {
            palette.border_focused.opacity(0.38)
        } else {
            palette.border_variant
        };
        let right_badge = match row.kind {
            BrowserRowKind::Group => Some(
                div()
                    .px_1p5()
                    .py_0p5()
                    .rounded_md()
                    .bg(if is_selected {
                        palette.text_accent.opacity(0.16)
                    } else {
                        palette.element_background
                    })
                    .text_xs()
                    .text_color(if is_selected {
                        palette.text_accent
                    } else {
                        palette.text_muted
                    })
                    .child(row.descendant_sessions.to_string()),
            ),
            BrowserRowKind::Session if !row.host_label.is_empty() => Some(
                div()
                    .px_1p5()
                    .py_0p5()
                    .rounded_md()
                    .bg(palette.element_background)
                    .text_xs()
                    .text_color(palette.text_muted)
                    .child(row.host_label.clone()),
            ),
            _ => None,
        };
        let guides = (0..row.depth)
            .map(|_guide_ix| {
                div()
                    .w(px(6.))
                    .text_xs()
                    .text_color(guide_color)
                    .child("│")
                    .into_any_element()
            })
            .collect::<Vec<_>>();

        let label = if is_top_group {
            div()
                .text_base()
                .text_color(label_color)
                .line_clamp(1)
                .text_ellipsis()
                .child(row.label.clone())
        } else {
            div()
                .text_sm()
                .text_color(label_color)
                .line_clamp(1)
                .text_ellipsis()
                .child(row.label.clone())
        };
        let mut body = div().flex().flex_col().gap_1().flex_1().child(
            div()
                .flex()
                .flex_row()
                .gap_1p5()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .gap_1p5()
                        .items_center()
                        .children(guides)
                        .child(
                            div()
                                .w(px(12.))
                                .text_color(disclosure_color)
                                .child(disclosure),
                        )
                        .child(label),
                )
                .children(right_badge),
        );
        if has_detail {
            body = body.child(
                div()
                    .pl(px(14.))
                    .pl(px(12.))
                    .text_xs()
                    .text_color(if is_selected {
                        palette.text_muted.opacity(0.9)
                    } else {
                        palette.text_muted
                    })
                    .line_clamp(1)
                    .text_ellipsis()
                    .child(detail),
            );
        }

        div()
            .id(("sidebar-row", ix))
            .w_full()
            .min_h(px(if has_detail { 34. } else { 27. }))
            .flex()
            .flex_row()
            .items_start()
            .justify_between()
            .px_1()
            .py(px(if has_detail { 5. } else { 3. }))
            .bg(if is_selected {
                palette.text_accent.opacity(0.14)
            } else if is_top_group {
                palette.element_background.opacity(0.32)
            } else {
                palette.window_background
            })
            .rounded_md()
            .border_l_2()
            .border_color(if is_selected {
                palette.border_focused.opacity(0.86)
            } else {
                transparent_black()
            })
            .on_click(cx.listener(move |this, _, _, cx| this.select_row(ix, cx)))
            .child(body)
            .into_any_element()
    }

    fn viewport(&self, cx: &mut Context<Self>, palette: &UiPalette) -> AnyElement {
        let body = match self.active_session() {
            Some(session) if self.server.active_view_mode() == WorkspaceViewMode::Terminal => {
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .child(
                                toolbar_chip_button("focus-ghostty", "Focus Ghostty", false, palette)
                                    .on_click(cx.listener(|this, _, _, cx| this.focus_ghostty_window(cx))),
                            )
                            .child(
                                toolbar_chip_button("resolve-window", "Resolve Window", false, palette)
                                    .on_click(cx.listener(|this, _, _, cx| this.resolve_ghostty_window(cx))),
                            ),
                    )
                    .child(terminal_surface_card(
                        "Server Terminal",
                        &session.terminal_lines,
                        Some(&session.status_line),
                        palette,
                    ))
                    .child(terminal_surface_card(
                        "Ghostty Integration",
                        &[
                            if self.bootstrap.ghostty_bridge_enabled {
                                if self.bootstrap.ghostty_embedded_surface_supported {
                                    "libghostty bridge is available and the embedded surface host is enabled on this platform."
                                } else {
                                    "libghostty is linked in this build, but the current upstream embedded surface host is not available on this platform."
                                }
                            } else {
                                "libghostty bridge is not linked in this build."
                            },
                            &self.bootstrap.ghostty_bridge_detail,
                        ],
                        Some("terminal"),
                        palette,
                    ))
                    .into_any_element()
            }
            Some(session) => {
                let subtitle = format!(
                    "{} · {} · {}",
                    self.metadata_value(session, "Host"),
                    self.metadata_value(session, "Started"),
                    self.metadata_value(session, "Messages")
                );
                let blocks = session
                    .preview
                    .blocks
                    .iter()
                    .enumerate()
                    .map(|(ix, block)| {
                        let grouped_with_previous = ix > 0
                            && session
                                .preview
                                .blocks
                                .get(ix - 1)
                                .is_some_and(|prev| prev.tone == block.tone);
                        self.preview_block(ix, block, grouped_with_previous, cx, palette)
                    })
                    .collect::<Vec<_>>();
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(preview_summary_card(
                                &session.title,
                                &subtitle,
                                "",
                                blocks.len(),
                                blocks.len(),
                                palette,
                            ))
                            .child(
                                div()
                                    .flex()
                                    .flex_row()
                                    .gap_2()
                                    .child(
                                        toolbar_chip_button("expand-preview", "Expand All", false, palette)
                                            .on_click(cx.listener(|this, _, _, cx| this.expand_preview_blocks(cx))),
                                    )
                                    .child(
                                        toolbar_chip_button("collapse-preview", "Collapse All", false, palette)
                                            .on_click(cx.listener(|this, _, _, cx| this.collapse_preview_blocks(cx))),
                                    ),
                            ),
                    )
                    .children(blocks)
                    .into_any_element()
            }
            None => div()
                .flex()
                .flex_col()
                .gap_3()
                .child(terminal_surface_card(
                    "No Session Selected",
                    &["Select a session from the sidebar to render its preview."],
                    Some("idle"),
                    palette,
                ))
                .into_any_element(),
        };

        div()
            .flex_1()
            .h_full()
            .flex()
            .flex_col()
            .bg(palette.surface_background)
            .child(
                div()
                    .id("viewport-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .scrollbar_width(px(10.))
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .justify_center()
                            .p_2()
                            .child(div().w_full().max_w(px(920.)).child(body)),
                    ),
            )
            .into_any_element()
    }

    fn preview_block(
        &self,
        block_ix: usize,
        block: &yggterm_core::SessionPreviewBlock,
        grouped_with_previous: bool,
        cx: &mut Context<Self>,
        palette: &UiPalette,
    ) -> AnyElement {
        div()
            .id(("preview-block", block_ix))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_preview_block(block_ix, cx)))
            .child(chat_preview_card(
                block.role,
                &block.timestamp,
                match block.tone {
                    PreviewTone::User => ChatBubbleTone::User,
                    PreviewTone::Assistant => ChatBubbleTone::Assistant,
                },
                grouped_with_previous,
                block.folded,
                "",
                &block.lines,
                palette,
            ))
            .into_any_element()
    }

    fn inspector(&self, palette: &UiPalette) -> AnyElement {
        if !self.right_panel_open {
            return div().into_any_element();
        }

        let inspector_body = if let Some(session) = self.active_session() {
            let overview_rows = vec![
                metadata_stat("Session", &session.title, palette),
                metadata_stat("Source", self.metadata_value(session, "Source"), palette),
                metadata_stat("Host", self.metadata_value(session, "Host"), palette),
            ];
            let timeline_rows = vec![
                metadata_stat("Started", self.metadata_value(session, "Started"), palette),
                metadata_stat("Updated", self.metadata_value(session, "Updated"), palette),
                metadata_stat(
                    "Messages",
                    self.metadata_value(session, "Messages"),
                    palette,
                ),
                metadata_stat(
                    "Preview",
                    self.metadata_value(session, "Preview Blocks"),
                    palette,
                ),
            ];
            let runtime_rows = vec![
                metadata_stat("Backend", self.metadata_value(session, "Backend"), palette),
                metadata_stat("Status", self.metadata_value(session, "Status"), palette),
                metadata_stat("Bridge", self.metadata_value(session, "Bridge"), palette),
                metadata_stat("PID", self.metadata_value(session, "Launch PID"), palette),
            ];
            let location_rows = vec![
                metadata_stat("Cwd", self.metadata_value(session, "Cwd"), palette),
                metadata_stat("Storage", self.metadata_value(session, "Storage"), palette),
                metadata_stat("Restore", self.metadata_value(session, "Restore"), palette),
            ];

            vec![
                metadata_section_card("Overview", overview_rows, palette),
                metadata_section_card("Timeline", timeline_rows, palette),
                metadata_section_card("Runtime", runtime_rows, palette),
                metadata_section_card("Location", location_rows, palette),
            ]
        } else {
            vec![metadata_section_card(
                "Session Metadata",
                vec![metadata_stat("State", "No session selected", palette)],
                palette,
            )]
        };

        div()
            .w(px(224.))
            .h_full()
            .flex()
            .flex_col()
            .bg(palette.window_background)
            .border_l_1()
            .border_color(palette.border_variant.opacity(0.84))
            .child(
                div()
                    .px_3()
                    .py_1()
                    .border_b_1()
                    .border_color(palette.border_variant.opacity(0.84))
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.text_muted)
                            .child("Session Metadata"),
                    ),
            )
            .child(
                div()
                    .id("inspector-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .scrollbar_width(px(10.))
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .children(inspector_body),
            )
            .into_any_element()
    }

    fn statusbar(&self, cx: &mut Context<Self>, palette: &UiPalette) -> AnyElement {
        statusbar_frame(
            div()
                .flex()
                .flex_row()
                .gap_3()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.text)
                        .child(format!("{} codex sessions", self.total_leaf_sessions())),
                )
                .child(div().text_xs().text_color(palette.text).child(format!(
                            "{} view · {}",
                            match self.server.active_view_mode() {
                                WorkspaceViewMode::Rendered => "preview",
                                WorkspaceViewMode::Terminal => "terminal",
                            },
                            self.active_session()
                                .map(|session| session.title.as_str())
                                .unwrap_or("none")
                        )))
                .into_any_element(),
            div()
                .flex()
                .flex_row()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.text_muted)
                        .child(self.last_action.clone()),
                )
                .child(
                    titlebar_icon_button("status-meta", TitlebarIcon::Info, palette)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_right_panel(cx))),
                )
                .into_any_element(),
            palette.window_background,
            palette.border_variant.opacity(0.92),
        )
        .into_any_element()
    }
}

impl Render for GpuiShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette();
        let decorations = window.window_decorations();
        let frame_inset = px(6.);
        let border_size = px(1.);
        let frame_border = palette.border.opacity(0.64);
        let frame_radius = px(9.);

        window.set_client_inset(px(0.));

        let content = div()
            .size_full()
            .flex()
            .flex_col()
            .bg(palette.window_background)
            .border_1()
            .border_color(frame_border)
            .cursor(CursorStyle::Arrow)
            .when(matches!(decorations, Decorations::Client { .. }), |div| {
                div.rounded_tl(frame_radius)
                    .rounded_tr(frame_radius)
                    .rounded_bl(frame_radius)
                    .rounded_br(frame_radius)
                    .overflow_hidden()
            })
            .on_mouse_move(|_, _, cx| {
                cx.stop_propagation();
            })
            .child(self.titlebar(window, cx, &palette))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_row()
                    .child(self.sidebar(cx, &palette))
                    .child(self.viewport(cx, &palette))
                    .child(self.inspector(&palette)),
            )
            .child(self.statusbar(cx, &palette));

        div()
            .size_full()
            .bg(transparent_black())
            .when(matches!(decorations, Decorations::Client { .. }), |div| {
                div.rounded_tl(frame_radius)
                    .rounded_tr(frame_radius)
                    .rounded_bl(frame_radius)
                    .rounded_br(frame_radius)
                    .overflow_hidden()
            })
            .map(|div| match decorations {
                Decorations::Server => div.child(content),
                Decorations::Client { tiling, .. } => div
                    .child(
                        canvas(
                            |_bounds, window, _cx| {
                                window.insert_hitbox(
                                    Bounds::new(
                                        point(px(0.), px(0.)),
                                        window.window_bounds().get_bounds().size,
                                    ),
                                    HitboxBehavior::Normal,
                                )
                            },
                            move |_bounds, hitbox, window, _cx| {
                                let mouse = window.mouse_position();
                                let size = window.window_bounds().get_bounds().size;
                                let Some(edge) = resize_edge(mouse, frame_inset, size) else {
                                    return;
                                };
                                window.set_cursor_style(
                                    match edge {
                                        ResizeEdge::Top | ResizeEdge::Bottom => {
                                            CursorStyle::ResizeUpDown
                                        }
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
                    )
                    .on_mouse_move(|_, window, _cx| {
                        window.refresh();
                    })
                    .on_mouse_down(MouseButton::Left, move |e, window, cx| {
                        let size = window.window_bounds().get_bounds().size;
                        if let Some(edge) = resize_edge(e.position, frame_inset, size) {
                            cx.stop_propagation();
                            window.start_window_resize(edge);
                        }
                    })
                    .child(content.map(|div| {
                        div.when(!tiling.top, |div| div.border_t(border_size))
                            .when(!tiling.bottom, |div| div.border_b(border_size))
                            .when(!tiling.left, |div| div.border_l(border_size))
                            .when(!tiling.right, |div| div.border_r(border_size))
                    })),
            })
    }
}

fn resize_edge(pos: gpui::Point<Pixels>, inset: Pixels, size: Size<Pixels>) -> Option<ResizeEdge> {
    let edge = if pos.y < inset && pos.x < inset {
        ResizeEdge::TopLeft
    } else if pos.y < inset && pos.x > size.width - inset {
        ResizeEdge::TopRight
    } else if pos.y < inset {
        ResizeEdge::Top
    } else if pos.y > size.height - inset && pos.x < inset {
        ResizeEdge::BottomLeft
    } else if pos.y > size.height - inset && pos.x > size.width - inset {
        ResizeEdge::BottomRight
    } else if pos.y > size.height - inset {
        ResizeEdge::Bottom
    } else if pos.x < inset {
        ResizeEdge::Left
    } else if pos.x > size.width - inset {
        ResizeEdge::Right
    } else {
        return None;
    };
    Some(edge)
}

fn metadata_stat(label: &str, value: &str, palette: &UiPalette) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap_0p5()
        .child(
            div()
                .text_xs()
                .text_color(palette.text_muted)
                .child(label.to_string()),
        )
        .child(
            div()
                .text_sm()
                .text_color(palette.text)
                .whitespace_normal()
                .child(value.to_string()),
        )
        .into_any_element()
}
