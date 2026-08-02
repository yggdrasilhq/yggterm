# Triage queue — recovered threads, none verified against today's build

**Not the bug queue.** Every item here was root-caused in a past session and
never closed, and every one is UNVERIFIED against the current build — the
newest is weeks old and much has shipped. Verify one before working it; when it
is confirmed, move it to [`pending-bugs.md`](pending-bugs.md) with a status, and
when it is dead, delete it.


## Residual threads recovered from archived memory

Residual threads recovered from archived memory (2026-07-31)

These were root-caused in past sessions, never closed, and were sitting in
memory files that no session indexed. They are transplanted here so the SSOT is
actually the SSOT. **⚠ Every one is UNVERIFIED against today's build** — the
newest is 3 weeks old and much has shipped since. Re-check before working one;
some are certainly fixed already. Full narrative for each is in
`~/.claude/memory-archive/-home-user-gh-yggterm/<slug>.md`.

- ⭐ **Broken-bottom during a working turn — NO LONGER A LEAD. USER-CONFIRMED LIVE
ON 2.12.19 (2026-07-31) AND ROOT-CAUSED TO ONE THRESHOLD.** User's words: "the
bottom region of CC opens up always TUI frame broken and I fix with slash usage
and scroll." That workaround is the tell — typing `/` makes Claude Code repaint
its own footer, i.e. **the CLI fixes it because yggterm's corrector never runs.**

The corrector is the reveal screen-reconcile in `shell.rs:~84866`, and its own
comment already says it "IS the broken-bottom fix". It is gated on
`screen_reconcile_output_quiet` = **1,200 ms with no forwarded output**
(`SCREEN_RECONCILE_OUTPUT_QUIET_MS`, shell.rs:550). On a miss it rearms
**+3,000 ms** (`SCREEN_RECONCILE_DEFER_REARM_MS`, shell.rs:555) — **with no
maximum defer count and no deadline.** A working agent CLI drives a spinner
continuously, so the quiet window never arrives.

There IS a bypass, and its threshold is the bug:
```rust
let reveal_incomplete = screen_reconcile_reason == "reveal_screen_reconcile"
    && last_host_health_visible_nonblank_rows < 3;
```
It only rescues a viewport that has degraded to **fewer than 3 non-blank rows** —
the catastrophic blank-frame case. **A partially broken bottom, which is what the
user actually sees, has 40+ non-blank rows, so it takes no escape at all** and
waits for a silence that never comes.

**Measured in `~/.yggterm/event-trace.jsonl`, 83 minutes, 2026-07-31:**
- **198 true deferrals** (71 traced + 127 hidden by the 10 s trace rate-limiter —
  read `suppressed_since_last`, the raw line count understates it 2.8×) against
  only **32 completed reconciles**: deferred 6:1.
- Chains are long, not isolated: session `…828dc5021f0d` deferred ~36 consecutive
  times over 106 s (10:18:19→10:20:05); `…1369e395ee57` ~40 times over 110 s
  (10:29:38→10:31:28). Both escaped only via the blank bypass.
- `reveal_forced_incomplete` fired **9 times** — nine occasions where the viewport
  had to rot to near-blank before anything corrected it.

✅ **FIXED IN CODE 2026-07-31 — `SCREEN_RECONCILE_DEFER_DEADLINE_MS = 12_000`.**
The 1,200 ms quiet test stays as the *preferred* path so brief bursts still avoid
a tear; once the correction has been owed for 12 s the reconcile is forced
regardless of output. Chain age is measured from the FIRST defer of a chain, not
the last rearm — the rearm cadence is 3 s, so "time since last defer" could never
exceed it no matter how long the user stared at a broken frame. Locks:
`a_deferred_reconcile_converges_even_while_output_never_stops` and
`the_defer_clock_measures_the_chain_not_the_last_rearm`, both red-proven
(`>=`→`>` on the boundary, and removing the no-chain guard, each fail).
⚠ **NOT YET LIVE-VERIFIED on jojo** — the running GUI is 2.12.19, which predates
this. Confirm after the next deploy by watching for
`terminal_mount/screen_reconcile_forced_deadline` in the trace.

Related and also open: `sidebugs-webgl-artifacts-stale-frame-on-switch` calls
stale-frames-after-switch "the biggest remaining UX bug".

- ⚠ **THE PATTERN BEHIND THREE SEPARATE BUGS: we gate corrective work on
output-silence, and an agent CLI is never output-silent.** Same false assumption
in three places, all live:
1. the hot-restart idle gate — 300 s idle threshold on a clock that OUTPUT bumps
   (`daemon.rs:501`, `session_idle_for_ms`); measured 0-of-40 samples open;
2. the screen reconcile above — 1,200 ms quiet, unbounded rearm;
3. the 2.12.19 drag-stall fix's own root cause, quoted from a81c7366: selection
   events fire "per streamed write that shifts the buffer under a live selection
   — an agent CLI that streams constantly multiplies the events."

