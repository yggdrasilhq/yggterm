use yggui_contract::{UiTheme, YgguiThemeColorStop, YgguiThemeSpec};

pub const MAX_THEME_STOPS: usize = 6;
const MAX_RENDER_THEME_STOPS: usize = 4;
const MIN_STOP_INSET: f32 = 0.08;
const MAX_STOP_INSET: f32 = 0.92;
const MIN_STOP_ALPHA: f32 = 0.28;
const MAX_STOP_ALPHA: f32 = 0.86;
const MIN_THEME_BRIGHTNESS: f32 = 0.38;
const MAX_THEME_BRIGHTNESS: f32 = 0.72;
<<<<<<< HEAD
const STABLE_THEME_ALPHA: f32 = 0.96;
const STABLE_THEME_GRAIN: f32 = 0.0;
=======
const MIN_THEME_ALPHA: f32 = 0.36;
const MAX_THEME_ALPHA: f32 = 0.92;
const MAX_THEME_GRAIN: f32 = 1.0;
const MIN_MATERIAL_BLUR_PX: f32 = 14.0;
const MAX_MATERIAL_BLUR_PX: f32 = 30.0;
>>>>>>> c162185 (Snapshot alpha blur experiment)
const FALLBACK_COLOR: &str = "#7cc8ff";
pub const THEME_EDITOR_SWATCHES: [&str; 8] = [
    "#7cc8ff", "#b8a1ff", "#efc6dc", "#e3a08f", "#e8c16d", "#7acfb0", "#9caed8", "#dfe8ef",
];

pub fn clamp_theme_spec(spec: &YgguiThemeSpec) -> YgguiThemeSpec {
    let mut next = spec.clone();
    next.brightness = next
        .brightness
        .clamp(MIN_THEME_BRIGHTNESS, MAX_THEME_BRIGHTNESS);
<<<<<<< HEAD
    next.alpha = STABLE_THEME_ALPHA;
    next.grain = STABLE_THEME_GRAIN;
=======
    next.alpha = next.alpha.clamp(MIN_THEME_ALPHA, MAX_THEME_ALPHA);
    next.grain = next.grain.clamp(0.0, MAX_THEME_GRAIN);
>>>>>>> c162185 (Snapshot alpha blur experiment)
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
<<<<<<< HEAD
        alpha: STABLE_THEME_ALPHA,
        grain: STABLE_THEME_GRAIN,
=======
        alpha: 0.78,
        grain: 0.12,
>>>>>>> c162185 (Snapshot alpha blur experiment)
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
<<<<<<< HEAD
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
    grain_value: &'static str,
) -> String {
    let spec = clamp_theme_spec(spec);
=======
    let spec = clamp_theme_spec(spec);
    let alpha_range = (MAX_THEME_ALPHA - MIN_THEME_ALPHA).max(f32::EPSILON);
    let translucency = ((MAX_THEME_ALPHA - spec.alpha) / alpha_range).clamp(0.0, 1.0);
    MIN_MATERIAL_BLUR_PX + translucency * (MAX_MATERIAL_BLUR_PX - MIN_MATERIAL_BLUR_PX)
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
    grain_value: &'static str,
) -> String {
    let spec = clamp_theme_spec(spec);
>>>>>>> c162185 (Snapshot alpha blur experiment)
    let base_layers = if spec.colors.is_empty() {
        1
    } else {
        spec.colors.len().min(MAX_RENDER_THEME_STOPS) + 1
    };
    let mut values = vec![layer_value; base_layers];
    if spec.grain > 0.01 {
        values.push(grain_value);
    }
    values.join(", ")
}

fn gradient_css_with_alpha_scale(
    theme: UiTheme,
    spec: &YgguiThemeSpec,
    alpha_scale: f32,
) -> String {
    let spec = clamp_theme_spec(spec);
    let alpha_scale = alpha_scale.clamp(0.0, 1.0);
    if spec.colors.is_empty() {
        let mut layers = vec![default_gradient(theme, alpha_scale)];
        let grain = grain_layer(theme, spec.grain, alpha_scale);
        if !grain.is_empty() {
            layers.push(grain);
        }
        return layers.join(", ");
    }
    let mut layers = spec
        .colors
        .iter()
        .take(MAX_RENDER_THEME_STOPS)
        .enumerate()
        .map(|(index, stop)| {
            let rgba =
                rendered_stop_rgba(theme, stop, spec.brightness, spec.alpha, index, alpha_scale);
            format!(
                "radial-gradient(circle at {:.0}% {:.0}%, {} 0%, transparent 46%)",
                stop.x * 100.0,
                stop.y * 100.0,
                rgba
            )
        })
        .collect::<Vec<_>>();
    layers.push(default_backdrop(theme, &spec, alpha_scale));
    let grain = grain_layer(theme, spec.grain, alpha_scale);
    if !grain.is_empty() {
        layers.push(grain);
    }
    layers.join(", ")
}

