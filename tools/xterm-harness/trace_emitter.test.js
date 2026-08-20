// Behavioral guard for the trace-plane emitter that the GUI actually ships.
//
// The subject is `crates/yggterm-shell/src/shell/trace_emitter.js`, loaded here
// as the SAME bytes the Rust side `include_str!`s into the terminal script — no
// transcription, so a rule asserted here is a rule that runs in the webview.
//
// What is being guarded is not the grammar (that is `trace_contract.rs`) but
// the three properties that decide whether the instrument perturbs the thread
// it measures: emit does no I/O, the drain runs off the hot path, and the timer
// suspends itself when there is nothing to say.

const { test } = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const EMITTER_JS = path.resolve(
  __dirname, '..', '..',
  'crates', 'yggterm-shell', 'src', 'shell', 'trace_emitter.js',
);

// A fake timer plane, so "did it schedule a wakeup" is an assertion rather than
// a wait. Real timers would make the self-suspension property untestable: an
// absent wakeup and a wakeup that has not fired yet look identical from a test
// that can only sleep.
function makeSandbox(options = {}) {
  const sent = [];
  const timers = new Map();
  let nextTimerId = 1;
  let clock = 1_000_000;

  const sandbox = {
    sentBatches: sent,
    sendTerminalEvent: (payload) => {
      if (options.senderThrows) { throw new Error('channel closed'); }
      sent.push(payload);
    },
    setTimeout: (fn, ms) => {
      const id = nextTimerId++;
      timers.set(id, { fn, at: clock + ms });
      return id;
    },
    clearTimeout: (id) => { timers.delete(id); },
    Date: { now: () => clock },
    Math,
    Object,
  };
  sandbox.window = sandbox;
  sandbox.performance = { now: () => clock };

  const ctx = vm.createContext(sandbox);
  vm.runInContext(fs.readFileSync(EMITTER_JS, 'utf8'), ctx, { filename: 'trace_emitter.js' });

  return {
    ctx,
    sandbox,
    sent,
    pendingTimers: () => timers.size,
    advance(ms) {
      clock += ms;
      const due = [...timers.entries()].filter(([, t]) => t.at <= clock);
      for (const [id, t] of due) { timers.delete(id); t.fn(); }
    },
    tick(ms) { clock += ms; },
    trace: () => sandbox.window.__yggtermTrace,
  };
}

test('emit performs no send of its own — the drain is a separate task', () => {
  // The whole design rests on this. If emit ever sends inline, every probe on
  // the write path costs an IPC hop plus a synchronous file append on the UI
  // thread, which is the freeze this channel exists to avoid.
  const h = makeSandbox();
  for (let i = 0; i < 10; i++) {
    h.trace().emit({ category: 'xterm_write', name: 'enqueue', payload: { i } });
  }
  assert.strictEqual(h.sent.length, 0, 'emit must not send');
  assert.strictEqual(h.trace().stats().queued, 10);
});

test('an idle emitter schedules no wakeups, and one flush drains the whole ring', () => {
  const h = makeSandbox();
  assert.strictEqual(h.pendingTimers(), 0, 'nothing queued means no timer at all');

  h.trace().emit({ category: 'xterm_write', name: 'enqueue' });
  h.trace().emit({ category: 'xterm_write', name: 'enqueue' });
  assert.strictEqual(h.pendingTimers(), 1, 'one timer for the batch, not one per record');

  h.advance(250);
  assert.strictEqual(h.sent.length, 1, 'two records, ONE batch');
  assert.strictEqual(h.sent[0].kind, 'trace');
  assert.strictEqual(h.sent[0].records.length, 2);
  assert.strictEqual(h.pendingTimers(), 0, 'the timer suspends itself once drained');
});

test('a burst is brought forward instead of being averaged into the interval', () => {
  const h = makeSandbox();
  for (let i = 0; i < 64; i++) {
    h.trace().emit({ category: 'xterm_write', name: 'enqueue', payload: { i } });
  }
  // The high-water reschedule fires on the next task, not a quarter second
  // later — a burst should reach the plane while it is still a burst.
  h.advance(0);
  assert.strictEqual(h.sent.length, 1);
  assert.strictEqual(h.sent[0].records.length, 64);
});

