"""Read-only source collectors for bounded immutable inspection snapshots.

Every collector returns ``(rows, state, detail)`` where state is
``ok | absent | unreadable | invalid | stale`` — an empty verified result and
an unreadable source are different facts, and an absent file is not an
unobserved one. Rows are bounded metadata only: prompt bodies, transcripts,
raw provider errors and credentials never enter a frame. Collectors perform
no writes and no network egress beyond the enrolled read commands they are
given (queue CLI over the same ssh hop the orchestrator itself uses).

Bounds come from the enrollment ``limits`` or the documented defaults and are
narrowed, never widened, by callers.
"""
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
from datetime import datetime, timedelta, timezone
from pathlib import Path

from . import model

# Documented defaults when the optional introspection config subtree is not
# activated (the subtree is classified, not enrolled; readers already support
# it — see docs/swarm-introspection.md).
DEFAULT_LIMITS = {"read_timeout_seconds": 10, "total_timeout_seconds": 45,
                  "max_records": 2000, "max_bytes": 4 * 1024 * 1024,
                  "log_tail_bytes": 8192, "log_tail_lines": 40}
DEFAULT_RETENTION = {"raw_days": 31, "segment_bytes": 8 * 1024 * 1024,
                     "max_store_bytes": 256 * 1024 * 1024}

_HOME = Path.home()
DREAM_DIR = _HOME / ".yggterm/dream"
DREAM_CFG = _HOME / ".yggterm/config/dream/floors.json"
QUEUE_STATE = _HOME / ".local/state/zcode-tui/queue"
HEARTBEAT_STATE = _HOME / ".yggterm/swarm/heartbeat/health.json"
PLANES_CFG = _HOME / ".yggterm/config/planes.json"
SWARM_STATE = _HOME / ".yggterm/swarm"
LEDGER_PATH = _HOME / ".yggterm/memory/namespaces/_global/fleet-task-ledger.md"

# Kernel maintenance events worth retaining; anything else is noise.
_DMESG_FOCUS = re.compile(
    r"(?i)out of memory|oom-kill|killed process|segfault|general protection|"
    r"blocked for more|\bi/o error|filesystem.*read-only|thermal|throttl")
# The dreamer's own production ledger location (dream-audit.py writes it);
# this module only ever reads it.
DREAMER_STATE = Path("/tmp/dreamer")


def _state(path, reader=json.loads, max_bytes=None):
    """Read a JSON file: (value, state, detail). absent != unreadable, and an
    empty or oversized file is invalid — not a proven absence. The read is
    bounded BEFORE allocation: the size is checked first and at most one byte
    past the limit is ever read, so a huge file is refused without being
    loaded, and its content is reported unknown rather than truncated-then-
    parsed."""
    limit = max_bytes or DEFAULT_LIMITS["max_bytes"]
    p = Path(path).expanduser()
    try:
        size = p.stat().st_size
    except FileNotFoundError:
        return None, "absent", str(p)
    except (IsADirectoryError, PermissionError):
        return None, "unreadable", "permission"
    except OSError:
        return None, "unreadable", "os error"
    if size == 0:
        return None, "invalid", "empty file (not a proven absence)"
    if size > limit:
        return None, "invalid", "over read limit (%d > %d bytes); content unknown" % (size, limit)
    try:
        with p.open("rb") as f:
            raw = f.read(limit + 1)
    except OSError:
        return None, "unreadable", "os error"
    if len(raw) > limit:
        return None, "invalid", "over read limit; content unknown"
    if not raw.strip():
        return None, "invalid", "empty file (not a proven absence)"
    try:
        return reader(raw), "ok", ""
    except Exception as exc:
        return None, "invalid", type(exc).__name__


def read_jsonl(path, tail=None, max_bytes=None, now=None):
    """Bounded tail of a JSON Lines file: (rows, state, skipped, truncated).

    Malformed lines are counted and skipped, never silently dropped; a tail
    bound that cut older records reports truncated=True so callers never
    present a partial window as the whole history.
    """
    p = Path(path).expanduser()
    max_bytes = max_bytes or DEFAULT_LIMITS["max_bytes"]
    try:
        size = p.stat().st_size
    except FileNotFoundError:
        return [], "absent", 0, False
    except OSError:
        return [], "unreadable", 0, False
    if size == 0:
        # An existing-but-empty ledger is an anomaly (or a truncated write),
        # not a verified empty history and not a proven absence.
        return [], "invalid", 0, False
    start = max(0, size - max_bytes)
    truncated = start > 0
    try:
        with p.open("rb") as f:
            if start:
                f.seek(start)
            raw = f.read(max_bytes)
    except OSError:
        return [], "unreadable", 0, truncated
    rows, skipped = [], 0
    for line in raw.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            rows.append(json.loads(line))
        except Exception:
            skipped += 1
    if isinstance(tail, int) and tail is not None and len(rows) > tail:
        rows = rows[-tail:]
        truncated = True
    if not raw.strip():
        # a whitespace-only ledger is an anomaly, not a verified empty history
        return [], "invalid", skipped, truncated
    return rows, "ok", skipped, truncated


def _run(argv, timeout):
    """Bounded argv subprocess (no shell): (rc, stdout, state)."""
    if not isinstance(argv, list) or not argv or not all(isinstance(a, str) for a in argv):
        return None, "", "invalid"
    try:
        p = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout, "ok"
    except subprocess.TimeoutExpired:
        return None, "", "timeout"
    except (OSError, ValueError) as exc:
        return None, "", type(exc).__name__


