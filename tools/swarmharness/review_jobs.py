"""Local POSIX review-job storage; no dispatcher or completion authority.

One immutable admitted job per private store. The trusted controller validates
permission, Admission, scope, policy and protocol before calling us. This module
checks their closed wire shape, not their authority. Same-user trusted storage
(C12) is assumed: this is not isolation from a malicious same-UID writer.
Logical root aliases resolve ONCE at construction; relocation is unsupported.

All writes, including stop and submission authorization, share one flock fence.
Intent/authorization/start expiry is unresolved: only pre-intent ownership can
be explicitly recovered. Stop after authorization records unresolved execution;
it cannot promise zero external bytes. No external operations occur here.

Each mutation needs a unique controller operation UUID. Retry the SAME request
with that UUID after a lost receipt: Receipt.reconciled then reports the CURRENT
durable state, never permission to replay an external call. A new operation UUID
is a new CAS request. No automatic retry, resume, acceptance or task application.
SQLite FULL transactions retain immutable genesis and hash-chained events.
Unknown/corrupt/partial stores HOLD without reset or migration. Supported only
on qualified local POSIX filesystems with working flock/fsync/atomic SQLite;
the controller must establish locality (network filesystems are not qualified).
An orphan SQLite journal also HOLDs, preserved for a separate recovery adapter;
opening the store never lets SQLite silently repair unclassified partial data.
"""
from contextlib import contextmanager
from dataclasses import dataclass
import hashlib
import json
import math
import os
from pathlib import Path, PurePosixPath
import re
import sqlite3
import stat
import sys
import time
import uuid

from .review_completion import Admission, _parse


class Hold(Exception):
    """Fail closed; messages contain codes, never submitted private payloads."""


@dataclass(frozen=True)
class Receipt:
    state: dict
    reconciled: bool = False


def _bytes(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'),
                      ensure_ascii=True, allow_nan=False).encode()


def digest(value):
    """Content identity of canonical JSON (not the kernel's local file digest)."""
    return hashlib.sha256(_bytes(value)).hexdigest()


def _hash(value):
    if type(value) is not str or not re.fullmatch('[0-9a-f]{64}', value):
        raise Hold('invalid_hash')


def _uuid(value):
    if type(value) is not str or str(uuid.UUID(value)) != value:
        raise Hold('invalid_uuid')


def _keys(value, keys):
    if type(value) is not dict or set(value) != set(keys.split()):
        raise Hold('closed_schema_required')


def _envelope(e):
    _keys(e, 'schema original_request_sha256 parent_ref admission admission_digest artifact_sha256 source_scope review_policy_sha256 protocol')
    if type(e['schema']) is not int or e['schema'] != 1:
        raise Hold('unknown_schema')
    a = _parse(Admission, e['admission'])
    for key in ('original_request_sha256', 'admission_digest',
                'artifact_sha256', 'review_policy_sha256'):
        _hash(e[key])
    if a.digest != e['admission_digest']:
        raise Hold('admission_mismatch')
    for key in ('parent_ref', 'protocol'):
        if type(e[key]) is not str or not e[key].strip() or len(e[key]) > 1024:
            raise Hold('invalid_reference')
    scope = e['source_scope']
    if type(scope) is not list or not scope or len(scope) > 256:
        raise Hold('invalid_scope')
    for p in scope:
        if (type(p) is not str or not p or len(p) > 1024
                or PurePosixPath(p).is_absolute() or '\\' in p
                or any(c in ('', '.', '..') for c in p.split('/'))):
            raise Hold('invalid_scope')
    if scope != sorted(set(scope)):
        raise Hold('invalid_scope')


def _initial(e):
    _envelope(e)
    return dict(job_id=digest(e), review_id='review-' + digest(e), envelope=e, revision=0, generation=e[
        'admission']['owner_stop_generation'], stopped=False, state='waiting',
        attempt=None, unresolved=False)


