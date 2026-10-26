"""swarm_ledger — the multi-directional config ledger (protocol v1, sol
turn-6). The editable YAML is a WORKSPACE; active.json selects operational
truth. Any host may propose; dev adjudicates a frozen batch and commits the
accepted chain; replicas converge by verified ancestry.

Protocol invariants encoded here:
  - capture compares workspace bytes against workspace_base (NEVER advanced
    by an incoming release — a dirty workspace is never auto-overwritten);
  - echo suppression via durable install-intent transactions, not mtimes;
  - raw edits are UNATTRIBUTED: owner-only diffs stay pending_authorization
    until an HMAC owner endorsement binds area|parent|bytes|change_id;
  - identical replay acknowledges; ID reuse with different bytes quarantines;
  - stale-parent proposals are preserved, never auto-rebased (owner included);
  - batch order: authenticated owner, then ingress_seq, then change_id;
  - the journal append is the commit point; head.json is a rebuildable cache.
"""
import fcntl
import hashlib
import hmac
import json
import os
import socket
import uuid
from pathlib import Path

from swarmconfig import (  # kernel primitives
    DEFAULT_STATE, DEFAULT_YAML, GENESIS_HASH, HEX64, _agent_unauthorized,
    accept, active_head, atomic_write, atomic_write_json, canonical_json,
    content_hash, parse_yaml_bytes, validate, _paths,
)

PROTOCOL_VERSION = 1


def _sha256(b):
    return hashlib.sha256(b).hexdigest()


def _now():
    import datetime
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _sync_dir(state, area):
    return Path(state).expanduser() / "sync" / area


def _ledger_dir(state, area):
    return Path(state).expanduser() / "ledger" / area


def _replica_path(state, area):
    return _sync_dir(state, area) / "replica.json"


class _AreaLock:
    """Advisory flock over sync/<area>/lock; sanctioned writers participate."""

    def __init__(self, state, area):
        d = _sync_dir(state, area)
        d.mkdir(parents=True, exist_ok=True)
        self.fh = open(d / "lock", "a+")

    def __enter__(self):
        fcntl.flock(self.fh, fcntl.LOCK_EX)
        return self

    def __exit__(self, *exc):
        fcntl.flock(self.fh, fcntl.LOCK_UN)
        self.fh.close()
        return False


# ------------------------------------------------------------- replica manifest

def load_replica(state=DEFAULT_STATE, area="config"):
    p = _replica_path(state, area)
    try:
        return json.loads(p.read_text())
    except FileNotFoundError:
        return None
    except Exception:
        return None


def _save_replica(state, area, rep):
    rep["updated_at"] = _now()
    atomic_write_json(_replica_path(state, area), rep)


def init_state(state=DEFAULT_STATE, workspace=DEFAULT_YAML, host=None):
    """Bootstrap: owner key, classification manifest, replica manifest with
    workspace_base at GENESIS (or at the currently-installed head)."""
    state = Path(state).expanduser()
    state.mkdir(parents=True, exist_ok=True)
    key_path = state / "control" / "owner.key"
    if not key_path.exists():
        atomic_write(key_path, os.urandom(32).hex().encode() + b"\n")
    cls = state / "classification.json"
    if not cls.exists():
        atomic_write_json(cls, classification_manifest())
    rep = load_replica(state) or {"host": host or socket.gethostname()}
    ws = Path(workspace).expanduser()
    # an unset base OR a pre-ledger GENESIS base re-resolves against sealed
    # releases: after the first accept, the base must track the accepted
    # release or every later edit is mislabeled stale
    if "workspace_base" not in rep or rep.get("workspace_base") == GENESIS_HASH:
        rep["workspace_base"] = GENESIS_HASH
        if ws.exists():
            rep["workspace_base_bytes_hash"] = _sha256(ws.read_bytes())
            # if the workspace bytes match a sealed release, the base IS that
            # release's canonical hash (a genesis-constant base would mislabel
            # every subsequent edit as stale, sol t9 regression L5)
            rel_root = Path(state).expanduser() / "config" / "releases"
            if rel_root.is_dir():
                for d in rel_root.iterdir():
                    ry = d / "release.yaml"
                    if ry.exists() and _sha256(ry.read_bytes()) == rep["workspace_base_bytes_hash"]:
                        rep["workspace_base"] = d.name
                        break
        else:
            rep["workspace_base_bytes_hash"] = None
    _save_replica(state, "config", rep)
    return rep


def owner_key(state=DEFAULT_STATE):
    """The owner authority token. Distributed to replicas at bootstrap
    (a local authority token, never an external credential); dev verifies.
    Bootstraps its state dir if touched before init."""
    kp = Path(state).expanduser() / "control" / "owner.key"
    if not kp.exists():
        init_state(state)
    return kp.read_bytes().strip()


