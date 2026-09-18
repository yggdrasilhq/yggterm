// tools/probe-battery/suites/devin.js — the 11.6.12 devin suite (class C,
// TUI-with-store). FIRST MEASUREMENT pass: the registration (2026-09-15,
// lane/integration/devin) was the declared-unmeasured posture — every
// contract docs-sourced, screen tables empty, store a declared scan gap.
// This suite is the install that pays the measurement debt, live on
// 3000.10.27 (the muse lab host — the only fleet install).
//
// Measured firsts (2026-09-15/16, this suite's exploratory drives):
//   - trust gate on a fresh cwd: `Do you trust the authors of this
//     directory?` / `Yes, trust` / `No, exit`; Enter answers the highlighted
//     row; the grant lands in ~/.local/share/devin/cli/trusted_workspaces.json
//   - composer glyph is U+276D `❭` — NOT the registered U+276F placeholder
//   - working line: `⠙ Thinking · <N>s (esc twice to interrupt)` (braille
//     frames are frames of one live line, muse lesson); the composer
//     placeholder swaps to `Guide Devin while it works` mid-turn
//   - store: ~/.local/share/devin/cli/sessions.db (SQLite WAL; sessions
//     table: id, working_directory, model, agent_mode, title, ...) —
//     session ids are word-word SLUGS (scythe-snowplow); `title` = the
//     first prompt (eager self-titling, Store authority with a real reader)
//   - a session row is written when a turn COMPLETES; an interrupted or
//     turnless session leaves only session_locks/<slug>.lock (content =
//     pid) behind — locks persist after exit and are NOT a live registry
//   - resume: `devin -r <slug>` rederives the history (prompt echo + reply);
//     ctrl+d exits rc 0 with the farewell naming the resume contract
//   - print mode answers (`-p`); in an untrusted dir it cannot show the
//     trust prompt and fails
//
//   node run.js --suite suites/devin.js --cwd /tmp/devin-suite-ws \
//        [--suite-arg bin=devin] [--suite-arg prompt="..."]
//
// The suite makes its OWN fresh workdirs (a fresh dir is untrusted, which
// is what arms the gate), so it never touches owner sessions: devin keys
// sessions by cwd and the suite's dirs are disposable.

const fs = require('fs');
const os = require('os');
const path = require('path');
const { execSync } = require('child_process');

const WORKING_NEEDLES = ['Thinking', 'esc twice to interrupt'];
const COMPOSER_PLACEHOLDER = 'Ask Devin to build features, fix bugs, or work on your code';
const MIDTURN_PLACEHOLDER = 'Guide Devin while it works';
// v3000.10.31 (2026-09-18): the footer is `SWE-1.6 Slow … Press alt+m to
// switch between available models` — the 10.27-era ctrl+v hint no longer paints.
const FOOTER_HINT = 'SWE-1.6 Slow'; // left-side, verbatim case, survives truncation; the RIGHT side rotates (alt+m variant / Shift+Tab permission-cycling variant both measured) // left-side, survives narrow-pty truncation (the right-side alt+m text truncates at 80 cols)
const GATE_QUESTION = 'Do you trust the authors of this directory?';
const GLYPH_CODEPOINT = 0x276d; // ❭ — measured; the registered ❯ U+276F is dead

function freshDir(tag) {
  const dir = path.join(os.tmpdir(), `devin-suite-${tag}-${Date.now()}`);
  fs.mkdirSync(dir, { recursive: true });
  return dir;
}

function sh(cmd, cwd, timeoutMs = 150000) {
  return execSync(cmd, { cwd, timeout: timeoutMs, encoding: 'utf8' });
}

