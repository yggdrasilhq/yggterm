//! Click-grid geometry — the ONE owner of "what rectangle is cell B7".
//!
//! Two consumers share this math and must never re-derive it:
//!
//! * the **live DOM grid** (`server app grid show/click`, painted into the page
//!   via `click_grid_core_script` in the shell), and
//! * the **capture-side grid** (`server app screenshot --grid`, composited into
//!   the returned PNG only — the agent-presence rung of the control plane, see
//!   `docs/agent-control-plane.md` slice 3).
//!
//! Cell codes are spreadsheet-style: a bijective base-26 row label (`A`..`Z`,
//! `AA`, `AB`, …) followed by a 1-based column number, optionally refined by a
//! `.1`..`.9` sub-cell (3x3, row-major). `B7` and `B7.5` are both valid.

use serde::{Deserialize, Serialize};

/// An axis-aligned rectangle in whatever pixel space the caller supplied for
/// `GridGeometry::region` — CSS pixels for the DOM grid, capture pixels for the
/// screenshot grid. This type deliberately carries no space tag: the caller
/// owns that meaning and labels it in its own output.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct GridRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl GridRect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }

    /// Center point — the coordinate a click resolves to.
    pub fn center(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

/// A parsed cell code: zero-based row/col plus an optional 1..=9 sub-cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedCell {
    pub row: u32,
    pub col: u32,
    pub sub: Option<u8>,
}

/// One labelled cell of a rendered grid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GridCell {
    pub code: String,
    pub rect: GridRect,
}

/// Cols x rows over a region rect. All cell math derives from these three.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridGeometry {
    pub cols: u32,
    pub rows: u32,
    pub region: GridRect,
}

/// Sub-cells per axis in refine mode (3x3 = 9 labels, `.1`..`.9`).
pub const REFINE_DIVISIONS: u32 = 3;

impl GridGeometry {
    /// Clamps `cols`/`rows` to at least 1 so no caller can build a geometry
    /// that divides by zero.
    pub fn new(cols: u32, rows: u32, region: GridRect) -> Self {
        Self {
            cols: cols.max(1),
            rows: rows.max(1),
            region,
        }
    }

    /// Bijective base-26 row label: 0 -> `A`, 25 -> `Z`, 26 -> `AA`.
    pub fn row_label(index: u32) -> String {
        let mut out = String::new();
        let mut i = index as u64 + 1;
        while i > 0 {
            i -= 1;
            out.insert(0, (b'A' + (i % 26) as u8) as char);
            i /= 26;
        }
        out
    }

    /// Inverse of [`row_label`], plus the column and optional sub-cell. Returns
    /// `None` for malformed codes and for cells outside this geometry.
    ///
    /// [`row_label`]: GridGeometry::row_label
    pub fn parse_cell(&self, code: &str) -> Option<ParsedCell> {
        let code = code.trim().to_ascii_uppercase();
        let (head, sub) = match code.split_once('.') {
            Some((head, tail)) => {
                let value: u8 = tail.parse().ok()?;
                if !(1..=(REFINE_DIVISIONS * REFINE_DIVISIONS) as u8).contains(&value) {
                    return None;
                }
                (head.to_string(), Some(value))
            }
            None => (code, None),
        };
        let split = head.find(|ch: char| ch.is_ascii_digit())?;
        let (letters, digits) = head.split_at(split);
        if letters.is_empty() || !letters.chars().all(|ch| ch.is_ascii_uppercase()) {
            return None;
        }
        if digits.is_empty() || !digits.chars().all(|ch| ch.is_ascii_digit()) {
            return None;
        }
        let mut row: u64 = 0;
        for ch in letters.chars() {
            row = row
                .checked_mul(26)?
                .checked_add((ch as u8 - b'A') as u64 + 1)?;
        }
        let row = u32::try_from(row.checked_sub(1)?).ok()?;
        let col = digits.parse::<u32>().ok()?.checked_sub(1)?;
        if row >= self.rows || col >= self.cols {
            return None;
        }
        Some(ParsedCell { row, col, sub })
    }

