// Behavioral guard for the cell-aware retained-frame cache. This loads the
// exact JS module embedded by terminal_scripts.rs and replays its ANSI output
// through the exact vendored xterm.js build shipped by yggterm.

const { test } = require('node:test');
const assert = require('node:assert');

const { createTerminal, write } = require('./harness.js');
const frameCache = require('../../crates/yggterm-shell/src/shell/terminal_frame_cache.js');

function cellAt(term, row, col) {
  return term.buffer.active.getLine(row).getCell(col);
}

test('frame cache preserves RGB and palette colours plus text attributes', async () => {
  const source = createTerminal({ cols: 32, rows: 4, open: false });
  await write(
    source,
    '\x1b[38;2;255;0;0m\x1b[48;5;238m\x1b[1;4mRED\x1b[0m plain',
  );

  const serialized = frameCache.serializeBuffer(source, 32);
  assert.ok(serialized);
  assert.strictEqual(serialized.text.includes('RED plain'), true);
  assert.strictEqual(serialized.hasAttributes, true);
  assert.ok(serialized.colorCellCount >= 3);
  assert.match(serialized.ansiText, /\x1b\[0;1;4;38;2;255;0;0;48;5;238m/);

  const restored = createTerminal({ cols: 32, rows: 4, open: false });
  await write(restored, serialized.ansiText);
  const red = cellAt(restored, 0, 0);
  assert.strictEqual(red.getChars(), 'R');
  assert.strictEqual(red.isFgRGB(), true);
  assert.strictEqual(red.getFgColor(), 0xff0000);
  assert.strictEqual(red.isBgPalette(), true);
  assert.strictEqual(red.getBgColor(), 238);
  assert.strictEqual(Boolean(red.isBold()), true);
  assert.strictEqual(Boolean(red.isUnderline()), true);

  const plain = cellAt(restored, 0, 4);
  assert.strictEqual(plain.getChars(), 'p');
  assert.strictEqual(plain.isFgDefault(), true);
  assert.strictEqual(plain.isBgDefault(), true);
  assert.strictEqual(Boolean(plain.isBold()), false);
});

test('frame cache joins wrapped visual lines without inventing a newline', async () => {
  const source = createTerminal({ cols: 5, rows: 2, open: false });
  await write(source, 'abcdef');
  const serialized = frameCache.serializeBuffer(source, 16);
  assert.strictEqual(serialized.text.includes('abcdef'), true);
  assert.strictEqual(serialized.logicalLineCount, 1);
  assert.strictEqual(serialized.visualLineCount >= 2, true);
});

