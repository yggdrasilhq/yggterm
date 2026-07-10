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

Full mission statement: `[[project-purpose]]` in `~/.claude/projects/-home-pi-gh-yggterm/memory/project-purpose.md`.

## Pending bugs

Open, user-confirmed bugs live in `docs/pending-bugs.md`. When the user says
"finish the pending bugs" (or similar), that file is the work queue. Remove an
entry in the same commit as its verified fix.

## Core working rules

### Single source of truth — no exceptions

Every concept has exactly one owner. Before adding code, name the source of truth for the thing you are changing. If two places could answer the same question, collapse them. Never add a second encoding, copy, derived field, or fallback layer that can silently diverge. This applies to session identity, sidebar rows, start page rows, icon kinds, CWD matching, launch commands, scan results, and every other domain concept.

### Specs are applied holistically

When a spec changes (e.g. start page shows all sessions, or CC sessions appear in CWD tree), apply it completely across every code path that touches that concept. Sidebar, start page, remote machines, local files, and any future surfaces must all reflect the same rule. Do not patch one callsite and leave another inconsistent.

### No non-determinism

Do not introduce behavior that differs based on timing, environment, or ordering that the code does not control. Scan results must be deterministic. Row injection order must be stable. Modified-epoch fallbacks must be explicit. If a function can produce different output for the same input, that is a bug.

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
