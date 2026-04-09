use dioxus::prelude::*;
use std::env;

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
@keyframes yggterm-toast-progress-indeterminate {
  0% { transform: translateX(-65%); }
  100% { transform: translateX(165%); }
}
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    pub job_key: Option<String>,
    pub progress: Option<f32>,
    pub persistent: bool,
}

#[derive(Clone, Copy, PartialEq)]
pub struct ToastPalette {
    pub text: &'static str,
    pub muted: &'static str,
    pub accent: &'static str,
    pub is_dark: bool,
}

fn parse_css_color(input: &str) -> Option<(f32, f32, f32, f32)> {
    let trimmed = input.trim();
    if trimmed.eq_ignore_ascii_case("transparent") {
        return Some((0.0, 0.0, 0.0, 0.0));
    }
    if let Some(hex) = trimmed.strip_prefix('#') {
        let expanded = match hex.len() {
            3 => hex.chars().flat_map(|ch| [ch, ch]).collect::<String>(),
            6 => hex.to_string(),
            _ => return None,
        };
        let red = u8::from_str_radix(&expanded[0..2], 16).ok()? as f32 / 255.0;
        let green = u8::from_str_radix(&expanded[2..4], 16).ok()? as f32 / 255.0;
        let blue = u8::from_str_radix(&expanded[4..6], 16).ok()? as f32 / 255.0;
        return Some((red, green, blue, 1.0));
    }
    let open = trimmed.find('(')?;
    let close = trimmed.rfind(')')?;
    let kind = trimmed[..open].trim();
    let values = trimmed[open + 1..close]
        .split(',')
        .map(|part| part.trim())
        .collect::<Vec<_>>();
    match (kind, values.as_slice()) {
        ("rgb", [red, green, blue]) => Some((
            red.parse::<f32>().ok()? / 255.0,
            green.parse::<f32>().ok()? / 255.0,
            blue.parse::<f32>().ok()? / 255.0,
            1.0,
        )),
        ("rgba", [red, green, blue, alpha]) => Some((
            red.parse::<f32>().ok()? / 255.0,
            green.parse::<f32>().ok()? / 255.0,
            blue.parse::<f32>().ok()? / 255.0,
            alpha.parse::<f32>().ok()?,
        )),
        _ => None,
    }
}

fn blended_luminance(foreground: &str, background: &str) -> Option<f32> {
    let (fr, fg, fb, fa) = parse_css_color(foreground)?;
    let (br, bg, bb, _ba) = parse_css_color(background)?;
    let red = (fr * fa) + (br * (1.0 - fa));
    let green = (fg * fa) + (bg * (1.0 - fa));
    let blue = (fb * fa) + (bb * (1.0 - fa));
    Some((0.2126 * red) + (0.7152 * green) + (0.0722 * blue))
}

fn contrast_text_for_layer(foreground: &str, background: &str, emphasized: bool) -> &'static str {
    match blended_luminance(foreground, background) {
        Some(luminance) if luminance < 0.46 => {
            if emphasized {
                "#f6fbff"
            } else {
                "#e7f1fb"
            }
        }
        _ => {
            if emphasized {
                "#18222d"
            } else {
                "#31404d"
            }
        }
    }
}

