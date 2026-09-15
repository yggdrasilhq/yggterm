// tools/probe-battery/suites/claude.js — the 11.6.2 Claude Code suite (class
// A, store-authoritative).
//
// The descriptor's claude block was measured on 2.1.223/2.1.x of 2026-08-07
// .. 2026-08-21; this suite is the regression net that re-asks those facts of
// whatever binary is installed NOW. Measured live against 2.1.272
// (2026-09-15): the `❯` composer marker holds, the idle footer reads
// `⏸ manual mode on · ? for shortcuts · ← for agents`, and the declared
// working footer hint `esc to interrupt` still paints — observed in the
// spinner-phase footer (`⏸ manual mode on · esc to interrupt · ← for
// agents`) even on the run this suite had to log as auth-gated.
//
// ⛔ AUTH STATE ON THE FLEET (measured 2026-09-15, owner-gated to fix): the
// subscription seat is DEAD on both hosts that run claude. On the muse-lab
// host the OAuth tokens in `~/.claude/.credentials.json` are EMPTIED
// (access + refresh = empty strings, expires 0, file mtime 2026-09-08) and
// the TUI banners `Not logged in · Run /login`; on the CI host the tokens
// are present but `claude -p` answers "Your organization has disabled
// Claude subscription access for Claude Code · Use an Anthropic API key
// instead". So the REAL-TURN and RESUME probes degrade to honest nulls
// here (the kimi-suite shape), and the pre-turn spinner is the only
// working-phase chrome this suite can see. When auth returns, re-run — the
// probes un-null themselves.
//
//   node run.js --suite suites/claude.js --cwd <fresh-empty-dir> \
//        [--suite-arg bin=claude]
//
// Store note: sessions live at `~/.claude/projects/<encoded-cwd>/<id>.jsonl`
// (row id IS the transcript id from birth — `--session-id` at launch). An
// UNAUTHED session writes NO rollout (measured: no bucket appears for the
// probe cwd), so the store probe asserts the negative until auth returns.

const fs = require('fs');
const path = require('path');

const WORKING_NEEDLES = ['esc to interrupt'];
const FOOTER_WINDOW_ROWS = 10; // SCREEN_FOOTER_WINDOW_ROWS in agent_cli.rs

// Mirror screen_phrases_match EXACTLY: the classifier lowercases each of the
// last N non-empty lines and matches lowercase needle fragments (the grok
// suite's 2026-09-15 sampling-trap lesson).
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

