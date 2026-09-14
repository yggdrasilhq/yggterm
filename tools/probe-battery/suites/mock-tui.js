// tools/probe-battery/suites/mock-tui.js — the CI-reachable reference suite.
//
// mock-tui (built with the workspace) is the deterministic CLI stand-in: its
// `delayed-prompt` scenario models the real cycle the battery exists to
// measure — a working window (`working... (esc to interrupt)`), then a
// composer (`›` + model footer), then echo turns. Driving it proves the
// ENGINE (launch → working-phases → turn → panic falsifier) end-to-end in CI
// without any real CLI's auth or store; per-CLI suites (zcode-tui, muse, ...)
// reuse this exact shape against the real binary and fill the real needles.
//
//   node run.js --suite suites/mock-tui.js --cwd /tmp/probe-ws \
//        --suite-arg bin=<path-to-mock-tui>

const path = require('path');

module.exports = {
  name: 'mock-tui',

  async run(ctx) {
    const bin = ctx.args.bin || path.join(ctx.REPO_ROOT, 'target', 'debug', 'mock-tui');
    ctx.drive = new ctx.Drive({
      command: bin,
      args: ['--scenario', 'delayed-prompt', '--ready-after-ms', '1200'],
      cwd: ctx.cwd,
      artifactsDir: ctx.artifactsDir,
      label: 'mock-tui',
    });
    const drive = ctx.drive;

    // 1. launch → first paint (the CPR trap makes this non-trivial)
    await ctx.probe('launch-first-paint', async () => {
      const painted = await drive.waitFor(() => drive.screen().trim().length > 0, 15000);
      if (!painted) throw new Error('no paint within 15s');
      drive.snap('first-paint');
      return 'painted';
    });

    // 2. working-phases → the measured material for working_screen_phrases +
    //    composer_marker (schema v2), observed frame by frame.
    await ctx.probe('working-phases', async () => {
      const frames = await drive.phrasePoll(
        ['working... (esc to interrupt)', '\u203a', 'gpt-mock'],
        { maxPolls: 40, pollMs: 150, settlePolls: 8 },
      );
      const sawWorking = frames.some((f) => f.phrases.includes('working... (esc to interrupt)'));
      const sawComposer = frames.some((f) => f.phrases.includes('\u203a'));
      ctx.facts.working_screen_phrases = {
        observed: frames.filter((f) => f.phrases.length).map((f) => f.phrases),
        first_working_ms: (frames.find((f) => f.phrases.length) || {}).t ?? null,
      };
      ctx.facts.composer_marker = sawComposer ? '\u203a' : null;
      drive.snap('phases-settled');
      if (!sawWorking) throw new Error('working window never observed');
      if (!sawComposer) throw new Error('composer marker never observed');
      return `working + composer observed across ${frames.length} frames`;
    });

    // 3. a turn: submit and await the echo round-trip.
    await ctx.probe('turn', async () => {
      drive.write('PROBE-TURN-OK\r');
      const echoed = await drive.waitFor(
        () => drive.screen().includes('ECHO: PROBE-TURN-OK'),
        10000,
      );
      drive.snap('turn-echoed');
      ctx.facts.turn_echo = echoed;
      if (!echoed) throw new Error('echo never arrived');
      return 'round-trip';
    });

    // 4. panic falsifier: SIGKILL the child; the drive must NOTICE (pty
    //    closes, exit surfaces) — a probe that hangs here is exactly the
    //    "dead child, alive viewport" lie the falsifier exists to kill.
    await ctx.probe('panic-falsifier', async () => {
      const code = await drive.killChild();
      if (code === null) throw new Error('child kill did not surface as pty exit');
      return `exit ${code}`;
    });

    // 5. capabilities a real CLI has and mock-tui deliberately does not.
    ctx.facts.title_source = null;
    ctx.facts.resume = null;
    ctx.facts.in_tui_switch = null;
    ctx.log('title/resume/in-TUI-switch: null (no such capability in mock-tui — honest null)');
  },
};
