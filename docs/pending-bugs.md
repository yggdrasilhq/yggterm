# Pending bugs

Open, user-confirmed bugs that are NOT yet fixed. An agent asked to "finish the
pending bugs" should start here. Remove an entry (in the same commit as the
fix) once the fix is verified live on jojo.

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
  (the staircase/interleave garble). Fixed on branch: both now `split('\n')`
  (CR-faithful); regression test
  `batch_terminal_chunks_preserves_carriage_returns_in_kept_lines`; the probe now
  emits `cr_dropped`. **Delete this entry once the fix is live-verified on jojo**
  (gated on the shared-GUI deploy guardrail). Suspect (b) not yet investigated.
  See `docs/telemetry-campaign-log.md` run 1.

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
