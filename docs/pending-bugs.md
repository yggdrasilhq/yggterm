# Pending bugs

Open, user-confirmed bugs that are NOT yet fixed. An agent asked to "finish the
pending bugs" should start here. Remove an entry (in the same commit as the
fix) once the fix is verified live on jojo.

## yedit document-surface bugs (user-reported 2026-07-18, REPRODUCED + root-caused; fix next session with the ychrome Phase 5 work)

- **Focus-steal: cannot type in the yedit editor (any of the 3 modes).**
  REPRODUCED live on jojo. A yedit document surface is a viewport-placement
  pane rendered as shell DOM OVER a Terminal-view (shell) session. The
  terminal underneath still believes it owns input, so its focus-reclaim
  cascade (`focusTerminal` / `scheduleInputDriftRecovery`, the
  0/32/96/220/420/760/1200ms `setTimeout` ladder in the terminal mount JS,
  ~line 77970-78060) drives focus into the `.xterm-helper-textarea`. Probe:
  focus the editor and it holds for ~300ms, then is yanked to the helper
  textarea and STAYS (once in the helper, `elementBlocksTerminalAutofocus`
  returns false, so it sticks). Any transient blur of the editor (a
  re-render from draft-sync / heartbeat / schema refetch) opens the window
  the cascade needs. Keystrokes then go to the shell PTY, not the editor.
  **Root cause pinned**: the document surface does NOT signal that it owns
  input. DECISIVE TEST: holding `window.__yggtermUiFocusClaimUntilMs` in the
  future makes editor focus survive indefinitely (the cascade stands down
  via the `Date.now() < globalClaimUntilMs` branch of
  `activeElementBlocksTerminalAutofocus`). **Fix path** (prefer the
  structural gate, matching the existing `web_surface_owns_viewport`
  early-return in `reclaim_active_terminal_input_from_viewport_click`): when
  a document surface is visible for the active session, the terminal mount
  must treat itself as NOT owning input — a `document_surface_owns_viewport`
  gate on `hostOwnsActiveTerminalInput()` plus a `data-*`/window flag the
  reconciler sets so the JS cascade reads it. (A weaker fix — the surface
  setting the focus claim — works but leaves the terminal machinery running
  under an overlay.) Note: on a QUIESCENT surface (no recent re-activation)
  programmatic focus can stick, which is why it looks intermittent; a real
  click + any re-render loses it every time.

- **Document↔Terminal slider degrades to a lone button on the shell side.**
  On the document surface the control is a proper 2-segment slider
  `[📄 Document | ⌨ Terminal]` (inline top-bar OR floating top-right, both in
  `DocumentSurfaceBody`, ~line 85963-86002). When the surface is HIDDEN
  (terminal shown), `DocumentSurfaceBody` is not rendered at all; MainSurface
  instead draws a lone floating `button` "📄 Document" (~line 57012-57033).
  User: "on shell mode, the slider should not change to a button." **Fix**:
  render the SAME segmented control on the terminal side with the Terminal
  segment active and Document the inactive clickable one, so the control is
  consistent (and toggles both ways) in both states. Keep the stale-overlay
  "Show terminal" as a plain button (single action, not a mode switch).

- **THE STALE-DAEMON TRAP — read before diagnosing ANY "the fix didn't work".**
  A deploy that lands new binaries does NOT mean the new code is running. The
  daemon's idle gate defers its own retirement while any owned session is
  actively working — and on a campaign machine an agent session is ~always
  working, so the daemon can stay pinned indefinitely. On jojo 2026-07-11 the
  daemon ran **2.10.3 for 19h44m while 2.10.13 sat on disk**: the CR-faithful
  sanitizer fix and the CC re-birth fix from campaign run 1 were compiled,
  deployed, and never executed. Both bugs were still live for the user, and run 1
  had recorded them as "fixed on branch, live-verify pending" — the gap was
  invisible.
  **Always check `yggterm-headless server status → server_version` against the
  on-disk binary BEFORE concluding anything about a fix.** As of 2.10.14 the
  metadata sidebar's Daemon section surfaces version, uptime, a
  newer-build-on-disk flag, and the daemon's own deferral reason, plus a manual
  hot-restart button — so this is visible in the product rather than only to an
  agent who thinks to look.