module.exports = {
  name: 'claude',

  async run(ctx) {
    const bin = ctx.args.bin || 'claude';
    const claudeHome = path.join(process.env.HOME || '', '.claude');

    ctx.drive = new ctx.Drive({
      command: bin,
      args: [],
      cwd: ctx.cwd,
      artifactsDir: ctx.artifactsDir,
      label: 'claude',
    });
    const drive = ctx.drive;

    // 1. launch → first paint; record the banner chrome and whether the
    //    DECLARED trust gate stands in the way. Measured 2026-09-15: the
    //    gate did NOT fire on a fresh /tmp cwd on 2.1.272 (neither host) —
    //    recorded honestly when absent, answered when present.
    let authGated = false;
    await ctx.probe('launch-first-paint', async () => {
      const painted = await drive.waitFor(() => drive.screen().trim().length > 0, 90000);
      if (!painted) throw new Error('no paint within 90s');
      await new Promise((r) => setTimeout(r, 4000)); // let the chrome settle
      drive.snap('first-paint');
      const screen = drive.screen();
      const gateNeedles = ['quick safety check', 'trust this folder'];
      const gateHits = phraseHits(screen, gateNeedles);
      ctx.facts.startup_gate = {
        declared_needles: gateNeedles,
        observed: gateHits.filter((h) => h.observed).map((h) => h.needle),
        gate_fired: gateHits.some((h) => h.observed),
      };
      if (ctx.facts.startup_gate.gate_fired) {
        drive.write('\r'); // `❯ 1. Yes, I trust this folder` is the default
        const cleared = await drive.waitFor(
          () => !phraseHits(drive.screen(), gateNeedles)[0].observed,
          20000,
          300,
        );
        await new Promise((r) => setTimeout(r, 2000));
        ctx.facts.startup_gate.answered = cleared;
        if (!cleared) throw new Error('trust gate answered but never cleared');
      }
      authGated = /not logged in|run \/login/i.test(drive.screen());
      const lines = screen.split('\n').map((l) => l.trim()).filter(Boolean);
      ctx.facts.first_paint = {
        banner_version: screen.match(/Claude Code v(\d+\.\d+\.\d+)/)?.[1] ?? null,
        model_footer: lines.find((l) => /context|billing|model/i.test(l))?.slice(0, 120) ?? null,
        auth_gate_line: authGated
          ? lines.find((l) => /not logged in|run \/login/i.test(l)) ?? null
          : null,
      };
      return authGated
        ? `painted v${ctx.facts.first_paint.banner_version} — AUTH-GATED: ${ctx.facts.first_paint.auth_gate_line}`
        : `painted v${ctx.facts.first_paint.banner_version}, no auth banner; gate fired: ${ctx.facts.startup_gate.gate_fired}`;
    });

    // 2. composer idle shape: the declared ❯ (U+276F) marker must be DRAWN,
    //    and the declared footer-hint vocabulary re-asked honestly.
    await ctx.probe('composer-idle-shape', async () => {
      if (authGated) {
        // The unauthed TUI still paints the composer; measure it anyway.
        await new Promise((r) => setTimeout(r, 1500));
      } else {
        await new Promise((r) => setTimeout(r, 1500));
      }
      drive.snap('composer-idle');
      const screen = drive.screen();
      const lines = screen.split('\n').map((l) => l.trim()).filter(Boolean);
      const declaredHints = ['claude', 'permissions', 'shift+tab', 'for agents', 'ctrl', 'esc'];
      ctx.facts.composer_idle = {
        marker_observed: screen.includes('\u276f') ? '\u276f' : null,
        // Scoped to the classifier's footer window: a whole-screen
        // includes() matches the banner ('Claude Code') not the footer.
        declared_footer_hints_seen: phraseHits(screen, declaredHints).map((h) => ({
          hint: h.needle,
          seen: h.observed,
        })),
        footer_lines: lines.filter((l) => /mode on|shortcuts|agents|permissions/i.test(l) && l.length < 120).slice(-3),
      };
      if (!ctx.facts.composer_idle.marker_observed) {
        throw new Error('declared composer marker U+276F not drawn — re-measure before any descriptor edit');
      }
      return `marker ❯ drawn; hints seen: ${ctx.facts.composer_idle.declared_footer_hints_seen.filter((h) => h.seen).map((h) => h.hint).join(',') || 'NONE'}`;
    });

    // 3. THE TURN — with auth dead this cannot complete; the measured
    //    behavior on 2.1.272 is: the prompt is ACCEPTED into the composer,
    //    a spinner phase paints (`✽ <Gerund>…`) whose footer carries the
    //    declared working hint, then the unauth banner appears and no reply
    //    ever lands. Probes what CAN be probed: the working needle during
    //    the spinner phase, the refusal surface, and the absence of a reply.
    let turnObservedSpinner = false;
    let turnSettledScreen = null;
    await ctx.probe('turn-attempt', async () => {
      const sentinel = `DONE-${Date.now().toString(36).slice(-6)}`;
      drive.write(`Count slowly from one to five, then say ${sentinel}`);
      await new Promise((r) => setTimeout(r, 600));
      drive.write('\r');
      let refusal = null;
      const deadline = Date.now() + 120000;
      let stable = 0;
      let last = '';
      while (Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, 300));
        const screen = drive.screen();
        if (phraseHits(screen, WORKING_NEEDLES)[0].observed) turnObservedSpinner = true;
        const refMatch = screen
          .split('\n')
          .map((l) => l.trim())
          .find((l) => /not logged in|run \/login|subscription access|api key/i.test(l));
        if (refMatch && !refusal) refusal = refMatch;
        const hash = screen.length + ':' + screen.slice(-200);
        stable = hash === last ? stable + 1 : 0;
        last = hash;
        if (stable >= 25) break; // ~7.5s unchanged: this attempt is over
        if (drive.exitCode !== null) break;
      }
      await new Promise((r) => setTimeout(r, 2000));
      drive.snap('turn-settled');
      turnSettledScreen = drive.screen();
      const replied = turnSettledScreen.includes(sentinel);
      ctx.facts.turn = {
        auth_gated: authGated || !!refusal,
        working_needle_observed_in_spinner_phase: turnObservedSpinner,
        refusal_surface: refusal,
        // The composer ECHO of the typed prompt carries the sentinel once;
        // a real reply adds a second occurrence. Count, don't includes().
        replied: (turnSettledScreen.match(new RegExp(sentinel, 'g')) ?? []).length >= 2, // false is the MEASURED answer here, not a null
      };
      if (replied) {
        return `TURN COMPLETED — auth has returned; reply landed. Re-run: the nulls above are stale.`;
      }
      if (!refusal && !turnObservedSpinner) {
        throw new Error('no reply, no refusal surface, no spinner — an unknown turn state, re-measure');
      }
      return `auth-gated turn: spinner-phase working needle ${turnObservedSpinner ? 'OBSERVED' : 'not seen'}; refusal: ${JSON.stringify(refusal)}; no reply (measured, credential-gated)`;
    });

    // 4. store side-car: an unauthed session writes NO rollout. Assert the
    //    negative honestly; when auth returns this probe flips to reading
    //    the new bucket for the probe cwd.
    await ctx.probe('store-side-car', async () => {
      const projectsRoot = path.join(claudeHome, 'projects');
      let bucketExists = false;
      let bucketEntries = 0;
      if (fs.existsSync(projectsRoot)) {
        // Bucket name = the cwd with separators collapsed to `-` (measured
        // verbatim in the real tree, e.g. `-tmp-claude-3001-...`).
        const expected = ctx.cwd.replaceAll('/', '-');
        const buckets = fs
          .readdirSync(projectsRoot, { withFileTypes: true })
          .filter((d) => d.isDirectory())
          .map((d) => d.name);
        const mine = buckets.find((b) => b === expected || ctx.cwd.endsWith(b.replaceAll('-', '/').slice(0, 12)));
        bucketExists = !!mine;
        if (mine) {
          bucketEntries = fs
            .readdirSync(path.join(projectsRoot, mine))
            .filter((f) => f.endsWith('.jsonl')).length;
        }
      }
      ctx.facts.store = {
        root_exists: fs.existsSync(projectsRoot),
        bucket_exists: bucketExists,
        rollout_count_in_bucket: bucketEntries,
        declared_glob: '.claude/projects/*/*.jsonl',
        note: bucketExists
          ? 'bucket exists — a session DID write; if this run was auth-gated that is unexpected'
          : 'NO bucket for the probe cwd — the unauthed session wrote nothing (measured 2026-09-15)',
      };
      if (bucketExists && authGated) {
        throw new Error('auth-gated run but a store bucket exists — either auth returned or the store writes pre-auth; re-measure');
      }
      return bucketExists
        ? `bucket exists with ${bucketEntries} rollout(s)`
        : 'no bucket (measured negative under auth-gate)';
    });

    // 5. flag surface: claude --help re-asked of the installed binary — the
    //    declared selector (`--resume`), birth id (`--session-id`), model,
    //    permission-mode and the standalone bypass must still spell that way.
    await ctx.probe('help-flag-surface', async () => {
      const help = new ctx.Drive({
        command: bin,
        args: ['--help'],
        cwd: ctx.cwd,
        artifactsDir: ctx.artifactsDir,
        label: 'claude-help',
      });
      await help.waitFor(() => /usage|options|commands/i.test(help.screen()) || help.exitCode !== null, 20000);
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
        bypass: line(/--dangerously-skip-permissions/),
      };
      if (!ctx.facts.help_flags.resume || !ctx.facts.help_flags.session_id) {
        throw new Error('declared flag surface lost — --resume or --session-id missing from --help');
      }
      return `resume: ${JSON.stringify(ctx.facts.help_flags.resume)}; session-id: ${JSON.stringify(ctx.facts.help_flags.session_id)}`;
    });

    // 6. panic falsifier: SIGKILL the child; the drive must NOTICE.
    await ctx.probe('panic-falsifier', async () => {
      const code = await drive.killChild();
      if (code === null) throw new Error('child kill did not surface as pty exit');
      return `exit ${code}`;
    });

    // Persisted for the outcome post: the exact screens behind the facts.
    if (turnSettledScreen) {
      ctx.facts.turn_settled_tail = turnSettledScreen.split('\n').filter((l) => l.trim()).slice(-6);
    }
  },
};
