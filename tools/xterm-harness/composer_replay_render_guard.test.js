// Content-clip-on-reveal repro (campaign issue #1).
// LIVE FINDING (2026-06-09, a giant remote codex session): the codex composer
// (gray bar + "›" prompt + "gpt-5.5 medium · ~/git/project" footer) is MISSING from
// the client viewport while the daemon's authoritative screen HAS it — and it
// survives a clean fresh GUI mount + daemon retained-replay (so it is NOT a stale
// client-snapshot reveal bug). The gray composer BAR renders but the "›" + dim
// placeholder TEXT does not.
//
// This test splits the two remaining hypotheses by replaying the codex composer
// the way the GUI replay does (reset + writeSync of the daemon screen, absolute
// CUP into the bottom rows, after a tall scrollback):
//   (B) xterm DROPS the absolute-CUP composer text on render  -> this test FAILS
//   (A) xterm renders it fine                                 -> this test PASSES,
//       which means the daemon retained-replay PAYLOAD lacks the composer text
//       (daemon-side), not an xterm render bug.
const { test } = require('node:test');
const assert = require('node:assert');
const h = require('./harness');

const GRAY = '\x1b[39;48;2;64;67;75m';

// Faithful reconstruction of a real codex composer region (daemon terminal_lines tail):
//   row 60: gray bar      \x1b[60;1H<gray> \x1b[K
//   row 61: prompt        \x1b[1m›\x1b[22m \x1b[2mSummarize recent commits\x1b[22m\x1b[K
//   row 63: footer        \x1b[63;3H...gpt-5.5 medium · ~/git/project
function composerFrame() {
  return (
    `\x1b[60;1H${GRAY} \x1b[K` +
    `\x1b[61;1H\x1b[1m›\x1b[22m \x1b[2mSummarize recent commits\x1b[22m\x1b[K` +
    `\x1b[63;3H\x1b[38;2;246;226;183;49mgpt-5.5 medium\x1b[39;2m · \x1b[38;2;171;223;167;22m~/git/project`
  );
}

// The RENDER consequence of the bug: a replay buffer that ENDS mid-2026-frame
// (codex's repaint cleared the composer row to gray but the "›"+placeholder and
// the ESU never arrive — exactly what flush_due flushes after the 250ms cap when
// the ESU never comes) paints an EMPTY gray composer bar = the faithful live
// symptom. The fix must guarantee the replay payload never ends mid-frame (or
// reconciles the final frame from the daemon's complete vt100 screen).
test('TORN replay (ends mid-2026-frame after the gray-bar clear) loses the "›" composer text', async () => {
  const term = h.createTerminal({ cols: 167, rows: 63, scrollback: 4000 });
  let payload = '\x1b[2J\x1b[H';
  for (let i = 1; i <= 120; i++) payload += `transcript line ${i} content\r\n`;
  // codex opens a synchronized frame, clears the composer row to gray... CUT here
  // (no "›" text, no ESU) — the torn flush.
  payload += '\x1b[?2026h' + `\x1b[60;1H${GRAY} \x1b[K`;
  await h.write(term, payload);
  const by = h.baseY(term);
  let joined = '';
  for (let r = by; r < by + 63; r++) joined += (h.lineText(term, r) || '') + '\n';
  assert.ok(!/›/.test(joined), 'TORN frame must NOT have rendered the "›" prompt (reproduces the empty composer)');
  assert.ok(!/Summarize recent commits/.test(joined), 'TORN frame must NOT have rendered the placeholder');
});

test('codex composer replay: absolute-CUP "›" prompt + placeholder render after tall scrollback', async () => {
  const term = h.createTerminal({ cols: 167, rows: 63, scrollback: 4000 });
  let payload = '\x1b[2J\x1b[H';
  // A transcript taller than the screen so baseY grows (like the real 1572-line session).
  for (let i = 1; i <= 120; i++) payload += `transcript line ${i} content\r\n`;
  payload += composerFrame();
  await h.write(term, payload);

  const by = h.baseY(term);
  // Locate the composer row by content rather than arithmetic (robust to scroll math).
  let promptRow = -1;
  for (let r = by; r < by + 63; r++) {
    const t = h.lineText(term, r) || '';
    if (t.includes('›') || /Summarize recent commits/.test(t)) { promptRow = r; break; }
  }
  const promptText = promptRow >= 0 ? (h.lineText(term, promptRow) || '') : '';
  // Dump the bottom 6 visible rows for diagnosis on failure.
  const tail = [];
  for (let r = by + 57; r < by + 63; r++) tail.push(`row${r}=${JSON.stringify(h.lineText(term, r) || '')}`);

  assert.ok(promptRow >= 0, `composer prompt row not found. baseY=${by}\n${tail.join('\n')}`);
  assert.match(promptText, /›/, `"›" must render. ${promptText}`);
  assert.match(promptText, /Summarize recent commits/, `placeholder must render. ${promptText}`);
});
