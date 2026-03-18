use anyhow::Result;
use gpui::{
    AnyElement, App, AppContext, Bounds, Context, FocusHandle, InteractiveElement, IntoElement,
    KeyBinding, ParentElement, Render, ScrollStrategy, StatefulInteractiveElement, Styled,
    UniformListScrollHandle, Window, WindowBackgroundAppearance, WindowBounds, WindowDecorations,
    WindowOptions, actions, div, px, size, uniform_list,
};
use gpui_platform::application;
use yggterm_core::{
    BrowserRow, BrowserRowKind, ManagedSessionView, PreviewTone, SessionBrowserState,
    SessionMetadataEntry, SessionNode, UiTheme, WorkspaceViewMode, YggtermServer,
};

use crate::{
    ChatBubbleTone, TitlebarIcon, ToggleState, UiPalette, chat_preview_card, preview_summary_card,
    statusbar_frame, terminal_surface_card, titlebar_frame, titlebar_icon_button,
    titlebar_mode_toggle, toolbar_chip_button,
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
                window_background: WindowBackgroundAppearance::Opaque,
                window_decorations: Some(WindowDecorations::Server),
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

    fn connect_ssh_target(&mut self, target_ix: usize, cx: &mut Context<Self>) {
        if let Some(key) = self.server.connect_ssh_target(target_ix) {
            self.last_action = format!("ssh session {key}");
            cx.notify();
        }
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
            .w(px(380.))
            .h(px(30.))
            .flex()
            .items_center()
            .justify_between()
            .px_3()
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
            .gap_2()
            .items_center()
            .child(
                div()
                    .text_sm()
                    .text_color(palette.text_muted)
                    .child("yggterm"),
            )
            .child(titlebar_mode_toggle(
                "preview-toggle",
                "Preview",
                self.server.active_view_mode() == WorkspaceViewMode::Rendered,
                cx.listener(|this, state, _, cx| {
                    this.set_view_mode(
                        if *state == ToggleState::Selected {
                            WorkspaceViewMode::Rendered
                        } else {
                            WorkspaceViewMode::Terminal
                        },
                        cx,
                    )
                }),
                palette,
            ))
            .child(
                titlebar_icon_button("toggle-sidebar", TitlebarIcon::Sidebar, palette)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_sidebar(cx))),
            )
            .into_any_element();

        let right = div()
            .flex()
            .flex_row()
            .gap_2()
            .items_center()
            .child(
                toolbar_chip_button("connect-ssh", "SSH", false, palette)
                    .on_click(cx.listener(|this, _, _, cx| this.connect_ssh_target(0, cx))),
            )
            .child(
                titlebar_icon_button("toggle-meta", TitlebarIcon::Info, palette)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_right_panel(cx))),
            )
            .into_any_element();

        titlebar_frame(
            left,
            self.search_bar(window, cx, palette),
            right,
            palette.window_background,
            palette.border,
        )
        .into_any_element()
    }

    fn sidebar(&self, cx: &mut Context<Self>, palette: &UiPalette) -> AnyElement {
        if !self.sidebar_open {
            return div().into_any_element();
        }

        let row_count = self.browser.rows().len();
        div()
            .w(px(224.))
            .h_full()
            .flex()
            .flex_col()
            .bg(palette.window_background)
            .border_r_1()
            .border_color(palette.border)
            .child(
                div()
                    .px_3()
                    .py_2p5()
                    .border_b_1()
                    .border_color(palette.border_variant)
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.text)
                            .child("Codex Sessions"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(palette.text_muted)
                            .child(format!(
                                "{} stored · {} visible{}",
                                self.browser.total_session_count(),
                                self.total_leaf_sessions(),
                                self.active_session()
                                    .map(|session| format!(" · viewing {}", session.title))
                                    .unwrap_or_default()
                            )),
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

        let glyph = match row.kind {
            BrowserRowKind::Group => {
                if row.expanded {
                    "▾"
                } else {
                    "▸"
                }
            }
            BrowserRowKind::Session => "•",
        };

        let detail = if row.kind == BrowserRowKind::Session {
            row.detail_label.clone()
        } else {
            format!(
                "{} sessions{}",
                row.descendant_sessions,
                if row.detail_label.is_empty() {
                    String::new()
                } else {
                    format!(" · {}", row.detail_label)
                }
            )
        };

        div()
            .id(("sidebar-row", ix))
            .w_full()
            .min_h(px(42.))
            .flex()
            .flex_row()
            .items_start()
            .justify_between()
            .px_2()
            .py_2()
            .pl(px(12. + row.depth as f32 * 14.))
            .bg(if is_selected {
                palette.text_accent.opacity(0.14)
            } else {
                palette.window_background
            })
            .border_l_2()
            .border_color(if is_selected {
                palette.border_focused
            } else {
                palette.window_background
            })
            .on_click(cx.listener(move |this, _, _, cx| this.select_row(ix, cx)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .flex_1()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .items_center()
                            .child(div().text_color(palette.text_muted).child(glyph))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(if row.kind == BrowserRowKind::Session {
                                        palette.text
                                    } else {
                                        palette.text_muted
                                    })
                                    .line_clamp(1)
                                    .text_ellipsis()
                                    .child(row.label),
                            ),
                    )
                    .child(
                        div()
                            .pl(px(18.))
                            .text_xs()
                            .text_color(palette.text_muted)
                            .line_clamp(1)
                            .text_ellipsis()
                            .child(detail),
                    ),
            )
            .into_any_element()
    }

    fn viewport(&self, cx: &mut Context<Self>, palette: &UiPalette) -> AnyElement {
        let selected_path = self
            .active_session()
            .map(|session| session.session_path.clone())
            .or_else(|| self.selected_row().map(|row| row.full_path.clone()))
            .unwrap_or_else(|| "~/.yggterm/sessions".to_string());

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
                let blocks = session
                    .preview
                    .blocks
                    .iter()
                    .enumerate()
                    .map(|(ix, block)| self.preview_block(ix, block, cx, palette))
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
                            .child(preview_summary_card("Conversation", "", blocks.len(), blocks.len(), palette))
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
                    .w_full()
                    .h(px(34.))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(palette.border_variant)
                    .bg(palette.window_background)
                    .child(
                        div()
                            .text_sm()
                            .text_color(palette.text_muted)
                            .child(selected_path),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .gap_2()
                            .child(
                                toolbar_chip_button(
                                    "view-preview",
                                    "Preview",
                                    self.server.active_view_mode() == WorkspaceViewMode::Rendered,
                                    palette,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.set_view_mode(WorkspaceViewMode::Rendered, cx)
                                    },
                                )),
                            )
                            .child(
                                toolbar_chip_button(
                                    "view-terminal",
                                    "Terminal",
                                    self.server.active_view_mode() == WorkspaceViewMode::Terminal,
                                    palette,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.set_view_mode(WorkspaceViewMode::Terminal, cx)
                                    },
                                )),
                            ),
                    ),
            )
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
                            .p_3()
                            .child(div().w_full().max_w(px(980.)).child(body)),
                    ),
            )
            .into_any_element()
    }

    fn preview_block(
        &self,
        block_ix: usize,
        block: &yggterm_core::SessionPreviewBlock,
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

        let metadata = self
            .active_session()
            .map(|session| session.metadata.clone())
            .unwrap_or_default();

        div()
            .w(px(280.))
            .h_full()
            .flex()
            .flex_col()
            .bg(palette.window_background)
            .border_l_1()
            .border_color(palette.border)
            .child(
                div()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(palette.border_variant)
                    .child(
                        div()
                            .text_sm()
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
                    .children(
                        metadata
                            .into_iter()
                            .map(|entry| metadata_row(entry, palette)),
                    ),
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
                        .text_sm()
                        .text_color(palette.text)
                        .child(format!("{} codex sessions", self.total_leaf_sessions())),
                )
                .child(div().text_sm().text_color(palette.text).child(format!(
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
                        .text_sm()
                        .text_color(palette.text_muted)
                        .child(self.last_action.clone()),
                )
                .child(
                    titlebar_icon_button("status-meta", TitlebarIcon::Info, palette)
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_right_panel(cx))),
                )
                .into_any_element(),
            palette.window_background,
            palette.border,
        )
        .into_any_element()
    }
}

impl Render for GpuiShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = self.palette();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(palette.window_background)
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
            .child(self.statusbar(cx, &palette))
    }
}

fn metadata_row(entry: SessionMetadataEntry, palette: &UiPalette) -> AnyElement {
    div()
        .px_3()
        .py_2p5()
        .border_b_1()
        .border_color(palette.border_variant.opacity(0.65))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(palette.text_muted)
                        .child(entry.label),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(palette.text)
                        .whitespace_normal()
                        .child(entry.value),
                ),
        )
        .into_any_element()
}
