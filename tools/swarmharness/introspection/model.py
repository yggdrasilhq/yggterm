"""Portable observation/identity/query contracts. No host or process I/O."""
import base64
from copy import deepcopy
from datetime import datetime, timezone
import hashlib
import json
import math
import re

SCHEMA = 1
QUALITIES = {"verified", "stale", "unavailable", "conflicting", "unknown"}
SEVERITY = {"debug": 0, "info": 1, "warning": 2, "error": 3, "critical": 4}
_SECRET = re.compile(r"(?i)(?:bearer\s+\S+|(?:api[_-]?key|password|access[_-]?token|secret)\s*[:=]\s*[^\s,;]+|\bsk-[A-Za-z0-9_-]{12,})")
_CONTROL = re.compile(r"[\x00-\x08\x0b-\x1f\x7f-\x9f]")


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False)


def digest(value):
    return hashlib.sha256(canonical(value).encode()).hexdigest()


def text(value, limit=300):
    """Bounded metadata only: source bodies never enter a query frame."""
    value = str(value or "")
    if _SECRET.search(value):
        return "[redacted: credential-like metadata]"
    return _CONTROL.sub("", value).replace("\n", " ").replace("\r", " ")[:limit]


def epoch(value):
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        if not math.isfinite(value):
            return None
        return value / 1000 if value > 100_000_000_000 else float(value)
    if isinstance(value, str):
        try:
            dt = datetime.fromisoformat(value.replace("Z", "+00:00"))
            return dt.timestamp() if dt.tzinfo else None
        except ValueError:
            return None
    return None


def iso(value):
    return datetime.fromtimestamp(value, timezone.utc).isoformat(timespec="seconds")


def boundary(value, anchor, future=False):
    if value is None:
        return None
    match = re.fullmatch(r"(\+?)(\d+)([smhdw])", value)
    if match:
        seconds = int(match[2]) * {"s": 1, "m": 60, "h": 3600, "d": 86400, "w": 604800}[match[3]]
        return anchor + seconds if match[1] or future else anchor - seconds
    found = epoch(value)
    if found is None:
        raise ValueError("time must be a timezone-qualified timestamp or a duration such as 7d/+7d")
    return found


def coverage(source, host, kind, *, observed_at, received_at, ttl, quality="verified",
             revision=None, reason=None, collection_mode=None):
    if quality not in QUALITIES:
        quality, reason = "unknown", "unsupported_source_quality"
    return {"source": source, "host": host, "kind": kind,
            "observed_at": observed_at, "received_at": received_at,
            "ttl_seconds": ttl, "quality": quality, "revision": revision, "reason": reason,
            "collection_mode": collection_mode}


def assess(row, now):
    row = deepcopy(row)
    observed = epoch(row.get("observed_at"))
    row["age_seconds"] = None if observed is None else round(now - observed, 3)
    if row.get("quality") == "verified":
        ttl = row.get("ttl_seconds")
        if observed is None or not isinstance(ttl, (int, float)) or ttl <= 0:
            row.update(quality="unknown", reason="missing_observation_time_or_ttl")
        elif observed > now + 2:
            row.update(quality="conflicting", reason="source_clock_ahead")
        elif now - observed > ttl:
            row.update(quality="stale", reason="observation_expired")
    return row


def frame(config_hash, observed_at, sources, datasets):
    body = {"schema_version": SCHEMA, "config_hash": config_hash,
            "as_of": observed_at, "coverage": sources, "datasets": datasets}
    return dict(body, snapshot_id=digest(body))