def _run_shell(shell, timeout, host=None):
    """Bounded read-only shell read. Local hosts run through bash -c; remote
    hosts ride the same ssh hop the orchestrator uses. shell is a composed
    string of quoted words (see _shjoin) — never untrusted text."""
    if host and host not in ("local", os.uname().nodename.split(".")[0]):
        argv = ["ssh", "-o", "ConnectTimeout=5", "-o", "BatchMode=yes", host, shell]
    else:
        argv = ["/bin/bash", "-o", "pipefail", "-c", shell]
    try:
        p = subprocess.run(argv, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout, "ok"
    except subprocess.TimeoutExpired:
        return None, "", "timeout"
    except (OSError, ValueError) as exc:
        return None, "", type(exc).__name__


def _shjoin(words):
    import shlex
    return " ".join(shlex.quote(str(w)) for w in words)


def error_class(text):
    """Closed vocabulary for failure reasons. Raw provider/API error bodies
    (transcripts, HTTP traces) never enter frames — only the class and a
    queue-native short prefix survive, and unknown stays unknown."""
    t = str(text or "").lower()
    if any(s in t for s in ("quota", "usage limit", "rate limit", "429",
                            "hourly", "cap reached", "free tier")):
        return "quota"
    if any(s in t for s in ("auth", "unauthorized", "401", "403", "forbidden",
                            "credential", "login", "refresh token")):
        return "auth"
    if "timeout" in t or "stalled" in t:
        return "timeout-stall"
    if "budget" in t:
        return "budget"
    if "spawn" in t or "launch" in t:
        return "launch"
    if "killed" in t or "interrupt" in t:
        return "interrupted"
    return "other" if t else ""


def _fingerprint(obj):
    return hashlib.sha256(model.canonical(obj).encode()).hexdigest()[:16]


def _clean_task(row):
    """Metadata-only projection of a queue task row. The prompt text is
    dropped here, before any frame or store can see it."""
    if not isinstance(row, dict):
        return None
    keep = ("id", "route", "status", "attempts", "max_attempts", "model",
            "priority", "next_eligible_at", "created_at", "updated_at",
            "started_at", "finished_at", "last_exit", "position")
    out = {k: row.get(k) for k in keep}
    out["title"] = model.text(row.get("title"))
    err = row.get("last_error")
    err_text = model.text(err, 160) if err else ""
    low = err_text.lower()
    if err_text and ("http" in low or "error:" in low or "traceback" in low):
        # provider-adjacent failure bodies reduce to their class
        out["last_error"] = "failure-class=" + error_class(err_text)
    else:
        out["last_error"] = err_text
    run_log = row.get("run_log")
    out["run_log_ref"] = _fingerprint({"run_log": run_log}) if run_log else ""
    return out


def resolve_queue_source():
    """The orchestrator's own queue endpoint from floors.json, else the local
    default. Returns (argv_tail, host, [repo]) or (None, reason, None)."""
    cfg, state, _ = _state(DREAM_CFG)
    if state == "ok" and isinstance(cfg, dict):
        q = cfg.get("queue") or {}
        cli, repo, host = q.get("cli"), q.get("repo"), q.get("host")
        if cli and repo:
            import shlex
            tail = shlex.split(str(cli).replace("~", str(_HOME)))
            repo = str(repo).replace("~", str(_HOME))
            if tail and repo:
                return tail, host or "local", [repo]
    bun = _HOME / ".bun/bin/bun"
    script = _HOME / "gh/zcode-tui/src/bin/queue.ts"
    if bun.exists() and script.exists():
        return [str(bun), "run", str(script)], "local", [str(_HOME / "gh/zcode-tui")]
    return None, "queue cli not resolvable", None


def collect_queue(now, limits=None):
    """Executor queue: task rows, dispatch/done metrics, serve liveness, and
    run-log coverage (digests only, never log bodies)."""
    limits = limits or {}
    timeout = min(int(limits.get("read_timeout_seconds", DEFAULT_LIMITS["read_timeout_seconds"])), 40)
    resolved = resolve_queue_source()
    if resolved[0] is None:
        return {"tasks": [], "events": [], "serve": {}, "runs": []}, "absent", resolved[1]
    tail, host, prefix = resolved
    shell = "cd %s && %s" % (_shjoin(prefix), _shjoin(tail))
    rc, out, state = _run_shell(shell + " list --json", timeout, host)
    if state != "ok" or rc != 0:
        return {"tasks": [], "events": [], "serve": {}, "runs": []}, "unobserved", f"queue rc={rc} state={state}"
    try:
        rows = json.loads(out)
    except Exception as exc:
        return {"tasks": [], "events": [], "serve": {}, "runs": []}, "invalid", type(exc).__name__
    if not isinstance(rows, list):
        return {"tasks": [], "events": [], "serve": {}, "runs": []}, "invalid", "non-list rows"
    tasks = [t for t in (_clean_task(r) for r in rows if isinstance(r, dict)) if t]
    events, ev_state, skipped, truncated = read_jsonl(
        QUEUE_STATE / "metrics.jsonl", tail=limits.get("max_records"),
        max_bytes=limits.get("max_bytes"))
    events = [e for e in events if isinstance(e, dict) and e.get("ev") in ("dispatch", "done")]
    rc, out, sstate = _run_shell(shell + " status --json", timeout, host)
    serve = {}
    if sstate == "ok" and rc == 0:
        try:
            serve = json.loads(out)
        except Exception:
            serve = {}
    runs = []
    runs_dir = QUEUE_STATE / "runs"
    try:
        entries = sorted(runs_dir.iterdir(), key=lambda p: p.stat().st_mtime, reverse=True)
    except OSError:
        entries = []
    for p in entries[:64]:
        if not p.is_file():
            continue
        try:
            st = p.stat()
        except OSError:
            continue
        runs.append({"ref": _fingerprint({"run": p.name}), "name": p.name,
                     "bytes": st.st_size, "mtime": st.st_mtime})
    detail = {"metrics_state": ev_state, "metrics_skipped": skipped,
              "metrics_truncated": truncated, "serve_state": sstate}
    return {"tasks": tasks, "events": events, "serve": serve, "runs": runs}, "ok", detail


def collect_dream_state(now):
    """Dream orchestrator state: per-day counts, wake-ledger supervision
    passes, policy overlays, falsify claims, arming."""
    rows = {"state": {}, "wake": [], "overlays": [], "claims": [], "armed": None}
    st, state, _ = _state(DREAM_DIR / "state.json")
    rows["state"] = st if state == "ok" and isinstance(st, dict) else {}
    wake, wstate, skipped, trunc = read_jsonl(DREAM_DIR / "wake-ledger.jsonl", tail=500)
    rows["wake"] = [w for w in wake if isinstance(w, dict)]
    fb, fbstate, _ = _state(DREAM_DIR / "audit-feedback.json")
    rows["feedback_state"] = fbstate
    pol = DREAM_DIR / "policy"
    ov, ostate, _ = _state(pol / "overlays.json")
    rows["overlays"] = ov if ostate == "ok" and isinstance(ov, list) else []
    cl, cstate, _, _ = read_jsonl(pol / "claims.jsonl", tail=400)
    rows["claims"] = [c for c in cl if isinstance(c, dict)]
    rows["wake_state"], rows["claims_state"] = wstate, cstate
    rows["overlays_state"] = ostate
    rows["armed"] = (DREAM_CFG.parent / "enabled").exists()
    detail = {"wake_skipped": skipped, "wake_truncated": trunc}
    ok_states = {"state": state, "wake": wstate, "overlays": ostate, "claims": cstate}
    overall = "ok" if all(v in ("ok", "absent") for v in ok_states.values()) else "stale"
    return rows, overall, detail


def collect_dreamer_ledgers(now):
    """The dreamer side ledgers: dispatch audit rows and terminal audit
    verdicts (verdicts/checks are metadata; no transcript bodies)."""
    dispatch, dstate, dskip, dtrunc = read_jsonl(DREAMER_STATE / "dream-ledger.jsonl", tail=800)
    audit, astate, askip, atrunc = read_jsonl(DREAMER_STATE / "audit-ledger.jsonl", tail=800)
    rows = {
        "dispatch": [r for r in dispatch if isinstance(r, dict)],
        "audit": [],
        "dispatch_state": dstate, "audit_state": astate,
    }
    for r in audit:
        if not isinstance(r, dict):
            continue
        keep = {k: r.get(k) for k in ("id", "route", "status", "terminal",
                                      "repair_ref", "verdict", "audited_at",
                                      "model_expected", "model_found",
                                      "review_id")}
        checks = r.get("checks")
        if isinstance(checks, dict):
            keep["checks"] = {k: checks.get(k) for k in sorted(checks) if isinstance(checks.get(k), (bool, type(None)))}
        issues = r.get("issues")
        keep["issue_count"] = len(issues) if isinstance(issues, list) else None
        keep["issue_codes"] = [model.text(i, 80) for i in (issues or [])[:5]]
        rows["audit"].append(keep)
    detail = {"dispatch_skipped": dskip, "audit_skipped": askip,
              "truncated": dtrunc or atrunc}
    if dstate in ("ok", "absent") and astate in ("ok", "absent"):
        overall = "ok"
    else:
        overall = "stale"
    return rows, overall, detail


def collect_heartbeat(now, path=None):
    """The observation-only heartbeat state. Quota planes carry their native
    units; unreadable funding readers stay QUOTA_UNKNOWN — never zeros."""
    data, state, detail = _state(path if path is not None else HEARTBEAT_STATE)
    if state != "ok":
        return {"planes": [], "findings": [], "as_of": None}, state, detail or "heartbeat state absent"
    planes, findings = [], []
    for pid, row in ((data.get("planes") or {}).items() if isinstance(data.get("planes"), dict) else []):
        obs = row.get("obs") if isinstance(row, dict) else None
        quality = row.get("quality", "unknown") if isinstance(row, dict) else "unknown"
        entry = {"plane": pid, "quality": quality,
                 "observed_at": model.epoch(row.get("observed_at")) if isinstance(row, dict) else None,
                 "error": model.text(row.get("error"), 120) if isinstance(row, dict) else None}
        if isinstance(obs, dict):
            grants = obs.get("grants")
            if isinstance(grants, list):
                units = []
                for g in grants:
                    if not isinstance(g, dict):
                        continue
                    units.append({"id": model.text(g.get("id"), 120),
                                  "grant_units": g.get("grant_units"),
                                  "remaining": g.get("remaining"),
                                  "status": model.text(g.get("status"), 20),
                                  "ends_at": model.epoch(g.get("ends_at")),
                                  "effective_at": model.epoch(g.get("effective_at"))})
                entry["grants"] = units
            windows = obs.get("windows")
            if isinstance(windows, list):
                entry["windows"] = [
                    {k: w.get(k) for k in ("remaining", "usage", "number", "nextResetTime")
                     if not isinstance(w, dict) or k in w}
                    for w in windows[:6] if isinstance(w, dict)]
            for k in ("available", "note"):
                if obs.get(k) is not None:
                    entry[k] = model.text(obs[k], 120) if isinstance(obs[k], str) else obs[k]
        planes.append(entry)
    for f in data.get("findings") or []:
        if isinstance(f, dict):
            findings.append({k: f.get(k) for k in ("id", "kind", "domain", "count",
                                                   "active", "alarmed", "detail",
                                                   "first_seen", "last_seen", "source")})
    return {"planes": planes, "findings": findings,
            "as_of": model.epoch(data.get("as_of"))}, "ok", ""


_UNIT_SAFE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9@_.:-]*$")


