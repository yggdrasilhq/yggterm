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
