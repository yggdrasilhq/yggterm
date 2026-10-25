"""ygg-swarm-maintenance-orchestrator — the DETERMINISTIC maintenance
boundary (sol turn-5/6). No model. No network creativity. It:

  validates/activates the swarm config (activation store, never the workspace),
  captures workspace edits as proposals (multi-directional ledger),
  as controller: reconciles every host (capture-before-pull), adjudicates the
    frozen batch, publishes accepted releases, collects acknowledgements,
  as replica: pushes its outbox to dev and installs verified accepted chain,
  supervises the executor adapter (bounded restarts, incident escalation),
  writes a heartbeat the whole fleet can inspect.

Permitted actions are the allowlist in swarm-config.yaml maintenance.*.
Violations are defects, not judgment. Serve/DENY latches are never cleared
here (owner verb only).

Usage:
  python3 maintenance.py once [--state DIR]     # single maintenance pass
  python3 maintenance.py daemon [--state DIR]   # poll loop (systemd unit)
  python3 maintenance.py reconcile [--state DIR]
  python3 maintenance.py status [--state DIR]
"""
import argparse
import json
import os
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import swarm_ledger as sl  # noqa: E402
import swarmconfig as sc   # noqa: E402

DEFAULT_EXECUTOR_UNIT = "ygg-swarm-executor@zcode.service"
SAFE_HOST = frozenset("abcdefghijklmnopqrstuvwxyz0123456789-.")


def _cfg(res):
    return res.get("config") or {}


def _hosts(cfg):
    hosts = ((cfg.get("fleet") or {}).get("hosts")) or []
    return [h for h in hosts if h and set(h) <= SAFE_HOST]


def _poll_seconds(cfg):
    ps = (((cfg.get("maintenance") or {}).get("supervisor")) or {}).get("poll_seconds")
    return ps if isinstance(ps, int) and ps >= 5 else 30


def _role(cfg, host):
    fleet = cfg.get("fleet") or {}
    sup = ((cfg.get("maintenance") or {}).get("supervisor")) or {}
    if not fleet:
        return "controller"  # no config: deterministic defaults keep containment alive
    if host == (fleet.get("authority_host") or "dev"):
        return "controller"
    if host in (sup.get("replica_hosts") or []):
        return "replica"
    return "replica"


def _ssh(host, argv, timeout=20, input_bytes=None):
    """Transport adapter (ssh, at-least-once). Host names are pre-validated
    against SAFE_HOST; argv list form never passes through a shell."""
    cmd = ["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", host, "--"] + argv
    try:
        p = subprocess.run(cmd, capture_output=True, timeout=timeout,
                           input=input_bytes)
        return {"ok": p.returncode == 0, "rc": p.returncode,
                "stdout": p.stdout, "stderr": p.stderr[:400]}
    except subprocess.TimeoutExpired:
        return {"ok": False, "rc": None, "stdout": b"", "stderr": b"timeout"}
    except FileNotFoundError:
        return {"ok": False, "rc": None, "stdout": b"", "stderr": b"ssh missing"}


# ------------------------------------------------------------ reconcile