test('the ring is bounded, drops the OLDEST, and the drop count rides the next record out', () => {
  // ⛔ The property that matters is the second half. Under sustained pressure a
  // separately-reported drop count is itself subject to the pressure, so the
  // one number that proves the stream is incomplete is the one most likely to
  // go missing. Carrying it ON a record makes that impossible.
  const h = makeSandbox();
  for (let i = 0; i < 600; i++) {
    h.trace().emit({ category: 'xterm_write', name: 'enqueue', payload: { i } });
  }
  assert.strictEqual(h.trace().stats().queued, 512, 'ring must be bounded');

  h.advance(0);
  const records = h.sent[0].records;
  assert.strictEqual(records.length, 512);
  // Oldest dropped: the survivors end at the newest record emitted.
  assert.strictEqual(records[records.length - 1].payload.i, 599);
  assert.strictEqual(records[0].payload.i, 88);

  // 600 emitted, 512 retained: the 88 dropped are accounted for ON records that
  // got out, not lost with the records that did not. The arrears attach to the
  // record whose own arrival caused the eviction, so the count is not merely
  // conserved — it is placed at the moment the loss happened, which is what
  // makes a drop burst distinguishable from a steady trickle of the same total.
  const dropCarriers = records.filter((r) => typeof r.dropped === 'number');
  const droppedTotal = dropCarriers.reduce((sum, r) => sum + r.dropped, 0);
  assert.strictEqual(droppedTotal, 88, 'every dropped record must be accounted for');
  assert.ok(dropCarriers.length > 0, 'the arrears must reach the plane on a record');
});

test('every record is stamped at EMIT time, never at drain time', () => {
  // ⛔ This is what makes a deferred flush safe. Stamping on arrival would shift
  // foreign rows later by however long the UI thread was busy — i.e. most wrong
  // during exactly the stalls the probes exist to explain, producing a timeline
  // where the probe fires after the fault it measured.
  const h = makeSandbox();
  h.trace().emit({ category: 'xterm_write', name: 'enqueue' });
  const emittedAt = h.sandbox.Date.now();
  h.tick(5_000);
  h.advance(0);
  assert.strictEqual(h.sent[0].records[0].ts_ms, emittedAt);
});

test('seq is monotonic across records that share a millisecond', () => {
  // A corrupted repaint is a question about what interleaved INSIDE one
  // millisecond, which `ts_ms` cannot answer at its resolution.
  const h = makeSandbox();
  for (let i = 0; i < 5; i++) {
    h.trace().emit({ category: 'xterm_render', name: 'frame' });
  }
  h.advance(250);
  // `Array.from` rather than the mapped array itself: the records come from the
  // vm realm, so a deep-equal against a host array fails on prototype identity
  // even when every value matches — an assertion failure about realms wearing
  // the costume of an assertion failure about sequence numbers.
  const seqs = Array.from(h.sent[0].records, (r) => r.seq);
  assert.deepStrictEqual(seqs, [1, 2, 3, 4, 5]);
  assert.strictEqual(new Set(h.sent[0].records.map((r) => r.ts_ms)).size, 1,
    'the test only proves anything if the timestamps really do collide');
});

test('a span is wall-clocked and never claims the cpu clock', () => {
  // There is no per-thread CPU clock in a webview content process, so a `cpu`
  // duration from here could only be wall time wearing the wrong unit. The Rust
  // boundary refuses one; the emitter must not be able to produce one.
  const h = makeSandbox();
  const span = h.trace().span('xterm_write', 'flush', { host: 'terminal-a' });
  h.tick(12);
  span.finish({ chars: 4096 });
  h.advance(250);

  const record = h.sent[0].records[0];
  assert.strictEqual(record.clock, 'wall');
  assert.strictEqual(record.kind, 'span');
  assert.strictEqual(record.duration_ms, 12);
  assert.strictEqual(record.payload.host, 'terminal-a');
  assert.strictEqual(record.payload.chars, 4096);
});

