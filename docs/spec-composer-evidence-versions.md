# Composer logical observations and immutable evidence versions

Status: source contract for CSR09 with the canonical typed encodings an
implementation must follow; no empirical benefit is claimed. The store
implementation is tools/swarmharness/evidence_versions.py (schema 2). This note refines the existing swarmharness2 law
that independent evidence, rather than repeated correlated representations,
controls repair eligibility. It must receive architecture review before an
implementation is delegated.

A normalized fingerprint deliberately identifies several byte strings with
one symptom. Let L = H(source-kind, physical-lineage, observation-span,
detector, detector-version, normalized-fingerprint) be the logical occurrence
key. Its stability prevents a reread or changed sampled generation from
manufacturing another independent incident. That compression must not erase
observed evidence changes.

For a validated bounded page B at raw byte interval [a,b), with sampled
source generation g, define an evidence-window key
W = H(source-kind, physical-lineage, g, a, b, H(B)). Each consumed page
retains its evidence-window fact, even when it yields no new logical
observation. For an observation using window references E and carried
detector context C, define V = H(L, sorted(E), H(C)). The references label
which bytes were actually observed; a current page does not impersonate an
earlier carried error record. Carried context without a source-byte binding
remains explicitly unknown evidence rather than a fabricated byte digest.

Hashes here mean deterministic content addressing; the implementation uses
SHA-256 and does not claim collision impossibility. Window and version facts
store logical keys, source generation, exact page intervals, byte digests,
detector identity and provenance. They store no raw prompt, transcript,
credential or page text. A page digest is explicitly an evidence-window
digest, not a claim that it hashes only the observation's single line.

Logical recurrence and independent-lineage counts remain functions of L
and its source lineage, never the number of V rows. A changed same-offset
widget1/widget2 record may have the same normalized fingerprint and L, but
must retain both observed evidence versions and their provenance. Repeated
capture of exactly the same version is idempotent. Different page geometry
or generation can create another evidence view without another vote.

Historical normalized records lacking original byte evidence are retained
as legacy stored-record provenance with raw evidence unavailable. A digest
of their serialized normalized fields is never relabeled a source-byte
hash. Existing exact original migration archives remain authoritative for
what bytes they actually preserve. No backfilled source generation or
independent confirmation is invented.

Version writes precede logical projection and cursor commitment. A crash
between them can cause reread and dedupe, not a cursor beyond unrecorded
evidence. Corrupt or future version-store schemas hold before further source
reads or claims; damaged bytes remain exact and inspectable. An append or
storage error cannot authorize a successful pass. Current logical projection
counts, immutable migration archives and evidence versions have distinct
owners and meanings. Retention defaults to preserving referenced evidence;
future garbage collection requires an explicit owner adapter and fences.

Falsifiers: same-fingerprint changed bytes at the same offset, unchanged
reread, equal-byte generation changes, exact byte offsets after invalid UTF-8
and Unicode-only separators, detector state carried across pages, two real
independent lineages, corrupt/future version metadata, disk failure before
each durable write, crash/replay between version/projection/cursor writes,
and a counterexample that fails the original CSR09 baseline. All existing
raw-offset, stream, tail, source-race, store and kernel controls must remain
valid. This does not prove evaluator improvement, repair execution or system
convergence. Those require the separate externally observed outcomes in the
research evaluation protocol.

## Canonical typed encodings (implementation contract, schema 2)

Canonical serialization is `json.dumps(obj, sort_keys=True, separators=(",", ":"))`
encoded UTF-8; H(x) is the lowercase hex SHA-256 of those bytes. Timestamps are
provenance columns only and never enter any identity. Identities use the `L-`,
`W-`, `C-`, `V-`, `U-`, `G-` prefixes followed by 64 hex characters.

- **Logical occurrence L** = `L-` + H([source_kind, lineage_id, span,
  detector_id, detector_version, fingerprint]) — the same six elements as the
  composer's v2 logical identity, computed by the store so the store does not
  import composer.py. Generation is never an element of L.
- **Window W** = `W-` + H([source_kind, lineage_id, generation, start, end,
  H(B)]) for a page of raw bytes B over the half-open interval [start, end).
  When raw is None (an oversized or otherwise uncaptured page) the content
  element is the literal sentinel string `"UNKNOWN"` — a typed gap — never a
  fabricated byte digest. The window row stores `page_digest` = H(B) or the
  sentinel and `raw_sha256` = H(B) or NULL, plus source_kind, lineage_id,
  generation, start, end, created_at. For a typed gap the stored interval is
  `[start, start)` — the uncaptured length is unknown and never invented.
  Raw page text itself is never stored;
  the digest is the durable binding. A window fact is written for every
  consumed page, including pages that yield no observation.
