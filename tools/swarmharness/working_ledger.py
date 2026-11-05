#!/usr/bin/env python3
"""working_ledger.py — standalone working-ledger preview slice.

Append-only event store plus current-entry projection on local SQLite.
Independent architectural preview per PAPER-PROPOSAL.md: this is NOT an
installed ygg ledger, NOT runtime-qualified, NOT "4.0 ready". No daemon,
scheduler, network/RPC, model invocation, GC/compaction, checkpointing,
or standing-law promotion happens here.

Slice guarantees:
- Stable ledger identity (UUID + human slug) stored inside the store, so
  same-host logical-root relocation preserves identity.
- Immutable appended events; mutations carry a caller-known operation UUID
  bound by canonical JSON + sha256 to exact input. Same op + same input
  replays the prior result; same op + different input is HELD unchanged.
- CLI auto-allocated op UUIDs are durably pre-recorded as an owner-private
  outbox intent BEFORE the mutation transaction; `ops` provides recovery
  lookup for a lost first ACK. No blind retry with a fresh ID. A pending
  intent under an op UUID with a DIFFERENT payload holds, even with no
  event yet (the intent is the durable first word, not a hint).
- CAS revision checks for same-entry payload replacement; stale proposals
  are retained as events marked not_applied and never overwrite a winner;
  a replay of a not_applied op stays not_applied.
- Store creation is atomic: schema AND complete ledger identity AND an
  OS-reported creator proof commit in ONE transaction; concurrent
  first-creates yield exactly one identity. The DB file is private 0600
  before any byte is written; chmod failures abort (never ignored);
  pre-existing foreign/corrupt/zero-byte stores are held,
  never repaired in place; non-regular files (FIFO/socket) are rejected.
- Read commands open read-only and never create store dir/DB/WAL/index.
- Effective verification is always UNKNOWN/UNVERIFIED here: every payload
  status is actor-reported, never attested; a note saying "verified: true"
  cannot self-certify.
- Pasted command text is stored as data, never executed.
"""
from __future__ import annotations

import json
import math
import os
import sqlite3
import stat
import time
import uuid as uuid_mod
from datetime import datetime, timezone
from hashlib import sha256
from pathlib import Path

SCHEMA_VERSION = "1"
DEFAULT_DB_NAME = "working-ledger.sqlite3"
BUSY_TIMEOUT_S = 5.0
MAX_EVENT_BYTES = 64 * 1024
MAX_NOTE_BYTES = 32 * 1024
MAX_TITLE_BYTES = 512
MAX_LABEL_BYTES = 1024
MAX_REVISION = (1 << 63) - 1
EVENT_ENVELOPE_RESERVE = 4096
MAX_RESUME_ENTRIES = 50
MAX_RESUME_NOTES = 20
EFFECTIVE_VERIFICATION = "UNKNOWN"
ENTRY_UUID_NS = uuid_mod.uuid5(uuid_mod.NAMESPACE_URL, "yggterm-working-ledger:entry")
_SLUG_A = ("amber", "basalt", "cedar", "ember", "flint", "grove", "harbor", "indigo")
_SLUG_N = ("otter", "falcon", "heron", "maple", "quartz", "willow", "beacon", "juniper")

_SCHEMA = (
    "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
    """CREATE TABLE IF NOT EXISTS entries (
        uuid TEXT PRIMARY KEY, slug TEXT UNIQUE NOT NULL, title TEXT NOT NULL,
        status TEXT NOT NULL DEFAULT 'open', payload TEXT NOT NULL,
        revision INTEGER NOT NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)""",
    """CREATE TABLE IF NOT EXISTS notes (
        entry_uuid TEXT NOT NULL, note_seq INTEGER NOT NULL, text TEXT NOT NULL,
        actor TEXT NOT NULL, recorded_at TEXT NOT NULL,
        PRIMARY KEY (entry_uuid, note_seq))""",
    """CREATE TABLE IF NOT EXISTS events (
        seq INTEGER PRIMARY KEY AUTOINCREMENT, event_id TEXT NOT NULL UNIQUE,
        op_uuid TEXT NOT NULL UNIQUE, payload_hash TEXT NOT NULL, kind TEXT NOT NULL,
        entry_uuid TEXT, body TEXT NOT NULL, actor TEXT NOT NULL,
        recorded_at TEXT NOT NULL, applied INTEGER NOT NULL, result TEXT NOT NULL)""",
    """CREATE TABLE IF NOT EXISTS outbox (
        op_uuid TEXT PRIMARY KEY, kind TEXT NOT NULL, payload_hash TEXT NOT NULL,
        state TEXT NOT NULL, created_at TEXT NOT NULL, settled_at TEXT)""",
)

