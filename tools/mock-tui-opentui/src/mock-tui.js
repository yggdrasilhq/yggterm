#!/usr/bin/env node
// mock-tui — the deterministic agent-CLI TUI yggterm can interrogate.
// Spec: docs/spec-mock-tui-opentui.md. The mock is the STIMULUS; yggterm is
// the WITNESS (it emits nothing into the ytrace plane).
//
// Engines: `raw` (default, zero deps — the executable spec of the bytes) and
// `opentui` (@opentui/core — the reason this exists; same ops, real TUI
// framework render loop).

const { compileOps } = require('./ops.js');
const { SCENARIOS, scenarioNames } = require('./scenarios.js');

function parseArgs(argv) {
  const args = {
    scenario: null,
    engine: 'raw',
    holdMs: null,
    list: false,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const a = argv[i];
    if (a === '--scenario') args.scenario = argv[(i += 1)];
    else if (a === '--engine') args.engine = argv[(i += 1)];
    else if (a === '--hold-ms') args.holdMs = Number(argv[(i += 1)]);
    else if (a === '--list') args.list = true;
    else if (a === '--help' || a === '-h') args.help = true;
  }
  return args;
}

function size() {
  return {
    cols: (process.stdout && process.stdout.columns) || 80,
    rows: (process.stdout && process.stdout.rows) || 24,
  };
}

async function loadOpentui() {
  // @opentui/core is ESM with top-level await — dynamic import only.
  try {
    return await import('@opentui/core');
  } catch (_e) {
    process.stderr.write('mock-tui: --engine opentui requires @opentui/core (npm i @opentui/core)\n');
    process.exit(3);
  }
}

async function emit(ops, engine) {
  if (engine === 'opentui') {
    // The opentui engine renders through @opentui/core when installed. It is
    // loaded lazily and NEVER required for the raw engine. Until wired, the
    // loader fails honestly instead of silently degrading mid-scenario —
    // a silent fallback would change the bytes the witness sees.
    await loadOpentui();
    // TODO(lane): route ops through the @opentui/core renderer; until then
    // the raw compiler is the only byte-accurate path (the spec's identity
    // test is defined so wiring this in is itself falsifiable).
    process.stderr.write('mock-tui: opentui engine not wired yet; running raw\n');
  }
  const bytes = compileOps(ops, size());
  if (bytes) process.stdout.write(bytes);
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.help || args.list) {
    process.stdout.write(`mock-tui — scenarios:\n  ${scenarioNames().join('\n  ')}\n`);
    process.stdout.write('usage: mock-tui --scenario <name> [--engine raw|opentui] [--hold-ms N]\n');
    process.exit(0);
  }
  if (!args.scenario || !SCENARIOS[args.scenario]) {
    process.stderr.write(`mock-tui: unknown scenario ${args.scenario}; try --list\n`);
    process.exit(2);
  }

  const built = SCENARIOS[args.scenario]();
  process.on('SIGWINCH', () => {
    if (built.onWinch) emit(built.onWinch(), args.engine).catch(e => { throw e; });
  });
  process.on('SIGINT', () => {
    process.stdout.write('\x1b[?1049l\x1b[?25l\x1b[0m');
    process.stdout.write(`MOCKTUI ${args.scenario} ok\n`);
    process.exit(0);
  });
  process.stdin.setEncoding('utf8');
  process.stdin.on('data', (chunk) => {
    for (const line of chunk.split('\n')) {
      if (line === 'q') {
        process.stdout.write(`MOCKTUI ${args.scenario} ok\n`);
        process.exit(0);
      }
      // Scripted transitions: one PTY, a SEQUENCE of stimuli.
      if (line.startsWith(':')) {
        const [cmd, ...rest] = line.slice(1).trim().split(/\s+/);
        if (cmd === 'fill') emit([{ op: 'fill', color: rest[0] || 'red' }], args.engine);
        else if (cmd === 'title') emit([{ op: 'title', text: rest.join(' ') }], args.engine);
        else if (cmd === 'alt-enter') emit([{ op: 'alt-enter' }], args.engine);
        else if (cmd === 'alt-exit') emit([{ op: 'alt-exit' }], args.engine);
      }
    }
  });

  await emit(built.ops, args.engine);
  if (built.tick) {
    let n = 0;
    const timer = setInterval(() => {
      n += 1;
      emit(built.tick.ops(n), args.engine);
    }, built.tick.everyMs);
    timer.unref();
  }
  if (Number.isFinite(args.holdMs)) {
    setTimeout(() => {
      process.stdout.write(`MOCKTUI ${args.scenario} ok\n`);
      process.exit(0);
    }, args.holdMs).unref();
  }
}

main();
