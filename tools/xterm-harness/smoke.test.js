const { test } = require('node:test');
const assert = require('node:assert');
const h = require('./harness');

test('vendored xterm.js loads and exposes Terminal', () => {
  const { Terminal } = h.loadXterm();
  assert.strictEqual(typeof Terminal, 'function', 'Terminal constructor must load');
});

test('write reaches the buffer', async () => {
  const term = h.createTerminal({ cols: 80, rows: 24 });
  await h.write(term, 'hello world');
  assert.match(h.lineText(term, 0) || '', /hello world/);
});
