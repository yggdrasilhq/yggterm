# tools/audit — independent completion-audit instruments

Swarm-harness2 turn-6 unit 1 (chain
`msgGraph/lores/chain-of-thought/swarm-harness2/2026-10-26-sol-high-turn6.md`,
board infra/yggterm ACK-48dde30521): make product-level completion evidence
independent of the implementer's claim.

## startup_evidence.py — the installed-build startup-evidence runner

Launches a yggterm GUI binary **hermetically** (scratch HOME/XDG_DATA_HOME,
Xvfb + dbus-run-session — the mergekey-rig launch shape), collects the F11
startup evidence, and emits an immutable evidence bundle with a
**PASS / FAIL / UNKNOWN** verdict:

- `manifest.json` — verdict, reasons, identity block, per-file sha256 digests
- `gui.log` — raw GUI stdout/stderr (panic signatures are scanned from here)
- `trace-excerpt.jsonl` — the trace lines the verdict relied on
- `screenshot.png` — the visual leg (required for PASS)

### Verdict semantics (sol turn-6 Q3 law)

- **PASS**: daemon ready, zero panic signatures, `component_window` events
  present (the F11 DOM-empty signature absent), artifact identity proven from
  the run's OWN trace ([11.231]), screenshot captured. Missing any leg caps
  the verdict at UNKNOWN — missing evidence never PASSes.
- **FAIL**: the product demonstrably failed its startup contract — never
  became ready, a panic signature in the log, or zero component_window
  events in the small-world environment.
- **UNKNOWN**: an instrument fault, an unprovable identity (no trace
  executable_path / unreadable or mid-run-replaced served exe), or a missing
  required leg. UNKNOWN certifies nothing and is never laundered into PASS.

### Scope boundary

Hermetic trials certify the **artifact** in hermetic environments (real-home
F11 recovery vs redirected-home DOM-empty both belong to acceptance, but the
owner's real production profile is `yggterm-deeptest` territory —
owner-present). The production panic witness remains the owner's
screenshot until a deeptest pass replays it.

### Identity law ([11.231], hardened per sol turn-6 Q3)

The launcher may silently hand off to `~/.local/share/yggterm/direct/builds/<sha>`
when the real user state is visible. The bundle therefore records the trace's
own `executable_path`, the served exe's sha256 **pre and post run** (mid-run
replacement invalidates), and the launcher's hash. `--strict-identity` adds
the equality requirement for worktree-binary audits.

### Usage

```sh
python3 tools/audit/startup_evidence.py --selftest          # decision tests + fail/mismatch controls
python3 tools/audit/startup_evidence.py                     # 1 hermetic trial of the installed build
python3 tools/audit/startup_evidence.py --binary <path> --trials 3
python3 tools/audit/startup_evidence.py --binary <worktree-bin> --strict-identity
```

`--binary` takes the **GUI binary** (`~/.local/bin/yggterm`, or a direct-build /
worktree `yggterm`) — not the `yggterm-headless` CLI, which has no GUI to launch.
Every probe (ready, screenshot) runs under the SAME scratch env as the GUI, so a
probe can never answer from the owner's live production daemon.

Bundles land in `~/.local/state/yggterm-audit/<attempt-id>/`. Exit code is 0
only when every trial PASSed.

### Falsifiers (what would break this instrument)

The runner reporting PASS on a wrong binary, an empty Root render, a
swallowed panic, a missing observation, or an absent run. The selftest's live
controls pin the two cheapest: the fail-control stub (panics, never serves →
must FAIL) and the mismatch-control stub (serves a trace naming another
binary → must never PASS).