pub fn shell_tint(theme: UiTheme, spec: &YgguiThemeSpec) -> String {
    let spec = clamp_theme_spec(spec);
    let rgb = themed_shell_rgb(theme, &spec);
<<<<<<< HEAD
    let alpha = spec.alpha;
=======
    let alpha = spec.alpha.clamp(MIN_THEME_ALPHA, MAX_THEME_ALPHA);
>>>>>>> c162185 (Snapshot alpha blur experiment)
    format!("rgba({}, {}, {}, {:.3})", rgb.0, rgb.1, rgb.2, alpha)
}

pub fn chrome_material_tint(theme: UiTheme, spec: &YgguiThemeSpec) -> String {
    let spec = clamp_theme_spec(spec);
    let rgb = themed_shell_rgb(theme, &spec);
    let brightness_lift = theme_brightness_lift(spec.brightness);
<<<<<<< HEAD
    let alpha_control = spec.alpha;
=======
    let alpha_control = spec.alpha.clamp(MIN_THEME_ALPHA, MAX_THEME_ALPHA);
>>>>>>> c162185 (Snapshot alpha blur experiment)
    let alpha = match theme {
        UiTheme::ZedLight => 0.86 + brightness_lift * 0.04,
        UiTheme::ZedDark => 0.78 + brightness_lift * 0.06,
    } * (0.86 + alpha_control * 0.14);
    format!(
        "rgba({}, {}, {}, {:.3})",
        rgb.0,
        rgb.1,
        rgb.2,
        alpha.clamp(0.74, 0.92)
    )
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

fn default_gradient(theme: UiTheme, alpha_scale: f32) -> String {
    let alpha_scale = alpha_scale.clamp(0.0, 1.0);
    match theme {
        UiTheme::ZedLight => format!(
            "linear-gradient(180deg, rgba(236,243,248,{:.3}) 0%, rgba(240,245,249,{:.3}) 50%, rgba(232,238,244,{:.3}) 100%)",
            0.86 * alpha_scale,
            0.78 * alpha_scale,
            0.86 * alpha_scale
        ),
        UiTheme::ZedDark => format!(
            "linear-gradient(180deg, rgba(56,74,92,{:.3}) 0%, rgba(48,60,76,{:.3}) 50%, rgba(32,38,48,{:.3}) 100%)",
            0.76 * alpha_scale,
            0.70 * alpha_scale,
            0.82 * alpha_scale
        ),
    }
}

fn default_backdrop(theme: UiTheme, spec: &YgguiThemeSpec, alpha_scale: f32) -> String {
    let base = themed_shell_rgb(theme, spec);
    let brightness_lift = theme_brightness_lift(spec.brightness);
<<<<<<< HEAD
    let global_alpha = spec.alpha;
=======
    let global_alpha = spec.alpha.clamp(MIN_THEME_ALPHA, MAX_THEME_ALPHA);
>>>>>>> c162185 (Snapshot alpha blur experiment)
    let alpha_scale = alpha_scale.clamp(0.0, 1.0);
    let (top, middle, bottom, top_alpha, middle_alpha, bottom_alpha) = match theme {
        UiTheme::ZedLight => (
            mix_rgb(base, (255, 255, 255), 0.34),
            mix_rgb(base, (243, 247, 250), 0.44),
            mix_rgb(base, (232, 238, 244), 0.52),
            0.66 + brightness_lift * 0.08,
            0.60 + brightness_lift * 0.08,
            0.72 + brightness_lift * 0.06,
        ),
        UiTheme::ZedDark => (
            mix_rgb(base, (92, 110, 128), 0.20),
            mix_rgb(base, (48, 56, 68), 0.18),
            mix_rgb(base, (24, 28, 36), 0.26),
            0.48 + brightness_lift * 0.08,
            0.42 + brightness_lift * 0.07,
            0.58 + brightness_lift * 0.06,
        ),
    };
    format!(
        "linear-gradient(180deg, rgba({}, {}, {}, {:.3}) 0%, rgba({}, {}, {}, {:.3}) 54%, rgba({}, {}, {}, {:.3}) 100%)",
        top.0,
        top.1,
        top.2,
        top_alpha * global_alpha * alpha_scale,
        middle.0,
        middle.1,
        middle.2,
        middle_alpha * global_alpha * alpha_scale,
        bottom.0,
        bottom.1,
        bottom.2,
        bottom_alpha * global_alpha * alpha_scale
    )
}

fn grain_layer(theme: UiTheme, grain: f32, alpha_scale: f32) -> String {
    if grain <= 0.01 {
        return String::new();
    }
    let alpha = (0.020 + grain * 0.070) * alpha_scale.clamp(0.35, 1.0);
    match theme {
        UiTheme::ZedLight => {
            format!(
                "radial-gradient(circle at 1px 1px, rgba(70,88,104,{alpha:.3}) 0.55px, transparent 0.75px)"
            )
        }
        UiTheme::ZedDark => format!(
            "radial-gradient(circle at 1px 1px, rgba(255,255,255,{alpha:.3}) 0.55px, transparent 0.75px)"
        ),
    }
}

fn rendered_stop_rgba(
    theme: UiTheme,
    stop: &YgguiThemeColorStop,
    brightness: f32,
    global_alpha: f32,
    index: usize,
    alpha_scale: f32,
) -> String {
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
<<<<<<< HEAD
    let layer_alpha = (stop.alpha * global_alpha * (0.30 + brightness * 0.28) + 0.08
=======
    let layer_alpha = (stop.alpha
        * global_alpha.clamp(MIN_THEME_ALPHA, MAX_THEME_ALPHA)
        * (0.30 + brightness * 0.28)
        + 0.08
>>>>>>> c162185 (Snapshot alpha blur experiment)
        - index as f32 * 0.04)
        .clamp(0.12, 0.54)
        * alpha_scale.clamp(0.0, 1.0);
    format!(
        "rgba({}, {}, {}, {:.3})",
        polished.0, polished.1, polished.2, layer_alpha
    )
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
<<<<<<< HEAD
        assert_eq!(clamped.alpha, STABLE_THEME_ALPHA);
        assert_eq!(clamped.grain, STABLE_THEME_GRAIN);
=======
        assert_eq!(clamped.alpha, MAX_THEME_ALPHA);
        assert_eq!(clamped.grain, 0.8);
>>>>>>> c162185 (Snapshot alpha blur experiment)
        assert_eq!(clamped.colors[0].color, "#e25050");
        assert!((MIN_STOP_INSET..=MAX_STOP_INSET).contains(&clamped.colors[0].x));
        assert!((MIN_STOP_INSET..=MAX_STOP_INSET).contains(&clamped.colors[0].y));
    }

    #[test]
<<<<<<< HEAD
    fn clamp_theme_spec_pins_grain_for_stable_theme() {
        let mut spec = default_theme_editor_spec();
        spec.grain = 0.9;
        assert_eq!(clamp_theme_spec(&spec).grain, STABLE_THEME_GRAIN);

        spec.grain = 1.8;
        assert_eq!(clamp_theme_spec(&spec).grain, STABLE_THEME_GRAIN);
=======
    fn clamp_theme_spec_keeps_grain_as_full_range_dial() {
        let mut spec = default_theme_editor_spec();
        spec.grain = 0.9;
        assert_eq!(clamp_theme_spec(&spec).grain, 0.9);

        spec.grain = 1.8;
        assert_eq!(clamp_theme_spec(&spec).grain, 1.0);
>>>>>>> c162185 (Snapshot alpha blur experiment)
    }

    #[test]
    fn gradient_css_uses_limited_render_stops() {
        let mut spec = default_theme_editor_spec();
        for _ in 0..4 {
            spec = append_theme_stop(&spec, Some("#7cc8ff"));
        }
        let gradient = gradient_css(UiTheme::ZedDark, &spec);
        assert_eq!(
            gradient.matches("radial-gradient(circle at ").count()
                - gradient
                    .matches("radial-gradient(circle at 1px 1px")
                    .count(),
            MAX_RENDER_THEME_STOPS
        );
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
<<<<<<< HEAD
        assert!(!gradient.contains("radial-gradient(circle at 1px 1px"));
    }

    #[test]
    fn gradient_background_properties_do_not_emit_grain_layer_on_stable_theme() {
=======
        assert!(gradient.contains("radial-gradient(circle at 1px 1px"));
    }

    #[test]
    fn gradient_background_properties_repeat_only_the_grain_layer() {
>>>>>>> c162185 (Snapshot alpha blur experiment)
        let mut spec = default_theme_editor_spec();
        spec.grain = 0.72;
        let size = gradient_background_size_css(&spec);
        let repeat = gradient_background_repeat_css(&spec);

<<<<<<< HEAD
        assert_eq!(size.split(',').count(), spec.colors.len() + 1);
        assert_eq!(repeat.split(',').count(), spec.colors.len() + 1);
        assert!(!size.ends_with("4px 4px"));
        assert!(repeat.split(',').all(|layer| layer.trim() == "no-repeat"));
=======
        assert_eq!(size.split(',').count(), spec.colors.len() + 2);
        assert_eq!(repeat.split(',').count(), spec.colors.len() + 2);
        assert!(size.ends_with("4px 4px"));
        assert!(repeat.ends_with("repeat"));
>>>>>>> c162185 (Snapshot alpha blur experiment)
        assert_eq!(size.matches("100% 100%").count(), spec.colors.len() + 1);
        assert_eq!(repeat.matches("no-repeat").count(), spec.colors.len() + 1);

        spec.grain = 0.0;
        assert!(!gradient_background_size_css(&spec).contains("4px 4px"));
        assert!(
            gradient_background_repeat_css(&spec)
                .split(',')
                .all(|part| part.trim() == "no-repeat")
        );
    }

    #[test]
<<<<<<< HEAD
    fn alpha_is_fixed_for_stable_theme() {
=======
    fn global_alpha_changes_shell_gradient_and_tint() {
>>>>>>> c162185 (Snapshot alpha blur experiment)
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

<<<<<<< HEAD
        assert_eq!(low_gradient, high_gradient);
        assert_eq!(low_tint, high_tint);
        assert_eq!(low_material, high_material);
        assert!(low_tint.contains("0.960"));
=======
        assert_ne!(low_gradient, high_gradient);
        assert_ne!(low_tint, high_tint);
        assert_ne!(low_material, high_material);
        assert!(low_tint.contains("0.420"));
        assert!(high_tint.contains("0.900"));
>>>>>>> c162185 (Snapshot alpha blur experiment)
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
        assert!(
            blurred.contains("rgba("),
            "blur material should still export tinted layers"
        );
    }

    #[test]
<<<<<<< HEAD
    fn material_blur_radius_is_disabled_on_stable_theme() {
        let mut translucent = default_theme_editor_spec();
        translucent.alpha = 0.50;
        let mut opaque = translucent.clone();
        opaque.alpha = 1.0;

        assert_eq!(material_blur_radius_px(&translucent), 0.0);
        assert_eq!(material_blur_radius_px(&opaque), 0.0);
=======
    fn material_blur_radius_increases_as_alpha_drops() {
        let mut translucent = default_theme_editor_spec();
        translucent.alpha = 0.50;
        let mut opaque = translucent.clone();
        opaque.alpha = MAX_THEME_ALPHA;

        assert!(material_blur_radius_px(&translucent) > material_blur_radius_px(&opaque));
        assert!((material_blur_radius_px(&translucent) - 26.0).abs() < 0.25);
        assert!((material_blur_radius_px(&opaque) - MIN_MATERIAL_BLUR_PX).abs() < 0.01);
>>>>>>> c162185 (Snapshot alpha blur experiment)
    }

    #[test]
    fn chrome_material_tint_stays_readable_on_transparent_window_backends() {
        let mut spec = default_theme_editor_spec();
<<<<<<< HEAD
        spec.alpha = 0.10;
=======
        spec.alpha = MIN_THEME_ALPHA;
>>>>>>> c162185 (Snapshot alpha blur experiment)
        let light = chrome_material_tint(UiTheme::ZedLight, &spec);
        let dark = chrome_material_tint(UiTheme::ZedDark, &spec);
        let alpha = |value: &str| {
            value
                .trim_end_matches(')')
                .split(',')
                .next_back()
                .and_then(|part| part.trim().parse::<f32>().ok())
                .unwrap()
        };

        assert!(alpha(&light) >= 0.80);
        assert!(alpha(&dark) >= 0.74);
    }
}
