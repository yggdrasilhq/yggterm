"""swarm-config v2 — the swarm survival ledger: schema, strict validator,
activation store, structured load result, deny latches.

v2 replaces the v1 loader wholesale (turn-4 defects retired: nested tuple
returns, mutable-source last-good copy, shallow validation). One file is the
authority: ~/.yggterm/swarm-config.yaml. Activation is a transaction:

    candidate bytes -> strict validate -> seal immutable release
      -> CAS active.json -> checkpoint.json (PyYAML-free recovery)

Owner stops/drains live in deny latches (swarm/control/deny.json), stored
independently of the config so recovery can never resurrect them.

Public API (contract for every later phase):
    load(...)   -> LoadResult dict {mode, config, revision, source, faults, denials}
    accept(...) -> receipt dict (CAS against parent hash; invalid/stale/
                   unauthorized proposals never replace activated state)
    deny_set / deny_clear / deny_list
    status(...)
"""
import sys
import hashlib
import json
import os
import re
import shutil
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path

try:
    import yaml
except ImportError:  # kernel portability: pyyaml is the only non-stdlib dep
    yaml = None

DEFAULT_YAML = Path("~/.yggterm/swarm-config.yaml")
LEGACY_YAML = Path("~/.yggterm/swarmconfig.yaml")
DEFAULT_STATE = Path("~/.yggterm/swarm")
MAX_BYTES = 256 * 1024
KNOWN_MAJOR = 3
SUPPORTED_MAJORS = (2, 3)
GENESIS_HASH = "0" * 64
HEX64 = re.compile(r"^[0-9a-f]{64}$")

# semantic-diff authority: proposals from non-owner principals may not touch
# these POLICY paths (prefix match over the policy view — the identity
# envelope is excluded: parent/revision/change_id are re-stamped by accept).
# Owner proposals may change anything.
OWNER_ONLY_PREFIXES = (
    "introspection",          # source/metadata enrollment is owner policy
    "fleet",
    "kernel",
    "maintenance",
    "planes",
    "roles",
    "sync",
    "catalog.harnesses",
    "catalog.providers",
    "catalog.plans",
    "catalog.accounts",
    "supply.budgets",          # budgets/reserves: never agent-writable (sol t7)
    "supply.maintenance_reserve_ref",
    "campaign_policies",       # authorization + allocation envelopes: owner/delegate only
    "editor_scenarios",        # swap triggers/authority: owner policy (sol t9#8)
    "catalog.execution_profiles",   # profiles: owner doctrine (sol pressure D1)
    "supply.pressure.ceilings",     # hard ceilings: owner only, ever (D5)
    "supply.pressure.levels",       # level tables: owner doctrine (D3)
    "supply.pressure.control",      # delegation boundary itself: owner (D5)
    "roles.*.routing",              # chains/lanes: owner doctrine (D4)
)


def _policy_view(cfg):
    return {k: v for k, v in cfg.items() if k != "identity"}


def _agent_unauthorized(old, new, *, issuer_role_ref=None):
    """-> sorted list of policy paths an agent-class proposal may not change.
    Bindings: agents may ADD declared bindings only — edits to existing
    bindings, admissions, and removals are owner decisions."""
    bad = [p for p in semantic_diff_paths(_policy_view(old), _policy_view(new))
           if p.startswith(OWNER_ONLY_PREFIXES)]
    ob = (old.get("catalog") or {}).get("bindings") or {}
    nb = (new.get("catalog") or {}).get("bindings") or {}
    for k in set(ob) & set(nb):
        if ob[k] != nb[k]:
            bad.append(f"catalog.bindings.{k} (existing-binding edit)")
    for k in set(nb) - set(ob):
        if (nb[k] or {}).get("status") != "declared":
            bad.append(f"catalog.bindings.{k} (non-declared addition)")
    for k in set(ob) - set(nb):
        bad.append(f"catalog.bindings.{k} (removal)")
    # pressure demand: agents tune only within the control boundary
    # (sol pressure design D5); anything else under supply.pressure is
    # owner-only via OWNER_ONLY_PREFIXES
    odem = ((old.get("supply") or {}).get("pressure") or {}).get("demand") or {}
    ndem = ((new.get("supply") or {}).get("pressure") or {}).get("demand") or {}
    if odem != ndem:
        ctrl = ((old.get("supply") or {}).get("pressure") or {}).get("control") or {}
        if not ctrl.get("persistent_agent_edits"):
            bad.append("supply.pressure.demand (persistent_agent_edits disabled)")
        elif issuer_role_ref is None or issuer_role_ref not in (
                ctrl.get("agent_role_refs") or []):
            bad.append(f"supply.pressure.demand (issuer role "
                       f"{issuer_role_ref!r} not delegated)")
        else:
            if ndem.get("level") not in (ctrl.get("agent_level_names") or []):
                bad.append(f"supply.pressure.demand.level -> "
                           f"{ndem.get('level')!r} (not delegated)")
            old_over = odem.get("profile_slot_overrides") or {}
            new_over = ndem.get("profile_slot_overrides") or {}
            for pid in set(old_over) | set(new_over):
                if old_over.get(pid) != new_over.get(pid) and pid not in (
                        ctrl.get("agent_profile_refs") or []):
                    bad.append("supply.pressure.demand.profile_slot_overrides"
                               f" -> {pid} (not delegated)")
            # effective-baseline: a level change must not raise any
            # undelegated profile's resolved target
            try:
                ov_old = pressure_view(old).get("baseline") or {}
                ov_new = pressure_view(new).get("baseline") or {}
                aliases = pressure_view(old).get("profile_aliases") or {}
                aliases.update(pressure_view(new).get("profile_aliases") or {})

                def _canon(pid):
                    return aliases.get(pid, pid)

                _delegated = set(ctrl.get("agent_profile_refs") or [])
                _raised = {}
                for pid in set(ov_old) | set(ov_new):
                    canon = _canon(pid)
                    if canon in _delegated:
                        continue
                    if ov_new.get(pid, 0) > ov_old.get(pid, 0):
                        _raised[pid] = (ov_old.get(pid), ov_new.get(pid))
                if _raised:
                    for pid, (a, b) in sorted(_raised.items()):
                        bad.append(f"supply.pressure.demand raises "
                                   f"undelegated profile {pid} ({a} -> {b})")
            except Exception:
                bad.append("supply.pressure.demand (normalization failed)")
    return sorted(set(bad))

FAULT_CODES = (
    "yaml_syntax", "duplicate_key", "size_bound", "unsupported_major",
    "unknown_field", "type_mismatch", "invalid_enum", "ref_missing",
    "invariant_violation", "not_a_mapping", "checkpoint_corrupt",
    "no_usable_config", "parser_missing",
    "PRESSURE_SCHEMA", "PRESSURE_REF", "PRESSURE_PROFILE",
    "PRESSURE_BOUNDS", "PRESSURE_SOURCE_CONFLICT", "PRESSURE_ROUTE_CONFLICT",
    "PRESSURE_DELEGATION", "PRESSURE_BELOW_INFLIGHT",
)

PLANE_POLICIES = ("enabled", "draining", "disabled")
BINDING_STATUS = ("declared", "admitted")
ROLE_KINDS = ("deterministic", "model-assisted-maintenance", "editorial",
              "proposal-and-review", "execution")


# ---------------------------------------------------------------- utilities

def _now():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def canonical_json(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"),
                      ensure_ascii=False).encode("utf-8")


def content_hash(cfg):
    """Semantic hash: full config minus the machine-generated sync.integrity
    subtree (display metadata, never part of identity)."""
    body = {k: v for k, v in cfg.items() if k != "sync"}
    sync = dict(cfg.get("sync") or {})
    sync.pop("integrity", None)
    if sync:
        body["sync"] = sync
    return hashlib.sha256(canonical_json(body)).hexdigest()


def atomic_write(path: Path, data: bytes):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + f".tmp-{os.getpid()}-{uuid.uuid4().hex[:8]}")
    with open(tmp, "wb") as f:
        f.write(data)
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)
    fd = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def atomic_write_json(path: Path, obj):
    atomic_write(path, canonical_json(obj) + b"\n")


def _flatten(prefix, obj, out):
    if isinstance(obj, dict):
        for k, v in obj.items():
            _flatten(f"{prefix}.{k}" if prefix else str(k), v, out)
    else:
        out[prefix] = obj


def semantic_diff_paths(a, b):
    fa, fb = {}, {}
    _flatten("", a, fa)
    _flatten("", b, fb)
    changed = {k for k in set(fa) | set(fb) if fa.get(k) != fb.get(k)}
    # a moved subtree shows as changed leaves; keep only leaf paths
    return {p for p in changed if not any(q != p and q.startswith(p + ".")
                                          for q in changed)}


# ------------------------------------------------------------ yaml parsing

if yaml is not None:  # module must import (and recover) without pyyaml

    class _DupCheckLoader(yaml.SafeLoader):
        pass

    def _no_dup_keys(loader, node, deep=False):
        seen = {}
        for kn, _vn in node.value:
            k = loader.construct_object(kn, deep=deep)
            if k in seen:
                raise yaml.YAMLError(f"duplicate key: {k}")
            seen[k] = True
        return yaml.SafeLoader.construct_mapping(loader, node, deep)

    _DupCheckLoader.add_constructor(
        yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG,
        lambda l, n: _no_dup_keys(l, n))


def parse_yaml_bytes(data: bytes):
    """-> (cfg_dict, None) or (None, fault_dict). Strict: duplicate keys and
    YAML tags are faults, not crashes."""
    if yaml is None:
        return None, {"code": "parser_missing",
                      "detail": "pyyaml unavailable; recovery uses checkpoint.json"}
    if len(data) > MAX_BYTES:
        return None, {"code": "size_bound",
                      "detail": f"{len(data)} bytes exceeds {MAX_BYTES}"}
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError as e:
        return None, {"code": "yaml_syntax", "detail": f"invalid UTF-8: {e}"}
    try:
        cfg = yaml.load(text, Loader=_DupCheckLoader)
    except yaml.YAMLError as e:
        msg = str(e)
        code = "duplicate_key" if "duplicate key" in msg else "yaml_syntax"
        return None, {"code": code, "detail": msg[:200]}
    if not isinstance(cfg, dict):
        return None, {"code": "not_a_mapping", "detail": type(cfg).__name__}
    return cfg, None


# -------------------------------------------------------------- validation

_TOP_KEYS = {
    "schema_version": int, "identity": dict, "fleet": dict, "kernel": dict,
    "maintenance": dict, "catalog": dict, "planes": dict, "roles": dict,
    "editor_scenarios": dict, "campaign_policies": dict, "supply": dict,
    "intent": dict, "sync": dict,
    "introspection": dict,
}

_IDENTITY_KEYS = {"config_id": str, "authority_epoch": int, "revision": int,
                  "parent_hash": str, "change_id": str, "proposed_by": str,
                  "reason": str}

_SUPervisor_KEYS = {"harness": str, "model": object, "controller_host": str,
                    "replica_hosts": list, "poll_seconds": int,
                    "dependency_timeout_seconds": int,
                    "restart_limit": dict, "backoff_seconds": list}

_HARNESS_KINDS = ("dispatch-queue", "consult-cli", "desktop-chat",
                  "dreamer-cli", "none")


def _fault(faults, code, detail):
    faults.append({"code": code, "detail": detail[:220]})