KINDS = ("entry.create", "entry.note", "entry.edit")


class LedgerError(Exception):
    code = "internal_error"


class UsageError(LedgerError):
    code = "usage"


class PathPolicy(LedgerError):
    code = "path_rejected"


class StoreAbsent(LedgerError):
    code = "store_absent"


class ConflictHold(LedgerError):
    code = "op_conflict_hold"


class EntryNotFound(LedgerError):
    code = "entry_not_found"


class CorruptStore(LedgerError):
    code = "store_corrupt"


class SchemaUnknown(LedgerError):
    code = "schema_unknown"


# ---------- canonical / strict JSON ----------

def utf8_size(text):
    try:
        return len(text.encode("utf-8"))
    except UnicodeError:
        raise UsageError("invalid Unicode scalar in input") from None


def canonical_json(obj) -> str:
    try:
        encoded = json.dumps(obj, sort_keys=True, separators=(",", ":"),
                             ensure_ascii=False, allow_nan=False)
        utf8_size(encoded)
        return encoded
    except (ValueError, TypeError, RecursionError):
        raise UsageError("JSON must contain bounded, finite values") from None


def finite_float(text):
    value = float(text)
    if not math.isfinite(value):
        raise ValueError("non-finite number")
    return value


def bounded_limit(value, cap):
    if type(value) is not int or value < 1:
        raise UsageError("--limit must be a positive integer")
    return min(value, cap)


def validate_event_budget(canonical, actor):
    # Replay identity stays bound to request semantics; a retry cannot rewrite
    # the original actor. Actor bytes plus a reserved result/envelope budget
    # also count toward the complete event bound before any intent is stored.
    if utf8_size(canonical) + utf8_size(canonical_json(actor)) + EVENT_ENVELOPE_RESERVE > MAX_EVENT_BYTES:
        raise UsageError("complete event including actor metadata exceeds byte bound")


def sha256_hex(text: str) -> str:
    return sha256(text.encode("utf-8")).hexdigest()


def _pairs_no_dup(pairs):
    seen = {}
    for k, v in pairs:
        if k in seen:
            raise ValueError(f"duplicate JSON key: {k!r}")
        seen[k] = v
    return seen


def _reject_const(name):
    raise ValueError(f"non-finite number not allowed: {name}")


def parse_json_strict(text: str, cap: int):
    """Strict JSON: rejects duplicate keys, NaN/Infinity, over-cap input."""
    size = utf8_size(text)
    if size > cap:
        raise UsageError(f"JSON input exceeds {cap} byte cap")
    try:
        return json.loads(text, object_pairs_hook=_pairs_no_dup,
                          parse_constant=_reject_const, parse_float=finite_float)
    except (ValueError, RecursionError) as e:
        raise UsageError(f"invalid JSON: {e}" if isinstance(e, ValueError) else "JSON nesting exceeds parser limit") from None


def esc(s: str) -> str:
    """Escape non-printable/control chars for human display (data stays data)."""
    out = []
    for ch in s:
        if ch == "\\":
            out.append("\\\\")
        elif ch.isprintable():
            out.append(ch)
        else:
            out.append(ch.encode("unicode_escape").decode("ascii"))
    return "".join(out)


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


def _os_username(uid):
    try:
        import pwd
        return pwd.getpwuid(uid).pw_name if uid is not None else None
    except (ImportError, KeyError):
        return None