fn linux_kde_wayland_safe_mode() -> bool {
    #[cfg(target_os = "linux")]
    {
        env::var_os("WAYLAND_DISPLAY").is_some()
            && env::var("XDG_CURRENT_DESKTOP")
                .map(|value| value.to_ascii_lowercase().contains("kde"))
                .unwrap_or(false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

fn linux_x11_safe_mode() -> bool {
    #[cfg(target_os = "linux")]
    {
        env::var_os("DISPLAY").is_some() && env::var_os("WAYLAND_DISPLAY").is_none()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[component]
pub fn ToastViewport(
    items: Vec<ToastItem>,
    palette: ToastPalette,
    center_offset: i32,
    max_age_ms: u64,
    max_visible: usize,
    now_ms: u64,
    on_clear: EventHandler<u64>,
) -> Element {
    let visible = items
        .into_iter()
        .rev()
        .filter(|notification| {
            notification.persistent
                || now_ms.saturating_sub(notification.created_at_ms) <= max_age_ms
        })
        .take(max_visible)
        .collect::<Vec<_>>();
    if visible.is_empty() {
        return rsx! {};
    }
    let stack_key = visible
        .iter()
        .map(|notification| notification.id.to_string())
        .collect::<Vec<_>>()
        .join("-");
    rsx! {
        div {
            key: "{stack_key}",
            style: format!(
                "position:fixed; top:22px; left:50%; transform:translateX(calc(-50% + {}px)); z-index:80; display:flex; flex-direction:column; gap:10px; width:320px; max-width:min(320px, calc(100vw - 32px)); pointer-events:none;",
                center_offset
            ),
            for notification in visible {
                div {
                    key: "{notification.id}",
                    style: "pointer-events:auto; animation:yggterm-toast-stack-in 220ms ease both;",
                    div {
                        style: if notification.persistent {
                            "animation:none;"
                        } else {
                            "animation:yggterm-toast-fade 7s ease forwards;"
                        },
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
    let background = if palette.is_dark {
        "rgba(8,12,16,0.97)"
    } else {
        "rgba(252,253,255,0.96)"
    };
    let shell_background = if palette.is_dark {
        "rgba(24,31,38,0.96)"
    } else {
        "rgba(243,247,250,0.94)"
    };
    let title_fg = contrast_text_for_layer(background, shell_background, true);
    let body_fg = contrast_text_for_layer(background, shell_background, false);
    let (tone_accent, close_fg) =
        toast_tone_colors(item.tone, palette, background, shell_background);
    let blur_style = if linux_kde_wayland_safe_mode() || linux_x11_safe_mode() {
        "backdrop-filter:none; -webkit-backdrop-filter:none;"
    } else {
        "backdrop-filter: blur(28px) saturate(165%); -webkit-backdrop-filter: blur(28px) saturate(165%);"
    };
    rsx! {
        div {
            style: format!(
                "display:flex; flex-direction:column; gap:7px; padding:12px 12px 11px 12px; border-radius:14px; \
                 background:{}; {} box-shadow:{};",
                background,
                blur_style
                ,
                if palette.is_dark {
                    "0 22px 44px rgba(0,0,0,0.32), inset 0 0 0 1px rgba(214,229,242,0.18)"
                } else {
                    "0 18px 38px rgba(49,67,82,0.12), inset 0 0 0 1px rgba(255,255,255,0.88)"
                }
            ),
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
                            title_fg
                        ),
                        "{item.title}"
                    }
                }
                button {
                    style: format!(
                        "width:22px; height:22px; border:none; border-radius:8px; background:{}; color:{}; font-size:12px; font-weight:700;",
                        if palette.is_dark {
                            "rgba(255,255,255,0.12)"
                        } else {
                            "rgba(241,244,247,0.96)"
                        },
                        close_fg
                    ),
                    onclick: move |evt| on_clear.call(evt),
                    "×"
                }
            }
            div {
                style: format!(
                    "font-size:11px; line-height:1.45; color:{}; text-rendering:optimizeLegibility; \
                     -webkit-font-smoothing:antialiased; -moz-osx-font-smoothing:grayscale;",
                    body_fg
                ),
                "{item.message}"
            }
            if item.persistent || item.progress.is_some() {
                ToastProgressBar {
                    progress: item.progress,
                    tone: item.tone,
                }
            }
        }
    }
}

#[component]
fn ToastProgressBar(progress: Option<f32>, tone: ToastTone) -> Element {
    let clamped = progress.map(|value| value.clamp(0.0, 1.0));
    let accent = match tone {
        ToastTone::Info => "#72bef7",
        ToastTone::Success => "#2f9e62",
        ToastTone::Warning => "#d79b24",
        ToastTone::Error => "#d95c5c",
    };
    rsx! {
        div {
            style: "position:relative; width:100%; height:8px; border-radius:999px; overflow:hidden; \
                    background:rgba(191,206,221,0.3); box-shadow:inset 0 0 0 1px rgba(255,255,255,0.24);",
            if let Some(progress) = clamped {
                div {
                    style: format!(
                        "height:100%; width:{:.2}%; border-radius:999px; background:{}; transition:width 180ms ease;",
                        progress * 100.0,
                        accent
                    )
                }
            } else {
                div {
                    style: format!(
                        "position:absolute; inset:0 auto 0 0; width:44%; border-radius:999px; background:{}; \
                         animation:yggterm-toast-progress-indeterminate 1.1s ease-in-out infinite;",
                        accent
                    )
                }
            }
        }
    }
}

pub fn toast_tone_colors(
    tone: ToastTone,
    palette: ToastPalette,
    background: &str,
    shell_background: &str,
) -> (&'static str, &'static str) {
    let foreground = contrast_text_for_layer(background, shell_background, true);
    match tone {
        ToastTone::Info => (palette.accent, foreground),
        ToastTone::Success => ("#2f9e62", foreground),
        ToastTone::Warning => ("#d79b24", foreground),
        ToastTone::Error => ("#d95c5c", foreground),
    }
}
