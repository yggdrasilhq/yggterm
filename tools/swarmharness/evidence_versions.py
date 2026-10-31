"""Standalone immutable evidence-version store (CSR09, schema 2).

Durable facts for composer evidence: every consumed raw page window W
(even when it yields no observation), every observation version V that
references durable windows plus a carried detector context C, and legacy
normalized records whose original bytes are unavailable. Identities are
SHA-256 of canonical JSON (json.dumps default ensure_ascii=True, matching
the composer's canonicalizer) per docs/spec-composer-evidence-versions.md;
timestamps are provenance only and never identity.

The store is independent of composer.py: it computes the logical key L
from the same six elements the composer's v2 identity uses, so a caller
can map one onto the other without either module importing the other.

Context has ONE authoritative source per observation: the observation's
own ``context`` when present, otherwise the call's top-level ``context``;
supplying both for one observation is refused. Accepted context values
are a closed typed metadata projection (dict keys, values limited to
null/bool/bounded int/digest-hashref/nested containers — no free text);
body-bearing values travel only as canonical digests or ``unbound`` refs
(typed UNKNOWN evidence). Stored facts contain no raw page, prompt,
transcript, credential or path text.

Storage: SQLite (stdlib) at <state_dir>/evidence-versions.db, adopted to
WAL only after the schema marker validates. All writers serialize on a
stable flock (<state_dir>/evidence-versions.lock). A store file that is
absent, with no sidecars, is the only genesis path: the complete schema
is built in a private temp file, fsynced, then published with a
no-replace hardlink (namespace CAS); a lost race validates the winner's
store instead. An existing store — including an empty one, one without
the marker, or a future schema — is validated through a genuinely
read-only connection (URI ``mode=ro``, never ``query_only`` on a
read/write handle, never ``immutable=1``): validation failure raises
EvidenceHeld before any read/write handle, journal-mode change or schema
creation, preserving existing durable DB and WAL evidence. Read-only
SQLite can create empty WAL/SHM coordination files and update volatile
SHM read marks. SQLite itself owns those coordination effects; this
module never moves, deletes or manually checkpoints live sidecars.
"""

import fcntl
import hashlib
import json
import os
import re
import sqlite3
import time
import urllib.request
import uuid

SCHEMA_VERSION = "2"
DB_NAME = "evidence-versions.db"
LOCK_NAME = "evidence-versions.lock"

_PAGE_DIGEST_UNKNOWN = "UNKNOWN"       # typed gap: raw is None, never a fake hash
_RAW_EVIDENCE_UNAVAILABLE = "unavailable"  # legacy rows: typed constant
_SEVERITIES = ("high", "medium", "low")
_STATUSES = ("verified-symptom", "unverified-symptom")

_OBS_FIELDS = ("detector_id", "detector_version", "severity", "status",
               "span", "fingerprint")
_OBS_OPTIONAL = ("refs", "context")

_SOURCE_FIELDS = ("project", "source_kind", "lineage_id", "generation")
_CONTEXT_KEYS = frozenset({"depth", "count", "counts", "schema_version", "unknown",
                          "carried", "carried_observations", "carried_digest",
                          "prior_offset", "prior_window_id", "prior_page_digest",
                          "cmd_fps", "pending_fps", "att_offs", "fp", "window", "byte_off"})

_CONTEXT_MAX_DEPTH = 4
_CONTEXT_MAX_NODES = 64
_SUMMARY_LIMIT_MAX = 200

# Closed typed context domain: values admit NO free text. The only legal
# strings are content digests (64 lowercase hex) or store identities
# (W-/V-/C-/U-/L-/G- prefixed). Floats are excluded: canonical float
# formatting is not part of the schema-2 typed projection.
_DIGEST_RE = re.compile(r"\A[0-9a-f]{64}\Z")
_IDENTITY_RE = re.compile(r"\A[WVCLUG]-[0-9a-f]{64}\Z")
_CONTEXT_MAX_INT = 2 ** 63 - 1

# Context keys that declare carried state and trigger the durable
# prior-window binding rule (EV-CARRIED-UNKNOWN-02).
_CARRIED_KEYS = ("prior_offset", "prior_window_id", "prior_page_digest")

_LOCK_TIMEOUT_S = 60.0
_WINDOW_COLS = ("window_id", "source_kind", "lineage_id", "generation",
                "start", "end", "page_digest", "raw_sha256")


class EvidenceHeld(Exception):
    """An unknown, corrupt, contradictory or storage-faulted evidence
    condition. No success receipt authorizes cursor advance. The fact
    transaction is rolled back; a complete empty genesis or an uncertain
    namespace publication may remain and must be retained for replay."""


def _canonical(obj):
    # json.dumps defaults: ensure_ascii=True — matches the composer
    # canonicalizer and the spec's documented encoding exactly.
    return json.dumps(obj, sort_keys=True, separators=(",", ":"))


def _canon_bytes(obj):
    return _canonical(obj).encode("utf-8")


def _sha_hex(data):
    return hashlib.sha256(data).hexdigest()


def _identity(prefix, obj):
    return prefix + _sha_hex(_canon_bytes(obj))


def _require_nonempty_str(value, name):
    if not isinstance(value, str) or not value:
        raise EvidenceHeld(f"{name} must be a nonempty string")


