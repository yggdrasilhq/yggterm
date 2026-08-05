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

## Reading the table today

```bash
yggterm-headless server terminal tenants          # every row, with its verdict
yggterm-headless server terminal tenants <path>   # one row
```

Each row answers `hygiene` — the verdict above — beside the numbers it was
computed from, so the verdict can be checked rather than believed.

## What is built, and what is not

**Built:** the tenant walk, the creator stamp, the pre-declared ephemeral
reaper, and the verdict — the classification every other piece needs.

**Not built, in the order it should be taken:**

1. **The verdict on the row itself.** The user reads the sidebar, not a CLI. An
   agent's row should say so, and an empty plate should say how long it has been
   empty. Until this lands, the policy answers a question the user cannot ask.
2. **A bulk clear.** One verb that closes exactly the rows the verdict names as
   clearable, dry-run by default, so a table can be cleared in one act instead
   of one click per row.
3. **The two-stage sweep** on the daemon's existing chore tick, per rule 3.
4. **The occupied half.** Nothing currently tells the user that a `ychrome` has
   been squatting in a row for three days. It is not clearable — but it is the
   other half of what they are looking at, and it is invisible.
