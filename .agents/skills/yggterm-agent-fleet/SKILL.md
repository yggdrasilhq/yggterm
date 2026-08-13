---
name: yggterm-agent-fleet
description: What an agent CLI gains by running inside yggterm — its own addressable row, the ability to spawn and verify delegate sessions, to message any other session, to read its own context budget, and a one-time bootstrap that wires a durable memory + campaign system. Read this before spawning any session, before claiming a row at the start of a campaign (§1), before SUCCEEDING a session that has gone cold (§6 — harvest its transcript, never prompt it), before trusting any row-management verb's own success field (§7), before HANDING OFF a campaign to a successor (§8 — the baton relay, and how to write the brief), and before messaging another campaign or recovering a stalled one (§9 — cross-talk and the single `continue`).
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

### The four steps. Do not collapse them.

```sh
# 1. CREATE with no prompt. A prompt passed at create is delivered by a path
#    that has silently dropped briefs; see the caveat below.
ROW=$(yggterm server app terminal new \
        --kind claude-code --machine-key <host> --cwd <dir> \
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

# 4. ⛔ VERIFY BY TRANSCRIPT CONTENT. This is the only step that cannot lie.
grep -q 'PUT-A-DISTINCTIVE-TOKEN-IN-YOUR-BRIEF' \
     ~/.claude/projects/<cwd-slug>/<uuid>.jsonl
```

### Why step 4 is not optional

**A transcript FILE exists the moment the CLI starts.** It tells you a process is
running; it tells you nothing about what was delivered into it. A launch that
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
completion report, `pgrep -P <pid>` shows no children, and its work is committed. Then `TERM`,
**read `/proc/<pid>` back**, and escalate to `KILL` only if it survives.

⚖ **Whose job:** the session that SPAWNED it. Not the delegate — it cannot reap itself after its
last turn — and not the next human to notice.

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

## 6. ⛔ Succeed a session that has gone cold — HARVEST IT, never ask it

**A long-running row drifts into the worst cell of a two-by-two: a COLD prompt
cache holding a LARGE context.** Those costs multiply rather than add — every
turn re-writes a huge input at full price instead of reading it cheaply — so one
turn on such a row can cost more than the entire remaining job.

The remedy is a **succession**: harvest what the session knows from artefacts that
cost nothing to read, write it somewhere durable, and continue in a fresh, small,
warm session. Both problems disappear at once.

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
   for the human. Then it kills you.
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
  answer them. Research, subagents and fan-out are all in scope here.

⇒ **What is worth forbidding is re-deriving a SETTLED fact, never investigating
an UNSETTLED one.** ⚠ And *"do not re-derive"* is not *"do not verify"* — an
inherited fact can be wrong, and stale baselines passed down a relay are a
documented failure.

---

## 9. Cross-talk — campaigns that answer each other

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

### Stall recovery — a stopped session is usually one word from resuming

**A session's dominant failure is STOPPING, not dying**, and the two look
identical from outside. Signature of a stall: **the turn ENDED, the work is
unfinished, and the transcript shows no error, no API failure and no model
fallback.** Causes are mundane — a CLI hiccup, a transient API error, a model
demotion mid-turn.

⇒ **That state is recoverable by a single `continue`.** A monitor that only
*detects* stalls and tells a human is doing half the job.

⛔ Three guards, or the cure is worse than the disease:
- **Once per stall, never per poll.** A watcher that re-nudges every tick is
  worse than one that never nudges.
- **Only ASSIGNED sessions.** A session parked by design is *supposed* to be
  idle; nudging it trains its reader to ignore the alarm.
- **Escalate if it does not resume.** One unanswered nudge means the fault is not
  a stall, and a human should hear about it.

---

## 10. ⭐⭐ THE N.x ORCHESTRATOR — cluster the remaining work, then run the clusters in parallel

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
| **`N`** — the orchestrator | clusters the work, writes the briefs, launches, monitors, merges, reaps. It does **not** fix bugs. |
| **`N.1` … `N.k`** — the clusters | each owns one cluster and relays *within* it: a cluster session hands off to its own successor until its cluster is done, then reports up. |

Seat them as a book — `6`, `6.1`, `6.2` — so the sidebar reads as the plan. The
orchestrator takes the bare integer; a cluster that spawns its own successor
keeps the same seat.

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

⚠ **A childless top-level row is a different case** and still wants `N.` from the
renderer — that one has no sub-seats to be consistent with.

**The seat lives in `outline_prefix` and nowhere else.** The sidebar composes
`label = "<outline_prefix> <title>"` at render time, so a title that also carries
the number gets it twice.

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

---

## 11. Adapting this to your own setup

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
