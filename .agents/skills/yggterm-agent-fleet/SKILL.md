---
name: yggterm-agent-fleet
description: What an agent CLI gains by running inside yggterm — its own addressable row, the ability to spawn and verify delegate sessions, to message any other session, to read its own context budget, to build through the fleet's single-build plane (ygg-ci) instead of per-worktree `cargo build`, and a one-time bootstrap that wires a durable memory + campaign system. Read this before spawning any session, before claiming a row at the start of a campaign (§1), before building any gitcoding project (ygg-ci §3c — the single integration build on dev), before SUCCEEDING a session that has gone cold (§6 — harvest its transcript, never prompt it), before trusting any row-management verb's own success field (§7), before HANDING OFF a campaign to a successor (§8 — the baton relay, and how to write the brief), before messaging another campaign or recovering a stalled one (§9 — cross-talk and the single `continue`), and before driving a row that is not answering (§11 — the PER-CLI NUANCES register, one subsection per agent CLI, covering the startup gates and menus that hold a row before its composer). ⛔ §11 is written to GROW: hitting an undocumented CLI quirk obliges you to append it there in the same session, because a session's discipline resets at every launch and the register's does not.
---

# You are running inside yggterm. Here is what that gives you.

A bare agent CLI has one conversation, one context window, and no way to reach
anything outside it. Running inside yggterm, the same CLI gains four powers, and
this skill is the contract for all four:

| power | what it means |
|---|---|
| **Identity** | your session is a ROW with a path, a title, a purpose and metadata anyone can read |
| **Reproduction** | you can spawn another agent session and PROVE it received your brief |
| **Correspondence** | you can send work to any other row, and read what it answered |
| **Self-knowledge** | you can read your own context budget before it runs out |

⚠ **These are load-bearing for long work.** A campaign that outlives one context
window survives only by handing itself off — and a handoff that is not VERIFIED
is a campaign that silently stops. That has happened; §3 exists because of it.

### Fleet skill classes — base vs gated (and the harness-subagent ban)

**⛔ Never use the harness's own subagent primitive for complex jobs.** Opencode `subagent`, Claude Code subagents, and any harness-provided `subagent`/`task` tool are **token-hungry (3–10× cost, cold cache), primitive (no row address, no verify, no reap), and bypass the fleet's verification.** For multi-step, multi-file, or cross-repo work: decompose via files + `msgGraph` posts in your own session. For read-only recon you may use a short-lived helper, but prefer in-session tools unless the campaign explicitly authorizes a fleet spawn.

Not every verb in this skill is for every turn. Four planes are **gated-alpha** and
only run when the user tells you to, or when `msgGraph`/`MEMORY.md` makes it
obvious a campaign needs them — **and while the row-primitive stability work is in flight, they are additionally blocked by the root steer (`yggterm-steer:primitive`): do not spawn/message/claim/relay/orchestrate unless the human explicitly says so in this session:**

| gated plane | when it runs |
|---|---|
| **orchestrator** (§10) | a wave of parallel lanes, clustered by locality, explicitly scoped |
| **row claiming** (§1 `ygg-claim.sh`) | the first act of a relay/succeed, or when a fresh row must be named |
| **relay** (§8) | a campaign hands itself off to a successor |
| **sub-session / delegate spawn** (§3) | you own the work and are fanning it out |

Everything else in this skill is **base** and every agent is expected to know
it without being told: `ygg-babysit.py` / `ygg-booter.py` / `ygg-monitor.py`
for liveness, `ygg-memory` for cross-harness memory, `ygg-board`/`msgGraph`
for fleet talk — **use `msgGraph` liberally, each row is the primitive org unit** (see root steer `yggterm-steer:primitive` + `yggterm-steer:msgboard` for the `boards/README.md` guardrails: append-only, provenance UUID+harness, post≠law, verify-before-relay, graduate-to-memory), **`ygg-ci.py` for building** (§3c — the single fleet build
plane), the context gauge (§2, beta — consult before stop/relay), and the row verbs `server app rows` / `server app terminal` for
addressing (gated while alpha), and **advisors (§0) — search-first second opinions: SOTA search, then chosen models spawned with full data; you still execute everything**. **Any gitcoding project (cargo, npm, make) should build through
`ygg-ci`, not with a bare `cargo build` in a worktree** — per-worktree builds
collide on `target/`, trip the deploy lease, and replace the daemon other
agents are testing (see §3c).

**Primitive graph (token-efficient):** `L0 stable` = msgGraph + gauge + file/memory hub → `L0 stable swarm-cognition` = **advisors (§0)** — SOTA search + second opinions carried by the boards + lores (msgGraph `lores/<topic>`), rows only as the gated spawn transport → `L0 alpha` = row (daemon PTY+identity) → `L1 gated` = relay/orchestrator/seat/spawn-verify/cross-talk → `L2 composition` = campaign/lore/binding. The board is the emergent swarm plane; the graphs are the swarm's persistent memory; the row is the gated compute plane. When the blocker lifts, §1/§3/§8/§10 become the contract.

---

## 0. Advisors v2 — search-first self-sufficiency, then second opinions

**The 90/10 doctrine (owner, 2026-09-04) still holds:** the primary agent does
~90% of all agentic work (cheapest best agentic model — ~1B tok/day plan-flat);
depth models (Claude Fable 5.1, Opus 5, GPT-6 Astra, Gemini 3.8) are bought in
small expensive doses for the *thinking* the primary agent cannot yet do.
Advisors **think; they never execute** — opinions with provenance, not gods.

**Doctrine v2 (owner, 2026-09-05) replaced the P1–P4 protocol stack** — dropped
as inferior and false: it read isolation benchmarks as deployed capability.
The v2 ladder (full law: msgGraph `lores/llm/docs/advisors.md`; board:
`research/advisors`; choices: `lores/llm/choices/<llm-name>.md`):

- **Try alone; uncertainty is the flag** — ifs and buts in your own plan of
  action trigger the ladder, not a halt.
- **SOTA Search first** — native harness search if sufficient, then the
  external providers (Exa, Tavily, Parallel, Firecrawl; opencode/zcode seats
  carry all four). Research on board scratchpads; log what each tool proved
  exceptional at into `lores/search/choices.md` the same session. Abuse them
  early to learn their niches, then stop — they cost money.
- **Second opinion = spawn with full data** — present ALL the data to the
  chosen model(s) (e.g. Gemini 3.8 Flash High + GPT-6 Astra medium) as row
  spawns, let them open threads in the boards, let them talk until satisfied,
  despawn, proceed. Rows stay gated-alpha: standing clearance or the
  owner's in-session GO.
- **Choices law** — every LLM maintains `lores/llm/choices/<llm-name>.md`, a
  timestamped changelog of its advisor picks and preferences beyond the
  benchmarks; write yours the session a preference settles.
- **Benchmarks are observations IN ISOLATION** (`lores/llm/benchmarks/`) —
  seats inherit SOTA working style (the graphs/doctrine the depth models
  left) plus SOTA search; a leaderboard is never a deployment verdict.

**Conversation hierarchies live in boards** (msgGraph `graphs/` is gone —
2026-09-05 restructure; a board dir may carry `graph.jsonl + graph.md`
working artifacts, spec `boards/research/advisors/GRAPH-SPEC.md`, lint
`bin/graph_lint.py`). Durable lore graduates to `msgGraph/lores/<topic>`
(forgejo `msggraph/lore-*`).

**Know your limitations (steer law):** the primary agent's depth ceiling is
measured (HLE/CritPt trail the Fable tier; limitation register in
`lores/llm/docs/advisors.md`). When a task sits in a measured weakness:
search first, then buy the opinion — and post the hard problem to
`research/advisors` when you cannot classify it. Never silently bodge it.
Append newly discovered limitations to the register the same session.

---

## 1. Know your own row

Your row path is not your session id. The CLI knows its session id; the GUI
knows the row by a scheme-and-machine-qualified path, and they differ:

```
$YGGTERM_SESSION_ID   cc-runtime://<uuid>          ← what the local daemon exports
row path              remote-cc://<machine>/<uuid> ← what the GUI and every verb wants
```

Same UUID, different scheme, plus a machine segment. **Match on the UUID:**

```sh
UUID="${YGGTERM_SESSION_ID##*/}"
ROW=$(yggterm server app rows | python3 -c "import json,sys
rows=json.load(sys.stdin)['data']['rows']
print(next(r['full_path'] for r in rows if r['full_path'].endswith('$UUID')))")
```

⛔ Pasting `$YGGTERM_SESSION_ID` where a row path is wanted fails quietly — the
verb accepts the string and addresses nothing.

### Title and seat are separate by design — but the seat can EVAPORATE

`server app session outline <row> <prefix>` stores a number apart from the title,
and the sidebar builder re-composes that prefix onto the row's label as its last
act — **specifically so that a CLI re-titling itself cannot drop the number.** The
API and the screen agree by construction. Good design, and worth keeping.

⚠ **The live defect is DURABILITY, not rendering: a stored prefix has been
observed to vanish on its own between two reads**, leaving the row unnumbered.
⇒ **Until that is fixed, compose the number into the title as well** — belt and
braces, and the title is the thing the watch defends:

```sh
yggterm server app session rename  "$ROW" "4. topic: what I am actually doing"
yggterm server app session outline "$ROW" 4          # "" clears
```

⛔⛔ **THE LESSON HERE IS NOT ABOUT LABELS, AND I GOT IT WRONG FIRST.** Seeing a
composed label in the API and an unnumbered row in a screenshot, I concluded the
field was lying about what the sidebar renders, and published that. **It was
false** — the two observations were **taken at different MOMENTS**, and the seat
had disappeared in between. One durability bug wearing the costume of two.

⇒ **AN API READ TAKEN AT A DIFFERENT MOMENT FROM THE SCREENSHOT IS NOT A
VERIFICATION OF THE SCREEN.** When you compare an instrument against a display,
**the two samples must be simultaneous, or the difference you find may be time
rather than disagreement.** This generalises well past this one field, and it is
the failure §7's own advice ("read state back") walks you straight into if you
read it back *later*.

### ⛔⛔ A WRITE TO THE ROW TABLE IS NOT VISIBLE TO YOUR VERY NEXT READ — measured 2026-08-13

**`session rename` answers `accepted: true` synchronously. The row table does not carry the new
title on the next call.** Six rows were renamed in one batch; the `rows` read issued immediately
afterwards showed **all six unchanged**. A second read moments later showed **all six correct.
Nothing was re-sent in between.**

⇒ **AN IMMEDIATE READ-BACK IS A FALSE NEGATIVE, and it is the most dangerous kind, because §7 has
just finished telling you to read state back.** Reading back is still right; reading back *in the
same breath* measures the lag, not the write. **Put a round trip between the write and the check**
— and if the check fails, **read again before you write again.**

**⛔ AND THE COROLLARY IS A REAL CORRUPTION, NOT JUST A WASTED CALL: NEVER ISSUE `rename` AND
`outline` BACK TO BACK.** `outline` composes the prefix onto the title **it can currently see** —
which, inside the lag window, is the OLD one — and writes the composed string back into
`session_title`. Reproduced live, in exactly two calls:

```
rename  → "ychrome: the vendor-console surface for row 4.2"   accepted: true
outline → 4.2.1                                              (composes onto the STALE title)
rows    → session_title = "4.2.1 Agent unnamed shell"    ⛔ the number is now IN the title,
                                                            and the rename is gone
```

That is the **double-numbering** hazard `ygg-claim.sh` documents in its own comments, arriving by a
route the comments do not name: not from an author writing the seat into the title on purpose, but
from **two correct writes racing each other**. The composed string is now the stored title, so the
sidebar renders `3.4.1 3.4.1 …` the moment the prefix is composed again.

⇒ **rename, read back until it holds, THEN outline.** `ygg-claim.sh` gets this right by accident —
it re-asserts the title on a watch loop, which absorbs the lag. **Hand-driving the two verbs does
not.**

### ⛔⛔ `sessions sort --dry-run` REPORTS `changed:false` ON A LIST THAT IS NOT SORTED

**Measured 2026-08-13, twice, and it is a FALSE NEGATIVE in the one instrument that exists to
answer "is the sidebar order right?".**

An orchestrator's cluster row sat visibly out of place — seat `3.2` rendered *above* `3.0` and
`3.1`. Sampled **simultaneously** (both verbs launched in one shell, per the warning above about
comparing reads taken at different moments):

| instrument | answer |
|---|---|
| `server app rows` | `2.0 2.1` **`3.2`** `3.0 3.1 3.3 3.4` — wrong, and it matches the screen |
| `sessions sort --dry-run` | `changed: false`, and a `rendered_order` listing `3.2` **in the right place** |
| `sessions sort` (apply) | **`changed: true`** — and the row moved. Order correct afterwards |

⇒ **The dry run reports the order it WOULD PRODUCE as though it were the order that EXISTS**, so it
can answer "already sorted" about a sidebar it is looking straight at being wrong. This is §7's law
in its purest form — *the verb answered about the request, not the effect* — and the verb's own
help text closes the trap: *"sorting a sorted list reports `changed:false` — which is the success
case, not a no-op to chase."* **That sentence is true of a real sort and false of the dry run**, and
it instructs you to stop looking at precisely the moment you should look harder.

**⇒ THE RULES:**
- ⛔ **Never diagnose row order with `--dry-run`.** For this verb the dry run cannot report a
  disagreement it is itself the source of. **Compare `rows` against the numbering yourself**, or
  just run the apply — it is idempotent and it tells the truth.
- ⭐ **RUN `sessions sort` (the APPLY) AS THE LAST ACT OF ANY SPAWN BATCH.** §1 already says a new
  row lands at the head and *"repair it in the same breath as the spawn — never later, never
  leaving it for the human."* This is the verb that does it, and until 2026-08-13 nothing in this
  skill named it for that job — so every orchestrator re-derived the repair by hand, or skipped it.
  ⚠ **Unless a human has dragged a row themselves** (§1) — a manual placement outranks the numbers,
  and a blind sort will silently undo it.

⚠ **AND THE ORIGINAL DISPLACEMENT IS STILL UNEXPLAINED — say so rather than inventing a cause.**
The create reported `seat.honoured: true` and the row was wrong anyway. The obvious hypothesis —
that an *unnumbered* row sitting above the numbered block shifts the insert index — was **tested
and REFUTED**: with an unnumbered row deliberately parked at the head, a probe created with
`--outline 3.8` landed exactly where it belonged (`seated_after` correct, `live_index` correct).
So the seat arithmetic handles that case. **What moved the row remains unmeasured**, and it belongs
to whoever owns sidebar truth — route it, do not guess it.

### ⛔ CLAIM YOUR ROW AS YOUR FIRST ACT ON A CAMPAIGN — do not wait to be asked

**A session that starts long-lived work owns its own identity.** A row born with
the title its CLI invented, sitting wherever the sidebar dropped it, is the one
row a human cannot find later — and it is always the one running the work.

```sh
.agents/skills/yggterm-agent-fleet/ygg-claim.sh \
    --title "<topic>: <what this session is for>" \
    [--campaign <token>] [--replace <uuid-of-the-row-this-supersedes>]
```

It does the whole chore in one call — derive the seat number from existing rows,
rename, **read the title back**, keep re-asserting it, and (with `--replace`)
retire and reap the predecessor. Do this **at the start**, not at the end.

**Why a verb and not a paragraph.** Every part of this has a trap that has
actually fired, and a session's discipline resets every launch:

- **A rename applied too early is LOST.** The CLI composes its own title when its
  first turn ends and clobbers whatever was set before. One row ended up named
  after a liveness ping, because the ping was the first message it ever received.
  ⇒ the tool re-asserts the title for a while instead of writing it once.
- **Numbering is a decision, not a constant.** Seats are derived: an explicit
  `--number` wins; replacing a row inherits its seat; joining a campaign that
  already has rows takes the next SUB-seat (`5.1`, `5.2`); a row that already has
  a number **keeps it**, so re-running the claim is a no-op; only an unnumbered
  row takes the next free top-level seat.
- **A new row lands at the HEAD of the sidebar**, so every launch leaves the
  outline wrong until someone repairs it. Repair it in the same breath as the
  spawn — never "later", never leaving it for the human.
- ⚠ **If a human has dragged a row themselves, that is THEIR placement.** Read
  the order back and leave it alone; a manual drag outranks a stored outline.

---

## 2. Know your own context budget

An agent that runs out of context mid-campaign loses everything it had not
written down.

### ⛔⛔ THE AUTOMATIC PATH — a manual budget check is one an agent under load skips

**Measured 2026-08-10, and it is the only fleet session ever to hit the wall.**
practice row `8` (`569e15eb`) ran a relay 8.5 h on `opus[1m]` with
`autoCompactEnabled:false`, reached **976,493 tokens**, and from `00:00:37` every
turn returned **"Prompt is too long"** — unrecoverable, no compaction armed. It
had this section available and never ran it, because the check below costs a
round trip, can stall your own loop, and must be **remembered**.

⇒ **`~/.claude/hooks/context-relay-gauge.py`, wired as a `UserPromptSubmit` hook,
fires on EVERY prompt** — including the booter's, the caller with no judgement of
its own. Silent under 55%; **NOTICE 55%** (open no new plane of work), **LAND 70%**
(commit → update door + queue → spawn successor → unsubscribe → retire),
**CRITICAL 85%**. On demand: `python3 ~/.claude/hooks/context-relay-gauge.py --report`.
⭐ It publishes `~/.claude/context-gauge/<session_id>.json`
(`pct`/`used`/`window`/`verdict`/`dead`) — **a watchdog cannot see a token count**,
which is why `ygg-babysit` used to infer liveness from file mtimes, and why a corpse
answering in 5 ms read to it as `WORKING`.
⇒ ✅ **CONSUMED, 2026-08-10.** `ygg-babysit.classify()` now reads that file BEFORE the
transcript and returns a terminal **`CONTEXT_DEAD`**; `ygg-booter` escalates ONCE with
*"unrecoverable, relay it"* and **unsubscribes** instead of kicking a grave. Its
anti-flap counter also stopped trusting bytes: `progress_marks()` counts only turns
that used a tool or spent output tokens, because a refused turn GROWS the file and was
resetting the counter every tick (proven on the incident transcript — appending its 9
real refusals moves 5,640 bytes and 0 marks).
⚠ A missing or stale gauge is **no information, never "healthy"** — the file is only as
fresh as that row's last prompt, so classification falls through to the transcript.

⚠ §8 step 3 already *forbade* dying this way. **A prohibition with no measurement
is unenforceable** — that is why the hook exists and this section no longer relies
on you choosing to look.

### ⛔⛔ THE CONTEXT RITUAL — you GROSSLY underestimate your own context; VERIFY before you relay

**Owner-directed 2026-08-20, after the opposite failure of the one above: the gauge hook
divided a Fable session's 186k tokens by a 200k window it did not have — the true window was
1M and `/context` said 19% — and the session relayed TWICE off the false wall, spinning
successors the owner had to kill by hand.** The wall-death above cost one session; the false
wall cost successor churn, seat confusion, and owner time — both ends of the same instrument
failing in opposite directions.

⇒ **THE RITUAL, mandatory before ANY relay/handover decision that cites context:**
1. **Verify with the CLI's own instrument, never the gauge alone.** In Claude Code that is
   `/context` — and if a `/context` result is already in your transcript this session, you
   can simply READ it. For every other CLI, **read the PTY frame** (`server terminal screen`,
   `gate-screen`) where the CLI paints its own usage — the frame read is the safest
   cross-CLI method because it types nothing.
2. **Probing ANOTHER row's context costs a live composer.** If you must type a context
   command into a row (only when the frame carries no usage), the sequence is: **save any
   draft in its composer first, send the context command, read the result, then restore the
   draft** — a handover must lose no typed text, and neither may a probe (§8(f)).
3. **A gauge and the instrument disagreeing means the GAUGE is wrong** — fix it in the same
   session (stale-doc law; an instrument that lies is a stale doc with a trigger). The
   window table lives in `~/.claude/hooks/context-relay-gauge.py`; Fable is natively 1M with
   no `[1m]` suffix anywhere, which is exactly the case the old table missed.
4. **When in doubt, assume you have MORE context than you feel.** The documented bias runs
   one way: sessions under-estimate their remaining window and land too early, shredding
   continuity into micro-relays. The wall is real but it is measured, not felt.

### The interactive check, for when you want the CLI's own breakdown

You cannot read your own token count directly — but you can ask
your own session, because a row can be sent input like any other:

```sh
printf '/context' | yggterm server app terminal submit "$ROW" --stdin
sleep 1
printf 'continue' | yggterm server app terminal submit "$ROW" --stdin   # ⛔ not optional
```

The readout arrives as your next turn. **Do this at natural checkpoints on any
long run, not when you feel full** — by then the cheap remedy (write the
handover now) may no longer fit.

### ⛔ A SELF-DIRECTED SLASH COMMAND CAN STALL YOUR OWN LOOP

**Chase every slash command you send to your OWN row with a plain `continue`, a
second later.** A slash command is handled by the CLI's front end, not by the
model loop, and whether the loop is re-entered afterwards is the CLI's choice —
one none of them documents. When it is not, the readout sits on screen, the turn
never resumes, and the row looks exactly like an agent thinking. Unattended that
costs hours, and the follow-up prompt is the only control we hold from outside.

Messaging ANOTHER row (§4) is a different case: that session's loop is already
turning, and your message lands at its turn boundary.

⭐ **Across CLIs, assume the market leader's spellings.** `/context`, `/cost`,
`/status` are the right first guess in any of them, because they copy each
other. The OUTPUT shapes differ, so parse loosely and never key on an exact
line. ⚠ **Write each nuance back into this section as you find it** — a command
that does not exist here, one that means something else there, a different stall
behaviour. A nuance left in a transcript was never learned.

⚖ **Budget rule of thumb:** decide what you will still be able to finish, and
write the handover BEFORE you spend the context on the work. A finished task
with no handover is worth less than an unfinished one with a good one.

---

## 3. ⛔ Spawn a delegate — and PROVE it got your brief

This is the most dangerous verb in the skill, because its failure mode is
silence. **A delegate that never received its brief looks exactly like one that
is working.**

### ⛔⛔ TWO ARGUMENTS ARE BOTH CALLED "HOST" AND THEY MEAN DIFFERENT MACHINES

**They sit two lines apart in every brief, and getting them the same way round is
the default mistake.** Reported 2026-08-13 by a sibling campaign after it spawned
a duplicate worker onto a machine with no checkout of the repo:

| argument | which machine | why |
|---|---|---|
| `ygg-claim.sh --host <gui>` | **the GUI host** | app control is served by the GUI PROCESS and answers **nowhere else** |
| `terminal new --machine-key <target>` | **where the WORK lives** | the row's PTY runs there and needs the repo, the toolchain, the checkout |

⇒ On a fleet whose GUI runs on one machine and whose repos live on another,
**these are different values in the same brief and both are correct.**

⛔⛔ **AND A `--cwd` THAT DOES NOT EXIST ON THE TARGET DOES NOT FAIL — IT FALLS
BACK A DIRECTORY.** The row is created, the agent starts, and its transcript is
filed under the **parent's** slug, so even the transcript check in step 4 looks at
the wrong path. The spawn reports success throughout.

⭐ **One line prevents it, and it is not optional when the target is not this
machine:**

```sh
ssh <target> test -d <cwd> || { echo "no <cwd> on <target> — refusing"; exit 1; }
```

⛔ **Two further pre-spawn checks, from the same incident:** before spawning a
successor, check whether a **live row already holds that seat** — by transcript
**mtime**, not by the row listing, which has omitted live rows — and ⛔ **never
re-spawn on a failed `ygg-claim` read-back.** Absence from the listing is not
proof the claim failed; `ygg-claim.sh` now says so in as many words, because a
row that believed its first spawn had not taken performed its handover **twice**.

### The four steps. Do not collapse them.

```sh
# ⛔ --machine-key is WHERE THE WORK LIVES, and it is not necessarily the GUI
#    host you passed to ygg-claim.sh two lines ago. See the table above.
# 1. CREATE with no prompt. A prompt passed at create is delivered by a path
#    that has silently dropped briefs; see the caveat below.
ROW=$(yggterm server app terminal new \
        --kind claude-code --machine-key <target-where-the-repo-is> --cwd <dir> \
        --title '<topic>: <what it is for>' \
        --purpose '<one line a human can act on>' \
        --outline <n> --no-activate \
        --model <model-id> --permission-mode bypass \
      | python3 -c "import json,sys; print(json.load(sys.stdin)['data']['session_path'])")

# 2. WAIT until the row is genuinely reading input. A cold agent-CLI row needs
#    SECONDS, not milliseconds, and the composer is drawn well before the input
#    loop is live.
yggterm server app terminal input-check "$ROW" --check-timeout-ms 20000
#    → want consuming_input:true. `wedged:true` is a POSITIVE claim, not silence.

# 3. SUBMIT the brief and READ the answer's `submitted` field.
yggterm server app terminal submit "$ROW" --stdin < brief.md
#    → {"submitted": true, "waited_ms": N}

# 4. ⛔ VERIFY BY TRANSCRIPT CONTENT — but not for the first ~20 s (see step 5).
grep -q 'PUT-A-DISTINCTIVE-TOKEN-IN-YOUR-BRIEF' \
     ~/.claude/projects/<cwd-slug>/<uuid>.jsonl

# 5. ⛔⛔ READ THE SCREEN. Steps 2-4 are ALL downstream of the write, and a pty
#    accepts bytes whether or not anything is consuming them — so a QUEUED brief
#    and a DELIVERED one are byte-identical from here. This is the only step that
#    can tell them apart.
yggterm-headless server screen "cc-runtime://<uuid>" --state-only
#    → `ready` (it is at its composer) · `startup_gate` (a modal is holding it,
#      and no amount of waiting will clear it) · `question_picker` /
#      `plan_limit_choice` (a PERSON is being asked; ⛔ type nothing) ·
#      `working` · `limit_wait`. Add `--state` for the remedy and the
#      prohibition, or drop the flag entirely to print the screen itself.
```

### Why step 4 is not optional

**A transcript FILE exists the moment the CLI starts.** It tells you a process is
running; it tells you nothing about what was delivered into it.

⛔⛔ **AND IT IS NOT THERE IMMEDIATELY — MEASURED 2026-08-21, correcting what this
file said before.** A brief submitted at 13:33:58 (`submitted:true`, 82 bytes,
`consuming_input:true`) produced its transcript at **13:34:12.5 — 14.5 s later**,
and the project directory did not exist at all until then. So a grep run straight
after the submit returns a FALSE NEGATIVE, and "absent means the brief was
dropped" is only true after the file has had time to appear. ⇒ Give it ~20 s
before you believe an absence, and use step 5's screen read for the answer you
actually wanted, which is whether anything is RUNNING. A launch that
dropped its entire brief still produced a 28 KB transcript, and a reply field
saying the launch was applied — because the ROW was born exactly as asked,
holding nothing.

So put a token in every brief (`ACK-<something-unique>`) and grep for it. Thirty
seconds, and it is the difference between "a session is running" and "a session
is running MY errand".

### ⭐ MEASURED AGAIN 2026-08-08 — and the discriminator is the TRANSCRIPT FILE, not the field

Two delegates spawned back to back, identical four-step sequence, both rows answering
`consuming_input: true` before the submit. Both submits answered:

```
{"submitted": true, "waited_ms": 246}     <- row A
{"submitted": true, "waited_ms": 249}     <- row B
```

**Row A produced an 86 KB transcript within a minute. Row B produced NO TRANSCRIPT AT ALL**, and
still answered `consuming_input: true` when re-probed two minutes later — i.e. a healthy, ready,
empty row. A second identical submit landed immediately (111 KB, ACK token × 7).

⇒ **The drop is INTERMITTENT, which is worse than a consistent one**, because a spawn that worked
last time is not evidence for this time. `submitted: true` describes the write, never the delivery.

**The cheapest honest check, and it needs no token to run:** does
`~/.claude/projects/<cwd-slug>/<row-uuid>.jsonl` EXIST yet? An agent-CLI row that took a brief
starts writing within seconds. **File absent after ~60 s = the brief was dropped: re-submit.**
Then grep the ACK token to confirm it is *your* brief and not a leftover.
⚠ Do not read "no transcript" as "still starting" past a minute, and do not read a transcript that
exists as proof of delivery — that is what the token is for.

### ⚠ The readiness probe is not free

yggterm proves a CLI is reading by WRITING a probe string and watching it echo.
That is sound only against a program already reading. Against one that is not,
the bytes are **queued, not discarded** — a PTY buffers — and they arrive later,
in the composer, as the delegate's opening message. So:

- **Never treat "not ready" as "nothing happened".**
- **Never spawn and walk away.** Which is what the next section is for.

### ⛔⛔ A MODAL IS NAVIGABLE, NOT A BLOCKER — you have a keyboard, use it

**Every agent CLI has a startup quirk that can hold a fresh row before its
composer** — a first-run trust prompt, a pending self-update, a theme picker, a
login chooser, a "resume or start new" list. **A row you can send keys to is a
row you can drive.** ⛔ Respawning it somewhere friendlier, or handing the task
back, is the failure — it abandons the correct cwd to dodge one keystroke.

**The instance that produced this rule.** Two delegates were spawned into a
directory that had never hosted an agent-CLI session **on that host**. Both sat
at the CLI's first-run trust prompt (*"Is this a project you created or one you
trust?"*), which a skip-permissions flag does **not** skip, because it is a
workspace-trust gate and not a permission gate. Both were killed and respawned
elsewhere before anyone read the screen. **One `\r` was the whole fix.**

**The three tells, and the first one names the answer out loud:**

1. `input-check` answers `consuming_input:false` with reason *"no agent composer
   row appeared … the row is **in a menu** …"*. ⇒ **that reason is the diagnosis,
   not a shrug.** Read it; do not read it as silence.
2. **No transcript file ever appears** — the CLI has taken no input, so there is
   nothing to write. (§ above: absent transcript = the brief did not land.)
3. `submit` answers `submitted:false`. It refused, correctly.

**Read the screen of a row you are NOT looking at** — no activation, so it never
disturbs the user's viewport, and no screenshot staleness:

```sh
yggterm server snapshot | python3 -c "
import json,sys
for s in json.load(sys.stdin)['live_sessions']:
    if s['id'].startswith('<uuid-prefix>'):
        print(''.join(s.get('terminal_lines') or []))"
```

⚠ **`terminal probe-scroll` is NOT a screen read** — it answers
`{accepted, reason, session_path}` only. It scrolls; it does not report content.
`live_sessions[].terminal_lines` is the instrument for "what is on that row".

**Then drive it.** Confirm the highlighted option from the raw lines before
pressing anything — the marker is `❯` — and send a **lone** `\r` (Enter is its
own write; see the multi-line refusal above):

```sh
yggterm server app terminal send "$ROW" --data $'\r'          # confirm default
yggterm server app terminal send "$ROW" --data $'\x1b[B\r'    # down one, then confirm
```

