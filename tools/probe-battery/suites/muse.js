// tools/probe-battery/suites/muse.js — the 11.6.5 muse suite (class C,
// TUI-with-store).
//
// Every declared fact in the descriptor's muse block was re-measured on
// 1.2.1 on 2026-09-14; this suite is the regression net that re-asks them of
// whatever binary is installed NOW. Measured live against 1.3.0
// (1.3.0-R3057.1, 2026-09-15, the muse lab host): the trust gate needles
// hold verbatim, the `❯` (U+276F) composer holds (U+27E9 zero), the working
// needle `esc to interrupt` holds — the live turn line now also paints an
// `◈` glyph variant beside the known `◇`, which is exactly why the needle
// is the fragment and not the glyph — and `muse resume <uuid>` rederives
// with its `resumed session <uuid>` banner plus the abnormal-run self-report
// (`ended abnormally … resumed cleanly`) after a SIGKILL.
//
// 1.3.0 drift facts this suite records (not descriptor changes — the hard
// declared facts all held):
//   • The idle hint `Start a message with ! …` (measured 1.1.1) is GONE.
//   • `.session.lock` gained a `host=` line (`pid=<pid>\nhost=<host>`), and
//     a SUBAGENT lock (`<uuid>/subagent/<sub-uuid>/.session.lock`, same pid)
//     shadows the session lock — store selection here matches the DIRECT
//     lock only (parent dir is the session dir itself).
//   • The sessions tree now carries FILE artifacts beside the month dirs
//     (measured: an empty `.prior-crash-telemetry-markers-v1`) — the walk
//     filters isDirectory at every level.
//   • `session-index.db` DIVERGES from the day-dir tree: a SIGKILLed
//     session's dir has NO index row (its writes never committed) and the
//     row is not repaired by a later resume — yet `muse resume <uuid>` and
//     `muse resume --last` both find it. The index is NOT the resume
//     universe; the day dirs are. Store readers must not treat index
//     completeness as session existence.
//
// ⛔ TRAP ENCODED (cost probe #1): muse paints the trust gate + update
// banner LONG before the composer (post-update init: skills scan, catalog,
// rules warnings). Typing on first paint leaves the prompt as an unsent
// composer draft with the `\r` swallowed — the suite waits for the composer
// itself (U+276F + the model·effort·cwd footer) before typing.
//
// Auth is assumed (auth.json present on a fleet host); a login screen is
// recorded as the gate it is and the turn facts degrade to honest nulls.
// The cwd MUST be a fresh dir (the trust gate only fires for untrusted
// workspaces; a trusted cwd skips probe 1's gate half honestly).
//
//   node run.js --suite suites/muse.js --cwd <fresh-empty-dir> \
//        [--suite-arg bin=muse] [--suite-arg prompt="..."]
//
// Store note: sessions live at
// `~/.local/share/muse/sessions/YYYY/MM/DD/<uuid>/session.jsonl` with a
// direct `.session.lock` holding `pid=<pid>` while the process lives (and
// LINGERING after death — the /proc-verification law). THIS suite selects
// its session by the live lock's pid, the strongest class-C anchor measured.

const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const WORKING_NEEDLES = ['esc to interrupt'];
const FOOTER_WINDOW_ROWS = 10; // SCREEN_FOOTER_WINDOW_ROWS in agent_cli.rs

// Mirror screen_phrases_match EXACTLY: the classifier lowercases each of the
// last N non-empty lines and matches lowercase needle fragments (the grok
// suite's 2026-09-15 lesson — a case-sensitive full-screen includes() misses
// Title Case rows and long screens alike).
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

const alive = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch (_) {
    return false;
  }
};

