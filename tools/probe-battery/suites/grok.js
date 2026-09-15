// tools/probe-battery/suites/grok.js — the 11.6.8 grok-build suite (class C,
// TUI-with-store).
//
// Every declared fact in the descriptor's grok block was measured on 1.0.24
// (2026-08-13 / 2026-09-10); this suite is the regression net that re-asks
// them of whatever binary is installed NOW. Measured live against 1.0.30
// (the needle surgery of 09-11 — `waiting for response` + `[stop]`, the
// Ctrl+c footer swap, `ctrl+b:send to bg` — must hold or the suite reddens).
// Auth is assumed (the descriptor's own 08-13 note: a fleet host is signed
// in); a login/trust screen is recorded as the gate it is and the turn
// facts degrade to honest nulls, kimi-suite style.
//
//   node run.js --suite suites/grok.js --cwd <real-empty-workdir> \
//        [--suite-arg bin=grok] [--suite-arg prompt="..."]
//
// Store note: sessions bucket percent-encode the cwd
// (`~/.grok/sessions/%2Fhome%2F.../<uuid>/summary.json` beside
// chat_history.jsonl / events.jsonl / updates.jsonl), and the LIVE tenancy
// registry `~/.grok/active_sessions.json` ([{session_id, pid, cwd,
// opened_at}]) is the strongest row→session anchor of any class-C CLI. Both
// are filtered by THIS suite's cwd so a sibling live grok row elsewhere
// cannot shadow the measurement.

const fs = require('fs');
const path = require('path');

const WORKING_NEEDLES = ['waiting for response', '[stop]'];
const FOOTER_WINDOW_ROWS = 10; // SCREEN_FOOTER_WINDOW_ROWS in agent_cli.rs

// Mirror screen_phrases_match EXACTLY: the classifier lowercases each of the
// last N non-empty lines and matches lowercase needle fragments — a
// case-sensitive includes() against the raw screen misses everything the
// vendor draws in Title Case (measured 2026-09-15: 'Waiting for response',
// 'Ctrl+c:cancel' — the suite reddened on its own comparison, not on drift).
function phraseHits(screen, needles) {
  const window = screen
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0)
    .slice(-FOOTER_WINDOW_ROWS)
    .map((l) => l.toLowerCase());
  return needles.map((n) => ({
    needle: n,
    observed: window.some((line) => line.includes(n)),
  }));
}
const BG_HINT = 'ctrl+b:send to bg';
const CANCEL_HINT = 'ctrl+c:cancel';