/** Read the sessions table read-only through python3's sqlite3 (WAL-safe). */
function readSessions(home, dir) {
  const db = path.join(home, '.local/share/devin/cli/sessions.db');
  if (!fs.existsSync(db)) return null;
  const script = `
import sqlite3, json
con = sqlite3.connect('file:${db}?mode=ro', uri=True)
con.row_factory = sqlite3.Row
try:
    rows = [dict(r) for r in con.execute(
        'select id, working_directory, title, model, agent_mode from sessions')]
except sqlite3.Error as e:
    print(json.dumps({'error': str(e)})); raise SystemExit
print(json.dumps(rows))
`;
  try {
    return JSON.parse(sh(`python3 - <<'PYEOF'\n${script}\nPYEOF`, dir, 20000));
  } catch (e) {
    return null;
  }
}

module.exports = {
  name: 'devin',

  async run(ctx) {
    const bin = ctx.args.bin || 'devin';
    const prompt = ctx.args.prompt || 'Reply with exactly: OK';
    const home = process.env.HOME || '';
    const trustStore = path.join(home, '.local/share/devin/cli/trusted_workspaces.json');
    let slug = null;

    // ── auth posture first: the suite's facts are real only while the
    // owner's credentials hold. A login screen is a gate, not a drift.
    const dir = freshDir('main');
    const d = new ctx.Drive({
      command: bin,
      args: [],
      cwd: dir,
      artifactsDir: ctx.artifactsDir,
      label: 'devin-fresh',
    });
    ctx.facts.bin = bin;
    ctx.facts.workdir = dir;

    // ── 1. the trust gate arms on a fresh (untrusted) cwd
    const gateSeen = await d.waitFor(() => d.screen().includes(GATE_QUESTION), 45000, 300);
    if (!gateSeen) {
      // Either a login screen (auth lost) or startup drift — capture and
      // answer honestly; the turn/store probes cannot run past this.
      d.snap('no-gate-first-paint');
      const s = d.screen();
      const loginLike = /log in|login|authenticate/i.test(s);
      ctx.facts.trust_gate = null;
      ctx.facts.trust_gate_blocked_reason = loginLike
        ? 'auth gate (login TUI) — credentials lost; turn facts are nulls until auth returns'
        : 'trust gate never painted within 45s — screen captured in artifacts';
      ctx.probe('trust_gate_blocks_programmatic_input', () => {
        throw new Error(ctx.facts.trust_gate_blocked_reason);
      }).catch(() => {});
      d.killChild();
      return;
    }
    d.snap('trust-gate');
    const gateScreen = d.screen();
    ctx.facts.trust_gate = {
      question: GATE_QUESTION,
      options: ['Yes, trust', 'No, exit'].filter((o) => gateScreen.includes(o)),
      shows_path: gateScreen.includes(dir),
      esc_quits: gateScreen.includes('esc'),
    };
    await ctx.probe('trust_gate_blocks_and_records', async () => {
      if (!gateScreen.includes('Yes, trust') || !gateScreen.includes('No, exit')) {
        throw new Error('gate picker options missing from the captured screen');
      }
      // Enter answers the highlighted default row ("Yes, trust").
      d.write('\r');
      const cleared = await d.waitFor(
        () => !d.screen().includes('No, exit'),
        20000,
        300,
      );
      if (!cleared) throw new Error('gate did not clear after Enter');
      d.snap('gate-cleared');
      // The grant is durable in the CLI's own trust store.
      const recorded =
        fs.existsSync(trustStore) && fs.readFileSync(trustStore, 'utf8').includes(dir);
      if (!recorded) throw new Error(`trust grant not recorded in ${trustStore}`);
      return { trust_store: trustStore, granted_path: dir };
    });

    // ── 2. the composer: glyph, placeholder, footer
    const composerSeen = await d.waitFor(
      () => d.screen().includes(COMPOSER_PLACEHOLDER),
      60000,
      300,
    );
    if (!composerSeen) {
      d.snap('no-composer');
      ctx.facts.composer = null;
      ctx.log(`composer placeholder never painted — screen captured`);
      d.killChild();
      return;
    }
    await ctx.probe('composer_glyph_and_footer', async () => {
      await new Promise((r) => setTimeout(r, 1500)); // chrome settle
      d.snap('composer-empty');
      const line = d
        .screen()
        .split('\n')
        .find((l) => l.includes(COMPOSER_PLACEHOLDER));
      if (!line) throw new Error('placeholder line vanished after settle');
      const glyph = line.trim().codePointAt(0);
      if (glyph !== GLYPH_CODEPOINT) {
        throw new Error(
          `composer glyph is U+${glyph.toString(16)} — expected U+276d (❭); ` +
            `the registered U+276f placeholder is falsified if this drifts`,
        );
      }
      const footerHit = d.screen().includes(FOOTER_HINT);
      if (!footerHit) throw new Error(`footer hint "${FOOTER_HINT}" not on screen`);
      ctx.facts.composer = { marker: '❭ U+276D', placeholder: COMPOSER_PLACEHOLDER, footer_hint: FOOTER_HINT };
      return { glyph_codepoint: `U+${glyph.toString(16)}`, footer_hint: FOOTER_HINT };
    });

    // ── 3. a real turn: working line, then the two-esc interrupt
    await ctx.probe('turn_working_phrases_and_interrupt', async () => {
      d.write(prompt);
      await new Promise((r) => setTimeout(r, 1200));
      d.snap('composer-typed');
      d.write('\r');
      const frames = await d.phrasePoll(WORKING_NEEDLES, {
        maxPolls: 90,
        pollMs: 500,
        settlePolls: 10,
      });
      fs.writeFileSync(path.join(ctx.artifactsDir, 'turn-frames.json'), JSON.stringify(frames));
      const hits = new Set();
      for (const f of frames) for (const p of f.phrases) if (p) hits.add(p);
      const sawWorking = hits.has('Thinking');
      const sawInterrupt = hits.has('esc twice to interrupt');
      const sawMidturnPlaceholder = frames.some((f) =>
        (f.screen_tail || '').includes(MIDTURN_PLACEHOLDER),
      );
      ctx.facts.working = {
        needles_seen: [...hits],
        // The model is often slower than the poll window; the working line's
        // presence is the fact, the reply is not required by this probe.
        saw_working_line: sawWorking,
        interrupt_contract: 'esc twice to interrupt',
      };
      if (!sawWorking || !sawInterrupt) {
        d.snap('no-working-line');
        throw new Error(
          `working needles missing (Thinking=${sawWorking}, esc-twice=${sawInterrupt})`,
        );
      }
      // The measured interrupt: TWO DISTINCT escape presses (one write of
      // "\x1b\x1b" reads as a single event — measured 2026-09-16: the turn
      // kept Thinking through it). Working line must go away.
      d.write('\x1b');
      await new Promise((r) => setTimeout(r, 300));
      d.write('\x1b');
      const stopped = await d.waitFor(
        () => !d.screen().includes('esc twice to interrupt'),
        45000,
        400,
      );
      d.snap('after-interrupt');
      if (!stopped) throw new Error('working line survived two distinct esc presses');
      const composerBack = await d.waitFor(
        () => d.screen().includes(COMPOSER_PLACEHOLDER),
        20000,
        400,
      );
      return {
        working: [...hits],
        midturn_placeholder_seen: sawMidturnPlaceholder,
        composer_placeholder_back: composerBack,
      };
    });

    // ── exit the interactive drive; the farewell names the resume contract
    d.write('\x04');
    await d.waitFor(() => d.exitCode !== null, 15000, 200);
    d.snap('after-ctrl-d');
    const farewell = d.screen();
    ctx.facts.exit = {
      farewell_names_resume: farewell.includes('Resume this session with'),
      graceful: d.exitCode === 0,
    };
    d.dispose();

    // ── 4. print mode answers in a trusted dir
    await ctx.probe('print_mode_answers', async () => {
      const out = sh(`${bin} -p '${prompt.replace(/'/g, `'\\''`)}'`, dir, 150000);
      if (!out.includes('OK')) throw new Error(`print reply missing "OK": ${out.slice(0, 200)}`);
      ctx.facts.print_mode = { works: true, reply_excerpt: out.trim().slice(0, 80) };
      return out.trim().slice(0, 80);
    });

    // ── 5. the store: slug id, cwd row, eager title
    await ctx.probe('session_store_row_and_slug', async () => {
      const rows = readSessions(home, dir);
      if (rows === null) throw new Error('sessions.db missing or unreadable');
      const mine = rows.filter((r) => r.working_directory === dir);
      if (!mine.length) throw new Error(`no sessions row for ${dir} after a completed turn`);
      const row = mine[0];
      slug = row.id;
      const slugShape = /^[a-z]+(-[a-z]+)+$/.test(row.id);
      if (!slugShape) throw new Error(`session id "${row.id}" is not the measured word-word slug shape`);
      if (row.title !== prompt) {
        throw new Error(`store title "${row.title}" != first prompt "${prompt}" (eager titling drift)`);
      }
      ctx.facts.store = {
        db: '~/.local/share/devin/cli/sessions.db',
        id_shape: 'word-word slug',
        id: row.id,
        title_equals_first_prompt: true,
        model: row.model,
        agent_mode: row.agent_mode,
      };
      return { id: row.id, title: row.title, model: row.model, agent_mode: row.agent_mode };
    });

    // ── 6. `devin list --format json` is the cwd-scoped machine surface
    await ctx.probe('list_json_reports_the_session', async () => {
      if (!slug) throw new Error('skipped: no store row');
      const out = sh(`${bin} list --format json`, dir, 30000);
      const sessions = JSON.parse(out);
      const mine = sessions.find((s) => s.id === slug);
      if (!mine) throw new Error(`list --format json lacks ${slug}`);
      if (mine.title !== prompt) throw new Error('list title != first prompt');
      ctx.facts.list_json = { fields: Object.keys(mine).sort(), cwd_scoped: true };
      return { id: mine.id, title: mine.title };
    });

    // ── 7. resume rederives the history through a real pty
    await ctx.probe('resume_rederives_history', async () => {
      if (!slug) throw new Error('skipped: no store row');
      const r = new ctx.Drive({
        command: bin,
        args: ['-r', slug],
        cwd: dir,
        artifactsDir: ctx.artifactsDir,
        label: 'devin-resume',
      });
      const painted = await r.waitFor(
        () => r.screen().includes(prompt) && r.screen().includes(COMPOSER_PLACEHOLDER),
        60000,
        400,
      );
      await new Promise((res) => setTimeout(res, 1500));
      r.snap('resumed');
      if (!painted) {
        r.killChild();
        throw new Error('resumed session never painted the echoed prompt + composer');
      }
      r.write('\x04');
      await r.waitFor(() => r.exitCode !== null, 15000, 200);
      const farewell = r.screen().includes(`devin -r ${slug}`);
      ctx.facts.resume = {
        rederives_history: true,
        farewell_names_slug: farewell,
        exit_code: r.exitCode,
      };
      r.snap('resume-exit');
      r.dispose();
      if (!farewell) throw new Error('farewell did not name the resume contract');
      return { slug, farewell_named: farewell };
    });

    // ── 8. print mode refuses an untrusted dir (cannot show the gate)
    await ctx.probe('print_mode_refuses_untrusted', async () => {
      const fresh = freshDir('untrusted');
      let failed = false;
      let output = '';
      try {
        output = sh(`${bin} -p 'ping'`, fresh, 60000);
      } catch (e) {
        failed = true;
        output = String((e.stdout || '') + (e.stderr || ''));
      }
      ctx.facts.print_mode_untrusted = {
        refuses: failed,
        output_excerpt: output.trim().slice(0, 200),
      };
      // Measured 2026-09-16: "Error: Refusing to run in an untrusted
      // workspace: <path>" + the interactive-start/config guidance.
      const NEEDLE = 'Refusing to run in an untrusted workspace';
      if (!failed) throw new Error('print mode ran in an untrusted dir — trust posture drift');
      if (!output.includes(NEEDLE)) {
        throw new Error(`refusal does not carry the measured needle "${NEEDLE}": ${output.slice(0, 200)}`);
      }
      return output.trim().slice(0, 160);
    });
  },
};
