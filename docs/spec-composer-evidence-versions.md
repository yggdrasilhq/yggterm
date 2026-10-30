# Composer logical observations and immutable evidence versions

Status: proposed source contract for CSR09; no implementation or empirical
benefit is claimed here. This note refines the existing swarmharness2 law
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