# ---------------------------------------------------------- endorsements

def endorse(envelope: dict, state=DEFAULT_STATE) -> dict:
    """Owner endorsement: HMAC binds area|parent|bytes|change_id. Returns the
    authority_ref to embed in the envelope. A CLI --owner flag alone is never
    proof — the adjudicator verifies this signature."""
    msg = "|".join([envelope["area"], (envelope.get("parent") or {}).get("canonical_hash") or "",
                    envelope["candidate_bytes_hash"], envelope["change_id"]]).encode()
    return {"principal": "owner", "scheme": "hmac-sha256",
            "sig": hmac.new(owner_key(state), msg, hashlib.sha256).hexdigest()}


def verify_endorsement(envelope: dict, state=DEFAULT_STATE) -> bool:
    ar = envelope.get("authority_ref") or {}
    if ar.get("scheme") != "hmac-sha256" or ar.get("principal") != "owner":
        return False
    msg = "|".join([envelope["area"], (envelope.get("parent") or {}).get("canonical_hash") or "",
                    envelope["candidate_bytes_hash"], envelope["change_id"]]).encode()
    return hmac.compare_digest(
        ar["sig"], hmac.new(owner_key(state), msg, hashlib.sha256).hexdigest())


# ------------------------------------------------------------- capture

def capture(state=DEFAULT_STATE, area="config", workspace=DEFAULT_YAML,
            reason="", max_retries=3):
    """Detect a workspace edit against workspace_base and preserve it as an
    immutable outbox proposal. Bytes-only: works without PyYAML. Returns
    {"captured": bool, "change_id": ..., "deduped": bool, "faults": [...]}."""
    ws = Path(workspace).expanduser()
    faults = []
    with _AreaLock(state, area):
        recover_install(state, area)
        rep = load_replica(state, area)
        if rep is None:
            rep = init_state(state, workspace)
        data = None
        for _ in range(max_retries):
            try:
                b1 = ws.read_bytes()
            except FileNotFoundError:
                data = None
                break
            except IsADirectoryError:
                faults.append({"code": "capture_unstable", "detail": "workspace is a directory"})
                return {"captured": False, "faults": faults}
            if ws.is_symlink():
                faults.append({"code": "capture_unstable", "detail": "workspace is a symlink"})
                return {"captured": False, "faults": faults}
            b2 = ws.read_bytes()
            if _sha256(b1) == _sha256(b2):
                data = b1
                break
        else:
            faults.append({"code": "capture_unstable",
                           "detail": "workspace changed during read; never settled"})
            return {"captured": False, "faults": faults}

        if data is None:
            faults.append({"code": "deletion_observed",
                           "detail": "workspace absent; deletion proposals never auto-activate"})
            return {"captured": False, "faults": faults}

        bh = _sha256(data)
        if bh == rep.get("workspace_base_bytes_hash"):
            return {"captured": False, "faults": faults}
        lc = rep.get("last_captured") or {}
        if lc.get("candidate_bytes_hash") == bh:
            return {"captured": False, "deduped": True,
                    "change_id": lc.get("change_id"), "faults": faults}

        head = active_head(state) or {}
        # the proposal parent is the WORKSPACE BASE the edit was made against
        # (replica manifest hash), NEVER the latest head — an old-base edit
        # must surface as stale and be re-ratified, not masquerade as fresh
        # (sol t7#4 / t9#2)
        base_hash = rep.get("workspace_base") or GENESIS_HASH
        base_meta = None
        for r in journal_records(state, area):
            if r.get("canonical_hash") == base_hash:
                base_meta = r
                break
        cid = str(uuid.uuid4())
        ob = _sync_dir(state, area) / "outbox" / cid
        ob.mkdir(parents=True, exist_ok=True)
        atomic_write(ob / "candidate.yaml", data)
        envelope = {
            "protocol_version": PROTOCOL_VERSION,
            "area": area,
            "change_id": cid,
            "origin_host": rep.get("host") or socket.gethostname(),
            "parent": {"authority_epoch": (base_meta or {}).get("authority_epoch")
                       or (head or {}).get("epoch") or 1,
                       "revision": (base_meta or {}).get("revision") or 0,
                       "canonical_hash": base_hash},
            "workspace_base_bytes_hash": rep.get("workspace_base_bytes_hash"),
            "candidate_bytes_hash": bh,
            "claimed_principal": None,      # raw edits are unattributed
            "authority_ref": None,
            "reason": reason,
            "derived_from": None,
        }
        atomic_write_json(ob / "envelope.json", envelope)
        rep["last_captured"] = {"parent_hash": envelope["parent"]["canonical_hash"],
                                "candidate_bytes_hash": bh, "change_id": cid}
        _save_replica(state, area, rep)
        return {"captured": True, "change_id": cid, "faults": faults}


