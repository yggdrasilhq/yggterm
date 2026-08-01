# Automations — the yggui automation layer

> **Spec of record, 2026-08-01.** Supersedes the 2026-05-31 first draft that
> lived on `experimental/automations`. That branch is retired, not deleted:
> `git checkout retired/experimental-automations` recovers its increments 1–2
> (an `Automation` data model and an `automations.json` store). Its reasoning
> survives here; its code does not.

## What it is

An **automation** runs an agent-CLI session on a schedule, unattended, and then
either cleans the session up or tells you it could not.

The user's case, in their words, is the whole spec in one sentence: *a Claude
Code session spawned in a particular path in the middle of midnight for a
particular upgrade-infra job — midnight because then no work gets impacted and
the agent gets ample time to upgrade the infras meticulously — every 2 weeks on
Sunday, without me having to worry about it.*

The second half of that sentence is load-bearing and is where the first draft
stopped: **without cleanup, an automation layer is a session leak with a timer
in front of it.** The user has the receipts — "otherwise we have angry jojo fan
and leaks throughout the system."

## The one thing to understand before reading further

**Almost none of this is new session machinery.** The agent CLI already spawns
sessions the way an automation needs to:

```
server app terminal new --kind claude-code --cwd <dir> --machine-key <host> \
    --no-activate --purpose "automation:<id>" \
    --ephemeral --ephemeral-idle-ttl-secs <n>
server app terminal send <session> --data <prompt>
```

`--no-activate` IS detached. `--purpose` IS the tenancy record. `--ephemeral
--ephemeral-idle-ttl-secs` IS the reaper, already running on an existing daemon
chore tick, already closing through the daemon's one graceful close path, and
already refusing to touch a row the user made by hand (the flag exists only on
the agent-CLI create path, so a hand-made row carries no declaration and never
reaches `ephemeral_reap_reason` at all — see `session_tenancy.rs`).

So an automation is **a schedule, a durable record, and the notification half**.
It calls the verbs above; it does not grow a second way to open a session. That
is the single-source-of-truth rule applied to this feature: if two places could
answer "how does a session get created", they have already diverged.

## Settled decisions (user, 2026-08-01 — do not relitigate)

**D1 — yggterm owns the schedule; the OS fires it.** The automation record in
`~/.yggterm/automations.json` is the SSOT. From it yggterm **generates** a
systemd user timer (Linux), a launchd agent (macOS), or a Task Scheduler entry
(Windows). Those files are **derived artifacts**: regenerated on every change,
reconciled by `automation sync`, and never hand-edited. The OS fires the
trigger; `yggterm-headless automation run <id>` executes it.

Rejected: a daemon-thread scheduler as the only mechanism (fires only while the
daemon is alive, invisible to OS tooling, and needs an autostart install step
anyway). Rejected: hand-authored units with no yggterm record (nothing in the
GUI could then list, edit or disable an automation).

**D2 — close on idle-TTL, notify on deadline.** A run's session is closed
automatically when its PTY has been silent for the automation's `idle_ttl_secs`.
A separate wall-clock `deadline_secs` **never closes anything**; it raises the
persisting notice instead. A noisy session is never touched.

**D3 — catch up within a grace window, else skip.** A run missed because the
machine was off or asleep is executed late only if it can still *start* within
`grace_secs` of when it was due (default 6 h). Past that it is skipped and
rescheduled to the next window. This honours both halves of the user's intent:
the job runs unattended, and it never ambushes them at 2 pm on a Wednesday.

## ⚠ The known risk in D2, stated plainly

`idle_secs` in the existing reaper means **seconds since the PTY last produced
output**. On an agent session that is precisely the measurement this project
knows to be unreliable: an agent spinner IS output, which is why the hot-restart
idle gate never opened once in 40 samples (`campaign-yggterm-unified` ROUND 33,
§THE QUIET-GATE PATTERN). See also
`finding-agent-session-liveness-is-invisible-to-os-signals`.

The safe direction holds — a working, spinning agent is never reaped. **The
unsafe direction is real**: a Claude Code session paused on a question is
silent, and D2 will close it. The user accepted that risk knowingly. Three
things bound it, and all three are requirements, not nice-to-haves:

