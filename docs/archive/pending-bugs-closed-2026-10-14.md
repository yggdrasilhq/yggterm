# pending-bugs closed 2026-10-14

Closed narratives moved out of the queue in the same commit (docs-ssot law:
a fixed entry is deleted; git remembers it). Nothing here is lost.

## ⛔ [11.239] THE DISK-BALLOONING FAMILY — FOUR UNBOUNDED GROWTH PATHS IN THE FLEET'S OWN TOOLING (ynpm scratchpad, ynpm generations, ynpm/npm caches, ygg-memory memory-backups) FILLED THE MUSE-LAB HOST'S SSD TO 84% POOL AND /tmp TO 100% INODES (measured 2026-10-11→14, the muse lab host zcode sess_cf046eec; the /tmp arm cross-measured by sitting 36, infra/meta ACK-e26171ca6c)

**Status:** OPEN

OWNER PRIORITY (2026-10-14, in-session): "these are important fixes that the
yggterm agent should prioritize on multiple layers." The layers, named: (1) a
full SSD takes down the PRIMARY GUI host for every seat and campaign at once;
(2) the inode half ALREADY FIRED — sitting 36 found /tmp's inode table at 100%
and the host refusing writes; (3) every cleanup so far has been manual heroics
inside a sitting, which is exactly the class the verb-first doctrine exists to
kill; (4) the growth is SELF-INFLICTED — our own verbs (ynpm install, sync
harness) write the files, so the retention fix belongs IN the verbs.

Root mechanism (one shape, four surfaces): each tool treats its local state as
append-only and retention as someone else's job. Nothing sweeps, nothing caps,
nothing prunes at write time.

| id | member | measured on the muse lab host 2026-10-11→14 | fix direction | falsifier |
|---|---|---|---|---|
| 11.239-a | ynpm scratchpad — per-update extraction dirs (`npm-<pkg>-<ver>-<rand>/`) never swept, in BOTH locations | `~/.yggterm/scratchpad/ynpm` 22G (claude-code 2.1.267→2.1.272 one dir per update); `/tmp/.yggterm/scratchpad/ynpm` 138G, 455 dirs, same versions re-downloaded per install (opencode 2.0.26 ×5 at 803M each) — sitting 36 swept it after the inode event | sweep-on-install (keep newest N extractions per package, N≈2, never the dir a live process runs from — verify via /proc cmdline before deleting) + a `ynpm sweep [--dry-run]` verb with a default cap | after 30 days incl. ≥3 CLI update cycles, both scratchpad roots stay under ~3G and ≤2 extractions per package |
| 11.239-b | ynpm generations — every downloaded version kept forever | 21G: claude-code 25 versions (5.3G), opencode 9G, codex 2.2G; the ACTIVE generation is wired by absolute path (ychrome runs FROM `generations/ychrome/0.2.2/`) | `ynpm gc` = keep active-wired + newest N per package (N≈2), refuse to remove any version referenced by a running process or a registered path | a gc run followed by every fleet CLI launching clean (ychrome, claude, codex, opencode all exec) and generations du ≤ ~8G steady-state |
| 11.239-c | ynpm cache + npm-cache `_cacache` — download caches never pruned | 13G + 1.9G on the muse lab host (2026-10-14, swept by hand) | cap + LRU prune inside ynpm's cache path (npm-style `--cache min=…` or post-install prune) | cache dir stays under its cap across a month of installs; hit-rate unchanged (re-downloads only for pruned entries) |
| 11.239-d | ygg-memory sync-harness memory-backups — a FULL backup run per invocation, zero retention | `~/.yggterm/memory-backups/<ns>/<ts>/`: ~1,950-2,000 timestamped runs PER namespace × ~20 namespaces (~40k dirs, 14G; runs as small as 0.5MB) — the harness runs at every session end per fleet law, so this is the fastest-growing of the four | retention AT WRITE TIME: keep newest N runs per namespace (N≈5), prune older in the same pass; never prune the run being written | a namespace that syncs daily for a month holds ≤ N+1 run dirs and memory-backups du stays flat |

