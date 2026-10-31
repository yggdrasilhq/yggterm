"""Read-only admission-to-completion eligibility, NOT a dispatcher or approval store.

API: assess(admission, job, observation, review) -> Decision. Inputs are closed
schema-1 dictionaries (see dataclasses); malformed/legacy inputs HOLD. Admission
and Job are immutable identities; their digest is canonical JSON, without host,
time or root-path metadata. Prompt hashes are references, never authorization.
Only an already-authorized controller may admit work.

The controller must obtain observations, configured editor policy and a genuine
SessionReview with trusted verifiers OUTSIDE the worker workspace. A worker
providing its own verifier/key defeats that boundary. This module cannot prove
caller authority. It issues no permission token and writes no queue or review.
An eligible decision contains exact CAS expectations, not transport execution:
the controller MUST atomically compare them at apply, including a fresh subject
rehash and chain head. This pure kernel cannot close filesystem/queue TOCTOU.

Supported review adapter: opened.worker is exactly {harness, native_uuid};
opened.editor_binding is a JSON mapping with identity, hashed canonically;
opened.config_digest binds the activated policy. Full/lightweight reviews only.
The signed editor invocation_ref must be a canonical native UUID distinct from
the worker UUID; a role label or opaque invocation string proves no separation.
Other invocation-address formats require a qualified adapter and HOLD here.
Payload-only reviews hold: current identity must come from actual staged files.
Existing SessionReview private validation helpers are deliberately reused here;
terminal resolve() alone caches acceptance and does not reverify provenance.
"""
from dataclasses import asdict, dataclass
from enum import Enum
import json
from pathlib import Path
import re
import uuid

from .session_review import SessionReview, _canonical, _sha, _live_digest


class Status(str, Enum):
    ELIGIBLE = 'ELIGIBLE'
    HOLD = 'HOLD'


class Fault(str, Enum):
    MALFORMED = 'malformed_contract'
    OBSERVATION = 'observation_mismatch'
    REVIEW = 'untrusted_or_incomplete_review'


@dataclass(frozen=True)
class Admission:
    schema: int
    task_id: str
    dispatch_uuid: str
    prompt_sha256: str
    criteria: tuple
    worker_harness: str
    worker_native_uuid: str
    editor_identity: str
    editor_binding_digest: str
    config_digest: str
    owner_stop_generation: int

    def __post_init__(self):
        # Defensive copy: a caller retaining its wire-format criteria list
        # cannot change an admitted identity through an alias.
        if isinstance(self.criteria, (list, tuple)):
            object.__setattr__(self, 'criteria', tuple(self.criteria))

    @property
    def digest(self):
        return _sha(_canonical(asdict(self)))


@dataclass(frozen=True)
class Job:
    schema: int
    admission_digest: str
    subject_digest: str
    review_id: str

    @property
    def digest(self):
        return _sha(_canonical(asdict(self)))


@dataclass(frozen=True)
class Observation:
    schema: int
    task_id: str
    dispatch_uuid: str
    prompt_sha256: str
    admission_digest: str
    owner_stop_generation: int
    state: str
    editor_identity: str
    editor_binding_digest: str
    config_digest: str


@dataclass(frozen=True)
class Preconditions:
    observation: Observation
    job: Job
    review_head: str
    subject_digest: str


@dataclass(frozen=True)
class Decision:
    status: Status
    fault: Fault | None = None
    preconditions: Preconditions | None = None


def binding_digest(binding):
    """Canonical configured binding digest; caller supplies trusted policy."""
    return _sha(_canonical(binding))


def _parse(cls, value):
    if type(value) is cls:
        value = asdict(value)
    if type(value) is not dict or set(value) != set(cls.__dataclass_fields__):
        raise ValueError('closed schema required')
    value = dict(value)
    if type(value['schema']) is not int or value['schema'] != 1:
        raise ValueError('unknown schema')
    for key, item in value.items():
        if key == 'criteria':
            if (type(item) not in (tuple, list) or not item
                    or any(type(c) is not str or not c.strip() for c in item)
                    or len(set(item)) != len(item)):
                raise ValueError('original explicit criteria required')
            value[key] = tuple(item)
        elif key == 'owner_stop_generation':
            if type(item) is not int or item < 0:
                raise ValueError('invalid generation')
        elif key != 'schema':
            if type(item) is not str or not item.strip():
                raise ValueError('nonempty string required')
            if key.endswith(('digest', 'sha256')) and not re.fullmatch('[0-9a-f]{64}', item):
                raise ValueError('invalid digest')
            if key.endswith('uuid') and str(uuid.UUID(item)) != item:
                raise ValueError('canonical UUID required')
    return cls(**value)


