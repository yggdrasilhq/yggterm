---
name: yggterm-onboarding
description: Use when someone is setting up yggterm, extending an existing setup, or asking how to work well inside it — "help me set up yggterm", "how should I organise my sessions/rows/machines", "audit my fleet", "how do I make my agents find my data", "what should go in my global agent instructions". Guides a human of ANY experience level from a bare install to a working multi-machine agent fleet: the mental model, the row and seat scheme, relays and orchestration, how to build their own data-fabric skill so agents stop hand-walking the filesystem, and which steers belong in their global instructions. Runs as an INTERVIEW, not a manual — read the person's level first and pitch to it. Also runs as an AUDIT against a live fleet.
triggers:
  - set up yggterm
  - onboard me to yggterm
  - audit my fleet / my yggterm setup
  - how should I organise my sessions or machines
  - how do I make a data fabric skill
  - what should be in my global agent instructions
  - my agents keep losing track of my sessions
---

# Onboarding someone into yggterm

**You are guiding a human, interactively. This is not a document to recite.**
Read what they already have, ask what they are trying to do, and pitch every
answer at the level you find them at. Someone who has never run a terminal
multiplexer and someone who is tuning a three-machine fleet both arrive here, and
the same words will fail one of them.

⛔ **Do not dump this file at them.** Work through it, one phase at a time, doing
the setup *with* them and showing the result. If they say "just do it", do it and
narrate what you did and why — the narration is the onboarding.

---

## What yggterm actually is, in one honest paragraph

Yggterm replaces the pile of editor terminals, multiplexer panes and remembered
`ssh` incantations that agent CLI work otherwise becomes. Its core promise: **you
click a session and it does the equivalent of `ssh <machine> "cd <dir> && <agent
CLI> resume <id>"` and hands you the terminal.** You just type. Everything else —
the sidebar, the start page, the apps — exists to make that click findable.

Two classes of thing live in it, and conflating them is the most common early
confusion:

| | what it is | survives what |
|---|---|---|
| **agent CLI sessions** | first-class. Organised by working directory. The CLI persists itself; yggterm's job is to faithfully re-invoke it | everything, including yggterm restarts |
| **plain shells** | second-class. Connect to yggterm's own multiplexer layer | GUI death **only if** marked keep-alive |

**Architecture, and why it matters to a user:** a **daemon** owns the terminals; a
**GUI** is a viewer onto them. They are deliberately separate so the app can be
updated — daily, several times a day — **without costing anyone their work**. When
you understand that the daemon holds your sessions and the window is just glass,
most "did I just lose everything?" moments stop happening.

---

## Phase 0 — find out who you are talking to

Ask two questions, and let the answers set the depth for everything after:

1. **"What do you want to be able to do that you can't do today?"** — this gives
   you the goal. Someone who says *"stop re-typing ssh commands"* needs Phase 1
   and 2 and should stop there. Someone who says *"I want six agents working in
   parallel and to not lose track of them"* needs all of it.
2. **"How many machines, and do your agents run on them or on this one?"** — this
   gives you the shape. One machine is a genuinely complete setup; do not push a
   fleet on someone who does not need one.

⚠ **Signals to pitch DOWN**: they ask what a daemon is, they have never used tmux
or screen, they call the whole thing "the terminal". ⇒ use the glass-and-projector
analogy, avoid the word "row" until you have shown them one, never mention seats
or relays in the first pass.

⚠ **Signals to pitch UP**: they already run agent CLIs over ssh by hand, they ask
about persistence or restarts, they mention losing sessions. ⇒ go straight to the
daemon/GUI split and the keep-alive distinction, then to Phase 3.

⛔ **Never guess their level from their vocabulary alone.** Confident-sounding
wrong models are common and the interview is what catches them.

---

## Phase 0.5 — what a fleet is and why you would want one

**Say this in plain language before you ask about their machines — most people have
never been offered the choice.**

A **fleet** is just yggterm running on more than one machine that trust each other
(say `laptop`, `builder`, `gpu` — ssh aliases, one per machine). Every row
knows which machine its terminal lives on, and clicking a row does the `ssh` + `cd`
+ `resume` for you. From your chair there is one sidebar; behind it are several
computers acting as one.

**Pitch the *why* at their pain, not the mechanism:**

