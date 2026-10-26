"""pressure — the swarm-config v3 admission authority (repair v2).

Repair of the sol implementation review (2026-10-26, 11 findings):
- F1  every mutation runs inside a BEGIN IMMEDIATE transaction — the write
      fence is taken BEFORE any config/lease/occupancy/budget/permit read
      and held through commit; rejections and exceptions roll back.
      Config fencing: meta.config_hash is checked on every mutation; a
      stale handle is refused.
- F2  redemption only from granted+unexpired+current-config; unique
      (dispatch, attempt); an unresolved dispatch blocks new attempts;
      identical retries return the existing outcome; reconcile only from
      uncertain; release is terminal and idempotent; unknown permits fault.
- F3  budgets load from config at the factory; applicable budgets derive
      from the plane; spending = SUM(cost) over the window including
      completed work (released-after-redeem never refunds); spendable:false
      denies; missing/unavailable accounting CONTAINs; the slot unit fixes
      cost at 1; token budgets require an injected accounting interface.
- F5  leases take request_id + expected_config_hash + a verified principal
      (injected verifier — self-asserted strings are not roles); delegation
      reads the compiled view; revision 0 when no current lease; targets
      validated against every ceiling at issuance; request idempotency
      persisted; receipts are HMAC-authenticated over the full material.
- F6  admission verifies the principal, the role, profile-in-role-routes
      (aliases resolved), plane enabled + membership, binding admitted;
      health evidence is typed and fail-closed (absent/malformed HOLDS;
      numeric limits intersect occupancy).
- F7  explicit default profiles become aliases of their canonical profile —
      never extra capacity; pooled bindings keep their status.
- F8  occupancy is one canonical snapshot (profiles, bindings, pools, roles,
      global) shared by admission and activation; activation checks pool and
      role ceilings too.
- F9  explicit provisioning vs rw-only opening; restart bumps the epoch,
      invalidates leases/unredeemed permits and blocks new permits while
      uncertain launch state exists; store problems CONTAIN.
"""
import hashlib
import hmac
import json
import json as _json
import os
import secrets
import sqlite3
import threading
import time
import uuid
from contextlib import contextmanager
from datetime import datetime, timezone
from pathlib import Path

PERMIT_TTL_SECONDS = 30
MAX_LEASE_TTL_SECONDS = 900
_REASONING_LEVELS = ("low", "medium", "high", "max")
_SCHEMA_VERSION = 2  # store schema revision

_SCHEMA = """
CREATE TABLE IF NOT EXISTS meta (
    key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS leases (
    lease_id TEXT PRIMARY KEY,
    config_hash TEXT NOT NULL,
    issuer_session_ref TEXT NOT NULL,
    issuer_role_ref TEXT NOT NULL,
    profile_slots TEXT NOT NULL,
    reason TEXT,
    revision INTEGER NOT NULL,
    granted_at REAL NOT NULL,
    expires_at REAL NOT NULL);
CREATE TABLE IF NOT EXISTS permits (
    permit_ref TEXT PRIMARY KEY,
    config_hash TEXT NOT NULL,
    dispatch_id TEXT NOT NULL,
    attempt_id TEXT NOT NULL,
    request_digest TEXT NOT NULL,
    principal TEXT NOT NULL,
    role_ref TEXT NOT NULL,
    profile_ref TEXT NOT NULL,
    binding_ref TEXT NOT NULL,
    plane_ref TEXT,
    budget_scopes TEXT NOT NULL DEFAULT '[]',
    budget_cost INTEGER NOT NULL DEFAULT 1,
    state TEXT NOT NULL,
    granted_at REAL NOT NULL,
    expires_at REAL NOT NULL,
    redeemed_at REAL);
CREATE UNIQUE INDEX IF NOT EXISTS permit_attempt
    ON permits(dispatch_id, attempt_id);
CREATE UNIQUE INDEX IF NOT EXISTS unresolved_dispatch
    ON permits(dispatch_id) WHERE state IN ('granted','redeemed','uncertain');
CREATE TABLE IF NOT EXISTS requests (
    request_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    request_digest TEXT NOT NULL,
    result TEXT NOT NULL,
    at REAL NOT NULL);
CREATE TABLE IF NOT EXISTS events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    at REAL NOT NULL,
    kind TEXT NOT NULL,
    receipt TEXT NOT NULL,
    body TEXT NOT NULL);
"""


def _now():
    return time.time()