def validate_frame(value):
    if not isinstance(value, dict) or value.get("schema_version") != SCHEMA:
        raise ValueError("unsupported inspection snapshot")
    if value.get("snapshot_id") != digest({k: v for k, v in value.items() if k != "snapshot_id"}):
        raise ValueError("inspection snapshot digest mismatch")
    if not isinstance(value.get("coverage"), list) or not isinstance(value.get("datasets"), dict):
        raise ValueError("invalid inspection snapshot shape")
    if epoch(value.get("as_of")) is None:
        raise ValueError("snapshot capture time unavailable")
    for c in value["coverage"]:
        if (not isinstance(c, dict) or c.get("quality") not in QUALITIES
                or not all(isinstance(c.get(k), str) for k in ("source", "host", "kind"))):
            raise ValueError("invalid snapshot coverage record")
    allowed = {"dreams", "attempts", "services", "schedules", "plans", "events",
               "heartbeat", "findings", "summary", "repairs", "workers",
               "composer", "reviews"}
    if set(value["datasets"]) - allowed:
        raise ValueError("unenrolled snapshot dataset")
    if any(not isinstance(rows, list) or not all(isinstance(row, dict) for row in rows)
           for rows in value["datasets"].values()):
        raise ValueError("invalid snapshot records")
    def metadata_only(obj):
        if isinstance(obj, dict):
            for k, v in obj.items():
                if str(k).lower().replace("_", "") in {
                        "prompt", "promptfile", "messages", "authorization", "password",
                        "apikey", "accesstoken", "credentials", "secret"}:
                    raise ValueError("non-metadata field refused")
                metadata_only(v)
        elif isinstance(obj, list):
            for v in obj:
                metadata_only(v)
        elif isinstance(obj, str) and _SECRET.search(obj):
            raise ValueError("credential-like metadata refused")
    metadata_only(value)
    return value


def queue_records(tasks, metrics, run_metadata, source, host, now):
    """Join real dispatch receipts; never synthesize N attempts from a count.

    Retry ordinals can be reused by a queue after budget deferral. Physical
    dispatch event IDs keep those executions separate and mark overwritten
    run-file references rather than crediting a later file to an old attempt.
    """
    attempts, latest = {}, {}
    metrics = [e for e in metrics if isinstance(e, dict)]
    tasks = [t for t in tasks if isinstance(t, dict)]
    for event in metrics:
        raw_id = str(event.get("id", ""))
        if not raw_id:
            continue
        if event.get("ev") == "dispatch":
            event_id = event.get("event_id") or digest(event)
            attempt = {"id": "attempt:" + source + ":" + event_id,
                "work_id": "work:" + source + ":" + raw_id, "queue_id": raw_id,
                "host": host, "source": source, "ordinal": event.get("attempt"),
                "started_at": epoch(event.get("t")), "finished_at": None,
                "state": "unknown", "queue_state": None, "verification": "unknown",
                "reason": "no terminal observation", "evidence_state": "unknown",
                "review": "unreviewed", "review_scope": "inspection-ledger",
                "composer_link": "unknown", "session_id": None, "run_ref": None}
            attempts[attempt["id"]] = attempt
            latest[raw_id] = attempt
        elif event.get("ev") == "done" and raw_id in latest:
            attempt = latest[raw_id]
            outcome = str(event.get("outcome", "unknown"))
            state = "completed" if outcome == "completed" else "failed" if outcome.startswith("failed") else "interrupted"
            attempt.update(state=state, finished_at=epoch(event.get("t")), reason=text(outcome),
                           exit_code=event.get("rc"))
    work = []
    for task in tasks:
        raw_id = str(task["id"])
        work_id = "work:" + source + ":" + raw_id
        current = latest.get(raw_id)
        if current is None and task.get("attempts", 0):
            # A single current-state receipt is real evidence, but not an
            # assertion that every previous execution was retained.
            current = {"id": "attempt:" + source + ":current:" + raw_id,
                "work_id": work_id, "queue_id": raw_id, "source": source, "host": host,
                "ordinal": task.get("attempts"), "started_at": epoch(task.get("started_at")),
                "finished_at": epoch(task.get("finished_at")), "state": task.get("status", "unknown"),
                "queue_state": task.get("status"), "verification": "unknown", "evidence_state": "unknown",
                "reason": text(task.get("last_error")), "review": "unreviewed",
                "review_scope": "inspection-ledger", "composer_link": "unknown",
                "session_id": None, "run_ref": None}
            attempts[current["id"]] = current
        meta = run_metadata.get(raw_id, {})
        if current is not None:
            current.update(queue_state=task.get("status"), run_ref=meta.get("ref"),
                evidence_state=meta.get("quality", "unavailable"),
                session_id=meta.get("session_id"), harness=meta.get("harness"),
                reason=text(task.get("last_error") or current.get("reason")),
                route=text(task.get("route")), title=text(task.get("title")),
                observed_at=now)
            written = epoch(meta.get("written_at"))
            started = current.get("started_at")
            if written is not None and started is not None and written < started:
                current.update(evidence_state="stale-other-execution", session_id=None)
            if task.get("status") == "running":
                current["state"] = "running"
            elif current.get("state") == "unknown":
                current["state"] = task.get("status", "unknown")
            # Older executions may have reused this ordinal's filename.
            for prior in attempts.values():
                if prior is not current and prior["work_id"] == work_id and prior.get("ordinal") == current.get("ordinal"):
                    prior["evidence_state"] = "overwritten-or-unlinked"
        item = {"id": work_id, "queue_id": raw_id, "source": source, "host": host,
            "title": text(task.get("title")), "route": text(task.get("route")),
            "state": task.get("status", "unknown"), "model": text(task.get("model")),
            "created_at": epoch(task.get("created_at")), "updated_at": epoch(task.get("updated_at")),
            "next_eligible_at": epoch(task.get("next_eligible_at")),
            "reported_attempt_count": task.get("attempts"),
            "attempt_ids": [a["id"] for a in attempts.values() if a["work_id"] == work_id],
            "current_attempt_id": current["id"] if current else None,
            "history_quality": "verified-dispatches" if metrics else "unknown",
            "review": current.get("review") if current else "unknown"}
        work.append(item)
    return work, list(attempts.values())


