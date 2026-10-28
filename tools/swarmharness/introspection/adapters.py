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
    """Read the optional introspection subtree and classify the collection
    mode honestly:

      configured — an ACTIVE config with an enabled subtree: the enrolled
                   sources (kind/host/path/units/ttl) drive capture and only
                   those sources are collected.
      disabled   — an active config whose subtree is explicitly disabled: a
                   standing read is NOT silently substituted; capture fails
                   closed with the disable recorded.
      invalid    — introspection-related config faults or an unreadable
                   config layer: enrollment intent existed but cannot be
                   trusted, so collection is refused, never widened.
      standing   — no active introspection subtree: the documented standing
                   read-only operator defaults apply, and every coverage row
                   carries collection_mode=standing (never configured-complete).
    """
    spec = {"mode": "standing", "activated": False, "enabled": False,
            "sources": [], "limits": dict(collectors.DEFAULT_LIMITS),
            "retention": dict(collectors.DEFAULT_RETENTION), "faults": [],
            "config_state": "absent"}
    try:
        sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
        from swarmharness import swarmconfig as sc
        res = sc.load()
    except Exception as exc:
        spec["mode"] = "invalid"
        spec["faults"].append({"code": "enrollment_unreadable",
                               "detail": type(exc).__name__})
        return spec
    res = res or {}
    cfg = res.get("config") or {}
    faults = [f for f in (res.get("faults") or []) if isinstance(f, dict)]
    spec["faults"] = faults[:4]
    insp_faults = [f for f in faults
                   if str(f.get("code", "")) == "inspection_schema"
                   or "introspection" in str(f.get("detail", "")).lower()]
    if insp_faults:
        spec.update(mode="invalid", config_state="inspection_faults")
        return spec
    if res.get("mode") != "active" or not isinstance(cfg, dict) or not cfg:
        spec.update(config_state=str(res.get("mode", "absent")))
        return spec
    sub = cfg.get("introspection")
    if not isinstance(sub, dict):
        spec.update(config_state="active")
        return spec
    spec.update(config_state="active", config_revision=res.get("revision"))
    if sub.get("enabled") is not True:
        spec.update(mode="disabled")
        return spec
    spec.update(mode="configured", activated=True, enabled=True)
    for sid, s in (sub.get("sources") or {}).items():
        if not isinstance(s, dict):
            continue
        spec["sources"].append({"id": sid, "enabled": s.get("enabled") is True,
                                **{k: s.get(k) for k in ("kind", "host", "path",
                                                         "units", "ttl_seconds",
                                                         "queue_source_ref")}})
    spec["limits"] = {**spec["limits"], **(sub.get("limits") or {})}
    spec["retention"] = {**spec["retention"], **(sub.get("retention") or {})}
    return spec


STANDING_NOTE = ("standing read-only operator default (introspection config "
                 "not activated; not configured-complete)")


def _file_mtime(path):
    """The source file's own write time — its observation clock. The reader's
    clock never substitutes for it."""
    try:
        return Path(path).stat().st_mtime
    except OSError:
        return None


def _max_epoch(rows, keys):
    """Newest recorded time across bounded rows, or None. A row without a
    usable time contributes nothing rather than now."""
    return max((t for r in rows if isinstance(r, dict)
                for t in (model.epoch(r.get(k)) for k in keys) if t),
               default=None)


def _coverage(source, kind, state, now, ttl, source_observed_at=None,
              reason=None, host="local", collection_mode=None):
    """One coverage record. ``observed_at`` is the SOURCE's own observation
    time, never the reader's clock: a read that just succeeded does not
    freshen an old observation. Only a source as-of inside the TTL with a
    known time can stay verified; everything else is demoted with the exact
    reason on the record."""
    quality = {"ok": "verified", "absent": "unavailable", "stale": "stale",
               "invalid": "unknown", "unobserved": "unknown",
               "refused": "unavailable", "timeout": "unknown",
               "conflicting": "conflicting"}.get(state, "unknown")
    if quality in ("unknown", "unavailable") and not reason:
        reason = state
    if state == "absent":
        reason = reason or "source absent (not unobserved)"
    if quality == "verified":
        if source_observed_at is None:
            quality, reason = "unknown", reason or "source observation time unknown"
        else:
            age = now - source_observed_at
            if age > ttl:
                quality = "stale"
                reason = ("source observation age %ds exceeds ttl %ds"
                          % (int(age), int(ttl)))
            elif age < -model.CLOCK_SKEW_SECONDS:
                quality = "conflicting"
                reason = "source clock ahead of reader by %ds" % int(-age)
    if collection_mode == "standing" and reason:
        reason = str(reason) + "; " + STANDING_NOTE
    elif collection_mode == "standing":
        reason = STANDING_NOTE
    return model.coverage(source, host, kind,
                          observed_at=(model.iso(source_observed_at)
                                       if source_observed_at is not None else None),
                          received_at=model.iso(now), ttl=ttl,
                          quality=quality, reason=reason, revision=None,
                          collection_mode=collection_mode)


def _reviews_dir():
    return Path(os.environ.get("YGG_INSPECT_REVIEWS",
                               str(HOME / ".yggterm/swarm/reviews"))).expanduser()


