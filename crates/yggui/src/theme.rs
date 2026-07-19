use yggui_contract::{UiTheme, YgguiThemeColorStop, YgguiThemeSpec};

pub const MAX_THEME_STOPS: usize = 6;
const MAX_RENDER_THEME_STOPS: usize = 4;
const MIN_STOP_INSET: f32 = 0.08;
const MAX_STOP_INSET: f32 = 0.92;
const MIN_STOP_ALPHA: f32 = 0.28;
const MAX_STOP_ALPHA: f32 = 0.86;
const MIN_THEME_BRIGHTNESS: f32 = 0.38;
const MAX_THEME_BRIGHTNESS: f32 = 0.72;
const STABLE_THEME_ALPHA: f32 = 0.96;
const STABLE_THEME_GRAIN: f32 = 0.0;
const FALLBACK_COLOR: &str = "#7cc8ff";
pub const THEME_EDITOR_SWATCHES: [&str; 8] = [
    "#7cc8ff", "#b8a1ff", "#efc6dc", "#e3a08f", "#e8c16d", "#7acfb0", "#9caed8", "#dfe8ef",
];

pub fn clamp_theme_spec(spec: &YgguiThemeSpec) -> YgguiThemeSpec {
    let mut next = spec.clone();
    next.brightness = next
        .brightness
        .clamp(MIN_THEME_BRIGHTNESS, MAX_THEME_BRIGHTNESS);
    next.alpha = STABLE_THEME_ALPHA;
    next.grain = STABLE_THEME_GRAIN;
    next.colors = next
        .colors
        .iter()
        .take(MAX_THEME_STOPS)
        .cloned()
        .map(|mut stop| {
            stop.x = stop.x.clamp(MIN_STOP_INSET, MAX_STOP_INSET);
            stop.y = stop.y.clamp(MIN_STOP_INSET, MAX_STOP_INSET);
            stop.alpha = stop.alpha.clamp(MIN_STOP_ALPHA, MAX_STOP_ALPHA);
            stop.color =
                normalize_hex_color(&stop.color).unwrap_or_else(|| FALLBACK_COLOR.to_string());
            stop
        })
        .collect();
    rebalance_stop_positions(&mut next.colors);
    next
}

pub fn default_theme_editor_spec() -> YgguiThemeSpec {
    YgguiThemeSpec {
        colors: vec![
            YgguiThemeColorStop {
                color: normalize_hex_color("#7cc8ff").unwrap_or_else(|| FALLBACK_COLOR.to_string()),
                x: 0.18,
                y: 0.24,
                alpha: 0.74,
            },
            YgguiThemeColorStop {
                color: normalize_hex_color("#67d7a3").unwrap_or_else(|| "#7acfb0".to_string()),
                x: 0.64,
                y: 0.34,
                alpha: 0.66,
            },
            YgguiThemeColorStop {
                color: normalize_hex_color("#d7e3ee").unwrap_or_else(|| "#dfe8ef".to_string()),
                x: 0.82,
                y: 0.76,
                alpha: 0.48,
            },
        ],
        brightness: 0.56,
        alpha: STABLE_THEME_ALPHA,
        grain: STABLE_THEME_GRAIN,
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
        alpha: 0.68,
    });
    clamp_theme_spec(&next)
}

pub fn gradient_css(theme: UiTheme, spec: &YgguiThemeSpec) -> String {
    gradient_css_with_alpha_scale(theme, spec, 1.0)
}

pub fn live_blur_gradient_css(theme: UiTheme, spec: &YgguiThemeSpec) -> String {
    // Blur is a material effect paired with the alpha control, not a second
    // opacity knob. Keep the gradient alpha faithful to the theme spec and let
    // the shell/compositor blur path provide readability.
    gradient_css_with_alpha_scale(theme, spec, 1.0)
}

pub fn material_blur_radius_px(spec: &YgguiThemeSpec) -> f32 {
    let _ = spec;
    0.0
}

pub fn gradient_background_size_css(spec: &YgguiThemeSpec) -> String {
    repeated_gradient_background_property(spec, "100% 100%", "4px 4px")
}

pub fn gradient_background_repeat_css(spec: &YgguiThemeSpec) -> String {
    repeated_gradient_background_property(spec, "no-repeat", "repeat")
}

fn repeated_gradient_background_property(
    spec: &YgguiThemeSpec,
    layer_value: &'static str,
    _grain_value: &'static str,
) -> String {
    let _ = clamp_theme_spec(spec);
    layer_value.to_string()
}