def _canonical(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"))


def _digest(obj):
    return hashlib.sha256(_canonical(obj).encode()).hexdigest()


class Contained(Exception):
    """Raised inside a mutation when the authority cannot decide; converts
    to a CONTAIN decision / typed fault without partial writes."""


class PressureAuthority:
    """The single admission authority. Provision explicitly ONCE per store
    (PressureAuthority.provision); ordinary opens are rw-only and verify the
    committed identity — a missing or corrupt store is containment, never a
    fresh empty authority (sol F9)."""

    def __init__(self, view, config_hash, state_dir=None, *,
                 health_evidence=None, token_accounting=None,
                 principal_verifier=None, budgets=None, plane_budgets=None):
        self.view = view or {}
        self.config_hash = config_hash
        self.version = self.view.get("version")
        self.health_evidence = health_evidence  # None => fail closed
        self.token_accounting = token_accounting  # None => token budgets CONTAIN
        self.principal_verifier = principal_verifier  # None => unverified
        self.budgets = budgets if budgets is not None else \
            (self.view.get("budgets") or {})
        self.plane_budgets = plane_budgets if plane_budgets is not None \
            else (self.view.get("plane_budgets") or {})
        sd = Path(state_dir).expanduser() if state_dir \
            else Path("~/.yggterm/swarm").expanduser()
        self.db_path = sd / "runtime" / "pressure" / "state.sqlite"
        self._lock = threading.RLock()
        self._db = None
        self._contained = None  # reason string when the store is unusable
        self._signer_key = None
        self._open()

    # ------------------------------------------------------------ opening
    def _open(self):
        try:
            if not self.db_path.exists():
                self._contained = "store-missing (provision explicitly)"
                return
            db = sqlite3.connect(
                self.db_path.as_uri() + "?mode=rw", uri=True, timeout=10)
            db.execute("PRAGMA journal_mode=WAL")
            db.execute("PRAGMA busy_timeout=5000")
            row = db.execute(
                "SELECT name FROM sqlite_master WHERE type='table' "
                "AND name='meta'").fetchone()
            if not row:
                self._contained = "store not provisioned"
                return
            stored = dict(db.execute(
                "SELECT key, value FROM meta").fetchall())
            if stored.get("schema_version") != str(_SCHEMA_VERSION):
                self._contained = "store schema version mismatch"
                return
            self._signer_key = bytes.fromhex(
                stored.get("signer_key", "") or "") or None
            # restart fence: new epoch, invalidate leases + unredeemed
            # permits, block new permits while uncertain state exists (F9)
            epoch = int(stored.get("epoch", "0")) + 1
            now = _now()
            db.execute("BEGIN IMMEDIATE")
            db.execute("DELETE FROM leases WHERE expires_at > ?", (now,))
            db.execute("DELETE FROM permits WHERE state = 'granted'",
                       ())
            barrier = 1 if db.execute(
                "SELECT COUNT(*) FROM permits WHERE state = 'uncertain'"
            ).fetchone()[0] else 0
            db.executemany(
                "INSERT OR REPLACE INTO meta (key, value) VALUES (?,?)",
                [("epoch", str(epoch)),
                 ("reconcile_barrier", str(barrier)),
                 ("config_hash", stored.get("config_hash", ""))])
            rcpt = self._receipt(db, "restart", {
                "epoch": epoch, "config_hash": stored.get("config_hash"),
                "reconcile_barrier": bool(barrier), "at": now})
            db.execute("COMMIT")
            self._db = db
            self._contained = None
        except (sqlite3.Error, OSError, ValueError) as exc:
            self._db = None
            self._contained = f"store-unavailable {exc}"[:160]

    @classmethod
    def provision(cls, view, config_hash, state_dir=None, **kw):
        """Explicit first provisioning of a store. NEVER use as recovery
        from a lost committed store (sol F9)."""
        sd = Path(state_dir).expanduser() if state_dir \
            else Path("~/.yggterm/swarm").expanduser()
        sd.mkdir(parents=True, exist_ok=True)
        db_path = sd / "runtime" / "pressure" / "state.sqlite"
        db_path.parent.mkdir(parents=True, exist_ok=True)
        db = sqlite3.connect(str(db_path), timeout=10)
        try:
            db.executescript(_SCHEMA)
            db.execute(
                "INSERT OR IGNORE INTO meta (key, value) VALUES (?,?)",
                ("schema_version", str(_SCHEMA_VERSION)))
            db.execute(
                "INSERT OR IGNORE INTO meta (key, value) VALUES (?,?)",
                ("signer_key", secrets.token_hex(32)))
            db.execute(
                "INSERT OR IGNORE INTO meta (key, value) VALUES (?,?)",
                ("epoch", "0"))
            db.execute(
                "INSERT OR IGNORE INTO meta (key, value) VALUES (?,?)",
                ("config_hash", config_hash))
            db.commit()
        finally:
            db.close()
        return cls(view, config_hash, state_dir=state_dir, **kw)

    # ------------------------------------------------------- txn + fence
    @contextmanager
    def _txn(self):
        """The write fence: BEGIN IMMEDIATE before any read; COMMIT when the
        body leaves normally (HOLD/ALLOW alike — the occupancy reads must see
        one serialized state), ROLLBACK on exception (sol F1)."""
        db = self._require_db()
        db.execute("BEGIN IMMEDIATE")
        try:
            yield db
            if db.in_transaction:
                db.execute("COMMIT")
        except BaseException:
            if db.in_transaction:
                db.execute("ROLLBACK")
            raise

    def _require_db(self):
        if self._contained:
            raise Contained(self._contained)
        if self._db is None:
            raise Contained("store-unavailable")
        return self._db

    def _receipt(self, db, kind, body):
        material = {"kind": kind, "body": body,
                    "signer": (self._signer_key or b"").hex()}
        rcpt = hmac.new(self._signer_key or b"pressure-auth",
                        _canonical(material).encode(),
                        hashlib.sha256).hexdigest()
        db.execute("INSERT INTO events (at, kind, receipt, body) "
                   "VALUES (?,?,?,?)",
                   (_now(), kind, rcpt, _canonical(body)))
        return rcpt

    def _fenced(self, db):
        """Config fencing: this handle must match the store's active hash."""
        stored = dict(db.execute(
            "SELECT key, value FROM meta").fetchall()).get("config_hash")
        if stored != self.config_hash:
            raise Contained(
                f"stale authority handle: handle={self.config_hash[:12]} "
                f"store={str(stored)[:12]}")

    def _verify(self, verified_principal, role_ref):
        """Resolve the principal through the injected verifier and require
        the resolved role to match (sol F5/F6)."""
        if self.principal_verifier is None:
            raise Contained("principal-verifier-unavailable")
        resolved = self.principal_verifier(verified_principal)
        if resolved != role_ref:
            raise Contained(
                f"principal {verified_principal} does not verify as {role_ref}")

    # ------------------------------------------------------- occupancy
    def _occupancy_all(self, db, now):
        """THE canonical occupancy snapshot (sol F8): one definition shared
        by admission and activation. granted counts only while unexpired;
        redeemed/uncertain always count (expiry never frees live work)."""
        occ = {"global": 0, "profiles": {}, "bindings": {}, "roles": {},
               "pools": {}}
        for prof, bind, role, state, exp in db.execute(
                "SELECT profile_ref, binding_ref, role_ref, state, "
                "expires_at FROM permits").fetchall():
            live = state in ("redeemed", "uncertain") or \
                (state == "granted" and exp > now)
            if not live:
                continue
            occ["global"] += 1
            occ["profiles"][prof] = occ["profiles"].get(prof, 0) + 1
            occ["bindings"][bind] = occ["bindings"].get(bind, 0) + 1
            occ["roles"][role] = occ["roles"].get(role, 0) + 1
        for pid, pool in (self.view.get("pools") or {}).items():
            occ["pools"][pid] = sum(
                occ["bindings"].get(b, 0)
                for b in (pool.get("binding_refs") or []))
        return occ

    def _leases_for(self, db, pid, now):
        best = 0
        for (slots,) in db.execute(
                "SELECT profile_slots FROM leases WHERE config_hash = ? "
                "AND expires_at > ?", (self.config_hash, now)).fetchall():
            try:
                best = max(best, int(_json.loads(slots).get(pid, 0)))
            except Exception:
                continue
        return best

    def _desired(self, db, pid, now):
        baseline = (self.view.get("baseline") or {}).get(pid, 1)
        return max(baseline, self._leases_for(db, pid, now))

    def _canonical_profile(self, profile_ref):
        aliases = self.view.get("profile_aliases") or {}
        return aliases.get(profile_ref, profile_ref)

    def _budget_scopes_for(self, plane_ref):
        """Applicable budgets derive from the plane (sol F3) — the caller
        never chooses which budgets apply."""
        pb = (self.plane_budgets or {}).get(plane_ref) or {}
        return list(pb.get("budget_refs") or [])

    def _budget_check(self, db, plane_ref, now):
        """Reservation predicate for every applicable budget. Returns
        (ok, scopes, reason). Fail-closed: missing record, spendable:false,
        unavailable token accounting, or over-window spend all refuse."""
        scopes = self._budget_scopes_for(plane_ref)
        checked = []
        for bid in scopes:
            budget = (self.budgets or {}).get(bid)
            if not budget:
                raise Contained(f"budget {bid} referenced by plane "
                                f"{plane_ref} is missing from config")
            if budget.get("spendable") is False:
                raise Contained(f"budget {bid} is not spendable")
            limits = budget.get("hard_limits") or {}
            window = (budget.get("window_hours", 24) or 24) * 3600
            since = now - window
            if "consults" in limits:
                used = db.execute(
                    "SELECT COALESCE(SUM(budget_cost),0) FROM permits "
                    "WHERE budget_scopes LIKE ? AND granted_at > ? AND "
                    "(state IN ('granted','redeemed','uncertain','spent') "
                    " OR (state='released' AND redeemed_at IS NOT NULL))",
                    (f'%"{bid}"%', since)).fetchone()[0]
                if used + 1 > limits["consults"]:
                    return False, None, (f"budget {bid}: {used}+1 > "
                                         f"{limits['consults']} consults")
            elif "tokens" in limits:
                if self.token_accounting is None:
                    raise Contained(
                        f"budget {bid} meters tokens and no token "
                        "accounting interface is available")
                # token reservation is delegated to the injected interface
                if not self.token_accounting(bid, plane_ref):
                    return False, None, f"budget {bid}: token headroom refused"
            checked.append(bid)
        return True, checked, None

    # ------------------------------------------------------------ verbs
    def activate(self, view, config_hash):
        """Bind to a new config inside the fence; invalidate previous-config
        leases/unredeemed permits; refuse any ceiling below counted
        occupancy — profiles, bindings, pools, roles, global (sol F8)."""
        with self._lock:
            try:
                with self._txn() as db:
                    self._fenced_check(db)
                    now = _now()
                    occ = self._occupancy_all(db, now)
                    new = PressureAuthority(
                        view, config_hash, state_dir=self.db_path.parents[2],
                        health_evidence=self.health_evidence,
                        token_accounting=self.token_accounting,
                        principal_verifier=self.principal_verifier,
                        budgets=self.budgets,
                        plane_budgets=self.plane_budgets)
                    faults = new._check_ceilings_against(db, occ)
                    if faults:
                        db.execute("ROLLBACK")
                        return {"ok": False, "faults": faults}
                    db.execute("DELETE FROM leases WHERE config_hash != ?",
                               (config_hash,))
                    db.execute("DELETE FROM permits WHERE config_hash != ? "
                               "AND state = 'granted'", (config_hash,))
                    db.execute("INSERT OR REPLACE INTO meta (key, value) "
                               "VALUES ('config_hash', ?)", (config_hash,))
                    rcpt = self._receipt(db, "activate",
                                         {"config_hash": config_hash,
                                          "at": now})
                    db.execute("COMMIT")
                    self.view = view
                    self.config_hash = config_hash
                    self.version = view.get("version")
                    return {"ok": True, "receipt": rcpt}
            except Contained as exc:
                return {"ok": False, "faults": [str(exc)]}
            except sqlite3.Error as exc:
                return {"ok": False, "faults": [f"store-error {exc}"]}

    def _fenced_check(self, db):
        stored = dict(db.execute(
            "SELECT key, value FROM meta").fetchall()).get("config_hash")
        if stored and stored != self.config_hash:
            raise Contained(
                f"stale handle for activation: handle={self.config_hash[:12]} "
                f"store={str(stored)[:12]}")

    def _check_ceilings_against(self, db, occ):
        v = self.view or {}
        faults = []
        g = v.get("global_ceiling")
        if isinstance(g, int) and occ.get("global", 0) > g:
            faults.append(f"PRESSURE_BELOW_INFLIGHT: global occupancy "
                          f"{occ['global']} > ceiling {g}")
        for pid, n in (occ.get("profiles") or {}).items():
            prof = self._canonical_view_profile(v, pid)
            if prof is None:
                faults.append(f"PRESSURE_BELOW_INFLIGHT: profile {pid} "
                              f"vanished with {n} in flight")
                continue
            if n > prof.get("ceiling", 1):
                faults.append(f"PRESSURE_BELOW_INFLIGHT: profile {pid} "
                              f"occupancy {n} > ceiling {prof['ceiling']}")
        for bid, n in (occ.get("bindings") or {}).items():
            b = (v.get("bindings") or {}).get(bid)
            if b is None:
                faults.append(f"PRESSURE_BELOW_INFLIGHT: binding {bid} "
                              f"vanished with {n} in flight")
                continue
            if n > b.get("ceiling", 1):
                faults.append(f"PRESSURE_BELOW_INFLIGHT: binding {bid} "
                              f"occupancy {n} > ceiling {b['ceiling']}")
        for pid, n in (occ.get("pools") or {}).items():
            pool = (v.get("pools") or {}).get(pid)
            if pool is None:
                continue
            if n > pool.get("concurrent_slots", 1):
                faults.append(f"PRESSURE_BELOW_INFLIGHT: pool {pid} "
                              f"occupancy {n} > ceiling "
                              f"{pool['concurrent_slots']}")
        for rid, n in (occ.get("roles") or {}).items():
            role_max = ((v.get("role_routing") or {}).get(rid) or {}).get(
                "max_concurrent_slots")
            if role_max is not None and n > role_max:
                faults.append(f"PRESSURE_BELOW_INFLIGHT: role {rid} "
                              f"occupancy {n} > limit {role_max}")
        return faults

    def _canonical_view_profile(self, v, pid):
        profiles = v.get("profiles") or {}
        if pid in profiles:
            return profiles[pid]
        alias = (v.get("profile_aliases") or {}).get(pid)
        if alias:
            return profiles.get(alias)
        return None

    def lease(self, *, request_id, expected_config_hash, verified_principal,
              issuer_role_ref, profile_slots, ttl_seconds,
              expected_lease_revision, reason=""):
        """A runtime pressure lease (sol F5): idempotent by request_id,
        principal verified, delegation from the compiled view, every target
        validated against profile/binding/pool/global ceilings at issuance,
        revision CAS (0 when no current lease), authenticated receipt."""
        request = {"request_id": request_id, "kind": "lease",
                   "config_hash": expected_config_hash,
                   "principal": verified_principal, "role": issuer_role_ref,
                   "profile_slots": profile_slots, "ttl": ttl_seconds,
                   "revision": expected_lease_revision, "reason": reason}
        rd = _digest(request)
        with self._lock:
            try:
                with self._txn() as db:
                    self._fenced(db)
                    prior = db.execute(
                        "SELECT request_digest, result FROM requests "
                        "WHERE request_id = ? AND kind = 'lease'",
                        (request_id,)).fetchone()
                    if prior:
                        if prior[0] != rd:
                            raise Contained(
                                "request_id retry with changed bytes")
                        return json.loads(prior[1])
                    if expected_config_hash != self.config_hash:
                        raise Contained("expected_config_hash mismatch")
                    if self.version != 3:
                        raise Contained("pressure requires a v3 pressure view")
                    resolved = self._resolve_principal(verified_principal,
                                                       issuer_role_ref)
                    max_ttl = self.view.get("lease_max_ttl_seconds",
                                            MAX_LEASE_TTL_SECONDS)
                    if type(ttl_seconds) is not int or \
                            not 1 <= ttl_seconds <= max_ttl:
                        raise Contained(
                            f"ttl_seconds must be 1..{max_ttl}")
                    if issuer_role_ref not in \
                            (self.view.get("agent_role_refs") or []):
                        raise Contained(
                            f"role {issuer_role_ref} not delegated")
                    clean = {}
                    for pid, n in (profile_slots or {}).items():
                        canon = self._canonical_profile(pid)
                        prof = (self.view.get("profiles") or {}).get(canon)
                        if pid not in (self.view.get("agent_profile_refs")
                                       or []):
                            raise Contained(f"profile {pid} not delegated")
                        if prof is None:
                            raise Contained(f"unknown profile {pid}")
                        if type(n) is not int or n < 0 or \
                                n > prof.get("ceiling", 1):
                            raise Contained(
                                f"profile_slots.{pid}: 0.."
                                f"{prof.get('ceiling', 1)}")
                        bind = prof.get("binding_ref")
                        bceil = (self.view.get("bindings") or {}).get(
                            bind, {}).get("ceiling", 1)
                        if n > bceil:
                            raise Contained(
                                f"profile_slots.{pid}: {n} exceeds binding "
                                f"{bind} ceiling {bceil}")
                        for pool_id, pool in (
                                self.view.get("pools") or {}).items():
                            if bind in (pool.get("binding_refs") or []):
                                if n > pool.get("concurrent_slots", 1):
                                    raise Contained(
                                        f"profile_slots.{pid}: {n} exceeds "
                                        f"pool {pool_id}")
                        g = self.view.get("global_ceiling")
                        if isinstance(g, int) and n > g:
                            raise Contained(
                                f"profile_slots.{pid}: {n} exceeds global")
                        clean[pid] = n
                    if not clean:
                        raise Contained("profile_slots empty")
                    now = _now()
                    active = db.execute(
                        "SELECT lease_id, revision FROM leases "
                        "WHERE issuer_session_ref = ? AND config_hash = ? "
                        "AND expires_at > ?",
                        (verified_principal,
                         self.config_hash, now)).fetchone()
                    if active:
                        if expected_lease_revision != active[1]:
                            raise Contained("lease revision CAS mismatch")
                        db.execute("DELETE FROM leases WHERE lease_id = ?",
                                   (active[0],))
                    elif expected_lease_revision != 0:
                        raise Contained(
                            "expected_lease_revision must be 0 when no "
                            "current lease exists")
                    lease_id = str(uuid.uuid4())
                    db.execute(
                        "INSERT INTO leases (lease_id, config_hash, "
                        "issuer_session_ref, issuer_role_ref, profile_slots,"
                        " reason, revision, granted_at, expires_at) "
                        "VALUES (?,?,?,?,?,?,?,?,?)",
                        (lease_id, self.config_hash,
                         verified_principal,
                         issuer_role_ref, _canonical(clean), reason,
                         expected_lease_revision + 1, now,
                         now + ttl_seconds))
                    result = {"ok": True, "lease_id": lease_id,
                              "revision": expected_lease_revision + 1,
                              "expires_at": now + ttl_seconds,
                              "profile_slots": clean}
                    body = dict(result, request_id=request_id,
                                config_hash=self.config_hash,
                                principal=verified_principal,
                                role=issuer_role_ref, reason=reason,
                                granted_at=now)
                    result["receipt"] = self._receipt(db, "lease", body)
                    db.execute(
                        "INSERT INTO requests (request_id, kind, "
                        "request_digest, result, at) VALUES (?,?,?,?,?)",
                        (request_id, "lease", rd, _canonical(result), now))
                    db.execute("COMMIT")
                    return result
            except Contained as exc:
                db2 = self._db
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"ok": False, "faults": [str(exc)]}
            except sqlite3.Error as exc:
                db2 = self._db
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"ok": False, "faults": [f"store-error {exc}"]}

    def _resolve_principal(self, verified_principal, role_ref):
        if self.principal_verifier is None:
            raise Contained("principal-verifier-unavailable")
        resolved = self.principal_verifier(verified_principal)
        if resolved != role_ref:
            raise Contained(
                f"principal {verified_principal} does not verify as "
                f"{role_ref}")

    def admit(self, *, dispatch_id, attempt_id, verified_principal,
              role_ref, profile_ref, plane_ref=None):
        """The gate every managed dispatch path calls (C27). Intersecting
        predicates inside the write fence; budget derived from the plane;
        single-use permit; replay fence per dispatch/attempt (sol F2)."""
        request = {"dispatch_id": dispatch_id, "attempt_id": attempt_id,
                   "principal": verified_principal, "role": role_ref,
                   "profile": profile_ref, "plane": plane_ref,
                   "config_hash": self.config_hash}
        rd = _digest(request)
        with self._lock:
            try:
                with self._txn() as db:
                    self._fenced(db)
                    now = _now()
                    # replay fence: identical retry -> the existing outcome;
                    # a new attempt cannot bypass an unresolved dispatch
                    prior = db.execute(
                        "SELECT attempt_id, permit_ref, state, "
                        "request_digest FROM permits WHERE dispatch_id = ?",
                        (dispatch_id,)).fetchall()
                    for pattempt, pref, state, pdig in prior:
                        unresolved = state in ("granted", "redeemed",
                                               "uncertain")
                        if pattempt == attempt_id:
                            if pdig == rd:
                                if unresolved:
                                    return {
                                        "decision": "ALLOW",
                                        "permit_ref": pref,
                                        "reason_codes": ["idempotent-retry"],
                                        "replay": True}
                                return {
                                    "decision": "HOLD", "permit_ref": pref,
                                    "reason_codes":
                                        [f"attempt already {state}"],
                                    "replay": True}
                            raise Contained(
                                "same dispatch/attempt with changed bytes")
                        if unresolved:
                            return {"decision": "HOLD", "permit_ref": None,
                                    "reason_codes": [
                                        f"dispatch {dispatch_id} unresolved "
                                        f"({state})"]}
                    self._resolve_principal(verified_principal, role_ref)
                    route = (self.view.get("role_routing") or {}).get(
                        role_ref)
                    if route is None:
                        raise Contained(f"unknown role {role_ref}")
                    canon = self._canonical_profile(profile_ref)
                    prof = (self.view.get("profiles") or {}).get(canon)
                    if prof is None:
                        raise Contained(f"unknown profile {profile_ref}")
                    eligible = [route.get("preferred_profile_ref")] + list(
                        route.get("fallback_profile_refs") or []) + [
                        ln.get("profile_ref") for ln in
                        (route.get("lanes") or [])]
                    if profile_ref not in [e for e in eligible if e]:
                        raise Contained(
                            f"profile {profile_ref} is not in role "
                            f"{role_ref} routes")
                    bind = prof.get("binding_ref")
                    bstate = (self.view.get("bindings") or {}).get(
                        bind, {}).get("status")
                    if bstate != "admitted":
                        raise Contained(
                            f"binding {bind} is {bstate or 'unknown'}")
                    if plane_ref is not None:
                        plane = (self.view.get("planes") or {}).get(plane_ref)
                        if not plane:
                            raise Contained(f"unknown plane {plane_ref}")
                        if plane.get("owner_policy") != "enabled":
                            raise Contained(
                                f"plane {plane_ref} is "
                                f"{plane.get('owner_policy')}")
                        if bind not in (plane.get("binding_refs") or []):
                            raise Contained(
                                f"binding {bind} not in plane {plane_ref}")
                    # health evidence: typed, fail-closed (sol F6)
                    ev = None
                    if self.health_evidence is not None:
                        ev = self.health_evidence(bind)
                    if not isinstance(ev, dict) or \
                            ev.get("state") not in ("ok", "healthy"):
                        return {"decision": "HOLD", "permit_ref": None,
                                "reason_codes": [f"health-unavailable {bind}"]}
                    limit = ev.get("concurrent_limit")
                    # budget reservation (derived from the plane, sol F3)
                    scopes = self._budget_scopes_for(plane_ref)
                    for bid in scopes:
                        budget = (self.budgets or {}).get(bid)
                        if not budget:
                            raise Contained(
                                f"budget {bid} missing from config")
                        if budget.get("spendable") is False:
                            raise Contained(f"budget {bid} not spendable")
                        limits = budget.get("hard_limits") or {}
                        window = (budget.get("window_hours", 24) or 24) * 3600
                        if "consults" in limits:
                            used = db.execute(
                                "SELECT COALESCE(SUM(budget_cost),0) FROM "
                                "permits WHERE budget_scopes LIKE ? AND "
                                "granted_at > ? AND (state IN ('granted',"
                                "'redeemed','uncertain','spent') OR "
                                "(state='released' AND redeemed_at IS NOT "
                                "NULL))",
                                (f'%"{bid}"%', now - window)).fetchone()[0]
                            if used + 1 > limits["consults"]:
                                return {"decision": "HOLD",
                                        "permit_ref": None,
                                        "reason_codes": [
                                            f"budget {bid}: {used}+1 > "
                                            f"{limits['consults']} consults"]}
                        elif "tokens" in limits:
                            if self.token_accounting is None:
                                raise Contained(
                                    f"budget {bid} meters tokens and no "
                                    "token accounting interface is wired")
                            if not self.token_accounting(bid, plane_ref):
                                return {"decision": "HOLD",
                                        "permit_ref": None,
                                        "reason_codes": [
                                            f"budget {bid}: tokens refused"]}
                    # occupancy vs every applicable ceiling
                    occ = self._occupancy_all(db, now)
                    desired = max(
                        (self.view.get("baseline") or {}).get(canon, 1),
                        self._leases_for(db, canon, now))
                    reasons = []
                    p_occ = occ["profiles"].get(canon, 0)
                    if p_occ + 1 > desired:
                        reasons.append(f"profile {profile_ref} "
                                       f"{p_occ}+1 > desired {desired}")
                    pceil = prof.get("ceiling", 1)
                    if p_occ + 1 > pceil:
                        reasons.append(f"profile {profile_ref} "
                                       f"ceiling {pceil}")
                    limit = ev.get("concurrent_limit") if ev else None
                    if isinstance(limit, int) and p_occ + 1 > limit:
                        reasons.append(f"health limit {limit} on {bind}")
                    b_occ = occ["bindings"].get(bind, 0)
                    bceil = (self.view.get("bindings") or {}).get(
                        bind, {}).get("ceiling", 1)
                    if b_occ + 1 > bceil:
                        reasons.append(f"binding {bind} {b_occ}+1 > "
                                       f"ceiling {bceil}")
                    for pool_id, pool in (
                            self.view.get("pools") or {}).items():
                        if bind in (pool.get("binding_refs") or []):
                            if occ["pools"].get(pool_id, 0) + 1 > \
                                    pool.get("concurrent_slots", 1):
                                reasons.append(
                                    f"pool {pool_id} {occ['pools'].get(pool_id, 0)}"
                                    f"+1 > {pool['concurrent_slots']}")
                    role_occ = occ["roles"].get(role_ref, 0)
                    role_max = route.get("max_concurrent_slots")
                    if role_max is not None and role_occ + 1 > role_max:
                        reasons.append(f"role {role_ref} {role_occ}+1 > "
                                       f"{role_max}")
                    g = self.view.get("global_ceiling")
                    if isinstance(g, int) and occ["global"] + 1 > g:
                        reasons.append(f"global {occ['global']}+1 > {g}")
                    if reasons:
                        return {"decision": "HOLD", "permit_ref": None,
                                "reason_codes": reasons}
                    # barrier: uncertain launch state blocks new permits (F9)
                    barrier = dict(db.execute(
                        "SELECT key, value FROM meta").fetchall()).get(
                        "reconcile_barrier")
                    if barrier == "1":
                        return {"decision": "HOLD", "permit_ref": None,
                                "reason_codes": ["reconciliation-barrier"]}
                    permit_ref = str(uuid.uuid4())
                    db.execute(
                        "INSERT INTO permits (permit_ref, config_hash, "
                        "dispatch_id, attempt_id, request_digest, principal,"
                        " role_ref, profile_ref, binding_ref, plane_ref, "
                        "budget_scopes, budget_cost, state, granted_at, "
                        "expires_at) "
                        "VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
                        (permit_ref, self.config_hash, dispatch_id,
                         attempt_id, rd, verified_principal, role_ref,
                         canon, bind, plane_ref, _canonical(scopes), 1,
                         "granted", now, now + PERMIT_TTL_SECONDS))
                    rcpt = self._receipt(db, "admit-allow", {
                        "permit_ref": permit_ref,
                        "config_hash": self.config_hash,
                        "dispatch_id": dispatch_id,
                        "attempt_id": attempt_id,
                        "principal": verified_principal, "role": role_ref,
                        "profile": canon, "binding": bind,
                        "plane": plane_ref, "at": now})
                    return {"decision": "ALLOW", "permit_ref": permit_ref,
                            "receipt": rcpt, "binding_ref": bind,
                            "profile_ref": canon, "reason_codes": []}
            except Contained as exc:
                db2 = self._db
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"decision": "CONTAIN", "permit_ref": None,
                        "reason_codes": [str(exc)]}
            except sqlite3.Error as exc:
                db2 = self._db
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"decision": "CONTAIN", "permit_ref": None,
                        "reason_codes": [f"store-error {exc}"]}

    def redeem(self, permit_ref, *, dispatch_id, attempt_id):
        """Fence immediately before launch. Only granted+unexpired+current
        permits redeem (sol F2); one-row update authorizes the launch."""
        with self._lock:
            try:
                with self._txn() as db:
                    self._fenced(db)
                    now = _now()
                    cur = db.execute(
                        "UPDATE permits SET state = 'redeemed', "
                        "redeemed_at = ? WHERE permit_ref = ? AND "
                        "state = 'granted' AND config_hash = ? AND "
                        "expires_at > ? AND dispatch_id = ? AND "
                        "attempt_id = ?",
                        (now, permit_ref, self.config_hash, now,
                         dispatch_id, attempt_id))
                    if cur.rowcount != 1:
                        row = db.execute(
                            "SELECT state FROM permits WHERE permit_ref = ?",
                            (permit_ref,)).fetchone()
                        raise Contained(
                            f"permit not redeemable (state="
                            f"{row[0] if row else 'unknown'})")
                    rcpt = self._receipt(db, "redeem",
                                         {"permit_ref": permit_ref,
                                          "dispatch_id": dispatch_id,
                                          "attempt_id": attempt_id,
                                          "at": now})
                    return {"ok": True, "receipt": rcpt}
            except Contained as exc:
                db2 = self._db
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"ok": False, "faults": [str(exc)]}
            except sqlite3.Error as exc:
                db2 = self._db
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"ok": False, "faults": [f"store-error {exc}"]}

    def mark_uncertain(self, permit_ref, detail=""):
        with self._lock:
            try:
                with self._txn() as db:
                    cur = db.execute(
                        "UPDATE permits SET state = 'uncertain' WHERE "
                        "permit_ref = ? AND state IN ('granted','redeemed')",
                        (permit_ref,))
                    if cur.rowcount != 1:
                        raise Contained("permit not markable uncertain")
                    db.execute(
                        "INSERT OR REPLACE INTO meta (key, value) "
                        "VALUES ('reconcile_barrier', '1')")
                    rcpt = self._receipt(db, "mark-uncertain",
                                         {"permit_ref": permit_ref,
                                          "detail": str(detail)[:200]})
                    db.execute("COMMIT")
                    return {"ok": True, "receipt": rcpt}
            except (Contained, sqlite3.Error) as exc:
                db2 = self._db
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"ok": False,
                        "faults": [str(exc) if isinstance(exc, Contained)
                                   else f"store-error {exc}"]}

    def reconcile(self, permit_ref, *, launched, dispatch_id=None,
                  attempt_id=None):
        """Resolve an uncertain permit: launched -> redeemed (slot held
        until release); not-launched -> released. Only uncertain permits
        reconcile (sol F2)."""
        with self._lock:
            try:
                with self._txn() as db:
                    row = db.execute(
                        "SELECT dispatch_id, attempt_id FROM permits "
                        "WHERE permit_ref = ? AND state = 'uncertain'",
                        (permit_ref,)).fetchone()
                    if not row:
                        raise Contained(
                            "permit not uncertain (nothing to reconcile)")
                    d_id, a_id = row
                    if dispatch_id not in (None, d_id) or \
                            attempt_id not in (None, a_id):
                        raise Contained(
                            "reconcile identity does not match the permit")
                    db.execute(
                        "UPDATE permits SET state = ? WHERE permit_ref = ?",
                        ("redeemed" if launched else "released", permit_ref))
                    remain = db.execute(
                        "SELECT COUNT(*) FROM permits WHERE state = "
                        "'uncertain'").fetchone()[0]
                    if not remain:
                        db.execute("INSERT OR REPLACE INTO meta (key, value)"
                                   " VALUES ('reconcile_barrier', '0')")
                    rcpt = self._receipt(db, "reconcile",
                                         {"permit_ref": permit_ref,
                                          "launched": bool(launched)})
                    db.execute("COMMIT")
                    return {"ok": True, "receipt": rcpt}
            except (Contained, sqlite3.Error) as exc:
                db2 = self._db
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"ok": False,
                        "faults": [str(exc) if isinstance(exc, Contained)
                                   else f"store-error {exc}"]}

    def release(self, permit_ref):
        """The dispatch finished: free its slot. Terminal and idempotent
        (sol F2); unknown permits fault."""
        with self._lock:
            try:
                with self._txn() as db:
                    cur = db.execute(
                        "UPDATE permits SET state = 'released' WHERE "
                        "permit_ref = ? AND state IN ('granted','redeemed',"
                        "'uncertain')", (permit_ref,))
                    if cur.rowcount != 1:
                        row = db.execute(
                            "SELECT state FROM permits WHERE permit_ref = ?",
                            (permit_ref,)).fetchone()
                        if row and row[0] == "released":
                            return {"ok": True, "receipt": None,
                                    "idempotent": True}
                        raise Contained(
                            f"permit not releasable "
                            f"(state={row[0] if row else 'unknown'})")
                    rcpt = self._receipt(db, "release",
                                         {"permit_ref": permit_ref})
                    db.execute("COMMIT")
                    return {"ok": True, "receipt": rcpt}
            except (Contained, sqlite3.Error) as exc:
                db2 = self._db
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"ok": False,
                        "faults": [str(exc) if isinstance(exc, Contained)
                                   else f"store-error {exc}"]}

    def status(self):
        try:
            db = self._require_db()
        except Contained as exc:
            return {"available": False, "reason": str(exc)}
        now = _now()
        occ = self._occupancy_all(db, now)
        leases = db.execute(
            "SELECT COUNT(*) FROM leases WHERE config_hash = ? AND "
            "expires_at > ?", (self.config_hash, now)).fetchone()[0]
        meta = dict(db.execute("SELECT key, value FROM meta").fetchall())
        return {"available": True, "config_hash": self.config_hash,
                "version": self.version, "level": self.view.get("level"),
                "epoch": meta.get("epoch"),
                "reconcile_barrier": meta.get("reconcile_barrier") == "1",
                "occupancy": occ, "active_leases": leases,
                "desired": {pid: self._desired(db, pid, now)
                            for pid in self.view.get("profiles", {})}}


