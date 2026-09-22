<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Spec — the dispersed ygg verbs (`~/.yggterm/bin/ygg/`)

**Status:** normative, 2026-09-22 (owner grievance relayed as dream
ACK-a0bd107330: fleet tooling lived only in the repo checkouts on dev/oc and the local GUI host,
and a practice seat had to hand-stage `ygg-auth.py` into `/tmp` mid-consult-wave
just to switch a codex account). The repo skill dir
(`.agents/skills/yggterm-agent-fleet/`) remains the SSOT; every fleet host
carries faithful replicas; one verb owns the copies.

## The law

1. **The repo is the SSOT; the replica tree is `~/.yggterm/bin/ygg/`.**
   A replica is never hand-edited — drift is fixed by re-running
   `ygg-disperse.py install`. Hand-editing a replica is writing junk.
2. **One verb ships the set; one manifest proves it.** `ygg-disperse.py`
   is the only component that writes `bin/ygg/` (the one-subtree-one-owner
   law of `spec-yggterm-fs.md`), and it records
   `~/.yggterm/config/ygg-verbs/manifest.json`: source commit, per-file
   sha256, executable list, tier lists. The shipment list itself is
   `ygg-verbs.json` in the SSOT — tiers are decided in the repo, not on
   hosts.
3. **The base tier is fleet law.** Every fleet host (`dev`, `oc`,
   `practice` — the list lives in `~/.yggterm/auth/.fleet-hosts`) carries
   the base tier WORKING: `ygg-auth.py`, `ygg-board.py`, `ygg-procfind.sh`,
   `repo-doctor.py`, `ygg-disperse.py`. These are the daemon-independent
   verbs an agent seat needs anywhere — auth rotation, boards, repo
   verification, and the dispersal itself.
4. **Owner-managed files are shipped but never claimed.** The ygg-memory
   runner (`ygg-memory`, `ygg-memory.py`) disperses ITSELF — `sync-fleet`
   scp's fresh copies to `~/.local/bin` and `~/.yggterm/bin` on every peer
   and unlinks symlinks there BY DESIGN — and `bootstrap.sh` installs
   `ygg-memory-sync`. Two owners on one path is a war the fs spec's
   one-owner law forbids, so the dispersal ships those files into
   `bin/ygg/` (a working copy for a host the memory system has not
   reached yet) but never links or adopts them at the root or in PATH.
5. **The GUI tier ships everywhere and matters only where the GUI is.**
   The row/bridge/orchestrator verbs (`ygg_row.py`, `ygg_bridge.py`,
   `ygg_appctl.py`, `ygg-ci.py`, booter, monitor, spawn, …) ride in the
   same shipment — identical bytes on every host — but only the yggterm
   GUI hosts (dev/oc and the local one) run them against a live daemon. One shape, no
   per-host subsets to drift; a row verb aimed at a host without a daemon
   fails with an honest connection refusal.
6. **PATH story.** The base tier is linked `~/.local/bin/<name>` →
   `~/.yggterm/bin/ygg/<name>` (atomic symlink swap). A pre-existing hand
   copy of the same verb in `~/.local/bin` is *adopted*: replaced by the
   link, because the SSOT wins. GUI-tier and owner-managed verbs get no
   PATH links — the owner's copy is authoritative wherever it exists.
7. **Update flow — dispersal is part of shipping a verb.** A merge that
   changes a fleet verb and does not re-disperse leaves the fleet stale,
   exactly like an unmerged deploy. The flow: land the change in the repo
   through the normal lane → run `ygg-disperse.py install` (local host)
   and `install --fleet` (or `install --hosts a,b,c`; fan-out) from the
   merged checkout → the manifest then names the new source commit.
   Fan-out ships the tar through the ssh pipe and self-pipes the verb to
   finalize remotely — the `ygg-auth` idiom; no `/tmp` staging, no repo
   needed on the target host.
8. **Provenance / doctor.** `ygg-disperse.py verify` exits 0 only when
   every replica matches its manifest sha AND every base-tier PATH link
   resolves into the replica tree; `status` reports the same evidence
   without enforcing. `verify --hosts` cross-checks the fleet over the ssh
   self-pipe. A red verify is a defect against this spec, filed in
   `docs/pending-bugs.md`.

## Acceptance

On ANY fleet host, fresh or old: `~/.yggterm/bin/ygg/` carries the verbs,
`ygg-auth.py status` works from PATH without hand-staging, and
`ygg-disperse.py verify` is green.

## Migration

Hand-staged copies predating this spec (the 2026-09-22 `ygg-memory*` at
`~/.yggterm/bin/` root on a GUI host, the `~/.local/bin/ygg-memory` wrapper+copy
pair, dev/oc equivalents) are handled by ownership, not by one grand
adoption: the ygg-memory runner files belong to ygg-memory itself (its
`sync-fleet` re-installs them on every peer, symlinks included in the
unlink), so the dispersal leaves them exactly where the owner put them;
the remaining base-tier hand copies at `bin/` root become relative links
into `bin/ygg/` — identical duplicates are retired, differing bytes are
preserved under `config/ygg-verbs/` (nothing is lost, the root stops
drifting) — and `~/.local/bin` hand copies become links, with differing
bytes preserved the same way. The install log records what was adopted.
`/tmp` staging is never needed again.
