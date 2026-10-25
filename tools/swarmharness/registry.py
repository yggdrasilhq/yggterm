"""registry — P1 v2: the campaign registry and procedure library (sol
turn-5 §3). Authoritative records with typed schemas, revision-checked
writes, and append-only change receipts. T1 (ledgered) through these APIs;
the empire index is a derived T3 projection.

Storage: ~/.yggterm/swarm/registry/<kind>/<id>.json  (kind: campaign|trip|
procedure|decision|ask), receipts in registry/receipts.jsonl.

Queue completion alone can never mark a commitment complete; every write
carries actor + revision and refuses stale revisions.
"""
import json
import os
import uuid
from datetime import datetime, timezone
from pathlib import Path

from swarmconfig import atomic_write_json, canonical_json

DEFAULT_REGISTRY = Path("~/.yggterm/swarm/registry")

KINDS = ("campaign", "trip", "procedure", "decision", "ask")

CAMPAIGN_STATUS = ("proposed", "active", "waiting", "paused", "complete", "retired")
TRIP_STATUS = ("observed", "reproduced", "addressed", "disproved")
PROC_STATUS = ("draft", "verified", "superseded", "retired")
PROC_AUTHORITY = ("read_only", "reversible_change", "separately_authorized")
BLOCKER_CLASSES = ("technical", "capacity", "dependency", "authority", "input")

NOW = lambda: datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")  # noqa: E731


def _fault(faults, detail):
    faults.append(detail)


def validate_record(rec):
    """-> list of faults ([] = valid). Typed, required-field validation per
    the sol turn-5 record shapes."""
    f: list = []
    kind = rec.get("kind") or rec.get("_kind")
    if kind not in KINDS:
        return [f"kind must be one of {KINDS}"]
    rid = rec.get("id")
    if not isinstance(rid, str) or not rid:
        _fault(f, "id: non-empty string required")
    if not isinstance(rec.get("revision"), int) or rec["revision"] < 1:
        _fault(f, "revision: int >= 1 required")

    if kind == "campaign":
        if rec.get("status") not in CAMPAIGN_STATUS:
            _fault(f, f"status: {rec.get('status')!r} not in {CAMPAIGN_STATUS}")
        for k in ("objective",):
            if not isinstance(rec.get(k), str) or not rec[k]:
                _fault(f, f"{k}: non-empty string required")
        for c in rec.get("commitments") or []:
            if c.get("status") not in ("open", "in_progress", "waiting", "complete", "cancelled"):
                _fault(f, f"commitment {c.get('id')}: bad status {c.get('status')!r}")
        for b in rec.get("blockers") or []:
            if b.get("class") not in BLOCKER_CLASSES:
                _fault(f, f"blocker {b.get('id')}: class {b.get('class')!r} invalid")
    elif kind == "trip":
        if rec.get("status") not in TRIP_STATUS:
            _fault(f, f"status: {rec.get('status')!r} not in {TRIP_STATUS}")
        for k in ("cue", "observed_failure"):
            if not isinstance(rec.get(k), str) or not rec[k]:
                _fault(f, f"{k}: non-empty string required")
        if not isinstance(rec.get("evidence_refs"), list):
            _fault(f, "evidence_refs: list required")
    elif kind == "procedure":
        if rec.get("status") not in PROC_STATUS:
            _fault(f, f"status: {rec.get('status')!r} not in {PROC_STATUS}")
        if rec.get("authority_class") not in PROC_AUTHORITY:
            _fault(f, f"authority_class: {rec.get('authority_class')!r} invalid")
        for k in ("title", "cue"):
            if not isinstance(rec.get(k), str) or not rec[k]:
                _fault(f, f"{k}: non-empty string required")
        if not isinstance(rec.get("steps"), list) or not rec["steps"]:
            _fault(f, "steps: non-empty list required")
        if rec.get("status") == "verified" and not rec.get("verified_against"):
            _fault(f, "verified procedure requires verified_against fingerprints")
    elif kind == "decision":
        for k in ("question", "options", "authority_needed"):
            if k not in rec:
                _fault(f, f"{k}: required")
    elif kind == "ask":
        for k in ("context", "requested_input", "expected_answer_shape"):
            if k not in rec:
                _fault(f, f"{k}: required")
    return f


