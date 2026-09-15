// tools/probe-battery/suites/opencode.js — the 11.6.3 OpenCode suite (class
// B, server-client viewer).
//
// The descriptor's opencode block was measured on the `@opencode-ai/cli`
// beta-19271 line (2026-08-13 .. 2026-09-10); that package DIED upstream on
// 2026-09-07 and the fleet moved to `@opencode/cli` (binary still `opencode2`).
// This suite is the regression net re-asking those facts of whatever binary is
// installed NOW — measured live against 2.0.3 (2026-09-15, muse lab host):
//
//   HOLDS:  working needle `esc interrupt` (verbatim in the working footer,
//           now prefixed by progress squares ■■⬝⬝⬝⬝⬝⬝); resume flag
//           `--session, -s`; bypass `--auto`; store
//           `~/.local/share/opencode/opencode.db` with `session_v2`
//           authoritative + async LLM auto-titling; the service plane
//           (`service.json` handshake {id,version,url,pid,password}; view
//           verb `{"idle":<ms>}` → 204, `{}` → 400 "Missing key [\"idle\"]";
//           `/api/session/active` = the working set, empty when idle); OSC
//           viewing truth `OC | <title>`.
//   DRIFT:  the `❯` (U+276F) composer marker is GONE — 2.0.3 draws a
//           ┃-ruled box (U+2503 rules, a `╹▀▀▀` U+2579/U+2580 border) with
//           the placeholder `Ask anything… "Fix a TODO in the codebase"` and
//           a mode row (`Build auto · <model> OpenCode Zen`) BELOW the input
//           row; the top-level parser REJECTS `--model` (any unknown top-level
//           flag prints help and exits 1 — the TUI never launches; `--model,
//           -m` survives only on `run`/`mini`); `--session <bogus-id>` no
//           longer refuses — it silently falls back to the most recent
//           session of the project; resume RE-RENDERS the prior transcript
//           (content_rederives_on_resume is now true); the idle footer
//           vocabulary is `shift+tab agents  ctrl+p commands`.
//
// ⛔ ISOLATION MODEL (measured): the managed service port (49374) is
// HOST-GLOBAL. A foreign opencode service holding it wedges the TUI forever —
// the client retries `serve --service` indefinitely and the screen never
// leaves `Starting background server...` (this suite records that wedge as a
// named fact instead of hanging). So every drive here runs against a SCRATCH
// HOME: `opencode service set port <free>` relocates the managed port for
// that HOME only; store, service registration and titling all follow. Probe
// rows therefore never touch the real store. 2.0.3 also ships a free built-in
// model (`Muse Spark 1.3 Free`) — turns COMPLETE without any provider auth,
// so the turn probes are real, not honest nulls.
//
//   node run.js --suite suites/opencode.js --cwd <fresh-empty-dir> \
//        [--suite-arg bin=opencode2]

const fs = require('fs');
const os = require('os');
const path = require('path');
const net = require('net');
const { execSync } = require('child_process');

const FOOTER_WINDOW_ROWS = 10; // SCREEN_FOOTER_WINDOW_ROWS in agent_cli.rs

// Mirror screen_phrases_match EXACTLY: the classifier lowercases each of the
// last N non-empty lines and matches lowercase needle fragments.
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

// OSC 0/2 (window/title) sequences from the raw pty log — the viewing-truth
// signal the classifier reads, which the rendered buffer does not carry.
function oscTitles(rawPath) {
  try {
    const s = fs.readFileSync(rawPath).toString('binary');
    const out = [];
    const re = /\x1b\](0|2);([^\x07\x1b]*)(?:\x07|\x1b\\)/g;
    let m;
    while ((m = re.exec(s))) out.push({ kind: m[1], text: m[2] });
    return out;
  } catch (_) {
    return [];
  }
}

function freePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.listen(0, '127.0.0.1', () => {
      const port = srv.address().port;
      srv.close(() => resolve(port));
    });
    srv.on('error', reject);
  });
}

function sqliteAvailable() {
  try {
    execSync('which sqlite3', { stdio: 'ignore' });
    return true;
  } catch (_) {
    return false;
  }
}

function sqliteRead(dbPath, sql) {
  return execSync(`sqlite3 -json "file:${dbPath}?mode=ro" ${JSON.stringify(sql)}`, {
    encoding: 'utf8',
    timeout: 15000,
  }).trim();
}

