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
KNOWN_MAJOR = 2
GENESIS_HASH = "0" * 64
HEX64 = re.compile(r"^[0-9a-f]{64}$")

# semantic-diff authority: proposals from non-owner principals may not touch
# these POLICY paths (prefix match over the policy view — the identity
# envelope is excluded: parent/revision/change_id are re-stamped by accept).
# Owner proposals may change anything.
OWNER_ONLY_PREFIXES = (
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
)


def _policy_view(cfg):
    return {k: v for k, v in cfg.items() if k != "identity"}


def _agent_unauthorized(old, new):
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
    return sorted(set(bad))

FAULT_CODES = (
    "yaml_syntax", "duplicate_key", "size_bound", "unsupported_major",
    "unknown_field", "type_mismatch", "invalid_enum", "ref_missing",
    "invariant_violation", "not_a_mapping", "checkpoint_corrupt",
    "no_usable_config", "parser_missing",
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
        cfg = yaml.load(data.decode("utf-8"), Loader=_DupCheckLoader)
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
    if v > KNOWN_MAJOR:
        _fault(f, "unsupported_major",
               f"schema_version {v} newer than kernel major {KNOWN_MAJOR}; update the kernel first")
        return f
    if v != KNOWN_MAJOR:
        _fault(f, "unsupported_major", f"schema_version {v} is not {KNOWN_MAJOR}")
        return f

    for k in cfg:
        if k not in _TOP_KEYS:
            _fault(f, "unknown_field", f"top-level .{k}")
    for sec, spec in _TOP_KEYS.items():
        if sec in cfg and spec is dict and not isinstance(cfg[sec], dict):
            _fault(f, "type_mismatch", f"{sec}: expected mapping, got {type(cfg[sec]).__name__}")

    ident = cfg.get("identity") or {}
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
    _keys(f, "fleet", fleet, {"authority_host", "hosts", "dispatch_host", "failover"})
    hosts = fleet.get("hosts")
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
    _keys(f, "catalog", cat, {"harnesses", "providers", "accounts", "plans", "bindings"})
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
    for plid, pl in planes.items():
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
                if r not in bindings:
                    _fault(f, "ref_missing", f"planes.{plid}.binding_refs -> {r}")
        hc = pl.get("health_contract") or {}
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
               "repair_role_ref"},
              {"kind"})
        if r.get("kind") not in ROLE_KINDS:
            _fault(f, "invalid_enum", f"roles.{rid}.kind: {r.get('kind')!r}")
        if r.get("kind") == "deterministic" and r.get("model") not in (None, "none"):
            _fault(f, "invariant_violation", f"roles.{rid}: deterministic role must not declare a model")
        for key in ("binding_refs", "fallback_binding_refs"):
            for ref in (r.get(key) or []):
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
        for ref in (r.get("binding_refs") or []):
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
           "learned_patch_policy_ref"})
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

    return f


# ------------------------------------------------------------- deny latches

def _deny_path(state_dir):
    return Path(state_dir).expanduser() / "control" / "deny.json"


def deny_list(state_dir=DEFAULT_STATE):
    p = _deny_path(state_dir)
    try:
        data = json.loads(p.read_text())
    except FileNotFoundError:
        return {}
    except Exception:
        return {}
    now = _now()
    latches = {}
    for k, v in (data.get("latches") or {}).items():
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
        # last_good: newest re-validating release regardless of head pointer
        best = None
        if P["releases"].is_dir():
            for d in sorted(P["releases"].iterdir()):
                if not d.is_dir() or not HEX64.fullmatch(d.name):
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
           change_id=None, reason=""):
    """Validation -> authorization -> seal -> CAS activation. Returns a
    receipt dict. NEVER mutates activated state on any rejection."""
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

    # authority: owner-only semantic paths
    if authority_class != "owner" and head is not None:
        old = _read_release(P, head_hash)
        if old is not None:
            bad = _agent_unauthorized(old, cfg)
            if bad:
                receipt["faults"].append({
                    "code": "invariant_violation",
                    "detail": f"unauthorized semantic paths for {authority_class}: {bad[:8]}"})
                return finish("rejected_unauthorized")

    # seal the immutable release, then CAS the activation pointer
    rdir = P["releases"] / h
    rdir.mkdir(parents=True, exist_ok=True)
    if raw is not None:
        atomic_write(rdir / "release.yaml", raw)
    atomic_write_json(rdir / "release.json", cfg)

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


def status(state_dir=DEFAULT_STATE):
    r = load(state_dir)
    head = active_head(state_dir)
    rel = sorted([d.name for d in (_paths(state_dir)["releases"]).iterdir()]
                 ) if (_paths(state_dir)["releases"]).is_dir() else []
    return {"mode": r["mode"], "revision": r["revision"], "head": head,
            "releases": rel, "denials": list(r["denials"]),
            "faults": r["faults"]}


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
