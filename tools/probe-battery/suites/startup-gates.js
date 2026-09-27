// tools/probe-battery/suites/startup-gates.js — the [99.1] startup-gate
// audit (one drive per CLI installed on this host, each into a NEVER-OPENED
// directory, reading the rendered grid the way the user's viewport does).
//
// The question this suite re-asks of whatever binaries are installed NOW:
// does the CLI park a startup/trust gate between spawn and composer in a
// directory it has never seen — the state the classifier can only see
// through `startup_gate_screen_phrases`, and which the descriptors declared
// EMPTY (= UNMEASURED, the [99.1] defect) for most of the family?
//
// Per CLI the suite records: the first settled screen (verbatim lines), the
// trust-family lines on it, and — when the composer is reachable without
// answering anything — whether the DECLARED composer glyph is on the idle
// render. ⛔ The suite never answers a gate by itself: on a numbered picker a
// wrong keypress can QUIT the CLI or SELECT a paid option ([11.97] class).
// When a gate is detected the drive STOPS there; passing it and reading the
// composer behind it is the seat's manual follow-up.
//
// Sandbox law (measured on the muse lab host): the per-CLI cwd is a fresh
// mkdtemp under ~/probe-lab (never-opened by construction; NOT /tmp — /tmp is
// SHARED on this host and a sibling seat's staging has been overwritten
// mid-lane before). opencode additionally gets the scratch-HOME + port
// relocation its own suite measures as mandatory.
//
// MEASURED 2026-09-27 (muse lab host, all four drives green):
//   opencode 2.0.3   — NO gate (composer direct, with AND without --auto);
//                      declared ┃ U+2503 confirmed on the idle render.
//   grok 1.0.30      — the unauth device-code SIGN-IN is the first screen
//                      ("Approve in your browser to finish signing in." /
//                      "Waiting for approval..." / "ctrl+q  quit"): a gate
//                      by the field's own doctrine — auth-scoped, not
//                      directory-scoped. Declared ❯ NOT re-askable behind it.
//   kimi 1.52.0      — UNMEASURABLE: the binary is now a deprecation SHIM
//                      that auto-runs the code.kimi.com install script
//                      ("kimi-cli is no longer maintained"); the drive
//                      rendered the installer, not a TUI. ([11.175])
//   zcode-tui 0.6.14 — NO gate (composer direct; first-party source has no
//                      directory-trust gate). Declared ▏ U+258F draws
//                      NOWHERE — the live input head is ▌ U+258C (marker
//                      corrected in the same sitting).
//
//   node run.js --suite suites/startup-gates.js --cwd <any-dir> \
//        --artifacts ~/probe-lab/991-artifacts \
//        [--suite-arg clis=opencode,kimi,grok,zcode-tui] \
//        [--suite-arg bin.opencode=opencode2]

const fs = require('fs');
const os = require('os');
const path = require('path');
const net = require('net');

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// The DECLARED composer glyphs (agent_cli.rs at the time of writing) — the
// audit re-asks each of the live idle render. `null` marker = the CLI whose
// readiness rides the region label (kimi), nothing to re-ask.
const DECLARED_MARKER = {
  opencode: '\u{2503}',
  kimi: null, // composer_region_label Some("input"); declared ❯ is absent BY MEASUREMENT
  grok: '\u{276f}',
  'zcode-tui': '\u{258f}',
};

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

function screenLines(drive) {
  return drive
    .screen()
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l.length > 0);
}

module.exports = {
  name: 'startup-gates',

  async run(ctx) {
    const which = (ctx.args.clis || 'opencode,kimi,grok,zcode-tui')
      .split(',')
      .map((s) => s.trim())
      .filter(Boolean);
    const binOf = (cli) => ctx.args[`bin.${cli}`] || cli;

    const stageRoot = path.join(os.homedir(), 'probe-lab', 'startup-gates');
    fs.mkdirSync(stageRoot, { recursive: true });

    for (const cli of which) {
      await ctx.probe(`gate:${cli}`, async () => {
        const bin = binOf(cli);
        const cwd = fs.mkdtempSync(path.join(stageRoot, `${cli}-ws-`)); // never-opened
        const facts = { bin, cwd, gate_detected: null, trust_lines: [], picker_lines: [], declared_marker: DECLARED_MARKER[cli] ?? null, marker_present: null, screen: [] };

        let drive;
        if (cli === 'opencode') {
          // ISOLATION FIRST (the opencode suite's measured law): scratch HOME
          // + scratch-local managed port, before any Drive exists.
          const scratchHome = fs.mkdtempSync(path.join(stageRoot, 'opencode-home-'));
          process.env.HOME = scratchHome;
          const port = await freePort();
          const { execSync } = require('child_process');
          execSync(`${bin} service set port ${port}`, { env: process.env, cwd, encoding: 'utf8', timeout: 30000 });
          facts.managed_port = port;
          drive = new ctx.Drive({ command: bin, args: ['--auto'], cwd, artifactsDir: ctx.artifactsDir, label: `gate-${cli}` });
        } else {
          drive = new ctx.Drive({ command: bin, args: [], cwd, artifactsDir: ctx.artifactsDir, label: `gate-${cli}` });
        }

        try {
          const painted = await drive.waitFor(() => drive.screen().trim().length > 0, 90000);
          if (!painted) throw new Error(`${cli}: no paint within 90s`);
          await sleep(5000); // let the first screen fully settle
          drive.snap('first-screen');
          const lines = screenLines(drive);
          facts.screen = lines.slice(0, 40);
          facts.trust_lines = lines.filter((l) => /trust/i.test(l));
          facts.picker_lines = lines.filter((l) => /^\s*\d[\.\)]\s*\S/.test(l) || /no,?\s*(quit|exit)/i.test(l));
          facts.gate_detected = facts.trust_lines.length > 0;

          if (facts.gate_detected) {
            // ⛔ STOP: a picker's wrong keypress quits or spends. The composer
            // behind the gate is the seat's manual follow-up.
            facts.marker_present = null;
            facts.note = 'gate on the first screen; drive stopped before answering it';
          } else {
            const glyph = DECLARED_MARKER[cli];
            facts.bottom_lines = lines.slice(-8);
            facts.marker_present = glyph == null ? 'n/a (region-label readiness)' : lines.some((l) => l.includes(glyph));
          }
        } finally {
          try { drive.dispose(); } catch (_) {}
        }
        Object.assign(ctx.facts, { [cli]: facts });
        return facts.gate_detected
          ? `${cli}: GATE on first screen (${facts.trust_lines.length} trust lines)`
          : `${cli}: no gate — composer reachable unasked`;
      });
    }
  },
};
