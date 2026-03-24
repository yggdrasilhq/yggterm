use dioxus::desktop::window;
use dioxus::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HoveredChromeControl {
    AlwaysOnTop,
    Minimize,
    Maximize,
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChromeControlIcon {
    AlwaysOnTop,
    Minimize,
    Maximize,
    Restore,
    Close,
}

#[derive(Clone, Copy, PartialEq)]
pub struct ChromePalette {
    pub titlebar: &'static str,
    pub text: &'static str,
    pub muted: &'static str,
    pub accent: &'static str,
    pub close_hover: &'static str,
    pub control_hover: &'static str,
}

#[component]
pub fn TitlebarChrome(
    background: String,
    zoom_percent: f32,
    left: Element,
    center: Element,
    right: Element,
    on_toggle_maximized: EventHandler<()>,
) -> Element {
    let mut drag_armed = use_signal(|| false);
    rsx! {
        div {
            style: format!(
                "display:flex; align-items:center; justify-content:space-between; height:44px; \
                 padding:0 12px; background:{}; zoom:{}%; user-select:none; -webkit-user-select:none;",
                background, zoom_percent
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
            {left}
            {center}
            {right}
        }
    }
}

#[component]
pub fn WindowControlsStrip(
    palette: ChromePalette,
    hovered: Option<HoveredChromeControl>,
    maximized: bool,
    always_on_top: bool,
    on_hover_control: EventHandler<Option<HoveredChromeControl>>,
    on_toggle_maximized: EventHandler<()>,
    on_toggle_always_on_top: EventHandler<()>,
) -> Element {
    rsx! {
        div {
            style: "display:flex; align-items:stretch; gap:0;",
            WindowControlButton {
                icon: ChromeControlIcon::AlwaysOnTop,
                hovered: hovered == Some(HoveredChromeControl::AlwaysOnTop),
                active: always_on_top,
                hover_tone: HoveredChromeControl::AlwaysOnTop,
                palette: palette,
                on_hover_control: on_hover_control,
                on_press: move |_| on_toggle_always_on_top.call(()),
            }
            WindowControlButton {
                icon: ChromeControlIcon::Minimize,
                hovered: hovered == Some(HoveredChromeControl::Minimize),
                active: false,
                hover_tone: HoveredChromeControl::Minimize,
                palette: palette,
                on_hover_control: on_hover_control,
                on_press: move |_| window().set_minimized(true),
            }
            WindowControlButton {
                icon: if maximized { ChromeControlIcon::Restore } else { ChromeControlIcon::Maximize },
                hovered: hovered == Some(HoveredChromeControl::Maximize),
                active: false,
                hover_tone: HoveredChromeControl::Maximize,
                palette: palette,
                on_hover_control: on_hover_control,
                on_press: move |_| on_toggle_maximized.call(()),
            }
            WindowControlButton {
                icon: ChromeControlIcon::Close,
                hovered: hovered == Some(HoveredChromeControl::Close),
                active: false,
                hover_tone: HoveredChromeControl::Close,
                palette: palette,
                on_hover_control: on_hover_control,
                on_press: move |_| window().close(),
            }
        }
    }
}

#[component]
fn WindowControlButton(
    icon: ChromeControlIcon,
    hovered: bool,
    active: bool,
    hover_tone: HoveredChromeControl,
    palette: ChromePalette,
    on_hover_control: EventHandler<Option<HoveredChromeControl>>,
    on_press: EventHandler<MouseEvent>,
) -> Element {
    let is_close = hover_tone == HoveredChromeControl::Close;
    let background = if hovered {
        if is_close { palette.close_hover } else { palette.control_hover }
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
fn WindowControlGlyph(icon: ChromeControlIcon) -> Element {
    match icon {
        ChromeControlIcon::AlwaysOnTop => rsx! {
            svg { width: "12", height: "12", view_box: "0 0 12 12", fill: "none", xmlns: "http://www.w3.org/2000/svg",
                path { d: "M3.1 5.2L6 2.4L8.9 5.2", stroke: "currentColor", stroke_width: "1.2", stroke_linecap: "round", stroke_linejoin: "round" }
                path { d: "M3.1 9.2L6 6.4L8.9 9.2", stroke: "currentColor", stroke_width: "1.2", stroke_linecap: "round", stroke_linejoin: "round" }
            }
        },
        ChromeControlIcon::Minimize => rsx! {
            svg { width: "11", height: "11", view_box: "0 0 10 10", fill: "none", xmlns: "http://www.w3.org/2000/svg",
                path { d: "M2 5.5H8", stroke: "currentColor", stroke_width: "1.1", stroke_linecap: "round" }
            }
        },
        ChromeControlIcon::Maximize => rsx! {
            svg { width: "11", height: "11", view_box: "0 0 10 10", fill: "none", xmlns: "http://www.w3.org/2000/svg",
                rect { x: "2.1", y: "2.1", width: "5.8", height: "5.8", stroke: "currentColor", stroke_width: "1.1" }
            }
        },
        ChromeControlIcon::Restore => rsx! {
            svg { width: "11", height: "11", view_box: "0 0 10 10", fill: "none", xmlns: "http://www.w3.org/2000/svg",
                path { d: "M3.2 2.1H7.7V6.6", stroke: "currentColor", stroke_width: "1.1", stroke_linejoin: "round" }
                path { d: "M2.3 3.4H6.8V7.9H2.3V3.4Z", stroke: "currentColor", stroke_width: "1.1", stroke_linejoin: "round" }
            }
        },
        ChromeControlIcon::Close => rsx! {
            svg { width: "11", height: "11", view_box: "0 0 10 10", fill: "none", xmlns: "http://www.w3.org/2000/svg",
                path { d: "M2.6 2.6L7.4 7.4", stroke: "currentColor", stroke_width: "1.1", stroke_linecap: "round" }
                path { d: "M7.4 2.6L2.6 7.4", stroke: "currentColor", stroke_width: "1.1", stroke_linecap: "round" }
            }
        },
    }
}

pub fn search_input_style(text_color: &str) -> String {
    format!(
        "width:min(560px, 100%); height:32px; padding:0 12px; border-radius:8px; \
         border:none; background:rgba(255,255,255,0.66); color:{}; outline:none; font-size:12px; \
         box-shadow: inset 0 0 0 1px rgba(255,255,255,0.36); user-select:text; -webkit-user-select:text;",
        text_color
    )
}
