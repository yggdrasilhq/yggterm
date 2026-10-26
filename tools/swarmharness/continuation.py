"""continuation — reviewer succession: reviews spawn dreamers (owner directive 2026-10-26).

A review session — especially on a plane known to break across many prompts
(e.g. the zcode autoclaw plan) — must not try to outlive its context window.
At each meaningful boundary the session writes a deterministic spawn packet
from its review chain and lets a FRESH dreamer continue the same work area;
big-context planes (opencode free, ~200k) are the preferred home for
review-heavy sessions. Proven long-session seats (an orchestrator holding
the campaign thread with working context management) keep the single-session
default where they work — the law interpolates, it does not replace.

Kernel: packet build / write / validate / rederive / render / drain —
deterministic, zero model calls.

Trust model (sol turn-22): a packet is a DERIVATION of a trusted review
chain, never an assertion. validate_packet checks only schema + self-hash;
the DRAIN re-derives every packet from its review chain and rejects any
drift (forged bodies, stale heads, reviews that resolved terminal after the
packet was built). Dispatch itself re-gates the exact effective route (the
caller's enqueue_fn owns that final check — route aliases must not share a
plane-level allowlist). Every packet is processed in isolation: one bad or
crashing packet never starves its neighbors.

Packet schema v1: {kind, v, work_area, route_preference, review_base,
review_id, review_head, outcome, open_items, completion_claim, notes,
created_at, packet_hash} — packet_hash = sha256 over the canonical body
WITHOUT packet_hash; validation fails closed on any drift.
"""
import fcntl
import hashlib
import json
import os
import re
from datetime import datetime, timezone
from pathlib import Path

from swarmconfig import atomic_write_json
from session_review import SessionReview

DEFAULT_SPOOL = Path("~/.yggterm/swarm/continuations")
PACKET_KIND = "dreamer_spawn"
PACKET_VERSION = 1
_ID_RE = re.compile(r"^[A-Za-z0-9_-]+$")
_HEAD_RE = re.compile(r"^[0-9a-f]{64}$")