def _typecheck(faults, path, val, spec):
    if spec is object:
        return
    if spec is float:
        if not isinstance(val, (int, float)) or isinstance(val, bool):
            _fault(faults, "type_mismatch", f"{path}: expected number, got {type(val).__name__}")
        return
    if spec is str:
        if not isinstance(val, str) or not val.strip():
            _fault(faults, "type_mismatch", f"{path}: expected non-empty string, got {type(val).__name__}")
        return
    if isinstance(spec, tuple):  # enum
        if val not in spec:
            _fault(faults, "invalid_enum", f"{path}: {val!r} not in {list(spec)}")
        return
    if isinstance(spec, type):
        if not isinstance(val, spec) or (spec in (int, str, list, dict) and isinstance(val, bool) and spec is int):
            _fault(faults, "type_mismatch", f"{path}: expected {spec.__name__}, got {type(val).__name__}")
        return
    _fault(faults, "unknown_field", f"{path}: schema spec missing for {spec!r}")


def _keys(faults, path, val, allowed, required=()):
    for k in val:
        if k not in allowed:
            _fault(faults, "unknown_field", f"{path}.{k}")
    for k in required:
        if k not in val:
            _fault(faults, "unknown_field", f"{path}: missing required key {k}")


def validate(cfg):
    """-> list of fault dicts. Empty list = valid. Pure-dict level so the same
    check gates YAML candidates and JSON checkpoint recovery alike."""
    f: list = []
    if not isinstance(cfg, dict):
        return [{"code": "not_a_mapping", "detail": type(cfg).__name__}]

    v = cfg.get("schema_version")
    if not isinstance(v, int) or isinstance(v, bool):
        _fault(f, "type_mismatch", f"schema_version: {v!r}")
        return f
    if v not in SUPPORTED_MAJORS:
        _fault(f, "unsupported_major",
               f"schema_version {v} not supported (supported: {list(SUPPORTED_MAJORS)}); update the kernel first")
        return f

    for k in cfg:
        if k not in _TOP_KEYS:
            _fault(f, "unknown_field", f"top-level .{k}")
    for sec, spec in _TOP_KEYS.items():
        if sec in cfg and spec is dict and not isinstance(cfg[sec], dict):
            _fault(f, "type_mismatch", f"{sec}: expected mapping, got {type(cfg[sec]).__name__}")

    ident = cfg.get("identity") or {}
    if not isinstance(ident, dict):
        _fault(f, "type_mismatch", f"identity: expected mapping, got {type(ident).__name__}")
        ident = {}
    _keys(f, "identity", ident, _IDENTITY_KEYS, _IDENTITY_KEYS)
    for k, spec in _IDENTITY_KEYS.items():
        if k in ident:
            _typecheck(f, f"identity.{k}", ident[k], spec)
    if isinstance(ident.get("authority_epoch"), int) and ident["authority_epoch"] < 1:
        _fault(f, "invariant_violation", "identity.authority_epoch must be >= 1")
    if isinstance(ident.get("revision"), int) and ident["revision"] < 1:
        _fault(f, "invariant_violation", "identity.revision must be >= 1")
    ph = ident.get("parent_hash")
    if ph is not None and not (isinstance(ph, str) and HEX64.fullmatch(ph)):
        _fault(f, "invariant_violation", "identity.parent_hash must be 64 hex chars")
    try:
        uuid.UUID(str(ident.get("change_id")))
    except (ValueError, TypeError, AttributeError):
        _fault(f, "invariant_violation", "identity.change_id must be a UUID")

    fleet = cfg.get("fleet") or {}
    if not isinstance(fleet, dict):
        _fault(f, "type_mismatch", f"fleet: expected mapping, got {type(fleet).__name__}")
        fleet = {}
    _keys(f, "fleet", fleet, {"authority_host", "hosts", "dispatch_host", "failover"})
    hosts = fleet.get("hosts")
    if not isinstance(hosts, list):
        hosts = []  # fault already recorded; keep later checks type-safe
    if not isinstance(hosts, list) or not hosts or not all(isinstance(h, str) and h for h in hosts):
        _fault(f, "type_mismatch", "fleet.hosts: non-empty string list")
    if len(set(hosts or [])) != len(hosts or []):
        _fault(f, "invariant_violation", "fleet.hosts: duplicates")
    if fleet.get("dispatch_host") != fleet.get("authority_host"):
        _fault(f, "invariant_violation",
               "one dispatch authority: dispatch_host must equal authority_host")
    for hk in ("authority_host", "dispatch_host"):
        if fleet.get(hk) and hosts and fleet[hk] not in hosts:
            _fault(f, "ref_missing", f"fleet.{hk}={fleet[hk]!r} not in fleet.hosts")

    # --- maintenance: the deterministic boundary
    maint = cfg.get("maintenance") or {}
    _keys(f, "maintenance", maint,
          {"service", "executor_service", "supervisor", "repair_role_ref",
           "permitted_actions", "prohibited_actions"})
    sup = maint.get("supervisor") or {}
    _keys(f, "maintenance.supervisor", sup, _SUPervisor_KEYS,
          {"harness", "model", "controller_host", "poll_seconds"})
    for k, spec in _SUPervisor_KEYS.items():
        if k in sup:
            _typecheck(f, f"maintenance.supervisor.{k}", sup[k], spec)
    if "harness" in sup and sup["harness"] != "deterministic-python":
        _fault(f, "invariant_violation",
               "maintenance.supervisor.harness must be deterministic-python")
    if sup.get("model") is not None:
        _fault(f, "invariant_violation",
               "deterministic supervisor must declare model: null (no model dependency)")
    rl = sup.get("restart_limit") or {}
    _keys(f, "maintenance.supervisor.restart_limit", rl, {"attempts", "window_seconds"})
    for k in ("attempts", "window_seconds"):
        if k in rl and (not isinstance(rl[k], int) or isinstance(rl[k], bool) or rl[k] < 1):
            _fault(f, "invariant_violation", f"restart_limit.{k} must be int >= 1")
    for i, b in enumerate(sup.get("backoff_seconds") or []):
        if not isinstance(b, int) or isinstance(b, bool) or b < 1:
            _fault(f, "invariant_violation", f"backoff_seconds[{i}] must be int >= 1")
    if sup.get("poll_seconds") is not None and (not isinstance(sup["poll_seconds"], int) or sup["poll_seconds"] < 5):
        _fault(f, "invariant_violation", "poll_seconds must be int >= 5")

    # --- catalog: harnesses/providers/accounts/plans/bindings
    cat = cfg.get("catalog") or {}
    if not isinstance(cat, dict):
        _fault(f, "type_mismatch", f"catalog: expected mapping, got {type(cat).__name__}")
        cat = {}
    _keys(f, "catalog", cat, {"harnesses", "providers", "accounts", "plans",
                              "bindings", "execution_profiles"})
    if cat.get("execution_profiles") is not None and v != 3:
        _fault(f, "unknown_field",
               "catalog.execution_profiles requires schema_version 3")
    cat = dict(cat)  # never repair the caller's input while rejecting it
    for space in ("harnesses", "providers", "accounts", "plans", "bindings"):
        if cat.get(space) is not None and not isinstance(cat[space], dict):
            _fault(f, "type_mismatch", f"catalog.{space}: expected mapping")
            cat[space] = {}
    harnesses = cat.get("harnesses") or {}
    for hid, h in harnesses.items():
        _keys(f, f"catalog.harnesses.{hid}", h, {"kind", "adapter_ref"}, {"kind"})
        if h.get("kind") not in _HARNESS_KINDS:
            _fault(f, "invalid_enum", f"catalog.harnesses.{hid}.kind: {h.get('kind')!r}")
    providers = cat.get("providers") or {}
    for pid, p in providers.items():
        _keys(f, f"catalog.providers.{pid}", p, {"id", "kind"}, {"id"})
    accounts = cat.get("accounts") or {}
    for aid, a in accounts.items():
        _keys(f, f"catalog.accounts.{aid}", a,
              {"provider", "credential_ref", "permitted_hosts"}, {"provider"})
        if a.get("provider") and a["provider"] not in providers:
            _fault(f, "ref_missing", f"catalog.accounts.{aid}.provider -> {a['provider']}")
    plans = cat.get("plans") or {}
    for plid, pl in plans.items():
        _keys(f, f"catalog.plans.{plid}", pl,
              {"provider", "account", "kind", "quota_dimensions", "metering_source"},
              {"provider", "kind"})
        if pl.get("provider") and pl["provider"] not in providers:
            _fault(f, "ref_missing", f"catalog.plans.{plid}.provider -> {pl['provider']}")
        if pl.get("account") and pl["account"] not in accounts:
            _fault(f, "ref_missing", f"catalog.plans.{plid}.account -> {pl['account']}")

    bindings = cat.get("bindings") or {}
    for bid, b in bindings.items():
        _keys(f, f"catalog.bindings.{bid}", b,
              {"harness", "provider", "model", "reasoning", "plan", "account",
               "status", "capabilities", "task_restrictions", "note"},
              {"status", "capabilities"})
        st = b.get("status")
        if st not in BINDING_STATUS:
            _fault(f, "invalid_enum", f"catalog.bindings.{bid}.status: {st!r}")
        if st == "admitted":
            for ref_key, space in (("harness", harnesses), ("provider", providers),
                                   ("plan", plans)):
                if not b.get(ref_key) or b[ref_key] not in space:
                    _fault(f, "invariant_violation",
                           f"admitted binding {bid} requires resolved {ref_key}")
            if not b.get("model"):
                _fault(f, "invariant_violation",
                       f"admitted binding {bid} requires a concrete model identifier")
            acct = b.get("account")
            if acct and acct not in accounts:
                _fault(f, "ref_missing", f"catalog.bindings.{bid}.account -> {acct}")
        caps = b.get("capabilities")
        if not isinstance(caps, list) or not all(isinstance(c, str) for c in caps):
            _fault(f, "type_mismatch", f"catalog.bindings.{bid}.capabilities: string list")

    # --- planes: owner policy separated from observed health
    planes = cfg.get("planes") or {}
    if not isinstance(planes, dict):
        _fault(f, "type_mismatch", f"planes: expected mapping, got {type(planes).__name__}")
        planes = {}
    for plid, pl in planes.items():
        if not isinstance(pl, dict):
            _fault(f, "type_mismatch", f"planes.{plid}: expected mapping")
            continue
        _keys(f, f"planes.{plid}", pl,
              {"owner_policy", "binding_refs", "allowed_task_classes",
               "health_contract", "budget_refs", "note"},
              {"owner_policy", "binding_refs"})
        if pl.get("owner_policy") not in PLANE_POLICIES:
            _fault(f, "invalid_enum", f"planes.{plid}.owner_policy: {pl.get('owner_policy')!r}")
        refs = pl.get("binding_refs")
        if not isinstance(refs, list):
            _fault(f, "type_mismatch", f"planes.{plid}.binding_refs: list")
        else:
            for r in refs:
                if not isinstance(r, str):
                    _fault(f, "ref_missing",
                           f"planes.{plid}.binding_refs: string refs required")
                    continue
                if r not in bindings:
                    _fault(f, "ref_missing", f"planes.{plid}.binding_refs -> {r}")
        hc = pl.get("health_contract")
        if hc is not None and not isinstance(hc, dict):
            _fault(f, "type_mismatch",
                   f"planes.{plid}.health_contract: expected mapping")
            hc = {}
        hc = hc or {}
        _keys(f, f"planes.{plid}.health_contract", hc, {"source_ref", "ttl_seconds"})
        ttl = hc.get("ttl_seconds")
        if ttl is not None and (not isinstance(ttl, int) or ttl < 10):
            _fault(f, "invariant_violation", f"planes.{plid}.health_contract.ttl_seconds >= 10")

    # --- roles
    roles = cfg.get("roles") or {}
    for rid, r in roles.items():
        if not isinstance(r, dict):
            _fault(f, "type_mismatch", f"roles.{rid}: expected role mapping, got {type(r).__name__}")
            continue
        _keys(f, f"roles.{rid}", r,
              {"kind", "supervisor_ref", "binding_refs", "default_binding_ref",
               "fallback_binding_refs", "scenario_policy_ref", "authority_ref",
               "required_capabilities", "triggers", "rhythms", "profiles",
               "reserve_ref", "lease_seconds", "heartbeat_seconds",
               "max_attempts_per_incident", "model", "campaign_ns", "escalation",
               "repair_role_ref", "routing"})
        if r.get("routing") is not None and v != 3:
            _fault(f, "unknown_field",
                   f"roles.{rid}.routing requires schema_version 3")
        if r.get("kind") not in ROLE_KINDS:
            _fault(f, "invalid_enum", f"roles.{rid}.kind: {r.get('kind')!r}")
        if r.get("kind") == "deterministic" and r.get("model") not in (None, "none"):
            _fault(f, "invariant_violation", f"roles.{rid}: deterministic role must not declare a model")
        for key in ("binding_refs", "fallback_binding_refs"):
            refs_val = r.get(key)
            if refs_val is not None and not isinstance(refs_val, list):
                _fault(f, "type_mismatch",
                       f"roles.{rid}.{key}: expected list")
                continue
            for ref in (refs_val or []):
                if not isinstance(ref, str):
                    # unhashable/malformed elements fault structurally
                    _fault(f, "ref_missing",
                           f"roles.{rid}.{key}: string refs required")
                    continue
                if ref not in bindings:
                    _fault(f, "ref_missing", f"roles.{rid}.{key} -> {ref}")
        dref = r.get("default_binding_ref")
        if dref:
            if dref not in bindings:
                _fault(f, "ref_missing", f"roles.{rid}.default_binding_ref -> {dref}")
            elif bindings[dref].get("status") != "admitted":
                _fault(f, "invariant_violation",
                       f"role {rid} default binding {dref} must be admitted")
        caps = set(r.get("required_capabilities") or [])
        refs_val = r.get("binding_refs")
        for ref in (refs_val if isinstance(refs_val, list) else []):
            if not isinstance(ref, str):
                continue  # faulted above; unhashable never reaches `in`
            bcaps = set((bindings.get(ref) or {}).get("capabilities") or [])
            missing = caps - bcaps
            if ref in bindings and (bindings[ref].get("status") == "admitted") and missing:
                _fault(f, "invariant_violation",
                       f"role {rid}: admitted binding {ref} lacks capabilities {sorted(missing)}")

    # --- editor scenarios
    for sid, s in (cfg.get("editor_scenarios") or {}).items():
        _keys(f, f"editor_scenarios.{sid}", s,
              {"scope", "permitted_trigger_classes", "binding_order",
               "activation", "handover_contract_ref", "on_no_eligible_editor"},
              {"binding_order", "activation"})
        order = s.get("binding_order")
        if not isinstance(order, list) or not order:
            _fault(f, "type_mismatch", f"editor_scenarios.{sid}.binding_order: non-empty list")
        for ref in order or []:
            if ref not in bindings:
                _fault(f, "ref_missing", f"editor_scenarios.{sid}.binding_order -> {ref}")
        act = s.get("activation") or {}
        _keys(f, f"editor_scenarios.{sid}.activation", act,
              {"requires_admitted_target", "authority_ref", "ttl_seconds",
               "max_swaps_per_incident"})
        msi = act.get("max_swaps_per_incident")
        if msi is not None and (not isinstance(msi, int) or msi < 0 or msi > 5):
            _fault(f, "invariant_violation", f"editor_scenarios.{sid} max_swaps_per_incident in 0..5")

    # --- campaign policies (exploration burn directive)
    for cid, c in (cfg.get("campaign_policies") or {}).items():
        _keys(f, f"campaign_policies.{cid}", c,
              {"mode", "authorization", "editor", "directions", "allocation",
               "review", "stop_conditions"},
              {"mode", "authorization", "directions", "allocation"})
        modes = ("directed_multi_direction",)
        if c.get("mode") not in modes:
            _fault(f, "invalid_enum", f"campaign_policies.{cid}.mode: {c.get('mode')!r}")
        d = c.get("directions") or {}
        _keys(f, f"campaign_policies.{cid}.directions", d,
              {"require_distinct_question", "require_evidence_deliverable",
               "max_active", "max_waves"})
        alloc = c.get("allocation") or {}
        _keys(f, f"campaign_policies.{cid}.allocation", alloc,
              {"budget_ref", "reserve_per_direction", "editor_review_reserve",
               "maintenance_reserve_is_spendable"})
        if alloc.get("maintenance_reserve_is_spendable") is True:
            _fault(f, "invariant_violation",
                   f"campaign_policies.{cid}: maintenance reserve is never spendable")
        for rsv in ("reserve_per_direction", "editor_review_reserve"):
            rv = alloc.get(rsv) or {}
            for k, vv in rv.items():
                if not isinstance(vv, int) or vv < 0:
                    _fault(f, "invariant_violation", f"campaign_policies.{cid}.allocation.{rsv}.{k}: int >= 0")

    # --- supply
    suply = cfg.get("supply") or {}
    _keys(f, "supply", suply,
          {"namespaces", "namespaces_ref", "admission", "schedules",
           "retry_policy", "budgets", "maintenance_reserve_ref",
           "learned_patch_policy_ref", "pressure"})
    if "admission" in suply and "pressure" in suply:
        _fault(f, "PRESSURE_SOURCE_CONFLICT",
               "supply.admission and supply.pressure cannot coexist (C15)")
    if suply.get("pressure") and v != 3:
        _fault(f, "unknown_field", "supply.pressure requires schema_version 3")
    budgets = suply.get("budgets") or {}
    for bid_, b_ in budgets.items():
        _keys(f, f"supply.budgets.{bid_}", b_,
              {"scope", "window_hours", "hard_limits", "spendable", "note"},
              {"scope", "hard_limits"})
        if not isinstance(b_.get("hard_limits"), dict) or not b_["hard_limits"]:
            _fault(f, "invariant_violation", f"supply.budgets.{bid_}.hard_limits: non-empty map")
    for plid, pl in planes.items():
        for br in (pl.get("budget_refs") or []):
            if br not in budgets:
                _fault(f, "ref_missing", f"planes.{plid}.budget_refs -> {br}")
    ck = roles.get("core-keeper") or {}
    if ck.get("reserve_ref") and ck["reserve_ref"] not in budgets:
        _fault(f, "ref_missing", f"roles.core-keeper.reserve_ref -> {ck['reserve_ref']}")
    for cid_, c_ in (cfg.get("campaign_policies") or {}).items():
        bref = ((c_.get("allocation") or {}).get("budget_ref")) or ""
        if bref and bref not in budgets:
            _fault(f, "ref_missing", f"campaign_policies.{cid_}.allocation.budget_ref -> {bref}")

    # --- intent + sync
    intent = cfg.get("intent") or {}
    _keys(f, "intent", intent,
          {"canonical", "revision", "hash", "authority_class", "research",
           "registry_ref", "kasten_ref", "hub_door"}, {"canonical"})
    sync = cfg.get("sync") or {}
    _keys(f, "sync", sync,
          {"protocol_version", "transport_adapter_ref", "reconcile_seconds",
           "require_receiver_validation", "activation_ack_consumers",
           "disconnected_dispatch", "integrity"}, {"protocol_version"})
    rs = sync.get("reconcile_seconds")
    if rs is not None and (not isinstance(rs, int) or rs < 30):
        _fault(f, "invariant_violation", "sync.reconcile_seconds >= 30")

    if v == 3:
        _validate_pressure(f, cfg)

    _validate_introspection(f, cfg)

    return f


