"""pressure — the swarm-config v3 admission authority (repair v4).

Repair of the sol implementation review (2026-10-26, 11 findings; delta
reviews' residuals F35.1r-F35.11r and F35.2rr-F35.11rr):
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

Repair v3 (delta review residuals):
- F35.1r ordinary opening is READ-ONLY with respect to authority state:
  it never writes meta (never restores a cached config_hash, never bumps
  the epoch). The epoch transition is the explicit, fenced restart()
  operation. Every public mutation — including mark_uncertain, reconcile
  and release — fences against the store's active config hash. activate
  is pure ceiling comparison: no recursive authority construction.
- F35.2r redeem re-verifies the permit's principal through the injected
  verifier (the executor IS the admitted principal); reconcile requires
  an authenticated reconciler and exact identity when supplied. Expired
  never-redeemed grants terminalize under the transaction at admit time
  (state 'expired', attempt history preserved), so a definite nonlaunch
  frees the dispatch for a fresh attempt. Never touches redeemed or
  uncertain work.
- F35.3r an omitted plane is authoritatively derived (an enabled plane
  covering the binding; none means CONTAIN — an unbudgeted omission is
  impossible). Budget checks use one implementation that enforces consult
  AND token limits independently. Reconciled launches keep their spent
  marker (redeemed_at) so release never refunds spending. Budgets and
  plane-budget mappings ride the compiled view and refresh atomically
  with activation, bound to the same policy hash.
- F35.9r the factory bootstraps a NEVER-initialized area only; a vacated
  store inside an initialized area is a lost committed store (CONTAIN).
  Opening validates the signer key (exact 32 bytes, no fallback key).
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
from fractions import Fraction
from datetime import datetime, timezone
from pathlib import Path

PERMIT_TTL_SECONDS = 30
MAX_LEASE_TTL_SECONDS = 900
# evidence sampled mid-transaction can lead `now` by microseconds; a
# one-millisecond skew absorbs that while any real future-dating (>= 1s)
# still refuses
HEALTH_CLOCK_SKEW_SECONDS = 0.001
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
            (view.get("budgets") if isinstance(view, dict) else None) or {}
        self.plane_budgets = plane_budgets if plane_budgets is not None \
            else (view.get("plane_budgets") if isinstance(view, dict)
                  else None) or {}
        sd = Path(state_dir).expanduser() if state_dir \
            else Path("~/.yggterm/swarm").expanduser()
        self.db_path = sd / "runtime" / "pressure" / "state.sqlite"
        self._lock = threading.RLock()
        self._db = None
        self._contained = None  # reason string when the store is unusable
        self._signer_key = None
        self._recovery_barrier = False  # outstanding work seen at open time
        self._latch_refs = frozenset()  # the specific permits the latch binds
        self._open()

    # ------------------------------------------------------------ opening
    def _open(self):
        """READ-ONLY open (F35.1r): the store's meta is authoritative and
        never rewritten from the handle. Ordinary opening performs no
        epoch transition, no lease/permit invalidation and no identity
        write — that is restart()'s job, explicitly. The only state this
        computes is the crash-recovery latch: outstanding launched or
        uncertain work at open time holds new permits until reconciled."""
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
                db.close()
                return
            stored = dict(db.execute(
                "SELECT key, value FROM meta").fetchall())
            if stored.get("schema_version") != str(_SCHEMA_VERSION):
                self._contained = "store schema version mismatch"
                db.close()
                return
            try:
                key = bytes.fromhex(stored.get("signer_key") or "")
            except ValueError:
                key = b""
            if len(key) != 32:
                # presence and exact length are verified; there is no
                # fallback key (F35.9r)
                self._contained = "signer key missing or invalid (containment)"
                db.close()
                return
            self._signer_key = key
            # the crash latch binds to the ACTUAL outstanding work seen at
            # open (F35.9rrr): it holds only while one of THOSE permits is
            # still unresolved — resolution by any handle ends it, and
            # unrelated later work never resurrects it
            self._latch_refs = frozenset(
                r[0] for r in db.execute(
                    "SELECT permit_ref FROM permits WHERE state IN "
                    "('uncertain','redeemed')").fetchall())
            self._recovery_barrier = bool(self._latch_refs)
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
        if not self._signer_key:
            raise Contained("signer key unavailable")
        material = {"kind": kind, "body": body,
                    "signer": self._signer_key.hex()}
        rcpt = hmac.new(self._signer_key,
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

    def _write_barrier(self, db):
        """Recompute the persisted reconciliation flag from UNCERTAIN state
        and drop recovery obligations when outstanding work is fully
        resolved (F35.9rr). recovery_owed is only ever SET by restart();
        transitions may only CLEAR it (when nothing is outstanding)."""
        n_unc = db.execute(
            "SELECT COUNT(*) FROM permits WHERE state = "
            "'uncertain'").fetchone()[0]
        db.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES "
            "('reconcile_barrier', ?)", ("1" if n_unc else "0",))
        n_out = db.execute(
            "SELECT COUNT(*) FROM permits WHERE state IN "
            "('uncertain','redeemed')").fetchone()[0]
        if not n_out:
            db.execute(
                "INSERT OR REPLACE INTO meta (key, value) VALUES "
                "('recovery_owed', '0')")
            self._recovery_barrier = False
        return bool(n_unc) or bool(
            dict(db.execute("SELECT key, value FROM meta").fetchall()).get(
                "recovery_owed") == "1")

    def _barriered(self, db):
        """The composite recovery barrier (F35.9rr): the persisted
        uncertain flag, the persisted restart obligation (seen by every
        handle), or this handle's crash latch — which holds only while
        authoritative outstanding work still exists (resolution by another
        handle clears it)."""
        meta = dict(db.execute(
            "SELECT key, value FROM meta").fetchall())
        if meta.get("reconcile_barrier") == "1" or \
                meta.get("recovery_owed") == "1":
            return True
        if not self._recovery_barrier:
            return False
        # the latch holds only while ITS OWN outstanding permits are still
        # unresolved (F35.9rrr): peer resolution ends it; new unrelated
        # work cannot resurrect a completed recovery episode
        if self._latch_refs:
            n = db.execute(
                "SELECT COUNT(*) FROM permits WHERE permit_ref IN "
                "(%s) AND state IN ('uncertain','redeemed')"
                % ",".join("?" * len(self._latch_refs)),
                tuple(self._latch_refs)).fetchone()[0]
            if n == 0:
                self._recovery_barrier = False
                return False
        return True

    def restart(self):
        """The explicit serialized epoch transition (F35.1r/F35.9rr) — the
        ONLY startup write, and only when a consumer calls it: a new
        epoch, EVERY unredeemed grant of the running config terminalizes
        (attempt and digest history preserved — not merely expired ones),
        all runtime leases invalidate, and outstanding launched/uncertain
        work persists a recovery obligation every handle observes until it
        is resolved."""
        with self._lock:
            db2 = self._db
            try:
                with self._txn() as db:
                    self._fenced(db)
                    now = _now()
                    db.execute(
                        "UPDATE permits SET state='expired' "
                        "WHERE state='granted'", ())
                    db.execute("DELETE FROM leases")
                    epoch = int((dict(db.execute(
                        "SELECT key, value FROM meta").fetchall()).get(
                        "epoch") or "0")) + 1
                    db.execute(
                        "INSERT OR REPLACE INTO meta (key, value) VALUES "
                        "('epoch', ?)", (str(epoch),))
                    outstanding = db.execute(
                        "SELECT COUNT(*) FROM permits WHERE state IN "
                        "('uncertain','redeemed')").fetchone()[0]
                    if outstanding:
                        db.execute(
                            "INSERT OR REPLACE INTO meta (key, value) "
                            "VALUES ('recovery_owed', '1')")
                        self._recovery_barrier = True
                        self._latch_refs = frozenset(
                            r[0] for r in db.execute(
                                "SELECT permit_ref FROM permits WHERE state "
                                "IN ('uncertain','redeemed')").fetchall())
                    rcpt = self._receipt(db, "restart", {
                        "epoch": epoch, "config_hash": self.config_hash,
                        "reconcile_barrier": bool(outstanding), "at": now})
                    return {"ok": True, "receipt": rcpt, "epoch": epoch}
            except Contained as exc:
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"ok": False, "faults": [str(exc)]}
            except sqlite3.Error as exc:
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"ok": False, "faults": [f"store-error {exc}"]}

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
        base = (self.view.get("baseline") or {}).get(pid)
        if base is None:
            # legacy executable mode: no per-profile target is invented —
            # only the binding/pool/role/global ceilings constrain (F35.11rr)
            return self._leases_for(db, pid, now)
        return max(base, self._leases_for(db, pid, now))

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
            if "tokens" in limits:
                # independent check — consult and token limits both
                # enforce (F35.3r: no elif bypass for mixed budgets)
                if self.token_accounting is None:
                    raise Contained(
                        f"budget {bid} meters tokens and no token "
                        "accounting interface is available")
                # the accounting boundary is contained: provider failure is
                # unavailable evidence, and ONLY an exact True authorizes —
                # a truthy non-boolean result never does (F35.3rr)
                try:
                    tok = self.token_accounting(bid, plane_ref)
                except Exception as exc:
                    raise Contained(
                        f"budget {bid}: token accounting unavailable "
                        f"({type(exc).__name__})")
                if tok is False:
                    # a well-typed refusal is a HOLD, not a fault
                    return False, None, f"budget {bid}: token headroom refused"
                if tok is not True:
                    raise Contained(
                        f"budget {bid}: malformed token accounting result")
            checked.append(bid)
        return True, checked, None

    def _pool_cooldowns(self, db):
        raw = dict(db.execute(
            "SELECT key, value FROM meta").fetchall()).get("pool_cooldowns")
        try:
            out = _json.loads(raw) if raw else {}
        except Exception:
            return {}
        return {k: v for k, v in out.items()
                if isinstance(v, (int, float)) and not isinstance(v, bool)}

    def _persist_pool_cooldown(self, db, pool_ref, retry_at):
        """An identified pool refusal persists as a shared cooldown: every
        member binding of the pool holds until the retry time (F35.6r)."""
        cd = self._pool_cooldowns(db)
        cur = cd.get(pool_ref)
        if isinstance(cur, (int, float)) and cur >= retry_at:
            return
        cd[pool_ref] = retry_at
        db.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES (?,?)",
            ("pool_cooldowns", _canonical(cd)))

    # ------------------------------------------------------------ verbs
    def activate(self, view, config_hash, *, budgets=None, plane_budgets=None):
        """Bind to a new config inside the fence; invalidate previous-config
        leases/unredeemed permits; refuse any ceiling below counted
        occupancy — profiles, bindings, pools, roles, global (sol F8).
        Pure ceiling comparison against the PROPOSED view (F35.1r: no
        recursive authority construction). Budgets and plane-budget
        mappings come from the same proposed view and refresh atomically
        with the policy hash (F35.3r)."""
        with self._lock:
            db2 = self._db
            try:
                with self._txn() as db:
                    self._fenced(db)
                    now = _now()
                    occ = self._occupancy_all(db, now)
                    faults = self._check_ceilings(view, occ)
                    if faults:
                        return {"ok": False, "faults": faults}
                    nb = budgets if budgets is not None else \
                        (view.get("budgets") if isinstance(view, dict)
                         else None)
                    npb = plane_budgets if plane_budgets is not None else \
                        (view.get("plane_budgets") if isinstance(view, dict)
                         else None)
                    db.execute("DELETE FROM leases WHERE config_hash != ?",
                               (config_hash,))
                    db.execute("DELETE FROM permits WHERE config_hash != ? "
                               "AND state = 'granted'", (config_hash,))
                    db.execute("INSERT OR REPLACE INTO meta (key, value) "
                               "VALUES ('config_hash', ?)", (config_hash,))
                    rcpt = self._receipt(db, "activate",
                                         {"config_hash": config_hash,
                                          "at": now})
                    # COMMIT happens on context exit; only after it succeeds
                    # does this handle adopt the new identity atomically
                self.view = view
                self.config_hash = config_hash
                self.version = view.get("version")
                if nb is not None:
                    self.budgets = nb
                if npb is not None:
                    self.plane_budgets = npb
                return {"ok": True, "receipt": rcpt}
            except Contained as exc:
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"ok": False, "faults": [str(exc)]}
            except sqlite3.Error as exc:
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"ok": False, "faults": [f"store-error {exc}"]}

    @staticmethod
    def _check_ceilings(view, occ):
        v = view or {}
        faults = []
        g = v.get("global_ceiling")
        if isinstance(g, int) and occ.get("global", 0) > g:
            faults.append(f"PRESSURE_BELOW_INFLIGHT: global occupancy "
                          f"{occ['global']} > ceiling {g}")
        for pid, n in (occ.get("profiles") or {}).items():
            prof = PressureAuthority._canonical_view_profile(v, pid)
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
        # pool occupancy derives from EVERY PROPOSED pool's membership over
        # binding counts — never from the old view's pool ids (F35.8r)
        for pid, pool in (v.get("pools") or {}).items():
            n = sum((occ.get("bindings") or {}).get(b, 0)
                    for b in (pool.get("binding_refs") or []))
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

    @staticmethod
    def _canonical_view_profile(v, pid):
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
                    # normalize spellings BEFORE validating the complete
                    # target set (F35.7rrrr): identical duplicates
                    # deduplicate, conflicting duplicates refuse — never
                    # sum
                    norm = {}
                    for pid, n in (profile_slots or {}).items():
                        if type(n) is not int or isinstance(n, bool) or \
                                n < 0:
                            raise Contained(
                                f"profile_slots.{pid}: nonnegative int "
                                "required")
                        canon = self._canonical_profile(pid)
                        if canon in norm and norm[canon] != n:
                            raise Contained(
                                f"profile_slots: conflicting duplicate "
                                f"spellings for {canon} "
                                f"({norm[canon]} vs {n})")
                        norm[canon] = n
                    if not norm:
                        raise Contained("profile_slots empty")
                    clean = {}
                    for canon, n in norm.items():
                        prof = (self.view.get("profiles") or {}).get(canon)
                        # delegation is checked on the CANONICAL profile —
                        # an alias key never dodges or breaks delegation
                        # (F35.7rr)
                        if canon not in (self.view.get("agent_profile_refs")
                                         or []):
                            raise Contained(f"profile {canon} not delegated")
                        if prof is None:
                            raise Contained(f"unknown profile {canon}")
                        if n > prof.get("ceiling", 1):
                            raise Contained(
                                f"profile_slots.{canon}: 0.."
                                f"{prof.get('ceiling', 1)}")
                        bind = prof.get("binding_ref")
                        bceil = (self.view.get("bindings") or {}).get(
                            bind, {}).get("ceiling", 1)
                        if n > bceil:
                            raise Contained(
                                f"profile_slots.{canon}: {n} exceeds binding "
                                f"{bind} ceiling {bceil}")
                        for pool_id, pool in (
                                self.view.get("pools") or {}).items():
                            if bind in (pool.get("binding_refs") or []):
                                if n > pool.get("concurrent_slots", 1):
                                    raise Contained(
                                        f"profile_slots.{canon}: {n} exceeds "
                                        f"pool {pool_id}")
                        g = self.view.get("global_ceiling")
                        if isinstance(g, int) and n > g:
                            raise Contained(
                                f"profile_slots.{canon}: {n} exceeds global")
                        # leases persist CANONICAL keys so the raised
                        # target actually applies to the canonical profile
                        # (F35.7rrr)
                        clean[canon] = n
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
        try:
            resolved = self.principal_verifier(verified_principal)
        except Contained:
            raise
        except Exception as exc:
            # verifier unavailability is containment, never an exception
            # escape and never an authorization (F35.2rrrr)
            raise Contained(
                f"principal verifier unavailable "
                f"({type(exc).__name__})")
        if resolved != role_ref:
            raise Contained(
                f"principal {verified_principal} does not verify as "
                f"{role_ref}")

    def admit(self, *, dispatch_id, attempt_id, verified_principal,
              role_ref, profile_ref, plane_ref=None):
        """The gate every managed dispatch path calls (C27). Intersecting
        predicates inside the write fence; budget derived from the plane
        (an omitted plane is derived from the binding — F35.3r); single-use
        permit; replay fence per dispatch/attempt (sol F2). Expired
        never-redeemed grants terminalize under this transaction before
        the replay scan (F35.2r) — attempt history is preserved."""
        request = {"dispatch_id": dispatch_id, "attempt_id": attempt_id,
                   "principal": verified_principal, "role": role_ref,
                   "profile": profile_ref, "plane": plane_ref,
                   "config_hash": self.config_hash}
        rd = _digest(request)
        with self._lock:
            db2 = self._db
            try:
                with self._txn() as db:
                    self._fenced(db)
                    now = _now()
                    # definite nonlaunch: an expired, never-redeemed grant
                    # terminalizes (history kept) and frees the dispatch
                    db.execute(
                        "UPDATE permits SET state='expired' "
                        "WHERE state='granted' AND expires_at <= ?", (now,))
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
                    aliases = self.view.get("profile_aliases") or {}

                    def _c(x):
                        return aliases.get(x, x) if isinstance(x, str) else x

                    canon = _c(profile_ref)
                    prof = (self.view.get("profiles") or {}).get(canon)
                    if prof is None:
                        raise Contained(f"unknown profile {profile_ref}")
                    eligible = {_c(e) for e in (
                        [route.get("preferred_profile_ref")]
                        + list(route.get("fallback_profile_refs") or [])
                        + [ln.get("profile_ref") for ln in
                           (route.get("lanes") or [])]) if e}
                    if canon not in eligible:
                        raise Contained(
                            f"profile {profile_ref} is not in role "
                            f"{role_ref} routes")
                    bind = prof.get("binding_ref")
                    bdoc = (self.view.get("bindings") or {}).get(bind, {})
                    if bdoc.get("status") != "admitted":
                        raise Contained(
                            f"binding {bind} is "
                            f"{bdoc.get('status') or 'unknown'}")
                    # role capability requirements survive compilation and
                    # intersect at admission (F35.6r)
                    missing = set(route.get("required_capabilities") or []) \
                        - set(bdoc.get("capabilities") or [])
                    if missing:
                        raise Contained(
                            f"binding {bind} lacks required capabilities "
                            f"{sorted(missing)}")
                    # the plane is required or authoritatively derived
                    # (F35.3r): an unbudgeted omission is impossible
                    if plane_ref is None:
                        cands = sorted(
                            pid for pid, p in
                            (self.view.get("planes") or {}).items()
                            if p.get("owner_policy") == "enabled"
                            and bind in (p.get("binding_refs") or []))
                        if not cands:
                            raise Contained(
                                f"no enabled plane covers binding {bind}; "
                                "plane_ref is required")
                        plane_ref = cands[0]
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
                    # health evidence: typed, fail-closed; provider failure
                    # is unavailable evidence, never an exception escape
                    # (F35.6r)
                    try:
                        ev = self.health_evidence(bind) \
                            if self.health_evidence is not None else None
                    except Exception as exc:
                        return {"decision": "HOLD", "permit_ref": None,
                                "reason_codes": [
                                    f"health-evidence-unavailable {bind} "
                                    f"({type(exc).__name__})"]}
                    def _finite(x):
                        return (isinstance(x, (int, float))
                                and not isinstance(x, bool)
                                and x == x and x not in
                                (float("inf"), float("-inf")))

                    hc = ((self.view.get("planes") or {}).get(plane_ref, {})
                          .get("health_contract") or {})
                    ttl = hc.get("ttl_seconds")

                    def _health_contracted(ok_ev):
                        """The configured contract binds EVERY evidence
                        shape (F35.6rrrr, ruling B): the source must match
                        and the evidence must be dated + fresh. -> None
                        when conforming, else a refusal reason string."""
                        src_ref = hc.get("source_ref")
                        if isinstance(src_ref, str) and src_ref and \
                                ok_ev.get("source_ref") != src_ref:
                            return f"health source mismatch on {bind}"
                        if type(ttl) is int and \
                                not _finite(ok_ev.get("observed_at")):
                            # a contract requires a CURRENT OBSERVATION —
                            # a null/absent observation is undated even
                            # when an expiry assertion is present
                            # (F35.6rrrr, ruling B)
                            return (f"health undated on {bind} "
                                    f"(contract ttl {ttl})")
                        obs_ = ok_ev.get("observed_at")
                        if obs_ is not None:
                            if not _finite(obs_):
                                return (f"health observation malformed on "
                                        f"{bind}")
                            # int<->float comparisons are exact in CPython;
                            # keep the arithmetic on the small float side so
                            # huge finite integer timestamps cannot overflow
                            # (F35.6t)
                            if obs_ > now + HEALTH_CLOCK_SKEW_SECONDS:
                                return (f"health observation in the future "
                                        f"on {bind}")
                            if type(ttl) is int:
                                # EXACT freshness arithmetic (F35.6t):
                                # Fraction is exact over the accepted
                                # numeric domain (huge finite ints and
                                # floats alike) — an overflow fallback
                                # that asserts freshness is forbidden
                                try:
                                    stale = (Fraction(obs_) + Fraction(ttl)
                                             < Fraction(now))
                                except (ValueError, TypeError):
                                    return (f"health observation malformed "
                                            f"on {bind}")
                                if stale:
                                    return (f"health stale on {bind} "
                                            f"(older than ttl {ttl})")
                        exp_ = ok_ev.get("expires_at")
                        if exp_ is not None:
                            if not _finite(exp_):
                                return f"health expiry malformed on {bind}"
                            if exp_ <= now:
                                return f"health expired on {bind}"
                        return None

                    bad = _health_contracted(ev) \
                        if isinstance(ev, dict) else None
                    if bad:
                        return {"decision": "HOLD", "permit_ref": None,
                                "reason_codes": [bad]}
                    if not isinstance(ev, dict) or \
                            ev.get("state") not in ("ok", "healthy"):
                        # a refusal assertion that would mutate SHARED
                        # cooldown state must itself satisfy the contract
                        # first; invalid evidence still HOLDs this request
                        # but installs nothing (F35.6rrrr)
                        if isinstance(ev, dict) and \
                                isinstance(ev.get("pool_ref"), str) and \
                                _health_contracted(ev) is None:
                            # identified pool refusal persists a shared
                            # cooldown under the IDENTIFIED pool (never a
                            # first-membership neighbor, F35.6rrr): a
                            # finite CURRENT verified retry deadline, or
                            # now + that pool's configured fallback —
                            # NaN/past/infinite assertions fall back
                            identified = ev["pool_ref"]
                            rt_ = ev.get("retry_at")
                            if isinstance(rt_, (int, float)) and \
                                    not isinstance(rt_, bool) and \
                                    rt_ == rt_ and \
                                    rt_ not in (float("inf"),
                                                float("-inf")) and \
                                    rt_ > now:
                                        pass  # verified current deadline
                            else:
                                rt_ = None
                            if rt_ is None:
                                identified_pool = (
                                    self.view.get("pools") or {}).get(
                                    identified)
                                if identified_pool is not None:
                                    cd_secs = identified_pool.get(
                                        "cooldown_after_refusal_seconds") or 0
                                    if cd_secs > 0:
                                        rt_ = now + cd_secs
                            if rt_ is not None:
                                self._persist_pool_cooldown(
                                    db, identified, rt_)
                        return {"decision": "HOLD", "permit_ref": None,
                                "reason_codes": [f"health-unavailable {bind}"]}
                    limit = ev.get("concurrent_limit")
                    if limit is not None and type(limit) is not int:
                        return {"decision": "HOLD", "permit_ref": None,
                                "reason_codes": [
                                    f"health limit malformed on {bind}"]}
                    # persisted pool cooldowns intersect across member
                    # profiles regardless of what the probe now reports
                    cd = self._pool_cooldowns(db)
                    for pool_id, pool in (
                            self.view.get("pools") or {}).items():
                        if bind in (pool.get("binding_refs") or []):
                            rt_ = cd.get(pool_id)
                            if isinstance(rt_, (int, float)) and rt_ > now:
                                return {"decision": "HOLD",
                                        "permit_ref": None,
                                        "reason_codes": [
                                            f"pool {pool_id} cooldown until "
                                            f"{rt_}"]}
                    # budget reservation — one checked implementation,
                    # consult AND token limits enforced independently
                    ok, scopes, breason = self._budget_check(
                        db, plane_ref, now)
                    if not ok:
                        return {"decision": "HOLD", "permit_ref": None,
                                "reason_codes": [breason]}
                    # occupancy vs every applicable ceiling
                    occ = self._occupancy_all(db, now)
                    reasons = []
                    p_occ = occ["profiles"].get(canon, 0)
                    base_target = (self.view.get("baseline") or {}).get(canon)
                    if base_target is not None:
                        desired = max(base_target,
                                      self._leases_for(db, canon, now))
                        if p_occ + 1 > desired:
                            reasons.append(f"profile {profile_ref} "
                                           f"{p_occ}+1 > desired {desired}")
                    pceil = prof.get("ceiling")
                    if isinstance(pceil, int) and \
                            not isinstance(pceil, bool) and \
                            p_occ + 1 > pceil:
                        reasons.append(f"profile {profile_ref} "
                                       f"ceiling {pceil}")
                    # the health limit binds the BINDING, so it intersects
                    # every profile that shares it (F35.6r)
                    b_occ = occ["bindings"].get(bind, 0)
                    if type(limit) is int and b_occ + 1 > limit:
                        reasons.append(f"health limit {limit} on {bind}")
                    bceil = bdoc.get("ceiling")
                    if isinstance(bceil, int) and \
                            not isinstance(bceil, bool) and \
                            b_occ + 1 > bceil:
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
                    # barrier: outstanding reconciliation state blocks new
                    # permits (F9/F35.9r) — the persisted flag or the
                    # crash-recovery latch from opening
                    if self._barriered(db):
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
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"decision": "CONTAIN", "permit_ref": None,
                        "reason_codes": [str(exc)]}
            except sqlite3.Error as exc:
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"decision": "CONTAIN", "permit_ref": None,
                        "reason_codes": [f"store-error {exc}"]}

    def redeem(self, permit_ref, *, dispatch_id, attempt_id, executor):
        """Fence immediately before launch. Only granted+unexpired+current
        permits redeem (sol F2); the CURRENT executor identity is REQUIRED
        and trusted: it must be the permit's authorized principal AND
        re-verify through the injected verifier (F35.2rrrr — ruling A
        retired the omitted-executor fallback). One-row update authorizes
        the launch."""
        with self._lock:
            db2 = self._db
            try:
                with self._txn() as db:
                    self._fenced(db)
                    row = db.execute(
                        "SELECT state, principal, role_ref FROM permits "
                        "WHERE permit_ref = ?", (permit_ref,)).fetchone()
                    if not row:
                        raise Contained("unknown permit")
                    state, principal, prole = row
                    if state != "granted":
                        raise Contained(
                            f"permit not redeemable (state={state})")
                    if executor is None or executor != principal:
                        raise Contained(
                            "redeem requires the current executor identity "
                            "of the permit's authorized principal")
                    # re-verified NOW — a verifier that stopped authorizing
                    # stops all redemption (F35.2r/F35.2rr)
                    self._resolve_principal(principal, prole)
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
                        raise Contained(
                            "permit not redeemable (identity or expiry)")
                    rcpt = self._receipt(db, "redeem",
                                         {"permit_ref": permit_ref,
                                          "dispatch_id": dispatch_id,
                                          "attempt_id": attempt_id,
                                          "at": now})
                    return {"ok": True, "receipt": rcpt}
            except Contained as exc:
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"ok": False, "faults": [str(exc)]}
            except sqlite3.Error as exc:
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"ok": False, "faults": [f"store-error {exc}"]}

    def mark_uncertain(self, permit_ref, detail=""):
        with self._lock:
            db2 = self._db
            try:
                with self._txn() as db:
                    self._fenced(db)
                    cur = db.execute(
                        "UPDATE permits SET state = 'uncertain' WHERE "
                        "permit_ref = ? AND state IN ('granted','redeemed')",
                        (permit_ref,))
                    if cur.rowcount != 1:
                        raise Contained("permit not markable uncertain")
                    self._write_barrier(db)
                    rcpt = self._receipt(db, "mark-uncertain",
                                         {"permit_ref": permit_ref,
                                          "detail": str(detail)[:200]})
                    return {"ok": True, "receipt": rcpt}
            except (Contained, sqlite3.Error) as exc:
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"ok": False,
                        "faults": [str(exc) if isinstance(exc, Contained)
                                   else f"store-error {exc}"]}

    def reconcile(self, permit_ref, *, launched, dispatch_id=None,
                  attempt_id=None, reconciler=None, evidence=None):
        """Resolve an uncertain permit: launched -> redeemed (slot held
        until release, spent marker kept — F35.3r); not-launched ->
        released. Only uncertain permits reconcile (sol F2). EVERY
        supplied identity component is validated; all supported caller
        assertions (explicit reconciler, reconciliation evidence) must
        AGREE and name the permit's authorized principal; the stored
        principal re-verifies through the injected verifier on every call.
        Omitted identity requires authenticated reconciliation evidence —
        the unauthenticated in-process fallback is retired now that the
        editor re-baselined the round-2 lifecycle control to carry exact
        identity (F35.2rrr)."""
        if not isinstance(launched, bool):
            return {"ok": False, "faults": ["launched must be a bool"]}
        with self._lock:
            db2 = self._db
            try:
                with self._txn() as db:
                    self._fenced(db)
                    now = _now()
                    row = db.execute(
                        "SELECT dispatch_id, attempt_id, principal, "
                        "role_ref FROM permits WHERE permit_ref = ? AND "
                        "state = 'uncertain'", (permit_ref,)).fetchone()
                    if not row:
                        raise Contained(
                            "permit not uncertain (nothing to reconcile)")
                    d_id, a_id, principal, prole = row
                    # reconciliation authorization requires COMPLETE
                    # attempt identity (BOTH ids) or authenticated
                    # evidence — a partial identity without evidence
                    # refuses (F35.2rrrr)
                    if not (dispatch_id is not None
                            and attempt_id is not None) \
                            and evidence is None:
                        raise Contained(
                            "reconciliation requires complete attempt "
                            "identity or authenticated evidence")
                    # every SUPPLIED identity component is validated,
                    # independently of what else was supplied (F35.2rrr)
                    if dispatch_id is not None and dispatch_id != d_id:
                        raise Contained(
                            "reconcile dispatch does not match the permit")
                    if attempt_id is not None and attempt_id != a_id:
                        raise Contained(
                            "reconcile attempt does not match the permit")
                    # all supported caller assertions must AGREE: an
                    # explicit reconciler and reconciliation evidence can
                    # never name different principals (F35.2rrr)
                    ev_caller = None
                    if evidence is not None:
                        if not isinstance(evidence, dict) or \
                                not isinstance(evidence.get("reconciler"),
                                               str) or \
                                evidence.get("reconciler") != principal:
                            raise Contained(
                                "reconciliation evidence does not vouch "
                                "the authorized principal")
                        ev_caller = evidence["reconciler"]
                    if reconciler is None and ev_caller is not None:
                        reconciler = ev_caller
                    if reconciler is not None and reconciler != principal:
                        raise Contained(
                            "reconciler is not the permit's authorized "
                            "principal")
                    if reconciler is not None and ev_caller is not None \
                            and reconciler != ev_caller:
                        raise Contained(
                            "conflicting caller assertions")
                    # authenticated reconciliation: the authorized
                    # principal re-verifies through the injected verifier
                    # (F35.2r/F35.2rr)
                    self._resolve_principal(principal, prole)
                    if launched:
                        db.execute(
                            "UPDATE permits SET state = 'redeemed', "
                            "redeemed_at = ? WHERE permit_ref = ?",
                            (now, permit_ref))
                    else:
                        db.execute(
                            "UPDATE permits SET state = 'released' "
                            "WHERE permit_ref = ?", (permit_ref,))
                    self._write_barrier(db)
                    rcpt = self._receipt(db, "reconcile",
                                         {"permit_ref": permit_ref,
                                          "launched": launched})
                    return {"ok": True, "receipt": rcpt}
            except (Contained, sqlite3.Error) as exc:
                if db2 is not None and db2.in_transaction:
                    db2.execute("ROLLBACK")
                return {"ok": False,
                        "faults": [str(exc) if isinstance(exc, Contained)
                                   else f"store-error {exc}"]}

    def release(self, permit_ref):
        """The dispatch finished: free its slot. Terminal and idempotent
        (sol F2); unknown permits fault; fenced like every mutation
        (F35.1r)."""
        with self._lock:
            db2 = self._db
            try:
                with self._txn() as db:
                    self._fenced(db)
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
                    self._write_barrier(db)
                    rcpt = self._receipt(db, "release",
                                         {"permit_ref": permit_ref})
                    return {"ok": True, "receipt": rcpt}
            except (Contained, sqlite3.Error) as exc:
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
                "reconcile_barrier": self._barriered(db),
                "occupancy": occ, "active_leases": leases,
                "desired": {pid: self._desired(db, pid, now)
                            for pid in self.view.get("profiles", {})}}


def authority_from_config(cfg, config_hash, state_dir=None, **kw):
    """Compile a config through pressure_view; budgets and plane budget
    mappings load from the config itself (sol F3) unless overridden — the
    compiled view carries them too, so activation refreshes them bound to
    the same policy hash. First-use bootstrap: the store is provisioned
    ONLY when the runtime/pressure area has never been initialized; a
    vacated store inside an initialized area is a LOST committed store
    and stays containment, never a fresh authority (F35.9r)."""
    import swarmconfig as sc
    sd = Path(state_dir).expanduser() if state_dir         else Path("~/.yggterm/swarm").expanduser()
    area = sd / "runtime" / "pressure"
    if not area.exists():
        PressureAuthority.provision({}, config_hash, state_dir=state_dir)
    view = sc.pressure_view(cfg)
    kw.setdefault("budgets", (cfg.get("supply") or {}).get("budgets") or {})
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
