"""session_review — the AgentWorker completion contract (sol turn-17).

Institutionalizes the handhold loop the owner observed in the turns 9-16
polish: no AgentWorker session (any harness, dreamers included) claims
completion of substantive work without independent editor acceptance, and the
session stops only when BOTH parties accept the same evidenced result.

Deterministic kernel only — staging, record validation, finding history,
dispositions, and the stop resolver. The editor INVOCATION (a model call)
stays with the seat per the consulting system; this module consumes the
verdict the editor adapter produced, and rejects worker-authored JSON that
lacks editor provenance.

Record kinds (append-only chain per review, in ~/.yggterm/swarm/reviews/):
  review_opened      ids, worker identity, acceptance criteria, tier,
                     config digest, editor binding
  snapshot_staged    round, immutable subject digest, evidence manifest,
                     worker completion claim
  editor_verdict     snapshot digest, editor provenance, finding
                     dispositions, new findings, overall accept|revise|
                     undetermined, approval scope
  worker_disposition per-finding repair|rebuttal|defer-proposal
  debt_accepted      finding, owner, tracking ref, revisit trigger, authority
  resolution         outcome COMPLETE|REVISE|SUSPENDED|OWNER_STOPPED

Laws encoded:
  - approval binds to CONTENT (stale-snapshot verdicts are rejected);
  - a worker's "fixed" never closes a finding — the editor verifies closure;
  - omitted prior findings remain open (thus REVISE);
  - quota/transport/storage exhaustion SUSPENDS — never implies acceptance;
  - deferral requires authority + durable tracking, and cannot silently
    waive an acceptance criterion (scope changes need criterion-owner
    authority — recorded on the debt).
"""
import hashlib
import json
import os
import uuid
from datetime import datetime, timezone
from pathlib import Path

from swarmconfig import atomic_write_json

DEFAULT_REVIEWS = Path("~/.yggterm/swarm/reviews")

TIERS = ("full", "lightweight")
OVERALLS = ("accept", "revise", "undetermined")
DISPOSITIONS = ("repair", "rebuttal", "defer_proposed")
OUTCOMES = ("COMPLETE", "REVISE", "SUSPENDED", "OWNER_STOPPED")
FINDING_STATES = ("open", "closed_by_editor", "deferred_accepted")


def _now():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _sha(data):
    if isinstance(data, str):
        data = data.encode()
    return hashlib.sha256(data).hexdigest()


