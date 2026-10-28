"""Source wiring: capture bounded immutable snapshots and serve queries.

The adapter is the only place that touches real sources. It builds one frame
per capture from the collectors, persists it write-once in the store, and
answers queries strictly against that frame (a cursor can never escape the
snapshot it paginated). ``dream logs`` and ``dream open`` are the two
deliberate live reads: a log tail is streamed from the referenced run log
only when it still matches the coverage digested at capture time, and the
human jump prints exact facts, stating plainly when a capability (daemon
attach, GUI jump) is unavailable rather than implying it.
"""
import json
import os
import re
import sys
import time
from pathlib import Path

from . import collectors, model, store

HOME = Path.home()
QUEUE_RUNS = collectors.QUEUE_STATE / "runs"

# Standing local sources read while the optional introspection config
# subtree is NOT activated. When it activates, entries here are replaced by
# the enrolled sources (same kinds), and bounds come from the config.
STANDING_UNITS = ["ygg-swarm-executor@.service", "ygg-swarm-maintenance-orchestrator.service",
                  "ygg-heartbeat.service", "ygg-memory-sync.service", "ygg-panic.service",
                  "ygg-roll-watch.service"]
STANDING_TIMER_PATTERNS = ["ygg", "dream", "steer"]
DEFAULT_TTLS = {"queue": 30, "attempt_history": 30, "dream_state": 60,
                "dreamers": 60, "heartbeat": 60, "services": 60,
                "schedule": 300, "maintenance": 60, "kernel": 120,
                "pressure": 60, "commitments": 300, "composer": 120,
                "task_ledger": 300}


def enrollment():
    """Read the optional introspection subtree. It is supported by every
    reader but is NOT activated fleet-side; absence returns the standing
    defaults with activated=False (never a silent config adoption)."""
    spec = {"activated": False, "enabled": True, "sources": [],
            "limits": dict(collectors.DEFAULT_LIMITS),
            "retention": dict(collectors.DEFAULT_RETENTION), "faults": []}
    try:
        sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
        from swarmharness import swarmconfig as sc
        res = sc.load()
        cfg = (res or {}).get("config") or {}
        sub = cfg.get("introspection")
        if isinstance(sub, dict) and sub.get("enabled"):
            spec.update(activated=True, enabled=True,
                        limits={**spec["limits"], **(sub.get("limits") or {})},
                        retention={**spec["retention"], **(sub.get("retention") or {})},
                        faults=list((res or {}).get("faults") or [])[:4])
            for sid, s in (sub.get("sources") or {}).items():
                if isinstance(s, dict) and s.get("enabled"):
                    spec["sources"].append({"id": sid, **{k: s.get(k) for k in
                                          ("kind", "host", "path", "units", "ttl_seconds")}})
    except Exception as exc:
        spec["faults"].append({"code": "enrollment_unreadable", "detail": type(exc).__name__})
    return spec


def _coverage(source, kind, state, now, ttl, reason=None, host="local"):
    quality = {"ok": "verified", "absent": "unavailable", "stale": "stale",
               "invalid": "unknown", "unobserved": "unknown",
               "timeout": "unknown", "conflicting": "conflicting"}.get(state, "unknown")
    if quality in ("unknown", "unavailable") and not reason:
        reason = state
    if state == "absent":
        reason = reason or "source absent (not unobserved)"
    return model.coverage(source, host, kind, observed_at=model.iso(now),
                          received_at=model.iso(now), ttl=ttl,
                          quality=quality, reason=reason,
                          revision=None)


def _runmeta(audit_rows, now):
    """Attempt verification metadata from the dreamer audit ledger. Session
    IDs are unknown until the daemon metadata bridge lands (that lane owns
    it); unknown is recorded as unknown, never guessed."""
    meta = {}
    for i, row in enumerate(audit_rows):
        tid = str(row.get("id", ""))
        if not tid:
            continue
        prev = meta.get(tid)
        audited = model.epoch(row.get("audited_at"))
        if prev is None:
            fresh = True
        elif audited is None or prev.get("written_at") is None:
            fresh = False  # unorderable rows never displace ordering proof
        else:
            fresh = audited >= prev["written_at"]
        if not fresh:
            continue
        meta[tid] = {"quality": "verified" if row.get("terminal") else "observed",
                     "ref": "audit:" + str(i), "session_id": row.get("session_id"),
                     "harness": row.get("model_expected"),
                     "written_at": audited}
    return meta


