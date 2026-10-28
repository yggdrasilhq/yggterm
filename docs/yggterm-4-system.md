<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# yggterm 4.0 system contract

Status: **normative architectural target; implementation/qualification in
progress**. This replaces the GUI-primary product boundary. It does not
claim a published 4.0 artifact or healthy execution, GUI or quota planes.

## Boundary and sources of truth

| Domain | Authority | Clients/witnesses |
|---|---|---|
| OS processes/storage/network | Conventional host OS | Daemon/platform adapters |
| Execution lifetime and PTY bytes | Host daemon and preserved-owner runtime | CLI attachments, GUI surfaces, traces |
| Work/group/role/attempt identity | Typed coordination and execution records | CLI tree, GUI tree, registry projections |
| Harness-native identity | Verified harness adapter/store relation | Titles, summaries and outline labels |
| Admission, budgets, stops | Activated configuration and owned permission contracts | CLI/GUI explanations, observer findings |
| Semantic memory | Scoped shared hub with causal provenance | Native stores as ingress/projections |
| Accepted outcomes | Artifact/affected-path proof and independent review; owner criteria | Queue status, process exit, model reports |
| Improvement claims | Predeclared research/evaluation and retained observations | Experiments, composer notes, summaries |
| GUI rendering/interaction | GUI's own presentation contracts | Pixels, app-control, viewport/input proofs |

`ygg` is porcelain over these sources. The GUI is another porcelain. Neither
may synthesize a competing lifecycle, hierarchy, health or acceptance model.
Existing terminal invariants in [the architecture audit](architecture-audit-2026-05-16.md)
remain valid for their subsystem; their old product framing is historical.

## Progressive command surface

Target families are packages/registry (`ygg ynpm`, `ygg skills`), memory,
CI, gauges, onboarding/doctor, daemons, supervised agents, work hierarchy,
relays, dreams, services, schedules, system inspection and optional GUI.

One versioned command definition owns grammar, examples, capabilities,
completion and dispatch declarations. A minimal native entry is preferred
when it can reuse the supported compiled toolchain; component implementations
remain modular. Shared JSON metadata prevents a second handwritten command
table. Structured argv/platform APIs preserve arguments, signals and exit
status; arbitrary shell interpolation is not a command protocol.

Help/completion are local before runtime, transport, auth or collection.
Unsupported capabilities and unavailable observations are explicit. The
current inspection grammar/availability is in [swarm-introspection.md](swarm-introspection.md);
planned families here are not automatically implemented by being named.

Every exposed component has a canonical door or a documented compatibility
mapping. Old executable names remain during migration; new skills/docs use
`ygg`. Resolve the actual verified component/generation, not a shim that calls
the porcelain recursively. Owner-managed memory runners, package entry
activation and dispersed payloads retain exactly one owner per path.

## Taxonomy independent of layout

- **Work:** an objective/acceptance contract; it can span many attempts.
- **Group:** a scope of affairs with explicit membership and coordination.
- **Worker / operational row:** an addressable worker execution in a group,
  not a sidebar widget. Unclaimed membership is an explicit supported state.
- **Orchestrator:** an admitted coordination role over a defined scope.
  A **sub-orchestrator** has a parent, bounded authority, lease and handback.
- **Controller/watcher:** a named deterministic mechanism, not a synonym for
  every orchestrator or model role.
- **Attempt:** one execution exposure; retry ordinals alone are not identity.
- **Session:** harness-native context with a verified relation to execution.
- **Relay:** typed predecessor/successor context succession for the same
  work, with a sealed handoff and preserved evidence. A retry is not a relay.
- **Daemon incarnation:** a host runtime owner with boot/process identity.
- **GUI row/outline/title:** a projection or attribute of those records.

The hierarchy is inspectable from CLI on any authorized fleet seat. Missing
hosts, unknown associations, orphaned execution and retained predecessors
stay visible. Layout changes and labels grant no role or execution authority.

## Launch, reconnect and updates

