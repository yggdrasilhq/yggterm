// tools/probe-battery/suites/codex.js — the 11.6.1 codex suite (class A,
// store-authoritative).
//
// Every declared fact in the descriptor's codex block was measured on
// codex-cli 0.144.6 (2026-08-06 / 2026-08-22); this suite is the regression
// net that re-asks them of whatever binary is installed NOW. Measured live
// against 0.154.0 (2026-09-15, the muse-lab host): the trust gate's warning
// text grew ("…prompt injection. Trusting the directory allows project-local
// config, hooks, and exec policies to load.") but both declared needles
// still match verbatim; `• Working (0s • esc to interrupt)` holds; the `›`
// composer marker holds; and the turn gained a visible titling lifecycle in
// the footer (`· renaming… ⠋` while the thread is named, then the generated
// title rides the footer beside the cwd).
//
// Auth is assumed (the fleet host is signed in); a login screen is recorded
// as the gate it is and the turn facts degrade to honest nulls.
//
// COST NOTE: answering the trust gate persists
// `[projects."<cwd>"] trust_level = "trusted"` into the REAL
// `~/.codex/config.toml` — one entry per probe cwd per run. Probe cwds are
// throwaways; this is the measured price of driving a class-A CLI past its
// gate headlessly.
//
//   node run.js --suite suites/codex.js --cwd <fresh-empty-dir> \
//        [--suite-arg bin=codex] [--suite-arg prompt="..."]
//
// Store note: sessions file by date tree
// `~/.codex/sessions/YYYY/MM/DD/rollout-<local-timestamp>-<uuid>.jsonl`
// (depth is not fixed), so THIS suite selects its rollout by
// SENTINEL-IN-CONTENT, not by name or mtime — a sibling live codex row
// elsewhere writes rollouts into the same tree (the zcode-tui suite's
// mtime-recency lesson, 2026-09-15). The uuid in the filename is the store
// id (`session_meta.session_id`), and `codex resume <id>` APPENDS to the
// same file — the id-reuse proof.

const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const WORKING_NEEDLES = ['esc to interrupt', 'working ('];
const WORKING_NEGATION = 'worked for '; // codex's COMPLETION summary, not active work
const FOOTER_WINDOW_ROWS = 10; // SCREEN_FOOTER_WINDOW_ROWS in agent_cli.rs

// Mirror screen_phrases_match EXACTLY: the classifier lowercases each of the
// last N non-empty lines and matches lowercase needle fragments (the grok
// suite's 2026-09-15 sampling-trap lesson — phrasePoll is case-sensitive and
// full-screen; the classifier is neither).
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

function homeRel(p) {
  const home = process.env.HOME || '';
  return home && p.startsWith(home) ? p.slice(home.length + 1) : p;
}