def actor_context(session_label=None, label_source=None) -> dict:
    if session_label is not None and (not isinstance(session_label, str) or utf8_size(session_label) > MAX_LABEL_BYTES):
        raise UsageError("reported session label exceeds byte bound")
    uid = getattr(os, "getuid", lambda: None)()
    return {
        "uid": uid,
        "username": _os_username(uid),
        "source": "os",
        "attested": False,
        "reported": True,
        "session_label": session_label,
        "session_label_source": label_source,
        "session_label_attested": False,
    }


def creator_proof() -> str:
    """OS-reported creator ownership proof stored WITH the identity in the
    same atomic first transaction. Reported only — never attested, never a
    verified/completion claim."""
    uid = getattr(os, "getuid", lambda: None)()
    return canonical_json({"uid": uid, "username": _os_username(uid),
                           "source": "os", "reported": True, "attested": False})


def mint_slug(existing_check) -> str:
    for _ in range(16):
        r = uuid_mod.uuid4().hex
        slug = f"{_SLUG_A[int(r[:2], 16) % 8]}-{_SLUG_N[int(r[2:4], 16) % 8]}-{r[4:8]}"
        if not existing_check(slug):
            return slug
    raise LedgerError("could not mint unique slug")


# ---------- path policy ----------

def known_roots(home: Path) -> list:
    roots = []
    env_root = os.environ.get("YGG_LEDGER_ROOT")
    if env_root:
        roots.append(Path(env_root).expanduser())
    roots.append(home / ".yggterm" / "ledgers")
    roots.append(home / ".ygg" / "ledgers")  # approved legacy alias
    return roots


def default_db_path(home: Path) -> Path:
    """Prefer the logical ~/.ygg root, without forking an existing store."""
    env_root = os.environ.get("YGG_LEDGER_ROOT")
    if env_root:
        return Path(env_root).expanduser() / DEFAULT_DB_NAME
    preferred = home / ".ygg" / "ledgers" / DEFAULT_DB_NAME
    legacy = home / ".yggterm" / "ledgers" / DEFAULT_DB_NAME
    if preferred.exists() and legacy.exists() and not preferred.samefile(legacy):
        raise PathPolicy("two distinct ledger stores exist; choose one explicitly with --db")
    if legacy.exists() and not preferred.exists():
        return legacy
    return preferred


def _within(child: Path, parent: Path) -> bool:
    return child == parent or parent in child.parents


def validate_db_path(db: Path, roots: list) -> Path:
    """The DB must resolve inside a known logical root. Symlink targets are
    checked, not just the link name; 'some ancestor' is not enough."""
    db = Path(db)
    if db.is_dir():
        raise PathPolicy(f"database path is a directory: {db}")
    raw_ok = any(_within(db.absolute(), Path(r).absolute()) for r in roots)
    res_ok = any(_within(db.resolve(), Path(r).resolve()) for r in roots)
    if raw_ok and res_ok:  # symlink targets are checked, not just the link name
        return db
    raise PathPolicy(
        f"database path {db} is outside all known logical roots "
        f"({', '.join(str(r) for r in roots)}); refusing")


# ---------- store ----------

def _corrupt_or_schema(e) -> LedgerError:
    s = str(e).lower()
    if any(t in s for t in ("not a database", "malformed", "encrypted", "corrupt", "disk image")):
        return CorruptStore(f"store corrupt; holding without repair ({e})")
    return SchemaUnknown(f"unknown schema; holding without repair ({e})")


_COLUMNS = {
    "meta": ("key", "value"),
    "entries": ("uuid", "slug", "title", "status", "payload", "revision", "created_at", "updated_at"),
    "notes": ("entry_uuid", "note_seq", "text", "actor", "recorded_at"),
    "events": ("seq", "event_id", "op_uuid", "payload_hash", "kind", "entry_uuid", "body", "actor", "recorded_at", "applied", "result"),
    "outbox": ("op_uuid", "kind", "payload_hash", "state", "created_at", "settled_at"),
}


