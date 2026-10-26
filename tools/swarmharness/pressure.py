"""pressure — the swarm-config v3 admission authority (sol-certified design
2026-10-26, D5/D6).

ONE authority owns pressure decisions. Its transactional store (sqlite, WAL)
holds leases, single-use launch permits, budget window counters, and an
append-only event log with receipts. Everything is bound to the current
config's semantic hash: a config activation invalidates outstanding leases
and unredeemed permits; activating a ceiling below counted occupancy is
refused (drain first).

Intersecting predicates per candidate launch (D6) — routing eligibility,
profile desired(p) = max(baseline, valid lease targets), profile/binding/
pool/role/global ceilings, fresh health, budget reservation — any failure
HOLDs (or REROUTEs/CONTAINs per reason); nothing overwrites anything else.
Uncertain redemptions keep their reservation until reconciled. Corrupt or
unavailable state means CONTAIN, never reconstructed capacity (C26).
"""
import hashlib
import json
import os
import sqlite3
import threading
import time
import uuid
from datetime import datetime, timezone
from pathlib import Path

PERMIT_TTL_SECONDS = 30
_DECISIONS = ("ALLOW", "HOLD", "REROUTE", "CONTAIN")
_PERMIT_LIVE = ("granted", "redeemed", "uncertain")

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
    role_ref TEXT NOT NULL,
    profile_ref TEXT NOT NULL,
    binding_ref TEXT NOT NULL,
    budget_scope TEXT,
    budget_cost INTEGER NOT NULL DEFAULT 1,
    state TEXT NOT NULL,
    granted_at REAL NOT NULL,
    expires_at REAL NOT NULL,
    redeemed_at REAL);
CREATE TABLE IF NOT EXISTS events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    at REAL NOT NULL,
    kind TEXT NOT NULL,
    receipt TEXT NOT NULL,
    body TEXT NOT NULL);
