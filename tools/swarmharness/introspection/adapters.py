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


def enrollment(config_state_dir=None):
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
        res = sc.load() if config_state_dir is None else sc.load(state_dir=config_state_dir)
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
    # The opening identity is covered by the editor receipt. A later worker
    # annotation is not authority to associate the review with another job.
    for r in rows[:1]:
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


def _binding_failure(rows, claim, snaps):
    """Trusted-review association: the chain must be BOUND to this exact
    work — the audit row's native session identity must appear in the chain,
    and the chain's approved subject digest must equal the claimed repair's
    subject digest. A valid COMPLETE receipt of ANOTHER work never verifies
    this repair just because an audit row carries its review_id; missing
    proof of any binding stays unverified with the exact reason."""
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
    approved = [s.get("subject_digest") for s in snaps
                if isinstance(s.get("subject_digest"), str)]
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
        binding = _binding_failure(rows, claim,
                                   [r for r in rows if r.get("kind") == "snapshot_staged"][-1:])
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
    matching = [s for s in snaps if s.get("subject_digest") == verdict.get("snapshot_digest")]
    if not matching:
        return "unverified", "review chain carries no staged snapshot for the verdict"
    if snaps[-1]["subject_digest"] != verdict.get("snapshot_digest") or snaps[-1]["seq"] > verdict["seq"]:
        return "unverified", "latest snapshot has not been accepted by the editor"
    if verdict.get("verification_head") != verdict.get("prev_hash"):
        return "unverified", "editor receipt is detached from the actual review history"
    try:
        ok = verifier(review.review_id, rows[0], matching[-1],
                      verdict.get("editor"), verdict.get("signed_payload"),
                      verdict.get("verification_head")) is True
    except Exception as exc:
        return "unverified", ("editor receipt verification failed: "
                              + type(exc).__name__)
    if not ok:
        return "unverified", "editor invocation receipt did not authenticate"
    payload = verdict.get("signed_payload") or {}
    if not isinstance(payload, dict):
        return "unverified", "authenticated editor payload is malformed"
    for field in ("overall", "approval_scope", "checks_evidence", "prior_dispositions"):
        if verdict.get(field) != payload.get(field):
            return "unverified", "verdict fields disagree with the authenticated editor payload"
    signed_findings = payload.get("findings")
    actual_findings = verdict.get("findings")
    critical_fields = ("id", "criterion", "blocking", "probe")
    if (not isinstance(signed_findings, list) or not isinstance(actual_findings, list)
            or any(not isinstance(f, dict) for f in signed_findings + actual_findings)
            or [{k: f.get(k) for k in critical_fields} for f in signed_findings]
            != [{k: f.get(k) for k in critical_fields} for f in actual_findings]):
        return "unverified", "findings disagree with the authenticated editor payload"
    if any(r.get("kind") == "debt_accepted" and r["seq"] > verdict["seq"] for r in rows):
        return "unverified", "later debt has not been included in the authenticated review"
    try:
        decision = review._evaluate(
            current_subject_digest=verdict["snapshot_digest"],
            checks_evidence=payload.get("checks_evidence"))
    except Exception as exc:
        return "unverified", "review acceptance cannot be rederived: " + type(exc).__name__
    if decision.get("outcome") != "COMPLETE":
        return "unverified", "review is not accepted: " + "; ".join(decision.get("reasons", []))[:240]
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


