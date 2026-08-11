// Guards the emoji cell width the app ships. See docs/pending-bugs.md — the
// vendored xterm registers ONLY Unicode v6, where every Emoji_Presentation
// character is one cell wide while every modern agent CLI writes it as two. The
// disagreement shifts the rest of the line and a partial repaint strands the old
// glyph in the orphaned column, which is what the owner reported as "weird
// characters appearing here and there".
//
// This test runs against the SAME vendored bundle the app loads, so it fails if
// the bundle is upgraded to something that already scores emoji correctly (then
// the provider should be dropped, not kept) or if the table is edited wrongly.
const test = require('node:test');
const assert = require('node:assert');
const { createTerminal } = require('./harness.js');

const EMOJI_WIDE = [
            0x231A,0x231B,0x23E9,0x23EC,0x23F0,0x23F0,0x23F3,0x23F3,0x25FD,0x25FE,0x2614,0x2615,
            0x2648,0x2653,0x267F,0x267F,0x2693,0x2693,0x26A1,0x26A1,0x26AA,0x26AB,0x26BD,0x26BE,
            0x26C4,0x26C5,0x26CE,0x26CE,0x26D4,0x26D4,0x26EA,0x26EA,0x26F2,0x26F3,0x26F5,0x26F5,
            0x26FA,0x26FA,0x26FD,0x26FD,0x2705,0x2705,0x270A,0x270B,0x2728,0x2728,0x274C,0x274C,
            0x274E,0x274E,0x2753,0x2755,0x2757,0x2757,0x2795,0x2797,0x27B0,0x27B0,0x27BF,0x27BF,
            0x2B1B,0x2B1C,0x2B50,0x2B50,0x2B55,0x2B55,0x1F004,0x1F004,0x1F0CF,0x1F0CF,0x1F18E,
            0x1F18E,0x1F191,0x1F19A,0x1F1E6,0x1F1FF,0x1F201,0x1F201,0x1F21A,0x1F21A,0x1F22F,
            0x1F22F,0x1F232,0x1F236,0x1F238,0x1F23A,0x1F250,0x1F251,0x1F300,0x1F320,0x1F32D,
            0x1F335,0x1F337,0x1F37C,0x1F37E,0x1F393,0x1F3A0,0x1F3CA,0x1F3CF,0x1F3D3,0x1F3E0,
            0x1F3F0,0x1F3F4,0x1F3F4,0x1F3F8,0x1F43E,0x1F440,0x1F440,0x1F442,0x1F4FC,0x1F4FF,
            0x1F53D,0x1F54B,0x1F54E,0x1F550,0x1F567,0x1F57A,0x1F57A,0x1F595,0x1F596,0x1F5A4,
            0x1F5A4,0x1F5FB,0x1F64F,0x1F680,0x1F6C5,0x1F6CC,0x1F6CC,0x1F6D0,0x1F6D2,0x1F6D5,
            0x1F6D7,0x1F6DD,0x1F6DF,0x1F6EB,0x1F6EC,0x1F6F4,0x1F6FC,0x1F7E0,0x1F7EB,0x1F7F0,
            0x1F7F0,0x1F90C,0x1F93A,0x1F93C,0x1F945,0x1F947,0x1F9FF,0x1FA70,0x1FA74,0x1FA78,
            0x1FA7C,0x1FA80,0x1FA86,0x1FA90,0x1FAAC,0x1FAB0,0x1FABA,0x1FAC0,0x1FAC5,0x1FAD0,
            0x1FAD9,0x1FAE0,0x1FAE7,0x1FAF0,0x1FAF6
];

function installProvider(t) {
  const svc = t._core.unicodeService;
  const base = svc._providers['6'];
  const S = svc.constructor;
  const wide = (cp) => {
    let lo = 0, hi = (EMOJI_WIDE.length >> 1) - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      const a = EMOJI_WIDE[mid * 2], b = EMOJI_WIDE[mid * 2 + 1];
      if (cp < a) hi = mid - 1; else if (cp > b) lo = mid + 1; else return true;
    }
    return false;
  };
  t.unicode.register({
    version: '11',
    wcwidth: (cp) => (wide(cp) ? 2 : base.wcwidth(cp)),
    charProperties: (cp, pre) => {
      const p = base.charProperties(cp, pre);
      return wide(cp) ? S.createPropertyValue(S.extractShouldJoin(p), 2, true) : p;
    },
  });
  t.unicode.activeVersion = '11';
  return svc;
}

test('the vendored bundle alone scores emoji one cell wide — the bug', () => {
  const t = createTerminal({ cols: 40, rows: 6 });
  assert.deepStrictEqual(t.unicode.versions, ['6'],
    'if the bundle ever ships a newer table, delete the provider instead of keeping both');
  const svc = t._core.unicodeService;
  for (const cp of [0x2B50, 0x26D4, 0x2705, 0x1F680]) {
    assert.strictEqual(svc.wcwidth(cp), 1,
      `0x${cp.toString(16)} is expected to be WRONG (1) before the provider`);
  }
});

test('the provider widens exactly the Emoji_Presentation set', () => {
  const t = createTerminal({ cols: 40, rows: 6 });
  const svc = installProvider(t);
  const S = svc.constructor;
  const width = (cp) => [svc.wcwidth(cp), S.extractWidth(svc.charProperties(cp, 0))];

  // Emoji_Presentation=Yes must become two cells, in BOTH accessors — the
  // renderer reads charProperties, so wcwidth alone would fix nothing.
  for (const cp of [0x2B50 /* ⭐ */, 0x26D4 /* ⛔ */, 0x2705 /* ✅ */,
                    0x1F680 /* 🚀 */, 0x231A /* ⌚ */, 0x1FAF6 /* 🫶 */]) {
    assert.deepStrictEqual(width(cp), [2, 2], `0x${cp.toString(16)} must be wide`);
  }
  // Text-presentation symbols must STAY one cell. Widening these would create
  // the identical misalignment in the opposite direction — and the owner's own
  // frames showed ⚠ rendering correctly while ⭐ and ⛔ did not.
  for (const cp of [0x26A0 /* ⚠ */, 0x273B /* ✻ */, 0x276F /* ❯ */,
                    0x2714 /* ✔ */, 0x2139 /* ℹ */]) {
    assert.deepStrictEqual(width(cp), [1, 1], `0x${cp.toString(16)} must stay narrow`);
  }
  // Untouched by the provider.
  assert.deepStrictEqual(width(0x41), [1, 1], 'ASCII');
  assert.deepStrictEqual(width(0x4E2D), [2, 2], 'CJK was never wrong');
});

test('the table is sorted, non-overlapping and in pairs', () => {
  assert.strictEqual(EMOJI_WIDE.length % 2, 0, 'ranges come in pairs');
  for (let i = 0; i < EMOJI_WIDE.length; i += 2) {
    assert.ok(EMOJI_WIDE[i] <= EMOJI_WIDE[i + 1], `range ${i / 2} is inverted`);
    if (i > 0) {
      assert.ok(EMOJI_WIDE[i - 1] < EMOJI_WIDE[i],
        `range ${i / 2} overlaps or is unsorted — the binary search requires order`);
    }
  }
});