def _verify_schema(conn):
    try:
        for table, columns in _COLUMNS.items():
            actual = tuple(row[1] for row in conn.execute("PRAGMA table_info(" + table + ")"))
            if actual != columns:
                raise SchemaUnknown("unknown ledger schema; holding without repair")
        meta = dict(conn.execute("SELECT key,value FROM meta"))
        required = {"schema_version", "ledger_uuid", "ledger_slug", "created_at", "creator"}
        if not required <= meta.keys() or meta["schema_version"] != SCHEMA_VERSION:
            raise SchemaUnknown("incomplete or unknown ledger identity; holding without repair")
        if str(uuid_mod.UUID(meta["ledger_uuid"])) != meta["ledger_uuid"] or not meta["ledger_slug"]:
            raise ValueError("invalid ledger identity")
        creator = parse_json_strict(meta["creator"], MAX_EVENT_BYTES)
        if not isinstance(creator, dict):
            raise ValueError("invalid creator record")
    except sqlite3.DatabaseError as e:
        raise _corrupt_or_schema(e) from None
    except (ValueError, UsageError):
        raise SchemaUnknown("invalid ledger identity; holding without repair") from None


def _private_store_file(db):
    """The data file itself is never a symlink or a foreign/shared file."""
    try:
        info = Path(db).lstat()
    except FileNotFoundError:
        raise StoreAbsent("ledger store absent") from None
    if not stat.S_ISREG(info.st_mode):
        raise PathPolicy("ledger store must be a regular file, not a symlink or special file")
    if not hasattr(os, "getuid"):
        raise PathPolicy("private ledger file permissions are not qualified on this platform")
    if info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) & 0o077 or info.st_nlink != 1:
        raise PathPolicy("ledger store must be owned by this user, private and unshared")
    return info


def _rollback_journal_only(db):
    # Inspect bytes before SQLite can create WAL coordination files. Our writer
    # protocol uses rollback journals; externally switched WAL stores are held.
    with Path(db).open("rb") as stream:
        header = stream.read(100)
    if len(header) < 100:
        raise SchemaUnknown("incomplete ledger header; no repair performed")
    if header[:16] != b"SQLite format 3\x00":
        raise CorruptStore("invalid ledger header; no repair performed")
    if header[18:20] != b"\x01\x01":
        raise SchemaUnknown("only rollback-journal ledger stores are supported; no files created")


def _ensure_private_dir(p: Path):
    missing = []
    q = Path(p)
    while not q.exists():
        missing.append(q)
        q = q.parent
    try:
        for directory in reversed(missing):
            try:
                directory.mkdir(mode=0o700)
            except FileExistsError:
                if not directory.is_dir():
                    raise PathPolicy("ledger directory creation raced a non-directory")
            else:
                os.chmod(directory, 0o700)
        info = Path(p).stat()
        if not stat.S_ISDIR(info.st_mode) or not hasattr(os, "getuid") or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) & 0o022:
            raise PathPolicy("ledger directory must be owned by this user and not writable by others")
    except OSError:
        raise PathPolicy("cannot prepare private ledger directory") from None