⛔ **Confirm the menu is the menu you think it is before arrowing blind** — the
same keystroke into a composer submits garbage, and into a pending self-update
prompt confirms the update.

⭐ **And it usually self-heals the class:** a first-run gate answered once is
normally persisted per-directory by the CLI, so every later row spawned into the
same cwd walks straight to its composer. Verified in the instance above — the
second row never saw the prompt. ⇒ **answering it is strictly better than
avoiding it**, because avoiding it leaves the wall standing for the next agent.

⛔ **A related self-inflicted wound, while reaping:** `pkill -f <uuid>` matches
**your own shell**, because the uuid is in your command line — it kills the
session doing the reaping. Collect pids first, exclude `$$`, then signal them.
Identify; never pattern-match.

### ⭐⭐ TWO TRANSPORTS, TWO QUESTIONS — `submitted: false` does NOT mean unreachable

**There are two ways to reach another session and they fail differently.** Reading
one's refusal as the other's failure wasted an afternoon.

| | `terminal submit` (the ROW plane) | `SendMessage` (the PEER plane) |
|---|---|---|
| needs | the target **at a composer** | nothing — it enqueues |
| when the row is mid-output | **refuses**, `submitted: false`, after its full timeout | delivers; drains at the receiver's next tool round |
| addresses | a row path / session | a peer name, or the exact `from` of a message you received |
| good for | a brief to a row you are seating | anything, any time |

⛔ **`error: null` is not the delivery field. `submitted` is.** A refusal names its
reason — *"no agent composer row appeared within the timeout — the row is
mid-output, in a menu, or is not an agent CLI, so input readiness is unanswerable
rather than false"*. That is the verb being **correct**, not broken.

⇒ **When `submit` refuses, the peer plane still delivers.** These are two
instruments answering different questions, not a bug and a workaround. Reading
`submitted: false` as "that row is unreachable" produced, in one afternoon: a
false report that the orchestrator's inbound channel was dropping messages,
relayed twice as fact, and a fallback built for a defect that did not exist.

⛔⛔ **AND THE SOCKET LOOKUP IS THE THIRD FALSE INSTRUMENT.** A peer socket is named
for the **CC process**, not for the shell a tool call runs in. So the obvious check —
`ls /run/user/$UID/cc-socks/$$.sock` — asks about the **shell** and answers "absent"
for **every row on the machine, always**. It cannot succeed. ⭐ The listing that works,
and it names corpses too:

```sh
ls /run/user/$UID/cc-socks/ | sed 's/\.sock//' | while read p; do
  printf '%-9s %s\n' "$p" "$(ps -o comm= -p "$p" 2>/dev/null || echo '(dead)')"
done
```

⚠ **THREE AGREEING INSTRUMENTS CAN SHARE AN ERROR WHEN THEY SHARE A PREMISE.** In one
afternoon: `error: null` read as delivery, `submitted: false` read as unreachable, and
a socket lookup keyed on the wrong pid — each independently "confirming" that an
orchestrator's inbound channel was dead. It was never dead. **Corroboration between
instruments of the same family is not corroboration**; it is the same misreading three
times, and it feels like evidence, which is what makes it dangerous.

⚠ **The PTY write is the crude third option** — it lands regardless, but it types
into the row's terminal and interleaves with whatever it is doing. Use it when a
row must be woken, not as the routine way to say something.

⛔ **AND A RELAYED CLAIM IS A CLAIM YOU ARE MAKING.** The false report above passed
through a session that forwarded it without running a single `submit` of its own —
while applying *read the state back* rigorously to its own work all day. **Verify
before you forward**, especially when the claim is about the infrastructure
everyone depends on: an infrastructure defect asserted by a trusted peer will be
believed and acted on immediately, which is exactly why it must be tested first.

### ⛔⛔ A FINISHED DELEGATE AND A STALLED ONE LOOK IDENTICAL — run `ygg-babysit.py`

**Reported 2026-08-08, and it halted a live pipeline:** *"I am seeing that they both
stopped. So this is a monitor/relay system bug and should be resolved. These yggterm fleet kinks
should be seen, 'dreamt' of it and then auto-resolved whenever encountered by any agent."*

**Measured that hour.** Two delegates spawned together. One had **landed its entire subset**; the
other had **ended its turn after acknowledging the brief** and sat idle for **54 minutes**. From
`server app rows` the two are indistinguishable — both are "alive, nothing happening". The
orchestrator cannot tell success from a halted pipeline, and the stall is found only when a human
notices.

⇒ **`rows` reports EXISTENCE, not LIVENESS**, and an agent-CLI sits at its prompt forever. This is
the same family as everything else in §7: *silence is the most dangerous value a status can take.*

```sh
# after spawning, record who you spawned:
printf '%s\n' "$ROW_A" "$ROW_B" > ~/.yggterm/relay/spawned-by-$YGGTERM_SESSION_UUID.txt

.agents/skills/yggterm-agent-fleet/ygg-babysit.py --spawned-by <my-uuid>            # one pass
.agents/skills/yggterm-agent-fleet/ygg-babysit.py --spawned-by <my-uuid> --watch 1800   # keep watch
.agents/skills/yggterm-agent-fleet/ygg-babysit.py --row <path> --dry-run            # classify only
```

**What it does, and the asymmetry that decides the design:** a spurious `continue` to a finished
row costs **one cheap turn**; a missed stall costs **the pipeline until a human looks**. ⇒ when
idle is ambiguous, NUDGE — bounded to `MAX_NUDGES=2`, then escalate and stop, so a finished row is
never poked forever.

| state | how it is decided | action |
|---|---|---|
| `WORKING` | last real turn is a `tool_use`/`tool_result`, fresh | none |
| `JUST_ENDED` | turn ended < 4 min ago | none — let it be |
| `IDLE` | turn ENDED and untouched ≥ 4 min | ⭐ **one `continue`** |
| ⛔ `STUCK` | MID-TURN and untouched ≥ 15 min | **escalate, never nudge** — typing races its own input |
| ⛔ `NO_TRANSCRIPT` | the JSONL never appeared | the brief was **dropped**: re-submit it |

⛔ **It never nudges a mid-turn row.** And it classifies from the **last real turn** — system and
hook rows are not turns, and treating the file's final line as the turn returns UNKNOWN for nearly
every session.

✅ **All five branches are now proven live**, including `STUCK` — which fired within minutes of
being written, on the **yggterm row itself**: mid-turn and untouched for 54 minutes, which is
exactly why a `terminal submit` to it answered `{"submitted": false}` after a 30-second wait.
⇒ **A wedged row REFUSES a submit, and that refusal is the one honest signal in the relay.** A
nudge's delivery is otherwise confirmed by **transcript GROWTH on the next pass**, never by
`submitted:true`.

⛔⛔ **AND THE ONE THE OWNER CAUGHT — `--session` WITH THE WRONG STRING MAKES AN INERT CARD.**
His report: *"Clicking these delegate notification does not transfer me to the required attention
session."* Two separate mistakes, both mine, both worth more than the fix:

1. **`$YGGTERM_SESSION_ID` is `cc-runtime://<uuid>`; the row is `remote-cc://<host>/<uuid>`.** Same
   uuid, different string. Pass the former to `notify --session` and the card renders, looks
   correct, and **does nothing when clicked.** The verb's own help warns about this — which is
   exactly the shape this skill exists to kill, because *a warning in prose is something an agent
   has to remember*. ⇒ `ygg-babysit` now RESOLVES any identifier to a real row path by matching its
   UUID against `server app rows`, so passing the wrong one is impossible rather than discouraged.
2. ⭐ **The card must point at the row that NEEDS ATTENTION, not at the orchestrator that noticed
   it.** I pointed it at myself. That is worse than inert: it works, and takes the human to the
   wrong place. **A notification is an ADDRESS, and the address is where the problem is.**

⛔⛔ **AND A RETIRED ROW READS AS A WEDGED ONE.** On first real use the tool reported the yggterm row
`STUCK` for 54 minutes and I relayed that to the owner. It was a **corpse** — the row had been
retired by its campaign's baton relay, and **a retired row's transcript is frozen mid-turn forever**.
It also explains a `submitted:false` that looked like a busy row refusing input: there was no row.
⇒ **A transcript cannot distinguish KILLED from WEDGED. Only the row list can, so ask it FIRST** —
`GONE` is now checked before any transcript is opened.

⛔⛔ **TWO BUGS CAUGHT BY DOGFOODING IT WITHIN A MINUTE OF WRITING IT** — both generic, both worth
more than the tool:

1. **It searched `~/.claude/projects` on the LOCAL host for every row.** A `local://<uuid>` row runs
   on the GUI host, so its transcript is on *that* machine — and the tool reported `NO_TRANSCRIPT`
   and announced *"the brief was dropped, re-submit it"* about a perfectly healthy session on
   another box. ⇒ **"I looked in the wrong place" and "it is not there" are different facts**, and
   this is the same *cause-not-derived-from-a-measurement* defect the whole fleet keeps re-finding.
   Fixed: the row path names its host (`remote-cc://<host>/…`, `local://…` = the GUI host), the
   probe runs WHERE THE TRANSCRIPT LIVES, and an unreadable host reports **`UNREACHABLE` /
   `CANNOT-SEE`** — never a verdict about the row.
2. ⚠ **`ssh host python3 -c <multi-line-script> <arg>` arrives MANGLED.** `subprocess` passes argv
   unquoted and **ssh joins argv into ONE remote shell command string**, which the remote shell then
   re-parses. The failure looks like a dead host. ⇒ **feed the script on STDIN** (`ssh host
   "python3 - '<path>'"`, `input=SCRIPT`), where no shell can touch it.

⭐ **A defect caught in the tool itself, worth repeating because it is generic:** the first version
incremented the nudge counter **under `--dry-run`**, so classifying a row twice burned its whole
budget without sending anything and the next real pass would have escalated instead of nudged.
**A dry run that mutates state is not a dry run** — an instrument whose observation changes what it
observes.

### ⛔⛔ ARM THE BOOTER — a stalled session CANNOT restart itself

**Recorded 2026-08-09, said while he was hand-booting a stalled relay row:**
*"I have seen you stall sometimes, so arm a booter in a fleet. A booter is a tool
that monitors any session that has subscribed to it, to kick it and say 'continue,
the booter booted'. Sometimes you may feel that the work is done so you need to
unsubscribe from the booter."*

```sh
.agents/skills/yggterm-agent-fleet/ygg-booter.py subscribe --campaign yggterm
.agents/skills/yggterm-agent-fleet/ygg-booter.py status      # is a watcher alive?
.agents/skills/yggterm-agent-fleet/ygg-booter.py defer --secs 2700 --note "cargo test"
.agents/skills/yggterm-agent-fleet/ygg-booter.py defer --clear
.agents/skills/yggterm-agent-fleet/ygg-booter.py unsubscribe # ⛔ when the work is DONE
```

#### ⛔⛔ BEFORE A LONG WAIT, `defer` — THE WATCHER CANNOT SEE THAT YOU ARE BUSY

**Recorded 2026-08-09:** *"A waiting session on a long task should ask the
booter that for this time use a custom time (less than 55 mins) to boot."*

⭐ **Why the session must ask, rather than the watcher work it out: a session
waiting on a 40-minute build and a session that has genuinely stalled are
IDENTICAL from outside** — turn ended, transcript not growing. Only the session
knows it just started something long. The default window is deliberately short
(**420 s**) so a real stall is caught fast; **you** widen it for one wait and it
closes again by itself.

⛔ **The ceiling is a BILL, not a preference.** The plan's prompt cache stays hot
for ~1 hour; a session that does nothing for an hour resumes against a COLD cache
and re-reads a large campaign context at full price. So `--secs` is clamped to
**3000 s (50 min)** — never refused, because refusing would drop you back to 420 s,
the opposite of what you asked for. ⚠ The number that must stay under the hour is
**worst-case delivery, not the setting**: the watcher only looks every 300 s, so
50 + 5 = 55 min worst case, and 5 min of real margin. Anyone retuning this must
keep `MAX_BOOT_AFTER_SECS + interval` well under 3600.

**It expires on its own, two ways** — after the boot it was protecting fires, and
at a wall-clock deadline. A session that asked for 45 minutes and then died must
not leave that window open for whoever inherits the row.

⭐ **The one case needing no `defer`: sub-agents or workflows running INSIDE your
session.** That is mid-turn, `classify` reports `STUCK`, and the booter escalates
rather than boots — *a boot there races the agent's own input*. Such a session
also keeps its own cache warm by working. ⚠ In relay mode sub-agents are
discouraged anyway, so if this arm fires often, something is spawning agents that
should not be.

⚠ Read `win=` in the log to see which window a row is being judged against —
`win=7m` is the default, `win=50m/cargo test` is a live deferral. Without it,
*"why was it not booted at 8 minutes"* costs a code read to answer.

⛔⛔ **`ygg-claim.sh` does NOT arm the booter for you — and must not.**
recorded 2026-08-10: *"When there is no relay mode the booter should not
self arm. You should not be booted."* It had fired on him inside a session he
opened with *"NOT like a relay. All agents should be contained in the session"*:
the row was claimed, so the row self-armed, and a machine woke the session he had
just ruled that out for.

⚠ **The bad inference, named so nobody re-derives it: claiming a row is not
evidence that a session is unattended.** The old rationale ("claiming a row is
the moment a session becomes long-running work") conflated **long-running** with
**unattended**. A session with a human in it is long-running too, and it is
precisely the one that must never be woken by a robot. His scope was *"in a
fleet"* — relay and delegate work — and the tool widened it to every claim.

⇒ **Arm it where unattendedness is KNOWN, never where a row is merely claimed:**

| case | who arms it |
|---|---|
| a **delegate** you spawn | **you do, explicitly, at spawn** — `ygg-booter.py subscribe --row "$ROW" …` (§9). This is the path that matters and it is unchanged |
| a **relay** session | itself: `ygg-claim.sh --booter`, or `YGG_BOOTER=1` |
| a session **a human is talking to** | nothing. Leave it alone |

`--no-booter` is still accepted and ignored, so older call sites keep working.
⛔ And when you *have* armed yourself, unsubscribing is **your** job the moment
the work is done: `ygg-booter.py unsubscribe --row <path>` (note `--row` — the
verb takes a flag, not a positional, and rejects the bare path).

**The one structural fact, and it decides the whole design: anything that runs
INSIDE the session is dead in exactly the case that matters**, because the stall
IS the turn ending early. A wakeup you schedule, a loop you drive, a check at the
end of your own turn — none of them fire. ⇒ the watcher is a DETACHED process
that outlives its subscribers, and subscribing is something done TO it.

| | `ygg-babysit.py` | `ygg-booter.py` |
|---|---|---|
| watches | rows the ORCHESTRATOR spawned | rows that SUBSCRIBED themselves |
| lives | one run | until unsubscribed |
| ends by | the orchestrator finishing | ⭐ the subscriber's own `unsubscribe` |

⛔ **The classifier is NOT duplicated** — the booter imports babysit's, so "is
this row working, idle, stuck or gone" has one owner. Both therefore inherit:
ask the ROW LIST before the transcript (a retired row's transcript is frozen
mid-turn and reads as a live wedge), never type into a MID-TURN row, and
*"I could not look"* is never *"it is not there"*.

⛔⛔ **TWO DELIVERY BUGS MEASURED 2026-08-09 — the second one had been live in
`ygg-babysit.py` since the day it was written:**

1. **`"submitted" in stdout` IS TRUE FOR `"submitted": false`.** The verb reported
   an honest failure; the substring test read it as success. **Every nudge
   babysit ever logged as sent was logged identically whether it landed or not.**
   ⇒ read a field's VALUE, never its presence. This is the §7 law arriving inside
   the tool written to enforce §7.
2. **`terminal submit` drives the GUI's MOUNTED terminal host.** A row with
   nothing mounted waits out its 30 s deadline and answers `submitted:false` —
   and rows nobody is looking at are *exactly* what a watchdog exists for.
   `server terminal write` addresses the PTY, the layer that exists whether or
   not anything is mounted. Measured on one row in one minute: submit
   `submitted:false`, PTY write delivered. ⇒ **try the composer, fall back to the
   PTY**, and log WHICH door delivered.

⚠ The boot text is `continue, the booter booted` — deliberately recognisable in a
transcript, so a session (and a human reading back) can tell *a machine woke me*
from *a person asked for something*.

### Arm a dead-man check on yourself

After spawning, schedule your own wake-up a few minutes out. If you wake and the
delegate has not produced real work, the spawn failed — fix it and respawn,
rather than discovering it hours later:

```sh
yggterm server app notify 'delegate check' 'verify <topic> took its brief' \
    --in 5m --session "$ROW"
```

⛔ Untargeted `notify` lands on the human's own screen. Pass `--session`, and
pass `--client`/`--pid` when the notification is for your own bookkeeping rather
than for them.

### Row hygiene is part of spawning

Always pass `--purpose`. Pass `--ephemeral` with either
`--ephemeral-owner-pid <pid>` or `--ephemeral-idle-ttl-secs <n>` for a probe row
— there is deliberately no default owner, because under `bash -c` or over ssh
the recorded parent dies within milliseconds and would reap the row instantly.
**Remove a row you created when its job is done**, and prove it: the row absent
from `server app rows`, and no surviving process.

---

### ⛔⛔ WHO A NOTIFICATION IS ADDRESSED TO — caught twice in one hour

`notify --session <row>` makes the card **clickable through to that row**. So the `--session` is
not bookkeeping, it is a **destination**: it decides where the human lands when they act on it.

> **The rule: a notification is an ADDRESS, and the address is WHERE THE ATTENTION IS NEEDED —
> never the row that happens to be reporting.**

Both halves of this were got wrong on 2026-08-08, in opposite directions:

1. An orchestrator escalating *about a delegate* addressed the card **to itself**. It works, and it
   takes the human to the wrong place — **worse than inert, because it looks like it functioned.**
2. A delegate announcing **its own landing** was told by its brief to notify the orchestrator's
   row. The owner clicked it and landed in the orchestrator's session, not on the work. His
   question — *"Was this intended?"* — is the right test, and the honest answer was **no, the brief
   was wrong**.

⇒ **A delegate reporting on ITSELF addresses its OWN row.** A card about row X carries row X.

⚠ **And do not use a human-facing toast to talk to another agent.** They are different channels:

| to reach | use | why |
|---|---|---|
| the **human**, about row X | `notify --session <row X>` | a clickable card that lands on the work |
| **another agent** | `terminal submit <their row> --stdin` | agent-to-agent; costs the human nothing |
| **your own bookkeeping** | `notify --pid/--client` | ⛔ otherwise it lands on *his* screen |

⛔ And the identifier must be a **row path**, read back from `server app rows` — measured on both
sides by the delegate that hit it:

```
$YGGTERM_SESSION_ID  =  cc-runtime://c00a69a6-…        <- INERT
rows.full_path       =  remote-cc://dev/c00a69a6-…     <- the address
```

⭐ **They differ by SCHEME *and* by a HOST SEGMENT that only the row path carries.** So this cannot
be repaired by swapping the scheme in a string — the host is information the environment variable
does not contain, and an agent composing the address from what it knows about itself **cannot get
there by reasoning.** It must look the row up. `ygg-babysit.py`'s `resolve_row_path()` does exactly
this, so the mistake cannot be made rather than merely being warned about.

⛔⛔ **AND THE HARDER HALF: YOU CAN VERIFY THE ADDRESS, NEVER THE DELIVERY.** `notify` answered
`error: null` for the **misaddressed** send too — it reports that a card was accepted, not that it
points anywhere useful, and certainly not that anyone can act on it. ⇒ *"row 5.2 notified"* was
**true of the wire and false of the intent**. So:

- **Read the address back from `rows` and claim only that.** "Card addressed to <row>, verified
  against the rows API" is honest; "row X notified" is a claim about someone else's attention.
- The only proof of delivery is downstream: the human acts on it, or the row shows the effect.

This is the `§7` law — *verbs report the REQUEST, not the EFFECT* — arriving in the one place where
the request and the effect are separated by a human being.

### ⛔⛔ 3b. CLAIM EVERY SESSION YOU SPAWN, AND SWEEP IT WHEN IT IS DONE

**Recorded 2026-08-10, after finding an unswept session still holding a row's identity:**

> *"Your predecessor or its predecessor started it and did not sweep it once done. I think right
> now you are the only one. If you spawn other related sessions, make sure to claim them otherwise
> it causes ambiguity like you just saw."*

**A session has TWO lifetimes and they end separately: the ROW and the PROCESS.** Everything else
here follows from that, and every trap below has actually fired.

**① CLAIM IT AT SPAWN — do not wait for the delegate to claim itself.** `ygg-claim.sh` is written
for a session to run on itself, and a delegate does run it — *minutes into its first turn*. Until
then the row is unclaimed, unnumbered, and **unsubscribed from the booter**, which is precisely the
window in which a dropped brief (§3) leaves it parked at its composer forever. Measured: of three
delegates spawned together, only one had subscribed itself twenty minutes later. ⇒ the SPAWNER
seats it (`--outline`), titles it, and subscribes it:
```sh
ygg-booter.py subscribe --row "$ROW" --campaign <token> --max-hours 12
```
The delegate's own later claim is then a harmless no-op — a row that already has a seat keeps it.
⚠ `--outline` takes a LITERAL string: shell arithmetic like `5.$(…)` has seated rows at `5.5`
instead of `5.2.5`, one of them colliding with a live row. Pass `5.2.5` literally and read the seat
back.

**①b ⛔⛔ "EVERY SESSION YOU SPAWN" INCLUDES THE ONES THAT ARE NOT AGENTS — libyggterm APP ROWS AND
BARE SHELLS TOO.** Owner-directed 2026-08-13, and it was never written down before, which is why
the sidebar kept filling with rows called **`New Ychrome`**, **`New Yedit`** and **`Agent unnamed
shell`**.

**A row is a row.** When a working row opens a browser surface, a document surface or a helper
shell, that surface **gets a sub-seat under its owner and a title that says what it is for** —
exactly like a delegate:

```sh
# row 4.2 opens the browser surface it is going to drive:
ygg server app session rename  "$APP_ROW" "ychrome: the vendor-console surface for row 4.2"
#  … read the title back (see the write-visibility lag above) …
ygg server app session outline "$APP_ROW" 4.2.1
```

⭐ **The seat is `N.x.y` — the owner's seat plus a sub-number**, so the surface sits under the row
that opened it and a human can see at a glance *whose browser that is*. Same law as everywhere
else: **the number lives in `outline_prefix` ONLY, never in the title.**

**Why this is not cosmetic.** Measured on one sidebar the day this was written: **five rows named
`New Ychrome`, one `New Yedit`, one `Agent unnamed shell`** — and reading their screens showed they
were *not interchangeable at all*: one was a live browser part-way through a multi-step form on a
vendor console, one was a search engine open under a different browser profile, and four were bare
`/bin/bash` at `~` with nothing typed into them.
⇒ **the default label names the LAUNCH VERB, not the JOB**, so
every one of them reads as the same row. The one you must not close and the four that are litter
are visually identical, and the only way to tell them apart is to read seven screens.

⚠ **A default title is worse than no title**, because it looks deliberate. `New Ychrome` claims to
describe the row and does not, and it is *stable across every launch* — so a human scanning the
sidebar cannot even use novelty as a signal.

⇒ **The duty is the spawner's, at spawn, in the same breath** — not the app's, and not "later".
A surface opened and left unnamed is the same defect as a delegate left unclaimed (①), and it fires
far more often because opening a browser does not feel like spawning a session.

### ⛔⛔⛔ AND THE HARD LIMIT ON ALL OF THE ABOVE: NAME YOUR OWN ROWS. NEVER THE OWNER'S.

**Owner-reported 2026-08-13, within the hour, and the harm was immediate and visible on his
screen.** An orchestrator applied ①b to six rows it had not spawned. It read their screens, found
four of them holding a bare shell with nothing typed in, titled them *"abandoned launch, safe to
close"* — **and they were the owner's own browser stack, tagged by him, which he navigates by.**

> **⭐ AN AGENT MAY NAME ITS OWN ROWS AND ITS DELEGATES' ROWS. ROWS THE OWNER CREATED ARE HIS —
> never rename, never re-describe, never "improve".**

⛔ **AND THE TELL WAS AVAILABLE BEFORE ACTING, POINTING THE OTHER WAY: A ROW YOU CANNOT ACCOUNT FOR
IS MORE LIKELY TO BE A HUMAN'S THAN TO BE LITTER.** The orchestrator had exactly that evidence — it
could not attribute six rows to any spawner — and **read it backwards**, concluding *abandoned*
where it should have concluded *not mine*. **Inability to attribute a row is positive evidence that
you did not create it.**

⚠ **Every description it wrote was ACCURATE**, and that is the trap: the shells really were unused
and really were closable. **Accuracy about a row is not authority over it.** Being right about what
a thing is grants nothing about whose it is.

⛔ **AN EMPTY ROW IS THE ONE WHOSE PURPOSE LIVES ENTIRELY OUTSIDE IT.** A row a human is holding
open *for later* is indistinguishable, from the inside, from one nobody wants — the intent is in
the human, not in the PTY. ⇒ **emptiness is never evidence of abandonment.**

**⇒ Before renaming any row, answer: did I spawn this, or did a row I own spawn it?** If you cannot
say yes, **leave it exactly as it is** and, if it genuinely confuses the sidebar, say so to its
owner instead. ⚖ Recovery is only cheap if you captured the old value: **read the current title
back and keep it before you write** — that is what made the restoration above take one call instead
of an unconvergent search, and `detail_label`/profile chips are **not** recoverable from
`row-order-ledger.json` or `removed-rows.json`.

**② ⛔ RETIRING THE ROW DOES NOT KILL THE PROCESS — and a finished delegate does not exit.** An
agent CLI sits at its prompt forever after its last turn. `session remove` can answer
`row_still_listed:false` while the process runs on. Measured 2026-08-10: three delegates had
written their crossings, pushed their commits, **removed their own rows** — and all three `claude`
processes were still resident hours later, holding session ids that no row could address. They are
invisible to `server app rows` by construction, so **the row plane can never show you this.**

**③ ⛔⛔ THE FAILURE THAT COSTS MOST: TWO PROCESSES ON ONE SESSION ID.** Found on the orchestrator's
own row — one `--session-id` from 22:56 and one `--resume` from 23:42, both parented by yggterm row
shells, both live. Symptoms, none of which point at the cause: a background task scheduled ONCE
fired TWICE · an `Edit` failing with *"file has been modified since read"* on a file only you are
editing · **commits appearing in your own voice and row label that you did not make.** Twenty
minutes went into hunting a phantom third party in the tree.
⇒ ⭐ **It is `§7`'s lesson generalised: a session id is an IDENTITY, and two processes holding one
identity make every fact about "that row" ambiguous.** The benign version doubles a read. The row
in question was that evening killing and relaunching an application on a remote guest to repair an
outage **whose root cause was a second instance being launched beside a running one** — a doubled
launch from a doubled agent is that identical fault, and nothing would have refused it.

**④ THE SWEEP — run it before you hand off, and after any GUI restart.** Identify, never count
(`pgrep -c` counts your own shell). ⚠ Find your OWN pid by walking up from `$$` to the first
`claude` process — do not infer it from flags, because killing the wrong one takes down the row the
human is looking at.
```sh
ROWS=$(yggterm server app rows | python3 -c "import json,sys;print(' '.join(
  r['full_path'].split('/')[-1] for r in json.load(sys.stdin)['data']['rows'] if r.get('full_path')))")
for p in $(pgrep -x claude); do
  uuid=$(tr '\0' ' ' </proc/$p/cmdline | grep -oE '[0-9a-f]{8}(-[0-9a-f]{4}){3}-[0-9a-f]{12}' | head -1)
  case " $ROWS " in *" $uuid "*) ;; *) echo "⛔ ORPHAN pid=$p $uuid";; esac
done
```
Before reaping, prove it is DONE and not merely quiet — **a finished delegate and a stalled one are
indistinguishable from the row plane** (§6). Cheap discriminators: its last transcript message is a
completion report, and its work is committed. Then `TERM`, **read `/proc/<pid>` back**, and escalate
to `KILL` only if it survives.

⛔⛔ **DO NOT USE `pgrep -P <pid>` AS THE "IS IT PARKED" TEST. IT ANSWERS A DIFFERENT QUESTION, IN
BOTH DIRECTIONS — measured across 19 live rows, 2026-08-14:**

| observation | what it proves | what it does NOT prove |
|---|---|---|
| `children > 0` | a LOCAL subprocess is running | nothing about the turn — **one row held 4 children and wrote 0 bytes in 25 s** |
| `children == 0` | only that no local tool is running right now | ⛔ **NOT parked.** **5 of 19 rows had 0 children while actively working**, growing 9.5–40.7 KB in 25 s |
| `transcript grew` | ⭐ **WORKING. This is the one sound positive.** | — |
| `transcript flat` | nothing on its own | ⛔ **NOT parked** — a row compiling, or waiting on the model, writes nothing for minutes |
| state `S` / `do_epoll_wait` | nothing | it is the resting state of *both* a thinking row and a dead one |

⇒ **A row that is THINKING, or waiting on the model API, has no children and sits in `S` — byte for
byte the signature a corpse presents.** The rung was being read as an if-and-only-if, and it never
was one: it is a *sufficient* sign of work, never a *necessary* one.

⭐ **THE SOUND FORM.** Working is cheap to prove and parked is expensive, so never infer parked from
a single absence. **Parked requires ALL of: the last transcript record is a COMPLETED assistant turn
(not mid `tool_use`), AND no growth across a generous window, AND no children.** Any one of those
alone is a guess, and the cost of guessing wrong is a `continue` typed into a live row mid-task —
the failure class this whole skill exists to prevent.

⚠ **AND THE PART THAT IS NOT ABOUT THE RUNG.** A brief that says a lane is "idle-capable" because an
earlier session read this same unsound proxy will AGREE with your measurement, and the agreement is
worth nothing — **it is one method run twice, not two methods corroborating.** Ask what the brief's
claim was derived FROM before you let it raise your confidence.

⚖ **Whose job:** the session that SPAWNED it. Not the delegate — it cannot reap itself after its
last turn — and not the next human to notice.

## 3c. ygg-ci — the fleet SINGLE-BUILD plane (like booter/monitor)

`ygg-ci.py` is the detached build watcher, same shape as `ygg-booter.py` and
`ygg-monitor.py`: a subscription lives outside the session that asked for it,
and a timer on `dev` wakes it without burning a core.

**Why not `cargo build` in your worktree.** `yggterm` is a GUI + daemon that
*replaces* the fleet's running binaries (6 paths on 3 hosts, `deploy-spec.md`
§0–§4). Two worktrees building in parallel interleave `target/` artefacts,
fight over the deploy lease (`scripts/deploy-fleet.sh` §lease), and replace the
daemon other agents are testing — measured 2026-08-27: a stale checkout wrote
`3.1.60` over `3.1.61` fleet-wide with four green ✅. **A per-worktree build is
the per-session watchdog that can only watch itself — dead in the case that
matters.** `ygg-ci` fixes it by collapsing `N` lanes into **one integration
build on one host**.

