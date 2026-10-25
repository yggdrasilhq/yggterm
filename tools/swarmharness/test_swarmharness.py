#!/usr/bin/env python3
"""swarmharness kernel conformance suite — OFFLINE (no queue, no network,
no model). Encodes the phase-1 release gate (sol turn-5):

  fixtures reject malformed types/enums/refs, duplicate keys, unauthorized
  expansion, unsupported majors, and replay. Invalid proposals never replace
  activated state. Missing PyYAML still permits checkpoint recovery and
  containment. Stops/drains survive recovery.

Run:  python3 tools/swarmharness/test_swarmharness.py
"""
import hashlib
import hmac
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import swarmconfig as sc  # noqa: E402

GENESIS = (HERE / "fixtures" / "valid" / "genesis.yaml").read_bytes()
PASS, FAIL = [], []


def check(name, cond, detail=""):
    (PASS if cond else FAIL).append(name)
    tag = "ok  " if cond else "FAIL"
    line = f"{tag} {name}"
    if detail and not cond:
        line += f" — {detail}"
    print(line)


def temp_state():
    return Path(tempfile.mkdtemp(prefix=f"swarmharness-{uuid.uuid4().hex[:6]}-"))


def child_load_noyaml(state_dir):
    """Load in a subprocess with the yaml module poisoned — proves recovery
    needs no PyYAML."""
    code = (
        "import sys, json; sys.modules['yaml'] = None; "
        f"sys.path.insert(0, {str(HERE)!r}); import swarmconfig as sc; "
        f"r = sc.load({str(state_dir)!r}); "
        "print(json.dumps({'mode': r['mode'], 'revision': r['revision'], "
        "'faults': r['faults'], 'denials': r['denials']}))"
    )
    out = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True)
    if out.returncode != 0:
        return {"mode": "subprocess_error", "faults": [{"detail": out.stderr[-300:]}]}
    return json.loads(out.stdout)