- **Cool laptop, hot work elsewhere.** The GUI lives on the laptop you touch;
  the heavy agents run on the server that has the cores, the RAM, the GPUs, the
  repo checkouts. The laptop stays cool and quiet even with six agents grinding on the
  headless hosts,
  because the timers are `Nice=10` + `IOSchedulingClass=idle` and coalesced by the
  kernel — they wake, do 200ms, and exit. No resident Python loop burning your fan.
- **Right tool on the right host.** A browser-automation agent that needs the screen
  runs where the screen is; a build agent runs where the build cache is; a GPU job
  runs where the GPU is. Yggterm files that "which host" per-row, so you never re-type
  `ssh dev cd ~/gh/foo` again.
- **Memory that follows you.** With fleet memory (`~/.yggterm/memory` + `ygg-memory`),
  a finding Claude proves on one host is visible to Muse on another ten minutes later, via
  a background `sync-fleet` mesh over ssh. No hand-copying, no stale host re-breaking
  what another fixed. Deletes propagate via tombstones, not resurrection.
- **One machine is a complete yggterm.** You do not *need* a fleet to get value.
  Start with one; add a second when "my laptop is hot" or "my builds are slow on
  wi-fi" becomes real. The fleet is an *upgrade path*, not a prerequisite.

**Then the handholding promise — this is the USP to name out loud:**

> "All the advanced dances — which machine a job deploys on, which harness's memory
> is authoritative, which timer wakes when, how a relay hands off without losing
> context — look like choreography from the outside, but from your side they are
> child's play. **You just tell the agent what you want in plain language, and it
> wires or re-wires yggterm for you.** 'Make fleet sync every 5 minutes', 'keep my
> laptop off battery sync', 'sync only yggterm, not everything' — one sentence, one
> timer line changed, shown to you. That conversation *is* the product. You are never
> locked into the setup we do today."

**Name the flexibility explicitly, so they know to ask:**

- Intervals: `ygg-memory-fleet 10m` → `5m`, `ygg-booter-tick 7m` → `3m` — one `systemctl --user edit` line, show `list-timers`.
- Scope: `all namespaces` vs `just yggterm + cwd` — one hook flag.
- Deletes: propagate vs archive — tombstone vs `~/.yggterm/memory-archive`.
- Harnesses: `claude muse gemini codex grok` — comment out what they don't use.
- Power: `ConditionACPower=true` for laptops on battery.

If they are single-machine, say so: *"You get everything above, one host is the whole
fleet, and adding a second later is the same command plus an ssh alias."*

---

## Phase 1 — the fleet shape

Establish, with them, what their world contains. Write the answers down somewhere
durable as you go; this becomes the seed of their fabric skill in Phase 4.

- **Which machines**, and what each is FOR. A useful split when there is more than
  one: a machine that **builds and integrates**, a machine that has the **screen
  and the hands** (where a human actually looks), and anything with a **deploy
  surface** of its own. Name them by role, not by hardware.
- **Where the agent CLIs live** on each, and whether they are the same version.
- **Which machine's GUI is the one that matters** — the surface where a change is
  *proven*, as opposed to merely compiled.
- **Fleet or not?** After the "why a fleet" pitch above, ask: *"Does any of that
  sound worth it for you now, or is one machine the right start?"* Respect a "one
  is enough" — it is a complete setup.

⭐ **The rule that saves the most pain later: deploy every job on the host that
owns the thing it touches.** Building on the wrong machine deploys to nobody, and
that mistake is quiet — it looks like success.

---

## Phase 2 — rows, seats and names

A **row** is one session in the sidebar. Once there are more than about a dozen,
finding one becomes the actual problem, so the naming scheme is not cosmetics.

**The scheme:**

```
        2.3   deploy pipeline: make the staging push idempotent
        ───   ──────────────   ───────────────────────────────
         │           │                     └── what this row is FOR
         │           └── the CATEGORY — stable across successors
         └── the SEAT, stored separately and composed on at render time
```

⛔ **The number never goes in the title.** It is stored in the row's own seat
field and composed onto the label when the sidebar draws it. Put it in the title
as well and the sidebar shows it twice — and once several rows wear two numbers,
nobody can tell a seat from a name at the moment they most need to.

**Number them like a book**: `1`, `1.1`, `1.2`, `2`. Top-level per project or
theme, sub-seats per parallel worker. It makes "which row is this?" answerable and
makes a long sidebar navigable.

**Have them do it once, now**, on a real row, and look at the sidebar together.
The scheme only makes sense once seen.

---

## Phase 3 — relays and orchestration (skip for single-machine, single-task users)

