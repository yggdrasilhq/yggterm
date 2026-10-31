"""Staging only for an admitted, claimed local review job.

Trusted same-UID controller, qualified local POSIX storage and cooperative root
lifecycle writers are required. No policy resolution, lease acquisition, native
launch, provenance authority, verdict or completion endpoints exist here.

Bridge(root, snapshots, worker_workspace=..., qualified_local=True).stage(...)
uses a pre-existing private root OUTSIDE the worker workspace. One bridge root
belongs to one operation/job; reviews live beneath it. Supply the original CAS
revision/token/generation and stable operation UUID on EVERY retry. Exact retry
revalidates Git capture, physical roots, immutable intent and the entire initial
canonical chain before reconciling the same job operation. Later native history,
partial/corrupt writes, changed roots or conflicting requests HOLD, retained.
No migration, automatic repair, fallback, exemption or succession is supported.
The staging receipt grants no launch permission. Controller policy/quota/budget/
deny-latch checks remain mandatory future work. Fences contain local IO only.
"""
from dataclasses import asdict, dataclass
from contextlib import contextmanager
import fcntl
import json
import os
from pathlib import Path
import stat

from . import review_jobs as jobs
from . import review_snapshot as capture
from .review_completion import Admission, Job, _parse, binding_digest
from .session_review import SessionReview, _subject_identity, _now

PROTOCOL = 'review-dispatch-v1'


class Hold(Exception):
    """Retained unresolved staging work; no launch or completion permission."""


@dataclass(frozen=True)
class Receipt:
    mapping: dict
    job_state: dict
    reconciled: bool


def _identity(path):
    info = capture._directory(path)
    return [str(path), info.st_dev, info.st_ino]