def _chain_sessions(rows):
    """Session identities a review chain records. The existing chain format
    may carry only worker.session; anything else stays absent."""
    out = set()
    for r in rows:
        if not isinstance(r, dict):
            continue
        for k in ("session", "session_id", "worker_session"):
            v = r.get(k)
            if isinstance(v, str) and v:
                out.add(v)
        w = r.get("worker")
        if isinstance(w, dict) and isinstance(w.get("session"), str) and w["session"]:
            out.add(w["session"])
    return out


def _binding_failure(rows, claim):
    """Trusted-review association: the chain must be BOUND to this exact
    work — the audit row's native session identity must appear in the chain,
    and the claimed subject digest must equal the subject the chain's editor
    verdict actually authenticated (its snapshot_digest). Staging a snapshot
    is not approving it; a valid COMPLETE receipt of ANOTHER work never
    verifies this repair just because an audit row carries its review_id;
    missing proof of any binding stays unverified with the exact reason."""
    claimed_session = claim.get("session_id")
    if not claimed_session:
        return ("audit row records no native session identity; review "
                "association cannot be established")
    sessions = _chain_sessions(rows)
    if not sessions:
        return ("review chain records no worker session identity; "
                "association cannot be established")
    if claimed_session not in sessions:
        return ("review chain is bound to session(s) %s, not the audit row's "
                "session %s" % (", ".join(sorted(sessions))[:80],
                                str(claimed_session)[:40]))
    # The approved subject is the one the editor verdict actually authenticated
    # (its snapshot_digest) — the chain's LAST verdict, the one a COMPLETE
    # resolution closes over. Any staged snapshot is NOT approval: a chain
    # that staged subject A then B and received a verdict for B must never
    # verify a claim for the unapproved A just because A appears in history.
    verdicts = [r for r in rows if r.get("kind") == "editor_verdict"]
    approved = {v.get("snapshot_digest") for v in verdicts
                if isinstance(v.get("snapshot_digest"), str)}
    claimed_subject = claim.get("subject_digest")
    if not claimed_subject:
        return ("audit row carries no subject digest; the exact "
                "accepted-subject binding is missing")
    if claimed_subject not in approved:
        return ("approved subject digest does not match the claimed "
                "repair's subject")
    return None


def _receipt_state(review_id, review_dir=None, key_path=None, claim=None):
    """Validate one sessionReview chain for repair verification. Returns
    (verification, reason). ``editor-verified`` requires ALL of: an intact
    hash chain (any corruption raises on read), a terminal COMPLETE
    resolution, a recorded editor verdict, an editor invocation receipt
    that authenticates against the control key on this reader, and — when a
    claim row is given — a trusted work association: session identity and
    exact approved subject digest matching the claimed repair. A
    structurally valid chain without a verifiable receipt stays unverified —
    a verdict claim, even PASS, is never editor-verified by itself."""
    try:
        sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
        from swarmharness import session_review as sr
        review = sr.SessionReview(str(review_id), base=str(review_dir or _reviews_dir()))
        rows = review.rows  # corrupt/rewritten/reordered chains raise here
    except Exception as exc:
        return "unverified", ("review chain unreadable or corrupt: "
                              + type(exc).__name__)
    if not rows:
        return "unverified", "review chain empty"
    if not any(r.get("kind") == "resolution" and r.get("outcome") == "COMPLETE"
               for r in rows):
        return "unverified", "review chain has no COMPLETE resolution"
    verdicts = [r for r in rows if r.get("kind") == "editor_verdict"]
    if not verdicts:
        return "unverified", "review chain carries no editor verdict"
    if claim is not None:
        # Association is checked before provenance: a receipt that cannot be
        # bound to THIS work is rejected for the binding reason even before
        # the control key is consulted.
        binding = _binding_failure(rows, claim)
        if binding:
            return "unverified", binding
    try:
        # the verifier factory takes the key PATH and loads it itself; passing
        # the loaded key bytes here made every verification fail closed
        verifier = sr.hmac_provenance_verifier(key_path)
    except Exception:
        return "unverified", ("review chain structurally valid; editor "
                              "provenance not verifiable (no control key on "
                              "this reader)")
    verdict = verdicts[-1]
    snaps = [r for r in rows if r.get("kind") == "snapshot_staged"]
    matching = ([s for s in snaps if s.get("subject_digest") == verdict.get("snapshot_digest")]
                or snaps)
    if not matching:
        return "unverified", "review chain carries no staged snapshot for the verdict"
    try:
        ok = verifier(review.review_id, rows[0], matching[-1],
                      verdict.get("editor"), verdict.get("signed_payload"),
                      verdict.get("verification_head")) is True
    except Exception as exc:
        return "unverified", ("editor receipt verification failed: "
                              + type(exc).__name__)
    if not ok:
        return "unverified", "editor invocation receipt did not authenticate"
    return "editor-verified", ""


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


_EMPTY_DATASETS = {"dreams": [], "attempts": [], "services": [], "schedules": [],
                   "plans": [], "events": [], "heartbeat": [], "findings": [],
                   "repairs": [], "workers": [], "composer": [], "reviews": [],
                   "summary": []}


