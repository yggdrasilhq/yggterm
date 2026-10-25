#!/usr/bin/env python3
"""ygg-dream-orchestrator — the yggterm dream system's demand side.

One script, three passes (all bounded, all single-flight):

  status      human/json view: today counts, backlog, plane gates
  topup       supply->demand: stage briefs (ygg-memory dream --prepare) and
              enqueue them on the executor queue, bounded per pass
  supervise   fleet-level care for dreams that died at API drops / quota walls:
              classify queue failures against the quota calendars, reroute to
              planes with quota, hold until a reset window, revoke
              false-completions, write the wake ledger

Config (generated on first run, hand-moldable; bootstrap never overwrites):
  ~/.yggterm/config/dream/floors.json   namespaces, floors, routes, targets,
                                        peak windows, quota calendars
  ~/.yggterm/dream/state.json           per-day counts (the spawner's ledger)
  ~/.yggterm/dream/wake-ledger.jsonl    one line per supervision pass

Arming law (onboarding): autonomous dispatch runs on EXACTLY ONE host —
~/.yggterm/config/dream/enabled must exist; without it every mutating pass
exits 0 silently (units install fleet-wide, arming happens once).

Owner law baked in: banked resets are NEVER consumed by any automated path;
quota exhaustion means hold/reroute, never a reset.
"""
import fcntl, json, random, shlex, subprocess, sys, time, uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path

HOME = Path.home()
CFG_DIR = HOME / ".yggterm/config/dream"
CFG = CFG_DIR / "floors.json"
ENABLED = CFG_DIR / "enabled"
STATE_DIR = HOME / ".yggterm/dream"
STATE = STATE_DIR / "state.json"
LEDGER = STATE_DIR / "wake-ledger.jsonl"
DREAM_LEDGER = Path("/tmp/dreamer/dream-ledger.jsonl")
AUDIT_LEDGER = Path("/tmp/dreamer/audit-ledger.jsonl")
LOCK = Path("/tmp/ygg-dream-orchestrator.lock")
IST = timezone(timedelta(hours=5, minutes=30))

DEFAULT_CONFIG = {
    "_doc": "dream orchestrator config — mold freely; a tuned file is never overwritten",
    "queue": {"host": "dev", "repo": "~/gh/zcode-tui", "cli": "~/.bun/bin/bun run src/bin/queue.ts"},
    "supply": {"prepare_timeout_s": 240, "pass_max_enqueues": 2, "pass_wall_budget_s": 420},
    "targets": {"day": 60, "moderate": 20, "backlog_cap": 14},
    "routes": ["zai-coding-plan", "autoclaw", "codex", "opencode-free"],
    "quota_calendars": {
        "_doc": "per-plane gates; the orchestrator holds or reroutes, never resets accounts",
        "zai-coding-plan": {"peak_ist": "Mon-Fri 11:30-15:30"},
        "codex": {"_doc": "hourly rolling caps: the queue's cooldown ladder rides the refusal; supervision reroutes"},
        "opencode-free": {"_doc": "free tier — queue owns cooldowns; supervision only audits"},
        "autoclaw": {}
    },
    "namespaces": [
        {"ns": "-home-pi-data-medgraph", "per_day": 8, "moderate_share": 0.5, "budget": 90000},
        {"ns": "-home-pi-gh-yggterm", "per_day": 8, "moderate_share": 0.5, "budget": 90000},
        {"ns": "_global", "per_day": 6, "moderate_share": 0.5, "budget": 90000}
    ],
    "kinds": ["synthesis", "recon", "verify", "sweep"],
    "false_completion": {"audit_per_pass": 3}
}

CONTRACT = """# DREAM ({ns}, {kind}) — consolidate what the journals learned
You are the dreamer for `{ns}`. Read the staged brief and its manifest, compose
RESPONSE + CRITIC in the pending dir per the dream contract, apply what is
safe, publish what must survive. UNATTENDED: no questions, verify before done.
Brief: {brief}
Manifest: {manifest}
Finish with: DREAM {ns} consolidated=<yes|no> findings=<n>
"""


def load_cfg():
    if not CFG.exists():
        CFG_DIR.mkdir(parents=True, exist_ok=True)
        CFG.write_text(json.dumps(DEFAULT_CONFIG, indent=2) + "\n")
        print(f"orchestrator: generated default config {CFG} — mold it, then arm: touch {ENABLED}")
    return json.loads(CFG.read_text())


def run(cmd, timeout=120):
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout, p.stderr


def queue_cli(cfg):
    q = cfg["queue"]
    return ["ssh", q["host"], f"cd {q['repo']} && {q['cli']}"]


def qjson(cfg, args, timeout=60):
    # sol turn-2 repair 3: malformed/error output previously became [] —
    # indistinguishable from a healthy empty queue, which let the watch infer
    # nothing is alive. None means UNKNOWN; callers hold instead of acting.
    rc, out, err = run(queue_cli(cfg) + args + ["--json"], timeout)
    try:
        return json.loads((out or "").strip())
    except Exception:
        return None