def _systemctl(argv, timeout):
    if not shutil.which("systemctl"):
        return None, "absent"
    rc, out, state = _run(["systemctl", "--user", *argv], timeout)
    if state != "ok" or rc != 0:
        return None, "unobserved"
    return out, "ok"


def collect_services(units, now, limits=None):
    """User-unit placement and resource ownership. Units come from the
    standing supervisor/watcher config or enrollment; unknown units are not
    invented, and systemd absence is reported as absent. Template names
    (unit@.service) resolve through list-units, never invented instances."""
    limits = limits or {}
    timeout = min(int(limits.get("read_timeout_seconds", 10)), 30)
    names = [u for u in (units or []) if isinstance(u, str) and _UNIT_SAFE.match(u)]
    if not names:
        return [], "absent", "no enrolled or standing units"
    out, state = _systemctl(["list-units", "--all", "--no-legend", "--plain", *names], timeout)
    found = []
    dynamic = []
    if state == "ok" and out is not None:
        for line in out.splitlines():
            cols = line.split()
            # UNIT is the first column; descriptions vary in length, so no
            # fixed offset from the end is safe.
            if cols and re.fullmatch(r"[A-Za-z0-9@_.:-]+\.(?:service|timer|target|socket)", cols[0]):
                found.append(cols[0])
    # Dynamic instance roster: template units (name@.service) carry a live
    # population of instantiated units (name@N.service) that a literal
    # expected-name list can never name. Discover them; never invent an
    # instance systemd did not report.
    templates = [u for u in names if u.endswith("@.service")]
    if templates and state == "ok":
        pats = [t.replace("@.service", "@*") for t in templates]
        outi, istate = _systemctl(["list-units", "--all", "--no-legend", "--plain", *pats], timeout)
        if istate == "ok" and outi is not None:
            for line in outi.splitlines():
                cols = line.split()
                if cols and re.fullmatch(r"[A-Za-z0-9@_.:-]+@[^.]+\.service", cols[0]) \
                        and cols[0] not in found and cols[0] not in templates:
                    dynamic.append(cols[0])
    roster = {"expected": names, "observed": found + dynamic,
              "expected_not_found": [u for u in names if u not in found and u not in dynamic],
              "dynamic_instances": dynamic,
              "dynamic_discovery": "systemd list-units template glob" if templates and state == "ok" else None}
    if not found and not dynamic:
        detail = dict(roster, note="no matching user units observed for the expected roster")
        return [], ("absent" if state == "ok" else state), detail
    detail = dict(roster)
    props = ["Id,ActiveState,SubState,MainPID,ExecMainStartTimestamp,NRestarts,Description,LoadState"]
    show_names = found + dynamic
    out2, state2 = _systemctl(["show", *show_names, "--no-pager", "--property", props[0]], timeout)
    if state2 != "ok":
        return [], state2, "systemctl show unavailable"
    rows = []
    block = {}
    for line in out2.splitlines():
        if not line.strip():
            if block:
                rows.append(block)
                block = {}
            continue
        k, _, v = line.partition("=")
        if k:
            block[k] = v
    if block:
        rows.append(block)
    services = []
    for b in rows:
        uid = b.get("Id", "")
        if not uid:
            continue
        services.append({
            "id": "service:local:" + uid, "unit": uid, "host": "local",
            "active": b.get("ActiveState") or "unknown",
            "sub": b.get("SubState") or "unknown",
            "load_state": b.get("LoadState") or "unknown",
            "pid": int(b["MainPID"]) if str(b.get("MainPID", "")).isdigit() and int(b["MainPID"]) else None,
            "since": model.epoch(b.get("ExecMainStartTimestamp")),
            "restarts": int(b["NRestarts"]) if str(b.get("NRestarts", "")).isdigit() else None,
            "description": model.text(b.get("Description"), 120),
            "cpu_seconds": None, "rss_bytes": None,
        })
    for s in services:
        pid = s["pid"]
        if pid and Path(f"/proc/{pid}").exists():
            try:
                fields = (Path(f"/proc/{pid}/stat").read_text().rsplit(") ", 1)[1]).split()
                utime, stime = int(fields[11]), int(fields[12])
                s["cpu_seconds"] = round((utime + stime) / os.sysconf("SC_CLK_TCK"), 2)
                s["rss_bytes"] = int(fields[21]) * os.sysconf("SC_PAGE_SIZE")
            except (OSError, IndexError, ValueError):
                pass
    return services, "ok", detail