def _validate_context_value(value, path="context", depth=0):
    """Closed typed metadata projection: dict keys are short free strings
    (names, not payloads); values admit only None, bool, bounded int,
    digest/hashref strings and nested lists/dicts of the same. Any other
    string — prompt, credential, path, prose, enum text — is refused
    structurally, not by pattern-matching for known secret shapes."""
    if depth > _CONTEXT_MAX_DEPTH:
        raise EvidenceHeld(f"{path}: nesting exceeds {_CONTEXT_MAX_DEPTH}")
    if value is None or isinstance(value, bool):
        return 1
    if isinstance(value, int):
        if abs(value) > _CONTEXT_MAX_INT:
            raise EvidenceHeld(f"{path}: integer out of bounded range")
        return 1
    if isinstance(value, float):
        raise EvidenceHeld(
            f"{path}: floats are outside the schema-2 typed projection")
    if isinstance(value, str):
        if _DIGEST_RE.match(value) or _IDENTITY_RE.match(value):
            return 1
        raise EvidenceHeld(
            f"{path}: only 64-hex digests or W-/V-/C-/U-/L-/G- identities "
            "are allowed as context strings; free text is refused (pass a "
            "canonical digest or an unbound ref instead)")
    if isinstance(value, list):
        nodes = 1
        for i, item in enumerate(value):
            nodes += _validate_context_value(item, f"{path}[{i}]",
                                             depth + 1)
            if nodes > _CONTEXT_MAX_NODES:
                raise EvidenceHeld(
                    f"{path}: exceeds {_CONTEXT_MAX_NODES} nodes")
        return nodes
    if isinstance(value, dict):
        nodes = 1
        for key, item in value.items():
            if not isinstance(key, str) or (key not in _CONTEXT_KEYS
                    and not re.fullmatch(r"[0-9a-f]{16}|[0-9a-f]{64}", key)):
                raise EvidenceHeld(f"{path}: invalid key")
            nodes += _validate_context_value(item, f"{path}.{key}",
                                             depth + 1)
            if nodes > _CONTEXT_MAX_NODES:
                raise EvidenceHeld(
                    f"{path}: exceeds {_CONTEXT_MAX_NODES} nodes")
        return nodes
    raise EvidenceHeld(f"{path}: unrepresentable type {type(value).__name__}")


def _validate_context(context, name="context"):
    """Top-level context shape: a nonempty dict of typed metadata, or
    None for no context."""
    if context is None:
        return None
    if not isinstance(context, dict) or not context:
        raise EvidenceHeld(f"{name} must be a nonempty dict or None")
    _validate_context_value(context, name)
    return context


_SCHEMA_STATEMENTS = [
    """CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
)""",
    """CREATE TABLE IF NOT EXISTS windows (
  window_id TEXT PRIMARY KEY,
  source_kind TEXT NOT NULL,
  lineage_id TEXT NOT NULL,
  generation TEXT NOT NULL,
  start INTEGER NOT NULL,
  end INTEGER NOT NULL,
  page_digest TEXT NOT NULL,
  raw_sha256 TEXT,
  created_at TEXT NOT NULL
)""",
    """CREATE TABLE IF NOT EXISTS contexts (
  context_id TEXT PRIMARY KEY,
  context_json TEXT NOT NULL,
  created_at TEXT NOT NULL
)""",
    """CREATE TABLE IF NOT EXISTS versions (
  version_id TEXT PRIMARY KEY,
  logical_key TEXT NOT NULL,
  window_id TEXT NOT NULL REFERENCES windows(window_id),
  context_id TEXT NOT NULL REFERENCES contexts(context_id),
  detector_id TEXT NOT NULL,
  detector_version TEXT NOT NULL,
  severity TEXT NOT NULL,
  status TEXT NOT NULL,
  span TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  created_at TEXT NOT NULL
)""",
    """CREATE TABLE IF NOT EXISTS version_refs (
  version_id TEXT NOT NULL REFERENCES versions(version_id),
  ref_ord INTEGER NOT NULL,
  ref_json TEXT NOT NULL,
  window_id TEXT,
  unbound_id TEXT,
  PRIMARY KEY (version_id, ref_ord)
)""",
    """CREATE TABLE IF NOT EXISTS legacy_records (
  legacy_id TEXT PRIMARY KEY,
  record_digest TEXT NOT NULL,
  raw_evidence TEXT NOT NULL,
  source_kind TEXT NOT NULL,
  lineage_id TEXT NOT NULL,
  generation TEXT NOT NULL,
  record_json TEXT NOT NULL,
  created_at TEXT NOT NULL
)""",
]


_SCHEMA_STATEMENTS += [
    "CREATE INDEX IF NOT EXISTS ix_versions_window ON versions(window_id)",
    "CREATE INDEX IF NOT EXISTS ix_versions_context ON versions(context_id)",
    "CREATE INDEX IF NOT EXISTS ix_version_refs_window ON version_refs(window_id)",
]


def _db_path(state_dir):
    return os.path.join(state_dir, DB_NAME)


def _now():
    import datetime
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def _readonly_connect(path):
    """Genuinely read-only SQLite open (URI mode=ro). A read/write handle
    with query_only is NOT a read-only open — its close can checkpoint an
    unknown store — so preflight and validation never use one.
    immutable=1 is never used: it would hide live WAL commits."""
    uri = "file:" + urllib.request.pathname2url(os.path.abspath(path)) \
        + "?mode=ro"
    try:
        conn = sqlite3.connect(uri, uri=True, timeout=10.0,
                               isolation_level=None)
        conn.execute("PRAGMA busy_timeout=10000")
    except sqlite3.Error as exc:
        raise EvidenceHeld(f"version store unreadable: {exc}") from exc
    return conn