module.exports = {
  name: 'codex',

  async run(ctx) {
    const bin = ctx.args.bin || 'codex';
    const sentinel = `DONE-${Date.now().toString(36).slice(-6)}`;
    const prompt =
      ctx.args.prompt ||
      `Count slowly from one to twelve, one number per line, then say ${sentinel}.`;
    const codexHome = path.join(process.env.HOME || '', '.codex');

    ctx.drive = new ctx.Drive({
      command: bin,
      args: [],
      cwd: ctx.cwd,
      artifactsDir: ctx.artifactsDir,
      label: 'codex',
    });
    const drive = ctx.drive;

    // 1. launch → first paint; the trust gate is the DECLARED startup gate —
    //    verify its needles off the live screen, then answer it the way the
    //    screen offers (Enter = `1. Yes, continue`, the default).
    let authGated = false;
    await ctx.probe('launch-trust-gate', async () => {
      const painted = await drive.waitFor(() => drive.screen().trim().length > 0, 60000);
      if (!painted) throw new Error('no paint within 60s');
      await new Promise((r) => setTimeout(r, 3000)); // let the chrome settle
      drive.snap('first-paint');
      let screen = drive.screen();
      authGated = /sign in|login|api key|authenticate/i.test(screen);
      ctx.facts.auth_gate = authGated
        ? screen.split('\n').find((l) => /sign in|login|api key|authenticate/i.test(l))?.trim()
        : null;
      if (authGated) return `GATED: ${ctx.facts.auth_gate}`;
      const gateNeedles = ['do you trust the contents of this directory', 'no, quit'];
      const gateHits = phraseHits(screen, gateNeedles);
      ctx.facts.startup_gate = {
        declared_needles: gateNeedles,
        observed: gateHits.map((h) => h.needle),
        gate_screen_line: screen
          .split('\n')
          .find((l) => /trust the contents/i.test(l))
          ?.trim(),
        answered: false,
      };
      if (gateHits.every((h) => h.observed)) {
        drive.write('\r'); // `› 1. Yes, continue` is the default
        const cleared = await drive.waitFor(
          () => !phraseHits(drive.screen(), gateNeedles)[0].observed,
          20000,
          300,
        );
        await new Promise((r) => setTimeout(r, 2000));
        drive.snap('after-gate');
        screen = drive.screen();
        ctx.facts.startup_gate.answered = cleared;
        if (!cleared) throw new Error('trust gate answered but never cleared — re-measure the gate flow');
      } else if (!/ask codex to do anything/i.test(screen)) {
        throw new Error('neither the declared trust gate nor the composer painted — unknown startup screen');
      }
      const version =
        screen.match(/OpenAI Codex \(v?([0-9][^)]*)\)/)?.[1] ??
        screen.match(/Codex.*?v?(\d+\.\d+\.\d+)/)?.[1] ??
        null;
      ctx.facts.binary = { banner_version: version };
      return `gate needles ${gateHits.filter((h) => h.observed).length}/2 observed, answered: ${ctx.facts.startup_gate.answered}; banner v${version}`;
    });

    // 2. composer idle shape: the declared › (U+203A) marker must be DRAWN,
    //    the placeholder line must be up, and the declared footer-hint
    //    vocabulary is re-asked honestly (which members the idle screen
    //    actually shows).
    await ctx.probe('composer-idle-shape', async () => {
      if (authGated) return 'skipped — auth gate';
      await new Promise((r) => setTimeout(r, 1500));
      drive.snap('composer-idle');
      const screen = drive.screen();
      const lines = screen.split('\n').map((l) => l.trim()).filter(Boolean);
      const declaredHints = ['gpt-', 'claude', 'tab to ', 'ctrl', 'esc'];
      ctx.facts.composer_idle = {
        marker_observed: screen.includes('\u203a') ? '\u203a' : null,
        placeholder_observed: /ask codex to do anything/i.test(screen),
        declared_footer_hints_seen: declaredHints.map((h) => ({
          hint: h,
          seen: screen.toLowerCase().includes(h),
        })),
        footer_line:
          lines.find((l) => l.includes(ctx.cwd) || /gpt-.*·/i.test(l))?.slice(0, 120) ?? null,
      };
      if (!ctx.facts.composer_idle.marker_observed) {
        throw new Error('declared composer marker U+203A not drawn — re-measure before any descriptor edit');
      }
      return `marker › drawn; placeholder up; footer hints seen: ${ctx.facts.composer_idle.declared_footer_hints_seen.filter((h) => h.seen).map((h) => h.hint).join(',') || 'NONE'}`;
    });

    // 3. THE REAL TURN — needle measurement with the classifier's own
    //    windowed-lowercase semantics, plus the footer titling lifecycle the
    //    0.154.0 recon measured (renaming… spinner → generated title).
    await ctx.probe('real-turn-working-needles', async () => {
      if (authGated) {
        ctx.facts.working_screen_phrases = null; // login-gated, honest null
        ctx.facts.working_footer_hints = null;
        return 'skipped — auth gate';
      }
      drive.write(prompt);
      await new Promise((r) => setTimeout(r, 500));
      drive.write('\r');
      const watch = [...WORKING_NEEDLES, WORKING_NEGATION];
      const seen = new Map(watch.map((n) => [n, false]));
      const hitLines = new Set();
      let renamingObserved = false;
      const deadline = Date.now() + 180000;
      let stable = 0;
      let last = '';
      while (Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, 200));
        const screen = drive.screen();
        if (/renaming\.\.\./i.test(screen)) renamingObserved = true;
        for (const h of phraseHits(screen, watch)) {
          if (!h.observed) continue;
          seen.set(h.needle, true);
          for (const line of screen.split('\n')) {
            if (line.toLowerCase().includes(h.needle)) hitLines.add(line.trim());
          }
        }
        const hash = screen.length + ':' + screen.slice(-200);
        stable = hash === last ? stable + 1 : 0;
        last = hash;
        if (stable >= 25) break; // ~5s unchanged: the turn is over
        if (drive.exitCode !== null) break;
      }
      await new Promise((r) => setTimeout(r, 2000));
      drive.snap('turn-settled');
      const screen = drive.screen();
      ctx.facts.working_screen_phrases = WORKING_NEEDLES.map((n) => ({
        needle: n,
        observed: seen.get(n),
      }));
      ctx.facts.working_screen_negations = {
        declared: [WORKING_NEGATION],
        observed_during_turn: seen.get(WORKING_NEGATION),
      };
      ctx.facts.working_line_sample = [...hitLines].slice(0, 3);
      ctx.facts.turn_reply_ok = screen.includes(sentinel);
      // The footer is the LAST cwd-bearing line with a `·` separator — the
      // banner's `directory:` line also contains the cwd but has no `·`
      // (first-run bug: find() matched the banner and the title read []).
      const footerLine =
        screen
          .split('\n')
          .map((l) => l.trim())
          .filter(Boolean)
          .filter((l) => l.includes(ctx.cwd) && l.includes('·'))
          .pop() ?? null;
      ctx.facts.footer_titling_lifecycle = {
        renaming_spinner_observed: renamingObserved,
        footer_line_settled: footerLine,
        footer_tail_settled: footerLine
          ? footerLine.split('·').map((s) => s.trim()).slice(1)
          : null,
      };
      if (!ctx.facts.turn_reply_ok) {
        throw new Error('no reply on the settled screen — the turn failed, re-run before concluding drift');
      }
      if (!seen.get(WORKING_NEEDLES[0]) && !seen.get(WORKING_NEEDLES[1])) {
        throw new Error('NEITHER declared working needle seen on a successful turn — needle drift, re-measure');
      }
      return `needles ${WORKING_NEEDLES.filter((n) => seen.get(n)).join(' + ')} observed; negation seen: ${seen.get(WORKING_NEGATION)}; renaming-spinner: ${renamingObserved}; footer tail: ${JSON.stringify(ctx.facts.footer_titling_lifecycle.footer_tail_settled)}`;
    });

    // 4. store side-car: find THIS turn's rollout by sentinel-in-content
    //    (the same tree serves every live codex row on the host), then read
    //    session_meta and verify the filename-uuid law.
    let sessionId = null;
    let rolloutPath = null;
    await ctx.probe('store-rollout', async () => {
      const sessionsRoot = path.join(codexHome, 'sessions');
      if (!fs.existsSync(sessionsRoot)) {
        ctx.facts.store = { root_exists: false };
        return 'no ~/.codex/sessions at all';
      }
      const stack = [sessionsRoot];
      let candidates = [];
      while (stack.length) {
        const dir = stack.pop();
        let entries;
        try {
          entries = fs.readdirSync(dir, { withFileTypes: true });
        } catch (_) {
          continue;
        }
        for (const e of entries) {
          const p = path.join(dir, e.name);
          if (e.isDirectory()) stack.push(p);
          else if (/^rollout-.*\.jsonl$/.test(e.name)) candidates.push(p);
        }
      }
      candidates = candidates
        .map((p) => ({ p, m: fs.statSync(p).mtimeMs, size: fs.statSync(p).size }))
        .filter((c) => c.size < 8 * 1024 * 1024)
        .sort((a, b) => b.m - a.m)
        .slice(0, 200); // sentinel scan is content-based; mtime only bounds the scan
      let hit = null;
      for (const c of candidates) {
        try {
          if (fs.readFileSync(c.p, 'utf8').includes(sentinel)) {
            hit = c.p;
            break;
          }
        } catch (_) {
          /* unreadable — skip */
        }
      }
      if (!hit) {
        ctx.facts.store = { root_exists: true, rollout_found: false, scanned: candidates.length };
        return `no rollout under ${homeRel(sessionsRoot)} contains the sentinel`;
      }
      rolloutPath = hit;
      const firstLine = JSON.parse(fs.readFileSync(hit, 'utf8').split('\n')[0]);
      const meta = firstLine.type === 'session_meta' ? firstLine.payload : null;
      sessionId = meta?.session_id ?? meta?.id ?? null;
      const uuidInName = path.basename(hit).match(/-([0-9a-f-]{36})\.jsonl$/)?.[1] ?? null;
      ctx.facts.store = {
        root_exists: true,
        rollout_found: true,
        rollout: homeRel(hit),
        session_meta: meta
          ? {
              session_id: sessionId,
              cwd_matches_probe: meta.cwd === ctx.cwd,
              cli_version: meta.cli_version ?? null,
              originator: meta.originator ?? null,
              source: meta.source ?? null,
            }
          : null,
        filename_uuid_matches_session_id: !!(sessionId && uuidInName === sessionId),
        sentinel_hits: (fs.readFileSync(hit, 'utf8').match(new RegExp(sentinel, 'g')) ?? []).length,
      };
      if (!sessionId) return 'rollout found but no session_meta.session_id — schema drift';
      return `rollout ${homeRel(hit)}; session ${sessionId}; cwd-match: ${meta?.cwd === ctx.cwd}; filename-uuid law: ${ctx.facts.store.filename_uuid_matches_session_id}`;
    });

    // 5. title authority is Store — three measured surfaces for the one
    //    thread name: the footer (probe 3), session_index.jsonl, and the
    //    thread catalog sqlite (read through the sqlite3 CLI when present).
    await ctx.probe('title-authority-store', async () => {
      if (authGated || !sessionId) {
        ctx.facts.title = null; // honest null — no session to read
        return 'skipped — no session id';
      }
      let indexName = null;
      const indexPath = path.join(codexHome, 'session_index.jsonl');
      if (fs.existsSync(indexPath)) {
        for (const line of fs.readFileSync(indexPath, 'utf8').split('\n').filter(Boolean)) {
          try {
            const e = JSON.parse(line);
            if (e.id === sessionId) indexName = e.thread_name ?? indexName;
          } catch (_) {
            /* torn line — skip */
          }
        }
      }
      let catalogTitle = null;
      let catalogError = null;
      const dbPath = path.join(codexHome, 'sqlite', 'codex-dev.db');
      if (fs.existsSync(dbPath)) {
        try {
          const out = execFileSync(
            'sqlite3',
            [dbPath, `SELECT display_title FROM local_thread_catalog WHERE thread_id='${sessionId}';`],
            { encoding: 'utf8', timeout: 10000 },
          ).trim();
          catalogTitle = out || null;
        } catch (e) {
          catalogError = String(e.message).split('\n')[0];
        }
      } else {
        catalogError = `no ${homeRel(dbPath)} (reader fails open, per the descriptor)`;
      }
      const footerTail = ctx.facts.footer_titling_lifecycle?.footer_tail_settled ?? [];
      const footerTitle = footerTail.length ? footerTail[footerTail.length - 1] : null;
      ctx.facts.title = {
        authority: 'Store (owner titling law, second half 2026-09-05)',
        footer_title: footerTitle,
        session_index_thread_name: indexName,
        thread_catalog_display_title: catalogTitle,
        catalog_read_error: catalogError,
      };
      const named = [indexName, catalogTitle, footerTitle].filter(Boolean);
      return named.length
        ? `title surfaces: index=${JSON.stringify(indexName)} catalog=${JSON.stringify(catalogTitle)} footer=${JSON.stringify(footerTitle)}`
        : 'no surface carries a thread name yet — titling may be lazy (a heavier turn names it)';
    });

    // 6. flag surface: codex --help re-asked of the installed binary — the
    //    declared model/approval/sandbox vocabulary and the resume
    //    subcommand must still spell that way.
    await ctx.probe('help-flag-surface', async () => {
      const help = new ctx.Drive({
        command: bin,
        args: ['--help'],
        cwd: ctx.cwd,
        artifactsDir: ctx.artifactsDir,
        label: 'codex-help',
      });
      await help.waitFor(() => /usage|options|commands/i.test(help.screen()) || help.exitCode !== null, 20000);
      await new Promise((r) => setTimeout(r, 800));
      help.snap('help');
      const screen = help.screen();
      help.dispose();
      const line = (re) => screen.split('\n').find((l) => re.test(l))?.trim() ?? null;
      ctx.facts.help_flags = {
        model: line(/--model/),
        ask_for_approval: line(/--ask-for-approval/),
        sandbox: line(/--sandbox/),
        bypass: line(/--dangerously-bypass-approvals-and-sandbox/),
        bypass_hook_trust: line(/--dangerously-bypass-hook-trust/),
        resume_subcommand: line(/\bresume\b/),
      };
      if (!ctx.facts.help_flags.model || !ctx.facts.help_flags.resume_subcommand) {
        throw new Error('declared flag surface lost — --model or the resume subcommand missing from --help');
      }
      return `model: ${JSON.stringify(ctx.facts.help_flags.model)}; resume: ${JSON.stringify(ctx.facts.help_flags.resume_subcommand)}`;
    });

    // 7. resume id-reuse: `codex resume <id>` must rederive the prior
    //    transcript AND keep writing to the SAME rollout file (class-A
    //    store-authoritative law) — proven with a follow-up turn whose
    //    marker lands in the original file.
    let resumedDrive = null;
    await ctx.probe('resume-id-reuse', async () => {
      if (!sessionId || !rolloutPath) {
        ctx.facts.resume = null; // honest null — nothing to resume
        return 'skipped — no session id from the store probe';
      }
      const sizeBefore = fs.statSync(rolloutPath).size;
      await drive.killChild(); // end drive 1 the way a crash would
      await new Promise((r) => setTimeout(r, 1500));
      resumedDrive = new ctx.Drive({
        command: bin,
        args: ['resume', sessionId],
        cwd: ctx.cwd,
        artifactsDir: ctx.artifactsDir,
        label: 'codex-resume',
      });
      ctx.drive = resumedDrive;
      const painted = await resumedDrive.waitFor(() => resumedDrive.screen().trim().length > 0, 60000);
      if (!painted) throw new Error('resumed codex never painted');
      if (/do you trust the contents of this directory/i.test(resumedDrive.screen())) {
        resumedDrive.write('\r');
        await new Promise((r) => setTimeout(r, 3000));
      }
      // content_rederives_on_resume: the sentinel turn must come back.
      const rederived = await resumedDrive.waitFor(
        () => resumedDrive.screen().includes(sentinel),
        60000,
        500,
      );
      await new Promise((r) => setTimeout(r, 2000));
      resumedDrive.snap('resumed');
      const marker = `RESUMED-${sentinel}`;
      resumedDrive.write(`Reply with exactly: ${marker}`);
      await new Promise((r) => setTimeout(r, 500));
      resumedDrive.write('\r');
      const replied = await resumedDrive.waitFor(() => resumedDrive.screen().includes(marker), 120000, 400);
      let stable = 0;
      let last = '';
      const dl = Date.now() + 60000;
      while (Date.now() < dl) {
        await new Promise((r) => setTimeout(r, 300));
        const s = resumedDrive.screen();
        const h = s.length + ':' + s.slice(-200);
        stable = h === last ? stable + 1 : 0;
        last = h;
        if (stable >= 15) break;
      }
      const sizeAfter = fs.existsSync(rolloutPath) ? fs.statSync(rolloutPath).size : 0;
      let markerInSameFile = false;
      try {
        markerInSameFile = replied && fs.readFileSync(rolloutPath, 'utf8').includes(marker);
      } catch (_) {
        /* unreadable — leave false */
      }
      ctx.facts.resume = {
        resumed_session_id: sessionId,
        rederived: !!rederived,
        follow_up_replied: !!replied,
        same_rollout_grew: sizeAfter > sizeBefore,
        marker_in_same_rollout: markerInSameFile,
        rollout_bytes_before: sizeBefore,
        rollout_bytes_after: sizeAfter,
      };
      if (!rederived) return 'resumed but the prior transcript never rederived';
      return markerInSameFile
        ? `rederived ok; follow-up turn APPENDED to the same rollout (${sizeBefore}→${sizeAfter} bytes) — id reuse proven`
        : `rederived ok, but the follow-up marker is NOT in the original rollout (grew: ${sizeBefore}→${sizeAfter}) — store-id drift, re-measure`;
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