fn gradient_css_with_alpha_scale(
    theme: UiTheme,
    spec: &YgguiThemeSpec,
    _alpha_scale: f32,
) -> String {
    let spec = clamp_theme_spec(spec);
    if spec.colors.is_empty() {
        return default_gradient(theme);
    }
    let mut stops = spec
        .colors
        .iter()
        .take(MAX_RENDER_THEME_STOPS)
        .enumerate()
        .map(|(index, stop)| {
            let rgb = rendered_stop_rgb(theme, stop, spec.brightness, index);
            let position = ((index as f32 / MAX_RENDER_THEME_STOPS as f32) * 84.0).round();
            format!("rgb({}, {}, {}) {:.0}%", rgb.0, rgb.1, rgb.2, position)
        })
        .collect::<Vec<_>>();
    let base = themed_shell_rgb(theme, &spec);
    stops.push(format!("rgb({}, {}, {}) 100%", base.0, base.1, base.2));
    format!("linear-gradient(135deg, {})", stops.join(", "))
}

pub fn shell_tint(theme: UiTheme, spec: &YgguiThemeSpec) -> String {
    let spec = clamp_theme_spec(spec);
    let rgb = themed_shell_rgb(theme, &spec);
    format!("rgb({}, {}, {})", rgb.0, rgb.1, rgb.2)
}

pub fn chrome_material_tint(theme: UiTheme, spec: &YgguiThemeSpec) -> String {
    let spec = clamp_theme_spec(spec);
    let rgb = themed_shell_rgb(theme, &spec);
    let brightness_lift = theme_brightness_lift(spec.brightness);
    let polished = match theme {
        UiTheme::ZedLight => mix_rgb(rgb, (255, 255, 255), 0.08 + brightness_lift * 0.05),
        UiTheme::ZedDark => mix_rgb(rgb, (18, 24, 31), 0.08 + brightness_lift * 0.04),
    };
    format!("rgb({}, {}, {})", polished.0, polished.1, polished.2)
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
        .unwrap_or_else(|| {
            normalize_hex_color(fallback).unwrap_or_else(|| FALLBACK_COLOR.to_string())
        })
}

fn default_gradient(theme: UiTheme) -> String {
    match theme {
        UiTheme::ZedLight => {
            "linear-gradient(180deg, rgb(236, 243, 248) 0%, rgb(240, 245, 249) 50%, rgb(232, 238, 244) 100%)".to_string()
        }
        UiTheme::ZedDark => {
            "linear-gradient(180deg, rgb(56, 74, 92) 0%, rgb(48, 60, 76) 50%, rgb(32, 38, 48) 100%)".to_string()
        }
    }
}

fn rendered_stop_rgb(
    theme: UiTheme,
    stop: &YgguiThemeColorStop,
    brightness: f32,
    index: usize,
) -> (u8, u8, u8) {
    let color = normalize_hex_color(&stop.color).unwrap_or_else(|| FALLBACK_COLOR.to_string());
    let rgb = hex_to_rgb(&color).unwrap_or((124, 200, 255));
    let anchor = match theme {
        UiTheme::ZedLight => (234, 241, 246),
        UiTheme::ZedDark => (96, 122, 130),
    };
    let softened = mix_rgb(
        rgb,
        anchor,
        match theme {
            UiTheme::ZedLight => 0.28,
            UiTheme::ZedDark => 0.34,
        },
    );
    let polished = match theme {
        UiTheme::ZedLight => mix_rgb(softened, (255, 255, 255), 0.10 + brightness * 0.08),
        UiTheme::ZedDark => mix_rgb(softened, (230, 240, 248), 0.06 + brightness * 0.05),
    };
    let base = themed_shell_rgb(
        theme,
        &YgguiThemeSpec {
            colors: vec![stop.clone()],
            brightness,
            alpha: STABLE_THEME_ALPHA,
            grain: STABLE_THEME_GRAIN,
        },
    );
    let index_lift = (index as f32 * 0.035).min(0.12);
    match theme {
        UiTheme::ZedLight => mix_rgb(polished, base, 0.18 + index_lift),
        UiTheme::ZedDark => mix_rgb(polished, base, 0.24 + index_lift),
    }
}