# ------------------------------------------------------------------ storage

def _rec_path(base, kind, rid):
    return Path(base).expanduser() / kind / f"{rid}.json"


def get(rid, kind, base=DEFAULT_REGISTRY):
    try:
        return json.loads(_rec_path(base, kind, rid).read_text())
    except FileNotFoundError:
        return None
    except Exception:
        return "unreadable"


def list_records(kind=None, base=DEFAULT_REGISTRY):
    base = Path(base).expanduser()
    out = []
    for k in ([kind] if kind else KINDS):
        d = base / k
        if not d.is_dir():
            continue
        for p in sorted(d.glob("*.json")):
            try:
                out.append(json.loads(p.read_text()))
            except Exception:
                out.append({"id": p.stem, "kind": k, "_error": "unreadable"})
    return out


_REG_LOCKS = {}


def _registry_lock(base):
    """Serialize read-compare-write PER BASE (sol t9#9: one global fd bound
    to the first base let two processes fence different files)."""
    import fcntl
    key = str(Path(base).expanduser().resolve())
    lp = Path(base).expanduser() / ".lock"
    lp.parent.mkdir(parents=True, exist_ok=True)
    lk = _REG_LOCKS.get(key)
    if lk is None:
        lk = open(lp, "a+")
        _REG_LOCKS[key] = lk
    fcntl.flock(lk, fcntl.LOCK_EX)
    return lk


def _registry_unlock():
    import fcntl
    for lk in _REG_LOCKS.values():
        try:
            fcntl.flock(lk, fcntl.LOCK_UN)
        except Exception:
            pass


def put(rec, actor, base=DEFAULT_REGISTRY, expected_revision=None):
    """Revision-checked write with an append-only receipt. A stale write
    (expected_revision given and mismatched) is refused, never merged.
    Completion transitions require outcome evidence (sol t7 #7)."""
    kind = rec.get("kind") or rec.get("_kind")
    if kind not in KINDS:
        return {"ok": False, "faults": [f"unknown kind {kind!r}"]}
    faults = validate_record(rec)
    if faults:
        return {"ok": False, "faults": faults}
    # completion-proof: ANY commitment marked complete carries evidence —
    # regardless of the campaign's own status (sol t9#9)
    if kind == "campaign":
        for c in rec.get("commitments") or []:
            if c.get("status") == "complete" and not (c.get("evidence_refs") or []):
                prev = get(rec["id"], kind, base)
                was = False
                if isinstance(prev, dict):
                    for pc in prev.get("commitments") or []:
                        if pc.get("id") == c.get("id") and pc.get("status") == "complete":
                            was = True
                if not was:
                    return {"ok": False,
                            "faults": [f"commitment {c.get('id')} -> complete without evidence_refs "
                                       f"(queue completion alone never completes)"]}
    _registry_lock(base)
    try:
        return _put_locked(rec, actor, base, expected_revision, kind)
    finally:
        _registry_unlock()


def _put_locked(rec, actor, base, expected_revision, kind):
    rid = rec["id"]
    path = _rec_path(base, kind, rid)
    prev = None
    if path.exists():
        try:
            prev = json.loads(path.read_text())
        except Exception:
            return {"ok": False, "faults": ["existing record unreadable; refusing blind overwrite"]}
        if expected_revision is not None and prev.get("revision") != expected_revision:
            return {"ok": False, "faults": [f"stale write: expected revision "
                                            f"{expected_revision}, store has {prev.get('revision')}"]}
        if rec.get("revision", 0) <= (prev.get("revision") or 0):
            return {"ok": False, "faults": ["revision must increase"]}
    rec = dict(rec)
    rec.setdefault("revision", 1)
    rec["updated_at"] = NOW()
    rec["updated_by"] = actor
    atomic_write_json(path, rec)
    rp = Path(base).expanduser() / "receipts.jsonl"
    rp.parent.mkdir(parents=True, exist_ok=True)
    with open(rp, "a") as fh:
        fh.write(json.dumps({"at": NOW(), "actor": actor, "kind": kind, "id": rid,
                             "revision": rec["revision"],
                             "prev_revision": (prev or {}).get("revision"),
                             "record_hash": __import__("hashlib").sha256(
                                 canonical_json(rec)).hexdigest()}, sort_keys=True) + "\n")
        fh.flush()
        os.fsync(fh.fileno())
    return {"ok": True, "id": rid, "revision": rec["revision"]}


