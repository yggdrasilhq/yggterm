# CLAUDE.md

Read `AGENTS.md` in full before starting any task. It is the authoritative engineering contract for this project.

## Why yggterm exists (read before every session)

yggterm replaces the chaotic pre-existing workflow of VSCode terminals + tmux + ssh + `codex resume` / `claude -r` across multiple machines, where the user loses track of sessions across machines and has to redo the ssh+cd+resume mechanics every time the editor restarts.

**Core value proposition:** when the user clicks an agent session in the cwd tree (Codex, Claude Code, future first-class agent CLIs), yggterm performs the equivalent of `ssh <machine> "cd <cwd> && codex resume <UUID>"` (or `claude -r <UUID>`) and hands off the terminal. The user just types. **This handoff is the product.**

**First-class vs second-class:**
- First-class: agent CLI sessions (Codex, Claude Code, future). Organized by cwd in the tree. Persist by default (the agent CLI itself persists via JSONL; yggterm's job is to faithfully invoke `codex resume` / `claude -r`).
- Second-class: plain shell terminals. Connect to the yggterm-server tmux-like layer. Survive GUI death IF marked keep-alive; otherwise die with the GUI.

**Yggterm does NOT:**
- Parse codex/CC JSONL into the terminal viewport. Terminal-view delegates rendering to the CLI itself.
- Reinvent the agent CLI rendering.
- Add CLI flags beyond the minimum needed for handoff (cwd, UUID, terminal-appearance env).

**The wrapper-vs-manual parity rule:** if a session opened via yggterm renders differently from the equivalent `ssh -t <machine> codex resume <UUID>` typed into a shell, that is a yggterm bug. The fix is in yggterm's wrapper/handoff/preservation path, NOT in adding flags to the codex command that the manual case doesn't use. Diagnose by running the manual command in a clean shell first and comparing.

The web view (separate surface) is where JSONL gets pretty-formatted into a chat UI — that path is in active development. Don't confuse the two.

Full mission statement: `[[project-purpose]]` in `~/.claude/projects/-home-user-gh-yggterm/memory/project-purpose.md`.

## ⚖ THE CONSTITUTION — why daemon and GUI are separate (read before every restart decision)

**The daemon/GUI split exists so that yggterm's own development and the other
agents' work never block each other.** That is not an implementation detail; it
is the reason the architecture has the shape it has, stated by the person who
built it:

> *"When I built daemon and gui separate architecture, I knew that yggterm agent
> work and other agents should work in harmony; in other words, we should not
> stall updating while the other agents work and they should not stall their work
> for our work to finish. This basic fundamental architecture philosophy should
> always be preserved."*

Two obligations, symmetric, and neither may be traded away:

1. **We never stall our work waiting for other agents.** Ship, bump, restart,
   deploy — whenever the work is ready.
2. **They never stall their work waiting for ours.** A restart of ours must not
   interrupt, reset, or destroy what another agent is doing.

### The behaviour this forbids

⛔ **Do NOT ask permission to restart, and do NOT defer a deploy because other
agents are working.** An agent doing this has misread the architecture as
fragile and is trading away obligation 1 to protect obligation 2. Both are
supposed to hold at once. Asking is not caution here — it is a symptom that the
mechanism is not trusted, and the correct response is to make the mechanism
trustworthy, not to route around it with a question.

**If a restart would genuinely disturb another agent, THAT IS THE BUG.** File it,
fix it. Do not treat it as a cost of doing business and do not ask the user to
absorb it.

### What the architecture must therefore guarantee

- **Version-coexisting daemons.** A new daemon starts and serves new work while
  older daemons stay alive, unchanged, still owning the sessions that were
  mid-flight on them — until that work finishes on its own terms. Finessing a
  new daemon in is normal; evicting an old one that is still working is not.
- **Other agents' sessions survive our restarts**, including their shadow
  surfaces, and including across a version bump.
- **Row identity, order and count survive** a daemon handover.
- **The drain must not require a quiet window.** A gate that only converges when
  nothing is active can never converge on a machine that is always active, which
  turns every deploy into a choice between waiting forever and killing PTYs.
- **Plain shells are first-class** and must survive a bump like anything else.
- **A session owned by an OLDER daemon is still a first-class row in the current
  GUI**, and clicking it must WORK. The user's case, stated directly: restart
  yggterm while an agent is mid-flight, see that agent's shadow session as a
  running row on the finessed older daemon, click it, and **co-browse it**. Not
  "observe that it exists", not "get an error about a version mismatch" — open
  it and share it.

  ⚠ **This is harder than it looks and the difficulty is our own doing.** The
  session/view contract currently assumes ONE viewer per session; the shadow
  client only works because it was made READ-ONLY and pinned to the daemon's PTY
  grid, which dodges the assumption rather than fixing it. Genuine co-browse
  means two live viewers of one session, with different window sizes, and that
  is the thing the pin exists to avoid. **Solve multi-viewer properly** —
  per-viewer geometry over a shared session — instead of widening the read-only
  hatch.

  The cross-version half is equally real: a declare proxied to a pre-2.12.10
  owner silently returned nothing, so mixed-version rows have already failed
  quietly once. Version-coexisting daemons only deliver the constitution if the
  CURRENT GUI can drive a row on an OLD daemon without the user ever learning
  that two daemons exist.

**The user must never have to know which daemon owns what.** Every guarantee
above is in service of that: daemon topology is our bookkeeping, not their
concern, and any friction that leaks it to them is a bug.

### Where this stood on 2026-07-26, honestly

Not yet true, which is why it is being written down. `kill -TERM` on a daemon
cost ~7 agent PTYs because the graceful self-retire defers while any session was
active in the last 300 s and therefore never converged under load; a plain
shell's row was lost outright; and row order was not preserved. Until those
hold, an agent may still have to make a judgement call — but the default is to
proceed and fix the mechanism, never to ask the user to schedule around a
weakness in our own design.

**This is the highest-value load-bearing work in the project.**

## Pending bugs — and the docs SSOT law

Open, user-confirmed bugs live in `docs/pending-bugs.md`. When the user says
"finish the pending bugs" (or similar), that file is the work queue. Remove an
entry in the same commit as its verified fix.

**When something needs the user, it goes in `docs/owner-attention.md` and the
work continues.** That file is the one answer to *"what is waiting on him?"* —
one line per item, pointing at the entry that owns the detail, never copying it.
It exists because the campaign runs unattended: an owner-gated step is parked
there and the relay takes the next load-bearing subset, rather than stalling.
⛔ Only genuine owner gates belong there — a decision only he makes, a credential
only he holds, a real-money action, a third party only he can chase. Work that is
merely hard is a queue item.

⛔ **`docs/docs-ssot.md` is the law for every status document, and it is
enforced** (`scripts/check-docs-ssot.sh`, run by
`docs_ssot::the_bug_file_lists_only_open_items`). One question, one owner: the
queue says what is OPEN, git + CHANGELOG say what SHIPPED, the field guide says
which instruments lie, the specs say how it should behave, the campaign memory
says why a call was made, and `docs/archive/` + `~/.claude/memory-archive/` hold
the past — searched, never loaded. **Never answer a status question from a file
that does not own it**; a duplicate is how a queue rots unseen, which cost a
session on 2026-08-02.

## Core working rules

### Single source of truth — no exceptions

Every concept has exactly one owner. Before adding code, name the source of truth for the thing you are changing. If two places could answer the same question, collapse them. Never add a second encoding, copy, derived field, or fallback layer that can silently diverge. This applies to session identity, sidebar rows, start page rows, icon kinds, CWD matching, launch commands, scan results, and every other domain concept.

### Specs are applied holistically

When a spec changes (e.g. start page shows all sessions, or CC sessions appear in CWD tree), apply it completely across every code path that touches that concept. Sidebar, start page, remote machines, local files, and any future surfaces must all reflect the same rule. Do not patch one callsite and leave another inconsistent.

### No non-determinism

Do not introduce behavior that differs based on timing, environment, or ordering that the code does not control. Scan results must be deterministic. Row injection order must be stable. Modified-epoch fallbacks must be explicit. If a function can produce different output for the same input, that is a bug.

### ⛔ Presentation flags are a LAW, not a knob (read before any render/compositing work)

`docs/presentation-policy.md` + `crates/yggterm-core/src/presentation_policy.rs`
are the SSOT for display backend, GL, frame delivery and video decode, per
platform. **Never set `GDK_BACKEND`, `LIBGL_ALWAYS_SOFTWARE`,
`WEBKIT_DISABLE_DMABUF_RENDERER`, `WEBKIT_DISABLE_COMPOSITING_MODE`,
`YGGTERM_WEB_SURFACE_UNDER_GLASS` or any other `PRESENTATION_VARS` entry against
the user's running GUI.** Test arms in the sandbox
(`scripts/underglass-sandbox.sh`), never on their machine.

Two traps that have each cost hours, more than once:

- **A Wayland session runs Wayland-native.** What you learned testing under
  Xvfb (where `GDK_BACKEND=x11` is correct) does NOT travel to the user's
  desktop. Restarting their GUI into XWayland changes compositing, input
  latency and the terminal renderer at once, and every measurement after that
  describes a machine they do not run.
- **`/proc/<pid>/environ` cannot answer "what is in force"** — these are applied
  with `set_var` after exec. Read the `gui/startup/linux_desktop_backend_policy`
  trace event, which is the decision reporting itself. For "is it XWayland",
  count X11 sockets in `/proc/<pid>/fd` — an empty `xwininfo` is a blind
  instrument, not a negative result.

If a default is wrong, change the TABLE and put the measurement in the row.

### Verify live, not just in code

For any UI change — button color, icon, layout, start page content, sidebar rows — take a live screenshot before and after using `/yggui` (see `.agents/skills/yggui-app-control/SKILL.md`). Do not mark a UI fix done until the live screenshot confirms it. App state and screenshot together are the proof; code review alone is not.

The live desktop host is defined in `.agents/config/live-host`. The yggterm binary on that host is `~/.local/bin/yggterm`. This is the only running instance of the app that matters for UI proof.

### Recurring self-verification missteps — READ before you type "healthy" / "fixed" / "verified"

These are mistakes I (the agent) have made repeatedly. They waste the user's time and erode trust. Re-read this list every time I'm about to claim a visual/terminal state is good.

1. **A visual bug needs a FAITHFUL PIXEL, not telemetry.** Squish, flicker, broken-bottom paint, blank viewport are things the *eye* sees. `session_view_contract_violations:[]`, a matching `cols×rows` grid, `base_y`, `launch_phase:Running` — NONE of these prove the canvas is painted correctly. I once called a session "healthy" off these fields while the user was staring at a squished, flickering, broken-bottom screen. **If the symptom is visual, the proof is a faithful screenshot. Full stop.**
2. **Take the faithful terminal screenshot — it now works in-process (v2.8.46).** `server app screenshot <out.png>` composites the xterm canvas IN the webview (`capture_backend=xterm_canvas_composite`, `capture_faithful=true`) — works over SSH, unfocused, any platform. `scp` it back and **Read the PNG**. Still check `capture_faithful`: if it fell back to `linux_webkit_snapshot` (only when NOT a terminal view, or canvas renderer off) that frame is canvas-blind — a `faithful:false` frame is a LIE about the terminal, don't reason from it.
3. **The daemon screen is NOT what the client painted.** `server snapshot → active_session.terminal_lines` is the daemon's vt100 screen (source of truth for *content*), but the squish/broken-bottom bug is precisely the CLIENT painting *less* than the daemon holds. Comparing daemon-to-daemon proves nothing about the client. The client-buffer read instrument (focus-independent buffer API) is the missing piece — wire/repair it rather than substituting the daemon screen.
4. **DEPLOYING RE-INTRODUCES THE SYMPTOM.** A daemon swap re-resumes codex on a fresh PTY → that re-resume window IS the squish/broken-bottom. So after every deploy the live surface is likely broken until codex repaints. Never declare a post-deploy session usable without looking; and never "deploy to measure" a symptom the deploy itself causes.
5. **`reconcile` / any `daemon_screen_snapshot` replay is DESTRUCTIVE on a healthy session.** It does a full reset + re-seed to the current (often sparse) screen, collapsing scrollback and risking a BLANK viewport (snapshot-poison) that needs a manual switch to recover. Only run it on a surface ALREADY confirmed broken — never "just to test" on a working session. I blanked the user's live session this way.
6. **The user's eyes outrank my instruments.** If the user reports a symptom and my probes say "fine," my probes are wrong — investigate the instrument gap, don't argue with the user. "Instruments lie" is the default assumption on this Wayland host, not the exception.

### Never stop for the user to restart and test — do it yourself

yggui app-control exists precisely so the agent can perform the whole build → deploy → restart → test → screenshot loop without the user touching anything. When a change requires the GUI to relaunch to take effect, use yggui (kill the GUI process, relaunch via `yggterm-headless server app launch`, screenshot, probe state). Do NOT wait for the user to manually restart — that defeats the agent-first design. If the existing yggui surface is missing a probe or affordance you need to test something, extend yggui rather than hand the task back. Only stop for the user when an action is truly destructive or genuinely ambiguous.

### When diagnosing an issue, use `/investigate` — never free-list "issues" from raw telemetry

When the user reports a bug, an anomaly, "something is off," or asks "what issues do you see," reach for `gstack /investigate` first (skill is installed at `~/.claude/skills/gstack/investigate`). Its discipline: investigate → analyze → hypothesize → implement, with the Iron Law *"no fixes without root cause."* Each named issue must have:

1. A specific observed symptom (what the user sees, NOT what a telemetry field says)
2. A hypothesis with evidence supporting it
3. A falsification attempt (probe that would disprove it — e.g. send a keystroke, take a screenshot, query a different field) before naming it as an issue

**Do not list "issues" by free-associating from suspicious-looking fields.** Telemetry fields have semantics that may not match their names. War story: a field named `input_enabled` did NOT mean "user can type" — it meant "this host currently holds input focus/stdin," so it read `False` on a perfectly usable session whenever the window wasn't focused. That misread drove a whole false "the session is broken" investigation (2026-06-03). The flag was since renamed to `host_stdin_enabled` (per-host) / `foreground_input_ready` (summary aggregate) precisely so it can't be misread — see docs/xterm-bugs.md#surface-recovery-false-positive-on-transient. If you haven't read the code that sets a field OR falsified your interpretation against a live probe, do NOT cite it as a user-visible issue.

**Cross-validate every claim against the screenshot.** If the user is actively using the session right now, by construction it can't be "unusable" — anything you claim is broken must be visible to a human looking at the screen.

**Prefer ONE high-confidence issue named correctly over five low-confidence guesses.** Padding the list to look thorough is its own kind of dishonesty — it makes the user wonder if you understand anything at all.

### When the user reports issues, fix them — don't pre-emptively pause to ask

The user's workflow: they report issues; the agent fixes ALL of them and reports back with **causes + fixes** (not diagnoses awaiting permission). Don't ask "should I keep going?", don't ask "do you want me to pause?", don't enumerate trade-offs without taking action. The default is: keep working through the entire reported list, drive each fix end-to-end via yggui, and only stop when (a) every issue is fixed and live-verified, or (b) you've hit a genuinely destructive or ambiguous decision that needs user input. "Wait should I do this" is not the right reflex — the right reflex is "I'm doing this, here's why, here's the result."

### Never claim "shipped" or "fixed" without live proof

A fix is not shipped until you have observed the fixed behavior on the live host through yggui: screenshot of the visible change, state-snapshot showing the corrected field, telemetry trace showing the new code path firing, or a probe that exercises the affordance. Compiled binaries on disk, passing unit tests, and a successful `scp` are necessary but not sufficient — a stale daemon, deferred hot-restart, cached webview, or version-mismatch gate can keep the running system on the OLD behavior. Before saying "this is fixed" or "shipped":

1. Check the running version of every component that touches the fix (daemon, GUI, remote binary as relevant). `yggterm-headless server status` for the daemon; `pgrep` for the GUI; `ssh <target> ~/.yggterm/bin/yggterm --version` for the remote.
2. Confirm that the running version is the one that contains your fix. If not, drive the restart loop yourself per the previous rule until it is.
3. Exercise the fix on the live host (yggui probe, screenshot, state snapshot) and quote the evidence in the user-facing report.
4. If you cannot exercise it (no repro path available), say so explicitly — "code is on disk, daemon still at version N which lacks the fix, will activate on next swap" — instead of "shipped." The user reads "shipped" as "I can use it now," and a false shipped claim is worse than a documented gap.

### Check all affected surfaces together

If a change affects how sessions appear, check both the CWD tree sidebar and the start page. If it affects remote sessions, check both local and remote paths. If it changes an icon, check both the sidebar row and the start page card. Fixing one surface while leaving another inconsistent is a spec violation.

### Consult DESIGN.md before styling

`DESIGN.md` is the source of truth for colors, typography, spacing, button shapes, and interaction vocabulary. Do not invent new styles. If a style decision is not in `DESIGN.md` and needs to be durable, add it there, not in a comment or chat history.

## ⛔ BEFORE YOU SPAWN A SESSION, OR HAND WORK TO ANOTHER AGENT

Read `.agents/skills/yggterm-agent-fleet/SKILL.md`. It is the contract for the
four powers an agent CLI gains by running inside yggterm — its own addressable
row, spawning a delegate **and proving it received the brief**, messaging any
other row, and reading its own context budget before it runs out.

⛔ **`terminal new --prompt` has silently dropped an entire brief** while
reporting a good launch, costing eight hours of campaign time. The skill's §3
carries the four-step recipe that replaces it, and the one check that cannot lie:
**grep the delegate's transcript for a token from your own brief.** A transcript
file existing proves only that a process started.

## Custom commands

- `/yggui` — take a live screenshot, query app state, or run a terminal probe on the desktop host. See `.agents/skills/yggui-app-control/SKILL.md`.
- `/yggui-changelog-demo` — capture a proof bundle with screenshot, trace, and changelog entry. See `.agents/skills/yggui-changelog-demo/SKILL.md`.

## gstack skills

[gstack](https://github.com/garrytan/gstack) is installed globally (`~/.claude/skills/gstack`) on pi, dev, and jojo. Use these slash commands for engineering tasks:

- `/review` — rigorous multi-angle code review (architecture, security, performance, tests)
- `/ship` — full pre-ship checklist (review + QA + release)
- `/qa` — open a headless browser and run QA against a URL
- `/investigate` — structured investigation of a bug or anomaly
- `/plan-eng-review` — lock architecture before implementation
- `/plan-ceo-review` — product-level rethink of a feature idea
- `/retro` — retrospective across recent commits
- `/office-hours` — describe what you're building, get structured guidance
- `/autoplan` — auto-generate a plan for the current task