- **Context C** = `C-` + H(context_value), where context_value is the
  carried detector context or `None` when the observation carries no
  context. **One authoritative source per observation:** the observation's
  own `context` when present, otherwise the call's top-level `context`;
  supplying both for one observation is refused (`EvidenceHeld`), decided
  before any I/O. Every accepted context is identity-bound: it determines
  the version's C digest, so a per-observation carried-state change
  produces a different V.
- **Context schema (closed typed metadata projection, schema 2).** A
  context value is a nonempty JSON object (or `None`). Dict keys are
  closed metadata names (`depth`, `count`, `counts`, `schema_version`,
  `unknown`, `carried`, `carried_observations`, `carried_digest`,
  `prior_offset`, `prior_window_id`, `prior_page_digest`, `cmd_fps`,
  `pending_fps`, `att_offs`, `fp`, `window`, `byte_off`) or 16/64-hex
  fingerprint/digest keys. Arbitrary body-bearing keys are refused.
  Values admit NO free text: only
  `null`, booleans, bounded integers, 64-hex digest strings, store
  identity strings (`W-`/`V-`/`C-`/`U-`/`L-`/`G-` + 64 hex), and nested
  lists/objects of the same, depth ≤ 4 and ≤ 64 nodes. Any other string
  — prompt, credential, path, prose, enum text — is refused
  structurally (not by secret-marker pattern matching); floats are
  outside the projection. Body-bearing carried values travel only as
  their canonical digest under a supported metadata field, or as an
  `unbound` reference (typed UNKNOWN). Declared keys with meaning to
  the binding rule: `prior_offset` (nonnegative int), `prior_window_id`
  (`W-` identity), `prior_page_digest` (64 hex).
- **Carried binding.** An observation whose effective context declares
  any of `prior_offset` / `prior_window_id` / `prior_page_digest` must
  be durably bound or explicitly unknown: `prior_window_id` must be
  among the observation's window references; `prior_offset` must be
  covered by some referenced window's stored half-open interval (the
  current page counts only when its own interval covers the offset —
  the current page is never credited for earlier bytes); if neither
  holds, at least one `unbound` ref must mark the carried value
  explicitly unknown, else the call HOLDS.