test('a windowed aggregate is tagged as one, so no reader can correlate on its timestamp', () => {
  const h = makeSandbox();
  h.trace().window('xterm_render', 'frame_window', { frames: 42, window_ms: 1000 });
  h.advance(250);
  const record = h.sent[0].records[0];
  assert.strictEqual(record.kind, 'window');
  assert.strictEqual(record.duration_ms, undefined,
    'a window carries values, not a duration on some clock');
});

test('records survive a period with no channel and keep their original timestamps', () => {
  // A terminal can unmount between an event and its flush. Dropping the ring
  // then would delete evidence for a reason that has nothing to do with the
  // evidence; re-stamping it would move the event to when a channel came back.
  // The channel must be dead from the START: `registerSender` captures the
  // function, so reassigning the binding afterwards changes nothing — and the
  // emitter's fallback to an older channel is correct behaviour, not the thing
  // under test here.
  const h = makeSandbox({ senderThrows: true });

  h.trace().emit({ category: 'xterm_write', name: 'enqueue', payload: { i: 1 } });
  const emittedAt = h.sandbox.Date.now();
  h.advance(250);
  assert.strictEqual(h.sent.length, 0, 'nothing left through a dead channel');
  assert.strictEqual(h.trace().stats().queued, 1, 'and nothing was thrown away');

  // A fresh channel mounts and drains the backlog.
  h.sandbox.window.__yggtermTrace.registerSender((payload) => { h.sent.push(payload); });
  h.tick(3_000);
  h.trace().flush();
  assert.strictEqual(h.sent.length, 1);
  assert.strictEqual(h.sent[0].records[0].ts_ms, emittedAt, 'the late drain reports when it HAPPENED');
});

test('a malformed emit is ignored rather than throwing into the caller', () => {
  // Every call site is on a render or write path. An emitter that can throw
  // turns a diagnostic into an outage.
  const h = makeSandbox();
  assert.doesNotThrow(() => {
    h.trace().emit(null);
    h.trace().emit({});
    h.trace().emit({ category: 'only_category' });
    h.trace().emit({ name: 'only_name' });
  });
  assert.strictEqual(h.trace().stats().queued, 0);
});

// ── attach-stream capture ─────────────────────────────────────────────────
// The falsifier for the ghost-frame entry: does the reseed hand the canvas
// bytes with the SGR colour already gone, or does the canvas fail to apply
// attributes that were present? These guard both halves of the answer — that
// the census can tell those apart, and that finding out costs no content.

test('the census answers the ghost-frame question without recording the screen', () => {
  const h = makeSandbox();
  const ESC = '\x1b';
  h.trace().armStreamCapture('terminal-a', 'replay:snapshot');
  h.trace().captureStream(
    'terminal-a', 'reseed',
    `${ESC}[38;5;42mledger reconciled${ESC}[0m\r\n${ESC}[31mtotals differ${ESC}[0m\r\n`,
  );
  h.advance(250);

  const record = h.sent[0].records[0];
  assert.strictEqual(record.payload.stage, 'reseed');
  // (B) the bytes DID carry colour — so a colourless canvas would be an
  // apply-side fault, not a stripping one.
  assert.strictEqual(record.payload.sgr_colour, 2);
  assert.strictEqual(record.payload.sgr_total, 4);
  assert.strictEqual(record.payload.sgr_reset, 2);
});

test('a stripped reseed is distinguishable from a coloured one by the census alone', () => {
  // (A) the other answer. If this reads zero on a live capture, the stripping
  // happened before the canvas — and no sample needs to be read to know it.
  const h = makeSandbox();
  h.trace().armStreamCapture('terminal-a', 'replay:snapshot');
  h.trace().captureStream('terminal-a', 'reseed', 'ledger reconciled\r\ntotals differ\r\n');
  h.advance(250);
  const record = h.sent[0].records[0];
  assert.strictEqual(record.payload.sgr_colour, 0);
  assert.strictEqual(record.payload.sgr_total, 0);
});