def capture(now=None, root=None):
    """One bounded read-only pass over the standing sources -> frame."""
    now = now if now is not None else time.time()
    spec = enrollment()
    limits = spec["limits"]
    datasets, coverage = {}, []

    q, qstate, qdetail = collectors.collect_queue(now, limits)
    host = os.uname().nodename.split(".")[0]
    dstate, dst, ddetail = collectors.collect_dream_state(now)
    dl, dlst, dldetail = collectors.collect_dreamer_ledgers(now)
    runmeta = _runmeta(dl["audit"], now)
    work, attempts = model.queue_records(q["tasks"], q["events"], runmeta,
                                         "queue", host, now)
    datasets["dreams"] = work
    datasets["attempts"] = attempts

    repairs = []
    for row in dl["audit"]:
        if row.get("repair_ref"):
            repairs.append({"id": "repair:" + str(row["repair_ref"])[:60],
                            "work_id": "work:queue:" + str(row.get("id")),
                            "ref": model.text(row["repair_ref"], 120),
                            "verdict": model.text(row.get("verdict"), 24) or "unknown",
                            "at": model.epoch(row.get("audited_at")),
                            "verification": "editor-verified" if row.get("verdict") in ("PASS", "FAIL") else "unverified"})
    datasets["repairs"] = repairs

    hb, hbst, hbdetail = collectors.collect_heartbeat(now)
    hb_rows = []
    for plane in hb["planes"]:
        hb_rows.append({"id": "plane:" + plane["plane"], "row_type": "plane", **plane})
    for f in hb["findings"]:
        hb_rows.append({"id": "finding:" + str(f.get("id", ""))[:80], "row_type": "finding", **f})
    datasets["heartbeat"] = hb_rows

    services, svst, svdetail = collectors.collect_services(STANDING_UNITS, now, limits)
    maint, mst, mdetail = collectors.collect_maintenance(now)
    ex = maint["pass"].get("executor") or {}
    if ex.get("unit") and not any(s.get("unit") == ex["unit"] for s in services):
        # The supervisor's own read is a real service observation even when
        # this host's systemctl view differs; it carries its source with it.
        services.append({"id": "service:supervisor:" + str(ex["unit"]),
                         "unit": str(ex["unit"]), "host": host,
                         "active": "active" if ex.get("active") else "inactive",
                         "sub": "supervised", "load_state": "unknown",
                         "pid": None, "since": model.epoch(maint["pass"].get("at")),
                         "restarts": None,
                         "description": "supervisor-observed executor (%s)" % ex.get("action", "n/a"),
                         "cpu_seconds": None, "rss_bytes": None})
    datasets["services"] = services

    sched, scst, scdetail = collectors.collect_timers(STANDING_TIMER_PATTERNS, now, limits)
    for ov in dstate["overlays"]:
        if not isinstance(ov, dict):
            continue
        sched.append({"id": "schedule:overlay:" + str(ov.get("patch_id", ""))[:40],
                      "kind": "overlay", "type": model.text(ov.get("type"), 30),
                      "route": model.text(ov.get("route"), 40),
                      "namespace": model.text(ov.get("namespace"), 80),
                      "expires_at": model.epoch(ov.get("expires_at")),
                      "finding_ids": [model.text(f, 60) for f in (ov.get("finding_ids") or [])[:4]]})
    for cl in dstate["claims"]:
        deadline = cl.get("observation_deadline_min")
        opened = model.epoch(cl.get("opened_at"))
        sched.append({"id": "schedule:claim:" + str(cl.get("claim_id", ""))[:50],
                      "kind": "claim", "patch_id": model.text(cl.get("patch_id"), 40),
                      "disposition": model.text(cl.get("disposition"), 20) or "open",
                      "deadline_min": deadline, "opened_at": opened})
    st_counts = dstate["state"].get("counts") or {}
    staged = sum(v for v in st_counts.values() if isinstance(v, (int, float)))
    targets = {}
    try:
        floors, _, _ = collectors._state(collectors.DREAM_CFG)
        targets = (floors or {}).get("targets") or {}
    except Exception:
        targets = {}
    day_target = targets.get("day")
    sched.append({"id": "schedule:plan:dream-day", "kind": "plan",
                  "target": day_target, "staged": staged,
                  "remaining": (day_target - staged) if isinstance(day_target, (int, float)) else None,
                  "backlog_cap": targets.get("backlog_cap")})
    commits, cmtst, cmtdetail = collectors.collect_commitments(
        str(Path(root).expanduser().parent) if root else None, now)
    for c in commits:
        sched.append({"id": c["id"], "kind": "commitment", **{k: v for k, v in c.items() if k != "id"}})
    datasets["schedules"] = sched

    pressure, pst, pdetail = collectors.collect_pressure(now)
    datasets["plans"] = [{"id": "plan:" + r["kind"] + ":" + str(r.get("plane", ""))[:60], **r}
                         for r in pressure]

    events = []
    for r in dl["dispatch"]:
        events.append({"id": "event:dispatch:" + str(r.get("id", ""))[:24] + ":" + str(r.get("created_at", "")),
                       "subsystem": "dreamer", "severity": "info",
                       "at": model.epoch(r.get("created_at")),
                       "summary": model.text(r.get("title"), 120),
                       "route": model.text(r.get("route"), 40),
                       "queue_id": r.get("id")})
    for w in dstate["wake"]:
        for key, sev in (("revoked", "warning"), ("rerouted", "info"), ("held", "info")):
            for item in w.get(key) or []:
                if isinstance(item, dict):
                    why = str(item.get("why") or "")
                    # Failure WHY text is provider-adjacent; only its class
                    # and length enter the index, never the raw body.
                    events.append({"id": "event:%s:%s:%s" % (key, str(item.get("id", ""))[:24], w.get("t", 0)),
                                   "subsystem": "supervision", "severity": sev,
                                   "at": w.get("t"), "action": key,
                                   "summary": "failure-class=%s (%d chars)" % (
                                       collectors.error_class(why), len(why)) if why else key,
                                   "queue_id": item.get("id")})
    inc = maint["pass"].get("executor") or {}
    events.append({"id": "event:maintenance:%s" % str(maint["pass"].get("at", ""))[:40],
                   "subsystem": "maintenance", "severity": "info",
                   "at": model.epoch(maint["pass"].get("at")),
                   "summary": model.text(json.dumps({k: inc.get(k) for k in ("unit", "active", "action")}),
                                          160)})
    for i in maint["incidents"]:
        events.append({"id": "event:incident:%s:%s" % (str(i.get("unit", ""))[:40], str(i.get("at", ""))[:30]),
                       "subsystem": "maintenance", "severity": "warning",
                       "at": model.epoch(i.get("at")),
                       "summary": model.text(str(i.get("action", "")), 120)})
    kernel, kst, kdetail = collectors.collect_dmesg(now, limits)
    for r in kernel:
        events.append({**r, "at": r.get("at") or r.get("captured_at")})
    try:
        store.append_events(kernel, root=root, now=now)
    except Exception:
        pass
    retained = store.read_events(root=root, since=now - 31 * 86400, limit=collectors.DEFAULT_LIMITS["max_records"])
    seen = {e["id"] for e in events if e.get("id")}
    for r in retained:
        if r.get("id") not in seen:
            events.append(r)
            seen.add(r["id"])
    datasets["events"] = events

    findings = _checks(datasets, q, hb, services, dstate, now)
    datasets["findings"] = findings
    datasets["summary"] = [{"id": "summary:1", "dreams": len(work),
                            "running": len([w for w in work if w.get("state") == "running"]),
                            "failed": len([w for w in work if str(w.get("state", "")).startswith("failed")]),
                            "services": len(services), "schedules": len(sched),
                            "events": len(events), "heartbeat_planes": len(hb["planes"]),
                            "active_findings": len([f for f in hb["findings"] if f.get("active")])}]

    cov = [
        _coverage("queue:" + host, "queue", qstate, now, DEFAULT_TTLS["queue"],
                  reason=None if qstate == "ok" else str(qdetail)[:120], host=host),
        _coverage("queue-metrics:" + host, "attempt_history", qdetail.get("metrics_state", "absent") if isinstance(qdetail, dict) else qstate,
                  now, DEFAULT_TTLS["attempt_history"],
                  reason=str(qdetail)[:120] if qstate != "ok" else None, host=host),
        _coverage("dream-state:" + host, "schedule", dst, now, DEFAULT_TTLS["dream_state"],
                  reason=str(ddetail)[:120] if dst != "ok" else None, host=host),
        _coverage("dreamer-ledgers:" + host, "attempt_history", dlst, now, DEFAULT_TTLS["dreamers"],
                  reason=str(dldetail)[:120] if dlst != "ok" else None, host=host),
        _coverage("heartbeat:" + host, "heartbeat", hbst, now, DEFAULT_TTLS["heartbeat"],
                  reason=str(hbdetail)[:120] if hbst != "ok" else None, host=host),
        _coverage("services:" + host, "services", svst, now, DEFAULT_TTLS["services"],
                  reason=str(svdetail)[:120] if svst != "ok" else None, host=host),
        _coverage("timers:" + host, "schedule", scst, now, DEFAULT_TTLS["schedule"],
                  reason=str(scdetail)[:120] if scst != "ok" else None, host=host),
        _coverage("maintenance:" + host, "services", mst, now, DEFAULT_TTLS["maintenance"],
                  reason=str(mdetail)[:120] if mst != "ok" else None, host=host),
        _coverage("kernel-ring:" + host, "events", kst, now, DEFAULT_TTLS["kernel"],
                  reason=str(kdetail)[:120] if kst != "ok" else None, host=host),
        _coverage("commitments:" + host, "task_ledger", cmtst, now, DEFAULT_TTLS["commitments"],
                  reason=str(cmtdetail)[:120] if cmtst != "ok" else None, host=host),
    ]
    frame = model.frame(model.digest({"sources": sorted(c["source"] + ":" + c["kind"] for c in cov),
                                      "limits": limits, "retention": spec["retention"],
                                      "activated": spec["activated"]}),
                        model.iso(now), cov, datasets)
    try:
        store.save_snapshot(frame, root=root)
        store.prune(root=root, now=now, retention=spec["retention"])
    except Exception:
        pass
    return frame


