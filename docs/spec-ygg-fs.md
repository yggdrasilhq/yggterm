<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Ygg filesystem locations, compatibility and retention

Status: version 1 transition contract. The first implementation provides root
identity, compatibility aliases, location diagnostics and non-destructive GC
plans. Physical relocation, native-reader convergence, app state conversion
and destructive collection remain separately gated work.

The machine-readable source is
[the location contract](../.agents/skills/yggterm-agent-fleet/ygg-fs-locations.json).
Its owner is [ygg-fs.py](../.agents/skills/yggterm-agent-fleet/ygg-fs.py).
The [role contract](swarmharness2-roles.md) separates swarm responsibilities
from optional graphical clients. A filesystem release never implicitly
changes a swarm configuration revision or grants a runtime role.

## Location identity and ownership

The canonical logical home is `~/.ygg`. Every root has a UUID, host UUID,
contract version and layout epoch. A host-local bootstrap record lives
outside the store at `$XDG_CONFIG_HOME/ygg/root-location.json`, or
`~/.config/ygg/root-location.json` when the variable is absent. It locates
an existing identity; it does not replace swarm policy or activation truth.

The intended layout is:

```text
~/.ygg/
  root.json
  migrations/<transaction-id>/
  config/<component-id>/
  kernel/{swarm,server,identities}/
  vault/
  library/{refs,skills,verbs}/
  reviews/<review-id>/
  attempts/<attempt-id>/
  ynpm/{state.json,generations,cache,bin}/
  libyggterm/<app-id>/{config,state,cache}/
  run/<component-id>/
  logs/<component-id>/
  trace/<component-id>/
  scratchpad/<owner-id>/<unique-id>/
  gc/{plans,receipts,quarantine}/
```

ynpm owns verified executable generations and app registration. The GUI,
ychrome and other libyggterm applications own their mutable settings and
application data. The GUI's data is not the daemon's durable execution state.
Native harness credentials and session stores remain with their native
owners; `vault/` is not an instruction to copy them. Campaign datasets and
source/research repositories keep their separate filesystem identities.

Stable location IDs resolve to the current adapter path and separately name
its eventual target. For example, `kernel.swarm` still resolves to `swarm/`
until the swarm owner migrates it; `review.store` remains `swarm/reviews/`.
`app.yggterm` names `libyggterm/yggterm/`, with app-owned `config/`, `state/`
and `cache/` beneath it. Merely exposing that path does not move existing app
settings or prove an installed app consumes it.

Signed or content-bound records are never rewritten to change paths.
Compatibility resolution must keep their historical subjects and evidence
readable. Root and object identity must outlive a physical path refactor.

## First phase: alias without moving state

On an existing POSIX host with only `~/.yggterm`, `ygg fs init` records its
identity and creates `~/.ygg -> .yggterm`. No directory, database, WAL/SHM,
socket, lock, native session or credential is copied, moved or recreated.
Existing consumers keep their names and live inodes. Initialization is
idempotent and serialized with an external bootstrap lock and a lock on the
physical store. The latter coordinates callers using different XDG profiles.
Identity publication is create-once: a raced winner is validated, never
overwritten. A different host identity HOLDs. Every default-root replay
repairs a missing approved compatibility alias before reporting success.

Fresh explicit initialization uses `.ygg` and creates `.yggterm -> .ygg`
so older consumers cannot accidentally create a competing store. Explicit
custom roots do not create home aliases. Discovery alone creates nothing.
Two independent roots, a broken/looping/escaping default alias, foreign or
unsupported identity, conflicting exact overrides and locator drift HOLD
instead of selecting by timestamps or creating replacement genesis state.

The filesystem tool honors `--root` and `YGG_ROOT` as exact state roots,
and retains the legacy exact-root meaning of `YGGTERM_HOME`. Supplied
conflicting overrides HOLD unless they identify the same directory. Explicit
sandbox roots do not replace the default bootstrap locator. `YNPM_HOME`
keeps its existing OS-home-input meaning; it is not an exact Ygg root.

Metadata publication uses directory handles with no-follow traversal and
regular-file checks; symlinked GC output parents or leaves HOLD. New
directory entries, identity files, aliases and the locator are synchronized
in dependency order. This provides explicit durability operations, not a
claim of qualified power-loss recovery on every filesystem. The trusted
same-user store assumption still applies; a malicious owner of the directory
tree can mutate its names and is outside this compatibility guarantee.

This phase does **not** claim every existing native/Bash/Python reader uses
the new location contract or `YGG_ROOT`. Existing exact-root overrides remain
owned by their consumers. Reader migration requires shared conformance
vectors and versioned handshakes before new writers switch locations.

