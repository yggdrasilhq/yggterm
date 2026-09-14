// tools/probe-battery/suites/zcode-tui.js — the §9 reference CLI suite
// (EXPERIMENTAL until the 11.6.11 arm-matrix work lands its needles).
//
// zcode-tui is 11.6.11, first-party, class B: it carries the NativeAnnounce
// reference implementation (identity + phase announced on the wire — the
// hot-restart §3.1 contract's phase enum). The SCREEN side of that contract
// is what this suite measures; the announce WIRE itself belongs to the
// daemon-side listener, not the pty drive.
//
//   node run.js --suite suites/zcode-tui.js --cwd <real-workdir> \
//        [--suite-arg bin=zcode-tui] [--suite-arg prompt="say PROBE-OK"]

module.exports = {
  name: 'zcode-tui',

  async run(ctx) {
    const bin = ctx.args.bin || 'zcode-tui';
    ctx.drive = new ctx.Drive({
      command: bin,
      args: [],
      cwd: ctx.cwd, // real cwd: zcode keys its rollout store off it
      artifactsDir: ctx.artifactsDir,
      label: 'zcode-tui',
    });
    const drive = ctx.drive;

    await ctx.probe('launch-first-paint', async () => {
      const painted = await drive.waitFor(() => drive.screen().trim().length > 0, 60000);
      if (!painted) throw new Error('no paint within 60s');
      await new Promise((r) => setTimeout(r, 2000)); // let the welcome settle
      drive.snap('first-paint');
      return 'painted';
    });

    // The composer glyph + working phrases are the 11.6.11 seat's needles to
    // pin; this suite records what the screen shows rather than guessing.
    await ctx.probe('phases-observed', async () => {
      const frames = await drive.phrasePoll(
        ['esc to interrupt', 'working', 'thinking', 'ctrl+c'],
        { maxPolls: 30, pollMs: 200, settlePolls: 10 },
      );
      const hits = new Set(frames.flatMap((f) => f.phrases));
      ctx.facts.working_screen_phrases = {
        observed: [...hits],
        frames_with_hits: frames.filter((f) => f.phrases.length).length,
      };
      ctx.facts.composer_marker = null; // the 11.6.11 seat fills this byte-for-byte
      drive.snap('phases-settled');
      return `${hits.size} phrase(s) observed pre-turn`;
    });

    if (ctx.args.prompt) {
      await ctx.probe('turn', async () => {
        drive.write(String(ctx.args.prompt));
        await new Promise((r) => setTimeout(r, 700));
        drive.write('\r');
        const frames = await drive.phrasePoll(
          ['esc to interrupt', 'ctrl+c:cancel', 'working', 'thinking'],
          { maxPolls: 150, pollMs: 200 },
        );
        drive.snap('turn-settled');
        ctx.facts.turn = {
          prompt: String(ctx.args.prompt),
          working_frames: frames.filter((f) => f.phrases.length).length,
        };
        return 'submitted (settled)';
      });
    }

    await ctx.probe('panic-falsifier', async () => {
      const code = await drive.killChild();
      if (code === null) throw new Error('child kill did not surface as pty exit');
      return `exit ${code}`;
    });

    ctx.facts.resume = null; // resume/rebind is wrapper+store territory (11.6.11 seat)
    ctx.facts.title_source = null;
  },
};
