//! Capture-side click grid — the agent-only overlay (control plane slice 3).
//!
//! `server app screenshot --grid` composites a labelled grid into the RETURNED
//! IMAGE and nothing else. The live page is never touched, so a human looking
//! at the screen (or taking their own screenshot at the same moment) sees no
//! grid — that is the acceptance gate this module exists to satisfy.
//!
//! Geometry is not re-derived here: [`yggterm_core::click_grid::GridGeometry`]
//! is the single owner, shared with the live DOM grid in the shell.
//!
//! Two coordinate spaces are in play and the manifest reports both:
//!
//! * **capture** — pixels of the frame as captured, before crop/scale. Same
//!   space as `--crop` and `active_terminal_hosts[].rows_rect` in app state, so
//!   these are the coordinates a click verb consumes.
//! * **image** — pixels of the PNG actually written, after crop and scale. What
//!   the agent's eye sees.

use serde::{Deserialize, Serialize};
use yggterm_core::click_grid::{GridCell, GridGeometry, GridRect};

/// What the caller asked for via `--grid` / `--grid-refine`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridSpec {
    pub cols: u32,
    pub rows: u32,
    /// When set, draw the nine sub-cells of this cell instead of the full grid.
    pub refine: Option<String>,
}

impl Default for GridSpec {
    fn default() -> Self {
        // Same defaults as the live DOM grid (docs/yggui-click-grid.md).
        Self {
            cols: 12,
            rows: 8,
            refine: None,
        }
    }
}

impl GridSpec {
    /// Parse the `--grid` value: absent/empty = defaults, else `COLSxROWS`
    /// (`16x10`). Returns `None` for a malformed pair so the caller can report
    /// the bad flag rather than silently drawing a default grid.
    pub fn parse(value: Option<&str>) -> Option<Self> {
        let raw = match value.map(str::trim) {
            None | Some("") => return Some(Self::default()),
            Some(raw) => raw,
        };
        let (cols, rows) = raw.split_once(['x', 'X'])?;
        let cols: u32 = cols.trim().parse().ok()?;
        let rows: u32 = rows.trim().parse().ok()?;
        if cols == 0 || rows == 0 {
            return None;
        }
        Some(Self {
            cols,
            rows,
            refine: None,
        })
    }

    /// Does this token look like a `--grid` dimension pair? `--grid`'s value is
    /// optional, so the CLI must decide whether the NEXT token belongs to it or
    /// is a separate argument (an output path, a subcommand). A dimension pair
    /// is unambiguous; anything else is not ours. Shared by both binaries so
    /// they cannot drift.
    pub fn looks_like_dimensions(token: &str) -> bool {
        matches!(token.trim().split_once(['x', 'X']), Some((cols, rows))
            if !cols.is_empty()
                && !rows.is_empty()
                && cols.chars().all(|ch| ch.is_ascii_digit())
                && rows.chars().all(|ch| ch.is_ascii_digit()))
    }
}

/// Maps capture-space coordinates onto the written image.
#[derive(Debug, Clone, Copy)]
pub struct CaptureTransform {
    /// Crop origin in capture pixels (0,0 when uncropped).
    pub crop_x: f64,
    pub crop_y: f64,
    /// Nearest-neighbour upscale applied after cropping.
    pub scale: f64,
}

impl CaptureTransform {
    fn to_image(&self, rect: GridRect) -> GridRect {
        GridRect::new(
            (rect.x - self.crop_x) * self.scale,
            (rect.y - self.crop_y) * self.scale,
            rect.w * self.scale,
            rect.h * self.scale,
        )
    }
}

/// One manifest entry: the same cell in both spaces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridCellManifest {
    pub code: String,
    /// Click here. Capture pixels, `{x, y, w, h, cx, cy}`.
    pub capture: ManifestRect,
    /// Look here. Written-image pixels.
    pub image: ManifestRect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub cx: f64,
    pub cy: f64,
}

impl From<GridRect> for ManifestRect {
    fn from(rect: GridRect) -> Self {
        let (cx, cy) = rect.center();
        let round = |value: f64| (value * 100.0).round() / 100.0;
        Self {
            x: round(rect.x),
            y: round(rect.y),
            w: round(rect.w),
            h: round(rect.h),
            cx: round(cx),
            cy: round(cy),
        }
    }
}

