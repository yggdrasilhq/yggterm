"""Local Linux start barrier for an already admitted/staged review attempt.

This is a mechanism, not an execution policy. A trusted controller must supply
a bounded LOCAL authorization verifier binding the entire plan and current job
state, including policy/account/quota/budget/privacy/memory preconditions.
Missing/unknown verification holds. No worker JSON may supply that verifier.
It cannot call models or network. No credential values are persisted here.

Qualified local disk, trusted interpreter/dependencies, cooperative same-UID
root lifecycle and exclusive child waiting are required. This is not worker
confinement, native-session attestation, verdict intake or task completion.
The caller owns deadline/output supervision and qualified cancellation AFTER
release. Output can grow after return; do not deploy without that supervisor.

One root belongs to one immutable invocation. Every retry after a plan exists
HOLDs for reconciliation, even if no process-start receipt was written. There
is no automated replay/recovery/succession. Process handles on post-release
holds are retained; absence of a receipt NEVER authorizes another launch.
"""
from dataclasses import dataclass
import fcntl
import json
import os
from pathlib import Path
import signal
import stat
import subprocess
import sys

from . import review_jobs as jobs
from . import review_snapshot as storage
from .review_stage import PROTOCOL


class Hold(Exception):
    def __init__(self, code, process=None, released=False):
        self.code = code
        self.process = process
        self.native_effect = 'UNKNOWN' if released else 'NOT_RELEASED'
        self.no_retry_authorized = True
        super().__init__(code)


@dataclass(frozen=True)
class Receipt:
    process: subprocess.Popen
    invocation_digest: str
    job_state: dict
    evidence_directory: str
    native_session_identity: str = 'UNVERIFIED'
    task_acceptance: str = 'PENDING'


# No caller-controlled program executes before exact one-byte grant + EOF.
# Parent death closes its writer; EOF alone exits without executing anything.
_BARRIER = '''import os,sys,json
fd=int(sys.argv[1])
grant=b''
while True:
 part=os.read(fd,2)
 if not part:break
 grant+=part
 if len(grant)>1:os._exit(125)
os.close(fd)
if grant!=b'G':os._exit(125)
environment_fd=int(sys.argv[2])
environment=json.loads(os.read(environment_fd,262145))
os.close(environment_fd)
os.execvpe(sys.argv[3],sys.argv[3:],environment)
'''