def _snapshot(row):
    if set(row) != {'kind', 'round', 'subject_digest', 'subject_manifest',
                    'base_revision', 'evidence_manifest', 'completion_claim',
                    'at', 'seq', 'prev_hash', 'record_hash'}:
        raise ValueError('unknown snapshot fields')
    m = row['subject_manifest']
    if type(m) is not dict or set(m) != {'kind', 'files'} or m['kind'] != 'files':
        raise ValueError('actual file subject required')
    files = m['files']
    if type(files) is not list or not files:
        raise ValueError('empty files')
    paths = []
    for f in files:
        if type(f) is not dict or set(f) != {'path', 'sha256'}:
            raise ValueError('malformed file')
        p = f['path']
        if (type(p) is not str or not Path(p).is_absolute()
                or not re.fullmatch('[0-9a-f]{64}', f['sha256'])):
            raise ValueError('invalid file identity')
        paths.append(p)
    if paths != sorted(set(paths)) or _sha(_canonical(m)) != row['subject_digest']:
        raise ValueError('invalid subject identity')
    if (type(row['round']) is not int or row['round'] < 1
            or type(row['completion_claim']) is not str
            or not row['completion_claim'].strip()):
        raise ValueError('invalid snapshot')


class _Replay(SessionReview):
    """Replay kernel validation in memory; historical files need not still exist.

    Historical manifests are authenticated by receipts. Only the CURRENT file
    set is rehashed. No real review writes or default control-key reads occur.
    """
    def __init__(self, source, opened):
        self.review_id = source.review_id
        self.history = [opened]
        self.expected = None
        self.validating = False
        self._debt_verifier = source._debt_verifier
        self._exemption_verifier = None

        def verify(rid, op, snap, editor, payload, head):
            original = [r for r in self.history if r['kind'] == 'snapshot_staged'][-1]
            return source._provenance_verifier(rid, op, original, editor, payload, head)
        self._provenance_verifier = verify

    def _read_rows(self):
        return list(self.history)

    def snapshots(self):
        snaps = super().snapshots()
        if self.validating:
            return [dict(s, subject_manifest={'kind': 'payload'}) for s in snaps]
        return snaps

    def _chain_append(self, row):
        expected = self.expected
        excluded = {'at', 'seq', 'prev_hash', 'record_hash'}
        if ({k: v for k, v in row.items() if k not in excluded}
                != {k: v for k, v in expected.items() if k not in excluded}):
            raise ValueError('stored record differs from validated payload')
        self.history.append(expected)


