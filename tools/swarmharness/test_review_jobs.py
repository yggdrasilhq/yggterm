"""Hermetic real-disk falsifiers; no model, queue, auth or network operations."""
from dataclasses import asdict
import json
import multiprocessing
import os
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch
import uuid

from .review_completion import Admission
from .review_jobs import Hold, Store, digest


def uid():
    return str(uuid.uuid4())


def envelope():
    a = Admission(1, 'example-task', uid(), '1' * 64, ('source only',),
                  'example-worker', uid(), 'example-editor', '2' * 64,
                  '3' * 64, 0)
    return dict(schema=1, original_request_sha256='4' * 64,
                parent_ref='example-parent/source', admission=asdict(a),
                admission_digest=a.digest, artifact_sha256='5' * 64,
                source_scope=['src/example.py'], review_policy_sha256='6' * 64,
                protocol='example-review-v1')


def claim_process(root, barrier, out):
    barrier.wait()
    try:
        Store(root).change('claim', operation_id=uid(), expected_revision=0,
                           generation=0, data={'lease_seconds': 5})
        out.put('claimed')
    except Hold:
        out.put('held')


def crash_process(root, action, args, point):
    def crash(at):
        if at == point:
            os._exit(23)
    Store(root, fault=crash).change(action, **args)


def ordered_process(root, action, args, wait, signal, out):
    if wait is not None:
        if not wait.wait(5):
            os._exit(24)
    try:
        Store(root).change(action, **args)
        out.put((action, 'committed'))
    except Hold:
        out.put((action, 'held'))
    finally:
        if signal is not None:
            signal.set()