def collect_timers(patterns, now, limits=None):
    """User timers as schedule rows: next due, last run, unit, result."""
    limits = limits or {}
    timeout = min(int(limits.get("read_timeout_seconds", 10)), 30)
    out, state = _systemctl(["list-timers", "--all", "--no-pager", "--no-legend"], timeout)
    if state != "ok":
        return [], state, "systemctl list-timers unavailable"
    schedules = []
    for line in out.splitlines():
        parts = line.split()
        # list-timers columns are variable width (NEXT alone is 5 tokens);
        # only the right side is stable: UNIT and ACTIVATES.
        if len(parts) < 3 or not parts[-2].endswith((".timer", ".service")):
            continue
        unit = parts[-2]
        if patterns and not any(p in unit for p in patterns):
            continue
        schedules.append({"id": "schedule:local:" + unit, "unit": unit, "host": "local",
                          "kind": "timer", "activates": parts[-1],
                          "next_text": model.text(" ".join(parts[:5]), 40)})
    return schedules, "ok", ""


def collect_maintenance(now):
    """Deterministic maintenance passes and bounded supervisor incidents."""
    sup, sstate, sdetail = _state(SWARM_STATE / "supervisor.json")
    incidents, istate, iskip, itrunc = read_jsonl(
        SWARM_STATE / "control/incidents.jsonl", tail=200)
    rows = {"pass": sup if sstate == "ok" and isinstance(sup, dict) else {},
            "incidents": [i for i in incidents if isinstance(i, dict)],
            "pass_state": sstate, "incidents_state": istate,
            "incidents_skipped": iskip}
    overall = "ok" if sstate in ("ok", "absent") and istate in ("ok", "absent") else "stale"
    return rows, overall, sdetail or ""


