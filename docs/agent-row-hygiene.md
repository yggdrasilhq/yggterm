# Agent row hygiene — whose plate is this, and is it empty?

**Owner of the question "what may clear a live session row, and when".** The
mechanism lives in `crates/yggterm-server/src/session_tenancy.rs`; this file is
the policy it enforces. What is currently OPEN belongs to `docs/pending-bugs.md`
and what shipped belongs to git — this file says only how it is supposed to
behave.

## The report, in the user's words

> *"All agents leave their dinner plates on the 'live session' table which I
> have to manually annoyingly clean up after a while, because I do not know if
> the plates are empty or some blinking session is on the chair with the plate."*
> — 2026-08-05

Two complaints, and **the second one is the blocker**. Clearing a row costs one
click. Deciding whether it is safe to clear costs real attention, on every row,
every time — and because getting it wrong means killing something that was still
working, the rational move is to clear nothing and let the table fill up. That
is what has been happening.

So the deliverable is not a reaper. It is an **answer**, per row, that a person
or a sweep can act on without thinking: *whose plate is this, and is it empty?*

## What the table actually looked like

Measured on the GUI host, 2026-08-05, `server terminal tenants`: 27 rows, 11 of
them measurable locally.

| what | count | evidence |
|---|---|---|
| empty plates — an agent's row holding nothing but its own login shell | 7 | `tenant_count: 0`, `foreground_command: /bin/bash -i`, `tree_cpu_seconds: 0.01` |
| occupied — something still squatting | 4 | all four `ychrome`; oldest tenants **2.1 days** and **3.0 days** |
| unmeasurable from here — the work is on another machine | 16 | `no_local_runtime` |

Two of the seven empty plates carried a purpose (`wcwidth parity probe`,
`records T5 UGC payment leg`) and named themselves instantly. Five carried nothing.
**A row with no stated purpose is indistinguishable from a row that matters**,
which is the whole problem restated.

## The verdict — one owner, safest first

`RowHygieneVerdict` (`session_tenancy.rs`) classifies every row, and **nothing
may re-derive it**. Two callers that each decide "is this safe to clear" from
the raw fields will eventually disagree, and the wrong one closes work.

| verdict | meaning | clearable |
|---|---|---|
| `user_row` | no creator stamp: a human or the GUI opened it | **never**, at any age |
| `unmeasurable` | this host cannot see the row's work (carries the named reason) | **never** |
| `occupied` | an agent's row with something alive in it | **never** — but worth SHOWING |
| `empty_plate` | an agent's row holding only the shell that IS the row | only this |

Ordering is load-bearing in two places:

- **`unmeasurable` is decided BEFORE the creator stamp.** A remote row carries no
  local stamp either, so a creator-first reading calls it the user's row — right
  by accident, and wrong the moment anyone acts on the reason rather than the
  verdict.
- **`empty_plate` with an unknown idle age is not clearable.** An unknown age
  must never read as "idle forever". This is the same rule the tenant report
  already keeps by never printing a zero it did not measure.

## The four rules

**1. Only an agent's own row is ever swept.** Provenance comes from the creator
stamp, which every agent-CLI `terminal new` writes. No stamp, no sweep — a row
the user made is not this policy's business, however idle it looks.

**2. A row is judged only by the daemon that can see its work.** A `remote-*`
row's agent lives on the other machine; locally there is only an ssh bridge, so
a local reading of "nothing is running" is meaningless. Sweeping on it would
take the row the user is *currently talking to an agent through* first. Same
per-host rule the clipboard sweep keeps: no cross-host deletion, ever.

**3. Clearing is two-stage and the first stage is reversible** — the shape of
the clipboard trash policy (`clipboard_sweep.rs`), for the same reason: a
deletion that cannot be undone has to be *certain*, and certainty is not
available here. Stage one takes the row off the table without killing anything;
stage two, much later, closes it. A row that stops being empty at any point
leaves the process entirely.