def reconcile(state=sc.DEFAULT_STATE):
    """Controller: capture locally, pull every reachable replica's outbox
    BEFORE installing anything there, adjudicate the frozen batch, publish,
    then install on replicas and collect acks."""
    res = sc.load(state)
    cfg = _cfg(res)
    rep = sl.init_state(state)
    cap = sl.capture(state)
    head = sc.active_head(state) or {}

    out = {"role": "controller", "captured": cap.get("captured"),
           "head": head.get("content_hash", "")[:12], "hosts": {}}
    # NO adjudication before remote collection: the frozen batch must include
    # every reachable host's proposals or same-batch owner priority is a lie

    sync_dir = Path(state).expanduser() / "sync" / "config"
    inbox = sync_dir / "inbox"
    for host in _hosts(cfg):
        if host == rep.get("host"):
            continue
        entry = {"reachable": False}
        # 1. capture-before-pull: pull the replica's outbox first
        remote_code = (
            "import sys, json, os; "
            "sys.path.insert(0, os.path.expanduser('~/.yggterm/swarmkernel'));"
            "sys.path.insert(0, os.path.expanduser('~/gh/yggterm/tools/swarmharness'));"
            "import swarm_ledger as sl; "
            "r = sl.capture(os.path.expanduser('~/.yggterm/swarm'), reason='controller-invoked'); "
            "print(json.dumps(r))"
        )
        r = _ssh(host, ['python3 -c "' + remote_code + '"'], timeout=25)
        if r["ok"]:
            entry["reachable"] = True
            try:
                entry["remote_capture"] = json.loads(r["stdout"].decode() or "{}")
            except Exception:
                entry["remote_capture"] = {"error": "unparseable"}
        # 2. fetch outbox bundles (at-least-once)
        r = _ssh(host, ["cd ~/.yggterm/swarm/sync/config/outbox 2>/dev/null "
                        "&& tar -cf - . || true"],
                 timeout=25)
        if r["ok"] and r["stdout"].strip():
            dest = inbox / host
            dest.mkdir(parents=True, exist_ok=True)
            t = subprocess.run(["tar", "-xf", "-", "-C", str(dest)],
                               input=r["stdout"], capture_output=True)
            entry["ingested"] = t.returncode == 0
        out["hosts"][host] = entry

    # 3. adjudicate the frozen batch (now includes remote proposals)
    verdicts = sl.adjudicate(state)
    out["verdicts"] = {v["change_id"]: v["decision"] for v in verdicts}
    head = sc.active_head(state) or {}
    out["head"] = head.get("content_hash", "")[:12]

    # consumer acknowledgements (sol turn-7 #5 tail): who has NOT acked the
    # active revision — distinct accepted/installed/acked states
    ack_dir = Path(state).expanduser() / "config" / "acks"
    wanted = ((cfg.get("sync") or {}).get("activation_ack_consumers")) or []
    acks = {}
    for c in wanted:
        try:
            rec = json.loads((ack_dir / f"{c}.json").read_text())
            acks[c] = rec.get("content_hash") or ""
        except Exception:
            acks[c] = None
    out["acks"] = {c: (h[:12] if h else None) for c, h in acks.items()}
    out["unacked"] = sorted(c for c, h in acks.items() if h != head.get("content_hash"))

    # 4. publish + install on replicas (verified bundles only)
    if head:
        rel_dir = sc._paths(Path(state).expanduser())["releases"] / head["content_hash"]
        import io as _io, tarfile as _tarfile
        _buf = _io.BytesIO()
        with _tarfile.open(fileobj=_buf, mode="w") as _tf:
            for _name in ("release.yaml", "release.json", "journal_tail.jsonl"):
                if _name == "journal_tail.jsonl":
                    # ship the acceptance chain so replicas verify ANCESTRY,
                    # not just bundle identity (sol turn-7 #5)
                    _recs = sl.journal_records(state)
                    if _recs:
                        _data = "\n".join(json.dumps(r, sort_keys=True)
                                          for r in _recs).encode() + b"\n"
                    else:
                        continue
                else:
                    _f = rel_dir / _name
                    if not _f.exists():
                        continue
                    _data = _f.read_bytes()
                _ti = _tarfile.TarInfo(_name)
                _ti.size = len(_data)
                _tf.addfile(_ti, _io.BytesIO(_data))
        payload = _buf.getvalue()
        pub_local = Path(state).expanduser() / "sync" / "config" / "published"
        pub_local.mkdir(parents=True, exist_ok=True)
        (pub_local / "bundle.tar").write_bytes(payload)
        for host in out["hosts"]:
            if not out["hosts"][host].get("reachable"):
                out["hosts"][host]["install"] = "pending"
                continue
            r = _ssh(host, ["mkdir -p ~/.yggterm/swarm/sync/config/published "
                            "&& cat > ~/.yggterm/swarm/sync/config/published/bundle.tar"],
                     timeout=30, input_bytes=payload)
            if not r["ok"]:
                out["hosts"][host]["install"] = f"push-failed rc={r['rc']}"
                continue
            inst_code = (
                "import sys, json, io, tarfile, pathlib, os; "
                "sys.path.insert(0, os.path.expanduser('~/.yggterm/swarmkernel'));"
                "sys.path.insert(0, os.path.expanduser('~/gh/yggterm/tools/swarmharness'));"
                "import swarm_ledger as sl, swarmconfig as sc; "
                "b = pathlib.Path('~/.yggterm/swarm/sync/config/published/bundle.tar').expanduser().read_bytes(); "
                "tf = tarfile.open(fileobj=io.BytesIO(b)); "
                "files = {m.name.split('/')[-1]: tf.extractfile(m).read() for m in tf.getmembers()}; "
                "body = json.loads(files['release.json']); "
                "h = sc.content_hash(body); "
                "rd = pathlib.Path('~/.yggterm/swarm/config/releases').expanduser() / h; "
                "rd.mkdir(parents=True, exist_ok=True); "
                "(rd / 'release.yaml').write_bytes(files['release.yaml']); "
                "(rd / 'release.json').write_bytes(files['release.json']); "
                "jr = [json.loads(x) for x in files['journal_tail.jsonl'].decode().splitlines() if x.strip()] if 'journal_tail.jsonl' in files else None; "
                "print(json.dumps(sl.install_accepted(os.path.expanduser('~/.yggterm/swarm'), h, origin_bytes=files['release.yaml'], shipped_journal=jr)))"
            )
            r2 = _ssh(host, ['python3 -c "' + inst_code + '"'], timeout=30)
            try:
                out["hosts"][host]["install"] = json.loads(r2["stdout"].decode() or "{}")
            except Exception:
                out["hosts"][host]["install"] = {"ok": False, "stderr": r2["stderr"][:200]}
    return out


