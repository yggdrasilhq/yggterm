// tools/probe-battery/suites/mimo.js — the 11.6.13 mimo intake suite (class B
// candidate: Xiaomi's MiMo Code, the `mimo` command, npm @mimo-ai/cli, MIT —
// an OpenCode fork with a diverged surface: `serve`/`attach`, `session list`,
// `db`, `export`/`import`, `providers`). INTAKE POSTURE (2026-09-26, the muse
// lab host, 0.1.15): registered alongside this suite; real turns are
// CREDENTIAL-GATED (`mimo run` answers `Invalid API Key`), so turn-dependent
// fields (working phrases, eager titling live) stay honest nulls until the
// owner drops a key. Measured WITHOUT auth:
//   - store: ~/.local/share/mimocode/mimocode.db (SQLite WAL) — FIRST LAUNCH
//     runs a one-time migration AND auto-imports Claude Code sessions
//     (`claude_import` table; `mimo session import-claude` is the manual
//     twin), so the store holds rows mimo never ran — a scanner must not
//     read them as mimo sessions' births. session ids are `ses_…` (the
//     opencode law, 3rd instance); the session table carries title_source
//     ('fallback'|'generated'|'user') + title_revision — title authority is
//     first-class in the store.
//   - per-instance locks: ~/.local/state/mimocode/locks/<sha1>.lock/ with
//     heartbeat + meta.json (NOT slug-named; hash of something instance-y).
//   - launch flags measured off --help: top-level `-m/--model provider/model`
//     (opencode 2.x LOST its top-level model flag; the fork KEPT one),
//     `-s/--session <id>`, `-c/--continue`, `--fork`, `--trust` (skip the
//     workspace trust prompt — a trust gate exists), `--yolo`
//     (= --dangerously-skip-permissions), `--never-ask`, `--prompt`,
//     `--agent`, and the server quartet --port/--hostname/--mdns/--cors.
//
//   node run.js --suite suites/mimo.js --cwd /tmp/mimo-suite-ws \
//        [--suite-arg bin=/home/pi/.local/bin/mimo]
//
// The suite only READS the store; imported sessions are owner data and are
// never deleted or modified.

const fs = require('fs');
const os = require('os');
const path = require('path');
const { execSync } = require('child_process');

function freshDir(tag) {
  const dir = path.join(os.tmpdir(), `mimo-suite-${tag}-${Date.now()}`);
  fs.mkdirSync(dir, { recursive: true });
  return dir;
}

function sh(cmd, cwd, timeoutMs = 30000) {
  return execSync(cmd, { cwd, timeout: timeoutMs, encoding: 'utf8' });
}

function readSessionRows(db) {
  if (!fs.existsSync(db)) return null;
  const script = `
import sqlite3, json
con = sqlite3.connect('file:${db}?mode=ro', uri=True)
con.row_factory = sqlite3.Row
rows = [dict(r) for r in con.execute(
    'select id, directory, title, title_source from session order by time_created')]
print(json.dumps(rows))
`;
  return JSON.parse(sh(`python3 - <<'PYEOF'\n${script}\nPYEOF`, path.dirname(db), 20000));
}

