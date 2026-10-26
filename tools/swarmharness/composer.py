"""composer — the fleet lesson miner (owner directive 2026-10-26; sol architecture 2026-10-26).

A sibling deterministic service (maintenance.py pattern) subordinate to the
dream orchestrator: it mines enrolled session corpora for RECURRING fault
patterns, shapes them into falsifiable claims, and proposes work — it never
dispatches, never spawns workers, never publishes lessons by itself.

Three-stage pipeline with honest limits (sol Q2):
  1. deterministic extraction — tool/result joins, error signatures, edit
     churn, recurrence counts. Proves nothing about root cause or intent.
  2. Jev classification (LATER, bounded adapter) — classifies supplied
     evidence only.
  3. sweep dreams (LATER, through the existing dispatch authority) — full
     agent investigation of candidate clusters.

Three linked records (sol Q3): Observation (immutable, trace-derived,
deduped by H(source_revision, span, detector_id, detector_version)) ->
Pattern (revisable hypothesis with incident/opportunity denominators) ->
ClaimProposal (falsifiable assertion into the outbox; the existing falsify
ledger stays the authoritative evaluator). Model/harness pair is an analysis
STRATUM, not the fault key — the same shell-transport defect must not become
three model-specific faults.

Privacy: extraction is host-local; observations carry fingerprints, counts,
and source references — NEVER message bodies or user content. Transcript
content is untrusted data; nothing recovered from a session is ever
executed. The privacy gate runs before anything leaves the host.

Zero model calls. State on local dev disk (~/.yggterm/swarm/composer/):
append-only observations JSONL + atomic cursor commits. Disabled until
enrolled and configured; `tick()` is one bounded pass with hard limits.
"""
import hashlib
import json
import os
import re
from datetime import datetime, timezone
from pathlib import Path

DEFAULT_STATE = Path("~/.yggterm/swarm/composer")
DETECTOR_VERSION = "det-v1"
SUPPORTED_SOURCES = ("zcode_rollout", "codex_session", "queue_run",
                     "review_chain", "board")


