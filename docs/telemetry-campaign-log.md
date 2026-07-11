# Telemetry campaign log

One dated entry per run of the **infinite telemetry campaign** (triggered by
"continue telemetry campaign"). Source of truth for "what did the last pass find,
fix, and leave open" so the next pass does not re-derive it. The campaign itself
is defined in agent memory (`campaign-telemetry-infinite`); this is its run ledger.

Each entry records: artifacts mined (host + files), patterns seen, root causes,
fixes (with commit), **probes added/retired/re-thresholded**, and what is still
open. Diagnostic JSONL is per-host and never committed — pull it live over ssh.

Format per run:

```
## <date> — run N (host: jojo @ <version>)
Mined: <files>
Findings: <pattern → root cause → fix/commit or OPEN>
Probes: <added / retired / retuned>
Open: <carried forward>
```

---

## 2026-07-11 — run 1 (host: jojo @ 2.10.11 binary, daemon fleet 2.10.x)

**Mined:** jojo `~/.yggterm/event-trace.g*.jsonl` + `.previous` + live (≈90 MiB, days
of data) via `scripts/render_fail_patterns.py`; `perf-incidents.jsonl`;
`terminal.sqlite3` present (71 MiB, not yet queried this run). Plus two user
screenshots (`clipboard-1783744570985.png`, `clipboard-1783744579675.png`).

**User hypothesis tested:** "local (jojo) sessions have more rendering bugs than ssh
sessions; the render paths diverge in code and local got less love."

**Findings:**

1. **render_fail_pattern census (days):** 38 `stale_atlas_paint`, 2 `redraw_burst`,
   1 `app_render_storm`. Low volume, mostly the known stale-atlas class (self-heals).
   Screenshot 1 (blank viewport) fits `canvas_blank_with_buffer_text` (seen as a
   render-health reason inside the stale-atlas events).

