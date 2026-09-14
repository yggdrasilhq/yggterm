# tools/probe-battery — the T2 probe battery (stone §5)

Per-CLI drive suites: launch → turn → working-phases → title → resume →
in-TUI switch (B) → panic falsifier. Each probe **launches its own child**
(node-pty), drives the EXACT vendored xterm.js the app ships, and emits the
**measured** data that fills descriptor schema v2 (phrase tables, composer
marker, recency/resume behavior). On demand and isolated — this is the T2
tier; store-watching is T1, kernel tracing is T3 and almost never on.

Lineage: graduated from the muse/kimi/grok probe lab
(`~/.yggterm/scratchpad/zseat-d-muse-probe-112700/`, the muse seat's
node-pty + CPR pattern), where the defining trap was measured:

> ⛔ **A naive pty drive renders nothing.** CLIs that query the cursor
> (DSR `\x1b[6n`) stall forever without a CPR answer, and assertions off raw
> bytes are not what the viewport shows. `drive.js` answers CPR from the
> rendered buffer and every assertion reads the rendered screen.

## Usage

```sh
npm install --prefix tools/probe-battery   # per-host: jsdom + node-pty
cargo build -p yggterm-server --bin mock-tui   # only for the mock-tui suite
node tools/probe-battery/run.js \
  --suite tools/probe-battery/suites/mock-tui.js \
  --cwd /tmp/probe-ws \
  --out /tmp/mock-tui-report.json
```

Suites: `suites/mock-tui.js` (CI-reachable reference — proves the engine), 
`suites/zcode-tui.js` (the §9 reference CLI — filled and green against
main zcode-tui 0.5.9, 2026-09-14; the measured-chrome regression net for
that binary). Add one file per CLI; keep per-CLI state in the
per-CLI campaign door (`campaign-cli-integration/<cli>.md`), defects in
`docs/pending-bugs.md` under the CLI's `11.6.N` id.

## The report

`report.json` carries `capabilities` (schema-v2-keyed measured facts:
`working_screen_phrases`, `composer_marker`, `title_source`, `resume`, ...)
and per-probe `pass|fail` rows with timings. A capability a suite cannot
answer is `null` + why — **a null is honest, a guess is the defect this
campaign exists to kill.** Artifacts (raw pty bytes, per-step screen snaps,
the timeline log) land in `--artifacts` for falsification.

## Contract with the campaign

- The battery never edits descriptors — it MEASURES; the per-CLI seats fill.
- The reattach SLA scenario is the Rust-side twin:
  `crates/yggterm-server/tests/reattach_sla_integration.rs` (wrapper-level,
  ledger-served vs empty-ledger counterfactual).
- PTY law applies to any "works" claim a suite seems to justify: the battery
  drives ITS OWN child in isolation; a row on the live daemon is proven by
  switching to the row and reading the viewport.