def load_proposal(state, area, cid, origin=None):
    """Read a proposal bundle (outbox, or dev inbox with origin host)."""
    base = _sync_dir(state, area)
    for d in ([base / "inbox" / origin / cid] if origin else []) + [base / "outbox" / cid]:
        try:
            env = json.loads((d / "envelope.json").read_text())
            cand = (d / "candidate.yaml").read_bytes()
            return {"dir": d, "envelope": env, "candidate": cand}
        except (FileNotFoundError, NotADirectoryError):
            continue
        except Exception:
            return None
    return None


def endorse_outbox(state=DEFAULT_STATE, area="config", change_id=None):
    """Sanctioned owner verb: embed the HMAC endorsement into an outbox
    envelope so the next adjudication treats it as authenticated owner."""
    base = _sync_dir(state, area) / "outbox" / str(change_id)
    env = json.loads((base / "envelope.json").read_text())
    env["authority_ref"] = endorse(env, state)
    atomic_write_json(base / "envelope.json", env)
    return env["authority_ref"]


# ------------------------------------------------------------- adjudication

def _ingress_append(state, area, rec):
    """First-receipt record: the durable ingress_seq AND the frozen identity
    (candidate/envelope hashes) live HERE — replay adjudication compares
    against this record, never against the mutable bundle (sol t9#3)."""
    j = _sync_dir(state, area) / "ingress.jsonl"
    j.parent.mkdir(parents=True, exist_ok=True)
    rec["ingress_seq"] = len(_ingress_all(state, area)) + 1  # dev-sequenced
    with open(j, "a") as f:
        f.write(json.dumps(rec, sort_keys=True) + "\n")
        f.flush()
        os.fsync(f.fileno())


def _ingress_all(state, area):
    j = _sync_dir(state, area) / "ingress.jsonl"
    try:
        return [json.loads(x) for x in j.read_text().splitlines() if x.strip()]
    except FileNotFoundError:
        return []


def _decisions_append(state, area, rec):
    j = _sync_dir(state, area) / "decisions.jsonl"
    j.parent.mkdir(parents=True, exist_ok=True)
    with open(j, "a") as f:
        f.write(json.dumps(rec, sort_keys=True) + "\n")
        f.flush()
        os.fsync(f.fileno())


def _decisions_all(state, area):
    j = _sync_dir(state, area) / "decisions.jsonl"
    try:
        return [json.loads(x) for x in j.read_text().splitlines() if x.strip()]
    except FileNotFoundError:
        return []


def _verdict_write(state, area, cid, verdict):
    d = _sync_dir(state, area) / "verdicts"
    d.mkdir(parents=True, exist_ok=True)
    atomic_write_json(d / f"{cid}.json", verdict)


def adjudicate(state=DEFAULT_STATE, area="config"):
    """Freeze a batch of known proposals, order them (owner, ingress_seq,
    change_id), and commit each against the current head. Returns the
    summary list of verdicts."""
    with _AreaLock(state, area):
        recover_install(state, area)
        # ingest any dev-side inbox bundles not yet in ingress
        base = _sync_dir(state, area)
        known = {r["change_id"] for r in _ingress_all(state, area)}
        inbox = base / "inbox"
        if inbox.is_dir():
            for hostdir in sorted(inbox.iterdir()):
                if not hostdir.is_dir():
                    continue
                for cid in sorted(hostdir.iterdir()):
                    if cid.name in known:
                        continue
                    if (cid / "envelope.json").exists():
                        env_i = json.loads((cid / "envelope.json").read_text())
                        cand_i = (cid / "candidate.yaml").read_bytes()
                        _ingress_append(state, area, {
                            "change_id": cid.name, "origin_host": hostdir.name,
                            "candidate_bytes_hash": env_i.get("candidate_bytes_hash"),
                            "envelope_hash": _sha256(json.dumps(env_i, sort_keys=True).encode()),
                            "received_at": _now()})
        # local outbox proposals too (dev proposes to itself the same way)
        for cid_dir in sorted((base / "outbox").glob("*/")):
            if cid_dir.name in known or not (cid_dir / "envelope.json").exists():
                continue
            env_i = json.loads((cid_dir / "envelope.json").read_text())
            cand_i = (cid_dir / "candidate.yaml").read_bytes()
            _ingress_append(state, area, {"change_id": cid_dir.name,
                                          "origin_host": "dev-local",
                                          "candidate_bytes_hash": env_i.get("candidate_bytes_hash"),
                                          "envelope_hash": _sha256(json.dumps(env_i, sort_keys=True).encode()),
                                          "received_at": _now()})

        ingress = _ingress_all(state, area)
        proposals = []
        for rec in ingress:
            pr = load_proposal(state, area, rec["change_id"], rec.get("origin_host"))
            if pr is None:
                pr = load_proposal(state, area, rec["change_id"])
            if pr is None:
                continue
            proposals.append({**rec, "bundle": pr})

        def sort_key(p):
            env = p["bundle"]["envelope"]
            owner_first = 0 if verify_endorsement(env, state) else 1
            return (owner_first, p.get("ingress_seq") or 0, env["change_id"])

        proposals.sort(key=sort_key)
        verdicts = []
        for p in proposals:
            v = _adjudicate_one(state, area, p)
            verdicts.append(v)
            _verdict_write(state, area, p["bundle"]["envelope"]["change_id"], v)
            _decisions_append(state, area, {
                "change_id": p["bundle"]["envelope"]["change_id"],
                "decision": v["decision"], "at": _now(),
                "detail": v.get("detail", "")[:200]})
        return verdicts


