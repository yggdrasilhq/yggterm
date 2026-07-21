use dioxus::prelude::*;

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

/// The reveal wiring for an auto-hidden side rail: the host owns the state
/// machine (hover / linger / focus) and hands the rail only handlers to call —
/// the rail never decides when it is revealed. `None` keeps the classic docked
/// rail with no hover behaviour.
#[derive(Clone, Copy, PartialEq)]
pub struct SideRailReveal {
    pub on_reveal: EventHandler<()>,
    pub on_reveal_if_idle: EventHandler<()>,
    pub on_mouse_leave: EventHandler<()>,
    pub on_focus_within: EventHandler<bool>,
}

#[component]
pub fn SideRailShell(
    /// Docked (in-flow) or auto-hidden — drives the `data-*` state attributes.
    visible: bool,
    auto_hide: bool,
    revealed: bool,
    /// Fully-formed outer + content styles, authored by the host so the left
    /// tree and the right rail share ONE geometry helper (`sidebar_panel_*`).
    /// The host emits a FIXED property key set across every mode; the rail must
    /// not add or drop keys here (Dioxus applies `style` property-by-property and
    /// does not clear dropped keys — that lingering was the docked-rail-ghost bug).
    outer_style: String,
    content_style: String,
    /// Present only for an auto-hidden rail. Ignored when docked.
    reveal: Option<SideRailReveal>,
    /// A resize grip on the rail's inner (left) edge — the host authors it so the
    /// drag machinery stays in the shell. `None` for a non-resizable rail.
    resize_handle: Option<Element>,
    body: Element,
) -> Element {
    rsx! {
        div {
            "data-yggui-side-rail": "1",
            "data-yggui-side-rail-visible": if visible { "1" } else { "0" },
            "data-yggui-side-rail-auto-hide": if auto_hide { "1" } else { "0" },
            "data-yggui-side-rail-autohide-revealed": if revealed { "1" } else { "0" },
            "data-covers-web-surface": if revealed { "sidebar-right" },
            style: outer_style,
            onmousedown: |evt| evt.stop_propagation(),
            onclick: |evt| evt.stop_propagation(),
            onmouseenter: move |_| {
                if let Some(reveal) = reveal.as_ref() {
                    reveal.on_reveal.call(());
                }
            },
            onmousemove: move |_| {
                if let Some(reveal) = reveal.as_ref() {
                    reveal.on_reveal_if_idle.call(());
                }
            },
            onmouseleave: move |_| {
                if let Some(reveal) = reveal.as_ref() {
                    reveal.on_mouse_leave.call(());
                }
            },
            onfocusin: move |_| {
                if let Some(reveal) = reveal.as_ref() {
                    reveal.on_focus_within.call(true);
                }
            },
            onfocusout: move |_| {
                if let Some(reveal) = reveal.as_ref() {
                    reveal.on_focus_within.call(false);
                }
            },
            div {
                "data-yggui-side-rail-content": "1",
                style: content_style,
                {body}
                // The grip rides the CARD's inner edge (inside it), so it moves
                // with the card in every mode — docked and revealed-overlay alike
                // — exactly as the left tree's grip lives inside its content card.
                if let Some(resize_handle) = resize_handle {
                    {resize_handle}
                }
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
