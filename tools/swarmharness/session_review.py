"""session_review — the AgentWorker completion contract (sol turn-17; turn-18 hardening).

Institutionalizes the handhold loop the owner observed in the turns 9-16
polish: no AgentWorker session (any harness, dreamers included) claims
completion of substantive work without independent editor acceptance, and the
session stops only when BOTH parties accept the same evidenced result.

Deterministic kernel only — staging, record validation, finding history,
dispositions, and the stop resolver. The editor INVOCATION (a model call)
stays with the seat per the consulting system; this module consumes the
verdict the editor adapter produced.

Hardening (sol turn-18 conformance pass, F01-F10):
  - storage is a hash-linked append-only chain (seq/prev_hash/record_hash);
    corrupt, rewritten or reordered history raises instead of approving;
  - every mutating verb + the resolver run under a per-review flock with a
    fresh validated read — no cached history, no lost finalization; completion
    is durable (a resolution record) and idempotent; terminal reviews
    (COMPLETE/OWNER_STOPPED) refuse further writes — open a delta review;
  - subjects are typed manifests (payload text, or file paths hashed by
    content); the live bytes are rehashed at verdict time and resolution —
    drift after staging is refused, never approved;
  - editor provenance, debt authority and none-tier exemptions are verified
    through INJECTED trusted verifiers (HMAC receipts over the exact payload,
    keyed on the swarm control key). No verifier wired -> the path is HELD,
    never assumed. Worker-authored verdict JSON is never editor approval;
  - findings are strictly typed, criteria-bound, and ID-stable; a renewed
    editor objection REOPENS its finding; `closed: "false"` is a fault;
  - approval must cover every acceptance criterion; checks evidence must be
    passing receipts bound to the reviewed snapshot; the resolver can never
    substitute unreviewed evidence; a nonempty worker completion claim is
    required at staging.

Record kinds (append-only chain per review, in ~/.yggterm/swarm/reviews/):
  review_opened      ids, worker identity, acceptance criteria, tier,
                     config digest, editor binding, exemption receipt
  snapshot_staged    round, subject digest + manifest, evidence manifest,
                     worker completion claim
  editor_verdict     snapshot digest, editor provenance + invocation receipt,
                     finding dispositions, new findings, overall, scope,
                     checks receipts
  worker_disposition per-finding repair|rebuttal|defer_proposed (never closes)
  debt_accepted      finding, owner, tracking ref, revisit, verified authority
  resolution         outcome COMPLETE|REVISE|SUSPENDED|OWNER_STOPPED + result
"""
import fcntl
import hashlib
import hmac
import json
import os
import uuid
from datetime import datetime, timezone
from functools import wraps
from pathlib import Path

DEFAULT_REVIEWS = Path("~/.yggterm/swarm/reviews")
CONTROL_KEY = Path("~/.yggterm/swarm/control/owner.key")

TIERS = ("full", "lightweight", "none")
OVERALLS = ("accept", "revise", "undetermined")
DISPOSITIONS = ("repair", "rebuttal", "defer_proposed")
OUTCOMES = ("COMPLETE", "REVISE", "SUSPENDED", "OWNER_STOPPED")
FINDING_STATES = ("open", "closed_by_editor", "deferred_accepted")
RECORD_KINDS = ("review_opened", "snapshot_staged", "editor_verdict",
                "worker_disposition", "debt_accepted", "resolution")
