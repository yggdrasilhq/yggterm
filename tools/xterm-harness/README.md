# xterm.js client-layer behavioral harness

Deterministic, headless behavioral tests for the **exact vendored** xterm.js the
app ships (`assets/xterm/xterm.js`), run under jsdom in Node. This is the
client-layer half of the determinism story: `crates/yggterm-server/tests/pipeline_integration.rs`
guards the **daemon pipeline** (pre-xterm.js); this guards **xterm.js buffer /
scrollback / reflow behavior** that our client fixes depend on.

Why faithful: we `require()` the same UMD `assets/xterm/xterm.js` bundle that is
`include_str!`'d into the GUI — not a different `@xterm/headless` build — so the
behaviors asserted here are the ones that actually run in the WebKit webview.

## Run

```bash
cd tools/xterm-harness
npm install      # jsdom (gitignored; first run only)
npm test         # node --test
```

(The `HTMLCanvasElement.getContext` jsdom warning is expected and harmless — we
assert the **buffer**, not the canvas renderer.)

## What it guards today

- normal newline output grows scrollback and retains scrolled-off lines
- cursor-addressed repaint (codex steady-state) keeps `baseY` at 0 — no scrollback
  (why the daemon's clean-scrollback base_y is correctly 0 for codex)
- reverse-index inside a scroll region does not grow scrollback (codex open-space)
- a painted background colour survives a column-widen reflow for written cells
  (baseline for the composer bg-split invariant)

## Extending (campaign next steps)

The campaign's D6 (scroll-flicker) and D1/D4 behavioral guards plug in here by
extracting the relevant client decision logic (the scrollback-intent /
follow-decision state machine in `crates/yggterm-shell/src/shell.rs`) into a
small testable JS module and asserting it against this harness — converting
"fixed again" into a deterministic guard. See `campaign-xterm-dealbreakers`.