def _apply(s, request, generated):
    _keys(request, 'operation_id action expected_revision generation token data')
    _uuid(request['operation_id'])
    if (type(request['expected_revision']) is not int
            or request['expected_revision'] != s['revision']
            or type(request['generation']) is not int
            or request['generation'] != s['generation']):
        raise Hold('stale_revision_or_generation')
    action, data, a = request['action'], request['data'], s['attempt']
    s = json.loads(_bytes(s))
    if action in ('stop', 'resume', 'claim', 'recover'):
        if request['token'] is not None:
            raise Hold('unexpected_token')
    elif not a or request['token'] != a['token']:
        raise Hold('stale_token')
    if action == 'stop':
        _keys(data, '')
        _keys(generated, '')
        s.update(stopped=True, generation=s['generation'] + 1)
        s['unresolved'] = s['unresolved'] or s['state'] in (
            'invocation-intent', 'submission-authorized', 'invocation-started')
        s['state'] = 'suspended'
    elif action == 'resume':
        _keys(data, '')
        _keys(generated, '')
        if not s['stopped'] or s['unresolved']:
            raise Hold('reconciliation_required')
        s.update(stopped=False, state='waiting', attempt=None)
    else:
        if s['stopped']:
            raise Hold('owner_stopped')
        if action in ('claim', 'recover'):
            _keys(data, 'lease_seconds')
            _keys(generated, 'attempt_id token now')
            _uuid(generated['attempt_id'])
            _uuid(generated['token'])
            now, lease = generated['now'], data['lease_seconds']
            if (type(now) not in (int, float) or not math.isfinite(now)
                    or type(lease) not in (int, float)
                    or not math.isfinite(lease) or not 0 < lease <= 86400):
                raise Hold('invalid_lease')
            if action == 'claim' and s['state'] != 'waiting':
                raise Hold('already_owned')
            if action == 'recover' and (s['state'] not in ('claimed', 'staged')
                    or not a or now < a['expires'] or s['unresolved']):
                raise Hold('reconciliation_required')
            s.update(state='claimed', attempt=dict(attempt_id=generated[
                'attempt_id'], token=generated['token'], expires=now + lease))
        else:
            _keys(generated, '')
            if action == 'stage':
                _keys(data, 'local_subject_digest')
                _hash(data['local_subject_digest'])
                if s['state'] != 'claimed':
                    raise Hold('invalid_transition')
                s['attempt']['local_subject_digest'] = data['local_subject_digest']
                s['state'] = 'staged'
            elif action in ('intent', 'authorize', 'record_start'):
                previous, target, field = {
                    'intent': ('staged', 'invocation-intent', 'intent_sha256'),
                    'authorize': ('invocation-intent', 'submission-authorized', None),
                    'record_start': ('submission-authorized', 'invocation-started', 'receipt_sha256'),
                }[action]
                _keys(data, field or '')
                if field:
                    _hash(data[field])
                if s['state'] != previous:
                    raise Hold('invalid_transition')
                s['attempt'].update(data)
                s['state'] = target
            elif action == 'suspend':
                _keys(data, '')
                s['unresolved'] = s['unresolved'] or s['state'] in (
                    'invocation-intent', 'submission-authorized', 'invocation-started')
                s['state'] = 'suspended'
            else:
                raise Hold('unsupported_transition')
    s['revision'] += 1
    return s


_SQL = 'CREATE TABLE records (n INTEGER PRIMARY KEY, payload TEXT NOT NULL, digest TEXT NOT NULL)'
_MAX_EVENTS = 1024
_MAX_BYTES = 8 * 1024 * 1024


