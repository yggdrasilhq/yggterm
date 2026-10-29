---
name: yggterm
description: The progressive entry door to the yggterm swarm operating layer: ygg CLI, shared memory, onboarding, packages/skills, daemons, work hierarchy, relays, inspection, learning, CI and optional GUI.
---

# yggterm — one door, progressively loaded

The system is the distributed swarm operating layer. `ygg` is the primary
porcelain; its GUI is another client. This is the **4.0 entry-skill contract**;
use installed help/capabilities to distinguish implemented operations from
the architecture being rolled out. Unknown is not healthy or empty.

## Start with the question

Run `ygg` or the relevant noun group for local next-word help and examples.
`ygg help examples --search "unused quota"` searches tasks without a model or
fleet probe. Do not preload the whole skill registry or every door below.

On an installed seed, `ygg docs` lists its bundled documents. For example,
`ygg docs yggterm-4-system.md` opens the versioned system contract without a
source checkout. Repository links below are source navigation; installed
readers use the corresponding bundled document when available. Documentation
describes contracts and limits, not the current health of a node.

Before recall or handover, synchronize the task's explicit area:

```text
ygg memory sync --harness <actual-harness> --ns=<area>
ygg memory status --harness <actual-harness> --ns=<area>
ygg memory diff --harness <actual-harness> --ns=<area>
ygg memory get --file NOW.md --ns=<area>
```

Use the actual harness identity, including the correct identity for an
embedded/desktop harness, not the name of its underlying runtime. Read the
area rollup when present, then only the required source doors. Missing,
conflicting or inaccessible context is not a passed freshness gate. Native
memory adapters do not create another authority. Diagnostics can expose a
blocked gate; they cannot declare recall fresh by bypassing it.

## Doors

| Need | First command / bounded guidance |
|---|---|
| Discover a skill or assumed context | `ygg ynpm skills list`, then `ygg ynpm skills info <name>`; load only the selected skill |
| New host / seed / node readiness | `ygg init`, `ygg doctor`; [onboarding](../yggterm-onboarding/SKILL.md) |
| Root steering / private policy | `ygg steer init`, `plan`, `check`, `stamp`; [steering contract](../../../docs/spec-ygg-steer.md) |
| Memory/provenance/conflicts | `ygg memory ...`; [memory contract](../../../docs/spec-unified-fleet-memory.md) |
| Package/generation ownership | `ygg ynpm ...`; [package contract](../../../docs/ynpm.md) |
| Auth vault and account rotation | `ygg auth status --harness all --json`, `ygg auth rotate --harness codex --fast --json`; inspect eligibility and policy before inferring native recovery |
| Daemon, hierarchy, supervised execution, relay | `ygg daemon`, `ygg agent`, `ygg work`, `ygg relay`; [system contract](../../../docs/yggterm-4-system.md) |
| Dreams, services, schedules, pressure, events, checks | `ygg dream`, `ygg service`, `ygg schedule`, `ygg swarm`; [inspection](../../../docs/swarm-introspection.md) |
| Integration build / landing | `ygg ci ...`; [CI contract](../../../docs/spec-ygg-ci.md) |
| Optional GUI/rendering | `ygg gui ...`; [design](../../../DESIGN.md), [harness integration](../../../docs/cli-integration.md) |
| Legacy detailed mechanisms | [fleet mechanism manual](../yggterm-agent-fleet/SKILL.md): open the relevant section, not the entire manual by default |

Some 4.0 families may be absent during migration. Do not guess a successful
result or silently fall back to untracked execution. Existing component
executables remain compatibility/plumbing paths until their qualified `ygg`
mapping is installed. Missing a mandatory capability prevents node readiness.

## Invariants carried through every door

Work/group/worker/attempt/session/daemon identities are distinct; titles and
GUI positions are projections. Relay is context succession, not ordinary
retry. Host daemons own supervised execution; transport/client loss must not
kill or duplicate it. Use existing admission, budget, privacy and owner-stop
contracts. Installing the seed is not consent to arm autonomous work.

Pulse, queue exit, artifact delivery, independent acceptance and owner
acceptance are separate evidence. Preserve source age/coverage and native
plan units. Repeated cached samples or model agreement cannot close a repair.
Use source-linked falsifiers and inspect the affected path before declaring
completion. The GUI and any execution plane retain their own health verdicts.
