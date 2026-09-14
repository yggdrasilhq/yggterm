// tools/probe-battery/suites/zcode-tui.js — the §9 reference CLI suite
// (11.6.11, first-party, class B). MEASURED on origin/main zcode-tui
// 0.5.9 (5b97267) via this battery's own drive, 2026-09-14
// (lane/integration/zcode-tui-battery).
//
// zcode-tui carries the NativeAnnounce reference implementation (identity +
// phase announced on the wire — the daemon-side listener owns that half).
// This suite measures the SCREEN side through a real pty: what the 0.5.9
// binary actually draws per state, the byte-exact composer caret, the
// sess_-shaped rollout-store law, and resume rederivation.
//
// Measured chrome (0.5.9), encoded below as the regression net:
//   idle-at-birth  status `idle · N sessions …`, footer `esc shortcuts …`,
//                  composer `› Ask anything…` placeholder + U+258F caret
//   typing         draft echoes, U+258F trails the text
//   working        status line `working · working…`, footer swaps to
//                  `esc cancel · enter send` (NOT the descriptor's
//                  `streaming…` / `● running` — descriptor drift, filed
//                  11.6.11; the suite reports what it sees)
//   resume         `--resume sess_<uuid>` boots the conversation back
//                  (content rederives; footer shows the `i to type · enter
//                  sends` mode-hint state)
//
//   node run.js --suite suites/zcode-tui.js --cwd <real-workdir> \
//        [--suite-arg bin=zcode-tui] [--suite-arg prompt-sentinel=PROBE-OK]

const fs = require('fs');
const path = require('path');

