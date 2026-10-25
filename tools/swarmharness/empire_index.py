"""The P1 information system, v2: the empire index.

A compact, deterministic rollup of the swarm's state — config revision,
campaign registry, queue, findings, overlays, planes, intent pointers —
regenerated on every supervise pass. Consults (sol/gemini briefs) embed the
markdown as their BOUNDED view; the JSON snapshot is the machine-readable
twin from the same capture.

v2 (sol turn-5/6): every source reports ok | missing | unreadable | invalid
| stale — empty data is ok with count zero; the projection honors a hard
size budget with deterministic section limits and overflow counts; index
failure never stops containment (the caller treats build faults as T3).
"""
import json
from datetime import datetime, timezone
from pathlib import Path

MAX_BYTES = 12 * 1024
STALE_AFTER_H = 26  # sources older than this are stale (audit wire: 2 wakes)


def _load_json_state(path):
    """-> (obj|None, state) with state in ok|missing|unreadable|invalid|stale."""
    p = Path(str(path).replace("~", str(Path.home()))).expanduser()
    try:
        raw = p.read_text()
    except FileNotFoundError:
        return None, "missing"
    except (IsADirectoryError, PermissionError):
        return None, "unreadable"
    except OSError:
        return None, "unreadable"
    try:
        obj = json.loads(raw)
    except Exception:
        return None, "invalid"
    return obj, "ok"


def _age_hours(iso):
    try:
        dt = datetime.fromisoformat(str(iso).replace("Z", "+00:00"))
        return (datetime.now(timezone.utc) - dt).total_seconds() / 3600.0
    except Exception:
        return None


def _age_state(iso):
    h = _age_hours(iso)
    if h is None:
        return "unknown"
    return "stale" if h > STALE_AFTER_H else "ok"


