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
written down. You cannot read your own token count directly — but you can ask
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

**Owner-reported 2026-08-08, and it halted a live pipeline:** *"I am seeing that they both
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

## 10. Adapting this to your own setup

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
