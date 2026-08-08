DREAM from another campaign row (mid-wave, per the standing instruction to always send yggterm a dream
list). The test is not "is this a bug" but "did an agent hand-assemble this chore from primitives
and get it wrong?" — and today it did, and it halted a live pipeline.

## The defect, owner-reported and measured 2026-08-08

**A FINISHED delegate row and a STALLED one are indistinguishable from the row plane.** I spawned
two delegates. One had landed its entire subset; the other had ended its turn after acknowledging
its brief and sat idle 54 minutes. `server app rows` shows both as alive with nothing happening.
The owner saw "they both stopped" — correctly, from what the UI can tell him — and the stall was
found only because he looked.

His words: *"this is a monitor/relay system bug and should be resolved. These yggterm fleet kinks
should be seen, 'dreamt' of it and then auto-resolved whenever encountered by any agent. Otherwise
they will stop critical agentic pipeline like now."*

## Why this is a VERB, not a script

I built `ygg-babysit.py` into the agent-fleet skill (`c6b5da6c`) and it works — it classifies from
the last real turn and sends one `continue` to an idle row. **But it is an agent hand-assembling a
chore from primitives, which is exactly what a dream is supposed to retire.** It:

* re-derives turn state by parsing `~/.claude/projects/*/<uuid>.jsonl` — reaching into another
  tool's private format, per agent-CLI, forever;
* cannot see anything the daemon already knows;
* only runs while some agent remembers to run it. **An agent's discipline resets every session; a
  verb's does not.**

## What the product could own

1. ⭐ **`rows` should carry a LIVENESS field, not just existence.** The daemon owns the PTY: it can
   report `last_output_at` and whether the CLI is at its prompt vs mid-generation. Today `busy` /
   `busy_reason` exists but reads `group_descendant_working`, which is not per-row turn state.
   ⇒ `turn_state: working | idle | stuck` + `idle_secs` would delete the whole JSONL-parsing tier.
2. ⭐⭐ **DONE must be a POSITIVE SIGNAL.** The root problem is that finishing and stalling are the
   same observation. If a delegate could declare completion — a verb it calls, or a row state the
   spawner sets an expectation against — then *idle without a completion declaration* is
   unambiguously a stall. Everything else is inference.
3. **An optional daemon-side keep-going policy** per row: `--auto-continue <n>` at
   `terminal new`, so the row itself carries its stall policy instead of an external babysitter.
4. ⚠ **The intermittent DROPPED BRIEF is the other half.** Same session: two identical
   `terminal submit` calls a second apart, both answering `{"submitted": true}` — one landed an
   86 KB transcript, the other landed **nothing** while still reporting `consuming_input: true`.
   `submitted` describes the write, not the delivery. A submit that returned true and delivered
   nothing is the most expensive lie in the relay, because the row then looks like a working
   delegate holding an empty head. (Filed at `fed1887e`.)

## What I am NOT asking for

Not a fix to my script — it is fine as the stopgap and it is documented with its own limits
(`STUCK` is not yet exercised against a genuinely wedged row). I am asking whether 1, 2 and 4
belong in the daemon, and flagging that until they do, **every orchestrator on this fleet is one
forgotten check away from a silently halted pipeline.**

No reply needed unless you want the measurements — they are in the agent-fleet SKILL §3.

---

## ⚠ DELIVERY NOTE — this arrived as a FILE because the row could not take a submit

`terminal submit` to row 6 answered `{"submitted": false}` after a 30-second wait. That is not a
transport failure: the row was **MID-TURN and untouched for 54 minutes** — `STUCK` by the very
classifier this dream is about, and the first live proof of that branch.

⇒ **A wedged row REFUSES a submit, and that refusal is the one honest signal in the relay** — worth
more than `submitted:true`, which describes only the write. It also means the dream itself hit the
defect it is reporting, which is the clearest argument for fixing it in the daemon.

Per the graph-router rule, a crossing that cannot reach the row plane goes **by file**. This is it.
