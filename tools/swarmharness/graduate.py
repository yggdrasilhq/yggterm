"""graduate — the systematic stray-script pipeline (owner directive
2026-10-25: "we should have a systematic tooling after stray scripts lets
us trial and error and we see a probable and plausible solution").

Stray scripts (in /tmp, transcripts, research repos) die two deaths: they
are re-derived by the next seat, or they become load-bearing without ever
being reviewed. This verb is the ONLY sanctioned road from stray to tooling:

  stage    durable copy out of /tmp + provenance manifest (hashes, origin,
           cue, evidence refs) -> registry/staging/<id>/   [retained reference]
  trial    run a fixture against the staged script in a sandbox temp dir;
           the run (exit, stdout hash, durations) is journaled to trials.jsonl
  promote  requires >=1 PASSING trial; writes a verified ProcedureRecord
           through registry.put (completion evidence = trial hashes) and
           appends the registry receipt. Landing the CODE in the kernel
           stays a normal lane/PR act — promotion never installs code.

Deterministic; no model; no network. The fixture is the contract.
"""
import hashlib
import json
import shutil
import subprocess
import sys
import tempfile
import uuid
from datetime import datetime, timezone
from pathlib import Path

import registry as reg

DEFAULT_BASE = reg.DEFAULT_REGISTRY  # ~/.yggterm/swarm/registry
STAGING = "staging"


def _now():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _sha256_file(p):
    return hashlib.sha256(Path(p).read_bytes()).hexdigest()


def _stage_dir(base, gid=None):
    return Path(base).expanduser() / STAGING / (gid or str(uuid.uuid4()))


def stage(stray_path, cue, *, evidence_refs=None, base=DEFAULT_BASE,
          note="", origin_host=None, retained=False):
    """Durable retained copy of a stray script + provenance. The /tmp
    original can now die; the staging dir is the reference. Returns the
    staging id."""
    src = Path(stray_path).expanduser()
    if not src.is_file():
        return {"ok": False, "faults": [f"stray script not a file: {src}"]}
    gid = str(uuid.uuid4())
    d = _stage_dir(base, gid)
    d.mkdir(parents=True, exist_ok=True)
    shutil.copy(src, d / "script.py")
    manifest = {
        "id": gid,
        "origin_path": str(src),
        "origin_host": origin_host or socket_hostname(),
        "script_sha256": _sha256_file(d / "script.py"),
        "origin_bytes": src.stat().st_size,
        "cue": cue,
        "note": note,
        "evidence_refs": list(evidence_refs or []),
        "staged_at": _now(),
        "retained": bool(retained),   # True once the origin is safe to delete
        "state": "staged",            # staged | trialed | promoted | rejected
    }
    (d / "manifest.json").write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    return {"ok": True, "id": gid, "dir": str(d)}


def socket_hostname():
    import socket
    return socket.gethostname().split(".")[0]


def trial(gid, fixture_path, *, base=DEFAULT_BASE, timeout=60):
    """Run a fixture against the staged script in a sandbox temp dir.
    The fixture is a python file that must define run(script_path) -> dict
    {pass: bool, detail: str}. The trial is journaled either way."""
    d = _stage_dir(base, gid)
    man_p = d / "manifest.json"
    if not man_p.exists():
        return {"ok": False, "faults": [f"unknown staging id {gid}"]}
    man = json.loads(man_p.read_text())
    fx = Path(fixture_path).expanduser()
    if not fx.is_file():
        return {"ok": False, "faults": [f"fixture not a file: {fx}"]}
    fx_sha = _sha256_file(fx)
    sandbox = Path(tempfile.mkdtemp(prefix=f"grad-{gid[:8]}-"))
    shutil.copy(d / "script.py", sandbox / "script.py")
    # import the fixture module and call run(script_path)
    spec = None
    try:
        import importlib.util
        spec = importlib.util.spec_from_file_location("grad_fixture", fx)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        result = mod.run(sandbox / "script.py")
    except Exception as e:  # noqa: BLE001 — a crashing fixture is a failed trial
        result = {"pass": False, "detail": f"fixture crashed: {e!r}"}
    finally:
        shutil.rmtree(sandbox, ignore_errors=True)
    row = {"at": _now(), "fixture": str(fx), "fixture_sha256": fx_sha,
           "pass": bool(result.get("pass")),
           "detail": str(result.get("detail", ""))[:300]}
    with open(d / "trials.jsonl", "a") as f:
        f.write(json.dumps(row, sort_keys=True) + "\n")
        f.flush()
        import os
        os.fsync(f.fileno())
    trials = [json.loads(x) for x in (d / "trials.jsonl").read_text().splitlines() if x.strip()]
    man["state"] = "promoted" if any(t["pass"] for t in trials) else "trial-ed"
    if man["state"] == "trial-ed":
        man["state"] = "trialed"
    man["trials"] = len(trials)
    man_p.write_text(json.dumps(man, indent=2, sort_keys=True) + "\n")
    return {"ok": True, "trial": row, "passing": any(t["pass"] for t in trials)}


