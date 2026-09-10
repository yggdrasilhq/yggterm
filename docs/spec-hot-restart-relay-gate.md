# The hot-restart gate — relay-aware, deadlined, and repairing

**This file is the ONE owner of "when may a daemon be swapped, and what is owed
to the sessions it was carrying".** It replaces the absence-gate that
`CLAUDE.md`'s ⚖ CONSTITUTION records as the project's unmet guarantee.

⚖ **Settled 2026-08-08.** It supersedes the standing ⛔ in the campaign
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

The declaration is `server relay-boundary`, and it is a field on the §4 queue
entry rather than a file of its own: a boundary does not change WHAT the host
owes, only when it may next try. ⛔ **A boundary buys exactly one attempt.** A
declaration that stayed live would release the retry floor on every 20 s poll,
which is the fork bomb that floor exists to prevent — rebuilt by the mechanism
meant to bypass it safely.

## 3. The session state machine — classification, never silence

The gate asks each owned session what it IS, not whether it has been quiet. Four
states, and only one of them blocks:

| state | meaning | blocks a swap? |
|---|---|---|
| **IDLE** | no turn in flight | no |
| **BLOCKED-ON-HUMAN** | stopped at a question, a permission prompt, or any dialog awaiting the owner | **no — owner-ruled** |
| **WORKING** | a turn is genuinely in flight | yes — **and past the deadline too, since the 2026-09-10 ruling (§5)** |
| **ORCHESTRATING** | a turn in flight that is itself running sub-agents | yes, **without deadline** (§6) |

**BLOCKED-ON-HUMAN is not working, and this is the owner's explicit call:**
*"Care should be taken when a session stalls at questions. They should be hot
restarted and considered not working."* A session waiting on a human may wait
forever; treating that as activity is how a gate written against silence
inverts into a gate that never opens. ⚠ The old gate scored these as busy,
because a question prompt is *output*, and output bumps the clock.

### §3.1 What "working" means, per CLI (2026-09-10)

⚖ Owner: *"only non-working daemons are auto updated and reattached to the
GUI."* The whole model therefore leans on ONE predicate, and the ruling on what
it may read is strict:

- **An agent session is working only by ITS OWN CLI's phrases** — the
  descriptor's `working_screen_phrases` matched against that session's screen,
  never the union across every registered CLI. The union is the shape that made
  the indicator "buggy for all CLIs except codex, claude, and plain shells":
  one CLI's completion trace, or prose on an unrelated row's screen, armed
  another CLI's work signal. The sidebar dot and this gate consume the same
  per-CLI matcher and must never disagree.
- **A plain shell is working iff a foreground job of its own is running** —
  the PTY's foreground process group. A shell's screen text is prose it
  happened to print and must never arm a working state. A background job is
  deliberately not part of this predicate: bg jobs survive a preserving
  handoff (the PTY fd moves with them), so they are protected by the
  handoff-integrity law, not by the gate.
- **An app row (a Shell whose launch verb is a local app — e.g. a ychrome
  launcher) is RUNNING, not working**, by design: its one long-lived
  foreground process would otherwise pin every update forever.
- **The phrase table is DATA and goes stale silently.** It is audited against
  the live CLIs — drive each CLI in a PTY, read its working screen through
  `server gate-screen`, and fix the needle — not argued from memory. The
  audit is standing campaign work (pending-bugs [11.93]); a CLI whose working
  footer the table misses reads IDLE mid-turn, which is the dangerous
  direction: the update fires into a live turn.

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

⇒ **The deadline is aimed at the COLD SHUTDOWN, and only at it.** That is the
path where nothing has swapped and the host stays stale. Under a preserving
handoff the successor is already serving every row, so what remains is ownership
tidiness — never worth interrupting a live turn for, and interrupting for it
would be the bare timeout the old prohibition was written about.

### §5.1 Amended 2026-09-10 (owner ruling): WORKING is exempt — the old daemon serves

⚖ Owner: *"In case of running sessions like these we should connect to the old
daemon. Only non-working daemons are auto updated and reattached to the GUI."*

The 30-minute force was written for a world where the only alternative to
forcing was a host stuck on a stale build. That world is gone: **version
coexistence is legitimate** — a daemon holding a working turn is not
stale-in-waiting, it is the SERVING daemon for the sessions it holds, and its
clients keep attaching to it. From today:

