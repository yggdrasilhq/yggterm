"""Real local marker/stop/crash controls; no model, credential or queue calls."""
from pathlib import Path
import fcntl
import json
import multiprocessing
import os
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch
import uuid

from . import review_jobs as jobs
from . import review_invocation as invocation
from .test_review_jobs import envelope
from .review_stage import PROTOCOL


def uid():
    return str(uuid.uuid4())


def crash_launch(root, workspace, job, options, phase):
    broker = invocation.Broker(root, worker_workspace=workspace, qualified_local=True)
    def crash(current):
        if current == phase:
            os._exit(23)
    broker._checkpoint = crash
    broker.launch(jobs.Store(job), **options, authorize=lambda plan, state: True)


def competing_launch(root, workspace, job, options, barrier, output):
    broker=invocation.Broker(root, worker_workspace=workspace, qualified_local=True)
    barrier.wait()
    try:
        receipt=broker.launch(jobs.Store(job), **options, authorize=lambda plan,state: True)
        receipt.process.wait(timeout=3)
        output.put('launched')
    except invocation.Hold:
        output.put('held')


class InvocationTests(unittest.TestCase):
    def setUp(self):
        scratch = Path.home()/'.yggterm/scratchpad/review-invocation-tests'
        scratch.mkdir(parents=True, exist_ok=True)
        self.tmp = tempfile.TemporaryDirectory(dir=scratch)
        self.base = Path(self.tmp.name)
        self.workspace = self.base/'workspace'
        self.workspace.mkdir(mode=0o700)
        self.root = self.base/'invocation'
        self.root.mkdir(mode=0o700)
        self.source = self.base/'stdin'
        self.source.write_bytes(b'invented review input')
        self.source.chmod(0o400)
        self.marker = self.base/'marker'
        self.store = jobs.Store(self.base/'job')
        admitted = envelope()
        admitted['protocol'] = PROTOCOL
        self.store.admit(admitted)
        state = self.store.change('claim', operation_id=uid(), expected_revision=0,
            generation=0, data=dict(lease_seconds=30)).state
        self.store.change('stage', operation_id=uid(), expected_revision=1,
            token=state['attempt']['token'], generation=0, data=dict(local_subject_digest='7'*64))
        self.options = dict(expected_revision=2, token=state['attempt']['token'], generation=0,
            operation_ids=dict(intent=uid(), authorize=uid(), start=uid()),
            command=[sys.executable, '-I', '-c',
                'from pathlib import Path;import sys;'
                'p=Path(sys.argv[1]);p.open("ab").write(b"one\\n");'
                'print(sys.stdin.buffer.read().decode())', str(self.marker)],
            environment=dict(PATH='/usr/bin:/bin'), cwd=str(self.workspace), stdin_path=str(self.source))
        self.broker = invocation.Broker(self.root, worker_workspace=self.workspace, qualified_local=True)

    def tearDown(self):
        self.tmp.cleanup()

    def launch(self, **changes):
        values=dict(self.options,authorize=lambda plan,state: True)
        values.update(changes)
        return self.broker.launch(self.store, **values)

    def stop(self):
        state=self.store.get().state
        return self.store.change('stop', operation_id=uid(), expected_revision=state['revision'],
                                 generation=state['generation']).state

    def test_authorization_and_no_execution_under_either_fence(self):
        phases=[]
        def checkpoint(phase):
            phases.append(phase)
            if phase in ('before_authorize','after_authorize','before_release'):
                self.assertFalse(self.marker.exists())
            if phase == 'before_release':
                with (self.root/'.lock').open('r+') as lock:
                    fcntl.flock(lock, fcntl.LOCK_EX|fcntl.LOCK_NB)
                self.assertEqual(self.store.get().state['state'], 'submission-authorized')
        self.broker._checkpoint=checkpoint
        result=self.launch()
        self.assertEqual(result.process.wait(timeout=3),0)
        self.assertEqual(self.marker.read_bytes(),b'one\n')
        self.assertIn(b'invented review input',(self.root/'stdout').read_bytes())
        self.assertEqual(result.job_state['state'],'invocation-started')
        self.assertEqual(result.native_session_identity,'UNVERIFIED')
        self.assertEqual(result.task_acceptance,'PENDING')
        self.assertEqual(phases[-1],'after_start')

    def test_missing_negative_unknown_verifier_never_spawns(self):
        for verifier in [None,lambda p,s: False,lambda p,s: {},lambda p,s: 1]:
            with self.subTest(verifier=verifier), patch.object(subprocess,'Popen') as spawn:
                with self.assertRaises(invocation.Hold):
                    self.launch(authorize=verifier)
                spawn.assert_not_called()
        self.assertEqual(self.store.get().state['state'],'staged')

    def test_second_verification_failure_cleans_blocked_broker(self):
        calls=[]
        def verifier(plan,state):
            calls.append(state['state'])
            return len(calls)==1
        with self.assertRaises(invocation.Hold) as held:
            self.launch(authorize=verifier)
        self.assertEqual(held.exception.native_effect,'NOT_RELEASED')
        self.assertEqual(held.exception.process.returncode,125)
        self.assertFalse(self.marker.exists())
        self.assertEqual(calls,['staged','invocation-intent'])

    def test_stop_wins_before_authorization_zero_program_executions(self):
        def checkpoint(phase):
            if phase=='before_authorize': self.stop()
        self.broker._checkpoint=checkpoint
        with self.assertRaises(invocation.Hold) as held: self.launch()
        self.assertEqual(held.exception.native_effect,'NOT_RELEASED')
        self.assertEqual(held.exception.process.returncode,125)
        self.assertFalse(self.marker.exists())
        self.assertTrue(self.store.get().state['stopped'])

    def test_authorization_wins_later_stop_cannot_apply_or_retry(self):
        def checkpoint(phase):
            if phase=='after_authorize': self.stop()
        self.broker._checkpoint=checkpoint
        with self.assertRaises(invocation.Hold) as held: self.launch()
        self.assertEqual(held.exception.native_effect,'UNKNOWN')
        held.exception.process.wait(timeout=3)
        self.assertEqual(self.marker.read_bytes(),b'one\n')
        with self.assertRaises(invocation.Hold): self.launch()
        self.assertEqual(self.marker.read_bytes(),b'one\n')

    def test_exact_retry_never_duplicates(self):
        result=self.launch();result.process.wait(timeout=3)
        with self.assertRaises(invocation.Hold):self.launch()
        self.assertEqual(self.marker.read_bytes(),b'one\n')

    def test_two_processes_cannot_duplicate_a_marker(self):
        ctx=multiprocessing.get_context('fork')
        barrier,output=ctx.Barrier(2),ctx.Queue()
        workers=[ctx.Process(target=competing_launch,args=(self.root,self.workspace,
            self.store.root,self.options,barrier,output)) for _ in range(2)]
        for worker in workers:worker.start()
        for worker in workers:
            worker.join(6)
            self.assertEqual(worker.exitcode,0)
        self.assertEqual(sorted(output.get(timeout=2) for _ in workers),['held','launched'])
        self.assertEqual(self.marker.read_bytes(),b'one\n')

    def test_same_job_and_operations_in_two_roots_require_reconciliation(self):
        second_root=self.base/'second-invocation';second_root.mkdir(mode=0o700)
        second=invocation.Broker(second_root,worker_workspace=self.workspace,qualified_local=True)
        both=threading.Barrier(2,timeout=3)
        first_ready,first_intent,second_done=threading.Event(),threading.Event(),threading.Event()
        outcomes={}
        def verifier(name):
            def verify(plan,state):
                if state['state']=='staged':
                    if name=='first':first_ready.set()
                    both.wait()
                return True
            return verify
        def first_checkpoint(phase):
            if phase=='after_intent':
                first_intent.set()
                self.assertTrue(second_done.wait(3))
        def second_checkpoint(phase):
            if phase=='before_intent':self.assertTrue(first_intent.wait(3))
        self.broker._checkpoint=first_checkpoint
        second._checkpoint=second_checkpoint
        def run(name,broker):
            try:
                result=broker.launch(self.store,**self.options,authorize=verifier(name))
                outcomes[name]=('receipt',result.process.wait(timeout=3))
            except invocation.Hold as held:
                outcomes[name]=('held',held.code,held.process is None)
            finally:
                if name=='second':second_done.set()
        workers=[threading.Thread(target=run,args=('first',self.broker)),
                 threading.Thread(target=run,args=('second',second))]
        workers[0].start()
        self.assertTrue(first_ready.wait(3))
        workers[1].start()
        for worker in workers:worker.join(6)
        self.assertFalse(any(worker.is_alive() for worker in workers))
        self.assertEqual(outcomes['first'],('receipt',0))
        self.assertEqual(outcomes['second'],('held','invocation_intent_requires_reconciliation',True))
        self.assertEqual(self.marker.read_bytes(),b'one\n')

    def test_reconciled_authorization_never_releases_broker(self):
        original=self.store.change
        def reconcile(action,**kwargs):
            result=original(action,**kwargs)
            return jobs.Receipt(result.state,True) if action=='authorize' else result
        with patch.object(self.store,'change',side_effect=reconcile):
            with self.assertRaises(invocation.Hold) as held:self.launch()
        self.assertEqual(held.exception.code,'submission_authorization_requires_reconciliation')
        self.assertEqual(held.exception.process.returncode,125)
        self.assertFalse(self.marker.exists())

    def test_loader_environment_has_no_effect_before_authorization(self):
        loader_marker=self.base/'loader-marker'
        source=self.base/'loader.c';library=self.base/'loader.so'
        source.write_text('#include <fcntl.h>\n#include <unistd.h>\n'
            '__attribute__((constructor)) static void mark(void) {'
            'int f=open('+json.dumps(str(loader_marker))+',O_WRONLY|O_CREAT|O_APPEND,0600);'
            'if(f>=0){write(f,"effect\\n",7);close(f);}}\n')
        subprocess.run(['/usr/bin/cc','-shared','-fPIC','-o',str(library),str(source)],
            check=True,timeout=10,env=dict(PATH='/usr/bin:/bin',TMPDIR=str(self.base)))
        def verify(plan,state):
            if state['state']=='staged':return True
            time.sleep(.15)  # Let the real blocked helper finish its safe startup.
            self.assertFalse(loader_marker.exists())
            return False
        with self.assertRaises(invocation.Hold) as held:
            self.launch(environment=dict(PATH='/usr/bin:/bin',LD_PRELOAD=str(library)),authorize=verify)
        self.assertEqual(held.exception.native_effect,'NOT_RELEASED')
        self.assertFalse(loader_marker.exists())
        self.assertFalse(self.marker.exists())
        # A fresh independent admitted fixture proves the requested environment
        # is actually applied after grant, rather than silently discarded.
        case=InvocationTests('test_exact_retry_never_duplicates');case.setUp()
        try:
            result=case.launch(environment=dict(PATH='/usr/bin:/bin',LD_PRELOAD=str(library)))
            self.assertEqual(result.process.wait(timeout=3),0)
            self.assertEqual(loader_marker.read_bytes(),b'effect\n')
        finally:case.tearDown()

    def test_requested_environment_values_are_not_persisted_in_metadata(self):
        result=self.launch(environment=dict(PATH='/usr/bin:/bin',EXAMPLE_VALUE='invented-ephemeral-value'))
        result.process.wait(timeout=3)
        for name in ['intent.json','broker.json']:
            self.assertNotIn(b'invented-ephemeral-value',(self.root/name).read_bytes())

    def test_unknown_storage_is_retained_and_never_used(self):
        unexpected=self.root/'future.json'
        unexpected.write_bytes(b'invented future format')
        with self.assertRaises(invocation.Hold):self.launch()
        self.assertEqual(unexpected.read_bytes(),b'invented future format')
        self.assertFalse(self.marker.exists())

    def test_controller_container_mutation_does_not_change_execution(self):
        calls=[]
        def verifier(plan,state):
            calls.append(plan)
            self.options['command'][:]=['/usr/bin/false']
            self.options['environment']['CHANGED']='later'
            return True
        result=self.launch(authorize=verifier)
        self.assertEqual(result.process.wait(timeout=3),0)
        self.assertEqual(self.marker.read_bytes(),b'one\n')
        self.assertEqual(calls[0],calls[1])

    def test_stale_claim_and_unsafe_input_start_no_program(self):
        for changes in [dict(expected_revision=1),dict(expected_revision=True),dict(token=uid()),
                        dict(generation=1),dict(generation=False)]:
            with self.subTest(changes=changes),self.assertRaises(invocation.Hold):self.launch(**changes)
        self.source.chmod(0o600)
        with self.assertRaises(invocation.Hold):self.launch()
        self.assertFalse(self.marker.exists())

    def test_persistent_receipt_failure_remains_typed_before_release(self):
        original=invocation.storage._write
        def fail(path,*args,**kwargs):
            if path.name=='broker.json':raise OSError('invented persistent disk fault')
            return original(path,*args,**kwargs)
        with patch.object(invocation.storage,'_write',side_effect=fail):
            with self.assertRaises(invocation.Hold) as held:self.launch()
        self.assertEqual(held.exception.native_effect,'NOT_RELEASED')
        self.assertEqual(held.exception.process.returncode,125)
        self.assertTrue(held.exception.no_retry_authorized)
        self.assertFalse(self.marker.exists())

    def test_postrelease_store_failure_retains_process_and_no_retry(self):
        original=self.store.change
        def fail(action,**kwargs):
            if action=='record_start':raise jobs.Hold('invented storage failure')
            return original(action,**kwargs)
        with patch.object(self.store,'change',side_effect=fail):
            with self.assertRaises(invocation.Hold) as held:self.launch()
        self.assertEqual(held.exception.native_effect,'UNKNOWN')
        self.assertTrue(held.exception.no_retry_authorized)
        held.exception.process.wait(timeout=3)
        with self.assertRaises(invocation.Hold):self.launch()
        self.assertEqual(self.marker.read_bytes(),b'one\n')

    def test_crash_boundaries_retain_history_and_never_duplicate(self):
        context=multiprocessing.get_context('fork')
        # Each case owns a separate real job/root and its own marker child.
        for phase in ['before_intent','after_intent','after_authorize','after_release','after_start']:
            case=InvocationTests('test_exact_retry_never_duplicates');case.setUp()
            try:
                worker=context.Process(target=crash_launch,args=(case.root,case.workspace,
                    case.store.root,case.options,phase))
                worker.start();worker.join(5)
                self.assertEqual(worker.exitcode,23,phase)
                if phase in ['before_intent','after_intent','after_authorize']:
                    time.sleep(.08)
                    self.assertFalse(case.marker.exists(),phase)
                else:
                    deadline=time.monotonic()+3
                    while not case.marker.exists() and time.monotonic()<deadline:time.sleep(.01)
                    self.assertEqual(case.marker.read_bytes(),b'one\n',phase)
                with case.assertRaises(invocation.Hold):case.launch()
            finally:case.tearDown()


if __name__=='__main__':unittest.main()
