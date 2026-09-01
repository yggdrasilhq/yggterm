// The frame-hash probe (daemon half) — a content hash of the AUTHORITATIVE
// viewport grid the daemon serves, for pairing against the client's
// applied-frame hash (ACT V, research/opencode-integration board: the
// ghost/glyph/squish triad needed an objective artifact detector — "a
// mismatch IS artifacting, no pixels").
//
// ⛔ ONE ALGORITHM, TWO IMPLEMENTATIONS, ONE TEST VECTOR. The client's twin
// lives in `crates/yggterm-shell/src/shell/frame_hash_probe.js` (shared
// verbatim with `tools/xterm-harness/frame_hash_probe.test.js`). Both tests
// pin the same literal for the same grid, so the two implementations cannot
// drift without a red test. Change the canonical form here and you change it
// there in the same commit.
//
// Canonical form: the visible viewport rows top-to-bottom, right-trimmed,
// blank rows KEPT (shape is information), then
// `<rows>x<cols>@<cursorRow>,<cursorCol>` — hashed FNV-1a 32. Deliberately
// coarse: characters only, no attributes — the ghost family (stitched stale
// frames, re-wrapped rows, orphaned fragments) is a CHARACTER defect, and a
// canonical form that ignores color survives color-only repaint noise.
//
// ⛔ ABSENT IS NOT ZERO. Every failure of this probe answers `None`, never an
// empty string or a hash of nothing — a reader must be able to tell "the
// daemon did not answer" from "the daemon hashed something".

/// FNV-1a 32-bit offset basis.
pub(crate) const FNV_OFFSET: u32 = 0x811c_9dc5;
/// FNV-1a 32-bit prime.
pub(crate) const FNV_PRIME: u32 = 0x0100_0193;

pub(crate) fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut hash = FNV_OFFSET;
    for &byte in bytes {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

/// The canonical text both implementations hash. `rows` are the visible
/// viewport rows (already right-trimmed by `vt_screen_plain_rows`), `cols`
/// the grid width, `cursor` the vt100 cursor as `(row, col)`, 0-based.
pub(crate) fn canonical_frame_text(rows: &[String], cols: u16, cursor: (u16, u16)) -> String {
    let mut text = rows.join("\n");
    text.push('\n');
    text.push_str(&format!(
        "{}x{}@{},{}",
        rows.len(),
        cols,
        cursor.0,
        cursor.1
    ));
    text
}

/// The daemon's grid hash: `"fnv32:"` + 8 lowercase hex digits.
pub(crate) fn frame_hash(rows: &[String], cols: u16, cursor: (u16, u16)) -> String {
    format!(
        "fnv32:{:08x}",
        fnv1a32(canonical_frame_text(rows, cols, cursor).as_bytes())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_vector_matches_the_client_implementation() {
        // THE shared vector with frame_hash_probe.js: xterm-harness writes
        // the same bytes into the REAL vendored xterm.js and asserts the
        // same literal. If you changed either canonical form, this test and
        // that one fail together — fix both in one commit.
        let rows = vec![
            "hello".to_string(),
            "world".to_string(),
            String::new(),
            "prom>".to_string(),
        ];
        assert_eq!(
            frame_hash(&rows, 20, (3, 5)),
            "fnv32:adc779eb",
            "the daemon half drifted from the pinned cross-implementation vector"
        );
    }

    #[test]
    fn trailing_whitespace_and_blank_rows_are_canonicalized_deterministically() {
        let a = vec!["hello".to_string(), String::new()];
        let b = vec!["hello   ".to_string(), "  ".to_string()];
        // rows arrive pre-trimmed from vt_screen_plain_rows, but the hash of
        // the trimmed forms must agree with the hash of their untrimmed
        // inputs' canonical TEXT only after trimming — so canonical_frame_text
        // itself must not care, and callers must always pass trimmed rows.
        // Pin the canonical text form instead of re-trimming here, so the JS
        // twin (translateToString(true)) and this stay one algorithm.
        assert_eq!(canonical_frame_text(&a, 20, (0, 0)), "hello\n\n2x20@0,0");
        assert_eq!(
            canonical_frame_text(&b, 20, (0, 0)),
            "hello   \n  \n2x20@0,0",
            "canonical_frame_text does NOT trim — trimming is the caller's job"
        );
    }
}