- `WORKING` joins `ORCHESTRATING` and `NOT_RESTORABLE` as deadline-exempt. The
  automatic update path never interrupts a mid-turn session; the update lands
  when the daemon goes quiet.
- The force arm itself remains for the one set it can still honestly fire on:
  deadline-stale `recently_active` blockers — a session quiet before the wait
  began and still quiet at its end. The `continue` repair travels with it,
  unchanged, for that case and for the user-initiated hot-restart verb, where
  a human pressed the button that may interrupt.
- **What keeps a legitimate wait honest is §13**: after a day held back, the
  daemon tells the user which rows hold it. A wait that cannot be interrupted
  must at least be visible.

Measured the morning of the ruling, on the GUI host and on dev: a same-version
newer-build rotation released only 2 of 4 owned sessions, left the predecessor
unreachable by name holding 9 PTY masters, and both dev codex rows ended at
the twin-writer *"open in another app"* screen — a state no `continue`
repairs. That is the cost this amendment retires.

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

⭐ **Measured 2026-08-10 on `dev`, across every Claude Code transcript on the
host — this is how invisible the state was.** Of **73,764 sub-agent records in 33
sessions, 21,453 (29.1%) were written while the parent transcript was silent for
over a minute**; the longest silence around a live sub-agent record was **30.6
minutes**, which is past the gate's 300 s window *and* past §5's deadline. ⇒ for
roughly a third of all delegate work, every signal the daemon had said "idle".

⛔ **And the obvious instrument is the wrong file, which §8's "positive, not
inferred" does not by itself protect you from.** `isSidechain` reads `false` on
**all 179,392 records of the parent transcripts** on that host, across sessions
that made **195 `Agent` and 29 `Workflow` calls**: sub-agents write to
`<session-id>/subagents/agent-<id>.jsonl`. A detector pointed at the parent is
positive, declared, correct-looking — and never fires. Point it at the
sub-agent files.

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

## 9. The observability contract — a handover you cannot see is a handover you cannot trust (2026-09-05)

The hot-restart mechanism spent weeks being blamed for defects it did not
cause and silently causing others; both costs came from the same hole — the
handover was not OBSERVABLE end to end, so every attach failure looked like
the swap and every swap fault looked like an attach failure. The owner's
ruling after the all-CLI attach plague: **the tracing must be wired so the
smooth handover is observable and any fault is caught by name.** The contract
every handover-path change must satisfy:

1. **Every swap emits a joinable event family.** Arm (the gate decision with
   its reason), handoff (predecessor pid + build, runtime count preserved),
   bind (successor pid + build, socket taken), adopt (per-runtime adoption
   or its deferral reason), drain (progressive migration releases). One
   identifier must join predecessor and successor sides so a single swap can
   be replayed from the trace alone.
2. **Faults are named events with self-explaining payloads.** No silent
   branch on the handover path: a refusal carries WHY per holder
   (`why_not_reaped`), a deferral carries the gate that held it, a timeout
   names the phase that expired. The test is the 2026-09-05 standard: the
   next screenshot of a stuck row must be diagnosable from its own trace
   payload, without a hand `/proc` investigation.
3. **Absence is not evidence.** Lossy writers and abrupt exits mean a missing
   `daemon_self_retire` proves nothing (measured: a daemon vanishing with
   zero lifecycle events). Observability of a COMPLETED handover is
   positive: successor birth + adoption events on the new build's pid.