def _adjudicate_one(state, area, p):
    env = p["bundle"]["envelope"]
    cid = env["change_id"]
    cand = p["bundle"]["candidate"]
    bh = env["candidate_bytes_hash"]
    base_detail = {"change_id": cid, "origin": p.get("origin_host")}

    if _sha256(cand) != bh:
        return {**base_detail, "decision": "rejected_invalid",
                "detail": "candidate bytes do not match candidate_bytes_hash"}

    # replay identity — only TERMINAL decisions ack; pending/held proposals
    # stay re-adjudicable (e.g. endorsed after a pending_authorization batch)
    non_terminal = ("pending_authorization", "held_parser_fault")
    prior = None
    for d in reversed(_decisions_all(state, area)):
        if d["change_id"] == cid:
            prior = d
            break
    if prior is not None:
        # identity comes from the PERSISTED first-receipt record (immutable),
        # never from the mutable bundle on disk (sol t9#3)
        ing = next((r for r in reversed(_ingress_all(state, area))
                    if r.get("change_id") == cid), None)
        same = ing is not None and ing.get("candidate_bytes_hash") == bh
        if not same:
            return {**base_detail, "decision": "ID_REUSE",
                    "detail": "same change_id with different bytes than first receipt; quarantined"}
        if prior["decision"] not in non_terminal:
            return {**base_detail, "decision": prior["decision"],
                    "detail": "identical replay acknowledged (idempotent)"}
        # non-terminal prior with identical bytes: fall through to re-adjudicate

    cfg, pf = parse_yaml_bytes(cand)
    if pf:
        return {**base_detail, "decision": "held_parser_fault",
                "detail": json.dumps(pf)[:200], "preserved": True}
    vf = validate(cfg)
    if vf:
        return {**base_detail, "decision": "rejected_invalid", "faults": vf}

    head = active_head(state)
    head_hash = head["content_hash"] if head else GENESIS_HASH
    head_epoch = (head or {}).get("epoch") or 1
    if (cfg.get("identity") or {}).get("authority_epoch") != head_epoch:
        return {**base_detail, "decision": "rejected_epoch_transfer_required",
                "detail": "epoch changes use the authenticated transfer procedure, never YAML"}

    if env["parent"].get("canonical_hash") != head_hash:
        return {**base_detail, "decision": "STALE_PARENT",
                "detail": f"proposal parent {env['parent'].get('canonical_hash', '')[:12]} "
                          f"!= head {head_hash[:12]}; preserved, never auto-rebased",
                "current_head": head_hash, "preserved": True}

    prev = None
    if head is not None:
        prev = _read_release_cfg(state, head_hash)
    endorsed = verify_endorsement(env, state)
    if endorsed:
        authority_class, principal = "owner", "owner"
    elif prev is None:
        return {**base_detail, "decision": "pending_authorization",
                "detail": "no accepted baseline (bootstrap): only an endorsed owner proposal may commit",
                "preserved": True}
    else:
        bad = _agent_unauthorized(prev, cfg)
        if bad:
            return {**base_detail, "decision": "pending_authorization",
                    "detail": f"raw edit; owner endorsement required for {bad[:5]}",
                    "preserved": True}
        authority_class, principal = "agent", f"agent:{env.get('claimed_principal') or p.get('origin_host')}"

    # dev constructs the final release with machine-owned identity fields
    final = dict(cfg)
    final["identity"] = {
        "config_id": (cfg.get("identity") or {}).get("config_id", "fleet-swarm"),
        "authority_epoch": head_epoch,
        "revision": ((head or {}).get("revision") or 0) + 1,
        "parent_hash": head_hash,
        "change_id": cid,
        "proposed_by": principal,
        "reason": env.get("reason") or f"sync batch accept from {p.get('origin_host')}",
    }
    # F35.4r: the ledger supplies the authenticated context accept()
    # requires. The sync transport vouches for the captured principal
    # (claimed principal, else the origin host); the role is DERIVED from
    # the vouch — "owner" for endorsed captures, "agent" otherwise (demand
    # delegation in a config therefore lists "agent" among its
    # agent_role_refs to enable ledger-carried demand edits).
    verified_agent = ("owner" if endorsed
                      else env.get("claimed_principal") or p.get("origin_host"))
    _vouched_role = "owner" if endorsed else "agent"

    def _ledger_verifier(who, _v=verified_agent, _r=_vouched_role):
        return _r if who == _v else None

    final_bytes = canonical_json(final)  # canonical JSON form is valid YAML
    # COMMIT ORDER (sol t7#3/t9#1): seal -> journal (fsync = commit point) ->
    # activate. A journal failure leaves an uncommitted seal, never a moved
    # pointer; recovery reads committed releases exclusively.
    r = accept(final_bytes, proposed_by=principal, authority_class=authority_class,
               parent_expected=head_hash, state_dir=state, change_id=cid,
               reason=final["identity"]["reason"], commit_point="seal-only",
               principal_verifier=_ledger_verifier,
               verified_principal=verified_agent, issuer_role_ref="agent")
    if r["decision"] != "accepted":
        return {**base_detail, "decision": r["decision"],
                "detail": "; ".join(x["detail"] for x in r["faults"])[:200]}

    rec = _journal_record(state, area, r, env, principal,
                          release_bytes_hash=_release_bytes_sha(state, r.get("content_hash")))
    _journal_append(state, area, rec)  # COMMIT POINT (fsync)
    r = accept(final_bytes, proposed_by=principal, authority_class=authority_class,
               parent_expected=head_hash, state_dir=state, change_id=cid,
               reason=final["identity"]["reason"], commit_point="activate-only",
               principal_verifier=_ledger_verifier,
               verified_principal=verified_agent, issuer_role_ref="agent")
    _refresh_current_yaml(state)
    return {**base_detail, "decision": "accepted",
            "revision": r.get("revision"), "content_hash": r.get("content_hash")}


