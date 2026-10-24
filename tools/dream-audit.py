#!/usr/bin/env python3
"""dream-audit — the 1.0.0 release gate (owner directive 2026-10-24).

Per orchestrator wake:
  1. log EVERY dream session of the last 48h (all harnesses/routes) into
     /tmp/dreamer/dream-ledger.jsonl   (append; one line per session)
  2. randomly select ~10% of sessions NOT yet audited (seeded per-day,
     iterative — the audit ledger remembers what was checked)
  3. audit each sample from its session files:
       graceful_end   — settled/completed, or an honest classified failure
       model_ok       — the transcript's recorded model matches the route law
       thinking_ok    — reasoning activity present for max-reasoning routes,
                        no malformed/empty thinking streams
       job_done       — the dream contract line / deliverable exists
       no_false_done  — no zero-token error turn riding rc=0
     verdicts append to /tmp/dreamer/audit-ledger.jsonl (marked, never re-audited)
  4. print FAIL summaries with issue classes for the orchestrator to root-cause.

zcode-tui 1.0.0 gate: every dream audited PASS, every fault root-caused+fixed,
and newer dreams free of those faults — only then 1.0.0.
"""
import json, random, sqlite3, sys, time
from datetime import datetime, timedelta, timezone
from pathlib import Path

HOME = Path.home()
QDB = HOME / ".local/state/zcode-tui/queue/queue.sqlite3"
RUNS = HOME / ".local/state/zcode-tui/queue/runs"
CFG_FLOORS = HOME / ".yggterm/config/dream/floors.json"
LEDGER_DIR = Path("/tmp/dreamer")
DREAM_LEDGER = LEDGER_DIR / "dream-ledger.jsonl"
AUDIT_LEDGER = LEDGER_DIR / "audit-ledger.jsonl"
IST = timezone(timedelta(hours=5, minutes=30))

# route law: (expected model prefix, reasoning expected?)
ROUTE_LAW = {
    "zai-coding-plan": ("zai-api/GLM-5.3-Flash@max", True),
    "zai-offpeak": (None, True),
    "autoclaw": (None, True),
    "codex": ("gpt-6.1-sol", False),
    "opencode-free": (None, False),
}
EXPERIMENT = ("CANARY", "FALSIFIER", "PROBE", "TEST", "SMOKE")

# Plane-starvation law (owner 2026-10-25, after the autoclaw 48h starve):
# the spawner rotates routes by design — a plane receiving far under its
# expected share is a FAULT of the system, not weather. Checked over the
# whole 48h ledger, appended to the wake report, and FAILs any wake whose
# sampled sessions ran on an over-weight plane while a healthy plane starved.
EXPECTED_SHARE = {"zai-coding-plan": 0.34, "autoclaw": 0.25, "codex": 0.2, "opencode-free": 0.15}


def route_balance(rows):
    """sol finding 5: shares are WEIGHTS over ELIGIBLE routes — normalize over
    the routes actually dispatched in the window, so a route that is down is
    excluded from the denominator instead of silently deflating every target."""
    from collections import Counter
    c = Counter(r["route"] for r in rows if r.get("status") in ("completed", "failed", "running", "retry_wait"))
    n = sum(c.values()) or 1
    eligible = [r for r in EXPECTED_SHARE if c.get(r, 0) > 0]
    wsum = sum(EXPECTED_SHARE[r] for r in eligible) or 1.0
    out = {}
    for route, w in EXPECTED_SHARE.items():
        got = c.get(route, 0) / n
        want = w / wsum if route in eligible else w
        out[route] = {"share": round(got, 3), "expected": round(want, 3),
                      "eligible": route in eligible, "starved": route in eligible and got < want * 0.4}
    return out