test('⛔ no printable content survives redaction, and the escapes do', () => {
  // The whole safety argument in one assertion. The trace file is read and
  // quoted by agents; the screen being sampled is whatever the user was working
  // on. If a secret can reach the sample, this instrument is a worse problem
  // than the bug it exists to solve.
  const h = makeSandbox();
  const ESC = '\x1b';
  const secret = 'correct-horse-battery-staple';
  const census = h.trace().redactPreservingControls(`${ESC}[32m${secret}${ESC}[0m`);

  assert.ok(!census.sample.includes(secret), 'the content must not be in the sample');
  assert.ok(!census.sample.includes('horse'), 'not even a fragment of it');
  assert.match(census.sample, /·28·/, 'a run of text becomes its length');
  assert.match(census.sample, /\\e\[32m/, 'the colour sequence is preserved verbatim');
  assert.strictEqual(census.sgr_colour, 1);
});

test('⛔ an OSC payload is reduced to opcode and length — never copied', () => {
  // OSC is the one escape family that IS content: window titles, and at OSC 52
  // the clipboard. Copying escapes verbatim is right for CSI and would be a
  // clipboard exfiltration here.
  const h = makeSandbox();
  const ESC = '\x1b';
  const BEL = '\x07';
  const clipboard = 'c;bGVkZ2VyLXNlY3JldC10b2tlbg==';
  const census = h.trace().redactPreservingControls(`${ESC}]52;${clipboard}${BEL}done`);

  assert.ok(!census.sample.includes('bGVkZ2Vy'), 'the OSC 52 payload must not survive');
  assert.ok(!census.sample.includes(clipboard));
  assert.match(census.sample, /\\e\]52;<\d+>/, 'opcode and length only');
  assert.strictEqual(census.osc_count, 1);

  const title = `${ESC}]0;a private window title${BEL}`;
  const titleCensus = h.trace().redactPreservingControls(title);
  assert.ok(!titleCensus.sample.includes('private'), 'a window title is content too');
});

test('capture is bounded per arm and per host, so a re-attach storm cannot flood', () => {
  const h = makeSandbox();
  h.trace().armStreamCapture('terminal-a', 'mount');
  // One oversized chunk exhausts the arm; the next is silent until re-armed.
  h.trace().captureStream('terminal-a', 'reseed', 'x'.repeat(9000));
  h.trace().captureStream('terminal-a', 'reseed', 'y'.repeat(100));
  h.advance(250);
  assert.strictEqual(h.sent[0].records.length, 1, 'the arm budget must bind');

  // And the arm count binds too: past the cap, arming stops re-opening it.
  for (let i = 0; i < 40; i++) {
    h.trace().armStreamCapture('terminal-a', 'storm');
    h.trace().captureStream('terminal-a', 'reseed', 'z'.repeat(9000));
  }
  h.advance(250);
  const total = h.sent.reduce((sum, batch) => sum + batch.records.length, 0);
  assert.ok(total <= 17, `arms per host must cap the capture, got ${total}`);
});

test('an unarmed host captures nothing at all', () => {
  const h = makeSandbox();
  h.trace().captureStream('terminal-never-armed', 'live_stream', 'anything');
  h.advance(250);
  assert.strictEqual(h.sent.length, 0);
});

test('a truncated sample says so rather than looking complete', () => {
  // A sample cut at the cap and a stream that really ended are the same string.
  // Only the flag tells them apart, and reading a truncated sample as complete
  // is how "there were no more colour sequences" gets concluded from a window
  // that simply stopped.
  const h = makeSandbox();
  const census = h.trace().redactPreservingControls('\x1b[m'.repeat(3000));
  assert.strictEqual(census.truncated, true);
  assert.ok(census.sample.length <= 2048 + 16);
});
