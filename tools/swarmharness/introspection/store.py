"""The local inspection store: immutable snapshots and retained evidence.

Layout under the store root (default ``~/.yggterm/swarm/introspection``;
classification manifest: ``swarm/introspection/`` is T3 protocol-managed and
``swarm/introspection/events/`` is T2 report-only):

  snapshots/<snapshot_id>.json   write-once immutable query frames
  events/YYYY-MM.jsonl           monthly bounded maintenance-event partitions

A snapshot file is written once through tmp+fsync+rename and never modified:
saving an ID that already exists is an idempotent no-op, never an overwrite.
Retention is bounded (raw days + store bytes); pruning removes whole expired
artifacts and reports exactly what it removed.
"""
import calendar
import fcntl
import json
import os
import re
import shutil
import time
from pathlib import Path

from . import model

SID = re.compile(r"[a-f0-9]{64}")
MONTH = re.compile(r"\d{4}-\d{2}")


def store_root(root=None):
    if root:
        return Path(root).expanduser()
    env = os.environ.get("YGG_INSPECT_STORE")
    return Path(env).expanduser() if env else Path.home() / ".yggterm/swarm/introspection"


def snapshots_dir(root=None):
    return store_root(root) / "snapshots"


def save_snapshot(frame, root=None):
    """Persist an immutable frame. Returns (snapshot_id, created_bool)."""
    model.validate_frame(frame)
    d = snapshots_dir(root)
    d.mkdir(parents=True, exist_ok=True)
    path = d / (frame["snapshot_id"] + ".json")
    if path.exists():
        return frame["snapshot_id"], False
    tmp = d / (".tmp-" + str(os.getpid()) + "-" + frame["snapshot_id"][:12])
    with open(tmp, "w") as f:
        json.dump(frame, f, sort_keys=True, separators=(",", ":"))
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)
    fd = os.open(d, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)
    return frame["snapshot_id"], True


def load_snapshot(snapshot_id, root=None):
    """Load and revalidate a retained snapshot by exact ID."""
    if not SID.fullmatch(str(snapshot_id)):
        raise ValueError("invalid snapshot ID")
    path = snapshots_dir(root) / (str(snapshot_id) + ".json")
    try:
        frame = json.loads(path.read_text())
    except FileNotFoundError:
        return None
    except OSError as exc:
        raise ValueError("snapshot unreadable") from exc
    except json.JSONDecodeError:
        raise ValueError("snapshot store entry corrupt")
    return model.validate_frame(frame)


def latest_snapshot(root=None, since=None):
    """The most recent retained snapshot at or after ``since`` (epoch), or
    the newest one overall when since is None. None when the store is empty."""
    d = snapshots_dir(root)
    best = None
    try:
        entries = sorted(d.glob("*.json"), key=lambda p: p.stat().st_mtime)
    except OSError:
        return None
    for p in entries:
        if not SID.fullmatch(p.stem):
            continue
        if since is not None and p.stat().st_mtime < since:
            continue
        best = p
    if best is None:
        return None
    try:
        return model.validate_frame(json.loads(best.read_text()))
    except (OSError, json.JSONDecodeError):
        return None


