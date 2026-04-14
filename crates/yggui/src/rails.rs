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

#[component]
pub fn SideRailShell(visible: bool, width_px: usize, zoom_percent: f32, body: Element) -> Element {
    let rail_width = if visible { width_px } else { 0 };
    let opacity = if visible { "1" } else { "0" };
    let translate = if visible {
        "translateX(0)"
    } else {
        "translateX(14px)"
    };
    let pointer_events = if visible { "auto" } else { "none" };
    rsx! {
        div {
            style: format!(
                "width:{}px; min-width:{}px; max-width:{}px; display:flex; flex-direction:column; \
                 background:transparent; overflow:hidden; text-rendering:optimizeLegibility; \
                 -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale; \
                 transition: width 180ms ease, min-width 180ms ease, max-width 180ms ease, opacity 180ms ease, transform 180ms ease; opacity:{}; transform:{}; \
                 pointer-events:{}; zoom:{}%;",
                rail_width, rail_width, rail_width, opacity, translate, pointer_events, zoom_percent
            ),
            {body}
        }
    }
}

#[component]
pub fn RailHeader(title: String, color: String) -> Element {
    rsx! {
        div {
            style: format!(
                "padding:16px 16px 10px 16px; font-size:12px; font-weight:700; letter-spacing:0.01em; color:{}; \
                 text-rendering:optimizeLegibility; -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                color
            ),
            "{title}"
        }
    }
}

#[component]
pub fn RailScrollBody(content: Element) -> Element {
    rsx! {
        style { "{RAIL_SCROLLBAR_CSS}" }
        div {
            class: "yggui-rail-scroll",
            style: "flex:1; overflow:auto; padding:10px 16px 14px 16px; display:flex; flex-direction:column; gap:14px; \
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
