"""editorial — editor hot-swap runtime (sol turn-5 §4). Resolves editor
scenarios from the ACTIVATED config, writes authenticated EditorSelection
receipts, and maintains the monotonic editorial epoch.

Rules encoded:
  - a binding marked `declared` can never be activated (requires_admitted_target);
  - no eligible editor -> retain-work-and-defer-editorial-decisions (containment);
  - max_swaps_per_incident is enforced against the incident counter;
  - the handover packet is a bounded evidence snapshot (registry campaign +
    intent pointer), never credentials and never hidden conversation state.
"""
import json
from pathlib import Path

import swarmconfig as sc

def _dir(state=sc.DEFAULT_STATE):
    return Path(state).expanduser() / "editorial"


def _epoch_path(state):
    return Path(state).expanduser() / "control" / "editorial-epoch"


def current_epoch(state=sc.DEFAULT_STATE):
    try:
        return int(_epoch_path(state).read_text().strip())
    except Exception:
        return 0


def current_editor(state=sc.DEFAULT_STATE, cfg=None):
    """Active selection receipt, or the config default."""
    p = _dir(state) / "current-editor.json"
    try:
        rec = json.loads(p.read_text())
        if rec.get("expires_at") and rec["expires_at"] <= sc._now():
            return None
        return rec
    except Exception:
        return None


def select_editor(scenario_id, trigger, campaign_ref=None, authority_ref="owner",
                  state=sc.DEFAULT_STATE, cfg=None, incident_id=None,
                  evidence_refs=None):
    """Resolve an editor scenario and record the selection receipt. Returns
    {decision: selected|deferred, ...receipt} — a declared target is never
    activated; with no eligible editor the decision is deferred."""
    if cfg is None:
        cfg = (sc.load(state) or {}).get("config") or {}
    scenarios = cfg.get("editor_scenarios") or {}
    bindings = ((cfg.get("catalog") or {}).get("bindings")) or {}
    scen = scenarios.get(scenario_id)
    if scen is None:
        return {"decision": "no_such_scenario", "scenario": scenario_id}
    # TRIGGER + SCOPE enforcement (sol turn-7 #8): an unlisted trigger or an
    # out-of-scope campaign can never drive a swap
    permitted = scen.get("permitted_trigger_classes") or []
    if permitted and trigger not in permitted:
        return {"decision": "trigger_not_permitted", "scenario": scenario_id,
                "trigger": trigger, "permitted": permitted}
    scope = scen.get("scope") or {}
    camp_refs = scope.get("campaign_refs") or []
    # a SCOPED scenario cannot be exercised without naming the campaign —
    # an omitted campaign_ref must refuse, not silently pass (sol t9#8)
    if camp_refs and not campaign_ref:
        return {"decision": "out_of_scope", "scenario": scenario_id,
                "detail": "scenario declares campaign_refs; campaign_ref required"}
    if campaign_ref and camp_refs and campaign_ref not in camp_refs:
        return {"decision": "out_of_scope", "scenario": scenario_id,
                "campaign_ref": campaign_ref, "scope": camp_refs}
    act = scen.get("activation") or {}
    order = scen.get("binding_order") or []
    reasons = []
    chosen = None
    for ref in order:
        b = bindings.get(ref) or {}
        # UNCONDITIONAL admission check: a declared binding is never
        # activatable, whatever the scenario's activation block says
        if b.get("status") != "admitted":
            reasons.append(f"{ref}: {b.get('status') or 'unresolved'} (not admitted) — skipped")
            continue
        chosen = ref
        break
    if chosen is None:
        return {"decision": "deferred", "scenario": scenario_id,
                "policy": scen.get("on_no_eligible_editor"),
                "reasons": reasons}

    # a 0 allowance means NO swaps, ever — refuse before any accounting
    limit0 = act.get("max_swaps_per_incident")
    if limit0 is not None and int(limit0) <= 0:
        return {"decision": "swap_budget_exhausted", "scenario": scenario_id,
                "swaps_so_far": 0, "limit": 0}
    # swap budget per incident
    if incident_id:
        inc_dir = _dir(state) / "incidents"
        try:
            swaps = len(json.loads((inc_dir / f"{incident_id}.json").read_text()).get("swaps", []))
        except Exception:
            swaps = 0
        limit = act.get("max_swaps_per_incident")
        limit = 1 if limit is None else int(limit)
        if swaps >= limit:  # a 0 allowance is honored, not defaulted
            return {"decision": "swap_budget_exhausted", "scenario": scenario_id,
                    "swaps_so_far": swaps}

    _epoch_path(state).parent.mkdir(parents=True, exist_ok=True)
    import fcntl as _fcntl
    _lk = open(_epoch_path(state).with_suffix(".lock"), "a+")
    _fcntl.flock(_lk, _fcntl.LOCK_EX)
    try:
        epoch = current_epoch(state) + 1
        sc.atomic_write(_epoch_path(state), str(epoch).encode())
        # selection publication + incident accounting share the epoch lock so
        # a race cannot mint two selections at one epoch (sol t9#8)
        sel = {
            "scenario": scenario_id, "trigger": trigger, "campaign_ref": campaign_ref,
            "selected_binding_ref": chosen, "config_revision": (sc.active_head(state) or {}).get("content_hash"),
            "editorial_epoch": epoch, "evidence_refs": evidence_refs or [],
            "authority_ref": authority_ref, "ttl_seconds": act.get("ttl_seconds"),
            "expires_at": None, "at": sc._now(), "skipped": reasons,
        }
        if act.get("ttl_seconds"):
            from datetime import datetime, timedelta, timezone
            sel["expires_at"] = (datetime.now(timezone.utc)
                                 + timedelta(seconds=int(act["ttl_seconds"]))
                                 ).strftime("%Y-%m-%dT%H:%M:%SZ")
        d = _dir(state)
        d.mkdir(parents=True, exist_ok=True)
        sc.atomic_write_json(d / "current-editor.json", sel)
        with open(d / "selections.jsonl", "a") as f:
            f.write(json.dumps(sel, sort_keys=True) + "\n")
            f.flush()
            import os as _os
            _os.fsync(f.fileno())
        if incident_id:
            inc_dir = d / "incidents"
            inc_dir.mkdir(parents=True, exist_ok=True)
            try:
                rec = json.loads((inc_dir / f"{incident_id}.json").read_text())
            except Exception:
                rec = {"swaps": []}
            rec["swaps"].append({"at": sel["at"], "to": chosen, "epoch": epoch})
            sc.atomic_write_json(inc_dir / f"{incident_id}.json", rec)
    finally:
        _fcntl.flock(_lk, _fcntl.LOCK_UN)
        _lk.close()
    sel = {
        "scenario": scenario_id, "trigger": trigger, "campaign_ref": campaign_ref,
        "selected_binding_ref": chosen, "config_revision": (sc.active_head(state) or {}).get("content_hash"),
        "editorial_epoch": epoch, "evidence_refs": evidence_refs or [],
        "authority_ref": authority_ref, "ttl_seconds": act.get("ttl_seconds"),
        "expires_at": None, "at": sc._now(), "skipped": reasons,
    }
    if act.get("ttl_seconds"):
        from datetime import datetime, timedelta, timezone
        sel["expires_at"] = (datetime.now(timezone.utc)
                             + timedelta(seconds=int(act["ttl_seconds"]))
                             ).strftime("%Y-%m-%dT%H:%M:%SZ")
    return {"decision": "selected", **sel}


