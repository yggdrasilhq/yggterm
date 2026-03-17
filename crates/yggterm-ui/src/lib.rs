use gpui::Div;
use gpui::{AnyElement, Hsla, Stateful, Window, div, px};
use ui::{
    ButtonSize, ButtonStyle, Color, IconButton, IconButtonShape, IconName, IconSize, h_flex,
    prelude::*,
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

pub fn titlebar_frame(
    left: AnyElement,
    center: AnyElement,
    window: &mut Window,
    background: Hsla,
    border: Hsla,
) -> Stateful<Div> {
    h_flex()
        .id("yggterm-titlebar")
        .w_full()
        .h(px(40.))
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
                .px_2()
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

fn window_control(
    id: &'static str,
    icon: IconName,
    on_click: impl Fn(&mut Window) + 'static,
) -> impl IntoElement {
    IconButton::new(id, icon)
        .shape(IconButtonShape::Square)
        .size(ButtonSize::Medium)
        .icon_size(IconSize::Small)
        .icon_color(Color::Muted)
        .style(ButtonStyle::Transparent)
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_click(window);
        })
}