Two patterns, and they compose:

**A relay** is a long-running piece of work that outlives one session's context. A
session works until it is nearly full, writes down what it knows, **spawns its own
successor**, hands over, and is retired. The campaign continues; the session does
not. ⇒ The thing that makes this work is that the state lives in **files**, never
in a session's head.

**An orchestrator** is one row that routes and N rows that grind. It clusters the
open work, launches a row per cluster, monitors them, merges what comes back, and
— importantly — **owns the machinery itself**: the naming, the watchers, the
handover protocol. Clusters never fix the machinery; to a cluster it is background
weather it routes around silently.

**Two rules worth stating to a newcomer even though they sound advanced:**

- ⛔ **A verb reports the REQUEST, not the EFFECT.** Read state back after every
  operation that matters. A remove can report the row gone and the process alive,
  in the same reply.
- ⛔ **Prove a delegate got its brief by finding a token from that brief in its
  own transcript.** A launch that dropped the entire brief still reports success
  and still produces a live-looking session. This has cost real days.

The full contract is in the **`yggterm-agent-fleet`** skill. Point them at it when
they are ready; do not front-load it.

---

## Phase 4 — build THEIR fabric skill ⭐ the highest-value hour in this whole process

**The problem it solves:** an agent that does not know where things live will
hand-walk the filesystem, guess, and get it wrong — every session, forever,
because an agent's memory resets and a file's does not. A **fabric skill** is one
document that answers *"what exists and how do I reach it?"* so that no session
ever has to rediscover it.

⇒ **This is the single highest-leverage artefact in an agent setup**, and it is
worth building even for someone with one machine and one project.

**Build it WITH them, by interview. The questions that produce a good one:**

1. **"What are the big stores?"** — repositories, note vaults, media libraries,
   document archives, databases, anything with an API. For each: *where it lives*,
   *what question it answers*, and *what it is NOT for.*
2. **"What has ever taken you more than five minutes to find?"** — the honest
   source of the map. Whatever they name goes in first.
3. **"What do you reach for that needs a login, a token or a device?"** — these
   are the reaches an agent will otherwise declare impossible. Each one gets a
   **tested recipe**, not a description.
4. **"What must an agent NEVER do here?"** — read-only stores, real-money actions,
   anything requiring their explicit approval. Boundaries belong in the same file
   as the map, or they will not be read.
5. **"Where does your reasoning live?"** — notes, decision logs, discussion
   threads. An agent that can retrieve *why* a past call was made stops relitigating
   settled questions, which is a large and invisible saving.

**The shape to write:**

```markdown
---
name: <their>-fabric
description: Use from ANY repo when a task touches <their domains>. Gives the
  store map, tested access recipes, and the boundaries. Invoke BEFORE answering
  "what data exists", before hand-walking the filesystem, and before declaring
  anything unreachable.
---
## The stores            <- a table: where · what it answers · what it is NOT for
## Access recipes        <- COMMANDS THAT HAVE BEEN RUN, not descriptions
## Routing               <- which question goes to which store
## Boundaries            <- read-only, approval-gated, never-touch
## Where decisions live  <- the reasoning layer
```

**Three laws to write into it, because they are what make a fabric skill compound
rather than rot:**

- ⛔ **A recipe that has never been run is a guess.** Run each one while writing it
  and paste the real invocation.
- ⭐ **When a session learns a durable nuance — a moved path, a changed endpoint, a
  stale assumption — it writes it back into the fabric skill as part of the task.**
  A finding left in a chat transcript was never learned.
- ⛔ **"Blocked" is a claim to falsify, not a fact to record.** Most "I can't reach
  that" is answered by a door already in the fabric. Check before writing it down,
  and re-check before repeating an inherited one.

---

## Phase 4.5 — Wire their unified cross-harness memory (`~/.yggterm/memory`)

**The problem it solves:** when you run Claude Code on Monday, Muse on Tuesday, and
Gemini/Codex on Wednesday on the same project — possibly from different machines
— each harness and each host starts isolated. Critical bug findings,
campaign handover ledgers, and rules learned by one agent are invisible to the next.

`~/.yggterm/memory` is the host-resident **unified fleet memory**: one SSOT that every
harness on every host converges through. It is why a Muse session on the builder sees
what Claude learned on the laptop yesterday, without anyone hand-copying files.

**Why this saves them real time and money:**

- **No re-derivation tax.** A finding proven once (`finding-*.md` with a code citation)
  is read in 20 tokens via `ygg-memory diff` instead of re-debugged for 15k tokens.
