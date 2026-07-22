// Pins the upstream xterm.js behavior that CREATES the husk — the counterpart to
// host_reopen_is_a_noop.test.js, which pinned why no repair ever healed one.
//
// The husk's live signature (jojo, 2026-07-22) is an `.xterm` root sitting in the
// host with NO screen under it:
//
//     terminal_host_element_detached ... unrepairable=true
//     orphan_root_without_screen=true xterm_roots=1 screen_in_host=false
//     rows_in_host=false screen_canvases=0
//
// `Terminal.open(parent)` builds that shape by construction, because it appends
// the root FIRST and the screen LAST:
//
//     this.element = document.createElement("div")   // div.terminal.xterm
//     ...
//     e.appendChild(this.element)                    // <- EMPTY root enters the DOM
//     const t = document.createDocumentFragment()    // viewport + screen built here
//     ...                                            // helpers, textarea, services
//     this.element.appendChild(t)                    // <- surface arrives, much later
//
// So anything that throws between those two statements leaves exactly the husk:
// a bare, connected `.xterm` root that every DOM-placement guard reads as "a
// terminal is present" while the viewport is blank forever.
//
// Worse, the early-return guard that makes a second open() a no-op is
//
//     if (this.element?.ownerDocument.defaultView && this._coreBrowserService) return
//
// and `_coreBrowserService` is assigned LATE inside open(). A partial open
// therefore sets `element` but NOT `_coreBrowserService`, so the guard does not
// hold and the next open() falls through and builds a SECOND root beside the
// husk — the "orphan root" with an owner that no longer matches.
//
// See docs/xterm-bugs.md and the surface owner `attachTerminalSurfaceToHost` in
// crates/yggterm-shell/src/shell.rs.

const { test } = require('node:test');
const assert = require('node:assert');
const h = require('./harness');

// Makes the Nth `document.createElement('div')` during open() throw, which is
// how we land inside the window between "root appended" and "screen appended"
// without depending on any single upstream statement staying put.
function openWithFailureAtDivNumber(term, host, failingDivNumber) {
  const realCreateElement = global.document.createElement.bind(global.document);
  let divCount = 0;
  global.document.createElement = (tag, ...rest) => {
    if (String(tag).toLowerCase() === 'div') {
      divCount += 1;
      if (divCount === failingDivNumber) {
        throw new Error('injected failure inside Terminal.open');
      }
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

test('a throw after the root is appended leaves the exact husk shape in the host', () => {
  const { term, host } = freshTerminal();

  // div #1 is `this.element` itself; div #2 is `.xterm-viewport`, which is built
  // AFTER the root has already been appended to the host.
  const threw = openWithFailureAtDivNumber(term, host, 2);
  assert.ok(threw, 'the injected failure must actually escape open()');

  // This is the husk, field for field, as the live autopsy reports it.
  assert.strictEqual(host.querySelectorAll('.xterm').length, 1, 'xterm_roots=1');
  assert.strictEqual(host.querySelector('.xterm-screen'), null, 'screen_in_host=false');
  assert.strictEqual(host.querySelector('.xterm-rows'), null, 'rows_in_host=false');
  assert.strictEqual(host.querySelectorAll('.xterm-screen canvas').length, 0, 'screen_canvases=0');
  assert.ok(term.element, 'term.element exists — so every "is a terminal present?" guard says yes');
  assert.ok(term.element.isConnected, 'and it is connected, so no detach guard fires either');
});

test('the husk root survives a reattach — moving it back cannot heal it', () => {
  // attachTerminalSurfaceToHost prefers `appendChild(term.element)` over open(),
  // which is correct for a COMPLETE surface (host_reopen_is_a_noop.test.js) and
  // powerless here: the root it moves is empty.
  const { term, host } = freshTerminal();
  openWithFailureAtDivNumber(term, host, 2);

  const replacement = global.document.createElement('div');
  global.document.body.appendChild(replacement);
  replacement.appendChild(term.element);

  assert.strictEqual(replacement.querySelectorAll('.xterm').length, 1, 'the root moved');
  assert.strictEqual(
    replacement.querySelector('.xterm-screen'),
    null,
    'and it is still a husk — this is why the live autopsy says unrepairable=true',
  );
});

test('a partial open does NOT arm the early-return guard, so the next open builds a SECOND root', () => {
  const { term, host } = freshTerminal();
  openWithFailureAtDivNumber(term, host, 2);

  const huskRoot = term.element;
  assert.ok(huskRoot, 'the husk root is remembered as term.element');
  assert.strictEqual(
    term._coreBrowserService,
    undefined,
    'the guard\'s second term was never assigned — that is what lets open() run again',
  );

  // A COMPLETE terminal would early-return here and change nothing at all.
  term.open(host);

  assert.notStrictEqual(term.element, huskRoot, 'open() built a brand new root');
  assert.ok(host.querySelector('.xterm-screen'), 'the new root carries a real surface');
  assert.strictEqual(
    host.querySelectorAll('.xterm').length,
    2,
    'the husk was never removed — the host now holds an ORPHAN root beside the live one',
  );
  assert.ok(huskRoot.isConnected, 'the orphan is still in the document, owned by nobody');
});

test('THE REPAIR: dropping the husk first, then re-opening, restores a single healthy surface', () => {
  // This is the fix in `attachTerminalSurfaceToHost` / the mount retry, reduced:
  // remove the husk root BEFORE opening, so the rebuild cannot strand an orphan.
  const { term, host } = freshTerminal();
  openWithFailureAtDivNumber(term, host, 2);
  const huskRoot = term.element;

  huskRoot.remove();
  term.open(host);

  assert.strictEqual(host.querySelectorAll('.xterm').length, 1, 'exactly one root — no orphan left behind');
  assert.ok(host.querySelector('.xterm-screen'), 'the surface is real this time');
  assert.notStrictEqual(term.element, huskRoot, 'and term.element points at the rebuilt root');
  assert.strictEqual(term.element.parentElement, host, 'which lives in the host');
  assert.strictEqual(huskRoot.isConnected, false, 'the husk is gone from the document');
});

test('the repair is a no-op-safe guard on a healthy surface: it never fires', () => {
  // `existingIsHusk` gates the whole rebuild path, so a healthy terminal keeps
  // taking the cheap reattach branch. This is the regression that matters — the
  // last husk "fix" shipped as an outage by acting on healthy hosts.
  const { term, host } = freshTerminal();
  term.open(host);
  const root = term.element;

  const isHusk = !term.element.querySelector('.xterm-screen');
  assert.strictEqual(isHusk, false, 'a healthy surface must never be classified as a husk');

  host.innerHTML = '';
  host.appendChild(term.element); // the reattach branch
  assert.strictEqual(term.element, root, 'same root, no rebuild');
  assert.ok(host.querySelector('.xterm-screen'), 'and the screen came back with it');
});

test('a COMPLETE terminal is the contrast case: open() early-returns and no orphan appears', () => {
  const { term, host } = freshTerminal();
  term.open(host);
  const root = term.element;

  term.open(host);

  assert.strictEqual(term.element, root, 'the guard held — same root');
  assert.strictEqual(host.querySelectorAll('.xterm').length, 1, 'no second root was built');
});