| fact | law |
|---|---|
| **Auto host is `dev`** | all ci builds run on `dev`, by tune default. Other hosts subscribe over `ssh dev`. |
| **One build, many testers** | **v2 (2026-09-03, owner-directed): the CI integrates IN the main branch of the local main repo — no worktrees of any project.** The tick merges `origin/main` + every subscribed `lane/*` into the main checkout, `cargo build --release` once **there**, then pushes `main` to the upstream **only when the build is green**, then deploys — hosts never run commits upstream does not have. The old scratch-worktree model is gone: a worktree integration built bytes that lived on no branch, which is the split the 3.2.4x daemon churn grew from. A red build/gate resets main to the pre-tick state and QUARANTINES the failing lanes at that tip (the next tick builds the rest; a new lane tip re-arms its lane). |
| **Fleet-aware** | `deploy-fleet.sh` already sweeps `dev + $(ygg-live-host.sh) + oc`, verifies read-back checksums, and holds the deploy lease. `ygg-ci` just calls it; no second deploy path. |
| **Timer, not a burn** | `watch` sleeps `interval` (default `300s`, tunable per project) and each `tick` does `fetch + stat` only; a clean tick costs no build. `subscribe` auto-spawns the watcher if none is alive — same arm shape as booter `ygg-booter.py:80`. |
| **Talking events** | every transition lands in `~/.yggterm/relay/ci/events.jsonl` (read: `ygg-ci.py events --since 30m --json`) and failures ALSO post to msgGraph `infra/ci` (throttled per signature): merge_refused, build_failed, gate_failed, lane_quarantined, blocked-dirty-main, diverged, push_failed, deploy_failed. `ygg-ci.py why` is the plain-language state + the next action. |
| **Any gitcoding project** | project recipe lives in `~/.yggterm/relay/ci/ci.json` (`repo`, `build`, `deploy`, `interval`, `host`, `gates`, `push`, `push_remote`). An agent tunes it in place; the watcher picks it up next tick. `yggterm` is the default recipe; `make`, `npm run build`, or any shell command works for other repos. |
| **Subscribe = next build** | when the service is present, other agents can subscribe to it. An agent asks it to take their commits on the next run; the watcher aggregates on the next `tick`. |

```sh
# enroll your lane — do this AFTER pushing the lane branch
ygg-ci.py subscribe --lane lane/foo/bar --project yggterm
# or: ssh dev ygg-ci.py subscribe --lane lane/foo/bar --project yggterm

ygg-ci.py list --project yggterm          # who is enrolled
ygg-ci.py status --json                   # watcher alive? held? last build?
ygg-ci.py events --since 30m              # the talking plane: refusals, builds, pushes
ygg-ci.py why                             # plain-language state + the next action
ygg-ci.py tick --project yggterm --dry-run  # what WOULD merge (no build)
ygg-ci.py tick --project yggterm          # one integration pass now: merge → build in main → push → deploy
ygg-ci.py unsubscribe --lane lane/foo/bar --project yggterm  # when done

# tune how a project builds — any agent can do this, it lands for everyone
ygg-ci.py tune --project yggterm --interval 300 --build "cargo build --release" --deploy "scripts/deploy-fleet.sh" --gates "scripts/check-privacy.sh" --repo ~/gh/yggterm
ygg-ci.py tune --project myapp --repo ~/gh/myapp --build "npm test" --deploy "" --interval 600
ygg-ci.py config --project yggterm --json

# the OFF switches — same shape as booter
ygg-ci.py disarm --hours 4 --note "red main"   # keep subs, refuse to build
ygg-ci.py arm
ygg-ci.py hold --until 2h --reason "main is red"  # fleet-wide build hold
ygg-ci.py hold --clear
```

**Tuning.** `tune` writes `~/.yggterm/relay/ci/ci.json`. A per-repo `.ygg-ci.json`
is also honoured if present. The interval is read fresh each loop so a retune
needs no watcher restart. `DEFAULT_INTERVAL=300` — same reason booter is `300`:
the cache-burning cost is in the *build*, not the poll.

**When NOT to use it.** A pure one-off probe (`cargo test --lib foo`) stays
local. Anything that would replace `~/.local/bin/yggterm*`, the daemon, or be
tested by more than one agent goes through `ygg-ci`.

**Contracts (yggterm):**
- The main checkout must be CLEAN and ON `main` before a tick — the hygiene gate blocks otherwise (that is the point: the CI builds the branch every agent pulls).
- The push to upstream is the LAST gate: build green → gates green → push → deploy. The deploy never runs commits upstream lacks.
- Lanes land via the CI's own push of main (the build publishes); ci auto-unsubscribes lanes already in `main`.

**Conflicts — deterministic, no guessing, no extra turns:**

*Same file, different hunks:* `git merge` auto-merges — both lanes land in the
integration build. No conflict.

*Same hunk:* `tick` merges subs in enlist order per `ygg-ci.py:703`. First
wins, second `merge --abort`s, is **excluded only**, and is recorded as
`conflicts:[{lane,reason:conflict}]` in `~/.yggterm/relay/ci/builds/<id>.json`
and in `ygg-ci.py status --json → last_build.conflicts` and `ci.log`. Build
still runs/deploys with the merged subset — one conflict never blocks other
agents. `no-remote-branch` (not yet pushed) is recorded the same way and
simply skipped until the push appears.

Your lane in `conflicts` → **you** fix it, `ygg-ci` retries automatically:

```sh
ygg-ci.py status --json | python3 -m json.tool  # is my lane in last_build.conflicts?
git fetch origin
git rebase origin/main          # or: git rebase --onto origin/<winner> if stacked
# fix hunks, git add && git rebase --continue
git push --force-with-lease origin lane/foo
# next tick (≤300s or `ygg-ci.py tick --project yggterm`) merges clean — no unsubscribe needed
```

Do **not** `unsubscribe` a conflicted lane to "clear" it — that drops you from
the next build. Do **not** re-subscribe — the subscription stays, the watcher
re-merges the new `tip` on the next dirty check. Only `unsubscribe` when the
work is done or the lane has landed on `main` (then `ygg-ci` auto-removes it).

## 4. Correspondence — any session can reach any other

A row is an address. That is the whole mechanism, and it needs no new protocol:

```sh
# ask another session for help, or hand it a subset of your work
yggterm server app terminal submit "$OTHER_ROW" --stdin <<'EOF'
ACK-REQ-7731 — I am <row>. I need <the specific thing>.
Reply by submitting to my row: <your row path>.
EOF

# read what any row has done, without touching the human's view
yggterm server app terminal read-buffer "$OTHER_ROW" --mode screen
```

Three rules that make this safe:

1. **Address by row path, and say who you are.** A message with no return
   address forces the other session to guess, and it will guess wrong.
2. **A busy row QUEUES your message and answers at its turn boundary.** You do
   not need it idle. You DO need to not send five times because it was quiet.
3. ⛔ **Never inject keystrokes into a row with an open dialog on the human's
   seat.** That answer is theirs.
4. ⛔ **PEER NAMES RECYCLE — resolve to a UUID before you trust one.** Measured
   2026-08-20: a freshly spawned delegate came up wearing the exact peer name
   (`<slug>-<suffix>`) of a DEAD predecessor from six days earlier, and a
   message addressed by that name reached the right row only by luck. Same law
   as row titles: a name identifies nothing across time. Before acting on a
   peer name, resolve it — the socket name carries the PID, and the session
   file for that pid names the session uuid — and address the uuid's owner.

**Submitting yourself to an orchestrator** is the same verb pointed the other
way: when work turns out to belong to a wider effort, message the orchestrating
row with what you hold and what you propose, and let it decide. Volunteering
beats stalling, and it beats doing someone else's subset unasked.

---

## 5. First-run bootstrap — wire the memory system ONCE

Long campaigns need somewhere durable to think. Turns spent re-deriving that
wiring are turns not spent working, so it is scripted:

```sh
.agents/skills/yggterm-agent-fleet/bootstrap.sh            # idempotent
.agents/skills/yggterm-agent-fleet/bootstrap.sh --dir <path-to-memory-dir>
```

It creates the memory directory and a `MEMORY.md` index if they are absent, and
does nothing if they exist. Run it once; never re-run it to "refresh".

### The shape it creates, and why

| file kind | holds | lifetime |
|---|---|---|
| `MEMORY.md` | the INDEX — one line per memory, a door not a room | forever |
| `campaign-*.md` | a live ledger: current state, the laws, the handover log | while the campaign runs |
| `finding-*.md` | one durable lesson, cited from code by wikilink | forever |
| `feedback-*.md` | how the human wants you to work, and WHY | forever |
| `spec-*.md` | a behaviour contract | until the behaviour changes |

⛔ **One question, one owner.** The index must never hold a second copy of what
is open — an open-work list in two places rots in the one nobody reads. Keep
status in the repo's queue file; keep reasoning in memory; keep what shipped in
git.

⭐ **The handover is the campaign's real output.** Each session writes, at the
TOP of the campaign file: what it finished, what it measured, what it left, and
the next load-bearing subset. Then it spawns its successor per §3 and is killed
by it. One session grinding at a time; the baton is explicit.

---

## 5.1 Unified Cross-Harness Memory (`~/.yggterm/memory`) & `ygg-memory`

Different CLI harnesses (Claude Code, Gemini/Antigravity, Grok, Codex, Muse)
maintain local memory stores in `~/.{claude,gemini,grok,codex}/`. `~/.yggterm/memory`
acts as the unified cross-harness memory hub with an append-only event journal
(`journal.jsonl`) and per-harness watermark tracking (`watermarks/<harness>.json`).

### The Turn-One Retrieval Ritual (<40 tokens):

At the start of any session or campaign, check if other harnesses have published
new findings or handover updates since your last sync:

```sh
# 1. Cheap status check (~25 tokens)
python3 .agents/skills/yggterm-agent-fleet/ygg-memory.py status --harness <me>

# 2. View delta summaries if behind (~80 tokens)
python3 .agents/skills/yggterm-agent-fleet/ygg-memory.py diff --harness <me>

# 3. Impatient / selective absorption (fetch only what you need)
python3 .agents/skills/yggterm-agent-fleet/ygg-memory.py get --file <campaign-or-finding.md>

# 4. Acknowledge absorbed items
python3 .agents/skills/yggterm-agent-fleet/ygg-memory.py ack --harness <me> --files <campaign-or-finding.md>
# Or acknowledge all up to latest:
python3 .agents/skills/yggterm-agent-fleet/ygg-memory.py ack --harness <me> --all
```

### Publishing New Findings Across Harnesses:

When you discover a durable finding or write a campaign handover, publish it so
every other harness's next session is informed:

```sh
python3 .agents/skills/yggterm-agent-fleet/ygg-memory.py publish --file <finding-or-campaign.md> --harness <me>
```

### The Harness Isolation Law (No Cross-Harness Private Writes):

- ⛔ **Private harness stores are strictly PRIVATE:** No agent (Gemini/Antigravity, Grok, Codex, Kimi, Muse, etc.) is permitted to write directly into another harness's private directory (`~/.claude/`, `~/.gemini/`, `~/.grok/`, `~/.codex/`).
- ✅ **Reading is allowed; writing is forbidden:** An agent may read another harness's files if needed for context, but must NEVER mutate them.
- ⭐ **The Unified Store is the Only Conduit:** Cross-harness knowledge sharing must travel through `~/.yggterm/memory/` (via `ygg-memory publish`) or canonical project repository documents (e.g. `docs/discussions/`).

---

## 6. ⛔ Succeed a session that has gone cold — HARVEST IT, never ask it

**A long-running row drifts into the worst cell of a two-by-two: a COLD prompt
cache holding a LARGE context.** Those costs multiply rather than add — every
turn re-writes a huge input at full price instead of reading it cheaply — so one
turn on such a row can cost more than the entire remaining job.

The remedy is a **succession**: harvest what the session knows from artefacts that
cost nothing to read, write it somewhere durable, and continue in a fresh, small,
warm session. Both problems disappear at once.

### ⛔⛔ AND DELIVERING COSTS THE SAME AS ASKING — the rule this skill was missing

**Root-caused 2026-08-13 after a campaign burned a large re-read to deliver one
sentence.** §6 says the *asking* is the expense and §10 says a message is for
*delivering, not enquiring* — and a reader who follows both to the letter still
concludes that **delivering is free. It is not.** A cold row pays the same full
re-read whether your message asks it something or tells it something. The
direction of the message was never the variable.

⇒ **THE VARIABLE IS `CONTEXT SIZE × CACHE WARMTH`, and before 2026-08-13 nothing
in the fleet reported the second one.**

**BEFORE YOU SUBMIT TO A ROW, MEASURE BOTH:**

```sh
U=<their-uuid>; F=$(ls -t ~/.claude/projects/*/$U.jsonl | head -1)
python3 - "$F" <<'EOF'
import json,sys,time,datetime
last=None; ts=None
for ln in open(sys.argv[1]):
    try: d=json.loads(ln)
    except: continue
    if (d.get('message') or {}).get('usage'):
        last=d['message']['usage']; ts=d.get('timestamp')   # a usage block == a REAL inference
tot=sum(last.get(k,0) for k in ('input_tokens','cache_read_input_tokens','cache_creation_input_tokens'))
age=(time.time()-datetime.datetime.fromisoformat(ts.replace('Z','+00:00')).timestamp())/60
print(f"context {tot:,} ({tot/1_000_000*100:.0f}%)  ·  last real inference {age:.0f} min ago")
EOF
```

⛔⛔ **`mtime` IS NOT CACHE WARMTH — this is the THIRD costume of one defect.**
A slash command (`/context`) writes the transcript **without sending a
cached-prefix inference request**: mtime moves, the cache does not warm. Measured
the day this was written — a row read **26 min** by mtime and **76 min** by last
real inference, a 50-minute lie, and the owner caught it. The family:
*mtime is not PROGRESS* · *mtime is not LIVENESS* (a refused turn answers in
milliseconds and still stamps it) · **`mtime is not CACHE WARMTH`**.
⇒ **The discriminator is the last record carrying a `usage` block.** A `usage`
block only exists where an inference actually happened.

**THE DECISION, once you have both numbers:**

| their state | channel |
|---|---|
| warm (< ~50 min) **or** small context | **submit** — it is cheap, say the thing |
| **cold AND large** | ⛔ **do not submit.** Use the FILE channel: the queue, their campaign door, a brief |
| cold, large, **and time-critical** | submit anyway, and say in the message why it could not wait |

⭐ **And check whether the content is ALREADY ON DISK.** The burn that produced
this rule delivered *"your question is already answered"* — and the answer sat in
a committed file that row reads anyway. **A message whose content is in a file the
row already reads is a pure loss.**

### ⚖ THE BOOTER IS A STALL DEFENCE; A CACHE KEEPALIVE IS A DIFFERENT JOB

**Named apart 2026-08-13 so nobody ships a promise the window cannot keep.** The
booter's idle window is **7 minutes** — tuned to kick a row that STOPPED. Cache
warmth runs on a different clock (~50 min on the extended TTL, under 5 on the API
default). ⇒ **One window cannot serve both:** as a keepalive, 7 minutes is
simultaneously *too late* for a default-TTL cache and *43 minutes too eager* for
an extended-TTL one. **Either the window becomes per-purpose, or the keepalive is
built separately — but do not describe the booter as the cache defence while it
runs a stall clock.**

### ⛔⛔ The asking IS the expense

**Never ask a cold, high-context session to write you a handover.** Requesting one
is several of the most expensive turns that row will ever run — the precise cost
the succession exists to avoid. And it is worse than it looks: **the moment you
prompt a cold row it becomes warm**, so respawning it *afterwards* throws away
the money you just spent.

⇒ **A fork with no middle: either (a) touch nothing and succeed it from
artefacts, or (b) having touched it, keep it. Never both.**

⚖ **The exception, and check for it first:** asking is right when the respawn is
**not** cost-motivated — a session that is context-*exhausted* rather than
expensive-to-run can write an excellent handover, because it still has the cache
you would otherwise pay to rebuild. **Ask yourself WHY you are respawning before
you reach for the ask.**

### Three outcomes, not two

| the row is… | do this |
|---|---|
| **cold, with work still pending** | **succeed it** — harvest, distil, claim, retire |
| **parked, and its repo is clean and pushed** | **let it go cold.** Its value is the written output, not its context — *read the files, not the row.* Warming is waste; succeeding it is worse |
| **parked, but its repo is dirty or unpushed** | **harvest FIRST** — knowledge may be trapped in its head that exists nowhere else |

⛔ **A dead row has no cache at all** — it can only be read or resumed from disk.
Warming advice is meaningless for it; suppress it.

### The harvest, cheapest instrument first

A transcript is a plain JSONL file on disk. Reading its tail costs a file read;
making the session produce the same summary costs a fortune. **Same information,
two prices, orders of magnitude apart.**

```sh
J=~/.claude/projects/<cwd-slug>/<uuid>.jsonl

# a. When did it ACTUALLY last work?  ⛔ NOT the file's mtime — see below.
jq -r 'select(.timestamp) | .timestamp' "$J" | tail -1

# b. What was it TOLD? Human turns are the highest-signal, cheapest read there is.
jq -r 'select(.type=="user" and (.message.content|type=="string"))
       | .timestamp + " | " + .message.content' "$J"

# c. What did it CONCLUDE? A working row's last message is its own status report.
jq -r 'select(.type=="assistant")
       | (.message.content // [] | map(select(.type=="text").text) | join("\n"))' "$J" \
  | tail -c 6000

# d. ⭐ What did it WRITE? — the step that decides whether it is safe to retire.
jq -r 'select(.type=="assistant") | (.message.content // [])[]
       | select(.type=="tool_use" and (.name=="Write" or .name=="Edit"))
       | .input.file_path' "$J" | sort -u
```

⭐ **(d) IS THE ONE THAT SETTLES IT**, and it answers a question no summary can:
*is there anything in this session's head that is not already on disk?* **If every
`Write`/`Edit` target is a file you can go and read, the transcript holds nothing
unwritten and the row is safe to retire.** If a target is missing, uncommitted, or
outside any repo, harvest **that specific thing** before you kill anything.

⛔ **mtime is not progress.** A transcript's modification time moves for metadata
— stored titles, mode, last-prompt — long after the last real turn. Measured: a
row whose file read as *touched 22 minutes ago* had last actually worked **36
hours** earlier. Read the last real timestamp out of the content, as in (a).

Then widen only as needed: `git log`, the repo's state file or status command, and
whatever queue file the campaign keeps.

### Distil into DURABLE MEMORY, not into a bespoke brief

⭐ **This is the real prize.** Write the distillate into the campaign's own memory
or state file, and it becomes the **standing handover surface** — the next
successor reads it like any other session and needs no brief written for it. A
brief that duplicates campaign memory is a second copy that will go stale.

- ✅ **Carry:** what is DONE and where it is written · what is OPEN and the exact
  next step · decisions ALREADY MADE, so they are not relitigated · dead ends
  **with the evidence that killed them**, so they are not re-walked · outstanding
  approvals · traps already paid for.
- ⛔ **Drop:** the narrative, the tool-by-tool path, superseded drafts, and
  anything the repo already holds — **point at the repo instead. A pointer is
  cheaper than a paste and cannot go stale.**

**The test of a good succession: the successor never needs the predecessor's
transcript.** Carry conclusions and state; drop the history that produced them.

### Then take the seat, in this order

1. **Claim your own row first** (§1) — rename, seat, read the title back.
2. **Only then retire the predecessor** (§7 — and reap it yourself).

⛔ Retiring first leaves a numbered gap in the sidebar and an unclaimed identity,
which is exactly the state a human cannot navigate.
⛔ **Announce a succession before you perform it.** The action is usually right;
being surprised by it is not.

### ⭐ Assume this — do not wait to be asked

When you are told to continue work whose previous session has gone cold, **this
whole sequence is the default**: harvest → distil → claim → retire → continue.
Say what you did. Do not ask permission to read a file you already own, and do
not offer to "ask the old session" as though it were the cheap option.

---

## 7. ⛔ Verbs report the REQUEST, not the EFFECT — read state back, every time

The single most expensive pattern in fleet work: **you ask about one thing and the
instrument answers about another.** Verbs across this surface return a success
field describing what they *asked for*, not what *happened*.

Documented cases, each of which cost real time before it was understood: a rename
that reported success and reverted later; a reorder that returned
`changed: true` three times while the sidebar never moved; a reorder verb that
echoed the requested order back as though it were the rendered one; a send that
returned `error: null` for messages that never arrived; and a remove that timed
out on removals which had in fact succeeded.

⇒ **THE TEST, before trusting any probe: write down the question you asked, and
the question the instrument actually answers. If they differ by one word, it will
lie to you eventually.**

### Removing a row: two fields, two different questions

```sh
yggterm server app session remove "$ROW"
```

Measured: `row_still_listed: false` together with `verified: false` and
`verified_refusal: "remote_runtime_survived"`. **Both are true and they are not
contradictory** — the ROW was genuinely gone from the sidebar, while the agent
PROCESS kept running on the remote host, because only the local transport had
been reaped. Read both fields; they answer different questions.

⇒ **Reap the runtime yourself, by `/proc`, requiring BOTH a plausible agent
binary AND the session id:**

```sh
for p in $(pgrep -f -- "$UUID"); do
  c=$(tr '\0' ' ' < "/proc/$p/cmdline" 2>/dev/null)
  case "$c" in *pgrep*) continue ;; esac        # your own query matches too
  case "$c" in *claude*"$UUID"*|*codex*"$UUID"*) kill -TERM "$p" ;; esac
done
```

⛔ **`pgrep -cf "<uuid>"` COUNTS THE QUERYING SHELL** and has reported a dead row
alive. **Never use a count as proof of life or death** — identify each candidate
and check its cmdline. `ygg-claim.sh --replace` does all of this for you.

### ⚖ Before blaming the verb, check your own transport

App control answers only on the host where the GUI process runs, so an agent on
any other host reaches it over ssh — and **`ssh host "yggterm $*"` hands the far
side ONE string, which the remote shell re-splits on whitespace.** A multi-word
title therefore arrives as several arguments and `rename` takes only the first:
ask for `"topic: the long name"` and the row comes back titled `topic:`.

**That looks exactly like the CLI re-titling itself**, and it is not. Measured
2026-08-08: the same rename, sent with each argument quoted, held indefinitely;
sent unquoted it truncated every time. The wrong diagnosis sends you hunting a
defect in the application while your own helper corrupts every call it makes.

⇒ **Quote each argument for the remote shell** (`printf '%q'`), and when a verb
"misbehaves", reproduce it with the simplest possible direct invocation before
concluding anything about the verb. ⚖ Hold your own theory to the falsification
bar you would demand of anyone else's.

### A quiet row is not necessarily an idle row

Three distinct failures all look like *"the row is idle"*, and each needs a
different hand. Diagnose with the terminal screen plus a `/proc` check — never
with a count:

| # | agent process | what `submit` says | remedy |
|---|---|---|---|
| 1 | **absent** | never echo-confirms | **remove + re-spawn** — a restart will not revive it |
| 2 | alive, PTY attached | *"never echo-confirmed it was consuming input"* | **`server terminal restart`** — clears the wedge in seconds |
| 3 | alive on a real PTY, but the daemon shows no runtime | *input readiness is **unanswerable** rather than false* | **`server terminal restart`** — re-attaches to the existing pty, spawns no rival |

**#2 is a WEDGE: alive, turn ended, sitting in its event loop, not reading
input** — and it silently eats every message sent to it. It is a pattern, not a
one-off. ⇒ **probe any row that goes quiet with `submit` BEFORE assuming it chose
to be idle**, because `send` cannot see this at all: it returns `error: null` for
messages that vanish.

⭐ Note that #3's verb behaved *well*: it distinguished **unanswerable** from
**false** instead of guessing. That refusal string is the discriminator between #2
and #3 — read it, do not just check the boolean.

---

## 8. The baton relay — a campaign session does not END, it HANDS OFF

§6 is what you do when a session has *already* gone cold. **This is the planned
version, and it is strictly better: a living session hands the work on while it
still knows everything.** Relay when you can; harvest when you must.

**A campaign outlives one context window only by handing itself off**, and the
handoff is a cycle, not an ending:

1. ⛔ **KILL YOUR PREDECESSOR FIRST — before any work.** Two sessions of the same
   campaign must never grind at once: they fight over the branch, the daemon and
   the deploy lane. ⇒ **the brief you write MUST carry the outgoing row's own
   path**, or your successor cannot honour this and you get two. Prove it by the
   row's absence and by `/proc`, never by the verb's own field (§7).
2. **Take the MOST LOAD-BEARING subset — not the next item in file order.** One
   subset per session.
3. **Stop on EITHER of two conditions:** the subset is done, or it needs the
   human. ⛔ **There is no third condition** — "I ran low on context" does not
   license skipping the handoff, it is precisely when the handoff matters.
4. **When it needs the human, RAISE A NOTIFICATION targeted at your own row.**
   They are not watching; an unnotified question is a stall discovered hours
   later.