Terminals for humans are mostly quiet; this product's primary workload never is.
**Any new gate must have a deadline, or hang off a positive liveness signal
rather than an absence-of-output signal** (see
`finding-agent-session-liveness-is-invisible-to-os-signals`).


## screen_snapshot_clipped_to_pty_width fires constantly and nobody has looked

**`screen_snapshot_clipped_to_pty_width` fires constantly and nobody has looked**
— 108 times in 17 minutes on `local://43c47548…`, every one reporting
`pty_cols: 171` against `screen_max_column: 260`. The daemon's vt100 screen is
holding content 89 columns wider than the PTY it belongs to and discarding the
overhang on every snapshot (`terminal.rs:2113`). Unexplained; may be benign
post-resize residue, may be a second content-loss path.


## LIVE-path frame corruption (finding-client-buffer-garble-attach-seed-and-live-path)

**LIVE-path frame corruption** (`finding-client-buffer-garble-attach-seed-and-live-path`)
— the attach-seed half was fixed in 2.10.4; the live-path half was left open
with probes already shipped to convict it.


## New-codex-session UUIDv4 identity drift (finding-new-codex-session-bug-class)

**New-codex-session UUIDv4 identity drift** (`finding-new-codex-session-bug-class`)
— the rebind sets `session.id` to codex's ULID but never rekeys the map, so key
and identity split. Named as the single cause of three symptoms. Not fixed.
`finding-uuidv4-codex-session-drift` (still in memory/, cited by code) holds the
Stage-2 remote-codex rebind that was never done.


## Daemon-side launch_phase stuck at RemoteBootstrap (finding-stale-phase-15s-remount-blink)

**Daemon-side `launch_phase` stuck at RemoteBootstrap** (`finding-stale-phase-15s-remount-blink`)
— the GUI half shipped 2026-07-07; the daemon half was left open. This is
plausibly the same wedge as ROUND 30's §THE WEDGE — check before treating them
as two bugs.


## app_render_storm cause (finding-render-storm-autopsy-armed-run4)

**`app_render_storm` cause** (`finding-render-storm-autopsy-armed-run4`) — fired
21× in 10 days, all unattributed; a self-arming autopsy was shipped to catch it
and the autopsies were never read.


## codex composer split background (finding-codex-composer-bg-split-reflow)

**codex composer split background** (`finding-codex-composer-bg-split-reflow`) —
xterm.js reflow on column resize drops cells' bg attribute. Root-caused, fix
pending, flagged trap-zone.


## OSC 52 double copy-chime + replay refire (finding-osc52-copy-chime-replay-refire)

**OSC 52 double copy-chime + replay refire** (`finding-osc52-copy-chime-replay-refire`)
— no dedupe and no replay-suppression, so every reattach re-parses the embedded
OSC. Root-caused code-grounded, never live-verified.


## ibus cumulative input fix never landed (finding-ibus-cumulative-input)

**ibus cumulative input fix never landed** (`finding-ibus-cumulative-input`) —
`GTK_IM_MODULE=gtk-im-context-simple`; fix was built in the 2.9.41 tree and
never committed. An end user hit this.


## Shipped-but-never-live-confirmed: finding-cc-blink-partial-2026-frame-flush

**Shipped-but-never-live-confirmed:** `finding-cc-blink-partial-2026-frame-flush`
(2.9.38), `finding-codex-select-scroll-kick` (2.9.32),
`finding-remote-cc-mislabeled-codex-gone-message` (2.9.50, deploy-pending).


## Owed proofs: full passkey crypto E2E against a real RP

**Owed proofs:** full passkey crypto E2E against a real RP
(`finding-passkey-browser-slice-shipped`) — gated on a vault unlock; jojo GUI
'+'-menu render proof (`finding-launcher-registry-one-app-registry`).


## Rows lost across a daemon swap (project-resume-after-2100-daemon-swap)

**Rows lost across a daemon swap** (`project-resume-after-2100-daemon-swap`) — 3
live rows lost, 2 of them keep-alive, with a rescue file. Same family as
`finding-daemon-handoff-drops-live-rows` (still in memory/, code-cited).


## ychrome queued slices (campaign-zoom-system-rework)

**ychrome queued slices** (`campaign-zoom-system-rework`) — **per-site zoom and
the settings pane BOTH SHIPPED** (verified on ychrome main 2026-07-31:
`src/webzoom.rs` is 238 lines of per-site overrides behind a `/zoom` endpoint
with a change-hash so the GUI refetches only when an override moved;
`src/sidebar.rs` serves "Tabs", "Browser identity" and "Userscripts"
sections). What is left of this slice is **session buddy**; **vertical tabs**
is NOT a separate item, it is the rail-as-cwdtree entry in the
ychrome-as-main-browser list above, and should be tracked there only.


## Non-code todos (project-blackboard-clearing-2026-07-16)

**Non-code todos** (`project-blackboard-clearing-2026-07-16`) —
awesome_steer_prompts repo, app-infra forecast.