/// The full `data.grid` payload of a `--grid` screenshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridManifest {
    pub cols: u32,
    pub rows: u32,
    pub refine: Option<String>,
    /// The gridded region in capture pixels.
    pub region: ManifestRect,
    /// Coordinate space of `cells[].capture` — named so a reader never has to
    /// guess which pixels a click wants.
    pub click_space: &'static str,
    /// Size of the frame as captured, before crop/scale. Compare it against
    /// `window.inner_size` in app state: when they match (they do on a 1x
    /// display) capture pixels ARE CSS pixels and `cells[].capture` coordinates
    /// can be handed straight to a click verb. Reported rather than assumed so a
    /// HiDPI host cannot silently mis-aim.
    pub capture_size: (u32, u32),
    pub cells: Vec<GridCellManifest>,
}

/// Parse `--grid [COLSxROWS]` + `--grid-refine CELL` out of a raw argv tail.
/// Both binaries call this so the GUI and headless CLIs cannot drift — the
/// headless one is what agents actually drive.
///
/// `--grid`'s value is optional, so the following token is claimed only when it
/// is unambiguously a dimension pair; `screenshot out.png --grid` and
/// `screenshot --grid out.png` therefore mean the same thing.
pub fn screenshot_grid_from_args(args: &[String]) -> Option<GridSpec> {
    let mut present = false;
    let mut dimensions: Option<&str> = None;
    let mut refine: Option<String> = None;
    for (index, arg) in args.iter().enumerate() {
        if arg == "--grid" {
            present = true;
            dimensions = args
                .get(index + 1)
                .map(String::as_str)
                .filter(|next| GridSpec::looks_like_dimensions(next));
        } else if let Some(inline) = arg.strip_prefix("--grid=") {
            present = true;
            dimensions = Some(inline);
        } else if arg == "--grid-refine" {
            refine = args
                .get(index + 1)
                .cloned()
                .filter(|next| !next.starts_with("--"));
        } else if let Some(inline) = arg.strip_prefix("--grid-refine=") {
            refine = Some(inline.to_string());
        }
    }
    if !present && refine.is_none() {
        return None;
    }
    let mut spec = GridSpec::parse(dimensions)?;
    spec.refine = refine;
    Some(spec)
}

const OUTLINE: [u8; 3] = [255, 150, 0];
const LABEL_TEXT: [u8; 3] = [255, 255, 255];
const LABEL_PILL: [u8; 3] = [0, 0, 0];

/// Composite the grid into `rgba` (already cropped and scaled) and return the
/// manifest. `region` is the gridded area in CAPTURE pixels.
pub fn composite(
    rgba: &mut [u8],
    width: u32,
    height: u32,
    spec: &GridSpec,
    region: GridRect,
    transform: CaptureTransform,
    capture_size: (u32, u32),
) -> Result<GridManifest, String> {
    let geometry = GridGeometry::new(spec.cols, spec.rows, region);
    let cells: Vec<GridCell> = match spec.refine.as_deref() {
        Some(code) => geometry
            .refine_cells(code)
            .ok_or_else(|| format!("unknown grid cell: {code}"))?,
        None => geometry.cells(),
    };

    let mut canvas = Canvas {
        rgba,
        width,
        height,
    };

    if let Some(code) = spec.refine.as_deref() {
        // Refine mode dims everything outside the chosen cell, exactly like the
        // DOM grid, so the agent's eye is drawn to the nine sub-cells.
        let whole = transform.to_image(region);
        canvas.fill_rect(whole, LABEL_PILL, 0.35);
        let parent = geometry
            .resolve(code)
            .ok_or_else(|| format!("unknown grid cell: {code}"))?;
        let parent_image = transform.to_image(parent);
        canvas.fill_rect(parent_image, [255, 255, 255], 0.06);
        canvas.stroke_rect(parent_image, OUTLINE, 0.9, 2);
    }

    // Labels are wayfinding, not billboards: match the DOM grid's ~13px pills
    // (docs/yggui-click-grid.md) rather than filling the cell, so the grid never
    // hides the content the agent is trying to read.
    let sample = transform.to_image(cells[0].rect);
    let longest = cells.iter().map(|cell| cell.code.len()).max().unwrap_or(2) as f64;
    let glyph_scale = glyph_scale_for(sample.w, sample.h, longest, transform.scale);
    let outline_alpha = if spec.refine.is_some() { 0.65 } else { 0.45 };
    let stroke = if spec.refine.is_some() { 1 } else { 1 };

    let mut manifest_cells = Vec::with_capacity(cells.len());
    for cell in &cells {
        let image_rect = transform.to_image(cell.rect);
        canvas.stroke_rect(image_rect, OUTLINE, outline_alpha, stroke);
        let (cx, cy) = image_rect.center();
        canvas.draw_label(&cell.code, cx, cy, glyph_scale);
        manifest_cells.push(GridCellManifest {
            code: cell.code.clone(),
            capture: cell.rect.into(),
            image: image_rect.into(),
        });
    }

    Ok(GridManifest {
        cols: geometry.cols,
        rows: geometry.rows,
        refine: spec.refine.clone(),
        region: region.into(),
        click_space: "capture",
        capture_size,
        cells: manifest_cells,
    })
}