def _read_release_cfg(state, h):
    try:
        obj = json.loads((_paths(Path(state).expanduser())["releases"] / h / "release.json").read_text())
        if isinstance(obj, dict) and content_hash(obj) == h:
            return obj
    except Exception:
        pass
    return None


# ------------------------------------------------------------- journal

def _event_hash(rec, prev):
    body = {k: v for k, v in rec.items() if k != "event_hash"}
    return hashlib.sha256(canonical_json(body) + prev.encode()).hexdigest()


def _release_bytes_sha(state, h):
    if not h:
        return None
    try:
        return _sha256((_paths(Path(state).expanduser())["releases"] / h / "release.yaml").read_bytes())
    except Exception:
        return None


def _journal_record(state, area, accept_receipt, env, principal,
                    release_bytes_hash=None):
    head = accept_receipt.get("revision") or {}
    prev = journal_head(state, area) or {}
    rec = {
        "protocol_version": PROTOCOL_VERSION,
        "area": area,
        "authority_epoch": head.get("epoch"),
        "revision": head.get("revision"),
        "change_id": accept_receipt["change_id"],
        "parent_hash": (env.get("parent") or {}).get("canonical_hash"),
        "candidate_bytes_hash": env["candidate_bytes_hash"],
        "release_bytes_hash": release_bytes_hash,
        "canonical_hash": head.get("content_hash"),
        "principal": principal,
        "authority_ref": (env.get("authority_ref") or {}).get("scheme"),
        "validator_ref": "swarmharness/swarmconfig v2",
        "previous_event_hash": prev.get("event_hash") or GENESIS_HASH,
    }
    rec["event_hash"] = _event_hash(rec, rec["previous_event_hash"])
    return rec


def _journal_append(state, area, rec):
    j = _ledger_dir(state, area) / "journal.jsonl"
    j.parent.mkdir(parents=True, exist_ok=True)
    with open(j, "a") as f:
        f.write(json.dumps(rec, sort_keys=True) + "\n")
        f.flush()
        os.fsync(f.fileno())
    atomic_write_json(_ledger_dir(state, area) / "head.json",
                      {"revision": rec["revision"], "canonical_hash": rec["canonical_hash"],
                       "event_hash": rec["event_hash"]})


def journal_head(state=DEFAULT_STATE, area="config"):
    try:
        return json.loads((_ledger_dir(state, area) / "head.json").read_text())
    except Exception:
        return None


