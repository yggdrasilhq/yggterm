# Architecture Audit, 2026-05-16

This audit records why terminal, session, hot-update, and theme regressions kept
returning even after telemetry, app-control probes, and smoke tests were added.
The repeated failure was not lack of instrumentation. It was source-of-truth
drift: fixes were allowed to satisfy one observer while bypassing the interface
contract that owns the behavior.

Yggterm's stable promise is narrow. A session should behave like durable,
snappy automation of:

```bash
ssh <machine>
cd <cwd>
codex resume <uuid>
```

Everything else supports that routine. Nothing else may become terminal truth,
session truth, daemon truth, or theme truth.

## Authority Table

| Domain | Authoritative Source | Valid Observers | Forbidden Alternate Truth |
| --- | --- | --- | --- |
| PTY lifecycle and byte stream | `yggterm-headless` daemon terminal runtime | app-control, telemetry, traces, screenshots | transcript preview, retained metadata, GUI state |
| Terminal parsing and painting | xterm.js buffer, renderer, cell attributes, viewport | canvas pixels, xterm buffer probes, app-control host fields | shell DOM text overlays, prompt/cursor repair layers |
| Web View conversation presentation | Provider-declared transcript/API model | conversation DOM attributes, app-control preview state, screenshots | terminal xterm buffer, row title, generated summaries alone |
| Session identity | saved Codex transcript id when available, otherwise durable session id | sidebar rows, cwd projection, title DB, telemetry | synthetic runtime keys, row titles, generated summaries |
| Live session retention | daemon runtime table plus preserved-owner registry | sidebar live rows, app-control runtime truth | keep-alive dot alone, retained xterm host alone |
| Hot update | protocol state machine in `docs/protocol.md` | monitor, server-list, app-control, screenshots, resource samples | install-state alone, readable retained buffer, stale daemon status |
| Theme and chrome | stable `YgguiThemeSpec` after stable clamping | app-control CSS variables, screenshots, smoke tests | compositor blur side effects, saved legacy alpha/grain |
| Telemetry | append-only incident observation | SQLite queries, perf traces | recovery decisions, render content, input routing |
| Smoke tests | release gate witnesses | screenshots, probes, telemetry queries | product behavior definition |

If two sources disagree, the owner in this table wins and the disagreement is an
incident. Do not patch the observer to make the disagreement disappear.

## Failure Answers

### 1. Why did the Codex prompt line change color?

The prompt line color came from multiple inputs being treated as equivalent:
xterm theme defaults, PTY SGR attributes, Codex's terminal identity probe,
exported remote environment (`TERM_PROGRAM`, `COLORFGBG`,
`YGGTERM_TERMINAL_APPEARANCE`), retained runtime launch age, and shell theme
experiments.

The specific jojo finding in `docs/xterm.md` showed Codex clearing the prompt
line with default background attributes, for example `ESC[0m ESC[49m ESC[K`,
without sending a `48;...m` cell background. xterm.js then correctly painted the
default terminal background. That is not a shell chrome problem and must not be
fixed with an overlay. The fix path is terminal identity, PTY bytes, xterm theme
mapping, or Codex-side behavior.

### 2. Why did the TUI break at the end but still work?

The process and PTY were still alive, so input and application state continued.
The visible xterm surface was stale or partially repainted. We repeatedly saw
this when retained hosts, activation repaint, recovery snapshots, low-power TUI
logic, or hot-update handoff replay gave xterm an incomplete cursor-addressed
frame.

"Still works" proves daemon/runtime survival only. It does not prove render
truth. The acceptance gate must require daemon runtime truth, xterm buffer
truth, visible pixels, cursor placement, input echo, and no recovery overlay.

### 3. Why did fast characters skip displaying, and why did a newline fix it?

The Rust write bridge and embedded xterm script treated synchronized Codex
repaint frames as collapsible high-volume output. The PTY accepted every typed
byte, but xterm parsed a later partial repaint without earlier clear/paint
frames, so characters were registered but missing on screen. Newline or later
output forced a fresh repaint, masking the original byte-loss bug.

The stable law is now explicit: batching may affect flush timing only. It must
never drop, reorder, trim, deduplicate, coalesce, or rewrite PTY bytes.

### 4. Why did manual force redraw terminal do nothing?

Manual redraw refreshed xterm's current buffer and renderer. It could not
recreate bytes already dropped before xterm parsed them. It also could not prove
that a retained replay source was correct, that daemon geometry matched, or that
a stale xterm host belonged to the selected runtime.

The command should be treated as `refresh current xterm renderer`, not `repair
terminal`. If redraw changes behavior, it is evidence of a renderer-settle bug.
If redraw does not change behavior, the missing truth is earlier in the stream,
identity, geometry, or runtime route.

### 5. Why did internal DOM or transport text leak into the terminal?

Internal control/status/error text was allowed too close to the PTY render
path, then later layers tried to sanitize it after it was already mixed with
terminal payloads. That created post-hoc filters for strings like stale-daemon
warnings or terminal-session-not-found errors.