def _failclosed_frame(spec, now, root):
    """A capture that read NOTHING. An explicit disable or an invalid
    enrollment is recorded as an unavailable collection with the exact
    reason — never an empty-looking healthy frame and never a silent
    standing substitution."""
    mode = spec["mode"]
    host = os.uname().nodename.split(".")[0]
    reason = ("introspection collection disabled by enrollment (owner disable "
              "respected; no standing substitution)" if mode == "disabled" else
              "introspection enrollment invalid or unreadable; collection "
              "refused (fail closed)")
    faults = "; ".join("%s:%s" % (f.get("code", "?"), str(f.get("detail", ""))[:60])
                       for f in spec["faults"][:3])
    cov = [_coverage("collection:" + host, "collection", "refused", now, 60,
                     reason=reason + (("; faults: " + faults) if faults else ""),
                     collection_mode=mode)]
    datasets = {k: list(v) for k, v in _EMPTY_DATASETS.items()}
    datasets["findings"] = [{"id": "finding:collection:" + mode,
                             "scope": "collection", "verdict": "unknown",
                             "detail": reason + (("; faults: " + faults) if faults else "")}]
    datasets["summary"] = [{"id": "summary:1", "collection_mode": mode,
                            "sources_collected": 0, "dreams": None,
                            "workers": None,
                            "note": "fail-closed capture: no sources were read"}]
    frame = model.frame(model.digest({"mode": mode, "faults": spec["faults"],
                                      "sources": sorted(c["source"] + ":" + c["kind"]
                                                        for c in cov)}),
                        model.iso(now), cov, datasets)
    try:
        store.save_snapshot(frame, root=root)
    except Exception:
        pass
    return frame


def _repairs_from_audit(audit_rows, now, reviews_dir=None):
    """Repairs from the dreamer audit ledger. The audit verdict (PASS/FAIL)
    is a worker-graded claim from the same population it grades: it is
    retained verbatim but can never self-certify. ``editor-verified``
    requires a trusted sessionReview chain linked by the audit row's
    review_id and validated end to end; everything else records exactly why
    it stays unverified."""
    repairs = []
    for row in audit_rows:
        if not isinstance(row, dict) or not row.get("repair_ref"):
            continue
        rec = {"id": "repair:" + str(row["repair_ref"])[:60],
               "work_id": "work:queue:" + str(row.get("id")),
               "ref": model.text(row["repair_ref"], 120),
               "verdict": model.text(row.get("verdict"), 24) or "unknown",
               "at": model.epoch(row.get("audited_at")),
               "verification": "unverified", "verification_reason": None}
        review_id = row.get("review_id")
        if review_id:
            rec["review_id"] = model.text(review_id, 80)
            state, why = _receipt_state(str(review_id)[:80], reviews_dir,
                                        claim=row)
            rec["verification"] = state
            rec["verification_reason"] = why or None
        else:
            rec["verification_reason"] = ("worker audit claim (no sessionReview "
                                          "linkage); verdict PASS/FAIL never "
                                          "self-certifies")
        repairs.append(rec)
    return repairs


def capture(now=None, root=None):
    """One bounded read-only pass -> frame. The enrollment mode decides what
    is read: standing defaults, exactly the enrolled configured sources, or
    nothing (fail closed)."""
    now = now if now is not None else time.time()
    spec = enrollment()
    if spec["mode"] in ("disabled", "invalid"):
        return _failclosed_frame(spec, now, root)
    host = os.uname().nodename.split(".")[0]
    if spec["mode"] == "configured":
        return _capture_configured(spec, now, root, host)
    return _capture_standing(spec, now, root, host)