def collect_dmesg(now, limits=None):
    """Bounded kernel maintenance events (OOM/segfault/hang/thermal). The
    kernel ring is read-only and retained here as evidence rows with a
    content digest so pruning the ring never destroys the observation. On
    hosts where the ring is unreadable the fact is reported, not guessed."""
    limits = limits or {}
    timeout = min(int(limits.get("read_timeout_seconds", 10)), 30)
    if not shutil.which("dmesg"):
        return [], "absent", "dmesg not installed"
    rc, out, state = _run(["dmesg", "--time-format", "iso", "--level", "err,warn"], timeout)
    if state != "ok":
        return [], "unobserved", "dmesg read refused (state=%s)" % state
    if rc != 0:
        return [], "unobserved", "dmesg rc=%d (ring restricted)" % rc
    rows, seen = [], set()
    for line in out.splitlines()[-2000:]:
        if not _DMESG_FOCUS.search(line):
            continue
        fp = hashlib.sha256(line.strip().encode()).hexdigest()
        if fp in seen:
            continue
        seen.add(fp)
        ts = None
        head, _, body = line.partition(" ")
        try:
            ts = datetime.fromisoformat(head.strip("[]")).timestamp()
        except Exception:
            pass
        rows.append({"id": "event:dmesg:" + fp[:24], "subsystem": "kernel",
                     "severity": "error" if re.search(r"(?i)oom|segfault|i/o error", line) else "warning",
                     "at": ts, "captured_at": now,
                     "summary": model.text(body if body else line, 160),
                     "evidence_sha256": fp})
        if len(rows) >= limits.get("max_records", 2000):
            break
    return rows, "ok", ""


def plane_fresh(entry, now, ttl):
    """Freshness of one heartbeat plane against ITS OWN observation time.
    Re-reading the source file cannot freshen an old observation: only the
    plane's recorded observed_at decides current vs stale vs unknown."""
    obs = entry.get("observed_at") if isinstance(entry, dict) else None
    if not isinstance(obs, (int, float)) or isinstance(obs, bool):
        return "unknown"
    if obs > now + model.CLOCK_SKEW_SECONDS:
        return "unknown"  # source clock ahead: not certifiable as current
    return "stale" if (now - obs) > ttl else "current"


def collect_pressure(now, heartbeat_ttl=None, heartbeat_path=None):
    """Native quota/execution capacity: the heartbeat's verified planes in
    their own units, queue depth, and orchestrator backlog. A plane that is
    stale or untimeable is demoted here — its numbers remain on the row as
    historical observations but never present as current verified capacity,
    and an unreadable funding reader stays an unknown reader, never zeros."""
    hb_ttl = heartbeat_ttl if isinstance(heartbeat_ttl, (int, float)) and heartbeat_ttl > 0 else 60
    hb, hb_state, hb_detail = collect_heartbeat(now, heartbeat_path)
    rows = []
    for plane in hb["planes"]:
        fresh = plane_fresh(plane, now, hb_ttl)
        # Only a current observation from a verified reader is current
        # verified capacity. Failed/lastverified/unverified reader states,
        # stale observations and untimeable observations are all demoted;
        # the numbers stay on the row as historical facts.
        if fresh == "stale":
            quality = "stale"
        elif fresh == "unknown" or plane["quality"] != "verified":
            quality = "unknown"
        else:
            quality = "verified"
        entry = {"kind": "funding", "plane": plane["plane"], "host": "local",
                 "quality": quality, "plane_freshness": fresh,
                 "reader_quality": plane["quality"],
                 "grants": plane.get("grants", []),
                 "observed_at": plane.get("observed_at"),
                 "error": plane.get("error"),
                 "unknown_reader": quality != "verified"}
        if plane.get("available") is not None:
            entry["available"] = plane["available"]
        rows.append(entry)
    q, qstate, _ = collect_queue(now)
    counts = {}
    for r in ((q.get("serve") or {}).get("routes") or {}).values():
        for st, n in (r.get("counts") or {}).items():
            counts[st] = counts.get(st, 0) + n
    rows.append({"kind": "execution", "plane": "queue", "host": "local",
                 "quality": "verified" if qstate == "ok" else "unavailable",
                 "counts": counts, "serve": (q.get("serve") or {}).get("serve"),
                 "unknown_reader": qstate != "ok"})
    hb_as_of = hb.get("as_of")
    hb_fresh = plane_fresh({"observed_at": hb_as_of}, now, hb_ttl)
    overall = "ok" if (hb_state == "ok" and qstate == "ok"
                       and hb_fresh == "current") else "stale"
    return rows, overall, {
        "heartbeat": hb_state, "queue": qstate, "heartbeat_detail": hb_detail,
        "heartbeat_as_of": hb_as_of, "heartbeat_freshness": hb_fresh}


