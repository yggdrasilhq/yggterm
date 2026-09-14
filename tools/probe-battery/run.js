#!/usr/bin/env node
// tools/probe-battery/run.js — the T2 probe-battery runner (stone §5, §8:
// "yobserve T0-T2 as the probe-battery runner").
//
//   node run.js --suite suites/mock-tui.js --cwd /tmp/probe-ws \
//        [--artifacts /tmp/probe-artifacts] [--out report.json] [--suite-arg k=v]...
//
// A suite is `async run(ctx)` with
//   ctx = { Drive, repoRoot, artifactsDir, cwd, args, log, probe(name, fn) }
// and reports measured facts via ctx.facts (schema-v2-keyed) and probes.
//
// The report is the deliverable a per-CLI seat reads to fill its descriptor
// v2 block — measured, not guessed. A suite that cannot answer a capability
// records `null` + why; ⛔ a null is honest, a guess is the defect this
// campaign exists to kill.

const fs = require('fs');
const path = require('path');
const { Drive, REPO_ROOT } = require('./drive');

function arg(flag, fallback) {
  const i = process.argv.indexOf(flag);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}

async function main() {
  const suitePath = path.resolve(arg('--suite', '') || '');
  if (!suitePath || !fs.existsSync(suitePath)) {
    console.error('usage: run.js --suite suites/<cli>.js --cwd <dir> [--out report.json] [--suite-arg k=v]...');
    process.exit(2);
  }
  const cwd = path.resolve(arg('--cwd', process.cwd()));
  const artifactsDir = path.resolve(
    arg('--artifacts', `/tmp/probe-battery-${path.basename(suitePath, '.js')}-${Date.now()}`),
  );
  fs.mkdirSync(artifactsDir, { recursive: true });
  const outPath = arg('--out', path.join(artifactsDir, 'report.json'));

  const suiteArgs = {};
  const argv = process.argv.slice(2);
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--suite-arg' && argv[i + 1]) {
      const [k, v] = argv[i + 1].split('=');
      suiteArgs[k] = v === undefined ? true : v;
    }
  }

  const suite = require(suitePath);
  const report = {
    suite: suite.name || path.basename(suitePath),
    started_at: new Date().toISOString(),
    cwd,
    artifacts: artifactsDir,
    // schema v2 five-capability answers, as measured by this suite
    capabilities: {},
    probes: [],
  };
  const ctx = {
    Drive,
    REPO_ROOT,
    artifactsDir,
    cwd,
    args: suiteArgs,
    facts: report.capabilities,
    log: (m) => console.log(`[suite] ${m}`),
    probe: (name, fn) => {
      const t0 = Date.now();
      return Promise.resolve()
        .then(fn)
        .then((detail) => {
          report.probes.push({ name, status: 'pass', ms: Date.now() - t0, detail: detail ?? null });
          console.log(`[pass] ${name} (${Date.now() - t0}ms)`);
        })
        .catch((err) => {
          report.probes.push({
            name,
            status: 'fail',
            ms: Date.now() - t0,
            detail: String((err && err.message) || err),
          });
          console.log(`[FAIL] ${name}: ${(err && err.message) || err}`);
        });
    },
    drive: null,
  };

  try {
    await suite.run(ctx);
  } catch (err) {
    report.fatal = String((err && err.stack) || err);
    console.error(`[fatal] ${report.fatal}`);
  }
  if (ctx.drive) ctx.drive.dispose();

  report.finished_at = new Date().toISOString();
  fs.writeFileSync(outPath, JSON.stringify(report, null, 2));
  console.log(`report → ${outPath}`);
  const failed = report.probes.filter((p) => p.status === 'fail').map((p) => p.name);
  process.exit(report.fatal || failed.length ? 1 : 0);
}

main();