def _validate_introspection(f, cfg):
    """Optional read-only enrollment; absence keeps every source disabled."""
    if "introspection" not in cfg or not isinstance(cfg["introspection"], dict):
        return
    spec = cfg["introspection"]
    _keys(f, "introspection", spec,
          {"version", "enabled", "authority_ref", "host_aliases", "sources", "limits", "retention"},
          {"version", "enabled", "sources"})
    if type(spec.get("version")) is not int or spec["version"] != 1:
        _fault(f, "inspection_schema", "introspection.version must be 1")
    if type(spec.get("enabled")) is not bool:
        _fault(f, "inspection_schema", "introspection.enabled must be boolean")
    hosts = ((cfg.get("fleet") or {}).get("hosts") or [])
    aliases = spec.get("host_aliases", {})
    if not isinstance(aliases, dict) or any(not isinstance(k, str) or v not in hosts for k, v in aliases.items()):
        _fault(f, "inspection_schema", "introspection.host_aliases must resolve to fleet hosts")
    sources = spec.get("sources")
    if not isinstance(sources, dict):
        _fault(f, "inspection_schema", "introspection.sources must be a mapping")
        return
    kinds = {"queue", "attempt_history", "heartbeat", "services", "task_ledger",
             "schedule", "composer", "reviews", "memory_lock"}
    for sid, source in sources.items():
        if not isinstance(sid, str) or not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_-]{0,79}", sid) or not isinstance(source, dict):
            _fault(f, "inspection_schema", "invalid introspection source ID/record")
            continue
        path = "introspection.sources." + sid
        _keys(f, path, source, {"kind", "host", "path", "enabled", "ttl_seconds", "privacy",
                              "units", "queue_source_ref", "note"},
              {"kind", "host", "enabled", "ttl_seconds", "privacy"})
        # Set/dict membership on an unhashable kind/ref raised TypeError and
        # crashed whole-config validation; type-check before membership.
        kind = source.get("kind")
        if not isinstance(kind, str) or kind not in kinds or source.get("host") not in hosts:
            _fault(f, "inspection_schema", path + ": unknown kind or host")
        if type(source.get("enabled")) is not bool:
            _fault(f, "inspection_schema", path + ".enabled must be boolean")
        ttl = source.get("ttl_seconds")
        if type(ttl) is not int or not 1 <= ttl <= 86400:
            _fault(f, "inspection_schema", path + ".ttl_seconds must be 1..86400")
        if source.get("privacy") != "inspection-metadata-v1":
            _fault(f, "inspection_schema", path + ": unsupported metadata privacy contract")
        source_path = source.get("path")
        if kind != "services":
            if not isinstance(source_path, str) or not source_path or any(c in source_path for c in "\x00\n\r"):
                _fault(f, "inspection_schema", path + ".path required")
            elif any(part.lower() in {".ssh", ".gnupg", "auth", "credentials", "vault", "vaults"}
                     for part in Path(source_path).parts):
                _fault(f, "inspection_schema", path + ": credential/vault paths are excluded")
        if "units" in source and (not isinstance(source["units"], list) or any(
                not isinstance(unit, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9@_.:-]*\.(?:service|timer)", unit)
                for unit in source["units"])):
            _fault(f, "inspection_schema", path + ": invalid service units")
        qref = source.get("queue_source_ref")
        if qref is not None and (not isinstance(qref, str) or qref not in sources):
            _fault(f, "inspection_schema", path + ": unknown queue_source_ref")
    for section, fields in {
        "limits": {"read_timeout_seconds": (1, 30), "total_timeout_seconds": (1, 60),
                   "max_records": (1, 10000), "max_bytes": (4096, 16777216)},
        "retention": {"raw_days": (1, 365), "segment_bytes": (1048576, 67108864),
                      "max_store_bytes": (8388608, 2147483648)},
    }.items():
        value = spec.get(section, {})
        if not isinstance(value, dict):
            _fault(f, "inspection_schema", "introspection." + section + " must be a mapping")
            continue
        _keys(f, "introspection." + section, value, set(fields))
        for key, number in value.items():
            if key in fields and (type(number) is not int or not fields[key][0] <= number <= fields[key][1]):
                _fault(f, "inspection_schema", "introspection." + section + "." + key + " outside bounds")


