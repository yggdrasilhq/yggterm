// tools/probe-battery/suites/zcode-tui.js — the §9 reference CLI suite
// (11.6.11, first-party, class B). Originally MEASURED on origin/main
// zcode-tui 0.5.9 (5b97267), 2026-09-14 (lane/integration/zcode-tui-battery).
//
// ★ RE-PINNED to the 0.6.x generation 2026-09-29 (suite-repin sitting,
// claim ACK-4ed263776f): re-measured live on the clean main candidate
// lane/zcode-tui/composer-submit-main 24cc1ef (main ee9714a + the
// composer-submit fix), dist sha f669b941f4ce2b7b…, battery artifacts
// /tmp/zt-suite-measure-1. The 0.5.9 chrome moved; every retired needle is
// kept below as a recorded fact (expected false) so the drift stays
// legible, and the descriptor-side phrases (working_screen_phrases /
// composer_footer_hints — `working · working…`, `esc cancel`, `i to type`,
// `zcode-tui`) are MEASURED-STALE against 0.6.x by these facts — the
// descriptor re-pin is the named follow-up.
//
// zcode-tui carries the NativeAnnounce reference implementation (identity +
// phase announced on the wire — the daemon-side listener owns that half).
// This suite measures the SCREEN side through a real pty: what the 0.6.x
// binary actually draws per state, the byte-exact composer caret, the
// sess_-shaped rollout-store law, and resume rederivation.
//
// Measured chrome (0.6.x, at a 120-col pty), encoded below as the net:
//   idle-at-birth  ZCODE block-art banner, ▌-rail composer box, placeholder
//                  `Ask anything…  "<rotating example>"`, caret U+2588 at
//                  the placeholder head (BLINKS ~200ms — caret checks poll
//                  a blink window, never a single sample), status line
//                  `Build auto · <model> · <effort>`, footer
//                  `… /mcps shift+tab agents …` (`ctrl+p commands` exists
//                  but truncates past 120 cols on the idle screen —
//                  needled on the resume screen instead, where it fits)
//   typing         draft echoes, U+2588 trails the text (was U+258F)
//   working        footer swaps to `▪ esc stop` (the SINGLE working arm —
//                  no `working` word, no spinner glyphs, measured zero in
//                  the raw capture; the 0.5.9 `working · working…` status
//                  phrase and `esc cancel` footer are retired)
//   resume         `--resume sess_<uuid>` boots the conversation back
//                  (content rederives; footer shows the cwd + `shift+tab
//                  agents · ctrl+p commands` grammar; `i to type` is a
//                  0.5.9-fact, retired)
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
    // The store is home-scoped and SHARED: any other live zcode session on
    // this host (the seat running this battery, a sibling seat) bumps its
    // own rollout's mtime continuously, so "newest mtime after t0" picks
    // the WRONG file. The drive's rollout is identified by NAME instead:
    // baseline the existing set now (before the turn) and require a NEW
    // filename — the lazy-birth law makes that exact (born at first
    // model-io write, never at launch).
    const rolloutBaseline = new Set(
      fs.readdirSync(rolloutDir)
        .filter((f) => f.startsWith('model-io-') && f.endsWith('.jsonl')),
    );

    // The composer caret BLINKS (the ~200ms App render tick): a single
    // screen sample can land in the OFF phase (measured 2026-09-29 — run 1
    // snapped ON, run 2 OFF at the same settle). Caret presence polls a
    // full blink window; a fixed settle is the flake this net must not be.
    // Returns the test's truthy result (a matched line, or true), else false.
    const blinkSeen = async (test, ms, pollMs = 150) => {
      const end = Date.now() + ms;
      do {
        const res = test(drive.screen());
        if (res) return res;
        await new Promise((r) => setTimeout(r, pollMs));
      } while (Date.now() < end);
      return false;
    };

    // 1. launch → first paint + settled idle chrome.
    await ctx.probe('launch-first-paint', async () => {
      const painted = await drive.waitFor(() => drive.screen().trim().length > 0, 60000);
      if (!painted) throw new Error('no paint within 60s');
      await new Promise((r) => setTimeout(r, 5000)); // store connect + settle
      drive.snap('idle-settled');
      const s = drive.screen();
      ctx.facts.idle = {
        // 0.6.x declared chrome:
        ask_placeholder: s.includes('Ask anything'),
        rotating_example: /Ask anything…\s+"[^"]+"/.test(s),
        composer_rail_u258C: s.includes('\u258C'),
        // anchored to the composer line AND polled over a blink window:
        // the banner art is ALSO U+2588, so a bare screen-wide █ check
        // would pass on the banner alone (vacuous), and a single sample
        // lands in the blink-OFF phase ~half the time.
        caret_u2588_at_rest: await blinkSeen((scr) => /\u2588\s*Ask anything/.test(scr), 1500),
        status_build_line: /Build( auto)? · .+ · /.test(s),
        footer_shift_tab_agents: s.includes('shift+tab agents'),
        banner_blockart_present: s.includes('\u2580\u2580\u2580\u2588') /* ▀▀▀█ */,
        // ctrl+p commands exists in the 0.6.x footer but truncates past
        // 120 cols on the idle screen — measured on the resume screen
        // (probe 5) where the shorter cwd row leaves it visible. null is
        // honest: not measurable at this geometry, not absent.
        footer_ctrl_p_commands: null,
        footer_ctrl_p_commands_why: 'idle footer truncates at 120 cols; needled on resume',
        // 0.5.9 needles, RETIRED — recorded so the drift stays legible:
        composer_glyph_u203A_retired_0_5x: s.includes('\u203A'),
        caret_u258F_retired_0_5x: s.includes('\u258F'),
        status_idle_word_retired_0_5x: /(^|\s|·)idle( |\s|·|$)/.test(s),
        status_ring_idle_u25CB_retired_0_5x: s.includes('\u25CB idle'),
        footer_esc_shortcuts_retired_0_5x: s.includes('esc shortcuts'),
        footer_i_to_type_retired_0_5x: s.includes('i to type'),
        brand_tail_zcode_tui_retired_0_5x: s.includes('zcode-tui'),
      };
      if (!ctx.facts.idle.ask_placeholder) {
        throw new Error('idle screen has no `Ask anything` composer placeholder');
      }
      if (!ctx.facts.idle.composer_rail_u258C || !ctx.facts.idle.caret_u2588_at_rest) {
        throw new Error('idle screen lost the 0.6.x composer chrome (▌ rail / U+2588 caret) — re-measure');
      }
      if (!ctx.facts.idle.status_build_line || !ctx.facts.idle.footer_shift_tab_agents) {
        throw new Error('idle screen lost the 0.6.x status line / footer grammar — re-measure');
      }
      const buildLine = s.split('\n').map((l) => l.trim()).filter((l) => /Build( auto)? · /.test(l))[0] ?? 'n/a';
      return `painted; build line: ${buildLine.slice(0, 80)}`;
    });

    // 2. the composer caret, byte-exact (0.6.x: U+2588 FULL BLOCK trailing
    //    the draft — re-measured true at rest AND typing; U+258F is the
    //    retired 0.5.9 glyph).
    await ctx.probe('composer-caret-byte-exact', async () => {
      drive.write(`Reply with exactly: ${SENTINEL}`);
      await new Promise((r) => setTimeout(r, 1500));
      drive.snap('typed');
      const s = drive.screen();
      if (!s.includes(SENTINEL)) throw new Error('draft text did not echo into the composer');
      // anchored to the DRAFT line and polled over a blink window: the
      // banner art is also U+2588, so the caret must be found on the line
      // that echoes the sentinel, and a single sample can catch blink-OFF.
      const caretLine = await blinkSeen(
        (scr) => scr.split('\n').find((l) => l.includes(SENTINEL) && l.includes('\u2588')) || null,
        2000,
      );
      ctx.facts.composer_marker = {
        declared: '\u2588',
        observed_while_typing: !!caretLine,
        context: caretLine ? caretLine.trim().slice(0, 90) : null,
        retired_0_5x_caret_u258F_seen: s.includes('\u258F'),
      };
      if (!caretLine) throw new Error('U+2588 caret not drawn on the draft line while typing — re-measure before any descriptor edit');
      return `caret drawn: ${caretLine.trim().slice(0, 60)}`;
    });

    // 3. the turn. Pass bar = the model's reply lands AND the 0.6.x working
    //    arm drew. MEASURED 0.6.x truth: the SINGLE working arm is the
    //    footer swap to `▪ esc stop` — there is no `working` word and no
    //    spinner glyph anywhere in the turn frames (raw capture census:
    //    zero braille, zero 'working'). The 0.5.9 declared pair
    //    (`working · working…` + `esc cancel`) is retired; both are kept
    //    as recorded needles expected NEVER to draw. Single-arm sampling
    //    note: a real model turn is multi-second, so 120ms polling cannot
    //    legitimately miss the footer swap — a miss with the reply landed
    //    means the chrome moved again; re-measure before any descriptor
    //    edit.
    await ctx.probe('turn-and-working-phrases', async () => {
      drive.write('\r');
      const needles = ['esc stop', SENTINEL];
      const retired = ['working', 'working \u00B7 working\u2026', 'esc cancel', 'streaming\u2026', '\u25CF running'];
      const frames = await drive.phrasePoll(needles.concat(retired), { maxPolls: 400, pollMs: 120, settlePolls: 25 });
      const seen = new Set(frames.flatMap((f) => f.phrases));
      drive.snap('turn-settled');
      ctx.facts.turn = {
        reply_landed: seen.has(SENTINEL),
        working_arm_declared: 'esc stop',
        esc_stop_frames: frames.filter((f) => f.phrases.includes('esc stop')).length,
        declared_arm_seen: seen.has('esc stop'),
        retired_0_5x_needles: retired.map((p) => ({ phrase: p, seen: seen.has(p) })),
      };
      if (!ctx.facts.turn.reply_landed) throw new Error(`no ${SENTINEL} reply within the turn window`);
      if (!ctx.facts.turn.declared_arm_seen) {
        throw new Error('the 0.6.x working arm (`esc stop` footer) never drew — chrome drifted; re-measure before any descriptor edit');
      }
      return `reply landed; esc-stop frames: ${ctx.facts.turn.esc_stop_frames}`;
    });

    // 4. the rollout-store law: this drive minted `model-io-sess_<uuid>.jsonl`
    //    (runtime mints sess_<uuid>; the FILE NAME is the identity). MEASURED:
    //    the file is born LAZILY at the first model-io write (turn time), not
    //    at launch — poll for it instead of checking once.
    //    Selection is three-stage because the store is home-scoped and SHARED:
    //    (a) NEW name vs the baseline (mtime recency alone is wrong — any
    //    concurrent live zcode session bumps its own file); (b) a sibling's
    //    idle-pruned jsonl can be REBORN mid-poll, so (c) candidates must
    //    carry THIS drive's turn sentinel in their content (the rollout's
    //    model-io rows embed the request body, sentinel included — the drive's
    //    only model call is probe 3's turn, so its file is the sentinel's
    //    file; sibling sessions never contain it).
    const driveOwnsRollout = (f) => {
      try {
        return fs.readFileSync(path.join(rolloutDir, f), 'utf8').includes(SENTINEL);
      } catch { return false; }
    };
    await ctx.probe('store-session-id-law', async () => {
      let fresh = null;
      const end = Date.now() + 30000;
      while (Date.now() < end && !fresh) {
        fresh = fs.readdirSync(rolloutDir)
          .filter((f) => f.startsWith('model-io-') && f.endsWith('.jsonl') && !rolloutBaseline.has(f))
          .filter((f) => /^model-io-sess_[0-9a-f-]{36}\.jsonl$/.test(f))
          .filter(driveOwnsRollout)
          .map((f) => ({ f, m: fs.statSync(path.join(rolloutDir, f)).mtimeMs }))
          .sort((a, b) => b.m - a.m)[0] || null;
        if (!fresh) await new Promise((r) => setTimeout(r, 1000));
      }
      ctx.facts.store = {
        glob: '.zcode/cli/rollout/model-io-*.jsonl',
        baseline_size: rolloutBaseline.size,
        selection: 'new-name + sentinel-in-content',
        new_rollout: fresh ? fresh.f : null,
        sess_shaped: fresh ? /^model-io-sess_[0-9a-f-]{36}\.jsonl$/.test(fresh.f) : null,
      };
      if (!fresh) throw new Error('no new rollout file within 30s of the turn — store law broken');
      if (!ctx.facts.store.sess_shaped) throw new Error(`rollout name not sess_-shaped: ${fresh.f}`);
      return fresh.f;
    });

    // 5. resume: kill the row, `--resume <sess id>`, the conversation must
    //    rederive (descriptor: content_rederives_on_resume). 0.6.x resume
    //    footer: cwd + `shift+tab agents · ctrl+p commands` (fits 120 cols
    //    on this screen — the idle footer truncates, this one doesn't).
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
      // Rederivation is the pass bar — POLL for it, never a fixed settle:
      // under host load the resumed TUI can hold `Resuming session… · idle
      // · opening` well past any fixed sleep (measured 2026-09-14/15).
      const rederived = await drive2.waitFor(() => drive2.screen().includes(SENTINEL), 60000);
      await new Promise((r) => setTimeout(r, 1000));
      drive2.snap('resumed');
      const rs = drive2.screen();
      ctx.facts.resume = {
        session_id: sid,
        painted: !!painted,
        content_rederived: !!rederived && rs.includes(SENTINEL),
        footer_agents_commands: rs.includes('shift+tab agents') && rs.includes('ctrl+p commands'),
        footer_mode_hints_retired_0_5x: rs.includes('i type') || rs.includes('i to type'),
      };
      if (!ctx.facts.resume.content_rederived) throw new Error(`resume of ${sid} did not rederive the conversation`);
      if (!ctx.facts.resume.footer_agents_commands) {
        throw new Error('resume footer lost the 0.6.x `shift+tab agents · ctrl+p commands` grammar — re-measure');
      }
      return `resumed ${sid}; sentinel rederived; 0.6.x footer grammar present`;
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