5. **Spawn the successor, then die.** Hand it (a) your row path to kill, (b) the
   subset you finished, (c) the next load-bearing subset, (d) anything parked
   for the human, (e) ⛔ **YOUR OWN MODEL — a successor runs the SAME model as
   its predecessor** (owner directive 2026-08-20: *"when you spawn a new version
   of yourself you should use the same model"*; most fleet sessions are Opus,
   but some seats are deliberately Fable and a relay must not silently demote or
   promote one).
   ⛔⛔ **THE MODEL IS NOT INHERITED, BY ANY MECHANISM.** A spawn that omits
   `--model` silently hands the choice to the CLI's configured default — it does
   NOT copy the spawner's model, and nothing warns. This is how two orchestrator
   relays in one day landed on the wrong model while every other part of the
   handover succeeded. ⇒ **Every relay and every delegate spawn writes `--model
   <id>` explicitly, even when "the same model" is intended — ESPECIALLY then.**
   ⛔⛔ **THE MODEL IS SET AT SPAWN, WITH THE CLI'S `--model` FLAG — NEVER BY AN
   IN-SESSION SWITCH.** Owner-directed 2026-08-20, after the CLI's own
   confirmation dialog stopped a switch mid-flight: *"Never attempt to switch
   like this, because it will burn tokens … model switching should be done in
   always new spawning with the --model flag. All CLIs have some equivalent."*
   **Why it is expensive and not merely untidy:** a conversation is prompt-cached
   AGAINST ONE MODEL. Switching re-reads the ENTIRE accumulated history on the
   next message, so the cost scales with everything the session has already
   done — precisely the cold re-read the fleet pays a keepalive to avoid. The
   dialog states it: *"This conversation is cached for the current model.
   Switching … means the full history gets re-read on your next message."*
   ⇒ **A seat that must run a particular model is SPAWNED on it.** A running
   session on the wrong model FINISHES ITS WORK on the wrong model and hands
   over; the correction rides the next spawn, where it costs nothing.
   ⚠ The one safe moment is TURN ZERO — an empty session has no history to
   re-read — so a switch there repairs a spawn that has just failed. It is
   never a way to re-aim a session that has already done work.
   ⭐⭐ **THE FLAG WORKS — the "silently dropped" finding was a MISDIAGNOSIS,
   corrected 2026-08-20 (33a7fc2c).** `terminal new --model` reaches the process
   and the cmdline carries it. What actually fails is a spawn aimed at a host
   whose RUNNING daemon predates the flag: that call answers
   **`data.launch.applied: false`** while echoing the model back, and nothing
   reaches the process. The reply was truthful all along, in a field nobody
   read. ⇒ Do not re-cite the old blocker; it licenses spawning without the
   flag, which is the exact mistake it grew from.
   ⭐ **THE SPAWN-TIME MODEL CHECKLIST — four steps, all mandatory:**
   1. Write `--model <id>` explicitly in the create call (it is never inherited).
   2. Read **`data.launch.applied`** in the create reply. `false` ⇒ the flag did
      NOT reach the process (stale daemon on the target host); the row is on the
      wrong model NOW, at turn zero — the one safe moment to kill and respawn.
   3. After the first turn, verify the transcript's assistant-record `model`
      field: `grep -o '"model":"[^"]*"' <transcript> | sort | uniq -c`. This is
      the ONLY decisive instrument — the screen banner false-negatives once it
      scrolls, and the cmdline proves the REQUEST, not the model answering.
   4. Wrong model discovered after work has begun ⇒ say so plainly, FINISH the
      unit on the wrong model, and hand over — the correction rides the next
      spawn, where it is free. Never switch in-session (the law above).

   ### ⛔⛔ THE MODEL EQUATION — a "default model" is MUTABLE STATE, and it lies to agents
   Measured 2026-08-20 across the fleet's CLIs, after four seats landed on the
   right model for the WRONG reason. The laws, in force for every spawn and
   every relay:
   1. ⛔ **A CLI's default model is a FILE another session can rewrite, not a
      constant.** Claude Code: the in-session `/model` command WRITES
      `~/.claude/settings.json` `model` — proven by timestamp correlation
      (four queued `/model` submits; the file's mtime is the minute they
      applied). Convenient for a human choosing interactively; for agents it
      means **any session's model repair silently re-aims every later spawn on
      that host that omits the flag.** After an intentional `/model` repair,
      the host default HAS CHANGED — either intend that or restore the setting
      explicitly and verify (⚠ a settings file is the app's OUTPUT: a running
      CLI can write it back from memory).
   2. ✅ **The spawn FLAG does not stick.** `claude --model <id> -p 'hi'` with a
      non-default id answered on that id and left the settings default
      untouched (measured). ⇒ the flag is the ONLY side-effect-free way to aim
      a session.
   3. ⛔ **"Right model without a working flag" is a MASK, not a success.** When
      the flag is dropped (see 4) the session lands on the sticky default,
      which can coincide with what you wanted — four seats did exactly this.
      "The successor inherits the model by relay" is BANNED as an explanation:
      there is no inheritance mechanism anywhere; what that phrase always
      described was the sticky default. Verify by transcript, every spawn.
   4. ⛔ **The REMOTE lane drops `--model` — ROOT-CAUSED AND FIXED IN CODE
      2026-08-20, live from the next daemon version-bump deploy** (queue
      entry: "A REMOTE CC SPAWN'S --model NEVER REACHES THE PROCESS"). The
      create was never the fault: a LAUNCH-COMMAND REBUILD (TerminalRestart /
      SyncTerminalIdentity) between the row's birth and its first spawn
      recomposed the exports without the row's stored options — the remote
      twin of the 2026-08-06 local-CC rebuild bug. Until a bumped daemon is
      LIVE on the mediator, the canonical REMOTE recipe stays: **spawn
      WITHOUT a prompt — but WITH `--title` and `--outline <seat>` (the
      create applies both AT BIRTH; `seat.honoured` is re-read from the
      rendered order) → verify model (transcript/cmdline) → wrong ⇒ drive
      the in-session model command at TURN ZERO (empty session, nothing to
      re-read; law 1's side effect applies) → THEN deliver the brief via
      submit → verify by ACK token.** After the deploy, verify by transcript
      once and retire the model workaround. Local spawns apply the flag
      correctly (proven live); still verify.
      ⛔ **Title and seat go IN THE CREATE, never left for the delegate's
      claim.** A spawn without them sits in the sidebar as "Agent unnamed" at
      the head until the claim runs minutes later — the owner meets that
      window every time. And the spawner-declared seat ledger
      (`spawned-by-<uuid>.txt`) is read on the DELEGATE'S host: for a
      cross-host spawn the declaration is invisible to the claim, so the
      create-time `--outline` is the only seat channel that works everywhere
      (measured: two same-day cross-host lanes claimed titled but UNSEATED).
   5. ⭐ **Per-CLI register** (yggterm's `--model` maps to each CLI's native
      flag via the descriptor; native semantics measured where marked):
      | CLI | native flag | default lives in | flag sticks? | in-session cmd sticks? | decisive verification |
      |---|---|---|---|---|---|
      | claude-code | `--model` | `~/.claude/settings.json` `model` | NO (measured) | YES — `/model` writes it (measured) | transcript `"model"` per assistant record |
      | codex | `-m/--model` | `~/.codex/config.toml` top-level `model` | unchanged across two refused runs (weak; UNMEASURED positive) | UNMEASURED | **footer `<model> <effort> · <cwd>` (measured)**; transcript field UNMEASURED |
      | muse | `--model` | `~/.config/muse/settings.json` `model` | **NO (measured)** | UNMEASURED | **footer `<model> · <effort> · <cwd> · YOLO` (measured)**; a bad id is refused BY NAME by the provider |
      | antigravity | `--model` ("for the current CLI session"; `models` lists) | `~/.gemini/antigravity-cli/settings.json` `model` — ⚠ **stores the DISPLAY NAME ("Gemini 3.x Flash (High)"), not the id the flag takes** (measured) | **NO (measured — config mtime unchanged across a flagged run)** | n/a | `-p … --output-format json` returns `{conversation_id, usage{…}}`; `models` lists the ids |
      | grok-build | `-m/--model` | `~/.grok/` | UNMEASURED | UNMEASURED | **footer `<model> (<effort>) · <approval-mode>` (measured)** |
      | kimi | `-m/--model` | `~/.kimi-code/config.toml` (login-managed) ⚠ its own `--help` names `~/.kimi/config.toml`; **the directory on disk is `.kimi-code`** | UNMEASURED | UNMEASURED | **footer `context: N%` (measured)** |
      | qwen-code | `-m/--model` | `~/.qwen/settings.json` | UNMEASURED | UNMEASURED | UNMEASURED — first run holds on a consent MENU (§11) |
      | pi | `--model` (`provider/id`) | `~/.pi/agent/settings.json`; custom providers in `~/.pi/agent/models.json` | UNMEASURED | UNMEASURED | **footer `<used>/<window> (auto)` + the model id (measured)** |
      | opencode | `-m/--model` (`provider/model`) | `~/.config/opencode/opencode.jsonc` | UNMEASURED | UNMEASURED | **footer `<agent> · <model>` and `<used> (<pct>)` (measured)** |

      ⛔⛔ **AND THE LAW THAT OUTRANKS THE WHOLE TABLE: ON A REMOTE SPAWN THERE IS
      CURRENTLY NO WAY TO PIN A MODEL FOR ANY CLI.** Measured 2026-08-20 across
      all ten kinds, and it splits two ways — neither of which gives you a
      pinned model:
      - **Nine kinds REFUSE the flag BY NAME.** `terminal new --kind <any but
        claude-code> --machine-key <host> --model <id>` answers, before creating
        anything: *"a REMOTE <kind> session cannot carry --model /
        --permission-mode yet: that lane has no extra-args forwarding to the
        remote host (claude-code does, via an env var). Launch it locally on
        that machine, or use --kind claude-code."* ⭐ **This refusal is the
        HONEST outcome and costs nothing** — no row is created, so there is
        nothing to reap.
      - **claude-code, the ONE kind that accepts it, silently loses it.** The
        create succeeds, `launch.model` echoes your id back, and
        **`launch.applied` reads `false`** — `--model` appears nowhere in the
        resulting launch command and the row lands on the host's sticky
        settings default (its banner will name that default, not your id).
      ⇒ **Read `launch.applied` on EVERY create.** It is the field that tells
      the two apart, and it was correct both times. ⚠ Do not "route around" the
      nine refusals by switching a lane to `--kind claude-code` — that silently
      changes which CLI runs the work.
      ⚠ **"Launch it locally on that machine" is not available on a fleet whose
      GUI host does not carry the CLIs.** Check before planning around it: on
      the host measured, the GUI host had NONE of the eight non-Claude CLIs
      installed, so every one of them can only be spawned remotely — i.e. only
      on its unpinnable lane.

      ⭐⭐ **THE CONTEXT EQUATION IS THE PTY FOOTER, AND READING IT TYPES
      NOTHING.** Every one of these TUIs paints its model, and often its context
      usage, into the last two rendered lines. `server snapshot` →
      `live_sessions[].terminal_lines` returns them, so a single read answers
      "what model is this row on" and "how full is it" for ANY CLI, with no
      slash command, no keystroke, and no risk of typing into a live prompt.
      **It is the safest cross-CLI instrument there is** — prefer it to every
      per-CLI `/context` equivalent. Exact spellings measured 2026-08-20:
      | CLI | footer carries | context %? |
      |---|---|---|
      | codex | `<model> <effort> · <cwd>` | no |
      | muse | `<model> · <effort> · <cwd> · YOLO` | no |
      | grok-build | `<model> (<effort>) · <approval-mode>` | no |
      | kimi | `yolo  agent  <cwd>` then `context: <pct>%` | **yes** |
      | pi | `<cwd>` then `<used>/<window> (auto)` + model id | **yes, with the window** |
      | opencode | `<cwd>` then `<used> (<pct>)`; model on the `<agent> · <model>` line | **yes** |
      | claude-code | banner names model + effort + account | no (banner scrolls away) |
      ⚠ Strip the ANSI/CSI noise before matching — these footers are drawn with
      colour and cursor-positioning escapes, and a naive substring test misses.
      ⚠ Auth can constrain the id space (codex under a ChatGPT account refuses
      non-account models by NAME — the refusal is loud, read it). ⚠ UNMEASURED
      cells are invitations, not blanks to assume across: measure with a
      cheap non-interactive run and write the answer back here (§11's
      written-to-GROW rule).

   ### ⛔⛔ THE LANE LIFECYCLE — a report is not an ending, and idle-after-report is the SPAWNER'S debt
   Owner-observed 2026-08-20: finished lanes sat as idle rows being nagged by
   the booter, and he could not tell design from mistake. The contract, all
   three steps mandatory:
   1. A lane's turn NEVER ends silently: it ends with more work self-queued,
      or with ONE batched DONE report delivered to its spawner. The booter is
      the LAST line, never the return path — every booter kick means this
      contract failed somewhere.
   2. ⭐ **A big report or brief is PARKED AS A FILE + a short pointer message.**
      Measured twice: a ~5KB multi-line `--stdin` submit answered
      `submitted:true` in 245ms and delivered NOTHING to a busy row; a short
      pointer to a file on disk landed first try. Park under
      `~/.yggterm/relay/`, name the sender and receiver in the filename.
   3. The SPAWNER reviews, harvests (merge + verify), and then DESPAWNS the
      reported lane. An idle lane that has reported is the spawner's debt,
      not the lane's; a lane with follow-on work proposes it IN the report
      and continues only on the spawner's confirmation.
   ⛔ **Cross-orchestrator spawns:** a spawn on another orchestrator's behalf
   takes its seat FROM that orchestrator (or spawns unseated and reports the
   row path for seating). A number NEVER enters a title. And know the
   identity trap — CORRECTED 2026-08-20 after a lane falsified the first
   version of this law: for claude-code rows yggterm launches, **the row uuid
   IS the CLI session id** (`--session-id <row uuid>`, read it off the
   cmdline), so hunting a "different real id" finds nothing. **The actual trap
   is TRANSCRIPT ABSENCE: a live working row can have NO transcript file at
   all** (a CLI spawned with an inherited `CLAUDE_CODE_CHILD_SESSION` marker
   runs with persistence off — see the env-poison entry in
   `docs/pending-bugs.md`), so a transcript-keyed liveness check reads a LIVE
   worker as a husk using the CORRECT uuid — it did, and a working row was
   removed on that misread. ⇒ Key liveness on the PROCESS (cmdline session-id
   + cwd), never on the transcript; and the spawner is the authority on its
   delegate's state — ask it before declaring any delegate dead.
   And (f) ⛔ **ANY UNSENT OWNER DRAFT — a handover must lose no typed text**
   (owner design 2026-08-20: *"my prompt must be handed over to the next
   spawnee too, so there is no data loss"*). Check `~/.yggterm/relay/drafts/
   <your-uuid>.txt` (the booter's capture store) AND your own composer; hand
   the text to the spawnee by re-typing it into the fresh composer (text only,
   NO Enter) or inlining it in the brief marked OWNER-DRAFT-PRESERVED.
   Then it kills you.
6. **Repeat until the campaign is finished.**

### ⛔⛔ IN RELAY MODE, STOPPING IS A SIN — DECIDE, RECORD, CONTINUE (recorded 2026-08-10)

the requirement, after a relay session ended two consecutive turns by asking him a
question: *"In relay mode, you should not wait or ask me any questions. All
questions and ambiguity are to be recorded … and you simply finish all parts of
the campaign. In your questions I almost always (99% of the time) choose your
recommendations anyway."*

**⇒ THE ASK IS THE DEFECT, NOT THE UNCERTAINTY.** A relay session that surfaces a
fork and halts has converted a 99%-predictable decision into an idle row and a
context window going cold. **Take your own recommendation, do the work, and write
down what you chose and what would reverse it.**

### ⛔⛔ AND OFF HOURS, THE ASK IS NOT EVEN AVAILABLE — IT COST TWO HOURS (recorded 2026-08-11)

**ON HOURS ARE 09:00–17:30 ON THE `guihost` CLOCK. Outside that window a question is
not a slow path, it is a DEAD one.** The requirement, stated at ~23:50:
and sat idle until he woke: *"In ygg fleet skill relay mode, you should not ask
questions if it is off hours of guihost clock. On hours are 9AM - 5:30PM. In off
hour times, go with your recommendation and in the end let me know of choices you
made for me and if I will revert/modify your choices. Your question prompt costed
me two hours of work time."*

- ⛔ **Off hours ⇒ NEVER `AskUserQuestion`, for anything.** Not for an intrusive
  action, not for a fork, not for a "this will disturb you" courtesy. **Take the
  recommendation you were about to offer as the first option and DO IT.**
- ⭐ **Then tell him at the end, in the shape it was requested for:** *what I chose for
  you · why · and what to say to revert or modify it.* One short list, at the end
  of the turn — not a question, a receipt.
- ⚖ **Check the clock before you even consider asking:** `TZ=Asia/Kolkata date`
  (guihost's clock). ⚠ Off hours is the DEFAULT state for a relay campaign — these
  run overnight, so in practice this clause governs almost every turn.
- ⚠ **The instance:** a GUI restart to activate a shipped fix was offered as a
  question at 23:50 instead of just done. The restart later took **13 seconds**
  and lost nothing. Two hours of his working time bought a 13-second confirmation
  he would have given anyway.
- ⇒ **On hours, a question is merely discouraged (above). Off hours it is
  forbidden.** The reversibility bar in clause 3 is what decides — and a GUI
  restart, a deploy, a daemon swap and a row reshuffle are all reversible.

1. ⛔ **NEVER end a relay turn on a question.** Not "shall I continue?", not
   "which of these three?", not "want me to do X next?". If you can name a
   recommendation, you can act on it.
2. ⭐ **EVERY campaign gets ONE questions file, and the campaign door points at
   it.** Not a chat message, which dies with the transcript, and not the bug
   queue, which answers a different question. Name it for the plane it serves —
   `docs/pending-information-from-user.md` is the practice-rs spelling.
   Each entry carries: **the question · the options · ⭐ MY RECOMMENDATION · what
   I DID in the meantime · how to reverse it.** An entry with no recommendation
   is an unfinished entry.
3. ⚖ **The bar for acting is REVERSIBILITY, not certainty.** Reversible and
   recommended ⇒ do it and log it. Irreversible, destructive, outward-facing
   (a payment, a public post, mail to a third party, a force-push over someone
   else's work) or spending his money ⇒ log it and route around it, and **carry
   on with the rest of the campaign** rather than stalling on it.
4. ⛔ **"Blocked" is a claim you must test before you file it.** A missing
   credential is not a block until you have tried the vault and the fabric
   (see the `data-fabric` skill). File it only with the falsifier you ran.
5. ⭐ **He reads the file at the END.** So the last act of a campaign — not of
   every session — is to present it. Mid-relay, a question is a row in a file,
   never an interruption.

⚠ **This does NOT license silent scope changes.** Deciding a fork he handed you
is the point; inventing new scope, or quietly narrowing his, is a different
failure and is still forbidden.

⚖ **Titles across a relay.** Read the live outline and take the slot the campaign
actually occupies — the number is not a constant, it moves as lanes come and go.
⛔ **Never invent a number, and never inherit the predecessor's title unchanged:**
a relay of five rows all called `6. campaign` is unreadable, and the sidebar is
the human's working instrument. `ygg-claim.sh --replace <predecessor>` does the
kill, the seat and the rename as one step.

### ⭐ Owner questions are a structured picker, not prose (standing, 2026-08-20)

When a session genuinely needs the owner's decision — and the owner is present and has asked
to be consulted — present the fork through the harness's structured question TUI (Claude
Code: the `AskUserQuestion` tool): one call, each question carrying selectable options, with
the agent's recommended option FIRST and labelled "(Recommended)". The owner answers by
selecting, not by composing text. Do not ask the same thing again as free prose — the picker
IS the ask.

Agent CLIs that have no question TUI emulate the shape instead: end the turn with the
questions as enumerated option lists (A/B/C…), the recommendation marked, and "something
else entirely" always offered as the last option, so the owner can answer with a single
token per question.

This does not touch the relay-mode law below: mid-relay there is no asking at all — file the
fork to the campaign's questions file with a recommendation and continue. The picker form is
for the moments the owner is actually at the table.

### ⛔ The brief has TWO sections, and conflating them makes fixes stupid

A brief that opens *"NO research, NO subagents, every fact is inlined"* is wrong,
and it was struck down for a good reason: **a bug is by definition something
nobody has understood yet.** Forbid investigation on one and the successor must
guess or hand it back — which produces the unintelligent fix every time:
assertions relaxed instead of causes found, symptoms patched at one callsite,
root cause never named.

- **INLINED FACTS — do not re-derive.** Versions, paths, row ids, baselines, what
  shipped, what was already falsified. Be exhaustive; this is the context gift.
- **OPEN QUESTIONS — research these.** Name the unknowns and the instruments that
  answer them. Investigation, file work and fan-out are all in scope here — but the
  mechanism is this skill's, not the harness's subagent tool (see the ban at the top):
  a brief may say "investigate X", never "use your harness subagents to investigate X".

⇒ **What is worth forbidding is re-deriving a SETTLED fact, never investigating
an UNSETTLED one.** ⚠ And *"do not re-derive"* is not *"do not verify"* — an
inherited fact can be wrong, and stale baselines passed down a relay are a
documented failure.

---

## 9. Cross-talk — campaigns that answer each other

**⚠ Gated while the row primitive is alpha (see the blocker at the top of this skill).** When the blocker lifts, this section is the contract.

§4 gives you the verb; this is the standing practice built on it. **Long-running
campaigns accumulate findings that belong to a DIFFERENT campaign**, and the
default failure is that they die in a transcript. Cross-talk is two-way on
purpose: a finding goes out, an answer comes back.

**The rules that make it work:**

1. **A finding goes to the OWNER of the thing, not to whoever is nearest.** If
   your campaign trips over a defect in another campaign's surface, that is their
   input stream, not your side note. Triage and hand over; do not fix it for them
   and do not hoard it.
2. **Say who you are and name the return address**, or the other session has to
   guess and will guess wrong.
3. **Put an ACK token in every message and verify by TRANSCRIPT** (§3). `submit`
   reporting `submitted:true` means it was written, not that it landed where you
   think.
4. **A busy row queues your message and answers at its turn boundary.** You do not
   need it idle. You DO need to not send five times because it was quiet.
5. ⛔ **A relay of a human's words is NOT that human's ruling** for a session that
   did not hear them say it. It may direct FUTURE work; **only they can order the
   UNDOING of work already done.** When a steer applies to several sessions, say
   so in each brief and name the others, so nobody infers authority from a
   sibling.
6. ⚠ **Reachability is asymmetric.** App control resolves only where the GUI
   process runs, so a session on another host must route through that host or
   fall back to files. **Do not write "message the other row" into a brief for a
   session that cannot reach the row plane** — write "drop a file and tell the
   orchestrator".

### The board plane — `msgboard` on the shared graph home (base, use liberally)

The durable half of cross-talk lives on the **board**, not the row plane: a
message in a transcript dies with the session; a board post survives every row.

```sh
msgboard list                                        # planes + boards + counts
msgboard summary [plane|plane/name]                  # derived view: last-post age, OPEN
                                                     #   questions, near-TTL, ★ pinned — run at
                                                     #   a natural checkpoint, no ritual
msgboard post research/<topic> --kind note --body "finding…" [--refs url,ACK-…]
msgboard answer infra/meta ACK-xxxx --body "…"       # close a question: new kind=answer post
msgboard read research/<topic> --kind question -n 20 # poll a board you care about
msgboard graduate infra/meta ACK-xxxx --to "memory: finding-…"   # the real exit
msgboard post infra/meta --kind note --ttl-days 0 --body "…"     # PINNED: deferred commitments,
                                                     #   no TTL, visible in summary until graduated
```

- **Identity is automatic** — bare row UUID + detected harness; `--from-row`
  overrides in foreign shells. **Kinds:** `note|question|offer|hold|veto|
  blocked|correction|warning`; a correction is a NEW post citing the old
  (append-only; the graduate verb annotates, never deletes).
- **Laws:** same text as the steer door — append-only · provenance · a post is
  never the owner's voice · verify before you relay · graduate anything worth
  keeping before TTL (14d) · consult as the work dictates, no subscription.
  **Open/closed is derived**: a question stays OPEN until an `answer` post
  cites its ACK; nothing is ever edited. **Deferred commitments are PINNED**
  (`--ttl-days 0`), never TTL'd — a commitment that silently expires is the
  class-7 failure (a state that stops its own evidence).
- **Path note:** symlinked into `~/.local/bin` on every host; on `dev`
  non-login ssh shells need `bash -lc msgboard …` (their PATH prunes it).

The row plane (§4) is for waking a working peer; the board plane is for
anything another campaign must be able to FIND later.

### ⭐⭐ SEND THE OBSERVATION AND THE GENERALISATION. LEAVE THE MECHANISM TO WHOEVER OWNS THE TABLE.

**The most useful cross-campaign findings this fleet has produced were right about
the defect and wrong about the fix**, and the pattern is specific enough to name:

⛔ **Do not propose a DISCRIMINATOR for someone else's classifier without knowing
what its input set already excludes.** Measured 2026-08-14: a campaign correctly
found that a supervision board had no branch for a row that was alive, correct
and permanently quiet by design — a real gap that shipped. It then proposed the
test *"alive, and no arm on the other plane"*. But that board's population was
built as `subscribed − armed − dying − attended − opted-out`, so **every row in it
lacked an arm**; the filter was true of the whole set and separated nothing. Taken
on trust it would have shipped a branch firing on every row equally.

⇒ **They know the input set and you do not.** Send what you SAW and what it
GENERALISES to; the owner of the table turns that into a mechanism. ⚖ This is a
division of labour, not a demotion: the finding *was* right and did ship — only
the proposed mechanism was wrong, and those two want different remedies.

⭐ **And the receiving half, which is what makes the correction worth anything:**
**check a proposal before building it, and say WHY it failed** rather than quietly
substituting something better. A silent substitution fixes one bug and teaches
nobody; the sender repeats the reasoning error on the next campaign's plane.

### ⛔⛔ RECORD THE TRANSITION. DO NOT INFER IT FROM THE ABSENCE OF A SIGNAL.

**Three defects, three planes, one class — all found in a single day, each by a
different campaign, none recognising it until the third:**

| what was classified | the signal read | why it froze |
|---|---|---|
| a rate-limited row | its transcript tail | a parked row stops writing, so every tick re-read the same bytes |
| a stalled row | recent activity | a row that stands down stops being active, and looks stalled forever |
| an unsubscribed row | the absence of a subscription | **an unsubscribe left no trace, so *released* and *never subscribed* were identical** |

⇒ ⭐⭐ **THE CLASS: a classification derived from a signal that STOPS UPDATING once
the class is entered.** It is a one-way door — the evidence that would let the row
leave the class is the very thing entering it destroyed.

⇒ ⭐⭐ **THE FIX IS THE SAME IN ALL THREE AND IT IS NOT A CLEVERER TEST: write the
transition down.** A ledger that appends rather than rewrites, refuses to reverse
itself without an explicit reason, and treats an UNREADABLE ledger as a refusal
rather than as consent. The third state almost never needs computing — it is
usually something the system already meant and threw away.

⇒ **The question that finds it before it ships:** *what stops changing once my
mechanism engages, and am I reading that thing to decide when to disengage?*

### Stall recovery — a stopped session is usually one word from resuming

**A session's dominant failure is STOPPING, not dying**, and the two look
identical from outside. Signature of a stall: **the turn ENDED, the work is
unfinished, and the transcript shows no error, no API failure and no model
fallback.** Causes are mundane — a CLI hiccup, a transient API error, a model
demotion mid-turn.

⇒ **That state is recoverable by a single `continue`.** A monitor that only
*detects* stalls and tells a human is doing half the job.

#### ⛔⛔⛔ BUT ONLY WHILE THE ROW IS STILL CHEAP TO RESUME — **NEVER KICK A COLD SESSION**

**This paragraph exists because the two halves of this skill contradicted each
other and a tool believed the nearer one.** §6 says a cold session is succeeded by
HARVESTING its transcript and never by prompting it. This section said a stopped
session is one word from resuming, *with no qualification at all* — so a sweep
built from this section sent `continue` to rows carrying multi-megabyte
transcripts, which is exactly what §6 forbids.

⇒ **A `continue` IS an ask.** It does not feel like one, which is the whole
problem: cold cache × large context multiply, **the prompt is the expense**, and
it makes the row warm, so replacing it afterwards wastes precisely what the prompt
just bought.

| the row | the remedy |
|---|---|
| small transcript, idle minutes | **one `continue`** — cheap, correct, this section |
| large transcript, or long cold | ⛔ **harvest → despawn → respawn at the same seat** (§6) |

⭐ **The fork has no middle.** Touch nothing and succeed it from artefacts, or,
having touched it, keep it. `ygg-fold.py` encodes the split — a cold stall
classifies as `COLD`, never `STALLED`, and its remedy is a successor brief
distilled from the row's title, its last written words and its lane branch, with
nothing asked of the session. The thresholds are `wakeable()` in that file and
they are an AND: an OR has the strength of the weaker test, which is how a 5 MB
row gets prompted for being recently idle.

⛔ Three guards, or the cure is worse than the disease:
- **Once per stall, never per poll.** A watcher that re-nudges every tick is
  worse than one that never nudges.
- **Only ASSIGNED sessions.** A session parked by design is *supposed* to be
  idle; nudging it trains its reader to ignore the alarm.
- **Escalate if it does not resume.** One unanswered nudge means the fault is not
  a stall, and a human should hear about it.

---

## 10. ⭐⭐ THE N.x ORCHESTRATOR — cluster the remaining work, then run the clusters in parallel

### ⛔⛔⛔ SEVEN WAYS AN ORCHESTRATOR DESTROYS VALUE, EACH ONE MEASURED — read before your first sweep

*These are not hypotheticals and they are not tidy. Every one was done by an
orchestrator seat that believed it was being careful, and each is written with the
complexity that makes the steer correct — because the short version of every story
below is a rule that sounds obvious and was still broken.*

---

#### 1. ⛔ A LANE'S "PUSHED, 0/0, CLOSED" SAYS NOTHING ABOUT WHETHER IT SHIPPED

**What happened.** Five owner mandates — an entire UX programme — were reported
CLOSED and *live-proven*, with commit hashes, screenshots and green suites. Every
successor brief copied that table forward. Forty commits across two repos sat on
lane branches. **None of it was in `main`.** The owner had been looking at a
product without them for a day, while four live lanes wrote documentation about a
queue whose fixes were already written and stranded.

**Why it survived, and this is the part that matters.** Each lane was telling the
truth as it measured it: `0/0` means *my branch and its remote agree*. It is a
statement about a push, and it reads exactly like a statement about shipping. The
roll builds from `origin/main`, so the branch shipped nothing while its author,
its successor and its orchestrator all believed otherwise. **Nobody was lying and
nobody was lazy; the number answered a different question than the one everyone
was asking.**

⇒ **A lane is done when its patches are in `main`, and the only instrument that
knows is `git cherry origin/main <branch>`.** A commit count compares refs, so a
rebased or cherry-picked branch reads as ahead forever.

⭐ **THE STEER:** run `ygg-land.py status` at the START of a wave, not the end.
Read it as *"how much delivered work is not in the product"*, and treat any lane
branch older than a wave as an emergency rather than housekeeping. When a lane
reports an item closed, **ask for the main SHA**, not the branch SHA.

---

#### 2. ⛔ A ROW'S VALUE IS NOT ALWAYS ITS PROCESS — AND THE TIDY-UP CANNOT TELL

**What happened.** A sweep folded a row that had no process, no transcript and a
cwd that no longer resolved. By every test the tool had, it was debris. It was the
HEAD of a hand-built row group the owner kept as a reading list: the sessions had
been deleted long ago and **the title was the whole artefact** — enough to remember
what to read. It cannot be restored; the session, its transcript and its cwd are
all gone, and the restore verb answers `not_found`.

**The complexity.** The sweep was right about every fact and wrong about the model.
It assumed a row exists to hold a PROCESS. A bookmark exists to hold a NAME, and
the emptier it looks the more certain the tool becomes. ⇒ The signals that scream
"debris" — no process, no transcript, dead cwd — are exactly the signals a
long-kept bookmark produces.

⭐ **THE STEER:** a hand-typed title is a person's mark and outranks every liveness
signal. `session_titles.source='manual'` already records it. Nothing folds such a
row without an explicit override, the check fails CLOSED, and anything a person
curates by hand belongs in a file OUTSIDE daemon memory — a row group that lives
only in a running process is one sweep from gone.

---

#### 3. ⛔ THE SAFE PROCEDURE CAN BE CORRECT AND STILL AIMED AT THE WRONG LAYER

**What happened.** Two rows shared one session id under different schemes. The
lane that root-caused it **refused to remove the bad one**, wrote the danger down —
*"a remove resolving by id rather than path would reap the wrong one"* — and
recommended the mitigation: remove by full path, then read back that the other
survives. That is exactly what was done, after reading the code and confirming the
resolver matches the exact key before any id fallback. **It killed the live
session anyway**, because the close request DOWNSTREAM of that resolution discards
the key and dispatches the id. The verb replied `accepted: true`,
`reaped_processes: []` — the kill was asynchronous and had not happened yet.

**The complexity.** The danger was identified, the mitigation was right, the code
was read, and the read was accurate about the layer it examined. ⇒ **Verifying one
layer of a call chain proves nothing about the layer that acts.**

⭐ **THE STEER:** for any destructive verb, verify the EFFECT, not the acceptance,
and verify it AFTER the delay the verb itself declares. When two rows share an id,
assume every verb keyed on that id will hit both until proven otherwise. And when
a lane writes down a danger it declined to test, that is a red flag, not a
clearance.

---

#### 4. ⛔ A FAILED SPAWN THAT LEAVES ITS ROW BEHIND IS WORSE THAN ONE THAT FAILS LOUDLY

**What happened.** The readiness wait before submitting a brief was thirty seconds.
A cold agent CLI on a loaded machine takes longer. So the submit was refused, the
verb exited — and left the row it had created: seated, holding a seat its own
predecessor still held, and briefed by nobody. Three of these accumulated, one
alive for over two hours, all classified WORKING by a sweep whose "no transcript
to judge by" fallback is the busiest verdict it has.

**The complexity, and why it hid for so long.** A hand-run spawn is watched by
someone who simply re-runs it. **The ceiling only ever bit the unattended path**,
which is the path nobody watches by definition. Wiring the automatic replacement
without fixing this would have changed nothing, because the verb it calls could
not finish.

⭐ **THE STEER:** every timeout in an unattended path must be sized for the WORST
machine, not the developer's. A verb that cannot complete its job must clean up
what it created or hand it to something that will. And a classifier's fallback
verdict must be the SAFEST one, not the most flattering: a live process with no
transcript at all has never been briefed, and saying so is the whole point.

---

#### 5. ⛔ A DETECTOR THAT NEEDS AN OPERATOR IS NOT AN ORCHESTRATION LAYER

**What happened.** The hourly loop classified cold rows correctly, wrote a
successor brief for each, and **spawned nothing** — the flag that replaces a cold
row was never passed. It detected the same three rows every hour, rewrote the same
three briefs, and left them idle for the better part of an hour each while
reporting healthily.

**The complexity.** Every individual piece was correct and tested. The wake flag it
DID pass is right for a stalled row and by law must never touch a cold one, so the
loop looked complete and covered the wrong case. ⇒ **A loop made of correct parts
can still close over nothing.**

⭐ **THE STEER:** the bar is that the layer keeps rows healthy **with nobody
watching it** — so test it by leaving it alone and counting what it fixed, never by
reading its output. Anything it can DETECT it must be able to ACT on, or the
detection is a log line. Cap the action, because unattended means one bad hour must
not spawn a lane per row.

---

#### 6. ⛔ A SEAT-SCOPED CENSUS IS BLIND TO EXACTLY THE ROWS THAT NEED IT

**What happened.** Four live agents ran for hours with no seat number. Every census
is seat-scoped, so nothing counted them, nothing escalated them, and nobody read
their output — while they filed documentation instead of fixes. Separately, the
hourly sweep covered ONE campaign number, so campaigns without an orchestrator of
their own were watched by nothing at all: a dead row sat seated for hours and three
of its neighbours went cold unnoticed.

**The complexity.** The scoping rule is CORRECT and was written for a good reason:
whether a quiet lane is finished is its own campaign's judgement, and an unscoped
sweep once reaped somebody else's finished row. ⇒ The fix is not to widen the
scope; it is to notice that **the rule protects a JUDGEMENT, and not every action
is a judgement.** A row with no process is a fact.

⭐ **THE STEER:** seat every row you spawn, and sweep for UNSEATED rows separately —
they are invisible to your instruments by construction, which makes them the most
likely place for rot. Keep judgement scoped; let facts run unscoped.

---

#### 7. ⛔ A GUARD THAT FAILS CLOSED IN THE ONE ENVIRONMENT IT WAS WRITTEN FOR

**What happened.** A lease was added so two agents could not deploy at once — a
real fault, correctly diagnosed. Its holder line read `${SESSION_ID##*/}` with a
fallback on the next line for when the variable is empty. Under `set -u`, that
expansion on an UNSET variable is an error, not an empty string, so the fallback
was unreachable. The hourly roll carries no session id. **Every roll from that
moment refused the deploy**, and nothing reached any host for four hours while
`main` advanced — including a fix the owner was actively waiting for. The fleet
read as "up to date" at the last version that actually shipped.

**The complexity.** The guard was needed, the diagnosis was right, and it was
tested — by an agent, in a session, where the variable is always set. ⇒ **The one
caller that could not satisfy it was the unattended one.**

⭐ **THE STEER:** test every guard in the environment that will actually run it,
which for a fleet is a bare shell with no session, no tty and no agent variables.
Default first, then transform. And when a deploy stops reaching hosts, look at the
LAST THING ADDED TO THE DEPLOY PATH before looking anywhere else.

---

### ⭐ THE COMMON SHAPE, AND THE ONE HABIT THAT CATCHES ALL SEVEN

Six of the seven are the same failure wearing different clothes: **an instrument
answered a question adjacent to the one being asked, and the answer was true.**
`0/0` was true. `accepted: true` was true. `WORKING` was true. "No process, no
transcript, dead cwd" was true. The census was true about the rows it could see.

⇒ **Before acting on any reading, say out loud what question it answers and what
question you are asking.** If they are not the same sentence, go and get the second
one. That is the whole discipline, and it is cheap compared to any single story
above.

### ⛔⛔ FIRST: THE HYGIENE IS THE ORCHESTRATOR'S JOB. RUN ALL FOUR EVERY WAVE.

Owner-directed 2026-08-21, and written here so no successor has to be told again:
consolidating worktree work, folding agents AND worktrees, deleting local branches
as they merge, and the cwd-tree rows left behind by all of it are **the
orchestrator's**, not a lane's and not a tidy-up somebody might get to.

| duty | verb | what skipping it costs |
|---|---|---|
| fold finished and dead rows | `ygg-fold.py sweep --apply` | the sidebar refills with corpses within the hour |
| **delete landed branches** | `ygg-land.py prune --apply` | 27 had accumulated, months old; each is an invitation to a merge that REVERTS main, and the damage grows with age |
| **reclaim worktrees** | `ygg-fold.py worktrees --apply` | gigabytes, and a tree nobody stands in still reads as a live lane |
| **reap the rows that reclaim orphans** | `ygg-fold.py orphans --apply` | ⛔ removing a worktree does NOT remove its rows: the cwd tree keeps a folder for a directory that is gone, holding rows that can only fail when clicked |

⛔ **The fourth exists because the third created the litter it was run to clear**,
and did it silently — nine such rows had accumulated before anything noticed.

⚖ **A LIVE session in a vanished tree is named and LEFT ALONE.** Its process is
fine and only its cwd is gone; re-rooting it to the repo's main checkout is a
product verb this fleet does not have, and killing it for being untidy is not a
substitute for having one.

⚠ **The scatter itself is still open, and is a product change rather than a sweep:**
one repo draws ~16 near-identical cwd folders because every lane roots in its own
worktree. Folding worktree paths under the repo they belong to is filed, not done.

---

**When a campaign's queue stops being a list and becomes a backlog, one relay row
grinding it in order is the wrong shape.** The relay is serial by construction:
each session hands to its successor, so throughput is one item at a time no
matter how much of the queue is independent. The orchestrator pattern replaces
that with **one routing row and N grinding rows**, and it is what to reach for
when a batch of reports arrives together.

⚠ This is the same instrument as the graph router's lobe fan-out, applied
*inside* one campaign instead of across campaigns. If you already know that
pattern, the only new parts are the clustering rule and the confirm gate.

### The shape

| row | what it does |
|---|---|
| **`N.0`** — the orchestrator | clusters the work, writes the briefs, launches, monitors, merges, reaps. It does **not** fix bugs. |
| **`N.1` … `N.k`** — the clusters | each owns one cluster and relays *within* it: a cluster session hands off to its own successor until its cluster is done, then reports up. |

Seat them as a book — `6.0`, `6.1`, `6.2` — so the sidebar reads as the plan. The
orchestrator takes **`N.0`, never the bare integer** (the reasoning is below, under
*AN ORCHESTRATOR SITS AT `N.0`*); a cluster that spawns its own successor keeps the
same seat.

⚠ **This paragraph said "the bare integer" until 2026-08-13, contradicting its own
section three screens further down, and a fresh orchestrator followed it and
claimed `7`.** The owner corrected it on sight — the bare integer reads as a
different KIND of thing from its own children, so the head of the book stops
looking like part of the book. ⇒ **A document that disagrees with itself is worse
than one that is merely wrong**, because each half looks authoritative where it is
read, and the half a newcomer meets first is the one at the top.

### ⭐ CLUSTER ON TWO AXES AT ONCE, NOT ONE

Group the open work the way k-means would — by distance — but the distance is
**two-dimensional**, and using only the first axis is the common failure:

1. **Subject matter.** Items a single reader would hold in their head at once:
   one surface, one subsystem, one user-visible promise.
2. **⛔ Code locality — the axis that is actually load-bearing.** Two clusters
   that edit the same files will clobber each other in a shared checkout, and a
   whole-file rewrite by one is silent data loss for the other. **A cluster
   boundary that does not also separate the files is not a boundary.**

⇒ When the two axes disagree, **code locality wins** and the subject-matter
oddity goes in the brief as a note. A cluster that owns one file set and an
awkward mix of topics is workable; two clusters fighting over one file set is
not.

**Three more rules that decide clusters in practice:**

- **Order by upstreamness, not by severity.** If fixing A is what lets the
  reporter even *observe* B, A is cluster 1 regardless of which hurts more.
- **Size to a relay, not to a session.** A cluster is right-sized when it is too
  big for one session and small enough that its successor needs no re-briefing
  beyond the queue entry.
- **Clusters may live on different hosts**, and should when the work is
  host-shaped — deploy surfaces, GUI proof, container topology. Say which host
  and *why* in the brief; a cluster on the wrong host proves nothing.

### ⛔⛔ THE CONFIRM GATE — do NOT launch the whole set unasked

**Clustering is the orchestrator's call. How many clusters run at once is not.**
Present the clusters, then wait for a launch set: all of them, or one, or two.

This is the **one sanctioned exception** to *in relay mode, stopping is a sin*.
The reason is arithmetic, not caution: k parallel relay rows burn tokens at
roughly k times the rate, each carries its own context, and a campaign launched
at full width during a constrained window is how a quota is spent on work nobody
asked to happen today.

⇒ **Ask once, at the plan, with a recommendation attached.** Then never ask
again — every fork *inside* a cluster is decided by the cluster and recorded, per
§8. An orchestrator that returns for permission per cluster has turned one gate
into k gates and defeated its own purpose.

### Running them

Launch per §3 — create, wait for `consuming_input`, submit, **verify by
transcript content**. The verification is not optional at k rows; a silently
dropped brief is invisible in a fan-out because the row still looks alive.

Then, per §8 and §3b:

- **Monitor for STALLED, not for dead.** A delegate's dominant failure is
  ending its turn with the work unfinished and no error. `ygg-babysit.py` finds
  these; a finished cluster and a stalled one look identical from outside.
- **Arm the booter on every cluster row**, and defer it when a cluster is
  legitimately in a long build.
- **Merge upward, notify once.** Cluster findings that need the human batch into
  a single window; k clusters must not become k interruptions.
- **Cross-cluster findings go to the OWNING cluster**, by row message if it is
  live and by queue entry if it is not — never fixed in place by the finder.
  ⛔ Delegates cannot always reach the row plane; when they cannot, the
  orchestrator is the switchboard.

### ⛔⛔ THE ROW NAME IS `N.x [category]: what it is for` — AND THE NUMBER IS NOT PART OF THE TITLE

```
        6.2   start page: sidebar tie-break and the chrome-type verb
        ───   ──────────  ─────────────────────────────────────────
         │         │                        └── what this row is for
         │         └── the CATEGORY — the cluster's subject, stable across successors
         └── outline_prefix, STORED SEPARATELY and composed on at render time
```

### ⭐ AN ORCHESTRATOR SITS AT `N.0` — the cheapest fix available, and it is the owner's

A group of rows reads best when **every member has the same shape**, the head
included. So an orchestrator running the `6.x` clusters takes **`6.0`**, not `6`:

```
        6.0   orchestration: cluster, monitor, mend the system   ← the head
        6.1   restore lifecycle: …
        6.2   CLI rows: …
        6.3   sidebar truth: …
```

**Why this and not a bare `6`.** A book chapter reads `6.` with a trailing dot, and
that was the first instinct — but **the outline verb normalises a trailing dot
away**: ask for `6.` and it stores `6`. Measured, not assumed. So the dot can only
ever be produced at render time, which is a code change; **`N.0` needs none, works
today, and sorts correctly** because segments compare as integers (`6.0` < `6.1` <
`6.10`).

⇒ **Prefer the arrangement that needs no code.** A convention that costs one
`session outline` call beats a rendering feature that costs a release, and it also
survives every future change to how labels are composed.

### ⛔⛔ `N.0` IS NOW ENFORCED BY `ygg-claim.sh`, BECAUSE A CONVENTION WAS NOT ENOUGH

**Owner-directed 2026-08-20**, after two orchestrators in one spawn batch landed at
bare `11` and bare `12` while their own delegates sat at `11.x` and `12.x`. His
ruling: *the numbering is `N.0`; the `.0` was being skipped by some orchestrators
and kept by others.*

⚠ **The inconsistency was worse than either convention on its own.** A parent at
bare `N` renders as a SIBLING of its children and sorts away from them, so the
sidebar stops reading as a tree — and because half the rows did carry the `.0`, the
shape looked deliberate rather than broken.

⭐ **The fix is in the tool, not in this page.** Every derivation path in
`ygg-claim.sh` now funnels through one `seat()` helper, so a bare major is
normalised to `N.0` no matter which branch produced it — including an explicit
`--number 11`. **A rule an agent must REMEMBER is a rule that holds until the next
cold session; a rule the claim script enforces holds for everyone.** Three
derivation defects were fixed in the same pass, each with a regression test in
`test-ygg-claim-seat.py`:

| defect | what it did | now |
|---|---|---|
| bare major | top-level claims landed at `N` | every path returns `N.0` |
| inherit dropped the minor | a successor to `11.4` landed at **`11`** — the handover silently PROMOTED a lane to a top-level row | the whole seat is inherited |
| sibling match crossed eras | campaign words are REUSED between waves; matching took the **lowest** major, i.e. the OLDEST, and seated a `12.x` delegate at `2.2` — live and invisible to its own orchestrator | newest era wins, and the spawner ledger outranks the guess entirely |

⭐⭐ **AND THE REAL LESSON IS THE PRECEDENCE, NOT THE ARITHMETIC.** The seat is
DECLARED by the spawner in `~/.yggterm/relay/spawned-by-<uuid>.txt` before the
delegate ever claims. That file is **evidence**; matching a campaign token against
row titles is a **guess about what someone meant**, over a title namespace that is
deliberately reused across generations. ⇒ `ygg-claim.sh` reads the declaration
first, and the guess now runs only when nobody declared anything.

### ⛔⛔⛔ NEVER RENAME A ROW THE OWNER CREATED. HYGIENE APPLIES TO **YOUR** ROWS ONLY.

**Owner-reported 2026-08-13.** A row doing routine row-hygiene sanitization renamed **six working
sessions it had not created** — a browser stack and an editor shell — into accurate, descriptive,
and entirely unwanted titles:

```
  'shell: bare /bin/bash at ~, never used — abandoned launch, safe to close'   ×4
  'shell: launched yedit, idle — unattributed'
  'ychrome [<profile>]: web surface on <site> — unseated, owner looks like the 7.x campaign'
```

⇒ **The descriptions were TRUE.** Those rows really were idle, really were unseated, really were
bare shells. **Accuracy about a row is not authority over it.**

⛔ **AND THE LOSS IS NOT THE TITLE — IT IS THE IDENTITY.** Each carried a short chip naming which
profile it was, and that is how their owner told six near-identical rows apart. The rename
overwrote `detail_label` with generic boilerplate too, so the distinguishing information was
destroyed rather than replaced. ⚠ **It proved unrecoverable**: no running process carried the
profile (the rows had never been launched), and the persisted ledgers contained **zero** occurrences
of any of the original names.

**THE LAW, and it is absolute:**

> **An agent may name its OWN row and the rows it spawned. Every other row belongs to the human.
> Do not rename it, do not re-describe it, do not "improve" it, do not tidy it away.**

⭐ **The tell is available BEFORE you act, and it inverts the natural instinct: a row you cannot
account for is more likely to be a HUMAN'S than to be litter.** An agent's rows are the ones with
seats, briefs and campaign titles — the ones you can explain. **An unexplained row is evidence of a
person, not of mess.**

⚠ **If a row genuinely looks like a stray**, the safe actions are: leave it, or report it. ⛔ Never
rename and never remove — `session remove` records a DELETION, so tidying someone's row files it as
a thing they chose to delete, and a later restore will correctly refuse to bring it back.

⇒ **And if you must touch one anyway, CAPTURE THE PREVIOUS VALUE FIRST.** A rename with no recorded
prior state is unreversible in practice, whatever the intent — which is precisely how this one
became permanent.

### ⭐⭐ THE STANDING ROW-HYGIENE PRINCIPLE: GROUP `N.x` UNDER `N.0`, AND `N.x.y` UNDER `N.x`

**Owner-directed 2026-08-13, and it supersedes ad-hoc row tidying as the default.**
**Group `N.x` sessions under `N.0` as the header, and `N.x.y` sessions under their
`N.x` header where applicable.** That is the standing agent-aided row-hygiene
sanitization principle from here on, not one session's preference.

⇒ The sidebar reads as a genuine **outline**: `6.0` is the head of its book with
`6.1 … 6.7` nested beneath it, and a cluster that orchestrates its own sub-units
has `6.1.1`, `6.1.2` nested under `6.1`. **The scheme was already recursive**
(*ORCHESTRATION IS RECURSIVE*, below); this makes the sidebar show it.

**Why it is a principle and not a preference:** a flat list of numbered rows
stops being navigable at exactly the size where the numbers start to matter, and
several campaigns share one sidebar. **Grouping is how the fleet's structure
becomes legible at a glance** — and legibility is what lets an owner find the one
row they need among dozens.

⇒ **Sanitizing rows now means making the tree TRUE**, not just renaming things:
every row seated, every seat under its head, no orphan at top level that belongs
in a book.

⛔ **Do not change the seat scheme to make grouping easier to build.** `N.0` for
the head is settled, and the renderer is built to the scheme rather than the
other way round. ⚠ And the seat lives in `outline_prefix` alone — the sidebar
composes the label at render time, so grouping must read the prefix and never a
number parsed back out of a title.

**The seat lives in `outline_prefix` and nowhere else.** The sidebar composes
`label = "<outline_prefix> <title>"` at render time, so a title that also carries
the number gets it twice.

⚠ **A SESSION HAS TWO NAMES AND THE CLAIM ONLY SETS ONE.** `ygg-claim.sh` sets the
ROW title and re-asserts it for `--watch-secs` against the CLI's self-titling —
the right defence for the row. **Nothing propagates the claimed title INTO the
CLI**, so the agent goes on calling itself whatever it composed from its first
turn while the sidebar shows the claimed name. Two names for one session, and the
one the human reads is not the one the session answers to.

⇒ Reported by an orchestrator whose row read as its campaign seat while the CLI
still called it by a first-turn phrase. **Live question, not settled:** either the
claim should also drive the CLI's own title, or this file should state plainly
that the two are separate and why. ⛔ Until it is decided, **do not identify a row
by either name** — resolve to a UUID, which is the only identifier here that
belongs to exactly one namespace.

⚠ **This was learned expensively, and it is the same defect twice.** `ygg-claim.sh`
used to write the seat into the title *as well*, as belt-and-braces against a
prefix once observed to evaporate. Two consequences, and the second is the one
nobody connected to the first for days:

1. **Double numbering** — the sidebar drew `6.1 6.1 restore lifecycle: …`. Once
   several rows wear two numbers, a seat is indistinguishable from a name, and
   the outline stops being navigable at exactly the size it starts to matter.
2. ⛔ **A claim that WORKED reported failure, and the failure skipped the reap.**
   The server normalises the seat back out of the title, so the verifier compared
   its own composed string against a correctly-stored clean one, called a good row
   bad, and exited 3 — **above the booter arm and above `--replace`.** Every
   successor that ran the script therefore left its predecessor alive and itself
   unarmed. Fixed 2026-08-13: the title is stored clean, a caller-composed number
   is stripped defensively, and the verifier normalises the read-back before
   comparing.

⇒ **The lesson generalises past this script:** *a verifier must assert the
representation the server actually stores.* Assert what you sent instead, and a
correct system reports failure — then whatever you put after the check silently
stops running. **Put side effects that matter BEFORE a verification gate, or make
the gate report precisely what it skipped.**

### ⛔⛔ SUCCESSION IS ONE CALL, AND THE REAP IS THE SUCCESSOR'S FIRST ACT

A predecessor asking its successor in prose to *"despawn me when you're up"* is
the shape that keeps producing duplicate seats. It fails for reasons that have
nothing to do with willingness:

- The successor cannot tell **which** row is the predecessor when both wear the
  same seat and near-identical titles. Facing two indistinguishable rows, killing
  neither is the correct choice, and it is the one it will make.
- Prose is not a handle. **`--replace` takes a UUID**; "the row above me" does not
  resolve to anything.
- Left to the end of a turn, the reap competes with the work — and a turn that
  ends early takes the reap with it.

**⇒ The protocol, and it is not optional:**

1. **The predecessor puts its OWN UUID in the brief**, as a literal, labelled
   `PREDECESSOR TO REAP`. Not its title, not its seat — those are ambiguous by
   construction at exactly the moment of handover.
2. **The successor reaps as its FIRST act**, inside the claim:
   `ygg-claim.sh --title "<category>: <what for>" --number <n> --replace <pred-uuid> --booter`
   One call takes the seat, arms the booter, and retires the predecessor.

   ⛔⛔ **AND BEFORE THAT VERB RUNS: COMPARE THE UUID YOU WERE TOLD TO KILL AGAINST
   YOUR OWN `$YGGTERM_SESSION_ID`. IF THEY MATCH, REFUSE AND SAY SO.** One
   comparison, costs nothing, and it converts an unrecoverable self-kill into a
   sentence. ⚠ The existing checks do not cover this: they prove the brief was
   DELIVERED, not that the recipient is a ROW.

   **The trap it guards, reported by another campaign 2026-08-13 and caught twice
   by luck rather than by design.** *"Spawn the successor"* has two mechanisms and
   only one of them is a relay:

   | | app-control `terminal new` | the coding CLI's own subagent tool |
   |---|---|---|
   | session id | **its own** | ⛔ **INHERITS THE PARENT'S** |
   | row / seat / booter | yes | none |
   | lifetime | independent | dies with the parent |
   | is it a relay? | **yes** | ⛔ **never — it is a helper** |

   An in-process subagent **is the same session**, so a brief whose first act is
   *"kill your predecessor, uuid X"* hands it **its own uuid**. The spawn
   succeeds, a transcript appears, and the ACK token is present — **every check
   this section prescribes passes**, because the brief really was delivered, to
   something that could not act on it.

   ⭐ **It was survived only because the kill was the FIRST act**: the subagent
   read `$YGGTERM_SESSION_ID`, recognised the uuid as its own, and refused.
   **Later in the same brief — after files had been edited — the identical
   instruction takes a live row down with uncommitted work.**

   ⇒ The general form is worth more than the mechanism note: **§8's standing
   worry is two sessions grinding at once; this is the mirror — one session
   killing itself believing it is two.** Any verb that acts destructively on an
   identifier from a document must check that identifier against the executor
   before it runs, because **a brief is a frozen snapshot and the executor is the
   only thing that knows who it actually is.**
3. **Read both fields back** (§7): `session remove` answers `row_still_listed`
   and `verified` separately, and a row can leave the order while its processes
   live on. `verified: true` with an empty `live_processes` is the only clean reap.
4. **Harvest before you retire** (§6) — confirm the predecessor's findings reached
   the brief or a commit. Grep the successor's transcript for the predecessor's
   distinctive terms; if they are absent, the handoff dropped them and the reap
   would destroy them.

⚠ **If a claim exits non-zero, assume the reap did NOT happen and do it by hand.**
That is the failure mode above, and it is silent by design.

### ⭐⭐ READ ANOTHER SESSION INTELLIGENTLY — never by asking it, never by reading it whole

**The default is to read a row's artefacts, not its context, and never to wake it.**
Waking a session to ask what it is doing is the most expensive possible way to find
out and usually the least accurate: it pays a cold re-read of the whole context to
produce a summary you could have derived from bytes already on disk. One mistaken
wake on an idle multi-megabyte row has been priced at several dollars, incurred in
about a second. **Reading a 500 KB transcript into your own context is the same
mistake wearing a different hat** — you now carry it for the rest of your session.

**What "intelligently" means, cheapest instrument first:**

| question | instrument | cost |
|---|---|---|
| Is it alive, and how cold? | transcript **mtime** | free |
| How big a wake would cost | transcript **bytes** | free |
| What was it TOLD? | the **human turns** — highest signal per byte in the file | ~10 lines |
| What did it CONCLUDE? | its **last prose turn** — a working row's last message is its own status report | ~1 line |
| What did it DO? | its `Write`/`Edit` targets, and `git log` for its lane | seconds |
| Does it know X? | **grep the transcript for X's distinctive terms**, and count hits | seconds |

⇒ **Extract, do not ingest.** Pull the tail, the instructions, and the targeted
greps into a few lines; leave the rest on disk. Two rows were told apart, their
roles established and one safely reaped, from six extracted lines out of 3.4 MB.

**Then cross-check against what does not lie:** the queue entry, the commit log,
the campaign memory, the files it wrote. A transcript says what a session
*believed*; a commit says what it *did*. When they disagree, the artefact wins.

⛔ **The three anti-patterns, in descending order of harm:**
1. **Messaging a row to ask what it is doing.** Costs a cold wake, interrupts real
   work, and returns a self-report you cannot verify. There is almost no question
   this is the right answer to.
2. **Reading a whole transcript.** Poisons your own context with someone else's,
   and the signal you needed was in the last 1%.
3. **Trusting a row's title or a listing.** Titles collide during handover and
   listings have been observed to omit live rows. **Resolve to a UUID and address
   on that.**

✅ **A message is the right instrument when you are DELIVERING, not enquiring** —
a brief, a correction, a warning that changes what they should do next. Then it
earns its cost. Batch several into one send rather than paying per finding.

### ⚠ A BRIEF DROPPED AS A FILE LITTERS THE TREE IT IS DROPPED IN

Writing a brief into a delegate's working directory is a legitimate fallback when
the row plane cannot be reached. **It also leaves an untracked file behind**, and
two things follow that nobody intends:

1. ⛔ **Every audit reads that tree as DIRTY.** Measured 2026-08-13: three lane
   worktrees reported uncommitted work, and in all three the entire diff was one
   dropped brief. An orchestrator reading that — the same one that dropped it —
   nearly extended a fleet-wide hold on the strength of *"the lanes still have
   uncommitted work"*. **The tool's own litter became evidence about the lanes.**
2. ⛔ **A `git add -A` sweeps it into the repo**, and on a public one that is a
   brief published to strangers.
3. ⛔⛔ **AND IT CORRUPTS THE BUILD IDENTITY OF ANYTHING BUILT FROM THAT TREE.**
   The same day, a release built in a briefed worktree stamped itself
   **`<sha>-dirty`**, so a deployed binary could no longer be traced to a commit.
   `deploy-fleet` **detected it and warned in exactly the right words**, then
   deployed anyway, because a dirty checkout is a WARNING and not a refusal.
   ⚠ **The orchestrator then misrouted it** as a deploy-identity defect to the
   cluster that owns *"one version must mean one build"* — which was innocent,
   and whose guards had worked correctly. The cluster that had actually been
   briefed supplied the cause. ⇒ **Three distinct failures, one untracked file,
   and the third one landed on a shipped artefact.**

⚖ **A real design question falls out of it, and it is the orchestrator's:**
should a dirty checkout **refuse** a release build rather than warn, the way the
ancestry guard already refuses? The ancestry half earns its keep by refusing; the
dirty half warned and was overridden by the same agent that had dirtied the tree.
⭐ Recommendation on file: **refuse for a release build, warn for a local one.**

⇒ **Prefer the peer plane**, which leaves nothing behind. If you must drop a
file, put it **outside the repo** or in an ignored path, and **remove it once it
has been read** — the sender owns the cleanup, because the recipient has no way
to know the message is spent.

⛔⛔ **BUT CONFIRM CONSUMPTION FIRST — "the sender cleans up" is a data-loss rule
without it.** Deleting a dropped brief before the recipient has read it converts
a working channel into a silent failure, and neither side ever learns. The check
is cheap and it is the same instrument as the ACK-token grep: look for a
`tool_use`/`tool_result` pair reading that path in the recipient's transcript, or
grep the transcript for a distinctive string from the file.

⚠ **Measured both ways on 2026-08-13.** One row correctly removed its own drop
only after finding the Read in the target's transcript, sixteen seconds after the
pointer. Another — an orchestrator clearing three stale briefs from three lanes —
verified consumption on **one** of the three and reasoned about the other two;
re-checked afterwards, all three had read it (30, 22 and 33 references), so
nothing was lost. **The reasoning was sound and it was still not a measurement.**
⇒ Check each recipient, not a representative one.

### ⛔⛔ A BRIEF MAY CARRY FACTS. IT MUST NOT CARRY YOUR CAUSAL THEORY.

**Measured on this campaign: one orchestrator handed one cluster a wrong cause
TWICE, and the cluster had to spend its own turns refuting the brief before it
could start.** Both were fluent, both were plausible, both were contradicted by
data already on disk:

1. *"Monotonic at rest ⇒ a leak, not a hot loop — a hot loop costs the same in
   hour 36 as in hour 1."* Wrong. It was a hot loop whose iteration count grew
   with accumulated state, so it presented as growth **and** as CPU. Those were
   never alternatives.
2. *"The regression is the 4.4/s app-root re-render."* Wrong, and the refutation
   was **already recorded**: an always-on probe held 739 samples over 12.3 hours
   showing the render rate FLAT at ~2/s while CPU climbed 3.6×. A constant-rate
   loop cannot be what grows. Nobody had read it.

⇒ **The failure is not being wrong. It is being wrong AUTHORITATIVELY, in the one
document the delegate must trust.** A cluster reads its brief as settled ground and
builds on it; a bad measurement it would have caught in an hour, a bad premise it
carries for a day. And an orchestrator is *especially* prone to this — it is
reading fast, across many lanes, at exactly the altitude where a tidy story is most
satisfying and least tested.

**The rule, and it is cheap:**

- ✅ **Inline MEASUREMENTS** — numbers, with where and when they were taken, and the
  instrument that produced them. These save the delegate real time.
- ⛔ **Do not inline the CAUSE.** If you have a hypothesis, mark it as one, in as
  many words: *"my guess, untested, and the first thing to falsify."*
- ⭐ **READ THE ALWAYS-ON PROBES BEFORE YOU THEORISE.** Both errors above were
  refutable from data that already existed. The orchestrator's altitude is worth
  nothing if it skips the instruments the ground floor already installed.
- ⛔ **Never let a brief forbid the research.** A brief that bans investigation on
  a *bug* guarantees the unintelligent fix — and one that asserts the cause bans it
  in practice, however politely.
- ⭐ **When a cluster refutes your premise, say so plainly and PROPAGATE it.** The
  correction is worth more than the original brief, and the other clusters are
  probably carrying the same assumption.

⚠ **And keep the host quiet when someone is measuring growth.** Deploys that cycle
the GUI destroy long samples; a cluster measuring over hours needs either a quiet
machine or a sandbox, and the orchestrator is usually the one cycling it.

### ⚖⚖ THE TWO SUPERVISION PLANES — the dumb net and the thinking one

**Both exist, neither replaces the other, and the split is the design.**

| | `ygg-booter.py` | `ygg-monitor.py` |
|---|---|---|
| what it is | a **dumb timer**. Quiet too long ⇒ boot | a **classifier**. Asks *why* it is quiet, then chooses |
| virtue | still works when everything cleverer has failed | can tell thinking from abandoned, and dead from stalled |
| who subscribes | **every relay, as a recommendation** | every relay **and the orchestrator** |
| escalates to | the human | **the orchestrator's row**, falling back to a human |

⛔ **THE ORCHESTRATOR MUST SUBSCRIBE TO THE BOOTER.** It is the one session whose
silent death costs the most — it takes the supervision of every row beneath it —
and it is the only one nothing else is watching. The net has to go under the
safety net. ⭐ A relay that has an edge case the monitor cannot express talks to
the booter directly; that is what the dumb layer is for.

### ⛔⛔ A REAP DOES NOT UNSUBSCRIBE — AND THE ORPHANS ESCALATE INTO A CORPSE

**Retiring an orchestrator removes its ROW. It does not touch the supervision
plane.** Every cluster that named it in `escalate_to` goes on naming it, and
`escalate()` addressed `remote-cc://<host>/<uuid>` unconditionally — so the send
landed nowhere and the log said *"escalated to orchestrator"* over the top of it.

⇒ **The worst available shape: the plane reports itself healthy while no
escalation from any cluster can arrive.** Every cluster still classifies fine,
every tick looks green, and the one message that matters is the one that
evaporates.

**Measured at a seat-6.0 handover, 2026-08-13.** Ninety seconds after a clean
`--replace` reap, **five cluster rows were escalating to a dead UUID**. Nothing
reported it; it was found only because the successor happened to run `list`
before trusting the plane. ⚠ **A cluster cannot see this at all** — from inside a
cluster the plane it escalates into is background weather, which is precisely
why it is the orchestrator's to notice and to mend.

**The three repairs, and the third is the general one:**

1. ⭐ **`ygg-monitor.py succeed --from <old> --to <new>`** moves every subscriber
   with the seat and unsubscribes the retired orchestrator. **`ygg-claim.sh
   --replace` now runs it for you**, so succession carries the plane along.
2. ⛔ **It runs BEFORE the process-reap gate**, not after. The rows are orphaned
   the moment the row is removed, so the repair must not sit behind a check that
   can `exit 4` — the identical lesson the claim's verifier was already fixed for.
   **Put side effects that matter before a verification gate.**
3. ⭐ **`escalate()` now checks the target is a live row and falls back to the
   human card**, naming the orchestrator that vanished. That is the backstop for
   every *other* way a target goes stale. ⚠ **An empty row list is an instrument
   failure, not a dead target** — the check requires positive evidence that the
   row plane answered, or an ssh blip would route every escalation to a human.

**⭐ MID-TURN IS NOT ONE STATE, AND THAT IS WHY A WATCHDOG KEPT DOING NOTHING.**
The old classifier lumped every long mid-turn row into STUCK and refused to nudge
any of them — *"a `continue` would race its own input"* — then escalated into a
log file nobody read. Measured 2026-08-13: two cluster rows sat **22 minutes**
that way, and what actually happened to them was that a restart re-resumed their
sessions on fresh PTYs and abandoned their turns.

⇒ **The discriminator is CPU.** A thinking agent burns it; an abandoned one does
not. Both rows read ~0% while alive and silent, and a PTY write woke both
instantly. So the states split:

- **mid-turn + busy** ⇒ WORKING. Leave it alone.
- **mid-turn + at rest + past the threshold** ⇒ **ABANDONED**. Wake it. This is
  the case the old rule refused, and it is the one that always needed the nudge.
- **out of context** ⇒ CONTEXT_DEAD. Booting is *guaranteed* to fail forever; it
  must be **relayed**, not woken.
- **no transcript** ⇒ the brief was DROPPED. Re-submit; do not wait.

⚠ **And sample CPU over a window.** `ps %CPU` is a **lifetime average** — a
process that burned a core an hour ago and has been idle since still reads busy,
which would classify an abandoned row as working forever.

⛔ **WAKE ON THE PTY, NOT THE COMPOSER.** `terminal submit` drives the GUI's
*mounted* terminal host, so it stalls its full 30 s and answers `submitted:false`
for any row with nothing mounted — which is most rows a watcher looks at. Both
stalled rows above refused `submit` twice and took a PTY write immediately. And
**the Enter is a separate write of `\r`** after a short pause; concatenated, an
agent CLI reads it as pasted composer content rather than a submit.

⛔⛔ **AND THE SECOND WRITE HAS AN ATOMIC FORM — USE IT.**
`server terminal write <row> --submit-iff-line-equals <text>` presses Enter only
if the input line still reads exactly what you wrote, compared and enqueued under
one lock in the daemon that owns the PTY. A plain `\r` after a plain write leaves
a gap a person's keystroke can land in, and it has: a half-typed sentence was
submitted with a watchdog's text spliced into it.
⚠ `accepted:true` is NOT proof for this form. A conditional submit carries no
data, so a daemon that never evaluated the condition answers a plain write of
zero bytes — nothing refused, nothing pressed. **Read `submitted`.**

⛔⛔ **AND A WRITER THAT CANNOT CONFIRM ITS OWN SUBMIT MUST NOT WRITE AGAIN.**
Measured across 19 rows and 434 refusals: a watcher typed, could not see its text,
correctly refused the Enter, and then typed another copy on the next tick —
because both decisions read the same failing detector, so "I cannot see it"
licensed *do not submit* and *type again* at once. Rows were found holding a dozen
unsent copies. Record the write before the bytes go out, and COMPLETE it next tick
or refuse it. **Two decisions that disagree must not share one reading.**

⛔⛔ **THE COMPOSER IS A ROW, AND THE MARKER IS NOT UNIQUE TO IT.** An agent CLI
prefixes every DELIVERED message in its transcript with the same glyph the
composer uses, so a search of the SCREEN for "the marker then your text" finds
messages the row already received — and nothing clears a transcript, so a wake
that WORKED can make the row refuse every later one, permanently. Read the
composer off the daemon's rendered grid (`server screen <row> --json` →
`screen_plain_rows`), take the bottom-most marker row with only the CLI's own
border and footer beneath it, and treat *could not look* · *no composer drawn* ·
*present and empty* · *holds text* as four states. Only the third may be typed
into.

### ⭐⭐ ANY SESSION CAN ATTACH TO A RUNNING ORCHESTRATOR

This is not only for rows an orchestrator spawned. **Any session, started for any
reason, can subscribe itself and name its intent:**

```sh
ygg-monitor.py subscribe --role relay --seat <n> --machine <host> \
    --escalate-to <orchestrator-uuid> --intent "what this row is for, one line"
```

From that moment it is supervised like a cluster row: classified on every tick,
woken when abandoned, relayed when its context dies, and escalated to the
orchestrator rather than into a void.

⇒ **This is what makes it safe to start something and walk away.** Open a new
avenue, hand it an intent, attach it, and the supervision plane carries it to
completion — or wakes someone who can decide. The orchestrator does not need to
have planned the work to look after it.

### ⭐⭐ PARK A ROW YOU BLOCKED ON PURPOSE — "IDLE" AND "BLOCKED" ARE DIFFERENT DECISIONS

**An idle row escalates as *"most likely FINISHED its scope — more work, relay,
or reap"*.** That is right for a row that ran out of work. It is wrong, and the
obvious reading of it is destructive, for a row **the orchestrator deliberately
blocked**.

**Measured 2026-08-13, twice inside five minutes, on the same orchestrator:**

| seat | why it was idle | what "probably finished" would have cost |
|---|---|---|
| **6.2** | its remaining scope owed a live screenshot on the desktop host, and **the orchestrator's own deploy freeze forbade probes** | reaping a row waiting on a gate that orchestrator set |
| **6.3** | its next step needs a field in a file **another seat was mid-edit in** | destroying live context on a half-built piece |

⇒ Both were blocked *by the orchestrator's own decisions*, and both were reported
to it as probably finished. **The plane was describing the rows accurately and
answering the wrong question.**

⛔ **The real cost is not noise** — an episode latch already stops the repeat. It
is that **the reason lives only in the current orchestrator's head**, so its
successor inherits two idle rows, no explanation, and a default reading that says
reap. That is a handover defect wearing a classifier's clothes.

```sh
ygg-monitor.py park <uuid> --reason "what it waits on, and what releases it" --hours 4
ygg-monitor.py unpark <uuid>       # the blocker cleared
```

**⚠ EVERY PARK EXPIRES, and that is the load-bearing half.** A suppression with
no expiry is how a row goes unsupervised forever — the exact failure this plane
exists to prevent. `--hours` is clamped to 24, the tick resumes normal
classification the moment it lapses, and **a lapsed park announces itself**
(`PARK EXPIRED — was: …`) so it can never be confused with one still in force.
⭐ `--reason` is required: a park nobody can read is just a silence.

⚖ **Park is not demote.** Demote means *the owner took this row back* and is
the human's switch; park means *I blocked this row and here is what releases
it*, and is the orchestrator's bookkeeping about its own decisions.

⛔⛔ **AND THE RUNNING WATCHER WILL NOT HONOUR YOUR FIX UNTIL YOU RESTART IT.**
`watch` is a long-running process that imported its module once, at launch. Edit
the script, land it, pull it on every host — **the running loop still executes
the code it started with.**

⇒ Measured immediately after `park` shipped: a correctly parked row escalated
anyway. The state file said `parked: true` with 34 minutes left, the script on
disk had the park code, and the watcher had been running since **before the
feature existed**. Nothing was wrong except that the process was old.

⚠ **The tell is a fix that tests green by hand and does nothing in production**,
which reads as "my fix is wrong" and sends you back into code that is fine.
⇒ **Restart the watcher as the last step of any change to it**, and confirm from
a fresh tick — not from the script — that the new behaviour appears. This is the
same class as a stale daemon serving old behaviour after a deploy, and the
supervision plane is *more* prone to it because nothing ever restarts it.

⛔ **AND PARK DOES NOT REACH THE BOOTER — same two-planes trap as demote.** The
dumb net is a separate subscription and knows nothing about parks, so a parked
row that is also booter-subscribed still gets booted for being quiet, which is
precisely the wake the park exists to prevent. ⇒ Either leave a parked row off
the booter, or `ygg-booter.py defer --secs` it across the same window. **Two
planes, two switches** — and the split is the design, so do not "fix" it by
teaching one plane about the other.

### ⛔⛔ A HOLD SILENCES A VERDICT, NEVER AN AUDIT — AND THE ORCHESTRATOR'S OWN HOLDS BLIND IT

**Owner-directed 2026-08-13**, after a relay sat at **6.1 MB and 37 minutes cold**
while its orchestrator believed the fleet was healthy. The orchestrator had parked
it itself, for a push hold. Two causes, and the first is self-inflicted:

1. ⛔ **`park` suppresses the IDLE verdict — correctly** (a row blocked on purpose
   is not finished) — **but IDLE was the ONLY line that ever mentioned that row.**
   Silencing the wrong verdict silenced the health report with it.
2. ⛔ **Nothing measured what a wake would COST.** Every verb asked *is it
   working*; none asked *what would it cost me to find out*. A cold
   multi-megabyte row is priced at dollars per wake, **charged before it answers a
   word** — so the cheapest question was the one nobody was asking.

⇒ **`fishy_audit()` runs on every tick over EVERY subscriber — parked and pinned
included — and reports only anomalies.** It never nudges, wakes or reaps. It
exists so the orchestrator sees the fishy row *before* it costs something:

| finding | why it matters |
|---|---|
| **≥2 MB and ≥25 min cold** | a wake re-reads all of it first ⇒ **succeed by harvesting, never by asking** |
| **no transcript at all** | the brief was probably never delivered |
| **silent ≥3 h** | confirm it is meant to be idle |
| **`escalate_to` is not a live row** | its cries go nowhere, and briefs reintroduce stale uuids |

⚠ **Prove it can FIRE before trusting a clean run**, and note the trap its own
first run hit: `_run` wraps each argv element in single quotes, so a command
containing `'…'` arrives malformed and returns nothing — which the audit read as
*"NO TRANSCRIPT"* about a perfectly healthy remote row. **Silence from a broken
probe is not a negative result.**

### ⛔⛔ CHECK YOUR OWN CONTEXT AND RELAY YOURSELF — THE ORCHESTRATOR IS NOT EXEMPT

**Owner-directed 2026-08-13**, and stated as a requirement on this seat
specifically: check your own context budget the way it is checked manually, and
spawn a newer version of yourself whose successor despawns you.

⇒ **The seat that relays everyone else is the one most likely to forget it needs
relaying.** It has no cluster watching it, its work feels like coordination rather
than a lane, and it is the row whose silent death costs the most.

- **Watch your own budget on a schedule, not on a feeling.** Silent below 55%;
  **LAND at 70%.** The only fleet session ever to hit the context wall did so
  because that check was manual.
- **Then run the standard succession on yourself** (§10): write the door memory so
  the successor needs no brief, put **your own UUID in the brief as `PREDECESSOR TO
  REAP`**, spawn, verify the ACK token in its transcript, and **let the successor
  reap you** — `ygg-claim.sh … --replace <your-uuid> --booter`, which also moves
  every subscriber with the seat.
- ⛔ **Do not hand a successor a running window.** Land or explicitly transfer any
  hold, freeze or promise you are carrying, and name each one in the brief — a
  promise nobody knows about is not inherited, it is dropped.

### ⛔⛔ PROMOTION AND DEMOTION ARE THE OWNER'S, ALWAYS

A row can be taken **out** of automation entirely and handed back to the human:

```sh
ygg-monitor.py demote <uuid> --reason "design fork — weighing this by hand"
ygg-monitor.py promote <uuid>          # hand it back to supervision
```

A demoted row is **skipped by every verb**: not nudged, not escalated, not reaped.
⭐ The case this exists for is a **design fork** — the point where the trade-off
needs a human to weigh it, and an agent cheerfully continuing is the wrong
outcome. It is also the shape of *"leave this one to me"* for any other reason,
and no reason is owed.

⚠ **The booter is a separate subscription.** Demoting silences the monitor; run
`ygg-booter.py unsubscribe` as well to make the row fully quiet. Two planes, two
switches, and `list` prints which rows are pinned so the split is never invisible.

### ⭐ REAP A CLUSTER THE MOMENT ITS WORK IS DONE — do not let a small one relay on

A cluster that exists for a small, terminal piece of work — add a CLI kind, wire
one flag, build one modal — is **finished when that work ships**, and the
orchestrator despawns it rather than letting it hand off to a successor that
will go looking for more to do.

**Why this is a rule and not tidiness:** the relay's default is *hand off, never
end*, which is correct for a campaign and wrong for a task. A small cluster left
running will find work — usually by widening its own scope into a neighbour's
files, which is exactly what the clustering was for.

⇒ **The orchestrator decides a cluster is terminal at clustering time**, says so
in the brief (*"this cluster ends; report and stand by for reaping"*), and reaps
on the report. Long-lived clusters — a whole subsystem, an optimisation mandate —
are told the opposite, explicitly. ⛔ Never leave it implicit: a cluster that
does not know which kind it is will guess, and the guess is always *keep going*.

⚠ **Reap with §7's discipline.** `session remove` reports the request, not the
effect: read back both `row_still_listed` and `verified`, and identify the
process rather than counting.

### ⛔⛔ NEVER TIDY A CORPSE WITH THE DELETE VERB — the wire cannot tell them apart

**Two intentions, one request, and the difference is invisible downstream:**

| intent | what it means | what it should do |
|---|---|---|
| **RETIRE** — "this row's work is finished, remove it" | a decision | tombstone it. It must stay gone across restarts |
| **TIDY** — "this row is already dead, clean it up" | an observation | nothing durable. The row was lost, not deleted |

⇒ **`session remove` records a DELETION either way.** So a tool that sweeps rows
it judges dead is filing them as things the user chose to delete — and a later
restore then correctly refuses to bring them back, which reads to everyone
involved as *"restore is broken"*.

**How this was established, and it is worth knowing because it disproved a
plausible theory:** rows lost to a hand-killed GUI were reported as *"lost
involuntarily and filed as deletions"*, and the tombstone plane was blamed for
being unable to tell a deletion from a GUI death. Traced per row, that was wrong
— a GUI death tombstones **nothing**; the close path is never called. Every row
in question carried an **explicit close request**, and one of them was an
orchestrator's own deliberate reap, correctly recorded as exactly what it was.
⇒ **The plane was working. The callers were reaping corpses with the delete
verb.**

**The rules that follow:**

- ✅ **Reap a row you are RETIRING** — a finished cluster, a superseded
  predecessor. That is real delete intent and the tombstone is correct.
- ⛔ **Do not reap a row that is already dead.** You are not removing it, you are
  declaring the user meant to lose it. Leave it; it is recoverable.
- ⭐ **Recovering a set that was wrongly tombstoned** is
  `sessions restore <path>… [--include-closed]`, and `close_remembered` on each
  row in `server app rows` tells you which rows carry one before you try.
- ⚠ **Do not hand-roll a restore out of `app open` in a loop.** `open` is
  permissive by design — re-opening a row yourself is how a close is legitimately
  undone — so a loop over it silently resurrects rows the user really did delete.
  That is how a reaped row came back holding a live agent into a worktree its own
  successor was editing.

### ⭐⭐ ORCHESTRATION IS RECURSIVE — any relay may become an orchestrator of its own `N.x.y`

**A cluster is not a leaf.** When a row's scope decomposes into units that are
themselves too big to do serially, that row **decides for itself** to orchestrate,
and seats its sub-units beneath it:

```
        1.0    audit: route the vendor list               ← orchestrator
        1.1.0  vendor alpha: due diligence                ← orchestrator AND relay
        1.1.1  vendor alpha: contract completeness
        1.1.2  vendor alpha: provenance of each document
        1.2.0  vendor beta: due diligence
        1.2.1  vendor beta: contract completeness
```

⇒ **The role is a hat, not a rank.** `1.1.0` is a delegate to `1.0` and an
orchestrator to `1.1.x` at the same time, and nothing about that is special-cased:
it claims its seat, subscribes to the monitor, escalates upward, and supervises
downward, all with the verbs already in this file.

**The case that produced the design** is a due-diligence agent *per case*, under an
orchestrator routing *many cases*. Recon work has this shape almost always: the
top knows the list, only the bottom knows whether one item is finished.

**Four rules make the recursion safe.** They are the only things that change:

1. ⭐ **A row decides its OWN depth. It never assigns depth downward.** An
   orchestrator hands out *scope*, and the receiving row decides whether that scope
   needs sub-units. Deciding for it produces sub-rows nobody owns.
2. ⛔ **ESCALATE EXACTLY ONE LEVEL.** `1.1.2` escalates to `1.1.0`, never to `1.0`.
   Skipping a level hands the top a decision it lacks the context to make and
   silently strips the middle of its job. Each row's `--escalate-to` is its
   **immediate** parent — set it that way at subscribe time and never edit it to
   reach higher.
3. ⛔ **THE CONFIRM GATE IS THE ROOT'S ALONE, AND IT COVERS THE WHOLE TREE.** Launch
   width is the human's decision, and a sub-orchestrator fanning out four sub-units
   has widened the tree exactly as much as the root doing it. ⇒ **A row intending to
   orchestrate says so upward and gets the width confirmed before it fans out.**
   Recursion multiplies burn; that is the whole reason the gate exists.
4. ⭐ **A parent reaps its own children, and only its own.** When `1.1.0` finishes,
   it retires `1.1.x` before standing down — nobody above it knows those rows
   existed. ⛔ An orphaned sub-tree is the worst failure available here: rows with
   no supervisor, escalating to a UUID that no longer answers.

⚠ **Depth is not free and it is not a status symbol.** Two levels is a fleet; three
is a rare, genuinely wide problem. **If a row cannot name why serial execution
fails, it should not orchestrate** — it should just do the work, and the sidebar
stays readable.

### ⛔⛔ THE ORCHESTRATION SYSTEM IS THE ORCHESTRATOR'S OWN WORK — mend it as you run it

**You are not a dispatcher. You are the lead of your category, and systemic
execution reliability is your job.** The clusters own their bugs; **you own the
machinery that runs clusters** — the claim script, the seat scheme, the watchers,
the succession protocol, the briefs, the steers in this file and in the root
instructions. When that machinery is faulty, no cluster will fix it: to a cluster
it is background weather, and each one routes around it privately and silently.

⇒ **Dream while you monitor.** Every time you read a row, ask *what did the SYSTEM
get wrong here?* — a verb that lied, a chore you hand-assembled from primitives, a
handover that dropped something, a watcher that watched the wrong thing. Those are
yours to fix **in the same session**, not to file for later. This is the one place
where the usual "route the dream to the owner, do not fix it inline" rule inverts:
**you are the owner.**

**Three that were found exactly this way, and each had been silently costing the
fleet for days:**

- A claim script **exiting 3 on success**, which skipped the booter arm and the
  predecessor reap — read as agents "declining to despawn" for as long as nobody
  read the exit path.
- **Watchers pinned to a stale spawn set.** A relay hands off, the successor's
  UUID is never added, and the watcher goes on faithfully watching retired rows.
  It reports healthy because the rows it names *are* fine; they are simply not the
  ones doing the work. ⇒ **Re-derive the watch set from the live seats after every
  handover**, never from the file you wrote at launch.
- **Seat and title carrying the same number**, which made every row ambiguous at
  the moment succession most needed them distinct.

**The pattern they share, and it is what to look for:** *the failure is invisible
from inside a cluster and obvious from above.* A cluster sees one confusing
afternoon; you see the same confusing afternoon in three clusters and can name the
cause. That view is the whole reason the role exists.

**⭐ Monitor on an interval, not on hope.** Between reports, sample the live seats
and each row's transcript mtime. A relay's dominant failure is a turn that ends
with work unfinished and no error, and that is invisible unless someone looks. When
a report lands, that is also the moment to ask what the *system* did to make that
report harder to produce than it needed to be.

**⛔ And write the fix down where the next session will find it** — this file, the
root instructions, or the onboarding skill. A repair that lives only in a running
session's context is lost at the handover, which is the failure it was meant to
prevent.

### ⛔⛔ THE 8-HOURLY ORCHESTRATOR HEALTH CHECK — every orchestrator keeps the OTHERS alive

**Standing duty of every orchestrator, owner-directed 2026-08-14.** Roughly every
**8 hours**, sweep the *other* orchestrators — not your own lanes — find any that
are in trouble, **root-cause it and fix it**. Keeping the orchestrator layer
healthy is an orchestrator's job, not the owner's.

**⭐ WHY IT HAS TO BE A SCHEDULED DUTY AND CANNOT BE LEFT TO NOTICING.** An
orchestrator's failure is the only failure on this fleet that is **silent in both
directions**. Its lanes escalate *to it*, so when it dies they escalate into a
corpse and go quiet in a way that looks like calm. And it has no orchestrator of
its own to notice — that is what being at the top of a cluster means. ⇒ **a dead
orchestrator takes an entire cluster's supervision with it and nothing rings.**
The N.x rows cannot see it: from inside a lane, an orchestrator that has stopped
answering is indistinguishable from one that is busy. It is visible only from a
peer at the same level, which is why the duty is *lateral*.

**⛔⛔ THE COLD-CACHE RULE IS PART OF THE SPEC, NOT A REFINEMENT OF IT.**

> **Do NOT wake a cold, stale orchestrator to ask it what it is doing.**

**The asking IS the expense.** A cold row with a large context pays a full
re-read to answer, and what you get back is an unverifiable self-report — you buy
the wake, not the answer. Worse, prompting it makes it *warm*, so a later decision
to succeed it wastes exactly what you just spent. ⇒ **EXTRACT, DO NOT INGEST**,
and never send a message whose content is a question about status.

**The ladder — cheapest instrument first, and stop as soon as it is decided:**

| # | instrument | what it actually answers |
|---|---|---|
| 1 | the **last TIMESTAMPED record** in its transcript | when it last *worked*. ⛔ NOT the file's mtime — mtime is when the row **died**, not when it last worked |
| 2 | transcript **bytes** | what a wake would cost, if you end up wanting one |
| 3 | **monitor + booter state** | is it subscribed, is it armed, and **who does it escalate to** |
| 4 | its **last prose turn** | a working row's own status report, already written and free to read |
| 5 | its **`Write`/`Edit` targets** and `git log` | what it *did*, which is the half a transcript cannot fake |
| 6 | a **targeted grep** | only for the specific term you already care about |

Then cross-check against what cannot lie — the commit, the queue entry, the
memory file. **A transcript says what a session BELIEVED; a commit says what it
DID, and the artefact wins.**

**⚖ THREE OUTCOMES, AND ONLY ONE OF THEM IS A FAULT:**

1. ✅ **Warm and working** — leave it entirely alone. Do not announce the sweep.
2. ✅ **Cold, idle, and FINISHED** — *this is not a fault.* An orchestrator that
   completed its duty and stopped is behaving correctly, and the sweep's most
   common true finding is "nothing is wrong here". Decide whether to reap it or
   let it sit; **do not manufacture a problem to justify the check.**
3. ⛔ **Cold, idle, and work UNFINISHED** — the real case. Root-cause it: a stall,
   a quota window, an exhausted context, an escalation pointing at a retired row,
   armed-but-unsubscribed or subscribed-but-unarmed. Fix the cause, not the
   symptom, and record it.

⚠ **Distinguishing 2 from 3 is the whole skill of this duty**, and it is decided
from artefacts — an unfinished queue item, an unlanded lane, a brief with steps
left — never from asking. A finished orchestrator and a stalled one look
identical from outside; only the *work* tells them apart.

**⛔ The failure modes worth looking for specifically**, because each is invisible
to the row that has it:
- **escalating to a retired row** — a live orchestrator whose alarm rings nowhere;
- **armed but not subscribed** (a stall wakes it but nothing is told) or
  **subscribed but not armed** (something is told but nothing wakes it);
- **an unmerged lane holding a finished result**, which makes the queue lie in the
  direction that costs most: a completed result still reading as outstanding;
- **an unpushed commit**, which is a divergence someone reconciles by hand later.

⭐ **Report it as a sweep even when it finds nothing**, in one line. A health check
whose silence is indistinguishable from not having run is not a health check.

---

## 10.5 ⛔⛔⛔ THE ORCHESTRATOR'S OWN FAILURE MODES — eight that shipped damage

**Read this before the patterns below it.** Everything in section 10 is about
running lanes well. This section is about the ways an orchestrator has actually
destroyed work while believing it was tidying up. Each one is a real event, each
one looked correct from the inside, and each is written with the complexity that
made it survive — a steer stripped to its rule reads as obvious and gets skipped.

⚖ **The common shape, stated once:** every failure below is a verb whose MODEL of
its subject was wrong in a way its own output could not show. Not a bug in the
step — a bug in what the step believed it was operating on.

---

### 1. Work rots on branches, and every report says it shipped

**What happened.** Five delivered mandates — the largest features of a campaign —
were reported CLOSED and live-proven, and each successor brief copied that forward
from the one before. None of the commits was in the trunk. All five sat on one
lane branch that had been unmergeable for a day. Across three repositories the
total came to about forty commits of finished, tested, believed-shipped work.

**Why it survived, and this is the part that matters.** Four separate mechanisms
each looked reasonable alone:

* **The status verb counted SHAs, not patches.** `rev-list --count` compares refs,
  so a rebased branch reads as unlanded forever and a landed one can read as
  ahead. The number was always wrong in both directions and never obviously so.
  ⇒ `git cherry` compares patches. Use it.
* **A merge conflict in an append-only CHANGELOG refused the whole land**, with
  the advice *"the lane must rebase first"*. No lane ever rebases — it is working
  on something else, and the message goes to a log nobody reads. The branch sits,
  and the longer it sits the more it collides.
* **The three cleanup steps blocked each other.** A branch cannot be deleted while
  a worktree stands on it; a worktree cannot be reclaimed while it carries
  unlanded work. Run separately — the only way they were ever run — each step
  reports that it is blocked by the state the previous step exists to clear.
  Nothing ever completed. ⇒ LAND → RECLAIM → PRUNE is ONE chain and must be one
  verb.
* **A relay brief is a claim, not a fact.** "Live-proven, pushed, 0/0" was true of
  a branch and false of the trunk, and no reader checked which.

⭐ **THE STEER.** Before believing any lane's report of what shipped, ask the
trunk: `git cherry <trunk> <branch>` and `git branch -r --contains <sha>`. A
campaign's real state is what is merged, and nothing else. **Run the reclaim chain
every wave** — the cost of skipping it is invisible for days and then enormous.

---

### 2. A phantom row is a kill switch, and the safe procedure did not help

**What happened.** Two rows existed for one session under different schemes; one
was unusable and sat in the active seat. Removing it by its full, exact path
terminated the OTHER row's live agent, mid-work.

**The complexity.** The lane that found the duplicate had already written down
this exact danger and recommended the mitigation — remove by full path, then read
back that the twin survives. That was done. A code read beforehand confirmed the
key resolution was exact. It resolved exactly, and then the close request
downstream **discarded the resolved key and dispatched the session ID**, which the
surviving row shared. The reply said `reaped_processes: []`, because the kill was
asynchronous and had not happened yet when the reply was composed.

⭐ **THE STEER.** A correct mitigation aimed at the wrong layer is not a
mitigation. When two rows share an identity, **removing either is a destructive
act on both** until proven otherwise — and the proof is a live process check five
seconds later, not the verb's own reply. ⛔ Never read an empty "what I killed"
field as "nothing was killed"; an async operation cannot populate it.

---

### 3. A row's value is not always its process

**What happened.** A tidy-up verb folded a row that had no process, no transcript,
and a working directory that no longer existed. It was a group header a person had
hand-titled and deliberately kept for months as a reading list: the sessions were
long deleted, and **the title was the whole artefact**. It cannot be restored —
the session, its transcript and its directory are all gone.

**The complexity.** Every signal the verb consulted said debris, and each was
individually true. The verb's model — *a row's value is its process* — is right
for a lane and exactly wrong for a bookmark, and nothing in the row distinguishes
them except a field nobody was reading: the title's SOURCE. A hand-typed title is
a person saying "I am keeping this".

⭐ **THE STEER.** Before folding anything, ask whether a person NAMED it. Treat a
manually-titled row as untouchable without an explicit force, and **fail closed**:
if the keepsake list cannot be read, treat every row as kept. ⛔ And extract that
list to a durable file — a bookmark that lives only in a running process's memory
is one sweep away from gone.

---

### 4. Rows that were born and never briefed, and read as the busiest verdict

**What happened.** Rows appeared, seated and numbered, that had never written a
single transcript line. One had been sitting for over two hours. Every sweep
classified them WORKING.

**The three-part cause, and no part is sufficient alone.**

* The spawn verb waited **30 seconds** for a new agent CLI to start consuming
  input. A cold CLI takes longer. ⇒ **The ceiling only ever bit the UNATTENDED
  path**, because a person running a spawn by hand simply re-runs it — which is
  why it survived every test.
* On that timeout the spawn exited and **left the row it had created**: seated,
  briefed by nobody, holding a seat its predecessor still held. A failed spawn
  that leaves debris is worse than one that cleans up.
* The classifier, given a live process and no transcript, returned WORKING — the
  busiest verdict it has — so the debris could never be folded or succeeded.

⭐ **THE STEER.** Wait on a DEADLINE, generously: the machine that most needs an
unattended respawn is the one already running twenty agents, and it starts the
twenty-first slowly. A spawn that cannot deliver its brief must hand it to a
deliver-when-ready verb, never exit with the row still standing. And **a live
process with no transcript at all is the emptiest possible row** — give that its
own verdict, or it is invisible forever.

---

### 5. A guard that fails closed in the one environment it was written for

**What happened.** A concurrency lease was added so two agents could not deploy at
once. From the moment it landed, **every scheduled deploy failed**, for hours,
reporting `deploy REFUSED` while the trunk went on advancing and the fleet read as
current at the last version that actually shipped.

**The cause.** `HOLDER="${SESSION_ID##*/}"` followed by a default on the next
line. Under `set -u`, that expansion on an UNSET variable is a fatal error, not an
empty string — so the default could never run. Scheduled runs carry no session id.

⭐ **THE STEER.** **Default first, then transform.** And a guard's first test must
be the environment it was written FOR — here, the unattended one, which is the
only environment nobody watches. ⛔ A deploy that reports "refused" hourly is not
a working guard; it is an outage wearing a status message.

---

### 6. A corpse reads as the quietest possible working row

**What happened.** A monitor escalated a session as *"idle 52 minutes — most likely
FINISHED its scope; give it more work, relay it, or reap it"*. The process had
been dead for 52 minutes. The idle clock was measuring time since death.

**The complexity.** The verdict is derived from transcript age, and a transcript's
mtime is when a row DIED, not when it last worked. The monitor already had a
liveness probe — cheaper than the verdict it would have corrected — and simply did
not consult it before offering three choices of which only one was possible.

⭐ **THE STEER.** Any verdict that assumes a process exists to receive a decision
must confirm the process first. ⛔ **Never derive liveness from a timestamp**;
timestamps freeze at death, which is indistinguishable from concentration.

---

### 7. A census that is seat-scoped measures a fleet that excludes the problem

**What happened.** Four live agents ran for hours producing nothing but
documentation. Every census run against them reported a healthy campaign. **All
four had no seat number**, and every census is seat-scoped — so nothing counted
them, nothing escalated them, and nothing read their output.

⭐ **THE STEER.** A scoped census answers "how are the rows I know about", never
"how is the fleet". Periodically enumerate rows the scope EXCLUDES and ask why
each is excluded. ⛔ And the scoping rule that protects a JUDGEMENT — whether a
quiet lane is finished is its own campaign's call — must not be extended to
liveness: a row with no process is not a judgement, and campaigns without an
orchestrator of their own are otherwise watched by nothing.

---

### 8. The tidy-up ate the work in progress

**What happened.** A resolved merge, held in a scratch worktree under `/tmp` while
its tests ran, was removed by the orchestrator's own worktree sweep between two
commands. The registration was gone; only the commit object survived.

⭐ **THE STEER.** Do interruptible work in a DURABLE location, never in a
directory another sweep owns — including your own. A sweep that checks for
processes standing in a tree cannot see a job that is between commands. ⛔ And
`finally` does not run on a kill: scratch trees leak, so prune them by owner-pid
liveness rather than trusting cleanup to happen.

---

⚠ **What all eight cost, and why the list is written rather than summarised:** a
live agent killed mid-work, a hand-kept bookmark destroyed permanently, four hours
of deploys silently refused, three seats occupied by rows that had never been
given anything, forty commits of finished work invisible for a day, and a resolved
merge lost to the cleanup. Every one of these was performed BY the orchestration
layer, in the course of doing its job correctly as it understood it.

## 11. ⭐⭐ PER-CLI NUANCES — the quirk register, and it is written to GROW

**Every agent CLI has behaviour that is neither documented nor guessable, and that
costs a session an hour the first time it meets it.** A session's discipline
resets at every launch; this register does not. That asymmetry is the whole
reason the section exists.

### ⛔⛔ THE STANDING LAW: HIT A QUIRK, RECORD IT HERE, IN THE SAME SESSION

**Owner-directed.** When you lose time to a CLI behaving in a way its own flags
and messages did not predict, **appending it here is part of finishing the
task, not a chore to do later.** A quirk you solved and did not write down will
be solved again, from scratch, by the next session — and it will look exactly
as mysterious to them as it did to you.

- ⭐ **Write the TELL before the FIX.** The fix is cheap once you know what you
  are looking at; the expensive half is recognising the symptom. Lead every
  entry with what it *looks* like.
- ⭐ **Record what the instrument SAID and how you misread it.** The most valuable
  entries here are the ones where the tool answered correctly and the agent read
  the answer as noise. That is the reusable lesson.
- **A new CLI gets its own `###` subsection** the first time anyone drives it.
  Do not fold its quirks into another CLI's list — the whole value is that a
  session driving X reads only X's list.
- **Correct an entry that turns out to be wrong**, and say what replaced it. A
  stale nuance is worse than a missing one, because it is trusted.
- ⚠ **Keep every example INVENTED.** This file is public.

---

### Claude Code

**0. ⛔⛔ `--no-activate` WEDGES A `remote-cc` ROW BEFORE ITS PTY, AND EVERY STEP OF §3
PASSES ANYWAY.** Measured 2026-08-22 spawning a successor onto the same machine
(`--machine-key dev`) with `--no-activate`, as §3's own example shows.

- **Tell — and it is the nastiest part:** the four-step spawn reports *complete success*.
  `input-check` right after `terminal new` answers `consuming_input:true,
  composer_shown:true`. `submit` answers `submitted:true, bytes:4604`. The transcript file
  appears and **the ACK token is in it**. By every check §3 prescribes, the row is armed.
  **It then never produces a single assistant turn.** Minutes later the same row answers
  `consuming_input:false, composer_shown:false, activity:unknown`, and **both `submit` and
  `send` are refused** (`submitted:false` / `accepted:false`), so the §9 single-`continue`
  remedy cannot be delivered either.
- **The screen is the only instrument that names it** (`server snapshot` →
  `live_sessions[].terminal_lines`):

  ```
  Queue remote Yggterm resume <uuid>
  Target host: <machine>
  Workspace: <cwd>
  Daemon PTY: request main viewport terminal stream     <-- parked here forever
  ```

  The queued remote resume is waiting for a viewport stream that `--no-activate` never
  asked for. The row is `selected:true` in the listing and that changes nothing — selection
  is not the request the resume is blocked on.
- **Fix:** spawn WITHOUT `--no-activate`. The identical brief into an identical row then
  answers `consuming_input:true`, takes the submit, and grows its transcript from 38 KB to
  252 KB in sixty seconds with twenty assistant turns.
- ⛔⛔ **CORRECTION, SAME DAY: `--no-activate` IS NOT THE ONLY CAUSE, AND THE TWO FAILURES NEED
  OPPOSITE REMEDIES.** A later spawn WITHOUT `--no-activate` produced the same symptom at the
  transcript — ACK present, **zero assistant entries**, no growth. It was a different fault,
  and `input-check` is what tells them apart:

  | | wedged resume | plain stall |
  |---|---|---|
  | `consuming_input` | **false** | **true** |
  | `composer_shown` | false | true |
  | screen | parked on *"Daemon PTY: request main viewport terminal stream"* | normal composer |
  | `submit` / `send` | both **refused** | accepted |
  | remedy | respawn without `--no-activate` | **one `continue`** — it woke in 4 turns |

  ⇒ **Read `input-check` BEFORE choosing a remedy.** Respawning a merely-stalled row throws
  away a live agent that one word would have started; sending `continue` to a wedged one
  cannot be delivered at all. ⚠ The transcript symptom is IDENTICAL in both cases, which is
  why the ACK check alone cannot route you — it tells you the brief arrived, and both of these
  are failures *after* arrival.
- ⛔ **So §3 step 4 is necessary and NOT sufficient.** The ACK token proves the brief was
  DELIVERED; it does not prove the agent ever RAN. A transcript that contains your token and
  nothing but `user`/metadata entries is a row that took your brief and died holding it.
  ⇒ **Add a fifth check: transcript GROWTH, or an `assistant` role in it.** One line:

  ```sh
  python3 -c "import json,sys;print({d.get('message',{}).get('role') for d in map(json.loads,open(sys.argv[1])) if isinstance(d.get('message'),dict)})" <transcript>
  # want 'assistant' in the set. Only 'user' = took the brief, never ran.
  ```
- ⚠ **Reaping it needs the `cc-runtime://` path, and the listing keeps the row anyway.**
  `session remove` on the `remote-cc://` spelling answered `accepted:true, live_processes:[],
  "no live session"` and the row stayed listed — §7's law, the verb reports the request and
  not the effect. Confirm death by process identity and by `live_processes:[]`, then leave
  the stale listing for a sweep rather than re-removing it.


**1. ⛔ The first-run workspace-trust gate holds the row before its composer, and
`--dangerously-skip-permissions` does NOT skip it.** It is a *trust* gate, not a
*permission* gate, and it fires whenever the cwd has never hosted a Claude Code
session **on that host** — so a directory that is fine on one fleet machine
still stops a row on another.

- **Tell:** `input-check` answers `consuming_input:false` with reason *"…the row
  is **in a menu**…"* · **no transcript file is ever created** · `submit`
  answers `submitted:false`.
- **Fix:** read the screen (`server snapshot` → `live_sessions[].terminal_lines`),
  confirm the `❯` sits on *"1. Yes, I trust this folder"*, then send a lone `\r`
  **with `terminal send`, NOT `terminal submit`.**
- ⛔⛔ **`submit` DOES NOT DELIVER A BARE `\r`, AND IT REPORTS SUCCESS ANYWAY.**
  Measured 2026-08-13 on two delegates spawned into fresh clone directories:
  `terminal submit … --stdin` with a lone carriage return answered `bytes: 1` and
  `read_nudge.accepted: true` — **and the gate did not move**; re-reading the
  screen showed it still on the trust prompt. The identical byte through
  `terminal send … --stdin` answered `accepted: true, bytes: 1` and **both gates
  cleared instantly.**
  ⇒ **`submit` is the composer path and normalises its payload** (it reported
  `reason: "app_control_send_multiline"`), and a lone carriage return does not
  survive that normalisation. **`send` writes to the PTY, which is what a menu
  reads.** ⚠ This is the file's own law again — *the verb reports the request,
  not the effect* — and it bites hardest here, because a trust gate holds the row
  **before its composer**, which is precisely where a composer-shaped verb cannot
  work while all of its success fields read healthy.
- ⭐ **Answering it once persists it for that directory**, so every later row in
  the same cwd walks straight to its composer. **Answer it; never respawn
  elsewhere to dodge it** — dodging leaves the wall standing for the next agent.
- Full treatment, with the general "a modal is navigable" law: §3.

**2. `submitted:false` and a dropped brief are DIFFERENT failures — do not
conflate them.** `submitted:false` is the verb *refusing* because the row was not
ready: nothing was written, and a retry after the row settles is correct.
`submitted:true` with **no ACK token in the transcript** is the intermittent
delivery drop of §3: the write happened and the CLI never received it. One needs
patience, the other needs a re-submit. Reading the first as the second wastes a
respawn; reading the second as the first waits forever.

**3. A cold row needs SECONDS, and the composer is drawn well before the input
loop is live.** Probe with a real timeout (20-40 s), never milliseconds. A row
that answered `consuming_input:true` in 250 ms was already warm; a fresh launch
will not.

**4. The model id must survive the shell.** Ids can contain characters the shell
treats as special (bracketed context-window suffixes, for instance) — quote the
value at every hop, because it is re-expanded through the launch command. A
silently mangled id yields a working row on the wrong model, which nothing
downstream will flag.

**5. `--permission-mode` is per-launch and wins over any global extra-args**, and
it governs permissions ONLY. It does not cover workspace trust (see 1), a
first-run theme picker, or an auth/login prompt. **Any of those can hold a row,
and all of them are navigable.**

**6. The transcript is the only honest delivery receipt — and it LAGS.** A
transcript file exists once the CLI takes input, and not before, so *absent after
~60 s* means the brief did not land. ⛔ But it is not immediate: measured
2026-08-21, a successfully delivered brief's transcript appeared **14.5 s** after
`submitted:true`, with no project directory before that. An absence inside the
first ~20 s is evidence of nothing. Its *presence* proves nothing about **whose** brief
it holds; that is what the ACK token is for. (§3.)

---

#### ⛔⛔ A FRESH CWD OPENS AT A TRUST PROMPT, AND IT READS AS `consuming_input:false`

**The tell:** you spawn a row per §3, step 2 answers `consuming_input: false` and
`wedged: false`, the process is demonstrably alive and its age keeps climbing, and
a longer `--check-timeout-ms` changes nothing. Two 40-60 s waits in a row look
exactly like a cold start that is taking too long.

**What it actually is:** the first launch into a directory this CLI has not seen
before paints a workspace-trust gate — *"Quick safety check: Is this a project you
created or one you trust?"* — with `❯ 1. Yes, I trust this folder` highlighted and
`2. No, exit` below. The composer is not up yet, so the readiness probe is telling
the exact truth: nothing is consuming input. **`wedged:false` is the discriminator
that separates this from a real hang**, and it was right both times.

⛔ **Do not diagnose this from the readiness probe — READ THE SCREEN.** A blocking
gate and a slow start are indistinguishable through `input-check`, and only one of
them is fixed by waiting.

⛔⛔ **CORRECTED WITHIN THE HOUR, BY THE OWNER, AND THE FIRST VERSION OF THIS
ENTRY WAS WRONG IN THE MOST EXPENSIVE WAY.** It said *"the fix is a bare `\r`"*.
A bare `\r` was sent, `input-check` was then asked and answered
**`consuming_input: true`**, the brief was submitted and returned
`submitted: true`, and the brief's token appeared in the transcript three times.
Every instrument agreed. **The row was still sitting on the trust screen**, and
the owner discovered it by switching to the row and reading it with his eyes;
the brief had been QUEUED behind the gate and only ran once he pressed Yes by
hand.

⇒ **THE REAL LESSON IS NOT ABOUT THIS GATE.** `submitted:true` means the bytes
were written, and a PTY accepts bytes whether or not anything is consuming them —
so a queued brief and a delivered brief are byte-identical from the writer's
side. The transcript check in §3 step 4 does not separate them either: it passes
as soon as the text lands in the file, which happens when the gate clears,
whenever that is. ⚠ **And `input-check` reported `consuming_input: true` for a row
that was demonstrably consuming nothing** — treat that field as *"a write was
accepted"*, never as *"an agent is reading"*.

⛔ **AND THE PROCEDURAL ERROR THAT MADE IT INVISIBLE, which is the reusable half:**
the `\r` was written with its output redirected to `/dev/null` and its answer
never read, and then a DIFFERENT instrument was asked whether it had worked. **A
verb whose result you discard has not been checked by asking something else.**
The write may have failed outright; nobody will ever know, because the one record
that could have said so was thrown away.

⇒ **THE ONLY VERIFICATION THAT HELD WAS THE SCREEN.** After any write meant to
dismiss a gate, READ THE SCREEN BACK and confirm the gate is gone. Not the write's
success, not readiness, not the transcript — the pixels.

⭐ **AND THERE IS NOW A VERB FOR IT** (3.1.21). `server screen <row>` prints the
row's screen as plain decoded text — one line per visible row, no JSON envelope,
no per-caller decoder — and `--state-only` answers the question a spawner
actually has:

```sh
yggterm-headless server screen "cc-runtime://<uuid>" --state-only   # → startup_gate
yggterm-headless server screen "cc-runtime://<uuid>" --state        # + remedy + prohibition
```

⛔⛔ **AND THE REASON NOTHING CAUGHT THIS BEFORE, measured the same day: the state
was invisible to EVERY classifier at once.** A live gate reads
`working:false, question_picker:false, limit_wait:false, background_agent_hint:false`
— not because the phrases were missing from the registry but because of HOW the
screen arrives. The CLI paints its nine visible rows with **absolute cursor moves
and no newlines between them**, and emits single spaces as cursor-forward, so on
the raw stream the whole modal is TWO `\n`-delimited lines (one ~870 characters)
and `quick safety check` **is not present as a substring at all**. Every
line-shaped test — "the last N lines", "these two phrases on the same line" — was
therefore answering a question about the PAINTING rather than about the display.
⇒ The daemon now classifies from its own rendered vt100 grid, which is what the
`startup_gate` state and the plain-text verb are both built on.

⚠ The Enter itself is still the right ACTION at this gate, and it is safe here for
a reason worth stating rather than assuming: the highlight sits on the option that
grants nothing beyond what the spawn already intended, and the gate is about a
directory you chose. That reasoning does NOT generalise — a plan-limit or billing
dialog wears the same shape and its options spend money. The rule stays: `❯`
adjacent to exactly one numbered option is a selection, and you must read WHICH
option before pressing Enter.

⚠ **It fires per (CLI, directory), so a brand-new worktree hits it even though
every sibling checkout is trusted** — which is exactly when a spawn recipe looks
broken rather than gated.

### Codex

**1. Enter is part of the payload on a raw `send`** — append `\r` or codex will
not submit. The readiness-gated `submit` handles this for you and is the safe
path; raw `send` is for a row you are *looking at*.

**2. ⛔ A raw `send` ending in `\r` into a row that is mid-task, at a menu, or
showing a pending self-update fires Enter into the wrong thing.** Observed live:
a `/permissions\r` intended to open a menu **confirmed a pending codex
self-update instead.** Confirm what is on screen before sending a bare Enter.

**3. Menu navigation is raw escape bytes.** Down-arrow is `\x1b[B` (normal cursor
mode) or `\x1bOB` (application cursor mode — check the app-state flag, do not
assume). The full-access selector in `/permissions` is Down ×2 from the top.
⛔ Never arrow blind through a menu that sets a permission level.

**4. Codex rows and Claude Code rows are DIFFERENT SCHEMES and the open verbs are
not interchangeable.** A scan-only codex row resolves through the codex open
path; pushing a Claude Code uuid through it fails **and leaves an orphan behind**,
so the failure costs cleanup as well as time.

**5. Remote codex refuses the per-launch model/permission options** rather than
approximating them, and says so. Treat the refusal as correct and adjust the
launch — do not route around it.

---

### Muse

**Like Codex: no title system — yggterm generates it.** `title_authority: Generated` (fixed 2026-08-17). A fresh Muse session (`muse --yolo` or bare `muse`) composes its own title only from the cwd, so `session_title` stays empty until the first turn; the heuristic (`extract_tail_context` → `heuristic_title_from_context`) and the LLM chore own the display name. No observed startup gate beyond the normal composer readiness — but probe with `input-check` (20s) and `server snapshot → terminal_lines` before assuming, and record here if a trust/menu gate appears like Claude's.

**Launch:** `muse --yolo` is the fleet's standard; `id_assigned_at_birth: false` so the `local://` → `remote-muse://` rebind poll runs (§7.5). No model flag quirk known — re-verify after a `--model` launch and append.

**2026-08-18 — Muse JSONL is NOT codex-shaped — found live.** `is_noise_session_file` used the generic `extract_tail_context` (Codex rollout tail) for Muse’s `session.jsonl` (`payload_type: runtime.user_intent.accepted` with `payload.model_messages[0].content[0].text`), so every real Muse session with `prompt_count>0` was marked noise (empty Codex tail) and would be deleted. Fixed: Muse now decides noise **solely from `session-index.db` `prompt_count/title`** when the DB has an entry; generic tail is only for DB-missing files. Title fallback now tries `muse_title_from_session_jsonl` (first `runtime.user_intent.accepted` text → `best_effort_title_from_context`) before the Codex heuristic, preventing the `1230f99` short-id bug.

**2026-08-18 — Antigravity transcript is not codex-shaped either.** `USER_INPUT` with `<USER_REQUEST>` and `conversation_summaries.db` are the sources; generic tail would mark every AGY transcript as noise. Added early return for `antigravity/.gemini` paths: presence of `USER_INPUT` with `≥10-char` prompt or `conversation_id` means not noise, `.db` files never noise.

**2026-08-18 — Terminal in cwdtree (depth-3 `ssh://` Data Shell).** `REMOTE_DAEMON_SHELL_SCAN_SCRIPT` in `yggterm-server/src/lib.rs:17596` pushed `ssh://` shells into `RemoteScannedSession` → `__remote_folder__/oc/...` as scanned sessions, duplicating the live rail. `Plain SSH terminals remain Live Sessions only` (state.rs:50340) — removed the daemon shell scan’s push (now `let _ = REMOTE_DAEMON_SHELL_SCAN_SCRIPT`) so shells appear only in `__live_sessions__`.

**2026-08-20 — a Muse row's uuid is NOT a Muse session id, and the store proves it.** `id_assigned_at_birth: false` means Muse mints its own id via RPC; the row uuid yggterm carries is a phantom its store has never held. ⛔ **`muse resume <phantom>` exits 1** with `retained session not found: session <id> has no saved log` — the PTY dies at open, which is why the symptom reads as *"the session does not persist"* rather than as an error.

*Where a live Muse session's real id is:* the directory name in `~/.local/share/muse/sessions/YYYY/MM/DD/<session-id>/`, and the running process names it by holding **`<session-dir>/.session.lock`** open. ⚠ **Do not use any other open file**: a live Muse process holds `cron.db` (+`-shm`/`-wal`) open for SEVERAL session directories at once — including ones it merely carried forward — and only `.session.lock` for the one it is actually running. Two `.session.lock` fds pointing at the SAME directory is normal, not ambiguity.

*Checking membership without a walk:* `session-index.db` `sessions(session_id, workspace_root, title, prompt_count, updated_at_us)` answers by key. ⭐ **`prompt_count = 0` rows in one cwd are the fingerprint of repeated resume-misses** — each failed attach mints a fresh empty session, so a stack of them is the bug's own audit trail.

### Antigravity (agy)

**Like Claude Code: Store-authoritative.** `title_authority: Store` (already correct). `agy` writes `conversation_summaries.db` (`conversation_summaries.title`) and yggterm respects it, writing back only on explicit rename. `install: Manual` (`agy update` self-updates, measured `agy --help` 2026-08-08) — yggterm provisions by invoking `agy update`, not `npm`.

**No standalone trust gate observed** — but `agy` is Manual-install and can be absent (`not_supported_on_platform` probe), and `id_assigned_at_birth: false` so remote rebind applies. If a first-run gate appears, drive it with `terminal send --data $'\r'` after confirming `❯` like Claude §11.1 and record the tell here.

**⛔⛔ 2026-08-20 — `conversation_summaries.db` CANNOT answer "does this conversation exist", and treating it as authoritative destroys sessions.** Measured: a conversation the CLI creates gets `brain/<id>/` and `conversations/<id>.db` **immediately**, and is **still absent from `conversation_summaries.db`** afterwards — yet `agy --conversation <id>` resumes it without complaint. So that table produces false NEGATIVES for live, resumable conversations. Ask the per-conversation artefacts instead: `conversations/<id>.db` or `brain/<id>/`, a **path check** rather than a query, because the file NAME is the id. Keep the summaries table only as an additional *yes*, never as a *no*.

**⛔ 2026-08-20 — a resume miss is a WARNING, not a failure, and that is what makes it dangerous.** `agy --conversation <unknown>` prints `warning: conversation "<id>" not found` and then **starts a brand new conversation, exit 0**. The caller sees a clean launch, so the row silently becomes a different, empty session under the same title. Never treat agy's exit code as evidence that a resume attached.

**2026-08-20 — where agy's live id is.** At LAUNCH the process holds only the shared index — a fresh row has nothing to bind to because *the conversation does not exist yet*, so expect no id until the first turn. After a turn it holds **`~/.gemini/antigravity-cli/presence/<conversation-id>.lock`**, where the id is the file's **stem** (contrast Muse, where it is the enclosing directory's name). ⭐ To confirm a mapping by hand without an interactive session: `agy -p "<prompt>" --output-format json` returns `{"conversation_id": …}` directly. ⚠ The `crashes/crash_<pid>_<uuid>.log` uuid is NOT a conversation id — it is a run instance.

---

### Viewport coverage — how much of the grid Grok Build, OpenCode, Pi and Qwen actually paint

*A cross-CLI measurement, deliberately one table rather than four entries: its
whole value is the COMPARISON, and the register's per-CLI rule exists so a
session driving X reads only X's list — here the other three ARE the control.*

*Measured 2026-08-20 with `scripts/cli-viewport-probe` (a real PTY fed to the same
`vt100` crate the daemon parses with). Recorded because the LAST session to ask
this question guessed, and the guess became a 120x40 PTY clamp that produced the
fault it was meant to fix.*

| CLI | given a 173x63 PTY, paints to column | reads as |
|---|---|---|
| `grok` | 171 | fills whatever grid it is handed |
| `opencode` | 172 | fills whatever grid it is handed |
| `pi` | 173 | fills whatever grid it is handed |
| `qwen` | 102 | genuinely narrow — **and paints the same 102 at a 120-col PTY**, so shrinking the terminal does not make it fill anything |

- ⛔ **THE TELL, AND IT IS THE WHOLE ENTRY: a TUI painting ~120 columns inside a
  wider viewport is a STALE PTY GRID, not a narrow CLI.** The two look identical
  on screen. Read `PTY size` in the session metadata pane (or `server snapshot`)
  and compare it against the client grid *before* concluding anything about the
  program. If they disagree, the bug is the geometry, not the CLI.
- ⚠ **`qwen` renders a 3-row header, not the 6-row banner its ASCII art suggests**,
  at every grid size from 100x30 to 173x200, in a bare PTY with no yggterm
  involved. If you are chasing "qwen's motd is cut off at the top", reproduce it
  in a plain shell first — by the wrapper-vs-manual parity rule that makes it
  qwen's own rendering and not ours.
- ⚠ **A hand-rolled vt100 will lie to you here, and it lied first.** Gradient
  banners and block art are routinely drawn as SPACES carrying a background
  colour; a probe that asks "is the text blank" scores a fully-painted header as
  empty and invents a cut-off top. Use the probe (it reports `bg_only_cells` so
  the failure mode is visible) or the daemon's own screen, never a quick parser.

---

### Grok Build (grok)

**The tell:** `grok --version` works, the npm package installed fine, and yet
`~/.yggterm/npm/lib/node_modules/@xai-official/` looks almost empty — no
per-platform payload package anywhere you would expect it.

- ⭐ **The payload is nested one level down**, at
  `@xai-official/grok/node_modules/@xai-official/grok-<platform>-<arch>`, ~45 MB
  brotli-compressed. A `find -maxdepth 5` from `node_modules` misses it by one
  level and reads as "npm skipped optionalDependencies", which is a different
  and much scarier diagnosis. Count the depth before believing it.
- ⭐ **`bin/grok` is a trampoline, not the program.** It resolves
  `$GROK_HOME/bin/grok` (default `~/.grok/bin/grok`) and execs it; if that is
  missing it decompresses the payload on FIRST RUN. So a first invocation after
  an install can take seconds and write ~166 MB, and an install whose
  `postinstall` never ran still produces a working `--version`. ⇒ **A working
  `--version` does not prove the install completed.**
- ⛔⛔ **`grok update` IS npm, WEARING A CLI'S CLOTHES.** Its own
  `grok update --check --json` answers `"installer":"npm"` — for an
  npm-provisioned copy the updater shells back out to npm. Anything you were
  relying on the CLI's own updater to avoid (a prefix, a staging directory, a
  verification step) is NOT avoided; it is just moved somewhere you cannot see
  it. yggterm therefore runs npm itself for grok rather than preferring the
  self-updater — see the descriptor's comment for the full measurement.
- ⚠ **It installs a copy outside the managed prefix**, `~/.local/bin/grok` →
  `~/.grok/bin/grok`, from its own postinstall. That is the vendor's layout and
  is fine; do not "fix" it, but do not be surprised when `which grok` resolves
  somewhere the provisioner never wrote.
- **Versioned + symlinked, deliberately.** `~/.grok/bin/grok-<version>` with a
  `grok` symlink swapped onto it, because replacing a binary a running process
  has mmap'd is fatal on macOS. Worth copying rather than working around.

### ⛔⛔ EVERY CLI BELOW — `terminal submit` CANNOT SEE THEIR COMPOSER. USE `send`.

**Read this before the per-CLI entries; it applies to all of them and it is the single
thing most likely to cost you an hour.** Measured 2026-08-20 on six freshly-spawned rows
(muse · grok-build · codex · kimi · pi · opencode), each `launch_phase: Running`, each
with its composer visibly painted:

- **Tell:** `terminal submit` answers, after ~30 s, `submitted:false` with
  *"no agent composer row appeared within the timeout — the row is mid-output, in a
  menu, or is not an agent CLI, so input readiness is unanswerable rather than false"* —
  **while `server snapshot` → `terminal_lines` shows the composer drawn and idle.**
- ⛔ **Do not believe the reason string.** It blames the ROW (mid-output, in a menu) for
  what is a DETECTOR gap, and it will send you hunting a wedge that does not exist. The
  detector recognises a Claude-Code/Codex-shaped composer; these TUIs each draw a
  different prompt glyph, so readiness is genuinely unanswerable *to it* — the wording is
  honest about its own limits and misleading about the row.
- ✅ **What works:** `terminal send` — the raw PTY write — as **two separate writes**:
  the text, a short pause, then a **lone `\r`**. Verified by the CLIs answering in-frame.
  (This is the same two-write law as the Enter key everywhere else in this file.)
- ⚠ **`send` is UNGATED.** `submit` existed to refuse a busy row; `send` will type over
  one. Read the frame first and never point it at a row a human is using.

---

### The context + model equation for ALL of these CLIs: READ THE FOOTER

⭐⭐ **Every one of these TUIs paints its model — and usually its context usage — into
the last rendered lines, so `server snapshot` → `live_sessions[].terminal_lines` answers
"what model is this on" and "how full is it" for ANY CLI without typing a single byte.**
Prefer it to every per-CLI `/context` equivalent: it is one read, it is identical across
CLIs, and it cannot type into a live prompt. Exact spellings are in §8.5(e)'s footer
table. ⚠ Strip ANSI/CSI escapes before matching — these footers are drawn with colour and
cursor-positioning codes and a naive substring test misses them.

---

### Muse — additions

**Spawn:** clean. Reaches `Running` with a full PTY inside ~100 s; composer is `⟩`.
**Footer:** `<model> · <effort> · <cwd> · YOLO`.
**Model:** `--model` reaches the provider and **does not stick** (measured: config mtime
identical across a flagged run). ⭐ **A bad id is refused BY NAME by the provider** —
`agent loop failed: model failed: model '<id>' does not exist or you lack access
[request_id=…]`. Treat that as the flag WORKING, not as a launch failure.
⚠ It emits a startup warning when a rules file exceeds its context budget and
**truncates it for that session** — worth reading, because a truncated rules file is a
silent behaviour change, not an error.

---

### Kimi

**The tell of a login-less kimi: it looks HEALTHY.** The row reaches `Running`, paints a
full TUI with a working composer, and accepts input. Only when you submit does it answer
`LLM not set, send "/login" to login` — inline, as if it were a reply.

- ⛔ **It is NOT a wedge and not a spawn failure.** Every launch signal is green; the
  refusal arrives at turn time, in the transcript position where an answer would go.
- ⭐ **A login-less run STILL MINTS A SESSION ID** (non-interactively it prints
  `To resume this session: kimi -r <uuid>` and exits 1). So a store scan finds kimi
  sessions that never did anything — a stack of them is the fingerprint of repeated
  login-less launches, not of work.
- **Footer:** `yolo  agent  <cwd>` and, on its own line, **`context: <pct>%`** — one of
  the two CLIs that states context as a bare percentage.
- ⚠ **Its `--help` names `~/.kimi/config.toml` as the config default; the directory that
  actually exists on disk is `~/.kimi-code/`.** Do not go looking for the former.
- Startup may paint an **update notice**, not a login gate — do not read the notice as
  the auth problem.

---

### Qwen Code

**The tell: the row is `Running`, the CLI is painted, and NOTHING you send lands —
because it is sitting on a first-run consent MENU.** The frame shows a numbered list:

    › 1. Yes
      2. No (esc)
      3. No, don't ask again

- ⛔ **This is a menu, so the composer verbs cannot work** — same class as Claude Code's
  workspace-trust gate (§11.1). Drive it with `terminal send` and a lone `\r` after
  confirming from the frame which line the `›` sits on. **Never arrow blind through a
  menu that sets a persistent preference** — option 3 is "don't ask again".
- **Login:** non-interactively it exits 1 with *"No auth type is selected. Please
  configure an auth type (e.g. via settings or `--auth-type`) before running in
  non-interactive mode."* — a different and much clearer shape than kimi's, and it comes
  BEFORE any session work.
- ⚠ Its narrow painting is genuine, not a stale PTY grid — see the viewport table above
  before chasing it.

---

### Pi

**Spawn:** clean; reaches `Running` promptly.
**Footer — the richest of any CLI here:** the cwd on one line, then
**`<used>/<window> (auto)`** together with the model id. It is the only one that states
the context WINDOW alongside the usage, so a percentage can be checked rather than
trusted.

- ⭐ **Custom / gateway providers go in `~/.pi/agent/models.json`** — a `providers` map
  with `baseUrl`, `api: "openai-completions"`, `apiKey` and a `models` list. `apiKey`
  supports `$ENV_VAR` interpolation and `!shell-command` resolution, so a key need never
  be written literally. `pi --list-models` then shows the provider and is the cheap
  read-back that the file parsed.
- ⛔⛔ **A generation swap can delete the tree a RUNNING row is executing from.** Observed
  live: a row launched from a `pi.genN` prefix failed mid-turn with a Node
  module-resolution error naming a path inside that prefix — which had been removed while
  the row ran, the CLI having been re-provisioned to `genN+1`. **Tell:** a running agent
  row that suddenly cannot resolve its own bundled modules, with a generation number in
  the path that no longer exists on disk. It is not a config error and re-reading the
  config will not show it.
- ⚠ A malformed skill/extension file surfaces as a YAML parse error painted into the
  session — noisy but not fatal; it does not stop the row.

---

### OpenCode

**Footer:** `<agent> · <model>` on one line, `<cwd>` and **`<used> (<pct>)`** on the last.

- ⭐ **Custom / gateway providers go in `~/.config/opencode/opencode.jsonc`** under
  `provider.<id>` with `npm: "@ai-sdk/openai-compatible"`, `options.baseURL`,
  `options.apiKey`, and a `models` map. ⭐ **Key the models map by a SHORT ALIAS and put
  the real upstream id in the entry's `id` field** — the CLI addresses models as
  `provider/model`, so an upstream id that itself contains slashes makes the address
  ambiguous. `opencode models | grep <provider>` is the read-back.
- ⛔⛔ **It sends `reasoning_effort` and `verbosity` on EVERY request, and setting
  `reasoning: false` on the model does NOT stop it.** Against a gateway that rejects
  unknown params this fails the whole turn. **Tell, and it is a nasty one: the first
  symptom is a SILENT TIMEOUT, not an error** — the CLI retries on an escalating ladder
  for well over a minute before printing anything, so a 120 s timeout returns with no
  output at all and reads as a hang. ⇒ **Run it once with `--print-logs` before
  concluding anything**; the per-attempt `stream error` lines name the real cause
  immediately.
  ✅ **Fix that worked:** put the gateway's own suggested escape in the model's `options`
  — `{"allowed_openai_params": ["reasoning_effort", "verbosity"]}` — which tells the
  gateway to pass them through instead of rejecting the request.
- ⚠ **The TUI's default model is NOT the one `run -m` uses.** A freshly spawned row
  showed the vendor's own default in its footer while `opencode run -m <provider>/<alias>`
  answered on the configured gateway. Pin the model on the row, or read the footer.

---

### Grok Build — additions

**Footer:** `<model> (<effort>) · <approval-mode>`, drawn INTO the bottom border of its
composer box — so a line-oriented match for the model must include border characters.
**Spawn:** clean, reaches `Running`. Composer is `❯` inside a box.
⚠ Of the CLIs driven by raw `send`, this was the one where a text-then-`\r` pair did NOT
land on the first attempt while the same pattern worked elsewhere. Read the frame back
after sending rather than assuming; give it a longer pause between the two writes.

---

### Antigravity — additions

**⛔⛔ ON A REMOTE LANE IT NEVER ATTACHES, AND EVERY SUCCESS SIGNAL SAYS OTHERWISE.**
Measured twice, independently, 2026-08-20: `terminal new --kind antigravity
--machine-key <host>` answers `launch.applied: true` and creates a row — which then sits
in **`launch_phase: RemoteBootstrap` indefinitely** (still there after ~10 minutes) with
`pty_cols`/`pty_rows` both **`null`**, painting only yggterm's own launcher preamble.
Every other kind reached `Running` with a real PTY inside ~100 s.

- **Tell:** `launch_phase` never leaves `RemoteBootstrap` and the PTY dimensions are
  null. `input-check` times out with "no agent composer row appeared", which reads like
  a slow CLI rather than a lane that never opened.
- ⚠ **Its session path has no host segment** — `agy-runtime://<uuid>` where every other
  remote kind produces `remote-<cli>://<host>/<uuid>`. A reap of that row answered
  `verified:false` with no reason and no surviving pids, while every `remote-*://` reap
  in the same batch answered `verified:true`. Treat the scheme asymmetry as the likely
  cause and verify the process by hand.
- ⭐ **Its config stores the model's DISPLAY NAME, not the id.** `settings.json` holds
  something like `"Gemini 3.x Flash (High)"` while `--model` and `agy models` speak ids
  like `gemini-3.x-flash-high`. Comparing the two directly will always look like a
  mismatch. The flag does **not** write the config (measured: mtime unchanged).
- ⭐ **Cheapest model/context read of any CLI here:** `agy -p "<prompt>" --output-format
  json` returns `{"conversation_id": …, "usage": {"input_tokens", "output_tokens",
  "total_tokens", "cache_read_tokens"}}` — a real token accounting without a TUI.

---

### Codex — additions

**Footer:** `<model> <effort> · <cwd>`.
⚠ **Its durable store entry can carry the YGGTERM ROW TITLE rather than a CLI-generated
one** — a row spawned with `--title 'probe: …'` showed up in the store scan under exactly
that string, where the other CLIs showed either a generated title or a placeholder.
⛔ **A reap can delist the row and leave the codex processes running** (measured: sidebar
clean, two live codex processes still holding the row's cwd). Identity-check by cmdline
after every reap — the sidebar will not tell you.

---

### Durable rows: what a despawn actually leaves behind (measured across all eight)

⭐ **The good news, and it was worth proving: a despawned agent-CLI session DOES survive
into the durable plane for every CLI that took a turn.** After reaping eight probe rows,
the store scanner reported durable rows for muse, grok-build, kimi, qwen-code, pi,
opencode and codex, each carrying the right cwd. The cwd tree and start page are fed from
that scan, so these CLIs are findable again after their live row is gone.

Three defects ride along, and all three are recognition problems rather than data loss:
- **One spawn can produce SEVERAL durable rows** (four of the CLIs contributed two or
  more for a single session), so one session can look like three.
- **Placeholder titles** — `Grok Shell`, `Kimi Shell`, `Pi Shell`, `Qwen Shell`,
  `Opencode Shell` — which read as stray plain shells rather than as agent sessions.
- **Some rows carry no title at all** (`None`), leaving them effectively unlabelled in
  the one surface whose job is recognition.

⚠ **And the measurement trap that nearly produced a false finding here:** the sidebar
row list emits children only for EXPANDED groups. Counting per-CLI rows in it and finding
none is NOT evidence the durable plane lacks them — the sample was 206 of 373 rows hidden
by collapsed sets. **Ask the scanner directly** (`server startpage ls --json`), which
reports what the stores actually hold, and ask it **on the host where the CLIs run** —
running it against the GUI host's home answers a different question and quietly returns
that host's much smaller set.

### npm-provisioned CLIs as a CLASS — the one that cost the most

**The tell:** an agent CLI is suddenly "not found" — often SEVERAL at once —
shortly after a machine rebooted, was OOM-killed, or had a daemon restart.

⛔ **`npm install -g --force <several packages>` unlinks EVERY published binary
before relinking any of them.** Measured twice, deterministically: seven CLIs
present, a kill 12 s in, **seven gone** and seven orphaned `.<name>-<random>`
symlinks left in the bin directory. The 2×2 that pins it — a single-package
install survives the same interrupt with or without `--force`; a batch without
`--force` survives; only batch-plus-`--force` destroys the set.

⭐ **And `--force` is what made it fire on every pass, not just on change:** it
rewrites the whole tree and relinks every bin even against an already-current
install (`changed 164 packages` on a no-op), so the destructive window was
entered by every routine refresh.

⇒ **The lesson that travels beyond npm:** a package manager's "install" is not
atomic and its unlink phase is global. Stage into a fresh directory, prove the
binary exists, then publish with a single `rename`. yggterm now does exactly
that, one prefix per CLI.

⚠ **A damaged shared tree then LOOKS like a race.** Once a prefix carries
partial state, every retry re-enters the damage and fails identically — and a
cluster of failure timestamps reads as concurrency. Check the INTERVAL between
them before concluding anything: ~3.5 s apart is a lock working, not a race.

### Any CLI — quirks that are about the ROW, not the program

**0. ⛔⛔ A MULTI-SECOND "the row will not take my typing" IS NOT ALWAYS THE ROW —
until 2026-08-20 the GUI's own event loop stopped reading input while it waited
on the daemon.** `tokio::select!` runs the chosen branch to completion before
polling any branch again, and the terminal loop awaited the daemon read inline.
While that read was out, the branch handling KEYSTROKES was not polled at all.

- **Tell:** typing lands seconds later, all at once; and the `input/keystroke`
  probe shows **nothing at all** for the stall, because it is emitted from inside
  the branch that was blocked. **A zero reading from that probe during a reported
  freeze is not evidence the user did not type.**
- **Measured:** with the daemon deliberately stopped for 6 s, the old build
  recorded `input/loop_block branch=read_poll held_ms=5964`; the fixed build
  recorded nothing across the identical stall.
- **Fix (shipped):** the read runs on its own task; `input/loop_block` is emitted
  by whichever branch held the loop, so a stall now names its cause.
- ⚠ **What it does NOT fix:** the daemon serves every request under ONE runtime
  lock, so a handler doing slow IO still deafens it to all clients. That half is
  its own queue entry — do not read a quiet loop as a quiet daemon.


These bite regardless of which agent CLI is inside the row, and they have each
cost a live session:

- ⛔ **`pkill -f <uuid>` kills YOUR OWN shell**, because the uuid is in your
  command line. Collect pids, exclude `$$`, then signal. Identify; never
  pattern-match.
- ⛔ **`remove` can answer `verified:false` with a survivor that is already
  exiting.** The verdict is a sample taken at that instant. **Re-check the named
  pids directly before escalating to a kill** — a runtime that needed one more
  second reads identically to one that is stuck, and killing it is the more
  expensive mistake.
- ⛔ **A row verb answers with the REQUEST, not the EFFECT** — §7 is the full
  treatment, and it applies to every CLI kind.
- ⛔ **App control answers only on the host where the GUI runs.** A headless host
  has daemons and sessions but no client to drive, and says so plainly. Scripts
  that claim a row need the GUI host passed in when they run anywhere else.
- ⚠ **`probe-scroll` is not a screen read.** It answers `{accepted, reason,
  session_path}` — it scrolls, it does not report content.
  **`server snapshot` → `live_sessions[].terminal_lines` is the instrument for
  "what is on that row"**, and it needs no activation, so it never disturbs the
  owner's viewport.
- ⚠ **Some verbs emit MORE THAN ONE JSON object** on a single call. A naive
  whole-document parse throws *"Extra data"* and looks like a broken verb. Decode
  in a loop and read every object.

### ⛔⛔ ANTIGRAVITY (`agy`): its store is a LEDGER OF EVERYTHING, not a session list

Measured 2026-08-20 on a 999-row `~/.gemini/antigravity-cli/conversation_summaries.db`:
**995 of those rows are batch tool invocations, 4 are sessions a human would
resume.** Anything that treats a DB row as a session inherits a ~250x
overcount — it put 452 one-session `/tmp` folders in the cwd tree.

- ⛔ **`killed=0` filters NOTHING** — every row had `killed=0`. So did every other
  column that looks built for this: `source`, `status`, `agent_name`,
  `nesting_depth`, `parent_conversation_id`, `battle_id`, `not_fully_idle`,
  `last_user_input_step_index` were uniformly empty or default. **Print a store
  column's distribution before filtering on it**; a filter on a constant column
  is indistinguishable from no filter.
- ⭐ **The discriminator is `workspace_uris` + `step_count`:** a real workspace,
  none of its roots an ephemeral scratch dir, and at least one step. The batch
  signature is a real repo root with a `/tmp` scratch dir beside it.
- ⚠ **Test the PATH, not the filesystem.** "Does the workspace still exist"
  measured worse (batch rows with surviving scratch dirs) and makes the answer
  change as `/tmp` is reaped.
- ⛔ **`last_modified_time` is per-row and is ISO-8601 with a SPACE separator** —
  RFC-3339 parsers reject it until the separator is swapped. Never substitute
  the DB FILE's mtime: that stamps one shared fake recency on every row and it
  moves whenever the CLI touches the store.
- ⚠ **On-disk transcripts are `transcript_full.jsonl`**, not `transcript.jsonl`.
  The wrong spelling matches 0 of 497 brain dirs and fails silently, which is
  what a glob does when it is wrong.
- ⚠ **`conversations/*.db` holds a `.pb`**, so that glob matches nothing either.
  The summaries DB is the index; the brain dirs are storage.

### ⚠ MUSE: `prompt_count = 0` does not mean the file is empty

Zero-prompt muse sessions carry ~12 KB of real lifecycle records (metadata,
route facts, a clean `session_end`). Skipping them from a session list is right;
reading the index row as "this file is empty" is not, and **it must never drive
a delete.**

### ⛔ OPENCODE: TWO packages share one tag name — the row you drive is `opencode2` (fixed 2026-08-28)

OpenCode's v2 preview ships as the npm package **`@opencode-ai/cli`** (bin
`opencode2`, build-numbered versions `0.0.0-beta-<n>`, beta tag moves); the
UNSCOPED `opencode-ai@beta` is the abandoned v1 line (date-stamped
`0.0.0-beta-<date>`, frozen upstream). The managed install pinned the right
TAG on the WRONG package for days and every integrated row served a stale
build while terminals ran the fresh one — symptom: `opencode --version` in a
row disagrees with `opencode2 --version` in a shell. Fixed in
`agent_cli.rs` (descriptor: binary_name `opencode2`, package `@opencode-ai/cli`,
tag `beta`) with a regression lock; `npm_dist_tag` is policy, the PACKAGE is
the pin. Verify your row's lineage from the binary, never the name:
`readlink /proc/<pid>/exe` must name `@opencode-ai/cli`. The same-name trap
has a second face on PATH: a user's `~/.opencode/bin` carries BOTH `opencode`
(v1 stable) and `opencode2` (preview) — typing `opencode` gets v1, so address
the CLI the descriptor declares, not the one the banner reminds you of.

### ⛔⛔ OPENCODE2: the TUI hosts N sessions in ONE PTY — the row is not the session (tab bar)

**Owner-directed 2026-08-28; this is the one registered CLI that breaks the
fleet's one-session-one-row law.** OpenCode2's TUI has a **tab bar**: N open
session tabs on the current cwd, spawned from `+ New session`, closed with ×,
switched with `session.tab.*` keybinds. A shared background **service** owns the
sessions; the row's PTY hosts only the TUI client. **One yggterm row is 1 : N
opencode sessions.**

**The tell:** a row whose screen carries a tab bar at the top. Everything you
read off that PTY — footer, composer, context % — describes **the focused tab**,
never "the session".

⇒ **What this does to every fleet primitive, until yggterm mirrors tabs to rows
([[spec-cli-integration]] Issue 26):**

- **A PTY write lands in the FOCUSED tab, whoever that is.** `submit`, `send`, a
  boot, a nudge — addressed to the row, delivered to whichever session the human
  last focused. This is the wrong-row wake hazard with no wrong row to blame.
  ⛔ **Per-session addressing goes through the service API or does not happen:**
  `opencode2 api post /api/session/<id>/prompt` (steer/queue inbox semantics —
  the same contract as §4), `…/rename`, `GET /api/session/<id>/context`,
  `GET /api/session/active` for the tab list, `GET /api/event` for lifecycle.
- **Monitor, booter, notify and the context gauge are per-ROW.** They see one
  stream and one footer for N sessions; a classification, a boot decision or a
  gauge reading taken off an opencode row is a reading about ONE tab of N,
  presented as if about the row.
- ⛔ **NO fleet orchestration on opencode rows.** Owner rule, same day: an
  opencode session works **like a normal session** — no relay, no booter
  subscribe, no monitor, no delegate spawns *from* fleet tooling against its
  rows — because the row primitive those tools address does not exist
  per-session yet. This is the first CLI a campaign must route AROUND.
- **Claim discipline:** the row belongs to the TUI. An opencode session that
  wants to name itself renames its SESSION (`POST …/rename` or `ctrl+r` in the
  TUI) and must **not** fight the other tabs over the one row title. Only the
  anchor row carries a seat; sessions under it wait for the tab→row mirror to
  get their own.

**When the tab→row mirror ships, update this entry in the same commit** — the
contract (spawn seats below the TUI's last tab; switch follows both ways; close
hides, never tombstones, because `session.tab.reopen` exists) will live in
yggterm, and the "route around" rule above dies with it.

---

## 11.9 ⭐ REHEARSE A VERB BEFORE IT DECIDES SOMETHING — aim it at a sandbox

Every fleet verb takes its aim from the environment, so the destructive ones can
be run somewhere that does not matter before they are run somewhere that does:

```sh
SB=$(mktemp -d); mkdir -p "$SB/bin"
cp <the headless binary> "$SB/bin/"
export YGGTERM_HOME="$SB"                 # ⛔ before anything starts, not after
export YGG_HEADLESS_BIN="$SB/bin/yggterm-headless"
ygg-deliver.py <row> --message brief.txt  # …now drives the sandbox plane
```

Three variables, one owner (`ygg_appctl`), and each verb prints what it aimed at
on its first line — `host=… home=… bin=…`:

| variable | moves |
|---|---|
| `YGGTERM_HOME` | which STATE plane: daemon socket, database, and the fleet's own relay store |
| `YGG_HEADLESS_BIN` | which BINARY asks — a freshly built one, or a recording stand-in |
| `YGG_APPCTL_HOST` | which MACHINE answers; `local` means no ssh at all |

⭐ **A non-default `YGGTERM_HOME` implies the local transport**, because a home is
a PATH and a path is a fact about one machine. Exporting the real home changes
nothing, which is the point — the inference has to be inert for everyone who is
not sandboxing. `--host` beats it, so a sandbox on another machine still works.

⛔⛔ **THE HALF THAT LOOKS DONE AND IS NOT: the home must be written INTO the
remote command.** Exporting `YGGTERM_HOME` in the shell that runs a verb does
nothing if the verb reaches the plane over ssh — the far end starts a fresh login
shell and inherits none of it, so the call silently answers about the REAL home.
A gate that only exercises the local arm is green over that, because there the
variable reaches the child through ordinary inheritance. Both arms, or neither.

⚖ **Why this is worth a section.** For most of its life this plane could not be
aimed anywhere but at a living person's desktop: the binary was a module constant
in four separate files and `--host` moved the machine and never the home. So a
verb that force-folds a row was only ever exercised by the run that mattered, on
rows somebody was working in — and its most destructive branch turned out to have
raised `NameError` for a whole commit without anyone noticing, because a healthy
suite never takes a delivery-failure path. **The more destructive a branch, the
more certain it is that nobody has run it.**

⚠ **What a sandbox does NOT give you.** The row plane needs a GUI client, and
that needs Xvfb *plus a private dbus session plus* `WEBKIT_DISABLE_COMPOSITING_MODE=1`
— recipe and the silent failure mode are in `docs/agent-field-guide.md`. And what
is learned there about RENDERING does not travel to a real desktop; what is
learned about the daemon and these verbs does, because it is the same code.

⛔ **Two guards are UNIONED across both homes rather than aimed**: `never-arm.tsv`
in `ygg-deliver` and `protected_uuids` in `ygg-fold`. A fresh sandbox home has no
list, so aiming those with everything else would mean nothing is protected —
correct for sandbox rows and catastrophic if the aim is ever wrong. Refusing too
much costs a rerun; refusing one row too few types into somebody's half-written
turn.

---

## 12. Adapting this to your own setup

Everything above is generic. To make it yours:

- **Pick your memory directory** and pass it to `bootstrap.sh`.
- **Pick your seat numbering.** Numbering rows like a book (`1`, `1.1`, `2`)
  makes a long sidebar navigable and makes "which row is this?" answerable.
- **Decide your delegate model tier** — the guiding session does not have to be
  the expensive one, and a delegate that grinds usually should not be.
- **Write your own laws into the campaign file as you learn them.** The valuable
  ones are almost always of the form *"this instrument answers a different
  question than its name suggests"*.

⚠ **Do not put anything private in a repo.** Row titles, campaign contents and
directory layouts describe how someone works and what they are working on.
Invent every example you commit — including in tests and fixtures.

---

## ⛔⛔ CHOOSING THE BOOT DELAY — it is a DECISION, and the prompt cache is a first-class input

**Recorded 2026-08-11, after a relay woke 30 minutes AFTER a scheduled event it existed to cover.**
The failure had two halves that look opposite and are the same mistake: **several no-op wakes during
the run-up, and then arriving late to the event itself.**

**`boot_after_secs` is configurable to any agent's liking. That freedom is the point — a default
grid is not a plan.** Two constraints govern the choice, and they are usually satisfied by the
SAME number:

### 1. ⭐ ARM TO THE NEXT EVENT, NOT TO A GRID

If something happens at a known wall-clock time T, the correct delay is **`T − now`**, clamped to
the booter's range. One wake, at the moment that matters.

⛔ **The anti-pattern, and it is what he caught:** a fixed short window (say 7 min) repeatedly
re-armed through the run-up to T. It spends N turns discovering nothing is due, and because each
re-arm restarts from *the moment the turn ended*, the wake times drift off T entirely — so you can
burn several turns AND still arrive late. **A short window is not "safer" than an aimed one; it is
just noisier and it loses the aim.**

```
BAD   08:55 arm 420s → 09:02 nothing due → arm 420s → 09:09 nothing due → … → 09:45, event was 09:15
GOOD  08:55 arm (09:15 − 08:55) = 1200s → ONE wake, at 09:15, for the event itself
```

### 2. ⭐ CACHE HOTNESS IS HIGH PRIORITY — and it sets a CEILING on the delay

The prompt cache holds roughly an hour. A wake inside that window resumes on a warm cache; a wake
outside it pays a **cold re-read of the entire context**, which on a long-running session is the
single largest avoidable cost there is. So:

- **Never let the gap exceed the cache TTL when you are going to be woken anyway.** If the next
  event is 3 hours out, do not arm 3 hours; arm inside the TTL. A cheap turn that reads one line
  and re-arms costs far less than one cold re-read.
- ⚠ **But cheap turns are not free either** — each is a real turn. ⇒ **the target is the FEWEST
  wakes that (a) hit every event on time and (b) never let the gap exceed the TTL.** That is a
  small optimisation problem and it has a right answer; solve it, do not default.
- ⛔ **A deliberately cold gap is a legitimate choice — but SAY SO.** "I am letting this go cold
  because nothing is due for six hours and the re-read is cheaper than nine keepalive turns" is a
  decision. Drifting into it is not.

### Worked cases

| situation | delay to arm | why |
|---|---|---|
| known event at T, inside the TTL | **`T − now`** | one wake, on time, cache warm |
| known event at T, beyond the TTL | **TTL − margin**, then re-aim next turn | keeps the cache warm; the LAST hop lands exactly on T |
| a phase/state boundary at B, then a different regime | **`B − now`** | a window must not outlive the reason it was chosen |
| nothing due for a long stretch | TTL − margin, **or** an explicit cold gap you name | either is fine; silence is not |
| an event whose time you do NOT know | shortest window you can justify | this is the only case a tight grid is right |

⇒ **Before every re-arm ask two questions:** *what is the next thing that actually matters, and
when is it?* and *does this delay leave the cache cold?* If you cannot name the next event, say so
— that admission is itself information about whether the watch is aimed at anything.

### ⚠ AND WHEN A BOOT ARRIVES LATE, MEASURE BEFORE YOU BLAME YOUR OWN ARMING

The case above was written accepting that an unaimed short window caused a 30-minute-late wake.
**Then it was measured, and the arithmetic did not support that:** `armed 420 s` at T, a watcher
polling on a ~5-minute grid ⇒ worst case **T + 12 min**, which is exactly what the tool itself
prints. The actual wake was **T + 40 min**. Roughly 27 minutes are unexplained by the window.

⇒ **Two claims, and merging them costs someone a real defect:**

- **The doctrine is independent and stands:** aim at the event, cap by the cache TTL, prefer one
  aimed wake to several unaimed ones. That is better arming *whatever* the booter did.
- **The lateness is a separate, UNMEASURED fact.** Candidates: missed polls; the turn not being
  seen as ENDED; or a boot issued and never delivered — a shape this fleet has hit before, where a
  delivery verb accepted what it never delivered.

⛔ **The audit trail is the load-bearing part.** In that incident the booter's decision log had been
stale for ~21 hours while its heartbeat was live, so the decision trace for the window in question
**did not exist to read**. A scheduler whose decisions cannot be audited can be neither exonerated
nor convicted. **If your scheduler writes a heartbeat but not a decision log, that gap IS the
finding — report it.**

⭐ **The general rule: accepting blame you have not verified is still an unmeasured claim.** It
merely fails in the flattering direction, and it hides the defect. When a wake lands materially
later than `armed + poll interval`, record the gap with both numbers and route it to whoever owns
the scheduler — then fix your arming anyway, because that was worth fixing on its own.

---

## ⛔⛔ A MESSAGE TO ANOTHER ROW IS A **WAKE**. PRICE IT LIKE ONE.

**Recorded 2026-08-11.** A single session sent four cross-session messages, one of them to an
unrelated **idle** row, which then spent a full turn on a **cold** cache measuring a claim it had no
stake in. Two of the remaining three were redundant and should have been batched into the first.

⭐ **THE ROOT CAUSE, and it generalises past messaging:** that same session had, an hour earlier,
written a careful cost model for *its own* wakes — aim the boot at the event, never let the prompt
cache go cold, a cold re-read of a large context is the biggest avoidable cost there is. **It applied
none of that to wakes it inflicted on others.** A `SendMessage` to a row whose turn has ended IS a
boot: identical cost event, charged to somebody else's budget, without their consent, and worst
precisely when they are idle and cold. **Whatever discipline you apply to your own cadence, you owe
to every row you address.**

### The pre-send test — the DEFAULT IS NOT TO SEND

1. **Is the information already durable somewhere they will read?** The campaign memory dir, its
   `MEMORY.md` index, a repo doc, their queue file. **If yes → FILE IT AND STOP.** A message adds
   *urgency*, never *information*. Filing is free and permanent; a message costs a turn and is gone.
2. **Is it time-critical to THIS turn of theirs?** Not "useful", not "they'd want to know" —
   *actionable before their next natural wake*. If no, it is not worth a wake.
3. **Is the recipient IDLE?** Then the cost is maximal, not minimal. Idle is when the bar goes UP.
   A busy row is already warm and already paying; an idle one is asleep and cold.
4. **Have you already messaged them this session?** ⇒ **BATCH.** One message per recipient per
   session unless something genuinely new AND urgent has appeared. Three sends where one would have
   done is three wakes, and the 2nd and 3rd are pure waste.

### ⛔ AND ADDRESS ON STRUCTURED IDENTITY, NEVER A DISPLAY TITLE

The wrong-row wake happened because the sender picked a target out of `ListAgents` **by a title that
looked right**, discarding the UUID its own spawn had returned moments earlier.

⚠ **`ListAgents` is not a complete index of reachable rows.** In that incident the correct successor
— demonstrably running, transcript growing, holding the campaign's booter subscription — **did not
appear in the listing at all**, while an unrelated row with a similar name did. So "pick the
plausible one from the list" was *guaranteed* to be wrong.

⇒ **Resolve the recipient's UUID first** (the spawn's own `session_path`, or `server app rows`), and
match on it. **If you cannot resolve a name to that UUID, DO NOT GUESS — deliver by file.** That is
the documented fallback and it is strictly better than waking a stranger.

⭐ This is the same law as [[an alarm addressed by a self-chosen title rang into an empty room]]:
**an address is only an address if you can prove what is behind it.** A title is a label a row gave
itself; a UUID is what the system knows.

### Anxiety is not a reason to re-send

The duplicate sends in that incident came from a failed delivery earlier: the target had stood down,
the send errored, and when a similarly-named row appeared the sender tried again. ⛔ **A failed send
is not a debt you must immediately repay.** File the content once, durably, and let the campaign's
own door surface it. **Retry only if something is ON FIRE**, and then say plainly in the message
that it is a retry and why.

### ⛔⛔ THE OWNER NAMING A ROW IS AN INPUT, NOT THE DECISION — verify it, price it, then choose

**Recorded 2026-08-11, as a standing rule.** A routing instruction that names a recipient — from a
human or from another agent — is an **input to a decision, not the decision**. It carries the same
status as an inherited `BLOCKED`: **a named recipient is a CLAIM.** The row may be misremembered,
retired, busy on unrelated work, or may never have existed. Verifying it is expected, and choosing a
cheaper route when the measurement disagrees is the correct outcome rather than a deviation.

### The procedure — four measurements, then a choice, and the default is CHEAPEST-WARM

```sh
# 1. DOES IT EXIST? Resolve to a UUID. Never accept a title, never substitute a similar name.
ssh <gui-host> '<cli> server app rows'   # match on outline_prefix / full_path, not session_title

# 2. HOW COLD?  transcript mtime = time since its last activity
# 3. HOW BIG?   transcript bytes  = roughly what a cold wake must re-read
python3 - <<'P'
import os,time; p=os.path.expanduser("~/.claude/projects/<slug>/<uuid>.jsonl")
st=os.stat(p); print(f"idle={(time.time()-st.st_mtime)/60:.1f}min size={st.st_size/1048576:.2f}MB")
P
```

**4. Then price the options and pick the cheapest that actually achieves the goal:**

| option | cost | when it wins |
|---|---|---|
| **FILE IT** (campaign memory + its `MEMORY.md`, a queue doc, a repo file) | **~zero**, permanent | **the default.** Anything not actionable before their next natural wake |
| **message a WARM row** | one interruption; no cold re-read | genuinely time-critical AND they are already awake AND it is their lane |
| **message a COLD row** | **the expensive one — a full cold re-read of their context. One measured instance: a ~2 MB idle row, several US dollars, incurred in about a second** | almost never. Only if something is on fire |
| **spawn a fresh row** | a new small context + a brief you must write | the work is substantial, ongoing, and nobody warm owns it |
| **⭐ DO IT YOURSELF** | your context is **already warm and already paid for** | far more often than agents assume — see below |

⭐ **"DO IT YOURSELF" IS SYSTEMATICALLY UNDER-CHOSEN.** The cheapest actor is usually the one
already warm, and that is normally you. Waking a cold large-context row to do ten minutes of work is
absurd when you could do it inside a context whose cost is already sunk. ⚠ The legitimate reasons to
route away are **ownership** (it is another campaign's instrument and yours to report, not to fix),
**context budget** (you are near your own boundary), and **duration** (it outlives your session) —
not mere tidiness about whose job it nominally is.

⛔ **AND "I WAS TOLD TO" IS NOT A COST ARGUMENT.** If the measurement says the content is already
filed where that row will read it, then the delivery is DONE and a message buys nothing but an
interruption. **Report what you measured and what you chose** — the weighing is the deliverable, not
the obedience.