# ------------------------------------------------------------- pressure v3

_PROFILE_KEYS = {"binding_ref", "default_for_binding", "reasoning_override"}
_ROUTING_KEYS = {"preferred_profile_ref", "fallback_profile_refs",
                 "max_concurrent_slots", "parallel"}
_PARALLEL_KEYS = {"strategy", "lanes"}
_LANE_KEYS = {"profile_ref", "weight"}
_PARALLEL_STRATEGIES = ("single", "weighted_fair")
_CEILINGS_KEYS = {"global_concurrent_slots", "default_binding_concurrent_slots",
                  "binding_concurrent_slots", "default_profile_concurrent_slots",
                  "profile_concurrent_slots", "pools"}
_POOL_KEYS = {"binding_refs", "concurrent_slots",
              "cooldown_after_refusal_seconds"}
_LEVEL_KEYS = {"default_profile_slots", "profile_slots"}
_DEMAND_KEYS = {"level", "profile_slot_overrides"}
_CONTROL_KEYS = {"persistent_agent_edits", "agent_role_refs",
                 "agent_profile_refs", "agent_level_names",
                 "lease_max_ttl_seconds", "lease_max_active_per_issuer"}
_PRESSURE_KEYS = {"ceilings", "levels", "demand", "control"}
_REASONING_LEVELS = ("low", "medium", "high", "max")
# deterministic adapter-specific reasoning support (F35.10r): a reasoning
# override is certified only against a known adapter that declares the
# level; an unknown adapter refuses certification
_ADAPTER_REASONING_SUPPORT = {
    "consult-codex-v1": ("low", "medium", "high", "max"),
    "consult-agy-v1": ("low", "medium", "high"),
    "opencode-cli-v1": ("low", "medium", "high"),
    "queue-serve-v2": ("low", "medium", "high", "max"),
    "autoclaw-managed-plane-v1": ("low", "medium", "high"),
}


def _pmap(f, path, val):
    """Map-typed pressure field: a non-dict value is a PRESSURE_SCHEMA
    fault, never an AttributeError from a later .items() (F35.10r)."""
    if val is None:
        return {}
    if not isinstance(val, dict):
        _fault(f, "PRESSURE_SCHEMA",
               f"{path}: expected mapping, got {type(val).__name__}")
        return {}
    return val


def _slots_int(f, path, val):
    if not isinstance(val, int) or isinstance(val, bool) or val < 1:
        _fault(f, "PRESSURE_SCHEMA",
               f"{path}: positive int required, got {val!r}")
        return False
    return True


def _pkeys(f, path, val, allowed, required=()):
    """_keys with pressure-section coding: unknown/missing fields are
    PRESSURE_SCHEMA faults (claim C13), not generic unknown_field."""
    n0 = len(f)
    _keys(f, path, val, allowed, required)
    for x in f[n0:]:
        if x["code"] == "unknown_field":
            x["code"] = "PRESSURE_SCHEMA"


