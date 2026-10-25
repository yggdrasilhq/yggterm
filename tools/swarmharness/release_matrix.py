#!/usr/bin/env python3
"""release_matrix — the offline catastrophe demos (sol turn-5 matrix,
turn-6 amendments). Each scenario prints PASS/FAIL with evidence. The
full conformance suite (test_swarmharness.py) covers S1–S9 units; this
script demonstrates the END-TO-END scenario 10 (replica-originated
convergence) plus quick replay/denial proofs, using the REAL protocol
functions with the SSH transport mocked by direct file transfer.

Run: python3 tools/swarmharness/release_matrix.py
"""
import hashlib
import json
import shutil
import sys
import tempfile
import uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))

import swarm_ledger as sl  # noqa: E402
import swarmconfig as sc   # noqa: E402

GENESIS = (HERE / "fixtures" / "valid" / "genesis.yaml").read_bytes()
CFG = (HERE.parent / "swarm-config.yaml").read_bytes()

PASS, FAIL = [], []


def check(name, cond, detail=""):
    (PASS if cond else FAIL).append(name)
    print(("ok   " if cond else "FAIL ") + name + ("" if cond else f" — {detail}"))


def transfer_outbox(src_state, src_host, dst_state):
    """Mock of the ssh tar pull: dev ingests a replica's outbox bundles."""
    src = Path(src_state).expanduser() / "sync" / "config" / "outbox"
    dst = Path(dst_state).expanduser() / "sync" / "config" / "inbox" / src_host
    dst.mkdir(parents=True, exist_ok=True)
    n = 0
    if src.is_dir():
        for cid in src.iterdir():
            if (cid / "envelope.json").exists():
                d = dst / cid.name
                d.mkdir(parents=True, exist_ok=True)
                shutil.copy(cid / "envelope.json", d / "envelope.json")
                shutil.copy(cid / "candidate.yaml", d / "candidate.yaml")
                n += 1
    return n


def transfer_release(src_state, dst_state):
    """Mock of the ssh push + remote install: copy the sealed bundle and
    install it through the verified path."""
    head = sc.active_head(src_state)
    rel = sc._paths(Path(src_state).expanduser())["releases"] / head["content_hash"]
    rd = sc._paths(Path(dst_state).expanduser())["releases"] / head["content_hash"]
    rd.mkdir(parents=True, exist_ok=True)
    shutil.copy(rel / "release.yaml", rd / "release.yaml")
    shutil.copy(rel / "release.json", rd / "release.json")
    return sl.install_accepted(dst_state, head["content_hash"],
                               origin_bytes=(rel / "release.yaml").read_bytes())


