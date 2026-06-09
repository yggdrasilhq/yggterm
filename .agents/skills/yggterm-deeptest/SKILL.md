---
name: yggterm-deeptest
description: Agent-driven deep-test pass on the LIVE yggterm install after a significant or structural bugfix, before claiming it shipped. The agent drives session switching, probing, screenshots, telemetry, and the salvaged release-gate checks; the user does ONLY the steps a remote agent cannot perform — OS-level focus changes (fg↔bg, minimize/restore) and final human-eye confirmation of sub-second visual transients. Gated on the live host being free to use.
---

# yggterm deeptest

This skill is the successor to the old `docs/23-smoke-tests.md` release-gate
protocol for the LIVE half: a structured, repeatable sweep the agent runs itself
over a real multi-session install to catch the recurring xterm / session-graph /
viewport / resource defects — instead of asking the user to "switch sessions and
tell me what you see." It composes the lower-level skills:

- **`yggui-app-control`** — the hands+eyes primitives (`app open`, `app state`,
  `app rows`, `server snapshot`, `terminal send/submit`, telemetry). Read it first.
- **`yggterm-diagnostics`** — the FAITHFUL screenshot + crop/zoom + resize/reconcile
  recovery tooling (for any visual claim).

Deterministic, code-level invariants do NOT belong here — they belong in the
offline suites (`cargo test -p yggterm-shell --lib`, `-p yggterm-server [--test
pipeline_integration]`, `tools/xterm-harness`). deeptest is ONLY for what genuinely
needs a live multi-session GUI. See "Salvage map" at the bottom for which old
23-test checks went where.

## When to run

Run a deeptest pass when BOTH are true:
1. You just landed a **giant / structural** fix (terminal pipeline, viewport/scroll,
   session graph, daemon lifecycle, render-scope) and are about to claim it shipped.
   Trivial/local edits don't need it — offline tests + a single faithful screenshot suffice.
2. The live host is **FREE to use** (the user is not actively working on it). Establish
   this one of two ways:
   - **Standing grant:** the user has said in this conversation that the host is free
     (e.g. "the machine is free, use it" / "I'm idle"). Then proceed autonomously.
   - **Otherwise ASK** before starting: "Issue X is fixed locally — may I run a deeptest
     pass on the live host? It re-resumes live sessions." Wait for the go-ahead.
   A deploy + deeptest re-resumes the user's sessions; never start one on active work.

## The agent ↔ user contract (READ THIS — it's the whole point)

**The agent drives everything reachable over SSH app-control:**
- switch among sessions and machines (`app open <path> [--view terminal|preview]`)
- query state (`app state`, `app rows`, `server snapshot`, telemetry sqlite)
- faithful screenshots + crop/zoom (yggterm-diagnostics)
- send input to user-GRANTED sessions (`terminal submit`/`send`)
- run the checked-in live scripts (below)
- assert the deterministic live invariants (contract_violations, runtime_truth, …)

**The user does ONLY what a remote agent physically cannot, and the skill must PROMPT
for each and WAIT:**
- **OS focus changes** — foreground↔background the window, minimize/restore, click to
  focus. On Wayland these can't be synthesized and several of the worst bugs
  (bg→fg reveal blink, focus-regain viewport jump) ONLY trigger on a real focus event.
