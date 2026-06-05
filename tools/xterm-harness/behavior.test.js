// Client-layer behavioral guards: assert the EXACT vendored xterm.js agrees
// with the behaviors our daemon-side fixes + findings depend on. If an xterm
// bump changes any of these, the campaign's assumptions break — fail loudly.
// See campaign-xterm-dealbreakers + finding-codex-owns-scrollback-not-term-program.
const { test } = require('node:test');
const assert = require('node:assert');
const h = require('./harness');

test('normal newline output grows scrollback and retains scrolled-off lines', async () => {
  const term = h.createTerminal({ cols: 80, rows: 5, scrollback: 1000 });
  let payload = '';
  for (let i = 1; i <= 20; i++) payload += `line ${i}\r\n`;
  await h.write(term, payload);
  assert.ok(h.baseY(term) > 0, `expected scrollback to grow, baseY=${h.baseY(term)}`);
  // The first line scrolled off the viewport but must be retained in scrollback.
  assert.match(h.lineText(term, 0) || '', /line 1\b/, 'scrolled-off line 1 must be retained');
  // The latest line is at/near the bottom.
  assert.match(h.bufferText(term), /line 20\b/, 'latest line must be present');
});

test('cursor-addressed repaint (codex steady-state) keeps baseY at 0 — no scrollback', async () => {
  // codex's working loop is pure absolute addressing (CSI r;c H), no newlines,
  // no full-screen scroll → it generates ZERO terminal scrollback. This is why
  // the daemon's clean-scrollback base_y is correctly 0 for codex.
  // (finding-codex-owns-scrollback-not-term-program)
  const term = h.createTerminal({ cols: 80, rows: 10, scrollback: 1000 });
  let frame = '\x1b[2J\x1b[H';
  for (let r = 1; r <= 10; r++) frame += `\x1b[${r};1Hrow ${r} content`;
  // Repaint several times (animation) — still must not scroll.
  await h.write(term, frame + frame + frame);
  assert.strictEqual(h.baseY(term), 0, `cursor-addressed repaint must not grow scrollback, baseY=${h.baseY(term)}`);
  assert.match(h.lineText(term, 0) || '', /row 1 content/);
  assert.match(h.lineText(term, 9) || '', /row 10 content/);
});

test('reverse-index inside a full-screen scroll region does not grow scrollback', async () => {
  // codex opens space with DECSTBM + reverse-index (ESC M), never forward
  // scroll. Reverse-index must NOT push lines into scrollback.
  const term = h.createTerminal({ cols: 80, rows: 10, scrollback: 1000 });
  // Fill the screen first (absolute, no scroll).
  let frame = '\x1b[2J\x1b[H';
  for (let r = 1; r <= 10; r++) frame += `\x1b[${r};1Hline ${r}`;
  await h.write(term, frame);
  assert.strictEqual(h.baseY(term), 0);
  // Set full-screen region, home, reverse-index x4 (codex's open-space pattern).
  await h.write(term, '\x1b[1;10r\x1b[1;1H\x1bM\x1bM\x1bM\x1bM\x1b[r');
  assert.strictEqual(h.baseY(term), 0, `reverse-index must not grow scrollback, baseY=${h.baseY(term)}`);
});

test('a painted background colour survives a column WIDEN reflow for already-written cells', async () => {
  // Guards the reflow-bg invariant our composer reconcile depends on: when the
  // terminal widens, cells that were already written WITH a bg keep that bg
  // (the composer bg-split is the codex DELTA case where un-rewritten cells stay
  // default — reproduced end-to-end in the daemon pipeline, not here; this locks
  // the baseline that a written bg is not silently dropped by reflow itself).
  const term = h.createTerminal({ cols: 40, rows: 5, scrollback: 1000 });
  // Paint row 0 fully with palette-238 bg.
  await h.write(term, '\x1b[H\x1b[48;5;238m' + 'x'.repeat(40) + '\x1b[0m');
  let bg = h.cellBg(term, 0, 20);
  assert.ok(bg && bg.isPalette && bg.color === 238, `pre-reflow cell must carry the bg, got ${JSON.stringify(bg)}`);
  term.resize(80, 5); // widen → reflow
  bg = h.cellBg(term, 0, 20);
  assert.ok(bg && bg.isPalette && bg.color === 238, `written bg must survive widen reflow, got ${JSON.stringify(bg)}`);
});
