"""Bounded immutable Git staging, qualified local POSIX storage only.

Trusted controller supplies the repository and attests local filesystem semantics.
Same-UID writers are trusted and use the stable flock fence; permissions are NOT
isolation from a malicious same-UID process. The enrolled root alias resolves
once. No admission, privacy approval, SessionReview digest/signature, execution
permission, relocation qualification, garbage collection or completion authority.

API: Store(root, qualified_local=True).stage(repo, request), with exact request
{schema: 1, commit: full lowercase object ID, scope: logical name, paths: list}.
Hold contains bounded reason codes only. After any uncertain publication, retry
the identical request; success revalidates bytes and fsyncs visible directories.
Private abandoned .stage-* directories are deliberately retained after failure.
"""
from contextlib import contextmanager
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import re
import selectors
import stat
import subprocess
import time
import uuid

try:
    import fcntl
except ImportError:  # Unsupported hosts fail through the public HOLD boundary.
    fcntl = None

MAX_FILES = 128
MAX_BLOB = 2 * 1024 * 1024
MAX_TOTAL = 8 * 1024 * 1024
MAX_OBJECT = 2 * 1024 * 1024
MAX_MANIFEST = 128 * 1024
TIMEOUT = 10


class Hold(Exception):
    """Fail closed without exposing source bytes or Git stderr."""


def _canonical(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'),
                      ensure_ascii=True, allow_nan=False).encode('ascii')


def _sha(data):
    return hashlib.sha256(data).hexdigest()


def _keys(value, names):
    if type(value) is not dict or set(value) != set(names.split()):
        raise Hold('schema')


def _path(value):
    if (type(value) is not str or not value or len(value.encode('utf-8')) > 1024
            or '\\' in value or any(ord(c) < 32 or ord(c) == 127 for c in value)
            or any(p in ('', '.', '..', '.git') for p in value.split('/'))):
        raise Hold('path')
    return value


def _request(r):
    _keys(r, 'schema commit scope paths')
    if type(r['schema']) is not int or r['schema'] != 1:
        raise Hold('version')
    if type(r['commit']) is not str or not re.fullmatch('[0-9a-f]{40}|[0-9a-f]{64}', r['commit']):
        raise Hold('commit')
    if type(r['scope']) is not str or not re.fullmatch('[a-zA-Z0-9][a-zA-Z0-9_.:/-]{0,127}', r['scope']):
        raise Hold('scope')
    paths = r['paths']
    if type(paths) is not list or not 0 < len(paths) <= MAX_FILES:
        raise Hold('file_count')
    for p in paths:
        _path(p)
    if len(set(paths)) != len(paths):
        raise Hold('duplicate')
    paths = sorted(paths)
    if any(b.startswith(a + '/') for a, b in zip(paths, paths[1:])):
        raise Hold('path_conflict')
    return paths