- **Final human-eye confirmation** of a sub-second visual transient (a blink/flash a
  still screenshot can't catch). The agent's faithful screenshot proves the *settled*
  frame; the user's eye is the authority on the *transient*.

Never claim a focus-triggered or transient symptom "fixed" from agent probes alone —
hand that specific step to the user with a precise "do X, then tell me Y" instruction.

## Host (never hardcode the private name — public repo)

```bash
LIVE_HOST=$(cat .agents/config/live-host)   # gitignored; holds the private SSH alias
BIN="~/.local/bin/yggterm"
HBIN="~/.local/bin/yggterm-headless"
```

Pass `--host "$LIVE_HOST"` to every script explicitly. (`scripts/live_mode_cycle_check.py`
still defaults `--host` to a baked-in private name — a leak; always override it, and
fix that default when convenient.)

## Preconditions (cheap, do every run)

1. **Version parity** — the fix is actually running: `ssh "$LIVE_HOST" "$HBIN server status"`
   (daemon `server_version`) and `ssh "$LIVE_HOST" "$BIN --version"` (GUI). Both must be
   the build containing the fix. A stale daemon/GUI = you're testing the OLD code.
2. **Single daemon, no split-brain** — `pgrep -af 'yggterm-headless.*server daemon'` shows
   exactly one; `server monitor --scenario server-list` shows no runtime key owned by two
   daemons. SIGTERM a lingering old daemon (idle-gated retire is the backstop).
3. **No contract wedge** — `app state` → `session_view_contract_violations == []`.
4. **Let the fleet settle.** After a deploy, remote sessions come back RemoteBootstrap and
   reconnect on first reveal. Do NOT storm `app open` — rapid switches prevent bootstrap
   settling and amplify every symptom (campaign lesson). Open, wait, observe, then move on.

## The pass

Work through these. Each is AGENT-driven unless tagged **[USER]**. Stop and file a defect
(root-cause first, Iron Law) the moment one fails; don't pad a pass.

### A. Session-graph + mount sweep (agent)
- Enumerate live sessions: `server snapshot` → `live_sessions[]` (use FULL paths — never
  truncate; a truncated `app open` path silently lands on the start page).
- Open a representative spread across machines + kinds (codex, CC, shell; local + remote;
  small + GIANT like the user's jyas/practice/antigravity). For each:
  - `active_session_path` matches what you opened; `view == Terminal`; `launch_phase==Running`.
  - faithful screenshot → terminal readable + current; composer bar uniform (no bg-split),
    bottom not clipped, full-width TUI lines coherent.
  - `runtime_truth.active_runtime_present==true`, non-empty surface, input enabled;
    reject ready+daemon_pty+no-surface-problem while input is disabled.
  - cwd placement + sidebar membership deterministic; no ghost/duplicate runtime.

### B. Live-mode cycle (agent, scripted)
```bash
python3 scripts/live_mode_cycle_check.py --host "$LIVE_HOST" --all-live
```
Cycles every Live Session Terminal→Web View→Terminal; asserts each settles in budget,
keeps `active_session_path`, clears `active_surface_requests`, `contract_violations==[]`,
preserves daemon runtime truth, Web View renders without detaching the live runtime.

### C. Loading / render-churn (agent)
- The "DOM leak on loading" is root-render-storm churn, NOT a node leak (host count is
  bounded at ~4 hot sessions). Measure render rate over a real load with the render-cause
  probe (env `YGGTERM_TRACE_RENDER=1`, trace at `~/.yggterm/event-trace.jsonl`) — NOT by
  probing (each app-control read force-re-renders the root ~4×; instruments-lie applies to
  YOU). Open a big RemoteBootstrap session, measure renders/s in the load window; compare to
  the 2.8.58 baseline (sidebar throttle cut load renders 270→65). Regression = churn returns.
- Render-count is the faithful instrument for a perf/churn fix (the faithful-PIXEL rule is
  for VISUAL paint bugs only).

### D. Viewport / scroll (agent + **[USER]**)
- Agent: click the active viewport at random positions (`app terminal` click probe) — must
  not flicker-scroll or land at an unintended scrollback location. Use `viewport_force_log`
  ring as the instrument; NEVER guard the low-level mover.
- **[USER]** focus-regain jump: prompt the user — "scroll session X to the bottom, then
  background and foreground the window; does it stay at the bottom or jump to a remembered
  position?" (the saved-offset-restored-over-PromptFollow bug; un-synthesizable focus event).

### E. Reveal / switch transient (agent screenshot + **[USER]** eye)
- Agent: after the fleet is warm, switch among GIANT sessions and faithful-screenshot the
  composer on each reveal — settled frame must be clean (no broken bottom / bg-split).
- **[USER]** bg→fg blink: prompt — "background then foreground the window a few times on
  session X; do you see a blink/shadow-session flash before it loads?" (reveal-reseed Class A;
  sub-second transient, needs a real focus event + human eye).

### F. Resize coherence (agent if a resize CLI exists, else **[USER]**)
- prompt-follow sessions return to the prompt after a resize/nudge; TUI sessions redraw
  coherently; full-width separators not broken by the terminal-ready phase. If no live resize
  affordance is in the running build, hand the window-resize to **[USER]**.

### G. Keep-alive + restore (agent, heavier — only on a release-candidate pass)
- Tag a keep-alive set, close the GUI without killing keep-alive runtimes (preserve-live
  close path), wait, relaunch, repeat A. Keep-alive runtimes survive; non-keep-alive follow
  the session-closing contract; sidebar grouping deterministic before/after.

### H. Telemetry + resource (agent)
- `~/.yggterm/telemetry/terminal.sqlite3` for the run window: every opened terminal has
  `terminal_open_attempt/begin` + `ready` (or a documented failure/recovery); no burst of
  retained-fault `begin` after the first ready (remount loop = CPU/fan defect); no
  `terminal_resize_from_paint` observer-induced geometry churn.
- Resource windows (baseline / active / cooled / respawn-burst / respawn-settled): no
  unexplained sustained CPU, no surviving swap growth, `dom.css_running_animation_count`
  returns to 0 after settle.

## Honesty rules (these caused past regressions — non-negotiable)
- VISUAL symptom ⇒ FAITHFUL screenshot (`server app screenshot`, check `capture_faithful`),
  NEVER telemetry. Daemon screen (`server snapshot → terminal_lines`) is logical-join text,
  not the visual grid.
- The user's eyes outrank your instruments. If the user reports a symptom and your probes say
  fine, your probes are wrong — investigate the instrument gap.
- `server app terminal reconcile` is DESTRUCTIVE on a HEALTHY session — repair-only.
- Don't over-switch/over-deploy: churn leaves the fleet half-cold and amplifies everything.
- Passing test ≠ live-fixed; live-clean settled frame ≠ transient-gone (hand the transient to
  the user). Report empirical negatives honestly.

## Known-bug catalog this pass targets
The campaign recurring classes (see `campaign-xterm-dealbreakers` memory) + current reports:
reveal-reseed bg→fg blink (E/[USER]), load-churn "DOM leak" (C), viewport scroll-jump on
fg (D/[USER]), composer bg-split bottom-paint (A/E), squish/grid-mismatch (A/F), latency-grows
(H), daemon-restart wedge (preconditions). Map each live finding back to its memory node.

## Salvage map (from docs/23-smoke-tests.md → here vs. offline tests)
- **Deterministic, code-level → offline suites** (NOT deeptest): reflow/scrollback-intent
  invariants → `tools/xterm-harness`; daemon vt100 / replay / grid-persist / write-bridge
  framing → `pipeline_integration` + `-p yggterm-shell --lib`; title/summary copy → core unit
  tests; install-state/launcher identity → unit checks.
- **Live multi-session required → this skill** (sections A–H): mount sweep, live-mode cycle,
  load-churn measurement, reveal/switch transients, keep-alive/restore, telemetry/resource.
- **Human-only → [USER] handoffs**: OS focus (fg↔bg), sub-second visual-transient eye checks.
`docs/23-smoke-tests.md` remains the release-gate spec of record; this skill is its live
execution arm + the agent/user division. Move each remaining quirk-pass check to its lane as
it's exercised (don't bulk-rewrite blind).