Windows alias mutation is not yet qualified by this tool and returns HOLD.
Use native platform diagnostics without inferring successful migration from
POSIX tests. Junction privileges, reparse traversal, ACLs, case collisions,
volume boundaries and open-handle refusal need native qualification.

## Physical and component cutover

The state machine is DISCOVERED → PLANNED → ALIASED → COMPATIBLE →
COMPONENT_READY → QUIESCED → COMMITTED → VERIFIED. Its later transitions
require owner adapters, an inventory of all writers, launch and pin fences,
a durable intent/outcome journal, storage-topology checks and rollback limits.
The first implementation stops at compatible aliases, not physical cutover.

Neither a disconnected client nor a stopped daemon proves all writers have
closed. Preserve SQLite state through its owner's consistent checkpoint;
never transfer a live lock inode between filesystems. Offline, unreadable or
unknown owners retain their old binding. Never merge populated roots.
After new writes, a stale backup cannot become authoritative by rollback.
Recovery must re-identify the recorded root and preserve owner-stop latches.

Roll out compatibility individually on the canonical server, other worker
hosts and the human workstation. Each host gets its own root and host IDs.
No credentials, native session stores, active databases, locks or sockets
move between hosts as part of this rollout.

## Garbage collection is a protocol, not recursive deletion

An owner enrolls an exact disposable object, its references and pins, a
restore method and a retention policy. A shared collector coordinates those
owners; it cannot infer disposal authority from age or a directory name.

Before quarantine **and** deletion, a transaction must bind the exact plan
bytes, host/root IDs, layout epoch, policy version, owner reference revision,
launch/pin fence epoch, object and file IDs, sealed content, action, expiry
and restore mapping. Revalidate under that same fence. An uncooperative
legacy launcher requires drainage; an incomplete process scan is not enough.

Retain all active cwd, executables, maps and open files; current/pinned and
fallback generations; review subjects and evidence; unfinished attempts;
config/recovery/deny state; DB/WAL/SHM groups; live lock inodes; native auth;
and unowned or custom data. Unknown visibility or ownership means retain.
PID alone cannot prove identity; process start and boot identity matter.

Initial policy:

| Class | Eligibility, after references and fences are proven |
| --- | --- |
| Package generations | Keep current, all pins, offline fallback and two verified predecessors; additional unreachable generations after 30 days |
| Ordinary logs | Producer rotation at 10 MiB; sealed unpinned segments up to 30 days / 100 MiB per component |
| Ordinary traces | Producer rotation at 10 MiB; sealed unpinned segments up to 7 days / 100 MiB per component |
| Reproducible download cache | Unreferenced verified blobs after 30 days |
| Owner-declared disposable scratch | Unleased objects after 7 days; reports and handoffs are not disposable by location |
| Reviews, config, auth, custom files | No automatic expiry |
| Migration recovery copies | At least 30 days after verified cutover, plus explicit retirement of rollback debt |
| Quarantine | 14-day restore window, cap 2 GiB; stop admitting when full rather than shorten the window |

Monthly planning reports eligible, protected and unknown usage. The current
`ygg fs gc plan` records metadata only, retains every object and reports zero
reclaimed bytes. `gc apply` explicitly refuses: owner/fence adapters are not
yet qualified. This is intentional coverage honesty, not completed rootwide
collection. Existing component collectors keep their own scope and require
a separate safety audit before delegation to a common coordinator.

A later apply is bounded to 1,000 objects, 1 GiB and 30 seconds per pass,
then checkpoints. Quarantine uses a same-filesystem rename and recovery
journal; deletion after grace needs fresh reachability and a new receipt.
Restore into an occupied destination HOLDs. Keep receipts for at least a
year and longer while referenced. Disk pressure can trigger planning or
reduce new disposable output; it never changes deletion authority.
Logical selected bytes, unlinked bytes and reclaimed allocation are distinct.
Snapshots, links and backups prevent any promise of secure erasure.

## Required falsifiers

The hermetic tests cover legacy/fresh roots, conflicting roots/overrides,
looping/escaping aliases, imported identities, locator drift, interrupted
initialization, preserved live flock and SQLite WAL access, explicit-root
isolation, metadata-only plans and refused apply. Fleet checks must separately
verify root/host IDs, old/new same-file identities and normal reader behavior.
Further gates include physical-move crashes, both-root conflict recovery,
platform-native paths, cross-filesystem transactions, full process visibility,
active/review/pinned evidence protection and concurrent launch/GC fencing.
Passing the first-phase tests does not close those later gates.
