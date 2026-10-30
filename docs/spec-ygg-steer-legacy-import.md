<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->
# Legacy steering import: plan, bind, stage (read-only)

Status: proposal-only importer increment over the accepted private-source
compiler (`ygg-steer-compiler.py`, see spec-ygg-steer-private-source.md). It
plans and stages; it never activates policy, stamps a target, syncs a host,
deletes a file, or claims the legacy checkout is fully retired. There is no
new top-level command: a parent transaction wires `ygg steer import-legacy`
after acceptance.

## What exists

- `ygg-steer-import.py` — the reusable importer API beside the compiler.
- `tools/yggfs/test_import.py` — hermetic tests on invented legacy trees
  (plus a structure-only smoke pass when the real legacy checkout exists;
  only hashes/structure are read, no content or host values leave it).
- No changes to the compiler, root steer, IR, disperser, or any verb table.

## API

- `plan_import(legacy_root)` — read-only classification. Parses `targets.toml`
  and AST-parses the legacy `steer` assembler; hashes every declared source
  through a single pre-validated handle; inventories the whole tree bounded
  and link-free; returns a proposal dict with metadata, digests, the
  unclassified blockers, observed placement shape, unknown fields and hold
  items. Writes nothing, executes nothing.
- `build_manifest(plan, harnesses=None)` — derives the accepted compiler
  manifest: per harness a canonical relative target, a literal core part, the
  guarded section parts in assembly order, an optional local part under the
  harness guard, and the foreign-owner region to preserve. Refuses harnesses
  carrying unknown fields or hold items, and refuses entirely on non-harness
  hold items (fail closed).
- `compile_preview(legacy_root, manifest, harness, live=b"")` — proposal-only
  projection through the accepted compiler; the compiler still enforces its
  own digests, bounds and foreign-region rules. Writes nothing.
- `candidate_identity(plan, manifest)` — the location-independent candidate
  ID: SHA-256 over the canonical descriptor (schema, adapter kind, steer
  digest, targets digest, whole manifest). Absolute old/new paths never enter.
- `stage_candidate(plan, manifest, *, home, env, root)` — optional staging of
  sealed private candidates (below).
- `cli` — plan/manifest/preview/stage porcelain for the future parent wiring.

## Adapter `yggsteer-steer-v1` (versioned and narrow)

Recognized: module `ROOT`, a `CFG` loader bound to `targets.toml`, literal
`DOOR_GUARD`/`HARNESS_GUARD` constants, a `guarded` wrapper whose format
string byte-matches the compiler's guard — plain format fields over the
wrapper's own `name`/`body` parameters (no `!conversion`, no format spec, no
cutset `rstrip`) — and an `extract_foreign` whose body binds `m =
re.search(<f-string interpolating the owner tag>, <text>, re flags)` and
returns `m.group(0) if m else None` (a recognized grammar that returns the
matched region, never discards or rewrites it), with a literal default owner
tag. The `assemble` body is exactly: `t = CFG[<harness variable>]` (operands
pinned, not mere presence), one core literal open `(ROOT / t['core'])
.read_text().rstrip()` with no cutset, 1–64 managed appends of
`guarded(<literal tag or guard constant>, (ROOT / '<relative path>')
.read_text())`, one optional `if t.get('local')` wrap under
`HARNESS_GUARD` specifically, `stamp = extract_foreign(...) if ... else
None`, a foreign-owner slot appending `stamp.rstrip() + '\n'` verbatim, and
a two-newline join over the parts list. Anything else — loops, computed
tags, wrong config or guard operands, `!r` views, cutset stripping, an
extractor that drops the region, replaced appends, extra statements,
missing pieces — HOLDs. No template code is ever executed; the legacy
script is only parsed. A parse that matches binds `steer_sha256` into the
candidate identity, so a changed assembler is a different proposal.

`[hosts]` in `targets.toml` is observed as private placement data only: its
keys are recorded, its values are not compiled, staged, or imported, and it
is not authority.

## Candidate layout and guarantees

Staged under `<initialized Ygg root>/steer/candidates/<id>/` via the shared
root resolver (initialized roots only; no genesis, no foreign-root bypass):