def replica_sync(state=sc.DEFAULT_STATE, dev_host="dev"):
    """Replica: push outbox to dev's inbox, pull + verify + install accepted."""
    res = sc.load(state)
    cfg = _cfg(res)
    authority = (cfg.get("fleet") or {}).get("authority_host") or dev_host
    sl.init_state(state)
    cap = sl.capture(state)
    out = {"role": "replica", "captured": cap.get("captured")}
    sync_dir = Path(state).expanduser() / "sync" / "config"
    # push outbox bundles (at-least-once; dev dedupes by change_id)
    bundles = sync_dir / "outbox"
    if bundles.is_dir() and any(bundles.iterdir()):
        t = subprocess.run(["tar", "-cf", "-", "."], cwd=bundles, capture_output=True)
        if t.stdout:
            host = sl.load_replica(state).get("host") or "unknown"
            r = _ssh(authority, [f"mkdir -p ~/.yggterm/swarm/sync/config/inbox/{host} "
                                 f"&& tar -xf - -C ~/.yggterm/swarm/sync/config/inbox/{host}"],
                     timeout=25, input_bytes=t.stdout)
            out["push"] = "ok" if r["ok"] else f"rc={r['rc']}"
    # pull the published head bundle
    r = _ssh(authority, ["cat ~/.yggterm/swarm/sync/config/published/bundle.tar 2>/dev/null"],
             timeout=30)
    if r["ok"] and r["stdout"].strip():
        import io, tarfile
        try:
            tf = tarfile.open(fileobj=io.BytesIO(r["stdout"]))
            files = {m.name.split("/")[-1]: tf.extractfile(m).read() for m in tf.getmembers()}
            body = json.loads(files["release.json"])
            h = sc.content_hash(body)
            rd = Path(state).expanduser() / "config" / "releases" / h
            rd.mkdir(parents=True, exist_ok=True)
            (rd / "release.yaml").write_bytes(files["release.yaml"])
            (rd / "release.json").write_bytes(files["release.json"])
            jr = [json.loads(x) for x in files["journal_tail.jsonl"].decode().splitlines()
                  if x.strip()] if "journal_tail.jsonl" in files else None
            inst = sl.install_accepted(state, h, origin_bytes=files["release.yaml"],
                                       shipped_journal=jr)
            out["install"] = inst
        except Exception as e:  # noqa: BLE001
            out["install"] = {"ok": False, "detail": repr(e)[:160]}
    return out


# ------------------------------------------------------------ supervision

def _incidents_path(state):
    return Path(state).expanduser() / "control" / "incidents.jsonl"


def _incident_append(state, rec):
    p = _incidents_path(state)
    p.parent.mkdir(parents=True, exist_ok=True)
    with open(p, "a") as f:
        f.write(json.dumps(rec, sort_keys=True) + "\n")
        f.flush()
        os.fsync(f.fileno())


def supervise_executor(state, cfg, unit=DEFAULT_EXECUTOR_UNIT):
    """Bounded dependency supervision: check the executor unit, restart within
    the configured limits, escalate to the durable incident log beyond them."""
    sup = ((cfg.get("maintenance") or {}).get("supervisor")) or {}
    rl = sup.get("restart_limit") or {"attempts": 3, "window_seconds": 900}
    r = subprocess.run(["systemctl", "--user", "is-active", unit],
                       capture_output=True, text=True, timeout=15)
    active = r.returncode == 0 and r.stdout.strip() == "active"
    entry = {"unit": unit, "active": active, "checked_at": sc._now()}
    if not active:
        # FAILED restart attempts count toward the window too (sol t7 #6):
        # an unbounded rc=5 loop must escalate, not spin
        recent = _incidents_read(state, unit, window=rl.get("window_seconds", 900))
        if len(recent) >= rl.get("attempts", 3):
            entry["action"] = "escalated"
            _incident_append(state, {"unit": unit, "action": "escalate-to-owner",
                                     "attempts_in_window": len(recent), "at": sc._now()})
        else:
            try:
                r2 = subprocess.run(["systemctl", "--user", "restart", unit],
                                    capture_output=True, text=True, timeout=20)
                rc = r2.returncode
            except subprocess.TimeoutExpired:
                rc = -1
            entry["action"] = "restarted" if rc == 0 else f"restart-failed rc={rc}"
            _incident_append(state, {"unit": unit, "action": entry["action"], "at": sc._now()})
    return entry