def journal_records(state=DEFAULT_STATE, area="config"):
    j = _ledger_dir(state, area) / "journal.jsonl"
    try:
        rows = []
        for x in j.read_text().splitlines():
            if not x.strip():
                continue
            try:
                rows.append(json.loads(x))
            except Exception:
                continue  # a corrupt line is skipped for reads; quarantine
                          # and recovery live in verify_ancestry/load()
        return rows
    except FileNotFoundError:
        return []
    except OSError:
        return []


def verify_ancestry(state=DEFAULT_STATE, area="config", records=None):
    """Chain verification: recompute every event_hash. -> (ok, bad_index).
    records=None verifies the local journal from genesis; a list is verified
    as either a full chain (from genesis) or a continuation of the LOCAL
    head (sol t9#5)."""
    recs = journal_records(state, area) if records is None else records
    local = journal_records(state, area)
    prev = GENESIS_HASH
    if records is not None and local:
        if records and records[0].get("previous_event_hash") == local[-1].get("event_hash"):
            prev = local[-1].get("event_hash")  # continuation suffix
    for i, rec in enumerate(recs):
        if not isinstance(rec, dict):
            return False, i
        try:
            if rec.get("previous_event_hash") != prev:
                return False, i
            if rec.get("event_hash") != _event_hash(rec, prev):
                return False, i
        except (KeyError, TypeError):
            return False, i  # malformed record: structural fault, never a crash
        prev = rec["event_hash"]
    return True, None



def _refresh_current_yaml(state):
    """Regenerate the accepted-view projection (never touches the workspace)."""
    head = active_head(state)
    if not head:
        return
    rel = _paths(Path(state).expanduser())["releases"] / head["content_hash"]
    try:
        raw = (rel / "release.yaml").read_bytes()
    except FileNotFoundError:
        raw = None
    if raw is None:
        try:
            obj = json.loads((rel / "release.json").read_text())
            raw = canonical_json(obj)
        except Exception:
            return
    atomic_write(Path(state).expanduser() / "config" / "current.yaml", raw)


# ------------------------------------------------------- install transaction

def install_accepted(state, release_hash, origin_bytes=None, area="config",
                     workspace=DEFAULT_YAML, shipped_journal=None):
    """Replica-side installation: durable intent -> atomic writes -> manifest
    completion. NEVER touches the workspace; workspace_base does NOT advance
    (a dirty workspace stays dirty; current.yaml exposes accepted truth).

    INSTALL GATE (sol turn-7 #5): schema validation must pass (a
    schema_version:999 body is rejected, not self-hash-trusted), and the
    release must be vouched by ancestry — either shipped journal records
    whose event chain verifies and whose head names this hash, or the local
    journal already containing the hash. Bundle identity alone proves
    nothing."""
    from swarmconfig import validate as _validate
    st = Path(state).expanduser()
    rel_dir = _paths(st)["releases"] / release_hash
    rel_yaml = rel_dir / "release.yaml"
    rel_json = rel_dir / "release.json"
    if not rel_json.exists():
        return {"installed": False, "detail": "release bundle absent locally"}
    body = json.loads(rel_json.read_text())
    if content_hash(body) != release_hash:
        return {"installed": False, "detail": "release bundle hash mismatch"}
    vf = _validate(body)
    if vf:
        return {"installed": False, "detail": f"release fails schema validation: {vf[0]['code']}"}
    if origin_bytes is not None and _sha256(origin_bytes) != _sha256(rel_yaml.read_bytes()):
        return {"installed": False, "detail": "origin bytes differ from sealed release"}
    # ancestry: shipped records with a verifying chain naming this hash...
    vouched = False
    local = journal_records(state, area)
    local_head = local[-1] if local else None
    if shipped_journal:
        ok, bad = verify_ancestry(state=state, area=area, records=shipped_journal)
        if not ok:
            return {"installed": False, "detail": f"shipped journal chain broken at record {bad}"}
        if shipped_journal[-1].get("canonical_hash") != release_hash:
            return {"installed": False, "detail": "shipped journal head does not name this release"}
        # trim to the suffix the replica lacks, then require continuity
        known = {r.get("event_hash") for r in local}
        suffix = [r for r in shipped_journal if r.get("event_hash") not in known]
        if local and suffix:
            if suffix[0].get("previous_event_hash") != local[-1]["event_hash"]:
                return {"installed": False, "detail": "shipped journal does not continue local ancestry"}
        shipped_journal = suffix
        vouched = True
    else:
        # no shipped records: the LOCAL journal must vouch the hash AND its
        # own chain must verify — a bare canonical_hash line proves nothing
        # (sol t10#5)
        ok_local, _bad = verify_ancestry(state=state, area=area)
        if ok_local and any(r.get("canonical_hash") == release_hash for r in local):
            vouched = True
        else:
            return {"installed": False,
                    "detail": "unvouched: local journal missing hash or fails chain verification"}
    if not vouched:
        return {"installed": False, "detail": "unvouched install refused"}

    ident = body.get("identity") or {}
    target = {"canonical_hash": release_hash, "epoch": ident.get("authority_epoch"),
              "revision": ident.get("revision"), "change_id": ident.get("change_id")}
    old_rep = load_replica(state, area) or {}
    intent = {"target": target, "old_manifest": old_rep, "at": _now()}
    ipath = _sync_dir(state, area) / "install-intent.json"
    atomic_write_json(ipath, intent)

    # REPLICA COMMIT ORDER (sol t10 reg-1): persist the shipped acceptance
    # records into the LOCAL journal FIRST (fsync = the local commit point),
    # and only then the pointer + checkpoint. A journal failure must leave
    # containment, never an active pointer over an uncommitted release.
    if shipped_journal:
        j = _ledger_dir(state, area) / "journal.jsonl"
        j.parent.mkdir(parents=True, exist_ok=True)
        existing = {r.get("event_hash") for r in journal_records(state, area)}
        with open(j, "a") as f:
            for r in shipped_journal:
                if r.get("event_hash") not in existing:
                    f.write(json.dumps(r, sort_keys=True) + "\n")
            f.flush()
            os.fsync(f.fileno())
        headj = journal_records(state, area)
        if headj:
            atomic_write_json(_ledger_dir(state, area) / "head.json",
                              {"revision": headj[-1].get("revision"),
                               "canonical_hash": headj[-1].get("canonical_hash"),
                               "event_hash": headj[-1].get("event_hash")})
    else:
        ok_local, _ = verify_ancestry(state=state, area=area)
        if local and not ok_local:
            return {"installed": False, "detail": "local journal fails chain verification"}

    atomic_write_json(_paths(st)["active"], {"content_hash": release_hash,
                                             "epoch": target["epoch"],
                                             "revision": target["revision"],
                                             "change_id": target["change_id"],
                                             "activated_at": _now()})
    atomic_write_json(_paths(st)["checkpoint"], body)
    _refresh_current_yaml(state)
    rep = load_replica(state, area) or dict(old_rep)
    rep["installed_head"] = target
    rep["active_head"] = target
    # a CLEAN workspace (bytes == installed release) advances its base with
    # the install; a dirty workspace keeps its base (never auto-overwritten)
    try:
        wsp = Path(str(workspace).replace("~", str(Path.home())))
        if wsp.exists() and _sha256(wsp.read_bytes()) == _sha256(rel_yaml.read_bytes()):
            rep["workspace_base"] = release_hash
            rep["workspace_base_bytes_hash"] = _sha256(wsp.read_bytes())
    except Exception:
        pass
    _save_replica(state, area, rep)
    ipath.unlink(missing_ok=True)
    return {"installed": True, "target": target}


