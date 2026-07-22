// Settles what "species B" actually is.
//
// husk_is_born_in_a_partial_open.test.js pinned the husk born by a throw BEFORE
// `_coreBrowserService` is assigned: `element` is set, the guard is not armed, so
// the next `open()` falls through and rebuilds (beside the husk, hence the orphan
// root). That one is repairable, and `attachTerminalSurfaceToHost` repairs it.
//
// The live autopsy then found husks that report `mode=rebuild_from_husk_failed` —
// the guard IS armed, so `open()` is a no-op and nothing but a remount helps. That
// was written up as a second species: "a terminal that opened COMPLETELY and lost
// its screen afterwards", with the open question "who removes `.xterm-screen` from
// an already-opened terminal?"
//
// NOBODY DOES. Read the order of statements inside `open()`:
//
//     this.element = createElement('div'); e.appendChild(this.element)  // root in DOM
//     const t = createDocumentFragment()                                // viewport,
//     ...                                                               // screen,
//     ...                                                               // helpers,
//     ...                                                               // textarea
//     this._coreBrowserService = ...                    // <- ★ THE GUARD ARMS HERE
//     this._charSizeService = ...                       //    and the screen is
//     this._themeService = ...                          //    still in the fragment
//     this._renderService = ...                         //    for six more services
//     this._compositionView = createElement('div')
//     this._mouseService = ...
//     this._linkifier = ...
//     this.element.appendChild(t)                       // <- screen ARRIVES, at last
//
// `_coreBrowserService` is assigned in the MIDDLE of the fragment's life, not at
// the end. So the birth window is not one window but two, split by that one
// statement:
//
//   * a throw BEFORE it  -> husk, guard unarmed  -> species A, open() rebuilds it
//   * a throw AFTER it   -> husk, guard ARMED    -> "species B", open() is a no-op
//
// Same birth site, same mount, same millisecond. The only difference is which side
// of one assignment the throw lands on. There is no "fully-opened terminal" in the
// story, so there is nothing that later removed its screen.
//
// That also names the repair: species B is only unrepairable because the guard is
// armed over a terminal that never finished opening. Disarm it and `open()` builds
// a complete surface — proven below.

const { test } = require('node:test');
const assert = require('node:assert');
const h = require('./harness');

// Same injection tool as husk_is_born_in_a_partial_open.test.js: make the Nth
// `document.createElement('div')` during open() throw. Counting divs keeps the
// test from depending on any single upstream statement staying put.
//
// The measured element order inside open() (probed, not assumed):
//
//   #1 div  this.element            #2 div  .xterm-viewport
//   #3 div  .xterm-screen           #4 div  .xterm-helpers
//   #5 textarea  .xterm-helper-textarea   <- _coreBrowserService is built FROM it,
//                                            so the guard arms right after #5
//   #6 span, #7 div                 <- ★ the species-B band: guard ARMED, no screen
//   #8 div  ...                     <- element.appendChild(fragment) has happened
function openWithFailureAtElementNumber(term, host, failingNumber) {
  const realCreateElement = global.document.createElement.bind(global.document);
  let created = 0;
  global.document.createElement = (tag, ...rest) => {
    created += 1;
    if (created === failingNumber) {
      throw new Error('injected failure inside Terminal.open');
    }
    return realCreateElement(tag, ...rest);
  };
  let threw = null;
  try {
    term.open(host);
  } catch (error) {
    threw = error;
  } finally {
    global.document.createElement = realCreateElement;
  }
  return threw;
}

function freshTerminal() {
  const { Terminal } = h.loadXterm();
  const term = new Terminal({ cols: 80, rows: 24, allowProposedApi: true });
  const host = global.document.createElement('div');
  global.document.body.appendChild(host);
  return { term, host };
}

// The shell's own completeness predicate, kept in sync with
// `terminalSurfaceIsComplete` in crates/yggterm-shell/src/shell.rs.
function surfaceIsComplete(element) {
  return Boolean(element && element.querySelector('.xterm-screen'));
}

// The guard as it is actually written in the bundle:
//   if (this.element?.ownerDocument.defaultView && this._coreBrowserService) return
//
// ⚠ It lives on the CORE terminal, not on the public `Terminal` wrapper — the
// wrapper's `element` is a getter that delegates to `_core.element`. Reading (or
// assigning) `term._coreBrowserService` / `term.element` on the wrapper silently
// does nothing, which is exactly the misread that made an earlier draft of this
// test report "guard never arms". Probe the core.
function guardIsArmed(term) {
  const core = term._core;
  return Boolean(core && core.element?.ownerDocument.defaultView && core._coreBrowserService);
}

