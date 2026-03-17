use gpui::Div;
use gpui::{AnyElement, Hsla, Stateful, Window, div, px};
use theme::ThemeColors;
use ui::{
    ButtonSize, ButtonStyle, Color, Divider, IconButton, IconButtonShape, IconName, IconSize,
    Label, LabelSize, Switch, ToggleState, h_flex, prelude::*, v_flex,
};

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
    h_flex()
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
    h_flex()
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
    h_flex()
        .id("yggterm-window-controls")
        .gap_1()
        .items_center()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(window_control(
            "minimize",
            IconName::GenericMinimize,
            move |window| {
                window.minimize_window();
            },
        ))
        .child(window_control(
            "maximize-or-restore",
            if window.is_maximized() {
                IconName::GenericRestore
            } else {
                IconName::GenericMaximize
            },
            move |window| {
                window.zoom_window();
            },
        ))
        .child(window_control(
            "close",
            IconName::GenericClose,
            move |window| {
                window.remove_window();
            },
        ))
        .into_any_element()
}

pub fn titlebar_icon_button(id: &'static str, icon: IconName) -> IconButton {
    IconButton::new(id, icon)
        .shape(IconButtonShape::Square)
        .size(ButtonSize::Large)
        .icon_size(IconSize::Medium)
        .icon_color(Color::Muted)
        .style(ButtonStyle::Transparent)
}

pub fn titlebar_mode_toggle(
    id: &'static str,
    label: &str,
    enabled: bool,
    on_toggle: impl Fn(&ToggleState, &mut Window, &mut gpui::App) + 'static,
    colors: &ThemeColors,
) -> AnyElement {
    h_flex()
        .gap_2()
        .items_center()
        .px_2()
        .py_1()
        .rounded_full()
        .bg(colors.element_background)
        .border_1()
        .border_color(if enabled {
            colors.border_focused
        } else {
            colors.border_variant
        })
        .child(
            Label::new(label.to_string())
                .size(LabelSize::Small)
                .color(Color::Muted),
        )
        .child(
            Switch::new(
                id,
                if enabled {
                    ToggleState::Selected
                } else {
                    ToggleState::Unselected
                },
            )
            .on_click(on_toggle),
        )
        .into_any_element()
}

fn window_control(
    id: &'static str,
    icon: IconName,
    on_click: impl Fn(&mut Window) + 'static,
) -> impl IntoElement {
    titlebar_icon_button(id, icon).on_click(move |_, window, cx| {
        cx.stop_propagation();
        on_click(window);
    })
}

pub fn terminal_surface_card<S: AsRef<str>>(
    title: &'static str,
    lines: &[S],
    badge: Option<&str>,
    colors: &ThemeColors,
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
                .child(Label::new(badge).size(LabelSize::Small).color(Color::Muted)),
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

pub fn preview_summary_card(
    query: &str,
    matching_blocks: usize,
    total_blocks: usize,
    _colors: &ThemeColors,
) -> AnyElement {
    v_flex()
        .gap_2()
        .px_1()
        .pt_1()
        .pb_2()
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .child(Label::new("Conversation").size(LabelSize::Small))
                .child(
                    Label::new(format!("{matching_blocks}/{total_blocks} blocks"))
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                ),
        )
        .when(!query.is_empty(), |this| {
            this.child(
                Label::new(format!("Filtered by \"{query}\""))
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
        })
        .child(Divider::horizontal().inset())
        .into_any_element()
}

pub fn chat_preview_card(
    _role: &str,
    timestamp: &str,
    tone: ChatBubbleTone,
    folded: bool,
    query: &str,
    lines: &[String],
    colors: &ThemeColors,
) -> AnyElement {
    const LINE_RENDER_LIMIT: usize = 8;
    let (bg, border) = match tone {
        ChatBubbleTone::User => (
            colors.text_accent.opacity(0.10),
            colors.text_accent.opacity(0.28),
        ),
        ChatBubbleTone::Assistant => (colors.surface_background, colors.border_variant),
        ChatBubbleTone::Neutral => (colors.surface_background, colors.border_variant),
    };

    div()
        .w_full()
        .flex()
        .justify_start()
        .when(matches!(tone, ChatBubbleTone::User), |this| {
            this.justify_end()
        })
        .child(
            v_flex()
                .w(px(780.))
                .max_w_full()
                .gap_3()
                .p_3()
                .rounded_lg()
                .bg(bg)
                .border_1()
                .border_color(border)
                .when(!query.is_empty(), |this| {
                    this.child(
                        Label::new(format!("match: {query}"))
                            .size(LabelSize::Small)
                            .color(Color::Muted),
                    )
                })
                .when(!folded, |this| {
                    let hidden_line_count = lines.len().saturating_sub(LINE_RENDER_LIMIT);
                    this.children(
                        lines
                            .iter()
                            .take(LINE_RENDER_LIMIT)
                            .map(|line| {
                                Label::new(line.clone())
                                    .size(LabelSize::Default)
                                    .color(Color::Default)
                                    .into_any_element()
                            })
                            .collect::<Vec<_>>(),
                    )
                    .when(hidden_line_count > 0, |this| {
                        this.child(
                            Label::new(format!("… {} more lines", hidden_line_count))
                                .size(LabelSize::Small)
                                .color(Color::Muted),
                        )
                    })
                })
                .child(
                    h_flex()
                        .w_full()
                        .justify_end()
                        .gap_2()
                        .items_center()
                        .child(
                            Label::new(if folded { "collapsed" } else { "" })
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Label::new(timestamp.to_string())
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        ),
                ),
        )
        .into_any_element()
}