TERMINAL_OUTCOMES = ("COMPLETE", "OWNER_STOPPED")
_ID_CHARS = frozenset(
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_")
_GENESIS = "0" * 64


def _now():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _sha(data):
    if isinstance(data, str):
        data = data.encode()
    return hashlib.sha256(data).hexdigest()


def _canonical(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"))


def _subject_identity(subject):
    """Typed subject manifest -> (digest, manifest). File sets are bound by
    content hash per path; a bare string is a payload. Rejects missing or
    duplicated artifacts so a pathname can never masquerade as content."""
    if isinstance(subject, Path):
        subject = [subject]
    if isinstance(subject, (list, tuple)):
        if not subject:
            raise ValueError("empty artifact set")
        files = []
        for item in subject:
            if not isinstance(item, (str, Path)):
                raise ValueError("artifact paths must be strings or Paths")
            p = Path(item).absolute()
            if not p.is_file():
                raise ValueError(f"missing artifact: {p}")
            files.append({"path": str(p), "sha256": _sha(p.read_bytes())})
        files.sort(key=lambda f: f["path"])
        if len({f["path"] for f in files}) != len(files):
            raise ValueError("duplicate artifact paths")
        manifest = {"kind": "files", "files": files}
    elif isinstance(subject, str):
        manifest = {"kind": "payload", "sha256": _sha(subject)}
    else:
        raise ValueError("subject must be payload text, Path, or artifact list")
    return _sha(_canonical(manifest)), manifest


def _live_digest(snapshot):
    """Rehash the staged file set NOW, or None for payload subjects (whose
    callers must attest the current digest explicitly at resolution)."""
    m = (snapshot or {}).get("subject_manifest") or {}
    if m.get("kind") != "files":
        return None
    return _subject_identity([f["path"] for f in m["files"]])[0]


# ------------------------------------------------------- receipt adapters
# The trust boundary: receipts are HMACs under the swarm control key. The
# EDITOR-side process holds the key and signs; worker-side handles verify.
# No verifier wired (or key absent) means the path is HELD — fail closed.

def _receipt_material(review_id, opened, snapshot, editor, payload):
    return {"kind": "editor-invocation", "review_id": review_id,
            "opened": opened, "snapshot_digest": (snapshot or {}).get("subject_digest"),
            "editor_identity": (editor or {}).get("identity"), "payload": payload}


def editor_receipt(review_id, opened, snapshot, editor, payload, *, key):
    if isinstance(key, str):
        key = key.encode()
    return hmac.new(key, _canonical(_receipt_material(
        review_id, opened, snapshot, editor, payload)).encode(),
        hashlib.sha256).hexdigest()


def verdict_payload(overall, findings, prior_dispositions=None,
                    approval_scope=None, checks_evidence=None):
    """The exact verdict payload covered by an invocation receipt — build the
    receipt from THIS object and pass the same args to ingest_verdict."""
    return {"overall": overall, "findings": findings,
            "prior_dispositions": list(prior_dispositions or []),
            "approval_scope": approval_scope, "checks_evidence": checks_evidence}


def debt_receipt(review_id, opened, history, debt, *, key):
    if isinstance(key, str):
        key = key.encode()
    material = {"kind": "debt-authority", "review_id": review_id,
                "opened": opened, "finding_history": history,
                "debt": {k: v for k, v in debt.items()
                         if k != "authority_receipt"}}
    return hmac.new(key, _canonical(material).encode(),
                    hashlib.sha256).hexdigest()


def exemption_receipt(review_id, acceptance_criteria, snapshot_digest,
                      current_subject_digest, *, key):
    if isinstance(key, str):
        key = key.encode()
    material = {"kind": "review-exemption", "review_id": review_id,
                "acceptance_criteria": list(acceptance_criteria),
                "snapshot_digest": snapshot_digest,
                "current_digest": current_subject_digest}
    return hmac.new(key, _canonical(material).encode(),
                    hashlib.sha256).hexdigest()


def _load_key(key_path=None):
    p = Path(key_path).expanduser() if key_path else CONTROL_KEY.expanduser()
    if not p.is_file():
        raise ValueError(f"control key not found: {p}")
    return p.read_bytes().strip()


def hmac_provenance_verifier(key_path=None):
    key = _load_key(key_path)

    def verify(review_id, opened, snapshot, editor, payload):
        rcpt = (editor or {}).get("invocation_receipt")
        if not isinstance(rcpt, str) or not rcpt:
            return False
        return hmac.compare_digest(rcpt, editor_receipt(
            review_id, opened, snapshot, editor, payload, key=key))
    return verify


def hmac_debt_verifier(key_path=None):
    key = _load_key(key_path)

    def verify(review_id, opened, history, debt):
        rcpt = (debt or {}).get("authority_receipt")
        if not isinstance(rcpt, str) or not rcpt:
            return False
        return hmac.compare_digest(rcpt, debt_receipt(
            review_id, opened, history, debt, key=key))
    return verify


def hmac_exemption_verifier(key_path=None):
    key = _load_key(key_path)

    def verify(review_id, opened, snapshot, receipt, current_subject_digest):
        if not isinstance(receipt, str) or not receipt:
            return False
        return hmac.compare_digest(receipt, exemption_receipt(
            review_id, opened.get("acceptance_criteria") or [],
            (snapshot or {}).get("subject_digest"), current_subject_digest,
            key=key))
    return verify


def _serialized(method):
    """One interprocess lock per review around read/check/append, with a
    fresh validated history every time — no stale-cache completion."""
    @wraps(method)
    def transaction(self, *args, **kwargs):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        with open(f"{self.path}.lock", "a+") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            try:
                rows = self._read_rows()
                if not rows:
                    raise ValueError("review_opened required")
                first = rows[0]
                if (first.get("kind") != "review_opened"
                        or first.get("review_id") != self.review_id):
                    raise ValueError("review identity mismatch")
                terminal = any(r.get("kind") == "resolution"
                               and r.get("outcome") in TERMINAL_OUTCOMES
                               for r in rows)
                if terminal and method.__name__ != "resolve":
                    raise ValueError("terminal review; open a new delta review")
                return method(self, *args, **kwargs)
            finally:
                fcntl.flock(lock, fcntl.LOCK_UN)
    return transaction


class SessionReview:
    def __init__(self, review_id, base=DEFAULT_REVIEWS, *,
                 provenance_verifier=None, debt_verifier=None,
                 exemption_verifier=None):
        if (not isinstance(review_id, str) or not review_id
                or any(c not in _ID_CHARS for c in review_id)):
            raise ValueError("invalid review id")
        self.review_id = review_id
        self.path = Path(base).expanduser() / f"{review_id}.jsonl"
        self._provenance_verifier = provenance_verifier
        self._debt_verifier = debt_verifier
        self._exemption_verifier = exemption_verifier

    @classmethod
    def create(cls, *, worker, acceptance_criteria, tier="full",
               config_digest=None, editor_binding=None, base=DEFAULT_REVIEWS,
               review_id=None, exemption_receipt=None,
               provenance_verifier=None, debt_verifier=None,
               exemption_verifier=None):
        if tier not in TIERS:
            raise ValueError(f"tier must be one of {TIERS}")
        if (not isinstance(acceptance_criteria, list) or not acceptance_criteria
                or any(not isinstance(c, str) or not c.strip()
                       for c in acceptance_criteria)):
            raise ValueError("acceptance_criteria must be a nonempty "
                             "list of nonempty strings")
        rid = review_id or f"rev-{uuid.uuid4().hex[:12]}"
        r = cls(rid, base, provenance_verifier=provenance_verifier,
                debt_verifier=debt_verifier, exemption_verifier=exemption_verifier)
        r.path.parent.mkdir(parents=True, exist_ok=True)
        with open(f"{r.path}.lock", "a+") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            try:
                if r.path.exists():
                    raise ValueError(f"review {rid} already exists")
                r._chain_append({"kind": "review_opened", "review_id": rid,
                                 "worker": worker,
                                 "acceptance_criteria": list(acceptance_criteria),
                                 "tier": tier,
                                 "exemption_receipt": exemption_receipt,
                                 "config_digest": config_digest,
                                 "editor_binding": editor_binding,
                                 "at": _now()})
            finally:
                fcntl.flock(lock, fcntl.LOCK_UN)
        return r

    # ------------------------------------------------------------ storage
    def _chain_append(self, row):
        """Caller holds the review lock. Extends the hash chain."""
        if row.get("kind") not in RECORD_KINDS:
            raise ValueError("unknown record kind")
        rows = self._read_rows()
        if rows and row["kind"] == "review_opened":
            raise ValueError("duplicate opening event")
        row = dict(row, seq=len(rows),
                   prev_hash=rows[-1]["record_hash"] if rows else _GENESIS)
        row["record_hash"] = _sha(_canonical(row))
        with open(self.path, "a") as f:
            f.write(json.dumps(row, sort_keys=True) + "\n")
            f.flush()
            os.fsync(f.fileno())

    @property
    def rows(self):
        """Fresh validated history on EVERY read — corrupt, rewritten or
        reordered records raise instead of silently approving."""
        return self._read_rows()

    def _read_rows(self):
        rows = []
        if not self.path.exists():
            return rows
        prev = _GENESIS
        for seq, line in enumerate(self.path.read_text().splitlines()):
            try:
                row = json.loads(line)
                if not isinstance(row, dict):
                    raise ValueError("record must be a mapping")
                body = {k: v for k, v in row.items() if k != "record_hash"}
                digest = _sha(_canonical(body))
                if (row.get("seq") != seq or row.get("prev_hash") != prev
                        or row.get("record_hash") != digest
                        or row.get("kind") not in RECORD_KINDS
                        or (seq == 0) != (row.get("kind") == "review_opened")):
                    raise ValueError("invalid record chain")
            except (ValueError, TypeError) as e:
                raise ValueError(f"corrupt review at record {seq}") from e
            rows.append(row)
            prev = digest
        if rows and rows[0].get("review_id") != self.review_id:
            raise ValueError("review identity mismatch")
        return rows

    def _last(self, kind):
        for row in reversed(self._read_rows()):
            if row["kind"] == kind:
                return row
        return None

    # ------------------------------------------------------------ staging
    @_serialized
    def stage_snapshot(self, *, subject, base_revision=None,
                       evidence_manifest=None, completion_claim):
        """subject: the exact work artifact(s) — file paths (bound by content)
        or a payload string. A nonempty completion claim is the worker's own
        acceptance of this snapshot."""
        if not isinstance(completion_claim, str) or not completion_claim.strip():
            raise ValueError("explicit worker acceptance claim required")
        digest, manifest = _subject_identity(subject)
        row = {"kind": "snapshot_staged", "round": len(self.snapshots()) + 1,
               "subject_digest": digest, "subject_manifest": manifest,
               "base_revision": base_revision,
               "evidence_manifest": evidence_manifest or [],
               "completion_claim": completion_claim[:500], "at": _now()}
        self._chain_append(row)
        return row

    def snapshots(self):
        return [r for r in self.rows if r["kind"] == "snapshot_staged"]

    # ------------------------------------------------------------ verdict
    @_serialized
    def ingest_verdict(self, *, snapshot_digest, editor, findings,
                       overall, approval_scope=None, prior_dispositions=None,
                       checks_evidence=None):
        """editor: {identity, invocation_ref, invocation_receipt} — the
        receipt must verify under the injected provenance verifier, which
        signs the EXACT payload (verdict_payload). findings:
        [{id, criterion, blocking, evidence, expected, observed, probe}];
        prior_dispositions: [{id, closed: bool}] — editor closure decisions.
        The live file set is rehashed: drift since staging is refused."""
        if overall not in OVERALLS:
            return {"ok": False, "faults": [f"overall must be in {OVERALLS}"]}
        opened = self._last("review_opened")
        if not isinstance(editor, dict) or any(
                not isinstance(editor.get(k), str) or not editor[k].strip()
                for k in ("identity", "invocation_ref")):
            return {"ok": False,
                    "faults": ["verdict lacks editor provenance (identity + "
                               "invocation_ref) — worker-authored JSON is not "
                               "editor approval"]}
        snaps = self.snapshots()
        if not snaps or snaps[-1]["subject_digest"] != snapshot_digest:
            return {"ok": False,
                    "faults": ["verdict digest does not match the latest staged "
                               "snapshot (stale-snapshot approval refused)"]}
        try:
            live = _live_digest(snaps[-1])
        except (ValueError, OSError):
            return {"ok": False, "faults": ["subject unavailable since staging"]}
        if live is not None and live != snapshot_digest:
            return {"ok": False,
                    "faults": ["subject changed since staging"]}
        prior_dispositions = [] if prior_dispositions is None else prior_dispositions
        if not isinstance(findings, list) or not isinstance(prior_dispositions, list):
            return {"ok": False,
                    "faults": ["findings and dispositions must be lists"]}
        criteria = opened["acceptance_criteria"]
        history = {f["id"]: f for r in self.rows
                   if r["kind"] == "editor_verdict"
                   for f in r.get("findings", [])}
        ids = set()
        for f in findings:
            if (not isinstance(f, dict)
                    or any(not isinstance(f.get(k), str) or not f[k].strip()
                           for k in ("id", "criterion", "evidence",
                                     "expected", "observed"))
                    or type(f.get("blocking")) is not bool
                    or not f.get("probe")):
                return {"ok": False,
                        "faults": ["incomplete or ill-typed finding: "
                                   f"{f!r}"[:160]]}
            if f["id"] in ids or f["criterion"] not in criteria:
                return {"ok": False,
                        "faults": ["duplicate finding id or unknown criterion"]}
            if f["id"] in history and history[f["id"]]["criterion"] != f["criterion"]:
                return {"ok": False,
                        "faults": ["finding id rebound to another criterion"]}
            ids.add(f["id"])
        seen = set()
        for d in prior_dispositions:
            if (not isinstance(d, dict) or not isinstance(d.get("id"), str)
                    or d["id"] not in history
                    or type(d.get("closed")) is not bool
                    or d["id"] in seen or d["id"] in ids):
                return {"ok": False,
                        "faults": ["invalid or conflicting prior disposition"]}
            seen.add(d["id"])
        if (approval_scope is not None
                and (not isinstance(approval_scope, list)
                     or any(not isinstance(c, str) for c in approval_scope)
                     or set(approval_scope) - set(criteria))):
            return {"ok": False,
                    "faults": ["approval scope must list acceptance criteria"]}
        if checks_evidence is not None:
            if (not isinstance(checks_evidence, list) or not checks_evidence
                    or any(not isinstance(e, dict)
                           or e.get("status") != "pass"
                           or e.get("snapshot_digest") != snapshot_digest
                           or not isinstance(e.get("receipt_ref"), str)
                           or not e["receipt_ref"].strip()
                           for e in checks_evidence)):
                return {"ok": False,
                        "faults": ["checks evidence must be passing receipts "
                                   "bound to the reviewed snapshot"]}
        payload = verdict_payload(overall, findings, prior_dispositions,
                                  approval_scope, checks_evidence)
        try:
            verified = (self._provenance_verifier is not None
                        and self._provenance_verifier(
                            self.review_id, opened, snaps[-1], editor,
                            payload) is True)
        except Exception:
            verified = False
        if not verified:
            return {"ok": False,
                    "faults": ["editor invocation not authenticated"]}
        norm = [{"id": f["id"], "criterion": f["criterion"],
                 "blocking": f["blocking"],
                 "evidence": f["evidence"][:300],
                 "expected": f["expected"][:200],
                 "observed": f["observed"][:200],
                 "probe": f["probe"]}
                for f in findings]
        row = {"kind": "editor_verdict", "snapshot_digest": snapshot_digest,
               "editor": editor, "overall": overall,
               "approval_scope": approval_scope,
               "checks_evidence": checks_evidence,
               "findings": norm,
               "prior_dispositions": prior_dispositions,
               "at": _now()}
        self._chain_append(row)
        return {"ok": True, "findings": norm}

    @_serialized
    def disposition(self, finding_id, action, evidence_refs=None):
        if not isinstance(finding_id, str) or finding_id not in self.finding_state():
            return {"ok": False, "faults": ["unknown finding id"]}
        if action not in DISPOSITIONS:
            return {"ok": False, "faults": [f"action must be in {DISPOSITIONS}"]}
        self._chain_append({"kind": "worker_disposition",
                            "finding_id": finding_id, "action": action,
                            "evidence_refs": evidence_refs or [], "at": _now()})
        return {"ok": True}

    @_serialized
    def defer_accepted(self, finding_id, *, owner, tracking_ref, revisit,
                       authority=None, waives_criterion=False,
                       authority_receipt=None):
        """A deferral is only accepted when the injected debt verifier
        authenticates criterion-owner authority (the worker's waiver flag is
        never authority by itself). waives_criterion=True means the deferral
        gives up an acceptance criterion — a scope change only the criterion's
        owner can authorize."""
        if (not isinstance(finding_id, str)
                or finding_id not in self.finding_state()
                or type(waives_criterion) is not bool
                or any(not isinstance(v, str) or not v.strip()
                       for v in (owner, tracking_ref, revisit))):
            return {"ok": False, "faults": ["invalid debt fields or unknown finding"]}
        history = [f for r in self.rows if r["kind"] == "editor_verdict"
                   for f in r.get("findings", []) if f["id"] == finding_id]
        debt = {"finding_id": finding_id, "owner": owner,
                "tracking_ref": tracking_ref, "revisit": revisit,
                "authority": authority,
                "waives_criterion": bool(waives_criterion),
                "authority_receipt": authority_receipt}
        try:
            verified = (self._debt_verifier is not None and self._debt_verifier(
                self.review_id, self._last("review_opened"), history,
                debt) is True)
        except Exception:
            verified = False
        if not verified:
            return {"ok": False, "faults": ["debt authority not authenticated"]}
        row = {"kind": "debt_accepted", "authority_verified": True,
               "finding_id": finding_id, "owner": owner,
               "tracking_ref": tracking_ref, "revisit": revisit,
               "authority": authority,
               "waives_criterion": bool(waives_criterion), "at": _now()}
        self._chain_append(row)
        return row

    @_serialized
    def suspend(self, reason):
        self._chain_append({"kind": "resolution", "outcome": "SUSPENDED",
                            "reason": str(reason)[:300], "at": _now()})

    @_serialized
    def stop_owner(self, reason):
        self._chain_append({"kind": "resolution", "outcome": "OWNER_STOPPED",
                            "reason": str(reason)[:300], "at": _now()})

    # ------------------------------------------------------------ resolver
    def finding_state(self):
        """-> {finding_id: state}. A new editor objection reopens its ID;
        worker 'repair' NEVER closes; only verified debts defer."""
        state = {}
        for r in self.rows:
            if r["kind"] == "editor_verdict":
                for f in r.get("findings", []):
                    state[f["id"]] = "open"
                for d in r.get("prior_dispositions", []):
                    state[d["id"]] = "closed_by_editor" if d.get("closed") else "open"
            elif r["kind"] == "worker_disposition":
                state.setdefault(r["finding_id"], "open")  # never closes
            elif r["kind"] == "debt_accepted" and r.get("authority_verified") is True:
                state[r["finding_id"]] = "deferred_accepted"
        return state

    @_serialized
    def resolve(self, *, current_subject_digest=None, checks_evidence=None):
        """The stop resolver (sol turn-17). Durable + idempotent: the first
        terminal outcome is recorded and returned verbatim forever; a
        non-terminal outcome equal to the last recorded one is not
        re-recorded (concurrent resolvers converge on one row)."""
        rows = self.rows
        for r in reversed(rows):
            if r["kind"] == "resolution" and r.get("outcome") in TERMINAL_OUTCOMES:
                recorded = r.get("result") or {
                    "outcome": r["outcome"], "reasons": [], "unresolved": []}
                return dict(recorded)
        result = self._evaluate(current_subject_digest=current_subject_digest,
                                checks_evidence=checks_evidence)
        for r in reversed(rows):
            if r["kind"] == "resolution":
                if (r.get("outcome") == result["outcome"]
                        and r.get("result") == result):
                    return result
                break
        self._chain_append({"kind": "resolution",
                            "outcome": result["outcome"],
                            "result": result, "at": _now()})
        return result

    def _evaluate(self, *, current_subject_digest=None, checks_evidence=None):
        rows = self.rows
        if any(r["kind"] == "resolution" and r["outcome"] == "OWNER_STOPPED"
               for r in rows):
            return {"outcome": "OWNER_STOPPED", "reasons": ["owner stopped the work"],
                    "unresolved": [k for k, v in self.finding_state().items()
                                   if v != "closed_by_editor"]}
        opened = self._last("review_opened")
        if opened and opened.get("tier") == "none":
            return self._evaluate_none(opened, current_subject_digest)
        res_idx = [i for i, r in enumerate(rows) if r["kind"] == "resolution"]
        if res_idx:
            last_i = res_idx[-1]
            if (rows[last_i].get("outcome") == "SUSPENDED"
                    and not any(r["kind"] in ("snapshot_staged", "editor_verdict",
                                              "worker_disposition", "debt_accepted")
                                for r in rows[last_i + 1:])):
                return {"outcome": "SUSPENDED",
                        "reasons": ["review suspended (quota/transport/storage) "
                                    "— retention, never acceptance"],
                        "unresolved": [k for k, v in self.finding_state().items()
                                       if v != "closed_by_editor"]}
        verdicts = [r for r in rows if r["kind"] == "editor_verdict"]
        snaps = [r for r in rows if r["kind"] == "snapshot_staged"]
        if not verdicts:
            return {"outcome": "REVISE", "reasons": ["no editor verdict yet"],
                    "unresolved": []}
        last = verdicts[-1]
        reasons = []
        if not snaps or snaps[-1]["subject_digest"] != last["snapshot_digest"]:
            reasons.append("latest snapshot has not been reviewed")
        try:
            live = _live_digest(snaps[-1]) if snaps else None
        except (ValueError, OSError):
            live = None
            reasons.append("current artifacts unavailable")
        cur = live if live is not None else current_subject_digest
        if (current_subject_digest is not None and live is not None
                and current_subject_digest != live):
            reasons.append("caller digest disagrees with current artifacts")
        if cur != last["snapshot_digest"]:
            reasons.append("current subject differs from the reviewed snapshot "
                           "(delta requires review)")
        if last["overall"] != "accept":
            reasons.append(f"editor overall is {last['overall']}")
        criteria = opened["acceptance_criteria"] if opened else []
        scope = last.get("approval_scope")
        if (not isinstance(scope, list) or any(not isinstance(c, str) for c in scope)
                or not criteria or set(scope) != set(criteria)):
            reasons.append("editor approval does not cover acceptance criteria")
        if not snaps or not (snaps[-1].get("completion_claim") or "").strip():
            reasons.append("worker has not accepted this snapshot")
        evidence = last.get("checks_evidence")
        if checks_evidence is not None and checks_evidence != evidence:
            reasons.append("resolver cannot substitute unreviewed checks evidence")
        if (not isinstance(evidence, list) or not evidence
                or any(not isinstance(e, dict) or e.get("status") != "pass"
                       or e.get("snapshot_digest") != last["snapshot_digest"]
                       or not isinstance(e.get("receipt_ref"), str)
                       or not e["receipt_ref"].strip() for e in evidence)):
            reasons.append("passing checks evidence not bound to reviewed snapshot")
        state = self.finding_state()
        unresolved = sorted(k for k, v in state.items()
                            if v not in ("closed_by_editor", "deferred_accepted"))
        if unresolved:
            reasons.append(f"findings not editor-closed: {unresolved}")
        debts = [r for r in rows if r["kind"] == "debt_accepted"]
        if any(d.get("authority_verified") is not True for d in debts):
            reasons.append("debt authority unverified; re-authorize retained debt")
        if reasons:
            return {"outcome": "REVISE", "reasons": reasons,
                    "unresolved": unresolved}
        return {"outcome": "COMPLETE", "reasons": [], "unresolved": [],
                "accepted_debts": sorted(d["finding_id"] for d in debts)}

    def _evaluate_none(self, opened, current_subject_digest):
        """Tier none: no editor call, but the exemption receipt must verify —
        it proves no substantive work (or an already-approved same subject).
        Missing verification stays REVISE; it is never a bypass."""
        snaps = self.snapshots()
        receipt = opened.get("exemption_receipt")
        try:
            verified = (snaps and not self.finding_state()
                        and self._exemption_verifier is not None
                        and self._exemption_verifier(
                            self.review_id, opened, snaps[-1], receipt,
                            current_subject_digest) is True)
        except Exception:
            verified = False
        if verified:
            return {"outcome": "COMPLETE", "reasons": [], "unresolved": []}
        return {"outcome": "REVISE", "reasons": ["no verified exemption evidence"],
                "unresolved": sorted(self.finding_state())}


# ------------------------------------------------------------------- CLI

def main():
    import argparse
    import sys
    ap = argparse.ArgumentParser(prog="session-review")
    ap.add_argument("verb", choices=["resolve", "findings", "tail"])
    ap.add_argument("--review", required=True)
    ap.add_argument("--base", default=str(DEFAULT_REVIEWS))
    ap.add_argument("--current-digest", default=None)
    ap.add_argument("--checks-evidence", default=None)
    a = ap.parse_args()
    try:
        r = SessionReview(a.review, a.base)
        rows = r.rows
    except ValueError as e:
        print(json.dumps({"faults": [str(e)]}))
        return 2
    if not rows:
        print(json.dumps({"faults": [f"unknown review {a.review}"]}))
        return 2
    try:
        if a.verb == "resolve":
            print(json.dumps(r.resolve(current_subject_digest=a.current_digest,
                                       checks_evidence=a.checks_evidence), indent=2))
            return 0
        if a.verb == "findings":
            print(json.dumps(r.finding_state(), indent=2))
            return 0
        for row in rows[-5:]:
            print(json.dumps(row, sort_keys=True)[:220])
        return 0
    except ValueError as e:
        print(json.dumps({"faults": [str(e)]}))
        return 2


if __name__ == "__main__":
    import sys
    sys.exit(main())