def _checks(datasets, q, hb, services, dstate, now):
    """swarm check assertions: scope, verdict (held/falsified/unknown) and a
    bounded evidence pointer each. Unknown is an honest verdict here."""
    out = []
    serve = (q.get("serve") or {}).get("serve") or {}
    alive = serve.get("alive")
    dispatch_events = [e for e in datasets["events"] if e.get("subsystem") == "dreamer"]
    recent = [e for e in dispatch_events if (e.get("at") or 0) > now - 86400]
    if alive is True and recent:
        out.append({"id": "check:dispatch", "scope": "dispatch", "verdict": "held",
                    "detail": "queue owner live; %d dispatches observed in 24h" % len(recent)})
    elif alive is False:
        out.append({"id": "check:dispatch", "scope": "dispatch", "verdict": "falsified",
                    "detail": "queue owner not live"})
    else:
        out.append({"id": "check:dispatch", "scope": "dispatch", "verdict": "unknown",
                    "detail": "queue liveness unobserved"})
    if services:
        bad = [s["unit"] for s in services if s.get("active") not in ("active", "activating")]
        out.append({"id": "check:services", "scope": "services",
                    "verdict": "falsified" if bad else "held",
                    "detail": ("inactive: " + ", ".join(bad[:5])) if bad
                              else "%d standing units active" % len(services)})
    else:
        out.append({"id": "check:services", "scope": "services", "verdict": "unknown",
                    "detail": "no units observed"})
    funding_findings = [f for f in hb["findings"] if f.get("active") and f.get("kind") in
                        ("QUOTA_UNKNOWN", "AUTH_BLOCKED", "QUOTA_SCOPE_CONFLICT")]
    if funding_findings:
        out.append({"id": "check:funding", "scope": "funding", "verdict": "falsified",
                    "detail": "%d active funding findings (unknown readers): %s" % (
                        len(funding_findings),
                        ", ".join(str(f.get("id", ""))[:40] for f in funding_findings[:4]))})
    elif hb["planes"]:
        out.append({"id": "check:funding", "scope": "funding", "verdict": "held",
                    "detail": "%d funding planes verified in native units" % len(hb["planes"])})
    else:
        out.append({"id": "check:funding", "scope": "funding", "verdict": "unknown",
                    "detail": "heartbeat state absent"})
    broken = [f for f in hb["findings"] if f.get("active") and f.get("kind") == "RUNNER_BROKEN"
              and "ledger" in str(f.get("source", ""))]
    out.append({"id": "check:memory", "scope": "memory",
                "verdict": "falsified" if broken else "unknown",
                "detail": ("%d broken runner findings on ledger sources" % len(broken))
                          if broken else "no enrolled memory-plane assertions; ledger findings only"})
    return out