def _now():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _canonical(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"))


def _sha(data):
    if isinstance(data, str):
        data = data.encode()
    return hashlib.sha256(data).hexdigest()


def build_packet(base, review_id, *, work_area, route_preference=None,
                 notes=""):
    """Derive a spawn packet from a VALIDATED review chain. The chain must
    exist and parse (corrupt history raises — never a packet from a lie)."""
    r = SessionReview(review_id, base)
    rows = r.rows  # raises on corrupt/tampered chains
    if not rows or rows[0].get("kind") != "review_opened":
        raise ValueError("review_opened required")
    outcome = None
    for row in reversed(rows):
        if row["kind"] == "resolution":
            outcome = row.get("outcome")
            break
    state = r.finding_state()
    open_items = []
    for fid, st in sorted(state.items()):
        if st in ("open", "deferred_accepted"):
            entry = {"id": fid, "state": st}
            for row in rows:
                if row["kind"] == "debt_accepted" and \
                        row.get("finding_id") == fid and \
                        row.get("authority_verified") is True:
                    entry["tracking_ref"] = row.get("tracking_ref")
                    entry["revisit"] = row.get("revisit")
            open_items.append(entry)
    snaps = [row for row in rows if row["kind"] == "snapshot_staged"]
    claim = snaps[-1]["completion_claim"] if snaps else ""
    body = {"kind": PACKET_KIND, "v": PACKET_VERSION,
            "work_area": str(work_area),
            "route_preference": (str(route_preference)
                                 if route_preference else None),
            "review_base": str(base), "review_id": review_id,
            "review_head": rows[-1]["record_hash"],
            "outcome": outcome or "unresolved",
            "open_items": open_items,
            "completion_claim": claim[:300],
            "notes": str(notes)[:300], "created_at": _now()}
    body["packet_hash"] = _sha(_canonical(
        {k: v for k, v in body.items() if k != "packet_hash"}))
    return body


def validate_packet(packet):
    """Exact typed schema + hash binding. Self-consistency only — drain
    additionally re-derives from the trusted chain."""
    if not isinstance(packet, dict):
        return {"ok": False, "faults": ["packet must be a mapping"]}
    required = {"kind", "v", "work_area", "route_preference", "review_base",
                "review_id", "review_head", "outcome", "open_items",
                "completion_claim", "notes", "created_at", "packet_hash"}
    faults = []
    missing = required - set(packet)
    if missing:
        faults.append(f"missing fields: {sorted(missing)}")
    if packet.get("kind") != PACKET_KIND or \
            type(packet.get("v")) is not int or packet.get("v") != PACKET_VERSION:
        faults.append("unknown packet kind/version")
    if not isinstance(packet.get("work_area"), str) or not packet["work_area"]:
        faults.append("work_area required")
    rp = packet.get("route_preference")
    if rp is not None and (not isinstance(rp, str) or not rp.strip()):
        faults.append("route_preference must be None or a nonempty string")
    for k in ("review_base", "outcome", "completion_claim", "notes",
              "created_at"):
        if not isinstance(packet.get(k), str):
            faults.append(f"{k} must be a string")
    if not isinstance(packet.get("review_id"), str) or \
            not _ID_RE.match(packet.get("review_id") or ""):
        faults.append("invalid review id")
    if not isinstance(packet.get("review_head"), str) or \
            not _HEAD_RE.match(packet.get("review_head") or ""):
        faults.append("invalid review head")
    oi = packet.get("open_items")
    if not isinstance(oi, list) or any(
            not isinstance(i, dict) or not isinstance(i.get("id"), str)
            or not i["id"] or not isinstance(i.get("state"), str)
            for i in oi):
        faults.append("open_items must be a list of {id, state} entries")
    ph = packet.get("packet_hash")
    if not isinstance(ph, str) or not _HEAD_RE.match(ph or ""):
        faults.append("invalid packet hash")
    if faults:
        return {"ok": False, "faults": faults}
    body = {k: v for k, v in packet.items() if k != "packet_hash"}
    if _sha(_canonical(body)) != ph:
        return {"ok": False, "faults": ["packet hash mismatch (drift)"]}
    return {"ok": True}


def rederive(packet):
    """Re-derive the packet from its trusted review chain NOW. Rejects
    forged bodies, stale heads, and reviews that resolved terminal after
    the packet was built."""
    try:
        fresh = build_packet(packet["review_base"], packet["review_id"],
                             work_area=packet["work_area"])
    except Exception as exc:
        return {"ok": False, "faults": [f"chain unavailable: {exc}"[:140]]}
    if fresh["review_head"] != packet["review_head"]:
        return {"ok": False, "faults": ["stale packet: review head moved"]}
    if fresh["outcome"] in ("COMPLETE", "OWNER_STOPPED"):
        return {"ok": False,
                "faults": [f"review resolved terminal ({fresh['outcome']}) "
                           "after the packet was built"]}
    for k in ("outcome", "open_items", "completion_claim"):
        if fresh[k] != packet[k]:
            return {"ok": False, "faults": [f"packet {k} disagrees with "
                                            "the current chain"]}
    return {"ok": True}


def _spool_dirs(spool):
    root = Path(spool).expanduser() if spool else DEFAULT_SPOOL.expanduser()
    return root, root / "pending", root / "processed", root / "rejected", \
        root / "orphaned"


def _review_terminal_locked(packet, lock):
    """Terminal check while the caller holds the review's flock."""
    rows = SessionReview(packet["review_id"], packet["review_base"]).rows
    return any(r.get("kind") == "resolution"
               and r.get("outcome") in ("COMPLETE", "OWNER_STOPPED")
               for r in rows)


def _reconcile_claims(pending, orphaned, report):
    """Abandoned claims (<name>.json.claim.<pid>) are never silent: a dead
    pid's claim moves to orphaned/ (loud, never blind-retried — the enqueue
    may have been accepted before the crash); a live pid's claim is still
    owned and skipped (sol t23#9)."""
    for claim in sorted(pending.glob("*.json.claim.*")):
        try:
            pid = int(claim.name.rsplit(".", 1)[1])
        except (IndexError, ValueError):
            pid = -1
        alive = False
        if pid > 0:
            try:
                os.kill(pid, 0)
                alive = True
            except PermissionError:
                alive = True
            except OSError:
                alive = False
        if alive:
            report["notes"].append(f"live claim left alone: {claim.name}")
            continue
        orphaned.mkdir(parents=True, exist_ok=True)
        dest = orphaned / claim.name
        if dest.exists():
            dest = orphaned / (claim.name + f".{len(list(orphaned.iterdir()))}")
        claim.rename(dest)
        report["notes"].append(f"orphaned claim reconciled: {claim.name}"
                               " (never blind-retried)")


def _packet_name(packet):
    return f"{packet['review_id']}-{packet['packet_hash'][:12]}.json"


def write_packet(packet, spool=None):
    check = validate_packet(packet)
    if not check["ok"]:
        raise ValueError("; ".join(check["faults"]))
    _, pending, _, _, _ = _spool_dirs(spool)
    pending.mkdir(parents=True, exist_ok=True)
    dest = (pending / _packet_name(packet)).resolve()
    if dest.parent != pending.resolve():
        raise ValueError("packet path escapes the spool")
    if dest.exists():
        return dest
    atomic_write_json(dest, packet)
    return dest


def render_prompt(packet):
    """The spawned dreamer's brief — deterministic text from the packet."""
    items = "; ".join(
        f"{i['id']} ({i['state']}"
        + (f", tracking {i['tracking_ref']}" if i.get("tracking_ref") else "")
        + ")" for i in packet["open_items"]) or "none recorded"
    pref = (f" Preferred route: {packet['route_preference']}."
            if packet["route_preference"] else "")
    return (
        f"Continue the work area {packet['work_area']} — you are the "
        f"successor dreamer of review {packet['review_id']} "
        f"(outcome {packet['outcome']}). Open items: {items}. "
        f"Latest claim: {packet['completion_claim'] or 'none'}. "
        f"Review chain head {packet['review_head'][:12]} under "
        f"{packet['review_base']} — reopen it with SessionReview for full "
        f"history before new claims.{pref} "
        f"Notes: {packet['notes'] or 'none'}.")


def drain(spool=None, *, enqueue_fn, allowed_routes=None,
          default_route="autoclaw", route_of=None):
    """Drain pending packets, each in ISOLATION: validate -> rederive ->
    gate -> enqueue. enqueue_fn returning (False, note) keeps the packet
    pending (counts as failed); raised exceptions are caught per packet.
    Malformed/forged/stale packets go to rejected/. A packet already in
    processed/ is a duplicate and is dropped."""
    _, pending, processed, rejected, orphaned = _spool_dirs(spool)
    for d in (pending, processed, rejected, orphaned):
        d.mkdir(parents=True, exist_ok=True)
    report = {"enqueued": 0, "suppressed": 0, "rejected": 0, "failed": 0,
              "duplicates": 0, "notes": []}
    _reconcile_claims(pending, orphaned, report)
    for path in sorted(pending.glob("*.json")):
        try:
            packet = json.loads(path.read_text())
            check = validate_packet(packet)
            if not check["ok"]:
                raise ValueError(check["faults"][0])
            rd = rederive(packet)
            if not rd["ok"]:
                raise ValueError(rd["faults"][0])
            name = _packet_name(packet)
            if (processed / name).exists():
                path.unlink()
                report["duplicates"] += 1
                report["notes"].append(f"duplicate dropped: {name}")
                continue
            claim = path.with_name(path.name + f".claim.{os.getpid()}")
            try:
                path.rename(claim)  # atomic claim — a competing drain loses
            except OSError:
                report["notes"].append(f"claimed elsewhere: {name}")
                continue
            route = packet.get("route_preference") or default_route
            plane = (route_of(route) if route_of else route)
            if allowed_routes is not None and plane not in allowed_routes:
                claim.rename(path)
                report["suppressed"] += 1
                report["notes"].append(f"gate-suppressed {name} ({route})")
                continue
            # serialize dispatch acceptance against owner stop: the review's
            # own flock is HELD across the terminal re-check AND the enqueue
            # (stop_owner/resolve take the same flock — sol t23#2)
            lock_path = (Path(packet["review_base"]).expanduser()
                         / f"{packet['review_id']}.jsonl.lock")
            lock_path.parent.mkdir(parents=True, exist_ok=True)
            with open(lock_path, "a+") as lock:
                fcntl.flock(lock, fcntl.LOCK_EX)
                try:
                    if _review_terminal_locked(packet, lock):
                        claim.rename(path)
                        report["rejected"] += 1
                        report["notes"].append(
                            f"terminal after claim: {name}")
                        continue
                    try:
                        ok, note = enqueue_fn(route, packet["work_area"],
                                              render_prompt(packet))
                    except Exception as exc:
                        ok, note = False, f"enqueue raised: {exc}"[:120]
                finally:
                    fcntl.flock(lock, fcntl.LOCK_UN)
            if ok:
                claim.rename(processed / name)
                report["enqueued"] += 1
                report["notes"].append(f"enqueued {name}: {note}"[:160])
            else:
                claim.rename(path)
                report["failed"] += 1
                report["notes"].append(f"enqueue refused {name}: {note}"[:160])
        except Exception as exc:
            try:
                path.rename(rejected / path.name)
            except OSError:
                pass
            report["rejected"] += 1
            report["notes"].append(f"rejected {path.name}: {exc}"[:160])
    return report