def capture(now=None, root=None, config_state_dir=None):
    """One bounded read-only pass -> frame. The enrollment mode decides what
    is read: standing defaults, exactly the enrolled configured sources, or
    nothing (fail closed)."""
    now = now if now is not None else time.time()
    spec = enrollment(config_state_dir=config_state_dir)
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
        _coverage("pressure:" + host, "pressure", pst, now, DEFAULT_TTLS["pressure"],
                  source_observed_at=(pdetail.get("heartbeat_as_of")
                                      if isinstance(pdetail, dict) else None),
                  reason=str(pdetail)[:140] if pst != "ok" else None,
                  host=host, collection_mode=mode),
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
                                          source_observed_at=now, collection_mode=mode))
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
                                      collection_mode=mode))
        elif kind == "heartbeat":
            hb, hbst, hbdetail = collectors.collect_heartbeat(now, s.get("path"))
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
                                      collection_mode=mode))
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
                                      reason=reason, collection_mode=mode))
        elif kind == "schedule":
            rows, schst, schdetail = collectors.collect_timers(s.get("units") or [], now, limits)
            datasets["schedules"].extend(rows)
            coverage.append(_coverage(src, kind, schst, now, ttl,
                                      source_observed_at=now if schst == "ok" else None,
                                      reason=str(schdetail)[:120] if schst != "ok" else None,
                                      collection_mode=mode))
        elif kind == "task_ledger":
            row = collectors.parse_tasks_ledger(s.get("path"))
            if row:
                datasets["schedules"].append({"id": "schedule:ledger:" + sid, "kind": "commitment",
                                              **row})
                coverage.append(_coverage(src, kind, "ok", now, ttl,
                                          source_observed_at=row.get("observed_at"),
                                          collection_mode=mode))
            else:
                coverage.append(_coverage(src, kind, "absent", now, ttl, collection_mode=mode))
        elif kind == "composer":
            rows_c, cst, cdet = collectors.collect_composer(now, limits, s.get("path"))
            datasets["composer"].extend(rows_c)
            coverage.append(_coverage(src, kind, cst, now, ttl,
                                      source_observed_at=max(
                                          ((r.get("last_at") or 0) for r in rows_c
                                           if r.get("row_type") == "observation"),
                                          default=0) or None,
                                      reason=str(cdet)[:140] if cst != "ok" else None,
                                      collection_mode=mode))
        elif kind == "reviews":
            rows_r, rst, rdt = collectors.collect_reviews(now, limits, s.get("path"))
            datasets["reviews"].extend(rows_r)
            coverage.append(_coverage(src, kind, rst, now, ttl,
                                      source_observed_at=max(
                                          ((r.get("resolution_at") or 0) for r in rows_r),
                                          default=0) or None,
                                      reason=(str(rst) + ": " + str(rdt)[:120])
                                             if rst != "ok" else None,
                                      collection_mode=mode))
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
                                      collection_mode=mode))
        elif kind == "workers":
            rows_w, wst, wdetail = collectors.collect_workers(now, s.get("path"))
            datasets["workers"].extend(rows_w)
            coverage.append(_coverage(src, kind, wst, now, ttl,
                                      source_observed_at=(wdetail.get("newest_mtime")
                                                          if isinstance(wdetail, dict) else None),
                                      reason=str(wdetail)[:140] if wst != "ok" else None,
                                      host=enrolled_host(s), collection_mode=mode))
        elif kind == "pressure":
            rows_p, pst, pdetail = collectors.collect_pressure(now, ttl, s.get("path"))
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
                                      collection_mode=mode))

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


# ------------------------------------------------------------- human view
# Everything below is the pure human projection of a query result: it only
# reads the result dict and prints. The machine contract is the --json path
# in serve() (json.dumps of the same result) and is never routed through
# this code. Formatting rules:
#   - every cell AND every dynamic key (execution count keys, flattened
#     dict keys, summary area keys) is single-line sanitized text: C0/C1/
#     ANSI bytes, tabs and embedded newlines never reach the output, and
#     credential-like strings are redacted;
#   - truncation is always marked with an explicit "..." suffix and bounded
#     row lists (grants, windows, fields) declare their omitted remainder
#     with a total, instead of silently pretending a shortened value or a
#     clipped list is the whole one;
#   - missing values render as "-" or an explicit word (not-collected,
#     unreported) and are never folded into 0 or "healthy";
#   - effective freshness is derived from ALL contributing assessed
#     coverage rows of a kind (worst source wins; unattributed areas stay
#     UNKNOWN), so a stale source's healthy-looking recorded state is
#     labeled "last reported" with its capture-time anchor on the same
#     row, never presented as current health;
#   - one logical row per line so piped grep/awk/sed stay stable.


def _cell(value, limit=160):
    """One display cell: bounded, single-line, credential-redacted text.
    None is "-" (missing), booleans are yes/no, numbers keep their value.
    C0 tabs are flattened to spaces here (the shared text bound keeps them,
    but a tab must never shift human output columns)."""
    if value is None:
        return "-"
    if isinstance(value, bool):
        return "yes" if value else "no"
    if isinstance(value, float):
        value = "%g" % value
    return model.text(str(value), limit).replace("\t", " ")


def _clip(value, width):
    """Bounded-width cell that MARKS truncation with "..." instead of
    pretending the full value was shown; --json carries the whole field."""
    text = _cell(value, max(width * 3, 160))
    return text if len(text) <= width else text[:max(1, width - 3)] + "..."


def _pad(value, width):
    return _clip(value, width).ljust(width)


