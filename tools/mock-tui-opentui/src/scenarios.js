// The scenario registry — the matrix in docs/spec-mock-tui-opentui.md, as
// code. Each scenario returns { ops, holdMs }. `frames` ops are produced by
// the small generator helpers so scripted stdin (`:fill red`,
// `:scenario alt-screen`) can re-enter the same builders mid-run.

const SCENARIOS = {
  // Issue 37 falsifier: paint the ENTIRE grid with a non-theme background.
  // Witness: server app screenshot → the fill reaches every card pixel.
  'bg-fill': () => ({
    ops: [
      { op: 'title', text: 'MOCKTUI bg-fill' },
      { op: 'fill', color: 'red' },
      { op: 'text', row: 1, col: 1, text: 'MOCKTUI bg-fill — every pixel of this card must be red', fg: [255, 255, 255], bg: 'red' },
    ],
  }),

  // Alt-screen truth: enter, frame, exit on ':alt-exit' or q.
  'alt-screen': () => ({
    ops: [
      { op: 'title', text: 'MOCKTUI alt-screen' },
      { op: 'alt-enter' },
      { op: 'fill', color: 'mock' },
      { op: 'text', row: 1, col: 1, text: 'MOCKTUI alt-screen — pty_in_alternate_screen must flip true', fg: [216, 222, 233] },
      { op: 'text', row: 3, col: 1, text: 'q exits; the grid must survive the round trip', fg: [130, 140, 155] },
    ],
  }),

  // Issue 34 stimulus: the per-PTY title plane (`OC | <session title>` shape).
  'title-cycle': () => ({
    ops: [
      { op: 'title', text: 'OC | mock-session-alpha', also1: true },
      { op: 'text', row: 1, col: 1, text: 'MOCKTUI title-cycle — titles change every 2s; the mirror must follow' },
    ],
    tick: {
      everyMs: 2000,
      ops: n => [{ op: 'title', text: `OC | mock-session-${n}`, also1: true }],
    },
  }),

  // [11.39] witness: repaint ONLY on SIGWINCH. A silent identical-geometry
  // resize must produce ZERO bytes; --nudge must produce WINCH_FRAME_<n+1>.
  'winch-witness': () => {
    let n = 0;
    return {
      ops: [
        { op: 'alt-enter' },
        { op: 'frame-marker', n: (n += 1) },
        { op: 'text', row: 1, col: 1, text: 'MOCKTUI winch-witness — repaints only on SIGWINCH' },
      ],
      onWinch: () => [{ op: 'frame-marker', n: (n += 1) }],
    };
  },

  // Mouse-mode arming + SGR click decode.
  'mouse-probe': () => ({
    ops: [
      { op: 'title', text: 'MOCKTUI mouse-probe' },
      { op: 'alt-enter' },
      { op: 'mouse-mode', on: true, modes: [1000, 1002, 1006] },
      { op: 'fill', color: 'mock' },
      { op: 'text', row: 1, col: 1, text: 'MOCKTUI mouse-probe — clicks are decoded and echoed below' },
      { op: 'text', row: 3, col: 1, text: '(no clicks yet)', fg: [130, 140, 155] },
    ],
    onSGRClick: (sgr) => [
      { op: 'text', row: 3, col: 1, text: `click: ${sgr}          ` },
    ],
  }),

  // The codex-inline pattern: committed lines scroll, bottom live region is
  // repainted IN PLACE via absolute CUP (pairs with the Rust pipeline test).
  composer: () => ({
    ops: [
      { op: 'title', text: 'MOCKTUI composer' },
      { op: 'text', row: 1, col: 1, text: 'MOCKTUI composer — committed lines scroll, the bottom region repaints in place' },
      { op: 'text', row: 10, col: 1, text: '▸ working …' },
    ],
  }),

  // Kitty keyboard protocol push/pop; q must pop what was pushed.
  'kitty-keys': () => ({
    ops: [
      { op: 'title', text: 'MOCKTUI kitty-keys' },
      { op: 'kitty-flags', on: true, flags: 1 },
      { op: 'text', row: 1, col: 1, text: 'MOCKTUI kitty-keys — flags pushed; keys echo decoded; q pops' },
    ],
  }),

  // Bracketed paste round-trip.
  'paste-bracketed': () => ({
    ops: [
      { op: 'title', text: 'MOCKTUI paste-bracketed' },
      { op: 'paste-mode', on: true },
      { op: 'text', row: 1, col: 1, text: 'MOCKTUI paste-bracketed — paste a multi-line blob; it lands as one bracketed paste' },
    ],
  }),
};

function scenarioNames() {
  return Object.keys(SCENARIOS);
}

module.exports = { SCENARIOS, scenarioNames };
