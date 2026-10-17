# The fleet task-ledger planner (owner directive 2026-10-17)

`plan.py` turns the fleet task ledger door into the daily execution batches.
The ledger is the SSOT (`ygg-memory get --file fleet-task-ledger.md --ns=_global`);
this tool only reads and sorts it — bookkeeping (claim/done/blocked) happens
by editing the door.

## The routing it encodes (owner laws, 2026-10-17)

- **GLM idle batch: exactly 1 BIG + 2 MODERATE** — the owner's measured
  3-task/day z.ai allowance. The owner fires them from the idle-time UI;
  seats arm only on his word (the quota is his).
- **codex: moderate-to-SMALL only** (owner correction 2026-10-17 — never big).
- Big tasks not taken by the GLM big slot wait or get **chunked** (chunks
  live in the door).
- Everything is **manually fireable by id** on any harness (GLM/Claude/
  Codex/Gemini/…; GLM-legal-Flash from AutoClaw on release).

## Sort (tokenomics)

`score = value / weight` (weight big 4, moderate 2, small 1) over READY
tasks only. `deps:` values starting with `none` (any parenthetical) are
ready; anything else holds the task out (listed with its deps). Route
hints refine placement (a codex-routed task never takes a GLM slot).
Owner-gated and blocked entries are listed, never auto-placed.

## Run

```sh
python3 tools/task-ledger/plan.py            # today's batches
python3 tools/task-ledger/plan.py --json     # machine form
python3 tools/task-ledger/plan.py --selftest # ordering check
```

The ledger path resolves on every host (hub door, memory-synced). Daily
flow for the dreamer/owner: run the plan → fire the GLM batch from the
door's §TODAY briefs → codex works its queue → bookkeep back on the door.