def parse_peak(spec, now):
    """'Mon-Fri 11:30-15:30' -> (active?, mins_left) or (None, 0) when the
    spec is unparseable. sol turn-2 repair 2: int("Mon-Fri") crashed and the
    except returned "not active" — an invalid calendar must read UNKNOWN
    (fail closed: the caller holds the route), never unrestricted."""
    daymap = {"mon": 0, "tue": 1, "wed": 2, "thu": 3, "fri": 4, "sat": 5, "sun": 6}

    def _day(tok):
        tok = tok.strip().lower()[:3]
        if tok.isdigit():
            return int(tok)
        if tok not in daymap:
            raise ValueError(f"bad day {tok!r}")
        return daymap[tok]

    try:
        days, win = spec.split(" ")
        if days.lower() == "daily":
            in_days = True
        elif "-" in days:
            d0, d1 = days.split("-")
            a, b = _day(d0), _day(d1)
            in_days = (a <= now.weekday() <= b) if a <= b else (now.weekday() >= a or now.weekday() <= b)
        else:
            in_days = now.weekday() == _day(days)
        start, end = win.split("-")
        sh, sm = map(int, start.split(":"))
        eh, em = map(int, end.split(":"))
        # range validation (sol turn-7 Q3-7): nonsense hours or an overnight
        # window are UNKNOWN (hold), never silently unrestricted
        if not (0 <= sh <= 23 and 0 <= eh <= 23 and 0 <= sm <= 59 and 0 <= em <= 59):
            return None, 0
        if (eh, em) <= (sh, sm):
            return None, 0  # cross-midnight semantics unsupported -> UNKNOWN
        cur = now.hour * 60 + now.minute
        return in_days and (sh * 60 + sm) <= cur < (eh * 60 + em), (eh * 60 + em) - cur
    except Exception:
        return None, 0


def plane_gates(cfg, now=None):
    now = now or datetime.now(IST)
    gates = {}
    for route in cfg["routes"]:
        cal = cfg["quota_calendars"].get(route, {})
        spec = cal.get("peak_ist")
        if spec:
            active, mins_left = parse_peak(spec, now)
            if active is None:
                # unparseable calendar: UNKNOWN — hold the route (fail closed)
                gates[route] = {"ok": False, "why": "calendar unparseable",
                                "retry_after_min": 0}
            else:
                gates[route] = {"ok": not active, "why": "peak window" if active else "",
                                "retry_after_min": mins_left if active else 0}
        else:
            gates[route] = {"ok": True, "why": "", "retry_after_min": 0}
    return gates


