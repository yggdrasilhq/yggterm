// tools/probe-battery/suites/agy.js — the 11.6.4 antigravity suite (class C,
// TUI-with-store).
//
// The descriptor's agy block was measured on 1.2.0 (2026-09-10 seat C,
// bare-PTY) and re-proven on 1.2.2 (2026-09-14 refix seat); this suite is the
// regression net that re-asks every declared fact of whatever binary is
// installed NOW. Measured live against 1.2.3 (2026-09-15, first pass findings
// baked into the drive order):
//   ⭐ 1.2.3 LAUNCHES THROUGH A SIGN-IN PHASE ("You are currently not signed
//   in." → "⣾ Signing in..." → composer) that can outlast 15s; input typed
//   during it is DISCARDED SILENTLY (run-1 artifacts; the seat-C trap, one
//   phase earlier than the trust gate). The suite waits for the signed-in
//   composer before touching the keyboard.
//   ⭐ The workspace-trust picker did NOT fire on a fresh /tmp folder in
//   1.2.3 (declared startup gate observed:false — recorded, not assumed).
//   ⭐ The question picker's declared SAME-LINE shape ("requesting
//   permission for:" + "run this command?" on one line) is FALSIFIED in
//   1.2.3 — the two lines draw apart, so the classifier's also_any rule can
//   never match (the descriptor fix rides this lane).
//   ⚠ 1.2.3 EXITED code=0 ~1.3s after the first tool approval in run 1, a
//   crash log flashing in crashes/ and auto-removed within minutes — the
//   suite records the post-approval liveness explicitly.
//
// Measured live against 1.2.11 (2026-09-26 re-pair seat, baseline +
// verification runs on the muse lab host):
//   ⭐ the conversations-db flush-while-live is NONDETERMINISTIC on 1.2.11 —
//   the baseline run's sentinel was IN the db while the child lived, the
//   verification run's was not until exit. Attribution therefore asks BOTH
//   arms (conversations db + brain/<id>/.system_generated/logs/*.jsonl) and
//   NAMES the arm that answered (attributed_via) — the brain jsonl is the
//   live-stream arm and answered the verification run.
//   ⭐ presence locks LEAK (49 stale on this host beside the run's child)
//   and are EMPTY files — existence alone reads lies; liveness = fd-held
//   RIGHT NOW (proven in-suite: the run's own child held its lock, the 49
//   leaked locks are held by nobody). The fact now reads live/stale, not a
//   bare list.
//   ⭐ crashes/ grows ZERO-BYTE crash_<pid>_<uuid>.log shells during
//   ordinary turns (both runs caught their own child's shell, pid-matched)
//   — recorded with size; a shell is not crash evidence, only a non-empty
//   log is.
//   ⭐ TRUSTED folder: the question picker NEVER DRAWS — 1.2.11
//   auto-approves the tool call (baseline: the write landed while the probe
//   burned its full 180s picker wait). The probe now bails the moment the
//   file lands and names the drift.
//   ⭐ resume of a COMPLETED conversation: the rederive paints (the needle
//   is scroll-safe: reply token or prompt head — the head-only check
//   false-negatived on a scrolled two-turn transcript) and the child's
//   post-rederive exit is INTERMITTENT — the baseline child self-exited
//   code 0 within seconds (the 1.2.3 exit-0 candidate), the verification
//   child stayed alive; recorded either way as resumed_child_self_exit.
//
// Auth is assumed (antigravity-oauth-token); a login that never completes
// degrades the turn facts to honest nulls, kimi-suite style.
//
//   node run.js --suite suites/agy.js --cwd <real-empty-fresh-workdir> \
//        [--suite-arg bin=agy] [--suite-arg prompt="..."]
//
// Store note: the conversation id is attributed by SENTINEL-IN-CONTENT
// (`buf.includes` over conversations/*.db), never by mtime — sibling live
// agy rows on the lab host create dbs concurrently and mtime recency lies
// (the zcode-tui 1111 lesson, re-proven here). The conversation db FLUSHES
// AT EXIT (run 1: sentinel absent from the store while the child was live,
// present after exit) — the store probe reads in both states and records
// the timing.