def handover_packet(campaign_id, state=sc.DEFAULT_STATE, registry_dir=None,
                    max_bytes=4096):
    """Bounded handover evidence: objective, commitments, blockers, open
    procedures for the campaign, plus the intent pointer. Bytes-bounded."""
    reg = Path(registry_dir or "~/.yggterm/swarm/registry").expanduser()
    packet = {"campaign": campaign_id, "at": sc._now()}
    try:
        c = json.loads((reg / "campaign" / f"{campaign_id}.json").read_text())
        packet["objective"] = c.get("objective")
        packet["commitments"] = [
            {k: x.get(k) for k in ("id", "deliverable", "status")}
            for x in (c.get("commitments") or [])[:8]]
        packet["blockers"] = [
            {k: x.get(k) for k in ("id", "class", "description")}
            for x in (c.get("blockers") or [])[:8]]
        packet["procedure_cues"] = (c.get("procedure_cues") or [])[:8]
    except Exception as e:  # noqa: BLE001
        packet["campaign_error"] = repr(e)[:120]
    cfg = (sc.load(state) or {}).get("config") or {}
    packet["intent"] = cfg.get("intent")
    out = json.dumps(packet, indent=2, sort_keys=True)
    if len(out.encode()) > max_bytes:
        out = out[:max_bytes] + "\n...[truncated]"
    p = _dir(state) / "handover"
    p.mkdir(parents=True, exist_ok=True)
    (p / f"epoch-{current_epoch(state)}.json").write_text(out + "\n")
    return packet