def append_events(rows, root=None, now=None):
    """Retain bounded evidence rows in a monthly partition. Dedupe is by row
    ID within the file (a replayed observation is a no-op); malformed rows
    are refused, never half-written."""
    if not rows:
        return {"appended": 0, "partitions": []}
    month_file = events_partition(now)
    path = events_dir(root) / month_file
    path.parent.mkdir(parents=True, exist_ok=True)
    # Dedupe and append are ONE critical section: reading existing IDs before
    # taking the lock lets two writers read the same un-updated file, both
    # conclude a row is absent, and both append it.
    lock = path.with_suffix(".lock")
    kept = []
    with open(lock, "a+") as lf:
        fcntl.flock(lf, fcntl.LOCK_EX)
        try:
            existing = {}
            try:
                for line in path.read_text().splitlines():
                    try:
                        row = json.loads(line)
                    except Exception:
                        continue
                    if isinstance(row, dict) and row.get("id"):
                        existing[row["id"]] = row
            except FileNotFoundError:
                pass
            for row in rows:
                if not isinstance(row, dict) or not row.get("id") or "summary" not in row:
                    continue
                if row["id"] in existing:
                    continue
                kept.append(row)
                existing[row["id"]] = row
            if kept:
                with open(path, "a") as f:
                    for row in kept:
                        f.write(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n")
                    f.flush()
                    os.fsync(f.fileno())
        finally:
            fcntl.flock(lf, fcntl.LOCK_UN)
    return {"appended": len(kept), "partitions": [month_file]}


def events_dir(root=None):
    return store_root(root) / "events"


def events_partition(now=None):
    stamp = time.gmtime(now if now else time.time())
    return time.strftime("%Y-%m", stamp) + ".jsonl"


def read_events(root=None, since=None, until=None, subsystem=None, limit=2000):
    """Bounded retained events across monthly partitions, newest first."""
    d = events_dir(root)
    rows = []
    try:
        parts = sorted(d.glob("*.jsonl"), reverse=True)
    except OSError:
        return []
    for p in parts:
        if not MONTH.fullmatch(p.stem):
            continue
        try:
            for line in p.read_text().splitlines():
                try:
                    row = json.loads(line)
                except Exception:
                    continue
                if not isinstance(row, dict):
                    continue
                at = model.epoch(row.get("at")) or row.get("captured_at")
                if since is not None and at is not None and at < since:
                    continue
                if until is not None and at is not None and at > until:
                    continue
                if subsystem and row.get("subsystem") not in subsystem:
                    continue
                rows.append(row)
        except OSError:
            continue
    rows.sort(key=lambda r: r.get("at") or r.get("captured_at") or 0, reverse=True)
    return rows[:limit]


def _review_cited_snapshot_ids(reviews_dir=None, max_files=100):
    """Snapshot IDs a sessionReview chain cites (subject_digest on
    snapshot_staged rows). Retention never deletes evidence a review may
    still be verifying; an unreadable reviews dir protects nothing extra but
    never widens deletion — pruning stays age/bounded either way."""
    base = Path(reviews_dir or os.environ.get("YGG_INSPECT_REVIEWS",
                                              str(Path.home() / ".yggterm/swarm/reviews"))).expanduser()
    cited = set()
    try:
        entries = sorted(base.glob("*.jsonl"))[:max_files]
    except OSError:
        return cited
    for p in entries:
        try:
            with p.open("rb") as f:
                raw = f.read(262144)
        except OSError:
            continue
        for line in raw.splitlines():
            try:
                row = json.loads(line)
            except Exception:
                continue
            if isinstance(row, dict) and row.get("kind") == "snapshot_staged":
                d = row.get("subject_digest")
                if isinstance(d, str) and re.fullmatch(r"[a-f0-9]{64}", d):
                    cited.add(d)
    return cited


def prune(root=None, now=None, retention=None):
    """Bound the store: drop snapshots older than raw_days and monthly event
    partitions entirely past the window; then enforce the byte cap by
    removing the oldest snapshots. Reports what was removed."""
    retention = retention or {}
    days = int(retention.get("raw_days", 31))
    cap = int(retention.get("max_store_bytes", 256 * 1024 * 1024))
    now = now if now else time.time()
    horizon = now - days * 86400
    removed = {"snapshots": 0, "event_files": 0, "bytes": 0,
               "protected_by_review": 0}
    cited = _review_cited_snapshot_ids()
    d = snapshots_dir(root)
    if d.is_dir():
        for p in sorted(d.glob("*.json"), key=lambda q: q.stat().st_mtime):
            if p.stem in cited:
                removed["protected_by_review"] += 1
                continue  # a review may still be verifying this snapshot
            if p.stat().st_mtime < horizon:
                removed["bytes"] += p.stat().st_size
                p.unlink(missing_ok=True)
                removed["snapshots"] += 1
        total = sum(q.stat().st_size for q in d.glob("*.json"))
        for p in sorted(d.glob("*.json"), key=lambda q: q.stat().st_mtime):
            if total <= cap:
                break
            if p.stem in cited:
                removed["protected_by_review"] += 1
                continue
            removed["bytes"] += p.stat().st_size
            total -= p.stat().st_size
            p.unlink(missing_ok=True)
            removed["snapshots"] += 1
    ed = events_dir(root)
    if ed.is_dir():
        for p in sorted(ed.glob("[0-9][0-9][0-9][0-9]-[0-9][0-9].jsonl")):
            if _partition_end(p.stem) < horizon:
                removed["bytes"] += p.stat().st_size
                p.unlink(missing_ok=True)
                removed["event_files"] += 1
    return removed


def _partition_end(stem):
    """First epoch second AFTER the partition, in UTC — the same clock
    events_partition() names partitions with. Local-time interpretation
    pruned whole-month evidence hours early in positive UTC offsets."""
    year, month = int(stem[:4]), int(stem[5:7])
    if month == 12:
        return calendar.timegm((year + 1, 1, 1, 0, 0, 0, 0, 0, 0))
    return calendar.timegm((year, month + 1, 1, 0, 0, 0, 0, 0, 0))


def store_bytes(root=None):
    total = 0
    for base in (snapshots_dir(root), events_dir(root)):
        if base.is_dir():
            total += sum(p.stat().st_size for p in base.rglob("*") if p.is_file())
    return total
