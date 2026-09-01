// ── the frame-hash probe (client half) ──────────────────────────────
// ACT V of the opencode-integration campaign (research/opencode-integration
// board): "opencode rows load as GHOSTS — stale frame stitched with new
// output" was undebuggable because nothing compared what the DAEMON's
// authoritative grid holds against what the CLIENT's applied buffer shows.
// This module is the client half of that pairing: it canonicalizes the
// visible viewport — one right-trimmed string per row, blank rows KEPT
// (shape is information), then the grid size and the cursor — and hashes it
// with FNV-1a 32.
//
// ⛔ ONE ALGORITHM, TWO IMPLEMENTATIONS, ONE TEST VECTOR. The daemon's twin
// lives in `crates/yggterm-server/src/frame_hash.rs` and must produce the
// same hash for the same grid. `tools/xterm-harness/frame_hash_probe.test.js`
// and the Rust unit test pin the SAME literal, so the two implementations
// cannot drift without a red test. Change the canonical form here and you
// change it there in the same commit.
//
// ⛔ THE PROBE IS A WITNESS, NOT A RENDER PATH. It runs after a write flush
// settles; it never writes, scrolls, or re-fits anything, and a probe
// failure must be swallowed where it stands.
//
// Pairing rule (the reason the daemon hash rides along): a MISMATCH at
// quiescence while at-bottom is artifacting, objectively, no pixels. A
// mismatch while scrolled back is NOT — the viewport shows history the
// daemon's screen does not hold — so `atBottom` gates the mismatch verdict
// and rides in every event for the reader.
(function (global) {
    'use strict';
    const FNV_OFFSET = 0x811c9dc5;
    const FNV_PRIME = 0x01000193;

    function fnv1a32(bytes) {
        let hash = FNV_OFFSET;
        for (let i = 0; i < bytes.length; i++) {
            hash ^= bytes[i];
            hash = Math.imul(hash, FNV_PRIME) >>> 0;
        }
        return hash >>> 0;
    }

    // The canonical form: rows top-to-bottom right-trimmed, blank rows kept,
    // then `<rows>x<cols>@<cursorRow>,<cursorCol>`. Identical to the daemon's
    // `canonical_frame_text` — see the header.
    function canonicalFrameTextOf(term) {
        const buffer = term.buffer.active;
        const rows = term.rows;
        const cols = term.cols;
        const baseY = buffer.baseY;
        const parts = [];
        for (let y = 0; y < rows; y++) {
            const line = buffer.getLine(baseY + y);
            parts.push(line ? line.translateToString(true) : '');
        }
        parts.push(rows + 'x' + cols + '@' + buffer.cursorY + ',' + buffer.cursorX);
        return parts.join('\n');
    }

    // Hash the terminal's visible viewport. Returns
    // `{ hash, atBottom }`, or `null` when there is nothing to read yet.
    function frameHashOf(term) {
        try {
            if (!term || !term.buffer || !term.buffer.active) {
                return null;
            }
            const buffer = term.buffer.active;
            const atBottom = buffer.viewportY === buffer.baseY;
            const text = canonicalFrameTextOf(term);
            const hash = 'fnv32:' + fnv1a32(new TextEncoder().encode(text)).toString(16).padStart(8, '0');
            return { hash: hash, atBottom: atBottom };
        } catch (_probeError) {
            return null;
        }
    }

    global.__yggtermFrameHash = {
        fnv1a32: fnv1a32,
        canonicalFrameTextOf: canonicalFrameTextOf,
        frameHashOf: frameHashOf,
    };
})(typeof globalThis !== 'undefined' ? globalThis : window);