// Every direct `.session.lock` under the sessions day-dir tree whose body
// names `pid`. Tree is `sessions/YYYY/MM/DD/<uuid>/` (FOUR levels below the
// root — the grok-style 3-level walk silently matches nothing). Subagent
// locks (<uuid>/subagent/<sub-uuid>/) are collected separately — they
// shadow the session lock on 1.3.0 and must not win.
function findLocks(museHome) {
  const root = path.join(museHome, 'sessions');
  const direct = [];
  const nested = [];
  if (!fs.existsSync(root)) return { direct, nested };
  // ⛔ The tree is not all directories: 1.3.0 drops FILE artifacts beside
  // the year dirs (measured: `.prior-crash-telemetry-markers-v1`) — filter
  // on isDirectory at every level or scandir throws ENOTDIR mid-walk.
  for (const year of fs.readdirSync(root, { withFileTypes: true })) {
    if (!year.isDirectory()) continue;
    const ydir = path.join(root, year.name);
    for (const month of fs.readdirSync(ydir, { withFileTypes: true })) {
      if (!month.isDirectory()) continue;
      const mdir = path.join(ydir, month.name);
      for (const day of fs.readdirSync(mdir, { withFileTypes: true })) {
        if (!day.isDirectory()) continue;
        const ddir = path.join(mdir, day.name);
        for (const uuid of fs.readdirSync(ddir, { withFileTypes: true })) {
          if (!uuid.isDirectory()) continue;
          const sdir = path.join(ddir, uuid.name);
          const lock = path.join(sdir, '.session.lock');
          if (fs.existsSync(lock)) {
            direct.push({ uuid: uuid.name, dir: sdir, lock });
          }
          const sub = path.join(sdir, 'subagent');
          if (fs.existsSync(sub)) {
            for (const s of fs.readdirSync(sub)) {
              const slock = path.join(sub, s, '.session.lock');
              if (fs.existsSync(slock)) nested.push({ uuid: s, dir: path.join(sub, s), lock: slock });
            }
          }
        }
      }
    }
  }
  return { direct, nested };
}