def _ledger_path_dirs():
    """Candidate dirs holding swarm_ledger.py across layouts: the repo tree
    (<root>/tools/swarmharness) and a staged kernel payload (share/ygg/kernel
    with swarmharness/ or a flat introspection/ layout)."""
    here = Path(__file__).resolve()
    return [here.parents[1], here.parents[1] / "swarmharness", here.parents[2]]


def collect_commitments(state_dir=None, now=None):
    """The config commitment ledger (journal + verdicts), via swarm_ledger.
    Predicates (endorsement verification) and revision/authority ride each
    row; this is the human-readable accountability view. Composer-notes and
    conditional-plan areas are inventoried the same way when the ledger
    carries them: a conditional row is PRESENTED with its predicate,
    deadline, priority, source and authority/timing — the read adapter never
    executes a conditional plan and never grants it spawn or repair
    authority. A hardcoded schedule is only claimed where the row itself is
    programmatic; prose intent stays prose."""
    rows = []
    extras = {"areas_inventoried": [], "absent_areas": []}
    state = str(state_dir or SWARM_STATE)
    import sys as _sys
    for cand in _ledger_path_dirs():
        if (cand / "swarm_ledger.py").exists() and str(cand) not in _sys.path:
            _sys.path.insert(0, str(cand))
    try:
        import swarm_ledger as sl
    except Exception as exc:
        return [], "absent", type(exc).__name__
    try:
        journal = sl.journal_records(state)
    except Exception:
        journal = []
    for rec in journal:
        rows.append({"id": "commit:config:" + str(rec.get("change_id", ""))[:40],
                     "area": rec.get("area"), "revision": rec.get("revision"),
                     "epoch": rec.get("authority_epoch"),
                     "principal": model.text(rec.get("principal"), 60),
                     "authority": model.text(rec.get("authority_ref"), 24) or "none",
                     "change_id": rec.get("change_id"),
                     "event_hash": rec.get("event_hash"),
                     "parent_hash": rec.get("parent_hash")})
    # Composer-notes and conditional-plan areas: inventory whatever journal
    # areas the local ledger actually holds; an area that does not exist is
    # absent, never folded into the config area.
    for area in ("composer-notes", "conditional", "conditional-ledger"):
        try:
            recs = sl.journal_records(state, area)
        except Exception:
            recs = None
        if not recs:
            extras["absent_areas"].append(area)
            continue
        extras["areas_inventoried"].append(area)
        for rec in recs:
            if not isinstance(rec, dict):
                continue
            programmatic = isinstance(rec.get("cron"), str) or \
                isinstance(rec.get("schedule"), str)
            rows.append({
                "id": ("commit:%s:%s" % (area, str(rec.get("change_id")
                       or rec.get("plan_id") or rec.get("id") or "")[:40])),
                "area": area, "kind": "conditional-plan" if area.startswith("conditional")
                                     else "composer-note",
                "revision": rec.get("revision"),
                "principal": model.text(rec.get("principal"), 60),
                "source": model.text(rec.get("source"), 80) or None,
                "plan_text": model.text(rec.get("plan") or rec.get("note"), 160) or None,
                "predicate": model.text(rec.get("predicate"), 120) or None,
                "deadline": model.epoch(rec.get("deadline")),
                "priority": model.text(rec.get("priority"), 20) or None,
                "authority": model.text(rec.get("authority_ref")
                                        or rec.get("authority"), 24) or "none",
                "timing": model.text(rec.get("timing"), 40) or None,
                "programmatic": programmatic,
                "execution": "presentation-only (never executed by the read adapter)"})
    ok, bad = (True, None)
    try:
        ok, bad = sl.verify_ancestry(state)
    except Exception:
        ok = False
    # The journal rows' own recorded time is the source observation clock for
    # coverage; the reader's clock is never substituted for it.
    extras["observed_at"] = max(
        (t for rec in journal if isinstance(rec, dict)
         for k in ("at", "recorded_at", "timestamp", "recorded_utc", "when")
         for t in [model.epoch(rec.get(k))] if t),
        default=None)
    return rows, ("ok" if ok else "conflicting"), extras if ok else \
        ("chain broken at %s" % bad)