def build(cfg_sw, floors, queue_rows, state_dir, out_path, now=None,
          registry_dir=None):
    """Assemble the index from the live sources. queue_rows: the queue's task
    rows (list of dicts) or None (unknown). Writes markdown + JSON snapshot;
    returns the markdown. Never raises on source faults — they are reported."""
    now = now or datetime.now().astimezone()
    sources: dict = {}

    def src(name, state, note=""):
        sources[name] = {"state": state, "note": note}
        return state

    lines = [f"# empire index — {now:%F %H:%M %Z}", ""]
    lines.append("Bounded view for swarm intelligence consults. Regenerated every "
                 "supervise pass; pointers below reach deeper evidence.")
    lines.append("")

    # ---- config revision (v2 identity; v1 configs have none)
    ident = (cfg_sw or {}).get("identity") or {}
    lines.append("## config")
    if ident:
        lines.append(f"- revision {ident.get('revision')} (epoch {ident.get('authority_epoch')}) "
                     f"hash {str(ident.get('parent_hash'))[:12]}… by {ident.get('proposed_by')}")
        lines.append(f"- reason: {ident.get('reason', '')}")
    else:
        lines.append("- v1 config (no identity chain) — phase-5 cutover pending")
    lines.append("")

    # ---- campaign registry (P1 v2)
    reg_dir = Path(str(registry_dir).replace("~", str(Path.home()))) if registry_dir else None
    lines.append("## campaigns (registry)")
    campaigns = []
    if reg_dir is None:
        reg_dir = Path.home() / ".yggterm" / "swarm" / "registry"
    try:
        cd = reg_dir / "campaign"
        if cd.is_dir():
            for p in sorted(cd.glob("*.json")):
                obj, st = _load_json_state(p)
                if st != "ok":
                    src(f"registry:{p.stem}", st)
                    continue
                campaigns.append(obj)
        if not campaigns:
            src("registry:campaign", "ok", "empty")
            lines.append("- none registered")
        for c in campaigns:
            blocked = len(c.get("blockers") or [])
            lines.append(f"- {c.get('id')}: {c.get('status')} — {str(c.get('objective'))[:80]}"
                         f"{' | BLOCKERS: %d' % blocked if blocked else ''}")
    except Exception as e:  # noqa: BLE001
        src("registry", "unreadable", repr(e)[:80])
        lines.append("- registry unreadable (reported, containment unaffected)")
    lines.append("")

    trips_total = 0
    try:
        td = reg_dir / "trip"
        trips = [p.stem for p in td.glob("*.json")] if td.is_dir() else []
        trips_total = len(trips)
        pd_ = reg_dir / "procedure"
        procs = [p.stem for p in pd_.glob("*.json")] if pd_.is_dir() else []
        lines.append(f"## procedure library — {trips_total} trips, {len(procs)} procedures")
        if trips_total:
            lines.append(f"- trips: {', '.join(sorted(trips)[:6])}{' …' if trips_total > 6 else ''}")
    except Exception:  # noqa: BLE001
        lines.append("## procedure library — unreadable")
    lines.append("")

    # ---- campaigns (supply namespaces; floors until phase-5 cutover)
    ns = [(n.get("ns"), n.get("per_day")) for n in ((floors or {}).get("namespaces") or [])]
    lines.append("## supply namespaces (floors, pre-cutover)")
    if ns:
        for name, per_day in ns:
            lines.append(f"- {name} (floor {per_day}/day)")
    else:
        lines.append("- none readable")
    lines.append("")

    # ---- queue state
    lines.append("## queue")
    if queue_rows is None:
        lines.append("- UNKNOWN (queue read failed) — containment only; novel decisions deferred")
    else:
        by = {}
        for r in queue_rows:
            by[(r.get("route"), r.get("status"))] = by.get((r.get("route"), r.get("status")), 0) + 1
        for (route, status), n in sorted(by.items()):
            lines.append(f"- {route} {status}: {n}")
    lines.append("")

    # ---- audit findings wire (with freshness)
    fb_path = str(((cfg_sw or {}).get("kernel") or {}).get("feedback",
                  "~/.yggterm/dream/audit-feedback.json"))
    fb, st = _load_json_state(fb_path)
    lines.append("## audit findings (wire)")
    if fb is None:
        src("audit-feedback", st)
        lines.append(f"- {st} (audit has never run or is unreadable)")
    else:
        st = _age_state(fb.get("generated_at"))
        src("audit-feedback", st if st != "unknown" else "ok",
            f"generated {fb.get('generated_at')}")
        lines.append(f"- generated {fb.get('generated_at')} [{st}], "
                     f"evaluator {fb.get('evaluator_version')}")
        for f in fb.get("findings", []):
            scope = f.get("scope") or {}
            lines.append(f"- {f['code']} [{f['status']}] route={scope.get('route')} "
                         f"family={scope.get('family') or '-'} alloc={f.get('allocation')}")
    lines.append("")

    # ---- policy overlays (freshness by expiry, not mtime)
    pol_dir = Path(str(((cfg_sw or {}).get("kernel") or {}).get("policy_dir",
                  "~/.yggterm/dream/policy")).replace("~", str(Path.home())))
    ov, st = _load_json_state(pol_dir / "overlays.json")
    lines.append("## policy overlays")
    if st != "ok" or not ov:
        src("overlays", st if st != "ok" else "ok", "empty" if st == "ok" else "")
        lines.append(f"- none active ({st})")
    else:
        src("overlays", "ok", f"{len(ov)} active")
        for o in ov:
            lines.append(f"- {o.get('type')} {o.get('route') or o.get('namespace')} "
                         f"until {o.get('expires_at')} ({o.get('reason_code')})")
    lines.append("")

    # ---- intelligence roles (v2 shape; v1 fallback)
    lines.append("## intelligence layer")
    roles = (cfg_sw or {}).get("roles") or {}
    intel = (cfg_sw or {}).get("intelligence") or {}
    if roles.get("editor") or roles.get("intelligence"):
        ed = roles.get("editor") or {}
        lines.append(f"- editor default: {ed.get('default_binding_ref')} "
                     f"(fallback: {', '.join(ed.get('fallback_binding_refs') or [])})")
        ii = roles.get("intelligence") or {}
        lines.append(f"- intelligence: {', '.join(ii.get('binding_refs') or [])} "
                     f"({', '.join(ii.get('triggers') or [])})")
    else:
        lines.append(f"- primary: {intel.get('primary')}")
        lines.append(f"- fallback: {intel.get('fallback')}")
        lines.append(f"- degraded: {intel.get('degraded_mode')}")
    lines.append("")

    # ---- planes (v2 owner_policy; v1 state)
    lines.append("## planes")
    planes = (cfg_sw or {}).get("planes") or {}
    if planes:
        for route, spec in planes.items():
            pol = spec.get("owner_policy") or spec.get("state")
            lines.append(f"- {route}: {pol} — {spec.get('note', '')}")
    else:
        lines.append("- none configured")
    lines.append("")

    # ---- intent pointers
    intent = (cfg_sw or {}).get("intent") or {}
    lines.append("## intent pointers")
    if intent:
        for k, v in intent.items():
            lines.append(f"- {k}: {v}")
    else:
        lines.append("- none")
    lines.append("")

    # ---- source freshness ledger
    lines.append("## source freshness")
    for name, meta in sorted(sources.items()):
        lines.append(f"- {name}: {meta['state']}" + (f" ({meta['note']})" if meta["note"] else ""))
    lines.append("")

    # ---- deeper-evidence pointers (bounded view -> full records)
    lines.append("## deeper evidence")
    lines.append("- tasks: queue.sqlite3 (dev:~/.local/state/zcode-tui/queue/)")
    lines.append("- audit ledger: /tmp/dreamer/{dream,audit}-ledger.jsonl")
    lines.append("- registry: ~/.yggterm/swarm/registry/ (receipts.jsonl = change log)")
    lines.append("- boards: ~/data/msggraph/boards/ (dreams/features, infra/meta)")
    lines.append("- memory: ygg-memory namespaces + NOW.md rollups")
    lines.append("- receipts: ~/.yggterm/dream/policy/receipts.jsonl")

    text = "\n".join(lines) + "\n"

    # ---- size budget: deterministic truncation with overflow counts
    overflow = 0
    if len(text.encode()) > MAX_BYTES:
        kept, cur = [], 0
        body = lines
        kept = [body[0], body[1], body[2], body[3]]  # header block
        cur = len("\n".join(kept).encode())
        for ln in body[4:]:
            b = len((ln + "\n").encode())
            if cur + b > MAX_BYTES - 200:
                overflow += 1
                continue
            kept.append(ln)
            cur += b
        kept.append(f"[projection truncated: {overflow} overflow lines — full data in sources]")
        text = "\n".join(kept) + "\n"

    out = Path(str(out_path).replace("~", str(Path.home()))).expanduser()
    out.parent.mkdir(parents=True, exist_ok=True)
    tmp = out.with_suffix(".tmp")
    tmp.write_text(text)
    tmp.replace(out)
    snap = out.with_suffix(".json")
    snap.write_text(json.dumps({
        "generated_at": now.astimezone(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "config_revision": ident or None,
        "sources": sources,
        "campaigns": len(campaigns), "trips": trips_total,
        "overflow_lines": overflow,
        "markdown_bytes": len(text.encode()),
    }, indent=2, sort_keys=True) + "\n")
    return text