def _capture_standing(spec, now, root, host):
    limits = spec["limits"]
    mode = "standing"
    datasets, coverage = {}, []

    q, qstate, qdetail = collectors.collect_queue(now, limits)
    dstate, dst, ddetail = collectors.collect_dream_state(now)
    dl, dlst, dldetail = collectors.collect_dreamer_ledgers(now)
    runmeta = _runmeta(dl["audit"], now)
    work, attempts = model.queue_records(q["tasks"], q["events"], runmeta,
                                         "queue", host, now)
    for w in work:
        w["origin"] = "queue-admitted"
    datasets["dreams"] = work
    datasets["attempts"] = attempts

    datasets["repairs"] = _repairs_from_audit(dl["audit"], now)

    hb, hbst, hbdetail = collectors.collect_heartbeat(now)
    hb_rows = []
    for plane in hb["planes"]:
        hb_rows.append({"id": "plane:" + plane["plane"], "row_type": "plane",
                        "freshness": collectors.plane_fresh(plane, now, DEFAULT_TTLS["heartbeat"]),
                        **plane})
    for f in hb["findings"]:
        hb_rows.append({"id": "finding:" + str(f.get("id", ""))[:80], "row_type": "finding", **f})
    datasets["heartbeat"] = hb_rows

    services, svst, svdetail = collectors.collect_services(STANDING_UNITS, now, limits)
    for s in services:
        s["provenance"] = "systemd"
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
                         "cpu_seconds": None, "rss_bytes": None,
                         "provenance": "supervisor-read"})
    datasets["services"] = services
    sv_reason = None
    if isinstance(svdetail, dict) and svdetail.get("expected_not_found"):
        sv_reason = ("expected roster units not observed by systemctl: "
                     + ", ".join(svdetail["expected_not_found"][:6]))

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

    workers, wst, wdetail = collectors.collect_workers(now)
    datasets["workers"] = workers

    comp, compst, compdetail = collectors.collect_composer(now, limits)
    datasets["composer"] = comp
    revs, revst, revdetail = collectors.collect_reviews(now, limits)
    datasets["reviews"] = revs
    meml, memst, memdetail = collectors.collect_memory_lock(now, limits)
    # Failed work keeps its failure explicit: attempts carry the composer
    # linkage state verbatim (no inferred correlation), and every failed work
    # row without a linked repair is a named unreviewed-failure finding.
    failed = [w for w in work if str(w.get("state", "")).startswith("failed")
              or str(w.get("state")) == "interrupted"]
    for a in attempts:
        if any(a["work_id"] == w["id"] for w in failed):
            a["composer_link"] = ("none-correlated; composer observations are "
                                  "inventoried separately, correlation is not "
                                  "inferred from failure text alone")

    pressure, pst, pdetail = collectors.collect_pressure(now, DEFAULT_TTLS["heartbeat"])
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
    datasets["events"].extend({
        "id": "event:memlock:" + r["id"].replace("memlock:", ""),
        "subsystem": "memory-lock", "severity": "info",
        "at": r.get("source_observed_at") or now,
        "summary": "sealed module %s bytes=%s sha16=%s" % (
            r.get("module"), r.get("bytes"), r.get("sha256_16"))}
        for r in meml)

    findings = _checks(datasets, q, hb, services, dstate, now)
    for w in failed:
        linked = [r for r in datasets["repairs"] if r.get("work_id") == w["id"]]
        reviewed = [r for r in linked if r.get("verification") == "editor-verified"]
        findings.append({
            "id": "finding:failure:" + str(w.get("id"))[:70],
            "scope": "failures",
            "verdict": "falsified",
            "detail": ("work %s state=%s: repair rows=%d (editor-verified=%d); "
                       "review=%s; composer_link=%s"
                       % (str(w.get("queue_id") or w.get("id"))[:40],
                          w.get("state"), len(linked), len(reviewed),
                          w.get("review", "unreviewed"),
                          (attempts[0].get("composer_link", "unknown")
                           if attempts else "unknown")))[:220]})
    if memst not in ("ok",):
        findings.append({"id": "finding:memory-lock:" + memst, "scope": "memory",
                         "verdict": "unknown",
                         "detail": ("composer sealed-kernel lock %s: %s"
                                    % (memst, str(memdetail)[:140]))})
    datasets["findings"] = findings
    datasets["summary"] = [{"id": "summary:1", "collection_mode": mode,
                            "dreams": len(work),
                            "running": len([w for w in work if w.get("state") == "running"]),
                            "failed": len([w for w in work if str(w.get("state", "")).startswith("failed")]),
                            "workers": len(workers) if workers else 0,
                            "workers_state": wst,
                            "queue_is_not_whole_compute": True,
                            "services": len(services),
                            "services_dynamic_instances": len(
                                [s for s in services
                                 if re.search(r"@[^.]+\.service$", str(s.get("unit", "")))]),
                            "composer_observations": len(
                                [c for c in comp if c.get("row_type") == "observation"]),
                            "composer_state": compst,
                            "reviews": len(revs),
                            "reviews_state": revst,
                            "memory_lock_modules": len(meml),
                            "memory_lock_state": memst,
                            "schedules": len(sched),
                            "events": len(events), "heartbeat_planes": len(hb["planes"]),
                            "active_findings": len([f for f in hb["findings"] if f.get("active")])}]

    # Source observation clocks: every file-backed source carries its OWN
    # clock (embedded as-of, newest row time, or file mtime) — never the
    # reader's now. Only live OS queries (the queue CLI, systemctl, dmesg)
    # may use the read clock, because there the observation IS the read.
    metrics_clock = (_max_epoch(q["events"], ("t",))
                     or _file_mtime(collectors.QUEUE_STATE / "metrics.jsonl"))
    dream_clock = model.epoch(dstate["state"].get("as_of"))
    ledger_clock = (_max_epoch(dl["audit"], ("audited_at",))
                    or _max_epoch(dl["dispatch"], ("created_at", "at", "t"))
                    or _file_mtime(collectors.DREAMER_STATE / "audit-ledger.jsonl"))
    maint_clock = model.epoch(maint["pass"].get("at"))
    workers_clock = (wdetail.get("newest_mtime") if isinstance(wdetail, dict) else None)
    commit_clock = (cmtdetail.get("observed_at") if isinstance(cmtdetail, dict) else None)
    cov = [
        # queue: live queue-CLI query (subprocess read of the live
        # orchestrator); a failed read is not an observation, so no clock
        _coverage("queue:" + host, "queue", qstate, now, DEFAULT_TTLS["queue"],
                  source_observed_at=now if qstate == "ok" else None,
                  reason=None if qstate == "ok" else str(qdetail)[:120], host=host,
                  collection_mode=mode),
        # queue-metrics: metrics.jsonl is a file — its newest event / mtime
        _coverage("queue-metrics:" + host, "attempt_history", qdetail.get("metrics_state", "absent") if isinstance(qdetail, dict) else qstate,
                  now, DEFAULT_TTLS["attempt_history"], source_observed_at=metrics_clock,
                  reason=str(qdetail)[:120] if qstate != "ok" else None, host=host,
                  collection_mode=mode),
        # dream-state: the state file's own as_of
        _coverage("dream-state:" + host, "schedule", dst, now, DEFAULT_TTLS["dream_state"],
                  source_observed_at=dream_clock,
                  reason=str(ddetail)[:120] if dst != "ok" else None, host=host,
                  collection_mode=mode),
        # dreamer-ledgers: audit/dispatch row times, else the ledger mtime
        _coverage("dreamer-ledgers:" + host, "attempt_history", dlst, now, DEFAULT_TTLS["dreamers"],
                  source_observed_at=ledger_clock,
                  reason=str(dldetail)[:120] if dlst != "ok" else None, host=host,
                  collection_mode=mode),
        _coverage("heartbeat:" + host, "heartbeat", hbst, now, DEFAULT_TTLS["heartbeat"],
                  source_observed_at=hb.get("as_of"),
                  reason=str(hbdetail)[:120] if hbst != "ok" else None, host=host,
                  collection_mode=mode),
        # services/timers: live systemctl reads
        _coverage("services:" + host, "services", svst, now, DEFAULT_TTLS["services"],
                  source_observed_at=now if svst == "ok" else None,
                  reason=sv_reason if svst == "ok" else
                  (str(svdetail)[:120] if not isinstance(svdetail, dict) else json.dumps(svdetail, sort_keys=True)[:120]),
                  host=host, collection_mode=mode),
        _coverage("timers:" + host, "schedule", scst, now, DEFAULT_TTLS["schedule"],
                  source_observed_at=now if scst == "ok" else None,
                  reason=str(scdetail)[:120] if scst != "ok" else None, host=host,
                  collection_mode=mode),
        # maintenance: supervisor.json's own pass time
        _coverage("maintenance:" + host, "services", mst, now, DEFAULT_TTLS["maintenance"],
                  source_observed_at=maint_clock,
                  reason=str(mdetail)[:120] if mst != "ok" else None, host=host,
                  collection_mode=mode),
        # workers-registry: newest registry file mtime
        _coverage("workers-registry:" + host, "workers", wst, now, DEFAULT_TTLS["services"],
                  source_observed_at=workers_clock,
                  reason=str(wdetail)[:120] if wst != "ok" else None, host=host,
                  collection_mode=mode),
        # kernel-ring: live kernel ring read
        _coverage("kernel-ring:" + host, "events", kst, now, DEFAULT_TTLS["kernel"],
                  source_observed_at=now if kst == "ok" else None,
                  reason=str(kdetail)[:120] if kst != "ok" else None, host=host,
                  collection_mode=mode),
        # commitments: the journal rows' own recorded time
        _coverage("commitments:" + host, "task_ledger", cmtst, now, DEFAULT_TTLS["commitments"],
                  source_observed_at=commit_clock,
                  reason=str(cmtdetail)[:120] if cmtst != "ok" else None, host=host,
                  collection_mode=mode),
        _coverage("composer:" + host, "composer", compst, now,
                  DEFAULT_TTLS.get("composer", 120),
                  source_observed_at=max(((c.get("last_at") or 0)
                                          for c in comp
                                          if c.get("row_type") == "observation"),
                                         default=0) or None,
                  reason=(str(compst) + ": " + str(compdetail)[:110])
                         if compst != "ok" else None,
                  host=host, collection_mode=mode),
        _coverage("reviews:" + host, "reviews", revst, now,
                  DEFAULT_TTLS.get("reviews", 120),
                  source_observed_at=max(((r.get("resolution_at") or 0)
                                          for r in revs), default=0) or None,
                  reason=str(revdetail)[:120] if revst != "ok" else None,
                  host=host, collection_mode=mode),
        _coverage("memory-lock:" + host, "memory_lock", memst, now,
                  DEFAULT_TTLS.get("memory_lock", 120),
                  source_observed_at=max(((r.get("source_observed_at") or 0)
                                          for r in meml), default=0) or None,
                  reason=(str(memst) + ": " + str(memdetail)[:110])
                         if memst != "ok" else None,
                  host=host, collection_mode=mode),
    ]
    frame = model.frame(model.digest({"sources": sorted(c["source"] + ":" + c["kind"] for c in cov),
                                      "limits": limits, "retention": spec["retention"],
                                      "activated": spec["activated"], "mode": mode}),
                        model.iso(now), cov, datasets)
    try:
        store.save_snapshot(frame, root=root)
        store.prune(root=root, now=now, retention=spec["retention"])
    except Exception:
        pass
    return frame


