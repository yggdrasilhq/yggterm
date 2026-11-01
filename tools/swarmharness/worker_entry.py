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
    def __init__(self, code, receipt=None, process=None):
        self.code = code
        self.receipt = receipt
        self.process = process
        self.harness_started = process is not None
        self.no_retry_authorized = True
        self.diagnostic_persisted = False
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
    # This helper is the gate's exclusive waiter. Leave even an exited
    # leader unreaped until every numeric process-group signal is finished;
    # its reserved PID prevents a later group's reuse during cleanup.
    if process.returncode is not None:
        raise Held('gate_wait_ownership_lost')
    try:
        os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
    except ChildProcessError:
        raise Held('gate_wait_ownership_lost') from None
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    deadline = time.monotonic() + .2
    while time.monotonic() < deadline:
        try:
            os.killpg(process.pid, 0)
        except ProcessLookupError:
            break
        time.sleep(.01)
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    deadline = time.monotonic() + 2
    while _group_running(process.pid):
        if time.monotonic() >= deadline:
            error = Held('gate_cleanup_unresolved')
            error.gate_process = process
            raise error
        time.sleep(.01)
    try:
        process.wait(timeout=2)
    except subprocess.TimeoutExpired:
        error = Held('gate_cleanup_unresolved')
        error.gate_process = process
        raise error from None


def _group_running(group):
    """Qualified local procfs, same-UID gate group; zombies perform no work."""
    try:
        text = Path('/proc/self/stat').read_text()
        if int(text.split(' ', 1)[0]) != os.getpid():
            raise Held('gate_process_visibility_unknown')
        for path in Path('/proc').iterdir():
            if not path.name.isdigit():
                continue
            try:
                if path.stat().st_uid != os.getuid():
                    continue
                text = (path / 'stat').read_text()
                fields = text[text.rfind(')') + 2:].split()
                if int(fields[2]) == group and fields[0] not in ('Z', 'X', 'x'):
                    return True
            except FileNotFoundError:
                continue  # Process exited while enumerating this observation.
        return False
    except (OSError, ValueError, IndexError):
        raise Held('gate_process_visibility_unknown') from None


def _exit_without_reaping(process, deadline):
    while True:
        result = os.waitid(os.P_PID, process.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
        if result is not None:
            return result.si_status if result.si_code == os.CLD_EXITED else -result.si_status
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise Held('memory_gate_timeout')
        time.sleep(min(.01, remaining))


def _diagnostic(stage, name, payload):
    try:
        _write(stage / name, json.dumps(payload).encode())
        return True
    except (OSError, ValueError, TypeError):
        return False


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
            code = _exit_without_reaping(process, deadline)
            return code, bytes(streams['stdout']), bytes(streams['stderr'])
    except subprocess.TimeoutExpired:
        raise Held('memory_gate_timeout') from None
    finally:
        try:
            _finish(process)
        except Held as error:
            error.gate_process = process
            raise
        finally:
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
    if (sys.platform != 'linux' or not all(hasattr(os, name) for name in ('O_NOFOLLOW', 'waitid', 'WNOWAIT'))
            or not Path('/proc/self/fd').is_dir()):
        raise Held('unsupported_gate_platform')
    if (type(command) not in (list, tuple) or not command
            or any(type(part) is not str or not part or '\0' in part for part in command)
            or not Path(command[0]).is_absolute()
            or type(gate_timeout) not in (int, float) or not 0 < gate_timeout <= 120
            or type(gate_output_cap) is not int or not 0 < gate_output_cap <= 1024 * 1024):
        raise Held('invalid_launch_specification')
    # A controller may retain these mutable containers. Freeze them before
    # gate I/O so REQUEST and SPAWNED bind the same requested execution.
    command = tuple(command)
    stage = None
    process = None
    try:
        child_environment = dict(os.environ if environment is None else environment)
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
        if type(gate) is not Gate:
            raise Held('invalid_gate_specification')
        _write(stage / 'REQUEST.json', json.dumps({
            'attempt_id': stage.name, 'harness': gate.harness, 'namespace': gate.namespace,
            'qualified_source_sha256': gate.source_sha256,
            'command_sha256': _hash(json.dumps(list(command)).encode()),
            'source_or_task_acceptance': 'PENDING'}, sort_keys=True).encode())
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
        error.process = process
        error.harness_started = process is not None
        if stage is not None:
            error.receipt = str(stage)
            error.diagnostic_persisted = _diagnostic(stage, 'HELD.json', {
                'reason': error.code, 'harness_started': error.harness_started,
                'no_retry_authorized': True})
        raise
    except (OSError, ValueError, TypeError, RuntimeError) as error:
        held = Held('entry_launch_unresolved', str(stage) if stage else None, process)
        if stage is not None:
            held.diagnostic_persisted = _diagnostic(stage, 'UNRESOLVED.json', {
                'reason': type(error).__name__, 'harness_started': held.harness_started,
                'native_execution_effect': 'UNKNOWN', 'no_retry_authorized': True})
        raise held from None
