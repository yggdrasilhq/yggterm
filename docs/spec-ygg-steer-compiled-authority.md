# Portable compiled steering authority

Status: proposed transition contract for architecture review, before code.
Existing generic-seed, private-compiler and lossless-importer mechanisms are
separately accepted. This proposal does not activate private policy, stamp a
host or authorize deletion by itself.

## Information and local effects

Public mechanism and sanitized defaults live in the yggterm repository.
Operator-private policy sources live below the logical `~/.ygg/steer` root.
A revision is an immutable content-addressed manifest plus the exact declared
Markdown parts, sealed through the accepted importer/packager. Source paths
are relative, bound to digests and read through owned no-follow handles.
Credentials, native sessions, databases, lock files and arbitrary skill data
are outside that package. The public seed contains no operator identifiers.

The same logical policy revision can be desired throughout a registered fleet.
Desired revision identity is information; actual activation, local root ID,
target bindings, saved target plans, locks and receipts are host-local effects.
Syncing desired state never copies one host's physical paths or activates a
projection on another host. Each host derives and validates its own exact plan.
Disagreement and unavailable hosts remain visible rather than called converged.

Desired policy has an append-only intent lineage independent of source-content
digests and host roots. Each update binds its own identity, the expected previous
intent identity, an immutable source revision and existing request provenance.
The admitted head changes only by compare-and-swap against that predecessor.
Replaying the identical update is idempotent. A delayed ancestor cannot replace
its descendant; incompatible children remain an explicit unresolved fork until
an authorized reconciliation names the competing heads. Arrival time never
chooses policy. An intentional rollback is a new authorized intent pointing to
older source bytes, not replay of the old intent. None of these records alone
activates or stamps a host.

Each host reports last-known desired intent and observation freshness separately
from its durable local activation and actual target/session readback. An offline
host may retain already-authorized local behavior. Its stale or unavailable
observation cannot certify fleet convergence or discharge retirement obligations.

## Reader-before-writer transition

Keep the existing schema-1 generic seed activation readable and unchanged by
ordinary initialization. Introduce an explicit schema-2 compiled mode only
once all required consumers understand both modes. Unknown kinds, fields,
source revisions, target adapters or roots HOLD; they never fall back to a
public seed or an unrelated source checkout. Re-running init preserves a
selected private authority. No generic install chooses one automatically.

A compiled activation binds the local root identity, immutable source revision,
previous activation digest and an operator-request provenance reference. It is
separate from stamping any harness. The caller still needs existing authority
for the requested change; a plan, an agent-authored reference or a source-review
verdict does not manufacture owner permission or endorse the policy's content.
This campaign's explicit migration request supplies its scope, not a universal
permission for later workers.

Activation is a separate exact planned transaction. Its saved plan binds the
local root, complete proposed activation bytes and their digest, immutable source
revision, expected current activation digest and existing request provenance.
Activation writers, activation readers and target stampers use one local
publication fence. Under that fence the writer rederives the plan, checks the
expected predecessor and all source identities, then publishes atomically and
synchronizes the parent directory. Two writers with the same predecessor cannot
both advance authority; a concurrent activation invalidates a saved stamp plan.
No activation may be inferred merely from a visible pointer.

The transaction outcomes and reader obligations are explicit:

- Failure before publication retains the old durable selection and reports that
  no new selection was published. It does not erase unrelated staged material.
- Replacement followed by a failed or unobserved directory synchronization is
  visible but unconfirmed. It reports HOLD and the intended digest. Every
  resolver first reconciles under the same fence: validate the complete visible
  activation, immutable source and root, then retry directory synchronization.
  Persistent failure or invalid/unknown bytes HOLD; neither the predecessor nor
  the generic seed is selected as a recovery fallback.
- Only successful reconciliation or publication with directory synchronization
  makes the selection eligible for plan/check/stamp. A receipt records the exact
  durable selection and predecessor. A crash after synchronization but before
  receipt is reconciled from the same complete bytes and produces a recovery
  receipt rather than repeating the transition.
- Replaying an identical durable transaction verifies its intended result and
  returns that result without rewriting it. Different intended bytes, a stale
  predecessor or an ambiguous visible selection HOLD for reconciliation.

The resolver synchronizes even when a prior receipt exists; a receipt does not
bypass validation or resolve a crash boundary by assertion. Network transfer and
fleet coordination occur outside the publication and native-memory fences.