def _writable_connect(path):
    """Read/write open for an ALREADY-VALIDATED store. Journal mode is
    only ever changed here — after the schema marker has validated — so
    an unknown store is never flipped to WAL or otherwise mutated."""
    try:
        conn = sqlite3.connect(path, timeout=10.0, isolation_level=None)
        conn.execute("PRAGMA busy_timeout=10000")
        conn.execute("PRAGMA journal_mode=WAL")
        conn.execute("PRAGMA synchronous=FULL")
    except sqlite3.Error as exc:
        raise EvidenceHeld(f"version store unreadable: {exc}") from exc
    return conn


def _read_schema_version(conn):
    normalize = lambda text: " ".join(text.replace("IF NOT EXISTS ", "").split()).lower()
    expected = {}
    for statement in _SCHEMA_STATEMENTS:
        kind, name = re.search(r"CREATE (TABLE|INDEX) IF NOT EXISTS (\w+)", statement).groups()
        expected[(kind.lower(), name)] = normalize(statement)
    try:
        objects = conn.execute("SELECT type, name, sql FROM sqlite_master "
                               "WHERE name NOT LIKE 'sqlite_%'").fetchall()
        if {(kind, name): normalize(sql or "") for kind, name, sql in objects} != expected:
            raise EvidenceHeld("version store table/column/dependency schema is unknown; held")
        row = conn.execute(
            "SELECT value FROM meta WHERE key='schema_version'").fetchone()
    except sqlite3.DatabaseError as exc:
        raise EvidenceHeld(f"version store unreadable: {exc}") from exc
    if row is None:
        raise EvidenceHeld("version store has no schema_version marker")
    return row[0]


def _readonly_validate(path):
    """Validate an existing store through a read-only connection.
    Unknown marker, missing marker, empty file, non-SQLite content or a
    future schema raises EvidenceHeld without changing existing durable
    evidence bytes. SQLite may create empty WAL/SHM coordination files."""
    conn = _readonly_connect(path)
    try:
        version = _read_schema_version(conn)
        if version != SCHEMA_VERSION:
            raise EvidenceHeld(
                f"version store schema {version!r} is unknown or future "
                f"(this build speaks {SCHEMA_VERSION!r}); held, not modified")
    finally:
        conn.close()