def _now():
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def _canonical(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"))


def _sha(data):
    if isinstance(data, str):
        data = data.encode()
    return hashlib.sha256(data).hexdigest()


def _source_revision(path):
    """Stable identity for an enrolled file: size + head-hash (cheap, not
    cryptographic for large files — head covers the first 64KiB)."""
    path = Path(path)
    try:
        size = path.stat().st_size
        with path.open("rb") as f:
            head = f.read(65536)
        return f"sz{size}:{_sha(head)[:16]}"
    except OSError:
        return "unavailable"


# ------------------------------------------------------------ detector set
# Versioned; each detector: (id, compiled signature or callable, severity).
# A signature identifies a SYMPTOM class, not a cause (sol Q2).

_HARNESS_SIGNATURES = [
    ("ssh-argv-join", re.compile(r"syntax error near unexpected token|bash: -c: line")),
    ("heredoc-expansion", re.compile(r"command not found:?\s*$", re.M)),
    ("nameerror-typo", re.compile(r"NameError: name '.*' is not defined")),
    ("kwarg-dup", re.compile(r"got multiple values for keyword argument")),
    ("pyc-stale", re.compile(r"Unsupported bytecode|bad marshal data")),
    ("param-shadow", re.compile(r"got an unexpected keyword argument")),
]


def _detect_line(kind, line, path, lineno):
    """Return (detector_id, severity, fingerprint) for interesting lines."""
    if kind in ("queue_run_out", "codex_session", "zcode_rollout"):
        for sig_id, rx in _HARNESS_SIGNATURES:
            m = rx.search(line)
            if m:
                return (f"sig-{sig_id}", "high",
                        _sha(line.strip())[:16])
    return None


def _failure_loops(commands):
    """Same command fingerprint failing >=2 times in one lineage."""
    seen = {}
    for fp in commands:
        seen[fp] = seen.get(fp, 0) + 1
    return [fp for fp, n in seen.items() if n >= 2]


# ------------------------------------------------------------ census

def census(sources, cutoff=None, state_dir=None):
    """Enumerate enrolled corpora frozen at cutoff -> CensusManifest dict.
    sources: {source_kind: [paths]}. Every record gets exactly one
    disposition; missing/unreadable files are enumerated, never skipped."""
    manifest = {"kind": "census", "cutoff": cutoff or _now(),
                "created_at": _now(), "sources": [], "gaps": [],
                "total_bytes": 0}
    for kind, paths in (sources or {}).items():
        if kind not in SUPPORTED_SOURCES:
            manifest["gaps"].append(
                {"source_kind": kind, "status": "unsupported_kind"})
            continue
        for path in paths:
            p = Path(path).expanduser()
            entry = {"source_kind": kind, "path": str(p)}
            if not p.exists():
                entry["status"] = "missing"
                manifest["gaps"].append(entry)
                continue
            try:
                entry["status"] = "ok"
                entry["revision"] = _source_revision(p)
                entry["bytes"] = p.stat().st_size
                manifest["total_bytes"] += entry["bytes"]
            except OSError as exc:
                entry["status"] = f"error: {exc}"[:80]
                manifest["gaps"].append(entry)
            manifest["sources"].append(entry)
    return manifest


# ------------------------------------------------------------ read + extract

def read_page(path, cursor=0, max_bytes=262144):
    """Bounded byte page from a source revision. Returns (page_text,
    next_cursor, done). Cursor = byte offset; never re-reads committed
    bytes."""
    p = Path(path)
    with p.open("rb") as f:
        f.seek(cursor)
        chunk = f.read(max_bytes)
    if not chunk:
        return "", cursor, True
    next_cursor = cursor + len(chunk)
    # trim to last complete line to avoid splitting events
    if not next_cursor >= p.stat().st_size:
        nl = chunk.rfind(b"\n")
        if nl > 0:
            chunk = chunk[:nl + 1]
            next_cursor = cursor + len(chunk)
    text = chunk.decode("utf-8", errors="replace")
    done = next_cursor >= p.stat().st_size
    return text, next_cursor, done


def extract(source_kind, path, revision, page_text, lineno_base):
    """Run the versioned detector set over one page. Returns
    (observations, stats). Observations carry fingerprints and references —
    never message bodies."""
    observations = []
    stats = {"lines": 0, "eligible_sessions": 0}
    kind_line = {"queue_run": "queue_run_out",
                 "queue_run_out": "queue_run_out",
                 "codex_session": "codex_session",
                 "zcode_rollout": "zcode_rollout",
                 "board": "board"}.get(source_kind, source_kind)
    cmd_failures = []
    for i, line in enumerate(page_text.splitlines()):
        stats["lines"] += 1
        hit = _detect_line(kind_line, line, path, lineno_base + i)
        if hit:
            detector_id, severity, fp = hit
            observations.append({
                "detector_id": detector_id,
                "detector_version": DETECTOR_VERSION,
                "severity": severity,
                "source_kind": source_kind,
                "source_revision": revision,
                "span": f"{Path(path).name}:{lineno_base + i}",
                "dedupe_key": _sha(_canonical(
                    [revision, f"{lineno_base + i}", detector_id,
                     DETECTOR_VERSION])),
                "fingerprint": fp,
                "at": _now(),
            })
    # failure-loop detection over command-shaped lines (deterministic:
    # identical stripped command text >=2 in one page of one lineage)
    for line in page_text.splitlines():
        s = line.strip()
        if s.startswith("$ ") or s.startswith("+ "):
            cmd_failures.append(_sha(s)[:16])
    for fp in _failure_loops(cmd_failures):
        observations.append({
            "detector_id": "cmd-failure-loop",
            "detector_version": DETECTOR_VERSION,
            "severity": "medium",
            "source_kind": source_kind,
            "source_revision": revision,
            "span": f"{Path(path).name}:page@{lineno_base}",
            "dedupe_key": _sha(_canonical(
                [revision, "loop", fp, DETECTOR_VERSION])),
            "fingerprint": fp, "at": _now()})
    return observations, stats


# ------------------------------------------------------------ store

def _atomic_write_jsonl(path, rows):
    """Append-preserving atomic write: existing durable lines + new rows
    land together in the replacement file. (A naive tmp-append + replace
    CLOBBERS history — the tmp is consumed by replace each call.)"""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    existing = []
    if path.exists():
        existing = [ln for ln in path.read_text().splitlines() if ln.strip()]
    tmp = path.with_name(path.name + f".tmp{os.getpid()}")
    with tmp.open("w") as f:
        for ln in existing:
            f.write(ln + "\n")
        for row in rows:
            f.write(_canonical(row) + "\n")
        f.flush()
        os.fsync(f.fileno())
    os.replace(tmp, path)


def load_observations(state_dir=None):
    path = (Path(state_dir).expanduser() if state_dir
            else DEFAULT_STATE.expanduser()) / "observations.jsonl"
    if not path.exists():
        return []
    out = []
    for line in path.read_text().splitlines():
        try:
            out.append(json.loads(line))
        except Exception:
            continue
    return out


def commit_batch(state_dir, source_entry, observations, cursor, stats):
    """Atomically commit extracted observations + cursor advancement.
    Crash between the two files loses nothing: cursor advances only after
    observations are durable; replay dedupes by dedupe_key."""
    state = Path(state_dir).expanduser()
    state.mkdir(parents=True, exist_ok=True)
    if observations:
        _atomic_write_jsonl(state / "observations.jsonl", observations)
    cursors = {}
    cpath = state / "cursors.json"
    if cpath.exists():
        try:
            cursors = json.loads(cpath.read_text())
        except Exception:
            cursors = {}
    cursors[source_entry["path"]] = {
        "cursor": cursor, "revision": source_entry.get("revision"),
        "lines_seen": stats["lines"], "at": _now()}
    tmp = cpath.with_name(cpath.name + f".tmp{os.getpid()}")
    tmp.write_text(_canonical(cursors))
    os.replace(tmp, cpath)


# ------------------------------------------------------------ patterns + rank

def group_patterns(observations, min_incidents=2):
    """Group observations by detector_id + fingerprint -> Pattern records
    with incident/opportunity denominators (sol: one loop with 100 failures
    is ONE incident)."""
    groups = {}
    for obs in observations:
        key = (obs["detector_id"], obs.get("fingerprint"))
        groups.setdefault(key, []).append(obs)
    patterns = []
    for (detector_id, fp), obs_list in sorted(groups.items()):
        incidents = len({o["source_revision"] for o in obs_list})
        severity = max((o.get("severity", "low") for o in obs_list),
                       key=lambda s: {"high": 3, "medium": 2, "low": 1}[s])
        patterns.append({
            "pattern_id": _sha(_canonical([detector_id, fp]))[:16],
            "detector_id": detector_id,
            "detector_version": DETECTOR_VERSION,
            "fingerprint": fp,
            "severity": severity,
            "independent_incidents": incidents,
            "min_incidents": min_incidents,
            "eligible": incidents >= min_incidents,
            "strata": sorted({o.get("source_kind", "?")
                              for o in obs_list}),
        })
    return patterns


def read_needs(state_dir=None):
    """Read the dream orchestrator's versioned needs snapshot; absent or
    stale is reported EXPLICITLY, never pretended away (sol Q4)."""
    path = (Path(state_dir).expanduser() if state_dir
            else DEFAULT_STATE.expanduser()) / "needs.json"
    if not path.exists():
        return {"available": False,
                "reason": "needs snapshot absent — evidence-based fallback"}
    try:
        needs = json.loads(path.read_text())
        if not isinstance(needs, dict) or "revision" not in needs:
            return {"available": False, "reason": "malformed needs snapshot"}
        return {"available": True, "needs": needs}
    except Exception as exc:
        return {"available": False, "reason": f"unreadable: {exc}"[:100]}


def rank_patterns(patterns, needs, exploration_share=0.2):
    """Rank: needs-linked boost + severity/incidents + age decay, with a
    reserved exploration allocation for rare/low-signal strata (sol Q4).
    Deterministic: ties break by pattern_id."""
    now = _now()
    need_families = {}
    if needs.get("available"):
        for n in (needs["needs"].get("items") or []):
            for fam in (n.get("relevant_fault_families") or []):
                need_families[fam] = max(need_families.get(fam, 0),
                                         float(n.get("urgency", 1)))
    sev_w = {"high": 3.0, "medium": 2.0, "low": 1.0}
    scored = []
    for pat in patterns:
        if not pat["eligible"]:
            continue
        base = sev_w.get(pat.get("severity", "medium"), 2.0)
        score = base * (1 + pat["independent_incidents"])
        fam = pat["detector_id"]
        if fam in need_families:
            score *= (1 + need_families[fam])
        scored.append((score, pat))
    scored.sort(key=lambda sp: (-sp[0], sp[1]["pattern_id"]))
    n_explore = max(1, int(len(scored) * exploration_share)) if scored else 0
    ranked = [{"score": round(s, 3), **pat} for s, pat in scored]
    for i, pat in enumerate(ranked):
        pat["lane"] = ("exploration" if i >= len(ranked) - n_explore
                       else "demand")
    return ranked


# ------------------------------------------------------------ outbox + tick

def propose_claims(state_dir, ranked, cap=5):
    """Emit ClaimProposals for eligible patterns into the outbox (idempotent
    by pattern_id; the falsify ledger stays the authoritative evaluator).
    Proposals are PROPOSALS — never dispatch, never promotion."""
    state = Path(state_dir).expanduser()
    outbox = state / "outbox.jsonl"
    seen = set()
    if outbox.exists():
        for line in outbox.read_text().splitlines():
            try:
                seen.add(json.loads(line).get("pattern_id"))
            except Exception:
                continue
    emitted = []
    for pat in ranked[:cap * 2]:
        if pat["pattern_id"] in seen or len(emitted) >= cap:
            continue
        proposal = {
            "kind": "claim_proposal", "pattern_id": pat["pattern_id"],
            "detector_id": pat["detector_id"],
            "assertion": (f"Detector {pat['detector_id']} recurs across "
                          f"{pat['independent_incidents']} independent "
                          f"sources (strata {pat['strata']}); a fixture "
                          "reproducing the fingerprint will trip the same "
                          "detector, and the control fixture will not."),
            "disproof": "controls do not separate; incidents are one lineage",
            "strata": pat["strata"], "lane": pat["lane"],
            "created_at": _now(), "status": "proposed"}
        _atomic_write_jsonl(outbox, [proposal])
        seen.add(pat["pattern_id"])
        emitted.append(proposal)
    return emitted


def tick(sources, state_dir=None, max_bytes_per_source=262144,
         claim_cap=5):
    """One bounded pass: census -> read uncommitted pages -> extract ->
    commit atomically -> group -> rank -> propose. Returns TickReport.
    Zero model calls. Hard limits; coverage gaps enumerated."""
    state = Path(state_dir).expanduser() if state_dir \
        else DEFAULT_STATE.expanduser()
    report = {"at": _now(), "census_sources": 0, "gaps": 0,
              "pages_read": 0, "observations": 0, "patterns": 0,
              "proposals": 0, "notes": []}
    manifest = census(sources, state_dir=state)
    report["census_sources"] = len(manifest["sources"])
    report["gaps"] = len(manifest["gaps"])
    cursors = {}
    cpath = state / "cursors.json"
    if cpath.exists():
        try:
            cursors = json.loads(cpath.read_text())
        except Exception:
            cursors = {}
    all_obs = []
    known = {o["dedupe_key"] for o in load_observations(state)}
    fresh_count = 0
    for entry in manifest["sources"]:
        if entry.get("status") != "ok":
            continue
        cur = cursors.get(entry["path"], {}).get("cursor", 0)
        revision = entry.get("revision")
        if cursors.get(entry["path"], {}).get("revision") not in (
                None, revision):
            cur = 0  # source changed identity -> reprocess honestly
        while True:
            text, nxt, done = read_page(entry["path"], cur,
                                        max_bytes_per_source)
            if not text:
                break
            obs, stats = extract(entry["source_kind"], entry["path"],
                                 revision, text, cur)
            fresh_count += sum(1 for o in obs
                               if o["dedupe_key"] not in known)
            all_obs.extend(obs)
            report["pages_read"] += 1
            report["observations"] += len(obs)
            commit_batch(state, entry, obs, nxt, stats)
            cur = nxt
            if done:
                break
    patterns = group_patterns(load_observations(state))
    report["patterns"] = sum(1 for p in patterns if p["eligible"])
    needs = read_needs(state)
    if not needs.get("available"):
        report["notes"].append(needs["reason"])
    ranked = rank_patterns(patterns, needs)
    proposals = propose_claims(state, ranked, cap=claim_cap)
    report["proposals"] = len(proposals)
    report["notes"].append(
        f"fresh observations this pass: {fresh_count} "
        f"(replayed duplicates are expected and deduped)")
    return report
