# The CLI integration layer — yggterm 3.3.0

**Status of this file:** THE owner of "how yggterm knows what a CLI session
is, and what it owes that session across daemon swaps." Set in stone
2026-09-10 (owner-directed architecture; consultant verdicts from
`gemini-3.8-flash HIGH` via agy folded in — see
`msgGraph/lores/chain-of-thought/2026-09-10-yobserve-tiered-observability.md`).
It supersedes the ad-hoc descriptor reality in `agent_cli.rs` (which becomes
schema v2's implementation) and pairs with
[`spec-hot-restart-relay-gate.md`](spec-hot-restart-relay-gate.md) §3.1/§5.1
(the working law this layer feeds).

## 0. The problem, in the owner's words

"yggterm cannot reliably understand what each CLI is doing. The CLIs are a
blackbox. So dynamic updating of titles, the webview of the sessions remain
broken because UUID drifts or webview wiring fails. CLIs like opencode which
use a superior server/client architecture and can rapidly switch working
sessions cannot be decoded by yggterm's metadata layer."

The felt symptom class: after a daemon update, **codex shows a named failure
and reattaches** (the twin-writer banner, ~12 s), while **agy, muse and
opencode silently open a NEW session** — the owner must `/resume` or
`/sessions` by hand. Silent lossy fallback is the failure this layer abolishes.

## 1. The four capability classes

Every managed CLI belongs to exactly one class. The class decides what the
metadata layer may ask for and what reattach owes.

| class | shape | roster | survival | metadata truth |
|---|---|---|---|---|
| **A** | store-authoritative, turn-based | codex, claude | PTY handoff preferred; resume-by-id otherwise | the CLI's own store + identity at birth |
| **B** | server/client switchable viewer | opencode, zcode-tui | PTY handoff (near-always survives) | **the SERVER's active-session answer**, never the TUI screen |
| **C** | TUI with a local store | agy, muse, kimi, qwen, grok, pi, codex-litellm | PTY handoff; resume via store-id discovery | store + measured screen phrases |
| **D** | no-store TUI | (none today — class exists so the registry can be total) | PTY handoff only | screen only, and the row says so |

Roster notes: zcode-tui is FIRST-PARTY — it must become the reference
implementation of the native announce (§3), the cheapest integration in the
fleet. The `gemini` CLI is not a managed session kind (it runs inside shell
rows); "there is no gemini CLI" refers to consultants — agy is the Gemini
route there too.

## 2. Descriptor schema v2 — five orthogonal capabilities

`AgentCliDescriptor` grew into 35+ ad-hoc fields. Schema v2 reorganizes it
into five capabilities; a CLI is INTEGRATED when all five are declared AND
the probe battery (§5.4) passes for its class.

1. **Invocation & Identity Contract.** `birth_argv(session)` — how yggterm
   launches it; `resume_argv(session_id)` — how it re-opens a store session;
   `id_assigned_at_birth: bool` — whether the row id IS the CLI's session id
   from birth (codex/claude) or the CLI mints its own (codex-picker rule,
   [11.74]).
2. **Tenancy & Rebind Discovery.** An explicit resolution-strategy enum, in
   precedence order: `NativeAnnounce` (first-party OSC/env — zcode-tui) →
   `EnvMarker(var)` → `Argv(pattern)` → `FdLock(glob)` → `StoreIndex(fn)` →
   `ServerIpc(endpoint)` (class B). Rebind = which strategy answers when the
   row's id drifts.
3. **Discrete Phase State Machine.** The row's phase is an enum, not a
   boolean: `Working | Idle | QuestionPrompt | LimitWait | StartupGate`.
   Sourced per class: native IPC (B, first-party), bounded screen/footer
   phrase arrays (A, C — the measured `ScreenWorkingPhrase` data), or "no
   opinion" (D). The hot-restart gate and the sidebar dot consume THIS enum
   (spec-hot-restart §3.1) and must never disagree.
4. **Durable Store & Copy Reader.** `store_roots`, `read_title`,
   `read_summary`, scheme prefixes — the title-authority machinery
   (`TitleAuthority::Store`) and recency readers hang off this.
5. **Tenancy Migration Protocol.** `supports_pty_fd_handoff: bool` and
   `content_rederives_on_resume: bool` — whether a daemon swap adopts the
   PTY via SCM_RIGHTS or cold-exits and re-resumes, and what that costs the
   user (scrollback loss vs none).

⛔ **Schema v2 is versioned.** Adding a capability = a descriptor-schema
version bump + the probe battery re-run per CLI, in the same commit.

## 3. The identity contract

Precedence, highest wins: **native announce → env marker → argv → store →
screen.** A row's session id may only move upward in this chain (a store
answer outranks a birth uuid — the [11.75]/title-authority law), and every
move is journaled with the answering strategy. `id_assigned_at_birth` CLIs
keep by-id resume; self-minting CLIs without a saved session get their own
picker arm — never a composed `resume <row-id>` the CLI never heard of.

## 4. Reattach: the handoff-witnessed ownership ledger

The 12 s codex wait is `wait_for_external_agent_resume_to_clear`
(lib.rs ~2585-2620) sleeping 3 000 ms per iteration while
`external_agent_resume_processes_for_session` does blind `/proc` forensic
scans — because yggterm lacks a positive "who owns this session NOW" answer.
**But the daemon orchestrated the swap; it knows.** Replacement:

- At handoff, the predecessor writes a **ledger record per owned row**:
  `adopted { by_pid, pty_fd: moved }` or `died_with_me { store_session_id,
  resume_argv }`. Crash-safe file, one writer, and **cleared by whoever
  satisfies it** (the hot-restart-queue law — a record that outlives its own
  satisfaction lies about ownership).