`ygg steer plan/check/stamp` consume the selected mode through one resolver.
Compiled mode uses the accepted private compiler and its per-harness manifest;
seed mode retains its existing behavior. The saved plan binds the mode/source
revision, current activation digest, local root ID, target identity and bytes,
foreign-owner region, intended output and relevant policy/module digests.
Stamp rederives that plan under the publication and native-memory locks before
an atomic target write. Concurrent source, activation, target, memory or
identity drift HOLDs rather than silently changing the proposal. Default native
memory synchronization keeps its one existing lock inode; custom memory roots
remain held until separately qualified. Two harnesses cannot own one target
inode, and an unknown extra file or widened candidate metadata is not sealed.

Activation plans and receipts report local effects, not fleet completion.
Namespace-visible publication with unconfirmed directory durability is retained
and held; replay validates complete bytes and retries synchronization. A failed
write is never reported as an invisible no-op if a visible object remains.
No network operation runs while local publication or memory locks are held.

## Migration and retirement

First stage the actual legacy policy read-only, preserve every private input
and Git/worktree artifact, and reproduce every declared harness projection.
Then create a new private revision whose authority/pointer text identifies the
new operating mechanism; policy-content changes are explicit reviewed deltas.
A target with unexpected drift is retained for reconciliation, not overwritten
under a parity claim. Foreign native-memory regions remain byte-identical.

Move orphan skill ownership to a declared private state location or the actual
product repository; retain skill content and registry notes. Registry homes,
installed links, watchdogs, units, scripts and root pointers must resolve to the
new authority before the old one retires. Archived code/history is clearly
historical and never a live compiler fallback. Public onboarding uses sanitized
seeds and progressively loads the root yggterm skill, including mandatory shared
memory; it does not seed private operator facts on unrelated infrastructure.

A consumer ledger inventories installed readers/writers and relevant running or
resumable harness sessions. Each entry identifies the consumer, its loaded
authority revision/epoch and the evidence and freshness for that observation;
missing evidence is unknown, not current. File hashes, cwd/fd/map checks and a
successful stamp do not prove a session has reloaded its instructions. Before
retirement, every relevant live session must demonstrate reload/rebinding to the
new revision or be safely drained and restarted. Owner-pinned or active work is
retained until that obligation can be met without discarding its state.

Native session history stays unchanged and outside policy packages. Resume and
entry have an authority-epoch fence before an old context can act on its loaded
steering: verify rehydration to the current durable revision, or HOLD execution.
Unsupported harnesses and unknown/stale consumers remain explicit retirement
obligations. The fence must prevent an old-context instruction from recreating
or writing the retired repository, including after an offline session resumes.

Deletion of the legacy repository is the final requested action, after exact
revision/projection and restore receipts, fleet readback, classified remaining
files, process/cwd/fd/map/lease/pin checks and updated references. Dirty or active
worktrees remain obligations to migrate; they are not erased as cleanup. The
old path is removed rather than left as a competing authority or misleading
compatibility repository. Remote-repository retirement is a separately recorded
effect, with all required history retained. Any unresolved material blocks that
effect; it does not make the migration complete.

## Falsifiers

Preserve existing seed/FS/compiler/importer controls. Add seed-to-compiled and
idempotent init, invalid/future activation, wrong local root, corrupt or widened
candidate data, source/activation/target/native-memory changes after plan, two
targets sharing an inode, fault before and after every durable publication
boundary, persistent directory-sync failure, real memory-lock contention,
unavailable fleet members, stale desired revision, fresh infrastructure with no
private policy, and restored historical archives that cannot become active by
accident. Every actual target/host needs readback; a transport ACK or matching
manifest alone cannot certify the root instructions loaded by a harness.

Transaction controls must kill/restart a publisher after pointer replacement
with directory synchronization failing, then invoke a second-process resolver
and stamper. They reconcile or HOLD without projecting an unconfirmed selection
or silently falling back. Race two activators against one predecessor and race
activation against a saved stamp plan. At most one activation advances and no
stamp uses a changed selection. Cover crash after durable publication before a
receipt and idempotent recovery without a second transition.

Deliver desired intents B then C, replay B and reconnect an offline member
holding B: C remains desired or a fork is explicit, and that member's actual
state and freshness remain separate. A new rollback intent targeting B's bytes
is distinguishable from the replay. Keep a synthetic session loaded with old
root A alive while root B passes target hashes and process-reference checks:
retirement still HOLDs until verified reload or safe drain. Resume A after B is
selected and prove the entry fence prevents old-path writes before rehydration.
A fully qualified retirement removes the actual old path, retains restorable
history outside live discovery and needs no repeated owner confirmation.
