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

test('TODO-1 root-direction: codex frame survives a reveal row-resize (so the blink is RESEED, not grid reflow)', async () => {
  // The switch-reveal blink (~70% of switches, campaign TODO-1) was HYPOTHESIZED
  // to be a cap-8 retained host whose grid drifted while hidden -> reveal re-fit
  // reflow. This test FALSIFIES that hypothesis on the EXACT vendored xterm.js and
  // redirects the fix: a codex steady-state frame (pure absolute addressing,
  // baseY=0) is NON-destructive across a row-resize down-then-back-up, so a reveal
  // re-fit does NOT lose/shift the composer-bottom content. Therefore the visible
  // "shadow flash + broken bottom paint" is the reveal RESEED (the client paints
  // its stale retained buffer, then reconciles from the daemon) — the Class A
  // reveal-reconcile path — NOT grid sizing. See campaign-xterm-dealbreakers
  // CLASS A + audit-viewport-scroll-control-flow.
  const buildCodexFrame = (rows) => {
    let frame = '\x1b[2J\x1b[H';
    for (let r = 1; r <= rows; r++) {
      frame += `\x1b[${r};1H` + (r === rows ? '> composer prompt row' : `transcript row ${r}`);
    }
    return frame;
  };

  // (a) same-grid reveal is a true no-op (frame + baseY byte-identical).
  const sized = h.createTerminal({ cols: 159, rows: 63, scrollback: 1000 });
  await h.write(sized, buildCodexFrame(63));
  const before = h.bufferText(sized);
  const beforeBaseY = h.baseY(sized);
  sized.resize(159, 63);
  assert.strictEqual(h.bufferText(sized), before, 'same-grid reveal must be a true no-op');
  assert.strictEqual(h.baseY(sized), beforeBaseY, 'same-grid reveal must not move baseY');

  // (b) FALSIFICATION: row-resize 63->40->63 of a codex frame PRESERVES the
  // composer-bottom row. Grid drift is NOT the flicker source -> the fix is in the
  // reseed/reconcile path, not host sizing.
  const drifted = h.createTerminal({ cols: 159, rows: 63, scrollback: 1000 });
  await h.write(drifted, buildCodexFrame(63));
  const composerRowBefore = h.lineText(drifted, 62);
  drifted.resize(159, 40);
  drifted.resize(159, 63);
  assert.strictEqual(
    h.lineText(drifted, 62),
    composerRowBefore,
    'codex composer row must survive a row-resize round-trip (grid drift is NOT the blink)'
  );
});

test('bg->fg broken bottom self-corrects on the NEXT codex CUP frame (answers: not indefinite)', async () => {
  // The bg->fg break: a focus-regain repaint from the stale CLIENT snapshot leaves
  // the bottom rows blank/clipped (missing the codex composer + footer). Question:
  // does it stay broken indefinitely, or self-correct when codex next emits? codex
  // repaints its live bottom region with ABSOLUTE CUP every frame, so its next
  // output must overwrite the stale rows. This proves the answer deterministically
  // on the EXACT vendored xterm.js: broken bottom is TRANSIENT-until-next-codex-frame
  // (i.e. it self-corrects the moment the user types / codex animates), NOT indefinite.
  const rows = 10;
  const term = h.createTerminal({ cols: 80, rows, scrollback: 1000 });
  const composerRow = rows - 1; // 0-based; codex composer just above the footer
  const footerRow = rows;
  // 1) codex paints a correct frame: transcript + composer + footer (1-based CUP).
  let frame = '\x1b[2J\x1b[H';
  for (let r = 1; r <= rows - 2; r++) frame += `\x1b[${r};1Htranscript line ${r}`;
  frame += `\x1b[${composerRow};1H> the codex composer`;
  frame += `\x1b[${footerRow};1Hgpt-5.5 medium · ~/proj`;
  await h.write(term, frame);
  assert.match(h.lineText(term, composerRow - 1) || '', /the codex composer/);

  // 2) bg->fg break: the stale-snapshot repaint blanks the bottom region (erase the
  //    composer + footer rows in place — what the user sees as "broken bottom").
  await h.write(term, `\x1b[${composerRow};1H\x1b[K\x1b[${footerRow};1H\x1b[K`);
  assert.doesNotMatch(h.lineText(term, composerRow - 1) || '', /codex composer/, 'bottom is now broken (composer erased)');

  // 3) codex's NEXT frame: it repaints the live bottom region via absolute CUP.
  const nextFrame = `\x1b[${composerRow};1H> the codex composer\x1b[${footerRow};1Hgpt-5.5 medium · ~/proj`;
  await h.write(term, nextFrame);

  // 4) ASSERT: the composer + footer are restored -> self-corrects on the next codex
  //    frame. So the live behavior is "broken until codex next emits (e.g. user types
  //    -> composer redraws), then correct" — the fix must force that repaint on
  //    focus-regain (reconcile from daemon) so the user never sees the transient.
  assert.match(h.lineText(term, composerRow - 1) || '', /the codex composer/, 'composer restored by next CUP frame');
  assert.match(h.lineText(term, footerRow - 1) || '', /gpt-5\.5 medium/, 'footer restored by next CUP frame');
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
