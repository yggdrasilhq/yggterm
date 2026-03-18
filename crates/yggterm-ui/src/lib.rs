use gpui::Div;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, FontWeight, Hsla, InteractiveElement, IntoElement, MouseButton, Stateful,
    StatefulInteractiveElement, Styled, Window, div, hsla, px, relative,
};

mod shell;

pub use shell::{ShellBootstrap, launch_shell};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UiPalette {
    pub window_background: Hsla,
    pub border: Hsla,
    pub border_variant: Hsla,
    pub border_focused: Hsla,
    pub surface_background: Hsla,
    pub element_background: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub text_accent: Hsla,
}

impl UiPalette {
    pub fn light() -> Self {
        Self {
            window_background: hsla(0., 0., 0.97, 1.0),
            border: hsla(0., 0., 0.82, 1.0),
            border_variant: hsla(0., 0., 0.88, 1.0),
            border_focused: hsla(0.59, 0.72, 0.46, 1.0),
            surface_background: hsla(0., 0., 0.99, 1.0),
            element_background: hsla(0., 0., 0.94, 1.0),
            text: hsla(0., 0., 0.17, 1.0),
            text_muted: hsla(0., 0., 0.42, 1.0),
            text_accent: hsla(0.59, 0.72, 0.46, 1.0),
        }
    }

