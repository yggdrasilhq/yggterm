"""legacy_supply — the ONE-WAY floors.json generator (phase 5, sol turn-5 §6
adapted by turn-6: proposal capture must NOT make shadow v2 a second
production supply authority; floors.json remains authoritative until the
owner flips the cutover).

Direction of authority:
  pre-cutover:  floors.json -> production (v2 is a shadow candidate)
  post-cutover: swarm-config.yaml -> production; render_floors() generates a
                compatibility floors.json tagged with the config revision;
                independent edits to the generated file are REJECTED by
                compare (revision tag mismatch = tampered).

Every authoritative floors.json field must be mapped here or listed in
RETIRED_FIELDS — none may be silently dropped.
"""
import json
from pathlib import Path

import swarmconfig as sc

RETIRED_FIELDS = {
    # floors.json key -> where it went in v2 (or why retired)
    "namespaces[].ns": "supply.namespaces_ref (ledgered; phase-5 import maps per-namespace floors)",
    "namespaces[].per_day": "supply.budgets + admission (v2 expresses limits as typed budgets)",
    "peak": "supply.schedules.peak_window (timezone-aware window string)",
    "daymap": "validator-internal (parse_peak); schedule strings are the authority",
}

_CUTOVER_FLAG = Path("~/.yggterm/swarm/control/supply-cutover.json")


def cutover_state():
    try:
        return json.loads(_CUTOVER_FLAG.expanduser().read_text())
    except Exception:
        return {"cutover": False,
                "authority": "floors.json (production) | swarm-config v2 (shadow)"}


def request_cutover(actor, state=sc.DEFAULT_STATE):
    """Explicit authority flip — an OWNER action recorded durably. Never
    performed automatically by proposal capture or the maintenance service."""
    rec = {"cutover": True, "authority": "swarm-config.yaml (production) "
           "| floors.json (generated compatibility artifact)",
           "at": sc._now(), "actor": actor,
           "config_revision": (sc.active_head(state) or {}).get("content_hash")}
    sc.atomic_write_json(_CUTOVER_FLAG.expanduser(), rec)
    return rec


def render_floors(cfg_sw):
    """-> floors-shaped dict projected from the v2 config, tagged with its
    revision. Deterministic; the generated file is a compatibility artifact,
    never independently editable post-cutover."""
    supply = (cfg_sw or {}).get("supply") or {}
    schedules = supply.get("schedules") or {}
    admission = supply.get("admission") or {}
    ident = (cfg_sw or {}).get("identity") or {}
    return {
        "_generated_by": "swarmharness/legacy_supply.render_floors",
        "_config_revision": {"epoch": ident.get("authority_epoch"),
                             "revision": ident.get("revision"),
                             "canonical_hash": ident.get("parent_hash")},
        "namespaces": [],
        "peak": schedules.get("peak_window"),
        "admission": admission,
        "namespaces_ref": supply.get("namespaces_ref"),
        "note": "post-cutover compatibility artifact; edit swarm-config.yaml instead",
    }


def verify_generated(floors_bytes, cfg_sw):
    """Reject independent edits to the generated artifact: its revision tag
    must match the ACTIVE config revision."""
    try:
        f = json.loads(floors_bytes)
    except Exception:
        return {"ok": False, "reason": "unreadable"}
    active = sc.active_head()
    tag = f.get("_config_revision") or {}
    if not active:
        return {"ok": True, "note": "no active head; nothing to compare"}
    if tag.get("canonical_hash") != active.get("content_hash"):
        return {"ok": False,
                "reason": f"generated floors tagged {str(tag.get('canonical_hash'))[:12]} "
                          f"but active config is {active.get('content_hash')[:12]} — "
                          f"independent edit detected; regenerate from swarm-config"}
    return {"ok": True}
