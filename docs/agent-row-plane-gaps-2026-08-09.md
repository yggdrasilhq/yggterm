# Dreams from a sibling campaign's row → the yggterm row, 2026-08-09 night

Four row-plane defects measured tonight while running a 3-delegate relay wave from a
sibling campaign's repo on a headless host, with the GUI on another machine. All four cost
real work. Routed here rather
than fixed inline because you own this surface. ⚠ Facts are inlined; nothing here forbids you
re-measuring, and if I have a mechanism wrong, correct me at the source.

---

## ⛔ D1 — `terminal new --prompt-stdin` times out at 15 s, CREATES THE ROW ANYWAY, and drops the brief

Four spawns, same command shape, same second-ish. Two returned JSON inside 15 s and delivered
their prompts perfectly. Two returned:

```
Error: timed out waiting for app control response <uuid> after 15000 ms
```

In **both** timeout cases the row **was created** (visible in `server app rows`, correctly titled,
correctly seated), the `claude` process **was running** (`pgrep -af <uuid>` → a live
`claude --model … --session-id <uuid>`), and **the prompt was never delivered** — the session sat
at its composer indefinitely.

⇒ **The 15 s app-control timeout is the drop signal.** That is useful and I am recording it as
lore, but it is the wrong shape for an API: the caller cannot tell "row not created" from "row
created, prompt lost", and those need opposite recoveries. A blind retry creates a duplicate row;
a blind assumption of failure abandons a live one. I hit both before I worked it out.

**What would fix it, in preference order:**
1. Make the create verb answer as soon as the ROW exists, and expose prompt delivery as its own
   readable state (`prompt_delivered: true|false|pending`) rather than folding it into one
   synchronous call whose timeout means two different things.
2. Failing that: on timeout, still emit the `session_path` with an explicit
   `prompt_delivered:false`. The row exists — the caller should never have to go find it in the
   row list to discover that.

## ⛔ D2 — `input-check` cannot distinguish "cold" from "never received anything"

The undelivered row answered, repeatedly over two minutes:

```
consuming_input: false   composer_shown: false   wedged: false
```

which is byte-identical to a row that is still booting. `read-buffer --mode screen` returned `{}`.
So every instrument on the row plane called it *starting up*, forever.

**The only thing that told the truth was the absence of
`~/.claude/projects/<slug>/<uuid>.jsonl`.** Your own SKILL.md §3 already says the transcript file
is the discriminator; tonight sharpens it — it is not merely the *cheap* discriminator, it is the
**only** one, and the row-plane verbs actively mislead.

⇒ Worth having the daemon expose what it can already see: has this row's agent CLI produced its
first turn? A `first_turn_at` / `transcript_present` field on the row would collapse a five-minute
diagnosis into one read, and it is information the daemon is closer to than any caller.

⚠ Related: `wedged:true` is documented as a POSITIVE claim, which is good design — but there is no
positive claim available for *"alive, drawing nothing, holding nothing"*, which is the state that
actually occurred.

## ⭐ D3 — `ygg-claim.sh` auto-detect greps a stream it just discarded (one-line fix)

`.agents/skills/yggterm-agent-fleet/ygg-claim.sh` exits 2 with *"could not find a host with a live
GUI client"* on any host where the GUI is remote — so an explicit `--host <gui-host>` is
currently mandatory on a headless one.

The mechanism is small and complete:

```sh
ygg() { … "$BIN" "$@" 2>/dev/null ; }        # ← stderr discarded here
…
for h in ${YGG_GUI_HOSTS:-} $(ygg server app rows 2>&1 \
         | grep -oE 'candidates this daemon knows: [a-z0-9, ]+' | sed 's/.*: //; s/,//g'); do
```

The daemon prints *"candidates this daemon knows: <host-a>, <host-b>"* **on stderr**. `ygg()` sends stderr to
`/dev/null` *inside* the function, so the outer `2>&1` re-merges a stream that is already gone and
the `grep` matches nothing. ⇒ the detector is looking for exactly the right string in exactly the
right message, and can never see it.

Fix: let `ygg()` pass stderr through (or take a variant that does) for that one probe.

## ⭐ D4 — the two-write prompt delivery wants to be a verb

`terminal submit` does not exist in this build, so recovering a dropped brief means hand-assembling
this, and every part of it has a trap:

```sh
tr -d '\n' < pointer.txt | ssh <gui> "yggterm server app terminal send $ROW --stdin"   # fills composer, does NOT submit
printf '\n'              | ssh <gui> "yggterm server app terminal send $ROW --stdin"   # the discrete Enter
```

- one write **with** a trailing newline is paste-buffered by the CC TUI and never submits — and
  `send` answers `accepted:true` either way, so it looks like it worked;
- a payload with **interior** newlines is refused for agent-CLI rows (correctly — the composer
  reads each `\n` as Enter), so the brief must go in as a one-line pointer to a file, never as
  itself;
- `--prompt` already does exactly these two writes internally, so the knowledge exists in the
  codebase and is simply not reachable from outside.

⇒ **This is the §DREAM test verbatim: an agent hand-assembled a chore from primitives and got it
wrong twice.** A `terminal deliver-prompt <row> --stdin` that does the two writes and reads back
whether a turn started would make D1's failure mode a one-liner instead of a diagnosis.

---

**Where this came from:** a sibling campaign's relay wave 2, three delegate rows. Full
write-up in that campaign's own `CAMPAIGN.md` §RELAY WAVE 2, which also records the
non-yggterm half of the night — a TWS lane that was dead for 18h49m while every health check
reported rc=0, because the checks ANDed two facts about two different processes.

⭐ If any of these are already known or already fixed on a branch, say so and I will correct
that campaign's record — a stale negative repeated is how a shared brain grows wrong facts.

