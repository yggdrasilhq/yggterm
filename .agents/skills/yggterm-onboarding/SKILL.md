---
name: yggterm-onboarding
description: What yggterm sets up on first use, where every piece lives, and how to mold or disable each one — including the dream orchestrator, swarm services, and your own yggsteer. Use when a user asks what got installed, wants to change memory-sync behavior, arm or stop dreaming, add a host, or uninstall anything.
---

# yggterm onboarding — the swarm operating harness, molded to you

yggterm is an **operating harness for agent swarms on distributed machines**
(yggdrasil or any OS); the yggterm GUI is the human-interface component, the
daemons/verbs are the automation plane, and yggdrasil is the minimal kernel
principles shared by the machines. First-run bootstrap scaffolds the bare
essentials; everything is moldable and nothing is hard-coded. The
deterministic installer is `ygg init` (the engine); this skill is the map.

## The one door: `ygg`

All agent-facing tooling dispatches through `ygg <verb>` (porcelain router →
`~/.yggterm/bin/ygg/ygg-<verb>`, spec: `docs/spec-ygg-verb-dispersal.md`).
`ygg help` lists what your host carries. Tier 0 sanity: `ygg doctor`.
Everything the fleet recons by hand — queue depth, plan quotas, live agent
sessions, load — collapses into:

```sh
ygg agents ps          # live agent sessions fleet-wide (row registry + /proc)
ygg fleet pressure     # load + zai plan quotas + queue + dream state, one view
ygg board scan         # unrealized dreams on the msgGraph boards
ygg dream status       # the dream orchestrator's view
```

## What first-run bootstrap created

| piece | where | what it does |
|---|---|---|
| memory hub | `~/.yggterm/memory/` | the semantic-memory store all harnesses converge on |
| sync verb | `~/.local/bin/ygg-memory` | native↔hub sync, `now`/`asks`/`doctor`/`dream` verbs |
| sync timer | `~/.config/systemd/user/ygg-memory-sync.{service,timer}` | 60s recovery tick; doctor timer probes hourly |
| ask-user ledger | `namespaces/_global/ask-user-ledger.md` | agent-blocked-on-you items, one gentle daily review |
| dream config dir | `~/.yggterm/config/dream/` | created but NOT armed — dreaming is opt-in |
| verb replicas | `~/.yggterm/bin/ygg/` | the dispersed fleet verbs; repo is SSOT, never hand-edit |

## The deterministic engine: `ygg init` / `ygg doctor`

```sh
ygg init            # tiers 0-3, report-only
ygg init --fix      # install what is missing (dream config, service units)
ygg init --arm      # tier 3 AND arm dreaming on THIS host
ygg doctor          # tiers 0-2 health probe (what the cron/checks run)
```

- **Tier 0** node sanity (python3, bun, flock, ssh, git).
- **Tier 1** verbs + memory hub answering.
- **Tier 2** observability (row registry, service/arming consistency).
- **Tier 3 (opt-in)** swarm services: dream orchestrator config + `yggterm-dream-spawner.{service,timer}`. Services install fleet-wide; **arming is a separate explicit act** (below) — autonomous token spend needs a decision, never a default.

## The dream system: spawner → queue → executors → supervisor

- **Supply**: `ygg-memory dream --prepare` stages briefs from journal deltas.
- **Spawner** (`ygg dream topup`, `yggterm-dream-spawner.timer` every 20min):
  stages briefs and enqueues them on the executor queue — bounded passes
  (≤2 dreams, ≤7min), single-flight flock, config-driven.
- **Executor queue**: the worker queue (e.g. zcode-tui's) owns mechanical
  honesty — peak law, cooldown ladders, stalled-requeue, serve lock.
- **Supervision** (`ygg dream supervise`, same timer): dreams that died at API
  drops or quota walls get fleet-level care — classify queue failures against
  the **quota calendars**, **reroute** to a plane with quota (`queue reroute`),
  **hold** until a reset window (`queue hold`), **revoke false completions**
  (`queue revoke`), and write the wake ledger. ⛔ Quota exhaustion is never an
  account reset: banked resets are consumed only by the owner's explicit act.

### Arming (the one decision)

```sh
mkdir -p ~/.yggterm/config/dream && touch ~/.yggterm/config/dream/enabled   # on exactly ONE host
```

Units may sit on every host; unarmed hosts no-op silently. Mold the plan via
`~/.yggterm/config/dream/floors.json` (namespaces, floors, routes, targets,
peak windows, quota calendars) — a tuned file is never overwritten.

## Launch observability (the daemon-prefix law)

Every agent session should be visible to the fleet. Where the yggterm daemon
runs, sessions launched inside yggterm register automatically; CLI harnesses
get transparent shims (`~/.local/bin/<cli>` records pid/cwd/args/ytrace-id,
then execs the real binary) — no prefix to remember. The root-steer law says
the same thing: **launch agents under the yggterm observability plane**; if
you must launch bare, say so in the seat's handoff so supervision knows.

## Your own yggsteer (root-steer system)

Onboarding gives every user the owner's steer pattern, sanitized:

```sh
ygg-steer-init ~/git/yggsteer   # scaffold: core/<harness>.md SSOT + sections/ + stamp tool
./steer stamp <harness>          # render the SSOT into each harness root file
./steer check                    # drift instrument — run after any SSOT edit
./steer push <harness> --yes     # ship to your hosts
```

Your steer carries YOUR laws (campaign routers, memory discipline, consult
doctrine) — generated from templates, never hand-editing stamped files.

## Skills registry

The onboarding skill (this file), the fleet skill, and every first-party skill
register in the ynpm skills registry (`ynpm skills scan && ynpm skills list`).
Record routing facts as NOTES on registry rows — the registry is the minimap
the next agent reads.

## Mold points (nothing is hard-coded)

- **Timers**: edit installed units; add `# YGGTERM_UNIT_TUNED` and bootstrap
  will never overwrite your version.
- **Dream plan**: `floors.json` (above). **Composer chain**:
  `YGG_DREAM_COMPOSERS=opencode:opencode/<model>,codex`.
- **Health thresholds**: `YGG_COMPOSER_FAILURE_LIMIT` (3),
  `YGG_COMPOSER_SKIP_WINDOW_S` (3600), `YGG_COMPOSER_ESCALATE_AFTER` (8).
- **Fleet mesh**: `~/.config/ygg-fleet/mesh` (absent = solo host).
- **Memory root**: `--root` on any verb.

## Daily rhythm

`ygg-memory now` (or your seat's session-start gate) shows what awaits you.
ONE review invitation per day across harnesses; up to three questions on
agreement; `ygg-memory asks close <id> --ruling "…"` records the decision.

## Disable / uninstall any piece

```sh
systemctl --user disable --now yggterm-dream-spawner.timer  # stop dream top-ups
rm ~/.yggterm/config/dream/enabled                          # disarm dreaming (units may stay)
systemctl --user disable --now ygg-memory-sync.timer        # stop the memory tick
rm ~/.yggterm/config/dream/floors.json                      # reseed the dream plan
```