Manual sweep state 2026-10-14 (the muse lab host, this seat): scratchpad roots emptied
(nothing ran from them — /proc-verified), caches deleted, backups pruned to
newest-2, generations pruned to newest-2 + wired versions → `~/.yggterm`
83G→9.1G logical. Without members a-d landing, the next CLI-update cadence
regrows (a)+(c) and every session-end regrows (d).

Cross-refs: sitting 36's sweep post (infra/meta ACK-e26171ca6c) measured the
/tmp arm and filed the ynpm-sweeper dream; this entry is the PRIORITIZED
parent. F8 owns the memory plane's consolidation; (d) is its backup-side
sibling and deliberately homed here because the leak is a disk-plane one.


## CLOSURE RECORD (sitting 39, 2026-10-14, zcode sess_ab07a759) — ALL FOUR MEMBERS FIXED AND LIVE-PROVEN

Root mechanism (one shape, four surfaces, now fixed AT THE WRITERS):
retention was never a verb's concern. Lands:

- (a)+(b)+(c) ynpm — lane/f9/ynpm-retention cdf15344, in main 210d9bfc0e38
  (ci deployed 12:35:13 IST, tip merge-base verified). install retires its
  staging tree on success (it was a duplicate of the generation all along —
  the single largest scratchpad growth path); a stateless invocation reuses
  a verified published generation instead of re-downloading (the /tmp
  138G/455-dir arm); `ynpm sweep [--dry-run] [--force] [--keep N]
  [--cache-cap-bytes N] [--grace-ms MS] [--scope scratch,generations,cache]
  [--no-alt-tmp] [--json]` with fail-safe guards (live /proc exe+cmdline,
  link targets, recorded destinations, textual refs, grace window,
  current-pid) that defer, never delete; capacity admission in bytes AND
  inodes before an install allocates; throttled (6h) automatic pass;
  cache cap evicts oldest-modified content and reconciles via npm's own
  `cache verify` only when an index exists. Sol-high consult (6×
  accept-with-corrections) folded in; crate tests 53/53; falsifier rig 6/6
  (3 same-version cycles → 1 fetch + 0 staging residue; wiped state → 0
  re-fetch; dry run deletes nothing; live generation untouched; impossible
  floor → honest numeric refusal; keep-set honored).
- (d) ygg-memory — runner dfe71ead hand-pinned on dev/jojo/oc ×
  {~/.local/bin,~/.yggterm/bin} (read back identical; stable across mesh
  ticks — the tick REVERTED mid-pin until every host's source copy was new;
  that is the [11.236] read-back law biting again). backup_native_file:
  collision-free buckets (parent name + short hash of the FULL parent path
  — a/common/x.md and b/common/x.md used to silently evict each other),
  atomic publish (a torn copyfile can no longer sit under a digest name),
  retention at write (newest-5 distinct digests per bucket, never the copy
  just written). New verb `prune-backups [--dry-run] [--keep N] [--legacy]`
  — legacy trees retired ONLY on positive evidence (non-digest .md leaves
  and zero digest-named .md anywhere; file-named DIRECTORIES excluded from
  the leaf test; anything ambiguous reported and left).

LIVE PROOF (effect level):
- dev: sweep (exe-proof 81940aff) reclaimed 125.2 GB / 513,182 files —
  scratchpad 19G→25K, generations 23G→3.3G (keep-set held), cache
  14G→2.0G (cap); claude 2.1.299 / codex 0.162.1 / opencode v2.0.28 /
  ychrome 0.2.2 all exec clean after the sweep.
- jojo (muse-lab host): 8.0 GB / 37,355 files reclaimed — scratchpad
  983M→25K, generations 5.2G→2.8G; codex/claude/opencode exec clean.
- memory-backups on jojo: 24M→1.9M (23 legacy trees, 12 over-kept
  copies, zero ambiguous); digest copies FLAT (34→34) through a 59-door
  churn sync-harness.

REMAINder (not defects, watched): the installed manager is the exe-proofed
lane build until the release train ships a release-built binary; the
30-day falsifier windows are standing observations; jojo /tmp at 94% is
stale mac-kvm rig residue (routed infra/meta ACK-295378a94f, not F9).
