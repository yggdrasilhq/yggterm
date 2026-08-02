# The delegate-session plane — costed gaps and feature asks (2026-08-02)

Written by the IP-estate guiding session after doing the full loop by hand
twice in one afternoon: find a working agent's open question from another
session, answer it, then launch a delegate Opus session in a yggterm row for
the eMudhra copilot job. The user has made this a standing pattern (*"We
continue like this"*, *"I want this pattern of work to be a cakewalk"*, *"We
should have easy for you (agents) access layer on yggui automations"*). Every
ask below carries the concrete cost paid today without it. Companion recipe:
data-fabric skill §THE BG-SESSION PLANE. Precedent format:
[`agent-cobrowse-gaps-2026-07-28.md`](agent-cobrowse-gaps-2026-07-28.md).

## What already works and must not regress

- `terminal new --kind shell --machine-key <host> --title --purpose
  --no-activate` creates a cross-machine row without moving the user's view.
- `terminal send --stdin` reaches the PTY; a typed `claude --model … "prompt"`
  launches a delegate the user can answer in his GUI.
- The metadata pane already shows type/machine/cwd/title/PID/session-id/
  versions — the "superpowers" the user names. The gap is agent-side VERBS to
  reach the same facts.
- `live_session_snapshot_debug` in `server app state` exposes row order,
  titles, session paths, working state — everything needed to find a session —
  but it is a debug field, not a contract.

## Ranked feature asks

### 1. First-class delegate launch on `terminal new` (or an automations verb)

**What:** `terminal new --kind claude-code|codex --model <id>
--permission-mode bypass --prompt-file <f> [--brief-file <f>]` — or the
automations-layer equivalent: `automation run delegate --model claude-opus-5
--bypass --brief <f> --machine dev --title <t>` (docs/automations.md I1–I5
already own scheduled agent-CLI sessions; this is the interactive sibling).
**Why beyond today:** every future delegate launch; the user has declared this
the standing work pattern.
**Cost paid:** `--kind claude-code` was unusable because it inherits the
user's default model — he had just set Fable as default, the exact tier he was
delegating AWAY from; the shell-kind workaround needed fragile printf quoting;
the first launch shipped without bypass and stalled in plan/auto mode; total
two relaunches and one dead TUI that swallowed a relaunch command into its
prompt box.

### 2. A session BRIEF as first-class metadata

**What:** `--brief <file>` stored by the daemon at session creation;
`server app session brief <session>` returns it; the metadata pane shows it.
**Why:** "bg sessions understanding fast" — a guide, the user, or any later
session pulls a delegate's mission without transcript archaeology; the brief
survives resume and daemon handover.
**Cost paid:** briefs live in the guiding session's scratchpad with the path
smuggled through the launch prompt; nothing about the row says what it is for
beyond one `--purpose` line; a killed-and-relaunched delegate needed the path
re-sent by hand.

### 3. Agent read layer over session transcripts and OPEN questions

**What:** `server app session transcript <session|uuid> [--last N]
[--text|--json]` resolving the CC/codex JSONL cross-host; and
`session questions <session>` returning the currently OPEN AskUserQuestion
(question, tabs, options) structurally.
**Why:** guiding is the pattern; the guide's first need is "what is this
session doing and what is it asking".
**Cost paid:** ~10 probes and four hand-rolled python JSONL parsers to find
one open dialog, because a pending AskUserQuestion reaches the JSONL only when
ANSWERED and exists nowhere else but the painted screen; the user had to
correct my session identification ("fifth not fourth") mid-hunt.

### 4. Daemon-side screen read for never-activated rows

**What:** `terminal read-buffer --source daemon <session>` serving the
daemon's vt100 screen when no client has activated the row.
**Why:** any observer of a background row; today the client buffer answers 65
blank lines, which reads as "session dead".
**Cost paid:** the first working-agent hunt bounced off a blank read-buffer
and fell back to JSONL archaeology; the open-dialog read (ask 3) only worked
because the user happened to have activated that row.

### 5. Row deixis — stable GUI-order addressing

**What:** promote row order to a contract: `server app rows` returning rows
exactly as the sidebar draws them, indexed; accept `--row N` where a session
path is accepted.
**Why:** the user speaks in "the top 4th session row"; agents resolve it
through a debug field and hope the order matches.
**Cost paid:** one wrong-session identification and a user correction.

### 6. Spawn-verified send

**What:** `terminal send --expect-child '<argv regex>'` (or `terminal exec`)
that answers with the spawned child pid or a named refusal.
**Why:** `accepted:true` is the injector's assumption everywhere on this
plane; launches need the pgrep loop hand-rolled each time.
**Cost paid:** three pgrep round-trips today, and one send that landed inside
a live TUI's prompt box because nothing on the send path knew the old child
still existed.

### 7. Ready-gated send

**What:** `terminal send --when-ready` blocking (bounded) until the PTY child
is at a prompt.
**Why:** kills the "sleep ~3s or be silently swallowed" folklore that every
recipe carries.
**Cost paid:** the folklore sleep, again, in every launch this afternoon.

### 8. A guide channel with attribution

**What:** `session say <session> --from <agent-id> 'text'` queuing an
attributed message into a CC/codex session (and refusing while a dialog is
open, with the dialog returned — composes with ask 3); optionally
`session answer-question --option N --notes '…'` for deterministic dialog
answers when the user delegates that.
**Why:** guide-to-delegate steering is the pattern's second half; today it is
raw keystroke injection, unattributed and able to race an open dialog on the
user's seat.
**Cost paid:** the libyggterm agent's three-tab dialog could only be answered
by the user personally after I published a brief to a scratchpad path; my
steer messages arrive looking like the user typed them.

## For the skills (data-fabric / yggui-app-control)

- data-fabric §THE BG-SESSION PLANE added 2026-08-02 (recipe + five traps) —
  keep it in sync as the verbs above land; the section is the contract most
  agents can reach (§A2 duty).
- yggui-app-control should gain the same section or a pointer once ask 1
  ships, so the launch recipe has one owner.
