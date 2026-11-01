"""Real child markers must remain absent for every failed entry precondition."""
from dataclasses import asdict
import hashlib
import json
import errno
import os
from pathlib import Path
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

from . import worker_entry as entry


class EntryTests(unittest.TestCase):
    def setUp(self):
        scratch = Path.home() / '.yggterm/scratchpad/worker-entry-tests'
        scratch.mkdir(parents=True, exist_ok=True)
        self.temp = tempfile.TemporaryDirectory(dir=scratch)
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.workspace = self.root / 'workspace'
        self.workspace.mkdir()
        self.evidence = self.root / 'controller'
        self.evidence.mkdir(mode=0o700)
        self.source = self.root / 'memory-gate.py'
        self.marker = self.workspace / 'child-started'
        self.command = [sys.executable, '-c',
            'from pathlib import Path; Path(' + repr(str(self.marker)) + ').write_text("started")']
        self.good = dict(status='ok', harness='example', namespace='-example',
                         conflicts=0, missing_objects=0)

    def specification(self, report=None, *, rc=0, raw=None, before=''):
        output = json.dumps(self.good if report is None else report) if raw is None else raw
        self.source.write_text('import sys, time\n' + before + '\n'
            + 'sys.stdout.write(' + repr(output) + ')\nraise SystemExit(' + str(rc) + ')\n')
        self.source.chmod(0o600)
        return entry.Gate('example', '-example', str(self.source),
                          hashlib.sha256(self.source.read_bytes()).hexdigest())

    def launch(self, gate, **options):
        return entry.guarded_spawn(gate, self.command, cwd=self.workspace,
                                   evidence_root=self.evidence, **options)

    def held(self, gate, **options):
        with self.assertRaises(entry.Held) as caught:
            self.launch(gate, **options)
        self.assertFalse(self.marker.exists(), 'failed gate started a harness child')
        return caught.exception

    def test_success_starts_exact_child_and_retains_private_evidence(self):
        process, receipt = self.launch(self.specification())
        self.assertEqual(process.wait(timeout=3), 0)
        self.assertEqual(self.marker.read_text(), 'started')
        self.assertEqual(receipt.harness, 'example')
        directory = Path(receipt.evidence_directory)
        self.assertEqual(json.loads((directory / 'MEMORY-READY.json').read_text()), asdict(receipt))
        self.assertEqual(json.loads((directory / 'SPAWNED.json').read_text())['native_identity'], 'UNVERIFIED')
        for path in directory.iterdir():
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def test_nonzero_gate_does_not_construct_worker_even_with_success_json(self):
        gate = self.specification(rc=2)
        with patch.object(entry.subprocess, 'Popen', wraps=entry.subprocess.Popen) as popen:
            error = self.held(gate)
        self.assertEqual(error.code, 'memory_gate_failed')
        self.assertEqual(popen.call_count, 1)  # Only the gate process, never the harness.
        self.assertIn('"status": "ok"', (Path(error.receipt) / 'gate.stdout').read_text())
        requested = json.loads((Path(error.receipt) / 'REQUEST.json').read_text())
        self.assertEqual(requested['harness'], 'example')
        self.assertEqual(requested['namespace'], '-example')
        self.assertEqual(requested['qualified_source_sha256'], gate.source_sha256)

    def test_unknown_and_mismatched_results_never_start_worker(self):
        variants = [dict(self.good, status='unknown'), dict(self.good, harness='other'),
                    dict(self.good, namespace='-other'), dict(self.good, conflicts=1),
                    dict(self.good, conflicts=False), dict(self.good, missing_objects=0.0),
                    dict(self.good, schema=2), dict(self.good, schema=True),
                    dict(self.good, schema_version=99), dict(self.good, new_permission=True),
                    [], None, 'invented']
        for value in variants:
            with self.subTest(value=value):
                self.held(self.specification(raw=json.dumps(value)))
        partial = dict(self.good)
        partial.pop('missing_objects')
        self.held(self.specification(partial))

    def test_invalid_and_duplicate_json_do_not_become_a_ready_gate(self):
        for raw in ['', 'not-json', json.dumps(self.good) + '\n' + json.dumps(self.good),
                    '{"status":"ok","harness":"example","namespace":"-example",'
                    '"conflicts":1,"conflicts":0,"missing_objects":0}']:
            with self.subTest(raw=raw):
                self.held(self.specification(raw=raw))

    def test_changed_source_is_held_before_any_gate_or_child_execution(self):
        gate = self.specification()
        self.source.write_text('raise SystemExit(0)\n')
        with patch.object(entry.subprocess, 'Popen', wraps=entry.subprocess.Popen) as popen:
            self.assertEqual(self.held(gate).code, 'gate_source_changed')
        self.assertEqual(popen.call_count, 0)

    def test_source_symlink_and_nonprivate_evidence_are_held(self):
        gate = self.specification()
        actual = self.root / 'real-source.py'
        self.source.rename(actual)
        self.source.symlink_to(actual)
        self.held(gate)
        self.source.unlink()
        actual.rename(self.source)
        self.evidence.chmod(0o777)
        self.held(gate)

    def test_timeout_and_output_overflow_prevent_worker_start(self):
        error = self.held(self.specification(before='time.sleep(2)'), gate_timeout=.05)
        self.assertEqual(error.code, 'memory_gate_timeout')
        error = self.held(self.specification(before='sys.stdout.write("x"*1000000)'),
                          gate_output_cap=128)
        self.assertEqual(error.code, 'memory_gate_output_cap')

    def test_atomic_source_replacement_cannot_change_opened_gate(self):
        started = self.root / 'gate-started'
        before = ('from pathlib import Path\nPath(' + repr(str(started))
                  + ').write_text("ready")\ntime.sleep(.2)')
        gate = self.specification(before=before)
        errors = []
        def replace():
            deadline = time.monotonic() + 2
            while not started.exists() and time.monotonic() < deadline:
                time.sleep(.005)
            if not started.exists():
                errors.append('gate never started')
                return
            replacement = self.root / 'replacement.py'
            replacement.write_text('raise SystemExit(99)\n')
            replacement.replace(self.source)
        thread = threading.Thread(target=replace)
        thread.start()
        try:
            process, receipt = self.launch(gate)
            self.assertEqual(process.wait(timeout=3), 0)
            self.assertEqual(receipt.source_sha256, gate.source_sha256)
        finally:
            thread.join(timeout=3)
        self.assertFalse(errors)
        self.assertTrue(self.marker.exists())
        self.marker.unlink()
        self.assertEqual(self.held(gate).code, 'gate_source_changed')

    def test_evidence_in_worker_workspace_and_unknown_platform_are_held(self):
        gate = self.specification()
        inside = self.workspace / 'evidence'
        inside.mkdir(mode=0o700)
        with self.assertRaises(entry.Held):
            entry.guarded_spawn(gate, self.command, cwd=self.workspace, evidence_root=inside)
        self.assertFalse(self.marker.exists())
        with patch.object(entry.sys, 'platform', 'unqualified'):
            self.assertEqual(self.held(gate).code, 'unsupported_gate_platform')

    def test_gate_excludes_caller_python_import_paths(self):
        poison = self.root / 'unqualified-python'
        poison.mkdir()
        imported = self.root / 'unqualified-import-executed'
        (poison / 'json.py').write_text('from pathlib import Path\nPath('
            + repr(str(imported)) + ').write_text("unqualified")\nraise SystemExit(0)\n')
        gate = self.specification(before='import json')
        environment = dict(os.environ, PYTHONPATH=str(poison))
        process, _ = entry.guarded_spawn(gate, ['/bin/true'], cwd=self.workspace,
            evidence_root=self.evidence, environment=environment)
        self.assertEqual(process.wait(timeout=3), 0)
        self.assertFalse(imported.exists())

    def test_command_and_environment_are_frozen_during_gate(self):
        started = self.root / 'gate-started'
        changed = self.workspace / 'changed-command'
        gate = self.specification(before='from pathlib import Path\nPath('
            + repr(str(started)) + ').write_text("ready")\ntime.sleep(.2)')
        command = [sys.executable, '-I', '-c', 'import os\nfrom pathlib import Path\nPath('
            + repr(str(self.marker)) + ').write_text(os.environ["YGG_ENTRY_FIXTURE"])']
        environment = dict(os.environ, YGG_ENTRY_FIXTURE='original')
        def alter():
            deadline = time.monotonic() + 2
            while not started.exists() and time.monotonic() < deadline:
                time.sleep(.005)
            command[-1] = 'from pathlib import Path;Path(' + repr(str(changed)) + ').touch()'
            environment['YGG_ENTRY_FIXTURE'] = 'changed'
        thread = threading.Thread(target=alter)
        thread.start()
        try:
            process, receipt = entry.guarded_spawn(gate, command, cwd=self.workspace,
                evidence_root=self.evidence, environment=environment)
            self.assertEqual(process.wait(timeout=3), 0)
        finally:
            thread.join(timeout=3)
        self.assertEqual(self.marker.read_text(), 'original')
        self.assertFalse(changed.exists())
        directory = Path(receipt.evidence_directory)
        requested = json.loads((directory / 'REQUEST.json').read_text())
        spawned = json.loads((directory / 'SPAWNED.json').read_text())
        self.assertEqual(requested['command_sha256'], spawned['command_sha256'])

    def test_persistent_storage_failure_before_and_after_spawn_stays_typed(self):
        gate = self.specification()
        with patch.object(entry, '_write', side_effect=OSError(errno.ENOSPC, 'invented full disk')):
            error = self.held(gate)
        self.assertEqual(error.code, 'entry_launch_unresolved')
        self.assertFalse(error.harness_started)
        self.assertTrue(error.no_retry_authorized)
        self.assertFalse(error.diagnostic_persisted)
        real_write = entry._write
        def full_after_spawn(path, data):
            if path.name in ('SPAWNED.json', 'UNRESOLVED.json'):
                raise OSError(errno.ENOSPC, 'invented full disk')
            return real_write(path, data)
        with patch.object(entry, '_write', side_effect=full_after_spawn), self.assertRaises(entry.Held) as held:
            self.launch(gate)
        error = held.exception
        self.assertEqual(error.code, 'entry_launch_unresolved')
        self.assertTrue(error.harness_started)
        self.assertTrue(error.no_retry_authorized)
        self.assertFalse(error.diagnostic_persisted)
        self.assertEqual(error.process.wait(timeout=3), 0)
        self.assertTrue(self.marker.exists())

    def test_failed_gate_diagnostic_failure_never_masks_primary_hold(self):
        gate = self.specification(rc=2)
        original = entry._write
        def unavailable(path, data):
            if path.name == 'HELD.json':
                raise OSError(errno.ENOSPC, 'invented full disk')
            return original(path, data)
        with patch.object(entry, '_write', side_effect=unavailable):
            error = self.held(gate)
        self.assertEqual(error.code, 'memory_gate_failed')
        self.assertFalse(error.harness_started)
        self.assertTrue(error.no_retry_authorized)
        self.assertFalse(error.diagnostic_persisted)

    def test_exited_gate_leader_and_term_resistant_descendants_are_cleaned(self):
        for ignore_term in (False, True):
            with self.subTest(ignore_term=ignore_term):
                child_info = self.root / ('child-' + str(ignore_term) + '.json')
                child = ('import os,json,time,signal\nfrom pathlib import Path\n'
                    + ('signal.signal(signal.SIGTERM,signal.SIG_IGN)\n' if ignore_term else '')
                    + 'raw=Path("/proc/self/stat").read_text()\n'
                    + 'Path(' + repr(str(child_info)) + ').write_text(json.dumps({"pid":os.getpid(),'
                      '"start":raw[raw.rfind(")")+2:].split()[19]}))\ntime.sleep(15)\n')
                before = ('import subprocess\nfrom pathlib import Path\n'
                    + 'subprocess.Popen([' + repr(sys.executable) + ',"-I","-c",' + repr(child) + '])\n'
                    + 'deadline=time.monotonic()+2\n'
                    + 'while not Path(' + repr(str(child_info)) + ').exists() and time.monotonic()<deadline:time.sleep(.005)\n')
                gate = self.specification(before=before)
                info = None
                try:
                    self.assertEqual(self.held(gate, gate_timeout=1.5).code, 'memory_gate_timeout')
                    info = json.loads(child_info.read_text())
                    status = Path('/proc') / str(info['pid']) / 'stat'
                    if status.exists():
                        raw = status.read_text()
                        fields = raw[raw.rfind(')')+2:].split()
                        self.assertTrue(fields[19] != info['start'] or fields[0] in ('Z', 'X', 'x'))
                finally:
                    if info is None and child_info.exists():
                        info = json.loads(child_info.read_text())
                    if info is not None:
                        try:
                            handle = os.pidfd_open(info['pid'])
                            raw = (Path('/proc') / str(info['pid']) / 'stat').read_text()
                            fields = raw[raw.rfind(')')+2:].split()
                            if fields[19] == info['start'] and fields[0] not in ('Z', 'X', 'x'):
                                import signal
                                signal.pidfd_send_signal(handle, signal.SIGKILL)
                            os.close(handle)
                        except OSError:
                            pass

    def test_external_reaper_never_authorizes_numeric_group_signal(self):
        class AlreadyReaped:
            pid = 1234567
            returncode = None
        with patch.object(entry.os, 'waitid', side_effect=ChildProcessError), \
                patch.object(entry.os, 'killpg') as signal_group:
            with self.assertRaises(entry.Held) as caught:
                entry._finish(AlreadyReaped())
        self.assertEqual(caught.exception.code, 'gate_wait_ownership_lost')
        signal_group.assert_not_called()


if __name__ == '__main__':
    unittest.main()
