// Locks the LOAD-BEARING scroll signal the working-session-cluster follow fix
// depends on (campaign-xterm-dealbreakers / finding-working-state-row-overlap).
//
// The fix must distinguish a PASSIVE strand (viewport fell behind the live
// bottom; should re-follow) from a USER SCROLL-UP (must NOT yank). The signal,
// verified here against the EXACT vendored xterm.js:
//   * writing output NEVER decreases the viewport ydisp — at the bottom it
//     auto-follows (ydisp INCREASES toward baseY); when scrolled up it leaves
//     ydisp UNCHANGED while baseY grows.
//   * a user scroll-up is the ONLY thing that DECREASES ydisp.
// Therefore: `ydisp decreased && !programmatic` <=> user scrolled up; a
// `viewportY < baseY` with ydisp unchanged is a passive strand to re-follow.
const test = require('node:test');
const assert = require('node:assert');
const { createTerminal, write } = require('./harness');

const vp = (term) => Math.max(0, Number(term.buffer.active.viewportY || 0));
const by = (term) => Math.max(0, Number(term.buffer.active.baseY || 0));

// xterm.js 6 reworked the viewport/scrollbar (VS Code integration): a programmatic
// USER scroll (`term.scrollLines(-n)`/`scrollToTop`) is now layout- and
// canvas-measurement-dependent and a COMPLETE no-op under jsdom (verified: even the
// internal `_bufferService.buffer.ydisp` does not move — no getContext, no layout).
// The OUTPUT-driven signals below (auto-follow on burst, reflow clamp, reset) DO
// work headless and stay locked here. The user-scroll-up contract these three guard
// is now verified LIVE on the WebGL renderer (deploy acceptance: scroll up in a
// session → stays put; output while scrolled up → never yanks). Skipped, not deleted,
// so the intent + the live-verify obligation stay visible.
const JSDOM_SCROLL_SKIP =
  'xterm6 viewport refactor: programmatic user-scroll is layout/canvas-dependent and a no-op under jsdom; the scroll-up signal contract is verified LIVE on the WebGL renderer.';

test('following at bottom + burst write auto-follows (ydisp only increases)', async () => {
  const term = createTerminal({ cols: 80, rows: 24, scrollback: 1000 });
  for (let i = 0; i < 40; i++) await write(term, `line ${i}\r\n`);
  const events = [];
  term.onScroll((y) => events.push(y));
  let burst = '';
  for (let i = 0; i < 30; i++) burst += `burst ${i}\r\n`;
  await write(term, burst);
  // Auto-followed: viewport tracked baseY to the bottom.
  assert.strictEqual(vp(term), by(term), 'viewport should be at the live bottom');
  // Every scroll event moved toward the bottom — no decrease on output.
  for (let i = 1; i < events.length; i++) {
    assert.ok(events[i] >= events[i - 1], `onScroll must be monotonic non-decreasing on output (got ${events})`);
  }
});

test('scrolled-up + subsequent write leaves ydisp UNCHANGED (passive strand signature)', { skip: JSDOM_SCROLL_SKIP }, async () => {
  const term = createTerminal({ cols: 80, rows: 24, scrollback: 1000 });
  for (let i = 0; i < 60; i++) await write(term, `line ${i}\r\n`);
  term.scrollLines(-20);
  const ydispAfterScroll = vp(term);
  const events = [];
  term.onScroll((y) => events.push(y));
  let burst = '';
  for (let i = 0; i < 10; i++) burst += `more ${i}\r\n`;
  await write(term, burst);
  // The hallmark: baseY grew, ydisp did NOT move on output.
  assert.ok(by(term) > ydispAfterScroll, 'baseY should grow with output');
  assert.strictEqual(vp(term), ydispAfterScroll, 'ydisp must stay put while scrolled up');
  for (const y of events) {
    assert.strictEqual(y, ydispAfterScroll, `output onScroll must report the unchanged ydisp (got ${events})`);
  }
});

test('a user scroll-up is the only thing that DECREASES ydisp', { skip: JSDOM_SCROLL_SKIP }, async () => {
  const term = createTerminal({ cols: 80, rows: 24, scrollback: 1000 });
  for (let i = 0; i < 60; i++) await write(term, `line ${i}\r\n`);
  const before = vp(term);
  const events = [];
  term.onScroll((y) => events.push(y));
  term.scrollLines(-15); // user scrolls up
  assert.ok(vp(term) < before, 'scroll-up must decrease ydisp');
  assert.ok(events.length >= 1 && events[events.length - 1] < before,
    'scroll-up must fire onScroll with a decreased ydisp');
});

// bg→fg stuck-at-top hardening (sum-test run #3): programmatic buffer/layout
// mutations that are NOT routed through forceXtermViewportY can ALSO decrease
// ydisp — but they decrease baseY with it, which a user gesture never does.
// These two tests lock that signature on the EXACT vendored xterm.js so the
// detector's baseY-non-decrease discriminator (scroll_mode.rs
// user_scroll_up_detected) is grounded in measured behavior, not theory.