module.exports = {
  name: 'zcode-tui',

  async run(ctx) {
    const bin = ctx.args.bin || 'zcode-tui';
    const SENTINEL = ctx.args['prompt-sentinel'] || 'PROBE-OK-BATTERY';
    ctx.drive = new ctx.Drive({
      command: bin,
      args: [],
      cwd: ctx.cwd, // the session's workspace (the rollout store is home-scoped)
      artifactsDir: ctx.artifactsDir,
      label: 'zcode-tui',
    });
    const drive = ctx.drive;
    const rolloutDir = path.join(
      process.env.HOME || '', '.zcode', 'cli', 'rollout',
    );
    const storeTop = (n = 8) => fs.readdirSync(rolloutDir)
      .filter((f) => f.startsWith('model-io-') && f.endsWith('.jsonl'))
      .map((f) => ({ f, m: fs.statSync(path.join(rolloutDir, f)).mtimeMs }))
      .sort((a, b) => b.m - a.m)
      .slice(0, n);

    // 1. launch → first paint + settled idle chrome.
    await ctx.probe('launch-first-paint', async () => {
      const painted = await drive.waitFor(() => drive.screen().trim().length > 0, 60000);
      if (!painted) throw new Error('no paint within 60s');
      await new Promise((r) => setTimeout(r, 5000)); // store connect + settle
      drive.snap('idle-settled');
      const s = drive.screen();
      ctx.facts.idle = {
        ask_placeholder: s.includes('Ask anything'),
        composer_glyph_u203A: s.includes('\u203A'),
        caret_u258F_at_rest: s.includes('\u258F'),
        status_idle_word: /(^|\s|·)idle( |\s|·|$)/.test(s),
        status_ring_idle_u25CB: s.includes('\u25CB idle'),
        footer_esc_shortcuts: s.includes('esc shortcuts'),
        footer_i_to_type: s.includes('i to type'),
        brand_tail_zcode_tui: s.includes('zcode-tui'),
      };
      if (!ctx.facts.idle.ask_placeholder) {
        throw new Error('idle screen has no `Ask anything` composer placeholder');
      }
      return `painted; idle status: ${s.split('\n').map((l) => l.trim()).filter((l) => /idle|sessions/.test(l))[0] ?? 'n/a'}`;
    });

    // 2. the composer caret, byte-exact (descriptor: U+258F, nothing else
    //    marks the input head — re-measured true on 0.5.9 at rest AND typing).
    await ctx.probe('composer-caret-byte-exact', async () => {
      drive.write(`Reply with exactly: ${SENTINEL}`);
      await new Promise((r) => setTimeout(r, 1500));
      drive.snap('typed');
      const s = drive.screen();
      const caretLine = s.split('\n').find((l) => l.includes('\u258F'));
      ctx.facts.composer_marker = {
        declared: '\u258F',
        observed_while_typing: !!caretLine,
        context: caretLine ? caretLine.trim().slice(0, 90) : null,
      };
      if (!s.includes(SENTINEL)) throw new Error('draft text did not echo into the composer');
      if (!caretLine) throw new Error('U+258F caret not drawn while typing — re-measure before any descriptor edit');
      return `caret drawn: ${caretLine.trim().slice(0, 60)}`;
    });

    // 3. the turn. Pass bar = the model's reply lands. Working-state phrases
    //    are REPORTED as measured (0.5.9 draws `working · working…` + swaps
    //    the footer to `esc cancel · enter send`; the descriptor's
    //    `streaming…`/`● running` did NOT appear across measured turns).
    await ctx.probe('turn-and-working-phrases', async () => {
      drive.write('\r');
      const needles = ['working', 'streaming\u2026', '\u25CF running', 'esc cancel', SENTINEL];
      const frames = await drive.phrasePoll(needles, { maxPolls: 400, pollMs: 120, settlePolls: 25 });
      const seen = new Set(frames.flatMap((f) => f.phrases));
      drive.snap('turn-settled');
      ctx.facts.turn = {
        reply_landed: seen.has(SENTINEL),
        working_phrase_frames: frames.filter((f) => f.phrases.includes('working')).length,
        streaming_ellipsis_seen: seen.has('streaming\u2026'),
        running_dot_seen: seen.has('\u25CF running'),
        esc_cancel_footer_seen: seen.has('esc cancel'),
        declared_phrases_observed: ['streaming\u2026', '\u25CF running'].map((p) => ({ phrase: p, seen: seen.has(p) })),
      };
      if (!ctx.facts.turn.reply_landed) throw new Error(`no ${SENTINEL} reply within the turn window`);
      return `reply landed; working frames: ${ctx.facts.turn.working_phrase_frames}, streaming… seen: ${ctx.facts.turn.streaming_ellipsis_seen}`;
    });

    // 4. the rollout-store law: this drive minted `model-io-sess_<uuid>.jsonl`
    //    (runtime mints sess_<uuid>; the FILE NAME is the identity). MEASURED:
    //    the file is born LAZILY at the first model-io write (turn time), not
    //    at launch — poll for it instead of checking once.
    await ctx.probe('store-session-id-law', async () => {
      let fresh = null;
      const end = Date.now() + 30000;
      while (Date.now() < end && !fresh) {
        fresh = storeTop().find((x) => x.m > drive.t0) || null;
        if (!fresh) await new Promise((r) => setTimeout(r, 1000));
      }
      ctx.facts.store = {
        glob: '.zcode/cli/rollout/model-io-*.jsonl',
        new_rollout: fresh ? fresh.f : null,
        sess_shaped: fresh ? /^model-io-sess_[0-9a-f-]{36}\.jsonl$/.test(fresh.f) : null,
      };
      if (!fresh) throw new Error('no new rollout file within 30s of the turn — store law broken');
      if (!ctx.facts.store.sess_shaped) throw new Error(`rollout name not sess_-shaped: ${fresh.f}`);
      return fresh.f;
    });

    // 5. resume: kill the row, `--resume <sess id>`, the conversation must
    //    rederive (descriptor: content_rederives_on_resume).
    await ctx.probe('resume-rederives', async () => {
      const sid = ctx.facts.store.new_rollout.replace(/^model-io-/, '').replace(/\.jsonl$/, '');
      const killExit = await drive.killChild();
      if (killExit === null) throw new Error('first kill did not surface as pty exit');
      const drive2 = new ctx.Drive({
        command: bin,
        args: ['--resume', sid],
        cwd: ctx.cwd,
        artifactsDir: ctx.artifactsDir,
        label: 'zcode-tui-resume',
      });
      ctx.drive2 = drive2;
      const painted = await drive2.waitFor(() => drive2.screen().trim().length > 0, 60000);
      await new Promise((r) => setTimeout(r, 4500));
      drive2.snap('resumed');
      const rs = drive2.screen();
      ctx.facts.resume = {
        session_id: sid,
        painted: !!painted,
        content_rederived: rs.includes(SENTINEL),
        footer_has_mode_hints: rs.includes('i type') || rs.includes('i to type'),
      };
      if (!ctx.facts.resume.content_rederived) throw new Error(`resume of ${sid} did not rederive the conversation`);
      return `resumed ${sid}; sentinel rederived`;
    });

    // 6. panic falsifier: SIGKILL the resumed row; the drive must NOTICE.
    await ctx.probe('panic-falsifier', async () => {
      const code = await ctx.drive2.killChild();
      if (code === null) throw new Error('child kill did not surface as pty exit');
      return `exit ${code}`;
    });

    // Honest nulls this drive cannot answer:
    //   permission banner (`y allow · a always`) needs a tool-using turn —
    //   phase: not driven here (the suite keeps to read-free turns so it can
    //   run unattended against a real model account).
    //   announce WIRE phases belong to the daemon-side listener, not the pty.
    ctx.facts.question_picker_screen_phrases = null;
    ctx.facts.title_source = 'store (descriptor TitleAuthority::Store; desktop titles)';
  },
};