test('the guard arms while the screen is still in the fragment, so a late throw is species B', () => {
  const { term, host } = freshTerminal();

  const threw = openWithFailureAtElementNumber(term, host, 6);
  assert.ok(threw, 'the injected failure must actually escape open()');

  // The husk shape, identical to species A's.
  assert.ok(term.element, 'open() set term.element before it threw');
  assert.ok(term.element.isConnected, 'the bare root is in the DOM');
  assert.strictEqual(host.querySelectorAll('.xterm').length, 1, 'exactly one root in the host');
  assert.strictEqual(
    surfaceIsComplete(term.element),
    false,
    'no .xterm-screen under the root — this is a husk',
  );

  // ...but unlike species A, the guard is armed. This one statement is the whole
  // difference between the two "species".
  assert.ok(
    guardIsArmed(term),
    'a throw after _coreBrowserService is assigned leaves the early-return guard ARMED',
  );
});

test('species A and species B differ only by which side of the guard assignment the throw lands on', () => {
  const early = freshTerminal();
  openWithFailureAtElementNumber(early.term, early.host, 2); // before _coreBrowserService
  const late = freshTerminal();
  openWithFailureAtElementNumber(late.term, late.host, 6); // after _coreBrowserService

  // Byte-identical DOM signature: both are a bare connected root with no screen.
  for (const { term } of [early, late]) {
    assert.ok(term.element && term.element.isConnected);
    assert.strictEqual(surfaceIsComplete(term.element), false);
  }

  // The only divergence is the guard.
  assert.strictEqual(guardIsArmed(early.term), false, 'species A: guard unarmed');
  assert.strictEqual(guardIsArmed(late.term), true, 'species B: guard armed');
});

test('species B really is unrepairable by open(): the screen never arrives', () => {
  const { term, host } = freshTerminal();
  openWithFailureAtElementNumber(term, host, 6);

  // This is exactly what attachTerminalSurfaceToHost does today for a husk.
  const husk = term.element;
  husk.remove();
  term.open(host);

  assert.strictEqual(
    surfaceIsComplete(term.element),
    false,
    'open() early-returned — the surface is still a husk, which is why the shell '
      + 'reports mode=rebuild_from_husk_failed',
  );
  assert.strictEqual(
    host.querySelector('.xterm-screen'),
    null,
    'and the host has no screen either',
  );
});

test('disarming the guard lets open() rebuild a complete surface from a species-B husk', () => {
  const { term, host } = freshTerminal();
  openWithFailureAtElementNumber(term, host, 6);
  assert.strictEqual(surfaceIsComplete(term.element), false, 'precondition: a species-B husk');

  // The repair: drop the husk, then clear `element` so the early-return guard no
  // longer holds. `open()` then runs its full body and builds the surface that the
  // throw denied us. Nothing private is reached into — `element` is public API.
  const husk = term.element;
  husk.remove();
  term._core.element = undefined;
  term.open(host);

  assert.ok(term.element, 'open() built a new root');
  assert.ok(term.element.isConnected, 'and it is in the DOM');
  assert.ok(
    surfaceIsComplete(term.element),
    'the rebuilt root HAS an .xterm-screen — species B is repairable after all',
  );
  assert.ok(host.querySelector('.xterm-screen'), 'the screen is in the host');
  assert.strictEqual(
    host.querySelectorAll('.xterm').length,
    1,
    'and the husk did not survive as an orphan root beside it',
  );
});

test('a rebuilt species-B terminal still works: it writes, resizes and reads back', () => {
  const { term, host } = freshTerminal();
  openWithFailureAtElementNumber(term, host, 6);
  term.element.remove();
  term._core.element = undefined;
  term.open(host);

  // A husk that renders nothing is no better than one that is absent, so prove the
  // rebuilt surface is a live terminal, not just a DOM shape that passes the guard.
  term.resize(40, 10);
  assert.strictEqual(term.cols, 40);
  assert.strictEqual(term.rows, 10);

  term.write('hello from a rebuilt husk');
  return new Promise((resolve) => {
    term.write('', () => {
      const line = term.buffer.active.getLine(0);
      assert.ok(line, 'the buffer has a first line');
      assert.match(line.translateToString(true), /hello from a rebuilt husk/);
      resolve();
    });
  });
});

test('disarming is not needed for species A and must not double the roots there', () => {
  // Guard against a regression in the shell owner: the repair below is written to
  // be safe for BOTH species, so pin that the unarmed case still ends with one root.
  const { term, host } = freshTerminal();
  openWithFailureAtElementNumber(term, host, 2);

  const husk = term.element;
  husk.remove();
  term._core.element = undefined;
  term.open(host);

  assert.ok(surfaceIsComplete(term.element));
  assert.strictEqual(host.querySelectorAll('.xterm').length, 1, 'exactly one root');
});