    pub fn dark() -> Self {
        Self {
            window_background: hsla(0.61, 0.13, 0.15, 1.0),
            border: hsla(0.61, 0.11, 0.26, 1.0),
            border_variant: hsla(0.61, 0.10, 0.22, 1.0),
            border_focused: hsla(0.59, 0.72, 0.46, 1.0),
            surface_background: hsla(0.61, 0.13, 0.18, 1.0),
            element_background: hsla(0.61, 0.13, 0.20, 1.0),
            text: hsla(0., 0., 0.92, 1.0),
            text_muted: hsla(0., 0., 0.66, 1.0),
            text_accent: hsla(0.59, 0.72, 0.62, 1.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToggleState {
    Selected,
    Unselected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TitlebarIcon {
    ConnectSsh,
    Info,
    Sidebar,
}

pub fn render_session_tree_text(root: &yggterm_core::SessionNode) -> anyhow::Result<String> {
    let mut out = String::new();
    out.push_str("Yggdrasil Terminal Session Tree\n");
    out.push_str("================================\n");
    render_node(root, 0, &mut out);
    Ok(out)
}

fn render_node(node: &yggterm_core::SessionNode, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    out.push_str(&format!("{}- {}\n", indent, node.name));
    for child in &node.children {
        render_node(child, depth + 1, out);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatBubbleTone {
    Neutral,
    User,
    Assistant,
}

pub fn titlebar_frame(
    left: AnyElement,
    center: AnyElement,
    right: AnyElement,
    background: Hsla,
    border: Hsla,
) -> Stateful<Div> {
    row()
        .id("yggterm-titlebar")
        .w_full()
        .h(px(40.))
        .items_center()
        .justify_between()
        .bg(background)
        .border_b_1()
        .border_color(border)
        .on_mouse_down(MouseButton::Left, |_, window, cx: &mut App| {
            cx.stop_propagation();
            window.start_window_move();
        })
        .child(
            div()
                .w(px(228.))
                .flex_none()
                .h_full()
                .flex()
                .items_center()
                .justify_start()
                .px_2()
                .child(left),
        )
        .child(
            div()
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .px_1p5()
                .child(center),
        )
        .child(
            div()
                .w(px(228.))
                .flex_none()
                .h_full()
                .flex()
                .items_center()
                .justify_end()
                .gap_1()
                .px_2()
                .child(right),
        )
}

pub fn statusbar_frame(
    left: AnyElement,
    right: AnyElement,
    background: Hsla,
    border: Hsla,
) -> Stateful<Div> {
    row()
        .id("yggterm-statusbar")
        .w_full()
        .h(px(26.))
        .items_center()
        .justify_between()
        .px_2p5()
        .bg(background)
        .border_t_1()
        .border_color(border)
        .child(left)
        .child(right)
}

pub fn window_controls(window: &mut Window) -> AnyElement {
    let supported = window.window_controls();
    let controls = row()
        .id("yggterm-window-controls")
        .gap_1()
        .items_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx: &mut App| {
            cx.stop_propagation()
        });

    #[cfg(target_os = "macos")]
    let controls = controls
        .child(traffic_light(
            "close",
            hsla(0.01, 0.78, 0.58, 1.0),
            |window| {
                window.remove_window();
            },
        ))
        .when(supported.minimize, |row| {
            row.child(traffic_light(
                "minimize",
                hsla(0.13, 0.94, 0.63, 1.0),
                |window| {
                    window.minimize_window();
                },
            ))
        })
        .when(supported.maximize, |row| {
            row.child(traffic_light(
                "maximize-or-restore",
                hsla(0.36, 0.65, 0.49, 1.0),
                |window| {
                    window.zoom_window();
                },
            ))
        });

    #[cfg(not(target_os = "macos"))]
    let controls = controls
        .when(supported.minimize, |row| {
            row.child(window_control("minimize", "−", move |window| {
                window.minimize_window();
            }))
        })
        .when(supported.maximize, |row| {
            row.child(window_control(
                "maximize-or-restore",
                if window.is_maximized() { "❐" } else { "□" },
                move |window| {
                    window.zoom_window();
                },
            ))
        })
        .child(window_control("close", "×", move |window| {
            window.remove_window();
        }));

    controls.into_any_element()
}

pub fn titlebar_icon_button(
    id: &'static str,
    icon: TitlebarIcon,
    palette: &UiPalette,
) -> Stateful<Div> {
    chrome_button(
        id,
        icon_glyph(icon),
        palette.text_muted,
        palette.element_background,
        palette.border_variant,
    )
}

pub fn toolbar_chip_button(
    id: &'static str,
    label: impl Into<String>,
    selected: bool,
    palette: &UiPalette,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(26.))
        .flex()
        .flex_row()
        .items_center()
        .justify_center()
        .px_2p5()
        .rounded_md()
        .bg(if selected {
            palette.text_accent.opacity(0.12)
        } else {
            palette.element_background
        })
        .border_1()
        .border_color(if selected {
            palette.border_focused.opacity(0.92)
        } else {
            palette.border_variant.opacity(0.9)
        })
        .text_xs()
        .text_color(if selected {
            palette.text
        } else {
            palette.text_muted
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx: &mut App| {
            cx.stop_propagation();
        })
        .child(label.into())
}

pub fn titlebar_mode_toggle(
    id: &'static str,
    label: &str,
    enabled: bool,
    on_toggle: impl Fn(&ToggleState, &mut Window, &mut App) + 'static,
    palette: &UiPalette,
) -> AnyElement {
    row()
        .gap_1p5()
        .items_center()
        .px_1p5()
        .py_0p5()
        .rounded_full()
        .bg(palette.element_background)
        .border_1()
        .border_color(if enabled {
            palette.border_focused
        } else {
            palette.border_variant
        })
        .child(
            div()
                .text_xs()
                .text_color(palette.text_muted)
                .child(label.to_string()),
        )
        .child(toggle_pill(enabled, palette))
        .id(id)
        .on_mouse_down(MouseButton::Left, |_, _, cx: &mut App| {
            cx.stop_propagation();
        })
        .on_click(move |_, window, cx| {
            let next = if enabled {
                ToggleState::Unselected
            } else {
                ToggleState::Selected
            };
            on_toggle(&next, window, cx);
        })
        .into_any_element()
}

fn window_control(
    id: &'static str,
    glyph: &'static str,
    on_click: impl Fn(&mut Window) + 'static,
) -> impl IntoElement {
    chrome_button(
        id,
        glyph,
        hsla(0., 0., 0.7, 1.0),
        hsla(0., 0., 0.0, 0.0),
        hsla(0., 0., 0.0, 0.0),
    )
    .on_click(move |_, window, cx| {
        cx.stop_propagation();
        on_click(window);
    })
}

fn traffic_light(
    id: &'static str,
    background: Hsla,
    on_click: impl Fn(&mut Window) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .size(px(12.))
        .rounded_full()
        .bg(background)
        .border_1()
        .border_color(background.opacity(0.75))
        .on_mouse_down(MouseButton::Left, |_, _, cx: &mut App| {
            cx.stop_propagation();
        })
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_click(window);
        })
}

pub fn terminal_surface_card<S: AsRef<str>>(
    title: &'static str,
    lines: &[S],
    badge: Option<&str>,
    palette: &UiPalette,
) -> AnyElement {
    let badge = badge.unwrap_or("session").to_string();
    column()
        .gap_2()
        .p_3p5()
        .rounded_lg()
        .bg(palette.surface_background)
        .border_1()
        .border_color(palette.border_variant.opacity(0.94))
        .child(
            row()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .text_color(palette.text)
                        .child(title.to_string()),
                )
                .child(div().text_sm().text_color(palette.text_muted).child(badge)),
        )
        .children(
            lines
                .iter()
                .enumerate()
                .map(|(ix, line)| {
                    let line = line.as_ref();
                    div()
                        .text_sm()
                        .text_color(if line.starts_with('$') {
                            palette.text_accent
                        } else {
                            palette.text
                        })
                        .line_height(relative(1.35))
                        .child(format!("{:02}  {line}", ix + 1))
                        .into_any_element()
                })
                .collect::<Vec<_>>(),
        )
        .into_any_element()
}

pub fn preview_summary_card(
    title: &str,
    subtitle: &str,
    query: &str,
    matching_blocks: usize,
    total_blocks: usize,
    palette: &UiPalette,
) -> AnyElement {
    column()
        .w(px(308.))
        .gap_1p5()
        .p_2p5()
        .rounded_lg()
        .bg(palette.surface_background)
        .border_1()
        .border_color(palette.border_variant.opacity(0.94))
        .child(
            row()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(palette.text)
                        .child(title.to_string()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(palette.text_muted)
                        .child(format!("{matching_blocks}/{total_blocks} blocks")),
                ),
        )
        .when(!subtitle.is_empty(), |this| {
            this.child(
                div()
                    .text_xs()
                    .text_color(palette.text_muted)
                    .line_clamp(1)
                    .child(subtitle.to_string()),
            )
        })
        .when(!query.is_empty(), |this| {
            this.child(
                div()
                    .text_sm()
                    .text_color(palette.text_muted)
                    .child(format!("Filtered by \"{query}\"")),
            )
        })
        .into_any_element()
}

pub fn metadata_section_card(
    title: &str,
    rows: Vec<AnyElement>,
    palette: &UiPalette,
) -> AnyElement {
    column()
        .gap_1p5()
        .p_2()
        .rounded_lg()
        .bg(palette.surface_background.opacity(0.88))
        .border_1()
        .border_color(palette.border_variant.opacity(0.92))
        .child(
            div()
                .text_xs()
                .text_color(palette.text_muted)
                .child(title.to_string()),
        )
        .children(rows)
        .into_any_element()
}

pub fn chat_preview_card(
    _role: &str,
    timestamp: &str,
    tone: ChatBubbleTone,
    grouped_with_previous: bool,
    folded: bool,
    query: &str,
    lines: &[String],
    palette: &UiPalette,
) -> AnyElement {
    let (bg, border, footer_color) = match tone {
        ChatBubbleTone::User => (
            palette.text_accent.opacity(0.12),
            palette.text_accent.opacity(0.26),
            palette.text_accent,
        ),
        ChatBubbleTone::Assistant => (
            palette.surface_background,
            palette.border_variant,
            palette.text_muted,
        ),
        ChatBubbleTone::Neutral => (
            palette.surface_background,
            palette.border_variant,
            palette.text_muted,
        ),
    };

    div()
        .w_full()
        .flex()
        .pt(if grouped_with_previous {
            px(2.)
        } else {
            px(12.)
        })
        .pb(px(2.))
        .justify_start()
        .when(matches!(tone, ChatBubbleTone::User), |this| {
            this.justify_end()
        })
        .child(
            column()
                .w_full()
                .max_w(px(match tone {
                    ChatBubbleTone::User => 700.,
                    ChatBubbleTone::Assistant | ChatBubbleTone::Neutral => 840.,
                }))
                .gap_2p5()
                .px_3p5()
                .py_3()
                .rounded_xl()
                .bg(bg)
                .border_1()
                .border_color(border)
                .when(!grouped_with_previous, |this| {
                    this.child(div().h(px(3.)).w(px(36.)).rounded_full().bg(match tone {
                        ChatBubbleTone::User => palette.text_accent.opacity(0.55),
                        ChatBubbleTone::Assistant => palette.border_focused.opacity(0.35),
                        ChatBubbleTone::Neutral => palette.border_variant,
                    }))
                })
                .when(!query.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(palette.text_muted)
                            .child(format!("Matched by \"{query}\"")),
                    )
                })
                .when(!folded, |this| {
                    this.children(render_preview_segments(lines, tone, palette))
                })
                .when(folded, |this| {
                    this.child(
                        div()
                            .text_sm()
                            .text_color(palette.text_muted)
                            .child("Collapsed"),
                    )
                })
                .child(
                    row()
                        .w_full()
                        .justify_end()
                        .gap_1()
                        .items_center()
                        .when(folded, |this| {
                            this.child(
                                div()
                                    .text_xs()
                                    .text_color(palette.text_muted)
                                    .child("collapsed"),
                            )
                        })
                        .child(
                            div()
                                .text_xs()
                                .text_color(footer_color)
                                .child(timestamp.to_string()),
                        ),
                ),
        )
        .into_any_element()
}

fn row() -> Div {
    div().flex().flex_row()
}

fn column() -> Div {
    div().flex().flex_col()
}

fn render_preview_segments(
    lines: &[String],
    tone: ChatBubbleTone,
    palette: &UiPalette,
) -> Vec<AnyElement> {
    let mut segments = Vec::new();
    let mut prose = Vec::<String>::new();
    let mut code = Vec::<String>::new();

    let flush_prose = |segments: &mut Vec<AnyElement>, prose: &mut Vec<String>| {
        if prose.is_empty() {
            return;
        }
        let chunk = std::mem::take(prose);
        segments.push(
            column()
                .gap_2()
                .children(
                    chunk
                        .into_iter()
                        .map(|line| {
                            div()
                                .text_base()
                                .line_height(relative(1.45))
                                .text_color(palette.text)
                                .whitespace_normal()
                                .child(line)
                                .into_any_element()
                        })
                        .collect::<Vec<_>>(),
                )
                .into_any_element(),
        );
    };

    let flush_code = |segments: &mut Vec<AnyElement>, code: &mut Vec<String>| {
        if code.is_empty() {
            return;
        }
        let chunk = std::mem::take(code);
        segments.push(
            column()
                .gap_1()
                .px_2p5()
                .py_2()
                .rounded_md()
                .bg(match tone {
                    ChatBubbleTone::User => palette.text_accent.opacity(0.10),
                    ChatBubbleTone::Assistant | ChatBubbleTone::Neutral => {
                        palette.window_background.opacity(0.55)
                    }
                })
                .border_1()
                .border_color(match tone {
                    ChatBubbleTone::User => palette.text_accent.opacity(0.24),
                    ChatBubbleTone::Assistant | ChatBubbleTone::Neutral => palette.border_variant,
                })
                .children(
                    chunk
                        .into_iter()
                        .map(|line| {
                            div()
                                .text_sm()
                                .line_height(relative(1.35))
                                .text_color(palette.text)
                                .whitespace_nowrap()
                                .child(line)
                                .into_any_element()
                        })
                        .collect::<Vec<_>>(),
                )
                .into_any_element(),
        );
    };

    for line in lines {
        if is_code_line(line) {
            flush_prose(&mut segments, &mut prose);
            code.push(line.clone());
        } else {
            flush_code(&mut segments, &mut code);
            prose.push(line.clone());
        }
    }

    flush_prose(&mut segments, &mut prose);
    flush_code(&mut segments, &mut code);
    segments
}

fn is_code_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }

    if trimmed.starts_with("```")
        || trimmed.starts_with("$ ")
        || trimmed.starts_with("> ")
        || trimmed.starts_with("fn ")
        || trimmed.starts_with("pub ")
        || trimmed.starts_with("impl ")
        || trimmed.starts_with("use ")
        || trimmed.starts_with("let ")
        || trimmed.starts_with("const ")
        || trimmed.starts_with("struct ")
        || trimmed.starts_with("enum ")
        || trimmed.starts_with("match ")
        || trimmed.starts_with("if ")
        || trimmed.starts_with("for ")
        || trimmed.starts_with("while ")
        || trimmed.starts_with("return ")
        || trimmed.starts_with("cargo ")
        || trimmed.starts_with("git ")
        || trimmed.starts_with("cd ")
        || trimmed.starts_with("ssh ")
        || trimmed.starts_with('{')
        || trimmed.starts_with('}')
    {
        return true;
    }

    trimmed.contains("::")
        || trimmed.contains("->")
        || trimmed.contains(" = ")
        || trimmed.ends_with('{')
        || trimmed.ends_with("};")
        || trimmed.ends_with(")]")
}

