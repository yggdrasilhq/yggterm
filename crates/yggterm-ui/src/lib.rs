use gpui::Div;
use gpui::{
    AnyElement, App, Hsla, InteractiveElement, IntoElement, MouseButton,
    Stateful, StatefulInteractiveElement, Styled, Window, div, hsla, px,
};
use gpui::prelude::*;

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
    window: &mut Window,
    background: Hsla,
    border: Hsla,
) -> Stateful<Div> {
    row()
        .id("yggterm-titlebar")
        .w_full()
        .h(px(42.))
        .items_center()
        .justify_between()
        .bg(background)
        .border_b_1()
        .border_color(border)
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
                .child(right)
                .child(window_controls(window)),
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
        .h(px(30.))
        .items_center()
        .justify_between()
        .px_2()
        .bg(background)
        .border_t_1()
        .border_color(border)
        .child(left)
        .child(right)
}

fn window_controls(window: &mut Window) -> AnyElement {
    row()
        .id("yggterm-window-controls")
        .gap_1()
        .items_center()
        .on_mouse_down(MouseButton::Left, |_, _, cx: &mut App| cx.stop_propagation())
        .child(window_control(
            "minimize",
            "−",
            move |window| {
                window.minimize_window();
            },
        ))
        .child(window_control(
            "maximize-or-restore",
            if window.is_maximized() { "❐" } else { "□" },
            move |window| {
                window.zoom_window();
            },
        ))
        .child(window_control(
            "close",
            "×",
            move |window| {
                window.remove_window();
            },
        ))
        .into_any_element()
}

pub fn titlebar_icon_button(id: &'static str, icon: TitlebarIcon, palette: &UiPalette) -> Stateful<Div> {
    chrome_button(id, icon_glyph(icon), palette.text_muted, palette.element_background, palette.border_variant)
}

pub fn titlebar_mode_toggle(
    id: &'static str,
    label: &str,
    enabled: bool,
    on_toggle: impl Fn(&ToggleState, &mut Window, &mut App) + 'static,
    palette: &UiPalette,
) -> AnyElement {
    row()
        .gap_2()
        .items_center()
        .px_2()
        .py_1()
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
                .text_sm()
                .text_color(palette.text_muted)
                .child(label.to_string()),
        )
        .child(toggle_pill(enabled, palette))
        .id(id)
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

pub fn terminal_surface_card<S: AsRef<str>>(
    title: &'static str,
    lines: &[S],
    badge: Option<&str>,
    palette: &UiPalette,
) -> AnyElement {
    let badge = badge.unwrap_or("session").to_string();
    column()
        .gap_2()
        .p_3()
        .rounded_md()
        .bg(palette.surface_background)
        .border_1()
        .border_color(palette.border_variant)
        .child(
            row()
                .items_center()
                .justify_between()
                .child(div().text_sm().text_color(palette.text).child(title.to_string()))
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
                        .child(format!("{:02}  {line}", ix + 1))
                        .into_any_element()
                })
                .collect::<Vec<_>>(),
        )
        .into_any_element()
}

pub fn preview_summary_card(
    query: &str,
    matching_blocks: usize,
    total_blocks: usize,
    palette: &UiPalette,
) -> AnyElement {
    column()
        .gap_2()
        .px_1()
        .pt_1()
        .pb_2()
        .child(
            row()
                .items_center()
                .justify_between()
                .child(div().text_sm().text_color(palette.text).child("Conversation"))
                .child(
                    div()
                        .text_sm()
                        .text_color(palette.text_muted)
                        .child(format!("{matching_blocks}/{total_blocks} blocks")),
                ),
        )
        .when(!query.is_empty(), |this| {
            this.child(
                div()
                    .text_sm()
                    .text_color(palette.text_muted)
                    .child(format!("Filtered by \"{query}\"")),
            )
        })
        .child(horizontal_divider(palette.border_variant))
        .into_any_element()
}

pub fn chat_preview_card(
    _role: &str,
    timestamp: &str,
    tone: ChatBubbleTone,
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
        .py_1()
        .justify_start()
        .when(matches!(tone, ChatBubbleTone::User), |this| {
            this.justify_end()
        })
        .child(
            column()
                .w_full()
                .max_w(px(match tone {
                    ChatBubbleTone::User => 760.,
                    ChatBubbleTone::Assistant | ChatBubbleTone::Neutral => 860.,
                }))
                .gap_2()
                .px_4()
                .py_3()
                .rounded_xl()
                .bg(bg)
                .border_1()
                .border_color(border)
                .when(!query.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(palette.text_muted)
                            .child(format!("Matched by \"{query}\"")),
                    )
                })
                .when(!folded, |this| {
                    this.children(
                        lines
                            .iter()
                            .map(|line| {
                                div()
                                    .text_sm()
                                    .text_color(palette.text)
                                    .whitespace_normal()
                                    .child(line.clone())
                                    .into_any_element()
                            })
                            .collect::<Vec<_>>(),
                    )
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

fn horizontal_divider(color: Hsla) -> Div {
    div().w_full().h(px(1.)).bg(color)
}

fn icon_glyph(icon: TitlebarIcon) -> &'static str {
    match icon {
        TitlebarIcon::ConnectSsh => "⇄",
    }
}

fn toggle_pill(enabled: bool, palette: &UiPalette) -> Div {
    row()
        .w(px(34.))
        .h(px(20.))
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
                .size(px(14.))
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
        .size(px(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .bg(background)
        .border_1()
        .border_color(border)
        .text_sm()
        .text_color(text_color)
        .child(glyph)
}
