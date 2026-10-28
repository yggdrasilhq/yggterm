# yggterm — the distributed swarm operating layer

yggterm provides durable agent execution, shared memory, coordination,
observation and governed repair across conventional operating systems.
Linux and the yggdrasil standard variant are the primary worker platforms.

**`ygg` is the human and agent porcelain. The yggterm GUI is an optional
client of the same system.** Work identity, process lifetime, hierarchy and
health must remain inspectable when the desktop is absent or broken.

> **4.0.0 migration in progress.** The system/porcelain contract is being
> implemented. Local progressive CLI help is available on enrolled nodes;
> source adapters, the CLI-first release seed and platform qualification are
> tracked separately. This page does not claim a 4.0 release, repaired GUI,
> healthy execution plans or calibrated resource allocation.

## Start at the CLI

```sh
ygg
ygg dream
ygg help examples --search "unused quota"
ygg dream inspect --help
ygg completion bash
```

Noun groups teach the next word with task examples. Help and completion are
local and never invoke operational backends. An unavailable observation is
reported as unknown, not as zero running work or a healthy empty system.

The 4.0 command family includes packages/skills (`ygg ynpm`), shared memory,
CI, daemon control, supervised harness execution, work hierarchy, relays,
dream history, services, conditional schedules and layered system inspection.
See [the system contract](docs/yggterm-4-system.md) for target interfaces and
migration status, and [CLI inspection](docs/swarm-introspection.md) for the
implemented command contract. A documented target is not a shipped capability.

## Install the `ygg` seed

The **4.0 installation contract** is a verified user-space CLI generation,
with local help and explicit capability readiness. GUI installation and paid
autonomy are separate operations. The installer/artifact cutover is not yet
qualified; the existing scripts below still describe the 3.x compatibility
channel until that work lands. They must not be used as proof of a CLI-only
4.0 installation.

Unix install entry:

```sh
curl -fsSL https://raw.githubusercontent.com/yggdrasilhq/yggterm/main/scripts/install.sh | sh
```

Windows PowerShell install entry:

```powershell
irm https://raw.githubusercontent.com/yggdrasilhq/yggterm/main/scripts/install.ps1 | iex
```

The release gate requires fresh Unix and PowerShell installation, mandatory
artifact checksums, rollback, verified installed bytes and no GUI prerequisite
for CLI help. Platform/role limitations must be visible. A seed can be
installed while a node is not ready to admit agent work.

Existing 3.x installers, executable names and package-managed installations
retain compatibility during the transition. Running sessions and user-owned
configuration must survive migration. The
[3.x GUI-first guide](https://github.com/yggdrasilhq/yggterm/blob/109b6608b6a924599b7c19e2f67cafeab991ede4/README.md)
records the preceding install/client interface.

## One system, multiple clients

- **System:** host-resident daemons own execution and transport-independent
  identities; shared memory and coordination records preserve continuity.
- **CLI porcelain:** `ygg` exposes typed operations, progressive examples,
  exact evidence, native resource units and partial/missing coverage.
- **GUI porcelain:** rendering and interaction add graphical convenience over
  those contracts. A GUI failure cannot certify or erase a worker's state.
- **Research:** hypotheses and evidence precede harness changes; reviewed
  implementation and affected-path results determine adoption.

Workers belong to work groups. Orchestrators coordinate admitted scopes;
sub-orchestrators have explicit parent/scope/handback contracts. A relay links
a predecessor and fresh successor carrying the same work, rather than
pretending context or process identity stayed unchanged. These concepts are
independent of sidebar position, labels or whether a GUI is running.

## Truthful operation

A heartbeat proves a pulse. A queue result proves a queue transition.
Neither proves a task was accepted, a GUI rendered correctly, or a repair
worked. Inspection distinguishes observed execution, source age, required
proof, independent review and owner acceptance. Repeated composer runs retain
their evidence, failed hypotheses, dispositions and resource cost.

The system's objective is better verified outcomes and lower failure
recurrence within authority/budget constraints. More tokens or more busy
workers alone do not establish improvement. Unknown evidence and existing
failures remain visible throughout a release.

## Progressive documentation

- [yggterm skill](.agents/skills/yggterm/SKILL.md): the agent entry door; load
  only the component guidance needed for the task.
- [4.0 system architecture](docs/yggterm-4-system.md): authority, taxonomy,
  daemon/CLI/GUI boundary, seed and qualification gates.
- [CLI inspection](docs/swarm-introspection.md): grammar and implemented
  availability; [verb shipment](docs/spec-ygg-verb-dispersal.md): ownership
  and replica proof.
- [Onboarding](.agents/skills/yggterm-onboarding/SKILL.md),
  [shared memory](docs/spec-unified-fleet-memory.md), and
  [packages](docs/ynpm.md).
- [Daemon protocol](docs/protocol.md) and
  [harness integration](docs/cli-integration.md).
- [GUI design](DESIGN.md) and [GUI proof bundles](docs/demos/ARCHITECTURE.md).
- [Product thesis](PRODUCT_THESIS.md) and [license](LICENSE).