/// Glyph scale for a label pill: a fixed target size (2 => 14px glyphs, the
/// DOM grid's ~13px pills) multiplied by the output upscale so `--scale 3` keeps
/// labels proportional rather than shrinking them, then shrunk to fit a cell too
/// small to hold it. Floored at 1 — a cramped label beats no label.
///
/// Deterministic in its four inputs; nothing here reads the image.
fn glyph_scale_for(cell_w: f64, cell_h: f64, chars: f64, output_scale: f64) -> u32 {
    const TARGET: u32 = 2;
    let mut scale = (TARGET as f64 * output_scale.max(1.0)).round() as u32;
    scale = scale.clamp(1, 8);
    while scale > 1 {
        let pill_w = chars * (GLYPH_W + 1) as f64 * scale as f64 + 2.0 * PILL_PAD_X as f64;
        let pill_h = GLYPH_H as f64 * scale as f64 + 2.0 * PILL_PAD_Y as f64;
        if pill_w <= cell_w * 0.9 && pill_h <= cell_h * 0.9 {
            break;
        }
        scale -= 1;
    }
    scale
}

struct Canvas<'a> {
    rgba: &'a mut [u8],
    width: u32,
    height: u32,
}

impl Canvas<'_> {
    fn blend(&mut self, x: i64, y: i64, color: [u8; 3], alpha: f64) {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 || alpha <= 0.0 {
            return;
        }
        let alpha = alpha.min(1.0);
        let index = ((y as usize) * self.width as usize + x as usize) * 4;
        if index + 3 >= self.rgba.len() {
            return;
        }
        for channel in 0..3 {
            let dst = self.rgba[index + channel] as f64;
            let src = color[channel] as f64;
            self.rgba[index + channel] =
                (dst + (src - dst) * alpha).round().clamp(0.0, 255.0) as u8;
        }
        // The capture is opaque; keep it that way so viewers do not see holes.
        self.rgba[index + 3] = 255;
    }

    fn fill_rect(&mut self, rect: GridRect, color: [u8; 3], alpha: f64) {
        let x0 = rect.x.round() as i64;
        let y0 = rect.y.round() as i64;
        let x1 = (rect.x + rect.w).round() as i64;
        let y1 = (rect.y + rect.h).round() as i64;
        for y in y0..y1 {
            for x in x0..x1 {
                self.blend(x, y, color, alpha);
            }
        }
    }

    fn stroke_rect(&mut self, rect: GridRect, color: [u8; 3], alpha: f64, thickness: i64) {
        let x0 = rect.x.round() as i64;
        let y0 = rect.y.round() as i64;
        let x1 = (rect.x + rect.w).round() as i64;
        let y1 = (rect.y + rect.h).round() as i64;
        for t in 0..thickness.max(1) {
            for x in x0..x1 {
                self.blend(x, y0 + t, color, alpha);
                self.blend(x, y1 - 1 - t, color, alpha);
            }
            for y in y0..y1 {
                self.blend(x0 + t, y, color, alpha);
                self.blend(x1 - 1 - t, y, color, alpha);
            }
        }
    }

    /// A centered label pill: white 5x7 bitmap glyphs on translucent black,
    /// the same vocabulary as the DOM grid's label pills.
    fn draw_label(&mut self, text: &str, cx: f64, cy: f64, scale: u32) {
        let scale = scale.max(1) as i64;
        let advance = (GLYPH_W + 1) as i64 * scale;
        let text_w = advance * text.chars().count() as i64 - scale;
        let text_h = GLYPH_H as i64 * scale;
        let pad_x = PILL_PAD_X as i64 * scale.min(2);
        let pad_y = PILL_PAD_Y as i64 * scale.min(2);
        let left = (cx.round() as i64) - text_w / 2;
        let top = (cy.round() as i64) - text_h / 2;
        self.fill_rect(
            GridRect::new(
                (left - pad_x) as f64,
                (top - pad_y) as f64,
                (text_w + 2 * pad_x) as f64,
                (text_h + 2 * pad_y) as f64,
            ),
            LABEL_PILL,
            0.62,
        );
        let mut pen = left;
        for ch in text.chars() {
            let glyph = glyph(ch);
            for (row, bits) in glyph.iter().enumerate() {
                for col in 0..GLYPH_W {
                    if bits & (1 << (GLYPH_W - 1 - col)) == 0 {
                        continue;
                    }
                    for dy in 0..scale {
                        for dx in 0..scale {
                            self.blend(
                                pen + col as i64 * scale + dx,
                                top + row as i64 * scale + dy,
                                LABEL_TEXT,
                                1.0,
                            );
                        }
                    }
                }
            }
            pen += advance;
        }
    }
}

