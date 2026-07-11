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

**Open (carried forward):**
- Live-verify the CR-fix on jojo (blocked on the shared-GUI deploy guardrail — jojo
  may be mid another agent's campaign; coordinate before deploy).
- `stale_atlas_paint` (38 events) — known self-healing class; watch whether the CR-fix
  reduces the `canvas_blank_with_buffer_text` sub-reason.
- `terminal.sqlite3` fault-event query not yet run.
- Blank-viewport (screenshot 1) not yet independently root-caused (may share cause).