# ------------------------------------------------------------- seed library

def seed_trip_library(base=DEFAULT_REGISTRY):
    """The first library population: the 8 measured seat trippings (sol
    turn-5 table; evidence = the ygg incident log + session ledgers). Each
    trip ships its verified procedure and the fixture that guards it."""
    E = ["journal-papers/swarm-harness2/data/ygg-incident-log.md",
         "session ledgers /tmp/dreamer/{dream,audit}-ledger.jsonl"]
    trips = [
        ("trip-ssh-payload-quoting", "ssh heredoc quoting breaks payloads",
         "single quotes inside a payload broke a mega-heredoc; backticks in an unquoted heredoc executed remotely",
         "proc-stage-transfer-execute"),
        ("trip-unquoted-doc-heredoc", "documentation transferred through shell",
         "markdown backticks executed as command substitution, silently corrupting a remote SKILL.md",
         "proc-docs-as-data"),
        ("trip-drifted-replace-anchor", "silent s.replace no-op",
         "a drifted anchor made an s.replace edit land as a silent no-op; asserts now required",
         "proc-anchored-edit"),
        ("trip-import-root-errors", "package/import root confusion",
         "running a bin script from the wrong cwd raised import errors unrelated to the change",
         "proc-package-entrypoints"),
        ("trip-wrong-transcript-attempt", "stale filename-sorted attempt won",
         "audit graded a dead a1 transcript because attempts were sorted by name, not mtime/manifest",
         "proc-attempt-by-manifest"),
        ("trip-timer-runtime-nameerror", "compile-valid but runtime-dead",
         "an orphaned elif branch compiled fine and crashed every supervise pass for ~4h",
         "proc-exercise-edited-branch"),
        ("trip-restart-lock-races", "predecessor unlinked successor's lock",
         "a dying serve unconditionally removed serve.lock after its successor started; up to three serves",
         "proc-fenced-ownership"),
        ("trip-misread-evidence", "log text graded as repository truth",
         "a legitimate merge was declared a clobber from log text; merge-base proved otherwise",
         "proc-verify-objects-not-logs"),
    ]
    procs = [
        ("proc-stage-transfer-execute", "Stage locally, transfer, verify hash, execute",
         ["write payload locally", "scp to a unique remote path",
          "verify remote sha256 equals local", "execute the absolute remote path"],
         "apostrophes, backticks, $() and multiline payloads survive byte-for-byte",
         "reversible_change"),
        ("proc-docs-as-data", "Transfer documentation as data",
         ["treat docs as opaque bytes", "use the same staging helper as code",
          "verify hash before installing"],
         "embedded shell syntax in docs is never executed", "reversible_change"),
        ("proc-anchored-edit", "Assert the anchor, then edit",
         ["count expected anchor matches first", "fail loudly on 0 or >expected",
          "apply edit", "re-read and verify the resulting diff"],
         "missing or duplicated anchor fails without mutation", "reversible_change"),
        ("proc-package-entrypoints", "Use package entry points",
         ["run via console-script/entrypoint or python -m",
          "declare the installed/module layout explicitly"],
         "import succeeds from an unrelated working directory", "reversible_change"),
        ("proc-attempt-by-manifest", "Select attempts by manifest identity",
         ["resolve attempts from the task/attempt manifest",
          "tie-break by mtime, never filename order"],
         "a stale filename-sorted attempt can never win grading", "read_only"),
        ("proc-exercise-edited-branch", "Exercise old AND new runtime branches",
         ["import the edited module", "drive the edited path (not just new code)",
          "smoke the full runtime path before deploy"],
         "a compile-valid fixture fails before deployment", "reversible_change"),
        ("proc-fenced-ownership", "Drain, verify exit, fenced ownership, start",
         ["drain the predecessor", "verify process exit", "acquire an ownership token",
          "start the successor; never unlink a lock you do not own"],
         "a delayed predecessor cannot remove successor ownership", "reversible_change"),
        ("proc-verify-objects-not-logs", "Verify objects, not log text",
         ["check exit codes AND repository objects",
          "git merge-base --is-ancestor for lineage claims"],
         "exit code versus log text; legitimate merge versus overwrite", "read_only"),
    ]
    out = []
    for tid, title, failure, proc_id in trips:
        r = {"kind": "trip", "id": tid, "revision": 1, "title": title,
             "cue": title.lower(), "observed_failure": failure,
             "failure_mechanism": "measured; see evidence",
             "evidence_refs": E, "procedure_ref": proc_id,
             "status": "reproduced", "reviewer_ref": "owner+zcode-orchestrator"}
        out.append(put(r, "seed:swarm-harness2", base=base))
    for pid, title, steps, fixture, auth in procs:
        r = {"kind": "procedure", "id": pid, "revision": 1, "title": title,
             "cue": title.lower(), "scope": "yggterm agents, all harnesses",
             "preconditions": [], "steps": steps,
             "postconditions": ["result verified"], "stop_conditions": [],
             "rollback_ref": "not_applicable",
             "fixture_refs": [fixture], "evidence_refs": E,
             "authority_class": auth, "status": "verified",
             "verified_against": "measured 2026-09..10 sessions",
             "verified_at": NOW()}
        out.append(put(r, "seed:swarm-harness2", base=base))
    return out