fn themed_shell_rgb(theme: UiTheme, spec: &YgguiThemeSpec) -> (u8, u8, u8) {
    let spec = clamp_theme_spec(spec);
    if spec.colors.is_empty() {
        return match theme {
            UiTheme::ZedLight => (242, 246, 249),
            UiTheme::ZedDark => (40, 49, 58),
        };
    }

    let mut weighted = (0.0f32, 0.0f32, 0.0f32);
    let mut total = 0.0f32;
    for stop in spec.colors.iter().take(MAX_RENDER_THEME_STOPS) {
        let rgb = hex_to_rgb(&stop.color).unwrap_or((124, 200, 255));
        let weight = stop.alpha.max(0.1);
        weighted.0 += rgb.0 as f32 * weight;
        weighted.1 += rgb.1 as f32 * weight;
        weighted.2 += rgb.2 as f32 * weight;
        total += weight;
    }
    let averaged = if total <= f32::EPSILON {
        (124, 200, 255)
    } else {
        (
            (weighted.0 / total).round() as u8,
            (weighted.1 / total).round() as u8,
            (weighted.2 / total).round() as u8,
        )
    };
    match theme {
        UiTheme::ZedLight => mix_rgb(
            averaged,
            (245, 248, 250),
            0.68 - theme_brightness_lift(spec.brightness) * 0.10,
        ),
        UiTheme::ZedDark => mix_rgb(
            averaged,
            (30, 37, 45),
            0.72 - theme_brightness_lift(spec.brightness) * 0.12,
        ),
    }
}

fn theme_brightness_lift(brightness: f32) -> f32 {
    ((brightness - MIN_THEME_BRIGHTNESS) / (MAX_THEME_BRIGHTNESS - MIN_THEME_BRIGHTNESS))
        .clamp(0.0, 1.0)
}

fn rebalance_stop_positions(stops: &mut [YgguiThemeColorStop]) {
    for index in 0..stops.len() {
        let (head, tail) = stops.split_at_mut(index + 1);
        let current = &head[index];
        for (offset, other) in tail.iter_mut().enumerate() {
            let dx = other.x - current.x;
            let dy = other.y - current.y;
            let distance = (dx * dx + dy * dy).sqrt();
            if distance >= 0.15 {
                continue;
            }
            let angle = ((index + offset + 1) as f32) * 0.78;
            let nudge = 0.17 - distance;
            other.x = (other.x + angle.cos() * nudge).clamp(MIN_STOP_INSET, MAX_STOP_INSET);
            other.y = (other.y + angle.sin() * nudge).clamp(MIN_STOP_INSET, MAX_STOP_INSET);
        }
    }
}

fn normalize_hex_color(value: &str) -> Option<String> {
    let rgb = hex_to_rgb(value)?;
    let (h, s, l) = rgb_to_hsl(rgb);
    let (safe_s, safe_l) = if s < 0.12 {
        (s.clamp(0.04, 0.18), l.clamp(0.78, 0.92))
    } else {
        (s.clamp(0.26, 0.72), l.clamp(0.60, 0.82))
    };
    let (r, g, b) = hsl_to_rgb(h, safe_s, safe_l);
    Some(rgb_to_hex((r, g, b)))
}

fn rgb_to_hsl((r, g, b): (u8, u8, u8)) -> (f32, f32, f32) {
    let r = r as f32 / 255.0;
    let g = g as f32 / 255.0;
    let b = b as f32 / 255.0;
    let max = r.max(g.max(b));
    let min = r.min(g.min(b));
    let lightness = (max + min) / 2.0;
    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, lightness);
    }
    let delta = max - min;
    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
    let hue = if (max - r).abs() < f32::EPSILON {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if (max - g).abs() < f32::EPSILON {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };
    (hue, saturation, lightness)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    if s <= f32::EPSILON {
        let grey = (l * 255.0).round() as u8;
        return (grey, grey, grey);
    }
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - (((h / 60.0).rem_euclid(2.0)) - 1.0).abs());
    let m = l - c / 2.0;
    let (r1, g1, b1) = match h {
        h if (0.0..60.0).contains(&h) => (c, x, 0.0),
        h if (60.0..120.0).contains(&h) => (x, c, 0.0),
        h if (120.0..180.0).contains(&h) => (0.0, c, x),
        h if (180.0..240.0).contains(&h) => (0.0, x, c),
        h if (240.0..300.0).contains(&h) => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    )
}

fn rgb_to_hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn mix_rgb(left: (u8, u8, u8), right: (u8, u8, u8), right_weight: f32) -> (u8, u8, u8) {
    let left_weight = 1.0 - right_weight.clamp(0.0, 1.0);
    (
        (left.0 as f32 * left_weight + right.0 as f32 * right_weight).round() as u8,
        (left.1 as f32 * left_weight + right.1 as f32 * right_weight).round() as u8,
        (left.2 as f32 * left_weight + right.2 as f32 * right_weight).round() as u8,
    )
}