const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const WORKING_NEEDLES = [
  'esc to cancel',
  'esc to interrupt',
  'generating...',
  'thinking...',
  'working...',
];
const GATE_NEEDLE = 'do you trust the contents of this project?';
const PICKER_NEEDLE = 'requesting permission for:';
const PICKER_ALSO = 'run this command?';
const FOOTER_WINDOW_ROWS = 10; // SCREEN_FOOTER_WINDOW_ROWS in agent_cli.rs
const AGY_HOME = path.join(process.env.HOME || '', '.gemini', 'antigravity-cli');

// Mirror screen_phrases_match EXACTLY: last N non-empty trimmed lines,
// lowercased, needle contained (also_any must hit the SAME line).
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

function sqliteDump(dbPath, tailRows, artifactsDir, label) {
  // Schema-agnostic: dump the newest `tailRows` rows of every table as lines,
  // plus the schema itself, so the door can re-read the layout later.
  const out = { db: dbPath, exists: fs.existsSync(dbPath), tables: [], rows_text: null };
  if (!out.exists) return out;
  try {
    out.tables = execFileSync('sqlite3', ['-readonly', dbPath, '.tables'], { encoding: 'utf8' })
      .split(/\s+/)
      .filter(Boolean);
    const schema = execFileSync('sqlite3', ['-readonly', dbPath, '.schema'], { encoding: 'utf8' });
    fs.writeFileSync(path.join(artifactsDir, `${label}-schema.sql`), schema);
    const lines = [];
    for (const t of out.tables) {
      lines.push(
        execFileSync(
          'sqlite3',
          ['-readonly', '-cmd', '.mode line', dbPath, `select * from ${t} order by rowid desc limit ${tailRows}`],
          { encoding: 'utf8' },
        ),
      );
    }
    out.rows_text = lines.join('\n');
    fs.writeFileSync(path.join(artifactsDir, `${label}-tail.txt`), out.rows_text);
  } catch (e) {
    out.error = String(e.message || e);
  }
  return out;
}