def _age_text(seconds):
    """Compact human duration; an untimeable or future-skewed age stays "?"
    rather than being rounded into a confident-looking number."""
    if seconds is None:
        return "?"
    seconds = int(seconds)
    if seconds < 0:
        return "?"
    if seconds < 60:
        return "%ds" % seconds
    if seconds < 3600:
        return "%dm%02ds" % (seconds // 60, seconds % 60)
    if seconds < 86400:
        return "%dh%02dm" % (seconds // 3600, (seconds % 3600) // 60)
    return "%dd%02dh" % (seconds // 86400, (seconds % 86400) // 3600)


def _row_age(row, anchor):
    """Age of the row's own observation clock against the snapshot anchor —
    never the reader's wall clock. "?" when the row carries no usable time."""
    if anchor is None:
        return "?"
    for key in ("age_seconds", "observed_at", "source_observed_at", "last_at"):
        v = row.get(key)
        if v is None:
            continue
        if key == "age_seconds":
            return _age_text(v)
        t = model.epoch(v)
        if t is not None:
            return _age_text(anchor - t)
    return "?"


def _loud(freshness):
    """Freshness word for an observation row: only "current" stays as is;
    everything else is rendered as the loud word it is (STALE/UNKNOWN). A
    row without a usable freshness word is UNKNOWN, not a quiet hyphen."""
    text = _clip(freshness, 12)
    if text in ("-", ""):
        return "UNKNOWN"
    return text if text == "current" else text.upper()


_QUALITY_RANK = {"verified": 0, "stale": 1, "conflicting": 2, "unavailable": 3}
_WORD_RANK = {"current": 0, "STALE": 1, "CONFLICTING": 2, "UNAVAILABLE": 3}


def _kind_rows(coverage, kind):
    """ALL assessed coverage rows contributing to one kind. A kind is only
    as fresh as its worst enrolled source, so every row participates — a
    fresh first row never certifies a later stale or unavailable one."""
    return [c for c in (coverage or []) if c.get("kind") == kind]


def _worst_row(rows):
    """The most demoted row of a kind (None when the kind has no coverage
    row at all). Unknown quality and unrankable rows rank as the worst
    state there is, never as a healthy one."""
    if not rows:
        return None
    return max(rows, key=lambda c: _QUALITY_RANK.get(str(c.get("quality")), 4))


def _effective_word(cov_row):
    """Query-time assessed freshness word for one row's source coverage:
    "current" only when a verified in-TTL coverage row exists; every other
    assessed quality renders as the loud word it is; no coverage row at all
    is UNKNOWN, never silently strong-fresh."""
    if cov_row is None:
        return "UNKNOWN"
    q = str(cov_row.get("quality") or "unknown")
    return "current" if q == "verified" else q.upper()


def _worse_word(a, b):
    """The more demoted of two freshness words: a query-time assessment may
    demote a recorded capture-time word but never freshens it."""
    return max((a, b), key=lambda w: _WORD_RANK.get(w, 4))


def _coverage_by_kind(coverage):
    """The WORST assessed coverage row per kind: an aggregate that cannot
    attribute its inputs beyond kind level inherits the worst contributing
    source rather than the first-seen one, and a kind with no coverage row
    stays absent (rendered UNKNOWN downstream) — never verified."""
    kinds = {c.get("kind") for c in (coverage or []) if c.get("kind")}
    return {k: _worst_row(_kind_rows(coverage, k)) for k in kinds}


def _freshness(cov_row):
    """Effective freshness words for an assessed coverage row. Only a
    verified in-TTL source reads "verified"; stale/unavailable/conflicting/
    unknown (or a missing coverage row) render as the honest loud words and
    are never folded into a healthy one."""
    if cov_row is None:
        return "UNKNOWN"
    q = cov_row.get("quality")
    if q == "verified":
        return "verified age=%s" % _age_text(cov_row.get("age_seconds"))
    if q == "stale":
        return "STALE age=%s" % _age_text(cov_row.get("age_seconds"))
    if q == "conflicting":
        return "CONFLICTING"
    if q == "unavailable":
        return "UNAVAILABLE"
    return "UNKNOWN"


def _last_reported_note(state, cov_row, label):
    """The stale-ok honesty clause. A healthy-looking recorded state (state=ok)
    read from a source that is not verified-fresh is that source's LAST
    REPORTED value; the note says so on its own line instead of letting the
    reader take stale ok as current health."""
    if state is None:
        return None
    if cov_row is None:
        if str(state).lower() in ("ok", "verified", "active", "healthy"):
            return ("note: %s=%s is recorded without a coverage row; its "
                    "freshness is UNKNOWN" % (label, _clip(state, 20)))
        return None
    if cov_row.get("quality") == "verified":
        return None
    if str(state).lower() not in ("ok", "verified", "active", "healthy"):
        return None
    return ("note: %s=%s is the last reported value from a %s source (age %s); "
            "it does not assert current health"
            % (label, _clip(state, 20), str(cov_row.get("quality", "unknown")).upper(),
               _age_text(cov_row.get("age_seconds"))))


def _flatten(value, max_fields=12):
    """Bounded deterministic key=value flattening for JSON-shaped rows that
    have no dedicated human layout (unknown composer rows, nested provider
    windows, extra summary keys). Dicts widen to dotted keys, lists to [i]
    indexes; every leaf is sanitized; overflow is declared as a "+fields"
    pair — never silently dropped, never rendered as raw JSON."""
    pairs = []

    def walk(v, key, depth):
        if len(pairs) >= 200 or depth > 4:
            pairs.append((key or "value", "(bound reached; see --json)"))
            return
        if isinstance(v, dict):
            if not v:
                pairs.append((key or "value", "empty-object"))
                return
            for k in sorted(v, key=str):
                walk(v[k], (key + "." if key else "") +
                     _clip(k, 40).replace(" ", "_"), depth + 1)
        elif isinstance(v, list):
            if not v:
                pairs.append((key or "value", "empty-list"))
                return
            for i, item in enumerate(v[:4]):
                walk(item, "%s[%d]" % (key, i), depth + 1)
            if len(v) > 4:
                pairs.append((key + "[4..]", "(%d more; see --json)" % (len(v) - 4)))
        else:
            pairs.append((key or "value", _cell(v)))

    walk(value, "", 0)
    if len(pairs) > max_fields:
        return pairs[:max_fields] + [("+fields", "%d more; see --json"
                                     % (len(pairs) - max_fields))]
    return pairs


def _kv(pairs, width=60, preferred=()):
    """key=value line; keys named in ``preferred`` come first (in that
    order) so the common fields read naturally, the rest stay sorted."""
    def rank(item):
        k = item[0].split(".")[-1].split("[")[0]
        return (preferred.index(k) if k in preferred else len(preferred), item[0])
    ordered = sorted(pairs, key=rank) if preferred else pairs
    return " ".join("%s=%s" % (k, _clip(v, width)) for k, v in ordered)


def _emit(result, as_json):
    if as_json:
        print(json.dumps(result, indent=1))
        return
    print("snapshot %s (short id; --json carries the full id and verbatim data)"
          "  as_of %s  query_as_of %s"
          % (str(result.get("snapshot_id", ""))[:16], result.get("as_of"),
             result.get("query_as_of")))
    print("coverage (effective freshness; a newer reader clock never freshens "
          "an old observation):")
    for c in result["coverage"]:
        age = ("never-observed" if not c.get("observed_at")
               else _age_text(c.get("age_seconds")))
        quality = str(c.get("quality") or "unknown")
        line = "  %s %s age=%s ttl=%ss" % (
            _pad(c.get("source"), 30),
            _pad(quality if quality == "verified" else quality.upper(), 12),
            _pad(age, 15), _cell(c.get("ttl_seconds"), 12))
        if c.get("reason"):
            line += "  " + _clip(c["reason"], 110)
        print(line)
    for f in result["findings"]:
        ident = f.get("id") or " ".join(str(x) for x in (f.get("command") or []))
        print("  ! finding code=%s severity=%s %s" % (
            _pad(f.get("code"), 22), _pad(f.get("severity"), 8),
            _clip(ident, 60)))
    print("total %d%s" % (result["total"], "" if result.get("next_cursor") is None
                          else " (next cursor available)"))


def _area_line(label, pairs, cov_row, contributing=()):
    """One scan-friendly key=value row with its effective freshness; the
    freshness always comes from the assessed coverage row — the WORST of
    all contributing sources of the kind (see _coverage_by_kind) — never
    from the recorded state value itself. Keys and values are single-line
    sanitized cells, and a kind fed by more than one source declares that
    count instead of letting one row speak for the whole kind."""
    parts = [_pad(label, 13)] + ["%s=%s" % (_cell(k, 40), _clip(v, 60))
                                 for k, v in pairs]
    parts.append("freshness=" + _freshness(cov_row))
    if len(contributing) > 1:
        parts.append("(worst of %d sources)" % len(contributing))
    print("  " + " ".join(parts))


def _render_status(result):
    """swarm status: the summary row as labeled sections, cross-referenced
    with the frame's assessed coverage. The recorded state values are shown
    verbatim; the freshness verdict is computed from coverage only, so a
    stale source's state=ok reads as "last reported", never current health.
    Missing datasets render as not-collected, never as 0."""
    cov = _coverage_by_kind(result.get("coverage"))
    raw_cov = result.get("coverage") or []
    items = result.get("items") or []

    def area(label, pairs, kind):
        """One status area: the kind's worst coverage row drives freshness;
        every contributing source of the kind is counted on the row."""
        _area_line(label, pairs, cov.get(kind),
                   [c for c in raw_cov if c.get("kind") == kind])

    if not items:
        print("  no summary row in this snapshot (see coverage above; a missing "
              "summary is not a healthy fleet)")
        return
    s = items[0]
    mode = _clip(s.get("collection_mode"), 20)
    if mode in ("disabled", "invalid"):
        print("  collection mode=%s — fail-closed: no sources were read; "
              "nothing below is an observation" % mode)
    elif mode == "standing":
        print("  collection mode=standing — standing local collection; NOT a "
              "complete fleet view")
    else:
        print("  collection mode=%s" % mode)
    if s.get("note"):
        print("  note: " + _clip(s["note"], 160))

    known = {"id", "collection_mode", "note"}

    def used(*keys):
        known.update(keys)

    if "dreams" in s:
        used("dreams", "running", "failed", "queue_is_not_whole_compute")
        if s.get("dreams") is None:
            area("work", [("rows", "not-collected")], "queue")
            print("    (no work dataset in this capture; not the same as zero work)")
        else:
            area("work", [("rows", s.get("dreams")), ("running", s.get("running")),
                       ("failed", s.get("failed"))], "queue")
            print("    (running counts queue-scope work rows only; running=0 is "
                  "not zero agents)")
    if "workers" in s or "workers_state" in s:
        used("workers", "workers_state")
        rows = "not-collected" if s.get("workers") is None else s.get("workers")
        area("workers", [("registry_rows", rows),
                          ("source_state", s.get("workers_state"))], "workers")
        note = _last_reported_note(s.get("workers_state"), cov.get("workers"),
                                   "workers_state")
        if note:
            print("    " + note)
    if "composer_observations" in s or "composer_state" in s:
        used("composer_observations", "composer_state")
        obs = ("not-collected" if s.get("composer_observations") is None
               else s.get("composer_observations"))
        area("composer", [("observations", obs),
                           ("state", s.get("composer_state"))], "composer")
        note = _last_reported_note(s.get("composer_state"), cov.get("composer"),
                                   "composer_state")
        if note:
            print("    " + note)
    if "reviews" in s or "reviews_state" in s:
        used("reviews", "reviews_state")
        chains = "not-collected" if s.get("reviews") is None else s.get("reviews")
        area("reviews", [("chains", chains),
                          ("state", s.get("reviews_state"))], "reviews")
        note = _last_reported_note(s.get("reviews_state"), cov.get("reviews"),
                                   "reviews_state")
        if note:
            print("    " + note)
    if "memory_lock_modules" in s or "memory_lock_state" in s:
        used("memory_lock_modules", "memory_lock_state")
        mods = ("not-collected" if s.get("memory_lock_modules") is None
                else s.get("memory_lock_modules"))
        area("memory-lock", [("modules", mods),
                              ("state", s.get("memory_lock_state"))], "memory_lock")
        note = _last_reported_note(s.get("memory_lock_state"),
                                   cov.get("memory_lock"), "memory_lock_state")
        if note:
            print("    " + note)
    if "heartbeat_planes" in s or "active_findings" in s:
        used("heartbeat_planes", "active_findings")
        area("heartbeat", [("planes", s.get("heartbeat_planes")),
                            ("active_findings", s.get("active_findings"))], "heartbeat")
    if "services" in s:
        used("services", "services_dynamic_instances")
        area("services", [("units", s.get("services")),
                           ("dynamic_instances",
                            s.get("services_dynamic_instances"))], "services")
    if "schedules" in s:
        used("schedules")
        area("schedules", [("rows", s.get("schedules"))], "schedule")
    if "events" in s:
        used("events")
        area("events", [("rows", s.get("events"))], "events")
    if "sources_enrolled" in s:
        used("sources_enrolled")
        area("enrollment", [("sources_enrolled", s.get("sources_enrolled"))], None)
    leftovers = {k: v for k, v in s.items() if k not in known}
    if leftovers:
        print("  other " + _kv(_flatten(leftovers, max_fields=10)))


def _grant_line(g, with_ends=True):
    if not isinstance(g, dict):
        print("    grant " + _clip(g, 90))
        return
    ends = model.epoch(g.get("ends_at")) if with_ends else None
    tail = (" ends=%s" % (model.iso(ends) if ends is not None
                          else _cell(g.get("ends_at"), 24))) if with_ends else ""
    print("    grant id=%s grant_units=%s remaining=%s status=%s%s" % (
        _pad(_clip(g.get("id"), 26), 26),
        _pad(_cell(g.get("grant_units"), 18), 12),
        _pad(_cell(g.get("remaining"), 16), 12),
        _pad(g.get("status"), 10), tail))


def _time_cell(value, limit=26):
    """A time-ish cell: epoch numbers render as UTC ISO (times are facts,
    not exponents); anything else falls back to the plain cell."""
    t = model.epoch(value) if not isinstance(value, str) else None
    return _clip(model.iso(t) if t is not None else value, limit)


def _window_lines(row, limit=4):
    """Nested provider windows flattened field by field. Values keep the
    provider's own fields and unit meaning; nothing is converted or invented,
    and windows beyond the shown bound are declared, never dropped."""
    windows = row.get("windows") or []
    for i, w in enumerate(windows[:limit]):
        if isinstance(w, dict):
            print("    window[%d] %s" % (i, _kv(_flatten(w, max_fields=8), width=40,
                                                preferred=("remaining", "usage", "number",
                                                           "nextResetTime"))))
        else:
            print("    window[%d] %s" % (i, _clip(w, 80)))
    if len(windows) > limit:
        print("    windows[%d..] (%d more; see --json)" % (limit, len(windows) - limit))


def _render_pressure(result):
    """swarm pressure: funding planes in their native units (a missing unit
    stays "-", never invented), execution counts labeled as the queue's own
    tallies. Every row carries the EFFECTIVE freshness of its source kind —
    the worst contributing assessed coverage row at query time — so a stale
    assessment demotes the row: its recorded capture-time words (freshness,
    observed age) stay on the row but labeled as capture facts, and the
    recorded numbers read as LAST REPORTED, explicitly not current
    capacity."""
    anchor = model.epoch(result.get("as_of"))
    eff = _worst_row(_kind_rows(result.get("coverage"), "pressure"))
    eff_age = _age_text(eff.get("age_seconds")) if eff else "?"
    funding = False
    for r in result.get("items") or []:
        kind = r.get("kind")
        if kind == "funding":
            funding = True
            recorded = _loud(r.get("plane_freshness"))
            word = _worse_word(recorded, _effective_word(eff))
            print("  funding plane=%s reader=%s freshness=%s age=%s observed_age=%s" % (
                _pad(r.get("plane"), 18), _pad(r.get("reader_quality"), 10),
                _pad(word, 8), _pad(eff_age, 7), _pad(_row_age(r, anchor), 6)))
            if word != "current":
                print("    reported(%s, age %s, at capture %s); the plane state "
                      "below is the LAST REPORTED observation — it does not "
                      "assert current health or capacity"
                      % (recorded, _row_age(r, anchor), result.get("as_of")))
            if r.get("unknown_reader"):
                print("    unknown reader: the numbers below are the LAST "
                      "REPORTED observation, not current capacity")
            if r.get("error"):
                print("    reader error: " + _clip(r["error"], 130))
            grants = r.get("grants") or []
            for g in grants[:6]:
                _grant_line(g)
            if len(grants) > 6:
                print("    grants[6..] (%d more; see --json; %d total)"
                      % (len(grants) - 6, len(grants)))
            _window_lines(r)
            if r.get("available") is not None:
                print("    available=%s (as reported by the provider)"
                      % _clip(r["available"], 80))
        elif kind == "execution":
            word = _effective_word(eff)
            print("  execution plane=%s reader=%s freshness=%s age=%s" % (
                _pad(r.get("plane"), 18),
                _pad(r.get("reader_quality") or r.get("quality"), 10),
                _pad(word, 8), _pad(eff_age, 7)))
            if word != "current":
                print("    note: these counts are the LAST REPORTED tallies "
                      "(recorded at capture %s); freshness %s — not current "
                      "queue state" % (result.get("as_of"), word))
            counts = r.get("counts")
            if isinstance(counts, dict) and counts:
                print("    counts " + " ".join(
                    "%s=%s" % (_clip(k, 40), _clip(v, 12))
                    for k, v in sorted(counts.items(), key=lambda kv: str(kv[0]))))
            elif counts is not None:
                print("    counts none-reported")
            print("    note: counts are the queue's own tallies; running=0 is "
                  "not zero agents")
            serve = r.get("serve")
            if isinstance(serve, dict):
                print("    serve alive=%s" % _clip(serve.get("alive"), 10))
            else:
                print("    serve unobserved (queue liveness is not part of "
                      "this reading)")
        else:
            print("  %s %s" % (_pad(kind, 10), _kv(_flatten(
                {k: v for k, v in r.items() if k not in ("id", "kind")},
                max_fields=10))))
    if funding:
        print("  units: funding values keep the provider's native unit names; "
              "'-' means the provider did not report one — none is invented; "
              "remaining balances unavailable at the CLI stay unknown")


def _render_heartbeat(result):
    """swarm heartbeat: one line per plane/finding, then each grant and
    nested provider window on its own line in native units. The plane row's
    freshness word is the WORSE of its recorded capture-time freshness and
    the effective query-time assessment of the heartbeat source; a demoted
    row keeps its recorded words only as labeled capture facts and reads as
    LAST REPORTED, never as current plane health."""
    anchor = model.epoch(result.get("as_of"))
    eff = _worst_row(_kind_rows(result.get("coverage"), "heartbeat"))
    eff_age = _age_text(eff.get("age_seconds")) if eff else "?"
    for r in result.get("items") or []:
        if r.get("row_type") == "finding":
            print("  finding %s kind=%s active=%-5s count=%-5s %s" % (
                _pad(r.get("id"), 34), _pad(r.get("kind"), 16),
                _pad(_cell(r.get("active"), 6), 5), _pad(_cell(r.get("count"), 8), 5),
                _clip(r.get("detail"), 70)))
            continue
        recorded = _loud(r.get("freshness"))
        word = _worse_word(recorded, _effective_word(eff))
        print("  plane %s quality=%s freshness=%s age=%s observed_age=%s grants=%d" % (
            _pad(r.get("plane"), 16), _pad(r.get("quality"), 9),
            _pad(word, 8), _pad(eff_age, 7), _pad(_row_age(r, anchor), 6),
            len(r.get("grants") or [])))
        if word != "current":
            print("    reported(%s, age %s, at capture %s); the plane state "
                  "below is the LAST REPORTED observation — it does not "
                  "assert current plane health"
                  % (recorded, _row_age(r, anchor), result.get("as_of")))
        if r.get("error"):
            print("    reader error: " + _clip(r["error"], 120))
        grants = r.get("grants") or []
        for g in grants[:8]:
            _grant_line(g, with_ends=False)
        if len(grants) > 8:
            print("    grants[8..] (%d more; see --json; %d total)"
                  % (len(grants) - 8, len(grants)))
        _window_lines(r)
        if r.get("note"):
            print("    note: " + _clip(r["note"], 120))


def _render_events(items):
    for r in items:
        t = model.epoch(r.get("at"))
        print("  %-12s %-8s %-20s %s" % (
            _pad(r.get("subsystem"), 12), _pad(r.get("severity"), 8),
            _pad(model.iso(t) if t is not None else r.get("at"), 20),
            _clip(r.get("summary"), 80)))


def _dream_rows(items):
    for r in items:
        att = r.get("reported_attempt_count")
        print("  %s state=%s route=%s attempts=%s %s" % (
            _pad(r.get("queue_id") or r.get("id"), 28), _pad(r.get("state"), 13),
            _pad(r.get("route"), 16),
            _pad(att if att is not None else "unreported", 10),
            _clip(r.get("title"), 64)))
        for a in r.get("attempts") or []:
            print("    attempt %s state=%s ord=%-3s started=%s finished=%s evidence=%s" % (
                _pad(str(a.get("id", "")).replace("attempt:queue:", ""), 38),
                _pad(a.get("state"), 11), _pad(a.get("ordinal"), 3),
                _pad(_time_cell(a.get("started_at")), 26),
                _pad(_time_cell(a.get("finished_at")), 26),
                _clip(a.get("evidence_state"), 24)))
            if a.get("reason"):
                print("      reason: %s" % _clip(a["reason"], 120))
        for rp in r.get("repairs") or []:
            print("    repair %s verdict=%s verification=%s" % (
                _pad(rp.get("ref"), 40), _pad(rp.get("verdict"), 12),
                _clip(rp.get("verification"), 40)))


def _service_rows(items):
    for r in items:
        rss = ""
        if r.get("rss_bytes"):
            rss = " rss=%dMiB" % (r["rss_bytes"] // (1024 * 1024))
        print("  %s %-9s %-9s pid=%-7s src=%-11s%s" % (
            _pad(r.get("unit"), 46), _pad(r.get("active"), 9),
            _pad(r.get("sub"), 9), _pad(r.get("pid"), 7),
            _pad(r.get("provenance") or "systemd", 11), rss))


def _schedule_rows(items):
    for r in items:
        kind = r.get("kind", "timer")
        when = r.get("next_text") or r.get("expires_at") or ""
        extra = ""
        if kind == "plan":
            extra = " staged=%s target=%s remaining=%s" % (
                _clip(r.get("staged"), 10), _clip(r.get("target"), 10),
                _clip(r.get("remaining"), 10))
        elif kind == "commitment":
            extra = " rev=%s authority=%s" % (_clip(r.get("revision"), 16),
                                              _clip(r.get("authority"), 16))
        print("  %s %-11s %s%s" % (_pad(r.get("id"), 46), _pad(kind, 11),
                                   _clip(_time_cell(when) if when else "", 26), extra))


def _worker_rows(items):
    for r in items:
        print("  %s %-10s %-10s %-12s %s" % (
            _pad(r.get("id"), 30), _pad(r.get("row_type") or "worker", 10),
            _pad(r.get("status") or r.get("state") or "?", 10),
            _pad(r.get("origin"), 12), _clip(r.get("note"), 50)))


def _composer_rows(items, anchor):
    for r in items:
        if r.get("row_type") == "observation":
            print("  obs %s status=%s severity=%-8s detector=%s/%s repeats=%-4s last_age=%s" % (
                _pad(r.get("id"), 44), _pad(r.get("status"), 12),
                _pad(r.get("severity"), 8), _clip(r.get("detector_id"), 14),
                _clip(r.get("detector_version"), 8), _pad(r.get("repeat_count"), 4),
                _row_age(r, anchor)))
        else:
            # Rows without a closed human layout (cursors, state rows, and
            # any future row_type) get the bounded deterministic flattening
            # instead of a raw JSON dump.
            pairs = _flatten({k: v for k, v in r.items()
                              if k not in ("id", "row_type")}, max_fields=12)
            print("  %s %s %s" % (_pad(r.get("row_type"), 8),
                                  _pad(_clip(r.get("id"), 40), 40),
                                  _kv(pairs, width=44)))


def _review_rows(items):
    for r in items:
        print("  %s resolution=%s state=%s editor=%s chain=%s" % (
            _pad(r.get("review_id"), 40), _pad(r.get("resolution") or "open", 12),
            _pad(r.get("review_state"), 9), _pad(r.get("editor_bound") or "-", 24),
            _clip(r.get("chain_state"), 24)))


def _row_briefs(result, family):
    items = result.get("items") or []
    anchor = model.epoch(result.get("as_of"))
    if family == "dream":
        _dream_rows(items)
    elif family == "service":
        _service_rows(items)
    elif family == "schedule":
        _schedule_rows(items)
    elif family == "heartbeat":
        _render_heartbeat(result)
    elif family == "plans":
        _render_pressure(result)
    elif family == "events":
        _render_events(items)
    elif family == "findings":
        for r in items:
            print("  %-18s %-10s %s" % (_pad(r.get("scope"), 18),
                                        _pad(r.get("verdict"), 10),
                                        _clip(r.get("detail"), 90)))
    elif family in ("worker", "workers"):
        _worker_rows(items)
    elif family == "composer":
        _composer_rows(items, anchor)
    elif family == "reviews":
        _review_rows(items)
    elif family == "summary":
        _render_status(result)


def _empty_scope():
    print("  no rows in this query scope (see coverage above: an empty verified "
          "scope is not proof the fleet is idle)")


def _render(result, path):
    family = path[0]
    verb = path[1] if len(path) > 1 else ""
    _emit(result, False)
    if family == "swarm" and verb == "check":
        for r in result["items"]:
            print("  check %s %s %s" % (_pad(r.get("scope"), 12),
                                        _pad(r.get("verdict"), 10),
                                        _clip(r.get("detail"), 110)))
        print("  verdicts: held=assertion held  falsified=assertion falsified  "
              "unknown=evidence insufficient (unknown is not a pass)")
        return
    if family == "swarm" and verb == "pressure":
        _render_pressure(result) if result["items"] else _empty_scope()
        return
    if family == "swarm" and verb == "status":
        _render_status(result)
        return
    if family == "swarm" and verb == "heartbeat":
        _render_heartbeat(result) if result["items"] else _empty_scope()
        return
    if family == "swarm" and verb == "events":
        _render_events(result["items"]) if result["items"] else _empty_scope()
        return
    if result["items"]:
        _row_briefs(result, family)
    elif result["total"] == 0:
        _empty_scope()


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
