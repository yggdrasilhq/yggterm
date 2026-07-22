// Pins the ONE upstream xterm.js behavior that made every "wipe the host, then
// term.open() to rebuild it" recovery in shell.rs a silent no-op — and therefore
// made a blank viewport unrecoverable without a remount.
//
// `Terminal.open(parent)` early-returns as soon as `this.element` exists:
//
//     open(e) { if (!e) throw ...;
//       if (e.isConnected || this._logService.debug(...),
//           this.element?.ownerDocument.defaultView && this._coreBrowserService)
//         return void (...)        // <- no appendChild(e)
//
// This guard exists so a future xterm bump that changes it is caught here rather
// than on the user's screen. See docs/xterm-bugs.md and the surface owner
// `attachTerminalSurfaceToHost` in crates/yggterm-shell/src/shell.rs.

const { test } = require('node:test');
const assert = require('node:assert');
const h = require('./harness');

function mountedTerminal() {
  const { Terminal } = h.loadXterm();
  const term = new Terminal({ cols: 80, rows: 24, allowProposedApi: true });
  const host = global.document.createElement('div');
  global.document.body.appendChild(host);
  term.open(host);
  return { term, host };
}

test('term.open builds a full surface on a fresh host', () => {
  const { term, host } = mountedTerminal();
  assert.ok(term.element, 'the first open must create the .xterm root');
  assert.strictEqual(term.element.parentElement, host, 'the root belongs to the host');
  assert.ok(host.querySelector('.xterm-screen'), 'a healthy host carries the screen');
});

test('re-opening after a host wipe rebuilds NOTHING — the host stays empty', () => {
  const { term, host } = mountedTerminal();
  const firstRoot = term.element;

  host.innerHTML = '';
  assert.strictEqual(host.childElementCount, 0, 'the wipe cleared the host');

  term.open(host);

  // The whole point: open() returned without re-parenting anything.
  assert.strictEqual(
    host.childElementCount,
    0,
    'term.open() must NOT be trusted to rebuild a wiped host',
  );
  assert.strictEqual(term.element, firstRoot, 'open() did not build a new root either');
  assert.strictEqual(term.element.isConnected, false, 'the surface is stranded outside the DOM');
  assert.strictEqual(host.querySelector('.xterm-screen'), null, 'no screen came back');
});

test('moving term.element back is what actually restores the surface', () => {
  const { term, host } = mountedTerminal();
  const root = term.element;

  host.innerHTML = '';
  // This is `attachTerminalSurfaceToHost`'s reattach branch, reduced.
  host.appendChild(term.element);

  assert.strictEqual(term.element, root, 'the same root is reused');
  assert.strictEqual(term.element.parentElement, host, 'the surface is back in the host');
  assert.ok(host.querySelector('.xterm-screen'), 'the screen travelled with the root');
  assert.ok(term.element.isConnected, 'and it is connected again');
});

test('a wiped host can also be repaired into a DIFFERENT host element', () => {
  // The rebind path runs when Dioxus replaces the host node, so the destination
  // is frequently not the element the terminal was opened into.
  const { term } = mountedTerminal();
  const replacement = global.document.createElement('div');
  global.document.body.appendChild(replacement);

  replacement.appendChild(term.element);

  assert.strictEqual(term.element.parentElement, replacement, 'appendChild MOVES the root');
  assert.ok(replacement.querySelector('.xterm-screen'), 'the new host has a real surface');
});