def acquire_lock():
    # sol turn-2 repair 1 (reproduced): the returned handle MUST be held for
    # the process lifetime — a discarded reference gets GC'd, closing the fd
    # and silently releasing the flock (a second pass acquired immediately).
    lf = open(LOCK, "w")
    try:
        fcntl.flock(lf, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        print("orchestrator: another pass holds the lock — skipping")
        sys.exit(0)
    return lf


def armed():
    if ENABLED.exists():
        return True
    print(f"orchestrator: not armed ({ENABLED} absent) — no-op (single-host arming law)")
    return False


def load_state():
    try:
        return json.loads(STATE.read_text())
    except Exception:
        return {}


def save_state(st):
    STATE_DIR.mkdir(parents=True, exist_ok=True)
    STATE.write_text(json.dumps(st, indent=1))


# ------------------------------------------------------------- policy ----
# swarm-harness2 Phase-1 (sol turn-2 contract, owner directive 2026-10-24):
# deterministic, temporary dispatch overlays derived from audit-feedback
# findings. Exactly three permitted patch types; holds only NARROW
# eligibility; starvation never disables the starved route; re-reading the
# same finding never creates a patch nor renews a TTL; no model, credential,
# budget, or doctrine changes ride this layer.
POLICY_DIR = HOME / ".yggterm/dream/policy"
try:
    import falsify as _falsify  # kernel sibling: claim/exposure/evaluation ledger
except ImportError:  # any run context: find the kernel dir beside the verb/repo
    _here = Path(__file__).resolve()
    for _kp in (_here.parents[3] / "tools" / "swarmharness", _here.parent,
                HOME / "gh" / "yggterm" / "tools" / "swarmharness",
                HOME / ".yggterm" / "swarmkernel"):
        if (_kp / "falsify.py").exists():
            sys.path.insert(0, str(_kp))
            break
    import falsify as _falsify

# ---- swarm-harness v2 gate (sol turn-7 Q3-1): the LoadResult CONSTRAINS
# dispatch — containment, owner denials, and plane owner_policy are not
# advisory. floors.json remains the supply QUANTITY authority; v2 is the
# PERMISSION authority.
ROUTE_TO_PLANE = {"zai-coding-plan": "zai-peak", "zai-offpeak": "zai-offpeak",
                  "opencode-free": "opencode-free", "codex": "intelligence-consult",
                  "autoclaw": "autoclaw-chat"}
_SWRES = None  # bound in main(); LoadResult dict


def v2_blocked_routes(routes):
    """-> (blocked_set, note). Intersects routes with the v2 activation store:
    containment blocks everything; owner deny latches and disabled/draining
    planes block their routes; an unreadable deny store blocks everything."""
    global _SWRES
    blocked, notes = set(), []
    if _SWRES is None:
        try:
            here = Path(__file__).resolve()
            for kpath in [here.parents[3] / "tools", here.parent,
                          HOME / "gh" / "yggterm" / "tools"]:
                if (kpath / "swarmharness" / "__init__.py").exists():
                    sys.path.insert(0, str(kpath))
                    break
            from swarmharness import swarmconfig as _sc
            _SWRES = _sc.load()
        except Exception as e:
            return set(routes), f"v2 gate unavailable ({e!r}) — containment"
    mode = _SWRES.get("mode")
    if mode == "containment_only":
        return set(routes), "v2 CONTAINMENT-ONLY — all dispatch held"
    denials = _SWRES.get("denials") or {}
    if "__denies_corrupt__" in denials:
        return set(routes), "v2 deny store UNREADABLE — all dispatch held (fail-closed)"
    planes = ((_SWRES.get("config") or {}).get("planes") or {})
    for r in routes:
        pl = planes.get(ROUTE_TO_PLANE.get(r, r))
        if pl is None:
            continue  # route not governed by a v2 plane: floors decides
        pol = pl.get("owner_policy")
        if pol in ("disabled", "draining"):
            blocked.add(r)
            notes.append(f"{r}:{pol}")
        if r in denials or ROUTE_TO_PLANE.get(r, r) in denials:
            blocked.add(r)
            notes.append(f"{r}:owner-denied")
    return blocked, ("; ".join(notes) if notes else "v2 gate clear")
OVERLAYS = POLICY_DIR / "overlays.json"
RECEIPTS = POLICY_DIR / "receipts.jsonl"
FEEDBACK = HOME / ".yggterm/dream/audit-feedback.json"
POLICY_VERSION = 1
HOLD_TTL_MIN = {"route_hold": 60, "namespace_hold": 30}


def _sha8(text):
    import hashlib
    return hashlib.sha256(text.encode()).hexdigest()[:8]


def _receipt(decision, **kw):
    POLICY_DIR.mkdir(parents=True, exist_ok=True)
    row = {"t": int(time.time()), "decision": decision, "policy_version": POLICY_VERSION, **kw}
    with RECEIPTS.open("a") as f:
        f.write(json.dumps(row) + "\n")


def _write_overlays(active):
    POLICY_DIR.mkdir(parents=True, exist_ok=True)
    tmp = OVERLAYS.with_suffix(".tmp")
    tmp.write_text(json.dumps(active, indent=1))
    tmp.replace(OVERLAYS)


def load_feedback(cfg):
    """Validated audit-feedback snapshot, or (None, reason). Sol's normative
    checks: staleness, reversed window, schema version; missing/malformed/
    stale feedback activates nothing."""
    try:
        fb = json.loads(FEEDBACK.read_text())
    except FileNotFoundError:
        return None, "no feedback snapshot yet"
    except Exception as e:
        return None, f"malformed feedback: {e}"
    if fb.get("schema_version") != 1:
        return None, f"unsupported schema_version {fb.get('schema_version')}"
    vu = fb.get("valid_until")
    try:
        if datetime.fromisoformat(vu) < datetime.now(IST):
            return None, f"stale snapshot (valid_until {vu})"
    except Exception:
        return None, "invalid valid_until"
    if not isinstance(fb.get("findings"), list):
        return None, "findings missing"
    for f in fb.get("findings", []):
        if not isinstance(f, dict):
            return None, "finding row not a mapping"
        if not f.get("code") or not f.get("status") or not f.get("finding_id"):
            return None, f"finding missing code/status/finding_id: {str(f)[:80]}"
        if f.get("status") not in ("confirmed", "suspected", "cleared", "watch"):
            return None, f"unknown finding status {f.get('status')!r}"
        gen = fb.get("generated_at") or ""
        vu = fb.get("valid_until") or ""
        try:
            if datetime.fromisoformat(gen) > datetime.fromisoformat(vu):
                return None, "reversed window (generated after valid_until)"
        except Exception:
            return None, "unparseable window stamps"
    return fb, "ok"


def build_overlays(cfg):
    """Findings -> permitted overlays. Deterministic; idempotent per finding
    (an active overlay citing the same finding_id is KEEP_BASE, not renewal)."""
    fb, why = load_feedback(cfg)
    if fb is None:
        return f"feedback unusable ({why}) — no overlays"
    now = datetime.now(IST)
    active = []
    try:
        existing = json.loads(OVERLAYS.read_text()) if OVERLAYS.exists() else []
    except Exception:
        existing = []
    by_pid = {o.get("patch_id"): o for o in existing if o.get("expires_at")}
    # processed-finding identity outlives overlay lifetime (sol turn-7 Q3-2):
    # a finding whose hold EXPIRED never silently re-arms from the same id
    seen_path = POLICY_DIR / "seen-findings.json"
    try:
        seen = json.loads(seen_path.read_text()) if seen_path.exists() else {}
    except Exception:
        seen = {}
    summary = []
    for f in fb.get("findings", []):
        code, status, scope = f.get("code"), f.get("status"), (f.get("scope") or {})
        fid = f.get("finding_id") or ""
        if status != "confirmed" or not fid:
            continue
        pid = None
        if code in ("route_auth_failure", "route_quota_failure", "route_model_incompatible") and scope.get("route"):
            pid = "rh-" + _sha8(fid)
            if pid in by_pid and datetime.fromisoformat(by_pid[pid]["expires_at"]) > now:
                _receipt("keep_base", patch_id=pid, finding_id=fid, reason="active overlay cites this finding")
                continue
            if not by_pid.get(pid) and seen.get(fid):
                continue  # expired hold for this finding: ratification needed, not renewal
            ov = {"patch_id": pid, "type": "route_hold", "finding_ids": [fid],
                  "route": scope["route"],
                  # family-scoped when the evidence only indicts one family
                  # (sol: a route-wide hold needs shared-credential evidence)
                  "family": scope.get("family") or "",
                  "reason_code": code,
                  "created_at": now.isoformat(timespec="seconds"),
                  "expires_at": (now + timedelta(minutes=HOLD_TTL_MIN["route_hold"])).isoformat(timespec="seconds"),
                  "policy_version": POLICY_VERSION,
                  "rollback": "deactivate_this_patch"}
            active.append(ov); summary.append(f"route_hold {scope['route']}" + (f" [{scope.get('family')}]" if ov["family"] else ""))
            _falsify.claim_open(pid, scope={"route": scope["route"], "family": ov.get("family") or ""},
                                predicted_effect=f"no new {code} failures on {scope['route']} while held",
                                falsifier=f"post-policy attempt on {scope['route']} fails again with {code}",
                                observation_deadline_min=HOLD_TTL_MIN["route_hold"] + 20,
                                config_revision=str((_SWRES or {}).get("revision") or ""),
                                finding_id=fid)
        elif code == "namespace_prepare_failure" and scope.get("namespace"):
            pid = "nh-" + _sha8(fid)
            if pid in by_pid and datetime.fromisoformat(by_pid[pid]["expires_at"]) > now:
                _receipt("keep_base", patch_id=pid, finding_id=fid, reason="active overlay cites this finding")
                continue
            ov = {"patch_id": pid, "type": "namespace_hold", "finding_ids": [fid],
                  "namespace": scope["namespace"], "reason_code": code,
                  "created_at": now.isoformat(timespec="seconds"),
                  "expires_at": (now + timedelta(minutes=HOLD_TTL_MIN["namespace_hold"])).isoformat(timespec="seconds"),
                  "policy_version": POLICY_VERSION,
                  "rollback": "deactivate_this_patch"}
            active.append(ov); summary.append(f"namespace_hold {scope['namespace']}")
            _falsify.claim_open(pid, scope={"namespace": scope["namespace"]},
                                predicted_effect=f"no new {code} on {scope['namespace']} while held",
                                falsifier=f"post-policy prepare failure recurs on {scope['namespace']}",
                                observation_deadline_min=HOLD_TTL_MIN["namespace_hold"] + 20,
                                config_revision=str((_SWRES or {}).get("revision") or ""),
                                finding_id=fid)
        elif code == "route_starved" and scope.get("route") and isinstance(f.get("allocation"), dict):
            pid = "dp-" + _sha8(fid)
            if pid in by_pid and datetime.fromisoformat(by_pid[pid]["expires_at"]) > now:
                _receipt("keep_base", patch_id=pid, finding_id=fid, reason="active overlay cites this finding")
                continue
            # starvation BOOSTS the starved route's next selection once; it
            # never suppresses anything (sol: balancing never disables).
            ov = {"patch_id": pid, "type": "route_debt_preference", "finding_ids": [fid],
                  "route": scope["route"], "max_extra_selections": 1,
                  "created_at": now.isoformat(timespec="seconds"),
                  "expires_at": (now + timedelta(minutes=30)).isoformat(timespec="seconds"),
                  "policy_version": POLICY_VERSION,
                  "rollback": "deactivate_this_patch"}
            active.append(ov); summary.append(f"route_debt_preference {scope['route']}")
            _falsify.claim_open(pid, scope={"route": scope["route"]},
                                predicted_effect=f"{scope['route']} share recovers toward its expected share",
                                falsifier=f"share still below expected after the preferred selection",
                                observation_deadline_min=50,
                                config_revision=str((_SWRES or {}).get("revision") or ""),
                                finding_id=fid)
    # carry forward still-active prior overlays (expiry pruning with receipts)
    for o in existing:
        try:
            if datetime.fromisoformat(o["expires_at"]) > now:
                if not any(a["patch_id"] == o["patch_id"] for a in active):
                    active.append(o)
            elif o.get("patch_id") not in by_pid or datetime.fromisoformat(by_pid[o["patch_id"]]["expires_at"]) <= now:
                pass  # expiry receipt only on the pass that prunes it
        except Exception:
            continue
    for f in fb.get("findings", []):
        if f.get("status") == "confirmed" and f.get("finding_id"):
            seen[f["finding_id"]] = now.isoformat(timespec="seconds")
    try:
        seen_path.parent.mkdir(parents=True, exist_ok=True)
        seen_path.write_text(json.dumps(seen, indent=1, sort_keys=True))
    except Exception:
        pass
    _write_overlays(active)
    if summary:
        for s_ in summary:
            print(f"orchestrator: policy overlay applied — {s_}")
        return "; ".join(summary)
    return "no confirmed findings warranted overlays"


def resolve_policy(family=None, ns=None):
    """Active overlays narrowed to (family, ns). Returns
    {"blocked_routes": set, "blocked_ns": bool, "debt_route": str|None,
     "overlays": [...]}. Holds only narrow; nothing here can widen."""
    out = {"blocked_routes": set(), "blocked_ns": False, "debt_route": None, "overlays": []}
    now = datetime.now(IST)
    try:
        active = json.loads(OVERLAYS.read_text()) if OVERLAYS.exists() else []
    except Exception:
        return out
    live = []
    for o in active:
        try:
            exp = datetime.fromisoformat(o["expires_at"])
        except Exception:
            continue
        if exp <= now:
            _receipt("expired", patch_id=o.get("patch_id"), type=o.get("type"))
            continue
        live.append(o)
        t = o.get("type")
        if t == "route_hold":
            fam = o.get("family") or ""
            if not fam or (family and fam == family):
                out["blocked_routes"].add(o.get("route"))
        elif t == "namespace_hold":
            if not ns or o.get("namespace") == ns:
                out["blocked_ns"] = True
        elif t == "route_debt_preference":
            if out["debt_route"] is None:
                out["debt_route"] = o.get("route")
    out["overlays"] = live
    return out


def consume_debt_preference(route):
    try:
        active = json.loads(OVERLAYS.read_text()) if OVERLAYS.exists() else []
    except Exception:
        return
    cands = sorted((o for o in active
                    if o.get("type") == "route_debt_preference" and o.get("route") == route),
                   key=lambda o: o.get("expires_at") or "")
    if not cands:
        return
    used = cands[0]  # consume ONE identified preference, not every one
    keep = [o for o in active if o is not used and o.get("patch_id") != used.get("patch_id")]
    _write_overlays(keep)
    _receipt("applied", type="route_debt_preference", route=route,
             patch_id=used.get("patch_id"),
             reason="one preferred selection consumed (identified patch)")

# ---------------------------------------------------------------- topup ----

def stage_brief(cfg, ns, budget):
    q = cfg["queue"]
    ygg = "~/.local/bin/ygg-memory.py"
    to = cfg["supply"]["prepare_timeout_s"]
    rc, out, err = run(["ssh", q["host"],
                        f"timeout {to} python3 {ygg} dream --prepare --ns={shlex.quote(ns)} --force --budget {budget} --json"], to + 30)
    root = f"~/.yggterm/memory/dream/{ns}/pending"
    chk, chk_out, _ = run(["ssh", q["host"],
                           f"test -f {root}/BRIEF.md && echo BRIEF-YES || echo BRIEF-NO; "
                           f"test -f {root}/MANIFEST.json && echo MAN-YES || echo MAN-NO"], 20)
    # sol turn-2 repair 5: a failed prepare must not accept a stale brief from
    # an earlier pass, and the manifest is not optional (the contract cites
    # it). Clear the staging dir so the next pass stages fresh.
    if rc != 0 or "BRIEF-YES" not in chk_out or "MAN-YES" not in chk_out:
        run(["ssh", q["host"], f"rm -f {root}/BRIEF.md {root}/MANIFEST.json"], 20)
        return None
    return {"brief": f"{root}/BRIEF.md", "manifest": f"{root}/MANIFEST.json"}


def enqueue(cfg, route, ns, kind, moderate, prompt):
    pf = Path("/tmp") / f"ygg-dream-{uuid.uuid4().hex[:8]}.md"
    pf.write_text(prompt)
    q = cfg["queue"]
    title = f"DREAM[{kind}{'/mod' if moderate else ''}]: {ns} consolidation"
    # ssh JOINS argv into one remote shell string — every variable arg must be
    # shlex-quoted or a title with spaces shatters (measured 2026-10-24).
    # Measured 2026-10-25: "~" -> "$HOME" still fails — shlex.quote wraps the
    # path in SINGLE quotes and $HOME does not expand inside them. Expand to
    # the absolute path locally (fleet convention: pi@/home/pi everywhere).
    repo = q["repo"]
    if repo.startswith("~/"):
        repo = str(HOME) + repo[1:]
    remote = (f"cd {shlex.quote(repo)} && {q['cli']} enqueue"
              f" --route {shlex.quote(route)} --prompt-file {shlex.quote(str(pf))}"
              f" --workspace {shlex.quote(str(HOME / 'gh/zcode-tui'))}"
              f" --title {shlex.quote(title)}")
    up_rc, _, up_err = run(["scp", "-q", str(pf), f"{q['host']}:{pf}"], 30)
    rc, out, err = run(["ssh", q["host"], remote], 90)
    run(["ssh", q["host"], f"rm -f {shlex.quote(str(pf))}"], 20)
    pf.unlink(missing_ok=True)
    if up_rc != 0:
        return False, f"upload failed: {up_err.strip()[:120]}"
    tail = (out + err).strip().splitlines()
    if rc != 0:
        return False, f"rc={rc} " + " | ".join(tail[-3:])
    return True, tail[-1] if tail else "queued"


def cmd_topup(cfg):
    st = load_state()
    today = datetime.now(IST).strftime("%F")
    if st.get("day") != today:
        st = {"day": today, "counts": {}, "moderate": 0, "dry": {}, "staged_shas": {}}
    rows = qjson(cfg, ["list"])
    if rows is None:
        # repair 3: queue state UNKNOWN — hold supply rather than act blind
        print("orchestrator: queue state UNKNOWN (qjson failed) — holding supply")
        return
    backlog = len([r for r in rows if r.get("status") in ("queued", "retry_wait", "cooldown_wait")])
    gates = plane_gates(cfg)
    pol_note = build_overlays(cfg)
    print(f"orchestrator: policy — {pol_note}")
    livable = [r for r in cfg["routes"] if gates[r]["ok"]]
    # v2 PERMISSION gate (sol turn-7 Q3-1): containment/denials/disabled
    # planes block dispatch regardless of floors supply arithmetic
    v2_blocked, v2_note = v2_blocked_routes(cfg["routes"])
    if v2_blocked:
        livable = [r for r in livable if r not in v2_blocked]
    print(f"orchestrator: v2 gate — {v2_note or 'clear'}")
    if not livable:
        print("orchestrator: all planes gated — holding supply (" +
              ", ".join(f"{r}:{gates[r]['why']}" for r in cfg["routes"]) +
              ("; v2: " + v2_note if v2_note else "") + ")")
        return
    enq, t0 = 0, time.time()
    tried_ns = set()  # in-pass exclusions: held/failed/duplicate namespaces do
    # not stay the minimum-count pick and starve the others (sol t7 Q3-4)
    while (sum(st["counts"].values()) + backlog < cfg["targets"]["day"]
           and backlog < cfg["targets"]["backlog_cap"]
           and enq < cfg["supply"]["pass_max_enqueues"]
           and time.time() - t0 < cfg["supply"]["pass_wall_budget_s"]):
        pick = min((n for n in cfg["namespaces"]
                    if n["ns"] not in tried_ns
                    and st["counts"].get(n["ns"], 0) < n["per_day"]),
                   key=lambda n: st["counts"].get(n["ns"], 0), default=None)
        if not pick:
            break
        # policy: namespace holds narrow this pick; route holds narrow the
        # family's livable set (sol contract: holds only narrow eligibility)
        pol = resolve_policy(family="dream", ns=pick["ns"])
        if pol["blocked_ns"]:
            print(f"orchestrator: policy — namespace {pick['ns']} held, skipping")
            st["dry"][pick["ns"]] = st["dry"].get(pick["ns"], 0) + 1
            tried_ns.add(pick["ns"])
            for o in pol.get("overlays") or []:
                _falsify.exposure(_falsify.claim_id(o.get("patch_id")),
                                  decision="skip-ns-held", task_id=None,
                                  policy_changed=True,
                                  detail=f"namespace {pick['ns']} held")
            save_state(st)
            continue
        staged = stage_brief(cfg, pick["ns"], pick["budget"])
        if not staged:
            st["dry"][pick["ns"]] = st["dry"].get(pick["ns"], 0) + 1
            tried_ns.add(pick["ns"])
            save_state(st)
            continue
        # the brief stays in pending until the executor consumes it — without a
        # sha guard every topup pass re-enqueued the SAME brief (measured:
        # medgraph/yggterm queued 2-3x each on 2026-10-24)
        rc2, sha_out, _ = run(["ssh", cfg["queue"]["host"],
                               f"sha256sum {staged["brief"]} 2>/dev/null | cut -d\' \' -f1"], 20)
        brief_sha = (sha_out or "").strip()
        if brief_sha and st.get("staged_shas", {}).get(pick["ns"]) == brief_sha:
            st["dry"][pick["ns"]] = st["dry"].get(pick["ns"], 0) + 1
            tried_ns.add(pick["ns"])
            save_state(st)
            continue
        n_done = st["counts"].get(pick["ns"], 0)
        # route/kind rotate on the GLOBAL dispatch counter: per-ns count is 0
        # for every first staging (dedupe makes it the only one), so the old
        # modulo pinned every dream on livable[0] and starved the other
        # planes — autoclaw went 48h unused (owner dashboard, 2026-10-24).
        total_done = sum(st["counts"].values())
        fam_livable = [r for r in livable if r not in pol["blocked_routes"]]
        if not fam_livable:
            print("orchestrator: policy — all livable routes held for this family, holding supply")
            return
        route = fam_livable[total_done % len(fam_livable)]
        # route_debt_preference: exactly one extra selection for the starved
        # route when healthy and eligible (never overrides a hold; consumed
        # on use — sol contract)
        debt = pol["debt_route"]
        if debt and debt in fam_livable and debt != route:
            route = debt
        kind = cfg["kinds"][n_done % len(cfg["kinds"])]
        moderate = kind in ("verify", "sweep")
        ok, line = enqueue(cfg, route, pick["ns"], kind, moderate,
                           CONTRACT.format(ns=pick["ns"], kind=kind, **staged))
        if ok:
            enq += 1
            st["counts"][pick["ns"]] = n_done + 1
            st["moderate"] += 1 if moderate else 0
            # sol repair 5: record the staged sha only AFTER the enqueue
            # succeeded — a pre-success record suppressed every retry
            if brief_sha:
                st.setdefault("staged_shas", {})[pick["ns"]] = brief_sha
            if debt == route:
                consume_debt_preference(route)
            save_state(st)
            print(f"orchestrator: dispatched {pick['ns']} on {route}")
        else:
            print(f"orchestrator: enqueue failed: {line}", file=sys.stderr)
            break
        rows = qjson(cfg, ["list"])
        if rows is not None:
            backlog = len([r for r in rows if isinstance(r, dict) and r.get("status") in ("queued", "retry_wait", "cooldown_wait")])
    print(f"orchestrator: topup staged={sum(st['counts'].values())} moderate={st['moderate']} backlog={backlog}")


# ----------------------------------------------------------- supervise ----

QUOTA_SIGNS = ("quota", "usage limit", "rate limit", "429", "hourly", "cap reached", "free tier")
AUTH_SIGNS = ("auth", "unauthorized", "401", "refresh token", "login")


EXPERIMENT_MARKS = ("CANARY", "FALSIFIER", "PROBE", "TEST", "SMOKE")


def falsify_pass():
    """The claim->evaluation->disposition consumer (sol turn-7 Q2c, minimal
    increment). For every OPEN claim past its observation deadline: pull the
    affected route's audit rows inside the window; one idempotent evaluation;
    then retain (supported) / rollback (falsified) / expire (undetermined)."""
    notes = []
    try:
        now = datetime.now(IST)
        for c in _falsify.open_claims():
            dl_min = c.get("observation_deadline_min") or 60
            opened = datetime.fromisoformat(c["opened_at"])
            if (now - opened).total_seconds() < dl_min * 60:
                continue  # window still open
            route = (c.get("scope") or {}).get("route") or ""
            w_end = now.isoformat(timespec="seconds")
            rows = [json.loads(x) for x in open(DREAM_LEDGER).read_text().splitlines()] if DREAM_LEDGER.exists() else []
            exposed = [r for r in rows if r.get("route") == route
                       and r.get("created_at", "") >= c["opened_at"]]
            if not exposed:
                _falsify.consume(c["claim_id"], None, action="expire")
                _falsify.mark_consumed(c["claim_id"])
                notes.append(f"claim {c['claim_id']} expired (no exposure)")
                continue
            # outcomes from the audit ledger for those task ids
            tasks = {r.get("id") for r in exposed}
            failed = outcome = 0
            if AUDIT_LEDGER.exists():
                for x in AUDIT_LEDGER.read_text().splitlines():
                    try:
                        a = json.loads(x)
                    except Exception:
                        continue
                    if a.get("id") in tasks and a.get("terminal"):
                        outcome += 1
                        if a.get("verdict") in ("FAIL", "UNKNOWN") or a.get("issues"):
                            failed += 1
            ev = _falsify.evaluate(c["claim_id"], window_start=c["opened_at"],
                                   window_end=w_end, route=route,
                                   exposed_tasks=len(exposed), outcome_tasks=outcome,
                                   failed_tasks=failed)
            if ev["disposition"] == "falsified":
                _falsify.consume(c["claim_id"], ev, action="rollback")
                notes.append(f"claim {c['claim_id']} FALSIFIED ({ev['verdict']}) — patch rolled back")
            elif ev["disposition"] == "supported":
                _falsify.consume(c["claim_id"], ev, action="retain")
                notes.append(f"claim {c['claim_id']} supported ({ev['verdict']}) — retained")
            else:
                _falsify.consume(c["claim_id"], ev, action="expire")
                notes.append(f"claim {c['claim_id']} undetermined — expired")
            _falsify.mark_consumed(c["claim_id"])
    except Exception as e:  # noqa: BLE001 — falsification failures never stop supervision
        notes.append(f"falsify_pass fault: {e!r}")
    return notes


def cmd_supervise(cfg):
    ledger = {"t": int(time.time()), "audited": 0, "revoked": [], "rerouted": [], "held": [], "notes": []}
    try:  # consumer acknowledgement of the active revision (sol turn-7 #5)
        from swarmharness import swarmconfig as _sc2
        _sc2.ack("dream-orchestrator")
    except Exception:
        pass
    for fnote in falsify_pass():
        ledger["notes"].append(fnote)
        print(f"orchestrator: falsify — {fnote}")
    rows = qjson(cfg, ["list"])
    if rows is None:
        # sol repair 3/6: unknown queue state — take NO mutating action
        print("orchestrator: supervise — queue state UNKNOWN, skipping mutations")
        LEDGER.parent.mkdir(parents=True, exist_ok=True)
        with LEDGER.open("a") as f:
            f.write(json.dumps(ledger) + "\n")
        return
    gates = plane_gates(cfg)
    day_ago_ms = int(time.time() * 1000) - 24 * 3600 * 1000
    seen_notes = set()
    for r in rows:
        low = (r.get("last_error") or "").lower()
        if r.get("status") != "failed":
            continue
        # one-shot experiments stay outside automatic rerouting (sol repair 6)
        if any(k in (r.get("title") or "").upper() for k in EXPERIMENT_MARKS):
            continue
        if (r.get("updated_at") or 0) < day_ago_ms:
            continue  # history, not news
        if r["id"] in seen_notes:
            continue
        seen_notes.add(r["id"])
        if any(s in low for s in QUOTA_SIGNS):
            alt = next((p for p in cfg["routes"] if p != r["route"] and gates[p]["ok"]), None)
            if alt:
                rc, _, _ = run(queue_cli(cfg) + ["reroute", r["id"], "--to", alt, "--json"], 60)
                # sol repair 6: mutations are verified by readback, not by rc
                verified = False
                if rc == 0:
                    chk = qjson(cfg, ["list"])
                    for t2 in (chk or []):
                        if t2.get("id") == r["id"]:
                            verified = t2.get("route") == alt or t2.get("status") in ("retry_wait", "cooldown_wait", "queued")
                if verified:
                    ledger["rerouted"].append({"id": r["id"], "to": alt, "verified": True, "why": (r.get("last_error") or "")[:80]})
                    print(f"supervise: rerouted {r['id'][:16]} {r['route']} -> {alt} (quota death, readback ok)")
                elif rc == 0:
                    ledger["notes"].append({"id": r["id"], "why": f"reroute rc=0 but readback unverified ({alt})"})
            else:
                run(queue_cli(cfg) + ["hold", r["id"], "--until-min", "60", "--json"], 60)
                ledger["held"].append({"id": r["id"], "min": 60, "why": "all planes gated"})
                print(f"supervise: held {r['id'][:16]} 60m (all planes gated)")
        elif any(s in low for s in AUTH_SIGNS):
            ledger["notes"].append({"id": r["id"], "why": f"auth death needs ygg-auth: {(r.get('last_error') or '')[:80]}"})
    # false-completion audit: completed runs whose event stream shows an
    # error turn with zero tokens riding rc=0 (the G1 class)
    done = [r for r in rows if r.get("status") == "completed"
            and not any(k in (r.get("title") or "").upper() for k in EXPERIMENT_MARKS)]
    random.shuffle(done)
    q = cfg["queue"]
    for r in done[: cfg["false_completion"]["audit_per_pass"]]:
        ledger["audited"] += 1
        # sol repair 6: the false-completion verdict must tie the two markers
        # to ONE terminal event — '"stopReason":"error"' with '"totalTokens":0'
        # immediately following — not two substrings anywhere in a log tail.
        rc, log, _ = run(["ssh", q["host"],
                          f"tail -c 400000 ~/.local/state/zcode-tui/queue/runs/{r['id']}*.json.log 2>/dev/null || true"], 30)
        flat = log.replace(" ", "")
        i = flat.rfind('"stopReason":"error"')
        terminal_zero = i >= 0 and '"totalTokens":0' in flat[i:i + 200]
        if terminal_zero:
            run(queue_cli(cfg) + ["revoke", r["id"], "--reason", "false-completion: zero-token error turn (terminal event)", "--json"], 60)
            ledger["revoked"].append({"id": r["id"], "evidence": "terminal event"})
            print(f"supervise: REVOKED false completion {r['id'][:16]}")
        elif i < 0:
            # no terminal error marker at all — nothing to act on; note only
            # when the transcript itself is MISSING (a completed task with no
            # log is the anomaly worth recording)
            ledger["notes"].append({"id": r["id"], "why": "false-completion check: no transcript found"})
    # UX-parity watch (owner 2026-10-24): the TUI parity dreamer runs 24x7 —
    # if no parity task is alive on any plane, spawn the next one from the
    # standing contract so the work never sleeps.
    watch = cfg.get("ux_parity_watch") or {}
    if watch.get("enabled"):
        # sol repair 4: the watch is a dispatch path and goes through the SAME
        # policy resolver and plane gates as topup — no side door. A family-
        # scoped route_hold (the first pilot patch: ux_parity x autoclaw auth
        # wall) makes the watch WAIT; its next natural pass after expiry is
        # the bounded recovery check. It never silently re-routes planes.
        alive = [r for r in rows if "UX-PARITY" in (r.get("title") or "").upper()
                 and r.get("status") in ("queued", "running", "retry_wait", "cooldown_wait")]
        if not alive:
            pol = resolve_policy(family="ux_parity")
            route = watch.get("route", "autoclaw")
            gate = gates.get(route, {})
            if route in pol["blocked_routes"]:
                ledger["notes"].append({"ux_watch": f"route {route} held by policy — waiting (recovery check after expiry)"})
                print(f"supervise: UX-parity watch — {route} under policy hold, waiting")
            elif gate and not gate.get("ok", True):
                ledger["notes"].append({"ux_watch": f"route {route} plane-gated ({gate.get('why')})"})
                print(f"supervise: UX-parity watch — {route} gated, waiting")
            else:
                pf = watch.get("prompt_file")
                chk, chk_out, _ = run(["ssh", cfg["queue"]["host"], f"test -f {pf} && echo YES || echo NO"], 15)
                if "YES" in chk_out:
                    remote = (f"cd {cfg["queue"]["repo"]} && {cfg["queue"]["cli"]} enqueue"
                              f" --route {route} --prompt-file {pf}"
                              f" --workspace {watch.get("workspace", "/home/pi/gh/zcode-tui--ux-sol-2")}"
                              f" --title 'UX-PARITY continuous: opencode-indistinguishable TUI (autoclaw 24x7)'"
                              f" --max-attempts 3")
                    rc3, out3, _ = run(["ssh", cfg["queue"]["host"], remote], 90)
                    if rc3 == 0:
                        ledger["notes"].append({"spawned": "ux-parity", "route": route})
                        print(f"supervise: UX-parity dreamer not alive — respawned on {route}")
                    else:
                        ledger["notes"].append({"spawn_failed": "ux-parity", "err": (out3 or "")[-120:]})
                else:
                    ledger["notes"].append({"ux_watch": f"prompt file missing: {pf}"})
    LEDGER.parent.mkdir(parents=True, exist_ok=True)
    with LEDGER.open("a") as f:
        f.write(json.dumps(ledger) + "\n")
    print(f"orchestrator: supervise audited={ledger['audited']} revoked={len(ledger['revoked'])} "
          f"rerouted={len(ledger['rerouted'])} held={len(ledger['held'])}")
    # P1 information system v1: regenerate the empire index (the bounded view
    # every intelligence consult embeds). Deterministic; never fatal.
    try:
        from swarmharness import empire_index  # noqa: E402  # via the kernel package
        idx_path = ((SWCFG or {}).get("intelligence") or {}).get(
            "empire_index", "~/.yggterm/dream/empire-index.md")
        empire_index.build(SWCFG, load_cfg(), rows, None, idx_path)
    except Exception as e:
        print(f"orchestrator: empire index write failed: {e}", file=sys.stderr)


def cmd_status(cfg, as_json=False):
    st = load_state()
    gates = plane_gates(cfg)
    rows = qjson(cfg, ["list"])
    if rows is None:
        rows = []  # UNKNOWN renders as zeros + a flag, never a crash
    backlog = len([r for r in rows if isinstance(r, dict) and r.get("status") in ("queued", "retry_wait", "cooldown_wait")])
    running = len([r for r in rows if isinstance(r, dict) and r.get("status") == "running"])
    pol = resolve_policy()
    out = {
        "armed": ENABLED.exists(),
        "policy": {"active_overlays": len(pol["overlays"]),
                   "blocked_routes": sorted(pol["blocked_routes"]),
                   "debt_route": pol["debt_route"]},
        "today": {"staged": sum(st.get("counts", {}).values()), "moderate": st.get("moderate", 0),
                  "targets": cfg["targets"]},
        "planes": gates,
        "queue": {"backlog": backlog, "running": running},
        "dry": st.get("dry", {}),
    }
    if as_json:
        print(json.dumps(out, indent=1))
        return
    t = cfg["targets"]
    print(f"dream orchestrator — armed={out['armed']} staged={out['today']['staged']}/{t['day']} "
          f"moderate={out['today']['moderate']}/{t['moderate']} backlog={backlog} running={running}")
    for r in cfg["routes"]:
        g = gates[r]
        state = "OK" if g["ok"] else f"GATED {g['why']} ({g['retry_after_min']}m)"
        print(f"  {r:16} {state}")


def load_swarmconfig():
    """The survival ledger v2 (activation store; workspace edits are proposals,
    operational truth is active.json). Never fatal: containment returns None
    and the deterministic core runs floors-only, reporting the fault loudly."""
    here = Path(__file__).resolve()
    candidates = [
        here.parents[3] / "tools",                              # repo SSOT run
        here.parent,                                            # dispersed: kernel beside the verb
        Path.home() / "gh" / "yggterm" / "tools",               # dispersed, repo present
    ]
    for kpath in candidates:
        if (kpath / "swarmharness" / "__init__.py").exists():
            sys.path.insert(0, str(kpath))  # parent: the package is swarmharness/
            break
    try:
        from swarmharness import swarmconfig  # noqa: E402
        res = swarmconfig.load()  # LoadResult: mode/config/revision/faults/denials
        faults = "; ".join(f.get("detail", f["code"]) for f in res.get("faults", [])[:3])
        if res["mode"] == "containment_only":
            print(f"orchestrator: swarmconfig CONTAINMENT-ONLY ({faults}) — floors-only",
                  file=sys.stderr)
            return None
        if res["mode"] == "last_good" or faults:
            print(f"orchestrator: swarmconfig {res['mode']} ({faults})", file=sys.stderr)
        denials = res.get("denials") or {}
        if denials:
            print(f"orchestrator: owner denials active: {sorted(denials)}", file=sys.stderr)
        return res["config"]
    except Exception as e:
        print(f"orchestrator: swarmconfig unavailable: {e}", file=sys.stderr)
        return None


SWCFG = None  # bound in main; read by supervise for the empire index


def main():
    global SWCFG
    verb = sys.argv[1] if len(sys.argv) > 1 else "status"
    cfg = load_cfg()
    SWCFG = load_swarmconfig()
    if verb == "status":
        cmd_status(cfg, "--json" in sys.argv)
    elif verb in ("topup", "supervise"):
        # sol turn-2 repair 1: the arming gate was NEVER enforced (main went
        # straight to the verbs) and the lock handle was discarded, releasing
        # the flock at once. Both bound here; `_lock` holds the fd for life.
        _lock = acquire_lock()
        if not armed():
            sys.exit(0)
        if verb == "topup":
            cmd_topup(cfg)
        else:
            cmd_supervise(cfg)
    else:
        print("usage: ygg-dream-orchestrator.py <status|topup|supervise> [--json]")
        sys.exit(2)


if __name__ == "__main__":
    main()
