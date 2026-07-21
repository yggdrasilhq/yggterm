use dioxus::prelude::*;

use crate::motion::{emphasized_enter_transition, emphasized_exit_transition};

const RAIL_SCROLLBAR_CSS: &str = r#"
.yggui-rail-scroll {
  scrollbar-width: thin;
  scrollbar-color: var(--yggui-scrollbar-thumb, rgba(255,255,255,0.18))
    var(--yggui-scrollbar-track, transparent);
  scrollbar-gutter: stable;
}

.yggui-rail-scroll::-webkit-scrollbar {
  width: 10px;
  height: 10px;
}

.yggui-rail-scroll::-webkit-scrollbar-track {
  background: var(--yggui-scrollbar-track, transparent);
}

.yggui-rail-scroll::-webkit-scrollbar-corner {
  background: transparent;
}

.yggui-rail-scroll::-webkit-scrollbar-thumb {
  background: var(--yggui-scrollbar-thumb, rgba(255,255,255,0.18));
  border-radius: 999px;
  border: 2px solid transparent;
  background-clip: padding-box;
  min-height: 32px;
}

.yggui-rail-scroll:hover::-webkit-scrollbar-thumb {
  background: var(--yggui-scrollbar-thumb-hover, rgba(255,255,255,0.28));
  border: 2px solid transparent;
  background-clip: padding-box;
}
"#;

/// How a hidden side rail behaves: gone, or revealed on hover as a floating
/// overlay. The host owns the reveal state machine (hover/linger/focus) and hands
/// the rail only the two styles it must paint with — the rail never decides when
/// it is revealed.
#[derive(Clone, PartialEq, Props)]
pub struct SideRailOverlay {
    /// Fully-formed outer style for the floating box (positioning, width,
    /// z-index, chrome). Authored by the host so both sidebars share ONE
    /// geometry helper.
    pub outer_style: String,
    /// Fully-formed style for the content layer, which holds the rail's full
    /// width at all times so nothing re-wraps as the box animates.
    pub content_style: String,
    pub revealed: bool,
}

#[component]
pub fn SideRailShell(
    visible: bool,
    width_px: usize,
    zoom_percent: f32,
    /// Present when the rail is HIDDEN and should hover-reveal over the viewport
    /// instead of collapsing to nothing. `None` keeps the classic in-flow rail.
    overlay: Option<SideRailOverlay>,
    /// Pointer/focus handlers for the overlay's edge. Ignored without `overlay`.
    on_reveal: Option<EventHandler<()>>,
    on_reveal_if_idle: Option<EventHandler<()>>,
    on_mouse_leave: Option<EventHandler<()>>,
    on_focus_within: Option<EventHandler<bool>>,
    body: Element,
) -> Element {
    let rail_width = if visible { width_px } else { 0 };
    let opacity = if visible { "1" } else { "0" };
    let translate = if visible {
        "translateX(0)"
    } else {
        "translateX(14px)"
    };
    let pointer_events = if visible { "auto" } else { "none" };
    let transition = if visible {
        emphasized_enter_transition(&["width", "min-width", "max-width", "opacity", "transform"])
    } else {
        emphasized_exit_transition(&["width", "min-width", "max-width", "opacity", "transform"])
    };
    let in_flow_style = format!(
        "width:{}px; min-width:{}px; max-width:{}px; display:flex; flex-direction:column; \
         background:transparent; overflow:hidden; text-rendering:optimizeLegibility; \
         -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale; \
         transition:{}; opacity:{}; transform:{}; \
         pointer-events:{}; zoom:{}%;",
        rail_width,
        rail_width,
        rail_width,
        transition,
        opacity,
        translate,
        pointer_events,
        zoom_percent
    );
    let overlay_revealed = overlay.as_ref().is_some_and(|overlay| overlay.revealed);
    let outer_style = match overlay.as_ref() {
        Some(overlay) => format!(
            "{} text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; \
             -moz-osx-font-smoothing:grayscale; zoom:{}%;",
            overlay.outer_style, zoom_percent
        ),
        None => in_flow_style,
    };
    let content_style = match overlay.as_ref() {
        Some(overlay) => overlay.content_style.clone(),
        None => "display:flex; flex-direction:column; flex:1; min-height:0; width:100%; min-width:0;"
            .to_string(),
    };
    rsx! {
        div {
            "data-yggui-side-rail": "1",
            "data-yggui-side-rail-visible": if visible { "1" } else { "0" },
            "data-yggui-side-rail-auto-hide": if overlay.is_some() { "1" } else { "0" },
            "data-yggui-side-rail-autohide-revealed": if overlay_revealed { "1" } else { "0" },
            "data-covers-web-surface": if overlay_revealed { "sidebar-right" },
            style: outer_style,
            onmousedown: |evt| evt.stop_propagation(),
            onclick: |evt| evt.stop_propagation(),
            onmouseenter: move |_| {
                if let Some(handler) = on_reveal.as_ref() {
                    handler.call(());
                }
            },
            onmousemove: move |_| {
                if let Some(handler) = on_reveal_if_idle.as_ref() {
                    handler.call(());
                }
            },
            onmouseleave: move |_| {
                if let Some(handler) = on_mouse_leave.as_ref() {
                    handler.call(());
                }
            },
            onfocusin: move |_| {
                if let Some(handler) = on_focus_within.as_ref() {
                    handler.call(true);
                }
            },
            onfocusout: move |_| {
                if let Some(handler) = on_focus_within.as_ref() {
                    handler.call(false);
                }
            },
            div {
                "data-yggui-side-rail-content": "1",
                style: content_style,
                {body}
            }
        }
    }
}

/// A rail's section heading, with an optional trailing action cluster.
///
/// `actions` is where a rail hangs the verbs that ACT ON the section it heads
/// (the tab rail's new-tab and new-folder buttons). They belong on the heading
/// rather than in a band below it: a heading row is one line the eye already
/// reads, and a verb parked next to its noun needs no label to explain itself.
#[component]
pub fn RailHeader(title: String, color: String, actions: Option<Element>) -> Element {
    rsx! {
        div {
            "data-yggui-rail-header": "1",
            style: "display:flex; align-items:center; gap:8px; padding:16px 16px 10px 16px;",
            span {
                style: format!(
                    "flex:1 1 auto; min-width:0; font-size:12px; font-weight:700; letter-spacing:0.01em; color:{}; \
                     text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                    color
                ),
                "{title}"
            }
            if let Some(actions) = actions {
                div {
                    style: "flex:0 0 auto; display:flex; align-items:center; gap:6px;",
                    {actions}
                }
            }
        }
    }
}

#[component]
pub fn RailScrollBody(content: Element) -> Element {
    rsx! {
        style { "{RAIL_SCROLLBAR_CSS}" }
        div {
            "data-yggui-rail-scroll": "1",
            class: "yggui-rail-scroll",
            style: "flex:1; overflow:auto; padding:10px 16px 32px 16px; display:flex; flex-direction:column; gap:14px; scroll-padding-block:16px 32px; \
             text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
            {content}
        }
    }
}

#[component]
pub fn RailSectionTitle(title: String, muted_color: String) -> Element {
    rsx! {
        div {
            style: format!(
                "font-size:11px; font-weight:700; letter-spacing:0.02em; color:{}; \
                 text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                muted_color
            ),
            "{title}"
        }
    }
}