def parse_tasks_ledger(path=None):
    """Bounded task-ledger headline counts (open/claimed) via the heartbeat's
    parser when importable; absence is a normal local state."""
    p = Path(path or LEDGER_PATH).expanduser()
    try:
        observed_at = p.stat().st_mtime
        text = p.read_text(errors="replace")
    except OSError:
        return None
    open_n = len(re.findall(r"^\#{2,3}\s*T-\d+.*?\n.*?status:\s*open", text, re.M))
    claimed_n = len(re.findall(r"^\#{2,3}\s*T-\d+.*?\n.*?status:\s*claimed", text, re.M))
    # The ledger file's own write time is the source observation clock; the
    # parse moment is the reader's clock and must never stand in for it.
    return {"path": str(p), "open": open_n, "claimed": claimed_n,
            "observed_at": observed_at}


_ROW_KEYS = ("id", "kind", "number", "claim", "harness", "host", "status",
             "state", "pid", "campaign", "role", "updated_at", "claimed_at",
             "released_at", "note")


COMPOSER_STATE = _HOME / ".yggterm/swarm/composer"
REVIEWS_DIR = _HOME / ".yggterm/swarm/reviews"
MEMORY_LOCK_DIR = _HOME / ".yggterm/swarm/composer/kernel"

# Closed observation fields of a composer OBS row. Anything else in the
# source row (bodies, notes, provider text) never enters a frame.
_COMPOSER_OBS_FIELDS = ("at", "dedupe_key", "detector_id", "detector_version",
                        "fingerprint", "generation", "lineage_id", "severity",
                        "source_kind", "span", "status")


def collect_composer(now, limits=None, state_dir=None):
    """Composer scan state: sealed OBS observations, read cursors, scanned
    lineages, outbox presence. Repeated observations of the same dedupe_key
    collapse into ONE row with a repeat count — a replayed detection is the
    same vote, never a second confirmation. Cursor sources are fingerprinted
    (label + digest), never stored as raw filesystem paths."""
    limits = limits or {}
    base = Path(state_dir or COMPOSER_STATE).expanduser()
    obs, ostate, oskip, otrunc = read_jsonl(
        base / "observations.jsonl", tail=limits.get("max_records"),
        max_bytes=limits.get("max_bytes"))
    merged = {}
    for r in obs:
        if not isinstance(r, dict):
            continue
        key = r.get("dedupe_key") or _fingerprint(r)
        at = model.epoch(r.get("at"))
        cur = merged.get(key)
        if cur is None:
            row = {"id": "composer-obs:" + str(key)[:40], "row_type": "observation"}
            row.update({f: r.get(f) for f in _COMPOSER_OBS_FIELDS})
            row.update(first_at=at, last_at=at, repeat_count=1,
                       source_observed_at=at)
            merged[key] = row
        else:
            cur["repeat_count"] += 1
            if at and (cur["last_at"] is None or at > cur["last_at"]):
                cur.update(last_at=at, source_observed_at=at)
            if at and (cur["first_at"] is None or at < cur["first_at"]):
                cur["first_at"] = at
    rows = sorted(merged.values(), key=lambda r: r.get("last_at") or 0,
                  reverse=True)
    cursors, cstate, _ = _state(base / "cursors.json", max_bytes=262144)
    cursor_rows = []
    if cstate == "ok" and isinstance(cursors, dict):
        for k, v in sorted(cursors.items())[:100]:
            if not isinstance(v, dict):
                continue
            cursor_rows.append({
                "id": "composer-cursor:" + _fingerprint({"source": k})[:24],
                "row_type": "cursor",
                "source_ref": _fingerprint({"source": k}),
                "source_label": model.text(k.rsplit("/", 1)[-1], 60),
                "cursor": v.get("cursor") if isinstance(v.get("cursor"), int) and v.get("cursor") >= 0 else None,
                "generation": model.text(v.get("generation"), 60) or None,
                "lineage_id": model.text(v.get("lineage_id"), 40) or None,
                "pending_fps": len(v.get("pending_fps") or [])})
    scanned, sstate, _ = _state(base / "scanned.json", max_bytes=262144)
    scanned_n = len(scanned) if sstate == "ok" and isinstance(scanned, list) else None
    outbox = base / "outbox"
    try:
        outbox_entries = len([p for p in outbox.iterdir()][:200]) if outbox.is_dir() else None
        outbox_state = "ok" if outbox_entries is not None else "absent"
    except OSError:
        outbox_entries, outbox_state = None, "unreadable"
    detail = {"state_dir": str(base), "observations_state": ostate,
              "observations_skipped": oskip, "observations_truncated": otrunc,
              "cursors_state": cstate, "scanned_state": sstate,
              "scanned_lineages": scanned_n, "outbox_state": outbox_state,
              "outbox_entries": outbox_entries}
    sub = {"observations": ostate, "cursors": cstate, "scanned": sstate,
           "outbox": outbox_state}
    if all(v == "absent" for v in sub.values()):
        # a fully absent composer state is a material unknown about the
        # composer, never a verified-empty healthy scan
        overall = "absent"
    elif any(v in ("invalid", "unreadable") for v in sub.values()):
        overall = "stale"
    else:
        overall = ostate if ostate != "ok" else "ok"
    rows.extend(cursor_rows)
    rows.append({"id": "composer-state:1", "row_type": "state", **detail})
    return rows, overall, detail


