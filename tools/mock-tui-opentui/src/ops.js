// The scenario op vocabulary + the ANSI compiler for the `raw` engine.
//
// A scenario is a LIST of ops. Ops are engine-agnostic: the raw engine
// compiles them to ANSI byte runs (this file); the opentui engine renders
// them through @opentui/core. Both engines must be indistinguishable to the
// yggterm-side witness (docs/spec-mock-tui-opentui.md — the stimulus/witness
// law): same DECSETs, same OSCs, same repaint geometry.

const C = {
  reset: '\x1b[0m',
  clear: '\x1b[2J',
  home: '\x1b[H',
  hideCursor: '\x1b[?25l',
  showCursor: '\x1b[?25h',
  altEnter: '\x1b[?1049h',
  altExit: '\x1b[?1049l',
  cup: (row, col) => `\x1b[${row};${col}H`,
  sgrBg: (r, g, b) => `\x1b[48;2;${r};${g};${b}m`,
  sgrFg: (r, g, b) => `\x1b[38;2;${r};${g};${b}m`,
};

const NAMED_COLORS = {
  red: [190, 40, 40],
  blue: [40, 80, 190],
  green: [30, 140, 70],
  yellow: [190, 160, 40],
  mock: [46, 52, 64],
};

function color(token) {
  if (Array.isArray(token)) return token;
  return NAMED_COLORS[token] || NAMED_COLORS.mock;
}

/**
 * Compile a list of ops into ANSI bytes.
 * cols/rows come from the caller's current process.stdout size so fill ops
 * paint the REAL grid (a fill against a hardcoded grid is not a full-bleed
 * stimulus).
 */
function compileOps(ops, { cols, rows }) {
  let out = '';
  for (const op of ops) {
    switch (op.op) {
      case 'title':
        out += `\x1b]0;${op.text}\x07`;
        if (op.also1) out += `\x1b]1;${op.text}\x07`;
        if (op.also2 !== false) out += `\x1b]2;${op.text}\x07`;
        break;
      case 'alt-enter':
        out += C.altEnter + C.clear + C.home;
        break;
      case 'alt-exit':
        out += C.altExit;
        break;
      case 'fill': {
        const [r, g, b] = color(op.color);
        const blank = ' '.repeat(Math.max(0, cols));
        out += C.home + C.sgrBg(r, g, b);
        for (let i = 0; i < rows; i += 1) out += blank + (i < rows - 1 ? '\n' : '');
        out += C.reset;
        break;
      }
      case 'text': {
        const [fr, fg_, fb] = color(op.fg || 'mock');
        const [br, bgc, bb] = color(op.bg || 'mock');
        out += C.cup(op.row, op.col) + C.sgrFg(fr, fg_, fb) + C.sgrBg(br, bgc, bb) + op.text + C.reset;
        break;
      }
      case 'line':
        // A COMMITTED line: newline-terminated, scrolling the grid the way a
        // streaming CLI's output does (codex token flow, build logs). No CUP,
        // no repaint — the bytes are append-only so the terminal's own
        // scrollback accumulates them. That scrollback IS the history
        // witness: whatever survives a row switch or a daemon handoff is
        // exactly what yggterm retained.
        out += `${op.text}\r\n`;
        break;
      case 'frame-marker':
        out += C.cup(rows, 1) + `WINCH_FRAME_${op.n}`;
        break;
      case 'mouse-mode':
        // DECSET/DECRST 1000 (press), 1002 (drag), 1006 (SGR encoding).
        for (const m of op.modes) out += `\x1b[?${m}${op.on ? 'h' : 'l'}`;
        break;
      case 'cursor':
        out += op.style === 'bar' ? '\x1b[5 q' : op.style === 'underline' ? '\x1b[3 q' : '\x1b[2 q';
        break;
      case 'paste-mode':
        out += `\x1b[?2004${op.on ? 'h' : 'l'}`;
        break;
      case 'kitty-flags':
        // CSI > flags u (push) / CSI < u (pop).
        out += op.on ? `\x1b>${op.flags || 1}u` : `\x1b<u`;
        break;
      case 'sleep':
        // Not a byte op; the run loop honors it.
        break;
      default:
        throw new Error(`mock-tui: unknown op ${op.op}`);
    }
  }
  return out;
}

module.exports = { compileOps, NAMED_COLORS };
