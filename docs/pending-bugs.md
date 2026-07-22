# Pending bugs

Open, user-confirmed bugs that are NOT yet fixed. An agent asked to "finish the
pending bugs" should start here. Remove an entry (in the same commit as the
fix) once the fix is verified live on jojo.

## Standing traps / other open bugs

- **Blank viewport from a DETACHED `term.element` (jojo, 2026-07-22).** The
  viewport paints nothing — background only — while the session is alive, the
  daemon screen is correct, and **every health field reports healthy**. Cause:
  `term.element` is out of the DOM (`isConnected:false`, rect 0×0) while an
  empty husk — `div.terminal.xterm` holding only `.xterm-viewport`, no
  `.xterm-screen`/rows/canvas — occupies the host. It never self-heals because
  all three `rebindCurrentHost` reopen guards read false against that husk (it
  matches `.xterm`; the renderable-layer check requires the absent
  `.xterm-screen`), and `ensureVisibleHost` short-circuits on `emitPaint()`,
  whose `visible` is satisfied by any child.
  **Probes shipped 2026-07-22 (`terminal_host_element_detached`, host-attachment
  fields in `app state`, mutation breadcrumbs).** **FIX LANDED in code 2026-07-22
  (`rebindCurrentHost` now treats `termElementOutsideHost` — `term.element` not in
  the live host — as a fourth reopen trigger, so the reopen re-appends
  term.element and drops the husk; guarded by
  `terminal_eval_script_probes_detached_term_element`). LIVE PROOF still owed —
  activates on the next jojo deploy; watch the `rebind_host` debug event for
  `term_outside_host=true reopened=true reattached=true`.** Full write-up, the
  trace signature that dates past occurrences, and the three open questions (which
  wipe leaves the husk; whether the never-released reveal ghost is involved; why
  ~7% of mounts):
  [`docs/xterm-bugs.md#detached-term-element-blank-viewport`](xterm-bugs.md#detached-term-element-blank-viewport).
  Recovery with no restart: re-append `term.element` and drop the husk via
  `server app dom-eval`.

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

  **UPDATE 2026-07-20 (run 5): now USER-BLOCKING, and it reproduces hardest on
  the busiest remote-CC session.** The user reported a session that "100% never
  renders", where closing and reopening the GUI — their standing workaround —
  had stopped working. Named session: `remote-cc://dev/029a3955…`
  ("libyggterm Rebase"). Evidence gathered this run:

  - **The corruption is in the client BUFFER, not the paint.** `app terminal
    read-buffer --mode screen` shows three different screen states interleaved
    character-by-character on the same rows (an old report, a test-code frame, a
    `/context` usage panel, plus a stray line-number column). The faithful
    screenshot merely renders that corrupt buffer honestly, so this is NOT a
    canvas/renderer problem — do not chase the renderer again.
  - **It survives every repair that does not fix the pipe.** Two real SIGWINCHes
    (PTY winsize verified changing 63×167 → 62×166 → 63×167 on dev, so CC
    definitely re-authored its frame) left the buffer byte-identical in the
    corrupt regions; GUI restarts and repeated `app open` reveals do not stick.
    The attach/replay seed is clean (fixed in 2.10.4), so a fresh reveal paints
    correctly and then **re-corrupts within seconds** of live streaming.
  - **Why THIS session and not the neighbouring one.** CC on dev is writing
    ~1.2 MB/s (`/proc/<pid>/io` write_bytes +6 MB in 5 s). High throughput means
    more batches, and the excision is content-triggered — and this session's
    transcript is saturated with the exact transport phrases the sanitizer
    matches ("dropped", "eval failed", "never armed", and it literally quotes
    `terminal session not found`). The calm local session in the same window
    showed no such corruption. That is the "hit hardest" prediction above,
    confirmed on a session the user cannot use.

  **CORRECTION, same run — the sanitizers are NOT the cause of THIS symptom.**
  It was tempting to file the above under suspect (a) because it matches the
  narrative, but the probe refuses it: `terminal_forward_divergence` fired
  **3 times in the whole trace, all on an unrelated `live::5d0e22ed…` plain
  shell, and ZERO times on `remote-cc://dev/029a3955`**. The GUI forwards the
  daemon's bytes faithfully for the corrupted session. Two further facts clear
  the excision specifically: the per-line predicate requires a SCHEME-QUALIFIED
  match (`local://`, `remote-session://`, `codex-runtime://` — note
  `cc-runtime://` is absent), so prose quoting the phrase is already guarded by
  `batch_terminal_chunks_keeps_prose_about_missing_sessions`. An attach-phase
  gate for `batch_terminal_chunks` was written and then **reverted unshipped**
  because it fixed a bug this session does not have. Suspect (a) remains real
  for the sessions where divergence DOES fire; it is simply not this.

  **The actual mechanism, read off the raw stream.** The agent CLI paints by
  skipping unchanged cells with cursor-forward, not by overwriting them — the
  daemon-side bytes for this session are literally
  `❯ On\x1b[C the\x1b[C meta\x1b[C page` and `t\x1b[8C html`, i.e. every space
  and every run of spaces is a CUF. **Cells that CUF skips keep whatever was
  already in them.** So once the client buffer's base state diverges from the
  frame the CLI believes is on screen, every skipped region shows stale content
  and the CLI never rewrites it — permanent, character-by-character
  interleaving, exactly what is on screen. It re-corrupts within seconds of a
  clean reveal because the very next diff frame paints against the wrong base.

  **Next step (unverified hypothesis, do not ship on it):** find where the
  post-attach live stream resumes relative to where the attach replay stopped.
  A seam — overlap or gap — between the replayed snapshot and the live stream
  would leave the client buffer holding a base the CLI never authored, which is
  all it takes. A gap is consistent with a high-throughput session being hit
  hardest (~1.2 MB/s here). Note that two real SIGWINCHes did NOT repair it,
  which needs explaining: a resize normally forces a full repaint, so either CC
  did not receive it or its own full repaint is also CUF-based against a stale
  model. Settle that first — it discriminates between "client base is wrong"
  and "CLI model is wrong".

- **Remote CC session stays permanently blank: `resume-cc` deadlocks before it
  launches the CLI (dev, 2026-07-20).** User-reported as "it never renders", and
  it is NOT a render bug — the xterm buffer is genuinely empty (0 non-whitespace
  chars), so the blank viewport is honest. On the remote host the wrapper
  `yggterm server remote resume-cc <uuid> <cwd> --require-existing` sits in
  `unix_stream_read_generic` (blocked on a daemon unix socket) for many minutes
  with **no children** — it never spawns `claude` at all, so the PTY produces
  nothing forever. `Status` in the metadata rail reads `bootstrapping · idle`.

  **Neither workaround clears it.** Re-clicking the row just logs
  `terminal_bootstrap_existing_lease_skip` ("bootstrap skipped because an
  existing attach lease ...") — three attempts in a row did that here, none
  reaching `ready`. A full GUI restart does NOT fix it either (verified: fresh
  GUI, re-open, still 0 chars), which rules out GUI-side in-memory lease state
  as the blocker and matches the user's "even the workarounds do not work".

  **Recovery that DOES work:** kill the stuck wrapper on the remote host
  (`pgrep -af "resume-cc <uuid>"`, it has no children and holds no user work);
  the next open spawns a fresh wrapper which does launch `claude --resume`, and
  the session comes back with full scrollback. Confirmed end-to-end on
  `remote-cc://dev/75874380…`.

  **Prime suspect: the dev daemon fleet.** dev is still running **six**
  `yggterm-headless server daemon` processes (the consolidation item carried
  from telemetry run 3, [[finding-adopt-gap-untypeable-fixed-2113]]). A helper
  that connects to a stale/wrong daemon socket and blocks forever on read is
  exactly this signature. Fix direction: (1) consolidate dev's daemons, (2) give
  `resume-cc` a connect/read deadline so it can never block indefinitely before
  spawning the CLI, and (3) make `terminal_bootstrap_existing_lease_skip`
  reclaim a lease whose attach never reached ready, instead of deferring to it
  forever.

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

- **Clipboard-staging dir grows forever — no autoclean anywhere (user-confirmed
  2026-07-16).** `stage_local_clipboard_png` (shell.rs) and
  `stage_remote_clipboard_png` (yggterm-server lib.rs, incl. the python ssh
  fallback) write every image paste to `~/.yggterm/clipboard/` and nothing ever
  prunes it: pi is at 204 files / 182 MB since April, files up to 15 MB each.
  Left alone this fills the drive.
  **The careful part — these files are NOT free to delete.** The staged PATH is
  what gets pasted into agent CLIs (codex / Claude Code image attach), so agent
  session transcripts reference these paths; resuming a 15-day-old session and
  re-reading its image must not hit file-not-found. Design constraints for the
  fix (deterministic, explicit thresholds, per the no-non-determinism rule):
  1. Two-stage TTL, sweep in each host daemon's chore tick, oldest-first by
     filename (names embed epoch millis, so name order = age order):
     age > 45 days → move to `~/.yggterm/clipboard/.trash/`; trash age >
     45 more days → delete. A "something not found" event then has a 90-day
     recovery window and the trash hop is itself reversible.
  2. Before trashing, reference-check the (unique) filename against that
     host's agent transcript stores (`~/.codex/sessions`, `~/.claude/projects`
     JSONLs); referenced files get their clock reset, never silently dropped.
  3. Size backstop: if the live dir exceeds 1 GB, evict oldest-to-trash down
     to the cap (same reference check applies).
  4. Local and remote hosts each sweep their OWN dir (the daemon is per-host);
     no cross-host deletion.

## Diagnostics available

- `~/.yggterm/event-trace*.jsonl` — up to 3 days of trace generations (2.10.2).
- `~/.yggterm/agent-incidents.jsonl` — durable agent resume-error incidents.
- `scripts/render_fail_patterns.py` — groups render fail patterns.
