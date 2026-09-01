// Behavioral guard for the frame-hash probe's CLIENT half.
//
// The subject is `crates/yggterm-shell/src/shell/frame_hash_probe.js`, loaded
// here as the SAME bytes the Rust side `include_str!`s into the terminal
// script — no transcription, so a hash asserted here is a hash the webview
// computes. It runs against the EXACT vendored xterm.js (harness.js), so the
// canonicalization of real buffers (translateToString trimming, wide rows,
// cursor position) is the one that ships.
//
// ⛔ ONE ALGORITHM, TWO IMPLEMENTATIONS, ONE TEST VECTOR. The daemon's Rust
// twin (`crates/yggterm-server/src/frame_hash.rs`) pins the SAME literal in
// `the_test_vector_matches_the_client_implementation`. If both tests are not
// green against the same string, the two halves cannot be paired and every
// frame_hash_probe event is noise.

const { test } = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const { createTerminal, write } = require('./harness.js');

const PROBE_JS = path.resolve(
  __dirname, '..', '..',
  'crates', 'yggterm-shell', 'src', 'shell', 'frame_hash_probe.js',
);
require(PROBE_JS);
const probe = globalThis.__yggtermFrameHash;

test('the probe module is present and complete', () => {
  assert.ok(probe, 'frame_hash_probe.js must attach __yggtermFrameHash on require');
  assert.strictEqual(typeof probe.fnv1a32, 'function');
  assert.strictEqual(typeof probe.canonicalFrameTextOf, 'function');
  assert.strictEqual(typeof probe.frameHashOf, 'function');
});

test('the client frame hash matches the cross-implementation test vector', async () => {
  // THE shared vector with the Rust twin: hello / world / blank / prom>,
  // 20 cols, cursor parked at (row 3, col 5) by an absolute CUP.
  const term = createTerminal({ cols: 20, rows: 4, open: false });
  await write(term, 'hello\r\nworld\r\n\x1b[4;1Hprom>');
  const reading = probe.frameHashOf(term);
  assert.strictEqual(
    reading.hash,
    'fnv32:adc779eb',
    'the client half drifted from the pinned cross-implementation vector',
  );
  assert.strictEqual(reading.atBottom, true, 'a fresh grid sits at bottom');
});

test('trailing blanks are trimmed per row and blank rows are kept', async () => {
  // "abc" + 17 trailing columns must hash like "abc"; a blank MIDDLE row
  // must still contribute its (empty) line to the shape.
  const trimmed = createTerminal({ cols: 20, rows: 2, open: false });
  await write(trimmed, 'abc');
  const blankKept = createTerminal({ cols: 20, rows: 2, open: false });
  await write(blankKept, 'abc\r\n\x1b[2;1H');

  const rowsOf = (term) => probe.canonicalFrameTextOf(term);
  assert.strictEqual(rowsOf(trimmed), 'abc\n\n2x20@0,3');
  assert.strictEqual(rowsOf(blankKept), 'abc\n\n2x20@1,0');
  assert.notStrictEqual(
    probe.frameHashOf(trimmed).hash,
    probe.frameHashOf(blankKept).hash,
    'cursor position is part of the frame',
  );
});

test('the live-frame hash is scroll-invariant; atBottom gates the pairing', async () => {
  // The hash anchors to baseY — the LIVE bottom frame — so scrolling back
  // must NOT change it. What changes is `atBottom`: a scrolled-back viewport
  // legitimately differs from the daemon's bottom screen, so the pairing
  // verdict gates on the flag, not on the hash moving.
  const term = createTerminal({ cols: 20, rows: 4, scrollback: 100, open: false });
  let data = '';
  for (let i = 0; i < 20; i++) {
    data += 'line' + i + '\r\n';
  }
  await write(term, data);
  const atBottom = probe.frameHashOf(term);
  assert.strictEqual(atBottom.atBottom, true);
  term.scrollLines(-5);
  const scrolled = probe.frameHashOf(term);
  assert.strictEqual(
    scrolled.atBottom,
    false,
    'a scrolled-back viewport must report atBottom=false',
  );
  assert.strictEqual(
    scrolled.hash,
    atBottom.hash,
    'the hash is the live bottom frame and must be scroll-invariant',
  );
});