- `manifest.json` — the compiler manifest (canonical JSON).
- `descriptor.json` — the identity descriptor (recomputes the ID on replay).
- `provenance.json` — absolute legacy/root paths and observation time;
  deliberately outside the candidate identity.
- `sources/<relative path>` — only the declared policy Markdown parts.
- `complete.json` — written last, sealing the ID, manifest digest and every
  file digest.

Directories are 0700, files 0600, enforced after umask. Before anything is
staged or replayed, `steer` and `targets.toml` are re-read against the
digests bound by the plan: any drift between plan and stage HOLDs without
publishing (replan instead). Publication is a single Linux
`renameat2(RENAME_NOREPLACE)` from a private temp dir with fsync discipline
— a platform without a typed renameat2 route HOLDs rather than falling back
to replacing `rename` — so a pre-rename partial copy, pre-rename full-disk failure, or a
concurrently created directory at the destination (even an empty one)
publishes nothing into it: the occupied destination is preserved in place
and the stage HOLDs. Replay recomputes the candidate ID from the staged
`descriptor.json` bytes, binds the staged manifest to the descriptor's
manifest, and revalidates all sealed hashes; it is idempotent (no rewrite)
and reports the disk manifest digest. After a successful rename the
candidate is namespace-visible. A subsequent parent-directory fsync failure
leaves its durability unconfirmed. The result preserves `published` and
reports `status: VISIBLE_DURABILITY_UNKNOWN`, `durable: false`, a typed
`fsync_error` and CLI exit 75. This is a retained HOLD, not accepted policy
or a claim that no directory became visible. Every replay revalidates all
selected source and candidate bytes and retries that exact parent-directory
fsync. Persistent failure remains held; only a successful sync returns
`DURABLE_CANDIDATE` and exit 0. Pre-rename failures publish no candidate.
No imported candidate activates policy, stamps a harness or authorizes legacy
deletion. This distinction corrects the earlier campaign-derived requirement
that conflated namespace visibility with durability; it does not certify that
earlier requirement. An existing directory that is incomplete or disagrees with its seal
HOLDs and is never overwritten. Existing policy, `active.json`, target bytes
and foreign-memory guards stay unchanged; temporary staging and candidate
namespace metadata are the importer's own effects.

## Input controls

Every declared source is read through one handle opened with `O_NOFOLLOW`
per component under the resolved root: symlink leaves and components are
refused, non-regular files are refused, owners with no resolvable identity
are refused, and per-file/total bounds match the compiler (4 MiB per part,
bounded tree scan, bounded script and TOML). Identity (`dev`, `ino`, size,
mtimes) is captured before and after the read; any change HOLDs. A
group-shared legacy checkout may be read as unsealed proposed data — the
plan is marked `sealed: false` and never claims private ownership of inputs;
only the staged candidate is private. The tree scan never descends links and
never copies recursively: credentials, sessions, databases, locks and skill
data cannot enter a candidate by accident.

## Blockers and retirement

Every legacy entry that no declared part consumes is recorded as an
unclassified blocker and preserved in place; `delete_eligible` is always
empty. `.git` is recorded separately as the parent's archived history
transaction. The importer therefore makes no complete-retirement claim and
no deletion decision.

## Known strict differences versus the old assembler

The parent still proves actual parity against the real old outputs and live
targets; these are the deliberate differences to expect:

- Foreign-owner regions are preserved byte-exact, including CRLF; the old
  tool normalized line endings and forced a trailing newline.
- A live snapshot whose foreign block lacks a trailing newline is not given
  one.
- Foreign-region matching is stricter: the old lazy regex took the first
  match; ambiguous, doubled, reversed or nested regions now HOLD.
- `compile_preview` runs under the compiler's contract, which requires
  current-user-owned sources; group-shared reads belong to plan/stage, which
  treat them as unsealed proposed data.
- When the live target lacks the foreign block, the old tool warned; the
  plan records that slot note and `required=True` remains caller-available
  and HOLDs through the compiler.

## Obligations that remain open

Private activation/stamping, fleet cutover, orphan skill relocation,
watchdog replacement, the parent's git-history restore transaction, and
eventual legacy deletion are separate recorded obligations — none of them
are performed or claimed by this module.
