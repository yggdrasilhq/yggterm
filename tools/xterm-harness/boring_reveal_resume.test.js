// Locks the safety properties of the BORING retained reveal (cursor-resume
// append; spec-boring-session-loads): instead of reset+replaying the full
// retained stream into an already-painted buffer (the blink-blink shadow),
// the bridge resumes reads from the last consumed cursor and APPENDS only the
// missed delta. That is only sound if the vendored xterm.js is a pure
// function of the byte stream regardless of how it is chunked:
//   * stream-split invariance — feeding bytes 0..N in one write produces the
//     SAME buffer as feeding 0..k, pausing, then k..N (any k, even
//     mid-escape-sequence: parser state persists across writes).
//   * the reset+full-replay path necessarily passes through a CLEARED
//     intermediate frame (the user-visible blink); the append path never
//     shrinks the painted buffer.
const test = require('node:test');
const assert = require('node:assert');
const { createTerminal, write, bufferText, baseY } = require('./harness');

// A representative stream: scrolling shell output, then a codex-like in-place
// TUI repaint (cursor addressing + SGR + erase-line), then more output. Chunk
// boundaries from the daemon ring are arbitrary, so the payload deliberately
// mixes multi-byte escape sequences with plain text.
function buildStream() {
  let s = '';
  for (let i = 0; i < 60; i++) s += `line ${i} from the shell\r\n`;
  // In-place TUI frame: home, paint a "composer", colored footer.
  s += '\x1b[H\x1b[2K\x1b[38;2;200;200;200m› type your prompt here\x1b[0m\r\n';
  s += '\x1b[2;1H\x1b[48;2;60;60;60m  composer row with bg  \x1b[0m\r\n';
  s += '\x1b[3;1H\x1b[31mesc to interrupt\x1b[0m';
  for (let i = 0; i < 8; i++) s += `\r\ntail output ${i}`;
  return s;
}

function snapshot(term) {
  return {
    text: bufferText(term),
    baseY: baseY(term),
    cursorX: term.buffer.active.cursorX,
    cursorY: term.buffer.active.cursorY,
  };
}

test('stream-split invariance: one write == split writes at arbitrary offsets (incl. mid-escape)', async () => {
  const stream = buildStream();
  const reference = createTerminal({ cols: 80, rows: 24, scrollback: 1000 });
  await write(reference, stream);
  const want = snapshot(reference);

  // Split points chosen to land inside escape sequences and mid-text. The
  // first \x1b[H above sits at a known offset region; rather than hardcode,
  // probe several offsets including ones adjacent to every ESC in the stream.
  const splits = [1, 7, 100, Math.floor(stream.length / 2), stream.length - 3];
  for (let i = 0; i < stream.length; i++) {
    if (stream[i] === '\x1b') {
      splits.push(i + 1); // immediately after ESC = mid-sequence
      splits.push(i + 2);
    }
  }
  for (const k of splits) {
    if (k <= 0 || k >= stream.length) continue;
    const term = createTerminal({ cols: 80, rows: 24, scrollback: 1000 });
    await write(term, stream.slice(0, k));
    // The pause: a backgrounded retained host stops reading here.
    await write(term, stream.slice(k));
    assert.deepStrictEqual(
      snapshot(term),
      want,
      `split at ${k} must reproduce the un-split buffer exactly`
    );
  }
});

test('resume-append after a long pause == never-detached stream (the boring reveal)', async () => {
  const stream = buildStream();
  // Consumed-before-background prefix ends on a chunk boundary mid-stream.
  const k = Math.floor(stream.length * 0.7);
  const neverDetached = createTerminal({ cols: 80, rows: 24, scrollback: 1000 });
  await write(neverDetached, stream);

  const revealed = createTerminal({ cols: 80, rows: 24, scrollback: 1000 });
  await write(revealed, stream.slice(0, k));
  const painted = snapshot(revealed);
  assert.ok(painted.text.length > 0, 'retained buffer must hold content before the reveal');
  // Reveal: append ONLY the missed delta — no reset, no full replay.
  await write(revealed, stream.slice(k));
  assert.deepStrictEqual(snapshot(revealed), snapshot(neverDetached));
});

test('reset+full-replay paints a cleared intermediate frame; append never shrinks the buffer', async () => {
  const stream = buildStream();
  const k = Math.floor(stream.length * 0.7);

  // The 2.8.x retained-replay writer path (writePayloadIntoEntry): reset() +
  // clear() + "\x1bc\x1b[2J\x1b[3J\x1b[H" + full payload. Between the reset
  // and the replay landing, the buffer IS empty — that frame is the blink the
  // user sees on every reveal.
  const resetPath = createTerminal({ cols: 80, rows: 24, scrollback: 1000 });
  await write(resetPath, stream.slice(0, k));
  assert.ok(bufferText(resetPath).includes('line 30 from the shell'));
  resetPath.reset();
  resetPath.clear();
  const blinkFrame = bufferText(resetPath);
  assert.strictEqual(blinkFrame, '', 'reset+clear exposes an EMPTY intermediate frame (the blink)');
  await write(resetPath, '\x1bc\x1b[2J\x1b[3J\x1b[H' + stream);
  assert.ok(bufferText(resetPath).includes('esc to interrupt'), 'replay eventually repaints');
  // And \x1b[3J wiped scrollback: the replayed buffer holds ONLY the replayed
  // payload — any client-side history beyond the daemon ring would be gone
  // (the vacuum class). Recorded here as the cost of the reset path.

  // The append path: painted content only ever grows.
  const appendPath = createTerminal({ cols: 80, rows: 24, scrollback: 1000 });
  await write(appendPath, stream.slice(0, k));
  const beforeLen = bufferText(appendPath).length;
  const beforeBaseY = baseY(appendPath);
  await write(appendPath, stream.slice(k));
  assert.ok(baseY(appendPath) >= beforeBaseY, 'append must not collapse scrollback');
  assert.ok(
    bufferText(appendPath).length >= beforeLen,
    'append must not shrink the painted buffer'
  );
});
