// tools/probe-battery/suites/kimi.js — the 11.6.6 kimi suite (class C,
// TUI-with-store).
//
// Kimi 1.50.0's composer is the defect [11.6.6-b] shape: a labeled rule
// region `── input ──` with NO marker glyph — so this suite measures the
// LABEL (the region shape the readiness gate needs), not a glyph, and
// byte-checks that the declared `❯` really is absent before anyone re-points
// it. Measured on an UNAUTHENTICATED install (no /login on this host): the
// welcome panel, the composer region and the instant `LLM not set` refusal
// all render without credentials; the working phrases stay login-gated and
// are reported as the nulls they are.
//
//   node run.js --suite suites/kimi.js --cwd <real-workdir> \
//        [--suite-arg bin=kimi]
//
// Store note: kimi keys sessions off cwd (md5(cwd) buckets under
// ~/.kimi/sessions/<md5>/<uuid>/ with context.jsonl + wire.jsonl). Run with
// a dedicated probe cwd; the wire.jsonl TurnBegin/TurnEnd events of the
// unauth turn are the phase-feed evidence.

const fs = require('fs');
const path = require('path');

module.exports = {
  name: 'kimi',

  async run(ctx) {
    const bin = ctx.args.bin || 'kimi';
    ctx.drive = new ctx.Drive({
      command: bin,
      args: [],
      cwd: ctx.cwd, // real cwd: kimi buckets its store off md5(cwd)
      artifactsDir: ctx.artifactsDir,
      label: 'kimi',
    });
    const drive = ctx.drive;

    // 1. launch → welcome panel paints (Session: <uuid> is the measured
    //    screen-level id source — a schema-v2 fact independent of the store).
    await ctx.probe('launch-first-paint', async () => {
      const painted = await drive.waitFor(() => drive.screen().trim().length > 0, 60000);
      if (!painted) throw new Error('no paint within 60s');
      await new Promise((r) => setTimeout(r, 2500)); // let the welcome settle
      drive.snap('first-paint');
      const screen = drive.screen();
      const sessionLine = screen.split('\n').find((l) => /Session:\s*[\da-f-]{36}/i.test(l));
      ctx.facts.screen_id_source = sessionLine ? sessionLine.trim() : null;
      ctx.facts.welcome_auth_line =
        screen.split('\n').find((l) => l.toLowerCase().includes('/login'))?.trim() ?? null;
      return sessionLine ? 'painted, screen-level session id present' : 'painted, no session line';
    });

    // 2. the composer shape — THE [11.6.6-b] measurement. Find the labeled
    //    rule region and prove the declared glyph is NOT drawn.
    await ctx.probe('composer-region-shape', async () => {
      await new Promise((r) => setTimeout(r, 1500));
      drive.snap('composer-idle');
      const lines = drive
        .screen()
        .split('\n')
        .map((l) => l.trim())
        .filter((l) => l.length > 0);
      // The label line: a rule line carrying the word `input` (the rendered
      // trim of `──── input ────` once box chrome is stripped).
      const labelLine = lines.find((l) => /^─*[\s─]*input[\s─]*─*$/.test(l));
      const labelWord = lines.filter((l) => l === 'input').length;
      ctx.facts.composer_region_label = labelLine ? labelLine : labelWord ? 'input' : null;
      // Byte-exact absence check: U+276F (❯) must not be drawn anywhere on
      // the idle screen if the declared marker is to stay honest.
      ctx.facts.composer_marker_observed = screenHasGlyph(drive.screen())
        ? '\u276f'
        : null;
      if (!labelLine && !labelWord) {
        throw new Error(
          'no `── input ──` region label on the idle screen — re-measure before any descriptor edit',
        );
      }
      return labelLine ? `region label: ${JSON.stringify(labelLine)}` : 'label trims to `input`';
    });

    // 3. a turn (unauth): the prompt echoes `✨ <text>` and kimi answers the
    //    instant not-logged-in refusal — the wire.jsonl TurnBegin/TurnEnd
    //    pair this drive produces is the natural v2 phase feed, and the
    //    screen afterwards shows whether the region label persists.
    await ctx.probe('turn-unauth-refusal', async () => {
      drive.write('PROBE-KIMI-1104-OK');
      await new Promise((r) => setTimeout(r, 700));
      drive.write('\r');
      const refused = await drive.waitFor(
        () => /not set|\/login|not logged/i.test(drive.screen()),
        20000,
      );
      await new Promise((r) => setTimeout(r, 1500));
      drive.snap('turn-settled');
      const screen = drive.screen();
      ctx.facts.turn_unauth = {
        echoed: screen.includes('PROBE-KIMI-1104-OK'),
        echo_glyph: screen.includes('\u2728') ? '\u2728' : null,
        refusal_seen: refused,
      };
      // The composer region AFTER a turn — does the label persist, and is
      // the region empty again (the readiness-gate below-chrome question)?
      const lines = screen.split('\n').map((l) => l.trim()).filter((l) => l.length > 0);
      const idx = lines.lastIndexOf('input');
      const labelRule = lines.map((l) => /^─*[\s─]*input[\s─]*─*$/.test(l)).lastIndexOf(true);
      const at = Math.max(idx, labelRule);
      ctx.facts.composer_region_after_turn =
        at >= 0 ? { label_at_line: at, below: lines.slice(at + 1, at + 6) } : null;
      return refused ? 'unauth refusal observed' : 'no refusal phrase within 20s';
    });

    // 4. store side-car: the wire.jsonl this drive just created under the
    //    md5(cwd) bucket — TurnBegin/TurnEnd are the declared phase feed.
    await ctx.probe('store-wire-events', async () => {
      const md5 = (s) =>
        require('crypto').createHash('md5').update(s).digest('hex');
      const bucket = path.join(
        process.env.HOME || '',
        '.kimi',
        'sessions',
        md5(ctx.cwd),
      );
      let wire = null;
      if (fs.existsSync(bucket)) {
        const dirs = fs
          .readdirSync(bucket)
          .map((d) => path.join(bucket, d, 'wire.jsonl'))
          .filter((p) => fs.existsSync(p));
        dirs.sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs);
        if (dirs[0]) {
          wire = fs
            .readFileSync(dirs[0], 'utf8')
            .split('\n')
            .filter(Boolean)
            .map((l) => {
              try {
                return JSON.parse(l).type;
              } catch (_) {
                return null;
              }
            })
            .filter(Boolean);
        }
        ctx.facts.store = {
          bucket_exists: true,
          newest_wire_events: wire ? wire.slice(-6) : null,
        };
      } else {
        ctx.facts.store = { bucket_exists: false, bucket };
      }
      return ctx.facts.store.bucket_exists
        ? `wire events: ${(wire || []).join(',')}`
        : `no store bucket for ${ctx.cwd}`;
    });

    // 5. panic falsifier: SIGKILL the child; the drive must NOTICE.
    await ctx.probe('panic-falsifier', async () => {
      const code = await drive.killChild();
      if (code === null) throw new Error('child kill did not surface as pty exit');
      return `exit ${code}`;
    });

    // Login-gated facts this host CANNOT answer — honest nulls, not guesses.
    ctx.facts.working_screen_phrases = null; // composing.../thinking... need /login
    ctx.facts.title_source = null; // 1.50 store holds no title key (door record)
    ctx.facts.resume = null; // re-probe needs a second drive + authed store
  },
};

function screenHasGlyph(screen) {
  return screen.includes('\u276f');
}