def recover_install(state, area="config"):
    """First thing every local pass does: finish or roll back an interrupted
    install so its partial writes can never surface as a proposal."""
    ipath = _sync_dir(state, area) / "install-intent.json"
    try:
        intent = json.loads(ipath.read_text())
    except FileNotFoundError:
        return {"recovered": False}
    except Exception:
        ipath.unlink(missing_ok=True)
        return {"recovered": True, "detail": "corrupt intent discarded"}
    target = intent.get("target") or {}
    h = target.get("canonical_hash")
    if h and HEX64.fullmatch(h):
        rel_dir = _paths(Path(state).expanduser())["releases"] / h
        if (rel_dir / "release.json").exists():
            body = json.loads((rel_dir / "release.json").read_text())
            from swarmconfig import validate as _validate
            if content_hash(body) == h and not _validate(body):
                st = Path(state).expanduser()
                atomic_write_json(_paths(st)["active"], {
                    "content_hash": h, "epoch": target.get("epoch"),
                    "revision": target.get("revision"),
                    "change_id": target.get("change_id"), "activated_at": _now()})
                atomic_write_json(_paths(st)["checkpoint"], body)
                rep = load_replica(state, area) or {}
                rep["installed_head"] = target
                rep["active_head"] = target
                _save_replica(state, area, rep)
                _refresh_current_yaml(state)
                ipath.unlink(missing_ok=True)
                return {"recovered": True, "detail": "install completed from intent"}
    ipath.unlink(missing_ok=True)
    rep = load_replica(state, area)
    if rep is not None and intent.get("old_manifest"):
        _save_replica(state, area, intent["old_manifest"])
    return {"recovered": True, "detail": "intent rolled back to old manifest"}


# ------------------------------------------------------- tier classification