def _git(repo, args, cap, deadline):
    # No inherited Git settings, replacement objects, lazy fetch, hooks, filters,
    # external diff, pager or network protocols. Only raw cat-file plumbing.
    env = dict(PATH='/usr/bin:/bin', LC_ALL='C', GIT_CONFIG_NOSYSTEM='1',
               GIT_CONFIG_GLOBAL='/dev/null', GIT_NO_REPLACE_OBJECTS='1',
               GIT_NO_LAZY_FETCH='1', GIT_ALLOW_PROTOCOL='', GIT_TERMINAL_PROMPT='0')
    argv = ['/usr/bin/git', '--no-pager', '-C', str(repo), 'cat-file', *args]
    try:
        p = subprocess.Popen(argv, env=env, stdin=subprocess.DEVNULL,
                             stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        with selectors.DefaultSelector() as selector:
            selector.register(p.stdout, selectors.EVENT_READ)
            chunks, size = [], 0
            while True:
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise Hold('git_timeout')
                if not selector.select(remaining):
                    raise Hold('git_timeout')
                block = os.read(p.stdout.fileno(), min(65536, cap + 1 - size))
                if not block:
                    break
                size += len(block)
                if size > cap:
                    raise Hold('git_output_cap')
                chunks.append(block)
            if p.wait(timeout=max(0.001, deadline - time.monotonic())):
                raise Hold('git_object_unavailable')
            return b''.join(chunks)
    except (OSError, subprocess.TimeoutExpired):
        raise Hold('git_failure') from None
    finally:
        if 'p' in locals():
            if p.poll() is None:
                p.kill()
            p.wait()
            p.stdout.close()


def _object(repo, oid, kind, cap, deadline):
    if _git(repo, ['-t', oid], 32, deadline) != kind.encode() + b'\n':
        raise Hold('object_kind')
    size = _git(repo, ['-s', oid], 32, deadline)
    if not re.fullmatch(b'[0-9]+\n', size) or int(size) > cap:
        raise Hold('object_cap')
    data = _git(repo, [kind, oid], cap, deadline)
    if len(data) != int(size):
        raise Hold('object_size')
    # Verify the object itself; never rely only on Git's filename lookup.
    algorithm = hashlib.sha1 if len(oid) == 40 else hashlib.sha256
    if algorithm(kind.encode() + b' ' + str(len(data)).encode() + b'\0' + data).hexdigest() != oid:
        raise Hold('object_hash')
    return data


def _sources(repo, r, paths):
    deadline = time.monotonic() + TIMEOUT
    commit = _object(repo, r['commit'], 'commit', MAX_OBJECT, deadline)
    first = commit.split(b'\n', 1)[0]
    if not re.fullmatch(b'tree [0-9a-f]{' + str(len(r['commit'])).encode() + b'}', first):
        raise Hold('commit_tree')
    tree = first[5:].decode('ascii')
    total, files, blobs, objects = 0, [], [], []
    # No unbounded tree cache: at most one bounded tree object per traversal.
    for name in paths:
        oid = tree
        parts = name.split('/')
        for index, part in enumerate(parts):
            raw = _object(repo, oid, 'tree', MAX_OBJECT, deadline)
            entries, offset = {}, 0
            width = len(oid) // 2
            while offset < len(raw):
                end = raw.find(b'\0', offset)
                if end < 0 or end + 1 + width > len(raw):
                    raise Hold('tree_format')
                header = raw[offset:end].split(b' ', 1)
                if len(header) != 2 or header[1] in entries:
                    raise Hold('tree_format')
                entries[header[1]] = (header[0], raw[end+1:end+1+width].hex())
                offset = end + 1 + width
            entry = entries.get(part.encode('utf-8'))
            if entry is None:
                raise Hold('missing_path')
            mode, oid = entry
            if index < len(parts) - 1:
                if mode != b'40000':
                    raise Hold('ancestor_kind')
            elif mode not in (b'100644', b'100755'):
                raise Hold('source_kind')
        data = _object(repo, oid, 'blob', min(MAX_BLOB, MAX_TOTAL-total), deadline)
        total += len(data)
        files.append(dict(path=name, mode=mode.decode('ascii'), size=len(data), sha256=_sha(data)))
        blobs.append(data)
        objects.append(dict(path=name, blob=oid))
    return dict(schema=1, scope=r['scope'], files=files), blobs, tree, objects


def _owned(info, kind):
    if (info.st_uid != os.getuid() or not kind(info.st_mode)
            or info.st_mode & 0o077):
        raise Hold('unsafe_storage')


def _directory(path):
    _owned(os.lstat(path), stat.S_ISDIR)


def _read(path, cap):
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        info = os.fstat(fd)
        _owned(info, stat.S_ISREG)
        if info.st_nlink != 1 or info.st_size > cap:
            raise Hold('stored_file_cap_or_link')
        with os.fdopen(fd, 'rb', closefd=False) as f:
            data = f.read(cap + 1)
        if len(data) > cap:
            raise Hold('stored_file_cap_or_link')
        os.fsync(fd)
        return data, stat.S_IMODE(info.st_mode)
    finally:
        os.close(fd)


def _write(path, data, mode=0o400):
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    try:
        with os.fdopen(fd, 'wb', closefd=False) as f:
            f.write(data)
            f.flush()
        os.fchmod(fd, mode)
        os.fsync(fd)
    finally:
        os.close(fd)


def _syncdir(path):
    fd = os.open(path, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


@dataclass(frozen=True)
class Snapshot:
    artifact_id: str
    materialization: str
    files: tuple
    provenance_receipt: str
    source_commit: str
    source_tree: str


class Store:
    def __init__(self, root, *, qualified_local=False):
        if (os.name != 'posix' or fcntl is None or qualified_local is not True
                or not hasattr(os, 'O_NOFOLLOW')):
            raise Hold('unsupported_storage')
        try:
            self.root = Path(root).resolve(strict=True)
            _directory(self.root)
        except (OSError, ValueError, RuntimeError):
            raise Hold('storage_root') from None

    @contextmanager
    def _lock(self):
        fd = os.open(self.root / '.lock', os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_NONBLOCK, 0o600)
        try:
            info = os.fstat(fd)
            _owned(info, stat.S_ISREG)
            if info.st_nlink != 1:
                raise Hold('lock_kind')
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise Hold('lock_busy') from None
            yield
        finally:
            os.close(fd)

    def _checkpoint(self, phase):
        """Fault-injection seam; production does nothing."""

    def _verify(self, dest, manifest):
        _directory(dest)
        expected = {'manifest.json', 'source'}
        with os.scandir(dest) as entries:
            found = set()
            for entry in entries:
                if entry.name not in expected or entry.name in found:
                    raise Hold('artifact_layout')
                found.add(entry.name)
        if found != expected:
            raise Hold('artifact_layout')
        raw, mode = _read(dest / 'manifest.json', MAX_MANIFEST)
        # Exact canonical schema/bytes, including version and all nested keys.
        if raw != _canonical(manifest) or mode != 0o400:
            raise Hold('manifest_mismatch')
        source = dest / 'source'
        _directory(source)
        expected_files = {f['path']: f for f in manifest['files']}
        expected_dirs = {''}
        for name in expected_files:
            parts = name.split('/')
            expected_dirs.update('/'.join(parts[:i]) for i in range(1, len(parts)))
        seen_files, seen_dirs = set(), {''}
        def visit(directory, relative):
            # Bound listing even for a corrupt directory containing many entries.
            with os.scandir(directory) as entries:
                for count, entry in enumerate(entries):
                    if count > MAX_FILES * 1024:
                        raise Hold('artifact_layout')
                    name = relative + '/' + entry.name if relative else entry.name
                    if name in expected_dirs:
                        _directory(entry.path)
                        seen_dirs.add(name)
                        visit(Path(entry.path), name)
                    elif name in expected_files:
                        f = expected_files[name]
                        data, mode = _read(entry.path, min(MAX_BLOB, f['size']))
                        if (len(data) != f['size'] or _sha(data) != f['sha256']
                                or mode != (0o500 if f['mode'] == '100755' else 0o400)):
                            raise Hold('materialization_mismatch')
                        seen_files.add(name)
                    else:
                        raise Hold('artifact_layout')
            _syncdir(directory)
        visit(source, '')
        if seen_files != set(expected_files) or seen_dirs != expected_dirs:
            raise Hold('artifact_layout')
        _syncdir(dest)

    def stage(self, repo, request):
        """Return only verified, durable local bindings; all failures retain data."""
        try:
            return self._stage(repo, request)
        except Hold:
            raise
        except (OSError, ValueError, UnicodeError, RecursionError):
            raise Hold('storage_or_input_failure') from None

    def _stage(self, repo, request):
        paths = _request(request)
        request = dict(schema=1, commit=request['commit'], scope=request['scope'], paths=paths)
        repo = Path(repo).resolve(strict=True)  # trusted repository location
        manifest, blobs, tree, objects = _sources(repo, request, paths)
        encoded = _canonical(manifest)
        if len(encoded) > MAX_MANIFEST:
            raise Hold('manifest_cap')
        artifact = _sha(encoded)
        provenance = dict(schema=1, artifact_id=artifact, repository=str(repo),
                          commit=request['commit'], tree=tree, objects=objects)
        receipt = _canonical(provenance)
        if len(receipt) > MAX_MANIFEST:
            raise Hold('provenance_cap')
        dest = self.root / artifact
        receipt_path = self.root / ('provenance-' + _sha(receipt) + '.json')
        visible = False
        with self._lock():
            try:
                if not os.path.lexists(dest):
                    staging = self.root / ('.stage-' + uuid.uuid4().hex)
                    staging.mkdir(mode=0o700)
                    source = staging / 'source'
                    source.mkdir(mode=0o700)
                    for f, data in zip(manifest['files'], blobs):
                        path = source / f['path']
                        path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
                        _write(path, data, 0o500 if f['mode'] == '100755' else 0o400)
                    _write(staging / 'manifest.json', encoded)
                    self._verify(staging, manifest)
                    self._checkpoint('before_publish')
                    # All trusted controllers share this stable nonblocking fence.
                    # Destination checked under it; never rename onto an existing entry.
                    if os.path.lexists(dest):
                        raise Hold('publication_conflict')
                    os.rename(staging, dest)
                    visible = True
                    self._checkpoint('after_publish')
                else:
                    visible = True
                self._verify(dest, manifest)
                _syncdir(self.root)
                if os.path.lexists(receipt_path):
                    actual, mode = _read(receipt_path, MAX_MANIFEST)
                    if actual != receipt or mode != 0o400:
                        raise Hold('provenance_mismatch')
                else:
                    staged_receipt = self.root / ('.stage-' + uuid.uuid4().hex)
                    _write(staged_receipt, receipt)
                    # Same stable fence and no-existing-entry rule as the artifact.
                    if os.path.lexists(receipt_path):
                        raise Hold('publication_conflict')
                    os.rename(staged_receipt, receipt_path)
                _syncdir(self.root)
                self._checkpoint('durable')
            except OSError:
                raise Hold('durability_uncertain' if visible else 'storage_failure') from None
        return Snapshot(artifact, str(dest / 'source'),
                        tuple(str(dest / 'source' / p) for p in paths),
                        str(receipt_path), request['commit'], tree)