The correct boundary is upstream: internal Yggterm control messages must never
enter PTY application output. Any sanitizer is a quarantine and diagnostic
guard, not a normal render path.

### 6. Why did stale daemons not hot update?

Hot update had several partial truths: install-state version, active GUI path,
daemon socket, server protocol, preserved-owner registry, daemon runtime table,
and sidebar live rows. Fixes sometimes proved one of those and skipped the full
handoff state machine.

Per `docs/protocol.md`, a stale daemon that owns a live PTY is not disposable.
It remains a preserved owner until the new daemon has adopted or explicitly
routed that PTY and the verified state has passed. A stale daemon that does not
own a live PTY must not keep participating in mutation paths just because it is
reachable.

### 7. Why do keep-alive sessions break nondeterministically on restart?

Keep Alive is a retention request, not identity and not a viewport source. The
breakage came from treating several projections as if they independently proved
the session: keep-alive metadata, live sidebar row, cwd row, retained xterm host,
daemon snapshot, preserved-owner registry, and saved transcript identity.

After restart, any one of those could arrive before the others. If the GUI
promoted the early projection to truth, it could show a row with no runtime,
reuse a stale host, regenerate copy against the wrong identity, or open a fresh
Codex prompt for an existing saved session.

## Shortcut Classes To Ban

- **Screenshot repair:** drawing shell-owned terminal text, cursor, prompt
  background, or TUI lines to satisfy a visual proof.
- **Observer promotion:** using telemetry, app-control, screenshot text, or
  generated summaries as product truth.
- **Post-hoc stream cleanup:** filtering internal transport leaks near xterm
  instead of preventing them from entering terminal output.
- **Partial hot-update proof:** accepting version, install-state, or readable
  retained output without runtime ownership and input proof.
- **Identity substitution:** using runtime keys, row labels, titles, summaries,
  or cwd placement as saved-session identity.
- **Theme side effects:** allowing stable theme behavior to depend on alpha,
  blur, grain, compositor focus timing, or a branch-only experiment.
- **Low-power render substitution:** dropping TUI frames or drawing simplified
  TUI text outside xterm to save CPU.
- **Generic smoke proof:** adding a broad "looks loaded" check instead of a
  detector for the exact defect that escaped.

## Required Investigation Order

Before changing behavior for a regression:

1. Name the authoritative source of truth from the table above.
2. Capture its state with a read-only probe.
3. Capture each observer that disagrees.
4. State the exact contract violation in the relevant `docs/*.md` file.
5. Add or identify the smallest deterministic test/probe that fails on that
   violation.
6. Only then patch runtime behavior.
7. Prove the owner, observers, screenshot, telemetry, and smoke gate now agree.

If this order cannot be followed, write why in the issue or todo before making a
runtime change.

## Refactor Targets

The audit found that `crates/yggterm-shell/src/shell.rs` is too large to protect
source-of-truth boundaries by review alone. It mixes shell rendering, xterm JS
generation, app-control probes, session projection, theme, hot-update recovery,
and thousands of tests in one file.

Stabilization should split by ownership, not by convenience:

- terminal xterm embed and viewport probes
- session/sidebar projection and metadata copy
- theme/chrome rendering
- startup, hot-update, and recovery orchestration
- app-control and telemetry schemas

The split must not change runtime behavior by itself. Each extraction should
carry the existing tests for that ownership boundary with it.

First extraction boundary: terminal write classification now belongs in
`crates/yggterm-shell/src/terminal_write_policy.rs`, and lossless PTY staging
belongs in `crates/yggterm-shell/src/terminal_write_bridge.rs`. Shell code may
call that policy, but it must not recreate a second terminal-write classifier.
Stable theme/chrome feature gates now belong in
`crates/yggterm-shell/src/theme_contract.rs`; shell rendering may consume those
gates but must not redefine stable blur behavior locally.
Startup stale-daemon and preserved-owner selection now belongs in
`crates/yggterm-shell/src/hot_update_policy.rs`; launch orchestration may call
that policy but must not duplicate its ranking or session-survival predicates.
GUI telemetry append, rotation, and duplicate-throttle rules now belong in
`crates/yggterm-shell/src/ui_telemetry.rs`; shell interaction code may emit
events, but it must not keep a second telemetry writer or suppression policy.
Session copy/title heuristics now belong in
`crates/yggterm-shell/src/session_copy_policy.rs`; shell projection code may
consume those decisions, but it must not keep a second passive-generation gate or
low-signal title classifier.

## Stable Release Gate

The first stable release is blocked until:

- the stable docs agree on the authority table above
- `AGENTS.md` requires source-of-truth declaration before fixes
- terminal release smokes reject shell-owned terminal overlays and byte
  coalescing
- hot-update proof includes current daemon identity, preserved-owner truth,
  active session identity, input, screenshot, and resource budget
- session proof includes live row, cwd row, metadata title/summary, runtime key,
  and saved transcript identity reconciliation
- theme proof reports stable no-blur/no-alpha/no-grain behavior
- telemetry is queried for switching-pass recovery, blank surface recovery,
  stale owner events, input-without-echo, and retained-host mismatch