def _request_key(path, args):
    return digest({"path": list(path), "arguments": {k: v for k, v in args.items()
                  if k not in ("snapshot", "cursor", "limit", "json", "refresh")}})


def page_token(snapshot_id, request_key, offset):
    return base64.urlsafe_b64encode(canonical([snapshot_id, request_key, offset]).encode()).decode().rstrip("=")


def read_token(token):
    try:
        if len(token) > 1024:
            raise ValueError()
        value = json.loads(base64.urlsafe_b64decode(token + "=" * (-len(token) % 4)))
        if (not isinstance(value, list) or len(value) != 3 or not re.fullmatch(r"[a-f0-9]{64}", value[0])
                or not re.fullmatch(r"[a-f0-9]{64}", value[1]) or type(value[2]) is not int or value[2] < 0):
            raise ValueError()
        return value
    except (ValueError, TypeError, IndexError) as exc:
        raise ValueError("invalid inspection cursor") from exc


def query(snapshot, path, args, now):
    validate_frame(snapshot)
    data = snapshot["datasets"]
    family, verb = path
    anchor = epoch(snapshot["as_of"])
    since = boundary(args.get("since"), anchor)
    until = boundary(args.get("until"), anchor, future=True)
    def row_time(r):
        return epoch(r.get("finished_at") or r.get("at") or r.get("updated_at")
                     or r.get("last_at") or r.get("resolution_at")
                     or r.get("started_at") or r.get("created_at") or r.get("observed_at"))
    def in_window(r):
        t = row_time(r)
        return ((since is None and until is None) or t is not None
                and (since is None or t >= since) and (until is None or t <= until))
    if family == "dream":
        name, needed = "dreams", {"queue"}
    elif family in ("worker", "workers"):
        name, needed = "workers", {"workers"}
    elif family in ("composer", "reviews"):
        name, needed = family, {family, "memory_lock"}
    elif family == "service":
        name, needed = "services", {"services"}
    elif family == "schedule":
        name, needed = "schedules", {"schedule", "task_ledger"}
    else:
        name = {"heartbeat": "heartbeat", "pressure": "plans", "events": "events",
                "check": "findings", "status": "summary"}[verb]
        needed = {"heartbeat"} if verb in ("heartbeat", "pressure") else None
    rows = deepcopy(data.get(name, []))
    requested_id = args.get("id")
    if requested_id:
        rows = [r for r in rows if requested_id in (r.get("id"), r.get("queue_id"), r.get("unit"))]
        if len(rows) > 1:
            raise ValueError("ambiguous ID; use the qualified ID shown by ls")
        if family == "dream" and rows:
            rows[0]["attempts"] = [a for a in data.get("attempts", []) if a.get("work_id") == rows[0]["id"]]
            rows[0]["repairs"] = [r for r in data.get("repairs", []) if r.get("work_id") == rows[0]["id"]]
    if family == "dream":
        states = set(args.get("state") or [])
        if verb == "ps":
            states = {"running"}
        if states:
            selected = []
            for r in rows:
                matches = [a for a in data.get("attempts", []) if a.get("work_id") == r["id"]
                           and a.get("state") in states and in_window(a)]
                current_match = r.get("state") in states and in_window(r)
                if args.get("all"):
                    current_match = current_match and not r.get("attempt_ids")
                if current_match or args.get("all") and matches:
                    r["matching_attempt_ids"] = [a["id"] for a in matches]
                    selected.append(r)
            rows = selected
        if args.get("review"):
            rows = [r for r in rows if r.get("review") == args["review"]]
    if family == "swarm" and verb == "events":
        if args.get("subsystem"):
            rows = [r for r in rows if r.get("subsystem") in args["subsystem"]]
        if args.get("severity"):
            rows = [r for r in rows if SEVERITY.get(r.get("severity"), -1) >= SEVERITY[args["severity"]]]
    host = args.get("host")
    if host and host != "all":
        rows = [r for r in rows if r.get("host") == host]
    if since is not None or until is not None:
        # A history filter was evaluated on matching attempts, not on the
        # work row's later update time (which could be an unrelated edit).
        if not (family == "dream" and args.get("all") and args.get("state")):
            rows = [r for r in rows if in_window(r)]
    sources = [assess(c, now) for c in snapshot["coverage"]
               if (needed is None or c.get("kind") in needed)
               and (not host or host == "all" or c.get("host") == host)]
    findings = []
    if not sources:
        findings.append({"code": "NO_ENROLLED_SOURCE", "severity": "warning"})
    if requested_id and not rows:
        findings.append({"code": "NO_MATCH_IN_SEARCHED_SCOPE", "id": requested_id, "severity": "warning"})
    complete = bool(sources) and all(s.get("quality") == "verified" for s in sources)
    request_key = _request_key(path, args)
    offset = 0
    if args.get("cursor"):
        sid, req, offset = read_token(args["cursor"])
        if sid != snapshot["snapshot_id"] or req != request_key:
            raise ValueError("cursor belongs to another snapshot or query")
    limit = args.get("limit", 50)
    selected = rows[offset:offset + limit]
    next_cursor = page_token(snapshot["snapshot_id"], request_key, offset + limit) if offset + limit < len(rows) else None
    result = {"schema_version": SCHEMA, "snapshot_id": snapshot["snapshot_id"], "as_of": snapshot["as_of"],
        "query_as_of": iso(now), "config_hash": snapshot["config_hash"], "coverage": sources,
        "items": selected, "total": len(rows), "next_cursor": next_cursor, "findings": findings}
    code = 0 if complete and not (requested_id and not rows) else 3
    if family == "swarm" and verb == "check":
        scope = args.get("scope", "all")
        checks = [r for r in rows if scope == "all" or r.get("scope") == scope]
        result["items"] = checks
        if any(r.get("verdict") == "falsified" for r in checks):
            code = 1
        elif not checks or not complete or any(r.get("verdict") != "held" for r in checks):
            code = 3
    return result, code
