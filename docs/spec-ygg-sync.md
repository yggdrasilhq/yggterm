<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Shared fleet information and logical application profiles

Status: version 1 architecture; owner adapters and multi-host transport are
not yet qualified. This contract does not authorize a recursive home mirror.
The logical root is `~/.ygg`; each installation retains distinct root/host
identity. Shared fleet and object IDs are separate from those local IDs.

## One logical information space

Enrolled hosts expose the same committed steering, settings, lessons and
application profile names. A profile has one stable ID independent of where
its current executor runs. Host placement is operational metadata, not an
extra profile namespace the human must manage.

| Information | Replication/activation contract |
| --- | --- |
| Generic seeds and executable packages | Verified immutable generations, activated separately by each owner |
| Private steering/configuration | Reviewed versioned revisions, causal parents, explicit writer authority and conflict retention |
| Memory and experience | Existing memory-owner journal/object protocol; preserve causal conflicts and provenance |
| Review/attempt evidence | Sealed immutable artifacts and append-only records; historical subject bytes remain unchanged |
| App settings/profile catalog | App-schema revisions by stable app/profile ID; independently versioned local placement |
| Active mutable app profile | One fenced live owner, clients routed to it; qualified handoff/snapshot adapter for relocation |
| Useful app cache | Owner-declared immutable objects keyed by content, schema/runtime compatibility and inputs; shared when meaningful |
| Runtime lock/socket/PID/boot identity | Host-local, never replicated as live authority |
| Native credentials/OS keychain | Native auth owner's broker/export contract; no generic file copying |
| Disposable scratch/platform cache | Explicit owner classification and retention; no replication by default |
| Unknown/custom data | Retain, report unclassified; neither mirror nor erase by guessed directory name |

A config reference alone does not arm a service, grant an editorial role,
change account bindings, or activate an unaccepted swarm revision. Existing
shared datasets retain their own authority; do not create a competing copy
inside the Ygg root merely to make the tree look uniform.

## Causal objects and trustworthy materialization

A manifest binds fleet ID, object/app/profile ID, owner contract/schema,
payload digest, causal parents, writer identity/epoch, classification,
references and retention. Missing parents, bad hashes, unknown writers,
unsupported schemas and incomplete transfers HOLD. Concurrent unordered
revisions remain explicit conflict heads; wall clocks never select a winner.
A reviewed resolution preserves all parents. Sync status distinguishes
committed, fetched, verified, materialized and owner-activated revisions.
Unknown/offline coverage is visible. No healthy-fleet claim follows merely
from a successful local push.

Transfer stages immutable blobs, verifies them and records a durable receipt
before owner activation. Publication is compare-and-swap against the expected
head. An ordinary host is not entitled to publish shared root policy because
it can store a blob. The trusted transport and grant mechanism remain owner
adapters, not secret-bearing fields in public seed files.

## libyggterm application profiles and invocation locality

The concrete pilot is ychrome, a libyggterm app, not Chromium. Its daemon,
headless engine and visible/standalone surface paths must be investigated
through their actual owner contracts. Its existing state includes named
`web-profiles/` session jars, app settings and a separate vault owner. These
are different from a Chromium user-data directory. No Chromium-specific
file layout or compatibility assumption is adopted here.

The inspected source separates `src/main.rs::profile_dir`,
`src/engine/identity.rs`, `src/daemon.rs`, `src/webpolicy.rs` and its vault
crates. The shared profile contract in yggterm core normalizes names and owns
metadata; the surface context key additionally includes egress and control
endpoint. These are distinct authorities, not synonyms for a directory name.

In the inspected thin-client path, actual web-context storage can be on the
GUI host while egress and app policy belong to the invoking host; standalone
and headless-engine paths use their own app-host jars. The new adapter must
record these mode-specific physical owners rather than infer them from the
invocation or a stale host-resident-only description. Existing cookie import
does not qualify complete cross-host website-state synchronization.

During import, two existing jars with the same display name may hold different
accounts. Preserve both under stable global IDs and explicit collision
labels until a reviewed alias/merge decision; both become discoverable on
all enrolled hosts. Never choose a jar by timestamp, overwrite its identity,
or mistake its source-host provenance for a permanent machine isolation
rule. Profile enumeration consumes the shared catalog after qualification.

| Existing owner/state | Migration requirement |
| --- | --- |
| `web-profiles/<name>/profile.json` and jar | Stable global profile ID, preserved unknown metadata, consistent app-owned state revision |
| WebKit data/cache bases in the profile jar | Explicit classification of cookies, local storage, IndexedDB, HSTS, service-worker state and reusable cache |
| `ychrome/` | Split durable app settings from daemon locks, sockets, registries and operational journals; never mirror the mixed directory |
| `web-zoom.json`, adblock and userscript state | App-owned schema revisions, provenance and independently activated asset generations |
| Vault peer agent | Retain native encrypted-server sync, revision guards and local unlock/consent lifecycle |

The current engine has cookie-import support because its network process does
not reliably reload external jar changes. A successful file transfer or
cookie import does not establish synchronized local storage, IndexedDB,
service workers, settings or live views. Readback must test each declared
coverage class through the actual running app and surface.

The required user behavior is one shared profile catalog and logical identity
on every host, while URL resolution and network egress continue to follow the
invoking host. Selecting a shared profile must not silently reinterpret
`localhost` or change the user's network identity. Profile state ownership,
engine placement, GUI rendering and network egress are separately recorded.

The application adapter must classify mutable stores, settings, cookies,
permissions, useful cache and process-local state. App-owned schema updates
and immutable exports can replicate through the common object layer. Any
exclusive mutable snapshot or ownership handoff needs a monotonic fence and
consistent export/import. Live concurrent multi-host use requires a qualified
app state service/merge contract; a file mirror cannot stand in for it.
Unavailable or partitioned owners report their exact coverage and hold the
operation requiring strong state, rather than silently creating a new
machine-specific profile.

Vault credentials, unlocked memory and human-presence grants remain with the
vault's owner. Shared profile identity does not imply copying decrypted keys
or granting passkey/camera consent. The concrete design and falsifiers must
bind the actual ychrome APIs, not a similarly named browser's behavior.
No live owner session is used as a disposable migration fixture.

## Cache, deletion and offline replicas

Share information-bearing application cache through explicit owner manifests.
Do not assume all cache is junk, nor all cache bytes portable: platform GPU
and lock state remain local, while expensive reusable results can be shared.
Apply content deduplication, quotas and owner reference tracking. Unknown
cache classification retains data and reports coverage debt.

Deletion is an authenticated tombstone with causal revision and retention
roots, not an absent file. Replication and GC share pin/launch fences. An
unacknowledged offline replica prevents tombstone-history collection unless
an explicit membership retirement epoch fences its later rejoin. A retired
replica must re-enroll before publishing; it cannot resurrect deleted state.
Do not promise secure erasure from unlinking a file while snapshots exist.
The [filesystem contract](spec-ygg-fs.md) governs plan/quarantine/delete;
rootwide destructive collection remains unavailable until these edges qualify.

## Acceptance experiments

Test same-profile discovery on two hosts; remote client access to one live
owner; two concurrent launch attempts; network partition before/after fence;
stale writer on rejoin; corrupt/missing/partial snapshot; incompatible browser
schema; lost auth/keychain; concurrent settings revisions; deletion followed
by offline replay; cache input/runtime collision; GC during transfer/launch;
full disk; and interrupted activation. Bind actual runtime version, profile
ID, revision, epoch and affected behavior. Generic manifest unit tests cannot
close browser, auth, platform or distributed transport qualification.