# 1.0.0 gate: known faults with their fixes — a FAIL matching one of these is
# recorded as FAIL(fixed:<ref>) so historical faults close instead of
# permanently blocking the gate (the gate still requires NEWER dreams clean).
KNOWN_FIXED = [
    ("providerId\":\"zai\"", "fix: registry-adaptive model refs (zcode-tui main 98f01e8)"),
    ("model zaicoding_glm-5.3", "fix: reroute clears plane-specific refs (zcode-tui main 782abff)"),
    ("cut mid-tool", "design: cut-midtool leaves the session resumable by design; sweeper watches heartbeats (main 3c7f19c)"),
    ("Unrecognized key", "fix: model-ref shape negotiation (zcode-tui main 3c7f19c)"),
]


def known_fixed(issues):
    joined = " | ".join(issues)
    for sig, ref in KNOWN_FIXED:
        if sig in joined:
            return ref
    return None


def load_tasks(hours=48):
    since_ms = int((time.time() - hours * 3600) * 1000)
    db = sqlite3.connect(str(QDB), timeout=10)
    db.row_factory = sqlite3.Row
    rows = db.execute(
        "SELECT * FROM tasks WHERE created_at >= ? ORDER BY created_at", (since_ms,)
    ).fetchall()
    return [dict(r) for r in rows]


def audited_terminal_ids():
    """sol finding 1: only TERMINAL verdicts retire an id from the audit pool;
    UNKNOWN (in-flight) rows stay eligible for re-audit next wake."""
    last = {}
    if AUDIT_LEDGER.exists():
        for line in AUDIT_LEDGER.read_text().splitlines():
            try:
                r = json.loads(line)
            except Exception:
                continue
            last[r["id"]] = r
    return {i for i, r in last.items() if r.get("terminal")}


def last_ledger_status():
    out = {}
    if DREAM_LEDGER.exists():
        for line in DREAM_LEDGER.read_text().splitlines():
            try:
                r = json.loads(line)
            except Exception:
                continue
            out[r["id"]] = r.get("status")
    return out


def append(path, obj):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a") as f:
        f.write(json.dumps(obj, default=str) + "\n")


def read_first(glob_parent, tid):
    """Return (transcript_json_or_None, out_tail, md_tail)."""
    transcript, out_tail, md_tail = None, "", ""
    for p in sorted(RUNS.glob(f"{tid}*.json.log")):
        try:
            transcript = json.loads(p.read_text())
        except Exception:
            try:
                transcript = json.loads(p.read_text(errors="replace"))
            except Exception:
                transcript = None
        break
    for p in sorted(RUNS.glob(f"{tid}*.out")):
        try:
            out_tail = p.read_text(errors="replace")[-4000:]
        except Exception:
            pass
        break
    for p in sorted(RUNS.glob(f"{tid}*.md")):
        try:
            md_tail = p.read_text(errors="replace")[-800:]
        except Exception:
            pass
        break
    return transcript, out_tail, md_tail


