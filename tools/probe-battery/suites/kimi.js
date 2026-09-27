// tools/probe-battery/suites/kimi.js — the 11.6.6 kimi suite (class C,
// TUI-with-store), REWRITTEN for the Kimi Code CLI [11.175].
//
// The vendor killed kimi-cli: the `kimi` binary on PATH may be the uv 1.52.0
// DEPRECATION SHIM, whose every interactive launch prints the deprecation
// notice and RUNS the code.kimi.com installer (network + binary replace —
// measured; it updated the muse lab host's ~/.kimi-code/bin mid-probe). This
// suite prefers ~/.kimi-code/bin/kimi when it exists and FAILS with a named
// verdict when a drive lands on the installer screen — it never measures a
// different binary by accident.
//
// kimi-code 2.1.1 measured shape (2026-09-27, muse lab host): welcome panel
// with `Session:` EMPTY (ids mint at first turn), a rounded box composer
// `│ >  │` (marker `>` after box trim — no `── input ──` region, no ❯), and
// per-directory trust + one-time migration pickers at startup. This suite
// answers ONLY the directory-trust picker, for its own scratch cwd, logged —
// the [11.175] incident (a probe answering a migration picker migrated a
// host) is why the migration picker aborts the suite instead.
//
//   node run.js --suite suites/kimi.js --cwd <real-workdir> \
//        [--suite-arg bin=~/.kimi-code/bin/kimi]
//
// Store note: kimi-code buckets sessions per workdir under
// ~/.kimi-code/sessions/wd_<basename>_<sha256(cwd)[0..12]>/<session_<uuid>/>;
// a REFUSED turn creates nothing (the store-absence probe pins
// id_assigned_at_birth:false). Working phrases + resume replay stay
// login-gated and are reported as the nulls they are.

const fs = require('fs');
const os = require('os');
const path = require('path');
const crypto = require('crypto');