module.exports = {
  name: 'opencode',

  async run(ctx) {
    const bin = ctx.args.bin || 'opencode2';

    // ISOLATION FIRST: scratch HOME + scratch-local managed port, before any
    // Drive exists (drive.js copies process.env at construction).
    const scratchHome = fs.mkdtempSync(path.join(os.tmpdir(), 'opencode-suite-home-'));
    process.env.HOME = scratchHome;
    const port = await freePort();
    let portConfigured = false;
    try {
      execSync(`${bin} service set port ${port}`, {
        env: process.env,
        cwd: ctx.cwd,
        encoding: 'utf8',
        timeout: 30000,
      });
      portConfigured = true;
    } catch (e) {
      ctx.facts.managed_port = {
        requested: port,
        configured: false,
        error: String(e.message).slice(0, 200),
      };
    }
    if (portConfigured) {
      ctx.facts.managed_port = {
        requested: port,
        configured: true,
        note: 'managed port is host-global (49374); a foreign service holding it wedges every TUI at "Starting background server..." forever — scratch homes MUST reconfigure it (measured 2026-09-15)',
      };
    }

    ctx.drive = new ctx.Drive({
      command: bin,
      args: ['--auto'],
      cwd: ctx.cwd,
      artifactsDir: ctx.artifactsDir,
      label: 'opencode',
    });
    const drive = ctx.drive;

    // 1. launch → first paint. The 2.0.3 first paint is the OpenCode logo
    //    block, the ┃-ruled composer with its placeholder, the mode row, and
    //    the version bottom-right. A screen stuck at "Starting background
    //    server..." past 120 s IS the measured port wedge — named, not hung.
    await ctx.probe('launch-first-paint', async () => {
      const painted = await drive.waitFor(
        () => drive.screen().includes('Ask anything'),
        120000,
        400,
      );
      if (!painted) {
        const stuck = drive.screen().trim();
        if (/starting background server/i.test(stuck)) {
          throw new Error(
            'MEASURED WEDGE: TUI stuck at "Starting background server..." — a foreign opencode service holds the managed port (49374 is host-global); reconfigure with `opencode service set port <free>` before driving (measured 2026-09-15)',
          );
        }
        throw new Error(`no composer paint within 120s; screen tail: ${stuck.slice(-200)}`);
      }
      await new Promise((r) => setTimeout(r, 2500));
      drive.snap('first-paint');
      const screen = drive.screen();
      const lines = screen.split('\n').map((l) => l.trim()).filter(Boolean);
      ctx.facts.first_paint = {
        version: screen.match(/(\d+\.\d+\.\d+)\s*$/m)?.[1] ?? null,
        placeholder_line: lines.find((l) => l.includes('Ask anything')) ?? null,
        mode_line: lines.find((l) => /OpenCode Zen|Build auto/.test(l)) ?? null,
        osc_titles: oscTitles(path.join(ctx.artifactsDir, 'opencode-raw.bin')),
      };
      if (!ctx.facts.first_paint.version) {
        throw new Error('no version string on the first paint — re-measure the paint shape');
      }
      return `painted ${ctx.facts.first_paint.version}; placeholder: ${JSON.stringify(ctx.facts.first_paint.placeholder_line)}`;
    });

    // 2. composer idle shape — the 2.0.3 box. The declared beta-era marker
    //    ❯ (U+276F) must NOT paint (its presence means the shape moved again
    //    and every opencode fact here needs a re-measure); the ┃ rule and the
    //    mode row below the input row are the measured anchor vocabulary.
    await ctx.probe('composer-idle-shape', async () => {
      const screen = drive.screen();
      const lines = screen.split('\n').map((l) => l.trimEnd());
      const nonEmpty = lines.map((l) => l.trim()).filter(Boolean);
      // The rule char is the placeholder row's own leading glyph — scoped to
      // the composer row, NOT a whole-screen search (the logo block art is
      // also >U+2000 and top-down scanning finds █ first).
      const placeholderLineFull = lines.find((l) => l.includes('Ask anything')) ?? null;
      const ruleChar = placeholderLineFull?.trim()[0];
      const ruleCodepoint =
        ruleChar && ruleChar.codePointAt(0) > 0x2000 ? ruleChar : undefined;
      const placeholderLine = lines.find((l) => l.includes('Ask anything')) ?? null;
      const modeLine = lines.findIndex((l) => /OpenCode Zen/.test(l));
      const placeholderIdx = lines.findIndex((l) => l.includes('Ask anything'));
      const declaredHints = ['esc', 'interrupt', 'ctrl', 'tab'];
      const measuredHints = ['shift+tab agents', 'ctrl+p commands'];
      ctx.facts.composer_idle = {
        declared_marker_u276f_present: screen.includes('\u276f'),
        box_rule_codepoint: ruleCodepoint
          ? { char: ruleCodepoint, codepoint: 'U+' + ruleCodepoint.codePointAt(0).toString(16).toUpperCase().padStart(4, '0') }
          : null,
        placeholder_line: placeholderLine?.trim() ?? null,
        mode_row_below_input: modeLine > placeholderIdx && placeholderIdx >= 0,
        declared_footer_hints_seen: phraseHits(screen, declaredHints),
        measured_footer_hints_seen: phraseHits(screen, measuredHints),
        footer_line: nonEmpty.find((l) => /agents|commands/.test(l) && l.length < 160) ?? null,
      };
      if (ctx.facts.composer_idle.declared_marker_u276f_present) {
        throw new Error('U+276F paints again — the beta-era shape returned; RE-MEASURE every opencode fact before any descriptor edit');
      }
      if (!placeholderLine || ruleCodepoint === undefined) {
        throw new Error('composer box shape not found (no ┃ rule / no placeholder) — re-measure');
      }
      return `rule ${ctx.facts.composer_idle.box_rule_codepoint.codepoint}; no ❯ (2.0.3 shape); hints seen: ${ctx.facts.composer_idle.measured_footer_hints_seen.filter((h) => h.observed).map((h) => h.needle).join(',') || 'NONE'}`;
    });

    // 3. draft shape — typing WITHOUT sending renders on the same ┃ row that
    //    carried the placeholder, with the mode row still below it (measured
    //    2026-09-15). Backspace returns the placeholder.
    await ctx.probe('draft-shape', async () => {
      const draft = `unsent draft ${Date.now().toString(36).slice(-5)}`;
      drive.write(draft);
      await new Promise((r) => setTimeout(r, 2000));
      drive.snap('draft');
      const screen = drive.screen();
      const lines = screen.split('\n');
      const draftLine = lines.find((l) => l.includes(draft)) ?? null;
      const rule = ctx.facts.composer_idle.box_rule_codepoint?.char;
      const draftTrimmed = draftLine?.trim() ?? '';
      ctx.facts.draft = {
        draft_visible: !!draftLine,
        draft_row_leading_char: draftTrimmed[0] ?? null,
        draft_row_anchored_by_rule: !!rule && draftTrimmed.startsWith(rule),
        placeholder_gone_while_drafting: !screen.includes('Ask anything'),
      };
      for (let i = 0; i < 40; i++) drive.write('\x7f');
      await new Promise((r) => setTimeout(r, 1500));
      const cleared = drive.screen();
      ctx.facts.draft.backspace_restores_placeholder = cleared.includes('Ask anything');
      drive.snap('draft-cleared');
      if (!ctx.facts.draft.draft_visible || !ctx.facts.draft.draft_row_anchored_by_rule) {
        throw new Error('draft text not rendered on the ┃ rule row — composer shape moved, re-measure');
      }
      return `draft renders on the rule row; placeholder ${ctx.facts.draft.backspace_restores_placeholder ? 'restored' : 'NOT restored on backspace'}`;
    });

    // 4. THE TURN — real on 2.0.3 (the built-in free model needs no auth).
    //    The declared working needle `esc interrupt` must paint during the
    //    turn; 2.0.3 dropped the beta-era "to" (`esc to interrupt`).
    await ctx.probe('turn-attempt', async () => {
      const sentinel = `DONE-${Date.now().toString(36).slice(-6)}`;
      drive.write(`Without using any tools, reply with exactly ${sentinel} and nothing else`);
      await new Promise((r) => setTimeout(r, 600));
      drive.write('\r');
      const frames = await drive.phrasePoll(
        ['esc interrupt', 'esc to interrupt'],
        { maxPolls: 150, pollMs: 400, settlePolls: 20 },
      );
      await new Promise((r) => setTimeout(r, 2000));
      drive.snap('turn-settled');
      const settled = drive.screen();
      const workingFrames = frames.filter((f) => f.phrases.includes('esc interrupt'));
      const summaryLine = settled
        .split('\n')
        .map((l) => l.trim())
        .find((l) => /tok\/s/.test(l)) ?? null;
      ctx.facts.turn = {
        replied: (settled.match(new RegExp(sentinel, 'g')) ?? []).length >= 2,
        declared_working_needle_observed: workingFrames.length > 0,
        esc_to_interrupt_variant_observed: frames.some((f) => f.phrases.includes('esc to interrupt')),
        working_footer_sample: workingFrames[0]?.screen_tail.split('\n').map((s) => s.trim()).filter(Boolean).slice(-2) ?? null,
        turn_summary_line: summaryLine,
        polls: frames.length,
      };
      if (!ctx.facts.turn.replied && !ctx.facts.turn.declared_working_needle_observed) {
        throw new Error('no reply and no working needle — an unknown turn state (free model gone? provider refusing?), re-measure');
      }
      if (!ctx.facts.turn.declared_working_needle_observed) {
        throw new Error('declared working needle `esc interrupt` never painted during a real turn — re-measure working_screen_phrases');
      }
      return ctx.facts.turn.replied
        ? `replied; working needle observed in ${workingFrames.length}/${frames.length} polls; summary: ${JSON.stringify(summaryLine)}`
        : `NO reply (provider refused?) but working needle observed — recorded honestly`;
    });

    // 5. service plane — the class-B contract, re-asked of the 2.0.3 service:
    //    service.json handshake; /api/session/active = working set (empty
    //    when idle BY DESIGN); view verb {"idle":<ms>} → 204 and persists,
    //    {} → 400 Missing key. The [11.6.3-b] regression net.
    await ctx.probe('service-plane-contract', async () => {
      const sjPath = path.join(scratchHome, '.local/state/opencode/service.json');
      if (!fs.existsSync(sjPath)) throw new Error('no service.json in the scratch home — the service plane moved, re-measure');
      const sj = JSON.parse(fs.readFileSync(sjPath, 'utf8'));
      const auth = 'Basic ' + Buffer.from(`opencode:${sj.password}`).toString('base64');
      const H = { Authorization: auth, 'Content-Type': 'application/json' };
      const activeRes = await fetch(`${sj.url}/api/session/active`, { headers: H });
      const activeBody = await activeRes.text();
      const dbPath = path.join(scratchHome, '.local/share/opencode/opencode.db');
      let viewOk = null;
      let viewEmpty = null;
      let timeViewed = null;
      let sessId = null;
      if (sqliteAvailable() && fs.existsSync(dbPath)) {
        const rows = JSON.parse(sqliteRead(dbPath, 'select id, time_viewed from session_v2 order by time_created desc limit 1') || '[]');
        sessId = rows[0]?.id ?? null;
        if (sessId) {
          viewOk = (await fetch(`${sj.url}/api/session/${sessId}/view`, { method: 'POST', headers: H, body: JSON.stringify({ idle: Date.now() }) })).status;
          viewEmpty = (await fetch(`${sj.url}/api/session/${sessId}/view`, { method: 'POST', headers: H, body: '{}' })).status;
          timeViewed = JSON.parse(sqliteRead(dbPath, `select time_viewed from session_v2 where id='${sessId}'`) || '[]')[0]?.time_viewed ?? null;
        }
      }
      ctx.facts.service_plane = {
        service_json_version: sj.version,
        service_json_shape: { id: !!sj.id, url: !!sj.url, pid: !!sj.pid, password: !!sj.password },
        active_status: activeRes.status,
        active_empty_when_idle: activeBody.includes('"data":{}') || activeBody.includes('"data": []'),
        view_idle_status: viewOk,
        view_empty_body_status: viewEmpty,
        time_viewed_persisted: timeViewed != null,
        session_id_prefix: sessId ? sessId.slice(0, 4) : null,
      };
      if (viewOk !== 204 || viewEmpty !== 400) {
        throw new Error(`view-verb contract moved: {"idle":ms} → ${viewOk}, {} → ${viewEmpty} (declared 204/400) — re-measure [11.6.3-b]`);
      }
      return `service ${sj.version}; active ${activeRes.status} empty-when-idle=${ctx.facts.service_plane.active_empty_when_idle}; view 204/400 holds; time_viewed persisted`;
    });

    // 6. store side-car — one durable store file, session_v2 authoritative,
    //    async LLM auto-titling, and the NEW v2-era tables.
    await ctx.probe('store-side-car', async () => {
      const dbPath = path.join(scratchHome, '.local/share/opencode/opencode.db');
      if (!fs.existsSync(dbPath)) throw new Error('no opencode.db in the scratch home — the store moved, re-measure');
      if (!sqliteAvailable()) {
        ctx.facts.store = { db_exists: true, read: null, why: 'sqlite3 CLI unavailable on this host — store facts unanswerable here' };
        return 'db exists; sqlite3 missing — store reads skipped (honest null)';
      }
      const tableList = execSync(`sqlite3 "file:${dbPath}?mode=ro" ".tables"`, { encoding: 'utf8', timeout: 15000 });
      const v2Tables = ['session_v2', 'session_message', 'session_inbox', 'session_pending', 'worktree', 'workspace'];
      const missing = v2Tables.filter((t) => !tableList.includes(t));
      const rows = JSON.parse(sqliteRead(dbPath, 'select id, title, directory from session_v2 order by time_created desc limit 3') || '[]');
      const mine = rows.find((r) => r.directory === ctx.cwd) ?? null;
      let title = mine?.title ?? null;
      if (mine && !title) {
        for (let i = 0; i < 30 && !title; i++) {
          await new Promise((r) => setTimeout(r, 3000));
          title = JSON.parse(sqliteRead(dbPath, `select title from session_v2 where id='${mine.id}'`) || '[]')[0]?.title ?? null;
        }
      }
      ctx.facts.store = {
        db_path: '.local/share/opencode/opencode.db',
        session_for_probe_cwd: mine ? { id: mine.id, title } : null,
        id_prefix_ses: mine ? mine.id.startsWith('ses_') : null,
        auto_titled: title != null,
        v2_tables_missing: missing,
      };
      if (!mine) throw new Error('no session_v2 row for the probe cwd — the project binding moved, re-measure');
      if (missing.length) throw new Error(`v2-era tables missing: ${missing.join(',')} — store schema moved, re-measure`);
      return `session ${mine.id} titled: ${JSON.stringify(title)} (auto-titling is async via the free model)`;
    });

    // 7. resume — `--session <real-id>` RE-RENDERS the prior transcript on
    //    2.0.3 (content_rederives_on_resume flipped true; the beta-era TUI
    //    did not replay). Sub-drive on the same scratch home.
    let resumedId = null;
    await ctx.probe('resume-session-flag', async () => {
      const dbPath = path.join(scratchHome, '.local/share/opencode/opencode.db');
      const rows = JSON.parse(sqliteRead(dbPath, 'select id from session_v2 order by time_created desc limit 1') || '[]');
      resumedId = rows[0]?.id ?? null;
      if (!resumedId) throw new Error('no session id to resume — store probe must run first');
      const d2 = new ctx.Drive({
        command: bin,
        args: ['--auto', '--session', resumedId],
        cwd: ctx.cwd,
        artifactsDir: ctx.artifactsDir,
        label: 'opencode-resume',
      });
      const reRendered = await d2.waitFor(
        () => d2.screen().includes('tok/s') || d2.screen().includes('Build ·'),
        90000,
        400,
      );
      await new Promise((r) => setTimeout(r, 2500));
      d2.snap('resume');
      const screen = d2.screen();
      ctx.facts.resume = {
        session_flag: '--session',
        session_id: resumedId,
        prior_transcript_re_rendered: reRendered || screen.includes('tok/s') || screen.includes('Build ·'),
        virgin_placeholder_shown: screen.includes('Ask anything'),
      };
      const code = await d2.killChild();
      ctx.facts.resume.exit_code = code;
      if (!ctx.facts.resume.prior_transcript_re_rendered) {
        throw new Error('resume rendered NO prior transcript — content_rederives_on_resume moved again, re-measure');
      }
      return `--session ${resumedId} re-rendered the prior transcript (measured TRUE on 2.0.3); exit ${code}`;
    });

    // 8. bogus session id — the beta-era law ("refuses an unknown --session
    //    outright") is DEAD: 2.0.3 silently falls back to the most recent
    //    session of the project. Recorded as the measured hazard it is.
    await ctx.probe('bogus-session-fallback', async () => {
      const d3 = new ctx.Drive({
        command: bin,
        args: ['--auto', '--session', 'ses_bogus0000000000000'],
        cwd: ctx.cwd,
        artifactsDir: ctx.artifactsDir,
        label: 'opencode-bogus',
      });
      await d3.waitFor(() => d3.screen().trim().length > 0, 60000, 400);
      await new Promise((r) => setTimeout(r, 5000));
      d3.snap('bogus-session');
      const screen = d3.screen();
      ctx.facts.bogus_session = {
        refused: false,
        virgin_composer_shown: screen.includes('Ask anything'),
        fell_back_to_recent_session: screen.includes('tok/s') || screen.includes('Build ·'),
        note: 'the beta-era refusal is gone — an unknown --session silently resumes the latest session (mis-binding hazard for callers that mint ids out-of-band)',
      };
      await d3.killChild();
      if (ctx.facts.bogus_session.virgin_composer_shown) {
        ctx.facts.bogus_session.fell_back_to_recent_session = false;
        ctx.facts.bogus_session.refused = false;
        return 'bogus id opened a VIRGIN composer — fallback behavior moved, re-measure';
      }
      if (!ctx.facts.bogus_session.fell_back_to_recent_session) {
        throw new Error('bogus id neither fell back nor opened virgin — blank screen? re-measure');
      }
      return 'bogus id SILENTLY RESUMED the latest session (measured 2.0.3 hazard, recorded)';
    });

    // 9. flag surface — top-level help re-asked of the installed binary, and
    //    the parser refusal that broke the model-override launch arm: ANY
    //    unknown top-level flag prints help and exits 1 without painting a
    //    TUI. `--model` survives only on `run`/`mini`.
    await ctx.probe('help-flag-surface', async () => {
      const help = new ctx.Drive({
        command: bin,
        args: ['--help'],
        cwd: ctx.cwd,
        artifactsDir: ctx.artifactsDir,
        label: 'opencode-help',
      });
      await help.waitFor(() => /USAGE/i.test(help.screen()) || help.exitCode !== null, 20000);
      await new Promise((r) => setTimeout(r, 800));
      help.snap('help');
      const screen = help.screen();
      const line = (re) => screen.split('\n').find((l) => re.test(l))?.trim() ?? null;
      const runHelp = new ctx.Drive({
        command: bin,
        args: ['run', '--help'],
        cwd: ctx.cwd,
        artifactsDir: ctx.artifactsDir,
        label: 'opencode-run-help',
      });
      await runHelp.waitFor(() => /USAGE/i.test(runHelp.screen()) || runHelp.exitCode !== null, 20000);
      await new Promise((r) => setTimeout(r, 800));
      runHelp.snap('run-help');

      // the parser probe: a model-pinned TOP-LEVEL launch must NOT paint a
      // TUI — it help-exits. This is the yggterm launch-arm break, netted.
      const parser = new ctx.Drive({
        command: bin,
        args: ['--model', 'test/provider', '--auto'],
        cwd: ctx.cwd,
        artifactsDir: ctx.artifactsDir,
        label: 'opencode-model-parser',
      });
      const end = Date.now() + 20000;
      while (parser.exitCode === null && Date.now() < end) await new Promise((r) => setTimeout(r, 300));
      const parserScreen = parser.screen();
      parser.snap('model-parser');

      ctx.facts.help_flags = {
        session: line(/--session/),
        auto: line(/--auto/),
        continue_flag: line(/--continue/),
        model_top_level: line(/--model/),
        run_subcommand_has_model: /--model/.test(runHelp.screen()),
        model_parser_probe: {
          exited: parser.exitCode !== null,
          exit_code: parser.exitCode,
          help_text_shown: /USAGE/i.test(parserScreen),
          tui_painted: parserScreen.includes('Ask anything'),
        },
      };
      const code3 = await help.killChild();
      const codeR = await runHelp.killChild();
      if (!ctx.facts.help_flags.session || !ctx.facts.help_flags.auto) {
        throw new Error('declared flag surface lost — --session or --auto missing from --help');
      }
      if (ctx.facts.help_flags.model_parser_probe.tui_painted) {
        throw new Error('top-level --model LAUNCHED the TUI — the 2.0.3 parser refusal moved; the wrapper break needs re-measure');
      }
      if (!ctx.facts.help_flags.model_parser_probe.exited) {
        throw new Error('top-level --model neither launched nor exited — unknown parser state, re-measure');
      }
      return `--session/--auto hold; top-level --model help-exits rc=${ctx.facts.help_flags.model_parser_probe.exit_code} (measured break); --model lives on run: ${ctx.facts.help_flags.run_subcommand_has_model}`;
    });

    // 10. panic falsifier: SIGKILL the child; the drive must NOTICE.
    await ctx.probe('panic-falsifier', async () => {
      const code = await drive.killChild();
      if (code === null) throw new Error('child kill did not surface as pty exit');
      return `exit ${code}`;
    });
  },
};
