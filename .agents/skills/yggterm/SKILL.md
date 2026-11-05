---
name: yggterm
description: Use the ygg swarm operating plane to inspect work, initialize a host, or maintain shared memory, packages, steering and services. Load this entrypoint before the detailed fleet mechanisms.
---

# yggterm: one progressively loaded door

`ygg` is the primary porcelain for the distributed swarm operating layer.
The GUI is another client. Begin with `ygg`, a noun such as `ygg dream`, or
`ygg help examples --search "unused quota"`; local help needs no model or
fleet query. Consult installed help and the current status document before
assuming a planned family works.

## Shared memory is required

Every model harness uses `ygg memory` for shared semantic memory. A native
harness memory feature is an adapter, not a competing authority or permission
to bypass this gate. Before deep recall, continuation or handoff:

```text
ygg memory sync --harness <actual-harness> --ns=<task-area>
ygg memory status --harness <actual-harness> --ns=<task-area>
ygg memory diff --harness <actual-harness> --ns=<task-area>
ygg memory get --file NOW.md --ns=<task-area>
```

Use the actual harness identity and the area requested by the task. Add
`--fleet` when another host may hold relevant changes. A failed strict sync
means freshness is unverified; a later status check does not turn failure
into success. Read NOW.md when present, then the required source doors.
If the gate itself is the authorized repair, diagnose it without claiming
freshness or expanding into unrelated source work.

## Recover access before escalating

Consult the operator's configured credential chain before asking for login or
keys. When registered, infrastructure and credential-recovery skills own that
private route; see the [portable recovery contract](../../../steer/credential-recovery.example.md).
Check native status, use the authorized local vault/recovery path and verify
the requested read. Ask for human recovery only when both service access and
its authorized recovery are blocked. A missing protocol is an explicit routing
gap; do not guess credentials or claim that recovery ran. Quota, reserves and
work eligibility remain separate from authentication.

## Load only the required door

| Need | Entry |
|---|---|
| Find a skill or assumed context | `ygg ynpm skills list`, then `ygg ynpm skills info <name>` |
| New host, components, arming | `ygg init`, `ygg doctor`; [onboarding](../yggterm-onboarding/SKILL.md) |
| Memory and conflicts | `ygg memory`; [memory contract](../../../docs/spec-unified-fleet-memory.md) |
| Packages and skills | `ygg ynpm`; [package contract](../../../docs/ynpm.md) |
| Auth capabilities and accounts | `ygg auth harnesses`, `ygg auth status --harness all`; load the registered `ygg-auth` skill for account work |
| Private steering and locations | `ygg steer`, `ygg fs`; [steering](../../../docs/spec-ygg-steer.md), [filesystem](../../../docs/spec-ygg-fs.md) |
| Dreams, services, schedules, pressure | `ygg dream`, `ygg service`, `ygg schedule`, `ygg swarm`; [inspection](../../../docs/swarm-introspection.md) |
| Working context, observations and delegation references | `ygg ledger`; [working-ledger contract](../../../docs/working-ledgers.md) |
| Integration builds and publication | `ygg ci`; [build contract](../../../docs/spec-ygg-ci.md), [completion](../../../docs/repository-completion.md) |
| Rows, spawn, relay or GUI mechanisms | Relevant section of the [fleet manual](../yggterm-agent-fleet/SKILL.md) |
| System taxonomy and rollout boundaries | [System](../../../docs/ygg-system.md), [current status](../../../docs/operating-python-status.md) |

The logical information root is `~/.ygg`. Existing installations can reach
the same inodes through the legacy alias; that alone does not migrate apps,
prove fleet convergence or authorize garbage collection. Private policy and
credentials stay outside the public seed. Account rotation needs an eligible
local binding; it never authorizes a banked reset or replay of a failed
native request. Installing components does not consent to arm autonomous work.

Use a working ledger for an evolving objective, short notes, open obligations
and links to delegated jobs or evidence. Read `ygg ledger resume` at a handoff;
inspect operation lookup before retrying after a lost acknowledgement. Notes
are reported context: they do not dispatch workers, grant authority or certify
completion. Canonical task, review and memory stores retain those decisions.
This local notebook is distinct from fleet-wide job/role history. Consult the
installed command help and the contract's availability status before using it.

A live process, transport exit, delivered artifact and accepted work are
different observations. Inspect source age, coverage, exact attempts and the
configured independent acceptance chain. Current gaps include the complete
repair-execution/learning loop and the planned session/role-ledger sentences;
see their source contracts rather than inventing a successful fallback.