class Broker:
    def __init__(self, root, *, worker_workspace, qualified_local=False):
        if (sys.platform != 'linux' or not hasattr(os, 'pidfd_open')
                or not hasattr(os, 'memfd_create') or not hasattr(os, 'MFD_ALLOW_SEALING')
                or not all(hasattr(fcntl,n) for n in ('F_ADD_SEALS','F_SEAL_SEAL','F_SEAL_GROW','F_SEAL_SHRINK','F_SEAL_WRITE'))
                or not hasattr(signal, 'pidfd_send_signal')):
            raise Hold('unsupported_process_identity')
        self.storage = storage.Store(root, qualified_local=qualified_local)
        self.workspace = Path(worker_workspace).resolve(strict=True)
        if self.storage.root == self.workspace or self.workspace in self.storage.root.parents:
            raise Hold('invocation_storage_inside_worker_workspace')

    def _checkpoint(self, phase):
        """Deterministic local crash/ordering controls; production does nothing."""

    def launch(self, store, *, expected_revision, token, generation, operation_ids,
               command, environment, cwd, stdin_path, authorize):
        """operation_ids has immutable intent/authorize/start UUIDs.

        authorize(plan, current_state) must return the boolean True, before any
        broker and again immediately before store authorization. It is a local
        trusted adapter, not policy implemented here. Command/env are frozen;
        no shell, model prompt or credentials are put in metadata. stdin must
        be an owned private regular immutable input artifact <=1 MiB outside
        the worker workspace. The opened FD pins ordinary atomic replacement.
        """
        process = None
        released = False
        read_fd = write_fd = pidfd = input_fd = environment_fd = None
        try:
            if (not callable(authorize) or type(command) not in (list, tuple)
                    or not command or any(type(x) is not str or '\0' in x for x in command)
                    or not Path(command[0]).is_absolute() or type(environment) is not dict
                    or any(type(k) is not str or type(v) is not str or '\0' in k+v
                           for k, v in environment.items())
                    or sum(len(k)+len(v) for k,v in environment.items()) > 256*1024
                    or type(expected_revision) is not int or type(generation) is not int):
                raise Hold('invalid_launch_input')
            command, environment = tuple(command), dict(environment)
            environment_bytes=jobs._bytes(environment)
            if len(environment_bytes)>256*1024:
                raise Hold('environment_bound')
            ids = json.loads(jobs._bytes(operation_ids))
            jobs._keys(ids, 'intent authorize start')
            for value in ids.values():
                jobs._uuid(value)
            if len(set(ids.values())) != 3:
                raise Hold('operation_identity_reuse')
            directory = Path(cwd).resolve(strict=True)
            info = directory.stat()
            if not stat.S_ISDIR(info.st_mode):
                raise Hold('invalid_working_directory')
            source = Path(stdin_path).resolve(strict=True)
            if source == self.workspace or self.workspace in source.parents:
                raise Hold('input_inside_worker_workspace')
            payload, mode = storage._read(source, 1024 * 1024)
            if mode != 0o400:
                raise Hold('input_not_immutable')
            input_fd = os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
            opened = os.fstat(input_fd)
            if (not stat.S_ISREG(opened.st_mode) or opened.st_uid != os.getuid()
                    or opened.st_nlink != 1 or stat.S_IMODE(opened.st_mode) != 0o400
                    or opened.st_size != len(payload)
                    or storage._sha(os.read(input_fd, len(payload)+1)) != storage._sha(payload)):
                raise Hold('input_changed')
            os.lseek(input_fd, 0, os.SEEK_SET)
            state = store.get().state
            job_root = Path(store.root).resolve(strict=True)
            if job_root == self.workspace or self.workspace in job_root.parents:
                raise Hold('job_storage_inside_worker_workspace')
            if (state['envelope']['protocol'] != PROTOCOL
                    or state['state'] != 'staged' or state['stopped'] or state['unresolved']
                    or state['revision'] != expected_revision or state['generation'] != generation
                    or state['attempt']['token'] != token):
                raise Hold('claim_or_stop_mismatch')
            plan = dict(schema=1, job_id=state['job_id'], admission_digest=state['envelope']['admission_digest'],
                attempt_id=state['attempt']['attempt_id'], local_subject_digest=state['attempt']['local_subject_digest'],
                expected_revision=expected_revision, token=token, generation=generation,
                operation_ids=ids, command_digest=jobs.digest(command), environment_digest=jobs.digest(environment),
                cwd=[str(directory), info.st_dev, info.st_ino], stdin_digest=storage._sha(payload))
            if len(jobs._bytes(plan)) > 8192 or sum(len(x) for x in command) > 65536:
                raise Hold('input_bound')
            digest = jobs.digest(plan)
            if authorize(json.loads(jobs._bytes(plan)), json.loads(jobs._bytes(state))) is not True:
                raise Hold('authorization_unverified')
            with self.storage._lock():
                root = self.storage.root
                if {p.name for p in root.iterdir()} - {'.lock'}:
                    raise Hold('invocation_requires_reconciliation')
                storage._write(root/'intent.json', jobs._bytes(plan))
                storage._syncdir(root)
                self._checkpoint('before_intent')
                intent = store.change('intent', operation_id=ids['intent'], expected_revision=expected_revision,
                    token=token, generation=generation, data=dict(intent_sha256=digest))
                if intent.reconciled:
                    raise Hold('invocation_intent_requires_reconciliation')
                current = intent.state
                self._checkpoint('after_intent')
                environment_fd=os.memfd_create('ygg-review-command-env',os.MFD_CLOEXEC|os.MFD_ALLOW_SEALING)
                if os.write(environment_fd,environment_bytes)!=len(environment_bytes):
                    raise Hold('environment_transfer_unresolved')
                os.lseek(environment_fd,0,os.SEEK_SET)
                fcntl.fcntl(environment_fd,fcntl.F_ADD_SEALS,
                    fcntl.F_SEAL_SEAL|fcntl.F_SEAL_GROW|fcntl.F_SEAL_SHRINK|fcntl.F_SEAL_WRITE)
                read_fd, write_fd = os.pipe2(os.O_CLOEXEC)
                with (root/'stdout').open('xb') as out, (root/'stderr').open('xb') as err:
                    os.chmod(root/'stdout', 0o600)
                    os.chmod(root/'stderr', 0o600)
                    process = subprocess.Popen([sys.executable, '-I', '-c', _BARRIER,
                        str(read_fd), str(environment_fd), *command],
                        env=dict(PATH='/usr/bin:/bin',LC_ALL='C'), cwd=directory,
                        stdin=input_fd, stdout=out, stderr=err, pass_fds=(read_fd,environment_fd), start_new_session=True)
                os.close(read_fd)
                read_fd = None
                pidfd = os.pidfd_open(process.pid)
                fields = (Path('/proc')/str(process.pid)/'stat').read_text().rsplit(')', 1)[1].split()
                started = dict(schema=1, invocation_digest=digest, broker_pid=process.pid,
                    process_start_token=fields[19], native_session_identity='UNVERIFIED',
                    execution_effect='UNKNOWN_AFTER_RELEASE', task_acceptance='PENDING')
                storage._write(root/'broker.json', jobs._bytes(started))
                storage._syncdir(root)
                self._checkpoint('before_authorize')
                if authorize(json.loads(jobs._bytes(plan)), json.loads(jobs._bytes(current))) is not True:
                    raise Hold('authorization_unverified')
                authorization = store.change('authorize', operation_id=ids['authorize'],
                    expected_revision=expected_revision+1, token=token, generation=generation)
                if authorization.reconciled:
                    raise Hold('submission_authorization_requires_reconciliation')
                current = authorization.state
                self._checkpoint('after_authorize')
            # No model/network emission while either local mutation fence is held.
            self._checkpoint('before_release')
            released = True  # A write error is ambiguous, never permission to retry.
            if os.write(write_fd, b'G') != 1:
                raise Hold('barrier_release_unresolved')
            os.close(write_fd)
            write_fd = None
            self._checkpoint('after_release')
            started_receipt = store.change('record_start', operation_id=ids['start'],
                expected_revision=expected_revision+2, token=token, generation=generation,
                data=dict(receipt_sha256=jobs.digest(started)))
            if started_receipt.reconciled:
                raise Hold('start_receipt_requires_reconciliation')
            current=started_receipt.state
            self._checkpoint('after_start')
            return Receipt(process, digest, current, str(root))
        except Exception as error:
            code = error.code if isinstance(error, Hold) else 'invocation_unresolved'
            raise Hold(code, process, released) from None
        finally:
            active_error = sys.exc_info()[1]
            for fd in (read_fd, write_fd, input_fd, environment_fd):
                if fd is not None:
                    try:
                        os.close(fd)
                    except OSError:
                        if isinstance(active_error, Hold):
                            active_error.cleanup_unresolved = True
            if process is not None and not released:
                try:
                    try:
                        process.wait(timeout=2)
                    except subprocess.TimeoutExpired:
                        if pidfd is not None:
                            signal.pidfd_send_signal(pidfd, signal.SIGKILL)
                        process.wait(timeout=2)
                except (OSError, subprocess.TimeoutExpired):
                    if isinstance(active_error, Hold):
                        active_error.cleanup_unresolved = True
            if pidfd is not None:
                try:
                    os.close(pidfd)
                except OSError:
                    if isinstance(active_error, Hold):
                        active_error.cleanup_unresolved = True
