"""Standalone immutable evidence-version store (CSR09, schema 1).

Durable facts for composer evidence: every consumed raw page window W
(even when it yields no observation), every observation version V that
references durable windows plus a carried detector context C, and legacy
normalized records whose original bytes are unavailable. Identities are
SHA-256 of canonical JSON per docs/spec-composer-evidence-versions.md;
timestamps are provenance only and never identity.

The store is independent of composer.py: it computes the logical key L
from the same six elements the composer's v2 identity uses, so a caller
can map one onto the other without either module importing the other.

Storage: SQLite (stdlib) in WAL mode at <state_dir>/evidence-versions.db.
One BEGIN IMMEDIATE transaction per call makes concurrent writers safe;
WAL/SHM files are only ever touched by SQLite itself (we never move,
delete or checkpoint them manually). Corrupt, unreadable or unknown/
future-schema stores raise EvidenceHeld and are never modified; preflight
never initializes or writes anything.

Stored facts contain no raw page, prompt, transcript, credential or
token text: page content survives only as its digest, unbound carried
values survive only as their digest, and the census project field (a
path) is validated then dropped.
"""

import hashlib
import json
import os
import sqlite3

SCHEMA_VERSION = "1"
DB_NAME = "evidence-versions.db"

_PAGE_DIGEST_UNKNOWN = "UNKNOWN"       # typed gap: raw is None, never a fake hash
_RAW_EVIDENCE_UNAVAILABLE = "unavailable"  # legacy rows: typed constant
_SEVERITIES = ("high", "medium", "low")
_STATUSES = ("verified-symptom", "unverified-symptom")

_OBS_FIELDS = ("detector_id", "detector_version", "severity", "status",
               "span", "fingerprint")
_OBS_OPTIONAL = ("refs", "context")

_SOURCE_FIELDS = ("project", "source_kind", "lineage_id", "generation")

_CONTEXT_MAX_DEPTH = 4
_CONTEXT_MAX_NODES = 64
_CONTEXT_MAX_STRING = 512
_SUMMARY_LIMIT_MAX = 200


class EvidenceHeld(Exception):
    """An unknown, corrupt, contradictory or storage-faulted evidence
    condition. The call made no durable change; the caller must hold
    (never advance a cursor or claim success) on this."""


def _canonical(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=False)


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
    """Bounded JSON only: dict/list/str/int/float/bool/None, capped depth,
    node count and string length. No arbitrary body serialization."""
    if depth > _CONTEXT_MAX_DEPTH:
        raise EvidenceHeld(f"{path}: nesting exceeds {_CONTEXT_MAX_DEPTH}")
    if value is None or isinstance(value, (bool, int, float)):
        return 1
    if isinstance(value, str):
        if len(value) > _CONTEXT_MAX_STRING:
            raise EvidenceHeld(
                f"{path}: string exceeds {_CONTEXT_MAX_STRING} chars")
        return 1
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
            if not isinstance(key, str) or not key or len(key) > 128:
                raise EvidenceHeld(f"{path}: invalid key")
            nodes += _validate_context_value(item, f"{path}.{key}",
                                             depth + 1)
            if nodes > _CONTEXT_MAX_NODES:
                raise EvidenceHeld(
                    f"{path}: exceeds {_CONTEXT_MAX_NODES} nodes")
        return nodes
    raise EvidenceHeld(f"{path}: unrepresentable type {type(value).__name__}")


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


def _db_path(state_dir):
    return os.path.join(state_dir, DB_NAME)


def _now():
    import datetime
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def _connect(state_dir, query_only=False):
    path = _db_path(state_dir)
    try:
        conn = sqlite3.connect(path, timeout=10.0, isolation_level=None)
        conn.execute("PRAGMA busy_timeout=10000")
        if query_only:
            conn.execute("PRAGMA query_only=1")
        else:
            conn.execute("PRAGMA journal_mode=WAL")
            conn.execute("PRAGMA synchronous=FULL")
    except sqlite3.Error as exc:
        raise EvidenceHeld(f"version store unreadable: {exc}") from exc
    return conn


def _read_schema_version(conn):
    try:
        row = conn.execute(
            "SELECT value FROM meta WHERE key='schema_version'").fetchone()
    except sqlite3.DatabaseError as exc:
        raise EvidenceHeld(f"version store unreadable: {exc}") from exc
    if row is None:
        raise EvidenceHeld("version store has no schema_version marker")
    return row[0]


