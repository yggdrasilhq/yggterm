<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Portable steering and legacy repository retirement

Status: portable seed/generator increment; private migration and full fleet
retirement require their own acceptance receipts. The public `steer/` tree
contains generic guidelines only. `~/.ygg/steer/` owns the private deployment.
The root location resolver supplies the actual store during compatibility.

## Organization and boundaries

`steer/seeds/<digest>.json` contains immutable verified public seed generations.
`steer/overlays/` contains private owner policy and harness adaptations.
The immutable seed maps registered harness IDs to local target paths.
`steer/active.json` selects the seed revision; `steer/receipts/` records
plans and readback. A separate revision namespace holds private material;
public seed upgrades cannot overwrite it or silently acquire new authority.

The first generator implements a common private suffix in
`steer/overlays/policy.json` and known local target plans. The separately qualified
[private-source compiler](spec-ygg-steer-private-source.md) can render sealed
per-harness literal/guarded source graphs as read-only proposals. Legacy
import, shared publication authority, fleet push and repository deletion
remain later migration operations, not implicit effects of the local seed
or read-only compiler. No existing private repository is overwritten.

Fresh onboarding can initialize the organization and plan/stamp known local
harness roots using only the installed seed. It never clones the legacy
private repository, invents fleet hosts/model accounts, arms dreamers or
publishes local policy. Shared-memory discipline and independent completion
review are seed defaults; detailed component procedures are progressively
loaded through the yggterm skill and registry.

The generator owns only its explicit managed block. Foreign memory guards,
user text and unrelated managed regions remain byte-preserved. Duplicate,
unbalanced, nested or ambiguous guards HOLD. An unexpected target change
between plan and application HOLDs; writes are atomic and plans bind exact
input/output bytes and target identity. Unknown harness targets and target
collisions HOLD. Symlink targets need explicit enrollment, not blind writes.
No process is restarted merely because its instruction file changed.

Stamping coordinates the default native memory hub's real lock as well as
the steering writer lock. A custom steering-state root does not repoint that
native writer. `YGGTERM_MEMORY_ROOT` overrides HOLD in this first increment
until their memory-owner fencing adapter is qualified. Uncooperative external
editors remain outside the trusted-writer guarantee; this is not protection
against a same-user process deliberately bypassing the common locks.

## Private policy migration and deletion

The operator has directed retirement AND deletion of the legacy repository.
Perform this as a recorded transaction, without another generic permission
request:

1. Inventory every tracked/untracked file, local/remote branch, dirty or
   occupied worktree, skill link/registry source, template, target, timer,
   watchdog and reference on each enrolled host. Missing hosts are unknown.
2. Assign each item a new private or public owner. Public material is
   rewritten as generic mechanisms and scanned before egress. Preserve
   private content, exact revisions and unmerged changes in private deployed
   overlays/skills or an opaque provenance archive, never public fixtures.
3. Reproduce current harness output with the new generator, preserving
   memory-owned guards. Review any intentional policy changes separately.
   Transfer source ownership and skill registry pointers, then stamp through
   the new owner and verify every target's actual bytes on every host.
4. Repoint onboarding, root-steer self references, watchdogs, units, shell
   wrappers and documentation. Test new-infra bootstrap without a private
   repository, drift detection, failed writes, recovery and offline hosts.
5. Seal readback/recovery receipts, verify no process or unmerged work still
   depends on any old checkout, then delete obsolete checkouts/worktrees and
   retire/delete the remote repository through its owner adapter. Confirm
   absence of active old references and that the new system still works.

Deletion is not skipped: it is the final migration transition. An archive
must not remain an editable second root or be listed as current source.
A successful seed initialization alone does not satisfy these five steps.
The [shared-state contract](spec-ygg-sync.md) defines review and replication
of private policy; host-local root/target identities remain distinct.

## Falsifiers

Fresh sandbox onboarding; repeated seed initialization; modified private
policy; foreign memory region preserved byte-for-byte; malformed/duplicate
managed guards; unknown/colliding harness targets; stale plan; missing or
symlinked target; permission/full-disk/partial publication; concurrent
stamps; offline fleet target; orphan skill; dirty/unmerged/occupied legacy
worktree; watchdog still pointing at the old checkout; and deletion before
new-owner readback. Any of these can retain a migration debt and prevent an
obsolete-repository removal from being called complete.