"""


def _now():
    return time.time()


def _receipt(body):
    return hashlib.sha256(
        json.dumps(body, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()


def _event(db, kind, body):
    receipt = _receipt({"kind": kind, "body": body})
    db.execute("INSERT INTO events (at, kind, receipt, body) VALUES (?,?,?,?)",
               (_now(), kind, receipt, json.dumps(body, sort_keys=True)))
    return receipt


class PressureAuthority:
    """The single admission authority. All decisions fail closed: any store
    error -> CONTAIN with reason 'authority-unavailable' (C26)."""

    def __init__(self, view, config_hash, state_dir=None,
                 health_ok=None, budgets=None, plane_budgets=None):
        self.view = view or {}
        self.config_hash = config_hash
        self.version = self.view.get("version")
        self.health_ok = health_ok or (lambda binding: True)
        self.budgets = budgets or {}
        self.plane_budgets = plane_budgets or {}
        sd = Path(state_dir).expanduser() if state_dir \
            else Path("~/.yggterm/swarm").expanduser()
        self.db_path = sd / "runtime" / "pressure" / "state.sqlite"
        self.db_path.parent.mkdir(parents=True, exist_ok=True)
        self._lock = threading.RLock()
        self._db = None
        self._open()

    # ------------------------------------------------------------ store
    def _open(self):
        try:
            db = sqlite3.connect(str(self.db_path), timeout=10)
            db.execute("PRAGMA journal_mode=WAL")
            db.execute("PRAGMA busy_timeout=5000")
            db.executescript(_SCHEMA)
            db.commit()
            self._db = db
        except sqlite3.Error:
            self._db = None  # containment: every verb now CONTAINs

    def _conn(self):
        if self._db is None:
            self._open()
        return self._db

    # ------------------------------------------------------- activation
    def activate(self, view, config_hash):
        """Bind the authority to a new config. Invalidates previous-config
        leases and unredeemed permits (C22). Refuses to activate any hard
        ceiling below currently counted occupancy (PRESSURE_BELOW_INFLIGHT,
        C22) — drain first; live work is never killed."""
        with self._lock:
            db = self._conn()
            if db is None:
                return {"ok": False, "faults": ["authority-unavailable"]}
            occ = self._occupancy_all()
            new = PressureAuthority(view, config_hash,
                                    state_dir=self.db_path.parents[2],
                                    health_ok=self.health_ok,
                                    budgets=self.budgets,
                                    plane_budgets=self.plane_budgets)
            faults = new._check_ceilings_against(occ)
            if faults:
                return {"ok": False, "faults": faults}
            now = _now()
            db.execute("DELETE FROM leases WHERE config_hash != ?",
                       (config_hash,))
            db.execute("DELETE FROM permits WHERE config_hash != ? "
                       "AND state = 'granted'", (config_hash,))
            db.execute("INSERT OR REPLACE INTO meta (key, value) "
                       "VALUES ('config_hash', ?)", (config_hash,))
            rcpt = _event(db, "activate", {"config_hash": config_hash,
                                           "at": now})
            db.commit()
            self.view = view
            self.config_hash = config_hash
            return {"ok": True, "receipt": rcpt,
                    "invalidated_leases": True, "invalidated_permits": True}

    def _check_ceilings_against(self, occ):
        """-> faults when a new-view ceiling < counted occupancy (C22)."""
        v = self.view or {}
        faults = []
        g = v.get("global_ceiling")
        if isinstance(g, int) and occ.get("global", 0) > g:
            faults.append(f"PRESSURE_BELOW_INFLIGHT: global occupancy "
                          f"{occ['global']} > ceiling {g}")
        for pid, n in (occ.get("profiles") or {}).items():
            prof = (v.get("profiles") or {}).get(pid)
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
        return faults

    # ------------------------------------------------------- accounting
    def _occupancy_all(self):
        """Counted occupancy from live permits: granted (unexpired),
        redeemed, and uncertain — expiry never frees running or uncertain
        work (C21). -> {global: n, profiles: {pid: n}, bindings: {bid: n}}"""
        db = self._conn()
        occ = {"global": 0, "profiles": {}, "bindings": {}}
        if db is None:
            return occ
        now = _now()
        for row in db.execute(
                "SELECT profile_ref, binding_ref, state, expires_at "
                "FROM permits").fetchall():
            prof, bind, state, exp = row
            live = state in ("redeemed", "uncertain") or \
                (state == "granted" and exp > now)
            if not live:
                continue
            occ["global"] += 1
            occ["profiles"][prof] = occ["profiles"].get(prof, 0) + 1
            occ["bindings"][bind] = occ["bindings"].get(bind, 0) + 1
        return occ

    def _leases_for(self, pid):
        """Combined lease target for a profile: max across valid leases
        (never addition — repeated leases do not multiply capacity, C18)."""
        db = self._conn()
        if db is None:
            return 0
        now = _now()
        best = 0
        for (slots,) in db.execute(
                "SELECT profile_slots FROM leases WHERE config_hash = ? "
                "AND expires_at > ?", (self.config_hash, now)).fetchall():
            try:
                best = max(best, int(json.loads(slots).get(pid, 0)))
            except Exception:
                continue
        return best

    def _desired(self, pid):
        baseline = (self.view.get("baseline") or {}).get(pid, 1)
        return max(baseline, self._leases_for(pid))

    def _budget_headroom(self, scope, cost):
        """Consult-class window counter: permits granted in the budget's
        window vs its consults hard limit. Token limits are metered
        externally and never block here (C19 semantics: slots are not
        tokens). Missing scope/budget -> headroom assumed (config governs)."""
        db = self._conn()
        if db is None or not scope:
            return True, None
        budget = None
        for b in (self.budgets or {}).values():
            if b.get("scope") == scope:
                budget = b
                break
        if not budget:
            return True, None
        consults = (budget.get("hard_limits") or {}).get("consults")
        if not isinstance(consults, int):
            return True, None
        window = budget.get("window_hours", 24) * 3600
        since = _now() - window
        used = db.execute(
            "SELECT COUNT(*) FROM permits WHERE budget_scope = ? "
            "AND granted_at > ? AND state IN ('granted','redeemed',"
            "'uncertain','spent')", (scope, since)).fetchone()[0]
        if used + cost > consults:
            return False, f"budget {scope}: {used}+{cost} > {consults} consults"
        return True, None

    # ------------------------------------------------------------ verbs
    def lease(self, issuer_session_ref, issuer_role_ref, profile_slots,
              ttl_seconds, expected_lease_revision, reason="",
              agent_role_refs=(), agent_profile_refs=()):
        """A runtime pressure lease: absolute per-profile targets, max-TTL
        bounded, one active lease per issuer, replacement via revision CAS
        (C21). Targets bounded by the profile ceilings; undelegated
        profiles refused (C20)."""
        with self._lock:
            db = self._conn()
            if db is None:
                return {"ok": False, "faults": ["authority-unavailable"]}
            if self.version != 3:
                return {"ok": False, "faults": ["pressure requires v3 config"]}
            max_ttl = self.view.get("lease_max_ttl_seconds", 900)
            if not isinstance(ttl_seconds, int) or ttl_seconds < 1 \
                    or ttl_seconds > max_ttl:
                return {"ok": False,
                        "faults": [f"ttl_seconds must be 1..{max_ttl}"]}
            if issuer_role_ref not in tuple(agent_role_refs):
                return {"ok": False,
                        "faults": [f"role {issuer_role_ref} not delegated"]}
            clean = {}
            for pid, n in (profile_slots or {}).items():
                if pid not in tuple(agent_profile_refs):
                    return {"ok": False,
                            "faults": [f"profile {pid} not delegated"]}
                ceil = (self.view.get("profiles") or {}).get(pid, {}).get(
                    "ceiling", 1)
                if not isinstance(n, int) or isinstance(n, bool) or n < 0 \
                        or n > ceil:
                    return {"ok": False,
                            "faults": [f"profile_slots.{pid}: 0..{ceil}"]}
                clean[pid] = n
            if not clean:
                return {"ok": False, "faults": ["profile_slots empty"]}
            now = _now()
            active = db.execute(
                "SELECT lease_id, revision FROM leases "
                "WHERE issuer_session_ref = ? AND config_hash = ? "
                "AND expires_at > ?",
                (issuer_session_ref, self.config_hash, now)).fetchone()
            if active:
                if expected_lease_revision != active[1]:
                    return {"ok": False,
                            "faults": ["lease revision CAS mismatch"]}
                db.execute("DELETE FROM leases WHERE lease_id = ?",
                           (active[0],))
            lease_id = str(uuid.uuid4())
            db.execute(
                "INSERT INTO leases (lease_id, config_hash, "
                "issuer_session_ref, issuer_role_ref, profile_slots, reason, "
                "revision, granted_at, expires_at) VALUES (?,?,?,?,?,?,?,?,?)",
                (lease_id, self.config_hash, issuer_session_ref,
                 issuer_role_ref, json.dumps(clean, sort_keys=True), reason,
                 expected_lease_revision + 1, now, now + ttl_seconds))
            body = {"lease_id": lease_id, "config_hash": self.config_hash,
                    "issuer": issuer_session_ref, "role": issuer_role_ref,
                    "profile_slots": clean, "ttl": ttl_seconds, "at": now}
            rcpt = _event(db, "lease", body)
            db.commit()
            return {"ok": True, "lease_id": lease_id,
                    "revision": expected_lease_revision + 1,
                    "expires_at": now + ttl_seconds, "receipt": rcpt}

    def admit(self, dispatch_id, attempt_id, role_ref, profile_ref,
              plane_ref=None, budget_scope=None, budget_cost=1):
        """The gate every managed dispatch path calls (C27). Intersecting
        predicates (D6); any store problem -> CONTAIN."""
        with self._lock:
            db = self._conn()
            if db is None:
                return {"decision": "CONTAIN", "permit_ref": None,
                        "reason_codes": ["authority-unavailable"]}
            now = _now()
            prof = (self.view.get("profiles") or {}).get(profile_ref)
            if prof is None:
                return {"decision": "CONTAIN", "permit_ref": None,
                        "reason_codes": [f"unknown-profile {profile_ref}"]}
            bind = prof.get("binding_ref")
            bstate = (self.view.get("bindings") or {}).get(bind, {}).get(
                "status")
            if bstate is not None and bstate != "admitted":
                return {"decision": "HOLD", "permit_ref": None,
                        "reason_codes": [f"binding-{bind} {bstate}"]}
            # 1. health first: stale health HOLDs dispatch (C23)
            try:
                healthy = bool(self.health_ok(bind))
            except Exception:
                healthy = False
            if not healthy:
                return {"decision": "HOLD", "permit_ref": None,
                        "reason_codes": [f"health-stale {bind}"]}
            # 2. budget reservation before capacity (C19)
            ok, why = self._budget_headroom(budget_scope, budget_cost)
            if not ok:
                return {"decision": "HOLD", "permit_ref": None,
                        "reason_codes": [why or "budget exhausted"]}
            # 3. occupancy vs every applicable ceiling (C18)
            occ = self._occupancy_all()
            desired = self._desired(profile_ref)
            reasons = []
            prof_occ = occ["profiles"].get(profile_ref, 0)
            if prof_occ + 1 > desired:
                reasons.append(f"profile {profile_ref} {prof_occ}+1 > "
                               f"desired {desired}")
            pceil = prof.get("ceiling", 1)
            if prof_occ + 1 > pceil:
                reasons.append(f"profile {profile_ref} ceiling {pceil}")
            bocc = occ["bindings"].get(bind, 0)
            bceil = (self.view.get("bindings") or {}).get(bind, {}).get(
                "ceiling", 1)
            if bocc + 1 > bceil:
                reasons.append(f"binding {bind} {bocc}+1 > ceiling {bceil}")
            for pool_id, pool in (self.view.get("pools") or {}).items():
                if bind in (pool.get("binding_refs") or []):
                    pool_occ = sum(occ["bindings"].get(b, 0) for b in
                                   pool["binding_refs"])
                    if pool_occ + 1 > pool.get("concurrent_slots", 1):
                        reasons.append(f"pool {pool_id} "
                                       f"{pool_occ}+1 > {pool['concurrent_slots']}")
            role_max = ((self.view.get("role_routing") or {}).get(role_ref)
                        or {}).get("max_concurrent_slots")
            if role_max is not None:
                role_occ = db.execute(
                    "SELECT COUNT(*) FROM permits WHERE role_ref = ? AND "
                    "state IN ('granted','redeemed','uncertain')",
                    (role_ref,)).fetchone()[0]
                if role_occ + 1 > role_max:
                    reasons.append(f"role {role_ref} {role_occ}+1 > {role_max}")
            g = self.view.get("global_ceiling")
            if isinstance(g, int) and occ["global"] + 1 > g:
                reasons.append(f"global {occ['global']}+1 > {g}")
            if reasons:
                return {"decision": "HOLD", "permit_ref": None,
                        "reason_codes": reasons}
            # 4. reserve + single-use permit (30s unredeemed expiry, C21)
            permit_ref = str(uuid.uuid4())
            db.execute(
                "INSERT INTO permits (permit_ref, config_hash, dispatch_id, "
                "attempt_id, role_ref, profile_ref, binding_ref, "
                "budget_scope, budget_cost, state, granted_at, expires_at) "
                "VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
                (permit_ref, self.config_hash, dispatch_id, attempt_id,
                 role_ref, profile_ref, bind, budget_scope, budget_cost,
                 "granted", now, now + PERMIT_TTL_SECONDS))
            body = {"permit_ref": permit_ref,
                    "config_hash": self.config_hash,
                    "dispatch_id": dispatch_id, "attempt_id": attempt_id,
                    "role": role_ref, "profile": profile_ref,
                    "binding": bind, "budget_scope": budget_scope}
            rcpt = _event(db, "admit-allow", body)
            db.commit()
            return {"decision": "ALLOW", "permit_ref": permit_ref,
                    "receipt": rcpt, "binding_ref": bind,
                    "reason_codes": []}

    def redeem(self, permit_ref):
        """Fence immediately before launch. Double redemption is a fault
        (C25); an uncertain redemption keeps the reservation (C21)."""
        with self._lock:
            db = self._conn()
            if db is None:
                return {"ok": False, "faults": ["authority-unavailable"]}
            row = db.execute(
                "SELECT state, expires_at, config_hash FROM permits "
                "WHERE permit_ref = ?", (permit_ref,)).fetchone()
            if not row:
                return {"ok": False, "faults": ["unknown permit"]}
            state, exp, cfg = row
            if state == "redeemed":
                return {"ok": False, "faults": ["permit already redeemed"]}
            if state == "uncertain":
                return {"ok": False, "faults": ["permit uncertain — reconcile"]}
            if cfg != self.config_hash:
                return {"ok": False, "faults": ["permit bound to another config"]}
            now = _now()
            if state == "granted" and exp <= now:
                return {"ok": False, "faults": ["permit expired"]}
            db.execute("UPDATE permits SET state = 'redeemed', redeemed_at = ?"
                       " WHERE permit_ref = ?", (now, permit_ref))
            rcpt = _event(db, "redeem", {"permit_ref": permit_ref, "at": now})
            db.commit()
            return {"ok": True, "receipt": rcpt}

    def mark_uncertain(self, permit_ref, detail=""):
        """A lost launch acknowledgement keeps the reservation until
        reconciled — duplicate replay is prohibited meanwhile (C25)."""
        with self._lock:
            db = self._conn()
            if db is None:
                return {"ok": False, "faults": ["authority-unavailable"]}
            db.execute("UPDATE permits SET state = 'uncertain' "
                       "WHERE permit_ref = ? AND state IN "
                       "('granted','redeemed')", (permit_ref,))
            rcpt = _event(db, "mark-uncertain", {"permit_ref": permit_ref,
                                                 "detail": str(detail)[:200]})
            db.commit()
            return {"ok": True, "receipt": rcpt}

    def reconcile(self, permit_ref, launched):
        """Resolve an uncertain permit: launched -> redeemed (still holding
        its slot until release()); not-launched -> released."""
        with self._lock:
            db = self._conn()
            if db is None:
                return {"ok": False, "faults": ["authority-unavailable"]}
            db.execute("UPDATE permits SET state = ? WHERE permit_ref = ?",
                       ("redeemed" if launched else "released", permit_ref))
            rcpt = _event(db, "reconcile", {"permit_ref": permit_ref,
                                            "launched": bool(launched)})
            db.commit()
            return {"ok": True, "receipt": rcpt}

    def release(self, permit_ref):
        """The dispatch finished: free its slot."""
        with self._lock:
            db = self._conn()
            if db is None:
                return {"ok": False, "faults": ["authority-unavailable"]}
            db.execute("UPDATE permits SET state = 'released' "
                       "WHERE permit_ref = ?", (permit_ref,))
            rcpt = _event(db, "release", {"permit_ref": permit_ref})
            db.commit()
            return {"ok": True, "receipt": rcpt}

    def status(self):
        db = self._conn()
        if db is None:
            return {"available": False}
        now = _now()
        occ = self._occupancy_all()
        leases = db.execute(
            "SELECT COUNT(*) FROM leases WHERE config_hash = ? AND "
            "expires_at > ?", (self.config_hash, now)).fetchone()[0]
        return {"available": True, "config_hash": self.config_hash,
                "version": self.version, "level": self.view.get("level"),
                "occupancy": occ, "active_leases": leases,
                "desired": {pid: self._desired(pid)
                            for pid in self.view.get("profiles", {})}}


def authority_from_config(cfg, config_hash, state_dir=None, **kw):
    """Compile a config through pressure_view and open the authority."""
    import swarmconfig as sc
    return PressureAuthority(sc.pressure_view(cfg), config_hash,
                             state_dir=state_dir, **kw)


def main():
    import argparse
    ap = argparse.ArgumentParser(prog="pressure")
    ap.add_argument("verb", choices=["status", "admit", "redeem",
                                     "reconcile"])
    ap.add_argument("--state-dir", default="~/.yggterm/swarm")
    ap.add_argument("--config-hash", default=None)
    ap.add_argument("--role", default=None)
    ap.add_argument("--profile", default=None)
    ap.add_argument("--dispatch", default=None)
    ap.add_argument("--budget-scope", default=None)
    ap.add_argument("--permit", default=None)
    ap.add_argument("--launched", action="store_true")
    a = ap.parse_args()
    sd = Path(a.state_dir).expanduser()
    head = a.config_hash
    if head is None:
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
    auth = authority_from_config(cfg, head or sc.content_hash(cfg),
                                 state_dir=sd)
    if a.verb == "status":
        print(json.dumps(auth.status(), indent=1))
        return 0
    if a.verb == "admit":
        out = auth.admit(a.dispatch or str(uuid.uuid4()), str(uuid.uuid4()),
                         a.role or "dreamers", a.profile or "",
                         budget_scope=a.budget_scope)
        print(json.dumps(out, indent=1))
        return 0 if out["decision"] == "ALLOW" else 1
    if a.verb == "redeem":
        out = auth.redeem(a.permit)
        print(json.dumps(out, indent=1))
        return 0 if out.get("ok") else 1
    if a.verb == "reconcile":
        out = auth.reconcile(a.permit, a.launched)
        print(json.dumps(out, indent=1))
        return 0 if out.get("ok") else 1
    return 1


if __name__ == "__main__":
    import sys
    sys.exit(main())
