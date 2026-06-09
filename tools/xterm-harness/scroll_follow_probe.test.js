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

test('scrolled-up + subsequent write leaves ydisp UNCHANGED (passive strand signature)', async () => {
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

test('a user scroll-up is the only thing that DECREASES ydisp', async () => {
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