fn icon_glyph(icon: TitlebarIcon) -> &'static str {
    match icon {
        TitlebarIcon::ConnectSsh => "⇄",
        TitlebarIcon::Info => "i",
        TitlebarIcon::Sidebar => "☰",
    }
}

fn toggle_pill(enabled: bool, palette: &UiPalette) -> Div {
    row()
        .w(px(30.))
        .h(px(17.))
        .rounded_full()
        .items_center()
        .justify_start()
        .px_0p5()
        .bg(if enabled {
            palette.text_accent.opacity(0.28)
        } else {
            palette.border_variant
        })
        .child(
            div()
                .size(px(11.))
                .rounded_full()
                .bg(if enabled {
                    palette.text_accent
                } else {
                    palette.text_muted
                })
                .when(enabled, |this| this.ml_auto()),
        )
}

fn chrome_button(
    id: &'static str,
    glyph: &'static str,
    text_color: Hsla,
    background: Hsla,
    border: Hsla,
) -> Stateful<Div> {
    div()
        .id(id)
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .bg(background)
        .border_1()
        .border_color(border)
        .text_sm()
        .text_color(text_color)
        .on_mouse_down(MouseButton::Left, |_, _, cx: &mut App| {
            cx.stop_propagation();
        })
        .child(glyph)
}