def _incidents_read(state, unit, window):
    cutoff = time.time() - window
    out = []
    try:
        for line in _incidents_path(state).read_text().splitlines():
            try:
                rec = json.loads(line)
            except Exception:
                continue
            # incident timestamps are ISO; parse just the restarts for this unit
            if rec.get("unit") == unit and (rec.get("action") == "restarted"
                                            or str(rec.get("action", "")).startswith("restart-failed")):
                import datetime
                try:
                    ts = datetime.datetime.strptime(rec["at"], "%Y-%m-%dT%H:%M:%SZ")
                    ts = ts.replace(tzinfo=datetime.timezone.utc).timestamp()
                    if ts >= cutoff:
                        out.append(rec)
                except Exception:
                    continue
    except FileNotFoundError:
        pass
    return out


# ------------------------------------------------------------ pass + loop

def maintenance_pass(state=sc.DEFAULT_STATE):
    started = time.time()
    res = sc.load(state)
    cfg = _cfg(res)
    host = sl.load_replica(state) and sl.load_replica(state).get("host") or os.uname().nodename.split(".")[0]
    role = _role(cfg, host)
    out = {"at": sc._now(), "role": role, "host": host,
           "config_mode": res["mode"],
           "denials": sorted((res.get("denials") or {})),
           "faults": res.get("faults", [])[:4]}
    try:
        if role == "controller":
            out["reconcile"] = reconcile(state)
        else:
            out["replica_sync"] = replica_sync(state)
    except Exception as e:  # noqa: BLE001 — the supervisor never dies from a sync fault
        out["sync_fault"] = repr(e)[:200]
    if cfg:
        dispatch = (cfg.get("fleet") or {}).get("dispatch_host")
        if dispatch and host != dispatch:
            out["executor"] = {"unit": DEFAULT_EXECUTOR_UNIT, "supervised": False,
                               "note": f"executor runs on {dispatch}; not supervised here"}
        else:
            try:
                out["executor"] = supervise_executor(state, cfg)
            except FileNotFoundError:
                out["executor"] = {"unit": DEFAULT_EXECUTOR_UNIT, "active": None,
                                   "note": "systemctl unavailable"}
    out["elapsed_s"] = round(time.time() - started, 2)
    if cfg and res["mode"] == "active":
        try:
            sc.ack("ygg-swarm-maintenance-orchestrator", state)
        except Exception:
            pass
    hb = Path(state).expanduser() / "supervisor.json"
    sc.atomic_write_json(hb, out)
    return out


def daemon(state, interval=None):
    while True:
        try:
            res = sc.load(state)
            interval = interval or _poll_seconds(_cfg(res))
            out = maintenance_pass(state)
            print(json.dumps({"at": out["at"], "role": out["role"],
                              "mode": out["config_mode"]}), flush=True)
        except Exception as e:  # noqa: BLE001 — containment posture, never crash
            print(f"maintenance fault (contained): {e!r}", file=sys.stderr, flush=True)
            interval = interval or 30
        time.sleep(interval)


def main():
    ap = argparse.ArgumentParser(prog="ygg-swarm-maintenance")
    ap.add_argument("verb", choices=["once", "daemon", "reconcile", "status", "replica-sync"])
    ap.add_argument("--state", default=str(sc.DEFAULT_STATE))
    ap.add_argument("--interval", type=int, default=None)
    a = ap.parse_args()
    state = Path(a.state).expanduser()
    if a.verb == "daemon":
        daemon(state, a.interval)
        return 0
    if a.verb == "once":
        print(json.dumps(maintenance_pass(state), indent=2, default=str))
        return 0
    if a.verb == "reconcile":
        print(json.dumps(reconcile(state), indent=2, default=str))
        return 0
    if a.verb == "replica-sync":
        print(json.dumps(replica_sync(state), indent=2, default=str))
        return 0
    if a.verb == "status":
        print(json.dumps({"load": sc.status(state),
                          "replica": sl.load_replica(state),
                          "journal_ok": sl.verify_ancestry(state)[0],
                          "supervisor": json.loads((state / "supervisor.json").read_text())
                          if (state / "supervisor.json").exists() else None}, indent=2, default=str))
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