class Bridge:
    def __init__(self, root, snapshots, *, worker_workspace, qualified_local=False):
        # Reuse the accepted storage component's private-root and stable fence.
        self.storage = capture.Store(root, qualified_local=qualified_local)
        self.snapshots = snapshots
        self.workspace = Path(worker_workspace).resolve(strict=True)
        for path in (self.storage.root, snapshots.root):
            if path == self.workspace or self.workspace in path.parents:
                raise Hold('storage_inside_worker_workspace')

    def _checkpoint(self, phase):
        """Local fault-injection seam only; production does nothing."""

    @contextmanager
    def _review_lock(self, review):
        fd = os.open(str(review.path) + '.lock',
                     os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
        try:
            info = os.fstat(fd)
            if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid()
                    or info.st_nlink != 1 or info.st_mode & 0o022
                    or info.st_size != 0):
                raise Hold('unsafe_review_lock')
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            capture._syncdir(review.path.parent)
            yield
        finally:
            os.close(fd)

    def stage(self, store, *, expected_revision, token, generation, operation_id,
              repo, request, editor_binding, config_digest, tier,
              completion_claim):
        """Controller inputs are assertions to compare with durable admission.

        request is review_snapshot schema 1; paths must equal sealed source_scope.
        editor_binding is already selected by trusted policy. Completion claim is
        worker evidence only. Original request hash/parent references come ONLY
        from the durable envelope. Raises Hold with bounded codes on uncertainty.
        """
        try:
            # Freeze caller-owned mutable input before any validation or IO.
            values = json.loads(jobs._bytes(dict(request=request,
                editor_binding=editor_binding)))
            with self.storage._lock():
                return self._stage(store, expected_revision, token, generation,
                    operation_id, repo, values['request'], values['editor_binding'],
                    config_digest, tier, completion_claim)
        except Hold:
            raise
        except (jobs.Hold, capture.Hold, OSError, ValueError, TypeError,
                KeyError, AttributeError, RuntimeError):
            raise Hold('staging_unresolved') from None

    def _stage(self, store, revision, token, generation, operation, repo, request,
               binding, config, tier, claim):
        jobs._uuid(operation)
        job_root = Path(store.root).resolve(strict=True)
        if job_root == self.workspace or self.workspace in job_root.parents:
            raise Hold('job_storage_inside_worker_workspace')
        s = store.get().state
        e = s['envelope']
        jobs._envelope(e)
        a = _parse(Admission, e['admission'])
        if (e['protocol'] != PROTOCOL or tier not in ('full', 'lightweight')
                or type(binding) is not dict
                or binding.get('identity') != a.editor_identity
                or binding_digest(binding) != a.editor_binding_digest
                or config != a.config_digest):
            raise Hold('unsupported_protocol_or_binding')
        # Scope comparison happens before capture reads any source.
        paths = capture._request(request)
        if paths != e['source_scope']:
            raise Hold('sealed_scope_mismatch')
        if (type(revision) is not int or type(generation) is not int
                or s['stopped'] or s['unresolved']
                or generation != s['generation']
                or generation != a.owner_stop_generation
                or not s['attempt'] or token != s['attempt']['token']
                or s['state'] not in ('claimed', 'staged')
                or s['revision'] != revision + (s['state'] == 'staged')):
            raise Hold('claim_mismatch')
        if type(claim) is not str or not claim.strip() or len(claim) > 500:
            raise Hold('bounded_worker_claim_required')
        request = dict(request, paths=paths)
        plan = dict(schema=1, job_id=s['job_id'], admission_digest=a.digest,
            expected_revision=revision, token=token, generation=generation,
            operation_id=operation, request=request, repo=str(Path(repo).resolve()),
            editor_binding=binding, config_digest=config, tier=tier,
            completion_claim=claim, roots=[_identity(self.storage.root),
                _identity(self.snapshots.root), _identity(store.root)])
        root = self.storage.root
        if {p.name for p in root.iterdir()} - {'.lock', 'intent.json', 'reviews'}:
            raise Hold('unknown_bridge_storage')
        intent = root / 'intent.json'
        had_intent = os.path.lexists(intent)
        if had_intent:
            raw, mode = capture._read(intent, capture.MAX_MANIFEST)
            if raw != jobs._bytes(plan) or mode != 0o400:
                raise Hold('conflicting_or_relocated_intent')
        elif s['state'] != 'claimed' or os.path.lexists(root / 'reviews'):
            raise Hold('missing_intent')
        snapshot = self.snapshots.stage(repo, request)
        if snapshot.artifact_id != e['artifact_sha256']:
            raise Hold('admitted_artifact_mismatch')
        self._checkpoint('after_capture')
        if not had_intent:
            capture._write(intent, jobs._bytes(plan))
            capture._syncdir(root)
        self._checkpoint('after_intent')
        base = root / 'reviews'
        if not os.path.lexists(base):
            if s['state'] != 'claimed':
                raise Hold('missing_review_storage')
            base.mkdir(mode=0o700)
        capture._directory(base)
        capture._syncdir(root)
        review = SessionReview(s['review_id'], base=base)
        allowed = {review.path.name, review.path.name + '.lock'}
        for p in base.iterdir():
            info = p.lstat()
            if (p.name not in allowed or not stat.S_ISREG(info.st_mode)
                    or info.st_uid != os.getuid() or info.st_nlink != 1
                    or info.st_mode & 0o022 or info.st_size > 1024 * 1024):
                raise Hold('unsafe_or_unknown_review_storage')
        opened = dict(kind='review_opened', review_id=s['review_id'],
            worker=dict(harness=a.worker_harness, native_uuid=a.worker_native_uuid),
            acceptance_criteria=list(a.criteria), tier=tier,
            exemption_receipt=None, config_digest=a.config_digest,
            editor_binding=binding)
        with self._review_lock(review):
            # Public create/stage helpers reacquire a blocking lock. Use the
            # canonical appender under this already-held, bounded fence for
            # these two trusted controller events only; never editor verdicts.
            if not os.path.lexists(review.path):
                if s['state'] != 'claimed':
                    raise Hold('missing_canonical_review')
                review._chain_append(dict(opened, at=_now()))
            capture._syncdir(base)
            rows = review.rows
            def body(row):
                if (not isinstance(row.get('at'), str)
                        or set(row) != set(opened if row.get('kind') == 'review_opened'
                            else staged) | {'at', 'seq', 'prev_hash', 'record_hash'}):
                    raise Hold('unknown_chain_shape')
                return {k: v for k, v in row.items()
                        if k not in ('at', 'seq', 'prev_hash', 'record_hash')}
            if not rows or body(rows[0]) != opened or len(rows) > 2:
                raise Hold('conflicting_or_later_review')
            self._checkpoint('after_open')
            subject_digest, manifest = _subject_identity(snapshot.files)
            local_job = Job(1, a.digest, subject_digest, s['review_id'])
            mapping = dict(schema=1, artifact_id=snapshot.artifact_id,
                local_job=asdict(local_job), local_job_digest=local_job.digest,
                materialization=snapshot.materialization,
                provenance_receipt=snapshot.provenance_receipt,
                original_request_sha256=e['original_request_sha256'],
                parent_ref=e['parent_ref'], plan_digest=jobs.digest(plan))
            staged = dict(kind='snapshot_staged', round=1,
                subject_digest=subject_digest, subject_manifest=manifest,
                base_revision=request['commit'], evidence_manifest=mapping,
                completion_claim=claim)
            if len(rows) == 1:
                if s['state'] != 'claimed':
                    raise Hold('missing_staged_chain')
                review._chain_append(dict(staged, at=_now()))
                rows = review.rows
            if len(rows) != 2 or body(rows[1]) != staged:
                raise Hold('conflicting_snapshot')
            capture._syncdir(base)
            self._checkpoint('after_stage')
            # Stop uses the job store fence; preserve the original stage CAS.
            final_rows = review.rows
            if (len(final_rows) != 2 or body(final_rows[0]) != opened
                    or body(final_rows[1]) != staged
                    or _subject_identity(snapshot.files)[0] != subject_digest):
                raise Hold('chain_changed_before_cas')
            receipt = store.change('stage', operation_id=operation,
                expected_revision=revision, generation=generation, token=token,
                data=dict(local_subject_digest=subject_digest))
        self._checkpoint('after_cas')
        current = receipt.state
        if (current['state'] != 'staged' or current['stopped']
                or current['revision'] != revision + 1
                or current['attempt']['local_subject_digest'] != subject_digest):
            raise Hold('later_job_state')
        return Receipt(mapping, current, receipt.reconciled)