class Ledger:
    def __init__(self, conn, path: Path):
        self.conn = conn
        self.path = path
        self.hooks = []  # test-only crash-injection seams: fn(ledger, where)

    def close(self):
        try:
            self.conn.close()
        except sqlite3.Error:
            pass

    # -- openers --
    @classmethod
    def open_read(cls, db: Path):
        db = Path(db)
        _private_store_file(db)
        _rollback_journal_only(db)
        conn = None
        try:
            conn = sqlite3.connect(db.absolute().as_uri() + "?mode=ro", uri=True,
                                   timeout=BUSY_TIMEOUT_S, isolation_level=None)
            conn.execute("PRAGMA query_only=ON")
            conn.row_factory = sqlite3.Row
            _verify_schema(conn)
            return cls(conn, db)
        except BaseException as error:
            if conn is not None:
                conn.close()
            if isinstance(error, sqlite3.Error):
                raise _corrupt_or_schema(error) from None
            raise

    @classmethod
    def open_rw(cls, db: Path):
        db = Path(db)
        _private_store_file(db)
        _rollback_journal_only(db)
        conn = None
        try:
            conn = sqlite3.connect(db.absolute().as_uri() + "?mode=rw", uri=True,
                                   timeout=BUSY_TIMEOUT_S, isolation_level=None)
            conn.row_factory = sqlite3.Row
            _verify_schema(conn)
            return cls(conn, db)
        except BaseException as error:
            if conn is not None:
                conn.close()
            if isinstance(error, sqlite3.Error):
                raise _corrupt_or_schema(error) from None
            raise

    @classmethod
    def ensure_store(cls, db: Path):
        """Only the exclusive file creator initializes a store. Other creators
        wait boundedly for its one atomic schema/identity commit; they never
        repair a pre-existing empty, foreign or incomplete store."""
        db = Path(db)
        _ensure_private_dir(db.parent)
        created = False
        try:
            fd = os.open(db, os.O_CREAT | os.O_EXCL | os.O_WRONLY | getattr(os, "O_NOFOLLOW", 0), 0o600)
        except FileExistsError:
            _private_store_file(db)
        except OSError:
            raise PathPolicy("cannot exclusively create private ledger file") from None
        else:
            try:
                os.fchmod(fd, 0o600)
            finally:
                os.close(fd)
            created = True
        if not created:
            deadline = time.monotonic() + BUSY_TIMEOUT_S
            while True:
                try:
                    reader = cls.open_read(db)
                    reader.close()
                    return cls.open_rw(db)
                except SchemaUnknown:
                    # A first creator may still hold its schema transaction.
                    # Read-only verification changed no bytes. No takeover.
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(0.02)
        _private_store_file(db)
        conn = None
        try:
            conn = sqlite3.connect(db.absolute().as_uri() + "?mode=rw", uri=True,
                                   timeout=BUSY_TIMEOUT_S, isolation_level=None)
            conn.row_factory = sqlite3.Row
            conn.execute("PRAGMA journal_mode=DELETE")
            conn.execute("BEGIN IMMEDIATE")
            for statement in _SCHEMA:
                conn.execute(statement)
            for key, value in (("schema_version", SCHEMA_VERSION),
                               ("ledger_uuid", str(uuid_mod.uuid4())),
                               ("ledger_slug", mint_slug(lambda _: False)),
                               ("created_at", now_iso()), ("creator", creator_proof())):
                conn.execute("INSERT INTO meta(key,value) VALUES(?,?)", (key, value))
            _verify_schema(conn)
            conn.execute("COMMIT")
            return cls(conn, db)
        except BaseException as error:
            if conn is not None:
                if conn.in_transaction:
                    conn.execute("ROLLBACK")
                conn.close()
            # Retain the creator's failed file as evidence; never unlink a
            # pathname another process may already have opened.
            if isinstance(error, sqlite3.Error):
                raise _corrupt_or_schema(error) from None
            raise

    # -- identity / reads --
    def identity(self) -> dict:
        rows = self.conn.execute("SELECT key,value FROM meta").fetchall()
        m = {r[0]: r[1] for r in rows}
        creator = None
        if m.get("creator"):
            try:
                creator = json.loads(m["creator"])
            except ValueError:
                creator = {"unparsed": True}
        return {"uuid": m.get("ledger_uuid"), "slug": m.get("ledger_slug"),
                "created_at": m.get("created_at"), "schema_version": m.get("schema_version"),
                "creator": creator}

    def entry_row(self, ref: str):
        row = self.conn.execute(
            "SELECT * FROM entries WHERE slug=? OR uuid=?", (ref, ref)).fetchone()
        return row

    def entry_or_fail(self, ref: str):
        row = self.entry_row(ref)
        if row is None:
            raise EntryNotFound(f"no entry matching {ref!r}")
        return row

    def list_entries(self, limit: int):
        limit = bounded_limit(limit, 1000)
        rows = self.conn.execute(
            "SELECT * FROM entries ORDER BY created_at, uuid LIMIT ?",
            (limit + 1,)).fetchall()
        truncated = len(rows) > limit
        return rows[:limit], truncated

    def notes_for(self, entry_uuid: str, limit: int):
        limit = bounded_limit(limit, MAX_RESUME_NOTES)
        rows = self.conn.execute(
            "SELECT * FROM notes WHERE entry_uuid=? ORDER BY note_seq DESC LIMIT ?",
            (entry_uuid, limit)).fetchall()
        total = self.conn.execute(
            "SELECT COUNT(*) FROM notes WHERE entry_uuid=?", (entry_uuid,)).fetchone()[0]
        return list(reversed(rows)), total

    def history_for(self, entry_uuid: str, limit: int):
        limit = bounded_limit(limit, 500)
        return self.conn.execute(
            "SELECT * FROM events WHERE entry_uuid=? ORDER BY seq LIMIT ?",
            (entry_uuid, limit)).fetchall()

    def event_count(self) -> int:
        return self.conn.execute("SELECT COUNT(*) FROM events").fetchone()[0]

    # -- outbox (owner-private intent, durable BEFORE mutation) --
    def record_intent(self, op_uuid: str, kind: str, payload_hash: str):
        self.conn.execute("BEGIN IMMEDIATE")
        try:
            self.conn.execute(
                "INSERT OR IGNORE INTO outbox(op_uuid,kind,payload_hash,state,created_at,settled_at)"
                " VALUES(?,?,?,?,?,NULL)",
                (op_uuid, kind, payload_hash, "pending", now_iso()))
            self.conn.execute("COMMIT")
        except BaseException:
            if self.conn.in_transaction:
                self.conn.execute("ROLLBACK")
            raise

    def _settle_outbox(self, op_uuid: str):
        self.conn.execute(
            "UPDATE outbox SET state='settled', settled_at=? WHERE op_uuid=? AND state='pending'",
            (now_iso(), op_uuid))

    def pending_intents(self, limit: int):
        limit = bounded_limit(limit, 200)
        return self.conn.execute(
            "SELECT o.*, (SELECT seq FROM events e WHERE e.op_uuid=o.op_uuid) AS event_seq"
            " FROM outbox o WHERE o.state='pending' ORDER BY o.created_at LIMIT ?",
            (limit,)).fetchall()

    def op_lookup(self, op_uuid: str) -> dict:
        ev = self.conn.execute(
            "SELECT * FROM events WHERE op_uuid=?", (op_uuid,)).fetchone()
        ob = self.conn.execute(
            "SELECT * FROM outbox WHERE op_uuid=?", (op_uuid,)).fetchone()
        out = {"op_uuid": op_uuid}
        if ev is not None:
            out["recorded_result"] = json.loads(ev["result"])
            out["recorded_at"] = ev["recorded_at"]
            out["payload_hash"] = ev["payload_hash"]
            out["guidance"] = ("operation already committed; replaying the same op UUID "
                               "with identical input returns this prior result")
        elif ob is not None:
            out["intent_state"] = ob["state"]
            out["kind"] = ob["kind"]
            out["payload_hash"] = ob["payload_hash"]
            out["created_at"] = ob["created_at"]
            out["guidance"] = ("pending intent, no recorded result: safe to replay the "
                               "SAME op UUID with IDENTICAL payload; do not retry with a new ID")
        else:
            out["status"] = "unknown_operation"
            out["guidance"] = "no event and no outbox intent under this operation UUID"
        return out

    # -- mutation core --
    def apply(self, op_uuid: str, kind: str, entry_uuid, body: dict, actor: dict) -> dict:
        if kind not in KINDS:
            raise UsageError(f"unsupported operation kind {kind!r}")
        validate_body(kind, body)
        canonical = canonical_json({"kind": kind, "entry_uuid": entry_uuid, "body": body})
        if len(canonical.encode("utf-8")) > MAX_EVENT_BYTES:
            raise UsageError(f"event exceeds {MAX_EVENT_BYTES} byte bound")
        validate_event_budget(canonical, actor)
        ph = sha256_hex(canonical)
        conn = self.conn
        conn.execute("BEGIN IMMEDIATE")
        try:
            prior = conn.execute(
                "SELECT payload_hash, result, recorded_at FROM events WHERE op_uuid=?",
                (op_uuid,)).fetchone()
            if prior is not None:
                if prior["payload_hash"] != ph:
                    conn.execute("ROLLBACK")
                    raise ConflictHold(
                        f"operation {op_uuid} already exists with a DIFFERENT payload; "
                        "held unchanged (no blind retry; inspect with ops --op)")
                original = json.loads(prior["result"])
                self._settle_outbox(op_uuid)
                conn.execute("COMMIT")
                return {"status": "duplicate", "replayed": True, "kind": kind,
                        "op_uuid": op_uuid, "payload_hash": ph,
                        "original_recorded_at": prior["recorded_at"], "original": original}
            # a pending outbox intent under this op UUID with a DIFFERENT payload
            # holds even though no event exists yet: the durably recorded intent
            # is the caller's first word, so a same-ID retry can never reword it.
            ob = conn.execute(
                "SELECT payload_hash FROM outbox WHERE op_uuid=?", (op_uuid,)).fetchone()
            if ob is not None and ob["payload_hash"] != ph:
                conn.execute("ROLLBACK")
                raise ConflictHold(
                    f"operation {op_uuid} has a pending outbox intent with a DIFFERENT "
                    f"payload (intent hash {ob['payload_hash'][:12]}…, this hash "
                    f"{ph[:12]}…); held unchanged, no event written, intent preserved "
                    "(no blind retry; inspect with ops --op)")
            status, extra = self._dispatch(kind, entry_uuid, body, actor)
            recorded = now_iso()
            result = {"status": status, "op_uuid": op_uuid, "kind": kind,
                      "entry_uuid": entry_uuid, "recorded_at": recorded, **extra}
            conn.execute(
                "INSERT INTO events(event_id,op_uuid,payload_hash,kind,entry_uuid,body,"
                "actor,recorded_at,applied,result) VALUES(?,?,?,?,?,?,?,?,?,?)",
                (str(uuid_mod.uuid4()), op_uuid, ph, kind, entry_uuid, canonical,
                 canonical_json(actor), recorded, 1 if status == "applied" else 0,
                 canonical_json(result)))
            for hook in self.hooks:
                hook(self, "after-event-insert")
            self._settle_outbox(op_uuid)
            conn.execute("COMMIT")
            result["payload_hash"] = ph
            return result
        except BaseException:
            if conn.in_transaction:
                conn.execute("ROLLBACK")
            raise

    def _dispatch(self, kind, entry_uuid, body, actor):
        now = now_iso()
        a = canonical_json(actor)
        if kind == "entry.create":
            if self.entry_row(entry_uuid) is not None:
                return "not_applied", {"reason": "entry_exists"}
            slug = mint_slug(lambda s: self.conn.execute(
                "SELECT 1 FROM entries WHERE slug=?", (s,)).fetchone() is not None)
            self.conn.execute(
                "INSERT INTO entries(uuid,slug,title,status,payload,revision,created_at,updated_at)"
                " VALUES(?,?,?,?,?,?,?,?)",
                (entry_uuid, slug, body["title"], "open",
                 canonical_json(body.get("payload", {})), 1, now, now))
            return "applied", {"revision": 1, "slug": slug, "title": body["title"]}
        if kind == "entry.note":
            row = self.entry_or_fail(entry_uuid)
            seq = self.conn.execute(
                "SELECT COALESCE(MAX(note_seq),0)+1 FROM notes WHERE entry_uuid=?",
                (entry_uuid,)).fetchone()[0]
            self.conn.execute(
                "INSERT INTO notes(entry_uuid,note_seq,text,actor,recorded_at) VALUES(?,?,?,?,?)",
                (entry_uuid, seq, body["text"], a, now))
            self.conn.execute("UPDATE entries SET updated_at=? WHERE uuid=?",
                              (now, entry_uuid))
            return "applied", {"note_seq": seq, "revision": row["revision"],
                               "slug": row["slug"]}
        if kind == "entry.edit":
            row = self.entry_or_fail(entry_uuid)
            expected = body["expected_revision"]
            cur = self.conn.execute(
                "UPDATE entries SET payload=?, revision=revision+1, updated_at=?"
                " WHERE uuid=? AND revision=?",
                (canonical_json(body["payload"]), now, entry_uuid, expected))
            if cur.rowcount == 1:
                return "applied", {"revision": expected + 1, "slug": row["slug"]}
            return "not_applied", {"reason": "stale_revision",
                                   "expected_revision": expected,
                                   "current_revision": row["revision"],
                                   "slug": row["slug"]}
        raise UsageError(f"unsupported operation kind {kind!r}")

    # -- resume packet (bounded, read-only) --
    def resume(self) -> dict:
        ident = self.identity()
        entries, truncated = self.list_entries(MAX_RESUME_ENTRIES)
        open_entries = []
        for row in entries:
            if row["status"] != "open":
                continue
            notes, total = self.notes_for(row["uuid"], MAX_RESUME_NOTES)
            payload = json.loads(row["payload"])
            item = {
                "uuid": row["uuid"], "slug": row["slug"], "title": row["title"],
                "status": row["status"], "revision": row["revision"],
                "created_at": row["created_at"], "updated_at": row["updated_at"],
                "payload": payload,
                "payload_status": "actor-reported; effective verification "
                                  + EFFECTIVE_VERIFICATION,
                "delegate_ref": payload.get("delegate") if isinstance(payload, dict) else None,
                "artifact_ref": payload.get("artifact_ref") if isinstance(payload, dict) else None,
                "notes": {"total": total, "returned": len(notes),
                          "items": [{"note_seq": n["note_seq"], "text": n["text"],
                                     "recorded_at": n["recorded_at"],
                                     "actor": json.loads(n["actor"])} for n in notes]},
            }
            open_entries.append(item)
        return {"ledger": ident, "event_count": self.event_count(),
                "verification": {"effective": EFFECTIVE_VERIFICATION,
                                 "policy": ("every payload status is actor-reported; "
                                            "no qualified evidence bridge in this slice; "
                                            "an empty contract cannot yield verified")},
                "open_entries": open_entries,
                "open_entry_count": len(open_entries),
                "entries_truncated": truncated,
                "note": ("delegate_ref/artifact_ref are arbitrary typed payloads echoed "
                         "as references only; never read, dispatched, or dereferenced")}