- **No fleet drift.** Without it, `~/.claude/projects/<ns>/memory` on two hosts
  diverge silently — the same bug gets fixed twice, then re-broken by a stale host.
- **No resident watcher cost.** Sync is not a forever Python loop; it is
  kernel-coalesced timers (`systemd --user` on Linux, `launchd` on macOS, Task Scheduler
  on Windows) that wake, do a 200ms local catch-up, and exit. CPU > RAM > SPACE.

**Set it up WITH them, once per project — and once per fleet:**

1. **Bootstrap the memory tree (zero-turn wiring)**:
   ```sh
   .agents/skills/yggterm-agent-fleet/bootstrap.sh
   # also wires ~/.yggterm/memory/namespaces/<ns> if missing
   ```
   This creates `MEMORY.md` ("Doors, not rooms") with the steering header:
   ```markdown
   > 🌐 **UNIFIED FLEET MEMORY**: Before deep memory recall or after campaign handovers, consult `ygg-memory status --harness <me>` or `ygg-memory diff` to catch updates from Claude, Grok, Codex, Gemini, or Muse. Ingest full or partial diffs as needed.
   ```

2. **Wire auto-sync so they never have to remember it** (do it *with* them, show the timers):
   ```sh
   # Fast path: current project + yggterm at session start (blocking <500ms)
   ~/.local/bin/ygg-memory-sync claude        # hook does this; also: muse/gemini/codex/grok
   # Slow path: full fleet mesh in background (systemd-run, 2s delay, coalesced)
   #   ygg-memory sync-harness --all + ygg-memory sync-fleet
   #   (sync-fleet reads the peer roster from ~/.config/ygg-fleet/mesh — one ssh alias
   #    per line, outside every checkout, so no repo ever names their machines)

   # Persistent timers (installed by onboarding, kernel-optimized):
   systemctl --user enable --now ygg-memory-fleet.timer     # 10m, RandomizedDelaySec=45
   systemctl --user enable --now ygg-memory-harness.timer   # 15m catch-up
   systemctl --user enable --now ygg-booter-tick.timer      # 7m (was resident 5m loop)
   systemctl --user enable --now ygg-monitor-tick.timer     # 6m (was resident 3m loop)
   ```
   Explain: `AccuracySec=1m` lets the kernel batch wakes; `Persistent=true` does one
   catch-up after sleep instead of N; `Nice=10 IOSchedulingClass=idle` keeps it off
   their hot path. On macOS show the `launchd` plist equivalent; on Windows the
   Task Scheduler repetition. **The timers replace resident `watch` loops** — that is
   why the laptop stops running hot when the work is on the headless hosts.

3. **Teach the turn-one retrieval ritual (<40 tokens, but now mostly automatic)**:
   Auto-sync already pulled `yggterm` + cwd. Teach the manual fallback for cross-project recall:
   ```sh
   ygg-memory status --harness <me>          # ~25 tokens: check if behind
   ygg-memory diff --harness <me>            # ~80 tokens: view delta summaries
   ygg-memory get --file <finding-or-campaign.md>  # read on demand
   ygg-memory ack --harness <me> --all       # mark absorbed
   ygg-memory sync-harness --harness <me> --all   # force full pull (rare)
   ```

4. **Teach the knobs they can tune as they talk with agents** (flexibility, not dogma):
   - **"Sync everywhere or just yggterm?"** Default is yggterm + cwd fast, full fleet background.
     If they want instant full: change hook to `ygg-memory sync-harness --all` blocking.
     If they want leaner: keep only fleet timer, drop harness timer.
   - **"How often?"** Timers are `10m/15m/7m/6m` — show `systemctl --user list-timers` and
     `journalctl --user -u ygg-memory-fleet -n 20`. Let them say "make fleet 5m" — it is
     one `systemctl --user edit` line.
   - **"Deletes should propagate?"** Now yes via tombstone (`action=delete` in `journal.jsonl`).
     If they archive instead of delete, use `~/.yggterm/memory-archive` + publish with `kind: archive`.
   - **"Which harnesses?"** `for h in claude muse gemini codex grok` — comment out what they don't use.
   - **"Battery / metered?"** Add `ConditionACPower=true` to timers, or keep background `systemd-run` only.