def _validate_pressure(f, cfg):
    """The v3 pressure section: closed schemas, resolved references, at most
    one default profile per binding, every numeric target within its
    profile/binding/pool/global ceilings, routing agreeing with the retained
    legacy role projections (sol pressure design D1-D4, claims C13/C16/C24)."""
    suply = cfg.get("supply") or {}
    pr = suply.get("pressure") or {}
    catalog = cfg.get("catalog") or {}
    profiles = catalog.get("execution_profiles") or {}
    bindings = catalog.get("bindings") or {}
    harnesses = catalog.get("harnesses") or {}

    # --- execution profiles
    # typed local map: a faulted malformed profile record is reported and
    # then EXCLUDED from every later consumer (F35.10rrr/F35.10rrrr)
    if not isinstance(profiles, dict):
        # faulted upstream by the catalog typecheck; normalize so the
        # comprehension cannot .items() a nonmapping (F35.10t)
        profiles = {}
    profiles_ok = {k: v for k, v in profiles.items() if isinstance(v, dict)}
    defaults = {}
    aliases = {}
    resolver_supported = _REASONING_LEVELS
    for pid, prof in profiles.items():
        if not isinstance(prof, dict):
            _fault(f, "PRESSURE_SCHEMA",
                   f"catalog.execution_profiles.{pid}: expected mapping")
            continue
        _pkeys(f, f"catalog.execution_profiles.{pid}", prof, _PROFILE_KEYS,
              {"binding_ref"})
        if "default_for_binding" in prof and                 type(prof["default_for_binding"]) is not bool:
            _fault(f, "PRESSURE_SCHEMA",
                   f"catalog.execution_profiles.{pid}.default_for_binding: "
                   "bool required")
        ro = prof.get("reasoning_override")
        if ro is not None:
            if not isinstance(ro, str) or ro.strip() not in resolver_supported:
                _fault(f, "PRESSURE_SCHEMA",
                       f"catalog.execution_profiles.{pid}.reasoning_override: "
                       f"adapter-supported reasoning required {list(resolver_supported)}")
            else:
                # certification resolves the binding's harness adapter and
                # requires it to DECLARE support for the level (F35.10r)
                bref_ro = prof.get("binding_ref")
                aref = ((harnesses.get(
                    (bindings.get(bref_ro) or {}).get("harness")) or {})
                    .get("adapter_ref")) if isinstance(bref_ro, str) and \
                    bref_ro in bindings else None
                if not isinstance(aref, str):
                    aref = None  # unhashable/malformed: no declared support
                support = _ADAPTER_REASONING_SUPPORT.get(aref)
                if support is None:
                    _fault(f, "PRESSURE_REF",
                           f"catalog.execution_profiles.{pid}."
                           f"reasoning_override: adapter {aref!r} does not "
                           "declare reasoning support")
                elif ro.strip() not in support:
                    _fault(f, "PRESSURE_REF",
                           f"catalog.execution_profiles.{pid}."
                           f"reasoning_override: adapter {aref!r} does not "
                           f"support {ro.strip()!r}")
        bref = prof.get("binding_ref")
        if not isinstance(bref, str) or not bref:
            _fault(f, "PRESSURE_REF",
                   f"catalog.execution_profiles.{pid}.binding_ref: string "
                   f"required, got {bref!r}")
            continue
        if bref not in bindings:
            _fault(f, "PRESSURE_REF",
                   f"catalog.execution_profiles.{pid}.binding_ref -> {bref}")
        elif bindings[bref].get("status") != "admitted":
            _fault(f, "PRESSURE_PROFILE",
                   f"catalog.execution_profiles.{pid}: binding {bref} is not admitted")
        if prof.get("default_for_binding") is True:
            defaults.setdefault(bref, []).append(pid)
            aliases[f"{bref}/default"] = pid
    for bref, pids in defaults.items():
        if len(pids) > 1:
            _fault(f, "PRESSURE_PROFILE",
                   f"binding {bref}: multiple default profiles {sorted(pids)}")

    def _ck(k):
        return aliases.get(k, k)

    def _canon_conflict(where, m):
        """Canonical-plus-alias assignments must agree (F35.7r)."""
        if not isinstance(m, dict):
            return
        seen = {}
        for k, v in m.items():
            c = _ck(k)
            if c in seen and seen[c] != v:
                _fault(f, "PRESSURE_SCHEMA",
                       f"{where}: conflicting canonical and alias entries "
                       f"for {c} ({seen[c]!r} vs {v!r})")
            seen.setdefault(c, v)

    if not pr:
        return
    if not isinstance(pr, dict):
        _fault(f, "PRESSURE_SCHEMA", "supply.pressure: expected mapping")
        return
    _pkeys(f, "supply.pressure", pr, _PRESSURE_KEYS,
           {"ceilings", "levels", "demand", "control"})

    # --- ceilings
    ceilings = pr.get("ceilings")
    if ceilings is not None and not isinstance(ceilings, dict):
        _fault(f, "PRESSURE_SCHEMA",
               "supply.pressure.ceilings: expected mapping")
        ceilings = {}
    if not isinstance(ceilings, dict):
        ceilings = {}
    _pkeys(f, "supply.pressure.ceilings", ceilings, _CEILINGS_KEYS,
           {"global_concurrent_slots"})
    g = ceilings.get("global_concurrent_slots")
    _slots_int(f, "supply.pressure.ceilings.global_concurrent_slots", g)
    bceil_raw = _pmap(f, "supply.pressure.ceilings.binding_concurrent_slots",
                      ceilings.get("binding_concurrent_slots"))
    for k, n in bceil_raw.items():
        _slots_int(f, f"supply.pressure.ceilings.binding_concurrent_slots.{k}", n)
        if k not in bindings:
            _fault(f, "PRESSURE_REF",
                   f"supply.pressure.ceilings.binding_concurrent_slots -> {k}")
    # only valid typed values reach bounds math (F35.10rr)
    bceil = {k: v for k, v in bceil_raw.items() if type(v) is int}
    if "default_binding_concurrent_slots" in ceilings:
        _slots_int(f, "supply.pressure.ceilings.default_binding_concurrent_slots",
                   ceilings.get("default_binding_concurrent_slots"))
    pceil_raw = _pmap(f, "supply.pressure.ceilings.profile_concurrent_slots",
                      ceilings.get("profile_concurrent_slots"))
    for k, n in pceil_raw.items():
        _slots_int(f, f"supply.pressure.ceilings.profile_concurrent_slots.{k}", n)
        if k not in profiles:
            _fault(f, "PRESSURE_REF",
                   f"supply.pressure.ceilings.profile_concurrent_slots -> {k}")
    pceil = {k: v for k, v in pceil_raw.items() if type(v) is int}
    if "default_profile_concurrent_slots" in ceilings:
        _slots_int(f, "supply.pressure.ceilings.default_profile_concurrent_slots",
                   ceilings.get("default_profile_concurrent_slots"))
    pools = _pmap(f, "supply.pressure.ceilings.pools",
                  ceilings.get("pools"))
    pool_of = {}
    for pid, pool in pools.items():
        if not isinstance(pool, dict):
            _fault(f, "PRESSURE_SCHEMA",
                   f"supply.pressure.ceilings.pools.{pid}: expected mapping")
            continue
        _pkeys(f, f"supply.pressure.ceilings.pools.{pid}", pool, _POOL_KEYS,
              {"binding_refs", "concurrent_slots"})
        _slots_int(f, f"supply.pressure.ceilings.pools.{pid}.concurrent_slots",
                   pool.get("concurrent_slots"))
        cd = pool.get("cooldown_after_refusal_seconds")
        if cd is not None and (not isinstance(cd, int) or isinstance(cd, bool)
                               or cd < 0):
            _fault(f, "PRESSURE_SCHEMA",
                   f"supply.pressure.ceilings.pools.{pid}.cooldown_after_refusal_seconds: int >= 0")
        pcs = pool.get("concurrent_slots")
        pcs_ok = pcs if type(pcs) is int else None
        pref_raw = pool.get("binding_refs")
        if "binding_refs" in pool and not isinstance(pref_raw, list):
            _fault(f, "PRESSURE_SCHEMA",
                   f"supply.pressure.ceilings.pools.{pid}.binding_refs: "
                   "list required")
            pref_raw = []
        for b in (pref_raw or []):
            if not isinstance(b, str):
                # unhashable members fault structurally, never TypeError
                _fault(f, "PRESSURE_REF",
                       f"supply.pressure.ceilings.pools.{pid}.binding_refs: "
                       "string refs required")
                continue
            if b not in bindings:
                _fault(f, "PRESSURE_REF",
                       f"supply.pressure.ceilings.pools.{pid}.binding_refs -> {b}")
            if pcs_ok is not None:
                pool_of.setdefault(b, []).append((pid, pcs_ok))

    # typed local defaults: faulted values never reach comparisons
    # (F35.10rrr)
    dpcs = ceilings.get("default_profile_concurrent_slots", 1)
    dbcs = ceilings.get("default_binding_concurrent_slots", 1)
    dpcs = dpcs if type(dpcs) is int else 1
    dbcs = dbcs if type(dbcs) is int else 1

    def _ceil_profile(pid):
        # ONE canonical resolver (F35.7rr): an alias resolves to its
        # canonical profile's explicit ceiling before any fallback
        canon = aliases.get(pid, pid) if isinstance(pid, str) else pid
        if canon in pceil:
            return pceil[canon]
        if isinstance(canon, str) and canon.endswith("/default"):
            b = canon[:-len("/default")]
            if b in bceil:
                return bceil[b]
        return dpcs

    def _ceil_binding(b):
        if b in bceil:
            return bceil[b]
        return dbcs

    def _target_bounds(where, pid, n):
        if not isinstance(n, int) or isinstance(n, bool) or n < 1:
            _fault(f, "PRESSURE_SCHEMA", f"{where}: positive int required, got {n!r}")
            return
        prof = profiles_ok.get(pid)
        if prof is None and isinstance(pid, str) and pid.endswith("/default"):
            prof = {"binding_ref": pid[:-len("/default")]}
        if prof is None:
            _fault(f, "PRESSURE_REF", f"{where}: unknown profile {pid}")
            return
        bref = prof.get("binding_ref")
        if not isinstance(bref, str):
            bref = None  # faulted at the profile; never sorts or hashes
        if n > _ceil_profile(pid):
            _fault(f, "PRESSURE_BOUNDS",
                   f"{where}: {n} exceeds profile ceiling {_ceil_profile(pid)}")
        if bref is not None and n > _ceil_binding(bref):
            _fault(f, "PRESSURE_BOUNDS",
                   f"{where}: {n} exceeds binding {bref} ceiling {_ceil_binding(bref)}")
        for pool_id, ps in pool_of.get(bref, []):
            if n > ps:
                _fault(f, "PRESSURE_BOUNDS",
                       f"{where}: {n} exceeds pool {pool_id} ceiling {ps}")
        if isinstance(g, int) and n > g:
            _fault(f, "PRESSURE_BOUNDS",
                   f"{where}: {n} exceeds global ceiling {g}")

    def _bound(where, pid, n):
        _target_bounds(where, pid, n)

    # --- levels
    levels = pr.get("levels") or {}
    if not isinstance(levels, dict):
        _fault(f, "PRESSURE_SCHEMA", "supply.pressure.levels: expected mapping")
        levels = {}
    if not levels:
        _fault(f, "PRESSURE_SCHEMA", "supply.pressure.levels: non-empty mapping")
        levels = {}
    for lname, lv in levels.items():
        if not isinstance(lv, dict):
            _fault(f, "PRESSURE_SCHEMA",
                   f"supply.pressure.levels.{lname}: expected mapping")
            continue
        _pkeys(f, f"supply.pressure.levels.{lname}", lv, _LEVEL_KEYS,
              {"default_profile_slots"})
        _slots_int(f, f"supply.pressure.levels.{lname}.default_profile_slots",
                   lv.get("default_profile_slots"))
        lv_slots = _pmap(f, f"supply.pressure.levels.{lname}.profile_slots",
                         lv.get("profile_slots"))
        _canon_conflict(f"supply.pressure.levels.{lname}.profile_slots",
                        lv_slots)
        for pid, n in lv_slots.items():
            if pid not in profiles and not (
                    isinstance(pid, str) and pid.endswith("/default")
                    and pid[:-len("/default")] in bindings):
                _fault(f, "PRESSURE_REF",
                       f"supply.pressure.levels.{lname}.profile_slots -> {pid}")
                continue
            _bound(f"supply.pressure.levels.{lname}.profile_slots.{pid}", pid, n)

    # the canonical profile universe: explicit profiles plus every
    # synthesized <binding>/default that is not an alias (F35.10r —
    # one canonical fully expanded target calculation)
    canon_universe = set(profiles_ok.keys())
    for bid in bindings:
        if f"{bid}/default" not in aliases:
            canon_universe.add(f"{bid}/default")

    def _expanded(lname):
        lv = levels.get(lname)
        if not isinstance(lv, dict):
            return {}  # faulted upstream; never reaches .get consumers
        slots = {}
        m = lv.get("profile_slots")
        if isinstance(m, dict):
            for k, v in m.items():
                # VALUE-level normalization (F35.10t): a faulted non-int
                # target is reported upstream and never reaches the
                # delegation comparison
                if type(v) is int and not isinstance(v, bool):
                    slots[_ck(k)] = v
        dflt = lv.get("default_profile_slots")
        if type(dflt) is int and not isinstance(dflt, bool):
            for pid in canon_universe:
                slots.setdefault(pid, dflt)
        return slots

    # EVERY level's fully expanded targets stay within their applicable
    # profile/binding/pool/global ceilings — not just the global default
    # bound (F35.10r)
    for lname in list(levels):
        if not isinstance(levels.get(lname), dict):
            continue
        for pid, n in _expanded(lname).items():
            _target_bounds(
                f"supply.pressure.levels.{lname} (expanded target {pid})",
                pid, n)

    # --- demand
    demand = pr.get("demand")
    if demand is not None and not isinstance(demand, dict):
        _fault(f, "PRESSURE_SCHEMA", "supply.pressure.demand: expected mapping")
        demand = {}
    demand = demand or {}
    _pkeys(f, "supply.pressure.demand", demand, _DEMAND_KEYS, {"level"})
    lvl = demand.get("level")
    if lvl is None and "level" in demand:
        # an explicit null faults — level is required and string-typed
        # (F35.10rrrr)
        _fault(f, "PRESSURE_REF",
               "supply.pressure.demand.level: string required, got None")
    elif lvl is not None:
        if not isinstance(lvl, str):
            _fault(f, "PRESSURE_REF",
                   f"supply.pressure.demand.level: string required, got "
                   f"{lvl!r}")
            lvl = None  # typed local: never reaches delegation lookups
        elif lvl not in levels:
            _fault(f, "PRESSURE_REF", f"supply.pressure.demand.level -> {lvl}")
    dem_over = _pmap(f, "supply.pressure.demand.profile_slot_overrides",
                     demand.get("profile_slot_overrides"))
    _canon_conflict("supply.pressure.demand.profile_slot_overrides", dem_over)
    for pid, n in dem_over.items():
        _bound(f"supply.pressure.demand.profile_slot_overrides.{pid}", pid, n)

    # --- control (the delegation boundary)
    control = pr.get("control")
    if control is not None and not isinstance(control, dict):
        _fault(f, "PRESSURE_SCHEMA", "supply.pressure.control: expected mapping")
        control = {}
    control = control or {}
    _pkeys(f, "supply.pressure.control", control, _CONTROL_KEYS,
           {"persistent_agent_edits", "agent_level_names",
            "agent_profile_refs"})
    pae = control.get("persistent_agent_edits")
    if not isinstance(pae, bool):
        _fault(f, "PRESSURE_SCHEMA",
               "supply.pressure.control.persistent_agent_edits: bool required")
    control_ok = dict(control)  # normalize a LOCAL copy: input stays pure
    for lst_key in ("agent_role_refs", "agent_profile_refs", "agent_level_names"):
        lst = control.get(lst_key)
        if lst_key in control and not isinstance(lst, list):
            # an explicit null or non-list faults — it is never accepted
            # as an empty list (F35.10rrrr)
            _fault(f, "PRESSURE_SCHEMA",
                   f"supply.pressure.control.{lst_key}: list required, got "
                   f"{type(lst).__name__}")
            control_ok[lst_key] = []
    for lst_key in ("agent_role_refs", "agent_profile_refs", "agent_level_names"):
        for x in (control_ok.get(lst_key) or []):
            if not isinstance(x, str) or not x.strip():
                _fault(f, "PRESSURE_SCHEMA",
                       f"supply.pressure.control.{lst_key}: non-empty strings")
    for x in (control_ok.get("agent_level_names") or []):
        if not isinstance(x, str):
            continue  # faulted above; unhashable never reaches membership
        if x not in levels:
            _fault(f, "PRESSURE_DELEGATION",
                   f"supply.pressure.control.agent_level_names -> {x}")
    for x in (control_ok.get("agent_profile_refs") or []):
        if not isinstance(x, str):
            continue  # already faulted above; unhashable never reaches `in`
        if x not in profiles:
            _fault(f, "PRESSURE_DELEGATION",
                   f"supply.pressure.control.agent_profile_refs -> {x}")
    for x in (control_ok.get("agent_role_refs") or []):
        if not isinstance(x, str):
            continue
        if x not in (cfg.get("roles") or {}):
            _fault(f, "PRESSURE_DELEGATION",
                   f"supply.pressure.control.agent_role_refs -> {x}")
    if "lease_max_ttl_seconds" in control:
        _slots_int(f, "supply.pressure.control.lease_max_ttl_seconds",
                   control.get("lease_max_ttl_seconds"))
        ttl = control.get("lease_max_ttl_seconds")
        if isinstance(ttl, int) and ttl > 900:
            _fault(f, "PRESSURE_SCHEMA",
                   "supply.pressure.control.lease_max_ttl_seconds: the "
                   "certified maximum is 900")
    if "lease_max_active_per_issuer" in control:
        m_ = control.get("lease_max_active_per_issuer")
        if type(m_) is not int or m_ != 1:
            # exact int: True == 1 must not pass (F35.10r)
            _fault(f, "PRESSURE_SCHEMA",
                   "supply.pressure.control.lease_max_active_per_issuer: "
                   "exactly 1 (certified)")
    if control.get("persistent_agent_edits") is True:
        # delegation over the FULLY EXPANDED targets: a delegated level
        # must not raise any undelegated profile — explicit or synthesized
        # through the level default (F35.10r/F35.10rr)
        base_name = lvl  # typed local: non-string levels never look up
        delegated_refs = {_ck(x) for x in
                          (control_ok.get("agent_profile_refs") or [])
                          if isinstance(x, str)}
        for lvl_name in (control_ok.get("agent_level_names") or []):
            if not isinstance(lvl_name, str):
                continue  # faulted above; unhashable never reaches lookup
            if base_name and base_name != lvl_name:
                eb = _expanded(base_name)
                el = _expanded(lvl_name)
                for pid in sorted(canon_universe):
                    if el.get(pid, 0) > eb.get(pid, 0) and \
                            _ck(pid) not in delegated_refs:
                        _fault(f, "PRESSURE_DELEGATION",
                               f"level {lvl_name} raises undelegated "
                               f"profile {pid} "
                               f"({eb.get(pid)} -> {el.get(pid)})")

    # --- role routing
    def _prof_ok(ref):
        """An explicit profile, or a synthesized <binding>/default for any
        EXISTING binding (declared bindings route but never launch — sol
        pressure design: kimi-editor/default remains a fallback reference)."""
        if not isinstance(ref, str):
            return False
        if ref in profiles:
            return True
        if isinstance(ref, str) and ref.endswith("/default"):
            return ref[:-len("/default")] in bindings
        return False

    for rid, r in (cfg.get("roles") or {}).items():
        rt = r.get("routing") if isinstance(r, dict) else None
        if rt is None:
            continue
        if not isinstance(rt, dict):
            _fault(f, "PRESSURE_SCHEMA",
                   f"roles.{rid}.routing: expected mapping, got "
                   f"{type(rt).__name__}")
            continue
        _pkeys(f, f"roles.{rid}.routing", rt, _ROUTING_KEYS,
              {"preferred_profile_ref"})
        pref = rt.get("preferred_profile_ref")
        if not isinstance(pref, str) or not _prof_ok(pref):
            _fault(f, "PRESSURE_REF",
                   f"roles.{rid}.routing.preferred_profile_ref -> {pref}")
        fb_list = rt.get("fallback_profile_refs")
        if fb_list is not None and not isinstance(fb_list, list):
            _fault(f, "PRESSURE_SCHEMA",
                   f"roles.{rid}.routing.fallback_profile_refs: expected list")
            fb_list = []
        # chain uniqueness after ALIAS resolution (F35.10r)
        canon_seen = {_ck(pref)} if isinstance(pref, str) else set()
        for x in (fb_list or []):
            if not isinstance(x, str):
                _fault(f, "PRESSURE_REF",
                       f"roles.{rid}.routing.fallback_profile_refs: string "
                       "refs required")
                continue
            if not _prof_ok(x):
                _fault(f, "PRESSURE_REF",
                       f"roles.{rid}.routing.fallback_profile_refs -> {x}")
                continue
            cx = _ck(x)
            if cx in canon_seen:
                _fault(f, "PRESSURE_REF",
                       f"roles.{rid}.routing: duplicate fallback {x} "
                       f"(resolves to {cx})")
            canon_seen.add(cx)
        if "max_concurrent_slots" in rt:
            _slots_int(f, f"roles.{rid}.routing.max_concurrent_slots",
                       rt.get("max_concurrent_slots"))
        par = rt.get("parallel")
        if par is not None and not isinstance(par, dict):
            _fault(f, "PRESSURE_SCHEMA",
                   f"roles.{rid}.routing.parallel: expected mapping")
            par = {}
        _pkeys(f, f"roles.{rid}.routing.parallel", par or {}, _PARALLEL_KEYS)
        strat = (par or {}).get("strategy")
        if strat is not None and strat not in _PARALLEL_STRATEGIES:
            _fault(f, "PRESSURE_SCHEMA",
                   f"roles.{rid}.routing.parallel.strategy: {strat!r}")
        lanes = (par or {}).get("lanes")
        if lanes is not None and not isinstance(lanes, list):
            _fault(f, "PRESSURE_SCHEMA",
                   f"roles.{rid}.routing.parallel.lanes: expected list")
            lanes = []
        lane_seen = set()
        for ln in (lanes or []):
            if not isinstance(ln, dict):
                _fault(f, "PRESSURE_SCHEMA",
                       f"roles.{rid}.routing.parallel.lanes[]: expected mapping")
                continue
            _pkeys(f, f"roles.{rid}.routing.parallel.lanes[]", ln, _LANE_KEYS,
                  {"profile_ref", "weight"})
            lref = ln.get("profile_ref")
            if not isinstance(lref, str) or not _prof_ok(lref):
                _fault(f, "PRESSURE_REF",
                       f"roles.{rid}.routing.parallel.lanes[] -> {lref}")
            else:
                cl = _ck(lref)
                if cl in lane_seen:
                    _fault(f, "PRESSURE_REF",
                           f"roles.{rid}.routing.parallel: duplicate lane "
                           f"{lref} (resolves to {cl})")
                lane_seen.add(cl)
            w = ln.get("weight")
            if not isinstance(w, int) or isinstance(w, bool) or w < 1:
                _fault(f, "PRESSURE_SCHEMA",
                       f"roles.{rid}.routing.parallel.lanes[].weight: positive int")
        # retained legacy projections must agree with routing (C24)

        def _ref_binding(x):
            if not isinstance(x, str):
                return None  # already faulted; unhashable never reaches `in`
            prof = profiles_ok.get(x)
            if prof:
                b = prof.get("binding_ref")
                return b if isinstance(b, str) else None
            if x.endswith("/default"):
                return x[:-len("/default")]
            return None

        pref_b = _ref_binding(pref)
        if r.get("default_binding_ref") not in (None, pref_b):
            _fault(f, "PRESSURE_ROUTE_CONFLICT",
                   f"roles.{rid}: default_binding_ref disagrees with the "
                   "routing preferred profile's binding")
        legacy_fb = r.get("fallback_binding_refs")
        legacy_fb = legacy_fb if isinstance(legacy_fb, list) else []
        routed_fb = [_ref_binding(x)
                     for x in (fb_list or [])]
        if "fallback_binding_refs" in r and                 legacy_fb != [b for b in routed_fb if b]:
            _fault(f, "PRESSURE_ROUTE_CONFLICT",
                   f"roles.{rid}: fallback_binding_refs disagree with routing")
        legacy_refs = r.get("binding_refs")
        legacy_refs = legacy_refs if isinstance(legacy_refs, list) else []
        # element-level typing: malformed elements were faulted upstream
        # and never reach the set/hash consumers (F35.10t)
        legacy_refs = [x for x in legacy_refs if isinstance(x, str)]
        needed = {b for b in ([pref_b] + [b for b in routed_fb if b]) if b}
        # presence-based: an EXPLICIT binding_refs projection must cover
        # the routed bindings even when the projection is empty (F35.10r)
        if "binding_refs" in r and \
                not needed.issubset(set(legacy_refs)):
            _fault(f, "PRESSURE_ROUTE_CONFLICT",
                   f"roles.{rid}: binding_refs do not cover the routed "
                   f"bindings {sorted(needed)}")
        strat = (par or {}).get("strategy")
        lanes = (par or {}).get("lanes") or []
        if strat == "single" and lanes:
            _fault(f, "PRESSURE_ROUTE_CONFLICT",
                   f"roles.{rid}: strategy single with parallel lanes")
        if strat == "weighted_fair" and not lanes:
            _fault(f, "PRESSURE_ROUTE_CONFLICT",
                   f"roles.{rid}: strategy weighted_fair without lanes")