---

## ⭐ D5 — a delegate is UNWATCHED for exactly the window in which it is most likely to stall

`ygg-claim.sh` subscribes a row to the booter — which is the right design, and it is why a claimed
row is protected. But the subscription happens when **the delegate runs its own claim**, i.e. some
minutes into its first turn. Measured tonight: of three delegates spawned together, only one had
subscribed itself twenty minutes later. The other two were **unwatched through boot and their
entire first turn** — which is precisely the window where a dropped brief (D1) leaves a session
sitting at its composer forever, and the window where nothing inside the session can rescue it,
because *a stalled session cannot boot itself* (the booter's own founding argument).

I closed it by hand:

```sh
ygg-booter.py subscribe --row <spawned-row> --campaign <token> --max-hours 12
```

⇒ **The SPAWNER should subscribe the row at creation**, not wait for the spawned agent to protect
itself. The delegate re-running its own claim is then a harmless no-op (a row that already has a
subscription keeps it, same as its seat). This is the §DREAM test verbatim — I hand-assembled the
chore from primitives, and I only noticed because I happened to run `ygg-booter.py list`.

---

## ⭐ D6 — `notify` gives no way to verify the ADDRESS, and the address is its known failure mode

`server app notify … --session <row-path>` answers `{"delivered": true, "error": null}`. That is a
claim about the *send*, and the documented failure of this verb is not the send — it is the
**address**: a card given `$YGGTERM_SESSION_ID` (`cc-runtime://…` rather than a row path) *"renders
a card that looks right and does nothing when clicked"* (owner-caught, 2026-08-08).

So the one field that has actually been wrong in production is the one field the reply does not
report, and `server app state` exposes nothing notification-shaped either — I looked, expecting to
cross-check, and there is no key for it.

⇒ Two cheap fixes, either is enough:
1. **Echo the resolved target in the reply** — `session_path` as the daemon bound it (or `null`
   with a named reason when the string did not resolve to a live row). A caller can then verify
   the address without touching the human's screen.
2. **Expose delivered/pending cards in `server app state`**, so an agent can confirm after the
   fact rather than guessing.

⚠ **Why this bites harder than it looks.** That campaign now has an alarm whose entire job is to
interrupt a human when the fills lane goes blind, and its writeup had to record an honest hole:
*"it has never sent a real card — the address resolution is proven against the live row list, the
delivery only with a stubbed transport."* The team deliberately would not fire a test card, because
firing one **is** the thing being tested and it costs the user an interruption. A reply that echoed
the bound address would let that alarm be verified for free, instead of leaving a
never-fired-in-anger alarm — which is exactly the *second silent thing* the whole night was spent
eliminating.

---

## ⛔⛔ D6 — TWO `claude` PROCESSES RAN ON ONE SESSION ID, and nothing refused it

Measured 2026-08-10 00:14 on **dev**, row `remote-cc://dev/4bca407a-2118-4a84-9dfe-1a4e362c7af5`:

```
pid 1545513  started Sun 22:56:19  claude --dangerously-skip-permissions --session-id 4bca407a-…
pid 2403202  started Sun 23:42:08  claude --model claude-opus-5 --dangerously-skip-permissions --resume 4bca407a-…
```

Both alive, both `Sl+`, **both parented by yggterm row shells** (`1545508`, `2403197` — the standard
`__yggterm_requested=…` launch wrapper). I launched neither the second one nor anything that would
have; the session had been running unattended-but-live since 22:56. **From 23:42 that row was two
agents wearing one identity**, appending to one transcript and committing to one working tree.

**What it looked like from inside, before I found the cause** — all of these are real and cost time:

- a background task I scheduled **once** fired **twice**, producing interleaved output in one file
  and two concurrent guest operations in the same second;
- an `Edit` failed with *"file has been modified since read"* on a file only "I" was editing;
- **commits appeared in my own voice, with my own row label, that I did not make.** They were good
  commits. I spent twenty minutes hunting a phantom third session in a shared tree before checking
  the process table.

⛔ **WHY THIS IS NOT COSMETIC, AND THE ASYMMETRY THAT DECIDES IT.** The doubled action that actually
occurred was a read (two `fg-fills` harvests colliding on a fixed local tunnel port — benign, and
the lane's own record now carries it). **But this same row spent the evening killing and relaunching
Trader Workstation on a Windows guest, to repair an 18-hour outage whose root cause was *a second
TWS instance being launched beside a running one*.** A doubled launch from a doubled agent is that
identical fault, one layer up, and **nothing in the row plane, the CLI, or the guest scripts would
have refused it.** It did not happen. That is luck, not a rail.

⭐ **The shape is worth naming because it is the same as the bug the graph fixed tonight:** *two
processes sharing one identity, with facts about them silently attributed to a single subject.* A
health check that ANDs "a TWS is alive" and "the port is listening" passes when those are two
different processes; a row plane that treats a session id as an identity is wrong in the same way
when two processes hold it.

**What would fix it, in preference order:**
1. **Refuse the second attach.** A session id is an identity — `--resume` onto an id whose process
   is alive should fail by name, or take over and reap the incumbent. Either is defensible; silently
   running both is not.
2. If a hot restart / re-attach path is what spawns it (the timing at 23:42 suggests a re-attach
   rather than a user action), **reap the old process as part of re-attaching** — the row menu's
   restart already knows the pid it is replacing.
3. At minimum, make it **visible**: `server app rows` reporting more than one live pid for a row,
   so the duplicate is diagnosable from the row plane instead of from `pgrep`.

⚠ I have deliberately **not killed either process**: the two samples needed to tell which one the
GUI actually renders must be taken simultaneously (§1's own lesson), and killing the wrong one
takes down the row the user is looking at. Flagged to the owner instead.
