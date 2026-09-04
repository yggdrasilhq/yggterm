// Overlay scroll-thumb geometry — the single owner of "where is the thumb"
// (XTERM-BUG: terminal-edge-unpaintable, see docs/xterm-bugs.md).
//
// WHY THIS EXISTS: the terminal used to reserve an 8px layout gutter so the
// native viewport scrollbar stayed hit-testable (XTERM-BUG:
// scrollbar-not-draggable), and that reservation — plus the canvas renderer's
// integer-cell raster of fractional font metrics — stranded a right and bottom
// strip no TUI cell could ever paint (measured live on guihost at 3.2.60: a
// 921x904 card painting a 912x900 canvas). The scrollbar is now drawn by the
// shell as an overlay ABOVE the grid, and the grid is proposed from the full
// host box. The JS that positions the thumb is a format! string in
// shell/terminal_scripts.rs (`syncXtermScrollThumb`); this module is the
// testable statement of the same decision. Keep the two mirrors in sync — the
// guard test in shell/tests.rs names both.

/// Geometry of the overlay thumb for one scroll state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ScrollThumbGeometry {
    /// False when the content does not overflow the track (no scrollback
    /// beyond the screen) — the thumb must hide, not merely zero-size.
    pub visible: bool,
    /// Thumb height in px; never below the minimum click target.
    pub height_px: f64,
    /// Thumb offset from the track top in px, clamped to the track.
    pub offset_px: f64,
}

/// Thumb size and position for a viewport scroll state.
///
/// `track_px` is the visible scroll height (clientHeight), `content_px` the
/// full scrollable extent (scrollHeight), `scroll_top_px` the current offset.
/// The thumb scales as track²/content (viewport-to-content ratio), bottoms out
/// at `min_thumb_px` so the drag target never collapses on long scrollback,
/// and its offset is the proportional position of `scroll_top_px` within the
/// scrollable range, clamped into the track. All inputs are clamped to be
/// finite and non-negative: a garbled metric (NaN from a detached element)
/// must hide the thumb, never panic the paint path.
pub(crate) fn scroll_thumb_geometry(
    content_px: f64,
    track_px: f64,
    scroll_top_px: f64,
    min_thumb_px: f64,
) -> ScrollThumbGeometry {
    let content = if content_px.is_finite() && content_px > 0.0 {
        content_px
    } else {
        0.0
    };
    let track = if track_px.is_finite() && track_px > 0.0 {
        track_px
    } else {
        0.0
    };
    let scroll_top = if scroll_top_px.is_finite() && scroll_top_px > 0.0 {
        scroll_top_px
    } else {
        0.0
    };
    let min_thumb = if min_thumb_px.is_finite() && min_thumb_px > 0.0 {
        min_thumb_px
    } else {
        0.0
    };
    if content <= track + 0.5 || track <= 0.0 {
        return ScrollThumbGeometry {
            visible: false,
            height_px: 0.0,
            offset_px: 0.0,
        };
    }
    let height = (track * track / content).max(min_thumb).min(track);
    let max_offset = (track - height).max(0.0);
    let scrollable = content - track;
    let offset = (scroll_top / scrollable * max_offset).clamp(0.0, max_offset);
    ScrollThumbGeometry {
        visible: true,
        height_px: height,
        offset_px: offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACK: f64 = 900.0;
    const MIN_THUMB: f64 = 36.0;

    #[test]
    fn no_overflow_hides_the_thumb() {
        let g = scroll_thumb_geometry(TRACK, TRACK, 0.0, MIN_THUMB);
        assert!(!g.visible);
        let g = scroll_thumb_geometry(TRACK + 0.4, TRACK, 0.0, MIN_THUMB);
        assert!(!g.visible, "sub-half-pixel overflow must not show a thumb");
    }

    #[test]
    fn thumb_scales_with_the_viewport_ratio_and_floors_at_minimum() {
        let g = scroll_thumb_geometry(TRACK * 2.0, TRACK, 0.0, MIN_THUMB);
        assert!(g.visible);
        assert_eq!(g.height_px, TRACK / 2.0);
        // 100 screens of scrollback: the raw ratio thumb would be 9px — the
        // minimum click target must win.
        let g = scroll_thumb_geometry(TRACK * 100.0, TRACK, 0.0, MIN_THUMB);
        assert_eq!(g.height_px, MIN_THUMB);
    }

    #[test]
    fn offset_tracks_scroll_top_and_clamps_into_the_track() {
        let content = TRACK * 2.0;
        let half = scroll_thumb_geometry(content, TRACK, TRACK / 2.0, MIN_THUMB);
        assert!((half.offset_px - (TRACK - TRACK / 2.0) / 2.0).abs() < 1e-9);
        let bottom = scroll_thumb_geometry(content, TRACK, TRACK, MIN_THUMB);
        assert_eq!(bottom.offset_px, TRACK - TRACK / 2.0);
        let huge = scroll_thumb_geometry(content, TRACK, f64::MAX, MIN_THUMB);
        assert_eq!(huge.offset_px, TRACK - TRACK / 2.0, "huge scrollTop clamps to bottom");
        let non_finite = scroll_thumb_geometry(content, TRACK, f64::INFINITY, MIN_THUMB);
        assert_eq!(non_finite.offset_px, 0.0, "non-finite input carries no position — safe top");
        let negative = scroll_thumb_geometry(content, TRACK, -5.0, MIN_THUMB);
        assert_eq!(negative.offset_px, 0.0);
    }

    #[test]
    fn garbled_metrics_hide_never_panic() {
        for (content, track, top) in [
            (f64::NAN, TRACK, 0.0),
            (0.0, 0.0, 0.0),
            (1000.0, f64::NAN, 0.0),
            (-1.0, 500.0, 0.0),
        ] {
            let g = scroll_thumb_geometry(content, track, top, MIN_THUMB);
            assert!(!g.visible, "({content},{track},{top}) must hide");
            assert_eq!(g.height_px, 0.0);
            assert_eq!(g.offset_px, 0.0);
        }
    }
}
