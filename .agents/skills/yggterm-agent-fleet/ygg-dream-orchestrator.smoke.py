#!/usr/bin/env python3
"""Runtime smoke for the orchestrator: exercises cmd_supervise with MOCKED
run/qjson (no ssh, no mutations) so a NameError/AttributeError in the
supervise path fails here instead of in the timer (measured 2026-10-25: an
orphaned `verdict` reference crashed every supervise pass for 4h)."""
import importlib.util, json, sys, types
from pathlib import Path

src = Path(__file__).parent / "ygg-dream-orchestrator.py"
spec = importlib.util.spec_from_file_location("orch", src)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

calls = []
def fake_run(cmd, timeout=120):
    calls.append(" ".join(str(c) for c in cmd)[:80])
    # simulate a transcript tail for the false-completion check (clean)
    if "tail -c" in calls[-1]:
        return 0, json.dumps({"outcome": "completed", "events": [{"kind": "text"}]}), ""
    return 0, "", ""

ROWS = [
    {"id": "ztq-fake-1", "status": "failed", "route": "zai-coding-plan",
     "last_error": "quota exhausted 429", "title": "DREAM[syn]: -home-x consolidation",
     "updated_at": m.int(m.time.time() * 1000) if hasattr(m, "int") else 0},
    {"id": "ztq-fake-2", "status": "completed", "route": "zai-coding-plan",
     "title": "DREAM[syn]: -home-x consolidation"},
    {"id": "ztq-expr", "status": "completed", "title": "CANARY experiment — skip reroute"},
    {"id": "ztq-parity-old", "status": "failed", "title": "UX-PARITY continuous: stale"},
]

m.run = fake_run
m.qjson = lambda cfg, args, timeout=60: [dict(r, status=r["status"]) for r in ROWS]
m.queue_cli = lambda cfg: ["echo", "queue-cli"]
cfg = {"queue": {"host": "localhost", "repo": "/tmp", "cli": "echo"},
       "routes": ["zai-coding-plan"], "quota_calendars": {},
       "false_completion": {"audit_per_pass": 3},
       "ux_parity_watch": {"enabled": False}}
import tempfile, os
tmp = Path(tempfile.mkdtemp())
m.LEDGER = tmp / "wake-ledger.jsonl"
m.POLICY_DIR = tmp / "policy"; m.OVERLAYS = m.POLICY_DIR / "overlays.json"
m.RECEIPTS = m.POLICY_DIR / "receipts.jsonl"; m.FEEDBACK = tmp / "feedback.json"
m.build_overlays = lambda cfg: "smoke: feedback skipped"

m.cmd_supervise(cfg)
rows = m.LEDGER.read_text().strip().splitlines()
assert rows, "supervise wrote no ledger row"
d = json.loads(rows[-1])
assert d["audited"] >= 1, d
assert not any("ztq-expr" == x.get("id") for x in d.get("rerouted", []) + d.get("held", [])), "experiment was mutated!"
print("SUPERVISE SMOKE PASS — no NameError, experiment skipped, ledger written")
