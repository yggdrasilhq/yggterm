"""The P1 information system, v1: the empire index.

A compact, deterministic rollup of the swarm's state — campaigns, queue,
findings, overlays, planes, intent pointers — regenerated on every
supervise pass. Consults (sol/gemini briefs) embed this file as their
BOUNDED VIEW; deeper evidence is reachable through the pointers it carries.
A large context window cannot substitute for complete records; the index
makes missing coverage visible instead of hiding it.
"""
import json
from datetime import datetime
from pathlib import Path


def _load_json(path):
    try:
        return json.loads(Path(path).read_text())
    except Exception:
        return None


def build(cfg_sw, floors, queue_rows, state_dir, out_path, now=None):
    """Assemble the index from the live sources. queue_rows: the queue's task
    rows (list of dicts) or None (unknown). Writes markdown, returns it."""
    now = now or datetime.now().astimezone()
    lines = [f"# empire index — {now:%F %H:%M %Z}", ""]
    lines.append("Bounded view for swarm intelligence consults. Regenerated every "
                 "supervise pass; pointers below reach deeper evidence.")
    lines.append("")

    # campaigns (namespaces under supply)
    ns = [(n.get("ns"), n.get("per_day")) for n in ((floors or {}).get("namespaces") or [])]
    lines.append("## campaigns (supply namespaces)")
    for name, per_day in ns:
        lines.append(f"- {name} (floor {per_day}/day)")
    lines.append("")

    # queue state
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

    # findings + coverage (the audit wire)
    fb = _load_json((cfg_sw or {}).get("kernel", {}).get("feedback",
                    "~/.yggterm/dream/audit-feedback.json"))
    lines.append("## audit findings (wire)")
    if fb is None:
        lines.append("- no snapshot (audit has never run or is unreadable)")
    else:
        lines.append(f"- generated {fb.get('generated_at')}, valid_until {fb.get('valid_until')}, "
                     f"evaluator {fb.get('evaluator_version')}")
        for f in fb.get("findings", []):
            scope = f.get("scope") or {}
            lines.append(f"- {f['code']} [{f['status']}] route={scope.get('route')} "
                         f"family={scope.get('family') or '-'} alloc={f.get('allocation')}")
    lines.append("")

    # active policy overlays
    pol_dir = Path(str((cfg_sw or {}).get("kernel", {}).get("policy_dir",
                  "~/.yggterm/dream/policy")).replace("~", str(Path.home())))
    ov = _load_json(pol_dir / "overlays.json")
    lines.append("## policy overlays")
    if not ov:
        lines.append("- none active")
    else:
        for o in ov:
            lines.append(f"- {o.get('type')} {o.get('route') or o.get('namespace')} "
                         f"until {o.get('expires_at')} ({o.get('reason_code')})")
    lines.append("")

    # intelligence layer state (from swarmconfig)
    intel = (cfg_sw or {}).get("intelligence") or {}
    lines.append("## intelligence layer")
    lines.append(f"- primary: {intel.get('primary')}")
    lines.append(f"- fallback: {intel.get('fallback')}")
    lines.append(f"- degraded: {intel.get('degraded_mode')}")
    lines.append("")

    # planes
    lines.append("## planes")
    for route, spec in ((cfg_sw or {}).get("planes") or {}).items():
        lines.append(f"- {route}: {spec.get('state')} — {spec.get('note', '')}")
    lines.append("")

    # intent pointers
    intent = (cfg_sw or {}).get("intent") or {}
    lines.append("## intent pointers")
    for k, v in intent.items():
        lines.append(f"- {k}: {v}")
    lines.append("")

    # deeper-evidence pointers (bounded view -> full records)
    lines.append("## deeper evidence")
    lines.append("- tasks: queue.sqlite3 (dev:~/.local/state/zcode-tui/queue/)")
    lines.append("- audit ledger: /tmp/dreamer/{dream,audit}-ledger.jsonl")
    lines.append("- boards: ~/data/msggraph/boards/ (dreams/features, infra/meta)")
    lines.append("- memory: ygg-memory namespaces + NOW.md rollups")
    lines.append("- receipts: ~/.yggterm/dream/policy/receipts.jsonl")

    text = "\n".join(lines) + "\n"
    out = Path(out_path).expanduser()
    out.parent.mkdir(parents=True, exist_ok=True)
    tmp = out.with_suffix(".tmp")
    tmp.write_text(text)
    tmp.replace(out)
    return text
