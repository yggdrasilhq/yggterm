# The hot-restart gate — relay-aware, deadlined, and repairing

**This file is the ONE owner of "when may a daemon be swapped, and what is owed
to the sessions it was carrying".** It replaces the absence-gate that
`CLAUDE.md`'s ⚖ CONSTITUTION records as the project's unmet guarantee.

⚖ **Owner-settled 2026-08-08.** It supersedes the standing ⛔ in the campaign
memory against putting a deadline on the idle gate. That prohibition was
correct for the design it was written about and is not correct for this one;
§7 says exactly why.

## 1. The problem, and the live evidence

`CLAUDE.md` §THE QUIET-GATE LAW: **yggterm must never gate corrective work on
absence of output, because an agent CLI is never output-silent.** The hot-restart
gate is an AND over every owned session at 300 s on a clock that output bumps.
It was measured open in **0 of 40 samples**. A gate that only converges when
nothing is active cannot converge on a machine that is always active.

Measured on the GUI host while writing this:

    GUI binary        3.0.67
    live daemon       3.0.65      (older daemons alive at 3.0.62, 3.0.59, 3.0.29)
    hot_restart_pending      true
    hot_restart_blockers     []          ← empty
    hot_restart_block_reason null        ← nothing is blocking it
    last successful swap     241 minutes ago

⭐ **The gate reports that nothing is blocking it and still does not fire.** That
is worse than a gate held by a named blocker: there is no blocker to clear, so
there is nothing a human or an agent can do to help it along. Four hours of
version skew accumulated behind a condition with an empty blocker list.

## 2. The insight: a relay boundary IS the quiet window

The old gate hunted for a moment when **every** session is simultaneously quiet.
On a machine running a fleet of agents that moment does not arrive, and waiting
for it is waiting for an event with probability approaching zero.

But the campaign already produces per-session quiet moments constantly, and
**announces** them: **a relay hand-off.** A predecessor has finished and its
successor has not started. That is a genuine, declared, zero-cost quiet point,
and there are several per day.

⇒ **Drive the swap from relay boundaries, not from polling for silence.** The
gate stops being a search and becomes an appointment.

⇒ **And a relay must be daemon-aware in the other direction too:** a successor
born onto a stale daemon inherits the skew and compounds it. **A relay forces or
awaits the swap BEFORE handing off**, so every successor starts on current.

## 3. The session state machine — classification, never silence

The gate asks each owned session what it IS, not whether it has been quiet. Four
states, and only one of them blocks:

| state | meaning | blocks a swap? |
|---|---|---|
| **IDLE** | no turn in flight | no |
| **BLOCKED-ON-HUMAN** | stopped at a question, a permission prompt, or any dialog awaiting the owner | **no — owner-ruled** |
| **WORKING** | a turn is genuinely in flight | yes, up to the deadline (§5) |
| **ORCHESTRATING** | a turn in flight that is itself running sub-agents | yes, **without deadline** (§6) |

**BLOCKED-ON-HUMAN is not working, and this is the owner's explicit call:**
*"Care should be taken when a session stalls at questions. They should be hot
restarted and considered not working."* A session waiting on a human may wait
forever; treating that as activity is how a gate written against silence
inverts into a gate that never opens. ⚠ The old gate scored these as busy,
because a question prompt is *output*, and output bumps the clock.

## 4. Queue, do not poll

A swap request is **queued**, not attempted-and-abandoned. The queue is the
mechanism that keeps the promise *"the mechanism should not stale the daemons"*:
a request that cannot run now runs at the next boundary, and it is never lost.
One request is in flight at a time; a newer build supersedes a queued older one
rather than adding a second entry.

## 5. The 30-minute deadline, and the repair that makes it safe

Owner-ruled: **after 30 minutes of waiting, force the swap, stalling the working
sessions** — and then **inject `continue` into every session that was
interrupted.**

The two halves are one ruling and must ship together. A deadline alone is what
the campaign memory forbids, and rightly: it interrupts a live agent turn and
walks away. A deadline **plus** a repair is a different mechanism, because the
cost of being wrong drops from *"an agent's work is destroyed"* to *"an agent
loses a few seconds and resumes"*.

⇒ **`continue` is owed to exactly the sessions the swap interrupted**, and to no
others. A session that was IDLE or BLOCKED-ON-HUMAN must not be nudged: nudging
a session parked by design trains its reader to ignore the signal, which is the
same guard the fleet skill's stall-recovery section already states.

⚠ **Once per forced swap, never per tick.** The `continue` is a repair for a
known interruption, not a liveness poll.

## 6. The exemption: a session running sub-agents is waited for

Owner-ruled: *"sessions running multiple agents inside should be waited for
completion before hot restart (the 30 min rule does not apply here)."*

The reason the exemption is principled rather than a carve-out: **an
ORCHESTRATING session's work is not its own.** Interrupting it strands every
delegate it launched — processes that outlive the interruption, hold rows, and
have no idea their orchestrator is gone. `continue` repairs a session; it cannot
re-adopt an orphaned fleet. So the blast radius is unbounded in a way a single
turn's is not, and no deadline can price it.

⇒ ORCHESTRATING blocks indefinitely. If that stalls a swap for hours, that is
the correct outcome, and §4's queue means nothing is lost.

## 7. Why this supersedes the standing ⛔ on deadlines

The campaign memory says: *"⛔ Do not bolt a deadline onto that one — it protects
in-flight agent turns. The liveness lane is PARKED after failing its review."*

That was right about the design it reviewed, which proposed a bare timeout on
the existing absence-gate. This design differs in the two ways that caused the
review to fail:

1. **It has a positive definition of "working"** (§3) rather than an inference
   from silence — which is what the QUIET-GATE LAW asks for in its own words:
   *"prefer a positive signal ('safe now') over an absence."*
2. **It repairs what it interrupts** (§5), and it refuses to interrupt the one
   class where repair is impossible (§6).

⇒ The deadline is no longer the whole mechanism; it is the backstop on a
mechanism that mostly does not need it, because §2 means the common case is a
swap at an announced boundary with nothing in flight at all.

## 8. What must be true before this ships

- **Sub-agent detection must be positive, not inferred.** ORCHESTRATING is the
  state with an unbounded wait, so a session that merely *looks* busy must not
  reach it. Read it from the agent's own declared state, never from process
  ancestry alone.
- **The interrupted set must be recorded across the swap.** `continue` is owed to
  a list that is computed before the old daemon dies and consumed after the new
  one is up; it cannot be re-derived afterwards, because after the swap every
  interrupted session looks idle.
- **A forced swap is still subject to the constitution.** Older daemons keep
  their sessions (`CLAUDE.md` ⚖), rows keep identity, order and count, and the
  owner never learns that two daemons exist.
- **The gate must report a reason.** `hot_restart_blockers: []` beside
  `hot_restart_pending: true` is the state this document exists to abolish: if a
  swap is waiting, something must be nameable as the thing it waits for, and if
  nothing is, it must fire.
