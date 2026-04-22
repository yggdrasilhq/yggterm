use dioxus::desktop::window;
use dioxus::html::input_data::MouseButton;
use dioxus::prelude::*;

use crate::motion::standard_transition;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HoveredChromeControl {
    AlwaysOnTop,
    Minimize,
    Maximize,
    Fullscreen,
    Close,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChromeControlIcon {
    AlwaysOnTop,
    Minimize,
    Maximize,
    Restore,
    Fullscreen,
    ExitFullscreen,
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
    pub is_dark: bool,
}

#[component]
pub fn TitlebarChrome(
    background: String,
    zoom_percent: f32,
    left: Element,
    center: Element,
    right: Element,
    on_request_window_drag: EventHandler<()>,
    on_toggle_maximized: EventHandler<()>,
) -> Element {
    const TITLEBAR_DRAG_THRESHOLD_PX: f64 = 5.0;
    let mut pending_drag_origin = use_signal(|| None::<(f64, f64)>);
    rsx! {
        div {
            style: format!(
                "position:relative; display:grid; grid-template-columns:minmax(0,1fr) minmax(260px, 560px) minmax(0,1fr); align-items:center; \
                 gap:6px; height:32px; padding:0 8px 0 8px; box-sizing:border-box; background:{}; zoom:{}%; user-select:none; overflow:visible; \
                 -webkit-user-select:none;",
                background, zoom_percent
            ),
            onmousedown: move |evt| {
                if matches!(
                    evt.trigger_button(),
                    Some(button) if button != MouseButton::Primary
                ) {
                    return;
                }
                evt.prevent_default();
                let pointer = evt.client_coordinates();
                pending_drag_origin.set(Some((pointer.x, pointer.y)));
                on_request_window_drag.call(());
                window().drag();
            },
            onmousemove: move |evt| {
                let Some((start_x, start_y)) = pending_drag_origin() else {
                    return;
                };
                if !evt.held_buttons().contains(MouseButton::Primary) {
                    pending_drag_origin.set(None);
                    return;
                }
                let pointer = evt.client_coordinates();
                let delta_x = pointer.x - start_x;
                let delta_y = pointer.y - start_y;
                if delta_x.hypot(delta_y) >= TITLEBAR_DRAG_THRESHOLD_PX {
                    pending_drag_origin.set(None);
                    on_request_window_drag.call(());
                    window().drag();
                }
            },
            onmouseup: move |evt| {
                if matches!(
                    evt.trigger_button(),
                    Some(button) if button != MouseButton::Primary
                ) {
                    return;
                }
                let Some(_) = pending_drag_origin() else {
                    return;
                };
                evt.prevent_default();
                pending_drag_origin.set(None);
            },
            ondoubleclick: move |evt| {
                if matches!(
                    evt.trigger_button(),
                    Some(button) if button != MouseButton::Primary
                ) {
                    return;
                }
                evt.prevent_default();
                pending_drag_origin.set(None);
                on_toggle_maximized.call(());
            },
            div {
                style: "position:relative; z-index:1; min-width:0; height:100%; display:flex; align-items:center; justify-content:flex-start; box-sizing:border-box; pointer-events:none;",
                div {
                    style: "display:inline-flex; align-items:center; justify-content:flex-start; min-width:0; max-width:100%; height:100%; pointer-events:auto;",
                    {left}
                }
            }
            div {
                style: "position:relative; z-index:1; min-width:0; height:100%; display:flex; align-items:center; justify-content:center; box-sizing:border-box; pointer-events:none;",
                div {
                    style: "display:flex; align-items:center; justify-content:center; min-width:0; width:100%; height:100%; pointer-events:auto;",
                    {center}
                }
            }
            div {
                style: "position:relative; z-index:1; min-width:0; height:100%; display:flex; align-items:center; justify-content:flex-end; box-sizing:border-box; pointer-events:none;",
                div {
                    style: "display:inline-flex; align-items:center; justify-content:flex-end; min-width:0; max-width:100%; height:100%; pointer-events:auto;",
                    {right}
                }
            }
        }
    }
}

#[component]
pub fn WindowControlsStrip(
    palette: ChromePalette,
    hovered: Option<HoveredChromeControl>,
    maximized: bool,
    fullscreen: bool,
    always_on_top: bool,
    show_always_on_top_button: bool,
    show_fullscreen_button: bool,
    show_window_buttons: bool,
    overlay: bool,
    on_hover_control: EventHandler<Option<HoveredChromeControl>>,
    on_toggle_maximized: EventHandler<()>,
    on_toggle_fullscreen: EventHandler<()>,
    on_toggle_always_on_top: EventHandler<()>,
    on_close_app: EventHandler<()>,
) -> Element {
    let container_style = if overlay {
        "display:flex; align-items:stretch; gap:6px; padding:8px; border-radius:14px; \
         background:rgba(255,255,255,0.78); box-shadow:0 16px 36px rgba(96,124,158,0.18), \
         inset 0 0 0 1px rgba(198,212,226,0.72); backdrop-filter:blur(10px); -webkit-backdrop-filter:blur(10px);"
    } else {
        "display:flex; align-items:stretch; gap:0;"
    };
    rsx! {
        div {
            style: container_style,
            if show_always_on_top_button {
                WindowControlButton {
                    icon: ChromeControlIcon::AlwaysOnTop,
                    hovered: hovered == Some(HoveredChromeControl::AlwaysOnTop),
                    active: always_on_top,
                    hover_tone: HoveredChromeControl::AlwaysOnTop,
                    palette: palette,
                    overlay: overlay,
                    on_hover_control: on_hover_control,
                    on_press: move |_| on_toggle_always_on_top.call(()),
                }
            }
            if show_fullscreen_button {
                WindowControlButton {
                    icon: if fullscreen {
                        ChromeControlIcon::ExitFullscreen
                    } else {
                        ChromeControlIcon::Fullscreen
                    },
                    hovered: hovered == Some(HoveredChromeControl::Fullscreen),
                    active: fullscreen,
                    hover_tone: HoveredChromeControl::Fullscreen,
                    palette: palette,
                    overlay: overlay,
                    on_hover_control: on_hover_control,
                    on_press: move |_| on_toggle_fullscreen.call(()),
                }
            }
            if show_window_buttons {
                WindowControlButton {
                    icon: ChromeControlIcon::Minimize,
                    hovered: hovered == Some(HoveredChromeControl::Minimize),
                    active: false,
                    hover_tone: HoveredChromeControl::Minimize,
                    palette: palette,
                    overlay: overlay,
                    on_hover_control: on_hover_control,
                    on_press: move |_| window().set_minimized(true),
                }
                WindowControlButton {
                    icon: if maximized {
                        ChromeControlIcon::Restore
                    } else {
                        ChromeControlIcon::Maximize
                    },
                    hovered: hovered == Some(HoveredChromeControl::Maximize),
                    active: false,
                    hover_tone: HoveredChromeControl::Maximize,
                    palette: palette,
                    overlay: overlay,
                    on_hover_control: on_hover_control,
                    on_press: move |_| on_toggle_maximized.call(()),
                }
                WindowControlButton {
                    icon: ChromeControlIcon::Close,
                    hovered: hovered == Some(HoveredChromeControl::Close),
                    active: false,
                    hover_tone: HoveredChromeControl::Close,
                    palette: palette,
                    overlay: overlay,
                    on_hover_control: on_hover_control,
                    on_press: move |_| on_close_app.call(()),
                }
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
    overlay: bool,
    on_hover_control: EventHandler<Option<HoveredChromeControl>>,
    on_press: EventHandler<MouseEvent>,
) -> Element {
    let is_close = hover_tone == HoveredChromeControl::Close;
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
    } else if active {
        palette.accent
    } else if palette.is_dark {
        "#d7e3ee"
    } else {
        palette.text
    };
    let button_style = if overlay {
        format!(
            "width:32px; height:30px; border:none; border-radius:10px; background:{}; color:{}; \
             display:flex; align-items:center; justify-content:center; font-size:13px; font-weight:600; \
             user-select:none; -webkit-user-select:none; transition:{};",
            background,
            color,
            standard_transition(&["background-color", "color"])
        )
    } else {
        format!(
            "width:34px; height:30px; border:none; border-radius:0; background:{}; color:{}; \
             display:flex; align-items:center; justify-content:center; font-size:13px; font-weight:600; \
             user-select:none; -webkit-user-select:none; transition:{};",
            background,
            color,
            standard_transition(&["background-color", "color"])
        )
    };
    rsx! {
        button {
            style: button_style,
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
        ChromeControlIcon::Fullscreen => rsx! {
            svg { width: "11", height: "11", view_box: "0 0 10 10", fill: "none", xmlns: "http://www.w3.org/2000/svg",
                path { d: "M4.1 3.1L2.5 1.5", stroke: "currentColor", stroke_width: "1.1", stroke_linecap: "round", stroke_linejoin: "round" }
                path { d: "M2.5 3.5V1.5H4.5", stroke: "currentColor", stroke_width: "1.1", stroke_linecap: "round", stroke_linejoin: "round" }
                path { d: "M5.9 6.9L7.5 8.5", stroke: "currentColor", stroke_width: "1.1", stroke_linecap: "round", stroke_linejoin: "round" }
                path { d: "M5.5 8.5H7.5V6.5", stroke: "currentColor", stroke_width: "1.1", stroke_linecap: "round", stroke_linejoin: "round" }
            }
        },
        ChromeControlIcon::ExitFullscreen => rsx! {
            svg { width: "11", height: "11", view_box: "0 0 10 10", fill: "none", xmlns: "http://www.w3.org/2000/svg",
                path { d: "M2.7 3.3L4.3 4.9", stroke: "currentColor", stroke_width: "1.1", stroke_linecap: "round", stroke_linejoin: "round" }
                path { d: "M2.7 4.9H4.3V3.3", stroke: "currentColor", stroke_width: "1.1", stroke_linecap: "round", stroke_linejoin: "round" }
                path { d: "M7.3 6.7L5.7 5.1", stroke: "currentColor", stroke_width: "1.1", stroke_linecap: "round", stroke_linejoin: "round" }
                path { d: "M5.7 6.7H7.3V5.1", stroke: "currentColor", stroke_width: "1.1", stroke_linecap: "round", stroke_linejoin: "round" }
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

pub fn search_field_shell_style(dark_surface: bool) -> String {
    format!(
        "position:relative; display:flex; align-items:center; width:100%; min-width:0; height:26px; padding:0 8px; \
         min-height:26px; max-height:26px; flex:1 1 auto; border-radius:10px; background:{}; box-sizing:border-box; box-shadow:{}; \
         transition:{};",
        if dark_surface {
            "rgba(9,13,17,0.94)"
        } else {
            "rgba(255,255,255,0.96)"
        },
        if dark_surface {
            "inset 0 0 0 1px rgba(214,229,242,0.22), 0 1px 0 rgba(255,255,255,0.05)"
        } else {
            "inset 0 0 0 1px rgba(198,212,226,0.82), 0 1px 0 rgba(255,255,255,0.38)"
        },
        standard_transition(&["background-color", "box-shadow"])
    )
}

pub fn search_input_style(text_color: &str, _dark_surface: bool) -> String {
    format!(
        "width:100%; height:100%; padding:0 3px; border:none; background:transparent; color:{}; outline:none; \
         box-sizing:border-box; display:block; margin:0; min-width:0; flex:1 1 auto; appearance:none; -webkit-appearance:none; border-radius:0; \
         font-size:13.5px; font-weight:550; letter-spacing:-0.012em; line-height:1; \
         font-family:'Inter Variable', Inter, system-ui, sans-serif; text-rendering:optimizeLegibility; \
         -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale; \
         box-shadow:none; user-select:text; -webkit-user-select:text; caret-color:{};",
        text_color, text_color
    )
}