def _dsync_dir(state_dir):
    fd = os.open(state_dir, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def _genesis(state_dir):
    """Create and atomically publish a complete fresh store.

    Returns True when this call published the store; False when another
    writer won the publication race (the caller then validates theirs).
    The schema is built complete in a private temp file, fsynced, and
    published with a no-replace hardlink, so a crash mid-genesis can
    leave only an unreferenced temp file — never a partial store."""
    path = _db_path(state_dir)
    tmp = f"{path}.genesis-{os.getpid()}-{uuid.uuid4().hex}"
    created = False
    try:
        fd = os.open(tmp, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        created = True
        os.close(fd)
        conn = sqlite3.connect(tmp, isolation_level=None)
        try:
            conn.execute("PRAGMA journal_mode=DELETE")
            conn.execute("BEGIN IMMEDIATE")
            for statement in _SCHEMA_STATEMENTS:
                conn.execute(statement)
            conn.execute(
                "INSERT INTO meta(key, value) VALUES('schema_version', ?)",
                (SCHEMA_VERSION,))
            conn.execute("COMMIT")
        finally:
            conn.close()
        fd = os.open(tmp, os.O_RDONLY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
        try:
            os.link(tmp, path)  # namespace CAS: never replaces a winner
        except FileExistsError:
            return False
        _dsync_dir(state_dir)
        return True
    except (sqlite3.Error, OSError) as exc:
        raise EvidenceHeld(f"version store genesis fault: {exc}") from exc
    finally:
        if created and os.path.exists(tmp):
            os.unlink(tmp)


class _WriteSession:
    """flock-serialized write session: acquire the store lock, establish
    the store (genesis or read-only validation), then hand back a
    read/write connection. Any hold releases the lock and closes the
    connection without a durable change."""

    def __init__(self, state_dir):
        self.state_dir = state_dir
        self.lock_fd = None
        self.conn = None

    def __enter__(self):
        path = _db_path(self.state_dir)
        # A pre-existing unknown DB must hold before even creating our lock.
        # Recheck after locking too; the first read does not grant write scope.
        if os.path.lexists(path):
            if os.path.islink(path) or not os.path.isfile(path):
                raise EvidenceHeld("version store is linked or nonregular; held")
            _readonly_validate(path)
        os.makedirs(self.state_dir, mode=0o700, exist_ok=True)
        lock_path = os.path.join(self.state_dir, LOCK_NAME)
        self.lock_fd = os.open(lock_path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        deadline = time.monotonic() + _LOCK_TIMEOUT_S
        while True:
            try:
                fcntl.flock(self.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except OSError:
                if time.monotonic() > deadline:
                    self._release()
                    raise EvidenceHeld(
                        "version store writer lock busy; held")
                time.sleep(0.02)
        try:
            path = _db_path(self.state_dir)
            if os.path.exists(path):
                _readonly_validate(path)
            else:
                if os.path.exists(path + "-wal") or \
                        os.path.exists(path + "-shm"):
                    raise EvidenceHeld(
                        "store file absent but SQLite sidecars exist: "
                        "absence cannot be proven; held, nothing initialized")
                if not _genesis(self.state_dir):
                    _readonly_validate(path)  # lost publication race
            try:
                _dsync_dir(self.state_dir)
            except OSError as exc:
                raise EvidenceHeld("store namespace is visible but its "
                                   "durability is unconfirmed; held for replay") from exc
            self.conn = _writable_connect(path)
            return self.conn
        except Exception:
            self._close_conn()
            self._release()
            raise

    def _close_conn(self):
        if self.conn is not None:
            try:
                self.conn.close()
            except sqlite3.Error:
                pass
            self.conn = None

    def _release(self):
        if self.lock_fd is not None:
            try:
                fcntl.flock(self.lock_fd, fcntl.LOCK_UN)
            except OSError:
                pass
            os.close(self.lock_fd)
            self.lock_fd = None

    def __exit__(self, exc_type, exc, tb):
        self._close_conn()
        self._release()
        return False


def preflight(state_dir):
    """Classify storage/schema without initialization or fact writes.

    Returns {"status": "absent", "schema_version": None} when no store
    exists; {"status": "ready", "schema_version": SCHEMA_VERSION} for a
    readable current-schema store. Corrupt, unreadable or unknown/future
    stores raise EvidenceHeld without changing existing durable evidence.
    Read-only URI opens can create empty WAL/SHM coordination files; no
    schema initialization or fact write is performed by preflight."""
    path = _db_path(state_dir)
    if not os.path.exists(path):
        return {"status": "absent", "schema_version": None}
    conn = _readonly_connect(path)
    try:
        version = _read_schema_version(conn)
        if version != SCHEMA_VERSION:
            raise EvidenceHeld(
                f"version store schema {version!r} is unknown or future "
                f"(this build speaks {SCHEMA_VERSION!r}); held, not modified")
        return {"status": "ready", "schema_version": version}
    finally:
        conn.close()


def _validate_source(source):
    if not isinstance(source, dict) or set(source) != set(_SOURCE_FIELDS):
        raise EvidenceHeld(
            f"source must have exactly {sorted(_SOURCE_FIELDS)}")
    for key in _SOURCE_FIELDS:
        _require_nonempty_str(source[key], f"source.{key}")
    return {key: source[key] for key in _SOURCE_FIELDS
            if key != "project"}  # project is a path: validated, never stored


def _validate_ref(ref):
    if not isinstance(ref, dict):
        raise EvidenceHeld("ref must be a dict")
    kind = ref.get("kind")
    if kind == "window":
        allowed = {"kind", "window_id", "expected_digest"}
        if not set(ref) <= allowed or "window_id" not in ref:
            raise EvidenceHeld(f"window ref fields invalid: {sorted(ref)}")
        _require_nonempty_str(ref["window_id"], "ref.window_id")
        if not ref["window_id"].startswith("W-"):
            raise EvidenceHeld("ref.window_id must be a W- identity")
        if "expected_digest" in ref:
            digest = ref["expected_digest"]
            if not isinstance(digest, str) or len(digest) != 64:
                raise EvidenceHeld("ref.expected_digest must be 64 hex chars")
        encoding = {k: ref[k] for k in sorted(ref)}
        return encoding, ref["window_id"], None
    if kind == "unbound":
        if set(ref) != {"kind", "unbound_value"}:
            raise EvidenceHeld("unbound ref must be exactly "
                               "{kind, unbound_value}")
        value = ref["unbound_value"]
        if isinstance(value, str):
            value_bytes = value.encode("utf-8")
        elif isinstance(value, (bytes, bytearray)):
            value_bytes = bytes(value)
        else:
            raise EvidenceHeld("unbound_value must be str or bytes")
        if not value_bytes:
            raise EvidenceHeld("unbound_value must be nonempty")
        unbound_id = "U-" + _sha_hex(value_bytes)
        encoding = {"kind": "unbound", "unbound_id": unbound_id}
        return encoding, None, unbound_id
    raise EvidenceHeld(f"ref kind must be 'window' or 'unbound', got {kind!r}")


def _validate_observation(obs):
    """Closed observation schema. Returns the validated record."""
    if not isinstance(obs, dict):
        raise EvidenceHeld("observation must be a dict")
    allowed = set(_OBS_FIELDS) | set(_OBS_OPTIONAL)
    missing = allowed - set(obs)
    extra = set(obs) - allowed
    if missing or extra:
        raise EvidenceHeld(
            f"observation schema violation: missing={sorted(missing)} "
            f"extra={sorted(extra)}")
    record = {}
    for key in _OBS_FIELDS:
        _require_nonempty_str(obs[key], f"observation.{key}")
        record[key] = obs[key]
    if record["severity"] not in _SEVERITIES:
        raise EvidenceHeld(
            f"invalid severity {record['severity']!r}")
    if record["status"] not in _STATUSES:
        raise EvidenceHeld(f"invalid status {record['status']!r}")
    if not re.fullmatch(r"bytes@(0|[1-9][0-9]*)", record["span"]):
        raise EvidenceHeld("observation.span must be a canonical raw byte offset")
    refs = obs.get("refs")
    if refs is None:
        refs = []
    if not isinstance(refs, list):
        raise EvidenceHeld("observation.refs must be a list")
    parsed_refs = [_validate_ref(ref) for ref in refs]
    context = _validate_context(obs.get("context"), "observation.context")
    record["refs"] = parsed_refs
    record["context"] = context
    return record


def _logical_key(source_valid, obs):
    return _identity("L-", [source_valid["source_kind"],
                            source_valid["lineage_id"], obs["span"],
                            obs["detector_id"], obs["detector_version"],
                            obs["fingerprint"]])


def _row_matches(conn, table, key_column, key, columns, values):
    row = conn.execute(
        f"SELECT {', '.join(columns)} FROM {table} "
        f"WHERE {key_column} = ?", (key,)).fetchone()
    if row is None:
        return True  # will insert
    stored = dict(zip(columns, row))
    if stored != dict(zip(columns, values)):
        raise EvidenceHeld(
            f"contradictory payload for existing {table} id {key}: "
            "held, never overwritten")
    return False


def _fetch_window_validated(conn, window_id):
    """Load one stored window and recompute its content-addressed
    identity from the persisted payload. Never trusts the stored id or
    cached digests: any tampering contradicts and holds."""
    row = conn.execute(
        f"SELECT {', '.join(_WINDOW_COLS)} FROM windows "
        "WHERE window_id = ?", (window_id,)).fetchone()
    if row is None:
        return None
    stored = dict(zip(_WINDOW_COLS, row))
    recomputed = _identity("W-", [stored["source_kind"],
                                  stored["lineage_id"],
                                  stored["generation"], stored["start"],
                                  stored["end"], stored["page_digest"]])
    if recomputed != stored["window_id"]:
        raise EvidenceHeld(
            f"stored window {window_id} fails identity recomputation from "
            "its persisted payload: store contradiction, held")
    if stored["page_digest"] == _PAGE_DIGEST_UNKNOWN:
        if stored["raw_sha256"] is not None:
            raise EvidenceHeld(
                f"stored window {window_id} carries a raw hash under the "
                "UNKNOWN gap sentinel: store contradiction, held")
    elif stored["raw_sha256"] != stored["page_digest"]:
        raise EvidenceHeld(
            f"stored window {window_id} cached digests disagree: "
            "store contradiction, held")
    return stored


def _resolve_window_ref(conn, encoding, ref_window):
    """Resolve a caller window ref against the durable store. The
    referenced window must exist, must recompute from its persisted
    payload, and any supplied expected_digest must match the recomputed
    stored digest."""
    stored = _fetch_window_validated(conn, ref_window)
    if stored is None:
        raise EvidenceHeld(
            f"ref window {ref_window} is not durably recorded: held, "
            "no invented prior raw content")
    if "expected_digest" in encoding and \
            encoding["expected_digest"] != stored["page_digest"]:
        raise EvidenceHeld(
            f"ref window {ref_window} digest contradiction: "
            f"expected {encoding['expected_digest']}, "
            f"stored {stored['page_digest']}")
    return stored


def _validate_carried_binding(conn, context, all_refs, own_window):
    """Named carried facts must agree on the same source evidence window."""
    if not any(key in context for key in _CARRIED_KEYS):
        return
    unbound = any(unbound_id is not None for _encoding, _wid, unbound_id in all_refs)
    windows = {}
    for _encoding, wid, _unbound_id in all_refs:
        if wid is None:
            continue
        window = _fetch_window_validated(conn, wid)
        if window and window["page_digest"] != _PAGE_DIGEST_UNKNOWN \
                and window["source_kind"] == own_window["source_kind"] \
                and window["lineage_id"] == own_window["lineage_id"]:
            windows[wid] = window
    offset = context.get("prior_offset")
    digest = context.get("prior_page_digest")
    named = context.get("prior_window_id")
    if offset is not None and (type(offset) is not int or offset < 0):
        raise EvidenceHeld("context.prior_offset must be a nonnegative int")
    if digest is not None and (not isinstance(digest, str) or not _DIGEST_RE.fullmatch(digest)):
        raise EvidenceHeld("context.prior_page_digest must be a canonical byte digest")
    def matches(window):
        return (offset is None or window["start"] <= offset < window["end"]) \
            and (digest is None or window["page_digest"] == digest)
    if named is not None:
        if named not in windows or not matches(windows[named]):
            raise EvidenceHeld("named carried window is absent, foreign, or "
                               "contradicts its declared offset/digest")
        return
    if any(matches(window) for window in windows.values()):
        return
    if unbound:
        return  # explicit UNKNOWN; named contradictory facts were refused above
    raise EvidenceHeld("carried context has no matching same-source durable "
                       "window; explicit unbound evidence is required")


def _validate_observation_span(conn, source, observation, refs):
    """An evidence window cannot impersonate bytes at another span."""
    offset = int(observation["span"].split("@", 1)[1])
    for _encoding, window_id, _unbound_id in refs:
        if window_id is None:
            continue
        window = _fetch_window_validated(conn, window_id)
        if window is not None and window["page_digest"] != _PAGE_DIGEST_UNKNOWN \
                and window["source_kind"] == source["source_kind"] \
                and window["lineage_id"] == source["lineage_id"] \
                and window["start"] <= offset < window["end"]:
            return
    if any(unbound_id is not None for _encoding, _window, unbound_id in refs):
        return  # explicit UNKNOWN; current page does not contain this occurrence
    raise EvidenceHeld("observation span is outside every same-source evidence "
                       "window; a matching prior window or explicit unbound "
                       "evidence reference is required")


def _ensure_context_row(conn, context_id, context, created, now):
    if conn.execute("SELECT 1 FROM contexts WHERE context_id=?", (context_id,)).fetchone() is None \
            and conn.execute("SELECT 1 FROM versions WHERE context_id=? LIMIT 1", (context_id,)).fetchone():
        raise EvidenceHeld("existing version references a missing context; "
                           "replay cannot recreate lost provenance")
    """Insert the contexts row for one context identity once per call.
    The stored JSON must round-trip to the same identity on replay."""
    if _row_matches(conn, "contexts", "context_id", context_id,
                    ["context_json"], [_canonical(context)]):
        conn.execute(
            "INSERT INTO contexts(context_id, context_json, created_at) "
            "VALUES(?,?,?)",
            (context_id, _canonical(context), now))
        created["contexts"] += 1


def _validate_stored_version(conn, version_id, expect_refs):
    """Full dependency-graph validation of an already-stored version:
    persisted refs must exactly equal the refs this call derives, the V
    identity must recompute from stored parts, the context payload must
    round-trip to its identity, the logical key must recompute from the
    persisted source/observation fields, and the referenced window must
    recompute from its payload. Contradiction or absence holds."""
    stored_refs = [
        (row[1], row[2], row[3]) for row in conn.execute(
            "SELECT ref_ord, ref_json, window_id, unbound_id "
            "FROM version_refs WHERE version_id = ? ORDER BY ref_ord",
            (version_id,))]
    if sorted(stored_refs, key=lambda item: item[0]) != sorted(expect_refs, key=lambda item: item[0]):
        raise EvidenceHeld(
            f"stored version {version_id} dependency refs do not match "
            "the replay (missing, altered or extra child refs): "
            "contradiction, held")
    row = conn.execute(
        "SELECT logical_key, window_id, context_id, detector_id, "
        "detector_version, severity, status, span, fingerprint "
        "FROM versions WHERE version_id = ?", (version_id,)).fetchone()
    if row is None:
        raise EvidenceHeld(
            f"stored version {version_id} has refs but no row: "
            "store contradiction, held")
    stored = dict(zip(["logical_key", "window_id", "context_id",
                       "detector_id", "detector_version", "severity",
                       "status", "span", "fingerprint"], row))
    recomputed_v = _identity(
        "V-", [stored["logical_key"], sorted(r[0] for r in stored_refs),
               stored["context_id"]])
    if recomputed_v != version_id:
        raise EvidenceHeld(
            f"stored version {version_id} fails identity recomputation "
            "from its persisted payload: store contradiction, held")
    context_row = conn.execute(
        "SELECT context_json FROM contexts WHERE context_id = ?",
        (stored["context_id"],)).fetchone()
    if context_row is None:
        raise EvidenceHeld(
            f"stored version {version_id} references missing context "
            f"{stored['context_id']}: store contradiction, held")
    try:
        context_value = json.loads(context_row[0])
    except ValueError as exc:
        raise EvidenceHeld(
            f"stored context {stored['context_id']} is not JSON: "
            "store contradiction, held") from exc
    if _identity("C-", context_value) != stored["context_id"]:
        raise EvidenceHeld(
            f"stored context {stored['context_id']} does not round-trip "
            "to its identity: store contradiction, held")
    window = _fetch_window_validated(conn, stored["window_id"])
    if window is None:
        raise EvidenceHeld(
            f"stored version {version_id} references missing window "
            f"{stored['window_id']}: store contradiction, held")
    recomputed_l = _identity(
        "L-", [window["source_kind"], window["lineage_id"],
               stored["span"], stored["detector_id"],
               stored["detector_version"], stored["fingerprint"]])
    if recomputed_l != stored["logical_key"]:
        raise EvidenceHeld(
            f"stored version {version_id} logical key fails "
            "recomputation from persisted fields: store contradiction, "
            "held")


def record_page(state_dir, *, source, start, raw, context, observations):
    """Record one consumed page window plus its observation versions.

    raw is the exact page bytes (bytes) or None for a typed gap (oversized
    or otherwise uncaptured page — the window fact is still recorded with
    the UNKNOWN content sentinel, never a fabricated digest). Context has
    ONE authoritative source per observation: the observation's own
    ``context`` when present, else the call's top-level ``context``;
    both present for one observation is refused. Every insert happens in
    one transaction under the store lock; any fault raises EvidenceHeld
    with the store rolled back, so a held call never authorizes cursor
    advance. Returns a plain receipt:
      {"window_id", "version_ids", "created_counts", "unknown_evidence"}.
    """
    if not isinstance(start, int) or isinstance(start, bool) or start < 0:
        raise EvidenceHeld("start must be a nonnegative int")
    if raw is not None and not isinstance(raw, (bytes, bytearray)):
        raise EvidenceHeld("raw must be bytes or None")
    raw = bytes(raw) if raw is not None else None
    if not isinstance(observations, list):
        raise EvidenceHeld("observations must be a list")
    context = _validate_context(context)
    source_valid = _validate_source(source)
    parsed = [_validate_observation(obs) for obs in observations]

    # One authoritative context source per observation, decided before
    # any I/O: per-observation context wins; combining both is refused.
    effective_contexts = []
    for index, obs in enumerate(parsed):
        if obs["context"] is not None and context is not None:
            raise EvidenceHeld(
                f"observation {index}: both observation.context and the "
                "top-level context are present; context has one "
                "authoritative source and the combination is refused")
        effective_contexts.append(
            obs["context"] if obs["context"] is not None else context)

    if raw is not None:
        page_digest = _sha_hex(raw)
    else:
        page_digest = _PAGE_DIGEST_UNKNOWN
    window_key_obj = [source_valid["source_kind"],
                      source_valid["lineage_id"], source_valid["generation"],
                      start, start + (len(raw) if raw is not None else 0),
                      page_digest]
    window_id = _identity("W-", window_key_obj)
    end = start + (len(raw) if raw is not None else 0)

    with _WriteSession(state_dir) as conn:
        try:
            conn.execute("BEGIN IMMEDIATE")

            # Inspect every existing candidate V before adding dependencies.
            # A replay must not silently recreate deleted W/C provenance.
            for obs, eff_ctx in zip(parsed, effective_contexts):
                logical = _logical_key(source_valid, obs)
                cid = _identity("C-", eff_ctx)
                refs = [({"kind": "window", "window_id": window_id}, window_id, None)] + obs["refs"]
                expected_refs = sorted([(_canonical(enc), wid, uid) for enc, wid, uid in refs],
                                       key=lambda item: item[0])
                vid = _identity("V-", [logical, [item[0] for item in expected_refs], cid])
                if conn.execute("SELECT 1 FROM versions WHERE version_id=?", (vid,)).fetchone():
                    _validate_stored_version(conn, vid, expected_refs)
            if conn.execute("SELECT 1 FROM windows WHERE window_id=?", (window_id,)).fetchone() is None:
                if conn.execute("SELECT 1 FROM versions WHERE window_id=? LIMIT 1", (window_id,)).fetchone() \
                        or conn.execute("SELECT 1 FROM version_refs WHERE window_id=? LIMIT 1", (window_id,)).fetchone():
                    raise EvidenceHeld("existing evidence references a missing window; "
                                       "capture cannot silently recreate its provenance")
            created = {"windows": 0, "contexts": 0, "versions": 0}
            now = _now()
            raw_sha = page_digest if raw is not None else None
            if _row_matches(conn, "windows", "window_id", window_id,
                            ["source_kind", "lineage_id", "generation",
                             "start", "end", "page_digest", "raw_sha256"],
                            [source_valid["source_kind"],
                             source_valid["lineage_id"],
                             source_valid["generation"], start, end,
                             page_digest, raw_sha]):
                conn.execute(
                    "INSERT INTO windows(window_id, source_kind, lineage_id, "
                    "generation, start, end, page_digest, raw_sha256, "
                    "created_at) VALUES(?,?,?,?,?,?,?,?,?)",
                    (window_id, source_valid["source_kind"],
                     source_valid["lineage_id"], source_valid["generation"],
                     start, end, page_digest, raw_sha, now))
                created["windows"] = 1
            own_window = _fetch_window_validated(conn, window_id)
            if own_window is None:
                raise EvidenceHeld(
                    f"current window {window_id} absent after insert: "
                    "store fault, held")

            unknown_evidence = []
            if raw is None:
                unknown_evidence.append(
                    {"kind": "raw-gap", "window_id": window_id,
                     "reason": "raw is None: page content uncaptured"})

            version_ids = []
            ensured_contexts = set()
            for obs, eff_ctx in zip(parsed, effective_contexts):
                logical_key = _logical_key(source_valid, obs)
                # E is every window whose bytes this version actually
                # observed: the current page's own window plus each
                # explicit caller ref.
                own_ref = {"kind": "window", "window_id": window_id}
                all_refs = [(own_ref, window_id, None)] + obs["refs"]
                ref_encodings = []
                expect_refs = []
                for encoding, ref_window, unbound_id in all_refs:
                    ref_encodings.append(_canonical(encoding))
                    expect_refs.append(
                        (_canonical(encoding), ref_window, unbound_id))
                    if ref_window is not None:
                        _resolve_window_ref(conn, encoding, ref_window)
                if eff_ctx is not None:
                    _validate_carried_binding(conn, eff_ctx, all_refs,
                                              own_window)
                _validate_observation_span(conn, source_valid, obs, all_refs)
                context_id = _identity("C-", eff_ctx)
                expect_refs.sort(key=lambda item: item[0])
                if context_id not in ensured_contexts:
                    _ensure_context_row(conn, context_id, eff_ctx,
                                        created, now)
                    ensured_contexts.add(context_id)
                sorted_refs = sorted(ref_encodings)
                version_id = _identity(
                    "V-", [logical_key, sorted_refs, context_id])
                if _row_matches(
                        conn, "versions", "version_id", version_id,
                        ["logical_key", "window_id", "context_id",
                         "detector_id", "detector_version", "severity",
                         "status", "span", "fingerprint"],
                        [logical_key, window_id, context_id,
                         obs["detector_id"], obs["detector_version"],
                         obs["severity"], obs["status"], obs["span"],
                         obs["fingerprint"]]):
                    conn.execute(
                        "INSERT INTO versions(version_id, logical_key, "
                        "window_id, context_id, detector_id, "
                        "detector_version, severity, status, span, "
                        "fingerprint, created_at) "
                        "VALUES(?,?,?,?,?,?,?,?,?,?,?)",
                        (version_id, logical_key, window_id, context_id,
                         obs["detector_id"], obs["detector_version"],
                         obs["severity"], obs["status"], obs["span"],
                         obs["fingerprint"], now))
                    created["versions"] += 1
                    for ord_, (ref_json, ref_window, unbound_id) in \
                            enumerate(expect_refs):
                        conn.execute(
                            "INSERT INTO version_refs(version_id, ref_ord, "
                            "ref_json, window_id, unbound_id) "
                            "VALUES(?,?,?,?,?)",
                            (version_id, ord_, ref_json, ref_window,
                             unbound_id))
                else:
                    # Replay: recompute the full persisted dependency
                    # graph before accepting the reference.
                    _validate_stored_version(conn, version_id, expect_refs)
                version_ids.append(version_id)
                for _enc, _rw, unbound_id in obs["refs"]:
                    if unbound_id is not None:
                        unknown_evidence.append(
                            {"kind": "unbound-context",
                             "unbound_id": unbound_id,
                             "reason": "carried context without "
                                       "source-byte binding"})

            conn.execute("COMMIT")
        except EvidenceHeld:
            try:
                conn.execute("ROLLBACK")
            except sqlite3.Error:
                pass
            raise
        except (sqlite3.Error, OSError) as exc:
            try:
                conn.execute("ROLLBACK")
            except sqlite3.Error:
                pass
            raise EvidenceHeld(f"version store fault: {exc}") from exc

    return {"window_id": window_id, "version_ids": version_ids,
            "created_counts": created, "unknown_evidence": unknown_evidence}


_LEGACY_FIELDS = ("detector_id", "detector_version", "severity", "status",
                  "source_kind", "generation", "span", "fingerprint", "at",
                  "dedupe_key", "lineage_id")


def record_legacy(state_dir, rows):
    """Seed historical normalized records that lack original byte evidence.

    Each row must match the composer's closed eleven-field legacy shape
    (shape only — a v1 row may carry a v1 dedupe key). The record digest is
    the digest of the serialized normalized fields and is explicitly NOT a
    raw-source hash; raw_evidence is the constant 'unavailable'. Returns a
    receipt with created_counts and one typed unknown per row."""
    if not isinstance(rows, list):
        raise EvidenceHeld("rows must be a list")
    parsed = []
    for row in rows:
        if not isinstance(row, dict) or set(row) != set(_LEGACY_FIELDS):
            missing = set(_LEGACY_FIELDS) - set(row or {})
            extra = set(row or {}) - set(_LEGACY_FIELDS)
            raise EvidenceHeld(
                f"legacy row schema violation: missing={sorted(missing)} "
                f"extra={sorted(extra)}")
        for key in _LEGACY_FIELDS:
            _require_nonempty_str(row[key], f"legacy.{key}")
        if row["severity"] not in _SEVERITIES:
            raise EvidenceHeld(f"invalid severity {row['severity']!r}")
        if row["status"] not in _STATUSES:
            raise EvidenceHeld(f"invalid status {row['status']!r}")
        parsed.append(row)

    with _WriteSession(state_dir) as conn:
        try:
            conn.execute("BEGIN IMMEDIATE")
            created = 0
            now = _now()
            unknown_evidence = []
            for row in parsed:
                record_json = _canonical(row)
                legacy_id = "G-" + _sha_hex(record_json.encode("utf-8"))
                columns = ["record_digest", "raw_evidence", "source_kind",
                           "lineage_id", "generation", "record_json"]
                values = [legacy_id[2:], _RAW_EVIDENCE_UNAVAILABLE,
                          row["source_kind"], row["lineage_id"],
                          row["generation"], record_json]
                if _row_matches(conn, "legacy_records", "legacy_id",
                                legacy_id, columns, values):
                    conn.execute(
                        "INSERT INTO legacy_records(legacy_id, "
                        "record_digest, raw_evidence, source_kind, "
                        "lineage_id, generation, record_json, created_at) "
                        "VALUES(?,?,?,?,?,?,?,?)",
                        (legacy_id, legacy_id[2:],
                         _RAW_EVIDENCE_UNAVAILABLE, row["source_kind"],
                         row["lineage_id"], row["generation"],
                         record_json, now))
                    created += 1
                unknown_evidence.append(
                    {"kind": "legacy-raw-unavailable",
                     "legacy_id": legacy_id,
                     "reason": "normalized-record digest only; not a "
                               "raw-source hash; original bytes unavailable"})
            conn.execute("COMMIT")
        except EvidenceHeld:
            try:
                conn.execute("ROLLBACK")
            except sqlite3.Error:
                pass
            raise
        except (sqlite3.Error, OSError) as exc:
            try:
                conn.execute("ROLLBACK")
            except sqlite3.Error:
                pass
            raise EvidenceHeld(f"version store fault: {exc}") from exc

    return {"created_counts": {"legacy_records": created},
            "unknown_evidence": unknown_evidence}


def read_summary(state_dir, limit=20):
    """Bounded metadata for verification. Counts plus up to `limit` recent
    window and version rows. Contains no raw page, prompt, token or
    credential text; counts are storage facts, never incident counts."""
    if not isinstance(limit, int) or isinstance(limit, bool) or limit < 1:
        raise EvidenceHeld("limit must be a positive int")
    limit = min(limit, _SUMMARY_LIMIT_MAX)
    path = _db_path(state_dir)
    if not os.path.exists(path):
        raise EvidenceHeld("version store absent")
    conn = _readonly_connect(path)
    try:
        version = _read_schema_version(conn)
        if version != SCHEMA_VERSION:
            raise EvidenceHeld(
                f"version store schema {version!r} is unknown or future "
                f"(this build speaks {SCHEMA_VERSION!r}); held, not modified")
        counts = {}
        for table in ("windows", "contexts", "versions", "version_refs",
                      "legacy_records"):
            counts[table] = conn.execute(
                f"SELECT COUNT(*) FROM {table}").fetchone()[0]
        windows = [dict(zip(
            ["window_id", "source_kind", "lineage_id", "generation",
             "start", "end", "page_digest"],
            row)) for row in conn.execute(
            "SELECT window_id, source_kind, lineage_id, generation, "
            "start, end, page_digest FROM windows "
            "ORDER BY created_at DESC, window_id LIMIT ?", (limit,))]
        versions = [dict(zip(
            ["version_id", "logical_key", "window_id", "detector_id",
             "span", "fingerprint"],
            row)) for row in conn.execute(
            "SELECT version_id, logical_key, window_id, detector_id, "
            "span, fingerprint FROM versions "
            "ORDER BY created_at DESC, version_id LIMIT ?", (limit,))]
        legacy = conn.execute(
            "SELECT COUNT(*) FROM legacy_records "
            "WHERE raw_evidence = 'unavailable'").fetchone()[0]
        return {"status": "ready", "schema_version": version,
                "counts": counts,
                "legacy_raw_unavailable": legacy,
                "recent_windows": windows,
                "recent_versions": versions}
    except sqlite3.DatabaseError as exc:
        raise EvidenceHeld(f"version store unreadable: {exc}") from exc
    finally:
        conn.close()
