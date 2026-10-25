"""falsify — the claim→exposure→evaluation→disposition ledger (sol turn-7
Q2c). Extends the EXISTING overlay receipts + dream/audit ledgers; no new
service. This is what lets the editor (or a next-wake seat) FALSIFY a
policy claim instead of re-deriving it:

  claim      frozen at overlay creation: predicted effect + falsifier +
             observation deadline (policy/claims.jsonl)
  exposure   every affected dispatch decision, taken or skipped
             (receipts.jsonl decision=exposure)
  evaluation idempotent per claim+window: helped | hurt | no-observed-effect |
             confounded; disposition supported | falsified | undetermined
             (policy/evaluations.jsonl)
  consume    the orchestrator writes retention/expiry/rollback receipts
             referencing the evaluation (existing receipts.jsonl)

Laws encoded: zero failures while all work is held verifies NOTHING
(no eligible opportunity = unknown); a successful task alone establishes
nothing; automatic rollback removes ONLY this patch.
"""
import hashlib
import json
import os
from datetime import datetime, timedelta, timezone
from pathlib import Path

from swarmconfig import atomic_write_json

POLICY_DIR = Path("~/.yggterm/dream/policy")

VERDICTS = ("helped", "hurt", "no-observed-effect", "confounded")
DISPOSITIONS = ("supported", "falsified", "undetermined")


def _now():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _append(path, row):
    path = Path(str(path).replace("~", str(Path.home())))
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "a") as f:
        f.write(json.dumps(row, sort_keys=True) + "\n")
        f.flush()
        os.fsync(f.fileno())


def _read(path):
    path = Path(str(path).replace("~", str(Path.home())))
    try:
        return [json.loads(x) for x in path.read_text().splitlines() if x.strip()]
    except FileNotFoundError:
        return []


def claim_id(patch_id):
    return "claim-" + hashlib.sha256(str(patch_id).encode()).hexdigest()[:12]


def claim_open(patch_id, *, scope, predicted_effect, falsifier,
               observation_deadline_min, config_revision, finding_id,
               policy_dir=POLICY_DIR):
    """Freeze the prospective claim at overlay creation. Idempotent per
    patch_id. Returns the claim row (or the existing one)."""
    cid = claim_id(patch_id)
    existing = [c for c in _read(Path(policy_dir) / "claims.jsonl")
                if c.get("claim_id") == cid]
    if existing:
        return existing[-1]
    row = {
        "claim_id": cid, "patch_id": patch_id, "finding_id": finding_id,
        "scope": scope,
        "predicted_effect": predicted_effect,
        "falsifier": falsifier,
        "opened_at": _now(),
        "observation_deadline_min": observation_deadline_min,
        "config_revision": config_revision,
        "status": "open",          # open | evaluated | consumed
    }
    _append(Path(policy_dir) / "claims.jsonl", row)
    return row


def exposure(claim_id_, *, decision, task_id=None, policy_changed,
             detail="", policy_dir=POLICY_DIR):
    """Record an affected decision (taken or skipped — skips are exposures
    too). Appended to the overlay receipts stream."""
    row = {"t": int(datetime.now(timezone.utc).timestamp()),
           "decision": "exposure", "claim_id": claim_id_,
           "task_id": task_id, "policy_changed": bool(policy_changed),
           "detail": detail[:200], "at": _now()}
    if decision:
        row["decision_taken"] = decision
    _append(Path(policy_dir) / "receipts.jsonl", row)
    return row


def evaluate(claim_id_, *, window_start, window_end, route, exposed_tasks,
             outcome_tasks, failed_tasks, confounders=None, policy_dir=POLICY_DIR):
    """One idempotent evaluation keyed by claim+window. Zero failures while
    all work was held is UNKNOWN, never success (sol law)."""
    key = hashlib.sha256(f"{claim_id_}|{window_start}|{window_end}".encode()).hexdigest()[:12]
    existing = [e for e in _read(Path(policy_dir) / "evaluations.jsonl")
                if e.get("eval_key") == key]
    if existing:
        return existing[-1]
    held_only = (exposed_tasks == 0)
    if held_only:
        verdict, disposition = None, "undetermined"
        reason = "no eligible exposure in window (work held) — unknown, not success"
    elif failed_tasks == 0 and outcome_tasks > 0:
        verdict, disposition = "helped", "supported"
        reason = f"{outcome_tasks} post-policy attempts, 0 recurrences of the falsifier"
    elif failed_tasks > 0:
        verdict, disposition = "hurt", "falsified"
        reason = f"{failed_tasks} post-policy attempts hit the falsifier"
    else:
        verdict, disposition = "no-observed-effect", "undetermined"
        reason = "no post-policy outcomes landed in the window"
    row = {"eval_key": key, "claim_id": claim_id_, "route": route,
           "window_start": window_start, "window_end": window_end,
           "exposed_tasks": exposed_tasks, "outcome_tasks": outcome_tasks,
           "failed_tasks": failed_tasks,
           "verdict": verdict, "disposition": disposition,
           "reason": reason, "confounders": confounders or [],
           "evaluated_at": _now()}
    _append(Path(policy_dir) / "evaluations.jsonl", row)
    return row


def consume(claim_id_, evaluation, *, action, policy_dir=POLICY_DIR):
    """The orchestrator's verdict consumer: retain | expire | rollback.
    Rollback removes ONLY this patch (never broader adoption — that is the
    editor's decision)."""
    assert action in ("retain", "expire", "rollback")
    row = {"t": int(datetime.now(timezone.utc).timestamp()),
           "decision": action, "claim_id": claim_id_,
           "eval_key": (evaluation or {}).get("eval_key"),
           "verdict": (evaluation or {}).get("verdict"),
           "at": _now()}
    _append(Path(policy_dir) / "receipts.jsonl", row)
    if action == "rollback":
        # deactivate the patch this claim belongs to; nothing else
        ov_path = Path(str(policy_dir).replace("~", str(Path.home()))) / "overlays.json"
        try:
            ov = json.loads(ov_path.read_text())
        except Exception:
            ov = []
            return row
        pid = None
        for c in _read(Path(policy_dir) / "claims.jsonl"):
            if c.get("claim_id") == claim_id_:
                pid = c.get("patch_id")
        if pid is not None:
            kept = [o for o in ov if o.get("patch_id") != pid]
            tmp = ov_path.with_suffix(".tmp")
            tmp.write_text(json.dumps(kept, indent=1))
            tmp.replace(ov_path)
            row["rolled_back_patch_id"] = pid
    return row


def open_claims(policy_dir=POLICY_DIR):
    return [c for c in _read(Path(policy_dir) / "claims.jsonl")
            if c.get("status") == "open"]


def mark_consumed(claim_id_, policy_dir=POLICY_DIR):
    rows = _read(Path(policy_dir) / "claims.jsonl")
    changed = False
    for c in rows:
        if c.get("claim_id") == claim_id_ and c.get("status") == "open":
            c["status"] = "consumed"
            c["consumed_at"] = _now()
            changed = True
    if changed:
        p = Path(str(policy_dir).replace("~", str(Path.home()))) / "claims.jsonl"
        p.write_text("\n".join(json.dumps(r, sort_keys=True) for r in rows) + "\n")
    return changed
