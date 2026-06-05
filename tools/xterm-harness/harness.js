// Headless behavioral harness for the EXACT vendored xterm.js the app ships
// (assets/xterm/xterm.js). Lets us write byte sequences and assert buffer /
// scrollback / reflow behavior deterministically in Node — the client-layer
// determinism the campaign needs. See campaign-xterm-dealbreakers.
//
// We load the vendored UMD build under a jsdom DOM so it is byte-identical to
// what runs in the WebKit webview (not a different @xterm/headless build).

const path = require('path');
const { JSDOM } = require('jsdom');

const REPO_ROOT = path.resolve(__dirname, '..', '..');
const XTERM_JS = path.join(REPO_ROOT, 'assets', 'xterm', 'xterm.js');
const XTERM_FIT_JS = path.join(REPO_ROOT, 'assets', 'xterm', 'addon-fit.js');

let cachedXterm = null;

function ensureDomGlobals() {
  if (global.__yggtermDomReady) return;
  const dom = new JSDOM('<!doctype html><html><body></body></html>', {
    pretendToBeVisual: true,
  });
  const { window } = dom;
  // Things xterm.js touches that jsdom doesn't fully provide.
  if (!window.matchMedia) {
    window.matchMedia = () => ({
      matches: false,
      addEventListener() {},
      removeEventListener() {},
      addListener() {},
      removeListener() {},
    });
  }
  if (!window.ResizeObserver) {
    window.ResizeObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    };
  }
  if (!window.IntersectionObserver) {
    window.IntersectionObserver = class {
      observe() {}
      unobserve() {}
      disconnect() {}
    };
  }
  global.window = window;
  global.document = window.document;
  global.navigator = window.navigator;
  global.HTMLElement = window.HTMLElement;
  global.Element = window.Element;
  global.Node = window.Node;
  global.requestAnimationFrame = (cb) => setTimeout(() => cb(Date.now()), 0);
  global.cancelAnimationFrame = (id) => clearTimeout(id);
  global.__yggtermDomReady = true;
}

function loadXterm() {
  if (cachedXterm) return cachedXterm;
  ensureDomGlobals();
  // UMD: require returns the module namespace ({ Terminal, ... }).
  const xterm = require(XTERM_JS);
  let fit = null;
  try {
    fit = require(XTERM_FIT_JS);
  } catch (_e) {
    fit = null;
  }
  cachedXterm = { Terminal: xterm.Terminal, FitAddon: fit && fit.FitAddon };
  return cachedXterm;
}

// Create a terminal. `open` mounts it into a detached jsdom element so the full
// public API (including reflow on resize) is exercised; pass open:false for a
// pure buffer/parser construction.
function createTerminal(opts = {}) {
  const { Terminal } = loadXterm();
  const term = new Terminal({
    cols: opts.cols || 80,
    rows: opts.rows || 24,
    scrollback: opts.scrollback != null ? opts.scrollback : 1000,
    allowProposedApi: true,
  });
  if (opts.open !== false) {
    const el = global.document.createElement('div');
    global.document.body.appendChild(el);
    term.open(el);
  }
  return term;
}

// xterm write is async (parsed on a microtask/timer). Resolve when applied.
function write(term, data) {
  return new Promise((resolve) => term.write(data, resolve));
}

function activeBuffer(term) {
  return term.buffer.active;
}

function lineText(term, absoluteRow) {
  const line = term.buffer.active.getLine(absoluteRow);
  return line ? line.translateToString(true) : null;
}

// Full buffer text (scrollback + viewport), trailing-trimmed.
function bufferText(term) {
  const buf = term.buffer.active;
  const out = [];
  for (let y = 0; y < buf.length; y++) {
    const line = buf.getLine(y);
    out.push(line ? line.translateToString(true) : '');
  }
  return out.join('\n').replace(/\s+$/, '');
}

function baseY(term) {
  return term.buffer.active.baseY;
}

// Background color of a cell, as xterm reports it. Returns
// { default: bool, rgb: bool, color: number } so tests can detect bg-split
// (a cell that should carry a bg but renders the default).
function cellBg(term, absoluteRow, col) {
  const line = term.buffer.active.getLine(absoluteRow);
  if (!line) return null;
  const cell = line.getCell(col);
  if (!cell) return null;
  return {
    isDefault: cell.isBgDefault(),
    isRGB: cell.isBgRGB(),
    isPalette: cell.isBgPalette(),
    color: cell.getBgColor(),
  };
}

module.exports = {
  loadXterm,
  createTerminal,
  write,
  activeBuffer,
  lineText,
  bufferText,
  baseY,
  cellBg,
  paths: { XTERM_JS, XTERM_FIT_JS },
};
