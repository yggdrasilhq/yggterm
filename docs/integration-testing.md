# Spec: reproducible end-to-end integration testing

Status: SPEC + Phase A in progress (2026-06-03). Motivation: every terminal-pipeline
regression this project has shipped was a **deterministic** bug found by the user on
the live machine, because there was no reproducible harness below the full GUI. This
spec defines that harness. See `[[incident-gap-fix-cascade-2026-06-03]]`.

## Principle

Convert "ship to jojo and hope; the user finds it broken" into "a red test before
it leaves my machine." The harness must be **deterministic** (no real codex/CC, no
network, no timing luck) and must assert on **terminal/protocol state**, not pixels.

## The data path under test

```
mock-tui (scripted escape codes)  ──PTY──>  yggterm server (daemon)
   reader thread -> chunk ring (seq) + vt100 scrollback ring
      ──read(cursor) / SSH──>  client read-bridge  ->  xterm.js buffer
```

Seam to inject: the **PTY source** (`mock-tui` in place of codex/CC/shell). NOT the
transport — use real loopback (local server; `ssh 127.0.0.1` for the remote path) so
we test the real protocol, not a mock of it.

## Components

### 1. `mock-tui` — deterministic TUI byte source (a real binary)
A small program the daemon spawns as a session's PTY process. It emits a **scripted**
sequence of bytes/escape codes and exits (or idles) deterministically. Two input modes:
- **DSL / named scenarios** (built-in): the pathological patterns that have bitten us —
  `alt-screen-enter`, `alt-screen-exit`, `clear-storm` (repeated `\x1b[2J\x1b[H`),
  `partial-redraw` (cursor-addressed cell updates without full repaint),
  `burst <bytes>` (high-volume to exercise ring trim), `grow-scrollback <rows>`,
  `prompt-box` (codex-style bordered prompt), `idle`, `interleave`.
- **Trace replay** (fixtures): replay a recorded real codex/CC PTY byte stream from
  `tests/fixtures/tui/*.bytes` for realism. (Capture path: tee a real session's PTY.)
Invocation: `mock-tui --scenario alt-screen-enter` or `mock-tui --replay <file>`.
Deterministic: optional `--paced <ms>` for inter-chunk delay; default = emit-then-hold.

### 2. Daemon-pipeline integration tests (Phase A — highest value, no GUI)
Spawn a real `PtySessionRuntime` (or `TerminalManager`) whose launch command is
`mock-tui --scenario X`, let it run, drive `read(cursor)` (incl. forcing ring trims),
and assert on the returned chunks + the vt100 replay + buffer mode. Catches: silent
chunk-gap drop, alt-screen-aware re-sync, scrollback retention, resize replay.

### 3. Recovery-decision tests (Phase A — pure functions, no GUI)
The client recovery/gate decisions are pure functions in `shell.rs`
(`retained_ready_remote_empty_surface_should_recover`,
`retained_remote_surface_should_wait_for_prompt_ready`, the settle gates,
`rearm_stale_retained_fault_recovery`). Drive them with a **sequence** of evolving
host-health inputs that mimic a streaming session (mid-output non-prompt/empty frames
that resolve) and assert **no recovery fires while output is flowing**. This is the
test that would have caught the recovery churn.

### 4. Headless xterm.js buffer tests (Phase B)
Feed the same byte stream to a headless xterm.js (Node build) and assert `buffer.active`
state (content, alt/normal mode, scrollback length, cursor). Mirrors xterm.js's own
test approach.

### 5. GUI e2e (Phase C — small, slow)
Real WebKit under Xvfb + a server full of `mock-tui` sessions, driven by `yggui`
app-control (already half-built on jojo), screenshot + assert. Reserved for genuine
render-timing bugs (reveal scrollTop flicker, focus). Keep it minimal.

## Assertions that become regression gates
- No missing middle: emit N chunks, trim, read → every byte accounted for or an
  explicit re-sync (never a silent gap).
- Alt-screen re-sync never replays normal-buffer history (the 2.8.12/2.8.14 bug).
- No `resume_recovery`/`bootstrap_reset`/`empty_surface_recovery_begin` while output
  is actively flowing (the churn).
- Scrollback row count preserved across re-sync / reconnect.
- Buffer mode (alt vs normal) tracks the byte stream.
- Input stays enabled through a streaming session.

## Phasing
- **A (now):** `mock-tui` binary + daemon-pipeline integration tests + recovery-decision
  sequence tests. No GUI, fast, CI-able. Catches ~80% (everything that broke this
  session except the pure render-timing flicker).
- **B:** headless xterm.js buffer assertions.
- **C:** Xvfb + app-control GUI e2e.

## Workflow change (binding)
Per `[[incident-gap-fix-cascade-2026-06-03]]`: no terminal-pipeline change
(read/replay/seed/recovery/scroll) ships to a live machine without a green harness
test exercising BOTH normal and alternate-screen buffers AND a streaming session.