const GLYPH_W: usize = 5;
const GLYPH_H: usize = 7;
const PILL_PAD_X: usize = 2;
const PILL_PAD_Y: usize = 1;

/// 5x7 bitmap font covering the cell-code alphabet (`A`-`Z`, `0`-`9`, `.`).
/// Anything else renders as a filled box so a bad label is visible, not silent.
fn glyph(ch: char) -> [u8; GLYPH_H] {
    match ch.to_ascii_uppercase() {
        '0' => [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
        '1' => [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        '2' => [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
        '3' => [
            0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110,
        ],
        '4' => [
            0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010,
        ],
        '5' => [
            0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110,
        ],
        '6' => [
            0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
        '7' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
        '8' => [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
        '9' => [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100,
        ],
        'A' => [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'B' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
        'C' => [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
        'D' => [
            0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100,
        ],
        'E' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
        'F' => [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'G' => [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
        'H' => [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
        'I' => [
            0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
        'J' => [
            0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100,
        ],
        'K' => [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
        'L' => [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
        'M' => [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
        'N' => [
            0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001,
        ],
        'O' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'P' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
        'Q' => [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
        'R' => [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
        'S' => [
            0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
        'T' => [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'U' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
        'V' => [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
        'W' => [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001,
        ],
        'X' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
        'Y' => [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
        'Z' => [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
        '.' => [
            0b00000, 0b00000, 0b00000, 0b00000, 0b00000, 0b01100, 0b01100,
        ],
        _ => [
            0b11111, 0b11111, 0b11111, 0b11111, 0b11111, 0b11111, 0b11111,
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blank(width: u32, height: u32) -> Vec<u8> {
        vec![0u8; (width * height * 4) as usize]
    }

    fn identity() -> CaptureTransform {
        CaptureTransform {
            crop_x: 0.0,
            crop_y: 0.0,
            scale: 1.0,
        }
    }

    #[test]
    fn spec_parse_accepts_default_and_explicit_dimensions() {
        assert_eq!(GridSpec::parse(None), Some(GridSpec::default()));
        assert_eq!(GridSpec::parse(Some("")), Some(GridSpec::default()));
        assert_eq!(
            GridSpec::parse(Some("16x10")),
            Some(GridSpec {
                cols: 16,
                rows: 10,
                refine: None
            })
        );
        assert_eq!(GridSpec::parse(Some("16X10")).map(|s| s.rows), Some(10));
        assert_eq!(GridSpec::parse(Some("16")), None);
        assert_eq!(GridSpec::parse(Some("axb")), None);
        assert_eq!(
            GridSpec::parse(Some("0x8")),
            None,
            "zero columns is not a grid"
        );
    }

    #[test]
    fn only_a_dimension_pair_is_claimed_as_the_grid_value() {
        assert!(GridSpec::looks_like_dimensions("16x10"));
        assert!(GridSpec::looks_like_dimensions("4X4"));
        // The tokens that would otherwise be swallowed from the command line.
        assert!(!GridSpec::looks_like_dimensions("/tmp/shot.png"));
        assert!(!GridSpec::looks_like_dimensions("terminal"));
        assert!(!GridSpec::looks_like_dimensions("B7"));
        assert!(!GridSpec::looks_like_dimensions("x10"));
        assert!(!GridSpec::looks_like_dimensions("16x"));
        assert!(!GridSpec::looks_like_dimensions(""));
    }

    #[test]
    fn manifest_capture_coords_ignore_crop_and_scale_while_image_coords_follow_them() {
        let (w, h) = (400u32, 200u32);
        let mut rgba = blank(w, h);
        let transform = CaptureTransform {
            crop_x: 100.0,
            crop_y: 50.0,
            scale: 2.0,
        };
        // Region is in capture space: a 200x100 area starting at the crop origin.
        let manifest = composite(
            &mut rgba,
            w,
            h,
            &GridSpec {
                cols: 2,
                rows: 1,
                refine: None,
            },
            GridRect::new(100.0, 50.0, 200.0, 100.0),
            transform,
            (400, 200),
        )
        .expect("composite");
        assert_eq!(manifest.click_space, "capture");
        let a1 = &manifest.cells[0];
        assert_eq!(a1.code, "A1");
        // Capture space: untouched by crop/scale — directly clickable. The
        // 200x100 region splits into two 100x100 cells.
        assert_eq!((a1.capture.x, a1.capture.y), (100.0, 50.0));
        assert_eq!((a1.capture.w, a1.capture.h), (100.0, 100.0));
        assert_eq!((a1.capture.cx, a1.capture.cy), (150.0, 100.0));
        // Image space: crop subtracted, then scaled.
        assert_eq!((a1.image.x, a1.image.y), (0.0, 0.0));
        assert_eq!((a1.image.w, a1.image.h), (200.0, 200.0));
        assert_eq!((a1.image.cx, a1.image.cy), (100.0, 100.0));
        let a2 = &manifest.cells[1];
        assert_eq!(a2.code, "A2");
        assert_eq!(a2.capture.x, 200.0);
        assert_eq!(a2.image.x, 200.0);
    }

    #[test]
    fn composite_paints_only_inside_the_region() {
        let (w, h) = (100u32, 100u32);
        let mut rgba = blank(w, h);
        composite(
            &mut rgba,
            w,
            h,
            &GridSpec {
                cols: 1,
                rows: 1,
                refine: None,
            },
            GridRect::new(20.0, 20.0, 40.0, 40.0),
            identity(),
            (100, 100),
        )
        .expect("composite");
        let pixel = |x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [rgba[i], rgba[i + 1], rgba[i + 2]]
        };
        // Far outside the region: untouched.
        assert_eq!(pixel(5, 5), [0, 0, 0]);
        assert_eq!(pixel(90, 90), [0, 0, 0]);
        // The region's top-left corner carries the outline colour.
        assert!(pixel(20, 20)[0] > 0, "outline should tint the corner");
    }

    #[test]
    fn composite_never_writes_outside_the_buffer() {
        let (w, h) = (40u32, 40u32);
        let mut rgba = blank(w, h);
        // A region that runs well past the image on every side must clip, not panic.
        composite(
            &mut rgba,
            w,
            h,
            &GridSpec {
                cols: 3,
                rows: 3,
                refine: None,
            },
            GridRect::new(-50.0, -50.0, 500.0, 500.0),
            identity(),
            (40, 40),
        )
        .expect("composite");
        assert_eq!(rgba.len(), (w * h * 4) as usize);
    }

    #[test]
    fn refine_manifest_lists_the_nine_sub_cells() {
        let (w, h) = (240u32, 160u32);
        let mut rgba = blank(w, h);
        let manifest = composite(
            &mut rgba,
            w,
            h,
            &GridSpec {
                cols: 12,
                rows: 8,
                refine: Some("B7".to_string()),
            },
            GridRect::new(0.0, 0.0, 240.0, 160.0),
            identity(),
            (240, 160),
        )
        .expect("composite");
        assert_eq!(manifest.cells.len(), 9);
        assert_eq!(manifest.cells[0].code, "B7.1");
        assert_eq!(manifest.cells[8].code, "B7.9");
        assert_eq!(manifest.refine.as_deref(), Some("B7"));
    }

    #[test]
    fn refine_on_an_unknown_cell_is_an_error_not_a_silent_full_grid() {
        let (w, h) = (100u32, 100u32);
        let mut rgba = blank(w, h);
        let error = composite(
            &mut rgba,
            w,
            h,
            &GridSpec {
                cols: 4,
                rows: 4,
                refine: Some("Z99".to_string()),
            },
            GridRect::new(0.0, 0.0, 100.0, 100.0),
            identity(),
            (100, 100),
        )
        .expect_err("Z99 is outside a 4x4 grid");
        assert!(error.contains("Z99"), "error names the cell: {error}");
    }

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| part.to_string()).collect()
    }

    #[test]
    fn cli_claims_the_grid_value_only_when_it_is_a_dimension_pair() {
        // Bare flag before the output path must NOT swallow the path.
        let spec = screenshot_grid_from_args(&argv(["--grid", "/tmp/shot.png"].as_slice()))
            .expect("bare --grid");
        assert_eq!((spec.cols, spec.rows), (12, 8));
        let spec = screenshot_grid_from_args(&argv(["/tmp/shot.png", "--grid"].as_slice()))
            .expect("trailing --grid");
        assert_eq!((spec.cols, spec.rows), (12, 8));
        let spec = screenshot_grid_from_args(&argv(["--grid", "16x10"].as_slice()))
            .expect("explicit dimensions");
        assert_eq!((spec.cols, spec.rows), (16, 10));
        let spec =
            screenshot_grid_from_args(&argv(["--grid=6x4"].as_slice())).expect("inline form");
        assert_eq!((spec.cols, spec.rows), (6, 4));
    }

    #[test]
    fn cli_reads_refine_and_defaults_the_grid_around_it() {
        let spec = screenshot_grid_from_args(&argv(["--grid-refine", "B7"].as_slice()))
            .expect("refine implies a grid");
        assert_eq!(spec.refine.as_deref(), Some("B7"));
        assert_eq!((spec.cols, spec.rows), (12, 8));
        let spec =
            screenshot_grid_from_args(&argv(["--grid", "4x4", "--grid-refine=C2"].as_slice()))
                .expect("both flags");
        assert_eq!(spec.refine.as_deref(), Some("C2"));
        assert_eq!((spec.cols, spec.rows), (4, 4));
    }

    #[test]
    fn cli_returns_none_when_no_grid_flag_is_present() {
        assert!(screenshot_grid_from_args(&argv(["--scale", "2"].as_slice())).is_none());
    }

    #[test]
    fn cli_rejects_a_malformed_dimension_pair_instead_of_defaulting() {
        assert!(screenshot_grid_from_args(&argv(["--grid=0x8"].as_slice())).is_none());
        assert!(screenshot_grid_from_args(&argv(["--grid=nonsense"].as_slice())).is_none());
    }

    #[test]
    fn label_size_targets_the_dom_grid_pill_rather_than_filling_the_cell() {
        // A roomy 1920x1160 / 12x8 cell still gets a modest label, not a
        // billboard that hides the content underneath it.
        assert_eq!(glyph_scale_for(160.0, 145.0, 3.0, 1.0), 2);
        // A huge cell does not grow the label either.
        assert_eq!(glyph_scale_for(900.0, 900.0, 3.0, 1.0), 2);
    }

    #[test]
    fn label_size_follows_the_output_upscale() {
        assert_eq!(glyph_scale_for(480.0, 435.0, 3.0, 3.0), 6);
        assert_eq!(glyph_scale_for(320.0, 290.0, 3.0, 2.0), 4);
    }

    #[test]
    fn label_size_shrinks_to_fit_a_cramped_cell_and_never_reaches_zero() {
        assert_eq!(glyph_scale_for(8.0, 8.0, 2.0, 1.0), 1);
        assert_eq!(glyph_scale_for(1.0, 1.0, 4.0, 4.0), 1);
        assert!(glyph_scale_for(160.0, 120.0, 2.0, 1.0) >= glyph_scale_for(20.0, 15.0, 2.0, 1.0));
    }
}