def collect_reviews(now, limits=None, reviews_dir=None):
    """One row per sessionReview chain — chain-level truth, never a tally of
    repeated verdict rows (a cached repeat vote is not evidence). The
    resolution outcome and editor binding are recorded as claims; HMAC
    editor-verification is a query-time receipt check (_receipt_state), not
    something a chain row can assert about itself."""
    base = Path(reviews_dir or os.environ.get("YGG_INSPECT_REVIEWS",
                                              str(REVIEWS_DIR))).expanduser()
    try:
        entries = sorted(base.glob("*.jsonl"))
    except OSError:
        return [], "unreadable", {"reviews_dir": str(base)}
    if not entries:
        return [], "absent", {"reviews_dir": str(base)}
    rows, skipped = [], 0
    for p in entries[:100]:
        chain, cstate, _, _ = read_jsonl(p, tail=300, max_bytes=262144)
        if cstate != "ok":
            skipped += 1
            continue
        kinds = {}
        for r in chain:
            k = r.get("kind", "?")
            kinds[k] = kinds.get(k, 0) + 1
        res = [r for r in chain if r.get("kind") == "resolution"]
        verdicts = [r for r in chain if r.get("kind") == "editor_verdict"]
        snaps = [r for r in chain if r.get("kind") == "snapshot_staged"]
        opened = chain[0] if chain and isinstance(chain[0], dict) else {}
        rid = model.text(opened.get("review_id") or p.stem, 80)
        editor = verdicts[-1].get("editor") if verdicts else None
        rows.append({"id": "review:" + rid, "review_id": rid,
                     "reviewer_scope": "inspection-ledger",
                     "review_state": "unreviewed" if not verdicts else
                                     "verdict-claimed",
                     "row_count": len(chain),
                     "kinds": {k: v for k, v in sorted(kinds.items())},
                     "resolution": model.text(res[-1].get("outcome"), 24)
                                   if res else None,
                     "resolution_at": model.epoch(res[-1].get("at"))
                                      if res else None,
                     "editor_bound": model.text(
                         editor.get("name") if isinstance(editor, dict)
                         else editor, 60) or None,
                     "subject_digest": model.text(snaps[-1].get("subject_digest"), 80)
                                       if snaps else None,
                     "tier": model.text(opened.get("tier"), 20) or None,
                     "chain_state": cstate})
    return rows, ("ok" if rows or not skipped else "invalid"), {
        "reviews_dir": str(base), "entries": len(entries), "skipped": skipped}


def collect_memory_lock(now, limits=None, kernel_dir=None):
    """The sealed composer kernel the memory scans lock against: per-module
    presence, size and digest. An absent or empty lock dir is a material
    unknown, not a healthy zero; byte reads are bounded."""
    base = Path(kernel_dir or os.environ.get("YGG_MEMORY_LOCK_DIR",
                                             str(MEMORY_LOCK_DIR))).expanduser()
    limit = int((limits or {}).get("max_bytes") or DEFAULT_LIMITS["max_bytes"])
    try:
        entries = sorted(p for p in base.iterdir()
                         if p.is_file() and p.suffix in (".py", ".json")
                         and p.stat().st_size <= limit)
    except FileNotFoundError:
        return [], "absent", {"memory_lock_dir": str(base)}
    except OSError:
        return [], "unreadable", {"memory_lock_dir": str(base)}
    rows = []
    for p in entries[:50]:
        try:
            st = p.stat()
            fp = hashlib.sha256(p.read_bytes()).hexdigest()[:16]
        except OSError:
            continue
        rows.append({"id": "memlock:" + _fingerprint({"module": p.name})[:24],
                     "row_type": "sealed-module", "module": p.name,
                     "bytes": st.st_size, "sha256_16": fp,
                     "mtime": st.st_mtime, "source_observed_at": st.st_mtime})
    oversized = len([q for q in base.iterdir()
                     if q.is_file() and q.stat().st_size > limit])
    return rows, ("ok" if rows else "invalid"), {
        "memory_lock_dir": str(base), "entries": len(entries),
        "oversized_skipped": oversized}


def collect_workers(now, registry_dir=None):
    """Manually owned worker rows from the yggterm row registry
    (an explicitly enrolled registry dir, else ``${YGGTERM_HOME:-~/.yggterm}/rows``).
    These are dynamically discovered
    manual owner workers — a separate compute population from the
    autonomous queue-admitted dream stream. An undiscoverable or unreadable
    registry is reported as unknown worker coverage, never as zero workers,
    and never folded into the queue count."""
    if registry_dir:
        base = Path(registry_dir).expanduser()
    else:
        base = Path(os.environ.get("YGGTERM_HOME",
                                   str(_HOME / ".yggterm"))).expanduser() / "rows"
    rows = []
    try:
        entries = sorted(base.glob("*.json"))
    except OSError:
        return rows, "unreadable", {"registry": str(base)}
    if not entries:
        return rows, "absent", {"registry": str(base)}
    skipped = 0
    for p in entries[:200]:
        row, state, _ = _state(p, max_bytes=65536)
        if state != "ok" or not isinstance(row, dict):
            skipped += 1
            continue
        out = {"id": "worker:" + model.text(p.stem, 80),
               "row_type": "worker", "origin": "manual-owner-worker",
               "registry": model.text(str(base), 120)}
        for k in _ROW_KEYS:
            if row.get(k) is not None:
                out[k] = model.text(row[k], 120) if isinstance(row[k], (str, list)) else row[k]
        rows.append(out)
    detail = {"registry": str(base), "skipped": skipped,
              "entries": len(entries),
              "newest_mtime": max((p.stat().st_mtime for p in entries[:200]
                                   if p.is_file()), default=None)}
    return rows, ("ok" if rows or not skipped else "invalid"), detail
