# CLAUDE.md

Read `AGENTS.md` in full before starting any task. It is the authoritative engineering contract for this project.

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

### Never stop for the user to restart and test — do it yourself

yggui app-control exists precisely so the agent can perform the whole build → deploy → restart → test → screenshot loop without the user touching anything. When a change requires the GUI to relaunch to take effect, use yggui (kill the GUI process, relaunch via `yggterm-headless server app launch`, screenshot, probe state). Do NOT wait for the user to manually restart — that defeats the agent-first design. If the existing yggui surface is missing a probe or affordance you need to test something, extend yggui rather than hand the task back. Only stop for the user when an action is truly destructive or genuinely ambiguous.

### When diagnosing an issue, use `/investigate` — never free-list "issues" from raw telemetry

When the user reports a bug, an anomaly, "something is off," or asks "what issues do you see," reach for `gstack /investigate` first (skill is installed at `~/.claude/skills/gstack/investigate`). Its discipline: investigate → analyze → hypothesize → implement, with the Iron Law *"no fixes without root cause."* Each named issue must have:

1. A specific observed symptom (what the user sees, NOT what a telemetry field says)
2. A hypothesis with evidence supporting it
3. A falsification attempt (probe that would disprove it — e.g. send a keystroke, take a screenshot, query a different field) before naming it as an issue

**Do not list "issues" by free-associating from suspicious-looking fields.** Telemetry fields have semantics that may not match their names: `input_enabled: False` does not necessarily mean "user can't type," it might track which input mode owns focus or be a transient between mounts. If you haven't read the code that sets a field OR falsified your interpretation against a live probe, do NOT cite it as a user-visible issue.

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