5. **Teach the Lore Economics (why SOTA models write doors)**:
   - **SOTA models** (Claude Opus, Muse Spark xhigh, Gemini Pro) find gotchas, prove them, and publish doors (`ygg-memory publish --file <finding.md>`).
   - **Cheaper models** (Flash, Haiku, small local models) grind the execution without paying expensive re-derivation taxes.
   - **Tooling/Verbs** automate the janitorial work and make failure modes unrepeatable.

**What to say out loud while wiring it:**

> "This is the one setup that keeps paying you back. Without it every new session
> re-learns what the last one already knew — on a different harness or a different
> laptop — and you pay for it in tokens. With it, the first thing a fresh session
> does is ask 'what did the fleet learn since I last looked' — 25 tokens — and the
> timers keep the fleet identical while you sleep. If you ever want it tighter or
> lazier, just tell the agent: it's one timer line."

---

## Phase 5 — the steers that belong in their global instructions

Global agent instructions are read every session, so **every line is a tax on all
of them**. Keep them to doors and laws; detail belongs in the skills they point at.

**Recommend exactly these, adapted to their words:**

1. **A pointer to their fabric skill, with the trigger words that must fire it** —
   including the "I can't reach that" family, which is when it is most needed and
   least reached for.
2. **A pointer to the fleet skill before touching any row**, because row verbs
   report requests rather than effects.
3. **Where status lives** — one question, one owner. What is OPEN, what SHIPPED,
   what is waiting on the human. ⛔ A second copy of "what is open" is how a queue
   rots unseen.
4. **How to read another session** — Phase 6 below. This one pays for itself
   immediately.
5. **Whatever they have already had to correct twice.** That is the real signal:
   a correction that did not stick is a missing steer.

⛔ **Do not copy someone else's global instructions wholesale.** Most of any such
file is one person's scar tissue and will read as noise to anyone else.

---

## Phase 6 — reading another session intelligently ⛔ teach this explicitly

Two ways to find out what another session is doing, and **both obvious ones are
wrong**:

- ⛔ **Asking it** ("what were you working on?") wakes it. An idle session pays a
  **cold re-read of its entire context** to produce a self-report you cannot
  verify. You are buying the wake, not the answer.
- ⛔ **Reading its transcript whole** moves that same cost into *your* context and
  you carry it for the rest of your session — when the signal you wanted was in
  the last one percent.

⇒ **EXTRACT, DO NOT INGEST.** Cheapest instrument first:

| question | instrument |
|---|---|
| Is it alive, how cold? | transcript **mtime** |
| What would a wake cost? | transcript **size** |
| What was it TOLD? | the **human turns** — highest signal per byte in the file |
| What did it CONCLUDE? | its **last prose turn** — a working session's own status report |
| What did it DO? | the files it wrote, and the commit log |
| Does it know about X? | a **targeted grep**, and count the hits |

**Then cross-check against what cannot lie**: the commit, the queue entry, the
files. A transcript says what a session *believed*; a commit says what it *did*.
When they disagree, **the artefact wins.**

✅ **Messaging another session is for DELIVERING, not enquiring** — a brief, a
correction, a warning that changes what they do next. Then it earns its cost.

---

## The AUDIT mode — running this against a fleet that already exists

When someone says *"look at my setup and tell me what to fix"*, work through this
and report findings, most-costly-first. What to look at, and what "wrong" looks
like:

| check | what wrong looks like |
|---|---|
| **Row names** | numbers appearing twice · two rows sharing a seat · titles that do not say what the row is for |
| **Duplicate seats** | almost always a handover where the predecessor was never retired — check whether the retiring tool reported success |
| **Daemon population** | many daemons, several older than the current binary. Ask what each still owns before proposing anything |
| **Stalled rows** | a turn ENDED with work unfinished and no error. Invisible unless something watches; a finished row and a stalled one look identical from outside |
| **Watchers** | watching a set captured at launch, so every handover silently drops the successor. Re-derive from live state |
| **Fabric skill** | absent, or present but full of untested recipes |
| **Global instructions** | duplicated status, or laws with no door to the detail |
| **Resource at rest** | what does the app cost doing nothing? Measure **growth against uptime**, not a point sample — a single reading names a symptom, a trend names a class |

⛔ **Report what you MEASURED, and separate it from what you INFER.** An audit
that presents a guess in the same voice as a measurement is worse than no audit,
because it is acted on with the same confidence.

⭐ **Then offer the smallest change with the largest effect first.** Most setups
are one naming fix and one fabric skill away from working well, and a person who
gets a win in ten minutes will do the rest.
