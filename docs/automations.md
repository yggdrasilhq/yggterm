# Automations — spec (experimental/automations)

> First draft 2026-05-31. Foundation spec for the Automations feature. Implementation follows.

## What it is

**Automations** is a new top-level section — a sibling to **Live Sessions** — that runs agent-CLI sessions on a schedule, like cron / systemd timers, but driven through **yggui app control** programmatically. An automation opens a session of a chosen kind (Codex or Claude Code) at a cadence (e.g. monthly ± random days), sends it a prompt, and lets the session finish on its own. The resulting keep-alive session persists in the **Automated Sessions** list so the user can watch it work if they're around, and inspect the result afterward.

**Motivating example (user):** a Codex session opened ~monthly (± random days) with *"some time has passed, can you upgrade again"* that upgrades the whole infrastructure (jojo, main, practice, …) looking into update nuances and package-registry flaws. Today the user does this by hand; the automation does it on a timer and leaves the finished session as a keep-alive in Automated Sessions.

## Core model — SSOT, no second store

The **session object is the single source of truth** (same rule as [[spec-active-sessions-dual-presence]] / [[spec-unify-local-remote]]). "Live Sessions" and "Automated Sessions" are **pins / shortcuts at the top of the cwd tree**, NOT separate session stores. A session is never "dissolved from the cwd tree and re-created in a list" — it stays in the cwd tree; the Live/Automated groups are filtered *views* (presence flags) over the same sessions. (The user tightened the Live spec precisely to kill the "session moves between stores" thinking error; the same discipline applies to Automated.)

Two distinct concepts:

1. **Automation** — a schedule definition (the cron/timer entry). Fields:
   - `id`
   - `agent_kind`: `Codex | ClaudeCode`
   - `target`: machine + cwd (where the session launches)
   - `prompt`: the text sent after the session is open
   - `schedule`: cadence (`Weekly | Monthly | …`) + timing (`specific` time/day, or `random` hours/days within the cadence) + jitter (± random days)
   - `enabled`
   - `last_run_at`, `next_run_at`
   - `linked_session_id`: the keep-alive session it most recently spawned (if any)

2. **Session** — the actual agent-CLI session an automation spawns. It is a normal keep-alive session, plus an `automated: bool` (or `automation_id: Option<…>`) flag that places it in the **Automated Sessions** group instead of **Live Sessions**.

## Lifecycle & state transitions

When a timer fires:
1. The daemon scheduler opens/creates the session via the app-control path (`app open` / create) — the same surface agents use.
2. Sends `prompt` via `app terminal send`.
3. The session runs to completion on its own and **persists as keep-alive** in **Automated Sessions** (`automated = true`).

**Automated vs Live placement is decided by the `automated` flag, not by which code opened it.**

### Edge cases (user-specified)

- **E1 — manual session in an automated slot.** If the user manually closes the keep-alive in the Automated list, OR manually spawns a session that *is* configured as automated, that session spawns in **Live Sessions** normally (`automated = false`). When the automation's timer next fires, the scheduler **transfers** that Live session into **Automated Sessions** (sets `automated = true` / links it) rather than spawning a duplicate.
- **E2 — un-automate a running session.** If the user un-automates a running keep-alive in the Automated list, it **transfers** to **Live Sessions** (`automated = false`). The session itself is untouched (same PTY, same cwd-tree node) — only the pin/flag changes.
- **E3 — cwd tree is untouched in ALL cases.** Live ⇄ Automated transfers are flag flips on the session; the cwd-tree node never moves, dissolves, or re-appears.

## UX (from the whiteboard)

Minimal modification to the existing **start page**. The Automated Sessions start page reuses the start-page list, but composes each entry as a **sentence** rather than a plain session row, with a **New Automation** button (boxed, top, like a section header action):

```
Automations
[ New Automation ]

1. Launch [Codex ▾] on [ <timer spec> ]
   ────────────────────────────────────
2. Launch [ CC ] on [ Weekly ] at [ Random hours ]
   ────────────────────────────────────
   ⋮
```

- The kind selector is a dropdown/button — **Codex or CC**; **CC is rendered as an orange button** (per the sketch).
- The timer spec is a cadence (`Weekly`, `Monthly`, …) + timing (`specific` or `Random hours` / random days).
- Each automation is one composed list entry; the list lives under the Automations header like Live Sessions does.

## Mechanism — yggui app control IS the automation runtime

Automations are scheduled invocations of the **yggui app control** surface: open a session, send a prompt, leave it keep-alive. This is the programmatic, cron-like use of app control the user envisioned (and the same surface agents use to drive/verify the app). The scheduler lives in the daemon (`yggterm-headless`) so automations fire even when the GUI is closed, consistent with the keep-alive / daemon-owns-PTYs model.

## Open design questions (resolve during implementation)

- ~~Persistence: server-state.json vs separate file~~ → **RESOLVED: separate `~/.yggterm/automations.json`** (atomic write-temp-rename). Chosen for clean separation + to avoid churn across 52 `PersistedLiveSession` / 23 `PersistedDaemonState` literal sites.
- ~~Deterministic jitter~~ → **RESOLVED**: `compute_next_run_at_ms` seeds jitter from `(id, run-window)`; computed once into `next_run_at_ms`, never re-rolled per tick.
- ~~`agent_kind` general?~~ → **RESOLVED**: it IS `SessionKind` (future first-class CLIs, not a Codex/CC binary).
- ~~Automated-flag on session~~ → **RESOLVED: DERIVED** — `automation_for_session(id)` (an automation's `linked_session_id == session.id`). No duplicated flag on the session; the link on the automation is SSOT. E1/E2 = link add/remove.
- Still open (scheduler increment): daemon-down catch-up = run-on-next-start (planned); concurrency when a prior automated session still runs = transfer/attach, never duplicate (E1).

## Implementation status (experimental/automations)

- ✅ **Increment 1 — scheduling foundation** (`b072d63`): `automation.rs` — `Automation`, `AutomationCadence`, deterministic `compute_next_run_at_ms` + `automation_is_due`. 6 tests.
- ✅ **Increment 2 — persistence + server CRUD + derived grouping** (`577f8bd`): `automations.json` load/save; `YggtermServer.automations` + CRUD; `automation_for_session` / `session_is_automated`. 7 tests.
- ⬜ **Increment 3 — daemon scheduler chore**: load automations at startup; periodic chore fires due automations → opens/re-prompts the linked keep-alive session via the app-control session-open + `terminal send` path; updates `last_run_at`/`next_run_at` + saves. E1 transfer-not-duplicate.
- ⬜ **Increment 4 — UI**: Automated Sessions sidebar group (filtered by `session_is_automated`) + start-page "Automations" entries ("Launch [Codex/CC] on [cadence] at [time]") + New Automation create flow + E2 un-automate action.
- ⬜ **Increment 5 — live verify** on jojo.

## Build / verify

Implementation goes on `experimental/automations` (worktree `~/gh/yggterm--automations`). Verify live via yggui app control on jojo (now faithful on Wayland post-2.8.0): create an automation with a short test cadence, confirm it fires, opens the session, sends the prompt, and the finished session lands in Automated Sessions (not Live), with the cwd-tree node unchanged.

Related: [[spec-active-sessions-dual-presence]], [[spec-unify-local-remote]], [[spec-cwd-tree-agent-cli-unified]], [[session-keep-alive-spec]], [[spec-iteratively-tighten-specs]].