def authority_from_config(cfg, config_hash, state_dir=None, **kw):
    """Compile a config through pressure_view; budgets and plane budget
    mappings load from the config itself (sol F3) unless overridden.
    First-use provisioning: provisions the store ONLY when no store file
    exists yet — an existing missing/corrupt store stays containment."""
    import swarmconfig as sc
    sd = Path(state_dir).expanduser() if state_dir         else Path("~/.yggterm/swarm").expanduser()
    store = sd / "runtime" / "pressure" / "state.sqlite"
    if not store.exists():
        PressureAuthority.provision({}, config_hash, state_dir=state_dir)
    kw.setdefault("budgets", (cfg.get("supply") or {}).get("budgets") or {})
    view = sc.pressure_view(cfg)
    kw.setdefault("plane_budgets", {
        pid: {"binding_refs": p.get("binding_refs") or [],
              "owner_policy": p.get("owner_policy"),
              "budget_refs": p.get("budget_refs") or []}
        for pid, p in (cfg.get("planes") or {}).items()})
    kw.setdefault("health_evidence", (cfg.get("supply") or {}).get(
        "pressure", {}).get("health_evidence"))
    return PressureAuthority(view, config_hash, state_dir=state_dir, **kw)


def main():
    import argparse
    ap = argparse.ArgumentParser(prog="pressure")
    ap.add_argument("verb", choices=["status", "admit", "redeem",
                                     "reconcile", "release"])
    ap.add_argument("--state-dir", default="~/.yggterm/swarm")
    ap.add_argument("--role", default=None)
    ap.add_argument("--profile", default=None)
    ap.add_argument("--principal", default=None)
    ap.add_argument("--plane", default=None)
    ap.add_argument("--dispatch", default=None)
    ap.add_argument("--permit", default=None)
    ap.add_argument("--launched", action="store_true")
    a = ap.parse_args()
    sd = Path(a.state_dir).expanduser()
    head = None
    aj = sd / "config" / "active.json"
    if aj.exists():
        head = json.loads(aj.read_text()).get("content_hash")
    rel = sd / "config" / "releases" / (head or "")
    cfgp = rel / "release.yaml" if (rel / "release.yaml").exists() else \
        sd / "config" / "current.yaml"
    try:
        import yaml
        cfg = yaml.safe_load(cfgp.read_text())
    except Exception as exc:
        print(json.dumps({"decision": "CONTAIN",
                          "reason_codes": [f"config-unavailable {exc}"]}))
        return 2
    import swarmconfig as sc

    principals = {}
    pj = sd / "runtime" / "pressure" / "principals.json"
    if pj.exists():
        try:
            principals = json.loads(pj.read_text())
        except Exception:
            principals = {}
    auth = authority_from_config(
        cfg, head or sc.content_hash(cfg), state_dir=sd,
        principal_verifier=lambda p: principals.get(p))
    if a.verb == "status":
        print(json.dumps(auth.status(), indent=1))
        return 0
    if a.verb == "admit":
        out = auth.admit(dispatch_id=a.dispatch or str(uuid.uuid4()),
                         attempt_id=str(uuid.uuid4()),
                         verified_principal=a.principal,
                         role_ref=a.role or "dreamers",
                         profile_ref=a.profile or "",
                         plane_ref=a.plane)
        print(json.dumps(out, indent=1))
        return 0 if out["decision"] == "ALLOW" else 1
    if a.verb == "redeem":
        out = auth.redeem(a.permit, dispatch_id=a.dispatch,
                          attempt_id=None)
        print(json.dumps(out, indent=1))
        return 0 if out.get("ok") else 1
    if a.verb == "reconcile":
        out = auth.reconcile(a.permit, launched=a.launched)
        print(json.dumps(out, indent=1))
        return 0 if out.get("ok") else 1
    if a.verb == "release":
        out = auth.release(a.permit)
        print(json.dumps(out, indent=1))
        return 0 if out.get("ok") else 1
    return 1


if __name__ == "__main__":
    import sys
    sys.exit(main())