fn looks_like_hex_color(value: &str) -> bool {
    let bytes = value.as_bytes();
    matches!(bytes.len(), 7 | 9)
        && bytes.first() == Some(&b'#')
        && bytes[1..].iter().all(|byte| byte.is_ascii_hexdigit())
}

pub fn hex_to_rgb(value: &str) -> Option<(u8, u8, u8)> {
    if !looks_like_hex_color(value) {
        return None;
    }
    Some((
        u8::from_str_radix(&value[1..3], 16).ok()?,
        u8::from_str_radix(&value[3..5], 16).ok()?,
        u8::from_str_radix(&value[5..7], 16).ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_theme_spec_pastelizes_extreme_colors() {
        let spec = YgguiThemeSpec {
            colors: vec![YgguiThemeColorStop {
                color: "#ff0000".to_string(),
                x: 0.01,
                y: 1.2,
                alpha: 1.0,
            }],
            brightness: 1.0,
            alpha: 1.0,
            grain: 0.8,
        };
        let clamped = clamp_theme_spec(&spec);
        assert_eq!(clamped.brightness, MAX_THEME_BRIGHTNESS);
        assert_eq!(clamped.alpha, STABLE_THEME_ALPHA);
        assert_eq!(clamped.grain, STABLE_THEME_GRAIN);
        assert_eq!(clamped.colors[0].color, "#e25050");
        assert!((MIN_STOP_INSET..=MAX_STOP_INSET).contains(&clamped.colors[0].x));
        assert!((MIN_STOP_INSET..=MAX_STOP_INSET).contains(&clamped.colors[0].y));
    }

    #[test]
    fn clamp_theme_spec_pins_grain_for_stable_theme() {
        let mut spec = default_theme_editor_spec();
        spec.grain = 0.9;
        assert_eq!(clamp_theme_spec(&spec).grain, STABLE_THEME_GRAIN);

        spec.grain = 1.8;
        assert_eq!(clamp_theme_spec(&spec).grain, STABLE_THEME_GRAIN);
    }

    #[test]
    fn gradient_css_uses_limited_render_stops() {
        let mut spec = default_theme_editor_spec();
        for _ in 0..4 {
            spec = append_theme_stop(&spec, Some("#7cc8ff"));
        }
        let gradient = gradient_css(UiTheme::ZedDark, &spec);
        assert_eq!(gradient.matches("rgb(").count(), MAX_RENDER_THEME_STOPS + 1);
    }

    #[test]
    fn dominant_accent_uses_normalized_color() {
        let spec = YgguiThemeSpec {
            colors: vec![YgguiThemeColorStop {
                color: "#00ff00".to_string(),
                ..YgguiThemeColorStop::default()
            }],
            ..YgguiThemeSpec::default()
        };
        assert_eq!(dominant_accent(&spec, "#ff00ff"), "#50e250");
    }

    #[test]
    fn shell_tint_tracks_selected_color_family() {
        let spec = YgguiThemeSpec {
            colors: vec![
                YgguiThemeColorStop {
                    color: "#b8a1ff".to_string(),
                    x: 0.18,
                    y: 0.24,
                    alpha: 0.82,
                },
                YgguiThemeColorStop {
                    color: "#c9b8ff".to_string(),
                    x: 0.64,
                    y: 0.32,
                    alpha: 0.72,
                },
            ],
            brightness: 0.56,
            alpha: 0.78,
            grain: 0.12,
        };
        let rgb = themed_shell_rgb(UiTheme::ZedDark, &spec);
        assert!(rgb.2 >= rgb.1);
        assert!(rgb.0 >= 40);
    }

    #[test]
    fn gradient_css_no_longer_uses_old_green_backdrop_signature() {
        let spec = YgguiThemeSpec {
            colors: vec![
                YgguiThemeColorStop {
                    color: "#b8a1ff".to_string(),
                    x: 0.18,
                    y: 0.24,
                    alpha: 0.82,
                },
                YgguiThemeColorStop {
                    color: "#d9d1ff".to_string(),
                    x: 0.74,
                    y: 0.68,
                    alpha: 0.58,
                },
            ],
            brightness: 0.56,
            alpha: 0.78,
            grain: 0.12,
        };
        let gradient = gradient_css(UiTheme::ZedDark, &spec);
        assert!(!gradient.contains("69,89,82"));
        assert!(!gradient.contains("75,102,94"));
    }

    #[test]
    fn stable_gradient_ignores_saved_grain_when_theme_has_no_color_stops() {
        let spec = YgguiThemeSpec {
            colors: Vec::new(),
            brightness: 0.56,
            alpha: 0.78,
            grain: 0.24,
        };
        let gradient = gradient_css(UiTheme::ZedLight, &spec);
        assert!(gradient.contains("linear-gradient("));
        assert!(!gradient.contains("radial-gradient(circle at 1px 1px"));
    }

    #[test]
    fn gradient_background_properties_do_not_emit_grain_layer_on_stable_theme() {
        let mut spec = default_theme_editor_spec();
        spec.grain = 0.72;
        let size = gradient_background_size_css(&spec);
        let repeat = gradient_background_repeat_css(&spec);

        assert_eq!(size, "100% 100%");
        assert_eq!(repeat, "no-repeat");
        assert!(!size.ends_with("4px 4px"));

        spec.grain = 0.0;
        assert!(!gradient_background_size_css(&spec).contains("4px 4px"));
        assert_eq!(gradient_background_repeat_css(&spec), "no-repeat");
    }

    #[test]
    fn alpha_is_fixed_for_stable_theme() {
        let mut low = default_theme_editor_spec();
        low.alpha = 0.42;
        let mut high = low.clone();
        high.alpha = 0.90;

        let low_gradient = gradient_css(UiTheme::ZedLight, &low);
        let high_gradient = gradient_css(UiTheme::ZedLight, &high);
        let low_tint = shell_tint(UiTheme::ZedLight, &low);
        let high_tint = shell_tint(UiTheme::ZedLight, &high);
        let low_material = chrome_material_tint(UiTheme::ZedLight, &low);
        let high_material = chrome_material_tint(UiTheme::ZedLight, &high);

        assert_eq!(low_gradient, high_gradient);
        assert_eq!(low_tint, high_tint);
        assert_eq!(low_material, high_material);
        assert!(low_tint.starts_with("rgb("));
        assert!(!low_tint.contains("rgba("));
    }

    #[test]
    fn stable_theme_css_exports_opaque_single_layer_material() {
        let mut spec = default_theme_editor_spec();
        spec.alpha = 0.18;
        spec.grain = 0.84;

        let gradient = gradient_css(UiTheme::ZedLight, &spec);
        let tint = shell_tint(UiTheme::ZedLight, &spec);
        let material = chrome_material_tint(UiTheme::ZedLight, &spec);

        for css in [&gradient, &tint, &material] {
            assert!(
                !css.contains("rgba("),
                "stable theme CSS must not expose alpha channels: {css}"
            );
            assert!(
                !css.contains("transparent"),
                "stable theme CSS must not expose transparent layers: {css}"
            );
        }
        assert!(
            !gradient.contains("radial-gradient("),
            "stable shell material must stay a single opaque layer: {gradient}"
        );
        assert_eq!(gradient_background_size_css(&spec), "100% 100%");
        assert_eq!(gradient_background_repeat_css(&spec), "no-repeat");
    }

    #[test]
    fn live_blur_gradient_preserves_theme_alpha() {
        let spec = default_theme_editor_spec();
        let normal = gradient_css(UiTheme::ZedLight, &spec);
        let blurred = live_blur_gradient_css(UiTheme::ZedLight, &spec);

        assert_eq!(normal, blurred);
        assert_eq!(
            normal.matches("radial-gradient(circle at").count(),
            blurred.matches("radial-gradient(circle at").count()
        );
        assert!(!blurred.contains("rgba("));
        assert!(!blurred.contains("transparent"));
    }

    #[test]
    fn material_blur_radius_is_disabled_on_stable_theme() {
        let mut translucent = default_theme_editor_spec();
        translucent.alpha = 0.50;
        let mut opaque = translucent.clone();
        opaque.alpha = 1.0;

        assert_eq!(material_blur_radius_px(&translucent), 0.0);
        assert_eq!(material_blur_radius_px(&opaque), 0.0);
    }

    #[test]
    fn chrome_material_tint_stays_opaque_on_transparent_window_backends() {
        let mut spec = default_theme_editor_spec();
        spec.alpha = 0.10;
        let light = chrome_material_tint(UiTheme::ZedLight, &spec);
        let dark = chrome_material_tint(UiTheme::ZedDark, &spec);

        assert!(light.starts_with("rgb("));
        assert!(dark.starts_with("rgb("));
        assert!(!light.contains("rgba("));
        assert!(!dark.contains("rgba("));
    }
}