def _derive_route(rid, r, bindings):
    """D7 (F35.11r): absent explicit routing derives the role's existing
    chain through default profiles — in BOTH modes. A role with no binding
    material (deterministic maintenance) has no model route, which is not
    a defect."""
    dref = r.get("default_binding_ref")
    refs = [b for b in (r.get("binding_refs") or []) if b in bindings]
    pref = None
    fb = []
    if dref and dref in bindings:
        pref = f"{dref}/default"
        fb = [f"{b}/default" for b in (r.get("fallback_binding_refs") or [])
              if b in bindings]
    elif refs:
        pref = f"{refs[0]}/default"
        fb = [f"{b}/default" for b in refs[1:]]
    else:
        for _pn, pd_ in (r.get("profiles") or {}).items():
            b = (pd_ or {}).get("binding_ref") if isinstance(pd_, dict) \
                else None
            if b in bindings:
                pref = f"{b}/default"
                break
    if not pref:
        return None
    return {"preferred_profile_ref": pref, "fallback_profile_refs": fb,
            "max_concurrent_slots": None, "lanes": [], "strategy": "single",
            "required_capabilities": list(r.get("required_capabilities")
                                          or []),
            "derived": True}


def pressure_view(cfg):
    """Compile a v2 or v3 config into the admission service's internal
    representation (sol D7) WITHOUT rewriting the ledger document (C14).
    Selection is by PRESSURE-SOURCE PRESENCE (F35.11r), never by version
    alone: a v3 config without supply.pressure runs the legacy executable
    model, and a v2 config compiles an executable legacy view (profiles,
    bindings, role chains) instead of an empty stub. Budgets and plane
    budget mappings ride the view so activation refreshes them bound to
    the same policy hash (F35.3r)."""
    version = cfg.get("schema_version")
    suply = cfg.get("supply") or {}
    catalog = cfg.get("catalog") or {}
    bindings = catalog.get("bindings") or {}
    profiles = catalog.get("execution_profiles") or {}
    roles = cfg.get("roles") or {}
    pr = suply.get("pressure") if isinstance(suply.get("pressure"), dict) \
        else None
    adm = suply.get("admission") if isinstance(suply.get("admission"), dict) \
        else None
    out = {"version": version, "profiles": {}, "bindings": {},
           "pools": {}, "role_routing": {}, "baseline": {}, "level": None,
           "profile_aliases": {}}
    out["budgets"] = suply.get("budgets") or {}
    out["plane_budgets"] = {
        pid: {"binding_refs": p.get("binding_refs") or [],
              "owner_policy": p.get("owner_policy"),
              "budget_refs": p.get("budget_refs") or []}
        for pid, p in (cfg.get("planes") or {}).items()}
    out["planes"] = {
        pid: {"owner_policy": p.get("owner_policy"),
              "binding_refs": p.get("binding_refs") or [],
              "health_contract": (p.get("health_contract") or {})
              if isinstance(p.get("health_contract"), dict) else {}}
        for pid, p in (cfg.get("planes") or {}).items()}
    if not pr:
        # ---- legacy executable mode (F35.11r) ----
        g = (adm or {}).get("global_max_concurrent")
        per_family = (adm or {}).get("per_family") or {}
        out["legacy"] = {"global_concurrent_slots": g,
                         "per_family": per_family}

        def _family_limit(rid, r):
            """Original legacy limits: a direct role-id entry, else the
            role's declared profile-families (e.g. dreamers.profiles key
            ux_parity) when per_family names families, not roles. A
            multi-family role takes the MOST RESTRICTIVE matching limit —
            all-zero families must deny, never silently drop (F35.11rrr).
            No invented limits."""
            lim = per_family.get(rid)
            if lim is None:
                fams = [per_family[k] for k in per_family
                        if k in (r.get("profiles") or {})
                        and isinstance(per_family[k], int)]
                if fams:
                    lim = min(fams)
            return lim
        out["global_ceiling"] = g if isinstance(g, int) else None
        lim = out["global_ceiling"]

        def _lim():
            return lim if isinstance(lim, int) else None

        for bid, b in bindings.items():
            out["bindings"][bid] = {
                "ceiling": _lim(), "pools": [],
                "status": (b or {}).get("status"),
                "capabilities": list((b or {}).get("capabilities") or [])}
        explicit_default = {}
        for pid, prof in profiles.items():
            bid = prof.get("binding_ref")
            if prof.get("default_for_binding"):
                explicit_default[bid] = pid
                out["profile_aliases"][f"{bid}/default"] = pid
            out["profiles"][pid] = {
                "binding_ref": bid,
                "reasoning_override": prof.get("reasoning_override"),
                "ceiling": _lim(),
                "status": (bindings.get(bid) or {}).get("status")}
        for bid in bindings:
            if bid not in explicit_default:
                out["profiles"].setdefault(f"{bid}/default", {
                    "binding_ref": bid, "reasoning_override": None,
                    "ceiling": _lim(),
                    "status": (bindings.get(bid) or {}).get("status")})
        for rid, r in roles.items():
            if not isinstance(r, dict):
                continue
            rt = r.get("routing") if isinstance(r.get("routing"), dict) \
                else None
            if rt:
                out["role_routing"][rid] = {
                    "preferred_profile_ref": rt.get("preferred_profile_ref"),
                    "fallback_profile_refs":
                        rt.get("fallback_profile_refs") or [],
                    "max_concurrent_slots": _family_limit(rid, r),
                    "lanes": (rt.get("parallel") or {}).get("lanes") or [],
                    "strategy": (rt.get("parallel") or {}).get(
                        "strategy", "single"),
                    "required_capabilities": list(
                        r.get("required_capabilities") or [])}
            else:
                route = _derive_route(rid, r, bindings)
                if route:
                    route["max_concurrent_slots"] = _family_limit(rid, r)
                    out["role_routing"][rid] = route
        # legacy mode invents NO per-profile target: baseline None means
        # only the ceilings and family/global limits constrain (F35.11rr)
        for pid in out["profiles"]:
            out["baseline"][pid] = None
        for synth in out["profile_aliases"]:
            out["baseline"][synth] = None
        return out
    ceilings = pr.get("ceilings") or {}
    levels = pr.get("levels") or {}
    demand = pr.get("demand") or {}
    control = pr.get("control") or {}
    bceil = ceilings.get("binding_concurrent_slots") or {}
    pceil = ceilings.get("profile_concurrent_slots") or {}
    pools = ceilings.get("pools") or {}
    for pid, pool in pools.items():
        for b in (pool.get("binding_refs") or []):
            entry = out["bindings"].setdefault(
                b, {"ceiling": bceil.get(
                    b, ceilings.get("default_binding_concurrent_slots", 1)),
                    "pools": [],
                    "status": (bindings.get(b) or {}).get("status"),
                    "capabilities": list(
                        (bindings.get(b) or {}).get("capabilities") or [])})
            entry["pools"].append(pid)
    for bid in bindings:
        status = bindings[bid].get("status")
        if bid not in out["bindings"]:
            out["bindings"][bid] = {
                "ceiling": bceil.get(
                    bid, ceilings.get("default_binding_concurrent_slots", 1)),
                "pools": [], "status": status,
                "capabilities": list(
                    (bindings.get(bid) or {}).get("capabilities") or [])}
        synth = f"{bid}/default"
        explicit = [p for p, prof in profiles.items()
                    if prof.get("binding_ref") == bid
                    and prof.get("default_for_binding")]
        if not explicit:
            out["profiles"].setdefault(synth, {"binding_ref": bid,
                                      "reasoning_override": None,
                                      "ceiling": pceil.get(
                                          synth, ceilings.get(
                                              "default_profile_concurrent_slots",
                                              1)),
                                      "status": status})
    for pid, prof in profiles.items():
        bid = prof.get("binding_ref")
        if prof.get("default_for_binding"):
            # the explicit default IS the binding's default — the synth
            # <binding>/default aliases it, never extra capacity (F7)
            out["profile_aliases"][f"{bid}/default"] = pid
        out["profiles"][pid] = {
            "binding_ref": bid,
            "reasoning_override": prof.get("reasoning_override"),
            "ceiling": pceil.get(
                pid, ceilings.get("default_profile_concurrent_slots", 1)),
            "status": (bindings.get(bid) or {}).get("status")}
    for pid, pool in pools.items():
        out["pools"][pid] = {"binding_refs": pool.get("binding_refs") or [],
                             "concurrent_slots": pool.get("concurrent_slots"),
                             "cooldown_after_refusal_seconds":
                                 pool.get("cooldown_after_refusal_seconds", 0)}
    for rid, r in roles.items():
        if not isinstance(r, dict):
            continue
        rt = r.get("routing") if isinstance(r.get("routing"), dict) else None
        if rt:
            out["role_routing"][rid] = {
                "preferred_profile_ref": rt.get("preferred_profile_ref"),
                "fallback_profile_refs": rt.get("fallback_profile_refs") or [],
                "max_concurrent_slots": rt.get("max_concurrent_slots"),
                "lanes": (rt.get("parallel") or {}).get("lanes") or [],
                "strategy": (rt.get("parallel") or {}).get(
                    "strategy", "single"),
                "required_capabilities": list(
                    r.get("required_capabilities") or [])}
        else:
            # D7: absent routing derives the existing role chain (F35.11r)
            route = _derive_route(rid, r, bindings)
            if route:
                out["role_routing"][rid] = route
    lvl = demand.get("level")
    out["level"] = lvl
    lv = levels.get(lvl) or {}
    # level slots and demand overrides resolve through the alias map, so an
    # accepted alias override lands on its canonical profile (F35.7r)
    aliases = out["profile_aliases"]

    def _ck(k):
        return aliases.get(k, k) if isinstance(k, str) else k

    lv_slots = {}
    for k, v in (lv.get("profile_slots") or {}).items():
        lv_slots[_ck(k)] = v
    ovr = {}
    for k, v in (demand.get("profile_slot_overrides") or {}).items():
        ovr[_ck(k)] = v
    default_slots = lv.get("default_profile_slots", 1)
    for pid in out["profiles"]:
        out["baseline"][pid] = ovr.get(pid, lv_slots.get(pid, default_slots))
    for synth, canon in aliases.items():
        out["baseline"][synth] = out["baseline"].get(canon, default_slots)
    out["global_ceiling"] = ceilings.get("global_concurrent_slots")
    out["persistent_agent_edits"] = bool(control.get("persistent_agent_edits"))
    out["agent_level_names"] = control.get("agent_level_names") or []
    out["agent_profile_refs"] = control.get("agent_profile_refs") or []
    out["agent_role_refs"] = control.get("agent_role_refs") or []
    out["lease_max_ttl_seconds"] = control.get("lease_max_ttl_seconds", 900)
    out["lease_max_active_per_issuer"] = control.get(
        "lease_max_active_per_issuer", 1)
    return out