def scenario_10():
    """Replica-originated convergence: owner edit on jojo while disconnected
    beats a conflicting agent proposal on dev; chain distributes; repeated
    reconcile is idempotent."""
    dev = Path(tempfile.mkdtemp(prefix="s10-dev-"))
    jojo = Path(tempfile.mkdtemp(prefix="s10-jojo-"))
    ws = jojo / "workspace.yaml"

    # both hosts start at H (genesis); the owner key distributes to the
    # replica at bootstrap (local authority token; dev verifies signatures)
    sc.accept(GENESIS, proposed_by="owner", authority_class="owner", state_dir=dev)
    sl.init_state(dev, host="dev")
    sc.atomic_write(ws, GENESIS)
    sl.init_state(jojo, workspace=ws, host="jojo")
    (jojo / "control").mkdir(parents=True, exist_ok=True)
    shutil.copy(dev / "control" / "owner.key", jojo / "control" / "owner.key")
    transfer_release(dev, jojo)
    base = sc.active_head(dev)["content_hash"]
    check("S10 both hosts at H",
          sc.active_head(jojo)["content_hash"] == base
          and sl.load_replica(jojo)["active_head"]["canonical_hash"] == base)

    # DISCONNECTED owner edit on jojo (planes flip; policy = owner authority)
    cfgH = json.loads(GENESIS.decode()) if False else None
    g = sc.parse_yaml_bytes(GENESIS)[0]
    import copy
    edited = copy.deepcopy(g)
    edited["planes"]["plane1"]["owner_policy"] = "draining"
    eb = sc.canonical_json(edited)
    sc.atomic_write(ws, eb)
    cap = sl.capture(jojo, workspace=ws)
    check("S10 jojo captures owner edit", cap["captured"] is True)
    sl.endorse_outbox(jojo, "config", cap["change_id"])  # owner endorses on jojo

    # conflicting AGENT proposal against H on dev (supply edit; agent-delegated)
    cfgA = copy.deepcopy(g)
    cfgA["supply"]["schedules"] = {"wake_interval_min": 42}
    ab = sc.canonical_json(cfgA)
    dev_sync = dev / "sync" / "config" / "outbox" / str(uuid.uuid4())
    dev_sync.mkdir(parents=True)
    (dev_sync / "candidate.yaml").write_bytes(ab)
    env = {"protocol_version": 1, "area": "config", "change_id": dev_sync.name,
           "origin_host": "dev-local",
           "parent": {"authority_epoch": 1, "revision": 1, "canonical_hash": base},
           "candidate_bytes_hash": hashlib.sha256(ab).hexdigest(),
           "claimed_principal": "core-keeper", "authority_ref": None,
           "reason": "agent schedule tweak", "derived_from": None}
    (dev_sync / "envelope.json").write_text(json.dumps(env, sort_keys=True))

    # RECONNECT: dev reconcile — capture-before-pull, frozen batch, owner wins
    pulled = transfer_outbox(jojo, "jojo", dev)
    check("S10 reconcile pulled jojo outbox", pulled == 1)
    verdicts = sl.adjudicate(dev)
    dec = {v["change_id"]: v["decision"] for v in verdicts}
    check("S10 owner proposal accepted (batch priority)",
          dec.get(cap["change_id"]) == "accepted", json.dumps(dec))
    check("S10 agent proposal STALE_PARENT (loser preserved)",
          dec.get(dev_sync.name) == "STALE_PARENT", json.dumps(dec))
    check("S10 loser bytes preserved (outbox + verdict)",
          (dev_sync / "candidate.yaml").exists()
          and (dev / "sync" / "config" / "verdicts" / f"{dev_sync.name}.json").exists())

    # distribution: jojo installs the accepted chain
    inst = transfer_release(dev, jojo)
    check("S10 jojo installed accepted chain", inst.get("installed") is True,
          json.dumps(inst))
    check("S10 both hosts converge to H2",
          sc.active_head(dev)["content_hash"] == sc.active_head(jojo)["content_hash"])
    lr = sc.load(state_dir=jojo)
    check("S10 jojo load active at H2", lr["mode"] == "active"
          and lr["revision"]["content_hash"] == sc.active_head(dev)["content_hash"])
    check("S10 jojo workspace still dirty (never auto-overwritten)",
          sl.load_replica(jojo)["workspace_base_bytes_hash"]
          == hashlib.sha256(GENESIS).hexdigest())

    # idempotence: repeated reconcile captures nothing new, commits nothing new
    cap2 = sl.capture(jojo, workspace=ws)
    check("S10 repeat capture dedupes", cap2.get("deduped") is True
          and cap2.get("change_id") == cap["change_id"])
    journal_before = len(sl.journal_records(dev))
    v2 = sl.adjudicate(dev)
    journal_after = len(sl.journal_records(dev))
    check("S10 repeat adjudicate commits nothing (journal append-only, unchanged)",
          journal_after == journal_before, f"{journal_before} -> {journal_after}")

    for p in (dev, jojo):
        shutil.rmtree(p, ignore_errors=True)


def quick_proofs():
    # S7 (deny persistence) quick end-to-end proof
    sd = Path(tempfile.mkdtemp(prefix="s7-"))
    sc.accept(CFG, proposed_by="owner", authority_class="owner", state_dir=sd)
    sc.deny_set("plane/autoclaw-chat", "owner stop (BUG-005)", state_dir=sd)
    # catastrophe: destroy the pointer + checkpoint + releases
    P = sc._paths(sd)
    (P["active"]).write_text("{corrupt")
    (P["checkpoint"]).write_text("{corrupt")
    for d in P["releases"].iterdir():
        (d / "release.json").write_text("{corrupt")
    lr = sc.load(state_dir=sd)
    check("S7 deny survives total store corruption",
          "plane/autoclaw-chat" in lr["denials"], json.dumps(lr["faults"])[:150])
    shutil.rmtree(sd, ignore_errors=True)

    # S-replay: identical replay acks, mismatched ID reuse rejects (unit-level
    # already in the suite; prove at the real-fleet-config level)
    sd2 = Path(tempfile.mkdtemp(prefix="sreplay-"))
    r1 = sc.accept(CFG, proposed_by="owner", authority_class="owner", state_dir=sd2)
    r2 = sc.accept(CFG, proposed_by="owner", authority_class="owner", state_dir=sd2)
    check("replay of live fleet config -> already_active",
          r1["decision"] == "accepted" and r2["decision"] == "already_active")
    shutil.rmtree(sd2, ignore_errors=True)


def main():
    print("== release matrix (offline demos, sol turn-5/6) ==")
    quick_proofs()
    scenario_10()
    print(f"\n{len(PASS)} passed, {len(FAIL)} failed")
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())
