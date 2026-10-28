# Product thesis

## One line

yggterm is a distributed swarm operating layer above conventional operating
systems, with `ygg` as its primary porcelain and an optional GUI client.

## The problem

Long-lived agent work crosses hosts, model harnesses, contexts, toolchains
and quota windows. Execution can survive while its meaning becomes opaque:
what is running, where, for which objective, under whose authority, with
which evidence and cost, and what failed or changed since the last attempt.

A desktop tree helps navigate that work but cannot be the system of record.
An operator or agent must be able to inspect and administer it without a GUI
or an AI explaining undocumented fragments. A healthy-looking observer must
not hide a broken execution path.

## The operating layer

Host daemons own process/PTY lifetime and expose stable execution identities.
Coordination records describe work groups, workers, orchestrators and relay
succession independently of graphical layout. Shared memory preserves scoped
facts, provenance, conflicts and handoffs across harnesses. Package and
configuration generations make changes attributable and recoverable.

The `ygg` command surface binds those mechanisms into progressive sentences,
with local help, task examples, bounded inspection and explicit unknowns.
Existing components remain modular behind it. A fresh seed is the CLI and
its verified component contract; a GUI is an optional additional client.

## Intelligence is an empirical outcome

The improvement loop inspects versioned evidence, proposes a falsifiable
change, probes it, compares with a frozen baseline, and composes or rejects
it. Authorized changes receive independent affected-path verification.

Success means better verified outcomes, less recurring failure and useful
recovery within resource and authority constraints. Token burn, activity,
model agreement or a fix commit alone is not success. Unknowns, negative
controls, failed attempts and confounders are retained.

The research program owns hypotheses and evaluation logic before engineering.
The implementation owns executable contracts, adapters and test evidence.
Neither a research proposal nor a green fixture suite is a shipped system.

## The GUI's role

The GUI offers terminal surfaces, graphical navigation, previews and useful
interaction over the same system. It may have rich presentation-specific
state. It cannot own work authority, execution lifetime, coordination
identity or system health. Its rendering, focus and interaction receive their
own acceptance evidence; failure there remains a GUI failure even when a
daemon is healthy.

## 4.0.0 cutover

The [4.0 contract](docs/yggterm-4-system.md) replaces the former GUI-primary
product framing. The cutover is complete only when CLI-first installation,
component ownership, progressive skills, daemon-independent coordination and
layered inspection are qualified on the declared platform/role matrix.
Existing work must survive that migration. The version number does not close
unrelated GUI, plan or allocation failures.