- **References E** are the observation's explicit list of which bytes were
  actually observed: always the current page's own window
  (`{"kind": "window", "window_id": W}`) plus each explicit caller ref,
  each ref one of exactly two closed shapes:
  - `{"kind": "window", "window_id": W[, "expected_digest": hex64]}` — a
    reference to a durable window that MUST already exist (or be the current
    page's window). A missing window, or a supplied expected_digest that
    contradicts the stored window digest, HOLDS the whole call
    (EvidenceHeld): references never invent prior raw content.
  - `{"kind": "unbound", "unbound_value": str|bytes}` — carried context with
    no source-byte binding. It is hashed to `U-` + H(unbound_value) and only
    the `{"kind": "unbound", "unbound_id": U}` encoding is persisted; the
    unbound_value itself is never stored and never presented as a byte
    digest. This is the typed UNKNOWN evidence.
  E is ordered by the canonical encoding of its refs for identity purposes.
- **Version V** = `V-` + H([L, sorted(E-canonical-encodings), C-digest]) —
  an immutable evidence version of one observation over referenced windows.
  The version row stores V, L, the observation's own window_id, context_id,
  detector_id, detector_version, severity, status, span, fingerprint,
  created_at, and its refs in a child table (one row per ref, in E order).
- **Legacy record G** = `G-` + H(canonical(legacy row)). A legacy row is a
  historical normalized observation (the composer's closed eleven-field row
  shape) lacking original byte evidence; its `record_digest` is the digest of
  the serialized normalized fields and is explicitly NOT a raw-source hash.
  Its `raw_evidence` is the constant `unavailable`. Generation is taken from
  the row or stored as `"UNKNOWN"`; none is backfilled.
- **Source descriptor**: record_page accepts the current census entry
  (project, source_kind, lineage_id, generation). Only source_kind,
  lineage_id and generation are persisted; the project field — which is a
  path — is validated and dropped, never stored.

**Writes and holds.** record_page and record_legacy apply every insert inside
one SQLite transaction (BEGIN IMMEDIATE) under a stable writer flock
(`<state_dir>/evidence-versions.lock`); any validation, missing-window,
digest-contradiction, integrity or storage fault raises EvidenceHeld and rolls
the fact transaction back — original evidence is preserved and no cursor may advance
on a held call. Exact replays create nothing (same IDs, zero created counts,
timestamps untouched); before a replay is accepted, the persisted window,
context, logical-key and version identities and the version's full child-ref
list recompute from the stored payloads — a tampered or missing dependency
holds, never silently passes. A colliding ID with different stored payload is
held, never overwritten. A store file that exists is validated through a
genuinely read-only connection (URI `mode=ro`; a read/write handle with
`query_only` is NOT a read-only open, and `immutable=1` is never used on a
live WAL): an empty file, a non-SQLite file, a store without the schema
marker, or a future/unknown schema raises EvidenceHeld BEFORE any read/write
handle, journal-mode change or schema creation — the DB and WAL bytes stay
byte-identical and sidecars are never removed or checkpointed. (The `-shm`
is volatile SQLite reader coordination scratch; its read-mark bytes may
change under any reader, including read-only ones; DB and WAL evidence
bytes never do.) Only a store file proven absent — no DB, no WAL, no SHM —
may genesis: the complete schema is built in a private temp file, fsynced,
and published with a no-replace hardlink under the writer flock, so a crash
mid-genesis leaves at most an unreferenced temp file and concurrent writers
cannot race an unknown store into existence. `query_only`-style write
preflight happens under the lock only after the read-only marker validation.
preflight itself opens read-only: absent store
returns `{"status": "absent", "schema_version": None}` without creating or
writing anything; a ready store returns schema_version; a corrupt, unreadable,
or unknown/future-schema store raises EvidenceHeld and is never modified.
Contradictory old-schema data holds; there is no self-repair or destructive
migration. read_summary returns bounded counts and at most `limit` recent
window/version metadata rows for verification; it never contains raw page,
prompt, token or credential text. Row counts are storage facts, never
logical incident or lineage counts.


Every current observation has a canonical `bytes@N` span. Independently of
which context fields it declares, that offset must lie inside a referenced
known-content window of the same source kind and lineage, or carry an
explicit unbound reference. A window from another lineage or the current
page at another offset cannot substitute for the earlier evidence. Valid
fixtures use real byte positions; Unicode normalization is tested through
accepted metadata such as fingerprints, not a malformed byte offset.

Existing stores must match the complete known table/column/dependency
schema before any write-mode journal change or initialization. A marker
alone cannot authorize a partial, extended or unrelated SQLite schema.
This schema check does not certify every unreferenced stored fact; facts
and their dependencies are recomputed when referenced or replayed.

Named carried context fields must cohere with the same window: a supplied
`prior_window_id` must match a durable same-source reference, its offset must
fall inside that named window, and its declared byte digest must agree. A
contradictory named fact is refused even if an unrelated current window or
an unbound reference is also present. Unknown prior evidence can instead be
explicitly labeled unbound without asserting contradictory known facts.


The repaired standalone storage proposal uses schema marker **2**. Prototype
schema-1 stores are held without automatic mutation; none was qualified for
production. Indexed incoming dependency checks prevent a new capture from
silently restoring a W/C row still referenced by an older version. Existing
candidate V graphs are validated before dependency creation. Canonical E
ordering is used for identity and new child-row persistence; replay compares
canonical dependency facts, preserving valid equivalent caller permutations
without rewriting their provenance.

A held call never authorizes cursor advancement. Its fact transaction rolls
back, but complete empty genesis files or a namespace publication with
unconfirmed durability can remain. They are retained, never claimed absent.
Existing-store admission retries parent-directory synchronization before
writes; persistent failure stays held. A failed exclusive temporary-file
creation never authorizes deleting an occupied temporary name. New genesis
files are private0600 and referenced live SQLite/WAL/SHM files are not moved.

Read-only SQLite admission preserves existing durable evidence and never
initializes schema or writes facts. It can create empty WAL/SHM coordination
files for a WAL-mode store, including a rejected schema. Their possible
creation is not a successful admission, new evidence or cursor authorization;
zero filesystem effects are not promised.