1. **The default TTL is generous** (30 min), and it is per-automation.
2. **The close is graceful and the transcript survives.** The reaper goes
   through the daemon's ordinary tombstone-then-remove, and the agent CLI's own
   JSONL is untouched, so a wrongly-closed run is *resumable* — `claude -r
   <uuid>` still works. A wrong close costs a resume, not the work.
3. **Every reap is traced and recorded on the run** (`ephemeral_idle_ttl`), so
   "why did my session vanish" has an answer in `automation runs`.

When the parked positive-liveness signal (`lane/dev/agent-liveness`) lands, it
becomes the better input for this decision and D2's TTL rule becomes the
fallback rather than the primary. **Do not build a second liveness inference
here**; that lane is the owner of that question.

## The record

`~/.yggterm/automations.json`, atomic write-temp-then-rename. Kept out of
`server-state.json` deliberately: automations are a distinct concern from
per-session runtime state, and folding them in would churn the 52
`PersistedLiveSession` / 23 `PersistedDaemonState` literal sites for nothing.

| field | meaning |
| --- | --- |
| `id` | stable slug; also the generated unit's filename, so it must be filesystem- and systemd-safe |
| `enabled` | disabled automations keep their record and lose their OS unit |
| `agent_kind` | `SessionKind` — shell / codex / claude-code / future first-class CLIs. Never a binary Codex-or-CC flag |
| `machine_key`, `cwd` | where the session launches; the same addressing `terminal new` takes |
| `prompt` | injected after the session is open |
| `schedule` | calendar expression + `every_n_weeks` (see below) |
| `grace_secs` | D3 catch-up window. Default 21600 (6 h) |
| `attach` | false ⇒ `--no-activate`. **Default false**: at midnight nobody is watching, and a scheduled run that steals focus is a bug |
| `idle_ttl_secs` | D2 auto-close rule. Default 1800 |
| `deadline_secs` | D2 notify-only rule. Default 21600 (6 h) |
| `title` | optional row title; absent means the row is named for the automation |
| `created_at_ms`, `last_run_at_ms`, `next_run_at_ms` | |
| `runs` | bounded history (last 20), each with outcome — see below |

A **run** records `run_id`, `due_at_ms`, `started_at_ms`, `session_path`,
`outcome` (`ran` / `skipped_out_of_grace` / `skipped_off_cadence` /
`reused_live_session` / `spawn_failed`), `closed_at_ms` and `close_reason`
(`ephemeral_idle_ttl` / `ephemeral_owner_gone` / `user` / `never`).

**There is no `automated` flag on the session.** Whether a session is automated
is DERIVED: `automation_for_session(id)` finds the automation whose current run
holds that `session_path`. One owner for the answer; the cwd tree never moves a
node, and Live vs Automated are filtered views over one store. (This was settled
in the first draft and survives unchanged — E1/E2/E3 below.)

## Scheduling

**The calendar expression is systemd's `OnCalendar` syntax**, on every platform.
It is the most expressive of the three, it is what the user already thinks in,
and the macOS/Windows renderers translate *from* it rather than each inventing a
dialect. `Sun *-*-* 00:00:00` is the motivating case.

**`every_n_weeks` is a parity guard, not a calendar term.** `OnCalendar` cannot
express "every second Sunday". So the timer fires every Sunday and the executor
no-ops on the off weeks, deterministically, from the ISO-8601 week number of the
due instant:

```
honoured  ⇔  (iso_week_number(due) % every_n_weeks) == (anchor_week % every_n_weeks)
```

`anchor_week` is stored on the automation at create time. Same input, same
answer, forever — no counter to drift, no state to lose, and an off-week fire
costs one process start. An off-week fire records `skipped_off_cadence`.