def _expect_ready(state_dir, conn):
    version = _read_schema_version(conn)
    if version != SCHEMA_VERSION:
        raise EvidenceHeld(
            f"version store schema {version!r} is unknown or future "
            f"(this build speaks {SCHEMA_VERSION!r}); held, not modified")
    return version


def preflight(state_dir):
    """Classify the store without initializing or writing anything.

    Returns {"status": "absent", "schema_version": None} when no store
    exists; {"status": "ready", "schema_version": SCHEMA_VERSION} for a
    readable current-schema store. Corrupt, unreadable or unknown/future
    stores raise EvidenceHeld and are left byte-for-byte untouched."""
    path = _db_path(state_dir)
    if not os.path.exists(path):
        return {"status": "absent", "schema_version": None}
    conn = _connect(state_dir, query_only=True)
    try:
        version = _read_schema_version(conn)
        if version != SCHEMA_VERSION:
            raise EvidenceHeld(
                f"version store schema {version!r} is unknown or future "
                f"(this build speaks {SCHEMA_VERSION!r}); held, not modified")
        return {"status": "ready", "schema_version": version}
    finally:
        conn.close()


def _initialize(conn):
    # Individual executes, never executescript: executescript would commit
    # the surrounding BEGIN IMMEDIATE transaction and break call atomicity.
    for statement in _SCHEMA_STATEMENTS:
        conn.execute(statement)
    conn.execute(
        "INSERT OR IGNORE INTO meta(key, value) VALUES('schema_version', ?)",
        (SCHEMA_VERSION,))


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
    refs = obs.get("refs")
    if refs is None:
        refs = []
    if not isinstance(refs, list):
        raise EvidenceHeld("observation.refs must be a list")
    parsed_refs = [_validate_ref(ref) for ref in refs]
    context = obs.get("context")
    if context is not None:
        _validate_context_value(context)
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