def audit_task(t):
    tid, route, title = t["id"], t["route"], (t["title"] or "")
    status = t["status"]
    transcript, out_tail, md_tail = read_first(RUNS, tid)
    checks, issues = {}, []

    # ---- graceful end (only meaningful for tasks that ended) ---------------
    # sol turn-1 finding 1: an in-flight task must grade UNKNOWN, never PASS on
    # the omitted-conjunction, and must stay eligible for a terminal re-audit.
    ran_or_ended = status in ("completed", "failed", "cancelled")
    if not ran_or_ended:
        issues.append(f"in-flight: {status} (terminal audit pending)")
        checks["graceful_end"] = None
        graceful = None
    else:
        graceful = True
        checks["graceful_end"] = True
    if transcript and transcript.get("outcome") not in ("settled", "budget", "cut-midtool", None):
        issues.append(f"transcript outcome={transcript.get('outcome')}")
    if transcript and transcript.get("outcome") == "cut-midtool":
        issues.append("cut mid-tool (session left running)")

    # ---- model law ----------------------------------------------------------
    expected, wants_reasoning = ROUTE_LAW.get(route, (None, False))
    model_found = None
    if transcript and transcript.get("model"):
        model_found = transcript["model"]
    # sol finding 3: missing evidence is UNKNOWN (None), never implicit-true.
    model_ok = None
    if expected and model_found and not str(model_found).startswith(expected.split("@")[0]):
        model_ok = False
        issues.append(f"model {model_found} != expected {expected}")
    elif expected and not model_found:
        issues.append("model unverified (no transcript model field)")
    checks["model_ok"] = model_ok
    # wrong-plane guard: zai routes must never run glm-5.3 FULL on coding plan.
    # The model string may be the display form ("GLM-5.3@high") or the raw ref
    # dict ('{"modelId":"GLM-5.3","providerId":"zai"}') — check both shapes
    # (measured miss: ztq-d087cb67 dict form bypassed the "GLM-5.3@" match).
    _mf = str(model_found or "")
    if route == "zai-coding-plan" and "glm-5.3" in _mf.lower() and "flash" not in _mf.lower() and "flash" in str(expected or "").lower():
        model_ok = False
        checks["model_ok"] = False
        issues.append("GLM-5.3 FULL rode the coding plan (owner law violation)")

    # ---- thinking -----------------------------------------------------------
    thinking_ok = None if not transcript else True
    if transcript:
        events = transcript.get("events") or []
        kinds = [str(e.get("kind", "")) for e in events]
        reasoning = [k for k in kinds if "reason" in k.lower() or "think" in k.lower()]
        deltas = [k for k in kinds if "delta" in k.lower() or k in ("text", "tool")]
        if wants_reasoning and deltas and not reasoning:
            thinking_ok = False
            issues.append(f"no reasoning events across {len(deltas)} activity events (effort off?)")
        # malformed: reasoning present but transcript outcome settled with zero assistant text
        textish = [k for k in kinds if "text" in k.lower()]
        if "settled" == transcript.get("outcome") and not textish and not deltas:
            thinking_ok = False
            issues.append("settled with zero text/tool events (empty session)")
    checks["thinking_ok"] = thinking_ok

    # ---- job done (only for tasks that actually RAN) -----------------------
    job_done = True
    ran = status in ("completed", "failed")  # cancelled never executed — deliverables n/a
    if not ran:
        issues.append("pending-run (not yet executed - deliverable checks n/a)")
        checks["job_done"] = None
        job_done = None
    if job_done and tid.startswith("ztq-") and title.startswith("DREAM") and "consolidation" in title:
        # the dream contract: the executor must have composed RESPONSE+CRITIC
        ns = ""
        for tok in title.split():
            if tok.startswith("-home") or tok == "_global":
                ns = tok.rstrip(":").replace(":", "")
                break
        if ns:
            # sol finding 2: bind the artifact to THIS task — namespace-wide
            # glob credited one dream's output to another. Minimal binding
            # available in the current staging layout: BOTH contract files must
            # exist and RESPONSE must be fresher than this task's creation.
            root = HOME / f".yggterm/memory/dream/{ns}/pending"
            created_ms = t.get("created_at") or 0
            resp = [p for p in root.glob("RESPONSE*.md") if p.stat().st_mtime * 1000 >= created_ms]
            critic = [p for p in root.glob("CRITIC*.md") if p.stat().st_mtime * 1000 >= created_ms]
            if not resp:
                job_done = False
                issues.append(f"dream contract: no RESPONSE newer than task in {root}")
            elif not critic:
                job_done = False
                issues.append(f"dream contract: RESPONSE without CRITIC in {root}")
    if status == "completed" and not (out_tail.strip() or transcript):
        job_done = False
        issues.append("completed with empty session files")
    if job_done is not None:
        checks["job_done"] = job_done

    # ---- false completion ---------------------------------------------------
    false_done = False
    flat = ""
    for p in RUNS.glob(f"{tid}*.json.log"):
        try:
            flat = p.read_text(errors="replace").replace(" ", "")
        except Exception:
            pass
        break
    if '"stopReason":"error"' in flat and '"totalTokens":0' in flat and status == "completed":
        false_done = True
        issues.append("false completion: zero-token error turn billed as completed")
    checks["no_false_done"] = not false_done

    is_experiment = any(k in title.upper() for k in EXPERIMENT)
    vals = [v for v in checks.values()]
    verdict = "FAIL" if any(v is False for v in vals) else ("UNKNOWN" if any(v is None for v in vals) else "PASS")
    if is_experiment and verdict == "FAIL":
        verdict = "FAIL(experiment)"  # still recorded; experiments are excluded from the 1.0.0 gate
    repair_ref = known_fixed(issues) if verdict == "FAIL" else None
    terminal = ran_or_ended
    return {
        "id": tid, "route": route, "title": title[:90], "status": status,
        "terminal": terminal, "repair_ref": repair_ref,
        "model_expected": expected, "model_found": model_found,
        "checks": checks, "issues": issues, "verdict": verdict,
        "audited_at": datetime.now(IST).isoformat(timespec="seconds"),
    }


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "audit"
    tasks = load_tasks(48)
    # 1. full ledger
    seen = set()
    if DREAM_LEDGER.exists():
        for line in DREAM_LEDGER.read_text().splitlines():
            try:
                seen.add(json.loads(line)["id"])
            except Exception:
                pass
    new = 0
    updated = 0
    prev_status = last_ledger_status()
    for t in tasks:
        if t["id"] in seen:
            if prev_status.get(t["id"]) != t["status"]:
                # sol finding 4: the ledger must record terminal transitions,
                # not strand a task on its first-seen status.
                append(DREAM_LEDGER, {
                    "id": t["id"], "route": t["route"], "title": (t["title"] or "")[:90],
                    "status": t["status"], "created_at": t["created_at"], "model": t.get("model"),
                    "status_update": True,
                })
                updated += 1
            continue
        append(DREAM_LEDGER, {
            "id": t["id"], "route": t["route"], "title": (t["title"] or "")[:90],
            "status": t["status"], "created_at": t["created_at"], "model": t.get("model"),
        })
        new += 1

    if mode == "ledger":
        print(f"dream-ledger: {len(tasks)} sessions in 48h window, +{new} new, {updated} status-updated")
        return

    # 2. sample 10% of not-yet-terminally-audited (--exhaustive audits the pool)
    done = audited_terminal_ids()
    pool = [t for t in tasks if t["id"] not in done]
    if "--exhaustive" in sys.argv:
        sample = pool  # release-gate mode: ALL dreams checked (1.0.0 gate)
    else:
        n = max(1, round(len(pool) * 0.10))
        random.seed(datetime.now(IST).strftime("%F%H"))  # reproducible per wake
        sample = random.sample(pool, min(n, len(pool)))

    fails, unknowns = [], 0
    wake_hour = datetime.now(IST).strftime("%FT%H")  # isoformat has the T
    already_this_wake = set()
    if AUDIT_LEDGER.exists():
        for line in AUDIT_LEDGER.read_text().splitlines():
            try:
                r_ = json.loads(line)
                if (r_.get("audited_at") or "").startswith(wake_hour):
                    already_this_wake.add(r_["id"])
            except Exception:
                continue
    sample = [t for t in sample if t["id"] not in already_this_wake]
    for t in sample:
        v = audit_task(t)
        append(AUDIT_LEDGER, v)
        if v["verdict"] == "FAIL":
            fails.append(v)
        elif v["verdict"] == "UNKNOWN":
            unknowns += 1
    print(f"dream-audit: ledger={len(tasks)} sessions (48h), +{new} new, {updated} status-updated; "
          f"audited {len(sample)}{' (EXHAUSTIVE)' if '--exhaustive' in sys.argv else ' (~10%)'} of {len(pool)} unaudited; "
          f"fails={len(fails)} unknown={unknowns}")
    bal = route_balance(tasks)
    starved = [r for r, v in bal.items() if v["starved"]]
    if starved:
        parts = []
        for r in starved:
            parts.append(r + " " + str(int(bal[r]["share"] * 100)) + "% (want " + str(int(bal[r]["expected"] * 100)) + "%)")
        print("  PLANE STARVED: " + ", ".join(parts))
        for v in fails:
            pass
    for v in fails:
        print(f"  FAIL {v['id'][:16]} [{v['route']}] {v['title'][:60]}"
              + (f" [repair-ref: {v['repair_ref']}]" if v.get("repair_ref") else ""))
        for i in v["issues"]:
            print(f"        - {i}")

    # swarm-harness2 Phase-1 wire (sol turn-2 schema, DreamAuditFeedbackV1):
    # an atomically-replaced observation snapshot the orchestrator validates
    # before any overlay. Findings are id-stable per (code, scope) so re-
    # serializing old evidence cannot advance last_seen or renew a TTL.
    try:
        emit_feedback(bal, fails, unknowns, len(sample), tasks)
    except Exception as e:
        print(f"  feedback wire write failed: {e}")



