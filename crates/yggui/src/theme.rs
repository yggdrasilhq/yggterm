use yggui_contract::{UiTheme, YgguiThemeColorStop, YgguiThemeSpec};

pub const MAX_THEME_STOPS: usize = 6;
pub const THEME_EDITOR_SWATCHES: [&str; 8] = [
    "#7cc8ff", "#b78dff", "#f5c0d9", "#f38f7e", "#f0c454", "#67d7a3", "#8fa7d4", "#f8f8f8",
];

pub fn clamp_theme_spec(spec: &YgguiThemeSpec) -> YgguiThemeSpec {
    let mut next = spec.clone();
    next.brightness = next.brightness.clamp(0.0, 1.0);
    next.grain = next.grain.clamp(0.0, 1.0);
    next.colors = next
        .colors
        .iter()
        .take(MAX_THEME_STOPS)
        .cloned()
        .map(|mut stop| {
            stop.x = stop.x.clamp(0.0, 1.0);
            stop.y = stop.y.clamp(0.0, 1.0);
            stop.alpha = stop.alpha.clamp(0.12, 1.0);
            if !looks_like_hex_color(&stop.color) {
                stop.color = "#7cc8ff".to_string();
            }
            stop
        })
        .collect();
    next
}

pub fn default_theme_editor_spec() -> YgguiThemeSpec {
    YgguiThemeSpec {
        colors: vec![
            YgguiThemeColorStop {
                color: "#7cc8ff".to_string(),
                x: 0.18,
                y: 0.24,
                alpha: 0.86,
            },
            YgguiThemeColorStop {
                color: "#67d7a3".to_string(),
                x: 0.64,
                y: 0.34,
                alpha: 0.78,
            },
            YgguiThemeColorStop {
                color: "#d7e3ee".to_string(),
                x: 0.82,
                y: 0.76,
                alpha: 0.62,
            },
        ],
        brightness: 0.56,
        grain: 0.12,
    }
}

pub fn append_theme_stop(spec: &YgguiThemeSpec, color: Option<&str>) -> YgguiThemeSpec {
    let mut next = clamp_theme_spec(spec);
    if next.colors.len() >= MAX_THEME_STOPS {
        return next;
    }
    let swatch = color.map(str::to_string).unwrap_or_else(|| {
        THEME_EDITOR_SWATCHES[next.colors.len() % THEME_EDITOR_SWATCHES.len()].to_string()
    });
    let spread = next.colors.len() as f32;
    next.colors.push(YgguiThemeColorStop {
        color: swatch,
        x: (0.2 + spread * 0.16).clamp(0.12, 0.88),
        y: (0.24 + spread * 0.14).clamp(0.12, 0.88),
        alpha: 0.82,
    });
    clamp_theme_spec(&next)
}

pub fn gradient_css(theme: UiTheme, spec: &YgguiThemeSpec) -> String {
    let spec = clamp_theme_spec(spec);
    if spec.colors.is_empty() {
        return default_gradient(theme).to_string();
    }
    let mut layers = spec
        .colors
        .iter()
        .map(|stop| {
            let rgba = color_with_alpha(&stop.color, stop.alpha * (0.42 + spec.brightness * 0.58));
            format!(
                "radial-gradient(circle at {:.0}% {:.0}%, {} 0%, transparent 42%)",
                stop.x * 100.0,
                stop.y * 100.0,
                rgba
            )
        })
        .collect::<Vec<_>>();
    layers.push(default_backdrop(theme, spec.brightness));
    let grain = grain_layer(theme, spec.grain);
    if !grain.is_empty() {
        layers.push(grain);
    }
    layers.join(", ")
}

pub fn shell_tint(theme: UiTheme, spec: &YgguiThemeSpec) -> String {
    let spec = clamp_theme_spec(spec);
    let alpha = 0.88 + spec.brightness * 0.08;
    match theme {
        UiTheme::ZedLight => format!("rgba(244,248,250,{alpha:.3})"),
        UiTheme::ZedDark => format!("rgba(39,46,52,{alpha:.3})"),
    }
}

pub fn preview_surface_css(theme: UiTheme, spec: &YgguiThemeSpec) -> String {
    format!(
        "background:{}; border-radius:18px; box-shadow: inset 0 0 0 1px rgba(255,255,255,0.52);",
        gradient_css(theme, spec)
    )
}

pub fn dominant_accent(spec: &YgguiThemeSpec, fallback: &'static str) -> String {
    clamp_theme_spec(spec)
        .colors
        .first()
        .map(|stop| stop.color.clone())
        .unwrap_or_else(|| fallback.to_string())
}

fn default_gradient(theme: UiTheme) -> &'static str {
    match theme {
        UiTheme::ZedLight => {
            "linear-gradient(180deg, rgba(232,243,248,0.94) 0%, rgba(232,244,238,0.90) 48%, rgba(237,240,244,0.94) 100%)"
        }
        UiTheme::ZedDark => {
            "linear-gradient(180deg, rgba(68,95,106,0.92) 0%, rgba(75,102,94,0.88) 52%, rgba(58,65,73,0.94) 100%)"
        }
    }
}

fn default_backdrop(theme: UiTheme, brightness: f32) -> String {
    match theme {
        UiTheme::ZedLight => format!(
            "linear-gradient(180deg, rgba(242,247,249,{:.3}) 0%, rgba(238,244,241,{:.3}) 55%, rgba(235,239,244,{:.3}) 100%)",
            0.92 + brightness * 0.05,
            0.90 + brightness * 0.05,
            0.92 + brightness * 0.04
        ),
        UiTheme::ZedDark => format!(
            "linear-gradient(180deg, rgba(59,79,88,{:.3}) 0%, rgba(69,89,82,{:.3}) 50%, rgba(49,55,63,{:.3}) 100%)",
            0.86 + brightness * 0.08,
            0.84 + brightness * 0.08,
            0.90 + brightness * 0.06
        ),
    }
}

fn grain_layer(theme: UiTheme, grain: f32) -> String {
    if grain <= 0.01 {
        return String::new();
    }
    let alpha = 0.015 + grain * 0.05;
    match theme {
        UiTheme::ZedLight => {
            format!("radial-gradient(circle, rgba(70,88,104,{alpha:.3}) 0.7px, transparent 0.8px)")
        }
        UiTheme::ZedDark => format!(
            "radial-gradient(circle, rgba(255,255,255,{alpha:.3}) 0.7px, transparent 0.8px)"
        ),
    }
}

fn color_with_alpha(hex: &str, alpha: f32) -> String {
    let (r, g, b) = hex_to_rgb(hex).unwrap_or((124, 200, 255));
    format!("rgba({r}, {g}, {b}, {:.3})", alpha.clamp(0.0, 1.0))
}

fn looks_like_hex_color(value: &str) -> bool {
    let bytes = value.as_bytes();
    matches!(bytes.len(), 7 | 9)
        && bytes.first() == Some(&b'#')
        && bytes[1..].iter().all(|byte| byte.is_ascii_hexdigit())
}

fn hex_to_rgb(value: &str) -> Option<(u8, u8, u8)> {
    if !looks_like_hex_color(value) {
        return None;
    }
    Some((
        u8::from_str_radix(&value[1..3], 16).ok()?,
        u8::from_str_radix(&value[3..5], 16).ok()?,
        u8::from_str_radix(&value[5..7], 16).ok()?,
    ))
}