module.exports = {
  name: 'grok',

  async run(ctx) {
    const bin = ctx.args.bin || 'grok';
    const prompt =
      ctx.args.prompt || 'Count slowly from one to thirty, one number per line, then say DONE.';
    const grokHome = path.join(process.env.HOME || '', '.grok');

    ctx.drive = new ctx.Drive({
      command: bin,
      args: [],
      cwd: ctx.cwd,
      artifactsDir: ctx.artifactsDir,
      label: 'grok',
    });
    const drive = ctx.drive;

    // 1. launch → first paint; record the chrome the idle screen actually
    //    draws and whether a login/trust gate stands in the way.
    let authGated = false;
    await ctx.probe('launch-first-paint', async () => {
      const painted = await drive.waitFor(() => drive.screen().trim().length > 0, 60000);
      if (!painted) throw new Error('no paint within 60s');
      await new Promise((r) => setTimeout(r, 3000)); // let the chrome settle
      drive.snap('first-paint');
      const screen = drive.screen();
      ctx.facts.first_paint = {
        head_lines: screen.split('\n').map((l) => l.trim()).filter(Boolean).slice(0, 8),
        model_footer: screen.split('\n').find((l) => /grok\s*\d|model/i.test(l))?.trim() ?? null,
      };
      authGated = /sign in|login|api key|authenticate|trust this/i.test(screen);
      ctx.facts.auth_gate = authGated ? screen.split('\n').find((l) => /sign in|login|api key|authenticate|trust/i.test(l))?.trim() : null;
      return authGated ? `GATED: ${ctx.facts.auth_gate}` : 'painted, no auth gate';
    });

    // 2. composer idle shape: the declared ❯ marker (U+276F) must be DRAWN
    //    (the 08-13 lesson: strings were silent, the renderer composes it),
    //    the declared idle hints must appear, and idle must have NO Ctrl+c
    //    (the 09-10 swap law's idle half).
    await ctx.probe('composer-idle-shape', async () => {
      if (authGated) return 'skipped — auth gate';
      await new Promise((r) => setTimeout(r, 1500));
      drive.snap('composer-idle');
      const screen = drive.screen();
      const lines = screen.split('\n').map((l) => l.trim()).filter(Boolean);
      ctx.facts.composer_idle = {
        marker_observed: screen.includes('\u276f') ? '\u276f' : null,
        declared_hints_seen: ['/help for commands', 'ctrl', 'grok'].map((h) => ({
          hint: h,
          seen: screen.toLowerCase().includes(h),
        })),
        hint_bar_lines: lines.filter((l) => /:|help/i.test(l) && l.length < 120).slice(-4),
        idle_has_cancel_hint: phraseHits(screen, [CANCEL_HINT])[0].observed,
      };
      if (!ctx.facts.composer_idle.marker_observed) {
        throw new Error('declared composer marker U+276F not drawn — re-measure before any descriptor edit');
      }
      return `marker ❯ drawn; cancel-hint-at-idle: ${ctx.facts.composer_idle.idle_has_cancel_hint}`;
    });

    // 3. THE REAL TURN — the needle measurement. phrasePoll the declared
    //    working needles through the turn; the footer swap and the
    //    post-turn background hint are read off the settled screen.
    let turnSessionId = null;
    await ctx.probe('real-turn-working-needles', async () => {
      if (authGated) {
        ctx.facts.working_screen_phrases = null; // login-gated, honest null
        ctx.facts.working_footer_hints = null;
        return 'skipped — auth gate';
      }
      drive.write(prompt);
      await new Promise((r) => setTimeout(r, 500));
      drive.write('\r');
      // Poll the FULL rendered screen with the classifier's own semantics
      // (drive.phrasePoll's frame.phrases is a case-sensitive full-screen
      // match and its screen_tail is 400 chars — neither mirrors
      // screen_phrases_match's windowed lowercase search, and both miss the
      // spinner row on a fast turn; measured 2026-09-15).
      const hitLines = new Set();
      const seen = new Map([...WORKING_NEEDLES, CANCEL_HINT].map((n) => [n, false]));
      const deadline = Date.now() + 150000;
      let stable = 0;
      let last = '';
      while (Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, 200));
        const screen = drive.screen();
        for (const h of phraseHits(screen, [...WORKING_NEEDLES, CANCEL_HINT])) {
          if (!h.observed) continue;
          seen.set(h.needle, true);
          for (const line of screen.split('\n')) {
            if (line.toLowerCase().includes(h.needle)) hitLines.add(line.trim());
          }
        }
        const h = screen.length + ':' + screen.slice(-200);
        stable = h === last ? stable + 1 : 0;
        last = h;
        if (stable >= 25) break; // ~5s of an unchanged screen: the turn is over
        if (drive.exitCode !== null) break;
      }
      // Let the turn settle fully, then read the post-turn chrome.
      const done = await drive.waitFor(
        () => !phraseHits(drive.screen(), WORKING_NEEDLES)[0].observed,
        120000,
        500,
      );
      await new Promise((r) => setTimeout(r, 2000));
      drive.snap('turn-settled');
      const screen = drive.screen();
      ctx.facts.working_screen_phrases = WORKING_NEEDLES.map((n) => ({
        needle: n,
        observed: seen.get(n),
      }));
      ctx.facts.working_footer_hints = {
        declared: [CANCEL_HINT],
        observed_during_turn: seen.get(CANCEL_HINT),
      };
      ctx.facts.background_agent_hint = {
        declared: [BG_HINT],
        observed_after_turn: phraseHits(screen, [BG_HINT])[0].observed,
      };
      ctx.facts.working_line_sample = [...hitLines].slice(0, 3);
      ctx.facts.turn_reply_ok = /DONE/.test(screen);
      ctx.facts.turn_settled = done;
      if (!seen.get(WORKING_NEEDLES[0]) && !seen.get(WORKING_NEEDLES[1])) {
        if (!ctx.facts.turn_reply_ok) {
          throw new Error('NEITHER declared working needle seen AND no reply — the turn failed, re-run before concluding drift');
        }
        ctx.facts.needle_window_missed = true; // turn beat the poll; alarm on a heavier prompt
        return `turn completed but beat the poll — no needle sampled; heavier prompt needed before declaring drift`;
      }
      return `needles ${WORKING_NEEDLES.filter((n) => seen.get(n)).join(' + ')} observed; cancel-during-turn: ${seen.get(CANCEL_HINT)}; bg-hint-after: ${ctx.facts.background_agent_hint.observed_after_turn}`;
    });

    // 4. store side-car: the session dir this turn created — the quad
    //    (summary.json + chat_history.jsonl + events.jsonl + updates.jsonl),
    //    the summary's generated_title (the Store title authority), and the
    //    machine phase feed in events.jsonl.
    await ctx.probe('store-side-car', async () => {
      const sessionsRoot = path.join(grokHome, 'sessions');
      if (!fs.existsSync(sessionsRoot)) {
        ctx.facts.store = { root_exists: false };
        return 'no ~/.grok/sessions at all';
      }
      // Bucket = percent-encoded cwd; fall back to decoding every bucket in
      // case the encoding drifts (measure, don't assume).
      let bucket = path.join(sessionsRoot, encodeURIComponent(ctx.cwd));
      if (!fs.existsSync(bucket)) {
        const match = fs
          .readdirSync(sessionsRoot)
          .filter((d) => {
            try {
              return decodeURIComponent(d) === ctx.cwd;
            } catch (_) {
              return false;
            }
          })
          .map((d) => path.join(sessionsRoot, d));
        if (match[0]) bucket = match[0];
      }
      if (!fs.existsSync(bucket)) {
        ctx.facts.store = { root_exists: true, bucket_exists: false, bucket_expected: encodeURIComponent(ctx.cwd) };
        return `no store bucket for ${ctx.cwd}`;
      }
      const dirs = fs
        .readdirSync(bucket, { withFileTypes: true })
        .filter((d) => d.isDirectory())
        .map((d) => path.join(bucket, d.name));
      dirs.sort((a, b) => {
        const sa = path.join(a, 'summary.json');
        const sb = path.join(b, 'summary.json');
        return (
          (fs.existsSync(sb) ? fs.statSync(sb).mtimeMs : 0) -
          (fs.existsSync(sa) ? fs.statSync(sa).mtimeMs : 0)
        );
      });
      const newest = dirs[0];
      turnSessionId = newest ? path.basename(newest) : null;
      const quad = ['summary.json', 'chat_history.jsonl', 'events.jsonl', 'updates.jsonl'].map((f) => ({
        file: f,
        present: newest ? fs.existsSync(path.join(newest, f)) : false,
      }));
      let summary = null;
      if (newest && fs.existsSync(path.join(newest, 'summary.json'))) {
        summary = JSON.parse(fs.readFileSync(path.join(newest, 'summary.json'), 'utf8'));
      }
      let eventTail = null;
      if (newest && fs.existsSync(path.join(newest, 'events.jsonl'))) {
        eventTail = fs
          .readFileSync(path.join(newest, 'events.jsonl'), 'utf8')
          .split('\n')
          .filter(Boolean)
          .slice(-6)
          .map((l) => {
            try {
              const e = JSON.parse(l);
              return { type: e.type, phase: e.phase ?? undefined, outcome: e.outcome ?? undefined };
            } catch (_) {
              return null;
            }
          })
          .filter(Boolean);
      }
      ctx.facts.store = {
        bucket_exists: true,
        session_count_in_bucket: dirs.length,
        newest_session_id: turnSessionId,
        quad,
        title_source: summary
          ? { generated_title: summary.generated_title ?? null, authority: 'summary.json (Store)' }
          : null,
        events_tail: eventTail,
      };
      return quad.every((q) => q.present)
        ? `quad complete; title: ${JSON.stringify(summary?.generated_title)}; events: ${eventTail?.map((e) => e.type).join(',')}`
        : `quad INCOMPLETE: ${quad.filter((q) => !q.present).map((q) => q.file).join(',')}`;
    });

    // 5. live tenancy: active_sessions.json must register THIS session with
    //    a live pid and this cwd — the StoreIndex rebind chain's anchor.
    await ctx.probe('active-tenancy-registry', async () => {
      const regPath = path.join(grokHome, 'active_sessions.json');
      if (!fs.existsSync(regPath)) {
        ctx.facts.active_tenancy = { registry_exists: false };
        return 'no active_sessions.json';
      }
      const entries = JSON.parse(fs.readFileSync(regPath, 'utf8'));
      const mine = entries.filter((e) => e.cwd === ctx.cwd);
      const alive = (pid) => {
        try {
          process.kill(pid, 0);
          return true;
        } catch (_) {
          return false;
        }
      };
      ctx.facts.active_tenancy = {
        registry_exists: true,
        registry_entries_total: entries.length,
        mine,
        mine_alive: mine.map((e) => ({ pid: e.pid, alive: alive(e.pid) })),
        matches_session_id: turnSessionId ? mine.some((e) => e.session_id === turnSessionId) : null,
      };
      if (!mine.length) return 'no tenancy entry for this cwd — was the turn session registered?';
      return `tenancy: ${mine.length} entry(ies), session match: ${ctx.facts.active_tenancy.matches_session_id}, alive: ${ctx.facts.active_tenancy.mine_alive.map((m) => m.alive).join(',')}`;
    });

    // 6. flag surface: grok --help re-asked of the installed binary — the
    //    declared selector (`-r, --resume`), birth id (`-s, --session-id`),
    //    model and permission-mode vocabulary must still spell that way.
    await ctx.probe('help-flag-surface', async () => {
      const help = new ctx.Drive({
        command: bin,
        args: ['--help'],
        cwd: ctx.cwd,
        artifactsDir: ctx.artifactsDir,
        label: 'grok-help',
      });
      await help.waitFor(() => help.screen().toLowerCase().includes('resume') || help.exitCode !== null, 20000);
      await new Promise((r) => setTimeout(r, 800));
      help.snap('help');
      const screen = help.screen();
      help.dispose();
      const line = (re) => screen.split('\n').find((l) => re.test(l))?.trim() ?? null;
      ctx.facts.help_flags = {
        resume: line(/--resume/),
        session_id: line(/--session-id/),
        model: line(/--model/),
        permission_mode: line(/--permission-mode/),
        fork_session: line(/--fork-session/),
      };
      return `resume: ${JSON.stringify(ctx.facts.help_flags.resume)}; session-id: ${JSON.stringify(ctx.facts.help_flags.session_id)}`;
    });

    // 7. resume id-reuse + re-tenancy: `--resume <id>` must REUSE the
    //    session id and re-register it under a NEW live pid; the prior
    //    transcript must rederive onto the screen.
    let resumedDrive = null;
    await ctx.probe('resume-id-reuse', async () => {
      if (!turnSessionId) {
        ctx.facts.resume = null; // no session to resume — honest null
        return 'skipped — no session id from the store probe';
      }
      // End drive 1 the way a crash would; then measure whether its
      // tenancy entry lingers with a dead pid (hygiene fact).
      await drive.killChild();
      await new Promise((r) => setTimeout(r, 1500));
      const regPath = path.join(grokHome, 'active_sessions.json');
      const before = fs.existsSync(regPath) ? JSON.parse(fs.readFileSync(regPath, 'utf8')) : [];
      const stale = before.filter((e) => e.cwd === ctx.cwd);
      ctx.facts.tenancy_after_kill = {
        entries_for_cwd: stale.length,
        pids_alive: stale.map((e) => {
          try {
            process.kill(e.pid, 0);
            return true;
          } catch (_) {
            return false;
          }
        }),
      };
      resumedDrive = new ctx.Drive({
        command: bin,
        args: ['--resume', turnSessionId],
        cwd: ctx.cwd,
        artifactsDir: ctx.artifactsDir,
        label: 'grok-resume',
      });
      ctx.drive = resumedDrive;
      const painted = await resumedDrive.waitFor(() => resumedDrive.screen().trim().length > 0, 60000);
      if (!painted) throw new Error('resumed grok never painted');
      // content_rederives_on_resume: the prior turn's prompt must come back.
      const rederived = await resumedDrive.waitFor(
        () => resumedDrive.screen().includes(prompt.slice(0, 20)),
        60000,
        500,
      );
      await new Promise((r) => setTimeout(r, 2000));
      resumedDrive.snap('resumed');
      const after = fs.existsSync(regPath) ? JSON.parse(fs.readFileSync(regPath, 'utf8')) : [];
      const mine = after.filter((e) => e.cwd === ctx.cwd && e.session_id === turnSessionId);
      const alive = mine.map((e) => {
        try {
          process.kill(e.pid, 0);
          return true;
        } catch (_) {
          return false;
        }
      });
      ctx.facts.resume = {
        resumed_session_id: turnSessionId,
        rederived: !!rederived,
        re_tenancy_entries: mine.length,
        re_tenancy_alive: alive,
      };
      return rederived
        ? `rederived ok; re-tenancy: ${mine.length} entry(ies) for the SAME id, alive: ${alive.join(',')}`
        : 'resumed but prior transcript never rederived';
    });

    // 8. panic falsifier: SIGKILL the resumed child; the drive must NOTICE.
    await ctx.probe('panic-falsifier', async () => {
      const target = resumedDrive && resumedDrive.exitCode === null ? resumedDrive : drive;
      const code = await target.killChild();
      if (code === null) throw new Error('child kill did not surface as pty exit');
      return `exit ${code}`;
    });
  },
};