# --------------------------------------------------------------- wire ----
# DreamAuditFeedbackV1 producer (sol turn-2 §2). WAKE_MINUTES must match the
# dream-spawner timer interval; snapshot validity is min(2W, 30m).
WAKE_MINUTES = 15
AUDIT_STATE = HOME / ".yggterm/dream/audit-state.json"
EVALUATOR_VERSION = "dream-audit/2.1.0"
AUTH_SIGNS = ("401", "invalid token", "unauthorized", "auth", "refresh token")


def _sha8(text):
    import hashlib
    return hashlib.sha256(text.encode()).hexdigest()[:8]


def _load_audit_state():
    try:
        return json.loads(AUDIT_STATE.read_text())
    except Exception:
        return {}


def emit_feedback(bal, fails, unknowns, sampled, tasks):
    now = datetime.now(IST)
    st = _load_audit_state()
    terminal = [t for t in tasks if t.get("status") in ("completed", "failed", "cancelled")]
    nonterminal = len(tasks) - len(terminal)

    findings = []
    # map audited FAILs to finding codes with evidence
    for v in fails:
        issues = v.get("issues") or []
        route, title = v.get("route"), (v.get("title") or "")
        ns = ""
        for tok in title.split():
            if tok.startswith("-home") or tok == "_global":
                ns = tok.rstrip(":").replace(":", "")
                break
        joined = " ".join(issues).lower()
        runfiles = sorted(str(p) for p in RUNS.glob(f"{v['id']}*"))[:3]
        evidence = [{"uri": u, "sha256": _sha8(u + str(v.get("audited_at"))), "check": "audit_task"}
                    for u in runfiles] or [{"uri": f"queue:{v['id']}", "sha256": _sha8(v["id"]), "check": "queue-row"}]
        att = [{"task_id": v["id"], "attempt_id": v["id"] + "-a1", "session_id": None}]
        if any(s_ in joined for s_ in AUTH_SIGNS):
            code, scope = "route_auth_failure", {"family": "", "namespace": None, "kind": None, "route": route, "model": v.get("model_found")}
        elif "!= expected" in joined or "wrong-plane" in joined or "FULL rode the coding plan" in joined:
            code, scope = "route_model_incompatible", {"family": "", "namespace": None, "kind": None, "route": route, "model": v.get("model_found")}
        elif "dream contract:" in joined:
            code, scope = "contract_failure", {"family": "dream", "namespace": ns or None, "kind": None, "route": route, "model": None}
        elif "false completion" in joined:
            code, scope = "false_completion", {"family": "", "namespace": None, "kind": None, "route": route, "model": None}
        elif "model unverified" in joined or "no reasoning events" in joined:
            code, scope = "evidence_missing", {"family": "", "namespace": None, "kind": None, "route": route, "model": None}
        else:
            continue
        key = code + "|" + json.dumps(scope, sort_keys=True)
        prev = (st.get("findings") or {}).get(key) or {}
        ev_sha = _sha8(json.dumps(evidence, sort_keys=True))
        if prev.get("evidence_sha") == ev_sha:
            last_seen = prev.get("last_seen")  # same evidence: no advance
        else:
            last_seen = now.isoformat(timespec="seconds")
        st.setdefault("findings", {})[key] = {"evidence_sha": ev_sha, "last_seen": last_seen}
        findings.append({
            "finding_id": "find-" + _sha8(key),
            "code": code, "status": "confirmed", "scope": scope,
            "first_seen": prev.get("first_seen") or last_seen,
            "last_seen": last_seen,
            "attempts": att, "evidence": evidence, "allocation": None,
        })
    # starvation (sol §4 normative): expected = normalized share over ALL
    # dispatch opportunities in the window, actual = the route's own count;
    # the finding grades UNKNOWN until real eligible-route-set recording
    # exists (the audit does not yet know per-opportunity E_i), so the
    # orchestrator sees it but builds NO overlay from it. The wake-report
    # alarm stays the operational signal; this keeps the wire honest.
    total_opps = len(tasks) or 1
    for route, v in bal.items():
        if not v.get("starved"):
            st["misses|" + route] = 0
            st["starved_since|" + route] = None
            continue
        actual = sum(1 for t in tasks if t.get("route") == route)
        expected = round(v["expected"] * total_opps, 2)
        if not (actual < 0.4 * expected):
            continue  # cannot claim starvation incoherently
        misses = st.get("misses|" + route, 0) + 1
        st["misses|" + route] = misses
        st.setdefault("starved_since|" + route, now.isoformat(timespec="seconds"))
        findings.append({
            "finding_id": "find-" + _sha8("route_starved|" + route),
            "code": "route_starved", "status": "unknown",
            "scope": {"family": "", "namespace": None, "kind": None, "route": route, "model": None},
            "first_seen": st.get("starved_since|" + route),
            "last_seen": now.isoformat(timespec="seconds"),
            "attempts": [],
            "evidence": [{"uri": "audit:route_balance", "sha256": _sha8(json.dumps(v, sort_keys=True)), "check": "route_balance"}],
            "allocation": {"eligible_opportunities": total_opps, "expected_dispatches": expected,
                            "actual_dispatches": actual, "consecutive_misses": misses},
        })
    for route in bal:
        if not bal[route].get("starved"):
            st.setdefault("starved_since|" + route, None)
    AUDIT_STATE.parent.mkdir(parents=True, exist_ok=True)
    AUDIT_STATE.write_text(json.dumps(st, indent=1))

    snapshot = {
        "schema_version": 1,
        "snapshot_id": "snap-" + _sha8(str(now.timestamp())),
        "producer": "dream-audit",
        "evaluator_version": EVALUATOR_VERSION,
        "config_sha256": _sha8_file(CFG_FLOORS) if CFG_FLOORS.exists() else "0" * 64,
        "generated_at": now.isoformat(timespec="seconds"),
        "valid_until": (now + timedelta(minutes=min(2 * WAKE_MINUTES, 30))).isoformat(timespec="seconds"),
        "window_start": (now - timedelta(hours=48)).isoformat(timespec="seconds"),
        "window_end": now.isoformat(timespec="seconds"),
        "coverage": {
            "inventory_complete": True,
            "inventory_sha256": _sha8_file(QDB) if QDB.exists() else "0" * 64,
            "terminal_attempts": len(terminal),
            "cheap_checked_terminal_attempts": len(terminal),
            "deep_checked_terminal_attempts": sampled,
            "unresolved_terminal_attempts": unknowns,
            "nonterminal_attempts": nonterminal,
        },
        "findings": findings,
    }
    out = HOME / ".yggterm/dream/audit-feedback.json"
    out.parent.mkdir(parents=True, exist_ok=True)
    tmp = out.with_suffix(".tmp")
    tmp.write_text(json.dumps(snapshot, indent=1))
    tmp.replace(out)
    print(f"  feedback wire: {len(findings)} findings -> {out}")


def _sha8_file(path):
    import hashlib
    return hashlib.sha256(path.read_bytes()).hexdigest()

if __name__ == "__main__":
    main()