module.exports = {
  name: 'muse',

  async run(ctx) {
    const bin = ctx.args.bin || 'muse';
    const sentinel = `PROBE-ECHO-OK-${Math.random().toString(36).slice(2, 8).toUpperCase()}`;
    const prompt =
      ctx.args.prompt ||
      `Use your shell tool to run exactly: echo ${sentinel} — do not answer from memory; run the command and report its exact output.`;
    const museHome = path.join(process.env.HOME || '', '.local/share/muse');

    ctx.drive = new ctx.Drive({
      command: bin,
      args: [],
      cwd: ctx.cwd,
      artifactsDir: ctx.artifactsDir,
      label: 'muse',
    });
    const drive = ctx.drive;

    // 1. launch → trust gate → composer. The gate fires only on a fresh
    //    (untrusted) workspace; the composer draws LATE (post-update init),
    //    and typing before it produces an unsent draft (the probe #1 trap).
    let authGated = false;
    await ctx.probe('launch-gate-composer', async () => {
      const painted = await drive.waitFor(() => drive.screen().trim().length > 0, 90000);
      if (!painted) throw new Error('no paint within 90s');
      drive.snap('first-paint');
      let screen = drive.screen();
      authGated = /sign in|login|api key|authenticate/i.test(screen) && !/trust this workspace/i.test(screen);
      const gate = /trust this workspace/i.test(screen);
      ctx.facts.startup_gate = { declared_phrases: ['do you trust this workspace?', 'trust and continue'] };
      if (authGated) {
        ctx.facts.startup_gate.login_gate = screen.split('\n').find((l) => /sign in|login|api key/i.test(l))?.trim() ?? null;
        return `GATED: ${ctx.facts.startup_gate.login_gate}`;
      }
      ctx.facts.startup_gate.picker_present = gate;
      if (gate) {
        const s = screen.toLowerCase();
        ctx.facts.startup_gate.phrase_hits = ['do you trust this workspace?', 'trust and continue'].map((n) => ({
          needle: n,
          observed: s.includes(n),
        }));
        if (!ctx.facts.startup_gate.phrase_hits.every((h) => h.observed)) {
          throw new Error('trust picker on screen but a declared gate needle missed — re-measure before any descriptor edit');
        }
        drive.write('1');
        await new Promise((r) => setTimeout(r, 300));
        drive.write('\r');
      }
      // The composer itself — not first paint — is the typing-ready signal.
      const t0 = Date.now();
      const composer = await drive.waitFor(
        () => {
          const s = drive.screen();
          return s.includes('\u276f') && /muse-spark|\bmodel\b/i.test(s);
        },
        150000,
        300,
      );
      ctx.facts.startup_gate.composer_latency_s = composer ? (Date.now() - t0) / 1000 : null;
      if (!composer) {
        drive.snap('no-composer');
        throw new Error('composer (U+276F + footer) never drew within 150s of the gate answer');
      }
      await new Promise((r) => setTimeout(r, 1500));
      drive.snap('composer-ready');
      return gate
        ? `gate answered (both needles hit); composer ready ${ctx.facts.startup_gate.composer_latency_s?.toFixed(1)}s after`
        : `workspace already trusted (gate skipped); composer ready ${ctx.facts.startup_gate.composer_latency_s?.toFixed(1)}s after paint`;
    });

    // 2. composer idle shape: the declared ❯ (U+276F) must be DRAWN and the
    //    dead ⟩ (U+27E9) absent; idle must have NO working needle; the
    //    1.1.1-era idle hint is recorded (measured GONE on 1.3.0).
    await ctx.probe('composer-idle-shape', async () => {
      if (authGated) return 'skipped — auth gate';
      drive.snap('composer-idle');
      const screen = drive.screen();
      ctx.facts.composer_idle = {
        marker_observed: screen.includes('\u276f') ? '\u276f' : null,
        dead_marker_seen: screen.includes('\u27e9') ? '\u27e9' : null,
        idle_hint_line: screen.split('\n').find((l) => /start a message/i.test(l))?.trim() ?? null,
        status_footer: screen.split('\n').find((l) => /· .+ · /.test(l) && /\//.test(l))?.trim() ?? null,
        working_needle_at_idle: phraseHits(screen, WORKING_NEEDLES)[0].observed,
      };
      if (!ctx.facts.composer_idle.marker_observed) {
        throw new Error('declared composer marker U+276F not drawn — re-measure before any descriptor edit');
      }
      return `❯ drawn (⟩ zero); idle-hint present: ${!!ctx.facts.composer_idle.idle_hint_line}; working-needle at idle: ${ctx.facts.composer_idle.working_needle_at_idle}`;
    });

    // 3. THE REAL TURN — the needle measurement. The live turn line is
    //    `<State> (<N>s · esc to interrupt)` and its leading glyph is a
    //    SPINNER — ◇ ◈ ◆ are frames of the SAME line (measured 1.3.0: all
    //    three observed on "Thinking" within one turn), which is exactly
    //    why the needle is the fragment and not a glyph. Done tool events
    //    paint `◆ Ran command · … · ✓ · <N>s · ctrl+o` and PERSIST after
    //    the turn — the false-working trap the needle choice encodes.
    //    Tool use is judge/model-probabilistic: if the first turn answers
    //    without running the shell tool, a second, blunter turn forces it.
    const pollTurn = async (deadlineMs) => {
      const hitLines = new Set();
      const toolLines = new Set();
      const seen = new Map(WORKING_NEEDLES.map((n) => [n, false]));
      const deadline = Date.now() + deadlineMs;
      let stable = 0;
      let last = '';
      while (Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, 250));
        const screen = drive.screen();
        for (const h of phraseHits(screen, WORKING_NEEDLES)) {
          if (!h.observed) continue;
          seen.set(h.needle, true);
          for (const line of screen.split('\n')) {
            if (line.toLowerCase().includes(h.needle)) hitLines.add(line.trim());
          }
        }
        for (const line of screen.split('\n')) {
          if (line.includes('◆')) toolLines.add(line.trim());
        }
        const h = screen.length + ':' + screen.slice(-200);
        stable = h === last ? stable + 1 : 0;
        last = h;
        if (stable >= 24) break; // ~6s of an unchanged screen: the turn is over
        if (drive.exitCode !== null) break;
      }
      return { hitLines, toolLines, seen };
    };

    await ctx.probe('real-turn-working-needles', async () => {
      if (authGated) {
        ctx.facts.working_screen_phrases = null; // login-gated, honest null
        return 'skipped — auth gate';
      }
      drive.write(prompt);
      await new Promise((r) => setTimeout(r, 500));
      drive.write('\r');
      let { hitLines, toolLines, seen } = await pollTurn(180000);
      let toolTurns = 1;
      let doneToolLine = [...toolLines].find((l) => /· ✓ ·/.test(l)) ?? null;
      if (!doneToolLine) {
        // Turn 2: the model answered from itself — force the shell tool so
        // the ◆ persistence fact gets measured this run.
        drive.write(`Do not answer from memory. Use your shell tool now to run exactly: echo ${sentinel} — report its exact output.`);
        await new Promise((r) => setTimeout(r, 500));
        drive.write('\r');
        const second = await pollTurn(180000);
        toolTurns = 2;
        hitLines = new Set([...hitLines, ...second.hitLines]);
        toolLines = new Set([...toolLines, ...second.toolLines]);
        for (const [n, v] of second.seen) if (v) seen.set(n, true);
        doneToolLine = [...toolLines].find((l) => /· ✓ ·/.test(l)) ?? null;
      }
      const done = await drive.waitFor(
        () => !phraseHits(drive.screen(), WORKING_NEEDLES)[0].observed,
        120000,
        500,
      );
      await new Promise((r) => setTimeout(r, 2000));
      drive.snap('turn-settled');
      const screen = drive.screen();
      const toolLineSamples = [...toolLines].slice(0, 6);
      ctx.facts.working_screen_phrases = WORKING_NEEDLES.map((n) => ({
        needle: n,
        observed: seen.get(n),
      }));
      ctx.facts.working_line_samples = [...hitLines].slice(0, 4);
      ctx.facts.glyph_variants = [
        ...new Set([...hitLines].join(' ').match(/[◇◈◆]\s*\w+/g) ?? []),
      ];
      ctx.facts.tool_lines = {
        turns_used: toolTurns,
        observed: toolLines.size > 0,
        running_shape: toolLineSamples.find((l) => /running \(/i.test(l)) ?? null,
        done_shape: doneToolLine,
        persists_post_turn: doneToolLine ? screen.includes(doneToolLine.slice(0, 40)) : null,
      };
      ctx.facts.turn_reply_ok = screen.includes(sentinel);
      ctx.facts.working_needle_post_turn = phraseHits(screen, WORKING_NEEDLES)[0].observed;
      ctx.facts.turn_settled = done;
      if (!seen.get(WORKING_NEEDLES[0])) {
        if (!ctx.facts.turn_reply_ok) {
          throw new Error('working needle never seen AND no reply — the turn failed, re-run before concluding drift');
        }
        ctx.facts.needle_window_missed = true; // turn beat the poll; alarm on a heavier prompt
        return `turn completed but beat the poll — no needle sampled; heavier prompt needed before declaring drift`;
      }
      if (!ctx.facts.turn_reply_ok) throw new Error('needle seen but the echo sentinel never landed — turn output suspect');
      return `needle '${WORKING_NEEDLES[0]}' observed mid-turn (spinner glyphs: ${JSON.stringify(ctx.facts.glyph_variants)}); gone post-turn: ${!ctx.facts.working_needle_post_turn}; done ◆ line: ${doneToolLine ? 'painted + persists' : 'never (no tool ran in 2 turns)'}`;
    });

    // 4. store side-car: THIS session's day dir, selected by the DIRECT
    //    .session.lock naming the live child pid (subagent locks shadow —
    //    1.3.0). Records the lock body (pid + the new host= line) and the
    //    session-index.db divergence honestly.
    let turnSessionId = null;
    await ctx.probe('store-side-car', async () => {
      if (authGated || drive.exitCode !== null) {
        ctx.facts.store = null;
        return 'skipped — auth gate or child gone';
      }
      const { direct, nested } = findLocks(museHome);
      const mine = direct.filter((d) => {
        const body = fs.readFileSync(d.lock, 'utf8');
        const pid = Number((body.match(/pid=(\d+)/) || [])[1]);
        return pid === drive.child.pid;
      });
      ctx.facts.store = {
        direct_locks_total: direct.length,
        nested_subagent_locks_total: nested.length,
        nested_shadows_session: nested.some((n) => mine[0] && n.dir.startsWith(mine[0].dir + path.sep)),
        mine_found: mine.length > 0,
      };
      if (!mine.length) return 'no direct .session.lock names this child pid — store layout drifted?';
      const mineSorted = mine.sort((a, b) => fs.statSync(b.lock).mtimeMs - fs.statSync(a.lock).mtimeMs);
      const session = mineSorted[0];
      turnSessionId = session.uuid;
      const lockBody = fs.readFileSync(session.lock, 'utf8');
      const jsonl = path.join(session.dir, 'session.jsonl');
      const sidecars = ['cron.db', 'approval-review', 'session.peer-history.sqlite3', 'subagent'].map((f) => ({
        file: f,
        present: fs.existsSync(path.join(session.dir, f)),
      }));
      ctx.facts.store.session_uuid = turnSessionId;
      ctx.facts.store.lock_body = lockBody.trim().split('\n');
      ctx.facts.store.lock_pid_alive = alive(Number((lockBody.match(/pid=(\d+)/) || [])[1]));
      ctx.facts.store.session_jsonl_present = fs.existsSync(jsonl);
      ctx.facts.store.sidecars = sidecars;
      let indexRow = null;
      try {
        const out = execFileSync(
          'sqlite3',
          [path.join(museHome, 'session-index.db'), `select status, prompt_count, title from sessions where session_id='${turnSessionId}'`],
          { encoding: 'utf8' },
        ).trim();
        indexRow = out || null;
      } catch (e) {
        indexRow = `sqlite3 unavailable: ${e.message.split('\n')[0]}`;
      }
      ctx.facts.store.index_row = indexRow;
      // MEASURED 2026-09-15 on 1.3.0: a LIVE session may have no index row
      // yet (lazy commit); a SIGKILLed one never gets one, and resume works
      // regardless — the index is not the resume universe.
      if (!indexRow) ctx.facts.store.index_divergence_known = true;
      return `session ${turnSessionId}; jsonl: ${ctx.facts.store.session_jsonl_present}; lock pid alive: ${ctx.facts.store.lock_pid_alive}; lock lines: ${JSON.stringify(ctx.facts.store.lock_body)}; index row: ${indexRow ? 'present' : 'ABSENT (divergence)'}`;
    });

    // 5. flag surface: `muse --help` re-asked of the installed binary — the
    //    resume subcommand, the approval/safety vocabulary, the model flag.
    await ctx.probe('help-flag-surface', async () => {
      const help = new ctx.Drive({
        command: bin,
        args: ['--help'],
        cwd: ctx.cwd,
        artifactsDir: ctx.artifactsDir,
        label: 'muse-help',
      });
      await help.waitFor(() => help.screen().toLowerCase().includes('usage') || help.exitCode !== null, 20000);
      await new Promise((r) => setTimeout(r, 800));
      help.snap('help');
      const screen = help.screen();
      help.dispose();
      const line = (re) => screen.split('\n').find((l) => re.test(l))?.trim() ?? null;
      ctx.facts.help_flags = {
        resume_subcommand: line(/^\s*resume\s/),
        approval_mode: line(/--approval-mode/),
        approval_judge: line(/--approval-judge/),
        trust_workspace: line(/--trust-workspace/),
        yolo: line(/--yolo/),
        model: line(/--model/),
      };
      if (!ctx.facts.help_flags.resume_subcommand) {
        throw new Error("'resume' subcommand missing from --help — the resume selector contract broke");
      }
      return `resume: ${JSON.stringify(ctx.facts.help_flags.resume_subcommand)}; approval-mode: ${!!ctx.facts.help_flags.approval_mode}; yolo: ${!!ctx.facts.help_flags.yolo}`;
    });

    // 6. resume rederive + the abnormal-run self-report: end drive 1 the
    //    way a crash would (SIGKILL), then `muse resume <uuid>` must print
    //    `resumed session <uuid>`, rederive the transcript, and (muse says
    //    so itself) warn that the previous run ended abnormally.
    let resumedDrive = null;
    await ctx.probe('resume-rederive', async () => {
      if (!turnSessionId) {
        ctx.facts.resume = null; // no session to resume — honest null
        return 'skipped — no session id from the store probe';
      }
      await drive.killChild();
      await new Promise((r) => setTimeout(r, 1500));
      resumedDrive = new ctx.Drive({
        command: bin,
        args: ['resume', turnSessionId],
        cwd: ctx.cwd,
        artifactsDir: ctx.artifactsDir,
        label: 'muse-resume',
      });
      ctx.drive = resumedDrive;
      const painted = await resumedDrive.waitFor(() => resumedDrive.screen().trim().length > 0, 60000);
      if (!painted) throw new Error('resumed muse never painted');
      await resumedDrive.waitFor(
        () => {
          const s = resumedDrive.screen();
          return s.includes('\u276f') && /muse-spark|\bmodel\b/i.test(s);
        },
        120000,
        300,
      );
      // content_rederives_on_resume: the turn's sentinel must come back —
      // only the rederived transcript can carry it (the composer is fresh).
      const rederived = await resumedDrive.waitFor(
        () => resumedDrive.screen().includes(sentinel),
        60000,
        500,
      );
      await new Promise((r) => setTimeout(r, 1500));
      resumedDrive.snap('resumed');
      const screen = resumedDrive.screen();
      ctx.facts.resume = {
        resumed_session_id: turnSessionId,
        banner: new RegExp(`resumed session ${turnSessionId}`).test(screen),
        rederived: !!rederived,
        abnormal_self_report: screen.split('\n').find((l) => /ended abnormally|resumed cleanly/i.test(l))?.trim() ?? null,
        gate_shown: /trust this workspace/i.test(screen),
      };
      if (!ctx.facts.resume.banner) throw new Error('`resumed session <uuid>` banner never printed');
      if (!rederived) throw new Error('resumed but the prior transcript never rederived');
      return `banner + rederive ok; abnormal self-report: ${!!ctx.facts.resume.abnormal_self_report}; gate re-shown: ${ctx.facts.resume.gate_shown}`;
    });

    // 7. resume --last: the most recent session must be THIS one — which,
    //    with the index row absent (probe 4), proves --last follows the
    //    day-dir tree, not index recency. Guarded: a sibling live muse row
    //    elsewhere could legitimately be newer.
    await ctx.probe('resume-last-pick', async () => {
      if (!turnSessionId || !resumedDrive) {
        ctx.facts.resume_last = null;
        return 'skipped — no resumed session';
      }
      const code = await resumedDrive.killChild();
      await new Promise((r) => setTimeout(r, 1500));
      const last = new ctx.Drive({
        command: bin,
        args: ['resume', '--last'],
        cwd: ctx.cwd,
        artifactsDir: ctx.artifactsDir,
        label: 'muse-resume-last',
      });
      await last.waitFor(() => last.screen().trim().length > 0, 60000);
      await last.waitFor(
        () => {
          const s = last.screen();
          return s.includes('\u276f') && /muse-spark|\bmodel\b/i.test(s);
        },
        120000,
        300,
      );
      await new Promise((r) => setTimeout(r, 1500));
      last.snap('resumed-last');
      const screen = last.screen();
      const picked = (screen.match(/resumed session ([0-9a-f-]{36})/) || [])[1] ?? null;
      let siblingNewer = false;
      try {
        const { direct } = findLocks(museHome);
        const myMtime = Math.max(
          ...direct.filter((d) => d.uuid === turnSessionId).map((d) => fs.statSync(d.lock).mtimeMs),
        );
        siblingNewer = direct.some((d) => d.uuid !== turnSessionId && fs.statSync(d.lock).mtimeMs > myMtime);
      } catch (_) {
        /* keep false — the assert below carries the evidence */
      }
      ctx.facts.resume_last = {
        picked,
        is_our_session: picked === turnSessionId,
        guarded_sibling_newer: siblingNewer,
        rederived: screen.includes(sentinel),
      };
      ctx.drive = last;
      if (!picked) throw new Error('resume --last printed no `resumed session <uuid>` banner');
      if (picked !== turnSessionId && !siblingNewer) {
        throw new Error(`resume --last picked ${picked}, not our crashed session ${turnSessionId} — recency source drifted`);
      }
      return `--last picked our session: ${picked === turnSessionId} (sibling newer: ${siblingNewer}); rederive: ${ctx.facts.resume_last.rederived}`;
    });

    // 8. panic falsifier + lock hygiene: SIGKILL the resumed child; the pty
    //    must close, and the `.session.lock` must LINGER with a now-dead pid
    //    (measured on 1.1.1, re-measured 1.3.0 — store liveness needs
    //    /proc + cmdline identity, never the lock file alone).
    await ctx.probe('panic-falsifier', async () => {
      const target = ctx.drive && ctx.drive.exitCode === null ? ctx.drive : drive;
      const code = await target.killChild();
      if (code === null) throw new Error('child kill did not surface as pty exit');
      await new Promise((r) => setTimeout(r, 1500));
      let lockLingers = null;
      let lockPidDead = null;
      if (turnSessionId) {
        const lock = path.join(museHome, 'sessions');
        const hits = [];
        const walk = (d, depth) => {
          if (depth > 4) return;
          for (const e of fs.readdirSync(d, { withFileTypes: true })) {
            const p = path.join(d, e.name);
            if (e.isDirectory()) walk(p, depth + 1);
            else if (e.name === '.session.lock' && p.includes(turnSessionId)) hits.push(p);
          }
        };
        if (fs.existsSync(lock)) walk(lock, 0);
        lockLingers = hits.length > 0;
        lockPidDead = hits.every((h) => {
          const pid = Number((fs.readFileSync(h, 'utf8').match(/pid=(\d+)/) || [])[1]);
          return !alive(pid);
        });
      }
      ctx.facts.panic = { pty_exit: code, lock_lingers: lockLingers, lock_pid_dead: lockPidDead };
      return `exit ${code}; lock lingers: ${lockLingers} with dead pid: ${lockPidDead}`;
    });
  },
};