def record_page(state_dir, *, source, start, raw, context, observations):
    """Record one consumed page window plus its observation versions.

    raw is the exact page bytes (bytes) or None for a typed gap (oversized
    or otherwise uncaptured page — the window fact is still recorded with
    the UNKNOWN content sentinel, never a fabricated digest). Every insert
    happens in one transaction; any fault raises EvidenceHeld with the
    store rolled back, so a held call never authorizes cursor advance.
    Returns a plain receipt:
      {"window_id", "version_ids", "created_counts", "unknown_evidence"}.
    """
    if not isinstance(start, int) or isinstance(start, bool) or start < 0:
        raise EvidenceHeld("start must be a nonnegative int")
    if raw is not None and not isinstance(raw, (bytes, bytearray)):
        raise EvidenceHeld("raw must be bytes or None")
    raw = bytes(raw) if raw is not None else None
    if not isinstance(observations, list):
        raise EvidenceHeld("observations must be a list")
    if context is not None:
        _validate_context_value(context)

    source_valid = _validate_source(source)
    parsed = [_validate_observation(obs) for obs in observations]
    os.makedirs(state_dir, exist_ok=True)

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

    conn = _connect(state_dir)
    try:
        conn.execute("BEGIN IMMEDIATE")
        _initialize(conn)
        _expect_ready(state_dir, conn)

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
                "generation, start, end, page_digest, raw_sha256, created_at) "
                "VALUES(?,?,?,?,?,?,?,?,?)",
                (window_id, source_valid["source_kind"],
                 source_valid["lineage_id"], source_valid["generation"],
                 start, end, page_digest, raw_sha, now))
            created["windows"] = 1

        unknown_evidence = []
        if raw is None:
            unknown_evidence.append(
                {"kind": "raw-gap", "window_id": window_id,
                 "reason": "raw is None: page content uncaptured"})

        version_ids = []
        for obs in parsed:
            logical_key = _logical_key(source_valid, obs)
            # E is every window whose bytes this version actually observed:
            # the current page's own window plus each explicit caller ref.
            own_ref = {"kind": "window", "window_id": window_id}
            all_refs = [(own_ref, window_id, None)] + obs["refs"]
            ref_encodings = []
            for encoding, ref_window, unbound_id in all_refs:
                ref_encodings.append(_canonical(encoding))
                if ref_window is not None and encoding is not own_ref:
                    row = conn.execute(
                        "SELECT page_digest FROM windows "
                        "WHERE window_id = ?", (ref_window,)).fetchone()
                    if row is None:
                        raise EvidenceHeld(
                            f"ref window {ref_window} is not durably "
                            "recorded: held, no invented prior raw content")
                    if "expected_digest" in encoding and \
                            encoding["expected_digest"] != row[0]:
                        raise EvidenceHeld(
                            f"ref window {ref_window} digest contradiction: "
                            f"expected {encoding['expected_digest']}, "
                            f"stored {row[0]}")
            sorted_refs = sorted(ref_encodings)
            context_id = None
            if parsed:
                context_id = _identity("C-", context)
                if _row_matches(conn, "contexts", "context_id", context_id,
                                ["context_json"], [_canonical(context)]):
                    conn.execute(
                        "INSERT INTO contexts(context_id, context_json, "
                        "created_at) VALUES(?,?,?)",
                        (context_id, _canonical(context), now))
                    created["contexts"] = 1
            version_id = _identity(
                "V-", [logical_key, sorted_refs, context_id])
            if _row_matches(
                    conn, "versions", "version_id", version_id,
                    ["logical_key", "window_id", "context_id", "detector_id",
                     "detector_version", "severity", "status", "span",
                     "fingerprint"],
                    [logical_key, window_id, context_id, obs["detector_id"],
                     obs["detector_version"], obs["severity"], obs["status"],
                     obs["span"], obs["fingerprint"]]):
                conn.execute(
                    "INSERT INTO versions(version_id, logical_key, "
                    "window_id, context_id, detector_id, detector_version, "
                    "severity, status, span, fingerprint, created_at) "
                    "VALUES(?,?,?,?,?,?,?,?,?,?,?)",
                    (version_id, logical_key, window_id, context_id,
                     obs["detector_id"], obs["detector_version"],
                     obs["severity"], obs["status"], obs["span"],
                     obs["fingerprint"], now))
                created["versions"] += 1
                for ord_, (encoding, ref_window, unbound_id) in \
                        enumerate(all_refs):
                    conn.execute(
                        "INSERT INTO version_refs(version_id, ref_ord, "
                        "ref_json, window_id, unbound_id) VALUES(?,?,?,?,?)",
                        (version_id, ord_, _canonical(encoding),
                         ref_window, unbound_id))
            version_ids.append(version_id)
            for _enc, _rw, unbound_id in obs["refs"]:
                if unbound_id is not None:
                    unknown_evidence.append(
                        {"kind": "unbound-context", "unbound_id": unbound_id,
                         "reason": "carried context without source-byte "
                                   "binding"})

        conn.execute("COMMIT")
    except EvidenceHeld:
        try:
            conn.execute("ROLLBACK")
        except sqlite3.Error:
            pass
        conn.close()
        raise
    except (sqlite3.Error, OSError) as exc:
        try:
            conn.execute("ROLLBACK")
        except sqlite3.Error:
            pass
        conn.close()
        raise EvidenceHeld(f"version store fault: {exc}") from exc
    conn.close()
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

    os.makedirs(state_dir, exist_ok=True)
    conn = _connect(state_dir)
    try:
        conn.execute("BEGIN IMMEDIATE")
        _initialize(conn)
        _expect_ready(state_dir, conn)
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
            if _row_matches(conn, "legacy_records", "legacy_id", legacy_id,
                            columns, values):
                conn.execute(
                    "INSERT INTO legacy_records(legacy_id, record_digest, "
                    "raw_evidence, source_kind, lineage_id, generation, "
                    "record_json, created_at) VALUES(?,?,?,?,?,?,?,?)",
                    (legacy_id, legacy_id[2:], _RAW_EVIDENCE_UNAVAILABLE,
                     row["source_kind"], row["lineage_id"],
                     row["generation"], record_json, now))
                created += 1
            unknown_evidence.append(
                {"kind": "legacy-raw-unavailable", "legacy_id": legacy_id,
                 "reason": "normalized-record digest only; not a raw-source "
                           "hash; original bytes unavailable"})
        conn.execute("COMMIT")
    except EvidenceHeld:
        try:
            conn.execute("ROLLBACK")
        except sqlite3.Error:
            pass
        conn.close()
        raise
    except (sqlite3.Error, OSError) as exc:
        try:
            conn.execute("ROLLBACK")
        except sqlite3.Error:
            pass
        conn.close()
        raise EvidenceHeld(f"version store fault: {exc}") from exc
    conn.close()
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
    conn = _connect(state_dir, query_only=True)
    try:
        version = _expect_ready(state_dir, conn)
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
