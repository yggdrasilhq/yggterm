"""Linux prelaunch memory gate, not authority, confinement or a retry protocol.

Trusted controller supplies canonical harness/namespace and qualified source
hash. Models never construct these from their own output. A passing gate does
not grant policy, quota, budget, privacy or owner-stop permission. The caller
must already own those obligations and a durable invocation record. This
primitive cannot prevent an admitted process writing outside its task later.
"""
from dataclasses import asdict, dataclass
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import signal
import stat
import subprocess
import sys
import time
import uuid


class Held(Exception):
    def __init__(self, code, receipt=None):
        self.code = code
        self.receipt = receipt
        super().__init__(code)


@dataclass(frozen=True)
class Gate:
    harness: str
    namespace: str
    source: str
    source_sha256: str


@dataclass(frozen=True)
class Receipt:
    attempt_id: str
    harness: str
    namespace: str
    source_sha256: str
    observed_at: str
    report_sha256: str
    evidence_directory: str


def _hash(data):
    return hashlib.sha256(data).hexdigest()


def _write(path, data):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, 'wb') as output:
        output.write(data)
        output.flush()
        os.fsync(output.fileno())


def _finish(process):
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=1)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=2)


def _collect(command, source_fd, environment, timeout, cap):
    process = subprocess.Popen(command, env=environment, pass_fds=(source_fd,),
        stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        start_new_session=True)
    streams = {'stdout': bytearray(), 'stderr': bytearray()}
    deadline = time.monotonic() + timeout
    try:
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ, 'stdout')
            selector.register(process.stderr, selectors.EVENT_READ, 'stderr')
            while selector.get_map():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise Held('memory_gate_timeout')
                events = selector.select(remaining)
                if not events:
                    raise Held('memory_gate_timeout')
                for key, _ in events:
                    data = os.read(key.fileobj.fileno(), min(65536, cap + 1))
                    if not data:
                        selector.unregister(key.fileobj)
                    else:
                        streams[key.data].extend(data)
                        if sum(map(len, streams.values())) > cap:
                            raise Held('memory_gate_output_cap')
            process.wait(timeout=max(.001, deadline - time.monotonic()))
            return process.returncode, bytes(streams['stdout']), bytes(streams['stderr'])
    except subprocess.TimeoutExpired:
        raise Held('memory_gate_timeout') from None
    finally:
        _finish(process)
        process.stdout.close()
        process.stderr.close()


def _json_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise Held('memory_gate_unknown_result')
        value[key] = item
    return value


