"""Hermetic staging joins using real Git, job, snapshot and review components."""
from dataclasses import asdict
import multiprocessing
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import uuid

from . import review_jobs as jobs
from . import review_snapshot as capture
from .review_completion import Admission, binding_digest
from .review_stage import Bridge, Hold, PROTOCOL
from .session_review import SessionReview, _live_digest


def uid():
    return str(uuid.uuid4())


def run_bridge(root, snapshots, workspace, job, args, point, barrier, out):
    bridge = Bridge(root, capture.Store(snapshots, qualified_local=True),
                    worker_workspace=workspace, qualified_local=True)
    if point:
        def crash(phase):
            if phase == point:
                os._exit(23)
        bridge._checkpoint = crash
    if barrier:
        barrier.wait()
    try:
        bridge.stage(jobs.Store(job), **args)
        if out is not None:
            out.put('staged')
    except Hold:
        if out is not None:
            out.put('held')


class StageTest(unittest.TestCase):
    def setUp(self):
        scratch = Path.home() / '.yggterm/scratchpad/review-stage-tests'
        scratch.mkdir(parents=True, exist_ok=True)
        self.tmp = tempfile.TemporaryDirectory(dir=scratch)
        self.base = Path(self.tmp.name)
        self.repo = self.base / 'workspace'
        self.repo.mkdir(mode=0o700)
        self.git('init', '-q')
        for name in ('src/one.py', 'docs/two.md'):
            p = self.repo / name
            p.parent.mkdir(exist_ok=True)
            p.write_text('invented example\n')
        self.git('add', '.')
        self.git('-c', 'user.name=Example', '-c', 'user.email=example@example.test',
                 'commit', '-qm', 'fixture')
        self.request = dict(schema=1, commit=self.git('rev-parse', 'HEAD'),
                            scope='example-source', paths=['docs/two.md', 'src/one.py'])
        self.snapshot_root = self.base / 'snapshots'
        self.snapshot_root.mkdir(mode=0o700)
        self.snapshots = capture.Store(self.snapshot_root, qualified_local=True)
        artifact = self.snapshots.stage(self.repo, self.request).artifact_id
        self.binding = dict(identity='example-editor', harness='example-native')
        a = Admission(1, 'example-task', uid(), '1' * 64, ('source criterion',),
            'example-worker', uid(), 'example-editor', binding_digest(self.binding),
            '2' * 64, 0)
        self.envelope = dict(schema=1, original_request_sha256='3' * 64,
            parent_ref='example-parent/source', admission=asdict(a),
            admission_digest=a.digest, artifact_sha256=artifact,
            source_scope=self.request['paths'], review_policy_sha256='4' * 64,
            protocol=PROTOCOL)
        self.store = jobs.Store(self.base / 'job')
        self.store.admit(self.envelope)
        state = self.store.change('claim', operation_id=uid(), expected_revision=0,
            generation=0, data=dict(lease_seconds=600)).state
        self.root = self.base / 'bridge'
        self.root.mkdir(mode=0o700)
        self.bridge = self.make_bridge()
        self.args = dict(expected_revision=1, token=state['attempt']['token'],
            generation=0, operation_id=uid(), repo=str(self.repo),
            request=self.request, editor_binding=self.binding,
            config_digest=a.config_digest, tier='full', completion_claim='Source ready')

    def git(self, *args):
        return subprocess.check_output(['git', '-C', str(self.repo), *args],
                                       stderr=subprocess.DEVNULL).decode().strip()

    def make_bridge(self):
        return Bridge(self.root, self.snapshots, worker_workspace=self.repo,
                      qualified_local=True)

    def tearDown(self):
        self.tmp.cleanup()

    def stage(self, **changes):
        return self.bridge.stage(self.store, **dict(self.args, **changes))

    def review(self):
        return SessionReview(self.store.get().state['review_id'], self.root / 'reviews')

    def test_join_dirty_ignored_and_exact_retry(self):
        (self.repo / 'src/one.py').write_text('dirty ignored\n')
        result = self.stage()
        self.assertFalse(result.reconciled)
        self.assertEqual(result.mapping['artifact_id'], self.envelope['artifact_sha256'])
        rows = self.review().rows
        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[0]['acceptance_criteria'], ['source criterion'])
        self.assertEqual(rows[0]['config_digest'], self.args['config_digest'])
        self.assertEqual(rows[0]['worker'], dict(harness='example-worker',
            native_uuid=self.envelope['admission']['worker_native_uuid']))
        self.assertEqual(result.mapping['local_job']['subject_digest'], _live_digest(rows[1]))
        self.assertEqual(result.mapping['original_request_sha256'], '3' * 64)
        self.assertEqual(result.job_state['attempt']['local_subject_digest'],
                         rows[1]['subject_digest'])
        self.assertTrue(self.stage().reconciled)
        self.assertEqual(self.review().rows, rows)

    def test_preflight_mismatches_read_no_source(self):
        variants = [dict(token=uid()), dict(expected_revision=0), dict(generation=1),
            dict(config_digest='9' * 64), dict(editor_binding=dict(identity='other')),
            dict(tier='none'), dict(request=dict(self.request, paths=['src/one.py'])),
            dict(request=dict(self.request, paths=['docs/two.md', 'src/one.py', 'extra']))]
        with patch.object(self.snapshots, 'stage', side_effect=AssertionError('source read')):
            for variant in variants:
                with self.subTest(variant=variant), self.assertRaises(Hold):
                    self.stage(**variant)
        self.assertFalse((self.root / 'intent.json').exists())

    def test_identical_content_new_commit_retains_provenance(self):
        self.git('-c', 'user.name=Example', '-c', 'user.email=example@example.test',
                 'commit', '--allow-empty', '-qm', 'same bytes')
        # Logical artifact excludes commit provenance: identical bytes join.
        self.stage(request=dict(self.request, commit=self.git('rev-parse', 'HEAD')))

    def test_changed_source_artifact_refuses(self):
        (self.repo / 'src/one.py').write_text('changed committed source\n')
        self.git('add', '.')
        self.git('-c', 'user.name=Example', '-c', 'user.email=example@example.test',
                 'commit', '-qm', 'change')
        with self.assertRaises(Hold):
            self.stage(request=dict(self.request, commit=self.git('rev-parse', 'HEAD')))
        self.assertFalse((self.root / 'intent.json').exists())

    def test_stop_at_stage_prevents_job_transition(self):
        def stop(phase):
            if phase == 'after_stage':
                self.store.change('stop', operation_id=uid(), expected_revision=1, generation=0)
        self.bridge._checkpoint = stop
        with self.assertRaises(Hold):
            self.stage()
        self.assertEqual(self.store.get().state['state'], 'suspended')
        self.assertEqual(len(self.review().rows), 2)
        with self.assertRaises(Hold):
            self.stage()

    def test_stop_before_capture_and_wrong_phase(self):
        self.store.change('stop', operation_id=uid(), expected_revision=1, generation=0)
        with patch.object(self.snapshots, 'stage', side_effect=AssertionError('read')):
            with self.assertRaises(Hold):
                self.stage()

    def test_crash_boundaries_exact_retry(self):
        # Each boundary runs on a fresh independent fixture, including real exit.
        for point in ('after_capture', 'after_intent', 'after_open', 'after_stage', 'after_cas'):
            if point != 'after_capture':
                self.tearDown()
                self.setUp()
            ctx = multiprocessing.get_context('fork')
            child = ctx.Process(target=run_bridge, args=(self.root, self.snapshot_root,
                self.repo, self.store.root, self.args, point, None, None))
            child.start()
            child.join(10)
            self.assertEqual(child.exitcode, 23, point)
            result = self.stage()
            self.assertEqual(result.job_state['state'], 'staged')
            self.assertEqual(len(self.review().rows), 2)
            self.assertTrue(self.stage().reconciled)

    def test_two_processes_same_operation(self):
        ctx = multiprocessing.get_context('fork')
        barrier, out = ctx.Barrier(2), ctx.Queue()
        children = [ctx.Process(target=run_bridge, args=(self.root, self.snapshot_root,
            self.repo, self.store.root, self.args, None, barrier, out)) for _ in range(2)]
        for p in children:
            p.start()
        for p in children:
            p.join(10)
            self.assertEqual(p.exitcode, 0)
        results = [out.get(timeout=2) for _ in children]
        self.assertIn('staged', results)
        self.stage()
        self.assertEqual(len(self.review().rows), 2)

    def test_changed_inputs_or_later_chain_retained(self):
        self.stage()
        before = self.review().path.read_bytes()
        for change in (dict(operation_id=uid()), dict(completion_claim='Changed claim'),
                       dict(tier='lightweight')):
            with self.assertRaises(Hold):
                self.stage(**change)
            self.assertEqual(self.review().path.read_bytes(), before)
        self.review().suspend('native review progressed')
        later = self.review().path.read_bytes()
        with self.assertRaises(Hold):
            self.stage()
        self.assertEqual(self.review().path.read_bytes(), later)

    def test_corrupt_chain_retained(self):
        self.stage()
        with self.review().path.open('a') as stream:
            stream.write('{partial')
        before = self.review().path.read_bytes()
        with self.assertRaises(Hold):
            self.stage()
        self.assertEqual(self.review().path.read_bytes(), before)

    def test_changed_criteria_opening_retained(self):
        def fault(phase):
            if phase == 'after_open':
                raise OSError('fault')
        self.bridge._checkpoint = fault
        with self.assertRaises(Hold):
            self.stage()
        review = self.review()
        # Simulate a coherent conflicting opening rather than broken hash bytes.
        row = review.rows[0]
        review.path.unlink()
        SessionReview.create(review_id=review.review_id, base=review.path.parent,
            worker=row['worker'], acceptance_criteria=['different criterion'],
            config_digest=row['config_digest'], editor_binding=row['editor_binding'])
        before = review.path.read_bytes()
        self.bridge._checkpoint = lambda phase: None
        with self.assertRaises(Hold):
            self.stage()
        self.assertEqual(review.path.read_bytes(), before)

    def test_physical_root_relocation_holds(self):
        self.stage()
        moved = self.base / 'moved-bridge'
        self.root.rename(moved)
        self.root = moved
        self.bridge = self.make_bridge()
        with self.assertRaises(Hold):
            self.stage()

    def test_job_commit_uncertainty_reconciles_same_operation(self):
        def fault(phase):
            if phase == 'after_commit':
                raise OSError('receipt lost')
        self.store.fault = fault
        with self.assertRaises(Hold):
            self.stage()
        self.store.fault = None
        self.assertTrue(self.stage().reconciled)
        self.assertEqual(len(self.review().rows), 2)

    def test_wrong_phase_does_not_reopen(self):
        self.stage()
        current = self.store.get().state
        self.store.change('intent', operation_id=uid(), expected_revision=2,
            generation=0, token=current['attempt']['token'],
            data=dict(intent_sha256='5' * 64))
        before = self.review().path.read_bytes()
        with self.assertRaises(Hold):
            self.stage()
        self.assertEqual(self.review().path.read_bytes(), before)

    def test_missing_staged_review_not_recreated(self):
        self.stage()
        path = self.review().path
        path.unlink()
        with self.assertRaises(Hold):
            self.stage()
        self.assertFalse(path.exists())

    def test_changed_materialized_bytes_hold(self):
        result = self.stage()
        path = Path(result.mapping['materialization']) / 'src/one.py'
        path.chmod(0o600)
        path.write_text('corrupt materialization')
        before = self.review().path.read_bytes()
        with self.assertRaises(Hold):
            self.stage()
        self.assertEqual(self.review().path.read_bytes(), before)

    def test_partial_intent_retained(self):
        intent = self.root / 'intent.json'
        intent.write_bytes(b'{partial')
        intent.chmod(0o400)
        with self.assertRaises(Hold):
            self.stage()
        self.assertEqual(intent.read_bytes(), b'{partial')
        self.assertFalse((self.root / 'reviews').exists())

    def test_no_launch_completion_or_signing_surface(self):
        self.assertEqual({name for name in vars(Bridge) if not name.startswith('_')}, {'stage'})


if __name__ == '__main__':
    unittest.main()