**Jitter** (the first draft's "± random days") is retained and stays
deterministic: seeded from `(id, run-window)` via FNV-1a, computed once into
`next_run_at_ms`, never re-rolled per tick. It is not randomness the scheduler
observes; it is a pure function of the record.

## What a run actually does

`yggterm-headless automation run <id>` — the generated unit's `ExecStart`:

1. **Grace guard (D3).** If `now > due + grace_secs`, record
   `skipped_out_of_grace`, reschedule, exit 0. Success, not failure: a skipped
   run is the designed behaviour, and a unit that exits non-zero on it would
   have systemd reporting a permanently failed timer.
2. **Cadence parity guard.** Off-week ⇒ `skipped_off_cadence`, exit 0.
3. **Reuse before spawn (E1).** If the automation's last run holds a session
   that is still live, **re-prompt that session** and record
   `reused_live_session`. Never spawn a duplicate. This is the case where the
   previous fortnight's job is somehow still open.
4. **Spawn**, through the existing verb, with the tenancy declaration that arms
   the reaper:
   `terminal new --kind <agent_kind> --cwd <cwd> --machine-key <machine_key>
   [--no-activate] --purpose "automation:<id>" --ephemeral
   --ephemeral-idle-ttl-secs <idle_ttl_secs>`
5. **Inject** the prompt via `terminal send`.
6. **Record** the run and its deadline; persist.

Cleanup needs no new code: the row carries an ephemeral declaration and the
existing `ephemeral_session_reap_pass` closes it. The automation layer's only
cleanup job is to **notice** the close and stamp the run.

## The notification half (the genuinely new part)

A **notice** is raised when a run reaches its `deadline_secs` with its session
still open, or when a spawn fails. Notices live in the automations store and are
**persisting** in the strict sense: they survive a GUI restart, a daemon
restart, and a reboot, and are cleared only by the user acting on them — never
by a timeout and never by being displayed.

Surfaces (all three, per the check-all-affected-surfaces rule):

- the automation's row in the Automated group,
- the start page's Automations section,
- `automation notices [--json]` for the agent plane.

`automation dismiss <run-id>` is the clear. A desktop notification may echo a
notice, but an echo is not the notice — the durable record is.

## Per-platform renderers

The daemon must count ≥1 enabled automation as live work so it does not
self-retire out from under a scheduled run.

**Linux — systemd user units.** `~/.config/systemd/user/yggterm-automation-<id>.{service,timer}`, plus
`loginctl enable-linger <user>` so timers fire without an active login session.
`Persistent=true` (D3's late fire), with our grace guard deciding whether the
late fire is honoured. The unit carries a `# GENERATED BY yggterm — do not edit`
header and a hash of the record it came from, so `automation sync` can detect a
hand-edit and say so rather than silently overwrite.

**macOS — launchd.** `~/Library/LaunchAgents/dev.yggterm.automation.<id>.plist`,
`StartCalendarInterval` translated from the calendar expression, `RunAtLoad`
false. launchd's own missed-run behaviour is coarser than systemd's, which is
fine: the grace guard is ours and runs identically on every platform.

**Windows — Task Scheduler.** A logon/calendar trigger invoking the same
`automation run <id>`. ⚠ Windows is a **3.0.0** concern and the product does not
build there yet (`docs/pending-bugs.md` §3.0.0). The renderer is specified here
so the trait has three implementations by design rather than two plus a
retrofit; it is not in the first cut.

## Edge cases (settled in the first draft, unchanged)

- **E1 — a live session in an automated slot.** Re-prompt and link it; never
  spawn a duplicate. (Step 3 above.)
- **E2 — un-automate a running session.** The link is removed; the session is
  untouched — same PTY, same cwd-tree node. Only the derived grouping changes.
- **E3 — the cwd tree is untouched in ALL cases.** Live ⇄ Automated is a derived
  view. A node never moves, dissolves, or re-appears.

## The verbs

Both binaries expose them and **neither carries a copy of the parser** — the
same discipline `agent_cli_create_terminal_tenancy` already enforces for
`terminal new`, for the same reason: a flag must mean one thing on either
binary.

```
automation list [--json]
automation show <id> [--json]
automation create --kind <shell|codex|claude-code> --cwd <dir> --machine-key <host>
                  (--prompt <text> | --prompt-stdin)
                  --calendar <OnCalendar-expr> [--every-n-weeks <n>]
                  [--grace <dur>] [--idle-ttl <dur>] [--deadline <dur>]
                  [--attach] [--title <t>] [--id <slug>] [--jitter-days <n>]
automation edit <id> [same flags]
automation enable <id> | disable <id> | delete <id>
automation run <id> [--force]        # the unit's ExecStart; --force ignores both guards
automation runs [<id>] [--json]      # run history + outcomes + close reasons
automation notices [--json] | dismiss <run-id>
automation sync [--json]             # reconcile generated OS units against the store
```

`--force` exists for exactly one reason: the user pressing "Run now" in the GUI,
and an agent testing an automation without waiting a fortnight.

## ⚠ THE GAP: an automation cannot open a session with no GUI running

`CreateTerminal` is an **app-control** command, and app-control routes to a GUI
worker. There is no daemon-side create — `server terminal new` does not exist,
only `server app terminal new`. So an automation firing on a machine with no GUI
records:

```
could not open a session: no live Yggterm GUI client is registered for app control
```

…as `spawn_failed`, with a notice. That is honest, and it is still a gap. On the
live host the GUI is up essentially always, so the motivating midnight job
works; a machine that reboots to a login screen and stays there does not.

**The fix is a daemon-side create**, which is a real piece of work: the daemon
already owns PTYs, so the capability is there, but the create path and its
tenancy stamping currently live on the app-control side. Until it exists, the
first cut's honest scope is "automations fire wherever a GUI is running", not
"wherever the daemon is running" — and the first draft's claim to the latter was
never true.

## Build plan

- **I1 — record, store, guards.** ✅ `automation.rs`: record, run history, the
  grace guard, the whole-weeks parity guard. Pure — every decision function
  takes `now_ms` and `utc_offset_secs`. 35 tests.
- **I2 — the verbs.** ✅ `automation_cli.rs`, one parser, both binaries.
- **I3 — the executor.** ✅ `execute_run` driving
  `create_terminal_with_tenancy` + `submit_terminal_prompt`, with E1 reuse and
  the TTL-only tenancy that arms the existing reaper.
- **I4 — the systemd renderer.** ✅ `automation_units.rs` + `automation sync`
  with the fingerprint and the hand-edit refusal. Live-proven: systemd accepted
  the generated unit and its own next-fire matched our calendar evaluator to
  the second.
- **I5 — notices.** ✅ The durable store, the raise-on-spawn-failure path,
  idempotence-by-run and `dismiss` are live-proven. The daemon half landed too:
  `bookkeeping_pass` rides the same chore tick as the ephemeral reap,
  immediately after it, taking the reaper's outcome as its witness for WHY a row
  went away. It stamps `closed_at_ms` / `close_reason` on finished runs — the
  correctness half, without which a completed run stays `is_open()` forever and
  E1 re-prompts a dead session instead of spawning a fresh one — and raises
  `RunOverdue` at `deadline_secs` without ever closing anything.
- **I6 — the GUI**: the Automated group (filtered by the derived predicate) and
  the start-page Automations section with New Automation. Not started.
- **I7 — launchd renderer.** Not started. Windows deferred to 3.0.0 with the
  platform build.

## Acceptance

The feature is done when, on the live host and without the user touching
anything:

1. `automation create` for the user's real case writes the record AND the
   systemd timer, and `systemctl --user list-timers` shows it.
2. `automation run <id> --force` opens a Claude Code session at the named path
   on the named machine, detached, and the prompt arrives in it.
3. The row appears under Automated, not Live, and its cwd-tree node did not
   move (E3).
4. Left alone past its idle TTL, the session is closed by the existing reaper
   and `automation runs` names `ephemeral_idle_ttl` as the reason.
5. A run held open past its deadline raises a notice that survives a GUI
   restart and a daemon restart, and only `dismiss` clears it.
6. A run fired outside its grace window records `skipped_out_of_grace` and the
   unit exits 0.
7. An off-parity Sunday records `skipped_off_cadence`.

Related: [[spec-active-sessions-dual-presence]], [[spec-unify-local-remote]],
[[spec-cwd-tree-agent-cli-unified]], [[session-keep-alive-spec]],
[[spec-agent-shadow-client-control]],
[[finding-agent-session-liveness-is-invisible-to-os-signals]],
[[spec-terminal-notifications-richness]], [[spec-iteratively-tighten-specs]].