module.exports = {
  name: 'kimi',

  async run(ctx) {
    let bin = ctx.args.bin || path.join(os.homedir(), '.kimi-code', 'bin', 'kimi');
    if (bin.startsWith('~')) bin = path.join(os.homedir(), bin.slice(1));
    if (!fs.existsSync(bin)) bin = ctx.args.bin || 'kimi'; // PATH fallback: the shim hazard
    ctx.facts.bin = bin;
    ctx.drive = new ctx.Drive({
      command: bin,
      args: [],
      cwd: ctx.cwd, // real cwd: kimi-code buckets its store off the workdir
      artifactsDir: ctx.artifactsDir,
      label: 'kimi-code',
    });
    const drive = ctx.drive;

    // 1. gate classification — BEFORE anything is typed. Installer screen =>
    //    the PATH binary is the shim, fail named. Migration picker => the
    //    host needs a deliberate `migrate`, never answered here. Trust picker
    //    => answered for THIS scratch cwd only, logged (provisioning, not
    //    measurement — the pickers the suite measures are the ones it cannot
    //    get past honestly).
    await ctx.probe('gate-classification', async () => {
      const painted = await drive.waitFor(() => drive.screen().trim().length > 0, 60000);
      if (!painted) throw new Error('no paint within 60s');
      await new Promise((r) => setTimeout(r, 3000));
      let screen = drive.screen();
      drive.snap('first-paint');
      if (/kimi-cli is no longer maintained/i.test(screen)) {
        ctx.facts.gate = { class: 'deprecation-shim-installer' };
        throw new Error(
          'PATH kimi is the uv deprecation SHIM — the drive landed on the installer, not a TUI; ' +
            'point --suite-arg bin=~/.kimi-code/bin/kimi or retire the shim',
        );
      }
      if (/Migrate this data to kimi-code\?/i.test(screen)) {
        ctx.facts.gate = { class: 'migration-picker' };
        drive.snap('migration-picker');
        throw new Error(
          'one-time kimi-cli migration picker on screen — the suite NEVER answers it; ' +
            'run kimi-code migrate deliberately, then re-drive',
        );
      }
      if (/Trust this folder\?/i.test(screen)) {
        drive.snap('trust-picker');
        drive.write('\r'); // cursor rests on "Trust this folder"
        const cleared = await drive.waitFor(
          () => !/Trust this folder\?/i.test(drive.screen()),
          15000,
        );
        await new Promise((r) => setTimeout(r, 2500));
        drive.snap('after-trust');
        if (!cleared) throw new Error('trust picker did not clear after answering');
        ctx.facts.gate = {
          class: 'trust-picker-answered',
          scope: 'directory (this scratch cwd only), deliberate, logged',
        };
        screen = drive.screen();
      } else {
        ctx.facts.gate = { class: 'none (folder already trusted)' };
      }
      return `gate: ${ctx.facts.gate.class}`;
    });

    // 2. welcome facts — version/directory lines, and the EMPTY Session line:
    //    the screen-level evidence that ids mint at first turn (the
    //    id_assigned_at_birth flip).
    await ctx.probe('welcome-paint', async () => {
      const screen = drive.screen();
      const lines = screen.split('\n').map((l) => l.trim()).filter((l) => l.length);
      const version = (screen.match(/Version:\s*(\S+)/) || [null, null])[1];
      const directory = (screen.match(/Directory:\s*(\S+)/) || [null, null])[1];
      const sessionLine = lines.find((l) => l.includes('Session:'));
      ctx.facts.welcome = {
        version,
        directory_line: directory,
        session_line: sessionLine ?? null,
        // The panel row is box-ruled (`│  Session:`), so the emptiness check
        // reads AFTER the colon: nothing but chrome = no id at birth.
        session_empty_at_birth: sessionLine
          ? /^Session:\s*(\u{2502})?\s*$/u.test(sessionLine.replace(/^[^S]*Session:/, 'Session:'))
          : null,
        login_hint: lines.find((l) => l.includes('/login'))?.trim() ?? null,
      };
      if (!version) throw new Error('no Version line on the welcome panel');
      return `welcome: kimi-code ${version}, session line ${JSON.stringify(sessionLine ?? '')}`;
    });

    // 3. the composer shape — THE [11.175] measurement. The box input row
    //    `│ >  │`: the marker is `>` after box trim; the 1.50 `── input ──`
    //    region and the declared-`❯` era are both gone (byte-checked).
    await ctx.probe('composer-box-shape', async () => {
      await new Promise((r) => setTimeout(r, 1500));
      drive.snap('composer-idle');
      const screen = drive.screen();
      const lines = screen.split('\n').map((l) => l.trim()).filter((l) => l.length);
      const boxRow = lines.find((l) => l.startsWith('\u{2502}') && l.includes('>'));
      const regionRow = lines.find((l) => /^─*[\s─]*input[\s─]*─*$/.test(l));
      ctx.facts.composer = {
        box_row: boxRow ?? null,
        marker_after_box_trim: '>',
        region_label_row: regionRow ?? null,
        u276f_on_idle: screen.includes('\u{276f}'),
        footer_goal_hint: lines.some((l) => l.includes('/goal')),
        footer_context_meter: lines.some((l) => /context:\s*\d/.test(l)),
      };
      if (!boxRow) throw new Error('no `│ >` box composer row on the idle screen');
      if (regionRow) throw new Error('`── input ──` region still drawn — re-measure the block');
      if (screen.includes('\u{276f}')) {
        throw new Error('U+276F drawn on the idle screen — the marker story moved again');
      }
      return `box composer present; marker '>' (region label gone, ❯ absent)`;
    });

    // 3.5 THE DRAFT MEASUREMENT — type WITHOUT sending; the text renders
    //     after the `> ` head INSIDE the box row. Clear and prove empty.
    await ctx.probe('composer-draft-shape', async () => {
      const SENTINEL = 'PROBE-KC-DRAFT';
      drive.write(SENTINEL);
      const painted = await drive.waitFor(() => drive.screen().includes(SENTINEL), 10000);
      if (!painted) throw new Error('typed text never appeared on screen');
      await new Promise((r) => setTimeout(r, 1200));
      drive.snap('draft-typed');
      const lines = drive.screen().split('\n').map((l) => l.trim()).filter((l) => l.length);
      const draftRow = lines.find((l) => l.includes(SENTINEL));
      ctx.facts.composer_draft = {
        row: draftRow ?? null,
        inside_box_after_head: draftRow ? /^│\s*>\s/.test(draftRow) : false,
      };
      drive.write('\x7f'.repeat(SENTINEL.length + 4));
      const cleared = await drive.waitFor(() => !drive.screen().includes(SENTINEL), 10000);
      await new Promise((r) => setTimeout(r, 1000));
      drive.snap('draft-cleared');
      ctx.facts.composer_draft.cleared = cleared;
      return cleared
        ? `draft rendered in the box row after the '>' head; cleared ok`
        : 'sentinel never cleared — composer left dirty';
    });

    // 4. a turn: UNAUTH the CLI answers the measured refusal
    //    (`Error: LLM not set, send "/login" to login` — verbatim the 1.50
    //    phrase); AUTHED hosts answer for real and this probe accepts the
    //    reply, noting the posture. Working phrases stay null either way
    //    until a turn is actually observed grinding.
    await ctx.probe('turn-unauth-refusal', async () => {
      drive.write('Reply with exactly KIMI-CODE-INTAKE-OK and nothing else.');
      await new Promise((r) => setTimeout(r, 700));
      drive.write('\r');
      const settled = await drive.waitFor(
        () => /LLM not set|\/login|KIMI-CODE-INTAKE-OK/i.test(drive.screen()),
        120000,
      );
      await new Promise((r) => setTimeout(r, 2500));
      drive.snap('turn-settled');
      const screen = drive.screen();
      const refused = /LLM not set/i.test(screen);
      ctx.facts.turn = {
        settled,
        refusal_seen: refused,
        refusal_phrase: refused ? 'LLM not set, send "/login" to login' : null,
        echoed_sentinel: screen.includes('KIMI-CODE-INTAKE-OK'),
      };
      return refused ? 'unauth refusal observed' : 'turn settled (authed host?)';
    });

    // 5. store side-car: the refused turn creates NOTHING — the wd_ bucket
    //    for this cwd must not exist (id_assigned_at_birth:false, pinned). On
    //    an authed host a bucket DOES appear: read state.json instead.
    await ctx.probe('store-bucket-after-turn', async () => {
      const sha12 = (s) => crypto.createHash('sha256').update(s).digest('hex').slice(0, 12);
      const bucket = path.join(
        os.homedir(),
        '.kimi-code',
        'sessions',
        `wd_${path.basename(ctx.cwd)}_${sha12(ctx.cwd)}`,
      );
      ctx.facts.store = { bucket_expected: bucket, bucket_exists: fs.existsSync(bucket) };
      if (ctx.facts.store.bucket_exists) {
        const sess = fs
          .readdirSync(bucket)
          .map((d) => path.join(bucket, d))
          .filter((p) => fs.statSync(p).isDirectory())
          .sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs)[0];
        ctx.facts.store.session_id = sess && path.basename(sess);
        try {
          const state = JSON.parse(fs.readFileSync(path.join(sess, 'state.json'), 'utf8'));
          ctx.facts.store.state_title = state.title ?? null;
          ctx.facts.store.state_workdir = state.workDir ?? null;
        } catch (e) {
          ctx.facts.store.state_error = String(e);
        }
        return `bucket exists (authed turn?): session ${ctx.facts.store.session_id}`;
      }
      if (!ctx.facts.turn.refusal_seen) {
        throw new Error('turn answered but no store bucket — the bucket scheme moved, re-measure');
      }
      return `no bucket after refused turn — id minted at first real turn (birth-id flip pinned)`;
    });

    // 6. panic falsifier: SIGKILL the child; the drive must NOTICE.
    await ctx.probe('panic-falsifier', async () => {
      const code = await drive.killChild();
      if (code === null) throw new Error('child kill did not surface as pty exit');
      return `exit ${code}`;
    });

    // Login-gated facts this host CANNOT answer — honest nulls, not guesses.
    ctx.facts.working_screen_phrases = null; // composing.../thinking... need a real turn
    ctx.facts.resume = null; // replay needs a session: auth-gated ([11.175] residual)
    ctx.facts.store_layout_211 = null; // 2.1.1 store layout: no session observable unauth
  },
};