def _verify_review(review, admission, job):
    # Exact class prevents a caller-supplied rows/_evaluate facade. Authority of
    # this handle and its injected verifiers remains the controller's duty.
    if type(review) is not SessionReview or review._provenance_verifier is None:
        raise ValueError('trusted SessionReview required')
    rows = review.rows
    if not rows or review.review_id != job.review_id:
        raise ValueError('missing review')
    opened = rows[0]
    if (opened.get('worker') != {'harness': admission.worker_harness,
                                'native_uuid': admission.worker_native_uuid}
            or opened.get('acceptance_criteria') != list(admission.criteria)
            or opened.get('tier') not in ('full', 'lightweight')
            or opened.get('config_digest') != admission.config_digest):
        raise ValueError('opening differs from admission')
    binding = opened.get('editor_binding')
    if (type(binding) is not dict or binding.get('identity') != admission.editor_identity
            or binding_digest(binding) != admission.editor_binding_digest
            or admission.editor_identity in (admission.worker_native_uuid,
                admission.worker_harness + ':' + admission.worker_native_uuid)):
        raise ValueError('editor binding mismatch or worker is editor')
    replay = _Replay(review, opened)
    for row in rows[1:]:
        replay.expected = row
        kind = row['kind']
        if kind == 'snapshot_staged':
            _snapshot(row)
            if row['round'] != len(replay.snapshots()) + 1:
                raise ValueError('snapshot round gap')
            replay.history.append(row)
        elif kind == 'editor_verdict':
            if row['editor']['identity'] != admission.editor_identity:
                raise ValueError('wrong editor')
            invocation = row['editor'].get('invocation_ref')
            if (type(invocation) is not str
                    or str(uuid.UUID(invocation)) != invocation
                    or invocation == admission.worker_native_uuid):
                raise ValueError('independent native editor invocation required')
            if row['verification_head'] != replay.history[-1]['record_hash']:
                raise ValueError('receipt head mismatch')
            payload = row['signed_payload']
            replay.validating = True
            try:
                result = SessionReview.ingest_verdict.__wrapped__(
                    replay, snapshot_digest=row['snapshot_digest'],
                    editor=row['editor'], **payload)
            finally:
                replay.validating = False
            if result.get('ok') is not True:
                raise ValueError('verdict refused')
        elif kind == 'worker_disposition':
            result = SessionReview.disposition.__wrapped__(replay,
                row['finding_id'], row['action'], row['evidence_refs'])
            if result.get('ok') is not True:
                raise ValueError('disposition refused')
        elif kind == 'debt_accepted':
            if row['verification_head'] != replay.history[-1]['record_hash']:
                raise ValueError('debt head mismatch')
            result = SessionReview.defer_accepted.__wrapped__(replay,
                row['finding_id'], **{k: row[k] for k in (
                    'owner', 'tracking_ref', 'revisit', 'authority',
                    'waives_criterion', 'authority_receipt')})
            if result.get('authority_verified') is not True:
                raise ValueError('debt refused')
        elif kind == 'resolution':
            if row['outcome'] == 'OWNER_STOPPED':
                raise ValueError('owner stopped')
            if row['outcome'] not in ('COMPLETE', 'REVISE', 'SUSPENDED'):
                raise ValueError('unknown resolution')
            if row['outcome'] == 'COMPLETE' and row.get('result') != replay._evaluate():
                raise ValueError('forged terminal resolution')
            replay.history.append(row)
            if row['outcome'] == 'COMPLETE' and row is not rows[-1]:
                raise ValueError('writes after terminal resolution')
        else:
            raise ValueError('unknown review event')
    snap = replay.snapshots()[-1]
    if (snap['subject_digest'] != job.subject_digest
            or _live_digest(snap) != job.subject_digest):
        raise ValueError('subject drift')
    # Reevaluate rather than trusting a stored result claiming COMPLETE.
    if replay._evaluate()['outcome'] != 'COMPLETE':
        raise ValueError('canonical policy does not accept')
    if rows[-1]['kind'] != 'resolution' or rows[-1]['outcome'] != 'COMPLETE':
        raise ValueError('canonical COMPLETE resolution required')
    if rows[-1].get('result') != replay._evaluate():
        raise ValueError('resolution differs from canonical result')
    # Reject concurrent writes; actual queue apply must repeat this comparison.
    if review.rows != rows:
        raise ValueError('review changed while inspecting')
    return rows[-1]['record_hash']


def assess(admission, job, observation, review):
    """Deterministic typed HOLD or eligibility with exact future CAS inputs.

    No model calls, persistence, queue mutation, or acceptance credential.
    Unknown state/old tasks without an admission always hold.
    """
    try:
        a = _parse(Admission, admission)
        j = _parse(Job, job)
        o = _parse(Observation, observation)
    except (ValueError, TypeError, KeyError, AttributeError):
        return Decision(Status.HOLD, Fault.MALFORMED)
    if (j.admission_digest != a.digest or o.admission_digest != a.digest
            or o.state != 'review_pending'
            or any(getattr(o, k) != getattr(a, k) for k in (
                'task_id', 'dispatch_uuid', 'prompt_sha256', 'owner_stop_generation',
                'editor_identity', 'editor_binding_digest', 'config_digest'))):
        return Decision(Status.HOLD, Fault.OBSERVATION)
    try:
        head = _verify_review(review, a, j)
    except Exception:
        # Storage/provenance failures retain work; never authorize completion.
        return Decision(Status.HOLD, Fault.REVIEW)
    return Decision(Status.ELIGIBLE, preconditions=Preconditions(o, j, head, j.subject_digest))