module.exports = {
  name: 'mimo',

  async run(ctx) {
    const bin = ctx.args.bin || 'mimo';
    ctx.facts.bin = bin;
    ctx.facts.version = sh(`${bin} --version`, '/tmp').trim();

    const db = path.join(process.env.HOME || '', '.local/share/mimocode/mimocode.db');
    const before = readSessionRows(db);
    ctx.facts.store = {
      db,
      rows_before_launch: before ? before.length : null,
      ses_prefix: before && before[0] ? before[0].id.startsWith('ses_') : null,
      title_source_column: before && before[0] ? 'title_source' in before[0] : null,
    };

    // ── 1. auth posture on the non-TUI runner (credential-gated turns)
    const runDir = freshDir('auth');
    let authRefusal = null;
    try {
      sh(`${bin} run "Reply with exactly: OK"`, runDir, 60000);
    } catch (e) {
      authRefusal = `${e.stdout || ''}${e.stderr || ''}`;
    }
    ctx.facts.auth_posture = {
      turn_runs_without_auth: false,
      refusal_named: /Invalid API Key/i.test(authRefusal || ''),
      refusal_text: (authRefusal || '').slice(0, 200),
    };

    // ── 2. fresh-cwd TUI paint: trust gate first, else the composer
    const dir = freshDir('main');
    const d = new ctx.Drive({
      command: bin,
      args: [],
      cwd: dir,
      artifactsDir: ctx.artifactsDir,
      label: 'mimo-fresh',
    });
    ctx.facts.workdir = dir;

    const gateSeen = await d.waitFor(
      () => /trust|Trust/.test(d.screen()) || /›|❯|┃|❭/.test(d.screen()),
      45000,
      300
    );
    if (!gateSeen) {
      d.snap('no-paint-first-45s');
      ctx.facts.trust_gate = { seen: false, note: 'neither gate nor glyph painted in 45s' };
    } else {
      const s = d.screen();
      d.snap('first-paint');
      const gateQuestion = s.match(/.*[Tt]rust.*/g) || [];
      ctx.facts.trust_gate = {
        seen: gateQuestion.length > 0,
        lines: gateQuestion.slice(0, 4),
      };
      ctx.facts.first_paint = s.split('\n').slice(0, 12);
    }

    // If a trust gate armed, answer the highlighted row (Enter) and watch
    // the composer arrive; otherwise the paint above is already the composer.
    if (ctx.facts.trust_gate.seen) {
      d.write('\r');
      await d.waitFor(() => /›|❯|┃|❭/.test(d.screen()), 30000, 300);
      d.snap('post-gate');
      ctx.facts.post_gate_paint = d.screen().split('\n').slice(0, 14);
    }

    // ── 3. composer glyph + footer on the settled idle paint
    const settled = d.screen();
    const glyphCandidates = [];
    for (const ch of new Set(settled)) {
      const cp = ch.codePointAt(0);
      if (cp >= 0x2000 && cp <= 0x2bff) glyphCandidates.push(`U+${cp.toString(16)} ${ch}`);
    }
    ctx.facts.composer = {
      glyph_candidates: glyphCandidates,
      footer_lines: settled.split('\n').filter((l) => /ctrl|alt|esc|to (select|switch|send|exit)/i.test(l)).slice(0, 5),
      model_line: (settled.match(/.*mimo[- ]?v.*/g) || []).slice(0, 3),
    };
    d.snap('idle-settled');

    // ── 4. did a mere LAUNCH write a session row? (devin: only on turn
    // completion; opencode: on first message — measured, not assumed)
    await new Promise((r) => setTimeout(r, 3000));
    const after = readSessionRows(db);
    ctx.facts.store.rows_after_launch = after ? after.length : null;
    ctx.facts.store.launch_wrote_row = before && after ? after.length > before.length : null;

    await d.killChild();
    d.dispose();

    // ── 5. resume rederive on an IMPORTED session (real history, no auth):
    // `-s <id>` should repaint the transcript — the resume contract measured
    // without a credential.
    if (before && before.length) {
      const target = before[before.length - 1];
      const r = new ctx.Drive({
        command: bin,
        args: ['-s', target.id],
        cwd: target.directory && fs.existsSync(target.directory) ? target.directory : dir,
        artifactsDir: ctx.artifactsDir,
        label: 'mimo-resume',
      });
      const painted = await r.waitFor(() => r.screen().length > 200, 45000, 300);
      r.snap(painted ? 'resume-paint' : 'resume-no-paint');
      ctx.facts.resume = {
        session_id: target.id,
        painted,
        head: r.screen().split('\n').slice(0, 10),
        gate_seen: /trust|Trust/.test(r.screen()),
      };
      await r.killChild();
      r.dispose();
    }
  },
};