class Store:
    """admit/get/change only; change uses the closed actions documented above.

    claim/recover create fresh UUIDs and use injected clock for bounded leases.
    recover/resume are explicit trusted-controller actions, never background work.
    A stage digest records the global-to-local binding; staging bytes is outside
    this store. Authorization records ordering only, not configured permission.
    fault is a test hook at before_commit/after_commit; exceptions are HOLD and
    require reconciliation using the original operation UUID.
    """

    def __init__(self, root, *, clock=time.time, fault=None):
        if os.name != 'posix' or sys.platform not in ('linux', 'darwin'):
            raise Hold('unsupported_platform')
        self.root = Path(root).expanduser().resolve()
        self.clock, self.fault = clock, fault

    def _private(self, path, directory=False):
        st = path.lstat()
        if (st.st_uid != os.getuid() or st.st_mode & 0o077
                or not (stat.S_ISDIR(st.st_mode) if directory else stat.S_ISREG(st.st_mode))
                or (not directory and st.st_nlink != 1)):
            raise Hold('unsafe_storage')

    @contextmanager
    def _locked(self, initialize=False):
        import fcntl
        fd = None
        db = None
        try:
            fresh = False
            if initialize:
                try:
                    self.root.mkdir(mode=0o700)
                    fresh = True
                    parent = os.open(self.root.parent, os.O_RDONLY | os.O_DIRECTORY)
                    try:
                        os.fsync(parent)
                    finally:
                        os.close(parent)
                except FileExistsError:
                    pass
            self._private(self.root, True)
            lock = self.root / 'fence'
            # Never replace the fence inode. Existing stores without it HOLD.
            flags = os.O_RDWR | os.O_NOFOLLOW
            fd = os.open(lock, flags | (os.O_CREAT | os.O_EXCL if fresh else 0), 0o600)
            self._private(lock)
            try:
                fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                raise Hold('store_busy') from None
            for p in self.root.iterdir():
                if p.name not in ('fence', 'store.sqlite', 'store.sqlite-journal'):
                    raise Hold('unknown_storage')
                self._private(p)
                if p.stat().st_size > _MAX_BYTES:
                    raise Hold('storage_bound')
            path = self.root / 'store.sqlite'
            if (self.root / 'store.sqlite-journal').exists():
                raise Hold('unfinished_storage_transaction')
            if fresh:
                dfd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
                os.close(dfd)
            elif not path.exists():
                raise Hold('missing_schema')
            db = sqlite3.connect(path, timeout=0, isolation_level=None)
            db.execute('PRAGMA synchronous=FULL')
            db.execute('PRAGMA trusted_schema=OFF')
            # Keep a valid final receipt readable under the preflight bound.
            # This runtime limit does not rewrite an existing database header.
            db.execute('PRAGMA max_page_count=1024')
            if fresh:
                db.execute('BEGIN IMMEDIATE')
                db.execute(_SQL)
            else:
                if (db.execute('PRAGMA user_version').fetchone() != (0,)
                        or db.execute('PRAGMA application_id').fetchone() != (0,)
                        or db.execute('PRAGMA journal_mode').fetchone() != ('delete',)
                        or db.execute('PRAGMA page_size').fetchone() != (4096,)):
                    raise Hold('unknown_schema')
                if db.execute('PRAGMA integrity_check').fetchall() != [('ok',)]:
                    raise Hold('corrupt_storage')
                schema = db.execute("SELECT type,name,sql FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'").fetchall()
                if schema != [('table', 'records', _SQL)]:
                    raise Hold('unknown_schema')
                db.execute('BEGIN IMMEDIATE')
            yield db, fresh
        except Hold:
            raise
        except Exception:
            raise Hold('storage_or_contract_failure') from None
        finally:
            if db:
                db.close()
            if fd is not None:
                os.close(fd)

    def _read(self, db):
        rows = db.execute('SELECT n,payload,digest FROM records ORDER BY n LIMIT ?',
                          (_MAX_EVENTS + 1,)).fetchall()
        if not rows or len(rows) > _MAX_EVENTS:
            raise Hold('missing_or_bounded_history')
        prev, state, operations = '0' * 64, None, {}
        total = 0
        for index, (n, raw, h) in enumerate(rows):
            total += len(raw)
            if n != index or len(raw) > 65536 or total > _MAX_BYTES:
                raise Hold('invalid_history')
            value = json.loads(raw)
            if _bytes(value).decode() != raw or digest([prev, value]) != h:
                raise Hold('invalid_history')
            if index == 0:
                _keys(value, 'schema envelope')
                if type(value['schema']) is not int or value['schema'] != 1:
                    raise Hold('unknown_schema')
                state = _initial(value['envelope'])
            else:
                _keys(value, 'request generated')
                request = value['request']
                if request['operation_id'] in operations:
                    raise Hold('duplicate_operation')
                state = _apply(state, request, value['generated'])
                operations[request['operation_id']] = request
            prev = h
        return state, operations, prev, len(rows)

    def _commit(self, db, n, prev, value):
        raw = _bytes(value).decode()
        used = db.execute('SELECT coalesce(sum(length(payload)),0) FROM records').fetchone()[0]
        if len(raw) > 65536 or n >= _MAX_EVENTS or used + len(raw) > _MAX_BYTES // 2:
            raise Hold('storage_bound')
        db.execute('INSERT INTO records VALUES (?,?,?)', (n, raw, digest([prev, value])))
        if self.fault:
            self.fault('before_commit')
        db.execute('COMMIT')
        directory = os.open(self.root, os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
        if self.fault:
            self.fault('after_commit')

    def admit(self, envelope):
        """Identical envelope is idempotent; contradictory input never overwrites."""
        try:
            envelope = json.loads(_bytes(envelope))
            state = _initial(envelope)
        except Exception:
            raise Hold('invalid_envelope') from None
        with self._locked(initialize=True) as (db, fresh):
            if fresh:
                self._commit(db, 0, '0' * 64, dict(schema=1, envelope=envelope))
                return Receipt(state)
            current, _, _, _ = self._read(db)
            if current['envelope'] != envelope:
                raise Hold('contradictory_admission')
            return Receipt(current, True)

    def get(self):
        with self._locked() as (db, _):
            return Receipt(self._read(db)[0])

    def change(self, action, *, operation_id, expected_revision, generation,
               token=None, data=None):
        """CAS mutation. data: claim/recover={lease_seconds}; stage={local_subject_digest};
        intent={intent_sha256}; record_start={receipt_sha256}; others={}.
        Every attempt mutation needs its token, revision and stop generation.
        """
        with self._locked() as (db, _):
            s, operations, prev, n = self._read(db)
            request = dict(operation_id=operation_id, action=action,
                           expected_revision=expected_revision, generation=generation,
                           token=token, data={} if data is None else data)
            request = json.loads(_bytes(request))
            if operation_id in operations:
                if request != operations[operation_id]:
                    raise Hold('contradictory_operation')
                return Receipt(s, True)
            generated = (dict(attempt_id=str(uuid.uuid4()), token=str(uuid.uuid4()),
                              now=self.clock()) if action in ('claim', 'recover') else {})
            s = _apply(s, request, generated)
            self._commit(db, n, prev, dict(request=request, generated=generated))
            return Receipt(s)