A daemon-supervised launch records an idempotent intent before resolving an
admitted harness and owning its process/PTY. It exposes role/group, native
session mapping, title/model metadata, host/boot/PID-start identity and
evidence. Unknown launch outcomes are reconciled by request ID rather than
retried blindly. Prefix shims resolve the real harness and cannot recurse.

SSH is transport, not lifetime owner. Losing a client, desktop or connection
cannot kill protected work or fabricate a duplicate. Reattach verifies the
exact existing incarnation. Client/daemon replacement preserves old owners
and protocol compatibility while sessions remain live. A 4.0 version bump
does not authorize restarting, pruning or rewriting them.

Admission, spending, repair and owner-stop decisions remain in their owned
system contracts. Porcelain and observer findings do not widen those powers.

## Memory and skill entry

The [yggterm skill](../.agents/skills/yggterm/SKILL.md) is the bounded entry
door, progressively routing to registry, onboarding and needed component
guidance. It does not eagerly load every component manual.

Every harness must use `ygg memory` with its actual harness identity and an
explicit task namespace before recall/handover. Native memory is adapted
ingress/projection, not an alternate authority. Failed convergence is not
fresh context. Diagnostics must still expose the failure without claiming
recall is ready.

Scope is part of that contract: transient work status is not automatically
global semantic policy. Managed launch should supply verified namespace
mapping; unknown opaque native mappings are quarantined, not guessed.
Existing imports require a provenance-preserving migration. This target does
not waive the current strict gate or delete conflicting source versions.

Public seed instructions can be small. Private owner policy is preserved;
root-steer simplification goes through its SSOT/review/stamping workflow,
never a hand edit or an assumption that important constraints disappeared.

## Observation and improvement

Separate pulse, source freshness, useful progress, accepted outputs and owner
acceptance. Show native compute/plan units, shared-pool identity, uncertainty,
reason codes and sources. Do not count cached input twice or infer funding
from a missing usage reader. CLI and composer consume the same immutable
snapshots/events; stale reads cannot become repeated independent evidence.

The research loop hypothesizes, probes, compares, composes/rejects, authorizes
and independently verifies exact effects. Repeated composer runs must add
distinct evidence or justified interventions. There is no claimed analytic
gradient or general convergence theorem for discrete harness changes.
Unknown/failed/deferred work remains visible; agreement and token utilization
are not substitutes for accepted outcomes.

## Seed and release qualification

The 4.0 curl/PowerShell entry installs a verified `ygg` generation. GUI and
paid autonomy are opt-in. A native help-capable seed can exist before a node
is agent-ready; mandatory memory/execution prerequisites gate admission.
Interpreter-backed components require provisioned/checked runtime and honest
platform capabilities. Bare system-Python assumptions are not fresh-host proof.

Qualify local offline help, component resolution, checksum enforcement,
atomic activation, rollback, settings/live-work preservation, Unix/PowerShell
installs, and the declared platform/role matrix. CLI seed jobs must not depend
on a successful GUI package. Missing required CLI artifacts block a supported
platform claim; a successful Linux desktop build cannot qualify Windows.

Qualify daemon-owned launch/reattach, hierarchy/relay and failure inspection
without a GUI, then exercise interrupted transport/launch/update, stale and
missing sources, privacy, native accounting, denied authority and suspended
review. Record runtime as well as installed hashes. A release ACK, a green
build or a version edit is not a downstream-install proof.

Existing GUI/plan/pressure defects keep their own acceptance criteria. They
cannot be closed by this architecture or labelled green when required proof
or owner acceptance is missing.

## GitHub releases as the optional porcelain distribution source

The CLI seed is the first install. The owner-facing desktop install sentence
is `ygg ynpm https://github.com/yggdrasilhq/yggterm`. The URL resolves to an
explicit versioned release package manifest, not an arbitrary repository
install script. Every artifact declares its platform, architecture, checksum,
package identity and delivery capability. Desktop publication, rollback and
removal preserve the seed and daemon-owned sessions. Web and mobile clients
can share the schema while declaring different delivery mechanisms; support
is qualified per mechanism and target. Schema support is not artifact or
installer proof.