4. **The state machine of §3 and the queue of §4 report through it.**
   `hot_restart_blockers` must never be empty while a swap waits (§8's law),
   and each blocker names the session holding it.

## 10. The convergence unit is the BUILD, not the version (2026-09-05)

The relay gate fires on version skew, but the fleet's update cadence makes
**same-version newer builds a normal state**: a host can run yesterday's
build of the same version for hours beside today's. The different-bytes law
(`disk_binary_replaced` means different bytes, size+mtime latched,
`/proc/<pid>/exe` compared) already governs daemon self-retirement; the gate
and the convergence detection must compare BUILD identity the same way, so a
same-version newer build arms a handover exactly as a version bump does.
Under this spec a "stale daemon" is: older version, OR same version with
different (newer) build bytes on disk. The bidirectional convergence spec
carries the same rule for the client side.

## 11. The update model — old daemons serve, quiet daemons update (2026-09-10)

⚖ Owner, verbatim: *"One daemon or the client is updated. Then the client is
auto updated no issues. But only non-working daemons are auto updated and
reattached to the GUI. This includes plain shells having no fg/bg process
running in them."*

The laws this adds on top of §3/§5:

1. **The client never gates the daemon.** A GUI/CLI version skew is ordinary
   and self-healing — the client updates itself and reconnects. Nothing in
   the update path may break a running session because a CLIENT is behind.
2. **A daemon with any working session is not auto-updated.** It keeps
   serving; its clients attach to IT (the preserved-owner path is the
   mechanism; §9's observability contract is what keeps that attach honest).
   When the last turn ends and the idle window passes, the normal arms take
   over: preserving handoff if a successor exists, cold retire otherwise.
3. **A plain shell with no fg/bg process is the updateable case, and it is
   moved, not killed** — by full-fidelity snapshot (§12) when a successor
   cannot simply inherit the PTY, and by the fd handoff when it can.
4. **"The daemon the row is on" is the user-visible truth.** The daemon rail
   and the census must keep naming which process serves which row — including
   a predecessor that is draining — so "connect to the old daemon" is
   something the user can SEE, not a private arrangement.

## 12. The plain-shell snapshot law — full color, not just text (2026-09-10)

A plain shell's state IS its PTY (§`session_kind_state_survives_pty_loss`),
which is why shells pinned cold retires forever as permanent blockers and
why "very stale daemons kept running" was a standing fear. The owner's model
makes shells movable:

- **On a swap, a quiet shell's terminal state is captured as a FRAME SNAPSHOT
  in full color fidelity** — the same fidelity as any frame snapshotting: SGR
  attributes, 24-bit color, cursor position, alternate screen state — never a
  stripped plain-text transcript. The daemon's own vt100 screen model is the
  source of truth (it watches the PTY from birth and already holds the
  rendered grid with attributes).
- **After the update, the snapshot is PASTED back** into the fresh PTY before
  the row is revealed, so the user's scrollback-visible screen, colors and
  prompt position survive the swap. The entire scrollback history moves with
  the same mechanism.
- A shell with a foreground job is NOT snapshot-moved — it is working (§3.1),
  so its daemon simply waits (§11.2). The snapshot path exists for the quiet
  shell, replacing its old standing as an unmovable blocker.
- Fidelity bar: a user staring at a shell row across an update must not be
  able to tell it swapped — colors, formatting and cursor position included.
  The falsifier is exactly that: drive a shell that paints colored output,
  swap the daemon under it, compare the frame before and after.

## 13. The stale-held notification — a wait you cannot see is a wedge (2026-09-10)

With WORKING deadline-exempt (§5.1), a daemon can be legitimately held from
an update for a long time — and the fleet has already lived the version of
this that goes wrong (a 2.10.3 daemon serving beside a 2.10.13 build for
19h44m, invisible; an eighteen-daemon stack, the oldest 20.6 days). The
owner's ruling: *"notify the user after 1 day of stale daemon being hold
back, by which session row, so that user understands what yggterm
understands."*

- **After 24 hours of continuous deferral, the daemon raises a desktop
  notification naming the holding session rows and their blocker kinds** —
  `stale_daemon_update_held_24h` in the trace, best-effort `notify-send` on
  the host. It repeats once per day while the hold persists.
- **When the hold clears, the clock resets** — a fresh hold counts from zero,
  so the notification says what is true NOW, not what was true last week.
- The census and the daemon rail keep carrying the same answer synchronously:
  a reader must be able to go from the notification to the row to the remedy
  without grepping a trace.

### §13.1 Named remainder — the lingering PREDECESSOR owes the same notice

The notification above arms on the cold-retire deferral (the daemon that
cannot retire). The other stale-daemon shape is the HANDOFF PREDECESSOR that
drained nothing and lingers serving its rows — measured 2026-09-10 09:19 on
the GUI host: a predecessor unreachable by name still holding 9 PTY masters
while the successor owned the canonical socket. A predecessor that still
owns rows 24 h after its handoff owes the user the identical notification
(same trace name, `role: predecessor`), and its retirement plane's
verdicts ([11.67] family) should cite it. NOT YET BUILT — filed here so the
next session lands it with the same falsifier discipline.