class JobsTest(unittest.TestCase):
    def setUp(self):
        # Explicit disk-backed scratch, never the platform /tmp default.
        base = Path.home() / '.yggterm/scratchpad/review-jobs-tests'
        base.mkdir(parents=True, exist_ok=True)
        self.tmp = tempfile.TemporaryDirectory(dir=base)
        self.root = Path(self.tmp.name) / 'job'
        self.now = 100
        self.store = Store(self.root, clock=lambda: self.now)
        self.e = envelope()
        self.s = self.store.admit(self.e).state

    def tearDown(self):
        self.tmp.cleanup()

    def change(self, action, data=None, **kw):
        args = dict(operation_id=uid(), expected_revision=self.s['revision'],
                    generation=self.s['generation'], data=data)
        if action not in ('stop', 'resume', 'claim', 'recover'):
            args['token'] = self.s['attempt']['token']
        args.update(kw)
        receipt = self.store.change(action, **args)
        self.s = receipt.state
        return receipt

    def intent(self):
        self.change('claim', {'lease_seconds': 1})
        self.change('stage', {'local_subject_digest': '7' * 64})
        self.change('intent', {'intent_sha256': '8' * 64})

    def tree_bytes(self):
        return {p.name: p.read_bytes() for p in self.root.iterdir()}

    def test_competing_process_claims(self):
        ctx = multiprocessing.get_context('fork')
        barrier, out = ctx.Barrier(2), ctx.Queue()
        ps = [ctx.Process(target=claim_process, args=(str(self.root), barrier, out))
              for _ in range(2)]
        for p in ps:
            p.start()
        for p in ps:
            p.join(10)
            self.assertEqual(p.exitcode, 0)
        self.assertEqual(sorted([out.get(timeout=2), out.get(timeout=2)]),
                         ['claimed', 'held'])
        self.assertEqual(self.store.get().state['revision'], 1)

    def test_explicit_recovery_fences_old_worker(self):
        self.change('claim', {'lease_seconds': 1})
        old = self.s['attempt'].copy()
        with self.assertRaises(Hold):
            self.change('recover', {'lease_seconds': 1})
        self.now += 2
        self.change('recover', {'lease_seconds': 1})
        self.assertNotEqual(old['attempt_id'], self.s['attempt']['attempt_id'])
        with self.assertRaises(Hold):
            self.change('stage', {'local_subject_digest': '7' * 64}, token=old['token'])
        with self.assertRaises(Hold):
            self.change('stage', {'local_subject_digest': '7' * 64}, expected_revision=1)
        self.assertEqual(self.store.get().state['revision'], 2)

    def test_intent_expiry_never_reclaims(self):
        self.intent()
        self.now += 100
        with self.assertRaises(Hold):
            self.change('recover', {'lease_seconds': 1})
        self.change('suspend')
        self.assertTrue(self.s['unresolved'])

    def test_stop_before_authorize(self):
        self.intent()
        old = self.s.copy()
        self.change('stop')
        with self.assertRaises(Hold):
            self.store.change('authorize', operation_id=uid(), expected_revision=old['revision'],
                              generation=0, token=old['attempt']['token'])
        with self.assertRaises(Hold):
            self.change('authorize')
        self.assertTrue(self.s['unresolved'])
        self.assertEqual(Store(self.root).get().state['generation'], 1)

    def test_authorize_before_stop(self):
        self.intent()
        self.change('authorize')
        self.change('stop')
        self.assertTrue(self.s['unresolved'])
        with self.assertRaises(Hold):
            self.change('record_start', {'receipt_sha256': '9' * 64})
        with self.assertRaises(Hold):
            self.change('resume')
        with self.assertRaises(Hold):
            self.change('claim', {'lease_seconds': 1})

    def test_restart_and_start_receipt(self):
        self.intent()
        self.change('authorize')
        self.store = Store(self.root)
        self.assertEqual(self.store.get().state, self.s)
        self.change('record_start', {'receipt_sha256': '9' * 64})
        self.assertEqual(Store(self.root).get().state['state'], 'invocation-started')

    def test_preintent_stop_explicit_resume(self):
        self.change('claim', {'lease_seconds': 1})
        old = self.s['attempt']['token']
        self.change('stop')
        self.change('resume')
        self.change('claim', {'lease_seconds': 1})
        with self.assertRaises(Hold):
            self.change('stage', {'local_subject_digest': '7' * 64}, token=old)
        with self.assertRaises(Hold):
            self.change('stage', {'local_subject_digest': '7' * 64}, generation=0)

    def test_crash_before_commit_rolls_back(self):
        def fault(point):
            if point == 'before_commit':
                raise RuntimeError('simulated crash')
        self.store.fault = fault
        with self.assertRaises(Hold):
            self.change('claim', {'lease_seconds': 1})
        self.assertEqual(Store(self.root).get().state, self.s)

    def test_lost_receipt_reconciles_exact_operation(self):
        self.intent()
        op = uid()
        args = dict(operation_id=op, expected_revision=self.s['revision'],
                    generation=0, token=self.s['attempt']['token'])
        def fault(point):
            if point == 'after_commit':
                raise RuntimeError('lost receipt')
        self.store.fault = fault
        with self.assertRaises(Hold):
            self.store.change('authorize', **args)
        restarted = Store(self.root)
        receipt = restarted.change('authorize', **args)
        self.assertTrue(receipt.reconciled)
        self.assertEqual(receipt.state['state'], 'submission-authorized')
        with self.assertRaises(Hold):
            restarted.change('intent', **args)
        self.s = receipt.state
        self.store = restarted
        self.change('stop')
        self.assertTrue(restarted.change('authorize', **args).state['stopped'])

    def test_actual_process_death_at_intent_commit(self):
        self.change('claim', {'lease_seconds': 1})
        self.change('stage', {'local_subject_digest': '7' * 64})
        args = dict(operation_id=uid(), expected_revision=self.s['revision'],
                    generation=0, token=self.s['attempt']['token'],
                    data={'intent_sha256': '8' * 64})
        ctx = multiprocessing.get_context('fork')
        p = ctx.Process(target=crash_process,
                        args=(str(self.root), 'intent', args, 'before_commit'))
        p.start()
        p.join(10)
        self.assertEqual(p.exitcode, 23)
        before = self.tree_bytes()
        with self.assertRaises(Hold):
            Store(self.root).get()
        self.assertEqual(before, self.tree_bytes())
        # Orphan rollback journal is intentionally retained, not auto-repaired.
        # Exercise the committed crash in a separate clean admitted store.
        self.root = Path(self.tmp.name) / 'committed'
        self.store = Store(self.root)
        self.s = self.store.admit(self.e).state
        self.change('claim', {'lease_seconds': 1})
        self.change('stage', {'local_subject_digest': '7' * 64})
        args['token'] = self.s['attempt']['token']
        p = ctx.Process(target=crash_process,
                        args=(str(self.root), 'intent', args, 'after_commit'))
        p.start()
        p.join(10)
        self.assertEqual(p.exitcode, 23)
        restarted = Store(self.root, clock=lambda: 1000)
        result = restarted.change('intent', **args)
        self.assertTrue(result.reconciled)
        self.assertEqual(result.state['state'], 'invocation-intent')
        with self.assertRaises(Hold):
            restarted.change('recover', operation_id=uid(), expected_revision=3,
                             generation=0, data={'lease_seconds': 1})

    def test_cross_process_stop_authorize_fence_both_orders(self):
        self.intent()
        ctx = multiprocessing.get_context('fork')
        initial = self.s
        for first in ('stop', 'authorize'):
            root = Path(self.tmp.name) / first
            target = Store(root)
            target.admit(self.e)
            # Independent replay of ordinary setup, preserving genuine tokens.
            state = target.change('claim', operation_id=uid(), expected_revision=0,
                                  generation=0, data={'lease_seconds': 1}).state
            for rev, action, data in [(1, 'stage', {'local_subject_digest': '7' * 64}),
                                      (2, 'intent', {'intent_sha256': '8' * 64})]:
                state = target.change(action, operation_id=uid(), expected_revision=rev,
                                      generation=0, token=state['attempt']['token'], data=data).state
            second = 'authorize' if first == 'stop' else 'stop'
            done, out = ctx.Event(), ctx.Queue()
            def args_for(action, revision):
                return dict(operation_id=uid(), expected_revision=revision,
                            generation=0, token=state['attempt']['token'] if action == 'authorize' else None)
            # Stop second uses the post-authorization revision. Authorization
            # second deliberately uses the pre-stop generation and revision.
            ps = [ctx.Process(target=ordered_process,
                    args=(str(root), first, args_for(first, 3), None, done, out)),
                  ctx.Process(target=ordered_process,
                    args=(str(root), second, args_for(second, 4 if second == 'stop' else 3), done, None, out))]
            for p in reversed(ps):
                p.start()
            for p in ps:
                p.join(10)
                self.assertEqual(p.exitcode, 0)
            results = dict([out.get(timeout=2), out.get(timeout=2)])
            self.assertEqual(results['stop'], 'committed')
            self.assertEqual(results['authorize'], 'held' if first == 'stop' else 'committed')
            final = target.get().state
            self.assertTrue(final['stopped'])
            self.assertTrue(final['unresolved'])
        self.assertEqual(self.store.get().state, initial)

    def test_identity_immutable_and_new_dispatch_separate(self):
        self.assertTrue(self.store.admit(self.e).reconciled)
        changed = json.loads(json.dumps(self.e))
        changed['admission']['dispatch_uuid'] = uid()
        changed['admission_digest'] = digest(changed['admission'])
        before = self.tree_bytes()
        with self.assertRaises(Hold):
            self.store.admit(changed)
        self.assertEqual(before, self.tree_bytes())
        other = Store(Path(self.tmp.name) / 'other').admit(changed).state
        self.assertNotEqual(self.s['job_id'], other['job_id'])

    def test_no_completion_or_unknown_data_endpoint(self):
        self.change('claim', {'lease_seconds': 1})
        for action in ('accepted-subject', 'task-applied', 'review-pending', 'invented'):
            with self.assertRaises(Hold):
                self.change(action)
        with self.assertRaises(Hold):
            self.change('stage', {'local_subject_digest': '7' * 64, 'approved': True})

    def test_envelope_closed_schema_and_scope(self):
        for key, value in [('schema', 2), ('source_scope', ['../private']),
                           ('artifact_sha256', 'bad'), ('extra', True)]:
            e = dict(self.e)
            e[key] = value
            with self.assertRaises(Hold):
                self.store.admit(e)

    def test_unknown_empty_store_not_initialized(self):
        root = Path(self.tmp.name) / 'empty'
        root.mkdir(mode=0o700)
        with self.assertRaises(Hold):
            Store(root).admit(self.e)
        self.assertEqual(list(root.iterdir()), [])

    def test_missing_schema_not_empty(self):
        (self.root / 'store.sqlite').unlink()
        with self.assertRaises(Hold):
            self.store.admit(self.e)
        self.assertFalse((self.root / 'store.sqlite').exists())

    def test_corrupt_partial_data_retained(self):
        for payload in (b'SQLite format 3\x00partial', b'garbage', b''):
            (self.root / 'store.sqlite').write_bytes(payload)
            with self.assertRaises(Hold):
                self.store.get()
            self.assertEqual((self.root / 'store.sqlite').read_bytes(), payload)

    def test_future_schema_retained(self):
        db = sqlite3.connect(self.root / 'store.sqlite')
        db.execute('CREATE TABLE future(value TEXT)')
        db.commit()
        db.close()
        before = self.tree_bytes()
        with self.assertRaises(Hold):
            self.store.admit(self.e)
        self.assertEqual(before, self.tree_bytes())

    def test_partial_orphan_journal_retained(self):
        journal = self.root / 'store.sqlite-journal'
        journal.write_bytes(b'partial journal')
        journal.chmod(0o600)
        before = self.tree_bytes()
        with self.assertRaises(Hold):
            self.store.get()
        self.assertEqual(before, self.tree_bytes())

    def test_event_tamper_and_future_genesis_hold(self):
        db = sqlite3.connect(self.root / 'store.sqlite')
        payload = json.dumps({'schema': 2, 'envelope': self.e}, sort_keys=True,
                             separators=(',', ':'))
        db.execute('UPDATE records SET payload=?,digest=? WHERE n=0',
                   (payload, digest(['0' * 64, json.loads(payload)])))
        db.commit()
        db.close()
        before = self.tree_bytes()
        with self.assertRaises(Hold):
            self.store.get()
        self.assertEqual(before, self.tree_bytes())

    def test_event_hash_tamper_preserved(self):
        self.intent()
        db = sqlite3.connect(self.root / 'store.sqlite')
        db.execute("UPDATE records SET digest=? WHERE n=2", ('0' * 64,))
        db.commit()
        db.close()
        before = self.tree_bytes()
        with self.assertRaises(Hold):
            self.store.get()
        self.assertEqual(before, self.tree_bytes())

    def test_unsafe_storage_symlink_and_modes(self):
        path = self.root / 'store.sqlite'
        target = Path(self.tmp.name) / 'target'
        path.rename(target)
        path.symlink_to(target)
        before = target.read_bytes()
        with self.assertRaises(Hold):
            self.store.get()
        self.assertEqual(before, target.read_bytes())
        path.unlink()
        target.rename(path)
        path.chmod(0o644)
        with self.assertRaises(Hold):
            self.store.get()

    def test_root_alias_resolves_once(self):
        alias = Path(self.tmp.name) / 'alias'
        alias.symlink_to(self.root, target_is_directory=True)
        store = Store(alias)
        alias.unlink()
        alias.symlink_to(Path(self.tmp.name) / 'elsewhere')
        self.assertEqual(store.get().state, self.s)

    def test_unsupported_platform(self):
        with patch('tools.swarmharness.review_jobs.sys.platform', 'win32'):
            with self.assertRaises(Hold):
                Store(self.root)


if __name__ == '__main__':
    unittest.main()