- **Live-path frame corruption on busy CC sessions (jojo, 2026-07-10).** While
  an agent streams heavily, the CLIENT xterm buffer accumulates single-cell
  holes (`t ik` for `think`, including the user's own composer echo), merged
  rows, and whole frames interleaved at wrong positions — while the daemon
  vt100 screen stays clean and no `resync_required`/`cursor_rewound` events
  fire. So bytes are lost/mutated between the daemon read and `term.write` in
  the GUI. The ATTACH-seed variant of this class is fixed in 2.10.4 (viewport
  reconcile chunk); the live-path variant is still open. Prime suspects:
  (a) `batch_terminal_chunks` sanitizers rewriting live frames (the
  `observation` rejoin converts `\r\n`→`\n` and strips "noise" lines whenever
  a batch lacks alt-screen/hide-cursor/high-volume markers — content-triggered,
  so yggterm-dev sessions whose transcripts CONTAIN transport-noise phrases are
  hit hardest); (b) `terminal_write_bridge.stage_or_immediate` ordering under
  frame-budget mode. 2.10.4 ships the probes to convict: mine
  `terminal_forward_divergence` + `terminal_write_send_failed` in
  `event-trace.jsonl` and run the client-buffer vs daemon-screen diff recipe in
  `.agents/skills/yggui-app-control/SKILL.md` while a session streams.
  **UPDATE 2026-07-11 (telemetry campaign run 1): suspect (a) CONFIRMED.**
  `terminal_forward_divergence` fired on jojo (4/5 events on `local://`/`live::`
  sessions, drops of 1-11 bytes), and code trace convicted the sanitizers:
  `strip_internal_terminal_transport_noise_lines` did `.replace("\r\n","\n")` over
  the whole batch (content-gated on transport phrases, so it hits local dev
  sessions), and `strip_low_signal_terminal_noise_lines` used `str::lines().join`
  - both drop carriage returns, so xterm paints the next line at the wrong column
  (the staircase/interleave garble). Fixed in 2.10.13: both now `split('\n')`
  (CR-faithful); regression test
  `batch_terminal_chunks_preserves_carriage_returns_in_kept_lines`; the probe now
  emits `cr_dropped`. Suspect (b) not yet investigated.

  **UPDATE 2026-07-11 (run 2): the CR fix was NOT the whole bug — the excision
  itself is.** User re-reported (in different words): "local sessions are dropping
  chars sometimes and replacing the rendering with spaces." Run 1 sized the drops
  at 1-11 bytes and assumed CR loss was the entire mechanism. Re-mining
  `terminal_forward_divergence` found the real magnitude on the user's OWN session:

      local://20e56a8b   raw 9153  → forwarded 8474   = 679 bytes dropped
      local://20e56a8b   raw 23991 → forwarded 23312  = 679 bytes dropped

  679 bytes is a whole-line EXCISION, not a lost `\r`. Mechanism:
  `strip_internal_terminal_transport_noise_lines` content-matches three phrases
  (`terminal session not found`, `ignoring stale yggterm daemon…`, `hot update
  failed…`) and on a hit ALSO sets `drop_following_transport_tail_lines = 3` —
  deleting the matched line **plus the next three lines** of whatever the CLI was
  painting. A Claude Code session whose conversation quotes those phrases (an agent
  working on this very bug does) has four lines removed mid-frame. The daemon vt100
  screen stays clean, so every daemon-side instrument reports the session healthy —
  which is why this survived a run. Making the excision CR-faithful stopped the
  staircase garble but not the deletion.

  **Why it was NOT fixed in 2.10.14:** the excision cannot simply be removed. `ssh`
  writes `Shared connection to <ip> closed.` into the PTY, and yggterm's remote
  helper prints `Error: terminal session not found: <key>` to its stdout, which IS
  the PTY. Both arrive inside cursor-hide control batches, so no content-based or
  branch-based rule separates them from CLI output (5 existing tests lock this).
  The real fix is **per-session attach-phase state** — sanitize only while the
  launch wrapper owns the PTY, be a faithful pipe once the CLI does. That is the
  "collapse the forks / delete the accreted fixes" step of
  `campaign-render-pipeline-parity-rework`, which the user sequenced AFTER the
  parity harness. Deliberately not rushed into a deploy. The measurement, the
  mechanism, and the reason it can't be a one-liner are recorded in code at
  `batch_terminal_chunks`. **This is the next thing to do on that campaign.**

## Deployed live on jojo, faithful-gesture confirmation pending

- **Middle-click a link in a web surface → new tab (2.10.15, c6542edc).** Root
  cause found + fixed: the surface's WebView wired no `new_window_req_handler`, so
  WebKit's `create` signal (middle-click, ctrl/cmd-click, `target="_blank"`,
  `window.open`) returned a null widget and the link was dropped. Now routed into
  yggterm's tab model — background tab for middle/ctrl-click, foreground for
  `window.open`/`_blank`; egress + profile inherited. Unit-tested on the tab-model
  half. Kept GUI-only (no protocol bump) so it deploys against a running
  same-version daemon with no changeover. **Deployed to jojo 2026-07-11** via a
  GUI-only restart (new `~/.local/bin/yggterm` build, SIGTERM+relaunch, the three
  live daemons untouched — verified same PIDs before/after; new GUI pid confirmed
  answering app-control). **Still pending:** a FAITHFUL confirmation, which needs a
  real middle-click — the Xvfb harness is native-surface-blind, app-control clicks
  never reach a child webview, WebKitGTK blocks synthetic `window.open` (no user
  gesture), and jojo's Wayland input injection is unreliable (ydotoold). Ask the
  user to middle-click a link in a ychrome surface; confirm via the
  `web_surface / new_tab_from_link` trace event.

## Fixed in 2.10.2 — confirm live, then delete this section

- **Working-dot lag (10–45 s after the agent finished).** Root cause: the
  focused-terminal defer suspended ALL background snapshot applies, and
  snapshots were the only carrier of `working` flags. Fixed with the
  lightweight `WorkingFlags` daemon poll (2.5 s, defer-exempt) + in-place
  patching. Verify: watch `working_edge` events (source
  `working_flags_poll`) in `~/.yggterm/ui-telemetry.jsonl` while a background
  agent finishes; the dot should clear within ~3 s.
- **Collapsed local machine row never blinked.** Root cause: local agent
  sessions carry the loopback `ssh_target: "localhost"` for restore, which the
  local root's `ssh_target.is_none()` check excluded. Verify: collapse the
  local root while a local CC session works — the row must show the blinking
  working dot (`server app rows` → the `local` group row reports
  `busy: true, busy_reason: group_descendant_working`).

## Diagnostics available

- `~/.yggterm/event-trace*.jsonl` — up to 3 days of trace generations (2.10.2).
- `~/.yggterm/agent-incidents.jsonl` — durable agent resume-error incidents.
- `scripts/render_fail_patterns.py` — groups render fail patterns.