module.exports = {
  name: 'agy',

  async run(ctx) {
    const bin = ctx.args.bin || 'agy';
    const sentinel = ctx.args.sentinel || `BATTERYAGY${Date.now().toString(36)}`;
    const prompt =
      ctx.args.prompt || `${sentinel}: Count slowly from one to twenty, one number per line. When done, end your reply with exactly this token on its own line: ${sentinel}-REPLY`;
    const pickerPrompt = `${sentinel}P: Create a file named agy-battery-pick.txt in this folder containing exactly BATTERY_PICKER_OK, then report the file's contents.`;
    const replyToken = `${sentinel}-REPLY`;
    ctx.facts.sentinel = sentinel;

    const dirList = (dir, suffix) => {
      try {
        return fs.readdirSync(dir).filter((f) => f.endsWith(suffix)).sort();
      } catch (_) {
        return null;
      }
    };
    const findSentinelDb = (needle) => {
      const convDir = path.join(AGY_HOME, 'conversations');
      try {
        for (const f of fs.readdirSync(convDir).filter((f) => f.endsWith('.db'))) {
          try {
            if (fs.readFileSync(path.join(convDir, f)).includes(needle)) return path.basename(f, '.db');
          } catch (_) {}
        }
      } catch (_) {}
      return null;
    };
    // 1.2.7+ live arm: transcripts stream to brain/<id>/.system_generated/logs/*.jsonl
    // while the child runs; the conversations db only flushes at exit (the 09-20
    // re-pair note). The brain dir NAME is the session id.
    const findSentinelBrain = (needle) => {
      const brainDir = path.join(AGY_HOME, 'brain');
      try {
        for (const id of fs.readdirSync(brainDir)) {
          const logsDir = path.join(brainDir, id, '.system_generated', 'logs');
          let found = null;
          try {
            for (const f of fs.readdirSync(logsDir).filter((f) => f.endsWith('.jsonl'))) {
              try {
                if (fs.readFileSync(path.join(logsDir, f)).includes(needle)) {
                  found = id;
                  break;
                }
              } catch (_) {}
            }
          } catch (_) {}
          if (found) return found;
        }
      } catch (_) {}
      return null;
    };
    // Presence locks are EMPTY files (no pid, no heartbeat payload) that LEAK
    // (43/44 stale on the muse lab host, 09-20) — the honest liveness signal is
    // who holds the lock as an open fd RIGHT NOW (verified: a live agy child
    // holds its lock; stale leaked locks are held by nobody). One /proc sweep
    // answers every lock at once.
    const presenceLockHolders = () => {
      const holders = {}; // abs lock path -> [pids]
      let pids = [];
      try {
        pids = fs.readdirSync('/proc').filter((d) => /^\d+$/.test(d));
      } catch (_) {
        return null;
      }
      for (const pid of pids) {
        let fds = [];
        try {
          fds = fs.readdirSync(`/proc/${pid}/fd`);
        } catch (_) {
          continue; // raced exit or no permission
        }
        for (const fd of fds) {
          let tgt = null;
          try {
            tgt = fs.readlinkSync(`/proc/${pid}/fd/${fd}`);
          } catch (_) {
            continue;
          }
          if (tgt.includes('/presence/') && tgt.endsWith('.lock')) {
            (holders[tgt] = holders[tgt] || []).push(Number(pid));
          }
        }
      }
      return holders;
    };
    const presenceFacts = (holders) => {
      const dir = path.join(AGY_HOME, 'presence');
      let names;
      try {
        names = fs.readdirSync(dir).filter((f) => f.endsWith('.lock')).sort();
      } catch (_) {
        return null;
      }
      const live = [];
      const stale = [];
      for (const f of names) {
        const abs = path.join(dir, f);
        const pids = holders ? holders[abs] || [] : [];
        (pids.length ? live : stale).push({ id: f.replace(/\.lock$/, ''), held_by_pids: pids });
      }
      return {
        total: names.length,
        live: live.map((l) => l.id),
        live_pids: live.reduce((a, l) => a.concat(l.held_by_pids), []),
        stale_count: stale.length,
        stale_sample: stale.slice(0, 3).map((l) => l.id),
      };
    };
    // crashes/ grows zero-byte `crash_<pid>_<uuid>.log` SHELLS during ordinary
    // turns (1.2.3/1.2.7/1.2.11) — a shell is not crash evidence; only a
    // non-empty log is. Record both, never conflate them.
    const crashScan = () => {
      const crashDir = path.join(AGY_HOME, 'crashes');
      let entries;
      try {
        entries = fs.readdirSync(crashDir);
      } catch (_) {
        return null;
      }
      return entries.map((f) => {
        let bytes = null;
        let age_s = null;
        try {
          const st = fs.statSync(path.join(crashDir, f));
          bytes = st.size;
          age_s = Math.max(0, Math.round((Date.now() - st.mtimeMs) / 1000));
        } catch (_) {}
        return { file: f, bytes, zero_byte_shell: bytes === 0, age_s };
      });
    };

    ctx.drive = new ctx.Drive({
      command: bin,
      args: [],
      cwd: ctx.cwd,
      artifactsDir: ctx.artifactsDir,
      label: 'agy',
    });
    const drive = ctx.drive;

    // 1. launch → first paint → sign-in phase → signed-in composer. The gate
    //    (if any) is watched for across the whole wait, not just at paint.
    let gateShown = false;
    let loginGated = false;
    await ctx.probe('launch-signin-first-paint', async () => {
      const painted = await drive.waitFor(() => drive.screen().trim().length > 0, 60000);
      if (!painted) throw new Error('no paint within 60s');
      drive.snap('first-paint');
      const paintScreen = drive.screen();
      const signingIn = /signing in|not signed in/i.test(paintScreen);
      // Wait for the composer ('>' prompt line) or a gate; record both paths.
      const ready = await drive.waitFor(() => {
        const s = drive.screen();
        if (s.toLowerCase().includes(GATE_NEEDLE)) {
          gateShown = true;
          return true;
        }
        return s.split('\n').some((l) => /^>\s*$/.test(l.trim()));
      }, 120000, 300);
      await new Promise((r) => setTimeout(r, 3000)); // let the chrome settle
      drive.snap('signed-in-idle');
      const screen = drive.screen();
      loginGated = !ready && /sign in|login|api key|authenticate/i.test(screen);
      ctx.facts.first_paint = {
        head_lines: paintScreen.split('\n').map((l) => l.trim()).filter(Boolean).slice(0, 8),
        signin_phase_observed: signingIn,
        signed_in_wait_completed: ready,
        model_line: screen.split('\n').find((l) => /gemini\s|model/i.test(l))?.trim() ?? null,
        account_line: screen.split('\n').find((l) => /@/.test(l))?.trim() ?? null,
      };
      ctx.facts.startup_gate = {
        declared_needle: GATE_NEEDLE,
        observed: gateShown,
        gate_lines: gateShown
          ? screen.split('\n').filter((l) => /trust|yes|no/i.test(l)).map((l) => l.trim()).slice(0, 6)
          : null,
      };
      if (!ready) return loginGated ? 'LOGIN-GATED — never reached a composer' : 'no composer within 120s';
      return gateShown ? 'composer ready; GATED: workspace-trust picker shown' : 'composer ready, no gate';
    });

    // 2. trust-gate-clear: measure the picker, then clear it the way a user
    //    does (Enter on the highlighted "Yes" row). The answer lands in agy's
    //    OWN settings (trustedWorkspaces) — verify that, never write it.
    await ctx.probe('trust-gate-clear', async () => {
      if (!gateShown) {
        ctx.facts.trust_gate_clear = {
          skipped: loginGated ? 'login gate' : 'gate not shown on a fresh folder (1.2.3 drift vs 1.2.2 — recorded)',
        };
        return ctx.facts.trust_gate_clear.skipped;
      }
      const settingsPath = path.join(AGY_HOME, 'settings.json');
      const trustedBefore = (() => {
        try {
          return JSON.parse(fs.readFileSync(settingsPath, 'utf8')).trustedWorkspaces ?? null;
        } catch (_) {
          return null;
        }
      })();
      drive.write('\r'); // Enter confirms the highlighted "Yes, I trust" row
      const cleared = await drive.waitFor(
        () => !drive.screen().toLowerCase().includes(GATE_NEEDLE),
        30000,
      );
      await new Promise((r) => setTimeout(r, 2000));
      drive.snap('gate-cleared');
      let trustedAfter = null;
      try {
        trustedAfter = JSON.parse(fs.readFileSync(settingsPath, 'utf8')).trustedWorkspaces ?? null;
      } catch (_) {}
      ctx.facts.trust_gate_clear = {
        cleared,
        trusted_workspaces_before: Array.isArray(trustedBefore) ? trustedBefore.length : null,
        trusted_workspaces_after: Array.isArray(trustedAfter) ? trustedAfter.length : null,
        cwd_now_trusted: Array.isArray(trustedAfter) ? trustedAfter.includes(ctx.cwd) : null,
      };
      if (!cleared) throw new Error('trust picker never cleared after Enter — re-measure');
      return `gate cleared; cwd in trustedWorkspaces: ${ctx.facts.trust_gate_clear.cwd_now_trusted}`;
    });

    // 3. composer idle shape: the declared '>' marker must be DRAWN, the
    //    declared idle hints must appear in the footer window, and idle must
    //    carry NO working needle. Also snapshots the pre-turn store (the
    //    id_assigned_at_birth:false evidence delta).
    await ctx.probe('composer-idle-shape', async () => {
      if (loginGated) return 'skipped — login gate';
      const screen = drive.screen();
      const composerLines = screen
        .split('\n')
        .map((l) => l.trim())
        .filter((l) => l.startsWith('>'));
      const composerLine = composerLines[composerLines.length - 1];
      ctx.facts.composer_idle = {
        marker_observed: composerLine ? '>' : null,
        composer_line: composerLine ?? null,
        declared_hints_seen: ['shortcuts', 'esc', 'ctrl', 'enter', 'tab', 'gemini', '?'].map((h) => ({
          hint: h,
          seen: phraseHits(screen, [h])[0].observed,
        })),
        idle_working_needles: phraseHits(screen, WORKING_NEEDLES).filter((h) => h.observed).map((h) => h.needle),
      };
      ctx.facts.store_pre_turn = {
        conversations: dirList(path.join(AGY_HOME, 'conversations'), '.db'),
        presence_locks: presenceFacts(presenceLockHolders()),
        crashes: crashScan(),
      };
      if (!ctx.facts.composer_idle.marker_observed) {
        throw new Error('declared composer marker > not drawn — re-measure before any descriptor edit');
      }
      return `marker > drawn; working-needles-at-idle: ${ctx.facts.composer_idle.idle_working_needles.length}`;
    });

    // 4. THE REAL TURN — the needle measurement. The prompt ECHO is verified
    //    before the poll (input typed into a non-composer phase dies
    //    silently — run-1); one guarded retype, then the windowed classifier
    //    poll through the whole turn.
    let turnReplyOk = false;
    await ctx.probe('real-turn-working-needles', async () => {
      if (loginGated) {
        ctx.facts.working_screen_phrases = null;
        ctx.facts.working_footer_hints = null;
        return 'skipped — login gate';
      }
      const echoSeen = () => drive.screen().includes(prompt.slice(0, 24));
      drive.write(prompt);
      await new Promise((r) => setTimeout(r, 500));
      drive.write('\r');
      let echoed = await drive.waitFor(echoSeen, 10000, 200);
      if (!echoed) {
        drive.write('\r');
        await new Promise((r) => setTimeout(r, 300));
        drive.write(prompt);
        await new Promise((r) => setTimeout(r, 500));
        drive.write('\r');
        echoed = await drive.waitFor(echoSeen, 10000, 200);
      }
      if (!echoed) {
        ctx.facts.working_screen_phrases = null;
        ctx.facts.turn_prompt_echoed = false;
        throw new Error('prompt never echoed after guarded retype — the composer is not taking input; re-measure');
      }
      ctx.facts.turn_prompt_echoed = true;
      const seen = new Map(WORKING_NEEDLES.map((n) => [n, false]));
      const hitLines = new Set();
      const deadline = Date.now() + 240000;
      let stable = 0;
      let last = '';
      while (Date.now() < deadline) {
        await new Promise((r) => setTimeout(r, 200));
        const screen = drive.screen();
        for (const h of phraseHits(screen, WORKING_NEEDLES)) {
          if (!h.observed) continue;
          seen.set(h.needle, true);
          for (const line of screen.split('\n')) {
            if (line.toLowerCase().includes(h.needle)) hitLines.add(line.trim());
          }
        }
        const h = screen.length + ':' + screen.slice(-200);
        stable = h === last ? stable + 1 : 0;
        last = h;
        if (stable >= 25) break; // ~5s unchanged: the turn is over
        if (drive.exitCode !== null) break;
      }
      await new Promise((r) => setTimeout(r, 2000));
      drive.snap('turn-settled');
      const screen = drive.screen();
      turnReplyOk = screen.includes(replyToken);
      ctx.facts.working_screen_phrases = WORKING_NEEDLES.map((n) => ({
        needle: n,
        observed: seen.get(n),
      }));
      ctx.facts.working_footer_hints = {
        declared: WORKING_NEEDLES,
        observed_during_turn: WORKING_NEEDLES.filter((n) => seen.get(n)),
      };
      ctx.facts.working_line_sample = [...hitLines].slice(0, 4);
      ctx.facts.turn_reply_ok = turnReplyOk;
      if (![...seen.values()].some(Boolean)) {
        if (!turnReplyOk) {
          throw new Error('NO declared working needle seen AND no reply token — the turn failed, re-run before concluding drift');
        }
        ctx.facts.needle_window_missed = true;
        return `turn completed but beat the poll — no needle sampled; heavier prompt needed before declaring drift`;
      }
      return `needles seen: ${WORKING_NEEDLES.filter((n) => seen.get(n)).join(' + ')}; reply: ${turnReplyOk}`;
    });

    // 5. question picker: a shell-command ask must park the TUI on the
    //    picker. 1.2.3 DRAWS THE TWO DECLARED LINES APART (falsifying the
    //    same-line also_any shape) — both shapes are measured here. Approve
    //    with Enter and record the post-approval liveness (run-1's exit-0
    //    anomaly).
    await ctx.probe('question-picker-shape', async () => {
      if (loginGated) return 'skipped — login gate';
      drive.write(pickerPrompt);
      await new Promise((r) => setTimeout(r, 500));
      drive.write('\r');
      const echoed = await drive.waitFor(() => drive.screen().includes(pickerPrompt.slice(0, 24)), 10000, 200);
      if (!echoed) return 'picker prompt never echoed — composer not taking input; skipped';
      const pickFile = path.join(ctx.cwd, 'agy-battery-pick.txt');
      const pickerSeen = await drive.waitFor(
        () => {
          // 1.2.11 drift: a TRUSTED folder auto-approves the write — no picker
          // will ever come once the file lands; bail instead of burning 180s.
          if (fs.existsSync(pickFile)) return true;
          const s = drive.screen().toLowerCase();
          return s.includes(PICKER_NEEDLE) || s.includes(PICKER_ALSO);
        },
        180000,
        300,
      );
      drive.snap('picker');
      const screen = drive.screen();
      const low = screen.toLowerCase();
      const fileLanded = fs.existsSync(pickFile);
      const pickerShown = pickerSeen && !fileLanded; // file landed first ⇒ no picker ever drew
      const sameLine = low
        .split('\n')
        .some((l) => l.includes(PICKER_NEEDLE) && l.includes(PICKER_ALSO));
      ctx.facts.question_picker = {
        picker_seen: pickerShown,
        needle_line: screen.split('\n').find((l) => l.toLowerCase().includes(PICKER_NEEDLE))?.trim() ?? null,
        also_line: screen.split('\n').find((l) => l.toLowerCase().includes(PICKER_ALSO))?.trim() ?? null,
        declared_same_line_shape: sameLine,
        working_needle_also_visible: phraseHits(screen, WORKING_NEEDLES).filter((h) => h.observed).map((h) => h.needle),
      };
      if (!pickerShown) {
        ctx.facts.question_picker.approved = false;
        ctx.facts.question_picker.auto_approved_under_trust = fileLanded;
        return fileLanded
          ? 'no picker — the write RAN under a trusted folder (1.2.11 auto-approve drift)'
          : 'picker never shown and no file — the turn stalled or auto-denied; read the snap';
      }
      drive.write('\r'); // approve the highlighted choice; the turn continues
      // Output check is LINE-ANCHORED (the command echo also contains the
      // token — run-1's loose includes() lied).
      const outputSeen = await drive.waitFor(
        () => drive.screen().split('\n').some((l) => l.trim() === 'BATTERY_PICKER_OK'),
        120000,
        400,
      );
      await new Promise((r) => setTimeout(r, 1500));
      const exitedAfterApproval = drive.exitCode;
      drive.snap('picker-approved');
      ctx.facts.question_picker.approved = true;
      ctx.facts.question_picker.output_line_seen = outputSeen;
      ctx.facts.question_picker.exited_after_approval = exitedAfterApproval !== null ? exitedAfterApproval : false;
      return `picker measured (declared same-line shape: ${sameLine}); approved, output line: ${outputSeen}, child exited after approval: ${exitedAfterApproval ?? 'no'}`;
    });

    // 6. store side-car: attribute THIS conversation by sentinel-in-content
    //    (never mtime), in BOTH liveness states. 1.2.7+ streams the turn to
    //    brain/<id>/.system_generated/logs/*.jsonl while live; the
    //    conversations db flushes at exit (run 1) — both arms are asked and
    //    the answering arm is NAMED. The child's presence lock is read for
    //    fd-holders BEFORE the kill (the locks leak, existence alone lies).
    let turnSessionId = null;
    await ctx.probe('store-side-car', async () => {
      const convDir = path.join(AGY_HOME, 'conversations');
      if (!fs.existsSync(convDir)) {
        ctx.facts.store = { root_exists: false };
        return 'no ~/.gemini/antigravity-cli/conversations at all';
      }
      const childAlive = drive.exitCode === null;
      const sentinelDbLive = findSentinelDb(sentinel);
      const sentinelBrainLive = findSentinelBrain(sentinel);
      // the run's lock is named by its SESSION id (the attributed one), never
      // the sentinel — read its fd-holders BEFORE the kill
      const liveId = sentinelDbLive ?? sentinelBrainLive;
      const holders = presenceLockHolders();
      const runLockPath = liveId ? path.join(AGY_HOME, 'presence', `${liveId}.lock`) : null;
      const presenceHeldWhileLive =
        runLockPath && fs.existsSync(runLockPath) ? (holders ? holders[runLockPath] || [] : null) : null;
      if (childAlive) {
        await drive.killChild(); // crash semantics; the flush is what run 1 measured
        await new Promise((r) => setTimeout(r, 2500));
      }
      const sentinelDbPost = findSentinelDb(sentinel);
      const sentinelBrainPost = findSentinelBrain(sentinel);
      turnSessionId = sentinelDbPost ?? sentinelBrainPost ?? sentinelDbLive ?? sentinelBrainLive;
      const attributedVia =
        sentinelDbPost || sentinelDbLive
          ? 'conversations-db'
          : sentinelBrainPost || sentinelBrainLive
            ? 'brain-jsonl'
            : null;
      const lockPath = turnSessionId ? path.join(AGY_HOME, 'presence', `${turnSessionId}.lock`) : null;
      const presenceLock = turnSessionId ? fs.existsSync(lockPath) : null;
      const summaries = sqliteDump(
        path.join(AGY_HOME, 'conversation_summaries.db'),
        60,
        ctx.artifactsDir,
        'summaries',
      );
      const summariesHit = summaries.rows_text
        ? summaries.rows_text.includes(turnSessionId ?? sentinel)
        : false;
      ctx.facts.store = {
        root_exists: true,
        conversation_db_count: dirList(convDir, '.db')?.length ?? null,
        sentinel_matches_live_child: !!(sentinelDbLive || sentinelBrainLive),
        attributed_via_live: sentinelDbLive
          ? 'conversations-db'
          : sentinelBrainLive
            ? 'brain-jsonl'
            : null,
        turn_session_id: turnSessionId,
        attributed_via: attributedVia,
        db_flushed_while_child_alive: !!sentinelDbLive,
        brain_jsonl_live: !!sentinelBrainLive,
        new_since_pre_turn: turnSessionId
          ? !(ctx.facts.store_pre_turn?.conversations ?? []).includes(`${turnSessionId}.db`)
          : null,
        presence_lock: presenceLock,
        presence_lock_held_while_live: presenceHeldWhileLive,
        crashes_post_run: crashScan(),
        summaries_db: {
          exists: summaries.exists,
          tables: summaries.tables,
          row_for_turn_session: summariesHit,
        },
      };
      if (!turnSessionId)
        return 'sentinel not in any conversations db NOR any brain transcript even after exit — was the turn persisted?';
      return `session ${turnSessionId} (via ${attributedVia}; db flushed while live: ${!!sentinelDbLive}, brain live: ${!!sentinelBrainLive}; lock held while live: ${presenceHeldWhileLive ? 'pids ' + presenceHeldWhileLive.join(',') : 'NO'}); summaries row: ${summariesHit}`;
    });

    // 7. flag surface: agy --help re-asked of the installed binary — the
    //    declared selector (--conversation), -c/--continue, --model, --mode,
    //    --dangerously-skip-permissions, --sandbox must still spell that way;
    //    1.2.3's new flags recorded as drift evidence.
    await ctx.probe('help-flag-surface', async () => {
      const help = new ctx.Drive({
        command: bin,
        args: ['--help'],
        cwd: ctx.cwd,
        artifactsDir: ctx.artifactsDir,
        label: 'agy-help',
      });
      await help.waitFor(() => help.screen().toLowerCase().includes('usage') || help.exitCode !== null, 20000);
      await new Promise((r) => setTimeout(r, 800));
      help.snap('help');
      const screen = help.screen();
      help.dispose();
      const line = (re) => screen.split('\n').find((l) => re.test(l))?.trim() ?? null;
      ctx.facts.help_flags = {
        resume_by_id: line(/--conversation/),
        continue_most_recent: line(/--continue/),
        model: line(/--model/),
        mode: line(/--mode/),
        skip_permissions: line(/--dangerously-skip-permissions/),
        sandbox: line(/--sandbox/),
        new_in_123: ['--effort', '--input-format', '--output-format', '--json-schema', '--print', '--prompt-interactive', '--add-dir', '--agent'].map((f) => ({
          flag: f,
          present: screen.includes(f),
        })),
      };
      return `--conversation: ${JSON.stringify(ctx.facts.help_flags.resume_by_id)}; --continue: ${JSON.stringify(ctx.facts.help_flags.continue_most_recent)}`;
    });

    // 8. resume redrive: `--conversation <id>` must rederive the prior
    //    transcript onto the screen (content_rederives_on_resume).
    let resumedDrive = null;
    await ctx.probe('resume-redrive', async () => {
      if (!turnSessionId) {
        ctx.facts.resume = null; // no session to resume — honest null
        return 'skipped — no conversation id from the store probe';
      }
      resumedDrive = new ctx.Drive({
        command: bin,
        args: ['--conversation', turnSessionId],
        cwd: ctx.cwd,
        artifactsDir: ctx.artifactsDir,
        label: 'agy-resume',
      });
      ctx.drive = resumedDrive;
      const painted = await resumedDrive.waitFor(() => {
        const s = resumedDrive.screen();
        return s.trim().length > 0 && !/signing in/i.test(s);
      }, 60000);
      if (!painted) throw new Error('resumed agy never painted');
      // content_rederives_on_resume: the prior transcript must come back.
      // The scroll-safe needle is the REPLY TOKEN (unique, near the
      // transcript tail) or the prompt head — on 1.2.11 a two-turn
      // transcript scrolls the first prompt off-screen and the head-only
      // check false-negatived while the rederive was ON SCREEN (baseline
      // run 2026-09-26: rederive painted, fact said false).
      const rederived = await resumedDrive.waitFor(
        () => {
          const s = resumedDrive.screen();
          return s.includes(prompt.slice(0, 24)) || s.includes(replyToken);
        },
        90000,
        500,
      );
      // 1.2.11: a resumed COMPLETED conversation rederives, idles a beat,
      // then self-exits (the 1.2.3 exit-0 candidate again). Watch briefly
      // and record the exit — the panic probe null-skips on a dead child,
      // so here is where the drift must be written down.
      let selfExit = null;
      const selfDeadline = Date.now() + 10000;
      while (Date.now() < selfDeadline) {
        if (resumedDrive.exitCode !== null) {
          selfExit = resumedDrive.exitCode;
          break;
        }
        await new Promise((r) => setTimeout(r, 500));
      }
      await new Promise((r) => setTimeout(r, 2000));
      resumedDrive.snap('resumed');
      const runLock = path.join(AGY_HOME, 'presence', `${turnSessionId}.lock`);
      const lockExists = fs.existsSync(runLock);
      const lockHolders = lockExists ? presenceLockHolders() : null;
      // existence alone reads lies (the locks leak) — live means fd-held NOW
      const lockBack = lockExists && !!lockHolders && (lockHolders[runLock] || []).length > 0;
      ctx.facts.resume = {
        resumed_session_id: turnSessionId,
        rederived: !!rederived,
        resumed_child_self_exit: selfExit,
        presence_lock_after_resume: lockBack,
        presence_lock_exists: lockExists,
      };
      if (selfExit !== null) {
        return rederived
          ? `rederived ok; child SELF-EXITED code ${selfExit} (the 1.2.3 exit-0 candidate, live again on 1.2.11); presence lock fd-held: ${lockBack}`
          : `never rederived and child self-exited code ${selfExit}`;
      }
      return rederived
        ? `rederived ok; child alive; presence lock fd-held: ${lockBack}`
        : 'resumed but prior transcript never rederived';
    });

    // 9. panic falsifier: SIGKILL the resumed child; the drive must NOTICE.
    await ctx.probe('panic-falsifier', async () => {
      const target = resumedDrive && resumedDrive.exitCode === null ? resumedDrive : null;
      if (!target) {
        ctx.facts.panic_falsifier = null;
        return 'skipped — no live resumed child to kill';
      }
      const code = await target.killChild();
      if (code === null) throw new Error('child kill did not surface as pty exit');
      return `exit ${code}`;
    });
  },
};