    /// Rect of the zero-based `(row, col)` cell. Out-of-range indices are
    /// clamped rather than panicking — callers validate via `parse_cell`.
    pub fn cell_rect(&self, row: u32, col: u32) -> GridRect {
        let row = row.min(self.rows - 1) as f64;
        let col = col.min(self.cols - 1) as f64;
        let w = self.region.w / self.cols as f64;
        let h = self.region.h / self.rows as f64;
        GridRect::new(self.region.x + col * w, self.region.y + row * h, w, h)
    }

    /// Rect of one 1..=9 sub-cell inside `cell`, row-major.
    pub fn sub_rect(cell: GridRect, sub: u8) -> GridRect {
        let index = (sub.clamp(1, (REFINE_DIVISIONS * REFINE_DIVISIONS) as u8) - 1) as u32;
        let sr = (index / REFINE_DIVISIONS) as f64;
        let sc = (index % REFINE_DIVISIONS) as f64;
        let w = cell.w / REFINE_DIVISIONS as f64;
        let h = cell.h / REFINE_DIVISIONS as f64;
        GridRect::new(cell.x + sc * w, cell.y + sr * h, w, h)
    }

    /// Resolve a cell code (`B7` or `B7.5`) to its rect.
    pub fn resolve(&self, code: &str) -> Option<GridRect> {
        let parsed = self.parse_cell(code)?;
        let rect = self.cell_rect(parsed.row, parsed.col);
        Some(match parsed.sub {
            Some(sub) => Self::sub_rect(rect, sub),
            None => rect,
        })
    }

    /// The full cell table in row-major order — the manifest a `show`/`--grid`
    /// response hands back.
    pub fn cells(&self) -> Vec<GridCell> {
        let mut out = Vec::with_capacity((self.rows as usize) * (self.cols as usize));
        for row in 0..self.rows {
            let label = Self::row_label(row);
            for col in 0..self.cols {
                out.push(GridCell {
                    code: format!("{label}{}", col + 1),
                    rect: self.cell_rect(row, col),
                });
            }
        }
        out
    }