_CONFIGURED_KINDS = ("queue", "attempt_history", "heartbeat", "services",
                     "schedule", "task_ledger", "composer", "reviews",
                     "memory_lock", "workers", "pressure")


def _capture_configured(spec, now, root, host):
    """Collect EXACTLY the enrolled sources. An enrolled host this adapter
    cannot serve is reported unobserved rather than silently widened to
    local, a disabled enrolled source is recorded as not collected, and a
    kind without an implemented collector is an explicit open — none of
    these fall back to standing sources."""
    limits = spec["limits"]
    mode = "configured"
    datasets = {k: list(v) for k, v in _EMPTY_DATASETS.items()}
    coverage = []

    def served(entry):
        # only local hosts are served by this adapter; anything else is a
        # fail-visible unobserved, never a silent local read
        return entry.get("host") in (host, "local", None)

    have_queue = False
    hb = {"planes": [], "findings": [], "as_of": None}
    services = []
    compst = revst = "unobserved"
    wst = pst = "unobserved"
    wdetail = pdetail = {}
    enrolled_host = lambda s: str(s.get("host") or host)
    for s in spec["sources"]:
        sid, kind = str(s.get("id", "")), s.get("kind")
        ttl = s.get("ttl_seconds") if isinstance(s.get("ttl_seconds"), int) else DEFAULT_TTLS.get(kind, 120)
        src = kind + ":" + sid + ":" + host
        if not s.get("enabled"):
            coverage.append(_coverage(src, kind or "unknown", "unobserved", now, ttl,
                                      reason="enrolled source disabled; not collected",
                                      host=enrolled_host(s), collection_mode=mode))
            continue
        if kind not in _CONFIGURED_KINDS:
            coverage.append(_coverage(src, kind or "unknown", "unobserved", now, ttl,
                                      reason="unsupported source kind",
                                      host=enrolled_host(s), collection_mode=mode))
            continue
        if not served(s):
            coverage.append(_coverage(src, kind, "unobserved", now, ttl,
                                      reason="enrolled host %r is not served by this "
                                             "local adapter (not silently widened)"
                                             % str(s.get("host"))[:40],
                                      host=enrolled_host(s), collection_mode=mode))
            continue
        if kind in ("queue", "attempt_history"):
            if have_queue:
                coverage.append(_coverage(src, kind, "ok", now, ttl,
                                          source_observed_at=now, host=enrolled_host(s), collection_mode=mode))
                continue
            have_queue = True
            q, qstate, qdetail = collectors.collect_queue(now, limits)
            runmeta = _runmeta([], now)
            work, attempts = model.queue_records(q["tasks"], q["events"], runmeta,
                                                 "queue", host, now)
            for w in work:
                w["origin"] = "queue-admitted"
            datasets["dreams"] = work
            datasets["attempts"] = attempts
            coverage.append(_coverage(src, kind, qstate, now, ttl,
                                      source_observed_at=now if qstate == "ok" else None,
                                      reason=None if qstate == "ok" else str(qdetail)[:120],
                                      host=enrolled_host(s), collection_mode=mode))
        elif kind == "heartbeat":
            hb, hbst, hbdetail = collectors.collect_heartbeat(now)
            for plane in hb["planes"]:
                datasets["heartbeat"].append({"id": "plane:" + plane["plane"],
                                              "row_type": "plane",
                                              "freshness": collectors.plane_fresh(plane, now, ttl),
                                              **plane})
            for f in hb["findings"]:
                datasets["heartbeat"].append({"id": "finding:" + str(f.get("id", ""))[:80],
                                              "row_type": "finding", **f})
            coverage.append(_coverage(src, kind, hbst, now, ttl,
                                      source_observed_at=hb.get("as_of"),
                                      reason=str(hbdetail)[:120] if hbst != "ok" else None,
                                      host=enrolled_host(s), collection_mode=mode))
        elif kind == "services":
            rows, sst, sdetail = collectors.collect_services(s.get("units"), now, limits)
            for r in rows:
                r["provenance"] = "systemd"
            services.extend(rows)
            reason = None
            if sst == "ok" and isinstance(sdetail, dict) and sdetail.get("expected_not_found"):
                reason = ("expected roster units not observed by systemctl: "
                          + ", ".join(sdetail["expected_not_found"][:6]))
            elif sst != "ok":
                reason = json.dumps(sdetail, sort_keys=True)[:160] if isinstance(sdetail, dict) else str(sdetail)[:120]
            datasets["services"].extend(rows)
            coverage.append(_coverage(src, kind, sst, now, ttl,
                                      source_observed_at=now if sst == "ok" else None,
                                      reason=reason, host=enrolled_host(s), collection_mode=mode))
        elif kind == "schedule":
            rows, schst, schdetail = collectors.collect_timers(s.get("units") or [], now, limits)
            datasets["schedules"].extend(rows)
            coverage.append(_coverage(src, kind, schst, now, ttl,
                                      source_observed_at=now if schst == "ok" else None,
                                      reason=str(schdetail)[:120] if schst != "ok" else None,
                                      host=enrolled_host(s), collection_mode=mode))
        elif kind == "task_ledger":
            row = collectors.parse_tasks_ledger(s.get("path"))
            if row:
                datasets["schedules"].append({"id": "schedule:ledger:" + sid, "kind": "commitment",
                                              **row})
                coverage.append(_coverage(src, kind, "ok", now, ttl,
                                          source_observed_at=row.get("observed_at"),
                                          host=enrolled_host(s), collection_mode=mode))
            else:
                coverage.append(_coverage(src, kind, "absent", now, ttl, host=enrolled_host(s), collection_mode=mode))
        elif kind == "composer":
            rows_c, cst, cdet = collectors.collect_composer(now, limits, s.get("path"))
            datasets["composer"].extend(rows_c)
            coverage.append(_coverage(src, kind, cst, now, ttl,
                                      source_observed_at=max(
                                          ((r.get("last_at") or 0) for r in rows_c
                                           if r.get("row_type") == "observation"),
                                          default=0) or None,
                                      reason=str(cdet)[:140] if cst != "ok" else None,
                                      host=enrolled_host(s), collection_mode=mode))
        elif kind == "reviews":
            rows_r, rst, rdt = collectors.collect_reviews(now, limits, s.get("path"))
            datasets["reviews"].extend(rows_r)
            coverage.append(_coverage(src, kind, rst, now, ttl,
                                      source_observed_at=max(
                                          ((r.get("resolution_at") or 0) for r in rows_r),
                                          default=0) or None,
                                      reason=(str(rst) + ": " + str(rdt)[:120])
                                             if rst != "ok" else None,
                                      host=enrolled_host(s), collection_mode=mode))
        elif kind == "memory_lock":
            rows_m, mst, mdt = collectors.collect_memory_lock(now, limits, s.get("path"))
            datasets["events"].extend({
                "id": "event:memlock:" + r["id"].replace("memlock:", ""),
                "subsystem": "memory-lock", "severity": "info",
                "at": r.get("source_observed_at") or now,
                "summary": "sealed module %s bytes=%s sha16=%s" % (
                    r.get("module"), r.get("bytes"), r.get("sha256_16"))}
                for r in rows_m)
            coverage.append(_coverage(src, kind, mst, now, ttl,
                                      source_observed_at=max(
                                          ((r.get("source_observed_at") or 0) for r in rows_m),
                                          default=0) or None,
                                      reason=str(mdt)[:140] if mst != "ok" else None,
                                      host=enrolled_host(s), collection_mode=mode))
        elif kind == "workers":
            rows_w, wst, wdetail = collectors.collect_workers(now, s.get("path"))
            datasets["workers"].extend(rows_w)
            coverage.append(_coverage(src, kind, wst, now, ttl,
                                      source_observed_at=(wdetail.get("newest_mtime")
                                                          if isinstance(wdetail, dict) else None),
                                      reason=str(wdetail)[:140] if wst != "ok" else None,
                                      host=enrolled_host(s), collection_mode=mode))
        elif kind == "pressure":
            rows_p, pst, pdetail = collectors.collect_pressure(now, ttl)
            datasets["plans"].extend(
                {"id": "plan:" + r["kind"] + ":" + str(r.get("plane", ""))[:60], **r}
                for r in rows_p)
            coverage.append(_coverage(src, kind, pst, now, ttl,
                                      source_observed_at=(pdetail.get("heartbeat_as_of")
                                                          if isinstance(pdetail, dict) else None),
                                      reason=str(pdetail)[:140] if pst != "ok" else None,
                                      host=enrolled_host(s), collection_mode=mode))
        else:
            coverage.append(_coverage(src, kind, "unobserved", now, ttl,
                                      reason="collector not implemented for kind %s "
                                             "(recorded open, not healthy-empty)" % kind,
                                      host=enrolled_host(s), collection_mode=mode))

    # Workers and pressure exist here ONLY when enrolled: a configured capture
    # with zero enrolled sources (or a disabled heartbeat) reads neither —
    # hidden unenrolled reads would silently widen the enrollment.
    workers = datasets["workers"]
    datasets["findings"] = _checks(datasets,
                                   {"serve": {}}, hb, services, {"state": {}}, now)
    datasets["summary"] = [{"id": "summary:1", "collection_mode": mode,
                            "dreams": len(datasets["dreams"]),
                            "workers": len(workers) if workers else 0,
                            "workers_state": wst,
                            "services": len(datasets["services"]),
                            "schedules": len(datasets["schedules"]),
                            "events": len(datasets["events"]),
                            "composer_observations": len(
                                [c for c in datasets["composer"]
                                 if c.get("row_type") == "observation"]),
                            "composer_state": compst,
                            "reviews": len(datasets["reviews"]),
                            "reviews_state": revst,
                            "memory_lock_modules": len(
                                [r for r in datasets["events"]
                                 if r.get("subsystem") == "memory-lock"]),
                            "heartbeat_planes": len(hb["planes"]),
                            "sources_enrolled": len(spec["sources"]),
                            "queue_is_not_whole_compute": True}]
    cov = coverage  # coverage rows were appended per enrolled source above
    frame = model.frame(model.digest({"sources": sorted(c["source"] + ":" + c["kind"] for c in cov),
                                      "limits": limits, "retention": spec["retention"],
                                      "activated": True, "mode": mode}),
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
    current_planes = [p for p in hb["planes"]
                      if collectors.plane_fresh(p, now, DEFAULT_TTLS["heartbeat"]) == "current"
                      and p.get("quality") == "verified"]
    if funding_findings:
        out.append({"id": "check:funding", "scope": "funding", "verdict": "falsified",
                    "detail": "%d active funding findings (unknown readers): %s" % (
                        len(funding_findings),
                        ", ".join(str(f.get("id", ""))[:40] for f in funding_findings[:4]))})
    elif current_planes:
        out.append({"id": "check:funding", "scope": "funding", "verdict": "held",
                    "detail": "%d funding planes verified and current in native units" % len(current_planes)})
    else:
        stale_n = len(hb["planes"]) - len(current_planes)
        out.append({"id": "check:funding", "scope": "funding", "verdict": "unknown",
                    "detail": ("no current verified funding plane; %d plane(s) stale, "
                               "failed or untimeable — an old heartbeat never "
                               "certifies current capacity" % stale_n)
                              if hb["planes"] else "heartbeat state absent"})
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
    elif family in ("worker", "workers"):
        for r in items:
            print("  %-30.30s %-9.9s %-10.10s %-12.12s %s" % (
                r.get("id"), r.get("row_type") or "worker",
                r.get("status") or r.get("state") or "?",
                r.get("origin", ""), str(r.get("note", ""))[:50]))
    elif family == "composer":
        for r in items:
            if r.get("row_type") == "observation":
                print("  %-52.52s %-9.9s %-18.18s det=%s/%s rep=%s last=%s" % (
                    r.get("id"), r.get("severity"), r.get("status"),
                    r.get("detector_id"), r.get("detector_version"),
                    r.get("repeat_count"), r.get("last_at")))
            else:
                print("  %s %s" % (r.get("id"), json.dumps(
                    {k: v for k, v in r.items()
                     if k not in ("id", "row_type")}, sort_keys=True)[:150]))
    elif family == "reviews":
        for r in items:
            print("  %-40.40s %-12.12s %-9.9s editor=%-24.24s chain=%s" % (
                r.get("review_id"), r.get("resolution") or "open",
                r.get("review_state"), r.get("editor_bound") or "-",
                r.get("chain_state")))
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


def dream_logs(work_id, attempt_id=None, frame=None, root=None):
    """Live bounded tail of ONE attempt's run log, gated by capture-time
    coverage. The log body is shown to the human, redacted, never stored.
    The selection happens in the frame the caller pinned (--snapshot): a
    history selection never silently jumps to the latest frame."""
    now = time.time()
    if frame is None:
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


def dream_open(work_id, args, frame=None, root=None):
    """Selectable human jump: resolve the work item to its exact current —
    or --attempt-selected — attempt in the frame the caller pinned
    (--snapshot) and any native session identity, print the exact facts.
    Selecting a history frame or a physical attempt never silently falls
    back to the latest frame or the current attempt."""
    now = time.time()
    if frame is None:
        frame = store.latest_snapshot(root=root, since=now - 30)
        if frame is None:
            frame = capture(now=now, root=root)
    wanted_attempt = args.get("attempt")
    w, a = _attempt_for(frame, work_id, wanted_attempt)
    if w is None:
        return {"error": "work %s not in the selected snapshot; run ygg dream ls" % work_id}, 3
    if wanted_attempt and a is None:
        return {"error": "attempt %s is not in the selected snapshot for %s"
                         % (wanted_attempt, work_id)}, 3
    session = (a or {}).get("session_id")
    print("work      %s" % w["id"])
    print("queue_id  %s   state %s   route %s" % (w.get("queue_id"), w.get("state"), w.get("route")))
    if a:
        print("attempt   %s  state %s  evidence %s" % (a["id"], a.get("state"), a.get("evidence_state")))
    if session:
        print("session   %s" % session)
        print(JUMP_UNAVAILABLE)
        return {"work_id": w["id"], "session_id": session,
                "jump": "unavailable",
                "error": "native identity observed; a verified harness attach adapter is still required"}, 3
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
    refresh = bool(args.pop("refresh", False))
    requested = args.get("snapshot")
    try:
        if requested:
            # An explicitly selected snapshot is a pin: refresh cannot bypass
            # it, and a cursor can never escape it.
            frame = store.load_snapshot(requested, root=root)
            if frame is None:
                print("ygg: retained snapshot %s not found" % requested, file=sys.stderr)
                return 3
        else:
            frame = None
            if not refresh:
                frame = store.latest_snapshot(root=root, since=now - 120)
                if frame is not None:
                    # Reuse only while every source is still inside its own
                    # TTL: a stale cached snapshot must not impersonate
                    # fresh data.
                    as_of = model.epoch(frame.get("as_of")) or 0
                    ttls = [c.get("ttl_seconds") for c in frame.get("coverage", [])
                            if isinstance(c.get("ttl_seconds"), (int, float)) and c["ttl_seconds"] > 0]
                    if not ttls or now - as_of >= min(ttls):
                        frame = None
            if frame is None:
                # refresh=True (or an expired cache) forces an exact new frame.
                frame = capture(now=now, root=root)
    except ValueError as exc:
        print("ygg: " + str(exc), file=sys.stderr)
        return 3
    if path == ("dream", "logs"):
        result, code = dream_logs(args.get("id"), args.get("attempt"),
                                  frame=frame, root=root)
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
        result, code = dream_open(args.get("id"), args, frame=frame, root=root)
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