2. **★ CONFIRMED root cause — live-path frame corruption (was suspect (a) in
   `pending-bugs.md`).** Screenshot 2 (two frames interleaved cell-by-cell + stale
   green SGR blocks) is **byte-drop in the GUI batch sanitizers**, invisible to every
   daemon-side instrument (daemon vt100 stays clean; no resync/cursor_rewound).
   - `terminal_forward_divergence` fired 5× — **4 of 5 on `local://`/`live::`
     sessions, 1 remote** (drops of 1–11 bytes, escape/CR-sized). The two `drop=11`
     hits are the same local yggterm-dev session (`9d8b35ea`).
   - Mechanism: `batch_terminal_chunks` → `strip_internal_terminal_transport_noise_lines`
     did `.replace("\r\n","\n")` over the **whole batch** whenever the text contained
     a transport phrase (`terminal session not found`, `ignoring stale yggterm daemon`),
     and `strip_low_signal_terminal_noise_lines` used `str::lines().join("\n")` on the
     `observation` forward path. Both **drop carriage returns**, leaving the cursor
     mid-row so the next line paints at the wrong column → the staircase/interleave.
   - **Why local-biased (user's hypothesis, refined):** the render path does NOT fork
     on local-vs-remote. The corrupting sanitizer is **content-gated on phrases that
     appear disproportionately in local sessions** — local sessions actually emit
     `local://` transport errors, and yggterm-dev sessions (which run locally) print
     those strings in their own transcripts. So local sessions trip the sanitizer far
     more. The bias is real; the cause is content-correlation, not path divergence.

**Fix (branch, NOT yet live-verified on jojo):** `batch_terminal_chunks` sanitizers
now split on `'\n'` (CR-faithful) instead of `str::lines()` / `.replace`; only whole
matched lines are excised, all other bytes (incl. `\r` and real trailing `\n`) survive.
Regression test `batch_terminal_chunks_preserves_carriage_returns_in_kept_lines`.

**Probes:**
- **Added/upgraded** `terminal_forward_divergence`: now emits `raw_cr` / `forwarded_cr`
  / `cr_dropped`, isolating the *corruption* signal (CR loss) from benign whole-line
  excision (which legitimately shrinks `raw_bytes` post-fix). Next run: alert on
  `cr_dropped=true`, not mere byte divergence.
- **Noted limitation (not yet fixed):** the probe is throttled to 1 event / 2000 ms,
  so 5 events over days undercounts true frequency. Consider a monotonic per-session
  divergence counter next run if `cr_dropped` still shows up.

**Findings, part 2 — the local-vs-ssh divergence audit (user-directed):**

3. **★ The user's "local renders worse than ssh" hypothesis is CONFIRMED, structurally.**
   `is_remote_resume_session` is threaded as a **bool through 171 sites** in
   `crates/yggterm-shell/` (~517 local/remote branches in `shell.rs`), not one decision
   point. **`remote` is the NAMED concept; `local` is the unnamed `else` fallback** — every
   branch was reasoned about for remote, and local silently inherits whatever the fallback
   happened to be. This violates two specs we already wrote and never applied:
   `spec-unify-local-remote` ("drive from SessionKind") and `spec-agent-cli-wrapper-render-parity`.
   Opened campaign: **render-pipeline parity rework** (agent memory
   `campaign-render-pipeline-parity-rework`), sequencing chosen by the user:
   **parity harness FIRST** → collapse forks → delete accreted fixes → pre-planned probes.

4. **★ CC relaunch reap bug (user-hit, now evidenced).** Mined `agent-incidents.jsonl` — a
   probe that existed and had **never been mined**. 21 incidents: 11 `session_already_in_use`,
   8 `session_not_found`. The user's incident, session `1965f8d5-…`:
   - `11:10:58` `error: session id 1965f8d5-… is already in use` — the OLD CC process still
     held the lock; yggterm did not reap it before relaunching.
   - `11:11:26` `no conversation found with session id: 1965f8d5-…` — the session had **no
     transcript** (quit before turn 1), so `claude -r` is *correctly* refusing. **The startpage
     must relaunch a turn-0 session FRESH (`--session-id`), not with `-r`.** Not yet fixed.

5. **Probe bug — the incident probe was counting its own conversation.** The scanned PTY
   stream contains the agent's *rendered conversation*, so `agent_session_error_in_line`
   fired on prose that merely MENTIONED a refusal (the user's typed message, the agent's
   reply about the bug). 3 of the 21 "incidents" were this self-inflicted noise. Same disease
   as the sanitizers: content-matching on transcript text. **Fixed `7d26936`**: gate on SHAPE
   (a real refusal is terse, ≤16 words; a sentence about one is 28–30).

**Probes, part 2:**
- **Added — THE FAITHFUL-PIPE INVARIANT** (`batch_terminal_chunks_is_a_faithful_pipe_*`): a
  corpus of real TUI frames (CRLF+SGR, alt-screen, DEC 2026 sync, bare-CR spinner, cursor
  repaint, wide unicode, bare-LF, prose-mentioning-an-error) that must pass through
  `batch_terminal_chunks` **byte-for-byte**, plus the same across every chunk-split offset.
  This is **tier 1 of the parity harness** and the permanent gate against re-accretion: any
  future "fix" that mutates live bytes now fails here instead of on the user's screen. It has
  teeth — the pre-fix pipeline failed it (it was withholding trailing newlines).
- **Retuned** `agent_session_error_in_line` (terseness gate, above).

**Shipped in 2.10.13 (this run):**
- CR-faithful sanitizers (`38844e9`) — the interleaved-frame garble.
- Agent-incident probe shape gate (`7d26936`) — stopped it counting its own conversation.
- Faithful-pipe invariant, harness tier 1 (`ca9d6c3`).
- Transcript-less local CC row now re-births with `--session-id` instead of resuming
  nothing (`e6ffc31`) — the `no conversation found` half of the user's relaunch incident.
- `scripts/parity_harness.py` — harness tier 2 (manual PTY vs yggterm). **Control side
  works; wrapper side deliberately FAILS LOUD**: the headless CLI exposes no raw-stream
  read, so capturing the wrapper's byte stream would mean reimplementing the read path
  (i.e. measuring the harness, not the product). Refusing to fake it is the point.

**Open (carried forward) — honestly not done:**
- **Harness tier 2 needs a product affordance**: `yggterm-headless server terminal
  raw-stream --session <path> --for <secs>`, emitting the exact bytes the GUI forwards to
  `term.write`. That single command turns the wrapper-vs-manual rule into a CI gate. NEXT STEP.
- **CC "already in use" (the reap half)** — still open. The `no conversation found` half is
  fixed; the lock-held-by-a-live-process half is not. Suspected collision between keep-alive
  (yggterm keeps the CC process running after the user closes the view) and relaunch (which
  spawns a SECOND `claude` on the same id). Correct fix is likely "attach to the live runtime
  instead of spawning a second CLI", which touches the launch path — not rushed into a deploy.
- **Broken bottom on every switch** — my SIGWINCH-nudge hypothesis is DEAD. The nudge already
  exists (`remote_prompt_gap_resize_nudge_allowed`) and is **hard-disabled**, returning `false`
  with every parameter unused: a previous attempt resized the PTY and made live Codex TUIs
  redraw into broken prompt regions. Also, Linux only delivers SIGWINCH when the winsize
  actually CHANGES, so a same-size nudge is a no-op — the idea cannot work as stated. The
  function is accreted dead code and a deletion candidate. Real fix must come from the parity
  harness (what does the manual case do on switch that we don't?), not another guess.
- **The 171-fork collapse** — the core of the parity campaign; a large refactor deliberately
  NOT rushed ahead of this deploy.
- `stale_atlas_paint` (38 events) — watch whether the CR fix reduces the
  `canvas_blank_with_buffer_text` sub-reason.
- `terminal.sqlite3` fault-event query not yet run; blank-viewport (screenshot 1) not
  independently root-caused.
- Live-verify the CR-fix on jojo (blocked on the shared-GUI deploy guardrail — jojo
  may be mid another agent's campaign; coordinate before deploy).
- `stale_atlas_paint` (38 events) — known self-healing class; watch whether the CR-fix
  reduces the `canvas_blank_with_buffer_text` sub-reason.
- `terminal.sqlite3` fault-event query not yet run.
- Blank-viewport (screenshot 1) not yet independently root-caused (may share cause).