    /// The nine sub-cells of `code` (refine mode). `None` if `code` is not a
    /// cell of this geometry; a code that already names a sub-cell refines its
    /// parent, so `B7.5` and `B7` produce the same table.
    pub fn refine_cells(&self, code: &str) -> Option<Vec<GridCell>> {
        let parsed = self.parse_cell(code)?;
        let parent_code = format!("{}{}", Self::row_label(parsed.row), parsed.col + 1);
        let parent = self.cell_rect(parsed.row, parsed.col);
        Some(
            (1..=(REFINE_DIVISIONS * REFINE_DIVISIONS) as u8)
                .map(|sub| GridCell {
                    code: format!("{parent_code}.{sub}"),
                    rect: Self::sub_rect(parent, sub),
                })
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry() -> GridGeometry {
        GridGeometry::new(12, 8, GridRect::new(0.0, 0.0, 1200.0, 800.0))
    }

    #[test]
    fn row_labels_are_bijective_base_26() {
        assert_eq!(GridGeometry::row_label(0), "A");
        assert_eq!(GridGeometry::row_label(25), "Z");
        assert_eq!(GridGeometry::row_label(26), "AA");
        assert_eq!(GridGeometry::row_label(27), "AB");
        assert_eq!(GridGeometry::row_label(51), "AZ");
        assert_eq!(GridGeometry::row_label(52), "BA");
        assert_eq!(GridGeometry::row_label(701), "ZZ");
        assert_eq!(GridGeometry::row_label(702), "AAA");
    }

    #[test]
    fn parse_cell_round_trips_every_label() {
        let grid = GridGeometry::new(4, 30, GridRect::new(0.0, 0.0, 400.0, 3000.0));
        for cell in grid.cells() {
            let parsed = grid
                .parse_cell(&cell.code)
                .unwrap_or_else(|| panic!("{} should parse", cell.code));
            assert_eq!(grid.cell_rect(parsed.row, parsed.col), cell.rect);
        }
    }

    #[test]
    fn parse_cell_rejects_out_of_range_and_malformed() {
        let grid = geometry();
        assert!(grid.parse_cell("A13").is_none(), "col past cols");
        assert!(grid.parse_cell("I1").is_none(), "row past rows");
        assert!(grid.parse_cell("").is_none());
        assert!(grid.parse_cell("7").is_none(), "no row letters");
        assert!(grid.parse_cell("A").is_none(), "no column number");
        assert!(grid.parse_cell("A0").is_none(), "columns are 1-based");
        assert!(grid.parse_cell("A1.0").is_none(), "sub-cells are 1-based");
        assert!(grid.parse_cell("A1.10").is_none(), "only nine sub-cells");
        assert!(grid.parse_cell("A1B").is_none(), "trailing garbage");
    }

    #[test]
    fn parse_cell_is_case_insensitive_and_trims() {
        let grid = geometry();
        assert_eq!(grid.parse_cell(" b7 "), grid.parse_cell("B7"));
    }

    #[test]
    fn cell_rects_tile_the_region_without_gaps() {
        let grid = geometry();
        let cells = grid.cells();
        assert_eq!(cells.len(), 96);
        assert_eq!(cells[0].rect, GridRect::new(0.0, 0.0, 100.0, 100.0));
        let last = &cells[cells.len() - 1];
        assert_eq!(last.code, "H12");
        assert_eq!(last.rect, GridRect::new(1100.0, 700.0, 100.0, 100.0));
    }

    #[test]
    fn region_offset_is_carried_into_every_cell() {
        let grid = GridGeometry::new(2, 2, GridRect::new(40.0, 10.0, 200.0, 100.0));
        assert_eq!(
            grid.resolve("A1").unwrap(),
            GridRect::new(40.0, 10.0, 100.0, 50.0)
        );
        assert_eq!(
            grid.resolve("B2").unwrap(),
            GridRect::new(140.0, 60.0, 100.0, 50.0)
        );
    }

    #[test]
    fn refine_subdivides_row_major_into_nine() {
        let grid = geometry();
        let cells = grid.refine_cells("B7").expect("B7 refines");
        assert_eq!(cells.len(), 9);
        assert_eq!(cells[0].code, "B7.1");
        // B7 = row 1, col 6 -> origin (600, 100), 100x100 cell, 33.33 sub-cells.
        let (cx, cy) = cells[0].rect.center();
        assert!((cx - 616.666).abs() < 0.01, "cx was {cx}");
        assert!((cy - 116.666).abs() < 0.01, "cy was {cy}");
        assert_eq!(cells[4].code, "B7.5");
        assert_eq!(grid.resolve("B7.5").unwrap(), cells[4].rect);
        // The center sub-cell shares its center with the parent cell.
        assert_eq!(cells[4].rect.center(), grid.resolve("B7").unwrap().center());
        assert_eq!(cells[8].code, "B7.9");
    }

    #[test]
    fn refining_a_sub_cell_refines_its_parent() {
        let grid = geometry();
        assert_eq!(grid.refine_cells("B7.3"), grid.refine_cells("B7"));
    }

    #[test]
    fn degenerate_dimensions_clamp_instead_of_dividing_by_zero() {
        let grid = GridGeometry::new(0, 0, GridRect::new(0.0, 0.0, 10.0, 10.0));
        assert_eq!(grid.cols, 1);
        assert_eq!(grid.rows, 1);
        assert_eq!(grid.cells().len(), 1);
        assert_eq!(
            grid.resolve("A1").unwrap(),
            GridRect::new(0.0, 0.0, 10.0, 10.0)
        );
    }
}
