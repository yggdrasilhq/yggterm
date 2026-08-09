# Dreams from atlasgraph row 5.2 → the yggterm row, 2026-08-09 night

Four row-plane defects measured tonight while running a 3-delegate relay wave out of
`/home/user/data/atlasgraph` on **dev**, GUI on **guihost**. All four cost real work. Routed here rather
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
GUI client"* on any host where the GUI is remote — so `--host guihost` is currently mandatory on dev.

The mechanism is small and complete:

```sh
ygg() { … "$BIN" "$@" 2>/dev/null ; }        # ← stderr discarded here
…
for h in ${YGG_GUI_HOSTS:-} $(ygg server app rows 2>&1 \
         | grep -oE 'candidates this daemon knows: [a-z0-9, ]+' | sed 's/.*: //; s/,//g'); do
```

The daemon prints *"candidates this daemon knows: guihost, oc"* **on stderr**. `ygg()` sends stderr to
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

**Where this came from:** atlasgraph relay wave 2, rows 5.2.4/5.2.5/5.2.6. Full write-up in
`~/data/atlasgraph/CAMPAIGN.md` §RELAY WAVE 2 (commit `29d76d8`), which also records the
non-yggterm half of the night — a TWS lane that was dead for 18h49m while every health check
reported rc=0, because the checks ANDed two facts about two different processes.

⭐ If any of these are already known or already fixed on a branch, say so and I will correct
atlasgraph's record — a stale negative repeated is how the fabric grows wrong facts.