def validate_body(kind: str, body: dict):
    if kind == "entry.create":
        t = body.get("title")
        if not isinstance(t, str) or not t.strip():
            raise UsageError("--title is required and must be non-empty")
        if len(t.encode("utf-8")) > MAX_TITLE_BYTES:
            raise UsageError(f"title exceeds {MAX_TITLE_BYTES} byte cap")
        p = body.get("payload", {})
        if not isinstance(p, dict):
            raise UsageError("payload must be a JSON object (arbitrary, extensible)")
    elif kind == "entry.note":
        t = body.get("text")
        if not isinstance(t, str) or not t.strip():
            raise UsageError("note text must be non-empty")
        if len(t.encode("utf-8")) > MAX_NOTE_BYTES:
            raise UsageError(f"note exceeds {MAX_NOTE_BYTES} byte cap")
    elif kind == "entry.edit":
        p = body.get("payload")
        if not isinstance(p, dict):
            raise UsageError("payload must be a JSON object (arbitrary, extensible)")
        r = body.get("expected_revision")
        if not isinstance(r, int) or isinstance(r, bool) or not 1 <= r <= MAX_REVISION:
            raise UsageError("--expected-revision must be a positive SQLite integer (CAS)")
    else:
        raise UsageError(f"unsupported operation kind {kind!r}")
    if len(canonical_json(body).encode("utf-8")) > MAX_EVENT_BYTES:
        raise UsageError(f"event body exceeds {MAX_EVENT_BYTES} byte bound")