- The successor answers reattach from the ledger: adopted PTYs attach with
  **zero discovery**; `died_with_me` rows get an immediate
  `resume <store_session_id>` spawn — no store scan, no /proc polling.
- **Reattach SLA per class:** A/C adopted = instant; A/C re-resume = bounded
  (target ≤2 s from ledger answer to CLI spawn); B = instant (server holds
  the session). ⛔ **When the ledger cannot answer, the row shows the NAMED
  failure** (the codex-banner pattern, uniformly for every CLI) — a silent
  fresh spawn is a defect, not a fallback.

Implementation status (11.6.0, first land): `ownership_ledger.rs` in
yggterm-server holds the record types, the staleness law (an adoption dies
with its adopter's pid; a death sentence expires in minutes; consume-on-
satisfy) and the crash-safe writer. The handoff sweep writes
`adopted { by_pid }` per moved row from the ack itself, and all three resume
wrappers consult the ledger before the /proc wait. `died_with_me` is typed,
served and consumed but **no current exit path writes one**: a daemon exits
only after an AllMoved sweep (nothing left held), and a partial sweep's held
rows stay unrecorded because their writer keeps serving them — the first
writers of death sentences will be the force-retire / cold-exit paths and
the class-C resume arms. NoAnswer falls back to today's scan-and-banner.

## 5. Observability — tiered, mostly file-shaped

The owner's yobserve instinct (wrap the launch, see what the CLI does, tune
per CLI with probe groups, falsify panics/paint-fails) is the law — with the
constraint that observation is **tiered and mostly file-shaped**, never an
always-on kernel shim:

- **T0 — launch contract (always on, free).** The wrapper entrypoint
  `yobserve <cli...>` stamps identity (T0 markers), sets
  `prctl(PR_SET_PTRACER, parent)` so later T2 attach is legal under YAMA,
  and declares the observation budget for this CLI.
- **T1 — targeted store-watching (cheap, allow-listed).** Watchers on the
  descriptor's declared store paths only. ⛔ SQLite-backed stores fire on raw
  WAL flushes and punish concurrent readers with SQLITE_BUSY: watchers are
  debounced, read via short-lived read-only connections with busy_timeout,
  and **local-only** — remote rows stay on-demand reads over ssh, never
  watchers.
- **T2 — the probe battery (on demand, isolated).** Per-CLI drive suite:
  launch → turn → working-phases → title → resume → in-TUI switch (B) →
  panic falsifier. The probe **launches its own child** (YAMA: a successor
  daemon cannot ptrace a sibling), captures the CLI's real screen/store
  behaviour, and emits the measured data that fills schema v2 (phrase
  tables, recency readers, resume tokens). A naive pty drive is not a probe
  battery — use the xterm-harness.
- **T3 — kernel tracing (escalation only).** eBPF (Linux) / ETW (Windows)
  for a Class C/D CLI that proves opaque to T0-T2. Almost never on; never a
  release dependency.
- **Panics/paint-fails are ours already:** systemd-coredump wiring per row
  for CLI panics; the daemon holds the PTY bytes and ytrace for paint/amber
  falsification. No shim needed to see those.

Observation budget law: a CLI's watchers are **allow-listed in its
descriptor** — what may be watched, where. "Observe as little as possible"
is enforced by the allow-list, not by hope.

## 6. Staleness degrades loudly

Store schemas and CLIs drift. The probe battery re-runs on CLI updates (the
ynpm channel knows versions); a descriptor that fails its battery is marked
`stale` and the row **degrades loudly** — the named-failure banner — never a
silent fresh spawn.

## 7. Per-CLI issue scheme — the 11.6.x family

The cli-integration family owns the bugbath. Family id **11.6**; members are
`11.6.<n>`, pinned in this order (zcode-tui last, owner-ruled):

| id | CLI | class |
|---|---|---|
| 11.6.0 | the family itself (cross-CLI: schema, ledger, battery) | — |
| 11.6.1 | codex | A |
| 11.6.2 | claude | A |
| 11.6.3 | opencode | B |
| 11.6.4 | agy (Antigravity) | C |
| 11.6.5 | muse | C |
| 11.6.6 | kimi | C |
| 11.6.7 | qwen | C |
| 11.6.8 | grok | C |
| 11.6.9 | pi | C |
| 11.6.10 | codex-litellm | A |
| 11.6.11 | zcode-tui | B (first-party) |

Multi-session law (owner 2026-09-10): he runs SEVERAL sessions per CLI in
parallel; each claims one `11.6.<n>`, works a lane `lane/integration/<cli>`,
and keeps per-CLI state in its own door
(`campaign-cli-integration/<cli>.md`) — the architecture here is the stone
that makes parallel seats collision-free.

## 8. The 3.3.0 release plan

In 3.3.0: descriptor schema v2 (§2), the identity contract (§3), the
handoff-witnessed ledger + SLA (§4), per-row integration status on the wire
(protocol stamp bump), zcode-tui native announce (reference impl), yobserve
T0-T2 as the probe-battery runner, the 11.6.x scaffold. NOT in 3.3.0: T3
kernel tracing (follow-up, only for a proven-opaque CLI).

## 9. What must be true before this ships

- Schema v2 conversion is table-driven with a per-CLI test (every registry
  entry answers all five capabilities or names what is missing).
- Ledger crash tests: predecessor dies mid-write; successor satisfies and
  clears; a stale record cannot outlive its satisfier.
- The probe battery runs zcode-tui end-to-end in CI as the reference.
- The reattach SLA is measured per class on a forced same-version rotation —
  adopted-PTY rows attach with zero /proc polling (the `external_agent_resume_*`
  scans must be unreachable from the ledger-served path).