class SessionReview:
    def __init__(self, review_id, base=DEFAULT_REVIEWS):
        self.review_id = review_id
        self.path = Path(base).expanduser() / f"{review_id}.jsonl"
        self._rows = None

    @classmethod
    def create(cls, *, worker, acceptance_criteria, tier="full",
               config_digest=None, editor_binding=None, base=DEFAULT_REVIEWS,
               review_id=None):
        if tier not in TIERS:
            raise ValueError(f"tier must be one of {TIERS}")
        if not acceptance_criteria:
            raise ValueError("acceptance_criteria required")
        rid = review_id or f"rev-{uuid.uuid4().hex[:12]}"
        r = cls(rid, base)
        if r.path.exists():
            raise ValueError(f"review {rid} already exists")
        r._append({"kind": "review_opened", "review_id": rid,
                   "worker": worker, "acceptance_criteria": acceptance_criteria,
                   "tier": tier, "config_digest": config_digest,
                   "editor_binding": editor_binding, "at": _now()})
        return r

    # ------------------------------------------------------------ storage
    def _append(self, row):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with open(self.path, "a") as f:
            f.write(json.dumps(row, sort_keys=True) + "\n")
            f.flush()
            os.fsync(f.fileno())
        self._rows = None

    @property
    def rows(self):
        if self._rows is None:
            rows = []
            if self.path.exists():
                for x in self.path.read_text().splitlines():
                    try:
                        rows.append(json.loads(x))
                    except Exception:
                        continue
            self._rows = rows
        return self._rows

    def _last(self, kind):
        for row in reversed(self.rows):
            if row["kind"] == kind:
                return row
        return None

    # ------------------------------------------------------------ staging
    def stage_snapshot(self, *, subject, base_revision=None,
                       evidence_manifest=None, completion_claim):
        """subject: the exact work artifact(s) — pass file paths (hashed) or
        a string payload. Immutable digest binds all later verdicts."""
        if isinstance(subject, (list, tuple)) and subject and all(
                Path(str(p)).exists() for p in subject):
            digest = _sha("".join(
                _sha(Path(str(p)).read_bytes()) for p in sorted(map(str, subject))))
        else:
            digest = _sha(str(subject))
        row = {"kind": "snapshot_staged", "round": len(self.snapshots()) + 1,
               "subject_digest": digest, "base_revision": base_revision,
               "evidence_manifest": evidence_manifest or [],
               "completion_claim": str(completion_claim)[:500], "at": _now()}
        self._append(row)
        return row

    def snapshots(self):
        return [r for r in self.rows if r["kind"] == "snapshot_staged"]

    # ------------------------------------------------------------ verdict
    def ingest_verdict(self, *, snapshot_digest, editor, findings,
                       overall, approval_scope=None, prior_dispositions=None,
                       checks_evidence=None):
        """editor: {identity, invocation_ref} — provenance REQUIRED; a verdict
        without it is rejected as worker-authored (never editor approval).
        findings: [{id, criterion, blocking, evidence, expected, observed,
        probe}]. prior_dispositions: [{id, closed: bool}] — the editor's own
        closure decisions on earlier findings."""
        if overall not in OVERALLS:
            return {"ok": False, "faults": [f"overall must be in {OVERALLS}"]}
        if not isinstance(editor, dict) or not editor.get("identity") \
                or not editor.get("invocation_ref"):
            return {"ok": False,
                    "faults": ["verdict lacks editor provenance (identity + "
                               "invocation_ref) — worker-authored JSON is not "
                               "editor approval"]}
        snaps = self.snapshots()
        if not snaps or snaps[-1]["subject_digest"] != snapshot_digest:
            return {"ok": False,
                    "faults": ["verdict digest does not match the latest staged "
                               "snapshot (stale-snapshot approval refused)"]}
        norm = []
        for f in findings or []:
            if not isinstance(f, dict) or not f.get("id") or not f.get("criterion"):
                return {"ok": False, "faults": [f"finding missing id/criterion: {f!r}"[:160]]}
            norm.append({"id": f["id"], "criterion": f["criterion"],
                         "blocking": bool(f.get("blocking")),
                         "evidence": str(f.get("evidence", ""))[:300],
                         "expected": str(f.get("expected", ""))[:200],
                         "observed": str(f.get("observed", ""))[:200],
                         "probe": f.get("probe")})
        row = {"kind": "editor_verdict", "snapshot_digest": snapshot_digest,
               "editor": editor, "overall": overall,
               "approval_scope": approval_scope,
               "checks_evidence": checks_evidence,
               "findings": norm, "prior_dispositions": prior_dispositions or [],
               "at": _now()}
        self._append(row)
        return {"ok": True, "findings": norm}

    def disposition(self, finding_id, action, evidence_refs=None):
        if action not in DISPOSITIONS:
            return {"ok": False, "faults": [f"action must be in {DISPOSITIONS}"]}
        self._append({"kind": "worker_disposition", "finding_id": finding_id,
                      "action": action, "evidence_refs": evidence_refs or [],
                      "at": _now()})
        return {"ok": True}

    def defer_accepted(self, finding_id, *, owner, tracking_ref, revisit,
                       authority, waives_criterion=False):
        """A deferral with authority recorded. waives_criterion=True means the
        deferral gives up an acceptance criterion — that is a scope change the
        criterion's owner must authorize (recorded here; the resolver surfaces
        it in COMPLETE's unresolved list)."""
        row = {"kind": "debt_accepted", "finding_id": finding_id, "owner": owner,
               "tracking_ref": tracking_ref, "revisit": revisit,
               "authority": authority, "waives_criterion": bool(waives_criterion),
               "at": _now()}
        self._append(row)
        return row

    def suspend(self, reason):
        self._append({"kind": "resolution", "outcome": "SUSPENDED",
                      "reason": str(reason)[:300], "at": _now()})

    def stop_owner(self, reason):
        self._append({"kind": "resolution", "outcome": "OWNER_STOPPED",
                      "reason": str(reason)[:300], "at": _now()})

    # ------------------------------------------------------------ resolver
    def finding_state(self):
        """-> {finding_id: state}. Editor closures and accepted debts are
        terminal; worker 'repair' NEVER closes (editor verifies)."""
        state = {}
        for r in self.rows:
            if r["kind"] == "editor_verdict":
                for f in r.get("findings", []):
                    state.setdefault(f["id"], "open")
                for d in r.get("prior_dispositions", []):
                    if d.get("closed"):
                        state[d["id"]] = "closed_by_editor"
                    else:
                        state[d["id"]] = "open"
            elif r["kind"] == "worker_disposition":
                state.setdefault(r["finding_id"], "open")  # never closes
            elif r["kind"] == "debt_accepted":
                state[r["finding_id"]] = "deferred_accepted"
        return state

    def resolve(self, *, current_subject_digest=None, checks_evidence=None):
        """The stop resolver (sol turn-17). Returns
        {outcome, reasons, unresolved}."""
        if any(r["kind"] == "resolution" and r["outcome"] == "OWNER_STOPPED"
               for r in self.rows):
            return {"outcome": "OWNER_STOPPED", "reasons": ["owner stopped the work"],
                    "unresolved": [k for k, v in self.finding_state().items()
                                   if v != "closed_by_editor"]}
        if any(r["kind"] == "resolution" and r["outcome"] == "SUSPENDED"
               for r in self.rows):
            return {"outcome": "SUSPENDED", "reasons": ["review suspended "
                    "(quota/transport/storage) — retention, never acceptance"],
                    "unresolved": [k for k, v in self.finding_state().items()
                                   if v != "closed_by_editor"]}
        verdicts = [r for r in self.rows if r["kind"] == "editor_verdict"]
        snaps = self.snapshots()
        reasons = []
        if not verdicts:
            return {"outcome": "REVISE", "reasons": ["no editor verdict yet"],
                    "unresolved": []}
        last = verdicts[-1]
        cur = current_subject_digest or (snaps[-1]["subject_digest"] if snaps else None)
        if cur != last["snapshot_digest"]:
            reasons.append("current subject differs from the reviewed snapshot "
                           "(delta requires review)")
        if last["overall"] != "accept":
            reasons.append(f"editor overall is {last['overall']}")
        if checks_evidence is not None:
            if not checks_evidence:
                reasons.append("required checks have no evidence")
        elif last.get("checks_evidence") is None:
            reasons.append("no checks evidence bound to the reviewed snapshot")
        state = self.finding_state()
        unresolved = sorted(k for k, v in state.items() if v != "closed_by_editor")
        if unresolved:
            reasons.append(f"findings not editor-closed: {unresolved}")
        debts = [r for r in self.rows if r["kind"] == "debt_accepted"]
        if any(d.get("waives_criterion") for d in debts):
            reasons.append("a deferred finding waives an acceptance criterion "
                           "(scope change surfaced for criterion-owner)")
        if reasons:
            return {"outcome": "REVISE", "reasons": reasons, "unresolved": unresolved}
        return {"outcome": "COMPLETE", "reasons": [], "unresolved": []}


# ------------------------------------------------------------------- CLI

def main():
    import argparse
    ap = argparse.ArgumentParser(prog="session-review")
    ap.add_argument("verb", choices=["resolve", "findings", "tail"])
    ap.add_argument("--review", required=True)
    ap.add_argument("--base", default=str(DEFAULT_REVIEWS))
    ap.add_argument("--current-digest", default=None)
    ap.add_argument("--checks-evidence", default=None)
    a = ap.parse_args()
    r = SessionReview(a.review, a.base)
    if not r.path.exists():
        print(json.dumps({"faults": [f"unknown review {a.review}"]}))
        return 2
    if a.verb == "resolve":
        print(json.dumps(r.resolve(current_subject_digest=a.current_digest,
                                   checks_evidence=a.checks_evidence), indent=2))
        return 0
    if a.verb == "findings":
        print(json.dumps(r.finding_state(), indent=2))
        return 0
    if a.verb == "tail":
        for row in r.rows[-5:]:
            print(json.dumps(row, sort_keys=True)[:220])
        return 0
    return 1


if __name__ == "__main__":
    import sys
    sys.exit(main())