test('row-growth resize decreases ydisp AND baseY together (reflow clamp is not a user scroll-up)', async () => {
  const term = createTerminal({ cols: 80, rows: 24, scrollback: 1000 });
  for (let i = 0; i < 60; i++) await write(term, `line ${i}\r\n`);
  const ydispBefore = vp(term);
  const baseBefore = by(term);
  assert.ok(ydispBefore > 0 && ydispBefore === baseBefore, 'precondition: following at bottom with scrollback');
  const events = [];
  term.onScroll((y) => events.push(y));
  term.resize(80, 48); // rows grow (the focus-regain re-fit shape)
  const ydispAfter = vp(term);
  const baseAfter = by(term);
  assert.ok(baseAfter < baseBefore, 'row growth must shrink baseY');
  assert.ok(ydispAfter < ydispBefore, 'row growth must clamp ydisp down with baseY');
  // The discriminator: ydisp decreased but baseY decreased too -> NOT a user scroll-up.
  assert.ok(!(ydispAfter < ydispBefore && baseAfter >= baseBefore),
    'a reflow clamp must be rejected by the baseY-non-decrease discriminator');
});

test('term.reset() drops ydisp and baseY to 0 together (reseed is not a user scroll-up)', async () => {
  const term = createTerminal({ cols: 80, rows: 24, scrollback: 1000 });
  for (let i = 0; i < 60; i++) await write(term, `line ${i}\r\n`);
  const ydispBefore = vp(term);
  const baseBefore = by(term);
  assert.ok(ydispBefore > 0, 'precondition: scrollback accumulated');
  term.reset();
  const ydispAfter = vp(term);
  const baseAfter = by(term);
  assert.strictEqual(ydispAfter, 0, 'reset must drop ydisp to 0');
  assert.strictEqual(baseAfter, 0, 'reset must drop baseY to 0');
  assert.ok(!(ydispAfter < ydispBefore && baseAfter >= baseBefore),
    'a reseed must be rejected by the baseY-non-decrease discriminator');
});

// "Codex select kicks me to the bottom" (user report 2026-06-18): on a WORKING
// codex the user scrolls up to select a word and gets yanked to the live bottom.
// Codex's working pattern is NOT plain newline output — committed messages scroll
// in (newline-driven) WHILE a 3-row bottom live region (composer/status) is
// repainted in place via absolute CUP (`\x1b[{row};1H\x1b[K…`, no newline), exactly
// the mock-tui `codex-inline` scenario. This locks whether that mixed pattern keeps
// the scroll-up signal honest: output (committed lines + CUP repaints) must NEVER
// decrease ydisp while scrolled up. If it holds, the yank is NOT xterm/codex — it is
// yggterm's follow wiring (a force-follow on click/redraw racing the UserScrollback
// latch), which the live viewport_force_log must pin.
test('codex-inline working frames while scrolled up never decrease ydisp', { skip: JSDOM_SCROLL_SKIP }, async () => {
  const screenRows = 24;
  const term = createTerminal({ cols: 80, rows: screenRows, scrollback: 2000 });
  // Committed conversation lines scroll naturally (newline-driven), as codex emits.
  for (let i = 0; i < 80; i++) await write(term, `CODEX_MSG_${i} committed conversation line\r\n`);
  // User scrolls up to read/select a word in the scrollback.
  term.scrollLines(-20);
  const ydispAfterScroll = vp(term);
  assert.ok(ydispAfterScroll < by(term), 'precondition: user is scrolled up above the live bottom');
  const events = [];
  term.onScroll((y) => events.push(y));
  // codex working: interleave NEW committed messages (grow baseY) with the bottom
  // 3-row live-region repaint via absolute CUP (no newline) — its idle/working churn.
  const composerTop = screenRows - 2; // 1-based rows 22,23,24
  for (let frame = 0; frame < 12; frame++) {
    await write(term, `CODEX_MSG_working_${frame} streaming token\r\n`);
    let repaint = '';
    ['> ', 'model ', 'esc '].forEach((label, offset) => {
      const row = composerTop + offset;
      repaint += `\x1b[${row};1H\x1b[K${label}COMPOSER_FRAME_${frame}`;
    });
    await write(term, repaint);
  }
  // The invariant: even with CUP bottom-region repaints mixed into the stream, the
  // user's scroll position (ydisp) must not be dragged down by output.
  assert.ok(by(term) > ydispAfterScroll, 'baseY should grow as working messages arrive');
  assert.strictEqual(vp(term), ydispAfterScroll,
    `ydisp must stay put while scrolled up through codex working frames (got ${vp(term)}, want ${ydispAfterScroll})`);
  for (const y of events) {
    assert.ok(y >= ydispAfterScroll,
      `no codex working frame may decrease ydisp below the user's spot (events=${events})`);
  }
});
