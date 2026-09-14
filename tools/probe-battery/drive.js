// tools/probe-battery/drive.js — the T2 drive engine (stone
// docs/cli-integration-layer.md §5).
//
// Spawns a real CLI under node-pty, feeds the EXACT vendored xterm.js the app
// ships, and answers CPR/DSR from the rendered buffer. Graduated from the
// muse/kimi/grok probe lab (zseat-d-muse-probe-112700) where the pattern was
// measured: ⛔ a naive pty drive renders nothing — CLIs that query the cursor
// (DSR `\x1b[6n`) stall forever without a CPR answer, and anything asserted
// off raw bytes is not what the user's viewport shows. Everything here reads
// the rendered xterm buffer instead.
//
// Per-CLI suites (suites/<cli>.js) drive this engine through the T2 sequence
// and emit measured facts that fill schema v2 (phrase tables, composer
// marker, recency/resume behavior). Probes launch their own children — no
// daemon involvement, on-demand, isolated.

const path = require('path');
const fs = require('fs');
const { JSDOM } = require('jsdom');

const REPO_ROOT = path.resolve(__dirname, '..', '..');

function ensureDomGlobals() {
  if (global.__yggtermBatteryDomReady) return;
  const dom = new JSDOM('<!doctype html><html><body></body></html>', {
    pretendToBeVisual: true,
  });
  const { window } = dom;
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
  global.window = window;
  global.document = window.document;
  global.navigator = window.navigator;
  global.HTMLElement = window.HTMLElement;
  global.requestAnimationFrame = (cb) => setTimeout(() => cb(Date.now()), 0);
  global.cancelAnimationFrame = (id) => clearTimeout(id);
  global.__yggtermBatteryDomReady = true;
}

function loadXterm() {
  ensureDomGlobals();
  // UMD vendored build — byte-identical to the WebKit webview's terminal.
  return require(path.join(REPO_ROOT, 'assets', 'xterm', 'xterm.js'));
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

class Drive {
  /**
   * @param {object} opts
   * @param {string} opts.command absolute or PATH command to spawn
   * @param {string[]} [opts.args]
   * @param {string} opts.cwd working dir for the child (real, not scratch —
   *   CLIs key stores off cwd)
   * @param {number} [opts.cols] @param {number} [opts.rows]
   * @param {string} opts.artifactsDir where raw bytes / snaps / timeline go
   * @param {string} [opts.label] artifact filename prefix
   */
  constructor(opts) {
    const pty = require('node-pty');
    this.artifactsDir = opts.artifactsDir;
    this.label = opts.label || 'probe';
    fs.mkdirSync(this.artifactsDir, { recursive: true });
    this.t0 = Date.now();
    this.timeline = [];
    this.exitCode = null;

    const xterm = loadXterm();
    this.term = new xterm.Terminal({
      cols: opts.cols || 120,
      rows: opts.rows || 36,
      scrollback: 2000,
      allowProposedApi: true,
    });
    const el = global.document.createElement('div');
    global.document.body.appendChild(el);
    this.term.open(el);

    this.raw = fs.openSync(path.join(this.artifactsDir, `${this.label}-raw.bin`), 'w');
    this.child = pty.spawn(opts.command, opts.args || [], {
      name: 'xterm-256color',
      cols: opts.cols || 120,
      rows: opts.rows || 36,
      cwd: opts.cwd,
      env: { ...process.env, TERM: 'xterm-256color', COLORTERM: 'truecolor' },
    });

    // CPR: the CLI asks the cursor position; answer from the rendered buffer.
    let cprNeeded = false;
    let inFlight = 0;
    const maybeAnswerCpr = () => {
      if (inFlight === 0 && cprNeeded) {
        cprNeeded = false;
        this.child.write(
          `\x1b[${this.term.buffer.active.cursorY + 1};${this.term.buffer.active.cursorX + 1}R`,
        );
      }
    };
    const feed = (d) => {
      inFlight++;
      this.term.write(d, () => {
        inFlight--;
        maybeAnswerCpr();
      });
    };
    this.child.onData((d) => {
      fs.writeSync(this.raw, Buffer.from(d, 'binary'));
      const s = d.toString('binary');
      if (s.includes('\x1b[6n')) {
        cprNeeded = true;
        feed(s.split('\x1b[6n').join(''));
        maybeAnswerCpr();
      } else {
        feed(s);
      }
    });
    this.child.onExit(({ exitCode: e }) => {
      this.exitCode = e;
      this.log(`EXIT code=${e}`);
    });
    this.log(`SPAWN ${opts.command} ${(opts.args || []).join(' ')} (cwd ${opts.cwd})`);
  }

  log(msg) {
    const line = `${((Date.now() - this.t0) / 1000).toFixed(2)}s ${msg}`;
    this.timeline.push(line);
    fs.appendFileSync(path.join(this.artifactsDir, `${this.label}-timeline.log`), line + '\n');
  }

  /** The rendered viewport exactly as the user's buffer holds it. */
  screen() {
    const buf = this.term.buffer.active;
    const out = [];
    for (let y = 0; y < buf.length; y++) {
      const l = buf.getLine(y);
      out.push(l ? l.translateToString(true) : '');
    }
    while (out.length && !out[out.length - 1].trim()) out.pop();
    return out.join('\n');
  }

  snap(name) {
    fs.writeFileSync(
      path.join(this.artifactsDir, `${this.label}-${name}.txt`),
      this.screen() + '\n',
    );
    this.log(`SNAP ${name}`);
  }

  write(data) {
    this.child.write(data);
  }

  async waitFor(pred, timeoutMs, pollMs = 200) {
    const end = Date.now() + timeoutMs;
    while (Date.now() < end) {
      if (pred()) return true;
      if (this.exitCode !== null) return false;
      await sleep(pollMs);
    }
    return false;
  }

  /**
   * The working-window poll: sample the screen for `phrases` until the frame
   * hash settles. Returns the frames with per-poll phrase hits — the measured
   * material for working_screen_phrases (schema v2).
   */
  async phrasePoll(phrases, { maxPolls = 100, pollMs = 200, settlePolls = 15 } = {}) {
    const frames = [];
    let settled = 0;
    let last = '';
    for (let i = 0; i < maxPolls; i++) {
      await sleep(pollMs);
      const s = this.screen();
      const h = s.length + ':' + s.slice(-400);
      if (h === last) settled++;
      else {
        settled = 0;
        last = h;
      }
      frames.push({
        t: (Date.now() - this.t0) / 1000,
        phrases: phrases.filter((p) => s.includes(p)),
        screen_tail: s.slice(-400),
      });
      if (settled >= settlePolls && i > 10) break;
      if (this.exitCode !== null) break;
    }
    return frames;
  }

  /** The panic falsifier's kill: SIGKILL the child, expect the pty to close. */
  async killChild() {
    if (this.child.pid) {
      try {
        process.kill(this.child.pid, 'SIGKILL');
      } catch (_) {
        /* already gone */
      }
    }
    const end = Date.now() + 5000;
    while (this.exitCode === null && Date.now() < end) await sleep(50);
    return this.exitCode;
  }

  dispose() {
    try {
      fs.closeSync(this.raw);
    } catch (_) {}
    if (this.exitCode === null) {
      try {
        this.child.kill();
      } catch (_) {}
    }
  }
}

module.exports = { Drive, sleep, REPO_ROOT };