# ------------------------------------------------------------- deny latches

def _deny_path(state_dir):
    return Path(state_dir).expanduser() / "control" / "deny.json"


def deny_list(state_dir=DEFAULT_STATE):
    """-> latches dict. A CORRUPT store returns a deny-everything sentinel:
    unreadable owner stops must contain, never reopen, stopped work."""
    p = _deny_path(state_dir)
    try:
        data = json.loads(p.read_text())
    except FileNotFoundError:
        return {}
    except Exception as e:
        return {"__denies_corrupt__": {
            "reason": f"deny store unreadable — containment until owner repairs: {e!r}"[:180],
            "set_by": "kernel", "set_at": None, "expires_at": None}}
    if not isinstance(data, dict) or not isinstance(data.get("latches", {}), dict):
        return {"__denies_corrupt__": {
            "reason": "deny store malformed — containment until owner repairs",
            "set_by": "kernel", "set_at": None, "expires_at": None}}
    now = _now()
    latches = {}
    for k, v in (data.get("latches") or {}).items():
        if not isinstance(v, dict):
            return {"__denies_corrupt__": {
                "reason": f"deny store has malformed latch {k!r} — containment",
                "set_by": "kernel", "set_at": None, "expires_at": None}}
        exp = v.get("expires_at")
        if exp and exp <= now:
            continue
        latches[k] = v
    return latches


def deny_set(scope_id, reason, ttl_seconds=None, set_by="owner",
             state_dir=DEFAULT_STATE):
    p = _deny_path(state_dir)
    latches = deny_list(state_dir)
    latch = {"reason": str(reason), "set_by": set_by, "set_at": _now(),
             "expires_at": None}
    if ttl_seconds:
        latch["expires_at"] = (datetime.now(timezone.utc)
                               + timedelta(seconds=int(ttl_seconds))
                               ).strftime("%Y-%m-%dT%H:%M:%SZ")
    latches[str(scope_id)] = latch
    atomic_write_json(p, {"latches": latches, "updated_at": _now()})
    return latch


def deny_clear(scope_id, state_dir=DEFAULT_STATE):
    p = _deny_path(state_dir)
    latches = deny_list(state_dir)
    if str(scope_id) not in latches:
        return False
    del latches[str(scope_id)]
    atomic_write_json(p, {"latches": latches, "updated_at": _now()})
    return True


# ---------------------------------------------------------- activation store

def _paths(state_dir):
    sd = Path(state_dir).expanduser()
    return {
        "state": sd,
        "candidates": sd / "config" / "candidates",
        "releases": sd / "config" / "releases",
        "active": sd / "config" / "active.json",
        "checkpoint": sd / "config" / "checkpoint.json",
        "receipts": sd / "config" / "receipts",
        "control": sd / "control",
    }


def active_head(state_dir=DEFAULT_STATE):
    """-> {'content_hash','epoch','revision','change_id','activated_at'} or None."""
    try:
        data = json.loads(_paths(state_dir)["active"].read_text())
    except Exception:
        return None
    if not isinstance(data, dict) or not HEX64.fullmatch(str(data.get("content_hash") or "")):
        return None
    return data


def _read_release(P, h):
    rj = P["releases"] / h / "release.json"
    try:
        obj = json.loads(rj.read_text())
    except Exception:
        return None
    if not isinstance(obj, dict):
        return None
    if content_hash(obj) != h:
        return None
    return obj


def _committed_hashes(state_dir):
    """-> set of canonical_hashes with a journal acceptance record, or None
    when no journal exists yet (all sealed releases count as committed)."""
    j = Path(state_dir).expanduser() / "ledger" / "config" / "journal.jsonl"
    try:
        rows = [json.loads(x) for x in j.read_text().splitlines() if x.strip()]
    except FileNotFoundError:
        return set()  # no journal: nothing is committed (the append IS the commit point)
    except Exception:
        return set()  # unreadable journal: trust nothing
    return {r.get("canonical_hash") for r in rows if r.get("canonical_hash")}