**4. Absence of proof keeps the row.** If the measurement cannot complete — the
owning daemon did not answer, `/proc` was unreadable, the idle clock is unknown —
nothing is swept that round. Every failure mode resolves to "leave it alone".

## What an agent must do with the rows it creates

Provenance is not optional and neither is cleanup. Agents create rows through
`server app terminal new`, and:

- **Always pass `--purpose <what for>`.** A pid that died seconds after the
  create is an audit trail, not an answer. The purpose is what lets the user
  read their own table — and, in the measurement above, it is exactly what
  separated the two rows that explained themselves from the five that did not.
- **Pass `--ephemeral` with a rule** when the row is a probe rather than work
  the user will want to see: `--ephemeral-owner-pid <your own pid>` or
  `--ephemeral-idle-ttl-secs <n>`. A bare `--ephemeral` is refused on purpose —
  see `EPHEMERAL_NEEDS_AN_EXPLICIT_RULE` for the measurement behind that.
- **Remove your own row when you are done with it**
  (`server app session remove <path>`), rather than leaving it for the policy.
  The sweep is a backstop for what an agent forgot, not a substitute for
  clearing your own plate.
- ⛔⛔ **AN APP SESSION IS RUBBISH THE MOMENT ITS JOB IS DONE — owner-directed
  2026-08-07**, after an India Post booking run left two ychrome rows behind on
  a 38-row sidebar. His words: *"janitor work on the left our ychrome session
  should be done … so these used app sessions do not dangle around and cause row
  noise on the yggterm GUI UX."*
  A ychrome/yRDP row is **scaffolding for ONE task, not a shared asset**: its
  surface holds a finished page, its scrollback holds a spent URL, and it
  explains nothing to the human whose sidebar it sits in. **Removing the row is
  the last step of the task, not a courtesy afterwards.**
  ⚠ **The ONE exception is a HANDOVER, and it must be named in the report with
  its path**: a session deliberately left alive for the human to finish — a UPI
  QR he must scan, a login only he can complete. A leftover is not a handover.
  ⚠ **And "the row went" is not "the processes went".** Prove all three:
  `server app rows` (your path absent) · `server app state | jq
  .web_surface_tabs` (no row for your session) · `pgrep -af 'ychrome --profile
  <yours>'` (empty). Read `verified`, never `accepted`.
  ⛔ **Why this had to be said twice:** the rule above has been in this file
  since 2026-08-06, but `~/.claude/skills/data-fabric/SKILL.md` — the door most
  agents actually load — said the OPPOSITE ("LEAVE THE SESSION UP … visibility
  beats tidiness") until 2026-08-07. **The stale line won, because an agent
  obeys the doc it reads, not the doc that is right.** ⇒ when a rule here is
  disobeyed, check whether some other doc is teaching the reverse before
  concluding the agent was careless.

## The outline contract — every session names and numbers itself (owner-directed 2026-08-06)

The user runs ~19 sessions he reads, and the sidebar is a working instrument, not a log. He
organises it **like a book**, and the numbering also declares *which rows are under
orchestration* — an unnumbered row is his alone. **Every session does this for itself.** The
orchestrator is not the janitor: it runs on the expensive tier, the sessions run on the cheap
one, and janitorial work belongs where it is cheapest and best-informed.

**1. Name your OWN row — but compose with your CLI, do not fight it.**
Claude Code **renames its own session** once it has a title, which silently destroys any name
set at creation. Codex does not, but most agent CLIs behave like CC, so this is the rule:

> **Wait for your CLI to auto-title, then RENAME to `N. <lobe>: <the CLI's own title>`.**

The CLI's title is genuinely informative — it says what the session is doing — so prefixing it
keeps both facts. Re-assert the prefix after any restart, and after any point where you notice
your row has lost it.

```bash
yggterm server app session rename '<your row path>' '2. cogs: <CC title>'
yggterm-headless server app terminal keep '<your row path>'   # sessions are keep-alive…
```

**2. Number and name every row you SPAWN, as your child.** A row you create is `N.M`, where `N`
is your own number: `4. levers` spawning a ychrome surface for an ITR portal makes
`4.1 ychrome: itr-<label>`. Place it directly beneath yourself. An unnumbered spawn is an
orphan the user cannot attribute, and attributing it by hand is exactly the work this rule
exists to delete.

**3. Keep-alive SESSIONS, never their spawns.** The numbered agent sessions are keep-alive so he
can identify them at a glance; a transient surface (a ychrome page, a probe) is not.
⚠ `terminal new --help` claims agent CLI kinds are *born* keep-alive — measured false on 3.0.39,
so set it explicitly until that is fixed.

**4. When your task ends, REMOVE the rows you spawned — yourself.** Read `verified`, not
`accepted`: a `verified:false` names a refusal and lists surviving pids in `live_processes`,
**which you must then reap yourself**. ⚠ `session remove` has been measured TIMING OUT on
removals that fully succeeded, so re-read the table before retrying — and never conclude
"gone" from the row list alone, which is how a live agent was once orphaned with no row.

**⇒ The goal is that tooling does 99% of this.** Every rule above that a human or an agent has
to *remember* is a defect in the product: the parent chain should supply the number, a session's
children should be sweepable as a unit, and an explicit title should outrank the CLI's derived
one. Until then, the contract is manual and each session owns its own plate.

## Reading the table today

```bash
yggterm-headless server terminal tenants          # every row, with its verdict
yggterm-headless server terminal tenants <path>   # one row
```

Each row answers `hygiene` — the verdict above — beside the numbers it was
computed from, so the verdict can be checked rather than believed.

## What is built, and what is not

**Built:** the tenant walk, the creator stamp, the pre-declared ephemeral
reaper, the verdict — and, as of 2026-08-06, **the sanity system that acts on
it**: `crates/yggterm-server/src/row_sanity.rs` plus `server terminal sanity`.

```bash
yggterm-headless server terminal sanity           # the table, and what would happen
yggterm-headless server terminal sanity --apply   # actually record / close
```

The four rules above are BRANCHES in that module, each with a test, rather than
prose a caller has to remember. Thresholds: an empty plate must be idle
**30 min** before stage one looks at it (an agent composing its next command has
an idle PTY), and must still be empty **1 h** later before stage two closes it.
Records live in `$YGGTERM_HOME/row-sweep-records.json`.

⭐ **The idle clock works now, and that is what un-blocked all of this.** The
field shipped on 2026-08-05 read `null` on every row because ownership had not
migrated to a daemon carrying it, which left the policy inert. Measured on the
live table 2026-08-06: 39 rows, 19 measurable, idle values 0 / 870 / 5532 / 6542
— real numbers, so the verdict finally discriminates.

**Live-proven lifecycle** (jojo, 39 rows): dry run named two rows it would
record; `--apply` recorded them and the table was **39 rows before and 39
after**, because stage one kills nothing; the next run read them as *waiting out
the grace* instead of re-recording. The occupied half is sorted by tenancy age
and led with a ychrome that had been squatting **3.8 days**.

**Not built, in the order it should be taken:**

1. **The verdict on the ROW itself.** Still the biggest gap: the user reads the
   sidebar, and today the answer lives in a CLI. `RowHygieneVerdict` is confined
   to `yggterm-server` with nothing carrying it to the GUI, so this needs a wire
   field before it needs any pixels.
2. **The sweep on the daemon's chore tick.** `sanity --apply` is the manual
   form; nothing runs it on a schedule yet, so the table only stays sane if
   somebody asks. The decision function is already pure and the record store is
   already durable, so this is wiring rather than design.
3. **Acting on the occupied half.** It is now VISIBLE and sorted by age, which
   was the missing half — but a three-day ychrome tenant still needs a person to
   decide, and nothing offers them the choice in one place.