def _gate(gate, stage, environment, timeout, cap):
    if (type(gate) is not Gate or not re.fullmatch(r'[a-z][a-z0-9_-]{0,63}', gate.harness)
            or not re.fullmatch(r'_global|-[A-Za-z0-9_.-]+', gate.namespace)
            or not re.fullmatch(r'[0-9a-f]{64}', gate.source_sha256)):
        raise Held('invalid_gate_specification')
    source = Path(gate.source)
    if not source.is_absolute():
        raise Held('unqualified_gate_source')
    descriptor = os.open(source, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        info = os.fstat(descriptor)
        if (not stat.S_ISREG(info.st_mode) or info.st_uid != os.getuid()
                or info.st_mode & 0o022 or info.st_size > 4 * 1024 * 1024):
            raise Held('unqualified_gate_source')
        with os.fdopen(descriptor, 'rb', closefd=False) as file:
            data = file.read(4 * 1024 * 1024 + 1)
        if len(data) > 4 * 1024 * 1024 or _hash(data) != gate.source_sha256:
            raise Held('gate_source_changed')
        # Descriptor pinning preserves the exact script across atomic rename.
        command = [sys.executable, '-I', '/proc/self/fd/' + str(descriptor), 'sync',
                   '--harness', gate.harness, '--ns=' + gate.namespace, '--json']
        rc, output, errors = _collect(command, descriptor, environment, timeout, cap)
        _write(stage / 'gate.stdout', output)
        _write(stage / 'gate.stderr', errors)
        if rc != 0:
            raise Held('memory_gate_failed')
        try:
            report = json.loads(output, object_pairs_hook=_json_object)
        except (ValueError, UnicodeError):
            raise Held('memory_gate_unknown_result') from None
        if (type(report) is not dict or report.get('status') != 'ok'
                or set(report) - {'status', 'harness', 'namespace', 'pulled_in', 'pushed_out',
                                  'deleted', 'written', 'conflicts', 'missing_objects',
                                  'schema', 'schema_version'}
                or report.get('harness') != gate.harness
                or report.get('namespace') != gate.namespace
                or any(type(report[key]) is not int or report[key] != 1
                       for key in ('schema', 'schema_version') if key in report)
                or any(type(report.get(key)) is not int or report[key] != 0
                       for key in ('conflicts', 'missing_objects'))):
            raise Held('memory_gate_unknown_result')
        return Receipt(stage.name, gate.harness, gate.namespace, gate.source_sha256,
                       datetime.now(timezone.utc).isoformat(), _hash(output), str(stage))
    finally:
        os.close(descriptor)


def guarded_spawn(gate, command, *, cwd, evidence_root, environment=None,
                  gate_timeout=60, gate_output_cap=65536):
    """Run the exact gate before constructing a detached command process.

    Returns (Popen, Receipt). Caller supervises the process and records native
    identity; neither PID, exit0 nor this receipt is an editorial acceptance.
    Commands and environment are trusted controller inputs, never worker JSON.
    No automatic retry; every call creates a separate explicit attempt.
    """
    if (sys.platform != 'linux' or not hasattr(os, 'O_NOFOLLOW')
            or not Path('/proc/self/fd').is_dir()):
        raise Held('unsupported_gate_platform')
    if (type(command) not in (list, tuple) or not command
            or any(type(part) is not str or not part or '\0' in part for part in command)
            or not Path(command[0]).is_absolute()
            or type(gate_timeout) not in (int, float) or not 0 < gate_timeout <= 120
            or type(gate_output_cap) is not int or not 0 < gate_output_cap <= 1024 * 1024):
        raise Held('invalid_launch_specification')
    stage = None
    try:
        root = Path(evidence_root).resolve(strict=True)
        info = root.lstat()
        if (not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid()
                or info.st_mode & 0o077):
            raise Held('unsafe_gate_evidence')
        workspace = Path(cwd).resolve(strict=True)
        if root == workspace or workspace in root.parents:
            raise Held('gate_evidence_inside_worker_workspace')
        stage = root / str(uuid.uuid4())
        stage.mkdir(mode=0o700)
        child_environment = dict(os.environ if environment is None else environment)
        receipt = _gate(gate, stage, child_environment, gate_timeout, gate_output_cap)
        _write(stage / 'MEMORY-READY.json', json.dumps(asdict(receipt), sort_keys=True).encode())
        with (stage / 'worker.stdout').open('xb') as output, (stage / 'worker.stderr').open('xb') as errors:
            os.chmod(output.name, 0o600)
            os.chmod(errors.name, 0o600)
            process = subprocess.Popen(list(command), cwd=workspace, env=child_environment,
                stdin=subprocess.DEVNULL, stdout=output, stderr=errors, start_new_session=True)
        _write(stage / 'SPAWNED.json', json.dumps({
            'pid': process.pid, 'command_sha256': _hash(json.dumps(list(command)).encode()),
            'memory_receipt': asdict(receipt), 'native_identity': 'UNVERIFIED',
            'source_or_task_acceptance': 'PENDING'}, sort_keys=True).encode())
        return process, receipt
    except Held as error:
        if stage is not None:
            error.receipt = str(stage)
            _write(stage / 'HELD.json', json.dumps({'reason': error.code, 'harness_started': False}).encode())
        raise
    except (OSError, ValueError, TypeError, RuntimeError) as error:
        if stage is not None:
            _write(stage / 'UNRESOLVED.json', json.dumps({
                'reason': type(error).__name__, 'harness_started': 'UNKNOWN',
                'no_retry_authorized': True}).encode())
        raise Held('entry_launch_unresolved', str(stage) if stage else None) from None
