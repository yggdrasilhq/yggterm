---
name: yggterm-agent-fleet
description: What an agent CLI gains by running inside yggterm — its own addressable row, the ability to spawn and verify delegate sessions, to message any other session, to read its own context budget, and a one-time bootstrap that wires a durable memory + campaign system. Read this before spawning any session, before handing work to another agent, or when starting a long campaign that must outlive one context window.
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

### Title and seat are separate, on purpose

`server app session outline <row> <prefix>` stores a number APART from the
title, composed at render time. So a CLI that re-titles itself cannot destroy
its own position. Set the seat once; let the CLI name itself.

```sh
yggterm server app session outline "$ROW" 4.2      # seats it; "" clears
yggterm server app session rename  "$ROW" "topic: what I am actually doing"
```

---

## 2. Know your own context budget

An agent that runs out of context mid-campaign loses everything it had not
written down. You cannot read your own token count directly — but you can ask
your own session, because a row can be sent input like any other:

```sh
printf '/context' | yggterm server app terminal submit "$ROW" --stdin
```

The readout arrives as your next turn. **Do this at natural checkpoints on any
long run, not when you feel full** — by then the cheap remedy (write the
handover now) may no longer fit.

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

### ⚠ The readiness probe is not free

yggterm proves a CLI is reading by WRITING a probe string and watching it echo.
That is sound only against a program already reading. Against one that is not,
the bytes are **queued, not discarded** — a PTY buffers — and they arrive later,
in the composer, as the delegate's opening message. So:

- **Never treat "not ready" as "nothing happened".**
- **Never spawn and walk away.** Which is what the next section is for.

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

## 6. Adapting this to your own setup

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