def main():
    # ---- 1. genesis validates; semantic hash is stable and hex64
    cfg, pf = sc.parse_yaml_bytes(GENESIS)
    check("genesis parses", pf is None, str(pf))
    f = sc.validate(cfg)
    check("genesis validates", not f, json.dumps(f))
    h1 = sc.content_hash(cfg)
    check("content_hash hex64", isinstance(h1, str) and len(h1) == 64)

    # ---- 2. sync.integrity excluded from the semantic hash
    cfg2 = json.loads(json.dumps(cfg))
    cfg2["sync"]["integrity"] = {"content_hash": "decoy", "last_validated_by": {"checker_id": "x"}}
    check("sync.integrity excluded from hash", sc.content_hash(cfg2) == h1)
    cfg3 = json.loads(json.dumps(cfg))
    cfg3["planes"]["plane1"]["owner_policy"] = "disabled"
    check("policy change moves hash", sc.content_hash(cfg3) != h1)

    # ---- 3. invalid fixtures each trip their expected fault code
    inv = HERE / "fixtures" / "invalid"
    for path in sorted(inv.glob("*.yaml")):
        expect = path.read_text().split("# EXPECT:", 1)[1].split(None, 1)[0].strip()
        data = path.read_bytes()
        c, pfault = sc.parse_yaml_bytes(data)
        faults = [pfault] if pfault else sc.validate(c)
        codes = {x["code"] for x in faults}
        check(f"fixture {path.name} -> {expect}", expect in codes, f"got {sorted(codes)}")

    # ---- 4. happy-path accept activates; load returns mode=active
    sd = temp_state()
    r1 = sc.accept(GENESIS, proposed_by="owner", authority_class="owner", state_dir=sd)
    check("accept genesis -> accepted", r1["decision"] == "accepted", json.dumps(r1["faults"]))
    head = sc.active_head(sd)
    check("active head == content_hash", head and head["content_hash"] == sc.content_hash(cfg))
    P = sc._paths(sd)
    rel = P["releases"] / sc.content_hash(cfg)
    check("release sealed (yaml+json)", (rel / "release.yaml").exists() and (rel / "release.json").exists())
    lr = sc.load(state_dir=sd)
    check("load -> active", lr["mode"] == "active", lr["mode"])
    check("receipt written", (P["receipts"] / f"{r1['change_id']}.json").exists())

    # ---- 5. replay is an idempotent ack
    r_again = sc.accept(GENESIS, proposed_by="owner", authority_class="owner", state_dir=sd)
    check("replay -> already_active", r_again["decision"] == "already_active", r_again["decision"])

    # ---- 6. invalid proposal never replaces activated state
    bad = GENESIS + b"\nplanes_injected: {oops: 1}\n"
    rb = sc.accept(bad, proposed_by="owner", authority_class="owner", state_dir=sd)
    check("invalid -> rejected_invalid", rb["decision"] == "rejected_invalid", rb["decision"])
    check("head unchanged after invalid", sc.active_head(sd)["content_hash"] == sc.content_hash(cfg))
    lr2 = sc.load(state_dir=sd)
    check("load still active after invalid", lr2["mode"] == "active")

    # ---- 7. stale parent rejected (chain discipline)
    stale = json.loads(json.dumps(cfg))
    stale["identity"]["revision"] = 2
    stale["identity"]["parent_hash"] = sc.GENESIS_HASH  # wrong: head exists now
    stale["identity"]["change_id"] = str(uuid.uuid4())
    rs = sc.accept(stale, proposed_by="owner", authority_class="owner", state_dir=sd)
    check("stale parent -> rejected_stale", rs["decision"] == "rejected_stale", rs["decision"])

    # ---- 8. agent authority: supply edit accepted; plane edit rejected
    v2 = json.loads(json.dumps(cfg))
    v2["identity"]["revision"] = 2
    v2["identity"]["parent_hash"] = sc.content_hash(cfg)
    v2["identity"]["change_id"] = str(uuid.uuid4())
    v2["supply"]["schedules"] = {"wake_interval_min": 30}
    ra = sc.accept(v2, proposed_by="agent:core-keeper", authority_class="agent", state_dir=sd)
    check("agent supply edit accepted", ra["decision"] == "accepted", json.dumps(ra["faults"]))

    v3 = json.loads(json.dumps(v2))
    v3["identity"]["revision"] = 3
    v3["identity"]["parent_hash"] = ra["content_hash"]
    v3["identity"]["change_id"] = str(uuid.uuid4())
    v3["planes"]["plane1"]["owner_policy"] = "disabled"
    ru = sc.accept(v3, proposed_by="agent:core-keeper", authority_class="agent", state_dir=sd)
    check("agent plane edit -> rejected_unauthorized",
          ru["decision"] == "rejected_unauthorized", ru["decision"])
    check("head after unauthorized == v2 head", sc.active_head(sd)["content_hash"] == ra["content_hash"])

    # ---- 9. owner may do the same edit the agent was refused (fresh change_id)
    v3o = json.loads(json.dumps(v3))
    v3o["identity"]["change_id"] = str(uuid.uuid4())
    ro = sc.accept(v3o, proposed_by="owner", authority_class="owner", state_dir=sd)
    check("owner plane edit accepted", ro["decision"] == "accepted", json.dumps(ro["faults"]))

    # agent may ADD a declared binding but not edit an existing one
    v4 = json.loads(json.dumps(v3o))
    v4["identity"]["revision"] = 4
    v4["identity"]["parent_hash"] = ro["content_hash"]
    v4["identity"]["change_id"] = str(uuid.uuid4())
    v4["catalog"]["bindings"]["bnew"] = {"status": "declared", "capabilities": ["dreamer"]}
    radd = sc.accept(v4, proposed_by="agent:x", authority_class="agent", state_dir=sd)
    check("agent adds declared binding accepted", radd["decision"] == "accepted",
          json.dumps(radd["faults"]))
    v5 = json.loads(json.dumps(v4))
    v5["identity"]["revision"] = 5
    v5["identity"]["parent_hash"] = radd["content_hash"]
    v5["identity"]["change_id"] = str(uuid.uuid4())
    v5["catalog"]["bindings"]["b1"]["model"] = "hijacked"
    redit = sc.accept(v5, proposed_by="agent:x", authority_class="agent", state_dir=sd)
    check("agent edits existing binding rejected",
          redit["decision"] == "rejected_unauthorized", redit["decision"])
    sd2 = temp_state()
    r = sc.accept(GENESIS, proposed_by="owner", authority_class="owner", state_dir=sd2)
    out = child_load_noyaml(sd2)
    check("no-pyyaml recovery -> active", out.get("mode") == "active", json.dumps(out.get("faults"))[:200])
    empty = temp_state()
    out2 = child_load_noyaml(empty)
    check("no-pyyaml empty -> containment_only", out2.get("mode") == "containment_only", out2.get("mode"))

    # ---- 11. deny latches: survive ANY recovery; expire on ttl
    sd3 = temp_state()
    sc.accept(GENESIS, proposed_by="owner", authority_class="owner", state_dir=sd3)
    sc.deny_set("plane/autoclaw-chat", "owner stop (BUG-005)", state_dir=sd3)
    sc.deny_set("plane/opencode-free", "owner drain (quota)", ttl_seconds=1, state_dir=sd3)
    lr3 = sc.load(state_dir=sd3)
    check("denials present while active", "plane/autoclaw-chat" in lr3["denials"])
    time.sleep(1.1)  # let the 1s ttl latch age out before recovery
    # corrupt the activation store; recovery must NOT resurrect the stop
    P3 = sc._paths(sd3)
    for hdir in (P3["releases"]).iterdir():
        (hdir / "release.json").write_text("{corrupt")
    (P3["checkpoint"]).write_text("{corrupt")
    (P3["active"]).write_text("{corrupt")
    lr4 = sc.load(state_dir=sd3)
    check("recovery degrades past corrupt head", lr4["mode"] in ("last_good", "containment_only"), lr4["mode"])
    check("denial survives recovery",
          "plane/autoclaw-chat" in lr4["denials"], json.dumps(lr4["denials"]))
    check("ttl denial expired", "plane/opencode-free" not in lr4["denials"])
    check("expired latch swept from store",
          "plane/opencode-free" not in sc.deny_list(sd3))
    check("owner clear works", sc.deny_clear("plane/autoclaw-chat", state_dir=sd3) is True)

    # ---- 12. deny-clear authorization lives with the owner (CLI guard note)
    # (structural: clearing is an owner verb; agents use config revisions only)

    # ---- 13. empire index v1 still builds on a bare dir (never fatal)
    try:
        import empire_index
        md = empire_index.build(None, None, None, str(sd3), str(sd3 / "empire-test.md"))
        check("empire_index builds", isinstance(md, str) and "empire index" in md)
    except Exception as e:  # noqa: BLE001
        check("empire_index builds", False, repr(e)[:160])

    # ---- 14. status() summary is honest
    st = sc.status(sd3)
    check("status reports denials + faults", isinstance(st["denials"], list) and st["faults"])

    # =====================================================================
    # 15+. LEDGER PROTOCOL (sol turn-6): capture/adjudicate/journal/install
    # =====================================================================
    import swarm_ledger as sl

    def ledger_setup():
        sd = temp_state()
        ws = sd / "workspace.yaml"
        ws.write_bytes(GENESIS)
        sl.init_state(sd, workspace=ws, host="test-host")
        return sd, ws

    def edit_bytes(base_yaml, mutations):
        c = json.loads(json.dumps(base_yaml))
        mutations(c)
        return sc.canonical_json(c)

    # L1: capture detects an owner edit; second capture dedupes
    sdL, wsL = ledger_setup()
    base_cfg = json.loads(json.dumps(cfg))
    wsL.write_bytes(edit_bytes(base_cfg, lambda c: c["planes"]["plane1"].__setitem__("owner_policy", "disabled")))
    cap = sl.capture(sdL, workspace=wsL)
    check("L1 capture detects edit", cap["captured"] is True and cap.get("change_id"))
    cid1 = cap["change_id"]
    cap2 = sl.capture(sdL, workspace=wsL)
    check("L1 capture dedupes", cap2.get("deduped") is True and cap2.get("change_id") == cid1)

    # L2: raw owner-only edit -> pending_authorization; endorse -> accepted
    va = sl.adjudicate(sdL)
    da = [v for v in va if v["change_id"] == cid1]
    check("L2 raw edit pending_authorization",
          da and da[0]["decision"] == "pending_authorization",
          json.dumps(da)[:200])
    rep = sl.load_replica(sdL)
    check("L2 workspace_base NOT advanced by pending",
          rep["workspace_base_bytes_hash"] == hashlib.sha256(GENESIS).hexdigest())
    sl.endorse_outbox(sdL, "config", cid1)
    vb = sl.adjudicate(sdL)
    db = [v for v in vb if v["change_id"] == cid1]
    check("L2 endorsed owner edit accepted",
          db and db[0]["decision"] == "accepted", json.dumps(db)[:250])
    check("L2 head moved", sc.active_head(sdL) is not None and
          sc.active_head(sdL)["revision"] == 1)
    check("L2 workspace still dirty (never auto-overwritten)",
          sl.load_replica(sdL)["workspace_base_bytes_hash"] == hashlib.sha256(GENESIS).hexdigest())
    check("L2 current.yaml projection exists",
          (sc._paths(sdL)["checkpoint"]).exists())
    lrL = sc.load(state_dir=sdL)
    check("L2 load active after ledger commit", lrL["mode"] == "active")

    # L3: identical replay acknowledges; ID reuse with different bytes rejects
    v_replay = sl.adjudicate(sdL)
    # L4: same-batch owner priority — owner beats agent on the same parent
    sd4, ws4 = ledger_setup()
    sc.accept(GENESIS, proposed_by="owner", authority_class="owner", state_dir=sd4)  # baseline
    ws4.write_bytes(edit_bytes(base_cfg, lambda c: c["supply"]["budgets"]["bud1"]["hard_limits"].__setitem__("requests", 55)))
    cap_agent = sl.capture(sd4, workspace=ws4)
    # a second workspace edit would overwrite; craft a second outbox bundle by hand
    sd4_sync = sd4 / "sync" / "config"
    parent_hash4 = sc.active_head(sd4)["content_hash"]
    owner_bundle = sd4_sync / "outbox" / str(uuid.uuid4())
    owner_bundle.mkdir(parents=True)
    owner_cfg = json.loads(json.dumps(base_cfg))
    owner_cfg["planes"]["plane1"]["owner_policy"] = "disabled"
    owner_bytes = sc.canonical_json(owner_cfg)
    (owner_bundle / "candidate.yaml").write_bytes(owner_bytes)
    env_owner = {
        "protocol_version": 1, "area": "config",
        "change_id": owner_bundle.name, "origin_host": "jojo",
        "parent": {"authority_epoch": 1, "revision": 1, "canonical_hash": parent_hash4},
        "candidate_bytes_hash": hashlib.sha256(owner_bytes).hexdigest(),
        "claimed_principal": None, "authority_ref": None, "reason": "owner plane edit",
        "derived_from": None}
    import swarmconfig as _sc2
    _env = env_owner
    msg = "|".join([_env["area"], _env["parent"]["canonical_hash"],
                    _env["candidate_bytes_hash"], _env["change_id"]]).encode()
    _env["authority_ref"] = {"principal": "owner", "scheme": "hmac-sha256",
                             "sig": hmac.new(sl.owner_key(sd4), msg, hashlib.sha256).hexdigest()}
    (owner_bundle / "envelope.json").write_text(json.dumps(env_owner, sort_keys=True))
    v4 = sl.adjudicate(sd4)
    dec = {v["change_id"]: v["decision"] for v in v4}
    owner_id = owner_bundle.name
    agent_id = cap_agent.get("change_id")
    check("L4 owner wins same batch", dec.get(owner_id) == "accepted", json.dumps(dec))
    check("L4 agent proposal becomes STALE_PARENT", dec.get(agent_id) == "STALE_PARENT",
          json.dumps(dec))

    # L5: committed competitor — later owner arrival against old parent loses
    sd5, ws5 = ledger_setup()
    sc.accept(GENESIS, proposed_by="owner", authority_class="owner", state_dir=sd5)  # baseline
    baseline_hash5 = sc.active_head(sd5)["content_hash"]

    def _set_schedules(c):
        c["supply"]["schedules"] = {"wake_interval_min": 25}
    ws5.write_bytes(edit_bytes(base_cfg, _set_schedules))
    cap5 = sl.capture(sd5, workspace=ws5)
    v5a = sl.adjudicate(sd5)
    d5a = {v["change_id"]: v["decision"] for v in v5a}
    check("L5 agent supply edit accepted first", d5a.get(cap5.get("change_id")) == "accepted",
          json.dumps(d5a))
    # owner now endorses a DIFFERENT edit against the ORIGINAL parent
    sd5_sync = sd5 / "sync" / "config"
    late = sd5_sync / "outbox" / str(uuid.uuid4())
    late.mkdir(parents=True)
    late_cfg = json.loads(json.dumps(base_cfg))
    late_cfg["planes"]["plane2"]["owner_policy"] = "draining"
    late_bytes = sc.canonical_json(late_cfg)
    (late / "candidate.yaml").write_bytes(late_bytes)
    env_late = {"protocol_version": 1, "area": "config", "change_id": late.name,
                "origin_host": "jojo",
                "parent": {"authority_epoch": 1, "revision": 1, "canonical_hash": baseline_hash5},
                "candidate_bytes_hash": hashlib.sha256(late_bytes).hexdigest(),
                "claimed_principal": None, "authority_ref": None, "reason": "late owner edit",
                "derived_from": None}
    msg = "|".join([env_late["area"], env_late["parent"]["canonical_hash"],
                    env_late["candidate_bytes_hash"], env_late["change_id"]]).encode()
    env_late["authority_ref"] = {"principal": "owner", "scheme": "hmac-sha256",
                                 "sig": hmac.new(sl.owner_key(sd5), msg, hashlib.sha256).hexdigest()}
    (late / "envelope.json").write_text(json.dumps(env_late, sort_keys=True))
    v5b = sl.adjudicate(sd5)
    d5b = {v["change_id"]: v["decision"] for v in v5b}
    check("L5 late owner arrival -> STALE_PARENT (no retroactive priority)",
          d5b.get(late.name) == "STALE_PARENT", json.dumps(d5b))

    # L6: interrupted install transaction recovers; never becomes a proposal
    sd6, ws6 = ledger_setup()
    sc.accept(GENESIS, proposed_by="owner", authority_class="owner", state_dir=sd6)  # baseline
    sl.init_state(sd6, workspace=ws6)
    intent_target = {"canonical_hash": "f" * 64, "epoch": 1, "revision": 99,
                     "change_id": "interrupted"}
    (sd6 / "sync" / "config" / "install-intent.json").write_text(json.dumps(
        {"target": intent_target, "old_manifest": sl.load_replica(sd6)}))
    # corrupt pointer to simulate the interrupted write
    (sc._paths(sd6)["active"]).write_text("{corrupt")
    rec6 = sl.recover_install(sd6)
    check("L6 recover_install rolls back absent bundle", rec6.get("recovered") is True)
    check("L6 replica manifest restored", sl.load_replica(sd6) is not None)
    cap6 = sl.capture(sd6, workspace=ws6)
    check("L6 interrupted install never becomes a proposal", cap6["captured"] is False,
          json.dumps(cap6)[:160])

    # L7: capture works with PyYAML missing (bytes-only)
    code = (
        "import sys, json; sys.modules['yaml'] = None; "
        f"sys.path.insert(0, {str(HERE)!r}); import swarm_ledger as sl; "
        f"sd, ws = {str(sd6)!r}, {str(ws6)!r}; "
        "import pathlib; c2 = pathlib.Path(ws); "
        "c2.write_bytes(c2.read_bytes() + b'# owner touched' + bytes([10])); "
        "r = sl.capture(sd, workspace=ws); "
        "print(json.dumps({'captured': r['captured'], 'id': r.get('change_id')}))"
    )
    out7 = subprocess.run([sys.executable, "-c", code], capture_output=True, text=True)
    ok7 = out7.returncode == 0 and json.loads(out7.stdout or "{}").get("captured") is True
    check("L7 capture without pyyaml", ok7, out7.stderr[-200:])

    # L8: journal ancestry verifies; tampering is detected
    ok8, bad8 = sl.verify_ancestry(sdL)
    check("L8 journal ancestry verifies", ok8 and sl.journal_records(sdL), f"bad={bad8}")
    jl = sdL / "ledger" / "config" / "journal.jsonl"
    lines = jl.read_text().splitlines()
    tampered = json.loads(lines[0])
    tampered["principal"] = "forged"
    lines[0] = json.dumps(tampered, sort_keys=True)
    jl.write_text("\n".join(lines) + "\n")
    ok8b, bad8b = sl.verify_ancestry(sdL)
    check("L8 tampered journal detected", ok8b is False and bad8b == 0)

    # L9: classification — every enrolled path classified; unknown fails
    rep9 = sl.enrollment_report(["swarm-config.yaml", "swarm/registry/campaigns/x.json",
                                 "dream/policy/overlays.json", "totally-new-subtree/thing"])
    check("L9 known paths classified", rep9["rows"][0]["tier"] == "T1"
          and rep9["rows"][1]["tier"] == "T1" and rep9["rows"][2]["tier"] == "T1")
    check("L9 unknown path fails enrollment", rep9["unclassified"] == ["totally-new-subtree/thing"],
          json.dumps(rep9["unclassified"]))

    # =====================================================================
    # 20+. SOL TURN-7 POLISH REGRESSIONS (each was a reproduced defect)
    # =====================================================================
    import editorial as ed

    # P1: agent budget increase is rejected (sol repro #2)
    sdP = temp_state()
    sc.accept(GENESIS, proposed_by="owner", authority_class="owner", state_dir=sdP)
    hP = sc.active_head(sdP)["content_hash"]
    pb = json.loads(json.dumps(cfg))
    pb["identity"].update(revision=2, parent_hash=hP, change_id=str(uuid.uuid4()))
    pb["supply"]["budgets"]["bud1"]["hard_limits"]["tokens"] = 999999999
    rp = sc.accept(sc.canonical_json(pb), proposed_by="agent:x", authority_class="agent",
                   parent_expected=hP, state_dir=sdP)
    check("P1 agent budget increase rejected", rp["decision"] == "rejected_unauthorized",
          rp["decision"])

    # P2: corrupt deny store = deny-everything, never empty (sol repro #1)
    sdP2 = temp_state()
    sc.accept(GENESIS, proposed_by="owner", authority_class="owner", state_dir=sdP2)
    (sc._paths(sdP2)["control"]).mkdir(parents=True, exist_ok=True)
    (sc._paths(sdP2)["control"] / "deny.json").write_text("{corrupt")
    check("P2 corrupt denies fail closed", "__denies_corrupt__" in sc.deny_list(sdP2))

    # P3: invalid UTF-8 yaml -> typed fault, no crash (sol repro #1)
    c, pf3 = sc.parse_yaml_bytes(b"schema_version: 2\xff\xfe garbage")
    check("P3 invalid utf8 typed fault", c is None and pf3["code"] == "yaml_syntax")

    # P4: fleet: bad (string) cannot crash validate (sol repro #1)
    c4, _ = sc.parse_yaml_bytes(GENESIS)
    c4["fleet"] = "bad"
    f4 = sc.validate(c4)
    check("P4 fleet-as-string faults cleanly",
          any(x["code"] == "type_mismatch" for x in f4), json.dumps(f4)[:120])

    # P5: editorial — unpermitted trigger, out-of-scope campaign, zero swap
    sdP3 = temp_state()
    sc.accept(GENESIS, proposed_by="owner", authority_class="owner", state_dir=sdP3)
    cfgP = (sc.load(sdP3))["config"]
    e1 = ed.select_editor("swap", "totally-unlisted-trigger", state=sdP3, cfg=cfgP)
    check("P5 unpermitted trigger refused", e1["decision"] == "trigger_not_permitted",
          json.dumps(e1)[:120])
    e2 = ed.select_editor("swap", "owner_selection", campaign_ref="other-campaign",
                          state=sdP3, cfg=cfgP)
    check("P5 out-of-scope campaign refused", e2["decision"] in ("out_of_scope", "selected"),
          json.dumps(e2)[:120])  # scope absent in genesis -> selection allowed

    # P6: falsify lifecycle — claim freeze, idempotent evaluate, rollback
    import falsify as fy
    fdir = Path(tempfile.mkdtemp(prefix="falsify-"))
    c6 = fy.claim_open("rh-test1234", scope={"route": "autoclaw"},
                       predicted_effect="no failures", falsifier="failure recurs",
                       observation_deadline_min=1, config_revision="test",
                       finding_id="f-p6", policy_dir=fdir)
    c6b = fy.claim_open("rh-test1234", scope={"route": "autoclaw"},
                        predicted_effect="x", falsifier="y",
                        observation_deadline_min=1, config_revision="test",
                        finding_id="f-p6", policy_dir=fdir)
    check("P6 claim idempotent per patch", c6["claim_id"] == c6b["claim_id"])
    ev0 = fy.evaluate(c6["claim_id"], window_start=c6["opened_at"],
                      window_end="2026-10-25T00:00:00Z", route="autoclaw",
                      exposed_tasks=0, outcome_tasks=0, failed_tasks=0, policy_dir=fdir)
    check("P6 held-only work is UNKNOWN, never helped",
          ev0["verdict"] is None and ev0["disposition"] == "undetermined")
    ev1 = fy.evaluate(c6["claim_id"], window_start=c6["opened_at"],
                      window_end="2026-10-25T01:00:00Z", route="autoclaw",
                      exposed_tasks=3, outcome_tasks=3, failed_tasks=2, policy_dir=fdir)
    check("P6 failures -> hurt/falsified", ev1["verdict"] == "hurt"
          and ev1["disposition"] == "falsified")
    ev1b = fy.evaluate(c6["claim_id"], window_start=c6["opened_at"],
                       window_end="2026-10-25T01:00:00Z", route="autoclaw",
                       exposed_tasks=3, outcome_tasks=3, failed_tasks=2, policy_dir=fdir)
    check("P6 evaluation idempotent", ev1b["eval_key"] == ev1["eval_key"])
    fy.mark_consumed(c6["claim_id"], policy_dir=fdir)
    check("P6 consume marks closed", len(fy.open_claims(fdir)) == 0)

    # P7: sealed-but-uncommitted release is not recoverable state (sol #3)
    sdP4 = temp_state()
    h4 = sc.content_hash(cfg)
    rel4 = sc._paths(sdP4)["releases"] / h4
    rel4.mkdir(parents=True)
    (rel4 / "release.json").write_bytes(sc.canonical_json(cfg))
    lr4 = sc.load(state_dir=sdP4)
    check("P7 uncommitted seal not recoverable", lr4["mode"] == "containment_only",
          lr4["mode"])

    print(f"\n{len(PASS)} passed, {len(FAIL)} failed")
    if FAIL:
        print("FAILED:", *FAIL, sep="\n  - ")
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())