def classification_manifest():
    """Initial owner-controlled tier manifest (sol turn-6 §8). Per-path,
    per-record-purpose rules; unclassified paths fail enrollment checks."""
    return {
        "version": 1,
        "rules": [
            {"path": "swarm-config.yaml", "tier": "T1", "area": "config",
             "writer_api": "proposal-workspace", "replication_driver": "swarm-ledger-v1"},
            {"path": "swarm/config/releases/", "tier": "T3", "area": "config",
             "writer_api": "protocol-managed", "replication_driver": "swarm-ledger-v1"},
            {"path": "swarm/config/acks/", "tier": "T2", "area": "config",
             "writer_api": "ack-verb", "replication_driver": "report-only",
             "purpose": "per-consumer activation acknowledgements; reported up, never merged"},
            {"path": "swarm/config/", "tier": "T3", "area": "config",
             "writer_api": "protocol-managed", "replication_driver": "swarm-ledger-v1"},
            {"path": "swarm/registry/", "tier": "T1", "area": "registry",
             "writer_api": "record-api",
             "replication_driver": "planned-swarm-ledger-v1",
             "purpose": "registry records replicate through their record APIs once the registry area ships (honest label: not yet replicating)"},
            {"path": "dream/policy/overlays.json", "tier": "T1", "area": "policy",
             "writer_api": "learned-patch-contract",
             "replication_driver": "planned-swarm-ledger-v1",
             "purpose": "overlay definitions T1; receipts below are T2"},
            {"path": "dream/policy/receipts.jsonl", "tier": "T2", "area": "policy",
             "writer_api": "append-only-producer",
             "replication_driver": "report-only",
             "purpose": "activation/evaluation receipts: immutable producer records reported upward, never merged"},
            {"path": "dream/policy/claims.jsonl", "tier": "T1", "area": "policy",
             "writer_api": "falsify-claim-api", "replication_driver": "planned-swarm-ledger-v1"},
            {"path": "dream/policy/evaluations.jsonl", "tier": "T2", "area": "policy",
             "writer_api": "append-only-producer", "replication_driver": "report-only"},
            {"path": "dream/audit-feedback.json", "tier": "T2", "area": "audit",
             "writer_api": "audit-wire", "replication_driver": "report-only"},
            {"path": "dream/", "tier": "T3", "area": "runtime",
             "writer_api": "local", "replication_driver": "none"},
            {"path": "config/", "tier": "T3", "area": "runtime",
             "writer_api": "local", "replication_driver": "none"},
            {"path": "memory/", "tier": "NATIVE", "area": "memory",
             "writer_api": "ygg-memory", "replication_driver": "ygg-memory"},
            {"path": "bin/", "tier": "T3", "area": "installed",
             "writer_api": "ygg-disperse", "replication_driver": "ygg-disperse"},
            {"path": "skills/", "tier": "T3", "area": "installed",
             "writer_api": "ygg-disperse", "replication_driver": "ygg-disperse"},
            {"path": "relay/ci/", "tier": "T3", "area": "ci",
             "writer_api": "local", "replication_driver": "none"},
            {"path": "swarm/sync/", "tier": "T3", "area": "protocol",
             "writer_api": "protocol-managed", "replication_driver": "swarm-ledger-v1"},
            {"path": "swarm/ledger/", "tier": "T3", "area": "protocol",
             "writer_api": "protocol-managed", "replication_driver": "swarm-ledger-v1"},
            {"path": "swarm/control/", "tier": "T3", "area": "control",
             "writer_api": "local", "replication_driver": "none"},
        ],
        "excluded": [{"path": "auth*", "tier": "EXCLUDED",
                      "note": "credentials never ledgered"}],
    }


def load_classification(state=DEFAULT_STATE):
    try:
        m = json.loads((Path(state).expanduser() / "classification.json").read_text())
    except FileNotFoundError:
        m = classification_manifest()
    except Exception:
        return None
    # ambiguity check: two rules whose prefixes are equal -> fail
    prefixes = [r["path"] for r in m.get("rules", [])]
    if len(prefixes) != len(set(prefixes)):
        return None
    return m


def classify(path, manifest):
    """Longest-prefix match -> rule dict, or None if unclassified."""
    p = str(path)
    best = None
    for r in manifest.get("rules", []):
        if p == r["path"] or p.startswith(r["path"]):
            if best is None or len(r["path"]) > len(best["path"]):
                best = r
    return best


def enrollment_report(paths, state=DEFAULT_STATE):
    m = load_classification(state)
    if m is None:
        return {"ok": False, "detail": "classification manifest ambiguous/corrupt"}
    rows, unclassified = [], []
    for p in paths:
        r = classify(p, m)
        if r is None:
            unclassified.append(p)
        rows.append({"path": p, "tier": (r or {}).get("tier"), "area": (r or {}).get("area")})
    return {"ok": not unclassified, "rows": rows, "unclassified": unclassified}
