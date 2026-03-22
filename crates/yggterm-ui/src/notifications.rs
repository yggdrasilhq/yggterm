use dioxus::prelude::*;

pub const TOAST_CSS: &str = r#"
@keyframes yggterm-toast-stack-in {
  0% { opacity: 0; transform: translateY(12px); }
  100% { opacity: 1; transform: translateY(0); }
}
@keyframes yggterm-toast-fade {
  0% { opacity: 0; transform: translateY(-4px); }
  8% { opacity: 1; transform: translateY(0); }
  78% { opacity: 1; transform: translateY(0); }
  100% { opacity: 0; transform: translateY(-6px); }
}
"#;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastTone {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, PartialEq)]
pub struct ToastItem {
    pub id: u64,
    pub tone: ToastTone,
    pub title: String,
    pub message: String,
    pub created_at_ms: u64,
}

#[derive(Clone, Copy, PartialEq)]
pub struct ToastPalette {
    pub text: &'static str,
    pub muted: &'static str,
    pub accent: &'static str,
}

#[component]
pub fn ToastViewport(
    items: Vec<ToastItem>,
    palette: ToastPalette,
    right_inset: usize,
    max_age_ms: u64,
    max_visible: usize,
    now_ms: u64,
    on_clear: EventHandler<u64>,
) -> Element {
    let visible = items
        .into_iter()
        .rev()
        .filter(|notification| now_ms.saturating_sub(notification.created_at_ms) <= max_age_ms)
        .filter(|notification| notification.tone != ToastTone::Info)
        .take(max_visible)
        .collect::<Vec<_>>();
    let stack_key = visible
        .iter()
        .map(|notification| notification.id.to_string())
        .collect::<Vec<_>>()
        .join("-");
    rsx! {
        div {
            key: "{stack_key}",
            style: format!(
                "position:fixed; top:56px; right:{}px; z-index:80; display:flex; flex-direction:column; gap:10px; width:280px; pointer-events:none;",
                right_inset
            ),
            for notification in visible {
                div {
                    key: "{notification.id}",
                    style: "pointer-events:auto; animation:yggterm-toast-stack-in 220ms ease both;",
                    div {
                        style: "animation:yggterm-toast-fade 7s ease forwards;",
                        ToastCard {
                            item: notification.clone(),
                            palette: palette,
                            on_clear: move |_| on_clear.call(notification.id),
                        }
                    }
                }
            }
        }
    }
}

#[component]
pub fn ToastCard(
    item: ToastItem,
    palette: ToastPalette,
    on_clear: EventHandler<MouseEvent>,
) -> Element {
    let (tone_accent, tone_fg) = toast_tone_colors(item.tone, palette);
    rsx! {
        div {
            style: "display:flex; flex-direction:column; gap:7px; padding:12px 12px 11px 12px; border-radius:14px; \
                    background:rgba(249,250,252,0.86); backdrop-filter: blur(28px) saturate(165%); \
                    -webkit-backdrop-filter: blur(28px) saturate(165%); box-shadow: 0 18px 38px rgba(49,67,82,0.14), inset 0 0 0 1px rgba(255,255,255,0.72);",
            div {
                style: "display:flex; align-items:center; justify-content:space-between; gap:8px;",
                div {
                    style: "display:flex; align-items:center; gap:8px; min-width:0;",
                    div {
                        style: format!(
                            "width:8px; height:8px; border-radius:999px; background:{}; flex:none;",
                            tone_accent
                        ),
                    }
                    div {
                        style: format!(
                            "font-size:12px; font-weight:700; color:{}; text-rendering:optimizeLegibility; \
                             -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                            tone_fg
                        ),
                        "{item.title}"
                    }
                }
                button {
                    style: format!(
                        "width:22px; height:22px; border:none; border-radius:8px; background:rgba(241,244,247,0.92); color:{}; font-size:12px; font-weight:700;",
                        tone_fg
                    ),
                    onclick: move |evt| on_clear.call(evt),
                    "×"
                }
            }
            div {
                style: format!(
                    "font-size:11px; line-height:1.45; color:{}; text-rendering:optimizeLegibility; \
                     -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                    palette.text
                ),
                "{item.message}"
            }
        }
    }
}

pub fn toast_tone_colors(
    tone: ToastTone,
    palette: ToastPalette,
) -> (&'static str, &'static str) {
    match tone {
        ToastTone::Info => (palette.accent, "#315066"),
        ToastTone::Success => ("#2f9e62", "#315066"),
        ToastTone::Warning => ("#d79b24", "#315066"),
        ToastTone::Error => ("#d95c5c", "#315066"),
    }
}
