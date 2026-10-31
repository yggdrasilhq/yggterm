"""Hermetic controller fixtures; no native models, real keys or queue IO.

Run: python3 -m unittest tools.swarmharness.test_review_completion -v
"""
from dataclasses import asdict, replace
import json
from pathlib import Path
import tempfile
import unittest

from .session_review import (SessionReview, editor_receipt, verdict_payload,
    hmac_provenance_verifier, hmac_debt_verifier, debt_receipt, _canonical, _sha)
from .review_completion import (Admission, Job, Observation, Status, Fault,
    assess, binding_digest)


class CompletionTests(unittest.TestCase):
    def setUp(self):
        scratch = Path.home() / '.yggterm/scratchpad'
        scratch.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=scratch)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.file = self.root / 'subject.txt'
        self.file.write_text('invented fixture content')
        self.key = b'isolated-fixture-editor-key'
        key_path = self.root / 'fixture.key'
        key_path.write_bytes(self.key)
        self.verifier = hmac_provenance_verifier(key_path)
        self.debt_verifier = hmac_debt_verifier(key_path)
        self.binding = {'identity': 'fixture-editor', 'role': 'editor'}
        self.a = Admission(1, 'fixture-task',
            '00000000-0000-4000-8000-000000000001', '1' * 64,
            ('original criterion',), 'fixture-worker',
            '00000000-0000-4000-8000-000000000002',
            'fixture-editor', binding_digest(self.binding), '2' * 64, 0)
        self.r = SessionReview.create(
            worker={'harness': self.a.worker_harness, 'native_uuid': self.a.worker_native_uuid},
            acceptance_criteria=list(self.a.criteria), config_digest=self.a.config_digest,
            editor_binding=self.binding, base=self.root / 'reviews',
            review_id='fixture-review', provenance_verifier=self.verifier,
            debt_verifier=self.debt_verifier)
        self.snap = self.r.stage_snapshot(subject=[self.file], completion_claim='worker accepts')
        self.j = Job(1, self.a.digest, self.snap['subject_digest'], self.r.review_id)
        self.o = Observation(1, self.a.task_id, self.a.dispatch_uuid,
            self.a.prompt_sha256, self.a.digest, 0, 'review_pending',
            self.a.editor_identity, self.a.editor_binding_digest, self.a.config_digest)

    def verdict(self, *, overall='accept', findings=None, dispositions=None,
                checks=True, key=None, scope=None):
        snap = self.r.snapshots()[-1]
        evidence = [{'status': 'pass', 'snapshot_digest': snap['subject_digest'],
                     'receipt_ref': 'fixture-check'}] if checks else None
        payload = verdict_payload(overall, findings or [], dispositions,
                                  list(self.a.criteria) if scope is None else scope, evidence)
        editor = {'identity': self.a.editor_identity, 'invocation_ref': 'fixture-invocation'}
        editor['invocation_receipt'] = editor_receipt(self.r.review_id,
            self.r.rows[0], snap, editor, payload, key=key or self.key,
            review_head=self.r.rows[-1]['record_hash'])
        return self.r.ingest_verdict(snapshot_digest=snap['subject_digest'],
                                    editor=editor, **payload)

    def complete(self):
        self.assertTrue(self.verdict()['ok'])
        self.assertEqual(self.r.resolve()['outcome'], 'COMPLETE')

    def decision(self, **kw):
        return assess(kw.get('admission', self.a), kw.get('job', self.j),
                      kw.get('observation', self.o), kw.get('review', self.r))

    def hold(self, **kw):
        d = self.decision(**kw)
        self.assertEqual(d.status, Status.HOLD)
        self.assertIsNone(d.preconditions)

    def finding(self):
        return {'id': 'F1', 'criterion': self.a.criteria[0], 'blocking': True,
                'evidence': 'fixture', 'expected': 'pass', 'observed': 'fail',
                'probe': 'fixture-probe'}

    def rewrite(self, mutate):
        rows = self.r.rows
        mutate(rows)
        prev = '0' * 64
        for seq, row in enumerate(rows):
            row.update(seq=seq, prev_hash=prev)
            row.pop('record_hash', None)
            row['record_hash'] = _sha(_canonical(row))
            prev = row['record_hash']
        self.r.path.write_text(''.join(json.dumps(r) + '\n' for r in rows))

    def test_positive_real_chain_and_exact_preconditions_no_writes(self):
        self.complete()
        before = {p: p.read_bytes() for p in self.root.rglob('*') if p.is_file()}
        d = self.decision()
        self.assertEqual(d.status, Status.ELIGIBLE)
        self.assertEqual(d.preconditions.observation, self.o)
        self.assertEqual(d.preconditions.job, self.j)
        self.assertEqual(d.preconditions.review_head, self.r.rows[-1]['record_hash'])
        self.assertEqual(d.preconditions.subject_digest, self.j.subject_digest)
        self.assertEqual(d, self.decision())
        self.assertEqual(before, {p: p.read_bytes() for p in self.root.rglob('*') if p.is_file()})

    def test_fake_outcome_dictionary(self):
        self.hold(review={'outcome': 'COMPLETE'})

    def test_no_editor_verdict(self):
        self.r.resolve()
        self.hold()

    def test_worker_key_is_not_editor_provenance(self):
        self.assertFalse(self.verdict(key=b'worker-forged-key')['ok'])
        self.hold()

    def test_missing_provenance_verifier(self):
        self.complete()
        self.hold(review=SessionReview(self.r.review_id, self.r.path.parent))

    def test_missing_receipt_even_with_rehashed_chain(self):
        self.complete()
        self.rewrite(lambda rows: rows[2]['editor'].pop('invocation_receipt'))
        self.hold()

    def test_failed_checks(self):
        self.verdict(checks=False)
        self.assertEqual(self.r.resolve()['outcome'], 'REVISE')
        self.hold()

    def test_partial_criteria_approval(self):
        self.verdict(scope=[])
        self.r.resolve()
        self.hold()

    def test_worker_repair_does_not_close(self):
        self.verdict(overall='revise', findings=[self.finding()])
        self.r.disposition('F1', 'repair')
        self.verdict()
        self.r.resolve()
        self.hold()

    def test_editor_closed_finding(self):
        self.verdict(overall='revise', findings=[self.finding()])
        self.r.disposition('F1', 'repair')
        self.verdict(dispositions=[{'id': 'F1', 'closed': True}])
        self.r.resolve()
        self.assertEqual(self.decision().status, Status.ELIGIBLE)

    def test_authorized_debt_reverified(self):
        self.verdict(findings=[self.finding()])
        debt = dict(finding_id='F1', owner='fixture-owner', tracking_ref='fixture-debt',
                    revisit='fixture-review', authority='criterion-owner', waives_criterion=True)
        receipt = debt_receipt(self.r.review_id, self.r.rows[0], [self.finding()],
                              debt, key=self.key, review_head=self.r.rows[-1]['record_hash'])
        result = self.r.defer_accepted('F1', **{k: v for k, v in debt.items() if k != 'finding_id'},
                                     authority_receipt=receipt)
        self.assertTrue(result['authority_verified'])
        self.r.resolve()
        self.assertEqual(self.decision().status, Status.ELIGIBLE)
        self.r._debt_verifier = None
        self.hold()

    def test_new_subject_after_terminal_acceptance(self):
        self.complete()
        self.file.write_text('new unreviewed bytes')
        self.assertEqual(self.r.resolve()['outcome'], 'COMPLETE')  # cached kernel result
        self.hold()

    def test_missing_current_file(self):
        self.complete()
        self.file.unlink()
        self.hold()

    def test_historical_subject_can_change_with_fresh_review(self):
        self.verdict(overall='revise')
        self.file.write_text('repaired fixture')
        self.snap = self.r.stage_snapshot(subject=[self.file], completion_claim='repaired')
        self.j = replace(self.j, subject_digest=self.snap['subject_digest'])
        self.complete()
        self.assertEqual(self.decision().status, Status.ELIGIBLE)

    def test_new_snapshot_requires_new_approval(self):
        self.verdict()
        self.r.stage_snapshot(subject=[self.file], completion_claim='new round')
        self.r.resolve()
        self.hold()

    def test_changed_criteria_even_if_observation_rebound(self):
        self.complete()
        a = replace(self.a, criteria=('changed criterion',))
        self.hold(admission=a, job=replace(self.j, admission_digest=a.digest),
                  observation=replace(self.o, admission_digest=a.digest))

    def test_editor_policy_drift(self):
        self.complete()
        self.hold(observation=replace(self.o, editor_binding_digest='3' * 64))
        self.hold(observation=replace(self.o, config_digest='3' * 64))
        self.hold(observation=replace(self.o, editor_identity='another-editor'))

    def test_worker_is_editor(self):
        self.binding['identity'] = self.a.worker_native_uuid
        self.a = replace(self.a, editor_identity=self.a.worker_native_uuid,
                         editor_binding_digest=binding_digest(self.binding))
        self.r = SessionReview.create(
            worker={'harness': self.a.worker_harness, 'native_uuid': self.a.worker_native_uuid},
            acceptance_criteria=list(self.a.criteria), config_digest=self.a.config_digest,
            editor_binding=self.binding, base=self.root / 'self-review',
            review_id='self-review', provenance_verifier=self.verifier)
        self.snap = self.r.stage_snapshot(subject=[self.file], completion_claim='accepted')
        self.j = replace(self.j, admission_digest=self.a.digest, review_id=self.r.review_id)
        self.o = replace(self.o, admission_digest=self.a.digest,
                         editor_identity=self.a.editor_identity,
                         editor_binding_digest=self.a.editor_binding_digest)
        self.complete()
        self.assertEqual(self.decision().fault, Fault.REVIEW)

    def test_binding_mismatch_even_with_new_admission(self):
        self.complete()
        a = replace(self.a, editor_binding_digest='6' * 64)
        self.hold(admission=a, job=replace(self.j, admission_digest=a.digest),
                  observation=replace(self.o, admission_digest=a.digest,
                                      editor_binding_digest=a.editor_binding_digest))

    def test_stored_fields_must_match_signed_payload(self):
        self.complete()
        self.rewrite(lambda rows: rows[2].update(prior_dispositions=[{'id': 'invented', 'closed': True}]))
        self.hold()

    def test_receipt_bound_to_exact_chain_head(self):
        self.complete()
        self.rewrite(lambda rows: rows[2].update(verification_head='7' * 64))
        self.hold()

    def test_payload_subject_cannot_claim_current_files(self):
        self.r = SessionReview.create(
            worker={'harness': self.a.worker_harness, 'native_uuid': self.a.worker_native_uuid},
            acceptance_criteria=list(self.a.criteria), config_digest=self.a.config_digest,
            editor_binding=self.binding, base=self.root / 'payload-review',
            review_id='payload-review', provenance_verifier=self.verifier)
        snap = self.r.stage_snapshot(subject='payload-only', completion_claim='accepted')
        self.verdict()
        self.r.resolve(current_subject_digest=snap['subject_digest'])
        self.j = replace(self.j, review_id=self.r.review_id, subject_digest=snap['subject_digest'])
        self.hold()

    def test_queue_state_cancel_or_unknown(self):
        self.complete()
        for state in ('cancelled', 'completed', 'submitted', 'retained_partial', 'unknown', 'owner_stopped'):
            with self.subTest(state=state):
                self.hold(observation=replace(self.o, state=state))

    def test_owner_stop_generation_changed(self):
        self.complete()
        self.hold(observation=replace(self.o, owner_stop_generation=1))

    def test_reused_task_ordinal_new_dispatch(self):
        self.complete()
        self.hold(observation=replace(self.o, dispatch_uuid='00000000-0000-4000-8000-000000000099'))

    def test_prompt_or_admission_mismatch(self):
        self.complete()
        self.hold(observation=replace(self.o, prompt_sha256='4' * 64))
        self.hold(observation=replace(self.o, admission_digest='4' * 64))
        self.hold(job=replace(self.j, admission_digest='4' * 64))

    def test_suspension_quota_storage(self):
        for reason in ('quota exhausted', 'storage unavailable'):
            self.r.suspend(reason)
            self.hold()

    def test_owner_stopped_chain(self):
        self.verdict()
        self.r.stop_owner('fixture stop')
        self.hold()

    def test_unresolved_chain_not_terminal(self):
        self.verdict()
        self.hold()

    def test_corrupt_chain_or_storage_gap(self):
        self.complete()
        self.r.path.write_text(self.r.path.read_text().replace('fixture-invocation', 'forged-invocation'))
        self.hold()
        self.r.path.unlink()
        self.hold()

    def test_forged_complete_after_suspension(self):
        self.verdict()
        self.r.suspend('quota')
        self.r._chain_append({'kind': 'resolution', 'outcome': 'COMPLETE',
            'result': {'outcome': 'COMPLETE', 'reasons': [], 'unresolved': [], 'accepted_debts': []}})
        self.hold()

    def test_rehashed_worker_verdict_or_modified_signed_payload(self):
        self.complete()
        self.rewrite(lambda rows: rows[2]['signed_payload'].update(overall='revise'))
        self.hold()

    def test_malformed_closed_schema_inputs(self):
        self.complete()
        for name, obj in [('admission', self.a), ('job', self.j), ('observation', self.o)]:
            base = asdict(obj)
            variants = [None, {}, dict(base, unknown=True), dict(base, schema=True), dict(base, schema=2)]
            if name == 'admission':
                variants += [dict(base, criteria=[]), dict(base, criteria=['']),
                    dict(base, owner_stop_generation=True), dict(base, prompt_sha256='bad'),
                    dict(base, dispatch_uuid='bad'), dict(base, worker_native_uuid='bad')]
            for value in variants:
                with self.subTest(name=name, value=value):
                    d = self.decision(**{name: value})
                    self.assertEqual(d.status, Status.HOLD)
                    self.assertEqual(d.fault, Fault.MALFORMED)

    def test_wrong_review_id_and_digest(self):
        self.complete()
        self.hold(job=replace(self.j, review_id='different-review'))
        self.hold(job=replace(self.j, subject_digest='5' * 64))

    def test_immutable_canonical_identity(self):
        wire = asdict(self.a)
        wire['criteria'] = list(wire['criteria'])
        self.assertEqual(self.a.digest, _sha(_canonical(wire)))
        criteria = list(self.a.criteria)
        copied = replace(self.a, criteria=criteria)
        criteria.append('mutated')
        self.assertEqual(copied.criteria, self.a.criteria)
        with self.assertRaises(AttributeError):
            self.a.task_id = 'changed'


if __name__ == '__main__':
    unittest.main()