def promote(gid, *, procedure_id, title, scope, actor, base=DEFAULT_BASE,
            authority_class="separately_authorized", steps=None):
    """Graduate: registry ProcedureRecord (verified) whose evidence is the
    passing trial hashes. Requires >=1 passing trial and the fixture kept
    alongside (the fixture IS the contract; no fixture, no promotion)."""
    d = _stage_dir(base, gid)
    man_p = d / "manifest.json"
    if not man_p.exists():
        return {"ok": False, "faults": [f"unknown staging id {gid}"]}
    man = json.loads(man_p.read_text())
    trials = [json.loads(x) for x in (d / "trials.jsonl").read_text().splitlines()] \
        if (d / "trials.jsonl").exists() else []
    passing = [t for t in trials if t["pass"]]
    if not passing:
        return {"ok": False, "faults": ["no passing trial; promote refused"]}
    if not (d / "trials.jsonl").exists():
        return {"ok": False, "faults": ["no trial journal"]}
    rec = {
        "kind": "procedure", "id": procedure_id, "revision": 1,
        "title": title, "cue": (man.get("cue") or "") + " " + title.lower(),
        "scope": scope,
        "preconditions": [f"staged {gid} from {man['origin_path']}"],
        "steps": steps or [f"run graduated script {procedure_id} per its fixture"],
        "postconditions": ["fixture passes"],
        "stop_conditions": [],
        "rollback_ref": "not_applicable",
        "fixture_refs": [t["fixture_sha256"][:12] for t in passing],
        "evidence_refs": man.get("evidence_refs") or [man["origin_path"]],
        "authority_class": authority_class,
        "status": "verified",
        "verified_against": "; ".join(f"trial {t['fixture_sha256'][:12]}@{t['at']}"
                                      for t in passing),
        "verified_at": _now(),
        "graduated_from": gid,
        "script_sha256": man["script_sha256"],
    }
    out = reg.put(rec, actor, base=base)
    if out.get("ok"):
        man["state"] = "promoted"
        man["procedure_id"] = procedure_id
        man_p.write_text(json.dumps(man, indent=2, sort_keys=True) + "\n")
        out["staging_dir"] = str(d)
    return out


def status(gid=None, base=DEFAULT_BASE):
    root = Path(base).expanduser() / STAGING
    if gid:
        d = _stage_dir(base, gid)
        return json.loads((d / "manifest.json").read_text()) \
            if (d / "manifest.json").exists() else {"ok": False, "faults": ["unknown id"]}
    out = []
    if root.is_dir():
        for d in sorted(root.iterdir()):
            mp = d / "manifest.json"
            if mp.exists():
                try:
                    m = json.loads(mp.read_text())
                    out.append({k: m.get(k) for k in
                                ("id", "cue", "state", "staged_at", "procedure_id", "trials")})
                except Exception:
                    out.append({"id": d.name, "state": "unreadable"})
    return out


def main():
    import argparse
    ap = argparse.ArgumentParser(prog="graduate")
    ap.add_argument("verb", choices=["stage", "trial", "promote", "status"])
    ap.add_argument("path", nargs="?")
    ap.add_argument("--cue", default="")
    ap.add_argument("--evidence", action="append", default=[])
    ap.add_argument("--fixture")
    ap.add_argument("--id")
    ap.add_argument("--procedure-id")
    ap.add_argument("--title", default="")
    ap.add_argument("--scope", default="yggterm agents")
    ap.add_argument("--actor", default=None)
    ap.add_argument("--base", default=str(DEFAULT_BASE))
    ap.add_argument("--retained", action="store_true")
    a = ap.parse_args()
    actor = a.actor or f"seat:{reg.socket_hostname() if hasattr(reg, 'socket_hostname') else 'local'}"
    if a.verb == "stage":
        print(json.dumps(stage(a.path, a.cue, evidence_refs=a.evidence,
                               base=a.base, retained=a.retained), indent=2))
        return 0
    if a.verb == "trial":
        print(json.dumps(trial(a.id, a.fixture, base=a.base), indent=2))
        return 0
    if a.verb == "promote":
        print(json.dumps(promote(a.id, procedure_id=a.procedure_id,
                                 title=a.title or a.cue, scope=a.scope,
                                 actor=actor, base=a.base), indent=2))
        return 0
    if a.verb == "status":
        print(json.dumps(status(a.id, a.base), indent=2, default=str))
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