def seed_campaign(base, cid, objective, **kw):
    rec = {"kind": "campaign", "id": cid, "revision": 1, "objective": objective,
           "status": kw.pop("status", "active"),
           "responsible_role_ref": kw.pop("responsible_role_ref", "core-keeper"),
           "authority_ref": kw.pop("authority_ref", "owner"),
           "commitments": kw.pop("commitments", []),
           "blockers": kw.pop("blockers", []),
           "decision_refs": kw.pop("decision_refs", []),
           "ask_refs": kw.pop("ask_refs", []),
           "procedure_cues": kw.pop("procedure_cues", []),
           "intent_ref": kw.pop("intent_ref", None),
           "source": kw.pop("source", {"uri": "msggraph boards + repo docs"}),
           "review_due_at": kw.pop("review_due_at", None)}
    rec.update(kw)
    return put(rec, "seed:swarm-harness2", base=base)


# ------------------------------------------------------------------ CLI

def main():
    import argparse
    ap = argparse.ArgumentParser(prog="registry")
    ap.add_argument("verb", choices=["list", "get", "put", "validate", "seed", "receipts"])
    ap.add_argument("--kind")
    ap.add_argument("--id")
    ap.add_argument("--file")
    ap.add_argument("--base", default=str(DEFAULT_REGISTRY))
    a = ap.parse_args()
    base = a.base
    if a.verb == "list":
        for r in list_records(a.kind, base):
            print(f"{r.get('kind','?'):10} {r.get('id','?'):32} rev{r.get('revision','?')} "
                  f"{r.get('status','?'):12} {r.get('title','')[:60]}")
        return 0
    if a.verb == "get":
        print(json.dumps(get(a.id, a.kind, base), indent=2))
        return 0
    if a.verb == "validate":
        rec = json.loads(Path(a.file).read_text())
        f = validate_record(rec)
        print(json.dumps({"valid": not f, "faults": f}))
        return 0 if not f else 2
    if a.verb == "put":
        rec = json.loads(Path(a.file).read_text())
        print(json.dumps(put(rec, actor=os.environ.get("USER", "cli"), base=base)))
        return 0
    if a.verb == "seed":
        out = seed_trip_library(base)
        ok = all(r.get("ok") for r in out)
        print(json.dumps({"seeded": len(out), "ok": ok}))
        return 0 if ok else 2
    if a.verb == "receipts":
        p = Path(base).expanduser() / "receipts.jsonl"
        for line in (p.read_text().splitlines() if p.exists() else []):
            print(line)
        return 0
    return 1


if __name__ == "__main__":
    import sys
    sys.exit(main())
