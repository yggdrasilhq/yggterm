use crate::drag_tree::DragDropPlacement;
use dioxus::prelude::*;
use dioxus::html::input_data::MouseButton;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragGhostPalette {
    pub text: &'static str,
    pub muted: &'static str,
    pub accent: &'static str,
    pub accent_soft: &'static str,
}

#[component]
pub fn DragGhostCard(
    x: f64,
    y: f64,
    primary_label: String,
    extra_count: usize,
    target_hint: Option<String>,
    palette: DragGhostPalette,
) -> Element {
    rsx! {
        div {
            style: format!(
                "position:fixed; left:{}px; top:{}px; z-index:1600; pointer-events:none; \
                 transform:translate(16px, 10px); display:flex; flex-direction:column; gap:4px;",
                x, y
            ),
            if extra_count > 0 {
                div {
                    style: "position:absolute; inset:6px auto auto 10px; width:100%; pointer-events:none;",
                    div {
                        style: "width:100%; height:100%; min-width:150px; max-width:260px; padding:10px 12px; border-radius:12px; \
                                background:rgba(255,255,255,0.52); box-shadow:0 12px 28px rgba(72, 101, 128, 0.10), \
                                inset 0 0 0 1px rgba(201, 216, 230, 0.54); transform:translate(0px, 0px) rotate(-1.2deg);",
                    }
                    div {
                        style: "position:absolute; inset:0 auto auto 4px; width:100%; pointer-events:none;",
                        div {
                            style: "width:100%; height:100%; min-width:150px; max-width:260px; padding:10px 12px; border-radius:12px; \
                                    background:rgba(255,255,255,0.68); box-shadow:0 14px 34px rgba(72, 101, 128, 0.14), \
                                    inset 0 0 0 1px rgba(201, 216, 230, 0.70); transform:translate(0px, 0px) rotate(0.6deg);",
                        }
                    }
                }
            }
            div {
                style: format!(
                    "position:relative; min-width:150px; max-width:260px; padding:10px 12px; border-radius:12px; \
                     background:rgba(255,255,255,0.92); color:{}; box-shadow:0 18px 42px rgba(72, 101, 128, 0.22), \
                     inset 0 0 0 1px rgba(201, 216, 230, 0.92); backdrop-filter: blur(12px); \
                     -webkit-backdrop-filter: blur(12px);",
                    palette.text
                ),
                div {
                    style: "display:flex; align-items:center; gap:8px;",
                    div {
                        style: format!(
                            "width:9px; height:9px; border-radius:999px; background:{}; flex:none;",
                            palette.accent
                        ),
                    }
                    span {
                        style: "font-size:12px; font-weight:800; letter-spacing:0.01em; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;",
                        "{primary_label}"
                    }
                    if extra_count > 0 {
                        span {
                            style: format!(
                                "margin-left:auto; padding:2px 7px; border-radius:999px; font-size:10px; font-weight:800; \
                                 color:{}; background:{};",
                                palette.accent, palette.accent_soft
                            ),
                            "+{extra_count}"
                        }
                    }
                }
                if let Some(target_hint) = target_hint {
                    div {
                        style: format!(
                            "font-size:10.5px; font-weight:700; letter-spacing:0.01em; color:{}; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;",
                            palette.muted
                        ),
                        "{target_hint}"
                    }
                }
            }
        }
    }
}

#[component]
pub fn TreeDropZones(
    drag_active: bool,
    can_drop_inside: bool,
    on_drag_hover: EventHandler<(DragDropPlacement, MouseEvent)>,
    on_drop: EventHandler<()>,
    on_end_drag: EventHandler<()>,
) -> Element {
    if !drag_active {
        return rsx! {};
    }
    let commit = move || {
        on_drop.call(());
        on_end_drag.call(());
    };
    rsx! {
        div {
            style: "position:absolute; left:0; right:0; top:0; height:12px; z-index:2;",
            onmouseenter: move |evt| on_drag_hover.call((DragDropPlacement::Before, evt)),
            onmousemove: move |evt| on_drag_hover.call((DragDropPlacement::Before, evt)),
            onmouseup: move |_| commit(),
        }
        if can_drop_inside {
            div {
                style: "position:absolute; left:0; right:0; top:12px; bottom:12px; z-index:2;",
                onmouseenter: move |evt| on_drag_hover.call((DragDropPlacement::Into, evt)),
                onmousemove: move |evt| on_drag_hover.call((DragDropPlacement::Into, evt)),
                onmouseup: move |_| commit(),
            }
        }
        div {
            style: "position:absolute; left:0; right:0; bottom:0; height:12px; z-index:2;",
            onmouseenter: move |evt| on_drag_hover.call((DragDropPlacement::After, evt)),
            onmousemove: move |evt| on_drag_hover.call((DragDropPlacement::After, evt)),
            onmouseup: move |_| commit(),
        }
    }
}

#[component]
pub fn DragStartHandle(
    draggable: bool,
    on_start_drag: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        div {
            style: "position:absolute; inset:0; z-index:1; background:transparent;",
            onmousedown: move |evt| {
                if draggable
                    && evt.trigger_button() == Some(MouseButton::Primary)
                    && !evt.modifiers().contains(keyboard_types::Modifiers::SHIFT)
                    && !evt.modifiers().contains(keyboard_types::Modifiers::CONTROL)
                    && !evt.modifiers().contains(keyboard_types::Modifiers::META)
                {
                    evt.prevent_default();
                    on_start_drag.call(evt);
                }
            },
        }
    }
}