def _emit(result, as_json):
    if as_json:
        print(json.dumps(result, indent=1))
        return
    print("snapshot %s  as_of %s  code-coverage below" % (result["snapshot_id"][:16], result["as_of"]))
    for c in result["coverage"]:
        line = "  %-34s %-10s ttl=%ss" % (c["source"], c["quality"], c.get("ttl_seconds"))
        if c.get("reason"):
            line += "  " + str(c["reason"])[:100]
        print(line)
    for f in result["findings"]:
        print("  ! finding %s %s" % (f.get("code"), str(f.get("id", f.get("command", "")))[:60]))
    print("total %d%s" % (result["total"], "" if result.get("next_cursor") is None
                          else " (next cursor available)"))


def _row_briefs(items, family):
    if family == "dream":
        for r in items:
            att = r.get("reported_attempt_count")
            print("  %-28.28s %-13.13s %-16.16s att=%-4s %s" % (
                r.get("queue_id") or r.get("id"), r.get("state"), r.get("route"),
                att if att is not None else "?", r.get("title", "")[:64]))
            for a in r.get("attempts") or []:
                print("    attempt %-38.38s %-11.11s ord=%-3s started=%s finished=%s evidence=%s" % (
                    str(a.get("id")).replace("attempt:queue:", ""),
                    a.get("state"), a.get("ordinal"), a.get("started_at"),
                    a.get("finished_at"), a.get("evidence_state")))
                if a.get("reason"):
                    print("      reason: %s" % a["reason"][:120])
            for rp in r.get("repairs") or []:
                print("    repair   %-40.40s verdict=%-12.12s %s" % (
                    rp.get("ref"), rp.get("verdict"), rp.get("verification")))
    elif family == "service":
        for r in items:
            res = ""
            if r.get("rss_bytes"):
                res = " rss=%dMiB" % (r["rss_bytes"] // (1024 * 1024))
            print("  %-46.46s %-10.10s %-9.9s pid=%-7s%s" % (
                r.get("unit"), r.get("active"), r.get("sub"), r.get("pid"), res))
    elif family == "schedule":
        for r in items:
            kind = r.get("kind", "timer")
            when = r.get("next_text") or r.get("expires_at") or ""
            extra = ""
            if kind == "plan":
                extra = " staged=%s target=%s remaining=%s" % (
                    r.get("staged"), r.get("target"), r.get("remaining"))
            elif kind == "commitment":
                extra = " rev=%s authority=%s" % (r.get("revision"), r.get("authority"))
            print("  %-46.46s %-11.11s %s%s" % (r.get("id"), kind, when, extra))
    elif family == "heartbeat":
        for r in items:
            if r.get("row_type") == "finding":
                print("  %-34.34s %-16.16s active=%-5s count=%-5s %s" % (
                    r.get("id"), r.get("kind"), r.get("active"), r.get("count"),
                    str(r.get("detail", ""))[:70]))
            else:
                print("  %-20.20s %-10.10s grants=%-3s %s" % (
                    r.get("plane"), r.get("quality"), len(r.get("grants") or []),
                    str(r.get("error") or "")[:60]))
    elif family == "plans":
        for r in items:
            if r.get("kind") == "funding":
                grants = ", ".join(
                    "grant_units=%s%s" % (g.get("grant_units"),
                                          "" if g.get("remaining") is None
                                          else " remaining=%s" % g.get("remaining"))
                    for g in (r.get("grants") or [])[:3])
                if r.get("windows"):
                    grants += " windows=" + json.dumps(r["windows"][:2], sort_keys=True)
                print("  %-16.16s %-10.10s %s%s" % (
                    r.get("plane"), r.get("quality"), grants,
                    " [unknown reader: %s]" % r.get("error") if r.get("unknown_reader") else ""))
            else:
                print("  queue %-10.10s %s" % (r.get("quality"), json.dumps(r.get("counts") or {}, sort_keys=True)))
    elif family == "events":
        for r in items:
            print("  %-12.12s %-8.8s %-10.10s %s" % (
                r.get("subsystem"), r.get("severity"), r.get("at"),
                str(r.get("summary", ""))[:80]))
    elif family == "findings":
        for r in items:
            print("  %-18.18s %-10.10s %s" % (r.get("scope"), r.get("verdict"),
                                              str(r.get("detail", ""))[:90]))
    elif family == "summary":
        for r in items:
            print("  " + json.dumps({k: v for k, v in r.items() if k != "id"}, sort_keys=True))


def _render(result, path):
    family = path[0]
    verb = path[1] if len(path) > 1 else ""
    _emit(result, False)
    if family == "swarm" and verb == "check":
        for r in result["items"]:
            print("  check %-10.10s %-10.10s %s" % (r.get("scope"), r.get("verdict"),
                                                    str(r.get("detail", ""))[:100]))
        return
    if family == "swarm" and verb == "pressure" and result["items"]:
        _row_briefs(result["items"], "plans")
        print("  (native funding units above; remaining balances unavailable at the CLI stay unknown)")
        return
    if family == "swarm" and verb == "status":
        _row_briefs(result["items"], "summary")
        return
    if family == "swarm" and verb == "heartbeat":
        _row_briefs(result["items"], "heartbeat")
        return
    if family == "swarm" and verb == "events":
        _row_briefs(result["items"], "events")
        return
    if result["items"]:
        _row_briefs(result["items"], family)
    elif result["total"] == 0:
        print("  no rows in this query scope (see coverage above: an empty verified "
              "scope is not proof the fleet is idle)")


def _attempt_for(frame, work_id, attempt_id=None):
    for w in frame["datasets"].get("dreams", []):
        if w.get("id") == work_id:
            aid = attempt_id or w.get("current_attempt_id")
            for a in frame["datasets"].get("attempts", []):
                if a.get("id") == aid:
                    return w, a
            return w, None
    return None, None


def dream_logs(work_id, attempt_id=None, root=None):
    """Live bounded tail of ONE attempt's run log, gated by capture-time
    coverage. The log body is shown to the human, redacted, never stored."""
    now = time.time()
    frame = store.latest_snapshot(root=root, since=now - 30)
    if frame is None:
        frame = capture(now=now, root=root)
    w, a = _attempt_for(frame, work_id, attempt_id)
    if w is None:
        return {"error": "work %s not in current snapshot; run ygg dream ls" % work_id}, 3
    if a is None:
        return {"error": "no retained attempt for %s" % work_id}, 3
    ref = a.get("run_ref") or ""
    target = None
    verified = bool(ref)
    if ref:
        for p in sorted(QUEUE_RUNS.glob("*.log")):
            if collectors._fingerprint({"run_log": str(p)}) == ref:
                target = p
                break
    if target is None:
        # Fall back to the queue's own run-artifact naming
        # (ztq-<id>-a<ordinal>.json.log|.out|.json.log.hb): prefer the
        # attempt's ordinal, then transcript over stdout, else newest.
        qid = str(w.get("queue_id", ""))
        cands = [p for p in QUEUE_RUNS.iterdir()
                 if ((p.name.startswith(qid + "-") and p.suffix in (".log", ".out"))
                     or p.name.endswith(".json.log.hb"))]
        cands = sorted(cands, key=lambda p: p.stat().st_mtime, reverse=True)
        ordinal = a.get("ordinal")
        if isinstance(ordinal, int):
            stem = qid + "-a%d." % ordinal
            exact = [p for p in cands if stem in p.name]
            cands = exact + [p for p in cands if p not in exact]
        pref = ([p for p in cands if p.name.endswith(".json.log")]
                + [p for p in cands if p.name.endswith(".out")]
                + [p for p in cands if p.name.endswith(".json.log.hb")])
        target = pref[0] if pref else None
        verified = False
    if target is None:
        return {"error": "run log for %s unavailable (attempt evidence state: %s)"
                         % (a["id"], a.get("evidence_state"))}, 3
    try:
        size = target.stat().st_size
        with open(target, "rb") as f:
            f.seek(max(0, size - collectors.DEFAULT_LIMITS["log_tail_bytes"]))
            raw = f.read().decode("utf-8", "replace")
    except OSError as exc:
        return {"error": "run log unreadable: " + type(exc).__name__}, 3
    lines = [model.text(l, 200) for l in raw.splitlines()
             if l.strip()][-collectors.DEFAULT_LIMITS["log_tail_lines"]:]
    return {"attempt": a["id"], "log": target.name, "bytes": size,
            "tail_lines": lines,
            "coverage": "capture-verified" if verified else
                        "live-name-match (not digest-verified at capture time)",
            "note": "live bounded read at query time; redacted metadata; never stored"}, 0


JUMP_UNAVAILABLE = ("GUI jump is not available from this adapter: the daemon "
                    "worker hierarchy/attach contract is a separate lane; this "
                    "command selects and prints exact facts only, never resumes work.")


def dream_open(work_id, args, root=None):
    """Selectable human jump: resolve the work item to its exact current
    attempt and any native session identity, print the exact facts."""
    now = time.time()
    frame = store.latest_snapshot(root=root, since=now - 30)
    if frame is None:
        frame = capture(now=now, root=root)
    w, a = _attempt_for(frame, work_id)
    if w is None:
        return {"error": "work %s not in current snapshot; run ygg dream ls" % work_id}, 3
    session = (a or {}).get("session_id")
    print("work      %s" % w["id"])
    print("queue_id  %s   state %s   route %s" % (w.get("queue_id"), w.get("state"), w.get("route")))
    if a:
        print("attempt   %s  state %s  evidence %s" % (a["id"], a.get("state"), a.get("evidence_state")))
    if session:
        print("session   %s" % session)
        print("human jump (exact session): yggterm headless attach --session %s   # or: ssh <host> '<cli> resume %s'" % (session, session))
        print(JUMP_UNAVAILABLE)
        return {"work_id": w["id"], "session_id": session}, 0
    print("session   unknown (no native session metadata is enrolled for inspection yet; "
          "the run-metadata bridge is daemon-lane scope)")
    print(JUMP_UNAVAILABLE)
    return {"work_id": w["id"], "session_id": None,
            "error": "native session identity unknown for %s; selection shown above, "
                     "jump unavailable" % w["id"]}, 3


def serve(path, args, root=None):
    """Entry from the host porcelain: parse is already done; this resolves
    effects. Returns the exit code."""
    now = time.time()
    as_json = bool(args.pop("json", False))
    requested = args.get("snapshot")
    try:
        if requested:
            frame = store.load_snapshot(requested, root=root)
            if frame is None:
                print("ygg: retained snapshot %s not found" % requested, file=sys.stderr)
                return 3
        else:
            frame = store.latest_snapshot(root=root, since=now - 120)
            if frame is not None:
                # Reuse only while every source is still inside its own TTL:
                # a stale cached snapshot must not impersonate fresh data.
                as_of = model.epoch(frame.get("as_of")) or 0
                ttls = [c.get("ttl_seconds") for c in frame.get("coverage", [])
                        if isinstance(c.get("ttl_seconds"), (int, float)) and c["ttl_seconds"] > 0]
                if not ttls or now - as_of >= min(ttls):
                    frame = None
            if frame is None:
                frame = capture(now=now, root=root)
    except ValueError as exc:
        print("ygg: " + str(exc), file=sys.stderr)
        return 3
    if path == ("dream", "logs"):
        result, code = dream_logs(args.get("id"), args.get("attempt"), root=root)
        if code:
            print("ygg dream logs: " + result["error"], file=sys.stderr)
            return code
        print("attempt %s  log %s (%d bytes)  coverage: %s" % (
            result["attempt"], result["log"], result["bytes"], result["coverage"]))
        for line in result["tail_lines"]:
            print("  | " + line)
        print(result["note"])
        return 0
    if path == ("dream", "open"):
        result, code = dream_open(args.get("id"), args, root=root)
        if code:
            print("ygg dream open: " + result["error"], file=sys.stderr)
        return code
    try:
        result, code = model.query(frame, path, args, now)
    except ValueError as exc:
        print("ygg: " + str(exc), file=sys.stderr)
        return 3
    if as_json:
        print(json.dumps(result, indent=1))
    else:
        _render(result, path)
    return code