def load(state_dir=DEFAULT_STATE, live_yaml=DEFAULT_YAML):
    """Structured LoadResult. Precedence: verified checkpoint (active) ->
    newest re-validating release (last_good) -> containment_only. Deny
    latches intersect EVERY mode, so owner stops survive any recovery."""
    P = _paths(state_dir)
    faults: list = []
    result = {"mode": "containment_only", "config": None, "revision": None,
              "source": "none", "faults": faults, "denials": {}, "loaded_at": _now()}

    head = active_head(state_dir)
    if head is None:
        faults.append({"code": "checkpoint_corrupt",
                       "detail": "active.json missing or corrupt"})
    else:
        cfg = _read_release(P, head["content_hash"])
        if cfg is None:
            # fallback: checkpoint.json (independent copy, PyYAML-free)
            try:
                cfg = json.loads(P["checkpoint"].read_text())
                if content_hash(cfg) != head["content_hash"]:
                    cfg = None
            except Exception:
                cfg = None
            if cfg is not None:
                result["source"] = "checkpoint"
            else:
                faults.append({"code": "checkpoint_corrupt",
                               "detail": f"release {head['content_hash'][:12]} unreadable and checkpoint mismatch"})
        else:
            result["source"] = "release"
        if cfg is not None:
            vf = validate(cfg)
            if vf:
                faults.extend(vf)
                cfg = None
            else:
                result["mode"] = "active"
                result["config"] = cfg
                result["revision"] = {"epoch": head.get("epoch"),
                                      "revision": head.get("revision"),
                                      "content_hash": head["content_hash"],
                                      "change_id": head.get("change_id")}

    if result["mode"] != "active":
        # last_good: newest COMMITTED release — a journal record must vouch
        # for the hash; sealed-but-uncommitted seals are not recoverable state
        committed = _committed_hashes(state_dir)
        best = None
        if P["releases"].is_dir():
            for d in sorted(P["releases"].iterdir()):
                if not d.is_dir() or not HEX64.fullmatch(d.name):
                    continue
                if committed is not None and d.name not in committed:
                    continue
                cfg = _read_release(P, d.name)
                if cfg is None or validate(cfg):
                    continue
                key = ((cfg.get("identity") or {}).get("authority_epoch") or 0,
                       (cfg.get("identity") or {}).get("revision") or 0)
                if best is None or key > best[0]:
                    best = (key, cfg, d.name)
        if best is not None:
            _, cfg, h = best
            result["mode"] = "last_good"
            result["config"] = cfg
            result["source"] = f"release:{h[:12]}"
            ident = cfg.get("identity") or {}
            result["revision"] = {"epoch": ident.get("authority_epoch"),
                                  "revision": ident.get("revision"),
                                  "content_hash": h,
                                  "change_id": ident.get("change_id")}
            faults.append({"code": "checkpoint_corrupt",
                           "detail": "activated head unusable; recovered newest valid release"})
        else:
            faults.append({"code": "no_usable_config",
                           "detail": "containment-only: retain work, hold admissions, defer novel decisions"})

    result["denials"] = deny_list(state_dir)
    return result


def accept(cfg_or_bytes, *, proposed_by, authority_class="owner",
           parent_expected=None, state_dir=DEFAULT_STATE,
           change_id=None, reason="", commit_point="all",
           issuer_role_ref=None, principal_verifier=None,
           verified_principal=None):
    """Validation -> authorization -> seal -> CAS activation. Returns a
    receipt dict. NEVER mutates activated state on any rejection.
    Agent-class proposals require a VERIFIED principal through the
    injected verifier (F35.4r): a claimed issuer_role_ref string is input
    to the pure helper, never authorization; the role is derived from the
    verifier's result. No verification interface means refusal."""
    P = _paths(state_dir)
    if isinstance(cfg_or_bytes, (bytes, bytearray)):
        raw = bytes(cfg_or_bytes)
    elif isinstance(cfg_or_bytes, str):
        raw = cfg_or_bytes.encode("utf-8")
    else:
        raw = None
    cid = change_id or str(uuid.uuid4())
    receipt = {"change_id": cid, "proposed_by": proposed_by,
               "authority_class": authority_class, "at": _now(),
               "parent_expected": parent_expected, "decision": None,
               "content_hash": None, "faults": []}

    def finish(decision, extra=None):
        receipt["decision"] = decision
        if extra:
            receipt.update(extra)
        atomic_write_json(P["receipts"] / f"{cid}.json", receipt)
        return receipt

    if raw is not None:
        cdir = P["candidates"] / cid
        cdir.mkdir(parents=True, exist_ok=True)
        atomic_write(cdir / "candidate.yaml", raw)

    if raw is not None:
        cfg, pf = parse_yaml_bytes(raw)
        if pf:
            receipt["faults"].append(pf)
            return finish("rejected_invalid")
    else:
        cfg = cfg_or_bytes
        if not isinstance(cfg, dict):
            receipt["faults"].append({"code": "not_a_mapping", "detail": "accept() needs bytes or dict"})
            return finish("rejected_invalid")

    vf = validate(cfg)
    if vf:
        receipt["faults"].extend(vf)
        return finish("rejected_invalid")

    h = content_hash(cfg)
    receipt["content_hash"] = h
    head = active_head(state_dir)
    head_hash = head["content_hash"] if head else None

    if head_hash == h:
        return finish("already_active")

    parent = (cfg.get("identity") or {}).get("parent_hash") or GENESIS_HASH
    expected = parent_expected if parent_expected is not None else (head_hash or GENESIS_HASH)
    if parent != expected:
        return finish("rejected_stale", {"detail": f"parent {parent[:12]} != chain head {expected[:12]}"})

    # authority: owner-only semantic paths; agent-class proposals carry a
    # verified principal whose resolved role IS the authorization role
    if authority_class != "owner" and head is not None:
        old = _read_release(P, head_hash)
        if old is not None:
            if principal_verifier is None or verified_principal is None:
                receipt["faults"].append({
                    "code": "invariant_violation",
                    "detail": "agent-class accept requires a verified "
                              "principal through an injected verifier; a "
                              "claimed role is not authorization"})
                return finish("rejected_unauthorized")
            resolved = principal_verifier(verified_principal)
            if not resolved or (issuer_role_ref is not None and
                                issuer_role_ref != resolved):
                receipt["faults"].append({
                    "code": "invariant_violation",
                    "detail": f"principal {verified_principal!r} does not "
                              f"verify as {issuer_role_ref!r}"})
                return finish("rejected_unauthorized")
            bad = _agent_unauthorized(old, cfg, issuer_role_ref=resolved)
            if bad:
                receipt["faults"].append({
                    "code": "invariant_violation",
                    "detail": f"unauthorized semantic paths for {authority_class}: {bad[:8]}"})
                return finish("rejected_unauthorized")

    # seal the immutable release, then CAS the activation pointer.
    # commit_point: "seal-only" (ledger writes the journal first and re-enters
    # with "activate-only" — the journal append is the commit point, sol t7 #3);
    # "all" (default) = seal + activate for direct local activation.
    rdir = P["releases"] / h
    rdir.mkdir(parents=True, exist_ok=True)
    if raw is not None:
        atomic_write(rdir / "release.yaml", raw)
    atomic_write_json(rdir / "release.json", cfg)
    if commit_point == "seal-only":
        return finish("accepted", {"sealed": True, "revision": {
            "epoch": (cfg.get("identity") or {}).get("authority_epoch"),
            "revision": (cfg.get("identity") or {}).get("revision"),
            "content_hash": h, "change_id": cid}})

    ident = cfg.get("identity") or {}
    pointer = {"content_hash": h,
               "epoch": ident.get("authority_epoch"),
               "revision": ident.get("revision"),
               "change_id": cid,
               "activated_at": _now()}
    tmp = P["active"].with_name("active.json.cas-" + uuid.uuid4().hex[:8])
    atomic_write_json(tmp, pointer)
    cur = None
    try:
        cur = json.loads(P["active"].read_text())
    except Exception:
        cur = None
    if cur is not None and cur.get("content_hash") != head_hash:
        # lost the CAS race; discard our pointer file
        try:
            tmp.unlink()
        except OSError:
            pass
        return finish("rejected_stale", {"detail": "chain head moved during accept; retry against new head"})
    os.replace(tmp, P["active"])
    atomic_write_json(P["checkpoint"], cfg)
    return finish("accepted", {"activated": True, "revision": {
        "epoch": pointer["epoch"], "revision": pointer["revision"],
        "content_hash": h, "change_id": cid}})


def ack(consumer, state_dir=DEFAULT_STATE, revision_hash=None):
    """A consumer acknowledges the revision it has actually LOADED and is
    operating on — the pointer alone proves nothing (sol t9#6: a recovered
    last_good must not be acked as the pointer's revision). Callers pass the
    content_hash from their LoadResult; None refuses (no blind acks)."""
    if not revision_hash:
        return {"ok": False, "detail": "ack requires the loaded revision hash "
                                       "(pointer observation is not consumption)"}
    d = Path(state_dir).expanduser() / "config" / "acks"
    d.mkdir(parents=True, exist_ok=True)
    atomic_write_json(d / f"{consumer}.json",
                      {"consumer": consumer, "content_hash": revision_hash,
                       "at": _now()})
    return {"ok": True, "revision": revision_hash[:12]}


def acks(state_dir=DEFAULT_STATE):
    """-> {consumer: content_hash or None} for every ack file present."""
    d = Path(state_dir).expanduser() / "config" / "acks"
    out = {}
    if d.is_dir():
        for p_ in sorted(d.glob("*.json")):
            try:
                rec = json.loads(p_.read_text())
                out[p_.stem] = rec.get("content_hash")
            except Exception:
                out[p_.stem] = None
    return out


def status(state_dir=DEFAULT_STATE):
    r = load(state_dir)
    head = active_head(state_dir)
    rel = sorted([d.name for d in (_paths(state_dir)["releases"]).iterdir()]
                 ) if (_paths(state_dir)["releases"]).is_dir() else []
    return {"mode": r["mode"], "revision": r["revision"], "head": head,
            "releases": rel, "denials": list(r["denials"]),
            "faults": r["faults"], "acks": acks(state_dir)}


# -------------------------------------------------------------- CLI surface

def _cli(argv):
    import argparse
    ap = argparse.ArgumentParser(prog="swarmconfig",
                                 description="swarm-config v2 kernel")
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check").add_argument("path")
    acc = sub.add_parser("accept")
    acc.add_argument("path")
    acc.add_argument("--proposed-by", default="owner")
    acc.add_argument("--authority", default="owner", choices=["owner", "agent"])
    acc.add_argument("--parent", default=None)
    sub.add_parser("status")
    d = sub.add_parser("deny")
    d.add_argument("action", choices=["set", "clear", "list"])
    d.add_argument("scope", nargs="?")
    d.add_argument("--reason", default="owner stop/drain")
    d.add_argument("--ttl-seconds", type=int, default=None)
    sub.add_parser("recover-drill")
    a = ap.parse_args(argv)
    sd = DEFAULT_STATE

    if a.cmd == "check":
        cfg, pf = parse_yaml_bytes(Path(a.path).read_bytes())
        if pf:
            print(json.dumps({"valid": False, "faults": [pf]}))
            return 2
        f = validate(cfg)
        print(json.dumps({"valid": not f, "faults": f,
                          "content_hash": content_hash(cfg) if not f else None}))
        return 0 if not f else 2
    if a.cmd == "accept":
        r = accept(Path(a.path).read_bytes(), proposed_by=a.proposed_by,
                   authority_class=a.authority, parent_expected=a.parent,
                   state_dir=sd)
        print(json.dumps(r, indent=2))
        return 0 if r["decision"] == "accepted" else 2
    if a.cmd == "accept":
        r = accept(Path(a.path).read_bytes(), proposed_by=a.proposed_by,
                   authority_class=a.authority, parent_expected=a.parent,
                   state_dir=sd)
        print(json.dumps(r, indent=2))
        return 0 if r["decision"] == "accepted" else 2
    if a.cmd == "status":
        print(json.dumps(status(sd), indent=2))
        return 0
    if a.cmd == "deny":
        if a.action == "set":
            print(json.dumps(deny_set(a.scope, a.reason, a.ttl_seconds, state_dir=sd)))
        elif a.action == "clear":
            print(json.dumps({"cleared": deny_clear(a.scope, state_dir=sd)}))
        else:
            print(json.dumps(deny_list(sd), indent=2))
        return 0
    if a.cmd == "recover-drill":
        r = load(sd)
        print(json.dumps(r, indent=2, default=str))
        return 0 if r["mode"] != "containment_only" else 3
    return 1


if __name__ == "__main__":
    import sys
    sys.exit(_cli(sys.argv[1:]))
